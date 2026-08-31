---
title: Forms & Inputs
type: moc
impact: MEDIUM-HIGH
description: Forms are critical interaction points. Proper labeling, error handling, and grouping enable all users to complete tasks.
---

# Forms & Inputs

Forms are where accessibility most directly affects task completion. A user who can't fill out a form can't use the application.

[[forms-labels-required]] is the foundation — every input needs a visible, programmatically associated label. Placeholders are not labels. This connects to [[aria-labels-required]] for the general ARIA labeling principle.

When validation fails, [[forms-error-messages]] must be associated with their inputs and announced to screen readers via [[aria-live-regions]]. Error states should not rely solely on color per [[color-not-only-indicator]].

[[forms-autocomplete]] uses the `autocomplete` attribute for common fields (name, email, address) to help users fill forms faster and more accurately. [[forms-fieldset-legend]] groups related inputs (like radio buttons or address fields) with `<fieldset>` and `<legend>` for screen reader context.
