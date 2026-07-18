# Cursor Hooks

These hooks are registered in `.cursor/hooks.json` (schema `version: 1`). Each is a command hook: it receives JSON on stdin and signals via exit code (`0` = allow/continue, `2` = block). Cursor watches `hooks.json` and reloads on save.

## Active Hooks

### sessionStart: Local Context Scanner
**Script:** `local-context.sh`
**Purpose:** Prints project context at session start: git branch, Composer markers, PHP version, configured test/format/static-analysis tools, framework detection, and project structure.
**Return:** Always `0` (informational only).

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

| Claude Code (`.cursor/settings.json`) | Cursor (`.cursor/hooks.json`) |
|---|---|
| `SessionStart` | `sessionStart` |
| `PreToolUse` matcher `Bash` | `beforeShellExecution` |
| `PreToolUse` matcher `Write\|Edit` | `afterFileEdit` (post-edit; warns rather than blocks pre-write) |
| `PostToolUse` matcher `Edit` | `afterFileEdit` |
| `Notification` | No direct equivalent (closest: `permission: "ask"` on `beforeShellExecution`) |

Notes:
- Cursor `timeout` is in **seconds** (Claude Code used milliseconds).
- Cursor matchers use JavaScript regex, not POSIX; these hooks self-filter in-script, so no matcher is set.
- To fail-closed on hook crash/timeout, add `"failClosed": true` to a hook entry.

## References

- Cursor Hooks: https://docs.cursor.com/agent/hooks
