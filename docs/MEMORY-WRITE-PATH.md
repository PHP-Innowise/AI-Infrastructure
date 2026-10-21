# Memory write path: consolidation, provenance and deterministic facts

The retrieval work on 2026-10-19 raised the delivered-document precision on the
12-prompt stand to 21/31 useful, 10/31 noise, 11/12 useful turns. The remaining
noise was not separable by another ranking filter: every candidate gate traded a
measured useful document or failed a contract test. This document describes the
next layer - what memory **accepts and keeps**, rather than what it hands over.

## The problem

`Project Brain` in the evaluated project held **28 events, 2 findings, 0
decisions and 5 durable chunks**. Completed work therefore lived mostly as an
append-only event diary; the capsule had to choose one event beside one
changelog entry, and the rest was invisible. (A request that names a ticket now
lets two of that ticket's events share the history layer; the write path below
is what makes that a conclusion rather than a longer diary.) Retrieval can surface a diary
faithfully, but it cannot turn it into conclusions.

Three write-path gaps follow:

1. a ticket's series of completed events never becomes one reusable finding;
2. a record or chunk promoted automatically can carry an external source or
   instruction-like text, and nothing in the automatic path stops it;
3. two statements of one fact have no deterministic tie-breaker at the write
   boundary, so the model is left to blend them.

## 1. Consolidate a ticket's completed events

```bash
python3 memory-bank/scripts/context.py consolidate --ticket CMS-741          # dry-run
python3 memory-bank/scripts/context.py consolidate --ticket CMS-741 --apply
```

The turn boundary runs the same pass when a project sets
`automatic_consolidation` in `runtime.json`. It is off by default and in the
shipped configuration (see "Why it is off by default" below); the command is
the explicit replay. It groups the operator's own **recorded events whose title
says `Completed`** by the ticket the title names - `Completed CMS-741: ...` -
and, for a ticket with at least two of them, creates one `finding`:

- `external_id` = `CONSOLIDATED-<ticket>`;
- title and `goal` from the latest event's descriptive title (what follows
  `Completed <ticket>:`), falling back to its outcome's first sentence;
- `progress` carries **every** event on a line of its own - what was done,
  the outcome and the event's `Verification:` - ending with the event's UUID,
  then the external ids and UUIDs of the ticket's other findings;
- `sources` = the union of the events' **local, existing** sources (remote URLs
  are not fingerprinted, so they are left out rather than making the new record
  unvalidatable);
- status `resolved`, authority `observed` - mechanical consolidation is not a
  verification. With `automatic_consolidation_verification`, it becomes
  `verified` only when every source event recorded a `Verification:` line
  (counted per event: two events that both say "unit tests passed" are two)
  and at least one local source was fingerprinted; the authority transition is
  agent-attested, and the same boundary's automatic promotion may then carry
  the summary into a durable chunk.

Retrieval serves no resolved finding, so the events are **not** superseded
when the finding is written: they stay `recorded`, and on a fresh clone they
are where their outcomes are found. Once the finding's current revision is an
active Memory Bank chunk, the next pass marks them `superseded` - the operator
is the actor, never the event's owner - and `compact` moves them. The first
version superseded the events at once, into a finding that carried only the
latest event's topic: on a fresh clone a question about an earlier event of
the series found nothing.

Events that reach the ticket later are added to its active `observed`
finding; a `verified` finding takes only events that recorded their own
verification, as a revised claim the agent attests again. A `CONSOLIDATED-*`
finding stays in `dynamic/findings` until it is verified, even when automatic
compaction runs on the same boundary; once it is archived, a new finding takes
the ticket's later events. One pass reads the archive and the promotion ledger
at most once, and not at all when every ticket's active finding already covers
its events - the earlier pass read the archive once per ticket on every flush
(0.58 s at 40 tickets and 2,000 archived records).

Only the operator's own events are folded in: another owner's are theirs to
supersede. A private or restricted event is never folded into the team-visible
finding. If the ticket already has a different **resolved** finding,
consolidation creates no second conclusion and leaves the events recorded: that
finding does not carry their text. An open or investigating finding is a
specific work item, not a conclusion, so it does not suppress the summary.

