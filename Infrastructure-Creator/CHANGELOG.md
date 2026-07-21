# Changelog

All notable changes to Infrastructure-Creator are documented here. Format loosely follows [Keep a Changelog](https://keepachangelog.com/); versions are for this generator tool, not for anything it generates.

## [1.3.0] - 2026-07-21

### Added

- `skill-forge` now generates a much broader, still 100% evidence-driven skill set for the target, in six groups instead of three: **architecture** (1, unchanged), **design & interaction** (3, always: `architecture-implementer`, `api-designer`, `database-designer`), **frontend** (0 or 5, conditional on a detected rendering/asset layer: `frontend-design`, `coder-frontend`, `wcag-accessibility`, `web-design-guidelines`, `browser-verify`), **process & workflow** (14, always, framework-agnostic: `requirements-analyst`, `researcher`, `brainstorming`, `council`, `writing-plans`, `using-git-worktrees`, `systematic-debugger`, `refactorer`, `dependency-manager`, `review-pr`, `finishing-branch`, `documentation-generator`, `skill-creator`, `reflect`), **universal PHP** (7, unchanged), and **framework-specialty** (evidence-gated: ORM patterns, migration safety, async/queue jobs, event-boundary review, notification delivery, caching strategy, file storage, auth scaffolding, form/validator design, admin panel, declarative API resources, console commands, repository review, container review, test-data factories, package authoring), plus integrations (unchanged).
- This closes the gap identified by auditing the Laravel/Symfony/PHP-Core accelerators in this monorepo: a generated target previously received only an architecture skill + 7 universal skills + integrations, far narrower than the ~40-skill hand-built accelerators. The new framework-specialty catalog generalizes the useful Laravel-only (`eloquent`, `queues-jobs`, `caching`, `events-notifications`, `file-storage`, `console-scheduler`, `auth-scaffolding`, `filament`, `package-developer`) and Symfony-only (`doctrine-migration-designer`, `messenger-designer`, `event-subscriber-designer`, `security-voter-designer`, `form-validator-designer`, `fixture-factory-generator`, `console-command-coder`, `container-reviewer`, `repository-reviewer`, `api-platform-designer`) skills into signal-named, framework-neutral equivalents keyed to real scan evidence rather than to which framework folder they came from.
- New Project Profile section 3.1 ("Framework-Specialty Signals") and 3.2 ("Frontend Presence"), populated by an expanded `architecture-scanner` that now also detects ORM/data-access pattern, migration tooling, async/queue mechanism, event/notification patterns, caching/storage usage, auth/authorization scaffolding, form/validator design, admin panels, declarative API frameworks, console commands, repository/DI-container style, test factories, package-vs-application nature, and rendering/frontend-asset presence.
- Two new bundled references for `skill-forge`: `references/php-process-skills.md` (the 14 always-generated process/workflow skills) and `references/php-specialty-skills.md` (the evidence-gated specialty catalog, mapped from section 3.1 signals to generated skills). `references/php-frameworks.md` gained "Design & Interaction Skills" and "Frontend Skills" sections.
- Section 10.1 of the Project Profile now lists all six skill groups explicitly (with the group-by-group count breakdown carried into 10.2's arithmetic), and `infra-scan`'s "What Will Be Generated" summary reports the full group breakdown, not just architecture/universal/integrations.
- New DOD checks (Tier 1 and Tier 2) verifying sections 3.1/3.2 and that the fixed design/process groups and the conditional frontend/specialty groups were generated correctly.
- `examples/infra-scan-project-profile-example.md` rewritten end to end for the new sections and the resulting 43-skill total for the acme-billing example (up from 15).

### Fixed

- Removed the unintentional overlap between the universal `debugging` skill and the process skill `systematic-debugger` (both previously could restate the same "root-cause-first" debugging content). Their scope is now explicitly split and cross-referenced: `debugging` owns only the target's real tools/log locations ("where to look"); `systematic-debugger` owns the tool-agnostic investigative methodology ("how to look") and names zero target-specific tools. New DOD check verifies neither duplicates the other.
- Audited every remaining pair of generated skills for the same kind of overlap and fixed three more, all with the same scope-split-plus-cross-reference pattern (no skill counts changed):
  - `database-designer` (design & interaction, schema/table/index/migration design) vs. `orm-patterns` (specialty, how application code uses that schema through the ORM - relationships-as-used-in-code, casts, scopes, eager loading). Previously `database-designer`'s own reference text mentioned "Eloquent relationships," directly overlapping `orm-patterns`.
  - `performance` (universal, measure-first hot-path workflow) vs. `caching-strategy` (specialty, cache-aside correctness/invalidation). `performance` now names caching only as one possible hot-path lever and defers to `caching-strategy` for correctness depth when that specialty skill is generated.
  - `api-designer` (design & interaction, hand-rolled routes/controllers) vs. `api-platform-design` (specialty, declarative API resource frameworks). `api-designer` now explicitly narrows to any remaining hand-rolled endpoints and defers resource-level design to `api-platform-design` when a declarative framework is the target's primary/sole API mechanism.
  - DOD's Tier 2 check was broadened from just the `debugging`/`systematic-debugger` pair to cover all four scope-split pairs.

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
