---
name: browser-verify
description: Visually verify Symfony web UI changes in a running app. Use for server-rendered pages, forms, and progressive-enhancement workflows that need browser evidence.
phase: execution
flow-next: verify
flow-alternatives: [coder-frontend, systematic-debugger]
related: [coder-frontend, frontend-design, wcag-accessibility]
---

# Browser Verify

## Overview

Verify the user-facing behavior of a running Symfony application. Browser verification supplements tests; it does not replace PHPUnit/Pest coverage.

## Before Starting

Identify how the app runs:

```bash
php -S localhost:8000 -t public
composer dev
make dev
<frontend-dev-command>
```

Use the project's documented command. Do not invent a dev server setup if none exists.

## What To Walk (MANDATORY)

The checklist below verifies a page. Defects live in journeys. Build the walk
list before opening the browser, and walk it in this order:

1. **List every role the change can reach** - not the role it was developed
   as, but every role whose screens the changed code renders or guards.
2. **For each role, name the first thing a brand-new account of that role
   does.** A parent with a zero balance pressing the pay button; a trainer who
   has just uploaded a logo. That first action is the likeliest path in the
   product and the least likely to have been exercised while building it.
3. **Order by blast radius, not by proximity to the diff.** Money, access
   control, and account creation before layout and copy.
4. **Walk each journey in its empty state as well as its populated one.**
   Seeded demo data hides the zero, the one, and the rejected case, and that
   is where the crashes are.

Two failures this catches that a page-level check cannot:

- **The feature that saves and never renders.** A setting written to a table
  no template reads looks identical to a working one from the form's side.
  Verify the value on the screen it is meant to change, not on the screen that
  set it. The same goes for a feature present on one screen and missing from
  the other hundred.
- **The mechanism behind the button.** Authorization enforced in a controller
  is not enforced in the framework's own shortcut for the same action, and a
  link removed from the navigation is not a route removed. Reach the behavior
  the way the mechanism allows, not only the way the UI offers.

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

## Payload Bounds (MANDATORY)

Browser tooling returns the heaviest payloads in this workflow: one full-page
accessibility snapshot or screenshot can cost more context than every other
step of the verification combined, and it is carried for the rest of the
session. Stay inside these bounds regardless of which browser tool is wired:

- Prefer a targeted read - one element, one selector, the page title, the
  form error - over a full-page snapshot. Take a full snapshot only when a
  targeted read cannot answer the question.
- At most three screenshots per journey, each scoped to the element or
  viewport under test rather than the full page, unless layout itself is what
  is being verified.
- Filter console and network reads to errors and the request under test;
  never dump the whole log or the whole request list.
- Never paste a raw snapshot, DOM dump, or screenshot payload into the
  report, the handoff, or a Brain record: cite the URL, the element, and the
  observed behavior instead.
- Close the tab or session once the checklist is done, so no page state is
  carried into the next turn.

When a bound would prevent answering the question, say so in the report
rather than silently exceeding it. When the budget runs out before the walk
list does, name the journeys left unwalked. A short report that says what it
did not reach is worth more than a full one that implies it reached
everything.

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

## Defect Report

Write down every deviation found, before fixing anything, whether or not
fixing it falls in scope. One entry each:

| Field | Content |
| --- | --- |
| Id | `DEF-01`, sequential within this verification |
| Journey | The role, and the steps that reach it |
| Expected | What the spec or the surrounding product implies |
| Actual | What the screen did, including the exact error text or status code |
| Evidence | URL, element, log line |
| Severity | Blocking / broken behavior / cosmetic |

Do not fix defects inside this skill. A verifier that repairs what it finds
reports green and hands back no list - and the list is the deliverable, because
it is what tells the requester what a passing test suite is and is not worth.
Hand the report to `systematic-debugger` or `coder`.

A verification that found nothing says so explicitly, next to the journeys it
walked. "Verified" without a walk list is not a result.

## Final Output

Return the walk list (walked and unwalked), the defect report, evidence, blockers or risks, Context Summary, and next step.
