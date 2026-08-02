# Context and Memory

The accelerator separates current truth, active work, local retrieval, and
durable learning so that continuity does not become a competing source of
truth. This guide explains those boundaries and the reasoning behind them.

For commands and recovery procedures, see [Context and Memory Operations](OPERATIONS.md).
For mode selection and switching, see [Context Modes](CONTEXT-MODES.md). For a
step-by-step task narrative, see the
[User Task Workflow Example](examples/USER-TASK-WORKFLOW-EXAMPLE.md).

## The Core Principle

Context helps an agent find and continue work; it does not redefine the
project.

The authority order is:

1. enforcement such as hooks, CI, linters, and static analysis;
2. mandatory policy such as `AGENTS.md`;
3. current architecture and runtime truth: code, configuration, migrations,
   tests, living specifications, and other canonical project sources;
4. verified active Memory Bank knowledge;
5. operational skills;
6. examples;
7. general documentation.

Project Brain and retrieval results are continuity and discovery mechanisms.
When either conflicts with a higher-authority source, the higher source wins.
The conflict should be preserved and resolved; it must not be silently blended
into a false consensus.

## The Four Systems

### Canonical project sources

Canonical sources define behavior and obligations. Depending on the question,
they include policy, specifications, code, configuration, migrations, tests,
and enforcement output.

Canonical does not mean one file always wins. A test can be stale relative to
an approved specification, or documentation can lag code. The repository's
policy hierarchy and the scope of the claim determine authority. The important
rule is that a retrieved summary never outranks the current source it
summarizes.

### Project Brain

`project-brain/` is the Git-tracked control plane for shared active work. It
owns:

- governed tasks and their lifecycle;
- concise continuation handoffs;
- findings, bugs, incidents, decisions, and events;
- owner authorization and optimistic revisions;
- evidence paths and source fingerprints;
- explicit conflicts and supersession relationships;
- retrieval manifests;
- durable-memory promotion proposals and reviews;
- deterministic navigation indexes and archived history.

Project Brain answers questions such as:

- What is being worked on?
- Who may update it?
- What revision did I read?
- What is complete, blocked, verifying, or next?
- What evidence and conflicts are attached?
- What should another session know to continue safely?

It is not a transcript store, raw logging system, issue tracker replacement,
specification replacement, or durable knowledge base.

### Local Context Engine

The Local Context Engine is the Python/SQLite retrieval runtime exposed through
`memory-bank/scripts/context.py`. Its default database is the ignored
`memory-bank/local/context.db`.

It provides:

- SQLite FTS5/BM25 lexical indexing and search;
- task-aware context assembly;
- eligibility metadata and freshness checks;
- bounded snippets and estimated-token budgets;
- machine-local task bindings in governed mode;
- optional local working tasks and replay episodes in lightweight mode.

The database is derived and disposable only in part. Its document index can be
rebuilt from repository sources. Its lightweight tasks, local episodes, and
governed compatibility bindings cannot be reconstructed by the current CLI
after deletion.

The active provider is local `sqlite-fts5`: network access and embeddings are
disabled. An external provider entry exists only as a disabled contract. The
runtime does not include automatic prompt injection.

### Memory Bank

`memory-bank/` is reviewed, Git-tracked, durable project memory shared across
supported agent editions. It stores cohesive, reusable, source-backed knowledge
that will help more than one future task, for example:

- stable project constraints and conventions;
- accepted architectural consequences;
- durable domain terminology and invariants;
- integration contracts and failure semantics;
- verified operational lessons;
- explicit non-sensitive facts the user asks the project to remember.

Memory Bank does not own active progress. It must not contain raw task history,
speculative reasoning, chat transcripts, command output, generic framework
advice, duplicated living specifications, or unresolved guesses.

## Four Logical Context Layers

The combined model has four logical layers:

```text
Working
Procedural
Semantic
Episodic
```

### Working

Working context is the current objective, progress, next actions, affected
files, sources, revision, and handoff.

- In governed mode, Project Brain is authoritative. SQLite keeps a local
  external-ID-to-Brain-UUID binding and revision cache, plus a compatibility
  pointer with no duplicated progress.
