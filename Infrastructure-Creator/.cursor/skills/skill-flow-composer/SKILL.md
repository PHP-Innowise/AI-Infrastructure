---
name: skill-flow-composer
description: Build the target PHP project's own SKILL FLOW.md from the exact set of skills skill-forge actually generated - a Main Flow diagram, Shortcuts, a Phase Map table, and a Context Handoff section - never a template. Cross-references only point at generated skills. Takes a required target-project-path argument. Use after skill-forge (and ideally the other forges) have run. Triggers on "compose the skill flow", "build SKILL FLOW.md", "generate the target's flow map".
phase: generation
flow-next: bootstrap-verifier
flow-alternatives: [policy-forge, hook-forge, memory-seed]
related: [infra-generate, policy-forge, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, bootstrap-verifier]
---

# Skill Flow Composer

## Overview

`skill-flow-composer` writes the target project's navigational map -
`SKILL FLOW.md` - from the validated skill inventory and routing contracts.
It shows primary ownership, evidence-gated specialist entry points, explicit
sibling deferrals, and handoff artifacts; it never invents a universal chain
or routes every request through every available skill.

Consumes the validated **skill-forge log** and
`tasks/TASK-{N}/skill-generation-plan.json` as its source of truth, plus
profile section 1 for the selected editions.

## Generated File Naming Convention (MANDATORY)

For each selected edition, write `<edition-skills-dir>/SKILL FLOW.md` under the
required generation root (task staging during orchestration), where the edition
skills dir is `.claude/skills`, `.cursor/skills`, and/or `.agents/skills`.
Keep the evidence target path separate from the generation root.

## Process

1. **Require the target-project-path argument.** Resolve the selected editions from profile section 1 and their skills dirs.
2. **Read the validated contracts** to obtain the exact generated set,
   category, phase, positive/negative triggers, owned/excluded scope, nearest
   siblings, outputs, and declared relationships.
3. **Build the Main Flow** from primary core ownership only. Order applicable
   skills by phase and wire only declared, resolved handoffs. Do not place
   evidence-gated integration/domain specialists in a default path.
4. **Build Specialist Routing and Shortcuts.** For each optional specialist,
   state the concrete project trigger, exclusion, nearest sibling, and expected
   artifact. Every shortcut must have one primary owner or explicit ambiguity.
5. **Build the Phase Map table** mapping each phase to the generated skills that occupy it.
6. **Write the Context Handoff section** from each contract's output and
   evidence requirements so a fresh context can resume without loading
   unrelated project material.
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
- MUST derive routing and handoffs from the validated generation plan and MUST
  NOT make every generated specialist part of the default flow.
- MUST show positive and negative boundaries for adjacent skills; circular
  "use X for X" shortcuts fail.

## Final Output

Return the per-edition `SKILL FLOW.md` paths, the generated skill count reflected, confirmation that all cross-references resolve, the log path, and the next step (`bootstrap-verifier`).
