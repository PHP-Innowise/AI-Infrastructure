---
name: infra-generate
description: Turn an approved Project Profile into a real, working accelerator inside the target PHP project - policy, skills, agents, commands, hooks, and a seeded memory bank - for only the selected AI-tool edition(s). Use after infra-scan has produced and the user has reviewed tasks/TASK-{N}/infra-scan-project-profile.md. Triggers on "infra-generate", "generate the accelerator now", "build the infrastructure for my project", "run phase two".
phase: orchestration
flow-next: null
flow-alternatives: []
related: [policy-forge, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier, infra-scan, infra-update]
---

# Infra Generate

## Overview

`infra-generate` is the second orchestrator (see `AGENTS.md`'s "Orchestration Exception"). It is the only point in the pipeline that writes into the target project. It consumes an approved `infra-scan-project-profile.md`, fans out the forge skills, wraps their output with agents/commands, composes the flow, and runs a final verification pass before declaring the target's new accelerator ready - writing only the AI-tool edition(s) the profile selected.

## Generated File Naming Convention (MANDATORY)

This skill's own run notes live in `tasks/TASK-{N}/infra-generate-report.md`. Everything else it produces is written into the **target project's own root**, using the conventions that `policy-forge` just wrote for it.

## Process

1. **Locate the profile.** Require the target project path (must match a profile from `infra-scan`); if more than one `TASK-{N}/` exists for that target, use the most recent unless the user specifies one.
2. **Re-validate before trusting it.** Re-check cited evidence against current files, including section 8's canonical behavioral sources and section 12's memory concepts. Preserve contradictions and source type. If something changed, flag drift and ask whether to re-scan or explicitly accept stale parts.
3. **Read the selected editions** from the profile's section 1 (AI Tool Selection). Only these editions will be produced.
4. **Collision guard.** If the target already has `AGENTS.md` or any selected edition folder, STOP and ask: overwrite, merge (add only what is missing, never touch existing files), or abort. Do not proceed on assumption. **When the user chooses merge**, snapshot the pre-existing surface BEFORE any forge writes: record every file currently present under `AGENTS.md` plus the manifest include roots (`.claude`, `.cursor`, `.agents`, `.codex`, `memory-bank`, `project-brain`) into `tasks/TASK-{N}/preexisting-files.txt`, one target-relative path per line. The manifest recipe consumes this list so that not a single pre-existing team file is ever claimed as generator-owned - a claimed file would be silently overwritten by the next `infra-update` as a "safe update".
5. **Fan out the four independent forges.**
   - **Parallel-capable tools:** spawn `policy-forge`, `skill-forge`, `hook-forge`, and `memory-seed` together - none need each other's output, only the profile.
   - **Single-threaded tools:** run the same four sequentially in one session.
6. **Wrap the generated skills.** Once `skill-forge` has produced the final skill list, run `agent-forge` (generates matching agent wrappers for editions that carry them), then `command-forge` (wraps for editions with a command layer; skipped for Codex).
7. **Compose the flow.** Run `skill-flow-composer` once every skill/agent/command exists, to build the target's own `SKILL FLOW.md`.
8. **Stamp and write the manifest.** Once every forge is done and before verification (see "Version Stamp & Generation Manifest" below): prepend the version-stamp comment to the generated `AGENTS.md`, then write `.infra-manifest.json` at the target root using the exact recipe below. `infra-update` depends on this manifest - a generation without it is a legacy target that can never be upgraded safely.
9. **Verify.** Run `bootstrap-verifier` last - it now also checks the manifest (presence, coverage, matching hashes, the `AGENTS.md` stamp). Treat a failed verification as generation not done - auto-fix what is safe (e.g. a missing executable bit), refresh the manifest hashes after any content auto-fix (recipe in `bootstrap-verifier`'s SKILL.md), and re-run; escalate anything it cannot safely fix (e.g. a dangling cross-reference) to the user.
10. **Report.** Write `tasks/TASK-{N}/infra-generate-report.md` summarizing what was written where, for which edition(s), the manifest file count, and the verification results.

## Version Stamp & Generation Manifest (MANDATORY)

The generator's version has a single source of truth: the `VERSION` file at this generator's root. Never hardcode a version and never recall one from the changelog.

**Step A - stamp `AGENTS.md`.** Applies only to an `AGENTS.md` this run created: in merge mode a pre-existing `AGENTS.md` is never touched (it stays unstamped and untracked; `bootstrap-verifier` skips the stamp check for it). When this run did create it, make the FIRST line of the target's generated `AGENTS.md` this comment (real values, no placeholders), before hashing anything:

```markdown
<!-- Generated by Infrastructure-Creator v1.4.0 | TASK-003 | 2026-08-02 -->
```

- version = contents of this generator's `VERSION` file;
- task id = this run's `tasks/TASK-{N}/`;
- date = today, ISO `YYYY-MM-DD`.

**Step B - write `.infra-manifest.json`.** The manifest records the generator version, the profile this run consumed, the generation `mode` (`full` or `merge`), and the sha256 of every **generator-owned** file just written - in merge mode that means only files this run actually created, never files the team already had. Runtime state is deliberately NOT tracked - `memory-bank/chunks/`, `memory-bank/INDEX.md`, `memory-bank/.memory-counter`, `memory-bank/local/`, `project-brain/indexes/`, and the `project-brain/` record/state directories (`archive/`, `control/`, `dynamic/`, `local/`) belong to the target team from the moment they are seeded, and `infra-update` never touches a file the manifest does not list.

Exact recipe - run from this generator's root, after Step A, substituting the target path, this run's task id, the selected editions, and the mode. For an overwrite/fresh generation the mode is `full`; for a merge generation pass `merge` plus the path to `preexisting-files.txt` from step 4:

```bash
# full:  python3 - "<target>" "$(cat VERSION)" "TASK-003" "claude,cursor" "full" <<'PY' ...
# merge: python3 - "<target>" "$(cat VERSION)" "TASK-003" "claude,cursor" "merge" "tasks/TASK-003/preexisting-files.txt" <<'PY' ...
python3 - "<path-to-target>" "$(cat VERSION)" "TASK-003" "claude,cursor" "full" <<'PY'
import hashlib, json, sys, time
from pathlib import Path

target = Path(sys.argv[1]).resolve()
version, task = sys.argv[2].strip(), sys.argv[3].strip()
editions = [e.strip() for e in sys.argv[4].split(",") if e.strip()]
mode = sys.argv[5].strip() if len(sys.argv) > 5 else "full"
assert mode in ("full", "merge"), f"mode must be full|merge, got {mode!r}"
preexisting = set()
if mode == "merge":
    preexisting = {
        line.strip()
        for line in Path(sys.argv[6]).read_text(encoding="utf-8").splitlines()
        if line.strip()
    }

INCLUDE_ROOTS = (".claude", ".cursor", ".agents", ".codex",
                 "memory-bank", "project-brain")
ROOT_FILES = ("AGENTS.md",)
STATE_EXCLUDES = ("memory-bank/chunks/", "memory-bank/INDEX.md",
                  "memory-bank/.memory-counter", "memory-bank/local/",
                  "project-brain/indexes/", "project-brain/local/",
                  "project-brain/archive/", "project-brain/control/",
                  "project-brain/dynamic/")

def owned(rel):
    if rel == ".infra-manifest.json" or "__pycache__" in rel or rel.endswith(".pyc"):
        return False
    if rel in preexisting:
        return False  # merge mode: the team's file - never generator-owned
    return not any(rel == e.rstrip("/") or rel.startswith(e) for e in STATE_EXCLUDES)

files = {}
for rel in ROOT_FILES:
    p = target / rel
    if p.is_file() and owned(rel):
        files[rel] = hashlib.sha256(p.read_bytes()).hexdigest()
for root in INCLUDE_ROOTS:
    base = target / root
    if not base.is_dir():
        continue
    for p in sorted(base.rglob("*")):
        if not p.is_file() or p.is_symlink():
            continue
        rel = p.relative_to(target).as_posix()
        if owned(rel):
            files[rel] = hashlib.sha256(p.read_bytes()).hexdigest()

manifest = {
    "manifest_version": 1,
    "generator": "Infrastructure-Creator",
    "generator_version": version,
    "task": task,
    "profile": f"tasks/{task}/infra-scan-project-profile.md",
    "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "editions": editions,
    "mode": mode,
    "files": files,
}
(target / ".infra-manifest.json").write_text(
    json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
print(f"wrote .infra-manifest.json ({len(files)} files, mode={mode})")
PY
```

The recipe is self-limiting: it only walks edition roots that actually exist (the collision-guard/edition rules already ensure only selected roots exist) plus the two shared memory roots, and it skips runtime state and caches. In merge mode it is additionally bounded by `preexisting-files.txt`: only files this run actually created become generator-owned; everything the team already had stays untracked and therefore permanently invisible to `infra-update`. Do not extend or trim the include/exclude lists per target - `validate_generated.py` enforces exactly this contract (full coverage for `mode: full`; coverage deliberately not enforced for `mode: merge`, where untracked files under the roots are expected team property).

## Output Template

```markdown
# Infra Generate Complete: [target_name]

**Target:** [target path]
**Edition(s) generated:** [only the selected ones]
**Skills generated:** [count] ([list, including operational memory-bank and any domain skills])

## Verification
[bootstrap-verifier pass/fail summary]

## Upgrade Contract
`.infra-manifest.json` written: [file count] files, mode [full / merge], generator version [from VERSION], profile [TASK id]. `AGENTS.md` [stamped / pre-existing, untouched]. Future generator releases can be applied with `infra-update`.

## What To Do Next
The target now has its own working `AGENTS.md` + [selected edition folder(s)] + `memory-bank/`. Open [target path] and start with [suggested first generated command].

## Risks / Follow-Ups
[Anything bootstrap-verifier flagged, unresolved behavioral contradictions/unknowns, memory drift]
```

## Guardrails

- MUST NOT write into the target without the collision guard passing (explicit overwrite/merge/abort).
- MUST generate ONLY the selected edition(s) - never an unselected edition, never skip a selected one.
- MUST NOT invent content beyond what the profile supports - a forge needing missing data is a signal to re-scan/re-interview, not to guess.
- MUST stamp `AGENTS.md` and write `.infra-manifest.json` (per the recipe above) before running `bootstrap-verifier`, taking the version only from this generator's root `VERSION` file, and MUST refresh the manifest hashes after any content auto-fix.
- MUST NOT list runtime state (memory chunks, indexes, counters, `local/` dirs, Project Brain records) in the manifest - `infra-update` treats everything the manifest lists as generator-owned.
- MUST, in merge mode, snapshot `preexisting-files.txt` before any forge writes, record `"mode": "merge"`, and track ONLY files this run created - listing a pre-existing team file in the manifest authorizes `infra-update` to overwrite it later as a "safe update".
- MUST NOT report success while `bootstrap-verifier` has unresolved failures.

## Final Output

Return what was generated, where, for which edition(s), the verification results, remaining risks, and the concrete first command the target team should run inside their project.
