---
name: stack-scanner
description: Detect a PHP target's language version, framework(s), package manager, autoload map, entry points, build tooling, test tooling, and lint/format/static-analysis tooling from real evidence. Use as Phase 1 discovery input to profile-synthesizer. Triggers on "scan the stack", "detect the framework", "what PHP version/tooling does this project use", "stack-scanner".
phase: discovery
flow-next: stack-researcher
flow-alternatives: [profile-synthesizer]
related: [architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, infra-scan]
---

# Stack Scanner

## Overview

Read-only reconnaissance of a PHP target's identity: PHP version constraint, framework(s), package manager, PSR-4 autoload map, entry points, and build/test/lint/static-analysis tooling. This is the foundation scan - every other Phase 1 scanner and `profile-synthesizer` assume the target's basic PHP identity is known - so `stack-scanner` must be accurate and evidence-backed above all else.

The target project path is a **required** argument (e.g. "scan the stack of ../my-php-app"). Never assume the current working directory is the target. Operate strictly read-only within that path; never write to it, never read its `.env`/secrets.

## Generated File Naming Convention (MANDATORY)

Write exactly one findings file into the current run's task directory: `tasks/TASK-{N}/stack-scanner-findings.md`.

## Process

1. **Read `composer.json` (and `composer.lock` if present).** Extract: package `name`/`type`, the `require.php` version constraint, `require` and `require-dev` maps, the PSR-4 `autoload`/`autoload-dev` namespace->path map, and the `scripts` section (these reveal the team's real test/lint/analyse commands).
2. **Identify the framework** from `require` and entry files, citing evidence:
   - Laravel: `laravel/framework`, `artisan`, `bootstrap/app.php`.
   - Symfony: `symfony/framework-bundle`, `bin/console`, `config/bundles.php`.
   - Slim / Laminas / CodeIgniter / Yii / CakePHP: their respective root packages and entry points.
   - Plain PHP: `public/index.php` or a front controller with no framework package.
3. **Detect the package manager and PHP runtime.** Composer is expected; note the resolved PHP version from `composer.lock` `platform` or `require.php`, and any `.php-version`/Docker PHP base image if trivially visible (defer deep infra to `infra-ops-scanner`).
4. **Detect entry points and build tooling:** front controller(s), console entry (`artisan`/`bin/console`), asset build (`package.json` scripts, Vite/Mix config) noted only as a build-tool fact, not as a JS skill.
5. **Detect test tooling:** `phpunit.xml`/`phpunit.xml.dist` (PHPUnit), `pest` in `require-dev` + `tests/Pest.php` (Pest); note the test directory layout.
6. **Detect lint/format/static-analysis tooling:** `.php-cs-fixer.dist.php` (PHP-CS-Fixer), `pint.json` (Pint), `phpcs.xml`/`.phpcs.xml.dist` (PHP_CodeSniffer), `phpstan.neon(.dist)` (PHPStan/Larastan), `psalm.xml` (Psalm), `rector.php` (Rector).
7. **Mark confidence** per finding: `confirmed` (direct evidence), `inferred` (indirect signal), or `unknown`. Never present a guess as fact.

## Output Template

```markdown
# Stack Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## PHP Runtime
- Version constraint: [e.g. ^8.2] (confirmed - composer.json:L#)
- Resolved version: [from composer.lock] (confirmed/inferred)

## Framework
- [Name + version] (confirmed - evidence path:L#)

## Package Manager & Autoload
- Composer; PSR-4: [Namespace => path, ...] (confirmed - composer.json)

## Entry Points
- [public/index.php, artisan, bin/console, ...] (confirmed - path)

## Testing
- [PHPUnit/Pest + config path] (confirmed/inferred)

## Lint / Format / Static Analysis
- [tool: config path] for each detected; "N/A - not configured" where none

## Composer Scripts
- [script name: command] for test/lint/analyse-relevant scripts

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Guardrails

- MUST cite a real file path (and line where practical) for every finding.
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.
- MUST distinguish `require` from `require-dev`.
- MUST report absent tooling as `N/A - not configured` rather than assuming a default.
- MUST NOT deep-dive integrations, infra, security, or conventions - those belong to their own scanners.

## Final Output

Return the findings file path, the detected PHP version + framework, the test/lint/analysis tooling, and a one-line confidence summary. Suggest `stack-researcher` (to ground the detected framework/dependencies in current docs) as the next step.
