# LongMemEval registration bridge

`infra_pilots::longmemeval::registry::register_bridge(registry, project_factory)` registers
`native_fts5_states_v1` in a trusted, already loaded LongMemEval Memory API.
The factory is supplied by the operator outside JSON configuration and returns
one fresh, exclusively owned project and a typed `NativeRuntime` implementation. Registration
refuses an occupied name. Other writers to the upstream registry must be quiescent;
Rust registration requires exclusive mutable access to `MemoryRegistry`.

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
text or exception prose. Invocation bindings remain private to the bridge's per-thread state until
clear. Private keyed digests are not exported.

Before insertion or querying, `Bridge::configure_runtime` accepts typed optional
cancellation (`Arc<AtomicBool>`), trace-directory, temperature and top-p inputs.
The latter three are bounded, recorded only as ignored configuration metadata,
and cause no trace IO or model setting. The cancellation flag is checked before
work. Cancellation or malformed bridge input can consume a bridge attempt while
leaving the local query unit unlaunched; that unit remains unknown in the fixed
horizon.

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

The interface profile is pinned to LongMemEval-V2 commit
`2cc8c540bdb87fe6761629b585e727e1c4704520`. Historical Python fixtures verified
the full upstream `memory.py` image and exercised its bounded standard-library
prefix, with heavy backend imports excluded. Those results remain historical
evidence and do not establish current Rust, official-runner or native-host
execution.

The current Rust registry and typed traits implement this declared profile
without importing or executing Python. Current native validation is reported
separately. Images are ignored, no image bodies are read, and this text-only
profile is not an official leaderboard or multimodal accuracy test. Logical
evaluator separation is not an OS sandbox.

Local observation times cover backend open, insertion, query and close. Factory
setup, registration, bridge validation/HMACs, accounting/export, waits and project
removal are outside those component spans. Canonical JSON sizes are serialization
proxies, not actual transport or model tokens. Hooks report already booked units;
a future lifecycle controller must join them once and separately observe reader,
judge, retries, truncation, billing and the full fixed query horizon. No token
savings or cost-per-correct result follows from these local checks.
