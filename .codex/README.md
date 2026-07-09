# Native PHP Accelerator - Codex Edition

The Codex edition of the accelerator, laid out the way OpenAI Codex actually discovers things. Codex's model differs from Claude Code and Cursor, so the pieces live in three places:

| Piece | Location | Why |
|---|---|---|
| **Skills** (30 workflows) | `.agents/skills/<name>/SKILL.md` | Codex discovers repo skills from `.agents/skills`, not `.codex/`. |
| **Policy** | root `AGENTS.md` | Read natively by Codex (walked root -> cwd, concatenated). Shared with Claude/Cursor. |
| **Config** | `.codex/config.toml` | Project-scoped model/approval/sandbox/MCP + enables hooks. Loads only when the project is trusted. |
| **Hooks** | `.codex/hooks.json` + `.codex/hooks/*.sh` | Lifecycle hooks (same event schema as Claude Code). |
| **Reference docs** | `.codex/DOD.md`, `.codex/GOLDEN-PRINCIPLES.md`, `.codex/STABILIZATION.md` | Definition of Done, principles, error-to-rule process. |

## Key differences from the Cursor/Claude editions

- **No command layer.** Codex **deprecated and removed custom prompts / slash commands** (v0.117+). Skills are the superset replacement, so the `.claude/commands` and `.cursor/commands` entry points are **not** mirrored - you invoke a skill by name and Codex can also trigger it implicitly.
- **No subagents ported.** The `.claude/agents` wrappers just ran one skill; since Codex invokes skills directly, they add nothing here. (Codex does support subagents via `.agents/` + `agents.<name>.config_file` if you later want explicit delegation.)
- **Skills live in `.agents/skills`**, deliberately, because that is the path Codex loads.

## Setup for a Codex user

1. Open the repo with the Codex CLI or IDE extension.
2. **Trust the project** when prompted (project-scoped `.codex/` config, hooks, and rules load only for trusted projects).
3. Confirm skills are visible: type `/` (skills menu) or ask Codex to run one, e.g. "use the coder skill to ...".
4. Hooks require `features.hooks = true` (already set in `.codex/config.toml`).

## Relationship to `.claude/` and `.cursor/`

Each tool reads its own directory, so all three coexist without conflict:

- Claude Code -> `.claude/` (skills, agents, commands, `settings.json` hooks)
- Cursor -> `.cursor/` (skills, agents, commands, `hooks.json`)
- Codex -> `.agents/skills` + `.codex/` + root `AGENTS.md`

Codex does **not** auto-read `.claude/` or `.cursor/`, so there is no double-loading. When you change a skill, mirror the edit across the editions you support (or regenerate).

## Keeping in sync

`.agents/skills` was generated from `.claude/skills`; internal `.claude`/`.cursor` path references were rewritten to `.agents`/`.codex`. Hooks were copied and their event wiring translated to Codex's `hooks.json` schema.
