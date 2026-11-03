---
name: hook-forge
description: Generate the target PHP project's hooks - four enforcement hooks (local-context, bash-validator, file-naming-validator, loop-detection), the two working-memory hooks that automate the context-brain layer memory-seed installs, the chat-snapshot hook behind `context-load merge`, and the two subagent hooks (roster gate with write serialization, completion observer) - and per-edition wiring, tuned to the target's real detected stack and risk surface from an approved Project Profile. Blocks only dangerous commands whose tools were actually detected. Use once profile-synthesizer has produced a profile. Triggers on "generate hooks", "forge the target's hooks", "wire hooks for editions".
phase: generation
flow-next: memory-seed
flow-alternatives: [policy-forge, skill-forge]
related: [infra-generate, policy-forge, skill-forge, agent-forge, command-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Hook Forge

Use the generator's explicit bundled engine (`GENERATOR_NATIVE=<absolute-generator>/memory-bank/bin/infra-engine`, or `infra-engine.exe` on Windows). The native engine implements hook behavior. This forge emits only reviewed policy data, host wiring and compatibility entrypoints into task staging.

## Scope and authority

Read selected clients from profile section 1, confirmed framework/tool commands from section 2, integrations from section 4, infrastructure from section 5 and security posture from section 6. Do not invent a tool or denial from framework convention. The complete native hook selection must already be present in the report between `<!-- infra-hook-policy-selection-v1 -->` and `<!-- /infra-hook-policy-selection-v1 -->` before human review. Use the recorded SHA of those exact approved report bytes. Computing a digest does not approve a report; a missing block or changed report requires refreshed review.

The selection is bounded data, not executable policy. It declares `schema_version`, framework, approved tool IDs, literal command-prefix/option definitions and denial rules. Each rule refers to a declared tool and uses supported Rust matching/condition/reason enums. No detected tools permits empty arrays. Never enable ready-edition full framework tables as a fallback for an incomplete generated profile.

Behavioral domain permissions, approvals, lifecycles and invariants stay in policy, skills, tests and memory. Hooks enforce deterministic tool, command and file events only.

## Native outputs

For only the selected clients, stage the appropriate configuration and the nine compatibility filenames under `.<host>/hooks/`:

| Role / filename stem | Native responsibility |
| --- | --- |
| local-context | Stack/session orientation, bounded memory readiness diagnostics and registered subfolder/worktree dispatch |
| bash-validator | Deterministic native command analysis using the approved generated policy |
| file-naming-validator | Host file-event naming checks |
| loop-detection | Session-scoped repetition warning/block behavior |
| working-memory-read | Prompt identity, bounded refresh, complete capsule delivery and native-result arming |
| working-memory-write | Turn change buffering/checkpoint and the host's capsule/rule maintenance |
| context-continuity | Visible chat snapshots and delivery of an explicitly prepared merge |
| subagent-gate | Host-specific roster and serialized writer admission |
| subagent-dispatch | Completion observation, lock release and unrecorded-completion relay |

The fixed compatibility shim only resolves its own tooling root and execs `memory-bank/bin/infra-engine hook HOST ROLE --root TOOLING_ROOT`, forwarding stdin and status. It contains no JSON parser, task/branch discovery, memory operation, temporary publication, policy table or interpreter fallback. Use the trusted canonical shim bytes; keep mode 0755. Cursor/Codex host variants differ only in their literal host argument. The gate remains a tool-owned entrypoint because the Rust host behavior differs.

The same native engine is supplied by `memory-seed`; never compile or download a substitute during generation.

## Final-target binding

Distinguish the staging root from the final target. Files are written and inspected in staging; hook and MCP launch arguments must name the final target's absolute engine and tooling root. A published hook must not refer to discarded staging.

Use the shared Rust formatter for each role:

```sh
"$GENERATOR_NATIVE" repo hook-handler \
  --root "<final-target>" \
  --engine "<final-target>/memory-bank/bin/infra-engine" \
  --host "<selected-client>" --role "<role-stem>"
```

Use `infra-engine.exe` for a Windows artifact. Merge the returned launch fields into the correct event handler, preserving its required event metadata. Do not construct shell quoting yourself.

The formatter supplies POSIX exec-only quoting; Windows Claude's documented native `command`/`args` form; and Cursor/Codex's minimal encoded native invocation through the absolute system Windows PowerShell obtained from `GetSystemDirectoryW`. No project-local executable or PATH lookup selects that shell. Windows shell handlers must be materialized on their actual Windows system; cross-system authoring cannot invent its system directory. Client capability requirements are documented in the native host compatibility contract.

## Process

1. Read the full approved report, selected clients and recorded digest. Inspect its entire machine-readable selection and confirmed evidence. Resolve uncertainty through the review workflow before writing a changed policy.
2. Materialize and validate the approved policy with the native service:

   ```sh
   "$GENERATOR_NATIVE" creator materialize-hook-policy \
     --profile "<approved-report>" \
     --approved-profile-sha256 "<recorded-approved-sha256>" \
     --profile-path "tasks/TASK-N/infra-scan-project-profile.md" \
     --staging "<generation-root>" --json
   "$GENERATOR_NATIVE" creator validate-hook-policy \
     --profile "<approved-report>" \
     --approved-profile-sha256 "<recorded-approved-sha256>" \
     --profile-path "tasks/TASK-N/infra-scan-project-profile.md" \
     --target "<generation-root>" --json
   ```

   Include both owned outputs, `project-brain/config/hook-policy.json` and `project-brain/schemas/hook-policy.schema.json`, with their exact returned byte hashes in the publication/write plans. Preserve whole-file custody; never revise rules or bless a changed hash after review. The schema is data only.
3. Stage the canonical compatibility shims and generate every launch object through `repo hook-handler` using the final target binding.
4. Wire the selected host's supported events:

   - **Claude** uses `.claude/settings.json`. SessionStart runs local-context (10 seconds) and continuity (8); UserPromptSubmit runs memory-read and continuity (8 each); Stop runs memory-write and continuity (8 each). PreToolUse runs naming on Write|Edit, command validation on Bash and subagent admission on Agent|Task (5 each). PostToolUse runs loop detection on Edit|Write|MultiEdit|NotebookEdit and the completion relay on Agent|Task (5 each). SubagentStop runs dispatch (8). Preserve type/matcher/timeouts and the configuration half of built-in-agent restrictions. Permit the selected native memory command according to host permission syntax; do not add Python permissions.
   - **Cursor** uses `.cursor/hooks.json` and its native event/timeout/failClosed fields. Session start, beforeSubmitPrompt and stop select the corresponding native memory/continuity roles. Shell, file, loop and subagent events retain the trusted native wiring metadata. The prompt role always answers its continuation protocol and uses the reserved native-managed `.cursor/rules/working-memory.mdc` slot; the gate uses permission JSON and failClosed behavior.
   - **Codex** uses `.codex/hooks.json` with Claude-style event names and its documented context limits, without Claude matcher/timeout fields. SessionStart runs local-context/continuity; UserPromptSubmit runs memory-read/continuity; Stop runs memory-write/continuity; PreToolUse runs command, naming and subagent admission; PostToolUse runs loop detection. Ship dispatch for parity but leave it unregistered because built-in multi-agent execution is disabled. Preserve the native Memory MCP block and the configuration switches disabling built-in subagents. Commands are final-target native launches, never cwd/ancestor shell searches.
5. Preserve native behavioral contracts rather than transcribing them into shell:
   - A nonempty explicit task ID stays explicit; otherwise Rust resolves supported symbolic/rebase/bisect/Codex-thread identity. An ordinary detached HEAD stays unbound.
   - Prompt deadlines discard partial capsule output and report bounded unavailability. Cursor uses native rule admission/publication/retirement and no session/transcript suppression; Claude/Codex use their ordinary context channel.
   - Turn writes require their actual changed-path authority. Native result capture remains structured and agent-attested, never raw-chat adoption or automatic completion.
   - Visible chat continuity never reads private account/session stores or hidden reasoning, never writes Brain/Bank/SQLite and only restores an explicitly prepared merge.
   - Subagent admission and completion use the actual generated roster, host-specific decisions, writer ownership and bounded completion diagnostics. Native runtime code owns all locking and publication.
6. Write only the task-scoped root-ignore requirement for the reserved Cursor rule. Never compose the target root ignore file here.
7. Run the native generated gate after the staged bundle is complete:

   ```sh
   "$GENERATOR_NATIVE" creator validate-generated \
     --target "<generation-root>" --delivery-root "<final-target>" \
     --editions "<selected-clients>" \
     --skill-plan "<task>/skill-generation-plan.json" \
     --evidence-target "<real-target>" \
     --approved-profile "<approved-report>" \
     --approved-profile-sha256 "<recorded-approved-sha256>"
   ```

   The gate compares whole policy/schema bytes, approved-profile binding, exact launch fields, required roles, artifact capability identity and native behavior. Source inspection never executes an untrusted target hook. Fix failures before publication.
8. Log selected clients, native artifact identity, reviewed selection, policy/schema hashes, event wiring, final-target binding and gate results in `tasks/TASK-N/hook-forge-log.md`.

## Guardrails

- Generate only selected clients and use exact trusted event metadata and shared formatter output.
- No executable Python payload, interpreter detection, Bash business logic, shell ancestor routing, arbitrary evaluation or invented policy operation.
- Preserve the approved generated policy. Missing/malformed/mismatched generated policy refuses admission; it never silently enables a broad ready-edition default.
- Memory/continuity/observer failures follow the native nonblocking protocols and bounded diagnostics. Enforcement denial remains visible and keeps its host status/JSON contract.
- Preserve MCP servers, managed Codex MCP content and existing client trust. Installation, refresh and generation never approve hooks automatically.
- Keep local rendered rules, diagnostics, transient locks and snapshots outside shared authority.
- Never print or log secrets or translate business invariants into brittle text-matching hooks.

## Output

Return selected clients; all nine filenames and registered event mappings; artifact/capability identity; reviewed profile digest; policy/schema hashes; final-target paths; native gate results; the log path; and `memory-seed` as the next forge.
