# Frontend design: PracticePerfect platform

Component, template and interaction design for a white-label, multi-tenant
Symfony 7.4 application covering eight epics and 419 acceptance criteria,
server-rendered with Twig and AssetMapper, run exclusively via Docker Compose.

**Inputs already settled — do not reopen.** `specs/architect-architecture.md`
§§ "Approach", "White-label branding as a runtime value", "Stack" and
"Decisions"; Section A of `specs/requirements-analyst-open-questions.md`
(owner decisions A1–A12 and the fee call); `Task/designs/DESIGN_TOKENS.md` in
full. Where this document goes beyond them, it says so in the Decisions
table below, distinguishing a sourced extraction (narrowly grepped from a
design SVG) from a frontend-design addition (a gap DESIGN_TOKENS.md leaves
open that this stage has to fill to be buildable).

**This document does not contain application code.** `Task/app/` does not
exist yet and nothing here is implemented. It also does not contain database
schema (the database-designer stage) or endpoint contracts (the api-designer
stage) — only what a browser renders, the tokens and templates that render
it, and how it behaves with and without JavaScript. Verification and browser
test planning belong to implementation, not this design stage.

**Citation convention.** `AC-NN-n` references point to
`specs/requirements-analyst-epic-0N-*-spec.md` § "Acceptance criteria",
each of which already cites the original `Task/Epics/` file and line; this
document does not re-derive those. `Task/designs/*.svg` citations name the
file and the attribute grepped (per the task's own context warning, these
files were never read whole).

---

## Design tokens: CSS custom properties

Two layers, split by who can change them and when:

- **Static layer** — typography, grayscale, spacing, radius, shadow and
  semantic-status tokens. Compiled once by AssetMapper into
  `assets/styles/app.css`. Identical for every trainer, every request.
- **Runtime layer** — `--brand-primary`, `--brand-primary-soft`,
  `--brand-primary-deep`, `--brand-primary-rgb`, and one addition,
  `--brand-on-primary` (justified in the next section). Emitted as a small
  nonce-carrying inline `<style>` block on every request by `BrandingProvider`
  (`architect-architecture.md` § "White-label branding as a runtime value",
  lines 587–597). **These five are the only runtime-overridable properties.**
  Nothing else may be set inline, and nothing in the static layer may
  reference a trainer's raw hex directly — only through these five names.

CSS custom-property resolution happens at compute time against whatever is
cascaded onto `:root`, not at declaration time, so it does not matter that
the static file's shadow rules reference `var(--brand-primary-rgb)` before
the inline block (which sets that property) appears later in `<head>`.

### Static layer — `assets/styles/app.css`

```css
/* ==========================================================================
   PracticePerfect design tokens — static layer.
   Source of truth: Task/designs/DESIGN_TOKENS.md. Compiled once by
   AssetMapper; identical for every trainer. Never declare --brand-* here —
   see the runtime layer, emitted per-request by BrandingProvider.
   ========================================================================== */

:root {
  /* Typography scale — DESIGN_TOKENS.md "Typography Scale (from text.svg)",
     lines 9-20. No font-family survives in any source SVG (text is
     flattened to outlines on export); the family below is a placeholder —
     see Open questions. */
  --font-family-base: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
    Helvetica, Arial, sans-serif;

  --text-hero-title-size: 30px;    --text-hero-title-line: 38px;    --text-hero-title-weight: 700;
  --text-section-title-size: 22px; --text-section-title-line: 28px; --text-section-title-weight: 700;
  --text-block-title-size: 18px;   --text-block-title-line: 24px;   --text-block-title-weight: 700;
  --text-card-title-size: 16px;    --text-card-title-line: 22px;    --text-card-title-weight: 600;
  --text-body-lg-size: 16px;       --text-body-lg-line: 26px;       --text-body-lg-weight: 400;
  --text-body-size: 14px;          --text-body-line: 22px;          --text-body-weight: 400;
  --text-caption-size: 12px;       --text-caption-line: 18px;       --text-caption-weight: 400;
  --text-eyebrow-size: 11px;       --text-eyebrow-line: 16px;       --text-eyebrow-weight: 600;
  --text-eyebrow-tracking: 0.04em; /* addition: not in tokens; standard for a 600-weight uppercase 11px label */

  /* Grayscale palette — DESIGN_TOKENS.md "Base Palette (Grayscale)", lines 24-27 */
  --gray-50:  #F3F3F3;
  --gray-200: #CFCFCF;
  --gray-400: #868686;
  --gray-600: #5E5E5E;
  --gray-800: #363636;
  --gray-900: #0D0D0D;

  /* Surface/text semantics built on the grayscale — addition, see Decisions */
  --color-surface: #FFFFFF;
  --color-surface-sunken: var(--gray-50);
  --color-border: var(--gray-200);
  --color-border-strong: var(--gray-400);
  --color-text-primary: var(--gray-900);
  --color-text-secondary: var(--gray-600);
  --color-text-disabled: var(--gray-400);

  /* Semantic error — sourced narrowly from Task/designs/inputs.svg, the one
     place an input error state is actually drawn: fill="#FF5E58" and
     stroke="#FF5E58" both appear (grep -o 'fill="#[0-9A-Fa-f]*"'/'stroke=...'
     inputs.svg), i.e. it is used consistently for both text and border, not
     a one-off. DESIGN_TOKENS.md names an "error" input state (line 83) but
     gives no color — see Decisions. No sourced success/warning color exists;
     see Component inventory (Badges/pills) and Open questions. */
  --color-error: #FF5E58;
  --color-error-soft: #FFEEED; /* same 20%-toward-white treatment as --brand-primary-soft, applied to the sourced error hex */

  /* Spacing scale — DESIGN_TOKENS.md "Spacing Scale", lines 48-58 */
  --space-xxs: 4px;
  --space-xs: 8px;
  --space-sm: 12px;
  --space-md: 16px;
  --space-lg: 24px;
  --space-xl: 32px;
  --space-xxl: 40px;

  /* Border radius — DESIGN_TOKENS.md "Border Radius", lines 60-69 */
  --radius-xs: 6px;
  --radius-sm: 10px;
  --radius-md: 16px;
  --radius-lg: 24px;
  --radius-xl: 32px;
  --radius-pill: 999px;

  /* Shadows — DESIGN_TOKENS.md "Shadow System", lines 90-97. The two
     brand-dependent shadows reference the runtime --brand-primary-rgb; see
     runtime layer below. */
  --shadow-card-soft: 0 2px 8px rgba(0, 0, 0, 0.2);
  --shadow-card-strong: 0 4px 16px rgba(0, 0, 0, 0.3);
  --shadow-button-primary: 0 10px 30px rgba(var(--brand-primary-rgb), 0.55);
  --shadow-button-primary-hover: 0 12px 34px rgba(var(--brand-primary-rgb), 0.7);

  /* Component geometry — DESIGN_TOKENS.md "Component Specifications", lines 71-88 */
  --control-height-input: 40px;
  --control-padding-input: 10px;
  --control-size-checkbox: 14px;
  --control-hit-checkbox: 24px;  /* WCAG 2.2 SC 2.5.8 target size — see Decisions */
  --control-radius-checkbox: 2px; /* proportional to form_control_element.svg's 24x24/rx=4 reference box (rx:size = 1:6) applied to a 14px box */
  --control-size-radio: 22px;
  --control-size-toggle-w: 39px;
  --control-size-toggle-h: 24px;
  --control-size-toggle-knob: 21px;

  /* Buttons — DESIGN_TOKENS.md line 76: "sm (padding 18px 8px), md (padding
     24px 10px)". Reordered to CSS padding's vertical-then-horizontal
     convention (8px 18px / 10px 24px) — read literally the source order
     would make "sm" taller than the 40px input; see Decisions. */
  --button-padding-sm: 8px 18px;
  --button-padding-md: 10px 24px;
  --button-hover-lift: -4px;   /* DESIGN_TOKENS.md line 77: "-translate-y-1" */
  --button-hover-scale: 1.02;  /* DESIGN_TOKENS.md line 77 */

  /* Focus ring — DESIGN_TOKENS.md line 82 says "soft glow ring" with no
     spread/opacity number; this spec picks a concrete recipe — see Open
     questions. */
  --focus-ring: 0 0 0 3px rgba(var(--brand-primary-rgb), 0.25);
  --focus-ring-error: 0 0 0 3px rgba(255, 94, 88, 0.25); /* --color-error as a literal, since it must still work before/without brand context */

  /* Motion */
  --ease-standard: cubic-bezier(0.4, 0, 0.2, 1);
  --duration-fast: 120ms;
  --duration-standard: 200ms;
}

@media (prefers-reduced-motion: reduce) {
  :root {
    --duration-fast: 0ms;
    --duration-standard: 0ms;
    --button-hover-lift: 0px;
    --button-hover-scale: 1;
  }
}
```

