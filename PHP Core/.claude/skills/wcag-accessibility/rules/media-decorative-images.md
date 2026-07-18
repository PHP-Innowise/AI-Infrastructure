---
title: Mark Decorative Images Correctly
impact: MEDIUM
impactDescription: decorative images announced by AT create noise
tags: media, images, decorative
related: [media-alt-text, aria-hidden-misuse]
---

## Mark Decorative Images Correctly

**Impact: MEDIUM (decorative images announced by AT create noise)**

Purely decorative images must be hidden from assistive technologies. WCAG 1.1.1.

**Incorrect: decorative images announced**

```html
<img src="divider.svg" alt="decorative line" />
<button aria-label="Close"><img src="close.svg" alt="close icon" /></button>
```

**Correct: decorative images hidden**

```html
<img src="divider.svg" alt="" />
<button aria-label="Close"><img src="close.svg" alt="" /></button>
```

Use `alt=""` (empty string, not missing) to mark images as decorative. Icons inside labeled buttons should have `alt=""` to avoid redundancy.

The opposite of [[media-alt-text]] — these images should be invisible to screen readers. Uses `role="presentation"` or `alt=""` rather than `aria-hidden` on the image itself — see [[aria-hidden-misuse]] for proper usage.
