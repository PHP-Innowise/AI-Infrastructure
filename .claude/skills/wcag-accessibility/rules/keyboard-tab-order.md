---
title: Maintain Logical Tab Order
impact: CRITICAL
impactDescription: unpredictable tab order confuses keyboard users
tags: keyboard, tabindex, navigation
related: [keyboard-focus-visible, keyboard-interactive-elements]
---

## Maintain Logical Tab Order

**Impact: CRITICAL (unpredictable tab order confuses keyboard users)**

Tab order should follow visual reading order. Never use `tabindex` greater than 0. WCAG 2.4.3 Focus Order.

**Incorrect: positive tabindex creates unpredictable order**

```html
<input tabindex={3} placeholder="Last" />
<input tabindex={1} placeholder="First" />
<input tabindex={2} placeholder="Middle" />
```

**Correct: rely on DOM order**

```html
<input placeholder="First" />
<input placeholder="Middle" />
<input placeholder="Last" />
```

Only use `tabindex="0"` (add to tab order) or `tabindex="-1"` (programmatic focus only). Reorder DOM instead of using positive tabindex values.

Tab order is only meaningful when [[keyboard-focus-visible]] indicators are present. Ensure all tab stops are [[keyboard-interactive-elements]] that respond to keyboard events.
