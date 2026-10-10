---
description: PHP/Symfony 7.4 stack, Composer dependencies, and dev tooling for the PracticePerfect walking skeleton in Task/app/
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

# Stack

## Language and framework

- PHP: `composer.json` requires `>=8.2` and pins the Composer resolution platform to PHP 8.4 (`composer.json` `config.platform.php`). The actual runtime is PHP 8.4, built from the `php:8.4-fpm-alpine` base image in `Task/app/docker/php/Dockerfile`, whose own comment notes PHP 8.4 "satisfies Symfony 7.4's >=8.2 floor and keeps security support well past PHP 8.2's end of life."
- Framework: Symfony 7.4 (LTS). `composer.json` pins every `symfony/*` component to `7.4.*`; resolved versions in `composer.lock` include `symfony/framework-bundle` v7.4.16 and `symfony/security-bundle` v7.4.15.
- Kernel: `Task/app/src/Kernel.php` — `App\Kernel extends Symfony\Component\HttpKernel\Kernel`, using `MicroKernelTrait`, no custom overrides.
- Dependency manager: Composer only. There is no `package.json` in this scope; frontend assets are served by Symfony AssetMapper (see below), not a Node build.
- Recipe manager: `symfony/flex` `^2`. Installed recipes are recorded in `Task/app/symfony.lock` (one entry per bundle/recipe, e.g. `symfony/framework-bundle`, `doctrine/doctrine-bundle`, `symfony/stimulus-bundle`, `symfony/webapp-pack`).

## Core dependencies (`Task/app/composer.json` `require`, resolved versions from `Task/app/composer.lock`)

- **Persistence**: `doctrine/orm` `^3.6` (resolved `3.6.8`), `doctrine/dbal` (resolved `4.4.4`, pulled in transitively — not listed directly in `composer.json`), `doctrine/doctrine-bundle` `^3.3` (resolved `3.3.1`), `doctrine/doctrine-migrations-bundle` `^4.0` (resolved `4.0.0`).
- **Messaging**: `symfony/doctrine-messenger` `7.4.*` (resolved v7.4.15) — Messenger's Doctrine DBAL transport; see `INTEGRATIONS.md`.
- **Security**: `symfony/security-bundle` `7.4.*` (resolved v7.4.15).
- **HTTP/routing/kernel**: `symfony/framework-bundle`, `symfony/http-client`, `symfony/runtime` (all `7.4.*`) — `symfony/runtime` supplies `vendor/autoload_runtime.php`, the bootstrap used by `Task/app/public/index.php` and `Task/app/bin/console`.
- **Frontend delivery**: `symfony/asset`, `symfony/asset-mapper` (`7.4.*`, resolved v7.4.15), `symfony/stimulus-bundle` (`^3.4`, resolved v3.4.0), `symfony/ux-turbo` (`^3.4`, resolved v3.4.0). No Webpack Encore, no Node toolchain in this scope — see `STRUCTURE.md` for `Task/app/assets/`.
- **Templating**: `twig/twig` (`^2.12|^3.0`, resolved v3.28.0), `twig/extra-bundle`, `symfony/twig-bundle`.
- **Forms/validation/serialization**: `symfony/form`, `symfony/validator`, `symfony/serializer`, `symfony/property-info`, `symfony/property-access` (all `7.4.*`), plus `phpdocumentor/reflection-docblock` and `phpstan/phpdoc-parser` (property-info's docblock-extractor dependencies).
- **Mail/notifications**: `symfony/mailer`, `symfony/notifier`, `symfony/mime` (`7.4.*`).
- **Other**: `symfony/uid`, `symfony/expression-language`, `symfony/web-link`, `symfony/process`, `symfony/intl`, `symfony/string`, `symfony/translation`, `symfony/yaml`, `symfony/monolog-bundle` (`^3.0|^4.0`).

## Dev/test dependencies (`Task/app/composer.json` `require-dev`)

- `phpunit/phpunit` `^12.5` (resolved `12.5.33`) — see `TESTING.md`.
- `phpstan/phpstan` `^2.2` (resolved `2.2.8`) with `phpstan/phpstan-doctrine` `^2.0` and `phpstan/phpstan-symfony` `^2.0` extensions, configured at level 8 by `Task/app/phpstan.dist.neon`.
- `zenstruck/foundry` `^2.11` (resolved v2.11.3) — object factories and "Stories"; `Task/app/src/Story/AppStory.php` is the one Story registered in this scope.
- `doctrine/doctrine-fixtures-bundle` `^4.3` (resolved `4.3.1`) — `Task/app/src/DataFixtures/AppFixtures.php` is the one Fixture registered in this scope.
- `symfony/maker-bundle` `^1.0`, `symfony/web-profiler-bundle`, `symfony/debug-bundle`, `symfony/stopwatch` (dev/test-only bundle registrations — see `Task/app/config/bundles.php`).
- `symfony/browser-kit`, `symfony/css-selector` — power `WebTestCase`/`KernelBrowser`, used by `Task/app/tests/Smoke/HealthSmokeTest.php`.

## What is deliberately absent in this scope

- No Redis: `Task/app/config/packages/cache.yaml` uses the filesystem-backed default `cache.app`; PostgreSQL carries the Messenger transport instead of a separate broker (see `INTEGRATIONS.md`). `Task/app/README.md` states this is deliberate ("There is deliberately no Redis").
- No Node/npm/Webpack Encore: frontend assets are served through AssetMapper (`Task/app/config/packages/asset_mapper.yaml`, `Task/app/importmap.php`).
- No Stripe SDK or any payment library: nothing in `Task/app/composer.json` matches "stripe"; `Task/app/env.example` reserves blank `STRIPE_*` variables for a later epic only.
- No usage of the `symfony/http-client` dependency yet: nothing under `Task/app/src/` references it in this scope (`grep -rl HttpClient Task/app/src/` returns nothing).
