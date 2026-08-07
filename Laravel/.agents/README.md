# .agents/ - Shared Agent Skills

This directory holds the accelerator's skills in the cross-tool `.agents/` convention. **OpenAI Codex discovers repo skills here** (`.agents/skills/<name>/SKILL.md`), so this is where the Codex edition of the accelerator lives.

- **Skills:** `.agents/skills/<name>/SKILL.md` - Laravel workflows (including `project-brain` and `memory-bank`), plus `SKILL FLOW.md` describing the end-to-end flow.
- Codex loads these automatically for a trusted project; invoke a skill by name (explicitly or let Codex trigger it implicitly).

Companion Codex config lives in `.codex/` (config, hooks, DOD, principles). Policy is the shared root `AGENTS.md`.

Invoke Codex skills by their discovered names, including `documentation-generator`, `project-brain`, and `memory-bank`. Do not add fake Codex command or agent wrappers.

The `project-brain` skill owns governed shared task lifecycle, handoffs, all six governed record types, the unified `python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID` facade, compaction, and promotion proposals. Governed mode is the default; `--mode lightweight` is explicitly local-only.

The `memory-bank` skill owns durable retrieval/capture/audit/supersession and application of governed automatic or independently reviewed promotions. Session hooks report metadata only—mode, index health/staleness, active binding count, and validation status—and never index, retrieve, load, or inject record contents automatically.

See `.codex/README.md` for the full Codex setup and how it relates to the `.claude/` and `.cursor/` editions.
