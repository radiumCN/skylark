<script setup lang="ts">
/**
 * Shared on/off switch — one implementation for the whole app (previously the
 * same control existed as three divergent variants: Home's CSS knob, the
 * label+checkbox in Settings/Subscriptions, and Lucide icon toggles in Rules).
 *
 * Controlled via v-model. For call sites whose handler toggles based on the
 * current runtime state (rather than the emitted value), bind
 * `:model-value="isOn" @update:model-value="handler"` — the handler may ignore
 * the argument.
 */
withDefaults(
  defineProps<{
    modelValue: boolean;
    disabled?: boolean;
    ariaLabel?: string;
  }>(),
  { disabled: false, ariaLabel: "" }
);

const emit = defineEmits<{ "update:modelValue": [value: boolean] }>();

function toggle(current: boolean, disabled: boolean) {
  if (disabled) return;
  emit("update:modelValue", !current);
}
</script>

<template>
  <button
    type="button"
    role="switch"
    class="toggle-switch"
    :class="{ on: modelValue, disabled }"
    :aria-checked="modelValue"
    :aria-label="ariaLabel || undefined"
    :disabled="disabled"
    @click="toggle(modelValue, disabled)"
  >
    <span class="toggle-switch-knob" />
  </button>
</template>

<style scoped>
.toggle-switch {
  width: 42px;
  height: 24px;
  border-radius: 100px;
  background: var(--color-neutral-strong);
  border: none;
  cursor: pointer;
  position: relative;
  transition: background 0.2s ease, box-shadow 0.2s ease;
  flex-shrink: 0;
  padding: 0;
}
.toggle-switch.on {
  background: var(--color-primary);
  box-shadow: 0 2px 8px var(--color-primary-glow);
}
.toggle-switch:focus-visible {
  outline: 2px solid var(--color-primary);
  outline-offset: 2px;
}
.toggle-switch.disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.toggle-switch-knob {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--color-on-primary);
  box-shadow: var(--shadow-sm);
  transition: transform 0.2s ease;
  display: block;
}
.toggle-switch.on .toggle-switch-knob {
  transform: translateX(18px);
}
</style>
