# Repository Scripts

This directory contains repository-level build, installation, measurement, and
CI utilities. None of these files is part of a ready-made edition's installed
production payload. Runtime scripts that a consuming project needs live inside
the edition, primarily under `memory-bank/scripts/` and
`project-brain/scripts/`.

All Python utilities use only the Python standard library unless a section
states an external command requirement. Run examples from the repository root.

## Catalog

### `ai_system.py` / `ai_system_lib.py` / `ai_system_execution.py` / `ai_system_providers.py`

**Purpose and status.** Optional stack-neutral system coordination companion;
source-only and not part of installed editions. It accepts explicit service
passports and produces a declared impact map, globally bounded source context,
and a reviewable plan. Explicit execution launches sequential development
workers, native Brain tasks and receipt-backed recovery.

```bash
python3 scripts/ai_system.py validate --system docs/examples/ai-system/system.json
python3 scripts/ai_system.py map --system docs/examples/ai-system/system.json
python3 scripts/ai_system.py plan --system docs/examples/ai-system/system.json \
  --task "Change cancellation behavior" --change-id chg-001 --service orders
```

- Commands: `init`, `validate`, `catalog`, `map`, `locate`, `plan`, `verify`,
  `execute`, `resume`, `run-status`.
- Dependencies: Python 3.9+ standard library on POSIX; optional Git provenance.
  Execution uses the trusted checkout native runtime and a locally configured
  Codex, Claude Code or Cursor Agent CLI, or an explicitly selected provider adapter.
  Native adapters share stdlib Harness command/permission builders.
- Writes: `init` creates a new system workspace; `plan --output` creates only a
  new requested file. `execute`/`resume` write private journals and native tasks;
  `--mode edit` enables scoped worker edits, and `execute --access all` lets every
  worker read, and in edit mode change, all selected service folders (default
  `service`: own service only). Declaration/query commands are read-only.
- Access: external service roots require repeatable caller `--allow-root`;
  passport metadata cannot authorize arbitrary filesystem access.
- Tests: `python3 -m unittest tests.test_ai_system tests.test_ai_system_providers tests.test_ai_system_execution`, also in the dedicated CI job.
- Full contract and sample: [System-level AI Coordination](../docs/AI-SYSTEM-ORCHESTRATION.md).

### `check.py`

**Purpose and status.** The local entry point for what CI runs; source-only
and not installed. One group per job in `.github/workflows/*.yml`, each the
job's `run:` steps in order.

```bash
python3 scripts/check.py                    # every group; exit 1 if anything failed
python3 scripts/check.py --group lint --group links
python3 scripts/check.py --list             # groups, commands, and what would skip here
python3 scripts/check.py --fail-fast --jobs 4
python3 scripts/check.py --strict           # a missing tool fails instead of skipping
```

- **Options:** repeatable `--group`; `--list`; `--fail-fast`; `--jobs N`
  (jobs or matrix legs at once; default half the CPUs, at least 2);
  `--strict`; `--base REF` for the changelog group.
- **Execution:** jobs run in parallel, and so do the legs of a matrix job,
  which CI also runs as separate jobs; the steps of one job or leg run in
  order (`--jobs 1` runs everything one after another). A step that needs a
  shell runs as GitHub runs it (`bash --noprofile --norc -eo pipefail -c`),
  so a loop stops at its first failure. `python3`/`python` inside every
  command — and inside the processes tests spawn — is the interpreter running
  `check.py`; the job CI pins to 3.9 runs under `python3.9`. The `tests` job's
  file loop becomes one command per file, so a failure names its file.
- **Outputs:** one PASS/FAIL/SKIP line per command with its duration, the last
  60 lines of each failing command's output with the path of its full log, and
  a per-group summary table with wall times. A job or leg that took longer
  than CI's `timeout-minutes` is flagged, not failed. Exit 0 when nothing
  failed, 1 on a failure, 2 on a usage error, 130 when interrupted. Ctrl-C,
  SIGTERM and SIGHUP (`kill`, `timeout`, a closed terminal) all stop the run
  the same way: each running command's process group is terminated and the
  temporary directory removed.
- **Skips:** a tool the CI runner provides but this machine lacks (shellcheck,
  php, pwsh, bwrap, `python3.9`, `harness/.venv`) skips with the reason;
  `--strict` fails it. So does a changelog base with no merge base here (no
  `origin/main` and no local `main`); a `--base` that does not resolve fails
  either way. An entry whose files are not in the checkout skips as "not
  present" when its workflow has no such step; when the workflow runs it, the
  skip says CI fails there and `--strict` fails it. Windows-only jobs are
  listed and skip elsewhere; the runner itself supports Linux and macOS.
  Runner provisioning (apt-get, sysctl, venv creation, pip install) is not
  repeated.
- **Writes:** nothing in the repository. Logs and interpreter shims go to a
  temporary directory that is removed unless something failed.
- **CI relationship:** `tests/test_check.py` reads the workflows and fails when
  a CI command has no entry here, an entry here is no longer in CI, a
  guarded entry hides a step CI runs, or a job's setup-python versions (leg by
  leg) differ from the interpreters pinned here. Gates CI does not run yet are
  marked `local_only` with the reason.

### `build_mirrors.py`

**Purpose and status.** Maintainer build tool; source-only and not installed.
It verifies or regenerates the Claude Code, Cursor, and Codex mirrors from the
canonical files declared by each edition's `MIRROR_RULES`.

```bash
python3 scripts/build_mirrors.py --check
python3 scripts/build_mirrors.py --check --edition Symfony
python3 scripts/build_mirrors.py --write
python3 scripts/build_mirrors.py --write --edition Laravel \
  --edition "PHP Core"
```

- **Principal options:** exactly one of `--check` or `--write`;
  repeatable `--edition` for `Laravel`, `Symfony`, `PHP Core`, `WordPress`, or
  `Infrastructure-Creator`. The default is all editions.
- **Inputs:** canonical edition files and the edition-local mirror rules.
  Laravel, Symfony, PHP Core, and WordPress load rules from
  `memory-bank/scripts/context_retrieval.py`; Infrastructure-Creator loads
  `mirror_rules.py`.
- **Outputs:** `--check` prints drift, missing files, unaccounted mirror
  files, and mirrors whose executable bit differs from canon
  (`mode differs from canon`), and exits nonzero on findings. `--write` prints
  each changed path; a mode-only repair is printed as `(mode +x)`/`(mode -x)`.
- **Dependencies:** Python 3 standard library, Git, `file_modes.py`, and a
  complete repository checkout.
- **Writes:** `--check` never writes. `--write` creates/updates generated
  mirror files, gives each the executable bit of its canonical file (also
  when the bytes already match), and regenerates each selected edition's
  `.gitattributes`. For a tracked mirror the bit is also recorded in the Git
  index - the mode only, never the content - because a checkout without
  filesystem modes (Windows, `core.fileMode=false`) has nowhere else to keep
  it. An intent-to-add entry (`git add -N`) counts as untracked and is left
  as it is: its index entry holds the empty blob, not the file.
- **CI relationship:** the `mirrors` job runs `--check`. Edition parity jobs
  also execute the same mirror contract through
  `memory-bank/scripts/context.py parity`.

#### Canon and transformations

| File class | Canonical source | Generated mirrors | Transformation |
| --- | --- | --- | --- |
| Skills | `.agents/skills` | `.claude/skills`, `.cursor/skills` | Identity, subject to documented exceptions |
| `SKILL FLOW.md` | `.claude/skills` | `.cursor/skills` | Identity; `.agents` is a tool-specific adaptation |
| Hooks | `.claude/hooks` | `.cursor/hooks`, `.codex/hooks` | Ordered tool/event/path substitutions |
| Commands | `.claude/commands` | `.cursor/commands` | Cursor frontmatter and path adaptation |
| Agents | `.claude/agents` | `.cursor/agents` | Drops Claude-only frontmatter; preserves body |
| Governance docs | `.claude` | `.cursor`, `.codex` | Tool-tree self-reference rewrites |

Every transformation also carries the canonical file's executable bit: a
mirror is executable exactly when its canon is. The canonical bit is read from
the Git index, and from the filesystem only for a path the index does not
know, so a Windows checkout reads the same answer as Linux. Cursor and Codex
run the hook mirrors as direct commands; a mirror that lost the bit exits 126
in every installed project while its bytes still match.

