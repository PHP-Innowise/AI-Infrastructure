---
title: Responsive & Zoom
type: moc
impact: LOW-MEDIUM
description: Content must remain usable when zoomed, resized, or viewed on different screen sizes and orientations.
---

# Responsive & Zoom

Users with low vision zoom to 200-400%. Content must remain functional at these zoom levels.

[[responsive-text-resize]] requires text to be resizable up to 200% without loss of content or functionality. Avoid fixed heights on text containers and use relative units.

At extreme zoom (400% / 320px viewport width), [[responsive-reflow]] ensures content reflows to a single column without horizontal scrolling. No information should be lost, and no functionality should break.

For touch interfaces, [[responsive-touch-target]] sets minimum target sizes: 24×24px minimum (WCAG 2.2), 44×44px recommended. Small targets are especially problematic for users with motor impairments, connecting to [[keyboard-interactive-elements]] for ensuring all targets are keyboard-accessible too.
