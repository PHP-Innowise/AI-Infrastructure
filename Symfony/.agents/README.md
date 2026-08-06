# Symfony Accelerator Skills For Codex

Codex discovers repository skills from `.agents/skills/<name>/SKILL.md`.

These skills are the configured canonical source for shared Symfony skill parity. Their supported behavior is mirrored into `.claude/skills` and `.cursor/skills`, while tool-specific mechanics such as `skill-creator` remain native to each integration. Supporting Codex configuration, hooks, and engineering references live in `.codex`; enforceable shared policy lives in root `AGENTS.md`.

When updating a workflow:

1. Change the canonical skill in `.agents/skills`.
2. Mirror its supported behavior into `.claude/skills` and `.cursor/skills`.
3. Rewrite tool-specific paths and mechanics without changing the shared Symfony workflow contract.
4. Compare inventories and validate internal references.
5. Run the active edition's Definition of Done.

Do not create duplicate skills under `.codex/skills`.

Invoke Codex skills by their discovered names, such as `brainstorming`, `systematic-debugger`, `documentation-generator`, `project-brain`, `memory-bank`, and `using-git-worktrees`. Claude/Cursor slash-command aliases such as `/brainstorm`, `/debugger`, `/docs-generator`, `/project-brain`, `/memory-bank`, and `/git-worktrees` are not Codex skill names. Do not add fake Codex command or agent wrappers.

The `project-brain` skill owns governed shared task lifecycle, handoffs, the unified `python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID` facade, governed records, compaction, and promotion proposals. Governed mode is the default; `--mode lightweight` is explicitly local-only.

The `memory-bank` skill owns durable retrieval/capture/audit/supersession and application of governed automatic or independently reviewed promotions. Session hooks report metadata only—mode, index health/staleness, active binding count, and validation status—and never index, retrieve, load, or inject record contents automatically.
