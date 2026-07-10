# AGENTS.md - Policy Rules

These are enforceable rules for the Laravel accelerator. Wishes are ignored; constraints are enforced.

This branch targets **Laravel** as the default backend framework. It assumes Composer + a current Laravel LTS release, Eloquent, Artisan, and the Laravel ecosystem's conventional packages. The framework-agnostic native-PHP base lives on `main`; other frameworks (Symfony, etc.) belong on their own dedicated branches.

This policy is shared across editions. The same accelerator is mirrored for **Claude Code** (`.claude/`), **Cursor** (`.cursor/`), and **Codex** (`.agents/skills` + `.codex/`). Below, paths like `<edition>/hooks` and `<edition>/skills` refer to whichever edition is active.

## Hierarchy of Sources of Truth

1. **Enforcement** (`<edition>/hooks`, CI, linters, static analysis) - automated, highest authority.
2. **Policy** (`AGENTS.md`) - mandatory behavior and safety rules.
3. **Architecture** (`specs/`) - project-specific decisions.
4. **Operations** (`<edition>/skills/`) - how skills execute.
5. **Examples** (`examples/`) - reference outputs, never stronger than policy.
6. **Documentation** (`README.md`, per-edition `README.md`) - human reference.

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
- MUST read relevant PHP code, autoload config, routes/entry points, database access, tests, and specs before modifying behavior.

## Laravel Code Quality

- MUST target the project's declared PHP and Laravel version and follow PSR-12 / PER Coding Style (enforced via Pint).
- MUST use `declare(strict_types=1);` in new PHP files and add return/property types.
- MUST autoload via Composer PSR-4; MUST NOT add manual `require` chains for application classes.
- MUST validate external input via Form Requests (or equivalent explicit validators), not inline in controllers.
- MUST authorize protected actions through Policies/Gates, not by hiding UI or relying on obscurity.
- MUST keep controllers thin; move multi-step business logic into Actions, Services, or the model layer as the project's convention dictates.
- MUST access the database through Eloquent or the query builder with bound parameters; MUST NOT concatenate untrusted input into raw SQL.
- MUST depend on abstractions (interfaces bound in a Service Provider) at integration boundaries rather than `new`-ing concrete external clients.
- MUST manage schema changes through versioned Artisan migrations, never ad-hoc production edits.
- MUST document a stable response contract via API Resources for public APIs.
- MUST use Eloquent relationships and eager loading (`with()`/`load()`) to avoid N+1 queries.

## Verification

- MUST run applicable checks from the active edition's `DOD.md` (`.claude/DOD.md`, `.cursor/DOD.md`, or `.codex/DOD.md`) before claiming completion.
- MUST run tests if test tooling exists.
- MUST run formatting/lint/static analysis if configured.
- MUST NOT claim completion with failing tests, failing static analysis, or known broken entry points.
- MUST report unavailable tooling as `N/A - tooling not configured`; do not install tooling without user approval.

## Git Safety

- MUST NOT skip hooks with `--no-verify`.
- MUST NOT force-push, hard-reset, or drop database tables without explicit user consent.
- MUST NOT overwrite unrelated user changes.

## Security

- MUST NOT read, print, edit, or commit `.env` files or secrets.
- MUST NOT introduce OWASP Top 10 vulnerabilities.
- MUST escape output in templates to prevent XSS; MUST use CSRF protection for state-changing web requests.
- MUST validate file uploads by type, size, storage location, and visibility.
- MUST use parameterized queries; never concatenate untrusted input into SQL.
- MUST keep secrets in environment/config systems, never in source code.
- MUST avoid `eval`, unsafe `unserialize` of untrusted data, and dynamic includes of untrusted paths.

## Context And Documentation

- MUST read `specs/MANIFEST.md` before writing living specs.
- MUST check `tasks/.task-counter` before creating task directories.
- MUST avoid duplicating long-lived information across specs; reference the source spec instead.
- MUST update specs when architecture, API behavior, or user-facing workflows change.

## Definition Of Done

- See the active edition's `DOD.md` (`.claude/`, `.cursor/`, or `.codex/`) for the tiered Laravel verification checklist.
- MUST include verification evidence in final Context Summary when implementation work is performed.
