---
title: Ensure Visible Focus Indicators
impact: CRITICAL
impactDescription: keyboard users cannot navigate without visible focus
tags: keyboard, focus, visibility, css
related: [color-contrast-ui, keyboard-tab-order]
---

## Ensure Visible Focus Indicators

**Impact: CRITICAL (keyboard users cannot navigate without visible focus)**

All interactive elements must have a visible focus indicator. Never remove outline without providing an alternative. WCAG 2.4.7 Focus Visible, 2.4.11 Focus Not Obscured.

**Incorrect: removing focus outline**

```css
*:focus { outline: none; }
button:focus { outline: 0; }
```

**Correct: custom focus style**

```css
:focus-visible {
  outline: 2px solid #4A90D9;
  outline-offset: 2px;
}

:focus:not(:focus-visible) {
  outline: none;
}
```

**Tailwind:**

```html
<button class="focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-2">
  Click me
</button>
```

Use `:focus-visible` instead of `:focus` to show focus rings only for keyboard navigation.

Focus indicators must meet [[color-contrast-ui]] contrast requirements (3:1 ratio). Ensure focus moves logically per [[keyboard-tab-order]].
