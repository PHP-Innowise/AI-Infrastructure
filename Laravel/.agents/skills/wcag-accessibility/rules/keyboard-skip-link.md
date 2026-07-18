---
title: Provide Skip Navigation Link
impact: MEDIUM
impactDescription: saves keyboard users from tabbing through repetitive nav
tags: keyboard, skip-link, navigation
related: [semantic-landmark-regions, keyboard-focus-visible]
---

## Provide Skip Navigation Link

**Impact: MEDIUM (saves keyboard users from tabbing through repetitive navigation)**

Provide a "Skip to main content" link as the first focusable element. WCAG 2.4.1 Bypass Blocks.

**Incorrect: no skip link**

```html
function Layout() {
  return (
    <>
      <nav>{/* 20+ navigation links */}</nav>
      <main id="main">...</main>
    </>
  )
}
```

**Correct: skip link present**

```html
function Layout() {
  return (
    <>
      <a
        href="#main"
        class="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:p-2 focus:bg-white"
      >
        Skip to main content
      </a>
      <nav>{/* 20+ navigation links */}</nav>
      <main id="main" tabindex={-1}>...</main>
    </>
  )
}
```

The link is visually hidden until focused via keyboard.

Skip links jump to [[semantic-landmark-regions]] like `<main>`. Ensure the skip link itself has a [[keyboard-focus-visible]] indicator when focused.
