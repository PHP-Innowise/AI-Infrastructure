# Project Brain Directory Guide

Project Brain is the Git-tracked shared authority for active work. It stores
governed tasks, handoffs, operational records, retrieval manifests, and
promotion proposals. Durable reusable knowledge belongs in `memory-bank/`;
the disposable SQLite search database belongs in
`memory-bank/local/context.db`.

## Root Files

| Path | Purpose |
| --- | --- |
| `PROTOCOL.md` | Defines authority, privacy, lifecycle, revision, locking, retrieval, compaction, and promotion rules. |
| `.gitignore` | Excludes machine-local runtime files while preserving required directory placeholders. |
| `README.md` | Explains the Project Brain directory structure. |

## `dynamic/`

Contains active shared records. Each record uses a UUID filename, strict JSON
frontmatter, source evidence, revision history, and a type-specific lifecycle.

| Directory | Stored records |
| --- | --- |
| `dynamic/tasks/` | Active work, progress, next actions, affected files, and verification state. |
| `dynamic/findings/` | Observations that need investigation, confirmation, or resolution. |
| `dynamic/bugs/` | Reproducible defects, triage, fixing, and verification state. |
| `dynamic/incidents/` | Operational impact, containment, recovery, and closure state. |
| `dynamic/decisions/` | Proposed, accepted, rejected, or superseded project decisions. |
| `dynamic/events/` | Immutable notable project events that may later be superseded. |

Example:

```text
dynamic/tasks/550e8400-e29b-41d4-a716-446655440000.md
```

## `control/`

Contains records that coordinate Project Brain operations.

| Directory | Purpose |
| --- | --- |
| `control/handoffs/` | Compact continuation summaries for active tasks and future sessions. |
| `control/retrieval-manifests/` | Metadata describing a governed query, selected/excluded sources, filters, and token estimates. |
| `control/promotions/` | Proposals and independent human reviews for moving reusable knowledge into Memory Bank. |

Handoffs and manifests do not contain raw prompts, responses, hidden
reasoning, or complete source bodies.

Delivered Task Capsules are deterministic discovery aids: after ranking and
policy filters they contain at most 2 procedural, 3 semantic, and 1 episodic
item and no more than 8,000 serialized characters. Direct CLI queries are
privacy-checked before any index or manifest is opened.

## `archive/`

Contains terminal or superseded records moved by compaction. Records are moved,
not deleted, so history remains traceable. Archived records must satisfy the
same validation rules as active records.

Runtime-created subdirectories may include record types and archived handoffs:

```text
archive/task/
archive/finding/
archive/bug/
archive/incident/
archive/decision/
archive/event/
archive/handoffs/
```

## `indexes/`

Contains deterministic, Git-trackable summaries of active and archived Brain
records.

| File | Purpose |
| --- | --- |
| `indexes/active.json` | Lists active record IDs, types, statuses, revisions, and paths. |
| `indexes/archive.json` | Lists archived record IDs, types, statuses, revisions, and paths. |

Both files start as `[]` and are rebuilt after supported mutations or
compaction. They are navigation indexes, not independent authorities.

## `config/`

| File | Purpose |
| --- | --- |
| `config/runtime.json` | Selects governed/lightweight mode, framework label, canonical skill edition, privacy/authority filters, owners, and retrieval provider. |
| `config/providers.json` | Documents the native SQLite provider and optional disabled provider contracts. |
| `config/telemetry.json` | Configures disabled-by-default, metadata-only observability and prohibited fields. |

Task phases persist only as `understanding`, `planning`, `implementation`,
`verification`, and `finalization`; legacy input aliases normalize before
write. Task completion is explicit and numeric-revision checked. Merge
detection can report a sanitized candidate but cannot close a task.

Promotion has two independent modes. The shipped automatic mode labels output
`automatic` and `approved-without-review` with no reviewer. Disabling
`automatic_promotion` enables propose → independent review → apply; neither
mode is represented as the other.

Framework-specific differences belong in `runtime.json`; common runtime and
Brain assets remain byte-identical across accelerators.

## `schemas/`

Strict JSON schemas for:

- dynamic records;
- handoffs;
- retrieval manifests;
- promotion records;
- token-usage events.

Schemas reject unknown fields and define the portable contract used by
validators and tests.

## `templates/`

Starting structures for tasks, findings, bugs, incidents, decisions, events,
handoffs, retrieval manifests, and promotions. Templates are examples of the
required shape; supported CLI commands should perform mutations whenever
available.

## `scripts/`

| File | Purpose |
| --- | --- |
| `scripts/validate.py` | Runs dependency-free validation for active records, archived records, control files, indexes, and configuration. |

Operational mutations are exposed through:

```text
memory-bank/scripts/context.py
```

## `tests/`

Contains Project Brain contract and integration tests covering lifecycles,
concurrency, privacy, source freshness, retrieval, conflicts, rollback,
compaction, promotion, and telemetry defaults.

## `local/`

Contains ignored machine-local Brain runtime state, currently including the
repository-wide mutation lock:

```text
local/runtime.lock
```

This directory is not shared authority. The disposable SQLite context database
is stored separately at:

```text
memory-bank/local/context.db
```

## Why `.gitkeep` Files Exist

Git does not track empty directories. `.gitkeep` files preserve the required
initial layout before the first task, handoff, proposal, or archive record is
created. They contain no project knowledge and can remain after real records
appear.
