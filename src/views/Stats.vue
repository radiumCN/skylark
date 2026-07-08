<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { Line } from "vue-chartjs";
import {
  Chart as ChartJS,
  Title,
  Tooltip,
  Legend,
  LineElement,
  PointElement,
  Filler,
  CategoryScale,
  LinearScale,
  type TooltipItem,
} from "chart.js";
import { ArrowUp, ArrowDown, Database, RefreshCw } from "@lucide/vue";
import { useAppStore, type TrafficDay } from "../stores/app";
import { useI18n } from "vue-i18n";
import { formatBytes } from "../utils/format";
import { useDelayedRefresh } from "../composables/useDelayedRefresh";

const { t } = useI18n();

ChartJS.register(Title, Tooltip, Legend, LineElement, PointElement, Filler, CategoryScale, LinearScale);

const store = useAppStore();
const history = ref<TrafficDay[]>([]);
const loading = ref(false);
const rangeDays = ref(30);
const { refreshing, refresh } = useDelayedRefresh();

// Short MM-DD label for the x-axis.
function shortDate(date: string): string {
  const parts = date.split("-");
  return parts.length === 3 ? `${parts[1]}-${parts[2]}` : date;
}

const shown = computed(() => {
  const d = rangeDays.value;
  return d > 0 && history.value.length > d
    ? history.value.slice(history.value.length - d)
    : history.value;
});

const totalUpload = computed(() => shown.value.reduce((s, d) => s + d.upload, 0));
const totalDownload = computed(() => shown.value.reduce((s, d) => s + d.download, 0));
const totalAll = computed(() => totalUpload.value + totalDownload.value);

const todayEntry = computed(() => {
  if (history.value.length === 0) return null;
  return history.value[history.value.length - 1];
});

const chartData = computed(() => ({
  labels: shown.value.map((d) => shortDate(d.date)),
  datasets: [
    {
      label: t("stats.download"),
      data: shown.value.map((d) => d.download),
      borderColor: "#0e8f5a",
      backgroundColor: "rgba(14, 143, 90, 0.10)",
      borderWidth: 2,
      tension: 0.35,
      fill: true,
      pointRadius: 2,
      pointHoverRadius: 4,
    },
    {
      label: t("stats.upload"),
      data: shown.value.map((d) => d.upload),
      borderColor: "#5e6ad2",
      backgroundColor: "rgba(94, 106, 210, 0.10)",
      borderWidth: 2,
      tension: 0.35,
      fill: true,
      pointRadius: 2,
      pointHoverRadius: 4,
    },
  ],
}));

const chartOptions = computed(() => ({
  responsive: true,
  maintainAspectRatio: false,
  interaction: { mode: "index" as const, intersect: false },
  plugins: {
    legend: { position: "top" as const, labels: { color: "rgba(128,128,128,0.85)", boxWidth: 12, font: { size: 11 } } },
    tooltip: {
      callbacks: {
        label: (ctx: TooltipItem<"line">) =>
          `${ctx.dataset.label}: ${formatBytes(Number(ctx.parsed.y ?? 0))}`,
      },
    },
  },
  scales: {
    x: {
      grid: { display: false },
      ticks: { color: "rgba(128,128,128,0.65)", font: { size: 10 }, maxRotation: 0, autoSkip: true },
    },
    y: {
      beginAtZero: true,
      grid: { color: "rgba(128,128,128,0.10)" },
      ticks: {
        color: "rgba(128,128,128,0.65)",
        font: { size: 10 },
        callback: (v: number | string) => formatBytes(Number(v)),
      },
    },
  },
}));

