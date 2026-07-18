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
- [ ] The target was confirmed to be a PHP project (`composer.json` and/or `*.php` present); a non-PHP target was reported out of scope instead of scanned.
- [ ] Any failed/skipped scanner is reported as a gap in the confidence summary, not silently dropped.
- [ ] The clarifying interview asked the mandatory AI-tool-selection question and recorded the answer.
- [ ] The Project Profile validates against `profile-synthesizer/references/project-profile-schema.md`.

## Tier 2 - Generate (`infra-generate` and forges)

- [ ] The profile was re-validated against the target's *current* files; any drift since the scan was flagged.
- [ ] The collision guard passed: an explicit overwrite/merge/abort decision was obtained before writing, if the target already had an accelerator.
- [ ] Only the selected edition(s) were written - no unselected edition folders were created, and no selected edition was skipped.
- [ ] Every generated skill/agent/command carries valid frontmatter for its edition (see the forge skills' contracts).
- [ ] Every `flow-next`, `flow-alternatives`, `related`, `invokes`, and `spawns` reference resolves to a skill/agent that was actually generated.
- [ ] Every generated hook script passes `bash -n` and carries the executable bit.
- [ ] The seeded `memory-bank/` passes its own `scripts/validate.py`.
- [ ] No template placeholders (e.g. `{skill-name}`, `TODO`, `YYYY-MM-DD` left literal) remain in any generated file.
- [ ] `bootstrap-verifier` was run and reported no unresolved failures.

## Tier 3 - Release of this generator itself

- [ ] All 19 skills exist in `.claude/skills`, `.cursor/skills`, and `.agents/skills`, byte-identical where required.
- [ ] Agents/commands exist for the editions that carry them; Cursor frontmatter is the reduced form.
- [ ] Hooks are present and wired in all three editions (`settings.json`, `.cursor/hooks.json`, `.codex/hooks.json` + `config.toml`).
- [ ] `CHANGELOG.md` records the change.

Report unavailable tooling as `N/A - tooling not configured` rather than installing it without approval.
