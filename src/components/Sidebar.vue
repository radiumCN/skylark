<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  Home,
  Rss,
  Server,
  Activity,
  BarChart3,
  ScrollText,
  Filter,
  Settings,
  ArrowUp,
  ArrowDown,
} from "@lucide/vue";
import { listen } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";
import { useAppStore } from "../stores/app";
import { formatBytes } from "../utils/format";
import logoUrl from "../assets/logo.png";

const route = useRoute();
const router = useRouter();
const store = useAppStore();
const { t } = useI18n();

const hasUpdate = ref(false);
const updateVersion = ref("");

const navItems = [
  { path: "/home", icon: Home, key: "home" },
  { path: "/subscriptions", icon: Rss, key: "subscriptions" },
  { path: "/nodes", icon: Server, key: "nodes" },
  { path: "/connections", icon: Activity, key: "connections" },
  { path: "/stats", icon: BarChart3, key: "stats" },
  { path: "/logs", icon: ScrollText, key: "logs" },
  { path: "/rules", icon: Filter, key: "rules" },
];

const isActive = (path: string) => route.path === path;

onMounted(() => {
  listen<{ version: string }>("singbox-update-available", (e) => {
    hasUpdate.value = true;
    updateVersion.value = e.payload.version;
  });
  listen<{ success: boolean }>("singbox-download-done", (e) => {
    if (e.payload.success) {
      hasUpdate.value = false;
      updateVersion.value = "";
    }
  });
});
</script>

<template>
  <nav class="sidebar">
    <div class="brand">
      <img class="brand-mark" :src="logoUrl" alt="Skylark" />
      <div class="brand-text">
        <span class="brand-name">Skylark</span>
      </div>
    </div>

    <div class="sidebar-nav">
      <button
        v-for="item in navItems"
        :key="item.path"
        class="nav-item"
        :class="{ active: isActive(item.path) }"
        @click="router.push(item.path)"
      >
        <component :is="item.icon" :size="18" />
        <span>{{ t('nav.' + item.key) }}</span>
      </button>
    </div>

    <div class="sidebar-footer">
      <!-- Global proxy status (read-only; controlled from the dashboard switches) -->
      <div
        class="proxy-status"
        :class="store.proxying ? 'running' : 'stopped'"
        :title="store.proxying ? t('sidebar.connectedTitle') : t('sidebar.disconnectedTitle')"
      >
        <span class="status-dot" :class="{ running: store.proxying }" />
        <span>{{ store.proxying ? t('sidebar.connected') : t('sidebar.disconnected') }}</span>
      </div>

      <!-- Live traffic — app-wide poller keeps these fresh on every page. Shown
           only while proxying; reveal is animated to avoid a layout jump. -->
      <Transition name="traffic">
        <div v-if="store.proxying" class="sidebar-traffic">
          <div class="traffic-cell up" :title="t('home.uploadSpeed')">
            <ArrowUp :size="12" />
            <span class="traffic-val">{{ formatBytes(store.uploadSpeed) }}/s</span>
          </div>
          <div class="traffic-cell down" :title="t('home.downloadSpeed')">
            <ArrowDown :size="12" />
            <span class="traffic-val">{{ formatBytes(store.downloadSpeed) }}/s</span>
          </div>
        </div>
      </Transition>

      <button
        class="nav-item"
        :class="{ active: isActive('/settings') }"
        :title="hasUpdate ? t('sidebar.updateTitle', { version: updateVersion }) : ''"
        @click="router.push('/settings')"
      >
        <div class="icon-wrap">
          <Settings :size="18" />
          <span v-if="hasUpdate" class="update-dot" />
        </div>
        <span>{{ t('nav.settings') }}</span>
        <span v-if="hasUpdate" class="update-badge">{{ t('sidebar.update') }}</span>
      </button>
    </div>
  </nav>
</template>

<style scoped>
.sidebar {
  position: relative;
  z-index: 1;
  width: var(--sidebar-width);
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  backdrop-filter: blur(24px) saturate(180%);
  -webkit-backdrop-filter: blur(24px) saturate(180%);
  box-shadow: var(--edge-highlight);
  flex-shrink: 0;
  padding: 8px 8px 12px;
}
.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 10px 14px;
  margin-bottom: 4px;
}
.brand-mark {
  width: 32px; height: 32px;
  border-radius: var(--radius-md);
  object-fit: cover;
  flex-shrink: 0;
  box-shadow: 0 2px 10px var(--color-primary-glow);
}
.brand-text { display: flex; align-items: center; }
.brand-name { font-size: 15px; font-weight: 700; color: var(--color-text); letter-spacing: 0.2px; }
.sidebar-nav {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border-radius: var(--radius-md);
  border: none;
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  text-align: left;
  width: 100%;
  transition: background 0.15s ease, color 0.15s ease;
}
.nav-item:hover {
  background: var(--color-neutral);
  color: var(--color-text);
}
.nav-item.active {
  background: var(--color-primary-soft);
  color: var(--color-primary);
  font-weight: 600;
}
/* Left accent rail on the active item — the signature "you are here" indicator. */
.nav-item.active::before {
  content: "";
  position: absolute;
  left: -8px;
  top: 50%;
  transform: translateY(-50%);
  width: 3px;
  height: 18px;
  border-radius: 0 3px 3px 0;
  background: var(--color-primary);
  box-shadow: 0 0 8px var(--color-primary-glow);
}
.sidebar-footer {
  display: flex;
  flex-direction: column;
  gap: 2px;
  border-top: 1px solid var(--color-border);
  padding-top: 8px;
}
.proxy-status {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-radius: var(--radius-md);
  color: var(--color-text-secondary);
  font-size: 13px;
  font-weight: 500;
  width: 100%;
}
.proxy-status.running { color: var(--color-success); }

/* Live traffic readout (up/down speed). Tabular figures so the digits don't
   jitter as the numbers tick; direction is carried by the arrow icon, not
   colour alone. Upload=primary / download=success, matching the dashboard. */
.sidebar-traffic {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-1);
  padding: 6px 10px;
  margin-bottom: 2px;
  border-radius: var(--radius-md);
  background: var(--color-neutral-soft);
  font-variant-numeric: tabular-nums;
}
.traffic-cell {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  font-size: 11px;
  font-weight: 600;
}
.traffic-cell.up { color: var(--color-primary); }
/* --color-success-text lifts the emerald on dark surfaces (theme-split in main.css). */
.traffic-cell.down { color: var(--color-success-text); }
.traffic-cell svg { flex-shrink: 0; }
.traffic-val { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.traffic-enter-active, .traffic-leave-active {
  transition: opacity 0.2s ease, transform 0.2s ease;
}
.traffic-enter-from, .traffic-leave-to {
  opacity: 0;
  transform: translateY(4px);
}
.status-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--color-text-muted);
  transition: background 0.3s;
}
.status-dot.running {
  background: var(--color-success);
  animation: status-pulse 2s infinite;
}
@keyframes status-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}
.icon-wrap { position: relative; display: flex; align-items: center; }
.update-dot {
  position: absolute; top: -3px; right: -3px;
  width: 7px; height: 7px; border-radius: 50%;
  background: var(--color-error);
  border: 1.5px solid var(--color-surface);
  animation: pulse-dot 2s infinite;
}
.update-badge {
  margin-left: auto;
  font-size: var(--fs-2xs); font-weight: 700;
  padding: 1px 6px; border-radius: 100px;
  background: var(--color-error-soft); color: var(--color-error);
}
@keyframes pulse-dot {
  0%, 100% { transform: scale(1); }
  50% { transform: scale(1.3); }
}
</style>
