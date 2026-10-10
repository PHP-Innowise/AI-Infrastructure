---
description: Naming, migration, tenancy-manifest, and two-layer CSS token conventions observed in the PracticePerfect walking skeleton in Task/app/
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

# Conventions

This scope has very little hand-written code to generalize from (two controllers, one migration, one CSS file, a handful of Twig/JS files). Where a pattern below rests on only one or two examples, that is noted rather than presented as an established rule.

## PHP

- Autoload: PSR-4, `App\` → `src/`, `App\Tests\` → `tests/` (`composer.json`).
- `declare(strict_types=1)` appears in the hand-written classes — `src/Controller/HealthController.php`, `src/Controller/HomeController.php`, `migrations/Version20260809120000.php` — but not in the Flex-recipe-generated scaffolds (`src/Kernel.php`, `src/DataFixtures/AppFixtures.php`, `src/Story/AppStory.php`). Observed split, not a confirmed project-wide rule given the sample size.
- Controllers: `final class`, one `#[Route(...)]`-attributed `__invoke()` method per class (both `HomeController` and `HealthController` follow this invokable-controller shape).
- Doctrine ORM mapping: one `App`-prefixed, attribute-driven mapping over all of `src/` rather than per-directory mappings (`config/packages/doctrine.yaml`); underscore naming strategy; PostgreSQL `identity` generation preference. No entities exist yet to observe a per-entity convention from.

## Migrations

- Class name `Version<YYYYMMDDHHMMSS>` extending `Doctrine\Migrations\AbstractMigration`, namespace `DoctrineMigrations` (`config/packages/doctrine_migrations.yaml`). One example in scope: `migrations/Version20260809120000.php`.
- That example asserts the DB platform via `$this->abortIf(!... instanceof PostgreSQLPlatform, ...)` before doing anything else, and writes raw SQL via `addSql()` rather than building up the `Schema` object parameter.
- It documents, in its own class docblock, whether the table(s) it creates are trainer-scoped, and cites `specs/architect-architecture.md` by section name where relevant.

## The trainer-scoped table rule (binding, cross-cutting)

`config/tenancy/trainer_scoped_tables.txt` is a plain-text manifest: one table name per line, blank lines and `#`-prefixed lines ignored. Its own header states the rule verbatim:

> every migration that creates a trainer-scoped table must add the table here in the same commit. A table carrying the denormalized tenant key and missing from this list is a defect, not an omission.

This is not a style preference — `docker/php/docker-entrypoint.sh` reads this exact file at container startup (Gate 4) and refuses to boot the `php`/`worker` services if a listed table lacks Row-Level Security or has zero policies attached (see `ARCHITECTURE.md`). The file also names the table categories that must **not** be listed: accounts, the trainer registry, branding, feature toggles, account-trainer links, camp form submissions before conversion, and the audit log. At the mapped commit the file has zero table entries.

Practical consequence for anyone adding a migration in this codebase: a trainer-scoped `CREATE TABLE` and its `trainer_scoped_tables.txt` entry are one atomic unit of work, not two separate steps.

## CSS — two token layers

`assets/styles/app.css`'s own header comment names the split explicitly; this is the most load-bearing frontend convention in this scope.

