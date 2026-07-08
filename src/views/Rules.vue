<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";

const { t } = useI18n();
import {
  Plus, Trash2, ChevronDown, ChevronUp,
  Filter, Globe, Layers, BookMarked, Info, RotateCcw, GripVertical
} from "@lucide/vue";
import EmptyState from "../components/EmptyState.vue";
import ToggleSwitch from "../components/ToggleSwitch.vue";
import { useFeedbackStore } from "../stores/feedback";

const fb = useFeedbackStore();

// ─── Types ───────────────────────────────────────────────────────────

type RuleAction = "proxy" | "direct" | "block" | "dns";

interface RouteRule {
  id: string;
  name: string;
  enabled: boolean;
  action: RuleAction;
  domain: string[];
  domain_suffix: string[];
  domain_keyword: string[];
  geosite: string[];
  geoip: string[];
  ip_cidr: string[];
  port: string[];
  network: string | null;
  process_name: string[];
}

interface RuleProvider {
  id: string;
  name: string;
  url: string;
  action: RuleAction;
  enabled: boolean;
  format: string;
}

const rules = ref<RouteRule[]>([]);
const loading = ref(false);
const expandedId = ref<string | null>(null);
const showAddDialog = ref(false);
const saving = ref(false);

// ─── Remote rule-set providers ───────────────────────────────────────
const providers = ref<RuleProvider[]>([]);
const showProviderDialog = ref(false);
const newProviderName = ref("");
const newProviderUrl = ref("");
const newProviderAction = ref<RuleAction>("proxy");

// ─── Add dialog state ────────────────────────────────────────────────

const newRule = ref<RouteRule>({
  id: "",
  name: "",
  enabled: true,
  action: "proxy",
  domain: [],
  domain_suffix: [],
  domain_keyword: [],
  geosite: [],
  geoip: [],
  ip_cidr: [],
  port: [],
  network: null,
  process_name: [],
});

// Temporary textarea values (comma/newline separated)
const newDomain = ref("");
const newDomainSuffix = ref("");
const newDomainKeyword = ref("");
const newGeosite = ref("");
const newGeoip = ref("");
const newIpCidr = ref("");
const newPort = ref("");
const newProcessName = ref("");

function splitInput(s: string): string[] {
  return s
    .split(/[\s,，\n]+/)
    .map((v) => v.trim())
    .filter(Boolean);
}

// ─── Presets ─────────────────────────────────────────────────────────

const presets = [
  { labelKey: "presetBlockAds",     action: "block"  as RuleAction, geosite: ["category-ads-all"] },
  { labelKey: "presetPrivateDirect",action: "direct" as RuleAction, geoip: ["private"] },
  { labelKey: "presetChinaDirect",  action: "direct" as RuleAction, geosite: ["cn"], geoip: ["cn"] },
  { labelKey: "presetTelegram",     action: "proxy"  as RuleAction, geosite: ["telegram"], geoip: ["telegram"] },
  { labelKey: "presetGoogle",       action: "proxy"  as RuleAction, geosite: ["google"], geoip: ["google"] },
  { labelKey: "presetYoutube",      action: "proxy"  as RuleAction, geosite: ["youtube"] },
  { labelKey: "presetGithub",       action: "proxy"  as RuleAction, geosite: ["github"] },
  { labelKey: "presetTwitter",      action: "proxy"  as RuleAction, geosite: ["twitter"], geoip: ["twitter"] },
  { labelKey: "presetNetflix",      action: "proxy"  as RuleAction, geosite: ["netflix"], domain_suffix: ["netflix.com", "nflxvideo.net", "nflximg.net"] },
  { labelKey: "presetAiServices",   action: "proxy"  as RuleAction, domain_suffix: ["openai.com", "chatgpt.com", "anthropic.com", "claude.ai"] },
  { labelKey: "presetSteam",        action: "proxy"  as RuleAction, geosite: ["steam"] },
  { labelKey: "presetAppleDirect",  action: "direct" as RuleAction, geosite: ["apple"] },
  { labelKey: "presetMicrosoftDirect",action: "direct" as RuleAction, geosite: ["microsoft"] },
  { labelKey: "presetBilibiliDirect",action: "direct" as RuleAction, geosite: ["bilibili"] },
];

