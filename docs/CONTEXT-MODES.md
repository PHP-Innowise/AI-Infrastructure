# Context Modes: Governed and Lightweight

The combined Project Brain, Local Context Engine, and Memory Bank supports two
operating modes:

- **governed** — the default mode for shared, revision-safe project work;
- **lightweight** — an explicit local-only fallback for short-lived work.

Both modes use the Local Context Engine for search. They differ mainly in who
owns the Working layer.

## Governed Mode

Governed mode is configured by default:

```json
{
  "mode": "governed"
}
```

### Storage ownership

```text
Project Brain
  → authoritative tasks, progress, handoffs, findings, bugs,
    incidents, decisions, events, manifests, and promotions

Memory Bank
  → reviewed reusable knowledge

memory-bank/local/context.db
  → disposable search index, local task binding/cache,
    and optional non-authoritative replay episodes
```

### Behavior

- A caller supplies a stable task ID.
- `start` creates a UUID-based Project Brain task and handoff.
- SQLite stores only the task ID, Brain UUID, and current revision binding.
- `update` uses owner authorization and optimistic revision checks.
- `retrieve` assembles privacy-filtered, freshness-checked, bounded context.
- `complete` advances the shared Brain lifecycle after verification.
- `checkpoint` does not derive an authoritative task from the Git branch.
- `memory` validates Project Brain and refreshes the disposable index without
  creating competing task state.

### Use governed mode when

- work must continue across agents or sessions;
- task state must be shared through Git;
- findings, incidents, decisions, or handoffs must be governed;
- concurrent updates must be detected;
- reusable knowledge may later be promoted to Memory Bank.

## Lightweight Mode

Lightweight mode must be selected explicitly:

```json
{
  "mode": "lightweight"
}
```

### Storage ownership

```text
memory-bank/local/context.db
  → local working tasks, progress, completed episodes,
    and the disposable search index

Project Brain
  → not used as the authority for the lightweight task

Memory Bank
  → unchanged; stores durable knowledge, with automatic unreviewed
    promotions labeled `auto-promoted`
```

### Behavior

- `start`, `update`, `get`, `clear`, and `complete` use local SQLite task state.
- `checkpoint` may derive a local task ID from the current Git branch.
- Checkpointing reads only approved changed paths after ignored, sensitive,
  binary, and prohibited files are filtered.
- `memory` may run the lightweight checkpoint workflow before refreshing the
  long-lived search layers.
- `complete` converts the local working task into a searchable local episode.
- Local episodes are never promoted automatically.

### Use lightweight mode when

- work is temporary and machine-local;
- no shared handoff or governed lifecycle is required;
- losing state when `context.db` is deleted is acceptable;
- branch-based checkpointing is useful;
- the repository should not receive Project Brain task records.

## What Remains the Same

Both modes keep the four logical context layers:

```text
Working
Procedural
Semantic
Episodic
```

Both modes also:

- use SQLite FTS5/BM25 for local lexical search;
- index eligible policies, skills, documentation, specifications, active
  Memory Bank chunks, and history;
- reject likely secrets and invalid task values;
- exclude Git-ignored sources before reading them;
- fail safely on invalid UTF-8 without replacing the previous index;
- treat retrieved content as a discovery aid rather than canonical truth;
- require explicit completion;
- keep session-start banners metadata-only;
- deliver bounded context automatically when supported: fresh prompt capsules
  on Claude Code/Codex and a one-turn-stale `alwaysApply` rule on Cursor.

The difference is the Working layer:

```text
Governed:
Working authority → Project Brain
SQLite → binding/cache only

Lightweight:
Working authority → local SQLite working_tasks
Project Brain → not used for that task
```

## Selecting a Mode

### Persistent project configuration

Edit:

```text
project-brain/config/runtime.json
```

Set:

```json
"mode": "governed"
```

or:

```json
"mode": "lightweight"
```

Use the persistent setting when the `memory` and `checkpoint` AI workflows
should consistently follow that mode.

### One CLI invocation

The global option must appear before the subcommand:

```bash
python3 memory-bank/scripts/context.py \
  --mode lightweight \
  start --task-id LOCAL-001 --goal "Investigate locally"
```

### Environment override

```bash
PROJECT_BRAIN_MODE=lightweight \
python3 memory-bank/scripts/context.py status
```

Mode precedence is:

```text
explicit --mode option
    ↓
PROJECT_BRAIN_MODE environment variable
    ↓
project-brain/config/runtime.json
```

## Switching Modes

Changing the mode does not migrate task state automatically.

- A governed Project Brain task remains in Project Brain after switching to
  lightweight mode.
- A lightweight SQLite task remains local after switching back to governed
  mode.
- Do not use the same task ID in both modes unless the states have been
  reviewed and intentionally reconciled.
- Complete, cancel, or preserve the current task before switching long-running
  work to another mode.

## Recommendation

Use **governed mode** for normal accelerator work. It provides shared task
authority, handoffs, revision safety, evidence, retrieval manifests, archival,
and reviewed promotion.

Use **lightweight mode** only when local convenience is more important than
shared continuity and governed history.
