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
- [ ] `skill-generation-plan.json` parses and contains one complete contract per
  proposed skill: necessity, positive/negative routing, target-relative
  evidence, owned/excluded scope, procedure roles, decisions, failure handling,
  verification, output, siblings, and write capability. No grouped contract or
  fixed catalog count substitutes for evidence-gated inventory selection.
- [ ] Plan schema/catalog versions and top-level membership are exact;
  repository evidence has current fingerprints, bounded ranges, and supported
  claims; every selected skill has satisfied claim-backed selection conditions;
  every rejected candidate records why and what evidence is missing.
- [ ] The complete inventory passed plan-only ownership-ID, normalized
  write-surface, reciprocal sibling-routing, and contract-duplication checks
  before human approval. New profiles emit publication-eligible schema 1.2;
  legacy 1.0/1.1 plans remain audit-readable but were re-synthesized before
  generation.
- [ ] Every selected skill has structured capability, evidence-anchored
  procedure steps, concrete verification with pass/fail/skip behavior, path
  authority, critical-invariant coverage, routing cases, and flow membership.
- [ ] Every integration contract defaults to fake/fixture/local verification
  and records network policy, environment, authorization, rollback, and
  sanitization before any external-side-effect branch.
- [ ] Section 12 previews one chunk per cohesive durable concept composed only from confirmed evidence, linked to canonical sources, with no inferred/unknown fact, copied canonical document, customer data, or raw incident detail.

## Tier 2 - Generate & Update (`infra-generate`, `infra-update`, and forges)

- [ ] The profile was re-validated against the target's *current* files; any drift since the scan was flagged.
- [ ] The collision guard passed: an explicit overwrite/merge/abort decision was obtained before writing, if the target already had an accelerator.
- [ ] Only the selected edition(s) were written - no unselected edition folders were created, and no selected edition was skipped.
- [ ] Every generated skill has a validated necessity rationale and distinct
  operational value. The memory quartet is present because its runtime is
  generated; every other design, process, universal, frontend, specialty,
  integration, or domain skill is evidence-gated.
- [ ] Every skill cites resolvable target-relative canonical sources and
  implements its contract-specific procedure, decisions, failure handling,
  verification, output, and guardrails. It does not depend on the generator's
  `tasks/TASK-*` path at runtime.
- [ ] Every local textual citation renders its bounded line range or approved
  stable anchor; every owned-scope/output claim resolves to supporting evidence.
- [ ] Verification command aliases were resolved without execution and are
  non-mutating/non-networked unless an explicitly authorized capability branch
  classifies the side effect. Read-only procedures contain no edit/apply/write
  instruction.
- [ ] Per-skill and inventory-wide semantic validation passed: no ownership
  collision, repeated substantive template, paraphrased boilerplate cluster, or
  unjustified shared project invariant.
- [ ] Every bounded authoring batch passed `--allow-partial-skills`, and the
  final invocation without partial mode passed before wrappers, flows, manifest,
  or publication were constructed.
