# Native Rust runtime migration

The executable implementation moves to [`runtime-rust/`](../runtime-rust/README.md).
The public binary is `infra-engine`. Installed projects launch the explicit
`memory-bank/bin/infra-engine` artifact (`.exe` on Windows).

This replaces production Python modules, shell business logic and browser
business JavaScript. Shell files retained as compatibility entry points only
locate and exec the native engine. Browser JavaScript consists of generated
WebAssembly bindings and a small boot loader. PHP target-project examples,
declarative policies, schemas, specifications and historical experiment
evidence remain their original formats.

| Former entry point | Native command |
| --- | --- |
| `memory-bank/scripts/context.py` | `memory-bank/bin/infra-engine memory` |
| Bank validation/index scripts | `memory-bank/bin/infra-engine bank validate` / `bank index` |
| MCP Python bootstrap | `memory-bank/bin/infra-engine mcp --root PROJECT --host HOST` |
| Continuity helper | `memory-bank/bin/infra-engine continuity` |
| Root repository scripts | `./infra-engine repo OPERATION` |
| Infrastructure-Creator deterministic helpers | `./infra-engine creator OPERATION` |
| Harness Python package | `./infra-engine harness` / `./accelerator-app` |
| Memory evaluation and benches | `./infra-engine eval FAMILY` / `bench FAMILY` |
| QA workbook scripts | `./infra-engine qa OPERATION` |
| Vendored skill helpers | `./infra-engine skill-tools OPERATION` |

The Rust crates own storage, retrieval, lifecycle, proofs, source admission,
process supervision, installation, publication and browser interaction.
There is no Python runtime fallback. The native artifact must be explicitly
built or supplied before installation. Installer plans capture actual source
bytes, modes and identities; reviewed plans publish under conflict checks.
Generated projects receive the engine, native contract, schemas and a bounded
declarative hook policy. Hook and MCP commands are materialized against the
final target root, including when generation uses a temporary staging root.

Project Brain Markdown/JSON authority and Memory Bank chunks stay authoritative;
SQLite's indexed knowledge is rebuildable. The database also holds unfinished
turn deltas and local working/session state. Preserve and recover that state
during migration before replacing a database; rebuilding the knowledge index
alone does not recover unfinished local work. Existing data formats and frozen study
artifacts retain their attribution. Public command names and data contracts
are preserved where represented by the native facade. Human report layouts
may differ; no byte-for-byte or performance claim is implied by the port.

The root `memory-kernel-probe` package retains the initial primitive ABI
contracts. Its old opt-in Python launcher is retired; the full runtime uses
the Rust library directly.

Implementation is written before validation. Build results, behavioral
regressions, integration checks and native-host evidence must be reported
separately. Historical Python tests describe the former implementation and
are not silently counted as passing native tests. Provider availability,
ONNX assets and native Windows execution are only established by actually
running the relevant environment.

See [native host compatibility](NATIVE-HOST-COMPATIBILITY.md) for launcher
constraints and [the runtime README](../runtime-rust/README.md) for builds,
WebAssembly assets and distribution preparation.

## Initial local validation, 2026-11-02

The table records completed initial-port checks. Additional user-authorized
compatibility hardening is active; findings and new regressions are being
reproduced and corrected. These results do not establish that every later
working-tree edit has passed the same full suite.

The implementation and production caller switch were completed before builds
and validation began. Validation used Linux x86_64, Rust/Cargo 1.99.1, the locked
dependency graph and wasm-bindgen 0.2.130. The declared minimum Rust version was
not separately tested.

| Check | Observed result |
| --- | --- |
| Release engine build, locked and offline | Passed |
| Full workspace, all targets, locked and offline | 122 passed, 0 failed, 4 ignored Windows cases |
| Core and Harness library tests after final native-command guidance fixes | 12 passed, including 2 added regressions |
| Relocated executable CLI tests after help and desktop-argument fixes | 6 passed, including the added help/no-side-effects regression |
| Workspace/all-target Clippy with warnings denied | Passed after the initial port fixes |
| Workspace formatting | Passed |
| Strict parity, mirrors, installation, lint, links and changelog catalog groups | 23 passed, 0 failed, 0 skipped |
| Native WebAssembly builder | Passed; ABI manifest and generated bindings published with the Wasm artifact |
| Disposable installed PHP Core/Codex target | Dry-run, installation, memory start/update/retrieve/complete passed |
| Installed MCP stdio | Initialization, all 5 declared tools, memory status and outside-root refusal passed |
| Visible in-app browser | Custom-model state, SDD mode transitions, PHP Core/Codex installation preview and review-feedback round trip passed |

