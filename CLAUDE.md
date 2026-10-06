# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Repository Is

This is **not a PHP application**. It is a monorepo of AI-agent accelerators for PHP
projects: ready-made policy/skill/agent/hook bundles for Laravel, Symfony,
framework-neutral PHP Core, and WordPress, plus `Infrastructure-Creator/`, a generator that scans
a *target* PHP project and builds a bespoke accelerator for it. There is no
application code, no Composer install, and no PHP runtime to execute here — the
repository's own tests and tooling are Python (standard library only) and shell.

```
Laravel/, Symfony/, PHP Core/    # framework ready-to-use editions
Cms/wordpress/                   # WordPress ready-to-use edition
Infrastructure-Creator/          # generator that builds a bespoke edition for a target project
install/                         # installer docs + versioned inventories
scripts/                         # repo-level build/install/measurement/CI tools (Python, stdlib only)
harness/                         # optional external LangGraph batch orchestrator (own venv, own deps)
tests/                           # repository-level tests (installation, mirrors, framework semantics)
docs/                            # shared documentation (CI, adoption, tool integration, operations)
```

Each ready-made accelerator (`Laravel/`, `Symfony/`, `PHP Core/`,
`Cms/wordpress/`) and `Infrastructure-Creator/` is a self-contained unit with its own
`AGENTS.md` (enforceable policy), `README.md`, `CHANGELOG.md`, `VERSION`,
`memory-bank/`, `project-brain/` (Infrastructure-Creator excepted), `specs/`,
`tasks/`, and mirrored tool integrations `.claude/`, `.cursor/`, `.agents/` +
`.codex/`. Changes to one edition are not automatically synchronized to the
others — verify a change is meaningful for the target stack before porting it.

## Commands

All commands run from the repository root unless noted. Requirements: Python 3
(3.9+), `git`, `bash`, and `shellcheck` and `php` for two lint steps.

### Before pushing: run what CI runs

```bash
python3 scripts/check.py                               # every CI job; jobs and matrix legs run in parallel, exit 1 on any failure
python3 scripts/check.py --list                        # every command per group, and what would skip on this machine
python3 scripts/check.py --group lint --group mirrors  # only these jobs (repeatable)
python3 scripts/check.py --fail-fast                   # stop everything at the first failure
python3 scripts/check.py --strict                      # a missing tool (shellcheck, php, ...) fails instead of skipping
```

`scripts/check.py` has one group per job in `.github/workflows/*.yml`, each
the job's `run:` steps in order; runner provisioning (apt-get, venv creation,
pip install) is not repeated, and a tool the runner has but this machine lacks
is skipped and named. `tests/test_check.py` fails when a CI command has no
entry in `check.py` or an entry there is no longer in CI, so the two cannot
drift apart. The sections below are the individual steps, for running one
thing at a time; `docs/CI.md` says what each job verifies.

### Run all edition + generator test suites

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

Run a single test file directly, e.g. `(cd "Infrastructure-Creator/tests" && python3 test_scan_contracts.py)`.
The explicit-file loop (not `unittest discover`) is intentional: some
distribution test directories live under a hyphenated parent path and aren't
importable as packages, so `discover` can silently report zero tests.

### Repository-level tests

```bash
python3 -m unittest tests.test_installation
python3 -m unittest tests.test_framework_semantics
python3 -m unittest tests.test_collect_context
python3 -m unittest tests.test_build_mirrors
python3 -m unittest tests.test_check                    # scripts/check.py still matches the workflows
python3 -m unittest tests.test_hook_wiring              # Claude Code/Codex hook wiring reaches its scripts from any working directory
python3 -m unittest tests.test_file_modes               # hook scripts and root launchers are 100755 in the Git index
python3 -m unittest tests.test_bash_validator_corpus    # every bash-validator.sh copy against tests/fixtures/bash-validator-corpus.json
python3 -m unittest tests.test_check_routes             # regression tests for scripts/check_routes.py
```

`python3 scripts/check.py --list` prints the complete list CI runs.

### Mirror parity (Claude Code / Cursor / Codex trees must match canon)

