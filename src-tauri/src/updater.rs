use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Emitter;

const GITHUB_API: &str = "https://api.github.com/repos/SagerNet/sing-box/releases/latest";
const APP_GITHUB_STABLE_API: &str =
    "https://api.github.com/repos/radiumCN/skylark/releases/latest";
const APP_GITHUB_ALL_API: &str =
    "https://api.github.com/repos/radiumCN/skylark/releases";
/// Cache validity duration in seconds (1 hour)
const CACHE_TTL_SECS: u64 = 3600;

/// Optional GitHub API token, injected at BUILD time via the `SKYLARK_GH_TOKEN` env var
/// (e.g. `SKYLARK_GH_TOKEN=github_pat_xxx npm run tauri build`).
///
/// Never hardcode a literal token here: committing a `github_pat_*` string trips GitHub's
/// secret scanning / push protection and gets the token auto-revoked, and a shipped binary
/// can be unpacked by users. When set, it raises the api.github.com rate limit from
/// 60/hour to 5000/hour; when unset, requests stay anonymous exactly as before.
fn github_token() -> Option<&'static str> {
    option_env!("SKYLARK_GH_TOKEN").filter(|t| !t.is_empty())
}

/// Attach the bearer token (if configured) to an **api.github.com** request. Only use this
/// for the JSON metadata endpoints — never for asset downloads, which 302-redirect to a
/// CDN host (objects.githubusercontent.com) where the Authorization header must not be sent.
fn with_github_auth(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match github_token() {
        Some(token) => req.bearer_auth(token),
        None => req,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub published_at: String,
    pub release_notes: String,
    pub download_url: String,
    /// Expected SHA-256 of the download asset, lowercase hex (no `sha256:` prefix).
    /// Sourced from the GitHub release asset's `digest` field. `None` when the release
    /// predates GitHub's per-asset digests, in which case integrity is not verified.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Lowercase-hex SHA-256 of the given bytes.
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

/// Normalise a GitHub asset `digest` value (e.g. `"sha256:ABC…"`) to bare lowercase
/// hex. Returns `None` for empty / non-sha256 values so we never compare against a
/// hash we can't compute.
fn normalize_sha256_digest(raw: &str) -> Option<String> {
    let hex = raw.strip_prefix("sha256:").unwrap_or(raw).trim().to_lowercase();
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hex)
    } else {
        None
    }
}

/// The sing-box release asset matching the platform we are currently running on.
/// sing-box publishes one archive per OS/arch on its GitHub releases, e.g.
///   sing-box-1.x.y-windows-amd64.zip   → sing-box.exe
///   sing-box-1.x.y-darwin-amd64.tar.gz → sing-box   (macOS Intel)
///   sing-box-1.x.y-darwin-arm64.tar.gz → sing-box   (macOS Apple Silicon)
///   sing-box-1.x.y-linux-amd64.tar.gz  → sing-box
struct PlatformAsset {
    /// OS keyword used in the asset file name.
    os: &'static str,
    /// CPU arch keyword used in the asset file name (sing-box uses Go names).
    arch: &'static str,
    /// Archive file extension, including the leading dot.
    ext: &'static str,
}

fn current_platform_asset() -> PlatformAsset {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        // x86_64 → amd64 (the only other arch we ship)
        "amd64"
    };
    let ext = if cfg!(target_os = "windows") { ".zip" } else { ".tar.gz" };
    PlatformAsset { os, arch, ext }
}

/// File name of the sing-box executable for the current platform.
fn singbox_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "sing-box.exe"
    } else {
        "sing-box"
    }
}

