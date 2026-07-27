# Local Context Engine Design

## Goal

Extend each ready-to-use accelerator with repository-local context retrieval and
episodic task memory without introducing a service, network dependency, or new
package manager.

## Scope

The first version applies to `Laravel/`, `PHP Core/`, and `Symfony/`. Each
accelerator remains self-contained when opened directly or copied into another
repository.

The implementation adds:

- a dependency-free Python CLI backed by SQLite FTS5;
- indexing of policy, specifications, active memory chunks, task documents,
  capability epics, and changelog history;
- local, structured summaries of completed tasks;
- search across durable documents and local task episodes;
- secret-pattern rejection before an episode is stored;
- documentation in the existing memory-bank workflow.

It does not add embeddings, a vector database, an MCP transport, raw transcript
capture, or a central service.

The CLI supports Python 3.9+ and requires a Python SQLite build with FTS5.

## Authority And Storage

Git-tracked policy, code, tests, living specs, and active memory chunks remain
authoritative. The context database is a disposable local index at
`memory-bank/local/context.db`; the existing `.gitignore` rule keeps it out of
Git.

Task episodes are non-authoritative local summaries. They contain only a
summary, outcome, changed file paths, verification descriptions, and source
references. Raw prompts, raw model responses, credentials, customer data, and
logs are not accepted.

## CLI Contract

Every accelerator provides:

```text
python3 memory-bank/scripts/context.py index
python3 memory-bank/scripts/context.py search QUERY [--limit N] [--json]
python3 memory-bank/scripts/context.py record \
  --summary TEXT --outcome TEXT \
  [--file PATH] [--verification TEXT] [--source PATH]
python3 memory-bank/scripts/context.py status [--json]
```

`--root` and `--db` overrides support testing and controlled integrations.

## Retrieval

Indexing reads only a bounded set of Markdown sources:

1. `AGENTS.md` and `CLAUDE.md` as policy;
2. root `README.md` as project overview;
3. `specs/**/*.md` and `docs/**/*.md` as living specifications;
4. active `memory-bank/chunks/*.md` as verified memory;
5. `tasks/**/*.md` as task context;
6. `Task/Epics/**/*.md` as capability and business-rule context;
7. `CHANGELOG.md` as completed change history.

Re-indexing transactionally rebuilds the bounded document table and preserves
the separate episode table. Non-active or canonically invalid memory chunks are
excluded. Search uses FTS5 token matching and returns durable documents before
local episodes, preserving the authority hierarchy.

## Error Handling And Security

- Missing FTS5 support produces an actionable error.
- Empty search and episode fields are rejected.
- Existing memory-bank secret patterns are reused; detected values are never
  echoed.
- Invalid memory frontmatter is not promoted into the index.
- Database writes use SQLite transactions.

## Verification

Each accelerator ships integration tests that run the real CLI against a
temporary repository. Tests cover indexing/search, active-memory filtering,
episode recording/retrieval, stale document removal, and secret rejection.

The three script and test copies must remain byte-identical. Existing
memory-bank validator tests must continue to pass.

## Deferred Work

- Add local embeddings only if a golden-query evaluation shows FTS5 recall is
  insufficient.
- Add an MCP adapter only when clients need protocol-level discovery instead
  of invoking the repository CLI through the existing skill.
- Add generator support after the ready-to-use accelerators prove the contract.
