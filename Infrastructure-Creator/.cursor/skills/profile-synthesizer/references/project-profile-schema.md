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

### 10.1 Skills To Generate (with what each will do)
One line per skill, every description target-specific (never generic boilerplate), citing the same evidence as the section it is drawn from:
- Architecture: `[skill-name]` - [1 sentence: the detected pattern + what this skill will actually guide for this target, e.g. its real module/service boundaries] (from section 3)
- Universal PHP - `coding`, `testing`, `code-review`, `security-review`, `performance`, `release`, `debugging`: [1 sentence each, naming the REAL tools/config this target uses, e.g. "testing will reference Pest 2.x and tests/Pest.php"] (from section 2)
- Integrations (one per `confirmed` integration in section 4): `[skill-name]` - [1 sentence naming the real package/service and how it's wired] (from section 4)

### 10.2 Agents & Commands Preview
- Agents: one per skill listed in 10.1, generated only for selected editions with an agent layer (Claude: full frontmatter; Cursor: reduced frontmatter; Codex: none) -> state the arithmetic explicitly, e.g. "[N] skills x [M] agent-carrying selected editions = [N*M] agents".
- Commands: one per agent, generated only for selected editions with a command layer (Claude, Cursor; Codex invokes skills directly by name, no command layer) -> state the resulting count explicitly.
- If Codex is among the selected editions, say so plainly: "Codex: skills only, no agents or commands."

### 10.3 Non-PHP Neighbors (integration contracts only)
- [list] | none

## 11. Memory Bank Preview

One shared `memory-bank/` will be created at the target root: `README.md`, `INDEX.md`, `.memory-counter`, `templates/chunk.md`, `scripts/validate.py`, `local/.gitkeep`, and `chunks/`. `memory-seed` seeds exactly one chunk per durable **`confirmed`** fact below - never an `inferred` or `unknown` one. This table is the authoritative seed plan: `memory-seed` MUST produce this same set of chunks (same count, same grounding facts) at generation time, not more and not fewer; any difference is drift and must be flagged.

| Planned ID | Title | Type | Source |
| --- | --- | --- | --- |
| MEM-0001 | [short title of the fact] | [architecture\|constraint\|convention\|decision\|domain\|integration\|operations] | [file:line] |
| MEM-0002 | ... | ... | ... |

**Chunks to be seeded:** [count]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Consumption Contract (what each forge reads)

- `policy-forge`: sections 1, 2, 3, 6, 7 -> the target's `AGENTS.md`/`DOD.md`/`GOLDEN-PRINCIPLES.md`/`STABILIZATION.md`.
- `skill-forge`: sections 2, 3, 4, and 10.1's draft descriptions as a starting point -> architecture skill + universal PHP skills + one skill per confirmed integration.
- `agent-forge`/`command-forge`: the final skill list from `skill-forge` + section 1 (which editions); section 10.2's counts are what the user was shown to expect.
- `hook-forge`: sections 2, 4, 5, 6 + section 1 -> hooks tuned to real tooling/risks, for the selected editions only.
- `memory-seed`: section 11 -> the authoritative, pre-reviewed list of chunks to seed, each already cited to its source file.

## Validation Rules (profile-synthesizer self-checks)

- Section 1 MUST list at least one edition and MUST cite the interview as its source (never assumed).
- Every line in sections 2-7 MUST carry a confidence tag.
- Section 10.1's skill list MUST be derivable from sections 2-4 (no skill for an integration not in section 4) and every entry MUST carry a one-line, target-specific description - never generic boilerplate reused across projects.
- Section 10.2's agent/command counts MUST be arithmetically consistent with the 10.1 skill count and section 1's selected editions.
- Section 11 MUST list exactly one planned chunk per durable `confirmed` fact worth remembering across sections 2-7, using the same selection rule `memory-seed` applies, and MUST NOT include any `inferred`/`unknown` fact.
- No secrets or credential values may appear anywhere in the profile.
- A `confirmed` integration MUST cite runtime wiring, not merely a `composer.json` entry.
