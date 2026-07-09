---
name: browser-verify
description: Visually verify native PHP web UI changes in a running app. Use for server-rendered pages, forms, and progressive-enhancement workflows that need browser evidence.
phase: execution
flow-next: verify
flow-alternatives: [coder-frontend, debugger]
related: [coder-frontend, frontend-design, wcag-accessibility]
---

# Browser Verify

## Overview

Verify the user-facing behavior of a running native PHP application. Browser verification supplements tests; it does not replace PHPUnit/Pest coverage.

## Before Starting

Identify how the app runs:

```bash
php -S localhost:8000 -t public
composer dev
make dev
<frontend-dev-command>
```

Use the project's documented command. Do not invent a dev server setup if none exists.

## Verification Checklist

- Page loads without server errors (check the PHP error log) or browser console errors.
- Authenticated and unauthenticated states behave correctly.
- Forms show server-side validation errors and preserve prior input.
- CSRF-protected forms reject missing/invalid tokens.
- Authorization failures are handled gracefully.
- Success states and redirects are correct.
- Loading, empty, and error states are visible where relevant.
- Keyboard navigation and focus states work.
- Responsive layouts work at mobile and desktop widths.
- The page still works with JavaScript disabled (progressive enhancement baseline).

## Evidence

Capture:

- URL verified.
- User role/state used.
- Main interactions performed.
- Screenshots or concise visual notes.
- Any console/network/server-log errors observed.

## Blockers

Stop and report if blocked by:

- Login credentials.
- Manual MFA/passkey/captcha.
- Missing seed data.
- Broken dev server.
- Destructive confirmation.

## Final Output

Return verified flows, evidence, blockers or risks, Context Summary, and next step.
