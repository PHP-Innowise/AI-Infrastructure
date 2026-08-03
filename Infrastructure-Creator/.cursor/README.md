# Infrastructure-Creator - Cursor Edition

This is the Cursor edition of the generator. It mirrors the Claude edition's 23 skills exactly and adds Cursor-native wiring.

- **Skills:** `.cursor/skills/<name>/SKILL.md` (byte-identical to the Claude edition, including nested references, assets, and scripts).
- **Agents:** `.cursor/agents/<name>-agent.md` - 23 wrappers with reduced frontmatter (`name` + `description` only).
- **Commands:** `.cursor/commands/{infra-scan,infra-generate,infra-build,infra-update,infra-adapt}.md` - `name` + `description` frontmatter.
- **Rules:** `.cursor/rules/*.mdc` - `accelerator-workflow` and `safety-and-verification` are always applied; `php-standards` is on-demand reference.
- **Hooks:** `.cursor/hooks/*.sh` wired via `.cursor/hooks.json` (camelCase events, second timeouts).
- **Policy:** the shared root `AGENTS.md`, plus this edition's `DOD.md`, `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md`.

Usage: `/infra-scan <target-php-project>`, review the profile (including section 8's behavioral contract, section 11's generated-infrastructure preview, and section 12's memory preview), then `/infra-generate <target>` (or `/infra-build <target>` for one-shot); later, `/infra-update <target>` upgrades a generated target via its `.infra-manifest.json`. Every command requires an explicit target path; the generator never operates on its own folder.
