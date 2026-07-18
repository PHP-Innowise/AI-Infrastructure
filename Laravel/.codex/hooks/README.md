# Codex Hooks

Registered in `.codex/hooks.json` and enabled by `features.hooks = true` in `.codex/config.toml`. Codex lifecycle hooks use the **same event names and command-hook contract as Claude Code** (JSON on stdin, exit code `0` = allow, `2` = block), so these scripts port directly.

Project-scoped hooks load only when the project is **trusted**.

## Active Hooks

| Event | Script | Purpose | Exit codes |
|---|---|---|---|
| `SessionStart` | `local-context.sh` | Print project context (git, Composer, PHP version, tooling, structure). | always `0` |
| `PreToolUse` | `bash-validator.sh` | Block destructive shell commands (force-push, hard reset, DB drops/truncates, destructive migration resets, secret-writing Composer config, `--no-verify`). | `0` allow / `2` block |
| `PreToolUse` | `file-naming-validator.sh` | Flag `.md` files in `tasks/`/`specs/` that break the skill-prefix naming convention. | `0` / `1` warn |
| `PostToolUse` | `loop-detection.sh` | Track edit count per file to detect doom loops. | `0` / `1` warn / `2` block |

No `matcher` is set on any group: each script self-filters on its input (command string or file path), so it is safe to run on every tool call and returns `0` when not applicable.

## Supported events (Codex)

`PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SessionStart`, `SubagentStart`, `SubagentStop`, `UserPromptSubmit`, `Stop`. Only command handlers run today; prompt/agent handlers are parsed but skipped.

## Tuning notes

- Codex tool identifiers and payload keys may differ from Claude Code. The scripts read `command` and `file_path`/`path` from the stdin JSON and fail open (exit `0`) when a key is absent, so a mismatch degrades safely to "no-op" rather than breaking a turn. Adjust the key extraction in a script if your Codex version names fields differently.
- To hard-fail on a crashing hook, wrap the logic to `exit 2` on error.

## Reference

- Codex configuration & hooks: https://developers.openai.com/codex/config-reference
