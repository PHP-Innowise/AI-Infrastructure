# Context and Memory Operations

This runbook covers day-to-day use of the Project Brain, Local Context Engine,
and Memory Bank shipped with the Laravel, Symfony, and PHP Core accelerators.
Their context runtimes are identical; only framework-specific configuration and
workflow guidance differ.

Run commands from the selected accelerator or consuming-project root, where
`memory-bank/` and `project-brain/` are siblings:

```bash
python3 memory-bank/scripts/context.py --help
```

Requirements are Python 3.9 or later and SQLite with FTS5. No network service,
MCP server, embeddings provider, or daemon is required.

## Installation Inventory Operations

Ready-made edition installation is governed by exact, versioned inventories,
not by directory-wide exclusions. From the accelerator repository root:

```bash
python3 scripts/install_accelerator.py --verify-inventories
python3 scripts/install_accelerator.py \
  --edition Laravel \
  --target "/path/with spaces/project" \
  --tool claude \
  --merge-existing \
  --dry-run
```

The verifier fails for an unlisted or stale distribution path. The installer
preflights every destination. Standard adoption keeps `--merge-existing`: it
leaves byte-identical files unchanged, conservatively merges marked accelerator
blocks into `.gitignore`, `.gitattributes`, and `AGENTS.md`, and preserves an
existing root `README.md` while installing accelerator documentation as
`ACCELERATOR.md`. Every unsupported collision still aborts the whole operation
without writes. `--overwrite` is an explicit exceptional mode, not the normal
recovery for a collision. The tab-separated `WOULD_COPY`, `WOULD_MERGE`,
`WOULD_COPY_AS`, `COPY`, `MERGE`, `COPY_AS`, `COLLISION`, and `COMPLETE` lines
are deterministic for a given inventory and tool selection. Retain the
successful copy transcript outside the target as the rollback manifest.

Inventories identify files shipped by Laravel, Symfony, and PHP Core, grouped
as shared, Claude, Cursor, and Codex components. They never inventory
`memory-bank/local/`, SQLite databases, personal settings, `.env`, or other
runtime/user state. See [Safe Adoption](ADOPTION.md) for collision resolution,
activation, rollback, and upgrades.

For the ownership model, authority rules, and data boundaries behind these
procedures, see [Context and Memory](CONTEXT-AND-MEMORY.md). For the governed
and lightweight choices, see [Context Modes](CONTEXT-MODES.md). For a complete
worked task, see the [User Task Workflow Example](examples/USER-TASK-WORKFLOW-EXAMPLE.md).

## Operating Rules

- Use **governed mode** for normal shared work. Project Brain owns active task
  state and handoffs.
- Use **lightweight mode** only when machine-local task state is intentionally
  disposable.
- Supply a stable caller-owned task ID. In governed mode, do not derive an
  authoritative task from the Git branch.
- Set an explicit owner for shared work and keep using that owner for later
  mutations.
- **Always supply `--revision` for governed `update` and `complete`.** The
  compatibility parser currently permits omission, but omission bypasses the
  intended optimistic revision check. Treat it as unsupported operational
  practice.
- Reload and reconcile after a stale-revision failure; never overwrite
  concurrent work.
- Store concise, sanitized facts only. Do not store prompts, responses, hidden
  reasoning, raw diffs, command output, logs, secrets, credentials, customer
  data, personal data, or unredacted incident payloads.
- Treat retrieved snippets as discovery aids. Verify material claims against
  current policy, specifications, code, configuration, migrations, and tests.
- Do not hand-edit lifecycle, revision, deterministic index, handoff, manifest,
  or promotion fields when a supported command exists.
- Do not stage runtime changes automatically. Review all Git-tracked Project
  Brain or Memory Bank writes before staging them.

### Context Economy

The cost of a token depends on how long it stays resident, not on when it was
read: the whole context is re-read on every turn, so anything placed in context
early is paid again on each subsequent turn of the session. Two operating
consequences follow. Both thresholds were derived from local transcripts with
`scripts/cost_attribution.py`; re-derive them on your own corpus before
treating them as settled.

- **Delegate to a subagent only when it displaces roughly seven or more turns
  of the main session.** A subagent run is cheaper per turn than a main-session
  turn, but it pays its own context setup and returns a result the main session
  then carries. Below that break-even, fanning out costs more than doing the
  work inline. Fan out for genuinely parallel or genuinely disposable work -
  broad searches, independent reviews, anything whose intermediate context the
  main session must not inherit - not to keep the main transcript tidy.
- **Compact deliberately once the context approaches ~400k tokens.** Turns
  above that line are a minority of turns but a majority of main-session spend,
  because every one of them re-reads everything below it. Compacting at a
  natural boundary costs one summarization; drifting past it costs the full
  context on every remaining turn.

Two related facts worth knowing before optimizing anything here. A file under
`.claude/`, `.cursor/` or `.codex/` that `scripts/build_mirrors.py --check`
regenerates is not canon: edit the canonical source, or the change is silently
discarded on the next `--write`. Each edition's `.gitattributes` lists exactly
those generated files so they collapse to a stub in review diffs; the
canonical files living in the same directories are deliberately absent from it
and keep their diffs.

## Typical Prompt and Task Lifecycle

The user normally supplies intent and a stable task ID:

```text
Implement TASK-123 using specs/two-factor-auth.md.
```

The agent or operator then:

1. Selects the correct accelerator and reads its policy and relevant skill.
2. Reads `project-brain/config/runtime.json`; governed mode is the default.
3. Selects an explicit owner, for example `alice` or an agent identity.
4. Runs health checks.
5. Starts the task, or gets the existing task and revision.
6. Refreshes the local index explicitly.
7. Retrieves task-aware context explicitly.
8. Opens and verifies the cited canonical sources.
9. Performs the requested work and verification.
10. Updates Project Brain after meaningful milestones and before a handoff.
11. Completes only after verification succeeds.
12. Optionally proposes a reusable consequence for independent human review.
13. Compacts terminal records during maintenance, not as part of every prompt.

Example:

