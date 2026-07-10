<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, computed, nextTick } from "vue";
import {
  Shield, Globe, Cpu, Monitor, Download,
  RefreshCw, CheckCircle, AlertCircle, Package, ExternalLink,
  ShieldCheck, ShieldAlert, Check, Rocket, Zap, Save, Upload, Archive, Layers, Trash2,
  FolderOpen
} from "@lucide/vue";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { useI18n } from "vue-i18n";
import { useAppStore, type AppConfig } from "../stores/app";
import { useFeedbackStore } from "../stores/feedback";
import { setLocale } from "../i18n";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { formatBytes } from "../utils/format";
import { useTemporaryFlag } from "../composables/useTemporaryFlag";
import ToggleSwitch from "../components/ToggleSwitch.vue";
import Select from "../components/Select.vue";

const store = useAppStore();
const fb = useFeedbackStore();
const { t } = useI18n();

// Any link surviving sanitization in a (remote, untrusted) release note must never navigate
// the app's own webview in place — that would replace the UI with attacker-controlled content
// in the privileged Tauri context. Force every anchor to open externally and drop the
// opener's access to `window`. Registered once at module load (idempotent across renders).
DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A" && node.hasAttribute("href")) {
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noopener noreferrer");
  }
});

// Render release notes (GitHub release bodies) as Markdown. The source is remote, so the
// generated HTML is always run through DOMPurify before it reaches v-html — never trust the
// payload. Parse synchronously and treat any failure as empty so a malformed note can't break
// the settings page.
function renderReleaseNotes(md: string | null | undefined): string {
  if (!md) return "";
  try {
    const html = marked.parse(md, { async: false, breaks: true, gfm: true }) as string;
    return DOMPurify.sanitize(html, { USE_PROFILES: { html: true } });
  } catch {
    return "";
  }
}

// Apply a language change to the UI immediately; the debounced config save then persists it.
function onLanguageChange() {
  setLocale(localConfig.value.language);
}

// ─── Network diagnostics (N5) ──────────────────────────────────────────
interface ProbeResult { name: string; ok: boolean; latency_ms?: number | null; }
interface DiagnosticsResult {
  outbound_ip?: string | null;
  country?: string | null;
  city?: string | null;
  isp?: string | null;
  probes: ProbeResult[];
}
const diagResult = ref<DiagnosticsResult | null>(null);
const diagnosing = ref(false);
const diagError = ref("");

async function runDiagnostics() {
  diagnosing.value = true;
  diagError.value = "";
  diagResult.value = null;
  try {
    diagResult.value = await invoke<DiagnosticsResult>("cmd_run_diagnostics");
  } catch (e) {
    diagError.value = String(e);
  } finally {
    diagnosing.value = false;
  }
}

const diagLocation = computed(() => {
  const r = diagResult.value;
  if (!r) return "";
  return [r.city, r.country].filter(Boolean).join(", ");
});

// ─── Config profiles (N6) ──────────────────────────────────────────────
const profiles = ref<string[]>([]);
const newProfileName = ref("");
const profileBusy = ref(false);

async function refreshProfiles() {
  profiles.value = await store.listProfiles();
}
async function saveCurrentProfile() {
  const name = newProfileName.value.trim();
  if (!name) return;
  profileBusy.value = true;
  try {
    await store.saveProfile(name);
    newProfileName.value = "";
    await refreshProfiles();
  } catch (e) {
    fb.toastError(String(e));
  } finally {
    profileBusy.value = false;
  }
}
async function switchProfile(name: string) {
  profileBusy.value = true;
  try {
    await store.loadProfile(name);
    localConfig.value = { ...store.config };
    fb.toastSuccess(t("settings.profileSwitched", { name }));
  } catch (e) {
    fb.toastError(String(e));
  } finally {
    profileBusy.value = false;
  }
}
async function removeProfile(name: string) {
  if (!(await fb.confirm({ message: t("settings.confirmDeleteProfile", { name }), danger: true }))) return;
  await store.deleteProfile(name);
  await refreshProfiles();
}
const { flag: saved, trigger: triggerSaved } = useTemporaryFlag(1500);
const appVersion = ref("");
const localConfig = ref<AppConfig>({ ...store.config });

// ─── Section jump nav (additive only — no existing state/handler touched) ──
// One entry per section, in DOM order. Titles reuse the existing i18n keys so
// the chip bar stays localized.
const sections = [
  { id: "sec-update", key: "settings.appUpdate" },
  { id: "sec-kernel", key: "settings.kernelManagement" },
  { id: "sec-system", key: "settings.systemBehavior" },
  { id: "sec-ports", key: "settings.portConfig" },
  { id: "sec-subscription", key: "settings.subscription" },
  { id: "sec-dns", key: "settings.dnsAndNetwork" },
  { id: "sec-diagnostics", key: "settings.diagnostics" },
  { id: "sec-profiles", key: "settings.profiles" },
  { id: "sec-backup", key: "settings.configBackup" },
  { id: "sec-tun", key: "settings.tunMode" },
  { id: "sec-autoselect", key: "settings.autoSelect" },
  { id: "sec-advanced", key: "settings.advancedSettings" },
] as const;
const activeSection = ref<string>(sections[0].id);
let sectionObserver: IntersectionObserver | null = null;

// Click-to-scroll; sections carry scroll-margin-top so they clear the sticky bar.
function scrollToSection(id: string) {
  document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });
}

// Scroll-spy: highlight the top-most section currently sitting just below the
// sticky chip bar. Additive lifecycle hooks keep this isolated from the existing
// onMounted/onUnmounted logic.
onMounted(() => {
  sectionObserver = new IntersectionObserver(
    (entries) => {
      const visible = entries
        .filter((e) => e.isIntersecting)
        .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
      if (visible.length > 0) {
        activeSection.value = (visible[0].target as HTMLElement).id;
      }
    },
    { rootMargin: "-72px 0px -55% 0px", threshold: 0 }
  );
  for (const s of sections) {
    const el = document.getElementById(s.id);
    if (el) sectionObserver.observe(el);
  }
});

onUnmounted(() => {
  sectionObserver?.disconnect();
  sectionObserver = null;
});

// Platform detection (WinTun is a Windows-only requirement).
const isWindows = /win/i.test(navigator.userAgent);
const isMacOS = /mac/i.test(navigator.userAgent);
const kernelBinaryName = isWindows ? "sing-box.exe" : "sing-box";

let saveTimer: ReturnType<typeof setTimeout> | null = null;
// Set while we revert localConfig from store.config after a save error. The deep watcher
// fires on any reassignment (new object identity), so without this guard the revert would
// re-arm scheduleSave and, for a persistently-failing save, spin an infinite save/toast loop.
let reverting = false;

function scheduleSave() {
  if (reverting) return;
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      await store.saveConfig({ ...localConfig.value });
      triggerSaved();
    } catch (e) {
      // The backend rejected the config (e.g. an out-of-range/duplicate port). Surface it
      // instead of silently swallowing — and re-sync the form from the persisted truth so
      // the UI can't keep displaying a value that was never saved. Guard the watcher so this
      // revert doesn't re-trigger a save.
      fb.toastError(String(e));
      reverting = true;
      localConfig.value = { ...store.config };
      await nextTick();
      reverting = false;
    }
  }, 600);
}

watch(localConfig, scheduleSave, { deep: true });

