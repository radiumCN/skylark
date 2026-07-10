# Skylark（云雀）改进计划

> 基于对当前代码库（Rust 后端 + Vue 3 前端 + Tauri v2）的完整审阅整理。
> 目标：先修确定性 Bug，再还工程债，最后补功能。每项均含「问题 / 根因 / 方案 / 影响文件 / 风险 / 验证」。

> **进度更新（2026-06-25）**：P0 批次（B1 / B2 / B3）、B4、Q2、F1、S1、Q3、Q4、F3 已完成；Q1 巨型函数已拆为 5 个纯函数并补至 25 个测试；F2 完成 SS 健壮性批次（base64 修复 + plugin）；全量改名为品牌 `Skylark` 并同步远端仓库 `radiumCN/skylark`；F4 完成配置导入/导出（备份）批次，并核实节点搜索/批量测速本已存在、TUN mtu 9000 为内核默认。F2 批次 2/3 完成 Clash YAML 的 SS plugin 映射、hysteria2 健壮性（端口跳跃/pinSHA256/salamander/alpn）、vmess `scy` + vless `packet_encoding` + alpn 查漏；F4 全部完成（配置备份 + 日志可选落盘/事件增量推送）；F5 修复订阅获取健壮性（UA 默认值+可配置、节点名 percent-decode 乱码）。后端 `cargo test --lib` 37 passed、前端 `vue-tsc` 均通过。仅剩 F2 WireGuard（暂缓，需 1.12 endpoint 模型）。

## 优先级总览

| 优先级 | 编号 | 标题 | 类型 | 状态 |
|--------|------|------|------|------|
| P0 | B1 | 应用自更新版本号比较错误（永远提示更新） | Bug | ✅ 已完成 |
| P0 | B2 | macOS `pkill -f sing-box` 误杀 App 自身 | Bug | ✅ 已完成 |
| P0 | B3 | 单节点「测延迟/测速」在代理运行时测的不是该节点 | Bug | ✅ 已完成 |
| P1 | B4 | 打包却从未使用的规则集资源 | 质量/体积 | ✅ 已完成 |
| P1 | Q1 | `build_singbox_config` 巨型函数 + 零测试 | 可维护性 | ✅ 已完成（拆 5 纯函数 + 25 测试） |
| P1 | Q2 | Global 模式下系统代理仍旁路 CN 域名 | 行为一致性 | ✅ 已完成 |
| P2 | F1 | 订阅流量 / 到期信息未解析 | 功能 | ✅ 已完成 |
| P2 | F2 | 协议覆盖不全（SS plugin / SSR / WireGuard 等） | 功能 | 🟡 SS 链接+Clash SS plugin 完成；WG 暂缓 |
| P2 | F3 | IPv6 被完全关闭、DNS 写死不可配 | 功能 | ✅ 已完成 |
| P3 | Q3 | CN 域名 / WeChat 名单 4 处重复硬编码 | 去重 | ✅ 已完成 |
| P3 | Q4 | 版本与命名不一致 | 规范 | ✅ 已完成（命名约定已确立） |
| P3 | S1 | Clash API 无鉴权（secret 为空） | 安全 | ✅ 已完成 |
| P3 | F4 | 日志落盘、配置导入导出、节点搜索等小功能 | 功能 | ✅ 已完成（备份 + 日志落盘/推送） |

---

## P0 — 确定性 Bug

### B1. 应用自更新版本号比较错误 ✅ 已完成（2026-06-25）
> 实施：`Cargo.toml` 版本 `0.1.2` → `0.2.7`（与 `package.json` 对齐）；`auto_update.rs` 新增 `is_newer_version()` 语义化比较，替换应用自更新与内核更新两处 `!=`。`cargo check` 通过。
- **问题**：自更新始终提示「有新版本」。
- **根因**：版本号双源不一致。`tauri.conf.json` 取 `package.json`（`0.2.7`），但 `auto_update.rs` 用 `env!("CARGO_PKG_VERSION")`（`Cargo.toml` 的 `0.1.2`）比较，且用 `!=` 而非语义化大小比较。
  - `src-tauri/src/auto_update.rs:151-153`
- **方案**：
  1. 统一版本源（让 `Cargo.toml` 与 `package.json` 保持一致，或构建期注入）。
  2. 比较逻辑改为「latest > current」的语义化版本比较（避免 beta→stable 回退误报）。
- **影响文件**：`src-tauri/Cargo.toml`、`package.json`、`src-tauri/src/auto_update.rs`、`src-tauri/src/updater.rs`
- **风险**：版本源调整需确认 CI/打包脚本读取处。
- **验证**：本地版本等于最新 release 时不弹更新；低于时才弹。

