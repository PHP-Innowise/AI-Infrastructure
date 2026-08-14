---
name: bootstrap-verifier
description: Run the blocking staged/published QA gate for evidence and per-skill contract conformance, semantic distinctness, routing, structure, hooks, memory runtime, manifest ownership, and placeholders.
phase: verification
flow-next: null
flow-alternatives: []
related: [infra-generate, infra-update, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer]
---

# Bootstrap Verifier

## Overview

`bootstrap-verifier` is the blocking gate for both the complete staged bundle
and the published target. It validates generated skills against the approved
evidence/contract plan before running structural, routing, hook, runtime,
ownership, and placeholder checks.

It uses the bundled dependency-free `scripts/validate_generated.py` and
`scripts/analyze_commands.py` plus targeted manual checks.

## Generated File Naming Convention (MANDATORY)

Writes a report to `tasks/TASK-{N}/bootstrap-verifier-report.md`. Does not write into the target, with two sanctioned exceptions: safe auto-fixes (e.g. restoring an executable bit) and the manifest hash refresh those auto-fixes may require (recipe below).

## Process

1. **Determine the selected editions** from the profile (section 1) and the generate report.
2. **Run static command analysis without executing target commands.** Invoke:

   `python3 scripts/analyze_commands.py --target <real-target>`

   This inventories both `composer.json` and `package.json` scripts, expands
   Composer/npm aliases transitively with cycle detection, tokenizes
   conservatively, and reports `non_mutating`, `workspace_mutation`,
   `destructive_database_deploy`, and `external_provider_network` risk. Then
   extract every command prescribed as verification in manifest-owned
   `AGENTS.md`, edition `DOD.md` files, and generated skills, and invoke one
   analysis per command:

   `python3 scripts/analyze_commands.py --target <real-target> --no-scripts --verification --command '<exact-command>'`

   Exit 1 is blocking. Shell composition, malformed tokenization, unknown or
   cyclic aliases, any workspace-writing fix/format mode, destructive/database/
   deploy behavior, and provider/network behavior fail closed when used as
   verification. Do not execute a command to discover whether it is safe.
