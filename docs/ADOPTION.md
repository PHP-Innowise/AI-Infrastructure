# Safe Adoption Guide

This guide installs a ready-made Laravel, Symfony, PHP Core, or WordPress edition into an
existing project without replacing existing project files. An accelerator is
a workflow layer; the consuming project's code, configuration, tests,
specifications, and CI remain authoritative.

For the concise command reference, see the
[Installer Quick Start](../install/README.md). This document remains the
authoritative procedure for backups, collision resolution, validation,
rollback, and upgrades.

## 1. Select the Edition from Evidence

Inspect the target project's `composer.json`, lock file, executable entry
points, and declared PHP/framework versions.

- Choose [Laravel](../Laravel/README.md) when the project uses Laravel,
  Artisan, or Eloquent. Its baseline is PHP 8.2+ for Laravel 12 and PHP 8.3+
  for Laravel 13.
- Choose [Symfony](../Symfony/README.md) when the project uses Symfony. Its
  documented baselines are Symfony 7.4 LTS on PHP 8.2+ and Symfony 8.1 on PHP
  8.4+.
- Choose [PHP Core](../PHP%20Core/README.md) for native Composer/PSR projects,
  microframeworks, or frameworks without a dedicated edition. Its baseline is
  PHP 8.2+.
- Choose [WordPress](../Cms/wordpress/README.md) for plugins, classic/block
  themes, Gutenberg blocks, custom WordPress sites, multisite components, or
  WooCommerce extensions. Follow the target project's supported versions.

Do not select an edition by preference when the project evidence identifies a
different stack. Use
[Infrastructure-Creator](../Infrastructure-Creator/README.md) instead when
the project has substantial custom architecture, internal conventions,
unusual integrations, specialized CI/CD, or needs a generated tool subset
grounded in its actual files.

## 2. Check Prerequisites

Use the project's existing setup instructions first. At minimum, confirm:

```bash
php -v
composer --version
python3 --version
python3 -c "import sqlite3; db=sqlite3.connect(':memory:'); db.execute('CREATE VIRTUAL TABLE fts_check USING fts5(content)'); print('SQLite FTS5 available')"
```

The ready-made PHP editions expect PHP 8.2+ and Composer 2+. Run the
project's normal dependency installation and verification tooling; do not
silently install PHPUnit/Pest, formatters, static analyzers, framework CLIs,
bundles, or npm packages.

The local context engine requires Python 3.9+ and the standard-library
`sqlite3` module built with SQLite FTS5 support. The definitive functional
check is the post-install `context.py index` command: it reports
`SQLite FTS5 support is required` if FTS5 is unavailable.

## 3. Back Up and Inventory

Work on a clean branch or other recoverable VCS state. Record:

```bash
git status --short
```

Back up every target path that could collide, outside the target working tree
or in an approved VCS commit. At minimum inspect:

- `AGENTS.md` and `CLAUDE.md`;
- `.claude/`, `.cursor/`, `.agents/`, and `.codex/`;
- `memory-bank/` and `project-brain/`;
- `tasks/`, `specs/`, optional client-owned `Task/`, `.gitignore`, and root
  documentation.

The supported installer uses the versioned inventories under
`install/inventories/`. Those inventories define the resolved production
payload, including conceptual source exclusions and any production-specific
selection needed by the current inventory schema. Do not assume every tracked
edition file is installed, and do not couple adoption automation to an
undocumented inventory field name.

The production payload retains runtime scripts, policies, workflow
documentation, templates, Memory Bank, Project Brain, living-spec scaffolding,
and lowercase `tasks/`. Repository research, test suites, worked examples, and
bundled uppercase `Task/` material are source-only and are not installed.
Uppercase `Task/` may be created and populated later when real client
requirements or design assets exist; lowercase `tasks/` remains the operational
temporary-work scaffold.

