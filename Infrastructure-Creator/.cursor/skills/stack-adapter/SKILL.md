---
name: stack-adapter
description: Produce an independent sibling generator - Infrastructure-Creator-[Stack] - with the identical 21-skill, three-edition architecture as this generator, freshly researched and authored for a non-PHP stack detected in a target project (e.g. Flutter/Dart, Node.js, Python, Go). Use when infra-scan detects a target is not PHP but is a recognizable stack and the user opts in, or when the user directly asks to adapt the generator for a different stack. Triggers on "stack-adapter", "adapt the generator for Flutter", "build a sibling generator", "generate an accelerator generator for Node/Python/Go".
phase: orchestration
flow-next: null
flow-alternatives: []
related: [infra-scan]
---

# Stack Adapter

## Overview

`stack-adapter` is the fourth orchestrator skill (see `AGENTS.md`'s "Orchestration Exception"). It does not generate an accelerator for a target project - it generates an entire **new, independent generator**, structurally identical to Infrastructure-Creator, but fully re-authored for a different technology stack than PHP.

This is a meta-generation task: the same discipline `skill-forge` applies to one skill, this skill applies to an entire 21-skill tool - including a re-authored copy of `stack-adapter` itself, so the new generator is a complete peer of this one (an emergent property: it could later spawn a third generator for yet another stack, following the same process, though that is not separately tested here). Nothing about the new stack is pre-written or hardcoded here - this skill's job is to research the detected stack live (the same grounding principle `stack-researcher` uses) and author every stack-specific artifact fresh, while mechanically replicating the parts of Infrastructure-Creator's structure that are not stack-specific at all.

The produced sibling generator MUST be exactly as independent as Infrastructure-Creator itself: it must never mention PHP, Laravel, Symfony, PHP Core, or "Infrastructure-Creator" (this tool) anywhere in its own content. It is a standalone tool that happens to share an architecture by construction, not a fork or a themed copy.

## What Is Copied Verbatim vs. Re-Authored

Not everything needs to be rewritten - some of Infrastructure-Creator's own bundled material is already stack-agnostic by design:

- **Copy verbatim (already stack-agnostic):** `memory-seed/assets/scripts/validate.py`, `memory-seed/assets/templates/chunk.md`, `bootstrap-verifier/scripts/validate_generated.py`. These contain no PHP-specific logic - they validate structure (JSON frontmatter shape, ID/filename patterns, cross-references, hook syntax/executable bits, placeholders), not PHP content.
- **Replicate structurally, then adapt wording only:** the directory layout, `SKILL.md`/agent/command frontmatter contracts, hook *mechanics* (event wiring per edition schema), the three-edition layout, the Orchestration Exception model, and the general shape of `AGENTS.md`/`DOD.md`/`GOLDEN-PRINCIPLES.md`/`STABILIZATION.md`. Only the prose describing the target domain changes (e.g. "PHP project" -> "[Stack] project"); the policy structure itself does not.
- **Re-author entirely, grounded in fresh research:** all six scanners' detection signals, `stack-researcher`'s research targets, `clarifying-interview`'s question shape, `profile-synthesizer`'s schema content (not its section structure), every forge's stack-specific guidance, the three `skill-forge/references/*.md` equivalents, and `profile-synthesizer/references/project-profile-schema.md`'s stack-specific fields.
- **Re-author with identity swapped, not copied verbatim:** `stack-adapter` itself. Its own copy in the new generator must keep the same mechanism (research -> replicate -> re-author -> mirror -> self-verify -> report) but with every self-reference updated: "PHP" -> the new generator's own domain, "Infrastructure-Creator" -> the new generator's own name, and its own independence guardrail restated in terms of *that* generator's identity (e.g. a Flutter generator's `stack-adapter` copy must forbid mentions of Flutter/Dart or "Infrastructure-Creator-Flutter" in whatever it spawns next, not PHP).

## Generated File Naming Convention (MANDATORY)

- The new generator is written to a sibling folder next to Infrastructure-Creator itself, named `Infrastructure-Creator-[Stack]/` (e.g. `Infrastructure-Creator-Flutter/`), unless the user explicitly gave a different output path when invoking this skill directly.
- This skill's own run notes live in Infrastructure-Creator's `tasks/TASK-{N}/stack-adapter-report.md`. Nothing is written into the original target project - that project is only evidence for detecting the stack, never a write target for this skill.

## Process