/// On Unix, mark the freshly-extracted binary as executable (0755).
fn finalize_binary(_dest: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(_dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(_dest, perms)?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCache {
    cached_at_secs: u64,
    release: ReleaseInfo,
}

fn cache_path() -> PathBuf {
    crate::config::app_data_dir().join("update_cache.json")
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load_cache() -> Option<ReleaseInfo> {
    let data = std::fs::read_to_string(cache_path()).ok()?;
    let cache: UpdateCache = serde_json::from_str(&data).ok()?;
    if unix_now().saturating_sub(cache.cached_at_secs) < CACHE_TTL_SECS {
        Some(cache.release)
    } else {
        None
    }
}

fn save_cache(release: &ReleaseInfo) {
    let cache = UpdateCache {
        cached_at_secs: unix_now(),
        release: release.clone(),
    };
    if let Ok(data) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(cache_path(), data);
    }
}

/// Query latest sing-box release from GitHub, with 1-hour local cache.
/// Pass `force_refresh = true` to bypass the cache and always hit the API.
pub async fn fetch_latest_release(force_refresh: bool) -> Result<ReleaseInfo> {
    // Return cached result if still fresh
    if !force_refresh {
        if let Some(cached) = load_cache() {
            return Ok(cached);
        }
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("skylark/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;

    let resp = with_github_auth(client.get(GITHUB_API)).send().await?;

    // Detect rate limit (HTTP 403 or 429)
    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        // Try to read reset time from headers
        let reset_hint = resp
            .headers()
            .get("X-RateLimit-Reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(|reset_ts| {
                let wait = reset_ts.saturating_sub(unix_now());
                let mins = wait / 60;
                if mins > 0 {
                    format!("，请 {} 分钟后重试", mins)
                } else {
                    "，请稍后重试".to_string()
                }
            })
            .unwrap_or_else(|| "，请稍后重试".to_string());

        return Err(anyhow!(
            "GitHub API 请求频率超限（未认证 IP 每小时限 60 次）{}",
            reset_hint
        ));
    }

    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API 请求失败: HTTP {}", resp.status()));
    }

    let body: serde_json::Value = resp.json().await?;

    // Check for GitHub error message in body
    if let Some(msg) = body["message"].as_str() {
        if msg.to_lowercase().contains("rate limit") {
            return Err(anyhow!(
                "GitHub API 请求频率超限（未认证 IP 每小时限 60 次），请稍后重试"
            ));
        }
        return Err(anyhow!("GitHub API 错误: {}", msg));
    }

    let version = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("无法解析版本号"))?
        .to_string();

    let published_at = body["published_at"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let release_notes = body["body"]
        .as_str()
        .unwrap_or("")
        .lines()
        .take(10)
        .collect::<Vec<_>>()
        .join("\n");

    // Find the asset matching the current OS + CPU architecture.
    let p = current_platform_asset();
    let asset = body["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("未找到下载资源"))?
        .iter()
        .find(|a| {
            let name = a["name"].as_str().unwrap_or("").to_lowercase();
            name.contains(p.os) && name.contains(p.arch) && name.ends_with(p.ext)
        })
        .ok_or_else(|| anyhow!("未找到 {}-{} 平台的下载链接", p.os, p.arch))?;

    let download_url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow!("未找到 {}-{} 平台的下载链接", p.os, p.arch))?
        .to_string();

    // GitHub exposes a per-asset `digest` ("sha256:…") on newer releases; absent on older
    // ones (then we skip verification rather than block the update).
    let sha256 = asset["digest"].as_str().and_then(normalize_sha256_digest);

    let release = ReleaseInfo {
        version,
        published_at,
        release_notes,
        download_url,
        sha256,
    };

    save_cache(&release);
    Ok(release)
}

/// Download and install sing-box binary with progress events
/// Emits: "singbox-download-progress" { percent: f64, downloaded: u64, total: u64 }
/// Emits: "singbox-download-done" { success: bool, message: String }
pub async fn download_singbox(
    app_handle: tauri::AppHandle,
    download_url: String,
    dest_path: PathBuf,
    expected_sha256: Option<String>,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    // Only ever download over TLS — a plain-http asset URL could be MITM'd to swap the binary.
    if !download_url.starts_with("https://") {
        return Err(anyhow!("拒绝非 HTTPS 下载地址：{}", download_url));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .user_agent(concat!("skylark/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;

    let resp = client.get(&download_url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("下载失败: HTTP {}", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;

    // Ensure destination directory exists before writing
    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent).await
            .map_err(|e| anyhow!("无法创建目录 {:?}: {}", parent, e))?;
    }

    // Download to a temp archive file in the same directory. The extension is
    // inferred from the download URL so we know how to extract it (.zip vs .tar.gz).
    let is_tar_gz = download_url.to_lowercase().ends_with(".tar.gz");
    let archive_name = if is_tar_gz {
        "sing-box-download.tar.gz"
    } else {
        "sing-box-download.zip"
    };
    let archive_path = dest_path.parent()
        .unwrap_or(std::path::Path::new("."))
        .join(archive_name);

    let mut file = tokio::fs::File::create(&archive_path).await
        .map_err(|e| anyhow!("无法创建临时文件 {:?}: {}", archive_path, e))?;
    let mut stream = resp.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("下载中断: {}", e))?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        let percent = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let _ = app_handle.emit("singbox-download-progress", serde_json::json!({
            "percent": percent,
            "downloaded": downloaded,
            "total": total,
        }));
    }
    file.flush().await?;
    drop(file);

    // Extract the sing-box binary from the archive.
    let archive_data = std::fs::read(&archive_path)?;

    // Verify integrity against the release's published SHA-256 before trusting the bytes.
    // A mismatch means a corrupted or tampered download — discard it rather than install.
    if let Some(expected) = expected_sha256.as_deref().and_then(normalize_sha256_digest) {
        let actual = sha256_hex(&archive_data);
        if actual != expected {
            let _ = std::fs::remove_file(&archive_path);
            return Err(anyhow!(
                "内核校验失败：下载文件 SHA-256 与发布值不一致（期望 {}…，实际 {}…），已丢弃",
                &expected[..8],
                &actual[..8]
            ));
        }
    }

    if is_tar_gz {
        extract_binary_from_tar_gz(&archive_data, &dest_path)?;
    } else {
        extract_binary_from_zip(&archive_data, &dest_path)?;
    }
    let _ = std::fs::remove_file(&archive_path);

    // Ensure the extracted binary is executable on Unix.
    finalize_binary(&dest_path)?;

    let _ = app_handle.emit("singbox-download-done", serde_json::json!({
        "success": true,
        "message": "下载完成",
    }));

    Ok(())
}

/// Write extracted bytes to `dest`, going through a temp file + rename so an
/// already-running sing-box binary doesn't block the write.
///
/// On Windows a running executable cannot be overwritten directly (os error 5 /
/// Access Denied). However Windows *does* allow renaming a file that is currently
/// in use, so we:
///   1. Write the new binary to `<dest>.tmp`
///   2. Rename the existing binary (if any) to `<dest>.old`  ← allowed even while running
///   3. Rename `.tmp` → final name
///   4. Best-effort delete of the `.old` file (succeeds after the process exits)
fn write_binary(dest: &PathBuf, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = dest.with_extension("tmp");
    let mut out = std::fs::File::create(&tmp)
        .map_err(|e| anyhow!("无法创建临时文件 {:?}: {}", tmp, e))?;
    out.write_all(bytes)?;
    drop(out);

    #[cfg(target_os = "windows")]
    {
        // Step 2: move the current binary aside so it is no longer at `dest`.
        let old = dest.with_extension("old");
        // Remove a stale .old from a previous update attempt (ignore errors).
        let _ = std::fs::remove_file(&old);
        if dest.exists() {
            std::fs::rename(dest, &old)
                .map_err(|e| anyhow!("无法移走旧内核文件: {}", e))?;
        }

        // Step 3: put the new binary in place.
        if let Err(e) = std::fs::rename(&tmp, dest) {
            // Roll back: restore the old binary so the app is not left without a kernel.
            let _ = std::fs::rename(&old, dest);
            return Err(anyhow!("无法写入新内核文件: {}", e));
        }

        // Step 4: clean up — this will silently fail if the old process is still alive,
        // which is fine; the file will be cleaned up on the next update.
        let _ = std::fs::remove_file(&old);
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::fs::rename(&tmp, dest)
            .map_err(|e| anyhow!("无法写入新内核文件: {}", e))?;
    }

    Ok(())
}

/// Extract the sing-box executable from a Windows `.zip` archive.
fn extract_binary_from_zip(zip_data: &[u8], dest: &PathBuf) -> Result<()> {
    use std::io::{Cursor, Read};
    let target = singbox_binary_name().to_lowercase();
    let cursor = Cursor::new(zip_data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| anyhow!("ZIP 解压失败: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_lowercase();
        if name.ends_with(target.as_str()) {
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            write_binary(dest, &buf)?;
            return Ok(());
        }
    }

    Err(anyhow!("压缩包中未找到 {}", singbox_binary_name()))
}

/// Extract the sing-box executable from a macOS/Linux `.tar.gz` archive.
fn extract_binary_from_tar_gz(data: &[u8], dest: &PathBuf) -> Result<()> {
    use std::io::{Cursor, Read};
    use flate2::read::GzDecoder;

    let target = singbox_binary_name();
    let decoder = GzDecoder::new(Cursor::new(data));
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().map_err(|e| anyhow!("tar.gz 解压失败: {}", e))? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let is_match = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == target)
            .unwrap_or(false);
        if is_match {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            write_binary(dest, &buf)?;
            return Ok(());
        }
    }

    Err(anyhow!("压缩包中未找到 {}", target))
}

/// Path of the user-downloaded sing-box binary in the app data dir. A user-downloaded
/// build always wins over the bundled one, so people can self-upgrade the kernel.
pub fn singbox_binary_path() -> PathBuf {
    crate::config::app_data_dir().join("bin").join(singbox_binary_name())
}

/// Path of the sing-box kernel shipped with the app as a Tauri sidecar (`externalBin`).
/// At bundle time Tauri strips the target-triple suffix and places the binary right next
/// to the main executable, so the sidecar lives in the running app's own directory:
///   • Windows/Linux → alongside the app exe
///   • macOS         → `Skylark.app/Contents/MacOS/sing-box`
pub fn bundled_singbox_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join(singbox_binary_name())
}

