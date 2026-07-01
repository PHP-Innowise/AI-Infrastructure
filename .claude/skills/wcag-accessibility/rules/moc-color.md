---
title: Color & Contrast
type: moc
impact: HIGH
description: Sufficient color contrast ensures content is readable for users with low vision, color blindness, or in challenging lighting conditions.
---

# Color & Contrast

Roughly 1 in 12 men and 1 in 200 women have color vision deficiency. Contrast requirements ensure content is perceivable.

[[color-contrast-text]] defines the minimum ratios: 4.5:1 for normal text, 3:1 for large text (18px+ or 14px+ bold). These ratios are also relevant for [[keyboard-focus-visible]] focus indicators.

Beyond text, [[color-contrast-ui]] requires 3:1 contrast for UI components and graphical elements — borders, icons, form controls. This pairs with [[keyboard-focus-visible]] to ensure focus rings are visible.

[[color-not-only-indicator]] is often overlooked: never use color as the sole indicator of meaning. Error states need text or icons in addition to red coloring — this directly affects [[forms-error-messages]] patterns.
