# Continuous Integration

The repository is checked by GitHub Actions:
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml). The workflow runs
on every push to `main` and on every pull request. It uses only
`actions/checkout` and `actions/setup-python`, needs no dependencies beyond
the Python standard library, and never uses `sudo`.

| Job | What it verifies |
|---|---|
| `tests` | The unit-test suites of every edition (7 suites, run in a matrix). |
| `parity` | Mirror parity and cross-edition core parity for Laravel, Symfony, and PHP Core. |
| `mirrors` | Every per-tool mirror matches its canon (`scripts/build_mirrors.py --check`). |
| `installation` | Exact versioned inventories match the repository, and every Laravel/Symfony/PHP Core × Claude/Cursor/Codex selected-tool install passes isolated validate/status/index smoke tests without application, `.env`, or application-database access. |
| `lint` | `bash -n` and `shellcheck -S error` on all tracked `.sh`; `python3 -m json.tool` on all tracked `.json`; startup context budget within ceilings (`scripts/context_budget.py --check`). |
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
  "Infrastructure-Creator/tests"; do
  (cd "$suite" && for test_file in test_*.py; do python3 "$test_file"; done)
done
```

The explicit file loop is intentional: some distribution test directories are
not importable Python packages because their parent path contains a hyphen.
Plain `unittest discover` can report a misleading successful zero-test run
there.

### parity

```bash
(cd "Laravel"  && python3 memory-bank/scripts/context.py parity \
               && python3 memory-bank/scripts/context.py parity --cross-edition)
(cd "Symfony"  && python3 memory-bank/scripts/context.py parity \
               && python3 memory-bank/scripts/context.py parity --cross-edition)
(cd "PHP Core" && python3 memory-bank/scripts/context.py parity \
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
```

The synthetic matrix installs each PHP edition once for each selected AI tool
into a path containing spaces. It verifies required Memory Bank and Project
Brain files, exact copy transcripts, collision refusal, the canonical retired
file state, and `validate`/`status`/`index` smoke behavior. Synthetic `.env`,
Composer hooks, and application-database sentinels prove the test does not
execute application code or use application configuration/data. The context
engine uses only its disposable database inside the temporary target.

### lint

```bash
git ls-files -z -- '*.sh' | xargs -0 -r -n1 bash -n

# Requires shellcheck (preinstalled on GitHub ubuntu-latest runners).
git ls-files -z -- '*.sh' | xargs -0 -r shellcheck -S error

git ls-files -z -- '*.json' | while IFS= read -r -d '' f; do
  python3 -m json.tool "$f" > /dev/null || { echo "Invalid JSON: $f" >&2; exit 1; }
done

python3 scripts/context_budget.py --check
```

The budget step measures each edition's startup context price — `AGENTS.md`
plus the frontmatter (and, within it, the `description` trigger text) of
every skill in the canon `.agents/skills` — and compares it against the
per-edition ceilings in [`scripts/token_budget.json`](../scripts/token_budget.json)
(observed values + ~5% headroom, so only regressions fail). Run
`python3 scripts/context_budget.py` without flags for the current numbers;
raise a ceiling only together with the change that justifies the growth.

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

`check_links.py` walks the `.md` files tracked by git, so newly created
markdown files are checked once they are added to the index. An optional
allowlist (`scripts/check_links_ignore.txt`, `fnmatch` patterns, `#`
comments) can exempt known-external targets; the file is absent while the
allowlist is empty.
