---
name: wcag-accessibility
description: WCAG 2.2 accessibility guidelines for reviewing and writing accessible web interfaces. Use when reviewing frontend code for accessibility, writing accessible components, checking WCAG compliance, or auditing UI for a11y issues. Triggers on "accessibility", "a11y", "WCAG", "screen reader", "keyboard navigation", "aria", "contrast ratio", "alt text", "focus management", "accessible". Contains 30 rules across 8 categories prioritized by impact.
---

# WCAG Accessibility Guidelines

Practical accessibility rules for code review and implementation, based on WCAG 2.2 Level AA. Contains 30 rules across 8 categories, prioritized by impact.

## Scope Boundary

This skill is the authority for **accessibility (a11y)** only. For broader UX/interaction/visual review that is not accessibility-specific, use `web-design-guidelines`.

## When to Apply

Reference these guidelines when:
- Reviewing frontend code (server-rendered PHP templates, HTML, CSS, or progressive-enhancement JavaScript)
- Writing new UI components
- Auditing existing interfaces for accessibility
- Checking WCAG AA compliance
- Fixing accessibility issues reported by automated tools or users

## Navigation

Accessibility starts with the foundation: [[moc-semantic]] ensures correct HTML semantics that screen readers depend on, and [[moc-keyboard]] guarantees all functionality works without a mouse — both CRITICAL impact.

When native HTML isn't sufficient, [[moc-aria]] bridges the gap with ARIA attributes, while [[moc-color]] ensures visual information is perceivable regardless of vision ability.

Interactive elements require proper [[moc-forms]] labeling and error handling. Non-text content needs alternatives per [[moc-media]], and motion/animation must respect user preferences per [[moc-motion]].

Finally, [[moc-responsive]] covers zoom, reflow, and touch target requirements.

For the complete guide with all rules expanded: `AGENTS.md`
