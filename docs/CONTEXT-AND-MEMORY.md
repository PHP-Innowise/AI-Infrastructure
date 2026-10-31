# Context and Memory

The accelerator separates current truth, active work, local retrieval, and
durable learning so that continuity does not become a competing source of
truth. This guide explains those boundaries and the reasoning behind them.

For commands and recovery procedures, see [Context and Memory Operations](OPERATIONS.md).
For mode selection and switching, see [Context Modes](CONTEXT-MODES.md). For a
step-by-step task narrative, see the
[User Task Workflow Example](examples/USER-TASK-WORKFLOW-EXAMPLE.md).

Automatic read hooks distinguish an explicit `CONTEXT_TASK_ID` from a task
inferred from the branch. Only inferred reads carry a private expected-branch
marker. The helper checks the full symbolic branch before creating state and
again at delivery boundaries; named branches do not need a first commit.
Explicit requests may recall another task while on a different branch.

Governed retrieval checks admission after packing and schema validation,
before publishing its manifest and delivery receipts. Read-only working
context checks after rendering; marked hook-context failures cannot become
valid no-context or gate-skip return codes. Marked output is checked again
before printing. The branch is asked once per command - one `symbolic-ref`
with a two-second deadline; the checks at delivery boundaries compare the
checkout's HEAD file (and the reftable list) with the snapshot taken before
that answer, and ask Git again only when one of them is a new file.
These are point checks, with no atomic branch epoch or publication lock: a
later refusal may follow earlier metadata writes and does not imply rollback.
Legacy unmarked invocations cannot prove inference retrospectively. Stop
checkpoint attribution, raw cached search policy and downstream Cursor rule
publication retain their separate contracts.

Source-ignore checks use the project's observed Git top, relative prefix and
Git directory. Inherited Git repository, index, object and discovery selectors
(`GIT_DIR`, `GIT_INDEX_FILE`, `GIT_CEILING_DIRECTORIES`, ...) are dropped from
the probe's environment with the other command-environment overrides: the probe
runs `git -C <project>`, which finds the project's own repository, and ordinary
global exclusions and protected trust settings stay. A `.git` directory without
HEAD, objects and refs - an empty `~/.git` or `/tmp/.git` some tool left - is
skipped as Git itself skips it. Unknown `.git` entries (special files, dangling
links) and failed or malformed probes refuse source admission. The first
ignore check of a command confirms the frame with Git before and after its
query; later checks in the same command reuse it while the directories from
the project to its top, each `.git` marker, the admin directory, its config
and its commondir keep their file identities, and confirm it with Git again
when one changed.
Read entry points observe the Git frame before creating local state. Later
ignore failures stop publication and cannot become index warnings that still
expose progress; that later check does not promise zero local mutations.
Ordinary accelerator-source exemptions do not apply to working citations.

Each ordinary ignore operation shares one second across its preflight, query
and postflight; working-source checks share 0.1 seconds. Read entry admission
adds a separate one-second frame check. Output admission occurs after capture,
so these limits do not bound producer allocation, cleanup, filesystem races or
the complete hook. Branch inference and other Git evidence paths retain their
own contracts; this source check is not a new current-branch certificate.

Cold refresh screens source documents before publishing the complete index.
Exact known secret patterns skip full scans when a mandatory literal is
absent; the original regular expressions still decide every match and
replacement. This reduces work on ordinary prose without changing source
admission or masking. It does not provide a scan checkpoint: a large or slow
project can still exceed the hook's five-second budget and remain incomplete.
Deleting the disposable local index leaves the canonical sources intact.

The exact assigned-credential pattern finds key-and-separator positions before
checking their complete underscore prefixes with the original expression.
This avoids retrying every prefix suffix when a long prefixed lookup contains
no literal secret. Masked text, match spans and privacy decisions stay the
same; changed patterns retain their original search. Value lookahead and other
refresh stages can still be slow, so this is not a general deadline guarantee.

An optional Project Brain runtime configuration uses defaults only after
ordinary absence is observed. A directory, special file, dangling link,
unreadable file or invalid UTF-8 produces an explicit configuration error.
The loader and configuration fingerprint share the same strict text reader;
readable links, all valid object fields and the existing newline hash remain
supported. POSIX nonblocking open rejects a file replaced by a FIFO before
reading. This does not provide a filesystem deadline, a coherent snapshot
across separate calls, or the same FIFO race guarantee on Windows.

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
- a derived identifier index that maps "get user by id" to `getUserById`, so
  prose reaches a code name without a model; the identifier itself remains an
  exact token and is never expanded into its words;
- task-aware context assembly;
- eligibility metadata and freshness checks;
- bounded snippets and estimated-token budgets;
- machine-local task bindings in governed mode;
- optional local working tasks and replay episodes in lightweight mode.

The database is derived and disposable only in part. Its document, metadata
and identifier indexes can be rebuilt from repository sources. The identifier
table stores only path, identifier and expansion; a changed document replaces
its rows, and an index created before the table rebuilds it once. Query
expansion is a candidate aid, never an admission rule: coverage, authority,
privacy, freshness and source hashes still decide what a capsule may carry.

Governed compatibility bindings can be restored from Git-tracked task records
with `rebind`, and the next automated turn flush performs the same restoration.
Lightweight tasks and local episodes remain machine-local and cannot be
reconstructed after deletion.

The active provider is local `sqlite-fts5`: network access and embeddings are
disabled. An external provider entry exists only as a disabled contract.
Retrieval is not only on request: each edition's prompt hook injects a bounded
Task Capsule into every request (Cursor reads it from the rendered
`.cursor/rules/working-memory.mdc`), and the local memory MCP server
(`memory-bank/scripts/mcp_server.py`, see `memory-bank/MCP.md`) offers the
same runtime to the client as tools.

### Memory Bank

`memory-bank/` is Git-tracked, durable project memory shared across supported
agent editions. Depending on `automatic_promotion` in `runtime.json`, entries
arrive through independent human review or explicit automatic promotion; the
latter are labeled `auto-promoted`. It stores cohesive, reusable, source-backed
knowledge that will help more than one future task, for example:

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

Completing a governed task also writes a Git-tracked `event` record beside the
local episode, so the one layer meant to hold "what happened here" survives a
fresh clone instead of living only in the disposable database. `event` is the
record type that reports that something happened rather than what to do about
it, and it is excluded from promotion for exactly that reason — the record
fixes a lifecycle transition that occurred and asserts nothing about how to
act. It is best-effort: a task that completed is never reopened because its
episode could not be written. The reverse holds as well: a completion that
fails, up to and including the database commit, takes its event with it, so
no record reports a completion that did not happen.

`event` is the only record type mapped to the episodic layer. An `incident`
stays semantic even though it is also a record of something that happened: an
open incident is active, urgent, promotable content, and moving it to the
history slot would take it out of the semantic layer's three slots.

The episodic slot of a governed capsule is ranked by `retrieve()` along with
everything else. It used to be fetched separately by a query that never joined
the metadata table, which applied no privacy, owner, authority, lifecycle or
freshness filter and left the delivered document out of the manifest entirely.
That was harmless while the only episodic document was the changelog; it stops
being harmless once governed records live there.

Machine-local episodes also enter capsule assembly before the retrieval gate,
not as an unaccounted append after it. They share the recorded-history slot
with events (below), contribute to the capsule and token ceilings, and their
content hashes participate in repeat detection. A manifest records only
`local_episode_count` and their aggregate `token_estimates.local_episodes`: it
never stores a local episode ID or body.

### Recorded history beside the changelog (2026-10-10)

The episodic layer holds two items: the changelog, ranked by the main query as
before, and one recorded history item - a Project Brain event or a local
episode - found by a search of its own. They used to share one slot. On the 121
evaluated prompts the changelog held it on every turn that had an event, while
19 existing answers sat in 13 automatically written events that were never
delivered; every subject term of those events had been dropped by the 24-term
distillation, because an event's words also occur in the changelog, the README
and the task documents and are never among the corpus-rarest.

- **The search.** `history_query` matches every informative term of the whole
  request (document frequency at most half the index) against the subject only
  - an event's title and goal, an episode's summary and outcome - and keeps,
  of the terms that reach any history subject, the 24 rarest, so a long prompt
  cannot win by length. Events are no longer candidates of the main query: they
  take none of its rows and do not set the episodic floor the changelog is held
  to.
- **The bar.** An item is admitted when it covers `required_coverage` distinct
  tokenizer concepts. Porter variants such as `session` and `sessions` count
  once toward coverage; a query offering just one concept needs one. The
  existing raw matched-term weights still determine the floor,
  `k * ln((N + 1) / 1.5)`, and ranking. Normalization does not refill the raw
  24-term window or change those weights. Nothing is tuned to the evaluation
  prompts, and the bar is absolute: it never admits the best item of a bad list.
- **One slot.** The best admitted event that passes the runtime filters, or
  the best local episode if it ranks higher (ties go to the event: `complete`
  writes both). An episode adds to a term's frequency only where no document
  has the term, so the twin does not make the event's own words look common.
- **Render.** The history item renders before the changelog, so under the
  3,600-character ceiling the changelog's long excerpt shrinks first; a local
  episode now quotes its outcome, not only a summary.
- **Cost.** At most two FTS lookups per informative term, only when events or
  episodes exist: a few milliseconds per prompt.

The governed JSON contract is 1/3/2; lightweight mode keeps one history item,
because it has no such search. A delivered event no longer takes one of the
three semantic slots on its way out, which had lost the third semantic item.

## Governed and Lightweight Ownership

The default governed model is:

```text
Shared active work       -> Project Brain
Durable reusable memory  -> Memory Bank
Search and local binding -> Local Context Engine
Current behavior         -> canonical project sources
```

Governed mode is appropriate when work crosses agents, sessions, or machines;
concurrent updates matter; handoffs are required; or records may later support
reviewed or configured automatic promotion.

The explicit lightweight model is:

```text
Local active work        -> SQLite working_tasks
Local completed replay   -> SQLite episodes
Durable reusable memory  -> Memory Bank
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
rejection and may later be superseded. Record the alternatives that lost and
why; that section is what stops a later task from proposing the same rejected
option. If the decision changes architecture, API behavior, schema, security
behavior, or another living contract, update the canonical specification too;
the Brain decision does not replace it.

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
- `codebase/**/*.md` — recursive, so a map filed per module is indexed whole
  rather than only at its top level;
- active `memory-bank/chunks/*.md`;
- `tasks/**/*.md`;
- `CHANGELOG.md`;
- eligible active Project Brain dynamic records and handoffs.

Before reading a discovered repository document, the runtime excludes
Git-ignored paths. It skips symlinks and non-files, masks each value a secret
pattern recognises in a repository document (`[redacted: <label>]`, the key
such as `MAILER_PASSWORD` kept) and excludes a document whose masking does not
converge, includes only validated active Memory Bank chunks (a chunk with a
likely secret is still excluded whole), and requires UTF-8. The stored source
hash of a masked document is the file's, so governed retrieval does not call it
stale; `index --json` lists masked documents under `redacted` with a count of
values, never the values. A change to the secret patterns re-reads every
document once. Ordinary documents with invalid UTF-8 contents or non-UTF-8
path keys are omitted individually, before Git probes and SQLite writes for
those paths. Valid Unicode names remain supported. Index/refresh JSON reports
fixed reasons and opaque labels for malformed paths; refresh capsules and
health records expose only nonzero `source_notices` counts. These notices do
not disable retrieval and do not contain names or contents. A later successful
scan removes the notice; a renamed invalid source cannot retain its old row.
Malformed skill paths still fail strict policy inventory checks. Direct and
supplied-stat fingerprints, skill parity, full canonical/reverse inventories
and cross-edition parity reject malformed admitted keys with a fixed error,
before producing a digest or a clean parity result. Full mirror checks admit
reverse keys before the partial-install shortcut. Valid Unicode names, normal
digest serialization and drift shapes remain unchanged; documented ignored
suffixes and symlinks retain their existing treatment. Brain dynamic/archive
inventories reject malformed path keys before their first yield, preserving
strict lookup and duplicate admission. Direct record reads and held-text parses
also refuse them. Managed handoff discovery/validation admits each path list
before per-file error handling, so a malformed path cannot become a harmless
exclusion. Active handoffs are not a global preflight on every task lookup.
Portable handoff input/output/transcript/citation paths fail with a fixed error
before their affected reads/writes or returned-path construction. These are
effective absolute-path checks for file arguments as well as spelling checks;
an unrelated malformed current directory does not reject valid absolute paths.
Shared curated file-reference validation also rejects escaped-surrogate paths
before loaded handoff bodies are encoded or provenance probes run. These are
UTF-8 admission checks, not atomic filesystem or all-control-store validation;
valid non-ASCII paths retain their prior semantics and canonical-path rules.
Memory Bank validation reports invalid UTF-8 INDEX/chunks
as fixed, bank-relative errors; its summary skips invalid chunk metadata and
does not constitute successful validation.

For Brain records, eligibility is checked before insertion. Private,
disallowed-privacy, disallowed-authority, terminal, superseded, or invalid
records, and records whose cited source was deleted (`source-missing`) or never
digested (`source-undigested`), are excluded with safe reason metadata. A record
whose cited file was edited is indexed; retrieval marks it `source_changed` and
ranks it at half its score, as it does a chunk. Handoffs are indexed
only when their task is eligible and the handoff validates against it.

Indexing replaces the FTS document and metadata tables transactionally. It does
not modify canonical source files or transform indexed content into truth.

A pattern that matched files on disk and lost every one of them to
`.gitignore` is reported as an exclusion named for the pattern, with the
reason `pattern-all-git-ignored`. Individual ignored files stay silent on
purpose — Symfony alone contributes hundreds — but a whole source of truth
going dark is not a routine skip. Measured on a real installation: a project
whose own `.gitignore` carried a bare `docs` entry indexed 96 accelerator
skills, one `README.md`, and none of its own design documents, without a word.

The same pass builds `document_links`, the reverse index of which document
declares which source (`ref_kind` `source`), read by `links` and by `retrieve
--path`, and which active Brain task touched which file (`ref_kind` `file`,
from the task's newest 50 `files[]` entries Git tracks, screened; see "Files
the work touched" below), rebuilt with the records on every refresh. It is derived
and disposable: rows follow their document, and when the table is absent the
runtime drops the stat cache so the first index after an upgrade re-reads
every candidate. Populating it incrementally would leave it complete only for
whatever happened to change next, while claiming to be complete.

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
are unchanged and whose calendar boundary has not passed, so only new,
modified, or newly out-of-date documents are re-read and re-scanned. The two
conditions answer different questions and only the first is a filesystem
fact: a durable memory chunk stops being servable when a date arrives, which
no `stat` can see. The cache therefore stores `min(review_after, valid_to)`
alongside `(mtime_ns, size)`, and the day after that boundary the row is
re-validated instead of reused. Project Brain records are always rebuilt,
because their eligibility depends on configuration, lifecycle, and
cross-record conflict state rather than on the record file alone; the rows
are written only when they come out different from the stored ones (a digest
is kept with them), so an unchanged Brain leaves the tables, the index
generation and the token frequencies cached against it as they are. A change
that somehow reproduced its predecessor's stat is not served as truth in
governed mode: retrieval re-hashes every candidate and excludes the content
mismatch as stale.

### Two facts linked only by a shared source: a measured limit

The second measurement of what lexical retrieval can and cannot reach, beside
the embeddings result above.

Two durable chunks were written whose only connection was a `sources[]` entry
naming the same file — no shared vocabulary of their own. A query phrased in
the *source's* words, which appear in neither chunk, retrieved the source
document and **neither chunk: 0 of 2**. A control query using words the two
chunks do share retrieved **2 of 2**.

So co-retrieval of two related facts is available exactly when they happen to
share the query's vocabulary, and unavailable when the relationship is the
only thing joining them. `supersedes` and `superseded_by` are not read during
retrieval, and no other edge between chunks exists, so nothing walks that link.
That is not a ranking weakness to be tuned; it is a missing traversal.

This is the number that justifies giving durable chunks a one-step link
hydration, and it is why the semantic limit is not raised instead: widening
the layer limits would buy a larger lexical net, and the failure above is not a
net-size problem.

#### What was built, and what was not

The repair is a reverse index, not an authored edge. `document_links` records
which documents declare which source — one row per `sources[]` entry, derived
at index time and thrown away with the index — and `linked_documents` walks it
backwards: given a path, every document that cites it. On the same fixture
that measures 0 of 2 lexically, the link route returns **2 of 2**, with nobody
authoring anything.

That number is also why a `related:` field on chunks was *not* built. It would
repair the same case only if a human wrote an edge for every pair, and nothing
in the repository authors chunk edges; the link the two chunks already assert
was sitting unread in their own frontmatter. A `related:` field remains
justifiable only for a relationship that no shared source expresses, and no
such case has been measured. It is recorded here beside the embeddings
negative so the question is not reopened without new evidence.

#### Automatic source-linked knowledge

Every Task Capsule retrieval automatically follows the existing `sources[]`
relations. Users do not select a graph mode, enable a flag or maintain edges.
Native hooks for Claude Code, Codex and Cursor, Memory MCP and Harness use the
same runtime. A first prompt can retrieve before its task is provisioned;
lightweight capsules use the same policy-checked expansion. A hook without a
prompt or substantive task state has no lexical seed to expand.

From at most two strong, allowed lexical semantic matches, the runtime follows
reverse citations or a shared canonical source revision to durable chunks or
dynamic records. A common code file need not be indexed: both documents must
declare it, have matching fingerprints and still match its current digest.
Private/derived paths, symlinks, Windows junctions and sources over 8 MiB cannot establish a shared
relation. The runtime examines at most 32 neighbour rows (duplicates in shared
citations count as one document), considers at most 32 canonical source references
per anchor and checks that bounded pool for each shared neighbour. A covering
index reads each reference in path order with the remaining neighbour allowance;
it does not materialize the product of citations. Current source digests are
cached within the retrieval, and at most three neighbours are proposed. Weak/distinctive seeds
are not expanded. Every anchor and neighbour passes privacy, owner, authority,
lifecycle and source-hash checks. Records with outgoing or incoming conflicts
are excluded from this expansion until whole-component admission can preserve both sides within the
budget. There is no recursive traversal through a neighbour.

Excerpt scoring counts each tokenizer term even when FTS5 groups adjacent
matches into one highlight. Exact section-score ties favour matches in the
body. Oversized sentences are scored on the bounded text actually quoted,
with context before the match, so invisible words cannot win a window that
then cuts the answer away. Document admission and the capsule ceiling stay
under the same policy and budgets.

Direct matches and explicit `--path` items keep priority. A source-linked
promoted chunk cannot suppress its direct source record. Normal budgets,
session novelty and the 3,600-character renderer apply. `no_match` still means
that the lexical query found nothing in that layer. Delivered neighbours carry
`selection: source-link`; manifests record only safe `source_links` counters
(anchors/candidates/delivered), without a mode or withheld neighbour paths/counts.

The relation establishes provenance; it does not prove relevance to every
query. The synthetic comparison includes a relevant source with an irrelevant
citing chunk, so this limitation remains measured rather than hidden.

Maintainers can compare a committed runtime with the automatic working tree
on identical synthetic prompt-time snapshots:

```bash
python3 scripts/memory_graph_pilot.py --out-dir /tmp/source-link-comparison
```

The directory must be new and outside the clone. Baseline defaults to `HEAD`;
`--baseline-ref COMMIT` selects another local version for measurement. This
option belongs to repository evaluation tooling and is not installed into a
project. Baseline files are read as exact Git blobs, while both runs use the
current scorer. Inputs, judgments, future controls and runtime digests are
saved alongside `baseline.json` and `automatic.json`. The comparison measures
source-link delivery, not Neo4j performance or autonomous use of a delivered
fact by an agent.

Two things the index deliberately does not carry:

* **`source_fingerprints`.** `sources_are_fresh` requires the fingerprint path
  set to equal the sources path set, so a fingerprint can never name anything
  `sources` does not already name.
* **A task's `files[]`.** Every live task record in this repository lists
  twenty entries led by phpunit cache, vendored JavaScript and dev container
  dumps, and `changed_paths` writes them relative to the Git toplevel while
  every other path in the index is relative to the repository root. Linking
  them would fill the table with build artifacts that resolve to nothing.

The link is deliberately derived rather than durable. Carrying the same edge
on the chunk would put it under `chunk_source_digests` and `validate_metadata`,
where every edit to the referenced file would mark the chunk as changed since
verification and deleting it becomes a permanent bank-validation error
reachable from `apply_promotion`, `compact` and `validate`.

#### Written knowledge stays

A resolved finding, a closed incident or an accepted decision leaves the
index as a record: from then on its chunk is what retrieval serves. Two things
used to make that hand-over fail for good, and on a real project only 2 of 70
resolved findings ever reached the Memory Bank:

* **Promotion waited for the turn boundary**, up to five turns after the
  resolving update. The fix that resolved a finding usually edits the file the
  finding cites, so by then the citation no longer matched what was verified
  and promotion refused it. `brain-update` now promotes the record it resolves
  at once (`promote_on_resolution`), while its citations still say what was
  checked; a failure is reported and never undoes the update.
* **Any edit to a cited file evicted the knowledge.** A chunk or record whose
  cited file changed after verification now stays in retrieval, marked
  `source_changed` - the capsule says "cited file changed since this was
  verified" next to it - and ranked at half its score below fresh knowledge.
  Only a deleted citation (`source-missing`) takes it out. Automatic promotion
  of a record whose citation changed proceeds, and the chunk keeps the digests
  taken at verification rather than laundering the edit into a fresh citation.

`validate` lists such records as warnings instead of failing the project, and
the turns a branch keeps working after its task was completed are dropped with
a note instead of failing every flush, which had stopped promotion and
compaction as well.

#### Promotion carries the citation through

A promoted chunk records the records it was promoted from **and what those
records themselves cited**. Without the second the chain breaks exactly where
it matters: the chunk names the record, the record names the document, and
`links` is one hop by design, so a query on the document reaches the record
and never the knowledge derived from it.

This was found on a real installation rather than in a fixture. Two findings
about one design document promoted into two chunks that shared no source at
all — the shape the fixture hand-wrote is one the automatic pipeline never
produced. With inheritance the same two chunks share the document, and
`links --path` on it returns both.

Two limits on what is carried:

* **Only `sources`, never `files`.** A record's `files` is Git churn in the
  Git-toplevel frame; merging it would put code paths under
  `chunk_source_digests`, where the next edit to any of them marks the chunk
  as changed since verification.
* **Only citations whose file still exists.** A file deleted between review
  and apply would otherwise produce a chunk that fails `validate_metadata` at
  birth, failing a promotion that has nothing to do with that file.

An inherited citation is a full citation: it is digested, and an edit to the
document re-opens the chunk for `bank-reverify` the same way an edit to the
record does. That is the intended coupling — durable knowledge derived from a
document should not outlive a change to it silently.

#### Reaching it: `links` and `retrieve --path`

    python3 memory-bank/scripts/context.py links --path specs/rounding.md
    python3 memory-bank/scripts/context.py links --path specs --prefix
    python3 memory-bank/scripts/context.py retrieve "..." --task-id T --path specs/rounding.md

`links` reports the citations; `retrieve --path` delivers them in the same
capsule, budget and manifest as everything else. Both apply the full runtime
filter — privacy, owner, authority, lifecycle and freshness. That is not
incidental: `search_documents` deliberately does not join `document_metadata`,
and a `links` modelled on `search` would have republished restricted records
the way `search` once did.

Three properties worth knowing when reading a manifest:

* A path-linked item carries `selection: "path-link"` and **no** `match`.
  `match` says how the query matched, and nothing lexical selected these.
  They also carry no `rank`, and score zero, like a conflict partner.
* Path-linked items **lead** their layer, capped at `PATH_LINK_LIMIT`. Naming
  a path is a claim that these documents matter to this turn; appending them
  would let three query matches evict them and reproduce the 0-of-2 failure.
* A layer can report `no-match` and still deliver a path-linked document on
  the same turn. `no_match` is computed before injection and is a claim about
  the *query* — the capsule line and `gate.signals.no_match` both read it that
  way. The two answer different questions: whether the caller's words found
  anything there, and whether the caller's path did.

`--path`, MCP `paths` and the automatic source-linked expansion read citations
only (`source` rows); `links` also lists active tasks that touched the path
(`touched <path>`).

#### Files the work touched (2026-10-10)

The source links above had nothing to stand on: on the 121 evaluated prompts
the link table was empty in every snapshot, because no indexed record carried a
source. What records do carry is `files[]`: the checkpoint writes the files a
task's turns changed, and 12 of the 14 answers that tasks held on the evaluated
prompts were entries of that list. A task's `files[]` now become `file` rows:

- **Which files.** Only paths Git's index holds as files (one `git cat-file
  --batch-check` per change of the tasks' files, cached in the index state, so
  never per prompt; outside a checkout no row is written), project-relative,
  and not sensitive, runtime state, build output, a lockfile, a directory or
  secret-shaped. Only active tasks: completed ones are not indexed.
- **Who reads them.** An automatic channel, on `refresh` (hooks, Harness, MCP
  `memory_retrieve`) and `hook-context`, seeded by paths the request names -
  written out, as a PHP class through `composer.json`'s PSR-4 map, or as a bare
  file or class name that some link names - and by the files this branch
  touched on its last turns and its task's newest `files[]`. A seed reaches the
  documents that cite it and the other active tasks that touched it. Explicit
  retrieval stays deterministic and does not seed.
- **Restraint.** One item per capsule (`FILE_LINK_LIMIT`), placed after the
  strong lexical matches inside the three semantic slots; a weak match a link
  confirms moves there. A file linked from more than five documents is a hub
  and reaches nothing. A document reached only through touched files must also
  share an informative query term: file sharing is dense on real projects (on
  two installations every active task shared a file with another), and
  "continue" must not hand over a sibling task. The current task's own record
  and handoff are never candidates.
- **What it shows.** The item is marked `prompt-link` or `touch-link` in the
  manifest (no `match`, no rank) and `linked through <path>` in the capsule; a
  linked task quotes its goal and its files, the shared one first. Under the
  capsule's 8,000- and 3,600-character limits a linked item (like a
  `source-link`) is the first to give its place back, before history.
- **Not done.** Edges through commits (record and commit, files changed
  together) wait until live link density is measured: they add a second hop
  over edges that are still sparse.

The evaluation stand runs no hook and has no working task, so touched-file
seeds are not measured there; their go/no-go is prospective - live `file` rows
per project, the share of turns with a `prompt-link` or `touch-link`, and
labels of what they delivered.

### Skill pointers in the procedural layer: a measured negative

A separate idea was tested here and rejected on measurement, and the result is
recorded for the same reason the embeddings result is: so nobody spends the
effort again without new evidence.

The observation is real. Nineteen of twenty measured procedural slots hold
pointers to a `SKILL.md` whose name and `description` are already paid for at
the start of every session, and some of those pointers are plainly wrong. The
proposed remedy was to admit a skill document into the procedural layer only
by the `distinctive` route — on a rare term — rather than by covering the
query.

Both readings of that rule were simulated against real indexes and both fail.
Gating on the item's own match strength drops the *right* skill and keeps the
wrong one: "map the codebase" loses `codebase-mapper` and retains `researcher`.
Gating on membership of the distinctive-path set keeps the showcase failures
the proposal cited. The cause is that a skill's `description` is indexed as the
`summary` column at weight 8, so a correct and an incorrect skill match arrive
through exactly the same channel — no filter over that channel can separate
them.

What the measurement does support is that the procedural layer routes badly:
on real requests an acceptable skill reaches the top two only 10 times out of
16 on Symfony. That number is now a regression floor in
`project-brain/tests/fixtures/skill-routing-golden.json`, and
`retrieval-report` reports the match-strength distribution and the most-selected
paths over real turns. A remedy should be built when those two say what it
should be — not before.

**2026-10-07: one procedural slot, strong matches only.** New evidence decided
it. Agents opened 0 of 148 skill pointers they were handed on real
installations (every host loads its own skill catalogue and picks from it),
and on 61 real first prompts with graded relevance a skill admitted on one rare
word was useful once in 37 deliveries. The capsule now carries at most one
procedural item and never a `distinctive` one. On those 61 prompts turns with a
useful item rose from 29 to 33 and noise-only turns fell from 16 to 7 (against
the same runtime with two slots). The price is the bench's skill-routing view:
an acceptable skill reaches the capsule on Symfony golden-en .59 instead of
.71, because the second slot is gone; the ranking itself (hit@1, hit@2) is
unchanged, and so is the routing floor, which measures ranking.

**2026-10-10: a skill's sub-file never holds the procedural slot.** Files a
`SKILL.md` sends the agent to - `references/`, `agents/`, `rules/`, an
`AGENTS.md` inside a skill, a note at the tree's root such as `SKILL FLOW.md` -
were useful 1 time in 185 judgments on 121 graded prompts, against 71 in 627
for `SKILL.md`. When one heads the procedural ranking the slot stays empty and
the manifest records `skill-subfile`; it is not refilled, because on the 19
replayed turns where a sub-file held the slot the next skill down was useful on
none (5 noise, 6 unjudged). The rule is structural - the file's place in its
tree - not a score threshold. The sub-files stay indexed: `search` returns
them. On a vacated turn `no-match` still reports that the procedural layer
matched, as it does for a skill admitted on one rare term.

**2026-10-10: the capsule carries no skill.** Measured in the real sessions of
the five evaluated projects since January: on the 71 Claude Code turns where a
capsule named an accelerator skill, the agent made no Skill call and read no
`SKILL.md` at all; accelerator skills were used on 24 of 2,265 Claude turns
(1.1%), and on Codex mostly because the user typed `$code-reviewer`. Of the 35
graded core prompts with a useful skill, the agent used any accelerator skill
on 4. Every host lists its skills and loads its instruction files itself, so
the procedural line only took room from the excerpts that carry answers. The
procedural ranking still runs: its pick is recorded in the manifest as
`host-listed` (a sub-file pick as `skill-subfile`), `no_match` still reports
the layer, and `search` returns skills. The evaluation stand reports skill
routing apart from project knowledge (`skill_*`, `knowledge_*`).

### The retrieval gate

Restraint above was all about the document: which files are relevant enough to
occupy a slot. It cannot express the other question — whether this turn needed
retrieval at all. A pointer that says "relevant right now" about the wrong file
costs more than no pointer, because the agent opens it; the saving is bias, not
tokens, and the work the gate skips is trivially cheap either way.

`gate_decision` records a verdict and one deterministic reason on every governed
retrieval:

- `no-relevant-match` skips when neither indexed context nor a local episode
  matched;
- `empty-after-filter` skips when something matched but nothing remained
  deliverable;
- `repeat-retrieval` skips an unchanged query, selection and task revision;
- `not-held-by-conversation` retrieves what would be a repeat for a turn of a
  conversation (a host-supplied session ID): its own delivery record has
  already left out what it holds, so it does not hold what remains - another
  conversation was handed it, or this one lost it to a compaction or the
  novelty window;
- `task-changed` retrieves when the query and selection match but the task
  revision moved;
- `new-selection` retrieves every other non-empty result;
- `gate-off` retrieves without making a skip decision.

A repeat baseline is scoped to the task UUID, host, and entry point. Its
selection signature contains each selected path and source-content hash, any
selected local-episode content hash, and the task revision. Changing a source
without changing its path, changing a local episode, or advancing the task
therefore cannot be mistaken for a repeat from another client or lifecycle
state. The bounded baseline lives in the disposable index, and a skip never
overwrites it — otherwise the turn after a skip would compare against nothing.
The baseline is shared by every conversation of a task, so it decides repeats
only for a caller without a session ID; within a conversation the conversation's
record of what it was handed is the one notion of "seen", and a turn whose
every item it holds skips as `empty-after-filter`.

Participating retrieval writers update the baseline and conversation receipts
in one short SQLite transaction after packing. They merge unrelated entries
from the current maps and reject an outdated snapshot of their own target,
including a turn prepared before a compaction reset. Read-only, busy or failed
bookkeeping leaves ordinary retrieval available; a later turn may repeat an
item. A programmatic caller's existing transaction remains theirs to commit or
roll back. This disposable record is not an acknowledgment that an outer hook,
MCP client or model consumed the response, and it does not authorize extra
excerpt delivery. Old running versions and generic index-state writes do not
participate in this transaction contract.

The mode comes from `--gate`, then `CONTEXT_RETRIEVAL_GATE`, then
`retrieval_gate` in `runtime.json`, and defaults to `shadow`. In `shadow` the
decision is computed, recorded and ignored: the turn is served either way. That
is deliberate and is the same standard that kept embeddings out — the mechanism
is built and observed before it is trusted, and switching it on is H3-05's
decision, taken on the report `shadow` produces. `off` never decides. In
`enforce` a skip withholds the selection, prints `gate: skipped — <reason>`
after the `working:` marker, and still writes a manifest, because a withheld
turn has to be countable. On the Cursor path a skip exits with a status the
delivery hooks leave alone: rendering a withheld capsule would satisfy their
`working:` check and replace Cursor's only memory channel with an empty rule.

What the automatic capsule carries is a separate setting:
`CONTEXT_CAPSULE_PROFILE`, then `capsule_profile` in `runtime.json`, defaulting
to `full`. The four editions and the generator's template ship `recall`; a
project whose `runtime.json` names no profile keeps `full`.

- **`full`:** the established capsule — working state, recent files and the
  retrieved semantic and episodic documents.
- **`navigation`:** the working state and the recent files only. It searches
  nothing and records no delivery.
- **`recall`:** navigation plus what the task's own saves recorded, and
  nothing else. The request is searched only among those documents; without
  one of them indexed, the capsule is navigation's. The task's own saves are
  the findings and decisions:
  - whose name is the one a result of this task gives a learning
    (`record-result`, `native-result`, MCP `memory_record_result`);
  - or whose name starts with the task's external ID or a ticket it names
    (a Harness draft `<task>-finding-<hex>`, a finding an agent filed as
    `CMS-739-review-4` on `feature/CMS-739-…`).

  A saved finding is resolved as it is written and no resolved finding is
  indexed, so what the capsule finds is the Memory Bank chunk it was promoted
  to: a chunk counts when a record it cites is one of these, before or after
  compaction archives the record. An open finding or an accepted decision
  counts itself. Recorded history, path and file links, related knowledge
  and source hints are left out; a conflict partner is still delivered.
  Lightweight mode has no governed saves, so there `recall` is navigation.

The profile applies to the automatic entry points (`refresh`, which the Codex
and Cursor hooks run, and Claude's `hook-context`). An explicit
`context.py retrieve` always searches. Any other value leaves the turn without
a capsule and says why, as a broken index does.

`navigation` is an opt-in profile. On 30 client tasks it reached the file a
task changed in fewer calls than the full capsule (`docs/MEMORY-EVAL.md`,
"Measuring task outcomes"). It also turns off everything the hook delivers
through the document layers:
- a result saved in one session is no longer recalled by the next session's
  capsule;
- source hints are not delivered;
- the Cursor rule carries no documents.

An agent can still pull all of these with `context.py retrieve` or the MCP
`memory_retrieve` tool. `recall` keeps the first of them - a session's saved
result in the next session's capsule - and drops the other two.

Measured against both (`docs/MEMORY-EVAL.md`, "Measuring task outcomes"):
- on questions about a saved result, `recall` was as good as `full`;
- on cold starts it is navigation, byte for byte.

That is why the editions ship it. Two things held it back and are fixed:
- the Harness's automatic memory required a manifest, which a capsule
  without documents does not write; such a capsule now needs no source
  fingerprints, and a proof that does arrive is checked as before;
- a promoted chunk was quoted by its `Sources` footer, which `full` hid by
  delivering the cited file beside it; the excerpt now skips a citation-only
  footer whatever selected the chunk.

The Harness's explicit `refresh` and the LongMemEval bridge ask for `full`,
as MCP `memory_retrieve` does: they are requests for a search, not the
automatic capsule. A team that wants the old capsule sets `capsule_profile`
to `full`.

`retrieval-report` keeps shadow `would_skip` distinct from context actually
`withheld` in `enforce`. It slices gate results by `mode`, `query_source`,
`host`, and `entry_point`, and separately aggregates `no_match` layers and
`token_estimates`, so client-specific or query-specific failures are not hidden
in one skip rate.

Whatever the reason a document leaves, `index` names it rather than dropping
it silently — `retired`, `overdue-review`, a non-active status, `invalid`, or
`secret` (a chunk with a likely secret, or a document whose masking did not
converge) — so a red validator and a quietly shrinking index can no longer
disagree about the same chunk.

## Search and Governed Retrieval

Capsule assembly filters for relevance before ranking. Terms matched by more
than half the corpus are dropped as noise — measured against the index rather
than a stopword list, so it adapts to the languages a repository documents
itself in. A document then qualifies either by containing two distinct query
terms, or by containing one term rare enough in this corpus to be evidence on
its own - provided that term names something: it is shaped like an identifier
(a digit, `CamelCase`, `snake_case`: a ticket or version number, a class or
constant name) or it is a word of the document's own path or title.

That second route matters more than it looks. Counting terms equally punishes
exactly the wrong document: a focused note containing only the rare term that
matters scores one, while filler sharing two unremarkable words scores two and
takes the slot. Admitting a distinctive single match lets some noise back in on
queries no document covers, which is the cheaper error — a spurious result
wastes a slot, a hidden one denies an answer the project already holds. The
anchor condition came from measurement: in an index of about a hundred
documents a word in ten of them is "rare", and on 61 real first prompts the
single-word matches without an anchor were noise in six of seven judged cases.
Requiring the anchor removed 14 of 186 delivered items there without losing a
useful one.

Measurements like these are only honest on each project as it was when the
prompt was written: an index of the project's current state lets knowledge
written after the prompt, often about its own work, answer it. The
[Memory Evaluation Stand](MEMORY-EVAL.md) (`scripts/memory_eval.py`) rebuilds
that state, overlays the runtime under test and scores the capsule against
judged prompts.

### What a capsule quotes

A delivered project-knowledge item carries text, not just a pointer, and the
text is chosen to hold the answer:

- The document's matches are marked by FTS5 `highlight()` with the index's own
  tokenizer, so a request about a "session" finds the section that says
  "Sessions" - the words that selected the document choose its excerpt.
- Each matched term counts by its rarity in the index. A real prompt shares a
  dozen common words with every long section; the entry naming the ticket or
  the component is the one worth quoting. A marked word takes the weight of
  the request word it matched: both are read as the tokenizer reads them
  (`term_forms`), so "classes" and "class", or "policy" and "policies", are
  one term.
- The section with the most weight is chosen, and within it a window of whole
  sentences that starts where the matches are - a changelog entry runs to a
  kilobyte, and its answer is usually the sentence after the one that matched.
  A list item keeps the line that introduces it. Elided text shows as `…`.
- Linked knowledge skips generated `Sources`/`References` sections made only
  of declared citation paths when another section contains prose. Explicit
  requests for those headings, annotated references and source-only records
  retain their content. The remaining sections keep the same weighted window
  selection. Headings receive the excerpt body's privacy screen.
- Windows are 800, 600 and 400 characters by rank, 600 for a history item; the
  rendered capsule stays under 3,600.
- The item line names the section: `- memory specs/auth.md § Sessions — Auth`.

On the 61 graded prompts, with the passages that carry each useful document's
value labelled, the delivered text held such a passage for 20 of 36 delivered
useful documents instead of 12, in 14 turns instead of 11.

A conversation is not handed the same thing twice within four turns
(`refresh --session-id`), and "the same thing" is the section an excerpt came
from at its revision: a later question answered by another section of a
document already handed gets that section. A recorded episode is the same
thing while its id and content are. A host that compacts a conversation
records it in the transcript the hook is given (`--transcript`): Claude Code
writes a `compact_boundary` record, Codex a `compacted` one. After one, nothing
counts as handed any more, because the conversation holds a summary of it.
Claude Code writes a new conversation's transcript only after the first prompt
hook, so a transcript not written yet counts from its start: a compaction
before the second turn is seen too.
Only what the capsule shows counts as handed: an item its rendered text had no
room for is excluded from the manifest as `capsule-limit` and is handed on a
later turn.

Automatic linked tasks still quote their goal and files. Other automatic
links retain whole-document suppression unless the citation-footer correction
changes their quoted section; those corrected excerpts use the new section
identity so an existing session receives the substantive rule immediately.

Session retrieval also records a bounded, metadata-only observation in the
ignored local retrieval trace. It compares the sanitized window proposed
before novelty and packing with the previous final renderer body for the exact
same section/document identity and revision. `same-window` and
`different-exposure-window` describe bytes, while `unknown-prior-render`
preserves uncertainty for legacy, missing, pruned or unfinished traces.
Empty proposals, prior pointer-only bodies, prior shrinking and whole-source
changes remain distinguishable. No raw quote, query or new heading is stored
in the session companion: it points to an existing trace UUID using bounded
hashes and the original first-delivery turn. Repeats keep that pointer;
expiry, compaction and session eviction prune it with the existing receipt.
Diagnostic failures cannot release a repeat or change a capsule or gate.

A differing fingerprint is an exposure signal, not evidence that an answer
was lost. Rank allowances or previous render shrinking can also change it.
The prior trace records final renderer bytes at the local emitter boundary;
it does not certify stdout completion, host acceptance or model use. Existing
privacy/owner gates and secure trace retention apply, and local episode
windows are outside this document observation denominator. Hosts without
secure descriptor-relative trace I/O retain normal retrieval without a prior
render baseline. The four-turn novelty rule itself is unchanged.

Both admissions used to arrive looking identical, so the capsule names which
one applied. A document that carried the required number of distinct query
terms is `covered`; one admitted on rarity alone is `distinctive` and renders
as `weak-match: <path>`. `covered` is the common verdict and is carried by its
absence in the capsule — the capsule is zero-sum against its character ceiling
and each selected item is serialized more than once — while the manifest
records the verdict for every selected item in full. A conflict partner is
pulled in by record id rather than by the query and is recorded as `conflict`.

A capsule also says when nothing matched. `no-match: <layers>` lists the
layers in which no candidate passed the relevance test, and is computed before
any privacy, authority, lifecycle, budget or layer filter runs — a layer
emptied by one of those is not a layer memory had nothing for, and the
manifest's `excluded` entries say which it was. In the manifest the same
distinction reads as an empty `selected` with an empty `excluded`. Without
this a capsule that found nothing and a capsule that was never consulted
render identically, which is what makes "refuse when there is no data" a rule
the model can actually follow rather than a request to observe something it
was never shown.

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
3. uses FTS5/BM25 to identify candidates and loads matching local episodes
   before the retrieval gate;
4. applies runtime privacy, owner, authority, lifecycle, and freshness filters;
5. fetches eligible linked conflict partners by UUID even if they did not
   lexically match;
6. selects bounded snippets and any remaining episodic slot within category
   and total estimated-token budgets;
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

Confidence is a number from `0` to `1`. Governed retrieval ranks with it: each
candidate's BM25 relevance is scaled by its authority (`verified` outranks
`observed`), its declared confidence, and a recency decay over the record's
`updated_at`. Every multiplier is floored, so metadata reorders results but
never hides a lexical match, and repository documents without a provenance
timestamp are not decayed. Ranking is the only effect — no threshold excludes
a low-confidence record automatically, so do not promise that one will be
filtered out.

Freshness has three checks:

- the indexed document's current hash must match the hash stored when indexed;
- for Brain records, handoffs and chunks, a cited source that no longer exists
  excludes the document; one whose digest moved on keeps it, marked
  `source_changed` and ranked at `SOURCE_CHANGED_WEIGHT`;
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

Closing a period is an operation, not an editing convention: `context.py
bank-retire --id MEM-... --valid-to YYYY-MM-DD [--superseded-by MEM-...]`
writes the chunk, the other side of any replacement link, and the regenerated
index under one lock, validates the bank, and restores every touched file if
anything fails. Doing it by hand leaves the bank invalid between the first
edit and the last.

Every bank write — promotion, compaction, `bank-reverify`, `bank-retire` — is
refused for what it introduces, judged against the bank as it stood before
the write, and for any problem in the chunk it produces or attests. It is not
refused for a problem the bank already had. Validating the whole bank and
rolling back on any error made time alone a write lock: one chunk passing its
review date failed every promotion and compaction in the repository, and two
overdue chunks each blocked the `bank-reverify` that would have repaired the
other, leaving hand-editing as the only way out. A lapsed chunk is reported
where lapses belong instead — `validate.py`, `bank-audit`, the `overdue-review`
retrieval exclusion, and a `memory review:` line in every `refresh`.

An active chunk may not sit past its `valid_to`, the same rule `review_after`
already applies. Closing a period therefore removes the chunk from retrieval
without deleting it: automatically written memory earns a boundary rather than
an erasure, and what was believed during that period stays readable. The
boundary is enforced on both indexing paths, not only the full rebuild — the
incremental cache keys on it as well as on the stat pair, so the removal
happens on the date itself rather than on the next time someone edits the
file.

## Budgets and Snippets

Governed retrieval estimates tokens as roughly one token per four characters.
This is a deterministic safety budget, not a tokenizer-accurate provider usage
measurement.

The internal category limits are policy 1,200, handoff 1,500, durable 3,500,
dynamic 1,500, and evidence 2,000 estimated tokens. Candidate selection has an
8,000-token target and a 12,000-token conflict ceiling. After ranking and
policy filtering, the delivered capsule is independently capped at 3 semantic
and 3 episodic items (the changelog and up to two recorded events, or one
local episode) and
8,000 serialized characters; it carries no procedural item. Snippets are
deterministically shortened as needed.

A selected local episode shares the recorded-history slot with events. Its estimate is
included in `token_estimates.local_episodes` and in the total ceiling even
though its identity and content are deliberately absent from the manifest.

These controls bound selected retrieval context, not the size of source files,
Brain records, user prompts, or model responses. Conflict escalation can exceed
normal category/target budgets, never the hard ceiling.

## Retrieval Manifests

Every successful governed retrieval writes a manifest. By default it lands in
the Git-tracked `project-brain/control/retrieval-manifests/<uuid>.json`.
The writer emits strict schema version 4; validators continue to accept valid
version 1, 2 and 3 manifests already present in a consuming repository.

`--ephemeral` writes the same validated record to
`memory-bank/local/retrieval-manifests/<uuid>.json` instead. Automated
retrieval must use it: one manifest per request would otherwise add thousands
of files to shared history. Local manifests are ignored, are provenance for the
local machine only, and are capped by `local_manifest_retention` in
`project-brain/config/runtime.json` (default 200, floored at 1). That window is
the observation window for anything measured from manifests, and a withheld
turn writes one too — so a high skip rate shortens the history in proportion.

A manifest binds:

- creation time and the privacy-checked, distilled retrieval query;
- Brain task UUID and revision;
- host and entry point;
- privacy, owner, authority, freshness, and active-only filters;
- selected paths, categories, estimated tokens, source hashes, and how
  each one matched (`covered`, `distinctive`, or `conflict`);
- `local_episode_count` and `token_estimates.local_episodes` for selected local
  episodes;
- excluded paths and safe reasons;
- the retrieval gate's verdict for the turn: `decision`, `mode`, `reason`, and
  the signals it decided on;
- provider;
- target/hard estimates and any conflict escalation reason.

The manifest is provenance metadata, not a context snapshot. It does not retain
the selected snippets, full source bodies, or local episode IDs and bodies, and
it cannot replay the exact prompt seen by an agent. It deliberately excludes
prompts, responses, hidden reasoning, and tool payloads.

Governed manifests are Git-trackable and validated, but the current compactor
does not archive or prune them. Retention for those requires an explicit
repository policy. Only the local `--ephemeral` store prunes itself.

### Local delivery diagnostics

Governed retrieval also writes an ignored metadata-only sidecar at
`memory-bank/local/retrieval-traces/<manifest-uuid>.json`. It observes the
existing candidate windows, admission, ranking, runtime filters, linked
records, budget/layer limits, gate and capsule packing. A watched indexed
record absent from those windows stays `not_observed_in_bounded_search`:
its exclusion cause is unknown. The existing evidence-term coverage helps
distinguish a lexical miss from an unobserved bounded search. Trace ranks
count authorized records within the observed window; they omit protected
positions and are not the manifest's lexical ranks.

The trace's additive `funnel` child has its own version 1 schema. It records
fixed authorized observation counts before the per-record cap, plus observed
queue, budget, layer, repeat, gate and pack boundaries. Counts describe
occurrences, not distinct answering documents; saturation is a lower bound.
Path-only exclusions and render counts cover retained authorized records.
Uninstrumented channels and source absence remain unknown, so zero counts
cannot justify hybrid search or prove an answer is missing. Render projection
is separate from unknown emission and consumer states. Collection is automatic
and needs no watch list. If this child alone exceeds the file cap, it is omitted
while the original trace and rendered-window metadata remain available.

`refresh` automatically retains a fixed `funnel_diagnosis` child in its
existing local health record and additive JSON result, outside the capsule.
The reader binds only the current manifest and exact render form, byte count
and hash, through anchored same-owner metadata handles and one complete bounded
read. Missing, pruned, old, malformed, unsupported, mismatched or omitted
evidence has the same unavailable state with null counters. Plain output uses
its final printed form; a failed finalizer cannot reuse the earlier JSON form.
Counts remain occurrences or lower bounds, with retained-record render scope;
quality is unmeasured and source coverage, emission and consumer use stay
unknown. Positive observed loss means only `inspect-observed-loss`; it does
not establish loss of a needed fact or recommend a model.

`health --json` adds a fixed diagnosis summary over the last 500 records after
the existing `--window` selection. It partitions observed, unavailable, missing
and rejected metadata. Each counter reports known, unknown and saturated
contributors plus the sum of known counts. A sum with unknown or saturated
contributors is a lower bound; an entirely unknown sum is null. This cap
limits the new aggregation, not the existing health-file reader. Legacy JSON
fields and plain health output retain their behavior.

The evaluator keeps valid diagnosis under each item's `refresh` metadata.
Harness keeps it beside a usable prepared capsule in the session Brain record,
including across restarts, and clears it when that prepared context is
invalidated. Freshness checks do not replace the saved invocation observation.
Missing or rejected metadata is omitted; valid unavailable metadata remains
explicit. Unexpected metadata-validator errors do not discard usable context
or change evaluation scores. These observations are outside capsules, model
prompts, approval material, token meters and score/report calculations.

The same automatic refresh observation carries a separate `ordinary_receipt`
child with exactly `schema_version`, `observation_state`, `plans_present` and
`returned_status`. Observed records report the paired owner's returned status:
no plans can report only `skipped`; planned calls can report `skipped`,
`deferred` or `committed`. Unavailable observations use null plans and
`unknown` status. Missing or invalid evidence is unknown, not a no-plan call.
The status does not identify a refusal cause, confirm durable effects, or
acknowledge stdout or host consumption. Standalone legacy remember helpers
are outside this observation.

The child uses the same anchored current-render read, independently of funnel
validation. At the trace size cap it is omitted first, before the existing
funnel fallback. `health --json` summarizes the selected window's last 500
records in eight fields: `summary_schema_version`, `records_considered`,
`observed_records`, `unavailable_records`, `no_plan_skipped_records`,
`planned_skipped_records`, `planned_deferred_records` and
`planned_committed_records`. The four buckets sum to observed records;
observed plus unavailable equals considered records. These count historical
records, not attempts or unique deliveries. Missing, malformed and unavailable
children contribute only to unavailable records. Plain health and capsule
output remain unchanged. The evaluator and Harness retain this sibling
independently, with the same content exclusions and prepared-context lifetime.

`retrieve`, `context` and `refresh` accept repeatable `--trace-path PATH`
(at most 16 safe project-relative document paths). This observes a record;
it does not seed search or bypass privacy/owner filters. Missing and protected
watched records have the same `unavailable` state. Trace files retain no
prompt, title, body, snippet, citation lists or owner identities.

After an actual render, the sidecar records section kinds, body presence,
sanitized excerpt sizes, omission reasons and hashes. `render.form` identifies
`capsule_text` (the JSON field, with no added newline) or `printed_capsule`
(the text capsule, including its final newline). These hashes cover the
capsule; separate refresh status lines are outside it. Structured-only JSON
retrieval has no render claim. Delivery metadata does not prove model attention
or answer use.

Each trace is capped at 128 records, 16 events per record, 512 characters per
path and 512 KiB. Truncation is explicit. Trace-file retention independently
uses `local_manifest_retention`, keeping the current trace even after clock
rollback. Finalization does not revive pruned files. Diagnostics fail open:
missing/corrupt trace storage leaves ordinary retrieval, capsule text, exit
status and repetition state unchanged. Hosts without secure descriptor-relative
filesystem operations skip these optional sidecars; the memory runtime still
works. Capsule and manifest schemas are unchanged.

## Handoffs


A governed task handoff is a compact continuation record containing objective,
current state, next actions, files, sources, owner/privacy, task revision, and
active/closed status.

The runtime creates it with the task, refreshes it on supported task updates,
closes it on completion/cancellation, and archives it with its terminal task
during compaction.

A task may declare a `phase`: `understanding`, `planning`, `implementation`,
`verification`, or `finalization`. The compatibility inputs `implementing` and
`execution` normalize to `implementation`; `review` normalizes to
`verification`. Only canonical values reach records, handoffs, retrieval
output, and indexes. Progress says what was touched; the phase says which step
of the loop the work stopped on.

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
unattended on the same boundary as the working-memory flush. With
`automatic_consolidation` (off unless a project sets it), that boundary first
writes a ticket's completed-event series into one finding that carries every
event's outcome; the events stay recorded until a chunk carries that finding
(see `consolidate` in [OPERATIONS.md](OPERATIONS.md)):

```text
propose -> apply
```

`context.py promote-auto` runs the same pass on demand. The boundary waits for
`--flush-after` turns, and knowledge recorded deliberately — what a Harness run
drafted, saved when the run completes or after a person confirmed each
learning — has no reason to wait for a counter. A learning the Harness saved
unattended is written as observed and raised to verified with the reason
"agent-attested, not reviewed by a person", so its own ledger says who
attested it before promotion tags the chunk `auto-promoted`.

The commands an agent verifies with say who checked too. `brain-create
--authority verified` and `brain-update --authority verified` (or
`--reverify`, a verified claim checked again) record `agent` unless
`--attestation person` says a person did; until 2026-10-30 they recorded
nobody, and a record so verified carries no mark. The Harness passes `person`
for what a person does on its pages, where the runtime takes the flag. A
verified finding, bug, incident or decision keeps the source fingerprints
taken when it was checked through every write that checks nothing - progress,
next steps, a lifecycle move - and only a source cited for the first time is
fingerprinted then; retrieval and promotion compare the checked version with
the file now. Before, any such write took them anew, so an edit cleared
"cited file changed" from a claim nobody checked again. A person's verified
claim is revised only with `--attestation person`, as `record-result` holds
it.

There is no reviewer in the automatic mode, and the runtime refuses to pretend
otherwise. `reviewer` stays null, `review_mode` is `automatic`, the outcome is
recorded as `approved-without-review`, and the resulting chunk carries an
`auto-promoted` tag so the Memory Bank itself shows which knowledge no human
approved. `promote-review` rejects an automatic promotion outright rather than
letting a human signature be attached after the fact.

Eligibility is deliberately narrow and identical in reviewed and automatic
modes, because an applied chunk becomes durable shared knowledge:

- only `finding: resolved`, `bug: resolved`, `incident: closed`, and
  `decision: accepted`;
- only `verified` authority, because review cannot turn an unverified claim
  into a durable fact;
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
the proposer cannot review their own proposal. Apply rechecks the same type,
lifecycle, authority, privacy, freshness, content, and source-binding rules in
both modes, so a task, reopened record, changed revision, or newly ineligible
source cannot be promoted through an older proposal. Application sets the
proposal status to applied while preserving the outcome: `approved` for
reviewed promotion and `approved-without-review` for automatic promotion.

Application mints a conflict-free Memory Bank ID (`MEM-YYYYMMDD-xxxxxxxx`,
the promotion date plus eight hex characters of the source record's UUID),
writes a chunk, regenerates the index from chunk frontmatter, validates what
the write changed — the new chunk in full, and the bank for anything newly
broken — and updates the proposal. These writes are snapshotted and rolled back
together on failure. No shared counter is involved: the legacy
`.memory-counter` file is neither read nor written, so concurrent promotions
on different machines or branches cannot collide, and `context.py
reindex-bank` rebuilds the index after any merge.

Promotion is appropriate for a reusable consequence, not:

- active progress or a handoff;
- an unresolved finding or conflict;
- raw evidence or incident payloads;
- generic framework knowledge;
- private/restricted or sensitive material;
- content already owned by a living specification;
- a local episode with no eligible governed source.

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

Both validations leave out source freshness. A record whose cited file changed
after it was written is stale - retrieval serves it marked for checking and
`validate` lists it as a warning - but moving terminal records can neither cause
nor cure it. With freshness in, the first
edit to any file a record cited — the living spec behind an accepted decision,
the code a finding was about — refused every later compaction, manual or
automatic.

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
index, use the same authorized owner, and verify canonical sources. Restore a
missing governed binding explicitly with `rebind`, or allow the next automated
turn flush to reconnect it, as described in the
[operations runbook](OPERATIONS.md#missing-governed-task-binding).

If work must survive without special recovery, do not leave its only meaningful
state in a lightweight task or local episode. Capture it in the appropriate
governed record, task document, living specification, or durable Memory Bank
chunk whose reviewed or automatic provenance is acceptable for the environment.

## Hooks and Explicit Actions

Session-start hooks can report metadata such as:

- branch and uncommitted-change count. Changes that are the accelerator's own
  files are counted apart: its install, untouched since the install record or
  the generator's manifest, and the memory the runtime writes. Without that,
  a project that neither commits nor ignores the accelerator reads as holding
  uncommitted work in every session;
- dependency/runtime markers;
- selected context mode;
- index existence and approximate staleness;
- active local binding count;
- validation status;
- Memory Bank availability.

Session-start hooks print no record bodies and inject no context. The one
exception is `context-continuity.sh`, which delivers a chat merge prepared
for a new task with `context-load merge` and otherwise prints nothing; it
also keeps each chat's visible text in ignored `.context-handoff/` on prompt
and end of turn. See [Context Handoff](CONTEXT-HANDOFF.md).

Two further hooks automate working memory when the edition enables them:

```text
UserPromptSubmit -> working-memory-read.sh   (read: refresh + Task Capsule)
Stop             -> working-memory-write.sh  (write: buffer + bounded flush)
```

The split is deliberate. At prompt time nothing has happened yet, so there is
no delta to record; a request is the right moment to *read*. The delta exists
at the end of a turn, which is the right moment to *write*.

Cursor cannot add context to a prompt from a hook, so its read half arrives
through the `alwaysApply` rule `.cursor/rules/working-memory.mdc`: the Cursor
mirror of `working-memory-read.sh` runs on `beforeSubmitPrompt`, renders the
capsule for the prompt into the rule and lets the prompt through; the
`sessionStart` mirror renders the branch's capsule, and the `stop` mirror does
too unless the rule holds the prompt's capsule for the same task. See
`docs/TOOL-INTEGRATIONS.md` for the mechanism and its MIRROR_RULES
declaration.

Cursor requests JSON after the query conditional, including empty, missing or
non-string prompts. Rule publication therefore receives the current fixed
unavailable render when no capsule was assembled, rather than misreading an
operator text report and silently keeping same-task old facts. Whitespace and
privacy-withheld prompts do not use a stored task goal as replacement search.
The generator verifier enforces this placement for native and generated hook
profiles. Custom, linked and tracked-rule custody remains unchanged; local
native-hook checks do not establish live Cursor loading behavior.

Fresh isolated integration controls exercise actual CLI/MCP save, automatic
promotion and recall through copied stdlib runtimes. They verify full chunk
bytes, metadata, citation digests and INDEX rows, source deletion/restoration,
invalid ordinary-source notices, private-request refusal and identical saved
context from native Claude/Codex read hooks. Those checks use fixed query
horizons and public fixture facts. They establish local contract behavior;
provider tokens, semantic generalization, live clients and remote CI remain
separate measurements.

The read hook runs `refresh`, which re-indexes procedural, semantic, and
episodic memory in one incremental pass and reports each layer as `updated` or
`failed`:

```text
Procedural  AGENTS.md, CLAUDE.md, mirrored edition skills
Semantic    README, docs, specs, active Memory Bank chunks, task documents,
            eligible Brain records and handoffs