Documented exceptions live in the rules data with justifications; they are not
an invitation to hand-edit generated files. Notable exceptions include
tool-owned `skill-creator` generations, tool-specific hook READMEs,
Cursor-specific working-memory behavior and condensed wrappers, and
Infrastructure-Creator documentation.

The generated `.gitattributes` lists exactly the paths emitted by the rules as
`-diff linguist-generated=true`. It is derived rather than globbed because the
same tool directories also contain canonical settings, rules, config, and
other files whose diffs must remain visible. Do not hand-edit it.

When a consuming project already has a root `.gitattributes`,
`install_accelerator.py --merge-existing` preserves project entries and adds
missing accelerator directives in an installer-managed block. It does not
blindly replace or merely refuse that supported root-file collision.

### `install_accelerator.py`

**Purpose and status.** Distribution entry point; repository-level and not
copied into the installed payload. It verifies versioned inventories, performs
collision preflight, and installs a selected ready-made edition.

```bash
python3 scripts/install_accelerator.py --verify-inventories
python3 scripts/install_accelerator.py \
  --edition Symfony \
  --target "/path/to/project" \
  --tool cursor \
  --merge-existing \
  --dry-run
```

- **Principal modes:** `--verify-inventories`, `--write-inventories`,
  `--edition {Laravel,Symfony,PHP Core,WordPress}` with `--target`, or
  `--sync` with `--target` (see below).
- **Selection options:** repeatable `--tool {claude,cursor,codex}`; omission
  selects all tools. `--source-root` points at an alternate source checkout.
- **Sync options:** `--rewire-codex` also updates an untouched
  `.codex/hooks.json` (approve it again in Codex `/hooks`);
  `--update-tracked` also writes files the project's Git tracks, for a
  person to review and commit. `--dry-run` reports without writing.
- **Collision options:** `--dry-run`; conservative `--merge-existing`; and
  destructive `--overwrite`, which is for explicit maintainer-controlled use,
  not normal adoption.
- **Inputs:** edition files, `VERSION`, inventories under
  `install/inventories/`, selected tool set, and target filesystem state.
  Inventories are the versioned production boundary: source exclusions and
  production-specific selection are resolved there without requiring
  documentation to depend on a particular future schema field name.
- **Outputs:** tab-separated `VERIFIED`, `COLLISION`, `WOULD_*`, `COPY`,
  `MERGE`, `COPY_AS`, `UNCHANGED`, `FIX_MODE`, and `COMPLETE` records;
  meaningful nonzero exit status on inventory errors or refused collisions. A
  `FIX_MODE\t<component>\t<path>\t<destination>` record (`WOULD_FIX_MODE`
  with `--dry-run`) names a file already identical in the target that should
  be executable and was not - typically a hook from an install made before
  executable bits were enforced. Its content is left untouched and only the
  bit is added, under every collision mode, since that destroys nothing. A
  refused run changes nothing, modes included: an install from an earlier
  release collides first on the files that release changed, and its modes
  are repaired by the run that resolves those collisions (or by
  `chmod +x <destination>`).
- **Dependencies:** Python 3 standard library and Git. Inventory discovery for
  verification/writing is `git ls-files --cached`, so the payload is exactly
  what the index holds. Untracked working-tree content - a client application
  under `Task/`, build caches, local databases, `.env` files - is invisible to
  both modes: it cannot be written into an inventory and cannot fail
  verification on a dirty tree. A new distribution file therefore has to be
  staged (`git add`; no commit needed) before regeneration records it. Without
  a Git checkout both modes fail with an explicit error rather than falling
  back to a filesystem scan, because outside the index there is no way to tell
  distribution files from client data.
- **Writes:** verification and dry-run do not write. Installation creates
  selected files in the target. `--write-inventories` regenerates repository
  inventory JSON, or writes it to `--inventory-out DIR` instead, which lets a
  caller regenerate and compare without touching the checkout under test. The
  target path rejects symlink components.
- **CI relationship:** the `installation` job verifies inventories and runs
  installation and framework-semantics tests.

Installed executables get mode `0755` regardless of the source working tree:
every `*.sh` inside a `hooks/` directory, plus any file recorded `100755` in
the source checkout's Git index (or, without a Git checkout, carrying the
executable bit on disk). Every other file keeps normal copy semantics. The
index is read, not the disk, because a source checkout made on Windows or with
`core.fileMode=false` has no executable bit on disk and would otherwise
install hooks that exit 126. The installer reads the index itself rather than
through `file_modes.py`, because the Harness runs a standalone copy of it.

Collision preflight is completed before normal copies begin. With
`--merge-existing`, identical files remain `UNCHANGED` (or `FIX_MODE`, when
only a required executable bit is missing); `.gitignore` and
`.gitattributes` receive only missing directives in a marked block;
`AGENTS.md` preserves project policy before one replaceable accelerator block;
and an existing root `README.md` is preserved while accelerator documentation
is written to `ACCELERATOR.md`. Every other differing selected path remains a
collision. A malformed managed block, symlink, non-file obstruction, or
conflicting `ACCELERATOR.md` is also refused; `--sync` keeps an untouched
`ACCELERATOR.md` at the release, so adding a tool to a synced project later
finds it unchanged.

A file the install replaces - a merge into a project file, an `--overwrite`
collision - and every MCP configuration it writes are written whole beside
the file, flushed to disk and renamed over it, keeping the replaced file's
mode: a full disk or an interrupted run leaves the old file as it was, never
a truncated one a later merge cannot parse. An install that fails part way
puts back, newest first, the files it had already replaced (`RESTORED` on
stdout; `NOT_RESTORED` with the reason on stderr for a file someone changed
after the install wrote it, which is left as it is) and exits nonzero. Files
it created stay; they hold only the release, and the next run finds them
identical.

The install transcript is an action log, not a backup. Its
`COMPLETE files=` value counts selected inventory entries, including
`UNCHANGED` entries. Save the transcript with pipeline failure propagation
and retain a pre-install VCS/backup recovery point.

`--sync --target DIR` brings an installed project's untouched accelerator
files and its runtime up to the clone and prints a JSON report; the Harness
runs it when it opens a project. It never applies part of a release over
what that release's files run: while a runtime file (`memory-bank/scripts`,
`project-brain/scripts`, `project-brain/schemas`) stays at the project's
version - the project's Git tracks it, or it cannot be written safely - the
release's other files are held back too, because its skills, commands, hooks
and memory server call that runtime; a tool's new hooks wait for its wiring
(`.claude/settings.json`, `.cursor/hooks.json`, `.codex/hooks.json`), and the
wiring waits for a new hook that cannot be written. Each held-back file is
listed under `kept` with a reason starting `held back:`, and `partial` says
the project is not at the release (it is `null` otherwise).

The edition README that `--merge-existing` installed as `ACCELERATOR.md`
beside a project's own `README.md` follows the release by the same rules:
the sync records it in `memory-bank/local/accelerator-install.json`, updates
it while it holds a README the edition shipped or what the last sync wrote,
and keeps one the team edited, listed under `kept`. Syncs before this one
did not record it; such a copy is recognised from the clone's Git history of
the edition README, and without that history (a source from an archive) an
older copy stays, reported as edited. The sync writes no `README.md` and
adds no `ACCELERATOR.md`. After a sync, adding a tool with
`--edition E --tool T --target DIR --merge-existing` finds `ACCELERATOR.md`
unchanged; before, a release that changed the README made that install
refuse it (`accelerator-readme-exists`) until the file was moved aside.

An install writes the same record (`memory-bank/local/accelerator-install.json`,
ignored local state) for every file it leaves holding the release byte for
byte - copied, already identical, or given only its executable bit - and
keeps the entries of a record the project already has. A file it merged into
the project's own (`AGENTS.md`, `.gitignore`, an MCP configuration) is not
named: it is the project's, and a sync never replaces it whole. With the
record from the start, the first sync tells untouched files from edited ones
without the clone's history, and the memory runtime leaves the untracked
install out of a task's changed files (`changed_paths`) from the first turn.