```bash
python3 scripts/build_mirrors.py --check                 # all editions
python3 scripts/build_mirrors.py --check --edition Symfony
python3 scripts/build_mirrors.py --write --edition Laravel --edition "PHP Core"  # after intentional canon edits
```

`--write` also gives each mirror its canonical file's executable bit, on disk
and, for a tracked mirror, in the Git index (`--check` reports a mismatch as
`mode differs from canon`).

### Cross-edition / core parity

```bash
(cd "Laravel"  && python3 memory-bank/scripts/context.py parity && python3 memory-bank/scripts/context.py parity --cross-edition)
(cd "Symfony"  && python3 memory-bank/scripts/context.py parity && python3 memory-bank/scripts/context.py parity --cross-edition)
(cd "PHP Core" && python3 memory-bank/scripts/context.py parity && python3 memory-bank/scripts/context.py parity --cross-edition)
(cd "Cms/wordpress" && python3 memory-bank/scripts/context.py parity && python3 memory-bank/scripts/context.py parity --cross-edition)
```

`--cross-edition` covers the three PHP editions, including their
`.claude/hooks/*.sh` (except the two framework-shaped hooks,
`bash-validator.sh` and `local-context.sh`). It does not see the fourth copy
of the core — the generator asset that `memory-seed` copies verbatim into
every generated project. That one has its own gate:

```bash
python3 scripts/asset_parity.py --check
python3 scripts/asset_parity.py --write   # after an intentional core change
```

A change to `memory-bank/scripts/`, `memory-bank/templates/` or
`project-brain/` is not finished until both checks are green: the first keeps
the editions identical to each other, the second carries the same change into
the projects the generator builds.

`bash-validator.sh` is framework-shaped and excluded from cross-edition
parity, but its generic section (between the `bash-validator generic section`
markers) is byte-identical in the four editions and Infrastructure-Creator.
Edit it in one `.claude/hooks/bash-validator.sh`, paste the block into the
other four, run `python3 scripts/build_mirrors.py --write` to regenerate the
`.cursor`/`.codex` copies, and check with
`python3 -m unittest tests.test_bash_validator_corpus`.

### Installer inventory verification

```bash
python3 scripts/install_accelerator.py --verify-inventories   # must report VERIFIED for every edition
python3 scripts/install_accelerator.py --write-inventories     # only after intentionally changing distribution files; regenerate then re-verify
```

Inventories are generated from `git ls-files --cached`; stage new distribution
files before regenerating or they won't be included.

### Open-source kit selector (Kit 3 - see `install/open-source-kit/README.md`)

```bash
python3 scripts/install_open_source_kit.py --list                              # browse the reviewed catalog
python3 scripts/install_open_source_kit.py --target /path/to/client-project     # interactive checkbox-style selection
python3 scripts/install_open_source_kit.py --select ID1,ID2 --target /path/to/client-project --dry-run
```

Selects from `install/open-source-kit/resources.json` and writes a
`.kit3-manifest.json` into the target; it never downloads or executes
third-party code itself.

The catalog is browsable; the **risk registry**
(`install/open-source-kit/registry/`, one JSON file per candidate) records what
review found. Twelve gates — eight binary (`pass`/`fail`/`unknown`), four scored
0–5 — summarised as `clear`, `open_questions`, or `known_risks`:

```bash
python3 scripts/validate_registry.py            # report every entry and its status
python3 scripts/validate_registry.py --check    # CI gate
python3 scripts/validate_registry.py --id graphify
python3 -m unittest tests.test_registry
```

**The registry describes; it does not forbid.** Nothing refuses a tool — the
selector installs nothing either way, so refusing would only block writing the
choice down, and an install that happened anyway would then be missing from the
audit trail. The status is named before selection and written into
`.kit3-manifest.json`, where it is visible in a diff. A stored `clear` cannot
outrank a failing gate: the status is recomputed, never trusted. Most catalog
entries have no registry file yet and report `NOT REVIEWED` — see
`install/open-source-kit/README.md`.

### Lint