async function load() {
  loading.value = true;
  try {
    history.value = await store.fetchTrafficHistory();
  } finally {
    loading.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">{{ t('stats.title') }}</h1>
      <div style="display:flex;gap:8px;align-items:center;">
        <div class="range-tabs">
          <button
            v-for="d in [7, 30, 90]"
            :key="d"
            class="range-tab"
            :class="{ active: rangeDays === d }"
            @click="rangeDays = d"
          >
            {{ t('stats.daysN', { n: d }) }}
          </button>
        </div>
        <button class="btn btn-ghost" :disabled="refreshing" @click="refresh(() => load())">
          <RefreshCw :size="13" :class="{ spin: refreshing }" />
          {{ t('stats.refresh') }}
        </button>
      </div>
    </div>

    <!-- Summary cards -->
    <div class="summary-grid">
      <div class="card summary-card">
        <div class="summary-icon today"><Database :size="16" /></div>
        <div class="summary-body">
          <div class="summary-label">{{ t('stats.today') }}</div>
          <div class="summary-value">
            {{ formatBytes((todayEntry?.upload ?? 0) + (todayEntry?.download ?? 0)) }}
          </div>
        </div>
      </div>
      <div class="card summary-card">
        <div class="summary-icon down"><ArrowDown :size="16" /></div>
        <div class="summary-body">
          <div class="summary-label">{{ t('stats.downloadRange', { n: rangeDays }) }}</div>
          <div class="summary-value">{{ formatBytes(totalDownload) }}</div>
        </div>
      </div>
      <div class="card summary-card">
        <div class="summary-icon up"><ArrowUp :size="16" /></div>
        <div class="summary-body">
          <div class="summary-label">{{ t('stats.uploadRange', { n: rangeDays }) }}</div>
          <div class="summary-value">{{ formatBytes(totalUpload) }}</div>
        </div>
      </div>
      <div class="card summary-card">
        <div class="summary-icon total"><Database :size="16" /></div>
        <div class="summary-body">
          <div class="summary-label">{{ t('stats.totalRange', { n: rangeDays }) }}</div>
          <div class="summary-value">{{ formatBytes(totalAll) }}</div>
        </div>
      </div>
    </div>

    <!-- Chart -->
    <div class="card chart-card">
      <div class="chart-title">{{ t('stats.dailyTraffic') }}</div>
      <div v-if="shown.length > 0" class="chart-wrap">
        <Line :data="chartData" :options="chartOptions" />
      </div>
      <div v-else class="empty-hint">
        {{ t('stats.emptyHint') }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.range-tabs {
  display: flex;
  gap: 2px;
  background: var(--color-neutral);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  padding: 2px;
}
.range-tab {
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-sm);
  padding: var(--space-1) 10px;
  border-radius: var(--radius-sm);
  color: var(--color-text-secondary);
  transition: color 150ms ease-out, background 150ms ease-out;
}
.range-tab:hover { color: var(--color-text); }
.range-tab.active {
  background: var(--color-surface-strong);
  color: var(--color-text);
  box-shadow: var(--shadow-sm), var(--edge-highlight);
}

.summary-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}
.summary-card {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-4);
  border-radius: var(--radius-lg);
  transition: transform 150ms ease-out, box-shadow 150ms ease-out;
}
.summary-card:hover {
  transform: translateY(-1px);
  box-shadow: var(--shadow-md), var(--edge-highlight);
}
.summary-icon {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-md);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.summary-icon.today { background: var(--color-primary-soft); color: var(--accent-violet); }
.summary-icon.down { background: var(--color-success-soft); color: var(--color-success); }
.summary-icon.up { background: var(--color-primary-soft); color: var(--color-primary); }
.summary-icon.total { background: var(--color-neutral-strong); color: var(--color-text-secondary); }
.summary-label {
  font-size: var(--fs-xs);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--color-text-secondary);
  margin-bottom: 2px;
}
.summary-value {
  font-size: var(--fs-lg);
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: var(--color-text);
}

.chart-card { padding: var(--space-4); box-shadow: var(--shadow-md), var(--edge-highlight); }
.chart-title {
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--color-text);
  margin-bottom: var(--space-3);
}
.chart-wrap { height: 320px; }
.empty-hint {
  color: var(--color-text-muted);
  font-size: var(--fs-sm);
  padding: 40px 0;
  text-align: center;
  line-height: 1.6;
}

@media (max-width: 720px) {
  .summary-grid { grid-template-columns: repeat(2, 1fr); }
}
</style>
