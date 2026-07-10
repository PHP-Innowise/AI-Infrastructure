# Laravel Accelerator - Cursor Edition

This directory is the **Cursor-native** copy of the accelerator that lives in `.cursor/`. It gives Cursor users the same skills, agents, commands, hooks, and policy without depending on Cursor's opt-in "read third-party (`.cursor`) files" setting.

## Layout

| Path | Purpose |
|---|---|
| `.cursor/skills/<name>/SKILL.md` | Workflows the agent executes (31 skills). |
| `.cursor/agents/*.md` | Subagents that run one skill in isolation. |
| `.cursor/commands/*.md` | `/slash` entry points. Invoke as `/name`; context passes via `$ARGUMENTS`. |
| `.cursor/rules/*.mdc` | Always-on policy + Laravel/PHP standards (native Cursor rules). |
| `.cursor/hooks.json` + `.cursor/hooks/*.sh` | Session context, shell-command safety, file-naming, loop detection. |
| `.cursor/DOD.md`, `.cursor/GOLDEN-PRINCIPLES.md`, `.cursor/STABILIZATION.md` | Definition of Done, principles, error-to-rule process. |

Root `AGENTS.md` is the shared policy and is read automatically by Cursor.

## IMPORTANT: Avoid double-loading

Cursor can *also* read the `.cursor/` folder directly, but only if you enable the setting **Cursor Settings -> Rules & Memories -> "Include `.cursor` files"** (off by default).

- **Recommended:** keep that setting **OFF** and let this `.cursor/` copy be the single source of truth. Then nothing loads twice.
- If you turn it **ON** while both folders exist, skills/agents will appear **twice** and the hooks will **fire twice** (Cursor runs both `.cursor/settings.json` and `.cursor/hooks.json`). Pick one source per component.

## Keeping the two copies in sync

`.cursor/` was generated from `.cursor/` with these transforms:
- Command frontmatter converted to Cursor's `name` / `description` schema.
- Agent frontmatter reduced to Cursor-supported keys (dropped `model` / `invokes` / `phase`).
- Internal `.cursor/` path references rewritten to `.cursor/`.
- Hooks translated to Cursor events: `SessionStart -> sessionStart`, `PreToolUse(Bash) -> beforeShellExecution`, `PreToolUse/PostToolUse(Write|Edit) -> afterFileEdit`. See `.cursor/hooks/README.md`.

When you change one side, mirror the edit on the other (or regenerate `.cursor/` from `.cursor/`).

## Usage

Type `/` in Cursor chat to see the commands (e.g. `/coder`, `/architect`, `/security-reviewer`), or ask the agent to run a skill by name. Start big/ambiguous work with `/brainstorm` and follow the flow in `.cursor/rules/accelerator-workflow.mdc`.
