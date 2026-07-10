use tauri::State;
use std::sync::Mutex;
use anyhow::anyhow;
use serde_json::Value;

use crate::types::{self, *};
use crate::config;
use crate::subscription;
use crate::singbox::{SharedState, start_singbox, stop_singbox};
use crate::proxy;

pub struct AppState {
    pub singbox_state: SharedState,
    pub subscriptions: Mutex<Vec<Subscription>>,
    pub nodes: Mutex<Vec<ProxyNode>>,
    pub outbounds: Mutex<Vec<Value>>,
    pub app_config: Mutex<AppConfig>,
    /// Serializes the whole core lifecycle (stop + adapter cleanup + start). Every entry
    /// point that (re)starts the core — dashboard, tray menu, startup restore, global
    /// shortcuts — goes through `apply_connection_mode`/`ensure_core`, which hold this lock
    /// for their full body. Without it, a tray toggle racing a dashboard toggle (or a fast
    /// off→on) interleaves two stop/start sequences, so two cores fight over the Clash API
    /// port and the TUN adapter and the second fails its readiness check ("控制端口未就绪").
    /// An async (tokio) mutex is required because the critical section awaits.
    pub core_lock: tokio::sync::Mutex<()>,
    /// True while a tray-initiated connection-mode switch is in flight. The tray menu has
    /// no "disabled while applying" state like the dashboard, so without this a burst of
    /// rapid on/off clicks queues a stack of real toggles that flap the TUN adapter. The
    /// tray handler test-and-sets this and drops clicks that arrive mid-switch.
    pub switching: std::sync::atomic::AtomicBool,
    /// Set (permanently) once app-exit / updater teardown begins. The core (re)start paths
    /// check it so a mode switch queued on `core_lock` behind the shutdown can't spawn a
    /// fresh core afterwards — `process::exit` skips destructors, so that core would outlive
    /// the app as an orphan, possibly holding a TUN adapter with the system proxy already
    /// cleared (a machine-wide black hole until the next launch reaps it).
    pub shutting_down: std::sync::atomic::AtomicBool,
}

/// RAII guard for [`AppState::switching`]. [`SwitchGuard::acquire`] test-and-sets the flag,
/// returning `None` if a switch is already in flight; the returned guard ALWAYS clears the
/// flag on drop — including on an early return or a panic while the switch future is awaited —
/// so a wedged switch can never permanently block every future mode toggle.
pub struct SwitchGuard<'a>(&'a std::sync::atomic::AtomicBool);

impl<'a> SwitchGuard<'a> {
    pub fn acquire(flag: &'a std::sync::atomic::AtomicBool) -> Option<Self> {
        if flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
            None
        } else {
            Some(SwitchGuard(flag))
        }
    }
}

impl Drop for SwitchGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Holds cloned handles to tray check-menu items so commands can update them.
pub struct TrayState {
    pub sys_proxy_item: Mutex<Option<tauri::menu::CheckMenuItem<tauri::Wry>>>,
    pub tun_item:       Mutex<Option<tauri::menu::CheckMenuItem<tauri::Wry>>>,
}

// ─── Sing-box Control (persistent-core model) ───────────────────────
//
// The core runs persistently while the app is open: it is started (idle, with only the
// local mixed inbound) on launch and stopped on exit. An idle core intercepts nothing —
// the mixed inbound just listens on a local port and triggers no DNS lookups until a
// client actually uses it — so leaving it running has no effect on the system network.
//
// "Proxying" is therefore decoupled from "core running":
//   • System proxy on/off = a pure Windows registry toggle pointing at the (already
//     running) mixed port. Instant, no core restart.
//   • TUN cannot run idle (its adapter captures all traffic the moment it is up), so a
//     TUN switch still rebuilds the config and restarts the core. Turning TUN off drops
//     the core back to the idle mixed-only state.

/// Clear the system proxy, logging (not failing) if the OS refuses — e.g. a locked registry
/// / gsettings error. The callers are all best-effort teardown paths, but a silent failure
/// there is exactly what leaves a user "offline" with a dangling proxy, so make it traceable.
fn clear_system_proxy_logged() {
    if let Err(e) = proxy::set_system_proxy(false, 0, false) {
        log::warn!("清除系统代理失败：{}", e);
    }
}

/// Ensure the core is running in the requested TUN mode. Builds the matching config and
/// (re)starts the core only when needed — when it is not running, or when the running
/// instance was started in a different TUN mode. Does NOT touch the system proxy.
/// Returns once the core is up (or immediately if it was already in the right mode).
pub async fn ensure_core(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    want_tun: bool,
) -> Result<(), String> {
    // Serialize against any other core (re)start — see AppState::core_lock.
    let _guard = state.core_lock.lock().await;
    ensure_core_inner(app_handle, state, want_tun).await
}

/// Inner body of [`ensure_core`]. Assumes the caller ALREADY holds `core_lock`; call this
/// (never `ensure_core`) from inside the lock — e.g. `apply_connection_mode` — so the
/// non-reentrant async mutex doesn't deadlock.
async fn ensure_core_inner(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    want_tun: bool,
) -> Result<(), String> {
    // App is exiting: never (re)start the core — see AppState::shutting_down.
    if state.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("应用正在退出，已取消本次操作".to_string());
    }

    // TUN preconditions differ by platform:
    //   • macOS — a one-time privileged service (passwordless sudo for the root-owned core)
    //     must be installed; the GUI itself stays non-root.
    //   • Windows/Linux — the process must be elevated (+ WinTun driver on Windows).
    if want_tun {
        #[cfg(target_os = "macos")]
        if !crate::tun::tun_service_installed() {
            return Err("TUN 模式需要先安装 TUN 服务（仅首次需要授权一次）。请在「设置 → TUN 模式」中点击「安装 TUN 服务」。".to_string());
        }
        #[cfg(not(target_os = "macos"))]
        if !crate::tun::is_elevated() {
            return Err("TUN 模式需要管理员/root 权限。请在「设置 → TUN 模式」中点击「以管理员身份重启」后再启动。".to_string());
        }
        #[cfg(target_os = "windows")]
        if !crate::tun::wintun_available() {
            return Err("TUN 模式需要 WinTun 驱动。请在「设置 → TUN 模式」中点击「下载 WinTun」后再启动。".to_string());
        }
    }

    let (running, current_tun) = {
        let s = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner());
        (s.running, s.tun_mode)
    };
    // Already running in the desired mode → nothing to do.
    if running && current_tun == want_tun {
        return Ok(());
    }

    // Persist the TUN flag first so the generated config matches the chosen mode.
    {
        let mut cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        cfg.tun_enabled = want_tun;
        let cfg_clone = cfg.clone();
        drop(cfg);
        let _ = config::save_app_config(&cfg_clone);
    }

    let config = state.app_config.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let outbounds = state.outbounds.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let active_tag = config.active_nodes.get("proxy").cloned();

    let singbox_cfg = subscription::build_singbox_config(
        &outbounds,
        &config,
        active_tag.as_deref(),
        &nodes,
        subscription::host_has_ipv6(),
    );
    let config_path = config::singbox_config_path();
    config::ensure_dirs().map_err(|e| e.to_string())?;
    let cfg_json = serde_json::to_string_pretty(&singbox_cfg).map_err(|e| e.to_string())?;
    // write_atomic, not fs::write: this file carries every node's credentials plus the
    // Clash API secret, so it must be owner-only (0600) on Unix like the other secret files.
    config::write_atomic(&config_path, cfg_json.as_bytes()).map_err(|e| e.to_string())?;

    // Restart: the running instance uses the PREVIOUS tun setting, so a graceful
    // (adapter-cleanup) shutdown is only needed when it was previously in TUN mode.
    if running {
        let _ = stop_singbox(state.singbox_state.clone(), current_tun).await;
    }

    // Remove any leftover TUN adapter now that the previous core is stopped (so it is not
    // in use). Running this BEFORE the stop would no-op against an adapter the old core
    // still holds, leaving a conflicting adapter for the new core. Cold-path, TUN-only.
    if want_tun {
        crate::tun::cleanup_stale_tun_adapter().await;
    }

    start_singbox(app_handle, &config_path, state.singbox_state.clone(), config.api_port, want_tun)
        .await
        .map_err(|e| e.to_string())?;

    // Make app config the source of truth for routing mode (the core may have a stale
    // value cached from a previous session — cache_file persists the last mode and the
    // core boots into it, ignoring default_mode). Verify-and-retry so the live mode can't
    // silently diverge from config.proxy_mode; log (don't fail the bring-up) if it never
    // takes, since the core itself is already up.
    if let Some(m) = clash_mode_str(&config.proxy_mode) {
        if let Err(e) = clash_set_mode_verified(config.api_port, m).await {
            log::warn!("核心启动后同步代理模式失败：{}", e);
        }
    }

    Ok(())
}

fn clash_mode_str(mode: &ProxyMode) -> Option<&'static str> {
    match mode {
        ProxyMode::Global => Some("Global"),
        ProxyMode::Direct => Some("Direct"),
        ProxyMode::Rule => Some("Rule"),
        ProxyMode::Tun => None,
    }
}

/// Start the persistent core in its idle state (mixed inbound only, no TUN, no system
/// proxy). Called on app launch when there is no proxy state to restore. Idempotent:
/// no-op if the core is already running in non-TUN mode.
pub async fn start_idle_core(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    ensure_core(app_handle, state, false).await
}

#[tauri::command]
pub async fn cmd_start_singbox(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Map the legacy "start" entry point onto the persistent model: enable the proxy
    // in the mode implied by the saved config (TUN if configured, else system proxy).
    let want_tun = state.app_config.lock().unwrap_or_else(|e| e.into_inner()).tun_enabled;
    apply_connection_mode(&app_handle, &state, if want_tun { "tun" } else { "system" }).await
}

#[tauri::command]
pub async fn cmd_stop_singbox(
    state: State<'_, AppState>,
) -> Result<(), String> {
    // "Stop" in the dashboard sense = turn proxying off. The core keeps running idle so
    // re-enabling is instant; full teardown happens only on app exit (see lib.rs).
    // Clearing the system proxy is enough for a system-proxy session; a TUN session
    // additionally needs the core rebuilt to idle, which the dashboard does via
    // setConnectionMode("off") → apply_connection_mode. Here we just clear + persist.
    clear_system_proxy_logged();
    {
        let mut cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        cfg.last_proxy_running = false;
        cfg.last_system_proxy = false;
        let cfg_clone = cfg.clone();
        drop(cfg);
        let _ = config::save_app_config(&cfg_clone);
    }
    Ok(())
}

