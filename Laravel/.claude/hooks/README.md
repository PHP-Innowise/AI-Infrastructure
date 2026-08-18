# Claude Code Hooks Documentation

## Active Hooks

### SessionStart: Local Context Scanner
**Script:** `local-context.sh`
**Purpose:** Outputs project metadata at session start: git branch, Composer/PHP/Laravel/tooling markers, Livewire/Inertia detection, structure, governed or lightweight mode, index health/staleness, active binding count, Project Brain validation status, and Memory Bank validation summary. It never runs indexing or retrieval and never prints/injects record contents.
**Return:** Always 0 (informational only)

### UserPromptSubmit: Working-Memory Read
**Script:** `working-memory-read.sh`
**Purpose:** Runs `context.py refresh`, which re-indexes procedural (AGENTS.md, CLAUDE.md, skills), semantic (README, docs, specs, active Memory Bank chunks, task documents) and episodic (CHANGELOG.md) memory in one incremental pass and reports each layer as `updated` or `failed`. With a task — from `CONTEXT_TASK_ID` or the current branch — the same process also assembles a bounded Task Capsule with `--ephemeral`, so the per-request manifest, which records the query text, stays in ignored local state rather than shared Git history. A capsule failure is reported as a warning; the layer refresh stands.
**Return:** Always 0 (context tooling must never block a prompt)
**Budget:** `CONTEXT_HOOK_BUDGET` seconds, default 5

The capsule is retrieved context, not authority. It never outranks the source it summarizes.

### Stop: Working-Memory Write
**Script:** `working-memory-write.sh`
**Purpose:** Buffers this turn's change set and flushes it to the authoritative task on a boundary. Reads Git porcelain metadata only — working-tree contents never reach the buffer. Sensitive-looking paths are excluded and reported; the runtime's own record, handoff, and index churn is dropped rather than recorded as user work. The first flush provisions the task if it does not exist, deriving a goal from the branch name, so the automated path needs no manual `start`. An existing task is never overwritten.
**Return:** Always 0 (a failed checkpoint must never surface as a turn error)
**Budget:** `CONTEXT_HOOK_BUDGET` seconds, default 5
**Flush boundary:** `CONTEXT_FLUSH_AFTER` turns, default 5

Buffering is what keeps per-turn continuity affordable: without it, every turn would cost one governed revision and one rewritten handoff.

### PreToolUse (Write|Edit): File Naming Validator
**Script:** `file-naming-validator.sh`
**Purpose:** Validates that .md files in `tasks/` and `specs/` follow skill-prefix naming convention.
**Return:** 0 = valid name, 1 = warning (currently active), 2 = block (after tuning)
**Allowlist:** README.md, CHANGELOG.md, MANIFEST.md

### PreToolUse (Bash): Bash Validator
**Script:** `bash-validator.sh`
**Purpose:** Blocks destructive commands: force-push, hard reset, database drops/truncates, destructive `artisan migrate:*` resets/rollbacks/`db:wipe`, forced production seeding, secret-writing Composer config, and `--no-verify`.
**Return:** 0 = safe command, 2 = block

### PreToolUse (Agent|Task): Subagent Gate
**Script:** `subagent-gate.sh`
**Purpose:** Allows only subagents defined in `.claude/agents/` (frontmatter `name:`); Claude Code's built-in agents (Explore, Plan, general-purpose, claude, statusline-setup, claude-code-guide) are denied, so every delegation goes through the accelerator's command -> agent -> skill pipeline. Works together with the `Agent(...)` deny rules and `CLAUDE_CODE_DISABLE_EXPLORE_PLAN_AGENTS` in `.claude/settings.json`; the hook also covers built-in agents added after that deny list was written.
**Return:** 0 = allow, 2 = block
**Serialization:** agents marked `writes: true` in their frontmatter run one at a time - the gate takes `/tmp/claude-write-agent-lock-<repo-key>` (TTL 30 min, override with `SUBAGENT_WRITE_LOCK_TTL_MINUTES`), released by the completion observer or by expiry.
**Limits:** the lock is advisory and machine-local (`/tmp`, keyed by the repository path), so two containers or two machines working the same branch are not serialized against each other. It covers subagent spawns only - the main conversation's own edits are not gated, and nothing running outside the tool is. It fails open with no JSON extractor (`jq`/`php`/`python3`), with an unreadable agent roster, and - for the check-and-take race only - without `flock`. Past the TTL the holder stops blocking, so a run longer than `SUBAGENT_WRITE_LOCK_TTL_MINUTES` can be joined by a second writer.

### SubagentStop: Subagent Dispatch Observer
**Script:** `subagent-dispatch.sh`
**Purpose:** Records each subagent completion in the task's agent channel (`msg-dispatch --event complete`, one sanitized line from the final assistant message) and releases the write-agent lock the gate took for a `writes: true` agent. Degrades to a no-op without python3, the context runtime, or a resolvable task.
**Return:** Always 0 (observation must never break a turn)

### PostToolUse (Edit): Loop Detection
**Script:** `loop-detection.sh`
**Purpose:** Tracks edit count per file per session. Detects doom loops.
**Return:** 0 = normal, 1 = warning at 7 edits, 2 = block at 10 edits
**Tracking:** Uses `/tmp/claude-loop-detection/` (resets on reboot)

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
| `UserPromptSubmit` | A request is submitted, before the model runs |
| `Notification` | Claude needs user attention |
| `Stop` | Claude finishes responding |
| `PreToolUse` | Before a tool executes |
| `PostToolUse` | After a tool executes |

## Personal Hooks

Use `.claude/settings.local.json` for personal hooks that shouldn't be shared with the team.

## References

- [Claude Code Hooks Documentation](https://docs.anthropic.com/en/docs/claude-code/hooks)
