import { watch, nextTick, onScopeDispose, type Ref } from "vue";

/**
 * Shared keyboard/focus behavior for the self-drawn modal overlays (QR, node filters,
 * add-rule, import-config). ConfirmDialog aside, these only closed via a scrim click —
 * keyboard users had no way out, and focus stayed on the inert background page.
 *
 * While `open` is true: Escape calls `onClose`, and focus moves to the first focusable
 * element inside `dialog` (falling back to the dialog itself — give it tabindex="-1").
 * On close, focus returns to the element that was focused before the modal opened.
 * Callers still add `role="dialog" aria-modal="true"` + a label on the dialog markup.
 */
export function useModal(
  open: Ref<boolean>,
  opts: { onClose: () => void; dialog?: Ref<HTMLElement | null> }
) {
  let restoreFocus: HTMLElement | null = null;

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      opts.onClose();
    }
  }

  watch(open, (isOpen) => {
    if (isOpen) {
      restoreFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      window.addEventListener("keydown", onKey, true);
      nextTick(() => {
        const root = opts.dialog?.value;
        if (!root) return;
        const first = root.querySelector<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select, textarea, [tabindex]:not([tabindex="-1"])'
        );
        (first ?? root).focus();
      });
    } else {
      window.removeEventListener("keydown", onKey, true);
      restoreFocus?.focus();
      restoreFocus = null;
    }
  });

  // The listener must not outlive the owning component (e.g. navigating away with the
  // modal still open never runs the `open → false` branch).
  onScopeDispose(() => window.removeEventListener("keydown", onKey, true));
}
