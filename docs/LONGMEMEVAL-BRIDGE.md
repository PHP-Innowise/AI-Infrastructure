# LongMemEval registration bridge

`harness.longmemeval_bridge.register_bridge(api, project_factory)` registers
`native_fts5_states_v1` in a trusted, already loaded LongMemEval Memory API.
The factory is supplied by the operator outside JSON configuration and returns
one fresh, exclusively owned project and its native context module. Registration
refuses an occupied name. Other writers to the upstream registry must be quiescent;
the bridge's registration lock only coordinates bridge registrations.

The direct configuration is:

```json
{"memory_type":"native_fts5_states_v1","memory_params":{"horizon":2}}
```

`horizon` is an integer from 1 to 512. The optional `local_token_profile` is
`unknown` by default; `pinned_local_no_model` requires an operator's source-path
attestation. Neither profile establishes reader, judge, invoice or full-product
token costs. Configuration cannot select a module, directory or factory.

## Calls and ownership

The runner calls `insert`, then for each query supplies a fresh exact invocation
ID through `set_query_context`, calls `query`, invokes `post_query_hook` with the
same request and returned context, and calls `clear_query_context` in `finally`.
An ID consumes a horizon reservation even if the query never launches or fails.
Clearing does not authorize reuse or replacement. Queries are serialized per
instance; hooks remain attached to their invocation when threads interleave.

Hooks return copied, cached ordinal receipts. Repeating a hook does not repeat
retrieval or charge another local query. The pinned upstream runner calls this
hook before reader truncation, so it proves constructed context only. It does
not prove what the reader received, read or used. Reader usage remains unknown.

Fixed hook and snapshot exports contain no raw invocation IDs, query/context
text or exception prose. The inherited private thread-local `get_query_context`
can expose the invocation ID to the trusted runner until clear; this is outside
the fixed export guarantee. Private keyed digests are not exported.

Before insertion or querying, `configure_runtime` accepts the upstream trace,
generation and cancellation arguments. Trace and generation values are strictly
validated, ignored and not retained; they cause no trace IO or model setting.
Foreign subclasses are refused before adapter-body callbacks. Interpreter
keyword binding precedes the function body and is outside this guarantee. A supplied exact
`threading.Event` is checked before work. Cancellation or malformed bridge input
can consume a bridge attempt while leaving the local query unit unlaunched.
That missing unit remains unknown in the fixed horizon.

The runner must explicitly call `close` in its outer `finally`. Close freezes
the metadata, attempts backend descriptor cleanup once, and preserves an
observed cleanup failure. Repeated close returns the frozen view. Post-close
thread clearing removes private context without changing that view. The
operator remains responsible for project removal and incomplete construction
cleanup. The upstream Memory API supplies no automatic close lifecycle.

Class-level save, load-backend and saved-config reconciliation refuse persistence
before class IO. Upstream `build_memory(config_path)` and `load_memory(directory)`
read configuration before reaching these guards. Their reads are outside this
guarantee; use a trusted direct configuration and govern runner persistence.

## Validation and measurement scope

The selected API is pinned to LongMemEval-V2 commit
`2cc8c540bdb87fe6761629b585e727e1c4704520`. Tests verify the complete upstream
`memory.py` hash, then execute its unchanged 7,960-byte stdlib API prefix. Six
backend imports are excluded at their exact boundary. This validates registration,
construction and API contracts locally on Python 3.14; it does not execute the
whole official module, heavy backend imports, Python 3.11 environment or official
evaluation runner. No installed package or production memory default changes.

Fresh public fixtures exercise this registered prefix and native FTS5. A
construction seal precedes logical support checks. The evaluator separation is
logical, not an OS sandbox. Images are ignored, no image bodies are read, and
this text-only check is not an official leaderboard or multimodal accuracy test.

Local observation times cover backend open, insertion, query and close. Factory
setup, registration, bridge validation/HMACs, accounting/export, waits and project
removal are outside those component spans. Canonical JSON sizes are serialization
proxies, not actual transport or model tokens. Hooks report already booked units;
a future lifecycle controller must join them once and separately observe reader,
judge, retries, truncation, billing and the full fixed query horizon. No token
savings or cost-per-correct result follows from these local checks.