/// The sing-box binary actually used at runtime: a user-downloaded build takes priority,
/// otherwise the bundled sidecar that ships inside the installer — so the app works on
/// first run without needing a proxy to download the kernel (the chicken-and-egg case).
pub fn resolved_singbox_path() -> PathBuf {
    let user = singbox_binary_path();
    if user.exists() {
        user
    } else {
        bundled_singbox_path()
    }
}

/// Check if a usable sing-box binary exists (user-downloaded or bundled sidecar).
pub fn singbox_exists() -> bool {
    resolved_singbox_path().exists()
}

/// Get current installed version by running `sing-box version`.
/// Returns `None` only when the binary does not exist; when the binary exists
/// but version detection fails (spawn error, non-zero exit, empty output) this
/// returns `Some("<path>: version unknown")` so the caller can distinguish
/// "not installed" from "installed but version unreadable".
pub async fn get_installed_version() -> Option<String> {
    let path = resolved_singbox_path();
    if !path.exists() {
        return None;
    }

    let mut cmd = tokio::process::Command::new(&path);
    cmd.arg("version");
    // Null stdin so we never inherit a (possibly invalid) GUI std handle — inheriting one
    // makes spawn fail with ERROR_INVALID_HANDLE (os error 6) → "版本未知：句柄无效".
    cmd.stdin(std::process::Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);

    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => {
            log::warn!("sing-box version spawn error ({:?}): {}", path, e);
            // Binary exists but can't be executed — report "version unknown"
            // instead of returning None (which would wrongly say "not installed").
            return Some(format!("（版本未知，无法执行: {}）", e));
        }
    };

    // Try stdout first, then stderr (some platforms print there).
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Find the first non-empty line from stdout; fall back to stderr.
    // Typical stdout: "sing-box version 1.13.14"
    if let Some(line) = stdout.lines().find(|l| !l.trim().is_empty()) {
        return Some(line.to_string());
    }
    if let Some(line) = stderr.lines().find(|l| !l.trim().is_empty()) {
        return Some(line.to_string());
    }

    log::warn!(
        "sing-box version produced no output (exit {}): {:?}",
        output.status, path
    );
    // Binary exists but returned nothing — still report "installed, version unknown".
    Some(String::from("（版本未知）"))
}