- In lightweight mode, SQLite `working_tasks` is authoritative for that local
  workflow. There is no shared handoff or Brain lifecycle.

Switching modes does not migrate Working state. Reusing the same external task
ID in both modes creates ambiguity and should be avoided unless the states are
explicitly reconciled.

### Procedural

Procedural context explains how work must be performed: policy and skills such
as `AGENTS.md`, `CLAUDE.md`, and edition skill files.

The index deduplicates mirrored skills by logical path, preferring the first
discovered copy, while the separate parity check detects drift from the
configured canonical edition. Retrieval is convenient, but the actual policy
or skill file remains authoritative.

### Semantic

Semantic context describes the project: repository documentation,
specifications, task documents, capability epics, active Memory Bank chunks,
and eligible Project Brain records and handoffs.

Not every semantic document has equal authority. A living specification or
current configuration can outrank a durable memory chunk; an observed Brain
finding can be useful while still requiring verification.

### Episodic

Episodic context records completed or historical experience. It includes an
eligible changelog and local completed-work episodes.

Episodes support recall of what happened; they do not prove that the same
behavior or decision is still valid. Local episodes are non-authoritative,
machine-local, and never automatically promoted.

## Governed and Lightweight Ownership

The default governed model is:

```text
Shared active work       -> Project Brain
Reviewed reusable memory -> Memory Bank
Search and local binding -> Local Context Engine
Current behavior         -> canonical project sources
```

Governed mode is appropriate when work crosses agents, sessions, or machines;
concurrent updates matter; handoffs are required; or records may later support
a reviewed promotion.

The explicit lightweight model is:

```text
Local active work        -> SQLite working_tasks
Local completed replay   -> SQLite episodes
Reviewed reusable memory -> Memory Bank
Current behavior         -> canonical project sources
```

Lightweight mode is appropriate only when local convenience is more important
than shared continuity and losing `context.db` state is acceptable. See
[Context Modes](CONTEXT-MODES.md) for selection precedence and switching
guidance rather than duplicating those operational details here.

## Choosing the Right Record

Use the smallest authoritative record that matches the lifetime and audience
of the information.

### Governed dynamic records

Use a **task** for an intended body of work with a goal, progress, next steps,
affected files, verification, and completion/cancellation lifecycle. A task
answers “what are we doing and where are we now?”

Use a **finding** for an observation that needs investigation, confirmation, or
resolution. A finding can begin as observed evidence without asserting that a
defect exists.

Use a **bug** for a reproducible defect requiring triage, fixing, and
verification. Prefer a bug over a finding when expected versus actual behavior
and remediation are established.

Use an **incident** for operational impact that needs containment, recovery,
resolution, and closure. Store only sanitized impact and lessons; never include
raw customer data, production identifiers, confidential logs, or unredacted
payloads.

Use a **decision** for a proposed project choice that needs acceptance or
rejection and may later be superseded. If the decision changes architecture,
API behavior, schema, security behavior, or another living contract, update the
canonical specification too; the Brain decision does not replace it.

Use an **event** for a notable immutable occurrence. Event content cannot be
updated; only lifecycle supersession is supported. Do not use events as an
append-only substitute for task progress or logs.

### Task documents

Use `tasks/TASK-NNN/` documents for richer, Git-tracked delivery artifacts such
as plans, analysis, review notes, or temporary implementation documentation
that do not fit the concise governed record. Follow the selected accelerator's
file naming and counter policy.

A task document may be a source for a Brain record. It should not duplicate the
same status in several files without a clear owner.

### Living specifications

Use `specs/` when the information defines current product or technical behavior
that implementers must satisfy: architecture, API contracts, data models,
security rules, workflows, compatibility, or other maintained requirements.

Specifications are canonical project sources. Brain decisions and Memory Bank
chunks may link to them, but should not copy their full content.

### Durable Memory Bank chunks

Use a Memory Bank chunk only for a verified, reusable consequence that remains
useful across tasks and is not already adequately owned by a living spec or
other canonical source.

