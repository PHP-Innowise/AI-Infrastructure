---
name: infra-scan
description: Run the full Phase 1 discovery pipeline against a target PHP project - six parallel scanners, dependency/integration research, a minimal clarifying interview, and synthesis into one reviewable Project Profile. Use when the user wants to start generating a bespoke accelerator for a specific PHP project, points Infrastructure-Creator at a target path, or asks to "scan my project" / "analyze this codebase for infrastructure generation". Triggers on "infra-scan", "scan this project", "analyze my PHP project for the accelerator generator", "start the infrastructure creator".
phase: orchestration
flow-next: infra-generate
flow-alternatives: [infra-build]
related: [stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, stack-researcher, clarifying-interview, profile-synthesizer]
---

# Infra Scan

## Overview

`infra-scan` is one of the three orchestrator skills permitted to fan out other skills automatically (see `AGENTS.md`'s "Orchestration Exception"). Its purpose is to turn a target PHP project's path into one reviewable, evidence-backed Project Profile: it fans out the six Phase 1 scanners, runs bounded web research on the real dependencies, asks the user only what evidence could not settle (including which AI tool the target team uses), and synthesizes everything into a single file. It never writes into the target project - only into this folder's own `tasks/TASK-{N}/`.

This skill does not generate anything. It stops at the profile. `infra-generate` is a separate, later step the user runs only after reviewing the profile.

## Generated File Naming Convention (MANDATORY)

All output from this run lives under `tasks/TASK-{N}/` in Infrastructure-Creator's own folder (not the target project), where `{N}` is the next value from `tasks/.task-counter`:

- `stack-scanner-findings.md`, `architecture-scanner-findings.md`, `integration-scanner-findings.md`, `infra-ops-scanner-findings.md`, `security-compliance-scanner-findings.md`, `conventions-scanner-findings.md`
- `stack-researcher-findings.md`
- `clarifying-interview-questions.md`, `clarifying-interview-answers.md`
- `infra-scan-project-profile.md` (the deliverable)

## Process

1. **Validate the target.** Require an explicit target project path (e.g. "run infra-scan against ../my-php-app"). Refuse to proceed if no path was given, if the path does not exist, or if it resolves to Infrastructure-Creator's own directory tree.
2. **Confirm it is a PHP project.** There must be a `composer.json` and/or `*.php` files. If there is no PHP evidence at all, STOP and report the target is out of scope (this tool only generates PHP accelerators) rather than scanning further.
3. **Collision note.** Check whether the target already has `AGENTS.md` or any AI-tool edition folder (`.claude/`, `.cursor/`, `.codex/`, `.agents/`). If so, note it in the profile's "Generation Notes" so `infra-generate` asks about overwrite/merge/abort before writing. (This read-only phase does not need to ask yet.)
4. **Allocate the task directory.** Read `tasks/.task-counter`, create `tasks/TASK-{N}/`, and increment the counter.
5. **Fan out the six scanners.**
   - **If your AI tool supports parallel subagents/tool calls:** spawn all six in one batch so they run concurrently: `stack-scanner`, `architecture-scanner`, `integration-scanner`, `infra-ops-scanner`, `security-compliance-scanner`, `conventions-scanner`, each given the target path and the task directory. Wait for all six before continuing.
   - **If your AI tool is single-threaded:** invoke each scanner's logic sequentially in the same session. Output is identical; only mechanics differ. Say so in the Context Summary.
6. **Run `stack-researcher`** once the scanners have written findings - it needs `integration-scanner-findings.md` (what to research) and `stack-scanner-findings.md` (the PHP framework/version to ground research in).
7. **Run `clarifying-interview`** once research is done - it turns remaining `inferred`/`unknown` items into a short question set and always asks the mandatory AI-tool-selection question.
8. **Run `profile-synthesizer`** last - it consumes all of the above and produces `infra-scan-project-profile.md`, validating against `profile-synthesizer/references/project-profile-schema.md` before this skill reports done.
9. **Stop.** Do not proceed to generation automatically - the profile is a human checkpoint by design.

## Output Template

```markdown
# Infra Scan Complete: [target_name]

**Task:** tasks/TASK-{N}/
**Target:** [target path]
**PHP:** [detected PHP version + framework]
**Confidence summary:** [X confirmed, Y inferred, Z unknown]
**AI tool(s) selected:** [from the interview]

## What Was Found
[2-4 sentences: PHP framework, architecture pattern, key integrations]

## Open Items
[Anything still `unknown` after the interview, or flagged for the user to double check]

## Review This Before Generating
Read `tasks/TASK-{N}/infra-scan-project-profile.md`. Correct anything wrong, then run `infra-generate` against `[target path]`.
```

## Guardrails

- MUST NOT write anything into the target project - Phase 1 is read-only there.
- MUST confirm PHP evidence before scanning; a non-PHP target is reported out of scope.
- MUST NOT skip the interview's mandatory AI-tool-selection question, even if an edition folder already exists elsewhere - confirm explicitly.
- MUST NOT let a slow/failed scanner silently drop from the profile - report it as a gap in the confidence summary.
- MUST NOT re-run scanners against an unchanged target just to double-check - one scan per invocation is the contract.

## Final Output

Return the task directory path, the confidence summary, the detected PHP stack, explicit open items, the selected AI tool(s), and the exact next step (run `infra-generate` against `[target path]`) - framed as something the user runs only after reviewing the profile.
