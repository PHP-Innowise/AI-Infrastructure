# Native TC-AI catalog

`cases.json` is the frozen1.0.0 historical catalog. Its commands, observations,
run identities and grades are retained. `cases.native.json` is the executable2.0.0
catalog for the current Rust source. The same18 case/run identities connect the
intent, while native titles and the declared checks describe the new scope.
A native command pass is not proof that every assertion in the former per-edition
Python suites has been ported, and never changes a historical disposition.

The public defaults are:

- `infra-engine qa run`: native catalog, output `docs/qa/evidence-native`.
- `infra-engine qa validate`: native catalog and native run-evidence root.
- `--cases PATH` selects an explicit catalog for run/list/validation.
- `--run-evidence docs/qa/evidence` reads the historical evidence root explicitly.

Listing a retained1.0.0 catalog remains supported. Executing it remains refused;
there is no Python forwarding. Existing explicit `migrate-catalog` remains available
for caller-reviewed mappings. It does not overwrite this frozen historical file.

Native direct Cargo commands use `--locked --offline`; a missing compiler or cache
is a real unavailable/failed prerequisite, never a passing observation. The native
CI check group may build its declared targets according to the repository's shared
check catalog. No command in the default TC-AI catalog calls a model/provider.

## Coverage and exact commands

| Case | Native declaration | Evidence boundary |
| --- | --- | --- |
| TC-AI-001 | Laravel native Memory Bank legacy-storage/public contract suite | Declared native checks |
| TC-AI-002 | Laravel native Project Brain public lifecycle contract suite | Declared native checks |
| TC-AI-003 | Symfony native Memory Bank legacy-storage/public contract suite | Declared native checks |
| TC-AI-004 | Symfony native Project Brain public lifecycle contract suite | Declared native checks |
| TC-AI-005 | PHP Core native Memory Bank legacy-storage/public contract suite | Declared native checks |
| TC-AI-006 | PHP Core native Project Brain public lifecycle contract suite | Declared native checks |
| TC-AI-007 | Native Infrastructure-Creator unit and public CLI contract suites | Declared native checks |
| TC-AI-008 | Native cross-stack integrity and static gates | Declared native checks |
| TC-AI-009 | Native public governed lifecycle and retrieval contracts | Native client observation remains manual |
| TC-AI-010 | Native checkpoint turn-boundary and queue scope contracts | Native client observation remains manual |
| TC-AI-011 | Reviewed and automatic durable-memory promotion modes | Focused native regressions present; native Codex observation pending |
| TC-AI-012 | Explicit completion authority at merge boundary | Focused native regressions present; native Codex observation pending |
| TC-AI-013 | Validator handles relative and absolute target paths | Focused native relative/absolute fixture present |
| TC-AI-014 | Infrastructure-Creator full generate and update lifecycle | Focused native lifecycle fixture present; native Claude Code observation pending |
| TC-AI-015 | Exact-checkout native workspace and repository CI matrix | Declared native checks |
| TC-AI-016 | Curated governed retrieval relevance benchmark | Focused coverage/manual observation remains blocked |
| TC-AI-017 | Telemetry configured-off and sanitized adapter behavior | Focused native adapter regressions present; generic fixture label mapping unverified |
| TC-AI-018 | Reviewed native host smoke across Claude Code, Cursor and Codex (manual) | Focused coverage/manual observation remains blocked |

Cases1–6 pair the actual shared native public/legacy storage contract suite with
read-only parity in the named edition. They do not pretend the former separate
edition test directories still execute. Case7 covers current generator unit and
public CLI contracts. Case8 retains shell syntax and optional shellcheck plus
native mirror/parity/link/budget gates. Its JSON assertion uses the shared lint
JSON-syntax gate; the required catalog pattern prevents an absent gate from being
silently treated as coverage. Case15 runs the declared native workspace/Wasm/static
matrix and retained shell gates. Passing this local matrix does not claim a remote
CI result or native Windows execution on another OS.

Cases11–12 now name the implemented focused public native regressions: automatic
and disabled promotion, reviewed manual promotion, explicit completion authority,
and concurrent completion CAS. Their automated coverage was exercised in the
local native public-contract suite. This closes the former missing-regression
blockers, not the separate requirement for reviewed native Codex observations.
Neither case is declared passed by this catalog update.