/// Unified connection control shared by the dashboard and the tray menu. `mode` is one of:
///   • "off"    → clear the system proxy; if in TUN, drop the core back to idle. The core
///                keeps running (idle) so the next enable is instant.
///   • "system" → ensure an idle (non-TUN) core, then point the Windows system proxy at it.
///   • "tun"    → rebuild + restart the core with the TUN inbound; clear the system proxy.
/// On failure the TUN flag is rolled back so persisted state never diverges from reality.
pub async fn apply_connection_mode(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    mode: &str,
) -> Result<(), String> {
    // Hold the core lifecycle lock for the WHOLE switch so a concurrent tray/dashboard/
    // startup mode change can't interleave its own stop/start with ours (see core_lock).
    // Inside this guard we must call `ensure_core_inner` (already-locked variant), never
    // `ensure_core`, or the non-reentrant mutex would deadlock.
    let _guard = state.core_lock.lock().await;

    if mode == "off" {
        clear_system_proxy_logged();
        // If TUN is active, return the core to the idle mixed-only state.
        let in_tun = {
            let s = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner());
            s.running && s.tun_mode
        };
        if in_tun {
            // Best-effort: failing to rebuild idle still leaves us with proxy cleared.
            let _ = ensure_core_inner(app_handle, state, false).await;
        }
        let mut cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        cfg.last_proxy_running = false;
        cfg.last_system_proxy = false;
        let cfg_clone = cfg.clone();
        drop(cfg);
        let _ = config::save_app_config(&cfg_clone);
        return Ok(());
    }

    let want_tun = mode == "tun";
    let prev_tun = state.app_config.lock().unwrap_or_else(|e| e.into_inner()).tun_enabled;

    // Ensure the core is up in the right mode (instant when only the system proxy changes).
    if let Err(e) = ensure_core_inner(app_handle, state, want_tun).await {
        // Roll back the TUN flag if ensure_core changed it but couldn't start.
        if state.app_config.lock().unwrap_or_else(|e| e.into_inner()).tun_enabled != prev_tun {
            let mut cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
            cfg.tun_enabled = prev_tun;
            let cfg_clone = cfg.clone();
            drop(cfg);
            let _ = config::save_app_config(&cfg_clone);
        }
        // Fail SAFE, not dangerous: distinguish "early failure, old core untouched" (TUN
        // precondition missing — the previous session keeps working, leave it alone) from
        // "failed after the old core was already stopped" (new core wouldn't start). In the
        // latter case the OS proxy may still point at the now-dead mixed port — a machine-wide
        // black hole until the user toggles again. Clear it, persist the off-state so a
        // restart doesn't restore a broken session, and best-effort bring back an idle core.
        let core_running = state
            .singbox_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .running;
        if !core_running {
            clear_system_proxy_logged();
            {
                let mut cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
                cfg.last_proxy_running = false;
                cfg.last_system_proxy = false;
                let cfg_clone = cfg.clone();
                drop(cfg);
                let _ = config::save_app_config(&cfg_clone);
            }
            let _ = ensure_core_inner(app_handle, state, false).await;
        }
        return Err(e);
    }

    // Apply the routing path: system proxy ON for "system", cleared for "tun".
    let enable_sys_proxy = !want_tun;
    if enable_sys_proxy {
        let (port, global_mode) = {
            let cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
            (cfg.mixed_port, cfg.proxy_mode == ProxyMode::Global)
        };
        proxy::set_system_proxy(true, port, global_mode).map_err(|e| e.to_string())?;
    } else {
        clear_system_proxy_logged();
    }

    {
        let mut cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        cfg.last_proxy_running = true;
        cfg.last_system_proxy = enable_sys_proxy;
        let cfg_clone = cfg.clone();
        drop(cfg);
        let _ = config::save_app_config(&cfg_clone);
    }
    Ok(())
}

/// TUN self-heal: replays the manual "toggle TUN off then on" that fixes a black-holed tunnel
/// (TUN shows "on", 0 connections, 0 B) caused by a freshly created tunnel layering onto stale
/// routes left by a force-killed predecessor core. The graceful "off" runs sing-box's own
/// auto_route/strict_route teardown, and the "on" rebuilds the tunnel on a converged table.
///
/// ⚠️ MUST be called from a command / UI-thread context (e.g. a Tauri command), NEVER from a
/// background task such as the startup restore block. The graceful stop uses `send_ctrl_c`,
/// which broadcasts CTRL_C_EVENT to the console group; from a background context that self-kills
/// the GUI (the same crash that forced the updater onto a force kill — see updater.rs and the
/// note in lib.rs restore). It is currently UNUSED, kept as the building block for a future
/// frontend-triggered "reconnect / repair" action that runs in the safe context.
#[allow(dead_code)]
pub async fn heal_tun_after_upgrade(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    // Drop the restored TUN core back to idle: this is the route-clearing half of the manual
    // off→on. Graceful teardown (Ctrl+C / SIGTERM) lets sing-box remove the routes it set.
    apply_connection_mode(app_handle, state, "off").await?;
    // Let Windows settle the routing table before re-creating the tunnel.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    // Bring TUN back up on the now-clean table.
    apply_connection_mode(app_handle, state, "tun").await
}

/// Dashboard / unified entry point for switching the connection mode. Delegates to
/// `apply_connection_mode` so the dashboard and the tray menu share identical logic.
#[tauri::command]
pub async fn cmd_set_connection_mode(
    app_handle: tauri::AppHandle,
    mode: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    apply_connection_mode(&app_handle, &state, &mode).await
}

/// Full teardown of the core for app exit. Stops the process and clears the system proxy.
///
/// NOTE: this clears the OS system proxy but deliberately does NOT touch `last_proxy_running`
/// / `last_system_proxy`. Those persist the last *user-intended* state so the next launch can
/// restore it (see the restore block in lib.rs). Clearing them here would make every clean
/// exit look like "user turned proxy off" and silently break restore-on-startup — do not "fix"
/// this by persisting an off-state on exit.
pub async fn shutdown_core(state: &AppState) {
    // Refuse further core starts, then serialize with any in-flight mode switch via
    // core_lock. Without the lock, exiting mid-switch reads `running=false` in the window
    // between the switch's stop and start, returns immediately, and the switch then spawns
    // a core that outlives the exiting process (see `shutting_down`).
    state.shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
    let _guard = state.core_lock.lock().await;
    let graceful = {
        let s = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner());
        s.running && s.tun_mode
    };
    let _ = stop_singbox(state.singbox_state.clone(), graceful).await;
    clear_system_proxy_logged();
}

/// Like `shutdown_core` but ALWAYS force-kills the core — never the graceful Ctrl+C path.
/// Used by the in-app updater. The graceful TUN stop calls `send_ctrl_c`, which broadcasts a
/// console CTRL_C_EVENT to sing-box's whole process group; on the updater's worker thread that
/// was terminating THIS GUI process mid-teardown (observed: the app "crashed" right after the
/// "download done" log line, before the installer ever launched). We don't need a graceful core
/// stop during an update anyway — the updater removes the TUN adapter deterministically via
/// `tun::cleanup_stale_tun_adapter` — so a force kill is both safe and avoids self-termination.
pub async fn shutdown_core_forced(state: &AppState) {
    // Same shutdown ordering as `shutdown_core` (flag + core_lock) — see the note there.
    state.shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
    let _guard = state.core_lock.lock().await;
    let _ = stop_singbox(state.singbox_state.clone(), false).await;
    clear_system_proxy_logged();
}

#[tauri::command]
pub fn cmd_get_singbox_status(
    state: State<'_, AppState>,
) -> SingboxStatus {
    let s = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner());
    SingboxStatus {
        running: s.running,
        tun_active: s.running && s.tun_mode,
        uptime: s.start_time.map(|t| t.elapsed().as_secs()),
        pid: s.pid,
        version: s.version.clone(),
    }
}

#[tauri::command]
pub fn cmd_get_logs(state: State<'_, AppState>) -> Vec<String> {
    state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).logs.clone()
}

/// Export the current in-memory logs to a timestamped `.log` file under the app data
/// dir's `logs/` folder and return the absolute path. The frontend reveals it via the
/// opener plugin. Returns an error if there is nothing to export.
#[tauri::command]
pub fn cmd_export_logs(state: State<'_, AppState>) -> Result<String, String> {
    let logs = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).logs.clone();
    if logs.is_empty() {
        return Err("暂无日志可导出".to_string());
    }
    let dir = config::app_data_dir().join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let filename = format!("singbox-{}.log", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let path = dir.join(filename);
    std::fs::write(&path, logs.join("\n")).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

/// Return the path the frontend should reveal to land the user in the log directory
/// (`app_data_dir/logs/`). Finding it by hand is easy to describe on Windows but buried on
/// macOS (~/Library/Application Support/Skylark/logs), hence a one-click jump in Settings.
///
/// Prefers the most recently modified `.log` FILE inside the directory: `revealItemInDir`
/// on a file opens the logs folder itself with that file selected, whereas revealing the
/// directory only selects it in its PARENT. Falls back to the (freshly created, if needed)
/// directory when no log file exists yet.
#[tauri::command]
pub fn cmd_get_log_dir_entry() -> Result<String, String> {
    let dir = config::app_data_dir().join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let newest_log = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.path().extension().and_then(|x| x.to_str()) == Some("log")
                && e.file_type().map(|t| t.is_file()).unwrap_or(false)
        })
        .max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });
    let target = newest_log.map(|e| e.path()).unwrap_or(dir);
    Ok(target.to_string_lossy().to_string())
}

// ─── Config backup / restore ────────────────────────────────────────

/// Marker so import can reject unrelated JSON files.
const CONFIG_BUNDLE_FORMAT: &str = "skylark-config";

/// Build the full-setup bundle (settings, subscriptions + raw content, nodes, outbounds,
/// proxy groups, routing rules). Shared by config export and named profiles. The Clash
/// API secret is intentionally excluded — it is machine-local and regenerated on demand.
fn build_config_bundle(state: &AppState) -> Value {
    let app_config = state.app_config.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let subscriptions = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let outbounds = state.outbounds.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let proxy_groups = config::load_proxy_groups();
    let rules = crate::rules::load_rules();

    // Bundle each subscription's cached raw text so text-imported subs survive a restore.
    let mut contents = serde_json::Map::new();
    for sub in &subscriptions {
        if let Some(text) = config::load_subscription_content(&sub.id) {
            contents.insert(sub.id.clone(), Value::String(text));
        }
    }

    serde_json::json!({
        "format": CONFIG_BUNDLE_FORMAT,
        "version": 1,
        "exported_at": chrono::Local::now().to_rfc3339(),
        "app_version": env!("CARGO_PKG_VERSION"),
        "app_config": app_config,
        "subscriptions": subscriptions,
        "nodes": nodes,
        "outbounds": outbounds,
        "proxy_groups": proxy_groups,
        "rules": rules,
        "subscription_contents": Value::Object(contents),
    })
}

