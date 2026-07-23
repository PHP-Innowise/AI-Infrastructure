# Cursor Agents

These agents mirror the Claude edition in `.claude/agents/`, one agent per skill, but with **reduced frontmatter**: each file carries only `name` and `description` (the Cursor schema). The `model`, `invokes`, and `phase` keys used by the Claude edition are intentionally dropped; each agent body is otherwise identical to its Claude counterpart.

Every agent is a single-purpose executor that runs its one matching skill and stops - except the four orchestrators, `infra-scan`, `infra-generate`, `infra-build`, and `stack-adapter`, which are sanctioned to fan out the other agents/skills to run the full discovery and generation pipeline (or, for `stack-adapter`, the sibling-generator build).
