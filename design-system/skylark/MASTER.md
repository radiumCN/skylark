# Design System Master File — Skylark "Aurora"

> **LOGIC:** When building a specific page, first check `design-system/skylark/pages/[page-name].md`.
> If that file exists, its rules **override** this Master file.
> If not, strictly follow the rules below.
>
> **SOURCE OF TRUTH:** every token below mirrors `src/styles/main.css` (`@theme` block +
> theme overrides). If this file and `main.css` disagree, `main.css` wins — fix this file.

---

**Project:** Skylark（云雀）— sing-box GUI proxy client (Tauri v2 + Vue 3)
**Aesthetic:** "Aurora" — premium dark-glass (Linear-inspired): deep slate base, indigo-violet accent, glassmorphism + ambient aurora glow, emerald "connected" state. **Dark is the hero theme; light is fully supported.**
**Last synced with `main.css`:** 2026-07-10

---

## Global Rules

### Color Palette

All colors are consumed via CSS variables — never hardcode a hex in a component.
`@theme` carries the **light** values; `html[data-theme="dark"]` and the
`prefers-color-scheme: dark` + `html:not([data-theme])` pair override for dark.

| Role | Light | Dark | CSS Variable |
|------|-------|------|--------------|
| Accent (indigo-violet) | `#5e6ad2` | same | `--color-primary` |
| Accent hover / active | `#4f5cc9` / `#434fbe` | same | `--color-primary-hover` / `--color-primary-active` |
| Success / connected (emerald) | `#0b7a4c` | `#0e8f5a` | `--color-success` |
| Warning (yellow) | `#e0a008` | same | `--color-warning` |
| Attention (orange — "warn / mid / caution") | `#b45309` | `#d1741b` | `--color-attention` |
| Error | `#e5484d` | same | `--color-error` |
| Background | `#f2f3f9` | `#0c0e16` | `--color-bg` |
| Surface (glass card) | `rgba(255,255,255,.70)` | `rgba(28,31,46,.60)` | `--color-surface` |
| Surface strong (inputs, menus) | `rgba(255,255,255,.86)` | `rgba(37,41,59,.82)` | `--color-surface-strong` |
| Border | `rgba(17,20,45,.09)` | `rgba(255,255,255,.09)` | `--color-border` |
| Text | `#16182a` | `#eceef6` | `--color-text` |
| Text secondary | `#565b70` | `#a3a8bd` | `--color-text-secondary` |
| Text muted (11–12px small print) | `#676c82` | `#7d8298` | `--color-text-muted` |

Soft tinted fills (badges, active states, hovers): `--color-primary-soft`,
`--color-success-soft`, `--color-error-soft`, `--color-warning-soft` (fixed rgba, ~14–16% alpha).
Neutral gray scale: `--color-neutral-soft` (5%), `--color-neutral` (10%), `--color-neutral-strong` (16%) — use these instead of ad-hoc `rgba(128,128,128,x)`.
Glow (behind shell / hero elements): `--color-primary-glow`, `--color-success-glow`.
Latency tiers (good→bad): `--latency-good` / `--latency-mid` / `--latency-bad` (theme-split like success/attention).
Decorative stat accents: `--accent-violet #7c5cec`, `--accent-teal #0ea394`, `--accent-amber #c0870f`.

**Contrast rule:** muted/attention/success carry 11–12px text, so their **light** values are
tuned to ≥4.5:1 (WCAG AA) — do not "brighten them back" in light mode; dark mode already
restores the vivid hues via the theme overrides.

### Typography

- **One family for everything:** `'Segoe UI Variable', 'Segoe UI', system-ui, -apple-system, sans-serif` (native desktop feel; no webfonts, the app must work fully offline).
- Base size **14px**; `font-variant-numeric: tabular-nums` globally so live metrics don't jitter; antialiased.

