<script setup lang="ts">
import { ref, computed, onBeforeUnmount } from "vue";
import { ChevronDown, Check } from "@lucide/vue";

export interface SelectOption {
  value: string | number;
  label: string;
}

const props = withDefaults(
  defineProps<{
    modelValue: string | number;
    options: SelectOption[];
    disabled?: boolean;
    placeholder?: string;
    /** Match the trigger width or let the menu grow to content. */
    menuAlign?: "left" | "right";
  }>(),
  { disabled: false, placeholder: "", menuAlign: "left" }
);
const emit = defineEmits<{ "update:modelValue": [value: string | number] }>();

let uid = 0;
const menuId = `select-menu-${(uid += 1)}`;

const open = ref(false);
const trigger = ref<HTMLButtonElement | null>(null);
const activeIndex = ref(-1);
const pos = ref({ top: 0, left: 0, width: 0 });

const selectedLabel = computed(() => {
  const o = props.options.find((opt) => opt.value === props.modelValue);
  return o ? o.label : props.placeholder;
});

function updatePosition() {
  const el = trigger.value;
  if (!el) return;
  const r = el.getBoundingClientRect();
  pos.value = { top: r.bottom + 4, left: r.left, width: r.width };
}

function openMenu() {
  if (props.disabled) return;
  updatePosition();
  open.value = true;
  activeIndex.value = props.options.findIndex((o) => o.value === props.modelValue);
  window.addEventListener("scroll", onReposition, true);
  window.addEventListener("resize", onReposition);
  document.addEventListener("mousedown", onDocPointer, true);
}
function closeMenu() {
  if (!open.value) return;
  open.value = false;
  window.removeEventListener("scroll", onReposition, true);
  window.removeEventListener("resize", onReposition);
  document.removeEventListener("mousedown", onDocPointer, true);
}
function toggle() {
  open.value ? closeMenu() : openMenu();
}
function choose(opt: SelectOption) {
  emit("update:modelValue", opt.value);
  closeMenu();
  trigger.value?.focus();
}

function onReposition() {
  if (open.value) updatePosition();
}
function onDocPointer(e: MouseEvent) {
  const target = e.target as Node;
  if (trigger.value?.contains(target)) return;
  if (document.getElementById(menuId)?.contains(target)) return;
  closeMenu();
}

function onTriggerKey(e: KeyboardEvent) {
  if (props.disabled) return;
  if (!open.value) {
    if (["Enter", " ", "ArrowDown", "ArrowUp"].includes(e.key)) {
      e.preventDefault();
      openMenu();
    }
    return;
  }
  // Open: navigate / commit / dismiss.
  if (e.key === "Escape") {
    e.preventDefault();
    closeMenu();
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    activeIndex.value = Math.min(props.options.length - 1, activeIndex.value + 1);
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    activeIndex.value = Math.max(0, activeIndex.value - 1);
  } else if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    const opt = props.options[activeIndex.value];
    if (opt) choose(opt);
  }
}

onBeforeUnmount(closeMenu);
</script>

<template>
  <div class="select" :class="{ disabled }">
    <button
      ref="trigger"
      type="button"
      class="select-trigger"
      :class="{ open }"
      :disabled="disabled"
      :aria-expanded="open"
      aria-haspopup="listbox"
      @click="toggle"
      @keydown="onTriggerKey"
    >
      <span class="select-value" :class="{ placeholder: selectedLabel === placeholder }">
        {{ selectedLabel }}
      </span>
      <ChevronDown class="select-caret" :size="15" />
    </button>

    <Teleport to="body">
      <Transition name="select-pop">
        <ul
          v-if="open"
          :id="menuId"
          class="select-menu card-strong"
          role="listbox"
          :style="{
            top: pos.top + 'px',
            left: menuAlign === 'left' ? pos.left + 'px' : 'auto',
            right: menuAlign === 'right' ? 'auto' : 'auto',
            minWidth: pos.width + 'px',
          }"
        >
          <li
            v-for="(opt, i) in options"
            :key="opt.value"
            class="select-option"
            :class="{ selected: opt.value === modelValue, active: i === activeIndex }"
            role="option"
            :aria-selected="opt.value === modelValue"
            @click="choose(opt)"
            @mouseenter="activeIndex = i"
          >
            <span class="select-option-label">{{ opt.label }}</span>
            <Check v-if="opt.value === modelValue" :size="14" class="select-check" />
          </li>
        </ul>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
.select {
  display: inline-block;
  position: relative;
}
.select-trigger {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  padding: 8px 10px 8px 12px;
  border-radius: var(--radius-md);
  border: 1px solid var(--color-border);
  background: var(--color-surface-strong);
  color: var(--color-text);
  font-size: 13px;
  cursor: pointer;
  transition: border-color 0.15s, box-shadow 0.15s;
  text-align: left;
}
.select-trigger:hover { border-color: var(--color-neutral-strong); }
.select-trigger.open,
.select-trigger:focus-visible {
  outline: none;
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px var(--color-primary-soft);
}
.select.disabled .select-trigger {
  opacity: 0.5;
  cursor: not-allowed;
}
.select-value {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.select-value.placeholder { color: var(--color-text-muted); }
.select-caret {
  flex-shrink: 0;
  color: var(--color-text-secondary);
  transition: transform 0.18s ease;
}
.select-trigger.open .select-caret { transform: rotate(180deg); }
</style>

<style>
/* Menu is teleported to <body>, so its styles must be global (not scoped). */
.select-menu {
  position: fixed;
  z-index: var(--z-dropdown);
  margin: 0;
  padding: 4px;
  list-style: none;
  max-height: 280px;
  overflow-y: auto;
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg), var(--edge-highlight);
}
.select-option {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  font-size: 13px;
  color: var(--color-text);
  cursor: pointer;
  white-space: nowrap;
}
.select-option.active { background: var(--color-neutral); }
.select-option.selected { color: var(--color-primary); font-weight: 600; }
.select-option-label { flex: 1; }
.select-check { flex-shrink: 0; color: var(--color-primary); }

.select-pop-enter-active,
.select-pop-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
}
.select-pop-enter-from,
.select-pop-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
