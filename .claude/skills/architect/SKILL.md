---
name: architect
description: Design Symfony features using Controller -> Service -> Repository layered architecture, SOLID, Doctrine, validation, authorization, and clean code.
phase: planning
flow-next: api-designer
flow-alternatives: [writing-plans, architecture-implementer, coder]
---

# Symfony Architect

## Goal

Choose a simple, maintainable Symfony design before implementation.

Default to:

```text
Controller -> Service -> Repository
```

Add more structure only when it solves a real problem.

## Required Analysis

Before deciding, inspect:

- Existing routes/controllers/commands/handlers.
- Service and repository patterns already used.
- Doctrine entities, migrations, and indexes.
- Form/request DTO/Validator conventions.
- Security voters, roles, `access_control`, and route constraints.
- Tests and fixtures/factories.
- Relevant specs and task docs.

## Architecture Decision Template

Document:

- Entry point: controller, console command, Messenger handler, event subscriber, or API Platform resource.
- Validation: Form, request DTO, Validator constraints, custom constraints, or explicit validation.
- Authorization: voter, attribute, `access_control`, service policy, or route constraint.
- Service boundary: one use case per cohesive workflow.
- Repository boundary: query/persistence methods with business-readable names.
- Transaction boundary: where writes commit/rollback.
- Response contract: DTO, serializer group, normalizer, API Platform output, Twig model, or redirect.
- Tests: unit, functional, repository integration, Messenger, console.
- Rollout risk: migrations, workers, cache, BC breaks, data backfill.

## Good Decisions

- Put a multi-step registration flow in `CreateRegistrationService`.
- Put `existsForEmail()` and `findActiveForUser()` in repositories, not controllers.
- Use a voter for object-level permissions.
- Use a migration unique index to enforce uniqueness under concurrency.
- Use Messenger for slow/retryable email or integration work.

## Bad Decisions

- Controller queries Doctrine, mutates entities, sends mail, and returns raw entities.
- Event subscriber contains the business workflow.
- Service returns `JsonResponse`.
- Repository authorizes users.
- DTO validation is skipped because the UI already validates.

Use [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) to review dependency direction and responsibility placement. Treat them as contrastive examples, not a requirement to create every shown type.

## Output

Finish with:

- Recommended design.
- Layer placement table.
- Files likely touched.
- Tests needed.
- Risks and assumptions.
- Next command recommendation.
