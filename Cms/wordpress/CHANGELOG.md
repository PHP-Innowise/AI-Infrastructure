# WordPress Accelerator Changelog

All notable changes to the WordPress edition are recorded here. Shared
context-runtime changes are also recorded in the repository-root changelog.

## Unreleased

- The Task Capsule carries no skill pointer: hosts list skills themselves and agents took none a capsule named. `AGENTS.md`, the `project-brain` skill, `PROTOCOL.md` and the READMEs say so. Details in the root [`CHANGELOG.md`](../../CHANGELOG.md). Policy lock regenerated.

- The Task Capsule's episodic layer carries two items: the changelog and one recorded event or local episode, found by its own search. `AGENTS.md`, the `project-brain` skill, `PROTOCOL.md` and the READMEs say so. Details in the root [`CHANGELOG.md`](../../CHANGELOG.md). Policy lock regenerated.

### Added

- `context-save` and `context-load` for Codex, Claude Code and Cursor: save a curated summary or topic handoff, or preserve an explicitly exported visible conversation verbatim in `full` mode, as a task artifact under `tasks/`; loading it in another checkout, machine or client reports branch, commit and file drift. Handoffs stay out of the index and never write Project Brain, Memory Bank or SQLite. Shared runtime changes are recorded in the root changelog.
- Merge chosen chats into a new task: the `context-continuity` hook (one script for session start, prompt and end of turn) keeps each chat's visible prompts and final answers in ignored `.context-handoff/`; `context-load merge` freezes 2-8 of them into an attributed archive for the next new session in the chosen client, which receives a bounded preview with conflicts left unresolved for the agent. Only a prepared merge is delivered - a new session otherwise starts with the Task Capsule alone. `CONTEXT_CONTINUITY_DISABLED=1` turns snapshots off.
- `.cursor-plugin/plugin.json` loads this edition in place for an attached Cursor Agent session (`--plugin-dir`), pointing at the edition's own `.cursor` rules, skills, agents, commands and `hooks.json`; it is excluded from installs. The SessionStart banner (`local-context.sh`) names the clone and the state directory when the edition is attached, and describes the project rather than the hook's working directory. See [docs/ATTACHED-MODE.md](../../docs/ATTACHED-MODE.md).

### Fixed

