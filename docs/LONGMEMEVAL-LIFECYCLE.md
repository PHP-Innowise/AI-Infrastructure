# Local LongMemEval lifecycle controller

`harness.longmemeval_lifecycle.Controller` connects an exclusively owned
registered memory instance to fixed question slots, local request construction,
one-use reader/judge callback tickets and a separate cost ledger. This is an
experimental local text profile; it does not run the official LongMemEval
environment or dispatch to a provider.

The operator supplies a fresh memory instance, bounded gold-free question plan,
clock and callbacks outside JSON configuration. The plan contains only
`question` and a null `query_image`. Question images and context image items
are refused before controller dispatch. Memory construction and insertion before
controller ownership, image ingestion and provider setup are separate scopes.

## Fixed slots and local observations

`query(ordinal)` invokes the held question once and captures the compact native
receipt ordinal during that invocation. Pre-work failure may leave planned slot
0 without a native receipt and map planned slot 1 to native ordinal 0. Neither
the horizon nor the remaining questions is renumbered. Failed, cancelled or
unmeasured native receipts cannot become successful reader input.

Insertion/query counts detect calls outside controller ownership. At native
close the controller validates the whole frozen bounded observation projection
before importing anything. It charges build time/insertion ingress once and each
mapped query's time/ingress/egress once. It excludes aggregate query time, unique
Markdown bytes and repeated hook snapshots. The original local ledger remains
frozen. Its unknown usage profile is preserved, and no-model attestation requires
the already trusted source profile. Imported local billing remains incomplete.

## Held requests and dispatch

`prepare_reader(ordinal)` serializes a local JSON envelope with fixed system text,
question and captured text context. The budget applies to the complete UTF-8
envelope. The lone memory item is retained entirely or dropped entirely; an empty
envelope over budget refuses. This byte selection differs from upstream token
selection and message construction. Official tokenizer counts and vision remain
unmeasured.

The returned identity-only ticket permits one callback entry. `dispatch(ticket,
callback)` consumes it before entry and supplies the exact held immutable request
bytes. A failed callback can receive one fresh retry ticket; successful/cancelled
units and exhausted reservations cannot launch another callback. Observation
ledger tickets do not authorize execution. Declared callback usage accepts the
existing normalized inclusive input/output profile; cache/reasoning are subsets.
Missing/partial/invalid usage remains unknown. Exceptions retain fixed outcomes
without exporting their prose. Clock interruptions preserve already returned
usage and attributed charges, release active state, and propagate cancellation.

Callback entry is not proof of physical provider coverage: an SDK may hide
retries, extraction or other work. **Provider coverage always remains unknown**
in this version, including callbacks that supply every token field. Callback
durations and exact request byte counts are observed locally; unit overhead is
unknown. They are not provider transport sizes, invoices or full wall latency.
Reply and construction serialization are not added again to request subtotals.

## Construction and evaluator boundary

`seal_construction()` closes native memory, imports its frozen observations,
closes reader units, then seals all planned slots from held requests, answers and
failure/unlaunched markers. Construction data is private and contains text; fixed
cost snapshots contain counters, enums and the seal identity. The aggregate
logical payload budget includes the question plan, contexts, held requests,
answers and construction copy. Python object overhead/RSS and transient encoding
buffers are outside that byte budget; it is not an OS memory limit.

`prepare_judge(ordinal, owned_construction)` requires the same immutable object
issued by that controller and a successful reader result. `dispatch(judge_ticket,
evaluator_factory)` calls the factory only after the construction gate, then passes
the held judge request to its callback. The operator persists construction bytes
before evaluator lookup in the local controls. The module never opens benchmark
answers or gold. Trusted callbacks' globals, monkeypatch/introspection and their
external IO are outside this managed boundary; this is not an OS sandbox.

Ordinary cleanup errors remain recorded. Cleanup cancellation preserves frozen
partial observations, propagates the interruption and cannot issue a new judge
gate. `finalize()` closes remaining units, records one terminal grade per slot,
freezes the separate ledger and returns deep copied metadata. Repeated finalization
does not repeat cleanup/cost. Failed judge retries do not publish provisional
grades; missing grades remain null and incomplete.

## Current validation scope

Fresh public Maple inputs and complete controlled request/construction/arithmetic
oracles are fixed before implementation. Local callback reports are scripted,
including a paid failed retry. Their known subtotals are bookkeeping evidence,
not observed provider charges or benchmark quality. A separate fresh native FTS5
composition check uses the pinned unchanged Memory API prefix and a new public
trajectory; it does not execute heavy upstream imports or official Python 3.11.

The upstream reader/judge SDK clients have hidden retries, missing reader usage
is zero-filled, and the judge helper discards usage. A future provider adapter
needs actual post-token-selection request capture plus retries-disabled or
transport-level physical attempt observation and separate operational admission.
Full tokens, full billing, cost-per-correct, multimodal quality and net savings
remain unknown. Production automatic memory, Python 3.14/FTS5 defaults and ONNX
optional/off remain unchanged.
