<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import { useRouter } from "vue-router";
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
import { ArrowUp, ArrowDown, Database, RefreshCw, Home } from "@lucide/vue";
import { useAppStore, type TrafficDay } from "../stores/app";
import { useI18n } from "vue-i18n";
import { formatBytes } from "../utils/format";
import { useDelayedRefresh } from "../composables/useDelayedRefresh";
import StatTile from "../components/StatTile.vue";
import EmptyState from "../components/EmptyState.vue";
import Skeleton from "../components/Skeleton.vue";

const { t } = useI18n();
const router = useRouter();

ChartJS.register(Title, Tooltip, Legend, LineElement, PointElement, Filler, CategoryScale, LinearScale);

const store = useAppStore();
const history = ref<TrafficDay[]>([]);
const loading = ref(false);
const rangeDays = ref(30);
const { refreshing, refresh } = useDelayedRefresh();
/** Bumped on theme flips so Chart.js colors re-read CSS variables. */
const themeTick = ref(0);

function cssVar(name: string, fallback: string): string {
  void themeTick.value;
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

function bumpTheme() {
  themeTick.value += 1;
}

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

// Skeleton only on the very first fetch (no data yet). Later refreshes keep the
// existing numbers visible and use the refresh button's spinner instead.
const initialLoading = computed(() => loading.value && history.value.length === 0);

const chartData = computed(() => ({
  labels: shown.value.map((d) => shortDate(d.date)),
  datasets: [
    {
      label: t("stats.download"),
      data: shown.value.map((d) => d.download),
      borderColor: cssVar("--color-success", "#0e8f5a"),
      backgroundColor: cssVar("--color-success-soft", "rgba(14, 143, 90, 0.14)"),
      borderWidth: 2,
      tension: 0.35,
      fill: true,
      pointRadius: 2,
      pointHoverRadius: 4,
    },
    {
      label: t("stats.upload"),
      data: shown.value.map((d) => d.upload),
      borderColor: cssVar("--color-primary", "#5e6ad2"),
      backgroundColor: cssVar("--color-primary-soft", "rgba(94, 106, 210, 0.14)"),
      borderWidth: 2,
      tension: 0.35,
      fill: true,
      pointRadius: 2,
      pointHoverRadius: 4,
    },
  ],
}));

const chartOptions = computed(() => {
  const muted = cssVar("--color-text-muted", "#7d8298");
  const border = cssVar("--color-border", "rgba(128,128,128,0.10)");
  return {
    responsive: true,
    maintainAspectRatio: false,
    interaction: { mode: "index" as const, intersect: false },
    plugins: {
      legend: { position: "top" as const, labels: { color: muted, boxWidth: 12, font: { size: 11 } } },
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
        ticks: { color: muted, font: { size: 10 }, maxRotation: 0, autoSkip: true },
      },
      y: {
        beginAtZero: true,
        grid: { color: border },
        ticks: {
          color: muted,
          font: { size: 10 },
          callback: (v: number | string) => formatBytes(Number(v)),
        },
      },
    },
  };
});

async function load() {
  loading.value = true;
  try {
    history.value = await store.fetchTrafficHistory();
  } finally {
    loading.value = false;
  }
}

let mq: MediaQueryList | null = null;
onMounted(() => {
  load();
  mq = window.matchMedia("(prefers-color-scheme: dark)");
  mq.addEventListener("change", bumpTheme);
});
onUnmounted(() => {
  mq?.removeEventListener("change", bumpTheme);
});
watch(() => store.config.theme, bumpTheme);
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

    <!-- Summary cards (skeleton on first load) -->
    <div v-if="initialLoading" class="summary-grid">
      <div v-for="i in 4" :key="i" class="card skel-tile">
        <Skeleton width="40px" height="40px" radius="var(--radius-lg)" />
        <div class="skel-tile-body">
          <Skeleton width="60%" height="10px" />
          <Skeleton width="45%" height="18px" />
        </div>
      </div>
    </div>
    <div v-else class="summary-grid">
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
      <div v-if="initialLoading" class="chart-wrap">
        <Skeleton width="100%" height="100%" radius="var(--radius-md)" />
      </div>
      <div v-else-if="shown.length > 0" class="chart-wrap">
        <Line :data="chartData" :options="chartOptions" />
      </div>
      <EmptyState v-else :icon="Database" :title="t('stats.emptyHint')">
        <button class="btn btn-primary" @click="router.push('/home')">
          <Home :size="14" />
          {{ t('stats.emptyCta') }}
        </button>
      </EmptyState>
    </div>
  </div>
</template>

<style scoped>
.summary-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--space-3);
}
.skel-tile {
  padding: var(--space-4);
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
}
.skel-tile-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding-top: 2px;
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
