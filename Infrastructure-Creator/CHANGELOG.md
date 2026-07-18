# Changelog

All notable changes to Infrastructure-Creator are documented here. Format loosely follows [Keep a Changelog](https://keepachangelog.com/); versions are for this generator tool, not for anything it generates.

## [1.0.0] - 2026-07-18

### Added

- Initial release of Infrastructure-Creator: a standalone, PHP-only meta-accelerator that scans a target PHP project and generates a bespoke accelerator directly into it, for only the AI tool(s) the target team selects.
- Two-phase workflow with a human review checkpoint:
  - `infra-scan <target>` - six parallel PHP scanners (stack, architecture, integrations, infra/ops, security/compliance, conventions), grounded web research, a minimal clarifying interview (including the mandatory AI-tool-selection question), and synthesis into one reviewable Project Profile.
  - `infra-generate <target>` - policy/skill/agent/command/hook forges plus memory seeding, flow composition, and a final verification gate; writes only the selected edition(s).
  - `infra-build <target>` - optional one-shot that chains scan then generate, pausing only on blocking ambiguity or a collision.
- 20 skills total, following standard `SKILL.md`/agent/command frontmatter conventions.
- The generator itself ships in three editions - Claude Code (`.claude/`), Cursor (`.cursor/`), and Codex (`.agents/skills` + `.codex/`) - so it runs natively from whichever tool the operator uses.
- Bundled, PHP-specialized reference material: framework detection signals, an integration catalog, architecture patterns, the Project Profile schema, a dependency-free generated-output validator, and the memory-bank validator + chunk template.
- Safety guardrails: workspace-boundary enforcement, a collision guard for pre-existing target accelerators, evidence-cited findings, and a strict no-secrets policy.
