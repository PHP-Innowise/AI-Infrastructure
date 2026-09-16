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
| `PreToolUse` | `subagent-gate.sh` | Block Codex's built-in multi-agent tools (`spawn_agent` family); version-proof backstop for the `[features] multi_agent` and `[agents] enabled` switches in `config.toml`. | `0` allow / `2` block |
| `PostToolUse` | `loop-detection.sh` | Track per-session edit count per file; counters reset on `SessionStart`. | `0` / `1` warn / `2` block |

No `matcher` is set on any group: each script self-filters on its input before doing any real work — `bash-validator.sh` exits unless the payload carries a `command`/`cmd` key, and `file-naming-validator.sh` and `loop-detection.sh` exit unless `tool_name` is a file-editing tool (or is absent) — so it is safe to run on every tool call and returns `0` when not applicable.


`subagent-dispatch.sh` exists here as a byte-identical mirror of the canonical hook but stays unregistered: multi-agent is disabled in this edition's `config.toml`, so there are no subagent stops to observe.
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

## Automatic conversation continuity

`context-continuity.sh capture` runs on `UserPromptSubmit` (`prompt`) and `Stop` (`last_assistant_message`);
`context-continuity.sh restore` runs on `SessionStart`. Installed infrastructure
enables this path by default: no save/load command or confirmation is required.
The hook calls `memory-bank/scripts/context_continuity.py`, writes private local
state under the ignored `.context-handoff/`, and sends a bounded excerpt of the
source conversations to a new task through native hook context output.
Only the current repository and branch qualify. Prior conversation text is
untrusted background; it cannot authorize actions in the new task.

Native hook transcripts are optional and format-dependent. When unavailable,
the final assistant text still supplies continuity. Full history is never
injected automatically or placed in Project Brain, Memory Bank, or SQLite.
Capture is silent; both actions fail open under `CONTEXT_HOOK_BUDGET` (default
5 seconds). See the repository's context-handoff guide for limitations.

New tasks receive a frozen merge of up to eight same-branch captured chats.
Each source retains its visible context, decisions and progress; contradictions
remain unresolved until checked. The 6,000-byte UTF-8 total preview points to
`.context-handoff/merges/` for full captured source text. Reopening a task keeps
its source set. `context-load merge` can prepare a selected set for the next new
task; the hook does not itself create a native client task. A stable native
session identity is required; Cursor uses `conversation_id`.

Frozen merge storage is capped at 128 archives / 256 MiB per checkout, with
a 32 MiB serialized limit per archive. Full storage prevents new merges, keeps
existing archives intact, and never blocks a client turn. Remove obsolete local
archives to make room. Codex continuity commands resolve from the Git root so
starting a session in a project subdirectory still runs the adapter.