Examples include a durable convention, a stable integration failure rule, or
an operational recovery lesson. Prefer updating an existing chunk over
creating a near duplicate. Mark contradicted knowledge `superseded` and link
its replacement.

### Local episodes

Use `record` or lightweight completion only for a non-authoritative local replay
aid. An episode is not a shared task, governed event, durable memory, or
promotion source by itself.

## Record Identity, Ownership, and Revision

Governed records use UUIDv4 identity. Human ticket IDs and descriptive IDs are
external aliases. Relationships such as conflicts, supersession, task/handoff
links, and promotion source bindings use UUIDs.

Each dynamic record contains:

- an owner and `authorized_owners`;
- an integer revision beginning at `1`;
- append-only lifecycle transitions with actor, time, and reason;
- strict type-specific status;
- privacy, authority, and confidence metadata;
- source paths and SHA-256 fingerprints;
- explicit conflict and supersession relationships.

Mutation authorization and retrieval eligibility are different controls:

- a mutation actor must be listed in the record's `authorized_owners`;
- `runtime.json` `owners` participates in restricted-record retrieval filtering
  and does not grant mutation rights;
- the default configuration uses `owners: ["*"]`, but restricted records are
  still excluded unless `restricted` is added to `allowed_privacy`;
- private records are always excluded by the runtime's Brain eligibility check.

Optimistic revision checks prevent two writers from unknowingly updating the
same version. `brain-update` requires a revision in the parser. Governed task
`update` and `complete` accept an omitted revision for compatibility, but
operators must always supply it; omission means the runtime does not perform
the intended compare-and-swap check.

`--revision auto` resolves the current revision inside the runtime mutation
lock, so the read and the write it feeds form one compare-and-swap. It exists
for automated writers, which cannot know the revision in advance and would
otherwise have to omit the check entirely. An operator who knows the revision
should still pass it: only an explicit value can detect that the record moved
between reading it and deciding to write.

## Indexing

The indexer discovers eligible files from fixed repository patterns, including:

- `AGENTS.md` and `CLAUDE.md`;
- mirrored edition skills;
- root `README.md`;
- `specs/**/*.md` and `docs/**/*.md`;
- active `memory-bank/chunks/*.md`;
- `tasks/**/*.md` and `Task/Epics/**/*.md`;
- `CHANGELOG.md`;
- eligible active Project Brain dynamic records and handoffs.

Before reading a discovered repository document, the runtime excludes
Git-ignored paths. It skips symlinks and non-files, rejects likely
secret-bearing content, includes only validated active Memory Bank chunks, and
requires UTF-8. Invalid UTF-8 aborts the refresh without replacing the prior
index.

For Brain records, eligibility is checked before insertion. Private,
disallowed-privacy, disallowed-authority, terminal, superseded, stale, or
invalid records are excluded with safe reason metadata. Handoffs are indexed
only when their task is eligible and the handoff validates against it.

Indexing replaces the FTS document and metadata tables transactionally. It does
not modify canonical source files or transform indexed content into truth.

Important boundaries:

- the index stems with `porter unicode61`, so "review" reaches a document that
  only says "reviewer"; without stemming the correct skill is simply missed;
- two patterns may match one file; the earlier one owns its layer and kind, and
  the file is indexed once;
- the fixed patterns do not index arbitrary application source code;
- `search` queries the existing index and does not refresh it;
- parity drift is reported by `index`, but only `parity` makes drift fatal;
- `retrieve` and the request hook refresh the index; `index` remains the
  explicit full rebuild.

`index --incremental` reuses every row whose source modification time and size
are unchanged, so only new or modified documents are re-read and re-scanned.
Project Brain records are always rebuilt, because their eligibility depends on
configuration, lifecycle, and cross-record conflict state rather than on the
record file alone. A change that somehow reproduced its predecessor's stat is
not served as truth: retrieval re-hashes every candidate and excludes the
mismatch as stale.

## Search and Governed Retrieval

Capsule assembly filters for relevance before ranking. Terms matched by more
than half the corpus are dropped as noise — measured against the index rather
than a stopword list, so it adapts to the languages a repository documents
itself in. A document then qualifies either by containing two distinct query
terms, or by containing one term rare enough in this corpus to be evidence on
its own.

