---
name: researcher
description: "Run structured research for Symfony decisions: components, bundles, Doctrine/API Platform approaches, package options, or unfamiliar codebase areas."
phase: understanding
flow-next: council
flow-alternatives: [architect, brainstorming, writing-plans]
---

# Symfony Researcher

Research with decision-ready output.

## Method

1. Frame one answerable decision with constraints, non-goals, installed versions, operational environment, and acceptance criteria.
2. Inspect the local codebase and Composer metadata first so research answers the actual integration problem.
3. Prefer current primary sources: official Symfony/Doctrine/API Platform documentation, package source and release notes, relevant standards, and original benchmarks or papers.
4. Record version/date context for claims that may change. Separate sourced facts, local observations, assumptions, and inferences.
5. Compare at least the credible status quo and proposed option. Use a weighted matrix only when the criteria and weights are defensible.
6. Verify critical API/config claims with a minimal local experiment when tooling exists; do not treat a blog snippet as compatibility proof.

Prioritize:

- Existing project code and specs.
- Official Symfony documentation.
- Doctrine documentation.
- API Platform documentation when relevant.
- Package repositories, Packagist metadata, release notes, and security advisories.

Compare options against:

- Symfony version compatibility.
- Maintenance health.
- Security posture.
- Fit with Controller -> Service -> Repository.
- Testing and operational complexity.
- Dependency direction, coupling, data ownership, migration cost, observability, reversibility, and team familiarity.

Use [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) when evaluating whether an option keeps framework/vendor code at the boundary or leaks it into application workflows.

Finish with a concise recommendation, comparison table, evidence links, local compatibility notes, risks, rejected alternatives, confidence/unknowns, and next command. Do not present inference as sourced fact.
