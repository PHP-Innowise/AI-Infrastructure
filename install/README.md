# Accelerator Installer

`scripts/install_accelerator.py` installs a ready-made Laravel, Symfony, or PHP
Core accelerator into an existing project from a versioned file inventory. It
copies the contents of the selected edition into the target root and refuses
collisions before writing anything.

Use this installer for ready-made editions. For projects with substantial
custom architecture, integrations, or internal conventions, use
[Infrastructure-Creator](../Infrastructure-Creator/README.md).

## Prerequisites

- Clone or check out the accelerator repository separately from the target.
- Use Python 3.9 or newer.
- Put the target project in a clean, recoverable Git state.
- Back up existing AI configuration, policies, Memory Bank, and Project Brain
  data.
- Run commands below from the accelerator repository root.

The installer does not install PHP, Composer, framework packages, AI clients,
or other dependencies.

## 1. Choose an Edition

- `Laravel` for Laravel projects.
- `Symfony` for Symfony projects.
- `PHP Core` for native Composer/PSR projects, microframeworks, or frameworks
  without a dedicated edition.

Use the edition supported by evidence in the target project's `composer.json`
and lock file.

## 2. Verify the Distribution Inventories

```bash
python3 scripts/install_accelerator.py --verify-inventories
```

The command must report `VERIFIED` for Laravel, Symfony, and PHP Core. An
inventory mismatch means the source checkout is incomplete or its distribution
files changed without an inventory update.

## 3. Run a Dry Run

```bash
TARGET="/path/to/existing-project"

python3 scripts/install_accelerator.py \
  --edition "PHP Core" \
  --target "$TARGET" \
  --tool cursor \
  --dry-run
```

Available tool values are:

- `--tool claude` for `.claude/`;
- `--tool cursor` for `.cursor/`;
- `--tool codex` for `.agents/` and `.codex/`.

Repeat `--tool` to select multiple integrations. Omit it to install all three.
Shared policy, workflow, Memory Bank, and Project Brain files are included with
every selection.

Review all `WOULD_COPY` lines. If the target contains any selected path, the
installer prints `COLLISION`, returns a nonzero exit code, and copies nothing.

## 4. Resolve Collisions

Do not use `--overwrite` for normal adoption. Review every conflict and choose
one of these actions:

1. Keep the existing project file.
2. Merge compatible accelerator behavior into it manually.
3. Rename and update references when the AI tool supports that layout.
4. Abort the installation.

Never replace existing project policy, hooks, skills, Memory Bank chunks,
Project Brain records, or `.gitignore` without a semantic review.

## 5. Install and Save the Transcript

After obtaining a collision-free dry run, repeat the same command without
`--dry-run`:

```bash
python3 scripts/install_accelerator.py \
  --edition "PHP Core" \
  --target "$TARGET" \
  --tool cursor |
  tee "/safe/backup/path/accelerator-install-transcript.txt"
```

Keep the `COPY` transcript. It identifies files created by the installation and
is required for safe rollback. A final `COMPLETE` line reports the edition,
selected tools, and copied file count.

## 6. Validate the Installation

From the target project root:

```bash
python3 memory-bank/scripts/validate.py
python3 project-brain/scripts/validate.py --root .
python3 memory-bank/scripts/context.py status --json
python3 memory-bank/scripts/context.py validate --json
python3 memory-bank/scripts/context.py index
git status --short
```

Also run the target project's normal Composer validation, tests, formatter,
static analysis, and framework checks. Confirm that
`memory-bank/local/context.db` is ignored and that the selected AI client loads
its skills and hooks.

## Rollback

Use the saved transcript to remove only files created by the installer. Never
delete whole `.claude/`, `.cursor/`, `.agents/`, `.codex/`, `memory-bank/`, or
`project-brain/` directories because they may contain pre-existing project
files or active records.

## Maintainer Commands

After intentionally changing distribution files:

```bash
python3 scripts/install_accelerator.py --write-inventories
python3 scripts/install_accelerator.py --verify-inventories
python3 -m unittest tests.test_installation
```

Review generated inventory changes before committing them.

## Further Documentation

- [Full safe-adoption procedure](../docs/ADOPTION.md)
- [Tool-specific activation](../docs/TOOL-INTEGRATIONS.md)
- [Operations and health checks](../docs/OPERATIONS.md)
- [Troubleshooting](../docs/TROUBLESHOOTING.md)
