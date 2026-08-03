# Project Profile Schema

The Project Profile is the single hand-off artifact between Phase 1 (scan) and Phase 2 (generate). `profile-synthesizer` writes it; every forge reads it (re-validated against current files). It MUST follow this schema exactly so forges can parse it reliably. File name: `tasks/TASK-{N}/infra-scan-project-profile.md`.

Every factual line MUST carry a confidence tag - `(confirmed - path:L#)`, `(inferred - reason)`, or `(unknown)` - so generated artifacts remain auditable back to evidence. Behavioral findings in section 8 MUST additionally name the source type (`spec/ADR`, `test`, `database constraint`, `workflow configuration`, `authorization rule`, `domain code`, `application code`, `configuration`, or `interview answer`) because confirmed implementation and confirmed policy are not equal authorities.

## Required Structure

```markdown
# Project Profile: [target_name]

## 0. Metadata
- Target path: [absolute or relative path]
- Scanned: [date]
- Task: tasks/TASK-{N}/
- Generator version: [contents of the VERSION file at this generator's root, e.g. 1.4.0 - the single source of the generator's version; never hardcoded, never recalled from the changelog]

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

### 3.1 Framework-Specialty Signals (drives conditional skill generation)
Each line is a scannable, generalized signal (never named after a specific framework unless that IS the evidence) - see `skill-forge/references/php-specialty-skills.md` for the full mapping from signal to generated skill:
- ORM / data-access pattern: [ORM name+version | query-builder | plain PDO] (confidence) | none
- DB migration tooling: [tool + migrations dir] (confidence) | none
- Async/queue mechanism: [queued jobs | message bus | scheduled async work + real transport] (confidence) | none
- Event listener/subscriber/observer pattern: [present + real classes/dirs] (confidence) | none
- Multi-channel notification delivery: [mail/database/broadcast/SMS notification classes] (confidence) | none
- In-app caching strategy (beyond a bare cache integration): [cache-aside usage, tags, model/query caching] (confidence) | none
- File/object storage abstraction (beyond a bare storage integration): [disk config, signed URLs, upload handling] (confidence) | none
- Auth/authorization scaffolding: [session/cookie auth + Policy/Gate/Voter-style authorization layer] (confidence) | none
- Form/validator design: [dedicated Form/Request/DTO + Validator layer with custom constraints] (confidence) | none
- Admin/back-office panel: [package + wiring] (confidence) | none
- Declarative API resource framework: [package + wiring, e.g. attribute/annotation-driven REST layer] (confidence) | none
- Custom console commands: [count + real command classes] (confidence) | none
- Repository/data-access layer: [present + real classes/dirs] (confidence) | none
- DI container configuration style: [declarative config file (services.yaml/services.php) | code-driven bindings only] (confidence)
- Test data factories/fixtures: [present + real classes/dirs] (confidence) | none
- Package vs. application: [distributable Composer library | deployed application] (confidence)

### 3.2 Frontend Presence (drives the conditional frontend skill group)
- Rendering/templating layer: [Blade | Twig | plain PHP templates | none] (confidence)
- Frontend asset build: [bundler + real config, e.g. Vite/Mix/Webpack] (confidence) | none
- Verdict: [frontend skill group applies | no UI surface - skip frontend skills] (confidence)

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

## 8. Domain & Behavioral Contract (from domain-behavior-scanner)

### 8.1 Project Purpose & Domain Vocabulary
- Purpose: [short factual description] (confidence; source type - path:L#)
- [term]: [evidenced meaning] (confidence; source type - path:L#)

### 8.2 Project Sources of Truth
- [area]: [source/location] - authority: [what it governs] (confidence; source type - path:L#)
- Contradictions: [sources that disagree] (confidence; source types + paths) | none

### 8.3 Core Modules & Domain Model
- [module]: [evidenced responsibility + paths] (confidence; source type - path:L#)
- [entity]: [identifier, central relationships, ownership/lifecycle fields, integrity constraints only] (confidence; source type - path:L#)

### 8.4 Business Invariants
- [rule] - scope: [module/entity/workflow] - consequence: [what must remain true] (confidence; source type - path:L#)

### 8.5 Lifecycles & Transitions
- Statuses discovered: [entity -> statuses] (confidence; source type - path:L#)
- Confirmed transition: [entity: from -> to] - allowed when: [...] - forbidden when: [...] - side effects: [...] (confidence; source type - path:L#)
- A status list MUST NOT be presented as a transition graph without transition evidence.

### 8.6 Roles & Permissions
- [role/principal] -> [action] on [subject/resource] - restrictions: [...] - enforcement points: [...] (confidence; source type - path:L#)
- Completeness: [complete for named surface | partial | unknown] (confidence; source type - path:L#)

### 8.7 Audit Obligations
- [event] - fields required: [...] - retention/immutability: [...] (confidence; source type - path:L#) | none

### 8.8 High-Risk / Forbidden Workflows
- [workflow/action] - risk indicator: [...] - documented approval/checks: [...] (confidence; source type - path:L#) | none
- Risk indicators MUST NOT invent severity, owners, or approval requirements.

### 8.9 Critical QA / Regression Scenarios
- [scenario] - given/when/then: [...] - protected rule: [...] (confidence; source type - test/spec path:L#) | none

### 8.10 Known Risks & Incident Lessons
- [sanitized lesson] - prevention rule: [...] (confidence; source type - path:L#) | none

### 8.11 Domain Skill Candidates
- `[skill-name]` - bounded context: [...] - coherent rules covered: [...] - distinct review purpose: [...] (confidence; source paths)
- none

## 9. Research Notes (from stack-researcher)
- [Dependency: current version, key official-doc practices, source URL]

## 10. Open Items
- [Anything still `unknown` after the interview]

## 11. Generation Notes
- Pre-existing accelerator in target: [yes/no + which folders]

### 11.1 Skills To Generate (with what each will do)
One line per skill, every description target-specific (never generic boilerplate), citing the same evidence as the section it is drawn from. Organized into eight groups - see `skill-forge/references/` for the full authoring rules behind each group:
- **Architecture** (1, from section 3): `[skill-name]` - [1 sentence: the detected pattern + what this skill will actually guide for this target, e.g. its real module/service boundaries].
- **Design & Interaction** (3, always generated, from sections 2-3): `architecture-implementer`, `api-designer`, `database-designer` - [1 sentence each naming the target's real scaffolding tool/API shape/persistence approach].
- **Frontend** (0 or 5, conditional on section 3.2's verdict): if applicable, `frontend-design`, `coder-frontend`, `wcag-accessibility`, `web-design-guidelines`, `browser-verify` - [1 sentence naming the target's real templating/asset stack]; if not applicable, state "No UI surface detected - frontend skill group skipped" explicitly.
- **Process & Workflow** (18, always generated, framework-agnostic - per `skill-forge/references/php-process-skills.md`): `requirements-analyst`, `researcher`, `brainstorming`, `council`, `writing-plans`, `using-git-worktrees`, `systematic-debugger`, `refactorer`, `dependency-manager`, `review-pr`, `finishing-branch`, `documentation-generator`, `skill-creator`, `reflect`, `memory-bank`, `project-brain`, `checkpoint`, `memory` - list by name; a single shared sentence describing this fixed group is sufficient. The last four are the memory quartet: `memory-bank` is the operational retrieve/capture/audit/supersede skill for the shared bank that `memory-seed` creates, `project-brain` operates the governed control plane, and `checkpoint`/`memory` are the manual companions to the automatic working-memory hooks.
- **Universal PHP** (7, from section 2): `coding`, `testing`, `code-review`, `security-review`, `performance`, `release`, `debugging`: [1 sentence each, naming the REAL tools/config this target uses, e.g. "testing will reference Pest 2.x and tests/Pest.php"].
- **Framework-Specialty** (one per `confirmed`/`inferred` signal in section 3.1 - per `skill-forge/references/php-specialty-skills.md`): `[skill-name]` - [1 sentence naming the real signal and pattern it guides]; state "none" if section 3.1 has no confirmed/inferred signals.
- **Integrations** (one per `confirmed` integration in section 4): `[skill-name]` - [1 sentence naming the real package/service and how it's wired].
- **Domain** (0 or more, evidence-gated from section 8.11): generate one bounded-context review skill only when multiple coherent confirmed rules create a distinct review purpose; `[skill-name]` - [1 sentence naming the bounded context, invariants/transitions/permissions it protects, and source paths]. Never generate one skill per rule/entity/status/role. State "none - no cohesive domain skill candidate" when section 8.11 has none.

### 11.2 Agents & Commands Preview
- Skill count breakdown: state each group's count explicitly, e.g. "1 architecture + 3 design + [0 or 5] frontend + 18 process + 7 universal + [N] specialty + [M] integrations + [D] domain = [total] skills".
- Agents: one per skill listed in 11.1, generated only for selected editions with an agent layer (Claude: full frontmatter; Cursor: reduced frontmatter; Codex: none) -> state the arithmetic explicitly, e.g. "[total] skills x [E] agent-carrying selected editions = [total*E] agents".
- Commands: one per agent, generated only for selected editions with a command layer (Claude, Cursor; Codex invokes skills directly by name, no command layer) -> state the resulting count explicitly.
- If Codex is among the selected editions, say so plainly: "Codex: skills only, no agents or commands."

### 11.3 Non-PHP Neighbors (integration contracts only)
- [list] | none

## 12. Memory Bank Preview

One shared memory layer will be created at the target root, spanning two roots: `memory-bank/` (`README.md`, `INDEX.md`, `.memory-counter`, `templates/chunk.md`, `local/.gitkeep`, `chunks/`, and the dependency-free context-brain runtime `scripts/context.py` + `scripts/brain_runtime.py` + `scripts/context_retrieval.py` + `scripts/validate.py`) and the governed `project-brain/` control plane skeleton (`PROTOCOL.md`, schemas, record/control templates, config with the target's framework slug in `runtime.json`, empty indexes, and `.gitkeep`-held record directories). The automatic working-memory hooks `hook-forge` generates call this runtime; the memory quartet skills (`memory-bank`, `project-brain`, `checkpoint`, `memory`) operate it. `memory-seed` seeds one chunk per **cohesive durable concept** below, composed only from `confirmed` facts - never an `inferred` or `unknown` one. A chunk may group tightly related facts (for example, one invoice-lifecycle concept containing its confirmed statuses, transitions, guards, and audit consequence) instead of producing one tiny chunk per line. It MUST link to canonical sources rather than copying full specs, schemas, permission matrices, test inventories, or incident reports. This table is the authoritative seed plan: `memory-seed` MUST produce this same set of chunks (same count, same concepts, same sources) at generation time; any difference is drift and must be flagged.

| Planned ID | Title | Type | Source |
| --- | --- | --- | --- |
| MEM-0001 | [short title of the fact] | [architecture\|constraint\|convention\|decision\|domain\|integration\|operations] | [file:line] |
| MEM-0002 | ... | ... | ... |

**Chunks to be seeded:** [count]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Consumption Contract (what each forge reads)

- `policy-forge`: sections 1, 2, 3, 6, 7, and confirmed high-value rules from section 8 -> the target's `AGENTS.md`/`DOD.md`/`GOLDEN-PRINCIPLES.md`/`STABILIZATION.md`.
- `skill-forge`: sections 2, 3, 3.1, 3.2, 4, 8, and 11.1's draft descriptions as a starting point -> architecture + design/interaction + (conditional) frontend + process/workflow (including memory-bank) + universal PHP + framework-specialty + integration + evidence-gated domain skills.
- `agent-forge`/`command-forge`: the final skill list from `skill-forge` + section 1 (which editions); section 11.2's counts are what the user was shown to expect.
- `hook-forge`: sections 2, 4, 5, 6 + section 1 -> the four enforcement hooks tuned to real tooling/risks plus the two working-memory hooks calling `memory-bank/scripts/context.py`, for the selected editions only.
- `memory-seed`: section 12 -> the authoritative, pre-reviewed list of cohesive concepts to seed, each already cited to canonical source files; plus section 2's confirmed framework -> the framework slug written into `project-brain/config/runtime.json` (`generic` when none is confirmed).

## Validation Rules (profile-synthesizer self-checks)

- Section 1 MUST list at least one edition and MUST cite the interview as its source (never assumed).
- Every line in sections 2-8 (including 3.1 and 3.2) MUST carry a confidence tag; every section 8 finding MUST also carry source type.
- Section 8.5 MUST distinguish discovered statuses from evidenced transitions; section 8.6 MUST state permission-matrix completeness; section 8.8 MUST distinguish risk indicators from documented severity/approval.
- Section 11.1's skill list MUST be derivable from sections 2-4 and 8 (no framework-specialty skill for a signal not `confirmed`/`inferred` present in 3.1, no frontend group unless 3.2's verdict says it applies, no integration skill for an integration not in section 4, no domain skill without a cohesive section 8.11 candidate). Every entry MUST carry a one-line, target-specific description - never generic boilerplate reused across projects (the 18 process/workflow skills may share one group sentence since their mechanic is fixed, but MUST still be listed by name).
- Section 11.2's agent/command counts MUST be arithmetically consistent with the 11.1 skill count (including the group breakdown) and section 1's selected editions.
- Section 12 MUST list cohesive durable concepts composed only from `confirmed` facts across sections 2-8, MUST cite canonical sources, and MUST NOT include any `inferred`/`unknown` fact, raw incident data, customer data, or full copied source documents.
- No secrets or credential values may appear anywhere in the profile.
- A `confirmed` integration MUST cite runtime wiring, not merely a `composer.json` entry.
