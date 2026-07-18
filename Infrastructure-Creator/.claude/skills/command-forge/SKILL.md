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
6. **Log** every command path and the agent it spawns to `command-forge-log.md` (consumed by `skill-flow-composer` and `bootstrap-verifier`).

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
- MUST make each command spawn exactly its one `<name>-agent` and carry no skill logic.
- MUST derive flow keys from the wrapped skill's frontmatter so the generated flow graph stays consistent.

## Final Output

Return the editions written, the command-per-agent list, the log path, and the next step (`hook-forge`).
