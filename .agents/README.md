# Symfony Accelerator Skills For Codex

Codex discovers repository skills from `.agents/skills/<name>/SKILL.md`.

These skills are the Codex-native mirror of the canonical Symfony workflows in `.claude/skills`. Shared workflow contracts stay aligned, while tool-specific mechanics such as `skill-creator` use Codex-native capabilities. Supporting Codex configuration, hooks, and engineering references live in `.codex`; enforceable shared policy lives in root `AGENTS.md`.

When updating a workflow:

1. Change the canonical Claude skill.
2. Mirror its behavior into `.cursor/skills` and `.agents/skills`.
3. Rewrite tool-specific paths and mechanics without changing the shared Symfony workflow contract.
4. Compare inventories and validate internal references.
5. Run the active edition's Definition of Done.

Do not create duplicate skills under `.codex/skills`.

Invoke Codex skills by their discovered names, such as `brainstorming`, `systematic-debugger`, `documentation-generator`, and `using-git-worktrees`. Claude/Cursor slash-command aliases such as `/brainstorm`, `/debugger`, `/docs-generator`, and `/git-worktrees` are not Codex skill names.
