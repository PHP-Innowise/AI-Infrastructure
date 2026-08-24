---
title: Media & Images
type: moc
impact: MEDIUM
description: Non-text content must have text alternatives so information is available regardless of sensory abilities.
---

# Media & Images

Non-text content needs text alternatives for users who can't see images or hear audio.

[[media-alt-text]] is the most common requirement — every informative image needs meaningful alt text that conveys the same information. This aligns with [[aria-labels-required]] for providing accessible names. The alt text should describe function, not appearance.

Not all images need description: [[media-decorative-images]] marks purely decorative images with `alt=""` or `role="presentation"` so screen readers skip them entirely. Knowing when to apply each pattern is key.

For video content, [[media-video-captions]] requires captions for deaf/hard-of-hearing users and transcripts for deafblind users. Captions also benefit users in noisy/quiet environments.