Episodic    CHANGELOG.md, plus local episodes at query time
```

Working memory is deliberately not in that list. It is not refreshed from
sources; it is written by the turn hook, and in governed mode Project Brain
remains its only authority.

`refresh` limits each SQLite busy wait to 50 ms. If startup cannot acquire a
writer or pass its zero-row SQLite write-admission probe, it attempts an
existing compatible rollback-journal database in
read-only mode. This degraded path returns only freshly validated working
state, with an explicit incomplete warning; it does not search the old index,
read history, update Cursor topic state, publish a manifest or record delivery
receipts. Canonical task identity, lifecycle, privacy and authority must still
pass, and a deleted citation makes the record ineligible here as on the full
refresh; an edited or ignored citation keeps the working state, as it does
there. Unknown document counts remain `null`, and refresh exits 1.

`refresh --read-only` uses that path directly and creates no missing state.
WAL, incompatible schema and unavailable reads produce a fixed unavailable
report. The rollback-journal profile does not certify a concurrent filesystem
snapshot or an adversarial journal-mode switch. SQLite can create sidecars on
a read-only WAL open ([SQLite documentation](https://sqlite.org/wal.html#read_only_databases)),
so WAL is refused before opening. The outer hook timeout still applies:
bounded busy waits are not a whole-refresh deadline. Cold indexing, regular
retrieval and Harness acceptance of incomplete results are separate contracts.

Incremental refresh revalidates current eligibility for each retained Memory
Bank chunk. Matching chunk stat and calendar bounds do not prove that its
cited paths still exist or remain inside supported roots. Invalid chunks are
omitted from the keep set; the existing transaction removes their FTS,
metadata, outgoing links and source-state rows together. Ordinary cached
documents retain their existing reuse path. This adds chunk parsing, secret
screening and citation resolution on warm refreshes; it is a correctness check,
not evidence of faster refresh or token savings.

Legacy chunks can still omit historical `source_digests` or contain a subset,
as allowed by the existing validator. The runtime does not create missing
historical hashes or refresh declared ones during this check. Existing HTTP(S)
claims need no network lookup; local provenance and anchored citations follow
the same current existence/containment rules. Existence without an original
content digest does not establish historical content correspondence. This
check preserves the existing privacy contracts rather than certifying a new
privacy or concurrent filesystem snapshot profile.

A normal retrieval confirms its manifest before committing ordinary delivery
receipts. If optional local publication raises an I/O error, fresh context is
still returned with `manifest: null` and `manifest_publication: unconfirmed`;
no new ordinary receipt pair or trace is claimed. The write may have taken
effect before returning an error, so this does not assert absence, delete a
possible file or adopt it on retry. Failure to prune older local manifests
does not invalidate a successfully confirmed current manifest. Governed
manifest publication remains mandatory, as do result, attempt and approval
journals.

For ephemeral JSON reads, a supported helper accepts the internal
`--provenance-response` flag. Its `local-source-provenance-v1` response carries
source hashes from the final validated selection in the same invocation;
the capsule itself retains its 8,000-character limit. The separate sidecar
is limited to 32 unique rows and 65,536 UTF-8 bytes. Harness requires exact
task UUID/revision, capsule SHA-256 and complete visible-source coverage.
It does not rehash later files or downgrade a missing or corrupt claimed
physical manifest to this transport. The interface marker is not source
authentication or a filesystem snapshot. It adds a helper metadata read and
up to 8,192 header bytes outside the knowledge I/O observer; those costs are
not currently included in that observer's totals. Platforms without the
required protected-open flags, including native Windows, retain the existing
physical-manifest path; optional-failure transport there remains unsupported.
Initial reviewed preparation still requires its governed manifest. Later
ephemeral checks can use the new transport when supported.

Trace cleanup attempts each owned descriptor once and continues through the
remaining handles. Ordinary close errors cannot replace fresh context or a
primary failure; closure is then uncertain and the fd number is not retried.
Otherwise unmasked cancellation propagates after cleanup attempts. Controlled
local descriptor tests do not certify actual kernel close failure or crash
recovery.

The three layers come from one indexing pass and therefore succeed or fail
together, but they are reported separately so a stale layer is visible rather
than silently narrowing the result. When the hook also has a task — from
`CONTEXT_TASK_ID` or the current branch — the same process assembles a bounded
capsule with `--ephemeral`, avoiding the second index pass a separate
`retrieve` would run. A capsule failure is a warning; the layer refresh stands.
A host that has already put the turn's capsule into the prompt sets
`CONTEXT_CAPSULE_DELIVERED=1`, and the hook then stays silent: the Harness
retrieves for the message alone, and a second capsule distilled from the whole
prompt it assembled would cost the turn twice.

The hook passes the prompt as-is; `refresh` distills it into the retrieval
query itself. The whole prompt is tokenized, terms neither indexed documents
nor local episodes have seen or that match most of that combined searchable
corpus are dropped, and the rarest terms fill a bounded query — so a long
request whose actual subject arrives at the end no longer retrieves on its
preamble. The refresh report also carries per-phase wall-clock
durations (`stat`, `index`, `retrieval`) so an operator can see which side of
the work is approaching the hook budget.

The rendered capsule leads with the task itself: after `working: <task> —
<goal>` come bounded `phase:`, `progress:`, `next:`, `recent files:`,
`sources:` and `task record:` lines, and a `compaction:` line whenever those
bounds or the capsule budget left something out. Until they existed the plain
capsule — the only form the hooks hand to a model — printed the goal alone, so
a request to continue retrieved skill pointers and never the place the work
stopped. The same turn
spends no slot on what the model already has: the task's own record and
handoff are excluded as `working-task`, and the instruction files the host
loads by itself as `host-loaded` — `CLAUDE.md` and its `@path` imports for
Claude Code, `AGENTS.md` for Codex. On two real installations `CLAUDE.md` had
held a procedural slot on most Claude Code turns.

`recent files:` names the files the work is in. The Stop hook hands the turn
its host transcript (`turn --transcript`), and the runtime keeps from it only
the project files the turn read or changed: tool-call paths, and the words of
shell commands - `cat`, `sed -n`, `grep`, `git diff`, `git add`, a redirection
- that name an existing project file, changed first. The project's newest 64
stay in local state (`index_state`), newest first; the capsule shows ten, then
the task's own changed files, whole paths only, and shows them before the task
is recorded too, on a new branch's first turns. Dependencies (`vendor/`,
`node_modules/`, `var/`), prose, ignored and secret-looking paths are left out,
and commands, tool results and file contents are never kept. On 1,940 real
Claude Code turns ten such files named a file the agent then read or changed on
75% of new sessions' first turns, against 58% for the task's five changed
files; the reads came almost wholly through shell commands, not the Read tool.

A command is read as the shell reads it: quoting keeps a `|` or `;` inside an
argument (`rg "a|b" x.php`), a redirection names its file however it is
written, `cd` and `git -C` move the words after them, a here-document's body
is data, and Codex's PowerShell and cmd commands on native Windows are read
too. The turn starts at the last user message that is not a local command's
output - a prompt, a slash command, pasted text or a task notification. A Codex
turn starts at the rollout's own `task_started`, so a message the person sends
while it runs joins it; where the window holds none, at the last user message
that is not context Codex adds itself - its `AGENTS.md` instructions and
recommended plugins, a skill's text, a Stop hook's continuation, a subagent's
report, the answer to its question. A task file the line leaves out is counted
in the compaction line.

`ticket files (CMS-741): ...` is an experiment, off unless `ticket_files` is
`true` in `runtime.json` or `CONTEXT_TICKET_FILES=1` (`0` turns a configured
one off for a run). It names the files of the newest commits on any ref (200
at most, no merges) whose message names a ticket of the request, as written,
or of the task's ID, upper-cased - a branch keeps its ticket in lower case as
often as not. A ticket is matched in any case as a word of its own (`CMS-74`
is not `CMS-741`), and only the prefixes the project files its work under
count when the index knows any. Files still present and kept as recent files
are, not named in the request and not among the recent files already shown;
ten at most, whole paths within 720 characters, newest commit first, right
after the `recent files:` line. A Git failure or a two-second timeout leaves
the line out; the turn goes on. The `git log` costs a prompt 35 to 60 ms on
repositories of 4,000 to 5,000 commits. On 155 client cold starts it was the
most precise list - 9.4% of the files it showed were files the turn went on to
change, against 6.1% of the recent files - and it becomes a default only on
the task-outcome measurement sealed for it (`docs/MEMORY-EVAL.md`, "The
`ticket` arm"). The transcript is
read after the turn is recorded, for a second at most; each checkout of an
attached project keeps its own list; and excerpt growth leaves the line its
720 characters.

The capsule is retrieved context, not authority: the ordinary hierarchy still
applies, and a capsule entry never outranks the source it summarizes.

The write hook reads Git porcelain metadata only. Working-tree contents never
reach the buffer, paths that look sensitive are excluded and reported, and the
runtime's own record, handoff, and index churn is dropped rather than recorded
as user work. Paths are normalized relative to the installed edition even when
it lives below the Git root, and Git collapses untracked directory trees before
they enter the buffer. A dependency or cache tree therefore contributes one
directory path rather than thousands of files. Turns accumulate in ignored
local state and flush together, so continuity costs one governed revision per
`--flush-after` turns instead of one per turn. A flush contributes at most
`--max-files` paths, and reports the remainder rather than dropping it silently.

Paths say where work happened, not what it was, and agents almost never write
progress themselves: on four real installations 23 of 24 tasks held nothing
but the automatic checkpoint. So the checkpoint also carries the branch's
newest commit subjects, counted from the merge base with the default branch.
A subject is already shared history and already a person's one-line summary
of a change, so it costs no model and exposes nothing Git did not. Merges and
any subject a secret or privacy gate would refuse are left out. A turn that
ends in a commit leaves a clean tree, so a moved HEAD counts as work too —
otherwise committed work, the most finished there is, never reached the
buffer at all.

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

The turn boundary scans for branch-merge evidence and reports a sanitized
completion candidate. It never changes lifecycle state or records an episode.
The default ships `automatic_completion=false`; the compatibility setting no
longer grants closure authority. Completion requires an explicit `complete`
command with the caller's current numeric revision and verification evidence.
Merging the default branch *into* a long-running branch is not a candidate; the
default branch itself and deleted branches are also excluded.

With `automatic_consolidation` enabled, the turn hook first writes a ticket's
completed events into one finding, and lets the events go once a Memory Bank
chunk carries it. With
`automatic_promotion` it then promotes eligible resolved knowledge into durable
memory, and with `automatic_compaction` it archives terminal records once
`compaction_threshold` of them have accumulated. Compaction runs in batches
rather than every turn: archiving one record at a time would churn Git history
for no benefit.

The order within a turn is fixed — report completion candidates, consolidate,
promote, then archive. Archived records stay promotable, so a promotion run
that hits its per-run cap does not lose the remainder to the archive.

This distinction matters: a healthy status line means the tools are available;
it does not mean an agent has loaded, verified, or acted on the relevant
context.

### CLI stream encoding and newlines

At startup the memory CLI configures exact standard Python `TextIOWrapper`
stdout/stderr instances for UTF-8, replacement encoding errors and untranslated
LF. This applies to subsequent writes; it preserves explicit CR content and
does not transcode bytes buffered before startup. Buffers and stream ownership
are unchanged. Custom streams, including subclasses, retain the existing
encoding/errors-only call, once per stream; their newline behavior is not
certified. Unsupported configuration is best effort and cancellation propagates.
Fresh controls use real supplied CRLF/CR/LF wrappers and an actual CLI error
path. Native Windows, separate MCP/continuity entry points and live clients
require their own evidence.

### Generated read-hook identity

Infrastructure-Creator preserves explicit versus branch-inferred task origin
with the canonical fragments in hook-forge. The generated structural verifier
checks only manifest-owned prompt readers and Cursor turn/session readers.
Missing or unsupported fragments fail with a fixed contract diagnostic;
comments, quoted programs and heredoc data do not satisfy it. The check
protects the admitted identity/argv fragments and does not execute hooks or
prove arbitrary Bash control flow, sourced code, runtime redefinition, atomic
branch epochs or whole-hook budgets. Trusted fixture argv controls are separate
from actual AI generation and live-client memory delivery.

The lexical contract accepts only LF physical records, code/double-quote
continuations and simple quoted literal heredocs (one at a time; exact
delimiter word, optional tab stripping). Unquoted/concatenated delimiters,
arithmetic-shift ambiguity and unsupported quoted tails fail closed. Both
resolved task and explicit selector writes invalidate the contract. Alongside
the preferred ROOT_DIR/symbolic-ref fragment, the gate accepts the finite
installed PROJECT_DIR/branch-show-current fragment with its existing forwarding
relay, budget variable and guarded Cursor JSON append. Both profiles retain
the inferred-only marker. Cursor needs exactly one JSON append before dispatch;
other hosts cannot append it. These checks do not prove arbitrary shell code.

Redirection operators assembled across code continuations are unsupported.
The explicit selector may appear only in canonical fallback/guard lines;
named prefix increments and default-parameter assignments cannot rewrite
resolved task identities.


### Current ignore policy for cached memory

Cached search and governed recall re-admit project documents against the current
Git ignore policy by their own path: a document the project now ignores is not
served from the cache. A file a record cites being ignored withholds neither the
record nor the task's working state. The record is already shared through
Project Brain or the Memory Bank, and citing `vendor/...`, `.env` or the
accelerator's own ignored specs and skills is how framework work gets described;
holding such knowledge back protected nothing and cost the answer. The shared
probe retains protected Git configuration (including global excludes and
safe.directory), uses lexical working-tree names and normal tracked-file
semantics (`check-ignore`, without `--no-index`). Attached tooling/state roots
keep their own custody. Ordinary accelerator document exemptions remain. HTTP(S)
citations are not local file names. Citations still drive the existence check: a
deleted cited file withholds the record (`source-missing`), a cited directory
inside the project counts as present.

Runtime metadata channels are batched before file hydration; optional unsafe or
unindexed source hints abstain before policy admission. Raw search filters its
bounded ranking pool (at most ten times the requested limit) before returning the
limit; it does not promise arbitrary refill beyond that pool. Internal Python
`search_documents` callers must supply `repository` for source-policy admission;
all CLI/capsule runtime callers do so. A bare connection remains the legacy raw
index helper and is not a runtime authority check.

Final admission prunes all delivered views of a document whose own path became
ignored and recomputes signatures, usage and receipts, including episode-only
capsules. These are point-in-time
checks, not an atomic snapshot of the whole filesystem. Unknown Git policy fails
with `Git source context unavailable`; it is never an empty ignore set or an
ordinary read-only warning.

Fresh local controls exercise actual temporary-project CLI/MCP and supplied
SQLite/packing boundaries. Native Windows symlink cases require symlink privileges;
only those individual cases are skipped when Windows reports error1314. Native
Windows, live clients, remote CI, actor readiness, model quality and full lifecycle
provider-token savings require separate evidence and are not established here.


### Final source images and working records

After packing, governed recall rechecks only the delivered document rows using
fresh privacy/owner/authority config and current normalized UTF-8 content hashes.
Changed/deleted/unreadable ordinary or Brain content is removed from every packed
view before manifest, receipts and proof. Packed-out documents are not hydrated
again, and callbacks are never repeated or refilled. Full declared local citation
existence includes legacy undigested links; remote HTTP(S) content remains unknown.

Final checks do not rerank. Existing admitted citation drift retains its original
score and warning when its changed-path set stays equal. Newly changed/restored
sets are withheld so already packed warning text cannot expand. This does not prove
that an already-drifted citation's bytes stayed identical; unchanged-byte review
expiry and changes to secret-screen policy require their existing indexing and
eligibility paths, beyond this content-image check.

Working equality binds the full parsed record plus body from the original read,
its path/UUID and current eligibility. It is semantic equality, not a raw-byte
whitespace seal. Changed/missing/invalid/out-of-scope working state is withheld;
new fields are not adopted without packing. Derived queries and dependent views
withdraw together. A final working descriptor uses nonblocking/no-follow flags
where supported and regular-file admission. Single-attempt cleanup preserves an
active abort against ordinary close failure; cleanup cancellation propagates.
Failed closes remain uncertain and are never retried. These checks do not impose
a whole-operation deadline or an atomic snapshot of regular/network file reads.

Fresh controls assert boundary membership, canaries, hashes, scores, accounting,
receipts and native assembly proof consistency. They do not establish byte-for-byte
complete ordered outputs, full live-client CLI delivery, real Windows, actor
readiness, remote CI, model quality or provider-token savings. A POSIX FIFO control
retains its fixed2s bound and records started-helper timeouts as censored failures.


Current Memory Bank expiry is reapplied at delivery. Cached raw search checks the indexed review/validity boundary; governed retrieval checks the declared dates in the current source image, including after packing. The boundary day is inclusive. This temporal check does not certify a legacy manually inserted SQL row with no declared bounds or revalidate all chunk metadata; normal discovery still owns full chunk validation.

Cached delivery also reapplies the current secret-recognition grammar to full unhighlighted indexed title, summary and content, plus the returned path/title/snippet. A changed or refused masking result withholds the row; already-safe masked documents and template expressions retain their bytes. Raw search gets its checked image and snippet from the same SQL statement and preserves its five public fields. Governed retrieval retains the first admitted image through packing and rechecks it under the current grammar. Optional source hints enforce their cached-content byte bound before image hydration. Structured Brain/Memory Bank source records remain subject to strict whole-record checks, never masking metadata into authority. These checks do not establish an atomic prequote-to-prime database/filesystem epoch. Local manifest exclusion diagnostics may retain rejected identifiers; this is not a promise that every local artifact is free of such identifiers.


Claude Code and Codex automatic prompt reads call `context.py refresh` directly, bounded by the hook's own `timeout` and its single `CONTEXT_HOOK_BUDGET` (default 5 seconds). Whatever budget the person sets is honoured, and the prompt reaches the CLI whole, whatever its length. A refresh stopped at the budget (status 124) delivers nothing - a cut capsule reads like a whole one - and the turn is told memory was not consulted. A refresh that fails names its cause with the CLI's own last stderr line; one that exits 1 after printing its layer report delivers that report. Cursor keeps its persistent-rule path. The hooks start the CLI through `memory-bank/scripts/context_cli.py`, which runs `context.py` as the same script from its cached bytecode: started directly, the 8,000-line module is compiled again on every prompt (about 30 ms); a runtime without the launcher falls back to `context.py`. The launcher writes that cache even when the caller asks Python for none - Claude Desktop starts Claude Code with `PYTHONDONTWRITEBYTECODE=1`, the hooks inherit it, and the Harness sets it - for the runtime's own directory only: `memory-bank/scripts/__pycache__/` is ignored wherever the runtime is installed, and nothing else the runtime imports is cached. A cold index too large for the budget still restarts on every prompt; this path does not implement resumable indexing, and in an installed project the budget message says to build the index once outside the hook (`python3 memory-bank/scripts/context.py index`).

Candidate construction now holds the unhighlighted indexed image in the same SQL statement as its highlighted/plain quote. Missing rows and changed path/layer/kind/title identities remain unavailable rather than adopting a later cache image. Conflict/automatic-link rows capture the image with their body; path, hint and source-graph snippets reuse the already-admitted body, including the hint UTF-8 byte cap. This closes the quote-to-first-prime substitution gap.

Native governed and lightweight packets also carry immutable admitted images outside their public dictionary keys through budget copies, window proposals, packing and final return. Rich rendering and section novelty identities use those bodies. The normal FTS highlight is accepted only alongside an exactly matching unhighlighted image from the same statement; if the cache moved, isolated in-memory FTS marks the held body with the same tokenizer. Failure uses held plain text rather than a replacement body. Detached JSON/dict copies use bounded admitted snippets; a missing entry in a bound packet never enables a live-body read. Shared image sinks preserve the first public row, matching deduplication. No private image is added to JSON, manifests or provider input. Ordinary body byte sizes retain their existing profile; this is not a hard operation memory cap or one atomic snapshot across earlier ranking/admission metadata, corpus weights, citation reads and filesystem state.

Refresh health summaries expose `source_notices` only when retained records contain accepted counts: `records` counts journal records with notices and `totals` sums reported invalid UTF-8 content/path occurrences after the requested window. Repeated observations may describe the same source or turn. Unknown keys, nonpositive values, booleans, strings and floats are excluded; source identities/content are never included. Text health uses fixed labels. Missing counts remain absent and do not establish that all sources were valid.
