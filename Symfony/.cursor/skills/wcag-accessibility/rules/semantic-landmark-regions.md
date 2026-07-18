---
title: Use Landmark Regions
impact: CRITICAL
impactDescription: enables screen reader landmark navigation
tags: semantic, landmarks, navigation
related: [semantic-heading-hierarchy, keyboard-skip-link]
---

## Use Landmark Regions

**Impact: CRITICAL (enables screen reader landmark navigation)**

Use HTML5 landmark elements to define page regions. Screen readers provide shortcuts to jump between landmarks. WCAG 1.3.1, 2.4.1.

**Incorrect: div-only structure**

```html
function Layout() {
  return (
    <>
      <div class="header">...</div>
      <div class="sidebar">...</div>
      <div class="content">...</div>
      <div class="footer">...</div>
    </>
  )
}
```

**Correct: semantic landmarks**

```html
function Layout() {
  return (
    <>
      <header>...</header>
      <nav aria-label="Main">...</nav>
      <main>...</main>
      <aside aria-label="Related">...</aside>
      <footer>...</footer>
    </>
  )
}
```

When using multiple `nav` or `aside` elements, add `aria-label` to distinguish them.

Landmarks pair with [[semantic-heading-hierarchy]] for complete navigational structure. Provide a [[keyboard-skip-link]] so keyboard users can jump directly to landmarks.
