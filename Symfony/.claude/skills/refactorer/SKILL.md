---
name: refactorer
description: Perform behavior-preserving Symfony refactors under a test safety net, especially cleaning Controller -> Service -> Repository boundaries.
phase: execution
flow-next: verify
flow-alternatives: [code-reviewer, test-generator, performance-optimization]
---

# Symfony Refactorer

## Goal

Improve structure without changing observable behavior.

## Symfony Refactor Targets

- Move business logic from controllers into services.
- Move Doctrine queries from controllers/services into repositories.
- Replace raw arrays with request/result DTOs where boundaries are unclear.
- Extract voters from scattered role checks.
- Extract Messenger/console/event subscriber workflows into services.
- Improve types, guard clauses, names, and method size.
- Apply reviewed Rector rules only under a test safety net.

## Safety Rules

- Capture current behavior with tests before risky refactors.
- Run the same tests before and after.
- Avoid mixing refactor with feature work.
- Preserve public contracts unless a migration path is documented.

## Method

1. State the observable behavior and refactoring goal separately.
2. Establish a characterization test at the lowest useful boundary before moving risky behavior.
3. Make one structural transformation at a time: rename, extract, move, introduce parameter object, replace conditional with policy, or invert a real infrastructure dependency.
4. Run focused tests after each coherent step and the broader configured suite before completion.
5. Review the final dependency graph and delete transitional duplication only after callers use the new structure.

## Smell To Refactoring Map

- Fat controller -> extract one named application use case; keep request/response mapping in the controller.
- Query logic outside repository -> move a business-named query with bound parameters and integration coverage.
- Boolean mode flags -> split use cases or introduce a strategy only when variants genuinely substitute.
- Primitive/array contract -> introduce a typed DTO or value object at the boundary.
- Scattered access checks -> centralize capability rules in a voter while keeping collection scoping in queries/providers.
- Framework-coupled service -> move framework mapping to an adapter and inject a narrow external-boundary contract.
- Large class -> split by cohesive reason to change, not arbitrary method counts.

Use [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) for before/after review. Do not copy a pattern when the consuming project's simpler structure is already cohesive.

## Output

Include the behavior contract, smell/root cause, transformations performed, before/after dependency direction, tests run before and after, and any behavior intentionally left unchanged.