/**
 * Jump to the log directory in the system file manager. The backend returns the newest
 * log file (revealed = folder opens with it selected) or the folder itself when empty.
 * Mainly for macOS, where ~/Library/Application Support/Skylark/logs is hard to reach.
 */
async function openLogDir() {
  try {
    const path = await invoke<string>("cmd_get_log_dir_entry");
    await revealItemInDir(path);
  } catch (e) {
    fb.toastError(t('settings.openLogDirFailed', { e }));
  }
}

// ─── Config backup / restore ──────────────────────────────────────────

const exportingConfig = ref(false);
const importingConfig = ref(false);
const showImportModal = ref(false);
const importText = ref("");

/** Export the full setup to a JSON file and reveal it in the file manager. */
async function exportConfig() {
  exportingConfig.value = true;
  try {
    const path = await invoke<string>("cmd_export_config");
    try {
      await revealItemInDir(path);
    } catch {
      /** Reveal may be unavailable; the file is still written. */
    }
    fb.toastSuccess(t('settings.exportedTo', { path }));
  } catch (e) {
    fb.toastError(t('settings.exportFailed', { e }));
  } finally {
    exportingConfig.value = false;
  }
}

/** Restore from pasted backup JSON, then refresh all in-memory store state. */
async function importConfig() {
  const content = importText.value.trim();
  if (!content) {
    fb.toastInfo(t('settings.pasteBackupContent'));
    return;
  }
  if (!(await fb.confirm({ message: t('settings.importConfirm'), danger: true }))) return;
  importingConfig.value = true;
  try {
    await invoke("cmd_import_config", { content });
    await store.fetchConfig();
    localConfig.value = { ...store.config };
    await Promise.all([
      store.fetchSubscriptions(),
      store.fetchNodes(),
      store.fetchProxyGroups(),
    ]);
    showImportModal.value = false;
    importText.value = "";
    fb.toastSuccess(t('settings.importSuccess'));
  } catch (e) {
    fb.toastError(t('settings.importFailed', { e }));
  } finally {
    importingConfig.value = false;
  }
}

// ─── Kernel Updater ───────────────────────────────────────────────────

interface ReleaseInfo {
  version: string;
  published_at: string;
  release_notes: string;
  download_url: string;
  sha256?: string | null;
}

// ─── TUN / Admin ─────────────────────────────────────────────────────

const isAdmin = ref(false);
const wintunAvailable = ref(false);
const downloadingWintun = ref(false);
const wintunError = ref("");
// macOS: whether the one-time privileged TUN service is installed (passwordless sudo).
const tunServiceReady = ref(false);
const installingService = ref(false);
const serviceError = ref("");

async function refreshTunStatus() {
  if (isMacOS) {
    // macOS uses the privileged-service model: the GUI stays non-root, so report whether
    // the service is installed rather than whether the process is elevated.
    tunServiceReady.value = await invoke<boolean>("cmd_tun_service_installed");
    return;
  }
  [isAdmin.value, wintunAvailable.value] = await Promise.all([
    invoke<boolean>("cmd_is_elevated"),
    invoke<boolean>("cmd_wintun_available"),
  ]);
}

async function requestAdmin() {
  try {
    await invoke("cmd_relaunch_as_admin");
    // Process exits and re-opens elevated; if we reach here, user cancelled
  } catch (e) {
    // UAC cancelled - ignore
  }
}

// macOS: install the privileged TUN service. Prompts for the admin password ONCE; after
// that, enabling TUN needs no further authorization.
async function installTunService() {
  installingService.value = true;
  serviceError.value = "";
  try {
    await invoke("cmd_install_tun_service");
    tunServiceReady.value = await invoke<boolean>("cmd_tun_service_installed");
  } catch (e) {
    serviceError.value = String(e);
  } finally {
    installingService.value = false;
  }
}

async function downloadWintun() {
  downloadingWintun.value = true;
  wintunError.value = "";
  try {
    await invoke("cmd_download_wintun");
    wintunAvailable.value = true;
  } catch (e) {
    wintunError.value = String(e);
  } finally {
    downloadingWintun.value = false;
  }
}

// ─── Kernel Updater ───────────────────────────────────────────────────

const kernelExists = ref(false);
const installedVersion = ref<string | null>(null);
const latestRelease = ref<ReleaseInfo | null>(null);
const checkingUpdate = ref(false);
const checkError = ref("");
const downloading = ref(false);
const downloadProgress = ref(0);
const downloadedBytes = ref(0);
const totalBytes = ref(0);
const downloadError = ref("");
const downloadDone = ref(false);

const hasUpdate = computed(() => {
  if (!latestRelease.value || !installedVersion.value) return false;
  const installed = installedVersion.value.match(/(\d+\.\d+\.\d+)/)?.[1] ?? "";
  const latest = latestRelease.value.version.replace(/^v/, "");
  return installed !== latest && installed !== "";
});

const formatDate = (iso: string) => {
  if (!iso) return "";
  return new Date(iso).toLocaleDateString("zh-CN", {
    year: "numeric", month: "long", day: "numeric",
  });
};

async function refreshKernelStatus() {
  [kernelExists.value, installedVersion.value] = await Promise.all([
    invoke<boolean>("cmd_singbox_exists"),
    invoke<string | null>("cmd_get_installed_version"),
  ]);
}

async function checkUpdate(forceRefresh = false) {
  checkingUpdate.value = true;
  checkError.value = "";
  try {
    latestRelease.value = await invoke<ReleaseInfo>("cmd_check_singbox_update", { forceRefresh });
  } catch (e) {
    const msg = String(e);
    if (msg.includes("频率超限") || msg.includes("rate limit") || msg.includes("Rate limit")) {
      checkError.value = msg;
    } else {
      checkError.value = msg;
    }
  } finally {
    checkingUpdate.value = false;
  }
}

async function startDownload() {
  if (!latestRelease.value) return;
  downloading.value = true;
  downloadProgress.value = 0;
  downloadError.value = "";
  downloadDone.value = false;

  // Listen for progress events
  const unlistenProgress = await listen<{
    percent: number; downloaded: number; total: number;
  }>("singbox-download-progress", (event) => {
    downloadProgress.value = event.payload.percent;
    downloadedBytes.value = event.payload.downloaded;
    totalBytes.value = event.payload.total;
  });

  const unlistenDone = await listen<{ success: boolean; message: string }>(
    "singbox-download-done",
    async (event) => {
      if (event.payload.success) {
        downloadDone.value = true;
        downloadProgress.value = 100;
        await refreshKernelStatus();
      }
      unlistenProgress();
      unlistenDone();
    }
  );

  try {
    await invoke("cmd_download_singbox", {
      downloadUrl: latestRelease.value.download_url,
      sha256: latestRelease.value.sha256 ?? null,
    });
  } catch (e) {
    downloadError.value = String(e);
    unlistenProgress();
    unlistenDone();
  } finally {
    downloading.value = false;
  }
}

// ─── App Self-Updater ─────────────────────────────────────────────────

interface AppReleaseInfo {
  version: string;
  published_at: string;
  release_notes: string;
  download_url: string;
  is_prerelease: boolean;
  sha256?: string | null;
}

