---
name: skill-flow-composer
description: Build the target PHP project's own SKILL FLOW.md from the exact set of skills skill-forge actually generated - a Main Flow diagram, Shortcuts, a Phase Map table, and a Context Handoff section - assembled dynamically from the real generated set, never a template. Cross-references only point at generated skills. Takes a required target-project-path argument. Use after skill-forge (and ideally the other forges) have run. Triggers on "compose the skill flow", "build SKILL FLOW.md", "generate the target's flow map".
phase: generation
flow-next: bootstrap-verifier
flow-alternatives: [policy-forge, hook-forge, memory-seed]
related: [infra-generate, policy-forge, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, bootstrap-verifier]
---

# Skill Flow Composer

## Overview

`skill-flow-composer` writes the target project's navigational map - `SKILL FLOW.md` - describing how the generated skills chain together for that specific project. It is assembled dynamically from the skills `skill-forge` actually produced (read from the skill-forge log), so the flow reflects the target's real architecture skill, universal PHP skills, and one skill per confirmed integration - never a fixed template. Every arrow, shortcut, and table row references only a skill that exists in the generated set. The composed `SKILL FLOW.md` is written into each selected edition's skills directory so each edition is independently navigable.

Consumes the **skill-forge log** (`tasks/TASK-{N}/skill-forge-log.md`) as its source of truth, plus profile section 1 for the selected editions.

## Generated File Naming Convention (MANDATORY)

For each selected edition, write `<edition-skills-dir>/SKILL FLOW.md` (literal filename with a space), where the edition skills dir is `.claude/skills`, `.cursor/skills`, and/or `.agents/skills` (Codex). Requires a target-project-path argument to resolve those dirs. Append a log to `tasks/TASK-{N}/skill-flow-composer-log.md`.

## Process

1. **Require the target-project-path argument.** Resolve the selected editions from profile section 1 and their skills dirs.
2. **Read the skill-forge log** to obtain the EXACT generated skill set (architecture skill, universal skills, integration skills) and each skill's declared `phase`/`flow-next`.
3. **Build the Main Flow** as a diagram that orders the generated skills by phase, wiring each skill to the successor it actually declares - only among generated skills. Drop any edge whose target was not generated.
4. **Build Shortcuts** - the common jump-in entry points (e.g. straight to the coding or review skill) using only generated skill names.
5. **Build the Phase Map table** mapping each phase to the generated skills that occupy it.
6. **Write the Context Handoff section** describing what each step hands the next (artifacts, logs, memory-bank chunks) so a fresh context can resume mid-flow.
7. **Validate cross-references:** every skill named anywhere in `SKILL FLOW.md` MUST exist in the generated set; fail fast on a dangling reference rather than emitting it.
8. **Write** `SKILL FLOW.md` into each selected edition's skills dir and log the paths.

## Output Template

```markdown
# Skill Flow: [target_name]

## Main Flow
[dynamic diagram of generated skills, phase-ordered, edges only among generated skills]

## Shortcuts
- [entry point -> generated skill]

## Phase Map
| Phase | Skills (generated) |
|-------|--------------------|
| [phase] | [skill list] |

## Context Handoff
- [skill] -> [next]: [artifact/log/memory handed off]
```

## Guardrails

- MUST assemble the flow from the skill-forge log's real generated set - never a static template.
- MUST ensure every skill referenced in any section resolves to a generated skill; fail on a dangling reference.
- MUST require and use the target-project-path argument to locate edition skills dirs.
- MUST write `SKILL FLOW.md` into EACH selected edition's skills dir, and only selected ones.
- MUST reflect each skill's real declared `phase`/`flow-next` when wiring edges.

## Final Output

Return the per-edition `SKILL FLOW.md` paths, the generated skill count reflected, confirmation that all cross-references resolve, the log path, and the next step (`bootstrap-verifier`).
