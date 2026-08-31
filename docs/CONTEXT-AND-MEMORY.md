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

The database is derived and disposable only in part. Its document and metadata
indexes can be rebuilt from repository sources. Governed compatibility bindings
can be restored from Git-tracked task records with `rebind`, and the next
automated turn flush performs the same restoration. Lightweight tasks and local
episodes remain machine-local and cannot be reconstructed after deletion.

The active provider is local `sqlite-fts5`: network access and embeddings are
disabled. An external provider entry exists only as a disabled contract. The
runtime does not include automatic prompt injection.

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
episode could not be written.

`event` is the only record type mapped to the episodic layer. An `incident`
stays semantic even though it is also a record of something that happened: an
open incident is active, urgent, promotable content, and moving it to the
single episodic slot would take it out of the runtime filters, the budget and
the manifest.

The episodic slot of a governed capsule is ranked by `retrieve()` along with
everything else. It used to be fetched separately by a query that never joined
the metadata table, which applied no privacy, owner, authority, lifecycle or
freshness filter and left the delivered document out of the manifest entirely.
That was harmless while the only episodic document was the changelog; it stops
being harmless once governed records live there.

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
- `codebase/**/*.md` — recursive, so a map filed per module is indexed whole
  rather than only at its top level;
- active `memory-bank/chunks/*.md`;
- `tasks/**/*.md`;
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

A pattern that matched files on disk and lost every one of them to
`.gitignore` is reported as an exclusion named for the pattern, with the
reason `pattern-all-git-ignored`. Individual ignored files stay silent on
purpose — Symfony alone contributes hundreds — but a whole source of truth
going dark is not a routine skip. Measured on a real installation: a project
whose own `.gitignore` carried a bare `docs` entry indexed 96 accelerator
skills, one `README.md`, and none of its own design documents, without a word.

The same pass builds `document_links`, the reverse index of which document
declares which source, read by `links` and by `retrieve --path`. It is derived
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
cross-record conflict state rather than on the record file alone. A change
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
2/3/1 would buy a larger lexical net, and the failure above is not a
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
where editing the referenced file evicts the chunk as `source-changed` and
deleting it becomes a permanent bank-validation error reachable from
`apply_promotion`, `compact` and `validate`.

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
  `chunk_source_digests`, where the next edit to any of them evicts the chunk
  as `source-changed`.
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

### The retrieval gate

Restraint above was all about the document: which files are relevant enough to
occupy a slot. It cannot express the other question — whether this turn needed
retrieval at all. A pointer that says "relevant right now" about the wrong file
costs more than no pointer, because the agent opens it; the saving is bias, not
tokens, and the work the gate skips is trivially cheap either way.

`gate_decision` records a verdict on every governed retrieval. Two rules fire,
both deterministic and both computable before any document body is opened:
`no-match` when nothing survived the relevance test, so retrieving and skipping
deliver the same thing, and `repeat-retrieval` when the distilled query and the
selected path set both match the previous turn of the same task. That pair is
the invariant — not "the capsule repeats byte-for-byte", which can never happen:
every call mints a fresh manifest id, and the rendered capsule folds in an
automatic checkpoint sentence and a last-turn summary that move on their own.
The previous turn is remembered as two hashes in the disposable index, in a
single bounded row rather than one per task, and a skip never overwrites it —
otherwise the turn after a skip would compare against nothing.

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

Whatever the reason a document leaves, `index` names it rather than dropping
it silently — `retired`, `overdue-review`, a non-active status, `invalid`, or
`secret` — so a red validator and a quietly shrinking index can no longer
disagree about the same chunk.

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

Closing a period is an operation, not an editing convention: `context.py
bank-retire --id MEM-... --valid-to YYYY-MM-DD [--superseded-by MEM-...]`
writes the chunk, the other side of any replacement link, and the regenerated
index under one lock, validates the bank, and restores every touched file if
anything fails. Doing it by hand leaves the bank invalid between the first
edit and the last.

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
policy filtering, the delivered capsule is independently capped at 2
procedural, 3 semantic, and 1 episodic item and 8,000 serialized characters.
Snippets are deterministically shortened as needed.

These controls bound selected retrieval context, not the size of source files,
Brain records, user prompts, or model responses. Conflict escalation can exceed
normal category/target budgets, never the hard ceiling.

## Retrieval Manifests

Every successful governed retrieval writes a manifest. By default it lands in
the Git-tracked `project-brain/control/retrieval-manifests/<uuid>.json`.

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
- privacy, owner, authority, freshness, and active-only filters;
- selected paths, categories, estimated tokens, source hashes, and how
  each one matched (`covered`, `distinctive`, or `conflict`);
- excluded paths and safe reasons;
- the retrieval gate's verdict for the turn: `decision`, `mode`, `reason`, and
  the signals it decided on;
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

Application mints a conflict-free Memory Bank ID (`MEM-YYYYMMDD-xxxxxxxx`,
the promotion date plus eight hex characters of the source record's UUID),
writes a chunk, regenerates the index from chunk frontmatter, validates the
bank, and updates the proposal. These writes are snapshotted and rolled back
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

Cursor has no `UserPromptSubmit` equivalent, so its read half is delivered
differently: the Cursor mirrors of the `stop` and `sessionStart` hooks render
the most recently available capsule into the `alwaysApply` rule
`.cursor/rules/working-memory.mdc` - ignored local state, one turn stale by
design and labeled as such ("as of end of previous turn"). See
`docs/TOOL-INTEGRATIONS.md` for the mechanism and its MIRROR_RULES
declaration.

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

The three layers come from one indexing pass and therefore succeed or fail
together, but they are reported separately so a stale layer is visible rather
than silently narrowing the result. When the hook also has a task — from
`CONTEXT_TASK_ID` or the current branch — the same process assembles a bounded
capsule with `--ephemeral`, avoiding the second index pass a separate
`retrieve` would run. A capsule failure is a warning; the layer refresh stands.

The hook passes the prompt as-is; `refresh` distills it into the retrieval
query itself. The whole prompt is tokenized, terms the index has never seen or
that match most of the corpus are dropped, and the rarest terms fill a bounded
query — so a long request whose actual subject arrives at the end no longer
retrieves on its preamble. The refresh report also carries per-phase wall-clock
durations (`stat`, `index`, `retrieval`) so an operator can see which side of
the work is approaching the hook budget.

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

The turn boundary scans for branch-merge evidence and reports a sanitized
completion candidate. It never changes lifecycle state or records an episode.
The default ships `automatic_completion=false`; the compatibility setting no
longer grants closure authority. Completion requires an explicit `complete`
command with the caller's current numeric revision and verification evidence.
Merging the default branch *into* a long-running branch is not a candidate; the
default branch itself and deleted branches are also excluded.

With `automatic_promotion` enabled, the turn hook also promotes eligible
resolved knowledge into durable memory on the same boundary, and with
`automatic_compaction` it archives terminal records once `compaction_threshold`
of them have accumulated. Compaction runs in batches rather than every turn:
archiving one record at a time would churn Git history for no benefit.

The order within a turn is fixed — report completion candidates, promote, then
archive. Archived records stay promotable, so a promotion run that hits its
per-run cap does not lose the remainder to the archive.

This distinction matters: a healthy status line means the tools are available;
it does not mean an agent has loaded, verified, or acted on the relevant
context.