That second route matters more than it looks. Counting terms equally punishes
exactly the wrong document: a focused note containing only the rare term that
matters scores one, while filler sharing two unremarkable words scores two and
takes the slot. Admitting a distinctive single match lets some noise back in on
queries no document covers, which is the cheaper error — a spurious result
wastes a slot, a hidden one denies an answer the project already holds.

Ranking weights what a document declares itself to be about. Each document is
indexed with a `summary` — a frontmatter `description` where one exists, the
opening prose otherwise — weighted well above the body, because skill bodies
are procedural prose that reads much alike while the description states the
topic.

This is lexical retrieval, and its limit is real: it cannot distinguish a
document *about* a subject from one that merely contains a generic word from
the question. A query whose subject no document covers will still surface the
least-bad lexical match rather than nothing.

Local latent-semantic embeddings were built and measured against that gap, and
they did not close it. On a corpus this size the latent space encodes prose
style rather than topic: "what is the airspeed velocity of a swallow" scored
0.76 against the skill corpus while "design the database schema for invoices"
scored 0.48, so no similarity floor separates a relevant query from an absurd
one. Indexing declared descriptions instead of bodies improved ranking but not
separability. Closing the gap needs a trained embedding model, which would cost
this runtime its standard-library-only, network-free contract — a trade to make
deliberately, not as a side effect of tuning retrieval.

`search` is broad lexical discovery across the existing index and local
episodes. It stays unfiltered by design. It supports a procedural, semantic, or episodic layer filter. It does
not require a task and does not create a manifest.

`retrieve` is task-aware. In governed mode it:

0. refreshes the index incrementally, degrading to a warning rather than an
   error, because governed retrieval reads the index and not the sources;
1. resolves the supplied external task ID through the local binding;
2. loads the authoritative Brain task;
3. uses FTS5/BM25 to identify candidates;
4. applies runtime privacy, owner, authority, lifecycle, and freshness filters;
5. fetches eligible linked conflict partners by UUID even if they did not
   lexically match;
6. selects bounded snippets within category and total estimated-token budgets;
7. writes a strict retrieval manifest;
8. returns the working task, categorized selected context, exclusions, token
   estimates, and manifest path.

`context` is a compatibility alias. New operational guidance should use
`retrieve`.

In lightweight mode, `retrieve` and `context` build a simpler layered packet
from the local task, document search, and local episodes. They do not run the
governed manifest and eligibility pipeline.

## Privacy, Authority, and Freshness

Privacy expresses who the record is intended for:

- `public`;
- `team`;
- `restricted`;
- `private`.

The shipped configuration allows `public` and `team`. The runtime always
excludes private Brain records. Restricted owner filtering exists but only
matters if `restricted` is included in `allowed_privacy`.

Authority expresses the evidentiary quality of a record:

- `inferred`: reasoned but not directly established;
- `observed`: supported by an observation;
- `verified`: checked against authoritative evidence.

The shipped configuration allows `observed` and `verified`, so inferred records
remain governed but are not selected by normal indexing/retrieval.

Confidence is a number from `0` to `1`. The schema validates it, but the current
retrieval code does not threshold or rank by confidence. Do not promise that a
low-confidence record will be filtered automatically.

Freshness has three checks:

- the indexed document's current hash must match the hash stored when indexed;
- for Brain records and handoffs, cited source paths must still match the
  stored source fingerprints;
- for a codebase map under `codebase/`, the commits landed on its
  `mapped_scope` since its `mapped_commit` must stay within
  `codebase_map_max_drift`.

The third check exists because a codebase map describes code rather than
itself. Its own bytes staying unchanged proves nothing: the content hash that
keeps every other document honest cannot tell that the code moved on
underneath it. A map that no longer describes the code is worse than no map,
because an agent acts on it instead of reading the source. A map that records
no commit is excluded as `map-unverifiable` rather than assumed current, and
`refresh` reports how far each map has drifted so the silence of an exclusion
is not the only signal.