The full workspace run includes actual CLI generator validation and installer
publication/collision checks. Changes after that run corrected native command
guidance and new-record attribution, and made public help return before any
application actions while rejecting unsupported desktop mutator arguments.
Affected library and relocated-executable CLI tests, plus strict Clippy, were
rerun. Historical Python suites were not counted as native
test results. Existing SQLite schema and migration compatibility received
independent source review; this does not establish migration of every real
user database.

Repository reproduction commands, after building the native engine:

```bash
cargo test --manifest-path runtime-rust/Cargo.toml --workspace --all-targets --no-fail-fast --locked
cargo clippy --manifest-path runtime-rust/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo fmt --manifest-path runtime-rust/Cargo.toml --all --check
./infra-engine check --group parity --group mirrors --group installation --group lint --group links --group changelog --strict --base origin/main
```

Native Windows and macOS behavior, remote CI, real provider inference and
optional model assets remain unverified in this local run. No performance,
complete behavioral equivalence or frozen-study outcome claim follows from
these checks. Tests used disposable projects and state; no user trust records
were written. The migration remains local, without a commit or push.


## Compatibility and integration follow-up, 2026-11-03

The 2 November table above is the initial-port snapshot. The results below
are local evidence for their recorded source or artifact identities; an
earlier passing run does not certify subsequent edits. Historical Python
tests are not counted as passing Rust tests.

| Check on the latest source/artifacts | Observed result |
| --- | --- |
| Full workspace, all targets, locked and offline | 520 passed, 0 failed, 5 ignored and 0 filtered across 51 completed targets; Cargo exited 0 at 05:26:02 UTC. Four ignores are native Windows acceptance cases (three Creator sandbox, one owner-death); the fifth is the explicit disposable workbook/browser-fixture materializer. |
| Native Rust catalog orchestration | 4 passed, 0 failed, 0 skipped, not interrupted; exit 0 at 05:42:27 UTC, wall time 977.643553 seconds. Workspace, production OOXML, formatting and strict Clippy gates all passed. |
| Formatting and strict Clippy | Formatting passed. Linux workspace/all-target Clippy passed in 20.49 seconds; Windows GNU Clippy with Rust 1.99 passed in 29.68 seconds. |
| Minimum Rust 1.89.0, all targets | Linux check passed in 29.12 seconds; Windows GNU cross-check passed in 29.23 seconds. Toolchains were isolated without changing global defaults. |
| Release engine, native UI and distribution | Linux release built in 108 seconds: 47,469,944 bytes, mode 0755, SHA-256 `fc137a8517c786fd4646d1d6d3b55cb806990500c32b7e3c26658337e41bf287`. All 8 copies and descriptors independently verified. Native UI retains source `a453e5e2…`, bindings `be3fbba7…` and Wasm `6d16eb41…`, matching the later target-reset browser proof. |
| Static catalog, generated workflows and inventories | All 24 static gates passed: 0 failed, 0 skipped, in 28.7317 seconds. CI templates matched and all 4 inventories verified. |
| Installed CLI/MCP smoke | All 4 editions passed on release `fc137a85…`: 40 native-process invocations plus 12 Git fixture-setup calls, including 8 expected native stale-CAS/outside-project refusals, with the full Unicode notice present in each installed edition. No unexpected nonzero result or timeout; private host roots and project sources stayed unchanged. |
| PHP snippet-oracle controls | The true-profile and false-profile controls both returned 0, in 3.329 and 0.200 seconds respectively. |
| Optional CPU ONNX smoke and negative controls | Actual tokenizer/CPU ORT inference passed with release `fc137a85…`; finite scores, exact passage-permutation consistency (maximum difference 0) and corrupted-asset hash refusal passed. No provider calls or task-benefit claim. |
| Visible browser fixture | At 05:39 UTC the boot-referenced final `6d16eb41…` Wasm passed visible Memory filter/reset and stale Creator response A→B→A discard controls; explicit Refresh loaded the current A review. Separate direct HTTP probes returned 403 for missing CSRF and foreign Origin (with a valid token) before any Services call. Local-only receipt: `tasks/native-ui-browser-final-evidence.json`. No Creator mutation, provider or native publication was exercised; final-Wasm Excel behavior and download completion remain unverified. |

