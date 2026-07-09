---
name: researcher
description: Run structured research to inform a native PHP decision. Use to evaluate libraries/packages, compare approaches, study an unfamiliar codebase area, or gather authoritative references before committing. Triggers on "research", "compare", "evaluate", "which library", "investigate options", "find out how".
phase: understanding
flow-next: council
flow-alternatives: [architect, brainstorming, writing-plans]
related: [council, architect, brainstorming, dependency-manager]
---

# Researcher

## Overview

Turn an open question into a sourced, decision-ready findings document. Research is not open-ended browsing: scope the question, gather evidence, compare options against the project's real constraints, and recommend.

Two research surfaces:

- **Internal:** the current codebase (how something works, where behavior lives, current conventions). Use `Glob`/`Grep`/`Read`.
- **External:** libraries, standards (PSR), language features, and best practices. Use `WebFetch`/`WebSearch` within the allowed domains (php.net, php-fig.org, packagist.org, getcomposer.org, phpstan.org, psalm.dev, phpunit.de, pestphp.com, github.com).

## Generated File Naming Convention (MANDATORY)

Any file created by this skill MUST be prefixed with `researcher-`:
- Correct: `researcher-findings.md`, `researcher-library-comparison.md`
- Incorrect: `RESEARCH.md`, `NOTES.md`

## Process

1. **Scope the question.** Write the exact question and the decision it will inform. List the constraints that matter (PHP version, license, maintenance, dependencies, performance, team familiarity). If the question is too broad, narrow it or ask one clarifying question.
2. **Set acceptance criteria.** Define what a good answer must include so research has a clear finish line.
3. **Gather evidence.** Read primary sources first (official docs, the package repo, PSR text, source code). Note version and date; APIs drift.
4. **Evaluate options.** Score each option against the constraints in a comparison table. Prefer maintained, widely used, standards-aligned packages over bespoke code unless there is a real reason.
5. **Check the project fit.** Confirm the option works with the project's PHP version and existing dependencies (`composer.json`), and check for conflicts.
6. **Recommend.** Give a clear recommendation with rationale, risks, and what you did not verify.

## Library Evaluation Checklist

- Actively maintained (recent commits/releases; open issue responsiveness)?
- Compatible with the project's PHP version and dependencies?
- License compatible with the project?
- Reasonable dependency footprint (no heavy transitive tree)?
- Security posture (advisories via `composer audit`, known CVEs)?
- Typed, documented, and tested? PSR-compliant where relevant?
- Popularity/support (downloads on Packagist, community usage) as a tie-breaker, not the sole criterion?

## Output Template

```markdown
# Research: [Question]

## Question & Decision
[What we need to decide and why]

## Constraints
- PHP version, license, dependencies, performance, maintenance, team familiarity

## Options Compared
| Option | Maintained | PHP compat | License | Deps | Notes |
| --- | --- | --- | --- | --- | --- |
| A | ... | ... | ... | ... | ... |

## Evidence
- [Source] ([date/version]) - [what it establishes]

## Recommendation
**Use: [Option]** because [rationale].
- Risks: [...]
- Not verified: [...]

## Sources
- [Title](url) - accessed [date]
```

## Guardrails

- Cite sources; do not present recollection as fact. Note version and access date.
- Distinguish verified findings from assumptions.
- Do not install packages or modify code; this skill produces knowledge, not changes. Hand off to `/coder`, `/dependency-manager`, or `/architecture-implementer`.

## Final Output

Return the findings document (path if written), the recommendation, key risks, Context Summary, and next step (`/council`, `/architect`, `/brainstorm`, or `/writing-plans`).
