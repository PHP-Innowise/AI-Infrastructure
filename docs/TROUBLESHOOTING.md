# Troubleshooting

This guide starts from observable symptoms and uses non-destructive diagnosis
first. Run commands from the selected accelerator root (`Laravel/`,
`Symfony/`, or `PHP Core/`) unless a command explicitly names another path.

Before recovery:

1. inspect `git status` and preserve unrelated work;
2. read the exact error, active edition README, root `AGENTS.md`, and DOD;
3. confirm the current directory and selected tool edition;
4. avoid deleting, resetting, force-pushing, dropping databases, or installing
   tools as a first response;
5. copy machine-local state before replacing it.

## Command or Skill Is Missing

### Symptom

A slash command does not appear, a skill does not trigger, or the tool says an
agent/command is unknown.

### Diagnose

Confirm the tool's native discovery path:

| Tool | Skills | Commands | Agents |
| --- | --- | --- | --- |
| Claude Code | `.claude/skills/<name>/SKILL.md` | `.claude/commands/*.md` | `.claude/agents/*.md` |
| Cursor | `.cursor/skills/<name>/SKILL.md` | `.cursor/commands/*.md` | `.cursor/agents/*.md` |
| Codex | `.agents/skills/<name>/SKILL.md` | unsupported in this repository | none by default |

Check the skill directory and validate it:

```bash
python3 .agents/skills/skill-creator/scripts/quick_validate.py \
  .agents/skills/<skill-name>
```

For Claude/Cursor, also inspect the corresponding command and agent
frontmatter and ensure `spawns`/`invokes` resolves to the intended wrapper and
skill. For Codex, ask to use the discovered skill by name; do not expect a
custom slash-command mirror.

### Recover

- Correct invalid frontmatter or a broken reference in the owning edition.
- Restore parity across supported skill copies.
- Restart or reload the tool only after the files validate.
- If a command exists only in another accelerator, switch to the correct
  Laravel, Symfony, or PHP Core folder rather than copying it blindly.

## Cursor Shows Duplicates or Hooks Fire Twice

### Symptom

Skills, agents, or commands appear twice; a warning or hook output is printed
twice; edits appear to trigger duplicate validation.

### Cause

The self-contained `.cursor/` edition is loaded while Cursor's optional
Claude-file loading is also enabled. Both native trees may be discovered.

### Recover

1. Open Cursor Settings.
2. Under Rules & Memories, disable the option that includes/reads Claude
   files when using this repository's `.cursor/` edition.
3. Keep `.cursor/` as the one active Cursor source.
4. Reload the workspace and verify one command/skill entry and one hook event.

Do not delete `.claude/`; it is the maintained Claude Code edition.

## Claude Code Repeatedly Requests Permission or Denies a Safe Command

### Symptom

Claude Code prompts for a routine command, refuses a file operation, or a
repository hook blocks after the permission check.

### Diagnose

There are two distinct controls:

1. `.claude/settings.json` permission allow/deny rules;
2. `.claude/hooks/` command guards wired by the same settings file.

Read the denial text to determine which layer acted. Confirm the exact command
shape: an allowed `Bash(php artisan:*)` pattern does not necessarily match a
wrapper, alias, container prefix, or differently ordered command.

### Recover

- Prefer an already allowed project-native command with equivalent behavior.
- For a legitimate repeated command, propose the narrowest permission change
  and review its security impact.
- Keep machine-specific changes in uncommitted
  `.claude/settings.local.json`.
- If a hook false-positive occurred, reproduce it with sanitized input, narrow
  the hook pattern, and add a regression test.

Never weaken `.env`, secret-path, destructive-shell, or database protections
merely to remove a prompt.

## Codex Skills or Hooks Do Not Load

### Symptom

Codex cannot see repository skills, session metadata is absent, or dangerous
commands are not intercepted.

### Diagnose

1. Confirm skills exist under `.agents/skills/`, not `.codex/skills/`.
2. Confirm the project is trusted; project-scoped `.codex/config.toml` and
   hooks load only for trusted projects.
3. Confirm `.codex/config.toml` contains:

   ```toml
   [features]
   hooks = true
   ```

4. Confirm `.codex/hooks.json` is valid and scripts exist under
   `.codex/hooks/`.