1. **Confirm scope.** Record the detected stack name, the evidence that identified it (e.g. `pubspec.yaml` present), and the resolved output path (default per the naming convention above). If invoked directly (not via `infra-scan`'s auto-detection), confirm the stack and target path explicitly with the user before proceeding.
2. **Collision guard.** If `Infrastructure-Creator-[Stack]/` already exists at the resolved path, STOP and ask: overwrite, merge, or abort. Never overwrite silently.
3. **Research the stack** (grounded, generic methodology - apply this to whatever stack was detected, do not rely on prior knowledge alone):
   - The standard package manager and manifest format.
   - The dominant application framework(s) in that ecosystem, and how to detect each from real files.
   - The standard test framework(s), lint/format tool(s), and static-analysis tool(s).
   - 8-12 integration categories relevant to real applications in that ecosystem (payment, messaging/queue, cache, storage, auth, observability, etc.) with concrete, real package names per category.
   - 2-4 common architecture patterns used in that ecosystem and their detection signals.
   - Cite official/authoritative sources for each of the above, the same way `stack-researcher` cites sources.
4. **Replicate the structural skeleton** into the new folder: root `AGENTS.md`/`README.md`/`CHANGELOG.md`/`.gitignore`, `specs/`, `tasks/` (+ `.task-counter` set to `1`), `examples/`, and the three edition trees (`.claude/`, `.cursor/`, `.codex/` + `.agents/`) with their wiring files (`settings.json`, `.cursor/hooks.json` + `rules/*.mdc`, `.codex/config.toml` + `hooks.json`) - copying Infrastructure-Creator's own wiring files as-is, since hook *registration mechanics* do not depend on the target stack.
5. **Re-author all 21 skills** for the new stack (the 20 PHP-specific skills, plus `stack-adapter` itself with its identity swapped per the rule above), using this generator's own `.claude/skills/*/SKILL.md` files as STRUCTURAL exemplars only (same frontmatter keys, same section headings, same phase taxonomy, same orchestration-exception shape) - never copying their PHP content. Parallelize this across subagents in logical batches (discovery scanners together, forges together, etc.) the same way this generator itself was built, using one skill per batch as a style exemplar first. Every generated skill must:
   - Ground its detection/generation logic in step 3's research.
   - Carry zero mentions of PHP or Infrastructure-Creator (this instance).
   - Cross-reference (`flow-next`/`flow-alternatives`/`related`) only skills that exist in this new set.
6. **Re-author the bundled reference docs**: three `skill-forge/references/*.md` equivalents (frameworks, architecture patterns, integration catalog, all for the new stack) and `profile-synthesizer/references/project-profile-schema.md` (same section structure, stack-specific field content).
7. **Copy the verbatim assets** listed above into their new-generator paths, unchanged.
8. **Mirror the three editions**: `.claude/skills` is authored canonically; copy it byte-for-byte into `.cursor/skills` and `.agents/skills`; derive Cursor's reduced-frontmatter agents/commands from the Claude versions; skip agents/commands for Codex.
9. **Self-verify.** Run the copied `bootstrap-verifier/scripts/validate_generated.py` against the new generator's own directory (its `skills`/`agents`/`commands` trees have the same shape this script already expects to check), plus `bash -n` and executable-bit checks on its hooks. Fix what is safely fixable; do not report success with unresolved failures.
10. **Report.** Write `tasks/TASK-{N}/stack-adapter-report.md` and the final output below.

## Output Template

```markdown
# Stack Adapter Complete: Infrastructure-Creator-[Stack]

**Detected stack:** [Stack] (evidence: [file])
**New generator path:** [path]
**Skills generated:** 21 (mirrored across .claude/.cursor/.codex+.agents)
**Self-verification:** [pass/fail summary]

## What It Covers
[2-4 sentences: the framework(s), test/lint/analysis tooling, and integration categories the new generator's skills were grounded in]

## Next Step
Open [new generator path] as its own workspace (sibling to your target project, same as this generator), then run:
`infra-scan [original target path]`
```

## Guardrails

- MUST NOT invoke this skill without the user's explicit confirmation - a detected stack is never itself consent.
- MUST NOT overwrite an existing `Infrastructure-Creator-[Stack]/` without an explicit overwrite/merge/abort decision.
- MUST NOT write anything into the original target project - it is evidence only, never a write target here.
- MUST NOT let the new generator's content mention PHP, Laravel, Symfony, PHP Core, or "Infrastructure-Creator" anywhere - it must read as fully standalone.
- MUST ground every re-authored artifact in step 3's research, never in assumption; cite sources the same way `stack-researcher` does.
- MUST run self-verification and treat unresolved failures as "not done yet," exactly as `bootstrap-verifier` does for a normal generation run.
- MUST NOT skip re-authoring any of the 21 skills (including `stack-adapter`'s own identity-swapped copy) - a partial sibling generator is not a valid result.

## Final Output

Return the new generator's path, the detected stack and its evidence, the self-verification result, a short summary of what the new generator's skills were grounded in, and the exact next command to run there against the original target.