```bash
export PROJECT_BRAIN_OWNER=alice

python3 memory-bank/scripts/context.py status --json
python3 memory-bank/scripts/context.py validate
python3 memory-bank/scripts/context.py parity

python3 memory-bank/scripts/context.py start \
  --task-id TASK-123 \
  --goal "Add two-factor authentication" \
  --source specs/two-factor-auth.md \
  --json

python3 memory-bank/scripts/context.py index --json

python3 memory-bank/scripts/context.py retrieve \
  "two-factor authentication requirements and security rules" \
  --task-id TASK-123 \
  --json

python3 memory-bank/scripts/context.py update \
  --task-id TASK-123 \
  --revision 1 \
  --progress "TOTP enrollment and verification implemented" \
  --next-step "Implement recovery codes" \
  --file src/Security/TwoFactorService.php \
  --source specs/two-factor-auth.md \
  --json

python3 memory-bank/scripts/context.py complete \
  --task-id TASK-123 \
  --revision 2 \
  --outcome "Two-factor authentication implemented" \
  --verification "Authentication and recovery-code tests passed" \
  --json
```

Read the revision returned by each successful mutation and use that exact value
for the next governed mutation. The examples above assume no intervening write.

## Health Checks

Use these checks before shared work, after runtime maintenance, and before a
handoff or promotion:

```bash
python3 memory-bank/scripts/context.py status --json
python3 memory-bank/scripts/context.py validate --json
python3 memory-bank/scripts/context.py parity --json
python3 memory-bank/scripts/validate.py
```

Interpretation:

- `status` reports the selected mode, authority, database path, document and
  episode counts, working count, and counts for procedural, semantic, and
  episodic indexed layers. It does not prove index freshness or validate Brain
  records.
- `validate` checks active and archived Brain records, strict schemas,
  relationships, source fingerprints, handoffs, manifests, promotions, and
  deterministic indexes. It exits `1` when errors exist.
- `parity` compares mirrored skill implementations with the
  `canonical_edition` configured in `runtime.json`. It fails on drift.
- `memory-bank/scripts/validate.py` validates durable Memory Bank structure and
  metadata independently of Project Brain validation. It reports two severity
  levels. **Errors** exit `1`: a chunk that is wrong, or an index row pointing
  at a chunk that is not on disk. **Warnings** print to stderr and exit `0`: a
  terminal chunk — `superseded`, `archived`, or past its `valid_to` — whose
  cited source has since been deleted, and an index row kept for a chunk that
  is present but failed its own validation. Deleting a cited file is routine
  work, and a chunk that correctly reached the end of its life must not fail
  an unrelated task. A source path that escapes the repository stays an error
  in every status: that is a containment guarantee, not a freshness one.
- `index --json` reports exclusions and skill parity drift, but parity drift is
  reported rather than made fatal. Use the dedicated `parity` command when
  drift must fail the check.

Every CLI invocation opens the SQLite database and ensures its tables exist.
Therefore even read-oriented commands can create the ignored
`memory-bank/local/context.db` when it is absent.

## Global Arguments and Output

Global arguments must appear **before** the subcommand:

```text
--root PATH
--db PATH
--mode governed|lightweight
--owner OWNER
```

- `--root` selects the repository root. Its default is derived from the
  location of `context.py`, two directories above the script.
- `--db` overrides the default
  `memory-bank/local/context.db`. This changes local index, binding, task, and
  episode storage only; it does not relocate Project Brain.
- `--mode` overrides mode for one invocation. Precedence is command-line
  `--mode`, then `PROJECT_BRAIN_MODE`, then
  `project-brain/config/runtime.json`.
- `--owner` identifies the Project Brain actor. Its fallback is
  `PROJECT_BRAIN_OWNER`, then `local`.

`--json` is a per-command option and appears after the subcommand and its
arguments. It emits one JSON value on standard output. Result shapes differ by
command; consume named fields rather than assuming one universal schema.
User-facing errors are written to standard error as `context: MESSAGE` and
return exit code `1`. Argument-parser errors use argparse's normal nonzero exit.
Do not parse human-readable output in automation.

Repeated `--file`, `--source`, `--next-step`, `--verification`, `--conflict`,
and `--source-id` options append one value per occurrence:

```bash
python3 memory-bank/scripts/context.py --owner alice brain-create finding \
  --external-id FINDING-42 \
  --title "Two related constraints differ" \
  --source specs/first.md \
  --source specs/second.md \
  --conflict 550e8400-e29b-41d4-a716-446655440000
```

Task IDs and external IDs accept letters, digits, `.`, `_`, `/`, and `-`, must
start with a letter or digit, and are limited to 128 characters. Record
relationships use Project Brain UUIDs, not external aliases.

## CLI Reference

### `refresh`

```bash
python3 memory-bank/scripts/context.py refresh \
  [--query TEXT --task-id ID] \
  [--limit N] \
  [--ephemeral] \
  [--validate] \
  [--json]
```

Refreshes procedural, semantic, and episodic memory in one incremental pass and
reports each layer as `updated` or `failed`. All three come from the same
indexing pass and therefore succeed or fail together, but they are reported
separately: a caller told only that "refresh failed" cannot tell which part of
its context went stale.

With `--query` and `--task-id` it also assembles the Task Capsule in the same
process, skipping the second index pass that a separate `retrieve` would run.
A capsule failure is reported as a warning and does not undo the layer refresh.

Exit status is `1` when any layer failed, so an automated caller can tell a
stale refresh from a working one. `--validate` additionally reports Project
Brain validation, which reads every record and is not free as records
accumulate.

This is the command the request hook runs. Side effects: writes only the
selected SQLite database, plus an ignored local manifest when `--ephemeral`
accompanies a capsule.

### `index`

```bash
python3 memory-bank/scripts/context.py index [--incremental] [--json]
```

Rebuilds the disposable FTS5 document index from eligible policy, skills,
documentation, specifications, active Memory Bank chunks, task documents,
capability epics, changelog, active Brain records, and handoffs. It filters
Git-ignored candidates before reading them, rejects likely secrets, deduplicates
mirrored skills, and replaces the previous document/metadata index in one
SQLite transaction.

