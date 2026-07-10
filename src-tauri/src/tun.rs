use anyhow::{Result, anyhow};
#[cfg(target_os = "windows")]
use std::path::PathBuf;

/// Check if the current process is running as Administrator
#[cfg(target_os = "windows")]
pub fn is_elevated() -> bool {
    use std::mem;
    use winapi::um::processthreadsapi::OpenProcessToken;
    use winapi::um::processthreadsapi::GetCurrentProcess;
    use winapi::um::securitybaseapi::GetTokenInformation;
    use winapi::um::winnt::{TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY, HANDLE};

    unsafe {
        let mut token: HANDLE = mem::zeroed();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = mem::zeroed();
        let mut size = mem::size_of::<TOKEN_ELEVATION>() as u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            size,
            &mut size,
        );
        if ok == 0 {
            return false;
        }
        elevation.TokenIsElevated != 0
    }
}

/// On Unix (macOS/Linux), sing-box's TUN mode needs root privileges.
#[cfg(unix)]
pub fn is_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Relaunch the current process with UAC elevation (Windows only)
#[cfg(target_os = "windows")]
pub fn relaunch_as_admin() -> Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::shellapi::ShellExecuteW;
    use winapi::um::winuser::SW_SHOWNORMAL;

    let exe = std::env::current_exe()?;
    let exe_wide: Vec<u16> = OsStr::new(exe.to_str().unwrap_or(""))
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let verb: Vec<u16> = OsStr::new("runas")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            exe_wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns > 32 on success
    if result as usize > 32 {
        // Exit current non-elevated instance
        std::process::exit(0);
    } else {
        Err(anyhow!("UAC 提权请求被拒绝或失败（错误码: {}）", result as usize))
    }
}