### B2. macOS `pkill -f sing-box` 误杀 App 自身 ✅ 已完成（2026-06-25）
> 实施：`singbox.rs` 的 unix 清理逻辑由 `pkill -f sing-box` 改为 `pkill -f "sing-box run -c"`，仅匹配内核的启动签名（GUI 从不带 `run -c`）。Windows `taskkill /IM` 路径未改动（属现有发布行为，超出本次范围）。
- **问题**：macOS 启动代理时清理「孤儿内核」可能把 GUI 进程自己杀掉。
- **根因**：`pkill -f` 匹配完整命令行，而 App 路径/进程名含 `sing-box`。
  - `src-tauri/src/singbox.rs:117-128`
- **方案**：精确匹配内核可执行文件**绝对路径**，或 `pgrep` 后排除当前进程 PID（`std::process::id()`）再 kill；Windows 侧 `taskkill /IM sing-box.exe` 同样可考虑按路径精确化。
- **影响文件**：`src-tauri/src/singbox.rs`
- **风险**：低；仅清理逻辑收紧。
- **验证**：macOS 上反复开关代理，GUI 不退出；确有孤儿内核时仍被清理。

### B3. 单节点测延迟/测速测的不是该节点 ✅ 已完成（2026-06-25）
> 实施：`commands.rs` 新增 `clash_proxy_delay()`（`GET /proxies/{name}/delay`，节点名按 path 段编码）；`cmd_test_node_latency` 与 `cmd_test_node_speed` 在代理运行时改用按节点的 delay API 测延迟；删除已成死代码的 `test_via_proxy`。下载速度仍走本地 mixed 端口（反映当前选中节点吞吐，属 sing-box 限制，UI 已有说明）。`cargo check` 通过。
- **问题**：代理运行时对任意节点点「测延迟」，返回的都是**当前生效节点**的延迟，并写回到每个被点的节点。
- **根因**：`test_via_proxy` / `measure_download_speed` 走本地 mixed 端口 → `proxy` selector → 当前节点，与 `node_id` 无关。
  - `src-tauri/src/commands.rs:515-521`、`543-589`
- **方案**：代理运行时改用 Clash API `GET /proxies/{name}/delay`（分组测速 `cmd_test_group_delay` 已是正确范式）。测速可保留「仅对当前/选中节点」语义并在 UI 说明。
- **影响文件**：`src-tauri/src/commands.rs`、`src/views/Nodes.vue`（必要时调整文案）
- **风险**：需对节点名做 URL path 段编码（含空格/Unicode），参考 `cmd_test_group_delay` 已有实现。
- **验证**：开代理后对多个节点分别测延迟，得到各自不同且合理的值。

---

## P1 — 质量 / 一致性

### B4. 打包却未使用的规则集资源 ✅ 已完成（2026-06-25，采用方案 B）
> 实施：从 `tauri.conf.json` 的 `bundle.resources` 与 `lib.rs` 的复制数组中移除 `geosite-geolocation-noncn.srs`，并删除该 162 KB 资源文件。默认路由 `final: proxy` 已覆盖非中国域名，移除不改变分流行为。
- **问题**：`geosite-geolocation-noncn.srs` 被打包并复制到数据目录，但配置生成从未引用，纯增体积。
  - 打包：`src-tauri/tauri.conf.json:46`；复制：`src-tauri/src/lib.rs:138-147`；引用缺失：`src-tauri/src/subscription.rs`（仅 `geosite-cn`/`geoip-cn`）
- **方案**（二选一）：
  - A：在路由中加入 `geosite-geolocation-noncn → proxy` 规则，提升非中国域名分流精度（推荐）。
  - B：从 `resources` 与复制逻辑中移除。
- **影响文件**：`subscription.rs` + `lib.rs`（方案A）或 `tauri.conf.json` + `lib.rs`（方案B）
- **风险**：方案A 改变默认分流行为，需回归常见站点。
- **验证**：配置中存在对应 `rule_set` 且命中正确出站。

### Q1. `build_singbox_config` 巨型函数 + 零测试 ✅ 已完成（2026-06-25）
- **问题**：单函数约 500 行承载分组/DNS/路由/入站/TUN 全部逻辑；整个项目原先无任何测试，而解析与配置生成恰是最易回归的部分。
  - `src-tauri/src/subscription.rs:759-1266`
- **本次完成（先锁定现状）**：在 `subscription.rs` 内新增 `#[cfg(test)] mod tests`，11 个特征/单元测试，`cargo test --lib` 全绿：
  - 各协议链接解析：`vmess`(ws+tls / 纯 tcp)、`vless`(reality 不带 insecure)、`ss`(明文回退路径)、`trojan`(allowInsecure)、`hysteria2`(hy2 归一化)、`tuic`(alpn 拆分)、`anytls`；`detect_sub_type` 四类识别；`parse_node_link` 未知协议报错。
  - `build_singbox_config` 结构不变量：`dns/inbounds/route.rules` 形态、节点透传、`proxy` selector 与 `direct` 出站存在、`clash_api.external_controller` 绑回环端口、`secret` 非空——不依赖磁盘上的用户规则/分组，可稳定运行。
