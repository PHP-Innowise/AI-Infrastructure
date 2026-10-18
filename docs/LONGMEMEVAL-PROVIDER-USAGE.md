# Supplied provider usage for the isolated memory benchmark

`harness.longmemeval_provider_usage` projects a bounded supplied nonstreaming
ChatCompletion JSON response into the memory cost ledger's five-counter
profile. It captures those counters before a trusted answer decoder runs, so
decoder failure or cancellation does not erase already supplied usage. It
performs no network, SDK, native agent, model or service call.

`completion_usage(raw_bytes)` preserves missing/null fields as `None`. Exact
nonnegative signed 64-bit integers are required: booleans, strings and floats
are not counters. Inconsistent totals, overflow and orphan/oversized subsets
invalidate the entire usage tuple. Explicit zeros remain valid observations.

| Supplied ChatCompletion field | Normalized field |
| --- | --- |
| `usage.prompt_tokens` | `input_tokens` |
| `usage.completion_tokens` | `output_tokens` |
| `usage.total_tokens` | `total_tokens` |
| `usage.prompt_tokens_details.cached_tokens` | `cached_input_tokens` |
| `usage.completion_tokens_details.reasoning_tokens` | `reasoning_output_tokens` |

The mapping follows the [official OpenAI schema](https://developers.openai.com/api/reference/python/resources/chat/subresources/completions/methods/create).
Cache and reasoning are subsets of the main totals. Recognized audio, image,
text, cache-write and prediction breakdowns are admitted as bounded optional
counters but are not added again or used to calculate USD. Unknown usage
extensions refuse this declared profile. Streaming chunks, Responses API and
error objects are unsupported; they are not interchangeable counter schemas.

`usage_state="complete"` means only that all three main counters are explicit
and consistent. Cache/reasoning may still be unknown. It does not verify
provider origin, physical requests, billing, task correctness or complete
lifecycle cost. Every observation keeps provider coverage `UNKNOWN`, physical
attempts/USD `None`, billing `unknown` and usage verification `False`.

`CompletionCapture.deliver(raw, decoder)` consumes one private capture before
normalizing and invoking the decoder. The decoder receives the same supplied
object; its result is returned without being retained. Metadata is available
before decoder entry. Original decoder errors and cancellations propagate;
usage remains in the snapshot. A replay or reentry is refused before another
decoder entry. Ordinary observation failure marks the capture unmeasured and
allows decoding; observer cancellation propagates without launching decoding.
An awaitable return is labelled `returned_awaitable`, passed through without
automatic awaiting/closing, and never represented as completed decoding.
If inspecting that result fails, `returned_unknown` keeps paid usage and the
identical returned object. Classification cancellation propagates with that
state. Exception classification uses the actual type hierarchy, so a reflected
`__class__` property cannot replace the original decoder interruption.

Snapshots contain only bounded counters, fixed states and booleans. They do
not retain response bodies, answers, prompt, model IDs, request/response IDs,
headers, URLs, credentials or exception prose. Snapshot mutation cannot edit
the retained observation. Private Python attributes, the operator and decoder
are trusted; this is not an OS sandbox or a controller/ordinal authorization
capability.

Admission covers a supplied body of at most 512 KiB, container depth16 checked
before JSON decoding, 4096 tree nodes, 64 object members, 1024 array elements,
131072 characters per string and 64 characters per numeric literal. Duplicate
keys, nonfinite JSON numbers, invalid UTF8 and lone escaped surrogates refuse
observation. Larger/refused input still reaches the trusted decoder: this is
an observer, not a production task input policy. These are logical parser
bounds, not RSS, hard wall-time or whole-runtime limits. `response_bytes` is
the length of supplied bytes, not proof of HTTP traffic or provider tokens.

Fresh offline controls use independent public raw responses and scalar
oracles. Two additional controls execute exactly pinned selected upstream
reader/judge functions with fake SDK values. They demonstrate capture before
the reader's missing-usage zero fill and judge's usage loss, including an
empty-string judge return and the helper's invalid empty-list branch.
Selected-source compiled kwargs are inspected, but no
whole official runner, SDK serialization, HTTP or real model is exercised.
The reader is asynchronous; this leaf is not a drop-in synchronous lifecycle
adapter. Both pinned upstream clients permit hidden SDK retries, whose costs
remain unknown.

Next integration must bind an owned controller observation capability and
actual final request/physical attempt receipts, retaining captured usage
through the controller's failure path. This increment changes none of the
closed lifecycle/controller code. The official Python3.11 environment,
multimodal benchmark, provider usage/billing and token benefit remain unrun or
unmeasured. Local Python3.14/FTS5 defaults, optional disabled ONNX and automatic
production memory remain unchanged.
