---
title: ARIA & Screen Readers
type: moc
impact: HIGH
description: ARIA attributes bridge the gap between visual interfaces and assistive technologies, but must be used correctly.
---

# ARIA & Screen Readers

ARIA bridges the gap between visual UI and assistive technology — but only when used correctly. Misused ARIA makes accessibility worse.

The first rule: [[aria-prefer-semantic]] — don't use ARIA when native HTML provides the same semantics. A `<button>` is always better than `<div role="button">`. This connects back to [[moc-semantic]] for choosing the right elements.

When ARIA is needed, [[aria-labels-required]] ensures every interactive element has an accessible name (via `aria-label`, `aria-labelledby`, or visible text). This is the ARIA counterpart of [[forms-labels-required]] for form inputs.

For dynamic content, [[aria-live-regions]] announces updates to screen readers — essential for [[forms-error-messages]] and real-time notifications. [[aria-hidden-misuse]] warns against hiding visible interactive content, and [[aria-expanded-states]] communicates disclosure widget states for [[keyboard-focus-trap]] dialogs and dropdowns.
