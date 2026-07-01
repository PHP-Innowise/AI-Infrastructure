# AGENTS.md - Policy Rules

These are enforceable rules for the Laravel/PHP accelerator. Wishes are ignored; constraints are enforced.

## Hierarchy of Sources of Truth

1. **Enforcement** (`.claude/hooks`, CI, linters, static analysis) - automated, highest authority.
2. **Policy** (`AGENTS.md`) - mandatory behavior and safety rules.
3. **Architecture** (`specs/`) - project-specific decisions.
4. **Operations** (`.claude/skills/`) - how skills execute.
5. **Examples** (`examples/`) - reference outputs, never stronger than policy.
6. **Documentation** (`README.md`, `.claude/README.md` if present) - human reference.

## File Naming

- MUST prefix generated task/spec markdown with the skill name: `{skill-name}-{purpose}.md`.
- MUST use zero-padded task directories: `TASK-001/`, `TASK-002/`.
- MUST place temporary task docs in `tasks/TASK-{N}/`.
- MUST place living specs in `specs/`.
- MUST NOT create unprefixed markdown files in `tasks/` or `specs/`, except `README.md`, `CHANGELOG.md`, and `MANIFEST.md`.

## Agent Behavior

- MUST execute only the selected skill, then stop.
- MUST NOT chain to another skill automatically.
- MUST output a Context Summary and Next Steps.
- MUST NOT make workflow decisions for the user when a command is supposed to offer alternatives.
- MUST read relevant Laravel code, routes, migrations, tests, and specs before modifying behavior.

## Laravel Code Quality

- MUST prefer Laravel conventions before adding custom architecture.
- MUST validate request input at boundaries with Form Requests, validators, or explicit validation.
- MUST authorize protected actions with policies, gates, middleware, or route model binding constraints.
- MUST keep controllers thin when behavior becomes non-trivial; use services/actions/jobs where they clarify intent.
- MUST protect Eloquent mass assignment with `$fillable`, `$guarded`, DTOs, or explicit assignment.
- MUST use migrations for schema changes.
- MUST add or update factories/seeders when tests require realistic data.
- MUST use API Resources or a documented response contract for stable public APIs.

## Verification

- MUST run applicable checks from `.claude/DOD.md` before claiming completion.
- MUST run tests if test tooling exists.
- MUST run formatting/lint/static analysis if configured.
- MUST NOT claim completion with failing tests, failing static analysis, or known broken routes.
- MUST report unavailable tooling as `N/A - tooling not configured`; do not install tooling without user approval.

## Git Safety

- MUST NOT skip hooks with `--no-verify`.
- MUST NOT force-push, hard-reset, or drop database tables without explicit user consent.
- MUST NOT overwrite unrelated user changes.

## Security

- MUST NOT read, print, edit, or commit `.env` files or secrets.
- MUST NOT introduce OWASP Top 10 vulnerabilities.
- MUST validate file uploads by type, size, storage location, and visibility.
- MUST avoid raw SQL unless bindings are used and the reason is documented.
- MUST keep secrets in environment/config systems, never in source code.
- MUST consider CSRF/session behavior for web routes and token behavior for API routes.

## Context And Documentation

- MUST read `specs/MANIFEST.md` before writing living specs.
- MUST check `tasks/.task-counter` before creating task directories.
- MUST avoid duplicating long-lived information across specs; reference the source spec instead.
- MUST update specs when architecture, API behavior, or user-facing workflows change.

## Definition Of Done

- See `.claude/DOD.md` for the tiered Laravel/PHP verification checklist.
- MUST include verification evidence in final Context Summary when implementation work is performed.