Focused regressions reproduced concrete failures before the corresponding
fixes and were checked separately from that final workspace total:

- Strict JSON: before 2 passed / 4 failed; after all 6 passed, preserving
  finite fractions, exact big integers, literal reserved-key objects and
  duplicate/nonfinite refusal.
- Pinned Unicode 16.0.0 default full, non-Turkic case folding: before 0/4,
  after 4/4 in 1.37 seconds. Its 156 exact overrides extend the pinned
  Unicode 9 mapping; an independent comparison covered all 1,112,064 Unicode
  scalars. Tests preserve legacy hashes, conflict detection and duplicate
  reuse. The complete Unicode notice/license accompanies the implementation
  and canonical installed MCP documentation; all four final installed editions carried that notice.
- Creator capability admission: an intermediate workspace run was
  **510 passed, 1 failed, 5 ignored**, exposing `ETXTBSY` (errno 26). Valid
  inherited-writer controls then reproduced 3 passed / 2 failed; after the
  fix all 5 passed in 0.10 seconds. Admission retains the same private image
  and original five-second deadline, with mutation, permission-denied and
  nonzero-exit controls preserved. Only five intentional inheritance
  fixtures are serialized; this is not a passing final-workspace claim.
- Terminal dispatcher continuation: 4 private-PTY regressions passed in
  73.19 seconds. Legacy sync improved from 1 passed / 2 failed to 3 passed;
  declared custom hooks improved from 1 passed / 4 failed to 5 passed.
  Edited runtime/wiring/custom guards and local Brain/SQLite images remain
  held pending review; explicit external project hooks are not a fallback
  to this repository's retired Python runtime. Fixtures write no user trust.
- PHP pipe EOF: all 3 controls previously timed out near 5.01 seconds;
  after releasing the retained parent command handles, all 3 passed in
  0.01 seconds. Seven focused ONNX lifecycle controls passed in 0.51 seconds,
  including quiet workers, inherited-output descendants and panic cleanup;
  Windows-specific zero-grace cases were compiled only. Startup, I/O and
  cleanup still share the original capped five-second request deadline.
- Public Outcome seal/consolidation with the selected real engine and
  quality margin `0.1` passed in 83.21 seconds; an earlier integer-margin
  probe had failed snapshot admission in 34.45 seconds. Canonical NativeAB
  engine-budget coverage passed in 38.42 seconds. These are provider-free
  boundary checks, not task-outcome measurements.

Earlier artifact proofs remain independently useful. Linux release
`6b77c1473d2adb353327fb66e114342e011dfa4e2d032cc72986583312743d1b`
was 47,460,024 bytes, mode 0755, with eight assembled copies hash-verified.
On that snapshot, all 24 static gates passed (0 failed/skipped), generated
workflow parity passed and four inventories verified. All four disposable
installed editions passed 52 bounded calls, including fixture Git setup and
eight expected stale CAS/outside-project refusals; tasks completed at revision 3 with one episode
and `working=0`, without changing project sources or private host roots.
These results predate the final Unicode/Creator changes and must not be
silently attributed to a newer release hash.

Evidence paths under `tasks/` and artifacts under `target/` below are local-only,
ignored files; they are not expected to exist in a clean checkout.

