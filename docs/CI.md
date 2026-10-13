# Continuous Integration

The repository is checked by GitHub Actions.
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs on every push
to `main` and on every pull request;
[`.github/workflows/windows-creator.yml`](../.github/workflows/windows-creator.yml)
runs only when dispatched by hand, on a self-hosted Windows runner. Most jobs
need nothing beyond the Python standard library, `bash` and Git. The
exceptions: `lint` uses the runner's preinstalled `shellcheck` and `php`;
`harness-fleet` pip-installs the optional graph runtime into a venv;
`qa-tooling` pip-installs the pinned `openpyxl` and `jsonschema` from
[`requirements-qa.txt`](../requirements-qa.txt) into a venv of its own;
`system-orchestration` installs bubblewrap with `sudo apt-get` and lifts the
runner's AppArmor restriction on unprivileged user namespaces; and
`windows-harness` also uses `actions/setup-node`.

**Locally, run [`scripts/check.py`](../scripts/check.py).** It has one group
per job below, each the job's `run:` steps in order, and exits 1 if anything
failed:

```bash
python3 scripts/check.py                  # every job; jobs and matrix legs run in parallel
python3 scripts/check.py --group lint     # one job (repeatable)
python3 scripts/check.py --list           # every command, and what would skip on this machine
python3 scripts/check.py --strict         # a missing tool fails instead of skipping
```

[`tests/test_check.py`](../tests/test_check.py) reads the workflows and fails
when a CI command has no entry in `check.py`, or when an entry there is no
longer in CI, so the two cannot drift apart silently. `check.py` does not
repeat runner provisioning (apt-get, sysctl, venv creation, pip install); a
tool the runner provides but the machine lacks is skipped and named. Windows
jobs are listed and skipped on Linux and macOS.

| Job | What it verifies |
|---|---|
| `tests` | The unit-test files of nine suites, in a matrix: `memory-bank/tests` and `project-brain/tests` of Laravel, Symfony, PHP Core and WordPress, plus `Infrastructure-Creator/tests`. Each suite runs file by file and stops at its first failing file. |
| `qa-tooling` | The QA artifact tooling under `scripts/qa/` on its own venv (`scripts/qa/.venv`, the pinned [`requirements-qa.txt`](../requirements-qa.txt)): synthetic tests for ledger/workbook refusal, strict schemas, composite defect identity, evidence arithmetic/checksums and native-host skips, then the schemas and the `TC-AI` case catalog (`scripts/qa/validate_qa_artifacts.py --skip-run-evidence`). |
| `parity` | Mirror parity and cross-edition core parity for Laravel, Symfony, PHP Core, and WordPress; on the Laravel leg also generator-asset parity (`scripts/asset_parity.py --check`) and its regression tests. |
| `mirrors` | Every per-tool mirror matches its canon (`scripts/build_mirrors.py --check`), plus the mirror executor's regression tests. Also that every hook script and root launcher that must be executable is 100755 in the Git index (`tests/test_file_modes.py`); that every shipped `bash-validator.sh` copy blocks and allows the shared corpus (`tests/fixtures/bash-validator-corpus.json`) and keeps its generic section byte-identical (`tests/test_bash_validator_corpus.py`); and that every hook wiring, command/agent/flow route and skill reachability resolves (`scripts/check_routes.py`, exceptions in `scripts/check_routes_allowlist.json`), with its regression tests. |
| `infrastructure-creator-reliability` | Under Python 3.9, the floor: the complete Infrastructure-Creator suite (`unittest discover`), the canonical reference-catalog contracts, and the Infrastructure-Creator mirrors. |
| `installation` | Exact versioned inventories match the repository, and every Laravel/Symfony/PHP Core/WordPress × Claude/Cursor/Codex selected-tool install passes isolated validate/status/index smoke tests without application, `.env`, or application-database access. Also that framework-specific skill semantics survive; that Claude Code and Codex hook wiring reaches its scripts from any working directory (`tests/test_hook_wiring.py`); that the memory evaluation stand rebuilds a project as of a prompt, overlays an edition without touching project-owned state, scores, reports and parses transcripts (`tests/test_memory_eval.py`, one real refresh on a synthetic project); that the optional context-collection tool stays out of the editions and the installer; that the Kit 3 admission registry is well-formed, with the registry and catalog regression tests; and the native browser harness regression tests, among them the desktop application installer (`tests/test_desktop_app.py`: icon containers, the XDG entry, the macOS bundle, the Windows shortcut and Installed apps entry, all written into temporary folders). |
| `harness-fleet` | Python 3.11: builds `harness/.venv` with the optional graph runtime, runs its offline graph and checkpoint tests, then the browser Fleet integration without model calls. |
| `system-orchestration` | Python 3.9 and 3.x: universal system planning and execution tests behind a working bubblewrap sandbox, and the synthetic service catalog (`scripts/ai_system.py validate`). |
| `windows-harness` | `windows-latest`, Python 3.13: native Windows Harness acceptance and boundary tests; the merged-chat archive's files through NT handles (`tests/test_harness_merge_archive.py`: written, read, refused once changed or given a second name, removed with its folder, orphans recovered, never through a junction); and the desktop application's Start menu shortcut as PowerShell writes it (into a temporary folder; the registry is faked). The whole merge flow (`tests/test_harness_merge.py`) builds an in-process Harness store and runs on Linux only. |
| `lint` | `bash -n` and `shellcheck -S error` on all tracked shell scripts, including the extension-less `collect`, `kit3`, `harness-server` and `accelerator-app` launchers; `python3 -m json.tool` on tracked JSON; no clock/random invalidator in Cursor working-memory render hooks; every complete PHP snippet in tracked Markdown parses (`scripts/check_php_snippets.py --require-php`); startup context budget within ceilings (`scripts/context_budget.py --check`); stabilization rules are well-formed (`scripts/check_stabilization.py`); the policy lock matches the model-facing surface (`scripts/policy_lock.py --check`); the regression tests of the stabilization validator, the policy lock and the routing-eval scoring; and that `scripts/check.py` still matches the workflows (`tests/test_check.py`). |
| `core-changelog` (job `changelog`) | Pull requests only: a diff that touches shared-core files (memory/context core, Project Brain, hooks, `scripts/`) must also change the root `CHANGELOG.md` (`scripts/check_core_changelog.sh`). |
| `links` | All relative markdown links in tracked `.md` files resolve (`scripts/check_links.py`). |
| `creator` (`windows-creator.yml`) | Manual dispatch on a self-hosted Windows runner with Codex's elevated sandbox set up: the real sandbox boundaries for Creator and AI discovery. |

