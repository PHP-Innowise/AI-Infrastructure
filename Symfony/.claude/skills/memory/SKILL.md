---
name: memory
description: Use when the user invokes memory or asks to refresh repository-local context without lifecycle arguments.
phase: utility
flow-next: null
flow-alternatives: [checkpoint, project-brain, memory-bank, verify]
---

# Unified Repository Memory

`memory` is an AI skill/command name, not a shell executable. It accepts no
arguments, refreshes applicable repository-local context, reports each layer,
and stops. It never completes, promotes, or invents task state.

## Workflow

1. Resolve the repository root with `git rev-parse --show-toplevel`; stop without
   touching context state if it fails.
2. Read `project-brain/config/runtime.json` when present and determine whether
   the explicitly configured mode is `governed` or `lightweight`.
3. In governed mode:
   - treat Project Brain as the only authority for active tasks and handoffs;
   - run `python3 memory-bank/scripts/context.py validate`;
   - set `working: governed` and do not derive a branch task, checkpoint
     progress, or write SQLite working state.
4. In explicitly configured lightweight mode, read
   `.agents/skills/checkpoint/SKILL.md` as the canonical referenced procedure
   and execute its lightweight workflow inside this selected skill. This
   does not invoke or chain another skill. Set `working: updated`, `skipped`, or
   `failed`, retain safe warnings, and continue.
5. Regardless of the Working result, run
   `python3 memory-bank/scripts/context.py refresh --json` from the repository
   root. It refreshes only the disposable SQLite document index, and reports
   `procedural`, `semantic`, and `episodic` separately. This is the command the
   request hook runs, so this skill and the hook cannot drift apart.
6. Report the three layers exactly as `refresh` returned them, with the
   document counts it reported. Do not derive a layer status from the exit code
   and do not infer one from a successful run; a failed layer is named in the
   output. Do not roll back a successful lightweight checkpoint because a layer
   failed.
7. Run `python3 memory-bank/scripts/context.py status --json` and report the
   configured mode, index health/counts, Project Brain validation result, safe
   warnings, and:
   `working: governed | updated | skipped | failed`,
   `procedural: updated | failed`,
   `semantic: updated | failed`,
   `episodic: updated | failed`.

## Safety

- MUST NOT run `complete`, `record`, `clear`, promotion application, lifecycle
  transitions, or governed task mutations.
- MUST NOT pass `--query` to `refresh`. This skill reports layer health; it
  does not perform task-aware retrieval and writes no retrieval manifest.
- MUST NOT create or edit Project Brain records, Memory Bank chunks,
  changelogs, specs, or any other Git-tracked file.
- MUST NOT stage, commit, discard, or overwrite current changes.
- MUST preserve privacy, ownership, lifecycle, freshness, bounded retrieval,
  manifests, rollback, and telemetry policy.
- MUST update only ignored local context state; Project Brain remains
  authoritative in governed mode.
