<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";
import { Gauge, RefreshCw, CheckCircle, Signal, Zap, ArrowUpDown, Plus, Trash2, Pencil, Layers, ChevronDown, Columns3 } from "@lucide/vue";
import { useAppStore } from "../stores/app";
import { useFeedbackStore } from "../stores/feedback";
import { useI18n } from "vue-i18n";
import { useDelayedRefresh } from "../composables/useDelayedRefresh";
import EmptyState from "../components/EmptyState.vue";
import Skeleton from "../components/Skeleton.vue";
import Select from "../components/Select.vue";

const { t } = useI18n();

const groupTypeOptions = computed(() => [
  { value: "urltest", label: t("nodes.autoSelectByLatency") },
  { value: "selector", label: t("nodes.manualSelect") },
]);
const store = useAppStore();
const fb = useFeedbackStore();
const { refreshing, refresh } = useDelayedRefresh();
const testingAll = ref(false);
const testingGroup = ref(false);
const testingIds = ref<string[]>([]);
const filterSubId = ref<string>(localStorage.getItem("nodes_filter_sub") ?? "all");
const sortBy = ref<"none" | "latency" | "speed">(
  (localStorage.getItem("nodes_sort") as "none" | "latency" | "speed") ?? "none"
);
const search = ref("");

const COLUMN_OPTIONS = [1, 2, 3, 4];
const savedColumns = Number(localStorage.getItem("nodes_columns"));
const columns = ref(COLUMN_OPTIONS.includes(savedColumns) ? savedColumns : 1);

watch(sortBy, (v) => localStorage.setItem("nodes_sort", v));
watch(columns, (v) => localStorage.setItem("nodes_columns", String(v)));

function validateSubFilter() {
  const savedSub = filterSubId.value;
  if (savedSub !== "all" && store.subscriptions.length > 0 && !store.subscriptions.find((s) => s.id === savedSub)) {
    filterSubId.value = "all";
    localStorage.setItem("nodes_filter_sub", "all");
  }
}

// Re-validate when subscriptions load (async after mount)
watch(() => store.subscriptions.length, validateSubFilter);

onMounted(() => {
  validateSubFilter();
  // Use the store's single shared poller for the active auto group's current node.
  store.ensureActiveNowPoller();
  store.fetchProxyGroups();
});

// Keep the refresh spin visible for at least 600ms — the fetch is near-instant.
function manualRefresh() {
  refresh(() => store.fetchNodes());
}

// ─── Custom proxy groups ─────────────────────────────────────────────
const groupsOpen = ref(true); // collapse state for the groups card (layout affordance)
const showGroupEditor = ref(false);
const editingGroupId = ref<string | null>(null);
const groupForm = ref<{ name: string; group_type: string; nodes: string[] }>({
  name: "",
  group_type: "urltest",
  nodes: [],
});
const allNodeNames = computed(() => Array.from(new Set(store.nodes.map((n) => n.name))));

function openNewGroup() {
  editingGroupId.value = null;
  groupForm.value = { name: "", group_type: "urltest", nodes: [] };
  showGroupEditor.value = true;
}
function openEditGroup(g: { id: string; name: string; group_type: string; nodes: string[] }) {
  editingGroupId.value = g.id;
  groupForm.value = { name: g.name, group_type: g.group_type, nodes: [...g.nodes] };
  showGroupEditor.value = true;
}
function toggleMember(name: string) {
  const arr = groupForm.value.nodes;
  const i = arr.indexOf(name);
  if (i >= 0) arr.splice(i, 1);
  else arr.push(name);
}
async function saveGroup() {
  const name = groupForm.value.name.trim();
  if (!name || groupForm.value.nodes.length === 0) return;
  const groups = store.proxyGroups.map((g) => ({ ...g }));
  if (editingGroupId.value) {
    const idx = groups.findIndex((g) => g.id === editingGroupId.value);
    if (idx >= 0) {
      groups[idx] = { ...groups[idx], name, group_type: groupForm.value.group_type, nodes: [...groupForm.value.nodes] };
    }
  } else {
    groups.push({ id: crypto.randomUUID(), name, group_type: groupForm.value.group_type, nodes: [...groupForm.value.nodes] });
  }
  await store.saveProxyGroups(groups);
  showGroupEditor.value = false;
}
async function deleteGroup(id: string) {
  if (!(await fb.confirm({ message: t("nodes.confirmDeleteGroup"), danger: true }))) return;
  await store.saveProxyGroups(store.proxyGroups.filter((g) => g.id !== id));
}
async function useGroup(name: string) {
  await store.setAutoNode(name);
}