// ─── App Self-Updater ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppReleaseInfo {
    pub version: String,
    pub published_at: String,
    pub release_notes: String,
    pub download_url: String,
    pub is_prerelease: bool,
    /// Expected SHA-256 of the installer asset, lowercase hex (from the GitHub asset
    /// `digest`). `None` on older releases without per-asset digests.
    #[serde(default)]
    pub sha256: Option<String>,
}

fn app_cache_path(channel: &str) -> PathBuf {
    crate::config::app_data_dir().join(format!("app_update_cache_{}.json", channel))
}

#[derive(Debug, Serialize, Deserialize)]
struct AppUpdateCache {
    cached_at_secs: u64,
    release: AppReleaseInfo,
}

fn load_app_cache(channel: &str) -> Option<AppReleaseInfo> {
    let data = std::fs::read_to_string(app_cache_path(channel)).ok()?;
    let cache: AppUpdateCache = serde_json::from_str(&data).ok()?;
    if unix_now().saturating_sub(cache.cached_at_secs) < CACHE_TTL_SECS {
        Some(cache.release)
    } else {
        None
    }
}

fn save_app_cache(channel: &str, release: &AppReleaseInfo) {
    let cache = AppUpdateCache {
        cached_at_secs: unix_now(),
        release: release.clone(),
    };
    if let Ok(data) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(app_cache_path(channel), data);
    }
}

