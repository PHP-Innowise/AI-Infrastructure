# Agents (Infrastructure-Creator, Claude Code edition)

One agent wraps each of the 23 skills. An agent runs exactly one skill in an isolated context and then stops - except the five sanctioned orchestrators (`infra-scan`, `infra-generate`, `infra-build`, `infra-update`, `stack-adapter`), which may fan out other skills (see the root `AGENTS.md` Orchestration Exception).

- Frontmatter carries `name`, `description` (with usage `<example>` blocks), `model` (`opus` for heavy reasoning: orchestration, synthesis, `skill-forge`, `policy-forge`, `architecture-scanner`, `domain-behavior-scanner`, `stack-adapter`; `sonnet` for the rest), `invokes` (the skill it runs), and `phase`.
- Users normally interact through the five commands (`/infra-scan`, `/infra-generate`, `/infra-build`, `/infra-update`, `/infra-adapt`); the orchestrators spawn the remaining scanner/forge agents as needed.

The Cursor edition mirrors all 23 agents with reduced frontmatter (`name` + `description` only). Codex has no agent layer and invokes the same 23 skills directly.