const nodesForSub = computed(() => {
  if (filterSubId.value === "all") return store.nodes;
  return store.nodes.filter((n) => n.subscription_id === filterSubId.value);
});

const filtered = computed(() => {
  let nodes = nodesForSub.value;
  if (search.value) {
    const q = search.value.toLowerCase();
    nodes = nodes.filter(
      (n) =>
        n.name.toLowerCase().includes(q) ||
        n.server.toLowerCase().includes(q) ||
        n.protocol.toLowerCase().includes(q)
    );
  }
  if (sortBy.value === "latency") {
    nodes = [...nodes].sort((a, b) => {
      if (a.latency == null && b.latency == null) return 0;
      if (a.latency == null) return 1;
      if (b.latency == null) return -1;
      return a.latency - b.latency;
    });
  } else if (sortBy.value === "speed") {
    nodes = [...nodes].sort((a, b) => {
      if (a.download_speed == null && b.download_speed == null) return 0;
      if (a.download_speed == null) return 1;
      if (b.download_speed == null) return -1;
      return b.download_speed - a.download_speed;
    });
  }
  return nodes;
});

function switchSub(id: string) {
  filterSubId.value = id;
  localStorage.setItem("nodes_filter_sub", id);
}

const latencyColor = (ms?: number) => {
  if (ms === undefined || ms === null) return "var(--color-text-muted)";
  if (ms < 100) return "var(--latency-good)";
  if (ms < 300) return "var(--latency-mid)";
  return "var(--latency-bad)";
};

const latencyLabel = (ms?: number) => {
  if (ms === undefined || ms === null) return "--";
  return `${ms}ms`;
};

const speedColor = (kbps?: number) => {
  if (kbps === undefined || kbps === null) return "var(--color-text-muted)";
  if (kbps >= 5120) return "var(--latency-good)";   // ≥ 5 MB/s
  if (kbps >= 1024) return "var(--latency-mid)";    // ≥ 1 MB/s
  return "var(--latency-bad)";                       // < 1 MB/s
};

const speedLabel = (kbps?: number) => {
  if (kbps === undefined || kbps === null) return "";
  if (kbps >= 1024) return `${(kbps / 1024).toFixed(1)} MB/s`;
  return `${kbps} KB/s`;
};

function isTesting(id: string) {
  return testingIds.value.includes(id);
}

async function testAll() {
  testingAll.value = true;
  await Promise.allSettled(
    store.nodes.map(async (n) => {
      testingIds.value = [...testingIds.value, n.id];
      await store.testNodeSpeed(n.id);
      testingIds.value = testingIds.value.filter((id) => id !== n.id);
    })
  );
  testingAll.value = false;
}

async function testOne(nodeId: string) {
  testingIds.value = [...testingIds.value, nodeId];
  await store.testNodeSpeed(nodeId);
  testingIds.value = testingIds.value.filter((id) => id !== nodeId);
}

async function selectNode(nodeId: string) {
  await store.setActiveNode(nodeId);
}

// Switch to a dynamic urltest group (core continuously picks the fastest node).
// No arg = global "auto"; pass a subscription id for that subscription's group.
async function selectAuto(subId?: string) {
  await store.setAutoNode(subId ? `auto-${subId}` : undefined);
}

// Force an immediate re-test of the current view's auto group.
async function retestGroup() {
  if (testingGroup.value) return;
  testingGroup.value = true;
  try {
    await store.testGroupDelay(currentAutoTag.value);
  } finally {
    testingGroup.value = false;
  }
}