/// Pick the app installer asset for the current platform from a release's asset list.
/// Windows → `.exe`; macOS → `.dmg` (prefer arch-matched, then universal). Returns the
/// asset object so the caller can read both its download URL and `digest`.
fn find_app_installer_asset(assets: &[serde_json::Value]) -> Option<&serde_json::Value> {
    #[cfg(target_os = "windows")]
    {
        // Prefer a clearly-named x64 installer, then fall back to any .exe.
        assets
            .iter()
            .find(|a| {
                let name = a["name"].as_str().unwrap_or("").to_lowercase();
                name.ends_with(".exe")
                    && (name.contains("x64") || name.contains("setup") || name.contains("install"))
            })
            .or_else(|| {
                assets.iter().find(|a| {
                    a["name"].as_str().unwrap_or("").to_lowercase().ends_with(".exe")
                })
            })
    }

    #[cfg(target_os = "macos")]
    {
        // skylark releases ship per-arch .dmg files; match the running arch.
        let arch_kw = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x64" };
        let arch_alt = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
        assets
            .iter()
            .find(|a| {
                let name = a["name"].as_str().unwrap_or("").to_lowercase();
                name.ends_with(".dmg") && (name.contains(arch_kw) || name.contains(arch_alt))
            })
            .or_else(|| {
                // Universal or single-arch build.
                assets.iter().find(|a| {
                    a["name"].as_str().unwrap_or("").to_lowercase().ends_with(".dmg")
                })
            })
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        assets.iter().find(|a| {
            let name = a["name"].as_str().unwrap_or("").to_lowercase();
            name.ends_with(".appimage") || name.ends_with(".deb")
        })
    }
}

fn parse_app_release(body: &serde_json::Value) -> Result<AppReleaseInfo> {
    let version = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("无法解析版本号"))?
        .to_string();

    let published_at = body["published_at"].as_str().unwrap_or("").to_string();
    let is_prerelease = body["prerelease"].as_bool().unwrap_or(false);

    let release_notes = body["body"]
        .as_str()
        .unwrap_or("")
        .lines()
        .take(12)
        .collect::<Vec<_>>()
        .join("\n");

    let assets = body["assets"]
        .as_array()
        .ok_or_else(|| anyhow!("未找到下载资源"))?;

    // Find the installer asset matching the current platform.
    let asset = find_app_installer_asset(assets)
        .ok_or_else(|| anyhow!("未找到当前平台的安装包"))?;
    let download_url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow!("未找到当前平台的安装包"))?
        .to_string();
    let sha256 = asset["digest"].as_str().and_then(normalize_sha256_digest);

    Ok(AppReleaseInfo { version, published_at, release_notes, download_url, is_prerelease, sha256 })
}

