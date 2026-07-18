---
name: writing-plans
description: Create implementation plans for Symfony layered architecture work after requirements, brainstorming, architecture, API design, or database design.
phase: planning
flow-next: git-worktrees
flow-alternatives: [coder, architecture-implementer]
---

# Symfony Writing Plans

Create plans specific enough for implementation without rediscovering the architecture.

## Planning Method

1. Read the approved requirements/design/specs and the concrete controllers, services, repositories, entities, migrations, security config, templates, messages, and tests affected.
2. State current behavior, target behavior, non-goals, compatibility constraints, and assumptions that still need proof.
3. Assign every decision to its owning layer and call out deliberate deviations from Controller -> Service -> Repository.
4. Break work into independently verifiable, dependency-ordered steps. Each step names exact files, behavior, tests, commands, and expected outcome.
5. Sequence risky changes for reversibility: contract compatibility, expand/backfill/contract migrations, feature flags, worker deployment, cache invalidation, and rollback.
6. End with a Definition of Done mapped to the active edition's `DOD.md`.

Each plan must include:

- Goal and non-goals.
- Controller/service/repository placement.
- DTO/Form/Validator changes.
- Authorization/voter changes.
- Doctrine entity/migration/repository changes.
- Messenger/console/event subscriber impacts.
- Test plan by layer.
- Verification commands.
- Rollout risks.

Every implementation step must preserve pragmatic SOLID: cohesive responsibilities, inward dependency direction, interfaces only at real boundaries, and explicit validation/authorization/transaction/side-effect ownership. Use [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) to make structural expectations concrete without copying illustrative names.

Do not hide long-lived architecture decisions only in task docs; update `specs/` when behavior or architecture changes.

## Output Quality

Avoid vague steps such as "implement service" or "add tests." A developer should be able to execute the plan without redesigning the feature. Identify unknowns explicitly instead of silently choosing product behavior.
