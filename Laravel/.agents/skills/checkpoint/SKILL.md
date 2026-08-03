---
name: checkpoint
description: Use when the user invokes checkpoint or asks to save current AI work without lifecycle arguments.
phase: utility
flow-next: null
flow-alternatives: [memory, project-brain, memory-bank, verify]
---

# Working-Memory Checkpoint

`checkpoint` is an AI skill/command name, not a shell executable. It accepts no
arguments and never completes work.

## Authority Gate

1. Resolve the repository root with `git rev-parse --show-toplevel`; stop safely
   if it fails.
2. Read `project-brain/config/runtime.json` when present. If its mode is
   `governed`, Project Brain is authoritative: do not derive a task from the
   branch and do not mutate SQLite working state. Report `working: skipped`
   with an actionable instruction to use `project-brain update` with the
   caller-supplied task ID and expected revision, then stop.
3. Continue only when lightweight mode was explicitly configured. Every
   context command below must include global `--mode lightweight`.

## Lightweight Workflow

1. Resolve the task ID with `git symbolic-ref --quiet --short HEAD`. Stop with
   an actionable detached-HEAD error rather than inventing an ID.
2. Read `git status --porcelain=v1 -z --untracked-files=all` from the repository
   root and parse every staged, unstaged, untracked, renamed, copied, and deleted
   path. If there are no changes, report a no-op.
3. Before obtaining any diff, filter paths to safe, non-sensitive, non-ignored
   text files. Record excluded paths from porcelain metadata only; never read
   excluded contents.
4. Inspect only approved paths with separate argv entries after `--`:
   `git --literal-pathspecs diff --no-ext-diff --no-textconv --no-renames -- <safe-paths>`
   and
   `git --literal-pathspecs diff --cached --no-ext-diff --no-textconv --no-renames -- <safe-paths>`.
   Read safe untracked files only after filtering.
5. Create a concise sanitized progress summary. Never store raw diffs, file
   bodies, prompts, responses, logs, or secrets.
6. Run `python3 memory-bank/scripts/context.py --mode lightweight get --task-id
   <branch> --json`. If absent, run `start` with goal `Checkpoint work on branch
   <branch>` and one `--file` argv value per exact changed path. Then run
   `update` with the summary and the same exact paths, preserving leading and
   trailing whitespace.
7. Report task ID, changed-file count, exclusions, and whether the local task
   was created or updated.

## Safety

- MUST NOT read binary, `.env`, key, credential, ignored, or policy-prohibited
  files.
- MUST NOT run unscoped `git diff`, `--stat`, or `--numstat`.
- MUST NOT run `complete`, `record`, or `clear`, create an episode, stage,
  commit, discard, or modify repository files.
- MUST NOT create a second task authority in governed mode.
