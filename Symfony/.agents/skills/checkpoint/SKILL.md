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
   `governed`, Project Brain is authoritative and the hooks already keep the
   branch task; follow the Governed Workflow below, then stop.
3. Continue with the Lightweight Workflow only when lightweight mode was
   explicitly configured. Every context command there must include global
   `--mode lightweight`.

## Governed Workflow

1. The task ID is `CONTEXT_TASK_ID`, otherwise
   `git symbolic-ref --quiet --short HEAD`. Stop with an actionable
   detached-HEAD error rather than inventing an ID.
2. Run `python3 memory-bank/scripts/context.py turn --task-id <task> --flush --json`.
   It flushes the turns the Stop hook buffered into the governed task and
   creates the task when this is its first checkpoint.
3. Write one concise sanitized line of what the checkpoint cannot see - the
   goal, a decision, the next step - with
   `python3 memory-bank/scripts/context.py update --task-id <task> --revision auto --progress "<summary>"`
   and, when there is one, `--next-step "<step>"`. Never store raw diffs,
   file bodies, prompts, responses, logs, or secrets; the runtime refuses
   likely secrets and personal data.
4. Report the task ID, whether it was created, the flushed turn and file
   counts, and the new revision.

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
- MUST NOT create a second task authority in governed mode: the branch task the hooks keep is the one task.
