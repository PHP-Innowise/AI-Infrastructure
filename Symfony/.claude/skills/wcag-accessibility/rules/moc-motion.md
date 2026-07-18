---
title: Motion & Animation
type: moc
impact: MEDIUM
description: Motion can cause seizures, nausea, or distraction. Provide controls and respect user preferences for reduced motion.
---

# Motion & Animation

Motion can trigger vestibular disorders, seizures, or distraction. Always provide user control over animation.

[[motion-reduced-motion]] is the primary rule — respect the `prefers-reduced-motion` media query. When the user requests reduced motion, disable non-essential animations. This connects to [[motion-safe-transitions]] for a progressive enhancement approach.

[[motion-safe-transitions]] inverts the default: start with no motion and add it only when `prefers-reduced-motion: no-preference`. This is safer than removing motion after the fact.

[[motion-no-autoplay]] prevents auto-playing animations, videos, or carousels. Any motion that starts automatically must have pause/stop controls. This also affects [[media-video-captions]] content that includes motion.
