---
name: code-reviewer
description: Review Symfony changes for correctness, Controller -> Service -> Repository boundaries, security, maintainability, tests, and operational risk.
phase: quality
flow-next: test-generator
flow-alternatives: [security-reviewer, verify, coder]
---

# Symfony Code Reviewer

## Review Stance

Lead with findings. Prioritize bugs, regressions, security risk, broken boundaries, and missing tests. Keep summary secondary.

## Checklist

- Controllers are thin and do not own Doctrine queries or business workflows.
- Services own use cases, transaction boundaries, and side-effect orchestration.
- Repositories own Doctrine query shape and persistence helpers.
- Validation happens before service behavior uses external input.
- Authorization happens server-side with Symfony Security conventions.
- Public API responses have stable DTO/serializer contracts.
- Doctrine writes are flushed/transactional at clear boundaries.
- Database constraints enforce critical invariants.
- Messenger handlers and event subscribers delegate to services.
- Tests cover happy path and highest-risk failure path.
- Migrations, workers, cache, and rollout impacts are documented.
- Classes are cohesive, dependencies point inward, and interfaces represent real substitution/ownership boundaries rather than ceremony.
- Implementations preserve contract behavior; broad interfaces, boolean mode flags, service locators, hidden write side effects, and array-shaped public contracts are challenged.

Compare relevant changes with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). Report observable risk, not stylistic preference, and do not require every illustrative layer for a simple cohesive feature.

## Output

Use:

```text
Findings
- [severity] file:line - issue, impact, fix

Open Questions
- ...

Summary
- ...
```

If no issues are found, say that clearly and mention residual test or tooling gaps.
