---
name: code-reviewer
description: Review Laravel changes for correctness, security, maintainability, tests, and operational risk.
phase: execution
flow-next: test-generator
flow-alternatives: [coder, verify]
related: [coder, test-generator, verify, security-reviewer]
---

# Code Reviewer

## Review Stance

Prioritize defects, regressions, security issues, missing tests, and operational risks. Do not spend review budget on stylistic preferences unless they affect correctness or maintainability. Leave deep security audits to `/security-reviewer`, but flag obvious risks here.

**Scope boundary:** this skill reviews **local changes** (working tree / branch diff) for **broad** quality. Use `/security-reviewer` for a dedicated OWASP-depth security-only pass, and `/review-pr` when the target is a **remote GitHub pull request** (fetched and commented on via the `gh` CLI).

## Laravel Review Checklist

### HTTP Boundary

- Is input validated via a Form Request (`app/Http/Requests/...`) rather than inline `$request->validate()` scattered in the controller, once the rules grow non-trivial?
- Is authorization enforced through a Policy or Gate (`$this->authorize(...)`, `Gate::allows(...)`) rather than relying on hidden UI or ad-hoc `if` checks?
- Do routes carry the correct middleware (`auth`, `verified`, `throttle`, `signed`) for their sensitivity?
- Are controllers thin, delegating multi-step logic to Actions/Services rather than embedding business logic directly?
- Do responses use API Resources (`JsonResource`/`ResourceCollection`) for a stable, versioned response contract instead of returning raw models/arrays?

### Eloquent And Persistence

- Are migrations reversible (`down()` implemented) and safe for existing production data (no destructive column drops without a backfill plan)?
- Is mass assignment protected via `$fillable` (allow-list preferred) or a deliberate, reviewed `$guarded`? Flag `$guarded = []` on models that accept user input.
- Are N+1 query risks present? Look for relationship access inside loops without `with()`/`load()` eager loading, or missing `select()` on wide tables.
- Are queries built with the query builder/Eloquent (bound parameters) rather than raw SQL string interpolation (`DB::raw`, `whereRaw` with unescaped input)?
- Are important invariants backed by database constraints/indexes, not just application-level checks?
- Are multi-write workflows wrapped in `DB::transaction()` where partial failure would leave inconsistent state?

### Jobs, Queues, And Events

- Are queued jobs idempotent (safe to run twice if a queue redelivers), especially for jobs that create records or charge external services?
- Do jobs implement `ShouldBeUnique`/`middleware()` (`WithoutOverlapping`) where duplicate concurrent execution would cause bugs?
- Are failures handled (`failed()` method, sensible `$tries`/`backoff`) rather than silently dropped?

### Types And Structure

- Is `declare(strict_types=1)` present and are types (including return types and property types) complete?
- Do classes depend on interfaces bound in a Service Provider at integration boundaries, rather than `new`-ing concrete external clients directly?
- Are exceptions typed and mapped to responses (custom exception handler, `report()`/`render()`) rather than leaking raw stack traces?
- If PHPStan/Larastan is configured, favor a baseline-and-ratchet strategy on brownfield code: generate a baseline (`phpstan analyse --generate-baseline`) for existing violations and raise the configured level gradually, rather than jumping straight to level 9/10. Eloquent relationship generics (e.g. `HasMany<Order, $this>`) still have known invariance rough edges in some Larastan versions/patterns (notably relations defined in traits or interfaces) — don't block a PR purely on a generics-variance error without a second look.

### Security

- Any raw/concatenated SQL, or `DB::raw`/`whereRaw` with unescaped user input?
- Any unescaped Blade output (`{!! !!}`) on untrusted data (XSS)? `{{ }}` should be the default.
- Any missing Policy/Gate check or CSRF protection (`@csrf` in Blade forms, `VerifyCsrfToken` middleware) on state-changing actions?
- Any secret in source, `.env` committed, logs, exceptions, tests, or docs?
- Any unsafe file upload (missing MIME/size validation, public disk for sensitive files), `unserialize`, `eval`, or dynamic include of untrusted input?
- Any missing rate limiting (`throttle` middleware) on sensitive endpoints (login, password reset, expensive queries)?

### Tests

- Happy path covered via a feature test (`$this->actingAs()->postJson(...)`)?
- Form Request validation failure covered?
- Policy/Gate authorization failure covered (`assertForbidden()`)?
- Important Eloquent persistence side effects covered (`assertDatabaseHas`/`assertDatabaseMissing`)?
- Queues/mail/notifications/external HTTP calls faked (`Queue::fake()`, `Http::fake()`, `Mail::fake()`) and asserted where applicable?

### Operations

- Queue/worker (Horizon), cache, and config impacts documented?
- Long-running work moved out of the request cycle and onto a queued job?
- External API calls have timeout and error handling (`Http::timeout()->retry()`)?
- Are `config:cache`/`route:cache`/`view:cache` implications considered if config or route closures changed?

## Review Conduct Best Practices

- **Severity-label every finding** (High/Medium/Low/Nit) so the author knows what blocks merge vs. what is optional.
- **Be specific and actionable:** cite `file:line`, explain the risk, and suggest the fix. Avoid vague "this could be better".
- **Separate must-fix from preference:** prefix optional style comments with "Nit:" and do not block on them.
- **Explain the why:** tie feedback to a concrete failure mode, a principle in `.claude/GOLDEN-PRINCIPLES.md`, or a rule in `AGENTS.md`.
- **Respect scope:** review the diff and what it touches; do not demand unrelated refactors (note them separately as follow-ups).
- **Acknowledge good decisions**, and ask questions instead of asserting when intent is unclear.
- **Right-size the review:** if the change is too large to review well, say so and suggest splitting.

## Output Format

Lead with findings ordered by severity:

```markdown
## Findings

- **High:** `app/Http/Controllers/...` allows unauthorized updates because the Policy check is missing ...
- **Medium:** `database/migrations/...` adds a queried column without an index ...

## Open Questions

- [Only questions that affect correctness or rollout]

## Summary

[Brief summary after findings]
```

If there are no findings, say so clearly and mention residual test or rollout risk.

## Final Output

Return findings, questions, short summary, Context Summary, and next step.
