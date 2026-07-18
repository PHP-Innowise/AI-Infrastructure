---
title: Prefer Semantic HTML Over ARIA
impact: HIGH
impactDescription: native semantics are more robust than ARIA
tags: aria, semantic, roles
related: [semantic-button-link, semantic-heading-hierarchy]
---

## Prefer Semantic HTML Over ARIA

**Impact: HIGH (native semantics are more robust than ARIA)**

Use native HTML elements before reaching for ARIA. First rule of ARIA: don't use ARIA if a native element works. WCAG 4.1.2.

**Incorrect: ARIA replicating native semantics**

```html
<div role="button" tabindex={0} onclick={handleClick}>Submit</div>
<div role="navigation"><div role="list"><div role="listitem">Home</div></div></div>
<div role="heading" aria-level={2}>Section Title</div>
```

**Correct: native HTML elements**

```html
<button onclick={handleClick}>Submit</button>
<nav><ul><li><a href="/">Home</a></li></ul></nav>
<h2>Section Title</h2>
```

ARIA is only needed for custom widgets with no native equivalent (tabs, tree views, comboboxes).

For interactive elements, [[semantic-button-link]] covers when to use `<button>` vs `<a>`. For document structure, [[semantic-heading-hierarchy]] is always preferable to `role="heading"`.