const appLatestRelease = ref<AppReleaseInfo | null>(null);
const checkingAppUpdate = ref(false);
const appCheckError = ref("");
const downloadingApp = ref(false);
const appDownloadProgress = ref(0);
const appDownloadedBytes = ref(0);
const appTotalBytes = ref(0);
const appDownloadDone = ref(false);
const appDownloadError = ref("");

// Compare two dotted numeric versions; true only if `latest` is strictly newer than
// `current`. Mirrors the Rust `is_newer_version` so a beta channel whose latest release
// is OLDER than the installed build (e.g. 0.3.4 vs 0.3.5) is not flagged as an update.
function isNewerVersion(latest: string, current: string): boolean {
  const parts = (v: string) =>
    v.trim().replace(/^v/, "").split("-")[0].split(".").map((s) => parseInt(s, 10) || 0);
  const a = parts(latest);
  const b = parts(current);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0;
    const y = b[i] ?? 0;
    if (x !== y) return x > y;
  }
  return false;
}

const hasAppUpdate = computed(() => {
  if (!appLatestRelease.value || !appVersion.value) return false;
  return isNewerVersion(appLatestRelease.value.version, appVersion.value);
});

async function checkAppUpdate(forceRefresh = false) {
  checkingAppUpdate.value = true;
  appCheckError.value = "";
  appLatestRelease.value = null;
  try {
    appLatestRelease.value = await invoke<AppReleaseInfo>("cmd_check_app_update", {
      channel: localConfig.value.update_channel ?? "stable",
      forceRefresh,
    });
  } catch (e) {
    appCheckError.value = String(e);
  } finally {
    checkingAppUpdate.value = false;
  }
}

// Switching the update channel (stable ↔ beta) must re-detect against the new
// channel — otherwise the previous channel's result lingers. checkAppUpdate already
// clears the prior result, and forceRefresh avoids returning the other channel's cache.
watch(() => localConfig.value.update_channel, () => {
  checkAppUpdate(true);
});

async function startAppDownload() {
  if (!appLatestRelease.value) return;
  downloadingApp.value = true;
  appDownloadProgress.value = 0;
  appDownloadError.value = "";
  appDownloadDone.value = false;

  const unlistenProgress = await listen<{ percent: number; downloaded: number; total: number }>(
    "app-download-progress",
    (event) => {
      appDownloadProgress.value = event.payload.percent;
      appDownloadedBytes.value = event.payload.downloaded;
      appTotalBytes.value = event.payload.total;
    }
  );

  const unlistenDone = await listen<{ success: boolean; message: string }>(
    "app-download-done",
    (event) => {
      if (event.payload.success) {
        appDownloadDone.value = true;
        appDownloadProgress.value = 100;
      }
      unlistenProgress();
      unlistenDone();
    }
  );

  try {
    await invoke("cmd_download_app_update", {
      downloadUrl: appLatestRelease.value.download_url,
      sha256: appLatestRelease.value.sha256 ?? null,
    });
  } catch (e) {
    appDownloadError.value = String(e);
    unlistenProgress();
    unlistenDone();
  } finally {
    downloadingApp.value = false;
  }
}

let unlistenAppUpdate: (() => void) | null = null;

onMounted(async () => {
  await Promise.all([
    refreshKernelStatus(),
    refreshTunStatus(),
    refreshProfiles(),
    getVersion().then((v) => (appVersion.value = v)),
  ]);

  // Force a fresh check on every entry so the result reflects the latest release
  // rather than a stale value left over from a previous visit (bypasses the 1-hour cache).
  checkAppUpdate(true);

  // Also listen for the background checker's event (fired ~45s after launch).
  unlistenAppUpdate = await listen<{
    version: string;
    download_url: string;
    release_notes: string;
    published_at: string;
    is_prerelease: boolean;
    current_version: string;
  }>("app-update-available", (event) => {
    // The background checker runs on the backend's default channel, which may differ from the
    // channel the user is currently viewing. Only let its result update this pane when the
    // release's prerelease-ness matches the selected channel, so a stale stable-channel event
    // can't overwrite the beta release the user explicitly checked (or vice-versa).
    const wantPrerelease = (localConfig.value.update_channel ?? "stable") === "beta";
    if (event.payload.is_prerelease !== wantPrerelease) return;
    appLatestRelease.value = {
      version: event.payload.version,
      published_at: event.payload.published_at,
      release_notes: event.payload.release_notes,
      download_url: event.payload.download_url,
      is_prerelease: event.payload.is_prerelease,
    };
  });
});

onUnmounted(() => {
  unlistenAppUpdate?.();
});
</script>

