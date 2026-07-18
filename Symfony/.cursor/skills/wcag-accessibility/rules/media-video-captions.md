---
title: Provide Captions for Video Content
impact: MEDIUM
impactDescription: deaf users cannot access audio content
tags: media, video, captions, transcripts
related: [motion-no-autoplay]
---

## Provide Captions for Video Content

**Impact: MEDIUM (deaf users cannot access audio content)**

Pre-recorded video with audio must have synchronized captions. WCAG 1.2.2, 1.2.4.

**Incorrect: no captions**

```html
<video src="tutorial.mp4" controls />
```

**Correct: captions provided**

```html
<video controls>
  <source src="tutorial.mp4" type="video/mp4" />
  <track kind="captions" src="tutorial-en.vtt" srcLang="en" label="English" default />
</video>
```

Also provide text transcripts. Auto-generated captions are a starting point but should be reviewed for accuracy.

Video content must not auto-play without controls — see [[motion-no-autoplay]]. Captions benefit all users, not just those with hearing impairments.