3. **Run the validator:** distinguish the generation root (staging or published
   target) from the real evidence target:

   `python3 scripts/validate_generated.py --target <generation-root> --editions <selected> --skill-plan <task/skill-generation-plan.json> --evidence-target <real-target> --candidate-registry <generator>/skill-forge/references/candidate-registry.json`

   It checks:
   - The plan's required evidence metadata, containment, optional fingerprints
     and ranges, one complete contract per skill, and no generator task path as
     generated runtime evidence.
   - Every planned skill exists and no unplanned skill exists; required
     operational sections, procedure roles, decisions, outputs, failure
     handling, owned/excluded scope, sibling boundaries, and routing triggers
     are traceable to substantive skill content.
   - Inventory-wide ownership/write collisions, ambiguous positive routing,
     repeated substantive blocks, and line/token similarity after removing only
     exact approved fixed safety blocks.
   - Every selected edition root exists and every unselected edition root is absent (`.claude`; `.cursor`; `.agents` + `.codex` for Codex).
   - Frontmatter validity across every generated `SKILL.md`, agent, and command.
   - Every `flow-next`/`flow-alternatives`/`related`/`invokes`/`spawns` reference resolves to a skill/agent that exists in that edition.
   - Every generated hook passes `bash -n` and carries the executable bit, and the per-edition hook set is complete (eight hooks - six shared, the tool-owned `subagent-gate.sh` variant, and `subagent-dispatch.sh`; Cursor deliberately has no `working-memory-read.sh`, and Codex ships the dispatch observer unregistered).
   - Orchestration parity, when the run generated flows: every `stages` entry in a flow command names an agent that exists in that edition, no `parallel: true` stage contains more than one agent carrying `writes: true`, and each multi-stage flow declares at least one checkpoint (a single-stage read-only flow needs none). A generated accelerator whose flows name absent agents is a failed generation, not a warning.
   - Every hook wiring file (`.claude/settings.json`, `.cursor/hooks.json`, `.codex/hooks.json` + `config.toml`) references only hook scripts that exist and are executable - no dead hooks; every `.sh` token in a wired command is resolved, so an interpreter-prefixed `bash .claude/hooks/x.sh` cannot slip through.
   - The seeded `memory-bank/` passes its own `scripts/validate.py`.
   - The context-brain runtime is complete (`context.py`, `brain_runtime.py`, `context_retrieval.py`, `validate.py` under `memory-bank/scripts/`), the `project-brain/` skeleton exists, and `config/runtime.json` parses with a substituted, non-empty framework slug and a `canonical_edition` whose skills tree actually exists in the target (otherwise `context.py parity` would report false total drift).
   - Smoke: `python3 memory-bank/scripts/context.py status` and `python3 memory-bank/scripts/context.py validate` both exit 0 inside the generated tree.
   - Every selected edition contains the memory quartet skills (`memory-bank`, `project-brain`, `checkpoint`, `memory`) and their agent/command wrappers where applicable.
   - The upgrade contract: `.infra-manifest.json` exists at the target root, parses, carries a semver `generator_version`, a `TASK-{N}` reference, a valid mode (`full`/`merge`), valid selected editions, and a well-formed optional `decisions` map whose legacy `decision`, `rejected_sha256`, and `task` fields remain valid. Optional shared-file metadata (`origin`, `strategy`, proposal/resolved hashes, requirements) must be internally consistent; unknown future additive fields are tolerated. It lists no runtime state or itself, and every listed file exists with a matching sha256. Manifest membership exclusively defines ownership in both modes; unlisted target files are not opened or reported. A manifest-owned `AGENTS.md` carries the matching version/task stamp; an untracked team `AGENTS.md` is ignored.
   - A manifest-owned shared root `.gitignore` is validated through its exact
     sorted requirements and hashes. Unrelated team comments/content outside the
     managed block are not placeholder-scanned. An unchanged target-sourced
     shared file must also pass the watch-only publication baseline.
   - No template placeholders (`{skill-name}`, `TODO`, literal `YYYY-MM-DD`, `[target_name]`, `TASK-{N}`, `{{TARGET_FRAMEWORK}}`, etc.) remain in any manifest-owned text file. The only approved verbatim-asset declarations are the exact manifest-relative path-plus-regex pairs `memory-bank/templates/chunk.md` + `\bYYYY-MM-DD\b` and `memory-bank/scripts/validate.py` + `\bYYYY-MM-DD\b`. Each declaration exempts only matching occurrences, not the whole file: every other placeholder in those assets and the same ISO-date token at any other path remains blocking. Unmanifested team skills, agents, commands, hooks, memory, brain, and root documents are never scanned.
4. **Review generated operational quality.** Confirm every skill renders
   evidence-specific structured procedures and verifications with exact
   evidence IDs and target-relative line ranges or stable symbol/config-key
   anchors. Read-only skills use inspect/evaluate/compare/report wording and
   prescribe no edits. Provider/integration skills default-deny network and
   credential-backed execution and use static inspection, existing tests,
   dependency-injected fakes, local fixtures, or local adapters. Every check has
   explicit applies-when, expected/pass/fail criteria, and
   unavailable-dependency reporting with the skipped impact; a required skipped
   check remains unresolved.
5. **Confirm edition scoping passed:** treat a missing selected root or manifest-owned content under an unselected root as generation failure. An unselected root containing only unmanifested team files is outside generator ownership and is ignored.
6. **Classify failures:**
   - Auto-fixable (e.g. missing executable bit) - fix and re-run the validator. After any auto-fix that changed a file's *content*, refresh the manifest before re-running (recipe below); a permissions-only fix does not change hashes.
   - Not safely auto-fixable (e.g. a dangling cross-reference implying a forge under-produced) - escalate to the user; do not paper over it.
7. **Report** the result in `bootstrap-verifier-report.md`.

## Command Analyzer API

- `CommandAnalyzer(target)` reads only target `composer.json`/`package.json`.
- `CommandAnalyzer.analyze(command, verification=False, ecosystem=None)`
  returns `CommandAnalysis`.
- `CommandAnalyzer.analyze_scripts(verification=False)` returns analyses keyed
  as `composer:<name>` / `package:<name>`.
- `analyze_command(target, command, verification=False, ecosystem=None)` is the
  single-command convenience API.
- `analyze_target(target, commands=(), verification=False)` returns
  `TargetAnalysis`.
