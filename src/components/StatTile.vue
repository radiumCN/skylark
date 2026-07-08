<script setup lang="ts">
/**
 * Shared metric tile — unifies Home's `.stat-card` / `.traffic-stat` and Stats'
 * `.summary-card`, which were three near-identical implementations of the same
 * "tinted icon + label + big value + sub" idea.
 *
 * `icon` is an optional Lucide component. `accent` picks the tinted icon color
 * from the Aurora token palette.
 */
import type { Component } from "vue";

withDefaults(
  defineProps<{
    label: string;
    value: string | number;
    sub?: string;
    icon?: Component;
    accent?: "primary" | "success" | "violet" | "teal" | "amber";
    /** Smaller value type for long text (e.g. node names). */
    compact?: boolean;
  }>(),
  { accent: "primary", compact: false }
);
</script>

<template>
  <div class="card stat-tile">
    <div v-if="icon" class="stat-tile-icon" :class="'accent-' + accent">
      <component :is="icon" :size="18" />
    </div>
    <div class="stat-tile-body">
      <div class="stat-tile-label">{{ label }}</div>
      <div class="stat-tile-value" :class="{ compact }">{{ value }}</div>
      <div v-if="sub" class="stat-tile-sub">{{ sub }}</div>
    </div>
  </div>
</template>

<style scoped>
.stat-tile {
  padding: var(--space-4);
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  transition: transform 0.15s ease, box-shadow 0.15s ease;
}
.stat-tile:hover {
  transform: translateY(-1px);
  box-shadow: var(--shadow-md), var(--edge-highlight);
}
.stat-tile-icon {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-lg);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.accent-primary { background: var(--color-primary-soft); color: var(--color-primary); }
.accent-success { background: var(--color-success-soft); color: var(--color-success); }
.accent-violet { background: rgba(124, 92, 236, 0.14); color: var(--accent-violet); }
.accent-teal { background: rgba(14, 163, 148, 0.14); color: var(--accent-teal); }
.accent-amber { background: rgba(193, 128, 30, 0.14); color: var(--accent-amber); }

.stat-tile-body { flex: 1; min-width: 0; }
.stat-tile-label {
  font-size: var(--fs-xs);
  color: var(--color-text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: 4px;
}
.stat-tile-value {
  font-size: var(--fs-lg);
  font-weight: 700;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.stat-tile-value.compact { font-size: var(--fs-md); }
.stat-tile-sub {
  font-size: var(--fs-xs);
  color: var(--color-text-muted);
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