`--incremental` reuses every row whose source modification time and size are
unchanged **and whose calendar boundary has not passed**, so only new,
modified, or newly out-of-date documents are re-read and re-scanned and a
no-change refresh performs reads only. A durable memory chunk retires on a
date rather than on an edit, so the cache records `min(review_after,
valid_to)` next to the stat pair: the day after that boundary the row is
re-validated and the chunk leaves the index without anyone touching the file.
A cache row written before this column existed carries no boundary and is
re-validated once rather than trusted. Project Brain records are always
rebuilt, because their eligibility depends on configuration, lifecycle, and
cross-record state rather than on the record file alone. Without the flag this
is a full rebuild.

Every document dropped during discovery is reported in `excluded` with the
reason it was dropped: `retired` (past `valid_to`), `overdue-review` (past
`review_after`), `superseded` / `archived` / `needs-review` (a non-active
status), `invalid` (frontmatter that fails validation), and `secret` (the
secret scan rejected the file). `--json` lists the paths; the plain output
prints a per-reason tally. Structural skips — a Git-ignored candidate, a
second pattern matching a file the first already owns, a mirrored copy of an
indexed skill — are how discovery is defined and are deliberately not
reported.

Side effects: writes only the selected SQLite database. It does not create a
task, retrieve context, or inject context into a prompt. Invalid UTF-8 aborts
before replacing the previous index.

### `search`

```bash
python3 memory-bank/scripts/context.py search QUERY \
  [--limit N] \
  [--layer procedural|semantic|episodic] \
  [--json]
```

Performs lexical FTS5/BM25 search over the existing index. The default limit is
`8`. With no layer filter it also searches local completed episodes; episodic
episodes are included when `--layer episodic` is selected.

This is a broad index lookup, not governed task-aware retrieval. It does not
create a retrieval manifest or re-check all governed retrieval controls.
Refresh the index first when freshness matters.

Side effects: SQLite initialization only.

### `links`

```bash
python3 memory-bank/scripts/context.py links --path SOURCE \
  [--prefix] \
  [--json]
```

Reports every eligible document that declares `SOURCE` among its sources — the
reverse of reading a chunk's frontmatter. Use it to answer "what did we already
write down about this file" before editing it, and to see what
`retrieve --path` would pull in.

`--prefix` treats the path as a directory and matches sources beneath it. A
line-range fragment (`file.md#L4-L9`) is anchored to the document, as
`fingerprint()` anchors it.

Unlike `search`, this joins `document_metadata` and applies the full runtime
filter, so a record withheld by privacy, owner, authority, lifecycle or
freshness is reported under `excluded` rather than shown. Reads the existing
index without refreshing it.

Side effects: SQLite initialization only.

### `retrieve` and compatibility `context`

```bash
python3 memory-bank/scripts/context.py retrieve QUERY \
  --task-id ID \
  [--limit N] \
  [--path SOURCE ...] \
  [--ephemeral] \
  [--json]

python3 memory-bank/scripts/context.py context QUERY \
  --task-id ID \
  [--limit N] \
  [--path SOURCE ...] \
  [--ephemeral] \
  [--json]
```

`retrieve` is the public task-aware interface; `context` is a compatibility
alias. The default per-query limit is `3`.

Both refresh the index incrementally before reading it, because retrieval reads
the index rather than the sources and a stale index silently narrows the
result. A refresh failure degrades to a warning instead of failing the request.

`--path` is repeatable and also delivers documents that cite that source, for
the case lexical ranking cannot reach: two chunks whose only connection is a
shared `sources[]` entry are unreachable from each other and from the source's
own words — measured at 0 of 2, and 2 of 2 through the link index. Path-linked
items lead their layer, carry `selection: "path-link"` in the manifest and no
`match`, and do not change `no_match`, which remains a statement about the
query. See `docs/CONTEXT-AND-MEMORY.md`.

`--ephemeral` writes the retrieval manifest to the ignored
`memory-bank/local/retrieval-manifests/` instead of shared Git history, capped
at the most recent 200. Automated retrieval must use it: one manifest per
request would otherwise add thousands of tracked files.

In governed mode, the task ID must resolve through a local binding to a Brain
task. Retrieval applies privacy, configured owner, authority, lifecycle,
supersession, source-body freshness, and source-fingerprint checks; assembles
bounded snippets; retains eligible conflict partners; enforces category and
total token estimates; and writes a strict retrieval manifest.

Normal estimated-token budgets are:

- policy: 1,200;
- handoff: 1,500;
- durable: 3,500;
- dynamic: 1,500;
- evidence: 2,000;
- target total: 8,000;
- hard ceiling: 12,000.

An eligible conflict pair may exceed its category or target budget up to the
hard ceiling, with an escalation reason. Privacy, authority, lifecycle, owner,
and freshness filters still take precedence. Token counts are estimates based
on text length, not provider billing measurements.

The delivered capsule has a separate final contract in both modes: at most 2
procedural, 3 semantic, and 1 episodic item and 8,000 serialized characters.

It also reports the quality of what it found. `no-match: <layers>` names the
layers where no candidate passed the relevance test — measured before any
filter or budget runs, so it never claims memory was empty when something was
withheld. `weak-match: <path>` marks an item admitted on a single rare term
rather than on covering the query. Both lines are printed after the opening
`working:` line, which the Cursor hooks use as the render marker.

Side effects in governed mode: creates
`project-brain/control/retrieval-manifests/<uuid>.json`, a Git-trackable
metadata record. The manifest contains query and selection/exclusion metadata,
hashes, filters, and token estimates—not prompts, responses, hidden reasoning,
or source bodies.

In lightweight mode, these commands assemble the local working task plus
procedural, semantic, and episodic search results. They do not create a
governed manifest and do not apply the governed filter pipeline.

### `start`

```bash
python3 memory-bank/scripts/context.py [--owner OWNER] start \
  --task-id ID \
  --goal GOAL \
  [--file PATH]... \
  [--source PATH]... \
  [--json]
```

In governed mode, creates a UUIDv4 task, an active handoff, deterministic Brain
indexes, and a local SQLite binding/pointer. The Brain record—not SQLite—is
authoritative. The initial revision is `1`.