- `CommandAnalysis` exposes `categories`, structured `findings`,
  `expanded_commands`, traversed `aliases`, `verification_safe`, and
  `to_dict()`. The analyzer is standard-library-only and never starts a process
  other than its own CLI.

## Manifest Refresh Recipe (after content auto-fixes)

Re-hash every file the manifest already lists, in place - never add or remove entries here (set changes belong to `infra-generate`/`infra-update`'s full recipe):

```bash
python3 - "<path-to-target>" <<'PY'
import hashlib, json, sys, time
from pathlib import Path
target = Path(sys.argv[1]).resolve()
mp = target / ".infra-manifest.json"
manifest = json.loads(mp.read_text(encoding="utf-8"))
missing = []
for rel in list(manifest["files"]):
    p = target / rel
    if p.is_file():
        manifest["files"][rel] = hashlib.sha256(p.read_bytes()).hexdigest()
    else:
        missing.append(rel)
manifest["generated_at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
mp.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
print(f"refreshed {len(manifest['files'])} hash(es)"
      + (f"; STILL MISSING: {missing}" if missing else ""))
PY
```

A non-empty `STILL MISSING` list means a tracked file vanished - that is an escalation, not something to silently drop from the manifest.

## Output Template

```markdown
# Bootstrap Verifier Report: [target_name]

**Editions checked:** [selected]
**Result:** [PASS / FAIL]

## Checks
- Evidence/plan completeness and fingerprints: [pass/fail]
- Per-skill contract conformance: [pass/fail]
- Command safety (all prescribed verification resolves + non-mutating): [pass/fail]
- Operational quality (specific procedures/anchors/pass-fail/skip): [pass/fail]
- Provider safety (default-deny network + fake/local strategy): [pass/fail]
- Read-only wording: [pass/fail]
- Inventory ownership/distinctness/routing: [pass/fail]
- Frontmatter: [pass/fail]
- Cross-references: [pass/fail]
- Hooks (bash -n + exec bit + per-edition set): [pass/fail]
- Hook wiring (all wired scripts exist + executable): [pass/fail]
- Edition scope (selected present, unselected absent): [pass/fail]
- Memory bank validate.py: [pass/fail]
- Context-brain runtime + project-brain skeleton: [pass/fail]
- Smoke (context.py status / validate): [pass/fail]
- Manifest (.infra-manifest.json membership + hashes + tracked AGENTS.md stamp): [pass/fail]
- No placeholders: [pass/fail]
- Edition scoping: [pass/fail]

## Auto-fixed
- [list]

## Needs Human Attention
- [list, or "none"]
```

## Guardrails

- MUST treat any unresolved failure as "generation not done"; MUST NOT let `infra-generate` report success on failure.
- MUST NOT auto-fix anything ambiguous (e.g. rewrite a skill to satisfy a reference) - escalate instead.
- MUST require `--skill-plan` and `--evidence-target` for every generation or
  update gate; structural-only invocation is for validator maintenance tests,
  not release approval.
- MUST confirm no unselected edition was generated.
- MUST run the seeded memory bank's own validator, not a substitute.
- MUST keep placeholder exemptions occurrence-scoped to an explicitly approved manifest-relative path-plus-regex declaration; MUST NOT exempt a whole file or a placeholder pattern globally.
- MUST refresh the manifest hashes (recipe above) after any auto-fix that changed file content, and re-run the validator - a manifest describing pre-fix content is a broken upgrade contract.
- MUST NOT add or remove manifest entries merely to make the validator pass. Membership changes require the explicit `infra-generate` or `infra-update` write plan; unmanifested on-disk files are team-owned and are not a validation failure.
- MUST analyze every generated verification command against the real evidence
  target and fail closed on shell composition, tokenization uncertainty,
  unknown/cyclic aliases, mutation, destructive/database/deploy behavior, or
  external/provider/network risk.
- MUST NOT execute target commands as part of command-risk analysis.
- MUST reject bare-path evidence, generic procedures/verifications, mutating
  read-only wording, provider checks without a fake/local default, or silent
  skipped checks.

## Final Output

Return PASS/FAIL, the per-check results, what was auto-fixed, and what needs human attention. On PASS, `infra-generate` may report success.
