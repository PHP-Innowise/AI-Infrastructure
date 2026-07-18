# Definition of Done

Tiered checklist for native PHP work. Every item should be verified by command when tooling exists. If tooling is missing, report `N/A - tooling not configured` and do not install it without user approval.

Prefer running checks through the project's Composer scripts (for example `composer test`, `composer analyse`, `composer lint`) so local and CI use the same entry points. Fall back to the direct `vendor/bin/*` commands when no script is defined.

## Minimum

Use for documentation, planning, and small non-code tasks.

- [ ] Working tree state reviewed: `git diff --stat`
- [ ] Task/spec file naming follows skill-prefix convention.
- [ ] Context Summary provided with 2-3 sentences and Next Steps.
- [ ] No `.env`, secrets, credentials, database dumps, or personal local settings were read or modified.

## Standard

Use for implementation tasks.

All Minimum items, plus:

- [ ] Composer metadata is valid: `composer validate --strict` if `composer.json` exists.
- [ ] Syntax is clean: `php -l` on changed files (or `find src -name '*.php' -print0 | xargs -0 -n1 php -l`).
- [ ] Tests pass: `composer test`, `vendor/bin/phpunit`, or `vendor/bin/pest` depending on the project.
- [ ] Formatting passes: `vendor/bin/php-cs-fixer fix --dry-run --diff` or `vendor/bin/phpcs` if configured.
- [ ] Static analysis passes: `vendor/bin/phpstan analyse` or `vendor/bin/psalm` if configured.
- [ ] New behavior has focused test coverage, at least the happy path and the highest-risk failure path.
- [ ] Database changes include versioned migrations (or reviewed SQL) and any needed seed/fixture data.
- [ ] Input validation and authorization are implemented at the boundary.
- [ ] No OWASP Top 10 risk was introduced.
- [ ] Code was self-reviewed against `.codex/GOLDEN-PRINCIPLES.md`.

## Full

Use before merge, release, or PR creation.

All Standard items, plus:

- [ ] Dependency audit is clean or triaged: `composer audit`.
- [ ] CI status reviewed: `gh run list --limit 1` when GitHub Actions is used.
- [ ] PR description includes summary, test plan, and risk notes.
- [ ] `CHANGELOG.md` updated when release notes are expected.
- [ ] Living specs updated when architecture, API behavior, database schema, or user-facing workflows changed.
- [ ] No unresolved TODO/FIXME/HACK comments remain in changed source files.
- [ ] Public documentation updated for user-facing changes.
- [ ] Cron/worker/queue, cache, and migration impacts are documented when applicable.

## Command Selection

Prefer the command already used by the project:

```bash
composer validate --strict
composer audit
composer test           # or: vendor/bin/phpunit / vendor/bin/pest
composer lint           # or: vendor/bin/php-cs-fixer fix --dry-run --diff / vendor/bin/phpcs
composer analyse        # or: vendor/bin/phpstan analyse / vendor/bin/psalm
php -l path/to/File.php
```

For server-rendered frontend work (PHP templates + HTML/CSS), run the project-specific checks only if tooling exists:

```bash
<html-or-template-lint-command>
<css-lint-command>
<frontend-build-command>
```

Otherwise verify markup manually: valid HTML5, semantic structure, and the accessibility rules in `.agents/skills/wcag-accessibility/`.

## Failure Handling

1. Read the full failure output.
2. Fix the root cause, not just the symptom.
3. Re-run the failing command.
4. Stop after three unsuccessful fix attempts and escalate to `/debugger` with the exact command and failure summary.

## Reporting

Final Context Summary must include:

- Commands run.
- Pass/fail/N/A status.
- Any unresolved risks.
- Recommended next command in the workflow.
