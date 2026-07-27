---
name: domain-behavior-scanner
description: Detect a PHP target's evidence-backed behavioral contract - purpose and domain vocabulary, sources of truth, core entities, business invariants, lifecycle transitions, roles and permissions, audit obligations, high-risk or forbidden workflows, critical QA scenarios, and sanitized incident lessons. Use as Phase 1 discovery input to profile-synthesizer. Triggers on "scan domain behavior", "find business rules", "detect statuses and permissions", "map domain invariants", "domain-behavior-scanner".
phase: discovery
flow-next: profile-synthesizer
flow-alternatives: [clarifying-interview]
related: [infra-scan, architecture-scanner, security-compliance-scanner, conventions-scanner, profile-synthesizer, clarifying-interview, memory-seed, skill-forge, policy-forge]
---

# Domain Behavior Scanner

## Overview

Read-only reconnaissance of a PHP target's **behavioral contract**: what the system does and which observable rules future changes must preserve. This scanner complements the technical scanners. It does not re-detect the framework, infrastructure, third-party packages, or generic security posture. It reads tests, specs, workflow configuration, domain code, authorization code, validation, and database constraints to extract only behavior that has concrete evidence.

The target project path is a **required** argument. Operate strictly read-only within that path; never read `.env`, credentials, raw production data, customer payloads, or secrets. Write findings only into the current run's task directory.

## Evidence and Authority Model

Every finding records both:

- **Confidence:** `confirmed`, `inferred`, or `unknown`.
- **Source type:** `spec/ADR`, `test`, `database constraint`, `workflow configuration`, `authorization rule`, `domain code`, `application code`, `configuration`, or `interview answer`.

Source type matters because current implementation is not automatically intended policy. Prefer explicit specs/ADRs and executable constraints/tests over comments or incidental controller behavior. When sources conflict, report the contradiction; do not silently choose one.

## Generated File Naming Convention (MANDATORY)

Write exactly one findings file:

`tasks/TASK-{N}/domain-behavior-scanner-findings.md`

Never write into the target.

## Process

1. **Establish purpose and vocabulary.** Read the root README and relevant `docs/`, specs, ADRs, route names, module names, and central domain classes. Record only stable, repeated terminology and a short factual project purpose.
2. **Identify project-specific sources of truth.** Detect explicit authority language such as "source of truth", "canonical", "generated from", "do not edit", schema ownership, OpenAPI ownership, workflow specifications, and links between documents. Record authority scope and conflicts; never infer an owner from Git history.
3. **Map central modules and domain entities.** Use architecture findings as the structural baseline, then add each central module's evidenced responsibility and only the core entities, identifiers, relationships, ownership fields, lifecycle/status fields, and integrity constraints. Do not dump the whole schema.
4. **Extract business invariants.** Read, in descending authority: explicit specs/ADRs; behavior-focused tests; database constraints; workflow/state-machine configuration; entities/value objects; application services/actions/use cases; policies/voters; validation rules; controllers as weaker corroboration. Record a rule only when the source actually expresses it.
5. **Separate statuses from transitions.** An enum proves statuses exist, not which transitions are legal. Mark a transition `confirmed` only when a workflow config, guarded transition method, explicit spec, or behavioral test proves the edge and its guard. Record side effects only when evidenced.
6. **Map roles and permissions.** Derive observed role -> action -> subject/resource rules from Policies/Gates/Voters, middleware/access-control config, ownership/tenant checks, and negative tests. Never claim the matrix is complete unless all relevant entry points are covered. Keep product roles distinct from authentication technology.
7. **Detect audit obligations.** Look for explicit audit requirements, activity/event tables, audit packages, actor/timestamp/change-set fields, immutable logs, and tests. Record required events and fields only when evidenced; never copy log contents.
8. **Identify high-risk and forbidden workflows.** Flag concrete behavior touching money, authorization, personal data, data integrity, irreversible transitions, or external contracts as a **risk indicator**, not an automatic severity or approval policy. Record severity/required approval only when a source states it.
9. **Extract critical QA/regression scenarios.** Select a bounded set of behavior-focused tests/spec scenarios that protect important invariants, denied paths, transitions, retries/idempotency, or historical regressions. Prefer named scenarios over coverage metrics.
10. **Capture sanitized incident lessons and known risks.** Read tracked postmortems, known-issues docs, runbooks, and regression tests when present. Keep only the durable cause/prevention rule; never include raw logs, customer identifiers, payloads, or confidential incident detail.
11. **Propose domain-skill candidates.** A candidate must represent one coherent bounded context with multiple durable rules and a distinct review purpose (for example, `billing-rules-review`). Never propose one skill per rule, role, entity, or status.
12. **Bound the scan.** Prioritize central/high-risk modules and representative tests. Cite omitted low-value areas if the codebase is too large for exhaustive coverage.
13. **Mark confidence and source type** on every factual entry. Leave unsupported items `unknown` or `none`; never turn implementation convention into intended business policy without qualification.