/// Merge an incoming (frontend snapshot / imported bundle) `AppConfig` with the backend's
/// authoritative runtime fields. `last_proxy_running`, `last_system_proxy` and
/// `last_app_version` are owned by the backend — they're written on proxy start/stop and on
/// upgrade detection — and the frontend only ever holds a stale snapshot of them. A config
/// save or import must therefore NOT overwrite them, or the "restore proxy on startup" state
/// gets clobbered and recovery silently stops working. See docs/proxy-tun-lifecycle-plan.md.
///
/// `preserve_selection` additionally keeps `active_nodes` / `selected_subscription` from the
/// backend. Those are selection state written by `cmd_set_active_node` / `cmd_set_auto_node`
/// (and mirrored to the frontend via `fetchConfig`), NOT settings-form fields — a settings
/// save carries a possibly-stale snapshot of them, and letting it win silently reverts the
/// user's just-made node selection. A config IMPORT/restore, by contrast, must adopt the
/// bundle's selection, so it passes `false`.
fn merge_runtime_fields(
    incoming: AppConfig,
    current: &AppConfig,
    preserve_selection: bool,
) -> AppConfig {
    let mut merged = AppConfig {
        last_proxy_running: current.last_proxy_running,
        last_system_proxy: current.last_system_proxy,
        last_app_version: current.last_app_version.clone(),
        ..incoming
    };
    if preserve_selection {
        merged.active_nodes = current.active_nodes.clone();
        merged.selected_subscription = current.selected_subscription.clone();
    }
    merged
}

