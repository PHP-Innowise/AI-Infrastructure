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

Consumes the validated **skill-forge log** and schema **1.5**
`tasks/TASK-{N}/skill-generation-plan.json` as its source of truth, including
all `routing_cases[]` and the canonical `flow_contracts`, plus profile section
1 for the selected editions. The canonical graph is shared with
`command-forge`; `SKILL FLOW.md` and executable flow commands are two compiled
views of that one graph.

## Generated File Naming Convention (MANDATORY)

For each selected edition, write `<edition-skills-dir>/SKILL FLOW.md` under the
required generation root (task staging during orchestration), where the edition
skills dir is `.claude/skills`, `.cursor/skills`, and/or `.agents/skills`.
Keep the evidence target path separate from the generation root.

## Process

1. **Require the target-project-path argument.** Resolve the selected editions from profile section 1 and their skills dirs.
2. **Read the validated contracts** to obtain the exact generated set,
   category, phase, positive/negative triggers, owned/excluded scope, every
   adjacency, routing oracle, output, and declared relationship. Reject a
   schema 1.0/1.1/1.2/1.3/1.4 plan; flow compilation requires schema 1.5.
3. **Compile the Main Flow from `flow_contracts`.** Preserve canonical roster
   order and each flow's ordered `{phase, agents, parallel, checkpoint}` stages
   exactly. Render skill names for navigation, but never infer a second graph
   from skill frontmatter or prose. Do not place evidence-gated
   integration/domain specialists in a default path unless the canonical graph
   does so.
4. **Build Specialist Routing and Shortcuts.** For each optional specialist,
   state the concrete project trigger, exclusions, every adjacent owner, and
   expected artifact. Render every `routing_cases[]` oracle with its prompt,
   one primary owner, and complete ordered deferred set. Every shortcut must
   have one primary owner or explicit ambiguity.
5. **Build the Phase Map table** mapping each phase to the generated skills that occupy it.
6. **Write the Context Handoff section** from each contract's output and
   evidence requirements so a fresh context can resume without loading
   unrelated project material.
7. **Embed the canonical graph.** Add exactly one `## Canonical Flow Graph`
   section containing one fenced `json flow-contract` block whose JSON value is
   exactly the plan's `flow_contracts` object. This is a deterministic compiled
   record, not a second authoring surface.
8. **Validate cross-references and flow semantics:** every skill/agent named
   anywhere in `SKILL FLOW.md` MUST resolve through the canonical roster.
   Require the same order, phases, agents, checkpoints, required
   `code-review-agent`, roster, and writer serialization as executable flow
   commands. Fail fast on a dangling reference or mismatch.
9. **Write** `SKILL FLOW.md` into each selected edition's skills dir and log the paths.

## Output Template

````markdown
# Skill Flow: [target_name]

## Main Flow
[dynamic diagram of generated skills, phase-ordered, edges only among generated skills]

## Canonical Flow Graph
```json flow-contract
[exact schema 1.5 flow_contracts object]
```

## Shortcuts
- [entry point -> generated skill]

## Phase Map
| Phase | Skills (generated) |
|-------|--------------------|
| [phase] | [skill list] |

## Context Handoff
- [skill] -> [next]: [artifact/log/memory handed off]
````

## Guardrails

- MUST assemble the flow from the skill-forge log's real generated set - never a static template.
- MUST ensure every skill referenced in any section resolves to a generated skill; fail on a dangling reference.
- MUST require and use the target-project-path argument to locate edition skills dirs.
- MUST write `SKILL FLOW.md` into EACH selected edition's skills dir, and only selected ones.
- MUST reflect each skill's real declared `phase`/`flow-next` when wiring edges.
- MUST derive routing and handoffs from the validated generation plan and MUST
  NOT make every generated specialist part of the default flow.
- MUST preserve every adjacency and every schema 1.5 routing oracle; circular
  "use X for X" shortcuts and singular-sibling projections fail.
- MUST compile the human navigation and canonical JSON block from the same
  `flow_contracts` consumed by `command-forge`; independent flow authoring is
  forbidden.

## Final Output

Return the per-edition `SKILL FLOW.md` paths, the generated skill count reflected, confirmation that all cross-references resolve, the log path, and the next step (`bootstrap-verifier`).
