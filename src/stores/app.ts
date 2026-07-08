import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { register, unregisterAll } from "@tauri-apps/plugin-global-shortcut";
import { detectLocale } from "../i18n";

export interface SingboxStatus {
  running: boolean;
  uptime?: number;
  pid?: number;
  version?: string;
}

export interface Subscription {
  id: string;
  name: string;
  url: string;
  sub_type: "clash" | "v2ray" | "sip008" | "unknown";
  node_count: number;
  last_update?: string;
  auto_update: boolean;
  update_interval: number;
  // Airport usage / quota from the `Subscription-Userinfo` header (optional).
  upload?: number;   // bytes
  download?: number; // bytes
  total?: number;    // bytes
  expire?: number;   // unix timestamp (seconds)
  // Node-name filters / region grouping (N2).
  include?: string | null;
  exclude?: string | null;
  group_by_region?: boolean;
}

export interface ProxyNode {
  id: string;
  name: string;
  group: string;
  protocol: string;
  server: string;
  port: number;
  latency?: number;        // ms
  download_speed?: number; // KB/s
  is_active: boolean;
  subscription_id?: string;
}

export interface TrafficDay {
  date: string;   // YYYY-MM-DD
  upload: number; // bytes
  download: number;
}

export interface ProxyGroup {
  id: string;
  name: string;
  group_type: string; // "selector" | "urltest"
  nodes: string[];    // member node names
}

export interface SpeedResult {
  latency_ms?: number;
  download_kbps?: number;
}

export interface AppConfig {
  proxy_mode: "rule" | "global" | "direct" | "tun";
  startup_with_system: boolean;
  startup_minimized: boolean;
  allow_lan: boolean;
  http_port: number;
  socks_port: number;
  mixed_port: number;
  api_port: number;
  tun_enabled: boolean;
  log_level: string;
  theme: string;
  language: string;
  selected_subscription?: string;
  active_nodes: Record<string, string>;
  auto_update_interval: number;
  auto_update_notify: boolean;
  update_channel: string;
  close_to_tray: boolean;
  restore_proxy_on_startup: boolean;
  // last_proxy_running / last_system_proxy / last_app_version are backend-owned runtime
  // state. They're intentionally NOT modelled here so the UI can't bind or send them; the
  // backend ignores them on save anyway (see merge_runtime_fields in commands.rs).
  auto_test_url: string;
  auto_test_interval: number;
  auto_tolerance: number;
  enable_ipv6: boolean;
  dns_local: string;
  log_to_file: boolean;
  subscription_user_agent: string;
  enable_global_shortcuts: boolean;
}

export interface TrafficPoint {
  time: number;
  upload: number;
  download: number;
}

