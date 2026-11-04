# CI and local validation

Production tooling, memory, hooks, generator gates, the Harness, desktop entry
points, and browser business logic are implemented in Rust. The native CLI is
`infra-engine`; browser interfaces use generated Rust/Wasm assets and a small
ABI loader.

## Current phase gate

During the source-first migration, finish every production implementation before
fetching dependencies, building, formatting, testing, checking, or calling a
provider. Record those phases as deferred. A source change or a generated workflow
does not establish a passing build, native Windows evidence, provider behavior,
or memory benefit.

After that gate closes, build the native engine and use its catalog runner:

```bash
cargo build --manifest-path runtime-rust/Cargo.toml --locked --bin infra-engine
./runtime-rust/target/debug/infra-engine check --list
./runtime-rust/target/debug/infra-engine check --strict --base origin/main
./runtime-rust/target/debug/infra-engine check --group lint --group mirrors --strict
```

These examples use Cargo's default target directory. If `CARGO_TARGET_DIR`
is set, select that build's executable instead. On Windows use
`runtime-rust/target/debug/infra-engine.exe`. Rust formatting, tests, and
Clippy use the actual workspace;
PHP snippet validation additionally needs PHP. Optional provider CLIs and
ONNX/vector libraries are needed only for operations that explicitly select them.

## One catalog for local commands and GitHub workflows

[`install/check-catalog.json`](../install/check-catalog.json) defines each
group's native tool requests or bounded process arguments, platform, timeout and
workflow job. [The native emitter](../runtime-rust/crates/infra-tooling/src/checker.rs)
produces the workflow files; edit the catalog or emitter, then regenerate during
the maintenance phase:

```bash
./runtime-rust/target/debug/infra-engine repo write-ci
./runtime-rust/target/debug/infra-engine repo check-ci
```

Each generated job checks out source, provisions Rust, builds `infra-engine`,
then runs `infra-engine check --group GROUP --strict --base origin/main` from a
copy of it in the runner's temporary directory: the group's own Cargo builds
replace the engine in the target directory, which Windows refuses while it runs.
A missing required tool or input fails a strict run. Skipped work is reported
separately from passes. Failed command logs are retained, and each failed check
also prints the last lines of its stdout and stderr on stderr; interrupted
checks are reported as cancelled. `--log-dir DIR` keeps the failed logs in
`DIR` instead of a private temporary directory: each generated job passes one
under the runner's temporary directory and uploads it as the
`check-logs-JOB` artifact when the job fails, because a CI job keeps nothing
once it ends. The Windows workspace tests run with `--no-fail-fast`, so one run
reports every failing test target.

Each job restores its Cargo registry and target directory from the cache
before the build and saves them after the check, also when the check failed,
unless the run found its exact key. The key names the job, the compiler and a
hash of the Cargo manifests, lockfile and catalog; when that hash changes, the
job starts from its newest entry for the same compiler and Cargo rebuilds what
changed. Jobs build without incremental state (`CARGO_INCREMENTAL=0`), which a
CI build never reuses and the cache would only carry.

The `rust` and `windows-native` groups run the workspace tests through
`cargo nextest run`: each test in its own process, as many at once as the
runner has CPUs, across all test binaries, where `cargo test` runs one binary
after another. [`runtime-rust/.config/nextest.toml`](../runtime-rust/.config/nextest.toml)
keeps the conditions some tests were written for: the dispatch custom-guard and
installation cases run one at a time, and at most two end-to-end cases run
together. Doc tests, which nextest does not run, have their own
`cargo test --doc` gate. Generated jobs install a pinned nextest release through
`taiki-e/install-action`, with checksums and no source fallback. A process gate
may name a `fallback` that runs when one of its `tools` is unavailable, instead
of failing or skipping: a checkout without `cargo-nextest` runs the same tests
through `cargo test --tests`, one binary at a time, and the gate's reason names
the missing tool.

| Group | Actual validation |
| --- | --- |
| `rust` | Workspace Rust tests in parallel, doc tests, rustfmt and Clippy. |
| `parity` | Canonical shared-resource parity and generator assets. |
| `mirrors` | Native mirror generation compared with canonical resources for every edition. |
| `installation` | Native inventory contracts for Laravel, Symfony, PHP Core and WordPress. |
| `lint` | Tracked JSON syntax, context ceilings, stabilization structure, routes, registry, PHP snippets and policy identity. |
| `links` | Relative Markdown links, with the selected root's containment rules. |
| `changelog` | Shared-core and edition changelog coverage against the explicit base. |

The catalog is the source of truth if groups change. Historical Python test files
remain reference material. They import retired implementations and are not
registered as an opt-in oracle. A future differential oracle must first isolate
a fixed historical Git tree and prove its fixture/runtime contract; invoking
those files against the migrated checkout is not a useful check.

The manually dispatched [Windows Creator workflow](../.github/workflows/windows-creator.yml)
runs native Rust sandbox acceptance cases on a configured self-hosted Windows
runner. Hosted Rust workspace tests do not prove elevated Codex sandbox behavior;
actual native host and provider results must be reported separately.

## Direct maintenance commands

```bash
./runtime-rust/target/debug/infra-engine repo core-sync --check
./runtime-rust/target/debug/infra-engine repo asset-parity --check
./runtime-rust/target/debug/infra-engine repo mirrors --check
./runtime-rust/target/debug/infra-engine install --verify-inventories
./runtime-rust/target/debug/infra-engine repo registry --check
./runtime-rust/target/debug/infra-engine repo budget --check
./runtime-rust/target/debug/infra-engine repo stabilization
./runtime-rust/target/debug/infra-engine repo policy-lock --check
./runtime-rust/target/debug/infra-engine repo routes
./runtime-rust/target/debug/infra-engine repo php-snippets --require-php
./runtime-rust/target/debug/infra-engine repo links
./runtime-rust/target/debug/infra-engine repo changelog --base origin/main
```

Use `--write` only for a deliberate canonical-resource change, then regenerate
mirrors, inventories and policy identities through the native tools. Never edit
generated mirrors by hand or refresh an inventory to hide an unexplained
production payload change.

Context ceilings remain in [`scripts/token_budget.json`](../scripts/token_budget.json).
`infra-engine repo budget --headroom` reports remaining capacity; a justified
permanent increase updates the ceiling alongside its source change.

## Evaluation and runtime evidence

Maintainer experiments are explicit commands under `infra-engine eval`,
`infra-engine bench`, `infra-engine pilot` and `infra-engine routing-eval`.
They are not default CI gates. Provider runs may cost money and must be invoked
deliberately. Offline dry-runs describe their synthetic inputs and do not prove
routing quality, task benefit, native-save success, or real provider behavior.

Preserve sealed study artifacts, exact hashes, acquisition identities, fixed
denominators, no-retry rules and unknown usage. Keep proof/deadline clocks real;
historical knowledge calendars enter through controlled evaluation requests.
See [memory evaluation](MEMORY-EVAL.md), [migration status](RUST-MIGRATION.md),
and [the runtime workspace](../runtime-rust/README.md).
