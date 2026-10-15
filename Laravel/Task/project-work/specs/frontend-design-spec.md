# PracticePerfect — Frontend Design Specification (Laravel)

Server-rendered Blade + Vite frontend for PracticePerfect on Laravel. This
document defines the design-token system, the white-label branding
architecture, the Blade layout hierarchy, the component inventory, asset
organization, a page inventory per epic, and the WCAG 2.2 AA accessibility
contract baked into the components.

**Stack**: Blade templates and layouts, Vite as the asset pipeline, Tailwind
CSS (v4, CSS-first `@theme`) generating utilities from the same CSS custom
properties that are the actual source of truth. Alpine.js for narrow,
justified local interactivity; Livewire for a handful of genuinely dynamic
islands. The baseline works with JavaScript disabled — this is
progressive enhancement on top of full server round-trips, not a SPA.

## Purpose and sources

**Read in full**: `Task/designs/DESIGN_TOKENS.md` (the distilled, Laravel-
specific source of record for every token); `specs/requirements-analyst-
open-questions.md` Section A (settled owner decisions A1–A12 and the fee
call); `specs/requirements-analyst-epic-01..08-*-spec.md` — `## User
scenarios` in full and `## Acceptance criteria` in full for every epic (not
skimmed — the page inventory in §8 cites specific AC-NN-n numbers and those
need to be right); `specs/database-designer-schema.md` (narrow: the
`trainer_branding_settings`, `trainer`, `account_trainer_link`, and
`public_tenant_code` table definitions, which settle where and how brand
data and tenant resolution are actually stored); `specs/council-sharelink-
tenant-resolution.md` (the tenancy-resolution finding and recommendation —
per `specs/MANIFEST.md`, this is a **translate** carry-over: the finding is
framework-independent, but its wiring names Symfony mechanisms and is
re-expressed here as Laravel middleware).

**Extracted narrowly** (never read whole, per the size warning):
`Task/designs/color_transformation_example1.svg` and `_example2.svg` (496 B
and 553 B — read in full, they're tiny) confirm the accent ramp is a literal
`color-mix()` at 30/60/90% toward white and black — the exact math is in
§2.3. `Task/designs/form_control_element.svg` and `default_logo.svg` (2–2.5
KB — read in full) confirm checkbox/radio/toggle geometry and give the
platform's fallback logo mark. `buttons.svg` (413 KB), `inputs.svg` (310
KB), and `event_builder.svg` (929 KB) were never read whole — only
`grep -o 'fill="#[0-9A-Fa-f]*"'` / `stroke=` / `font-family=` were run
against them, to confirm semantic colors (error, success) and cross-check
that the real mockups use the same palette DESIGN_TOKENS.md documents.
Discrepancies found this way are in §11 Open questions, not silently
resolved.

**Not read**: `Task/Epics/` originals (client-owned, read-only, out of
scope per the brief). `architect-architecture.md` / `api-designer-spec.md`
do not exist in this Laravel edition (per `specs/MANIFEST.md`, deliberately
absent — their Symfony mechanism doesn't translate as prose). Where this
document needs a fact only those files would have had, it is sourced
instead from `council-sharelink-tenant-resolution.md`'s direct quotes of
them, cited as such.

### The four roles

Epic-01 fixes exactly four roles as constants, not runtime-editable data
(Section A, decision A2; BR-01-7): **Super Admin**, **Trainer**, **Coach**,
**Player/Parent**. Player and Parent are one role slot behaving as two data
subjects — a parent account can itself train (self), and can hold child
profiles with a separate, permission-constrained login per child
(AC-01-20). Throughout this document, a page inventory row's "Role" column
says **Player** for the shared adult/parent-acting-as-self experience,
**Parent** where behavior is specific to a parent managing a child's
profile or approvals, and **Child (constrained)** where the epic describes
materially different rendered output for a child's own login (AC-01-29
through AC-01-32) rather than just a hidden button.

Per Section A decision **A6**, multi-trainer players and coaches get
**separated, isolated contexts, never a combined cross-trainer view**, on
any player-facing screen (Epic-01 AC-01-15, BR-01-10). Every authenticated
player/parent/coach page in §8 is implicitly scoped to "the current trainer
context"; nowhere in this document is a merged view designed, including the
"Family Overview" Epic-05 floats in AC-05-24 — Section A explicitly drops
it.

---

## 1. Design token system — two layers

The brief for this document is unambiguous about the deliverable that
matters most: **white-label branding must be a runtime value, not a
rebuild.** That single requirement decides the whole token architecture,
so it's stated first and shapes every section after it.

Tokens split into two layers with different lifecycles:

| Layer | Compiled | Scope | Contains |
|---|---|---|---|
| **Static** | Once, at build time (Vite) | Identical for every trainer | Typography scale, grayscale, spacing, radius, shadows, control geometry, fixed semantic colors (error/success/warning) |
| **Runtime** | Per request | One trainer | Exactly two custom properties: `--brand-primary`, `--brand-on-primary` |

Nothing else is per-tenant. The runtime layer is deliberately as small as
it can be — everything a trainer's brand touches beyond those two raw
values (tints, shades, gradients, glow shadows, a safe focus-ring color) is
*derived in the browser* from them via CSS `color-mix()`, at zero
additional per-request cost. §4 justifies this split against the
alternative of precomputing a full palette in PHP.

### 1.1 Why exactly two runtime properties

`Task/designs/color_transformation_example2.svg` gives eight color swatches
that, decoded, are exact linear mixes of one base hue with white and black:

| Swatch | Hex | Reverse-engineered formula | Verified |
|---|---|---|---|
| lightest | `#E6FFE6` | `mix(base, white, 90%)` | `230,255,230` ✓ |
| | `#99FF99` | `mix(base, white, 60%)` | `153,255,153` ✓ |
| | `#4DFF4D` | `mix(base, white, 30%)` | `76.5,255,76.5` ✓ |
| base | `#00FF00` | — | — |
| | `#00B300` | `mix(base, black, 30%)` | `0,178.5,0` ✓ |
| | `#006600` | `mix(base, black, 60%)` | `0,102,0` ✓ |
| darkest | `#001900` | `mix(base, black, 90%)` | `0,25.5,0` ✓ |

Every one of the seven derived stops matches its formula to the nearest
integer channel value. This is not an approximation the design team
eyeballed — it is a literal `color-mix(in srgb, base P%, white/black
(100-P)%)` at 30/60/90%. Because the browser can compute this exactly from
one input, PHP does not need to precompute a palette; it needs to emit one
color. The second property, `--brand-on-primary`, exists because
`color-mix()` cannot make a contrast *decision* — picking black or white
text against an arbitrary fill requires a luminance computation with a
branch, which CSS cannot do on its own. That one decision is the entire
reason PHP is involved at all. §4 covers where and when it runs, and what
happens when neither black nor white clears AA contrast.

---

## 2. CSS custom properties — the full set

