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

The target project path is a **required** argument; never assume the current working directory is the target. Operate strictly read-only within it, under the contract's secrets rule.

## Outputs (MANDATORY)

Per run: exactly one report `tasks/TASK-{NNN}/stack-scanner-findings.md` and exactly one evidence ledger `tasks/TASK-{NNN}/stack-scanner-evidence.json` and exactly one coverage record `tasks/TASK-{NNN}/stack-scanner-coverage.json`, all shaped by `stack-scanner/references/scan-evidence-contract.md` - read it first. Never write into the target.

## Process

1. **Read `composer.json` (and `composer.lock` if present).** Extract: package `name`/`type`, the `require.php` version constraint, `require` and `require-dev` maps, the PSR-4 `autoload`/`autoload-dev` namespace->path map, and the `scripts` section (these reveal the team's real test/lint/analyse commands).
2. **Identify the framework** from `require` and entry files, citing evidence:
   - Laravel: `laravel/framework`, `artisan`, `bootstrap/app.php`.
   - Symfony: `symfony/framework-bundle`, `bin/console`, `config/bundles.php`.
   - Slim / Laminas / CodeIgniter / Yii / CakePHP: their respective root packages and entry points.
   - Plain PHP: `public/index.php` or a front controller with no framework package.
3. **Detect the package manager and PHP runtime, keeping constraint and pin apart** (contract, PHP runtime sources). Composer is expected. `composer.lock` `platform` mirrors the *constraint*: `{"php": ">=8.2.0"}` is not a version. The *pin* lives in `platform-overrides`, `.php-version`, or an image/CI declaration. Report both, labelled; never present a constraint as the resolved version, and leave the pin `unknown` when only a constraint exists (defer deep infra to `infra-ops-scanner`).
4. **Detect entry points and build tooling:** front controller(s), console entry (`artisan`/`bin/console`), asset build (`package.json` scripts, Vite/Mix config) noted only as a build-tool fact, not as a JS skill.
5. **Capture test topology, not just the runner:** `phpunit.xml`/`phpunit.xml.dist` (PHPUnit), `pest` in `require-dev` + `tests/Pest.php` (Pest), every configured suite/root/bootstrap, central versus module/tenant/provider test placement, unit/integration/feature/browser conventions, fixture/factory/fake locations, and the narrowest evidenced invocation for each suite. Cite config anchors and representative test paths; do not infer topology from framework defaults.
6. **Detect lint/format/static-analysis tooling:** `.php-cs-fixer.dist.php` (PHP-CS-Fixer), `pint.json` (Pint), `phpcs.xml`/`.phpcs.xml.dist` (PHP_CodeSniffer), `phpstan.neon(.dist)` (PHPStan/Larastan), `psalm.xml` (Psalm), `rector.php` (Rector).
7. **Compile repository command definitions from every command declaration site in the contract's checklist** - script sections, deploy/release scripts, environment and container hooks, CI steps, and project-declared console commands - not only Composer/package-manager scripts, where a real project's dangerous commands usually are not. Record each exact body, resolve aliases transitively where possible, and classify it as non-mutating, workspace-mutating, network-capable, or external-side-effect-capable. Mark bodies containing `--fix`, formatter writes, schema/migration writes (`doctrine:schema:update --force`, `migrate --force`), history rewrites (`git reset --hard`, force push), destructive database calls, or unresolved indirection; never label a command safe from its name alone, and report "no risky commands" only after the whole checklist was searched.
8. **Mark confidence** per finding: `confirmed` (direct evidence), `inferred` (indirect signal), or `unknown`. Never present a guess as fact.

## Report Structure

Follow the `stack-scanner` report template in appendix A of `stack-scanner/references/scan-evidence-contract.md`. Every factual line carries its confidence tag and its evidence id.

## Guardrails

- MUST cite a real file path (and line where practical) for every finding.
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.
- MUST distinguish `require` from `require-dev`.
- MUST report absent tooling as `N/A - not configured` rather than assuming a default.
- MUST preserve exact command definitions and classify effects from resolved bodies, not friendly script names; MUST search every declaration site in step 7 before reporting no risky commands.
- MUST distinguish the PHP version constraint from the pinned runtime; a constraint is never reported as the resolved version.
- MUST give every surface it saw one of the four dispositions in the coverage record; a surface nobody dispositioned is not the same as one nobody needed.
- MUST emit all three artifacts with contract-shaped evidence records (target-relative path, `sha256:` fingerprint, supported claims); a report with no ledger is an incomplete scan.
- MUST capture real suite boundaries and focused commands; MUST NOT collapse materially different central, module, tenant, integration, or provider tests into one generic test directory.
- MUST NOT deep-dive integrations, infra, security, or conventions - those belong to their own scanners.

## Final Output

Return all three artifact paths (report, evidence ledger, coverage record), the detected PHP version + framework, the test/lint/analysis tooling, and a one-line confidence summary. Suggest `stack-researcher` (to ground the detected framework/dependencies in current docs) as the next step.