The optional ONNX proof used the actual Rust tokenizer and CPU ORT 1.31.0
worker with `jina-tiny-fp32`, publisher revision
`aca45de6945b5dc6399abcd2a9c55ded5dc9111f` and all five compiled file pins.
The [official CPU archive](https://github.com/microsoft/onnxruntime/releases/expanded_assets/v1.31.0)
matched SHA-256
`cc5c72baf5ae5c8238a6841f0897227be2d02826b9cf98eaf02fdefaeeb03a57`.
`tasks/native-final-onnx-evidence.json` binds the tested worker and library.
Three explicit public synthetic passages returned finite logits, their
permutation had maximum difference 0, and an equal-size corrupted-config
copy returned status 1 / `hash-mismatch`; original assets stayed unchanged.
The first smoke's exact input was not persisted, so it is not a paired
equivalence comparison with that earlier run. No provider was invoked and
no retrieval improvement, task benefit or historical-worker parity follows.

The earlier 01:58 UTC target-reset browser proof used native UI source `a453e5e2…`,
bindings `be3fbba7…` and Wasm `6d16eb41…`, subsequently reproduced by the
28.0675-second UI build. It covers held-editor ownership, released System
reentry, same-target preservation and cross-target plan reset through
production HTTP/CSRF/static Wasm with ScriptedServices. The agent's tab was
closed and the fixture server exited 0; the person's tab was untouched.
This does not exercise a native provider or native project publication.

For the separate 05:39 UTC fixture, the local-only receipt
`tasks/native-ui-browser-final-cleanup-evidence.json` binds the unchanged
primary receipt and records owned fixture exit 0, refused connections to
both owned ports and closure of the agent's own tab. The person's ambient
tab was never claimed or closed.

Portable Windows codec/private-directory controls passed, while COM and
ACL runtime cases were compiled only. The final GNU/LLVM-MinGW UCRT cross-link
completed in 31.32 seconds and produced a 121,565,696-byte PE with SHA-256
`8841b9224ca2d32b8923738352205dfc60a6602e555547fe49e8df05ca50e3e7`.
It requires UCRT/libunwind; native Windows execution and standalone deployment
were not verified. macOS execution and remote CI were also not established.

Ordinary process capture keeps bounded split streams, the evaluator's default
strict native-event protocol and observed exit values: a genuine status 125
stays 125, while evaluator signal termination uses the signed `-signal`
projection. The scheduled Unix interruption experiment deliberately shares
the caller's session using owned anchor/observer groups, unlike the historical
`setsid` experiment. That separate experiment does not redefine ordinary
capture or establish session equivalence.

A separate offline release built in 183 seconds from a captured-index export
of 3,721 files verified before and after: 12 workspace crates and registry
dependencies compiled afresh without the original target, ignored sources or
`.git`. Its private 47,486,048-byte binary has SHA-256
`9a2bb9f3615f33f3a4fb1ce47729d140370504855da064968dca2db7d29f304b`,
different from the main `fc137a85…` release; this is not a bitwise
reproducibility claim. Capabilities, help and the corrected context-budget
command passed; the initial unsupported `--json` budget probe returned 2.
The local-only receipt `tasks/native-clean-index-final-ZReU553r-receipt.json`
preserves both attempts and the unchanged index/source checks.

A copy of that clean snapshot also passed native artifact assembly, core/asset
and mirror synchronization, policy checks and all four inventory verifications.
All eight private engine copies matched `9a2bb9f…`; one disposable PHP Core
installation passed its memory/CAS/completion cycle and MCP initialization,
five-tool contract, installed scopes and outside-project refusal. The 24 bounded
calls included two expected refusals, with no unexpected failure or timeout.
The local-only receipt `tasks/native-clean-distribution-final-evidence.json`
records this separate private-engine proof; the original release stayed unchanged.

Offline reproduction requires the locked dependency cache, selected Rust
toolchains, `wasm32-unknown-unknown`, matching `wasm-bindgen`, Git, PHP and
the local `origin/main` reference. Before static checks, assemble the selected
engine/UI and regenerate distribution data from intentionally selected Git
index membership; ignored binaries and host-local descriptors stay untracked.

```bash
cargo test --manifest-path runtime-rust/Cargo.toml --workspace --all-targets --locked --offline
cargo clippy --manifest-path runtime-rust/Cargo.toml --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --manifest-path runtime-rust/Cargo.toml --all --check
./infra-engine repo check-ci
CARGO_NET_OFFLINE=true ./infra-engine check --group parity --group mirrors --group installation --group lint --group links --group changelog --strict --jobs 1 --base origin/main
```

The six static groups contain 24 gates, separate from unit/integration totals.
Together with the 4 completed Rust gates, the two orchestrations passed all 28.
Rust (4 gates), Wasm (1) and MSRV (1) are separately selectable groups.
The MSRV group needs Rust 1.89.0 available through `rustup`; private direct
compiler runs use explicit `RUSTC`, `RUSTDOC` and separate target directories.
Set `INFRA_ENGINE_BINARY` for a nondefault release target; Windows uses the
explicit `.exe`. `./infra-engine repo build-ui` publishes the checked
bindings/Wasm/ABI-manifest set and defaults to locked, offline execution.

A delivery diff audit recorded 24 non-gating whitespace warnings across 23
imported payload or historical-template paths. Those bytes were intentionally
preserved; `git diff --check` was not clean. Rust formatting and the required
lint/catalog checks passed.

Local inference, cross-compilation, scripted UI and disposable integration
fixtures do not establish live client/provider behavior, native Windows/macOS
operation or task benefit. No user trust writes, global installation,
publication, commit or push is implied.
