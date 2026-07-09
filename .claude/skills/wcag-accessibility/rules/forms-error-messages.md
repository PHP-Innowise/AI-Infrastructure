---
title: Form Errors Must Be Associated With Inputs
impact: HIGH
impactDescription: users need to know which field failed and why
tags: forms, errors, aria
related: [forms-labels-required, aria-live-regions]
---

## Form Errors Must Be Associated With Inputs

**Impact: HIGH**

Validation errors must be visible and programmatically associated with the field.

**Incorrect: error text is not connected to input**

```html
<label for="email">Email</label>
<input id="email" name="email" type="email">
<p class="text-red-600">Email is required.</p>
```

**Correct: error is associated**

```html
<label for="email">Email</label>
<input id="email" name="email" type="email" aria-describedby="email-error" aria-invalid="true">
<p id="email-error" class="text-red-600" role="alert">Email is required.</p>
```

In PHP templates, render server-side validation errors with stable IDs and connect them through `aria-describedby`.
