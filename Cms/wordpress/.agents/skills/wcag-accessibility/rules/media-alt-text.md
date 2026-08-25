---
title: Provide Meaningful Alt Text
impact: MEDIUM
impactDescription: images without alt are invisible to screen readers
tags: media, images, alt-text
related: [aria-labels-required, media-decorative-images]
---

## Provide Meaningful Alt Text

**Impact: MEDIUM (images without alt are invisible to screen readers)**

Informative images must have alt text conveying the same information. WCAG 1.1.1.

**Incorrect: missing or unhelpful alt**

```html
<img src="chart.png" />
<img src="chart.png" alt="chart" />
<img src="team.jpg" alt="photo123.jpg" />
```

**Correct: meaningful alt text**

```html
<img src="chart.png" alt="Revenue grew 40% from Q1 to Q4 2025" />
<img src="team.jpg" alt="Engineering team at the annual offsite" />
```

Alt text should convey purpose and content, not describe every visual detail. For complex images, provide longer description via `aria-describedby`.

Alt text follows the same principle as [[aria-labels-required]] — content needs accessible names. For images that don't convey information, use [[media-decorative-images]] patterns instead.