- **副产物（已记录，待后续修）**：`parse_ss` 对标准带 padding 的 SIP008 base64 userinfo 解析失败（`url` crate 按 WHATWG 把 `=` 编码为 `%3D`），当前回退为明文 `method:password`；属 F2 协议健壮性范畴。
- **拆分完成（2026-06-25）**：将约 500 行的 `build_singbox_config` 拆为 5 个纯函数，主函数退化为编排者（只保留 rule-set 定义、server/CN 域名列表、最终 JSON 组装）：
  - `build_proxy_outbounds(outbounds, config, active_tag, nodes) -> Vec<Value>`：proxy selector + direct/block + auto/订阅/自定义分组 + 清洗后的节点出站。
  - `build_dns_rules(server_domains, cn_core_domains, user_rules) -> Vec<Value>`：本地解析器（真 IP）DNS 规则，含用户 DIRECT 域名提取。
  - `build_route_rules(cn_core_domains, user_rules, rule_providers) -> (route_rules, provider_rule_sets)`：路由规则 + 用户 rule-set provider 远程定义。
  - `build_inbounds(config) -> Vec<Value>`：mixed/http/socks 入站（端口去重）。
  - `build_tun_inbound(config) -> Value`：TUN 入站（IPv6 按开关、Windows 唯一网卡名）。
  - 逻辑、注释、字段顺序逐字保留；行为不变由原 19 个端到端测试兜底。
- **新增 6 个直接单测**（针对各 `build_*` 纯函数）：端口去重 / LAN 监听、DNS 规则优先级与 fakeip 收尾、路由首 sniff 尾 catch-all、空节点 selector 回退 direct、TUN 地址按 IPv6 开关增减。
- **副产物（已在 F2 解决）**：`parse_ss` SIP002 base64 userinfo 解析（见 F2）。
- **影响文件**：`src-tauri/src/subscription.rs`（重构 + 测试模块扩充）。
- **验证**：`cargo test --lib` → 25 passed（19 旧 + 6 新）；`cargo check` 通过；无 lint。

### Q2. Global 模式下系统代理仍旁路 CN 域名 ✅ 已完成（2026-06-25）
> 实施：`proxy.rs` 的 `set_system_proxy` 增加 `global_mode` 形参——本地/私有网段恒定旁路，CN 域名清单仅在非全局模式下追加；全局模式下 `ProxyOverride` 只剩本地网段。`commands.rs` 的 6 个调用点同步传参（启用时按 `proxy_mode == Global` 取值），并在 `cmd_set_proxy_mode` 运行时切换模式后立即重写 `ProxyOverride`（系统代理开启且非 TUN 时），无需重连即生效。macOS 无 per-domain 旁路、形参忽略。`cargo check` 通过。
- **问题**：`ProxyOverride` 把大量 CN 域名写死直连（WinINet 层），全局模式本意「全部走代理」时仍被旁路，行为不一致。
  - `src-tauri/src/proxy.rs:26-51`
- **方案**：`ProxyOverride` 按当前 `proxy_mode` 动态生成——全局模式仅保留本地/私有网段，去掉 CN 域名旁路。
- **影响文件**：`src-tauri/src/proxy.rs`、调用处 `commands.rs`（传入模式）
- **风险**：WeChat 截图翻译等依赖旁路的功能在全局模式下行为改变（属预期）。
- **验证**：全局模式下访问 CN 域名确实经代理；规则模式维持现状。

---

## P2 — 功能缺口

### F1. 订阅流量 / 到期信息 ✅ 已完成（2026-06-25）
> 实施：`types.rs` 的 `Subscription` 新增 `upload/download/total/expire`（均 `serde(default)`，兼容旧数据）；`commands.rs` 新增 `parse_userinfo()` 解析 `Subscription-Userinfo` 头，`fetch_url` 改为返回 `(内容, 用量)`；添加 / 更新订阅与后台自动更新（`auto_update.rs`）均写入用量，且刷新缺头时不覆盖已知值。前端 `stores/app.ts` 扩展接口、`Subscriptions.vue` 新增「已用 / 总量 + 进度条 + 到期日（过期标红）」展示，含字节格式化与百分比着色。`cargo check` + `vue-tsc` 通过。
- **价值**：同类客户端标配。`fetch_url` 当前丢弃响应头。
  - `src-tauri/src/commands.rs:1327-1337`
- **方案**：解析 `Subscription-Userinfo` 头（`upload/download/total/expire`），存入 `Subscription` 结构并在订阅页展示「已用 / 剩余 / 到期」。
- **影响文件**：`commands.rs`、`types.rs`、`auto_update.rs`（自动更新路径同样解析）、`src/views/Subscriptions.vue`、`stores/app.ts`
- **风险**：低；字段缺失需容错。