The installer emits one transcript line for every selected inventory entry.
Save that exact transcript as an action log and pair it with a pre-install
backup or VCS recovery point. It is not a content backup: it records actions
but not previous bytes, and its final selected-entry count includes
`UNCHANGED` files. The inventory likewise does not claim machine-local runtime
or user state.

## 4. Dry-Run Before Copying

Keep the accelerator source outside the target. From the accelerator repository
root, substitute the selected edition, target, and tool. Paths containing spaces
are supported:

```bash
TARGET="/path/to/existing-project"
python3 scripts/install_accelerator.py \
  --edition "PHP Core" \
  --target "$TARGET" \
  --tool cursor \
  --merge-existing \
  --dry-run
```

Use `--tool claude`, `--tool cursor`, or `--tool codex`; repeat `--tool` to
select more than one, or omit it to install all three integrations. Codex
selects both `.agents/` and `.codex/`. Shared cross-tool layout READMEs are
included as distribution documentation and do not activate an unselected tool.

With `--merge-existing`, identical files are `UNCHANGED`; `.gitignore`,
`.gitattributes`, and `AGENTS.md` use marked conservative merges; and an
existing root `README.md` is preserved while accelerator documentation is
written as `ACCELERATOR.md`. Review every `WOULD_COPY`, `WOULD_MERGE`, and
`WOULD_COPY_AS` line. Any unsupported existing path remains a `COLLISION`, and
the command exits nonzero before writing anything.

Once the dry run has no unsupported collisions, run the same command without
`--dry-run` and retain its exact transcript:

```bash
set -o pipefail
python3 scripts/install_accelerator.py \
  --edition "PHP Core" \
  --target "$TARGET" \
  --tool cursor \
  --merge-existing |
  tee "/safe/backup/path/accelerator-install-transcript.txt"
```

`pipefail` is important when using Bash: without pipeline failure propagation,
`tee` can return success even when the installer failed. In another shell,
use its equivalent or inspect the installer's status separately.

The installer refuses overwrite by default and performs a complete collision
preflight, so a late collision cannot leave a partially copied installation.
Do not use `--overwrite` for adoption. Use the supported merge mode for standard
root files and resolve every remaining collision explicitly as described below.

## 5. Resolve Collisions Explicitly

Use one of these outcomes for every collision:

1. **Keep:** retain the project file and do not install the conflicting
   accelerator file.
2. **Merge:** add only compatible missing policy or configuration while
   preserving project-specific behavior.
3. **Rename/reference:** keep both only when the tool supports the resulting
   location and references are updated consistently.
4. **Abort:** stop adoption and restore the pre-install state.

Do not choose destructive overwrite. In particular:

- Merge `AGENTS.md` by authority and scope; do not discard existing policy.
- Treat an existing `CLAUDE.md` as project policy/context that must be
  reconciled with `AGENTS.md`, not replaced.
- Merge native tool directories file by file. Existing settings, hooks,
  commands, agents, or skills may carry project-specific behavior.
- Reconcile `.agents/skills/` by skill name. The shipped runtime declares
  `.agents` as the canonical skill edition, so a same-name skill is a semantic
  conflict.
- Preserve existing `memory-bank/` chunks, IDs, index, and counter. Validate
  any merged result rather than copying a second authority over it.
- Preserve existing `project-brain/` records, indexes, schemas, runtime
  configuration, and revisions. Do not replace active work state.
- Merge `.gitignore`; never replace the project's ignore policy.

If collisions are broad or the project already has mature agent
infrastructure, stop manual adoption and use Infrastructure-Creator's
read-only `infra-scan` followed by reviewed generation. Its collision guard
requires an explicit merge/add-only or abort decision before writing.

## 6. Choose Tool Scope

All installations need the shared root policy and context/workflow support
used by the chosen edition. Preserve the selected production payload's
non-tool directories and files, including `AGENTS.md`, `memory-bank/`,
`project-brain/`, lowercase `tasks/`, `specs/`, and installed runtime
documentation and templates. Do not manually copy source-only tests, research,
worked examples, or bundled uppercase `Task/` material around the inventory.

