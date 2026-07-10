# Skylark（云雀）功能演进计划（Roadmap）

> 生成于 2026-06-25。基于对当前代码库的完整审阅。
> 定位：`IMPROVEMENT_PLAN.md` 是已基本收尾的「Bug / 工程债」清单；本文件是**面向未来**的演进规划——分「现有功能完善」与「新功能开发」两大块，每项含「价值 / 现状 / 方案 / 影响文件 / 工作量 / 风险」。
> 工作量标度：S（<0.5d）/ M（0.5–2d）/ L（2–5d）/ XL（>5d）。

---

## 一、现状盘点（决定优先级的事实）

通过通读代码确认的当前能力与缺口（**2026-07-10 更新**——本节初版（2026-06-25）中的多项缺口已在后续批次落地，当时的表述见 git history）：

- **平台**：CI 产出 Windows(.exe) / macOS(Intel+ARM, .dmg) / Linux(.deb)。Linux 系统代理已非 stub——`proxy.rs` 走 GNOME `gsettings`（C2 部分完成）；**剩余**：KDE（kwriteconfig）、`http_proxy` env 回退、Linux TUN 提权路径。
- **i18n**：`vue-i18n@11` 已全量落地（`src/i18n/`，zh-CN + en 键集完全镜像，Settings 有语言切换入口，store 层错误文案与托盘 tooltip 也已接入）。**剩余**：托盘菜单（`lib.rs` Rust 侧）与后端用户可见错误串仍是硬编码中文——C1 方案第 4 步（托盘 match 映射）尚未做。
- **协议**：vmess/vless/ss/trojan/hysteria2/tuic/anytls 及 Clash YAML 映射齐全；WireGuard 已按 1.12 endpoint 模型实现（N3，待真机校验）；SSR 内核不支持。
- **已有但可深化**：订阅 QR / 剪贴板导入、进程名分流、规则编辑器 + rule-provider、规则拖拽排序（C6）、连接视图过滤/排序/按主机聚合（C5）、多 Profile（N6）、全局快捷键（N4）、诊断面板（N5，**DNS 泄漏检测未做**）均已存在。
- **数据生命周期**：流量历史已按日聚合持久化 + 统计页（N1）；**节点延迟历史仍无**（N7 剩余项）。
- **更新**：应用自更新与内核下载均有 **SHA-256 fail-closed 校验**（C3 部分/C4）；**仍未接 Tauri 签名 updater（minisign）**，这是剩余的主要安全缺口。
- **待接通的半成品**：`src-tauri/ui/` 已提交完整 yacd 构建、生成的配置也写了 `external_ui`，但打包 resources 不含它、启动也不复制——要么接通（补打包 + 应用内入口，注意带 `?hostname&port&secret` 参数），要么移除以免误导。
- **测试**：后端 `cargo test --lib` 48 tests，集中在解析/配置生成/名称清洗；前端无单测、无 E2E。

---

## 二、现有功能完善（improve existing）

### C1. 国际化（i18n）真正落地 —— 优先级 P1，工作量 L
- **价值**：项目已对外提供英文 README、`language` 配置位已留，却无实现，是「最违和」的缺口；补齐直接扩大可用人群，也是后续上架/推广前提。
- **现状**：无 `vue-i18n`，文案散落各 `.vue`；`language` 字段写了但无人消费。
- **方案**：
  1. 引入 `vue-i18n@9`，建 `src/locales/{zh-CN,en}.ts`，按视图拆 namespace。
  2. 抽取硬编码中文 → key（可借脚本半自动：扫描 `>中文<` 与字符串字面量）。
  3. Settings 增「语言」下拉，绑定 `AppConfig.language`，切换即时生效并持久化；启动时按配置初始化。
  4. 托盘菜单文案（Rust 侧 `lib.rs`）同步做一层简单映射（托盘字符串少，可用 match）。
- **影响文件**：`package.json`、新增 `src/i18n/`、全部 `src/views/*`、`Sidebar.vue`、`stores/app.ts`、`src-tauri/src/lib.rs`（托盘）。
- **风险**：抽取工作量大但机械；分批迁移（先框架 + Settings/Home，再逐页）可降风险。

