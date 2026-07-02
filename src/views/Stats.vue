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
      borderColor: "rgba(16, 124, 16, 1)",
      backgroundColor: "rgba(16, 124, 16, 0.12)",
      borderWidth: 2,
      tension: 0.35,
      fill: true,
      pointRadius: 2,
      pointHoverRadius: 4,
    },
    {
      label: t("stats.upload"),
      data: shown.value.map((d) => d.upload),
      borderColor: "rgba(79, 110, 247, 1)",
      backgroundColor: "rgba(79, 110, 247, 0.12)",
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
    legend: { position: "top" as const, labels: { boxWidth: 12, font: { size: 11 } } },
    tooltip: {
      callbacks: {
        label: (ctx: TooltipItem<"line">) =>
          `${ctx.dataset.label}: ${formatBytes(Number(ctx.parsed.y ?? 0))}`,
      },
    },
  },
  scales: {
    x: { grid: { display: false }, ticks: { font: { size: 10 }, maxRotation: 0, autoSkip: true } },
    y: {
      beginAtZero: true,
      ticks: {
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
.range-tabs { display: flex; gap: 2px; background: var(--color-bg-secondary); border-radius: 8px; padding: 2px; }
.range-tab {
  border: none; background: transparent; cursor: pointer; font-size: 12px;
  padding: 4px 10px; border-radius: 6px; color: var(--color-text-secondary);
}
.range-tab.active { background: var(--color-bg); color: var(--color-text); box-shadow: 0 1px 2px rgba(0,0,0,0.08); }

.summary-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; margin-bottom: 14px; }
.summary-card { display: flex; align-items: center; gap: 12px; padding: 14px 16px; }
.summary-icon {
  width: 36px; height: 36px; border-radius: 9px; display: flex; align-items: center;
  justify-content: center; color: #fff; flex-shrink: 0;
}
.summary-icon.today { background: var(--accent-violet); }
.summary-icon.down { background: var(--color-success); }
.summary-icon.up { background: var(--color-primary); }
.summary-icon.total { background: #6b6b6b; }
.summary-label { font-size: 11.5px; color: var(--color-text-secondary); margin-bottom: 2px; }
.summary-value { font-size: 16px; font-weight: 600; }

.chart-card { padding: 16px; }
.chart-title { font-size: 13px; font-weight: 600; margin-bottom: 12px; }
.chart-wrap { height: 320px; }
.empty-hint { color: var(--color-text-secondary); font-size: 12.5px; padding: 40px 0; text-align: center; line-height: 1.6; }

@media (max-width: 720px) {
  .summary-grid { grid-template-columns: repeat(2, 1fr); }
}
</style>
