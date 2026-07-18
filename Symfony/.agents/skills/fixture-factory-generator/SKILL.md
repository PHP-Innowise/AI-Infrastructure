---
name: fixture-factory-generator
description: Generate Symfony fixtures, Foundry factories, object mothers, and deterministic test data builders.
phase: execution
flow-next: test-generator
flow-alternatives: [coder, verify]
---

# Symfony Fixture Factory Generator

Create test data helpers that make tests readable and deterministic.

Rules:

- Prefer existing fixture/factory conventions.
- Keep defaults realistic but overrideable.
- Avoid hidden random behavior unless seeded.
- Do not put assertions in factories.
- Model required relations explicitly.
- Keep production fixtures separate from test builders.

## Workflow

1. Inspect existing DoctrineFixturesBundle, Foundry, factory, fixture-group, and database-reset conventions.
2. Choose the smallest suitable tool: Foundry factories for expressive persistence, object mothers/builders for unit tests, fixtures for reusable integration scenarios.
3. Provide deterministic defaults, named states, explicit relation construction, and override points for fields relevant to each test.
4. Use password hashers and real value objects where production invariants require them; never embed real credentials or personal data.
5. Avoid unordered global dependencies. Use fixture references/groups only when shared scenarios justify them.
6. Keep large performance datasets separately opt-in and seeded.

Factories should expose business-meaningful named states and compose smaller builders rather than accumulating boolean mode parameters. See [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) for behavior-focused test boundaries.

## Verification

Add tests proving required relations and important named states. Confirm factories respect database uniqueness and entity invariants, and document whether loading fixtures purges data. Never run purging fixture commands against a non-test database without explicit consent.