### Runtime layer — inline per request, emitted by `BrandingProvider`

```twig
{# templates/base.html.twig <head>, after the AssetMapper app.css link #}
<style nonce="{{ csp_nonce('style') }}">
:root {
  --brand-primary: {{ brand.primaryHex }};
  --brand-primary-soft: {{ brand.primarySoftHex }};
  --brand-primary-deep: {{ brand.primaryDeepHex }};
  --brand-primary-rgb: {{ brand.primaryRgb }};   {# "r, g, b" — DESIGN_TOKENS.md line 45 #}
  --brand-on-primary: {{ brand.onPrimaryHex }};  {# addition — see next section #}
}
</style>
```

`BrandingProvider` always resolves a value on every request, authenticated or
public: the platform ships a default primary color and the default logo
(`Task/designs/default_logo.svg`) until a trainer sets their own, so this
block is unconditionally present — there is no code path where `--brand-*`
is undefined and the static layer's `var(--brand-primary-rgb)` references
resolve to nothing.

Trainer-uploaded logos always render through `<img src="...">`, never
inlined, per the settled architecture (`architect-architecture.md` lines
601–602: "an uploaded SVG cannot execute in the page's origin"). The
platform's own default logo is a trusted, first-party, non-uploaded asset
and may be inlined by AssetMapper if crisp scaling matters somewhere; that
distinction — trainer-uploaded versus platform-bundled — is what decides
`<img>` versus inline, not file type.

---

## Brand color derivation: PHP versus CSS `color-mix()`

**Decision: PHP, at render time, inside `BrandingProvider`.** This is already
settled — `architect-architecture.md` § "White-label branding as a runtime
value" (lines 587–592) states `BrandingProvider` "derives the shade ramp
server-side using the transformations `DESIGN_TOKENS.md` specifies
(`lightenColor`, `darkenColor`, `hexToRgb`)." This section is not reopening
that call; it substantiates it against the specific criteria this deliverable
asks for, because "the architecture already said so" is not itself a
technical justification.

Rejected alternative: store only the trainer's raw hex server-side and
compute `--brand-primary-soft` / `--brand-primary-deep` in the browser with
CSS `color-mix()`.

**Three reasons, load-bearing first:**

1. **`hexToRgb` has no CSS equivalent under the token contract as written.**
   DESIGN_TOKENS.md's own consumption pattern is
   `--brand-primary-rgb: r, g, b (for rgba usage)` (line 45) — a
   comma-separated component triplet, used exactly as written by
   `--shadow-button-primary` and `--shadow-button-primary-hover` above. No
   CSS function parses a `#RRGGBB` string held in a custom property into
   three numbers; that split has to happen somewhere capable of string
   manipulation before the value reaches CSS at all. The one CSS-native
   alternative — relative color syntax, `rgb(from var(--brand-primary) r g b
   / 0.55)` — is a different API than the one DESIGN_TOKENS.md documents, and
   adopting it would mean the token contract's own `--brand-primary-rgb`
   line is simply not implementable in CSS. At least this one derivation is
   server-side regardless of what is decided for the other two.
2. **Contrast-safety needs a luminance computation CSS cannot do.** Deciding
   whether text/icons on top of a trainer-chosen `--brand-primary` should be
   light or dark requires the WCAG relative-luminance formula (see
   Accessibility contract). No shipped CSS function computes this —
   `color-contrast()` was proposed for CSS Color 5 and dropped before any
   browser implemented it. `BrandingProvider` already has to run this
   computation to produce `--brand-on-primary`; running `lightenColor` /
   `darkenColor` in the same pass is one derivation surface instead of two.
3. **Consistency and drift.** Points 1–2 already force `hexToRgb` and the
   contrast check server-side. Splitting `lightenColor`/`darkenColor` into
   CSS `color-mix()` anyway would mean the same trainer hex is transformed by
   two implementations, in two languages, that must stay in exact numeric
   agreement forever — a maintenance liability with no offsetting benefit,
   since PHP is already unavoidably in the critical path.

**Browser support**, addressed because the task asks for it, is not what
decides this: `color-mix()` reached every evergreen browser in 2023
(Chrome/Edge 111, Safari 16.4, Firefox 113) and is safe to rely on today. The
relative color syntax that would be needed to also solve point 1 in pure CSS
shipped later and unevenly (Chrome 119, Safari 16.4 partial, Firefox 128).
Even with full support everywhere, points 1 and 2 still stand — this was
never a support problem.

**"Applies immediately" (AC-01-62).** Both approaches satisfy this equally,
and it would misstate the tradeoff to claim otherwise: nothing about
branding is cached or compiled (`architect-architecture.md` line 597), so
every request already re-reads `TrainerBrandingSettings` and re-emits the
inline block fresh. A `color-mix()` approach would recompute on every paint
from the same fresh `--brand-primary`; the PHP approach recomputes on every
request. Neither can ever serve a stale shade. The requirement is satisfied
by "nothing is cached," not by which side does the arithmetic — so it does
not discriminate between the two options.

**Companion decision — contrast-safe foreground, `--brand-on-primary`.**
DESIGN_TOKENS.md defines exactly four `--brand-*` properties (lines 42–46);
this spec adds a fifth, computed by `BrandingProvider` in the same pass:
WCAG relative luminance of `--brand-primary` decides whether primary-button
labels, filled-badge text and icons rendered on top of the brand color use a
near-white or near-black foreground, so a trainer who picks a pale accent
(AC-01-61 permits any hex via a free color picker) does not silently ship
unreadable white-on-pale-yellow text. This is a frontend-design addition
beyond the documented four, made directly in response to this deliverable's
contrast question — it is not implied by any AC and is called out again in
Decisions.

It does not restrict a trainer's color choice: AC-01-61 specifies a free hex
picker with no stated validation failure mode, so blocking the save on low
contrast would be inventing a rejection rule the epic never states. Instead,
saving a color whose contrast against white falls under 4.5:1 shows a
non-blocking inline warning ("This color may be hard to read — consider a
darker shade") next to the live preview (AC-01-61's own "real-time preview,"
lines 565–568) and still saves on confirmation. `--brand-on-primary` is the
robustness backstop for every color that passes that soft warning anyway, or
that a trainer dismisses it for.

---

## Twig template hierarchy

The directory layout mirrors `architect-architecture.md` § "Module map"
(lines 44–59) one-for-one, so the one place a template lives is predictable
from the module that owns the screen — `Platform`'s shared chrome stays
visibly separate from every epic-owned screen (Decisions).

