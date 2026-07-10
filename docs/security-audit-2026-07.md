# Skylark 安全与缺陷审计报告

- 审计日期：2026-07-09
- 审计版本：v0.5.3（`main` @ 9810241）
- 审计范围：逻辑层（`src-tauri/src/*.rs`，13 个模块）+ UI 层（`src/**`，Vue 3 / Pinia）+ Tauri 配置与构建脚本
- 方法：四路并行专项审计（进程/特权、网络/解析/持久化、前端、配置/IPC/供应链），高危项由主审逐条回读源码复核

---

## 一、结论摘要

代码整体防御意识不错：订阅解析全程走 `serde_json` / `json!` 转义（不存在 JSON 结构注入）、文件路径 id 有 `is_safe_id` 白名单、敏感文件用 `write_atomic` + 0600、更新下载强制 HTTPS 且带 SHA-256、前端唯一的 `v-html` 有 DOMPurify 兜底。**未发现可被远程订阅直接触发的内存破坏或一键 RCE。**

但存在 **5 个高危问题**，集中在三条线上：

1. **macOS 的 sudoers 规则未限制参数** —— 把「以 root 跑 sing-box」放大成「以 root 跑任意配置」，等价于本地提权到 root（H-1、H-2）。
2. **`csp: null` + webview 被授予 `shell:allow-execute/spawn/open`** —— 任何 XSS 直接升级为本机代码执行；而这几个权限**应用根本没用到**（sing-box 由 Rust 侧 `TokioCommand` 拉起）（H-3）。
3. **自更新是手写的、无签名验证**，且 `cmd_download_app_update` 接受前端传入的任意 URL，SHA-256 参数是 `Option`，传 `None` 就完全跳过校验（H-4）。

另有 1 个高危**功能性 BUG**（H-5）：设置页 TUN 开关只写标志位不建隧道，却让全局「已连接」指示灯变绿 —— 用户会误以为已在代理，实际流量直连。对代理客户端而言这是隐私层面的实质伤害。

| 等级 | 数量 | 编号 |
|---|---|---|
| 高危 High | 5 | H-1 ~ H-5 |
| 中危 Medium | 8 | M-1 ~ M-8 |
| 低危 Low | 9 | L-1 ~ L-9 |

复核说明：以下每条高危均已在报告中附「已核实」标记，代表主审已回读对应源码确认成立，非子代理单方结论。

---

## 二、高危问题（High）

### H-1 macOS sudoers 规则未限制参数 → 本地提权到 root ✅已核实
- 位置：`src-tauri/src/tun.rs:200`（写入规则），消费于 `singbox.rs:396`
- 证据（安装脚本以 root 写入 `/etc/sudoers.d/skylark`）：
  ```
  {user} ALL=(root) NOPASSWD: {bin}, /usr/bin/pkill -TERM -f {bin}, /usr/bin/pkill -KILL -f {bin}
  ```
  其中 `bin = /Library/Skylark/sing-box`。第一项**只写了二进制路径、没有参数约束** —— 在 sudoers 语义里等于「允许携带任意参数免密执行」。
- 攻击场景：安装一次后，以该登录用户身份运行的**任何**进程（第二个登录会话、被入侵的依赖、恶意脚本）都可执行：
  ```
  sudo -n /Library/Skylark/sing-box run -c /tmp/evil.json
  ```
  sing-box 即以 root 运行。构造的配置可用 `log.output` / `cache_file.path` 把 root 拥有的文件指向任意路径（如向 `/etc/sudoers.d/` 或 `/etc/pam.d/` 落文件），获得持久化 root。一条「只跑某个固定二进制」的授权被放大成通用的用户→root 提权。
- 修复：把规则收紧到精确调用，例如 `NOPASSWD: /Library/Skylark/sing-box run -c /Library/Skylark/config.json`，并把该配置放到 root 拥有的目录（而非用户可写的 app-data）。更稳妥：改用一个动作固定、不可参数化的 launchd 特权 helper，而不是授权整个内核二进制。

