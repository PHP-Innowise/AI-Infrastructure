# Native repository tooling

Production executable logic lives in [the Rust workspace](../runtime-rust/README.md).
The public entry point is `infra-engine`; `scripts/` keeps versioned data and
historical source references rather than an alternate runtime. Installed memory
uses `memory-bank/bin/infra-engine memory OP` (`.exe` on Windows).

During the source-first migration, complete all production implementation before
dependency fetches, builds, formatting, tests, checks or provider calls. The
commands below document the subsequent maintenance phase and do not claim that
validation has run.

## Repository and distribution operations

| Former helper | Native command |
| --- | --- |
| build_mirrors | `infra-engine repo mirrors` |
| core_sync | `infra-engine repo core-sync` |
| asset_parity | `infra-engine repo asset-parity` |
| policy_lock | `infra-engine repo policy-lock` |
| context_budget | `infra-engine repo budget` |
| check_links | `infra-engine repo links` |
| check_routes | `infra-engine repo routes` |
| check_php_snippets | `infra-engine repo php-snippets` |
| check_stabilization | `infra-engine repo stabilization` |
| check_core_changelog | `infra-engine repo changelog --base REF` |
| validate_registry | `infra-engine repo registry --check` |
| install_accelerator | `infra-engine install` |
| accelerator_attach | `infra-engine attach ACTION` |
| collect_context | `infra-engine collect SCOPE` |
| kit3 / install_open_source_kit | `infra-engine kit ACTION` |
| categorize_commits | `infra-engine repo release-categorize` |
| check | `infra-engine check` |

Use `--help` on the actual route for its arguments. There is no generic Python
forwarder or script runner. Repository tooling discovers the clone from the
native executable/current directory; `--source-root` selects it explicitly.

Installation inventories and collision decisions remain versioned contracts.
Always inspect a dry-run before applying an installation or sync:

```bash
infra-engine install --edition "PHP Core" --target /path/to/project --dry-run --merge-existing
infra-engine install --edition "PHP Core" --target /path/to/project --merge-existing
infra-engine install --sync --target /path/to/project --dry-run
infra-engine install --verify-inventories
```

Native install/generation materializes hook and MCP templates with the selected
engine's final absolute path and final target root. Source templates must not be
copied into a live host unchanged. Trust and user-global configuration remain
explicit operations. Installed client projects do not need a Rust toolchain.

## Memory and experiments

`infra-engine eval FAMILY [OP]` provides the native maintainer controllers:

- `memory-eval run|report|install-status|realized|field-check|funnel`
- `memory-outcome-ab seal|run|report|sample|recall-sources|recall-set`
- `memory-native-ab plan|run|inspect`
- `memory-graph-pilot`, `memory-rerank-pilot`, `memory-onnx-pilot`
- `memory-selective-retrieve-pilot`, `memory-consolidation-pilot`
- `memory-current-facts`, `memory-fact-consolidation`, `memory-freshness-plan`
- `memory-rollout-rehearsal`, `memory-lifecycle-fuzz`, `memory-miss-audit`
- `memory-throttle-oracle-v2`, `cost-attribution`

Host benches use `infra-engine bench codex` or
`infra-engine bench claude-code`. Semantic skill routing is
`infra-engine routing-eval`, separate from the static `repo routes` gate.

These commands preserve sealed protocols, exact artifact hashes, input custody,
fixed denominators, explicit retries/refusals, legacy report interpretation,
unknown cost, and versioned RNG behavior. A dry-run or retrieval result does not
establish task benefit. Never regrade an old study under a new protocol without
an explicit new artifact.

Historical knowledge calendars are explicit controlled requests. Ordinary
memory and hooks never import an evaluation clock from ambient environment
variables. Offset calendars advance with elapsed time; proof and process
deadlines stay real. A child path that cannot preserve the controlled calendar
refuses the request.

See [evaluation](../docs/MEMORY-EVAL.md),
[response contracts](../docs/MEMORY-RESPONSE-CONTRACT.md), and
[write-path experiments](../docs/MEMORY-WRITE-PATH.md).

## Skill creator utilities

`infra-engine skill-tools OP` supplies native implementations of
`aggregate-benchmark`, `generate-report`, `improve-description`,
`init-skill`, `package-skill`, `quick-validate`, `run-eval`,
`run-loop`, `generate-review`, `eval-set-review`, `platform-cli`, and
`utils`. Skill packages are real ZIP archives; validation uses YAML.

Symfony's explicit native Codex/Cursor probe behavior remains separate from
the Claude trigger protocol. Supply the original source path with
`--script-path` or select `--platform` explicitly when the edition is
ambiguous. Native probe failures are errors, not negative model observations.
Optimization blinds held-out results from improvement prompts.

Review pages run Rust/Wasm. Supply actual generated `infra_ui.js` and
`infra_ui_bg.wasm` through `--ui-assets` or `INFRA_UI_ASSETS`; missing
assets are an error. Static pages embed the generated ABI glue and Wasm.
Native workbook parsing prepares spreadsheet previews. Vendored attribution
is retained in
[`infra-skill-tools/LICENSE.txt`](../runtime-rust/crates/infra-skill-tools/LICENSE.txt).

## Generator, QA and optional pilots

Infrastructure-Creator helpers use `infra-engine creator HYPHENATED-NAME`,
including deterministic scan/content/plan/flow gates, ownership manifests,
staged publication, native runtime seeding, and native hook policy. They keep
the generator's approved-profile, staging and rollback boundaries.

QA commands use `infra-engine qa COMMAND`: `run`, `validate`,
`manifest`, `reconstruct-workbook`, `build-ledger`,
`apply-decisions` and `migrate-catalog`. Workbook, schema and manifest
processing is native; no QA virtualenv is needed.

Optional explicit library pilots use `infra-engine pilot sqlite-vec probe|fuse`,
`pilot onnx-score`, `pilot longmemeval-project`,
`pilot completion-usage` and `pilot compare-incremental`.
Optional model/extension files are selected explicitly. No interpreter fallback
or automatic model download is part of the installed runtime.

## Validation and canonical resources

After the source implementation gate closes:

```bash
cargo build --manifest-path runtime-rust/Cargo.toml --locked --bin infra-engine
infra-engine check --list
infra-engine check --strict --base origin/main
infra-engine repo mirrors --write
infra-engine repo core-sync --check
infra-engine repo asset-parity --check
infra-engine repo policy-lock --check
infra-engine install --verify-inventories
```

[`install/check-catalog.json`](../install/check-catalog.json) is authoritative
for local/CI check groups. Native `repo write-ci` emits workflows and
`repo check-ci` compares them. Retained historical Python tests are not a
working oracle after their implementation imports have been retired.

Keep `token_budget.json`, `check_links_ignore.txt`,
`check_routes_allowlist.json`, inventories and schema data versioned.
Change payload identities only for reviewed canonical changes. Generated
mirrors are regenerated, never hand-edited.