/// Fetch the latest app release for the given channel.
/// channel = "stable" → non-prerelease only; channel = "beta" → includes pre-release
pub async fn fetch_app_release(channel: &str, force_refresh: bool) -> Result<AppReleaseInfo> {
    if !force_refresh {
        if let Some(cached) = load_app_cache(channel) {
            return Ok(cached);
        }
    }

    // Bypass system proxy (which points to sing-box itself) to avoid circular dependency.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("skylark/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;

    let url = if channel == "beta" {
        APP_GITHUB_ALL_API
    } else {
        APP_GITHUB_STABLE_API
    };

    let resp = with_github_auth(client.get(url)).send().await?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Err(anyhow!("GitHub API 请求频率超限，请稍后重试"));
    }

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(anyhow!("暂无可用版本，请稍后再试"));
    }

    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API 请求失败: HTTP {}", resp.status()));
    }

    let body: serde_json::Value = resp.json().await?;

    // For beta channel the API returns an array; pick the first (most recent) release
    let release_body = if channel == "beta" {
        let arr = body.as_array().ok_or_else(|| anyhow!("无法解析版本列表"))?;
        if arr.is_empty() {
            return Err(anyhow!("暂无可用版本"));
        }
        arr[0].clone()
    } else {
        body
    };

    if let Some(msg) = release_body["message"].as_str() {
        return Err(anyhow!("GitHub API 错误: {}", msg));
    }

    let release = parse_app_release(&release_body)?;
    save_app_cache(channel, &release);
    Ok(release)
}

