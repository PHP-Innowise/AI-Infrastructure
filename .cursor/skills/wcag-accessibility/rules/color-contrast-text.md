---
title: Ensure Text Color Contrast
impact: HIGH
impactDescription: low contrast text is unreadable for low-vision users
tags: color, contrast, text, wcag-aa
related: [color-contrast-ui, color-not-only-indicator]
---

## Ensure Text Color Contrast

**Impact: HIGH (low contrast text is unreadable for low-vision users)**

Normal text: 4.5:1 contrast ratio. Large text (18px+ bold or 24px+): 3:1. WCAG 1.4.3.

**Incorrect: insufficient contrast**

```css
.subtitle { color: #ccc; background: #fff; } /* ~1.5:1 */
.muted { color: #999; background: #eee; }    /* ~2.3:1 */
input::placeholder { color: #ddd; }
```

**Correct: sufficient contrast**

```css
.subtitle { color: #595959; background: #fff; } /* ~7:1 */
.muted { color: #767676; background: #fff; }    /* ~4.5:1 */
input::placeholder { color: #767676; }
```

In Tailwind, `text-gray-500` on white passes AA, `text-gray-400` does not. Use browser DevTools Accessibility panel to verify.

For non-text UI elements, [[color-contrast-ui]] requires 3:1 contrast ratio. Remember that text color alone shouldn't convey meaning — see [[color-not-only-indicator]].