```bash
python3 scripts/check.py --group lint                  # the whole lint job, including the Cursor render grep and the regression tests
git ls-files -z -- '*.sh' 'collect' 'kit3' 'harness-server' 'accelerator-app' | xargs -0 -r -n1 bash -n
git ls-files -z -- '*.sh' 'collect' 'kit3' 'harness-server' 'accelerator-app' | xargs -0 -r shellcheck -S error
(git ls-files -z -- '*.json' | while IFS= read -r -d '' f; do python3 -m json.tool "$f" > /dev/null || { echo "Invalid JSON: $f" >&2; exit 1; }; done)
python3 scripts/check_php_snippets.py --require-php   # lints every complete ```<?php``` block in tracked Markdown
python3 scripts/context_budget.py --check              # startup context-price ceilings per edition (scripts/token_budget.json)
python3 scripts/context_budget.py                       # print current numbers without gating
python3 scripts/check_stabilization.py                 # stabilization rules are well-formed
python3 scripts/policy_lock.py --check                 # policy lock matches the model-facing surface
```

### Other CI-mirroring checks

```bash
python3 scripts/check_links.py                                    # relative markdown links resolve
python3 scripts/check_routes.py                                   # every hook wiring, command/agent/flow route and skill reachability resolves (allowlist: scripts/check_routes_allowlist.json)
bash scripts/check_core_changelog.sh                               # shared-core diff requires a root CHANGELOG.md entry
bash scripts/check_core_changelog.sh some-branch                   # against an explicit base ref
```

### Optional external harness (separate venv and deps; CI job `harness-fleet`)

```bash
python3 -m venv harness/.venv
harness/.venv/bin/python -m pip install -e harness
harness/.venv/bin/python -m unittest discover -s harness/tests -p 'test_*.py'
python3 -m unittest tests.test_harness_fleet
```

`python3 scripts/check.py` runs the last two; without `harness/.venv` the graph
tests skip and it prints the commands that create the venv (`--strict` fails
instead).

### Context collection bundler (for pasting a slice of this repo into another model)

```bash
./collect                                    # list scopes: edition, skills, core, hooks, tooling, docs, harness, diff, custom
./collect skills --edition Laravel --dry-run
./collect diff --base origin/main --stdout
```
In Claude Code this is the `/collect` command. Bundles land in the ignored `.c2p/`. This tool is never installed into a target project and is not a CI gate.

## Architecture

### Shared workflow model: Command → Agent → Skill

Every ready-made edition follows the same pipeline, adapted per stack:

```
User request → Command routes intent → Agent executes exactly one Skill in an isolated context → stop
```

- A **Command** (Claude Code/Cursor only) routes intent to a workflow.
- An **Agent** stays thin: executes one skill, then stops — no automatic chaining.
- A **Skill** holds the actual procedure: checks, examples, decision criteria, output format.
- **Hooks** and `AGENTS.md` policy enforce security, naming, and verification conventions.

Codex has no command layer: skills are invoked by name or auto-selected from `.agents/skills/`.

`tasks/` holds temporary, skill-prefixed `TASK-NNN/` work docs. `specs/` holds
durable living specifications registered in `specs/MANIFEST.md`. Uppercase
`Task/` in the ready-made editions is optional client-owned input material —
distinct from lowercase `tasks/` — and is excluded from the installed payload.

### Three mirrored tool integrations per edition

| Tool | Reads | Notes |
| --- | --- | --- |
| Claude Code | `.claude/` | Canonical source for agents, commands, hooks, skills, settings. |
| Cursor | `.cursor/` | Self-contained mirror; skills/commands/agents/rules/hooks. |
| Codex | `.agents/skills/` + `.codex/` | No command/agent-wrapper layer; skills only. |

