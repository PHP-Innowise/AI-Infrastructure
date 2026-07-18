---
title: Content Must Reflow at 320px Width
impact: LOW-MEDIUM
impactDescription: zoomed users see equivalent of 320px viewport
tags: responsive, reflow, zoom, viewport
related: [responsive-text-resize]
---

## Content Must Reflow at 320px Width

**Impact: LOW-MEDIUM (zoomed users see equivalent of 320px viewport)**

Content must reflow at 320px CSS width (400% zoom on 1280px). No horizontal scrolling except tables/images. WCAG 1.4.10 Reflow.

**Incorrect: fixed-width layout**

```css
.layout { display: flex; min-width: 1024px; }
.sidebar { width: 300px; flex-shrink: 0; }
```

**Correct: responsive layout**

```css
.layout { display: flex; flex-wrap: wrap; }
.sidebar { flex: 1 1 15rem; max-width: 20rem; }
.content { flex: 1 1 20rem; min-width: 0; }
```

Test by setting viewport to 320px or zooming to 400%.

Reflow handles extreme zoom; for standard text sizing, [[responsive-text-resize]] covers the 200% zoom requirement.
