import { defineStore } from "pinia";
import { ref } from "vue";

/**
 * App-wide feedback: a promise-based confirmation dialog and transient toasts.
 * Replaces the native window.confirm()/alert() popups (which render an unstyled
 * "tauri.localhost 显示…" chrome dialog) with Aurora-styled in-app surfaces.
 *
 * Usage:
 *   const fb = useFeedbackStore();
 *   if (!(await fb.confirm({ message: "…", danger: true }))) return;
 *   fb.toastSuccess("Done");   fb.toastError(String(err));
 */
export interface ConfirmOptions {
  message: string;
  title?: string;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
}

export type ToastType = "success" | "error" | "info";
export interface Toast {
  id: number;
  type: ToastType;
  message: string;
}

export const useFeedbackStore = defineStore("feedback", () => {
  // ─── Confirmation dialog ───────────────────────────────────────────────
  const confirmOpen = ref(false);
  const confirmOptions = ref<ConfirmOptions>({ message: "" });
  let resolver: ((value: boolean) => void) | null = null;

  function confirm(options: ConfirmOptions): Promise<boolean> {
    // A confirm() is already open (shouldn't happen in practice) — reject the old one.
    resolver?.(false);
    confirmOptions.value = options;
    confirmOpen.value = true;
    return new Promise<boolean>((resolve) => {
      resolver = resolve;
    });
  }

  function resolveConfirm(result: boolean) {
    if (!confirmOpen.value) return;
    confirmOpen.value = false;
    const r = resolver;
    resolver = null;
    r?.(result);
  }

  // ─── Toasts ────────────────────────────────────────────────────────────
  const toasts = ref<Toast[]>([]);
  let nextId = 1;

  function pushToast(type: ToastType, message: string, duration: number) {
    const id = nextId++;
    toasts.value.push({ id, type, message });
    if (duration > 0) {
      setTimeout(() => removeToast(id), duration);
    }
    return id;
  }
  function removeToast(id: number) {
    toasts.value = toasts.value.filter((t) => t.id !== id);
  }

  // Errors linger a little longer so they can be read; success/info auto-dismiss ~3.5s.
  const toastSuccess = (message: string) => pushToast("success", message, 3500);
  const toastError = (message: string) => pushToast("error", message, 6000);
  const toastInfo = (message: string) => pushToast("info", message, 4000);

  return {
    confirmOpen,
    confirmOptions,
    confirm,
    resolveConfirm,
    toasts,
    toastSuccess,
    toastError,
    toastInfo,
    removeToast,
  };
});