Then install only the native integration trees the team uses:

- **Claude Code only:** `.claude/`
- **Cursor only:** `.cursor/`
- **Codex only:** both `.agents/` and `.codex/`
- **All tools:** `.claude/`, `.cursor/`, `.agents/`, and `.codex/`

Do not install `.agents/` without `.codex/` when Codex hooks/configuration are
expected, and do not create `.codex/commands` or `.codex/agents` substitutes.
For cross-tool details, see [Tool Integrations](TOOL-INTEGRATIONS.md).

## 7. Add Required Local Ignores

The context engine creates a disposable SQLite database and machine-local
state under:

```text
memory-bank/local/
```

Ensure the target's ignore rules contain `memory-bank/local/` before running
indexing. The editions also ignore Python caches (`__pycache__/`, `*.py[cod]`)
and local worktrees (`.worktrees/`). Preserve equivalent existing project
rules. Keep personal tool overrides such as `.claude/settings.local.json`
uncommitted according to the tool and team policy.

Do not ignore shared `memory-bank/chunks/`, `project-brain/` governed records,
or installed policy/skill/hook files merely to make adoption easier.

## 8. Validate After Installation

From the target root:

```bash
python3 memory-bank/scripts/validate.py
python3 project-brain/scripts/validate.py --root .
python3 memory-bank/scripts/context.py status --json
python3 memory-bank/scripts/context.py validate --json
python3 memory-bank/scripts/context.py index
git status --short
```

Then:

1. Confirm `memory-bank/local/context.db` exists but remains ignored.
2. Inspect `project-brain/config/runtime.json`; ready-made editions should
   declare the selected framework, governed mode, `sqlite-fts5`, and
   `"canonical_edition": ".agents"`.
3. Validate the installed tool using the activation checks in
   [Tool Integrations](TOOL-INTEGRATIONS.md).
4. Run the consuming project's normal Composer validation, tests, formatter,
   static analysis, and framework checks. Use project scripts first.
5. Review `git diff` and the saved install transcript. Only intended accelerator
   additions and reviewed merges should remain.

Missing optional project tooling is reported as unavailable; adoption must
not install it silently or claim that it passed.

## 9. Roll Back or Uninstall

Close active AI-tool sessions before changing integration files.

1. Use the transcript to identify `COPY` and `COPY_AS` candidates, and remove
   only paths created by this adoption that have not since become
   project-owned.
2. Restore `MERGE` or `OVERWRITE` files from the verified backup or VCS state
   when exact rollback is required. The transcript does not contain their old
   contents.
3. Never delete an entire merged directory such as `.cursor/`, `.claude/`,
   `.agents/`, `.codex/`, `memory-bank/`, or `project-brain/`.
4. The ignored `memory-bank/local/context.db` is disposable and may be removed
   after confirming no needed lightweight-only local state remains.
5. Re-run project checks and `git status --short`.

There is no automatic installer rollback command. For a `MERGE`, do not blindly
delete every line that resembles accelerator content: `.gitignore` or
`.gitattributes` directives already present before adoption remain
project-owned, while installer-added directives are kept in a marked block.

Deleting the SQLite database does not remove governed Project Brain records or
reviewed Memory Bank chunks. In lightweight mode, however, local working tasks
and episodes live in that database and are lost with it.

## 10. Upgrade Safely

Treat an edition update as a new adoption:

1. Read the source edition's `CHANGELOG.md` and compare old, local, and new
   versions in a staging area.
2. Back up and dry-run again.
3. Preserve project-specific policy, hooks, tool permissions, skills, and
   active context records.
4. Resolve same-name skill changes semantically; do not bulk-replace local
   adaptations.
5. Keep native wrappers aligned with the canonical `.agents/skills` behavior
   while retaining each tool's required frontmatter and event schema.
6. Re-run structural, context, tool activation, and project verification.

For heavily adapted or drifting installations, regenerate from current
project evidence with Infrastructure-Creator rather than repeatedly layering
generic files over the project.