### F2. 协议覆盖不全 🟡 批次 1-3 已完成（SS 健壮性 / Clash SS plugin / hysteria2+vmess+vless 查漏），仅 WireGuard 暂缓
- **问题**：SS 链接 `plugin`（obfs / v2ray-plugin）未支持、带 plugin 的 SS 被静默降级；标准 SIP002 base64 userinfo 解析错误（Q1 测出）。
  - `src-tauri/src/subscription.rs`（各 `parse_*` 与 `clash_yaml_proxy_to_singbox`）
- **本次完成（批次 1：SS 链接健壮性）**：
  - 修复 base64 userinfo 解析：新增 `ss_method_password()`——从原始链接取 `@` 前 raw（绕过 `url` 的 WHATWG 百分号编码 `=`→`%3D` / `/`→`%2F`），按 url-safe / standard 两种字母表、padding 可选解码，以解码串含 `:` 作为"确为 base64"判据，否则回退 `url.username()/password()` 明文形式。
  - 新增 SS `plugin` / `plugin_opts` 支持：解析 query `plugin=<name>;<opts>`，`obfs`/`simple-obfs` 归一化为 sing-box `obfs-local`。
  - 测试 +4：标准带 padding base64、url-safe 无 padding base64、带 obfs plugin、obfs 别名归一化（`cargo test --lib` 15 passed）。
- **本次完成（批次 2：Clash YAML SS plugin/plugin-opts）**：
  - 新增 `clash_ss_plugin(proxy)` 纯函数：把 Clash 的**结构化** `plugin-opts`（map）翻译为 sing-box 的 `plugin` 名 + 分号分隔 `plugin_opts` 串，与 `ss://` 单链接路径行为一致。
  - 显式支持 `obfs`/`simple-obfs`/`obfs-local`（`mode`/`host` → `obfs=<mode>;obfs-host=<host>`）、`v2ray-plugin`（`mode`/`tls`/`host`/`path`/`mux`）、`shadow-tls`（`host`/`password`/`version`）；未知 plugin 走通用 `k=v;flag` 兜底序列化，**不丢节点**。
  - 接入 `clash_yaml_proxy_to_singbox` 的 `ss` 分支；之前带 plugin 的 Clash SS 节点会被静默降级为无混淆。
  - 测试 +3：obfs 映射、v2ray-plugin 映射、无 plugin 不带 plugin 字段（`cargo test --lib` 28 passed）。
- **不可行（已确认）**：**SSR** — sing-box 内核无 `shadowsocksr` 出站类型，解析后无法生成有效配置，故不支持。
- **WireGuard（暂缓，附理由）**：本项目目标内核为 **sing-box ≥ 1.12**（配置已用 1.12 新 DNS `servers[].type`、route `action: sniff/hijack-dns`、`default_domain_resolver`）。自 1.11 起 WireGuard 已**从 `outbound` 迁移为 `endpoint`**，需新增顶层 `endpoints` 数组、并改动 `build_proxy_outbounds`/选择器/`parse_subscription` 的数据模型（区分 outbound 与 endpoint，且 endpoint tag 仍要可被选择器引用）。该改动体量大、schema 版本敏感，且无法在当前环境对真实内核验证；为避免生成内核拒绝的配置，单列为独立任务，待确认内核版本后实现。
- **本次完成（批次 3：hysteria2 健壮性 + vmess/vless 字段查漏）**：
  - **hysteria2**（实测 liangxin 订阅驱动）：新增 `hysteria2_server_ports()` 把 `mport=60000-65530` / 多段 `443,8443-8500` 转为 sing-box 冒号格式 `["60000:65530"]`（已核对 sing-quic 统一为冒号、`server_port` 在 `server_ports` 存在时被忽略，保留基础端口不报错）；`pinSHA256`（sing-box 无指纹校验等价）→ `tls.insecure: true` 以保连通；salamander `obfs`+`obfs-password`（仅在 type=salamander 且有密码时生成，避免内核 `unknown obfs type` / `missing obfs password`）；`alpn`、`up`/`down` mbps（`parse_mbps` 容错单位）。
  - **vless**：新增 `packet_encoding`（xudp/packetaddr）与 `tls.alpn`。
  - **vmess**：新增 `scy` 加密方式（覆盖原写死的 `auto`）与 `tls.alpn`。
  - 测试 +7（hysteria2 端口跳跃/多段/pinSHA256/salamander/缺密码丢弃、vless packet_encoding+alpn、vmess scy）：`cargo test --lib` 37 passed。
- **影响文件**：`src-tauri/src/subscription.rs`。
- **剩余**：仅 WireGuard（暂缓，见上）。