- [ ] Every scope-split skill pair stays non-duplicative and cross-references its counterpart: `debugging` (tools/logs) vs. `systematic-debugger` (methodology); `database-designer` (schema design) vs. `orm-patterns` (ORM usage patterns, when generated); `performance` (hot-path measurement) vs. `caching-strategy` (cache correctness, when generated); `api-designer` (hand-rolled routes) vs. `api-platform-design` (declarative resources, when generated).
- [ ] Every generated skill/agent/command carries valid frontmatter for its edition (see the forge skills' contracts).
- [ ] Every `flow-next`, `flow-alternatives`, `related`, `invokes`, and `spawns` reference resolves to a skill/agent that was actually generated.
- [ ] Agent descriptions and positive/negative examples route concrete target
  concerns to one primary owner (or explicit ambiguity); no circular "use X for
  X" wrapper exists. Flows include only scope-matching specialists.
- [ ] Every material adjacent owner and routing case survived into wrappers.
  `SKILL FLOW.md`, `flow-feature`, and `flow-review` match the canonical flow
  contract exactly, including required code-review/checkpoint stages.
- [ ] Every generated hook script passes `bash -n` and carries the executable bit; the per-edition hook set is complete (four enforcement + working-memory pair; Cursor deliberately lacks `working-memory-read.sh`), and every wiring file references only existing executable scripts.
- [ ] The seeded `memory-bank/` passes its validator and matches section 12 (same count, concepts, and sources); the context-brain runtime (`context.py`, `brain_runtime.py`, `context_retrieval.py`, `validate.py`) and the `project-brain/` skeleton were copied verbatim with `runtime.json` substituted (`context.py status`/`validate` exit 0 in the target); the memory quartet skills were also generated and wrapped; any drift was reported.
- [ ] `context.py status --json` reports task-identity and Git readiness
  truthfully as active, retrieval-only, or degraded. Explicit task identity
  outside Git is never reported as checkpoint-capable; all degraded hooks remain
  non-blocking and provide bounded remediation.
- [ ] No template placeholders (e.g. `{skill-name}`, `TODO`, `YYYY-MM-DD` left literal) remain in any manifest-owned text file. The only approved verbatim-asset declarations are the exact manifest-relative path-plus-regex pairs `memory-bank/templates/chunk.md` + `\bYYYY-MM-DD\b` and `memory-bank/scripts/validate.py` + `\bYYYY-MM-DD\b`; each exempts only matching occurrences, never the whole file. Every other placeholder in those files and the same token at every other path remains blocking. Unmanifested team files were not opened by placeholder or structural validation.
- [ ] The complete bundle was built under task-scoped staging and passed
  evidence, semantic, routing, structural, runtime, and manifest checks before
  publication. The explicit publication plan and manifest write plan were used;
  no staging/target root walk inferred ownership.
- [ ] `.infra-manifest.json` was written from the explicit generation/update write plan (version from the root `VERSION` file, profile/task reference, sha256 per planned file, no runtime state tracked). Manifest membership exclusively defines ownership in both modes; a manifest-owned `AGENTS.md` carries the matching stamp, while an untracked team `AGENTS.md` remains untouched.
- [ ] Forges emitted task-scoped root-ignore requirements only. The centralized
  helper deterministically composed `.gitignore` with positive patterns before
  `!` negations so re-includes win last-match; missing requirements received
  explicit append approval (or aborted), a requirement exactly contradicting a
  team entry failed for an explicit decision, and structured shared decision
  fields agree with legacy decision fields and the manifest hash.
- [ ] Publication rechecked baseline hashes, kept a rollback journal, copied the
  manifest last, and passed the full post-publication gate; the publish gate
  refused publication paths absent from the staged manifest, overwrites of
  non-ownable runtime state (seeding absences only), and removals the target
  manifest does not own; watch-only target members were rechecked without
  copying/journaling, and any failure restored exact previous bytes and modes,
  removed publication-created directory chains, and preserved (reporting as
  conflicts) files third parties edited after publication.
- [ ] For `infra-update` runs additionally: the executable ownership helper aborted cleanly if no manifest existed (legacy target); no file whose hash differed from the manifest was overwritten without an explicit per-file decision; files absent from both the manifest and staged output were not read, reported, or touched; memory state (chunks, `INDEX.md`, counters, `project-brain` records/indexes) was not regenerated; the rewritten manifest reflects the explicit final write plan and validates all decision fields including `task`.
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
- [ ] All six stack-specific reference catalogs are substantive and complete;
  no stub, mechanical source-language substitution, or source-stack leftover
  remains.
- [ ] The copied stack-agnostic quality architecture (generation-plan schema,
  validators, bad/good fixtures, staged publication gates) matches the source.
- [ ] The sibling's semantic fixtures and synthetic profile-to-staging
  rehearsal passed in addition to structural and hook verification.

Report unavailable tooling as `N/A - tooling not configured` rather than installing it without approval.