### H-2 root 进程消费「用户可写」的配置文件 → 恶意/被篡改配置以 root 运行（TOCTOU）✅已核实
- 位置：`commands.rs:133`（写配置）、`singbox.rs:389-397`（提权启动）
- 证据：TUN 配置写到 `~/Library/Application Support/Skylark/config.json`（用户属主、普通 `std::fs::write` → 0644，**未走** `write_atomic`），随后 `sudo -n … run -c <该文件>` 由 root 解析。此外配置是用**用户**二进制 `check_config` 校验、却由 **root** 二进制执行同一个可变文件 —— 校验与执行之间存在 TOCTOU。
- 攻击场景：任何能在「生成配置」到「root 启动」之间改写 `config.json` 的主体（文件属主本人、或竞争写入的本地进程）即可控制 root 解析的内容，结合 sing-box 的写文件配置项 = root 任意写。与 H-1 是同一类根因（特权进程信任了用户可写的输入）。
- 修复：把 TUN 配置写到 root 拥有的目录、作为特权动作的一部分交付；或通过用户不可改写的通道把配置交给特权 helper；至少用原子写 + 每次启动不可预测的路径。

### H-3 `csp: null` 叠加未使用的 `shell:allow-*` 权限 → XSS 直接升级为本机 RCE ✅已核实
- 位置：`src-tauri/tauri.conf.json:29`（`"csp": null`）、`tauri.conf.json:74`（`"shell": {"open": true}`）、`capabilities/default.json:18-21`（`shell:allow-spawn/kill/open/execute`）
- 现状：无 CSP 意味着注入到 webview 的任何脚本都能触达授予 `main` 窗口的**全部**能力。应用存在 HTML 注入面 —— GitHub release notes 经 `v-html` 渲染（`Settings.vue:619` / `:724`）。当前该内容过了 DOMPurify，因此**今天不是一键 XSS**；但 `csp: null` 使 DOMPurify 成为**唯一**防线，一旦出现一次 DOMPurify 绕过（本项目钉 `dompurify ^3.4.11` / `marked ^18`），即为完整脚本执行，且无 CSP 兜底。
- 关键放大器：`shell` 插件 `open: true`（非正则白名单）允许 webview 用 `open()` 把**任意**路径/URL 交给系统默认处理器；Windows 上打开 `.exe` / `.lnk` / UNC 路径（`\\attacker\share\evil.exe`）即执行；叠加 `shell:allow-execute`/`spawn`，注入脚本有直达进程启动的路径。而 **TUN 模式下该进程是提权运行的**。
- 实测确认：`grep -rn "@tauri-apps/plugin-shell" src/` → **0 处**。sing-box 由 Rust 侧 `TokioCommand::new`（`singbox.rs:207/228/396`）拉起，不经 Tauri shell 侧车。**这四个 shell 权限 + `open:true` 是 100% 未使用的纯攻击面。**
- 修复（优先级最高、成本最低）：
  1. 删除 `capabilities/default.json` 里 4 条 `shell:allow-*`，删除 `tauri.conf.json` 的 `"shell": {"open": true}` 块，并可移除 `tauri-plugin-shell` / `@tauri-apps/plugin-shell` 依赖。
  2. 设置真实 CSP。本应用不加载任何远程字体/图片/CDN（`index.html` 只引本地 `/logo.png`，图表走 canvas，唯一远程 HTTP 在 Rust reqwest 里），可用收紧策略：
     ```json
     "security": {
       "csp": "default-src 'self'; img-src 'self' data: asset: http://asset.localhost; font-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; frame-ancestors 'none'"
     }
     ```
     注意：`'unsafe-inline'` 只加在 `style-src`（Vue scoped 样式 / Tailwind v4 / chart.js 内联样式需要），**不要**给 `script-src` 加 `'unsafe-inline'`/`'unsafe-eval'`。待测：`vue-i18n ^11` 运行时编译可能需要 `script-src 'unsafe-eval'`，若消息渲染失败请改用 `@intlify/unplugin-vue-i18n` 预编译，而非放宽 CSP。