5. Run `bash -n` on each changed hook script and check executable bits.
6. Review the hook README for payload compatibility with the installed Codex
   version.

### Recover

Trust the repository only after reviewing its config and hook scripts. Reload
Codex after trust or configuration changes. If payload keys changed in a
Codex upgrade, update extraction logic and tests in all supported hook copies.
Current scripts may fail open when expected keys are absent, so an apparently
quiet hook is not proof of safety.

Do not add fake Codex command or one-skill agent trees.

## Python or SQLite FTS5 Is Unavailable

### Symptom

`context.py` reports a Python version/import error, `no such module: fts5`,
or cannot create an FTS virtual table.

### Diagnose

The Local Context Engine requires Python 3.9+ and SQLite with FTS5:

```bash
python3 --version
python3 -c 'import sqlite3; print(sqlite3.sqlite_version)'
python3 -c 'import sqlite3; c=sqlite3.connect(":memory:"); c.execute("CREATE VIRTUAL TABLE t USING fts5(body)"); print("FTS5 OK")'
```

Also confirm the script is the selected accelerator's
`memory-bank/scripts/context.py`, not a similarly named global utility.

### Recover

Use the project's documented Python runtime that includes FTS5. Do not install
or replace system Python without approval. If tooling is unavailable, report
`N/A - tooling not configured`; Project Brain Markdown/JSON remains the shared
authority, but local indexing/retrieval cannot be claimed as validated.

## SQLite Database Is Stale or Corrupt

### Symptom

SQLite reports corruption, schema errors persist after an automatic migration,
search results are clearly stale, or `status` cannot open
`memory-bank/local/context.db`.

### Diagnose

```bash
python3 memory-bank/scripts/context.py status --json
python3 memory-bank/scripts/context.py index --json
```

If SQLite can open the database, an integrity check is read-only:

```bash
python3 -c 'import sqlite3; c=sqlite3.connect("memory-bank/local/context.db"); print(c.execute("PRAGMA integrity_check").fetchone()[0])'
```

Distinguish data classes before replacement:

- the document index is derived and rebuildable;
- governed task authority is in `project-brain/`, although SQLite holds local
  bindings/cache;
- lightweight working tasks and local episodes exist only in SQLite and are
  not reconstructable from the repository.

### Recover

First preserve the database:

```bash
cp memory-bank/local/context.db \
  memory-bank/local/context.db.backup-YYYYMMDD-HHMMSS
```

If rebuilding is necessary, stop other context-engine processes, move the
original aside instead of deleting it, then reindex:

```bash
mv memory-bank/local/context.db \
  memory-bank/local/context.db.quarantine-YYYYMMDD-HHMMSS
python3 memory-bank/scripts/context.py index --json
python3 memory-bank/scripts/context.py status --json
```

Replace the timestamp placeholders manually before running. Do not perform
this recovery until lightweight tasks/episodes have been reviewed and exported
or their loss has been explicitly accepted. Keep the quarantine copy until
recovery is verified.

## Validation or Skill Parity Fails

### Symptom

`validate`, Memory Bank validation, tests, or `parity` reports drift.

### Diagnose

Run the checks separately so the failing contract is clear:

```bash
python3 memory-bank/scripts/context.py validate
python3 memory-bank/scripts/context.py parity
python3 memory-bank/scripts/validate.py
python3 -m unittest discover project-brain/tests
python3 -m unittest discover memory-bank/tests
```

`parity` compares mirrored skills with canonical `.agents/skills/`, as declared
by `project-brain/config/runtime.json`. `index` may report parity drift but
still refresh eligible documents; that warning is not a parity pass.

### Recover

- For intentional skill changes, reconcile `.agents/skills/` first, then
  mirror the final workflow to `.claude/skills/` and `.cursor/skills/`.
- Preserve required tool-native frontmatter and path transformations.
- For accidental drift, compare the logical skill and restore the intended
  canonical content; do not blindly overwrite an edition before reviewing
  tool-specific differences.
- Fix the first validator error, rerun the focused check, then rerun the full
  set.

Do not suppress the parity check or change `canonical_edition` merely to make
drift disappear.

## Update Fails with a Stale Revision

### Symptom

Project Brain reports a stale task/record revision.

### Cause