## Running the checks locally

`python3 scripts/check.py` is the supported way to run all of it. The commands
below are the individual steps, for running one thing at a time; CI sets the
working directory per step, which locally is the `cd` in a subshell. Run
everything from the repository root. Requirements: Python 3, `git`, `bash`,
and for two lint steps `shellcheck` and `php`. `python3 scripts/check.py
--list` prints every command, including the full unittest module lists this
page does not repeat.

### tests

```bash
python3 scripts/check.py --group tests
```

The same as plain shell; the outer subshell makes the first failure the exit
status without closing your terminal:

```bash
(
  for suite in \
    "Laravel/memory-bank/tests" "Laravel/project-brain/tests" \
    "Symfony/memory-bank/tests" "Symfony/project-brain/tests" \
    "PHP Core/memory-bank/tests" "PHP Core/project-brain/tests" \
    "Cms/wordpress/memory-bank/tests" "Cms/wordpress/project-brain/tests" \
    "Infrastructure-Creator/tests"; do
    (cd "$suite" && for test_file in test_*.py; do python3 "$test_file" || exit 1; done) || exit 1
  done
)
```

The explicit file loop is intentional: some distribution test directories are
not importable Python packages because their parent path contains a hyphen.
Plain `unittest discover` can report a misleading successful zero-test run
there.

### harness-fleet (external orchestration harness)

The optional LangGraph harness has separate dependencies and its own offline
test suite. CI's `harness-fleet` job builds the venv and runs it:

```bash
python3 -m venv harness/.venv
harness/.venv/bin/python -m pip install -e harness
harness/.venv/bin/python -m unittest discover -s harness/tests -p 'test_*.py'
python3 -m unittest tests.test_harness_fleet
```

`check.py` runs the last two; without `harness/.venv` it skips the graph tests
and prints the commands that create the venv (`--strict` fails instead).

### QA artifact tooling

Use an isolated environment so the runtime editions retain their
standard-library-only dependency contract:

```bash
python3 -m venv scripts/qa/.venv
scripts/qa/.venv/bin/python -m pip install -r requirements-qa.txt
scripts/qa/.venv/bin/python -m unittest discover -s scripts/qa/tests
scripts/qa/.venv/bin/python scripts/qa/validate_qa_artifacts.py --skip-run-evidence
```

`check.py` runs the last two; without `scripts/qa/.venv` it skips them and
prints the commands that create the venv (`--strict` fails instead).

