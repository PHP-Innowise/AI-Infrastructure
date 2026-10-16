# Selected LongMemEval async reader and synchronous judge

`UpstreamController` is a NEW isolated mixed-call benchmark profile. It uses a
separate module and keeps the closed synchronous controllers unchanged. The
operator supplies trusted pinned helper functions and injected SDK-shaped create
callbacks/serializers. This module imports no SDK/heavy benchmark, opens no gold,
bootstraps no client/service and performs no native-agent/provider operation.

The pinned commit is `2cc8c540bdb87fe6761629b585e727e1c4704520`. Selected upstream
`call_reader_model_async` awaits `client.chat.completions.create`; the selected
`_call_chat_completion` judge is synchronous. NEW controls also execute unchanged
upstream `require` and `extract_text_from_response_message` rather than replacing
their RuntimeError/nonempty/content-list/reasoning-fallback behavior. Only those
selected stdlib function buffers run with supplied SDK-shaped values; full heavy
module imports, official environment, SDK and official evaluator are unrun.
The control loader defers annotations explicitly, preserving selected function
bodies without evaluating upstream union annotations on the Python3.9 floor.
Compile/AST compatibility is not a Python3.9 runtime pass.

The admitted request profile fixes two text messages, model/completion bounds and
timeout, `base_url=None` and all optional sampling/extra options absent. Prepared
canonical JSON includes the exact `model`, `messages`, `max_completion_tokens`
and `timeout` received by the owned shim. It omits `n`/`stream`, matching these
selected functions. It is a detached **SDK argument proxy**, not SDK encoding,
HTTP body/traffic, server receipt, model input or physical attempt count. The
budget covers this entire representation. Whole context drops if needed; an
oversized empty-context request refuses before ticket issuance. The leading
canonical-source warning remains. Images/streaming/tools/extra-body/base URLs/
auth/config changes are unsupported, not silently enabled.

Use `await dispatch_reader(ticket, async_create, serializer)` for the reader and
`dispatch_judge(ticket, factory)` after an owned construction for the judge.
The judge factory returns `(sync_create, serializer, local_grade_decoder)`.
That decoder is an explicit local policy, not official evaluator grading. Reader
outward return `(text, legacy_usage_dict)` remains unchanged. Its zero-filled,
coerced compatibility counters NEVER feed the ledger; judge's discarded usage
is independently observed before selected content parsing.

Admission consumes an owned phase/ordinal ticket and begins one ledger attempt
under a short lock. Busy state stays active across the await, while the lock is
released. Competing dispatch/preparation/seal/finalization refuses. No global
latest response or shared observer exists. SDK kwargs detach from caller aliases
and must exactly match the held body before one shim boundary. That create call
can hide retries or other work; physical attempts remain unknown.

After a create returns, one trusted serializer supplies a detached response
serialization. The bounded usage leaf captures it before the unchanged helper
decodes content. This is a **supplied serialization proxy**, not authenticated
SDK/provider evidence. Its512KiB admission is after serialization allocation and
does not bound RSS or callback time. Ordinary observer failures mark unmeasured
usage while the selected helper can continue. Serializer cancellation aborts
the call and is booked before propagation. Cancellation while create awaits has
no returned response and unknown usage. Paid content/type/grade failures retain
valid captured counters; missing/partial/null remain unknown. Outer terminal
bookkeeping finishes the ledger once synchronously; original task errors outrank
an additional metadata/end-clock cancellation. No second usage import occurs.

The complete reader packet binds local envelopes and all admitted actual SDK
arguments, including failed retries and unused reservations, under a NEW profile.
There is no temporary gate to replace. Judge factory lookup follows this gate;
the full-cycle control additionally persists its exact bytes before the factory.
Judge provenance remains a separate post-finalization journal bound to that hash.
Logical held-payload admission, not durable/OS/crash isolation, applies to these
packets. Numeric snapshots exclude prompts, responses, IDs, model labels and
arbitrary exception prose; private packets contain request/answer provenance.

Fresh controls use independent NEW Cobalt literal inputs, complete kwargs/packet
oracles and scripted usage/grades with supplied local memory. Real asyncio
suspension demonstrates cancellation/reentry guards; two interleaved owned
sessions show receipt isolation. Native FTS5 integration/retrieval, real models,
official Python3.11/heavy/multimodal benchmark, retries/storage/invoice/full
lifecycle cost/latency and quality/abstention/staleness/token benefit remain
unmeasured. Known supplied counters are subtotals only; full tokens/USD/physical
attempts remain null and coverage UNKNOWN. Python3.14/FTS5 defaults, optional
disabled ONNX and automatic production memory remain unchanged.
