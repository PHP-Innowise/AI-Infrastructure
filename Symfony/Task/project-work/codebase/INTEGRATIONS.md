---
description: PostgreSQL (three-role tenancy DSNs), Doctrine Messenger, Mailpit mail, and notifier integrations for the PracticePerfect walking skeleton in Task/app/
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

# Integrations

## PostgreSQL — the only datastore

`Task/app/compose.yaml`'s `db` service (`postgres:17-alpine`) backs the domain schema, the Messenger transport, HTTP sessions, the cache pool, and the lock store. There is no Redis and no second database in this scope (`Task/app/README.md`; `Task/app/compose.yaml` service comments).

### Three roles, three DSNs

Row-Level Security is only as strong as the privilege separation under it, so the stack provisions three distinct PostgreSQL roles instead of one shared connection. Roles are created once, on first init of the `db-data` volume, by `Task/app/docker/postgres/initdb/00-roles.sh`:

| Role (default name) | Privileges | DSN env var | Used by |
|---|---|---|---|
| owner (`pp_owner`, `POSTGRES_OWNER_USER`) | Owns the schema; all DDL | `MIGRATION_DATABASE_URL` | Migrations only (`make migrate`), never the running app |
| app (`pp_app`, `APP_DB_USER`) | `NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOINHERIT`; DML only, `CREATE` revoked on schema `public`; DML granted via `ALTER DEFAULT PRIVILEGES` | `DATABASE_URL` | `php` and `worker` services — the running application |
| crossing (`pp_crossing`, `CROSSING_DB_USER`) | `NOSUPERUSER NOCREATEDB NOCREATEROLE BYPASSRLS NOINHERIT`; `SELECT` only | `CROSSING_DATABASE_URL` | Reserved for a cross-tenant read service — **no code in this scope connects through it**; `grep -ri crossing Task/app/src/` returns nothing |

All three DSNs are composed in exactly one place — `Task/app/compose.yaml`'s `x-app-environment` YAML anchor — from the role credential variables, specifically so, per the file's own comment, "which role does the application connect as" is decided in exactly one place. `Task/app/env.example` deliberately does **not** set `DATABASE_URL`, `MIGRATION_DATABASE_URL`, `CROSSING_DATABASE_URL`, `MAILER_DSN`, or `MESSENGER_TRANSPORT_DSN`: Compose `environment` overrides `env_file`, and Symfony's Dotenv never overrides a real environment variable, so anything a Flex recipe appends to `.env` under these keys is inert.

### Why migrations run under a different DSN than the application

The app role holds no `CREATE` on schema `public` (`docker/postgres/initdb/00-roles.sh`), so it is structurally incapable of DDL — an app role that could alter its own tables could also disable a Row-Level Security policy, which is exactly what the separation prevents. `Task/app/Makefile`'s `migrate` target swaps `DATABASE_URL` for `$$MIGRATION_DATABASE_URL` for the duration of one command only:

```
migrate: ## Apply Doctrine migrations (runs as the owner role)
	$(PHP) sh -c 'DATABASE_URL="$$MIGRATION_DATABASE_URL" php bin/console doctrine:migrations:migrate --no-interaction --allow-no-migration'
```

Because `ALTER DEFAULT PRIVILEGES FOR ROLE <owner>` is set during initdb, anything the owner creates afterward in schema `public` is already reachable by the app role without a further per-table grant.

`Task/app/docker/postgres/grant-roles.sh` re-applies the same app/crossing grants to any database created **after** initdb — most importantly the test database — because PostgreSQL default privileges are per-database and the initdb bootstrap only covers the primary database. `Task/app/Makefile`'s `test-db` target invokes it.

### Doctrine configuration

`Task/app/config/packages/doctrine.yaml`: `dbal.url` resolves `%env(resolve:DATABASE_URL)%` — the running app's DBAL/ORM connection is always the `app`-role DSN in this scope, never owner or crossing. ORM mapping is a single `App`-prefixed, attribute-driven mapping over all of `src/` (not per-module yet — see `ARCHITECTURE.md`, "Designed but not built"), underscore naming strategy, PostgreSQL `identity` generation preference. Test environment appends a `dbname_suffix` for ParaTest parallelism.

`Task/app/config/packages/doctrine_migrations.yaml`: migrations live under the `DoctrineMigrations` namespace, mapped to `Task/app/migrations/`, deliberately excluded from the app's own PSR-4 autoload.

## Messenger — Doctrine DBAL transport, no broker

