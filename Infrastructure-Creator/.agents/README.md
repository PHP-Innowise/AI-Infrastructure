# .agents - Codex Skills Tree

This directory exists because Codex loads repository skills from `.agents/skills/`, deliberately outside `.codex/`.

- `.agents/skills/<name>/SKILL.md` - the 21 generator skills, byte-identical to the Claude edition's `.claude/skills`.
- Codex configuration, hooks, and this edition's policy docs live in `.codex/`; the shared policy is the root `AGENTS.md`.
- There are no agents or commands here - Codex invokes skills directly by name.

Do not edit these skills in isolation: they are mirrored from the canonical `.claude/skills` tree and must stay identical across `.claude/skills`, `.cursor/skills`, and `.agents/skills`.
