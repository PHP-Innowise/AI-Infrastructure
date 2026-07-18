# .agents/ - Shared Agent Skills

This directory holds the accelerator's skills in the cross-tool `.agents/` convention. **OpenAI Codex discovers repo skills here** (`.agents/skills/<name>/SKILL.md`), so this is where the Codex edition of the accelerator lives.

- **Skills:** `.agents/skills/<name>/SKILL.md` - 39 Laravel workflows (coder, architect, filament, eloquent, queues-jobs, test-generator, security-reviewer, ...), plus `SKILL FLOW.md` describing the end-to-end flow.
- Codex loads these automatically for a trusted project; invoke a skill by name (explicitly or let Codex trigger it implicitly).

Companion Codex config lives in `.codex/` (config, hooks, DOD, principles). Policy is the shared root `AGENTS.md`.

See `.codex/README.md` for the full Codex setup and how it relates to the `.claude/` and `.cursor/` editions.
