# Local observations for the LongMemEval text-projection pilot

`harness/src/harness/longmemeval_observed.py` connects the standalone native FTS5
projection to the fixed-horizon memory cost ledger. It does not register an
official upstream backend, evaluate benchmark answers or launch models/services.
Production memory remains automatic, Python 3.14/FTS5 remain defaults and ONNX
remains optional/off. The official Python 3.11 environment stays separate.

Create an observer with a positive fixed `horizon`, call `open(private_project,
trusted_native_module)` once, then insert public trajectory dictionaries. The
first query closes insertion admission. Each attempted query, including input
refusals and cancellation, consumes one ordinal. `query_receipt(ordinal)` returns
an independent fixed metadata copy. `close()` attempts backend descriptor cleanup,
closes build accounting and freezes the ledger. Cleanup errors propagate after
metadata freezes; repeated close returns the snapshot and never retries cleanup.
The runner retains responsibility for an incomplete cleanup and project removal.

The private backend must be exclusively owned. Each query and its native hook
run under one wrapper lock; the hook must advance the independently tracked
native query counter exactly once. Wrapper ordinals and native counters differ
after input refusals. Stale/missing/throwing metadata stays unmeasured, while a
valid returned context can still reach the caller. There is no latest-only hook
API in this wrapper. Out-of-phase/over-horizon calls refuse before backend work
without changing frozen observations.

## Measured components and limits

Named disjoint `monotonic_ns` intervals cover backend open, each insertion, each
query including native connection cleanup, and backend descriptor close. They
include admission/serialization within an active operation. Ledger bookkeeping,
snapshot export, fixture/runtime preparation, idle time, out-of-phase refusal
overhead and operator project removal are outside these spans. Component sums
are not complete end-to-end wall latency, p50/p95 or machine resource costs.
Invalid/backwards clocks and ordinary clock exceptions leave affected time unknown
without inventing zero or suppressing successful retrieval. Clock interruptions
propagate cancellation after observations are booked; interrupted query returns
have zero returned egress. Active ownership is released even on interruption.
Trusted clock/native source authenticity
is an operator prerequisite, not internally proven provenance or OS containment.

Constructed bytes are exactly compact, sorted-key, `ensure_ascii=False` UTF-8 JSON
of every captured insertion, every admitted `{"query": ..., "query_image": ...}`
request and context arrays actually returned. Exact insertion replay still costs
ingress/time. Empty `[]` is two bytes. Exceptions/cancellation have no returned
egress; captured ingress/time remain counted. Unique projected Markdown bytes
are separate and are never repeatedly charged as transport. Serialization escaping
is charged and the canonical context-array envelope is bounded to 256 KiB.
These are serialization proxies, not actual file reads, HTTP bytes or model tokens.

Default `local_token_profile="unknown"` keeps local token totals unknown. The
explicit `pinned_local_no_model` profile is a trusted caller attestation that the
executed pinned local path performs no model work. It cannot be selected merely
because usage is absent or context is empty. The integration control pins native
source/config/schema inputs and executes the local FTS5 path; this is not full
Python imported-object/linker closure or a generic guarantee about injected code.
Billing is always incomplete: no infrastructure/storage/invoice measurement is
provided. Reader, judge and grades remain unlaunched/unknown. Product/experiment
totals and cost per correct answer therefore remain null, even if local phases
have an explicit zero-model-token attestation.

No text, context, paths, source IDs, hashes, raw callbacks or exception prose enter
the exported numeric/enum snapshots. Private projected sources and native local
manifests remain on disk after close and may contain sanitized request text.
Screenshots and query-image contents are never opened; query image ignoredness
is explicit. The inherited text-only admission, ownership and source verification
boundaries still apply. No persistent/reopened/corrected corpus is supported.

## Upstream boundary and fresh verification

The selected official source files are pinned at
[`2cc8c540bdb87fe6761629b585e727e1c4704520`](https://github.com/xiaowu0162/LongMemEval-V2/tree/2cc8c540bdb87fe6761629b585e727e1c4704520).
An actual bridge still needs registered `Memory` construction, strict JSON params,
fresh-root allocation, invocation context and pre-write persistence refusal.
Upstream generic trajectory preparation opens images and produces incompatible
fields; it must not be silently used for this text-only profile. Its post-query
hook runs before reader truncation, so constructed output is not reader consumption.
Its usage extraction substitutes missing reports with zero; that output cannot
establish complete physical-attempt reader/judge accounting.

NEW public Saffron trajectories, complete controlled observations, fixed accounting
projections and adversarial input specifications precede implementation/calls.
The fresh test module is `tests/test_longmemeval_observed.py`. It covers lifecycle,
failure/cancellation, replay, stale hook isolation, bounded egress, clock failures,
privacy, fixed horizon and an actual native FTS5 construction on a copied project.
That construction is sealed before the separate logical public support evaluator;
this is a controller/dataflow boundary, not separate OS isolation. No benchmark
dataset, answer/gold, upstream runtime, reader, judge, service or model is executed.
These controls do not establish leaderboard quality or net provider-token savings.