// ─── Computed ────────────────────────────────────────────────────────

const actionColor: Record<RuleAction, string> = {
  proxy: "badge-blue",
  direct: "badge-green",
  block: "badge-red",
  dns: "badge-gray",
};

const actionLabelKey: Record<RuleAction, string> = {
  proxy: "actionProxy",
  direct: "actionDirect",
  block: "actionBlock",
  dns: "actionDns",
};
function actionLabel(action: RuleAction): string {
  return t(`rules.${actionLabelKey[action]}`);
}

function matchSummary(rule: RouteRule): string {
  const parts: string[] = [];
  if (rule.geosite.length) parts.push(`GeoSite: ${rule.geosite.join(", ")}`);
  if (rule.geoip.length) parts.push(`GeoIP: ${rule.geoip.join(", ")}`);
  if (rule.domain.length) parts.push(`${t("rules.summaryDomain")}: ${rule.domain.slice(0, 2).join(", ")}${rule.domain.length > 2 ? "…" : ""}`);
  if (rule.domain_suffix.length) parts.push(`${t("rules.summarySuffix")}: ${rule.domain_suffix.slice(0, 2).join(", ")}${rule.domain_suffix.length > 2 ? "…" : ""}`);
  if (rule.domain_keyword.length) parts.push(`${t("rules.summaryKeyword")}: ${rule.domain_keyword.join(", ")}`);
  if (rule.ip_cidr.length) parts.push(`IP: ${rule.ip_cidr.join(", ")}`);
  if (rule.port.length) parts.push(`${t("rules.summaryPort")}: ${rule.port.join(", ")}`);
  if (rule.process_name.length) parts.push(`${t("rules.summaryProcess")}: ${rule.process_name.join(", ")}`);
  return parts.join(" · ") || t("rules.summaryEmpty");
}

// ─── Actions ─────────────────────────────────────────────────────────

async function load() {
  loading.value = true;
  try {
    rules.value = await invoke<RouteRule[]>("cmd_get_rules");
  } finally {
    loading.value = false;
  }
}

async function toggleRule(id: string) {
  rules.value = await invoke<RouteRule[]>("cmd_toggle_rule", { id });
}

async function deleteRule(id: string) {
  if (!(await fb.confirm({ message: t("rules.confirmDeleteRule"), danger: true }))) return;
  rules.value = await invoke<RouteRule[]>("cmd_delete_rule", { id });
  if (expandedId.value === id) expandedId.value = null;
}

async function saveAll() {
  saving.value = true;
  try {
    await invoke("cmd_save_rules", { rules: rules.value });
  } finally {
    saving.value = false;
  }
}

async function resetToDefault() {
  if (!(await fb.confirm({ message: t("rules.confirmReset"), danger: true }))) return;
  rules.value = await invoke<RouteRule[]>("cmd_reset_rules");
}

async function applyPreset(preset: typeof presets[0]) {
  const rule: RouteRule = {
    id: crypto.randomUUID(),
    name: t(`rules.${preset.labelKey}`),
    enabled: true,
    action: preset.action,
    domain: [],
    domain_suffix: (preset as any).domain_suffix ?? [],
    domain_keyword: [],
    geosite: (preset as any).geosite ?? [],
    geoip: (preset as any).geoip ?? [],
    ip_cidr: [],
    port: [],
    network: null,
    process_name: [],
  };
  rules.value = await invoke<RouteRule[]>("cmd_add_rule", { rule });
}

