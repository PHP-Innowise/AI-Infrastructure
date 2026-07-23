# Infrastructure-Creator - Codex Edition

This is the Codex edition of the generator. Codex discovers skills and policy differently from Claude/Cursor:

| Layer | Location | Notes |
| --- | --- | --- |
| Skills (21 workflows) | `.agents/skills/<name>/SKILL.md` | Codex loads repo skills from `.agents/skills`, not `.codex/`. Byte-identical to the Claude edition. |
| Policy | root `AGENTS.md` | Read natively by Codex (walked root -> cwd, concatenated). Shared with Claude/Cursor. |
| Config | `.codex/config.toml` | Enables lifecycle hooks; loads only when the project is trusted. |
| Hooks | `.codex/hooks.json` + `.codex/hooks/*.sh` | Same event schema as Claude Code (no matcher/timeout). |
| Definition of Done / principles | `.codex/DOD.md`, `.codex/GOLDEN-PRINCIPLES.md`, `.codex/STABILIZATION.md` | This edition's copies. |

- **No command layer.** Codex uses skills directly rather than slash commands; run a skill by name (`infra-scan`, `infra-generate`, `infra-build`, `infra-adapt`).
- **No agent wrappers.** Codex invokes skills directly, so the `.claude/agents` / `.cursor/agents` wrappers are not mirrored here.

Usage: invoke `infra-scan <target-php-project>`, review the profile, then `infra-generate <target>` (or `infra-build <target>`). Every invocation requires an explicit target path. If `infra-scan` detects a non-PHP stack and you opt in (or you already know the target isn't PHP), it hands off to `stack-adapter` - invoke that skill by name (`stack-adapter <target>`) to build an independent sibling generator for that stack.
