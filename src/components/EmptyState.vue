<script setup lang="ts">
import type { Component } from "vue";

defineProps<{
  icon: Component;
  title: string;
  desc?: string;
  iconSize?: number;
}>();
</script>

<template>
  <div class="empty-state">
    <component :is="icon" :size="iconSize ?? 36" class="empty-icon" />
    <div class="empty-title">{{ title }}</div>
    <div v-if="desc" class="empty-desc">{{ desc }}</div>
    <!-- Optional action area (e.g. an "Add" button) -->
    <slot />
  </div>
</template>

<style scoped>
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-8) var(--space-6);
  color: var(--color-text-muted);
  text-align: center;
}
.empty-icon { opacity: 0.35; }
.empty-title { font-size: var(--fs-lg); font-weight: 600; color: var(--color-text-secondary); }
.empty-desc { font-size: var(--fs-md); }
</style>
