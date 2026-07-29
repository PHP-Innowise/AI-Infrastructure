---
name: memory
description: Use when the user invokes memory or asks to refresh all repository-local context layers without lifecycle arguments.
phase: utility
flow-next: null
flow-alternatives: [checkpoint, memory-bank, verify]
---

# Unified Repository Memory

`memory` is an AI skill/command name, not a shell executable; when invoked,
begin directly at Workflow step 1. Never run or search for a `memory` binary.

Refresh every applicable repository-local Context Engine layer and stop. This
skill accepts no arguments.

## Workflow

1. Resolve the repository root with `git rev-parse --show-toplevel`. Stop
   without touching Context Engine state when this fails.
2. From the repository root, read
   `git status --porcelain=v1 -z --untracked-files=all`.
3. Read `.agents/skills/checkpoint/SKILL.md` as the canonical referenced procedure
   for the Working phase. Execute its Workflow steps inside this
   selected skill; this command does not invoke or chain another skill.
4. Set the Working result:
   - for a clean tree, set `working: skipped` and continue;
   - otherwise resolve the task ID with
     `git symbolic-ref --quiet --short HEAD`;
   - for detached HEAD or an invalid Context Engine task ID, set
     `working: skipped`, retain an actionable warning, and continue;
   - for a valid branch, execute the referenced checkpoint procedure through
     its report step, then set `working: updated`;
   - if the Working Memory failure occurs, set `working: failed`, retain the
     safe error, and continue.
5. Regardless of the Working result, run
   `python3 memory-bank/scripts/context.py index --json` from the repository
   root.
6. If indexing succeeds, set `procedural: updated`, `semantic: updated`, and
   `episodic: updated`, using the returned `layers` counts. If indexing fails,
   set all three source-driven layers to `failed`; do not delete or roll back a
   successful Working checkpoint.
7. Report:
   `working: updated | skipped | failed`,
   `procedural: updated | failed`,
   `semantic: updated | failed`, and
   `episodic: updated | failed`.
   Include the branch task ID and changed-file count when Working was updated,
   the JSON document counts when indexing succeeded, and every safe warning or
   error. Never report the full command as successful when indexing failed.
   Then stop.

## Safety

- All checkpoint safety rules remain mandatory for the Working phase.
- MUST NOT run `complete`, `record`, or `clear`, remove Working Memory, or
  create a completed-task episode.
- MUST NOT create or edit Procedural rules, Semantic memory chunks, changelog
  entries, or any other Git-tracked repository file.
- MUST NOT stage, commit, discard, or modify current Git changes.
- MUST preserve existing Working tasks and local completed episodes.
- MUST update only the ignored `memory-bank/local/context.db`.
- MUST keep Git-ignored and sensitive file contents excluded exactly as the
  referenced checkpoint procedure requires.
