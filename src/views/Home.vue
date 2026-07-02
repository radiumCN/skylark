<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted, computed } from "vue";
import {
  Wifi, WifiOff, ArrowUp, ArrowDown,
  Filter, Zap, Server, Clock, Globe, Shield, AlertTriangle
} from "@lucide/vue";
import { Line } from "vue-chartjs";
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  Filler,
  Tooltip,
} from "chart.js";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "vue-i18n";
import { useAppStore } from "../stores/app";
import { formatBytes } from "../utils/format";

ChartJS.register(CategoryScale, LinearScale, PointElement, LineElement, Filler, Tooltip);

const { t } = useI18n();
const store = useAppStore();
const systemProxyReady = ref(false);

// System proxy state now lives in the store (refreshed globally). Keep a thin local
// wrapper so existing call sites read through to the store.
const systemProxyEnabled = computed(() => store.systemProxyEnabled);
async function fetchSystemProxy() {
  await store.refreshSystemProxy();
}

// The two mutually-exclusive connection switches. While a switch is being applied
// (store.connecting), reflect the target optimistically so the toggle flips instantly;
// otherwise derive on-state from the actual runtime status so the UI mirrors reality.
const systemProxyOn = computed(() => {
  if (store.connecting === "system") return true;
  if (store.connecting === "tun" || store.connecting === "off") return false;
  return store.status.running && systemProxyEnabled.value && !store.config.tun_enabled;
});
const tunOn = computed(() => {
  if (store.connecting === "tun") return true;
  if (store.connecting === "system" || store.connecting === "off") return false;
  return store.status.running && store.config.tun_enabled;
});

// What the proxy-status card shows as the active routing method. With the persistent
// core, base this on whether we're actually proxying — not on whether the core is up.
const connectionLabel = computed(() => {
  if (tunOn.value) return t("home.tunMode");
  if (systemProxyOn.value) return t("home.systemProxy");
  return t("home.notConnected");
});

// Remember which switch initiated an "off" transition so the sub-label can show
// "断开中…" on the correct row while the core stops.
const wasSystem = ref(false);
const wasTun = ref(false);

async function toggleSystemProxy() {
  const turningOff = systemProxyOn.value;
  wasSystem.value = turningOff;
  wasTun.value = false;
  await store.setConnectionMode(turningOff ? "off" : "system");
  await fetchSystemProxy();
}

async function toggleTun() {
  const turningOff = tunOn.value;
  wasTun.value = turningOff;
  wasSystem.value = false;
  await store.setConnectionMode(turningOff ? "off" : "tun");
  await fetchSystemProxy();
}
// Live speed and cumulative totals are tracked globally by the store's traffic
// monitor (sourced from the Clash API), so they keep accruing regardless of which
// page is open. The dashboard is a pure viewer of those values.
const uploadSpeed = computed(() => store.uploadSpeed);
const downloadSpeed = computed(() => store.downloadSpeed);
const totalUpload = computed(() => store.totalUpload);
const totalDownload = computed(() => store.totalDownload);
const memoryUsage = ref<number | null>(null);
let pollTimer: ReturnType<typeof setInterval> | null = null;

const chartLabels = computed(() =>
  store.trafficHistory.map((_, i) => (i === store.trafficHistory.length - 1 ? "now" : ""))
);

const chartData = computed(() => ({
  labels: chartLabels.value,
  datasets: [
    {
      label: t("home.upload"),
      data: store.trafficHistory.map((p) => p.upload / 1024),
      borderColor: "#4f6ef7",
      backgroundColor: "rgba(79, 110, 247,0.08)",
      borderWidth: 1.5,
      fill: true,
      tension: 0.4,
      pointRadius: 0,
    },
    {
      label: t("home.download"),
      data: store.trafficHistory.map((p) => p.download / 1024),
      borderColor: "#107c10",
      backgroundColor: "rgba(16,124,16,0.08)",
      borderWidth: 1.5,
      fill: true,
      tension: 0.4,
      pointRadius: 0,
    },
  ],
}));