A project that commits its accelerator takes a release through a commit:
`--sync --update-tracked --target DIR` (`--dry-run` first) writes the files
the project's Git tracks too, by the same rules - a file still holding a
released version, the runtime (a local edit backed up under
`memory-bank/local`), the managed blocks of `AGENTS.md` and
`.claude/CLAUDE.md`, and the accelerator's marked block in `.gitignore` and
`.gitattributes` - and leaves every edited file, the project's own
`README.md` and the rest of `.gitignore` and `.gitattributes`, its seeded
state and the tools it did not install as they are. Without `--update-tracked`
the marked block of an untracked `.gitignore` or `.gitattributes` follows the
release too; a file without the block stays the project's. Add `--rewire-codex` when the project uses Codex and
approve the hooks again in Codex `/hooks`. Then review the report and
`git diff` and commit. Do not reinstall with `--overwrite` for this: it
replaces the project's own `AGENTS.md`, `README.md` and `.gitignore` and its
edited skills, and without the original `--tool` selection it adds every
tool. Nothing unattended (the Harness included) passes `--update-tracked`.

The sync never reads or writes through a symbolic link inside the project.
Where Python offers descriptor-relative calls
(`os.supports_dir_fd`: Linux, macOS), every read, write, backup and the sync's
record walk the path by folder descriptors with `O_NOFOLLOW`, so a folder
another process swaps for a link while the sync runs is not followed either;
a folder swapped during a write leaves the file alone, and the path is
reported under `kept`. Native Windows has no such calls in Python; there the
walk takes the same steps by NT handles (`NtCreateFile` relative to the
folder before, no reparse point followed, the temporary file renamed within
the last folder's handle), so a junction or link swapped in is not followed
there either. Where neither descriptors nor handles are available nothing is
written - each file that would have been is reported under `kept` - because
a write by path after a check would follow a folder swapped in between. The
same calls carry the files an install replaces or merges.

### `context_budget.py`

**Purpose and status.** Maintainer measurement and regression gate; source-only
and not installed. It measures edition startup surfaces and skill bodies.

```bash
python3 scripts/context_budget.py
python3 scripts/context_budget.py --check
python3 scripts/context_budget.py --headroom
```

- **Principal option:** `--check` compares all measured categories with
  `token_budget.json`.
- **Inputs:** each edition's `AGENTS.md`, canonical
  `.agents/skills/*/SKILL.md`, Claude command/agent listings, and, in check
  mode, `scripts/token_budget.json`.
- **Outputs:** a human-readable byte/token-estimate report, or per-category
  pass/fail lines. `--check` exits nonzero for budget excess, malformed/missing
  ceilings, or edition drift.
- **`--headroom`:** prints the bytes remaining under each ceiling,
  tightest first, and always exits 0. It is what a rule author needs
  before adding a paragraph to a gated file. No percentage warning is
  printed: `token_budget.json` states a policy of observed value plus about
  five per cent, but ceilings were raised inconsistently (some refit to
  observed + 5 %, others by exactly one change's growth), so headroom runs
  from a few bytes to just under 5 % of the ceiling. A 5 % warning would fire
  on every category at once; the byte count is the number that matters.
- **Dependencies:** Python 3 standard library.
- **Writes:** none.
- **CI relationship:** the `lint` job runs `--check`.

It measures `agents_md_bytes`, rendered skill descriptor bytes, Claude command
and agent listing bytes, full skill-frontmatter bytes, skill-body bytes, and
skill count. Command/agent measurements use Claude's richest surface, so they
slightly differ from Cursor and intentionally overstate Codex, which has no
command or agent tree. Token figures are calibrated cl100k estimates by content
class, not Anthropic token counts.

### `cost_attribution.py`

**Purpose and status.** Optional developer-local analysis; source-only, never
installed, and not called by hooks, skills, or CI. It attributes billed Claude
Code transcript usage in base-token equivalents (BTE).

```bash
python3 scripts/cost_attribution.py
python3 scripts/cost_attribution.py --project AI-Infrastructure
python3 scripts/cost_attribution.py --by skill --by agent --top 20
python3 scripts/cost_attribution.py --root /path/to/projects --json
```

- **Principal options:** transcript `--root` (default
  `~/.claude/projects`), substring `--project` filter, repeatable
  `--by {agent,mcp,plugin,skill,tool}`, `--top`, and `--json`.
- **Inputs:** local Claude Code JSONL transcripts. Duplicate content-block
  records are collapsed by `message.id` so repeated `message.usage` is counted
  once per billed call.
- **Outputs:** weighted token tiers and BTE grouped by main/subagent stratum,
  project, and selected attribution dimensions; or structured JSON.
- **Dependencies:** Python 3 standard library; no network or exporter.
- **Writes:** none.
- **CI relationship:** none.

The billing weights are constants in the script and must be reviewed if
published pricing ratios change. Records that are unreadable or lack usable
usage data are skipped; an absent transcript root or zero billed calls returns
nonzero.

### `memory_eval.py`

**Purpose and status.** Optional maintainer measurement; source-only, never
installed, and not run by CI (its tests are, in the `installation` job). It
measures the prompt-time memory capsule on each project as it was when the
prompt was written, so knowledge written after a prompt cannot be credited to
it. See [Memory Evaluation Stand](../docs/MEMORY-EVAL.md).

```bash
python3 scripts/memory_eval.py run --set SET --judgments JUDGMENTS [--passages PASSAGES] \
  --projects-root DIR --out RESULT [--edition auto|NAME] [--as-of prompt|now]
python3 scripts/memory_eval.py report RESULT [--compare OTHER] [--show-unjudged]
python3 scripts/memory_eval.py realized --project DIR [--since YYYY-MM-DD] [--json]
```

- **Inputs:** a prompt set, judgments and optional answer passages (client
  data, kept outside the repository); the projects' Git history and working
  trees, read-only; for `realized`, Claude Code transcripts and Codex rollouts.
- **Outputs:** a result JSON of prompt ids, document paths and counts - never
  prompt text, capsule text or document bodies; reports on stdout.
- **Dependencies:** Python 3.9+ standard library and `git`; no network. It
  imports `install_accelerator.py` (inventories, install rules) and
  `accelerator_attach.py` (edition detection), and runs the edition's own
  `memory-bank/scripts/context.py`.
- **Writes:** only its cache (default under the system temporary directory;
  refused inside the clone) and `--out`.
- **CI relationship:** `tests/test_memory_eval.py` in the `installation` job.

`run --diagnose-misses` adds judged-path stage counts and per-path passage
presence to result JSON and `report`. It uses the actual refresh manifest and
prompt-time clock, with recorded history searched separately from the main
FTS query. Skills and project knowledge are reported apart. No flag means no
new diagnostic fields.

`memory_onnx.py`, `memory_onnx_pilot.py` and `stateful_pilot.py` provide an **off-by-default,
maintainer-only** excerpt experiment, selected by `run --onnx-excerpts ASSETS
--onnx-python PYTHON`. The parent stays stdlib-only; the explicit isolated
worker needs Python 3.12+ with the pinned `requirements-memory-onnx.txt` (NumPy's
requirement; ONNX Runtime itself needs 3.11+). Assets are local
and hash-locked, with no automatic installation or download. The separate
versioned Jina profile requires the retained Python 3.14 and exact package/asset
pins; schema-less MiniLM assets remain supported. Worker budgets are capped at
five seconds, and a tighter invocation deadline prevails. Failure retains
lexical excerpts; the pilot cannot change document eligibility/order or the
paths retained by the ordinary packer. See the [optional experiment](../docs/MEMORY-EVAL.md#optional-onnx-excerpt-experiment)
for asset format, limits and evaluation rules. These files are not installed
into target projects.

### `memory_fact_consolidation.py`

**Purpose and status.** Optional, source-only deterministic fact resolver. It
groups candidate facts by key and chooses the maximum explicit
`serial`/`version`/`revision`, breaking an equal version on `updated_at`; a true
tie is reported as ambiguous rather than arbitrated by path or ID. It implements
the measured "extract candidates, then take a Python maximum" recipe at the
consolidation boundary and is deliberately not part of prompt-time retrieval.

```bash
python3 scripts/memory_fact_consolidation.py --input facts.json --json
```

- **Inputs:** a JSON list, or `{ "facts": [...] }`, with `id`, `fact`, an
  optional version field and an optional `updated_at`.
- **Outputs:** winner, superseded IDs, candidates and an ambiguity flag per
  fact group.
- **Dependencies:** Python 3.9+ standard library.
- **Writes:** none.
- **CI relationship:** `tests/test_memory_fact_consolidation.py` and
  `tests/fixtures/fact-consolidation.json`; see
  [the write-path doc](../docs/MEMORY-WRITE-PATH.md).

### `memory_outcome_ab.py`

**Purpose and status.** A maintainer-only measurement of task outcomes. It
answers whether the automatic capsule changes what an agent does, not whether
the capsule's documents are relevant.

For a real task — the first prompt of a recorded session that went on to change
files — it does the following:
- rebuilds the project as it was at that prompt, with the `memory_eval.py`
  reconstruction and recent-files history seeded from the earlier sessions;
- runs a headless Codex session on the prompt with no hooks and with the
  memory prompt hook alone;
- counts the tool calls before the agent first touches a file the original
  session changed.

Runs stop at that touch, so a pair costs the start of two sessions. It is never
installed and never run by CI; it spends model calls through the
maintainer's signed-in Codex CLI.

A *recall* set asks whether an agent uses what an earlier session saved:
- `recall-sources` lists first turns that read project files and answered
  with a conclusion naming one;
- the saved learning and a follow-up question are written outside the tool
  for each source, and `recall-set` checks them against it;
- `run` records the save on the corpus branch's task through `record-result`
  before any arm, and counts the calls until the agent touches a file the save
  cites or answers naming one;
- `--regime next` asks a minute after the source turn; `--regime later` asks
  at the first later session start whose recent files no longer show the files
  the answer named.

```bash
python3 scripts/memory_outcome_ab.py sample --projects next,scb --before 2026-10-07 \
  --n 30 --seed 1 [--exclude-set TUNING_SET] --out OUTSIDE/set.json
python3 scripts/memory_outcome_ab.py run --set OUTSIDE/set.json --out OUTSIDE/result.json \
  [--arms off,capsule,navigation,recall] [--repeat 2] [--cap 40] [--timeout 900] [--ids ID,...] \
  [--events OUTSIDE/events]
python3 scripts/memory_outcome_ab.py report OUTSIDE/result.json [--compare off,capsule] \
  [--metric gold|recall] [--json]
python3 scripts/memory_outcome_ab.py recall-sources --projects next,scb --before 2026-10-07 \
  [--exclude-set SET] --out OUTSIDE/recall-sources.json
python3 scripts/memory_outcome_ab.py recall-set --sources OUTSIDE/recall-sources.json \
  --scenarios OUTSIDE/authored.json [--regime next|later] [--exclude-id ID,...] \
  --n 30 --seed 1 --out OUTSIDE/recall-set.json
```

- **Inputs:**
  - Claude Code transcripts under `~/.claude/projects`;
  - the projects under `--projects-root`;
  - for `run`, a signed-in `codex` on PATH.
- **Outputs:**
  - `sample`: a set with prompts and gold paths;
  - `recall-sources`: first turns with their prompts and answers;
  - `recall-set`: the checked scenarios, each with its save, and the refusals;
  - `run`: per-task counts, stop reasons, whether the hook delivered and, for
    a recall set, whether it delivered the save;
  - `report`: paired differences, a bootstrap interval, a sign test and the
    verdict rule documented in [MEMORY-EVAL.md](../docs/MEMORY-EVAL.md).
- **Dependencies:** Python 3.9+ standard library, `memory_eval.py`,
  `accelerator_attach.py` and the edition runtime's transcript parsers.
- **Writes:**
  - the set and the result, owner-only, refused inside the clone;
  - disposable corpora under the cache.

  Sessions are `--ephemeral`, so `~/.codex/sessions` keeps nothing.
- **CI relationship:** `tests/test_memory_outcome_ab.py` covers path and event
  parsing, stopping, hook flags, the statistics and sampling, recall sources,
  scenario checks and a save recorded through the runtime, without a model.

### `memory_rollout_rehearsal.py`

**Purpose and status.** A maintainer-only rehearsal of `install_accelerator.py
--sync` on a copy of an installed project, before the project itself is
synced.

For each `--target` it does the following:
- copies the project with its Git history, leaving out `node_modules`,
  `vendor` and `var` at any depth and `.worktrees` and `.claude/worktrees` at
  the root (`--exclude` adds more);
- syncs the copy from this clone;
- for each host (`claude`, `codex`, and `cursor` when asked) and capsule
  profile, on a copy of its own, runs the installed hooks as the host would
  on one turn:
  - the session-start and prompt hooks;
  - the Stop hook and the save its request names, carrying one learning that
    cites a tracked project file;
  - the closing Stop;
  - a new session's prompt, whose capsule must recall the learning.

With `--worktree`, Claude Code is also rehearsed, under each profile, in a
Git worktree of a copy (`.claude/worktrees/rehearsal`, on a branch of its
own), which has none of an installation Git does not track: the hooks run
through the dispatcher the copy carries, as your Claude Code configuration
runs them, and the save takes the variables its `environment` entry gives a
session's commands. Such a run also has to save under the worktree's branch
and leave the worktree clean.

No model runs. HOME and TMPDIR point into the work directory, the copies are
removed unless `--keep`, and the project is never written. A symbolic link
that leads out of the project is left out of the copy, because a hook would
write through it. A Git worktree or submodule is refused, because its Git
directory belongs to another checkout. The `config` profile
is the one the project's own `runtime.json` selects; `--sync` does not change
that file. Any other profile is forced through `CONTEXT_CAPSULE_PROFILE`.

```bash
python3 scripts/memory_rollout_rehearsal.py --target DIR [--target DIR ...] \
  [--hosts claude,codex,cursor] [--profiles config,recall] [--work DIR] [--exclude NAME] \
  [--worktree] [--timeout 60] [--keep] [--json]
```

- **Outputs:**
  - what the sync changed and kept;
  - the profile `runtime.json` selects;
  - with Claude Code among the hosts, whether your own Claude Code ever
    accepted the project's trust dialog (`~/.claude.json`, or
    `$CLAUDE_CONFIG_DIR`; that flag and nothing else is read). Without it,
    `claude -p` there ignores the project's allow rules and refuses the save,
    which the copy's own HOME would hide. It does not change the exit status;
  - for each run:
    - every hook's exit and time;
    - whether the save was requested, made or refused (with its code);
    - the native journal's lines;
    - whether the next capsule recalled the save;
    - in a worktree, whether the save went to its branch and the worktree
      stayed clean.

  Exit 0 when every run saved and recalled with every hook exiting 0, 1
  otherwise, 2 for a target it will not copy (this clone, a folder with no
  installed runtime, a Git worktree or submodule).
- **Writes:** copies under `--work`; the default is `/dev/shm` when writable,
  otherwise the system temp directory.
- **CI relationship:** `tests/test_memory_rollout_rehearsal.py` in the
  `installation` job rehearses a fresh PHP Core install. Each of the three
  hosts under each profile must save and recall, the project must stay byte
  for byte as it was, and the copy must leave out dependency trees. Cursor's
  capsule is the rule file its prompt hook renders, and its Stop request
  arrives as `followup_message`.

### `memory_lifecycle_fuzz.py`

**Purpose and status.** A maintainer tool, and a CI test, that drives random
session and Git lifecycles through a fresh installation's memory hooks,
offline, and checks what must hold after any of them. The other tests of the
hooks run scripted sequences; what a person does between them - a branch
made or renamed under a running session, a worktree removed while a session
still names it, a session that enters a worktree, two sessions of a branch
answering in the same seconds, a commit while the agent writes its save, a
rebase stopped on a conflict - was driven nowhere.

From a seed, PHP Core is installed into a scratch project (its code
committed, the installation not) and random steps follow:
- turns of Claude Code sessions in the root, below it (`src/`) and in
  worktrees (through the dispatcher the installation carries), of a root
  session that enters a worktree during a turn, of Codex sessions in the
  root and below it and of Cursor sessions in the root: the prompt hook,
  the Stop hook, the save it asks for - made, skipped or ignored - and the
  Stop that settles it;
- bursts of up to three such turns at once, two of them in one folder too
  (never two Cursor sessions, which would share one rule file);
- new sessions, `/clear` and compactions;
- commits, branches made, renamed, detached and re-attached, a rebase
  stopped on a conflict (HEAD detached, the branch named only in the
  rebase's state) and later continued or aborted, and a commit or a branch
  move between a Stop and its save;
- worktrees added in `.claude/worktrees` and beside the root, some with
  spaces in their names, and removed.

What must hold: every hook exits 0 and the native journal records no error;
no prompt is told the memory was not consulted; every save lands in the task
of the branch it was asked on (in a stopped rebase, the branch being
rebased), and a sample of the saved learnings is found by `context.py
search` from the root and from every worktree left; after a turn checkpoint
in the root no index of a removed worktree is left; `context.py validate`
and the Memory Bank's `validate.py` pass; no worktree is written into. Only
a save asked for before its branch moved may be refused.

With `--attached` nothing is installed: every session is launched as the
Harness launches it, the edition lent from this clone through the overlay of
`accelerator_attach.py` (the project being the folder the session starts in)
and one state directory for the project and its worktrees. Claude Code and
Codex sessions start in the root or in a worktree; none enters one, since
the hooks lent to a session leave the memory out of a worktree it enters.
Besides the above, neither the project nor this clone's edition may be
written into (bytecode caches aside), and the Memory Bank is checked by
`context.py validate` alone: its `README.md` is the clone's.

```bash
python3 scripts/memory_lifecycle_fuzz.py [--seed N] [--steps N] [--edition "PHP Core"|Laravel|Symfony|WordPress] \
  [--attached] [--work DIR] [--verbose]
```

- **Outputs:** a JSON summary - the edition and layout, the sessions by host
  and kind, the events (bursts, worktrees entered, commits during a save,
  rebases stopped, continued and aborted), saves, the save lines of the
  native journal by code (`rebased` for a save another session overtook),
  worktrees added and removed, and every violation with the last steps
  before it. Exit 0 when everything held, 1 otherwise.
- **Writes:** only its work directory: a new temporary one, removed
  afterwards, unless `--work` names a missing or empty folder, which is
  kept. HOME, TMPDIR, the dispatcher's folder and an attached project's
  state are inside it; no model and no network.
- **CI relationship:** `tests/test_memory_lifecycle_fuzz.py`, a step of its
  own in the `memory` job, runs seeds 1 and 2 on PHP Core, which between
  them reach every host, a worktree entered, bursts, a commit during a save
  and a rebase stopped, continued and aborted, and seed 2 attached. A
  failing seed reproduces with the same `--seed` and `--steps`, apart from
  which of two sessions answering at once saves first; `--verbose` prints
  every step.

### `claude_code_bench.py`

**Purpose and status.** A maintainer-only check of the native save step on the
installed Claude Code, without a model. The save after every answer depends on
what Claude Code does with a Stop hook's output and on its permission check for
the command the agent runs. Both change between Claude Code releases, and a
unit test cannot see either.

It installs an edition into a small project. Then, inside a network namespace
with loopback only, it runs `claude -p` against a scripted
stand-in for the Messages API. The stand-in reads a file, answers, and follows
the Stop hook's save step exactly as written. The JSON it saves holds
backticks, `$`, brackets, an apostrophe, escapes and Cyrillic. Nothing leaves
the machine and no tokens are spent.

With `--target DIR` it runs in a copy of an installed project instead of a
fresh install. The copy leaves out dependency trees as the rollout
rehearsal's does, and `--sync` first brings it up to this clone. The saved
learning cites one of the project's own tracked files, and the project itself
is never written. A Git worktree or submodule is refused, as the rehearsal
refuses it.

With `--target` it also reads your own Claude Code configuration
(`~/.claude.json`, or `$CLAUDE_CONFIG_DIR/.claude.json`) for that project's
trust and nothing else. It warns when the trust dialog was never accepted:
there `claude -p` ignores the project's allow rules and refuses the save,
which the bench's own trusted HOME would hide.

`--attached` runs a fresh project with nothing installed and the edition
lent from this clone, as the Harness launches it (`accelerator_attach.py`:
`--add-dir`, one `--settings` with the edition's permissions and hooks, the
state outside the project). The project is left untrusted there, because
permissions from `--settings` count without the trust dialog.

`--mode` runs one permission mode (`default`, `acceptEdits`, `auto`,
`dontAsk`, `bypassPermissions` or `plan`). The namespace keeps your own uid
where `unshare` can map it (util-linux 2.38 and later), because Claude Code
refuses `bypassPermissions` to root; older `unshare` falls back to
`unshare -rn`. In `plan` no save is asked for, and the run passes when the
journal records `plan-mode`.

`--queued SECONDS` runs Claude Code in its terminal, through a
pseudo-terminal, instead of `claude -p`. The stand-in takes SECONDS to write
the save, and once it has begun a second question is typed: the terminal
takes it into the running turn and runs the prompt hook at once. The run
passes when the save in flight still lands (`save:saved`, with code
`in-flight` when the prompt moved its request aside) and the screen never
says the result was not saved. A question the stand-in has not received
within fifteen seconds - the terminal of a large project was not ready yet -
is typed again, up to three times, and the screen is kept in the work
directory as `screen.txt`. Its first run, on 2.1.278, found the prompt hook
re-arming the session: the save in flight was refused as
`request-unavailable`.

`--subfolder DIR` runs a first session in the project root, whose
session-start hook sets up the dispatcher for sessions started below the
root in the bench's own Claude Code configuration
(`memory-bank/scripts/claude_subfolders.py`), then the measured session in
DIR. The run passes when that session got the capsule and every line of the
project's `AGENTS.md` in the conversation itself - not in a file Claude Code
keeps an over-long hook output in - and its save landed.

`--worktree` does the same in a Git worktree of the project made where
Claude Code makes its own (`.claude/worktrees/bench`, on a branch of its
own), with `--subfolder DIR` in DIR of it. The installation is not
committed, so the worktree has none of it; the run also passes only when the
save landed in the root's Project Brain under the worktree's branch, the
worktree got an index of its own in the root, and nothing was written into
the worktree.

`--desktop` drives the measured session as the Claude desktop app drives
Claude Code: stream-json both ways, the initialize request first
(`--await-initialize`), `--setting-sources user,project,local`, the mode
named, and every permission prompt sent to the host over stdio, which the
bench refuses and lists. The run passes only when the session saved without
one: a save the app has to ask about asks the person after every answer.
With the dispatcher's allow for the memory CLI taken out, it fails there
with the save's command as the prompt. Left without a mode, Claude Code
2.1.299 under the app's flags decides an ask in auto mode, whose classifier
no offline run has: the app names the mode, and so does the bench.

`--sandbox` turns Claude Code's Bash sandbox (bubblewrap) on in the bench's
own settings, with the project on disk under `/var/tmp`: the sandbox mounts
a /dev of its own, and its bridge sockets under the run's temporary
directory must fit a socket path, which a run below `~/.cache` did not in
the desktop app's mode. Claude Code then starts without the namespace's
capabilities, which bubblewrap refuses. In the root the save writes the
project; below it and in a worktree it waits in the spool and the next
hook makes it (`spooled`).

`--permissions` (with `--subfolder`, `src` by default) has the stand-in try
nineteen tool calls in both sessions - reads of secrets the project denies
at the root and below it, of files it allows and of one outside, writes,
commands it denies or allows, `printenv` of the project's `env`, a domain it
allows and one it does not - and passes when the session below the root got
the decision the root got for every one (deny, ask or ran) and printed the
same `env`. Without the dispatcher's `permissions` entry ten of them differ.
Run it in `default` and `bypassPermissions` when Claude Code updates: its
path matching was probed, not documented.

```bash
python3 scripts/claude_code_bench.py [--claude PATH] [--edition "PHP Core"] \
  [--target DIR [--sync] [--exclude NAME] | --attached] [--shell /usr/bin/zsh] [--mode dontAsk] \
  [--queued 8 | --subfolder src [--permissions] | --worktree [--subfolder src]] [--desktop] [--sandbox] [--timeout 180] [--keep]
```

- **Outputs:** a JSON report:
  - Claude Code's version and the shell its Bash tool ran;
  - whether the save ran (the journal's events);
  - how many messages carried the step to the model, and their size;
  - the hook errors the transcript recorded, and the error notifications in
    the stream;
  - with `--queued`, whether the screen said the result was not saved;
  - with `--subfolder`, the folder, and whether the capsule and the whole
    policy reached the session;
  - with `--worktree`, also whether the save went to the worktree's branch,
    its own index exists and the worktree stayed clean;
  - with `--desktop`, the permission prompts the app would have shown.

  Exit 0 when the save ran, one message carried the step and no hook error
  was recorded; 1 otherwise; 2 without Linux namespaces.
- **Writes:** a temporary directory holding the project (or the copy) and an
  empty HOME, under `/dev/shm` when writable, removed unless `--keep`.
- **CI relationship:** CI has no Claude Code, so it cannot run the bench.
  `tests/test_host_benches.py`, in the `installation` job, holds what the
  bench relies on: the save command found in the step the runtime writes for
  every host, and the stand-in's turn. Run the bench itself when Claude Code
  updates - with `--queued 8` too - and with `--target … --sync` on a copy of
  each project before a rollout. Its first run, on 2.1.278, found two problems: the save command
  asked for permission after every answer, and the Stop block showed as a hook
  error.

### `codex_bench.py`

**Purpose and status.** The Codex counterpart of `claude_code_bench.py`, with
the same work directory, `--target` and `--sync`. Codex runs a project's hooks
only once their hashes are approved, and turns a Stop hook's block into a
`<hook_prompt>` item of the same turn. The save after every answer depends on
both, and on Codex's sandbox letting the save write the project.

The stand-in serves the Responses API. It follows the save step as written,
reading XML entities as a model does. It answers through `exec_command`, or
through code mode's `exec` script when Codex sends no tool list, as it does
for a project that pins a gpt-5.6 model.

An isolated `CODEX_HOME` approves the project's hooks by their current hashes,
as a person does once in `/hooks`. Then `codex exec` runs with the
workspace-write sandbox.

`--attached` runs a fresh project with nothing installed and the edition lent
from this clone, as the Harness launches Codex (`accelerator_attach.py`): the
hooks and instructions as `-c` options, the state as an added directory, and
the hooks trusted as `accelerator_attach.py trust-codex-hooks` records them.

`--queued SECONDS` types a second question while the save is written, as in
`claude_code_bench.py`, in Codex's terminal: Codex takes it into the running
turn (the same `turn_id`) and runs the prompt hook at once. The terminal runs
commands through the app-server daemon installed under `~/.codex/packages`,
which the isolated `CODEX_HOME` links to; `--queued` takes no `--attached`.
That daemon, and code mode's host, outlive the terminal and keep writing to
the run's `CODEX_HOME`: at the end of every run the bench stops each process
whose `CODEX_HOME` is the run's, and the report says how many
(`helpers_stopped`).

`--subfolder DIR` starts Codex in DIR below the project root. Its
workspace-write sandbox writes only below DIR - not the project's Brain, nor
its journal - so the save waits in the temporary directory and the session's
next hook, outside the sandbox, writes it: the run passes when the journal
says `save` `saved` with code `spooled` and the recovery was seen. Its work
folder is on disk, under `~/.cache/ai-infrastructure-benches/`: Codex's
bubblewrap sandbox mounts its own `/dev` and writes `/tmp`, so a project in
`/dev/shm` was invisible to a command started below its root, and one in
`/tmp` writable. With `--queued` it runs in Codex's terminal: the second
question moves the request aside (`previous-in-flight`) and the save still
lands through the spool.

```bash
python3 scripts/codex_bench.py [--codex PATH] [--edition "PHP Core"] [--sandbox read-only] \
  [--target DIR [--sync] [--exclude NAME]] [--queued 8] [--subfolder src] [--timeout 240] [--keep]
```

- **Outputs:** a JSON report:
  - Codex's version and the tools it offered;
  - how many hooks were approved;
  - how many messages carried the step, whether as a `<hook_prompt>` and
    XML-escaped;
  - whether the save ran and the second Stop saw the recovery (the journal's
    events).

  Exit 0 when the save ran and one message carried the step; 1 otherwise; 2
  without Linux namespaces. `CODEX_BENCH_DUMP=DIR` keeps every request body.
- **Writes:** as `claude_code_bench.py`.
- **CI relationship:** CI has no Codex. `tests/test_host_benches.py` holds the
  stand-in's turn: the escaped `<hook_prompt>` read unescaped, and code mode.
  Its first run, on 0.163,
  found that Codex adds the continuation to the same turn as a `<hook_prompt>`
  item, without running the prompt hook. The editions' documentation had said
  the continuation came back through the prompt hook.

### `memory_rerank_pilot.py`

**Purpose and status.** Optional, source-only maintainer pilot. It compares the
runtime's normal candidate order with a deterministic path/title feature
reranker on a local `context.db` and judged queries. It never changes the
runtime, index or CI.

```bash
python3 scripts/memory_rerank_pilot.py --database DB --queries QUERIES \
  [--judgments JUDGMENTS] [--limit 3] [--runtime DIR] [--json]
```

- **Inputs:** a local context index, JSON queries and optional grades.
- **Outputs:** per-query baseline/reranked useful paths and an aggregate.
- **Dependencies:** Python 3.9+ standard library; the selected edition's
  `context_retrieval.py`.
- **Writes:** none; it opens the database read-only.
- **CI relationship:** `tests/test_memory_rerank_pilot.py`; see
  [the pilot doc](../docs/MEMORY-RERANK-PILOT.md).

### `memory_sqlite_vec_pilot.py`

**Purpose and status.** Optional, off-by-default maintainer probe for a local
dense candidate pool using the sqlite-vec and sqlite-lembed loadable
extensions. The runtime stays standard-library only: this file is never
installed, imported by the runtime or run by CI.

```bash
python3 scripts/memory_sqlite_vec_pilot.py probe \
  --vec-extension PATH --lembed-extension PATH --model PATH --json
python3 scripts/memory_sqlite_vec_pilot.py fuse \
  --lexical lexical.json --dense dense.json --json
```

- **Inputs:** local extension and GGUF paths (assets are never downloaded) or
  two JSON lists of ranked paths.
- **Outputs:** JSON availability/versions, or deterministic RRF rows.
- **Dependencies:** Python 3.9+ standard library; the probe needs SQLite built
  with extension loading and the supplied local assets.
- **Writes:** none; it opens `:memory:` only.
- **CI relationship:** `tests/test_memory_sqlite_vec_pilot.py` covers the
  fail-closed paths and RRF; see
  [the pilot doc](../docs/MEMORY-SQLITE-VEC-PILOT.md).

### `collect_context.py`

**Purpose and status.** Optional developer-local `code2prompt` wrapper;
source-only and not installed. The root `./collect` wrapper and root Claude
`/collect` command expose the same implementation.

```bash
./collect --list
./collect skills --edition Laravel --dry-run
./collect edition --edition Symfony --format xml
./collect diff --base origin/main --stdout
./collect custom --include "docs/**" --exclude "docs/archive/**"
```

- **Scopes:** `edition`, `skills`, `core`, `hooks`, `tooling`, `docs`,
  `harness`, `diff`, and `custom`.
- **Principal options:** `--edition` where required; repeatable `--include` and
  `--exclude`; `--base` for `diff`; `--format`; `--encoding`; `--output`,
  `--stdout`, or `--dry-run`; `--top`; explicit `--with-task` and
  `--with-mirrors`.
- **Inputs:** scoped repository files and, for `diff`, Git changes against the
  merge base. `.git`, local databases/state, virtualenvs, credential-shaped
  files, and caches are always excluded. Uppercase `Task/` client material and
  generated mirrors are excluded by default.
- **Outputs:** Markdown, XML, or JSON bundle plus a JSON manifest when written
  to a file; stderr summary with file, byte, and tokenizer counts.
- **Dependencies:** Python 3 standard library, `code2prompt` in the tested
  range `>=4.2.0,<5.0.0`, and Git for `diff`. `CODE2PROMPT_BIN` can select the
  binary.
- **Writes:** list and help modes write nothing. Collection runs use an
  isolated temporary config directory. Normal output defaults to ignored
  `.c2p/` and writes a sibling manifest; `--stdout` and `--dry-run` do not
  create bundle/manifest files.
- **CI relationship:** no blocking CI job runs the external binary.
  `tests/test_collect_context` enforces containment, argument, and exclusion
  contracts in the `installation` job.

Every run rejects an empty match and isolates `XDG_CONFIG_HOME` because
`code2prompt` otherwise auto-loads ambient `.c2pconfig`. Its tokenizer count is
an OpenAI BPE comparison unit, not a Claude billing prediction.

### `check_links.py`

**Purpose and status.** Repository documentation validator; source-only and
not installed.

```bash
python3 scripts/check_links.py
python3 scripts/check_links.py --root /path/to/installed-accelerator
```

- **Options:** `--root PATH` checks every Markdown file in a standalone
  installed payload rather than the repository's tracked files.
- **Inputs:** by default, Markdown files returned by `git ls-files` and
  optional patterns from `check_links_ignore.txt`. Payload mode recursively
  scans `PATH` and deliberately applies no repository allowlist.
- **Outputs:** one diagnostic per unresolved relative target and exit 1 on
  findings; a success message and exit 0 otherwise.
- **Dependencies:** Python 3 standard library and Git.
- **Writes:** none.
- **CI relationship:** the `links` job runs it.

The checker URL-decodes paths, resolves file/directory targets, rejects links
that escape the selected root, and ignores external schemes, anchor-only
links, fenced code, and inline code. It checks target existence, not
heading-anchor validity. Tracked files deleted in the working tree are skipped.

### `asset_parity.py`

**Purpose and status.** Maintainer parity gate; source-only and not installed.
It holds the `memory-seed` generator asset
(`Infrastructure-Creator/.agents/skills/memory-seed/assets/`) to the canonical
edition, so an engine fix reaches the projects the generator builds and not
only the three ready-made editions.

```bash
python3 scripts/asset_parity.py --check
python3 scripts/asset_parity.py --check --json
python3 scripts/asset_parity.py --write
```

- **Options:** `--check` (default) reports and exits non-zero on drift;
  `--write` copies the canonical bytes over drifted or missing asset files,
  gives them the canonical executable bit (on disk and, for a tracked file,
  in the Git index), and re-checks; `--json` prints a machine-readable report.
  `--check` and `--write` are mutually exclusive.
- **Inputs:** the asset tree and the first present edition of `Laravel`,
  `Symfony`, `PHP Core`. Any of them will do, because
  `context.py parity --cross-edition` already holds the three to each other -
  that check is this one's prerequisite, not its duplicate.
- **Outputs:** one line per finding and exit 1; a success message and exit 0
  otherwise. Exit 2 on a missing asset tree.
- **Dependencies:** Python 3 standard library, Git, and `file_modes.py`.
- **Writes:** none under `--check`; only asset files (and their index modes)
  under `--write`.
- **CI relationship:** the `parity` job runs `--check` and
  `tests/test_asset_parity.py` once, on the `Laravel` matrix leg.

Five things are enforced: mapped asset files are byte-identical to their
counterpart (`scripts/` against `memory-bank/scripts/`, `templates/` against
`memory-bank/templates/`, `project-brain/` against itself) and carry the same
executable bit, read from the Git index first; every canonical
file the asset is supposed to seed exists in it, so a new core module cannot be
forgotten; every asset file is either mapped or listed in `ASSET_ONLY` with a
reason, so a new asset file cannot become silently unchecked;
`runtime.json.template` cannot match byte-for-byte and is compared by JSON key
set against the edition's `runtime.json`, so a new runtime setting reaches
generated projects or fails here; and every `memory-bank/scripts/` path in
`runtime-contract.json`'s `required_skeleton` is actually shipped.

Deliberate one-sided files live in `ASSET_ONLY` (the runtime contract, the
runtime template) and `EDITION_ONLY` (per-target prose, both Python test
suites, installer bookkeeping, the materialized `runtime.json`). Record a new
divergence there with its reason rather than widening a glob.

### `core_sync.py`

**Purpose and status.** Gives the memory core one source. A core change is
made in `PHP Core/` only, and this tool writes it to the other editions. The
file set is the runtime's own cross-edition manifest, read from `PHP Core`'s
`context_retrieval.py`, so the writer and `context.py parity --cross-edition`
cannot disagree.

`--write` then runs `asset_parity.py --write` and `build_mirrors.py --write`,
so one command carries a core change to the generator asset and the tool
mirrors as well. The copies are marked `linguist-generated` in
`.gitattributes`.

```bash
python3 scripts/core_sync.py --check
python3 scripts/core_sync.py --write [--no-chain]
```

- **Inputs:** the repository's four editions.
- **Outputs:** one `ADD`/`UPDATE`/`MODE`/`REMOVE` line per copy that
  differs from `PHP Core/`. `--check` exits 1 when any copy differs.
- **Dependencies:** Python 3.9+ standard library and `PHP Core`'s
  `context_retrieval.py`.
- **Writes:** only with `--write`:
  - the core files of Laravel, Symfony and WordPress, with the executable
    bit of the canonical file;
  - a core file removed from `PHP Core/` is removed from the copies;
  - the generator asset and the mirrors, through the two chained writers.
- **CI relationship:** `tests/test_core_sync.py` in the `parity` job. It
  covers the plan, the writer, the framework-shaped hooks it leaves alone and
  the `.gitattributes` marking.

### `file_modes.py`

**Purpose and status.** Shared helper module, imported by `build_mirrors.py`
and `asset_parity.py`; source-only, not installed, and not run directly.

- **What it answers:** the executable bit Git commits for a path - the index
  mode for a tracked path, the filesystem bit for an untracked one, and no
  answer for an untracked path on a filesystem without a trustworthy bit
  (Windows, `core.fileMode=false`), so callers skip the comparison rather than
  invent drift.
- **What it writes:** `set_executable` repairs the disk bit where the disk has
  one and queues the index mode of a tracked path; `flush` records the queued
  modes with `git update-index --index-info`, keeping the blob the index
  already holds. `git update-index --chmod` is avoided on purpose: it also
  stages the working-tree content.
- **Dependencies:** Python 3 standard library and Git.
- **Guard:** `tests/test_file_modes.py` (CI `mirrors` job) holds the
  repository's own index to the rule: every tracked `hooks/*.sh`, every hook
  script a tracked wiring file runs (whatever command form wraps it), and
  every root launcher with a shebang is `100755`.

### `policy_lock.py`

**Purpose and status.** Release gate for content identity; the lock files it
writes ARE installed, the script itself is not.

```bash
python3 scripts/policy_lock.py --check
python3 scripts/policy_lock.py --write --edition Symfony
python3 scripts/policy_lock.py --check --json
```

- **Options:** exactly one of `--check` (default) or `--write`; repeatable
  `--edition`; `--json`.
- **Inputs:** every tracked file of the model-facing surface — `AGENTS.md`,
  `CLAUDE.md`, `.claude/{DOD,GOLDEN-PRINCIPLES,STABILIZATION}.md`, the whole
  canonical `.agents/skills/` tree, the two `skill-creator` copies and
  `SKILL FLOW.md` that are canon in their own right, `.claude/agents/*-agent.md`,
  `.claude/commands/`, `.claude/settings.json`, `.cursor/rules/`,
  `.cursor/hooks.json`, `.codex/{hooks.json,config.toml}`.
- **Outputs:** `<edition>/.accelerator-policy-lock.json` — `schema_version`,
  `edition`, `release`, `policy_digest`, `agent_models`, `files`. Exit 1 on
  drift or a disallowed model.
- **Dependencies:** Python 3 standard library and Git.
- **CI relationship:** the `lint` job runs `--check` and its regression tests;
  `check_core_changelog.sh` requires an edition changelog entry when the same
  surface changes.

Three decisions worth knowing:

**Whole files, never projections.** Hashing only an agent's frontmatter would
store a digest under a path key that is not `sha256(path)`, making every
independent check a false positive. And an agent's *body* is its prompt: once
mirrors are regenerated, a body edit passes `mirrors`, `context_budget` and
`check_stabilization` alike. This gate is the only one that sees it.

**Enumerated from Git, never from disk.** `.cursor/rules/` holds a
runtime-rendered, gitignored `working-memory.mdc` that changes every turn; a
filesystem glob would bake it in and `--check` would fail for ever.

**The allowlist is enforced on `--write` too, and is per edition.** Validating
only on `--check` would let a regeneration record `model: sonet` and then
agree with itself. The three ready editions use haiku for four narrow agents;
Infrastructure-Creator's own `agent-forge` skill requires opus or sonnet and
it ships no haiku agent.

`policy_digest` identifies the surface, not a release: a surface change does
not require a `VERSION` bump, it requires the lock to be regenerated.

### `check_routes.py`

**Purpose and status.** Reference-integrity gate; source-only and not
installed. Mirrors prove the copies agree with canon; this proves canon points
at things that exist.

```bash
python3 scripts/check_routes.py
python3 scripts/check_routes.py --edition Symfony --edition wordpress
python3 scripts/check_routes.py --json
```

- **Options:** repeatable `--edition` (an edition path; `WordPress` and
  `wordpress` are accepted for `Cms/wordpress`); `--json`; `--allowlist PATH`.
- **Hook wiring:** every name under `(.claude|.cursor|.codex)/hooks/` in
  `.claude/settings.json`, `.cursor/hooks.json` and `.codex/hooks.json`, and
  the script name a Codex launcher passes as `$1` — matched on that tail, so
  the command prefix (`"$CLAUDE_PROJECT_DIR"/...`, a
  `git rev-parse --show-toplevel` form, bare relative) does not matter — must
  be a `*.sh` script that exists, is tracked, and carries index mode `100755`;
  a reference that drops `.sh` is an error, not a silent skip. A script under
  a tool's `hooks/` that nothing wires is a warning.
- **Routing:** Claude command `spawns`, `flow-next` and `flow-alternatives`;
  `stages[].agents`, command-body `subagent_type` and prose spawns
  ("spawn `x`", "Spawn the `x` agent", "Spawn `x` with", a line opening
  "Spawn x agent"), which must equal an agent's frontmatter `name` exactly —
  the host and `subagent-gate.sh` spawn by nothing else, so the file stem and
  the `<name>-agent` alias that `spawns` accepts are errors there; command-body
  "invoke the `x` skill" phrases, `` `/x` `` spans and `.<tool>/skills/<x>/`
  paths (Claude and Cursor); agent `name` (present, unique, bare kebab-case),
  Claude `invokes` and "invoke the `x` skill" phrases; every `SKILL FLOW.md`
  (slash tokens, backticked names with or without arguments, Phase Map items,
  and bare-name steps of fenced diagrams: a step at the line start, after a
  leading `->`/`→` or a tree branch `├─`/`└─`, that ends the cell — end of
  line, `(note)`, `<argument>`, or spaces before a box-drawing glyph — so
  diagram prose is not read as a step); and `AGENTS.md` slash spans plus
  "`x` skill/agent/command/hook" phrases. A `/x` resolves to a command or a
  skill of the same tool (in Claude Code every skill is also `/<skill>`); in
  the Codex flow (`.agents/skills/SKILL FLOW.md`) only a skill resolves,
  because Codex has no command layer.
- **Unreadable files:** a file that is not valid UTF-8 is an error finding
  (exit 1, valid `--json`), still checked with the bad bytes replaced.
- **Reachability:** every `.agents/skills/<x>` is named by some command,
  agent or flow.
- **Allowlist:** `scripts/check_routes_allowlist.json` — `unwired_hooks`,
  `unreachable_skills`, `references`; every entry needs a `reason`, an unknown
  edition is an error, a malformed entry is an error that silences nothing,
  and an entry that matches nothing is a stale warning.
- **Outputs:** findings grouped by edition with `file:line`; exit 1 on any
  error, 0 otherwise (warnings allowed), 2 on a usage or Git failure or an
  allowlist file that is not a UTF-8 JSON object (`--json` then prints
  `{"error": ...}`).
- **Dependencies:** Python 3.9+ standard library and Git (index modes).
- **Writes:** none.
- **CI relationship:** the `mirrors` job runs it and its regression tests.

### `check_stabilization.py`

**Purpose and status.** Policy structure gate; source-only and not installed.
It validates the `### Rule:` blocks in each edition's
`.claude/STABILIZATION.md` against the template that same file declares.

```bash
python3 scripts/check_stabilization.py
python3 scripts/check_stabilization.py --json
```

- **Checks:** the required fields (`Trigger`, `Root cause`, `Rule`,
  `Example`, `Enforcement`); that `Rule` states an obligation (MUST / MUST
  NOT), because a rule the harness cannot check compliance against is advice;
  that `Enforcement` names something; that `Added`/`Retired` are ISO dates;
  that a rule carrying `Retired` sits under `## Retired rules` and one sitting
  there carries a date; that `Superseded-by` names a rule in the same file;
  and that `Evidence`, when present, is a UUID resolving to a Project Brain
  record on disk.
- **Optional by design:** `Added` and `Evidence`. Requiring them would
  invalidate every rule written before they existed.
- **Inputs:** `<edition>/.claude/STABILIZATION.md` and, for `Evidence`, that
  edition's `project-brain/{dynamic,archive}/`.
- **Outputs:** one line per finding and exit 1; a per-edition rule count and
  exit 0 otherwise. An edition whose file carries no rule blocks is reported
  as *skipped*, not as passed — Infrastructure-Creator writes the cycle as
  prose, and silently approving a file nothing examined is the defect this
  gate exists to prevent.
- **Dependencies:** Python 3 standard library.
- **Writes:** none.
- **CI relationship:** the `lint` job runs it and its regression tests.

What it deliberately does not check: any `Edge`/`Blame` failure-localization
vocabulary. No such fields exist in any stabilization document in this
repository, and inventing a component taxonomy is a design decision, not a
validation one.

### `check_core_changelog.sh`

**Purpose and status.** Pull-request policy gate; source-only and not
installed. It requires a root `CHANGELOG.md` change when the diff touches the
shared memory/context core, Project Brain core, tool hooks, or repository
`scripts/`.

```bash
bash scripts/check_core_changelog.sh
bash scripts/check_core_changelog.sh origin/main
```

- **Input:** optional base ref, otherwise the PR base in GitHub Actions or
  `origin/main` with local `main` fallback; changed names from merge base
  through the current working tree.
- **Output:** pass/fail/skip explanation; exit 1 only when shared core changed
  without root changelog coverage.
- **Dependencies:** Bash, Git, `grep`, and `sed`.
- **Writes:** none.
- **CI relationship:** the pull-request-only `core-changelog` job invokes it
  with the PR base.

If no merge base can be resolved, the check deliberately degrades to an
explained exit-0 skip rather than answering from incomplete history.

### `token_budget.json`

**Purpose and status.** Versioned configuration for
`context_budget.py --check`; source-only and not installed.

- **Inputs/outputs:** no executable interface. It supplies per-edition ceilings
  for every measured category.
- **Dependencies:** valid JSON and exact edition/category alignment with
  `context_budget.py`.
- **Writes:** only maintainers edit it. Raise a ceiling only with a justified
  growth change; after deliberate slimming, tighten it with appropriate
  headroom.
- **CI relationship:** consumed by the `lint` job's context-budget step and
  also covered by the workflow's general JSON validation.

### `check_links_ignore.txt`

**Purpose and status.** Minimal allowlist configuration for `check_links.py`;
source-only and not installed.

- **Format:** one fnmatch-style pattern per line; blank lines and `#` comments
  are ignored. A pattern can match the raw target or resolved repository path.
- **Current rationale:** links into client-owned uppercase `Task/` material may
  target companion inputs absent from this repository. The allowlist avoids
  rewriting that client material.
- **Dependencies and writes:** no direct dependencies or runtime writes.
  Maintainers should prefer fixing repository-owned links over adding
  exceptions.
- **CI relationship:** read by the `links` job through `check_links.py`.

## Maintainer Workflow

When changing a canonical mirror source, edit only the canon, regenerate, and
verify:

```bash
python3 scripts/build_mirrors.py --write
python3 scripts/build_mirrors.py --check
python3 scripts/context_budget.py --check
python3 scripts/install_accelerator.py --verify-inventories
python3 scripts/check_links.py
```

Before pushing, run everything CI runs in one go, `python3 scripts/check.py`,
or the affected groups only (`--group mirrors --group parity`).

Inventory regeneration is appropriate only when intentionally changing the
production payload contract. Review source exclusions and production
overrides semantically; do not equate “tracked in an edition” with “installed
in a client project,” and do not hand-maintain exact payload counts in prose.
