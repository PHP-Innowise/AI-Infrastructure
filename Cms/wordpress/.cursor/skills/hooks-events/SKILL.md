---
name: hooks-events
description: Design and review WordPress actions and filters, callback contracts, priorities, accepted arguments, registration lifecycle, recursion safety, and side-effect ordering.
phase: planning
flow-next: coder
flow-alternatives: [plugin-development, writing-plans, code-reviewer]
related: [architect, plugin-development, performance-optimization]
---

# Hooks and Events

## Procedure

1. Locate every registration and callback for the behavior, including dynamic
   hook names, removals, priorities, accepted arguments, and third-party hooks.
2. Choose an action for notification/side effects and a filter for transforming
   and returning a value. Do not use a filter as an implicit command bus.
3. Register on the earliest lifecycle that guarantees dependencies while
   avoiding work before WordPress is ready. Keep registration centralized and
   callbacks narrow.
4. Document callback input, return value, mutation, failure behavior, priority,
   and whether re-entry is legal. A filter must always return a value of the
   documented shape.
5. Protect recursive update paths deliberately. Removing/re-adding callbacks
   is acceptable only when the exact callable and priority are stable; a scoped
   re-entry guard is often clearer.
6. Avoid expensive I/O on hot hooks such as broad query, render, option, or
   content filters. Measure frequency and move delay-tolerant side effects out
   of the request where appropriate.
7. For public extension hooks, use a stable vendor/plugin prefix, pass useful
   context, document the contract, and preserve it through deprecation.
8. Test registration, priority/order, input/output, zero/false/null behavior,
   recursion, and absence of unintended execution on unrelated requests.

## Output

Return a hook map, contracts and ordering decisions, recursion/performance
risks, compatibility impact, test cases, Context Summary, and next step.