Cases13–14 now select the actual native generator fixture that calls both
caller-relative and absolute target checks, then exercises generate, team edit,
classify, explicit keep, watched publication and revalidation. Case14 still
requires independent native Claude Code observations. Case17 names the focused
native configured-off, metadata sanitization and real refresh-adapter regressions.
Removing those missing-regression blockers does not declare TC-AI outcomes.

Case16 remains blocked: approved frozen retrieval thresholds are null and its
native frozen-corpus outcome regression is absent. Neither retrieval nor telemetry
fixture JSON is changed. Missing discovery still emits skipped assertions rather
than running a related suite as substitute evidence.

Cases9–12,14,18 retain the original manual native-host requirements. Declaring
`--native-host` only records availability. Even declaring every host cannot create
native-client observations or make these automated runs pass. Reviewed native
observations must be supplied independently through the existing disposition
process; provider credentials are not used by this runner.

### TC-AI-001

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_legacy_compat` (cwd `.`)
- `{infra} memory parity --root .` (cwd `Laravel`)

### TC-AI-002

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_public_contracts` (cwd `.`)
- `{infra} memory parity --root .` (cwd `Laravel`)

### TC-AI-003

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_legacy_compat` (cwd `.`)
- `{infra} memory parity --root .` (cwd `Symfony`)

### TC-AI-004

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_public_contracts` (cwd `.`)
- `{infra} memory parity --root .` (cwd `Symfony`)

### TC-AI-005

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_legacy_compat` (cwd `.`)
- `{infra} memory parity --root .` (cwd `PHP Core`)

### TC-AI-006

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_public_contracts` (cwd `.`)
- `{infra} memory parity --root .` (cwd `PHP Core`)

### TC-AI-007

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-generator --test native_contracts` (cwd `.`)
- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test native_generator` (cwd `.`)

### TC-AI-008

- `{infra} check --group mirrors --strict --jobs 1` (cwd `.`)
- `{infra} memory parity --root .` (cwd `Laravel`)
- `{infra} memory parity --root . --cross-edition` (cwd `Laravel`)
- `{infra} memory parity --root .` (cwd `Symfony`)
- `{infra} memory parity --root . --cross-edition` (cwd `Symfony`)
- `{infra} memory parity --root .` (cwd `PHP Core`)
- `{infra} memory parity --root . --cross-edition` (cwd `PHP Core`)
- `bash -c git ls-files -z -- '*.sh' | xargs -0 -n1 bash -n` (cwd `.`)
- `bash -c git ls-files -z -- '*.sh' | xargs -0 shellcheck -S error` (cwd `.`)
- `{infra} check --group lint --strict --jobs 1` (cwd `.`)
- `{infra} repo links` (cwd `.`)
- `{infra} repo budget --check` (cwd `.`)

### TC-AI-009

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_public_contracts` (cwd `.`)

Native observation requirement: `codex`.

### TC-AI-010

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_public_contracts next_prompt_and_continuation_stop_drain_pending_native_save_exactly_once -- --exact` (cwd `.`)
- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_queue_scope` (cwd `.`)

Native observation requirement: `codex`.

### TC-AI-011

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_public_contracts` (cwd `.`)

Focused native coverage: targeted automatic promotion; disabled promotion and failure preserving resolution; reviewed manual proposals requiring an independent reviewer before publication and applying once. The fixture exercises the review protocol and does not claim that a real human approved a project change.

Native observation requirement: `codex`.

### TC-AI-012

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_public_contracts` (cwd `.`)

Focused native coverage: a true merge is reported without closing authority or losing pending turns; a never-diverged branch produces no candidate; missing/auto/stale completion revisions preserve state; two independent completion engines yield one authority transition and one episode while retaining pending and unrelated authority.

Native observation requirement: `codex`.

### TC-AI-013

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test native_generator generate_edit_classify_keep_revalidate_lifecycle -- --exact` (cwd `.`)

Focused native coverage: the owning generator fixture actually calls relative and absolute target helpers from the caller parent and an unrelated working directory. Full/merge output, spaces, wrong-CWD relative refusal, no unintended retarget/file creation, and payload/mode preservation are asserted.

