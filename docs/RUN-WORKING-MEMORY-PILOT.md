# Run Working Memory pilot

This maintainer-only pilot compares three context variants on the same native
prompt builder. It does not enable a new memory strategy in ordinary system
runs, change the worker-report schema, or transfer existing working state to
Redis. `turn_deltas`, findings, approvals and native receipts keep their current
owners.

The fixed mechanics rehearsal uses the following treatments; the real-worker
example below adds a common producer protocol to both control and treatment.

| Arm | Delivered context |
| --- | --- |
| A | Existing PR #88 context, including complete prior reports until normal condensation. |
| B | A plus a bounded receipt-linked working-memory snapshot through SQLite. |
| C | The same working-memory module and snapshot through Redis. |

A already receives the fixture's observation in its existing report. The
rehearsal never removes that information to manufacture a memory benefit.

## Build and run

SQLite-only rehearsal, using a new or empty output directory:

```sh
cargo run --manifest-path runtime-rust/Cargo.toml --locked \
  -p infra-pilots --no-default-features --features run-memory \
  --example run_working_memory -- --output /tmp/run-memory-sqlite-new
```

For B/C parity, supply a separately created private Redis loopback endpoint;
the example does not discover or start a Redis server:

```sh
cargo run --manifest-path runtime-rust/Cargo.toml --locked \
  -p infra-pilots --no-default-features --features run-memory-redis \
  --example run_working_memory -- --output /tmp/run-memory-redis-new \
  --redis-endpoint 127.0.0.1:16379 --rounds 30
```

The endpoint above is illustrative; use the numeric port of your private test
instance. The pilot accepts only numeric loopback addresses without credentials.
Remote/TLS/ACL deployments are outside this pilot. The Redis adapter uses the
pinned synchronous `redis-rs` client; that dependency is optional and absent
from the ordinary CLI dependency graph.

After stopping only that private test instance, repeat against the retained
output with `--recover`:

```sh
cargo run --manifest-path runtime-rust/Cargo.toml --locked \
  -p infra-pilots --no-default-features --features run-memory-redis \
  --example run_working_memory -- --recover \
  --output /tmp/run-memory-redis-new --redis-endpoint 127.0.0.1:16379
```

A completed initial rehearsal writes `result.json`, exact A/B/C inputs and
matching digest receipts. Recovery writes separate `recovery-*` files and
refuses existing output files. `BC_snapshot_equal` and `BC_prompt_equal` are
null when C was not run. An unavailable C reports `local_fallback`; it is not
successful Redis delivery.

## Real-worker development experiment

The separately enabled `run_working_memory_live` example uses the actual native
planner, dispatch runner and installed model CLI. It launches real workers and
can edit the selected service roots. Use an isolated copy of the complete
system, including each service's dirty source changes, and a fresh run directory:

```sh
cargo run --manifest-path runtime-rust/Cargo.toml --locked \
  -p infra-pilots --no-default-features --features run-memory \
  --example run_working_memory_live -- \
  --system /tmp/harbor-copy/harbor-system/system.json \
  --workspace /tmp/harbor-copy --output /tmp/harbor-run-a \
  --task-file /tmp/harbor-task.txt --services inventory,harbor-payments \
  --provider codex --model gpt-6.1-sol --effort ultra \
  --arm A --change-id memory-stock-customer --timeout 480
```

Repeat against a separate identical initial copy with `--arm B` for the SQLite
view; C additionally requires `run-memory-redis` and `--redis-endpoint`.
Both A and B request the same producer protocol and keep complete native prior
reports. A withholds only the additional view. This is a comparison of the
shared producer protocol with/without its admitted view; A is not the exact
unmodified PR #88 prompt used in the earlier fixed rehearsal.

Experimental worker reports pass the full domain secret policy before native
receipt persistence; rejected experimental diagnostics are generic, including
parse/schema failures. The ordinary non-experimental report path is unchanged.

The model appends a one-line ` RUN_MEMORY_V1=<JSON array>` suffix to the existing
`summary`. It may emit zero to three observations with the existing typed fields
and actual citation digests. The report's 4000-character limit and native schema
remain unchanged. Completed canonical receipts are imported after native finish
and replayed before the next prompt, closing the finish/import crash window.
Malformed, missing or stale annotations are recorded as rejected; they never
become claims in the delivered view. Both the producer instruction and view count
toward the existing native prompt budget.

The runner's optional `DispatchContext` policy finalizes the prompt before its
native digest and exact-input file are saved. The regular CLI/Harness passes no
policy. Context-enabled runs skip native outcome sharing: they create no verified
Brain outcome finding, promotion proposal or Memory Bank chunk from these worker
claims. A run-bound private marker preserves this rule even on ordinary resume.
Native task completion and original receipts remain available as evidence. The pilot namespace derives from the validated native `system_file`,
without inserting an unsupported `project_id` into the native journal.

The controlled example supports Codex, with explicit `--model` and `--effort`,
`--ephemeral` and `--ignore-user-config`. Authentication remains the installed
CLI's; mutable global project-trust entries do not choose experiment settings.
The example saves safe lifecycle summaries, numeric token usage when emitted,
and command hashes/exit status, without raw provider/tool output. Command hashes
are not a precise source-read trace or proof of redundant investigation. A
blocked native run exits unsuccessfully while retaining its pilot result.

