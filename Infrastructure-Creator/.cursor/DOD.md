# Definition of Done - Infrastructure-Creator

This is the tiered verification checklist for the generator's *own* work (scanning a target and generating its accelerator). It is not the DOD of the accelerator that gets generated - that one is produced by `policy-forge` for the target.

Pick the tier that matches the work performed. Higher tiers include all lower-tier checks.

## Tier 0 - Any Skill Run (always)

- [ ] The target project path was explicit and validated; nothing outside it (or this folder's `tasks/`/`specs/`) was read or written.
- [ ] No `.env`, credentials, or secrets from the target were read, printed, or copied into any output.
- [ ] The skill produced a Context Summary and Next Steps.

## Tier 1 - Scan (`infra-scan` and individual scanners)

- [ ] Every finding cites concrete evidence: a real file path (with line numbers or an excerpt where practical) inside the target.
- [ ] Confidence is marked per finding: `confirmed`, `inferred`, or `unknown`. No guess is presented as fact.
- [ ] The target was confirmed to be a PHP project (`composer.json` and/or `*.php` present) before running the seven scanners; a non-PHP target was either offered `stack-adapter` or reported out of scope instead of being scanned.
- [ ] Any failed/skipped scanner is reported as a gap in the confidence summary, not silently dropped.
- [ ] The clarifying interview asked the mandatory AI-tool-selection question and recorded the answer.
- [ ] The Project Profile validates against the schema, including section 8's behavioral contract: every finding has confidence + source type; statuses are separate from proven transitions; permission completeness is stated; risk indicators do not invent severity/approval; contradictions remain visible.
- [ ] Section 11.1 describes every proposed skill and includes all eight groups (architecture / design / frontend / 18 process including the memory quartet (`memory-bank`, `project-brain`, `checkpoint`, `memory`) / universal / specialty / integrations / domain). Domain skills exist only for cohesive confirmed section 8.11 candidates. Section 11.2 arithmetic matches selected editions.
- [ ] Section 12 previews one chunk per cohesive durable concept composed only from confirmed evidence, linked to canonical sources, with no inferred/unknown fact, copied canonical document, customer data, or raw incident detail.

## Tier 2 - Generate & Update (`infra-generate`, `infra-update`, and forges)

- [ ] The profile was re-validated against the target's *current* files; any drift since the scan was flagged.
- [ ] The collision guard passed: an explicit overwrite/merge/abort decision was obtained before writing, if the target already had an accelerator.
- [ ] Only the selected edition(s) were written - no unselected edition folders were created, and no selected edition was skipped.
- [ ] `skill-forge` generated all 3 design & interaction skills and all 18 process & workflow skills (including the memory quartet: `memory-bank`, `project-brain`, `checkpoint`, `memory`) for every target; frontend and specialty conditions match the profile; domain skills exist only for cohesive confirmed section 8.11 candidates.
- [ ] Every scope-split skill pair stays non-duplicative and cross-references its counterpart: `debugging` (tools/logs) vs. `systematic-debugger` (methodology); `database-designer` (schema design) vs. `orm-patterns` (ORM usage patterns, when generated); `performance` (hot-path measurement) vs. `caching-strategy` (cache correctness, when generated); `api-designer` (hand-rolled routes) vs. `api-platform-design` (declarative resources, when generated).
- [ ] Every generated skill/agent/command carries valid frontmatter for its edition (see the forge skills' contracts).
- [ ] Every `flow-next`, `flow-alternatives`, `related`, `invokes`, and `spawns` reference resolves to a skill/agent that was actually generated.
- [ ] Every generated hook script passes `bash -n` and carries the executable bit; the per-edition hook set is complete (four enforcement + working-memory pair; Cursor deliberately lacks `working-memory-read.sh`), and every wiring file references only existing executable scripts.
- [ ] The seeded `memory-bank/` passes its validator and matches section 12 (same count, concepts, and sources); the context-brain runtime (`context.py`, `brain_runtime.py`, `context_retrieval.py`, `validate.py`) and the `project-brain/` skeleton were copied verbatim with `runtime.json` substituted (`context.py status`/`validate` exit 0 in the target); the memory quartet skills were also generated and wrapped; any drift was reported.
- [ ] No template placeholders (e.g. `{skill-name}`, `TODO`, `YYYY-MM-DD` left literal) remain in any generated file.
- [ ] The target's `AGENTS.md` first line carries the version stamp and `.infra-manifest.json` was written at the target root (version from the root `VERSION` file, profile/task reference, sha256 per generator-owned file, no runtime state tracked); the manifest was refreshed after any content auto-fix.
- [ ] For `infra-update` runs additionally: the run aborted cleanly if no manifest existed (legacy target); no file whose hash differed from the manifest was overwritten without an explicit per-file decision; files absent from the manifest were not touched; memory state (chunks, `INDEX.md`, counters, `project-brain` records/indexes) was not regenerated; the rewritten manifest reflects the final target state.
- [ ] `bootstrap-verifier` was run and reported no unresolved failures.

## Tier 2.5 - Edition Mirroring

- [ ] Exactly 23 generator skills exist in `.claude/skills`, `.cursor/skills`, and `.agents/skills`, including `domain-behavior-scanner`.
- [ ] Corresponding skill files and nested references/assets/scripts are byte-identical across all three editions.
- [ ] Cursor has 23 matching reduced-frontmatter agents; Codex has no agents or commands.
- [ ] Scanner/profile/forge/policy/memory documentation is internally consistent across editions.

## Tier 3 - Release of this generator itself

- [ ] All 23 skills exist in `.claude/skills`, `.cursor/skills`, and `.agents/skills`, byte-identical where required.
- [ ] Agents/commands exist for the editions that carry them; Cursor frontmatter is the reduced form.
- [ ] Hooks are present and wired in all three editions (`settings.json`, `.cursor/hooks.json`, `.codex/hooks.json` + `config.toml`).
- [ ] The root `VERSION` file was bumped and matches the new `CHANGELOG.md` entry - it is the single version source everything else reads.
- [ ] `CHANGELOG.md` records the change.

## Tier 4 - Stack Adaptation (`stack-adapter`)

- [ ] The user gave explicit confirmation before the sibling generator was built - detection alone was never treated as consent.
- [ ] The collision guard passed on the sibling folder path (overwrite/merge/abort decided explicitly, not assumed).
- [ ] Every stack-specific claim (framework, tooling, integration categories, architecture patterns) is grounded in this run's own research, cited to an authoritative source - never carried over from PHP knowledge.
- [ ] The sibling generator contains zero mentions of PHP, Laravel, Symfony, PHP Core, or "Infrastructure-Creator" anywhere in its own content.
- [ ] All 23 skills were re-authored (not left as PHP copies), including `domain-behavior-scanner` and `stack-adapter`'s own identity-swapped copy, and mirrored byte-identically across its three editions.
- [ ] The copied stack-agnostic assets (`memory-seed`'s runtime scripts, validator, chunk template, and `project-brain/` skeleton; `bootstrap-verifier`'s `validate_generated.py`) were copied verbatim, unmodified (sole exceptions: the `{{TARGET_FRAMEWORK}}` and `{{CANONICAL_EDITION}}` substitutions in `runtime.json.template`).
- [ ] The sibling generator's own self-verification (its copy of `bootstrap-verifier`'s script, plus `bash -n`/executable-bit checks on its hooks) passed before reporting done.

Report unavailable tooling as `N/A - tooling not configured` rather than installing it without approval.
