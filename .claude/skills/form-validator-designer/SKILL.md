---
name: form-validator-designer
description: Design Symfony Forms, request DTOs, Validator constraints, custom constraints, validation groups, and web/API error behavior.
phase: planning
flow-next: coder
flow-alternatives: [api-designer, test-generator]
---

# Symfony Form And Validator Designer

Choose the validation approach:

- Symfony Forms for server-rendered forms, CSRF, rich field handling, and form themes.
- Request DTOs plus Validator for APIs and mapped payloads.
- Custom constraints for reusable domain validation.
- Validation groups only when they clarify lifecycle-specific rules.

Define invalid payload behavior, field errors, translation keys, tests, and accessibility requirements.

## Design Workflow

1. Inspect whether the boundary is server-rendered HTML, JSON/API Platform, console, Messenger, or an internal service call.
2. Define a typed input model. Do not bind privileged entity fields or workflow state directly from untrusted input.
3. Place syntactic/boundary rules in Validator constraints and keep stateful business decisions in services backed by repositories and database constraints.
4. Use data transformers or DTO mapping for value objects and normalized types; distinguish empty, missing, and null values deliberately.
5. Use validation groups only for genuinely different lifecycle contracts. Prefer separate DTOs when groups would create a conditional field maze.
6. For Forms, define method, CSRF intent/id, unmapped controls, `empty_data`, collection behavior, upload constraints, translation keys, and accessible help/error relationships.
7. For APIs, define malformed JSON, type, validation, domain-conflict, and authorization error shapes without leaking internal exceptions.

Compare typed allowlisted input with the mass-assignment counterexample in [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). Validation proves input shape; it does not replace authorization, database constraints, or stateful business decisions.

## Tests And Output

Cover valid input, each high-risk invalid boundary, nested/collection paths, custom-constraint dependencies, CSRF for web forms, and stable API violation paths. Output the form/DTO fields, constraints, mapping, error contract, authorization boundary, service call, and required tests.