```css
/* ==========================================================================
   resources/css/tokens/static.css
   Compiled once by Vite. Identical for every trainer. No per-tenant value
   is ever written into this file or into anything it defines.
   ========================================================================== */

:root {
  /* ---- Typography scale (Task/designs/text.svg, via DESIGN_TOKENS.md) --- */
  --text-hero-title-size: 30px;    --text-hero-title-leading: 38px;    --text-hero-title-weight: 700;
  --text-section-title-size: 22px; --text-section-title-leading: 28px; --text-section-title-weight: 700;
  --text-block-title-size: 18px;   --text-block-title-leading: 24px;   --text-block-title-weight: 700;
  --text-card-title-size: 16px;    --text-card-title-leading: 22px;    --text-card-title-weight: 600;
  --text-body-lg-size: 16px;       --text-body-lg-leading: 26px;       --text-body-lg-weight: 400;
  --text-body-size: 14px;          --text-body-leading: 22px;          --text-body-weight: 400;
  --text-caption-size: 12px;       --text-caption-leading: 18px;       --text-caption-weight: 400;
  --text-eyebrow-size: 11px;       --text-eyebrow-leading: 16px;       --text-eyebrow-weight: 600;

  /* No font-family is named or extractable from the design source (the SVG
     exports carry no font-family attribute — text is likely outlined to
     paths). A system stack is used rather than inventing a brand typeface;
     it also avoids a render-blocking webfont request, which helps every
     "loads in <2s" acceptance criterion (AC-01-77, AC-03-42, AC-08-39, …).
     See §11 Open questions. */
  --font-family-base: ui-sans-serif, system-ui, -apple-system, "Segoe UI",
    Roboto, Helvetica, Arial, sans-serif;

  /* ---- Grayscale (Task/designs/color_transformation_example1.svg) ------- */
  --gray-50:  #F3F3F3;
  --gray-200: #CFCFCF;
  --gray-400: #868686;
  --gray-600: #5E5E5E;
  --gray-800: #363636;
  --gray-950: #0D0D0D;

  /* ---- Semantic neutrals — derived from the grayscale, each one
     contrast-verified against a white surface (ratios computed in §9) ---- */
  --color-bg-page:         var(--gray-50);
  --color-bg-surface:      #FFFFFF;
  --color-bg-surface-alt:  var(--gray-50);
  --color-border-subtle:   var(--gray-200); /* ~1.6:1 vs white — decorative dividers ONLY, never a meaningful boundary */
  --color-border-default:  var(--gray-400); /* ~3.6:1 vs white — clears the 3:1 UI-component floor; default for inputs/cards/tables */
  --color-border-strong:   var(--gray-600);
  --color-text-primary:    var(--gray-800); /* ~12:1 vs white */
  --color-text-secondary:  var(--gray-600); /* ~6.5:1 vs white */
  --color-text-disabled:   var(--gray-400); /* disabled text is exempt from AA contrast; kept legible anyway */
  --color-text-inverse:    #FFFFFF;

  /* ---- Fixed semantic state colors — NEVER overridden per trainer.
     Kept independent of --brand-primary on purpose: a trainer's accent can
     legally be red or green, and error/success must stay recognizable
     regardless of what a given trainer picked. ---- */
  --color-danger:       #FF5E58; /* Task/designs/inputs.svg — ~3.0:1 vs white: safe for icons/borders/fills, NOT small text */
  --color-danger-text:  color-mix(in srgb, var(--color-danger) 70%, black 30%);  /* clears 4.5:1 — use for error copy */
  --color-danger-bg:    color-mix(in srgb, var(--color-danger) 10%, white 90%);
  --color-success:      #099137; /* Task/designs/inputs.svg — ~4.1:1 vs white: safe for icons/fills, NOT small text */
  --color-success-text: color-mix(in srgb, var(--color-success) 80%, black 20%); /* clears 4.5:1 — use for success copy */
  --color-success-bg:   color-mix(in srgb, var(--color-success) 10%, white 90%);
  --color-warning:      #B7791F; /* NOT present in the design source — proposed, contrast-checked; see §11 Open questions */
  --color-warning-text: color-mix(in srgb, var(--color-warning) 85%, black 15%);
  --color-warning-bg:   color-mix(in srgb, var(--color-warning) 10%, white 90%);

  /* ---- Spacing scale ---- */
  --space-xxs: 4px; --space-xs: 8px; --space-sm: 12px; --space-md: 16px;
  --space-lg: 24px; --space-xl: 32px; --space-xxl: 40px;

  /* ---- Border radius ---- */
  --radius-xs: 6px; --radius-sm: 10px; --radius-md: 16px;
  --radius-lg: 24px; --radius-xl: 32px; --radius-pill: 999px;

  /* ---- Shadows — brand-independent card elevation ---- */
  --shadow-card-soft:   0 2px 8px rgba(0, 0, 0, 0.2);
  --shadow-card-strong: 0 4px 16px rgba(0, 0, 0, 0.3);

  /* ---- Control geometry ---- */
  --control-input-height: 40px;
  --control-input-padding-inline: 10px;
  --control-btn-sm-padding-inline: 18px; --control-btn-sm-padding-block: 8px;
  --control-btn-md-padding-inline: 24px; --control-btn-md-padding-block: 10px;
  --control-checkbox-size: 14px;   /* visual glyph — see §9 for the required hit-area padding */
  --control-radio-size: 22px;      /* visual glyph — see §9 for the required hit-area padding */
  --control-toggle-width: 39px; --control-toggle-height: 24px; /* already clears the 24px target minimum */
  --control-hit-min: 24px;         /* WCAG 2.2 SC 2.5.8 minimum target size */

  /* ---- Motion ---- */
  --ease-standard: cubic-bezier(0.2, 0, 0, 1);
  --duration-fast: 120ms;
  --duration-standard: 200ms;
}

@media (prefers-reduced-motion: reduce) {
  :root { --duration-fast: 0ms; --duration-standard: 0ms; }
}
```

```css
/* ==========================================================================
   resources/css/tokens/runtime.css
   Ships with SAFE FALLBACK values only. The two custom properties below
   are overwritten per request by an inline <style> block a view composer
   writes into the <head> (see §4.4). This file exists so a page never
   renders unstyled on a cache-miss edge case, and so the derivation
   formulas live in one committed, reviewable place rather than being
   generated as an inline string per request.
   ========================================================================== */

:root {
  --brand-primary:    #00B300; /* platform default — see §10 Decisions */
  --brand-on-primary: #FFFFFF; /* platform default */

  /* Everything below is derived from the two properties above, in the
     browser, at zero per-request PHP cost. Nothing past this line is ever
     computed or written by PHP. */
  --brand-primary-tint-90:  color-mix(in srgb, var(--brand-primary) 10%, white 90%);
  --brand-primary-tint-60:  color-mix(in srgb, var(--brand-primary) 40%, white 60%);
  --brand-primary-soft:     color-mix(in srgb, var(--brand-primary) 70%, white 30%);
  --brand-primary-deep:     color-mix(in srgb, var(--brand-primary) 70%, black 30%);
  --brand-primary-shade-60: color-mix(in srgb, var(--brand-primary) 40%, black 60%);
  --brand-primary-shade-90: color-mix(in srgb, var(--brand-primary) 10%, black 90%);

  --brand-gradient: linear-gradient(135deg, var(--brand-primary), var(--brand-primary-deep));

  /* Focus rings, input focus borders, and other thin accents use
     --brand-primary-deep rather than the raw base — see §9 for why this
     is required, not stylistic. */
  --brand-ring: color-mix(in srgb, var(--brand-primary-deep) 40%, transparent 60%);

  /* rgba()-style alpha via modern relative color syntax — no separate
     --brand-primary-rgb triplet needs to be emitted by PHP (see §10). */
  --shadow-button-primary:       0 10px 30px rgb(from var(--brand-primary) r g b / 55%);
  --shadow-button-primary-hover: 0 12px 34px rgb(from var(--brand-primary) r g b / 70%);
}
```

### 2.1 Tailwind wiring (v4, CSS-first)

```css
/* resources/css/app.css — authenticated portal entry */
@import "tailwindcss";
@import "./tokens/static.css";
@import "./tokens/runtime.css";

@theme {
  /* Only the STATIC layer is registered here, so Tailwind can generate
     utilities (text-body, bg-surface, rounded-md, shadow-card-soft, p-lg, …)
     at build time. @theme values are resolved when Vite compiles — a
     per-request brand value cannot live here. Representative subset shown;
     the remaining static tokens in §2 follow the same --key: var(--token)
     pattern. */
  --font-size-hero-title: var(--text-hero-title-size);
  --font-size-body: var(--text-body-size);
  --spacing-xxs: var(--space-xxs);
  --spacing-lg: var(--space-lg);
  --radius-md: var(--radius-md);
  --color-border-default: var(--color-border-default);
  --color-danger-text: var(--color-danger-text);
  --shadow-card-soft: var(--shadow-card-soft);
}

@import "./base.css";
@import "./components/index.css";
```

Brand utilities are **arbitrary-value classes**, never `@theme` entries,
because `@theme` is a build-time table and the brand value is a per-request
one: `class="bg-[var(--brand-primary)] text-[var(--brand-on-primary)]"`,
`class="focus-visible:ring-2 focus-visible:ring-[var(--brand-ring)]"`.

**Why Tailwind at all**: with roughly 120 pages across eight epics (§8),
hand-rolling BEM-style classes for every spacing/radius/color combination
is slower and drifts from the token spec over time. Tailwind here is
strictly an *application* mechanism for tokens that already have one
source of truth in `tokens/static.css` and `tokens/runtime.css` — it does
not introduce a second, competing palette. A prebuilt admin-UI kit was
rejected for the same reason a hand-rolled utility-free approach was: both
fight a client-supplied token spec instead of applying it (see §10).

---

## 3. Runtime layer contract

Stated precisely, once, so §4 can focus on mechanism rather than
re-litigating scope:

| Property | Type | Set by | Validated | Platform default |
|---|---|---|---|---|
| `--brand-primary` | `#RRGGBB` | Server, per request | `^#[0-9A-Fa-f]{6}$` — matches the `trainer_branding_settings.primary_color_hex` CHECK constraint (`specs/database-designer-schema.md`) | `#00B300` |
| `--brand-on-primary` | `#000000` or `#FFFFFF` (exactly one of two literals) | Server, computed | Never an arbitrary color — always the AA-verified pick, or a corrected fallback (§4.3) | `#FFFFFF` |

That is the complete answer to "exactly which properties are
runtime-overridable, kept minimal": **two**, both scalar, both cheap to
validate, both listed above. No other static token from §2 is reachable
from trainer input. The logo is not a CSS property at all — it is a file
path rendered through an `<img>` tag (§4.5).

---

## 4. White-label branding architecture

This is the section the brief calls out as the deliverable that matters
most. It covers, in request order: where the tenant is known (§4.1), where
the two runtime properties are computed (§4.2), what happens when a
trainer's chosen color can't be made accessible against either black or
white text (§4.3), how "applies immediately" is honored without paying a
full DB read on every request (§4.4), and how branding reaches a page that
has no authenticated session at all (§4.5).

### 4.1 Where the tenant is resolved

`specs/council-sharelink-tenant-resolution.md` establishes (and
`specs/database-designer-schema.md`'s `account_trainer_link` and
`public_tenant_code` table definitions confirm at the schema level) that
tenant resolution has to run **before** most of the request — before the
controller, before any tenant-scoped query, before branding can be looked
up — because on the routes that matter most (a ShareLink landing, a coach
invite, a public camp form) nothing else in the request carries the
trainer's identity yet. The council's finding: a tenant-agnostic
`code → trainer` lookup isn't one resolution strategy among several, it's
the *precondition* for all of them, because reading the trainer-scoped
`ShareLink`/`Form` row to learn its owner would itself require already
knowing the tenant — circular. The fix is a small, deliberately global,
RLS-exempt mapping table (`public_tenant_code`) that exists for no purpose
other than answering "which trainer does this code belong to," read before
tenant scoping engages.

Translated into Laravel (the council document itself names Symfony
mechanisms — `TenantResolver`, `kernel.request` — that this project
re-expresses as follows, not reuses):

