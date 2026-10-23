# Cursor Hooks

These hooks are registered in `.cursor/hooks.json` (schema `version: 1`). Each is a command hook: it receives JSON on stdin and signals via exit code (`0` = allow/continue, `2` = block). Cursor watches `hooks.json` and reloads on save.

## Active Hooks

### sessionStart: Local Context Scanner
**Script:** `local-context.sh`
**Purpose:** Prints project metadata at session start: git branch, Composer/PHP/tooling markers, native-PHP framework detection/structure, governed or lightweight mode, index health/staleness, active binding count, Project Brain validation status, and Memory Bank validation summary. The banner never prints/injects record contents; this Cursor copy additionally re-renders `.cursor/rules/working-memory.mdc` silently (see the capsule section below).
**Return:** Always `0` (informational only).


### beforeSubmitPrompt: Working-Memory Read
**Script:** `working-memory-read.sh`
**Purpose:** Retrieves for the prompt being submitted (`refresh --host cursor --query <prompt> --sanitize --json`) and renders the capsule into `.cursor/rules/working-memory.mdc` before the request leaves (see the capsule section below). Writes no task state. Stands down when the host already put the capsule into the prompt (`CONTEXT_CAPSULE_DELIVERED=1`, the Harness).
**Return:** Always `0` with `{"continue": true}` on stdout - memory never blocks a prompt.

### stop: Working-Memory Write
**Script:** `working-memory-write.sh`
**Purpose:** Buffers this turn's change set and flushes it to the authoritative task on a boundary. Reads Git porcelain metadata only; sensitive-looking paths and the runtime's own churn are excluded. The first flush provisions the task if it does not exist, so no manual `start` is required; an existing task is never overwritten. After the checkpoint, this Cursor copy renders the branch's Task Capsule into `.cursor/rules/working-memory.mdc` unless the prompt hook rendered it for this task (see the capsule section below).
**Return:** Always `0` (a failed checkpoint must never surface as a turn error).
**Flush boundary:** `CONTEXT_FLUSH_AFTER` turns, default 5.

### beforeShellExecution: Bash Validator
**Script:** `bash-validator.sh`
**Purpose:** Blocks destructive and secret-exposing shell commands: force push (including `--force-with-lease` and `+refspec`), hard reset, forced clean, `git branch -D`, hook bypass (`--no-verify`, `git commit -n`), recursive `rm` of root/home/working-tree paths, destructive SQL outside read-only searches, destructive `gh` calls, Composer auth tokens, printing `.env` files, and this edition's framework commands (listed in `BV_FRAMEWORK_RULES` at the end of `bash-validator.sh` in this directory). The command is parsed like a shell would split it, so chains, `$(...)`, `sh -c`, `eval`, wrappers such as `sudo`/`env`/`xargs` and console abbreviations do not hide a command. It is a guard against accidental destruction, not a sandbox: a script written to disk and run later is not inspected. The same command a sixth time in a session (the payload's `conversation_id`) warns and a twelfth time blocks, since the edit counter cannot see a command loop; a file edit `loop-detection.sh` recorded since the command last ran starts its count over, and a read-only status query standing alone - `gh pr checks/status/view`, `gh run list/view/watch`, `git status`, `docker [compose] ps/logs`, `kubectl get/describe/logs`, `tail`, optionally after `sleep N &&` and piped into filters - is polling, not a loop, and is not counted. Counts live in `${TMPDIR:-/tmp}/cursor-loop-detection-<uid>-<repo-key>/`. The counter directory is created private (mode 700); one that is a symbolic link or belongs to another user turns the count off, a counter that is a symbolic link is never read or written, and a count is read as base-10 digits only.
**Input key:** `.command` (Cursor supplies the full command string).
**Return:** `0` = safe, `1` = repetition warning (sixth to eleventh identical run), `2` = block. Cursor documents no channel that shows the agent a message about a command it lets run (`agent_message` accompanies a denial), so the warning reaches only the user, in the hooks output; the agent meets the guard first at the block. Claude Code and Codex hand the same warning to the model.

