# Codex Hooks

Registered in `.codex/hooks.json` and enabled by `features.hooks = true` in `.codex/config.toml`. Codex lifecycle hooks use the **same event names and command-hook contract as Claude Code** (JSON on stdin, exit code `0` = allow, `2` = block), so these scripts port directly.

Project-scoped hooks load only when the project is **trusted**.

## Active Hooks

| Event | Script | Purpose | Exit codes |
|---|---|---|---|
| `SessionStart` | `local-context.sh` | Print metadata-only mode, index health/staleness, active binding count, and validation summaries; never index, retrieve, print, or inject records. | always `0` |
| `UserPromptSubmit` | `working-memory-read.sh` | Re-index procedural, semantic, and episodic memory in one incremental pass, report each layer, and emit a bounded Task Capsule; uses `--ephemeral` so per-request manifests stay out of shared history. | always `0` |
| `Stop` | `working-memory-write.sh` | Buffer the turn's change set from Git porcelain metadata and flush a consolidated update on a boundary, provisioning the task on first flush; sensitive paths and runtime churn are excluded. | always `0` |
| `PreToolUse` | `bash-validator.sh` | Block destructive shell commands (force-push, hard reset, database/schema drops, unsafe down migrations, purging fixtures, failed-message bulk removal, destructive SQL, and secret-writing Composer config). | `0` allow / `2` block |
| `PreToolUse` | `file-naming-validator.sh` | Block `.md` files that break skill-prefix, task-directory, or memory-chunk naming conventions. | `0` allow / `2` block |
| `PostToolUse` | `loop-detection.sh` | Track per-session edit count per file; counters reset on `SessionStart`. | `0` / `1` warn / `2` block |

No `matcher` is set on any group: each script self-filters on its input (command string or file path), so it is safe to run on every tool call and returns `0` when not applicable.

## Supported events (Codex)

`PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SessionStart`, `SubagentStart`, `SubagentStop`, `UserPromptSubmit`, `Stop`. Only command handlers run today; prompt/agent handlers are parsed but skipped.

## Tuning notes

- Codex tool identifiers and payload keys may differ from Claude Code. The scripts parse all nested `command`/`cmd` and `file_path`/`path` strings from stdin JSON and return `0` when no applicable key exists. Adjust extraction if a future Codex version changes its payload schema.
- To hard-fail on a crashing hook, wrap the logic to `exit 2` on error.

## Reference

- Codex configuration & hooks: https://developers.openai.com/codex/config-reference

## Wiring

`working-memory-read.sh` and `working-memory-write.sh` are wired here to
`UserPromptSubmit` and `Stop`, the same events they use under Claude Code.
Both names are listed in the Codex configuration reference (see above), which
is what makes the wiring safe: an unverified event name silently produces a
hook that never runs.

The read hook carries `additionalContextLimit: 4000`. A Task Capsule is capped
at 8,000 Unicode characters, which exceeds the 2,500-token default and would
otherwise be truncated mid-capsule. Lower it if your Codex version accounts
tokens differently than this estimate.

Codex tool identifiers and payload keys may still differ from Claude Code. Both
scripts fail open (`exit 0`) when a key is missing, so a payload mismatch
degrades to "no capsule this turn" rather than a broken turn. If the capsule
never appears, check the payload keys before assuming the wiring is wrong.
