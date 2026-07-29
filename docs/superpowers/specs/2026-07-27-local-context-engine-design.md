# Local Context Engine Design

## Goal

Extend each ready-to-use accelerator with repository-local working, episodic,
semantic, and procedural memory without introducing a service, network
dependency, or new package manager.

## Scope

The first version applies to `Laravel/`, `PHP Core/`, and `Symfony/`. Each
accelerator remains self-contained when opened directly or copied into another
repository.

The implementation adds:

- a dependency-free Python CLI backed by SQLite FTS5;
- explicit temporary working memory keyed by a caller-supplied `task-id`;
- automatic source-based classification of repository documents;
- local, structured summaries of completed tasks;
- layered search and bounded context-packet assembly;
- atomic conversion of working memory into a completed-task episode;
- secret-pattern rejection before local working or episodic data is stored;
- documentation in the existing memory-bank workflow.

It does not add embeddings, a vector database, an MCP transport, raw transcript
capture, or a central service.

The CLI supports Python 3.9+ and requires a Python SQLite build with FTS5.

## Authority And Storage

Git-tracked policy, code, tests, living specs, and active memory chunks remain
authoritative. The context database lives at `memory-bank/local/context.db`;
the existing `.gitignore` rule keeps it out of Git. Its document index is
disposable and rebuildable, while working tasks and local episodes remain only
in that database until explicitly completed or cleared.

Working memory and task episodes are non-authoritative local records. They
contain only task goals, progress, next steps, outcomes, changed file paths,
verification descriptions, and source references. Raw prompts, raw model
responses, credentials, customer data, and logs are not accepted.

## Memory Layers

All four layers live in the same ignored SQLite database but have different
sources and lifecycles:

| Layer | Source | Lifecycle |
| --- | --- | --- |
| Working | Explicit `start` and `update` calls for a `task-id` | Temporary; removed only by successful `complete` or explicit `clear` |
| Episodic | Completed local task episodes and `CHANGELOG.md` | Append-only local episodes plus rebuildable changelog index |
| Semantic | Root README, specs, docs, active memory chunks, task documents, capability epics | Rebuildable from repository sources |
| Procedural | `AGENTS.md`, `CLAUDE.md`, and installed skill documentation | Rebuildable from repository sources |

The `task-id` is supplied by the caller. Prefer an issue identifier such as
`BAUMAS-133`, then a dedicated branch name, then a short descriptive slug. It
must match `[A-Za-z0-9][A-Za-z0-9._/-]{0,127}`. The CLI does not generate IDs
or depend on one task-management system.

Mirrored skill files are deduplicated by their logical skill path so equivalent
Claude, Cursor, and Codex copies do not consume the entire procedural result
budget.

## CLI Contract

Every accelerator provides:

```text
python3 memory-bank/scripts/context.py index
python3 memory-bank/scripts/context.py search QUERY [--limit N] [--json]
python3 memory-bank/scripts/context.py start \
  --task-id ID --goal TEXT [--file PATH] [--source PATH] [--json]
python3 memory-bank/scripts/context.py update \
  --task-id ID [--progress TEXT] [--next-step TEXT] \
  [--file PATH] [--source PATH] [--json]
python3 memory-bank/scripts/context.py context \
  QUERY --task-id ID [--limit N] [--json]
python3 memory-bank/scripts/context.py complete \
  --task-id ID --outcome TEXT [--summary TEXT] \
  [--file PATH] [--verification TEXT] [--source PATH] [--json]
python3 memory-bank/scripts/context.py clear --task-id ID [--json]
python3 memory-bank/scripts/context.py record \
  --summary TEXT --outcome TEXT \
  [--file PATH] [--verification TEXT] [--source PATH] [--json]
python3 memory-bank/scripts/context.py status [--json]
```

`--root` and `--db` overrides support testing and controlled integrations.

`start` rejects an existing ID. `update` changes supplied scalar fields and
appends unique list values while leaving omitted fields untouched. `clear`
removes unfinished working memory without creating an episode.

`complete` defaults the episode summary to the working goal, combines its files
and sources with values supplied at completion, writes the episode, and deletes
the working row in one `BEGIN IMMEDIATE` transaction. An insertion or validation
failure rolls the entire operation back.

## Retrieval

Indexing reads only a bounded set of repository sources:

1. procedural: `AGENTS.md`, `CLAUDE.md`, and skill Markdown under `.agents/`,
   `.claude/`, `.cursor/`, and `.codex/`;
2. semantic: root `README.md`, `specs/**/*.md`, `docs/**/*.md`, active
   `memory-bank/chunks/*.md`, `tasks/**/*.md`, and `Task/Epics/**/*.md`;
3. episodic: `CHANGELOG.md`.

Re-indexing transactionally rebuilds the bounded document table and preserves
the working and episode tables. Non-active or canonically invalid memory chunks
are excluded.

`search` remains a general FTS5 query and includes the layer on each result.
`context QUERY --task-id ID` requires an existing working task and returns:

1. the complete working record for that `task-id`;
2. up to `--limit` procedural results;
3. up to `--limit` semantic results;
4. up to `--limit` episodic results, combining changelog matches and local
   episodes.

The default per-layer limit is three. Results keep their source paths and must
be verified against the repository before use.

`status` reports indexed counts by layer, active working-task count, local
episode count, and the database path.

## Error Handling And Security

- Missing FTS5 support produces an actionable error.
- Empty search, task, and episode fields are rejected.
- Invalid, duplicate, or unknown task IDs produce safe errors.
- Existing memory-bank secret patterns are reused; detected values are never
  echoed.
- Invalid memory frontmatter is not promoted into the index.
- All writes use SQLite transactions; working-task completion uses one explicit
  immediate transaction for insertion and deletion.

## Verification

Each accelerator ships integration tests that run the real CLI against a
temporary repository. Tests cover:

- automatic classification and skill-mirror deduplication;
- isolation of multiple explicit task IDs;
- working-memory start, update, clear, and secret rejection;
- layered context-packet assembly;
- atomic complete success and failure rollback;
- episode recording and retrieval;
- active-memory filtering and stale document removal;
- compatibility migration from the existing local database.

The three script and test copies must remain byte-identical. Existing
memory-bank validator tests must continue to pass. The completed implementation
must also pass the existing Bauherrenmappe black-box scenario without changing
that repository.

## Root README Documentation

The root `README.md` must introduce the Context Engine and its four logical
memory layers in practical language for developers evaluating the repository.
The section will explain:

- the problem it solves and why it is more than a folder of notes;
- what working, episodic, semantic, and procedural mean in this implementation;
- which repository sources populate each layer;
- the `start`, `update`, `context`, `complete`, `index`, `search`, `record`, and
  `status` workflow;
- what a completed-task episode contains;
- the authority boundary between the local index and repository sources;
- local SQLite storage, Git exclusion, and secret rejection;
- the successful Bauherrenmappe real-project verification;
- the deliberate deferral of embeddings, MCP, LangGraph, and a central service.

The section must not claim automatic per-request context injection, hybrid or
vector search, separate services, or integrations that the implementation does
not provide.

## Deferred Work

- Add local embeddings only if a golden-query evaluation shows FTS5 recall is
  insufficient.
- Add an MCP adapter only when clients need protocol-level discovery instead
  of invoking the repository CLI through the existing skill.
- Add generator support after the ready-to-use accelerators prove the contract.
