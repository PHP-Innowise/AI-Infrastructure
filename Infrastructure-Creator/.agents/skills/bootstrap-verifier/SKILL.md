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
   cyclic aliases, shell interpreters running `-c` strings (including
   clustered forms such as `-lc`) or script files, sudo, xargs/variable
   indirection, executables outside the curated read-only allow-list
   (`UNKNOWN_EXECUTABLE`), alias expansion beyond the safety cap, any
   workspace-writing fix/format mode, destructive/database/deploy behavior,
   and provider/network behavior fail closed when used as verification.
   Wrapper commands (`env`, `nice`, `nohup`, `stdbuf`, `sudo`, `timeout`) are
   unwrapped by basename and the wrapped command is classified. Blocked or
   unresolvable commands additionally report the `verification_blocker`
   category in JSON output instead of defaulting to `non_mutating`. Do not
   execute a command to discover whether it is safe.

   Generated skill *prose* is analyzed mechanically by
   `validate_skill_quality.py` (step 3), so this manual pass covers `AGENTS.md`
   and `DOD.md` commands; do not skip a skill body that the validator already
   reported.
3. **Run the validator:** distinguish the generation root (staging or published
   target) from the real evidence target:

   `python3 scripts/validate_generated.py --target <generation-root> --editions <selected> --skill-plan <task/skill-generation-plan.json> --evidence-target <real-target> --candidate-registry <generator>/skill-forge/references/candidate-registry.json`

   It checks:
   - The plan's required evidence metadata, containment, optional fingerprints
     and ranges, one complete contract per skill, and no generator task path as
     generated runtime evidence.
   - Every `evidence_anchors[].anchor` resolves against the file it cites, not
     only against its shape: an `L` range must lie inside the real bounds of
     that file and run forwards (`EVIDENCE_ANCHOR_RANGE`), and a `symbol:`
     anchor's final segment must actually occur there
     (`EVIDENCE_ANCHOR_SYMBOL_ABSENT`, case-insensitive whole-identifier
     match, so a qualified `Ns\Class::member` resolves through `member`). This
     is the same bound `evidence[].line_range` already carries. An unreadable
     source is reported (`EVIDENCE_ANCHOR_UNRESOLVABLE`), never raised.
   - Each `evidence[].supported_claims` entry must share with its cited range
     wording that no other PHP file would: sharing only PHP keywords is
     `EVIDENCE_CLAIM_UNSUPPORTED`, sharing only software-English boilerplate
     is `EVIDENCE_CLAIM_GENERIC_SUPPORT` (warning). Cited identifiers split
     (`publishReminder` grounds "reminder"). Polarity is beyond any such rule
     - `docs/ADR-001-claim-adjudication.md`.
   - Every planned skill exists and no unplanned skill exists; required
     operational sections, procedure roles, decisions, outputs, failure
     handling, owned/excluded scope, sibling boundaries, and routing triggers
     are traceable to substantive skill content.
   - That the skill *instructs*, not only that it names the project.
     Traceability is lexical, so a body can carry the real paths, classes, and
     constants everywhere and still prescribe nothing. Every procedure step is
     therefore read structurally: it must command an action - a verb from the
     open technical set (`inspect`, `trace`, `run`, `reject`, `defer`, ...) or
     a prescribed invocation - otherwise `SKILL_STEP_NOT_OPERATIONAL`; and a
     step (or a sentence inside it) that *opens* by deferring to reflection -
     `consider`, `take into account`, `bear in mind`, `reflect on`,
     `form an opinion`, `think about`, `keep in mind` - is
     `SKILL_STEP_HEDGED`. The procedure as a whole must name at least one
     concrete anchor (path, symbol, command, constant, quoted value, or
     number), counted over step text only, never over the `1.` list markers
     (`SKILL_PROCEDURE_UNANCHORED`). Verification must state a check: an
     impression (`still looks reasonable`, `nothing seems broken`) is
     `SKILL_VERIFICATION_NOT_FALSIFIABLE` and a section naming only subject
     nouns is `SKILL_VERIFICATION_NOT_OPERATIONAL`. The same reading applies
     to the plan: `procedure_steps[].action`
     (`PROCEDURE_STEP_NOT_OPERATIONAL`), a step that names no `path_refs`, no
     `evidence_ids`, and no anchor in its own text
     (`PROCEDURE_STEP_UNANCHORED`), and `verification[]`
     (`VERIFICATION_NOT_FALSIFIABLE`). The `GENERIC_PHRASES` blacklist is only
     the secondary net; the verb/anchor structure is the mechanism, because a
     blacklist is defeated by one paraphrase. Calibration is deliberate: the
     verb set is open and matched anywhere in the step, mid-sentence hedging
     next to a real action stays legitimate, and no step is required to carry
     an anchor of its own - a gate that fails honest instruction is worse than
     the miss it closes.
   - Every command a skill body hands to the agent, not only the plan's
     `verification[].command`. Fenced code blocks tagged `bash`, `sh`, `shell`,
     or `console` are read strictly (transcript prompts stripped, backslash
     continuations joined, `#` comments dropped). Inline single-backtick spans
     are read permissively: a segment counts only when it names a known
     executable and carries at least one argument, and a bare English-word
     executable (`test`, `install`) counts only when path-qualified - so class
     names, config paths, constants, YAML keys, and PHP fragments in backticks
     are never mistaken for commands. Every command is split at quote-aware
     shell separators so a composed chain is classified leaf by leaf, and
     non-breaking/zero-width characters are folded first so a homoglyph cannot
     hide the executable. Arguments of a code host (`bash`/`sh`/`zsh`/`dash`/
     `fish`, `php`/`node`/`python`/`python3`) are re-read as nested commands so
     a payload quoted behind `bash -c` or `php -r` is classified; every other
     executable keeps its arguments as data, so `grep -rn "rm -rf" config/`
     stays a grep. Destructive, workspace-mutating, provider/network, or
     `sudo` behavior is blocking (`SKILL_BODY_COMMAND_RISK`); attestability-only
     findings (unknown executable, unresolved alias, tokenization) are not,
     because prose legitimately carries placeholders and sample output.
   - A literal `grep` in `verification[].command` is resolved offline, never
     run, and graded on `expected_result`: no match `VERIFICATION_SEARCH_DEAD`,
     a named path missing `VERIFICATION_SEARCH_EXPECTATION`, an `only` claim
     the target denies `VERIFICATION_SEARCH_EXCLUSIVITY`. Other forms skip.
   - Every target path the *rendered body* cites, not only the plan's. A bare
     path code span rooted in the target tree - first segment and parent
     directory both present - that resolves to nothing is
     `SKILL_BODY_PATH_MISSING`; an evidence row must keep the identifier and
     path the plan declares (`SKILL_EVIDENCE_ROW_UNDECLARED`,
     `SKILL_EVIDENCE_ROW_ANCHOR`). Skipped, being indistinguishable from an
     honest citation: backslashed class names, dotted keys, bare file names,
     globs/placeholders, `vendor`/`node_modules`/`var`, foreign layouts and
     Twig logical names, declared `writes`, a line commanding creation, and
     any string the skill's own cited source spells out. So an invented path
     under a directory the target lacks is missed - deliberately.
   - Inventory-wide ownership/write collisions, ambiguous positive routing,
     repeated substantive blocks, and line/token similarity after removing only
     exact approved fixed safety blocks. Ownership is checked against writes,
     not only writes against writes: no skill may write a path another skill
     holds under `mode: exclusive` (`OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT`), so
     a read-only owner - whose own `writes` is empty by contract - is protected
     too. `shared`/`composed` zones keep their own rules.
   - Duplication is judged twice: once on the raw text (`SKILL_SIMILARITY`,
     `REPEATED_BLOCK`) and once on a *skeleton* of it (`SKILL_TEMPLATE_REUSE`,
     `SKILL_TEMPLATE_BLOCK`). The raw pass compares normalized lines for
     equality, so it only ever saw byte-identical prose - and a
     plan-conforming `SKILL.md` is *required* to carry its own claim, paths
     and neighbours' names, so those mandated differences dilute raw
     similarity below any usable threshold. The skeleton pass therefore erases
     exactly that identity first - backticked spans, paths, CamelCase and
     `UPPER_SNAKE` identifiers, dotted/underscored ids, PHP variables and
     calls, numbers, and the inventory's other skill names all collapse to one
     placeholder - drops fenced code (a shared framework idiom is legitimate)
     and approved fixed blocks, folds punctuation and interchangeable
     connectives (`,`/`;`, `and`/`plus`), and ignores lines that are almost
     entirely identity. What remains is the reusable scaffolding, compared by
     one-to-one line matching plus token trigrams. Thresholds are measured,
     not guessed: across 2371 pairs of honest hand-written skills the worst
     pair scores line 0.350 / token 0.154 (and that pair is a deliberately
     parallel scanner family; unrelated skills top out at 0.231 / 0.107),
     while one template with substituted project nouns scores 0.400 / 0.329,
     the same template differing only in punctuation 0.400 / 0.311, and a byte
     clone 0.500 / 0.471. `SKILL_TEMPLATE_REUSE` blocks at 0.38 / 0.26, inside
     that gap on both axes; a warning sits at 0.28 / 0.20.
     `SKILL_TEMPLATE_BLOCK` (two adjacent shared skeleton lines, down from
     three byte-identical ones) is deliberately a *warning*: on the same
     honest corpus it fires five times on legitimately shared policy
     sentences, so it names a passage worth reading rather than failing a
     generation.
   - A declared primary/defer precedence resolves a *partial* overlap only. If
     two skills claim the same `owned_scope` entry, `ownership[].description`,
     or positive trigger word for word, that is
     `BOUNDARY_NOT_SEPARATING` regardless of the declared precedence: a
     boundary that repeats itself divides nothing. Nested or otherwise partial
     overlap under an explicit precedence stays legitimate.
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
   - Optional `--baseline-plan <prior plan>` on `validate_skill_quality.py`
     names coverage this run lost: each absent skill with the paths it cited
     (`BASELINE_SKILL_DROPPED`), each `evidence[].path`/`source_paths` entry
     covered there and not here (`BASELINE_COVERAGE_DROPPED`), both repeated
     by name on the `coverage baseline:` line (`coverage_baseline` under
     `--json`). Warnings, not errors - re-composition can be honest, silence
     never is. An unreadable baseline reads `NOT COMPARED`
     (`BASELINE_PLAN_UNREADABLE`), not "nothing lost". No flag, no change.
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
- `RUNNER_EXECUTABLES` is the public set of every executable name the module
  recognises (including the unwrapped wrappers). Callers deciding whether a
  token found in prose is plausibly a command at all gate on it.