/// Apply a config bundle to both on-disk files and the in-memory `AppState`. Each section
/// is applied tolerantly (a bad section is skipped, not fatal). Shared by config import
/// and profile loading. Takes effect on the next core (re)start.
fn apply_config_bundle(bundle: &Value, state: &AppState) -> Result<(), String> {
    if bundle["format"] != CONFIG_BUNDLE_FORMAT {
        return Err("不是 Skylark 配置备份文件".to_string());
    }
    if let Some(v) = bundle.get("app_config") {
        if let Ok(cfg) = serde_json::from_value::<AppConfig>(v.clone()) {
            let mut guard = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
            // Import/restore: adopt the bundle's selection state.
            let merged = merge_runtime_fields(cfg, &guard, false);
            config::save_app_config(&merged).map_err(|e| e.to_string())?;
            *guard = merged;
        }
    }
    if let Some(v) = bundle.get("subscriptions") {
        if let Ok(subs) = serde_json::from_value::<Vec<Subscription>>(v.clone()) {
            config::save_subscriptions(&subs).map_err(|e| e.to_string())?;
            *state.subscriptions.lock().unwrap_or_else(|e| e.into_inner()) = subs;
        }
    }
    if let Some(v) = bundle.get("nodes") {
        if let Ok(ns) = serde_json::from_value::<Vec<ProxyNode>>(v.clone()) {
            config::save_nodes(&ns).map_err(|e| e.to_string())?;
            *state.nodes.lock().unwrap_or_else(|e| e.into_inner()) = ns;
        }
    }
    if let Some(v) = bundle.get("outbounds") {
        if let Ok(obs) = serde_json::from_value::<Vec<Value>>(v.clone()) {
            config::save_outbounds(&obs).map_err(|e| e.to_string())?;
            *state.outbounds.lock().unwrap_or_else(|e| e.into_inner()) = obs;
        }
    }
    if let Some(v) = bundle.get("proxy_groups") {
        if let Ok(groups) = serde_json::from_value::<Vec<ProxyGroup>>(v.clone()) {
            config::save_proxy_groups(&groups).map_err(|e| e.to_string())?;
        }
    }
    if let Some(v) = bundle.get("rules") {
        if let Ok(rules) = serde_json::from_value::<Vec<crate::rules::RouteRule>>(v.clone()) {
            crate::rules::save_rules(&rules).map_err(|e| e.to_string())?;
        }
    }
    if let Some(Value::Object(map)) = bundle.get("subscription_contents") {
        for (id, text) in map {
            if let Some(s) = text.as_str() {
                let _ = config::save_subscription_content(id, s);
            }
        }
    }

    // An imported bundle may predate the `subscription_id` outbound key (or come from another
    // install). Backfill ownership from the paired node and de-duplicate tags globally so the
    // restored data is internally consistent before the next core start — mirrors the startup
    // migration in `lib.rs`. Locks nodes then outbounds (the module-wide order).
    {
        let sub_order: Vec<String> = state
            .subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let mut nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let mut outbounds = state.outbounds.lock().unwrap_or_else(|e| e.into_inner());
        if outbounds.iter().any(|ob| ob.get("subscription_id").is_none()) {
            let owner: std::collections::HashMap<String, Option<String>> = nodes
                .iter()
                .map(|n| (n.name.clone(), n.subscription_id.clone()))
                .collect();
            for ob in outbounds.iter_mut() {
                if ob.get("subscription_id").is_some() {
                    continue;
                }
                let sid = ob
                    .get("tag")
                    .and_then(|t| t.as_str())
                    .and_then(|t| owner.get(t))
                    .and_then(|s| s.clone());
                if let (Some(sid), Some(map)) = (sid, ob.as_object_mut()) {
                    map.insert("subscription_id".into(), Value::String(sid));
                }
            }
            outbounds.retain(|ob| ob.get("subscription_id").is_some());
        }
        subscription::dedupe_tags_globally(&mut nodes, &mut outbounds, &sub_order);
        config::save_nodes(&nodes).map_err(|e| e.to_string())?;
        config::save_outbounds(&outbounds).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Directory holding named config profiles.
fn profiles_dir() -> std::path::PathBuf {
    config::app_data_dir().join("profiles")
}

/// Validate a user-supplied profile name: non-empty and free of path separators / traversal
/// so it can only ever map to a file directly inside `profiles/`.
///
/// Beyond `/ \ ..`, Windows needs more: a `:` makes `PathBuf::join("C:evil.json")` a
/// drive-relative path that REPLACES the profiles dir entirely (escaping it), or an NTFS
/// alternate data stream; `* ? " < > |` and control chars are invalid in filenames; device
/// names (CON, NUL, COM1…) are reserved even with an extension; trailing dots/spaces are
/// silently stripped by the filesystem, aliasing two "different" names to one file.
fn sanitize_profile_name(name: &str) -> Option<String> {
    let n = name.trim();
    if n.is_empty() || n.len() > 64 || n.contains("..") {
        return None;
    }
    if n.chars().any(|c| {
        matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
    }) {
        return None;
    }
    if n.ends_with('.') {
        return None;
    }
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = n.split('.').next().unwrap_or(n);
    if RESERVED.iter().any(|r| stem.eq_ignore_ascii_case(r)) {
        return None;
    }
    Some(n.to_string())
}

/// Export the full user setup to a single timestamped JSON file under the app data dir's
/// `backups/` folder and return the absolute path. The frontend reveals it via the opener
/// plugin.
#[tauri::command]
pub fn cmd_export_config(state: State<'_, AppState>) -> Result<String, String> {
    let bundle = build_config_bundle(state.inner());

    let dir = config::app_data_dir().join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let filename = format!(
        "skylark-config-{}.json",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );
    let path = dir.join(filename);
    let data = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;
    // write_atomic: the bundle embeds full node credentials — owner-only on Unix.
    config::write_atomic(&path, data.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

/// Restore a configuration bundle produced by `cmd_export_config` from pasted JSON text.
/// Each section is applied independently and tolerantly — a malformed/missing section is
/// skipped rather than aborting the whole restore — and both the on-disk files and the
/// in-memory `AppState` are updated so the UI reflects the change without a relaunch.
/// Takes effect on the next core (re)start. Ports/secret unaffected mid-session.
#[tauri::command]
pub fn cmd_import_config(content: String, state: State<'_, AppState>) -> Result<(), String> {
    let bundle: Value = serde_json::from_str(&content)
        .map_err(|_| "无效的备份文件（JSON 解析失败）".to_string())?;
    apply_config_bundle(&bundle, state.inner())
}

// ─── Config profiles (N6) ───────────────────────────────────────────

/// List saved profile names (the `.json` files in `profiles/`), sorted.
#[tauri::command]
pub fn cmd_list_profiles() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(profiles_dir()) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

/// Snapshot the current full setup into a named profile (overwrites if it exists).
#[tauri::command]
pub fn cmd_save_profile(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let name = sanitize_profile_name(&name).ok_or("无效的配置名".to_string())?;
    let dir = profiles_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bundle = build_config_bundle(state.inner());
    let data = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;
    // write_atomic: the bundle embeds full node credentials — owner-only on Unix.
    config::write_atomic(&dir.join(format!("{}.json", name)), data.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Load a named profile, applying it to disk + in-memory state. Takes effect on the next
/// core (re)start (the frontend refreshes its state afterwards).
#[tauri::command]
pub fn cmd_load_profile(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let name = sanitize_profile_name(&name).ok_or("无效的配置名".to_string())?;
    let path = profiles_dir().join(format!("{}.json", name));
    let content = std::fs::read_to_string(&path).map_err(|_| "配置不存在".to_string())?;
    let bundle: Value = serde_json::from_str(&content).map_err(|_| "配置文件损坏".to_string())?;
    apply_config_bundle(&bundle, state.inner())
}

#[tauri::command]
pub fn cmd_delete_profile(name: String) -> Result<(), String> {
    let name = sanitize_profile_name(&name).ok_or("无效的配置名".to_string())?;
    let _ = std::fs::remove_file(profiles_dir().join(format!("{}.json", name)));
    Ok(())
}

// ─── Subscriptions ──────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_subscriptions(state: State<'_, AppState>) -> Vec<Subscription> {
    state.subscriptions.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Replace one subscription's nodes + outbounds in the global lists, keyed by
/// `subscription_id`, then globally de-duplicate tags so the generated sing-box config has
/// unique outbound tags and each node's name still equals its outbound tag. Persists both.
///
/// Nodes and outbounds are ALWAYS added/removed together, keyed the same way — the previous
/// code keyed nodes by `subscription_id` but outbounds by tag string, which desynced the two
/// whenever two subscriptions shipped a node of the same name (kept two nodes but one
/// outbound → a node silently dialing another subscription's server). `sub_order` is the
/// subscriptions' stable display order, so cross-subscription collisions resolve to the same
/// renamed tag on every refresh. Locks nodes then outbounds (the module-wide order).
pub fn merge_subscription_entries(
    state: &AppState,
    sub_order: &[String],
    id: &str,
    nodes: Vec<ProxyNode>,
    outbounds: Vec<Value>,
) -> Result<(), String> {
    let mut all_nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
    let mut all_outbounds = state.outbounds.lock().unwrap_or_else(|e| e.into_inner());

    all_nodes.retain(|n| n.subscription_id.as_deref() != Some(id));
    all_nodes.extend(nodes);
    all_outbounds.retain(|ob| {
        ob.get("subscription_id").and_then(|v| v.as_str()) != Some(id)
    });
    all_outbounds.extend(outbounds);

    subscription::dedupe_tags_globally(&mut all_nodes, &mut all_outbounds, sub_order);

    config::save_nodes(&all_nodes).map_err(|e| e.to_string())?;
    config::save_outbounds(&all_outbounds).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn cmd_add_subscription(
    name: String,
    url: String,
    include: Option<String>,
    exclude: Option<String>,
    group_by_region: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Subscription, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let group_by_region = group_by_region.unwrap_or(false);
    let proxy_port = active_proxy_port(&state);
    let (content, userinfo) = fetch_url(&url, proxy_port).await.map_err(|e| e.to_string())?;
    config::save_subscription_content(&id, &content).map_err(|e| e.to_string())?;
    let sub_type = subscription::detect_sub_type(&content, &url);
    let (nodes, outbounds) = subscription::parse_subscription(&content, &id)
        .map_err(|e| e.to_string())?;
    let (nodes, outbounds) = subscription::apply_node_filters(
        nodes, outbounds, include.as_deref(), exclude.as_deref(), group_by_region,
    );

    let sub = Subscription {
        id: id.clone(),
        name,
        url,
        sub_type,
        node_count: nodes.len(),
        last_update: Some(chrono::Utc::now()),
        auto_update: true,
        update_interval: 24,
        upload: userinfo.upload,
        download: userinfo.download,
        total: userinfo.total,
        expire: userinfo.expire,
        include,
        exclude,
        group_by_region,
    };

    let sub_order = {
        let mut subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
        subs.push(sub.clone());
        config::save_subscriptions(&subs).map_err(|e| e.to_string())?;
        subs.iter().map(|s| s.id.clone()).collect::<Vec<_>>()
    };
    merge_subscription_entries(&state, &sub_order, &id, nodes, outbounds)?;

    Ok(sub)
}

/// Import a subscription from pasted text (single node links / Base64 / Clash YAML /
/// SIP008) instead of a remote URL. Persisted as a local subscription with no URL and
/// auto-update disabled — there is no remote source to re-fetch from.
#[tauri::command]
pub async fn cmd_import_subscription_from_text(
    name: String,
    content: String,
    include: Option<String>,
    exclude: Option<String>,
    group_by_region: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Subscription, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let group_by_region = group_by_region.unwrap_or(false);
    let (nodes, outbounds) = subscription::parse_subscription(&content, &id)
        .map_err(|e| e.to_string())?;
    let (nodes, outbounds) = subscription::apply_node_filters(
        nodes, outbounds, include.as_deref(), exclude.as_deref(), group_by_region,
    );
    let sub_type = subscription::detect_sub_type(&content, "");
    config::save_subscription_content(&id, &content).map_err(|e| e.to_string())?;

    let sub = Subscription {
        id: id.clone(),
        name,
        url: String::new(),
        sub_type,
        node_count: nodes.len(),
        last_update: Some(chrono::Utc::now()),
        auto_update: false,
        update_interval: 0,
        upload: None,
        download: None,
        total: None,
        expire: None,
        include,
        exclude,
        group_by_region,
    };

    let sub_order = {
        let mut subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
        subs.push(sub.clone());
        config::save_subscriptions(&subs).map_err(|e| e.to_string())?;
        subs.iter().map(|s| s.id.clone()).collect::<Vec<_>>()
    };
    merge_subscription_entries(&state, &sub_order, &id, nodes, outbounds)?;

    Ok(sub)
}

#[tauri::command]
pub async fn cmd_update_subscription(
    id: String,
    state: State<'_, AppState>,
) -> Result<Subscription, String> {
    let (url, include, exclude, group_by_region) = {
        let subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
        subs.iter()
            .find(|s| s.id == id)
            .map(|s| (s.url.clone(), s.include.clone(), s.exclude.clone(), s.group_by_region))
            .ok_or_else(|| "订阅不存在".to_string())?
    };

    let proxy_port = active_proxy_port(&state);
    let (content, userinfo) = fetch_url(&url, proxy_port).await.map_err(|e| e.to_string())?;
    let sub_type = subscription::detect_sub_type(&content, &url);
    let (nodes, outbounds) = subscription::parse_subscription(&content, &id)
        .map_err(|e| e.to_string())?;
    let (nodes, outbounds) = subscription::apply_node_filters(
        nodes, outbounds, include.as_deref(), exclude.as_deref(), group_by_region,
    );
    // A successful fetch that parses to zero nodes (an error/notice page, an expired account,
    // or an all-placeholder body) must not wipe the last-known working nodes or clobber the
    // good cached content. Bail with no side effects; the existing set stays selectable.
    if nodes.is_empty() {
        return Err("订阅刷新未解析到任何节点，已保留原有节点".to_string());
    }
    config::save_subscription_content(&id, &content).map_err(|e| e.to_string())?;

    let (updated_sub, sub_order) = {
        let mut subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
        let sub = subs.iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| "订阅不存在".to_string())?;
        sub.sub_type = sub_type;
        sub.node_count = nodes.len();
        sub.last_update = Some(chrono::Utc::now());
        // Only overwrite quota fields when the provider actually returned them, so a
        // missing header on one refresh doesn't wipe previously-known usage.
        if userinfo.upload.is_some() { sub.upload = userinfo.upload; }
        if userinfo.download.is_some() { sub.download = userinfo.download; }
        if userinfo.total.is_some() { sub.total = userinfo.total; }
        if userinfo.expire.is_some() { sub.expire = userinfo.expire; }
        let cloned = sub.clone();
        config::save_subscriptions(&subs).map_err(|e| e.to_string())?;
        (cloned, subs.iter().map(|s| s.id.clone()).collect::<Vec<_>>())
    };

    merge_subscription_entries(&state, &sub_order, &id, nodes, outbounds)?;

    Ok(updated_sub)
}

#[tauri::command]
pub fn cmd_save_subscription_settings(
    id: String,
    auto_update: bool,
    update_interval: u32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
    let sub = subs.iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| "订阅不存在".to_string())?;
    sub.auto_update = auto_update;
    sub.update_interval = update_interval;
    config::save_subscriptions(&subs).map_err(|e| e.to_string())
}

/// Update a subscription's node-name filters / region grouping and re-apply them to the
/// already-cached content (no network fetch). Returns the new node count.
#[tauri::command]
pub fn cmd_set_subscription_filters(
    id: String,
    include: Option<String>,
    exclude: Option<String>,
    group_by_region: bool,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    // Re-parse from the cached subscription content so a filter edit takes effect offline.
    let content = config::load_subscription_content(&id)
        .ok_or_else(|| "订阅内容缓存不存在，请先更新订阅".to_string())?;
    let (nodes, outbounds) = subscription::parse_subscription(&content, &id)
        .map_err(|e| e.to_string())?;
    let (nodes, outbounds) = subscription::apply_node_filters(
        nodes, outbounds, include.as_deref(), exclude.as_deref(), group_by_region,
    );
    let node_count = nodes.len();

    let sub_order = {
        let mut subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
        let sub = subs.iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| "订阅不存在".to_string())?;
        sub.include = include;
        sub.exclude = exclude;
        sub.group_by_region = group_by_region;
        sub.node_count = node_count;
        config::save_subscriptions(&subs).map_err(|e| e.to_string())?;
        subs.iter().map(|s| s.id.clone()).collect::<Vec<_>>()
    };

    merge_subscription_entries(&state, &sub_order, &id, nodes, outbounds)?;

    Ok(node_count)
}

#[tauri::command]
pub fn cmd_delete_subscription(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    config::delete_subscription_content(&id);
    {
        let mut subs = state.subscriptions.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|s| s.id != id);
        config::save_subscriptions(&subs).map_err(|e| e.to_string())?;
    }
    // Remove the subscription's nodes and outbounds together, both keyed by
    // `subscription_id` (outbounds carry it too — see `subscription::parse_subscription`).
    // No global re-dedup here: removing entries can never create a tag collision, and
    // leaving surviving nodes' names untouched keeps active selection stable.
    {
        let mut nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let mut outbounds = state.outbounds.lock().unwrap_or_else(|e| e.into_inner());
        nodes.retain(|n| n.subscription_id.as_deref() != Some(&id));
        outbounds.retain(|ob| {
            ob.get("subscription_id").and_then(|v| v.as_str()) != Some(id.as_str())
        });
        config::save_nodes(&nodes).map_err(|e| e.to_string())?;
        config::save_outbounds(&outbounds).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ─── Nodes ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_nodes(state: State<'_, AppState>) -> Vec<ProxyNode> {
    state.nodes.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
pub async fn cmd_test_node_latency(
    node_id: String,
    state: State<'_, AppState>,
) -> Result<u32, String> {
    let (name, server, port, api_port, test_url, is_running) = {
        let nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let node = nodes.iter().find(|n| n.id == node_id)
            .ok_or_else(|| "节点不存在".to_string())?;
        let cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
        let url = if cfg.auto_test_url.trim().is_empty() {
            "https://www.gstatic.com/generate_204".to_string()
        } else {
            cfg.auto_test_url.trim().to_string()
        };
        (node.name.clone(), node.server.clone(), node.port, cfg.api_port, url, running)
    };

    let latency_ms = if is_running {
        // Proxy is running: measure THIS specific node via the Clash delay API. Routing
        // through the local mixed port would only ever exercise the currently selected
        // proxy, so every node would report the same latency.
        clash_proxy_delay(api_port, &name, &test_url).await
    } else {
        // Proxy not running: TCP connect to the server itself
        test_tcp_connect(&server, port).await
    };

    match latency_ms {
        Some(ms) => {
            let mut nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(node) = nodes.iter_mut().find(|n| n.id == node_id) {
                node.latency = Some(ms);
            }
            let _ = config::save_nodes(&nodes);
            Ok(ms)
        }
        None => {
            let mut nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(node) = nodes.iter_mut().find(|n| n.id == node_id) {
                node.latency = None;
            }
            let _ = config::save_nodes(&nodes);
            Err("超时".to_string())
        }
    }
}

/// Measure a single node's latency through the running core via the Clash API
/// `GET /proxies/{name}/delay`. This probes the SPECIFIC node regardless of which
/// proxy is currently selected — unlike routing a request through the local mixed
/// port, which only ever exercises the active selection. The node name is appended
/// as a path segment so names with spaces / unicode / slashes are encoded safely.
/// Returns None on timeout, error, or an unreachable node.
async fn clash_proxy_delay(api_port: u16, name: &str, test_url: &str) -> Option<u32> {
    let mut endpoint = reqwest::Url::parse(
        &format!("http://127.0.0.1:{}/proxies", api_port)
    ).ok()?;
    if let Ok(mut seg) = endpoint.path_segments_mut() {
        seg.push(name).push("delay");
    }
    endpoint.query_pairs_mut()
        .append_pair("url", test_url)
        .append_pair("timeout", "5000");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        // Control-plane call to the local Clash API — never route it through an
        // env-var HTTP(S)_PROXY the user might have set.
        .no_proxy()
        .build()
        .ok()?;
    let resp = client.get(endpoint)
        .bearer_auth(crate::config::api_secret())
        .send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    body["delay"].as_u64().map(|d| (d as u32).max(1))
}

/// Measure download speed in KB/s by downloading a file through the proxy.
async fn measure_download_speed(mixed_port: u16) -> Option<u32> {
    let proxy_url = format!("http://127.0.0.1:{}", mixed_port);
    // Use Proxy::all so HTTPS CONNECT tunneling works too
    let proxy = reqwest::Proxy::all(&proxy_url).ok()?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()?;
    let start = std::time::Instant::now();
    let resp = client
        .get("https://speed.cloudflare.com/__down?bytes=2097152") // 2 MB
        .send()
        .await
        .ok()?;
    let bytes = resp.bytes().await.ok()?;
    let ms = start.elapsed().as_millis() as u64;
    if ms == 0 || bytes.is_empty() {
        return None;
    }
    // bytes / ms * 1000 = bytes/s → divide by 1024 → KB/s
    Some(((bytes.len() as u64 * 1000 / ms) / 1024) as u32)
}

#[tauri::command]
pub async fn cmd_test_node_speed(
    node_id: String,
    state: State<'_, AppState>,
) -> Result<types::SpeedResult, String> {
    let (name, server, port, mixed_port, api_port, test_url, is_running) = {
        let nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let node = nodes.iter().find(|n| n.id == node_id)
            .ok_or_else(|| "节点不存在".to_string())?;
        let cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
        let url = if cfg.auto_test_url.trim().is_empty() {
            "https://www.gstatic.com/generate_204".to_string()
        } else {
            cfg.auto_test_url.trim().to_string()
        };
        (node.name.clone(), node.server.clone(), node.port, cfg.mixed_port, cfg.api_port, url, running)
    };

    let (latency_ms, download_kbps) = if is_running {
        // Latency probes THIS node via the Clash delay API; download speed is measured
        // through the local mixed port and therefore reflects the currently SELECTED
        // node's throughput (sing-box cannot route a one-off download via an arbitrary
        // node without selecting it). Run both in parallel.
        let (lat, spd) = tokio::join!(
            clash_proxy_delay(api_port, &name, &test_url),
            measure_download_speed(mixed_port),
        );
        (lat, spd)
    } else {
        // Proxy not running: only TCP connect, no meaningful speed
        (test_tcp_connect(&server, port).await, None)
    };

    {
        let mut nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(node) = nodes.iter_mut().find(|n| n.id == node_id) {
            node.latency = latency_ms;
            node.download_speed = download_kbps;
        }
        let _ = config::save_nodes(&nodes);
    }

    Ok(types::SpeedResult { latency_ms, download_kbps })
}

/// Test latency via TCP connect when proxy is not running.
async fn test_tcp_connect(server: &str, port: u16) -> Option<u32> {
    let addr = format!("{}:{}", server, port);
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::TcpStream::connect(&addr),
    ).await;
    match result {
        Ok(Ok(_)) => {
            let micros = start.elapsed().as_micros() as u32;
            // Convert to ms, minimum 1ms to avoid showing 0
            Some((micros / 1000).max(1))
        }
        _ => None,
    }
}

/// Switch a *running* sing-box selector to `name` via the Clash API so the change
/// takes effect immediately without restarting the core. Best-effort: callers
/// persist the choice first, so a later restart still applies it even if this fails.
async fn clash_select_proxy(api_port: u16, group: &str, name: &str) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{}/proxies/{}", api_port, group);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .put(&url)
        .bearer_auth(crate::config::api_secret())
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("Clash API 返回 {}", resp.status()))
    }
}

/// Force an immediate latency re-test of every node in an auto (urltest) group via
/// the Clash API. `group` is "auto" (all nodes) or "auto-<sub.id>" (one
/// subscription). Testing each member updates the core's delay history, which the
/// urltest group then uses to (re)select the fastest node. Best-effort per node.
#[tauri::command]
pub async fn cmd_test_group_delay(
    group: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let (api_port, running, test_url, members) = {
        // Lock order MUST be nodes → app_config (the order every other command that holds both
        // uses, e.g. cmd_test_node_latency / cmd_set_active_node). Locking app_config first here
        // created an ABBA deadlock with those commands under concurrent invocation.
        let nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
        let cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        let members: Vec<String> = if group == "auto" {
            nodes.iter().map(|n| n.name.clone()).collect()
        } else if let Some(sid) = group.strip_prefix("auto-") {
            nodes.iter()
                .filter(|n| n.subscription_id.as_deref() == Some(sid))
                .map(|n| n.name.clone())
                .collect()
        } else {
            vec![group.clone()]
        };
        let url = if cfg.auto_test_url.trim().is_empty() {
            "https://www.gstatic.com/generate_204".to_string()
        } else {
            cfg.auto_test_url.trim().to_string()
        };
        (cfg.api_port, running, url, members)
    };

    if !running {
        return Err("代理未运行".to_string());
    }
    if members.is_empty() {
        return Err("该分组没有可测试的节点".to_string());
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;

    // Cap concurrent delay probes: each one makes the core dial the node, so testing a
    // group of hundreds of nodes all at once would spike CPU / local bandwidth and open a
    // flood of simultaneous connections. A small semaphore keeps it bounded while still
    // finishing quickly.
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(16));
    let mut tasks = Vec::new();
    for tag in members {
        let client = client.clone();
        let url = test_url.clone();
        let sem = sem.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            };
            // Build the endpoint with path-segment encoding so node names containing
            // spaces / unicode / slashes don't corrupt the request path.
            let mut endpoint = match reqwest::Url::parse(&format!(
                "http://127.0.0.1:{}/proxies",
                api_port
            )) {
                Ok(u) => u,
                Err(_) => return,
            };
            if let Ok(mut seg) = endpoint.path_segments_mut() {
                seg.push(&tag).push("delay");
            }
            endpoint
                .query_pairs_mut()
                .append_pair("url", &url)
                .append_pair("timeout", "5000");
            let _ = client.get(endpoint)
                .bearer_auth(crate::config::api_secret())
                .send().await;
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
    Ok(())
}

#[tauri::command]
pub async fn cmd_set_active_node(
    node_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    /* Collect everything that needs locks first, then drop the guards before any
       `.await` — std Mutex guards are not Send and cannot be held across await. */
    let (tag, api_port, running) = {
        let mut nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let mut found_tag = None;
        for node in nodes.iter_mut() {
            if node.id == node_id {
                node.is_active = true;
                found_tag = Some(node.name.clone());
            } else {
                node.is_active = false;
            }
        }
        let tag = found_tag.ok_or_else(|| "节点不存在".to_string())?;
        config::save_nodes(&nodes).map_err(|e| e.to_string())?;

        let mut config = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        config.active_nodes.insert("proxy".to_string(), tag.clone());
        config::save_app_config(&config).map_err(|e| e.to_string())?;
        let api_port = config.api_port;
        let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
        (tag, api_port, running)
    };

    /* Apply live when the core is running so switching is instant (no restart). */
    if running {
        let _ = clash_select_proxy(api_port, "proxy", &tag).await;
    }
    Ok(())
}

/// Rebuild the sing-box config from current app state and restart the running core so the
/// change takes effect. Unlike [`ensure_core`], this ALWAYS rebuilds — for changes that
/// alter config *content* while the TUN mode stays the same (e.g. switching the active
/// auto group, which materialises a different urltest group). Preserves the current TUN
/// mode; the system proxy keeps pointing at the unchanged mixed port, so it needs no
/// retouch. When the core is not running it only rewrites the file for the next start.
async fn rebuild_and_restart_core(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    // App is exiting: never (re)start the core — see AppState::shutting_down.
    if state.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("应用正在退出，已取消本次操作".to_string());
    }
    let (config, outbounds, nodes) = {
        let config = state.app_config.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let outbounds = state.outbounds.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner()).clone();
        (config, outbounds, nodes)
    };
    let active_tag = config.active_nodes.get("proxy").cloned();
    let singbox_cfg = subscription::build_singbox_config(
        &outbounds,
        &config,
        active_tag.as_deref(),
        &nodes,
        subscription::host_has_ipv6(),
    );
    let config_path = config::singbox_config_path();
    config::ensure_dirs().map_err(|e| e.to_string())?;
    let cfg_json = serde_json::to_string_pretty(&singbox_cfg).map_err(|e| e.to_string())?;
    // write_atomic: credentials + API secret inside — keep it 0600 (see ensure_core_inner).
    config::write_atomic(&config_path, cfg_json.as_bytes()).map_err(|e| e.to_string())?;

    let (running, current_tun) = {
        let s = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner());
        (s.running, s.tun_mode)
    };
    if running {
        let _ = stop_singbox(state.singbox_state.clone(), current_tun).await;
        if let Err(e) = start_singbox(
            app_handle,
            &config_path,
            state.singbox_state.clone(),
            config.api_port,
            current_tun,
        )
        .await
        {
            // The old core is already gone and the rebuilt one wouldn't start: the OS proxy
            // would keep pointing at a dead mixed port (machine-wide black hole). Fail safe —
            // clear it and persist the off-state so the next launch doesn't restore a broken
            // session. (No idle-core retry here: the config content itself was rejected, so
            // an identical rebuild would fail the same way.)
            clear_system_proxy_logged();
            let mut cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
            cfg.last_proxy_running = false;
            cfg.last_system_proxy = false;
            let cfg_clone = cfg.clone();
            drop(cfg);
            let _ = config::save_app_config(&cfg_clone);
            return Err(e.to_string());
        }
        if let Some(m) = clash_mode_str(&config.proxy_mode) {
            if let Err(e) = clash_set_mode_verified(config.api_port, m).await {
                log::warn!("核心重建后同步代理模式失败：{}", e);
            }
        }
    }
    Ok(())
}

