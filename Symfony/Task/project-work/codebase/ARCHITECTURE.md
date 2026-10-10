---
description: Five-service Docker Compose runtime, HTTP entry points, and the Row-Level-Security startup gate for the PracticePerfect walking skeleton in Task/app/
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

# Architecture

## Deployment shape: Docker Compose only

`Task/app/compose.yaml`'s own comment states Compose is "the only supported way to run this application: nothing here requires PHP, Composer, Node or PostgreSQL on the host." `Task/app/Makefile` and `Task/app/README.md` repeat the same constraint. Every operator action in this scope runs through `make` targets that shell into containers (`docker compose exec -T php ...`) — see `STRUCTURE.md` for the full Makefile target table.

### The five Compose services (`Task/app/compose.yaml`)

| Service | Image / build | Role |
|---|---|---|
| `db` | `postgres:17-alpine` | Domain schema, Messenger transport table, HTTP sessions, cache pool, lock store. Bootstraps the three tenancy roles on first init (see `INTEGRATIONS.md`). Host port default `15433` → `5432` — deliberately non-default to avoid colliding with another local Postgres. |
| `php` | built from `Task/app/docker/php/Dockerfile` (`php:8.4-fpm-alpine`) | Runs php-fpm, serves the application, connects to `db` as the `app` role. Healthcheck: `Task/app/docker/php/healthcheck.sh` (FastCGI `/ping`). |
| `web` | `nginx:1.27-alpine` | Serves `Task/app/public/` and proxies `*.php` to `php:9000` over FastCGI (`Task/app/docker/nginx/default.conf`). Host port default `8080` → `80`. |
| `worker` | same build as `php` | Runs `command: ["worker"]`, which `docker-entrypoint.sh` turns into `messenger:consume async --time-limit=3600 --memory-limit=256M`. Also the intended home for the Symfony Scheduler once a `Schedule` is defined (`docker-entrypoint.sh` comment: "Symfony Scheduler joins this command as `scheduler_default` once the first Schedule is defined") — no Schedule exists in this scope. |
| `mail` | `axllent/mailpit:v1.21` | SMTP catcher; no outbound mail leaves the stack. Host UI port default `8125` → `8025`. |

Named volumes: `db-data` (Postgres data directory), `app-var` (shared `Task/app/var/` between `php` and `worker`), `uploads` (shared `Task/app/public/uploads/` between `php`, `worker`, and read-only-mounted into `web`). `compose.yaml` deliberately keeps the `###> doctrine/doctrine-bundle ###` Flex recipe marker block empty, with a comment explaining this stops a future `composer require` from silently grafting on a second, differently-credentialed PostgreSQL service.

## HTTP entry point and routing

- `Task/app/public/index.php` is the sole front controller: it requires `vendor/autoload_runtime.php` (from `symfony/runtime`) and returns a closure constructing `App\Kernel`. `Task/app/docker/nginx/default.conf` enforces this at the web-server layer — one location block proxies `index.php` to php-fpm, and a later, more general block returns `404` for any other `\.php$` request.
- `Task/app/src/Kernel.php` is a bare `MicroKernelTrait` kernel with no overrides.
- `Task/app/config/routes.yaml` auto-imports every `#[Route]` attribute from controllers (`resource: routing.controllers`). `Task/app/config/routes/security.yaml` wires the `_security_logout` route. `Task/app/config/routes/framework.yaml` (dev-only `/_error`) and `Task/app/config/routes/web_profiler.yaml` (dev-only `/_wdt`, `/_profiler`) are environment-gated via `when@dev`.
- `Task/app/bin/console` boots the same `App\Kernel`, wrapped in a Symfony `Console\Application`.

### Controllers in this scope

- `App\Controller\HomeController` (`Task/app/src/Controller/HomeController.php`) — `GET /`, renders `Task/app/templates/home/index.html.twig`. Its own docblock names it a placeholder: "Epic-01 replaces this... It exists now only so the skeleton renders the layout and the design-token pipeline end to end rather than answering 404 at the root."
- `App\Controller\HealthController` (`Task/app/src/Controller/HealthController.php`) — `GET /health`, returns JSON. Runs two checks and reports `200`/`healthy` only if both pass, else `503`/`unhealthy`:
  1. `database` — `SELECT 1` over the injected `Doctrine\DBAL\Connection`.
  2. `tenancy_isolation` — queries `pg_roles` for the connection's own `rolbypassrls`/`rolsuper`; fails if either is true.

  Its docblock states this "asserts the property the whole tenancy model rests on... it is the one RLS failure that produces no error of its own," citing `specs/architect-architecture.md` § "The RLS disagreement, resolved". This is the per-request, runtime counterpart to the once-at-boot container startup gate below.

## Tenancy and Row-Level Security — the load-bearing property of this codebase

Per `Task/app/README.md`: "This product's worst possible defect is one trainer seeing another trainer's data." Two of the tenancy layers the README describes live in this scope's infrastructure rather than in PHP:

### 1. Three-role privilege separation

See `INTEGRATIONS.md` for the full owner/app/crossing breakdown and DSNs.

### 2. The container startup gate (`Task/app/docker/php/docker-entrypoint.sh`)