Indexing a map is not indexing application source. The fixed patterns still do
not read arbitrary code; they read a document *about* it, which a skill writes
and the runtime only governs.

Freshness proves that bytes have not changed since fingerprinting. It does not
prove that the claim is semantically correct, complete, or still applicable.
Operators must still inspect canonical sources.

## Conflicts and Supersession

Conflicts represent simultaneously relevant disagreement. They are UUID
relationships, not free-form claims. When one side matches retrieval, the
runtime attempts to retrieve eligible linked partners even without a lexical
match. Conflict pairs may exceed normal category and target budgets up to the
hard ceiling, and the manifest records why.

A conflict partner can still be excluded for privacy, ownership, authority,
lifecycle, or freshness. The runtime must not leak it merely to complete the
pair.

Supersession represents replacement, not disagreement. A superseded record is
excluded from active retrieval and can be compacted. Relationships require a
replacement backlink: the old record's `superseded_by` and the replacement's
`supersedes` must agree. Current public CLI update flags can append conflicts
but do not expose general supersession editing; use a reviewed maintenance
workflow rather than inventing unsupported flags.

Memory Bank has its own chunk-level `supersedes` and `superseded_by` lifecycle.
Do not assume Brain supersession automatically updates durable memory.

A chunk may also carry a validity period, `valid_from` and `valid_to`. The two
answer different questions and neither replaces the other: `superseded_by` says
*what* took a chunk's place, `valid_to` says *when* it stopped being true. Both
are optional, so chunks written before periods existed stay valid.

`valid_from` may precede `created` — knowledge is often true well before anyone
writes it down. A closed period pairs with a status: `superseded` when a
successor took over, which the bank requires a link for, and `archived` when
the knowledge simply ceased and nothing replaced it. The boolean model could
not express that second case at all.

An active chunk may not sit past its `valid_to`, the same rule `review_after`
already applies. Closing a period therefore removes the chunk from retrieval
without deleting it: automatically written memory earns a boundary rather than
an erasure, and what was believed during that period stays readable.

## Budgets and Snippets

Governed retrieval estimates tokens as roughly one token per four characters.
This is a deterministic safety budget, not a tokenizer-accurate provider usage
measurement.

The category limits are policy 1,200, handoff 1,500, durable 3,500, dynamic
1,500, and evidence 2,000 estimated tokens. The normal target is 8,000 and the
hard ceiling is 12,000. Individual snippets are capped at 1,200 characters.

These controls bound selected retrieval context, not the size of source files,
Brain records, user prompts, or model responses. Conflict escalation can exceed
normal category/target budgets, never the hard ceiling.

## Retrieval Manifests

Every successful governed retrieval writes a manifest. By default it lands in
the Git-tracked `project-brain/control/retrieval-manifests/<uuid>.json`.

`--ephemeral` writes the same validated record to
`memory-bank/local/retrieval-manifests/<uuid>.json` instead. Automated
retrieval must use it: one manifest per request would otherwise add thousands
of files to shared history. Local manifests are ignored, are capped at the most
recent 200, and are provenance for the local machine only.

A manifest binds:

- creation time and query;
- Brain task UUID and revision;
- privacy, owner, authority, freshness, and active-only filters;
- selected paths, categories, estimated tokens, and source hashes;
- excluded paths and safe reasons;
- provider;
- target/hard estimates and any conflict escalation reason.

The manifest is provenance metadata, not a context snapshot. It does not retain
the selected snippets or full source bodies, and it cannot replay the exact
prompt seen by an agent. It deliberately excludes prompts, responses, hidden
reasoning, and tool payloads.

Governed manifests are Git-trackable and validated, but the current compactor
does not archive or prune them. Retention for those requires an explicit
repository policy. Only the local `--ephemeral` store prunes itself.

## Handoffs

A governed task handoff is a compact continuation record containing objective,
current state, next actions, files, sources, owner/privacy, task revision, and
active/closed status.

The runtime creates it with the task, refreshes it on supported task updates,
closes it on completion/cancellation, and archives it with its terminal task
during compaction.