### C2. Linux 一等公民化（系统代理 + TUN）—— 优先级 P2，工作量 L
- **价值**：已经在发 `.deb`，但核心功能在 Linux 上是空转，等于「假支持」。要么补齐，要么明确下线以免误导。
- **现状**：`proxy.rs` Linux 分支 no-op；TUN 的 WinTun 路径是 Windows 专属，Linux 走 sing-box 原生 tun 但提权/路由清理未适配。
- **方案**：
  1. 系统代理：GNOME 走 `gsettings set org.gnome.system.proxy`，KDE 走 `kwriteconfig`，并写 `~/.config` 兜底 + `http_proxy` 环境变量提示；检测桌面环境。
  2. TUN：Linux 下用 `pkexec`/`setcap cap_net_admin` 方案替代 UAC；复用 sing-box 原生 tun，去掉 WinTun 依赖分支。
  3. 若短期不投入，则从 CI matrix 移除 ubuntu 产物 + README 注明，避免「装了不能用」。
- **影响文件**：`proxy.rs`、`tun.rs`、`singbox.rs`、`lib.rs`、`.github/workflows/*`。
- **风险**：桌面环境碎片化、提权模型差异大；建议先做「系统代理」（性价比高、风险低），TUN 二期。

### C3. 应用自更新接入 Tauri 签名 updater —— 优先级 P1，工作量 M
- **价值**：当前自更新直接下二进制并重启安装器，**无更新包签名验证**，是明确的安全弱点（投毒/劫持即可静默替换可执行文件）；也缺增量与回滚。
- **现状**：`updater.rs` 自实现下载 + `prepare-installer.js` 重启；走 GitHub API 比对版本。
- **方案**：接入 `@tauri-apps/plugin-updater` + `tauri-plugin-updater`，用 minisign 密钥对发布物签名（CI 注入私钥），客户端内置公钥校验；保留 GitHub releases 作为 endpoint。内核(sing-box)更新仍走现有 SagerNet 源（可加 SHA256 校验，见 C4）。
- **影响文件**：`Cargo.toml`、`tauri.conf.json`（updater 配置 + pubkey）、`updater.rs`、`auto_update.rs`、`release.yml`、`Settings.vue`。
- **风险**：需在 CI 管理签名私钥（secret）；迁移期需同时支持旧路径一个版本。

### C4. 内核下载完整性校验 —— 优先级 P2，工作量 S
- **价值**：`updater.rs` 下载 sing-box 内核未校验哈希，破坏面同 C3。
- **方案**：从 release 资产一并取官方 `*.sha256` / checksums，下载后比对再落盘；失败则丢弃不替换。
- **影响文件**：`updater.rs`。
- **风险**：低；需确认 SagerNet 资产命名规律。

### C5. 连接视图增强（过滤 / 排序 / 聚合）—— 优先级 P3，工作量 M
- **价值**：`Connections.vue` 已有实时列表 + 单条/全部关闭，但连接多时不可用——缺按域名/规则/出站过滤、按流量排序、按进程聚合。
- **方案**：前端加搜索框 + 列排序 + 「按 host/进程分组」折叠视图；复用现有 Clash API 数据，无需后端改动。
- **影响文件**：`Connections.vue`、`stores/app.ts`。
- **风险**：低，纯前端。

### C6. 规则编辑器体验打磨 —— 优先级 P3，工作量 M
- **价值**：`Rules.vue` 已支持 geosite/geoip/ip/port/process/rule-provider，但缺「规则命中实时高亮（哪条规则正在生效）」「拖拽排序」「规则连通性/语法校验」「从某条连接一键生成规则」。
- **方案**：从 Connections 的 `rule`/`chains` 反查并在 Rules 高亮；引入拖拽排序（规则顺序即优先级）；「右键连接 → 加为直连/代理规则」。
- **影响文件**：`Rules.vue`、`Connections.vue`、`rules.rs`（顺序持久化）。
- **风险**：中；规则顺序语义需与 `build_route_rules` 一致。

---

## 三、新功能开发（new features）

