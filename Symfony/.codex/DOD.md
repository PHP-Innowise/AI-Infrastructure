# Definition of Done - Symfony Layered Architecture

Tiered checklist for Symfony work. Every item should be verified by command when tooling exists. If tooling is missing, report `N/A - tooling not configured` and do not install it without user approval.

Prefer project Composer scripts (`composer test`, `composer analyse`, `composer lint`) so local and CI use the same entry points.

## Minimum

Use for documentation, planning, and small non-code tasks.

- [ ] Working tree state reviewed: `git diff --stat`.
- [ ] Task/spec file naming follows skill-prefix convention.
- [ ] Context Summary provided with 2-3 sentences and Next Steps.
- [ ] No `.env`, secrets, credentials, database dumps, or personal local settings were read or modified.
- [ ] Relevant active memory chunks were verified against current sources; stale chunks were updated or reported.
- [ ] Memory-bank structure passes `python3 memory-bank/scripts/validate.py` when `memory-bank/` exists.

## Standard

Use for implementation tasks.

All Minimum items, plus:

- [ ] Composer metadata is valid: `composer validate --strict` if `composer.json` exists.
- [ ] PHP syntax is clean: `php -l` on changed PHP files.
- [ ] Tests pass: `composer test`, `vendor/bin/phpunit`, or `vendor/bin/pest`.
- [ ] Formatting passes: configured PHP-CS-Fixer, PHP_CodeSniffer, Easy Coding Standard, or project equivalent.
- [ ] Static analysis passes: PHPStan or Psalm when configured.
- [ ] Symfony container/routes are coherent when relevant: `php bin/console lint:container`, `php bin/console debug:router`.
- [ ] Changed Symfony configuration/templates/translations are valid when relevant: `php bin/console lint:yaml config`, `php bin/console lint:twig templates`, and `php bin/console lint:xliff translations`.
- [ ] Doctrine changes include migrations and schema validation when relevant: `php bin/console doctrine:migrations:diff --check-database-platform` or project equivalent, and `php bin/console doctrine:schema:validate --skip-sync`.
- [ ] New behavior has focused tests covering the happy path and highest-risk failure path.
- [ ] Controller -> Service -> Repository boundaries are respected.
- [ ] Pragmatic SOLID review passes: responsibilities are cohesive, dependencies point inward, contracts are narrow/substitutable, and interfaces have a concrete boundary justification.
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
- [ ] Living specs updated when architecture, API behavior, database schema, security behavior, async behavior, or user-facing workflows changed.
- [ ] No unresolved TODO/FIXME/HACK comments remain in changed source files.
- [ ] Public documentation updated for user-facing changes.
- [ ] Durable reusable context was added to `memory-bank/` only when source-backed, non-sensitive, indexed, and not already authoritative in a spec.
- [ ] Messenger workers, cron jobs, cache, migrations, and rollout impacts are documented when applicable.
- [ ] Production cache warmup/build succeeds when deployment configuration changed.
- [ ] New Symfony/PHP deprecations are absent or explicitly triaged when deprecation tooling is configured.

## Command Selection

Prefer the command already used by the project:

```bash
composer validate --strict
composer audit
composer test
composer lint
composer analyse
php bin/console lint:container
php bin/console lint:yaml config
php bin/console lint:twig templates
php bin/console lint:xliff translations
php bin/console debug:router
php bin/console doctrine:schema:validate --skip-sync
php bin/console cache:warmup --env=prod
```

For Twig/Symfony UX/frontend work, also run configured frontend checks:

```bash
npm test
npm run lint
npm run build
```

Report each unavailable command as `N/A - tooling not configured`.

## Failure Handling

1. Read the full failure output.
2. Fix the root cause, not just the symptom.
3. Re-run the failing command.
4. Stop after three unsuccessful fix attempts and escalate to `/debugger` with the exact command and failure summary.

## Reporting

Final Context Summary must include:

- Commands run.
- Pass/fail/N/A status.
- Layering decisions.
- Any unresolved risks.
- Recommended next command in the workflow.
