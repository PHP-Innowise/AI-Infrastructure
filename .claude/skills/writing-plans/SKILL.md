---
name: writing-plans
description: Create implementation plans for Laravel/PHP work. Use after requirements, brainstorming, architecture, or API design when execution needs clear steps.
phase: planning
flow-next: git-worktrees
flow-alternatives: [coder, test-generator]
related: [requirements-analyst, brainstorming, architect, api-designer]
---

# Writing Plans

## Overview

Create precise implementation plans for Laravel/PHP projects. Plans should be specific enough for an implementer to execute without rediscovering the architecture.

## Required Inputs

Read available context:

- Requirement docs in `tasks/TASK-N/`.
- Living specs in `specs/`.
- Relevant Laravel files: routes, controllers, requests, models, migrations, tests, policies, services/actions.
- Existing project conventions for tests, formatting, static analysis, and frontend tooling.

## Plan Structure

```markdown
# [Feature] Implementation Plan

## Goal
[1-2 sentence outcome]

## Existing Context
- Relevant files and current behavior.

## Proposed Laravel Design
- Routes/controllers/requests/resources.
- Models/migrations/factories.
- Policies/gates.
- Services/actions/jobs/events if needed.

## Implementation Steps
1. [Specific file-level step]
2. [Specific file-level step]

## Test Plan
- Feature tests.
- Unit tests.
- Authorization/validation cases.

## Verification
- `php artisan test`
- `vendor/bin/pint --test`
- `vendor/bin/phpstan analyse`

## Risks
- [Migration, security, queue, cache, API compatibility risks]
```

## Laravel Planning Checklist

- Does a migration need a backfill or safe rollout?
- Does route model binding apply?
- Which Form Requests validate input?
- Which policies authorize behavior?
- Are API Resources needed for response stability?
- Are factories needed for tests?
- Should work be queued or synchronous?
- Are events/notifications/mail/storage involved?
- Are indexes needed?
- Are frontend assets or Blade/Livewire/Inertia views affected?

## Output Rules

- Generated markdown must be prefixed with `writing-plans-`.
- Store temporary plans in `tasks/TASK-N/`.
- Include commands that actually match the project tooling.
- Do not include fake certainty about unavailable tests or tools.

## Final Output

Return the plan path, key decisions, Context Summary, and recommended next command.
