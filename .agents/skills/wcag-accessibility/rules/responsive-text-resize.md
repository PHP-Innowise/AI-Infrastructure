---
title: Text Must Be Resizable
impact: LOW-MEDIUM
impactDescription: low-vision users need to enlarge text
tags: responsive, text, resize, zoom
related: [responsive-reflow]
---

## Text Must Be Resizable

**Impact: LOW-MEDIUM (low-vision users need to enlarge text)**

Text must be resizable up to 200% without loss of content. Use relative units. WCAG 1.4.4.

**Incorrect: fixed pixels**

```css
body { font-size: 14px; }
.heading { font-size: 24px; }
.container { max-width: 1200px; overflow: hidden; }
```

**Correct: relative units**

```css
html { font-size: 100%; }
body { font-size: 1rem; }
.heading { font-size: 1.5rem; }
.container { max-width: 75rem; overflow: visible; }
```

Avoid `overflow: hidden` on text containers. Test by zooming to 200%.

At extreme zoom levels (400%), [[responsive-reflow]] takes over to ensure content reflows to a single column.