/// Relaunch the whole app with root privileges via a macOS admin prompt, then exit
/// this instance. sing-box (spawned as a child) then inherits root, which the macOS
/// utun-based TUN mode requires. This mirrors the Windows UAC relaunch flow.
#[cfg(target_os = "macos")]
pub fn relaunch_as_admin() -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy().to_string();

    // Single-quote the path inside the AppleScript shell command so spaces are handled.
    let script = format!(
        "do shell script \"'{}' > /dev/null 2>&1 &\" with administrator privileges",
        exe_str
    );

    let status = std::process::Command::new("osascript")
        .args(["-e", &script])
        .status()?;

    if status.success() {
        std::process::exit(0);
    } else {
        Err(anyhow!("管理员授权被取消或失败"))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn relaunch_as_admin() -> Result<()> {
    Err(anyhow!("不支持此平台"))
}

// ─── macOS TUN privileged service (authorize once) ──────────────────
//
// macOS TUN (utun) needs root. Rather than relaunching the WHOLE GUI as root on every
// launch — a password prompt each time, and a root-owned GUI is itself a security smell —
// we install a one-time privileged "service":
//   • the sing-box binary is copied to a root-owned location (`/Library/Skylark/sing-box`,
//     root:wheel 0755 — NOT user-writable, so the NOPASSWD rule below can't be hijacked by
//     swapping the binary), and
//   • a sudoers drop-in grants the current user NOPASSWD rights to run EXACTLY that binary
//     and to pkill it.
// After the single install prompt, starting/stopping TUN needs no password: the core is
// launched via `sudo -n` and the GUI stays as the normal user. Updating the kernel later
// requires re-running the install so the root-owned copy is refreshed.

/// Absolute path of the root-owned sing-box used for TUN. Referenced by the pkill teardown
/// and copied in by the install; the sudoers rule pins the WRAPPER below, never this binary
/// directly (a direct grant would allow `sudo sing-box run -c /tmp/evil.json` → root runs an
/// attacker config → local root escalation).
#[cfg(target_os = "macos")]
pub const TUN_ROOT_BIN: &str = "/Library/Skylark/sing-box";
/// Root-owned, non-user-writable wrapper that is the ONLY thing the sudoers rule grants.
/// It takes NO arguments and always runs a FIXED, root-owned config, so an attacker can
/// neither pass an arbitrary config path nor an arbitrary sing-box subcommand to root.
#[cfg(target_os = "macos")]
pub const TUN_ROOT_RUN: &str = "/Library/Skylark/skylark-tun-run.sh";
/// Root-owned config the wrapper actually runs (copied from the app-generated staging file
/// and locked to root:wheel 0600 by the wrapper before launch).
#[cfg(target_os = "macos")]
const TUN_ROOT_CONFIG: &str = "/Library/Skylark/config.json";
#[cfg(target_os = "macos")]
const TUN_SUDOERS_PATH: &str = "/etc/sudoers.d/skylark";

/// Reject any path that could break out of the shell quoting we interpolate it into (the
/// install script and the wrapper). A hostile `$HOME` / `$TMPDIR` at launch is the vector:
/// without this, a path containing a quote/`$`/backtick could inject commands into the
/// root-run install script. Valid macOS app paths never contain these.
#[cfg(target_os = "macos")]
fn shell_safe_path(p: &str) -> bool {
    !p.is_empty() && !p.contains(['"', '\'', '`', '$', '\\', '\n', '\r'])
}

/// The login user's short name. The GUI runs as this user; we must capture it HERE in the
/// (non-elevated) GUI process — inside the elevated install script `whoami` would be `root`.
#[cfg(target_os = "macos")]
fn current_username() -> Option<String> {
    // The name is later interpolated into a shell script + a sudoers line, so it MUST be a
    // plain login name — reject anything with whitespace/quotes/shell metacharacters so a
    // hostile $USER can't inject into either. Valid macOS short names are already a subset
    // of this charset.
    fn valid(name: &str) -> Option<String> {
        let ok = !name.is_empty()
            && name != "root"
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        ok.then(|| name.to_string())
    }
    if let Ok(u) = std::env::var("USER") {
        if let Some(u) = valid(u.trim()) {
            return Some(u);
        }
    }
    let out = std::process::Command::new("id").arg("-un").output().ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    valid(&name)
}

/// True when the privileged TUN service is installed AND usable: the user can run the
/// root-owned sing-box via `sudo -n` with no password. Probing the real allowed command
/// confirms both the binary's presence and that the NOPASSWD sudoers rule is active.
#[cfg(target_os = "macos")]
pub fn tun_service_installed() -> bool {
    if !std::path::Path::new(TUN_ROOT_BIN).exists()
        || !std::path::Path::new(TUN_ROOT_RUN).exists()
    {
        return false;
    }
    // Confirm the NOPASSWD rule for the wrapper is active WITHOUT executing it (`sudo -l`
    // only checks permission — running the wrapper would actually start TUN). `-n` never
    // prompts, so this stays silent when the rule is missing.
    std::process::Command::new("sudo")
        .args(["-n", "-l", TUN_ROOT_RUN])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Install the privileged TUN service with a SINGLE admin prompt. Copies the currently
/// downloaded sing-box to the root-owned path and installs a `visudo`-validated sudoers
/// drop-in. The privileged steps run from a temp script executed via one `osascript … with
/// administrator privileges` call (a script file keeps the shell quoting sane).
#[cfg(target_os = "macos")]
pub fn install_tun_service() -> Result<()> {
    let user = current_username().ok_or_else(|| anyhow!("无法确定当前用户"))?;
    let src = crate::updater::resolved_singbox_path();
    if !src.exists() {
        return Err(anyhow!("未找到 sing-box 内核，无法安装 TUN 服务，请重新安装应用或在「设置」中下载内核"));
    }
    let src_str = src.to_string_lossy().to_string();
    // The staging config the wrapper reads at every TUN start — the SAME path the app writes
    // its generated config to (see singbox_config_path). Baked into the wrapper as a literal.
    let stage_str = crate::config::singbox_config_path().to_string_lossy().to_string();

    // These paths are interpolated into a root-run shell script; a hostile $HOME/$TMPDIR must
    // not be able to inject commands. Reject anything with shell metacharacters.
    if !shell_safe_path(&src_str) || !shell_safe_path(&stage_str) {
        return Err(anyhow!("内核或配置路径包含非法字符，已中止安装"));
    }

    // The sudoers rule grants ONLY the argument-less wrapper (+ pkill), never the core binary
    // directly — so root can never be handed an arbitrary config path or subcommand. The
    // wrapper copies the app-generated config into a root-owned file (0600) and runs that.
    // Both heredocs use a QUOTED delimiter so the outer shell performs no `$` expansion; the
    // wrapper's own `$SRC`/`$DST` are written literally and expand only when the wrapper runs.
    let script = format!(
        "#!/bin/sh\n\
         set -e\n\
         mkdir -p /Library/Skylark\n\
         cp \"{src}\" \"{bin}\"\n\
         chown root:wheel \"{bin}\"\n\
         chmod 755 \"{bin}\"\n\
         xattr -dr com.apple.quarantine \"{bin}\" 2>/dev/null || true\n\
         cat > {run} <<'WRAP'\n\
#!/bin/sh\n\
set -e\n\
SRC=\"{stage}\"\n\
DST={config}\n\
BIN={bin}\n\
cp \"$SRC\" \"$DST\"\n\
chown root:wheel \"$DST\"\n\
chmod 600 \"$DST\"\n\
exec \"$BIN\" run -c \"$DST\"\n\
WRAP\n\
         chown root:wheel {run}\n\
         chmod 755 {run}\n\
         TMP=$(mktemp)\n\
         cat > \"$TMP\" <<'EOF'\n\
         {user} ALL=(root) NOPASSWD: {run}, /usr/bin/pkill -TERM -f {bin}, /usr/bin/pkill -KILL -f {bin}\n\
         EOF\n\
         chmod 440 \"$TMP\"\n\
         if /usr/sbin/visudo -cf \"$TMP\" >/dev/null 2>&1; then\n\
           chown root:wheel \"$TMP\"\n\
           mv \"$TMP\" {sudoers}\n\
         else\n\
           rm -f \"$TMP\"\n\
           echo 'sudoers validation failed' >&2\n\
           exit 1\n\
         fi\n",
        src = src_str,
        bin = TUN_ROOT_BIN,
        run = TUN_ROOT_RUN,
        config = TUN_ROOT_CONFIG,
        stage = stage_str,
        user = user,
        sudoers = TUN_SUDOERS_PATH,
    );

    let tmp_script = std::env::temp_dir().join("skylark-install-tun.sh");
    std::fs::write(&tmp_script, script).map_err(|e| anyhow!("无法写入安装脚本: {}", e))?;
    let tmp_str = tmp_script.to_string_lossy().to_string();
    if !shell_safe_path(&tmp_str) {
        let _ = std::fs::remove_file(&tmp_script);
        return Err(anyhow!("临时脚本路径包含非法字符，已中止安装"));
    }

    // Single-quote the script path inside the AppleScript command so spaces are handled.
    let osa = format!(
        "do shell script \"/bin/sh '{}'\" with administrator privileges",
        tmp_str
    );
    let status = std::process::Command::new("osascript")
        .args(["-e", &osa])
        .status();
    let _ = std::fs::remove_file(&tmp_script);

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err(anyhow!("管理员授权被取消或安装失败")),
        Err(e) => Err(anyhow!("无法运行授权: {}", e)),
    }
}

/// Remove the privileged TUN service (sudoers drop-in + root-owned binary). One admin prompt.
#[cfg(target_os = "macos")]
pub fn uninstall_tun_service() -> Result<()> {
    let script = format!(
        "rm -f {sudoers}; rm -f {run}; rm -f {config}; rm -f {bin}; rmdir /Library/Skylark 2>/dev/null || true",
        sudoers = TUN_SUDOERS_PATH,
        run = TUN_ROOT_RUN,
        config = TUN_ROOT_CONFIG,
        bin = TUN_ROOT_BIN,
    );
    let osa = format!(
        "do shell script \"{}\" with administrator privileges",
        script.replace('"', "\\\"")
    );
    let status = std::process::Command::new("osascript")
        .args(["-e", &osa])
        .status()
        .map_err(|e| anyhow!("无法运行授权: {}", e))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("管理员授权被取消"))
    }
}

