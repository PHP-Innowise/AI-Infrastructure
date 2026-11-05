# Rust runtime

The executable implementation is a Rust workspace. The public native binary is
`infra-engine`; installed projects use
`memory-bank/bin/infra-engine` (`.exe` on Windows). The shell launchers at the
repository root only locate and exec that binary.

The workspace separates memory and Project Brain (`infra-core`), process and
provider adapters (`infra-host`), repository tooling and installation
(`infra-tooling`), Infrastructure-Creator (`infra-generator`), declarative
orchestration (`infra-system`), the local application (`infra-harness`),
evaluation and optional pilots, QA workbook tooling, skill tools, the CLI,
and the browser interface (`infra-ui`, compiled to WebAssembly).

Requires a Rust toolchain supporting every locked dependency and at least
Rust 1.89, plus Git. SQLite is bundled. Provider CLIs, browser launchers,
PHP documentation linting, sandbox tools and local ONNX assets are required
only by the commands that use them. No command installs Python packages or
falls back to the retired implementation.

Build the engine from the repository root:

```bash
cargo build --manifest-path runtime-rust/Cargo.toml --locked --release --bin infra-engine
./infra-engine capabilities --json
```

The default build includes local ONNX inference. For a smaller build that does
not need local ONNX models, select the CLI package without default features:

```bash
cargo build --manifest-path runtime-rust/Cargo.toml --locked --release --package infra-cli --bin infra-engine --no-default-features
```

Add `--features onnx` to enable it explicitly. Memory, hooks, repository tools
and the application remain available in either variant; a lean local worker
returns the existing `dependency-unavailable` response for valid inference
requests. External ONNX workers remain usable. `capabilities --json` includes
`onnx-inference-v1` only when the compiled pilots dependency supports inference,
so running engines advertising that marker require it in replacement artifacts.
Earlier engines without the marker cannot distinguish full from lean this way.
Source identity names the source tree; the executable hash and capabilities
distinguish the two artifacts.

Cargo unifies features across selected packages. Ordinary `cargo test
--workspace` therefore enables the standalone packages' ONNX defaults; use
`--workspace --no-default-features` to validate the lean workspace.

`cargo nextest run --workspace` runs the same tests in parallel across all test
binaries (`cargo install cargo-nextest --locked`). [`.config/nextest.toml`](.config/nextest.toml)
keeps two binaries serial. Linux admits four end-to-end cases and starts the
long generator lifecycle first; Windows keeps two end-to-end cases, starts
its two longest generator cases first and runs the evaluation cases one at a
time after the rest. Other platforms retain the two-case limit. Doc tests still run through `cargo test --doc --workspace`.

For faster local optimized builds, select the opt-in `quick-release` profile.
It inherits the release settings, disables LTO and uses 16 codegen units. This
trades binary size and potentially runtime performance for shorter compilation;
distribution builds continue to use `--release`.

```bash
# Full build with local ONNX inference
cargo build --manifest-path runtime-rust/Cargo.toml --locked --package infra-cli --bin infra-engine --profile quick-release
runtime-rust/target/quick-release/infra-engine capabilities --json

# Lean build
cargo build --manifest-path runtime-rust/Cargo.toml --locked --package infra-cli --bin infra-engine --profile quick-release --no-default-features
runtime-rust/target/quick-release/infra-engine capabilities --json
```

These paths assume Cargo's default target directory. With `CARGO_TARGET_DIR`,
run its `quick-release/infra-engine`; on Windows the executable is
`quick-release/infra-engine.exe`.

The maintainer launcher accepts an explicit `INFRA_ENGINE_BINARY` when the
binary lives elsewhere. Installed hooks and MCP registrations use the
explicit installed artifact. Installing an accelerator does not build or
download an engine.

Prepare the browser artifacts with the native builder:

```bash
./infra-engine repo build-ui
```

The builder requires the `wasm32-unknown-unknown` target and the
`wasm-bindgen` CLI version selected by `Cargo.lock`. It writes the generated
JavaScript glue, WebAssembly and their digest manifest together under
`runtime-rust/target/wasm-ui/`. JavaScript is limited to generated bindings
and the boot loader; interaction and rendering logic live in `infra-ui`.

Build distribution artifacts explicitly before installation:

```bash
./infra-engine repo artifacts --write --include-generator
./infra-engine repo core-sync --write
./infra-engine repo policy-lock --write
./infra-engine install --write-inventories
```

Assembly updates the canonical seed engine and its descriptor. After each new
engine build, synchronize the generator assets and tool mirrors before deriving
policy identities and inventories. `core-sync --write` includes that synchronization.

Assembly metadata describes the binary built on this host.
`memory-bank/runtime-capabilities.json` stays outside ready-edition inventories.
Installation identifies the selected engine in
`memory-bank/local/accelerator-install.json` through its `engine_sha256` and
`engine_capabilities` fields. Seed assets use their separate
`native-artifacts.json` descriptor to verify the bundled engine by hash.

Each engine also names the sources it was compiled from. `crates/infra-cli/build.rs`
embeds the SHA-256 that `infra_tooling::source_identity` computes over the
compiled inputs (`Cargo.lock`, `.cargo/`, each package's `Cargo.toml`,
`build.rs` and `src/`, and every repository file they include);
`capabilities --json` reports it as `source`, and assembly copies it into
`runtime-capabilities.json`. `repo build-ui` records the digest of the UI
sources as `source_sha256` in `abi-manifest.json`. The Harness's Update
restarts only on an engine and interface whose recorded sources match the
clone; a change to that scheme is a new `source_identity::SCHEME`.

Repository operations use `./infra-engine repo <operation>`. Installed memory
operations use `memory-bank/bin/infra-engine memory <operation>`. See each
command's `--help` for arguments. The native check catalog is
[`install/check-catalog.json`](../install/check-catalog.json).

After the implementation has been written, validation runs against the actual
workspace, generated payload and disposable target projects. Old Python tests
are historical behavior specifications; they are not evidence that the new
runtime passes. Frozen experiment artifacts keep their original attribution
and grades. Live provider and native Windows coverage require those actual
environments.

The root `memory-kernel-probe` package retains the early byte/framing ABI
contracts as compatibility fixtures. It is not the installed runtime.