### subagentStart: Subagent Gate
**Script:** `subagent-gate.sh`
**Purpose:** Allows only subagents defined in `.cursor/agents/` (frontmatter `name:`); Cursor's built-in subagents (explore, bash/shell, browser, general-purpose) are denied. Cursor has no setting that disables its built-ins, so this hook is the enforcement point; `.cursor/rules/subagent-policy.mdc` is the steering layer on top.
**Contract:** JSON on stdout - `{"permission": "allow"}` or `{"permission": "deny", "user_message": "..."}` - always exit `0`; registered with `failClosed: true`.
**Serialization:** agents marked `writes: true` in their frontmatter run one at a time - the gate takes `/tmp/cursor-write-agent-lock-<repo-key>` (TTL 30 min, override with `SUBAGENT_WRITE_LOCK_TTL_MINUTES`), released by the completion observer or by expiry.
**Limits:** the lock is advisory and machine-local (`/tmp`, keyed by the repository path), so two containers or two machines working the same branch are not serialized against each other. It covers subagent spawns only - the main conversation's own edits are not gated, and nothing running outside the tool is. It fails open with no JSON extractor (`jq`/`php`/`python3`), with an unreadable agent roster, and - for the check-and-take race only - without `flock`. Past the TTL the holder stops blocking, so a run longer than `SUBAGENT_WRITE_LOCK_TTL_MINUTES` can be joined by a second writer.
**Caveat:** before Cursor 3.4 the payload reported every subagent as `general-purpose`, which would deny project agents too; on such versions remove this entry from `hooks.json` and rely on the rule.

### subagentStop: Subagent Dispatch Observer
**Script:** `subagent-dispatch.sh`
**Purpose:** Records each subagent completion in the task's agent channel (`msg-dispatch --event complete`; uses `subagent_type` and `status` - the documented `summary` field is unreliable in current Cursor builds) and releases the write-agent lock the gate took for a `writes: true` agent. Degrades to a no-op without Python 3.9+ or the context runtime. A completion write that fails for any other reason (not a missing task, not lightweight mode) is reported in one line - its exit status and the last line of its error - on stdout and stderr. That line reaches only the hooks output: Cursor's `subagentStop` has no channel to the orchestrating agent but `followup_message`, which would start another turn, so - unlike Claude Code, where a `PostToolUse` relay hands it on - the orchestrator is not told.
**Return:** Always exit 0, no JSON output (observation only)

### afterFileEdit: File Naming Validator
**Script:** `file-naming-validator.sh`
**Purpose:** Flags `.md` files in `tasks/` and `specs/` that do not follow the skill-prefix naming convention.
**Return:** `0` = valid, `1` = warning (currently active).
**Allowlist:** `README.md`, `CHANGELOG.md`, `MANIFEST.md`.

