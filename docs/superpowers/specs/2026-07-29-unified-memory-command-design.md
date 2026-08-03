# Unified Memory Command Design

## Goal

Add one argument-free AI command:

```text
memory
```

The command updates every applicable repository-local Context Engine layer
without asking the user for task IDs, summaries, outcomes, verification text,
or layer names.

It combines the existing safe Working Memory checkpoint with the existing
source-driven document index. It does not introduce another storage path,
model integration, service, dependency, or Git hook.

## User Experience

The user invokes `memory` without arguments:

- Claude and Cursor expose `/memory`;
- Codex exposes the repository skill `memory`.

The existing `checkpoint` command remains available and Working-only.

A successful run reports each layer independently:

```text
working: updated | skipped | failed
procedural: updated | failed
semantic: updated | failed
episodic: updated | failed
```

The report also includes the branch-derived task ID when Working Memory was
updated, the current changed-file count, indexed document counts by layer, and
any warning that caused one layer to be skipped.

## Selected Approach

Three approaches were considered:

1. A compositional AI skill that reuses the existing checkpoint procedure and
   existing `context.py index` operation.
2. A second skill containing a copied checkpoint implementation.
3. A new Python command, background process, service, or hook.

The compositional skill is selected. It keeps one authoritative safe Git
inspection procedure and reuses the current transactional index. Copying the
checkpoint instructions would create two security-sensitive workflows that
could drift. A new runtime component is unnecessary because the repository
already provides both required operations.

## Layer Behaviour

### Working

When current Git-visible changes exist on a valid branch, `memory` executes the
existing checkpoint procedure:

- derive the task ID from the current branch;
- parse NUL-delimited Git porcelain status;
- retain staged, unstaged, copied, renamed, deleted, and untracked non-ignored
  paths;
- filter sensitive or binary content before inspection;
- inspect only approved paths with literal, scoped, external-diff-disabled,
  text-conversion-disabled, rename-disabled Git commands;
- generate a sanitized semantic progress summary;
- create or update the corresponding Working Memory task through the existing
  `context.py get/start/update` lifecycle.

The procedure is referenced from the existing checkpoint skill rather than
copied into the new skill. `memory` applies one override: a no-change result
skips Working Memory and continues to indexing instead of stopping the entire
command.

When the repository is on detached HEAD or the branch is not a valid Context
Engine task ID, Working Memory is skipped with a warning and indexing
continues.

### Procedural

The command runs the existing source index, which rebuilds Procedural context
from current repository policy and skills:

- `AGENTS.md`;
- `CLAUDE.md`;
- skill Markdown under `.agents/`, `.claude/`, `.cursor/`, and `.codex/`.

The command does not create, edit, or propose procedural rules. Git-tracked
policy remains authoritative.

### Semantic

The same source index rebuilds Semantic context from the existing repository
sources already supported by Context Engine:

- the root `README.md`;
- `specs/**/*.md`;
- `docs/**/*.md`;
- active `memory-bank/chunks/*.md`;
- `tasks/**/*.md`;
- `Task/Epics/**/*.md`.

The command does not generate committed memory chunks or store a second AI
summary in the Semantic layer. Existing repository documents remain the source
of truth.

### Episodic

The source index refreshes rebuildable Episodic documents from `CHANGELOG.md`.
Existing local completed-task episodes remain in SQLite unchanged.

The command never calls `complete` or `record`, never deletes Working Memory,
and never creates a local completed-task episode. Task completion remains an
explicit separate action.

## Data Flow

```text
user invokes memory
        |
        v
validate repository root and read Git status
        |
        +--> changes + valid branch
        |        |
        |        v
        |    existing safe checkpoint procedure
        |        |
        |        v
        |    Working Memory created or updated
        |
        +--> clean tree / detached HEAD / invalid branch
                 |
                 v
             Working skipped
        |
        v
context.py index --json
        |
        +--> Procedural documents rebuilt
        +--> Semantic documents rebuilt
        +--> CHANGELOG Episodic documents rebuilt
        +--> local episodes and Working tasks preserved
        |
        v
per-layer result returned
```

## Composition Boundary

