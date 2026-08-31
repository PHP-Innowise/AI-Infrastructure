---
title: Touch Targets Must Be Large Enough
impact: MEDIUM
impactDescription: small controls are hard to activate on touch devices
tags: responsive, touch, mobile
related: [keyboard-focus-visible]
---

## Touch Targets Must Be Large Enough

**Impact: MEDIUM**

Interactive controls should be at least 44 by 44 CSS pixels or have equivalent spacing. WCAG 2.5.8.

**Incorrect: tiny icon button**

```html
<button type="button" class="w-5 h-5">×</button>
```

**Correct: adequate target size**

```html
<button type="button" class="min-w-11 min-h-11 inline-flex items-center justify-center" aria-label="Close">
  <span aria-hidden="true">×</span>
</button>
```

Apply the same rule in every rendered template and partial, regardless of how it is generated.
