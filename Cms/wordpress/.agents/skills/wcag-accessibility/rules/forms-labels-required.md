---
title: Every Form Input Must Have a Label
impact: MEDIUM-HIGH
impactDescription: unlabeled inputs are invisible to screen readers
tags: forms, labels, input
related: [aria-labels-required, forms-error-messages]
---

## Every Form Input Must Have a Label

**Impact: MEDIUM-HIGH**

Every form control must have a visible, programmatically associated label. Placeholder is not a label. WCAG 1.3.1, 3.3.2.

**Incorrect: no associated label**

```html
<input type="email" placeholder="Email">
<div class="label">Username</div>
<input type="text">
```

**Correct: properly associated labels**

```html
<label for="email">Email</label>
<input id="email" name="email" type="email" placeholder="user@example.com">

<label>
  Username
  <input name="username" type="text">
</label>
```

Placeholder disappears on input, so it cannot serve as a label. Always use `<label>` with `for` or wrapping.
