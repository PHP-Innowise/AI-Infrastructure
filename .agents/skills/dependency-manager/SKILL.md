---
name: dependency-manager
description: Manage Composer dependencies for Laravel projects: audit for vulnerabilities, review outdated packages (including Laravel version compatibility), tighten version constraints, optimize autoloading, and vet new packages before adding them. Triggers on "composer", "dependency", "update packages", "composer audit", "outdated", "add a library".
phase: execution
flow-next: verify
flow-alternatives: [security-reviewer, researcher, code-reviewer]
related: [researcher, security-reviewer, verify]
---

# Dependency Manager

## Overview

Keep the dependency tree healthy, secure, and reproducible. Dependencies are attack surface and maintenance cost, so every addition and update is a deliberate decision.

This branch targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release). For framework-agnostic native PHP, use the `main` branch instead.

## Core Commands

```bash
composer validate --strict     # composer.json/lock integrity
composer audit                 # known security advisories in the tree
composer outdated --direct     # direct deps with newer versions
composer show --tree           # inspect the dependency graph
composer why <package>         # why a package is installed
composer why-not <package> <version>  # what blocks an upgrade
php artisan about              # installed Laravel version, environment, and key package versions
```

## Health Check Workflow

1. **Integrity.** `composer validate --strict`; confirm `composer.lock` is committed and in sync.
2. **Security.** `composer audit`; triage each advisory (severity, whether the vulnerable path is used, fixed version available).
3. **Freshness.** `composer outdated --direct`; separate patch/minor (low risk) from major (needs review/changelog, especially a `laravel/framework` major bump).
4. **Laravel compatibility.** Check the `laravel/framework` constraint in `composer.json` and run `php artisan about` to see the resolved Laravel version plus first-party package versions (Sanctum, Horizon, Telescope, etc.) at a glance; confirm any package you touch declares support for the project's Laravel version.
5. **Footprint.** Look for redundant, abandoned, or overly heavy packages (`composer show --tree`).
6. **Autoload.** Ensure PSR-4 autoload is correct; for production builds recommend `composer dump-autoload -o` (or `--classmap-authoritative`).

## Updating Safely

- Prefer scoped updates: `composer update vendor/package --with-dependencies` over a blanket `composer update`.
- Read the changelog/UPGRADE notes for minor/major bumps.
- Run the full test suite and static analysis after any update; a green build is the acceptance gate.
- Commit `composer.json` and `composer.lock` together with a note on what changed and why.

## Version Constraints

- Use caret constraints (`^1.2`) for libraries following SemVer to allow safe minor/patch updates.
- Avoid `*` and unbounded `>=` constraints; they make builds non-reproducible.
- Pin only when necessary (a known incompatibility) and document the reason.
- Keep `require` (runtime) and `require-dev` (tooling: PHPUnit/Pest, PHPStan/Larastan, Laravel Pint, Rector) correctly separated.
- Check that third-party packages declare a `laravel/framework` compatibility range that includes the project's version before bumping; a package capped below the project's Laravel version will block `composer update`.

## Laravel Package Ecosystem

- **First-party (Laravel-maintained):** `laravel/sanctum` (first-party API/session token auth — default choice unless OAuth2 is explicitly required, in which case use `laravel/passport`), `laravel/horizon` (Redis queue dashboard/monitoring), `laravel/telescope` (local/staging debugging — never expose in production without gating), `laravel/scout` (search indexing), `laravel/cashier` (Stripe/Paddle billing — webhook handlers must be idempotent: verify the signature and de-duplicate on the event/webhook ID, since both providers retry delivery; a common security/correctness gap in billing integrations).
- **AI-agent tooling:** `laravel/boost` — an official first-party dev dependency that runs as an MCP server, giving AI coding agents structured, live access to a project's actual routes, Eloquent schema, config values, a Tinker REPL, recent log entries, and version-pinned Laravel documentation. Directly reduces hallucinated APIs for AI-agent workflows like this accelerator's. Suggested, not mandatory, for new Laravel projects using this accelerator (`composer require laravel/boost --dev`, then `php artisan boost:install`).
- **Popular third-party:** Spatie packages (`spatie/laravel-permission` for roles/permissions, `spatie/laravel-data` for DTOs, `spatie/laravel-medialibrary`, `spatie/laravel-query-builder`), `laravel/pint` (formatting, first-party but distributed as a dev dependency), `nunomaduro/larastan` (PHPStan rules for Laravel/Eloquent-aware static analysis), `driftingly/rector-laravel` (Rector rules for Laravel upgrades/refactors).
- Prefer first-party packages for core cross-cutting concerns (auth, queues, search) before reaching for a third-party alternative, unless the third-party package is clearly the ecosystem standard for the need (e.g. `spatie/laravel-permission` for complex role/permission matrices beyond simple Gates).

## Vetting A New Package

Before adding a dependency (coordinate with `/researcher` for deeper comparisons):

- Actively maintained and compatible with the project's PHP and Laravel versions (check the package's `composer.json` constraints)?
- License compatible with the project?
- Reasonable transitive dependency footprint?
- No open advisories (`composer audit` after install)?
- Would a small amount of first-party code, or a feature Laravel already ships (queues, notifications, Sanctum), avoid a heavy dependency? Weigh it.

## Guardrails

- Never write credentials into `composer.json` (`github-oauth`, `http-basic`); use auth outside the repo.
- Do not commit `vendor/`.
- Do not run a blanket `composer update` right before release; do scoped, tested updates.
- Do not add a dependency to solve a one-line problem.

## Final Output

Return audit/outdated results, actions taken (updates, constraint changes, autoload optimization), residual advisories/risks, verification run, Context Summary, and next step (`/verify`, `/security-reviewer`, or `/researcher`).