| Token | Value | Usage |
|-------|-------|-------|
| `--fs-xs` | 11px | badges, small print |
| `--fs-sm` | 12px | secondary text, table cells |
| `--fs-md` | 13px | controls (buttons, inputs, menus) |
| `--fs-lg` | 15px | dialog / section titles |
| `--fs-xl` | 20px | page titles |

### Spacing — 4px grid

| Token | Standard | Compact (`html[data-density="compact"]`) |
|-------|----------|------------------------------------------|
| `--space-1` | 4px | 3px |
| `--space-2` | 8px | 6px |
| `--space-3` | 12px | 8px |
| `--space-4` | 16px | 12px |
| `--space-5` | 20px | 14px |
| `--space-6` | 24px | 16px |
| `--space-8` | 32px | 22px |

Compact density (Settings → UI density) shrinks **container padding and gaps only** —
font sizes, icon sizes and radii stay put. Compact overrides live centrally in `main.css`
(scored `html[data-density] .x` to beat scoped view rules); when a view's container
padding changes, grep `data-density` and revisit its compact value.

Other layout tokens: `--sidebar-width: 220px` (compact 200px — floor set by the two-column
traffic readout), `--page-max: 960px` reading column (`.page--wide` opts out).

### Radii

| Token | Value | Usage |
|-------|-------|-------|
| `--radius-sm` | 6px | menu options, checkboxes, small chips |
| `--radius-md` | 10px | buttons, inputs, segmented controls |
| `--radius-lg` | 14px | cards |
| `--radius-xl` | 20px | dialogs |

### Shadows & glass

| Token | Usage |
|-------|-------|
| `--shadow-sm` | resting cards |
| `--shadow-md` | strong surfaces (menus, `.card-strong`) |
| `--shadow-lg` | modals, dropdown menus |
| `--edge-highlight` | inset 1px top highlight that sells the glass look — pair with every card/dialog shadow |

Glass recipe: `.card` = surface + border + `backdrop-filter: blur(22px) saturate(180%)` +
`var(--shadow-sm), var(--edge-highlight)`. `.card-strong` = surface-strong + blur(28px)/200% +
shadow-md. Never pure `#000` backgrounds (OLED smear) and never pure-white light surfaces.

### Z-index ladder

`--z-sticky: 20` → `--z-dropdown: 40` → `--z-modal: 1000` → `--z-toast: 1100`.
One ladder for the whole app; never invent intermediate values in components.

---

## Component Specs (shared classes in `main.css`)

### Buttons — `.btn` + variant

- Base: `padding 7px 14px` (compact 5px 11px), `--radius-md`, 13px/500, 1px `--color-border`, `gap 6px`, `white-space: nowrap`.
- Press feedback: `:active { transform: scale(0.97) }`.
- **Disabled (mandatory on every async action):** `:disabled { opacity: .5; cursor: not-allowed }`, hover/press feedback suppressed per variant.
- Variants: `.btn-primary` (accent bg + `0 2px 10px var(--color-primary-glow)`), `.btn-ghost` (transparent, hover `--color-neutral`), `.btn-danger` (error bg).

### Inputs — `.input`

`padding 8px 12px` (compact 6px 10px), `--radius-md`, surface-strong bg, 13px.
Focus: `border-color: var(--color-primary)` + `box-shadow: 0 0 0 3px var(--color-primary-soft)`.

### Segmented control — `.segmented` / `.segmented__item`

The single "pill group" for every mode/level/view switch. 2px padded track on
`--color-neutral-soft`; active item gets surface-strong + `--color-primary` text + shadow-sm.
Do not build bespoke pill groups.

### Badges — `.badge` + `-green/-red/-yellow/-blue/-gray`

Pill (radius 100px), 11px/500, soft tinted fills. Dark theme lifts yellow/green text
(`#f0c044` / `#34d399`) — handled centrally in `main.css`.

### Checkboxes / toggles