/// Append a timestamped line to `app_data/logs/update.log`. The self-update handoff (download
/// → teardown → launch installer → self-exit) is notoriously hard to debug because the app is
/// gone by the time anything goes wrong, so we leave a breadcrumb trail on disk. Best-effort:
/// a logging error must never abort the update.
pub(crate) fn update_log(msg: &str) {
    let dir = crate::config::app_data_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let line = format!(
        "{} {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        msg
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("update.log"))
    {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

/// Whether this process holds an elevated (admin) token. Logged at install time to confirm the
/// TUN-on elevation theory against real data. Returns None if the token can't be queried.
#[cfg(target_os = "windows")]
fn process_is_elevated() -> Option<bool> {
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcessToken};
    use winapi::um::securitybaseapi::GetTokenInformation;
    use winapi::um::winnt::{TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};

    unsafe {
        // `token` is inferred as winnt::HANDLE from OpenProcessToken's PHANDLE parameter, so we
        // avoid naming the type (its import path varies across winapi module features).
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        if ok == 0 {
            return None;
        }
        Some(elevation.TokenIsElevated != 0)
    }
}

/// Download app installer to temp directory and launch it.
/// Emits: "app-download-progress" { percent, downloaded, total }
/// Emits: "app-download-done"     { success, message }
pub async fn download_and_install_app(
    app_handle: tauri::AppHandle,
    download_url: String,
    expected_sha256: Option<String>,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    // Only ever download the installer over TLS — a plain-http URL could be MITM'd to swap
    // the executable that we are about to run.
    if !download_url.starts_with("https://") {
        return Err(anyhow!("拒绝非 HTTPS 下载地址：{}", download_url));
    }
    // Bypass system proxy (which points to sing-box itself) to avoid circular dependency.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .user_agent(concat!("skylark/", env!("CARGO_PKG_VERSION")))
        .no_proxy()
        .build()?;

    let resp = client.get(&download_url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("下载失败: HTTP {}", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;

    let temp_dir = std::env::temp_dir();
    let file_name = download_url
        .split('/')
        .last()
        .unwrap_or("skylark-setup.exe")
        .to_string();
    let installer_path = temp_dir.join(&file_name);

    let mut file = tokio::fs::File::create(&installer_path).await
        .map_err(|e| anyhow!("无法创建临时文件: {}", e))?;

    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("下载中断: {}", e))?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        let percent = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        let _ = app_handle.emit("app-download-progress", serde_json::json!({
            "percent": percent,
            "downloaded": downloaded,
            "total": total,
        }));
    }
    file.flush().await?;
    drop(file);

    // Verify the installer's integrity against the release's published SHA-256 BEFORE
    // launching it. Running an unverified installer is the highest-risk step in the whole
    // app, so a mismatch (corruption / tampering / MITM) must abort the update.
    if let Some(expected) = expected_sha256.as_deref().and_then(normalize_sha256_digest) {
        let bytes = std::fs::read(&installer_path)?;
        let actual = sha256_hex(&bytes);
        if actual != expected {
            let _ = std::fs::remove_file(&installer_path);
            return Err(anyhow!(
                "安装包校验失败：SHA-256 与发布值不一致（期望 {}…，实际 {}…），已取消安装",
                &expected[..8],
                &actual[..8]
            ));
        }
    }

    let _ = app_handle.emit("app-download-done", serde_json::json!({
        "success": true,
        "message": "下载完成，即将启动安装程序",
        "path": installer_path.to_string_lossy(),
    }));
    update_log(&format!("download done: installer={:?}", installer_path));

    // Cleanly tear down BEFORE the installer restarts the app. The NSIS installer
    // force-terminates the running process, which bypasses the window-close / tray-quit
    // handlers that normally run shutdown_core. Without this, the old core would be left
    // orphaned (still holding the mixed/API port and TUN adapter) and the Windows system
    // proxy would stay pointing at it — so the upgraded app restores "proxy on" on top of
    // a stale/dead core and ends up with no network. Stopping here clears both.
    // Capture TUN status BEFORE teardown (used for adapter cleanup below and the launch log).
    // The graceful `taskkill` (no /F) can't reach sing-box — a windowless console process —
    // so the core is always force-killed, which skips its WintunDeleteAdapter() + strict_route
    // route cleanup. That leaves the TUN adapter and its strict routes behind. If the upgraded
    // app then restores TUN on top of that stale adapter, traffic is black-holed.
    let was_tun;
    {
        use tauri::Manager;
        let state = app_handle.state::<crate::commands::AppState>();
        was_tun = {
            let s = state.singbox_state.lock().unwrap_or_else(|e| e.into_inner());
            s.running && s.tun_mode
        };
        update_log(&format!("teardown: begin, was_tun={}", was_tun));
        // FORCED stop (no graceful Ctrl+C). The graceful TUN path broadcasts a console
        // CTRL_C_EVENT to sing-box's process group, which on this worker thread was killing
        // the GUI itself mid-teardown — the app "crashed" right after the "download done" log
        // and never reached the installer launch. The adapter is removed deterministically
        // below, so we don't lose anything by force-killing.
        crate::commands::shutdown_core_forced(state.inner()).await;
        update_log("teardown: core stopped");
        // Deterministically remove the leftover TUN adapter NOW, while this (still elevated)
        // process is alive. The subsequent install runs for several seconds, giving Windows
        // ample time to settle the routing table before the relaunched app re-creates TUN —
        // breaking the rapid-restart race that the new app's own cleanup can't win on its own.
        if was_tun {
            update_log("teardown: cleaning stale TUN adapter");
            crate::tun::cleanup_stale_tun_adapter().await;
            update_log("teardown: TUN adapter cleaned");
        }
    }
    update_log(&format!("teardown done: was_tun={}", was_tun));

    // Launch the installer.
    // Windows: we close this app ourselves (see below) and the NSIS installer restarts it.
    //
    // Launch the installer DIRECTLY via CreateProcess (Command::spawn) — not explorer.exe,
    // not ShellExecuteW, not `cmd /C start`. History of this bug:
    //   • `cmd /C start` — broke because send_ctrl_c's FreeConsole left us console-less.
    //   • ShellExecuteW   — under TUN the app runs elevated (runas, tun::relaunch_as_admin),
    //                       so it launched the installer elevated; it killed the running app
    //                       but its UI never showed.
    //   • explorer.exe    — meant to de-elevate via the medium-IL shell, but that hand-off is
    //                       unreliable FROM an elevated process and the installer often never
    //                       started at all (observed: app exits, version stays old).
    // A direct CreateProcess simply inherits our token: when elevated (TUN on) it runs the
    // installer elevated — no ELEVATION_REQUIRED since we already hold admin; when not elevated
    // the `currentUser` (RequestExecutionLevel user) installer needs no elevation anyway. It
    // reliably starts in both cases, needs no console/COM/shell, and — crucially — we EXIT
    // ourselves right after, so the installer finds the exe unlocked and shows its full wizard
    // instead of stalling on a locked file or fighting a still-running same-or-higher-IL app.
    #[cfg(target_os = "windows")]
    {
        update_log(&format!(
            "launch: installer={:?} exists={} was_tun={} elevated={:?}",
            installer_path,
            installer_path.exists(),
            was_tun,
            process_is_elevated(),
        ));

        match std::process::Command::new(&installer_path).spawn() {
            Ok(child) => update_log(&format!("launch: installer spawned, pid={}", child.id())),
            Err(e) => {
                update_log(&format!("launch: spawn FAILED: {}", e));
                return Err(anyhow!("无法启动安装程序: {}", e));
            }
        }

        // Exit ourselves so the installer can replace the (possibly locked) binary and run its
        // full wizard. We don't rely on the installer force-closing us: under TUN we're
        // elevated, and leaving a running same-/higher-IL app is exactly what made the wizard
        // fail to appear before. The core was already torn down and the system proxy cleared
        // (shutdown_core), so exiting now is the same clean teardown the quit handler performs.
        // A short grace lets the installer fully start; it's an independent process and
        // survives our exit. The relaunched app restores proxy/TUN on next start as usual.
        update_log("launch: scheduling self-exit in 1500ms");
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            std::process::exit(0);
        });
    }

    // macOS: open the downloaded .dmg in Finder so the user can drag-install.
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&installer_path)
            .spawn()
            .map_err(|e| anyhow!("无法打开安装包: {}", e))?;
    }

    // Linux: open the downloaded package with the default handler.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&installer_path)
            .spawn()
            .map_err(|e| anyhow!("无法打开安装包: {}", e))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_vector() {
        // SHA-256("") and SHA-256("abc") — standard NIST test vectors.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn normalize_digest_strips_prefix_and_lowercases() {
        let h = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        assert_eq!(
            normalize_sha256_digest(&format!("sha256:{}", h)),
            Some(h.to_lowercase())
        );
        // Bare hex (no prefix) is accepted too.
        assert_eq!(normalize_sha256_digest(h), Some(h.to_lowercase()));
    }

    #[test]
    fn normalize_digest_rejects_malformed() {
        assert_eq!(normalize_sha256_digest(""), None);
        assert_eq!(normalize_sha256_digest("sha256:"), None);
        assert_eq!(normalize_sha256_digest("sha256:deadbeef"), None); // too short
        assert_eq!(normalize_sha256_digest("sha512:abc"), None);
        // Non-hex characters of otherwise-correct length.
        assert_eq!(normalize_sha256_digest(&"z".repeat(64)), None);
    }

    #[test]
    fn release_info_deserializes_without_sha256() {
        // Older cached ReleaseInfo (pre-digest) must still load — sha256 defaults to None.
        let json = r#"{"version":"1.10.0","published_at":"","release_notes":"","download_url":"u"}"#;
        let r: ReleaseInfo = serde_json::from_str(json).unwrap();
        assert_eq!(r.sha256, None);
    }
}
