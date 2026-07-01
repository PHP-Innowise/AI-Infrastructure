# Definition of Done

Tiered checklist for Laravel/PHP work. Every item should be verified by command when tooling exists. If tooling is missing, report `N/A - tooling not configured` and do not install it without user approval.

## Minimum

Use for documentation, planning, and small non-code tasks.

- [ ] Working tree state reviewed: `git diff --stat`
- [ ] Task/spec file naming follows skill-prefix convention.
- [ ] Context Summary provided with 2-3 sentences and Next Steps.
- [ ] No `.env`, secrets, credentials, database dumps, or personal local settings were read or modified.

## Standard

Use for implementation tasks.

All Minimum items, plus:

- [ ] Composer metadata is valid: `composer validate` if `composer.json` exists.
- [ ] Tests pass: `php artisan test`, `vendor/bin/pest`, or `vendor/bin/phpunit` depending on the project.
- [ ] Formatting passes: `vendor/bin/pint --test` if Pint is configured.
- [ ] Static analysis passes: `vendor/bin/phpstan analyse` or `vendor/bin/psalm` if configured.
- [ ] Laravel route/config changes are coherent: `php artisan route:list` when routes changed.
- [ ] New behavior has focused test coverage, at least the happy path and the highest-risk failure path.
- [ ] Database changes include migrations and, when useful, factories or seeders.
- [ ] Authorization and validation are implemented at the boundary.
- [ ] No OWASP Top 10 risk was introduced.
- [ ] Code was self-reviewed against `.claude/GOLDEN-PRINCIPLES.md`.

## Full

Use before merge, release, or PR creation.

All Standard items, plus:

- [ ] CI status reviewed: `gh run list --limit 1` when GitHub Actions is used.
- [ ] PR description includes summary, test plan, and risk notes.
- [ ] `CHANGELOG.md` updated when release notes are expected.
- [ ] Living specs updated when architecture, API behavior, database schema, or user-facing workflows changed.
- [ ] No unresolved TODO/FIXME/HACK comments remain in changed source files.
- [ ] Public documentation updated for user-facing changes.
- [ ] Queue, schedule, cache, event, and migration impacts are documented when applicable.

## Laravel Command Selection

Prefer the command already used by the project:

```bash
composer validate
php artisan test
vendor/bin/pest
vendor/bin/phpunit
vendor/bin/pint --test
vendor/bin/phpstan analyse
vendor/bin/psalm
php artisan route:list
```

For frontend assets in Laravel projects, run the project-specific lint/build commands only if frontend tooling exists:

```bash
<frontend-lint-command>
<frontend-build-command>
```

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
