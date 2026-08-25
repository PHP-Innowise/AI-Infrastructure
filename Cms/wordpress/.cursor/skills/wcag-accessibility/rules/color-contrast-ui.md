---
title: UI Component Contrast
impact: MEDIUM-HIGH
impactDescription: invisible UI boundaries make interfaces unusable
tags: color, contrast, ui-components, borders
related: [keyboard-focus-visible, color-contrast-text]
---

## UI Component Contrast

**Impact: MEDIUM-HIGH (invisible UI boundaries make interfaces unusable)**

UI component boundaries and meaningful graphics need at least 3:1 contrast against adjacent colors. WCAG 1.4.11 Non-text Contrast.

**Incorrect: low-contrast UI**

```css
.input { border: 1px solid #e0e0e0; background: #fff; }
.btn-ghost { border: 1px solid #ddd; color: #ccc; }
```

**Correct: sufficient UI contrast**

```css
.input { border: 1px solid #767676; background: #fff; }
.btn-ghost { border: 1px solid #767676; color: #595959; }
```

Applies to: input borders, button boundaries, checkboxes, toggles, sliders, focus indicators, and meaningful icons.

Focus indicators must meet this 3:1 ratio per [[keyboard-focus-visible]]. For text content, the higher ratios in [[color-contrast-text]] apply.
