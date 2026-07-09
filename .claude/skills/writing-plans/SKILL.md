---
name: writing-plans
description: Create implementation plans for native PHP work. Use after requirements, brainstorming, architecture, or API design when execution needs clear steps.
phase: planning
flow-next: git-worktrees
flow-alternatives: [coder, test-generator]
related: [requirements-analyst, brainstorming, architect, api-designer]
---

# Writing Plans

## Overview

Create precise implementation plans for native PHP projects. Plans should be specific enough for an implementer to execute without rediscovering the architecture.

## Required Inputs

Read available context:

- Requirement docs in `tasks/TASK-N/`.
- Living specs in `specs/`.
- Relevant PHP files: entry points/routing, handlers, request validators, domain/services, data access, migrations, tests.
- Existing project conventions for tests, formatting, static analysis, and any frontend tooling.

## Plan Structure

```markdown
# [Feature] Implementation Plan

## Goal
[1-2 sentence outcome]

## Existing Context
- Relevant files and current behavior.

## Proposed Design
- Routes/entry points and handlers.
- Request DTOs / validators.
- Domain entities / value objects.
- Use-case / service classes.
- Repositories/gateways and migrations.
- Middleware, workers/queue jobs, cache if needed.

## Implementation Steps
1. [Specific file-level step]
2. [Specific file-level step]

## Test Plan
- Unit tests.
- Integration/HTTP handler tests.
- Authorization/validation cases.

## Verification
- `composer test`
- `composer lint`
- `composer analyse`

## Risks
- [Migration, security, worker, cache, API compatibility risks]
```

## Planning Checklist

- Does a migration need a backfill or safe rollout?
- How is the route parameter resolved to an entity?
- Which request DTO/validator guards input?
- Which access-control check authorizes behavior?
- Is a response serializer needed for a stable contract?
- Are fixtures/factories needed for tests?
- Should work be queued or run synchronously?
- Are events/mail/external clients involved?
- Are indexes needed for new query patterns?
- Are server-rendered templates affected?

## Plan Quality Best Practices

- **Sequence for safety:** order steps so the build stays green after each one; put risky/uncertain work early to surface unknowns.
- **Deliver incrementally:** slice into independently mergeable, testable steps rather than one big drop. Each step should leave the system working.
- **Make it reversible:** note how each risky step is rolled back (feature flag, expand/contract migration, revertible commit).
- **State assumptions and open questions explicitly;** flag anything that needs a decision before coding.
- **Right-size:** if a step is too vague to implement directly, break it down; if the whole plan is large, mark the smallest shippable milestone.
- **Tie each step to verification:** name the test/check that proves the step is done, not just "implement X".

## Output Rules

- Generated markdown must be prefixed with `writing-plans-`.
- Store temporary plans in `tasks/TASK-N/`.
- Include commands that actually match the project tooling.
- Do not include fake certainty about unavailable tests or tools.

## Final Output

Return the plan path, key decisions, Context Summary, and recommended next command.
