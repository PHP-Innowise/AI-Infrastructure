---
title: No Auto-Playing Motion Without Controls
impact: MEDIUM
impactDescription: uncontrolled motion distracts users and can trigger vestibular symptoms
tags: motion, animation, controls
related: [motion-reduced-motion, motion-safe-transitions]
---

## No Auto-Playing Motion Without Controls

**Impact: MEDIUM**

Do not auto-play moving content unless users can pause, stop, or hide it. WCAG 2.2.2.

**Incorrect: motion cannot be paused**

```html
<div class="animate-marquee">Scrolling text...</div>
```

**Correct: motion has controls**

```html
<div class="marquee" data-paused="false">Scrolling text...</div>
<button type="button" aria-controls="announcement-marquee">Pause animation</button>
```

Keep the control state server-side or in small client-side JavaScript, but ensure the rendered HTML exposes a real button and an understandable label.
