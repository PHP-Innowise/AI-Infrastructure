---
title: Group Related Inputs with Fieldset
impact: MEDIUM
impactDescription: provides context for related form controls
tags: forms, fieldset, legend, grouping
related: [forms-labels-required, semantic-list-structure]
---

## Group Related Inputs with Fieldset

**Impact: MEDIUM (provides context for related form controls)**

Group related controls with `<fieldset>` and `<legend>`. Screen readers announce the legend before each control. WCAG 1.3.1, 3.3.2.

**Incorrect: ungrouped radio buttons**

```html
<p>Shipping method</p>
<label><input type="radio" name="shipping" value="standard" /> Standard</label>
<label><input type="radio" name="shipping" value="express" /> Express</label>
```

**Correct: fieldset with legend**

```html
<fieldset>
  <legend>Shipping method</legend>
  <label><input type="radio" name="shipping" value="standard" /> Standard</label>
  <label><input type="radio" name="shipping" value="express" /> Express</label>
</fieldset>
```

Use for: radio groups, checkbox groups, address sections, any group sharing a common label.

Fieldset/legend provides group-level context that complements individual [[forms-labels-required]] labels. Similar to how [[semantic-list-structure]] groups related items, fieldset groups related inputs.
