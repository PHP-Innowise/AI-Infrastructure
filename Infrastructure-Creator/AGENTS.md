# AGENTS.md - Infrastructure-Creator Policy

These are enforceable rules for Infrastructure-Creator itself. Wishes are ignored; constraints are enforced.

Infrastructure-Creator is a standalone **generator** for **PHP projects**: a
two-phase, multi-agent pipeline that scans an arbitrary target PHP project,
researches its specific framework and integrations, asks the few questions it
could not answer from evidence, and produces a bespoke accelerator. Candidate
output is generated and validated in task-scoped staging, then transactionally
published into the target. Every generated artifact is derived from evidence
found in the target, never from a bundled prose template.

This policy is shared across the editions in which the **generator itself** ships. The generator is mirrored for **Claude Code** (`.claude/`), **Cursor** (`.cursor/`), and **Codex** (`.agents/skills` + `.codex/`) so any team member can run it from whichever tool they use. This is distinct from what the generator *produces*: the accelerator it writes into a target project contains only the edition(s) that target's team selects during `clarifying-interview`.

## Hierarchy of Sources of Truth

1. **Policy** (this `AGENTS.md`) - mandatory behavior and safety rules for the generator.
2. **Enforcement** (each edition's hooks) - runtime guards that block unsafe actions.
3. **Evidence** (the target project's actual files: `composer.json`, config, PHP source, CI, IaC) - the only legitimate source for scan findings; never inferred from framework popularity or assumption.
4. **Runtime truth** (`specs/`, `tasks/TASK-{N}/`) - this tool's own working documents for a given run.
5. **Operations** (`skills/`) - how the generator's skills execute.
6. **Examples** (`examples/`) - reference outputs, never stronger than policy.
7. **Documentation** (`README.md`) - human reference.

Infrastructure-Creator intentionally has no `memory-bank/` of its own: its job is to seed a `memory-bank/` **for the target project**, not to accumulate memory about unrelated scans.

## PHP Scope (MANDATORY)

- The target is assumed to be a PHP project (plain PHP or a PHP framework such as Laravel, Symfony, Slim, Laminas, CodeIgniter, or similar). Scanners MUST ground every finding in PHP evidence: `composer.json`/`composer.lock`, `*.php` sources, PHP framework entry points (`public/index.php`, `artisan`, `bin/console`), and PHP tooling config (`phpunit.xml`, `phpstan.neon`, `psalm.xml`, `.php-cs-fixer.dist.php`, `pint.json`, `rector.php`).
- Non-PHP components that a PHP app talks to (a JS/TS frontend, a managed queue, an external microservice) MUST be captured only as lightweight **integration contracts**, never given deep generated skills.
- If the target has no PHP evidence at all (`no composer.json`, no `*.php`), `infra-scan` MUST NOT generate a non-PHP accelerator directly. It MUST probe for a recognizable non-PHP stack and, if found, offer `stack-adapter` (see below); if nothing recognizable is found, it MUST stop and report the target out of scope.
- This instance of Infrastructure-Creator only ever generates PHP accelerators directly. Building tooling for another stack is never done in place - it is always delegated to `stack-adapter`, which produces a separate, fully independent sibling tool (see "Self-Adaptation" below). Infrastructure-Creator itself never becomes multi-stack.

## Independence (MANDATORY)

- Infrastructure-Creator is fully self-contained. Generated and authored content MUST NOT reference, copy from, or depend on any other accelerator or sibling folder.
- All reference material the generator needs (integration catalog, framework detection signals, architecture patterns, the memory-bank validator and chunk template) is bundled inside this folder's own skills.

## Self-Adaptation (MANDATORY, SCOPED)

- `stack-adapter` is the sanctioned exception to "PHP only": when a target is not PHP but is a recognizable stack, it MAY build an entirely separate generator - `Infrastructure-Creator-[Stack]/`, a sibling folder next to this one - with the same architecture, freshly researched and authored for that stack.
- The sibling generator produced by `stack-adapter` MUST meet the exact same independence bar as this one: zero mentions of PHP, Laravel, Symfony, PHP Core, or "Infrastructure-Creator" anywhere in its own content.
- `stack-adapter` MUST NOT run without explicit user confirmation - a detected foreign stack is evidence, never consent.
- This is a deliberate, honest-scope choice: offering to build the right tool for the job, rather than silently failing on a non-PHP target or stretching this generator to cover stacks it was never designed for.

## Workspace Boundary (MANDATORY)

- MUST run from a location **outside** the target project (its own workspace, or a sibling folder) - never copied into the target project.
- MUST treat the target project path as an explicit, required argument to every skill (e.g. "run infra-scan against ../my-app"). MUST NOT assume the current working directory is the target unless the user explicitly confirms it.
- MUST NOT read or write anything outside the target path during scanning/generation, except this folder's own `tasks/` and `specs/` (for the run's own working documents).
- MUST detect and refuse to silently overwrite a target that already has `AGENTS.md` or any AI-tool edition folder (`.claude/`, `.cursor/`, `.codex/`, `.agents/`) - stop and ask the user to choose overwrite, merge, or abort before writing anything.

## Generation Philosophy (MANDATORY)

- MUST generate every artifact from what was actually found in the target (`composer.json`, config, PHP source, CI/CD, IaC). MUST NOT invent an integration, framework, or service that has no evidence in the target.
- MUST cite concrete evidence (file path, and line numbers or a matching excerpt when practical) for every scan finding and every claim carried into a generated artifact.
- MUST mark any unverifiable or low-confidence finding as `inferred` or `unknown` rather than presenting a guess as fact, and MUST route genuinely ambiguous items through `clarifying-interview`.
- Behavioral findings MUST also preserve source type (`spec/ADR`, `test`, `database constraint`, `workflow configuration`, `authorization rule`, code/configuration, or `interview answer`) and surface contradictions. Implementation behavior and user testimony MUST NOT be silently promoted to stronger authority.
- Generated skills/agents/commands MUST conform to the standard `SKILL.md`/agent/command frontmatter and structure documented in each forge skill's own `SKILL.md`.
- The Project Profile MUST compile into a machine-readable evidence ledger and
  one complete generation contract per proposed skill. Group summaries are not
  generation contracts.
- Every proposed skill MUST prove distinct selection, owned scope, procedure,
  output, evidence, and routing value. Catalog membership alone is not a reason
  to generate it; unsupported or overlapping skills are pruned or merged.
- Generated skills MUST cite canonical target-relative sources and contain
  project-specific procedures, decision points, verification, outputs, and
  failure handling. Generator task paths are provenance only and MUST NOT be a
  runtime evidence dependency of a generated target.
- Agents MUST remain thin wrappers but carry contract-derived positive/negative
  routing and sibling deferrals. Flows MUST select specialist agents by matching
  scope, never run every available specialist unconditionally.
- Generated output is tool-selected: `policy-forge`, `skill-forge`, `agent-forge`, `command-forge`, and `hook-forge` produce ONLY the edition(s) the target team selected in `clarifying-interview` - never more editions than selected, never fewer.
- The generator's version has exactly one source of truth: the root `VERSION` file. The Project Profile's metadata, the target's `AGENTS.md` stamp, and `.infra-manifest.json` all read it; nothing hardcodes or recalls a version from the changelog.
- Generated output is upgradeable: every successful `infra-generate` run MUST
  stage and validate the complete bundle, publish with rollback, then leave a
  stamped target `AGENTS.md` and `.infra-manifest.json`. `infra-update` MUST NOT
  overwrite any file whose hash differs from that manifest without an explicit
  per-file human decision, and MUST NOT touch files the manifest does not list.

## Orchestration Exception (MANDATORY, SCOPED)

The general accelerator rule is "an agent executes exactly one skill, then stops; it never auto-chains." Five skills in this folder are a deliberate, narrowly scoped exception, because pipeline orchestration is their entire purpose:

- `infra-scan` MAY fan out to the seven scanner skills (including `domain-behavior-scanner`), `stack-researcher`, `clarifying-interview`, and `profile-synthesizer` in one run. It MAY also hand off to `stack-adapter` when a non-PHP stack is detected and the user opts in.
- `infra-generate` MAY fan out evidence-independent forges into staging, but
  MUST generate skills in bounded contract-driven batches and pass semantic
  validation before agent/command/flow generation, publication, or success.
- `infra-build` MAY chain `infra-scan` then `infra-generate` in one run, pausing at the profile checkpoint only when a blocking ambiguity or a collision is detected.
- `infra-update` MAY re-validate the profile, fan out the forge skills into its staging directory, apply manifest-verified safe replacements, rewrite `.infra-manifest.json`, and run `bootstrap-verifier` in one run. It writes into the target only what the target's `.infra-manifest.json` proves untouched (sha256 match) or what the user explicitly approved per file; without that manifest it MUST abort.
- `stack-adapter` MAY research, replicate, re-author, mirror, and self-verify an entire sibling generator in one run, after explicit user confirmation.

Fan-out runs in parallel when the AI tool supports concurrent subagents/tool calls; otherwise sequentially in one session. Every other skill still follows the standard rule when invoked on its own: execute, stop, report, suggest a next step.

## Agent Behavior

- MUST output a Context Summary and Next Steps at the end of every skill.
- MUST NOT make workflow decisions for the user beyond the five sanctioned orchestrators above.
- MUST read the target's actual `composer.json`/config/PHP source/CI/IaC before making any claim about it.
- MUST NOT read, print, or write the target's `.env` files, credentials, or anything under a `secrets/`-style path.
- MUST re-validate a `profile-synthesizer` profile against the target's current files before `infra-generate` consumes it, and MUST flag drift if the target changed since the scan.

## Subagents

- MUST delegate only through this accelerator's own agents and skills; the
  host tool's built-in subagents (Claude Code's Explore/Plan/general-purpose,
  Cursor's explore/bash/browser, Codex's spawn_agent roles) are disabled by
  configuration and denied by the `subagent-gate` hook.
