---
title: Don't Hide Interactive Content with aria-hidden
impact: HIGH
impactDescription: creates invisible but focusable ghost elements
tags: aria, hidden, interactive
related: [media-decorative-images, aria-labels-required]
---

## Don't Hide Interactive Content with aria-hidden

**Impact: HIGH (creates invisible but focusable ghost elements)**

Never apply `aria-hidden="true"` to elements containing focusable content. This creates elements users can tab to but not perceive. WCAG 4.1.2.

**Incorrect: hiding focusable content**

```html
<div aria-hidden="true">
  <button onclick={handleAction}>Click me</button>
  <a href="/page">Link</a>
</div>
```

**Correct: hide only decorative content**

```html
<button><SpinnerIcon aria-hidden="true" /> Loading...</button>

{/* Use inert for truly hidden interactive sections */}
<div inert={!isOpen}>
  <button onclick={handleAction}>Click me</button>
</div>
```

Use `inert` attribute or `dialog.showModal()` to properly remove both visibility and interactivity.

`aria-hidden` is appropriate for decorative content — see [[media-decorative-images]] for the image-specific pattern. If an element needs to be visible, it needs an accessible name per [[aria-labels-required]] instead.