<template>
  <div class="page">
    <div class="page-header">
      <h1 class="page-title">{{ t('settings.title') }}</h1>
      <div class="header-actions">
        <Transition name="autosave">
          <span v-if="saved" class="autosave-badge">
            <Check :size="12" />{{ t('settings.saved') }}
          </span>
        </Transition>
      </div>
    </div>

    <!-- Sticky section-jump nav (additive) -->
    <nav class="section-nav" :aria-label="t('settings.title')">
      <button
        v-for="s in sections"
        :key="s.id"
        type="button"
        class="section-chip"
        :class="{ active: activeSection === s.id }"
        @click="scrollToSection(s.id)"
      >
        {{ t(s.key) }}
      </button>
    </nav>

    <!-- ─── 应用更新 ─── -->
    <section id="sec-update" class="settings-section">
      <div class="section-header">
        <Rocket :size="15" />
        <span>{{ t('settings.appUpdate') }}</span>
      </div>
      <div class="card settings-card kernel-card">

        <!-- Current app version + channel -->
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.currentVersion') }}</div>
            <div class="setting-desc">v{{ appVersion }}</div>
          </div>
          <div class="channel-select-wrap">
            <span class="channel-label">{{ t('settings.updateChannel') }}</span>
            <Select
              v-model="localConfig.update_channel"
              :options="[
                { value: 'stable', label: t('settings.channelStable') },
                { value: 'beta', label: t('settings.channelBeta') },
              ]"
              style="width:110px"
            />
          </div>
        </div>

        <!-- Latest release info -->
        <div v-if="appLatestRelease" class="release-info">
          <div class="release-header">
            <span class="release-tag">{{ appLatestRelease.version }}</span>
            <span class="release-date">{{ formatDate(appLatestRelease.published_at) }}</span>
            <span v-if="appLatestRelease.is_prerelease" class="badge badge-purple">{{ t('settings.preview') }}</span>
            <span v-if="hasAppUpdate" class="badge badge-yellow">{{ t('settings.hasNewVersion') }}</span>
            <span v-else class="badge badge-green">{{ t('settings.upToDate') }}</span>
          </div>
          <div
            v-if="appLatestRelease.release_notes"
            class="release-notes markdown-body"
            v-html="renderReleaseNotes(appLatestRelease.release_notes)"
          ></div>
        </div>

        <div class="setting-divider" />

        <!-- Action buttons -->
        <div class="kernel-actions">
          <button
            class="btn btn-ghost"
            :disabled="checkingAppUpdate"
            @click="() => checkAppUpdate(true)"
          >
            <RefreshCw :size="13" :class="{ spin: checkingAppUpdate }" />
            {{ checkingAppUpdate ? t('settings.checking') : t('settings.checkUpdate') }}
          </button>

          <button
            v-if="appLatestRelease && hasAppUpdate"
            class="btn btn-primary"
            :disabled="downloadingApp"
            @click="startAppDownload"
          >
            <Download :size="13" :class="{ spin: downloadingApp }" />
            {{ downloadingApp ? t('settings.downloading') : t('settings.downloadAndInstall') }}
          </button>

          <a
            class="btn btn-ghost"
            href="https://github.com/radiumCN/skylark/releases"
            target="_blank"
          >
            <ExternalLink :size="13" />
            {{ t('settings.viewAllVersions') }}
          </a>
        </div>

        <!-- Download progress -->
        <div v-if="downloadingApp || appDownloadDone" class="download-progress-area">
          <div class="progress-info">
            <span>{{ appDownloadDone ? t('settings.downloadDoneInstaller') : t('settings.downloadingPercent', { percent: appDownloadProgress.toFixed(1) }) }}</span>
            <span v-if="!appDownloadDone && appTotalBytes > 0" class="progress-bytes">
              {{ formatBytes(appDownloadedBytes) }} / {{ formatBytes(appTotalBytes) }}
            </span>
          </div>
          <div class="progress-bar-track">
            <div
              class="progress-bar-fill"
              :style="{ width: `${appDownloadProgress}%` }"
              :class="{ done: appDownloadDone }"
            />
          </div>
        </div>

        <!-- Errors -->
        <div v-if="appCheckError || appDownloadError" class="kernel-error">
          <AlertCircle :size="13" />
          {{ appCheckError || appDownloadError }}
        </div>

        <div class="kernel-hint">
          {{ t('settings.appUpdateHint') }}
        </div>
      </div>
    </section>

    <!-- ─── sing-box 内核管理 ─── -->
    <section id="sec-kernel" class="settings-section">
      <div class="section-header">
        <Package :size="15" />
        <span>{{ t('settings.kernelManagement') }}</span>
      </div>
      <div class="card settings-card kernel-card">

        <!-- Current status -->
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.currentKernelVersion') }}</div>
            <div class="setting-desc">
              <template v-if="installedVersion">{{ installedVersion }}</template>
              <span v-else-if="kernelExists" class="version-unknown">{{ t('settings.kernelVersionUnknown') }}</span>
              <span v-else class="not-installed">{{ t('settings.kernelNotInstalledDesc') }}</span>
            </div>
          </div>
          <div class="kernel-status">
            <span v-if="kernelExists" class="badge badge-green">
              <CheckCircle :size="11" /> {{ t('settings.installed') }}
            </span>
            <span v-else class="badge badge-red">
              <AlertCircle :size="11" /> {{ t('settings.notInstalled') }}
            </span>
          </div>
        </div>

        <!-- Latest release info -->
        <div v-if="latestRelease" class="release-info">
          <div class="release-header">
            <span class="release-tag">{{ latestRelease.version }}</span>
            <span class="release-date">{{ formatDate(latestRelease.published_at) }}</span>
            <span v-if="hasUpdate" class="badge badge-yellow">{{ t('settings.hasUpdate') }}</span>
            <span v-else-if="kernelExists" class="badge badge-green">{{ t('settings.upToDate') }}</span>
          </div>
          <div
            v-if="latestRelease.release_notes"
            class="release-notes markdown-body"
            v-html="renderReleaseNotes(latestRelease.release_notes)"
          ></div>
        </div>

        <div class="setting-divider" />

        <!-- Action buttons -->
        <div class="kernel-actions">
          <button
            class="btn btn-ghost"
            :disabled="checkingUpdate"
            @click="() => checkUpdate()"
          >
            <RefreshCw :size="13" :class="{ spin: checkingUpdate }" />
            {{ checkingUpdate ? t('settings.checking') : t('settings.checkUpdate') }}
          </button>

          <button
            v-if="latestRelease && (!kernelExists || hasUpdate)"
            class="btn btn-primary"
            :disabled="downloading"
            @click="startDownload"
          >
            <Download :size="13" :class="{ spin: downloading }" />
            {{ downloading ? t('settings.downloading') : kernelExists ? t('settings.updateKernel') : t('settings.downloadInstall') }}
          </button>

          <a
            class="btn btn-ghost"
            href="https://github.com/SagerNet/sing-box/releases"
            target="_blank"
            :title="t('settings.viewAllVersionsGithub')"
          >
            <ExternalLink :size="13" />
            GitHub Releases
          </a>
        </div>

        <!-- Download progress -->
        <div v-if="downloading || downloadDone" class="download-progress-area">
          <div class="progress-info">
            <span>{{ downloadDone ? t('settings.downloadDone') : t('settings.downloadingPercent', { percent: downloadProgress.toFixed(1) }) }}</span>
            <span v-if="!downloadDone && totalBytes > 0" class="progress-bytes">
              {{ formatBytes(downloadedBytes) }} / {{ formatBytes(totalBytes) }}
            </span>
          </div>
          <div class="progress-bar-track">
            <div
              class="progress-bar-fill"
              :style="{ width: `${downloadProgress}%` }"
              :class="{ done: downloadDone }"
            />
          </div>
        </div>

        <!-- Errors -->
        <div v-if="checkError || downloadError" class="kernel-error">
          <AlertCircle :size="13" />
          {{ checkError || downloadError }}
        </div>

        <div class="kernel-hint">
          {{ t('settings.kernelHintPart1') }}
          {{ t('settings.kernelHintPart2') }} <code>{{ kernelBinaryName }}</code> {{ t('settings.kernelHintPart3') }} <code>bin/</code> {{ t('settings.kernelHintPart4') }}
        </div>
      </div>
    </section>

    <!-- System Behavior -->
    <section id="sec-system" class="settings-section">
      <div class="section-header">
        <Monitor :size="15" />
        <span>{{ t('settings.systemBehavior') }}</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.startupWithSystem') }}</div>
            <div class="setting-desc">{{ t('settings.startupWithSystemDesc') }}</div>
          </div>
          <ToggleSwitch v-model="localConfig.startup_with_system" :aria-label="t('settings.startupWithSystem')" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.startupMinimized') }}</div>
            <div class="setting-desc">{{ t('settings.startupMinimizedDesc') }}</div>
          </div>
          <ToggleSwitch v-model="localConfig.startup_minimized" :aria-label="t('settings.startupMinimized')" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.allowLan') }}</div>
            <div class="setting-desc">{{ t('settings.allowLanDesc') }}</div>
          </div>
          <ToggleSwitch v-model="localConfig.allow_lan" :aria-label="t('settings.allowLan')" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.closeButtonBehavior') }}</div>
            <div class="setting-desc">
              {{ localConfig.close_to_tray ? t('settings.closeToTrayDesc') : t('settings.closeToExitDesc') }}
            </div>
          </div>
          <ToggleSwitch v-model="localConfig.close_to_tray" :aria-label="t('settings.closeButtonBehavior')" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.rememberProxyState') }}</div>
            <div class="setting-desc">{{ t('settings.rememberProxyStateDesc') }}</div>
          </div>
          <ToggleSwitch v-model="localConfig.restore_proxy_on_startup" :aria-label="t('settings.rememberProxyState')" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.logToFile') }}</div>
            <div class="setting-desc">{{ t('settings.logToFileDesc') }}</div>
          </div>
          <button class="btn btn-ghost btn-sm" @click="openLogDir">
            <FolderOpen :size="12" />
            {{ t('settings.openLogDir') }}
          </button>
          <ToggleSwitch v-model="localConfig.log_to_file" :aria-label="t('settings.logToFile')" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.enableGlobalShortcuts') }}</div>
            <div class="setting-desc">{{ t('settings.enableGlobalShortcutsDesc') }}</div>
          </div>
          <label class="toggle">
            <input
              type="checkbox"
              v-model="localConfig.enable_global_shortcuts"
              @change="store.applyGlobalShortcuts(localConfig.enable_global_shortcuts)"
            />
            <span class="toggle-track" />
          </label>
        </div>
      </div>
    </section>

    <!-- Ports -->
    <section id="sec-ports" class="settings-section">
      <div class="section-header">
        <Globe :size="15" />
        <span>{{ t('settings.portConfig') }}</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.httpPort') }}</div>
            <div class="setting-desc">{{ t('settings.httpPortDesc') }}</div>
          </div>
          <input class="input port-input" type="number" v-model.number="localConfig.http_port" min="1024" max="65535" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.socksPort') }}</div>
            <div class="setting-desc">{{ t('settings.socksPortDesc') }}</div>
          </div>
          <input class="input port-input" type="number" v-model.number="localConfig.socks_port" min="1024" max="65535" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.mixedPort') }}</div>
            <div class="setting-desc">{{ t('settings.mixedPortDesc') }}</div>
          </div>
          <input class="input port-input" type="number" v-model.number="localConfig.mixed_port" min="1024" max="65535" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.apiPort') }}</div>
            <div class="setting-desc">{{ t('settings.apiPortDesc') }}</div>
          </div>
          <input class="input port-input" type="number" v-model.number="localConfig.api_port" min="1024" max="65535" />
        </div>
      </div>
    </section>

    <!-- DNS / 网络 -->
    <!-- Subscription -->
    <section id="sec-subscription" class="settings-section">
      <div class="section-header">
        <RefreshCw :size="15" />
        <span>{{ t('settings.subscription') }}</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.subscriptionUserAgent') }}</div>
            <div class="setting-desc">
              {{ t('settings.subscriptionUserAgentDesc') }}
              {{ t('settings.subscriptionUserAgentCommon') }}<code>v2rayN/6.45</code>、<code>clash-verge/v2.0.0</code>、<code>ClashMetaForAndroid/2.10</code>、<code>sing-box/1.12</code>{{ t('settings.subscriptionUserAgentLeaveEmpty') }}
            </div>
          </div>
          <input
            class="input"
            type="text"
            v-model.trim="localConfig.subscription_user_agent"
            placeholder="v2rayN/6.45"
            style="width:220px"
          />
        </div>
      </div>
    </section>

    <section id="sec-dns" class="settings-section">
      <div class="section-header">
        <Globe :size="15" />
        <span>{{ t('settings.dnsAndNetwork') }}</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.domesticDnsResolver') }}</div>
            <div class="setting-desc">{{ t('settings.domesticDnsResolverDesc') }}</div>
          </div>
          <input class="input" type="text" v-model.trim="localConfig.dns_local" placeholder="223.5.5.5" style="width:220px" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.enableIpv6') }}</div>
            <div class="setting-desc">{{ t('settings.enableIpv6Desc') }}</div>
          </div>
          <ToggleSwitch v-model="localConfig.enable_ipv6" :aria-label="t('settings.enableIpv6')" />
        </div>
      </div>
    </section>

    <!-- Network diagnostics (N5) -->
    <section id="sec-diagnostics" class="settings-section">
      <div class="section-header">
        <Zap :size="15" />
        <span>{{ t('settings.diagnostics') }}</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.diagnostics') }}</div>
            <div class="setting-desc">{{ t('settings.diagnosticsDesc') }}</div>
          </div>
          <button class="btn btn-ghost" :disabled="diagnosing" @click="runDiagnostics">
            <RefreshCw v-if="diagnosing" :size="13" class="spin" />
            {{ diagnosing ? t('settings.diagnosing') : t('settings.runDiagnostics') }}
          </button>
        </div>
        <template v-if="diagError">
          <div class="setting-divider" />
          <div class="diag-error"><AlertCircle :size="13" /> {{ diagError }}</div>
        </template>
        <template v-if="diagResult">
          <div class="setting-divider" />
          <div class="diag-results">
            <div class="diag-line">
              <span class="diag-key">{{ t('settings.diagOutboundIp') }}</span>
              <span class="diag-val">{{ diagResult.outbound_ip ?? '--' }}</span>
            </div>
            <div class="diag-line" v-if="diagLocation">
              <span class="diag-key">{{ t('settings.diagLocation') }}</span>
              <span class="diag-val">{{ diagLocation }}</span>
            </div>
            <div class="diag-line" v-if="diagResult.isp">
              <span class="diag-key">{{ t('settings.diagIsp') }}</span>
              <span class="diag-val">{{ diagResult.isp }}</span>
            </div>
            <div class="diag-probes">
              <div v-for="p in diagResult.probes" :key="p.name" class="diag-probe">
                <span class="diag-dot" :class="p.ok ? 'ok' : 'fail'" />
                <span class="diag-probe-name">{{ p.name }}</span>
                <span class="diag-probe-status">
                  {{ p.ok ? (p.latency_ms != null ? p.latency_ms + 'ms' : t('settings.diagReachable')) : t('settings.diagUnreachable') }}
                </span>
              </div>
            </div>
          </div>
        </template>
      </div>
    </section>

    <!-- Config profiles (N6) -->
    <section id="sec-profiles" class="settings-section">
      <div class="section-header">
        <Layers :size="15" />
        <span>{{ t('settings.profiles') }}</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.profiles') }}</div>
            <div class="setting-desc">{{ t('settings.profilesDesc') }}</div>
          </div>
        </div>
        <div class="setting-divider" />
        <div class="profile-save-row">
          <input
            class="input"
            v-model="newProfileName"
            :placeholder="t('settings.profileNamePlaceholder')"
            @keyup.enter="saveCurrentProfile"
          />
          <button class="btn btn-ghost" :disabled="profileBusy || !newProfileName.trim()" @click="saveCurrentProfile">
            <Save :size="13" /> {{ t('settings.saveCurrentProfile') }}
          </button>
        </div>
        <div v-if="profiles.length === 0" class="profile-empty">{{ t('settings.noProfiles') }}</div>
        <div v-else class="profile-list">
          <div v-for="p in profiles" :key="p" class="profile-item">
            <span class="profile-name">{{ p }}</span>
            <div class="profile-actions">
              <button class="btn btn-ghost btn-sm" :disabled="profileBusy" @click="switchProfile(p)">
                {{ t('settings.switchProfile') }}
              </button>
              <button class="btn btn-ghost btn-sm danger" :disabled="profileBusy" @click="removeProfile(p)">
                <Trash2 :size="13" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- Config backup / restore -->
    <section id="sec-backup" class="settings-section">
      <div class="section-header">
        <Archive :size="15" />
        <span>{{ t('settings.configBackup') }}</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.exportConfig') }}</div>
            <div class="setting-desc">{{ t('settings.exportConfigDesc') }}</div>
          </div>
          <button class="btn btn-ghost btn-sm" :disabled="exportingConfig" @click="exportConfig">
            <Save :size="12" />
            {{ exportingConfig ? t('settings.exporting') : t('settings.export') }}
          </button>
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.importConfig') }}</div>
            <div class="setting-desc">{{ t('settings.importConfigDesc') }}</div>
          </div>
          <button class="btn btn-ghost btn-sm" :disabled="importingConfig" @click="showImportModal = true">
            <Upload :size="12" />
            {{ t('settings.import') }}
          </button>
        </div>
      </div>
    </section>

    <!-- TUN Mode -->
    <section id="sec-tun" class="settings-section">
      <div class="section-header">
        <Shield :size="15" />
        <span>{{ t('settings.tunMode') }}</span>
      </div>
      <div class="card settings-card">
        <!-- Enable toggle -->
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.enableTunMode') }}</div>
            <div class="setting-desc">{{ t('settings.enableTunModeDesc') }}</div>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="localConfig.tun_enabled" />
            <span class="toggle-track" />
          </label>
        </div>

        <!-- TUN requirements (show when enabled) -->
        <div v-if="localConfig.tun_enabled" class="tun-checklist">
          <!-- Admin status (Windows/Linux: the process must run elevated) -->
          <div v-if="!isMacOS" class="tun-check-row">
            <div class="check-status">
              <ShieldCheck v-if="isAdmin" :size="15" class="check-ok" />
              <ShieldAlert v-else :size="15" class="check-bad" />
            </div>
            <div class="check-info">
              <div class="check-label">{{ t('settings.adminPrivilege') }}</div>
              <div class="check-desc">
                <span v-if="isAdmin" class="text-ok">{{ t('settings.runningAsAdmin') }}</span>
                <span v-else class="text-bad">{{ t('settings.noAdminPrivilege') }}</span>
              </div>
            </div>
            <button v-if="!isAdmin" class="btn btn-primary btn-sm" @click="requestAdmin">
              <ShieldCheck :size="12" />
              {{ t('settings.restartAsAdmin') }}
            </button>
          </div>

          <!-- WinTun driver (Windows only; macOS/Linux use the kernel TUN device) -->
          <template v-if="isWindows">
            <div class="tun-divider" />
            <div class="tun-check-row">
              <div class="check-status">
                <CheckCircle v-if="wintunAvailable" :size="15" class="check-ok" />
                <AlertCircle v-else :size="15" class="check-bad" />
              </div>
              <div class="check-info">
                <div class="check-label">{{ t('settings.wintunDriver') }}</div>
                <div class="check-desc">
                  <span v-if="wintunAvailable" class="text-ok">{{ t('settings.wintunReady') }}</span>
                  <span v-else class="text-bad">{{ t('settings.wintunNotFound') }}</span>
                </div>
              </div>
              <button
                v-if="!wintunAvailable"
                class="btn btn-ghost btn-sm"
                :disabled="downloadingWintun"
                @click="downloadWintun"
              >
                <Download :size="12" :class="{ spin: downloadingWintun }" />
                {{ downloadingWintun ? t('settings.downloading') : t('settings.downloadWintun') }}
              </button>
            </div>
            <div v-if="wintunError" class="tun-error">{{ wintunError }}</div>
          </template>

          <!-- macOS privileged TUN service: authorize once, then passwordless -->
          <template v-else-if="isMacOS">
            <div class="tun-check-row">
              <div class="check-status">
                <ShieldCheck v-if="tunServiceReady" :size="15" class="check-ok" />
                <ShieldAlert v-else :size="15" class="check-bad" />
              </div>
              <div class="check-info">
                <div class="check-label">{{ t('settings.tunService') }}</div>
                <div class="check-desc">
                  <span v-if="tunServiceReady" class="text-ok">{{ t('settings.tunServiceReady') }}</span>
                  <span v-else class="text-bad">{{ t('settings.tunServiceNotReady') }}</span>
                </div>
              </div>
              <button
                v-if="!tunServiceReady"
                class="btn btn-primary btn-sm"
                :disabled="installingService"
                @click="installTunService"
              >
                <ShieldCheck :size="12" />
                {{ installingService ? t('settings.installingTunService') : t('settings.installTunService') }}
              </button>
            </div>
            <div v-if="serviceError" class="tun-error">{{ serviceError }}</div>
          </template>
        </div>
      </div>
    </section>

    <!-- Auto-select (URLTest) -->
    <section id="sec-autoselect" class="settings-section">
      <div class="section-header">
        <Zap :size="15" />
        <span>{{ t('settings.autoSelect') }}</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.testUrl') }}</div>
            <div class="setting-desc">{{ t('settings.testUrlDesc') }}</div>
          </div>
          <input
            class="input"
            type="text"
            v-model="localConfig.auto_test_url"
            placeholder="https://www.gstatic.com/generate_204"
            style="width: 260px"
          />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.testInterval') }}</div>
            <div class="setting-desc">{{ t('settings.testIntervalDesc') }}</div>
          </div>
          <input
            class="input port-input"
            type="number"
            v-model.number="localConfig.auto_test_interval"
            min="1" max="1440"
          />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.tolerance') }}</div>
            <div class="setting-desc">{{ t('settings.toleranceDesc') }}</div>
          </div>
          <input
            class="input port-input"
            type="number"
            v-model.number="localConfig.auto_tolerance"
            min="0" max="2000"
          />
        </div>
        <div class="kernel-hint">
          {{ t('settings.autoSelectHint') }}
        </div>
      </div>
    </section>

    <!-- Advanced -->
    <section id="sec-advanced" class="settings-section">
      <div class="section-header">
        <Cpu :size="15" />
        <span>{{ t('settings.advancedSettings') }}</span>
      </div>
      <div class="card settings-card">
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.logLevel') }}</div>
            <div class="setting-desc">{{ t('settings.logLevelDesc') }}</div>
          </div>
          <Select
            v-model="localConfig.log_level"
            :options="[
              { value: 'trace', label: t('settings.logLevelTrace') },
              { value: 'debug', label: 'Debug' },
              { value: 'info', label: t('settings.logLevelInfo') },
              { value: 'warn', label: 'Warn' },
              { value: 'error', label: t('settings.logLevelError') },
            ]"
          />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.theme') }}</div>
            <div class="setting-desc">{{ t('settings.themeDesc') }}</div>
          </div>
          <Select
            v-model="localConfig.theme"
            :options="[
              { value: 'system', label: t('settings.themeSystem') },
              { value: 'light', label: t('settings.themeLight') },
              { value: 'dark', label: t('settings.themeDark') },
            ]"
          />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.density') }}</div>
            <div class="setting-desc">{{ t('settings.densityDesc') }}</div>
          </div>
          <Select
            v-model="localConfig.density"
            :options="[
              { value: 'standard', label: t('settings.densityStandard') },
              { value: 'compact', label: t('settings.densityCompact') },
            ]"
          />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.language') }}</div>
            <div class="setting-desc">{{ t('settings.languageDesc') }}</div>
          </div>
          <Select
            v-model="localConfig.language"
            :options="[
              { value: 'zh-CN', label: t('settings.languageZh') },
              { value: 'en', label: t('settings.languageEn') },
            ]"
            @update:model-value="onLanguageChange"
          />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.autoCheckKernelUpdate') }}</div>
            <div class="setting-desc">{{ t('settings.autoCheckKernelUpdateDesc') }}</div>
          </div>
          <ToggleSwitch v-model="localConfig.auto_update_notify" :aria-label="t('settings.autoCheckKernelUpdate')" />
        </div>
        <div class="setting-divider" />
        <div class="setting-row">
          <div class="setting-info">
            <div class="setting-label">{{ t('settings.checkInterval') }}</div>
            <div class="setting-desc">{{ t('settings.checkIntervalDesc') }}</div>
          </div>
          <input
            class="input port-input"
            type="number"
            v-model.number="localConfig.auto_update_interval"
            min="0" max="168"
            :disabled="!localConfig.auto_update_notify"
          />
        </div>
      </div>
    </section>

    <!-- About -->
    <div class="card about-card">
      <div class="about-logo">
        <svg width="32" height="32" viewBox="0 0 24 24" fill="none">
          <circle cx="12" cy="12" r="10" stroke="var(--color-primary)" stroke-width="2"/>
          <path d="M8 12h8M12 8l4 4-4 4" stroke="var(--color-primary)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </div>
      <div>
        <div class="about-name">Skylark</div>
        <div class="about-desc">{{ t('settings.aboutDesc') }}</div>
        <div class="about-version">v{{ appVersion }} · {{ installedVersion ?? (kernelExists ? t('settings.kernelVersionUnknown') : t('settings.singboxNotInstalled')) }}</div>
      </div>
    </div>

    <!-- Import config modal -->
    <div v-if="showImportModal" class="dialog-overlay" @click.self="showImportModal = false">
      <div class="dialog-card">
        <div class="dialog-title">{{ t('settings.importConfig') }}</div>
        <div class="dialog-hint">{{ t('settings.importDialogHint') }}</div>
        <textarea
          class="input import-area"
          v-model="importText"
          rows="8"
          :placeholder="t('settings.importPlaceholder')"
        />
        <div class="dialog-actions">
          <button class="btn btn-ghost" @click="showImportModal = false">{{ t('settings.cancel') }}</button>
          <button class="btn btn-primary" :disabled="importingConfig" @click="importConfig">
            <RefreshCw v-if="importingConfig" :size="13" class="spin" />
            {{ importingConfig ? t('settings.importing') : t('settings.importAndOverwrite') }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Network diagnostics (N5) */
.diag-error { display: flex; align-items: center; gap: 6px; color: var(--color-error); font-size: 12.5px; padding: 4px 18px; }
.diag-results { display: flex; flex-direction: column; gap: 8px; padding: 4px 18px 14px; }
.diag-line { display: flex; gap: 10px; font-size: 13px; }
.diag-key { color: var(--color-text-secondary); min-width: 70px; }
.diag-val { font-weight: 500; font-family: var(--font-mono, monospace); }
.diag-probes { display: flex; flex-wrap: wrap; gap: 12px; margin-top: 4px; }
.diag-probe { display: flex; align-items: center; gap: 6px; font-size: 12px; }
.diag-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }
.diag-dot.ok { background: var(--color-success); }
.diag-dot.fail { background: var(--color-error); }
.diag-probe-name { font-weight: 500; }
.diag-probe-status { color: var(--color-text-secondary); }

