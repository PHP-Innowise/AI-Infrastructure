---
title: Dynamic Updates Need Live Regions
impact: MEDIUM
impactDescription: screen reader users may miss async updates
tags: aria, live-region, dynamic-content
related: [forms-error-messages]
---

## Dynamic Updates Need Live Regions

**Impact: MEDIUM**

When content changes without a full page load, announce important updates with live regions.

**Incorrect: results update silently**

```html
<div id="search-results">
  <p>12 results found</p>
</div>
```

**Correct: result count is announced**

```html
<p role="status" aria-live="polite">12 results found</p>
<div id="search-results">
  <!-- Render Blade, Livewire, or Inertia result items here. -->
</div>
```

Use `role="alert"` only for urgent errors. Prefer `role="status"` for normal async updates.
