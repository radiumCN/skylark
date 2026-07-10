use std::path::PathBuf;
use std::fs;
use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::Value;
use crate::types::{AppConfig, Subscription, ProxyNode};

/// Read + parse a JSON config file. A MISSING file yields `T::default()` (the normal first
/// run). A file that EXISTS but fails to parse is preserved as `<name>.corrupt` — rather than
/// being silently overwritten by the next atomic save — and `T::default()` is returned, so a
/// disk-level corruption or a bad hand-edit can be recovered instead of silently wiping the
/// user's data.
fn load_json_or_default<T: DeserializeOwned + Default>(path: &std::path::Path) -> T {
    let Ok(data) = fs::read_to_string(path) else {
        return T::default();
    };
    match serde_json::from_str::<T>(&data) {
        Ok(v) => v,
        Err(e) => {
            let bak = path.with_extension("corrupt");
            let _ = fs::rename(path, &bak);
            log::error!("配置 {:?} 解析失败（{}）；已备份为 {:?} 并回退默认值", path, e, bak);
            T::default()
        }
    }
}

/// User-Agent used when fetching subscriptions. Many airports gate the returned content
/// on the client UA: a legacy "Clash" identifier (e.g. `ClashForWindows`) makes
/// protocol-rich airports (vless-reality / hysteria2 / tuic) serve a "please switch
/// client" placeholder config (fake `ss` nodes on 127.0.0.1) instead of the real nodes,
/// because the original Clash core cannot handle those protocols. A modern, widely
/// whitelisted client identifier makes them return the universal Base64 node list (or a
/// Clash.Meta YAML), both of which the parser fully supports. Our core is sing-box, which
/// supports every protocol these airports serve.
pub const SUBSCRIPTION_USER_AGENT: &str = "v2rayN/6.45";

/// Effective subscription User-Agent: the user-configured value, or the built-in default
/// when it is unset/blank. Read fresh from the persisted config so a settings change takes
/// effect on the next fetch without restarting.
pub fn subscription_user_agent() -> String {
    let ua = load_app_config().subscription_user_agent;
    if ua.trim().is_empty() {
        SUBSCRIPTION_USER_AGENT.to_string()
    } else {
        ua
    }
}

pub fn app_data_dir() -> PathBuf {
    let base = dirs_next::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Skylark")
}

pub fn singbox_config_path() -> PathBuf {
    app_data_dir().join("config.json")
}

pub fn subscriptions_dir() -> PathBuf {
    app_data_dir().join("subscriptions")
}

/// Directory holding the locally-bundled sing-box rule-set (.srs) files.
/// These are copied from the app resources on startup so the generated config
/// can reference them by absolute path even where the remote CDN is blocked.
pub fn rule_sets_dir() -> PathBuf {
    app_data_dir().join("rule-sets")
}

pub fn ensure_dirs() -> Result<()> {
    fs::create_dir_all(app_data_dir())?;
    fs::create_dir_all(subscriptions_dir())?;
    fs::create_dir_all(rule_sets_dir())?;
    Ok(())
}

/// Write `data` to `path` atomically: write a sibling temp file, then rename it over the
/// target. `fs::write` truncates the target in place, so a crash/kill mid-write leaves a
/// half-written file behind — which `load_*`'s `unwrap_or_default()` then reads as invalid
/// JSON and silently resets to defaults, wiping every saved subscription / node / setting.
/// The temp-then-rename keeps the previous good file intact until the new one is complete.
/// `fs::rename` replaces the destination on both Windows (MoveFileEx REPLACE_EXISTING) and
/// Unix, so this is a safe cross-platform swap.
pub(crate) fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    // These files hold node passwords / secrets. On Unix, restrict them to the owner (0600)
    // so other local users can't read them (Windows app-data is already per-user ACL'd).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path)
}

/// Stable random secret guarding the Clash API (`external_controller`). Generated once
/// on first use and persisted to a dedicated file (NOT app_config.json, which round-trips
/// through the frontend and could otherwise be wiped on a settings save). Cached for the
/// process lifetime. Both the generated sing-box config and every Clash API caller read
/// this same value, so the `Authorization: Bearer <secret>` header always matches.
pub fn api_secret() -> String {
    static API_SECRET: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    API_SECRET
        .get_or_init(|| {
            let path = app_data_dir().join("api_secret");
            if let Ok(s) = fs::read_to_string(&path) {
                let s = s.trim().to_string();
                if !s.is_empty() {
                    return s;
                }
            }
            let secret = uuid::Uuid::new_v4().simple().to_string();
            let _ = ensure_dirs();
            // This secret is the only guard on the local Clash API (`external_controller`),
            // so it must be owner-only (0600 on Unix) — write it through the atomic helper
            // rather than a bare, world-readable `fs::write`.
            let _ = write_atomic(&path, secret.as_bytes());
            secret
        })
        .clone()
}

