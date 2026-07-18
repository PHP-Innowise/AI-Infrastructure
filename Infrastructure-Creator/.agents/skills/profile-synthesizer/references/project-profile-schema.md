# Project Profile Schema

The Project Profile is the single hand-off artifact between Phase 1 (scan) and Phase 2 (generate). `profile-synthesizer` writes it; every forge reads it (re-validated against current files). It MUST follow this schema exactly so forges can parse it reliably. File name: `tasks/TASK-{N}/infra-scan-project-profile.md`.

Every factual line MUST carry a confidence tag - `(confirmed - path:L#)`, `(inferred - reason)`, or `(unknown)` - so generated artifacts remain auditable back to evidence.

## Required Structure

```markdown
# Project Profile: [target_name]

## 0. Metadata
- Target path: [absolute or relative path]
- Scanned: [date]
- Task: tasks/TASK-{N}/
- Generator version: [Infrastructure-Creator version]

## 1. AI Tool Selection (MANDATORY)
- Selected editions: [one or more of: claude, cursor, codex]
- Source: clarifying-interview answer (never assumed)

## 2. PHP Stack
- PHP version: [constraint + resolved] (confidence)
- Framework: [name + version] (confidence)
- Package manager: composer (confidence)
- PSR-4 autoload map: [Namespace => path, ...] (confidence)
- Entry points: [list] (confidence)
- Test tooling: [PHPUnit/Pest + config] (confidence)
- Lint/format: [tool + config] (confidence)
- Static analysis: [PHPStan/Psalm + config] (confidence)

## 3. Architecture
- Pattern: [monolith | modular-monolith | microservices | event-driven] (confidence)
- Layering/DDD: [layered | hexagonal | ports-and-adapters | none-detected] (confidence)
- Service/module boundaries: [list] (confidence)
- Communication: [HTTP | messaging | RPC | none] (confidence)

## 4. Integrations (per category; only those with evidence)
- Payment: [package + how wired] (confidence) | none
- Messaging/Queue: [...] | none
- Search: [...] | none
- Cache: [...] | none
- Object storage: [...] | none
- Email/SMS: [...] | none
- Auth/Identity: [...] | none
- Observability: [...] | none
- Feature flags / CDN / ML-AI / secondary DB: [...] | none

## 5. Infrastructure & Ops
- Containers: [Docker/Compose/K8s + files] (confidence) | none
- CI/CD: [provider + config files] (confidence) | none
- IaC: [Terraform/etc + files] (confidence) | none
- Deployment target: [...] (confidence) | unknown

## 6. Security & Compliance
- Auth pattern: [session | token | OAuth | SAML | none-detected] (confidence)
- Secrets handling: [.env usage, vault, ...] (confidence)
- Security tooling: [composer audit, taint analysis, ...] (confidence) | none
- Compliance mentions: [GDPR/PCI/HIPAA textual mentions only] | none

## 7. Conventions
- Code style: [tool + rules] (confidence)
- Git hooks: [captainhook/husky/...] (confidence) | none
- Docs/ADRs: [locations] (confidence) | none

## 8. Research Notes (from stack-researcher)
- [Dependency: current version, key official-doc practices, source URL]

## 9. Open Items
- [Anything still `unknown` after the interview]

## 10. Generation Notes
- Pre-existing accelerator in target: [yes/no + which folders]
- Skills to generate: [derived list - architecture + universal + one per confirmed integration]
- Non-PHP neighbors (integration contracts only): [list] | none

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Consumption Contract (what each forge reads)

- `policy-forge`: sections 1, 2, 3, 6, 7 -> the target's `AGENTS.md`/`DOD.md`/`GOLDEN-PRINCIPLES.md`/`STABILIZATION.md`.
- `skill-forge`: sections 2, 3, 4 -> architecture skill + universal PHP skills + one skill per confirmed integration (section 10's "Skills to generate").
- `agent-forge`/`command-forge`: the final skill list from `skill-forge` + section 1 (which editions).
- `hook-forge`: sections 2, 4, 5, 6 + section 1 -> hooks tuned to real tooling/risks, for the selected editions only.
- `memory-seed`: all confirmed findings -> one seeded chunk per durable fact, each cited to its source file.

## Validation Rules (profile-synthesizer self-checks)

- Section 1 MUST list at least one edition and MUST cite the interview as its source (never assumed).
- Every line in sections 2-7 MUST carry a confidence tag.
- Section 10's "Skills to generate" MUST be derivable from sections 2-4 (no skill for an integration not in section 4).
- No secrets or credential values may appear anywhere in the profile.
- A `confirmed` integration MUST cite runtime wiring, not merely a `composer.json` entry.