### H-4 自更新手写、无签名验证；下载 URL 由前端传入、SHA-256 可缺省 → 供应链 / XSS 提权代码执行 ✅已核实
- 位置：`commands.rs:1959`（`cmd_download_app_update(download_url, sha256: Option<String>)`）、`commands.rs:1931`（`cmd_download_singbox` 同型）、执行于 `updater.rs:962`（`Command::new(&installer_path).spawn()`）
- 证据：
  - 无 `tauri-plugin-updater`、无 `tauri.conf.json` 的 `"updater"` 块、无 pubkey —— Tauri 内置签名更新器**未启用**。
  - `updater.rs:348` / `:875` 均为 `if let Some(expected) = …`：`sha256` 是 `Option`，**传 `None` 即完全跳过完整性校验**。
  - 即便校验，期望摘要与下载 URL 来自**同一个 GitHub API 响应**（`asset["digest"]`）—— 摘要不是独立信任锚，只能挡传输损坏，挡不住 release 内容被控制（仓库/账号/token 被入侵、恶意维护者、GitHub 侧事故）。
- 攻击场景：任何前端代码执行（如经 DOMPurify 绕过的 XSS）可调用 `cmd_download_app_update`，传攻击者的 HTTPS URL + 匹配的 `sha256`（两者都由攻击者掌握），下载的安装器随后被 `spawn()` —— TUN 下为提权执行。或直接传 `None` 跳过校验。
- 修复：改用 `tauri-plugin-updater` + 编入二进制的 pubkey，对 release 产物做 minisign/ed25519 签名校验，**校验强制化（fail-closed）**；下载 URL/host 由后端自身从 GitHub 取得，前端只传 release id/channel，不接受任意 URL。

### H-5 设置页 TUN 开关只写标志位、不建隧道，却点亮全局「已连接」→ 用户误判自己在代理（隐私伤害）✅已核实
- 位置：`Settings.vue:1094`（复选框）→ `Settings.vue:180`（debounce 保存）→ `app.ts:169`（`proxying` 计算属性）
- 证据链：
  - 复选框 `<input type="checkbox" v-model="localConfig.tun_enabled" />` **无 `@change`**，仅由 deep watcher 防抖调 `store.saveConfig()`。
  - `saveConfig`（`app.ts:661`）只 `invoke("cmd_save_app_config")` 写标志位，**从不调 `setConnectionMode`**；后端 `cmd_save_app_config`（`commands.rs:1362`）也只落盘，不重建/重启内核。（已核实：`saveConfig` 路径全程无 `setConnectionMode` 调用。）
  - 而全局指示灯信任该标志：
    ```ts
    const proxying = computed(() => {
      if (connecting.value === "system" || connecting.value === "tun") return true;
      if (connecting.value === "off") return false;
      return status.value.running && (config.value.tun_enabled || systemProxyEnabled.value);
    });
    ```
    按 store 自身注释（`app.ts:164`），持久内核模型下 `status.running` 几乎全程为真。
- 失败场景：用户在设置页打开「启用 TUN」开关（哪怕只是想看下面的管理员/WinTun 清单），并**不去仪表盘真正连接**。`tun_enabled` 变 `true` + 内核在跑 → `proxying` 计算为 `true`：仪表盘显示「已连接 / TUN 模式」、侧栏状态点变绿、托盘提示「● 已连接」、`pollTraffic` 启动会话计时 —— 但**从未创建 TUN 接口，全部系统流量走直连**。依赖该指示的用户会误以为受保护。`Home.vue:33` 的 `tunOn` 同样反映此假状态。
- 修复：`proxying` / TUN 开关状态不要从持久标志 `config.tun_enabled` 推导，应基于后端上报的真实路由运行态（一个与 `systemProxyEnabled` 平行的权威 `tun_active` 布尔）；或让设置页复选框直接驱动 `setConnectionMode('tun'/'off')` 而非仅持久标志。

---

## 三、中危问题（Medium）

### M-1 订阅响应体无大小上限 → 内存耗尽 DoS ✅已核实
- 位置：`commands.rs:2152` `let content = resp.text().await?;`（无 size cap），无人值守地被 `auto_update.rs:118` 定时重拉。
- 15s 超时不限制体积；恶意/被 MITM 的机场可在超时内流式返回数 GB，`resp.text()` 全缓冲 → OOM 崩溃，且自动更新定时器每次启动都复现。base64/`serde_yaml` 还会再翻倍。
- 修复：先查 `content_length()` 或用 `bytes_stream()` 超过几 MB 即中止。

