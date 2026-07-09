use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    pub sub_type: SubType,
    pub node_count: usize,
    pub last_update: Option<DateTime<Utc>>,
    pub auto_update: bool,
    pub update_interval: u32, // hours
    /// Airport usage / quota parsed from the `Subscription-Userinfo` response header.
    /// All optional — present only when the provider returns the header. `serde(default)`
    /// keeps older subscriptions.json (without these fields) deserializable.
    #[serde(default)]
    pub upload: Option<u64>,   // bytes used (upload)
    #[serde(default)]
    pub download: Option<u64>, // bytes used (download)
    #[serde(default)]
    pub total: Option<u64>,    // total quota in bytes
    #[serde(default)]
    pub expire: Option<i64>,   // expiry as unix timestamp (seconds)
    /// Keyword/regex filters applied to node names after parsing. `include`: keep only
    /// nodes whose name matches; `exclude`: drop nodes whose name matches. An empty/None
    /// value disables that filter; an invalid regex is treated as disabled (never drops
    /// everything). `serde(default)` keeps older subscriptions.json loadable.
    #[serde(default)]
    pub include: Option<String>,
    #[serde(default)]
    pub exclude: Option<String>,
    /// When true, each node's `group` is set to its detected region (from name flag/keyword)
    /// instead of the default group.
    #[serde(default)]
    pub group_by_region: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SubType {
    Clash,
    V2ray,
    Sip008,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyNode {
    pub id: String,
    pub name: String,
    pub group: String,
    pub protocol: String,
    pub server: String,
    pub port: u16,
    pub latency: Option<u32>,        // ms
    pub download_speed: Option<u32>, // KB/s
    pub is_active: bool,
    pub subscription_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedResult {
    pub latency_ms: Option<u32>,
    pub download_kbps: Option<u32>,
}

/// A user-defined proxy group. sing-box natively supports only `selector` (manual
/// pick) and `urltest` (auto, lowest-latency) group types — those are the two allowed
/// values for `group_type`. `nodes` holds member node names (outbound tags).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyGroup {
    pub id: String,
    pub name: String,
    pub group_type: String,
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub network: String,
    pub conn_type: String,
    pub source: String,
    pub destination: String,
    pub host: String,
    pub rule: String,
    pub rule_payload: String,
    pub chains: Vec<String>,
    pub upload: u64,
    pub download: u64,
    pub start: String,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub proxy_mode: ProxyMode,
    pub startup_with_system: bool,
    pub startup_minimized: bool,
    pub allow_lan: bool,
    pub http_port: u16,
    pub socks_port: u16,
    pub mixed_port: u16,
    pub api_port: u16,
    pub tun_enabled: bool,
    pub log_level: String,
    pub theme: String,
    /// UI density: "standard" or "compact"
    #[serde(default = "default_density")]
    pub density: String,
    pub language: String,
    pub selected_subscription: Option<String>,
    pub active_nodes: std::collections::HashMap<String, String>,
    /// 0 = disabled, otherwise check every N hours
    pub auto_update_interval: u32,
    pub auto_update_notify: bool,
    /// App self-update channel: "stable" or "beta"
    #[serde(default = "default_update_channel")]
    pub update_channel: String,
    /// true = close button minimizes to tray; false = exits the app
    pub close_to_tray: bool,
    /// Restore proxy running state (sing-box + system proxy) on next startup
    pub restore_proxy_on_startup: bool,
    /// Last known sing-box running state (written on start/stop). Backend-owned runtime
    /// state: never overwritten from a frontend save / config import — see
    /// `merge_runtime_fields` in commands.rs.
    #[serde(default)]
    pub last_proxy_running: bool,
    /// Last known system proxy enabled state (written on start/stop). Backend-owned, same
    /// rule as `last_proxy_running`.
    #[serde(default)]
    pub last_system_proxy: bool,
    /// URLTest probe URL for the auto-select (urltest) groups.
    #[serde(default = "default_auto_test_url")]
    pub auto_test_url: String,
    /// URLTest re-evaluation interval, in minutes.
    #[serde(default = "default_auto_test_interval")]
    pub auto_test_interval: u32,
    /// URLTest tolerance, in milliseconds: only switch when the new best beats the
    /// current node by more than this margin (avoids flapping between close nodes).
    #[serde(default = "default_auto_tolerance")]
    pub auto_tolerance: u32,
    /// Enable IPv6: dual-stack DNS (`prefer_ipv4`) + fake-ip inet6 range + IPv6 TUN
    /// address. Default off keeps the previous IPv4-only behaviour.
    #[serde(default)]
    pub enable_ipv6: bool,
    /// Domestic / direct DNS resolver. A plain IP (e.g. `223.5.5.5`) maps to a UDP
    /// server; a `https://…` URL maps to DoH; a `tls://…` URL maps to DoT.
    #[serde(default = "default_dns_local")]
    pub dns_local: String,
    /// Persist core logs to a daily rolling file (`logs/skylark-YYYYMMDD.log`) as they
    /// arrive, so they survive a crash. Default off (in-memory ring buffer only).
    #[serde(default)]
    pub log_to_file: bool,
    /// User-Agent sent when fetching subscriptions. Airports gate the returned content on
    /// the client UA; a legacy "Clash" UA can yield a "switch client" placeholder instead
    /// of real nodes. Defaults to a modern, widely whitelisted client identifier. An empty
    /// value falls back to that default (see `config::subscription_user_agent`).
    #[serde(default = "default_subscription_user_agent")]
    pub subscription_user_agent: String,
    /// Enable global hotkeys (toggle system proxy / TUN / cycle mode). Default off so the
    /// app never silently grabs system-wide key combos until the user opts in.
    #[serde(default)]
    pub enable_global_shortcuts: bool,
    /// App version that last wrote this config. Used on startup to detect a just-upgraded
    /// launch (`last_app_version != CARGO_PKG_VERSION`), which is the one case where the
    /// installer force-killed the previous core and may have left stale TUN routes behind —
    /// the restored tunnel then black-holes until a manual off→on. See the post-upgrade
    /// TUN self-heal in `lib.rs`. Empty on a fresh install / pre-upgrade config.
    #[serde(default)]
    pub last_app_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    Rule,
    Global,
    Direct,
    Tun,
}

fn default_update_channel() -> String {
    "stable".to_string()
}

fn default_density() -> String {
    "standard".to_string()
}

fn default_auto_test_url() -> String {
    "https://www.gstatic.com/generate_204".to_string()
}

fn default_auto_test_interval() -> u32 {
    3
}

fn default_auto_tolerance() -> u32 {
    50
}

fn default_dns_local() -> String {
    "223.5.5.5".to_string()
}

fn default_subscription_user_agent() -> String {
    crate::config::SUBSCRIPTION_USER_AGENT.to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            proxy_mode: ProxyMode::Rule,
            startup_with_system: false,
            startup_minimized: false,
            allow_lan: false,
            http_port: 7890,
            socks_port: 7891,
            mixed_port: 7890,
            api_port: 9090,
            tun_enabled: false,
            log_level: "info".to_string(),
            theme: "system".to_string(),
            density: default_density(),
            language: "zh-CN".to_string(),
            selected_subscription: None,
            active_nodes: std::collections::HashMap::new(),
            auto_update_interval: 24,
            auto_update_notify: true,
            update_channel: "stable".to_string(),
            close_to_tray: true,
            restore_proxy_on_startup: true,
            last_proxy_running: false,
            last_system_proxy: false,
            auto_test_url: default_auto_test_url(),
            auto_test_interval: default_auto_test_interval(),
            auto_tolerance: default_auto_tolerance(),
            enable_ipv6: false,
            dns_local: default_dns_local(),
            log_to_file: false,
            subscription_user_agent: default_subscription_user_agent(),
            enable_global_shortcuts: false,
            last_app_version: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingboxStatus {
    pub running: bool,
    pub uptime: Option<u64>, // seconds
    pub pid: Option<u32>,
    pub version: Option<String>,
}
