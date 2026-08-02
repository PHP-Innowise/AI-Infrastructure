# Cursor Hooks

These hooks are registered in `.cursor/hooks.json` (schema `version: 1`). Each is a command hook: it receives JSON on stdin and signals via exit code (`0` = allow/continue, `2` = block). Cursor watches `hooks.json` and reloads on save.

## Active Hooks

### sessionStart: Local Context Scanner
**Script:** `local-context.sh`
**Purpose:** Prints project metadata at session start: git branch, Composer/PHP/tooling markers, native-PHP framework detection/structure, governed or lightweight mode, index health/staleness, active binding count, Project Brain validation status, and Memory Bank validation summary. It never runs indexing or retrieval and never prints/injects record contents.
**Return:** Always `0` (informational only).


### stop: Working-Memory Write
**Script:** `working-memory-write.sh`
**Purpose:** Buffers this turn's change set and flushes it to the authoritative task on a boundary. Reads Git porcelain metadata only; sensitive-looking paths and the runtime's own churn are excluded. The first flush provisions the task if it does not exist, so no manual `start` is required; an existing task is never overwritten.
**Return:** Always `0` (a failed checkpoint must never surface as a turn error).
**Flush boundary:** `CONTEXT_FLUSH_AFTER` turns, default 5.

### beforeShellExecution: Bash Validator
**Script:** `bash-validator.sh`
**Purpose:** Blocks destructive shell commands: force-push, hard reset, database drops/truncates, destructive migration resets/rollbacks, secret-writing Composer config, and `--no-verify`.
**Input key:** `.command` (Cursor supplies the full command string).
**Return:** `0` = safe, `2` = block.

### afterFileEdit: File Naming Validator
**Script:** `file-naming-validator.sh`
**Purpose:** Flags `.md` files in `tasks/` and `specs/` that do not follow the skill-prefix naming convention.
**Return:** `0` = valid, `1` = warning (currently active).
**Allowlist:** `README.md`, `CHANGELOG.md`, `MANIFEST.md`.

### afterFileEdit: Loop Detection
**Script:** `loop-detection.sh`
**Purpose:** Tracks edit count per file to detect doom loops.
**Return:** `0` = normal, `1` = warning at 7 edits, `2` = block at 10 edits.
**Tracking:** `/tmp/cursor-loop-detection/` (resets on reboot).

## Claude Code -> Cursor Event Mapping

| Claude Code (`.claude/settings.json`) | Cursor (`.cursor/hooks.json`) |
|---|---|
| `SessionStart` | `sessionStart` |
| `UserPromptSubmit` | **No equivalent** - see below |
| `Stop` | `stop` |
| `PreToolUse` matcher `Bash` | `beforeShellExecution` |
| `PreToolUse` matcher `Write\|Edit` | `afterFileEdit` (post-edit; warns rather than blocks pre-write) |
| `PostToolUse` matcher `Edit` | `afterFileEdit` |
| `Notification` | No direct equivalent (closest: `permission: "ask"` on `beforeShellExecution`) |

Notes:
- Cursor `timeout` is in **seconds** (Claude Code used milliseconds).
- Cursor matchers use JavaScript regex, not POSIX; these hooks self-filter in-script, so no matcher is set.
- To fail-closed on hook crash/timeout, add `"failClosed": true` to a hook entry.

## References

- Cursor Hooks: https://cursor.com/docs/hooks

## Why there is no Task Capsule on Cursor

The read half of automatic memory is not available on this tool, and no
workaround in this repository can supply it. Cursor's nearest event,
`beforeSubmitPrompt`, returns `{"continue": true|false, "user_message": "..."}`:
it can allow or block a submission, but it cannot add context to the prompt.
Emitting a capsule from it would produce output the client discards.

`working-memory-read.sh` is therefore absent from `.cursor/hooks/` rather than
present and unwired - a script that can never run is worse than no script,
because it reads as an installation fault. The write half is unaffected:
`stop` runs at turn end and needs no prompt access, so continuity is recorded
on Cursor exactly as it is elsewhere. What is lost is retrieval into the
prompt, not the record of what happened.

Explicit retrieval still works on Cursor: run `context.py retrieve` (or the
`memory` command) when a task needs prior context.
