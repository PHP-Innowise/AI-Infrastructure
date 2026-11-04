# Memory write path: consolidation, provenance and deterministic facts

## Explicitly rechecking existing knowledge

`record-result`, `native-result` and `memory_record_result` accept an existing
learning reference alongside ordinary new learnings. After checking the claim
against its current sources, preserve its original type, title and complete
source citation set, and provide **all three** fields:

- `record_id`: the existing Brain finding/decision UUID, not its MEM identifier;
- `expected_revision`: the numeric revision actually inspected;
- `source_fingerprints`: the complete checked file image returned by
  `memory-bank/bin/infra-engine memory source-fingerprints --source PATH --json` or the read-only
  `memory_source_fingerprints` MCP tool. Text hashes normalize CRLF to LF;
  reading hashes alone does not verify a claim.

For example, a checked learning can include:

```json
{"type":"finding","title":"Reservation rounding","consequence":"Reservations round upward to four slots.","sources":["src/Layout.php"],"record_id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","expected_revision":2,"source_fingerprints":[{"path":"src/Layout.php","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}]}
```

These are placeholders, not evidence; use the inspected record and real runtime
hashes. The enclosing result still requires `verified: true`. A reference must
be an authorized, allowed-privacy, verified terminal finding/decision. Agents
cannot re-attest person-verified or unknown-attestation claims. Changing its
title or source set is a separate explicit maintenance workflow, not inferred
from similar wording. Missing references never fall through to record creation.

A confirmed source change updates the same Brain record and its agent/person
verification ledger under the shared mutation lock and original CAS. It stores
the supplied checked image; a concurrent later source edit remains visibly
stale instead of acquiring an unperformed verification. An already identical
claim and checked image are a record no-op. Conflicting aliases targeting one
record are refused before effects; independent protected learning conflicts
remain reported while unrelated progress can save.

Automatic promotion, when enabled, creates a successor for changed checked
canonical evidence even if the consequence is unchanged. The predecessor body
is preserved, validity closed, and both supersession links recorded using the
existing rollback-protected promotion operation. Human-reviewed, inactive,
manually edited or malformed predecessors require review. Promotion failure is
reported separately from a saved Brain revision; exact result replay can retry
promotion without re-attesting newer sources. A completed result is otherwise
passive; a genuinely new check needs a new result ID.

New promotion proposals additionally retain source_verifications: a hash of each
source record's explicit verification ledger. This identifies a new check
without ordering clocks on different machines. It also permits a safe successor
when bank-reverify already refreshed the inherited file hashes before the Brain
record was confirmed. Ordinary metadata edits do not change that check identity.
Older proposals remain readable: changed checked canonical evidence still works,
but equal-image legacy records without a saved check identity require explicit
review rather than a wall-clock inference. Older strict runtimes must be updated
before reading newly written proposals containing this optional field.

Rollback attempts every saved destination before reporting a restoration error.
An unavailable index must not prevent restoring the predecessor after a failed
successor write. A failed unfinished result cannot use absent progress fields as
proof that it already updated a task whose revision changed meanwhile.

### Interrupted promotion publication

Before writing a successor, changing its predecessor or regenerating the index,
the reviewed promotion retains an optional bounded `publication` checkpoint.
It binds the original proposal, source metadata, frozen publication date and
exact before/after file images. Replaying the same result resumes or rolls back
these owned effects without another source attestation or Brain/task revision.
Completed publication clears the checkpoint atomically with the applied ledger.
Failed rollback retains it until all owned effects and the index are repaired.
The index is regenerated from current chunks so an unrelated valid publication
is preserved. Unknown edits, corrupt checkpoints, linked paths and unavailable
predecessors stop recovery rather than authorizing overwrite or adoption.

Publication uses held directory handles and bounded nonblocking reads. A new
destination requires an atomic exclusive rename: Linux `renameat2`, macOS
`renameatx_np`, or the installed Windows relative-handle backend. Missing host
or filesystem support refuses publication; no plain replacement or hard-link
fallback is used. The first save may create a missing chunks directory through
the same rooted operations. Native platform execution is still required to
establish support on that platform; Linux tests and mocked ABI tests do not
provide native Windows or macOS evidence.

