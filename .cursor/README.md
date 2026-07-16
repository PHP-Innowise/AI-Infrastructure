# Symfony Layered Architecture Accelerator - Cursor Edition

This directory is the **Cursor-native** copy of the accelerator that lives in `.claude/`. It gives Cursor users the same skills, agents, commands, hooks, and policy without depending on Cursor's opt-in "read third-party (`.claude`) files" setting.

## Layout

| Path | Purpose |
|---|---|
| `.cursor/skills/<name>/SKILL.md` | Symfony workflows the agent executes (42 skills). |
| `.cursor/agents/*.md` | Subagents that run one skill in isolation. |
| `.cursor/commands/*.md` | `/slash` entry points. Invoke as `/name`; context passes via `$ARGUMENTS`. |
| `.cursor/rules/*.mdc` | Always-on policy + PHP standards (native Cursor rules). |
| `.cursor/hooks.json` + `.cursor/hooks/*.sh` | Session context, shell-command safety, file-naming, loop detection. |
| `.cursor/DOD.md`, `.cursor/GOLDEN-PRINCIPLES.md`, `.cursor/STABILIZATION.md` | Definition of Done, principles, and error-to-rule process. |

Root `AGENTS.md` is the shared policy and is read automatically by Cursor.

## IMPORTANT: Avoid double-loading

Cursor can *also* read the `.claude/` folder directly, but only if you enable the setting **Cursor Settings -> Rules & Memories -> "Include `.claude` files"** (off by default).

- **Recommended:** keep that setting **OFF** and let this `.cursor/` copy be the single source of truth. Then nothing loads twice.
- If you turn it **ON** while both folders exist, skills/agents will appear **twice** and the hooks will **fire twice** (Cursor runs both `.claude/settings.json` and `.cursor/hooks.json`). Pick one source per component.

## Keeping the two copies in sync

`.cursor/` was generated from `.claude/` with these transforms:
- Command frontmatter converted to Cursor's `name` / `description` schema.
- Agent frontmatter reduced to Cursor-supported keys (dropped `model` / `invokes` / `phase`).
- Internal `.claude/` path references rewritten to `.cursor/`.
- Hooks translated to Cursor events: `SessionStart -> sessionStart`, `PreToolUse(Bash) -> beforeShellExecution`, `PreToolUse/PostToolUse(Write|Edit) -> afterFileEdit`. See `.cursor/hooks/README.md`.
- Tool-integrated workflows such as `skill-creator` use Cursor-native capabilities instead of Claude CLI helpers.

When you change one side, mirror the shared Symfony behavior on the other while preserving these native integration differences.

## Usage

Type `/` in Cursor chat to see the commands (e.g. `/coder`, `/architect`, `/security-reviewer`), or ask the agent to run a skill by name. Start big/ambiguous work with `/brainstorm` and follow the flow in `.cursor/rules/accelerator-workflow.mdc`.

The baseline supports Symfony 7.4 LTS on PHP 8.2+ and Symfony 8.1 on PHP 8.4+. Always follow the consuming project's declared versions and installed components.
