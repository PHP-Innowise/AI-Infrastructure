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
   - **Claude** -> `.claude/settings.json`: `SessionStart` (local-context), `PreToolUse` (bash-validator + loop-detection), `PostToolUse` (file-naming-validator), each with a `matcher`, millisecond `timeout`, and command form `echo '$TOOL_INPUT' | <script>`.
   - **Cursor** -> `.cursor/hooks.json`: `version: 1`, camelCase events `sessionStart`/`beforeShellExecution`/`afterFileEdit`, timeouts in **seconds**.
   - **Codex** -> `.codex/hooks.json`: Claude-style event names, NO `matcher`/`timeout` fields, plus `.codex/config.toml` containing `[features]` with `hooks = true`.
7. **Log** every hook, its wiring per edition, and the section 4/5/6 evidence that authorized each danger rule.

## Output Template

```markdown
# Hook Forge Complete: [target_name]

**Editions:** [selected]
**Hooks per edition:** local-context.sh, bash-validator.sh, file-naming-validator.sh, loop-detection.sh

## Wiring
- claude: .claude/settings.json (ms timeouts, matchers)
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
- MUST use ms timeouts for Claude, second timeouts for Cursor, and no matcher/timeout for Codex.
- MUST NOT print or log any secret or credential value from the target.

## Final Output

Return the per-edition hook paths, the wiring file per edition, the evidence-gated danger rules, the `bash -n`/exec-bit results, the log path, and the next step (`memory-seed`).