### N1. 流量 / 延迟历史持久化与可视化 —— 优先级 P1，工作量 L
- **价值**：当前所有曲线和累计流量重启即清零，无法看「今天/本周用了多少」「某节点长期稳定性」——是同类成熟客户端（Clash Verge 等）的核心差异点。
- **方案**：后端按分钟/小时聚合写入轻量存储（`AppData/stats.json` 滚动，或引入 `rusqlite`）；记录每日上下行、各订阅用量、节点延迟历史。前端新增「统计」页：日/周流量柱状、节点延迟趋势、按订阅用量占比。
- **影响文件**：新增 `src-tauri/src/stats.rs`、`commands.rs`、`Cargo.toml`（可选 rusqlite）、新增 `src/views/Stats.vue`、路由、`Sidebar.vue`。
- **风险**：存储增长需设上限/滚动；选 JSON 可零依赖起步，量大再升 sqlite。

### N2. 节点订阅过滤 / 重命名规则（include/exclude + 正则）—— 优先级 P2，工作量 M
- **价值**：机场订阅常含大量无用节点（官网/到期提醒/倍率节点）。支持按关键字/正则保留或剔除、按地区分组、emoji 国旗归一化，是高频刚需。
- **方案**：`Subscription` 增 `include`/`exclude`（正则）与可选「地区分组」开关；解析后过滤 + 按地区（解析名中 🇭🇰/HK/香港）自动分组；可选剥离倍率标记。Settings/订阅编辑暴露配置。
- **影响文件**：`types.rs`、`subscription.rs`（解析后过滤管线）、`commands.rs`、`Subscriptions.vue`。
- **风险**：正则错误需容错（编译失败回退不过滤）；分组与现有「按订阅分组」选优需协调。

### N3. WireGuard 出站支持（落地 IMPROVEMENT_PLAN 暂缓项）—— 优先级 P2，工作量 L
- **价值**：补齐唯一缺失主流协议；WARP / 自建 WG 用户刚需。
- **方案**：按 sing-box ≥1.11 模型，新增顶层 `endpoints` 数组（WG 从 outbound 迁为 endpoint），数据模型区分 outbound/endpoint，选择器/分组仍能引用 endpoint tag；解析 `wireguard://`/Clash `wireguard` 类型。
- **影响文件**：`subscription.rs`（`build_proxy_outbounds`、`parse_*`、配置组装）、`types.rs`。
- **风险**：schema 版本敏感、需对真实内核验证；建议先固定支持的内核版本下限再实施。

### N4. 全局快捷键（global shortcut）—— 优先级 P3，工作量 S
- **价值**：快速切换「系统代理 / TUN / 模式」无需开窗，桌面客户端常见便利。
- **方案**：接 `tauri-plugin-global-shortcut`，Settings 可配置组合键，绑定到现有 `apply_connection_mode` / 模式切换命令。
- **影响文件**：`Cargo.toml`、`lib.rs`、`commands.rs`、`Settings.vue`、`stores/app.ts`。
- **风险**：跨平台默认键冲突；允许用户自定义并做冲突提示。

### N5. 连通性 / 节点诊断面板 —— 优先级 P3，工作量 M
- **价值**：用户排障刚需——「我的出口 IP / 地区是什么」「DNS 是否泄漏」「能否直连 Google/落地」一键自测。
- **方案**：新增诊断命令：经代理请求 ip-api/cloudflare trace 取出口 IP+地区；并发探测若干目标站可达性；DNS 泄漏检测（对比直连/代理解析结果）。结果以面板展示。
- **影响文件**：新增 `commands.rs` 诊断命令、新增 `src/views/Diagnostics.vue` 或并入 Home。
- **风险**：依赖第三方探测端点（需可配/容错）；隐私上仅本机发起、不上报。

### N6. 配置 Profile / 多套快速切换 —— 优先级 P3，工作量 M
- **价值**：现有「配置导入/导出」是一次性备份；缺「多 Profile（家庭/公司/不同机场组合）一键切换」。
- **方案**：在备份基础上扩展为命名 Profile 列表（订阅集 + 规则 + 模式快照），一键切换并热重载内核。
- **影响文件**：`config.rs`、`commands.rs`、`types.rs`、新增/扩展 `Settings.vue` 或独立页。
- **风险**：切换需稳妥停/起内核，复用现有优雅退出（F7+）路径。

### N7. 节点延迟可视化与「健康度」标记 —— 优先级 P3，工作量 S
- **价值**：`Nodes.vue` 已有测延迟/测速/排序；可叠加颜色分级（绿/黄/红）、丢包/超时标记、「最近一次测试时间」，提升一眼可读性。
- **方案**：前端按延迟阈值着色 + 失败态徽标；与 N1 历史打通后可显示稳定性。
- **影响文件**：`Nodes.vue`（+ N1 落地后 `stores`）。
- **风险**：低，纯前端增强。