A task may declare a `phase` — `understanding`, `planning`, `execution`, or
`finalization` — using the same vocabulary the skills declare in their own
frontmatter. Progress says what was touched; the phase says which step of the
loop the work stopped on, which is what a reader needs in order to resume. The
handoff carries it.

`phase` is optional, and deliberately so: requiring it would invalidate every
record written before it existed. An automatically provisioned task declares
none, because nothing observed its phase. Only a task may carry one.

A good handoff:

- states what is true now;
- identifies the exact next action;
- references relevant files and sources;
- reflects the same task revision;
- omits conversation, raw diffs, logs, and speculative narrative.

The handoff improves continuation but does not transfer the ignored SQLite
binding. On another machine, Git carries the handoff and task while the current
compatibility commands still require a local binding that the CLI cannot
automatically reconstruct.

## Promotion to Durable Memory

Promotion has two modes, and a promotion record always states which one it used.

The reviewed mode is the three-stage process:

```text
propose -> independent human review -> apply
```

The automatic mode, enabled by `automatic_promotion` in `runtime.json`, runs
unattended on the same boundary as the working-memory flush:

```text
propose -> apply
```

There is no reviewer in the automatic mode, and the runtime refuses to pretend
otherwise. `reviewer` stays null, `review_mode` is `automatic`, the outcome is
recorded as `approved-without-review`, and the resulting chunk carries an
`auto-promoted` tag so the Memory Bank itself shows which knowledge no human
approved. `promote-review` rejects an automatic promotion outright rather than
letting a human signature be attached after the fact.

Eligibility is deliberately narrow, because nothing downstream will catch a bad
promotion:

- only `finding: resolved`, `bug: resolved`, `incident: closed`, and
  `decision: accepted`;
- only `verified` authority, since there is no reviewer to question an
  unverified claim;
- only privacy the runtime already allows, and only with fresh source
  fingerprints;
- never a task. Auto-checkpoint progress describes what happened in a session,
  not a consequence worth carrying into another one.

A record must also carry content beyond its own title. A record's rendered
body is mostly lifecycle scaffolding, and for a decision the goal merely
repeats the title, so the promoted chunk is assembled from the record's
progress note and cited sources rather than copied verbatim. Durable memory is
therefore only as good as what was written into the record: the pipeline
carries knowledge, it does not synthesize it.

A source already bound to a non-rejected promotion is never promoted again, so
repeated runs do not fill the bank with duplicates. At most five records are
promoted per run and the remainder is reported, not dropped.

Every rule that holds a record back reports its reason. Editing a cited source
is enough to block a decision forever, and an unexplained absence from durable
memory is impossible to notice otherwise.

Automatic promotion writes durable, Git-tracked memory with no human in the
loop. That is the trade it makes: continuity without attention, at the cost of
the check that would have caught a wrong or unreusable claim before it became
durable. Set `automatic_promotion` to `false` to return to reviewed promotion.

Proposal records bind each source Brain record's exact UUID, type, path, and
revision. In the reviewed mode an independent human reviewer must approve, and
the proposer cannot review their own proposal. Apply rechecks every source
binding in both modes, so a changed source revision invalidates the proposal
rather than silently promoting changed content.

Application allocates a Memory Bank ID, writes a chunk, updates the index and
counter, validates the bank, and updates the proposal. These writes are
snapshotted and rolled back together on failure.

Promotion is appropriate for a reusable consequence, not:

- active progress or a handoff;
- an unresolved finding or conflict;
- raw evidence or incident payloads;
- generic framework knowledge;
- private/restricted or sensitive material;
- content already owned by a living specification;
- a local episode with no governed, reviewed source.

The runtime's automatic promotion application currently creates a fixed
`decision`-type, application-scoped memory chunk with predefined tags and a
one-year review date. It does not search semantically for duplicate chunks or
choose a richer category. Human review should catch duplication and ownership
problems before approval.

## Compaction and Retention

Compaction validates the repository, then moves terminal or explicitly
superseded dynamic records and associated handoffs into `project-brain/archive/`.
It rebuilds deterministic indexes, validates the result, and restores all
moves/indexes on failure.