In lightweight mode, creates one machine-local `working_tasks` row. No Brain
record or handoff is created.

If cross-store governed creation fails, the runtime attempts to restore both
the Brain files and local binding state.

### `update`

```bash
python3 memory-bank/scripts/context.py [--owner OWNER] update \
  --task-id ID \
  --revision N \
  [--progress TEXT] \
  [--next-step TEXT]... \
  [--file PATH]... \
  [--source PATH]... \
  [--json]
```

At least one changed field is required. `progress` replaces the previous
progress text; repeated list values are merged without duplicates.

In governed mode, this verifies actor authorization, checks the expected
revision when supplied, updates the task and handoff, rebuilds deterministic
indexes, advances the revision, and refreshes the local binding. The operation
uses a repository-wide lock and compensating snapshots across Brain and SQLite.

Operational policy requires `--revision` in governed mode even though the
compatibility parser does not mark it required. It is ignored by lightweight
updates, where SQLite owns the working task.

`--revision auto` resolves the current revision inside the runtime mutation
lock, making the read and the write it feeds one compare-and-swap. It exists
for automated writers that cannot know the revision in advance. An operator who
knows the revision should still pass it: only an explicit value detects that
the record moved between reading it and deciding to write.

### `turn`

```bash
python3 memory-bank/scripts/context.py [--owner OWNER] turn \
  --task-id ID \
  [--flush] \
  [--flush-after N] \
  [--max-files N] \
  [--json]
```

Buffers one turn's change set and flushes a consolidated update to the
authoritative task on a boundary. Buffering is what keeps per-turn continuity
affordable: without it, every turn would cost one governed revision and one
rewritten handoff. The default boundary is every `5` turns; `--flush` forces
one.

Change sets come from Git porcelain metadata alone. Working-tree contents are
never read, paths that look sensitive are excluded and reported, and the
runtime's own record, handoff, and index churn is dropped rather than recorded
as user work. A flush contributes at most `--max-files` paths (default `20`)
and reports the count it omitted.

