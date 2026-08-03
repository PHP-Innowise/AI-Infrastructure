# SKILL FLOW - Infrastructure-Creator

How the generator's 23 skills fit together. This describes the generator itself, not what it produces.

## Main Flow

```text
                          infra-build  (optional one-shot: runs both phases)
                               |
        +----------------------+----------------------+
        v                                             v
  === PHASE 1: infra-scan (read-only) ===       === PHASE 2: infra-generate (writes target) ===

  infra-scan                                    infra-generate
    ├─ PHP evidence? ──no──► recognizable       ├─ policy-forge          ┐
    │                        non-PHP stack?     ├─ skill-forge          │ parallel
    │                          ├─ no  → out of  ├─ hook-forge           │ (four
    │                          │       scope    └─ memory-seed          ┘  forges)
    │                          └─ yes → offer     ├─ agent-forge   (needs skill list)
    │                                  stack-adapter (see below)         ├─ command-forge (needs agents)
    │                                                                    ├─ skill-flow-composer
    ├─ stack-scanner            ┐                                       └─ bootstrap-verifier  (QA gate)
    ├─ architecture-scanner     │ parallel
    ├─ integration-scanner      │ (seven
    ├─ infra-ops-scanner        │  scanners)
    ├─ security-compliance-scanner
    ├─ conventions-scanner
    └─ domain-behavior-scanner
    → stack-researcher
    → clarifying-interview
    → profile-synthesizer  ──────►  PROFILE  ──────►  (re-validated here)
                              (human review checkpoint)

  === PHASE 3 (optional, later): infra-update (upgrades a generated target) ===
  infra-update <target>
    ├─ read <target>/.infra-manifest.json  (no manifest → ABORT: legacy target)
    ├─ re-validate profile → compare manifest version vs VERSION
    ├─ re-run the forges into tasks/TASK-{N}/infra-update-staging/
    ├─ per-file sha256 triage: untouched → replace | user-edited → human decision
    │  | not in manifest → never touched
    └─ rewrite manifest → bootstrap-verifier

  === stack-adapter (independent side path, own orchestration) ===
  infra-adapt <target>  OR  infra-scan's offer, on user consent
    → stack-adapter: research stack → replicate skeleton → re-author 23 skills
      + reference docs → mirror 3 editions → self-verify
    → reports path to new Infrastructure-Creator-[Stack]/ sibling generator
```

## Shortcuts

- Full run in one step: `infra-build <target>`.
- Just discover: `infra-scan <target>` (stops at the profile).
- Just generate from an approved profile: `infra-generate <target>`.
- Upgrade a previously generated target to this generator's current version: `infra-update <target>` (requires the target's `.infra-manifest.json`).
- Target isn't PHP and you already know it: `infra-adapt <target>` (builds a sibling generator directly).

## Phase Map

| Phase | Skills |
| --- | --- |
| orchestration | infra-scan, infra-generate, infra-build, infra-update, stack-adapter |
| discovery | stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, domain-behavior-scanner |
| research | stack-researcher |
| synthesis | clarifying-interview, profile-synthesizer |
| generation | policy-forge, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer |
| verification | bootstrap-verifier |

## Context Handoff

- The scanners each write `tasks/TASK-{N}/<name>-findings.md`.
- `profile-synthesizer` merges everything into `tasks/TASK-{N}/infra-scan-project-profile.md` - the single contract between phases.
- `infra-generate` re-validates that profile against the target's current files, then the forges consume it. `skill-forge`'s log drives `agent-forge`, `command-forge`, and `skill-flow-composer`.
- `bootstrap-verifier` gates success; a failure means generation is not done.
- `infra-generate` finishes by stamping the target's `AGENTS.md` and writing `.infra-manifest.json` (generator version from the root `VERSION` file, profile reference, sha256 per generated file). That manifest is the sole contract `infra-update` consumes later: untouched files are safely replaced, user-edited files become explicit human decisions, unlisted files are never touched, and the run ends with a rewritten manifest plus a fresh `bootstrap-verifier` pass.
- `stack-adapter` writes its own run notes to `tasks/TASK-{N}/stack-adapter-report.md` and its deliverable to a sibling folder, `Infrastructure-Creator-[Stack]/` - never into the target project and never merged into this generator's own tree.
