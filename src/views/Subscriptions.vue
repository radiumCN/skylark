<script setup lang="ts">
import { ref, nextTick, onMounted } from "vue";
import {
  Plus, RefreshCw, Trash2, QrCode, Clock, Server, AlertCircle, X, Copy, Check as CheckIcon, Zap, Filter
} from "@lucide/vue";
import QRCode from "qrcode";
import { useI18n } from "vue-i18n";
import { useAppStore } from "../stores/app";
import { useFeedbackStore } from "../stores/feedback";
import { formatBytes } from "../utils/format";
import { copyToClipboard, readFromClipboard } from "../utils/clipboard";
import { useTemporaryFlag } from "../composables/useTemporaryFlag";
import EmptyState from "../components/EmptyState.vue";
import Skeleton from "../components/Skeleton.vue";
import ToggleSwitch from "../components/ToggleSwitch.vue";

const { t } = useI18n();
const store = useAppStore();
const fb = useFeedbackStore();

onMounted(() => {
  // Shared poller keeps the active auto group's current node fresh for the badge below.
  store.ensureActiveNowPoller();
});

const showAddDialog = ref(false);
const addMode = ref<"url" | "text">("url");
const newSubName = ref("");
const newSubUrl = ref("");
const newSubContent = ref("");
const newSubInclude = ref("");
const newSubExclude = ref("");
const newSubGroupRegion = ref(false);
const addLoading = ref(false);
const addError = ref("");
const updatingId = ref<string | null>(null);

const formatDate = (iso?: string) => {
  if (!iso) return t("subscriptions.never");
  const d = new Date(iso);
  const now = new Date();
  const diff = now.getTime() - d.getTime();
  if (diff < 60000) return t("subscriptions.justNow");
  if (diff < 3600000) return t("subscriptions.minutesAgo", { n: Math.floor(diff / 60000) });
  if (diff < 86400000) return t("subscriptions.hoursAgo", { n: Math.floor(diff / 3600000) });
  return d.toLocaleDateString();
};

const subTypeLabel = (type: string) => {
  const map: Record<string, string> = { clash: "Clash", v2ray: "V2Ray", sip008: "SIP008", unknown: t("subscriptions.unknown") };
  return map[type] ?? type;
};

// ─── Airport usage / quota (from Subscription-Userinfo header) ──────────────
interface SubLike {
  upload?: number;
  download?: number;
  total?: number;
  expire?: number;
}

// Used = upload + download (bytes). Returns null when neither is known.
function usedBytes(sub: SubLike): number | null {
  if (sub.upload == null && sub.download == null) return null;
  return (sub.upload ?? 0) + (sub.download ?? 0);
}

// Show a usage row only when there is something meaningful to show.
function hasQuota(sub: SubLike): boolean {
  return usedBytes(sub) != null || sub.total != null || sub.expire != null;
}

// Percentage of quota used (0–100), or null when total is unknown/zero.
function usagePercent(sub: SubLike): number | null {
  const used = usedBytes(sub);
  if (used == null || !sub.total || sub.total <= 0) return null;
  return Math.min(100, Math.round((used / sub.total) * 100));
}

function usageColor(pct: number | null): string {
  if (pct == null) return "var(--color-primary)";
  if (pct >= 90) return "var(--color-error)";
  if (pct >= 70) return "var(--color-attention)";
  return "var(--color-primary)";
}

// Format the expiry timestamp; flags expired / imminent for emphasis in the template.
function formatExpire(expire?: number): { text: string; expired: boolean } {
  if (expire == null) return { text: "--", expired: false };
  const d = new Date(expire * 1000);
  const expired = d.getTime() <= Date.now();
  return { text: d.toLocaleDateString(), expired };
}

async function addSub() {
  const name = newSubName.value.trim();
  if (!name) {
    addError.value = t("subscriptions.errNameRequired");
    return;
  }
  if (addMode.value === "url" && !newSubUrl.value.trim()) {
    addError.value = t("subscriptions.errUrlRequired");
    return;
  }
  if (addMode.value === "text" && !newSubContent.value.trim()) {
    addError.value = t("subscriptions.errContentRequired");
    return;
  }
  addLoading.value = true;
  addError.value = "";
  try {
    const filters = {
      include: newSubInclude.value.trim() || null,
      exclude: newSubExclude.value.trim() || null,
      groupByRegion: newSubGroupRegion.value,
    };
    if (addMode.value === "url") {
      await store.addSubscription(name, newSubUrl.value.trim(), filters);
    } else {
      await store.importSubscriptionFromText(name, newSubContent.value.trim(), filters);
    }
    cancelAdd();
  } catch (e) {
    addError.value = String(e);
  } finally {
    addLoading.value = false;
  }
}