### N8. 安全：本地敏感数据加密 / 单实例锁 —— 优先级 P3，工作量 M
- **价值**：订阅 URL、缓存原文以明文存于数据目录；多开会争抢内核/端口。
- **方案**：①敏感字段用 OS keyring（`keyring` crate）或本机密钥加密落盘；②接 `tauri-plugin-single-instance` 防多开，二次启动唤起既有窗口。
- **影响文件**：`config.rs`、`Cargo.toml`、`lib.rs`。
- **风险**：加密需兼容旧明文数据（迁移读取）；keyring 在 Linux 依赖 secret service。

---

## 四、建议执行顺序

1. **第一梯队（直接提升产品完成度与安全底线）**：C1 i18n、C3 签名 updater、N1 流量历史。
2. **第二梯队（高频刚需 + 平台补齐）**：N2 订阅过滤、N3 WireGuard、C2 Linux 系统代理、C4 内核校验。
3. **第三梯队（体验与便利）**：C5 连接增强、C6 规则打磨、N5 诊断、N4 快捷键、N6 Profile、N7 健康度、N8 安全加固。

## 五、推进进度

> 验证环境说明：本机为 AlmaLinux 8（glib 2.56 < Tauri 所需 2.70），无法整体编译 Tauri crate，故 Rust **纯逻辑**（解析/哈希/过滤/聚合）以独立 harness crate（不含 tauri 依赖）验证，前端以 `vue-tsc --noEmit` 验证，集成胶水代码（命令签名 / 托盘）人工审阅。

- [x] **C4 内核下载哈希校验**（2026-06-25）：用 GitHub 资产自带 `digest`（sha256）字段，下载后比对再落盘，失败丢弃。新增 `sha2` 依赖、`sha256_hex` / `normalize_sha256_digest` 纯函数 + 4 单测；`ReleaseInfo.sha256`（`serde(default)` 向后兼容）；`cmd_download_singbox` 增 `sha256` 参数；`Settings.vue` 透传。harness 校验通过、vue-tsc 通过。
- [x] **N2 订阅节点过滤 / 地区分组**（2026-06-25）：`Subscription` 增 `include`/`exclude`（正则，大小写不敏感，非法正则不丢节点）/`group_by_region`（`serde(default)`）；`subscription.rs` 新增 `detect_region` + `apply_node_filters` 纯函数 + 4 单测（含 outbound 锁步过滤、token 边界、非法正则保全）；接入 add/import/update/auto_update 四条解析路径；新增 `cmd_set_subscription_filters`（离线对缓存内容重过滤）+ lib.rs 注册；前端 store action、Subscriptions 添加对话框过滤区、每订阅过滤按钮 + 弹窗。harness 校验通过、vue-tsc 通过。
- [x] **N1 流量历史持久化与统计页**（2026-06-25）：新增后端 `stats.rs`——按日 bucket 聚合上下行，`StatsData::{add_sample,recent,prune}` 纯函数（saturating 防溢出、保留 180 天）+ 4 单测；`OnceLock<Mutex>` 进程级缓存，落盘 `traffic_stats.json`。新增 `cmd_add_traffic_sample` / `cmd_get_traffic_history` + lib.rs 注册。前端 store 累积每秒 delta、~30s 批量 flush（含停止时 flush、失败回滚），`fetchTrafficHistory` action；新增 `Stats.vue`（汇总卡 + chart.js 堆叠柱状，7/30/90 天切换）+ 路由 + 侧栏「流量统计」入口。harness 校验通过、vue-tsc 通过。延迟历史归入后续 N7。
- [x] **C1 国际化 i18n 全量落地**（2026-06-25）：装 `vue-i18n@11`，建 `src/i18n/`（zh-CN + en，`createI18n` legacy:false）；main.ts 注入；`setLocale` 持久化 localStorage；App.vue `watch(config.language)` 响应式切换；Settings「语言」选择器（写入 `AppConfig.language`，即时切换）。**全部 8 个视图 + Sidebar 已完成中文抽取**（Home/Subscriptions/Nodes/Connections/Logs/Rules/Stats/Settings，按命名空间组织 ~330 个 key，zh/en 双语镜像，含命名插值）。`vue-tsc` + `npm run build` 通过；视图模板已无硬编码中文（仅余 Settings 一处后端错误串匹配的 logic，非显示文案）。**2026-07-10 补充**：`formatDate` 日期本地化、store 层错误文案与托盘 tooltip 已接入 i18n；**仍未做**：托盘菜单（`lib.rs` Rust 侧 match 映射，即原方案第 4 步）与后端命令返回的中文错误串——严格说 C1 是「前端完成、Rust 侧待补」。

