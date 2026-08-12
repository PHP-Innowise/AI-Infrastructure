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
  verification/writing uses `git ls-files`.
- **Writes:** verification and dry-run do not write. Installation creates
  selected files in the target. `--write-inventories` regenerates repository
  inventory JSON. The target path rejects symlink components.
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
```

- **Principal option:** `--check` compares all measured categories with
  `token_budget.json`.
- **Inputs:** each edition's `AGENTS.md`, canonical
  `.agents/skills/*/SKILL.md`, Claude command/agent listings, and, in check
  mode, `scripts/token_budget.json`.
- **Outputs:** a human-readable byte/token-estimate report, or per-category
  pass/fail lines. `--check` exits nonzero for budget excess, malformed/missing
  ceilings, or edition drift.
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
