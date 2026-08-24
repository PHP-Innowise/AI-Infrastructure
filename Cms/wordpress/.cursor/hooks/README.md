# Cursor Hooks

These hooks are registered in `.cursor/hooks.json` (schema `version: 1`). Each is a command hook: it receives JSON on stdin and signals via exit code (`0` = allow/continue, `2` = block). Cursor watches `hooks.json` and reloads on save.

## Active Hooks

### sessionStart: Local Context Scanner
**Script:** `local-context.sh`
**Purpose:** Prints project metadata at session start: git branch, Composer/PHP/tooling markers, native-PHP framework detection/structure, governed or lightweight mode, index health/staleness, active binding count, Project Brain validation status, and Memory Bank validation summary. The banner never prints/injects record contents; this Cursor copy additionally re-renders `.cursor/rules/working-memory.mdc` silently (see the capsule section below).
**Return:** Always `0` (informational only).


### stop: Working-Memory Write
**Script:** `working-memory-write.sh`
**Purpose:** Buffers this turn's change set and flushes it to the authoritative task on a boundary. Reads Git porcelain metadata only; sensitive-looking paths and the runtime's own churn are excluded. The first flush provisions the task if it does not exist, so no manual `start` is required; an existing task is never overwritten. After the checkpoint, this Cursor copy renders the freshest Task Capsule into `.cursor/rules/working-memory.mdc` (see the capsule section below).
**Return:** Always `0` (a failed checkpoint must never surface as a turn error).
**Flush boundary:** `CONTEXT_FLUSH_AFTER` turns, default 5.

### beforeShellExecution: Bash Validator
**Script:** `bash-validator.sh`
**Purpose:** Blocks destructive shell commands: force-push, hard reset, database drops/truncates, destructive migration resets/rollbacks, secret-writing Composer config, and `--no-verify`.
**Input key:** `.command` (Cursor supplies the full command string).
**Return:** `0` = safe, `2` = block.

### subagentStart: Subagent Gate
**Script:** `subagent-gate.sh`
**Purpose:** Allows only subagents defined in `.cursor/agents/` (frontmatter `name:`); Cursor's built-in subagents (explore, bash/shell, browser, general-purpose) are denied. Cursor has no setting that disables its built-ins, so this hook is the enforcement point; `.cursor/rules/subagent-policy.mdc` is the steering layer on top.
**Contract:** JSON on stdout - `{"permission": "allow"}` or `{"permission": "deny", "user_message": "..."}` - always exit `0`; registered with `failClosed: true`.
**Serialization:** agents marked `writes: true` in their frontmatter run one at a time - the gate takes `/tmp/cursor-write-agent-lock-<repo-key>` (TTL 30 min, override with `SUBAGENT_WRITE_LOCK_TTL_MINUTES`), released by the completion observer or by expiry.
**Limits:** the lock is advisory and machine-local (`/tmp`, keyed by the repository path), so two containers or two machines working the same branch are not serialized against each other. It covers subagent spawns only - the main conversation's own edits are not gated, and nothing running outside the tool is. It fails open with no JSON extractor (`jq`/`php`/`python3`), with an unreadable agent roster, and - for the check-and-take race only - without `flock`. Past the TTL the holder stops blocking, so a run longer than `SUBAGENT_WRITE_LOCK_TTL_MINUTES` can be joined by a second writer.
**Caveat:** before Cursor 3.4 the payload reported every subagent as `general-purpose`, which would deny project agents too; on such versions remove this entry from `hooks.json` and rely on the rule.

### subagentStop: Subagent Dispatch Observer
**Script:** `subagent-dispatch.sh`
**Purpose:** Records each subagent completion in the task's agent channel (`msg-dispatch --event complete`; uses `subagent_type` and `status` - the documented `summary` field is unreliable in current Cursor builds) and releases the write-agent lock the gate took for a `writes: true` agent. Degrades to a no-op without python3 or the context runtime.
**Return:** Always exit 0, no JSON output (observation only)

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
| `UserPromptSubmit` | **No equivalent** - the capsule is rendered into an `alwaysApply` rule instead; see below |
| `Stop` | `stop` |
| `PreToolUse` matcher `Bash` | `beforeShellExecution` |
| `PreToolUse` matcher `Write\|Edit` | `afterFileEdit` (post-edit; warns rather than blocks pre-write) |
| `PostToolUse` matcher `Edit` | `afterFileEdit` |
| `PreToolUse` matcher `Agent\|Task` | `subagentStart` |
| `SubagentStop` | `subagentStop` |
| `Notification` | No direct equivalent (closest: `permission: "ask"` on `beforeShellExecution`) |

Notes:
- Cursor `timeout` is in **seconds** (Claude Code used milliseconds).
- Cursor matchers use JavaScript regex, not POSIX; these hooks self-filter in-script, so no matcher is set.
- To fail-closed on hook crash/timeout, add `"failClosed": true` to a hook entry.

## References

- Cursor Hooks: https://cursor.com/docs/hooks

## How the Task Capsule reaches Cursor

The read half of automatic memory cannot run at prompt time on this tool.
Cursor's nearest event, `beforeSubmitPrompt`, returns
`{"continue": true|false, "user_message": "..."}`: it can allow or block a
submission, but it cannot add context to the prompt. Emitting a capsule from
it would produce output the client discards, so `working-memory-read.sh` is
absent from `.cursor/hooks/` rather than present and unwired.

The capsule arrives through a rule file instead. The Cursor copies of
`working-memory-write.sh` (after the turn checkpoint) and `local-context.sh`
(at session start, so a fresh session or a branch switch never serves the
previous session's capsule) render the freshest capsule into
`.cursor/rules/working-memory.mdc`, an `alwaysApply` rule Cursor attaches to
every prompt. The rendered file is one turn stale by design and says so in
its header ("as of end of previous turn"); it is replaced atomically and only
when a fresh render succeeds, and it is ignored local state (this edition's
`.gitignore` lists it). This divergence from the canonical `.claude/hooks`
scripts is a declared MIRROR_RULES transformation (the `_WM_DELIVERY_*`
constants in `memory-bank/scripts/context_retrieval.py`), verified by
`scripts/build_mirrors.py --check` - not drift.

Explicit retrieval still works on Cursor: run `context.py retrieve` (or the
`memory` command) when a task needs sharper context than the rendered rule.