All supported writers must honor the shared mutation lock. Existing-file
replacement/removal checks the expected image immediately before the native
operation; this is point-in-time custody, not an atomic content CAS against an
arbitrary external editor writing inside that final interval. Pause external
edits during promotion/recovery. A checkpoint's hashes establish consistency,
not the truth of its claims or protection from rewriting the authority itself.
Read-only inspection may omit malformed legacy proposals; write admission is
strict and cannot treat a damaged checkpoint as an absent proposal. Older
strict runtimes must be updated before consuming the optional checkpoint.

Harness preserves reference fields in prepared bindings and replay. An older
create-only runtime holds them as unsupported rather than silently turning
them into new findings. No automatic inference, consolidation, retrieval
profile or truth-assessment default is enabled by this API.

## Offline lifecycle pilots (2026-10-31)

Retirement has its own bounded recovery checkpoint at
`project-brain/control/bank-retirement.json`. It preserves the exact chunk
images and allows only retirement metadata changes. Before checkpoint removal,
process death is recovered as rollback on the next cooperative mutation; after
removal the retirement is committed. Foreign chunk edits stop recovery, while
the derived index is regenerated from the restored bank. This mechanism neither
re-attests knowledge nor provides an atomic view to unlocked readers. See
[the operational procedure](OPERATIONS.md#bank-retire).

These maintainer-only tools use native Rust services and never install a
provider or change a client's configuration. They are local implementation and
measurement infrastructure, not evidence of improved task outcomes. Both
automatic consolidation defaults remain off. Live held-out acceptance follows
`docs/MEMORY-EVAL.md`; first-touch and relevance improvements cannot enable them.

### Consolidation on a copy

```bash
infra-engine eval memory-consolidation-pilot --root /private/project-snapshot --variant consolidate-verify --out /private/consolidation.json --export /private/new-treatment
```

Variants are `baseline`, `consolidate` (verification off), and
`consolidate-verify`. The tool makes a physical bounded copy, excludes Git
administration, credentials, attachment launchers and local caches, and never
executes runtime code from the input. Public canonical skill/policy citations
remain as data. A fresh isolated native worker invokes the trusted canonical
`auto_consolidate → auto_promote → auto_compact` for three boundaries. It checks
its roots before mutation and verifies the input digest afterwards.

The report includes step counts, event→finding revision→chunk mappings,
validation and byte-idempotence of the last boundary, input/runtime dependency
digests and, when exported, the actual export digest. The export omits local
locks/caches. New UUIDs and verification dates mean replay is a reproducible
recipe, not a promise of byte-identical fresh runs. Capture and seal one actual
treatment image for repeated comparisons. `task_outcome: not-measured` and
`human_reviewed: false` are intentional: mechanical folding and agent attestation
are not independent human review. Promotion failure cannot be treated as an
accessible durable replacement. Inputs over 20,000 regular files, 4 MiB per file
or 64 MiB total are refused rather than silently truncated.

### Current-fact projection

```bash
infra-engine eval memory-current-facts --root /private/project-snapshot --selectors /private/selectors.json --as-of 2026-10-31T00:00:00Z --json
```

The default source format remains **public JSON**, not free-text extraction. A
selector identifies a scoped fact and extraction pointers; values and version
metadata come from the source. For a source containing a `database` object:

```json
[{"id":"database-v2","namespace":"project/snapshot-frame","subject":"backend","predicate":"database","qualifiers":{"environment":"production"},"source":"specs/facts.json","pointer":"/database/value","recorded_at_pointer":"/database/recorded_at","lineage_pointer":"/database/lineage","revision_pointer":"/database/revision","valid_to_pointer":"/database/valid_to"}]
```

Use a namespace identifying the project and source frame; qualifiers distinguish
applicable environments/components. RFC6901 pointers support escaped keys and
scalar `false`, `0`, empty string and `null`. Metadata dates are aware instants,
normalized to UTC; validity is `[valid_from, valid_to)`, with JSON null meaning
an open end. Recorded-after-query facts cannot be backdated into the snapshot.
Resolution rechecks time even when called separately from collection.

Revisions order candidates only within an explicit shared lineage. An explicit
unresolved conflict remains ambiguous even when its partner was not selected.
Within one existing identity/scope group, eligible canonical JSON values outrank
Brain/Bank retellings before lineage comparison. Conflicts, stale evidence and
unknown candidates still apply across the complete history; precedence does not
remove those blockers or merge distinct Bank scopes.
Missing/unsupported/unsafe knowledge is unclassified; a digest mismatch is stale.
An unknown candidate does not permit a confidently selected old fact. History,
source digests and applicability survive in the output. Secret values and private
or linked paths are not echoed. Claims are bounded to 2,000 characters and
selector files to 1 MiB/1,000 selectors. No Brain/Bank schema changes or SQLite
table is introduced; deletion of this derived output loses no authority.

Opt-in selectors can also use `source_kind: "brain-frontmatter"` or
`"bank-frontmatter"`. Their `source` names an exact canonical finding, bug,
incident, decision or chunk path; `pointer` selects a scalar metadata value or
the exact `/body`. For example, select `/progress` from a resolved finding with
an explicit project namespace, subject, predicate and environment qualifiers.
Do not supply time, revision, conflict or provenance pointers for these adapters:
they derive those fields from the validated record and its checked origins.

Adapters intersect public/team and verified admission with project runtime
privacy/authority settings. Missing source pins, unsafe paths, unknown
attestation, malformed or linked configuration, and unsupported provenance are
unclassified. Bank tags do not establish human review; every origin must have
consistent complete original pins. Changed evidence remains stale even if a
chunk alone has been repinned. Agent and person attestations remain distinct;
metadata consistency does not authenticate its editor or establish claim truth.

Brain facts use the current record's update instant; a later edit cannot be
backdated to its creation. Bank day precision is conservative: recorded knowledge
is available after that UTC day, while validity/review end dates include their
day. Canonical validation also uses today's review dates, so overdue Bank chunks
can remain unclassified for historical queries. Bank scope is a separate grouping
key, and explicit conflicts remain ambiguous. Limits are 4 MiB per read, 64 MiB
total, 1,000 read images and 100 citations per record; `adapter_limits` reports
them. Reads are cached only within one call. This is an offline projection and
is not connected to the installed capsule.

### Experimental ordinary-session maintenance

`native_memory_maintenance: "encountered"` in the disposable project's
`project-brain/config/runtime.json` enables bounded encounter hints. The default
is `off` (an absent or unknown value is off). The optional
`CONTEXT_NATIVE_MEMORY_MAINTENANCE=encountered` environment override is useful
for isolated experiments. CLI `retrieve` and prompt `refresh`
remember up to three final returned semantic paths and their exact admitted
images in ignored `project-brain/local/native-maintenance/`. Hints are scoped to
the project, actor and external task ID, optionally pinned to an existing task
UUID, expire after one hour and are replaced by the next nonempty encounter
packet. This is a same-task discovery queue, not proof of session delivery,
reading or use. MCP-only retrieval and direct file reads do not populate it.

The normal native Stop save rechecks exact images, current privacy/owner
admission, terminal verified state and agent attestation. An active MEM may
point to a Brain record only through its exact stored source digest; a newer
unseen record is omitted. Only changed canonical-source images produce hints.
The hint preserves the original record ID/revision/type/title/source set and
asks for confirmation or correction **only when the original task already
checked the claim and current sources**. The agent reads checked fingerprints
and submits the existing reference contract; hashes alone never verify a claim.
Otherwise it omits the learning and saves ordinary progress. No extra research,
application edits or tests are started by the Stop step. Full and cached-short
save instructions both carry dynamic hints.

Missing, expired, unsafe, changed or corrupt packets omit hints without blocking
normal saves. The ignored packet is disposable; existing shared write admission,
CAS, promotion history and passive replay remain authoritative. A record that
ceased to apply or changed its source set requires the separate governed
retire/review workflow; no replacement is invented. This flag remains off until
independent completed-task evaluation on a real-project copy accepts it.

The 2026-10-31 prospective real-source-slice pilot did **not** accept this
mode. One native A session saved two correct claims; six fresh B sessions
compared available memory with/without maintenance and ablated memory across
unchanged/changed identifier behavior. All six completed patches passed the
private 157-assertion oracle. Both maintenance sessions received exact reference
hints in their native user continuation, but every B saved empty learnings:
no referenced update or fingerprint request was attempted. All four available
memory copies retained two active chunks with stale Address evidence. In the
changed case, the old case-preservation claim also contradicted current code.
Checking every cited dependency/importer source was not established, so this
does not prove instruction disobedience. It shows that these conditional hints
did not complete the maintenance cycle. Defaults remain off; no task-quality,
natural-delay or cost-saving benefit is established by this small controlled
pilot. Native saves and passive replay were healthy, while each B still retained
one pending turn observation; strict freshness remained unresolved.

### Experimental pre-answer review stage

`native_memory_maintenance: "review"` (or the isolated environment override)
adds a separate workflow footer to CLI retrieve/refresh output. After completing
application work and tests, the agent opens a review plan **before its final
answer**:

```bash
memory-bank/bin/infra-engine memory memory-review --task-id ID --json
```

This stage currently requires an existing authorized active task and up to three
safely admitted same-task encounter records. Missing tasks/unsafe encounters do
not provision a new task or claim a successful review. The plan writes only
bounded local metadata; it does not open SQLite or change Brain/Bank authority.
Its exact submit_command preserves the tooling/project context, including from
a subdirectory or attached layout.

Read each claim and the necessary current canonical sources, then explicitly
choose confirm, correct or defer for **every** offered record. Confirm/correct
require top-level verified=true, complete checked source_fingerprints matching
the plan and current source image; correct supplies a new consequence. Defer
requires a short safe reason and leaves shared knowledge and review dates alone.
The original type/title/source set is preserved; renaming a claim or changing
its source set is a separate workflow. Hashes and this API do not prove truth or
human verification. The JSON body has plan_id and decisions; use the returned
submit schema and exact command with --input -.

An explicit working-stage application uses the existing record_result guard
with progress=None and deterministic review result identity. It can revise the
selected claims and promote successors before the answer. It is not the closing
answer save: native Stop still records ordinary progress using its unchanged
wire/proof/replay path. Current task/actor/config/runtime/checkout and raw record
images are fenced under the mutation lock. The first accepted decisions are
frozen; incompatible later decisions are refused. Completed exact replay reaches
the canonical proof before checking expired plans or now-missing sources. Local
plan/status retention is bounded and required for this convenience command;
shared result proofs and authoritative knowledge remain separate.

States applied, held and deferred must be distinguished. Promotion failures do
not establish accessible replacement memory. A deferred item is unresolved, not
verified. The host instruction to work before the answer is cooperative:
existing Stop hooks run after a visible answer and cannot universally enforce
that timing. The agent may safely defer when evidence or title applicability is
insufficient. The mode stays off by default pending independent task evaluation;
it does not enable consolidation or a new retrieval ranking.

The 2026-10-31 prospective CSV source-slice pilot exercised this stage on a
different family from the earlier Address experiment. All six B patches passed
the independent 96-request oracle. Both review sessions applied an explicit
confirmation/correction before the final answer, retained record UUIDs, refreshed
source pins and preserved successor history. Full consequence accuracy was only
1/2: the correction generalized whitespace-only strings to empty strings and
lost the formfeed/NBSP exceptions to PHP's trim mask. Fresh hashes therefore did
not establish true knowledge. Both sessions had substantive progress messages
before review, failing the stricter sealed timing criterion even though review
preceded the final answer. One chained submission required supplemental literal
input/proof inspection because the sealed parser omitted its validation suffix;
raw primary results were kept unchanged. The pilot was not accepted and the
mode remains off. This is controlled development evidence, not a natural-delay,
task-quality or economic benefit result.

### Experimental independently verified proposals

`native_memory_maintenance: "review-verified"` keeps the same encounter and
explicit decision interface, but confirm/correct returns `proposal-staged`.
It writes local metadata only. The primary's closing native result must use
`learnings=[]`; the reusable claim is not yet verified or published. Deferral
continues to leave shared knowledge untouched.

This is a **trusted external controller** workflow, not an authentication
mechanism. The host must isolate the primary with canonical Brain knowledge,
Memory Bank chunks/index and promotion state read-only, leaving task progress
and local metadata writable. Separate reviewer and controller files must be
invisible to it. The reviewer runs in a separate process/session with a read-only
packet/source view, no memory-writing hooks and no access to the controller.
Different actor names, session strings and JSON hashes alone prove none of this.
Running an unrestricted primary with only the mode enabled does not enforce
independent verification: other write APIs and direct file edits remain possible.

After the primary exits and completes its native progress-only save, the host
exports a packet and later supplies its captured review:

```bash
memory-bank/bin/infra-engine memory memory-review-controller --task-id ID \
  --proposal-id PROPOSAL --proposal-sha SHA --action export --json
memory-bank/bin/infra-engine memory memory-review-controller --task-id ID \
  --proposal-id PROPOSAL --proposal-sha SHA --action apply \
  --packet /private/controller/packet.json \
  --authorization /private/controller/authorization.json --json
```

Apply inputs must be absolute files outside project, tooling and state roots.
Export includes exact old/proposed consequences, a complete text-clause checklist,
admitted active predecessor bodies and full bounded UTF-8 canonical source
snapshots. It holds unsafe, oversized, secret-bearing or non-admitted material;
it never silently truncates a claim. Whitespace-sensitive replacement text is
held if the existing promotion renderer would change it.

The controller authorization has profile `memory-semantic-controller-v1`,
packet_sha256, primary_request (the finished native challenge key),
reviewer_session and verdict. The verdict has profile
`memory-semantic-verdict-v1`, packet_sha256, proposal_sha256, verdict
(`accept|reject|unresolved`) and one records entry per proposed UUID. Every entry
must review every exact old/new clause ID/hash with a reason and literal source
evidence, plus explicit boundary_checks. Old statuses are retained,
changed-with-evidence, removed-with-evidence or unresolved; new clauses and
boundary checks use supported, contradicted or unresolved. Evidence contains
path, sha256 and a literal quote from the packet's canonical source. The separate
reviewer judges meaning: matching a quotation is only a mechanical prerequisite.
Any unresolved/contradicted item or a reject/unresolved verdict blocks the whole
batch. The first captured authorization is frozen, including a negative verdict;
a different verdict requires a fresh proposal and review.

Accepted publication rechecks task, raw record, source and predecessor snapshots
under the shared mutation lock and reuses the guarded record_result path.
Completed canonical replay is passive even after primary-state removal, task
archival or source loss. Proposal/authorization metadata and current scope/runtime
are still required by this convenience command. Partial publication uses the
existing recovery fences; failed promotion is reported as held. Published
knowledge remains agent-attested, never human-reviewed.

Proposal creation is cooperative before the final answer. Independent review
and publication occur after the primary session exits; they do **not** gate the
already visible answer. The runtime invokes no provider or model. This mode is
off by default pending isolated source-grounded outcome evaluation; it changes
neither retrieval ranking nor automatic consolidation defaults.

The 2026-10-31 prospective actionAccess source-slice safety pilot did not
establish end-to-end publication. Its real native A diagnosis completed with a
valid progress-only save, zero reusable records and zero chunks; the sealed
handoff gate therefore refused B derivation. B models were not launched, A was
not retried and no memory was hand-seeded. The separate reviewer rejected the
false wildcard claim despite valid source quotations. It supported both clauses
of the intended positive replacement, but returned unresolved because a
still-supported part of the old claim was omitted without justification. Thus
the negative control passed and the expected complete positive acceptance did
not. Both remained unpublished. This is useful failure localization at capture
and old-claim preservation, not acceptance of the mode or evidence of task
benefit. Full-source/model checks used isolated copies; defaults stayed off.

### Read-only freshness packet

```bash
infra-engine eval memory-freshness-plan --root /private/project-snapshot --limit 50 --json
```

This runs the real provenance audit on a physical copy and groups its findings
into a deterministic review queue. Each entry preserves applicable reasons,
safe cited paths, available expected/current digests and review dates; unsafe
source details are withheld. Duplicate pairs remain unresolved. Queue limits
report `total_candidates`, `returned`, `remaining` and `truncated`; they do **not**
bound the existing O(n²) duplicate audit, which stays outside prompt hooks.

Nothing calls reverify/retire/quarantine automatically. After reading a claim
and checking its current canonical evidence, use the existing governed workflow:
`bank-reverify` for a still-valid claim, correction with superseding promotion
for a changed claim, `bank-retire` for ceased applicability, or `bank-quarantine`
for unsafe knowledge. `bank-reverify` attests its caller's check; it does not
perform that check or encode human identity. Unattended apply is deliberately
absent: it needs an independently reviewed actor/evidence and under-lock source
image contract, not a bulk digest refresh or a check followed by a separate
unfenced mutation.

The tools return unavailable/refusal for unsafe or unsupported snapshots. They
are not a new authority, an automatic truth verifier, or a proof of benefit on
real client tasks.

### Captured evidence and checked replay on a copy

Explicit evidence capture can include a fresh chunk before its sources change:

```bash
infra-engine eval memory-freshness-plan --root /private/project-snapshot --evidence --chunk memory-bank/chunks/MEM-YYYYMMDD-xxxxxxxx-claim.md --out /private/evidence.json
infra-engine eval memory-freshness-plan --root /private/project-snapshot --packet /private/evidence.json --decisions /private/decisions.json --export /private/new-reviewed-copy --out /private/review-report.json
```

Without `--chunk`, evidence capture uses the freshness queue. Packets bind the
input snapshot, chunk, originating record/revision and complete screened source
bytes. A previously captured packet retains historical observations after a
later edit. Capture of an already stale chunk contains current bytes and the
old expected fingerprints; it does not reconstruct earlier source content.
Replay requires the exact input image from capture and writes only a new
physical export outside the input and tooling checkout.
Copy/export preserves ordinary file permission bits, including executable hooks;
snapshot identity binds those bits as well as bytes. Capture and replay use the
same UTF-8 JSON byte limits, including packets with non-ASCII source evidence.

Each decision names `chunk`, `row_sha256`, `action`, `attestation: "agent"`,
`reason` and `evidence`. Each evidence item supplies `path`, `sha256` and a literal
`quote` from the captured source; all source files must be covered. `confirm`
checks the same claim, `correct` also supplies `consequence`, and `retire` supplies
`successor` plus `valid_to`. Retirement requires an explicitly selected fresh
active successor that is applicable today: its start date has arrived and its
expiry/review dates have not passed (today is included). Evidence reports
`currently_applicable`; replay rechecks it before retirement. Overdue selected
claims remain available for maintenance. This pilot check does not change the
installed retrieval rules. Retirement preserves both bodies and reciprocal links. A `defer`
decision supplies only chunk, row hash, action and reason.
Retirement rechecks both selected frontiers under the shared mutation lock,
then checks the legal body/link transition and source/record images again before
reporting success. A source writer outside that lock can still race; observed
drift reports failure and possible copy effects rather than overwriting it.

For an unchanged claim with unchanged source pins whose review date has passed,
explicit confirmation uses canonical `reverify_chunk` to renew the review date.
Chunk, originating record and complete source images are checked under the
mutation lock and checked again after renewal. The chunk's body and originating
record remain unchanged. A source change during renewal is a reported failure
with possible copy effects, never a successful check of the new source.

The same opt-in pilot can capture an originating resolved finding at the exact
canonical path `project-brain/archive/finding/<UUID>.md`. It must validate as an
archived record, match its path and type, have a unique identity across active
and archived records, and pass the existing agent-attestation, owner, privacy,
authority and complete-source checks. Ready archived-origin rows additionally
seal `origin_placement: "archive"` and `maintenance_capabilities`; existing
dynamic-origin packet fields are unchanged.

An archived finding remains immutable. Confirmation with unchanged claim and
source images renews an overdue Bank review through the guarded operation above;
a fresh confirmation is an explicit no-op with no new task or record. A
`correct` decision with the identical consequence follows the same path.
Changed application evidence or a correction requiring a new archived-record
revision is held as `archived-origin-revision-required` before task creation,
promotion or other knowledge effects. Retirement can use an admitted archived
origin through the same selected-successor and locked pre/post checks. Neither
operation rewrites the archived record, its authority, attestation or source pins.

This support is limited to archived findings. Accepted decisions remain dynamic
under the canonical lifecycle and retain their existing maintenance path;
canonically archived rejected or superseded decisions remain held. Person and
unknown legacy attestations also remain held, including when their files and
pins are otherwise readable. Archive location never supplies a missing check.

Only admitted, record-backed agent-attested claims are supported. Missing,
private, binary, oversized or unsupported evidence is held. Capture is bounded
to the originating record and its complete local source set; extra chunk
citations outside that captured set are unsupported and held. Capture is bounded
to 50 chunks, 256 KiB per source, 512 KiB combined source text and a 1 MiB packet.
Keep these full source packets private. Automatic consolidation, verification,
compaction and broad promotion are disabled on the copy; promotion is limited
to the selected checked record. Original settings and memory remain unchanged.

Read action states, `audit_after`, validation errors and copy overrides. Partial
effects report failure; a successful explicit deferral does not establish that
freshness was resolved. Supplied decisions attest the caller's check, not
independent semantic verification, human confirmation or a native review proof.
This experiment does not enable unattended changes in an installed project.

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
memory-bank/bin/infra-engine memory consolidate --ticket CMS-741          # dry-run
memory-bank/bin/infra-engine memory consolidate --ticket CMS-741 --apply
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
memory-bank/bin/infra-engine memory bank-audit --provenance --json
memory-bank/bin/infra-engine memory bank-quarantine --id MEM-... --reason "suspected poisoning"
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
infra-engine eval memory-fact-consolidation --input facts.json --json
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

`infra-engine pilot sqlite-vec probe` probes local `sqlite-vec`/`sqlite-lembed`
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