### M-2 `serde_yaml` alias 展开（billion laughs）DoS ✅已核实
- 位置：`subscription.rs:326` `serde_yaml::from_str(content)`；依赖 `Cargo.toml` `serde_yaml = "0.9"`（**已归档、不再维护**）。
- Clash YAML 走此路径，无 anchor/alias 展开上限。~1–2KB 的嵌套 alias 可在 `from_str` 阶段膨胀到数 GB，在应用自身逻辑执行前就耗尽内存/打满 CPU。
- 修复：解析前限制输入大小（M-1 一并解决）；迁移到维护中的解析器（`serde_yml` / `saphyr`）或预扫描拒绝过量 alias。

### M-3 `api_secret` 以 world-readable、非原子方式写入 → 本地 Clash API 被接管 ✅已核实
- 位置：`config.rs:114` `let _ = fs::write(&path, &secret);` —— **绕过了** `write_atomic`（后者会在 Unix 设 0600）。
- 该密钥是 `127.0.0.1:{api_port}` Clash `external_controller` 的唯一防护。多用户主机上，另一个本地用户读到 `~/.local/share/Skylark/api_secret` 后即可用 `Authorization: Bearer <secret>` 切换受害者节点、读出站配置、改路由模式。
- 修复：`api_secret` 改走 `write_atomic`（或显式 `set_permissions(0o600)`）。

### M-4 `rules.json` / `rule_providers.json` 非原子且 world-readable ✅已核实
- 位置：`rules.rs:424`、`rules.rs:471` 裸 `std::fs::write`。`RuleProvider.url`（用户提供）常含 token。同居用户可读到；写入中途崩溃会截断文件、静默回落到 `default()` 丢规则。
- 修复：两处改走 `config::write_atomic`（把它 `pub(crate)`）。

### M-5 `pinSHA256` 参数静默关闭 TLS 证书校验 ✅已核实
- 位置：`subscription.rs:1115-1118`：只要 Hysteria2 节点带 `pinSHA256`（含大小写/下划线变体）就置 `insecure=true`，且 pin 值从不真正校验（sing-box 无对应能力，代码注释亦承认）。
- 场景：恶意订阅 `hysteria2://pw@host:443?sni=fake.apple.com&pinSHA256=AA:BB#N` → 客户端关证书校验、也不做 pin 校验，中间人对该代理跳无需任何 TLS 验证。「pin」实为降级开关。
- 修复：不要因存在 pin 就置 `insecure`；要么实现真实 pinning，要么保持校验并丢弃不支持的 pin。

### M-6 `taskkill /IM` / `pkill -f` 按名杀进程 → 误杀无关进程 ✅已核实
- 位置：`singbox.rs:262`（Win `taskkill /F /IM sing-box.exe`）、`singbox.rs:277`（Unix `pkill -f "sing-box run -c"`），每次 `start_singbox` 及启动失败清理都执行。
- 用户机上任何其它基于 sing-box 的客户端/CLI 隧道会被每次启动/切换 TUN 时强杀；`-f` 子串匹配还会误杀命令行恰好含该子串的良性进程。无 PID/属主限定。
- 修复：只杀本应用 spawn 的 PID；广义按名清理仅限首次启动的孤儿扫描，并匹配完整解析后的二进制路径。

### M-7 危险命令：任意 URL 下载即成为被执行的内核/安装器 ✅已核实
- 位置：`commands.rs:1930`（`cmd_download_singbox`）、`commands.rs:1958`（`cmd_download_app_update`）。与 H-4 同根，单列强调「下载物随后被提权执行」这一后果面：下载的 sing-box 会在 TUN 下以 admin/root 运行。
- 修复：见 H-4；摘要强制、URL/host 由后端派生。

### M-8 订阅 / 规则提供者 URL 未限制 scheme（SSRF）✅已核实
- 位置：`commands.rs:2133`（`fetch_url_via`）无 scheme/host 白名单。reqwest 不支持 `file://`，但 `http://127.0.0.1:9090/…`、`http://169.254.169.254/…`（云元数据）等内网地址可经直连尝试触达。桌面端权限内、结果对用户可见，故列中危。
- 修复：限制 `http`/`https`，并对订阅/提供者拉取可选阻断 RFC1918/loopback/link-local 字面量 host。

