# Cursor Agents

One agent wraps each of the 23 skills. Every wrapper mirrors its Claude counterpart's useful body while using Cursor's reduced frontmatter: exactly `name` and `description`; Claude-specific `model`, `invokes`, and `phase` keys are intentionally omitted.

An agent runs exactly one skill in an isolated context and then stops, except the five sanctioned orchestrators (`infra-scan`, `infra-generate`, `infra-build`, `infra-update`, and `stack-adapter`), which may fan out the skills required by their workflows.

Users normally interact through the five commands (`/infra-scan`, `/infra-generate`, `/infra-build`, `/infra-update`, and `/infra-adapt`). `infra-scan` coordinates seven scanners, including `domain-behavior-scanner`.

Codex has no agent layer; it invokes the mirrored skills in `.agents/skills/` directly.