### F3. IPv6 关闭、DNS 写死 ✅ 已完成（2026-06-25）
- **问题**：DNS `strategy: "ipv4_only"` + fakeip 仅 `inet4_range`，纯 IPv6 资源不可达；DNS 服务器写死 `223.5.5.5`，不支持 DoH/DoT 与自定义。
- **已实现**：
  - `types.rs`：`AppConfig` 新增 `enable_ipv6: bool`（默认 false）与 `dns_local: String`（默认 `223.5.5.5`），均 `serde(default)` 向后兼容。
  - `subscription.rs`：新增 `dns_local_server()`——按 scheme 生成 DNS 服务器：`https://…`→DoH、`tls://…`→DoT（自动拆 `host:port`）、其余→UDP。`build_singbox_config` 据 `enable_ipv6` 决定 `strategy`（off→`ipv4_only`、on→`prefer_ipv4`）、fakeip 是否加 `inet6_range`（`fc00::/18`）、TUN 是否分配 IPv6 地址。
  - 前端：`stores/app.ts` 接口与默认值；`Settings.vue` 新增「DNS 与网络」区——国内 DNS 解析器输入（IP/DoH/DoT）+ IPv6 开关。
  - 测试 +4：默认 ipv4_only、IPv6 开启转 prefer_ipv4 + inet6 fakeip、自定义 DoH、`dns_local_server` 各 scheme 解析（`cargo test --lib` 19 passed）。
- **默认行为不变**：IPv6 默认关、DNS 默认 `223.5.5.5`，与改动前完全一致（无回归）；变更于下次启动内核时生效。
- **影响文件**：`types.rs`、`subscription.rs`、`stores/app.ts`、`Settings.vue`。

---

## P3 — 优化 / 规范 / 安全

### Q3. CN 域名 / WeChat 名单去重 ✅
- 原现状散落多处：`subscription.rs`（DNS + route 共用一份）、`proxy.rs`（ProxyOverride）、`rules.rs`（preset）。
- **已实现**：新建单一来源模块 `src-tauri/src/cn_direct.rs`：
  - `CN_DIRECT_SUFFIXES`（取 subscription 现有全集为权威）、`WECHAT_PROCESSES`、`proxy_override_fragment()`（由常量派生 WinINet 旁路串，`#[cfg(windows)]`）。
  - `subscription.rs`：`cn_core_domains`、WeChat 进程名改为引用常量。
  - `proxy.rs`：`CN_DOMAINS` 字面量删除，改用 `cn_direct::proxy_override_fragment()`。
- **有意行为对齐**：proxy ProxyOverride 列表随之扩充约 15 个 CN 域名。规则模式下这些域名本就路由 direct，扩充只改变直连路径（WinHTTP 直连 vs 经 sing-box 直连），最终结果一致且消除两份列表漂移；非回归。
- **未纳入（有意保留）**：`rules.rs` 的 阿里系/腾讯系/百度系/字节 等预设是**用户可编辑的种子默认规则**（含分组 + geosite/keyword），语义不同于"安全网"硬编码，强行合并会破坏 UI 分组与种子数据，故不去重。
- **影响文件**：`cn_direct.rs`（新增）、`lib.rs`、`subscription.rs`、`proxy.rs`。

### Q4. 版本与命名统一 ✅
- 原现状：crate `sing-box-win` / `productName` `sing-box` / 仓库 `sing-box-desktop` 三名并存；版本 0.1.2 vs 0.2.7。
- **已全量改名为品牌 `Skylark`（云雀）**（确认无存量用户，改名零影响）：
  - `productName` / 窗口标题 / 托盘 tooltip / 关于页 / index.html title → `Skylark`。
  - `identifier` → `com.radium.skylark`。
  - crate `name` → `skylark`、lib → `skylark_lib`（同步 `main.rs::skylark_lib::run()`）。
  - `package.json` / `package-lock.json` name → `skylark`，版本对齐 `0.2.7`。
  - 数据目录 → `%LOCALAPPDATA%/Skylark`（旧 `sing-box-win` 目录废弃）。
  - TUN 适配器名 `sing-box-tun-*` → `skylark-tun-*`（`subscription.rs` 生成与 `tun.rs` 清理同步）。
  - HTTP `user-agent` → `skylark/<version>`；App 安装包回退名 → `skylark-setup.exe`。
  - 安装包 / DMG 图形文案、README(中/英)、release.yml `releaseName` → `Skylark`。
- **版本唯一来源** = `package.json`（`tauri.conf.json` 经 `../package.json` 读取；`Cargo.toml` 手工镜像 `0.2.7`）。
- **刻意保留（属内核而非本应用）**：内核二进制名 `sing-box.exe`、内核版本展示回退、`pkill -f "sing-box run -c"`、updater 内核源 `SagerNet/sing-box`。
- **远端仓库已改名** `radiumCN/skylark`：同步更新 `updater.rs` 自更新源常量（`APP_GITHUB_STABLE_API` / `APP_GITHUB_ALL_API`）、`auto_update.rs` 更新通知文案、`Settings.vue` releases 链接、README(中/英) clone URL 与项目结构目录名。
- **影响文件**：`Cargo.toml`、`main.rs`、`config.rs`、`lib.rs`、`updater.rs`、`tun.rs`、`subscription.rs`、`singbox.rs`、`tauri.conf.json`、`index.html`、`stores/app.ts`、`views/Settings.vue`、`package.json`、`package-lock.json`、`scripts/prepare-installer.js`、`README*.md`、`release.yml`。

