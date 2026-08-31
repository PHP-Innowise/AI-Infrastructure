---
title: Ensure Keyboard Accessibility for All Interactive Elements
impact: CRITICAL
impactDescription: 100% of functionality must be keyboard operable
tags: keyboard, interactive, operable
related: [semantic-button-link, responsive-touch-target]
---

## Ensure Keyboard Accessibility for All Interactive Elements

**Impact: CRITICAL (100% of functionality must be keyboard operable)**

Every clickable element must be reachable and operable via keyboard. Custom widgets need keyboard event handlers. WCAG 2.1.1 Keyboard.

**Incorrect: mouse-only interactions**

```html
<div onclick={handleOpen}>Open Panel</div>
<img src="close.svg" onclick={handleClose} />
```

**Correct: keyboard-accessible interactions**

```html
<button onclick={handleOpen}>Open Panel</button>
<button onclick={handleClose} aria-label="Close">
  <img src="close.svg" alt="" />
</button>
```

For custom widgets that cannot use native elements, add `role`, `tabindex={0}`, and `onKeyDown` handlers for Enter/Space.

Use correct semantic elements per [[semantic-button-link]] to get keyboard behavior for free. For touch interfaces, ensure targets meet minimum sizes per [[responsive-touch-target]].