## Output Template

```markdown
# Domain Behavior Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Project Purpose & Domain Vocabulary
- Purpose: [short factual description] (confidence; source type - path:L#)
- Terms: [term = evidenced meaning] (confidence; source type - path:L#)

## Project Sources of Truth
- [area] -> [source/location] -> [authority scope] (confidence; source type - path:L#)
- Contradictions: [sources that disagree] | none

## Core Modules & Domain Model
- [module] - responsibility: [...] - paths: [...] (confidence; source type - path:L#)
- [entity] - identifiers/relationships/lifecycle fields: [...] (confidence; source type - path:L#)

## Business Invariants
- [rule] - affected scope: [...] - consequence: [...] (confidence; source type - path:L#)

## Lifecycles & Transitions
- Statuses discovered: [entity -> statuses] (confidence; source type - path:L#)
- Confirmed transition: [entity: from -> to] - allowed when: [...] - forbidden when: [...] - side effects: [...] (confidence; source type - path:L#)

## Roles & Permissions
- [role/principal] -> [action] on [subject/resource] - restrictions: [...] - enforcement points: [...] (confidence; source type - path:L#)
- Completeness: [complete for named surface | partial/unknown]

## Audit Obligations
- [event] - fields required: [...] - retention/immutability: [...] (confidence; source type - path:L#)

## High-Risk / Forbidden Workflows
- [workflow/action] - risk indicator: [...] - documented approval/checks: [...] (confidence; source type - path:L#)

## Critical QA / Regression Scenarios
- [scenario] - given/when/then: [...] - protected rule: [...] (confidence; source type - test/spec path:L#)

## Known Risks & Incident Lessons
- [sanitized lesson] - prevention rule: [...] (confidence; source type - path:L#)

## Domain Skill Candidates
- `[skill-name]` - bounded context: [...] - rules covered: [...] - distinct review purpose: [...] (confidence; source paths)
- none

## Gaps & Contradictions
- [material unknown or disagreement that could change generated policy/skills/memory]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Guardrails

- MUST operate read-only on the target and MUST NOT read `.env`, secrets, customer data, or raw production payloads.
- MUST cite a target file and source type for every finding.
- MUST distinguish status discovery from confirmed transitions.
- MUST distinguish authentication technology from product roles/permissions.
- MUST NOT infer owners, severity, approval requirements, legal obligations, or intended behavior from popularity or convention.
- MUST surface contradictions between tests, specs, constraints, and implementation instead of silently resolving them.
- MUST keep output bounded: central entities, durable rules, and representative critical scenarios only.
- MUST NOT propose a domain skill without multiple coherent rules and a distinct operational purpose.

## Final Output

Return the findings path, a short summary of confirmed invariants/transitions/permissions/risks, any domain-skill candidates, material contradictions or unknowns, and a confidence summary. Suggest `clarifying-interview` for material unresolved behavior or `profile-synthesizer` when no material ambiguity remains.