/// Preferred UI language for a fresh install, derived from the OS locale. A Chinese
/// system (`zh*`) maps to `zh-CN`; everything else falls back to English. Only used when
/// no `app_config.json` exists yet — once the user has a config, their saved choice wins.
fn detect_system_language() -> String {
    let locale = sys_locale::get_locale().unwrap_or_default().to_lowercase();
    if locale.starts_with("zh") {
        "zh-CN".to_string()
    } else {
        "en".to_string()
    }
}

pub fn load_app_config() -> AppConfig {
    let path = app_data_dir().join("app_config.json");
    match fs::read_to_string(&path) {
        Ok(data) => match serde_json::from_str(&data) {
            Ok(c) => c,
            Err(e) => {
                // Preserve the unparseable file for recovery instead of silently resetting.
                let bak = path.with_extension("corrupt");
                let _ = fs::rename(&path, &bak);
                log::error!("app_config.json 解析失败（{}）；已备份为 {:?} 并回退默认值", e, bak);
                AppConfig { language: detect_system_language(), ..AppConfig::default() }
            }
        },
        // Missing file → normal first run.
        Err(_) => AppConfig { language: detect_system_language(), ..AppConfig::default() },
    }
}

pub fn save_app_config(config: &AppConfig) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("app_config.json");
    let data = serde_json::to_string_pretty(config)?;
    write_atomic(&path, data.as_bytes())?;
    Ok(())
}

pub fn load_subscriptions() -> Vec<Subscription> {
    load_json_or_default(&app_data_dir().join("subscriptions.json"))
}

pub fn save_subscriptions(subs: &[Subscription]) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("subscriptions.json");
    let data = serde_json::to_string_pretty(subs)?;
    write_atomic(&path, data.as_bytes())?;
    Ok(())
}

pub fn load_nodes() -> Vec<ProxyNode> {
    load_json_or_default(&app_data_dir().join("nodes.json"))
}

pub fn save_nodes(nodes: &[ProxyNode]) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("nodes.json");
    let data = serde_json::to_string_pretty(nodes)?;
    write_atomic(&path, data.as_bytes())?;
    Ok(())
}

pub fn load_outbounds() -> Vec<Value> {
    load_json_or_default(&app_data_dir().join("outbounds.json"))
}

pub fn save_outbounds(outbounds: &[Value]) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("outbounds.json");
    let data = serde_json::to_string_pretty(outbounds)?;
    write_atomic(&path, data.as_bytes())?;
    Ok(())
}

pub fn load_proxy_groups() -> Vec<crate::types::ProxyGroup> {
    load_json_or_default(&app_data_dir().join("proxy_groups.json"))
}

pub fn save_proxy_groups(groups: &[crate::types::ProxyGroup]) -> Result<()> {
    ensure_dirs()?;
    let path = app_data_dir().join("proxy_groups.json");
    let data = serde_json::to_string_pretty(groups)?;
    write_atomic(&path, data.as_bytes())?;
    Ok(())
}

/// Guard the subscription `id` before using it as a filename component. Ids are
/// backend-generated UUIDs, but they cross the frontend command boundary (delete / load),
/// so reject anything that isn't a plain slug to prevent path traversal (e.g. `../../…`)
/// out of the subscriptions dir.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Cache the raw text content of a subscription so it can be re-parsed on startup.
pub fn save_subscription_content(id: &str, content: &str) -> Result<()> {
    if !is_safe_id(id) {
        return Err(anyhow::anyhow!("非法订阅 ID"));
    }
    ensure_dirs()?;
    let path = subscriptions_dir().join(format!("{}.txt", id));
    write_atomic(&path, content.as_bytes())?;
    Ok(())
}

pub fn load_subscription_content(id: &str) -> Option<String> {
    if !is_safe_id(id) {
        return None;
    }
    let path = subscriptions_dir().join(format!("{}.txt", id));
    fs::read_to_string(path).ok()
}

pub fn delete_subscription_content(id: &str) {
    if !is_safe_id(id) {
        return;
    }
    let path = subscriptions_dir().join(format!("{}.txt", id));
    let _ = fs::remove_file(path);
}
