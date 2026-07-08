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
import StatTile from "../components/StatTile.vue";
import EmptyState from "../components/EmptyState.vue";

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
      <div class="header-actions">
        <div class="segmented">
          <button
            v-for="d in [7, 30, 90]"
            :key="d"
            class="segmented__item"
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
      <StatTile
        :icon="Database"
        :label="t('stats.today')"
        :value="formatBytes((todayEntry?.upload ?? 0) + (todayEntry?.download ?? 0))"
        accent="violet"
      />
      <StatTile
        :icon="ArrowDown"
        :label="t('stats.downloadRange', { n: rangeDays })"
        :value="formatBytes(totalDownload)"
        accent="success"
      />
      <StatTile
        :icon="ArrowUp"
        :label="t('stats.uploadRange', { n: rangeDays })"
        :value="formatBytes(totalUpload)"
        accent="primary"
      />
      <StatTile
        :icon="Database"
        :label="t('stats.totalRange', { n: rangeDays })"
        :value="formatBytes(totalAll)"
        accent="teal"
      />
    </div>

    <!-- Chart -->
    <div class="card chart-card">
      <div class="chart-title">{{ t('stats.dailyTraffic') }}</div>
      <div v-if="shown.length > 0" class="chart-wrap">
        <Line :data="chartData" :options="chartOptions" />
      </div>
      <EmptyState v-else :icon="Database" :title="t('stats.emptyHint')" />
    </div>
  </div>
</template>

<style scoped>
.summary-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--space-3);
}

.chart-card { padding: var(--space-4); box-shadow: var(--shadow-md), var(--edge-highlight); }
.chart-title {
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--color-text);
  margin-bottom: var(--space-3);
}
.chart-wrap { height: 320px; }

@media (max-width: 720px) {
  .summary-grid { grid-template-columns: repeat(2, 1fr); }
}
</style>
