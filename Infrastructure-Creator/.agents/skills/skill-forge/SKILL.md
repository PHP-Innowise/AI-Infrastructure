---
name: skill-forge
description: Author the evidence-gated skill inventory one contract or small sibling group at a time in staging, using only the relevant target evidence slice and blocking generic, duplicated, or unsupported content.
phase: generation
flow-next: agent-forge
flow-alternatives: [policy-forge, hook-forge, memory-seed]
related: [infra-generate, policy-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Skill Forge

## Overview

`skill-forge` authors the evidence-gated inventory in `skill-generation-plan.json`. It works on one skill, or one small sibling group with an explicit shared boundary, at a time. Each pass receives only that contract and its referenced evidence slice, writes to task-scoped staging, and produces a substantive project-specific procedure rather than a renamed template. There are no fixed category counts and no minimum-line padding. Only the memory quartet is unconditional because `memory-seed` always installs its runtime.

Completeness means every justified contract is implemented, not that every catalog slot is filled.

Consult the bundled references for PHP-specific grounding:
- `references/php-frameworks.md` - detection signals and evidence-gated contracts for Design & Interaction, Universal PHP, and Frontend candidates.
- `references/php-architecture-patterns.md` - architecture detection and its generated-skill implications.
- `references/php-integration-catalog.md` - integration categories and what good coverage looks like per category.
- `references/php-process-skills.md` - evidence gates and distinct contracts for process/workflow candidates; only the memory quartet (`memory-bank`, `project-brain`, `checkpoint`, `memory`) is unconditional.
- `references/php-specialty-skills.md` - the evidence-gated framework-specialty catalog, keyed to profile section 3.1's signals.
- `references/php-domain-behavior.md` - how section 8 enriches existing skills, when a domain skill is justified, and the always-generated operational `memory-bank` skill contract.

## Generated File Naming Convention (MANDATORY)

Write each authoring unit into the selected skill trees under
`tasks/TASK-{N}/infra-generate-staging/` (or
`infra-update-staging/`): `.claude/skills`, `.cursor/skills`, and/or
`.agents/skills`. Selected copies must be byte-identical. Staging is generator
runtime output and is never cited as evidence. Target publication is a later
orchestration step. The log maps every staged skill to exactly one plan entry.

## Process

1. **Validate inputs.** Run the plan-only gate. Require exact schema/catalog
   versions, Profile sibling identity, current fingerprints/ranges, supported
   claims, a satisfied evidence-backed `selection_gate` for every selected
   skill, auditable rejected candidates, resolved references, canonical paths,
   and plan/profile inventory equality.
2. **Choose one authoring unit.** Default to one skill. A group may contain only a small set of nearest siblings whose contracts explicitly define their ownership boundary (for example `debugging`/`systematic-debugger`). Never batch a whole category.
3. **Load the minimum slice.** Provide the authoring pass only the selected contract(s), referenced `evidence[]`, cited target files/excerpts, and applicable reference contract. Do not feed unrelated profile prose or prior generated skill bodies.
4. **Author into staging.** Implement every contract field as operational content: bounded purpose and positive/negative triggers; canonical inputs; owned/excluded scope; ordered procedure with real decision branches; target commands, paths, types, packages, and invariants where evidenced; verification; explicit outputs; and failure handling. Procedure text must materially reflect the evidence, not merely mention it in a generic preface.
5. **Preserve boundaries.** If a nearest sibling is planned, state what routes to each and cross-reference it without copying its procedure. If it is not planned, do not create a dangling reference or silently absorb unsupported scope.
6. **Write valid frontmatter:** `name`, one-line trigger-aware `description`, `phase`, `flow-next`, `flow-alternatives`, and `related`. Every reference resolves within the plan.
7. **Review substance, not length.** Reject a skill that could serve an unrelated PHP repository after renaming nouns; reject generic five-step loops, unsupported commands, decorative evidence lists, grouped procedures, and line padding. There is no minimum line count.
   Shared safety/authority text is allowed only through a plan
   `fixed_blocks` entry with an explicit ID, version, and exact content; keep it
   confined to guardrails/failure handling.
8. **Verify the unit.** Check frontmatter, contract-field coverage, evidence/source resolution, sibling boundaries, write scope, commands against real config, and absence of secrets/placeholders/task paths. Record pass/fail before selecting the next unit.
9. **Log exactly one mapping per skill.** Record plan name/category, evidence IDs, staged path, verification result, and publication eligibility. Compute category and total counts from successful log entries.

## Output Template

```markdown
# Skill Forge Complete: [target_name]

**Editions:** [selected]
**Skills staged:** [total from successful log entries]
**Dynamic breakdown:** [category=count, including zero-count categories]
- [category]: [names, or none]

## Log
tasks/TASK-{N}/skill-forge-log.md

## Next
agent-forge (wrap these skills), then command-forge; policy-forge/hook-forge/memory-seed if not already run.
```

## Guardrails

- MUST author only plan entries whose evidence gates passed; the memory quartet is the only unconditional set.
- MUST process one skill or a small explicit sibling group at a time from a minimal contract/evidence slice.
- MUST stage before publication and MUST NOT cite generator task/staging paths in generated target skills.
- MUST produce substantive project-specific procedures; evidence-name substitution and minimum-line padding are invalid.
- MUST reflect the target's real framework/version and real tooling, not assumed defaults.
- MUST write only the selected edition(s).
- MUST ensure every cross-reference resolves to a skill generated in this run.
- MUST NOT generate deep skills for non-PHP neighbors - those are integration contracts only.
- MUST NOT let a scope-split pair duplicate content: `debugging`/`systematic-debugger`, `database-designer`/`orm-patterns`, `performance`/`caching-strategy`, and `api-designer`/`api-platform-design` each have one explicit owner per concern and MUST cross-reference, not restate, their counterpart's half.
- MUST enrich existing skills from confirmed section 8 behavior before creating a domain skill; MUST NOT generate a domain skill without multiple coherent confirmed rules and a distinct operational purpose.
- MUST preserve source type, unknowns, and contradictions; MUST NOT turn statuses into transitions, observed enforcement into a complete permission matrix, or risk indicators into invented severity/approval.

## Final Output

Return the staged skill list by dynamic category, each skill's plan/evidence mapping and verification result, failures that block publication, the log path, and the next validation/publication step. Do not claim target editions were written while output remains in staging.