// The auto group tag matching the current view (global vs per-subscription).
const currentAutoTag = computed(() =>
  filterSubId.value === "all" ? "auto" : `auto-${filterSubId.value}`
);
// Show a per-subscription auto card only when that subscription has ≥2 nodes
// (matches the backend, which only builds a urltest group in that case).
const showAutoCard = computed(() =>
  filterSubId.value === "all" ? store.nodes.length > 0 : nodesForSub.value.length >= 2
);
// The node the active auto group is currently routing through (for display).
const autoNowName = computed(() => store.activeNodeNow);
</script>

<template>
  <div class="page">
    <!-- Sticky glass toolbar: header actions + filters travel with the scroll. -->
    <div class="toolbar">
      <div class="page-header">
        <h1 class="page-title">{{ t("nodes.title") }}</h1>
        <div class="header-actions">
          <span class="node-count">{{ t("nodes.nodeCount", { n: store.nodes.length }) }}</span>
          <button class="btn btn-ghost" :disabled="testingAll" @click="testAll" :title="t('nodes.testAllTip')">
            <Gauge :size="14" :class="{ spin: testingAll }" />
            {{ testingAll ? t("nodes.testing") : t("nodes.testAll") }}
          </button>
          <!-- Sort selector -->
          <div class="sort-wrap">
            <ArrowUpDown :size="13" class="sort-icon" />
            <div class="segmented">
              <button
                v-for="[k, label] in [['none', t('nodes.sortDefault')],['latency', t('nodes.sortLatency')],['speed', t('nodes.sortSpeed')]]"
                :key="k"
                class="segmented__item"
                :class="{ active: sortBy === k }"
                @click="sortBy = k as typeof sortBy"
              >{{ label }}</button>
            </div>
          </div>

          <!-- Column-count selector -->
          <div class="sort-wrap">
            <Columns3 :size="13" class="sort-icon" />
            <div class="segmented">
              <button
                v-for="c in COLUMN_OPTIONS"
                :key="c"
                class="segmented__item"
                :class="{ active: columns === c }"
                :title="t('nodes.columnsTip', { n: c })"
                @click="columns = c"
              >{{ c }}</button>
            </div>
          </div>

          <button class="btn btn-ghost" @click="manualRefresh" :disabled="refreshing">
            <RefreshCw :size="14" :class="{ spin: refreshing }" />
            {{ t("nodes.refresh") }}
          </button>
        </div>
      </div>

      <!-- Filters -->
      <div class="filters">
        <input class="input search-input" v-model="search" :placeholder="t('nodes.searchPlaceholder')" />

        <!-- Subscription selector (show only if there is at least one sub) -->
        <div v-if="store.subscriptions.length > 0" class="segmented sub-segmented">
          <button
            class="segmented__item"
            :class="{ active: filterSubId === 'all' }"
            @click="switchSub('all')"
          >
            {{ t("nodes.allTab") }} <span class="sub-count">{{ store.nodes.length }}</span>
          </button>
          <button
            v-for="sub in store.subscriptions"
            :key="sub.id"
            class="segmented__item"
            :class="{ active: filterSubId === sub.id }"
            @click="switchSub(sub.id)"
          >
            {{ sub.name }}
            <span class="sub-count">{{ store.nodes.filter(n => n.subscription_id === sub.id).length }}</span>
          </button>
        </div>
      </div>
    </div>

    <!-- Custom proxy groups -->
    <div v-if="store.nodes.length > 0" class="card group-card">
      <div class="group-head">
        <button
          class="group-title"
          :aria-expanded="groupsOpen || showGroupEditor"
          @click="groupsOpen = !groupsOpen"
        >
          <Layers :size="14" />
          <span>{{ t("nodes.customGroups") }}</span>
          <span v-if="store.proxyGroups.length > 0" class="group-count">{{ store.proxyGroups.length }}</span>
          <ChevronDown :size="15" class="group-chevron" :class="{ open: groupsOpen || showGroupEditor }" />
        </button>
        <button class="btn btn-ghost btn-sm" @click="openNewGroup">
          <Plus :size="13" />
          {{ t("nodes.newGroup") }}
        </button>
      </div>

      <div v-if="(groupsOpen || showGroupEditor) && store.proxyGroups.length === 0 && !showGroupEditor" class="group-empty">
        {{ t("nodes.groupEmptyHint") }}
      </div>

      <div v-if="(groupsOpen || showGroupEditor) && store.proxyGroups.length > 0" class="group-list">
        <div
          v-for="g in store.proxyGroups"
          :key="g.id"
          class="group-item"
          :class="{ active: store.activeProxyTag === g.name }"
        >
          <div class="group-info">
            <div class="group-name">
              {{ g.name }}
              <span class="group-badge">{{ g.group_type === "urltest" ? t("nodes.autoSelect") : t("nodes.manualSelect") }}</span>
            </div>
            <div class="group-members">{{ t("nodes.nodeCount", { n: g.nodes.length }) }}</div>
          </div>
          <div class="group-actions">
            <button class="btn btn-ghost btn-sm" @click="useGroup(g.name)">
              {{ store.activeProxyTag === g.name ? t("nodes.inUse") : t("nodes.useGroup") }}
            </button>
            <button class="icon-btn" :title="t('nodes.edit')" @click="openEditGroup(g)">
              <Pencil :size="14" />
            </button>
            <button class="icon-btn danger" :title="t('nodes.delete')" @click="deleteGroup(g.id)">
              <Trash2 :size="14" />
            </button>
          </div>
        </div>
      </div>

      <!-- Group editor -->
      <div v-if="showGroupEditor" class="group-editor">
        <div class="editor-row">
          <input class="input" v-model="groupForm.name" :placeholder="t('nodes.groupNamePlaceholder')" />
          <Select class="editor-type" v-model="groupForm.group_type" :options="groupTypeOptions" />
        </div>
        <div class="member-label">{{ t("nodes.selectMembers", { n: groupForm.nodes.length }) }}</div>
        <div class="member-grid">
          <label
            v-for="name in allNodeNames"
            :key="name"
            class="member-chip"
            :class="{ on: groupForm.nodes.includes(name) }"
          >
            <input
              type="checkbox"
              :checked="groupForm.nodes.includes(name)"
              @change="toggleMember(name)"
            />
            {{ name }}
          </label>
        </div>
        <div class="editor-actions">
          <button class="btn btn-ghost" @click="showGroupEditor = false">{{ t("nodes.cancel") }}</button>
          <button
            class="btn btn-primary"
            :disabled="!groupForm.name.trim() || groupForm.nodes.length === 0"
            @click="saveGroup"
          >
            {{ editingGroupId ? t("nodes.saveChanges") : t("nodes.create") }}
          </button>
        </div>
      </div>
    </div>

    <!-- Speed-test notice when proxy is not running -->
    <div v-if="store.nodes.length > 0 && !store.status.running" class="speed-notice">
      <span>⚡ {{ t("nodes.speedNoticePrefix") }}<strong>{{ t("nodes.speedNoticeStrong") }}</strong></span>
    </div>

    <!-- Cold-start skeleton (before the first data load resolves) -->
    <div v-if="store.nodes.length === 0 && !store.initialized" class="node-skel-list">
      <div v-for="i in 6" :key="i" class="card node-skel">
        <Skeleton width="8px" height="8px" circle />
        <div class="node-skel-body">
          <Skeleton width="42%" height="13px" />
          <Skeleton width="26%" height="10px" />
        </div>
        <Skeleton width="52px" height="20px" radius="100px" />
      </div>
    </div>

    <!-- Empty -->
    <EmptyState
      v-else-if="store.nodes.length === 0"
      :icon="Signal"
      :title="t('nodes.emptyTitle')"
      :desc="t('nodes.emptyDesc')"
    />

    <!-- Node List -->
    <div class="node-list" :style="{ '--node-cols': columns }">
      <!-- Dynamic auto-select (urltest) group — global or per-subscription -->
      <div
        v-if="showAutoCard"
        class="card node-item auto-item"
        :class="{ active: store.activeProxyTag === currentAutoTag }"
        @click="selectAuto(filterSubId === 'all' ? undefined : filterSubId)"
        :title="t('nodes.autoCardTip')"
      >
        <div class="node-left">
          <div class="active-indicator">
            <Zap v-if="store.activeProxyTag === currentAutoTag" :size="16" class="auto-icon" />
            <div v-else class="check-placeholder" />
          </div>
          <div class="node-info">
            <div class="node-name">
              {{ filterSubId === 'all' ? t('nodes.autoAllNodes') : t('nodes.autoThisSub') }}
            </div>
            <div class="node-meta">
              <span class="badge badge-gray protocol-badge">URLTest</span>
              <span
                v-if="store.activeProxyTag === currentAutoTag && autoNowName"
                class="node-server auto-now"
              >{{ t("nodes.currentHit", { name: autoNowName }) }}</span>
              <span v-else class="node-server">{{ t("nodes.autoSwitchDesc") }}</span>
            </div>
          </div>
        </div>
        <div class="node-right">
          <button
            class="btn btn-ghost icon-btn"
            :disabled="testingGroup || !store.status.running"
            @click.stop="retestGroup"
            :title="store.status.running ? t('nodes.retestGroupTip') : t('nodes.needStartProxy')"
          >
            <RefreshCw :size="13" :class="{ spin: testingGroup }" />
          </button>
        </div>
      </div>

      <div
        v-for="node in filtered"
        :key="node.id"
        class="card node-item"
        :class="{ active: node.is_active }"
        @click="selectNode(node.id)"
      >
        <div class="node-left">
          <div class="active-indicator">
            <CheckCircle v-if="node.is_active" :size="16" class="check-icon" />
            <div v-else class="check-placeholder" />
          </div>
          <div class="node-info">
            <div class="node-name">
              <span
                class="health-dot"
                :style="{ background: latencyColor(node.latency) }"
                :title="latencyLabel(node.latency)"
              />
              {{ node.name }}
            </div>
            <div class="node-meta">
              <span class="badge badge-gray protocol-badge">{{ node.protocol }}</span>
              <span class="node-server">{{ node.server }}:{{ node.port }}</span>
            </div>
          </div>
        </div>
        <div class="node-right">
          <div class="speed-info">
            <span class="latency" :style="{ color: latencyColor(node.latency) }">
              {{ latencyLabel(node.latency) }}
            </span>
            <!-- Show download speed if measured; show "↓ --" if tested but proxy was off -->
            <span
              v-if="node.latency !== undefined && node.latency !== null"
              class="download-speed"
              :style="{ color: node.download_speed != null ? speedColor(node.download_speed) : 'var(--color-text-muted)' }"
              :title="node.download_speed == null ? t('nodes.downloadNeedProxy') : ''"
            >
              ↓ {{ node.download_speed != null ? speedLabel(node.download_speed) : '--' }}
            </span>
          </div>
          <button
            class="btn btn-ghost icon-btn"
            :disabled="isTesting(node.id)"
            @click.stop="testOne(node.id)"
            :title="store.status.running ? t('nodes.testNodeTip') : t('nodes.testNodeTipNoProxy')"
          >
            <Gauge :size="13" :class="{ spin: isTesting(node.id) }" />
          </button>
        </div>
      </div>
    </div>

    <div v-if="filtered.length === 0 && store.nodes.length > 0" class="no-result">
      {{ t("nodes.noResult", { q: search }) }}
    </div>
  </div>
