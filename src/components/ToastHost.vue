<script setup lang="ts">
import { CheckCircle, AlertCircle, Info, X } from "@lucide/vue";
import type { Component } from "vue";
import { useFeedbackStore } from "../stores/feedback";
import type { ToastType } from "../stores/feedback";

const fb = useFeedbackStore();
const icons: Record<ToastType, Component> = {
  success: CheckCircle,
  error: AlertCircle,
  info: Info,
};
</script>

<template>
  <Teleport to="body">
    <div class="toast-host" role="status" aria-live="polite">
      <TransitionGroup name="toast">
        <div
          v-for="tst in fb.toasts"
          :key="tst.id"
          class="toast card-strong"
          :class="'toast-' + tst.type"
        >
          <component :is="icons[tst.type]" :size="16" class="toast-icon" />
          <span class="toast-msg">{{ tst.message }}</span>
          <button class="toast-close" aria-label="Dismiss" @click="fb.removeToast(tst.id)">
            <X :size="14" />
          </button>
        </div>
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<style scoped>
.toast-host {
  position: fixed;
  top: var(--space-4);
  right: var(--space-4);
  z-index: var(--z-toast);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  pointer-events: none;
  max-width: min(360px, calc(100vw - 32px));
}
.toast {
  pointer-events: auto;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 10px var(--space-3);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg), var(--edge-highlight);
  font-size: var(--fs-md);
  color: var(--color-text);
}
.toast-icon { flex-shrink: 0; }
/* --color-success-text lifts the emerald on dark surfaces (theme-split in main.css). */
.toast-success .toast-icon { color: var(--color-success-text); }
.toast-error .toast-icon { color: var(--color-error); }
.toast-info .toast-icon { color: var(--color-primary); }
/* Left accent edge carries the type without relying on the icon alone. */
.toast-success { border-left: 3px solid var(--color-success); }
.toast-error { border-left: 3px solid var(--color-error); }
.toast-info { border-left: 3px solid var(--color-primary); }
.toast-msg {
  flex: 1;
  min-width: 0;
  line-height: 1.45;
  word-break: break-word;
}
.toast-close {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: none;
  background: transparent;
  color: var(--color-text-muted);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}
.toast-close:hover {
  background: var(--color-neutral);
  color: var(--color-text);
}

/* Slide in from the right + fade; leaving collapses smoothly. */
.toast-enter-active,
.toast-leave-active {
  transition: opacity 0.22s ease, transform 0.22s ease;
}
.toast-enter-from {
  opacity: 0;
  transform: translateX(16px);
}
.toast-leave-to {
  opacity: 0;
  transform: translateX(16px);
}
.toast-leave-active {
  position: absolute;
  right: 0;
}
</style>