function resetNewRule() {
  newRule.value = {
    id: "", name: "", enabled: true, action: "proxy",
    domain: [], domain_suffix: [], domain_keyword: [],
    geosite: [], geoip: [], ip_cidr: [], port: [], network: null, process_name: [],
  };
  newDomain.value = "";
  newDomainSuffix.value = "";
  newDomainKeyword.value = "";
  newGeosite.value = "";
  newGeoip.value = "";
  newIpCidr.value = "";
  newPort.value = "";
  newProcessName.value = "";
}

async function addRule() {
  const rule: RouteRule = {
    ...newRule.value,
    id: crypto.randomUUID(),
    domain: splitInput(newDomain.value),
    domain_suffix: splitInput(newDomainSuffix.value),
    domain_keyword: splitInput(newDomainKeyword.value),
    geosite: splitInput(newGeosite.value),
    geoip: splitInput(newGeoip.value),
    ip_cidr: splitInput(newIpCidr.value),
    port: splitInput(newPort.value),
    process_name: splitInput(newProcessName.value),
  };
  if (!rule.name.trim()) return;
  rules.value = await invoke<RouteRule[]>("cmd_add_rule", { rule });
  showAddDialog.value = false;
  resetNewRule();
}

function moveUp(index: number) {
  if (index <= 0) return;
  [rules.value[index - 1], rules.value[index]] = [rules.value[index], rules.value[index - 1]];
}
function moveDown(index: number) {
  if (index >= rules.value.length - 1) return;
  [rules.value[index], rules.value[index + 1]] = [rules.value[index + 1], rules.value[index]];
}

// ─── Drag-to-reorder ────────────────────────────────────────────────
// Rules match top-down, so order = priority. Dragging a row to a new position is a more
// ergonomic alternative to the up/down buttons. Like those, it only mutates the local
// list — the new order is applied to the core when the user clicks "Save & Apply".
const dragIndex = ref<number | null>(null);
const dragOverIndex = ref<number | null>(null);

function onDragStart(index: number) {
  dragIndex.value = index;
}
function onDragOver(index: number) {
  // Must run on dragover (and preventDefault in the template) for drop to fire.
  dragOverIndex.value = index;
}
function onDrop(target: number) {
  const from = dragIndex.value;
  dragIndex.value = null;
  dragOverIndex.value = null;
  if (from === null || from === target) return;
  const [moved] = rules.value.splice(from, 1);
  rules.value.splice(target, 0, moved);
}
function onDragEnd() {
  dragIndex.value = null;
  dragOverIndex.value = null;
}

// ─── Rule provider actions ───────────────────────────────────────────
async function loadProviders() {
  providers.value = await invoke<RuleProvider[]>("cmd_get_rule_providers");
}

async function addProvider() {
  const name = newProviderName.value.trim();
  const url = newProviderUrl.value.trim();
  if (!name || !url) return;
  providers.value = await invoke<RuleProvider[]>("cmd_add_rule_provider", {
    name,
    url,
    action: newProviderAction.value,
  });
  showProviderDialog.value = false;
  newProviderName.value = "";
  newProviderUrl.value = "";
  newProviderAction.value = "proxy";
}

async function deleteProvider(id: string) {
  if (!(await fb.confirm({ message: t("rules.confirmDeleteProvider"), danger: true }))) return;
  providers.value = await invoke<RuleProvider[]>("cmd_delete_rule_provider", { id });
}

async function toggleProvider(id: string) {
  providers.value = await invoke<RuleProvider[]>("cmd_toggle_rule_provider", { id });
}

onMounted(() => {
  load();
  loadProviders();
});
</script>

