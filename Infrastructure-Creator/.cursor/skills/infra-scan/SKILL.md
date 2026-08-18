---
name: infra-scan
description: Run Phase 1 discovery against a target PHP project and compile a human Project Profile plus a machine-readable evidence ledger with one complete generation contract per justified skill. Use when starting bespoke infrastructure generation from a target path.
phase: orchestration
flow-next: infra-generate
flow-alternatives: [infra-build, stack-adapter]
related: [stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, domain-behavior-scanner, stack-researcher, clarifying-interview, profile-synthesizer, stack-adapter]
---

# Infra Scan

## Overview

`infra-scan` turns a target PHP project into two reviewable handoff artifacts:
a human Project Profile and a machine-readable evidence/skill generation plan.
The plan gives every justified skill its own evidence, ownership, procedure,
verification, output, failure, and routing contract. This skill never writes
into the target project - only into this folder's own `tasks/TASK-{NNN}/`.

This skill does not generate anything for a PHP target. It stops at the profile. `infra-generate` is a separate, later step the user runs only after reviewing the profile.

If the target turns out not to be PHP at all, this skill does not silently fail - see step 2 and `stack-adapter` below.

## Generated File Naming Convention (MANDATORY)

All output from this run lives under `tasks/TASK-{NNN}/` in Infrastructure-Creator's own folder (not the target project), where `{NNN}` is the next value from `tasks/.task-counter` zero-padded to three digits (`TASK-001`); see `stack-scanner/references/scan-evidence-contract.md`:

- per scanner, a report and an evidence ledger: `stack-scanner-findings.md` + `stack-scanner-evidence.json`, and the same pair for `architecture-scanner`, `integration-scanner`, `infra-ops-scanner`, `security-compliance-scanner`, `conventions-scanner`, `domain-behavior-scanner`
- `stack-researcher-findings.md`
- `clarifying-interview-questions.md`, `clarifying-interview-answers.md`
- `infra-scan-project-profile.md` (the deliverable)
- `skill-generation-plan.json` (validated evidence ledger and per-skill contracts)

## Process

