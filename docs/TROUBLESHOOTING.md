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
   `.codex/hooks/`. Codex trusts each hook by the hash of its definition, so
   after an update that changed a command, `/hooks` lists it for review and
   it is skipped until trusted again.
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

In governed mode the commands the hooks run (`refresh`, `turn`,
`hook-context`, `retrieve`, `index`) recover by themselves when SQLite reports
the file damaged ("database disk image is malformed", "file is not a
database"): they move it and its journal files aside to
`memory-bank/local/context.db.quarantine-<UTC time>`, build a new one and run
once more, and the capsule says so. Everything in it is derived there - the
index is rebuilt, the binding comes back from the Brain at the next
checkpoint - except the turns buffered since the last checkpoint. Lightweight
mode keeps its working tasks only in that file, so it is never moved
automatically; use the steps below.

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
by `project-brain/config/runtime.json`. A single-tool install (`--tool claude`
or `--tool cursor`) has no `.agents/skills/`; `parity` then compares against the
first tree that is installed and names it in its output, and a tool directory
that holds only its README is not treated as an installed mirror. `index` may
report parity drift but still refresh eligible documents; that warning is not a
parity pass.

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

## Two Task Records Claim One Branch

### Symptom

The capsule warns "Task record <UUID> also claims <branch>; this turn uses
<UUID>.", or `context.py rebind --task-id <branch>` reports that the
identifier is ambiguous.

### Cause

Two copies of the branch each started its task - two people, or one person
on two machines, before either pulled the other's commit - and the branches
were merged. Each record is valid; the task ID now names both.

### What the runtime does

A machine without a binding for the branch reads, and binds to at its next
checkpoint, the earliest-created active record, so every new clone settles
on the same one. A machine already bound to a record keeps it. The capsule
names the other record on every prompt until one of them is closed.

### Recover

Keep one task per branch. Carry what matters from the other record's
progress and next steps into the kept one, then cancel the other:

```bash
python3 memory-bank/scripts/context.py brain-update --record-id <other UUID> \
  --revision auto --transition cancelled --reason "Duplicate of <kept UUID>"
```

A machine bound to the record you cancelled follows the branch to the record
that is left: its next prompt reads it and its next checkpoint binds to it.
When owners are configured (`PROJECT_BRAIN_OWNER`), only an authorized owner
of the record can cancel it - see "Update Fails with an Owner Authorization
Error".

## The Capsule Carries No Working State, or Another Task's

### Symptom

A prompt's capsule opens with something other than `working: <task> — <goal>`,
or carries a warning about the branch's task.

### What each first line means

| First line or warning | Meaning | What to do |
|---|---|---|
| `working: not recorded yet (the task starts at the 5-turn checkpoint)` | The branch has no task yet; a later checkpoint records one. | Nothing, or `context.py start` to record it now. |
| `working: closed (this branch's task is finished; ...)` | The branch's task was completed or cancelled, and turns after it are not recorded. | `python3 memory-bank/scripts/context.py start --task-id <branch> --goal "..."`; the finished record moves to the archive. |
| `working: unavailable (the task's record cannot be read)` | The record is invalid - a merge conflict, a hand edit; a warning names the file. | Repair or restore that file; checkpoints wait for it. |
| `working: unavailable (the task's record changed while this turn read it; read <path>)` | Another session's checkpoint rewrote the record in the last moments of this read. | Read the named record; the next prompt carries it again. |
| warning `This branch was renamed from <old>; its task continues here.` | The branch was renamed with `git branch -m`, and its earlier name's task is used. | Nothing; `start --task-id <new name>` records a task of its own instead. |
| `working: <id> — ...` where `<id>` is the branch name with `-` for some characters and eight hex digits at the end | The branch name has characters a task ID does not take; the ID is derived from it. | Nothing; use that ID with the CLI. |

On a detached HEAD there is no task, and the prompt hook stays silent -
except during a rebase, where it reads the branch being rebased.

## The Agent Saves After Every Answer

### Symptom

After each answer the agent runs one more shell command. The command pipes a
JSON object into `context.py ... native-result`, and in some sessions the
agent then repeats its answer. Each turn takes noticeably longer and costs
more.

### Cause

This is the automatic native result capture of governed mode. The Stop hook
asks the same agent to save the result of the work it just did - progress,
next steps and at most three source-backed learnings - so that the next
session's capsule can recall them. The request follows every answer. The
agent repeats its answer where a headless run (`codex exec`, `claude -p`)
would show only the last message. A session the host marks as attended -
Claude Code in the terminal, VS Code or the desktop app; Codex in its app,
IDE extension or terminal UI - ends with at most one short line instead;
every other session, Cursor's included, repeats (see
`docs/MEMORY-INTEGRATION.md`). On 21 Codex runs with one save each, the
continuation cost a median of 10 seconds, 383 output and 59k mostly cached
input tokens: about half of the answer's own. Only the first step of a
conversation, and the first after a compaction, carries the full rules; later
ones point back to them and are about 900 characters shorter.