1. **Static layer** — `assets/styles/app.css`. Compiled once by AssetMapper, identical for every trainer. Declares:
   - Typography: `--font-family-base`, `--text-{hero-title,section-title,block-title,card-title,body-lg,body,caption,eyebrow}-{size,line,weight}`, `--text-eyebrow-tracking`.
   - Grayscale + semantic colors: `--gray-{50,200,400,600,800,900}`, `--color-{surface,surface-sunken,border,border-strong,text-primary,text-secondary,text-disabled,error,error-soft}`.
   - Spacing: `--space-{xxs,xs,sm,md,lg,xl,xxl}`. Radius: `--radius-{xs,sm,md,lg,xl,pill}`.
   - Shadows: `--shadow-{card-soft,card-strong,button-primary,button-primary-hover}` — the two button shadows read `var(--brand-primary-rgb)` from the runtime layer below; this resolves correctly because CSS custom properties resolve at compute time against whatever is cascaded onto `:root`, not at declaration time, so the ordering relative to the runtime `<style>` block in `<head>` is safe (the file's own comment makes this point explicitly).
   - Component geometry: `--control-*`, `--button-*`.
   - Motion: `--ease-standard`, `--duration-{fast,standard}`, both zeroed under `@media (prefers-reduced-motion: reduce)`.
   - The file's own comment states the rule for this layer: **`--brand-*` must never be declared here.**

2. **Runtime layer** — an inline `<style>` block inside `{% block branding %}` in `templates/base.html.twig`, emitted fresh on every request rather than compiled at build time. Exactly five custom properties are defined there, and the surrounding Twig comment states these are **the only runtime-overridable custom properties in the system**:
   - `--brand-primary`
   - `--brand-primary-soft`
   - `--brand-primary-deep`
   - `--brand-primary-rgb` (a bare `r, g, b` triple, composed into `rgba(var(--brand-primary-rgb), <alpha>)` at call sites, not pre-wrapped in `rgb()`)
   - `--brand-on-primary`

   At the mapped commit the block hardcodes platform-default values (a green palette: `--brand-primary: #00B300`, etc.). The same Twig comment states Epic-01 replaces the block's contents with `BrandingProvider` output resolved per active trainer, and that nothing in `app.css` may reference a trainer's raw hex color except through these five names.

Naming convention across both layers: kebab-case custom properties grouped by prefix (`--text-`, `--color-`, `--space-`, `--radius-`, `--shadow-`, `--control-`, `--button-`, `--brand-`).

## Accessibility conventions already encoded in this scope

- `.skip-link` (`assets/styles/app.css`, used in `templates/base.html.twig`) is off-screen-until-focused (`left: -9999px` → `left: var(--space-xs)` on `:focus-visible`), not `display:none`, so it stays in the tab order — cited by the CSS comment to WCAG 2.2 SC 2.4.1.
- A global `:focus-visible` outline plus `--focus-ring`/`--focus-ring-error` — cited to WCAG 2.2 SC 2.4.11. `:focus-visible` specifically (not `:focus`), so pointer users don't see a ring on click.
- `--control-hit-checkbox: 24px` deliberately larger than `--control-size-checkbox: 14px` (the drawn box) — cited to WCAG 2.2 SC 2.5.8.

## JavaScript / Stimulus

- Default-exported class extending `Controller`; filename `<name>_controller.js` maps to `data-controller="<name>"`, auto-registered by `stimulus_bootstrap.js` — the standard `symfony/stimulus-bundle` convention, not project-specific.
- CSRF protection configuration spans two files: `config/packages/csrf.yaml` sets `form.csrf_protection.token_id: submit` and `csrf_protection.stateless_token_ids: [submit, authenticate, logout]`; `config/packages/ux_turbo.yaml` additionally sets `csrf_protection.check_header: true`, which is what makes the stateless CSRF check honor the header `assets/controllers/csrf_protection_controller.js` sends alongside Turbo's fetch-based form submissions.

## Configuration files

- One YAML file per bundle under `config/packages/`, named after the bundle — Symfony Flex's standard per-recipe convention.
- Environment-specific overrides use `when@dev`/`when@test`/`when@prod` blocks within the same file, not separate per-environment files.

## Environment / secrets

- Connection strings and role credentials are composed exactly once, in `compose.yaml`'s `x-app-environment` anchor, rather than restated in `.env` — see `INTEGRATIONS.md` for why.
- `env.example` is the committed, non-secret template (dev-only placeholder credentials; blank Stripe keys). `.env`/`.env.dev`/`.env.test` exist in this scope but are **not opened by this map** — recorded by existence only, per mapping safety rules (`.env` and `.env.*` are never read). `env.example` itself is not secret — it is explicitly documented, in its own header and in `Task/app/README.md`, as containing only throwaway local-only values; its filename lacks the leading dot specifically because this repository's `.claude/settings.json` denies writes matching `.env.*`.
