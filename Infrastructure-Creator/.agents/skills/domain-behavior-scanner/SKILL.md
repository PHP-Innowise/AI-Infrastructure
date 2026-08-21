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

The target project path is a **required** argument. Operate strictly read-only within it, under the contract's secrets rule, and never read raw production data or customer payloads. Write only into the current run's task directory.

## Evidence and Authority Model

Every finding records the contract's `confidence` and `source_type` values. Source type matters because current implementation is not automatically intended policy. Prefer explicit specs/ADRs and executable constraints/tests over comments or incidental controller behavior. When sources conflict, report the contradiction; do not silently choose one.

## Outputs (MANDATORY)

Per run: exactly one report `tasks/TASK-{NNN}/domain-behavior-scanner-findings.md` and exactly one evidence ledger `tasks/TASK-{NNN}/domain-behavior-scanner-evidence.json` and exactly one coverage record `tasks/TASK-{NNN}/domain-behavior-scanner-coverage.json`, all shaped by `stack-scanner/references/scan-evidence-contract.md` - read it first, including its sibling-input fallback. Never write into the target.

## Process

1. **Establish purpose and vocabulary.** Read the root README and relevant `docs/`, specs, ADRs, route names, module names, and central domain classes. Record only stable, repeated terminology and a short factual project purpose.
2. **Identify project-specific sources of truth.** Detect explicit authority language such as "source of truth", "canonical", "generated from", "do not edit", schema ownership, OpenAPI ownership, workflow specifications, and links between documents. Record authority scope and conflicts; never infer an owner from Git history.
3. **Map central modules and domain entities.** Use the sibling artifacts at `tasks/TASK-{NNN}/architecture-scanner-findings.md` and `tasks/TASK-{NNN}/architecture-scanner-evidence.json` in this run's task directory as the structural baseline; if they are absent, apply the contract's sibling-input fallback - derive first-hand only the modules your invariants need, mark them `inferred`, and record the gap. Then add each central module's evidenced responsibility and only the core entities, identifiers, relationships, ownership fields, lifecycle/status fields, and integrity constraints. Do not dump the whole schema.
4. **Extract and identify business invariants.** Read, in descending authority: explicit specs/ADRs; behavior-focused tests; database constraints; workflow/state-machine configuration; entities/value objects; application services/actions/use cases; policies/voters; validation rules; controllers as weaker corroboration. Record a rule only when the source actually expresses it. Assign a stable kebab-case invariant ID, affected bounded context/path authority, consequence, priority (`high`, `normal`), and a bounded anchor (`line range`, `symbol`, or `JSON pointer`). High priority is reserved for evidenced data-loss, authorization, money, irreversible lifecycle, external-contract, or known-regression consequences.
5. **Separate statuses from transitions.** An enum proves statuses exist, not which transitions are legal. Mark a transition `confirmed` only when a workflow config, guarded transition method, explicit spec, or behavioral test proves the edge and its guard. Record side effects only when evidenced.
6. **Map roles and permissions.** Derive observed role -> action -> subject/resource rules from Policies/Gates/Voters, middleware/access-control config, ownership/tenant checks, and negative tests. Never claim the matrix is complete unless all relevant entry points are covered. Keep product roles distinct from authentication technology.
7. **Detect audit obligations.** Look for explicit audit requirements, activity/event tables, audit packages, actor/timestamp/change-set fields, immutable logs, and tests. Record required events and fields only when evidenced; never copy log contents.
8. **Identify high-risk and forbidden workflows.** Flag concrete behavior touching money, authorization, personal data, data integrity, irreversible transitions, or external contracts as a **risk indicator**, not an automatic severity or approval policy. Record severity/required approval only when a source states it.
9. **Extract critical QA/regression scenarios.** Select a bounded set of behavior-focused tests/spec scenarios that protect important invariants, denied paths, transitions, retries/idempotency, partial failure, or historical regressions. Prefer named scenarios over coverage metrics. Map each scenario to invariant IDs, its suite/root and focused repository command taken from the sibling `tasks/TASK-{NNN}/stack-scanner-findings.md` (when that artifact is absent, cite the test file and mark the invocation `unknown` with a recorded gap rather than inventing a command), exact fixture/fake prerequisites, and expected pass/fail result. For a failure invariant, state both the required retained state and the forbidden later state.
10. **Capture sanitized incident lessons and known risks.** Read tracked postmortems, known-issues docs, runbooks, and regression tests when present. Keep only the durable cause/prevention rule; never include raw logs, customer identifiers, payloads, or confidential incident detail.
11. **Propose domain-skill candidates and invariant coverage.** A candidate must represent one coherent bounded context with multiple durable rules and a distinct review purpose (for example, `billing-rules-review`). For each high-priority invariant, nominate at least one candidate procedure owner and concrete verification assertion owner, plus material adjacent owners for framework, security, testing, persistence, and provider mechanics. Never propose one skill per rule, role, entity, or status.
12. **Bound the scan.** Prioritize central/high-risk modules and representative tests. Cite omitted low-value areas if the codebase is too large for exhaustive coverage.
13. **Mark confidence and source type** on every factual entry. Leave unsupported items `unknown` or `none`; never turn implementation convention into intended business policy without qualification.

## Report Structure

Follow the `domain-behavior-scanner` report template in appendix A of `stack-scanner/references/scan-evidence-contract.md`. Every factual line carries its confidence tag and its evidence id.

## Guardrails

- MUST operate read-only on the target and MUST NOT read `.env`, secrets, customer data, or raw production payloads.
- MUST give every surface it saw one of the four dispositions in the coverage record; a surface nobody dispositioned is not the same as one nobody needed.
- MUST cite a target file and source type for every finding, and MUST emit all three artifacts with contract-shaped evidence records (target-relative path, `sha256:` fingerprint, supported claims).
- MUST take sibling `architecture-scanner`/`stack-scanner` input from this run's task directory and degrade with a recorded gap when it is missing; MUST NOT invent it.
- MUST distinguish status discovery from confirmed transitions.
- MUST distinguish authentication technology from product roles/permissions.
- MUST NOT infer owners, severity, approval requirements, legal obligations, or intended behavior from popularity or convention.
- MUST surface contradictions between tests, specs, constraints, and implementation instead of silently resolving them.
- MUST keep output bounded: central entities, durable rules, and representative critical scenarios only.
- MUST NOT propose a domain skill without multiple coherent rules and a distinct operational purpose.
- MUST assign stable IDs and concrete regression mappings to every high-priority confirmed invariant; an unowned high-priority invariant is a blocking synthesis gap.
- MUST NOT elevate an inferred convention, catalog concern, or external standard into a confirmed invariant.

## Final Output

Return all three artifact paths (report, evidence ledger, coverage record), a short summary of confirmed invariants/transitions/permissions/risks, any domain-skill candidates, material contradictions or unknowns, and a confidence summary. Suggest `clarifying-interview` for material unresolved behavior or `profile-synthesizer` when no material ambiguity remains.
