---
name: documentation-generator
description: Generate and maintain Symfony project documentation, READMEs, ADRs, API docs, worker/cron docs, deployment notes, and living specs.
phase: finalization
flow-next: verify
flow-alternatives: [release, finishing-branch]
---

# Symfony Documentation Generator

Write documentation that is accurate, runnable, appropriately scoped, and maintainable with the code.

## Choose The Right Artifact

| Information | Location |
| --- | --- |
| Setup, local development, common commands | Root or project README |
| Durable architecture/security/data/API decision | Skill-prefixed living spec in `specs/` |
| Significant decision and alternatives | Project ADR location when configured |
| Temporary implementation/research notes | `tasks/TASK-N/{skill-name}-{purpose}.md` |
| Public HTTP contract | OpenAPI/API Platform config plus API documentation |
| Deployment/worker/cron/runbook behavior | Operations documentation used by the project |
| User-visible release change | `CHANGELOG.md` following project format |

Read `specs/MANIFEST.md` before changing living specs and check `tasks/.task-counter` before creating a task directory. Do not duplicate a durable decision across multiple files; link to its source of truth.

## Workflow

1. Identify the audience, decision or workflow, owner, and event that should cause the documentation to change.
2. Read the implemented code/config/tests and existing documentation. Documentation does not override runtime behavior or policy.
3. Verify every command, path, option, route, environment/config key, version statement, and example against the repository.
4. Write the smallest complete document with prerequisites, expected result, failure/recovery behavior, and links to deeper sources.
5. Run snippets or validation commands where safe. Mark placeholders clearly and never invent successful output.
6. Check local links, headings, naming policy, sensitive-data exposure, and duplication.
7. Update `specs/MANIFEST.md`, changelog, OpenAPI, or runbook indexes when their contracts require it.

## Symfony Content Checklist

Document applicable concerns:

- supported PHP/Symfony/component versions and consuming-project detection;
- Composer/Flex setup and recipe/config files, without exposing `.env` or secrets;
- routes/controllers, request DTOs/Forms/Validator behavior, voters/access rules, response/error contracts;
- Controller -> Service -> Repository placement and transaction/side-effect boundaries;
- Doctrine entities, constraints, migrations, backfills, compatibility windows, and rollback limitations;
- Messenger transports/routing, retry/failure policy, worker start/stop/restart, monitoring, replay, and payload compatibility;
- console commands, Scheduler/cron ownership, locking, idempotency, exit codes, and alerting;
- Twig/Symfony UX asset build, progressive enhancement, accessibility, and browser support;
- cache pools/keys/invalidation/warmup, external services, timeouts, and degraded behavior;
- deployment order, migrations, cache warmup, worker drain/restart, health checks, observability, abort conditions, and forward recovery.

## Environment And Secret Safety

- Document variable/secret names, purpose, required/optional status, format, and source system; never include real values.
- Prefer Symfony config, Symfony secrets, and deployment secret managers; document configuration contracts without inspecting secret-bearing files.
- Do not read, print, edit, or quote `.env` files while generating documentation.
- Redact tokens, credentials, private URLs, personal data, database dumps, and production identifiers from examples and command output.

## API And Operations Examples

- Keep examples minimal and executable, using reserved domains and synthetic data.
- Show authentication placeholders without plausible secrets.
- Include expected status/exit behavior and the most important failure/recovery command.
- Explain destructive or irreversible commands before showing them; do not normalize unsafe flags.
- For workers/cron, include process ownership, concurrency, time/memory limits, signals, retries, failure inspection, and deployment restart behavior.

## Quality Rules

- Use precise headings, short paragraphs, tables only for comparable data, and code fences with language identifiers.
- Prefer links to official Symfony/Doctrine/API Platform documentation for version-sensitive details.
- Avoid promises such as “always safe,” “zero downtime,” or “fully compatible” without evidence and stated assumptions.
- Keep examples aligned with declared PHP/Symfony versions and existing project conventions.
- Use Keep a Changelog/SemVer only when the project already follows them or explicitly adopts them.

## Verification

Run applicable checks such as Markdown/local-link validation, OpenAPI validation, example command help, config linting, route inspection, and the active edition's DOD. Report unavailable tooling as N/A rather than installing it.

## Output

Report documents changed, audience and source-of-truth decisions, commands/examples verified, indexes/specs/changelog updated, unresolved assumptions, Context Summary, and Next Steps.