/// Switch the proxy group to a dynamic urltest selection. `group` defaults to the
/// global "auto" group; pass "auto-<sub.id>" to use a per-subscription group. The
/// core then continuously health-checks that group's nodes and routes via the
/// fastest one.
///
/// Only the active auto group is materialised in the config (so startup probes just that
/// group's nodes), which means the target group may not exist in the running config — a
/// live Clash `select` cannot reach it. So switching auto groups rebuilds the config and
/// restarts the core. (Concrete-node switches stay live; see `cmd_set_active_node`.)
#[tauri::command]
pub async fn cmd_set_auto_node(
    app_handle: tauri::AppHandle,
    group: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let group = group.unwrap_or_else(|| "auto".to_string());
    // Serialize the whole core rebuild through core_lock, like apply_connection_mode does.
    // Without it, this stop+start races a concurrent connection-mode switch (or another auto
    // switch): two uncoordinated cores fight over the Clash API port and the TUN adapter and
    // the second fails its readiness probe. Held across the awaits below (async mutex).
    let _core = state.core_lock.lock().await;
    {
        let mut nodes = state.nodes.lock().unwrap_or_else(|e| e.into_inner());
        for node in nodes.iter_mut() {
            node.is_active = false;
        }
        config::save_nodes(&nodes).map_err(|e| e.to_string())?;

        let mut config = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        config.active_nodes.insert("proxy".to_string(), group.clone());
        config::save_app_config(&config).map_err(|e| e.to_string())?;
    }

    rebuild_and_restart_core(&app_handle, state.inner()).await
}