async function pasteContent() {
  const text = await readFromClipboard();
  if (text != null) {
    newSubContent.value = text;
  }
  // Clipboard read may be blocked — user can paste manually.
}

async function updateSub(id: string) {
  updatingId.value = id;
  try {
    await store.updateSubscription(id);
  } finally {
    updatingId.value = null;
  }
}

async function deleteSub(id: string, name: string) {
  if (await fb.confirm({ message: t("subscriptions.confirmDelete", { name }), danger: true })) {
    await store.deleteSubscription(id);
  }
}

const refreshingAll = ref(false);
async function refreshAll() {
  if (store.subscriptions.length === 0) return;
  refreshingAll.value = true;
  try {
    await Promise.allSettled(store.subscriptions.map((s) => store.updateSubscription(s.id)));
  } finally {
    refreshingAll.value = false;
  }
}

function cancelAdd() {
  showAddDialog.value = false;
  addMode.value = "url";
  newSubName.value = "";
  newSubUrl.value = "";
  newSubContent.value = "";
  newSubInclude.value = "";
  newSubExclude.value = "";
  newSubGroupRegion.value = false;
  addError.value = "";
}

// ─── Node filter editor (per existing subscription) ─────────────────
const filterVisible = ref(false);
const filterSubId = ref("");
const filterSubName = ref("");
const filterInclude = ref("");
const filterExclude = ref("");
const filterGroupRegion = ref(false);
const filterSaving = ref(false);
const filterError = ref("");

function openFilter(sub: {
  id: string; name: string;
  include?: string | null; exclude?: string | null; group_by_region?: boolean;
}) {
  filterSubId.value = sub.id;
  filterSubName.value = sub.name;
  filterInclude.value = sub.include ?? "";
  filterExclude.value = sub.exclude ?? "";
  filterGroupRegion.value = sub.group_by_region ?? false;
  filterError.value = "";
  filterVisible.value = true;
}

function closeFilter() {
  filterVisible.value = false;
}

async function saveFilter() {
  filterSaving.value = true;
  filterError.value = "";
  try {
    await store.setSubscriptionFilters(
      filterSubId.value,
      filterInclude.value.trim() || null,
      filterExclude.value.trim() || null,
      filterGroupRegion.value
    );
    filterVisible.value = false;
  } catch (e) {
    filterError.value = String(e);
  } finally {
    filterSaving.value = false;
  }
}

// ─── QR Code share dialog ──────────────────────────────────────────
const qrVisible = ref(false);
const qrSubName = ref("");
const qrSubUrl = ref("");
const qrDataUrl = ref("");
const { flag: qrCopied, trigger: triggerQrCopied } = useTemporaryFlag(1800);

async function showQr(name: string, url: string) {
  qrSubName.value = name;
  qrSubUrl.value = url;
  qrVisible.value = true;
  await nextTick();
  try {
    qrDataUrl.value = await QRCode.toDataURL(url, {
      width: 240,
      margin: 2,
      color: { dark: "#1a1a1a", light: "#ffffff" },
      errorCorrectionLevel: "M",
    });
  } catch {
    qrDataUrl.value = "";
  }
}

function closeQr() {
  qrVisible.value = false;
  qrCopied.value = false;
  qrDataUrl.value = "";
}

async function copyUrl() {
  const ok = await copyToClipboard(qrSubUrl.value);
  if (ok) {
    triggerQrCopied();
  }
  // else: fallback — show the URL selected
}

const INTERVAL_OPTIONS = [
  { value: 1,  label: t("subscriptions.every1h") },
  { value: 3,  label: t("subscriptions.every3h") },
  { value: 6,  label: t("subscriptions.every6h") },
  { value: 12, label: t("subscriptions.every12h") },
  { value: 24, label: t("subscriptions.every24h") },
  { value: 72, label: t("subscriptions.every3d") },
];

