# Agents (Infrastructure-Creator, Claude Code edition)

One agent wraps each of the 21 skills. An agent runs exactly one skill in an isolated context and then stops - except the four sanctioned orchestrators (`infra-scan`, `infra-generate`, `infra-build`, `stack-adapter`), which may fan out other skills (see the root `AGENTS.md` Orchestration Exception).

- Frontmatter carries `name`, `description` (with usage `<example>` blocks), `model` (`opus` for heavy reasoning: orchestration, synthesis, `skill-forge`, `policy-forge`, `architecture-scanner`, `stack-adapter`; `sonnet` for the rest), `invokes` (the skill it runs), and `phase`.
- Users normally interact through the four commands (`/infra-scan`, `/infra-generate`, `/infra-build`, `/infra-adapt`); the orchestrators spawn the remaining scanner/forge agents as needed.

The Cursor edition mirrors these agents with reduced frontmatter (`name` + `description` only). Codex has no agents - it invokes skills directly.
