---
name: infra-scan
description: Run the full Phase 1 discovery pipeline against a target PHP project - six parallel scanners, dependency/integration research, a minimal clarifying interview, and synthesis into one reviewable Project Profile. Use when the user wants to start generating a bespoke accelerator for a specific PHP project, points Infrastructure-Creator at a target path, or asks to "scan my project" / "analyze this codebase for infrastructure generation". Triggers on "infra-scan", "scan this project", "analyze my PHP project for the accelerator generator", "start the infrastructure creator".
phase: orchestration
flow-next: infra-generate
flow-alternatives: [infra-build, stack-adapter]
related: [stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, stack-researcher, clarifying-interview, profile-synthesizer, stack-adapter]
---

# Infra Scan

## Overview

`infra-scan` is one of the four orchestrator skills permitted to fan out other skills automatically (see `AGENTS.md`'s "Orchestration Exception"). Its purpose is to turn a target PHP project's path into one reviewable, evidence-backed Project Profile: it fans out the six Phase 1 scanners, runs bounded web research on the real dependencies, asks the user only what evidence could not settle (including which AI tool the target team uses), and synthesizes everything into a single file. It never writes into the target project - only into this folder's own `tasks/TASK-{N}/`.

This skill does not generate anything for a PHP target. It stops at the profile. `infra-generate` is a separate, later step the user runs only after reviewing the profile.

If the target turns out not to be PHP at all, this skill does not silently fail - see step 2 and `stack-adapter` below.

## Generated File Naming Convention (MANDATORY)

All output from this run lives under `tasks/TASK-{N}/` in Infrastructure-Creator's own folder (not the target project), where `{N}` is the next value from `tasks/.task-counter`:

- `stack-scanner-findings.md`, `architecture-scanner-findings.md`, `integration-scanner-findings.md`, `infra-ops-scanner-findings.md`, `security-compliance-scanner-findings.md`, `conventions-scanner-findings.md`
- `stack-researcher-findings.md`
- `clarifying-interview-questions.md`, `clarifying-interview-answers.md`
- `infra-scan-project-profile.md` (the deliverable)

## Process

1. **Validate the target.** Require an explicit target project path (e.g. "run infra-scan against ../my-php-app"). Refuse to proceed if no path was given, if the path does not exist, or if it resolves to Infrastructure-Creator's own directory tree.
2. **Confirm it is a PHP project - and branch if it is not.** There must be a `composer.json` and/or `*.php` files for the PHP pipeline (steps 3-9) to proceed. If there is no PHP evidence:
   - **Probe for a recognizable non-PHP stack** using manifest/signal evidence (this is a lightweight presence check, not deep analysis - deep analysis of the detected stack happens only inside `stack-adapter`, and only if the user opts in): `pubspec.yaml` (+ `*.dart`) -> Flutter/Dart; `package.json` -> Node.js/JavaScript/TypeScript; `requirements.txt`/`pyproject.toml`/`Pipfile` -> Python; `go.mod` -> Go; `Gemfile` -> Ruby; `pom.xml`/`build.gradle`/`build.gradle.kts` -> Java/Kotlin; `*.csproj`/`*.sln` -> .NET/C#; `Cargo.toml` -> Rust; `Package.swift` -> Swift. This list is illustrative, not exhaustive - any other clear ecosystem manifest counts too.
   - **If a recognizable non-PHP stack is found:** STOP the PHP pipeline (do not run the six PHP scanners) and ask the user one question: *"This target uses [detected stack], not PHP. Infrastructure-Creator only generates PHP accelerators directly, but it can build you an independent sibling generator - `Infrastructure-Creator-[Stack]` - with the identical architecture, freshly researched and authored for [detected stack]. Create it?"* If yes, invoke `stack-adapter` with the target path and the detected stack name; report its result and stop (do not continue this skill's own PHP steps). If no, STOP and report the target is out of scope, same as below.
   - **If nothing recognizable is found at all:** STOP and report the target is out of scope (this tool only generates PHP accelerators, and no other stack could even be identified) rather than scanning further.
   - Otherwise (PHP evidence found): continue to step 3.
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

## What Will Be Generated (see profile sections 10-11 for full detail)
- **Skills:** [total count] across architecture (1) / design & interaction (3) / frontend (0 or 5) / process & workflow (14) / universal PHP (7) / framework-specialty ([N], evidence-driven) / integrations ([M], one per confirmed integration) - each with a target-specific description in section 10.1 (not just a bare name)
- **Agents & commands:** [counts from section 10.2, for the selected edition(s)]
- **Memory bank:** [count] chunks planned in section 11, one per durable confirmed fact, each cited to its source

## Open Items
[Anything still `unknown` after the interview, or flagged for the user to double check]

## Review This Before Generating
Read `tasks/TASK-{N}/infra-scan-project-profile.md` in full - not just the confidence summary. Section 10 tells you exactly what each generated skill/agent/command will be; section 11 previews every memory-bank chunk before it exists. Correct anything wrong, then run `infra-generate` against `[target path]`.
```

## Guardrails

- MUST NOT write anything into the target project - Phase 1 is read-only there.
- MUST confirm PHP evidence before scanning; a non-PHP target with no recognizable stack at all is reported out of scope.
- MUST NOT invoke `stack-adapter` without first asking the user - detecting a foreign stack is never itself consent to generate a sibling tool.
- MUST NOT run the six PHP scanners against a target that already failed the PHP-evidence check.
- MUST NOT skip the interview's mandatory AI-tool-selection question, even if an edition folder already exists elsewhere - confirm explicitly.
- MUST NOT let a slow/failed scanner silently drop from the profile - report it as a gap in the confidence summary.
- MUST NOT re-run scanners against an unchanged target just to double-check - one scan per invocation is the contract.

## Final Output

For a PHP target: return the task directory path, the confidence summary, the detected PHP stack, a preview of what will be generated (skill/agent/command counts and the memory-bank chunk count, per profile sections 10-11), explicit open items, the selected AI tool(s), and the exact next step (run `infra-generate` against `[target path]`) - framed as something the user runs only after reviewing the profile.

For a non-PHP target where the user opted into adaptation: return `stack-adapter`'s result (the new sibling generator's path and the suggested next command to run there) instead of a Project Profile.

For a non-PHP target with no recognizable stack, or where the user declined adaptation: return a short out-of-scope report - what path was checked, what (if anything) was tentatively recognized, and why nothing was generated.
