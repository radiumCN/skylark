<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick, watch, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Trash2, ArrowDown, Copy, Download } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useTemporaryFlag } from "../composables/useTemporaryFlag";
import { copyToClipboard } from "../utils/clipboard";

const { t } = useI18n();

const LOG_CAP = 1000;
const logs = ref<string[]>([]);
const autoScroll = ref(true);
const filterLevel = ref("all");
const logContainer = ref<HTMLElement | null>(null);
let unlistenLog: UnlistenFn | null = null;

const levelColors: Record<string, string> = {
  error: "var(--color-error)",
  warn: "var(--color-attention)",
  info: "var(--color-primary)",
  debug: "var(--color-text-secondary)",
};

const parsedLogs = computed(() =>
  logs.value.map((line) => {
    const lower = line.toLowerCase();
    let level = "info";
    if (lower.includes(" error") || lower.includes("[error]")) level = "error";
    else if (lower.includes(" warn") || lower.includes("[warn]")) level = "warn";
    else if (lower.includes(" debug") || lower.includes("[debug]")) level = "debug";
    return { raw: line, level };
  })
);

const filtered = computed(() => {
  if (filterLevel.value === "all") return parsedLogs.value;
  return parsedLogs.value.filter((l) => l.level === filterLevel.value);
});

async function fetchLogs() {
  try {
    logs.value = await invoke<string[]>("cmd_get_logs");
  } catch {
    // ignore
  }
}

const { flag: copySuccess, trigger: triggerCopySuccess } = useTemporaryFlag(1500);

async function scrollToBottom() {
  await nextTick();
  if (autoScroll.value && logContainer.value) {
    logContainer.value.scrollTop = logContainer.value.scrollHeight;
  }
}

async function copyAllLogs() {
  const text = logs.value.join("\n");
  const ok = await copyToClipboard(text);
  if (ok) triggerCopySuccess();
}

const exporting = ref(false);
async function exportLogs() {
  exporting.value = true;
  try {
    const path = await invoke<string>("cmd_export_logs");
    try {
      await revealItemInDir(path);
    } catch {
      // Reveal may be unavailable; the file is still written.
    }
    alert(t("logs.exportSuccess", { path }));
  } catch (e) {
    alert(t("logs.exportFailed", { error: e }));
  } finally {
    exporting.value = false;
  }
}

watch(filtered, scrollToBottom);

onMounted(async () => {
  // Load the current buffer once, then receive new lines incrementally via events
  // instead of re-cloning the whole buffer on a timer.
  await fetchLogs();
  unlistenLog = await listen<string>("singbox-log", (e) => {
    logs.value.push(e.payload);
    if (logs.value.length > LOG_CAP) {
      logs.value.splice(0, logs.value.length - LOG_CAP);
    }
  });
});
onUnmounted(() => {
  if (unlistenLog) unlistenLog();
});
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">{{ t('logs.title') }}</h1>
      <div class="header-actions">
        <div class="level-tabs">
          <button
            v-for="level in ['all', 'error', 'warn', 'info', 'debug']"
            :key="level"
            class="level-tab"
            :class="{ active: filterLevel === level }"
            @click="filterLevel = level"
          >
            {{ level === 'all' ? t('logs.all') : level.toUpperCase() }}
          </button>
        </div>
        <button class="btn btn-ghost" :title="t('logs.autoScroll')" @click="autoScroll = !autoScroll">
          <ArrowDown :size="14" :style="{ opacity: autoScroll ? 1 : 0.4 }" />
          {{ t('logs.autoScroll') }}
        </button>
        <button class="btn btn-ghost" @click="copyAllLogs" :title="copySuccess ? t('logs.copied') + '!' : t('logs.copyAll')">
          <Copy :size="14" :style="{ color: copySuccess ? 'var(--color-success)' : undefined }" />
          {{ copySuccess ? t('logs.copied') : t('logs.copy') }}
        </button>
        <button class="btn btn-ghost" @click="exportLogs" :disabled="exporting" :title="t('logs.exportToFile')">
          <Download :size="14" />
          {{ exporting ? t('logs.exporting') : t('logs.export') }}
        </button>
        <button class="btn btn-ghost" @click="logs = []" :title="t('logs.clear')">
          <Trash2 :size="14" />
        </button>
      </div>
    </div>

    <div class="log-container card" ref="logContainer">
      <div v-if="filtered.length === 0" class="log-empty">
        {{ t('logs.empty') }}
      </div>
      <div
        v-for="(log, i) in filtered"
        :key="i"
        class="log-line"
        :style="{ color: levelColors[log.level] ?? 'inherit' }"
      >
        {{ log.raw }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.page { display: flex; flex-direction: column; gap: 14px; height: calc(100vh - 40px - 48px); }
.page-header { display: flex; align-items: center; justify-content: space-between; flex-shrink: 0; }
.page-title { font-size: 20px; font-weight: 600; }
.header-actions { display: flex; align-items: center; gap: 8px; }

.level-tabs { display: flex; gap: 4px; }
.level-tab {
  padding: 3px 10px; border-radius: var(--radius-sm);
  border: 1px solid var(--color-border);
  background: transparent; color: var(--color-text-secondary);
  font-size: 11px; font-weight: 500; cursor: pointer; transition: all 0.15s;
}
.level-tab:hover { background: var(--color-neutral); }
.level-tab.active { background: var(--color-primary); color: white; border-color: transparent; }

.log-container {
  flex: 1;
  overflow-y: auto;
  padding: 12px 16px;
  background: #1a1a1a;
  border-radius: var(--radius-lg);
  font-family: 'Cascadia Code', 'Consolas', 'Courier New', monospace;
  font-size: 12px;
  line-height: 1.6;
}
.log-line {
  padding: 1px 0;
  white-space: pre-wrap;
  word-break: break-all;
}
.log-line:hover { background: rgba(255,255,255,0.04); }
.log-empty {
  color: var(--color-text-secondary);
  text-align: center;
  padding: 48px;
  font-family: 'Segoe UI', sans-serif;
}
</style>