- **`App\Http\Middleware\ResolveTenantContext`**, registered in the `web`
  middleware group, running after session/auth start but before any
  controller executes. It resolves the tenant in priority order:
  1. An authenticated **Trainer** viewing their own portal, or a **Super
     Admin** in an active impersonation session — the account *is* the
     tenant, or explicitly stands in for one.
  2. The session-held **current trainer context** for a Player/Parent or
     Coach (the "context switcher" from AC-01-15/18/32) — read from the
     session, then *validated* against the global `account_trainer_link`
     table before being trusted. An unvalidated session value resolves
     nothing.
  3. On an explicit route allow-list only, a **public code carried in the
     route** (`{code}` on `/join/{code}`, `/join/{code}/associate`,
     `/invite/{code}`, and `/forms/{code}` plus its sibling routes — the
     literal URL shapes AC-03-59, AC-03-50, and AC-08-13 already specify)
     — resolved by looking up `public_tenant_code WHERE code = :code`, a
     global, unscoped table holding nothing but `{code, trainer_id, kind,
     reference_id}`. On these routes, source 3 outranks source 2: an
     existing player accepting a *different* trainer's link must land in
     the new trainer's tenant, not silently re-use whatever trainer their
     session happened to be scoped to a moment before.
  - Code not found → the resolver sets no tenant and the request 404s,
    indistinguishable from any unknown URL. This is deliberately **not**
    audit-logged (an audit entry per anonymous scanner probe would make
    the audit log an amplification target); it is rate-limited and
    counted instead.
- The resolved tenant is bound into the service container for the rest of
  the request (e.g. `app()->instance(TenantContext::class, ...)`), which
  is what every tenant-scoped Eloquent global scope, Policy, and the
  branding lookup in §4.5 read from. `MembershipService`-equivalent code
  (creating a `PlayerTrainerMembership`) never opens its own tenant scope
  — it only ever writes under whatever `ResolveTenantContext` already
  decided, keeping exactly one place in the codebase capable of deciding
  the tenant for a request.

### 4.2 Where the brand ramp is derived

**PHP computes and emits exactly the two properties in §3. CSS
`color-mix()` derives every tint, shade, gradient stop, and glow-shadow
alpha from them, in the browser, at zero additional per-request cost.**

This is not a default kept for convenience — it's forced by what §1.1
already proved: the design system's own ramp *is* a linear `color-mix()`,
verified to the nearest integer channel value against the shipped example
swatches. There is no information in a precomputed seven-color palette
that the browser can't reconstruct itself from one hex value and two
percentages. Precomputing it in PHP would mean:

- A larger runtime payload per request (seven-plus colors instead of two).
- A migration/cache-shape change every time a new derived shade is needed
  (a new hover state, a new chart accent) — versus a CSS-only change today.
- Two places that can disagree about what "the brand ramp" is, instead of
  one formula the browser always applies consistently.

The one thing CSS categorically cannot do is the `--brand-on-primary`
*decision* — contrast is a threshold comparison with a branch, not a
blend. That is the entire, minimal reason PHP is in this path at all, and
§4.3 is what it does with that responsibility.

### 4.3 Contrast safety: when neither black nor white clears AA

A trainer can enter any 6-digit hex through a free-text color picker
(AC-01-61). Nothing constrains it to be dark, light, or even saturated —
so it is not safe to assume one of {black, white} will always pass 4.5:1
against it. A mid-tone, medium-saturation color (a medium orange, a
muted violet, a mid-blue) can fail both.

