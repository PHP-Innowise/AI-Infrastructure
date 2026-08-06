# Laravel Accelerator - Cursor Edition

This directory is the **Cursor-native** copy of the accelerator that lives in `.cursor/`. It gives Cursor users the same skills, agents, commands, hooks, and policy without depending on Cursor's optional Claude-file loading.

## Layout

| Path | Purpose |
|---|---|
| `.cursor/skills/<name>/SKILL.md` | Workflows the agent executes (including `project-brain` and `memory-bank`). |
| `.cursor/agents/*.md` | Subagents that run one skill in isolation. |
| `.cursor/commands/*.md` | `/slash` entry points. Invoke as `/name`; context passes via `$ARGUMENTS`. |
| `.cursor/rules/*.mdc` | Always-on policy + Laravel/PHP standards (native Cursor rules). |
| `.cursor/hooks.json` + `.cursor/hooks/*.sh` | Session context, shell-command safety, file-naming, loop detection. |
| `.cursor/DOD.md`, `.cursor/GOLDEN-PRINCIPLES.md`, `.cursor/STABILIZATION.md` | Definition of Done, principles, error-to-rule process. |

Root `AGENTS.md` is the shared policy and is read automatically by Cursor.

## IMPORTANT: Avoid double-loading

Cursor can *also* read the `.claude/` folder when optional Claude-file loading is enabled.

- **Recommended:** keep that setting **OFF** and let this `.cursor/` copy be the single source of truth. Then nothing loads twice.
- If you turn it **ON** while both folders exist, skills and agents may appear twice and equivalent hooks may run twice. Pick one integration source per component.

## Keeping the two copies in sync

Shared skill behavior is declared canonically in `.agents/skills` for parity checks. Cursor keeps native wrappers under `.cursor/`; those wrappers use these transformations from the equivalent Claude integration:
- Command frontmatter converted to Cursor's `name` / `description` schema.
- Agent frontmatter reduced to Cursor-supported keys (dropped `model` / `invokes` / `phase`).
- Internal `.claude/` path references rewritten to `.cursor/`.
- Hooks translated to Cursor events: `SessionStart -> sessionStart`, `PreToolUse(Bash) -> beforeShellExecution`, `PreToolUse/PostToolUse(Write|Edit) -> afterFileEdit`. See `.cursor/hooks/README.md`.

When shared behavior changes, update `.agents/skills` first, then mirror the supported behavior into `.claude/skills` and `.cursor/skills` while preserving native wrappers and hook models.

## Usage

Type `/` in Cursor chat to see the commands (e.g. `/coder`, `/architect`, `/security-reviewer`, `/project-brain`, `/memory-bank`), or ask the agent to run a skill by name. Start big/ambiguous work with `/brainstorm` and follow the flow in `.cursor/rules/accelerator-workflow.mdc`.

`/project-brain` manages governed shared tasks, handoffs, all six governed record types, unified retrieval, compaction, and promotion proposals. Governed mode is the default; `--mode lightweight` is explicit and local-only. `/memory-bank` is limited to durable retrieval/capture/audit/supersession and applying governed automatic or independently reviewed promotions.

The one public task-aware retrieval command is `python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID`. Canonical project sources outrank all retrieved context. Session hooks report metadata only and never index or inject records automatically. Keep Cursor's optional Claude-file loading disabled so native commands, agents, skills, and hooks are not duplicated.
