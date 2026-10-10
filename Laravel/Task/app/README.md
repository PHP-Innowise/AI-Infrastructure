# PracticePerfect — Laravel edition

A white-label, multi-tenant platform for sports trainers, their coaches, and the
players and parents they work with.

Docker Compose is the only supported way to run this application. Nothing here
requires PHP, Composer, Node or PostgreSQL on your machine — Docker is the sole
prerequisite. Every documented command runs inside a container, including the
Vite asset build.

- Requirements are derived in [`../../specs/`](../../specs) from the read-only
  client material in [`../Epics/`](../Epics). Those epics are never edited.
- The platform design, including the tenancy model this stack is shaped around,
  is in [`specs/architect-architecture.md`](../project-work/specs/architect-architecture.md),
  with the seam resolutions in
  [`specs/architect-reconciliation.md`](../project-work/specs/architect-reconciliation.md).

---

## Quick start

```bash
make up
```

That creates `.env` from `env.example` if missing, starts the stack, installs
dependencies, generates an application key, applies migrations, then starts the
queue worker and waits for every service to report healthy.

| What | Where |
|---|---|
| Application | http://localhost:8091 |
| Health endpoint | http://localhost:8091/health |
| Mail catcher (Mailpit) | http://localhost:8126 |
| PostgreSQL | `localhost:15434` |

Host ports are configurable in `.env`. The defaults avoid 8080, 8025 and 5432,
which another local stack usually already owns.

---

## Commands

Run `make help` for the full list.

| Command | Does |
|---|---|
| `make up` | Start everything, install, migrate, wait for healthy |
| `make down` | Stop the stack, keep the data |
| `make destroy` | Stop the stack and delete the database volume |
| `make reset` | `destroy`, `up`, then `seed` — a clean database from nothing |
| `make migrate` | Apply migrations (as the owner role) |
| `make seed` | Load seeders, including a login for each of the four MVP roles |
| `make test` | Run the Pest suite |
| `make lint` | Pint and an environment report |
| `make assets` | Rebuild the Vite bundle inside the image |
| `make smoke` | Prove the running stack answers over HTTP |
| `make shell` | Shell inside the php container |
| `make env-reset` | Overwrite `.env` from `env.example` |

---

## Services

| Service | Image | Purpose |
|---|---|---|
| `php` | built from `docker/php/Dockerfile` | php-fpm 8.4, serves the application |
| `web` | `nginx:1.27-alpine` | Serves `public/`, proxies PHP to `php` |
| `db` | `postgres:17-alpine` | Domain schema, queue, cache, sessions |
| `worker` | same image as `php` | `queue:work database`, and scheduled work |
| `mail` | `axllent/mailpit` | Catches all outbound mail; nothing leaves the stack |

There is deliberately no Redis: queue, cache and sessions all use the database
driver. There is deliberately no host cron: scheduled work belongs to the
worker, because a Compose-only constraint forbids host dependencies. Vite runs
in a Docker build stage, so there is no host Node either.

---

## The part worth understanding before you change anything

This product's worst possible defect is one trainer seeing another trainer's
data — a leak would disclose minors' medical and financial-aid flags. Two of the
enforcement layers live here rather than in PHP.

**Three database roles, not one.** `docker/postgres/initdb/00-roles.sh` creates
them on first boot:

| Role | Holds | Used by |
|---|---|---|
| `pp_owner` | Owns the schema, all DDL | Migrations and seeders only. Never the running app |
| `pp_app` | DML only. `NOBYPASSRLS`, no `CREATE` | php-fpm and the queue worker |
| `pp_crossing` | `SELECT` only, `BYPASSRLS` | The cross-tenant read service, which returns arrays rather than models |

That split is why `make migrate` and `make seed` substitute the owner
credentials for the duration of the command.

**A startup gate that refuses to boot.** Row-Level Security has one failure mode
it cannot detect itself: if the application connects as the table owner or as a
role holding `BYPASSRLS`, every policy is silently skipped.
`docker/php/docker-entrypoint.sh` checks for exactly that before php-fpm accepts
a request. It also verifies both directions of the tenancy manifests:

- every table in `config/tenancy/trainer_scoped_tables.txt` **has** RLS enabled
  with at least one policy;
- every table in `config/tenancy/resolver_global_tables.txt` **has not** — the
  resolver reads those before a tenant exists, so a policy there would make
  every login silently resolve nothing.

**Every migration that adds a trainer-scoped table must add it to
`config/tenancy/trainer_scoped_tables.txt` in the same commit.** The list is
pre-populated from the settled schema, so the failure is inverted: the migration
that creates the table must give it RLS, or the container will not boot.

`/health` re-asserts the same property at runtime, so a misconfigured deployment
shows up as an unhealthy container rather than as quiet cross-tenant reads.

---

## Configuration

`env.example` is committed; `.env` is generated from it and git-ignored.

> **Note on the filename.** The convention would be `.env.example`. This
> repository's `.claude/settings.json` denies writes matching `.env.*`, so the
> template is committed without the leading dot.

Database, mail, queue, cache and session settings are **not** in `.env`.
`compose.yaml` composes them from the role credentials, so which role the
application connects as is decided in exactly one place. Compose `environment`
beats `env_file`, so setting them in `.env` would have no effect.

Never commit a real secret. Stripe keys are blank in the template.

---

## Testing

```bash
make test
```

Pest 3, including `pest-arch` — the architecture tests are how several module
boundaries are actually enforced, rather than by packaging.