function formatNextUpdate(sub: { last_update?: string; update_interval: number }) {
  if (!sub.last_update) return t("subscriptions.notUpdatedYet");
  const next = new Date(sub.last_update).getTime() + sub.update_interval * 3600 * 1000;
  const diff = next - Date.now();
  if (diff <= 0) return t("subscriptions.updatingSoon");
  const h = Math.floor(diff / 3600000);
  const m = Math.floor((diff % 3600000) / 60000);
  return h > 0 ? t("subscriptions.inHm", { h, m }) : t("subscriptions.inM", { m });
}

async function toggleAutoUpdate(id: string, currentAutoUpdate: boolean, interval: number) {
  await store.saveSubscriptionSettings(id, !currentAutoUpdate, interval);
}

async function changeInterval(id: string, autoUpdate: boolean, interval: number) {
  await store.saveSubscriptionSettings(id, autoUpdate, interval);
}
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">{{ t("subscriptions.title") }}</h1>
      <div class="header-actions">
        <button
          v-if="store.subscriptions.length > 0"
          class="btn btn-ghost"
          :disabled="refreshingAll"
          @click="refreshAll"
        >
          <RefreshCw :size="13" :class="{ spin: refreshingAll }" />
          {{ refreshingAll ? t("subscriptions.updating") : t("subscriptions.refreshAll") }}
        </button>
        <button class="btn btn-primary" @click="showAddDialog = true">
          <Plus :size="14" />
          {{ t("subscriptions.add") }}
        </button>
      </div>
    </div>

    <!-- Cold-start skeleton (before the first data load resolves) -->
    <div
      v-if="store.subscriptions.length === 0 && !showAddDialog && !store.initialized"
      class="sub-skel-list"
    >
      <div v-for="i in 3" :key="i" class="card sub-skel">
        <div class="sub-skel-main">
          <Skeleton width="38%" height="14px" />
          <Skeleton width="62%" height="10px" />
        </div>
        <Skeleton width="72px" height="24px" radius="var(--radius-md)" />
      </div>
    </div>

    <!-- Empty State -->
    <EmptyState
      v-else-if="store.subscriptions.length === 0 && !showAddDialog"
      :icon="Server"
      :title="t('subscriptions.empty')"
      :desc="t('subscriptions.emptyDesc')"
    >
      <button class="btn btn-primary" @click="showAddDialog = true">
        <Plus :size="14" />
        {{ t("subscriptions.addFirst") }}
      </button>
    </EmptyState>

    <!-- Add Dialog (inline) -->
    <div v-if="showAddDialog" class="card add-dialog">
      <div class="dialog-title">{{ t("subscriptions.add") }}</div>
      <div class="segmented mode-tabs">
        <button class="segmented__item" :class="{ active: addMode === 'url' }" @click="addMode = 'url'">
          {{ t("subscriptions.modeUrl") }}
        </button>
        <button class="segmented__item" :class="{ active: addMode === 'text' }" @click="addMode = 'text'">
          {{ t("subscriptions.modeText") }}
        </button>
      </div>
      <div class="form-group">
        <label class="form-label">{{ t("subscriptions.nameLabel") }}</label>
        <input
          class="input"
          v-model="newSubName"
          :placeholder="t('subscriptions.namePlaceholder')"
          @keyup.enter="addSub"
        />
      </div>
      <div v-if="addMode === 'url'" class="form-group">
        <label class="form-label">{{ t("subscriptions.urlLabel") }}</label>
        <input
          class="input"
          v-model="newSubUrl"
          :placeholder="t('subscriptions.urlPlaceholder')"
          @keyup.enter="addSub"
        />
      </div>
      <div v-else class="form-group">
        <div class="form-label-row">
          <label class="form-label">{{ t("subscriptions.contentLabel") }}</label>
          <button class="btn btn-ghost paste-btn" @click="pasteContent">
            <Copy :size="12" />
            {{ t("subscriptions.pasteFromClipboard") }}
          </button>
        </div>
        <textarea
          class="input content-area"
          v-model="newSubContent"
          rows="6"
          :placeholder="t('subscriptions.contentPlaceholder')"
        />
      </div>
      <details class="filter-fold">
        <summary class="filter-summary">{{ t("subscriptions.filterFold") }}</summary>
        <div class="form-group">
          <label class="form-label">{{ t("subscriptions.includeLabel") }}</label>
          <input class="input" v-model="newSubInclude" :placeholder="t('subscriptions.includePlaceholder')" />
        </div>
        <div class="form-group">
          <label class="form-label">{{ t("subscriptions.excludeLabel") }}</label>
          <input class="input" v-model="newSubExclude" :placeholder="t('subscriptions.excludePlaceholder')" />
        </div>
        <label class="filter-check">
          <input type="checkbox" v-model="newSubGroupRegion" />
          {{ t("subscriptions.groupByRegion") }}
        </label>
      </details>
      <div v-if="addError" class="form-error">
        <AlertCircle :size="13" />
        {{ addError }}
      </div>
      <div class="dialog-actions">
        <button class="btn btn-ghost" @click="cancelAdd">{{ t("subscriptions.cancel") }}</button>
        <button class="btn btn-primary" :disabled="addLoading" @click="addSub">
          <RefreshCw v-if="addLoading" :size="13" class="spin" />
          {{ addLoading ? (addMode === "url" ? t("subscriptions.fetching") : t("subscriptions.importing")) : (addMode === "url" ? t("subscriptions.confirmAdd") : t("subscriptions.confirmImport")) }}
        </button>
      </div>
    </div>

    <!-- Subscription List -->
    <div class="sub-list">
      <div v-for="sub in store.subscriptions" :key="sub.id" class="card sub-item">
        <div class="sub-main">
          <div class="sub-left">
            <div class="sub-name">{{ sub.name }}</div>
            <div class="sub-meta">
              <span class="badge badge-blue">{{ subTypeLabel(sub.sub_type) }}</span>
              <span class="meta-item">
                <Server :size="11" />
                {{ t("subscriptions.nodeCount", { n: sub.node_count }) }}
              </span>
              <span class="meta-item">
                <Clock :size="11" />
                {{ formatDate(sub.last_update) }}
              </span>
              <span
                v-if="store.activeProxyTag === `auto-${sub.id}`"
                class="meta-item auto-hit"
                :title="t('subscriptions.autoHitTitle')"
              >
                <Zap :size="11" />
                {{ t("subscriptions.autoArrow") }} {{ store.activeNodeNow ?? t("subscriptions.selecting") }}
              </span>
            </div>
            <div class="sub-url">{{ sub.url || t("subscriptions.localImport") }}</div>
          </div>
          <div class="sub-actions">
            <button
              v-if="sub.url"
              class="btn btn-ghost icon-btn"
              :disabled="updatingId === sub.id"
              :title="t('subscriptions.updateTitle')"
              @click="updateSub(sub.id)"
            >
              <RefreshCw :size="14" :class="{ spin: updatingId === sub.id }" />
            </button>
            <button
              v-if="sub.url"
              class="btn btn-ghost icon-btn"
              :title="t('subscriptions.qrShareTitle')"
              @click="showQr(sub.name, sub.url)"
            >
              <QrCode :size="14" />
            </button>
            <button
              class="btn btn-ghost icon-btn"
              :class="{ 'filter-active': sub.include || sub.exclude || sub.group_by_region }"
              :title="t('subscriptions.filterTitle')"
              @click="openFilter(sub)"
            >
              <Filter :size="14" />
            </button>
            <button
              class="btn btn-ghost icon-btn danger"
              :title="t('subscriptions.delete')"
              @click="deleteSub(sub.id, sub.name)"
            >
              <Trash2 :size="14" />
            </button>
          </div>
        </div>

        <!-- Airport usage / quota (only when the provider reports it) -->
        <div v-if="hasQuota(sub)" class="sub-quota">
          <div class="quota-line">
            <span class="quota-text">
              {{ t("subscriptions.used") }} {{ formatBytes(usedBytes(sub) ?? undefined) }}
              <template v-if="sub.total != null"> / {{ formatBytes(sub.total) }}</template>
            </span>
            <span
              v-if="sub.expire != null"
              class="quota-expire"
              :class="{ expired: formatExpire(sub.expire).expired }"
            >
              {{ formatExpire(sub.expire).expired ? t("subscriptions.expired") : t("subscriptions.expireOn") }} {{ formatExpire(sub.expire).text }}
            </span>
          </div>
          <div v-if="usagePercent(sub) != null" class="quota-bar">
            <div
              class="quota-fill"
              :style="{ width: usagePercent(sub) + '%', background: usageColor(usagePercent(sub)) }"
            />
          </div>
        </div>

        <!-- Auto-update row (URL subscriptions only) -->
        <div v-if="sub.url" class="sub-autoupdate">
          <div class="autoupdate-left">
            <Clock :size="12" class="autoupdate-icon" />
            <span class="autoupdate-label">{{ t("subscriptions.autoUpdate") }}</span>
            <span v-if="sub.auto_update" class="autoupdate-next">
              {{ formatNextUpdate(sub) }}
            </span>
          </div>
          <div class="autoupdate-right">
            <select
              v-if="sub.auto_update"
              class="interval-select"
              :value="sub.update_interval"
              @change="changeInterval(sub.id, sub.auto_update, Number(($event.target as HTMLSelectElement).value))"
            >
              <option v-for="opt in INTERVAL_OPTIONS" :key="opt.value" :value="opt.value">
                {{ opt.label }}
              </option>
            </select>
            <ToggleSwitch
              :model-value="sub.auto_update"
              :aria-label="sub.auto_update ? t('subscriptions.autoUpdateOff') : t('subscriptions.autoUpdateOn')"
              @update:model-value="toggleAutoUpdate(sub.id, sub.auto_update, sub.update_interval)"
            />
          </div>
        </div>
      </div>
    </div>

    <!-- QR Code dialog -->
    <Teleport to="body">
      <Transition name="qr-fade">
        <div v-if="qrVisible" class="qr-overlay" @click.self="closeQr">
          <div class="qr-dialog">
            <div class="qr-header">
              <div class="qr-title">
                <QrCode :size="15" />
                {{ qrSubName }}
              </div>
              <button class="qr-close" @click="closeQr">
                <X :size="16" />
              </button>
            </div>

            <div class="qr-body">
              <div class="qr-image-wrap">
                <img v-if="qrDataUrl" :src="qrDataUrl" alt="QR Code" class="qr-image" />
                <div v-else class="qr-placeholder">{{ t("subscriptions.generating") }}</div>
              </div>

              <div class="qr-desc">{{ t("subscriptions.qrScanHint") }}</div>

              <div class="qr-url-row">
                <span class="qr-url-text">{{ qrSubUrl }}</span>
                <button class="btn btn-ghost qr-copy-btn" @click="copyUrl" :title="qrCopied ? t('subscriptions.copied') : t('subscriptions.copyLink')">
                  <CheckIcon v-if="qrCopied" :size="13" class="copy-ok" />
                  <Copy v-else :size="13" />
                </button>
              </div>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- Node filter / grouping dialog -->
    <Teleport to="body">
      <Transition name="qr-fade">
        <div v-if="filterVisible" class="qr-overlay" @click.self="closeFilter">
          <div class="qr-dialog filter-dialog">
            <div class="qr-header">
              <div class="qr-title">
                <Filter :size="15" />
                {{ t("subscriptions.filterTitle") }} — {{ filterSubName }}
              </div>
              <button class="qr-close" @click="closeFilter">
                <X :size="16" />
              </button>
            </div>
            <div class="filter-body">
              <div class="form-group">
                <label class="form-label">{{ t("subscriptions.includeLabel") }}</label>
                <input class="input" v-model="filterInclude" :placeholder="t('subscriptions.includePlaceholder')" />
              </div>
              <div class="form-group">
                <label class="form-label">{{ t("subscriptions.excludeLabel") }}</label>
                <input class="input" v-model="filterExclude" :placeholder="t('subscriptions.excludePlaceholder')" />
              </div>
              <label class="filter-check">
                <input type="checkbox" v-model="filterGroupRegion" />
                {{ t("subscriptions.groupByRegion") }}
              </label>
              <div class="filter-note">{{ t("subscriptions.filterNote") }}</div>
              <div v-if="filterError" class="form-error">
                <AlertCircle :size="13" />
                {{ filterError }}
              </div>
              <div class="dialog-actions">
                <button class="btn btn-ghost" @click="closeFilter">{{ t("subscriptions.cancel") }}</button>
                <button class="btn btn-primary" :disabled="filterSaving" @click="saveFilter">
                  <RefreshCw v-if="filterSaving" :size="13" class="spin" />
                  {{ filterSaving ? t("subscriptions.applying") : t("subscriptions.saveApply") }}
                </button>
              </div>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- Supported formats hint -->
    <div class="hint-card card">
      <div class="hint-title">{{ t("subscriptions.hintTitle") }}</div>
      <div class="hint-list">
        <div class="hint-item">
          <span class="badge badge-blue">Clash</span>
          <span>{{ t("subscriptions.hintClash") }}</span>
        </div>
        <div class="hint-item">
          <span class="badge badge-green">V2Ray</span>
          <span>{{ t("subscriptions.hintV2ray") }}</span>
        </div>
        <div class="hint-item">
          <span class="badge badge-yellow">SIP008</span>
          <span>{{ t("subscriptions.hintSip008") }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.add-dialog { padding: 20px; display: flex; flex-direction: column; gap: 14px; }
.dialog-title { font-size: 15px; font-weight: 600; }
.form-group { display: flex; flex-direction: column; gap: 6px; }
.form-label { font-size: 12px; font-weight: 500; color: var(--color-text-secondary); }
.form-label-row { display: flex; align-items: center; justify-content: space-between; }
.paste-btn { padding: 2px 8px !important; font-size: 11px; }
.content-area {
  resize: vertical; min-height: 110px;
  font-family: 'Cascadia Code', monospace; font-size: 12px; line-height: 1.5;
}

/* Stretch the shared segmented control to fill the dialog width. */
.mode-tabs { display: flex; }
.mode-tabs .segmented__item { flex: 1; justify-content: center; }
.form-error {
  display: flex; align-items: center; gap: 6px;
  font-size: 12px; color: var(--color-error);
}
.dialog-actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 4px; }

