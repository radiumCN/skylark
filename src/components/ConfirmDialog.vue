<script setup lang="ts">
import { watch, nextTick, ref } from "vue";
import { AlertTriangle } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useFeedbackStore } from "../stores/feedback";

const { t } = useI18n();
const fb = useFeedbackStore();
const confirmBtn = ref<HTMLButtonElement | null>(null);

function onKey(e: KeyboardEvent) {
  if (!fb.confirmOpen) return;
  if (e.key === "Escape") {
    e.preventDefault();
    fb.resolveConfirm(false);
  } else if (e.key === "Enter") {
    e.preventDefault();
    fb.resolveConfirm(true);
  }
}

// Global key handling + focus the primary action only while the dialog is open.
watch(
  () => fb.confirmOpen,
  (open) => {
    if (open) {
      window.addEventListener("keydown", onKey);
      nextTick(() => confirmBtn.value?.focus());
    } else {
      window.removeEventListener("keydown", onKey);
    }
  }
);
</script>

<template>
  <Teleport to="body">
    <Transition name="confirm">
      <div
        v-if="fb.confirmOpen"
        class="confirm-overlay"
        @click.self="fb.resolveConfirm(false)"
      >
        <div class="confirm-dialog card-strong" role="alertdialog" aria-modal="true">
          <div class="confirm-head">
            <div class="confirm-icon" :class="{ danger: fb.confirmOptions.danger }">
              <AlertTriangle :size="18" />
            </div>
            <h2 class="confirm-title">
              {{ fb.confirmOptions.title ?? t('common.confirmTitle') }}
            </h2>
          </div>
          <p class="confirm-message">{{ fb.confirmOptions.message }}</p>
          <div class="confirm-actions">
            <button class="btn btn-ghost" @click="fb.resolveConfirm(false)">
              {{ fb.confirmOptions.cancelText ?? t('common.cancel') }}
            </button>
            <button
              ref="confirmBtn"
              class="btn"
              :class="fb.confirmOptions.danger ? 'btn-danger' : 'btn-primary'"
              @click="fb.resolveConfirm(true)"
            >
              {{ fb.confirmOptions.confirmText ?? t('common.confirm') }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.confirm-overlay {
  position: fixed;
  inset: 0;
  z-index: var(--z-modal);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--space-6);
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(2px);
  -webkit-backdrop-filter: blur(2px);
}
.confirm-dialog {
  width: 100%;
  max-width: 380px;
  padding: var(--space-5);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-lg), var(--edge-highlight);
}
.confirm-head {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-3);
}
.confirm-icon {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-lg);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  background: var(--color-primary-soft);
  color: var(--color-primary);
}
.confirm-icon.danger {
  background: var(--color-error-soft);
  color: var(--color-error);
}
.confirm-title {
  font-size: var(--fs-lg);
  font-weight: 600;
  color: var(--color-text);
}
.confirm-message {
  font-size: var(--fs-md);
  line-height: 1.6;
  color: var(--color-text-secondary);
  margin-bottom: var(--space-5);
  word-break: break-word;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
}

/* Scale + fade from centre — dialog scales up, scrim fades. */
.confirm-enter-active,
.confirm-leave-active {
  transition: opacity 0.2s ease;
}
.confirm-enter-active .confirm-dialog,
.confirm-leave-active .confirm-dialog {
  transition: transform 0.2s cubic-bezier(0.34, 1.4, 0.64, 1), opacity 0.2s ease;
}
.confirm-enter-from,
.confirm-leave-to {
  opacity: 0;
}
.confirm-enter-from .confirm-dialog,
.confirm-leave-to .confirm-dialog {
  opacity: 0;
  transform: scale(0.94) translateY(6px);
}
</style>
