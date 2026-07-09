---
title: Semantic HTML
type: moc
impact: CRITICAL
description: Screen readers and assistive technologies rely on correct HTML semantics to convey structure and meaning.
---

# Semantic HTML

Semantic HTML is the foundation of all accessibility. Before reaching for ARIA (see [[moc-aria]]), use native elements — [[aria-prefer-semantic]] explains why native semantics are always more robust.

Start with document structure: [[semantic-heading-hierarchy]] ensures screen readers can navigate by headings (h1→h2→h3), while [[semantic-landmark-regions]] provides skip-navigation landmarks (main, nav, aside) that pair with [[keyboard-skip-link]].

For interactive elements, [[semantic-button-link]] clarifies when to use `<button>` vs `<a>` — a common source of accessibility bugs that also affects [[keyboard-interactive-elements]]. Content grouping uses [[semantic-list-structure]] for lists and [[semantic-table-markup]] for data tables, both requiring proper labeling per [[aria-labels-required]].
