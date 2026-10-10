---
description: PHPUnit layout, the one smoke test, PHPStan level 8, and how to run both for the PracticePerfect walking skeleton in Task/app/
mapped_commit: 7514d20e82620f1509cb5ae7870044d8990be95a
mapped_scope:
  - Task/app/src/
  - Task/app/config/
  - Task/app/migrations/
  - Task/app/templates/
  - Task/app/assets/
  - Task/app/tests/
  - Task/app/docker/
  - Task/app/compose.yaml
  - Task/app/Makefile
---

# Testing

## Framework and configuration

PHPUnit `^12.5` (resolved `12.5.33` in `composer.lock`), configured by `Task/app/phpunit.dist.xml`:

- `bootstrap="tests/bootstrap.php"` — boots `Symfony\Component\Dotenv\Dotenv` against `.env` (`Task/app/tests/bootstrap.php`).
- `failOnDeprecation`, `failOnNotice`, `failOnWarning` all `true`.
- `<env name="APP_ENV" value="test" force="true">` **and** `<server name="APP_ENV" value="test" force="true">` are both set — not redundant. The file's own comment (echoed in `Task/app/README.md`) explains why: `KernelTestCase` resolves `$_ENV['APP_ENV']` before `$_SERVER['APP_ENV']`, and the container already exports a real `APP_ENV=dev`; setting only the `<server>` value (Symfony's own default recipe) would leave `$_ENV` winning and boot the dev kernel instead of test.
- `<extensions><bootstrap class="Zenstruck\Foundry\PHPUnit\FoundryExtension" /></extensions>` — pairs with `src/Story/AppStory.php` and zenstruck/foundry's factory system. No Foundry factories exist in this scope, only the Story stub.
- Single `<testsuite name="Project Test Suite">` pointed at the whole `tests/` directory.
- `<source>` includes `src/` for coverage/deprecation tracking, with `Doctrine\Deprecations\Deprecation::trigger`/`::delegateTriggerToBackend` and `trigger_deprecation` registered as deprecation triggers.

## What exists in this scope

Exactly one test class: `Task/app/tests/Smoke/HealthSmokeTest.php` (`final class HealthSmokeTest extends WebTestCase`). Its own docblock: "The walking skeleton's proof of life: the application boots, routes a request, reaches PostgreSQL, and is connected as a role that Row-Level Security applies to." Three test methods, all driving `GET /health` through a `KernelBrowser` (`self::createClient()`):

- `testHealthEndpointReportsHealthy()` — asserts `200`, `content-type: application/json`, and decoded `status === 'healthy'`.
- `testDatabaseIsReachable()` — asserts decoded `checks.database.status === 'ok'`.
- `testApplicationRoleDoesNotBypassRowLevelSecurity()` — asserts decoded `checks.tenancy_isolation.status === 'ok'`, with an explicit assertion message calling out RLS bypass. Its docblock: "Tenancy isolation is the highest-risk defect class in this product... so the skeleton asserts it from the very first commit."

A private `decode()` helper parses the JSON body with `json_decode(..., \JSON_THROW_ON_ERROR)`.

No unit tests, no repository/integration tests, and no fixture-loading tests exist in this scope. `src/DataFixtures/AppFixtures.php` and `src/Story/AppStory.php` are both no-op stubs (see `CONCERNS.md`), so nothing exercises them yet either.

## Static analysis (run alongside testing, via the same `lint` target)

`Task/app/phpstan.dist.neon`: level **8**, over `bin/`, `config/`, `public/`, `src/`, `tests/`, using `vendor/phpstan/phpstan-symfony/{extension,rules}.neon` and the container XML at `var/cache/dev/App_KernelDevDebugContainer.xml`. The file's own comment explains the level choice: "Level 8 from the first commit rather than level 6. Raising the level later, across nine modules and ~45 entities, is the kind of debt that never gets paid; nullability discipline is cheapest to hold from an empty `src/`." The Doctrine PHPStan extension (`phpstan/phpstan-doctrine`, already a dependency per `STACK.md`) is not yet wired into this config — the comment defers it to "the Epic-01 work, together with `tests/object-manager.php`, since it needs a bootable EntityManager and there are no entities yet."

## How to run (Docker Compose only)

- `make test` → `test-db` (creates `practiceperfect_test` as the owner role, grants it to the app role via `docker/postgres/grant-roles.sh`, migrates it as owner — because PostgreSQL default privileges are per-database and initdb only covers the primary database) → `vendor/bin/phpunit` inside the `php` container.
- `make lint` → `lint:container`, `lint:yaml config`, `lint:twig templates`, `composer validate --strict`, `vendor/bin/phpstan analyse --no-progress`, in that order, inside the `php` container.
- `make smoke` is a separate, black-box check: `curl -fsS http://localhost:$HTTP_PORT/health`, run against the already-running stack from the host, not through PHPUnit. Despite the similar name, it is distinct from `HealthSmokeTest`.
