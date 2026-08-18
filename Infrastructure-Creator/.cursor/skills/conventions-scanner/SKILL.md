---
name: conventions-scanner
description: Detect a PHP target's coding conventions and project hygiene - code style/format config, git hooks, editorconfig, commit conventions, docs/ADRs, and contribution governance - from real evidence, focusing on style and governance rather than duplicating stack-scanner's tooling scan. Use as Phase 1 discovery input to profile-synthesizer. Triggers on "scan conventions", "what code style does this project use", "detect git hooks / commit conventions", "conventions-scanner".
phase: discovery
flow-next: profile-synthesizer
flow-alternatives: [stack-researcher]
related: [stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, infra-scan]
---

# Conventions Scanner

## Overview

Read-only reconnaissance of a PHP target's coding conventions and project hygiene: code style/format configuration, git hooks, editor config, commit-message conventions, architectural decision records, and contribution governance. This scanner focuses on **style and governance**. Static-analysis and test/lint tooling identity is `stack-scanner`'s job - reference those findings rather than re-detecting or duplicating them here.

The target project path is a **required** argument; never assume the current working directory is the target. Operate strictly read-only within it, under the contract's secrets rule.

## Outputs (MANDATORY)

Per run: exactly one report `tasks/TASK-{NNN}/conventions-scanner-findings.md` and exactly one evidence ledger `tasks/TASK-{NNN}/conventions-scanner-evidence.json`, both shaped by `stack-scanner/references/scan-evidence-contract.md` - read it first, including its sibling-input fallback. Never write into the target.

## Process

1. **Detect code style/format config:** `.php-cs-fixer.dist.php`/`.php-cs-fixer.php` (PHP-CS-Fixer), `pint.json` (Pint), `phpcs.xml`/`.phpcs.xml.dist` (PHP_CodeSniffer). Record the configured ruleset/preset where the file states it. Cite path:line.
2. **Reference, do not duplicate, static analysis.** Read the sibling artifacts at `tasks/TASK-{NNN}/stack-scanner-findings.md` and `tasks/TASK-{NNN}/stack-scanner-evidence.json` in this run's task directory. If PHPStan/Psalm/Rector baselines appear there, note only that they govern conventions and cite the sibling evidence id; do not re-scan tooling identity here. If those artifacts are absent, apply the contract's sibling-input fallback - record the gap line, leave this section `unknown`, and do not substitute a tooling scan of your own.
3. **Detect git hooks:** `captainhook.json`, `.husky/` directory, `.pre-commit-config.yaml`, or hook scripts under `.git/hooks` templates tracked in the repo. Cite each path.
4. **Detect editor config:** `.editorconfig` and its key settings (indent style/size, end-of-line, final-newline). Cite path:line.
5. **Detect commit conventions:** Conventional Commits config (`commitlint.config.*`, `.commitlintrc*`), `.gitmessage` templates, or a documented commit policy in `CONTRIBUTING.md`. Cite the evidence.
6. **Detect docs/ADRs:** `docs/`, `adr/`, `decisions/`, or `doc/adr/` directories and any ADR index; note count and location. Cite paths.
7. **Detect contribution governance and path authority:** `CONTRIBUTING.md`, `CODEOWNERS`, PR/issue templates under `.github/`, generated-file notices, canonical/source-of-truth declarations, and documented creatable output locations. For each governed path/glob, cite the bounded authority anchor and classify it `required-existing`, `generated-runtime`, `creatable`, or `unknown`; Git history or directory presence alone does not grant write authority.
8. **Cross-check command behavior.** For documented lint/test/check commands, compare the documentation against the resolved definition in the same sibling `stack-scanner` artifacts and record whether documentation conflicts with actual mutation/network behavior (for example, a `lint` alias that invokes `--fix`). With the sibling absent, record the documented command verbatim, mark its real effect `unknown`, and log the gap - never assume the documentation describes what the command does.
9. **Mark confidence** per finding: `confirmed` (config file present), `inferred` (indirect signal, e.g. consistent style with no config), or `unknown`. Never present a guess as fact.

## Report Structure

Follow the `conventions-scanner` report template in appendix A of `stack-scanner/references/scan-evidence-contract.md`. Every factual line carries its confidence tag and its evidence id.

## Guardrails

- MUST cite a real file path (and line where practical) for every finding, and MUST emit both artifacts with contract-shaped evidence records (target-relative path, `sha256:` fingerprint, supported claims).
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.
- MUST take sibling `stack-scanner` input from this run's task directory and degrade with a recorded gap when it is missing; MUST NOT re-derive or invent it.
- MUST focus on style + governance and REFERENCE `stack-scanner` for static-analysis/test/lint tooling rather than duplicating it.
- MUST report absent config as `N/A - not configured` rather than assuming a default style.
- MUST NOT infer path ownership or creatability from Git history, directory shape, or a catalog recommendation.
- MUST flag documented verification commands whose resolved definitions mutate files, use the network, or perform external side effects.
- MUST NOT deep-dive stack identity, architecture, integrations, infra, or security - those belong to their own scanners.

## Final Output

Return both artifact paths (report and evidence ledger), the detected code style/format config, git hooks, commit conventions, docs/ADR locations, and governance files, plus a one-line confidence summary. Suggest `profile-synthesizer` (to fold conventions into the target profile) as the next step.