- [x] **C5 连接视图增强**（2026-06-25）：`Connections.vue` 在既有搜索基础上新增——可点击表头排序（主机 / 上传 / 下载 / 协议，升降序切换，箭头指示）、当前筛选连接的上下行**实时总计**、「列表 / 按主机聚合」视图切换（聚合视图按 host 汇总连接数与流量，支持一键关闭该主机全部连接）、无匹配结果提示。纯前端，复用 Clash API 数据；新增 6 个 i18n key（zh/en）。vue-tsc + build 通过。

- [~] **C3 应用自更新安全（安装包完整性校验已落地；完整签名 updater 待续）**（2026-06-25）：在不做高风险 updater 迁移的前提下，先补齐 C3 的核心安全价值——`download_and_install_app` 在**启动安装器前**用 GitHub 资产 `digest` 校验安装包 SHA-256，不一致则删除并中止（复用 C4 的 `sha256_hex`/`normalize_sha256_digest`）。`AppReleaseInfo` 加 `sha256`（serde default）、`find_app_installer_asset` 改返回 asset 以读 digest、`cmd_download_app_update` + 前端透传。vue-tsc 通过（vite build 因环境权限超时未复跑，前端改动与已 build 通过的 C4 同构）。**剩余**：迁移到 `tauri-plugin-updater` + minisign 签名（需 CI 注入私钥 + 真实 release 验证，单列）。

- [~] **N3 WireGuard 出站（endpoint 模型，待真机校验）**（2026-06-25）：按 sing-box ≥1.12 把 WireGuard 建模为顶层 `endpoints[]`。新增 `parse_wireguard`（`wireguard://`/`wg://` 链接：私钥取 `@` 前 raw + percent-decode 保留 base64、address 自动补 /32 或 /128、reserved 数组、psk/mtu）、Clash `wireguard` 类型映射（`clash_wireguard_endpoint` + `wg_with_prefix`/`wg_reserved_from_clash` 支持数组或 base64 reserved）；`build_singbox_config` 用 `partition` 把 `type=="wireguard"` 对象分流到 `endpoints[]`，selector/urltest 仍引用其 tag；`detect_sub_type` 识别 wireguard 链接。新增 3 个测试（链接解析 / Clash 映射 / 配置分流），harness 校验通过。**待验证**：受限于本机无法跑真实 sing-box，endpoint schema 仅按官方 1.12 文档构造，建议接入真机或 `sing-box check` 做最终确认。

- [~] **C2 Linux 系统代理（GNOME gsettings；KDE 待续）**（2026-06-25）：`proxy.rs` 的 Linux 分支由 no-op 改为通过 `gsettings` 操作 `org.gnome.system.proxy`——`set_system_proxy` 设置 http/https/socks 为 127.0.0.1:port、`ignore-hosts` 恒定旁路 loopback + RFC1918、mode 置 manual/none；`get_system_proxy_status` 读 mode 判断。best-effort（无 gsettings 时静默）。无新依赖、仅 `std::process`。**待验证/剩余**：需真实 GNOME 桌面确认；KDE（kwriteconfig）与 per-app `http_proxy` env 回退为后续。`.deb` 至此具备基本系统代理能力（TUN 提权仍为 Windows/macOS 路径）。

- [~] **C6 规则编辑器：拖拽排序（命中高亮 / 从连接生成规则待续）**（2026-06-25）：`Rules.vue` 规则行支持 HTML5 拖拽重排（顺序=优先级，拖拽手柄 + dragging/drag-over 视觉反馈），与既有上移/下移一致——仅改本地列表，点「保存并应用」下发内核。新增 `GripVertical` 图标 + `rules.dragHint` i18n（zh/en）。vue-tsc 通过。**剩余（C6 子项）**：规则命中实时高亮（需从连接反查）、从某条连接一键生成规则（跨视图）为后续。