Another writer updated the governed record after the caller read it. This is a
compare-and-swap safety failure, not corruption.

### Recover

1. Reload the task or record:

   ```bash
   python3 memory-bank/scripts/context.py get --task-id TASK-ID --json
   python3 memory-bank/scripts/context.py brain-get --record-id UUID --json
   ```

2. Review the current revision, progress, transitions, handoff, and conflicts.
3. Reconcile both writers' changes without discarding either.
4. Retry the supported update with the new `--revision`.

Never edit the revision or transition history by hand and never retry with a
guessed number.

## Update Fails with an Owner Authorization Error

### Symptom

A Project Brain mutation says the actor is not authorized.

### Diagnose

The CLI actor comes from global `--owner`, then `PROJECT_BRAIN_OWNER`, then
`local`. The global option must precede the subcommand:

```bash
python3 memory-bank/scripts/context.py \
  --owner alice \
  brain-get --record-id UUID --json
```

Inspect the record's `owner` and `authorized_owners`. Confirm the operator
identity configured by the team:

```bash
printf '%s\n' "${PROJECT_BRAIN_OWNER:-local}"
```

### Recover

Use the correct stable owner identity if already authorized. If ownership must
change, obtain the project owner's approval and use a supported governance
workflow; do not hand-edit `authorized_owners` or impersonate another actor.

`PROJECT_BRAIN_OWNER` is an authorization label, not authentication. Setting it
to another name does not grant legitimate authority.

## Source Fingerprint Is Stale

### Symptom

Validation reports `source fingerprint is stale`, or retrieval excludes a
record for source freshness.

### Diagnose

Open the governed record and each path listed in `sources` and
`source_fingerprints`. Determine whether:

- the canonical source changed intentionally;
- the source moved or was removed;
- the record references the wrong source;
- an unreviewed modification should be reverted.

Do not treat the old hash as authority.

### Recover

- If the source change is wrong, restore the source through normal reviewed
  Git workflow.
- If the source change is correct, re-verify the record's claim against the
  new content, then use `update` or `brain-update` with the current revision
  and supported `--source`/evidence fields so the runtime recalculates
  fingerprints.
- If the claim is no longer true, transition, supersede, resolve, or cancel the
  record according to its lifecycle.

Never paste a newly computed hash into frontmatter by hand.

## Mode Switching Produces Missing or Duplicate Task State

### Symptom

A task visible in governed mode is absent in lightweight mode, a branch task
reappears after switching, or the same external task ID has divergent state.

### Cause

Mode changes do not migrate state:

- governed working authority is Project Brain;
- lightweight working authority is local SQLite.

### Diagnose

```bash
python3 memory-bank/scripts/context.py status --json
PROJECT_BRAIN_MODE=lightweight \
  python3 memory-bank/scripts/context.py status --json
```

Check `project-brain/config/runtime.json`, `PROJECT_BRAIN_MODE`, and any global
`--mode`; precedence is explicit option, environment, then runtime config.

### Recover

Complete, cancel, or preserve the current task before changing modes. Review
both states and intentionally reconcile them; do not use the same task ID in
both modes by default. Use governed mode for shared continuation and
lightweight mode only for explicitly local, disposable work.

See [`CONTEXT-MODES.md`](CONTEXT-MODES.md) for the full contract.

## Promotion Review or Apply Fails

### Symptom

A promotion cannot be reviewed/applied, remains unapplied, or validation fails
after an attempted apply.

### Diagnose

Confirm:

- the proposal exists under `project-brain/control/promotions/`;
- source IDs, paths, types, and revisions still match;
- an independent human reviewer is recorded;
- the outcome is approved rather than merely proposed;
- destination memory does not duplicate or conflict with an active chunk;
- privacy and sensitivity rules permit promotion.

Run:

```bash
python3 memory-bank/scripts/context.py validate
python3 memory-bank/scripts/validate.py
```

### Recover

Do not hand-edit promotion status, Memory Bank counters, or destination
revision fields. Supported apply operations are transactional: on failure,
partial Memory Bank and promotion writes should roll back. Preserve the error,
verify both stores, correct the underlying source/revision/privacy/conflict
problem through supported operations, and retry only after human approval
still applies to the current content.

If validation indicates partial state, stop promotion work, preserve the diff
and local database, and escalate for manual review. Do not claim promotion
completed.

