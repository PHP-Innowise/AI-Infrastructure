---
name: agent-forge
description: Generate one agent wrapper per skill that skill-forge produced in the target PHP project, for the selected edition(s) that carry an agent layer (Claude with full frontmatter, Cursor with reduced frontmatter). Each agent invokes exactly its one skill and stops. Skip Codex - it has no agent layer. Use once skill-forge has produced its log. Triggers on "forge agents", "generate agent wrappers", "wrap the target's skills as agents".
phase: generation
flow-next: command-forge
flow-alternatives: [command-forge, hook-forge, memory-seed]
related: [infra-generate, policy-forge, skill-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Agent Forge

## Overview

`agent-forge` wraps each semantically validated skill that `skill-forge`
generated into a single-purpose agent that runs that one skill in an isolated
context and then stops. It reads the validated skill log and that skill's
contract from `skill-generation-plan.json`; a wrapper is a routing surface, not
a second copy of the workflow. Its description and examples must make adjacent
skills distinguishable using the target's actual concerns.

Only two editions carry an agent layer: **Claude** (`.claude/agents/`, full frontmatter) and **Cursor** (`.cursor/agents/`, reduced frontmatter). **Codex has no agent layer** and is always skipped. The agent body is identical across the two editions; only the frontmatter differs.

Consumes: `tasks/TASK-{N}/skill-forge-log.md` (validated skill list),
`tasks/TASK-{N}/skill-generation-plan.json` schema **1.5** (`routing_cases`,
every `nearest_siblings[]` adjacency, `flow_contracts`, and write contracts),
and profile section **1** (selected editions). If the skill log does not record
a passing semantic validation result, stop; wrappers must never legitimize an
unvalidated skill set.

## Generated File Naming Convention (MANDATORY)

Into the required **generation root** (task staging during orchestration), for
each validated skill `<name>` and selected agent-carrying edition, write
`<edition-agents-dir>/<name>-agent.md`. Never write agents into `.codex`.

## Process

1. **Read the validated inventory.** Require the skill log's semantic-validation
   PASS and load the exact matching contract for every generated skill. Confirm
   selected editions from profile section 1. If neither Claude nor Cursor is
   selected, write nothing and report that no edition carries agents.
2. **Decide whether a wrapper adds routing value.** Create one agent per
   validated skill for the currently supported Claude/Cursor host model, but
   fail if its contract has no positive trigger, negative trigger, expected
   output, or distinct boundary for every adjacent skill. Never invent a
   wrapper for a skill absent from the validated log.
3. **Author the Claude agent** at `.claude/agents/<name>-agent.md` with frontmatter keys `name`, `description`, `model`, `invokes`, `phase`, and `writes` when write-capable:
   - `description` is a QUOTED selection sentence derived from the contract:
     when to use this agent, its target-specific owned concern, and the
     adjacent owners for excluded concerns. Keep it under ~250 characters.
     Circular descriptions such as "runs the X skill" or "use for work governed
     by X" are invalid.
   - Put at least one contract-derived positive example and one negative sibling
     deferral in the BODY under `## Selection examples`. Each example names a
     concrete project concern from the evidence ledger and explains the routing
     decision; repeating the skill name is not an explanation.
   - `model`: `opus` for heavy planning/architecture skills (architecture skill, security-review, performance, and evidence-gated domain-review skills); `sonnet` for the rest.
   - `invokes`: the exact skill name; `phase`: the skill's phase.
   - Set wrapper `writes: true` exactly when the validated contract's `writes`
     path list is non-empty; omit it when that list is empty. Do not infer it
     again from phase or prose.
4. **Author the Cursor agent** at `.cursor/agents/<name>-agent.md` with the SAME body but frontmatter reduced to `name`, `description`, and `writes` when the Claude agent carries it - Cursor runs the same gate and needs the same flag.
5. **Write the shared body** for both: `## Role` (bounded ownership and
   exclusions), `## Selection examples`, `## Instructions` (invoke exactly the
   skill and pass its evidence-bounded delegation capsule), `## Output Format`
   (the contract's expected result plus Context Summary/Next Steps), and
   `## Constraints`. Keep the operational procedure in the skill; do not copy
   it into the wrapper.
6. **Validate all adjacency and routing oracles.** Carry every
   `nearest_siblings[]` entry into the wrapper's negative deferrals; never
   collapse the list to one convenient sibling. Execute every schema 1.5
   `routing_cases[]` oracle. Each case must preserve its prompt, one primary
   owner, and the complete ordered deferred set. Fail on an omitted adjacency,
   an undeclared owner, or two agents claiming the same example without an
   explicit primary/deferred relationship.
7. **Log** every agent path, contract name, positive/negative routing example,
   complete adjacency list, routing-case result, expected output, and write
   flag to `agent-forge-log.md` (consumed by `command-forge` and
   `skill-flow-composer`).

## Output Template

```markdown
# Agent Forge Complete: [target_name]

**Editions with agents:** [Claude and/or Cursor] (Codex skipped - no agent layer)
**Agents generated:** [count] ([skill count] skills x [edition count] editions)
- [<name>-agent -> invokes <name> (model: opus|sonnet)]
- ...

## Log
tasks/TASK-{N}/agent-forge-log.md

## Next
command-forge (wrap these agents as commands); hook-forge/memory-seed if not already run.
```

## Guardrails

- MUST author one agent per skill in the skill-forge log; MUST NOT invent an agent for a skill that was not generated.
- MUST write agents ONLY into selected editions among {Claude, Cursor}; MUST NEVER write an agent into Codex.
- MUST give the Claude agent full frontmatter (`name`, `description` as a usage sentence with NO embedded `<example>` blocks - those belong in the body - `model`, `invokes`, `phase`) and the Cursor agent frontmatter reduced to `name`, `description`; both carry `writes: true` when the wrapped skill modifies repository files, and neither carries the key otherwise.
- MUST decide `writes` from what the wrapped skill writes, and MUST keep the flag identical for the same skill across both editions: the flag is what makes the generated accelerator's write serialization work at all.
- MUST set `model: opus` for heavy planning/architecture/domain-review skills and `sonnet` for the rest.
- MUST make each agent invoke exactly one skill and STOP - no auto-chaining.
- MUST keep the agent body identical across Claude and Cursor for the same skill.
- MUST derive routing and `writes` from the validated per-skill contract; MUST
  reject circular selection text, any omitted adjacent deferral, a routing-case
  mismatch, and ambiguous sibling ownership.
- MUST keep agents DRY: project-specific routing and expected output belong in
  the wrapper, while the complete procedure remains in the skill.

## Final Output

Return the editions written, the agent-per-skill list with each model choice, the log path, and the next step (`command-forge`).