- [x] **N7 节点健康度可视化**（2026-06-25）：`Nodes.vue` 节点名前新增延迟健康度色点（复用既有 `latencyColor`：绿<100ms / 橙<300ms / 红 / 灰=未测，title 显示延迟值），多节点一眼可扫。延迟数值着色本已存在；色点为增量。`npm run build` 整体通过（i18n + C5 + C6 + N7 一并验证）。**剩余（N7 子项）**：失败/超时与未测的区分、最近测试时间——需与 N1 历史/数据模型打通，后续。

- [x] **N5 连通性 / 出口诊断面板**（2026-06-25）：后端 `cmd_run_diagnostics`——经本地 mixed 端口代理请求 ip-api 取出口 IP/地区/ISP，并探测 Google/YouTube/GitHub/Cloudflare 可达性与延迟（需代理运行，否则报错）。纯解析 `parse_ipapi` + harness 校验 + 在文件 `#[cfg(test)]`。前端 Settings 新增「网络诊断」区（运行按钮 + IP/地区/ISP + 探测色点列表），i18n zh/en。vue-tsc 通过；网络请求路径需真机/运行代理验证。

- [x] **N4 全局快捷键**（2026-06-25）：接 `tauri-plugin-global-shortcut`。设计上**全部注册/响应在前端 JS API**完成（vue-tsc 可验证），Rust 侧仅 `.plugin(Builder::new().build())` 一行 + capabilities 加 4 项 `global-shortcut:*` 权限。默认绑定 Ctrl/Cmd+Shift+P（系统代理）/ +Shift+T（TUN）/ +Shift+R（循环 rule→global→direct），调用既有 `setConnectionMode`/`setProxyMode`。`AppConfig.enable_global_shortcuts`（默认关，`serde(default)`）+ Settings 开关（切换即时 register/unregisterAll）。vue-tsc + build 通过；插件 Rust 注册需 `cargo build` 确认。

- [x] **N6 多 Profile 快速切换**（2026-06-25）：把 export/import 抽为复用的 `build_config_bundle`/`apply_config_bundle`，在其上新增 `profiles/` 目录与 4 命令（`cmd_list/save/load/delete_profile`），含 `sanitize_profile_name`（拒空/超长/路径分隔符/`..`，防穿越）+ harness 校验 + 在文件测试。前端 store 4 actions + Settings「配置 Profile」区（命名保存当前 / 列表切换 / 删除，切换后刷新内存状态，下次启动代理生效）。i18n zh/en。vue-tsc + build 通过；后端命令注册需 cargo build 确认。

- [~] **N8 安全加固（单实例锁已落地；敏感数据加密待续）**（2026-06-25）：接 `tauri-plugin-single-instance` 并作为**首个**注册的插件（Tauri 要求）——二次启动不再启动抢占端口/TUN 的第二个进程，而是唤起既有 `main` 窗口（show + unminimize + set_focus）。纯 Rust init、无需 capabilities/前端改动。**剩余**：订阅 URL / 缓存原文的 keyring 加密落盘——需 plaintext→密文迁移 + Linux secret-service 运行时依赖 + 密钥管理，风险较高，单列待编译/运行环境实现。cargo build 需确认插件注册。

## 六、跨项工程项（贯穿始终）

> 整份 ROADMAP 的「现有功能完善（C1–C6）+ 新功能（N1–N8）」14 项均已推进完毕（含若干显式标注的延后子项：签名 updater、KDE 代理、规则命中高亮/从连接生成规则、keyring 加密）。
- **前端测试基线**：当前前端零测试。引入 Vitest 对 `stores/app.ts` 关键逻辑、Playwright（项目已有 webapp-testing skill）做 1~2 条核心 E2E（开关代理 / 切模式）。
- **可观测性**：N1 落地后，崩溃/异常可选本地落盘（已有 `log_to_file` 基础），便于用户反馈附日志。
- **每新增一个用户可配置项**：同步 `types.rs` 默认值（`serde(default)`）+ `stores/app.ts` 接口 + Settings UI + 测试，遵循 IMPROVEMENT_PLAN 既定约定。