</template>

<style scoped>
/* Sticky toolbar: title/actions + filters pinned to the scroll top. Full-bleed
   over the shell's 24px padding so it reads as an edge-to-edge header band rather
   than a flat rectangle floating over the ambient glow; opaque bg hides the list
   scrolling underneath. */
.toolbar {
  position: sticky;
  top: 0;
  z-index: var(--z-sticky);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  margin: -24px -24px 0;
  padding: 24px 24px var(--space-3);
  background: var(--color-bg);
  border-bottom: 1px solid var(--color-border);
}
.node-count { font-size: 12px; color: var(--color-text-secondary); }

.filters { display: flex; align-items: center; gap: var(--space-3); flex-wrap: wrap; }
.search-input { max-width: 340px; flex: 1 1 220px; }

/* Sort control: leading icon + shared segmented pill group. */
.sort-wrap { display: inline-flex; align-items: center; gap: 6px; }
.sort-icon { color: var(--color-text-muted); flex-shrink: 0; }

/* Subscription selector reuses the segmented control; may wrap when many subs. */
.sub-segmented { flex-wrap: wrap; }
.sub-count {
  font-size: 10px; font-weight: 700;
  background: var(--color-neutral-strong); color: var(--color-text-secondary);
  border-radius: var(--radius-sm); padding: 0 5px; min-width: 18px; text-align: center;
}
.segmented__item.active .sub-count {
  background: var(--color-primary-soft); color: var(--color-primary);
}

