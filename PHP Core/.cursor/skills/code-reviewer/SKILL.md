---
name: code-reviewer
description: Review native PHP changes for correctness, security, maintainability, tests, and operational risk.
phase: execution
flow-next: test-generator
flow-alternatives: [coder, verify]
related: [coder, test-generator, verify, security-reviewer]
---

# Code Reviewer

## Review Stance

Prioritize defects, regressions, security issues, missing tests, and operational risks. Do not spend review budget on stylistic preferences unless they affect correctness or maintainability. Leave deep security audits to `/security-reviewer`, but flag obvious risks here.

**Scope boundary:** this skill reviews **local changes** (working tree / branch diff) for **broad** quality. Use `/security-reviewer` for a dedicated OWASP-depth security-only pass, and `/review-pr` when the target is a **remote GitHub pull request** (fetched and commented on via the `gh` CLI).

## Native PHP Review Checklist

### HTTP / CLI Boundary

- Are entry points wrapped by the right middleware (auth, throttling, CSRF)?
- Is input validated and normalized into typed DTOs/value objects before use?
- Is authorization enforced server-side in an explicit access-control layer?
- Are request inputs trusted only after validation?
- Do handlers return correct status codes and stable response shapes?

### Persistence

- Are migrations (or reviewed SQL) reversible and safe for existing data?
- Are indexes and constraints present for important invariants?
- Are all queries parameterized with bound values (no string concatenation)?
- Are transactions used for atomic multi-write workflows and rolled back on failure?
- Are N+1 query patterns avoided (batch/join instead of per-row queries)?

### Types And Structure

- Is `declare(strict_types=1)` present and are types complete?
- Do classes depend on interfaces at boundaries and receive collaborators via injection?
- Is there hidden global state or `new` for collaborators that blocks testing?
- Are exceptions typed and mapped to responses/exit codes at the edge?

### Security

- Any raw/concatenated SQL?
- Any unescaped output in templates (XSS)?
- Any missing authorization or CSRF check on state-changing actions?
- Any secret in source, logs, exceptions, tests, or docs?
- Any unsafe file upload, `unserialize`, `eval`, or dynamic include of untrusted input?
- Any missing rate limit on sensitive endpoints?

### Tests

- Happy path covered?
- Validation failure covered?
- Authorization failure covered?
- Important persistence side effects covered?
- Queues/mail/external clients faked and asserted where applicable?

### Operations

- Cron/worker/queue, cache, and config impacts documented?
- Long-running work moved out of the request cycle?
- External API calls have timeout and error behavior?

## Review Conduct Best Practices

- **Severity-label every finding** (High/Medium/Low/Nit) so the author knows what blocks merge vs. what is optional.
- **Be specific and actionable:** cite `file:line`, explain the risk, and suggest the fix. Avoid vague "this could be better".
- **Separate must-fix from preference:** prefix optional style comments with "Nit:" and do not block on them.
- **Explain the why:** tie feedback to a concrete failure mode, a principle in `.cursor/GOLDEN-PRINCIPLES.md`, or a rule in `AGENTS.md`.
- **Respect scope:** review the diff and what it touches; do not demand unrelated refactors (note them separately as follow-ups).
- **Acknowledge good decisions**, and ask questions instead of asserting when intent is unclear.
- **Right-size the review:** if the change is too large to review well, say so and suggest splitting.

## Output Format

Lead with findings ordered by severity:

```markdown
## Findings

- **High:** `src/Http/Controller/...` allows unauthorized updates because ...
- **Medium:** `migrations/...` adds a queried column without an index ...

## Open Questions

- [Only questions that affect correctness or rollout]

## Summary

[Brief summary after findings]
```

If there are no findings, say so clearly and mention residual test or rollout risk.

## Final Output

Return findings, questions, short summary, Context Summary, and next step.
