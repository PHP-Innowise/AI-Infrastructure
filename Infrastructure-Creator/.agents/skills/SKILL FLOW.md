# SKILL FLOW - Infrastructure-Creator

How the generator's 20 skills fit together. This describes the generator itself, not what it produces.

## Main Flow

```text
                          infra-build  (optional one-shot: runs both phases)
                               |
        +----------------------+----------------------+
        v                                             v
  === PHASE 1: infra-scan (read-only) ===       === PHASE 2: infra-generate (writes target) ===

  infra-scan                                    infra-generate
    ├─ stack-scanner            ┐                 ├─ policy-forge          ┐
    ├─ architecture-scanner     │ parallel        ├─ skill-forge          │ parallel
    ├─ integration-scanner      │ (six            ├─ hook-forge           │ (four
    ├─ infra-ops-scanner        │  scanners)      └─ memory-seed          ┘  forges)
    ├─ security-compliance-scanner                 ├─ agent-forge   (needs skill list)
    └─ conventions-scanner                         ├─ command-forge (needs agents)
    → stack-researcher                             ├─ skill-flow-composer
    → clarifying-interview                         └─ bootstrap-verifier  (QA gate)
    → profile-synthesizer  ──────►  PROFILE  ──────►  (re-validated here)
                              (human review checkpoint)
```

## Shortcuts

- Full run in one step: `infra-build <target>`.
- Just discover: `infra-scan <target>` (stops at the profile).
- Just generate from an approved profile: `infra-generate <target>`.

## Phase Map

| Phase | Skills |
| --- | --- |
| orchestration | infra-scan, infra-generate, infra-build |
| discovery | stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner |
| research | stack-researcher |
| synthesis | clarifying-interview, profile-synthesizer |
| generation | policy-forge, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer |
| verification | bootstrap-verifier |

## Context Handoff

- The scanners each write `tasks/TASK-{N}/<name>-findings.md`.
- `profile-synthesizer` merges everything into `tasks/TASK-{N}/infra-scan-project-profile.md` - the single contract between phases.
- `infra-generate` re-validates that profile against the target's current files, then the forges consume it. `skill-forge`'s log drives `agent-forge`, `command-forge`, and `skill-flow-composer`.
- `bootstrap-verifier` gates success; a failure means generation is not done.
