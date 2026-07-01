---
title: Respect prefers-reduced-motion
impact: MEDIUM
impactDescription: motion triggers vestibular disorders
tags: motion, reduced-motion, animation, css
related: [motion-safe-transitions, motion-no-autoplay]
---

## Respect prefers-reduced-motion

**Impact: MEDIUM**

Honor `prefers-reduced-motion` to disable or reduce animations. WCAG 2.3.3.

**Incorrect: animations always play**

```css
.card { transition: transform 0.3s ease; }
.panel { animation: slide-in 0.5s ease-out; }
```

**Correct: reduced motion respected**

```css
.card { transition: transform 0.3s ease; }
.panel { animation: slide-in 0.5s ease-out; }

@media (prefers-reduced-motion: reduce) {
  .card { transition: none; }
  .panel { animation: none; }
}
```

In Blade, Livewire, or Inertia views, prefer CSS media queries for this rule so the behavior works regardless of JavaScript framework.