/// Resolve the concrete node the "proxy" group is currently routing through by
/// following the selector → urltest chain via the Clash API. Returns `None` when
/// the core is not running or the chain cannot be resolved. Used by the UI/tray to
/// show which node an "auto" group actually picked.
#[tauri::command]
pub async fn cmd_get_active_proxy_now(
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let (api_port, running) = {
        let cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
        (cfg.api_port, running)
    };
    if !running {
        return Ok(None);
    }

    let url = format!("http://127.0.0.1:{}/proxies", api_port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url)
        .bearer_auth(crate::config::api_secret())
        .send().await.map_err(|e| e.to_string())?;
    let json: Value = resp.json().await.map_err(|e| e.to_string())?;
    let proxies = match json.get("proxies") {
        Some(p) => p,
        None => return Ok(None),
    };

    /* Walk from "proxy" through any Selector/URLTest layers down to a real node.
       The hop cap prevents an infinite loop on a malformed/cyclic response. */
    let mut cur = "proxy".to_string();
    for _ in 0..6 {
        let node = match proxies.get(&cur) {
            Some(n) => n,
            None => break,
        };
        let typ = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if typ == "Selector" || typ == "URLTest" {
            match node.get("now").and_then(|n| n.as_str()) {
                Some(now) if !now.is_empty() => cur = now.to_string(),
                _ => break,
            }
        } else {
            break;
        }
    }
    Ok(Some(cur))
}

// ─── Config ─────────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_app_config(state: State<'_, AppState>) -> AppConfig {
    state.app_config.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
pub fn cmd_save_app_config(
    new_config: AppConfig,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // The mixed inbound and the Clash API control port are mandatory — a 0 (or the two
    // colliding) makes the core fail to bind / start. http_port / socks_port may be 0
    // (that intentionally disables the dedicated inbound), so they aren't checked here.
    if new_config.mixed_port == 0 {
        return Err("混合端口不能为 0".to_string());
    }
    if new_config.api_port == 0 {
        return Err("控制端口不能为 0".to_string());
    }
    // The control port must not collide with any local inbound port (a 0 http/socks port
    // means that inbound is disabled, so it can't conflict).
    let ap = new_config.api_port;
    if ap == new_config.mixed_port
        || (new_config.http_port != 0 && ap == new_config.http_port)
        || (new_config.socks_port != 0 && ap == new_config.socks_port)
    {
        return Err("控制端口不能与代理端口相同".to_string());
    }
    let mut guard = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
    // Settings-form save: keep the live node selection, don't let a stale snapshot revert it.
    let merged = merge_runtime_fields(new_config, &guard, true);
    config::save_app_config(&merged).map_err(|e| e.to_string())?;
    *guard = merged;
    Ok(())
}

/// Switch the running core's Clash routing mode live via `PATCH /configs`. This takes
/// effect immediately with NO core restart (sing-box re-evaluates routing per the
/// clash_mode rules) and is persisted by the core to its cache file. Best-effort:
/// callers persist the choice to app config first, so a later restart still applies it
/// through `default_mode` even if this live call fails.
async fn clash_set_mode(api_port: u16, mode: &str) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{}/configs", api_port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .patch(&url)
        .bearer_auth(crate::config::api_secret())
        .json(&serde_json::json!({ "mode": mode }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("Clash API 返回 {}", resp.status()))
    }
}

/// Read the core's CURRENT clash routing mode via `GET /configs`. Used to verify that a
/// `clash_set_mode` PATCH actually took effect (see `clash_set_mode_verified`).
async fn clash_get_mode(api_port: u16) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{}/configs", api_port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .bearer_auth(crate::config::api_secret())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Clash API 返回 {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    body.get("mode")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Clash /configs 响应缺少 mode 字段".to_string())
}

/// Set the clash routing mode and CONFIRM it stuck. A plain `clash_set_mode` is
/// fire-and-forget: right after a core (re)start the control port can be up but not yet
/// ready, and — because `cache_file` persists the last mode — the core boots into whatever
/// was cached (often a stale "Direct" from an earlier session). If the lone PATCH is
/// dropped in that window the core keeps the cached mode while app config / the UI show a
/// different one, so e.g. "规则" silently routes every foreign site direct (GFW-blocked).
///
/// This PATCHes then reads back `GET /configs`, retrying a few times across the
/// just-started window, and only returns `Ok` once the core reports the requested mode.
async fn clash_set_mode_verified(api_port: u16, mode: &str) -> Result<(), String> {
    let mut last_err = String::new();
    for attempt in 0..6 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        if let Err(e) = clash_set_mode(api_port, mode).await {
            last_err = e;
            continue;
        }
        match clash_get_mode(api_port).await {
            // sing-box compares clash_mode case-insensitively; match that here.
            Ok(current) if current.eq_ignore_ascii_case(mode) => return Ok(()),
            Ok(current) => last_err = format!("下发 {mode} 后内核仍为 {current}"),
            Err(e) => last_err = e,
        }
    }
    Err(format!("设置代理模式({mode})未生效: {last_err}"))
}

#[tauri::command]
pub async fn cmd_set_proxy_mode(
    mode: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let proxy_mode = match mode.as_str() {
        "rule" => ProxyMode::Rule,
        "global" => ProxyMode::Global,
        "direct" => ProxyMode::Direct,
        "tun" => ProxyMode::Tun,
        _ => return Err(format!("未知模式: {}", mode)),
    };

    let (api_port, running) = {
        let mut config = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        config.proxy_mode = proxy_mode.clone();
        config::save_app_config(&config).map_err(|e| e.to_string())?;
        let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
        (config.api_port, running)
    };

    // Apply live so the switch is instant (no restart). rule/global/direct map to the
    // core's clash_mode; "tun" is a connection mode, not a routing mode, so skip it.
    // Verify the switch actually landed and surface failure to the UI — a silently dropped
    // PATCH used to leave the core in its cached mode while the UI showed the new one (e.g.
    // "规则" but traffic still routed direct). The choice is already persisted above, so a
    // later restart re-applies it regardless.
    if running {
        let clash_mode = match proxy_mode {
            ProxyMode::Global => Some("Global"),
            ProxyMode::Direct => Some("Direct"),
            ProxyMode::Rule => Some("Rule"),
            ProxyMode::Tun => None,
        };
        if let Some(m) = clash_mode {
            clash_set_mode_verified(api_port, m).await?;
        }
    }

    // The Windows system-proxy bypass list depends on whether we're in Global mode, so
    // when the system proxy is currently active (non-TUN) rewrite it to match the new
    // mode right away instead of waiting for the next reconnect. No-op on macOS (no
    // per-domain bypass) and when the system proxy is off.
    let (mixed_port, tun_enabled) = {
        let cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        (cfg.mixed_port, cfg.tun_enabled)
    };
    if !tun_enabled && crate::proxy::get_system_proxy_status() {
        let global_mode = proxy_mode == ProxyMode::Global;
        let _ = crate::proxy::set_system_proxy(true, mixed_port, global_mode);
    }
    Ok(())
}

#[tauri::command]
pub async fn cmd_get_connections(
    state: State<'_, AppState>,
) -> Result<Vec<ConnectionInfo>, String> {
    let port = state.app_config.lock().unwrap_or_else(|e| e.into_inner()).api_port;
    crate::singbox::fetch_connections(port)
        .await
        .map_err(|e| e.to_string())
}

/// Close a single active connection by id.
#[tauri::command]
pub async fn cmd_close_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let port = state.app_config.lock().unwrap_or_else(|e| e.into_inner()).api_port;
    crate::singbox::close_connection(port, &id)
        .await
        .map_err(|e| e.to_string())
}

/// Close all active connections at once.
#[tauri::command]
pub async fn cmd_close_all_connections(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let port = state.app_config.lock().unwrap_or_else(|e| e.into_inner()).api_port;
    crate::singbox::close_all_connections(port)
        .await
        .map_err(|e| e.to_string())
}

/* Cumulative traffic counters reported by the Clash API, in bytes. The core keeps
   these from the moment it starts, so they represent ALL traffic since the proxy
   service started — independent of which UI page is open. They reset to zero each
   time the core restarts. */
#[derive(serde::Serialize, Default)]
pub struct TrafficTotal {
    pub upload: u64,
    pub download: u64,
}

/// Returns total upload/download bytes since the sing-box core started by reading
/// the top-level `uploadTotal`/`downloadTotal` of the Clash `/connections` endpoint.
/// Returns zeros when the core is not running or the query fails.
#[tauri::command]
pub async fn cmd_get_traffic_total(
    state: State<'_, AppState>,
) -> Result<TrafficTotal, String> {
    let (api_port, running) = {
        let cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
        (cfg.api_port, running)
    };
    if !running {
        return Ok(TrafficTotal::default());
    }

    let url = format!("http://127.0.0.1:{}/connections", api_port);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(_) => return Ok(TrafficTotal::default()),
    };

    /* Best-effort: a transient API hiccup should not surface as a hard error in the
       UI — fall back to zeros so the poller simply keeps the last good display. */
    let body: Value = match client.get(&url)
        .bearer_auth(crate::config::api_secret())
        .send().await
    {
        Ok(resp) => match resp.json().await {
            Ok(v) => v,
            Err(_) => return Ok(TrafficTotal::default()),
        },
        Err(_) => return Ok(TrafficTotal::default()),
    };

    Ok(TrafficTotal {
        upload: body["uploadTotal"].as_u64().unwrap_or(0),
        download: body["downloadTotal"].as_u64().unwrap_or(0),
    })
}

/// Accumulate transferred bytes into today's persistent traffic bucket. Called
/// periodically by the frontend traffic monitor with the bytes seen since the last flush.
#[tauri::command]
pub fn cmd_add_traffic_sample(upload: u64, download: u64) {
    crate::stats::record_today(upload, download);
}

/// Recent daily traffic history (oldest-first). `days = None`/0 returns all retained days.
#[tauri::command]
pub fn cmd_get_traffic_history(days: Option<usize>) -> Vec<crate::stats::TrafficDay> {
    crate::stats::history(days.unwrap_or(0))
}

// ─── Network diagnostics (N5) ───────────────────────────────────────

#[derive(serde::Serialize)]
pub struct ProbeResult {
    pub name: String,
    pub ok: bool,
    pub latency_ms: Option<u32>,
}

#[derive(serde::Serialize, Default)]
pub struct DiagnosticsResult {
    pub outbound_ip: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub isp: Option<String>,
    pub probes: Vec<ProbeResult>,
}

