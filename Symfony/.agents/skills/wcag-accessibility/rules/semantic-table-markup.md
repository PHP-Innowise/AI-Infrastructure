---
title: Use Semantic Table Markup For Tabular Data
impact: MEDIUM
impactDescription: screen readers need row and column relationships
tags: semantic, table, data
related: [semantic-heading-hierarchy]
---

## Use Semantic Table Markup For Tabular Data

**Impact: MEDIUM**

Use `<table>` for tabular data instead of div-based grids. WCAG 1.3.1.

**Incorrect: div grid for data table**

```html
<div class="grid">
  <div class="row header"><div>Name</div><div>Email</div></div>
  <div class="row"><div>Ada</div><div>ada@example.com</div></div>
</div>
```

**Correct: semantic table**

```html
<table>
  <thead>
    <tr><th scope="col">Name</th><th scope="col">Email</th></tr>
  </thead>
  <tbody>
    <tr><td>Ada</td><td>ada@example.com</td></tr>
  </tbody>
</table>
```

PHP loops can render rows inside `<tbody>` while preserving table semantics.
