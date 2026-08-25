# WordPress Definition of Done

Run the highest applicable tier. Prefer repository Composer/npm/CI scripts.
Missing tooling is `N/A - tooling not configured`; do not install it without
approval.

## Minimum

- [ ] Review `git diff --stat` and all relevant changed files.
- [ ] Preserve unrelated user changes; do not modify WordPress core, generated
      vendor code, `.env`, secrets, database dumps, uploads, or local settings.
- [ ] Follow task/spec naming and update durable specs when required.
- [ ] Validate Memory Bank and Project Brain when present and used.
- [ ] Provide a Context Summary, Next Steps, and honest verification status.

## Standard

All Minimum items plus applicable checks:

- [ ] Confirm repository shape and supported WordPress, PHP, Gutenberg,
      WooCommerce, Node and database versions from canonical sources.
- [ ] Validate Composer metadata and syntax-check changed PHP.
- [ ] Run the full relevant PHP suite (`composer test`, PHPUnit, Pest,
      wp-browser/Codeception, or project equivalent); filtered tests are labeled.
- [ ] Run WordPress Coding Standards through project PHPCS scripts.
- [ ] Run PHPStan/Psalm and configured JavaScript unit/lint/style checks.
- [ ] Run the production asset/block build when source assets or metadata changed.
- [ ] Add focused coverage for success and the highest-risk failure.
- [ ] Verify lifecycle registration, namespacing/prefixing and public backward
      compatibility for hooks, routes, blocks, shortcodes and stored formats.
- [ ] Verify capability/object authorization separately from nonce/authentication.
- [ ] Verify sanitization, domain validation, contextual escaping, REST
      permissions, SQL preparation, safe redirects/HTTP/uploads and secret handling.
- [ ] Verify data storage choice, migration rerun/recovery, cache invalidation,
      cron/background idempotency and multisite scope when relevant.
- [ ] Verify accessibility, i18n, RTL and editor/frontend behavior for UI changes.
- [ ] Self-review against `GOLDEN-PRINCIPLES.md`.

## Full

All Standard items plus:

- [ ] Run Composer/npm security audits and triage applicable advisories.
- [ ] Run relevant integration environment, WP-CLI, REST, browser/e2e,
      multisite and WooCommerce HPOS/checkout checks.
- [ ] Test minimum supported runtime versions or cite CI evidence that covers them.
- [ ] Validate release packaging: correct plugin/theme headers and versions;
      no dev dependencies, secrets, caches or unintended source artifacts.
- [ ] Review activation/deactivation/uninstall, schema/data backfill, rollback,
      scheduled-action cleanup and large-site/network operational impact.
- [ ] Update changelog, public docs, upgrade notices and living specs.
- [ ] Review CI status and include summary, test plan, compatibility and risk in
      the PR/release description.
- [ ] Leave no unresolved release-blocking TODO/FIXME/HACK or known failing entry point.

## Typical Commands

Use only commands that the project supports:

```bash
composer validate --strict
composer audit
php -l path/to/changed.php
composer test
vendor/bin/phpunit
vendor/bin/phpcs
vendor/bin/phpstan analyse
npm audit
npm test
npm run lint:js
npm run lint:css
npm run build
wp core version
wp plugin status
wp theme status
```

Environment-specific commands may include `wp-env`, DDEV, Lando, Docker,
Playwright, VIP tooling, WooCommerce test helpers, or host scripts. Do not
guess production credentials or mutate production data to satisfy verification.

## Failure Handling

Read complete output, fix the root cause, and rerun the exact failing command.
After three unsuccessful attempts, stop and recommend `systematic-debugger`
with the command and concise failure evidence. Any required failed check makes
the result FAIL.

## Report

Report tier; each check as PASS/FAIL/N/A with command or evidence; compatibility,
data, security and operational risks; unresolved failures; Context Summary; and
recommended next step.