### TC-AI-014

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test native_generator generate_edit_classify_keep_revalidate_lifecycle -- --exact` (cwd `.`)

Focused native coverage: generate, team edit, ownership classification, explicit keep, watched publication and revalidation preserve the chosen bytes/modes. A standing decision remains effective only for its admitted proposal and reopens after proposal change. Reviewed native Claude Code observation remains independently required.

Native observation requirement: `claude-code`.

### TC-AI-015

- `{infra} check --group rust --group wasm --group parity --group mirrors --group installation --group lint --group links --strict --jobs 1` (cwd `.`)
- `bash -c git ls-files -z -- '*.sh' | xargs -0 -n1 bash -n` (cwd `.`)
- `bash -c git ls-files -z -- '*.sh' | xargs -0 shellcheck -S error` (cwd `.`)

### TC-AI-016

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-cli --test memory_public_contracts` (cwd `.`)

Coverage blocker: The frozen corpus still declares awaiting-approved-thresholds with null precision/recall/token limits, and no native frozen-corpus outcome regression is present. General retrieval tests are not a replacement.

### TC-AI-017

- `cargo test --manifest-path runtime-rust/Cargo.toml --locked --offline --package infra-core --lib` (cwd `.`)

Focused native coverage: disabled adapter/runtime gates produce no events or
filesystem effects; known metadata counts are preserved; prohibited payloads and
invalid metrics are refused without publication; the actual refresh adapter omits
capsule bodies and treats telemetry failures as best effort.

The frozen generic `sanitized-adapter` fixture has input_tokens=120 and
output_tokens=30, `cost:null` and `expected_status:"measured"`. The test preserves those bytes and
known counts, but does not equate that generic label with emitted event
`availability`. Canonical and native event adapters require all six metric fields
to be known before availability is `measured`; otherwise it is `N/A`. Unknown
cost/usage remains null. A separate explicit synthetic all-six-known input checks
the measured branch. No mapping from the fixture's generic expected_status label
to event availability is established or reported verified.

### TC-AI-018

- No automatic command; reviewed native observations are required.

Coverage blocker: Requires reviewed native Claude Code, Cursor, and Codex observations.

Native observation requirement: `claude-code,cursor,codex`.



Direct QA command execution currently has no per-command deadline, matching the
retained runner. The separate shared check catalog's workspace process gate has a
1200-second deadline. Before the focused extension, local complete
native_generator target observations were240.21seconds and285seconds. Those
measurements do not establish the full workspace duration. The extended owning
fixture subsequently passed in484.23seconds; this is a local observation, not a
new deadline. Cases13/14 select their exact owning
fixture rather than unrelated generator tests. No timeout policy is changed.

## Snapshot, validation and collision contract

The runner reads the selected catalog into one snapshot. New native provenance
records its repository-relative source name and SHA-256 of the exact captured raw
JSON. An explicitly selected external catalog uses the redacted source label
`{external-catalog}` plus its digest. `case.json` is the selected case snapshot.
Validation with the same selected catalog checks both case equality and a supplied
catalog digest. Earlier2.0.0 artifacts without the additive digest field remain
readable if their selected case/provenance otherwise match; the new emitter always
includes it. Retained1.0.0 bundles select the frozen legacy catalog by their own run
version, including the earlier explicit `cases.v1.json` migration layout.

All selected run destinations are checked before any command starts. Existing
files, directories and dangling links cause refusal; existing evidence is never
removed or overwritten. Each run repeats destination admission before publication.
This is bounded preflight plus a publication-time recheck, not a claim of a global
filesystem compare-and-swap transaction. A later execution failure may leave
already published earlier runs from that batch; their recorded outcomes remain
available. The new default output root keeps historical and native packages apart.

The versioned catalog snapshot is also selected when an explicitly invoked
`apply-decisions` consumes run evidence, so native regression requirements are not
replaced by retired Python discovery paths. Neither generating native evidence nor
validating it applies decisions or edits the75 historical observations.

## Regression coverage

Native catalog tests verify identity/host preservation and blocked requirements;
actual disposable-Git evidence emission/provenance; whole-batch collision refusal
before an earlier run appears; custom raw-catalog digest mismatch; retained-v1
bundle selection; manual-host declarations remaining skipped; and legacy execution
refusal with legacy listing preserved. The legacy reader bundle is an explicitly
synthetic protocol fixture, not a claim that old Python was executed.