const chartOptions = {
  responsive: true,
  maintainAspectRatio: false,
  animation: { duration: 0 },
  scales: {
    x: { display: false },
    y: {
      display: true,
      grid: { color: "rgba(128,128,128,0.08)" },
      ticks: {
        color: "var(--color-text-muted)",
        font: { size: 10 },
        callback: (v: string | number) => `${Number(v).toFixed(0)} KB/s`,
      },
    },
  },
  plugins: { legend: { display: false }, tooltip: { mode: "index" as const, intersect: false } },
};

// Proxy session timer — tracks how long the proxy has been actively proxying in the
// current session. The session START lives in the store (`proxySessionStartMs`) so it
// survives page navigation; resets only on a real off → on transition and freezes
// (shows "--") when proxying is off. `nowMs` is bumped every second so the display ticks.
// Intentionally decoupled from the backend's process uptime, which counts from core start
// and keeps ticking even in idle mode.
const nowMs = ref(Date.now());

function formatUptime(sec: number) {
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = sec % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

const displayUptime = computed(() => {
  const start = store.proxySessionStartMs;
  if (!store.proxying || start === null) return "--";
  return formatUptime(Math.max(0, Math.floor((nowMs.value - start) / 1000)));
});

const proxyModeLabel = computed(() => {
  const map: Record<string, string> = {
    rule: t("home.ruleMode"),
    global: t("home.globalMode"),
    direct: t("home.directMode"),
    tun: t("home.tunMode"),
  };
  return map[store.config.proxy_mode] ?? store.config.proxy_mode;
});

onMounted(async () => {
  // Fetch the real state first, then enable transitions to avoid the initial "flash" animation.
  await fetchSystemProxy();
  await nextTick();
  systemProxyReady.value = true;

  // Shared pollers (status + active node + traffic totals) run at app scope. The traffic
  // poller also owns the proxy-session start time, so the uptime is correct on mount.
  store.ensureActiveNowPoller();
  store.ensureTrafficPoller();

  let lastProxying = store.proxying;

  pollTimer = setInterval(async () => {
    // Always sync system proxy — avoids the timing race where the watcher fired
    // before the backend finished setting the proxy on auto-restore startup.
    fetchSystemProxy();
    // Drive the uptime display; the session start itself lives in the store.
    nowMs.value = Date.now();

    // React to proxy connect/disconnect transitions (tray tooltip only — the session
    // timer is maintained by the store's traffic poller).
    if (store.proxying !== lastProxying) {
      lastProxying = store.proxying;
      store.updateTrayTooltip();
    }

    memoryUsage.value = store.status.running
      ? await invoke<number | null>("cmd_get_memory_usage").catch(() => null)
      : null;
  }, 1000);
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
});
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">{{ t('home.dashboard') }}</h1>
    </div>

    <!-- Error Banner (top, prominent) -->
    <div v-if="store.error" class="error-banner">
      <span class="error-msg"><AlertTriangle :size="15" /> {{ store.error }}</span>
      <button class="btn btn-ghost" @click="store.error = null">{{ t('home.close') }}</button>
    </div>

    <!-- Connection Hero — promoted status, driven by the switches below -->
    <div class="card hero-card" :class="{ active: store.proxying }">
      <div class="hero-icon" :class="store.proxying ? 'on' : 'off'">
        <component :is="store.proxying ? Wifi : WifiOff" :size="26" />
      </div>
      <div class="hero-main">
        <div class="hero-label">{{ t('home.proxyStatus') }}</div>
        <div class="hero-status">{{ connectionLabel }}</div>
      </div>
      <div v-if="store.proxying" class="hero-meta">
        <span class="hero-pill">{{ proxyModeLabel }}</span>
      </div>
    </div>

    <!-- Network Settings -->
    <div class="card net-settings-card">
      <div class="net-settings-title">
        <Globe :size="14" />
        {{ t('home.networkSettings') }}
      </div>
      <div class="net-settings-body">
        <!-- System Proxy toggle — starts/stops the proxy; mutually exclusive with TUN -->
        <div class="net-row">
          <div class="net-row-left">
            <div class="net-row-icon icon-blue"><Globe :size="15" /></div>
            <div>
              <div class="net-row-label">{{ t('home.systemProxy') }}</div>
              <div class="net-row-sub">
                <template v-if="store.connecting === 'system'">{{ t('home.connecting') }}</template>
                <template v-else-if="store.connecting === 'off' && wasSystem">{{ t('home.disconnecting') }}</template>
                <template v-else-if="systemProxyOn">{{ `127.0.0.1:${store.config.mixed_port}` }}</template>
                <template v-else>{{ t('home.systemProxyHint') }}</template>
              </div>
            </div>
          </div>
          <button
            class="toggle-btn"
            :class="{ on: systemProxyOn, 'no-anim': !systemProxyReady }"
            :disabled="store.loading"
            @click="toggleSystemProxy"
            :title="systemProxyOn ? t('home.systemProxyToggleOff') : t('home.systemProxyToggleOn')"
          >
            <span class="toggle-knob" />
          </button>
        </div>

        <div class="net-divider" />

        <!-- Proxy mode selector -->
        <div class="net-row">
          <div class="net-row-left">
            <div class="net-row-icon icon-violet"><Filter :size="15" /></div>
            <div>
              <div class="net-row-label">{{ t('home.proxyMode') }}</div>
              <div class="net-row-sub">{{ proxyModeLabel }}</div>
            </div>
          </div>
          <div class="mode-pills">
            <button
              v-for="[k, labelKey] in [['rule','rule'],['global','global'],['direct','direct']]"
              :key="k"
              class="mode-pill"
              :class="{ active: store.config.proxy_mode === k }"
              @click="store.setProxyMode(k)"
            >{{ t('home.' + labelKey) }}</button>
          </div>
        </div>

        <div class="net-divider" />

        <!-- TUN Mode toggle — starts/stops the proxy; mutually exclusive with system proxy -->
        <div class="net-row">
          <div class="net-row-left">
            <div class="net-row-icon icon-teal"><Shield :size="15" /></div>
            <div>
              <div class="net-row-label">{{ t('home.tunMode') }}</div>
              <div class="net-row-sub">
                <template v-if="store.connecting === 'tun'">{{ t('home.connecting') }}</template>
                <template v-else-if="store.connecting === 'off' && wasTun">{{ t('home.disconnecting') }}</template>
                <template v-else>{{ tunOn ? t('home.tunOnHint') : t('home.tunOffHint') }}</template>
              </div>
            </div>
          </div>
          <button
            class="toggle-btn"
            :class="{ on: tunOn }"
            :disabled="store.loading"
            @click="toggleTun"
          >
            <span class="toggle-knob" />
          </button>
        </div>
      </div>
    </div>

    <!-- Detail stats -->
    <div class="stat-grid">
      <!-- Active Node -->
      <div class="card stat-card">
        <div class="stat-icon icon-node">
          <Server :size="18" />
        </div>
        <div class="stat-content">
          <div class="stat-label">{{ t('home.currentNode') }}</div>
          <div class="stat-value text-sm">
            {{ store.isAutoGroup ? t('home.autoSelect') : (store.activeNode?.name ?? t('home.noneSelected')) }}
          </div>
          <div class="stat-sub">
            <template v-if="store.isAutoGroup">
              {{ store.activeNodeNow ? `→ ${store.activeNodeNow}` : t('home.dynamicSelecting') }}
            </template>
            <template v-else>{{ store.activeNode?.server ?? "--" }}</template>
          </div>
        </div>
      </div>

      <!-- Uptime -->
      <div class="card stat-card">
        <div class="stat-icon icon-uptime">
          <Clock :size="18" />
        </div>
        <div class="stat-content">
          <div class="stat-label">{{ t('home.uptime') }}</div>
          <div class="stat-value">{{ displayUptime }}</div>
          <div class="stat-sub">
            <template v-if="memoryUsage !== null">
              {{ t('home.memory', { value: formatBytes(memoryUsage) }) }} · {{ store.status.version ?? "sing-box" }}
            </template>
            <template v-else>
              {{ store.status.version ?? "sing-box" }}
            </template>
          </div>
        </div>
      </div>
    </div>

    <!-- Traffic Stats -->
    <div class="traffic-row">
      <div class="card traffic-stat upload">
        <ArrowUp :size="16" />
        <span class="traffic-label">{{ t('home.uploadSpeed') }}</span>
        <span class="traffic-value">{{ formatBytes(uploadSpeed) }}/s</span>
        <span class="traffic-total">{{ t('home.totalSinceStart', { value: formatBytes(totalUpload) }) }}</span>
      </div>
      <div class="card traffic-stat download">
        <ArrowDown :size="16" />
        <span class="traffic-label">{{ t('home.downloadSpeed') }}</span>
        <span class="traffic-value">{{ formatBytes(downloadSpeed) }}/s</span>
        <span class="traffic-total">{{ t('home.totalSinceStart', { value: formatBytes(totalDownload) }) }}</span>
      </div>
    </div>

    <!-- Traffic Chart -->
    <div class="card chart-card">
      <div class="chart-header">
        <Zap :size="15" />
        <span>{{ t('home.realtimeTraffic') }}</span>
        <div class="chart-legend">
          <span class="legend-item upload-color"><i class="legend-dot" /> {{ t('home.upload') }}</span>
          <span class="legend-item download-color"><i class="legend-dot" /> {{ t('home.download') }}</span>
        </div>
      </div>
      <div class="chart-body">
        <Line v-if="store.trafficHistory.length > 1" :data="chartData" :options="chartOptions" />
        <div v-else class="chart-empty">{{ t('home.chartEmpty') }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; gap: 16px; max-width: 900px; }
.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.page-title {
  font-size: 20px;
  font-weight: 600;
  color: var(--color-text);
}
@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

/* Connection Hero */
.hero-card {
  padding: 18px 20px;
  display: flex;
  align-items: center;
  gap: 16px;
  position: relative;
  overflow: hidden;
  transition: border-color 0.25s ease;
}
.hero-card.active {
  border-color: rgba(16, 124, 16, 0.28);
  background: var(--color-surface);
}
.hero-icon {
  width: 52px; height: 52px;
  border-radius: var(--radius-lg);
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0;
  position: relative;
  transition: all 0.25s ease;
}
.hero-icon.off { background: var(--color-neutral); color: var(--color-text-muted); }
.hero-icon.on {
  background: var(--color-success);
  color: white;
  box-shadow: 0 4px 14px rgba(16, 124, 16, 0.35);
  animation: hero-pop 0.42s cubic-bezier(0.34, 1.56, 0.64, 1);
}
/* Soft breathing ring while connected */
.hero-icon.on::after {
  content: "";
  position: absolute;
  inset: -6px;
  border-radius: inherit;
  border: 2px solid rgba(16, 124, 16, 0.35);
  animation: hero-ring 2.6s ease-out infinite;
}
@keyframes hero-pop {
  0% { transform: scale(0.82); }
  60% { transform: scale(1.07); }
  100% { transform: scale(1); }
}
@keyframes hero-ring {
  0% { opacity: 0.6; transform: scale(0.92); }
  70%, 100% { opacity: 0; transform: scale(1.15); }
}
.hero-main { flex: 1; min-width: 0; }
.hero-label {
  font-size: 11px; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.6px; margin-bottom: 3px;
}
.hero-status { font-size: 20px; font-weight: 700; color: var(--color-text); }
.hero-meta { flex-shrink: 0; }
.hero-pill {
  font-size: 12px; font-weight: 600;
  padding: 4px 12px; border-radius: 100px;
  background: var(--color-primary-soft); color: var(--color-primary);
}

.stat-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 12px;
}
.stat-card { padding: 16px; display: flex; align-items: flex-start; gap: 12px; }
.stat-icon {
  width: 40px; height: 40px; border-radius: var(--radius-lg);
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0;
}
.stat-content { flex: 1; min-width: 0; }
.stat-label { font-size: 11px; color: var(--color-text-secondary); text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 4px; }
.stat-value { font-size: 15px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.text-sm { font-size: 13px !important; }
.stat-sub { font-size: 11px; color: var(--color-text-muted); margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

/* Harmonized icon palette — indigo-led cool set + green for status */
.icon-green { background: var(--color-success-soft); color: var(--color-success); }
.icon-blue,
.icon-node { background: var(--color-primary-soft); color: var(--color-primary); }
.icon-violet { background: rgba(124, 92, 236, 0.13); color: var(--accent-violet); }
.icon-teal { background: rgba(14, 155, 142, 0.14); color: var(--accent-teal); }
.icon-amber,
.icon-uptime { background: rgba(193, 128, 30, 0.14); color: var(--accent-amber); }

.traffic-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
.traffic-stat {
  padding: 14px 16px;
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.traffic-stat.upload { color: var(--color-primary); }
.traffic-stat.download { color: var(--color-success); }
.traffic-label { font-size: 12px; color: var(--color-text-secondary); }
.traffic-value { font-size: 16px; font-weight: 700; margin-left: auto; }
.traffic-total { font-size: 11px; color: var(--color-text-muted); width: 100%; text-align: right; }

.chart-card { padding: 16px; }
.chart-header {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
  margin-bottom: 12px;
}
.chart-legend { margin-left: auto; display: flex; gap: 12px; }
.legend-item { display: inline-flex; align-items: center; gap: 5px; font-size: 11px; font-weight: 500; }
.legend-dot { width: 8px; height: 8px; border-radius: 50%; background: currentColor; display: inline-block; }
.upload-color { color: var(--color-primary); }
.download-color { color: var(--color-success); }
.chart-body { height: 160px; }
.chart-empty {
  height: 100%; display: flex; align-items: center; justify-content: center;
  color: var(--color-text-muted); font-size: 13px;
}

/* Network Settings Card */
.net-settings-card { padding: 14px 16px; }
.net-settings-title {
  display: flex; align-items: center; gap: 6px;
  font-size: 12px; font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.5px;
  margin-bottom: 10px;
}
.net-settings-body { display: flex; flex-direction: column; }
.net-row {
  display: flex; align-items: center; justify-content: space-between;
  padding: 8px 0; gap: 12px;
}
.net-row-left { display: flex; align-items: center; gap: 10px; }
.net-row-icon {
  width: 32px; height: 32px; border-radius: var(--radius-md);
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0;
}
.net-row-label { font-size: 13px; font-weight: 500; }
.net-row-sub { font-size: 11px; color: var(--color-text-muted); margin-top: 1px; }
.net-divider { height: 1px; background: var(--color-border); margin: 2px 0; }

/* Toggle switch */
.toggle-btn {
  width: 42px; height: 24px; border-radius: 100px;
  background: var(--color-border); border: none; cursor: pointer;
  position: relative; transition: background 0.2s; flex-shrink: 0;
  padding: 0;
}
.toggle-btn.on {
  background: var(--color-primary);
}
.toggle-btn:focus-visible {
  outline: 2px solid var(--color-primary);
  outline-offset: 2px;
}
.toggle-knob {
  position: absolute; top: 3px; left: 3px;
  width: 18px; height: 18px; border-radius: 50%;
  background: white; transition: transform 0.2s;
  display: block;
}
.toggle-btn.on .toggle-knob { transform: translateX(18px); }
.toggle-btn.no-anim,
.toggle-btn.no-anim .toggle-knob { transition: none; }
.toggle-btn.toggle-disabled {
  opacity: 0.35; cursor: not-allowed;
  background: var(--color-border) !important;
}
.row-dimmed { opacity: 0.6; }
.row-dimmed .net-row-sub { color: var(--color-text-muted); font-style: italic; }

/* Mode pills */
.mode-pills { display: flex; gap: 4px; }
.mode-pill {
  padding: 3px 10px; border-radius: var(--radius-sm);
  border: 1px solid var(--color-border);
  background: transparent; color: var(--color-text-secondary);
  font-size: 11px; cursor: pointer; transition: all 0.15s;
}
.mode-pill:hover { background: var(--color-neutral); }
.mode-pill.active { background: var(--color-primary); color: white; border-color: transparent; }

.error-banner {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 16px;
  background: rgba(209,52,56,0.08);
  border: 1px solid rgba(209,52,56,0.2);
  border-radius: var(--radius-md);
  color: var(--color-error);
  font-size: 13px;
}
.error-msg { display: inline-flex; align-items: center; gap: 7px; }
</style>
