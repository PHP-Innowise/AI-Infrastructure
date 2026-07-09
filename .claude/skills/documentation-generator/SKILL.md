---
name: documentation-generator
description: Generate and maintain native PHP project documentation, READMEs, ADRs, API docs, changelogs, and living specs.
phase: finalization
flow-next: release
flow-alternatives: [finishing-branch, verify]
related: [architect, api-designer, coder]
---

# Documentation Generator

## Overview

Document native PHP implementation details in a way that helps future maintainers run, test, extend, and safely operate the system.

## Documentation Targets

- `README.md` for setup, local development, test commands, deployment notes, and troubleshooting.
- `specs/docs-generator-implementation.md` for living implementation details.
- ADRs for durable architecture decisions.
- API docs for public or cross-team endpoints.
- Changelog entries for release-visible changes.

## README Checklist

Include:

- Required PHP version and extensions (e.g. `ext-pdo`, `ext-mbstring`).
- Composer install/usage.
- Local setup and how to run the app (front controller / built-in server).
- Database setup and migration commands.
- Worker/queue, cron, cache, and mail setup if used.
- Test, format, and static-analysis commands (prefer Composer scripts).
- Any frontend asset build commands if a pipeline exists.
- Environment variables by name only, never secret values.

Example local setup commands:

```bash
composer install
cp .env.example .env   # then fill values locally (never commit real secrets)
php bin/migrate.php    # or the project's migration command
php -S localhost:8000 -t public
composer test
```

## Living Specification Updates

Read `specs/MANIFEST.md` first. Update or create `specs/docs-generator-implementation.md` when build process, deployment, tooling, worker/cron behavior, or development workflow changes.

Use task-prefixed sections:

```markdown
### [TASK-001] Invitation Registration Tooling (2026-06-29)

**Runtime:** PHP 8.3+, PDO MySQL

**Verification:**
- `composer test`
- `composer lint`
- `composer analyse`

**Operational Notes:**
- Invitation emails are dispatched by a queue worker.
- A cron entry must run the scheduler script every minute in production.
```

## API Documentation

For API changes, include:

- Route table.
- Auth requirements.
- Request validation.
- Response serializer shapes.
- Error codes.
- Rate limits.
- OpenAPI generation command if configured.

## ADR Template

```markdown
# ADR-0001: Use A Dedicated Access-Control Service For Invitation Authorization

## Status
Accepted

## Context
[Why this decision exists]

## Decision
[What was decided]

## Consequences
[Trade-offs and follow-up requirements]
```

## Documentation Best Practices

- **Keep docs next to the code** and update them in the same change that alters behavior; stale docs are worse than none.
- **Write for the newcomer:** a reader should be able to clone, install, configure, and run from the README alone.
- **Show runnable examples;** prefer copy-pasteable commands and real code over prose. Verify the commands actually work.
- **Explain the "why", not just the "what":** reserve rationale and trade-offs for ADRs so history is not lost.
- **Follow Keep a Changelog + SemVer** for `CHANGELOG.md`: group by Added/Changed/Fixed/Removed, newest first, human-readable.
- **Document by name, never by value:** list required environment variables and secrets by name only.
- **One source of truth:** link to the canonical spec instead of duplicating long-lived information across docs.

## Final Output

Return:

- Documents updated.
- Key operational details.
- Verification performed.
- Context Summary.
- Next by flow: `/release`, `/finishing-branch`, or `/verify`.