Evaluate outcomes independently after workers stop. Place oracle code outside
worker roots/context and run it on a disposable candidate copy. The standard
Codex sandbox can read the host filesystem; merely placing an oracle elsewhere
does not make it adversarially inaccessible. These development runs are not a
sealed held-out evaluation or grounds to enable admission heuristics by default.

## Interface and authority

`infra_pilots::run_working_memory::RunWorkingMemory` owns admission, journal
acceptance, replay, logical expiry, deterministic projection and bounded view
publication. Its two real `SnapshotStore` adapters are `SqliteStore` and
`RedisStore`. The caller supplies trusted coordinator run state and explicitly
chooses the adapter; a storage namespace is not remote authentication.

The producer calls `append` after a native dispatch has a matching successful
or explicitly accepted receipt. Each observation has a topic key, a kind
(`progress`, `hypothesis`, `open_question`), `open/resolved` status, bounded text
and one to four canonical citations. Canonical citations exclude dependency,
secret and local-state paths and Brain/Memory Bank folders at any depth.

Observations are separate annotations; the unchanged native report does not
contain their complete text, kinds, status and expiry. The indispensable
record is therefore **the private pilot journal plus its native receipt
linkage**, not native receipts alone. The fixed rehearsal supplies annotations through
its fixture/coordinator code. The real-worker example instead retains the
producer annotations in the unchanged native report summary. Its private journal
still owns acceptance and original expiry. Neither path accepts observations
from an unfinished worker process.

Acceptance commits the journal before publishing a disposable view. Identical
replay preserves the original expiry; conflicting replay is refused. Latest
state follows canonical `state.steps` position and attempt, so delayed first
delivery of an older dispatch cannot reopen a later resolution. Topic ownership
is `(producer service, key)`; automatic cross-service resolution is not part of
this pilot. A coordinator may resolve its own question after reading service
reports, retaining that statement as a worker claim.

Scope binds project, workspace identity, run directory, run ID, generation,
selected services and roots. Every consumption revalidates current receipt
identity and source bytes/containment/secrets. Expiry, source invalidation and
run completion are checked before backend lookup. An expired newer resolution
does not resurrect an older open question. These are cooperative, point-in-time
filesystem checks, not distributed transactions or atomic source snapshots.

## Retention and limits

- At most 256 receipt envelopes and 2 MiB of strict journal JSON.
- At most 12 annotations per receipt, 320 characters per annotation and four citations.
- At most 12 delivered entries and 8192 bytes for the complete snapshot.
- Logical observation lifetime: positive milliseconds up to 24 hours.
- Overall native prompt and Cursor byte limits still apply, including condensation.

TTL controls disposable view retention. Journal expiry is evaluated separately,
so rebuilding a missing view cannot revive an expired observation. The journal
retains annotations for recovery/audit; TTL is neither journal deletion nor
secure erasure. SQLite removes expired view rows during publication; Redis uses
expiring immutable version keys. Findings and Memory Bank promotion are never
performed by the pilot.

Cache hits are compared against the fully reconstructed current allowed view.
The pilot deliberately repeats journal/receipt/source checks before trusting
backend bytes. It establishes safe interchangeable representations, not an
acceleration of those checks or a central Redis authority.

SQLite busy waits and Redis connect/read/write operations each have a 100 ms
timeout. These do **not** establish a total snapshot/fallback deadline: load
and publish can consume separate intervals, filesystem work has its existing
contracts, and slow trickled network replies have not been measured. Only a
private trusted loopback instance is supported.

## Verification and acceptance

```sh
cargo test --manifest-path runtime-rust/Cargo.toml --locked \
  -p infra-pilots --no-default-features --features run-memory-redis \
  --lib run_working_memory
```

The explicitly ignored Redis check requires a private endpoint and real server:

```sh
INFRA_RUN_MEMORY_REDIS_ENDPOINT=127.0.0.1:16379 \
  cargo test --manifest-path runtime-rust/Cargo.toml --locked \
  -p infra-pilots --no-default-features --features run-memory-redis --lib \
  run_working_memory::tests::redis_live_matches_sqlite_and_rebuilds_expired_and_corrupt_views \
  -- --ignored --exact
```

Coverage includes replay/conflicts, narrowed selected scope, generation,
changed source/receipt, escaping links and replaced roots, out-of-order
resolution, logical expiry, secret-bearing annotations, nested memory paths,
poisoned backend data and prompt budgets. The real Redis check exercises
corruption and TTL expiry, not maxmemory eviction, partitions or failover.
The stopped-server rehearsal verifies local reconstruction after unavailability.

In the fixed rehearsal, the consumer is a deterministic command checking fixture signal presence. A
passing check does not prove the observation true or replace a real worker's
canonical-source verification. Timings are prompt-build costs on one synthetic
case, not task completion, model tokens or efficacy. Extra context can increase
prompt size without adding knowledge.

New context admission/budget behavior remains opt-in until its own sealed
held-out task-outcome comparison under [MEMORY-EVAL.md](MEMORY-EVAL.md#accepting-a-retrieval-change).
The existing outcome runner has fixed hook treatments; no generic
`--arms baseline,sqlite,redis` command is introduced. Adoption requires an
appropriate system-run workload, independent oracles and predeclared criteria.