/* Config profiles (N6) — align with .setting-row's 18px horizontal padding (the card
   itself has padding:0, so direct children must supply their own inset). */
.profile-save-row { display: flex; gap: 8px; align-items: center; padding: 12px 18px; }
.profile-save-row .input { flex: 1; }
.profile-empty { color: var(--color-text-secondary); font-size: 12.5px; padding: 4px 18px 14px; }
.profile-list { display: flex; flex-direction: column; gap: 6px; padding: 2px 18px 14px; }
.profile-item {
  display: flex; align-items: center; justify-content: space-between;
  padding: 7px 10px; border-radius: var(--radius-md);
  background: var(--color-neutral-soft);
  border: 1px solid var(--color-border);
  transition: background 0.2s ease-out, border-color 0.2s ease-out;
}
.profile-item:hover { background: var(--color-neutral); border-color: var(--color-primary-soft); }
.profile-name { font-size: 13px; font-weight: 500; }
.profile-actions { display: flex; gap: 6px; }
.btn-sm { padding: 3px 10px; font-size: 12px; }
.btn-sm.danger { color: var(--color-error); }

/* Sticky section-jump nav pinned to the app-content scroll region so sections
   scroll underneath it. Full-bleed over the shell's 24px side padding so it reads
   as a pinned header band (not a floating card), with an OPAQUE bg and no
   backdrop blur — blurring the moving content underneath is what produced the
   on-scroll ghosting / see-through artifact. */
