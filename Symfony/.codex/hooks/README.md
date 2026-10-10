# Codex Hooks

Registered in `.codex/hooks.json` and enabled by `features.hooks = true` in `.codex/config.toml`. Codex lifecycle hooks use the **same event names and command-hook contract as Claude Code** (JSON on stdin, exit code `0` = allow, `2` = block), so these scripts port directly.

Project-scoped hooks load only when the project is **trusted**.

## Active Hooks

| Event | Script | Purpose | Exit codes |
|---|---|---|---|
| `SessionStart` | `local-context.sh` | Print metadata-only mode, index health/staleness, active binding count, and validation summaries; never index, retrieve, print, or inject records. | always `0` |
| `UserPromptSubmit` | `working-memory-read.sh` | Re-index procedural, semantic, and episodic memory in one incremental pass, report each layer, and emit a bounded Task Capsule; uses `--ephemeral` so per-request manifests stay out of shared history. | always `0` |
| `Stop` | `working-memory-write.sh` | Buffer the turn's change set from Git porcelain metadata and flush a consolidated update on a boundary, provisioning the task on first flush; sensitive paths and runtime churn are excluded. | always `0` |
| `PreToolUse` | `bash-validator.sh` | Block destructive and secret-exposing shell commands (force push, hard reset, hook bypass, broad `rm -r`, destructive SQL, `gh` deletions, Composer auth tokens, printing `.env`, and this edition's framework commands - listed in `BV_FRAMEWORK_RULES` at the end of `bash-validator.sh` in this directory). Chains, `$(...)`, `sh -c`, wrappers and console abbreviations are parsed; it is a guard against accidental destruction, not a sandbox. The same command a sixth time in a session warns and a twelfth time blocks (the edit counter cannot see a command loop). | `0` allow / `1` warn / `2` block |
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
- Codex hooks (working directory, trust review): https://developers.openai.com/codex/hooks

## Wiring

Every command in `.codex/hooks.json` is the same root-finding launcher; only the trailing script name changes:

```sh
sh -c 'd=$(pwd); until [ -f "$d/.codex/hooks.json" ]; do [ -n "$d" ] || { echo "$1: no .codex/hooks.json at or above the working directory" >&2; exit 127; }; d=${d%/*}; done; exec "$d/.codex/hooks/$1"' sh bash-validator.sh
```

Codex runs hooks in the session's working directory through the user's login shell (`$SHELL -lc`) and exports no project-root variable, so a bare `.codex/hooks/<script>.sh` exits 127 whenever Codex starts in a subdirectory - a non-blocking failure that silently disables the guard. The launcher walks from that directory towards `/` to the nearest directory holding `.codex/hooks.json` - the project that declared the hook - and runs only that project's `.codex/hooks/<script>.sh`, so it works from any subdirectory, without Git, and when the project sits inside a larger repository. It never looks past that project: a missing script exits 127 with the path it expected, instead of running a same-named script from an ancestor directory such as `~/.codex/hooks/` (Codex's own user-hook directory). Handing the walk to `sh` keeps it independent of the login shell (bash, zsh, fish). Codex trusts each hook by the hash of its definition: after an update that changes these commands, the changed hooks stay skipped until you review and trust them again in `/hooks`.

`working-memory-read.sh` and `working-memory-write.sh` are wired here to
`UserPromptSubmit` and `Stop`, the same events they use under Claude Code.
Both names are listed in the Codex configuration reference (see above), which
is what makes the wiring safe: an unverified event name silently produces a
hook that never runs.

The read hook exits without a capsule when `CONTEXT_CAPSULE_DELIVERED=1`: a
host that already put this turn's capsule into the prompt sets it (the Harness
does), and the Stop hook still checkpoints the task.

The read hook carries `additionalContextLimit: 4000`. The capsule it prints is
capped at 3,600 characters (the JSON packet at 8,000), and the 2,500-token
default could still truncate one dense with code mid-capsule. Lower it if your Codex version accounts
tokens differently than this estimate.

Codex tool identifiers and payload keys may still differ from Claude Code. Both
scripts fail open (`exit 0`) when a key is missing, so a payload mismatch
degrades to "no capsule this turn" rather than a broken turn. If the capsule
never appears, check the payload keys before assuming the wiring is wrong.

## Chat snapshots and merges

`context-continuity.sh` is wired on `SessionStart` (with
`additionalContextLimit: 6000`), `UserPromptSubmit` and `Stop` through the same
root-finding launcher as the other hooks; the payload's `hook_event_name`
selects the action. A prompt (`prompt`) or a final answer
(`last_assistant_message`, or the visible transcript the hook names) is
appended to a per-chat snapshot under the ignored `.context-handoff/` - in an
attached session, under the launcher's state directory instead. At
`SessionStart` a merge prepared with `context-load merge` for Codex is
delivered to a new session as additional context: at most 6,000 bytes,
pointing at the full archive under `.context-handoff/merges/`. A merged task
gets the same sources again when it resumes or compacts. Nothing else is
restored: a new session on the branch starts with the Task Capsule from
`working-memory-read.sh`.

Snapshots never reach Project Brain, Memory Bank or the local index, and a
recognised secret refuses the capture. Earlier conversation text is untrusted
background and carries no approval. Both halves fail open under
`CONTEXT_HOOK_BUDGET` (default 5 seconds). `CONTEXT_CONTINUITY_DISABLED=1`
turns them off; `CONTEXT_CONTINUITY_RESTORE_DISABLED=1`, which the Harness sets
for its own merged tasks, skips only the delivery. Storage is bounded: eight
chats per branch and 64 per checkout, none kept 30 days past its last capture,
4 MiB of visible text per chat, and 128 merge archives / 256 MiB per checkout
with 32 MiB per archive; a full store refuses a new merge and never blocks a
turn. The store writes its own `.gitignore`, so it stays out of Git even where
the project's `.gitignore` does not name it. The repository's context-handoff guide lists the
limits of each client.
