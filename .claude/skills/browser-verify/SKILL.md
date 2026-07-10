---
name: browser-verify
description: Visually verify Laravel web UI changes in a running app. Use for Blade pages, Livewire components, and Inertia.js SPA pages that need browser evidence.
phase: execution
flow-next: verify
flow-alternatives: [coder-frontend, debugger]
related: [coder-frontend, frontend-design, wcag-accessibility]
---

# Browser Verify

## Overview

Verify the user-facing behavior of a running Laravel application, whether the page is a plain Blade view, a Blade view hosting a Livewire component, or an Inertia.js page (Vue/React/Svelte). Browser verification supplements tests; it does not replace PHPUnit/Pest coverage.

## Before Starting

Identify how the app runs. Common Laravel local dev setups, in likely order of preference:

```bash
php artisan serve
composer dev          # if the project defines a dev script (often runs serve + queue + vite concurrently)
./vendor/bin/sail up  # Laravel Sail (Docker)
valet link && valet open   # Laravel Valet
# Herd runs sites automatically at http://<project>.test
npm run dev            # Vite dev server, required alongside the above for Inertia/Livewire asset HMR
```

Use the project's documented command (check `README.md`, `composer.json` scripts, and whether a `docker-compose.yml` or `Procfile` is present). Do not invent a dev server setup if none exists. If the frontend uses Vite (Inertia, or Blade with hot-reloaded CSS/JS), confirm `npm run dev` (or an equivalent already running process) is active, otherwise assets may 404 or be stale.

## Verification Checklist

- Page loads without server errors (check `storage/logs/laravel.log`) or browser console errors.
- Authenticated and unauthenticated states behave correctly (Laravel auth/session, or route middleware).
- Forms show server-side validation errors and preserve prior input:
  - Blade: `$errors` bag and `old()` values render correctly.
  - Livewire: `$this->validate()` failures re-render the component with per-field errors, without a full page reload.
  - Inertia: `form.errors` populate after a failed `useForm` submission.
- CSRF-protected forms reject missing/invalid tokens (`@csrf` in Blade; Livewire/Inertia rely on Laravel's session/Axios CSRF wiring — confirm it isn't broken by a misconfigured domain/proxy).
- Authorization failures (Policy/Gate denials) are handled gracefully, not just hidden in the UI.
- Success states and redirects are correct (`redirect()->route(...)`, Inertia `router.visit`/`to_route`, or Livewire `redirect()`/`dispatch` events).
- Loading, empty, and error states are visible where relevant — especially Livewire's `wire:loading`/`wire:offline` states and Inertia's in-flight/page-transition states.
- Keyboard navigation and focus states work.
- Responsive layouts work at mobile and desktop widths.
- For Blade + Alpine.js pages, the page still works with JavaScript disabled where the design calls for a no-JS baseline; for Livewire/Inertia pages (inherently JS-dependent), instead confirm a clear loading indicator appears while the JS/assets are loading and a sensible error message appears if a request fails.

## Evidence

Capture:

- URL verified.
- Stack involved (Blade / Livewire component / Inertia page) and user role/state used.
- Main interactions performed.
- Screenshots or concise visual notes.
- Any console/network/`laravel.log` errors observed.

## Blockers

Stop and report if blocked by:

- Login credentials.
- Manual MFA/passkey/captcha.
- Missing seed data (check for a relevant seeder/factory before assuming this).
- Broken dev server (`php artisan serve`, Sail, Valet/Herd, or the Vite dev server not starting).
- Destructive confirmation.

## Final Output

Return verified flows, evidence, blockers or risks, Context Summary, and next step.
