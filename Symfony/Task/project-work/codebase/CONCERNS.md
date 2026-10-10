---
description: Placeholder code, unimplemented pieces, and the designed-vs-built module gap in the PracticePerfect walking skeleton in Task/app/
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

# Concerns

Everything below is either explicitly self-flagged by the code/docs in this scope, or a directly observed gap. This scope is a deliberately thin walking skeleton; most items here are expected, not surprising, and are recorded so a later session does not mistake "not yet built" for "broken."

## Designed but not built

`specs/architect-architecture.md` (Symfony edition root, outside this map's scope) describes an intended nine-module structure — Platform, Identity, Scheduling, Crm, Content, Billing, Growth, Administration, Forms. None of those module directories exist under `Task/app/src/` at the mapped commit; `src/` is flat (`Controller/`, `DataFixtures/`, `Entity/`, `Repository/`, `Story/`, `Kernel.php`). `Task/app/config/packages/doctrine.yaml`'s own mapping comment already anticipates the module shape, but the app is not there yet. This map does not treat the spec as verified or built — see `ARCHITECTURE.md`, "Designed but not built."

## Self-flagged placeholders

- `Task/app/src/Controller/HomeController.php` — docblock: "Placeholder landing page for the walking skeleton. Epic-01 replaces this."
- `Task/app/templates/base.html.twig`'s `{% block branding %}` — comment: "Epic-01 replaces this block's contents with BrandingProvider output resolved for the active trainer (and adds the CSP nonce, once the security headers land)."
- `Task/app/src/DataFixtures/AppFixtures.php` and `Task/app/src/Story/AppStory.php` are both no-op stubs — commented-out example code only, nothing actually persisted.

## Documentation/code mismatch

`Task/app/README.md`'s command table describes `make seed` as loading "fixtures, including a login for each of the four MVP roles." The fixture code it runs (`doctrine:fixtures:load`, which executes `AppFixtures::load()`) does not do this in the current scope — `AppFixtures.php` only calls `$manager->flush()` after commented-out example code, and `AppStory.php`'s `build()` is equally empty. At the mapped commit, `make seed` loads no data. This is a documentation/code gap worth closing before it misleads an operator, not a runtime defect.

## Provisioned but unused

The `crossing` PostgreSQL role (`pp_crossing`, `BYPASSRLS`, read-only) is fully provisioned at the infrastructure layer — created in `Task/app/docker/postgres/initdb/00-roles.sh`, granted in `Task/app/docker/postgres/grant-roles.sh`, and given its own `CROSSING_DATABASE_URL` in `Task/app/compose.yaml`. No PHP code in `Task/app/src/` connects through it in this scope (`grep -ri crossing Task/app/src/` returns nothing). The "cross-tenant read service" its surrounding comments describe (`Task/app/README.md`: "returns arrays and has no EntityManager") does not exist yet.

## No authentication/authorization in this scope

`Task/app/config/packages/security.yaml` has no authenticator, an in-memory-only user provider, and no `access_control` rules. Nothing in this scope restricts access to any route.

## Minor scaffold leftovers

- `Task/app/assets/controllers/hello_controller.js` is the unmodified Symfony UX Stimulus example controller.
- `Task/app/config/packages/notifier.yaml` still has the Flex-recipe default `admin_recipients: [{ email: admin@example.com }]`.

## Not fully read

`Task/app/config/reference.php` is a large (1600+ line) auto-generated docblock-only IDE reference covering every installed bundle's config array-shape. It was confirmed to be Symfony-generated boilerplate ("This file is auto-generated and is for apps only") and not consulted by the application at runtime, but was not read line-by-line beyond that confirmation. Recorded here as explicitly partial coverage rather than silently omitted.

## Things easy to silently get wrong later (self-documented by this codebase, not observed defects)

- **No persistent PDO connections, ever.** Both `Task/app/docker/php/docker-entrypoint.sh` (Gate 3) and a comment in `Task/app/docker/php/php.ini` call out that the tenancy design plans to put the active tenant in a PostgreSQL session variable, and a pooled/persistent connection would leak it across requests/tenants. Only the `DATABASE_URL`-persistent-flag case is actually enforced by the startup gate; a future connection pooler placed in front of PostgreSQL would not be caught by anything in this scope.
- **`env.example` vs `.env` drift.** `compose.yaml` composes all DSNs itself, so a Flex recipe appending its own `DATABASE_URL`-shaped variable to `.env` is silently inert at runtime. The inverse risk, documented in `env.example`'s own comment: if `composer require` adds a genuinely new variable to `.env` and it is not mirrored into `env.example` (or into `compose.yaml`'s `x-app-environment`), `make env-reset` discards it without warning.
- **The RLS table gate has nothing to check yet.** `config/tenancy/trainer_scoped_tables.txt` lists zero tables at the mapped commit, so Gate 4 in `docker-entrypoint.sh` is currently a no-op in practice. It activates the moment any migration creates a trainer-scoped table — the manifest update is easy to forget precisely because nothing has exercised the failure path yet.