### S1. Clash API 无鉴权 ✅
- 原现状 `secret: ""` 绑定 `127.0.0.1`，本机任意进程可控制内核。
- **已实现**：
  - `config::api_secret()`：用 `OnceLock` 缓存、持久化到数据目录 `api_secret` 文件，首次自动生成随机 UUID。独立于 `app_config.json`，避免前端保存设置时被覆盖（保存设置会整体回写 AppConfig，secret 若混在其中会被洗掉）。
  - `subscription.rs::build_singbox_config` 将 `clash_api.secret` 设为该随机值。
  - 所有 9 处 Clash API 调用统一加 `.bearer_auth(crate::config::api_secret())`：
    - `singbox.rs`：`fetch_connections` / `close_connection` / `close_all_connections`。
    - `commands.rs`：`clash_proxy_delay` / `clash_select_proxy` / `cmd_test_group_delay` / `cmd_get_active_proxy_now` / `clash_set_mode` / `cmd_get_traffic_total`。
  - 下载测速（`measure_download_speed`，走代理）与订阅拉取（`fetch_url`）非 Clash API，未加鉴权头。
- **风险/回滚**：secret 跨重启稳定（持久化文件）；内核每次启动用同一值重建配置，调用方读同一值，header 必然匹配。前端全部经 Tauri 命令间接访问，无需改前端。删除 `api_secret` 文件即可重置。
- **影响文件**：`config.rs`、`subscription.rs`、`commands.rs`、`singbox.rs`。

### F4. 其他小功能 ✅ 已完成（2026-06-25）
- ✅ **配置导入 / 导出（备份）**：新增 `cmd_export_config` / `cmd_import_config`。
  - 导出：把 `app_config` / 订阅（含缓存原文）/ 节点 / outbounds / 分组 / 路由规则打包为单个 JSON，写入 `app_data_dir/backups/skylark-config-<时间戳>.json`，前端经 opener `revealItemInDir` 在文件管理器中定位（复用 `cmd_export_logs` 范式）；**不导出 API 密钥**（机器本地、按需重建）。
  - 导入：粘贴备份 JSON（沿用「从文本导入订阅」交互），校验 `format == "skylark-config"`；逐段宽容应用（单段损坏跳过而非整体失败），**同时写盘并更新内存 `AppState`**，前端随后 `fetchConfig/Subscriptions/Nodes/ProxyGroups` 刷新，路由 / DNS 在下次启动代理生效。
  - 前端：`Settings.vue` 新增「配置备份」区块（导出按钮 + 导入弹窗 textarea）。
- ✅ **节点搜索 / 过滤、一键批量测速**：经核对 `Nodes.vue` 已内置搜索框（按名称/服务器过滤）与「全部测速」按钮，无需新增。
- ✅ **TUN `mtu: 9000`**：核对为 sing-box TUN 默认值（非偏大笔误），有意保留，无需变更。
- ✅ **日志可选落盘 + 增量推送**：
  - 落盘：`AppConfig` 新增 `log_to_file: bool`（默认关，`serde(default)` 兼容）。开启后 `singbox.rs` 的 stderr 采集任务在启动时打开 `logs/skylark-<日期>.log`（append），逐行写入——崩溃也不丢。运行时切换于下次启动内核生效。
  - 增量推送：采集点每行通过 `app_handle.emit("singbox-log", line)` 推送给前端；`Logs.vue` 改为「挂载时拉取一次快照 + 监听事件追加」（前端保留 1000 行上限），**移除每秒 `cmd_get_logs` 整表克隆轮询**。
  - 前端：`Settings.vue`「系统行为」新增「日志写入文件」开关；`stores/app.ts` 接口与默认值同步。
- **影响文件**：`types.rs`、`singbox.rs`、`commands.rs`（既有 `cmd_export_logs`）、`src/views/Logs.vue`、`src/views/Settings.vue`、`src/stores/app.ts`。
- **验证**：`cargo check` 无警告、`cargo test --lib` 28 passed、`vue-tsc` 通过。

### F5. 订阅获取健壮性（用户实测发现）✅ 已完成（2026-06-25）
- **问题 1（显示「不支持」占位节点）**：拉订阅的 User-Agent 写死为 `ClashForWindows/0.20.39`（老版 Clash，不支持 vless-reality / hysteria2）。V2board 类机场按 UA 返回「请更换客户端」占位配置（一堆 `ss 127.0.0.1:65535`）而非真实节点。
  - **根因**：机场按客户端 UA 分发不同订阅模板；旧 Clash 标识被判为不支持现代协议。我们的内核是 sing-box，全协议支持。
  - **修复**：默认 UA 改为 `v2rayN/6.45`（机场白名单内、返回通用 base64 节点列表），并**做成可配置项**——`AppConfig.subscription_user_agent`（`serde(default)`，空值回退默认）；`config::subscription_user_agent()` 实时读取，`fetch_url` 与 `auto_update` 共用；`Settings.vue` 新增「订阅」区输入框 + 常用预设。