The finding cites the events' canonical sources, not the event files, so a
later `compact` cannot turn the durable conclusion stale by moving its cited
events.

A ticket here is an uppercase prefix, a hyphen and a number. "PHP 8.2" (a
space) and a standard's number - `UTF-8`, `PSR-4`, `SHA-256`, `ISO-8601` - are
not tickets: the first version grouped two unrelated events under "UTF-8".
Retrieval uses the same rule, and, once the index has seen the prefixes the
project files its work under (its tasks' IDs and the tasks its completion
events name), counts only those.

### Why it is off by default

The pass rewrites the team's tracked Project Brain on its first flush - 27 of
28 events on the evaluated project - and a verified summary becomes an
agent-attested chunk no person reviewed. The redesigned pass above has
deterministic tests but has not been measured on that project, so a project
turns it on deliberately (`automatic_consolidation`, and
`automatic_consolidation_verification` for the verified path) rather than
receiving it with an update.

## 2. Provenance and poisoning

Automatic promotion now refuses a record when:

- any cited source is `http(s)://` (external material needs a person's review),
  or
- the title or promoted content matches an instruction-like pattern
  (`ignore … previous instructions`, `system prompt`, `<|im_start|>`, …).

The reviewed path is unchanged: a person can still review and promote such a
record. The gate is about what the machine does without a reviewer.

Two operator commands expose the state:

```bash
python3 memory-bank/scripts/context.py bank-audit --provenance --json
python3 memory-bank/scripts/context.py bank-quarantine --id MEM-... --reason "suspected poisoning"
```

An `auto-promoted` chunk that reads like an instruction is withheld from
retrieval automatically, since no person read it; the same chunk is still
visible in `bank-audit --provenance`. A remote citation is **not** withheld by
itself: a pinned source URL is legitimate evidence, and withholding every one
of them lost a measured useful chunk. A manually reviewed chunk is never
withheld this way.

`bank-audit --provenance` adds:

- `auto_promoted`: active chunks carrying the `auto-promoted` tag, with their
  creation and verification dates;
- `external_sources`: chunks citing a remote URL;
- `instruction_like`: chunks whose title or body matches the instruction
  patterns.

`bank-quarantine` closes a chunk's validity **today** and archives it, so it
stops being served immediately; the existing `bank-retire` with
`--superseded-by` remains the way to name a replacement. The chunk schema is a
closed key set, so the reason is reported in the command output, not written
into the frontmatter.

This follows the memory-poisoning threat model: memory is a persistence layer
for an attacker, and a durable chunk is a higher-value target than a prompt
injection that disappears with the session.

## 3. Cross-language technical queries

The evaluated project's real prompts include Russian questions while the indexed
source is English. Three of them previously selected nothing. `query_tokens`
now carries a small seed glossary for common software terms (`путь` → path,
`лог` → log, `отбрасывание` → drop, `свойства` → properties, `сущности` →
entity, `срок` → ttl, `просмотр` → preview, …). It adds the English term
**beside** a Cyrillic word, which stays in the query, so a Russian document is
still found by its own words; the first version replaced the word and lost
them. Each entry matches the whole word form: a bare prefix took `логика` and
`логин` for log, `старт` for legacy, `действительно` for validity and
`полезно` for field. English text, identifiers and tickets pass through
unchanged. On those three prompts the same corpus returned 2–3 semantic
documents and the changelog, including the exact TTL-preview chunk for one of
them. The 12-prompt English stand is unchanged.

The glossary is a bridge, not a translation service: it is deliberately small,
local, dependency-free and extendable in one table.

## 4. Deterministic fact consolidation

```bash
python3 scripts/memory_fact_consolidation.py --input facts.json --json
```

The tool groups candidate facts by `fact`, takes the maximum explicit
`serial`/`version`/`revision`, breaks an equal version on `updated_at`, and
reports an **ambiguous** group when both are equal instead of picking a path or
an ID arbitrarily. It is the local implementation of the 2026 finding that the
bottleneck in fact-conflict resolution is the assembly step, not the storage
engine: candidate extraction plus a Python maximum beat LLM-judged freshness.
The known-answer fixture is
[`tests/fixtures/fact-consolidation.json`](../tests/fixtures/fact-consolidation.json).

It is deliberately **not** wired into the prompt-time capsule. Two records that
declare each other in Project Brain `conflicts` are shown together by contract
and marked `match: conflict`; that is a disagreement the reader must see, not a
version to collapse. When both partners are selected, the capsule JSON also
carries `conflict: [partner paths]`, so the reader can name the other side
without guessing from the mark. A restatement carries an explicit version or
timestamp and belongs at the consolidation boundary.

## Dry-run observation

On the evaluated project's 28 events, 27 were titled `Completed` and named one
of eight tickets. `consolidate --json` planned **seven findings covering 27
events**; `CMS-748` had a single event and stayed `skipped`. On a copy with the
cited local sources present, the first version's `--apply` created those seven
findings, superseded 27 events, blocked none, and `validate` passed - and lost
every outcome but each ticket's latest from retrieval, which is what the
current rules prevent. The dry run was never applied to the original project,
and the current rules have not been run on it.

The first copy exposed a title-quality defect: a latest goal that is literally
`completed Verification: ...` produced titles like `CMS-744: completed
Verification: ...`. Consolidation now takes the descriptive part of the latest
event title after `Completed <ticket>:`, skips auto-provisioned or ticket-led
subjects, and falls back to the goal. The same copy then produced titles such
as `CMS-744: Remove legacy unsigned ACL headers and preserve SDK presigner
behavior` and `CMS-741: Read-only ten-agent review of current CMS-741 invalid
to-many reference handling`.

## Why not a section-level index (yet)

The planned next lever was an FTS5 section index so an excerpt could quote a
better section of a selected document. The 12-prompt baseline disproved the
premise: **every delivered answer-bearing document already yielded its answer
passage** (`answer_passages_delivered == answer_passages_delivered_deliverable`
on every turn except the one noise-only turn, where no answer document was
delivered at all). The 15/37 recall came from answer-bearing documents that
were never selected, not from a section inside a selected document being
chosen badly. A section index can only reorder or replace sections **within**
a delivered document; it cannot raise that ceiling without changing document
selection.

The measured lever was the history layer instead: when the request names a
ticket, the two best events carrying it share the event slot. On the same
12-prompt stand that delivered **18/37 answer passages (48.6%) instead of
15/37 (40.5%)** and **26 judged-useful documents instead of 21**, with noise
unchanged at 10 and no unjudged documents after the two new events were graded
1 (same ticket / same subject). Without a ticket the single-event behavior is
unchanged. A section index remains a possible
future experiment for a corpus where long documents hold several answer
passages, but the current stand does not justify it.

## 5. Optional local vector pool

`scripts/memory_sqlite_vec_pilot.py` probes local `sqlite-vec`/`sqlite-lembed`
assets and fuses a dense path list with the lexical one through RRF. It is
maintainer-only and fails closed; the runtime never imports it. See
[the pilot doc](MEMORY-SQLITE-VEC-PILOT.md).

## What is not claimed

- Consolidation is not a verification: a finding is `verified` only when the
  project enabled `automatic_consolidation_verification` and every event it
  covers recorded its own verification; otherwise it stays `observed` and is
  never promoted. Promotion still requires `verified` authority and fresh
  sources, and only a promoted finding lets its events leave the diary.
- No retrieval filter was added in this layer. The remaining 10/31 noise
  documents on the 12-prompt stand are unchanged.
- The feature tests are deterministic unit/integration tests. The end-to-end
  effect on a real project's event count, chunk count and capsule composition
  has not been measured; that measurement is deferred with the rest of the
  testing.
- The human review packet remains ungraded by a person; the adjudicated labels
  are an explicit-rule agent set, not an external check.