With `automatic_promotion` enabled, a flush also runs automatic promotion; see
[Promotion to Durable Memory](CONTEXT-AND-MEMORY.md#promotion-to-durable-memory)
for what qualifies.

The first flush provisions the task when it does not exist, deriving a goal
from the branch name and recording that provenance in the goal itself. Buffered
turns alone provision nothing, so visiting a branch mints no record. An
existing task is never overwritten: a goal written by an operator survives.

When the governed record already exists but the machine-local binding does not
— a second machine or a fresh clone, where `context.db` never travels with Git
— the flush restores the binding to the existing record instead of failing on
a duplicate. The turn result and the last-turn report carry the reconnected
record UUID in `rebound`, and the next capsule's `Last turn` section states
`binding restored to the existing task`. A turn that fails outright also
writes the report, with the error text, so the next capsule surfaces
`turn failed: ...` rather than losing the failure with the silenced output.

Buffered turns are deleted only after the authoritative write lands, so an
interrupted flush replays rather than loses the buffer.

This is the command the turn-end hook runs. Side effects: the ignored SQLite
buffer, plus one task update per flush.

### `rebind`

```bash
python3 memory-bank/scripts/context.py rebind \
  --task-id ID \
  [--record UUID] \
  [--json]
```

Governed mode only. Restores the machine-local SQLite binding for a governed
task that already exists in Git — the recovery for a second machine, a fresh
clone, or a deleted `context.db`. This is a pointer repair, not a mutation:
the Brain record is never touched, no revision is consumed, and no owner
check applies.

`--record` names the task record UUID explicitly; without it the record is
resolved by the task ID. Either way the record must exist, be of type `task`,
be non-terminal, and carry `--task-id` as its own `external_id` — the command
can only reconnect a task to its own identity, never repoint one task at
another. An intact identical binding is reported (`already_bound`) rather
than recreated; a binding that points at a different record is refused.

The automated flush performs the same restoration on its own (see `turn`), so
`rebind` is needed when you want the compatibility commands (`get`, `update`,
`complete`, ...) to resolve before any flush has happened, or when you want
the reconnection reviewed explicitly.

Side effects: one row in the local `task_bindings` table plus its
compatibility pointer. Nothing under `project-brain/` changes.

### `get`

```bash
python3 memory-bank/scripts/context.py get --task-id ID [--json]
```

Returns the authoritative Brain task through its local binding in governed
mode, including UUID, revision, status, progress, next steps, files, and
sources. In lightweight mode it returns the local SQLite task.

Side effects: SQLite initialization only.

### `clear`

```bash
python3 memory-bank/scripts/context.py [--owner OWNER] clear \
  --task-id ID \
  [--json]
```

This means **abandon**, not complete.

In governed mode, it transitions an authorized active/blocked/verifying task to
`cancelled`, closes its handoff through the task update path, rebuilds indexes,
and removes the local binding. The command has no revision option and the
runtime does not perform a compare-and-swap check for cancellation. Reload the
task immediately before clearing it and use this command only with deliberate
authorization.

In lightweight mode, it deletes the local working-task row. It does not create
an episode.

### `complete`

```bash
python3 memory-bank/scripts/context.py [--owner OWNER] complete \
  --task-id ID \
  --revision N \
  --outcome TEXT \
  [--summary TEXT] \
  [--file PATH]... \
  [--verification TEXT]... \
  [--source PATH]... \
  [--json]
```

Complete only after applicable tests and checks pass. Provide at least one
meaningful `--verification` value for governed operational use, although the
parser permits an empty list.

In governed mode, the runtime creates a non-authoritative local replay episode,
transitions the task to `completed`, closes the handoff, rebuilds indexes, and
removes the local binding. Brain and SQLite changes are rolled back if the
cross-store operation fails.

Operational policy requires `--revision` in governed mode even though the
compatibility parser permits omission. In lightweight mode, the command
atomically converts the local working task into an episode and deletes the
working row.

### `msg-send`, `msg-read`, `msg-dispatch`, and `capsule`

```bash
python3 memory-bank/scripts/context.py msg-send --task-id ID \
  --from AGENT --to AGENT|main|'*' --type finding|question|handoff \
  (--body TEXT | --body-file PATH) [--ref PATH]... [--json]
python3 memory-bank/scripts/context.py msg-read --task-id ID \
  [--for ACTOR] [--since SEQ] [--type TYPE] [--json]
python3 memory-bank/scripts/context.py msg-dispatch --task-id ID \
  --agent AGENT --event spawn|complete [--note TEXT] \
  [--capsule-file PATH] [--json]
python3 memory-bank/scripts/context.py capsule --validate --file PATH|- [--json]
```

The agent message channel behind orchestrated flows (governed mode only):
one append-only JSONL journal per task under
`project-brain/control/messages/`, written under the global mutation lock and
never rewritten, so its git history is the audit trail. Bodies are capped at
8,000 characters and screened by the same secret patterns as task fields;
actors are lowercase slugs plus the reserved `main`, and `*` broadcasts.
`msg-dispatch` records the orchestration log — who was spawned with which
delegation capsule (SHA-256 digest) and who completed — and refuses a spawn
whose capsule fails the mandatory-section check that `capsule --validate`
runs standalone (objective, output format, tool and source guidance,
boundaries, decisions and assumptions). The journal keeps no read cursor:
consumers pass `--for` and `--since` and track their own position. A
terminal task refuses new messages but its journal stays readable, and
`validate` checks every line. Related guard: `update --phase` moves only
forward through the stored phase order unless `--allow-phase-regression`
states the backward move is deliberate, and `update --actor` prefixes the
progress line with the recording agent's slug.

### `record`

```bash
python3 memory-bank/scripts/context.py record \
  --summary TEXT \
  --outcome TEXT \
  [--file PATH]... \
  [--verification TEXT]... \
  [--source PATH]... \
  [--json]
```

Stores a standalone completed-work episode in local SQLite. It is available in
either mode but remains local and non-authoritative. Prefer governed tasks and
records for shared work. This command does not create a Brain record, handoff,
or promotion, and local episodes are never promoted automatically.

### `brain-create`

```bash
python3 memory-bank/scripts/context.py [--owner OWNER] brain-create TYPE \
  --external-id ID \
  --title TITLE \
  [--goal TEXT] \
  [--file PATH]... \
  [--source PATH]... \
  [--conflict UUID]... \
  [--privacy public|team|restricted|private] \
  [--authority inferred|observed|verified] \
  [--confidence 0..1] \
  [--json]
```

`TYPE` is `finding`, `bug`, `incident`, `decision`, or `event`. Tasks use
`start`. Defaults are `privacy=team`, `authority=observed`, and
`confidence=1.0`. Creation writes a UUID-named Brain record and rebuilds
deterministic indexes. Non-task records do not receive handoffs.

Initial statuses are `open` for findings and incidents, `reported` for bugs,
`proposed` for decisions, and `recorded` for events. The schema and runtime
validate confidence and metadata. Private and configured-ineligible records
can exist as governed records but are excluded from indexing/retrieval.

### `brain-update`

```bash
python3 memory-bank/scripts/context.py [--owner OWNER] brain-update \
  --record-id UUID_OR_EXTERNAL_ID \
  --revision N \
  [--progress TEXT] \
  [--next-step TEXT]... \
  [--file PATH]... \
  [--source PATH]... \
  [--conflict UUID]... \
  [--transition STATE] \
  [--reason TEXT] \
  [--json]
```

Performs a required compare-and-swap revision update, verifies that the actor
is in `authorized_owners`, validates type-specific lifecycle transitions,
refreshes source fingerprints, appends transition history when transitioning,
and rebuilds indexes. At least one field or transition is required. The default
reason is `Record updated`.

`--revision auto` resolves the current revision inside the runtime mutation
lock, so an automated writer still performs a real compare-and-swap rather than
omitting the check.

Events are content-immutable: they may only transition from `recorded` to
`superseded`. The parser accepts any transition string; the runtime validates
it against the record type and current status.

### `brain-get`

```bash
python3 memory-bank/scripts/context.py brain-get \
  --record-id UUID_OR_EXTERNAL_ID \
  [--json]
```

Reads and validates a governed dynamic record. It searches active records by
UUID or external ID; it is not an archive-query interface.

Side effects: SQLite initialization only.

### `status`

```bash
python3 memory-bank/scripts/context.py status [--json]
```

Reports local database counts and selected mode/authority. In governed mode,
`working` counts local task bindings, not all active Brain task files. A zero
count may mean the current machine has no binding even when Git contains active
tasks; `rebind` restores such a binding.

Side effects: creates or migrates the local database schema if necessary.

### `validate`

```bash
python3 memory-bank/scripts/context.py validate [--json]
```

Validates active and archived Brain records and control data, including strict
schema keys, UUID/path consistency, legal status history, source freshness,
relationships and supersession backlinks, task/handoff alignment, manifest
ceilings, promotions, and deterministic indexes.

It reports errors; it does not repair them. Exit status is `0` only when valid.
Side effects are limited to SQLite initialization.

### `parity`

```bash
python3 memory-bank/scripts/context.py parity [--json]
```

Fails when mirrored skill implementations drift from the configured
`canonical_edition`. Edition orchestration catalogs are intentionally outside
this parity comparison.

Side effects are limited to SQLite initialization.

### `compact`

```bash
python3 memory-bank/scripts/context.py compact [--json]
```

Validates first, then atomically moves terminal or superseded dynamic records
from `project-brain/dynamic/` to type-specific `project-brain/archive/`
directories. Related handoffs move to `archive/handoffs/`. It rebuilds active
and archive indexes, validates again, and restores the snapshot if a move or
post-validation step fails.

Compaction moves history; it does not delete it. Current terminal states are:

- tasks: `completed`, `cancelled`;
- findings: `resolved`, `superseded`;
- bugs: `resolved`, `cancelled`;
- incidents: `closed`, `cancelled`;
- decisions: `rejected`, `superseded`;
- events: `superseded`.

Incident `resolved` and decision `accepted` are intentionally not terminal.

### `promote-propose`

```bash
python3 memory-bank/scripts/context.py [--owner PROPOSER] promote-propose \
  --source-id UUID \
  [--source-id UUID]... \
  --title TITLE \
  --content CONTENT \
  [--json]
```

Creates a Git-tracked proposal under
`project-brain/control/promotions/`. Each source is bound to its exact type,
path, UUID, and revision. The source may be active or archived.

Propose only a verified, reusable consequence that is not already better owned
by a specification or other canonical source. Proposal does not approve or
write durable memory.

### `promote-review`

```bash
python3 memory-bank/scripts/context.py promote-review \
  --promotion-id UUID \
  --reviewer HUMAN_ID \
  [--reject] \
  [--json]
```

Records independent human review. Without `--reject`, the review approves the
proposal; with it, the proposal becomes rejected. The runtime rejects a
reviewer equal to the proposer. Agents must not invent or impersonate a human
reviewer and must stop at proposal unless explicit human review has occurred.

Side effects: updates the Git-tracked promotion record and revision.

### `promote-apply`

```bash
python3 memory-bank/scripts/context.py promote-apply \
  --promotion-id UUID \
  [--json]
```

Requires an approved reviewed proposal. At apply time it rechecks each exact
source type, path, UUID, and revision. It then mints a conflict-free Memory
Bank ID — `MEM-YYYYMMDD-xxxxxxxx`, the promotion date plus eight hex
characters of the source record's UUID — writes one chunk, regenerates
`memory-bank/INDEX.md` from chunk frontmatter, validates the bank, and marks
the promotion applied with its destination ID and revision. Because no shared
counter is involved, promotions on different machines or branches cannot
allocate the same ID; merging two branches reduces to a union of chunk files
followed by `reindex-bank`. The legacy `.memory-counter` file is neither read
nor written (it may remain on disk; the validator ignores it), and legacy
`MEM-0001`-style chunks keep their IDs.

All promotion writes are snapshotted and restored on failure. The generated
chunk currently uses the runtime's fixed promoted-memory defaults; use the
normal Memory Bank capture workflow when a proposal requires more nuanced
categorization or an update/supersession decision.

### `retrieval-report`

```bash
python3 memory-bank/scripts/context.py retrieval-report \
  [--since N] [--scope local|governed|all] [--json]
```

Aggregates what the retrieval manifests recorded. Until it existed the
manifests were written and never read: every number a gate decision needs
lived for one turn and then only on disk.

`--since N` counts **records, not days** — the N most recent manifests by
creation time. Lightweight mode writes no manifests at all, which the command
reports as its own fact rather than as an empty aggregate.

Two reporting choices worth knowing. Turns are broken down by `query_source`,
because a prompt, a task, a bare branch name and an operator's query are not
comparable and an undifferentiated average of them says nothing. And the top
score is reported **twice**: `top_candidate_score` from the gate signals,
taken before any policy filter, and `top_delivered_score` from the selection.
Those diverge exactly when the best match was withheld — the case a report
like this exists to surface — so the command does not pick one for you.

Manifests written before schema version 2 carry no gate, no phase timings, no
query source and no per-item score. They are counted, and each metric reports
how many manifests could answer it, rather than silently averaging over the
subset that can. Path counts come from the manifest's selection, which is
recorded before the capsule's character ladder may drop an item, so they are
an upper bound on what the model was shown.

The report never contains the query text, in either output mode.

### `health`

```bash
python3 memory-bank/scripts/context.py health [--window N] [--json]
```

Aggregates `memory-bank/local/refresh-health.ndjson`, one record per turn,
bounded by `refresh_health_retention` in `runtime.json` (default 500, floored
at 1). Reports p50/p95/max per phase, the timeout rate, the share of turns
whose capsule dropped content, and the share where not every layer refreshed.

It reads only the health series, never the manifests: the manifest's
`retrieval` phase times the body of `retrieve()`, while a health record times
the whole refresh including the episodic slot and the capsule contract. Those
are different intervals and averaging them under one name would compare two
things.

A record is appended by `refresh`, by `hook-context` (Cursor's read path never
enters the refresh branch, so without it that client would be invisible), and
by the prompt hook itself — the hook writes its own line carrying only an exit
status, because on a timeout the Python process is killed before it can write
anything, and that is the one case most worth measuring. Records carry no
query text and no selected paths.

### `status` consolidation counters

`status` reports a `Consolidation:` line and a `consolidation` object:
promotable candidates, blocked candidates, applied promotions, and durable
chunks. It exists because "nothing was ever consolidated" and "consolidation
ran and had nothing to do" produced identical output — which is how a fully
built promotion pipeline received no input for nine days without anyone
noticing.

Each count is `null`, and prints as `unavailable`, when it cannot be
established: in lightweight mode, without a `project-brain/` directory, or if
the walk fails. `null` and `0` are deliberately different answers — an absent
Brain is not an empty one.

The counters walk `project-brain/` and hash each cited source, so `status`
costs more than the SQLite counts it used to be. It is also what the
session-start hook calls; the walk degrades to `null` rather than failing, but
on a very large Brain expect `status` to take proportionally longer.

### `bank-audit`

```bash
python3 memory-bank/scripts/context.py bank-audit [--json]
```

Read-only. Reports, for active chunks only: `source_changed` (a cited local
file whose digest no longer matches, or that is gone), `overdue_review` (past
`review_after`), `undigested` (cites local files but records no digests, which
is every chunk written before digests existed), and `duplicates` (pairs of
active chunks whose text overlaps at or above `bank_duplicate_ratio`). A
duplicate pair is a decision for a human: update the survivor, or close the
older one with `bank-retire`. The engine never merges chunks by itself. A
retired chunk is not audited — it has nothing left to attest to, and its dead citations would bury
the live ones.

### `bank-reverify`

```bash
python3 memory-bank/scripts/context.py bank-reverify --id MEM-... \
  [--review-after YYYY-MM-DD] [--json]
```

Re-attests an active chunk against its sources as they are now: recomputes
`source_digests` for every local path in `sources`, stamps `last_verified`
with today, and sets the next `review_after` (default one year). Runs under
the same transaction as `bank-retire` — lock, snapshot, regenerated index,
whole-bank validation, restore on failure.

Running it is an assertion by whoever ran it that they re-read the sources and
the chunk still holds; the command records that judgement, it does not make
it. If the chunk no longer holds, retire it instead. Only active chunks can be
re-verified, so this cannot quietly resurrect retired knowledge.

Without this command the digests would be a one-way ratchet: the first time a
cited file changed, the chunk would leave retrieval and stay out, because
nothing else writes chunk frontmatter and hand-editing is what these commands
exist to replace.

A URL in `sources` is cited but never digested — the runtime has no network,
so the only honest thing it can say about a remote citation is nothing.

### `bank-retire`

```bash
python3 memory-bank/scripts/context.py bank-retire \
  --id MEM-20260101-a1b2c3d4 \
  --valid-to 2026-08-22 \
  [--superseded-by MEM-20260815-e5f6a7b8] [--reason "..."] [--json]
```

Closes a durable chunk's validity period as one transaction. `--valid-to` is
the date the knowledge stopped being true and is what removes the chunk from
retrieval; `--superseded-by` names what replaced it. With a successor the
chunk becomes `superseded` and the successor's `supersedes` list gains it in
the same write; without one it becomes `archived`, the honest status for
knowledge that ceased with nothing taking its place.

Use this rather than editing frontmatter. Retiring by hand means editing both
sides of a replacement link and then regenerating the index, and between any
two of those edits the bank is invalid. Every write here happens under the
mutation lock against a snapshot, `memory-bank/INDEX.md` is regenerated, and
the whole bank is validated; any failure restores every touched file, so an
interruption leaves the bank exactly as it was.

What does not change: the chunk body, its other frontmatter fields, the file
on disk, and its `INDEX.md` row, which survives with the new status — a chunk
on disk without an index row is itself a validation error. What leaves is
retrieval, and only at the next `index`/`refresh`, which reports the chunk in
`excluded` with its new status as the reason. `--reason` is recorded in the
command's output only: the chunk schema is a closed key set, so writing a
reason into the frontmatter would fail validation.

Because the whole bank is validated before the write is accepted, a
pre-existing error in an unrelated chunk will roll a valid retire back. Fix
that error first, or retire the offending chunk — a terminal chunk whose cited
source was deleted downgrades from error to warning, so retiring it clears the
failure.

### `reindex-bank`

```bash
python3 memory-bank/scripts/context.py reindex-bank [--json]
```

Regenerates `memory-bank/INDEX.md` deterministically from chunk frontmatter:
rows are sorted by ID (legacy numeric IDs first in numeric order, then
date-based IDs), and any prose above the table header is preserved. The
command is idempotent — rerunning it against unchanged chunks rewrites
nothing (`"changed": false`) — and reconstructs a deleted or conflicted index
in full. Run it after manually creating or updating a chunk, and after a
merge that brought chunk files from another branch. Promotion applies it
automatically.

### `export`

```bash
python3 memory-bank/scripts/context.py export \
  --destination ../context-bundle \
  [--include-archive] \
  [--include-superseded] \
  [--force] \
  [--json]
```

Writes a point-in-time, privacy-filtered copy of this installation's Project
Brain records and Memory Bank chunks into a directory, for handing accumulated
context to another person or repository. It reads only; nothing in the
installation changes.

What is included:

- Project Brain records whose `privacy` is in the configured `allowed_privacy`
  (`public` and `team` by default). `restricted` and `private` records never
  leave. Add `--include-archive` for archived records.
- Memory Bank chunks with status `active` or `needs-review`. Add
  `--include-superseded` for the rest.

The bundle contains `MANIFEST.json` and `README.md` alongside the copied
files. The manifest lists every included item with its ID, type, revision, and
cited sources, **and every excluded item with the reason it was excluded** - a
bundle that quietly dropped records would read as a complete one. It also
records the source commit and whether that installation had
`automatic_promotion` enabled, and flags each chunk that was promoted
automatically and therefore never human-reviewed.

Two fail-closed behaviors:

- A destination that already contains files is refused unless `--force`.
- If any selected record or chunk matches a secret pattern, the entire export
  aborts before writing anything. The offending path is named; its content is
  not echoed. Remove the secret from the record and re-run.

A bundle is a handoff artifact, not a second installation. Project Brain
records carry the originating `owner` and `authorized_owners`, so a recipient
cannot mutate a copied record under their own identity - they should read the
bundle as history and open their own records for active work. There is no
`import` command; adopting a bundle is a deliberate, manual act.

## AI Skills: Memory, Checkpoint, Project Brain, and Memory Bank

These names describe AI workflows, not additional shell executables:

- `memory` takes no arguments. In governed mode it validates Project Brain,
  runs `refresh`, reports each memory layer as that command returned it, and
  stops. In explicitly configured lightweight mode it may perform the
  checkpoint procedure first. It uses the same `refresh` the request hook runs,
  so the skill and the hook cannot drift apart, and it never passes `--query`:
  `memory` reports layer health rather than retrieving. It never completes
  tasks, applies promotions, or invents state.
- `checkpoint` takes no arguments and never completes work. In governed mode it
  skips local working-state mutation and directs the caller to a governed
  revisioned update. Only in explicitly configured lightweight mode may it
  derive a local task ID from the current branch, inspect approved changed text
  paths, and save a sanitized local progress summary.
- `project-brain` operates shared task/record lifecycles, handoffs, retrieval,
  compaction, and promotion proposals through the CLI.
- `memory-bank` retrieves, captures, audits, supersedes, archives, or applies
  independently approved durable memory. It does not own active task progress.

Skills execute only when selected.

Session-start hooks stay informational and do not print record bodies. Cursor,
which has no prompt-submit hook, atomically refreshes its ignored always-apply
rule: before provisioning it contains only sanitized warming metadata; after
the fifth turn it is replaced by the governed capsule. A valid branch with no
task or changes removes foreign context, while runtime failures preserve the
last valid rule. The request and turn-end hooks do only what their halves
require:

- the request hook runs `refresh` and injects a bounded capsule; it writes no
  task state, because at prompt time nothing has happened yet to record;
- the turn-end hook runs `turn`, which buffers the change set and flushes a
  consolidated task update on a boundary.

With `automatic_promotion` enabled in `runtime.json`, the turn-end hook also
promotes eligible resolved, verified knowledge into durable memory without
review; such promotions name no reviewer and their chunks are tagged
`auto-promoted`. Reviewed mode remains independently propose → review → apply.
The turn boundary may report merged branches as sanitized completion
candidates, but it never closes a task. Shipped `automatic_completion=false`;
explicit completion requires the current numeric revision and verification.
With `automatic_compaction` the turn boundary archives terminal records once
`compaction_threshold` of them accumulate, repointing any promoted Memory Bank
chunk at the record's new archive path as it goes.

See [Context and Memory](CONTEXT-AND-MEMORY.md#hooks-and-explicit-actions) for
the reasoning behind the read/write split.

## Failure and Recovery

### Stale revision

Symptom: `Stale ... revision: expected N, current M`.

1. Run `get --task-id ID --json` or `brain-get --record-id ID --json`.
2. Review the newer progress, transition history, files, sources, and handoff.
3. Reconcile your intended change with the current record.
4. Retry using revision `M`. Do not simply substitute the number without
   reconciling content.

### Owner not authorized

Symptom: `Owner is not authorized to mutate record`.

Use the same explicit owner that created the record or an identity already
present in `authorized_owners`. `runtime.json` retrieval owners do not grant
mutation rights. The public CLI has no command to add an authorized owner; do
not hand-edit the record casually. Escalate the ownership transfer for reviewed
repository maintenance.

### Missing governed task binding

Symptom: `Working task not found` even though a Brain task exists in Git.

The governed compatibility commands resolve through the machine-local SQLite
binding. Pulling Project Brain files on another machine does not recreate that
binding by itself. Two supported recoveries exist:

1. `rebind --task-id ID [--record UUID]` restores the binding explicitly after
   you verify the task with repository records and `validate`.
2. Doing nothing: the first automated flush (`turn`) reconnects to the
   existing record on its own and reports the fact in the turn result, the
   last-turn report, and the next capsule's `Last turn` section.

Do not run `start` with the same external ID: duplicate Brain records are
rejected, and deleting the database does not remove the shared task.

### Stale source fingerprint

`validate`, indexing, or retrieval may report `stale` after a cited source
changes.

1. Open the current canonical source.
2. Decide whether the Brain claim remains valid, changed, or conflicted.
3. Update the governed record through a revisioned supported mutation so source
   fingerprints are recomputed.
4. Re-index and retrieve again.

Do not suppress freshness checks or copy old source content into the record.

### Invalid UTF-8 or secret-like content

Indexing aborts on invalid UTF-8 for otherwise eligible source documents and
skips likely secret-bearing candidates. Mutating commands reject likely secret
patterns. Remove sensitive material from governed/local records, keep secrets
in proper secret systems, and retry. Do not print suspected values while
diagnosing.

### Validation or parity failure

Do not compact, promote, or claim a healthy handoff. Correct the authoritative
record/source or mirrored skill through normal reviewed maintenance, rebuild
using supported mutations, and rerun all health checks. `validate` does not
repair stale deterministic indexes.

### Interrupted or cross-store write failure

Supported mutations use file snapshots, SQLite transactions, atomic file
replacement, and a repository-wide lock. The runtime attempts rollback, but a
process kill, disk failure, or manual edit can still leave uncertainty. Preserve
the working tree, run `status`, `validate`, Memory Bank validation, and inspect
the relevant task/promotion and local binding before retrying. Do not repeat a
mutation blindly.

### Damaged or deleted SQLite database

Stop all processes using the database. Preserve a copy if forensic recovery
matters, then recreate the ignored database by running `index`.

Rebuildable: document and metadata indexes. Governed compatibility bindings
are restorable from the Git-tracked records with `rebind` (or by the next
automated flush; see
[Missing governed task binding](#missing-governed-task-binding)).

Not rebuildable from canonical sources: lightweight working tasks and local
episodes. Project Brain records, handoffs, manifests, promotions, and Memory
Bank chunks remain in Git.

## Maintenance

Recommended routine:

```bash
python3 memory-bank/scripts/context.py validate
python3 memory-bank/scripts/context.py parity
python3 memory-bank/scripts/validate.py
python3 memory-bank/scripts/context.py index --json
python3 memory-bank/scripts/context.py status --json
```

Periodically:

- review `index --json` exclusions and parity drift;
- reconcile stale source fingerprints;
- complete or cancel abandoned governed tasks deliberately;
- audit active Memory Bank chunks for changed sources and review dates;
- review pending/rejected promotions;
- compact after validation when terminal records accumulate;
- keep `project-brain/config/runtime.json`, provider, telemetry, schemas, and
  runtime copies in reviewed parity across accelerators.

Do not delete retrieval manifests merely to reduce noise without an explicit
retention decision. The current `compact` command archives dynamic records and
handoffs only; it does not compact manifests, promotions, local episodes, or
Memory Bank chunks.

## Continuing on Another Machine

For governed continuity:

1. Commit and transfer the intended Git-tracked Project Brain task, handoff,
   indexes, manifests, promotions, and Memory Bank changes through the normal
   review workflow.
2. On the new machine, select the same accelerator and pull those files.
3. Set the same authorized owner identity.
4. Run `validate`, `parity`, Memory Bank validation, and `index`.
5. Read the task/handoff and verify cited canonical sources before continuing.
6. Restore the local binding: `context.db` is ignored and is not transferred,
   and the compatibility task commands resolve through it. Run
   `rebind --task-id ID` (optionally with `--record UUID` for an explicitly
   reviewed target) — or simply keep working: the first automated flush
   reconnects to the existing record on its own and reports that it did.

Do not copy `context.db` between machines as shared authority. Besides being
ignored and environment-specific, it may contain local-only tasks and episodes.
`rebind` rebuilds everything the governed workflow needs from the Git-tracked
record.

Lightweight work has no guaranteed another-machine continuity. Export its
verified outcome into an appropriate canonical task document, specification,
or governed record before leaving the machine if losing it is unacceptable.
