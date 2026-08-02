# Cursor Hooks

These hooks are registered in `.cursor/hooks.json` (schema `version: 1`). Each is a command hook: it receives JSON on stdin and signals via exit code (`0` = allow/continue, `2` = block). Cursor watches `hooks.json` and reloads on save.

## Active Hooks

### sessionStart: Local Context Scanner
**Script:** `local-context.sh`
**Purpose:** Prints project metadata at session start: git branch, Composer/PHP/tooling markers, framework/structure, governed or lightweight mode, index health/staleness, active binding count, Project Brain validation status, and Memory Bank validation summary. It never runs indexing or retrieval and never prints/injects record contents.
**Return:** Always `0` (informational only).

### beforeSubmitPrompt: Working-Memory Read
**Script:** `working-memory-read.sh`
**Purpose:** Runs `context.py refresh`, re-indexing procedural, semantic, and episodic memory in one incremental pass and reporting each layer as `updated` or `failed`. With a task — from `CONTEXT_TASK_ID` or the current branch — the same process also assembles a bounded Task Capsule with `--ephemeral`, keeping the per-request manifest in ignored local state rather than shared Git history.
**Return:** Always `0` (context tooling must never block a prompt).
**Budget:** `CONTEXT_HOOK_BUDGET` seconds, default 5.

### stop: Working-Memory Write
**Script:** `working-memory-write.sh`
**Purpose:** Buffers this turn's change set and flushes it to the authoritative task on a boundary. Reads Git porcelain metadata only; sensitive-looking paths and the runtime's own churn are excluded. The first flush provisions the task if it does not exist, so no manual `start` is required; an existing task is never overwritten.
**Return:** Always `0` (a failed checkpoint must never surface as a turn error).
**Flush boundary:** `CONTEXT_FLUSH_AFTER` turns, default 5.

### beforeShellExecution: Bash Validator
**Script:** `bash-validator.sh`
**Purpose:** Blocks destructive shell commands: force-push, hard reset, database/schema drops, unsafe down migrations, purging fixture loads, failed-message bulk removal, destructive SQL, secret-writing Composer config, and verification bypass.
**Input key:** `.command` (Cursor supplies the full command string).
**Return:** `0` = safe, `2` = block.

### afterFileEdit: File Naming Validator
**Script:** `file-naming-validator.sh`
**Purpose:** Enforces discovered skill prefixes, zero-padded `tasks/TASK-001/` directories, and `memory-bank/chunks/MEM-0001-short-slug.md` naming.
**Return:** `0` = valid/not applicable, `2` = violation. Because Cursor invokes this after an edit, a violation stops continuation and must be corrected; it cannot undo the completed write.
**Allowlist:** `README.md`, `CHANGELOG.md`, `MANIFEST.md`.

### afterFileEdit: Loop Detection
**Script:** `loop-detection.sh`
**Purpose:** Tracks edit count per file to detect doom loops.
**Return:** `0` = normal, `1` = warning at 7 edits, `2` = block at 10 edits.
**Tracking:** `/tmp/cursor-loop-detection/`, reset by `sessionStart`.

## Claude Code -> Cursor Event Mapping

| Claude Code (`.claude/settings.json`) | Cursor (`.cursor/hooks.json`) |
|---|---|
| `SessionStart` | `sessionStart` |
| `PreToolUse` matcher `Bash` | `beforeShellExecution` |
| `PreToolUse` matcher `Write\|Edit` | `afterFileEdit` (post-edit; reports a blocking violation that must be corrected) |
| `PostToolUse` matcher `Edit` | `afterFileEdit` |
| `Notification` | No direct equivalent (closest: `permission: "ask"` on `beforeShellExecution`) |

Notes:
- Cursor and current Claude Code hook `timeout` values are in **seconds**.
- Cursor matchers use JavaScript regex, not POSIX; these hooks self-filter in-script, so no matcher is set.
- To fail-closed on hook crash/timeout, add `"failClosed": true` to a hook entry.

## References

- Cursor Hooks: https://docs.cursor.com/agent/hooks

## Wiring

`working-memory-read.sh` and `working-memory-write.sh` are present here and
resolve their own edition directory, but they are wired to events only in
`.claude/settings.json`. The equivalent event names for this tool were not
verified, and wiring an unverified event name is worse than leaving the script
unwired. Add the entries once the names are confirmed for the installed
version.
