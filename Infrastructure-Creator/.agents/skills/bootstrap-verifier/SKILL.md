---
name: bootstrap-verifier
description: Run the final QA gate for a freshly generated accelerator before infra-generate is allowed to report success - validates frontmatter and cross-references across every generated skill/agent/command, every generated hook's syntax/executable bit and wiring, the seeded memory-bank validator, the context-brain runtime, the .infra-manifest.json upgrade contract, and leftover template placeholders. Takes a required target-project-path argument. Use as the last step of infra-generate or infra-update. Triggers on "verify the generated accelerator", "bootstrap-verifier", "run the QA gate", "check what infra-generate produced".
phase: verification
flow-next: null
flow-alternatives: []
related: [infra-generate, infra-update, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer]
---

# Bootstrap Verifier

## Overview

`bootstrap-verifier` is the last step of `infra-generate` and of `infra-update`. It mechanically checks that the generated accelerator is internally consistent and immediately usable, for the selected edition(s) only. A failed run means generation is not done - it must be fixed and re-run before success is reported.

It uses the bundled `scripts/validate_generated.py` (dependency-free) plus targeted manual checks.

## Generated File Naming Convention (MANDATORY)

Writes a report to `tasks/TASK-{N}/bootstrap-verifier-report.md`. Does not write into the target, with two sanctioned exceptions: safe auto-fixes (e.g. restoring an executable bit) and the manifest hash refresh those auto-fixes may require (recipe below).

## Process

1. **Determine the selected editions** from the profile (section 1) and the generate report.
2. **Run the validator:** `python3 scripts/validate_generated.py --target <target> --editions <selected>`. It checks:
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
   - The upgrade contract: `.infra-manifest.json` exists at the target root, parses, carries a semver `generator_version`, a `TASK-{N}` reference, a valid mode (`full`/`merge`), valid selected editions, and a well-formed optional `decisions` map whose `decision`, `rejected_sha256`, and `task` fields are valid. It lists no runtime state or itself, and every listed file exists with a matching sha256. Manifest membership exclusively defines ownership in both modes; unlisted target files are not opened or reported. A manifest-owned `AGENTS.md` carries the matching version/task stamp; an untracked team `AGENTS.md` is ignored.
   - No template placeholders (`{skill-name}`, `TODO`, literal `YYYY-MM-DD`, `[target_name]`, `TASK-{N}`, `{{TARGET_FRAMEWORK}}`, etc.) remain in any manifest-owned text file. Unmanifested team skills, agents, commands, hooks, memory, brain, and root documents are never scanned.
3. **Confirm edition scoping passed:** treat a missing selected root or manifest-owned content under an unselected root as generation failure. An unselected root containing only unmanifested team files is outside generator ownership and is ignored.
4. **Classify failures:**
   - Auto-fixable (e.g. missing executable bit) - fix and re-run the validator. After any auto-fix that changed a file's *content*, refresh the manifest before re-running (recipe below); a permissions-only fix does not change hashes.
   - Not safely auto-fixable (e.g. a dangling cross-reference implying a forge under-produced) - escalate to the user; do not paper over it.
5. **Report** the result in `bootstrap-verifier-report.md`.

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
- MUST confirm no unselected edition was generated.
- MUST run the seeded memory bank's own validator, not a substitute.
- MUST refresh the manifest hashes (recipe above) after any auto-fix that changed file content, and re-run the validator - a manifest describing pre-fix content is a broken upgrade contract.
- MUST NOT add or remove manifest entries merely to make the validator pass. Membership changes require the explicit `infra-generate` or `infra-update` write plan; unmanifested on-disk files are team-owned and are not a validation failure.

## Final Output

Return PASS/FAIL, the per-check results, what was auto-fixed, and what needs human attention. On PASS, `infra-generate` may report success.