- Native checkboxes are restyled globally (16px Aurora box + white check).
- Boolean switches use the **`ToggleSwitch.vue`** component exclusively — never a
  hand-rolled `label.toggle` + hidden checkbox (three legacy forks were already unified once).
  Pass `aria-label` when there is no visible label.

### Select — `Select.vue`

Teleported listbox (`--z-dropdown`, `.card-strong` panel, max-height 280px), keyboard
navigable (arrows / Enter / Esc), flips upward near the window bottom, unique ids via `useId()`.

### Modals

- Confirm prompts: **always** `feedback` store `fb.confirm()` → `ConfirmDialog.vue`
  (danger mode focuses **Cancel** and ignores bare Enter).
- Custom overlays: pair with the **`useModal()`** composable (Escape closes, focus moves in
  and is restored on close) + `role="dialog" aria-modal="true"` + label + `tabindex="-1"`.
- Scrim/motion: use the shared tokens/classes — `--scrim` + `.modal-overlay` +
  `<Transition name="modal-pop">` from `main.css`. Enter 0.2s (scale 0.94 → 1, spring-ish
  `cubic-bezier(0.34,1.4,0.64,1)`), leave 0.15s. No bespoke per-view scrim values.

### Feedback

- Toasts via `feedback` store; durations are tiered (info ~3.5s / success ~4s / error ~6s).
- Every destructive bulk action confirms first; every successful explicit save toasts.
- Loading: `Skeleton.vue` for first paint, `.spin` (0.8s linear) on refresh icons.
- Empty states: `EmptyState.vue` with icon + title + desc.

---

## Interaction & A11y (non-negotiable)

- Keyboard reachability: anything clickable is a `<button>` or has `role="button" tabindex="0"` + Enter/Space handlers.
- Focus visibility: append new interactive classes to the shared `:focus-visible` group in `main.css` (2px `--color-primary` outline, offset 2px). Never remove outlines without a replacement.
- Hover-revealed controls (e.g. row close buttons) must also reveal on `:focus-visible`.
- `prefers-reduced-motion: reduce` is honored globally (all animation/transition durations collapse) — don't opt out.
- Text selection: UI chrome is `user-select: none`; copyable data opts in via `.selectable` (or the whitelisted classes: `.sub-url`, `.log-line`, `.conn-table`, `.node-server`).

## Motion

- Micro-interactions: 0.15s ease (borders, hovers); 0.18–0.2s for entrances.
- Page transitions: `name="page"` fade + 6px translateY, 0.18s.
- Theme flip: background cross-fades 0.3s.
- No GSAP / no JS animation libs — CSS transitions + Vue `<Transition>` only.

## Anti-Patterns (Do NOT Use)

- ❌ Hardcoded colors/sizes in components — use the tokens (grep `main.css` first).
- ❌ New pill groups / toggles / dialogs when `.segmented`, `ToggleSwitch`, `fb.confirm`, `useModal` exist.
- ❌ Emojis as icons — Lucide (`@lucide/vue`) only, sizes 13–16px in controls.
- ❌ Pure black/white surfaces; low-contrast small text (<4.5:1 in light mode).
- ❌ Layout-shifting hovers; instant state changes (always 150–300ms).
- ❌ Invisible focus states; hover-only affordances.
- ❌ Web-only assumptions: this is a desktop webview — no external network for fonts/CDN.

## Pre-Delivery Checklist

- [ ] All colors/spacing/radii pulled from tokens; both themes checked (toggle `data-theme`)
- [ ] Disabled + loading + empty + error states covered
- [ ] Keyboard: Tab reaches it, Enter/Space activates, Esc closes overlays, focus visible
- [ ] Compact density looks right (`html[data-density="compact"]`)
- [ ] Long text (node names, URLs) truncates with ellipsis + `title`/tooltip
- [ ] New user-facing strings go through vue-i18n (`zh-CN` **and** `en`)
- [ ] `prefers-reduced-motion` unaffected
