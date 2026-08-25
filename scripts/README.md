# Repository Scripts

This directory contains repository-level build, installation, measurement, and
CI utilities. None of these files is part of a ready-made edition's installed
production payload. Runtime scripts that a consuming project needs live inside
the edition, primarily under `memory-bank/scripts/` and
`project-brain/scripts/`.

All Python utilities use only the Python standard library unless a section
states an external command requirement. Run examples from the repository root.

## Catalog

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
  repeatable `--edition` for `Laravel`, `Symfony`, `PHP Core`, or
  `Infrastructure-Creator`. The default is all editions.
- **Inputs:** canonical edition files and the edition-local mirror rules.
  Laravel, Symfony, and PHP Core load rules from
  `memory-bank/scripts/context_retrieval.py`; Infrastructure-Creator loads
  `mirror_rules.py`.
- **Outputs:** `--check` prints drift, missing files, and unaccounted mirror
  files and exits nonzero on findings. `--write` prints each changed path.
- **Dependencies:** Python 3 standard library and a complete repository
  checkout.
- **Writes:** `--check` never writes. `--write` creates/updates generated
  mirror files and regenerates each selected edition's `.gitattributes`.
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

- **Principal modes:** `--verify-inventories`, `--write-inventories`, or
  `--edition {Laravel,Symfony,PHP Core}` with `--target`.
- **Selection options:** repeatable `--tool {claude,cursor,codex}`; omission
  selects all tools. `--source-root` points at an alternate source checkout.
- **Collision options:** `--dry-run`; conservative `--merge-existing`; and
  destructive `--overwrite`, which is for explicit maintainer-controlled use,
  not normal adoption.
- **Inputs:** edition files, `VERSION`, inventories under
  `install/inventories/`, selected tool set, and target filesystem state.
  Inventories are the versioned production boundary: source exclusions and
  production-specific selection are resolved there without requiring
  documentation to depend on a particular future schema field name.
- **Outputs:** tab-separated `VERIFIED`, `COLLISION`, `WOULD_*`, `COPY`,
  `MERGE`, `COPY_AS`, `UNCHANGED`, and `COMPLETE` records; meaningful nonzero
  exit status on inventory errors or refused collisions.
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

Collision preflight is completed before normal copies begin. With
`--merge-existing`, identical files remain `UNCHANGED`; `.gitignore` and
`.gitattributes` receive only missing directives in a marked block;
`AGENTS.md` preserves project policy before one replaceable accelerator block;
and an existing root `README.md` is preserved while accelerator documentation
is written to `ACCELERATOR.md`. Every other differing selected path remains a
collision. A malformed managed block, symlink, non-file obstruction, or
conflicting `ACCELERATOR.md` is also refused.

The install transcript is an action log, not a backup and not an automatic
rollback facility. Its `COMPLETE files=` value counts selected inventory
entries, including `UNCHANGED` entries. Save the transcript with pipeline
failure propagation and retain a pre-install VCS/backup recovery point.

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
  printed: `token_budget.json` sets every ceiling at the observed value
  plus about five per cent, so headroom is ~4.8 % of the ceiling by
  construction and a 5 % warning would fire on every category at once.
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
  `--write` copies the canonical bytes over drifted or missing asset files and
  re-checks; `--json` prints a machine-readable report. `--check` and `--write`
  are mutually exclusive.
- **Inputs:** the asset tree and the first present edition of `Laravel`,
  `Symfony`, `PHP Core`. Any of them will do, because
  `context.py parity --cross-edition` already holds the three to each other -
  that check is this one's prerequisite, not its duplicate.
- **Outputs:** one line per finding and exit 1; a success message and exit 0
  otherwise. Exit 2 on a missing asset tree.
- **Dependencies:** Python 3 standard library.
- **Writes:** none under `--check`; only asset files under `--write`.
- **CI relationship:** the `parity` job runs `--check` and
  `tests/test_asset_parity.py` once, on the `Laravel` matrix leg.

Five things are enforced: mapped asset files are byte-identical to their
counterpart (`scripts/` against `memory-bank/scripts/`, `templates/` against
`memory-bank/templates/`, `project-brain/` against itself); every canonical
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

Inventory regeneration is appropriate only when intentionally changing the
production payload contract. Review source exclusions and production
overrides semantically; do not equate “tracked in an edition” with “installed
in a client project,” and do not hand-maintain exact payload counts in prose.
