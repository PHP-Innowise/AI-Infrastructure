---
description: Directory layout of Task/app/ (src, config, migrations, templates, assets, tests, docker) and where new code belongs
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

# Structure

The Symfony application is **not** at the repository or Symfony-edition root. It lives entirely under `Task/app/` — there is no top-level `src/` alongside it; that absence does not make the repository unmappable. Everything below is relative to `Task/app/`.

## `src/` — PSR-4 `App\` (`composer.json` `autoload.psr-4`)

- `Controller/` — `HomeController.php`, `HealthController.php` (see `ARCHITECTURE.md`). Also holds an empty `.gitignore` placeholder, a leftover of the original `symfony/skeleton` scaffold that predates any real controller and is unrelated to the two classes now present.
- `DataFixtures/` — `AppFixtures.php`, a `doctrine/doctrine-fixtures-bundle` `Fixture` whose `load()` body is commented-out example code only (no-op).
- `Entity/` — empty except a `.gitignore` placeholder. No Doctrine entities exist in this scope.
- `Repository/` — empty except a `.gitignore` placeholder. No repositories exist in this scope.
- `Story/` — `AppStory.php`, a `zenstruck/foundry` `Story` (`#[AsFixture(name: 'main')]`) whose `build()` body is commented-out example code only (no-op).
- `Kernel.php` — see `ARCHITECTURE.md`.

## `config/`