.sub-list { display: flex; flex-direction: column; gap: 10px; }
.sub-skel-list { display: flex; flex-direction: column; gap: 10px; }
.sub-skel {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-4);
}
.sub-skel-main { flex: 1; display: flex; flex-direction: column; gap: var(--space-2); }
.sub-item {
  padding: 0;
  display: flex; flex-direction: column;
  border-radius: var(--radius-lg);
  transition: box-shadow 0.15s ease-out, border-color 0.15s ease-out, transform 0.15s ease-out;
  overflow: hidden;
}
.sub-item:hover {
  box-shadow: var(--shadow-md), var(--edge-highlight);
  border-color: var(--color-primary-soft);
  transform: translateY(-1px);
}
.sub-main {
  display: flex; align-items: flex-start; justify-content: space-between;
  gap: 12px; padding: 16px 18px;
}
.sub-left { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
.sub-name { font-size: 14px; font-weight: 600; }
.sub-meta { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.meta-item {
  display: flex; align-items: center; gap: 3px;
  font-size: 11px; color: var(--color-text-secondary);
}
.meta-item.auto-hit {
  color: var(--accent-amber); font-weight: 500;
  background: var(--color-warning-soft); padding: 1px 7px; border-radius: 100px;
}
.sub-url {
  font-size: 11px; color: var(--color-text-muted);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 500px;
}
.sub-actions { display: flex; gap: 4px; flex-shrink: 0; }
.icon-btn { padding: 6px !important; transition: color 0.15s ease-out, background 0.15s ease-out; }
.icon-btn.danger:hover { color: var(--color-error); background: var(--color-error-soft); }

/* Airport usage / quota */
.sub-quota {
  display: flex; flex-direction: column; gap: 6px;
  padding: 8px 18px;
  border-top: 1px solid var(--color-border);
}
.quota-line {
  display: flex; align-items: center; justify-content: space-between; gap: 8px;
  font-size: 11px; color: var(--color-text-secondary);
}
.quota-text { font-weight: 500; }
.quota-expire {
  font-size: 11px; color: var(--color-text-muted);
  background: var(--color-neutral); padding: 1px 7px; border-radius: 100px;
}
.quota-expire.expired { color: var(--color-error); background: var(--color-error-soft); font-weight: 600; }
.quota-bar {
  height: 5px; border-radius: 100px; overflow: hidden;
  background: var(--color-neutral-strong);
}
.quota-fill { height: 100%; border-radius: 100px; transition: width 0.3s ease-out; }

/* Auto-update row */
.sub-autoupdate {
  display: flex; align-items: center; justify-content: space-between;
  padding: 8px 18px;
  background: var(--color-neutral-soft);
  border-top: 1px solid var(--color-border);
}
.autoupdate-left {
  display: flex; align-items: center; gap: 6px;
  font-size: 12px; color: var(--color-text-secondary);
}
.autoupdate-icon { color: var(--color-text-muted); flex-shrink: 0; }
.autoupdate-label { font-weight: 500; }
.autoupdate-next {
  font-size: 11px; color: var(--color-primary);
  background: var(--color-primary-soft); padding: 1px 7px; border-radius: 100px;
}
.autoupdate-right { display: flex; align-items: center; gap: 8px; }

.interval-select {
  font-size: 12px; padding: 2px 6px; border-radius: var(--radius-sm);
  border: 1px solid var(--color-border); background: var(--color-surface-strong);
  color: var(--color-text); cursor: pointer; outline: none;
  transition: border-color 0.15s ease-out, box-shadow 0.15s ease-out;
}
.interval-select:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-soft);
}

