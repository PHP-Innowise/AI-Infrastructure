# Changelog

All notable changes to Infrastructure-Creator are documented here. Format loosely follows [Keep a Changelog](https://keepachangelog.com/); versions are for this generator tool, not for anything it generates.

## [1.2.0] - 2026-07-21

### Added

- The Project Profile's "Generation Notes" section is now split into 10.1 (Skills To Generate), 10.2 (Agents & Commands Preview), and 10.3 (Non-PHP Neighbors). 10.1 now requires a one-line, target-specific description for every proposed skill (naming the real package/pattern/tool found) instead of a bare skill name; 10.2 states the exact agent and command counts implied by the selected edition(s).
- New Project Profile section 11 ("Memory Bank Preview") - `profile-synthesizer` now previews every memory-bank chunk `memory-seed` will seed (planned ID, title, type, source) before the user ever runs `infra-generate`, using the same confirmed-only selection rule `memory-seed` applies.
- `memory-seed` now treats the profile's section 11 as its authoritative seed plan and reports any drift between the preview and what it actually seeds, rather than silently reconciling differences.
- `infra-scan`'s final report now surfaces a "What Will Be Generated" summary (skill/agent/command counts and memory-bank chunk count) alongside the existing confidence summary, so the scope of a generation run is visible without opening the full profile.
- New DOD checks (Tier 1 and Tier 2) verifying the enriched profile sections and that seeded memory-bank chunks match their preview.
- `examples/infra-scan-project-profile-example.md` rewritten to demonstrate the new sections 10.1-10.3 and 11 end to end.

## [1.1.0] - 2026-07-20

### Added

- Self-adaptation: when `infra-scan` finds no PHP evidence, it now probes for a recognizable non-PHP stack (Flutter/Dart, Node.js, Python, Go, Ruby, Java/Kotlin, .NET, Rust, Swift, or similar, via a generic manifest-signal table) instead of only reporting out of scope.
- New `stack-adapter` skill (21st skill) - a meta-generator that, with explicit user confirmation, researches the detected stack live and builds a brand-new, fully independent sibling generator (`Infrastructure-Creator-[Stack]/`, a sibling folder next to this one) with the identical 21-skill, three-edition architecture (including its own re-authored copy of `stack-adapter`), freshly authored for that stack. Zero mentions of PHP or of this generator appear in the sibling's own content.
- New `infra-adapt <target-path>` entry point (command + agent, mirrored Claude/Cursor) for invoking `stack-adapter` directly, without going through `infra-scan`'s auto-detection first.
- `stack-adapter` is documented as the fourth sanctioned Orchestration Exception in `AGENTS.md`, alongside `infra-scan`, `infra-generate`, and `infra-build`.
- New Golden Principle ("Honest Scope Over Silent Failure or Scope Creep") and a new DOD tier (Tier 4 - Stack Adaptation) covering verification of a self-generated sibling.

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