CI validates the strict schemas, the complete `TC-AI-001..018` catalog, and the
staged disposition ledger and reconstructed workbook. It skips only run
evidence, which remains intentionally untracked. The release gate omits the
skip flag and adds `--release`.

### parity

```bash
(cd "Laravel"  && python3 memory-bank/scripts/context.py parity \
               && python3 memory-bank/scripts/context.py parity --cross-edition)
(cd "Symfony"  && python3 memory-bank/scripts/context.py parity \
               && python3 memory-bank/scripts/context.py parity --cross-edition)
(cd "PHP Core" && python3 memory-bank/scripts/context.py parity \
               && python3 memory-bank/scripts/context.py parity --cross-edition)
(cd "Cms/wordpress" && python3 memory-bank/scripts/context.py parity \
                     && python3 memory-bank/scripts/context.py parity --cross-edition)
python3 scripts/asset_parity.py --check
python3 -m unittest tests.test_asset_parity
```

### mirrors

```bash
python3 scripts/build_mirrors.py --check
python3 -m unittest tests.test_build_mirrors

# Hooks run as direct commands: a hook script recorded in the index without
# its executable bit exits 126 in every installed project.
python3 -m unittest tests.test_file_modes

# Every case in tests/fixtures/bash-validator-corpus.json through all 15
# shipped bash-validator.sh copies, with each host's payload; also asserts the
# generic section is byte-identical in every copy.
python3 -m unittest tests.test_bash_validator_corpus

# Mirrors prove the copies match canon; check_routes proves canon points at
# things that exist (exceptions, each with a reason, in
# scripts/check_routes_allowlist.json).
python3 scripts/check_routes.py
python3 -m unittest tests.test_check_routes
```

### infrastructure-creator-reliability

```bash
python3.9 -m unittest discover -s Infrastructure-Creator/tests -p "test_*.py"
python3.9 Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_reference_catalogs.py \
  --references-dir Infrastructure-Creator/.agents/skills/skill-forge/references
python3.9 scripts/build_mirrors.py --check --edition Infrastructure-Creator
```

### installation

```bash
python3 scripts/install_accelerator.py --verify-inventories
python3 -m unittest tests.test_installation
python3 -m unittest tests.test_framework_semantics

# A bare relative hook path exits 127 from a subdirectory, which Claude Code
# and Codex treat as non-blocking: the guard fails open.
python3 -m unittest tests.test_hook_wiring

# The memory evaluation stand (docs/MEMORY-EVAL.md) on a synthetic project.
python3 -m unittest tests.test_memory_eval

python3 -m unittest tests.test_collect_context
python3 scripts/validate_registry.py --check
python3 -m unittest tests.test_registry tests.test_open_source_kit tests.test_kit_fetcher tests.test_kit3 tests.test_kit3_catalog
python3 scripts/check.py --group installation   # adds the native browser harness regression tests
```

The job's `timeout-minutes` is 20. It took 577 s of the former 600 on a hosted
runner, and its harness step alone ranges from about 340 to 470 s between
runs. `check.py` flags a group that ran longer than its job's limit, and
`tests/test_check.py` keeps each group's limit equal to the workflow's.

The synthetic matrix installs each PHP edition once for each selected AI tool
into a path containing spaces. It verifies required Memory Bank and Project
Brain files, exact copy transcripts, collision refusal, the canonical retired
file state, and `validate`/`status`/`index` smoke behavior. Synthetic `.env`,
Composer hooks, and application-database sentinels prove the test does not
execute application code or use application configuration/data. The context
engine uses only its disposable database inside the temporary target.

`tests.test_collect_context` covers the optional context-collection tool
(`scripts/collect_context.py`). Its live checks skip themselves when the
`code2prompt` binary is absent, which is always the case here — CI carries no
Rust toolchain and the tool is never a blocking gate. What runs is the
contract: the argv and exclude pins, and `test_containment`, which fails if
`code2prompt` is ever referenced from an edition or from the installer.

### lint

