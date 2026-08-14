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

The target project path is a **required** argument (e.g. "scan conventions of ../my-php-app"). Never assume the current working directory is the target. Operate strictly read-only within that path; never write to it, never read its `.env`/secrets.

## Generated File Naming Convention (MANDATORY)

Write exactly one findings file into the current run's task directory: `tasks/TASK-{N}/conventions-scanner-findings.md`. Never write into the target.

## Process

1. **Detect code style/format config:** `.php-cs-fixer.dist.php`/`.php-cs-fixer.php` (PHP-CS-Fixer), `pint.json` (Pint), `phpcs.xml`/`.phpcs.xml.dist` (PHP_CodeSniffer). Record the configured ruleset/preset where the file states it. Cite path:line.
2. **Reference, do not duplicate, static analysis.** If PHPStan/Psalm/Rector baselines exist, note only that they govern conventions and point to `stack-scanner` findings; do not re-scan tooling identity here.
3. **Detect git hooks:** `captainhook.json`, `.husky/` directory, `.pre-commit-config.yaml`, or hook scripts under `.git/hooks` templates tracked in the repo. Cite each path.
4. **Detect editor config:** `.editorconfig` and its key settings (indent style/size, end-of-line, final-newline). Cite path:line.
5. **Detect commit conventions:** Conventional Commits config (`commitlint.config.*`, `.commitlintrc*`), `.gitmessage` templates, or a documented commit policy in `CONTRIBUTING.md`. Cite the evidence.
6. **Detect docs/ADRs:** `docs/`, `adr/`, `decisions/`, or `doc/adr/` directories and any ADR index; note count and location. Cite paths.
7. **Detect contribution governance and path authority:** `CONTRIBUTING.md`, `CODEOWNERS`, PR/issue templates under `.github/`, generated-file notices, canonical/source-of-truth declarations, and documented creatable output locations. For each governed path/glob, cite the bounded authority anchor and classify it `required-existing`, `generated-runtime`, `creatable`, or `unknown`; Git history or directory presence alone does not grant write authority.
8. **Cross-check command behavior.** For documented lint/test/check commands, reference stack-scanner's resolved definition and record whether documentation conflicts with actual mutation/network behavior (for example, a `lint` alias that invokes `--fix`).
9. **Mark confidence** per finding: `confirmed` (config file present), `inferred` (indirect signal, e.g. consistent style with no config), or `unknown`. Never present a guess as fact.

## Output Template

```markdown
# Conventions Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Code Style / Format
- [tool: config path:L# + preset] (confirmed); "N/A - not configured" where none

## Static Analysis (reference only)
- [governs conventions; see stack-scanner findings] (confirmed/inferred)

## Git Hooks
- [captainhook.json / .husky / .pre-commit-config.yaml] (confirmed - path)

## Editor Config
- [.editorconfig key settings] (confirmed - path:L#)

## Commit Conventions
- [Conventional Commits / template / documented policy] (confirmed/inferred - path)

## Docs / ADRs
- [docs/, adr/, decisions/ + count] (confirmed - path)

## Contribution Governance
- [CONTRIBUTING.md, CODEOWNERS, .github templates] (confirmed - path)

## Path Authority & Documented Commands
- [path/glob -> required-existing | generated-runtime | creatable | unknown -> bounded authority anchor]
- [documented command -> resolved definition/effect class -> conflict or none]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Guardrails

- MUST cite a real file path (and line where practical) for every finding.
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.
- MUST focus on style + governance and REFERENCE `stack-scanner` for static-analysis/test/lint tooling rather than duplicating it.
- MUST report absent config as `N/A - not configured` rather than assuming a default style.
- MUST NOT infer path ownership or creatability from Git history, directory shape, or a catalog recommendation.
- MUST flag documented verification commands whose resolved definitions mutate files, use the network, or perform external side effects.
- MUST NOT deep-dive stack identity, architecture, integrations, infra, or security - those belong to their own scanners.

## Final Output

Return the findings file path, the detected code style/format config, git hooks, commit conventions, docs/ADR locations, and governance files, plus a one-line confidence summary. Suggest `profile-synthesizer` (to fold conventions into the target profile) as the next step.
