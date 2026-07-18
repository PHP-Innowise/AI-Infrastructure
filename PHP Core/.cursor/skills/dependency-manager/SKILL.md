---
name: dependency-manager
description: Manage Composer dependencies for native PHP projects: audit for vulnerabilities, review outdated packages, tighten version constraints, optimize autoloading, and vet new packages before adding them. Triggers on "composer", "dependency", "update packages", "composer audit", "outdated", "add a library".
phase: execution
flow-next: verify
flow-alternatives: [security-reviewer, researcher, code-reviewer]
related: [researcher, security-reviewer, verify]
---

# Dependency Manager

## Overview

Keep the dependency tree healthy, secure, and reproducible. Dependencies are attack surface and maintenance cost, so every addition and update is a deliberate decision.

## Core Commands

```bash
composer validate --strict     # composer.json/lock integrity
composer audit                 # known security advisories in the tree
composer outdated --direct     # direct deps with newer versions
composer show --tree           # inspect the dependency graph
composer why <package>         # why a package is installed
composer why-not <package> <version>  # what blocks an upgrade
```

## Health Check Workflow

1. **Integrity.** `composer validate --strict`; confirm `composer.lock` is committed and in sync.
2. **Security.** `composer audit`; triage each advisory (severity, whether the vulnerable path is used, fixed version available).
3. **Freshness.** `composer outdated --direct`; separate patch/minor (low risk) from major (needs review/changelog).
4. **Footprint.** Look for redundant, abandoned, or overly heavy packages (`composer show --tree`).
5. **Autoload.** Ensure PSR-4 autoload is correct; for production builds recommend `composer dump-autoload -o` (or `--classmap-authoritative`).

## Updating Safely

- Prefer scoped updates: `composer update vendor/package --with-dependencies` over a blanket `composer update`.
- Read the changelog/UPGRADE notes for minor/major bumps.
- Run the full test suite and static analysis after any update; a green build is the acceptance gate.
- Commit `composer.json` and `composer.lock` together with a note on what changed and why.

## Version Constraints

- Use caret constraints (`^1.2`) for libraries following SemVer to allow safe minor/patch updates.
- Avoid `*` and unbounded `>=` constraints; they make builds non-reproducible.
- Pin only when necessary (a known incompatibility) and document the reason.
- Keep `require` (runtime) and `require-dev` (tooling: PHPUnit, PHPStan, php-cs-fixer, Rector) correctly separated.

## Vetting A New Package

Before adding a dependency (coordinate with `/researcher` for deeper comparisons):

- Actively maintained and compatible with the project's PHP version?
- License compatible with the project?
- Reasonable transitive dependency footprint?
- No open advisories (`composer audit` after install)?
- Would a small amount of first-party code avoid a heavy dependency? Weigh it.

## Guardrails

- Never write credentials into `composer.json` (`github-oauth`, `http-basic`); use auth outside the repo.
- Do not commit `vendor/`.
- Do not run a blanket `composer update` right before release; do scoped, tested updates.
- Do not add a dependency to solve a one-line problem.

## Final Output

Return audit/outdated results, actions taken (updates, constraint changes, autoload optimization), residual advisories/risks, verification run, Context Summary, and next step (`/verify`, `/security-reviewer`, or `/researcher`).
