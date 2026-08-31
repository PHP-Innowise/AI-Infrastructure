# Continuous Integration

The repository is checked by GitHub Actions:
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml). The workflow runs
on every push to `main` and on every pull request. Workflow actions are limited
to `actions/checkout` and `actions/setup-python`, and the Python test/runtime
gates use the standard library. The lint job additionally depends on the
runner-provided `bash` and `shellcheck`; Git is used throughout. The workflow
does not install project packages or use `sudo`.

| Job | What it verifies |
|---|---|
| `tests` | The unit-test suites of every edition (7 suites, run in a matrix). |
| `parity` | Mirror parity and cross-edition core parity for Laravel, Symfony, PHP Core, and WordPress. |
| `mirrors` | Every per-tool mirror matches its canon (`scripts/build_mirrors.py --check`). |
| `installation` | Exact versioned inventories match the repository, and every Laravel/Symfony/PHP Core/WordPress × Claude/Cursor/Codex selected-tool install passes isolated validate/status/index smoke tests without application, `.env`, or application-database access. Also that framework-specific skill semantics survive, and that the optional context-collection tool stays out of the editions and the installer. |
| `lint` | `bash -n` and `shellcheck -S error` on all tracked shell scripts (including root `collect`); `python3 -m json.tool` on tracked JSON; no clock/random invalidator in Cursor working-memory render hooks; every complete PHP snippet in tracked Markdown parses (`scripts/check_php_snippets.py --require-php`); startup context budget within ceilings (`scripts/context_budget.py --check`). |
| `changelog` | Pull requests only: a diff that touches shared-core files (memory/context core, Project Brain, hooks, `scripts/`) must also change the root `CHANGELOG.md` (`scripts/check_core_changelog.sh`). |
| `links` | All relative markdown links in tracked `.md` files resolve (`scripts/check_links.py`). |

## Running the checks locally

The commands below are exactly the commands the workflow runs; CI sets the
working directory per step, which locally is the `cd` in a subshell. Run
everything from the repository root. Requirements: Python 3, `git`, `bash`,
and (for one lint step) `shellcheck`.

### tests

```bash
for suite in \
  "Laravel/memory-bank/tests" "Laravel/project-brain/tests" \
  "Symfony/memory-bank/tests" "Symfony/project-brain/tests" \
  "PHP Core/memory-bank/tests" "PHP Core/project-brain/tests" \
  "Cms/wordpress/memory-bank/tests" "Cms/wordpress/project-brain/tests" \
  "Infrastructure-Creator/tests"; do
  (cd "$suite" && for test_file in test_*.py; do python3 "$test_file"; done)
done
```

The explicit file loop is intentional: some distribution test directories are
not importable Python packages because their parent path contains a hyphen.
Plain `unittest discover` can report a misleading successful zero-test run
there.

### External orchestration harness

The optional LangGraph harness is intentionally outside the edition test
matrix and is not a current release gate. It has separate dependencies and a
separate offline test suite:

```bash
python3 -m venv harness/.venv
harness/.venv/bin/pip install -e "harness[dev]"
harness/.venv/bin/python -m pytest harness/tests
```

Run this suite manually when changing `harness/` or its orchestration contract.
Its exclusion from the standard-library-only CI jobs must not be interpreted as
automatic validation or a pass.

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
```

### mirrors

```bash
python3 scripts/build_mirrors.py --check
```

### installation

```bash
python3 scripts/install_accelerator.py --verify-inventories
python3 -m unittest tests.test_installation
python3 -m unittest tests.test_framework_semantics
python3 -m unittest tests.test_collect_context
```

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
# 'collect' is a shell script without the .sh extension; it is listed
# explicitly so that every tracked shell file stays inside the gate.
git ls-files -z -- '*.sh' 'collect' | xargs -0 -r -n1 bash -n

# Requires shellcheck (preinstalled on GitHub ubuntu-latest runners).
git ls-files -z -- '*.sh' 'collect' | xargs -0 -r shellcheck -S error

git ls-files -z -- '*.json' | while IFS= read -r -d '' f; do
  python3 -m json.tool "$f" > /dev/null || { echo "Invalid JSON: $f" >&2; exit 1; }
done

if git ls-files -z -- '*/.cursor/hooks/working-memory-write.sh' \
                          '*/.cursor/hooks/local-context.sh' \
  | xargs -0 -r grep -nE '\bdate[[:space:]]+[-+]|\$\(date|\$RANDOM|uuidgen'; then
  echo "Per-turn invalidator in the Cursor rule render (above)." >&2
  exit 1
fi

# Requires php (preinstalled on GitHub ubuntu-latest runners). --require-php
# turns a runner that lost it into a failure instead of a silent pass.
python3 scripts/check_php_snippets.py --require-php

python3 scripts/context_budget.py --check
```

The snippet step exists because the repository tracks no `.php` files while
its skills and examples ship hundreds of fenced PHP blocks - the blocks an
agent copies when it writes code. Only blocks beginning with `<?php` are
linted, since they claim to be whole files; fragments are counted and
reported rather than checked, so the step never overstates its coverage.

The budget step measures each edition's startup context price and compares it
against the per-edition ceilings in
[`scripts/token_budget.json`](../scripts/token_budget.json) (observed values
+ ~5% headroom, so only regressions fail). The startup price is `AGENTS.md`
plus every listing the tool shows the model before any work happens: skill
descriptors from the canon `.agents/skills`, the `.claude/commands` listing,
and the `.claude/agents` listing. All of it is paid on every session of the
edition, used or not, which is what makes a rarely-invoked skill or agent
expensive rather than cheap. `frontmatter_bytes` and `body_bytes` are gated
alongside as file facts — the first a superset of the descriptors including
orchestration keys the model never sees, the second the per-invocation cost.

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