The `memory` skill reads `.agents/skills/checkpoint/SKILL.md` as the canonical
referenced procedure and executes its Workflow steps inside the selected
`memory` command. All three tool editions include that path. `memory` does not
invoke or chain a second agent skill. This preserves the repository policy that
an agent executes only the selected skill while keeping checkpoint security
rules in one place.

The new skill must state explicitly that `memory` is an AI command/skill name,
not a shell executable. When invoked, the agent begins directly at the
workflow. It must never search for or run a `memory` binary.

After the Working phase, the agent runs:

```text
python3 memory-bank/scripts/context.py index --json
```

No new `context.py` subcommand or SQLite table is required.

## Error And Partial-Success Behaviour

The two phases use existing independent transactions and are not made
artificially atomic.

- Outside a Git repository, `memory` stops before either phase and stores
  nothing.
- A clean tree skips Working and still runs indexing.
- Detached HEAD or an invalid branch skips Working, reports the reason, and
  still runs indexing.
- A Working checkpoint failure is reported, but indexing still runs.
- An indexing failure is reported without deleting or rolling back a successful
  Working checkpoint.
- A failed index transaction preserves the previous valid document index,
  existing Working tasks, and existing local episodes.
- The final result never describes the full command as successful when any
  required indexing layer failed.

This partial-success model is deliberate: source indexing is still useful when
Working identity is unavailable, and a safe Working checkpoint remains useful
when an unrelated source document prevents indexing.

## Security And Authority

- Repository policy, code, configuration, tests, specifications, and committed
  memory remain authoritative.
- The existing checkpoint safe-path filter and literal scoped diff commands are
  mandatory for the Working phase.
- Sensitive excluded paths are retained as metadata only; their contents are
  never read.
- Raw diffs, file contents, prompts, responses, command output, logs,
  credentials, customer data, and personal data are not stored.
- Existing Context Engine secret detection remains the final storage boundary.
- The command does not modify, stage, commit, or discard Git working files. It
  updates only the existing ignored `memory-bank/local/context.db`.
- Git-ignored files remain excluded.
- Existing local completed episodes are preserved.

## Tool Integration

Each Laravel, PHP Core, and Symfony accelerator receives:

- `.claude/commands/memory.md`;
- `.cursor/commands/memory.md`;
- `memory` skills in `.claude/skills/`, `.cursor/skills/`, and
  `.agents/skills/`;
- skill-flow discovery entries;
- focused integration contract tests.

The nine `memory` skills must be byte-identical. Command wrappers remain
tool-native and accept no arguments.

## Verification

Automated contract tests verify:

- command and skill discovery in all three accelerators;
- rejection of user arguments;
- explicit AI-workflow semantics rather than shell-binary execution;
- reuse of the existing checkpoint procedure without copying its security
  workflow;
- `complete` and `record` are never called;
- layer counts are reported from `index --json`;
- tool and accelerator mirrors remain synchronized;
- existing Context Engine and memory-bank tests continue to pass.

Existing Context Engine integration tests continue to prove transactional
indexing and preservation of Working tasks and local episodes.

Skill pressure testing uses a disposable Git repository containing safe
changes, a sensitive modified file, and a pathspec-magic filename. A fresh
Codex agent must execute `memory`, avoid reading the sensitive content, update
Working Memory, rebuild the index, and report each layer. A no-guidance control
must demonstrate the failure the skill prevents. Separate pressure scenarios
cover dirty, clean, detached HEAD, Working-failure, and index-failure flows.

A real-project black-box uses a disposable clone of
`/home/aliaksei/Desktop/bauherrenmappe` and a temporary Context Engine
database. It verifies dirty and clean flows without changing the original
Bauherrenmappe status, HEAD, index tree, or permanent local database.

Claude and Cursor behaviour is verified through byte-identical skill contracts.
Claude runtime may be exercised when practical. Cursor Agent runtime is not
required while it is unavailable on the host and must not be installed only
for this verification.

## Deferred Work

- Automatic task completion and episode creation remain explicit separate
  operations.
- Automatic authoring of semantic chunks or procedural policy remains outside
  this command.
- A native `context.py memory` subcommand is deferred because it would either
  duplicate orchestration or require a model integration.
- Hooks and background synchronization remain deferred until unattended
  capture is explicitly required.
