# Hindsight source-only recall pilot

## Private ingestion ownership journal

`harness.hindsight_journal` is an offline, programmatic component. It is not
connected to Sessions, automatic memory or a service executor. Python/FTS5 and
automatic local memory remain the defaults.

Public export screening also rejects literal quoted PHP/JSON credential keys,
WordPress key/salt `define(...)` calls, nonempty or malformed Composer auth maps,
and recognized tokens/personal data in decoded JSON strings. A credential key is
any name ending in a credential word - password, pass, pwd, passphrase, secret,
token, API/app/encryption key, salt - with a snake, camel or kebab prefix
(`DB_PASS`, `dbPassword`, `x-api-key`). Unquoted `.env`, ini and YAML values
(`API_TOKEN=...`, `pass: ...`), literal defaults of credential variables
(`env('DB_PASSWORD', '...')`, `os.getenv(...)`) and SQL `IDENTIFIED BY` /
`PASSWORD '...'` refuse too, unless the value is a placeholder or a reference.
Exact approved source bytes are preserved; no redacted replacement is silently
exported. Runtime references such as `getenv(...)`, empty defaults, JSON
property schemas and empty auth maps are permitted. This conservative syntax screen is not a general PHP
interpreter or proof that arbitrary encoded/dynamic content is public. Trusted
approval and source-revision binding remain required, and no service executor
is enabled by this change.

