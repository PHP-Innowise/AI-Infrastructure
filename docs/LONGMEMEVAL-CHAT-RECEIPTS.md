# Invocation-owned Chat arguments and response receipts

`ChatController` is an isolated, modified synchronous benchmark profile. It
extends the existing local lifecycle helpers in a separate module and leaves
the original controller and production memory unchanged. It performs no SDK,
HTTP, provider, native-agent or service operation itself.

The controller compiles a deterministic final ChatCompletion JSON argument
from the held local envelope. It fixes two text messages, operator-specified
model labels and completion bounds, `n=1` and `stream=false`. The leading system
message retains canonical-source verification. The user message contains the
question/context or post-gate judge payload without repeating that system
field. Models/options/endpoints cannot be supplied in the response or changed
through dispatch. Tools, images, auth, URLs, timeout, `extra_body`, streaming
and asynchronous SDK integration are outside this profile.

Reader budget applies to the **entire final UTF8 JSON argument**, including
model, messages and options. Over budget, the entire memory item is dropped;
if the empty-context final body still exceeds budget, admission refuses before
transport. The local envelope remains separately held and privately sealed.
Each final body and each construction/journal copy reserves aggregate payload
capacity. This is a logical held-payload bound, not an RSS/OS/hard-time bound.
Judge preparation stages both local and final reservations in one in-memory
issuance transaction. Pre-ticket cancellation or refusal cannot leak a local
reservation, consume an attempt or exhaust a later valid preparation. This
does not provide durable/process-crash recovery.

Reader use is `dispatch(ticket, injected_transport, decoder)`. Judge use is
`dispatch(ticket, factory)` after an owned construction gate; the factory
returns an exact `(transport, decoder)` pair. Foreign, replayed, wrong-phase
and reentrant calls refuse before entry. No arbitrary normalized cost report
or caller-supplied USD is accepted. The private ticket registry supplies the
phase, ordinal and reservation; identical body bytes across ordinals are not
used as invocation identity.

Each owned ticket has a fresh private response capture. The transport receives
the exact immutable held final body. Supplied raw-response usage is captured
before decoding. A controlled wrapper validates the decoded answer/grade and
generates the base report using only that capture. On error or cancellation,
it returns a valid failed/cancelled report so the base ledger finishes once,
then rethrows the original exception. The original task exception outranks an
additional end-clock cancellation. There is no second ledger import/finish.
Invalid decoded values and answer-held refusal also retain supplied paid
counters. Awaitables are not automatically awaited or certified as successful
answers. Missing/invalid observation remains unknown, not zero provider work.
The normalized capture is read once and cached. Failure-report construction
cannot downgrade earlier known usage by rereading a failing observer. Ordinary
capture-creation or snapshot failure marks usage unmeasured while decoding can
continue; creation cancellation occurs before ticket issuance. Snapshot
cancellation is recorded in the attempt before it propagates. An original
decoder error retains precedence over an additional metadata interruption.

The base callback-entry count includes judge factory admission. The new
`injected_transport_boundaries` counter is separate. Factory/start-clock
failure has zero such boundaries and zero charged final-argument bytes,
although the prepared body remains privately recorded. Request subtotal uses
final bytes once per injected boundary, including failed retries. Local
envelopes, response lengths and construction copies are separate evidence;
they are not added again to that subtotal. Component clocks still omit full
operator/runtime/bookkeeping/cleanup overhead.
The overall ledger's product subtotal also includes separately imported local
build/query component bytes; only its reader/judge phase subtotal represents
the final injected arguments. Neither subtotal is total wire traffic.

`seal_construction()` emits one final owned reader gate after local close/join
and reader closure. It contains the local envelopes and every admitted final
reader body, including failed retries and unused reservations. No temporary
base gate is issued and then replaced. An augmented-seal failure blocks judges.
Judge bodies arise after this gate and cannot be part of its hash. After
finalization, `private_judge_journal()` produces a separate idempotent private
artifact bound to the reader hash. Publication reserves its copy first; a
journal failure cannot alter or reopen the frozen cost ledger.

Private construction/journal artifacts contain prompts, model labels and
answers. Numeric snapshots exclude those, raw responses, IDs, endpoint/auth
material and exception prose. They expose only bounded fixed state/count
projections. Trusted injected callables and private Python attributes are
outside the JSON observation boundary; this is not an OS sandbox or proof of
provider origin.

The observed boundary is an **injected callable argument and supplied response**.
It is not SDK serialization, HTTP traffic, server receipt, model consumption,
all physical attempts or invoice attribution. A callable can hide retries or
other work. Provider coverage stays `UNKNOWN`, physical attempts/USD remain
null, and complete lifecycle metrics remain unavailable. Valid supplied usage
tuples are known subtotals only.

Fresh controls use independent public Larch inputs and complete body/seal/cost
oracles with a supplied local-memory profile and injected callables. They
exercise real base mapping/join/ledger/cleanup helpers, not native retrieval or
real model quality. Public evaluator lookup follows a persisted reader packet
in the full-cycle control. No original benchmark answers/gold are opened.
Scripted grades and token counters are not measured accuracy or provider cost.
The official Python3.11/full upstream/multimodal runner is still unrun, and this
profile is not an official leaderboard result. Python3.14/FTS5 defaults,
optional disabled ONNX and automatic production memory remain unchanged.
