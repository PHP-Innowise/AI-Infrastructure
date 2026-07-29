# Automatic Working-Memory Checkpoint Design

## Goal

Replace the manual `start` and `update` argument flow for routine progress
capture with one agent command:

```text
checkpoint
```

The current AI agent inspects the repository's current Git changes, writes a
short semantic summary, and stores that snapshot in the existing local Working
Memory. The user does not supply a task ID, summary, outcome, verification, or
completion flag.

## Scope

The feature applies to the Laravel, PHP Core, and Symfony accelerators and keeps
their integrations synchronized.

It adds:

- `/checkpoint` commands for Claude and Cursor;
- a repository `checkpoint` skill for Codex;
- automatic task identity from the current Git branch;
- automatic discovery of all current Git-visible changes;
- agent-written, sanitized progress summaries;
- automatic creation or update of the corresponding Working Memory task;
- focused contract checks and a real-project Bauherrenmappe verification.

It does not add a model call to `context.py`, a new service, a dependency, a Git
hook, automatic task completion, or automatic episodic-memory creation.

## Why The Agent Owns The Command

Three approaches were considered:

1. An AI command that inspects Git, summarizes the changes, and invokes the
   existing Context Engine.
2. A native Python checkpoint command that collects paths but cannot create a
   semantic summary without a model integration.
3. A Git hook that captures state during commit or another Git event.

The AI command is selected because the current agent already understands the
work and can summarize it without adding a model dependency to the repository.
A Git hook runs at the wrong lifecycle boundary, while a standalone local CLI
would either store only mechanical file metadata or introduce a second AI
runtime.

## User Experience

The user invokes `checkpoint` without arguments. Claude and Cursor expose it as
`/checkpoint`; Codex exposes the equivalent repository skill by the same name.

A successful result reports:

- the branch-derived task ID;
- the number of current changed files;
- whether Working Memory was created or updated;
- the sanitized summary that was saved.

The command is a progress checkpoint only. It never calls `complete`, creates
an episode, commits files, stages files, or changes the working tree.

## Task Identity

The current branch name returned by Git is the task ID. Repeated checkpoints on
the same branch update the same Working Memory task. Different branches remain
isolated.

The branch must satisfy the existing Context Engine task-ID validation. A
detached HEAD is rejected with an actionable error because inventing an ID
would make later retrieval ambiguous.

## Git Change Collection

The agent discovers all current Git-visible changes in the repository:

- staged tracked changes;
- unstaged tracked changes;
- added, modified, deleted, copied, and renamed paths;
- untracked, non-ignored files.

Git-ignored files are excluded naturally. The command reflects the complete
current dirty state rather than trying to infer which files were changed by a
specific model or session. Therefore, pre-existing or unrelated dirty files are
included deliberately.

Path discovery uses Git's NUL-delimited porcelain output so whitespace and
renames are handled safely. The agent may inspect staged and unstaged diffs and
safe untracked text files to understand the changes. It does not read binary
content, `.env` files, private keys, credential files, or other paths prohibited
by repository policy.

## Stored Working Memory

The command reuses the existing `start`, `get`, and `update` lifecycle rather
than introducing another storage path.

For a branch without a Working Memory task, the agent:

1. creates the task with the goal `Checkpoint work on branch <branch>`;
2. stores the semantic summary as current progress;
3. adds every discovered changed path.

For an existing task, the agent replaces the scalar progress summary and merges
new paths into the existing file list without duplicates. The existing
`updated_at` field records the checkpoint time.

Only normalized paths and a concise semantic summary are stored. Raw diffs,
file contents, prompts, responses, logs, credentials, personal data, and
customer data are not stored.

## Data Flow

```text
user invokes checkpoint
        |
        v
agent validates Git repository and current branch
        |
        v
agent collects all current Git-visible changed paths
        |
        v
agent safely inspects relevant text changes and writes a summary
        |
        v
agent calls existing Context Engine start/update operations
        |
        v
Working Memory is returned to the user
```

The SQLite database and four-layer model remain unchanged. Repository policy,
code, tests, configuration, and specifications remain authoritative over the
saved Working Memory.

## No-Change And Error Behaviour

- Outside a Git repository, the command stops before touching Context Engine
  state.
- On detached HEAD, the command reports that a branch is required.
- If the repository has no current changes, the command reports a no-op and
  does not create or update Working Memory.
- If the branch name is not a valid task ID, the existing validation error is
  surfaced without creating fallback identity.
- Binary or sensitive paths may be listed as changed but their contents are not
  read or summarized.
- Existing secret-pattern validation remains the final storage boundary. A
  rejected summary does not replace valid Working Memory.
- Git inspection and Context Engine errors do not modify, stage, commit, or
  discard repository files.

## Integration Shape

Each accelerator receives the smallest tool-native wrapper:

- Claude command instructions under `.claude/commands/`;
- Cursor command instructions under `.cursor/commands/`;
- a shared repository skill under `.agents/skills/` for Codex, mirrored into
  the existing Claude and Cursor skill trees only where their integration model
  requires it.

The wrapper instructs the active agent to perform Git inspection, semantic
summarization, and existing Context Engine calls. There is no duplicate
checkpoint implementation inside each tool integration.

The Laravel, PHP Core, and Symfony copies must keep equivalent behaviour while
retaining their framework-specific wording and paths.

## Verification

Automated checks cover the deterministic integration contract:

- all three accelerators expose equivalent checkpoint instructions;
- the instructions derive task identity from the current branch;
- they include staged, unstaged, renamed, deleted, and untracked Git-visible
  paths while excluding ignored content;
- they create a missing task and update an existing task through the current
  Context Engine lifecycle;
- they treat no changes as a no-op;
- they reject detached HEAD and preserve the secret-storage boundary;
- the existing Context Engine and memory-bank test suites continue to pass;
- mirrored Context Engine scripts and tests remain byte-identical.

A black-box verification in the real Bauherrenmappe repository exercises both
the first and repeated checkpoint flows against a temporary local database. It
must confirm the branch-derived task ID, stored summary, changed paths, and
update behaviour without modifying Bauherrenmappe's index, working tree,
commits, or permanent `memory-bank/local/context.db`.

## Deferred Work

- Automatic `complete` remains separate because completion requires an explicit
  verified outcome.
- Per-session attribution is deferred because Git cannot reliably identify
  which actor produced an existing dirty change.
- Diff or content persistence is excluded because Working Memory needs a
  summary and provenance paths, not a second copy of repository state.
- Hooks and background checkpoints are deferred until a real need for
  unattended capture appears.
