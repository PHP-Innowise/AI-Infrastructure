# Local scheduler/inbox A/B runner

`infra-harness-ab` is an offline maintainer companion, not an installed
accelerator command. It compares the same native Sessions implementation at
concurrency **1** and **2**. Both arms use the same inbox, source files,
instructions, edit authority and resource settings. It currently accepts only
the built-in fixture worker; it cannot launch a live model.

## Run locally

Linux, Git, an installed Rust toolchain, a C linker driver at `/usr/bin/cc` and
`/usr/bin/bwrap` with working user namespaces are required. No network, provider login or API key is needed.
Generated Linux workspace-test CI installs bubblewrap, enables user namespaces
on its ephemeral runner when AppArmor restricts them, and probes the boundary.

```bash
cargo build --manifest-path runtime-rust/Cargo.toml -p infra-harness \
  --bin infra-harness-ab --bin infra-harness-ab-oracle --locked --offline
AB_ROOT=$(mktemp -d /var/tmp/harness-ab.XXXXXX)
runtime-rust/target/debug/infra-harness-ab prepare "$AB_ROOT/sealed" gold
runtime-rust/target/debug/infra-harness-ab run "$AB_ROOT/sealed/plan.json" "$AB_ROOT/result"
```

The two binaries must remain siblings. If copying them, copy both before
preparation. A plan pins their contents and the oracle helper's absolute path.
Changing the binary, declared compiler identity, fixtures, oracle, settings or
slot membership requires a new plan. Output directories must not exist; the
controller refuses overwrite and has no automatic retry/resume operation.

`prepare` writes `plan.json`, containing the complete public pilot corpus,
reference patches, oracle cases/expected output, fixture policy, event rules,
limits, compiler/helper/bwrap executable hashes and four slots in **ABBA** order:
serial, parallel, parallel, serial. The SHA-256 seal detects changes; the runner
also reconstructs the supported design, so recomputing the seal cannot admit
arbitrary commands or modified tasks. This is a reproducibility check, not a
signature or protection against someone changing the runner itself.

## Pilot tasks and independent grading

The dataset lives in `runtime-rust/crates/infra-harness/fixtures/session-ab/`.
It contains one public stream with three prompt stages:

| Stage | Project | Task |
| --- | --- | --- |
| A1 | A | Fix `compress_runs`: preserve the final consecutive run |
| A2, accepted through inbox while A1 runs | A | Implement bounded `expand_runs`, retaining A1 behavior |
| B | Independent Git repository | Fix strict ASCII duration parsing and checked seconds conversion |

A1 is held at a fixture gate until B is created and A2 is durably accepted.
The same event rule applies in both arms. The fixture writes predetermined
patches; it reports provider success even for incorrect/no-op controls.
Native processes, guardians, scheduling, inbox assignment, launch generations,
cleanup and receipts are real.

Grading uses the **final A1+A2 artifact** and B's final artifact. It does not
infer intermediate A1 correctness from files A2 may already have changed.
After native drain, bounded ordinary files are copied into a separate oracle
directory. Required files, hardlinks/symlinks, size limits, extra files and the
protected README are checked before execution. Only `source.rs` may change.
Git internals are excluded from semantic grading; this runner does not certify
arbitrary Git metadata edits or repeat every scope/generation proof covered by
the separate `native_flow` qualification.

The controller owns expected results. A trusted wrapper compiles with direct
`rustc`, then a second process executes the program. Both stages run in
networkless bubblewrap namespaces with a read-only toolchain/system runtime,
read-only inputs and private scratch. Runtime sees its compiled binary, not
the project, gold patch or controller. Its separate, dataset-free
`infra-harness-ab-oracle` helper contains no corpus or expected outputs. That
helper uses the existing native process supervisor and quotes child output in
a receipt, allowing setup/exec failure to remain distinct from a wrong answer.
The host compares the entire output with all expected responses. Exit code 0,
provider text and a forged test-summary string are insufficient.

Preflight proves baseline A/B fail, gold A/B pass, partial A, early exit and
64 KiB binary output fail;
it also exercises missing/protected/extra/oversized/linked artifacts and a
missing compiler or linker; an unusable toolchain is an unknown oracle result,
never a failed artifact. Failed preflight leaves all four planned streams
unattempted.

This finite, open corpus is **pilot qualification**. An implementation tailored
to print its known expected outputs can pass it. It is not a hidden test set or
a proof of arbitrary implementation semantics. Compiler executable/version,
the resolved C linker, helper and bwrap are pinned; system/toolchain libraries
are not fully sealed by content. rustc links through `cc`, and Debian-family
hosts resolve `/usr/bin/cc` through `/etc/alternatives`, which the sandbox does
not mount, so the controller resolves it on the host, requires the result inside
a mounted runtime directory and passes it with `-C linker`. Namespace isolation and deadlines are not a hostile-code resource
containment certification.
System runtime directories are bound from their actual host contents, including
`/lib64`; the oracle does not assume one distribution's dynamic loader layout.

## Reports and failure accounting

`report.json` is journaled before preflight and before/after each slot. It starts
with `finished: false` and unavailable usable grades. Four slots remain in the
denominator even when interrupted, unattempted or unknown. Bindings are checked
before every slot and at completion. A missing/changed pinned input invalidates
usable aggregate grades; raw observations and the binding error remain visible.
An unfinished journal is incomplete evidence, not a completed experiment.

Each slot keeps private Git projects, state/attempt SQLite databases, launch
receipts, copied artifacts, compiler/runtime output and oracle receipts. Known
artifact errors are incorrect. Missing verifier infrastructure or uncertain
cleanup are unknown. The controller's explicit native execution deadline is
incorrect only after cleanup is proved; typed budget observations are handled
the same way. A native failure whose cause is unavailable remains unknown:
the public store does not preserve every native error code, so this runner
does not infer a per-turn timeout from text or elapsed time.

The `deadline` fault policy uses a 1-second controller native deadline and
15-second per-turn limits; ordinary policies use 40 and 15 seconds. Native
cleanup/drain and external verification have separate bounds. Oracle child
execution is bounded at 8 seconds/64 KiB, its enclosing supervisor at 20
seconds/512 KiB (including worst-case JSON escaping of child output). These
are smoke-test limits, not an efficacy acceptance margin.

Request-to-verification timings use the controller's monotonic clock and
include shared native drain and predeclared ordered verification (A then B).
They are batch verification endpoints, not independently observed intermediate
task completion times. Raw launch receipts retain observed lifecycle/usage
coverage. Unknown provider tokens, total billing and cost per correct task stay
null; missing usage does not mean zero.

There are **two counterbalanced pairs of the same single stream**, not four
independent tasks. The report always states `efficacy_verdict: not_evaluated`
and leaves latency benefit null. It computes no significance, adoption decision
or p95 over selected successful cases.

For deliberate controls, pass one of `no-op`, `partial`, `wrong-b`, `forbidden`,
`exit-zero` or `deadline` to `prepare` instead of `gold`. The chosen policy is
sealed and applied equally to both arms. These are controller tests, not model
observations.

```bash
cargo test --manifest-path runtime-rust/Cargo.toml -p infra-harness \
  --test session_ab --locked --offline
```

The next live-provider stage needs an explicit provider/executable/model/effort
identity, account-state and filesystem-isolation contract, independently
qualified init/resume/cancel behavior, and a separately sealed pilot protocol.
Holdout membership, sample size and practical quality/latency criteria follow
pilot evidence; this public fixture supplies none of those conclusions.