- **问题 2（节点名乱码）**：`Url::fragment()` 返回**未解码**的百分号编码串，6 个 `parse_*`（vless/ss/trojan/hysteria2/tuic/anytls）直接使用，导致节点名显示为 `%E5%89%A9...`。
  - **修复**：新增无依赖 `percent_decode()` + `node_name_from_fragment()`，统一对 fragment 名 percent-decode（UTF-8，残缺转义原样保留）；6 处统一替换。
- **测试 +2**：percent-decode UTF-8/容错、vless fragment 名解码（`cargo test --lib` 30 passed）。
- **影响文件**：`types.rs`、`config.rs`、`commands.rs`、`auto_update.rs`、`subscription.rs`、`src/stores/app.ts`、`src/views/Settings.vue`。
- **次要遗留**：~~hysteria2 链接的端口跳跃 `mport` 与 `pinSHA256` 证书指纹暂未映射~~ → 已在 F2 批次 3 完成（mport→`server_ports`、pinSHA256→`insecure`、salamander obfs）。

### F6. 托盘连接控制三处缺陷 ✅ 已完成（2026-06-25）
- **问题 1（TUN 从托盘静默失败）**：托盘点「TUN 模式」时 `let _ = apply_connection_mode(...)` 吞掉错误，非管理员/缺 WinTun 时勾选框闪一下复位、无任何提示（仪表盘路径有 `error` 提示，托盘没有）。
- **问题 2（托盘切换不回流前端）**：托盘只改了注册表/内核与托盘勾选，未通知前端；而 Home 轮询每秒只刷新系统代理、**不刷新 config**，导致从托盘切 TUN 后仪表盘 `tun_enabled` 长期 stale（`tunOn = running && config.tun_enabled` 显示错误）。
- **问题 3（启动初值口径不一致）**：托盘初值用持久化的 `app_config.tun_enabled`，真实判定却是 `singbox_state.running && tun_mode`，且启动时未同步一次 → 权限丢失等场景勾选框短暂错误。
- **修复（后端 `lib.rs`）**：
  - 新增 `emit_tray_mode_result()`：托盘 apply 成功→ emit `connection-mode-changed`、失败→ emit `connection-mode-error`（携带原因）；两个托盘 handler 改为捕获 `Result` 后调用。
  - 新增 `sync_tray_from_state()`：从 `TrayState` 取菜单项句柄并按真实运行态回写；在启动恢复 task 末尾调用一次（修问题 3）。
- **修复（前端 `stores/app.ts`）**：`init()` 注册 `listenTrayConnectionEvents()`（幂等）——`connection-mode-changed` → `fetchConfig + fetchStatus + refreshSystemProxy + updateTrayTooltip`；`connection-mode-error` → 写入 `error`。仪表盘路径不 emit，无重复刷新。
- **影响文件**：`src-tauri/src/lib.rs`、`src/stores/app.ts`。
- **验证**：`cargo check` 无警告、`vue-tsc` 通过。

### F7. 更新重启后 TUN「已连接但无网络」✅ 已完成（2026-06-25）
- **复现**：TUN 模式下载更新 → 不关 TUN 直接安装 → 重启后自动恢复 TUN，但无真实速率、代理不生效（UI 显示已连接）。
- **根因**：TUN inbound 用 `auto_route + strict_route`。`stop_singbox` 的 graceful 路径用 `taskkill /PID`（不带 `/F`）发 `WM_CLOSE`，但 sing-box 是 `CREATE_NO_WINDOW` 的无窗口控制台进程收不到 → 超时后强杀 `/F`，**跳过 WintunDeleteAdapter + strict_route 路由清理**，残留 TUN 网卡与强制路由。更新时安装器秒级重启 App，新 App 的 `cleanup_stale_tun_adapter`→建新 TUN 在极短窗口内连续发生，OS 路由表来不及收敛，新 `strict_route` 叠加在残留路由上 → 流量黑洞。`wait_until_ready` 只探测 Clash API 端口、不校验 TUN 路由，故"启动成功"假象。
- **修复**：
  - `updater.rs`：更新拆除阶段在 `shutdown_core` 后、安装器重启前，于**仍提权的旧进程**内显式 `cleanup_stale_tun_adapter()`（仅当先前为 TUN）；安装耗时即成为 OS 路由收敛缓冲，破除竞态。
  - `lib.rs`：启动恢复 TUN 前主动清理残留网卡 + 800ms 收敛等待（冷路径，每次启动仅一次，不影响运行时切换延迟），使「从无此修复的旧版本升级」与崩溃重启场景也能自愈。
