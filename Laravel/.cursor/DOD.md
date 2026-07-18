# Definition of Done

Tiered checklist for Laravel work. Every item should be verified by command when tooling exists. If tooling is missing, report `N/A - tooling not configured` and do not install it without user approval.

## Minimum

Use for documentation, planning, and small non-code tasks.

- [ ] Working tree state reviewed: `git diff --stat`
- [ ] Task/spec file naming follows skill-prefix convention.
- [ ] Context Summary provided with 2-3 sentences and Next Steps.
- [ ] No `.env`, secrets, credentials, database dumps, or personal local settings were read or modified.

## Standard

Use for implementation tasks.

All Minimum items, plus:

- [ ] Composer metadata is valid: `composer validate --strict`.
- [ ] Syntax is clean: `php -l` on changed files (or `find app -name '*.php' -print0 | xargs -0 -n1 php -l`).
- [ ] Tests pass: `php artisan test` (or `vendor/bin/pest` / `vendor/bin/phpunit`).
- [ ] Formatting passes: `vendor/bin/pint --test`.
- [ ] Static analysis passes: `vendor/bin/phpstan analyse` (Larastan) or `vendor/bin/psalm` if configured.
- [ ] New behavior has focused test coverage, at least the happy path and the highest-risk failure path.
- [ ] Database changes include a migration (and factory/seeder updates where relevant).
- [ ] Input validation is via a Form Request (or explicit validator) and authorization via a Policy/Gate.
- [ ] No N+1 queries introduced (`with()`/`load()` used for relationships accessed in loops).
- [ ] No OWASP Top 10 risk was introduced.
- [ ] Code was self-reviewed against `.cursor/GOLDEN-PRINCIPLES.md`.

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
- [ ] Queue/job, cache, scheduled-command (`app/Console/Kernel.php` or `routes/console.php`), and migration impacts are documented when applicable.
- [ ] If this release includes a major Laravel version bump or changes queued job payload shapes, queues are drained/compatible before deploying (mixed-version job payloads across a Laravel major upgrade can fail).

## Command Selection

Prefer the command already used by the project:

```bash
composer validate --strict
composer audit
php artisan test          # or: vendor/bin/pest / vendor/bin/phpunit
php artisan test --parallel  # preferred for larger suites when brianium/paratest is configured
vendor/bin/pint --test
vendor/bin/phpstan analyse  # Larastan
php -l path/to/File.php
```

For frontend work (Blade, Livewire, Inertia, or a Vite-built SPA), run the project-specific checks only if tooling exists:

```bash
npm run build
npm run lint
```

Otherwise verify markup manually: valid HTML5, semantic structure, and the accessibility rules in `.cursor/skills/wcag-accessibility/`.

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
