<script setup lang="ts">
import { onMounted, watch } from "vue";
import { RouterView } from "vue-router";
import Sidebar from "./components/Sidebar.vue";
import ConfirmDialog from "./components/ConfirmDialog.vue";
import ToastHost from "./components/ToastHost.vue";
import { useAppStore } from "./stores/app";
import { setLocale } from "./i18n";

const store = useAppStore();

// Keep the active UI locale in sync with the persisted AppConfig.language.
watch(() => store.config.language, (lang) => setLocale(lang), { immediate: true });

function applyTheme(theme: string) {
  const html = document.documentElement;
  if (theme === "dark") {
    html.setAttribute("data-theme", "dark");
  } else if (theme === "light") {
    html.setAttribute("data-theme", "light");
  } else {
    html.removeAttribute("data-theme");
  }
}

watch(() => store.config.theme, applyTheme, { immediate: true });

onMounted(async () => {
  await store.init();
  // Re-apply after config loads from backend
  applyTheme(store.config.theme);
});
</script>

<template>
  <div class="app-shell">
    <Sidebar />
    <main class="app-content">
      <RouterView v-slot="{ Component }">
        <Transition name="page" mode="out-in">
          <component :is="Component" :key="$route.path" />
        </Transition>
      </RouterView>
    </main>

    <!-- Global feedback surfaces (replace native confirm()/alert()) -->
    <ConfirmDialog />
    <ToastHost />
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  height: 100vh;
  overflow: hidden;
  position: relative;
  background: var(--color-bg);
}
/* Ambient "aurora" wash — soft indigo/violet/teal blobs bleeding from the corners.
   Fixed to the shell (behind the glass sidebar + content) so panels read as frosted
   glass floating over depth. Subtle in light mode, luminous in dark. */
.app-shell::before {
  content: "";
  position: absolute;
  inset: 0;
  pointer-events: none;
  z-index: 0;
  background:
    radial-gradient(60% 55% at 12% 0%, var(--aurora-1), transparent 70%),
    radial-gradient(50% 50% at 100% 25%, var(--aurora-2), transparent 68%),
    radial-gradient(55% 55% at 85% 100%, var(--aurora-3), transparent 72%);
}
.app-content {
  position: relative;
  z-index: 1;
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  /* No TOP padding: it lives on .page instead, so sticky headers can pin flush
     to the scroll top (a top padding here would leave a gap the content peeks
     through). Sides + bottom stay here. */
  padding: 0 24px 24px;
}
</style>