`.claude/skills` (and `.agents/skills`, the canonical source) generate the
`.cursor/skills` mirror; `scripts/build_mirrors.py` enforces this per the
transformation rules in `scripts/README.md`. **Never hand-edit a generated
mirror file** — edit the canonical source and regenerate with `--write`, or
`build_mirrors --check` (and CI's `mirrors`/`parity` jobs) will fail.

### Project Brain / Local Context Engine / Memory Bank

Three distinct persistence layers exist inside each edition (and inside a
generated target project):

- **Project Brain** (`project-brain/`) — Git-tracked shared authority for
  active tasks, handoffs, findings, decisions. Governed mode is default.
- **Local Context Engine** — indexes policy/skills/docs/specs/Project Brain
  records into ignored SQLite (`memory-bank/local/context.db`), retrieval via
  FTS5/BM25. Disposable: deleting it deletes no authoritative data.
- **Memory Bank** (`memory-bank/`) — Git-tracked durable knowledge chunks
  (`MEM-YYYYMMDD-xxxxxxxx-{slug}.md`), auto-promoted by default from resolved
  findings/incidents/decisions (tagged `auto-promoted`, unreviewed) unless
  `automatic_promotion` is set to `false` in `project-brain/config/runtime.json`.

Authority order (highest first): enforcement/policy/hooks/CI → canonical
project sources (code, config, migrations, tests, specs) → Project Brain →
Memory Bank → skills/operations → examples → README docs. Retrieved snippets
are discovery aids only, always verified against the cited canonical source.

The public CLI is `python3 memory-bank/scripts/context.py {start,update,retrieve,complete,index,parity,status,validate,compact,export}`,
run from an edition or a consuming project's root. `retrieve` is the current
name; `context` is a compatibility alias. `memory`/`checkpoint` (Codex) or
`/memory`/`/checkpoint` (Claude/Cursor) are the argument-free refresh/capture
entry points — see `AGENTS.md` in any edition for the exact MUST/MUST NOT rules.

### Infrastructure-Creator's own pipeline

`Infrastructure-Creator/` is itself a generator, not an edition to install.
Its workflow (`infra-scan → review → infra-generate`, or `infra-build` to
chain both) is invoked via AI-assistant commands/skills, not terminal
executables:

```
infra-scan <target>     # read-only; 7 parallel PHP scanners + stack-researcher +
                         # clarifying-interview -> tasks/TASK-{N}/infra-scan-project-profile.md
                         # (human review checkpoint — read the full report, not just the summary)
infra-generate <target> # revalidates the approved profile against current target state,
                         # stages policy/hooks/memory + evidence-scoped skill batches,
                         # runs a semantic gate, then agents/commands/flows, then bootstrap-verifier
infra-update <target>   # requires .infra-manifest.json; per-file decision for anything team-edited
```

Never point the generator at itself, and never copy `Infrastructure-Creator/`
into the target project. It supports non-PHP targets by offering to build an
independent sibling generator (`Infrastructure-Creator-[Stack]/`) rather than
silently failing — see `Infrastructure-Creator/README.md#non-php-targets`.

### Installer (`scripts/install_accelerator.py`)

Installs one of the four ready-made editions into a target project from
versioned inventories (`install/inventories/*.json`), refusing unsupported
collisions before writing anything. Always dry-run first
(`--dry-run --merge-existing`), resolve every `COLLISION`, then repeat without
`--dry-run`. Use Infrastructure-Creator instead for projects with substantial
custom architecture — see `install/README.md`.

## Contributing Conventions

- Only change a skill in the edition(s) where it actually applies — Laravel,
  Symfony, WordPress and PHP Core semantics must remain distinct.
- Evaluate a universal policy change in `PHP Core/` first, then adapt it to
  other frameworks' boundaries rather than copying it blindly.
- Within one edition, mirror supported skill/agent/command changes across
  `.claude/`, `.cursor/`, `.agents/`, and `.codex/` (via `build_mirrors.py --write`,
  not by hand).
- Record edition-local changes in that edition's own `CHANGELOG.md`. Changes
  to shared core (memory/context core, Project Brain, hooks, mirror machinery,
  root `scripts/`) go in the root `CHANGELOG.md` instead — CI's `changelog`
  job enforces this on PRs via `scripts/check_core_changelog.sh`.
- Run the affected edition's `DOD.md` verification after a change.
