---
name: checkpoint
description: Capture all current Git-visible changes as sanitized repository-local Working Memory. Use when the user invokes checkpoint or asks to save current AI work without supplying lifecycle arguments.
phase: utility
flow-next: null
flow-alternatives: [memory-bank, verify]
---

# Working-Memory Checkpoint

Capture one progress snapshot and stop. This skill accepts no arguments.

## Workflow

1. Resolve the repository root with `git rev-parse --show-toplevel`. Stop without
   touching Context Engine state when this fails.
2. Resolve the task ID with `git symbolic-ref --quiet --short HEAD`. Stop with
   an actionable `detached HEAD` error when it returns no branch. Do not invent
   another ID.
3. From the repository root, read
   `git status --porcelain=v1 -z --untracked-files=all`. Parse the NUL-delimited
   output and collect every current staged, unstaged, and untracked non-ignored
   path, including renamed, copied, and deleted paths.
4. If there are no current Git changes, report a no-op and stop without
   creating or updating Working Memory.
5. Before obtaining any diff, filter staged, unstaged, and untracked paths to safe non-sensitive text files. Record excluded paths from porcelain metadata only. Never read excluded path contents.
6. Inspect the staged and unstaged diffs plus safe untracked text files only
   after filtering, as needed to understand the change. Pre-existing or
   unrelated dirty files are part of the snapshot.
7. Write a concise semantic progress summary. Sanitize it before storage and
   never copy raw diff text, command output, prompts, responses, or secrets.
8. Run `python3 memory-bank/scripts/context.py get --task-id <branch> --json`.
   If the task does not exist, run
   `python3 memory-bank/scripts/context.py start --task-id <branch> --goal
   "Checkpoint work on branch <branch>"` with one `--file` value per normalized
   changed path.
9. Run `python3 memory-bank/scripts/context.py update --task-id <branch>
   --progress <summary>` and include one `--file` value per normalized changed
   path. Pass generated values as argv data; never interpolate Git content into
   executable shell syntax.
10. Report the task ID, current changed-file count, whether the task was created
   or updated, and the saved summary. Then stop.

## Safety

- MUST NOT read binary file contents.
- MUST NOT read `.env` files, private keys, credential files, or other paths
  prohibited by repository policy.
- MUST NOT store raw diffs or file contents.
- MUST NOT run `complete`, create an episode, stage, commit, discard, or modify
  repository files.
- MUST keep Git-ignored files excluded.
- MUST allow existing Context Engine task-ID and secret validation to fail
  safely without replacing valid Working Memory.
