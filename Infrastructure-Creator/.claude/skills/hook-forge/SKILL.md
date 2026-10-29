---
name: hook-forge
description: Generate the target PHP project's hooks - four enforcement hooks (local-context, bash-validator, file-naming-validator, loop-detection), the two working-memory hooks that automate the context-brain layer memory-seed installs, the chat-snapshot hook behind `context-load merge`, and the two subagent hooks (roster gate with write serialization, completion observer) - and per-edition wiring, tuned to the target's real detected stack and risk surface from an approved Project Profile. Its bash-validator runs the generator's own shell parser and adds rules only for dangerous commands whose tools were actually detected. Use once profile-synthesizer has produced a profile. Triggers on "generate hooks", "forge the target's hooks", "wire hooks for editions".
phase: generation
flow-next: memory-seed
flow-alternatives: [policy-forge, skill-forge]
related: [infra-generate, policy-forge, skill-forge, agent-forge, command-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Hook Forge

## Overview

`hook-forge` writes the target project's deterministic guardrails, its automatic memory loop and its chat snapshots - nine shell hooks in four groups, plus the per-edition wiring that runs them:

- **Enforcement (4):** inject context, validate risky shell commands, enforce file-naming policy, and detect agent loops. Every enforcement hook is tuned to the target's REAL stack and risk surface from the profile. `bash-validator.sh` runs the generator's own shell parser and generic rules and the editions' command repetition guard, copied byte for byte (step 2); the danger list authored below them contains only commands whose underlying tools were actually detected.
- **Working memory (2):** `working-memory-read.sh` (prompt-time: refresh every memory layer and emit a bounded task capsule) and `working-memory-write.sh` (turn-end: buffer the turn's change set and flush it to the authoritative task on a boundary). Every prompt read calls the runtime directly under the hook's own `timeout`. These are stack-agnostic thin callers of the runtime `memory-seed` installs: they run `memory-bank/scripts/context_cli.py`, which runs `memory-bank/scripts/context.py` from its cached bytecode, and fall back to `context.py` itself where the loader is missing - both paths are a fixed contract between the two forges.
- **Chat snapshots (1):** `context-continuity.sh` keeps each chat's visible prompts and answers in ignored `.context-handoff/` for `context-load merge`, and hands a new task only the merge prepared for it - the Task Capsule, not this hook, carries a branch's work. A thin caller of `memory-bank/scripts/context_continuity.py`; it never writes Project Brain, Memory Bank or SQLite.
- **Subagent orchestration (2):** `subagent-gate.sh` restricts subagent spawning to the target's own generated roster; the host tool's built-in agents (Claude Code's Explore/Plan/general-purpose, Cursor's explore/bash/browser, Codex's spawn_agent roles) are denied, and agents whose frontmatter carries `writes: true` are serialized one at a time. `subagent-dispatch.sh` observes the other end: when a subagent finishes it appends a `completion` entry to the task's agent channel and releases that write lock. Unlike the seven shared scripts, the gate is TOOL-OWNED: each edition gets a variant matching its host's contract - Claude reads `tool_input.subagent_type` and blocks with exit 2, Cursor answers `subagentStart` with `{"permission": "allow"|"deny"}` JSON, Codex denies the `spawn_agent` tool family outright (its edition delegates through skills and has no agent roster). The dispatch observer is one script in all three editions.

Hooks are generated ONLY for the selected editions under the required
generation root (task staging during orchestration), and each edition wires the
scripts through its own configuration mechanism. Stack/risk evidence is always
read from the real target; generated paths are resolved from staging.

Consumes profile sections **1** (which editions), **2** (stack/tooling for context + naming rules), **4** (integrations that widen the risk surface), **5** (infra/ops - the only source that authorizes infra danger rules), and **6** (security - secrets/destructive-op posture).

## Generated File Naming Convention (MANDATORY)

For each selected edition, into the target write the nine hook scripts under that edition's hooks dir (`.claude/hooks/`, `.cursor/hooks/`, `.codex/hooks/`): `local-context.sh`, `bash-validator.sh`, `file-naming-validator.sh`, `loop-detection.sh`, `working-memory-read.sh`, `working-memory-write.sh`, `context-continuity.sh`, `subagent-gate.sh`, `subagent-dispatch.sh`. Cursor's `working-memory-read.sh` runs on `beforeSubmitPrompt` and renders the capsule into `.cursor/rules/working-memory.mdc`, as its `working-memory-write.sh` and `local-context.sh` do at the end of a turn and at session start (steps 6 and 9). All scripts MUST pass `bash -n` and be `chmod +x`. Then write the edition-specific wiring file (below). Append a log to `tasks/TASK-{N}/hook-forge-log.md` listing every hook, its wiring, and the profile evidence backing each danger rule.

## Memory read identity contract (mandatory)

A non-empty `CONTEXT_TASK_ID` is explicit, even when it equals the branch;
never mark it as inferred. Otherwise read the symbolic branch from `ROOT_DIR`.
Detached HEAD supplies no inferred task, except during a rebase: both prompt
reads and turn checkpoints may follow their fallback line with exactly
`[ -n "$TASK_ID" ] || TASK_ID=$(rebased_branch)`, defined below; the runtime
admits that branch only while the rebase runs. Ordinary detached HEAD stays
unbound. Keep the
following canonical memory fragments: the structural gate rejects
unsupported rewrites rather than guessing arbitrary shell equivalence. These
fragments supplement the root, readiness, privacy, publication and response
rules in steps 3, 6 and 9.

```bash
rebased_branch() {
  local directory state name
  directory=$(git -C "$PROJECT_DIR" rev-parse --absolute-git-dir 2>/dev/null) || return 1
  for state in rebase-merge rebase-apply; do
    [ -f "$directory/$state/head-name" ] || continue
    name=
    IFS= read -r name < "$directory/$state/head-name" || [ -n "$name" ] || continue
    case "$name" in refs/heads/?*) printf '%s' "${name#refs/heads/}"; return 0 ;; esac
  done
  return 1
}
```

All prompt reads use this Claude fragment. Codex changes only the host to
`codex`. Cursor initializes `ARGUMENTS=(refresh --host cursor --delivery-profile cursor-rule)`
and appends `ARGUMENTS+=(--json)` after the conditional. Claude/Codex may append
the existing guarded session/transcript arguments immediately after the marker;
Cursor must not append them. Preserve the private stderr file and diagnostics.

```bash
TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$ROOT_DIR" symbolic-ref --quiet --short HEAD 2>/dev/null)}"
ARGUMENTS=(refresh --host claude)
if [ -n "$QUERY" ] && [ -n "$TASK_ID" ]; then
  ARGUMENTS+=(--query "$QUERY" --task-id "$TASK_ID" --ephemeral --sanitize)
  [ -n "${CONTEXT_TASK_ID:-}" ] || ARGUMENTS+=(--expected-branch "$TASK_ID")
fi
REPORT=$(run "${PYTHON[@]}" "$CONTEXT_CLI" "${ARGUMENTS[@]}" 2>"$ERROR_FILE")
HOOK_STATUS=$?
# A refresh stopped at its budget may already have printed part of a
# capsule, and a cut capsule reads like a whole one. It is dropped, and the
# turn is told below that memory was not consulted.
[ "$HOOK_STATUS" -eq 124 ] && REPORT=""
```

Set `BUDGET_SECONDS="${CONTEXT_HOOK_BUDGET:-5}"` before the call; any budget the person sets is honoured, and the prompt reaches the CLI whole, whatever its length. Define the prompt relay once before use, with both forwarding arms intact:

```bash
run() {
  if command -v timeout >/dev/null 2>&1; then
    timeout "$BUDGET_SECONDS" "$@"
  else
    "$@"
  fi
}
```

Every hook that runs the memory runtime - both working-memory hooks, `local-context.sh` and `subagent-dispatch.sh` - resolves `CONTEXT_CLI` from `ROOT_DIR` with exactly these two lines. The loader runs `context.py` as `__main__` in the same process from its cached bytecode (run as a script, the 8,300-line `context.py` is compiled again on every start), and the fallback keeps the hook working on an older runtime without the loader:

```bash
CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context_cli.py"
[ -f "$CONTEXT_CLI" ] || CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"
```

Never assign `CONTEXT_CLI` any other way: a path that names no file makes the hook's `[ -f "$CONTEXT_CLI" ] || exit 0` skip memory on every prompt without a word. A hook generated before the loader, with the single line `CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"`, stays valid.

Every hook that runs Python - both working-memory hooks, `context-continuity.sh`, `local-context.sh`, `subagent-dispatch.sh`, and the validators where they decode a call with Python - finds it with this function and calls it as `"${PYTHON[@]}"`, never as a bare `python3`: native Windows (Git Bash) often has only `python` or `py`, or a Microsoft Store stub named `python3`, and a hook naming only `python3` reads and writes no memory there. The working-memory hooks and `context-continuity.sh` call `resolve_python || exit 0` before their runtime check; `local-context.sh` and `subagent-dispatch.sh` call it once and test `[ -n "${PYTHON[0]:-}" ]`; the validators call it once, only where neither `jq` nor `php` is. Python snippets run isolated (`"${PYTHON[@]}" -I -c`) - in the session's folder `-c` imported a project's own `json.py` - read `sys.stdin.buffer` and write `sys.stdout.buffer` as UTF-8: a Windows text stream reads by the ANSI code page and writes `\n` as `\r\n`, and a validator that fails there lets the call through; Cursor's prompt hook also calls it where it retires its rule early. A hook generated earlier, calling `python3`, stays valid. Where Python has another name, `local-context.sh` says ``Python here is `<name>`, not python3: run the memory CLI commands the policy names with `<name>` in place of `python3`.`` and step 9 allows the CLI under `python` and `py -3` too.

```bash
resolve_python() {
  PYTHON=()
  if [ "${OS:-}" != "Windows_NT" ] && command -v python3 > /dev/null 2>&1; then
    PYTHON=(python3)
    return 0
  fi
  local name path remembered="${TMPDIR:-/tmp}/ai-accelerator-python-${UID:-0}"
  if [ "${OS:-}" = "Windows_NT" ] && [ -f "$remembered" ] && [ ! -L "$remembered" ] \
      && IFS=$'\t' read -r name path < "$remembered" && [ -n "$path" ]; then
    case "$name" in
      python|py|python3)
        if hash "$name" 2> /dev/null && [ "${BASH_CMDS[$name]:-}" = "$path" ]; then
          PYTHON=("$name")
          [ "$name" = py ] && PYTHON+=(-3)
          return 0
        fi
        ;;
    esac
  fi
  for name in python py python3; do
    PYTHON=("$name")
    [ "$name" = py ] && PYTHON+=(-3)
    if "${PYTHON[@]}" -I -S -c 'import sys; sys.exit(sys.version_info < (3, 9))' > /dev/null 2>&1; then
      if [ "${OS:-}" = "Windows_NT" ] && [ ! -L "$remembered" ] && hash "$name" 2> /dev/null; then
        printf '%s\t%s\n' "$name" "${BASH_CMDS[$name]:-}" > "$remembered" 2> /dev/null
      fi
      return 0
    fi
  done
  PYTHON=()
  return 1
}
```

Cursor turn reads use this separate inferred-only array after resolving
`TASK_ID` with the same fallback above. Keep markers off `turn`, message
dispatch and native `cursor-rule` maintenance. Preserve existing empty-task
and readiness guards before either read. Both arms expand the quoted array:

```bash
CAPSULE_IDENTITY_ARGUMENTS=()
if [ -z "${CONTEXT_TASK_ID:-}" ] && [ -n "$TASK_ID" ]; then
  CAPSULE_IDENTITY_ARGUMENTS=(--expected-branch "$TASK_ID")
fi
if command -v timeout >/dev/null 2>&1; then
  CAPSULE=$(timeout "$BUDGET_SECONDS" "${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
else
  CAPSULE=$("${PYTHON[@]}" "$CONTEXT_CLI" hook-context --host cursor --delivery-profile cursor-rule --task-id "$TASK_ID" "${CAPSULE_IDENTITY_ARGUMENTS[@]}" 2>/dev/null)
fi
```

Cursor session reads in `local-context.sh` use `CAPSULE_TASK_ID` throughout
instead of resolved `TASK_ID`, and `CAPSULE_BUDGET_SECONDS` for the budget;
the environment selector remains `CONTEXT_TASK_ID`. Do not overwrite resolved
identities or argument arrays later. Comments, quoted programs and heredocs
cannot substitute for live callers. The gate is structural lint, not proof of
arbitrary control flow, sourced code, eval, runtime redefinition, atomic branch
epochs or a whole-hook deadline. Never execute unchecked generated hooks in
the structural verifier to fill those gaps.

## Process

1. **Read the profile.** Confirm selected editions (section 1). Build the context payload from section 2 (framework/version, test/lint/static tools). Build the risk list ONLY from what is present: section 5 (containers/CI/IaC), section 4 (integrations), section 6 (secrets/destructive ops).
2. **Build `bash-validator.sh` from the generator's own `.claude/hooks/bash-validator.sh`; author only its rules.** That file holds the shell parser every shipped edition runs - it finds a command inside chains, `$(...)`, backquotes, heredocs, `sh -c`, `eval`, wrappers and launchers - and the generic rules (force push, hard reset, hook bypass, broad `rm -r`, destructive SQL, printing `.env`). Never write a validator from scratch. Copy three parts of it: everything above the section (the payload decoding), changing only the header comment to name the target; every byte from the line `# >>> bash-validator generic section >>>` through the line `# <<< bash-validator generic section <<<`, unchanged; and the call that ends the file (`bv_validate "$cmd"`, the block message with `exit 2`, `exit 0`). Between the call's `fi` and that `exit 0`, copy the editions' command repetition guard from this skill's `assets/bash-validator-repetition-guard.sh`, every byte from `# >>> bash-validator repetition guard >>>` through `# <<< bash-validator repetition guard <<<`: it counts each command that passed the rules per session and exact string, warns at the sixth run, refuses the twelfth, leaves a standalone status query uncounted, and starts a count over after an edit `loop-detection.sh` recorded (step 5). Between the closing marker and the call, author the target's settings and rules in the rule syntax the section documents: `BV_HOST` (the edition, `claude`, `cursor` or `codex`: it names the guard's counter directory, and on `cursor` the guard warns on stderr with exit 1, as Cursor shows the agent nothing about a command it lets run), `BV_CONSOLE_ENTRIES` (the target's console entry points, `''` without one), `BV_CONSOLE_PHP_SCRIPTS` (`1` when it also runs console commands through other `php <script>` files, else `0`), `BV_CONSOLE_VALUES` and `BV_CONSOLE_SHORT_VALUES` (that console's options whose value is the next word), and `BV_FRAMEWORK_RULES`. Into `BV_FRAMEWORK_RULES` take the generator's own framework lines (the four editions' rules for Artisan, Symfony console, Doctrine, Phinx and WP-CLI) only for a detected tool - `php artisan migrate:fresh` / `migrate:rollback` ONLY if that framework's migration tooling was detected - and add a rule per detected risk: `kubectl delete` ONLY if Kubernetes was detected in section 5, `terraform destroy` ONLY if Terraform was detected in section 5. A subcommand that may follow global options needs `has:` (`"argv|kubectl||has:delete|destructive command - kubectl delete"` also stops `kubectl -n prod delete`). Never author a rule for a tool the target does not use. The hook runs under `set -u`, so an unset variable ends it with exit 1, which lets the command through: assign `BV_CONSOLE_ENTRIES` and `BV_CONSOLE_PHP_SCRIPTS` even when empty, and copy a framework line together with the helper list it expands (`BV_LARAVEL_SECRET_CONFIG`, `BV_WP_SECRETS`). The copied section is not trimmed to the profile: each generic rule names its own command and fires only when an agent runs it, so one for a tool the target lacks never fires. bootstrap-verifier compares the section and the guard byte for byte with their sources and requires below the section the two settings, an assignment before every variable expanded, the `bv_validate` call and its `exit 2`, then `BV_HOST` naming the edition, no `exit` before the guard and only `exit 0` after it.
3. **Author `local-context.sh`** to echo the target's real stack summary at session start (framework, version, key command lines), then run `status --json` through the runtime entry (`CONTEXT_CLI`, above) under `timeout "${CONTEXT_HOOK_BUDGET:-5}"` when Python, the CLI, and `timeout` are available. Parse only `automatic_memory.status`, `reason`, and `remediation` with a dependency-free `"${PYTHON[@]}" -c` filter; print at most one bounded warning plus one remediation line when status is `retrieval-only` or `degraded`, print nothing when it is `active`, and discard malformed/failed/timed-out status output. This readiness warning is diagnostic only: the hook MUST always exit 0.
4. **Author `file-naming-validator.sh`** to enforce the target's naming policy (task/spec prefixes, zero-padded task dirs) as stated in the generated policy.
5. **Author `loop-detection.sh`** to detect repeated identical tool calls/edits and warn (warn at 7, block at 10). Count every file an edit names: `file_path` (Cursor and Codex also `path`), `notebook_path`, and each `*** Add File:`/`*** Update File:`/`*** Move to:` line of a Codex `apply_patch` (`\n`-escaped in the JSON). Key each counter by the payload's `session_id` (Cursor `conversation_id`), letters and digits only, in the per-user, per-repository directory the repetition guard of step 2 counts in: copy the guard's four lines that assign `REPO_ROOT`, `REPO_KEY`, `TRACK_BASE` and `TRACK_DIR` from `assets/bash-validator-repetition-guard.sh`, the edition's name in place of `$BV_HOST`, and on every counted edit write the file's count to `"$TRACK_DIR/edit-$SESSION_KEY-<hash of its path>"`. The guard keeps its `cmd-` counters beside them and starts a command's count over when an `edit-*` file is newer than that command's counter; without them every edit-and-rerun cycle is refused at its twelfth run. bootstrap-verifier requires the four lines and an `edit-` counter in `$TRACK_DIR`. Its name is predictable: create it with `mkdir -m 700`, count nothing when it is a symbolic link or not yours (`[ -L ]`, `[ -O ]`), never touch a counter that is a link, and accept a count only as digits read in base 10 (`$((10#$COUNT))`; a planted `a[$(cmd)]` would run `cmd`). Warn where the model reads it - Claude and Codex: exit 0 with `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"..."}}`; Cursor's `afterFileEdit` has no such channel, so print and exit 1. Block on stderr with exit 2. `local-context.sh`, computing the directory with the same four lines, deletes only its own session's `edit-` and `cmd-` counters and any untouched for a day.
6. **Author the working-memory pair** as thin, always-exit-0 callers of the runtime entry (`CONTEXT_CLI`: `memory-bank/scripts/context_cli.py`, falling back to `memory-bank/scripts/context.py`; resolve `ROOT_DIR` from the script's own location - `$(dirname "${BASH_SOURCE[0]}")/../..` - never from `$PWD`):
   - `working-memory-read.sh` (prompt time; no task writes): keep the Python/CLI, attached-root and `CONTEXT_MEMORY_RECOVERY=1` guards. Stand down for `CONTEXT_CAPSULE_DELIVERED=1`. Read stdin JSON using one `"${PYTHON[@]}" -c` program, never sed: retain the prompt's trailing newlines; cap it to 24,000 characters (first 16,000 + last 8,000), since it travels in argv. Accept session IDs only matching `[A-Za-z0-9._:-]{1,128}` and transcript paths only when absolute, at most 4096 characters and without newline. Resolve task identity from nonempty `CONTEXT_TASK_ID`, then the Git branch/rebase resolver above; ordinary detached HEAD supplies none. Call the canonical `refresh` block above directly on `"$CONTEXT_CLI"`. Add query/task/ephemeral/sanitize flags only when query and task are nonempty; append session and, with it, transcript flags only when validated. The runtime distills and sanitizes the raw query; never persist raw prompt text. Bound calls with `timeout "${CONTEXT_HOOK_BUDGET:-5}"`, stderr in private `mktemp`. Discard status-124 output before publication: a partial capsule must never look complete. Preserve the layer report on exit 1. Emit admitted text without a banner; empty successful output stays silent. Failure prints the exit status or exceeded budget, a final stderr line capped at 160 characters, and `Working memory was NOT consulted this turn. Read the canonical sources directly.` Installed projects may suggest `${PYTHON[*]} memory-bank/scripts/context.py index` after repeated timeouts. Always exit 0; Cursor emits only `{"continue": true}` and uses its proven rule/capsule channel.
   - `working-memory-write.sh` (the write half; runs at turn end, where a real delta exists): same Python/CLI/`timeout` guards; require a task ID (a non-empty `$CONTEXT_TASK_ID`, otherwise the symbolic Git branch or `rebased_branch` during a rebase) and exit 0 silently without one; read the Stop payload's `transcript_path` with the read hook's validation (a string of at most 4,096 characters, no newline, absolute) in the same Python call that reads `stop_hook_active`; run `"${PYTHON[@]}" "$CONTEXT_CLI" turn --task-id "$TASK_ID" --flush-after "${CONTEXT_FLUSH_AFTER:-5}"`, adding `--transcript "$TRANSCRIPT"` when that path is an existing file, with all output discarded; always `exit 0` - a failed checkpoint must never surface as a turn error. The transcript gives the runtime the files the turn read or changed - tool-call paths and the shell words that name an existing project file - for the next session's `recent files` line; it keeps project-relative paths only, never commands, tool results or file contents, and an unreadable transcript records nothing. An explicit identity outside Git does not make checkpointing available: `turn` requires Git porcelain changed-path metadata. Working-tree contents never reach the buffer.
   - **Cursor read path (Cursor copies only):** `beforeSubmitPrompt` must always answer `{"continue": true}` through an EXIT trap. It cannot inject context, so direct-host delivery uses the reserved generated `.cursor/rules/working-memory.mdc` slot. Never publish or retire that rule directly in shell. Delegate exclusively to `context.py cursor-rule --action write --phase prompt --task-id "$TASK_ID" --report-json --diagnostic`, piping the actual `refresh --host cursor --json` report on stdin. Do not use `--session-id`: the whole rule is resent. At turn end use `--action prompt-held` to retain a delivered prompt for the same task - a question whose own diagnostics are discarded, since `preserved` there only means another rule is in place for the write that follows; otherwise pipe fresh `hook-context --host cursor` output into `--action write --phase turn`. Session start uses `--phase session`. Failed/empty renders use native `--action keep`; no task, disallowed readiness or `CONTEXT_CAPSULE_DELIVERED=1` uses native `cursor-rule-retire`. Attached shared clones stand down. Native maintenance owns `alwaysApply: true`, the reference-only warning, byte equality, locking, parent/link checks, publication and advisory receipt reconciliation. Give maintenance three seconds where `timeout` exists; discard its JSON stdout except the held-state check, retain fixed stderr diagnostics and always exit 0. Timeout/nonzero exit prints only `Cursor memory rule was not updated: maintenance_unavailable.`; never echo capsule or arbitrary error content. The root-ignore contribution explicitly registers `.cursor/rules/working-memory.mdc`; shared-file composition handles it. Exact received bytes reconcile after copy/touch/cache loss; the whole legacy machine grammar recovers only in this registered untracked slot. A file that keeps the auto-rendered header but was edited is copied to `memory-bank/local/cursor-rule-edits/` before the slot is written again; a file without that header, tracked files and foreign links stay intact. Use current memory-seed helpers, including the supported Windows backend; do not regenerate an older shell temporary-file replacement or permanent inode ownership gate. Claude and Codex use their ordinary stdout capsule and MUST NOT maintain this rule. Native loading order and token savings need separate measurement.
   - **Cursor runs the Claude hooks too** (third-party hooks are on by default, and every matching hook from every source runs). The Claude copies of `working-memory-read.sh`, `working-memory-write.sh` and `context-continuity.sh` read their stdin payload and exit 0 when it carries `cursor_version` (which Claude Code never sends; an environment variable could leak into a Claude Code session started from Cursor's terminal) and the project has `.cursor/hooks.json`; the Cursor and Codex copies carry no such check. Running both doubled every checkpoint and wrote retrievals nobody received.
   - **`context-continuity.sh` (chat snapshots; one script, three events, no argument):** same `CONTEXT_MEMORY_RECOVERY`, Python (`resolve_python`), `timeout` and `ROOT_DIR` handling, silent when `memory-bank/scripts/context_continuity.py` is missing; pipe stdin to `context_continuity.py --root "$STATE_DIR" --host <claude|codex|cursor> --event hook`, stderr discarded; always `exit 0`. `STATE_DIR` is `ROOT_DIR`, or `ACCELERATOR_STATE_DIR` in attached mode (`ACCELERATOR_HOME` resolves to `ROOT_DIR`, state and project dirs absolute). The runtime reads `hook_event_name`: a session start delivers only a merge prepared for that new task (again on its resume or compaction); a prompt or final answer is appended to the chat's snapshot. The runtime itself honours `CONTEXT_CONTINUITY_DISABLED=1` (both halves) and `CONTEXT_CONTINUITY_RESTORE_DISABLED=1` (delivery).
   - **Native result capture:** pipe prompt JSON to bounded `context.py native-prompt --host <host> --task-id "$TASK_ID"` before refresh; pipe Stop JSON to `native-stop` after checkpoint. Capture output and emit only after exit 0; never emit a timed-out prefix. Cursor warnings belong in its capsule, never mixed with response JSON. These commands use memory-seed's required `native_results.py` to request one same-agent `native-result`; no transcript adoption, new model process or task completion. Set `loop_limit: 1` on Cursor's working-memory-write Stop entry. Keep the canonical Cursor rule-maintenance block.
7. **Author `subagent-gate.sh` per edition** (tool-owned variants, one contract each):
   - **Claude variant:** self-filter to `tool_name` Agent/Task; extract `tool_input.subagent_type` (jq/php/Python fallback chain); allow only names found as frontmatter `name:` in the target's `.claude/agents/*.md`; deny everything else with a reason on stderr and exit 2; fail open (exit 0) on missing payload fields or an unreadable roster.
   - **Cursor variant:** `subagentStart` contract - read top-level `subagent_type`, allowlist against `.cursor/agents/*.md` frontmatter names, answer `{"permission":"allow"}` or `{"permission":"deny","user_message":"..."}` on stdout, always exit 0.
   - **Codex variant:** deny the multi-agent tool family (`spawn_agent`, `Agent`, `send_input`, `resume_agent`, `wait_agent`, `close_agent`) unconditionally with stderr + exit 2 - the Codex edition delegates through skills and ships no agents.
   - **Write serialization (Claude and Cursor variants):** when the requested agent's frontmatter carries `writes: true`, take a per-repository lock file before allowing the spawn and deny a second write-capable agent while it is held. Give the lock a TTL (default 30 minutes, overridable by env) so a crashed run cannot deadlock delegation, validate that TTL is numeric before use, and serialize the check-and-take with `flock` where available - parallel spawns arrive in one message and would otherwise both observe an unlocked state. Read the roster from the FIRST frontmatter block only (a `---` rule inside an agent's body must not be able to declare `writes:`). A missing roster, a missing JSON extractor, or an unwritable lock path fails OPEN with a warning on stderr; only a recognized built-in fails closed.
8. **Author `subagent-dispatch.sh`** (one script, all editions): read the stop payload, take the agent name from `agent_type` or `subagent_type` with a structure-aware parser (never a greedy `sed` - a key quoted inside the final message must not shadow the real field), append the completion to the task's channel with `context.py msg-dispatch --task-id <ID> --agent <name> --event complete --note "<one sanitized line>"`, and delete the write lock if this agent holds it. Resolve identity from a non-empty `CONTEXT_TASK_ID`, otherwise from a symbolic branch name; never use the literal detached-HEAD token `HEAD` as a task ID. Always `exit 0`, and degrade silently without Python (`resolve_python`), without the CLI, or without a resolvable task - an observer must never break a turn. A completion write that fails for any other reason MUST be reported: unless the error says the task was not found or the project is in lightweight mode, print one line on stdout and stderr with the agent, the exit status and the last non-blank stderr line (at most 160 characters), saying to record it with `msg-dispatch --event complete` before trusting `msg-read`. Claude Code shows a `SubagentStop` hook's output to no conversation the orchestrator reads, so for `hook_event_name` `SubagentStop` also append the line to the ignored `memory-bank/local/unrecorded-completions` (no symbolic link on the way); run with `hook_event_name` `PostToolUse` (wired on `Agent|Task`, step 9), the script records and releases nothing: it moves that file aside, prints its last 20 lines as `{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"..."}}` (`json.dumps`) and deletes it.
9. **Wire per edition** (identical scripts, different config):
   - **Claude** -> `.claude/settings.json`: `SessionStart` (local-context, timeout 10; context-continuity, timeout 8), `UserPromptSubmit` (working-memory-read, timeout 8; context-continuity, timeout 8), `Stop` (working-memory-write, timeout 8; context-continuity, timeout 8), `PreToolUse` (file-naming-validator on `Write|Edit`, bash-validator on `Bash`, subagent-gate on `Agent|Task`, timeout 5 each), `PostToolUse` (loop-detection on `Edit|Write|MultiEdit|NotebookEdit`, and subagent-dispatch - the relay of step 8 - on `Agent|Task`, timeout 5 each), `SubagentStop` (subagent-dispatch, timeout 8), each with a `matcher` where noted and a `timeout` in **seconds**. The same file also carries the configuration half of the gate: `permissions.deny` entries `Agent(Explore)`, `Agent(Plan)`, `Agent(general-purpose)`, `Agent(claude)`, `Agent(statusline-setup)`, `Agent(claude-code-guide)`, plus the `permissions.allow` entries `Bash(python3 memory-bank/scripts/context.py:*)`, `Bash(python memory-bank/scripts/context.py:*)` and `Bash(py -3 memory-bank/scripts/context.py:*)` - the policy tells the agent to read and write memory through that CLI, and without the rule Claude Code asks first in an interactive session and refuses outright under `-p`; native Windows often has Python only as `python` or `py -3` - plus `env` keys `CLAUDE_CODE_DISABLE_EXPLORE_PLAN_AGENTS: "1"` and `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH: "1"` - the deny list blocks today's built-ins, the hook covers built-ins added later, and the depth cap keeps orchestration in the main conversation where checkpoints and permissions actually apply. The command form is the script path anchored at the project root through Claude Code's documented placeholder, double-quoted as its shell form requires: `"${CLAUDE_PROJECT_DIR}"/.claude/hooks/<script>.sh`. Claude Code runs a hook in the session's current directory, which follows every `cd` the agent makes, so a bare `.claude/hooks/<script>.sh` exits 127 from any subdirectory - and every exit other than 2 is non-blocking, so the guard silently stops guarding. Claude Code pipes the tool-input JSON to the hook on stdin, so an `echo '$TOOL_INPUT' | <script>` wrapper feeds the hook the literal string `$TOOL_INPUT` instead of the payload and silently disables it.
   - **Cursor** -> `.cursor/hooks.json`: `version: 1`, camelCase events, timeouts in **seconds**: `sessionStart` (local-context, 15; context-continuity, 8), `afterAgentResponse` (context-continuity, 8), `subagentStop` (subagent-dispatch, 8), `subagentStart` (subagent-gate, 5, with `failClosed: true` - Cursor has no setting that disables its built-in subagents, so this hook is the only enforcement point), `beforeShellExecution` (bash-validator, 5), `afterFileEdit` (file-naming-validator, 5; loop-detection, 5), `stop` (working-memory-write, 20). These budgets cover every inner call the hook makes, each separately `timeout`-bounded: the session banner plus the capsule render (15), and on Stop the turn flush, the held-prompt check, the capsule render and the rule write (5 + 3 + 5 + 3, so 20). Commands are bare script paths, `.cursor/hooks/<script>.sh` - Cursor runs project hooks from the project root, so the relative path always resolves - with no `bash`/`sh` prefix and no shell wrapper. `beforeSubmitPrompt` (working-memory-read, 10 - the refresh's 5 plus the rule write's 3 and interpreter starts; context-continuity, 8 - one array, never a second `beforeSubmitPrompt` key) carries the read half: the Cursor copy renders the capsule for the prompt into the rule from step 6. A `.cursor/cli.json` with `{"permissions": {"allow": ["Shell(python3:memory-bank/scripts/context.py*)", "Shell(python:memory-bank/scripts/context.py*)"], "deny": []}}` lets the Cursor CLI run the memory CLI the policy names without a prompt. Declare `.cursor/rules/working-memory.mdc` in the task-scoped root-ignore requirements (the rendered rule is transient local state), and record the mechanism in the log and the hooks README.
   - **Codex** -> `.codex/hooks.json`: Claude-style event names, NO `matcher`/`timeout` fields: `SessionStart` (local-context; context-continuity, with `additionalContextLimit: 6000`), `UserPromptSubmit` (working-memory-read, with `additionalContextLimit: 4000`; context-continuity), `Stop` (working-memory-write; context-continuity), `PreToolUse` (bash-validator, file-naming-validator, subagent-gate - the gate self-filters to multi-agent tool names), `PostToolUse` (loop-detection). Every command is this one root-finding launcher, verbatim, with only the trailing script name changing: `sh -c 'd=$(pwd); until [ -f "$d/.codex/hooks.json" ]; do [ -n "$d" ] || { echo "$1: no .codex/hooks.json at or above the working directory" >&2; exit 127; }; d=${d%/*}; done; exec "$d/.codex/hooks/$1"' sh <script>.sh`. Codex runs hooks in the session's cwd through the user's login shell (`$SHELL -lc`) and exports no project-root variable, so a bare `.codex/hooks/<script>.sh` exits 127 whenever Codex starts in a subdirectory. The launcher walks from the cwd towards `/` to the nearest directory holding `.codex/hooks.json` - the project that declared the hook - and execs only that project's `.codex/hooks/<script>.sh`, which also covers a project that is not a Git repository and one nested inside a larger repository (where the Git top level is the wrong root). It never looks past that project: a script missing there exits 127 instead of running a same-named script from an ancestor (`~/.codex/hooks/` is Codex's own user-hook directory, `/tmp` is world-writable). Handing the walk to `sh` keeps it independent of the user's shell (fish, zsh, bash). No other wrapper or interpreter prefix. `subagent-dispatch.sh` ships for parity but stays UNREGISTERED here: multi-agent is off, so there are no subagent stops to observe. Plus `.codex/config.toml` containing a top-level `project_doc_max_bytes = 131072` (Codex reads the AGENTS.md chain only up to 32 KiB by default, and the policy block, coming last in a project that has its own AGENTS.md, is what gets cut), `[features]` with `hooks = true` and `multi_agent = false`, and an `[agents]` table with `enabled = false` - the config switches are the hard off-story for Codex built-in subagents; the hook is the backstop for CLI/model combinations that ignore the flags.
   - **Every edition:** `context-continuity.sh` follows an event's existing hooks and takes no argument - the payload names the event.
10. **Declare root-ignore requirements.** Write
    `tasks/TASK-{N}/gitignore-requirements/hook-forge.json` with the exact
    requirements `[".context-handoff/", ".cursor/rules/working-memory.mdc"]` when Cursor is selected, otherwise `[".context-handoff/"]`.
    Never read or mutate the target root `.gitignore`.
11. **Log** every hook, its wiring per edition, the section 4/5/6 evidence that authorized each danger rule, the Cursor read-path exception, the three gate variants, and where the dispatch observer is wired versus shipped unregistered.

## Output Template

```markdown
# Hook Forge Complete: [target_name]

**Editions:** [selected]
**Hooks per edition:** local-context.sh, bash-validator.sh, file-naming-validator.sh, loop-detection.sh, working-memory-read.sh (Cursor: beforeSubmitPrompt, renders the rule), working-memory-write.sh, context-continuity.sh (one script, no argument), subagent-gate.sh (tool-owned variant per edition), subagent-dispatch.sh (unregistered on Codex)

## Wiring
- claude: .claude/settings.json (second timeouts, matchers, "${CLAUDE_PROJECT_DIR}"-anchored script paths; UserPromptSubmit -> read, Stop -> write; SessionStart/UserPromptSubmit/Stop -> context-continuity; PreToolUse Agent|Task -> subagent-gate, SubagentStop -> subagent-dispatch, PostToolUse Edit|Write|MultiEdit|NotebookEdit -> loop-detection, PostToolUse Agent|Task -> subagent-dispatch relay, + Agent(...) deny rules + built-in-agent env keys)
- cursor: .cursor/hooks.json (version 1, second timeouts; subagentStart -> subagent-gate with failClosed, subagentStop -> subagent-dispatch; stop -> write + capsule render into .cursor/rules/working-memory.mdc, sessionStart re-renders; beforeSubmitPrompt -> read; sessionStart/beforeSubmitPrompt/afterAgentResponse -> context-continuity; rule and .context-handoff/ gitignored)
- codex: .codex/hooks.json + .codex/config.toml (root-finding launcher per hook; [features] hooks=true multi_agent=false, [agents] enabled=false; UserPromptSubmit -> read, Stop -> write, SessionStart/UserPromptSubmit/Stop -> context-continuity, PreToolUse -> subagent-gate; subagent-dispatch present but unregistered)

## Danger rules (evidence-gated)
- [rule -> profile section 4/5/6 citation]

## Checks
- bash -n: [pass] | chmod +x: [applied] | bash-validator generic section and repetition guard: [byte-identical to their sources; BV_HOST = edition] | runtime path contracts: memory-bank/scripts/context_cli.py (falling back to context.py) and memory-bank/scripts/context_continuity.py

## Log
tasks/TASK-{N}/hook-forge-log.md

## Next
memory-seed; policy-forge/skill-forge if not already run.
```

## Guardrails

- MUST make local-context, both working-memory hooks and context-continuity exit without work when `CONTEXT_MEMORY_RECOVERY=1`; a draft-only recovery must not retrieve, checkpoint, or render another capsule.
- MUST generate hooks ONLY for the selected editions, each wired through its own mechanism (settings.json / hooks.json / hooks.json+config.toml).
- MUST preserve memory-seed's managed Memory MCP block in `.codex/config.toml`; project MCP registration is emitted by the copied `mcp_config.py` for selected clients, using portable paths and detected Python. Do not replace other MCP servers or set client trust/approval.
- MUST build `bash-validator.sh` as step 2 says, never from scratch and never with the generic section or the repetition guard edited or trimmed; MUST author a rule for a destructive command ONLY when the profile confirms that tool is present (no `terraform destroy`/`kubectl delete`/migration-reset rules without section 5/2 evidence).
- MUST ensure every script passes `bash -n` and is `chmod +x`.
- MUST use second timeouts for Claude and Cursor, and no matcher/timeout for Codex.
- MUST wire every hook in exactly its edition's form from step 9: Claude `"${CLAUDE_PROJECT_DIR}"/.claude/hooks/<script>.sh`, Cursor bare `.cursor/hooks/<script>.sh`, Codex the verbatim root-finding launcher ending in `sh <script>.sh`. MUST NOT wire a bare relative path on Claude or Codex: both run hooks from whatever directory the session is in, the shell exits 127 from a subdirectory, and that exit is non-blocking, so the guard fails open. MUST NOT add any other `bash`/`sh` interpreter prefix or wrapper, trailing shell syntax (`|| true`, `; exit 0`) or quoting of the whole path, and never `echo '$TOOL_INPUT' | <script>`: the payload arrives on stdin, a trailing `|| true` swallows the blocking exit 2, and a wrapper or quoted path can hide a missing hook. bootstrap-verifier matches every command that names a hook against these three forms in full and rejects anything else, and it requires every hook step 9 registers to be wired in its form.
- MUST pass every blocked-pattern regex to `grep` after `--`, and MUST decode the tool-input JSON (jq/php/Python, as above) rather than scraping it with `sed`: a pattern such as `--no-verify` is otherwise read as a grep option, and a `"[^"]*"` scrape truncates any command containing escaped quotes. Both failures exit 0 and silently allow the command.
- MUST read every hook's stdin with the bash builtin (`IFS= read -r -d '' INPUT || :`), never through an external `cat`: on a system without `cat` in PATH the payload is lost, and the validator then allows every command before its no-extractor warning can fire.
- MUST make all memory-related hook paths always exit 0. `local-context.sh` may emit only the bounded readiness guidance above. Missing Python, CLI, task ID or Git metadata stay silent everywhere, as do write and context-continuity failures; a failed or timed-out read refresh and a failed dispatch completion write MUST print the one-line report of steps 6 and 8. Context tooling must never block a prompt or surface as a turn error. An explicit `CONTEXT_TASK_ID` outside Git enables task-aware retrieval/dispatch only; it MUST NOT be described as enabling `turn` checkpoint writes, which require Git changed-path metadata.
- MUST call the runtime through exactly the two-line entry of the identity contract - `memory-bank/scripts/context_cli.py`, falling back to `memory-bank/scripts/context.py`, both relative to the target root (the paths `memory-seed` creates); MUST NOT assign `CONTEXT_CLI` any other way (bootstrap-verifier rejects it in the memory read hooks), inline memory logic into the hooks themselves or write task state from the read hook.
- MUST generate all three Cursor read/turn/session paths as thin native `cursor-rule` callers from step 6, with phases `prompt`, `turn`, `session`, fixed diagnostics and current-task hold/retire behavior. Cursor `refresh` and `hook-context` MUST pass `--delivery-profile cursor-rule` without `--session-id` or `--transcript`; use the exact native mirror templates, including the scoped query-only continuation and stable render. Claude/Codex keep the ordinary profile. Declare the reserved rule in root-ignore requirements. Do not inline frontmatter, temporary publication, inode receipts or legacy header-only deletion in shell. Claude/Codex copies MUST NOT maintain the rule.
- MUST run chat snapshots through one unargumented `context-continuity.sh` calling exactly `memory-bank/scripts/context_continuity.py --host <edition> --event hook`, wired on the three events of step 9; MUST NOT read private account/session stores, keep hidden reasoning, deliver anything at session start but a prepared merge, or write Project Brain, Memory Bank or SQLite.
- MUST NOT read or modify the root `.gitignore`; only
  `infra-generate`/`infra-update` compose approved shared-file changes.
- MUST generate `subagent-dispatch.sh` in every selected edition, wired to `SubagentStop` and, as the relay, to `PostToolUse` on `Agent|Task` (Claude) and to `subagentStop` (Cursor), and left unregistered on Codex, always exiting 0; it is what releases the write lock, so omitting it strands every write-capable agent until the lock's TTL expires.
- MUST generate `subagent-gate.sh` as three TOOL-OWNED variants (Claude PreToolUse exit codes, Cursor subagentStart permission JSON, Codex spawn_agent-family deny) - never one shared script - and MUST pair the hook with its configuration half: the `Agent(...)` deny list and built-in-agent env keys for Claude, `failClosed: true` for Cursor, `multi_agent = false` plus `[agents] enabled = false` for Codex. The Claude and Cursor gates read the roster from that edition's generated agents' frontmatter `name:` values; a missing or unreadable roster fails OPEN (delegation must not brick), while a recognized built-in fails CLOSED.
- MUST NOT print or log any secret or credential value from the target.
- MUST NOT translate section 8 business invariants, permissions, approvals, or lifecycle rules into brittle shell/text-matching hooks. Behavioral rules belong in policy, skills, tests, review, and memory; hooks enforce only deterministic tool/command/file events.

## Final Output

Return the per-edition hook paths, the wiring file per edition, the evidence-gated danger rules, the `bash -n`/exec-bit results, the log path, and the next step (`memory-seed`).

The lexical contract accepts only LF physical records, code/double-quote
continuations and simple quoted literal heredocs (one at a time; exact
delimiter word, optional tab stripping). Unquoted/concatenated delimiters,
arithmetic-shift ambiguity and unsupported quoted tails fail closed. Both
resolved task and explicit selector writes invalidate the contract. Alongside
the preferred ROOT_DIR/symbolic-ref fragment, the gate accepts the finite
installed PROJECT_DIR/branch-show-current fragment with its existing forwarding
relay, budget variable and guarded Cursor JSON append. Both profiles retain
the inferred-only marker. Cursor needs exactly one JSON append before dispatch;
other hosts cannot append it. These checks do not prove arbitrary shell code.

Redirection operators assembled across code continuations are unsupported.
The explicit selector may appear only in canonical fallback/guard lines;
named prefix increments and default-parameter assignments cannot rewrite
resolved task identities.


The prompt read has one budget, `CONTEXT_HOOK_BUDGET` (default 5 seconds), applied by `timeout` to the whole refresh: whatever value the person sets is honoured, and the prompt reaches the CLI whole, whatever its length. A refresh stopped at the budget (status 124) delivers nothing, never a cut capsule, and the turn is told memory was not consulted. Any other failure prints the CLI's own last stderr line, so the reader learns why. Do not route the read through a separate supervisor process: it costs an interpreter start on every prompt and takes part of the budget from the refresh itself. The runtime entry is not one: `context_cli.py` runs `context.py` in the same process and saves its compile instead of adding a start.