/// Extract (ip, country, city, isp) from an ip-api.com JSON response. Pure (no I/O) so it
/// is unit-testable; empty strings map to None.
fn parse_ipapi(body: &Value) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let s = |k: &str| {
        body.get(k)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    (s("query"), s("country"), s("city"), s("isp"))
}

/// Run network diagnostics THROUGH the proxy: resolve the outbound IP / geo / ISP and
/// probe reachability of a few well-known endpoints. Requires the proxy to be running.
#[tauri::command]
pub async fn cmd_run_diagnostics(state: State<'_, AppState>) -> Result<DiagnosticsResult, String> {
    let (mixed_port, running) = {
        let cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
        (cfg.mixed_port, running)
    };
    if !running {
        return Err("代理未运行，无法诊断".to_string());
    }

    let proxy_url = format!("http://127.0.0.1:{}", mixed_port);
    let proxy = reqwest::Proxy::all(&proxy_url).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let mut result = DiagnosticsResult::default();

    // Outbound IP + geo via ip-api (free, no key; restricted to the fields we need).
    if let Ok(resp) = client
        .get("http://ip-api.com/json/?fields=status,country,city,isp,query")
        .send()
        .await
    {
        if let Ok(body) = resp.json::<Value>().await {
            let (ip, country, city, isp) = parse_ipapi(&body);
            result.outbound_ip = ip;
            result.country = country;
            result.city = city;
            result.isp = isp;
        }
    }

    // Reachability probes (204 / redirect / 200 all count as reachable).
    let targets = [
        ("Google", "https://www.google.com/generate_204"),
        ("YouTube", "https://www.youtube.com/generate_204"),
        ("GitHub", "https://github.com"),
        ("Cloudflare", "https://1.1.1.1/cdn-cgi/trace"),
    ];
    for (name, url) in targets {
        let start = std::time::Instant::now();
        let ok = client
            .get(url)
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().is_redirection())
            .unwrap_or(false);
        result.probes.push(ProbeResult {
            name: name.to_string(),
            ok,
            latency_ms: if ok { Some(start.elapsed().as_millis() as u32) } else { None },
        });
    }

    Ok(result)
}

#[tauri::command]
pub fn cmd_parse_subscription_from_text(
    content: String,
    sub_id: String,
) -> Result<Vec<ProxyNode>, String> {
    let (nodes, _) = subscription::parse_subscription(&content, &sub_id)
        .map_err(|e| e.to_string())?;
    Ok(nodes)
}

// ─── TUN / Admin ────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_is_elevated() -> bool {
    crate::tun::is_elevated()
}

#[tauri::command]
pub fn cmd_relaunch_as_admin() -> Result<(), String> {
    crate::tun::relaunch_as_admin().map_err(|e| e.to_string())
}

/// Whether TUN is ready to start without a fresh authorization prompt. On macOS that means
/// the one-time privileged service is installed; on Windows/Linux it means the process is
/// already elevated. Lets the frontend show a single "TUN ready" indicator cross-platform.
#[tauri::command]
pub fn cmd_tun_service_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::tun::tun_service_installed()
    }
    #[cfg(not(target_os = "macos"))]
    {
        crate::tun::is_elevated()
    }
}

/// macOS only: install the privileged TUN service (one admin prompt, then passwordless).
/// No-op error on other platforms, which use the elevation flow instead.
#[tauri::command]
pub async fn cmd_install_tun_service() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Runs osascript (blocking) on a worker thread so the UI stays responsive.
        tokio::task::spawn_blocking(|| crate::tun::install_tun_service())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("当前平台不需要安装 TUN 服务".to_string())
    }
}

/// macOS only: remove the privileged TUN service (one admin prompt).
#[tauri::command]
pub async fn cmd_uninstall_tun_service() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(|| crate::tun::uninstall_tun_service())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("当前平台无 TUN 服务".to_string())
    }
}

#[tauri::command]
pub fn cmd_wintun_available() -> bool {
    crate::tun::wintun_available()
}

#[tauri::command]
pub async fn cmd_download_wintun() -> Result<(), String> {
    let bin_dir = crate::updater::resolved_singbox_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    crate::tun::download_wintun(&bin_dir)
        .await
        .map_err(|e| e.to_string())
}

// ─── Rules ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_rules() -> Vec<crate::rules::RouteRule> {
    crate::rules::load_rules()
}

#[tauri::command]
pub fn cmd_save_rules(rules: Vec<crate::rules::RouteRule>) -> Result<(), String> {
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cmd_add_rule(rule: crate::rules::RouteRule) -> Result<Vec<crate::rules::RouteRule>, String> {
    let mut rules = crate::rules::load_rules();
    rules.push(rule);
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())?;
    Ok(rules)
}

#[tauri::command]
pub fn cmd_delete_rule(id: String) -> Result<Vec<crate::rules::RouteRule>, String> {
    let mut rules = crate::rules::load_rules();
    rules.retain(|r| r.id != id);
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())?;
    Ok(rules)
}

#[tauri::command]
pub fn cmd_toggle_rule(id: String) -> Result<Vec<crate::rules::RouteRule>, String> {
    let mut rules = crate::rules::load_rules();
    if let Some(rule) = rules.iter_mut().find(|r| r.id == id) {
        rule.enabled = !rule.enabled;
    }
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())?;
    Ok(rules)
}

#[tauri::command]
pub fn cmd_reset_rules() -> Result<Vec<crate::rules::RouteRule>, String> {
    let rules = crate::rules::preset_rules();
    crate::rules::save_rules(&rules).map_err(|e| e.to_string())?;
    Ok(rules)
}

// ─── Remote rule-set providers ───────────────────────────────────────

#[tauri::command]
pub fn cmd_get_rule_providers() -> Vec<crate::rules::RuleProvider> {
    crate::rules::load_rule_providers()
}

#[tauri::command]
pub fn cmd_add_rule_provider(
    name: String,
    url: String,
    action: crate::rules::RuleAction,
) -> Result<Vec<crate::rules::RuleProvider>, String> {
    let mut providers = crate::rules::load_rule_providers();
    providers.push(crate::rules::RuleProvider {
        id: uuid::Uuid::new_v4().to_string(),
        format: crate::rules::guess_provider_format(&url),
        name,
        url,
        action,
        enabled: true,
    });
    crate::rules::save_rule_providers(&providers).map_err(|e| e.to_string())?;
    Ok(providers)
}

#[tauri::command]
pub fn cmd_delete_rule_provider(id: String) -> Result<Vec<crate::rules::RuleProvider>, String> {
    let mut providers = crate::rules::load_rule_providers();
    providers.retain(|p| p.id != id);
    crate::rules::save_rule_providers(&providers).map_err(|e| e.to_string())?;
    Ok(providers)
}

#[tauri::command]
pub fn cmd_toggle_rule_provider(id: String) -> Result<Vec<crate::rules::RuleProvider>, String> {
    let mut providers = crate::rules::load_rule_providers();
    if let Some(p) = providers.iter_mut().find(|p| p.id == id) {
        p.enabled = !p.enabled;
    }
    crate::rules::save_rule_providers(&providers).map_err(|e| e.to_string())?;
    Ok(providers)
}

// ─── Custom proxy groups ─────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_proxy_groups() -> Vec<crate::types::ProxyGroup> {
    config::load_proxy_groups()
}

/// Replace the full list of custom proxy groups. Takes effect on the next config
/// rebuild (reconnect / mode switch), like routing rules.
#[tauri::command]
pub fn cmd_save_proxy_groups(
    groups: Vec<crate::types::ProxyGroup>,
) -> Result<(), String> {
    config::save_proxy_groups(&groups).map_err(|e| e.to_string())
}

// ─── Updater ────────────────────────────────────────────────────────

