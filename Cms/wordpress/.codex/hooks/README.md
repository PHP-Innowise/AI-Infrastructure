# Codex Hooks

Registered in `.codex/hooks.json` and enabled by `features.hooks = true` in `.codex/config.toml`. Codex lifecycle hooks use the **same event names and command-hook contract as Claude Code** (JSON on stdin, exit code `0` = allow, `2` = block), so these scripts port directly.

Project-scoped hooks load only when the project is **trusted**.

## Active Hooks

| Event | Script | Purpose | Exit codes |
|---|---|---|---|
| `SessionStart` | `local-context.sh` | Print metadata-only mode, index health/staleness, active binding count, and validation summaries; never index, retrieve, print, or inject records. | always `0` |
| `UserPromptSubmit` | `working-memory-read.sh` | Re-index procedural, semantic, and episodic memory in one incremental pass, report each layer, and emit a bounded Task Capsule; uses `--ephemeral` so per-request manifests stay out of shared history. | always `0` |
| `Stop` | `working-memory-write.sh` | Buffer the turn's change set from Git porcelain metadata and flush a consolidated update on a boundary, provisioning the task on first flush; sensitive paths and runtime churn are excluded. | always `0` |
| `PreToolUse` | `bash-validator.sh` | Block destructive shell commands (force-push, hard reset, DB drops/truncates, destructive migration resets, secret-writing Composer config, `--no-verify`). | `0` allow / `2` block |
| `PreToolUse` | `file-naming-validator.sh` | Flag `.md` files in `tasks/`/`specs/` that break the skill-prefix naming convention. | `0` / `1` warn |
| `PreToolUse` | `subagent-gate.sh` | Block Codex's built-in multi-agent tools (`spawn_agent` family); version-proof backstop for the `[features] multi_agent` and `[agents] enabled` switches in `config.toml`. | `0` allow / `2` block |
| `PostToolUse` | `loop-detection.sh` | Track edit count per file to detect doom loops. | `0` / `1` warn / `2` block |

No `matcher` is set on any group: each script self-filters on its input before doing any real work — `bash-validator.sh` exits unless the payload carries a `command`/`cmd` key, and `file-naming-validator.sh` and `loop-detection.sh` exit unless `tool_name` is a file-editing tool (or is absent) — so it is safe to run on every tool call and returns `0` when not applicable.


`subagent-dispatch.sh` exists here as a byte-identical mirror of the canonical hook but stays unregistered: multi-agent is disabled in this edition's `config.toml`, so there are no subagent stops to observe.
## Supported events (Codex)

`PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SessionStart`, `SubagentStart`, `SubagentStop`, `UserPromptSubmit`, `Stop`. Only command handlers run today; prompt/agent handlers are parsed but skipped.

## Tuning notes

- Codex tool identifiers and payload keys may differ from Claude Code. The scripts read `command` and `file_path`/`path` from the stdin JSON and fail open (exit `0`) when a key is absent, so a mismatch degrades safely to "no-op" rather than breaking a turn. Adjust the key extraction in a script if your Codex version names fields differently.
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