```
templates/
├── base.html.twig                  # <html><head>: AssetMapper links, nonce brand <style>, skip-link, <title> block
├── platform/                       # shared chrome — owned by no single epic
│   ├── _authenticated_shell.html.twig  # flash region, main landmark, impersonation-banner slot; extended by the 4 role shells
│   ├── shell/
│   │   ├── trainer.html.twig       # ROLE_TRAINER
│   │   ├── coach.html.twig         # ROLE_COACH
│   │   ├── player.html.twig        # ROLE_PLAYER — parent, adult-self, and child-flagged accounts (one shell, narrowed nav; see Decisions)
│   │   └── super_admin.html.twig   # ROLE_SUPER_ADMIN
│   ├── public.html.twig            # unauthenticated: ShareLink landing, camp/eval forms — renders the resolved trainer's branding, no nav
│   └── components/                 # presentational partials + macros, used across every role
│       ├── _button.html.twig       # macro: button(label, variant, size, attrs)
│       ├── _form_theme.html.twig   # Symfony form theme: aria-describedby/aria-invalid wiring, error summary
│       ├── _card.html.twig
│       ├── _badge.html.twig        # label | flag | status variants — see Component inventory
│       ├── _modal.html.twig        # <dialog> shell, paired with modal_controller.js
│       ├── _data_table.html.twig
│       ├── _flash_messages.html.twig
│       ├── _empty_state.html.twig
│       ├── _impersonation_banner.html.twig
│       └── _availability_grid.html.twig
├── identity/        # Epic 01 — login, registration, ShareLink landing, profile, availability, branding settings, Users tool
├── scheduling/       # Epic 02 — Event Builder, Training Calendar, RSVP, My Activities, attendance, Event Master
├── crm/               # Epic 03 — Players list/detail, labels, flags, notes, Quick View dashboard
├── content/           # Epic 04 — Learn/Practice playlists, Drill Database, LPPP portal, Progress
├── billing/           # Epic 05 — Tokens/Wallet, checkout hand-off, transactions, earnings, pricing
├── growth/            # Epic 06 — referrals, coupons
├── administration/    # Epic 07 — Super Admin dashboard, feature toggles, audit log
└── forms/             # Epic 08 — camp/evaluation builder, public submission, conversion
```

**`base.html.twig`** owns everything that must be true regardless of role:
the nonce'd brand `<style>` block, the AssetMapper `importmap()` call, a
skip-to-content link (Accessibility contract), `<html lang="{{ app.request.locale }}">`,
and a `{% block title %}` that every page overrides — never a bare
"PracticePerfect" title on every tab.

**The four role shells** extend `platform/_authenticated_shell.html.twig`
and differ only in their nav item set and any role-specific header
furniture (trainer-context switcher for `player.html.twig`; the
impersonation-banner slot rendered in all four, populated only when
`ImpersonationSession` is active). Nav items are filtered through
`FeatureGate` (`architect-architecture.md` § "Cross-cutting services", line
408) before render — see Component inventory (Navigation) for why hidden,
not disabled.

**`player.html.twig` covers three account shapes with one shell**: a parent
managing children, an adult self-managed player, and a child-flagged login
(AC-01-29/30). `ROLE_PLAYER` is one of exactly four fixed roles
(`architect-architecture.md` lines 415–416, BR-01-7) — a child account is a
permission state on that role, not a fifth role, so the nav narrows from a
view model (`PlayerNavContext`) rather than branching to a different
template. A child-flagged context view model omits: buy tokens, manage
payment methods, add a trainer, and delete account — exactly AC-01-30's
"cannot" list — while RSVP/cancel/browse/view-progress stay present but
route through the parent-approval voter path (AC-01-29).

**`public.html.twig`** serves ShareLink landing pages (`/join/...`,
`/invite/...`) and camp/evaluation forms (`/forms/{code}`) with no
authenticated tenant. It resolves branding the same way — `BrandingProvider`
"for the trainer that owns the form or portal being viewed"
(`architect-architecture.md` line 590) — and renders no primary nav, no
role chrome, just the trainer's logo/accent, the form or landing content,
and a minimal footer.

**Component partials are plain Twig `include`/`macro`, not
`ux-twig-component`.** `symfony/ux-twig-component` is not named anywhere in
`architect-architecture.md` § "Stack" (lines 628–642), which lists
everything the platform "adds, pins or changes" over the Symfony 7.4
`--webapp` default; assuming an unlisted package would be inventing a
dependency. Plain `include`s with an explicit `with {...}` context and
`{% macro %}` for the smallest components (button, badge) reuse the markup
with zero additional dependency.

---

## Component inventory

Every mutating control (RSVP, cancel, delete, publish, save, toggle) is a
real `<button type="submit">` inside a `<form method="post">` carrying a
CSRF token, never a JS `onclick` handler and never a state-changing GET
link, per `examples/symfony-clean-code-patterns.md` § "11. Twig, Forms, And
Progressive Enhancement" (lines 443–451).

### Buttons

DESIGN_TOKENS.md § "Buttons (from buttons.svg)" (lines 73–77).