<template>
  <div class="page">
    <div class="page-header">
      <div>
        <h1 class="page-title">{{ t("rules.pageTitle") }}</h1>
        <p class="page-subtitle">{{ t("rules.pageSubtitle") }}</p>
      </div>
      <div class="header-actions">
        <button class="btn btn-ghost" @click="resetToDefault" :title="t('rules.resetTooltip')">
          <RotateCcw :size="13" />
          {{ t("rules.resetDefault") }}
        </button>
        <button class="btn btn-ghost" @click="showAddDialog = true">
          <Plus :size="14" />
          {{ t("rules.addRule") }}
        </button>
        <button class="btn btn-primary" :disabled="saving" @click="saveAll">
          {{ saving ? t("rules.saving") : t("rules.saveApply") }}
        </button>
      </div>
    </div>

    <!-- Preset Quick Add -->
    <div class="card preset-card">
      <div class="preset-label">
        <BookMarked :size="13" />
        <span>{{ t("rules.quickAddPreset") }}</span>
      </div>
      <div class="preset-list">
        <button
          v-for="p in presets"
          :key="p.labelKey"
          class="preset-btn"
          :class="`preset-${p.action}`"
          @click="applyPreset(p)"
        >
          <span class="preset-action">{{ actionLabel(p.action) }}</span>
          {{ t(`rules.${p.labelKey}`) }}
        </button>
      </div>
    </div>

    <!-- Remote rule-set providers -->
    <div class="card preset-card">
      <div class="provider-head">
        <div class="preset-label">
          <Globe :size="13" />
          <span>{{ t("rules.remoteRuleSet") }}</span>
        </div>
        <button class="btn btn-ghost btn-sm" @click="showProviderDialog = true">
          <Plus :size="13" />
          {{ t("rules.addRuleSet") }}
        </button>
      </div>
      <p class="provider-hint">
        {{ t("rules.providerHint") }}
      </p>
      <div v-if="providers.length === 0" class="provider-empty">{{ t("rules.noRemoteRuleSet") }}</div>
      <div v-else class="provider-list">
        <div v-for="p in providers" :key="p.id" class="provider-item" :class="{ disabled: !p.enabled }">
          <span class="toggle-cell" :title="p.enabled ? t('rules.clickDisable') : t('rules.clickEnable')">
            <ToggleSwitch :model-value="p.enabled" @update:model-value="toggleProvider(p.id)" />
          </span>
          <div class="provider-info">
            <div class="provider-name">
              {{ p.name }}
              <span class="provider-badge" :class="`preset-${p.action}`">{{ actionLabel(p.action) }}</span>
              <span class="provider-fmt">{{ p.format === "source" ? "JSON" : "SRS" }}</span>
            </div>
            <div class="provider-url" :title="p.url">{{ p.url }}</div>
          </div>
          <button class="icon-btn danger" :title="t('rules.delete')" @click="deleteProvider(p.id)">
            <Trash2 :size="14" />
          </button>
        </div>
      </div>
    </div>

    <!-- Add provider dialog -->
    <div v-if="showProviderDialog" class="card add-dialog">
      <div class="dialog-title">{{ t("rules.addRemoteRuleSet") }}</div>
      <div class="form-group">
        <label class="form-label">{{ t("rules.name") }}</label>
        <input class="input" v-model="newProviderName" :placeholder="t('rules.namePlaceholder')" @keyup.enter="addProvider" />
      </div>
      <div class="form-group">
        <label class="form-label">{{ t("rules.ruleSetUrl") }}</label>
        <input class="input" v-model="newProviderUrl" :placeholder="t('rules.ruleSetUrlPlaceholder')" @keyup.enter="addProvider" />
      </div>
      <div class="form-group">
        <label class="form-label">{{ t("rules.hitAction") }}</label>
        <select class="input" v-model="newProviderAction">
          <option value="proxy">{{ t("rules.actionProxy") }}</option>
          <option value="direct">{{ t("rules.actionDirect") }}</option>
          <option value="block">{{ t("rules.actionBlock") }}</option>
        </select>
      </div>
      <div class="dialog-actions">
        <button class="btn btn-ghost" @click="showProviderDialog = false">{{ t("rules.cancel") }}</button>
        <button class="btn btn-primary" @click="addProvider">{{ t("rules.add") }}</button>
      </div>
    </div>

    <!-- Rules List -->
    <EmptyState
      v-if="rules.length === 0 && !loading"
      :icon="Filter"
      :title="t('rules.emptyTitle')"
      :desc="t('rules.emptyDesc')"
    />

    <div class="rules-list">
      <div
        v-for="(rule, index) in rules"
        :key="rule.id"
        class="card rule-item"
        :class="{
          disabled: !rule.enabled,
          dragging: dragIndex === index,
          'drag-over': dragOverIndex === index && dragIndex !== index,
        }"
        draggable="true"
        @dragstart="onDragStart(index)"
        @dragover.prevent="onDragOver(index)"
        @drop="onDrop(index)"
        @dragend="onDragEnd"
      >
        <!-- Rule Header -->
        <div class="rule-header" @click="expandedId = expandedId === rule.id ? null : rule.id">
          <GripVertical :size="14" class="drag-handle" :title="t('rules.dragHint')" @click.stop />
          <div class="rule-order">{{ index + 1 }}</div>
          <span
            class="toggle-cell"
            :title="rule.enabled ? t('rules.clickDisable') : t('rules.clickEnable')"
            @click.stop
          >
            <ToggleSwitch :model-value="rule.enabled" @update:model-value="toggleRule(rule.id)" />
          </span>
          <div class="rule-info">
            <div class="rule-name">{{ rule.name }}</div>
            <div class="rule-summary">{{ matchSummary(rule) }}</div>
          </div>
          <span class="badge" :class="actionColor[rule.action]">
            {{ actionLabel(rule.action) }}
          </span>
          <div class="rule-controls">
            <button class="icon-btn" :title="t('rules.moveUp')" @click.stop="moveUp(index)">
              <ChevronUp :size="14" />
            </button>
            <button class="icon-btn" :title="t('rules.moveDown')" @click.stop="moveDown(index)">
              <ChevronDown :size="14" />
            </button>
            <button class="icon-btn danger" :title="t('rules.delete')" @click.stop="deleteRule(rule.id)">
              <Trash2 :size="13" />
            </button>
          </div>
        </div>

        <!-- Expanded Detail -->
        <Transition name="expand">
          <div v-if="expandedId === rule.id" class="rule-detail">
            <div class="detail-grid">
              <template v-if="rule.geosite.length">
                <div class="detail-key"><Globe :size="12" /> GeoSite</div>
                <div class="detail-val">{{ rule.geosite.join(", ") }}</div>
              </template>
              <template v-if="rule.geoip.length">
                <div class="detail-key"><Layers :size="12" /> GeoIP</div>
                <div class="detail-val">{{ rule.geoip.join(", ") }}</div>
              </template>
              <template v-if="rule.domain.length">
                <div class="detail-key">{{ t("rules.detailDomain") }}</div>
                <div class="detail-val">{{ rule.domain.join(", ") }}</div>
              </template>
              <template v-if="rule.domain_suffix.length">
                <div class="detail-key">{{ t("rules.detailDomainSuffix") }}</div>
                <div class="detail-val">{{ rule.domain_suffix.join(", ") }}</div>
              </template>
              <template v-if="rule.domain_keyword.length">
                <div class="detail-key">{{ t("rules.detailDomainKeyword") }}</div>
                <div class="detail-val">{{ rule.domain_keyword.join(", ") }}</div>
              </template>
              <template v-if="rule.ip_cidr.length">
                <div class="detail-key">IP / CIDR</div>
                <div class="detail-val">{{ rule.ip_cidr.join(", ") }}</div>
              </template>
              <template v-if="rule.port.length">
                <div class="detail-key">{{ t("rules.detailPort") }}</div>
                <div class="detail-val">{{ rule.port.join(", ") }}</div>
              </template>
              <template v-if="rule.network">
                <div class="detail-key">{{ t("rules.detailNetwork") }}</div>
                <div class="detail-val">{{ rule.network }}</div>
              </template>
              <template v-if="rule.process_name.length">
                <div class="detail-key">{{ t("rules.detailProcess") }}</div>
                <div class="detail-val">{{ rule.process_name.join(", ") }}</div>
              </template>
            </div>
          </div>
        </Transition>
      </div>
    </div>

    <!-- Add Rule Dialog -->
    <div v-if="showAddDialog" class="dialog-overlay" @click.self="showAddDialog = false; resetNewRule()">
      <div class="dialog card-strong">
        <div class="dialog-title">
          <Plus :size="16" />
          {{ t("rules.addRuleDialogTitle") }}
        </div>

        <div class="dialog-form">
          <div class="form-row">
            <div class="form-group half">
              <label class="form-label">{{ t("rules.ruleName") }}</label>
              <input class="input" v-model="newRule.name" :placeholder="t('rules.ruleNamePlaceholder')" />
            </div>
            <div class="form-group half">
              <label class="form-label">{{ t("rules.action") }}</label>
              <select class="input" v-model="newRule.action">
                <option value="proxy">{{ t("rules.actionProxy") }}</option>
                <option value="direct">{{ t("rules.actionDirect") }}</option>
                <option value="block">{{ t("rules.actionBlock") }}</option>
                <option value="dns">{{ t("rules.actionDnsHandle") }}</option>
              </select>
            </div>
          </div>

          <div class="form-section-title">
            <Info :size="12" />
            {{ t("rules.matchConditionsHint") }}
          </div>

          <div class="form-row">
            <div class="form-group half">
              <label class="form-label">{{ t("rules.geositeLabel") }}</label>
              <input class="input" v-model="newGeosite" placeholder="cn, google, telegram" />
            </div>
            <div class="form-group half">
              <label class="form-label">{{ t("rules.geoipLabel") }}</label>
              <input class="input" v-model="newGeoip" placeholder="cn, private, telegram" />
            </div>
          </div>
          <div class="form-row">
            <div class="form-group half">
              <label class="form-label">{{ t("rules.domainExact") }}</label>
              <textarea class="input textarea-sm" v-model="newDomain" placeholder="example.com&#10;another.com" />
            </div>
            <div class="form-group half">
              <label class="form-label">{{ t("rules.domainSuffix") }}</label>
              <textarea class="input textarea-sm" v-model="newDomainSuffix" placeholder=".netflix.com&#10;.google.com" />
            </div>
          </div>
          <div class="form-row">
            <div class="form-group half">
              <label class="form-label">{{ t("rules.domainKeyword") }}</label>
              <input class="input" v-model="newDomainKeyword" placeholder="google, openai" />
            </div>
            <div class="form-group half">
              <label class="form-label">IP / CIDR</label>
              <input class="input" v-model="newIpCidr" placeholder="192.168.0.0/16, 10.0.0.0/8" />
            </div>
          </div>
          <div class="form-row">
            <div class="form-group half">
              <label class="form-label">{{ t("rules.portLabel") }}</label>
              <input class="input" v-model="newPort" placeholder="80, 443, 8080-8090" />
            </div>
            <div class="form-group half">
              <label class="form-label">{{ t("rules.processNameLabel") }}</label>
              <input class="input" v-model="newProcessName" placeholder="chrome.exe, steam.exe" />
            </div>
          </div>
          <div class="form-row">
            <div class="form-group half">
              <label class="form-label">{{ t("rules.networkProtocol") }}</label>
              <select class="input" v-model="newRule.network">
                <option :value="null">{{ t("rules.networkAny") }}</option>
                <option value="tcp">TCP</option>
                <option value="udp">UDP</option>
              </select>
            </div>
          </div>
        </div>

        <div class="dialog-actions">
          <button class="btn btn-ghost" @click="showAddDialog = false; resetNewRule()">{{ t("rules.cancel") }}</button>
          <button class="btn btn-primary" @click="addRule" :disabled="!newRule.name.trim()">
            <Plus :size="13" />
            {{ t("rules.addRule") }}
          </button>
        </div>
      </div>
    </div>

    <!-- Rule order note -->
    <div class="order-note">
      <Info :size="12" />
      {{ t("rules.orderNote") }}
    </div>
  </div>
