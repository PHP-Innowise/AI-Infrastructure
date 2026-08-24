---
name: frontend-design
description: Design WordPress frontend, editor, and admin experiences for classic themes, block themes, Gutenberg blocks, or approved decoupled clients with accessibility and WordPress design-system awareness.
phase: planning
flow-next: coder-frontend
flow-alternatives: [theme-development, block-development, writing-plans]
related: [wcag-accessibility, web-design-guidelines, architect]
---

# WordPress Frontend Design

## Procedure

1. Identify user, task, surface, theme type, editor experience, content states,
   design tokens, browser/accessibility support and customization constraints.
2. Prefer native blocks, patterns, styles, templates and `theme.json` when they
   meet the need; choose a custom block or application surface only for a
   distinct interaction/data contract.
3. Define information hierarchy, semantic regions/headings, responsive and
   long-content behavior, keyboard/focus sequence, loading/error/empty/success
   feedback, validation and destructive confirmation.
4. Specify editor/frontend parity, content authorship boundaries, style
   isolation, global styles, child-theme/site-editor customization, RTL and
   translation expansion.
5. Define asset scope and performance budget. Avoid site-wide libraries for a
   single component and avoid fragile selectors tied to editor internals.
6. Produce testable acceptance criteria for keyboard, screen reader, contrast,
   zoom/reflow, touch targets, reduced motion and representative content.

## Output

Return surface choice, component/content model, state and interaction contract,
accessibility/responsive/editor behavior, asset constraints, acceptance
criteria, Context Summary, and next step.