| Aspect | Spec |
|---|---|
| Variants | Primary (gradient `--brand-primary` → `--brand-primary-deep`, `--shadow-button-primary`, label `--brand-on-primary`); Secondary (`--color-border` outline, `--color-surface` fill, `--color-text-primary` label); Ghost (transparent, no border, `--brand-primary` label) |
| Sizes | sm `--button-padding-sm` (8px 18px); md `--button-padding-md` (10px 24px) |
| Hover | `transform: translateY(var(--button-hover-lift)) scale(var(--button-hover-scale))`, shadow steps to `--shadow-button-primary-hover` on Primary |
| Focus-visible | `box-shadow: var(--focus-ring)` — addition, not in tokens; required regardless (Accessibility contract) |
| Active/pressed | Addition: no lift, shadow steps back to `--shadow-card-soft` |
| Disabled | Reduced opacity (0.5), `cursor: not-allowed`; see Accessibility contract for why this is `aria-disabled`, not the native `disabled` attribute, on a button that must still explain itself to a screen reader |
| Loading | Addition: label replaced by a spinner + `aria-busy="true"`, button stays focusable but its `<form>` submit is guarded server-side by idempotency, not by disabling the button (prevents the double-submit the skill's Forms section calls out) |
| Danger | Addition beyond DESIGN_TOKENS.md's three named variants: `--color-error` fill, white label. Justified by volume — delete/deactivate/remove confirmations recur across nearly every entity (AC-01-52 deactivate, AC-01-55 delete, AC-03-13 delete label, AC-03-17 remove flag, AC-04-34/35 delete playlist/drill, AC-08-31 delete form) and a visually undifferentiated destructive action is a real misclick risk (Decisions) |

### Inputs

DESIGN_TOKENS.md § "Inputs (from inputs.svg)" (lines 79–83).

| State | Spec |
|---|---|
| Default | `height: var(--control-height-input)` (40px), `padding: var(--control-padding-input)` (10px), `border: 1px solid var(--color-border)` |
| Focus | `border-color: var(--brand-primary)`, `box-shadow: var(--focus-ring)` — DESIGN_TOKENS.md says "accent color border, soft glow ring" (line 82) with no numeric recipe; this spec's 3px/25%-alpha ring is a concrete choice — see Open questions |
| Error | `border-color: var(--color-error)`, `box-shadow: var(--focus-ring-error)` on focus, helper text in `--color-error`, `aria-invalid="true"`, `aria-describedby` pointing at the error text id (Accessibility contract) |
| Disabled | `background: var(--color-surface-sunken)`, `color: var(--color-text-disabled)`, `cursor: not-allowed` |
| Label | Always a real `<label>`, always visible above the field — never placeholder-as-label. Required fields marked with visible text, not color alone |

### Checkbox 14px / radio 22px / toggle 39×24

DESIGN_TOKENS.md § "Form Controls (from form_control_element.svg)" (lines
85–88), corroborated by `Task/designs/form_control_element.svg` itself
(narrow read — the file is 2 KB, small enough to read whole):

| Control | Visual | Evidence |
|---|---|---|
| Checkbox | 14×14px, `--control-radius-checkbox` (2px, proportional to the SVG's 24×24/`rx=4` reference) | SVG: checked = `rx=4` box, `fill="#00B300"` (the frame's accent-example green) + white check path; unchecked = `rx=3.25` stroked box, `stroke="#AAAAAA"` |
| Checkbox hit target | `--control-hit-checkbox` (24×24px), padding around the 14px visual mark | WCAG 2.2 SC 2.5.8 Target Size (Minimum, AA) requires ≥24×24 CSS px; the SVG's own unchecked-state box is already drawn at 24×24 (`x=220.75 y=16.75 width/height=22.5 stroke-width=1.5` → outer 24×24), which reads as corroborating evidence for a 24px target around a smaller visual indicator, not a contradiction of the 14px figure — see Decisions |
| Radio | 22×22px circle, inner dot ≈54% of outer diameter when selected | SVG: selected = `r=11.25` white circle, `stroke="#00B300"` width 1.5, inner `r=6` dot `fill="#00B300"`; unselected = `r=11.25` circle, `stroke="#AAAAAA"`, no fill |
| Toggle | 39×24px capsule, 21×21px white knob, ~1.5px inset | SVG: on = `39×24 rx=12` `fill="#00B300"` track, knob at `x=564.5` (right); off = same shape `fill="#AAAAAA"`, knob at `x=604.5` (left) |

Checked/on state uses `--brand-primary` (not the SVG's illustrative
`#00B300`, which is that specific export's sample accent, not a fixed
token). Unchecked/off state reuses `--gray-400` rather than adding a seventh,
undocumented gray for the SVG's `#AAAAAA` — one step off the nearest
documented stop (Decisions).

### Cards

DESIGN_TOKENS.md gives radius (`--radius-md` 16px default, `--radius-lg`
24px for larger cards, line 66–67) and shadow (`--shadow-card-soft` /
`--shadow-card-strong`, lines 94–95) but no dedicated "Card" component
section. "Card" is nonetheless a named unit in the source material — AC-04-7
explicitly describes public-drill discovery "result card"s showing name,
creator, thumbnail, and badges — so this spec defines one shared shape used
by: drill/playlist discovery cards, dashboard metric tiles (Quick View),
and calendar event cards. `background: var(--color-surface)`,
`border-radius: var(--radius-md)`, `box-shadow: var(--shadow-card-soft)`,
`padding: var(--space-lg)`; interactive cards (clickable discovery results)
step to `--shadow-card-strong` on hover/focus-within.

### Badges/pills

`--radius-pill` (999px, DESIGN_TOKENS.md line 69). Three distinct semantic
uses, each styled differently because they mean different things:

1. **Trainer-defined labels** — trainer-chosen color, "a color picker with
   presets" (AC-03-11). Pill background is the label's stored hex at reduced
   opacity, text/border at full opacity. The one badge type where color is
   arbitrary and per-trainer by design.
2. **System flags** — fixed, non-customizable treatment (icon + text, never
   trainer-colored): the 8 flags are Behavior, High no-show rate, Injured,
   Medical restriction, Scholarship, Financial aid, Contact priority,
   Attendance risk (`requirements-analyst-epic-03-crm-players-spec.md` §
   "Data requirements", line 352, citing BR-03-6). These carry medical and
   financial-aid categories about minors — precisely what
   `architect-architecture.md`'s RLS justification calls out (line 650:
   "discloses minors' medical and financial-aid flags") — so the icon must
   differ per flag type and never rely on color alone to distinguish e.g.
   "Injured" from "Scholarship" (WCAG 1.4.1 Use of Color; also the practical
   case for a color-blind trainer).
3. **Status pills** — fixed vocabulary pulled directly from the ACs:
   Registered, Confirmed, Canceled, Completed, Pending, Pending Parent
   Approval, Pending Coach Confirmation, Active, Inactive, Deleted, Paid,
   Refunded, Failed, Public, Private, Coach-only. Deliberately
   **brand-independent** — positive/active states reuse `--brand-primary`,
   negative states use the sourced `--color-error`, neutral/pending states
   use `--gray-600`/`--gray-800` with a distinguishing icon (clock for
   pending). A trainer whose brand color happens to be red must not make a
   "Refunded" pill and an on-brand element collide in meaning — see
   Decisions and Open questions (no sourced system success/warning color
   exists to do better than this).

### Modals

One shared pattern (`<dialog>` + `modal_controller.js`) for every
confirmation and detail-overlay: event details (AC-02-20), duplicate-event
(AC-02-14), and every delete/deactivate/remove confirmation across every
epic. Header (heading + close), body, footer (Cancel + primary action —
Danger-styled when destructive). Focus moves to the modal heading on open,
`Tab`/`Shift+Tab` cycles inside it, `Escape` closes and returns focus to the
triggering control, `aria-modal="true"` `role="dialog"`
`aria-labelledby` pointing at the heading (Accessibility contract).

**Every modal is backed by two real routes** — a GET that renders the same
content full-page, a POST that commits — with the `<dialog>` as a
progressive enhancement that intercepts the same link/form and renders it in
place instead of navigating. There is no JS-only overlay hiding a broken
link; see AssetMapper/Stimulus for the no-JS fallback this implies.

### Data tables

One shared shape behind: Players list, Event Builder list, Event Master
Tool, Users tool, Audit Log, Coupon list, Camp submissions, Transaction
history. Sortable column headers (`aria-sort`), search box + filter panel
above, row actions as an actions column, `--radius--md` container, empty
state (below), pagination.

**Pagination defaults to 50 rows/page everywhere**, not a per-screen
choice — the epics independently converge on 50 in four places (AC-03-5
players, AC-02-56 events, AC-07-12 users, AC-07-26 events again), so
matching it is consistency, not a new decision.

**Responsive**: no separate mobile-table spec exists in the source; AC-02-66
only requires the Training Calendar itself to "work on both desktop and
mobile." Data-dense admin tables (Players, Event Master, Audit Log) use a
horizontal-scroll wrapper (`overflow-x: auto` on the table container, never
the page body) below the breakpoint where columns stop fitting; card-grid
screens (Training Calendar, Drill Database, Content Library) reflow to
single-column cards natively since they are card layouts, not tables, at
every width.

### Navigation

Primary nav per role shell, items filtered through `FeatureGate`
(`architect-architecture.md` § "Cross-cutting services", line 408) **before**
render. A trainer with Marketing disabled sees no "Marketing" nav item at
all — not a grayed-out one — because "a disabled feature is denied at the
same place as any other permission" (architecture line 431) and a
clickable-looking nav item that 403s on click is exactly the kind of
generic-blank-screen-behind-an-error the frontend-design skill prohibits.

Trainer-context switcher (native `<select>`, auto-submitting) for the
Player shell per AC-01-15 ("a context switcher is available in navigation");
child switcher per AC-01-18. Impersonation banner renders inside the
authenticated shell's document flow (`position: sticky`, not `fixed`) so it
cannot obscure the first keyboard-focused element as a Super Admin tabs from
the top of the page (WCAG 2.2 SC 2.4.11 Focus Not Obscured, Decisions).

### Flash messages

Success/error/warning/info variants. `role="status"` (polite) for
success/info, `role="alert"` (assertive) for error/warning, rendered once at
the top of the main landmark, immediately after the page heading. Manual
close button always present; auto-dismiss (8s) is a JS-only enhancement, not
the only way to dismiss. A flash is never the *sole* evidence an action
worked — the resulting state change (row removed, status pill updated) is
the primary confirmation, since a user who scrolled past or missed the flash
must still be able to see the outcome from the page itself.

### Empty states

Icon (optional) + heading + body copy + primary action where one exists.
Copy is not invented — every case below is the exact string an AC already
specifies:

