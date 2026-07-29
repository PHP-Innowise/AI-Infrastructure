# Infrastructure-Creator - Codex Edition

This is the Codex edition of the generator. Codex discovers skills and policy differently from Claude/Cursor:

| Layer | Location | Notes |
| --- | --- | --- |
| Skills (22 workflows) | `.agents/skills/<name>/SKILL.md` | Codex loads repo skills from `.agents/skills`, not `.codex/`. The complete tree is byte-identical to the Claude edition. |
| Policy | root `AGENTS.md` | Read natively by Codex (walked root -> cwd, concatenated). Shared with Claude/Cursor. |
| Config | `.codex/config.toml` | Enables lifecycle hooks; loads only when the project is trusted. |
| Hooks | `.codex/hooks.json` + `.codex/hooks/*.sh` | Same event schema as Claude Code (no matcher/timeout). |
| Definition of Done / principles | `.codex/DOD.md`, `.codex/GOLDEN-PRINCIPLES.md`, `.codex/STABILIZATION.md` | This edition's copies. |

- **No command layer.** Codex invokes skills directly by name (`infra-scan`, `infra-generate`, `infra-build`, `stack-adapter`).
- **No agent wrappers.** Codex invokes the 22 skills directly, so Claude/Cursor wrappers are not mirrored here.
- **Full discovery and generation.** `infra-scan` runs seven scanners, including `domain-behavior-scanner`; the profile captures section 8's behavioral contract, section 11's generated-infrastructure preview, and section 12's memory preview. Generation includes evidence-gated domain skills and the operational `memory-bank` skill.

Usage: invoke `infra-scan <target-php-project>`, review the profile, then `infra-generate <target>` (or `infra-build <target>`). Every invocation requires an explicit target path. For a confirmed non-PHP target, invoke `stack-adapter <target>` only after explicit user approval.
