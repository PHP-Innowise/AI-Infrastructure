---
title: Use Buttons For Actions And Links For Navigation
impact: HIGH
impactDescription: wrong elements break keyboard and screen reader behavior
tags: semantic, button, link, keyboard
related: [keyboard-interactive-elements, aria-labels-required]
---

## Use Buttons For Actions And Links For Navigation

**Impact: HIGH**

Use `<button>` for in-page actions and `<a>` for navigation to URLs. Using `<div>` or `<span>` with click handlers loses keyboard support and screen reader semantics. WCAG 4.1.2.

**Incorrect: non-semantic actions**

```html
<div class="btn">Submit</div>
<span class="menu-trigger">Open menu</span>
<a href="#">Delete</a>
```

**Correct: semantic controls**

```html
<button type="submit">Submit</button>
<button type="button">Open menu</button>
<button type="button" class="text-danger">Delete</button>
<a href="/settings">Settings</a>
```

In Blade, prefer reusable button/link components that preserve native semantics.