.section-nav {
  position: sticky;
  top: 0;
  z-index: var(--z-sticky);
  display: flex;
  gap: 6px;
  overflow-x: auto;
  margin: 0 -24px;
  padding: 10px 24px;
  background: var(--color-bg);
  border-bottom: 1px solid var(--color-border);
  scrollbar-width: none;
}
.section-nav::-webkit-scrollbar { display: none; }
.section-chip {
  flex-shrink: 0;
  padding: 5px 12px;
  border-radius: 100px;
  border: 1px solid var(--color-border);
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 12px;
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.18s ease, color 0.18s ease, border-color 0.18s ease;
}
.section-chip:hover { background: var(--color-neutral-soft); color: var(--color-text); }
.section-chip.active {
  background: var(--color-primary);
  color: #fff;
  border-color: transparent;
  box-shadow: 0 2px 8px var(--color-primary-glow);
}

.autosave-badge {
  display: inline-flex; align-items: center; gap: 5px;
  font-size: 12px; font-weight: 500;
  color: var(--color-success); padding: 4px 10px;
  background: var(--color-success-soft);
  border: 1px solid var(--color-border);
  border-radius: 100px;
}
.autosave-enter-active, .autosave-leave-active { transition: opacity 0.3s, transform 0.3s; }
.autosave-enter-from, .autosave-leave-to { opacity: 0; transform: translateY(-4px); }

