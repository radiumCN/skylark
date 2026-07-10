use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::config;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Proxy,
    Direct,
    Block,
    Dns,
}

impl std::fmt::Display for RuleAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Proxy => write!(f, "proxy"),
            Self::Direct => write!(f, "direct"),
            Self::Block => write!(f, "block"),
            Self::Dns => write!(f, "dns-out"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub action: RuleAction,
    /// Match by domain (exact)
    pub domain: Vec<String>,
    /// Match by domain suffix
    pub domain_suffix: Vec<String>,
    /// Match by domain keyword
    pub domain_keyword: Vec<String>,
    /// Match by GeoSite tag (e.g. "cn", "google")
    pub geosite: Vec<String>,
    /// Match by GeoIP country code (e.g. "cn", "private")
    pub geoip: Vec<String>,
    /// Match by CIDR block
    pub ip_cidr: Vec<String>,
    /// Match by port (single or range like "80,443,8080-8090")
    pub port: Vec<String>,
    /// Match by protocol (tcp/udp)
    pub network: Option<String>,
    /// Match by process name
    pub process_name: Vec<String>,
}

impl RouteRule {
    pub fn new_empty(name: &str, action: RuleAction) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            enabled: true,
            action,
            domain: vec![],
            domain_suffix: vec![],
            domain_keyword: vec![],
            geosite: vec![],
            geoip: vec![],
            ip_cidr: vec![],
            port: vec![],
            network: None,
            process_name: vec![],
        }
    }
}