### afterFileEdit: Loop Detection
**Script:** `loop-detection.sh`
**Purpose:** Tracks edit count per file per session (the payload's `conversation_id`) to detect doom loops. Every edit it records also restarts the Bash Validator's repetition count for the commands run before it.
**Return:** `0` = normal, `1` = warning at 7 edits (shown in the hooks output; `afterFileEdit` has no channel back to the agent), `2` = block at 10 edits.
**Tracking:** `${TMPDIR:-/tmp}/cursor-loop-detection-<uid>-<repo-key>/`, created private; `sessionStart` clears that session's counters and any left untouched for a day. A linked or foreign directory turns the count off, and a linked counter is never followed.

## Claude Code -> Cursor Event Mapping

| Claude Code (`.claude/settings.json`) | Cursor (`.cursor/hooks.json`) |
|---|---|
| `SessionStart` | `sessionStart` |
| `UserPromptSubmit` | `beforeSubmitPrompt` - it cannot add context, so the capsule is rendered into an `alwaysApply` rule; see below |
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

Cursor's prompt-time event, `beforeSubmitPrompt`, receives the prompt but
returns only `{"continue": true|false, "user_message": "..."}`: it can allow or
block a submission, not add context to it. The capsule therefore arrives
through `.cursor/rules/working-memory.mdc`, an `alwaysApply` rule Cursor sends
with every request:

- `working-memory-read.sh` (`beforeSubmitPrompt`) retrieves for the prompt and
  renders that capsule into the rule before the request leaves - in the same
  turn when Cursor reads its rules after the hook, on the next prompt
  otherwise. An identical complete render leaves the file unchanged; a
  changed excerpt or selected set replaces it even when document labels match.
  Its header says "retrieved for a recent prompt". Provider cache/token effects
  and native rule-loading order are not measured by these hooks.
- `working-memory-write.sh` (`stop`) renders the branch's capsule after the
  turn checkpoint, unless the rule holds the prompt hook's capsule for the same
  task; `local-context.sh` (`sessionStart`) attempts a fresh render. Publication
  and retirement require current custody; unproven rules remain intact.

The ignored `memory-bank/local/cursor-rule.json` receipt binds exact bytes,
phase and task. Stored device/inode/mtime fields are advisory historical locators,
not permanent ownership: copy, restore, rename, touch and reboot changes reconcile
against the current opened image. Losing the disposable local cache is safe.
A Git ignore directive for the path - the root `.gitignore` line the installer
writes, `.git/info/exclude` or a global excludes file - reserves this generated
output slot. A complete historical machine envelope in that untracked slot
upgrades on the next invocation, including a stale receipt left by an older
branch. A file that still opens with the auto-rendered header but was edited -
a note appended below the capsule, say - is copied to
`memory-bank/local/cursor-rule-edits/` (the newest 20 are kept) and the slot is
written again, so it never freezes with another task's capsule. Put custom rules
at another path: a file without that header, tracked files and foreign links
are preserved. The rule is written only into the project's own `.cursor/rules`
directory: a `.cursor` or `.cursor/rules` that is a symbolic link - a rules
folder shared between projects - is never written through, since the capsule
would reach every project sharing it, and the hooks report `linked_directory`.
Keep `.cursor/rules` a directory and link shared rule files inside it. A rule
the first releases (2026-08-03..05) wrote under their own frontmatter is
generated output too: it is saved like an edited one and the slot is written
again.
A failed render keeps a proven current-task rule; a proven other-task rule or
missing task can be retired. This is local maintenance, not source authority or
chat ownership. New facts always come from the current render, never the old rule.

Direct-host maintenance uses anchored POSIX operations or the native Windows
handle backend and the cooperative Brain lock. Linux uses no-clobber rename
when supported; other POSIX systems can use the link fallback. An old interrupted
two-link publication is recoverable only with its exact same-directory UUID
alias, full managed image, two untracked names and locked identity rechecks;
other links stay untouched. Native Windows/foreign filesystem behavior requires
its own platform CI; unsupported operations preserve the file.

Harness and delivered hook paths attempt retirement before redundant delivery.
Hook maintenance has a three-second process budget where `timeout` exists;
Harness uses its execution supervisor. Cleanup may outlast those budgets. Fixed
stderr diagnostics report refusal/incomplete/unavailable maintenance without
printing capsule content. Protected Git system/global `safe.directory` policy is
honored narrowly for the selected root; inherited Git command configuration never
grants trust. Rule and receipt remain separate publications, without a durable
transaction or atomic CAS against unsynchronized foreign writers.

The rule is ignored local state (this edition's `.gitignore` lists it). Cursor also
runs the Claude Code hooks it finds; the Claude copies of the two
working-memory hooks recognize Cursor's payload and stand down while this
folder's hooks serve. These Cursor copies differ from the canonical
`.claude/hooks` scripts by declared MIRROR_RULES transformations (the
`_WM_DELIVERY_*` and `_WM_PROMPT_*` constants in
`memory-bank/scripts/context_retrieval.py`), verified by
`scripts/build_mirrors.py --check` - not drift.

Explicit retrieval still works on Cursor: run `context.py retrieve` (or the
`memory` command) when a task needs sharper context than the rendered rule.

## Chat snapshots and merges

`context-continuity.sh` is wired on `sessionStart`, `beforeSubmitPrompt` and
`afterAgentResponse`; the payload's `hook_event_name` selects the action. A
prompt (`prompt`) or a response (`text`) is appended to a per-chat snapshot,
keyed by `conversation_id`, under the ignored `.context-handoff/` - in an
attached session, under the launcher's state directory instead. At
`sessionStart` a merge prepared with `context-load merge` for Cursor is
delivered to a new conversation as `additional_context`: at most 6,000 bytes,
pointing at the full archive under `.context-handoff/merges/`. Session-start
delivery depends on the installed desktop client; cloud agents do not expose
this event. A merged conversation gets the same sources again when it resumes.
Nothing else is restored: a new conversation starts with the Task Capsule the
working-memory hooks render into `working-memory.mdc`.

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

In governed native sessions the prompt hook arms a metadata-only challenge. At Stop, after the Git checkpoint, the same agent receives at most one internal continuation to submit progress, optional next steps and up to three checked source-backed learnings via `context.py native-result`. A clean research turn needs no Git delta or manual memory command. Empty learnings and an explicit skip are valid; no transcript or final prose is automatically adopted as knowledge. Task provisioning occurs only after a valid structured result; agent attestation never becomes person review or task completion.

Pending captures pin project, actor, session, task revision, checkout and runtime. Conflicts are held; repeated saves use the same result ID. Recovery does not checkpoint the user turn twice. A failed or uncertain save is reported in the next capsule. Harness/recovery and lightweight sessions do not arm this path. Local challenges expire for save after 24 hours and are bounded to 128 sessions; a new prompt always starts fresh. Cursor's Stop entry uses `loop_limit: 1`.

Every prompt, Stop and save also appends one metadata line to ignored `memory-bank/local/native-results.ndjson` - the host, the step and a fixed code such as `memory-result-v1:unverified`, never content, paths or IDs - and the Stop hook adds its save request's exit status, 124 when the budget killed it. How many saves were requested, made or refused, and why, is then counted from disk. The journal keeps eight lines per `refresh_health_retention` record.
