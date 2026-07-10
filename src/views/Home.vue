<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted, computed } from "vue";
import {
  Wifi, WifiOff, ArrowUp, ArrowDown,
  Filter, Server, Clock, Globe, Shield, AlertTriangle
} from "@lucide/vue";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "vue-i18n";
import { useAppStore } from "../stores/app";
import { formatBytes } from "../utils/format";
import ToggleSwitch from "../components/ToggleSwitch.vue";
import StatTile from "../components/StatTile.vue";

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
  // Reflect the real runtime state: system proxy on AND the core is not actually in TUN.
  // (tun_active is the backend truth; config.tun_enabled is only a persisted preference.)
  return store.status.running && systemProxyEnabled.value && store.status.tun_active !== true;
});
const tunOn = computed(() => {
  if (store.connecting === "tun") return true;
  if (store.connecting === "system" || store.connecting === "off") return false;
  // Backend truth — the core is actually running in TUN mode — not the persisted preference,
  // which would otherwise light this up even when no tunnel was ever established.
  return store.status.running && store.status.tun_active === true;
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
// Cumulative totals since the proxy session start. Live up/down speed now lives
// in the sidebar, so the dashboard only surfaces the running totals here.
const totalUpload = computed(() => store.totalUpload);
const totalDownload = computed(() => store.totalDownload);
const memoryUsage = ref<number | null>(null);
let pollTimer: ReturnType<typeof setInterval> | null = null;

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

// True once onUnmounted has run. The interval below is created AFTER awaits in onMounted,
// so a fast navigate-away would otherwise see a null pollTimer during cleanup and the
// later-created interval would tick (two IPCs per second) forever.
let disposed = false;
onMounted(async () => {
  // Fetch the real state first, then enable transitions to avoid the initial "flash" animation.
  await fetchSystemProxy();
  await nextTick();
  if (disposed) return;
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
  disposed = true;
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

    <!-- Control Center — status hero + connection controls in one glass panel -->
    <div class="card control-center" :class="{ active: store.proxying }">
      <!-- Status hero -->
      <div class="cc-hero">
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

      <div class="cc-divider" />

      <!-- Connection controls -->
      <div class="cc-body">
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
          <ToggleSwitch
            :model-value="systemProxyOn"
            :disabled="store.loading"
            :aria-label="systemProxyOn ? t('home.systemProxyToggleOff') : t('home.systemProxyToggleOn')"
            @update:model-value="toggleSystemProxy"
          />
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
          <div class="segmented">
            <button
              v-for="[k, labelKey] in [['rule','rule'],['global','global'],['direct','direct']]"
              :key="k"
              class="segmented__item"
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
          <ToggleSwitch
            :model-value="tunOn"
            :disabled="store.loading"
            :aria-label="t('home.tunMode')"
            @update:model-value="toggleTun"
          />
        </div>
      </div>
    </div>

    <!-- Key status tiles. Live up/down speed now lives in the sidebar; the
         dashboard shows the active node, session uptime, and running totals. -->
    <div class="stat-grid">
      <StatTile
        :icon="Server"
        :label="t('home.currentNode')"
        :value="store.isAutoGroup ? t('home.autoSelect') : (store.activeNode?.name ?? t('home.noneSelected'))"
        :sub="store.isAutoGroup
          ? (store.activeNodeNow ? `→ ${store.activeNodeNow}` : t('home.dynamicSelecting'))
          : (store.activeNode?.server ?? '--')"
        accent="primary"
        compact
      />
      <StatTile
        :icon="Clock"
        :label="t('home.uptime')"
        :value="displayUptime"
        :sub="memoryUsage !== null
          ? `${t('home.memory', { value: formatBytes(memoryUsage) })} · ${store.status.version ?? 'sing-box'}`
          : (store.status.version ?? 'sing-box')"
        accent="amber"
      />
      <StatTile
        :icon="ArrowUp"
        :label="t('home.totalUpload')"
        :value="formatBytes(totalUpload)"
        accent="primary"
      />
      <StatTile
        :icon="ArrowDown"
        :label="t('home.totalDownload')"
        :value="formatBytes(totalDownload)"
        accent="success"
      />
    </div>
  </div>
</template>

<style scoped>
/* Control Center — status hero + connection controls fused into one glass panel */
.control-center {
  padding: var(--space-4) var(--space-5);
  box-shadow: var(--shadow-md), var(--edge-highlight);
  transition: border-color 0.25s ease, box-shadow 0.25s ease;
}
.control-center.active {
  border-color: var(--color-success-glow);
  box-shadow: var(--shadow-md), var(--edge-highlight), 0 0 0 1px var(--color-success-glow);
}
.cc-hero {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  position: relative;
}
.cc-divider {
  height: 1px;
  background: var(--color-border);
  margin: var(--space-4) 0 var(--space-2);
}
.cc-body { display: flex; flex-direction: column; }
.hero-icon {
  width: 52px; height: 52px;
  border-radius: var(--radius-lg);
  display: flex; align-items: center; justify-content: center;
  flex-shrink: 0;
  position: relative;
  /* Not `all`: hero-pop drives transform, and a transition on the same property
     fights the keyframes as the animation settles. */
  transition: background 0.25s ease, color 0.25s ease, box-shadow 0.25s ease;
}
.hero-icon.off { background: var(--color-neutral); color: var(--color-text-muted); }
.hero-icon.on {
  background: var(--color-success);
  color: white;
  box-shadow: 0 4px 14px var(--color-success-glow);
  animation: hero-pop 0.42s cubic-bezier(0.34, 1.56, 0.64, 1);
}
/* Soft breathing ring while connected. The ring box sits 6px outside the icon,
   so its radius must be the icon's + 6px to stay concentric — `inherit` would
   reuse the icon's radius on a larger box and pinch the corners.
   Solid success + keyframe opacity: --color-success-glow is already 40% alpha,
   and multiplying that by the keyframe opacity left the ring near-invisible on
   the light theme's #f2f3f9 background. */
.hero-icon.on::after {
  content: "";
  position: absolute;
  inset: -6px;
  border-radius: calc(var(--radius-lg) + 6px);
  border: 2px solid var(--color-success);
  animation: hero-ring 2.6s ease-out infinite;
}
@keyframes hero-pop {
  0% { transform: scale(0.82); }
  60% { transform: scale(1.07); }
  100% { transform: scale(1); }
}
/* Starts at rest (concentric, hugging the icon) and expands outward; the tail
   holds at opacity 0 so each pulse is followed by a beat of stillness. */
@keyframes hero-ring {
  0% { opacity: 0.45; transform: scale(1); }
  70%, 100% { opacity: 0; transform: scale(1.18); }
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

/* Status tiles — two per row (2×2). Four across is too cramped and overflows
   the content column in a narrow window. */
.stat-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: var(--space-3);
}

/* Icon tints used by the control-center rows */
.icon-blue { background: var(--color-primary-soft); color: var(--color-primary); }
.icon-violet { background: rgba(124, 92, 236, 0.13); color: var(--accent-violet); }
.icon-teal { background: rgba(14, 155, 142, 0.14); color: var(--accent-teal); }

/* Connection control rows (inside the Control Center panel) */
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

.error-banner {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 16px;
  background: var(--color-error-soft);
  border: 1px solid var(--color-error-soft);
  border-radius: var(--radius-md);
  color: var(--color-error);
  font-size: 13px;
}
.error-msg { display: inline-flex; align-items: center; gap: 7px; }
</style>