`Task/app/config/packages/messenger.yaml` configures two transports sharing one PostgreSQL table:

- `async` — DSN `%env(MESSENGER_TRANSPORT_DSN)%` (`doctrine://default?auto_setup=0` in `compose.yaml`), retry strategy `max_retries: 3`, `multiplier: 2`. Routes `Symfony\Component\Mailer\Messenger\SendEmailMessage`, `Symfony\Component\Notifier\Message\ChatMessage`, and `Symfony\Component\Notifier\Message\SmsMessage`.
- `failed` — `doctrine://default?queue_name=failed&auto_setup=0`, configured as `failure_transport`.

`auto_setup` is off on both because the app role holds no DDL; the shared `messenger_messages` table is instead created explicitly by `Task/app/migrations/Version20260809120000.php`, which also adds a `notify_messenger_messages()` PL/pgSQL trigger function and a `notify_trigger` (`AFTER INSERT OR UPDATE`) that calls `pg_notify('messenger_messages', NEW.queue_name)`, so the worker reacts via LISTEN/NOTIFY instead of polling.

The `worker` service in `compose.yaml` runs `command: ["worker"]`, which `Task/app/docker/php/docker-entrypoint.sh` turns into `messenger:consume async --time-limit=3600 --memory-limit=256M`. The entrypoint waits for the `messenger_messages` table to exist before starting the worker rather than crash-looping, logging `"waiting for messenger_messages: run 'make migrate' to create it"`.

## Mail — Mailpit, nothing leaves the stack

`Task/app/compose.yaml`'s `mail` service (`axllent/mailpit:v1.21`) is an SMTP catcher; `MAILER_DSN` is fixed to `smtp://mail:1025`. `Task/app/config/packages/mailer.yaml` reads `%env(MAILER_DSN)%`. The Mailpit web UI is published on host port `MAIL_UI_PORT` (default `8125`), mapped to container port `8025`.

## Notifier

`Task/app/config/packages/notifier.yaml` routes every `channel_policy` level (`urgent`, `high`, `medium`, `low`) to `['email']` only; no `chatter_transports` or `texter_transports` are configured. `admin_recipients` is still the Flex-recipe placeholder `admin@example.com` (see `CONCERNS.md`).

## Routing / URL generation outside HTTP

`Task/app/config/packages/routing.yaml` sets `framework.router.default_uri` to `%env(DEFAULT_URI)%`, used to generate absolute URLs outside an HTTP request (worker-sent emails, console commands). `DEFAULT_URI` is set in `compose.yaml`'s `x-app-environment` to `http://localhost:${HTTP_PORT:-8080}`, with a comment noting it "must match the port the stack is published on." `strict_requirements` is relaxed to `null` under `when@prod`.

## Security / authentication

`Task/app/config/packages/security.yaml` — one in-memory user provider (`users_in_memory: { memory: null }`), one firewall (`main`, `lazy`, no authenticator configured), a `dev` firewall exempting `^/(_profiler|_wdt|assets|build)/`, and no `access_control` rules. This is Symfony Flex's unmodified security skeleton; no real authentication is wired in this scope.

CSRF protection configuration spans two files: `Task/app/config/packages/csrf.yaml` sets `form.csrf_protection.token_id: submit` and `csrf_protection.stateless_token_ids: [submit, authenticate, logout]`; `Task/app/config/packages/ux_turbo.yaml` additionally sets `csrf_protection.check_header: true`, which is what makes the stateless CSRF check honor the header that `Task/app/assets/controllers/csrf_protection_controller.js` sends alongside Turbo's fetch-based form submissions.

## Not integrated in this scope

- **Stripe**: `Task/app/env.example` reserves blank `STRIPE_PUBLISHABLE_KEY`/`STRIPE_SECRET_KEY`/`STRIPE_WEBHOOK_SECRET` variables with the comment "Leave blank until Epic-05." No Stripe SDK dependency and no Stripe-calling code exist in this scope.
- **File storage beyond the local volume**: `compose.yaml` mounts a named `uploads` volume at `/app/public/uploads` for the `php` and `worker` services, served read-only by `web` under `Task/app/docker/nginx/default.conf`'s `location /uploads/` block (`Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; sandbox`, `X-Content-Type-Options: nosniff` — guarding against a crafted SVG executing in-origin). No code under `Task/app/src/` writes to this path in this scope.
- **External HTTP APIs**: `symfony/http-client` is a dependency (see `STACK.md`) but unused by any class in `Task/app/src/` in this scope.
