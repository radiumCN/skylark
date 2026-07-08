<script setup lang="ts">
/**
 * Shimmer placeholder block for loading states. Size it via props (or wrap it in
 * a layout). Purely decorative — hidden from the a11y tree; the reduced-motion
 * media query freezes the shimmer.
 */
withDefaults(
  defineProps<{
    width?: string;
    height?: string;
    radius?: string;
    circle?: boolean;
  }>(),
  { width: "100%", height: "14px", radius: "var(--radius-sm)", circle: false }
);
</script>

<template>
  <span
    class="skeleton"
    :class="{ circle }"
    :style="{ width, height, borderRadius: circle ? '50%' : radius }"
    aria-hidden="true"
  />
</template>

<style scoped>
.skeleton {
  display: block;
  flex-shrink: 0;
  background: linear-gradient(
    100deg,
    var(--color-neutral) 30%,
    var(--color-neutral-strong) 50%,
    var(--color-neutral) 70%
  );
  background-size: 200% 100%;
  animation: skeleton-shimmer 1.4s ease-in-out infinite;
}
@keyframes skeleton-shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}
@media (prefers-reduced-motion: reduce) {
  .skeleton { animation: none; }
}
</style>