`App\Services\Branding\BrandColorResolver` runs this pass **once, at save
time** (not per request — see §4.4 for why that's cheap):

1. Validate the submitted hex against `^#[0-9A-Fa-f]{6}$` (Form Request
   rule, mirroring the DB CHECK constraint).
2. Compute relative luminance (the WCAG formula: sRGB → linearized RGB →
   `0.2126R + 0.7152G + 0.0722B`) and the contrast ratio against pure white
   and pure black.
3. **If either clears 4.5:1** (the normal-text AA threshold — the
   applicable bar, because `--brand-on-primary` labels sit on solid button
   fills and filled badges at body text size): pick the winner as
   `--brand-on-primary`. Ship the trainer's hex **unmodified** as
   `--brand-primary`. The large majority of real accent colors land here —
   their brand is respected exactly as entered.
4. **If both fail**: walk the same `color-mix()` curve the ramp already
   uses, in 5% steps toward whichever pole had the higher (closer-to-
   passing) ratio in step 2, recomputing contrast at each step, and stop
   at the first step where either pole clears 4.5:1. The walk is capped at
   a 40% shift.
5. **If even the capped walk fails** (only near-gray, barely-a-hue inputs
   reach this): fall back to the platform default (`#00B300`) for
   `--brand-primary`, and show a persistent notice on the trainer's
   branding settings page ("This color doesn't have enough contrast for
   text — using the default until you choose another"). The trainer's
   choice is never silently discarded, and an inaccessible combination is
   never shipped.
6. As a defensive, independent check in the same pass: verify
   `--brand-primary-deep` (the 30%-toward-black stop, computed in PHP with
   the identical percentage CSS will use, purely to check it — not to emit
   it) clears 3:1 against white. Darkening toward black from any starting
   point only increases contrast against a white page, so this
   essentially never fails in practice, but it is verified rather than
   assumed, because focus rings and input borders use this value directly
   (§9) and a UI-component-contrast failure there is a keyboard-visibility
   defect, not a cosmetic one.

One consequence worth stating plainly: `--brand-primary` is *never* used
as small foreground text directly on the neutral page background (a
caption, a body-copy link). That use case would need a second,
independently-computed "safe as foreground on white" variant, and nothing
in the epics asks for brand-colored body text. Restricting raw
`--brand-primary` to **fills** (buttons, active-state backgrounds, badges,
progress bars) and to **large-scale/graphical** elements (≥24px icons, the
3:1 threshold) avoids needing that third computed property at all — one
more way the runtime-overridable set stays at two.

### 4.4 Caching: honoring "applied immediately, nothing cached"

`specs/database-designer-schema.md` states plainly against
`trainer_branding_settings.updated_at`: "AC-01-62: applied immediately,
nothing cached." Read as a literal zero-caching mandate, every page view
would re-read the table and re-run the luminance computation in §4.3 —
correct, but wasteful on a value that changes rarely and is read on every
single request. The requirement the AC actually protects is narrower and
more useful: **no request may ever render stale branding after a save.**

The design here is write-through invalidation, not "no cache":

- `BrandColorResolver`'s output — `{effective_primary_hex, on_primary}` —
  is cached under `branding:{trainer_id}` with a short safety TTL (e.g. 1
  hour) that exists only as a belt-and-suspenders bound, not as the real
  invalidation mechanism.
- The real mechanism: saving branding settings runs `Cache::forget
  ('branding:'.$trainerId)` synchronously, in the same request, before
  responding with the save confirmation. The very next request for that
  trainer — even one a millisecond later — recomputes and repopulates.
  "Applies immediately" holds exactly as strictly as a literal no-cache
  design would, without paying a DB round trip and a luminance
  computation on every page view of every trainer's portal.

A literal zero-cache design was considered and rejected — see §10.

### 4.5 The public, unauthenticated rendering path

This is the case the brief names directly: a camp registration form
(Epic-08, `/forms/{code}`) rendering trainer branding for a visitor who
has never authenticated and has no session-held tenant. Two schema facts
make this tractable:

- `trainer_branding_settings` is explicitly documented as **"No RLS —
  global... Public by nature — rendered on unauthenticated camp forms
  (BR-08-6)"** (`specs/database-designer-schema.md`). In Eloquent terms:
  the `TrainerBrandingSettings` model carries **no tenant global scope**.
  Every other trainer-owned model in this application is scoped to "only
  the resolved tenant may read this row"; this one is deliberately
  excluded from that scope, on the same reasoning the schema doc already
  applies to `account_trainer_link` and `public_tenant_code` — a resolver
  (or, here, a branding lookup that has to work *before* full tenant
  machinery engages) cannot depend on the thing it's establishing.
- `logo_path` is "rendered via `<img>`... never inlined." A trainer can
  upload an SVG (AC-01-60 accepts it). Inlining trainer-supplied SVG
  markup into the page DOM (a `{!! file_get_contents(...) !!}`-style
  pattern) would let a compromised or malicious trainer account inject
  `<script>` or event-handler attributes into every page carrying that
  brand — including pages viewed by players and coaches who never chose
  to trust that account. Rendering exclusively through `<img src="...">`
  treats the upload as an opaque image resource, never as parsed markup,
  closing that off entirely. `Task/designs/default_logo.svg` (a simple
  black monogram mark) is the fallback `<img>` source when `logo_path IS
  NULL`, so the header never shows a broken-image icon before a trainer
  uploads anything.

Request sequence for `GET /forms/{code}`:

1. Route matches; `ResolveTenantContext` (allow-listed for this route)
   resolves `{code}` via `public_tenant_code` → `trainer_id`. Not found →
   404 (§4.1).
2. The same middleware pass reads `TrainerBrandingSettings` for that
   `trainer_id` (unscoped, cached per §4.4) and shares `brandPrimary`,
   `brandOnPrimary`, and `logoUrl` to the view — via `View::share()` for
   the request, consumed by `layouts/trainer-public.blade.php` (§5).
3. **Separately**, the `Epic08\FormController` does its own ordinary
   tenant-scoped read of the actual `Form` row, which needs the tenant
   `ResolveTenantContext` already set in step 1. This can still fail on
   its own terms — inactive, at capacity, deleted — independent of
   branding.
4. Because steps 2 and 3 are independent, a closed or expired form still
   renders **with the trainer's own branding** on its "Registration
   Closed" (AC-08-29) or expired-link state, rather than falling back to
   bare platform styling. That is the concrete, testable meaning of "a
   public layout... that still renders trainer branding."

---

## 5. Blade layout hierarchy

```
resources/views/
  layouts/
    shell.blade.php            <html>/<head> primitive only: meta, @vite entry
                                points, the runtime <style> slot (§3), skip
                                link, favicon. Never used by a page directly —
                                only by the two layouts below.
    public.blade.php           extends shell. PLATFORM-branded (not
                                trainer-branded) chrome: /login, /register,
                                /password/*, /email/verify/*, generic 404/500.
    trainer-public.blade.php   extends shell. TRAINER-branded, unauthenticated
                                chrome: ShareLink landing, coach invite
                                landing, camp/evaluation forms + confirmation
                                + convert-account. Renders --brand-primary /
                                --brand-on-primary / logo via the §4.5 view
                                composer — independent of session state.
    portal.blade.php           extends shell. Authenticated app shell: skip
                                link target, top bar (logo, context switcher,
                                user menu), role-scoped primary nav,
                                impersonation-banner slot, flash-message
                                region, <main> landmark, footer.
  components/
    layout/
      nav-trainer.blade.php, nav-coach.blade.php, nav-player.blade.php,
      nav-admin.blade.php
      context-switcher.blade.php
      impersonation-banner.blade.php
      flash-messages.blade.php
      skip-link.blade.php
    ui/
      button.blade.php, input.blade.php, select.blade.php, textarea.blade.php
      checkbox.blade.php, radio.blade.php, toggle.blade.php
      card.blade.php, badge.blade.php, modal.blade.php, table.blade.php
      pagination.blade.php, empty-state.blade.php, avatar.blade.php,
      progress-bar.blade.php
  trainer/...     feature views — Event Builder, CRM, LPPP authoring, Camps,
  coach/...       Marketing (all under portal.blade.php)
  player/...
  admin/...
  share-links/...  join/associate/invite views (under trainer-public.blade.php)
  forms/...        Epic-08 public form + confirmation + convert (under
                   trainer-public.blade.php)
  auth/...         login/register/password (under public.blade.php)
```

**Why three layouts, not one with conditionals.** `portal.blade.php`
assumes an authenticated, tenant-scoped session — the nav needs a role,
the context switcher needs `account_trainer_link` rows. None of that
exists on a camp form. `trainer-public.blade.php` needs trainer branding
but must explicitly *not* assume auth. `public.blade.php` needs neither —
a login page has no trainer yet, and is deliberately platform-styled so a
visitor isn't shown a brand before they've chosen one. Collapsing these
into one layout behind `@if(auth()->check())` branches would turn §4.5's
guarantee — branding renders even when the form itself is closed or
expired — into something a conditional could accidentally break exactly
on the states that matter most. Three narrow layouts make it structural
instead.

Layouts are Blade components with named slots, not `@extends`/`@section`:

```blade
{{-- resources/views/forms/show.blade.php --}}
<x-layout.trainer-public :trainer="$trainer">
  <x-slot:heading>{{ $form->name }}</x-slot:heading>

  @if ($form->isFull())
    <x-empty-state
      heading="This camp is full"
      body="Contact {{ $trainer->name }} to ask about a waitlist." />
  @elseif (! $form->isActive())
    <x-empty-state heading="Registration closed" />
  @else
    <form method="POST" action="{{ route('forms.submit', $form->code) }}">
      @csrf
      {{-- field partials, §6 --}}
    </form>
  @endif
</x-layout.trainer-public>
```

| Layout | Used by | Auth | Trainer branding |
|---|---|---|---|
| `public.blade.php` | `/login`, `/register`, `/password/*`, `/email/verify/*`, generic error pages | guest | platform default only |
| `trainer-public.blade.php` | `/join/{code}*`, `/invite/{code}`, `/forms/{code}*` | guest, or lightly-authenticated on the `associate` branch (AC-01-13/14) | **required** |
| `portal.blade.php` | everything under `/trainer`, `/coach`, `/admin`, and the bare player/parent paths (`/calendar`, `/reservations`, `/lppp`, …) | authenticated | required |

---

## 6. Component inventory

Every component consumes tokens from §2 by custom property, never a
hard-coded value — that's what makes the runtime layer (§3–§4) actually
reach the rendered page. Deep accessibility rationale (contrast numbers,
hit-area math) lives in §9 and is referenced, not repeated, below.

### 6.1 Button — `<x-button>`

| | |
|---|---|
| Variants | `primary` (fill `--brand-gradient`, text `--brand-on-primary`, `--shadow-button-primary`), `secondary` (`--color-border-default` outline, `--color-text-primary`), `ghost` (transparent, `--color-text-secondary`), `danger` (`--color-danger` fill, white text) |
| Sizes | `sm` (`--control-btn-sm-padding-*`), `md` (`--control-btn-md-padding-*`) |
| Radius | `--radius-sm` |
| States | default; hover (`translateY(-1px) scale(1.02)`, `--shadow-button-primary-hover` — disabled under `prefers-reduced-motion`, §2); `:focus-visible` (`--brand-ring`, §9); `:disabled` (50% opacity, no hover transform, contrast exempt); `aria-busy` (submitting) |
| Element | always a real `<button>` or `<a>` — never a `<div>` with a click handler (semantic-button-link) |
| Usage | `<x-button variant="primary" size="md" type="submit">Save changes</x-button>` |

### 6.2 Input — `<x-input>`, `<x-select>`, `<x-textarea>`

| | |
|---|---|
| Height | `--control-input-height` (40px); textarea grows from the same line-height |
| Padding | `--control-input-padding-inline` |
| Border | `--color-border-default`, `--radius-sm` |
| States | default; `:focus-visible` (border → `--brand-primary-deep`, ring → `--brand-ring`, §9); `[aria-invalid="true"]` (border → `--color-danger`, not `--color-danger-text` — border is non-text UI, 3:1 rule); `:disabled` (`--color-bg-surface-alt`, `--color-text-disabled`) |
| Label | always a real `<label for>` — placeholder is never the only label (forms-labels-required) |
| Error | `aria-describedby` pointing at the error `<p id>`, `role="alert"` (§9, forms-error-messages) |
| Usage | `<x-input name="email" type="email" label="Email" :error="$errors->first('email')" required autocomplete="email" />` |

### 6.3 Checkbox / Radio / Toggle

| | Visual size | Hit area | Fill (checked) |
|---|---|---|---|
| `<x-checkbox>` | `--control-checkbox-size` (14×14, `--radius-xs`-derived corner) | `--control-hit-min` (24×24) via label padding | `--brand-primary` |
| `<x-radio>` | `--control-radio-size` (22×22 circle) | `--control-hit-min` (24×24) via label padding | `--brand-primary` |
| `<x-toggle>` | `--control-toggle-width`×`--control-toggle-height` (39×24 pill) | already ≥24×24 — no augmentation needed | `--brand-primary` |

All three are real `<input type="checkbox/radio">` under custom visual
styling (`appearance: none` + a sibling glyph), never a `<div>`
role-mimicking a form control — this is what keeps them working with
JavaScript disabled: a plain form `POST` still submits their value. The
checkbox/radio hit-area gap is real (§9) and is closed with `padding` on
the wrapping `<label>`, not by resizing the visible glyph past what
§2/DESIGN_TOKENS.md specifies.

```blade
<label class="inline-flex items-center gap-xs p-xs -m-xs">
  <input type="checkbox" name="allow_token_spend" class="peer sr-only">
  <span class="h-[14px] w-[14px] rounded-xs border border-[var(--color-border-default)]
               peer-checked:bg-[var(--brand-primary)] peer-checked:border-transparent
               peer-focus-visible:ring-2 peer-focus-visible:ring-[var(--brand-ring)]"
        aria-hidden="true"></span>
  Allow token spending without approval
</label>
```

### 6.4 Card — `<x-card>`

`--radius-md` default / `--radius-lg` for hero-scale cards, `--space-lg`
padding, `--shadow-card-soft` default → `--shadow-card-strong` on hover
**only if the whole card is a single link/button** (an interactive card is
one focusable element, not a `<div>` wrapping other focusable content in a
way that creates nested/ambiguous targets). Non-interactive cards (a
stats tile, a read-only summary) never get a hover shadow — that would
imply an affordance that isn't there.

### 6.5 Badge — `<x-badge>`

`--radius-pill`, `--text-caption-*`. Three families:

- **Status** (Active/Inactive/Pending, event status, payment status):
  fixed semantic colors (`--color-success-bg`/`-text`, `--color-danger-*`,
  `--color-warning-*`, or neutral `--gray-200`/`--gray-600`) — never
  `--brand-*`, so status stays legible regardless of a trainer's chosen
  accent.
- **Label** (CRM custom labels, AC-03-11): trainer-chosen from a
  **curated preset swatch list**, not free-text hex — this is the one
  place the epics themselves specify presets rather than a picker
  (distinguishing it from `--brand-primary`, which *is* free-text, §4.3).
  Presets are pre-vetted at design time (fill = a light tint of the
  preset, text/border = a pre-darkened variant of the same preset) so no
  runtime contrast computation is needed for labels at all.
- **Flag** (the 8 system flags, Epic-03 Data requirements): icon **plus**
  text always — flags are safety/care-relevant (Injured, Medical
  restriction, Behavior, …) and must never rely on color alone
  (color-not-only-indicator, §9).

### 6.6 Modal — `<x-modal>`

Native `<dialog>`, opened via `.showModal()`. Every modal-gated action
(cancel RSVP, delete a form, deactivate a user, toggle a feature) has a
**plain confirmation page at the same URL the modal's confirm button
posts to** as its true no-JS baseline (`GET` renders the confirmation
content full-page, `POST` performs the action) — the modal is a
progressive enhancement that intercepts the triggering link and renders
the same Blade partial inside a `<dialog>` instead of navigating away.
Escape closes; focus returns to the triggering control on close
(keyboard-focus-trap, §9).

### 6.7 Table — `<x-table>`

Real `<table>`/`<thead>`/`<th scope="col">`/`<tbody>` — never a div-grid
(semantic-table-markup). Dense tables (Users, RSVP List, CRM Players,
Audit Log) sit inside an `overflow-x: auto` wrapper rather than
collapsing to a card layout on narrow viewports — this preserves full
table semantics with no JS dependency (§10 rejects the collapse-to-card
alternative). Sortable columns are real links carrying `?sort=&dir=` query
parameters, so sorting works identically with or without JavaScript.
Pagination wraps Laravel's built-in paginator (`->paginate(50)`, matching
the 50/page figure repeated across AC-01-72, AC-03-5, AC-07-12, AC-07-26),
which already renders accessible prev/next and page links.

### 6.8 Navigation

- **Primary nav** (`nav-trainer` / `-coach` / `-player` / `-admin`):
  `<nav aria-label="Primary">`, current page marked `aria-current="page"`.
- **Context switcher**: a `<form method="POST">` wrapping a native
  `<select>` of the account's `account_trainer_link` rows plus an explicit
  "Switch" submit button — functional with no JavaScript; enhanced with
  Alpine to auto-submit on change for users who have it. A child's own
  login sees the same component rendered with only its own trainer
  contexts, no parent data, no "Me" section (AC-01-32).
- **Impersonation banner**: sticky, `role="status"`, a **fixed platform
  color independent of `--brand-*`** — it must stay visually distinct
  regardless of which trainer's brand is active, since it's a
  security-relevant indicator, not a themed element (AC-01-34). Contains
  the target user's name/role and a real `<a>`/`<button>` "Exit
  Impersonation."
- **Mobile nav toggle**: `<details>`/`<summary>` as the no-JS baseline
  disclosure, enhanced with Alpine for animation and Escape-to-close.

### 6.9 Flash messages — `<x-flash-messages>`

Rendered from Laravel's session flash (`session('status')`) and the
`$errors` bag at the top of `<main>`, every request, no JavaScript
required to appear. Success/info: `role="status" aria-live="polite"`.
Validation/error summaries: `role="alert"`. Icon plus text always — never
a colored bar alone (color-not-only-indicator). Auto-dismiss is a
JavaScript enhancement applied only to success/info messages; error
messages are never auto-dismissed.

### 6.10 Empty state — `<x-empty-state>`

`heading` + `body` + optional `action` slot. Several exact strings are
already specified by the acceptance criteria and are reused verbatim,
never reworded: *"No players found. Try different search terms."*
(AC-03-9), *"No players match your filters. Try adjusting criteria."*
(AC-03-26), *"No drills match your filters. Try adjusting criteria."*
(AC-04-9), *"Event Full - No spots available"* (AC-02-27).

---

## 7. Vite and asset organization

```
resources/
  css/
    app.css          authenticated portal entry point: imports
                      tokens/static.css, tokens/runtime.css, Tailwind
                      (§2.1), base.css, components/*.css, utilities.css
    public.css        entry for public.blade.php + trainer-public.blade.php:
                      the same tokens, but excludes authenticated-app-only
                      component styles (Event Builder authoring chrome, CRM
                      filter panel, Super Admin tables, …)
  js/
    app.js            Alpine.js init + registered modules: modal.js,
                      context-switcher.js, char-counter.js, drag-reorder.js,
                      video-player.js, toast.js. Livewire's runtime is
                      included only here.
    public.js         minimal: a <dialog> feature check and light inline
                      validation. No authenticated-app JS ships to it.
  views/...
vite.config.js
```

```js
// vite.config.js
import { defineConfig } from 'vite';
import laravel from 'laravel-vite-plugin';

export default defineConfig({
  plugins: [
    laravel({
      input: [
        'resources/css/app.css', 'resources/js/app.js',
        'resources/css/public.css', 'resources/js/public.js',
      ],
      refresh: true,
    }),
  ],
});
```

```blade
{{-- layouts/shell.blade.php picks the entry per which layout extends it --}}
@vite($vitEntries ?? ['resources/css/public.css', 'resources/js/public.js'])
```

**Why split entries**: an anonymous camp registrant on a phone (Epic-08's
highest-scrutiny conversion path, AC-08-39: "forms load in <2 seconds")
never downloads Event Builder's drag-reorder code, the CRM filter panel,
or Livewire's runtime just to submit a name/email/age form. This is the
same reasoning as §4 applied to bytes instead of database reads: keep
what a request doesn't need out of that request.

### 7.1 What must work with JavaScript disabled

The baseline is a full server round trip for every interaction; JavaScript
only ever shortens or smooths a path that already works without it.

| Interaction | No-JS baseline | JS enhancement |
|---|---|---|
| All navigation | full-page `<a>` links | none — plain links throughout; no client-side routing anywhere in this design |
| Context switcher (AC-01-15/18/32) | `<form method="POST">` + native `<select>` + "Switch" submit button | Alpine auto-submits on `change` |
| Filters — CRM segmentation (AC-03-22..26), calendar (AC-02-21), coupons (AC-06-26) | `<form method="GET">` + submit button, full reload, server-rendered results | Alpine debounce + fetch, or a scoped Livewire component (CRM filter panel specifically — AC-03-25 asks for a live match count) |
| Confirmation-gated actions — cancel RSVP, delete a playlist/form, deactivate/delete a user, toggle a feature | a dedicated confirmation **page** at the same URL the action posts to (`GET` renders it, `POST` performs it) | `<dialog>` intercepts the triggering link and renders the same Blade partial in place (§6.6) |
| Playlist/drill reordering (AC-04-2, AC-04-12) | a numbered "Position" `<select>` per row, or real `<button formaction="...">` up/down controls posting a one-step move | Alpine drag handles, `PATCH` on drop |
| Toggle switches — feature toggles (AC-07-19/20), token-approval (AC-01-27) | a real checkbox inside a form with a submit button; AC-07-20's required confirmation becomes an explicit second page | Alpine auto-submit + inline confirm panel |
| Video/drill auto-complete-on-play (AC-04-25) | an always-visible "Mark as complete" button that `POST`s directly — the YouTube iframe itself plays fine with no site JS either way | the YouTube IFrame API listens for the play event and submits the same endpoint automatically |
| Search-as-you-type (AC-03-8) | text input + explicit "Search" button, `GET`, full reload | debounced fetch, or Livewire `wire:model.live` |
| Coupon live price preview (AC-06-22: "$20 → $16 (20% off)") | "Apply" submit button re-renders the checkout page with the discounted total | a small Livewire component updates the total in place |
| Dashboard/analytics counts (Quick View, referral dashboard, Super Admin dashboard) | server-computed values on load; a "Refresh" link re-requests the page | Livewire polling, only where a metric plausibly changes within the time someone stares at the screen |
| Form validation | `$errors` bag + `old()`-repopulated fields, full-page re-render on failed submit | inline Alpine validation as a preview, never a replacement for the server check |

Livewire is deliberately narrow — three or four components, not the whole
application — because most of this product is straightforward CRUD over
eight epics' worth of forms and lists, where a full request/response cycle
is simpler to reason about, test, and keep working with JavaScript off
than a persistent component tree would be. See §10 for the rejected
full-Livewire and separate-SPA alternatives.

---

## 8. Page inventory by epic

Route paths below are **this document's proposal**, built from the tool
names the epics themselves use ("Event Builder," "CRM Tools," "My
Activities," "Quick View," …) — they are not a client-specified contract.
The exception is the three public, code-carrying entry points
(`/join/{code}`, `/invite/{code}`, `/forms/{code}`), whose literal shapes
come straight out of AC-03-59, AC-03-50, and AC-08-13/AC-06-1 and are
reused verbatim. See §11.

| Area | Prefix | Guard |
|---|---|---|
| Public, no tenant yet | `/`, `/login`, `/register`, `/password/*`, `/email/verify/*` | guest |
| Public, tenant-branded, no auth required | `/join/{code}`, `/invite/{code}`, `/forms/{code}*` | guest (the `associate` branch of `/join/{code}` requires `role:player`) |
| Player / Parent (+ constrained Child — see "The four roles" above) | `/calendar`, `/reservations`, `/lppp/*`, `/tokens`, `/transactions`, `/family`, `/availability`, `/profile`, `/referrals`, `/approvals`, `/subscription/*` | `role:player` |
| Coach | `/coach/*` | `role:coach` |
| Trainer | `/trainer/*` | `role:trainer` |
| Super Admin | `/admin/*` | `role:super_admin` |

AC ranges use the source specs' own `..` notation (e.g. `AC-01-1..8`).

### 8.1 Epic-01 — User Management & Authentication

| Page | Role | Route | AC served |
|---|---|---|---|
| Users tool (list) | Super Admin | `GET /admin/users` | AC-01-6..8, 65, 68, 71, 72, 74, 75, 77 |
| Create Trainer | Super Admin | `GET/POST /admin/users/trainers/create` | AC-01-1..8 |
| ShareLink landing / register | Public | `GET/POST /join/{code}` | AC-01-9..12 |
| ShareLink associate (authenticated branch) | Player/Parent | `GET/POST /join/{code}/associate` | AC-01-13..15 |
| Context switcher (nav component) | Player/Parent, Coach, Child | component in `portal.blade.php` | AC-01-15, 18, 32 |
| Player Profiles / Family | Parent | `GET /family` | AC-01-16..24 |
| Pending purchase approvals | Parent | `GET /approvals` | AC-01-25..28 |
| Constrained calendar/reservations/tokens views | Child (constrained) | same routes as Player, policy-gated | AC-01-29, 30 |
| ShareLink-blocked notice (child clicks a link) | Child (constrained) | `GET /join/{code}` (child branch) | AC-01-31 |
| Impersonate confirm + banner | Super Admin / all (banner) | `POST /admin/users/{user}/impersonate` | AC-01-33..38 |
| Coaches section (invite) | Trainer | `GET /trainer/coaches`, `POST /trainer/coaches/invite` | AC-01-39..42 |
| Coach invite landing | Public | `GET/POST /invite/{code}` | AC-01-39, 40 |
| Availability / My Times | Player/Parent | `GET/POST /availability` | AC-01-43..45 |
| Coach My Times | Coach | `GET/POST /coach/availability` | AC-01-46, 47 |
| Profile / Account Settings | All roles | `GET/POST /profile` | AC-01-48..51 |
| Deactivate / reactivate user | Super Admin | action on Users tool | AC-01-52..54 |
| Delete user (GDPR) | Super Admin | action on Users tool | AC-01-55..59 |
| Portal Branding settings | Trainer | `GET/POST /trainer/settings/branding` | AC-01-60..63 |
| Camp-to-account conversion prompt (cross-epic) | Public → Player | part of `/forms/{code}/convert-account` (Epic-08) | AC-01-64 |
| Login | All | `GET/POST /login` | AC-01-65, 69 |
| Password reset | All | `GET/POST /password/reset` | AC-01-66 |
| Email verification | All | `GET /email/verify/{id}/{hash}` | AC-01-67 |

### 8.2 Epic-02 — Event Management & Scheduling

| Page | Role | Route | AC served |
|---|---|---|---|
| Event Builder (list) | Trainer | `GET /trainer/events` | AC-02-2 |
| Create Event | Trainer | `GET/POST /trainer/events/create` | AC-02-1..3, 58..59, 61..62, 65 |
| Private-event invitee picker | Trainer | within create/edit | AC-02-4..7 |
| Assign coach (with availability check) | Trainer | within create/edit | AC-02-8..11 |
| Date/time picker with player-availability count | Trainer | within create/edit | AC-02-12, 13 |
| Duplicate event | Trainer | `GET/POST /trainer/events/{event}/duplicate` | AC-02-14..17 |
| Training Calendar | Player/Parent | `GET /calendar` | AC-02-18..22, 64, 66 |
| Event details | Player/Parent | `GET /calendar/events/{event}` | AC-02-20, 23, 60 |
| RSVP (free) | Player/Parent | `POST /calendar/events/{event}/rsvp` | AC-02-23, 24, 27, 28 |
| RSVP / pay (paid — hands off to Epic-05) | Player/Parent | `GET /calendar/events/{event}/checkout` | AC-02-25, 26 |
| My Reservations | Player/Parent | `GET /reservations` | AC-02-29 |
| Cancel RSVP confirm | Player/Parent | `GET/POST /reservations/{reservation}/cancel` | AC-02-29..33 |
| My Activities — Events to Confirm | Coach | `GET /coach/activities` | AC-02-34, 65 |
| Confirm / decline assignment | Coach | `POST /coach/activities/{assignment}/confirm\|decline` | AC-02-35..37 |
| Take Attendance | Coach | `GET/POST /coach/activities/{event}/attendance` | AC-02-38..42 |
| RSVP List tab | Trainer | `GET /trainer/events/{event}/rsvps` | AC-02-43..45 |
| Cancel Event confirm | Trainer | `GET/POST /trainer/events/{event}/cancel` | AC-02-46..49 |
| Edit Event | Trainer | `GET/POST /trainer/events/{event}/edit` | AC-02-50..54 |
| Event Master Tool | Super Admin | `GET /admin/events` | AC-02-55..57 |

### 8.3 Epic-03 — CRM & Player Management

| Page | Role | Route | AC served |
|---|---|---|---|
| Players list (+ search) | Trainer | `GET /trainer/players` | AC-03-1..10, 67 |
| Manage Labels | Trainer | `GET/POST /trainer/labels` | AC-03-11, 13, 14 |
| Apply label (player detail action) | Trainer | `POST /trainer/players/{player}/labels` | AC-03-12 |
| Apply / remove flag | Trainer (apply/view/remove); Coach (apply/view, assigned players only) | `POST/DELETE /trainer/players/{player}/flags` | AC-03-15..18 |
| Add note (general + per-event) | Trainer | `POST /trainer/players/{player}/notes` | AC-03-19..21 |
| Segment / filter panel | Trainer | within Players list, query params | AC-03-22..26 |
| Player Detail | Trainer | `GET /trainer/players/{player}` | AC-03-27..33 |
| Quick View Dashboard | Trainer | `GET /trainer/dashboard` | AC-03-34..42, 66 |
| Coach Players list | Coach | `GET /coach/players` | AC-03-43, 64, 65 |
| Coach Player Detail | Coach | `GET /coach/players/{player}` | AC-03-44, 45 |
| Add Session Feedback | Coach | `POST /coach/players/{player}/feedback` | AC-03-46..49 |
| Coach Invite Player (ShareLink) | Coach | `GET/POST /coach/players/invite` | AC-03-50..53 |
| Super Admin CRM Master | Super Admin | `GET /admin/crm` | AC-03-54..58 |
| Trainer Invite Players (mass ShareLink) | Trainer | `GET /trainer/players/invite` | AC-03-59..62 |
| ShareLink tracking report | Trainer | `GET /trainer/players/invite/report` | AC-03-63 |

### 8.4 Epic-04 — LPPP Content System

| Page | Role | Route | AC served |
|---|---|---|---|
| Create Learn playlist | Trainer | `GET/POST /trainer/lppp/learn/create` | AC-04-1..3 |
| Drill Database — My Drills (create drill) | Trainer | `GET/POST /trainer/lppp/drills/create` | AC-04-4..6 |
| Drill Database — Public Drills discovery | Trainer | `GET /trainer/lppp/drills?scope=public` | AC-04-7..10 |
| Create Practice playlist (workout) | Trainer | `GET/POST /trainer/lppp/practice/create` | AC-04-11, 12 |
| Assign Playlist | Trainer | `GET/POST /trainer/lppp/playlists/{playlist}/assign` | AC-04-13..16 |
| Toggle content Public/Private | Trainer | `POST /trainer/lppp/{content}/visibility` | AC-04-17..19, 41 |
| LPPP portal — Learn tab | Player/Parent | `GET /lppp/learn` | AC-04-20..22 |
| LPPP portal — Practice tab | Player/Parent | `GET /lppp/practice` | AC-04-20, 23 |
| Video / drill player page | Player/Parent | `GET /lppp/play/{item}` | AC-04-24..26 |
| LPPP Progress tab | Player/Parent | `GET /lppp/progress` | AC-04-27..29 |
| Player progress (within CRM detail, cross-ref §8.3) | Trainer | `GET /trainer/players/{player}` | AC-04-30 |
| Edit Playlist | Trainer | `GET/POST /trainer/lppp/playlists/{playlist}/edit` | AC-04-31..33 |
| Delete Playlist / Drill confirm | Trainer | `GET/POST /trainer/lppp/{content}/delete` | AC-04-34..36 |
| Super Admin LPPP Analytics | Super Admin | `GET /admin/lppp-analytics` | AC-04-37..40, 42 |

### 8.5 Epic-05 — Payments & Tokens

| Page | Role | Route | AC served |
|---|---|---|---|
| Connect Stripe | Trainer | `GET /trainer/settings/payments` (redirects to Stripe Connect Express) | AC-05-1..3 |
| Tokens / Wallet | Player/Parent | `GET /tokens` | AC-05-4..6 |
| Buy Tokens checkout | Player/Parent | `GET/POST /tokens/purchase` (redirects to Stripe Checkout) | AC-05-4 |
| RSVP with tokens confirm (cross-ref §8.2) | Player/Parent | `POST /calendar/events/{event}/rsvp` (tokens branch) | AC-05-7..9 |
| RSVP with card (cross-ref §8.2) | Player/Parent | `GET /calendar/events/{event}/checkout` | AC-05-10..13 |
| Cancel / refund confirm (cross-ref §8.2) | Player/Parent | `POST /reservations/{reservation}/cancel` | AC-05-14..17 |
| Content purchase — locked playlist paywall (cross-ref §8.4) | Player/Parent | `GET/POST /lppp/learn/{playlist}/purchase` | AC-05-18, 19 |
| Payment Methods | Parent | `GET /settings/payment-methods` (redirects to Stripe Customer Portal) | AC-05-20, 21 |
| Transaction History | Player/Parent | `GET /transactions` | AC-05-22..24 |
| Payout / Earnings summary | Trainer | `GET /trainer/payments` (links to Stripe Express Dashboard) | AC-05-25, 26 |
| Edit Per-Trainer Pricing | Super Admin | `GET/POST /admin/trainers/{trainer}/pricing` | AC-05-27, 28 |
| Subscription purchase (Unlimited Access) | Player/Parent | `GET/POST /subscription/purchase` | AC-05-29 |
| Gift Tokens | Trainer | `GET/POST /trainer/players/{player}/gift-tokens` | AC-05-33 |

Webhook handling (AC-05-34..38) has no page — it's a `POST
/webhooks/stripe` endpoint outside the Blade layer entirely, listed here
only so the epic's full AC set is accounted for somewhere in this
document.

### 8.6 Epic-06 — Marketing & Growth Tools

| Page | Role | Route | AC served |
|---|---|---|---|
| Get the Assist (referral link) | Player/Parent | `GET /referrals` | AC-06-1..3 |
| Referral landing (friend clicks the link) | Public | `GET /join/{trainerSlug}/{playerCode}` | AC-06-4..7 |
| Trainer Referral Dashboard | Trainer | `GET /trainer/marketing/referrals` | AC-06-13..16 |
| Create Coupon | Trainer | `GET/POST /trainer/marketing/coupons/create` | AC-06-17..19 |
| Apply coupon at checkout (cross-ref §8.2/§8.4/§8.5) | Player/Parent | same checkout routes | AC-06-20..25 |
| Coupon Analytics list | Trainer | `GET /trainer/marketing/coupons` | AC-06-26..28 |
| Super Admin Referral Program settings | Super Admin | `GET/POST /admin/referral-settings` | AC-06-29..31 |

Performance/analytics-freshness criteria (AC-06-32, 33) are cross-cutting
and are not tied to one page.

### 8.7 Epic-07 — Super Admin & System Management

Several rows below render the same physical page as an Epic-01/02 row —
marked "(shared)" — but are listed here under their own AC citations
because Epic-07 states requirements Epic-01/02 don't (e.g. the dashboard's
"no financial data, ever" negative criterion).

| Page | Role | Route | AC served |
|---|---|---|---|
| Operational Dashboard | Super Admin | `GET /admin/dashboard` | AC-07-1..7 |
| Users tool (shared with §8.1) | Super Admin | `GET /admin/users` | AC-07-8..12 |
| Edit User | Super Admin | `GET/POST /admin/users/{user}/edit` | AC-07-13..15 |
| Create Trainer (shared with §8.1) | Super Admin | `GET/POST /admin/users/trainers/create` | AC-07-16, 17 |
| Trainer Feature Settings | Super Admin | `GET/POST /admin/trainers/{trainer}/features` | AC-07-18..21 |
| Event Master Tool (shared with §8.2) | Super Admin | `GET /admin/events` | AC-07-22..26 |
| Event conflict override (within Event Master edit) | Super Admin | `POST /admin/events/{event}` | AC-07-27, 28 |
| Audit Log | Super Admin | `GET /admin/audit-log` | AC-07-29..34 |
| Stripe Dashboard links | Super Admin | buttons on Dashboard / Trainer detail | AC-07-35..37 |
| Trainers list / detail | Super Admin | `GET /admin/trainers`, `GET /admin/trainers/{trainer}` | AC-07-38..40 |

Performance/scale criteria (AC-07-41) are cross-cutting, not tied to one
page.

### 8.8 Epic-08 — Forms & Registration (Camps & Evaluations)

| Page | Role | Route | AC served |
|---|---|---|---|
| Camps & Evaluations list | Trainer | `GET /trainer/camps` | AC-08-27, 32 |
| Create Camp form builder | Trainer | `GET/POST /trainer/camps/create` | AC-08-1..8, 33 |
| Create Evaluation form builder | Trainer | `GET/POST /trainer/camps/create?type=evaluation` | AC-08-9..12 |
| Public camp / evaluation form | Public | `GET/POST /forms/{code}` | AC-08-13..15, 34..36, 39..42 |
| Form confirmation | Public | `GET /forms/{code}/confirmation` | AC-08-16 |
| Camp Submissions list | Trainer | `GET /trainer/camps/{form}/submissions` | AC-08-17..22 |
| Convert submission to account | Public → Player | `GET/POST /forms/{code}/convert-account` | AC-08-23..26 |
| Edit form settings | Trainer | `GET/POST /trainer/camps/{form}/edit` | AC-08-28..31 |

Business-metric criteria (AC-08-43..46 — adoption rate, conversion rate,
revenue share, zero payment errors) describe outcomes, not screens, and
are not tied to a page.

**State notes for the public form** (`/forms/{code}`, the page the brief
names directly): full-capacity → "Camp Full," no submission control
(AC-08-15); disabled by the trainer → "Registration Closed" (AC-08-29);
code not found or the form's owning trainer deactivated → a 404 rendered
under `trainer-public.blade.php` still carrying whatever branding
resolved (§4.5), never a bare platform 404. All three render inside the
same layout as a working form, per §4.5's branding-independent-of-
form-state guarantee.

---

## 9. Accessibility — WCAG 2.2 AA, baked into the components

This targets the **AA** baseline throughout, as the brief specifies. One
correction to flag up front: target size is WCAG 2.2 **SC 2.5.8 Target
Size (Minimum), Level AA — 24×24 CSS px**, not the 44×44 figure from SC
2.5.5 (Level **AAA**, carried over from WCAG 2.1). This document designs
to 24×24 as the required floor and exceeds it toward 44×44 where a control
is a primary action or mobile-critical, without treating 44×44 as the AA
requirement it isn't.

### 9.1 Contrast — including against the dynamic brand color

Every neutral and semantic token in §2 was checked against a white surface
using the WCAG relative-luminance formula (sRGB → linearized channels →
`0.2126R + 0.7152G + 0.0722B`, contrast = `(L₁+0.05)/(L₂+0.05)`), not
assumed:

| Token | Hex | Ratio vs white | Passes | Approved use |
|---|---|---|---|---|
| `--gray-50` | `#F3F3F3` | ~1.1:1 | neither | surface tint only — never a border or text |
| `--gray-200` | `#CFCFCF` | ~1.6:1 | neither | decorative dividers only — never a meaningful boundary |
| `--gray-400` | `#868686` | ~3.6:1 | 3:1 (UI) | default border floor — inputs, cards, tables |
| `--gray-600` | `#5E5E5E` | ~6.5:1 | 4.5:1 (text) | `--color-text-secondary` |
| `--gray-800` | `#363636` | ~12.1:1 | 4.5:1 (text) | `--color-text-primary` |
| `--gray-950` | `#0D0D0D` | ~19.6:1 | 4.5:1 (text) | reserved for max-emphasis text; rarely needed over gray-800 |
| `--color-danger` (`#FF5E58`) | — | ~3.0:1 | 3:1 (UI) only | icons, borders, fills — **not** small text |
| `--color-danger-text` | derived | ~5.0:1 | 4.5:1 (text) | error message copy |
| `--color-success` (`#099137`) | — | ~4.1:1 | neither (just under 4.5) | icons, fills — **not** small text |
| `--color-success-text` | derived | >4.5:1 | 4.5:1 (text) | success message copy |

Two concrete, non-obvious findings came out of this: `--gray-200` cannot
be used for an input border (1.6:1 — a common mistake, since it's the
"light gray" a designer reaches for first), and the raw semantic red and
green extracted from `inputs.svg` are **not** safe as small error/success
*text* even though they're exactly what a designer would grab for that —
they clear the 3:1 non-text floor but not the 4.5:1 text floor. This is
why §2 defines `-text` variants for both and the component rule is:
**message copy always uses the `-text` variant, the raw semantic color
never appears as small text.**

**Against the dynamic brand color specifically** (the hard case the brief
calls out): §4.3 is the full algorithm. Two structural rules keep the
*component* layer safe regardless of what a trainer picks: (1)
`--brand-primary` is only ever used as a **fill** or at large/graphical
scale (≥24px icons, the 3:1 threshold) — never as small foreground text on
the neutral page background, which sidesteps needing a third
runtime-computed "safe as foreground" variant; (2) focus rings and input
focus borders use `--brand-primary-deep`, not the raw base — darkening
toward black from any starting hue only increases contrast against a
white page, so this reliably clears the 3:1 non-text floor even for pale
trainer-chosen accents that would fail as a thin ring in their raw form.

### 9.2 Focus visibility (SC 2.4.7, 2.4.11)

`outline` is never removed without a replacement. Every interactive
element gets a `:focus-visible` ring at `--brand-ring` (2px, offset 2px),
so keyboard focus is always visible and never obscured by a sticky header
or modal. `:focus` (mouse-triggered) does not show the ring —
`:focus-visible` only — so the ring doesn't appear on every click for
pointer users while remaining mandatory for keyboard users.

### 9.3 Target size (SC 2.5.8 — 24×24 CSS px, AA)

| Control | Visual size (§2/DESIGN_TOKENS.md) | Hit area |
|---|---|---|
| Checkbox | 14×14 | padded to 24×24 via the wrapping `<label>` (§6.3) |
| Radio | 22×22 | padded to 24×24 via the wrapping `<label>` |
| Toggle | 39×24 | already ≥24×24 — no augmentation needed |
| Button (sm/md) | padding per §2 control geometry | ≥38px computed height either way — clears the floor comfortably |
| Icon-only buttons (close, remove) | — | `min-width/min-height: 24px` enforced explicitly; never a bare `w-4 h-4` glyph |

The glyph stays at its specified visual size; only the *hit area* grows,
via padding on the label/wrapper, which is the standard, well-tested
pattern for satisfying 2.5.8 without visually inflating a small control.

### 9.4 Error association (SC 1.3.1, 3.3.1, 3.3.2)

Every input is a real `<label for>`-associated control — placeholder text
is never the only label (forms-labels-required). Every server-side
validation error renders with a stable `id`, wired to its field via
`aria-describedby`, with `aria-invalid="true"` on the field and
`role="alert"` on the error text (§6.2). A page-level error summary lists
all failures with links to each field for a form with multiple errors
(camp/evaluation form builder, event creation), so a screen-reader user
isn't left to discover them one field at a time.

### 9.5 Keyboard paths (SC 2.1.1, 2.4.1, 2.4.3)

"Skip to main content" is the first focusable element on `portal.blade.php`
and `trainer-public.blade.php` (both carry repeated navigation worth
bypassing). DOM order matches visual/reading order throughout, so tab
order needs no `tabindex` overrides. Every custom-styled control (checkbox,
radio, toggle, the context switcher) is a real native form element under
the hood, so it is reachable and operable by keyboard with zero additional
work. Modals trap focus while open and restore it to the triggering
control on close (§6.6); their true no-JS baseline is a full page, so
keyboard operability never depends on the modal implementation at all.

### 9.6 Non-color cues (SC 1.4.1)

Flags, status badges, availability indicators (green/gray/red, AC-02-13),
and payment status all pair color with text or an icon, never color
alone — critical for the CRM's care-relevant flags (Injured, Medical
restriction, …) where a color-blind trainer or coach must not lose that
information. Links in body copy carry an underline, not color alone.

### 9.7 Motion (SC 2.3.3)

Button hover lift/scale and any Alpine-driven open/close transition read
their duration from `--duration-fast`/`--duration-standard`, both zeroed
under `prefers-reduced-motion: reduce` (§2) — a single media query
disables every such effect platform-wide, not a per-component opt-out
that's easy to forget.

### 9.8 Zoom and reflow (SC 1.4.10, 1.4.4)

Layouts use flex/grid with wrapping, not fixed-width columns, so content
reflows at 320px CSS width / 400% zoom with no horizontal page scroll.
The one deliberate exception is dense tables (§6.7), which scroll
horizontally **inside their own wrapper** — an explicit, WCAG-permitted
exception for tabular data, never the page body itself.

---

## 10. Decisions

| Decision | Rejected option(s) | Why |
|---|---|---|
| Brand ramp derived in CSS via `color-mix()` from one PHP-emitted hex | Precompute a full 7+ color palette in PHP and emit all of it per request | The ramp is a verified linear mix (§1.1) — the browser can reconstruct it exactly from one value; a full precomputed palette bloats the runtime payload and creates two sources of truth that can drift |
| `--brand-primary-rgb`-style alpha via CSS relative-color syntax (`rgb(from var(--brand-primary) r g b / 55%)`) | Have PHP compute and emit a third `--brand-primary-rgb` triplet, as DESIGN_TOKENS.md's prose literally names | Keeps the runtime-overridable set at exactly two properties (§3); accepted trade-off is requiring evergreen-browser CSS support, judged acceptable for a 2026 build |
| Contrast failure (both black and white fail 4.5:1): auto-correct via a bounded walk down the same ramp curve, with a visible in-UI notice | (a) Hard-reject the color at save time; (b) ship it unmodified and accept the AA failure | (a) is disruptive during onboarding for a narrow edge case; (b) fails the brief's own accessibility requirement; the bounded auto-correct preserves the trainer's intent as closely as possible while guaranteeing AA |
| Tenant resolution via a route-carried public code (`/join/{code}`, `/invite/{code}`, `/forms/{code}`), not subdomains | Per-trainer subdomains (`{trainer}.practiceperfect.app`) | The epics' own literal URL formats (AC-03-59, AC-06-1) are path-based; subdomains add wildcard DNS/SSL operational cost the specs never ask for |
| Public branding read from a dedicated, tenant-scope-exempt table (`trainer_branding_settings`, per the DB schema doc) | Read branding through the same tenant-scoped connection/query as everything else | An RLS-scoped read cannot succeed before a tenant exists, which is exactly the state an anonymous camp-form visitor is in; the schema doc already classifies this table as public/global for this reason |
| Modals: native `<dialog>` + a full confirmation **page** as the true no-JS baseline | (a) A div-based custom modal with no native semantics; (b) never offering a modal, always a full page even with JS available | (a) fails focus-trap/keyboard baseline; (b) needlessly worsens the experience for JS users with no accessibility benefit, since the page-based version is preserved either way |
| Dense tables scroll horizontally inside their own wrapper | Collapse tables to a stacked "card per row" layout on narrow viewports | Preserves full native table semantics (WCAG 1.3.1) with zero JS dependency; the collapse pattern is harder to keep accessible and adds client-side complexity for a baseline requirement |
| Tailwind v4 (CSS-first `@theme`), generating utilities from the same custom properties defined in §2 | (a) Hand-rolled component CSS with no utility layer; (b) a third-party prebuilt admin UI kit | (a) is slower to keep consistent across ~120 pages; (b) fights a client-supplied token spec instead of applying it |
| Alpine.js for local interactivity + a small number of justified Livewire islands (§7.1) | (a) A full Livewire-driven application; (b) a separate SPA (Vue/React) consuming an API | (a) adds persistent-connection/state overhead to pages that are simple CRUD forms; (b) is explicitly ruled out by the brief ("not a SPA") and would double the surface (API layer + separate build) against a server-rendered, progressive-enhancement requirement |
| CRM label colors use curated presets (no runtime computation) | Treat label colors the same as `--brand-primary` (free-text hex + runtime contrast correction) | AC-03-11 itself specifies "a color from a color picker **with presets**," not free entry — presets can be pre-vetted for contrast at design time, so duplicating §4.3's machinery for a lower-stakes, already-constrained input would be unjustified complexity |
| Split Vite entry points (`app.*` for the authenticated portal, `public.*` for unauthenticated layouts) | A single bundle for every page | Keeps the highest-scrutiny public conversion path (camp forms, AC-08-39: "<2 seconds") from downloading authenticated-app-only JS/CSS it never uses |
| Branding cache: write-through invalidation (`Cache::forget` synchronously on save, short safety TTL otherwise) | A literal zero-cache design (re-read and recompute on every request) | Satisfies "applied immediately" exactly as strictly, without a DB read + luminance computation on every page view of every trainer's portal |

## 11. Open questions

1. **Grayscale duplicate.** `DESIGN_TOKENS.md`'s base palette lists
   `#363636` at two consecutive positions, with `#0D0D0D` actually the
   sixth and final stop. Treated as six distinct values as given; likely a
   transcription artifact. Confirm the intended fifth stop.
2. **Checkbox visual size disagrees between sources.** `DESIGN_TOKENS.md`
   states 14×14px; the raw `form_control_element.svg` renders the checked/
   unchecked checkbox artwork at ~24×24px (radio and toggle match exactly
   across both sources). This document follows the smaller, documented
   value and relies on hit-area padding regardless (§9.3) — confirm which
   is the real intended visual size.
3. **Unexplained light-blue fill in `inputs.svg`** (`#B6D8FF`), not
   described by any of DESIGN_TOKENS.md's named input states (default/
   focus/error/disabled). Not adopted into the token set. Confirm whether
   it denotes an "info" state, a disabled-field background, or is
   unrelated to inputs.
