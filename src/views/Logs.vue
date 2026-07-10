<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick, watch, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Trash2, ArrowDown, Copy, Download, ScrollText } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useTemporaryFlag } from "../composables/useTemporaryFlag";
import { copyToClipboard } from "../utils/clipboard";
import EmptyState from "../components/EmptyState.vue";
import { useFeedbackStore } from "../stores/feedback";

const { t } = useI18n();
const fb = useFeedbackStore();

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
    // The file is already written at this point. Revealing it in the OS file
    // manager (Explorer/Finder) spawns a native window, which can take a second
    // or two — don't `await` it, or the export appears to hang for that whole
    // time. Fire it in the background so the success feedback shows immediately.
    revealItemInDir(path).catch(() => {
      // Reveal may be unavailable; the file is still written.
    });
    fb.toastSuccess(t("logs.exportSuccess", { path }));
  } catch (e) {
    fb.toastError(t("logs.exportFailed", { error: e }));
  } finally {
    exporting.value = false;
  }
}

watch(filtered, scrollToBottom);

// True once onUnmounted has run. The listen() below resolves AFTER awaits in onMounted,
// so a fast navigate-away would otherwise register the listener after cleanup already
// looked at a null handle — leaking it (plus this component's log buffer) forever.
let disposed = false;
onMounted(async () => {
  // Load the current buffer once, then receive new lines incrementally via events
  // instead of re-cloning the whole buffer on a timer.
  await fetchLogs();
  const un = await listen<string>("singbox-log", (e) => {
    logs.value.push(e.payload);
    if (logs.value.length > LOG_CAP) {
      logs.value.splice(0, logs.value.length - LOG_CAP);
    }
  });
  if (disposed) {
    un();
    return;
  }
  unlistenLog = un;
});
onUnmounted(() => {
  disposed = true;
  if (unlistenLog) unlistenLog();
});
</script>

<template>
  <div class="page page--wide page--fill">
    <div class="page-header">
      <h1 class="page-title">{{ t('logs.title') }}</h1>
      <div class="header-actions">
        <div class="segmented">
          <button
            v-for="level in ['all', 'error', 'warn', 'info', 'debug']"
            :key="level"
            class="segmented__item"
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
        <button class="btn btn-ghost clear-btn" @click="logs = []" :title="t('logs.clear')" :aria-label="t('logs.clear')">
          <Trash2 :size="14" />
        </button>
      </div>
    </div>

    <div class="log-container card" ref="logContainer">
      <EmptyState
        v-if="filtered.length === 0"
        :icon="ScrollText"
        :title="t('logs.empty')"
      />
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
/* Layout comes from the shared .page / .page--wide / .page--fill primitives;
   the header + segmented control use the global classes too. */

/* Destructive clear: neutral ghost by default, red-tinted on hover */
.clear-btn:hover {
  background: var(--color-error-soft);
  color: var(--color-error);
}

.log-container {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-3) var(--space-4);
  /* Deeper inset so the scrolling body reads as a console inside the glass shell */
  background: var(--color-bg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-sm), var(--edge-highlight);
  font-family: 'Cascadia Code', 'Consolas', 'Courier New', monospace;
  font-size: 12px;
  line-height: 1.5;
}
.log-container::-webkit-scrollbar { width: 10px; }
.log-container::-webkit-scrollbar-thumb {
  background: var(--color-neutral-strong);
  border-radius: var(--radius-sm);
  border: 3px solid transparent;
  background-clip: padding-box;
}
.log-container::-webkit-scrollbar-thumb:hover { background: var(--color-neutral); background-clip: padding-box; }
.log-line {
  padding: 1px var(--space-1);
  border-radius: var(--radius-sm);
  white-space: pre-wrap;
  word-break: break-all;
  transition: background 0.12s;
}
/* Theme token, not a hardcoded white wash — rgba(255,255,255,.04) was invisible on the
   light theme's near-white background. */
.log-line:hover { background: var(--color-neutral-soft); }
</style>
