---
title: Icon Buttons Need Accessible Names
impact: HIGH
impactDescription: icon-only controls are silent to screen readers
tags: aria, buttons, labels
related: [semantic-button-link, aria-prefer-semantic]
---

## Icon Buttons Need Accessible Names

**Impact: HIGH**

Every interactive element must have an accessible name. WCAG 4.1.2.

**Incorrect: icon-only button has no name**

```html
<button type="button">
  <span aria-hidden="true">×</span>
</button>
```

**Correct: icon-only button is labelled**

```html
<button type="button" aria-label="Close dialog">
  <span aria-hidden="true">×</span>
</button>
```

Prefer visible text where possible. Use `aria-label` for icon-only controls.
