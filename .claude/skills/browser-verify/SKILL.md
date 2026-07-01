---
name: browser-verify
description: Visually verify Laravel web UI changes in a running app. Use for Blade, Livewire, Inertia, or frontend workflows that need browser evidence.
phase: execution
flow-next: verify
flow-alternatives: [coder-frontend, debugger]
related: [coder-frontend, frontend-design, wcag-accessibility]
---

# Browser Verify

## Overview

Verify the user-facing behavior of a running Laravel application. Browser verification supplements tests; it does not replace PHPUnit/Pest coverage.

## Before Starting

Identify how the app runs:

```bash
php artisan serve
<frontend-dev-command>
composer dev
make dev
```

Use the project's documented command. Do not invent a dev server setup if none exists.

## Verification Checklist

- Page loads without server or console errors.
- Authenticated and unauthenticated states behave correctly.
- Forms show validation errors from Laravel.
- Authorization failures are handled gracefully.
- Success states and redirects are correct.
- Loading, empty, and error states are visible where relevant.
- Keyboard navigation and focus states work.
- Responsive layouts work at mobile and desktop widths.
- Livewire/Inertia interactions preserve state correctly.

## Evidence

Capture:

- URL verified.
- User role/state used.
- Main interactions performed.
- Screenshots or concise visual notes.
- Any console/network errors observed.

## Blockers

Stop and report if blocked by:

- Login credentials.
- Manual MFA/passkey/captcha.
- Missing seed data.
- Broken dev server.
- Destructive confirmation.

## Final Output

Return verified flows, evidence, blockers or risks, Context Summary, and next step.
