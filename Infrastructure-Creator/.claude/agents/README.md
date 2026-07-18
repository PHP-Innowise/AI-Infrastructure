# Agents (Infrastructure-Creator, Claude Code edition)

One agent wraps each of the 20 skills. An agent runs exactly one skill in an isolated context and then stops - except the three sanctioned orchestrators (`infra-scan`, `infra-generate`, `infra-build`), which may fan out other skills (see the root `AGENTS.md` Orchestration Exception).

- Frontmatter carries `name`, `description` (with usage `<example>` blocks), `model` (`opus` for heavy reasoning: orchestration, synthesis, `skill-forge`, `policy-forge`, `architecture-scanner`; `sonnet` for the rest), `invokes` (the skill it runs), and `phase`.
- Users normally interact through the three commands (`/infra-scan`, `/infra-generate`, `/infra-build`); the orchestrators spawn the remaining scanner/forge agents as needed.

The Cursor edition mirrors these agents with reduced frontmatter (`name` + `description` only). Codex has no agents - it invokes skills directly.