An agent that also runs `context.py record-result` or `memory_record_result`
during the answer saves it twice. The `AGENTS.md` of releases before 24
October 2026 asked for that explicit save after every answer; the current
policy asks for it only where the Stop hook requests no save.
`install_accelerator.py --sync` rewrites the policy's managed block, with
`--update-tracked` where the project's Git tracks `AGENTS.md`.

### Recover

To keep the capsule but drop the request, set `"native_result_request": "off"`
in `project-brain/config/runtime.json`, or `CONTEXT_NATIVE_RESULT_REQUEST=off`
for one session.

- `--sync` never rewrites `runtime.json`, so the setting stays.
- Results then reach memory only when an agent or a person records them:
  `context.py record-result`, the MCP `memory_record_result` tool, or the
  Harness.
- The native journal (`memory-bank/local/native-results.ndjson`) shows each
  skipped prompt and Stop as `skipped` with the code `switched-off`.

To keep the request but stop the repeat in a session the hooks do not
recognise as attended, set `"native_result_answer": "brief"` (or
`CONTEXT_NATIVE_RESULT_ANSWER=brief`). Only do this where nobody reads the
final message alone: a headless caller would then get one short line instead
of the answer. `repeat` restores the repeat everywhere.

## Claude Code Asks Before the Save After Every Answer

### Symptom

After each answer Claude Code asks to approve a command like
`cd <project> && echo -E '{...}' | python3 memory-bank/scripts/context.py ... native-result ...`.
Under `claude -p` the command is refused instead and the result is not
saved: `memory-bank/local/native-results.ndjson` records `stop` `failed` with
the code `host-refused` (`unsaved` on runtimes before 24 October 2026).

### Cause

Claude Code runs that command without asking only when
`.claude/settings.json` allows the memory CLI
(`Bash(python3 memory-bank/scripts/context.py:*)`, with `python` and `py -3`
for Windows) and the project is trusted. Claude Code ignores a project's allow
rules until its trust dialog has been accepted once. A team-edited
`settings.json` may have lost the rules.

Runtimes released before 23 October 2026 gave the command as
`(cd <project> && python3 ...)`. Claude Code asks before any command holding a
subshell, whatever the allow rules say. They also sent the request as a Stop
block, which the transcript shows as "Stop hook error" with an error
notification after every answer.

### Recover

- Accept the trust dialog once in an interactive session.
  `python3 scripts/claude_code_bench.py --target DIR` says whether your Claude
  Code has accepted it for that project.
- Put the three allow rules back into `.claude/settings.json`. The edition's
  own file lists them. The dialog's "Yes, and don't ask again for: echo *"
  does not help: it adds `Bash(echo *)` to `.claude/settings.local.json`,
  which leaves the memory CLI unapproved, and the next session asks again.
- On an older runtime, update the accelerator (`--sync`). Until then, set
  `"native_result_request": "off"` to stop the request (see the previous
  section).

After Claude Code refuses the save once, the session is not asked again for
a day: the journal records `stop` `failed` `host-refused`, then `prompt`
`skipped` `host-refused`. A new session is asked again, so check the trust
and the rules before starting one.

## A Headless Run Hits Its Turn Limit After a Complete Answer

### Symptom

`claude -p ... --max-turns N` (or an Agent SDK query with `maxTurns`) ends
with `"subtype": "error_max_turns"`, `"is_error": true` and no `result`,
although the agent had answered. In `memory-bank/local/native-results.ndjson`
the run's last lines are `stop` `requested`, sometimes followed by `save`
`saved`.

### Cause

The save continues the turn: one model call runs the save command and one
more ends the turn. Claude Code counts both against `--max-turns`. An answer
given in the last two allowed calls leaves the save no room, so the run ends
on the limit instead of on the answer. Claude Code tells the Stop hook
neither the limit nor the count, so the hook cannot step aside.

Seen offline on Claude Code 2.1.278, with the answer on the second model call:

- `--max-turns 2`: `error_max_turns`, nothing saved;
- `--max-turns 3`: the result saved, then `error_max_turns` before the final
  message.

### Recover

- Give a headless run that keeps the request two more turns than its work
  needs.
- Or switch the request off for that run with
  `CONTEXT_NATIVE_RESULT_REQUEST=off` in its environment. Its results then
  reach memory only through `context.py record-result`, the MCP
  `memory_record_result` tool or the Harness.

## Promotion Review or Apply Fails

### Symptom

A promotion cannot be reviewed/applied, remains unapplied, or validation fails
after an attempted apply.

### Diagnose

Confirm:

- the proposal exists under `project-brain/control/promotions/`;
- source IDs, paths, types, and revisions still match;
- `review_mode` matches the configured path:
  - `human` requires an independent recorded reviewer and an approved outcome;
  - `automatic` requires `reviewer: null`, outcome
    `approved-without-review`, and an eligible verified terminal source;
