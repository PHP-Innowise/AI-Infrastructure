---
name: council
description: Convene a multi-perspective council for high-stakes Symfony decisions involving architecture, Doctrine, Messenger, security, performance, testing, maintainability, or build-vs-buy.
phase: planning
flow-next: architect
flow-alternatives: [researcher, writing-plans, architecture-implementer]
---

# Symfony Council

Use when a decision has real trade-offs.

Perspectives:

- Symfony Maintainer: conventions, DI, framework fit.
- Doctrine Specialist: model, migrations, query performance, data integrity.
- Security Reviewer: voters, access control, validation, OWASP risk.
- Test Lead: testability, fixtures, confidence.
- Operations Engineer: rollout, workers, cache, observability.
- Pragmatic Tech Lead: simplest maintainable path.

Output:

- Options considered.
- Recommendation.
- Trade-offs.
- Risks.
- Decision criteria.
- Next command.
