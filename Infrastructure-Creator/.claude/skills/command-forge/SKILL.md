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

`command-forge` writes one `/slash` command per agent that `agent-forge` produced, giving each generated skill a user-facing entry point. It reads the `agent-forge-log.md` (the authoritative agent list) and profile section 1 (selected editions), and writes one command file per agent into every selected edition that carries a command layer. It authors from the log only - it never invents a command for an agent that was not generated.

Only two editions carry a command layer: **Claude** (`.claude/commands/`, `spawns` + flow keys) and **Cursor** (`.cursor/commands/`, `name` + `description`). **Codex has no command layer** and is always skipped. A command's sole job is to spawn its matching `<name>-agent`, which in turn invokes the one skill.

Consumes: `tasks/TASK-{N}/agent-forge-log.md` (agent list) and profile section **1** (selected editions).

## Generated File Naming Convention (MANDATORY)

Into the target, for each wrapped agent `<name>-agent` and each selected command-carrying edition, write `<edition-commands-dir>/<name>.md` where the commands dir is `.claude/commands` and/or `.cursor/commands`. Never write commands into `.codex`. Append a generation log to `tasks/TASK-{N}/command-forge-log.md` listing every command produced and the agent it spawns.

## Process

1. **Read the agent-forge log** to get the exact set of agents; read profile section 1 to confirm which of {Claude, Cursor} were selected. If neither is selected, write nothing and report that no edition carries commands.
2. **For each wrapped agent, one command per selected edition.** Never create a command for an agent absent from the log.
3. **Author the Claude command** at `.claude/commands/<name>.md` with frontmatter keys ONLY: `spawns` (the `<name>-agent`), `phase`, `flow-next`, `flow-alternatives`. NO `name`/`description`. Derive `phase`/`flow-next`/`flow-alternatives` from the wrapped skill's own frontmatter so the flow graph stays consistent.
4. **Author the Cursor command** at `.cursor/commands/<name>.md` with frontmatter keys ONLY: `name`, `description`. The body is a short `/name` usage note passing context as `$ARGUMENTS`.
5. **Keep both commands thin** - a command spawns exactly its one agent and stops; it embeds no skill logic of its own.
6. **Author the two flow commands** (`.claude/commands/flow-feature.md`, `.claude/commands/flow-review.md`, plus the Cursor copies with `name` + `description` frontmatter). A flow is the ONE command class that spawns more than one agent: the MAIN conversation executes it as orchestrator. Compose each flow from the agents this run actually generated - never name an agent absent from the log:
   - Frontmatter: `flow` (the flow name) and `stages`, a list of `{ phase, agents, parallel, checkpoint }` entries. Phases use the runtime's stored vocabulary (`understanding`, `planning`, `implementation`, `verification`, `finalization`).
   - `flow-feature` walks the target's own delivery loop: a requirements/analysis agent, then design/planning agents, then a planning agent whose output is the plan, then a **checkpoint** where the user approves it, then the implementation agent(s), then the test agent, then the review agents in one `parallel: true` stage, then verification, then a finalization checkpoint. Drop any stage the target has no agent for and say so in the log.
   - `flow-review` is a single `parallel: true` stage of the target's read-only review agents, synthesized into one deduplicated report by the main conversation.
   - Body: instruct the main conversation to build one bounded delegation capsule per spawn (objective, output format, tool and source guidance, task boundaries, decisions-and-assumptions so far), record it with `python3 memory-bank/scripts/context.py msg-dispatch --task-id <ID> --agent <name> --event spawn --capsule-file <file>` (which refuses an under-specified capsule), spawn with the Task tool, then record progress with `context.py update --task-id <ID> --actor <name>`. Completions are recorded by the `subagent-dispatch.sh` hook, so the flow must NOT record them a second time.
   - Rules to state in every flow: spawn only agents from this accelerator's roster; a `parallel: true` stage may hold at most one agent carrying `writes: true`; stop at every checkpoint for explicit user approval - a multi-stage flow MUST declare at least one; stop and report a failed stage rather than retrying silently.
7. **Log** every command path and the agent it spawns - and, for each flow, its stage list and any stage dropped for lack of an agent - to `command-forge-log.md` (consumed by `skill-flow-composer` and `bootstrap-verifier`).

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
hook-forge; memory-seed/skill-flow-composer if not already run.
```

## Guardrails

- MUST author one command per agent in the agent-forge log; MUST NOT invent a command for an agent that was not generated.
- MUST write commands ONLY into selected editions among {Claude, Cursor}; MUST NEVER write a command into Codex.
- MUST give the Claude command only `spawns`, `phase`, `flow-next`, `flow-alternatives` (no `name`/`description`), and the Cursor command only `name`, `description`.
- MUST make each per-agent command spawn exactly its one `<name>-agent` and carry no skill logic.
- MUST generate `flow-feature` and `flow-review` for every selected command-carrying edition, composed ONLY of agents present in the agent-forge log, with `flow` + `stages` frontmatter instead of `spawns`.
- MUST NOT place TWO agents carrying `writes: true` in the same `parallel: true` stage: the gate runs write-capable agents one at a time, so the stage would block itself. One write-capable agent beside read-only agents is fine.
- MUST NOT let a flow spawn agents outside the generated roster, skip a declared checkpoint, or re-record completions the `subagent-dispatch.sh` hook already writes.
- MUST derive flow keys from the wrapped skill's frontmatter so the generated flow graph stays consistent.

## Final Output

Return the editions written, the command-per-agent list, the log path, and the next step (`hook-forge`).
