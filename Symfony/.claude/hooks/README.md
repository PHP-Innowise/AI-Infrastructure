# Claude Code Hooks Documentation

## Active Hooks

### SessionStart: Local Context Scanner
**Script:** `local-context.sh`
**Purpose:** Outputs project metadata at session start: git branch, Composer/PHP/tooling markers, framework/structure, governed or lightweight mode, index health/staleness, active binding count, Project Brain validation status, and Memory Bank validation summary. It never runs indexing or retrieval and never prints/injects record contents.
**Return:** Always 0 (informational only)

### UserPromptSubmit: Working-Memory Read
**Script:** `working-memory-read.sh`
**Purpose:** Runs `context.py refresh`, which re-indexes procedural (AGENTS.md, CLAUDE.md, skills), semantic (README, docs, specs, active Memory Bank chunks, task documents) and episodic (CHANGELOG.md) memory in one incremental pass and reports each layer as `updated` or `failed`. With a task — from `CONTEXT_TASK_ID` or the current branch — the same process also assembles a bounded Task Capsule with `--ephemeral`, so the per-request manifest, which records the query text, stays in ignored local state rather than shared Git history. A capsule failure is reported as a warning; the layer refresh stands.
**Return:** Always 0 (context tooling must never block a prompt)
**Budget:** `CONTEXT_HOOK_BUDGET` seconds, default 5
**Stands down:** when `CONTEXT_CAPSULE_DELIVERED=1`, set by a host that already put this turn's capsule into the prompt (the Harness does). The Stop hook still checkpoints the task.

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
**Purpose:** Enforces discovered skill prefixes, zero-padded `tasks/TASK-001/` directories, and `memory-bank/chunks/MEM-0001-short-slug.md` naming.
**Return:** 0 = valid/not applicable, 2 = block
**Allowlist:** README.md, CHANGELOG.md, MANIFEST.md

### PreToolUse (Bash): Bash Validator
**Script:** `bash-validator.sh`
**Purpose:** Blocks destructive and secret-exposing shell commands. One generic section, byte-identical in every edition and in Infrastructure-Creator (between the `bash-validator generic section` markers), blocks:
- force push (`--force`, `-f` in any option cluster, `--force-with-lease`, `--mirror`, a `+refspec`), `git reset --hard`, `git clean -f`, `git branch -D`;
- hook bypass: `--no-verify`, `git commit -n`, `git -c core.hooksPath=...` or `--config-env=core.hooksPath=...`, and writing or unsetting `core.hooksPath` with `git config`;
- recursive `rm` of `/`, a top-level directory, `~`, `$HOME`, `.`, `..` or `*`, and `--no-preserve-root`;
- destructive SQL (`DROP TABLE/DATABASE/SCHEMA`, `TRUNCATE TABLE`, `DELETE` with an always-true `WHERE` such as `1=1` or `1`), and `TRUNCATE` without `TABLE` or `DELETE` with no `WHERE` when the command runs SQL (`mysql`, `psql`, `sqlite3`, `wp db query`, `dbal:run-sql`, `artisan tinker`, ...), so a test name or PR title saying "delete from the cart" passes; never inside a read-only search (`grep`, `rg`, `git grep`, `git log -S`) or a commit message; `dropdb` and `mysqladmin drop`;
- `gh repo delete/archive`, `gh issue/release delete`, `gh api -X DELETE`, and auth tokens written by `composer config`;
- printing `.env`, `.env.local` or `.env.*.local` (`cat`, `head`, `tail`, `less`, `more`, `bat`, `sed` without `-i`, `awk`, `cut`, and `grep`/`rg`/`ag`/`ack` unless `-q`, `-l`, `-L` or `-c` keeps the lines out); `.env.example` and `.env.dist` stay readable, and a search whose pattern is `.env` (`grep -rn .env src/`) is not a read.

**Symfony rules** (`bin/console`, `php bin/console`, `symfony console`): `doctrine:database:drop`, `doctrine:schema:drop`, `doctrine:migrations:execute --down`, `doctrine:migrations:migrate prev|first|0|current-N`, `doctrine:fixtures:load` without `--append`, `messenger:failed:remove --all`, and printing secrets with `secrets:reveal`, `secrets:list --reveal` (or `-r`), `secrets:decrypt-to-local`, `debug:dotenv` or `debug:container --env-vars|--env-var`. The console resolves abbreviations, so `d:d:d` and `doc:data:drop` are blocked too.

**How it matches:** the command is parsed once, in pure bash with no fork per rule. Line continuations are joined, quotes removed, and `;` `&&` `||` `|` `&`, subshells and newlines split it into simple commands; `$(...)`, backticks, `sh`/`bash -c` (also after `-euo pipefail`), `eval`, `git submodule foreach` and a heredoc or here-string fed to a shell (directly, through `docker exec -i app bash` or `ssh host bash -s`, or to `ssh host` with no remote command) are checked as commands of their own. Leading `VAR=value`, wrappers (`command`, `exec`, `sudo`, `env`, `nohup`, `time`, `timeout`, `xargs`, ...), a binary's directory (`/usr/bin/git`), git's global options (`-C`, `-c`, `--git-dir`, `--no-pager`) and launchers without rules of their own (`docker compose exec app`, `ddev`, `lando`, `ssh host`) are looked through, and so is a quoted command line given to one (`ssh host '...'`, `ddev exec "..."`, `vagrant ssh -c "..."`). Console options may stand before the command name (`bin/console --reveal secrets:list`), and an option's value (`--env production`) is not taken for the command's argument. Git and option rules are case-sensitive (`git branch -d` passes); SQL keywords are not. A heredoc written to a file or used as a commit message is data.

**Limits:** the validator is a guard against accidental destruction, not a sandbox: a script written to disk and run later, or text piped into a shell, is not inspected. Nesting deeper than any real command (about a hundred levels of `(`, `$(` or backticks) is refused as `command too complex` instead of parsed, since bash would run out of stack and the hook would fail open. Parsing time grows linearly with the command: a 100 KB heredoc file write takes a fraction of a second, 100 KB of dense shell syntax about two seconds, inside the 5 s hook timeout.

**Diagnostics:** a block prints its rule category, never the command body. Without `jq`, `php` or `python3` to decode the payload the hook warns once and fails open.

**Tests:** the shared corpus `tests/fixtures/bash-validator-corpus.json` in the accelerator repository runs every case through every shipped copy (`.claude`, `.cursor`, `.codex`) and its host's payload shape.

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
| `UserPromptSubmit` | A request is submitted, before the model runs |
| `Notification` | Claude needs user attention |
| `Stop` | Claude finishes responding |
| `PreToolUse` | Before a tool executes |
| `PostToolUse` | After a tool executes |

## Wiring

`.claude/settings.json` registers every script as `"${CLAUDE_PROJECT_DIR}"/.claude/hooks/<script>.sh`. Claude Code runs a hook in the session's current directory, and that directory follows every `cd` the agent makes. A bare `.claude/hooks/<script>.sh` would exit 127 from any subdirectory, and Claude Code treats every exit other than `2` as non-blocking: the Bash Validator would stop blocking without a word. `CLAUDE_PROJECT_DIR` is the project root Claude Code exports to every hook; it stays double-quoted, as Claude Code's shell form requires, so a project path containing spaces survives. Register any hook you add the same way.

## Personal Hooks

Use `.claude/settings.local.json` for personal hooks that shouldn't be shared with the team.

## References

- [Claude Code Hooks Documentation](https://docs.anthropic.com/en/docs/claude-code/hooks)
