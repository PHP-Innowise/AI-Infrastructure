# .agents - Codex Skills Tree

This directory exists because Codex loads repository skills from `.agents/skills/`, deliberately outside `.codex/`.

- `.agents/skills/<name>/SKILL.md` - the complete 25-skill generator tree, byte-identical to `.claude/skills`, including nested references, assets, and scripts.
- Codex configuration, hooks, and this edition's policy docs live in `.codex/`; the shared policy is the root `AGENTS.md`.
- There are no agents or commands here: Codex invokes skills directly by name.

Do not edit these skills in isolation. They are canonical mirrors and must stay byte-identical across `.claude/skills`, `.cursor/skills`, and `.agents/skills`.
