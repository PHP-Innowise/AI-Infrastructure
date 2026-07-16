# Claude Code Hooks Documentation

## Active Hooks

### SessionStart: Local Context Scanner
**Script:** `local-context.sh`
**Purpose:** Outputs project context at session start: git branch, Composer markers, PHP version, configured test/format/static-analysis tools, framework detection (with a note to use the matching branch), and project structure.
**Return:** Always 0 (informational only)

### PreToolUse (Write|Edit): File Naming Validator
**Script:** `file-naming-validator.sh`
**Purpose:** Enforces discovered skill prefixes and zero-padded `tasks/TASK-001/` directories for task/spec Markdown.
**Return:** 0 = valid/not applicable, 2 = block
**Allowlist:** README.md, CHANGELOG.md, MANIFEST.md

### PreToolUse (Bash): Bash Validator
**Script:** `bash-validator.sh`
**Purpose:** Blocks destructive commands: force-push, hard reset, database/schema drops, unsafe down migrations, purging fixture loads, failed-message bulk removal, destructive SQL, secret-writing Composer config, and verification bypass.
**Return:** 0 = safe command, 2 = block

### PostToolUse (Edit): Loop Detection
**Script:** `loop-detection.sh`
**Purpose:** Tracks edit count per file per session. Detects doom loops.
**Return:** 0 = normal, 1 = warning at 7 edits, 2 = block at 10 edits
**Tracking:** Uses `/tmp/claude-loop-detection/` and resets at `SessionStart`.

### Notification: Desktop Alert
**Purpose:** Desktop notification when Claude needs user attention.
**Variants:** macOS (`osascript`), Linux (`notify-send`), shell fallback.

## Hook Return Codes

| Code | Meaning |
|------|---------|
| `0` | Success, continue |
| `1` | Warning, continue (logged) |
| `2` | Block operation (shows error) |

## Tuning Strategy

Safety hooks block only operations that are destructive, irreversible, or likely to expose secrets. If a command is blocked incorrectly, update the pattern after reviewing the exact command and risk.

## Hook Types

| Hook | When it fires |
|------|--------------|
| `SessionStart` | New Claude Code session |
| `Notification` | Claude needs user attention |
| `Stop` | Claude finishes responding |
| `PreToolUse` | Before a tool executes |
| `PostToolUse` | After a tool executes |

## Personal Hooks

Use `.claude/settings.local.json` for personal hooks that shouldn't be shared with the team.

## References

- [Claude Code Hooks Documentation](https://docs.anthropic.com/en/docs/claude-code/hooks)