- `bundles.php` — bundle registration per environment (`all`/`dev`/`test`). 14 bundles registered: `FrameworkBundle`, `DoctrineBundle`, `DoctrineMigrationsBundle`, `DebugBundle` (dev), `TwigBundle`, `WebProfilerBundle` (dev, test), `StimulusBundle`, `TurboBundle`, `TwigExtraBundle`, `SecurityBundle`, `MonologBundle`, `MakerBundle` (dev), `DoctrineFixturesBundle` (dev, test), `ZenstruckFoundryBundle` (dev, test).
- `packages/` — one YAML file per bundle: `asset_mapper.yaml`, `cache.yaml`, `csrf.yaml`, `debug.yaml`, `doctrine.yaml`, `doctrine_migrations.yaml`, `framework.yaml`, `mailer.yaml`, `messenger.yaml`, `monolog.yaml`, `notifier.yaml`, `property_info.yaml`, `routing.yaml`, `security.yaml`, `translation.yaml`, `twig.yaml`, `ux_turbo.yaml`, `validator.yaml`, `web_profiler.yaml`, `zenstruck_foundry.yaml`. Environment-specific overrides live inside each file under `when@dev`/`when@test`/`when@prod` blocks rather than as separate files. See `INTEGRATIONS.md` and `CONVENTIONS.md` for the content of the ones that matter most (`doctrine.yaml`, `messenger.yaml`, `security.yaml`, `csrf.yaml`/`ux_turbo.yaml`, `routing.yaml`).
- `routes.yaml` + `routes/framework.yaml`, `routes/security.yaml`, `routes/web_profiler.yaml` — see `ARCHITECTURE.md`.
- `services.yaml` — default `autowire`/`autoconfigure: true`, one resource covering all of `App\` → `../src/`. No explicit service definitions beyond the defaults in this scope.
- `preload.php` — opcache class preloading, conditional on a compiled prod container existing; not exercised outside a prod build.
- `reference.php` — a large (1600+ line) auto-generated docblock-only reference of every bundle's config array-shape (IDE support: "This file is auto-generated and is for apps only"). Not consulted by the application at runtime; only partially read for this map since it is Symfony-generated boilerplate rather than project-specific configuration (see `CONCERNS.md`, "Not fully read").
- `tenancy/trainer_scoped_tables.txt` — the Row-Level-Security manifest; see `ARCHITECTURE.md` and `CONVENTIONS.md`.

## `migrations/`

`Version20260809120000.php` (namespace `DoctrineMigrations`, deliberately excluded from the app's own PSR-4 autoload — see `Task/app/config/packages/doctrine_migrations.yaml`'s comment). Empty `.gitignore` placeholder alongside it. See `ARCHITECTURE.md` for its content.

## `templates/`

- `base.html.twig` — root layout: doctype/head/body shell, a `.skip-link`, the `importmap('app')` call, and the inline runtime-branding `<style>` block (`{% block branding %}`). See `CONVENTIONS.md` for the CSS token layers this defines.
- `home/index.html.twig` — extends `base.html.twig`; the walking-skeleton placeholder page rendered by `HomeController`.

## `assets/` — AssetMapper-managed, no Node build step

- `app.js` — entrypoint (registered via `Task/app/importmap.php`); imports `./stimulus_bootstrap.js` then `./styles/app.css`.
- `stimulus_bootstrap.js` — framework-provided; autoloads `controllers/*_controller.js` by filename convention.
- `controllers/hello_controller.js` — the unmodified Symfony UX Stimulus example controller (not yet removed; see `CONCERNS.md`).
- `controllers/csrf_protection_controller.js` — the stock Symfony CSRF double-submit Stimulus controller (framework recipe file, not project-specific code).
- `controllers.json` — registers the `@symfony/ux-turbo` controller package (`turbo-core` enabled; `mercure-turbo-stream` disabled).
- `images/favicon.svg`.
- `styles/app.css` — the static design-token layer; see `CONVENTIONS.md`.
- `vendor/` — AssetMapper-downloaded JS vendor files (`@hotwired/stimulus`, `@hotwired/turbo`, `installed.php`). Git-ignored build output (`Task/app/.gitignore`: `/assets/vendor/`), analogous to Composer's own `vendor/`; not further mapped here.

## `tests/`

- `bootstrap.php` — boots `Symfony\Component\Dotenv\Dotenv` against `.env`.
- `Smoke/HealthSmokeTest.php` — the only test class in this scope. See `TESTING.md`.

## `docker/`

- `nginx/default.conf` — see `ARCHITECTURE.md`.
- `php/Dockerfile`, `php/docker-entrypoint.sh`, `php/fpm-pool.conf`, `php/healthcheck.sh`, `php/php.ini` — see `ARCHITECTURE.md` and `STACK.md`.
- `postgres/grant-roles.sh`, `postgres/initdb/00-roles.sh` — see `INTEGRATIONS.md`.

## `public/`

`index.php` only — the sole HTTP front controller (see `ARCHITECTURE.md`). `public/uploads/` is a runtime, volume-mounted directory (`compose.yaml`'s `uploads` volume) and is not a tracked path in this scope.

## `translations/`

Empty except a `.gitignore` placeholder. No translation catalogs exist in this scope, despite `config/packages/translation.yaml` configuring a `default_path` to this directory.

## `bin/`

`console` (Symfony Console entry point, boots `App\Kernel`), `phpunit` (thin shim requiring the Composer-vendored PHPUnit binary).

## Root files

`composer.json`/`composer.lock`, `compose.yaml` (see `ARCHITECTURE.md`), `Makefile` (see below), `phpunit.dist.xml`, `phpstan.dist.neon`, `symfony.lock`, `importmap.php`, `env.example` (committed environment template), `.env`/`.env.dev`/`.env.test` (present but not opened by this map — existence only, see `CONVENTIONS.md`), `README.md` (the most complete single hand-written description of this stack; this map cites it throughout but does not merely restate it), `.editorconfig`, `.gitignore`.

## The Makefile — how everything is run

`Task/app/Makefile`'s own header: "Every target runs inside the Compose containers. Nothing here requires PHP, Composer, Node or PostgreSQL on the host." All PHP-executing targets shell into the `php` container via `docker compose exec -T php`.

| Target | Does |
|---|---|
| `help` (default) | Lists all targets with their `##` descriptions |
| `setup` | Copies `env.example` → `.env` if `.env` does not already exist |
| `env-reset` | Overwrites `.env` from `env.example` unconditionally, discarding local edits |
| `build` | `setup`, then `docker compose build` |
| `up` | `setup`; starts `db php web mail` and waits healthy; runs `migrate`; starts `worker` and waits healthy; runs `health` |
| `down` | `docker compose down` (keeps volumes) |
| `destroy` | `docker compose down -v` (deletes all volumes, including the database) |
| `ps` | `docker compose ps` |
| `logs` | `docker compose logs -f` |
| `shell` | Opens a shell in the `php` container |
| `install` | `composer install --no-interaction` inside `php` |
| `migrate` | Runs `doctrine:migrations:migrate` with `DATABASE_URL` swapped for `$$MIGRATION_DATABASE_URL` (owner role) |
| `migration` | Runs `doctrine:migrations:diff` with the same owner-DSN swap |
| `seed` | `doctrine:fixtures:load --no-interaction` (loads `AppFixtures`/`AppStory`, both currently no-ops — see `CONCERNS.md`) |
| `reset` | `destroy`, then `up`, then `seed` |
| `test-db` | Creates `practiceperfect_test` as owner, grants it via `docker/postgres/grant-roles.sh`, migrates it as owner |
| `test` | `test-db`, then `vendor/bin/phpunit` inside `php` |
| `lint` | `lint:container`, `lint:yaml config`, `lint:twig templates`, `composer validate --strict`, `vendor/bin/phpstan analyse` |
| `smoke` | `curl`s the running stack's `/health` over HTTP — a black-box check, distinct from the PHPUnit smoke test class of a similar name |
| `health` | `docker compose ps --format 'table {{.Service}}\t{{.Status}}'` |

## Where new code belongs (inferred from what exists, not from the unbuilt spec)

- New HTTP endpoints: invokable controllers under `src/Controller/`, following `HealthController`'s shape (constructor-injected dependencies, `#[Route]` attribute, single `__invoke()` action) — the only pattern this scope has two data points for.
- New Doctrine entities: `src/Entity/`, picked up automatically by the single attribute-driven `App` mapping in `config/packages/doctrine.yaml`. That mapping's own comment anticipates a future per-module layout (`src/Identity/Entity/`, etc. — see `ARCHITECTURE.md`, "Designed but not built"), but no module subdirectory exists yet, so today the flat `src/Entity/` is what is actually wired.
- New migrations: generated into `migrations/` via `make migration`; any migration creating a trainer-scoped table must add that table to `config/tenancy/trainer_scoped_tables.txt` in the same commit (see `CONVENTIONS.md`).
- New views: `templates/`, extending `base.html.twig`.
- New Stimulus controllers: `assets/controllers/`, auto-registered by filename via `stimulus_bootstrap.js`.
