---
name: frontend-design
description: Design user interfaces for Laravel applications using Blade templates, Livewire, or Inertia.js (Vue/React/Svelte). Covers choosing between the three stacks plus semantic HTML, CSS, and progressive enhancement.
phase: planning
flow-next: writing-plans
flow-alternatives: [coder-frontend, api-designer]
related: [api-designer, coder-frontend, wcag-accessibility, web-design-guidelines]
---

# Frontend Design

## Overview

Design frontend experiences for Laravel applications. Laravel ships three first-party frontend stacks, and part of the design job is picking the right one for the feature: **Blade** (server-rendered templates, optionally enhanced with Alpine.js), **Livewire** (full-stack reactive components without a separate JS build), and **Inertia.js** (a Vue/React/Svelte SPA-like experience backed by Laravel controllers, no separate REST API to maintain).

Do not assume a stack until the project reveals it — check `composer.json` for `livewire/livewire` or `inertiajs/inertia-laravel`, and `package.json` for `@inertiajs/vue3`/`@inertiajs/react`/`@inertiajs/svelte` or `alpinejs`.

## Rendering Decision

| Need | Good fit |
| --- | --- |
| Content pages, simple forms, marketing pages | Blade templates + plain HTML |
| Small dynamic touches (toggles, dropdowns, validation hints) | Blade + Alpine.js progressive enhancement |
| Reactive UI (live search, inline edit, real-time validation) without a JS build/SPA | Livewire component |
| Rich, app-like SPA UX, team wants to write Vue/React/Svelte components, but no separate REST API | Inertia.js page |
| Admin CRUD | Blade + Livewire (e.g. Livewire tables/forms) unless the project already has an admin package |

Default to the least JavaScript/complexity that meets the requirement:

1. Can it be a static Blade view? Use Blade.
2. Does it need light interactivity (a few dynamic elements)? Add Alpine.js on top of Blade.
3. Does it need server-driven reactivity (forms, live validation, polling) without a JS framework? Use Livewire.
4. Does the team want full Vue/React/Svelte components and client-side routing, without hand-building an API? Use Inertia.

Pages must degrade sensibly: Blade pages should work with JS disabled where feasible; Livewire and Inertia inherently require JS, so make sure loading/error states are handled explicitly.

## Required Context

Read:

- `composer.json`/`package.json` to detect which stack(s) are already in use (Livewire, Inertia + Vue/React/Svelte, Alpine).
- Existing `resources/views/` (Blade layouts, components, partials) and/or `resources/js/Pages` (Inertia pages).
- Existing CSS/design tokens under `resources/css` and the Vite config (`vite.config.js`).
- API/route design (`routes/web.php`, controllers) when the UI consumes data from the backend.
- Accessibility rules in `wcag-accessibility`.

## Design Output

Include:

- User flow.
- **Chosen stack** (Blade / Livewire / Inertia) with a one-line rationale tied to the decision table above.
- Page/component list: Blade views + components, or Livewire components, or Inertia page components + shared layout.
- Form fields, validation feedback, loading states, empty states, and error states (`wire:loading`/`wire:target` for Livewire, Inertia `useForm` processing/error state for Inertia, standard `$errors` bag for Blade).
- Accessibility notes (semantic structure, labels, focus, keyboard).
- Responsive behavior.
- Data dependencies: Eloquent models/relationships, controller/Livewire component props, or `Inertia::render()` props.
- Recommended rendering approach for this feature, and how it fits with the rest of the app if a stack is already established.

## Laravel Frontend Notes

- **CSRF:** every state-changing Blade form needs `@csrf`; Livewire and Inertia handle CSRF automatically through Laravel's session/Axios wiring, but confirm the front-end HTTP client sends the `X-CSRF-TOKEN`/`XSRF-TOKEN` cookie correctly.
- Authorization stays server-side (Policies/Gates); hiding a control or component is not authorization — re-check authorization in the controller or Livewire component method, not just in the view.
- Blade's `{{ }}` auto-escapes output; never use `{!! !!}` on untrusted data.
- Re-render forms with prior input and per-field errors on validation failure: Blade uses `old()` and the `$errors` bag, Livewire re-renders automatically from component state after `$this->validate()`, Inertia's `useForm` exposes `form.errors` after a failed submission.
- Progressive enhancement applies most directly to Blade + Alpine.js; for Livewire/Inertia, design explicit loading and error states instead, since the baseline experience already depends on JS.

## Frontend Best Practices

These apply regardless of which of the three stacks is chosen:

- **Semantic HTML first:** use the right element (`button`, `a`, `nav`, `main`, `form`, `table`, `dialog`) before reaching for `div` + ARIA. Correct semantics give you accessibility and keyboard support for free — true whether it's rendered by Blade, a Livewire component, or a Vue/React/Svelte Inertia page.
- **Progressive enhancement where applicable:** for Blade pages, the core flow should work with HTML alone, with Alpine.js layered on for convenience. Livewire and Inertia are JS-dependent by design, so compensate with clear loading/error/offline states.
- **Responsive and fluid:** mobile-first CSS, relative units (`rem`, `%`, `clamp()`), flexbox/grid, and `max-width` containers. Test at small and large widths.
- **Performance:** minimize render-blocking assets, defer non-critical JS, lazy-load below-the-fold images (`loading="lazy"`), set width/height to avoid layout shift, and let Vite code-split Inertia pages per route.
- **Forms UX:** label every field, show inline per-field errors, preserve entered values on failure, use correct `type`/`inputmode`/`autocomplete`, and disable the submit button only after starting submission (never as the sole guard). Use `wire:loading.attr="disabled"` (Livewire) or `form.processing` (Inertia `useForm`) to drive this. Move focus to the first invalid field (or the error summary) on validation failure, guard against double-submit on slow connections, and validate on blur/submit rather than on every keystroke unless the feedback is genuinely instant (e.g. password strength).
- **CSS architecture:** keep it consistent (utility classes such as Tailwind, BEM, or design tokens); avoid deep specificity wars and `!important`. Define spacing/color/type scales as tokens, shared via the Vite-built stylesheet.
- **State feedback:** always design loading, empty, error, and success states, not just the happy path — especially important for Livewire (network round-trips) and Inertia (page visits/form submissions). Distinguish "empty because there's no data yet" (show onboarding/CTA) from "empty because filters/search matched nothing" (show a clear reset action); on error states, give the user a retry action and a specific message instead of a dead end or a generic "something went wrong".
- **Accessibility as a requirement:** color contrast, visible focus, keyboard operability, and reduced-motion support are acceptance criteria, not extras. See `wcag-accessibility`.

## Final Output

Return the design, chosen stack with rationale, implementation notes, Context Summary, and next step.