.hint-card { padding: 16px 18px; display: flex; flex-direction: column; gap: 10px; }
.hint-title { font-size: 11px; font-weight: 600; color: var(--color-text-secondary); text-transform: uppercase; letter-spacing: 0.5px; }
.hint-list { display: flex; flex-direction: column; gap: 8px; }
.hint-item { display: flex; align-items: center; gap: 10px; font-size: 12px; color: var(--color-text-secondary); }

/* ─── QR Code dialog ─── */
.qr-overlay {
  position: fixed; inset: 0; z-index: var(--z-modal);
  background: rgba(0,0,0,0.55);
  backdrop-filter: blur(3px);
  display: flex; align-items: center; justify-content: center;
}
.qr-dialog {
  background: var(--color-surface-strong);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg), var(--edge-highlight);
  width: 320px;
  overflow: hidden;
}
.qr-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 14px 16px 12px;
  border-bottom: 1px solid var(--color-border);
}
.qr-title {
  display: flex; align-items: center; gap: 7px;
  font-size: 14px; font-weight: 600;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  max-width: 240px;
}
.qr-close {
  background: none; border: none; cursor: pointer;
  color: var(--color-text-muted); padding: 4px; border-radius: var(--radius-sm);
  display: flex; align-items: center;
  transition: color 0.15s, background 0.15s;
}
.qr-close:hover { color: var(--color-text); background: var(--color-neutral); }

