---
name: hook-forge
description: Generate the target PHP project's hooks - four enforcement hooks (local-context, bash-validator, file-naming-validator, loop-detection) plus the two working-memory hooks that automate the context-brain layer memory-seed installs - and per-edition wiring, tuned to the target's real detected stack and risk surface from an approved Project Profile. Blocks only dangerous commands whose tools were actually detected. Use once profile-synthesizer has produced a profile. Triggers on "generate hooks", "forge the target's hooks", "wire hooks for editions".
phase: generation
flow-next: memory-seed
flow-alternatives: [policy-forge, skill-forge]
related: [infra-generate, policy-forge, skill-forge, agent-forge, command-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Hook Forge

## Overview

`hook-forge` writes the target project's deterministic guardrails and its automatic memory loop - seven shell hooks in three groups, plus the per-edition wiring that runs them:

- **Enforcement (4):** inject context, validate risky shell commands, enforce file-naming policy, and detect agent loops. Every enforcement hook is tuned to the target's REAL stack and risk surface from the profile, not a template: the danger list in `bash-validator.sh` contains only commands whose underlying tools were actually detected.
- **Working memory (2):** `working-memory-read.sh` (prompt-time: refresh every memory layer and emit a bounded task capsule) and `working-memory-write.sh` (turn-end: buffer the turn's change set and flush it to the authoritative task on a boundary). These are stack-agnostic thin callers of the runtime `memory-seed` installs at `memory-bank/scripts/context.py` - that path is a fixed contract between the two forges.
- **Subagent gate (1):** `subagent-gate.sh` restricts subagent spawning to the target's own generated roster; the host tool's built-in agents (Claude Code's Explore/Plan/general-purpose, Cursor's explore/bash/browser, Codex's spawn_agent roles) are denied. Unlike the six shared scripts, the gate is TOOL-OWNED: each edition gets a variant matching its host's contract - Claude reads `tool_input.subagent_type` and blocks with exit 2, Cursor answers `subagentStart` with `{"permission": "allow"|"deny"}` JSON, Codex denies the `spawn_agent` tool family outright (its edition delegates through skills and has no agent roster).

Hooks are generated ONLY for the selected editions, and each edition wires the identical scripts through its own configuration mechanism.

Consumes profile sections **1** (which editions), **2** (stack/tooling for context + naming rules), **4** (integrations that widen the risk surface), **5** (infra/ops - the only source that authorizes infra danger rules), and **6** (security - secrets/destructive-op posture).

## Generated File Naming Convention (MANDATORY)

For each selected edition, into the target write the seven hook scripts under that edition's hooks dir (`.claude/hooks/`, `.cursor/hooks/`, `.codex/hooks/`): `local-context.sh`, `bash-validator.sh`, `file-naming-validator.sh`, `loop-detection.sh`, `working-memory-read.sh`, `working-memory-write.sh`, `subagent-gate.sh` - with ONE exception: Cursor gets no `working-memory-read.sh`; its read path is instead rendered into `.cursor/rules/working-memory.mdc` by the Cursor copies of `working-memory-write.sh` and `local-context.sh` (steps 6 and 8). All scripts MUST pass `bash -n` and be `chmod +x`. Then write the edition-specific wiring file (below). Append a log to `tasks/TASK-{N}/hook-forge-log.md` listing every hook, its wiring, and the profile evidence backing each danger rule.

## Process

1. **Read the profile.** Confirm selected editions (section 1). Build the context payload from section 2 (framework/version, test/lint/static tools). Build the risk list ONLY from what is present: section 5 (containers/CI/IaC), section 4 (integrations), section 6 (secrets/destructive ops).
2. **Author `bash-validator.sh`** to block only detected risks. Include `php artisan migrate:fresh` / `migrate:rollback` ONLY if that framework's migration tooling was detected; include `kubectl delete` ONLY if Kubernetes was detected in section 5; include `terraform destroy` ONLY if Terraform was detected in section 5. Never block a tool the target does not use.
3. **Author `local-context.sh`** to echo the target's real stack summary at session start (framework, version, key command lines).
4. **Author `file-naming-validator.sh`** to enforce the target's naming policy (task/spec prefixes, zero-padded task dirs) as stated in the generated policy.
5. **Author `loop-detection.sh`** to detect repeated identical tool calls/edits and warn (warn at 7, block at 10). Track edit counts in a per-repository temp directory namespaced by a stable hash of the repo root (e.g. `TRACK_DIR="/tmp/<tool>-loop-detection-$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)"`), so parallel projects never share counters.
6. **Author the working-memory pair** as thin, always-exit-0 callers of `memory-bank/scripts/context.py` (resolve `ROOT_DIR` from the script's own location - `$(dirname "${BASH_SOURCE[0]}")/../..` - never from `$PWD`):
   - `working-memory-read.sh` (the read half; runs at prompt time, writes no task state): skip silently (`exit 0`) when `python3` or the CLI is missing; reduce the prompt JSON arriving on stdin to a bounded, quoteless query via a `python3 -c` one-liner (first ~24 word tokens - never `sed`-scrape it); resolve the task ID from `$CONTEXT_TASK_ID` falling back to the current git branch; run `context.py refresh`, adding `--query "$QUERY" --task-id "$TASK_ID" --ephemeral` only when both are non-empty (`--ephemeral` keeps the per-request manifest in ignored local state); bound the call with `timeout "${CONTEXT_HOOK_BUDGET:-5}"` when `timeout` exists; print the layer report under a header stating retrieved context is not authoritative; always `exit 0`.
   - `working-memory-write.sh` (the write half; runs at turn end, where a real delta exists): same python3/CLI/`timeout` guards; require a task ID (`$CONTEXT_TASK_ID` or git branch) and exit 0 silently without one; run `context.py turn --task-id "$TASK_ID" --flush-after "${CONTEXT_FLUSH_AFTER:-5}"` with all output discarded; always `exit 0` - a failed checkpoint must never surface as a turn error. It reads only Git porcelain metadata; working-tree contents never reach the buffer.
   - **Cursor read path (Cursor copies only):** Cursor cannot receive the capsule at prompt time, so the Cursor copies of `working-memory-write.sh` (after the `turn` call) and `local-context.sh` (at session start, so a fresh session or branch switch never serves the previous session's capsule) render the freshest capsule into the alwaysApply rule `.cursor/rules/working-memory.mdc`, which Cursor attaches to every prompt. Render with the existing capsule command - `context.py context "$TASK_ID" --task-id "$TASK_ID" --ephemeral`, bounded by the same `timeout` budget. The rendered file MUST carry frontmatter `alwaysApply: true` and a staleness header stating the capsule is **as of end of previous turn** and not authoritative; write it atomically (`mktemp` in `.cursor/rules/`, then `mv` over the rule) and only when the render produced output - a failed or empty render keeps the previous rule. The render is silent on stdout and never changes the exit code. The Claude and Codex copies of both hooks MUST NOT render this file - they receive the capsule through `working-memory-read.sh`.
7. **Author `subagent-gate.sh` per edition** (tool-owned variants, one contract each):
   - **Claude variant:** self-filter to `tool_name` Agent/Task; extract `tool_input.subagent_type` (jq/php/python3 fallback chain); allow only names found as frontmatter `name:` in the target's `.claude/agents/*.md`; deny everything else with a reason on stderr and exit 2; fail open (exit 0) on missing payload fields or an unreadable roster.
   - **Cursor variant:** `subagentStart` contract - read top-level `subagent_type`, allowlist against `.cursor/agents/*.md` frontmatter names, answer `{"permission":"allow"}` or `{"permission":"deny","user_message":"..."}` on stdout, always exit 0.
   - **Codex variant:** deny the multi-agent tool family (`spawn_agent`, `Agent`, `send_input`, `resume_agent`, `wait_agent`, `close_agent`) unconditionally with stderr + exit 2 - the Codex edition delegates through skills and ships no agents.
8. **Wire per edition** (identical scripts, different config):
   - **Claude** -> `.claude/settings.json`: `SessionStart` (local-context, timeout 10), `UserPromptSubmit` (working-memory-read, timeout 8), `Stop` (working-memory-write, timeout 8), `PreToolUse` (file-naming-validator on `Write|Edit`, bash-validator on `Bash`, subagent-gate on `Agent|Task`, timeout 5 each), `PostToolUse` (loop-detection on `Edit`, timeout 5), each with a `matcher` where noted and a `timeout` in **seconds**. The same file also carries the configuration half of the gate: `permissions.deny` entries `Agent(Explore)`, `Agent(Plan)`, `Agent(general-purpose)`, `Agent(claude)`, `Agent(statusline-setup)`, `Agent(claude-code-guide)`, plus `env` keys `CLAUDE_CODE_DISABLE_EXPLORE_PLAN_AGENTS: "1"` and `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH: "1"` - the deny list blocks today's built-ins, the hook covers built-ins added later. The command form is the bare script path (`.claude/hooks/<script>.sh`): Claude Code pipes the tool-input JSON to the hook on stdin, so an `echo '$TOOL_INPUT' | <script>` wrapper feeds the hook the literal string `$TOOL_INPUT` instead of the payload and silently disables it.
   - **Cursor** -> `.cursor/hooks.json`: `version: 1`, camelCase events, timeouts in **seconds**: `sessionStart` (local-context, 15), `subagentStart` (subagent-gate, 5, with `failClosed: true` - Cursor has no setting that disables its built-in subagents, so this hook is the only enforcement point), `beforeShellExecution` (bash-validator, 5), `afterFileEdit` (file-naming-validator, 5; loop-detection, 5), `stop` (working-memory-write, 15). The two 15s budgets cover the turn flush/session banner plus the capsule render (each inner call is separately `timeout`-bounded). Commands are the same bare script paths (`.cursor/hooks/<script>.sh`) - no `bash`/`sh` prefix, no shell wrapper. **Cursor has no `UserPromptSubmit`-equivalent event, so the read half of automatic memory is deliberately absent from the Cursor edition** - do not generate `working-memory-read.sh` for Cursor; its read path is the rendered rule from step 6. Add `.cursor/rules/working-memory.mdc` to the target's `.gitignore` (the rendered rule is transient local state), and record the mechanism in the log and the hooks README.
   - **Codex** -> `.codex/hooks.json`: Claude-style event names, NO `matcher`/`timeout` fields: `SessionStart` (local-context), `UserPromptSubmit` (working-memory-read, with `additionalContextLimit: 4000`), `Stop` (working-memory-write), `PreToolUse` (bash-validator, file-naming-validator, subagent-gate - the gate self-filters to multi-agent tool names), `PostToolUse` (loop-detection). Commands are bare script paths (`.codex/hooks/<script>.sh`) - no `bash`/`sh` prefix. Plus `.codex/config.toml` containing `[features]` with `hooks = true` and `multi_agent = false`, and an `[agents]` table with `enabled = false` - the config switches are the hard off-story for Codex built-in subagents; the hook is the backstop for CLI/model combinations that ignore the flags.
9. **Log** every hook, its wiring per edition, the section 4/5/6 evidence that authorized each danger rule, the Cursor read-path exception, and the three gate variants.

## Output Template

```markdown
# Hook Forge Complete: [target_name]

**Editions:** [selected]
**Hooks per edition:** local-context.sh, bash-validator.sh, file-naming-validator.sh, loop-detection.sh, working-memory-read.sh (not Cursor), working-memory-write.sh, subagent-gate.sh (tool-owned variant per edition)

## Wiring
- claude: .claude/settings.json (second timeouts, matchers, bare script paths; UserPromptSubmit -> read, Stop -> write; PreToolUse Agent|Task -> subagent-gate + Agent(...) deny rules + built-in-agent env keys)
- cursor: .cursor/hooks.json (version 1, second timeouts; subagentStart -> subagent-gate with failClosed; stop -> write + capsule render into .cursor/rules/working-memory.mdc, sessionStart re-renders - no read event exists; rule gitignored)
- codex: .codex/hooks.json + .codex/config.toml ([features] hooks=true multi_agent=false, [agents] enabled=false; UserPromptSubmit -> read, Stop -> write, PreToolUse -> subagent-gate)

## Danger rules (evidence-gated)
- [rule -> profile section 4/5/6 citation]

## Checks
- bash -n: [pass] | chmod +x: [applied] | context.py path contract: memory-bank/scripts/context.py

## Log
tasks/TASK-{N}/hook-forge-log.md

## Next
memory-seed; policy-forge/skill-forge if not already run.
```

## Guardrails

- MUST generate hooks ONLY for the selected editions, each wired through its own mechanism (settings.json / hooks.json / hooks.json+config.toml).
- MUST block a destructive command ONLY when the profile confirms that tool is present (no `terraform destroy`/`kubectl delete`/migration-reset rules without section 5/2 evidence).
- MUST ensure every script passes `bash -n` and is `chmod +x`.
- MUST use second timeouts for Claude and Cursor, and no matcher/timeout for Codex.
- MUST wire EVERY edition's hooks as bare repo-relative script paths (`.claude/hooks/<script>.sh` / `.cursor/hooks/<script>.sh` / `.codex/hooks/<script>.sh`) - never behind a `bash`/`sh` interpreter prefix and never as `echo '$TOOL_INPUT' | <script>`: the payload arrives on stdin, and a wrapped or prefixed path is exactly the form that lets a missing hook slip past a dead-hook check.
- MUST pass every blocked-pattern regex to `grep` after `--`, and MUST decode the tool-input JSON (jq/php/python3) rather than scraping it with `sed`: a pattern such as `--no-verify` is otherwise read as a grep option, and a `"[^"]*"` scrape truncates any command containing escaped quotes. Both failures exit 0 and silently allow the command.
- MUST make both working-memory hooks always exit 0 and degrade silently (missing `python3`, missing CLI, no task ID, timeout): context tooling must never block a prompt or surface as a turn error.
- MUST call the runtime at exactly `memory-bank/scripts/context.py` relative to the target root (the path `memory-seed` creates); MUST NOT inline memory logic into the hooks themselves or write task state from the read hook.
- MUST NOT generate a Cursor `working-memory-read.sh` or invent a Cursor read event; the asymmetry is documented, not a bug to fix. The Cursor read path is the rendered rule: the Cursor copies of `working-memory-write.sh` and `local-context.sh` MUST render the capsule into `.cursor/rules/working-memory.mdc` (frontmatter `alwaysApply: true`, staleness header "as of end of previous turn", atomic temp-file-then-`mv` replace, previous rule preserved on a failed or empty render, silent on stdout), the target's `.gitignore` MUST list that file, and the Claude/Codex copies MUST NOT render it.
- MUST generate `subagent-gate.sh` as three TOOL-OWNED variants (Claude PreToolUse exit codes, Cursor subagentStart permission JSON, Codex spawn_agent-family deny) - never one shared script - and MUST pair the hook with its configuration half: the `Agent(...)` deny list and built-in-agent env keys for Claude, `failClosed: true` for Cursor, `multi_agent = false` plus `[agents] enabled = false` for Codex. The Claude and Cursor gates read the roster from that edition's generated agents' frontmatter `name:` values; a missing or unreadable roster fails OPEN (delegation must not brick), while a recognized built-in fails CLOSED.
- MUST NOT print or log any secret or credential value from the target.
- MUST NOT translate section 8 business invariants, permissions, approvals, or lifecycle rules into brittle shell/text-matching hooks. Behavioral rules belong in policy, skills, tests, review, and memory; hooks enforce only deterministic tool/command/file events.

## Final Output

Return the per-edition hook paths, the wiring file per edition, the evidence-gated danger rules, the `bash -n`/exec-bit results, the log path, and the next step (`memory-seed`).
