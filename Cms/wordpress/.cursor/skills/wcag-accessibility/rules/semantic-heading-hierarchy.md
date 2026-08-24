---
title: Use Correct Heading Hierarchy
impact: CRITICAL
impactDescription: screen readers use headings for navigation
tags: semantic, headings, structure
related: [aria-prefer-semantic, semantic-landmark-regions]
---

## Use Correct Heading Hierarchy

**Impact: CRITICAL (screen readers use headings as navigation shortcuts)**

Headings must follow a logical hierarchy (h1 -> h2 -> h3). Skipping levels or using headings for styling breaks screen reader navigation. WCAG 1.3.1 Info and Relationships.

**Incorrect: skipping heading levels for styling**

```html
function Page() {
  return (
    <div>
      <h1>Dashboard</h1>
      <h4>Recent Activity</h4>  {/* Skips h2, h3 */}
      <h2>Settings</h2>
      <h6>Advanced</h6>  {/* Skips h3, h4, h5 */}
    </div>
  )
}
```

**Correct: sequential heading levels**

```html
function Page() {
  return (
    <div>
      <h1>Dashboard</h1>
      <h2>Recent Activity</h2>
      <h2>Settings</h2>
      <h3>Advanced</h3>
    </div>
  )
}
```

Use CSS classes for visual sizing instead of incorrect heading levels.

Headings work with [[semantic-landmark-regions]] to provide complete page structure. Always use native headings rather than ARIA roles — see [[aria-prefer-semantic]].