4. **A second, cool-toned neutral family appears in the real mockups**
   (`#101928`, `#667185`, `#98A2B3`, `#F0F2F5` — seen via narrow `grep` of
   `buttons.svg`/`inputs.svg`/`event_builder.svg`) alongside the warm
   grayscale DESIGN_TOKENS.md documents. This document treats the
   documented six-stop warm grayscale as sole authoritative and does not
   adopt the second family. Confirm whether the product intends one
   neutral scale or two.
5. **No warning/info semantic color is documented anywhere**, and none
   could be extracted with confidence from the SVGs (only error-red and a
   success-teal were found). Several ACs describe warning-toned states
   (capacity conflicts, scheduling-conflict overrides) with no named
   color. `--color-warning` in §2 is a proposed, contrast-checked
   placeholder, not a sourced value — confirm or replace it.
6. **The authenticated app's URL scheme is this document's proposal**,
   built from the tool names the epics use, not a client-specified
   contract (unlike the three public code-based routes, which quote AC
   text directly). Confirm before treating §8's routes as final.
7. **Whether Livewire is an approved dependency at all is unstated** in
   every spec read. It's proposed narrowly (§7.1, a handful of islands);
   the whole product works on Alpine.js plus classic forms alone if
   Livewire is rejected as a dependency. Flag as a build decision, not a
   requirement this document assumes.
8. **No font-family is specified or extractable** from the design source
   (the SVG text exports carry no `font-family` attribute, most likely
   outlined to paths). A system-font stack is used rather than inventing a
   named webfont (§2). Confirm if the client has a specific typeface in
   mind.
9. **Dark mode is not mentioned** in DESIGN_TOKENS.md, any epic, or the
   open-questions register. The heavy real-mockup use of `#101928` (a
   near-black, cool-toned fill — see item 4) hints a dark surface may
   exist somewhere in the source material, but nothing in scope requires
   one, so none is designed here. Flag if dark mode is actually planned.
10. **Uploaded-SVG handling.** AC-01-60 accepts SVG as a logo format. This
    document specifies `<img>`-only rendering (§4.5, never inline
    SVG-as-DOM) as the XSS mitigation, but whether uploads are also
    re-encoded, rasterized, or scanned server-side before being served at
    all is not addressed by any epic read. Flag for a security review
    before implementation.