Compaction is archival, not deletion. Archived records remain subject to the
same strict schema and relationship checks and can serve as promotion sources.

Because a Memory Bank chunk cites its source record by path, compaction
repoints any chunk citing a record it moves, atomically with the move and its
rollback. Without that, archiving a promoted record would leave a dangling
citation, fail Memory Bank validation, and block every later promotion.

The current compactor does not process:

- retrieval manifests;
- promotion records;
- Memory Bank chunks;
- local SQLite episodes;
- lightweight working tasks;
- stale but nonterminal records merely because their source changed.

“Compact” therefore must not be described as global garbage collection or a
privacy erasure mechanism.

## Telemetry and Observability

Telemetry is disabled by default in both `runtime.json` and
`project-brain/config/telemetry.json`. The provider configuration is local and
network-free.

The telemetry contract, if enabled by future reviewed runtime work, is
metadata-only. Its strict schema permits provider, operation, input/output/
context token counts, timestamp, event UUID, and an optional task UUID.
Configuration explicitly prohibits:

- prompts;
- responses;
- source bodies;
- tool payloads;
- secrets;
- customer data;
- raw logs.

The current context CLI does not emit telemetry events. A schema and config are
a contract, not evidence that collection is implemented. Retrieval manifest
token estimates are also not telemetry or billable usage.

## Data Loss and Recovery Boundaries

### Git-tracked shared state

Subject to normal Git history and repository backup, these are shared:

- Project Brain dynamic records and handoffs;
- active/archive deterministic indexes;
- retrieval manifests and promotions;
- archived Brain records;
- Memory Bank chunks, index, counter, templates, and contract;
- canonical source documents.

Uncommitted tracked files are not yet durable on another machine. Git can
preserve history, but it does not protect secrets that were mistakenly
committed; prevention remains mandatory.

### Ignored local state

These are machine-local:

- `memory-bank/local/context.db`;
- the Project Brain runtime lock;
- governed task bindings;
- lightweight working tasks;
- local replay episodes;
- derived FTS documents and metadata.

Deleting the database permanently removes bindings, lightweight tasks, and
episodes. Only the document index can be rebuilt. The shared Brain record is
not deleted, but current compatibility commands may no longer locate it by
external task ID without a binding.

### Atomicity limits

The runtime uses SQLite transactions, atomic file replacement, repository-wide
file locking, snapshots, and compensating rollback for supported operations.
Tests cover many write boundaries, including task cross-store operations,
compaction, and promotion.

These controls reduce partial-write risk; they are not a substitute for
filesystem durability, backups, Git review, or recovery after power loss,
process termination at an untested boundary, manual edits, disk exhaustion, or
concurrent tools that bypass the runtime lock.

### Privacy deletion

Compaction does not erase data, and Git history may preserve deleted tracked
content. Never store sensitive data in the first place. If sensitive material
enters tracked history, use the repository's incident and history-rewrite
procedure with appropriate authorization; normal context commands do not solve
that problem.

## Another-Machine Continuity

Governed mode provides shared continuity through Git-tracked tasks and
handoffs, but it is not fully location-transparent:

```text
Transferred by Git:
task + handoff + records + manifests + promotions + durable memory

Not transferred:
SQLite binding + local index + episodes + lightweight state
```