/// Remove leftover "skylark-tun*" network adapters from previous runs (fast path).
///
/// Because each start now uses a UNIQUE interface name, an orphaned adapter can no longer
/// cause the "Cannot create a file when that file already exists" conflict — so this cleanup
/// is NOT a blocking precondition for startup, it only prevents stale adapters from
/// accumulating over time. It is therefore kept lightweight (no fixed sleep).
///
/// We deliberately do NOT delete the wintun driver service here: doing that on every start
/// forces sing-box to reinstall the driver each time, which is the slow
/// "open interface take too much time" path. Driver-service repair is handled separately
/// by `repair_wintun_driver()` for the rare wedged-service case.
///
/// Must be called with admin privileges (the app always runs elevated).
#[cfg(target_os = "windows")]
pub async fn cleanup_stale_tun_adapter() {
    use tokio::process::Command as TokioCommand;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let ps = "Get-NetAdapter -Name 'skylark-tun*' -ErrorAction SilentlyContinue | \
              Remove-NetAdapter -Confirm:$false -ErrorAction SilentlyContinue";
    let _ = TokioCommand::new("powershell")
        .args(["-NonInteractive", "-NoProfile", "-WindowStyle", "Hidden", "-Command", ps])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;
}

#[cfg(not(target_os = "windows"))]
pub async fn cleanup_stale_tun_adapter() {}