| Context | Copy | Source |
|---|---|---|
| Player search, no match | "No players found. Try different search terms." | AC-03-9 |
| Player filter, no match | "No players match your filters. Try adjusting criteria." | AC-03-26 |
| Drill filter, no match | "No drills match your filters. Try adjusting criteria." | AC-04-9 |
| Event full (free) | "Event Full - No spots available" | AC-02-27 |
| Event full (paid) | "Event Full" | AC-02-27 |
| Private event, not invited | "Access Denied" | AC-02-7 |
| Camp at capacity | "Camp Full" | AC-08-15 |
| Camp registration disabled | "Registration Closed" | AC-08-29 |
| Deleted playlist, stale reference | "Content unavailable" | AC-04-34 |
| Deleted drill, stale reference | "Drill unavailable" | AC-04-35 |
| Duplicate RSVP | "Already registered" | AC-02-28 |

Where no AC supplies copy (e.g. a trainer's first-ever, zero-event Event
Builder list), this spec does not invent wording — that is left to the
implementation stage, following the same voice as the strings above.

---

## AssetMapper and Stimulus organization

Assumes the full Symfony `--webapp` skeleton (AssetMapper + Stimulus +
Turbo), not AssetMapper in isolation — `architect-architecture.md` names
AssetMapper as "the current `--webapp` default" (line 638) and states
"everything is server-rendered Twig with progressive enhancement" (line 40),
which presumes Stimulus and Turbo are already scaffolded alongside it
(Decisions).

`assets/app.js` is the single entrypoint (Stimulus application boot,
imported via `importmap.php`); controllers live under
`assets/controllers/*_controller.js` and auto-register via the Stimulus
bridge. `assets/styles/app.css` is the static token layer from the previous
section plus component rules; nothing under `assets/` ever contains a
trainer's raw hex.

**Turbo Drive** is on by default (progressive full-page navigation, no
markup changes required). **Turbo Frames** are used narrowly, for: list/
filter regions that re-render in place (Players, Drill Database, Event
Master, Audit Log), and the modal-in-frame pattern above. **Turbo Streams
are not used.** No Mercure or WebSocket service exists in the five-service
Compose deployment (`architect-architecture.md` § "Deployment shape") to
push one; the epics' own async paths (refund fan-out, notification fan-out
on event cancellation) already resolve to eventual-consistency-on-next-load
— AC-05-17 states refunds are simply "tracked in the transaction history
log," not pushed live — so there is nothing to stream to, and this spec does
not invent an infrastructure piece to enable a UI behavior no AC asks for.

### Stimulus controller inventory

Every controller enhances a control that already works as a plain form/link;
the "No-JS fallback" column is that baseline, not a degraded afterthought.

| Controller | Purpose | Key AC(s) | No-JS fallback |
|---|---|---|---|
| `modal_controller.js` | Open a `<dialog>` in place of navigating to its GET-confirm route; trap focus; `Escape`/backdrop closes | Cancel/delete/deactivate confirmations across every epic | The GET confirm page and POST commit route both exist and work standalone |
| `playlist-reorder_controller.js` | Drag-and-drop reordering of playlist videos/drills | AC-04-2, AC-04-12, AC-04-31 | Always-visible "Move up"/"Move down" submit buttons per row — the *baseline* control, not a fallback-only affordance (WCAG 2.2 SC 2.5.7 Dragging Movements, AA) |
| `branding-preview_controller.js` | Live-update a swatch/button mockup as the trainer picks a color, before saving | AC-01-61 "real-time preview" | Hex text input still submits; the authoritative PHP-derived shades apply on the save round trip regardless |
| `live-search_controller.js` | Debounced Turbo Frame reload on `input` | AC-03-8 ("results update in real time"), AC-04-7 | An adjacent `<button type="submit">Search</button>` performs the same full-page GET/response |
| `filter-autosubmit_controller.js` | Auto-submit the filter panel on change | AC-03-22–26, AC-04-9 | An explicit "Apply Filters" submit button |
| `context-switcher_controller.js` | Auto-submit trainer/child context `<select>` on change | AC-01-15, AC-01-18 | An explicit "Switch" submit button beside the select |
| `clipboard_controller.js` | Copy-to-clipboard for ShareLinks and referral links | AC-03-50, AC-06-2 ("Share" button copies to clipboard) | The link renders as selectable plain text in a readonly input; user selects and copies manually |
| `toggle-autosubmit_controller.js` | Auto-submit a feature/visibility toggle on change | AC-07-19/20 feature toggles, AC-04-17 visibility toggle | An explicit "Save" button |
| `availability-check_controller.js` | Fetch the eligible-players-available count as a trainer picks an event date/time | AC-02-12/13 | An intermediate "Check availability" submit button reloads the page with the count computed server-side |
| `coupon-apply_controller.js` | Turbo Frame swap showing the discounted price after "Apply" | AC-06-20–22 | A plain "Apply" submit reloads the page with the discount already reflected |
| `youtube-player_controller.js` | Detect the YouTube IFrame API "play" event, mark content complete automatically | AC-04-25 | An always-visible manual "Mark Complete" button/link performs the same mutation; only the *automatic* trigger requires JS |
| `flash_controller.js` | Auto-dismiss a flash message after 8s | Component inventory (Flash messages) | The manual close button; absent JS, the flash simply persists until the next navigation — an acceptable degradation, not a violation |
| `availability-grid_controller.js` | "Copy to all days" convenience on the My Times grid | AC-01-43, AC-01-46 | Every day/time-range control is a plain checkbox/time input inside one form; fully functional without the convenience |