#[tauri::command]
pub async fn cmd_check_singbox_update(force_refresh: Option<bool>) -> Result<crate::updater::ReleaseInfo, String> {
    crate::updater::fetch_latest_release(force_refresh.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cmd_get_installed_version() -> Option<String> {
    crate::updater::get_installed_version().await
}

#[tauri::command]
pub fn cmd_singbox_exists() -> bool {
    crate::updater::singbox_exists()
}

#[tauri::command]
pub async fn cmd_download_singbox(
    app_handle: tauri::AppHandle,
    download_url: String,
    sha256: Option<String>,
) -> Result<(), String> {
    let dest = crate::updater::singbox_binary_path();
    crate::updater::download_singbox(app_handle, download_url, dest, sha256)
        .await
        .map_err(|e| {
            // Emit failure event so frontend can handle it
            e.to_string()
        })
}

// ─── App Self-Updater ───────────────────────────────────────────────

#[tauri::command]
pub async fn cmd_check_app_update(
    channel: Option<String>,
    force_refresh: Option<bool>,
) -> Result<crate::updater::AppReleaseInfo, String> {
    let ch = channel.as_deref().unwrap_or("stable");
    crate::updater::fetch_app_release(ch, force_refresh.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cmd_download_app_update(
    app_handle: tauri::AppHandle,
    download_url: String,
    sha256: Option<String>,
) -> Result<(), String> {
    crate::updater::download_and_install_app(app_handle, download_url, sha256)
        .await
        .map_err(|e| e.to_string())
}

// ─── System Proxy ───────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_get_system_proxy_status() -> bool {
    crate::proxy::get_system_proxy_status()
}

#[tauri::command]
pub fn cmd_set_system_proxy(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if enabled {
        let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
        if !running {
            return Err("sing-box 未运行，请先启动代理再开启系统代理".to_string());
        }
        if state.app_config.lock().unwrap_or_else(|e| e.into_inner()).tun_enabled {
            return Err("TUN 模式已接管全部流量，无需且不能再开启系统代理".to_string());
        }
    }
    let (port, global_mode) = {
        let cfg = state.app_config.lock().unwrap_or_else(|e| e.into_inner());
        (cfg.mixed_port, cfg.proxy_mode == ProxyMode::Global)
    };
    crate::proxy::set_system_proxy(enabled, if enabled { port } else { 0 }, global_mode)
        .map_err(|e| e.to_string())
}

// ─── Tray ───────────────────────────────────────────────────────────

#[tauri::command]
pub fn cmd_update_tray_tooltip(app_handle: tauri::AppHandle, tooltip: String) {
    if let Some(tray) = app_handle.tray_by_id("tray-main") {
        let _ = tray.set_tooltip(Some(&tooltip));
    }
}

/// Update the tray check-menu items to reflect current system-proxy and TUN state.
#[tauri::command]
pub fn cmd_sync_tray_menu(
    sys_proxy_enabled: bool,
    tun_enabled: bool,
    tray_state: State<'_, TrayState>,
) {
    if let Ok(guard) = tray_state.sys_proxy_item.lock() {
        if let Some(item) = guard.as_ref() {
            let _ = item.set_checked(sys_proxy_enabled);
        }
    }
    if let Ok(guard) = tray_state.tun_item.lock() {
        if let Some(item) = guard.as_ref() {
            let _ = item.set_checked(tun_enabled);
        }
    }
}

// ─── Process Memory ─────────────────────────────────────────────────

/// Returns the working-set memory (RSS) of the sing-box process in bytes,
/// or None if sing-box is not running or the query fails.
#[tauri::command]
pub fn cmd_get_memory_usage(state: State<AppState>) -> Option<u64> {
    let pid = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).pid?;

    #[cfg(target_os = "windows")]
    unsafe {
        use winapi::um::handleapi::CloseHandle;
        use winapi::um::processthreadsapi::OpenProcess;
        use winapi::um::psapi::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use winapi::um::winnt::{PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = GetProcessMemoryInfo(
            handle,
            &mut pmc,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );
        CloseHandle(handle);
        if ok != 0 { Some(pmc.WorkingSetSize as u64) } else { None }
    }

    // macOS: read resident set size (in KB) from `ps`.
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let kb: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .ok()?;
        Some(kb * 1024)
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

// ─── Helpers ────────────────────────────────────────────────────────

/// Airport usage / quota parsed from a subscription's `Subscription-Userinfo` header.
/// All fields optional — present only when the provider sends the header.
#[derive(Default, Clone)]
pub(crate) struct SubUserinfo {
    pub upload: Option<u64>,
    pub download: Option<u64>,
    pub total: Option<u64>,
    pub expire: Option<i64>,
}

/// Parse a `Subscription-Userinfo` header value, e.g.
/// `upload=455727941; download=6174903220; total=214748364800; expire=1762524000`.
/// Unknown keys and malformed numbers are ignored so a partial header still yields
/// whatever fields are valid.
pub(crate) fn parse_userinfo(header: &str) -> SubUserinfo {
    let mut info = SubUserinfo::default();
    for part in header.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        match k.trim() {
            "upload" => info.upload = v.trim().parse().ok(),
            "download" => info.download = v.trim().parse().ok(),
            "total" => info.total = v.trim().parse().ok(),
            "expire" => info.expire = v.trim().parse().ok(),
            _ => {}
        }
    }
    info
}

/// The local mixed-inbound port to retry a failed direct subscription fetch through, or
/// None when the core isn't running. Read without holding a lock across the later await.
pub(crate) fn active_proxy_port(state: &AppState) -> Option<u16> {
    let running = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner()).running;
    running.then(|| state.app_config.lock().unwrap_or_else(|e| e.into_inner()).mixed_port)
}

/// Hard cap on a subscription response body. Airport subscriptions are at most a few KB;
/// this bounds memory so a hostile/compromised host (or a MITM) can't stream gigabytes into
/// `resp.text()` and OOM the process — a crash the unattended auto-updater would then repeat
/// on every launch. Also caps the amplification from base64 / YAML-alias expansion.
const MAX_SUBSCRIPTION_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

/// Summarize a reqwest error WITHOUT its `Display`, which embeds the request URL — and a
/// subscription URL commonly carries a secret `token=`. Returns only the failure category
/// so the message can be surfaced/logged without leaking credentials.
fn fetch_err_summary(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "请求超时".to_string()
    } else if e.is_connect() {
        "连接失败".to_string()
    } else if e.is_redirect() {
        "重定向次数过多".to_string()
    } else if e.is_body() || e.is_decode() {
        "响应读取失败".to_string()
    } else {
        "网络请求失败".to_string()
    }
}

/// Guard a user-supplied fetch URL before it hits the HTTP client: restrict the scheme to
/// http/https (no `file://`, `gopher://`, …) and reject IP-literal hosts that point at the
/// local machine or link-local metadata (e.g. the local Clash API port, or 169.254.169.254)
/// to blunt SSRF. Hostnames and private-LAN literals are allowed so self-hosted airports on
/// a LAN keep working.
fn validate_fetch_url(url: &str) -> Result<(), anyhow::Error> {
    let parsed = reqwest::Url::parse(url).map_err(|_| anyhow!("订阅地址格式无效"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(anyhow!("不支持的协议 {}（仅允许 http/https）", other)),
    }
    if let Some(host) = parsed.host() {
        use std::net::IpAddr;
        let ip = match host {
            url::Host::Ipv4(v4) => Some(IpAddr::V4(v4)),
            url::Host::Ipv6(v6) => Some(IpAddr::V6(v6)),
            url::Host::Domain(_) => None,
        };
        if let Some(ip) = ip {
            let link_local = match ip {
                IpAddr::V4(v4) => v4.is_link_local(),
                // fe80::/10
                IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
            };
            if ip.is_loopback() || ip.is_unspecified() || link_local {
                return Err(anyhow!("订阅地址指向受限的本机/元数据地址，已拒绝"));
            }
        }
    }
    Ok(())
}

/// Fetch a subscription. Tries a DIRECT fetch first (most airport hosts are directly
/// reachable, and this needs no working tunnel); if that fails and the core is running,
/// retries THROUGH the local mixed inbound so hosts only reachable via the proxy
/// (GFW-blocked / geo-fenced direct) still update.
pub(crate) async fn fetch_url(url: &str, proxy_port: Option<u16>) -> Result<(String, SubUserinfo), anyhow::Error> {
    match fetch_url_via(url, None).await {
        Ok(v) => Ok(v),
        Err(direct_err) => match proxy_port {
            Some(port) => fetch_url_via(url, Some(port)).await.map_err(|proxy_err| {
                anyhow!("直连失败（{}）；经代理重试也失败（{}）", direct_err, proxy_err)
            }),
            None => Err(direct_err),
        },
    }
}

/// One subscription fetch attempt. `proxy_port = None` forces a truly direct request
/// (bypassing any env-var proxy); `Some(port)` routes through `http://127.0.0.1:<port>`.
async fn fetch_url_via(url: &str, proxy_port: Option<u16>) -> Result<(String, SubUserinfo), anyhow::Error> {
    validate_fetch_url(url)?;
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(config::subscription_user_agent());
    builder = match proxy_port {
        Some(port) => builder.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", port))?),
        None => builder.no_proxy(),
    };
    let client = builder.build()?;
    let resp = client.get(url).send().await.map_err(|e| anyhow!("{}", fetch_err_summary(&e)))?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    // Capture the quota header (case-insensitive lookup) before consuming the body.
    let userinfo = resp.headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .map(parse_userinfo)
        .unwrap_or_default();
    // Reject an oversized body up front when the server declares its length…
    if let Some(len) = resp.content_length() {
        if len > MAX_SUBSCRIPTION_BYTES as u64 {
            return Err(anyhow!("订阅内容过大（{} 字节），已拒绝", len));
        }
    }
    // …and enforce the cap while streaming, since Content-Length may be absent or lie.
    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("{}", fetch_err_summary(&e)))?;
        if buf.len() + chunk.len() > MAX_SUBSCRIPTION_BYTES {
            return Err(anyhow!(
                "订阅内容超过大小上限（{} MiB），已拒绝",
                MAX_SUBSCRIPTION_BYTES / 1024 / 1024
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    let content = String::from_utf8_lossy(&buf).into_owned();
    Ok((content, userinfo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ipapi_extracts_fields_and_drops_empties() {
        let body: Value = serde_json::from_str(
            r#"{"status":"success","country":"Japan","city":"Tokyo","isp":"Acme","query":"203.0.113.7"}"#,
        ).unwrap();
        let (ip, country, city, isp) = parse_ipapi(&body);
        assert_eq!(ip.as_deref(), Some("203.0.113.7"));
        assert_eq!(country.as_deref(), Some("Japan"));
        assert_eq!(city.as_deref(), Some("Tokyo"));
        assert_eq!(isp.as_deref(), Some("Acme"));

        let partial: Value = serde_json::from_str(r#"{"query":"1.2.3.4","country":""}"#).unwrap();
        let (ip2, c2, ci2, is2) = parse_ipapi(&partial);
        assert_eq!(ip2.as_deref(), Some("1.2.3.4"));
        assert_eq!(c2, None);
        assert_eq!(ci2, None);
        assert_eq!(is2, None);
    }

    #[test]
    fn merge_runtime_fields_preserves_selection_only_on_save() {
        let mut current = AppConfig::default();
        current.last_proxy_running = true;
        current.active_nodes.insert("proxy".into(), "香港01".into());
        current.selected_subscription = Some("sub-a".into());

        // A settings-form save carries a stale/empty selection snapshot; it must NOT win.
        let mut incoming = AppConfig::default();
        incoming.active_nodes.insert("proxy".into(), "STALE".into());
        incoming.selected_subscription = None;

        let saved = merge_runtime_fields(incoming.clone(), &current, true);
        assert_eq!(saved.active_nodes.get("proxy").map(String::as_str), Some("香港01"));
        assert_eq!(saved.selected_subscription.as_deref(), Some("sub-a"));
        // Backend-owned lifecycle fields always come from `current`.
        assert!(saved.last_proxy_running);

        // An import/restore, by contrast, must adopt the bundle's selection.
        let imported = merge_runtime_fields(incoming, &current, false);
        assert_eq!(imported.active_nodes.get("proxy").map(String::as_str), Some("STALE"));
        assert_eq!(imported.selected_subscription, None);
        assert!(imported.last_proxy_running);
    }

    #[test]
    fn sanitize_profile_name_blocks_traversal_and_separators() {
        assert_eq!(sanitize_profile_name("home").as_deref(), Some("home"));
        assert_eq!(sanitize_profile_name("  Work 公司  ").as_deref(), Some("Work 公司"));
        assert_eq!(sanitize_profile_name(""), None);
        assert_eq!(sanitize_profile_name("   "), None);
        assert_eq!(sanitize_profile_name("../etc/passwd"), None);
        assert_eq!(sanitize_profile_name("a/b"), None);
        assert_eq!(sanitize_profile_name("a\\b"), None);
        assert_eq!(sanitize_profile_name(&"x".repeat(65)), None);
        // Windows: a drive-relative `C:…` escapes profiles/ entirely via PathBuf::join;
        // `name:stream` writes an NTFS alternate data stream.
        assert_eq!(sanitize_profile_name("C:evil"), None);
        assert_eq!(sanitize_profile_name("a:b"), None);
        // Windows invalid filename characters and control chars.
        assert_eq!(sanitize_profile_name("a*b"), None);
        assert_eq!(sanitize_profile_name("a?b"), None);
        assert_eq!(sanitize_profile_name("a\tb"), None);
        // Windows reserved device names, with or without an extension, any case.
        assert_eq!(sanitize_profile_name("CON"), None);
        assert_eq!(sanitize_profile_name("nul.backup"), None);
        assert_eq!(sanitize_profile_name("Com1"), None);
        // Trailing dot is stripped by the Windows filesystem (name aliasing).
        assert_eq!(sanitize_profile_name("backup."), None);
        // Interior dots stay legal.
        assert_eq!(sanitize_profile_name("v0.5.4").as_deref(), Some("v0.5.4"));
    }
}
