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

`agent-forge` wraps each skill that `skill-forge` generated into a single-purpose agent that runs that one skill in an isolated context and then stops. It reads the `skill-forge-log.md` (the authoritative list of skills this run produced) and profile section 1 (which editions were selected), and writes one agent file per skill into every selected edition that carries an agent layer. It authors from the log only - it never invents an agent for a skill that was not generated.

Only two editions carry an agent layer: **Claude** (`.claude/agents/`, full frontmatter) and **Cursor** (`.cursor/agents/`, reduced frontmatter). **Codex has no agent layer** and is always skipped. The agent body is identical across the two editions; only the frontmatter differs.

Consumes: `tasks/TASK-{N}/skill-forge-log.md` (skill list) and profile section **1** (selected editions).

## Generated File Naming Convention (MANDATORY)

Into the target, for each generated skill `<name>` and each selected agent-carrying edition, write `<edition-agents-dir>/<name>-agent.md` where the agents dir is `.claude/agents` and/or `.cursor/agents`. Never write agents into `.codex`. Append a generation log to `tasks/TASK-{N}/agent-forge-log.md` listing every agent produced and the skill it wraps.

## Process

1. **Read the skill-forge log** to get the exact set of generated skills; read profile section 1 to confirm which of {Claude, Cursor} were selected. If neither is selected, write nothing and report that no edition carries agents.
2. **For each generated skill, one agent per selected edition.** Never create an agent for a skill absent from the log.
3. **Author the Claude agent** at `.claude/agents/<name>-agent.md` with frontmatter keys `name`, `description`, `model`, `invokes`, `phase`:
   - `description` is a QUOTED string that embeds the usage sentence plus one or more `<example>...</example>` blocks (user request -> why this agent fires).
   - `model`: `opus` for heavy planning/architecture skills (architecture skill, security-review, performance); `sonnet` for the rest.
   - `invokes`: the exact skill name; `phase`: the skill's phase.
4. **Author the Cursor agent** at `.cursor/agents/<name>-agent.md` with the SAME body but frontmatter REDUCED to only `name` and `description`.
5. **Write the shared body** for both: `## Role`, `## Instructions` (use the Skill tool to invoke `<skill>`, execute it fully, then STOP - do not chain), `## Output Format` (Context Summary + Next Steps), `## Constraints`.
6. **Log** every agent path and its wrapped skill to `agent-forge-log.md` (consumed by `command-forge` and `skill-flow-composer`).

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
- MUST give the Claude agent full frontmatter (`name`, `description` with embedded `<example>` blocks, `model`, `invokes`, `phase`) and the Cursor agent frontmatter reduced to only `name`, `description`.
- MUST set `model: opus` for heavy planning/architecture skills and `sonnet` for the rest.
- MUST make each agent invoke exactly one skill and STOP - no auto-chaining.
- MUST keep the agent body identical across Claude and Cursor for the same skill.

## Final Output

Return the editions written, the agent-per-skill list with each model choice, the log path, and the next step (`command-forge`).