**Not built as Stimulus controllers, deliberately:** CSV export (a plain
link to a file-download route), Stripe Checkout/Customer Portal handoff (a
real HTTP redirect, no client JS involved on this platform's side), and
attendance-taking (a plain radio-button form per row with one "Save
Attendance" submit — a "mark all present" bulk action, if built, is pure
enhancement on top of that same form, not a replacement for it).

---

## Page inventory

One row per screen (not per user story — several stories share a screen,
e.g. RSVP and Cancel RSVP both live on Training Calendar/My Reservations).
Routes are illustrative paths, not Symfony route names. Three formats are
cited verbatim from their ACs rather than invented: the trainer mass-invite
link (`/join/[trainer-unique-code]`, AC-03-59), the one-time invite link
(`/invite/[unique-code]`, AC-03-50), and the referral link
(`platform.com/join/{trainer-slug}/{player-id}`, AC-06-1).

### Epic 01 — Identity (`templates/identity/`)

| Screen | Role | Route | AC(s) |
|---|---|---|---|
| Login | public | `/login` | AC-01-65, AC-01-68 |
| Password reset request/confirm | public | `/password/reset`, `/password/reset/{token}` | AC-01-66 |
| Email verification | public | `/email/verify/{token}` | AC-01-67 |
| ShareLink landing (player/parent join) | public → Player | `/join/{trainerCode}` | AC-01-9…14 |
| Coach invite landing | public → Coach | `/invite/{code}` | AC-01-39…42 |
| Users tool | Super Admin | `/admin/users` | AC-01-1…8, AC-01-71, AC-01-72 |
| Create trainer account | Super Admin | `/admin/users/new` | AC-01-1…8 |
| Player Profiles / Family | Player (parent) | `/player/family` | AC-01-16…24 |
| Availability / My Times (player) | Player | `/player/availability` | AC-01-43…45 |
| Availability / My Times (coach) | Coach | `/coach/availability` | AC-01-46, AC-01-47 |
| Pending purchase approvals | Player (parent) | `/player/approvals` | AC-01-25…28 |
| Profile / Account Settings | all | `/profile` | AC-01-48…51 |
| Impersonate confirm + exit | Super Admin | `/admin/users/{id}/impersonate` (POST), `/impersonation/exit` (POST) | AC-01-33…38 |
| Deactivate / reactivate user | Super Admin | `/admin/users/{id}/deactivate`, `/reactivate` | AC-01-52…54 |
| Delete user (GDPR) | Super Admin | `/admin/users/{id}/delete` | AC-01-55…59 |
| Portal Settings / Branding | Trainer | `/trainer/settings/branding` | AC-01-60…63 |
| Coaches (invite) | Trainer | `/trainer/coaches`, `/trainer/coaches/invite` | AC-01-39, AC-01-40 (trainer side) |
| Camp-conversion pre-filled registration | public → Player | `/register?fromSubmission={id}` | AC-01-64 |

### Epic 02 — Scheduling (`templates/scheduling/`)

| Screen | Role | Route | AC(s) |
|---|---|---|---|
| Event Builder list | Trainer | `/trainer/events` | AC-02-1, AC-02-2 |
| Create / edit event | Trainer | `/trainer/events/new`, `/trainer/events/{id}/edit` | AC-02-1…3, AC-02-50…54, AC-02-58…61 |
| Event detail + RSVP List tab | Trainer | `/trainer/events/{id}` | AC-02-43…45 |
| Duplicate event | Trainer | `/trainer/events/{id}/duplicate` | AC-02-14…17 |
| Cancel event | Trainer | `/trainer/events/{id}/cancel` | AC-02-46…49 |
| Training Calendar | Player | `/player/calendar` | AC-02-18…22, AC-02-64, AC-02-66 |
| Event details (modal/frame) | Player | `/player/events/{id}` | AC-02-20, AC-02-23 |
| RSVP / Register & Pay | Player | `/player/events/{id}/rsvp` | AC-02-23…28, AC-02-58…60 |
| My Reservations | Player | `/player/reservations` | AC-02-24, AC-02-26, AC-02-29 |
| Cancel RSVP | Player | `/player/reservations/{id}/cancel` | AC-02-29…33 |
| My Activities (Events to Confirm / Assigned Sessions) | Coach | `/coach/activities` | AC-02-34…37, AC-02-65 |
| Take Attendance | Coach | `/coach/events/{id}/attendance` | AC-02-38…42 |
| Event Master Tool | Super Admin | `/admin/events` | AC-02-55…57, AC-07-22…28 |

### Epic 03 — CRM (`templates/crm/`)

| Screen | Role | Route | AC(s) |
|---|---|---|---|
| Players list (CRM Tools) | Trainer | `/trainer/players` | AC-03-1…10, AC-03-22…26, AC-03-67 |
| Manage Labels | Trainer | `/trainer/labels` | AC-03-11, AC-03-13, AC-03-14 |
| Player detail | Trainer | `/trainer/players/{id}` | AC-03-27…33 |
| Quick View dashboard | Trainer | `/trainer/dashboard` | AC-03-34…42, AC-03-66 |
| Players (coach view) | Coach | `/coach/players` | AC-03-43…45, AC-03-64, AC-03-65 |
| Player detail (coach view) | Coach | `/coach/players/{id}` | AC-03-44, AC-03-46…49 |
| Invite player (coach) | Coach | `/coach/players/invite` | AC-03-50…53 |
| CRM Master / All Players | Super Admin | `/admin/players` | AC-03-54…58 |
| Invite players (trainer ShareLink) | Trainer | `/trainer/players/invite` | AC-03-59…63 |

### Epic 04 — Content / LPPP (`templates/content/`)

| Screen | Role | Route | AC(s) |
|---|---|---|---|
| Learn playlists list + create | Trainer | `/trainer/content/learn`, `/new` | AC-04-1…3 |
| Drill Database (My/Public Drills) | Trainer | `/trainer/content/drills` | AC-04-4…10 |
| Practice playlists list + create | Trainer | `/trainer/content/practice`, `/new` | AC-04-11, AC-04-12 |
| Assign playlist | Trainer | `/trainer/content/playlists/{id}/assign` | AC-04-13…16 |
| Edit playlist | Trainer | `/trainer/content/playlists/{id}/edit` | AC-04-31…33 |
| Delete playlist/drill | Trainer | `/trainer/content/playlists/{id}/delete`, `/drills/{id}/delete` | AC-04-34…36 |
| LPPP portal — Learn/Practice/Progress | Player | `/player/lppp/{learn,practice,progress}` | AC-04-20…24, AC-04-27…29 |
| Video/drill player page | Player | `/player/lppp/items/{id}` | AC-04-24…26 |
| LPPP Analytics | Super Admin | `/admin/lppp-analytics` | AC-04-37…40 |

### Epic 05 — Billing (`templates/billing/`)

| Screen | Role | Route | AC(s) |
|---|---|---|---|
| Connect Stripe | Trainer | `/trainer/settings/billing` | AC-05-1…3 |
| Tokens / Wallet | Player | `/player/tokens` | AC-05-4…6, AC-05-29 |
| RSVP with tokens / card (shared with Epic-02 RSVP) | Player | `/player/events/{id}/rsvp` | AC-05-7…13, AC-05-30…32 |
| Cancel & refund (shared with Epic-02 cancel) | Player | `/player/reservations/{id}/cancel` | AC-05-14…17 |
| Purchase content access | Player | `/player/lppp/items/{playlistId}/purchase` | AC-05-18, AC-05-19 |
| Payment methods (Stripe Customer Portal redirect) | Player (parent) | `/player/settings/payment-methods` | AC-05-20, AC-05-21 |
| Transaction History | Player | `/player/transactions` | AC-05-22…24 |
| Payments / Earnings | Trainer | `/trainer/earnings` | AC-05-25, AC-05-26 |
| Edit trainer pricing | Super Admin | `/admin/trainers/{id}/pricing` | AC-05-27, AC-05-28 |
| Gift tokens | Trainer | `/trainer/players/{id}/tokens/gift` | AC-05-33 |

### Epic 06 — Growth (`templates/growth/`)

| Screen | Role | Route | AC(s) |
|---|---|---|---|
| Get the Assist (referral link) | Player | `/player/referrals` | AC-06-1…3 |
| Referral dashboard | Trainer | `/trainer/marketing/referrals` | AC-06-13…16 |
| Coupons list + create | Trainer | `/trainer/marketing/coupons`, `/new` | AC-06-17…19 |
| Apply coupon at checkout (shared component) | Player | part of RSVP/purchase flow | AC-06-20…25 |
| Coupon detail / analytics | Trainer | `/trainer/marketing/coupons/{id}` | AC-06-26…28 |
| Referral Program settings | Super Admin | `/admin/settings/referral-program` | AC-06-29…31 |

### Epic 07 — Administration (`templates/administration/`)

| Screen | Role | Route | AC(s) |
|---|---|---|---|
| Super Admin dashboard | Super Admin | `/admin/dashboard` | AC-07-1…7 |
| Users tool (extends Epic-01) | Super Admin | `/admin/users` | AC-07-8…15 |
| Create trainer (extends Epic-01) | Super Admin | `/admin/users/new-trainer` | AC-07-16, AC-07-17 |
| Trainer Feature Settings | Super Admin | `/admin/trainers/{id}/features` | AC-07-18…21 |
| Event Master (shared with Epic-02) | Super Admin | `/admin/events` | AC-07-22…28 |
| Audit Log | Super Admin | `/admin/audit-log` | AC-07-29…34 |
| Stripe Dashboard links (outbound, no local view) | Super Admin | external | AC-07-35…37 |
| Trainers list | Super Admin | `/admin/trainers` | AC-07-38, AC-07-39 |

### Epic 08 — Forms (`templates/forms/`)

| Screen | Role | Route | AC(s) |
|---|---|---|---|
| Camps & Evaluations list | Trainer | `/trainer/forms` | AC-08-27, AC-08-32 |
| Create camp form | Trainer | `/trainer/forms/camps/new` | AC-08-1…8 |
| Create evaluation form | Trainer | `/trainer/forms/evaluations/new` | AC-08-9…12 |
| Edit form settings | Trainer | `/trainer/forms/{id}/edit` | AC-08-28…31 |
| Public submission page | public | `/forms/{code}` | AC-08-13…16, AC-08-39…42 |
| Camp submissions list | Trainer | `/trainer/forms/{id}/submissions` | AC-08-17…22 |
| Post-submission account creation | public → Player | `/forms/{code}/create-account` | AC-08-23…26, AC-01-64 |

Roughly 65 distinct screens across the eight epics, plus the public/
unauthenticated set (login, password reset, email verification, ShareLink
and invite landings, camp/evaluation forms) that render before any tenant is
authenticated.

---

## Accessibility contract

Target: WCAG 2.2 AA, meaning conformance to every Level A and Level AA
success criterion, not only the criteria new in 2.2. Native HTML semantics
are preferred over ARIA throughout (skill: "Use native elements before ARIA")
— the context-switcher `<select>`, the account menu as `<details>/<summary>`,
real `<button>`/`<a>` elements. Nothing below is a full audit; a dedicated
`wcag-accessibility` pass belongs to implementation, against real markup,
not this spec.

**Focus visibility (SC 2.4.7, AA).** Every interactive element gets
`box-shadow: var(--focus-ring)` (or `--focus-ring-error` inside an invalid
field) on `:focus-visible`, never suppressed with `outline: none` alone.

**Focus movement.** Validation failure: the error summary renders
immediately after the page heading, before the form, so it is the first
thing a re-rendered page presents; a small Stimulus enhancement additionally
calls `.focus()` on it, JS-optional since server-rendered order already puts
it first. Modal open: focus moves to the modal heading. Modal close: focus
returns to the triggering control. Impersonation entry/exit: focus moves to
the banner / to the page heading respectively.

**Target size (SC 2.5.8, AA).** ≥24×24 CSS px for every interactive
control, applied concretely to: the checkbox hit target (14px visual mark
padded to 24px, per Component inventory), icon-only row actions in data
tables (edit/impersonate/deactivate), badge/label remove buttons, pagination
controls, and modal close buttons.

**Dragging (SC 2.5.7, AA).** Playlist/drill reordering (AC-04-2, AC-04-12,
AC-04-31) ships pointer drag and always-visible "Move up"/"Move down"
buttons as two paths to the same result, not drag-only with buttons as an
afterthought (Component inventory, `playlist-reorder_controller.js`).

**Contrast against the dynamic brand color (SC 1.4.3 text, SC 1.4.11
non-text UI).** A trainer-chosen accent (AC-01-61, free hex picker) can fail
contrast in ways a fixed palette cannot. Two separate rules, because text
and non-text UI have different thresholds:

- *Non-text UI* (button fills, focus rings, borders, toggle tracks) only
  needs 3:1 against adjacent colors. `--brand-on-primary` (Brand color
  derivation section) guarantees this for anything painted *on* the brand
  color by choosing a near-white or near-black foreground per-render from
  the trainer's actual luminance — no fixed hue can get this right for every
  possible trainer color, which is exactly why it is computed, not stored.
- *Text* needs 4.5:1 (normal) / 3:1 (large/UI-component text). `--brand-primary`
  or `--brand-primary-deep` is used for small brand-colored text (links, an
  active-nav indicator) **only if the deep shade independently clears 4.5:1
  against `--color-surface`**; if a trainer's hue stays too light even at
  its darkest documented shade (e.g. a pale yellow), brand color is
  restricted to non-text UI for that trainer and text falls back to
  `--color-text-primary`, with the brand accent surviving only as an
  underline, icon, or border (Decisions).

**Non-color cues.** System flags differ by icon, not color alone (Component
inventory, Badges/pills). Availability indicators (green/gray/red,
AC-02-13) pair color with text ("Available"/"Unknown"/"Busy"), not color
alone. Required-field marking is text, not color alone.

**Form error association (SC 3.3.1, 3.3.2, 4.1.2).** One platform-wide Symfony
form theme (`platform/components/_form_theme.html.twig`) overrides
`form_row`/`form_errors` so every field gets `aria-invalid="true"` and
`aria-describedby` pointing at its error `<span id>` by default — a shared
theme, not a discipline every template author has to remember per field
(Decisions).

**Keyboard paths.**

- *Modals*: `Tab`/`Shift+Tab` trapped inside while open; `Escape` closes;
  background content gets `inert` so it cannot receive focus while hidden
  behind the dialog.
- *Menus*: the account menu and any nav overflow use `<details>/<summary>`
  wherever the interaction is a simple open/close disclosure — native,
  keyboard-operable with no ARIA needed. No screen in this inventory
  describes a nested/composite menu that would need a custom `role="menu"`
  pattern; none is introduced speculatively.
- *Context switchers*: native `<select>`, which is keyboard-operable by
  default (arrow keys, type-ahead).

**Live announcements.** Flash messages use `role="status"` (polite,
success/info) or `role="alert"` (assertive, error/warning) — see Component
inventory. Async-feeling actions that are actually synchronous full-page
responses (RSVP, token spend, coupon apply) need no separate live region;
the page re-render itself is the update.

**Sticky chrome and focus (SC 2.4.11, AA).** The impersonation banner is
`position: sticky` inside the layout's normal flow, not `position: fixed`
layered over content, specifically so it cannot sit on top of the first
focusable element as a Super Admin tabs in from page load (Decisions).

**Reduced motion (SC 2.3.3 is AAA, but honored anyway as low-cost).** The
button hover lift/scale (DESIGN_TOKENS.md line 77) and any modal/frame
transition are zeroed under `prefers-reduced-motion: reduce` (Design tokens
section).

**Touch and pointer.** Every button/control meeting the 24px target
minimum above also satisfies touch usability at the sizes this platform
actually specifies (40px inputs, 39×24 toggles with a 24px-tall hit area
already, 22px radios needing the same padding treatment as checkboxes).

**Captions/alternatives.** Learn/Practice video content is embedded
third-party YouTube video; captioning is the creator's responsibility on
YouTube itself, outside this platform's control. AC-04-24's required "Text
Instructions" section accompanying every video is already a text-based
companion to the video content, which partially serves the same purpose —
noted honestly as a partial answer, not a substitute; see Open questions for
whether the platform should require/verify captions before publish (no AC
asks for this).

**Zoom and reflow (SC 1.4.4, 1.4.10).** DESIGN_TOKENS.md's scale is authored
in literal `px` (Design tokens section keeps it that way, matching the
source exactly); this does not conflict with reflow — browser page zoom
scales `px` values along with everything else, and every layout container in
this spec uses relative/fluid widths with `overflow-x: auto` reserved for
genuinely wide content (data tables), never the page body, so 400% zoom
reflows to a single column rather than requiring two-directional scrolling.

**Accessible authentication (SC 3.3.8, AA).** Login/password-reset accept
pasted passwords and do not require solving a puzzle or transcription task
as the only path to authenticate.

**Redundant entry (SC 3.3.7, A — already satisfied by design, noted for
completeness).** Camp-to-account conversion pre-fills the registration form
from the submission's own data (AC-08-23, AC-01-64) rather than asking the
submitter to retype their name and email.

---

## Decisions

| Decision | Chosen | Rejected | Because |
|---|---|---|---|
| Brand shade derivation | PHP, `BrandingProvider`, at render time (already settled — see next row for what this spec adds) | CSS `color-mix()` | `hexToRgb` has no CSS equivalent under the documented `--brand-primary-rgb` contract; contrast computation needs PHP regardless; splitting derivation across two languages risks drift |
| Contrast-safe foreground | Added `--brand-on-primary`, computed by luminance | A fixed white label on every primary button | A trainer-chosen light accent (AC-01-61 permits any hex) could ship unreadable white-on-pale text; DESIGN_TOKENS.md defines no foreground-on-brand token |
| Branding color validation | Non-blocking contrast warning at save | Hard-blocking save on low contrast | AC-01-61 specifies a free hex picker with no stated validation failure mode; blocking would invent a rejection rule the epic never states |
| Status-pill color source | Fixed, brand-independent (error red sourced from `inputs.svg`; positive states reuse `--brand-primary`; neutral/pending use grayscale + icon) | Deriving all status colors from the trainer's brand accent | A trainer whose brand happens to be red/green must not make "Refunded" and "Confirmed" pills collide in meaning; DESIGN_TOKENS.md supplies no separate system success/warning color |
| Error color source | `#FF5E58`, extracted from `inputs.svg` `fill`/`stroke` (both, consistently) | Reusing a shade of the brand accent for errors; inventing a hex | DESIGN_TOKENS.md names an input "error" state (line 83) with no color; this hex is the one place an error state is actually drawn, appearing on both fill and stroke — the narrow-extraction case the task's own context warning permits |
| Checkbox hit target | 14×14 visual (DESIGN_TOKENS.md) padded to a 24×24 hit area | Rendering the checkbox at a literal 14×14px clickable area | WCAG 2.2 SC 2.5.8 (AA) requires ≥24×24 CSS px with no exception that applies here; `form_control_element.svg`'s own unchecked-checkbox stroke box is already 24×24, corroborating a padded target |
| Checkbox/toggle-off gray | Reused `--gray-400` (#868686) | A new, undocumented `#AAAAAA` token | The SVG evidence (`#AAAAAA`) is one step off the nearest documented grayscale stop; a seventh gray for a one-step difference isn't worth a new token |
| Button padding order | Reinterpreted as `<vertical> <horizontal>` (8px 18px sm / 10px 24px md) | Reading DESIGN_TOKENS.md's "18px 8px" literally as CSS `padding: v h` | Read literally, "sm" would carry 18px vertical padding alone — taller than the 40px input — contradicting "sm" being the more compact size; the swapped reading produces an sm button close to input height |
| Drag-and-drop reordering | Pointer drag **and** always-visible Move up/down buttons, both real | Drag-only reordering | WCAG 2.2 SC 2.5.7 (AA) requires a non-dragging alternative; buttons are the baseline control, drag is additive |
| Destructive actions | Dedicated Danger button (`--color-error` fill) | Styling every delete/deactivate/remove as a plain Secondary button | Delete/deactivate/remove confirmations recur on nearly every entity (labels, flags, playlists, drills, users, forms, coupons); DESIGN_TOKENS.md's three named variants don't cover this, and an undifferentiated destructive action is a real misclick risk |
| Confirmation modals | Always backed by two real routes (GET confirm, POST commit); `<dialog>` + Stimulus as enhancement | A JS-only modal with no server-rendered fallback | "What must still work with JavaScript disabled" requires every destructive action completable via plain HTML |
| Feature-toggled nav items | Hidden entirely when a feature is off | Shown disabled/grayed | Architecture: "a disabled feature is denied at the same place as any other permission" (line 431); a clickable-looking item that 403s is the generic-blank-screen-behind-an-error the frontend-design skill prohibits |
| Impersonation banner positioning | In-flow `position: sticky` | `position: fixed` overlay | WCAG 2.2 SC 2.4.11 (AA) — a fixed banner can sit on top of the first keyboard-focused element as a user tabs from page load |
| Brand color on text | `--brand-primary-deep` only if it independently clears 4.5:1 on `--color-surface`; otherwise text falls back to `--color-text-primary` | Always using some brand-derived shade for links/accented text | A trainer-chosen hue can fail 4.5:1 in every shade of itself (SC 1.4.3); the fallback guarantees legible text regardless of brand choice |
| Typeface | System font stack, pending client input | Selecting/licensing a specific webfont | Every text-bearing SVG has type flattened to outlines on export; no `font-family` survives in any of them, and DESIGN_TOKENS.md's typography table gives size/line-height/weight only — never a family. Naming one would invent a requirement |
| Card/page background tokens | Added `--color-surface` (white) / `--color-surface-sunken` (`--gray-50`) | Leaving background undefined, inferring per-component | DESIGN_TOKENS.md defines a grayscale ramp and a shadow system but never states which end is "page" vs "card"; some named surface token is required to use either at all |
| Theme | Light mode only | Also specifying a dark theme | No epic, AC, or DESIGN_TOKENS.md line requests one; see Open questions |
| Twig directory layout | Mirrors the architecture's nine-module map | A flat `templates/` tree, or a role-first tree | Keeps the one place a template lives predictable from the already-settled module map, and keeps `Platform`'s shared chrome visibly separate from epic-owned screens |
| Player shell scope | One shell for parent, adult-self, and child-flagged accounts, narrowed by a nav-context view model | A fifth "child" shell | `ROLE_PLAYER` is one of exactly four fixed roles (BR-01-7); a child account is a permission state on that role, not a fifth role |
| Turbo Streams | Not used | Using Turbo Streams for the async refund/notification fan-out | No Mercure/WebSocket service exists in the five-service Compose deployment; the epics' own async paths already resolve to eventual-consistency-on-next-load, not live push |
| Search/filter transport | Turbo Frame swap, with a plain submit button always present too | JS-only live search with no submit fallback | Matches the skill's progressive-enhancement default; the ACs describe the result, not the transport |
| YouTube auto-complete-on-play | JS-only enhancement; an always-visible manual "Mark Complete" control is the real baseline | Making auto-completion the only path | AC-04-25's trigger (an embedded third-party player's "play" event) is only observable via JS; the underlying mutation must still work with JS disabled |
| Form-error wiring | One platform-wide Twig form theme overriding `form_row`/`form_errors` | Hand-wiring `aria-describedby`/`aria-invalid` per template | Makes correct error association the default everywhere instead of a discipline every template author must remember |
| Pagination default | 50 rows/page everywhere a data table appears | A different page size per screen | The epics independently converge on 50 in four places (AC-03-5, AC-02-56, AC-07-12, AC-07-26); matching it is consistency, not a new decision |
| Component partial mechanism | Plain Twig `include`/`macro` | `symfony/ux-twig-component` | Not named in `architect-architecture.md` § "Stack"; assuming an unlisted package would invent a dependency, and plain Twig achieves the same reuse |
| Asset pipeline scope | Full `--webapp` skeleton (AssetMapper + Stimulus + Turbo) | AssetMapper alone, with Stimulus/Turbo added later | Architecture names AssetMapper as "the current `--webapp` default" (line 638), which scaffolds Stimulus and Turbo alongside it; "server-rendered Twig with progressive enhancement" (line 40) presumes both already exist |

---

## Open questions

| # | Question | Why it's open |
|---|---|---|
| Q-FE-1 | What typeface, if any, beyond the system-font fallback? | No source SVG retains a `font-family` (all text is flattened to outlines on export) and DESIGN_TOKENS.md's typography table gives size/line-height/weight only |
| Q-FE-2 | Is a dark theme required? | Not requested anywhere in the epics or DESIGN_TOKENS.md; this spec is light-mode only pending confirmation |
| Q-FE-3 | Is a dedicated system success/warning (amber) color wanted? | None is sourced anywhere; pending/warning-state status pills currently use a neutral gray treatment plus an icon rather than an invented color |
| Q-FE-4 | Do the unexplained hex values in `inputs.svg`/`buttons.svg`/`event_builder.svg` (e.g. `#667185`, `#98A2B3`, `#B6D8FF`, `#099137`, `#101928`) carry a system meaning DESIGN_TOKENS.md omitted? | Each appeared once in a narrow grep with no surrounding context; assigning them a role without reading the full file (forbidden by the task's own context budget) would be guessing |
| Q-FE-5 | Which mechanism issues the CSP nonce consumed as `csp_nonce('style')`? | `architect-architecture.md` commits to a nonce-carrying inline style block but not to how the nonce is generated or wired into the CSP header; treated here as an infrastructure/security concern outside this spec's scope |
| Q-FE-6 | Should the platform require or verify captions on trainer-submitted YouTube videos before publish? | No AC addresses this; flagged only as an accessibility completeness gap, not a stated requirement |
| Q-FE-7 | Is the input focus ring's exact recipe (`0 0 0 3px` at 25% alpha) correct, or does the design owner have a specific Figma value beyond what survives in the exported SVGs? | DESIGN_TOKENS.md says "soft glow ring" (line 82) with no spread or opacity number; this spec picked a concrete value to be buildable |