export const useAppStore = defineStore("app", () => {
  const status = ref<SingboxStatus>({ running: false });
  const subscriptions = ref<Subscription[]>([]);
  const nodes = ref<ProxyNode[]>([]);
  const proxyGroups = ref<ProxyGroup[]>([]);
  const config = ref<AppConfig>({
    proxy_mode: "rule",
    startup_with_system: false,
    startup_minimized: false,
    allow_lan: false,
    http_port: 7890,
    socks_port: 7891,
    mixed_port: 7890,
    api_port: 9090,
    tun_enabled: false,
    log_level: "info",
    theme: "system",
    // Placeholder until fetchConfig() loads the backend value. Detect from the OS so the
    // App.vue immediate locale watch matches the first paint instead of forcing zh-CN.
    language: detectLocale(),
    active_nodes: {},
    auto_update_interval: 24,
    auto_update_notify: true,
    update_channel: "stable",
    close_to_tray: true,
    restore_proxy_on_startup: false,
    auto_test_url: "https://www.gstatic.com/generate_204",
    auto_test_interval: 3,
    auto_tolerance: 50,
    enable_ipv6: false,
    dns_local: "223.5.5.5",
    log_to_file: false,
    subscription_user_agent: "v2rayN/6.45",
    enable_global_shortcuts: false,
  });
  const trafficHistory = ref<TrafficPoint[]>([]);
  // Cumulative bytes since the core started (authoritative, from the Clash API).
  const totalUpload = ref(0);
  const totalDownload = ref(0);
  // Live speed in bytes/s, derived from the per-second delta of the totals above.
  const uploadSpeed = ref(0);
  const downloadSpeed = ref(0);
  const loading = ref(false);
  // Flips true once the first data load (init) resolves and never flips back, so
  // views can show a loading skeleton on cold start without it re-triggering
  // during later connection ops (which toggle `loading`).
  const initialized = ref(false);
  const error = ref<string | null>(null);
  // The connection target currently being applied, for optimistic UI. The dashboard
  // toggles reflect this immediately (flip + "连接中…") while the core (re)starts in
  // the background, then reconcile to the real runtime state on completion.
  const connecting = ref<null | "system" | "tun" | "off">(null);
  // Whether the Windows system proxy registry toggle is currently on. Kept in the store
  // (refreshed by the global traffic poller) so any view can derive `proxying`.
  const systemProxyEnabled = ref(false);

  // With the persistent-core model the core is almost always running while the app is
  // open, so "core running" no longer means "actively proxying". `proxying` is the real
  // signal the UI should use for status: the core is up AND a routing path is active
  // (system proxy registry on, or TUN mode on). While a switch is being applied we honor
  // the optimistic target so indicators flip instantly.
  const proxying = computed(() => {
    if (connecting.value === "system" || connecting.value === "tun") return true;
    if (connecting.value === "off") return false;
    return status.value.running && (config.value.tun_enabled || systemProxyEnabled.value);
  });

  // Wall-clock ms at which the current proxy session began (off → on), or null while not
  // proxying. Owned by the app-scoped traffic poller so it survives page navigation — the
  // dashboard derives "运行时长" from it instead of from a component-local timer that would
  // reset on every remount.
  const proxySessionStartMs = ref<number | null>(null);

  async function refreshSystemProxy() {
    try {
      systemProxyEnabled.value = await invoke<boolean>("cmd_get_system_proxy_status");
    } catch {
      /* keep last value on a transient error */
    }
  }
  // Concrete node currently picked by the active auto (urltest) group.
  const activeNodeNow = ref<string | null>(null);

  // The proxy group's currently selected tag: a node name, "auto", or "auto-<subId>".
  const activeProxyTag = computed(() => config.value.active_nodes["proxy"] ?? "");
  // True when *any* dynamic auto group is selected (global or per-subscription).
  const isAutoGroup = computed(
    () => activeProxyTag.value === "auto" || activeProxyTag.value.startsWith("auto-")
  );
  const activeNode = computed(() => {
    // Prefer the node flagged as active; fall back to matching by saved tag name.
    const byFlag = nodes.value.find((n) => n.is_active);
    if (byFlag) return byFlag;
    const activeTag = activeProxyTag.value;
    if (!activeTag || isAutoGroup.value) return undefined;
    return nodes.value.find((n) => n.name === activeTag);
  });

  async function fetchStatus() {
    try {
      status.value = await invoke<SingboxStatus>("cmd_get_singbox_status");
    } catch (e) {
      console.error("fetchStatus error:", e);
    }
  }

  function updateTrayTooltip() {
    const modeMap: Record<string, string> = { rule: "规则", global: "全局", direct: "直连", tun: "TUN" };
    const mode = modeMap[config.value.proxy_mode] ?? config.value.proxy_mode;
    const node = isAutoGroup.value
      ? (activeNodeNow.value ? `自动 → ${activeNodeNow.value}` : "自动选优")
      : (activeNode.value?.name ?? "未选择");
    const state = proxying.value ? "● 已连接" : "○ 未连接";
    const tooltip = `Skylark\n${state}\n节点: ${node}\n模式: ${mode}`;
    invoke("cmd_update_tray_tooltip", { tooltip }).catch(() => {});
  }

  function syncTrayMenu(sysProxyEnabled: boolean, tunEnabled?: boolean) {
    const tun = tunEnabled ?? config.value.tun_enabled ?? false;
    invoke("cmd_sync_tray_menu", { sysProxyEnabled, tunEnabled: tun }).catch(() => {});
  }

  /** Unified connection control. The dashboard exposes only two mutually-exclusive
   *  switches — "system" proxy and "tun" — plus an implicit "off". Each call sets
   *  the TUN flag, (re)starts the core so the matching config takes effect, and
   *  enables/clears the Windows system proxy accordingly. On failure (e.g. TUN
   *  without admin rights) the TUN flag is rolled back so the UI stays truthful. */
  // Unified connection control. The backend (cmd_set_connection_mode → apply_connection_mode)
  // owns all the lifecycle logic for the persistent-core model: it keeps the core running,
  // toggling only the system proxy for "system"/"off" (instant) and rebuilding + restarting
  // for "tun". The store just invokes it, then refreshes the reactive state.
  async function setConnectionMode(target: "system" | "tun" | "off") {
    loading.value = true;
    connecting.value = target;
    error.value = null;
    try {
      await invoke("cmd_set_connection_mode", { mode: target });
      // Reconcile reactive state with reality (tun_enabled may have changed).
      await fetchConfig();
      await fetchStatus();
      await refreshSystemProxy();
      if (target === "off") {
        activeNodeNow.value = null;
      } else {
        resetTrafficStats();
        ensureActiveNowPoller();
      }
      syncTrayMenu(systemProxyEnabled.value, config.value.tun_enabled);
      updateTrayTooltip();
    } catch (e) {
      error.value = String(e);
      await fetchConfig();
      await fetchStatus();
      await refreshSystemProxy();
    } finally {
      loading.value = false;
      connecting.value = null;
    }
  }

  // ── Global hotkeys (N4) ─────────────────────────────────────────────
  // Fixed, sensible defaults. Registration/handling happens entirely in JS via the
  // global-shortcut plugin; the Rust side only initialises the plugin. Disabled by
  // default (config flag) so the app never grabs system-wide combos unasked.
  const SHORTCUT_TOGGLE_PROXY = "CommandOrControl+Shift+P";
  const SHORTCUT_TOGGLE_TUN = "CommandOrControl+Shift+T";
  const SHORTCUT_CYCLE_MODE = "CommandOrControl+Shift+R";

  async function applyGlobalShortcuts(enabled: boolean) {
    // Always clear first so toggling off (or re-applying) never leaves stale bindings.
    try { await unregisterAll(); } catch { /* nothing registered yet */ }
    if (!enabled) return;
    try {
      await register(SHORTCUT_TOGGLE_PROXY, (e) => {
        if (e.state === "Pressed") setConnectionMode(systemProxyEnabled.value ? "off" : "system");
      });
      await register(SHORTCUT_TOGGLE_TUN, (e) => {
        if (e.state === "Pressed") setConnectionMode(config.value.tun_enabled ? "off" : "tun");
      });
      await register(SHORTCUT_CYCLE_MODE, (e) => {
        if (e.state !== "Pressed") return;
        const m = config.value.proxy_mode;
        const next = m === "rule" ? "global" : m === "global" ? "direct" : "rule";
        setProxyMode(next);
      });
    } catch (err) {
      console.error("global shortcut registration failed:", err);
    }
  }

  async function fetchSubscriptions() {
    subscriptions.value = await invoke<Subscription[]>("cmd_get_subscriptions");
  }

  async function addSubscription(
    name: string,
    url: string,
    filters?: { include?: string | null; exclude?: string | null; groupByRegion?: boolean }
  ) {
    const sub = await invoke<Subscription>("cmd_add_subscription", {
      name,
      url,
      include: filters?.include ?? null,
      exclude: filters?.exclude ?? null,
      groupByRegion: filters?.groupByRegion ?? false,
    });
    subscriptions.value.push(sub);
    await fetchNodes();
    return sub;
  }

  async function importSubscriptionFromText(
    name: string,
    content: string,
    filters?: { include?: string | null; exclude?: string | null; groupByRegion?: boolean }
  ) {
    const sub = await invoke<Subscription>("cmd_import_subscription_from_text", {
      name,
      content,
      include: filters?.include ?? null,
      exclude: filters?.exclude ?? null,
      groupByRegion: filters?.groupByRegion ?? false,
    });
    subscriptions.value.push(sub);
    await fetchNodes();
    return sub;
  }

  /** Update an existing subscription's name filters / region grouping and re-apply
   * them to the cached content (no network fetch). Returns the new node count. */
  async function setSubscriptionFilters(
    id: string,
    include: string | null,
    exclude: string | null,
    groupByRegion: boolean
  ) {
    const nodeCount = await invoke<number>("cmd_set_subscription_filters", {
      id,
      include,
      exclude,
      groupByRegion,
    });
    const idx = subscriptions.value.findIndex((s) => s.id === id);
    if (idx !== -1) {
      subscriptions.value[idx] = {
        ...subscriptions.value[idx],
        include,
        exclude,
        group_by_region: groupByRegion,
        node_count: nodeCount,
      };
    }
    await fetchNodes();
    return nodeCount;
  }

  async function updateSubscription(id: string) {
    const sub = await invoke<Subscription>("cmd_update_subscription", { id });
    const idx = subscriptions.value.findIndex((s) => s.id === id);
    if (idx !== -1) subscriptions.value[idx] = sub;
    await fetchNodes();
    return sub;
  }

  async function deleteSubscription(id: string) {
    await invoke("cmd_delete_subscription", { id });
    subscriptions.value = subscriptions.value.filter((s) => s.id !== id);
    nodes.value = nodes.value.filter((n) => n.subscription_id !== id);
  }

  async function saveSubscriptionSettings(id: string, autoUpdate: boolean, updateInterval: number) {
    await invoke("cmd_save_subscription_settings", { id, autoUpdate, updateInterval });
    const idx = subscriptions.value.findIndex((s) => s.id === id);
    if (idx !== -1) {
      subscriptions.value[idx] = {
        ...subscriptions.value[idx],
        auto_update: autoUpdate,
        update_interval: updateInterval,
      };
    }
  }

  async function fetchNodes() {
    nodes.value = await invoke<ProxyNode[]>("cmd_get_nodes");
  }

  async function setActiveNode(nodeId: string) {
    await invoke("cmd_set_active_node", { nodeId });
    await fetchConfig();
    await fetchNodes();
    updateTrayTooltip();
  }

  // Switch to a dynamic urltest group: the core continuously picks the fastest
  // node. `group` defaults to the global "auto"; pass "auto-<subId>" for a
  // per-subscription group. Takes effect immediately when the proxy is running.
  async function setAutoNode(group?: string) {
    await invoke("cmd_set_auto_node", { group: group ?? null });
    await fetchConfig();
    await fetchNodes();
    updateTrayTooltip();
    await refreshActiveNow();
  }

  // ── Custom proxy groups ──────────────────────────────────────────
  async function fetchProxyGroups() {
    proxyGroups.value = await invoke<ProxyGroup[]>("cmd_get_proxy_groups");
  }

  // Persist the full group list. Changes apply on the next config rebuild
  // (reconnect / mode switch), like routing rules.
  async function saveProxyGroups(groups: ProxyGroup[]) {
    await invoke("cmd_save_proxy_groups", { groups });
    proxyGroups.value = groups;
  }

  // The concrete node the auto group is currently routing through (resolved via the
  // Clash API). Null when the proxy is stopped or not using an auto group.
  async function refreshActiveNow() {
    if (!status.value.running || !isAutoGroup.value) {
      activeNodeNow.value = null;
      return;
    }
    try {
      activeNodeNow.value = await invoke<string | null>("cmd_get_active_proxy_now");
    } catch {
      activeNodeNow.value = null;
    }
  }

  async function testNodeLatency(nodeId: string): Promise<number | null> {
    try {
      const ms = await invoke<number>("cmd_test_node_latency", { nodeId });
      const idx = nodes.value.findIndex((n) => n.id === nodeId);
      if (idx !== -1) nodes.value[idx] = { ...nodes.value[idx], latency: ms };
      return ms;
    } catch {
      const idx = nodes.value.findIndex((n) => n.id === nodeId);
      if (idx !== -1) nodes.value[idx] = { ...nodes.value[idx], latency: undefined };
      return null;
    }
  }

  async function testNodeSpeed(nodeId: string): Promise<SpeedResult | null> {
    try {
      const result = await invoke<SpeedResult>("cmd_test_node_speed", { nodeId });
      const idx = nodes.value.findIndex((n) => n.id === nodeId);
      if (idx !== -1) {
        nodes.value[idx] = {
          ...nodes.value[idx],
          latency: result.latency_ms,
          download_speed: result.download_kbps,
        };
      }
      return result;
    } catch {
      const idx = nodes.value.findIndex((n) => n.id === nodeId);
      if (idx !== -1) {
        nodes.value[idx] = { ...nodes.value[idx], latency: undefined, download_speed: undefined };
      }
      return null;
    }
  }

  // Force an immediate re-test of an auto group's nodes, then refresh which node it
  // now routes through. `group` defaults to the global "auto".
  async function testGroupDelay(group?: string) {
    try {
      await invoke("cmd_test_group_delay", { group: group ?? "auto" });
    } catch (e) {
      error.value = String(e);
    }
    await refreshActiveNow();
  }

  // ── Single shared poller for the active auto group's current node ───────────
  // One app-lifetime timer (idempotent) instead of per-page timers. refreshActiveNow
  // self-noops when the proxy is stopped or no auto group is selected, so the timer
  // does no network work unless it's actually needed.
  let activeNowTimer: ReturnType<typeof setInterval> | null = null;
  function ensureActiveNowPoller() {
    if (activeNowTimer) return;
    refreshActiveNow();
    activeNowTimer = setInterval(refreshActiveNow, 3000);
  }

  // ── Global traffic monitor ─────────────────────────────────────────────────
  // The cumulative totals come straight from the core (Clash API /connections
  // totals), so they count ALL traffic since the proxy service started and keep
  // counting no matter which page is open — fixing the old behaviour where totals
  // only accrued while the dashboard was visible. Live speed is the per-second
  // delta. The core resets its counters on restart, so a drop in the cumulative
  // value is treated as a fresh baseline rather than a (nonsensical) negative speed.
  let trafficTimer: ReturnType<typeof setInterval> | null = null;
  let lastUpTotal = 0;
  let lastDownTotal = 0;
  let trafficWasRunning = false;
  // Bytes accumulated since the last persistent-history flush. The poller runs every
  // second; we batch ~30s of deltas into one disk write to keep history I/O cheap.
  let pendingUp = 0;
  let pendingDown = 0;
  let flushCountdown = 30;

  async function flushTrafficHistory() {
    if (pendingUp === 0 && pendingDown === 0) return;
    const up = pendingUp;
    const down = pendingDown;
    pendingUp = 0;
    pendingDown = 0;
    try {
      await invoke("cmd_add_traffic_sample", { upload: up, download: down });
    } catch {
      // On failure, fold the bytes back so they aren't lost before the next flush.
      pendingUp += up;
      pendingDown += down;
    }
  }

  async function pollTraffic() {
    // Refresh status + system proxy here so the monitor reacts to changes from anywhere
    // (dashboard, tray, auto-restore) without depending on a mounted page.
    await fetchStatus();
    await refreshSystemProxy();

    // With the persistent core, "proxying" — not "core running" — is what gates traffic.
    const proxyingNow =
      status.value.running && (config.value.tun_enabled || systemProxyEnabled.value);

    if (!proxyingNow) {
      if (trafficWasRunning) {
        // Persist whatever accrued in this session before tearing down counters.
        await flushTrafficHistory();
        uploadSpeed.value = 0;
        downloadSpeed.value = 0;
        totalUpload.value = 0;
        totalDownload.value = 0;
        lastUpTotal = 0;
        lastDownTotal = 0;
        trafficWasRunning = false;
      }
      // Freeze the session timer whenever not proxying (independent of the traffic flag).
      proxySessionStartMs.value = null;
      return;
    }

    // Transition not-proxying → proxying: reset baseline and chart for a clean session.
    if (!trafficWasRunning) {
      lastUpTotal = 0;
      lastDownTotal = 0;
      clearTrafficHistory();
      trafficWasRunning = true;
    }
    // Start the session clock on the first proxying poll and keep it across core restarts
    // (e.g. an auto-group switch) — only a real off → on transition clears it above.
    if (proxySessionStartMs.value === null) {
      proxySessionStartMs.value = Date.now();
    }

    let up = 0;
    let down = 0;
    try {
      const t = await invoke<{ upload: number; download: number }>("cmd_get_traffic_total");
      up = t.upload;
      down = t.download;
    } catch {
      // Keep last good values on a transient API hiccup.
      return;
    }

    // Clamp negatives caused by a core counter reset.
    const upDelta = up >= lastUpTotal ? up - lastUpTotal : up;
    const downDelta = down >= lastDownTotal ? down - lastDownTotal : down;
    lastUpTotal = up;
    lastDownTotal = down;

    uploadSpeed.value = upDelta;
    downloadSpeed.value = downDelta;
    totalUpload.value = up;
    totalDownload.value = down;
    addTrafficPoint(upDelta, downDelta);

    // Accumulate for the persistent daily history; flush in ~30s batches.
    pendingUp += upDelta;
    pendingDown += downDelta;
    if (--flushCountdown <= 0) {
      flushCountdown = 30;
      await flushTrafficHistory();
    }
  }

  function ensureTrafficPoller() {
    if (trafficTimer) return;
    pollTraffic();
    trafficTimer = setInterval(pollTraffic, 1000);
  }

  // Immediately clear cumulative stats, live speed, the chart and the monitor's
  // baseline. Called on every (re)start of the core so a new proxy session always
  // begins counting from zero instead of inheriting the previous session's totals.
  function resetTrafficStats() {
    totalUpload.value = 0;
    totalDownload.value = 0;
    uploadSpeed.value = 0;
    downloadSpeed.value = 0;
    lastUpTotal = 0;
    lastDownTotal = 0;
    clearTrafficHistory();
    // Align the monitor's flag so its transition logic won't double-reset.
    trafficWasRunning = proxying.value;
  }

  async function fetchTrafficHistory(days?: number): Promise<TrafficDay[]> {
    return await invoke<TrafficDay[]>("cmd_get_traffic_history", { days: days ?? null });
  }

  // ── Config profiles (N6) ────────────────────────────────────────────
  async function listProfiles(): Promise<string[]> {
    return await invoke<string[]>("cmd_list_profiles");
  }
  async function saveProfile(name: string) {
    await invoke("cmd_save_profile", { name });
  }
  async function deleteProfile(name: string) {
    await invoke("cmd_delete_profile", { name });
  }
  /** Load a profile, then refresh all in-memory state so the UI reflects it (mirrors the
   * config-import flow). Routing / DNS apply on the next proxy (re)start. */
  async function loadProfile(name: string) {
    await invoke("cmd_load_profile", { name });
    await fetchConfig();
    await Promise.all([fetchSubscriptions(), fetchNodes(), fetchProxyGroups()]);
  }

  async function fetchConfig() {
    config.value = await invoke<AppConfig>("cmd_get_app_config");
  }

  // Apply the "launch on system startup" setting to the OS (registry Run key on Windows).
  // Only toggles when the desired state differs from the actual state to avoid redundant work.
  async function syncAutostart(want: boolean) {
    try {
      const current = await isEnabled();
      if (want && !current) {
        await enable();
      } else if (!want && current) {
        await disable();
      }
    } catch (e) {
      console.error("autostart sync failed:", e);
      error.value = `开机自启动设置失败: ${String(e)}`;
    }
  }

  async function saveConfig(newConfig: AppConfig) {
    await invoke("cmd_save_app_config", { newConfig });
    config.value = newConfig;
    // Apply the autostart setting to the OS — the toggle alone only persists a flag.
    await syncAutostart(newConfig.startup_with_system);
    // Keep tray TUN checkbox in sync whenever config changes
    const sysProxy = await invoke<boolean>("cmd_get_system_proxy_status").catch(() => false);
    syncTrayMenu(sysProxy, newConfig.tun_enabled);
  }

  async function setProxyMode(mode: string) {
    // Optimistic: reflect the new mode in the UI immediately. The backend applies it
    // live via the Clash API (no core restart), so the switch is effectively instant.
    const prev = config.value.proxy_mode;
    config.value.proxy_mode = mode as AppConfig["proxy_mode"];
    try {
      await invoke("cmd_set_proxy_mode", { mode });
      updateTrayTooltip();
    } catch (e) {
      // Roll back on failure so the UI never lies.
      config.value.proxy_mode = prev;
      error.value = String(e);
    }
  }

  function addTrafficPoint(upload: number, download: number) {
    trafficHistory.value.push({ time: Date.now(), upload, download });
    if (trafficHistory.value.length > 60) {
      trafficHistory.value.shift();
    }
  }

  function clearTrafficHistory() {
    trafficHistory.value = [];
  }

  async function init() {
    try {
      await Promise.all([
        fetchStatus(),
        fetchSubscriptions(),
        fetchNodes(),
        fetchConfig(),
      ]);
    } finally {
      // Always leave the loading skeletons even if a startup fetch failed —
      // otherwise the list views would shimmer forever.
      initialized.value = true;
    }
    // Reconcile the autostart toggle with the real OS state so the UI never lies,
    // and re-apply the configured value in case a previous session failed to persist it.
    try {
      const actual = await isEnabled();
      if (actual !== config.value.startup_with_system) {
        await syncAutostart(config.value.startup_with_system);
      }
    } catch (e) {
      console.error("autostart reconcile failed:", e);
    }
    updateTrayTooltip();
    // Start the app-lifetime traffic monitor so cumulative stats are tracked
    // regardless of which page the user is viewing.
    ensureTrafficPoller();
    // React to connection-mode changes driven from the tray menu. The tray mutates
    // the system proxy / TUN core directly in the backend; without these listeners the
    // dashboard's reactive state (especially config.tun_enabled, which no poller
    // refreshes) would stay stale, and tray-side failures would be invisible.
    await listenTrayConnectionEvents();
    // Register global hotkeys if the user has enabled them.
    await applyGlobalShortcuts(config.value.enable_global_shortcuts);
  }

  // Idempotent: register once for the app lifetime. `listen` returns an unlisten fn we
  // intentionally never call — the store lives as long as the app.
  let trayEventsBound = false;
  async function listenTrayConnectionEvents() {
    if (trayEventsBound) return;
    trayEventsBound = true;
    await listen("connection-mode-changed", async () => {
      await fetchConfig();
      await fetchStatus();
      await refreshSystemProxy();
      updateTrayTooltip();
    });
    await listen<string>("connection-mode-error", (event) => {
      error.value = event.payload || "切换连接模式失败";
    });
    // The core exited unexpectedly (crash / killed). The backend has already cleared the
    // now-dead system proxy (fail-open to direct); reconcile the UI so it stops showing a
    // live "connected" state, and surface the failure instead of silently black-holing.
    await listen("singbox-crashed", async () => {
      error.value = "内核意外退出，代理已停止（已恢复直连）。请重新开启代理。";
      await fetchStatus();
      await fetchConfig();
      await refreshSystemProxy();
      updateTrayTooltip();
    });
  }

  return {
    status,
    subscriptions,
    nodes,
    proxyGroups,
    fetchProxyGroups,
    saveProxyGroups,
    config,
    trafficHistory,
    totalUpload,
    totalDownload,
    uploadSpeed,
    downloadSpeed,
    loading,
    initialized,
    error,
    connecting,
    systemProxyEnabled,
    proxying,
    proxySessionStartMs,
    refreshSystemProxy,
    activeNode,
    fetchStatus,
    setConnectionMode,
    fetchSubscriptions,
    addSubscription,
    importSubscriptionFromText,
    setSubscriptionFilters,
    fetchTrafficHistory,
    applyGlobalShortcuts,
    listProfiles,
    saveProfile,
    loadProfile,
    deleteProfile,
    updateSubscription,
    deleteSubscription,
    saveSubscriptionSettings,
    fetchNodes,
    setActiveNode,
    setAutoNode,
    isAutoGroup,
    activeProxyTag,
    activeNodeNow,
    refreshActiveNow,
    ensureActiveNowPoller,
    ensureTrafficPoller,
    testGroupDelay,
    testNodeLatency,
    testNodeSpeed,
    fetchConfig,
    saveConfig,
    setProxyMode,
    addTrafficPoint,
    clearTrafficHistory,
    updateTrayTooltip,
    init,
  };
});
