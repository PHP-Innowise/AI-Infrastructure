---
name: command-forge
description: Generate one slash command per agent that agent-forge wrapped in the target PHP project, for the selected edition(s) that carry a command layer (Claude with spawns + flow keys, Cursor with name + description). Skip Codex entirely - it has no command layer. Use once agent-forge has produced its log. Triggers on "forge commands", "generate slash commands", "wrap the target's agents as commands".
phase: generation
flow-next: hook-forge
flow-alternatives: [memory-seed, skill-flow-composer]
related: [infra-generate, policy-forge, skill-forge, agent-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Command Forge

## Overview

`command-forge` writes thin user-facing commands for validated agents and
project-aware flow commands from their routing contracts. Per-agent commands
dispatch; they do not duplicate skill logic. Flows are adaptive: the main
conversation selects a specialist only when the current request matches that
agent's positive trigger and does not match its exclusions.

Only two editions carry a command layer: **Claude** (`.claude/commands/`, `spawns` + flow keys) and **Cursor** (`.cursor/commands/`, `name` + `description`). **Codex has no command layer** and is always skipped. A command's sole job is to spawn its matching `<name>-agent`, which in turn invokes the one skill.

Consumes: `tasks/TASK-{N}/agent-forge-log.md` (validated agent/routing list),
`tasks/TASK-{N}/skill-generation-plan.json` schema **1.3** (scope,
`routing_cases`, and canonical `flow_contracts`), and profile section **1**
(selected editions). `flow_contracts` is the sole source for the flow roster,
stage order, phases, agents, checkpoints, required code review, and writer
serialization; prose and existing generated files are not alternative sources.

## Generated File Naming Convention (MANDATORY)

Into the required **generation root** (task staging during orchestration), for
each validated wrapped agent and selected command-carrying edition, write
`<edition-commands-dir>/<name>.md`. Never write commands into `.codex`.

## Process

1. **Read the agent-forge log** to get the exact set of agents; read profile section 1 to confirm which of {Claude, Cursor} were selected. If neither is selected, write nothing and report that no edition carries commands.
2. **For each wrapped agent, one command per selected edition.** Never create a command for an agent absent from the log.
3. **Author the Claude command** at `.claude/commands/<name>.md` with frontmatter keys ONLY: `spawns` (the `<name>-agent`), `phase`, `flow-next`, `flow-alternatives`. NO `name`/`description`. Derive `phase` and flow references from the validated contract and generated skill.
4. **Author the Cursor command** at `.cursor/commands/<name>.md` with frontmatter keys ONLY: `name`, `description`. Derive the description from the agent's positive trigger plus its adjacent exclusions, not merely the agent name. The body is a short `/name` usage note passing context as `$ARGUMENTS`.
5. **Keep both commands thin** - a command spawns exactly its one agent and stops; it embeds no skill logic of its own.
6. **Compile all routing cases.** From the validated contracts, classify
   generated agents as core delivery roles or evidence-gated specialists.
   Record positive triggers, exclusions, evidence scope, phase, `writes`, and
   every adjacency for each agent. Execute every schema 1.3
   `routing_cases[]` entry as an oracle and require its one primary owner and
   complete ordered deferred set. Reject a missing adjacency, a singular
   sibling projection, or duplicate trigger ownership without explicit
   primary/deferred precedence.
7. **Compile executable flows from the canonical graph.** Author exactly the
   flows named by `flow_contracts.flows[]` (normally `flow-feature` and
   `flow-review`) in each selected command-carrying edition. A flow is the ONE
   command class that spawns more than one agent: the MAIN conversation
   executes it as orchestrator. Do not independently design a command flow:
   - Require `flow_contracts.roster[]` to equal the generated agent roster in
     skill-plan order, with exact `{skill, agent, phase, writes}` values.
   - Frontmatter `flow` and ordered `stages` compile byte-for-value from the
     matching graph entry. Every stage has exactly `{phase, agents, parallel,
     checkpoint}`. Phases use `understanding`, `planning`, `implementation`,
     `verification`, or `finalization`.
   - Add exactly one fenced `json flow-contract` block containing
     `{"roster": <canonical roster>, "flow": <matching canonical flow>}`. This
     machine-readable block is generated, not hand-maintained.
   - `flow-feature.required_code_review` MUST be `code-review-agent`, and that
     agent MUST occur in an ordered verification stage after implementation.
     Omitting review is a compile failure, not an adaptive routing choice.
   - `flow-feature` uses only applicable core roles in its declared stages:
     understand, plan, explicit approval, one write-capable implementation role
     at a time, targeted verification/review, and finalization. Its body carries
     a generated **Specialist Routing** section: inspect the request/change scope
     against each specialist contract and spawn only matching specialists.
     Architecture, API, database, integration, and domain agents are never
     unconditional merely because they exist.
     The **Specialist Routing** section lists every generated specialist agent,
     its positive trigger, negative exclusion, and nearest deferral. These are
     candidate branches in the body, never unconditional frontmatter stages.
   - `flow-review` first establishes the changed scope, then selects only
     read-only reviewers whose owned scope intersects that change. General code
     review may be the fallback; integration/domain reviewers are conditional.
     Synthesize selected outputs into one deduplicated report and state which
     available lenses were skipped and why.
     A specialist whose contract has a non-empty `writes` list is explicitly
     marked `write-capable; skip review` and is never spawned by `flow-review`.
   - Body: instruct the main conversation to build one bounded delegation capsule per spawn (objective, output format, tool and source guidance, task boundaries, decisions-and-assumptions so far), record it with `python3 memory-bank/scripts/context.py msg-dispatch --task-id <ID> --agent <name> --event spawn --capsule-file <file>` (which refuses an under-specified capsule), spawn with the Task tool, then record progress with `context.py update --task-id <ID> --actor <name>`. Completions are recorded by the `subagent-dispatch.sh` hook, so the flow must NOT record them a second time.
   - Rules to state in every flow: spawn only agents from the canonical roster;
     a `parallel: true` stage may hold at most one roster entry carrying
     `writes: true`; preserve graph order; stop at every checkpoint for explicit
     user approval (a multi-stage flow MUST declare at least one); stop and
     report a failed stage rather than retrying silently.
8. **Compiled-graph validation is orchestrator-owned - do NOT run it here.**
   `skill-flow-composer` runs after command-forge, so `SKILL FLOW.md` does not
   exist yet when this skill finishes; invoking `validate_flow_contracts.py`
   now would fail on the missing artifact instead of validating anything.
   After `skill-flow-composer` writes its artifact, `infra-generate` step 8
   runs that validator against each selected command-carrying edition, and any
   graph, order, phase, agent, checkpoint, required-review, roster, writer
   serialization, adjacency, or routing-oracle mismatch blocks publication.
   In this skill's report, record compiled-graph validation as pending that
   orchestrator gate.
9. **Log** every command path and agent plus each flow's core stages,
   conditional specialist lanes, skipped agents with reasons, and routing test
   results to `command-forge-log.md`.

## Output Template

```markdown
# Command Forge Complete: [target_name]

**Editions with commands:** [Claude and/or Cursor] (Codex skipped - no command layer)
**Commands generated:** [count] ([agent count] agents x [edition count] editions)
- [/<name> -> spawns <name>-agent]
- ...

## Log
tasks/TASK-{N}/command-forge-log.md

## Next
hook-forge; memory-seed/skill-flow-composer if not already run. Compiled-graph
validation stays pending until the orchestrator runs
`validate_flow_contracts.py` after `skill-flow-composer`.
```

## Guardrails

- MUST author one command per agent in the agent-forge log; MUST NOT invent a command for an agent that was not generated.
- MUST write commands ONLY into selected editions among {Claude, Cursor}; MUST NEVER write a command into Codex.
- MUST give the per-agent Claude command only `spawns`, `phase`, `flow-next`, `flow-alternatives` (no `name`/`description`), and the per-agent Cursor command only `name`, `description`. Flow commands are the sole exception: in every selected command-carrying edition they carry `flow` + ordered `stages` frontmatter and one fenced `json flow-contract` block, per step 7 and the flow guardrail below.
- MUST make each per-agent command spawn exactly its one `<name>-agent` and carry no skill logic.
- MUST generate exactly the canonical `flow_contracts.flows[]` for every
  selected command-carrying edition, composed ONLY of canonical roster agents,
  with `flow` + ordered `stages` frontmatter instead of `spawns`.
- MUST NOT place TWO agents carrying `writes: true` in the same `parallel: true` stage: the gate runs write-capable agents one at a time, so the stage would block itself. One write-capable agent beside read-only agents is fine.
- MUST NOT let a flow spawn agents outside the generated roster, skip a declared checkpoint, or re-record completions the `subagent-dispatch.sh` hook already writes.
- MUST derive flow keys from the wrapped skill's frontmatter so the generated flow graph stays consistent.
- MUST compile SKILL FLOW and executable flows from the same schema 1.3
  `flow_contracts`; MUST NOT infer, reorder, omit, or independently rewrite a
  stage, checkpoint, required code-review agent, roster entry, or write flag.
- MUST NOT place every available design, integration, or domain agent into a
  default flow. Specialist selection is task-scope-driven and must cite the
  matching routing contract.
- MUST fail on circular command descriptions or routing fixtures with two
  undeclared primary owners.

## Final Output

Return the editions written, the command-per-agent list, the log path, and the next step (`hook-forge`).
