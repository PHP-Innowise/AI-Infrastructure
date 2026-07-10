---
name: writing-plans
description: Create implementation plans for Laravel work. Use after requirements, brainstorming, architecture, or API design when execution needs clear steps.
phase: planning
flow-next: git-worktrees
flow-alternatives: [coder, test-generator]
related: [requirements-analyst, brainstorming, architect, api-designer]
---

# Writing Plans

## Overview

Create precise implementation plans for Laravel projects. Plans should be specific enough for an implementer to execute without rediscovering the architecture.

## Required Inputs

Read available context:

- Requirement docs in `tasks/TASK-N/`.
- Living specs in `specs/`.
- Relevant Laravel files: `routes/web.php`/`routes/api.php`, Controllers/Actions, Form Requests, Eloquent models, migrations, Policies/Gates, API Resources, existing tests.
- Existing project conventions: `composer.json` scripts, Pint/Larastan config, and any frontend tooling (Blade, Livewire, Inertia, Vite).

## Plan Structure

```markdown
# [Feature] Implementation Plan

## Goal
[1-2 sentence outcome]

## Existing Context
- Relevant files and current behavior.

## Proposed Design
- Routes (`routes/web.php` / `routes/api.php`) and Controllers/Actions.
- Form Requests for validation and inline authorization.
- Eloquent models, relationships, migrations, factories/seeders.
- Policies/Gates for authorization.
- API Resources for response shape (if API work).
- Jobs/queues, Events/Listeners, Notifications, cache if needed.

## Implementation Steps
1. [Specific file-level step]
2. [Specific file-level step]

## Test Plan
- Unit tests (Actions/Services, value objects).
- Feature tests (HTTP behavior, validation, authorization, persistence).
- Policy/Gate authorization cases.

## Verification
- `php artisan test` (or `vendor/bin/pest`)
- `vendor/bin/pint --test`
- `vendor/bin/phpstan analyse` (or `vendor/bin/psalm`)

## Risks
- [Migration, security, queue, cache, API compatibility risks]
```

## Planning Checklist

- Does a migration need a backfill or safe rollout (e.g. nullable column first, backfill, then constrain)?
- How is the route-model binding resolved (implicit binding vs explicit lookup)?
- Which Form Request guards input, and does it also carry the `authorize()` check?
- Which Policy/Gate authorizes the controller action?
- Is an API Resource needed for a stable response contract?
- Are factories/seeders needed for tests and local data?
- Should work be queued (`ShouldQueue`) or run synchronously?
- Are Events/Listeners, Notifications, or Mailables involved?
- Are indexes needed for new query patterns, and is eager loading (`with()`/`load()`) used to avoid N+1s?
- Are Blade views, Livewire components, or Inertia pages affected?

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