.qr-body {
  padding: 20px 24px 20px;
  display: flex; flex-direction: column; align-items: center; gap: 14px;
}
.qr-image-wrap {
  background: white;
  border-radius: var(--radius-md);
  padding: 8px;
  box-shadow: var(--shadow-md);
}
.qr-image { display: block; width: 200px; height: 200px; }
.qr-placeholder {
  width: 200px; height: 200px;
  display: flex; align-items: center; justify-content: center;
  font-size: 12px; color: var(--color-text-muted);
}
.qr-desc {
  font-size: 12px; color: var(--color-text-muted); text-align: center;
}
.qr-url-row {
  display: flex; align-items: center; gap: 6px; width: 100%;
  background: var(--color-neutral-soft); border: 1px solid var(--color-border);
  border-radius: var(--radius-md); padding: 6px 8px 6px 10px;
}
.qr-url-text {
  flex: 1; min-width: 0;
  font-size: 11px; font-family: 'Cascadia Code', monospace;
  color: var(--color-text-secondary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.qr-copy-btn { padding: 4px 6px !important; flex-shrink: 0; }
.copy-ok { color: var(--color-success); }

/* QR dialog enter/leave transition */
.qr-fade-enter-active, .qr-fade-leave-active { transition: opacity 0.22s ease-out, transform 0.22s ease-out; }
.qr-fade-enter-from, .qr-fade-leave-to { opacity: 0; transform: scale(0.95); }

/* Node filter UI */
.filter-fold { margin: 4px 0 2px; border-top: 1px solid var(--color-border); padding-top: 8px; }
.filter-summary { cursor: pointer; font-size: 12px; color: var(--color-text-secondary); user-select: none; }
.filter-summary:hover { color: var(--color-text); }
.filter-check { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--color-text-secondary); cursor: pointer; margin-top: 4px; }
.filter-check input { width: 14px; height: 14px; }
.icon-btn.filter-active { color: var(--color-primary); }
.filter-dialog { width: 420px; max-width: 92vw; }
.filter-body { padding: 16px; display: flex; flex-direction: column; gap: 12px; }
.filter-note { font-size: 11.5px; color: var(--color-text-secondary); line-height: 1.5; }
</style>
