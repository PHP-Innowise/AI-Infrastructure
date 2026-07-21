# Golden Principles - Infrastructure-Creator

These principles guide how the generator scans and generates. When a specific rule is missing, reason from these. Policy (`AGENTS.md`) and hooks still override principles.

## 1. Evidence Over Assumption

Every claim about the target comes from a real file in the target. If it is not in `composer.json`, config, PHP source, CI, or IaC, it is not a finding - it is a question for the interview or an `unknown`. Framework popularity is never evidence.

## 2. No Bundled Reference Accelerator

The generator authors each artifact from the target's own facts. It does not template from, copy, or reference any other accelerator. Two PHP projects with different integrations must produce visibly different accelerators.

## 3. PHP Is the Domain, Not a Guess

Depth of generated skills is spent on PHP: the detected framework, its persistence layer, its queues, its HTTP layer, its testing and static-analysis tooling. Non-PHP neighbors are documented as integration contracts, never given deep skills. A target with no PHP is out of scope for this generator directly - see Principle 9 for what that means in practice.

## 4. The Profile Is the Only Contract

Phase 1 produces exactly one hand-off artifact: the Project Profile. Phase 2 reads only the profile (re-validated against current files). If a forge needs something the profile does not contain, that is a signal to re-scan or re-interview, not to invent.

## 5. Generate Only What Was Chosen

The target team's AI-tool selection is law. Never emit an edition that was not selected; never skip one that was. Ease of use comes from asking once, not from generating everything and making the user clean up.

## 6. Cite the Source Into the Output

Findings carried into generated skills and seeded memory chunks keep their evidence trail (the file that proves them). A generated accelerator must be auditable back to the scan.

## 7. Fail Loud, Not Silent

A failed scanner, missing tooling, or drift between scan and generate is surfaced explicitly in the confidence summary or the generate report - never swallowed to make a run look clean.

## 8. Least Access, No Secrets

The generator reads only what it needs inside the target path, never `.env`/secrets, and never carries sensitive values into any output. Read-only in Phase 1; the single writer is Phase 2, and only after the collision guard passes.

## 9. Honest Scope Over Silent Failure or Scope Creep

Finding a non-PHP target is not a dead end and not an invitation to stretch this generator beyond PHP. `infra-scan` says plainly what it found and, if a stack is recognizable, offers `stack-adapter` - a distinct, independently built sibling generator for that stack - rather than either failing silently or bolting non-PHP generation onto this tool. The offer always requires explicit consent; detection is never treated as permission.
