# MANIFEST - Infrastructure-Creator Design

The living design record for the generator itself. `tasks/TASK-{N}/` holds per-run working docs; this file holds the durable design.

## Purpose

A standalone, PHP-only generator that scans a target PHP project and writes a bespoke accelerator into it, for only the AI tool(s) the target team selects. It is not itself a ready-to-use accelerator.

## Key Decisions

- **PHP-only.** Scanners and generated skills are specialized for PHP (`composer.json`, PHP frameworks, PHPUnit/Pest, PHPStan/Psalm, PHP-CS-Fixer/Pint/Rector). Non-PHP neighbors are captured only as integration contracts.
- **100% discovery-driven, with one carve-out: the stack-agnostic memory runtime ships verbatim.** Every authored artifact (policies, skills, agents, commands, hooks, chunks) is written from evidence in the target; there is no bundled reference accelerator and no templating from any other project. The only files copied byte-for-byte are the dependency-free context-brain runtime and skeleton bundled under `memory-seed/assets/` (`context.py`, `brain_runtime.py`, `context_retrieval.py`, `validate.py`, `templates/chunk.md`, and the `project-brain/` skeleton with `PROTOCOL.md`, schemas, and record templates) - they contain nothing target-specific to adapt, except the two sanctioned substitutions in `runtime.json.template`: `{{TARGET_FRAMEWORK}}` (degrading to `generic` for unconfirmed frameworks; unknown slugs are safe because the runtime's only framework-keyed lookup contributes no exemptions for unknown keys) and `{{CANONICAL_EDITION}}` (the parity source-of-truth skills root, derived from the selected editions so it always names a tree that exists in the target).
- **Fully independent.** No references to or dependencies on any other accelerator or sibling folder. All needed reference material is bundled under the skills.
- **Two-phase with a human checkpoint.** `infra-scan` produces one reviewable Project Profile; `infra-generate` consumes the approved profile. `infra-build` chains both, pausing only when necessary.
- **Tool-selected output.** `clarifying-interview` captures the AI-tool selection; generation produces only the selected edition(s).
- **The generator ships tripled.** It runs natively from Claude Code (`.claude/`), Cursor (`.cursor/`), and Codex (`.agents/skills` + `.codex/`).
- **Workspace boundary + collision guard.** It runs outside the target, takes the target path as a required argument, is read-only in Phase 1, and never overwrites a pre-existing accelerator without an explicit decision.
- **Self-adapting for non-PHP stacks.** `infra-scan` never silently fails on a non-PHP target: it probes for a recognizable non-PHP stack and, with explicit consent, hands off to `stack-adapter` to build an entirely independent sibling generator for that stack, rather than stretching this PHP-only generator beyond its domain.
- **Behavioral-contract discovery.** A seventh scanner extracts source-backed domain vocabulary, sources of truth, central entities, invariants, proven transitions, permissions, audit obligations, risk-sensitive workflows, critical regression scenarios, and sanitized incident lessons while preserving source type and contradictions.
- **Broad, evidence-gated target skill catalog.** `skill-forge` generates eight groups: architecture; design & interaction; conditional frontend; 18 process/workflow skills including the memory quartet (`memory-bank`, `project-brain`, `checkpoint`, `memory`); 7 universal PHP skills; framework specialties; confirmed integrations; and evidence-gated bounded-context domain skills.
- **Full context-brain delivery.** Generated targets receive the same memory architecture as the monorepo's hand-built accelerators: the shared durable `memory-bank/` plus its runtime (`memory-bank/scripts/context.py` facade over `brain_runtime.py`/`context_retrieval.py`/`validate.py`), the governed `project-brain/` control plane skeleton, the automatic working-memory hooks (`working-memory-read.sh` on prompt, `working-memory-write.sh` on turn end; Cursor gets only the write half - it has no prompt-time hook event), and the operational memory quartet skills. `bootstrap-verifier` smoke-runs `context.py status`/`validate` in the generated tree and rejects wiring files that reference missing or non-executable hooks.
- **Upgradeable output, single version source.** The root `VERSION` file is the only place the generator's version lives; the Project Profile's metadata, a generated target `AGENTS.md` stamp, and `.infra-manifest.json` all read it. Every generation writes `.infra-manifest.json` from an explicit write plan (version, source profile, sha256 per planned generated file; runtime state such as memory chunks, indexes, and counters is deliberately untracked). Manifest membership exclusively defines ownership in both full and merge modes. `infra-update` consumes it with a strict rule: hash-unchanged tracked files are safe to replace, hash-changed tracked files require an explicit per-file decision, unlisted files are untouchable unless they collide with a newly staged path, and a missing manifest aborts before writes. `bootstrap-verifier` validates membership, hashes, decision schema, tracked stamps, and placeholders without scanning unmanifested team files.

## Skill Inventory (23 in every edition)

- Orchestration: `infra-scan`, `infra-generate`, `infra-build`, `infra-update`, `stack-adapter`.
- Discovery: `stack-scanner`, `architecture-scanner`, `integration-scanner`, `infra-ops-scanner`, `security-compliance-scanner`, `conventions-scanner`, `domain-behavior-scanner`.
- Research: `stack-researcher`.
- Synthesis: `clarifying-interview`, `profile-synthesizer`.
- Generation: `policy-forge`, `skill-forge`, `agent-forge`, `command-forge`, `hook-forge`, `memory-seed`, `skill-flow-composer`.
- Verification: `bootstrap-verifier`.

## Tech Stack (of the generator itself)

- Plain-markdown skill convention (`SKILL.md` with `name`/`description`/`phase`/`flow-next`/`flow-alternatives`/`related` frontmatter), edition-specific agent/command wrappers, POSIX-bash hooks, and dependency-free Python validators (`bootstrap-verifier/scripts/validate_generated.py`, and the bundled `memory-seed/assets/scripts/validate.py`).
- A root `VERSION` file (semver, single line) as the sole version source, and stdlib-Python manifest recipes embedded in `infra-generate`'s and `bootstrap-verifier`'s SKILL.md for writing/refreshing the target's `.infra-manifest.json`.
- No `memory-bank/` or `project-brain/` of its own - it seeds both for the target, from the verbatim runtime and skeleton bundled under `memory-seed/assets/` (kept byte-identical with the three PHP editions' `memory-bank/scripts/` and `project-brain/` sources).

## Bundled References

- `skill-forge/references/php-frameworks.md`, `php-architecture-patterns.md`, `php-integration-catalog.md`, `php-process-skills.md`, `php-specialty-skills.md`, `php-domain-behavior.md`.
- `profile-synthesizer/references/project-profile-schema.md`.