---

## 四、低危问题（Low）

- **L-1 单个畸形节点使整份生成配置被拒（whole-proxy DoS）** —— `subscription.rs:396/845/922` 等对 `net`/`type`/`id`/`cipher` 直通不校验；一个未知 transport / 非 UUID id 会让 sing-box 整份 config 启动失败（「控制端口未就绪」），而非只丢该节点。修复：按节点校验、在 `sanitize_and_dedupe_tags` 里丢弃非法项（现已能丢空 server/零端口节点）。
- **L-2 窗口关闭处理器对可毒化互斥锁 `.unwrap()`** —— `lib.rs:160`，全局唯一未用 `unwrap_or_else(|e| e.into_inner())` 的锁点，`app_config` 一旦被毒化则关窗 panic。修复：统一容忍毒化模式。
- **L-3 订阅 URL（常含 token）泄漏进返回前端的错误串** —— `commands.rs:2119`：reqwest 错误 `Display` 含请求 URL，机场订阅的 `token=` 会随失败信息进 UI/日志。修复：错误映射时去掉 URL 或脱敏 query。
- **L-4 安装器临时路径由未清洗的 URL 段派生** —— `updater.rs:841` `download_url.split('/').last()`，`..` 型值可逃逸 temp 目录。实际 URL 来自钉死的 GitHub API，故低危；建议校验 host 白名单 + 文件名白名单。
- **L-5 `stop_singbox` 存在 PID 复用竞态** —— `singbox.rs:574-664`：快照 PID 后无条件 `kill`，若核心已退出且 PID 被复用则误伤无关进程。窗口很短。修复：优先经保留的 `Child` 句柄发信号，或发信号前复核存活/身份。
- **L-6 `traffic_stats.json` / 更新缓存非原子写** —— `stats.rs:98`、`updater.rs:155/609`。非敏感，崩溃中途截断丢历史。修复：复用 `write_atomic`。
- **L-7 设置页防抖保存吞掉后端失败** —— `Settings.vue:180`：`saveConfig` 无 try/catch，端口非法被后端拒绝时无 toast，`localConfig` 仍显示被拒值，用户误以为已保存。修复：包 try/catch + `fb.toastError` + 从 `store.config` 回同步表单。
- **L-8 订阅更新 / 自动更新开关吞掉后端错误** —— `Subscriptions.vue:139/279`：`updateSub`/`toggleAutoUpdate` 只有 `finally` 无 `catch`，机场不可达时点击「无反应」、静默保留旧节点。修复：补 `catch` + 错误 toast。
- **L-9 后台更新监听可覆盖用户当前 channel 结果 + release notes 链接无 `rel`/`target`** —— `Settings.vue:530-545`（陈旧覆盖竞态：用户切 beta 后 ~45s 的后台 stable 检查覆盖 `appLatestRelease`）；`Settings.vue:31`（DOMPurify 不加 `target=_blank`/`rel=noopener`，`csp:null` 下点恶意 release-notes 链接会让主 webview 导航到攻击者页面）。修复：按当前 channel 过滤后台事件；加 `afterSanitizeAttributes` 钩子强制外链新窗 + `noopener noreferrer`。

---

## 五、已核实为安全（覆盖说明）

审计同时确认以下面**未**发现具体缺陷，列出以说明覆盖度：