- `.claude/settings.json` runs `loop-detection.sh` on `Edit|Write|MultiEdit|NotebookEdit` (it ran on `Edit` only, so a fix written with `Write` never restarted the Bash Validator's repetition count) and `subagent-dispatch.sh` also on `PostToolUse` for `Agent|Task`, which hands the orchestrator a completion the `SubagentStop` run could not record. The hooks keep their counters private and per session, hand their warnings to the model on Claude Code and Codex, leave polling uncounted, and Cursor drops another task's working-memory rule after a failed render; the hook READMEs say how. Details in the root [`CHANGELOG.md`](../../CHANGELOG.md). Policy lock regenerated.
- The `writing-plans` description names feature planning before writing code again, so the skill keeps its place in the first two procedural results beside the new `context-save` and `context-load` skills.
- Memory reaches every host, and says something when it speaks. `.claude/settings.json` allows `python3 memory-bank/scripts/context.py`, which Claude Code refused under `-p`. Cursor gets `.cursor/hooks/working-memory-read.sh` on `beforeSubmitPrompt`, which renders the capsule for the prompt into `.cursor/rules/working-memory.mdc`, and `.cursor/cli.json`, which allows the memory CLI in `cursor-agent`; the Claude copies of the memory hooks stand down when Cursor runs them. `.codex/config.toml` sets `project_doc_max_bytes = 131072`, so Codex no longer cuts the policy after a long project `AGENTS.md`. The `project-brain` skill, `AGENTS.md`, the READMEs and the Codex and Cursor hook READMEs describe the capsule as delivered: one skill pointer, three Semantic and one Episodic item with the passage that best matches the request, within 3,600 characters, and how it reaches Cursor. Details, measurements and the installed-project sync are in the root [`CHANGELOG.md`](../../CHANGELOG.md). Policy lock regenerated.
- `AGENTS.md` describes the memory the hooks actually run. Its Agent Behavior, Memory Bank and Project Brain sections demanded a caller-supplied task ID and `start` before work, forbade saying that hooks index or inject anything, and told `checkpoint` not to derive a task from the branch - while the shipped hooks take the task from `CONTEXT_TASK_ID` or the branch, inject a Task Capsule on every prompt and checkpoint the branch task on Stop, so one piece of work ended up with two tasks. A new Working Memory section states the hook behaviour and what an agent adds (`retrieve` before material decisions, `update --revision auto` for what a checkpoint cannot see, explicit `complete` after verification), and one Memory Bank And Project Brain section replaces the two old ones. The file shrinks 17993 -> 14925 bytes (`agents_md_bytes` ceiling tightened). The `checkpoint` skill gains a governed workflow (`turn --flush`, then a revision-checked `update`) instead of declining, the `memory` skill points at `../checkpoint/SKILL.md` so the path resolves in every tool's tree, and DOD.md scopes the metadata-only rule to the SessionStart hook.
- `skill-creator` scripts import on Python 3.9, the supported floor (the default `python3` on macOS): four scripts in each of the `.agents`, `.claude` and `.cursor` copies used `X | None` annotations without `from __future__ import annotations` and raised `TypeError` at import.
- The installed `.gitattributes` is now `.install/gitattributes` (`*.sh text eol=lf`) instead of this repository's generated mirror list, which marked every mirror - hooks included - `-diff` in the client's reviews. Remove the old `-diff` lines from an earlier install by hand.
- New `.claude/CLAUDE.md` imports `@../AGENTS.md`, so Claude Code loads the policy in projects that have their own `CLAUDE.md` or `CLAUDE.local.md` (Claude Code reads `AGENTS.md` directly only when none exists). `--merge-existing` appends the import to an existing `.claude/CLAUDE.md`.
- Hooks are wired through the project root: `"${CLAUDE_PROJECT_DIR}"/.claude/hooks/<script>.sh` in `.claude/settings.json`, and a root-finding `sh` launcher in `.codex/hooks.json`. The safety hooks now keep running when the session's directory is a subdirectory; before, they exited 127 and failed open. Policy lock regenerated. Codex users must re-trust the changed hooks once in `/hooks`.
- `.cursor/hooks/subagent-dispatch.sh` and `.codex/hooks/subagent-dispatch.sh` are executable again (100644 -> 100755). Cursor runs the hook directly on `subagentStop`; without the bit it exited 126, and the write-agent lock taken by `subagent-gate` stayed held until its 30-minute TTL. To repair an existing install, run `chmod +x .cursor/hooks/*.sh .codex/hooks/*.sh`. Reinstalling also repairs it. A hook whose content already matches gets only the bit added and is reported as `FIX_MODE` (`WOULD_FIX_MODE` with `--dry-run`). An install from an earlier release first collides on the files this release changed (`.claude/settings.json`, `.codex/hooks.json`, `bash-validator.sh`, the hook READMEs, the policy lock). `--merge-existing` then refuses and changes nothing, modes included, so use the `chmod` above or rerun with `--overwrite` after reviewing `--overwrite --dry-run`.
- bash-validator: added `wp db clean` and `wp site empty`; WP-CLI rules accept global flags before the subcommand, the `wp-cli.phar` form and launchers (`ddev wp`, `docker compose run ... wp`). New secret rules block `wp config get` of `DB_PASSWORD` or the keys and salts, and an unfiltered `wp config list`. `wp-config-sample.php` is now readable. The generic rules are documented in the root CHANGELOG.
- The Codex `SKILL FLOW.md` (`.agents/skills/`) named the Claude/Cursor commands `/brainstorm`, `/git-worktrees`, `/debugger` and `/docs-generator`, which do not exist for Codex (Codex has no command layer). They now name the skills `brainstorming`, `using-git-worktrees`, `systematic-debugger` and `documentation-generator`. Found by the new `scripts/check_routes.py` gate.
- bash-validator: reading `wp-config.php` is also blocked through `rg`, `ag`, `ack`, `egrep`, `fgrep` and `cut`. A search whose pattern is the file name (`grep -rn wp-config.php .`) passes. The leftover `schema:drop` console rule was removed, because its abbreviation matching blocked `schema:d` (`schema:dump`). Sail `art`/`a` count as artisan for the schema-reset rules.
- The Codex hook launcher in `.codex/hooks.json` now stops at the project that declared the hook (the nearest directory holding `.codex/hooks.json`). A missing hook script exits 127 and names the path it expected. Before, the launcher could run a same-named script from an ancestor directory such as `~/.codex/hooks/`. Codex users must re-trust the changed hooks once in `/hooks`. Policy lock regenerated.
- `.codex/hooks/README.md` and `.cursor/hooks/README.md` no longer send readers to `.claude/hooks/README.md`, which Codex-only and Cursor-only installs do not get. They now point at `BV_FRAMEWORK_RULES` in the `bash-validator.sh` next to them.

### Changed

- Memory promotion policy now matches runtime configuration: eligible verified
  terminal records may be applied unattended only when automatic promotion is
  enabled and remain explicitly marked as unreviewed; otherwise an independent
  human review is required.

- `STABILIZATION.md` localizes a failure before naming its root cause, as the
  other three editions do: a `Localization` section (the components an agent
  interacts with, the earliest unrecovered failure, blame that follows
  behavior rather than opportunity), a `Routing` table from the blamed side to
  the repair that can work, `Edge:` and `Blame:` in the rule template, and two
  worked examples. The Project Brain bug, finding and incident templates this
  edition shares with them point here. `bash-validator.sh` gains the shared
  repetition guard below its framework rules. Details in the root
  [`CHANGELOG.md`](../../CHANGELOG.md). Policy lock regenerated.

## 2.0.0 - 2026-08-24

### Added

- First ready-made WordPress edition under `Cms/wordpress` for Claude Code,
  Cursor and Codex.
- WordPress policy covering lifecycle, namespacing, hooks, capabilities,
  nonces, sanitization, contextual escaping, REST permissions, `$wpdb`, data
  migrations, caching, cron, multisite, accessibility, i18n and compatibility.
- Dedicated plugin, theme, Gutenberg block, hooks/events, REST implementation,
  content modeling, WP-CLI, multisite, WooCommerce, and cron/background skills.
- WordPress-specific architecture, coding, frontend, database, testing,
  security, performance, dependency, review and verification workflows.
- Inventory-driven installation, generated tool mirrors, shared Project Brain,
  local context runtime and durable Memory Bank support.
