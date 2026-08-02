---
name: hook-forge
description: Generate the target PHP project's enforcement hooks (local-context.sh, bash-validator.sh, file-naming-validator.sh, loop-detection.sh) and per-edition wiring, tuned to the target's real detected stack and risk surface from an approved Project Profile. Blocks only dangerous commands whose tools were actually detected. Use once profile-synthesizer has produced a profile. Triggers on "generate hooks", "forge the target's hooks", "wire hooks for editions".
phase: generation
flow-next: memory-seed
flow-alternatives: [policy-forge, skill-forge]
related: [infra-generate, policy-forge, skill-forge, agent-forge, command-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Hook Forge

## Overview

`hook-forge` writes the target project's deterministic guardrails: shell hooks that inject context, validate risky shell commands, enforce file-naming policy, and detect agent loops - plus the per-edition wiring that runs them. Every hook is tuned to the target's REAL stack and risk surface from the profile, not a template: the danger list in `bash-validator.sh` contains only commands whose underlying tools were actually detected. Hooks are generated ONLY for the selected editions, and each edition wires the identical scripts through its own configuration mechanism.

Consumes profile sections **1** (which editions), **2** (stack/tooling for context + naming rules), **4** (integrations that widen the risk surface), **5** (infra/ops - the only source that authorizes infra danger rules), and **6** (security - secrets/destructive-op posture).

## Generated File Naming Convention (MANDATORY)

For each selected edition, into the target write the four hook scripts under that edition's hooks dir (`.claude/hooks/`, `.cursor/hooks/`, `.codex/hooks/`): `local-context.sh`, `bash-validator.sh`, `file-naming-validator.sh`, `loop-detection.sh`. All scripts MUST pass `bash -n` and be `chmod +x`. Then write the edition-specific wiring file (below). Append a log to `tasks/TASK-{N}/hook-forge-log.md` listing every hook, its wiring, and the profile evidence backing each danger rule.

## Process

1. **Read the profile.** Confirm selected editions (section 1). Build the context payload from section 2 (framework/version, test/lint/static tools). Build the risk list ONLY from what is present: section 5 (containers/CI/IaC), section 4 (integrations), section 6 (secrets/destructive ops).
2. **Author `bash-validator.sh`** to block only detected risks. Include `php artisan migrate:fresh` / `migrate:rollback` ONLY if that framework's migration tooling was detected; include `kubectl delete` ONLY if Kubernetes was detected in section 5; include `terraform destroy` ONLY if Terraform was detected in section 5. Never block a tool the target does not use.
3. **Author `local-context.sh`** to echo the target's real stack summary at session start (framework, version, key command lines).
4. **Author `file-naming-validator.sh`** to enforce the target's naming policy (task/spec prefixes, zero-padded task dirs) as stated in the generated policy.
5. **Author `loop-detection.sh`** to detect repeated identical tool calls/edits and warn.
6. **Wire per edition** (identical scripts, different config):
   - **Claude** -> `.claude/settings.json`: `SessionStart` (local-context), `PreToolUse` (file-naming-validator on `Write|Edit`, bash-validator on `Bash`), `PostToolUse` (loop-detection on `Edit`), each with a `matcher` and a `timeout` in **seconds**. The command form is the bare script path (`.claude/hooks/<script>.sh`): Claude Code pipes the tool-input JSON to the hook on stdin, so an `echo '$TOOL_INPUT' | <script>` wrapper feeds the hook the literal string `$TOOL_INPUT` instead of the payload and silently disables it.
   - **Cursor** -> `.cursor/hooks.json`: `version: 1`, camelCase events `sessionStart`/`beforeShellExecution`/`afterFileEdit`, timeouts in **seconds**.
   - **Codex** -> `.codex/hooks.json`: Claude-style event names, NO `matcher`/`timeout` fields, plus `.codex/config.toml` containing `[features]` with `hooks = true`.
7. **Log** every hook, its wiring per edition, and the section 4/5/6 evidence that authorized each danger rule.

## Output Template

```markdown
# Hook Forge Complete: [target_name]

**Editions:** [selected]
**Hooks per edition:** local-context.sh, bash-validator.sh, file-naming-validator.sh, loop-detection.sh

## Wiring
- claude: .claude/settings.json (second timeouts, matchers, bare script paths)
- cursor: .cursor/hooks.json (version 1, second timeouts)
- codex: .codex/hooks.json + .codex/config.toml ([features] hooks=true)

## Danger rules (evidence-gated)
- [rule -> profile section 4/5/6 citation]

## Checks
- bash -n: [pass] | chmod +x: [applied]

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
- MUST wire Claude hooks as bare script paths, never as `echo '$TOOL_INPUT' | <script>`; the payload arrives on stdin.
- MUST pass every blocked-pattern regex to `grep` after `--`, and MUST decode the tool-input JSON (jq/php/python3) rather than scraping it with `sed`: a pattern such as `--no-verify` is otherwise read as a grep option, and a `"[^"]*"` scrape truncates any command containing escaped quotes. Both failures exit 0 and silently allow the command.
- MUST NOT print or log any secret or credential value from the target.
- MUST NOT translate section 8 business invariants, permissions, approvals, or lifecycle rules into brittle shell/text-matching hooks. Behavioral rules belong in policy, skills, tests, review, and memory; hooks enforce only deterministic tool/command/file events.

## Final Output

Return the per-edition hook paths, the wiring file per edition, the evidence-gated danger rules, the `bash -n`/exec-bit results, the log path, and the next step (`memory-seed`).