On a new machine, validate the pulled Brain, validate Memory Bank, rebuild the
index, use the same authorized owner, and verify canonical sources. Plan
explicitly for the current binding limitation described in the
[operations runbook](OPERATIONS.md#missing-governed-task-binding).

If work must survive without special recovery, do not leave its only meaningful
state in a lightweight task or local episode. Capture it in the appropriate
governed record, task document, living specification, or independently reviewed
Memory Bank chunk.

## Hooks and Explicit Actions

Session-start hooks can report metadata such as:

- branch and uncommitted-change count;
- dependency/runtime markers;
- selected context mode;
- index existence and approximate staleness;
- active local binding count;
- validation status;
- Memory Bank availability.

Session-start hooks print no record bodies and inject no context.

Two further hooks automate working memory when the edition enables them:

```text
UserPromptSubmit -> working-memory-read.sh   (read: refresh + Task Capsule)
Stop             -> working-memory-write.sh  (write: buffer + bounded flush)
```

The split is deliberate. At prompt time nothing has happened yet, so there is
no delta to record; a request is the right moment to *read*. The delta exists
at the end of a turn, which is the right moment to *write*.

The read hook runs `refresh`, which re-indexes procedural, semantic, and
episodic memory in one incremental pass and reports each layer as `updated` or
`failed`:

```text
Procedural  AGENTS.md, CLAUDE.md, mirrored edition skills
Semantic    README, docs, specs, active Memory Bank chunks, task documents,
            capability epics, eligible Brain records and handoffs
Episodic    CHANGELOG.md, plus local episodes at query time
```

Working memory is deliberately not in that list. It is not refreshed from
sources; it is written by the turn hook, and in governed mode Project Brain
remains its only authority.

The three layers come from one indexing pass and therefore succeed or fail
together, but they are reported separately so a stale layer is visible rather
than silently narrowing the result. When the hook also has a task — from
`CONTEXT_TASK_ID` or the current branch — the same process assembles a bounded
capsule with `--ephemeral`, avoiding the second index pass a separate
`retrieve` would run. A capsule failure is a warning; the layer refresh stands.

The capsule is retrieved context, not authority: the ordinary hierarchy still
applies, and a capsule entry never outranks the source it summarizes.

The write hook reads Git porcelain metadata only. Working-tree contents never
reach the buffer, paths that look sensitive are excluded and reported, and the
runtime's own record, handoff, and index churn is dropped rather than recorded
as user work. Turns accumulate in ignored local state and flush together, so
continuity costs one governed revision per `--flush-after` turns instead of one
per turn. A flush contributes at most `--max-files` paths, and reports the
remainder rather than dropping it silently.

The first flush provisions the task if it does not exist. Automated continuity
is worthless if it buffers into a task nobody created, and requiring an
operator to run `start` first would mean the automated path only works after a
manual step. Provisioning happens at flush time, not at session start, so
visiting a branch mints nothing; only accumulated work does.

An automatically created task states its own provenance, because a goal cannot
be edited after creation:

```text
feature/add-caching  ->  Add caching (auto-provisioned from feature/add-caching)
```

Ticket identifiers survive intact: `BAUMAS-133` is a name, not two hyphenated
words. A task that already exists is never overwritten, so an operator who ran
`start` with a real goal keeps it. Set `CONTEXT_TASK_ID` to bind work to
something other than the branch.

Provisioning writes a Git-tracked record. On a long-lived branch that is the
point; on many short branches it accumulates tasks that `compact` archives only
once they reach a terminal status.

Both hooks are fail-open and time-bounded: context tooling never blocks a
prompt or turns a checkpoint failure into a turn error.

With `automatic_completion` enabled, the turn hook also completes tasks whose
branch has landed in the default branch, recording an episode as it goes. The
scan covers every active task rather than the current one, because a merge is
observed after the branch is left, not while it is being worked on. Detection
is `git merge-base --is-ancestor`, so merging the default branch *into* a
long-running branch never looks like completion, and a branch is skipped when
it is the default branch itself, which is trivially its own ancestor. A deleted
branch is never treated as merged: deletion cannot be told apart from
abandonment.

The outcome claims only what was actually checked:

```text
Branch feature/reports merged into main. Last recorded progress: <checkpoint>
Verification: feature/reports is an ancestor of main
```

Nothing here verified that the work is correct, so the record does not say so.

With `automatic_promotion` enabled, the turn hook also promotes eligible
resolved knowledge into durable memory on the same boundary, and with
`automatic_compaction` it archives terminal records once `compaction_threshold`
of them have accumulated. Compaction runs in batches rather than every turn:
archiving one record at a time would churn Git history for no benefit.

The order within a turn is fixed — complete, promote, then archive — because a
record must be promoted before it is moved. Archived records stay promotable
anyway, so a promotion run that hits its per-run cap does not lose the
remainder to the archive.

This distinction matters: a healthy status line means the tools are available;
it does not mean an agent has loaded, verified, or acted on the relevant
context.