/* Column count is user-chosen (1–4) and fed in as --node-cols. */
.node-list {
  display: grid;
  grid-template-columns: repeat(var(--node-cols, 1), minmax(0, 1fr));
  gap: 6px;
}
/* The auto-select card is a single logical entry — keep it on its own full row. */
.auto-item { grid-column: 1 / -1; }
.node-skel-list { display: flex; flex-direction: column; gap: 6px; }
.node-skel {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
}
.node-skel-body { flex: 1; display: flex; flex-direction: column; gap: var(--space-2); }
.node-item {
  padding: 12px 16px;
  display: flex; align-items: center; justify-content: space-between; gap: 12px;
  cursor: pointer;
  transition: background 0.15s ease-out, border-color 0.15s ease-out, box-shadow 0.15s ease-out;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
}
.node-item:hover {
  box-shadow: var(--shadow-md);
  background: var(--color-surface-strong);
  border-color: var(--color-primary-soft);
}
.node-item.active {
  border-color: var(--color-primary);
  background: var(--color-primary-soft);
  box-shadow: 0 0 0 1px var(--color-primary-glow);
}
.auto-item.active {
  border-color: var(--accent-amber);
  background: var(--color-warning-soft);
  box-shadow: none;
}
.auto-item .auto-icon { color: var(--accent-amber); }
.auto-item .auto-now { color: var(--accent-amber); font-weight: 500; }
.node-left { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0; }
.active-indicator { flex-shrink: 0; }
.check-icon { color: var(--color-primary); }
.auto-icon { color: var(--accent-amber); }
.check-placeholder { width: 16px; height: 16px; }
.node-info { flex: 1; min-width: 0; }
.node-name { font-size: 13px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.health-dot {
  display: inline-block; width: 7px; height: 7px; border-radius: 50%;
  margin-right: 6px; vertical-align: middle; flex-shrink: 0;
}
.node-meta { display: flex; align-items: center; gap: 6px; margin-top: 3px; }
.protocol-badge { font-size: 10px; padding: 1px 6px; }
.node-server { font-size: 11px; color: var(--color-text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

.node-right { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.speed-info { display: flex; flex-direction: column; align-items: flex-end; gap: 3px; min-width: 72px; }
.latency {
  font-size: var(--fs-sm); font-weight: 600;
  padding: 1px 7px; border-radius: var(--radius-sm);
  background: var(--color-neutral-soft);
}
.download-speed { font-size: var(--fs-xs); font-weight: 500; }
.icon-btn { padding: 5px !important; }

.no-result { text-align: center; color: var(--color-text-muted); font-size: 13px; padding: 24px; }

.speed-notice {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 14px; border-radius: var(--radius-md); font-size: var(--fs-sm);
  background: var(--color-warning-soft); border: 1px solid var(--color-warning-soft);
  color: var(--color-attention);
}
.speed-notice strong { color: var(--color-text); }

/* ─── Custom proxy groups ─── */
.group-card { padding: 14px 16px; display: flex; flex-direction: column; gap: 10px; }
.group-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.group-title {
  display: flex; align-items: center; gap: 7px;
  font-size: 13px; font-weight: 600;
  padding: 2px 4px; margin: -2px -4px;
  border: none; background: transparent; color: var(--color-text);
  border-radius: var(--radius-sm); cursor: pointer;
  transition: color 0.15s ease-out;
}
.group-title:hover { color: var(--color-primary); }
.group-count {
  font-size: 10px; font-weight: 700;
  background: var(--color-neutral-strong); color: var(--color-text-secondary);
  border-radius: var(--radius-sm); padding: 0 5px; min-width: 18px; text-align: center;
}
.group-chevron {
  color: var(--color-text-muted);
  transition: transform 0.18s ease-out;
}
.group-chevron.open { transform: rotate(180deg); }
.btn-sm { padding: 3px 10px !important; font-size: 12px; }
.group-empty { font-size: 12px; color: var(--color-text-muted); }
.group-list { display: flex; flex-direction: column; gap: 8px; }
.group-item {
  display: flex; align-items: center; justify-content: space-between; gap: 10px;
  padding: 8px 12px; border: 1px solid var(--color-border);
  border-left: 2px solid var(--color-border);
  border-radius: var(--radius-md); background: var(--color-neutral-soft);
  transition: background 0.15s ease-out, border-color 0.15s ease-out;
}
.group-item:hover { background: var(--color-neutral); }
.group-item.active {
  border-color: var(--color-primary);
  border-left-color: var(--color-primary);
  background: var(--color-primary-soft);
  box-shadow: 0 0 0 1px var(--color-primary-glow);
}
.group-info { min-width: 0; }
.group-name { font-size: var(--fs-md); font-weight: 600; display: flex; align-items: center; gap: 6px; }
.group-badge {
  font-size: 10px; font-weight: 600; padding: 1px 7px; border-radius: var(--radius-sm);
  background: var(--color-warning-soft); color: var(--accent-amber);
}
.group-members { font-size: 11px; color: var(--color-text-muted); margin-top: 2px; }
.group-actions { display: flex; align-items: center; gap: 4px; flex-shrink: 0; }

.icon-btn {
  display: inline-flex; align-items: center; justify-content: center;
  width: 28px; height: 28px; border: none; border-radius: var(--radius-sm);
  background: transparent; color: var(--color-text-secondary); cursor: pointer;
  transition: background 0.15s ease-out, color 0.15s ease-out;
}
.icon-btn:hover { background: var(--color-neutral); color: var(--color-text); }
.icon-btn.danger:hover { background: var(--color-error-soft); color: var(--color-error); }

.group-editor {
  display: flex; flex-direction: column; gap: 10px;
  padding: 12px; border: 1px dashed var(--color-border); border-radius: var(--radius-md);
}
.editor-row { display: flex; gap: 8px; }
.editor-type { max-width: 180px; }
.member-label {
  font-size: var(--fs-xs); font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.5px;
}
.member-grid {
  display: flex; flex-wrap: wrap; gap: 6px;
  max-height: 180px; overflow-y: auto;
}
.member-chip {
  display: inline-flex; align-items: center; gap: 5px;
  padding: 4px 9px; border: 1px solid var(--color-border);
  border-radius: var(--radius-sm); font-size: var(--fs-sm); cursor: pointer;
  transition: background 0.15s ease-out, border-color 0.15s ease-out, color 0.15s ease-out;
  user-select: none;
}
.member-chip:hover { background: var(--color-neutral); }
.member-chip.on { border-color: var(--color-primary); background: var(--color-primary-soft); color: var(--color-primary); }
.member-chip input { margin: 0; }
.editor-actions { display: flex; gap: 8px; justify-content: flex-end; }

/* ─── Narrow-width fallback: keep everything single-column and legible. ─── */
@media (max-width: 820px) {
  .page-header { flex-wrap: wrap; }
  .header-actions { width: 100%; margin-left: 0; }
  .node-count { margin-right: auto; }
  .filters { flex-direction: column; align-items: stretch; }
  .search-input { max-width: none; flex: 1 1 auto; }
  .sort-wrap { flex-wrap: wrap; }
  /* Ignore the saved column count — multi-column cards are unreadable this narrow. */
  .node-list { grid-template-columns: 1fr; }
  .editor-row { flex-direction: column; }
  .editor-type { max-width: none; }
}
</style>