- `CODE_HOST_EXECUTABLES` is the public set of executables that run code handed
  to them as an argument, so a caller can re-read that argument as a nested
  command instead of treating it as opaque data.
- `CommandAnalysis` exposes `categories` (the four risk categories, plus
  `verification_blocker` when the command is blocked or unresolvable),
  structured `findings`, `expanded_commands`, traversed `aliases`,
  `verification_safe`, and `to_dict()`. The analyzer is standard-library-only
  and never starts a process other than its own CLI.

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
- Skill-body command safety (no destructive/mutating/provider command in prose): [pass/fail]
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
- MUST treat a command prescribed in a generated skill body as prescribed
  behavior: destructive, mutating, provider/network, or `sudo` commands in
  prose are blocking even when the plan's verification commands are clean.
  MUST NOT widen that scan to fail a skill for an unknown executable,
  unresolved alias, or placeholder inside prose - a gate that rejects honest
  skills is worse than the miss it closes.
- MUST treat a skill that names the project but commands nothing as a failed
  generation: a procedure step that only invites reflection, a procedure that
  anchors nothing, or a verification that accepts an impression is not
  operational content, however project-specific its vocabulary is. MUST NOT
  turn that reading into a style gate - an unusual project verb, a delegation
  step, or a step without an anchor of its own is honest instruction.
- MUST NOT execute target commands as part of command-risk analysis.
- MUST reject bare-path evidence, generic procedures/verifications, mutating
  read-only wording, provider checks without a fake/local default, or silent
  skipped checks.

## Final Output

Return PASS/FAIL, the per-check results, what was auto-fixed, and what needs human attention. On PASS, `infra-generate` may report success.