</template>

<style scoped>
/* Page primitives (.page/.page-header/.page-title/.page-subtitle/.header-actions)
   are provided globally by main.css. */

/* Presets */
.preset-card { padding: 12px 16px; display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
.preset-label {
  display: flex; align-items: center; gap: 5px;
  font-size: 11px; font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.4px; white-space: nowrap;
}
.preset-list { display: flex; gap: 6px; flex-wrap: wrap; }
.preset-btn {
  display: flex; align-items: center; gap: 5px;
  padding: 4px 12px; border-radius: 100px;
  border: 1px solid var(--color-border);
  background: var(--color-surface-strong);
  font-size: 12px; cursor: pointer; transition: all 0.15s;
  color: var(--color-text-secondary);
}
.preset-btn:hover { box-shadow: var(--shadow-sm); color: var(--color-text); }
.preset-action {
  font-size: 10px; font-weight: 700; padding: 1px 5px;
  border-radius: 3px; margin-right: 2px;
}
.preset-proxy .preset-action { background: var(--color-primary-soft); color: var(--color-primary); }
.preset-direct .preset-action { background: var(--color-success-soft); color: var(--color-success); }
.preset-block .preset-action { background: var(--color-error-soft); color: var(--color-error); }

/* Rules List */
.rules-list { display: flex; flex-direction: column; gap: 6px; }
.rule-item {
  overflow: hidden;
  transition: border-color 0.15s ease-out, box-shadow 0.15s ease-out, transform 0.15s ease-out;
}
.rule-item:hover {
  border-color: var(--color-neutral-strong);
  box-shadow: var(--shadow-sm), var(--edge-highlight);
  transform: translateY(-1px);
}
.rule-item.disabled { opacity: 0.55; }
.rule-item.dragging { opacity: 0.4; }
.rule-item.drag-over { box-shadow: 0 -2px 0 0 var(--color-primary) inset, 0 0 0 1px var(--color-primary); transform: none; }
.drag-handle {
  color: var(--color-text-muted); cursor: grab; flex-shrink: 0;
  opacity: 0.5; transition: opacity 0.12s;
}
.drag-handle:hover { opacity: 1; }
.rule-item:active .drag-handle { cursor: grabbing; }
.rule-header {
  display: flex; align-items: center; gap: 10px;
  padding: 12px 14px; cursor: pointer;
  transition: background 0.15s ease-out;
}
.rule-header:hover { background: var(--color-neutral-soft); }
.rule-order {
  width: 22px; height: 22px; border-radius: 50%;
  background: var(--color-neutral-strong);
  display: flex; align-items: center; justify-content: center;
  font-size: 11px; font-weight: 700; color: var(--color-text-muted);
  flex-shrink: 0;
}
.toggle-cell { display: flex; align-items: center; flex-shrink: 0; }
.rule-info { flex: 1; min-width: 0; }
.rule-name { font-size: 13px; font-weight: 600; }
.rule-summary { font-size: 11px; color: var(--color-text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; margin-top: 1px; }
.rule-controls { display: flex; gap: 2px; flex-shrink: 0; }
.icon-btn {
  width: 28px; height: 28px; border: none; background: transparent;
  border-radius: var(--radius-sm); cursor: pointer;
  display: flex; align-items: center; justify-content: center;
  color: var(--color-text-muted); transition: all 0.15s ease-out;
}
.icon-btn:hover { background: var(--color-neutral); color: var(--color-text); }
.icon-btn.danger:hover { background: var(--color-error-soft); color: var(--color-error); }

/* Expanded detail */
.rule-detail {
  padding: 10px 14px 14px;
  border-top: 1px solid var(--color-border);
  background: var(--color-neutral-soft);
}
.detail-grid { display: grid; grid-template-columns: 120px 1fr; gap: 6px 12px; font-size: 12px; }
.detail-key {
  display: flex; align-items: center; gap: 4px;
  color: var(--color-text-secondary); font-weight: 500; padding-top: 1px;
}
.detail-val { color: var(--color-text); font-family: 'Cascadia Code', monospace; word-break: break-all; }
.expand-enter-active, .expand-leave-active { transition: all 0.2s ease; max-height: 300px; overflow: hidden; }
.expand-enter-from, .expand-leave-to { max-height: 0; opacity: 0; padding: 0 14px; }

/* Add Dialog */
.dialog-overlay {
  position: fixed; inset: 0; z-index: var(--z-modal);
  background: rgba(0,0,0,0.5);
  display: flex; align-items: center; justify-content: center;
  padding: 24px;
  backdrop-filter: blur(4px);
  animation: overlayIn 0.2s ease-out;
}
.dialog {
  width: 100%; max-width: 640px; max-height: 80vh; overflow-y: auto;
  padding: 24px;
  display: flex; flex-direction: column; gap: 16px;
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg), var(--edge-highlight);
  animation: dialogIn 0.22s ease-out;
}
@keyframes overlayIn {
  from { opacity: 0; }
  to { opacity: 1; }
}
@keyframes dialogIn {
  from { opacity: 0; transform: scale(0.97) translateY(6px); }
  to { opacity: 1; transform: scale(1) translateY(0); }
}
.dialog-title { display: flex; align-items: center; gap: 8px; font-size: 16px; font-weight: 700; }
.dialog-form { display: flex; flex-direction: column; gap: 10px; }
.form-section-title {
  display: flex; align-items: center; gap: 6px;
  font-size: 11px; font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.4px; padding: 4px 0;
  border-bottom: 1px solid var(--color-border);
}
.form-row { display: flex; gap: 12px; }
.form-group { display: flex; flex-direction: column; gap: 5px; }
.form-group.half { flex: 1; }
.form-label {
  font-size: 11px; font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.5px;
}
.textarea-sm { min-height: 60px; resize: vertical; }
.dialog-actions { display: flex; gap: 8px; justify-content: flex-end; padding-top: 4px; }

.order-note {
  display: flex; align-items: flex-start; gap: 6px;
  font-size: 11px; color: var(--color-text-muted); line-height: 1.5;
}

/* ─── Remote rule-set providers ─── */
.provider-head { display: flex; align-items: center; justify-content: space-between; }
.btn-sm { padding: 3px 10px !important; font-size: 12px; }
.provider-hint { font-size: 12px; color: var(--color-text-muted); margin: 6px 0 10px; line-height: 1.5; }
.provider-empty { font-size: 12px; color: var(--color-text-muted); padding: 8px 0; }
.provider-list { display: flex; flex-direction: column; gap: 8px; }
.provider-item {
  display: flex; align-items: center; gap: 10px;
  padding: 8px 10px; border: 1px solid var(--color-border);
  border-radius: var(--radius-md); background: var(--color-neutral-soft);
  transition: border-color 0.15s ease-out, background 0.15s ease-out;
}
.provider-item:hover { border-color: var(--color-neutral-strong); background: var(--color-neutral); }
.provider-item.disabled { opacity: 0.5; }
.provider-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.provider-name { font-size: 13px; font-weight: 600; display: flex; align-items: center; gap: 6px; }
.provider-url {
  font-size: 11px; color: var(--color-text-muted);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.provider-badge {
  font-size: 10px; font-weight: 600; padding: 1px 7px; border-radius: 100px;
}
.provider-badge.preset-proxy { background: var(--color-primary-soft); color: var(--color-primary); }
.provider-badge.preset-direct { background: var(--color-success-soft); color: var(--color-success); }
.provider-badge.preset-block { background: var(--color-error-soft); color: var(--color-error); }
.provider-fmt {
  font-size: 10px; font-weight: 600; padding: 1px 6px; border-radius: var(--radius-sm);
  background: var(--color-neutral-strong); color: var(--color-text-secondary);
}

.add-dialog { padding: 20px; display: flex; flex-direction: column; gap: 14px; margin-bottom: 4px; }
</style>
