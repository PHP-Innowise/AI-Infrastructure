---
title: Expandable Controls Need aria-expanded
impact: MEDIUM
impactDescription: assistive technology users need disclosure state
tags: aria, disclosure, state
related: [aria-labels-required]
---

## Expandable Controls Need aria-expanded

**Impact: MEDIUM**

Disclosure controls must expose expanded/collapsed state. WCAG 4.1.2.

**Incorrect: clickable div with hidden state**

```html
<div class="accordion-title">Details</div>
<div class="accordion-panel">More content</div>
```

**Correct: button exposes state and controls panel**

```html
<button type="button" aria-expanded="false" aria-controls="details-panel">
  Details
</button>
<div id="details-panel" hidden>
  More content
</div>
```

When state changes through Livewire, Alpine, or another client-side layer, keep `aria-expanded` and `hidden` synchronized with the visible state.