```bash
# 'collect', 'kit3', 'harness-server' and 'accelerator-app' are shell scripts without the .sh
# extension; they are listed explicitly so that every tracked shell file stays
# inside the gate.
git ls-files -z -- '*.sh' 'collect' 'kit3' 'harness-server' 'accelerator-app' | xargs -0 -r -n1 bash -n

# Requires shellcheck (preinstalled on GitHub ubuntu-latest runners).
git ls-files -z -- '*.sh' 'collect' 'kit3' 'harness-server' 'accelerator-app' | xargs -0 -r shellcheck -S error

(git ls-files -z -- '*.json' | while IFS= read -r -d '' f; do
  python3 -m json.tool "$f" > /dev/null || { echo "Invalid JSON: $f" >&2; exit 1; }
done)

(if git ls-files -z -- '*/.cursor/hooks/working-memory-write.sh' \
                           '*/.cursor/hooks/local-context.sh' \
   | xargs -0 -r grep -nE '\bdate[[:space:]]+[-+]|\$\(date|\$RANDOM|uuidgen'; then
  echo "Per-turn invalidator in the Cursor rule render (above)." >&2
  exit 1
fi)

# Requires php (preinstalled on GitHub ubuntu-latest runners). --require-php
# turns a runner that lost it into a failure instead of a silent pass.
python3 scripts/check_php_snippets.py --require-php

python3 scripts/context_budget.py --check
python3 scripts/check_stabilization.py
python3 -m unittest tests.test_check_stabilization
python3 scripts/policy_lock.py --check
python3 -m unittest tests.test_policy_lock
python3 -m unittest tests.test_routing_eval

# Fails when a CI command has no entry in scripts/check.py, or an entry there
# is no longer in CI.
python3 -m unittest tests.test_check
```

The snippet step exists because the repository tracks no `.php` files while
its skills and examples ship hundreds of fenced PHP blocks - the blocks an
agent copies when it writes code. Only blocks beginning with `<?php` are
linted, since they claim to be whole files; fragments are counted and
reported rather than checked, so the step never overstates its coverage.

The budget step measures each edition's startup context price and compares it
against the per-edition ceilings in
[`scripts/token_budget.json`](../scripts/token_budget.json). Its stated policy
is observed values + ~5% headroom, but ceilings have been raised
inconsistently - some refit to observed + 5%, others by exactly one change's
growth - so headroom runs from a few bytes to just under 5% of the ceiling and
a small addition can fail; `--headroom` lists what each category has left.
The startup price is `AGENTS.md` plus every listing the tool shows the model
before any work happens: skill descriptors from the canon `.agents/skills`,
the `.claude/commands` listing, and the `.claude/agents` listing. All of it
is paid on every session of the edition, used or not, which is what makes a
rarely-invoked skill or agent expensive rather than cheap. `frontmatter_bytes`
and `body_bytes` are gated alongside as file facts — the first a superset of
the descriptors including orchestration keys the model never sees, the second
the per-invocation cost.

Every startup category counts the text a tool renders to the model, not the
file that carries it: a frontmatter field contributes its value with the key,
the colon, any quotes and their escapes removed. Commands and agents usually
declare no `description` at all; Claude Code then derives one from the first
body paragraph and the script does the same. A skill declaring
`disable-model-invocation: true` is left out of `descriptor_bytes`, because
such a skill's description is not put in context at all. `frontmatter_bytes`
is the deliberate exception - it is a file fact, keys included.

The two listings are measured on the `.claude` tree, which is the richest of
the three tool surfaces, so gating it bounds the others instead of tracking
each separately. Cursor carries the same files with a few deliberately
condensed for it. **Codex carries neither**: `.codex/` holds only
`config.toml`, the governance documents, `hooks/` and `hooks.json`, and
`agent-forge` forbids writing an agent there. A Codex session's startup
surface is `AGENTS.md` plus the skill descriptors, so for Codex these two
categories over-state the cost by their whole value.

Run `python3 scripts/context_budget.py` without flags for the current
numbers; raise a ceiling only together with the change that justifies the
growth.

### changelog

```bash
bash scripts/check_core_changelog.sh              # against merge-base with origin/main
bash scripts/check_core_changelog.sh some-branch  # against an explicit base ref
```

The gate diffs the working tree against the merge base with the base ref
(so a local run before committing gives the same answer CI will give for
the pushed result; brand-new files count once they are staged) and fails
only when shared-core files changed without a root `CHANGELOG.md` change.
In CI the base ref is the pull request's base branch and the job checks
out with `fetch-depth: 0` so the merge base is resolvable. When no merge
base can be resolved locally (no `origin` remote and no local `main`),
the script reports why and exits 0 instead of guessing.

### links

```bash
python3 scripts/check_links.py
```

`check_links.py` walks the `.md` files tracked by Git, so newly created
Markdown files are checked once they are added to the index. The committed
allowlist, `scripts/check_links_ignore.txt`, uses `fnmatch` patterns and `#`
comments. It currently exempts only unresolved links into client-owned
`*/Task/*` material; keep that exception narrow rather than using it for
ordinary documentation drift.
