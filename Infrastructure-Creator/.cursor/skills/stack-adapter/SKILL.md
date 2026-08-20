---
name: stack-adapter
description: Produce an independent sibling generator - Infrastructure-Creator-[Stack] - with the identical 25-skill, three-edition architecture as this generator, including domain-behavior discovery, freshly researched and authored for a non-PHP stack detected in a target project. Use when infra-scan detects a recognizable non-PHP stack and the user opts in, or when the user directly asks to adapt the generator. Triggers on "stack-adapter", "adapt the generator for Flutter", "build a sibling generator", "generate an accelerator generator for Node/Python/Go".
phase: orchestration
flow-next: null
flow-alternatives: []
related: [infra-scan]
---

# Stack Adapter

## Overview

`stack-adapter` is one of the six sanctioned orchestrator skills (see `AGENTS.md`'s "Orchestration Exception"). It does not generate an accelerator for a target project - it generates an entire **new, independent generator**, structurally identical to Infrastructure-Creator, but fully re-authored for a different technology stack than PHP.

This is a meta-generation task: the same discipline `skill-forge` applies to one skill, this skill applies to an entire 25-skill tool - including `domain-behavior-scanner` and a re-authored copy of `stack-adapter` itself. Nothing about the new stack is pre-written or hardcoded here.

The produced sibling generator MUST be exactly as independent as Infrastructure-Creator itself: it must never mention PHP, Laravel, Symfony, PHP Core, or "Infrastructure-Creator" (this tool) anywhere in its own content. It is a standalone tool that happens to share an architecture by construction, not a fork or a themed copy.

## What Is Copied Verbatim vs. Re-Authored

Not everything needs to be rewritten - some of Infrastructure-Creator's own bundled material is already stack-agnostic by design:

- **Copy verbatim (already stack-agnostic):** memory runtime validation
  assets, `bootstrap-verifier/scripts/validate_generated.py`,
  `validate_skill_quality.py`, `validate_reference_catalogs.py`,
  command-risk and flow-contract validators, `publish_staging.py`, ownership
  helpers, the generation-plan schema
  mechanism, and their negative/positive/rollback regression fixtures. These
  enforce evidence, typed operational contracts, provider/capability safety,
  path/invariant/anchor coverage, routing/flow equivalence, transactional, and
  structural quality without encoding a source stack.
- **Replicate structurally, then adapt wording only:** the directory layout, `SKILL.md`/agent/command frontmatter contracts, hook *mechanics* (event wiring per edition schema), the three-edition layout, the Orchestration Exception model, and the general shape of `AGENTS.md`/`DOD.md`/`GOLDEN-PRINCIPLES.md`/`STABILIZATION.md`. Only the prose describing the target domain changes (e.g. "PHP project" -> "[Stack] project"); the policy structure itself does not.
- **Re-author entirely, grounded in fresh research:** all seven scanners' detection signals (including domain behavior), `stack-researcher`, `clarifying-interview`, `profile-synthesizer`, every forge's stack-specific guidance, all `skill-forge/references/*.md` equivalents, and the Project Profile schema.
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
4. **Replicate the structural skeleton** into the new folder: root `AGENTS.md`/`README.md`/`CHANGELOG.md`/`.gitignore`, a fresh `VERSION` file reset to `1.0.0` (the sibling's own single version source, consumed by its profile schema and its `.infra-manifest.json` stamping - it does not inherit this generator's version), `specs/`, `tasks/` (+ `.task-counter` set to `1`), `examples/`, and the three edition trees (`.claude/`, `.cursor/`, `.codex/` + `.agents/`) with their wiring files (`settings.json`, `.cursor/hooks.json` + `rules/*.mdc`, `.codex/config.toml` + `hooks.json`) - copying Infrastructure-Creator's own wiring files as-is, since hook *registration mechanics* do not depend on the target stack.
5. **Re-author all 25 skills** for the new stack (including its own
   domain-behavior scanner and identity-swapped `stack-adapter`), using this
   generator's canonical `.agents/skills/*/SKILL.md` files as structural
   exemplars only - never copying PHP content. Parallelize in logical batches.
   Every generated skill must:
   - Ground its detection/generation logic in step 3's research.
   - Carry zero mentions of PHP or Infrastructure-Creator (this instance).
   - Cross-reference (`flow-next`/`flow-alternatives`/`related`) only skills that exist in this new set.
6. **Re-author the complete contract corpus.** Produce all six
   `skill-forge/references/*.md` equivalents (frameworks, architecture
   patterns, process skills, specialty mapping, integration catalog, and
   domain behavior), a stack-specific `candidate-registry.json` whose every
   entry resolves to a real catalog anchor, plus the Project Profile schema.
   Every mapped skill kind
   defines evidence requirements, owned/excluded scope, procedure roles,
   typed verification, provider safety where applicable, path authority,
   capability, invariant mapping, evidence anchors, routing cases,
   decision points, verification, outputs, failure handling, sibling
   boundaries, and positive/negative examples. A heading-only catalog, package
   list, or three-line summary is a failed adaptation.
7. **Copy the verbatim assets** listed above into their new-generator paths, unchanged.
8. **Mirror the three editions**: `.agents/skills` is canonical; copy it
   byte-for-byte into `.claude/skills` and `.cursor/skills`; derive Cursor's
   reduced-frontmatter agents/commands from Claude's canonical wrapper layer;
   skip agents/commands for Codex.
9. **Prove the sibling carries the quality architecture.** Verify its profile
   synthesizer emits a complete schema 1.6 evidence ledger and one operational
   contract per planned
   skill; its forge generates into staging in small evidence-scoped batches;
   wrappers and every flow artifact derive routing from one contract graph; and
   generation/update refuse
   publication before semantic PASS. Confirm every generic quality script and
   regression fixture was copied byte-for-byte.
10. **Run semantic self-verification.** In addition to structural/hook checks,
    validate the sibling's reference-contract completeness, reject source-stack
    leftovers and mechanical substitutions, run duplicate/generic/good fixtures,
    and execute a dry synthetic profile-to-staged-skills rehearsal. The current
    generator's target-runtime/manifest checks may be skipped only where they
    genuinely require a generated target; semantic checks are never skipped.
    Run the copied `validate_reference_catalogs.py --references-dir <sibling
    references> --forbid "PHP,Laravel,Symfony,PHP Core,Infrastructure-Creator"`
    (replace the forbidden identity list appropriately in the sibling's own
    identity-swapped adapter).
11. **Report.** Write `tasks/TASK-{N}/stack-adapter-report.md`, including
    reference-contract coverage, fixture results, synthetic rehearsal, and the
    final output below.

## Output Template

```markdown
# Stack Adapter Complete: Infrastructure-Creator-[Stack]

**Detected stack:** [Stack] (evidence: [file])
**New generator path:** [path]
**Skills generated:** 25 (mirrored across .claude/.cursor/.codex+.agents)
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
- MUST NOT skip re-authoring any of the 25 skills (including `domain-behavior-scanner` and `stack-adapter`'s identity-swapped copy) - a partial sibling generator is not a valid result.
- MUST NOT accept stub references, grouped skill plans, structural-only
  verification, or a sibling that omits the evidence/contract/semantic/staging
  architecture.
- MUST run the copied bad/good quality fixtures and a synthetic generation
  rehearsal before reporting the sibling ready.

## Final Output

Return the new generator's path, the detected stack and its evidence, the self-verification result, a short summary of what the new generator's skills were grounded in, and the exact next command to run there against the original target.
