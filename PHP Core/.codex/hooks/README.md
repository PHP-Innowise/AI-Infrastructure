# Codex Hooks

Registered in `.codex/hooks.json` and enabled by `features.hooks = true` in `.codex/config.toml`. Codex lifecycle hooks use the **same event names and command-hook contract as Claude Code** (JSON on stdin, exit code `0` = allow, `2` = block), so these scripts port directly.

Project-scoped hooks load only when the project is **trusted**.

## Active Hooks

| Event | Script | Purpose | Exit codes |
|---|---|---|---|
| `SessionStart` | `local-context.sh` | Print metadata-only mode, index health/staleness, active binding count, and validation summaries; never index, retrieve, print, or inject records. | always `0` |
| `UserPromptSubmit` | `working-memory-read.sh` | Re-index procedural, semantic, and episodic memory in one incremental pass, report each layer, and emit a bounded Task Capsule; uses `--ephemeral` so per-request manifests stay out of shared history. | always `0` |
| `Stop` | `working-memory-write.sh` | Buffer the turn's change set from Git porcelain metadata and flush a consolidated update on a boundary, provisioning the task on first flush; sensitive paths and runtime churn are excluded. | always `0` |
| `PreToolUse` | `bash-validator.sh` | Block destructive and secret-exposing shell commands (force push, hard reset, hook bypass, broad `rm -r`, destructive SQL, `gh` deletions, Composer auth tokens, printing `.env`, and this edition's framework commands - listed in `BV_FRAMEWORK_RULES` at the end of `bash-validator.sh` in this directory). Chains, `$(...)`, `sh -c`, wrappers and console abbreviations are parsed; it is a guard against accidental destruction, not a sandbox. The same command a sixth time in a session (the payload's `session_id`) warns and a twelfth time blocks (the edit counter cannot see a command loop); a file edit, `apply_patch` included, that `loop-detection.sh` recorded since the command last ran starts its count over, and a read-only status query standing alone (`gh pr checks`, `gh run view`, `git status`, `docker ps/logs`, `kubectl get/logs`, `tail`, ...) is polling and is not counted. Counts live in the private `${TMPDIR:-/tmp}/codex-loop-detection-<uid>-<repo-key>/`; a linked or foreign directory turns the guard off. | `0` allow (from the sixth identical run with the warning as JSON `additionalContext`, which Codex adds to the model's context) / `2` block |
| `PreToolUse` | `file-naming-validator.sh` | Flag `.md` files in `tasks/`/`specs/` that break the skill-prefix naming convention. | `0` / `1` warn |
| `PreToolUse` | `subagent-gate.sh` | Block Codex's built-in multi-agent tools (`spawn_agent` family); version-proof backstop for the `[features] multi_agent` and `[agents] enabled` switches in `config.toml`. | `0` allow / `2` block |
| `PostToolUse` | `loop-detection.sh` | Track edit count per file per session; an `apply_patch` call counts every file its `*** Add File:`/`*** Update File:`/`*** Move to:` lines name. Each recorded edit restarts the Bash Validator's repetition count for the commands run before it. A session start clears that session's counters and any left untouched for a day. | `0` (from the seventh edit with the warning as JSON `additionalContext`) / `2` block |

No `matcher` is set on any group: each script self-filters on its input before doing any real work — `bash-validator.sh` exits unless the payload carries a `command`/`cmd` key, and `file-naming-validator.sh` and `loop-detection.sh` exit unless `tool_name` is a file-editing tool (or is absent) — so it is safe to run on every tool call and returns `0` when not applicable.


**In a worktree:** eligible linked worktrees without their own installation use the native dispatcher. `local-context.sh` requests registration in the matching root session; explicit setup is `memory-bank/bin/infra-engine dispatch-install codex --root <project-root>`. The user folder holds the copied engine, `native-dispatch.json`, served roots and managed `hooks.json` entries. The terminal install contract registers the selected host when stdin and stderr are terminals and its configuration directory already exists. These entries serve the root's memory with the worktree's project and task. Codex runs them only after the person's hook review (`/hooks` or its trust dialog); setup writes no approval or trust hash. Existing hooks in `config.toml` and `ai-accelerator/off` suppress registration. Global Codex dispatcher registration is currently Unix-only; project-local native hooks are separate. See `docs/MEMORY-INTEGRATION.md` in the accelerator repository.

`subagent-dispatch.sh` exists here as a byte-identical mirror of the canonical hook but stays unregistered: multi-agent is disabled in this edition's `config.toml`, so there are no subagent stops to observe.
## Supported events (Codex)

`PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SessionStart`, `SubagentStart`, `SubagentStop`, `UserPromptSubmit`, `Stop`. Only command handlers run today; prompt/agent handlers are parsed but skipped.

## Tuning notes

- Codex tool identifiers and payload keys may differ from Claude Code. The scripts read `command` and `file_path`/`path` (for `apply_patch`, the file markers inside the patch text) from the stdin JSON and fail open (exit `0`) when a key is absent, so a mismatch degrades safely to "no-op" rather than breaking a turn. Adjust the key extraction in a script if your Codex version names fields differently.
- To hard-fail on a crashing hook, wrap the logic to `exit 2` on error.

## Reference

- Codex configuration & hooks: https://developers.openai.com/codex/config-reference
- Codex hooks (working directory, trust review): https://developers.openai.com/codex/hooks

## Wiring

The installer writes every command in `.codex/hooks.json` in one form; only the role changes, and `<prefix>` is the project's path below Git's top level with a trailing slash (`apps/shop/`), empty when the project is the top:

```sh
sh -c 'r="$(git rev-parse --show-toplevel 2>/dev/null || pwd)/<prefix>"; e="$r"memory-bank/bin/infra-engine; [ -x "$e" ] && exec "$e" hook codex bash-validator --root "$r"; exec "$r".codex/hooks/bash-validator.sh'
```

Codex runs hooks in the session's working directory through the user's login shell (`$SHELL -lc`) and exports no project-root variable, so a bare `.codex/hooks/<role>.sh` exits 127 whenever Codex starts in a subdirectory - a non-blocking failure that silently disables the guard. The command finds the project from Git's top level instead, so it works from any subdirectory and when the project sits inside a larger repository; outside Git the session's directory stands in for the top level, so there Codex must start in the project root. Handing the command to `sh` keeps it independent of the login shell (bash, zsh, fish).

The checkout's engine, `memory-bank/bin/infra-engine`, starts directly, and the role's launcher, `.codex/hooks/<role>.sh`, runs only when that engine is absent. A linked worktree has the committed hooks but no engine of its own, since Git ignores it: there the launcher runs the main checkout's engine. With no engine installed, `bash-validator.sh` refuses every command with an install hint, `local-context.sh` reports at session start that the hooks and memory are off, and the other hooks pass the turn. Native Windows installations bind the absolute engine and root of the machine that installed them instead.

Codex trusts each hook by the hash of its definition: after an update that changes these commands, the changed hooks stay skipped until you review and trust them again in `/hooks`. A sync therefore leaves an installed `.codex/hooks.json` as it is unless given `--rewire-codex` (with `--update-tracked` once the file is committed); until then the commands of an earlier installation, which run the launcher alone, keep working.

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
command as the other hooks; the payload's `hook_event_name` selects the
action. A prompt (`prompt`) or a final answer
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

### Automatic structured result capture

In governed native sessions the prompt hook arms a metadata-only challenge. At Stop, after the Git checkpoint, the same agent receives at most one internal continuation to submit progress, optional next steps and up to three checked source-backed learnings via `memory-bank/bin/infra-engine memory native-result`. A clean research turn needs no Git delta or manual memory command. Empty learnings and an explicit skip are valid; no transcript or final prose is automatically adopted as knowledge. Task provisioning occurs only after a valid structured result; agent attestation never becomes person review or task completion.

Pending captures pin project, actor, session, task revision, checkout and runtime. Conflicts are held; repeated saves use the same result ID. Recovery does not checkpoint the user turn twice. A failed or uncertain save is reported in the next capsule. Harness/recovery and lightweight sessions do not arm this path. Local challenges expire for save after 24 hours and are bounded to 128 sessions; a new prompt always starts fresh. Cursor's Stop entry uses `loop_limit: 1`.

Every prompt, Stop and save also appends one metadata line to ignored `memory-bank/local/native-results.ndjson` - the host, the step and a fixed code such as `memory-result-v1:unverified`, never content, paths or IDs - and the Stop hook adds its save request's exit status, 124 when the budget killed it. How many saves were requested, made or refused, and why, is then counted from disk. The journal keeps eight lines per `refresh_health_retention` record.