.settings-section { display: flex; flex-direction: column; gap: 10px; scroll-margin-top: 60px; }
.section-header {
  display: flex; align-items: center; gap: 7px;
  font-size: 11px; font-weight: 600; color: var(--color-text-secondary);
  text-transform: uppercase; letter-spacing: 0.5px; padding: 0 4px;
}
.settings-card { padding: 0; overflow: hidden; }
.setting-row {
  display: flex; align-items: center; justify-content: space-between;
  gap: 16px; padding: 14px 18px;
  transition: background 0.2s ease-out;
}
.setting-row:hover { background: var(--color-neutral-soft); }
.setting-info { flex: 1; }
.setting-label { font-size: 13px; font-weight: 500; color: var(--color-text); margin-bottom: 2px; }
.setting-desc { font-size: 11px; color: var(--color-text-muted); line-height: 1.45; }
.not-installed { color: var(--color-error); }
.version-unknown { color: var(--color-text-muted); font-style: italic; }
.setting-divider { height: 1px; background: var(--color-border); margin: 0 18px; }

/* Toggle Switch */
.toggle { position: relative; display: inline-block; width: 42px; height: 24px; flex-shrink: 0; }
.toggle input { opacity: 0; width: 0; height: 0; }
.toggle-track {
  position: absolute; inset: 0;
  background: var(--color-neutral-strong); border-radius: 12px;
  cursor: pointer; transition: background 0.2s ease-out, box-shadow 0.2s ease-out;
}
.toggle-track::before {
  content: '';
  position: absolute; left: 3px; top: 3px;
  width: 18px; height: 18px; border-radius: 50%;
  background: white; transition: transform 0.2s ease-out;
  box-shadow: var(--shadow-sm);
}
.toggle input:checked + .toggle-track {
  background: var(--color-primary);
  box-shadow: 0 2px 8px var(--color-primary-glow);
}
.toggle input:checked + .toggle-track::before { transform: translateX(18px); }
.toggle input:focus-visible + .toggle-track { outline: 2px solid var(--color-primary); outline-offset: 2px; }