- `promote-review` is not being used on an automatic proposal, because the
  runtime deliberately refuses to attach a human signature after the fact;
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
problem through supported operations, and retry only when the recorded mode
still applies to the current content. For reviewed mode, reconfirm the human
approval; for automatic mode, reconfirm eligibility and do not invent a
reviewer.

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
- A merge of two branches that both added records can leave conflict markers,
  or one side's list, in `project-brain/indexes/*.json`. Those files are
  derived from the records: resolve the Git conflict either way, and the next
  Brain write (a checkpoint, `update`, `record-result`) rebuilds them;
  `context.py compact` rebuilds them too, even with nothing to archive.
  Compaction no longer refuses because of a stale or conflicted index, only
  because of an invalid record.
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
7. Check that hook commands reach the script from any directory. Claude
   commands read `"${CLAUDE_PROJECT_DIR}"/.claude/hooks/<script>.sh`, and
   Codex commands use the `sh -c` launcher that walks up from the session's
   directory to the project holding `.codex/hooks.json` and runs its
   `.codex/hooks/<script>.sh` (a missing script there exits 127 and names
   the path); Cursor keeps bare `.cursor/hooks/<script>.sh`
   because it runs project hooks from the project root. A bare relative path
   on Claude Code or Codex exits 127 once the session is in a subdirectory,
   and both hosts treat that as non-blocking: the guard silently stops. An
   `echo '$TOOL_INPUT' | <script>` wrapper passes the literal variable name
   instead of the payload, and the hook then exits 0 on every call.
8. Check return-code semantics in the edition hook README.
9. Memory hooks that stay silent - no capsule on any prompt, no checkpoint in
   `memory-bank/local/last-turn-report.json` - while `context.py status`
   works: check that Python 3.9+ runs in the shell the host starts hooks with
   (Git Bash on native Windows). The hooks take `python3`; on native Windows,
   where the python.org installer provides `python` and `py` but no
   `python3` and the `python3` found may be the Microsoft Store stub, they try
   `python`, `py -3` and `python3`, in that order, and take the first that is
   Python 3.9+. With none of them the hooks exit quietly, and the session
   banner says `Context governance: unavailable - no Python 3.9+ found`. The
   name found is remembered in `ai-accelerator-python-<uid>` in the temporary
   directory and used while the shell finds it at the same path; delete that
   file to make the hooks look again.
   Outside Windows a `python3` on `PATH` is taken without a check, so one that
   is not Python makes every prompt report `Memory refresh unavailable`.
   Where Python has another name, the session banner says which, and the
   agent runs the memory CLI with it: Claude Code allows the CLI under
   `python3`, `python` and `py -3`, the Cursor CLI under `python3` and
   `python`.
10. Automatic native saves that never land - no `learning-*` record after a
    session: read `memory-bank/local/native-results.ndjson`, one line per
    prompt, Stop and save with a fixed code. `stop-hook` `exited` with
    `status` 124 is a save request killed at its budget; `stop-hook`
    `skipped` `capsule-delivered` a Harness turn; `stop` `failed` `unsaved`
    a request nobody answered, `host-refused` one Claude Code refused (the
    session is then not asked again for a day); `save` `refused` carries the
    refusal's code
    (`memory-result-v1:learning_fields` is usually `verified` put inside a
    learning). From the repository, `python3 scripts/memory_eval.py realized
    --project DIR` counts the lines per host.
11. Hooks that run in one Claude Code session and not in another - no
    capsule, no save, no `bash-validator.sh` - or files the project denies
    read in one, while no hook reports an error: check the folder the session
    started in. Claude Code reads `.claude/settings.json`, with the hooks,
    the permission rules and `env`, only from the folder it starts in. For a
    folder below the root a dispatcher in your own Claude Code configuration
    gives the session all of them; the installer run in a terminal and a
    session in the root set it up: `~/.claude/ai-accelerator/projects` (or
    the same under `$CLAUDE_CONFIG_DIR`) lists the project root and
    `~/.claude/settings.json` holds `ai-accelerator-dispatch.sh` entries,
    `permissions` among them. If they are missing, start one session in the
    root. Nothing is set up while `~/.claude/ai-accelerator/off` exists
    (on Windows the dispatcher runs under Git Bash, as Claude Code's hooks
    do), and a folder of another Git checkout inside the project - a
    worktree, a vendored repository - is that checkout's. To remove the
    dispatcher, create that `off` file and delete the entries from
    `settings.json`. Codex started in a subdirectory finds and runs the
    project's hooks; its sandbox writes only below that folder, so the save
    waits in a temporary folder (`ai-accelerator-native-spool-<uid>`) and the
    next hook saves it (journal `saved` with code `spooled`: the sandbox
    cannot write the journal either).

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
