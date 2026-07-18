---
title: Use Safe Defaults for Transitions
impact: LOW-MEDIUM
impactDescription: progressive enhancement prevents motion issues
tags: motion, transitions, progressive-enhancement
related: [motion-reduced-motion]
---

## Use Safe Defaults for Transitions

**Impact: LOW-MEDIUM (progressive enhancement prevents motion issues)**

Design with no-motion as default, enhance for users who haven't requested reduced motion. WCAG 2.3.3.

**Incorrect: motion as default**

```css
.panel { animation: slide-in 0.5s ease-out; }
@media (prefers-reduced-motion: reduce) { .panel { animation: none; } }
```

**Correct: no-motion default**

```css
.panel { /* No animation by default */ }
@media (prefers-reduced-motion: no-preference) {
  .panel { animation: slide-in 0.5s ease-out; }
}
```

Prefer `opacity` and `transform` as safer alternatives to layout-shifting animations. Fade transitions are safer than slide/bounce/zoom.

This is the progressive enhancement approach to [[motion-reduced-motion]] — start with no motion, add it only when the user hasn't opted out.