Runs before php-fpm (or the worker) accepts any work, for both the `php` and `worker` services (both use the same built image and the same entrypoint). In order:

1. Fixes ownership of `/app/var` and `/app/public/uploads` to `APP_UID:APP_GID`.
2. Waits for PostgreSQL to answer `pg_isready` as the app role, up to 60 attempts at 1/second; fails the container otherwise.
3. **Gate 1** — fails if `APP_DB_USER` equals `POSTGRES_OWNER_USER`: connecting as the schema owner would exempt the connection from its own Row-Level Security policies (an owner is exempt unless `FORCE ROW LEVEL SECURITY` is set).
4. **Gate 2** — queries `pg_roles` for the connecting role's `rolbypassrls`/`rolsuper`; fails unless both are `false`.
5. **Gate 3** — fails if `DATABASE_URL` contains `persistent=true` or `persistent=1`: a pooled/persistent connection would let one tenant's PostgreSQL session variable leak into another tenant's request. (The same constraint is independently asserted in a comment in `Task/app/docker/php/php.ini`.)
6. **Gate 4** — reads `Task/app/config/tenancy/trainer_scoped_tables.txt` line by line (blank/`#` lines skipped); for each named table that actually exists in the schema, checks that `pg_class.relrowsecurity` is true and at least one row exists in `pg_policy` for it; fails, listing every table that is missing RLS (`rls-off`) or has RLS enabled but zero policies (`no-policy`). This gate is **skipped**, not failed, when `doctrine_migration_versions` does not yet exist — i.e., before the first `make migrate` — logged as `"startup gate: schema not migrated yet, RLS table check deferred"`.
7. Only if every gate passes does it dispatch: `php-fpm --nodaemonize` for the default command, or (for the `worker` command) wait for the `messenger_messages` table to exist, then `exec messenger:consume async ...`.

A container that fails any gate exits with a `FATAL` log line and does not start; there is no partial or degraded startup.

### 3. `Task/app/config/tenancy/trainer_scoped_tables.txt` — the tenancy manifest

The file Gate 4 reads. One table name per line; `#` and blank lines ignored. Its own header states the rule:

> every migration that creates a trainer-scoped table must add the table here in the same commit. A table carrying the denormalized tenant key and missing from this list is a defect, not an omission.

It also states which tables must **not** be listed (global, not trainer-scoped): accounts, the trainer registry, branding, feature toggles, account-trainer links, camp form submissions before conversion, and the audit log.

At the mapped commit the file contains **zero table entries** — comments only — consistent with its own note: "Populated by the Epic-01 migration onward." No migration in this scope creates a trainer-scoped table (see below).

## Migrations

`Task/app/migrations/Version20260809120000.php` is the only migration in scope. It:

- Asserts the platform is PostgreSQL via `$this->abortIf(...)` ("PracticePerfect targets PostgreSQL only: Row-Level Security is load-bearing.").
- Creates `messenger_messages` (see `INTEGRATIONS.md`) plus three indexes and the `notify_trigger`/`notify_messenger_messages()` LISTEN/NOTIFY pair, all via raw `addSql()` calls rather than the `Schema` object parameter.
- States explicitly in its own docblock that this table "is NOT trainer-scoped and must not appear in `config/tenancy/trainer_scoped_tables.txt`," and explains why it exists as a migration rather than via Messenger's `auto_setup`: "the application role deliberately holds no DDL."

Operator flow: `make migration` runs `doctrine:migrations:diff` (owner DSN) to generate a new migration from mapping changes; `make migrate` applies pending migrations (owner DSN) — see `INTEGRATIONS.md` for the DSN-swap mechanism.

## Frontend rendering path

Request → `HomeController` (the only controller in this scope that renders HTML) → Twig: `Task/app/templates/home/index.html.twig` extends `Task/app/templates/base.html.twig` → `base.html.twig`'s `importmap('app')` call resolves `Task/app/importmap.php` and `Task/app/assets/app.js` → `assets/app.js` imports `stimulus_bootstrap.js` (autoloads `Task/app/assets/controllers/*_controller.js`) and `styles/app.css`. See `CONVENTIONS.md` for the two CSS token layers rendered along this path.

## Designed but not built

`specs/architect-architecture.md` (Symfony edition root, **outside this map's scope** — read here only for orientation, not verified against code in this run) describes an intended nine-module structure: **Platform, Identity, Scheduling, Crm, Content, Billing, Growth, Administration, Forms**. `Task/app/config/packages/doctrine.yaml`'s own mapping comment already anticipates this shape ("Entities live inside their owning module (`src/Identity/Entity`, `src/Scheduling/Entity`, ...), not in a single global `src/Entity`... See `specs/architect-architecture.md`, 'Module map'").

At the mapped commit, **none of those module directories exist**. `Task/app/src/` is flat: `Controller/`, `DataFixtures/`, `Entity/` (empty except a `.gitignore` placeholder), `Repository/` (same), `Story/`, and `Kernel.php`. This is the expected state of a walking skeleton before Epic-01, not a defect — but a later session should not assume any module namespace exists until it is independently verified in code.
