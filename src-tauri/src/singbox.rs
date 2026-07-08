use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use anyhow::{Result, anyhow};
use serde_json::Value;
use tauri::Emitter;
use tokio::process::Command as TokioCommand;
use tokio::io::{AsyncBufReadExt, BufReader};

/// Open today's rolling log file in append mode under `app_data_dir/logs/`. Returns the
/// open file together with the date stamp (`YYYYMMDD`) it was opened for, so the writer
/// can notice a day rollover and reopen against the new day's file. `None` if the
/// directory or file cannot be created (logging then stays in-memory only).
fn open_daily_log_file() -> Option<(std::fs::File, String)> {
    let dir = crate::config::app_data_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    // Prune before opening today's file, so retention runs both at core start and on each
    // day rollover — old files never accumulate without bound.
    prune_old_logs(&dir);
    let stamp = chrono::Local::now().format("%Y%m%d").to_string();
    let path = dir.join(format!("skylark-{}.log", stamp));
    let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
    Some((file, stamp))
}

/// Number of days of daily log files to keep, counting today. Anything older is deleted.
const LOG_RETENTION_DAYS: i64 = 7;

/// Delete daily log files older than [`LOG_RETENTION_DAYS`] so `logs/` keeps only the most
/// recent week. Only files matching exactly `skylark-YYYYMMDD.log` with a parseable date are
/// considered — any other file in the directory (e.g. singbox-*.log) is left untouched.
/// Best-effort: unreadable dir / undeletable file is ignored.
fn prune_old_logs(dir: &std::path::Path) {
    // Keep today plus the previous RETENTION-1 days; delete strictly older stamps.
    let cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(LOG_RETENTION_DAYS - 1);
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let stamp = match name.strip_prefix("skylark-").and_then(|s| s.strip_suffix(".log")) {
            Some(s) => s,
            None => continue,
        };
        // Only prune files whose stamp is a valid date — never touch anything we can't parse.
        if let Ok(date) = chrono::NaiveDate::parse_from_str(stamp, "%Y%m%d") {
            if date < cutoff {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Windows CREATE_NO_WINDOW flag: prevents a console window from popping up
/// when spawning child processes (sing-box, taskkill).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Windows CREATE_NEW_PROCESS_GROUP flag: makes the core the root of its own console
/// process group (group id == its pid), so we can target a console control event at JUST
/// that group instead of broadcasting to the whole console (which includes this GUI).
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Ask sing-box to shut down **gracefully** by delivering a console CTRL+BREAK, which Go
/// maps to `os.Interrupt` — triggering sing-box's own shutdown path, including TUN teardown
/// (`WintunDeleteAdapter`) and `strict_route` route cleanup. A hard `taskkill /F` skips all
/// of that and orphans the TUN adapter + routes, which later surfaces as "TUN on, but no
/// network" after a restart.
///
/// How it works: the core is spawned with `CREATE_NEW_PROCESS_GROUP` (group id == its pid),
/// so we deliver the event to THAT GROUP specifically —
/// `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid)` reaches only the core, never this GUI.
/// We must briefly `AttachConsole` to the core's hidden console (so the caller shares it and
/// is allowed to signal its group), but because the event is *targeted* at the core's group
/// — not a group-0 broadcast — the GUI, which lives in a different process group, is
/// unaffected. This is what fixes the "app exits when turning TUN off" self-kill that the
/// previous group-0 `CTRL_C_EVENT` broadcast caused regardless of caller context.
///
/// CTRL+BREAK (not CTRL+C) is required: a process started with `CREATE_NEW_PROCESS_GROUP`
/// has CTRL+C disabled by default, whereas CTRL+BREAK is always delivered. Go's runtime
/// reports both as `os.Interrupt`, so sing-box's graceful shutdown still runs.
///
/// Returns `false` if attaching/sending failed (process already gone, or no console), in
/// which case the caller should fall back to a force kill.
#[cfg(target_os = "windows")]
fn send_graceful_break(pid: u32) -> bool {
    use winapi::um::wincon::{AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
    use winapi::um::processenv::SetStdHandle;
    use winapi::um::winbase::{STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE};

    unsafe {
        // Detach from any console we might already own (a GUI build normally has none), then
        // attach to the core's so we're permitted to signal its process group.
        FreeConsole();
        if AttachConsole(pid) == 0 {
            return false;
        }
        // Target ONLY the core's process group (id == pid) — not a group-0 broadcast — so the
        // GUI (a different group) can never be hit. No self-protection handler needed.
        let sent = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) != 0;
        FreeConsole();
        // CRITICAL: AttachConsole repointed our STD_INPUT/OUTPUT/ERROR slots at the core's
        // console; the FreeConsole above closed that console but leaves those slots holding
        // now-invalid handles. Any later child spawn that inherits stdin (the next core,
        // `sing-box version`, `taskkill` …) would then fail with ERROR_INVALID_HANDLE
        // (os error 6) — surfacing as a permanent "控制端口未就绪" / "版本未知：句柄无效"
        // until the app is restarted. Reset the slots to NULL, the no-console state the GUI
        // launched in, so subsequent spawns inherit cleanly.
        SetStdHandle(STD_INPUT_HANDLE, std::ptr::null_mut());
        SetStdHandle(STD_OUTPUT_HANDLE, std::ptr::null_mut());
        SetStdHandle(STD_ERROR_HANDLE, std::ptr::null_mut());
        sent
    }
}

/// Durable console control handler: swallow Ctrl+C / Ctrl+Break for THIS (GUI) process so a
/// console control event can never terminate us. Returning TRUE marks the event handled;
/// other control types (close/logoff/shutdown) fall through to default handling.
#[cfg(target_os = "windows")]
unsafe extern "system" fn ignore_console_ctrl(ctrl_type: u32) -> i32 {
    use winapi::um::wincon::{CTRL_BREAK_EVENT, CTRL_C_EVENT};
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => 1, // TRUE — handled, so the GUI is not killed
        _ => 0,                               // FALSE — defer to the default handler
    }
}

/// Permanently make this process ignore console Ctrl+C / Ctrl+Break. Call ONCE at startup.
///
/// Defense-in-depth: graceful TUN teardown sends a CTRL+BREAK *targeted* at the core's own
/// process group (see [`send_graceful_break`]), so it should never reach the GUI in the
/// first place. This handler is a belt-and-suspenders guard — should a console control event
/// ever land on the GUI (e.g. via an inherited console), it is swallowed instead of
/// terminating the app. No-op off Windows.
#[cfg(target_os = "windows")]
pub fn install_console_ctrl_guard() {
    use winapi::um::consoleapi::SetConsoleCtrlHandler;
    // SAFETY: registers a process-wide control handler with a 'static fn pointer.
    unsafe {
        SetConsoleCtrlHandler(Some(ignore_console_ctrl), 1);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn install_console_ctrl_guard() {}

#[derive(Debug, Clone)]
pub struct SingboxState {
    pub running: bool,
    pub pid: Option<u32>,
    pub start_time: Option<Instant>,
    pub version: Option<String>,
    pub logs: Vec<String>,
    /// Whether the currently-running core was started with the TUN inbound. Used to
    /// decide whether a mode switch needs a config rebuild + restart (TUN) or can be
    /// applied without touching the core (system-proxy toggle on a persistent core).
    pub tun_mode: bool,
    /// Set true by `stop_singbox` right before it signals the core, so the process-exit
    /// waiter can tell an INTENTIONAL stop from an unexpected crash. Reset on the next start.
    pub stopping: bool,
    /// Monotonic start counter. Each `start_singbox` bumps it and the spawned waiter captures
    /// its value; when the waiter later observes a DIFFERENT epoch it knows a newer core has
    /// superseded it (a fast restart) and must neither clobber the new core's state nor
    /// report a crash. Closes the stop→start race around crash detection.
    pub epoch: u64,
}

impl Default for SingboxState {
    fn default() -> Self {
        Self {
            running: false,
            pid: None,
            start_time: None,
            version: None,
            logs: Vec::new(),
            tun_mode: false,
            stopping: false,
            epoch: 0,
        }
    }
}

pub type SharedState = Arc<Mutex<SingboxState>>;

pub fn new_shared_state() -> SharedState {
    Arc::new(Mutex::new(SingboxState::default()))
}

/// Get the sing-box binary path.
/// Priority: user-downloaded binary in app data dir > bundled sidecar shipped with the app.
pub fn singbox_binary_path() -> Result<std::path::PathBuf> {
    let path = crate::updater::resolved_singbox_path();
    if path.exists() {
        return Ok(path);
    }
    Err(anyhow!(
        "未找到 sing-box 可执行文件。请在设置页面下载，或重新安装应用。\n检查路径: {:?}",
        path
    ))
}

/// Fetch sing-box version
#[allow(dead_code)]
pub async fn get_version(binary_path: &std::path::Path) -> Result<String> {
    let mut cmd = TokioCommand::new(binary_path);
    cmd.arg("version");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let version = stdout.lines()
        .next()
        .unwrap_or("unknown")
        .to_string();
    Ok(version)
}

/// Validate a generated config with `sing-box check -c <path>` before launching. Catches
/// semantic errors (duplicate outbound tag, unknown/removed fields, malformed rules) up
/// front and returns the core's OWN stderr, instead of letting the core spawn, silently
/// exit, and surface only as a generic "控制端口未就绪" readiness timeout. `check` needs no
/// privileges, so the user-owned binary is used even for a TUN start.
async fn check_config(binary: &std::path::Path, config_path: &std::path::Path) -> Result<()> {
    let mut cmd = TokioCommand::new(binary);
    cmd.args(["check", "-c", config_path.to_str().unwrap_or("")])
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .await
        .map_err(|e| anyhow!("无法运行 sing-box check：{}", e))?;
    if output.status.success() {
        return Ok(());
    }
    // Surface the last few stderr lines — the concrete reason (e.g. "duplicate tag: xxx").
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(6)..].join("\n");
    Err(anyhow!(
        "配置校验失败（sing-box check）：\n{}",
        if tail.trim().is_empty() { "未知错误".to_string() } else { tail }
    ))
}

/// Kill any orphaned sing-box.exe processes left over from a previous run or app update.
/// Uses process-name kill (not PID) so it catches instances started by any previous app version.
///
/// Only sleeps to let the OS release the bound ports when an orphan was actually found
/// and killed — the common case (no orphan) returns immediately, saving ~400ms on every start.
#[cfg(target_os = "windows")]
async fn kill_orphan_singbox() -> bool {
    // taskkill /F /IM sing-box.exe exits 0 when it killed at least one process,
    // and non-zero (128) when no matching process exists. The caller waits for the bound
    // port to actually free up (see wait_until_port_free) rather than a fixed sleep — a
    // force-killed TUN core can hold the Clash API port for seconds while Windows tears
    // down the Wintun driver, which a short fixed delay would race.
    TokioCommand::new("taskkill")
        .args(["/F", "/IM", "sing-box.exe"])
        .stdin(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
async fn kill_orphan_singbox() -> bool {
    // Match the core's invocation signature ("…/sing-box run -c …") rather than the bare
    // name. The core is the only process ever launched with `run -c`, so this pattern
    // targets exactly the orphaned core and never the GUI app itself.
    let killed = TokioCommand::new("pkill")
        .args(["-f", "sing-box run -c"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    // On macOS the previous run's TUN core was root-owned, so the user-level pkill above
    // can't reap it. Clear it through the passwordless rule (no-op when the service isn't
    // installed, or when no such orphan exists). Best-effort; failure is ignored.
    #[cfg(target_os = "macos")]
    {
        let _ = TokioCommand::new("sudo")
            .args(["-n", "/usr/bin/pkill", "-KILL", "-f", crate::tun::TUN_ROOT_BIN])
            .output()
            .await;
    }
    killed
}

/// After an orphan was killed, wait until the Clash API control port is actually released
/// before the new core tries to bind it — otherwise the new core hits "address already in
/// use" and exits, which the caller then misreads as "控制端口未就绪". A force-killed TUN
/// core can hold the port for a few seconds (Wintun teardown). Caps at ~3s so a port held
/// by an unrelated process can't hang startup indefinitely.
async fn wait_until_port_free(api_port: u16) {
    let addr = format!("127.0.0.1:{}", api_port);
    for _ in 0..60 {
        let in_use = tokio::time::timeout(
            Duration::from_millis(150),
            tokio::net::TcpStream::connect(&addr),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);
        if !in_use {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Poll the Clash API control port until it accepts a TCP connection, signalling that
/// sing-box has finished starting up. Returns as soon as the port is reachable (typically
/// ~100-200ms) instead of blocking on a fixed delay. Caps out after ~2s so a config that
/// never binds the port doesn't hang the caller indefinitely.
async fn wait_until_ready(api_port: u16, state: &SharedState) -> bool {
    let addr = format!("127.0.0.1:{}", api_port);
    // The spawn task sets `running = true` right after a successful spawn and flips it back
    // to false the moment `child.wait()` returns. We only treat a false flag as "the core
    // died" AFTER having observed it true — otherwise a slow spawn (flag not set yet) would
    // be misread as an early exit and abort a perfectly good start.
    let mut saw_running = false;
    for _ in 0..40 {
        if tokio::time::timeout(
            Duration::from_millis(200),
            tokio::net::TcpStream::connect(&addr),
        )
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
        {
            return true;
        }
        let running = state.lock().unwrap_or_else(|e| e.into_inner()).running;
        if running {
            saw_running = true;
        } else if saw_running {
            // Core was up, then exited before binding the port — a failed config / TUN
            // create / port bind. Stop now so the caller can report the real stderr instead
            // of burning the whole timeout.
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Start sing-box with the given config file
pub async fn start_singbox(
    app_handle: &tauri::AppHandle,
    config_path: &std::path::Path,
    state: SharedState,
    api_port: u16,
    tun_mode: bool,
) -> Result<()> {
    {
        let s = state.lock().unwrap_or_else(|e| e.into_inner());
        if s.running {
            return Err(anyhow!("sing-box 已在运行中"));
        }
    }

    // Kill any leftover sing-box processes (e.g. from app update / crash). This prevents
    // "address already in use" on the mixed/http/socks/API ports. When an orphan was
    // actually killed, wait for the control port to be released before rebinding — a
    // force-killed TUN core can hold it for seconds while the Wintun driver tears down.
    if kill_orphan_singbox().await {
        wait_until_port_free(api_port).await;
    }

    let binary = singbox_binary_path()?;
    let config_path = config_path.to_path_buf();

    // Statically validate the generated config BEFORE launching. This turns a semantic
    // error (duplicate outbound tag, unknown field, bad rule) into an explicit, actionable
    // message instead of an opaque "控制端口未就绪" timeout after the core silently exits.
    check_config(&binary, &config_path).await?;

    let state_clone = state.clone();
    let app_log = app_handle.clone();

    tokio::spawn(async move {
        let cfg_arg = config_path.to_str().unwrap_or("");
        // On macOS, TUN needs root. We launch the ROOT-OWNED core via passwordless `sudo -n`
        // (set up once by tun::install_tun_service) instead of running the whole GUI as root.
        // stderr is still piped straight to us, so the log view keeps working. Non-TUN runs,
        // and all runs on Windows/Linux, spawn the user-owned binary directly as before.
        #[cfg(target_os = "macos")]
        let mut cmd = if tun_mode {
            let mut c = TokioCommand::new("sudo");
            c.args(["-n", crate::tun::TUN_ROOT_BIN, "run", "-c", cfg_arg]);
            c
        } else {
            let mut c = TokioCommand::new(&binary);
            c.args(["run", "-c", cfg_arg]);
            c
        };
        #[cfg(not(target_os = "macos"))]
        let mut cmd = {
            let mut c = TokioCommand::new(&binary);
            c.args(["run", "-c", cfg_arg]);
            c
        };
        // Pin the working directory to the writable app data dir. A GUI app launched
        // from /Applications inherits cwd `/` (read-only on macOS), so any config field
        // that resolves a relative path — cache_file's db, external_ui — would otherwise
        // fail there. The config already passes absolute paths, but setting cwd makes the
        // core robust against any relative default sing-box might use.
        cmd.current_dir(crate::config::app_data_dir())
            .stdin(std::process::Stdio::null())
            // sing-box logs to stderr; stdout is unused. Discard it rather than pipe it —
            // a piped-but-never-read stdout would block the core if it ever filled the
            // (~64KB) pipe buffer. stderr is piped and drained below for the log view.
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        // CREATE_NEW_PROCESS_GROUP puts the core in its own console process group so a
        // graceful CTRL+BREAK can be targeted at JUST the core (see send_graceful_break),
        // never reaching this GUI. CREATE_NO_WINDOW keeps its console hidden.
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
        let mut child = match cmd.spawn()
        {
            Ok(c) => c,
            Err(e) => {
                log::error!("启动 sing-box 失败: {}", e);
                return;
            }
        };

        let pid = child.id();
        // Capture this core's epoch so the exit handler below can detect whether a newer
        // core has since superseded it (fast restart) and must not report a crash.
        let my_epoch = {
            let mut s = state_clone.lock().unwrap_or_else(|e| e.into_inner());
            s.running = true;
            s.pid = pid;
            s.start_time = Some(Instant::now());
            s.tun_mode = tun_mode;
            s.stopping = false;
            s.epoch = s.epoch.wrapping_add(1);
            s.epoch
        };

        // Read stderr for logs. Each line is (a) appended to the in-memory ring buffer,
        // (b) optionally appended to today's rolling log file (crash-safe persistence),
        // and (c) pushed to the UI via a `singbox-log` event so the frontend does not
        // have to poll and re-clone the whole buffer every second.
        if let Some(stderr) = child.stderr.take() {
            let state_log = state_clone.clone();
            let app_log = app_log.clone();
            tokio::spawn(async move {
                // Read the persistence flag once at start; a runtime toggle takes effect
                // on the next core (re)start, which is acceptable for this setting.
                let mut log_file = if crate::config::load_app_config().log_to_file {
                    open_daily_log_file()
                } else {
                    None
                };
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if log_file.is_some() {
                        use std::io::Write;
                        // Day rollover: the log file was opened for the date at core start and
                        // its handle would otherwise be written to forever, so a core that runs
                        // past midnight keeps appending to its start-day file (which is why
                        // skylark-<startday>.log grows unbounded across days). Compare the
                        // current date against the stamp we opened with and reopen against the
                        // new day's file when it advances.
                        let today = chrono::Local::now().format("%Y%m%d").to_string();
                        let rolled = log_file.as_ref().map(|(_, s)| *s != today).unwrap_or(false);
                        if rolled {
                            if let Some(reopened) = open_daily_log_file() {
                                log_file = Some(reopened);
                            }
                        }
                        if let Some((f, _)) = log_file.as_mut() {
                            let _ = writeln!(f, "{}", line);
                        }
                    }
                    {
                        let mut s = state_log.lock().unwrap_or_else(|e| e.into_inner());
                        s.logs.push(line.clone());
                        if s.logs.len() > 1000 {
                            s.logs.drain(0..100);
                        }
                    }
                    let _ = app_log.emit("singbox-log", line);
                }
            });
        }

        let _ = child.wait().await;
        // The core has exited. Decide what this exit means under the state lock:
        //   • superseded → a newer core (higher epoch) already replaced us; do NOTHING so we
        //     don't clobber the new core's state or fire a spurious crash.
        //   • intentional (`stopping`) → a normal stop/restart; just clear our own state.
        //   • otherwise → an UNEXPECTED exit (crash / killed / config died): clear state and
        //     signal the crash so the caller can drop the now-dead system proxy and notify.
        let crashed = {
            let mut s = state_clone.lock().unwrap_or_else(|e| e.into_inner());
            let superseded = s.epoch != my_epoch;
            if superseded {
                false
            } else {
                let unexpected = !s.stopping;
                s.running = false;
                s.pid = None;
                s.start_time = None;
                s.tun_mode = false;
                unexpected
            }
        };
        if crashed {
            // The system proxy (if on) now points at a dead port → total black-hole. Clear it
            // so traffic falls back to direct (fail-open), and tell the UI to reconcile. We do
            // NOT auto-restart here to avoid a crash loop; the user/frontend can re-enable.
            let _ = crate::proxy::set_system_proxy(false, 0, false);
            log::error!("sing-box 意外退出（非主动停止）");
            let _ = app_log.emit("singbox-crashed", ());
        }
    });

    // Confirm the core actually came up: the Clash API control port must accept a
    // connection. If it never binds — invalid config, the port still held by an orphan
    // right after an app upgrade, or TUN failing without admin rights — the process has
    // effectively failed even though `spawn()` succeeded. Returning Err here is critical:
    // otherwise the caller (apply_connection_mode) would enable the system proxy / treat
    // TUN as active on top of a DEAD core, which presents as "proxy on, but no network".
    if !wait_until_ready(api_port, &state).await {
        // Capture the core's own last log lines — the real reason (bad config, "address
        // already in use", TUN adapter create failure) is here, not in our generic guess.
        let tail = {
            let s = state.lock().unwrap_or_else(|e| e.into_inner());
            s.logs.iter().rev().take(8).rev().cloned().collect::<Vec<_>>().join("\n")
        };
        // Mark this as an intentional teardown so the waiter treats the core's exit as a
        // failed START (reported via the Err below), not a runtime crash.
        state.lock().unwrap_or_else(|e| e.into_inner()).stopping = true;
        let _ = kill_orphan_singbox().await;
        {
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            s.running = false;
            s.pid = None;
            s.start_time = None;
            s.tun_mode = false;
        }
        let base = "sing-box 启动失败：控制端口未就绪（配置无效 / 端口被占用 / TUN 需要管理员权限）";
        return Err(if tail.trim().is_empty() {
            anyhow!("{}", base)
        } else {
            anyhow!("{}\n\n内核最后日志：\n{}", base, tail)
        });
    }

    Ok(())
}

/// Stop sing-box.
///
/// `graceful` should be true only when the running instance is in TUN mode: a graceful
/// shutdown gives sing-box time to call WintunDeleteAdapter() and tear down the TUN driver
/// cleanly. For plain system-proxy / mixed-inbound runs there is no adapter to clean up, so
/// we force-kill immediately — that path is near-instant.
///
/// Liveness is detected by polling the in-memory `running` flag, which the background waiter
/// task flips to false the moment `child.wait()` returns. This avoids spawning a slow
/// `tasklist` process on every poll (the old approach cost up to ~3s).
pub async fn stop_singbox(state: SharedState, graceful: bool) -> Result<()> {
    let pid = {
        // Mark the coming exit as intentional BEFORE signalling the core, so the process-exit
        // waiter reports it as a normal stop rather than a crash (no proxy clear / crash event).
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.stopping = true;
        s.pid
    };

    if let Some(pid) = pid {
        #[cfg(target_os = "windows")]
        {
            let mut exited = false;

            if graceful {
                // Graceful shutdown: deliver a targeted CTRL+BREAK (→ os.Interrupt) so sing-box
                // runs its own TUN teardown (WintunDeleteAdapter) and strict_route cleanup
                // before exiting — a hard `taskkill /F` skips that and orphans the TUN
                // adapter/routes. The event targets the core's own process group only, so it
                // never takes the GUI down with it (see send_graceful_break).
                if send_graceful_break(pid) {
                    // TUN driver teardown can take a beat; poll the shared state (no process
                    // spawn) up to ~3s, returning as soon as the core has actually exited.
                    for _ in 0..60 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        if !state.lock().unwrap_or_else(|e| e.into_inner()).running {
                            exited = true;
                            break;
                        }
                    }
                }
            }

            // Force-kill if not already gone (or whenever a graceful wait wasn't requested).
            if !exited {
                let _ = TokioCommand::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .stdin(std::process::Stdio::null())
                    .creation_flags(CREATE_NO_WINDOW)
                    .output()
                    .await;
            }
        }
        // macOS: a TUN core runs as root (via sudo), so a direct `kill` from the non-root
        // GUI would be denied — and `pid` is sudo's, not sing-box's. Signal it through the
        // passwordless pkill rule instead. A non-TUN core runs as the user, so kill its pid.
        #[cfg(target_os = "macos")]
        {
            if graceful {
                // SIGTERM lets sing-box tear the utun device + auto_route down cleanly.
                let _ = TokioCommand::new("sudo")
                    .args(["-n", "/usr/bin/pkill", "-TERM", "-f", crate::tun::TUN_ROOT_BIN])
                    .output()
                    .await;
                let mut exited = false;
                for _ in 0..60 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    if !state.lock().unwrap_or_else(|e| e.into_inner()).running {
                        exited = true;
                        break;
                    }
                }
                if !exited {
                    let _ = TokioCommand::new("sudo")
                        .args(["-n", "/usr/bin/pkill", "-KILL", "-f", crate::tun::TUN_ROOT_BIN])
                        .output()
                        .await;
                }
            } else {
                let _ = TokioCommand::new("kill")
                    .args(["-SIGKILL", &pid.to_string()])
                    .output()
                    .await;
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let signal = if graceful { "-SIGTERM" } else { "-SIGKILL" };
            let _ = TokioCommand::new("kill")
                .args([signal, &pid.to_string()])
                .output()
                .await;
            if graceful {
                for _ in 0..30 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    if !state.lock().unwrap_or_else(|e| e.into_inner()).running {
                        break;
                    }
                }
            }
        }
    }

    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    s.running = false;
    s.pid = None;
    s.start_time = None;

    Ok(())
}


/// Fetch connections from Clash API
pub async fn fetch_connections(api_port: u16) -> Result<Vec<crate::types::ConnectionInfo>> {
    let url = format!("http://127.0.0.1:{}/connections", api_port);
    // Local Clash API call — bypass any env-var HTTP(S)_PROXY so it always goes direct.
    let client = reqwest::Client::builder().no_proxy().build()?;
    let resp = client.get(&url)
        .bearer_auth(crate::config::api_secret())
        .timeout(Duration::from_secs(2))
        .send()
        .await?;
    let body: Value = resp.json().await?;

    let mut result = Vec::new();
    if let Some(connections) = body["connections"].as_array() {
        for c in connections {
            result.push(crate::types::ConnectionInfo {
                id: c["id"].as_str().unwrap_or("").to_string(),
                network: c["metadata"]["network"].as_str().unwrap_or("").to_string(),
                conn_type: c["metadata"]["type"].as_str().unwrap_or("").to_string(),
                source: format!(
                    "{}:{}",
                    c["metadata"]["sourceIP"].as_str().unwrap_or(""),
                    c["metadata"]["sourcePort"].as_str().unwrap_or("")
                ),
                destination: format!(
                    "{}:{}",
                    c["metadata"]["destinationIP"].as_str().unwrap_or(""),
                    c["metadata"]["destinationPort"].as_str().unwrap_or("")
                ),
                host: c["metadata"]["host"].as_str().unwrap_or("").to_string(),
                rule: c["rule"].as_str().unwrap_or("").to_string(),
                rule_payload: c["rulePayload"].as_str().unwrap_or("").to_string(),
                chains: c["chains"].as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default(),
                upload: c["upload"].as_u64().unwrap_or(0),
                download: c["download"].as_u64().unwrap_or(0),
                start: c["start"].as_str().unwrap_or("").to_string(),
            });
        }
    }

    Ok(result)
}

/// Close a single active connection via the Clash API (`DELETE /connections/{id}`).
pub async fn close_connection(api_port: u16, id: &str) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/connections/{}", api_port, id);
    // Local Clash API call — bypass any env-var HTTP(S)_PROXY so it always goes direct.
    let client = reqwest::Client::builder().no_proxy().build()?;
    client.delete(&url)
        .bearer_auth(crate::config::api_secret())
        .timeout(Duration::from_secs(3))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// Close all active connections via the Clash API (`DELETE /connections`).
pub async fn close_all_connections(api_port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{}/connections", api_port);
    // Local Clash API call — bypass any env-var HTTP(S)_PROXY so it always goes direct.
    let client = reqwest::Client::builder().no_proxy().build()?;
    client.delete(&url)
        .bearer_auth(crate::config::api_secret())
        .timeout(Duration::from_secs(3))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
