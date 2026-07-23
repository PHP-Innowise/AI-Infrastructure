# MANIFEST - Infrastructure-Creator Design

The living design record for the generator itself. `tasks/TASK-{N}/` holds per-run working docs; this file holds the durable design.

## Purpose

A standalone, PHP-only generator that scans a target PHP project and writes a bespoke accelerator into it, for only the AI tool(s) the target team selects. It is not itself a ready-to-use accelerator.

## Key Decisions

- **PHP-only.** Scanners and generated skills are specialized for PHP (`composer.json`, PHP frameworks, PHPUnit/Pest, PHPStan/Psalm, PHP-CS-Fixer/Pint/Rector). Non-PHP neighbors are captured only as integration contracts.
- **100% discovery-driven.** Every generated artifact is authored from evidence in the target; there is no bundled reference accelerator and no templating from any other project.
- **Fully independent.** No references to or dependencies on any other accelerator or sibling folder. All needed reference material is bundled under the skills.
- **Two-phase with a human checkpoint.** `infra-scan` produces one reviewable Project Profile; `infra-generate` consumes the approved profile. `infra-build` chains both, pausing only when necessary.
- **Tool-selected output.** `clarifying-interview` captures the AI-tool selection; generation produces only the selected edition(s).
- **The generator ships tripled.** It runs natively from Claude Code (`.claude/`), Cursor (`.cursor/`), and Codex (`.agents/skills` + `.codex/`).
- **Workspace boundary + collision guard.** It runs outside the target, takes the target path as a required argument, is read-only in Phase 1, and never overwrites a pre-existing accelerator without an explicit decision.
- **Self-adapting for non-PHP stacks.** `infra-scan` never silently fails on a non-PHP target: it probes for a recognizable non-PHP stack and, with explicit consent, hands off to `stack-adapter` to build an entirely independent sibling generator for that stack, rather than stretching this PHP-only generator beyond its domain.
- **Broad, evidence-gated target skill catalog.** `skill-forge` generates six skill groups per target - architecture; always-on design & interaction (`architecture-implementer`/`api-designer`/`database-designer`); a conditional frontend group; 14 always-on, framework-agnostic process/workflow skills; the 7 universal PHP skills; and an evidence-gated framework-specialty catalog (ORM patterns, migration safety, async/queue jobs, event/notification design, caching, storage, auth scaffolding, form/validator design, admin panel, console commands, repository/container review, test-data factories, package authoring) plus one skill per confirmed integration - so a generated target's depth approaches the hand-built Laravel/Symfony/PHP-Core accelerators without ever templating from them.

## Skill Inventory (21)

- Orchestration: `infra-scan`, `infra-generate`, `infra-build`, `stack-adapter`.
- Discovery: `stack-scanner`, `architecture-scanner`, `integration-scanner`, `infra-ops-scanner`, `security-compliance-scanner`, `conventions-scanner`.
- Research: `stack-researcher`.
- Synthesis: `clarifying-interview`, `profile-synthesizer`.
- Generation: `policy-forge`, `skill-forge`, `agent-forge`, `command-forge`, `hook-forge`, `memory-seed`, `skill-flow-composer`.
- Verification: `bootstrap-verifier`.

## Tech Stack (of the generator itself)

- Plain-markdown skill convention (`SKILL.md` with `name`/`description`/`phase`/`flow-next`/`flow-alternatives`/`related` frontmatter), edition-specific agent/command wrappers, POSIX-bash hooks, and dependency-free Python validators (`bootstrap-verifier/scripts/validate_generated.py`, and the bundled `memory-seed/assets/scripts/validate.py`).
- No `memory-bank/` of its own - it seeds one for the target.

## Bundled References

- `skill-forge/references/php-frameworks.md`, `php-architecture-patterns.md`, `php-integration-catalog.md`, `php-process-skills.md`, `php-specialty-skills.md`.
- `profile-synthesizer/references/project-profile-schema.md`.
