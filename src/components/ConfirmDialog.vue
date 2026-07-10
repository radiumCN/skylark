<script setup lang="ts">
import { watch, nextTick, ref } from "vue";
import { AlertTriangle } from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useFeedbackStore } from "../stores/feedback";

const { t } = useI18n();
const fb = useFeedbackStore();
const confirmBtn = ref<HTMLButtonElement | null>(null);
const cancelBtn = ref<HTMLButtonElement | null>(null);

function onKey(e: KeyboardEvent) {
  if (!fb.confirmOpen) return;
  if (e.key === "Escape") {
    e.preventDefault();
    fb.resolveConfirm(false);
  } else if (e.key === "Enter") {
    // Danger dialogs must not confirm on a bare Enter — the highest-cost action can't be
    // the default key. Enter then activates whichever button holds focus (native behavior).
    if (!fb.confirmOptions.danger) {
      e.preventDefault();
      fb.resolveConfirm(true);
    }
  } else if (e.key === "Tab") {
    // Minimal focus trap: keep Tab cycling between the two buttons instead of escaping
    // into the inert background page.
    e.preventDefault();
    (document.activeElement === confirmBtn.value ? cancelBtn : confirmBtn).value?.focus();
  }
}

// Global key handling + initial focus only while the dialog is open. Danger dialogs
// focus Cancel (safe default); everything else focuses the primary action.
watch(
  () => fb.confirmOpen,
  (open) => {
    if (open) {
      window.addEventListener("keydown", onKey);
      nextTick(() => (fb.confirmOptions.danger ? cancelBtn : confirmBtn).value?.focus());
    } else {
      window.removeEventListener("keydown", onKey);
    }
  }
);
</script>

<template>
  <Teleport to="body">
    <Transition name="modal-pop">
      <div
        v-if="fb.confirmOpen"
        class="modal-overlay"
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
            <button ref="cancelBtn" class="btn btn-ghost" @click="fb.resolveConfirm(false)">
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
/* Overlay + entrance come from the shared .modal-overlay / modal-pop spec in main.css. */
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
</style>