- **注入面**：节点名/订阅字符串从不进入 `Command` 参数；延迟探测用 `reqwest::Url::path_segments_mut()`（percent 编码）。远程字符串经 `json!`/serde 只能成为转义的 JSON 字符串值，**无法**注入 `outbounds`/`inbounds`/`experimental`/`command_server` 键或 `tun` inbound；控制 tag（`proxy`/`direct`/`block`/`auto`）被保留并对碰撞改名。
- **解析 panic**：解析器内无可达的 `unwrap`/`expect`/越界；`percent_decode` 有 `i+2<len` 守卫且用 `from_utf8_lossy`（无非字符边界切片）；base64/`serde_json`/`Url::parse` 均用 `?`。
- **路径穿越**：`is_safe_id` 白名单 `[A-Za-z0-9_-]`；节点名从不用作路径；`sanitize_profile_name` 阻断 `/`、`\`、`..`、空、>64（有单测）。
- **前端 XSS**：全项目唯一 `v-html` 是 release-notes，经 **DOMPurify 3.4.11** 安全 profile 清洗后才绑定；所有远程订阅数据（节点名/备注/host/规则/日志行）均走 `{{ }}` 文本插值自动转义；`.ts` 内无 `eval`/`new Function`/`innerHTML`/`document.write`；`opener` 仅接收后端返回的本地导出路径，从不 `open` 远程 URL。
- **并发**：`switching: AtomicBool` 用 `swap(true, SeqCst)` 测试并置位（无竞态）；`core_lock`（tokio mutex）序列化所有核心重启；无 std `MutexGuard` 跨 `.await` 持有。
- **连接错误处理**：`setConnectionMode`（`app.ts:239`）捕获失败、置 `error.value`、并 `fetchConfig/fetchStatus/refreshSystemProxy` 回同步 —— 系统代理切换失败**不会**留下假「on」（H-5 的假状态源自另一条 TUN 标志路径，与此路径无关）。
- **更新完整性（当 sha256 提供时）**：下载强制 `https://`；GitHub bearer token 只附在 api.github.com 元数据调用、**不**附在会重定向到 CDN 的资产下载（防重定向凭据泄漏）；无 `danger_accept_invalid_certs`；更新客户端用 `no_proxy()` 避免经 sing-box。
- **敏感文件**：`subscriptions.json`（含 token URL）、`nodes.json`、`outbounds.json`（含节点密码）、`app_config.json` 均经 `write_atomic` + Unix 0600，损坏文件保留为 `.corrupt` 而非静默覆盖。
- **全局快捷键**：加速键为硬编码常量、默认关闭、`register` 全部 try/catch，无配置/订阅数据进入 `register()` —— 无 panic/劫持路径。
- **构建脚本 `prepare-installer.js`**：仅用 `sharp` 栅格化本地 SVG/PNG，不下载、不执行下载内容。

另注一个**构建期供应链缺口（中/高，取决于对构建机的信任）**：
- **`scripts/fetch-singbox.js:53-75`** 从 GitHub 下载 sing-box 内核，HTTPS 但**无 checksum / 无签名校验**，版本可来自未校验的 `SING_BOX_VERSION` 环境变量。构建机 TLS 被拦截或 release 被替换即打进木马内核并以厂商名义签名分发。修复：钉死版本并对下载归档校验 SagerNet 发布的 SHA-256，不匹配则构建失败。（应用内运行时下载 `download_singbox` 已强制 HTTPS + SHA-256，但存在 H-4 的「摘要非独立信任锚」同款问题。）

---

## 六、修复优先级建议

1. **H-3**：加 CSP + 删除未使用的 `shell`/`http`/`process`/`store`/`notification` 及多余 `core:window`/`opener` 授权 —— 成本最低、直接掐断「XSS→一切」桥梁。
2. **H-1 / H-2**：收紧 sudoers 参数、把 root 消费的配置移到 root 拥有目录（或改特权 helper）—— 关闭本地提权。
3. **H-4**：自更新改为钉死 pubkey 的签名校验、fail-closed，URL 由后端派生；`fetch-singbox.js` 加构建期 checksum。
4. **H-5**：`proxying`/TUN 状态改用后端权威运行态 —— 修掉「假已连接」隐私伤害。
5. **M 级**：订阅体积上限（M-1/M-2）、`api_secret` 与 `rules.*` 走原子 0600 写（M-3/M-4）、`pinSHA256` 不再关校验（M-5）、按 PID 杀进程（M-6）。
6. **L 级**：随手清理错误吞没、URL token 泄漏、非原子写等。
7. 例行：保持 DOMPurify 最新；跑 `npm audit` / `cargo audit`（重点关注解析不可信输入的 `zip`/`tar`/`serde_yaml`）。