/// Check if WinTun driver DLL is present alongside sing-box binary.
/// Only Windows needs WinTun; on macOS/Linux the kernel provides the TUN device,
/// so we always report it as available.
#[cfg(target_os = "windows")]
pub fn wintun_available() -> bool {
    let bin_dir = crate::updater::resolved_singbox_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // WinTun ships wintun.dll in the same directory as sing-box on Windows
    bin_dir.join("wintun.dll").exists()
}

#[cfg(not(target_os = "windows"))]
pub fn wintun_available() -> bool {
    true
}

/// Download WinTun driver DLL
/// WinTun is bundled inside some sing-box releases; if missing, download from wintun.net
pub async fn download_wintun(dest_dir: &std::path::Path) -> Result<()> {
    // Official WinTun zip download (amd64 build)
    let url = "https://www.wintun.net/builds/wintun-0.14.1.zip";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!("skylark/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("WinTun 下载失败: HTTP {}", resp.status()));
    }
    let zip_bytes = resp.bytes().await?;

    // Ensure destination directory exists
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| anyhow!("无法创建目录 {:?}: {}", dest_dir, e))?;

    // Extract wintun/bin/amd64/wintun.dll
    use std::io::Cursor;
    let cursor = Cursor::new(zip_bytes.as_ref());
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_lowercase();
        if name.ends_with("amd64/wintun.dll") || name.ends_with("amd64\\wintun.dll") {
            // Directory already created above
            let dest = dest_dir.join("wintun.dll");
            let mut out = std::fs::File::create(&dest)?;
            let mut buf = Vec::new();
            use std::io::Read;
            file.read_to_end(&mut buf)?;
            use std::io::Write;
            out.write_all(&buf)?;
            return Ok(());
        }
    }

    Err(anyhow!("WinTun zip 中未找到 amd64/wintun.dll"))
}
