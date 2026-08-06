# Safe Adoption Guide

This guide installs a ready-made Laravel, Symfony, or PHP Core edition into an
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
- `tasks/`, `specs/`, `examples/`, `.gitignore`, and root documentation.

The supported installer uses the versioned inventory under
`install/inventories/` and emits one transcript line for every file. Save that
exact transcript as the install manifest. It is required for safe rollback
because a merged directory may contain pre-existing project files that must
never be removed. The inventory describes repository distribution files only;
it does not claim machine-local, runtime, or user state.

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
  --dry-run
```

Use `--tool claude`, `--tool cursor`, or `--tool codex`; repeat `--tool` to
select more than one, or omit it to install all three integrations. Codex
selects both `.agents/` and `.codex/`. Shared cross-tool layout READMEs are
included as distribution documentation and do not activate an unselected tool.

Review every `WOULD_COPY` line. Any existing target path is reported as
`COLLISION`; the command exits nonzero before copying anything. Once the dry run
is collision-free, run the same command without `--dry-run` and retain its exact
`COPY` transcript:

```bash
python3 scripts/install_accelerator.py \
  --edition "PHP Core" \
  --target "$TARGET" \
  --tool cursor | tee "/safe/backup/path/accelerator-install-transcript.txt"
```

The installer refuses overwrite by default and performs a complete collision
preflight, so a late collision cannot leave a partially copied installation.
Do not use `--overwrite` for adoption; resolve and merge collisions explicitly
as described below.

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
used by the chosen edition. Preserve the selected edition's non-tool
directories and files, including `AGENTS.md`, `memory-bank/`,
`project-brain/`, `tasks/`, `specs/`, and the relevant documentation and
examples.

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
5. Review `git diff` and the install manifest. Only intended accelerator
   additions and reviewed merges should remain.

Missing optional project tooling is reported as unavailable; adoption must
not install it silently or claim that it passed.

## 9. Roll Back or Uninstall

Close active AI-tool sessions before changing integration files.

1. Use the install manifest to remove only paths created by this adoption.
2. Restore only files that this adoption changed from the verified backup or
   VCS state.
3. Never delete an entire merged directory such as `.cursor/`, `.claude/`,
   `.agents/`, `.codex/`, `memory-bank/`, or `project-brain/`.
4. The ignored `memory-bank/local/context.db` is disposable and may be removed
   after confirming no needed lightweight-only local state remains.
5. Re-run project checks and `git status --short`.

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