A narrow declaration subset permits HTTP bearer documentation in a whole JSON
source with `openapi: "3.1.1"`. The scheme must be a direct entry named `bearer`
(case-insensitive name) under `components.securitySchemes`, with exactly
`type: "http"`, `scheme: "bearer"` and optionally `bearerFormat: "JWT"`.
The format field is a documentation hint in the
[OpenAPI contract](https://spec.openapis.org/oas/v3.1.1.html#security-scheme-object).
Only the exact key/value/end character positions of that declared object are
exempted from the Composer-map rule. Both screening passes recompute positions
after normalization; original bytes and hashes remain untouched. Identical
objects elsewhere and whole OpenAPI documents inside decoded strings have no
exemption. Other secret/private/literal-credential checks still apply.

Duplicate decoded keys, nonfinite values, unknown versions, extra fields,
references and descriptions outside this small scheme grammar grant no
exception. The classifier limits its original view to 128 KiB UTF-8, nesting
16 and 4096 structural tokens/nodes, and rejects uncertain parsing. This is
an admission subset, not full OpenAPI validation. Security-requirement arrays,
YAML and PHP cast/validation grammar remain unsupported by this exception.
The existing credential decoder runs before classification; classifier bounds
do not certify all earlier decoder work. String spans are collected eagerly
within the byte bound, so the token ceiling does not cap allocated span tuples.
Additional parsing has a cost; no speed, hard cancellation deadline or provider
token savings have been established. Offline supplied construction proves byte
correspondence, not capture, service authorization or a remote no-op.
Nested JSON strings (including `COMPOSER_AUTH`) use at most 4096 total views
(the raw view plus changed decoded views), depth 16 and 1 MiB aggregate UTF-8
view bytes. Ordinary quoted strings consume no decoded-view allowance. A forward
scanner visits spans once without restarting at later escaped quotes, and a
quote never pairs across a line break. Raw credential checks precede malformed
encoded-span refusals; Unicode escapes must be well formed. JavaScript/PHP code
point escapes (`\u{1F600}`) are decoded and screened like any other. In PHP,
Markdown, text and SQL sources an unknown `\u` is literal text (a namespace such
as `"App\utils"`, a Windows path) and an unmatched quote may end the file.
Plain unmatched prose quotes remain compatible. Unsupported language escapes
such as JavaScript `\x41` remain opaque under this profile.
Exceeding admission refuses the source; it never marks unknown content safe or
silently exports only a selected fragment. Original source bytes stay unchanged.

Export capture, planning and current journal admission accept a trusted `active`
callback. Constructor revalidation receives it as an init-only argument, without
retaining it or changing serialized identities. Default/manual validation and
`dataclasses.replace` still apply current admission. Screening polls per view,
within 2048 scanner character inspections, and around regex/decoder operations.
Cancellation or ordinary callback failure returns fixed codes; journal admission
maps refusal to `input` and checks inside its transaction before COMMIT. There is
no post-COMMIT rollback promise. History integrity invokes no current callback.
This is cooperative cancellation: an already running regex/decoder cannot be
preempted. Bounded forward discovery does not prove whole-screen linearity or a
hard wall deadline. Callbacks must honor the caller's exclusive-connection and
nonmutation prerequisites; a supplied `BaseException` propagates through existing
transaction cleanup rather than becoming a cooperative refusal code.

Trusted operator code provides a dedicated, secure file-backed SQLite connection
and expected journal UUID. Use `create_new` explicitly for a new identity, or
`open_existing` for strict reopening; a missing/foreign/corrupt database is not an
empty successful history. The supported profiles are WAL with FULL/EXTRA, or
DELETE with EXTRA, and a busy timeout of at most 1000 ms. No ambient transaction,
custom row/text factory, attachments or extra schema objects are supported.
Transactions use explicit SQL, including on Python 3.14 `autocommit=True`.
Caller code owns filesystem permissions, connection exclusivity, VFS and sidecar
custody; these checks do not prove power-loss durability or live authorization.

Historical loading validates fixed profiles, canonical envelopes, scope and
identity correspondence, hashes, ordering, uniqueness and payload bounds. It
does not rerun today's content/path admission rules. Tightening those rules
therefore preserves access to intact history, settlement and reconciliation
of previously recorded versions. Historical integrity does not prove past
public approval, source eligibility or remote ownership.

`prepare` revalidates the public export and persists immutable exact request
bodies under a generation UUID and expected-head revision. Identical current
generation replay is input idempotency, not proof of upload. `request_body`
returns the selected frozen image only after current path/content admission.
`reserve_attempt` also re-admits only its selected image and returns a fresh ownership ticket
only after COMMIT; recording is immediately ambiguous because this component
does not observe transport. It permits one admission per generation-specific
intent. An unresolved attempt blocks that logical source across newer versions.
A commit exception fences the instance; strict reopening is required to inspect
possibly committed history. No exception/timeout implies a safe retry.
An image refused by current rules cannot be newly returned or reserved; unrelated
blocked historical images do not disable admissible current selections. Public
draft/export constructors and new `prepare` inputs retain current screening.
Previously returned buffers cannot be revoked by this local component.

`attest_terminal` requires the exact ticket and explicit
`caller-attested-operation-quiescent` profile. `success`, `definite_no_write` and
`settled_unknown` are caller attestations; the latter means quiescent with unknown
effect, never generic timeout/cancellation. Terminal replay must agree exactly.
Late completion updates only its attempt row, never the desired catalogue.
An admitted intent cannot be reserved again, even if later observations suggest
missing data. There is no automatic repair, replacement generation or cleanup.

`challenge` binds journal identity, a fresh instance UUID, current manifest/head
and complete attempt frontier. Nonces are single-use per instance; reopening
invalidates old handles. Expected observations cover current selection, explicit
prior ownership and **all historically attempted versions**, including terminal
ones. More than 64 distinct versions refuses the whole challenge, even if the
journal is below its other capacities. Limits are 32 sources per selection,
16 generations, 512 intents/attempts, 32 lifetime challenges per instance and
8 MiB of retained request-body/binding/manifest payload, without eviction.

`observe_document` compares supplied Hindsight 0.10.3 GET `id`, `bank_id`, raw
UTF-8 `original_text`, our metadata in both `document_metadata` and
`retain_params.metadata`, and exact scope tags. Upstream `content_hash` is not
assumed to use our algorithm. Missing/nullable/unsupported fields stay unknown;
mismatches remain conflicts. JSON rejects duplicates, nonfinite numbers,
invalid UTF-8, more than 16 nesting levels or 4096 nodes, and responses larger
than 256 KiB. It retains fixed states only, not arbitrary response/error prose.
Hindsight can mask tag-hidden documents as 404, so `observed_not_found` alone
does not establish global absence. See the pinned
[document read implementation](https://github.com/vectorize-io/hindsight/blob/v0.10.3/hindsight-api-slim/hindsight_api/engine/memory_engine.py).

`evaluate` requires a complete, exact supplied observation set and unchanged
head/frontier. Only with all attempts caller-settled and an explicit
`caller-attested-full-visibility-and-quiescence` profile can it return
`supplied_selection_current`, `missing_source_images` or `cleanup_required`.
Otherwise it remains incomplete. These are supplied source-image suggestions:
they do not prove credentials, freshness, full retain options, extraction,
native eligibility or semantic quality. **Every result authorizes zero
mutations; `actual_remote_noop` is always null.** This component executes zero
transport calls; recorded admissions are not HTTP counts. Physical SQLite I/O,
remote work, provider tokens and USD remain unmeasured.

Offline checks: `python3 -m unittest tests.test_hindsight_journal
tests.test_hindsight_history_admission`.
They exercise actual private SQLite files/connections and public supplied
images, not a live service, model, process crash or power loss.

`harness.hindsight_recall` is an experimental stdlib client. A programmatically
configured source-reference provider can now feed automatic TaskContext retrieval
through native guards, packing, manifests and receipts. The default provider is
`None`; FTS5 remains the ordinary backend. Native hooks and MCP do not call the
remote service. No service account, credentials,
live ingestion or Hindsight evaluation are included. An offline public-source
export and ingestion-draft builder is available separately.

The first profile uses Hindsight 0.10.3's
[recall endpoint](https://github.com/vectorize-io/hindsight/blob/v0.10.3/hindsight-clients/python/hindsight_client_api/api/memory_api.py),
`types=["world"]`, `budget="low"`, `max_tokens=512`, no supplementary chunks or
source facts, and `include.entities=null`. Observations/reflect/retain are outside
this profile. Upstream fact text is discarded: the service only proposes opaque
document IDs from an explicitly supplied local catalogue.

## Caller contract

Construct `Scope(team_id, project_id)` from trusted, registered canonical UUIDs.
The same IDs give the same bank across independent clones and linked worktrees;
different teams/projects give different banks. The local path-based Harness
project ID is unchanged. A matching ID or tag is consistency evidence, not
authorization: the remote credential must independently authorize the bank.
Never take this configuration from a cloned repository or a recalled fact.

`SourceBinding.from_bytes(scope, relative_path, raw_source)` binds an opaque
document ID to the full source SHA256, including dirty bytes. Source correction
changes that document ID. `Catalogue(scope, tuple_of_bindings)` admits at most
32 unique sources in one scope. Retained facts must carry matching
`document_id` and string metadata `harness_scope` and `source_sha256`.

`recall_context(query, client, catalogue, admit=..., local=..., active=...)`
computes `local()` exactly once. That callback returns an already-admissible
`PilotContext`; its errors and oversized baseline propagate as local failures.
Remote failures, denied/missing/stale evidence and cancellation return the exact
baseline object, with diagnostics beside it. A failed optional lookup does not
add native capsule warnings or trigger another local lookup.

`admit(binding)` must safely reopen the current local source, validate local
privacy, policy, ownership, eligibility and lifecycle, and return `Evidence`
with the same binding, those exact bytes and an exact UTF-8 byte range, or None.
The component validates the image hash and span. It does not implement native
eligibility. The callback is trusted code and is responsible for its own bounded
file access. Source images are at most 128 KiB. All tentative evidence is
discarded if a later callback fails or cancels. Callback execution has no hard
wall-clock bound.

The successful result appends locally admitted passages after the baseline in
service order, preserving baseline contents. Duplicate IDs are omitted. It
measures the complete compact JSON plus the shared whole-capsule warning and
trailing separators against **8,000 UTF-8 bytes and 8,000 characters** by default.
Overbudget candidates are omitted whole; claims are never clipped to fit.
`PilotContext` is a separate supplied-source format, not a native task-bound
capsule. Native task manifests, delivery receipts, policy gates and final
packing belong to the separate native reference path below.

## Native reference path

`recall_references()` resolves remote IDs through the operator's catalogue and
returns `ReferenceOutcome`, with at most eight bindings. Remote prose and scores
never enter native context. `source_hint_provider(client, catalogue,
project_id=..., bank=..., workspace=...)` binds an explicitly allowed local caller
to registered remote UUIDs. It compares all three local route fields before
transport; clone/worktree routes need explicit registration and a current
catalogue. Local paths do not generate remote bank IDs or grant authorization.
Supply that callback to `KnowledgeManager(..., source_hint_provider=...)`.
This is a trusted Python integration point, outside browser/session options.
No standard installation configures it automatically.

Automatic preparation invokes it once, outside knowledge-lock retries and with
generation checks before and after. Unsupported runtimes/platforms skip it.
Callback and remote failures preserve the single ordinary local refresh.
Cancellation stops the generation. Review preparation stays local.

The private native payload contains only `schema_version=1` and
`references=[{path, source_sha256}]`: at most eight entries, 8 KiB serialized
UTF8 and 512 bytes per relative path. Native admission hydrates exact indexed
semantic sources only, with current images no larger than 128 KiB. Policy and
episodic hints are outside this initial additive profile. POSIX descriptor
checks refuse links, multiple-link files and changed directory/file identities.
Raw SHA256 is checked separately from native universal-newline source hashes;
masked indexed text supplies the quote. Attached absolute tooling keys abstain.

Ordinary privacy, ownership, authority, lifecycle, current-source, own-task,
host-loaded, budget and novelty guards remain in force. Conflict components
abstain. Hints follow the local queue, relinquish capacity before history or
local excerpts, and cannot evict a lexical Brain source via promotion dedup.
Their selection is `source-hint`, with no lexical match/rank and zero score.
The lexical `no_match` remains a lexical claim.

After final packing, current hint custody and delivered identities determine
manifests and receipts. A dropped hint does not poison the repeat gate. The
manifest's `source_hints` records supplied, admitted and delivered counts;
these are separate stages, not quality scores. Missing/unsupported schema
neutralizes hints, and schema validation precedes delivery receipt writes.
Checks are point-in-time; they do not create a filesystem snapshot or authorize
remote claims as canonical facts.

`full_cost.remote_operations` records bounded callback/transport-method counts,
body bytes, EOF and elapsed time even if later native retrieval fails. It keeps
remote billing/tokens unknown and separate from native provider totals. A
trusted synchronous callback still has no hard wall deadline; live unattended
remote configuration therefore remains a separate readiness decision.

## Transport and accounting

`RecallClient` accepts an explicitly configured root HTTPS URL and a bearer
credential, uses ordinary certificate/hostname verification, connects directly
without ambient proxies, and performs one POST with no retries or redirects.
Credentials appear only in the request header. Userinfo, query, fragment and
non-root endpoint paths are refused. HTTP is permitted only for an explicit
literal-loopback fixture without credentials.

Request body: at most 8 KiB. Response body: at most 64 KiB including a one-byte
over-limit sentinel. Strict UTF-8 JSON rejects duplicate keys, nonfinite
numbers, excessive depth/cardinality and more than 32 results. Compressed
responses and non-JSON content types fail closed. The request asks for identity
encoding. Advertised Content-Length is not the admission authority; an observed
premature EOF fails. HTTP headers are handled by the standard library's own
limits and are not counted as body bytes.
Queries containing a recognized secret are refused before transport. Other
queries pass through the existing automatic-query privacy sanitizer; empty
sanitized queries use the original local result without a remote call.

Socket timeouts and monotonic checks apply at I/O boundaries. DNS and header
processing can overshoot this budget, and callbacks have separate execution
cost. A hard production wall deadline remains unimplemented. No background
transport worker is created. Connections close on success and failure.

Diagnostics contain fixed outcome codes, logical call attempts, serialized
request bytes, observed response bytes/EOF, accepted/rejected counts and elapsed
time. They contain no queries, bodies, paths, raw IDs, credentials or exception
prose. Request bytes describe a serialized body handed to the client, not proof
that every byte reached the server; response bytes are component reads, not
wire traffic. Server error bodies are discarded without reading them.

Remote/provider tokens and money remain `null`, and `total_cost_complete` stays
false. These values must not be merged into existing native-provider usage or
reported as zero cost. Fewer bytes or passing fixtures do not establish token
savings, service accuracy or native integration.
Replaceable transports receive an isolated component-metrics dictionary.
Only bounded integer body counts and boolean EOF enter the fixed diagnostics
schema; invalid counters remain `null`. Extra fields and arbitrary error prose
are discarded. The caller owns logical attempt and selection counts. Metrics
reported by a supplied transport establish format consistency, not authenticity
or wire observation; `transport_metadata_complete` describes only that schema.

## Public-source export and ingestion drafts

`harness.hindsight_export` supplies an offline, programmatic integration point:

```python
approval = PublicApproval(registered_scope, "docs/rules.md", approved_raw_sha256)
export = build_public_export(trusted_workspace, registered_scope, (approval,))
plan = plan_public_ingestion(export, previous_owned=prior_owned_catalogue)
```

The operator approves exact raw bytes as public outside cloned configuration.
The builder reads only those named sources: no repository/index scan, inherited
`allowed_privacy`, automatic Brain/Bank export or source auto-classification.
Ordinary `context export` is unsuitable for this profile because it permits
team records. This initial profile accepts canonical document/code suffixes and
refuses hidden/task/private/log directories and the private runtime state and
derived-memory directories covered by its path policy.
Path, secret, personal-data and transcript screening refuses the whole export;
pattern matching does not replace operator classification. CRLF and lone CR
are normalized only for screening, while approved hashes and retained images
keep their original bytes.

On supported POSIX platforms it retains descriptors through the absolute-root
and relative-source walk, refuses links/FIFO/multiple-link files, bounds reads
and rechecks named directory/file identity. A missing, changed, cancelled or
denied source aborts the entire capture, so partial input cannot generate a
retirement plan. Checks are point-in-time; they do not freeze the filesystem or
prove canonical facts, native eligibility or service access. The held image is
serialized without reopening the source. No output files are written.

Limits are 32 approved sources, 128 KiB per raw source, 512 KiB total raw sources,
256 KiB per complete retain draft and 1 MiB for the complete local packet.
Paths have a 512-byte UTF-8 ceiling; absolute roots have 64 components/4,096
characters. Escaping, tags and metadata count toward serialized ceilings.
Returned packet/body values are immutable bytes; constructors validate their
fixed shapes and consistency. Direct supplied construction is consistency
evidence, not proof that file capture or approval occurred.

Each retained draft has one full image with `async=false`, `update_mode=replace`
and `timestamp=unset`, matching the pinned
[v0.10.3 request](https://github.com/vectorize-io/hindsight/blob/v0.10.3/hindsight-clients/python/hindsight_client_api/models/retain_request.py)
and [item schema](https://github.com/vectorize-io/hindsight/blob/v0.10.3/hindsight-clients/python/hindsight_client_api/models/memory_item.py).
The remote body carries opaque scope/source/version identities and hashes, not
the local filesystem path. Registered IDs survive clone/worktree path changes;
logical `source_id` is scope plus relative path. Renaming a source is remove/add.
The existing raw-content version `document_id` remains compatible with recall.
Correction creates a new version ID: replacing that new ID cannot remove the
old document. Scope/hash metadata and scope tags are fixed. Credentials must
separately authorize the bank. Server extraction, embeddings and consolidation
still occur according to server settings; sync retain is incompatible with
server provider-batch mode.

`plan.current` contains only the newly approved current versions. Old changed
or omitted versions leave it immediately; existing recall providers do not
automatically switch catalogues. The caller must register the new catalogue and
continue current local admission. `previous_owned` is a trusted bounded ownership
catalogue, never a server-supplied list, acknowledgement or upload receipt.
Its version IDs are independently recomputed. Empty approval means an empty
supplied selection, not global source/document absence.

Every plan stays `reconciliation_required` with zero authorized mutations and
`remote_noop=null`. `retain_candidates` describes novel desired inputs only;
`current_retain_drafts` retains **all** current request bodies, including matching
inputs, for later GET-not-found repair. Targeted GET drafts cover the union of
prior/current versions; retirement candidates name only supplied owned prior
versions. These are plans, not dispatched requests. A failed or ambiguous upload
cannot be inferred from local equality or historical receipts. Remote ownership,
complete current state, in-flight ingestion, conditional deletion, concurrency,
service-bound dispatch/retry/acknowledgement handling and catalogue refresh
remain unimplemented; the private journal above supplies offline intentions and
caller attestations only. In particular, pinned
[async replay](https://github.com/vectorize-io/hindsight/blob/v0.10.3/hindsight-api-slim/hindsight_api/engine/memory_engine.py)
checks an operation's bank/type, not an immutable payload hash; a future async
executor must persist that binding before sending and serialize cleanup against
pending work. No cross-version atomic replacement is claimed.

Count-only diagnostics distinguish observed local source reads and complete
serialized draft/packet sizes from transport calls (zero in this component).
Retain, extraction, embeddings, consolidation, storage, deletion,
reconciliation, retries and provider token/USD costs remain unknown. Matching
inputs and smaller bodies do not establish cost savings. The fresh offline
suite is `python3 -m unittest tests.test_hindsight_public_export`; it uses new
public source images and literal pre-implementation packet/request oracles,
ordinary local temporary files and no service, credential or model.

## Local validation and next stage

Run the new public fixture suite with:

```bash
python3 -m unittest tests.test_hindsight_recall
```

It uses synthetic responses and a literal-loopback HTTP server, never a public
service, repository export, real credential or model. Source checks in this
fixture are supplied checks, not complete native policy certification.

The new native suite is `python3 -m unittest tests.test_hindsight_native_hints`.
It uses fresh public source images, copied memory runtimes and ordinary ROOT
processes, with synthetic transports and no provider/model or network calls.
It covers the actual TaskContext refresh/render/manifest/prompt path, not live
service quality or provider-token savings. Public export/draft construction is
locally validated separately; live ingestion/reconciliation, remote
authorization/bootstrap, hard deadlines and remote
billing remain separate prerequisites for a live cost comparison. Compare
FTS5 against FTS5 plus remote recall using identical public tasks, reader and
whole-context budget; charge ingestion and every seed/resume/recovery operation.