.port-input { width: 100px; text-align: right; }

/* ─── Kernel Card ─── */
.kernel-card {}
.kernel-status { flex-shrink: 0; }

.release-info {
  padding: 10px 18px 12px;
  background: var(--color-primary-soft);
  border-top: 1px solid var(--color-border);
  border-bottom: 1px solid var(--color-border);
}
.release-header {
  display: flex; align-items: center; gap: 8px; flex-wrap: wrap; margin-bottom: 6px;
}
.release-tag {
  font-size: 13px; font-weight: 700;
  color: var(--color-primary);
  font-family: 'Cascadia Code', monospace;
}
.release-date { font-size: 11px; color: var(--color-text-muted); }
.release-notes {
  font-size: 11px; color: var(--color-text-secondary);
  line-height: 1.5;
  max-height: 160px; overflow-y: auto;
}
/* Compact Markdown rendering for release notes (v-html, sanitized). */
.release-notes.markdown-body :where(h1, h2, h3, h4, h5, h6) {
  font-size: 12px; font-weight: 600; color: var(--color-text-primary);
  margin: 8px 0 4px; line-height: 1.4;
}
.release-notes.markdown-body :where(h1, h2, h3, h4, h5, h6):first-child { margin-top: 0; }
.release-notes.markdown-body p { margin: 4px 0; }
.release-notes.markdown-body :where(ul, ol) { margin: 4px 0; padding-left: 18px; }
.release-notes.markdown-body li { margin: 2px 0; }
.release-notes.markdown-body a { color: var(--color-primary); text-decoration: none; }
.release-notes.markdown-body a:hover { text-decoration: underline; }
.release-notes.markdown-body strong { color: var(--color-text-primary); font-weight: 600; }
.release-notes.markdown-body code {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 10px; padding: 1px 4px; border-radius: var(--radius-sm);
  background: var(--color-neutral-strong);
}
.release-notes.markdown-body pre {
  margin: 6px 0; padding: 8px; border-radius: var(--radius-sm); overflow-x: auto;
  background: var(--color-neutral-strong);
}
.release-notes.markdown-body pre code { padding: 0; background: none; }
.release-notes.markdown-body :where(h1, h2) { border: none; padding: 0; }

.kernel-actions {
  display: flex; align-items: center; gap: 8px; padding: 12px 18px; flex-wrap: wrap;
}

.download-progress-area {
  padding: 0 18px 14px;
  display: flex; flex-direction: column; gap: 6px;
}
.progress-info {
  display: flex; justify-content: space-between; align-items: center;
  font-size: 12px; color: var(--color-text-secondary);
}
.progress-bytes { font-family: 'Cascadia Code', monospace; font-size: 11px; }
.progress-bar-track {
  height: 6px; background: var(--color-neutral);
  border-radius: var(--radius-sm); overflow: hidden;
}
.progress-bar-fill {
  height: 100%; background: var(--color-primary);
  border-radius: var(--radius-sm);
  box-shadow: 0 0 8px var(--color-primary-glow);
  transition: width 0.3s ease-out;
}
.progress-bar-fill.done { background: var(--color-success); box-shadow: 0 0 8px var(--color-success-glow); }

.kernel-error {
  display: flex; align-items: flex-start; gap: 6px;
  margin: 0 18px 12px;
  padding: 10px 12px;
  background: var(--color-error-soft);
  border: 1px solid var(--color-error-soft);
  border-radius: var(--radius-md);
  font-size: 12px; color: var(--color-error); line-height: 1.4;
}
.kernel-hint {
  padding: 0 18px 14px;
  font-size: 11px; color: var(--color-text-muted); line-height: 1.5;
}
.kernel-hint code {
  font-family: 'Cascadia Code', monospace;
  background: var(--color-neutral-strong);
  padding: 1px 4px; border-radius: var(--radius-sm);
}

.badge-purple {
  display: inline-flex; align-items: center; gap: 4px;
  font-size: 11px; font-weight: 500; padding: 2px 8px;
  border-radius: 100px;
  background: rgba(124, 92, 236, 0.12); color: var(--accent-violet);
}

.channel-select-wrap {
  display: flex; align-items: center; gap: 8px; flex-shrink: 0;
}
.channel-label { font-size: 11px; color: var(--color-text-muted); white-space: nowrap; }

/* TUN checklist */
.tun-checklist {
  padding: 12px 18px 14px;
  border-top: 1px solid var(--color-border);
  display: flex; flex-direction: column; gap: 0;
  background: var(--color-neutral-soft);
}
.tun-check-row {
  display: flex; align-items: center; gap: 10px; padding: 8px 0;
}
.tun-divider { height: 1px; background: var(--color-border); margin: 2px 0; }
.check-status { flex-shrink: 0; }
.check-ok { color: var(--color-success); }
.check-bad { color: var(--color-error); }
.check-info { flex: 1; }
.check-label { font-size: 13px; font-weight: 500; margin-bottom: 1px; }
.check-desc { font-size: 11px; }
.text-ok { color: var(--color-success); }
.text-bad { color: var(--color-error); }
.btn-sm { padding: 4px 10px !important; font-size: 12px !important; flex-shrink: 0; }
.tun-error {
  font-size: 11px; color: var(--color-error);
  padding: 6px 8px; background: var(--color-error-soft);
  border: 1px solid var(--color-error-soft);
  border-radius: var(--radius-sm); margin-top: 4px;
}

.about-card { padding: 20px; display: flex; align-items: center; gap: 16px; }
.about-name { font-size: 15px; font-weight: 600; color: var(--color-text); margin-bottom: 3px; }
.about-desc { font-size: 12px; color: var(--color-text-muted); margin-bottom: 6px; }
.about-version {
  display: inline-flex; align-items: center;
  font-size: 11px; color: var(--color-text-secondary);
  font-family: 'Cascadia Code', monospace;
  padding: 2px 8px; border-radius: 100px;
  background: var(--color-neutral-strong);
}

/* Import config dialog */
.dialog-overlay {
  position: fixed; inset: 0; z-index: var(--z-modal);
  display: flex; align-items: center; justify-content: center;
  background: rgba(0,0,0,0.4); backdrop-filter: blur(4px);
}
.dialog-card {
  width: min(560px, 92vw);
  background: var(--color-surface-strong); border: 1px solid var(--color-border);
  border-radius: var(--radius-lg); padding: 20px;
  display: flex; flex-direction: column; gap: 12px;
  box-shadow: var(--shadow-lg), var(--edge-highlight);
}
.dialog-title { font-size: 15px; font-weight: 600; }
.dialog-hint { font-size: 12px; color: var(--color-text-secondary); line-height: 1.5; }
.import-area { width: 100%; resize: vertical; font-family: var(--font-mono, monospace); font-size: 12px; }
.dialog-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }

@media (prefers-reduced-motion: reduce) {
  .setting-row, .profile-item,
  .toggle-track, .toggle-track::before,
  .progress-bar-fill { transition: none; }
}
</style>