1. **Validate the target.** Require an explicit target project path (e.g. "run infra-scan against ../my-php-app"). Refuse to proceed if no path was given, if the path does not exist, or if it resolves to Infrastructure-Creator's own directory tree.
2. **Confirm it is a PHP project - and branch if it is not.** There must be a `composer.json` and/or `*.php` files for the PHP pipeline (steps 3-9) to proceed. If there is no PHP evidence:
   - **Probe for a recognizable non-PHP stack** using manifest/signal evidence (this is a lightweight presence check, not deep analysis - deep analysis of the detected stack happens only inside `stack-adapter`, and only if the user opts in): `pubspec.yaml` (+ `*.dart`) -> Flutter/Dart; `package.json` -> Node.js/JavaScript/TypeScript; `requirements.txt`/`pyproject.toml`/`Pipfile` -> Python; `go.mod` -> Go; `Gemfile` -> Ruby; `pom.xml`/`build.gradle`/`build.gradle.kts` -> Java/Kotlin; `*.csproj`/`*.sln` -> .NET/C#; `Cargo.toml` -> Rust; `Package.swift` -> Swift. This list is illustrative, not exhaustive - any other clear ecosystem manifest counts too.
   - **If a recognizable non-PHP stack is found:** STOP the PHP pipeline (do not run the seven PHP scanners) and ask the user one question: *"This target uses [detected stack], not PHP. Infrastructure-Creator only generates PHP accelerators directly, but it can build you an independent sibling generator - `Infrastructure-Creator-[Stack]` - with the identical architecture, freshly researched and authored for [detected stack]. Create it?"* If yes, invoke `stack-adapter` with the target path and the detected stack name; report its result and stop (do not continue this skill's own PHP steps). If no, STOP and report the target is out of scope, same as below.
   - **If nothing recognizable is found at all:** STOP and report the target is out of scope (this tool only generates PHP accelerators, and no other stack could even be identified) rather than scanning further.
   - Otherwise (PHP evidence found): continue to step 3.
3. **Collision note.** Check whether the target already has `AGENTS.md` or any AI-tool edition folder (`.claude/`, `.cursor/`, `.codex/`, `.agents/`). If so, note it in the profile's "Generation Notes" so `infra-generate` asks about overwrite/merge/abort before writing. (This read-only phase does not need to ask yet.)
4. **Allocate the task directory.** Read `tasks/.task-counter`, create the zero-padded `tasks/TASK-{NNN}/`, and increment the counter.
5. **Fan out the seven scanners.**
   - **If your AI tool supports parallel subagents/tool calls:** spawn all seven in one batch so they run concurrently: `stack-scanner`, `architecture-scanner`, `integration-scanner`, `infra-ops-scanner`, `security-compliance-scanner`, `conventions-scanner`, `domain-behavior-scanner`, each given the target path and the task directory. Wait for all seven before continuing.
   - **If your AI tool is single-threaded:** invoke each scanner's logic sequentially in the same session. Output is identical; only mechanics differ. Say so in the Context Summary.
   - Treat test topology, exact/resolved command definitions, stable high-priority invariant IDs, bounded evidence anchors, path authority/creatability, material adjacency, and routing cases as mandatory cross-scanner outputs. A scanner that omits its applicable portion is incomplete, not silently optional.
6. **Run `stack-researcher`** once the scanners have written findings - it needs `integration-scanner-findings.md` (what to research) and `stack-scanner-findings.md` (the PHP framework/version to ground research in).
7. **Run `clarifying-interview`** once research is done - it turns remaining `inferred`/`unknown` items into a short question set and always asks the mandatory AI-tool-selection question.
8. **Run `profile-synthesizer`** last - it produces both handoff artifacts,
   validates evidence paths and fingerprints, runs complete plan-level
   operational-safety, ownership/write/routing/flow diagnostics, prunes
   unjustified or conflicting skill proposals, and requires one complete schema
   1.2 contract per retained skill. It maps every high-priority confirmed
   invariant to a procedure and concrete verification assertion and compiles
   runtime-fixed contracts from `memory-seed/assets/runtime-contract.json`.
   Stop before approval on any blocking diagnostic; schema migration and
   calibrated similarity warnings remain visible but non-blocking.
9. **Stop.** Do not proceed to generation automatically - the profile is a human checkpoint by design.

## Output Template

```markdown
# Infra Scan Complete: [target_name]

**Task:** tasks/TASK-{NNN}/
**Target:** [target path]
**PHP:** [detected PHP version + framework]
**Confidence summary:** [X confirmed, Y inferred, Z unknown]
**AI tool(s) selected:** [from the interview]

## What Was Found
[2-4 sentences: PHP framework, architecture pattern, key integrations, and the most important confirmed domain invariants/risks]

## What Will Be Generated (see profile sections 11-12 for full detail)
- **Skills:** [dynamic evidence-gated count] - each has a distinct
  necessity/scope/procedure/output/routing contract; the memory quartet remains
  because its runtime is always installed
- **Agents & commands:** [counts from section 11.2, for the selected edition(s)]
- **Memory bank:** [count] cohesive confirmed concepts planned in section 12, each linked to canonical sources

## Open Items
[Anything still `unknown` after the interview, or flagged for the user to double check]

## Review This Before Generating
Read the Project Profile and review the proposed inventory. Inspect
`skill-generation-plan.json` when checking evidence, ownership boundaries, or
routing. Correct anything wrong, then run `infra-generate`.
```

## Guardrails

- MUST NOT write anything into the target project - Phase 1 is read-only there.
- MUST confirm PHP evidence before scanning; a non-PHP target with no recognizable stack at all is reported out of scope.
- MUST NOT invoke `stack-adapter` without first asking the user - detecting a foreign stack is never itself consent to generate a sibling tool.
- MUST NOT run the seven PHP scanners against a target that already failed the PHP-evidence check.
- MUST NOT skip the interview's mandatory AI-tool-selection question, even if an edition folder already exists elsewhere - confirm explicitly.
- MUST NOT let a slow/failed scanner silently drop from the profile - report it as a gap in the confidence summary.
- MUST NOT approve synthesis with missing applicable test topology, command definitions, evidence anchors, path authority, invariant mapping, or material routing adjacency.
- MUST NOT re-run scanners against an unchanged target just to double-check - one scan per invocation is the contract.

## Final Output

For a PHP target: return the task directory path, confidence summary, detected PHP stack, behavioral-contract highlights/contradictions, a preview of what will be generated (skill/agent/command counts and memory-bank concept count, per profile sections 11-12), explicit open items, selected AI tool(s), and the exact next step.

For a non-PHP target where the user opted into adaptation: return `stack-adapter`'s result (the new sibling generator's path and the suggested next command to run there) instead of a Project Profile.

For a non-PHP target with no recognizable stack, or where the user declined adaptation: return a short out-of-scope report - what path was checked, what (if anything) was tentatively recognized, and why nothing was generated.
