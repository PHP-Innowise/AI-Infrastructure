---
name: documentation-generator
description: Generate and maintain Laravel/PHP project documentation, READMEs, ADRs, API docs, changelogs, and living specs.
phase: finalization
flow-next: release
flow-alternatives: [finishing-branch, verify]
related: [architect, api-designer, coder]
---

# Documentation Generator

## Overview

Document Laravel/PHP implementation details in a way that helps future maintainers run, test, extend, and safely operate the system.

## Documentation Targets

- `README.md` for setup, local development, test commands, deployment notes, and troubleshooting.
- `specs/docs-generator-implementation.md` for living implementation details.
- ADRs for durable architecture decisions.
- API docs for public or cross-team endpoints.
- Changelog entries for release-visible changes.

## Laravel README Checklist

Include:

- PHP and Composer versions.
- Laravel version.
- Required PHP extensions.
- Local setup commands.
- Database setup and migration commands.
- Queue, scheduler, cache, storage, and mail setup if used.
- Test, format, and static-analysis commands.
- Frontend asset commands if Vite or another frontend pipeline is used.
- Environment variables by name only, never secret values.

Example local setup commands:

```bash
composer install
php artisan key:generate
php artisan migrate --seed
php artisan test
```

## Living Specification Updates

Read `specs/MANIFEST.md` first. Update or create `specs/docs-generator-implementation.md` when build process, deployment, tooling, queue/schedule behavior, or development workflow changes.

Use task-prefixed sections:

```markdown
### [TASK-001] Invitation Registration Tooling (2026-06-29)

**Runtime:** PHP 8.2+, Laravel 11

**Verification:**
- `php artisan test`
- `vendor/bin/pint --test`
- `vendor/bin/phpstan analyse`

**Operational Notes:**
- Invitation emails are queued.
- Scheduler must run every minute in production.
```

## API Documentation

For API changes, include:

- Route table.
- Auth requirements.
- Request validation.
- Response resources.
- Error codes.
- Rate limits.
- OpenAPI generation command if configured.

## ADR Template

```markdown
# ADR-0001: Use Laravel Policies For Invitation Authorization

## Status
Accepted

## Context
[Why this decision exists]

## Decision
[What was decided]

## Consequences
[Trade-offs and follow-up requirements]
```

## Final Output

Return:

- Documents updated.
- Key operational details.
- Verification performed.
- Context Summary.
- Next by flow: `/release`, `/finishing-branch`, or `/verify`.