- MUST NOT retry a denied subagent spawn; when no project agent fits the
  task, do the work in the main conversation instead.

## File Naming

- MUST prefix generated task/spec markdown in this folder with the skill name: `{skill-name}-{purpose}.md` (e.g. `infra-scan-project-profile.md`, `stack-scanner-findings.md`).
- MUST use zero-padded task directories under `tasks/`: `TASK-001/`, `TASK-002/`, one per scan/generate run against a given target.
- MUST check `tasks/.task-counter` before creating a new task directory.
- MUST NOT create unprefixed markdown files in `tasks/` or `specs/`, except `README.md`, `CHANGELOG.md`, and `MANIFEST.md`.

## Verification

- MUST run the applicable checks in `DOD.md` before claiming a scan or generation is complete.
- MUST run `bootstrap-verifier` (evidence and per-skill contract conformance,
  semantic distinctness, routing, frontmatter/cross-references, hook
  syntax/wiring, memory runtime, manifest ownership/hashes, and placeholders)
  against staging and again after publication before reporting
  `infra-generate` or `infra-update` as done.
- MUST report unavailable tooling as `N/A - tooling not configured`; do not install tooling without user approval.

## Git Safety

- MUST NOT skip hooks with `--no-verify`.
- MUST NOT force-push, hard-reset, or drop database tables in the target project without explicit user consent.
- MUST NOT overwrite unrelated user changes in the target project.

## Security

- MUST NOT read, print, edit, or commit `.env` files or secrets in either this folder or the target project.
- MUST NOT carry secrets, credentials, tokens, customer data, raw production payloads, logs, or unsanitized incident details from the target project into any generated file, scan finding, or seeded memory chunk.
- MUST keep personal/local scratch notes, if any, out of version control.

## Definition Of Done

- See `DOD.md` (per edition) for the tiered verification checklist for this generator's own work.
- MUST include verification evidence in the final Context Summary whenever a scan or generation run is performed.
