---
name: council
description: Convene a multi-perspective advisory council for high-stakes or ambiguous native PHP decisions. Use when a choice has significant trade-offs (architecture, security, performance, testing, maintainability, cost) and one viewpoint is not enough. Triggers on "council", "get multiple perspectives", "weigh trade-offs", "which approach", "decision".
phase: planning
flow-next: architect
flow-alternatives: [writing-plans, researcher, architecture-implementer]
related: [architect, researcher, security-reviewer, performance-optimization]
---

# Council

## Overview

Simulate a panel of senior specialists debating one decision, then synthesize a recommendation. The value is surfacing the strongest argument from each viewpoint plus the disagreements, so the user decides with full context.

Use this for decisions that are expensive to reverse: architecture boundaries, persistence strategy, sync vs. async, build vs. buy, framework/library selection, data model, rollout strategy.

## Generated File Naming Convention (MANDATORY)

Any file created by this skill MUST be prefixed with `council-`:
- Correct: `council-decision.md`, `council-tradeoffs.md`
- Incorrect: `DECISION.md`, `NOTES.md`

## The Council Members

Convene the perspectives that matter for the decision (default set below). Add or drop members based on relevance; state which you used.

| Member | Advocates for | Typical concerns (native PHP) |
| --- | --- | --- |
| Architect | Simplicity, clear boundaries, changeability | Coupling, dependency direction, YAGNI, layering |
| Security Engineer | Safety and least privilege | Injection, authz, secrets, input trust, attack surface |
| Performance Engineer | Latency, throughput, resource use | N+1, hot paths, caching, memory, OPcache/JIT |
| Test/Quality Engineer | Verifiability | Testability, seams, flaky risk, coverage cost |
| Maintainer | Long-term cost | Readability, onboarding, operational burden, docs |
| Pragmatist | Shipping value | Effort vs. payoff, deadline risk, reversibility |

## Process

1. **Frame the decision.** State the question, the constraints, and what "good" looks like. If the question is vague, ask one clarifying question first.
2. **Enumerate options.** List 2-4 concrete options (not strawmen). If research is needed to make an option concrete, recommend `/researcher` first.
3. **Round of statements.** For each member, write a short position: their preferred option and the single strongest reason.
4. **Cross-examination.** Record the sharpest objection each option faces and any objection that cannot be mitigated.
5. **Synthesis.** Identify consensus, genuine disagreement, and the deciding factors.
6. **Recommendation.** Give one recommended option with rationale, explicit trade-offs accepted, and what would change the recommendation.

## Output Template

```markdown
# Council Decision: [Question]

## Context & Constraints
[What must hold true; what is out of scope]

## Options
1. **[Option A]** - [one line]
2. **[Option B]** - [one line]

## Perspectives
### Architect
- Prefers: [option] because [reason]
- Objection to others: [...]
### Security Engineer
- ...
### Performance Engineer
- ...
### Test/Quality Engineer
- ...
### Maintainer
- ...
### Pragmatist
- ...

## Consensus
- [Where members agree]

## Disagreements
- [Real, unresolved trade-offs]

## Recommendation
**Chosen: [Option X]**
- Rationale: [...]
- Trade-offs accepted: [...]
- Revisit if: [signal that would flip the decision]
```

## Guardrails

- Members must give real arguments, not caricatures; the weakest-looking option still gets its best case.
- Do not invent facts about the codebase; read relevant files or defer to `/researcher`.
- Keep it decision-focused; this skill advises, it does not implement.

## Final Output

Return the council decision document (path if written), the recommendation, key trade-offs, Context Summary, and next step (`/architect`, `/writing-plans`, `/researcher`, or `/architecture-implementer`).