/// Built-in preset rules
pub fn preset_rules() -> Vec<RouteRule> {
    vec![
        // ── 基础 ────────────────────────────────────────────────────
        {
            let mut r = RouteRule::new_empty("DNS 流量", RuleAction::Dns);
            r.port = vec!["53".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("拦截广告", RuleAction::Block);
            r.geosite = vec!["category-ads-all".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("本地及私有地址直连", RuleAction::Direct);
            r.domain_suffix  = vec!["local".into(), "localhost".into()];
            r.ip_cidr = vec![
                "10.0.0.0/8".into(),
                "100.64.0.0/10".into(),
                "127.0.0.0/8".into(),
                "172.16.0.0/12".into(),
                "192.168.0.0/16".into(),
                "198.18.0.0/16".into(),
                "169.254.0.0/16".into(),
                "::1/128".into(),
                "fc00::/7".into(),
            ];
            r.geoip = vec!["private".into()];
            r
        },
        // ── 国际服务走代理 ──────────────────────────────────────────
        {
            let mut r = RouteRule::new_empty("Google", RuleAction::Proxy);
            r.geosite = vec!["google".into()];
            r.geoip   = vec!["google".into()];
            r.domain_suffix = vec![
                "googleapis.com".into(),
                "gstatic.com".into(),
                "ggpht.com".into(),
                "ytimg.com".into(),
                "googlevideo.com".into(),
                "googleusercontent.com".into(),
            ];
            r.domain_keyword = vec!["google".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("YouTube", RuleAction::Proxy);
            r.geosite = vec!["youtube".into()];
            r.domain_suffix = vec!["youtube.com".into(), "youtu.be".into()];
            r.domain_keyword = vec!["youtube".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("GitHub", RuleAction::Proxy);
            r.geosite = vec!["github".into(), "githubusercontent".into()];
            r.domain_suffix = vec!["github.com".into(), "githubusercontent.com".into(), "githubassets.com".into()];
            r.domain_keyword = vec!["github".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Telegram", RuleAction::Proxy);
            r.geosite = vec!["telegram".into()];
            r.domain_suffix = vec!["t.me".into(), "telegram.org".into(), "telegram.me".into()];
            r.domain_keyword = vec!["telegram".into()];
            r.ip_cidr = vec!["91.108.0.0/16".into(), "149.154.160.0/20".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Twitter / X", RuleAction::Proxy);
            r.geosite = vec!["twitter".into()];
            r.geoip   = vec!["twitter".into()];
            r.domain_suffix = vec!["twitter.com".into(), "x.com".into(), "twimg.com".into()];
            r.domain_keyword = vec!["twitter".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Facebook / Instagram", RuleAction::Proxy);
            r.geosite = vec!["facebook".into(), "instagram".into()];
            r.geoip   = vec!["facebook".into()];
            r.domain_suffix = vec![
                "facebook.com".into(), "fb.com".into(), "fbcdn.net".into(),
                "instagram.com".into(), "cdninstagram.com".into(),
                "whatsapp.com".into(), "whatsapp.net".into(),
            ];
            r.domain_keyword = vec!["facebook".into(), "instagram".into(), "whatsapp".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Reddit", RuleAction::Proxy);
            r.domain_suffix = vec!["reddit.com".into(), "redd.it".into(), "redditmedia.com".into(), "redditstatic.com".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Wikipedia", RuleAction::Proxy);
            r.domain_suffix = vec!["wikipedia.org".into(), "wikimedia.org".into(), "wikidata.org".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Discord", RuleAction::Proxy);
            r.domain_suffix = vec!["discord.com".into(), "discordapp.com".into(), "discord.gg".into(), "discordapp.net".into()];
            r.domain_keyword = vec!["discord".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Netflix", RuleAction::Proxy);
            r.geosite = vec!["netflix".into()];
            r.domain_suffix = vec!["netflix.com".into(), "nflxvideo.net".into(), "nflximg.net".into(), "nflxext.com".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Twitch", RuleAction::Proxy);
            r.domain_suffix = vec!["twitch.tv".into(), "twitchapps.com".into(), "jtvnw.net".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Spotify", RuleAction::Proxy);
            r.domain_suffix = vec!["spotify.com".into(), "scdn.co".into(), "spotifycdn.com".into()];
            r.domain_keyword = vec!["spotify".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("AI 服务", RuleAction::Proxy);
            r.domain_suffix = vec![
                "openai.com".into(), "oaistatic.com".into(), "oaiusercontent.com".into(),
                "chatgpt.com".into(),
                "anthropic.com".into(), "claude.ai".into(),
                "perplexity.ai".into(),
                "gemini.google.com".into(),
                "copilot.microsoft.com".into(),
            ];
            r.domain_keyword = vec!["openai".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("加密货币", RuleAction::Proxy);
            r.domain_suffix = vec![
                "binance.com".into(), "binance.me".into(),
                "coinbase.com".into(), "bybit.com".into(),
                "okx.com".into(), "kraken.com".into(),
            ];
            r.domain_keyword = vec!["binance".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("国际云服务", RuleAction::Proxy);
            r.domain_suffix = vec![
                "amazonaws.com".into(), "aws.amazon.com".into(),
                "cloudfront.net".into(),
                "dropbox.com".into(), "dropboxapi.com".into(),
                "medium.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("国际流媒体", RuleAction::Proxy);
            r.geosite = vec!["disney".into(), "hbo".into()];
            r.domain_suffix = vec![
                "disneyplus.com".into(), "hulu.com".into(),
                "hbomax.com".into(), "max.com".into(),
                "primevideo.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("TikTok", RuleAction::Proxy);
            r.geosite = vec!["tiktok".into()];
            r.domain_suffix = vec!["tiktok.com".into(), "tiktokcdn.com".into(), "musical.ly".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("Steam", RuleAction::Proxy);
            r.geosite = vec!["steam".into()];
            r.domain_suffix = vec![
                "steampowered.com".into(), "steamcommunity.com".into(),
                "steamstatic.com".into(), "steamcdn-a.akamaihd.net".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("Speedtest", RuleAction::Proxy);
            r.domain_suffix = vec!["speedtest.net".into(), "fast.com".into()];
            r
        },
        // ── 国内服务直连 ─────────────────────────────────────────────
        {
            let mut r = RouteRule::new_empty("阿里系直连", RuleAction::Direct);
            r.geosite = vec!["alibaba".into()];
            r.domain_suffix = vec![
                "taobao.com".into(), "tmall.com".into(),
                "alicdn.com".into(), "tbcdn.cn".into(),
                "alipay.com".into(), "alibaba.com".into(),
                "aliyun.com".into(), "aliyuncs.com".into(),
                "mmstat.com".into(), "amap.com".into(),
                "autonavi.com".into(), "dingtalk.com".into(),
                "1688.com".into(), "ele.me".into(),
            ];
            r.domain_keyword = vec!["alicdn".into(), "alipay".into(), "taobao".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("腾讯系直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "qq.com".into(), "wechat.com".into(), "weixin.com".into(),
                "tencent.com".into(), "gtimg.cn".into(), "qpic.cn".into(),
                "myqcloud.com".into(), "tenpay.com".into(), "qcloud.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("百度系直连", RuleAction::Direct);
            r.geosite = vec!["baidu".into()];
            r.domain_suffix = vec![
                "baidu.com".into(), "bdstatic.com".into(),
                "bcebos.com".into(), "baidupcs.com".into(),
            ];
            r.domain_keyword = vec!["baidu".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("字节跳动直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "bytedance.com".into(), "toutiao.com".into(),
                "douyin.com".into(), "douyincdn.com".into(),
                "pstatp.com".into(), "snssdk.com".into(),
                "ixigua.com".into(), "feishu.cn".into(), "feishu.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("哔哩哔哩直连", RuleAction::Direct);
            r.geosite = vec!["bilibili".into()];
            r.domain_suffix = vec![
                "bilibili.com".into(), "bilivideo.com".into(),
                "hdslb.com".into(), "acgvideo.com".into(),
            ];
            r.domain_keyword = vec!["bilibili".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("京东直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "jd.com".into(), "jdcdn.com".into(), "360buyimg.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("美团直连", RuleAction::Direct);
            r.domain_suffix = vec!["meituan.com".into(), "meituan.net".into(), "dianping.com".into(), "dpfile.com".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("微博直连", RuleAction::Direct);
            r.domain_suffix = vec!["weibo.com".into(), "sinaimg.cn".into(), "sina.com".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("网易直连", RuleAction::Direct);
            r.domain_suffix = vec!["163.com".into(), "126.net".into(), "netease.com".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("小红书 / 拼多多直连", RuleAction::Direct);
            r.domain_suffix = vec!["xiaohongshu.com".into(), "pinduoduo.com".into(), "yangkeduo.com".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("知乎 / 豆瓣直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "zhihu.com".into(), "zhimg.com".into(),
                "douban.com".into(), "doubanio.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("CSDN直连", RuleAction::Direct);
            r.domain_suffix = vec!["csdn.net".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("国内视频直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "iqiyi.com".into(), "qiyipic.com".into(),
                "youku.com".into(), "ykimg.com".into(),
                "sohu.com".into(), "kuaishou.com".into(), "yximgs.com".into(),
                "mgtv.com".into(), "migu.cn".into(),
                "douyu.com".into(), "huya.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("国内电商直连", RuleAction::Direct);
            r.domain_suffix = vec![
                "suning.com".into(), "vip.com".into(),
                "ctrip.com".into(), "qunar.com".into(),
                "58.com".into(), "zhipin.com".into(),
                "ximalaya.com".into(), "sogou.com".into(),
                "12306.com".into(), "12306.cn".into(),
                "360.com".into(),
            ];
            r
        },
        {
            let mut r = RouteRule::new_empty("小米直连", RuleAction::Direct);
            r.domain_suffix = vec!["xiaomi.com".into(), "mi.com".into(), "miui.com".into()];
            r.domain_keyword = vec!["xiaomi".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("华为直连", RuleAction::Direct);
            r.domain_suffix = vec!["huawei.com".into(), "vmall.com".into(), "hicloud.com".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("苹果服务直连", RuleAction::Direct);
            r.geosite = vec!["apple".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("微软服务直连", RuleAction::Direct);
            r.geosite = vec!["microsoft".into()];
            r
        },
        // ── 兜底 ────────────────────────────────────────────────────
        {
            let mut r = RouteRule::new_empty("中国大陆直连 (GeoIP)", RuleAction::Direct);
            r.geoip = vec!["cn".into()];
            r
        },
        {
            let mut r = RouteRule::new_empty("中国大陆直连 (GeoSite)", RuleAction::Direct);
            r.geosite = vec!["cn".into()];
            r.domain_suffix = vec!["cn".into()];
            r
        },
    ]
}

pub fn load_rules() -> Vec<RouteRule> {
    let path = config::app_data_dir().join("rules.json");
    if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        // First run: use preset rules
        preset_rules()
    }
}

pub fn save_rules(rules: &[RouteRule]) -> Result<()> {
    config::ensure_dirs()?;
    let path = config::app_data_dir().join("rules.json");
    let data = serde_json::to_string_pretty(rules)?;
    config::write_atomic(&path, data.as_bytes())?;
    Ok(())
}

// ─── Remote rule-set providers ───────────────────────────────────────

fn default_provider_format() -> String {
    "binary".to_string()
}

/// A user-added remote rule-set (sing-box rule-set / `.srs` or source `.json`).
/// Matched domains/IPs in the set are routed to `action`'s outbound.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleProvider {
    pub id: String,
    pub name: String,
    pub url: String,
    pub action: RuleAction,
    pub enabled: bool,
    /// sing-box rule-set format: "binary" (.srs) or "source" (.json).
    #[serde(default = "default_provider_format")]
    pub format: String,
}

/// Guess the rule-set format from a URL's extension; defaults to binary (.srs).
pub fn guess_provider_format(url: &str) -> String {
    let lower = url.split('?').next().unwrap_or(url).to_lowercase();
    if lower.ends_with(".json") {
        "source".to_string()
    } else {
        "binary".to_string()
    }
}

pub fn load_rule_providers() -> Vec<RuleProvider> {
    let path = config::app_data_dir().join("rule_providers.json");
    if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub fn save_rule_providers(providers: &[RuleProvider]) -> Result<()> {
    config::ensure_dirs()?;
    let path = config::app_data_dir().join("rule_providers.json");
    let data = serde_json::to_string_pretty(providers)?;
    config::write_atomic(&path, data.as_bytes())?;
    Ok(())
}