- **影响文件**：`src-tauri/src/updater.rs`、`src-tauri/src/lib.rs`。
- **验证**：`cargo check` 无警告。

#### F7+. 真正优雅退出 sing-box（从源头消除残留）✅ 已完成（2026-06-25；**机制于后续重写，本节已按现行实现更新 2026-07-10**）
- **背景**：F7 的根因之一是 graceful 停止对无窗口控制台进程无效——`taskkill /PID`（无 `/F`）发 `WM_CLOSE`，sing-box（`CREATE_NO_WINDOW` 的无窗口控制台进程）收不到，超时后强杀，跳过自身 TUN/路由清理。
- **现行实现**（`singbox.rs::send_graceful_break`）：投递**定向 CTRL+BREAK**（→ Go 同样按 SIGINT 语义处理），触发 sing-box 自身的 `WintunDeleteAdapter` + `strict_route` 清理。
  - 核心以 `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW` 启动——**新进程组 id == 核心 pid**，于是 `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid)` 只到达核心进程组，**不会波及 GUI**。
  - 必须用 CTRL+**BREAK** 而非 CTRL+C：`CREATE_NEW_PROCESS_GROUP` 启动的进程默认禁用 Ctrl+C，但 CTRL_BREAK 不受影响。
  - `AttachConsole(pid) → SetConsoleCtrlHandler(handler, TRUE) → GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) → FreeConsole` + 恢复 handler；GUI 自身 handler 吞掉 CTRL_C/CTRL_BREAK 防误杀。
  - `stop_singbox` 的 graceful 分支轮询最多 ~3s 等待核心自行退出，仍未退出再强杀兜底。
  - > 历史注：初版实现用「无 `CREATE_NEW_PROCESS_GROUP` + 广播 CTRL_C_EVENT(0)」，该广播语义在更新器工作线程上会连带终止 GUI 自身（updater.rs 有事故记录），故重写为「专属进程组 + 定向 CTRL_BREAK」。
- **效果**：TUN 停止/重启/更新拆除时核心都能自清理，网卡/路由不再常驻；F7 的清理与收敛等待降级为兜底防御。
- **影响文件**：`src-tauri/src/singbox.rs`、`src-tauri/Cargo.toml`。
- **验证**：`cargo check` 无警告、`cargo test --lib` 37 passed。
- **跨平台**：macOS/Linux 原本即用 `SIGTERM` 优雅停止，行为一致。

---

## 建议执行顺序

1. **第一批（P0）**：B1 → B3 → B2。聚焦、用户感知最强、互不耦合。
2. **第二批（P1）**：先 Q1「补测试锁定现状」，再做 B4 / Q2 重构与行为修正。
3. **第三批（P2/P3）**：按需排期 F1（流量信息）最具性价比，其余迭代推进。

## 进度跟踪

- [x] B1 自更新版本比较
- [x] B2 pkill 误杀
- [x] B3 单节点测速
- [x] B4 死资源处理
- [x] Q1 巨型函数已拆为 5 个纯函数（build_proxy_outbounds/dns_rules/route_rules/inbounds/tun_inbound），25 测试全绿
- [x] Q2 Global 旁路
- [x] F1 订阅流量信息
- [~] F2 SS 链接健壮性（base64 + plugin）+ Clash YAML SS plugin/plugin-opts 映射完成（测试 28 全绿）；WireGuard 暂缓（需 1.12 endpoint 模型，附理由）；SSR 不可行；vmess/vless 字段查漏待办
- [x] F3 IPv6 / DNS 可配（后端 + 前端 + 测试 19 全绿）
- [x] Q3 去重
- [x] Q4 版本命名统一（版本来源 + 命名约定已确立）
- [x] S1 Clash API secret
- [x] F4 小功能集（配置导入/导出；日志可选落盘 + 事件增量推送；节点搜索/批量测速本已存在；TUN mtu 确认）

---

## 已知依赖债（2026-07-10 登记，暂不迁移）

- **`serde_yaml 0.9`**：官方已归档停维（RUSTSEC-2024-0320），且用在解析**不可信订阅输入**的路径（`subscription.rs`）。候选：`yaml-rust2` 或手写受限解析；`serde_yml` 分叉质量存疑不推荐。迁移涉及全部 Clash YAML 解析路径，需专门批次 + 全量订阅样本回归，故登记不动。
- **`winapi 0.3`**：不再维护，生态已迁 `windows-sys`。本项目用到 12 个 features、散布 tun/singbox/proxy 多处，迁移量大、收益低（API 冻结），低优先级。
- **`tokio` features "full"**：可裁剪至实际用到的 feature 集（仅编译期成本，运行时无差别）。
- 已完成的小项：`dirs-next` → `dirs`（2026-07-10）；npm 侧删除零引用的 `js-yaml`/`@vueuse/core` 及 5 个未用的 `@tauri-apps/plugin-*` JS wrapper（Rust 侧插件保留），`@types/qrcode` 移入 devDependencies。