## Archive or Index State Is Inconsistent

### Symptom

`project-brain/indexes/active.json` or `archive.json` is stale, an archived
record appears active, compaction fails, or SQLite search still returns a
deleted document.

### Diagnose

Project Brain indexes are deterministic navigation summaries; records remain
authority. SQLite's document index is separate.

```bash
python3 memory-bank/scripts/context.py validate
python3 memory-bank/scripts/context.py index --json
```

For terminal/superseded Project Brain records, validate before compacting:

```bash
python3 memory-bank/scripts/context.py compact
python3 memory-bank/scripts/context.py validate
```

### Recover

- Use `index` to refresh the disposable SQLite document index.
- Use `compact` only for records eligible under the protocol. It moves history
  atomically; it does not delete records.
- Do not move records between `dynamic/` and `archive/` or rewrite Project
  Brain index JSON manually.
- If a manually edited/merged record caused deterministic-index drift,
  reconcile the record through supported mutation paths or restore the last
  valid tracked state, then validate again.
- If compaction fails, retain all files and indexes. The runtime is designed to
  roll moves back; inspect the reported blocker before retrying.

## Hooks Do Not Run, Overblock, or Crash

### Symptom

No session metadata appears, a dangerous command is allowed, a safe command is
blocked, or a hook times out.

### Diagnose

1. Confirm the correct tool edition and native wiring file.
2. Validate JSON/TOML syntax with an available parser.
3. Run `bash -n <hook-script>`.
4. Confirm executable bits.
5. Compare the actual sanitized payload keys with the hook's extraction logic.
6. Check timeout units: Cursor and Claude both use seconds. A value such as
   `5000` is a millisecond leftover and is not a valid Claude timeout.
7. Check that Claude hook commands are bare script paths. An
   `echo '$TOOL_INPUT' | <script>` wrapper passes the literal variable name
   instead of the payload, and the hook then exits 0 on every call.
8. Check return-code semantics in the edition hook README.

### Recover

Make the smallest pattern or payload fix, then test:

- a known safe operation;
- a known blocked destructive operation;
- missing/unexpected payload keys;
- a script failure and timeout;
- duplicate loading;
- secret-safe error output.

Mirror shared script behavior across editions while preserving native event
wiring. Do not make hooks fail closed globally without evaluating the risk of
blocking all work, and do not make them fail open silently for high-risk
operations without documenting the limitation.

## Framework Tools Are Missing

### Symptom

`artisan`, `bin/console`, Pint, Doctrine commands, PHPUnit/Pest, PHPStan/Psalm,
or a Composer script is unavailable.

### Diagnose

First confirm accelerator placement and project evidence:

```bash
php -v
composer --version
composer validate --strict
```

Then inspect, without changing dependencies:

- `composer.json` scripts and `require`/`require-dev`;
- `composer.lock` and `vendor/bin/`;
- Laravel's `artisan`;
- Symfony's `bin/console`;
- native PHP entry points and project-specific scripts;
- container/dev-environment documentation.

An accelerator folder is workflow infrastructure, not necessarily a runnable
application. A missing framework executable may be expected if commands are
being run from the accelerator repository instead of a consuming project.

### Recover

Use the project's documented command or container wrapper. Prefer Composer
scripts because local and CI commands then match. Do not install a framework,
bundle, package, CLI, or analysis tool without approval. Report an applicable
missing check as:

```text
N/A - tooling not configured
```

Do not report it as passed, and do not substitute Laravel commands in Symfony,
Symfony commands in Laravel, or framework commands in PHP Core.

## Escalation Checklist

Stop recovery and ask for review when:

- the next step would discard uncommitted work or local SQLite episodes;
- owner/approval intent is unclear;
- source and record truth conflict;
- a promotion appears partially applied;
- a hook must be broadly weakened;
- a database target may be production;
- corruption affects tracked Project Brain records;
- three evidence-based recovery attempts fail.

Include the exact command, sanitized error, current mode/tool edition, affected
paths, checks already run, and preserved backup location. Do not include
secrets, customer data, raw logs, or private incident payloads.

For extension practices, see [`EXTENDING.md`](EXTENDING.md). For security
boundaries, see [`SECURITY.md`](SECURITY.md).
