# Golden Principles - Infrastructure-Creator

These principles guide how the generator scans and generates. When a specific rule is missing, reason from these. Policy (`AGENTS.md`) and hooks still override principles.

## 1. Evidence Over Assumption

Every claim about the target comes from a real file in the target. If it is not in `composer.json`, config, PHP source, CI, or IaC, it is not a finding - it is a question for the interview or an `unknown`. Framework popularity is never evidence.

## 2. No Bundled Reference Accelerator

The generator authors each artifact from the target's own facts. It does not template from, copy, or reference any other accelerator. Two PHP projects with different integrations must produce visibly different accelerators.

## 3. PHP Is the Domain, Not a Guess

Depth of generated skills is spent on PHP: the detected framework, its persistence layer, its queues, its HTTP layer, its testing and static-analysis tooling. Non-PHP neighbors are documented as integration contracts, never given deep skills. A target with no PHP is out of scope for this generator directly - see Principle 9 for what that means in practice.

## 4. Evidence and Generation Contracts Are the Handoff

Phase 1 produces a human Project Profile plus a machine-readable evidence ledger
and one generation contract per proposed skill. Phase 2 consumes only validated
contracts and their cited evidence. If a forge needs something absent from them,
it re-scans or re-interviews instead of inventing or falling back to boilerplate.

## 5. Generate Only What Was Chosen

The target team's AI-tool selection is law. Never emit an edition that was not selected; never skip one that was. Ease of use comes from asking once, not from generating everything and making the user clean up.

## 6. Cite the Source Into the Output

Findings carried into generated skills and seeded memory chunks keep canonical
target-relative paths and fingerprints. A generated accelerator must be
auditable inside the target; a generator-only `tasks/TASK-*` path is provenance,
not runtime evidence.

## 7. Fail Loud, Not Silent

A failed scanner, missing tooling, or drift between scan and generate is surfaced explicitly in the confidence summary or the generate report - never swallowed to make a run look clean.

## 8. Least Access, No Secrets

The generator reads only what it needs inside the target path, never `.env`/secrets, and never carries sensitive values into any output. Read-only in Phase 1; the single writer is Phase 2, and only after the collision guard passes.

## 9. Honest Scope Over Silent Failure or Scope Creep

Finding a non-PHP target is not a dead end and not an invitation to stretch this generator beyond PHP. `infra-scan` says plainly what it found and, if a stack is recognizable, offers `stack-adapter` - a distinct, independently built sibling generator for that stack - rather than either failing silently or bolting non-PHP generation onto this tool. The offer always requires explicit consent; detection is never treated as permission.

## 10. Behavioral Evidence Has Authority, Not Just Confidence

A behavior may be confirmed as implementation without being confirmed as intended policy. Preserve source type, prefer explicit canonical specs/ADRs and executable constraints/tests over incidental code, and surface contradictions. Status enums do not prove transitions; observed authorization does not prove a complete permission matrix; risk-sensitive code does not prove severity, ownership, or approval rules.

## 11. Distinct Operational Value Before Inventory Size

A familiar catalog name does not justify a skill. Every generated skill proves
a distinct trigger, owned and excluded scope, procedure, decisions, output,
verification, failure handling, and routing boundary. Unsupported or
overlapping proposals are pruned or merged.

## 12. Validate Before Publishing

Candidate output remains in task staging until per-skill semantics, global
distinctness, routing, structure, runtime, and ownership all pass. Publication
uses an explicit path plan and rollback journal. Partial publication is failure.
