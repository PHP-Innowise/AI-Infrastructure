# PracticePerfect

A white-label, multi-tenant platform for sports trainers, their coaches, and the
players and parents they work with.

Docker Compose is the only supported way to run this application. Nothing here
requires PHP, Composer, Node or PostgreSQL on your machine — Docker is the sole
prerequisite. Every documented command runs inside a container.

- Requirements are derived in [`../../specs/`](../../specs) from the read-only
  client material in [`../Epics/`](../Epics). Those epics are never edited.
- The platform design, including the tenancy model this stack is shaped around,
  is in [`specs/architect-architecture.md`](../../specs/architect-architecture.md).

---

## Quick start

```bash
make up
```

That creates `.env` from `env.example` if it is missing, starts the stack, waits
for every service to report healthy, applies migrations, and then starts the
worker. From a clean clone it is the only command you need.

Then:

| What | Where |
|---|---|
| Application | http://localhost:8080 |
| Health endpoint | http://localhost:8080/health |
| Mail catcher (Mailpit) | http://localhost:8125 |
| PostgreSQL | `localhost:15433` |

Host ports are configurable in `.env`. The defaults deliberately avoid 5432 and
8025, which another local stack usually already owns.

---

## Commands

Run `make help` for the full list.

| Command | Does |
|---|---|
| `make up` | Start everything, migrate, wait for healthy |
| `make down` | Stop the stack, keep the data |
| `make destroy` | Stop the stack and delete the database volume |
| `make reset` | `destroy`, then `up`, then `seed` — a clean database from nothing |
| `make migrate` | Apply Doctrine migrations (as the owner role) |
| `make migration` | Generate a migration from mapping changes |
| `make seed` | Load fixtures, including a login for each of the four MVP roles |
| `make test` | Create/migrate the test database, then run PHPUnit |
| `make lint` | Container, YAML, Twig, Composer and PHPStan checks |
| `make smoke` | Prove the running stack answers over HTTP |
| `make shell` | Shell inside the php container |
| `make logs` | Follow logs for every service |
| `make env-reset` | Overwrite `.env` from `env.example` |

---

## Services

| Service | Image | Purpose |
|---|---|---|
| `php` | built from `docker/php/Dockerfile` | php-fpm 8.4, serves the application |
| `web` | `nginx:1.27-alpine` | Serves `public/`, proxies PHP to `php` |
| `db` | `postgres:17-alpine` | Domain schema, Messenger transport, sessions, cache, locks |
| `worker` | same image as `php` | `messenger:consume`, and the Scheduler once schedules exist |
| `mail` | `axllent/mailpit` | Catches all outbound mail; nothing leaves the stack |

There is deliberately no Redis: sessions, the cache pool and the lock store all
sit on the PostgreSQL that is already there. There is deliberately no host cron:
scheduled work belongs to the worker, because a Compose-only constraint forbids
host dependencies.

---

## The part worth understanding before you change anything

This product's worst possible defect is one trainer seeing another trainer's
data — a leak would disclose minors' medical and financial-aid flags. The
architecture answers that with five independent layers, and two of them are
visible in this directory rather than in PHP:

**Three database roles, not one.** `docker/postgres/initdb/00-roles.sh` creates
them on first boot:

| Role | Holds | Used by |
|---|---|---|
| `pp_owner` | Owns the schema, all DDL | Migrations only. Never the running app |
| `pp_app` | DML only. `NOBYPASSRLS`, no `CREATE` | php-fpm and the worker |
| `pp_crossing` | `SELECT` only, `BYPASSRLS` | The cross-tenant read service, which returns arrays and has no EntityManager |

That separation is why `make migrate` swaps `DATABASE_URL` for the owner DSN for
the duration of the command, and why the Messenger transport table is created by
a migration instead of by `auto_setup`.

**A startup gate that refuses to boot.** Row-Level Security has one failure mode
it cannot detect itself: if the application connects as the table owner or as a
role holding `BYPASSRLS`, every policy is silently skipped and nothing raises.
`docker/php/docker-entrypoint.sh` checks for exactly that before php-fpm accepts
a single request, and refuses to start otherwise. It also verifies that every
table listed in `config/tenancy/trainer_scoped_tables.txt` actually has RLS
enabled with at least one policy.

**Every migration that adds a trainer-scoped table must add it to
`config/tenancy/trainer_scoped_tables.txt` in the same commit.** A table carrying
the tenant key but missing from that list is a defect, and the gate is what turns
it into a boot failure instead of a leak discovered months later.

`/health` re-asserts the same property at runtime, so a misconfigured deployment
shows up as an unhealthy container rather than as quiet cross-tenant reads.

---

## Configuration

`env.example` is committed; `.env` is generated from it and git-ignored.

> **Note on the filename.** The convention would be `.env.example`. This
> repository's `.claude/settings.json` denies writes matching `.env.*`, so the
> template is committed without the leading dot.

Connection strings are **not** set in `.env`. `compose.yaml` composes them from
the role credentials, so which role the application connects as is decided in
exactly one place. Compose `environment` beats `env_file`, and Symfony's Dotenv
never overrides a real environment variable, so a Flex recipe appending its own
`DATABASE_URL` to your `.env` is ignored rather than silently repointing the app.

The flip side: `make env-reset` discards recipe-added variables too. When
`composer require` adds a new variable to `.env`, mirror it into `env.example`
(or into `compose.yaml`'s `x-app-environment` if it is a property of the stack
rather than of your machine).

Never commit a real secret. Stripe keys are blank in the template.

---

## Testing

```bash
make test
```

The test database is a separate database (`practiceperfect_test`), created and
migrated by the owner role and then granted to the application role — PostgreSQL
default privileges are per-database, so the initdb bootstrap does not reach it.

`phpunit.dist.xml` sets `APP_ENV` as both an `env` and a `server` variable. That
is not redundant: Symfony's `KernelTestCase` resolves `$_ENV` before `$_SERVER`,
and this application runs in a container that exports a real `APP_ENV=dev`, so
setting only `$_SERVER` leaves the suite booting the dev kernel.

PHPStan runs at level 8.
