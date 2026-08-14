# Project Profile Schema

The profile and its machine-readable skill plan are the hand-off artifacts between Phase 1 (scan) and Phase 2 (generate). `profile-synthesizer` writes both; every forge reads them after re-validating evidence against current target files.

- Human review: `tasks/TASK-{N}/infra-scan-project-profile.md`
- Machine contract: `tasks/TASK-{N}/skill-generation-plan.json`

Every factual line MUST carry a confidence tag - `(confirmed - path:L#)`, `(inferred - reason)`, or `(unknown)` - so generated artifacts remain auditable back to evidence. Behavioral findings in section 8 MUST additionally name the source type (`spec/ADR`, `test`, `database constraint`, `workflow configuration`, `authorization rule`, `domain code`, `application code`, `configuration`, or `interview answer`) because confirmed implementation and confirmed policy are not equal authorities.

## Machine-Readable Skill Plan

The JSON top level MUST contain exactly these required members (extensions require a schema-version change):

```json
{
  "schema_version": "1.0",
  "catalog_version": "2.5.0",
  "target_root": "/absolute/path/to/target",
  "profile": "tasks/TASK-001/infra-scan-project-profile.md",
  "evidence": [],
  "skills": [],
  "rejected_candidates": []
}
```

Top-level membership is exact and `schema_version` is exactly `1.0`.
`catalog_version` must equal
`skill-forge/references/candidate-registry.json`. Every registry candidate must
appear in exactly one disposition: a selected skill's `selection_gate` or
`rejected_candidates` (family candidates may select multiple concrete skills).
`profile` must identify the sibling Profile in the same
`tasks/TASK-{N}/`. By contrast, evidence paths and generated-skill source
citations MUST be canonical paths relative to `target_root`; they MUST NOT be
absolute, contain `..`, or point into `tasks/TASK-{N}/`.

Each `evidence[]` entry is:

```json
{
  "id": "EV-0001",
  "path": "composer.json",
  "source_type": "configuration",
  "authority": "Declares runtime dependencies and PHP constraints",
  "confidence": "confirmed",
  "line_range": {"start": 8, "end": 19},
  "fingerprint": "sha256:<digest>",
  "supported_claims": ["Laravel is a runtime dependency"]
}
```

Exactly one of `path` or `url` is required. `id`, `source_type`, `authority`,
`confidence`, and non-empty `supported_claims` are required. Repository-path
evidence also requires the current `sha256:<digest>` fingerprint; `line_range`
is optional but may not exceed the source. URLs rely on their recorded
authority/version and do not require a local fingerprint.

Each `skills[]` entry is one complete, independently actionable contract:

```json
{
  "name": "testing",
  "category": "universal",
  "kind": "project-adapted",
  "phase": "execution",
  "necessity_rationale": "The target has an evidenced test suite and project-specific test conventions that require operational guidance.",
  "selection_gate": {
    "catalog": "php-frameworks.md#testing",
    "candidate_id": "testing",
    "candidate": "testing",
    "conditions": [
      {
        "requirement": "A configured target test suite and conventions exist",
        "evidence_ids": ["EV-0007", "EV-0008"],
        "status": "satisfied",
        "explanation": "The cited PHPUnit config and base test case prove distinct operational guidance."
      }
    ],
    "distinct_value_from": ["coding", "systematic-debugger"]
  },
  "triggers": {
    "positive": ["A change requires tests under tests/Feature"],
    "negative": ["The request is only to investigate a production symptom"]
  },
  "evidence_ids": ["EV-0007", "EV-0008"],
  "source_paths": ["phpunit.xml", "tests/TestCase.php"],
  "owned_scope": ["Select and implement tests using the target's suite structure"],
  "excluded_scope": ["Root-cause investigation", "Release orchestration"],
  "required_procedure_roles": [
    {"role": "load-evidence", "requirements": ["Read the cited test configuration and changed behavior"]},
    {"role": "execute", "requirements": ["Choose test level and add target-specific cases"]},
    {"role": "verify", "requirements": ["Run the narrow suite, then required broader checks"]}
  ],
  "decision_points": [
    {"question": "Which test level protects this behavior?", "branches": ["unit", "integration", "feature/end-to-end"]}
  ],
  "verification": ["Use the evidenced test command and report failures"],
  "output_contract": ["Changed tests or a test plan", "Commands and results"],
  "failure_handling": ["Stop when required tooling is unavailable and report N/A", "Do not weaken assertions to force a pass"],
  "related_skills": ["coding"],
  "nearest_siblings": [{"name": "systematic-debugger", "boundary": "Finds root cause; testing protects expected behavior."}],
  "writes": ["tests/**"],
  "fixed_blocks": [
    {"id": "safety.no-secrets", "version": "1.0", "content": "Exact approved text"}
  ]
}
```

All shown members except `fixed_blocks` are required, including non-empty positive and negative triggers, owned and excluded scope, required procedure roles, decision points, verification, output, failure handling, siblings, and writes (use an empty array only when the skill is intentionally read-only). `kind` distinguishes `runtime-fixed` from `project-adapted`, `integration`, `specialty`, and `domain-review`. `fixed_blocks`, when used, contains explicit `id`, `version`, and exact `content`; only that exact block is exempt from duplication checks, and it never exempts the skill-specific procedure.

Every non-runtime `selection_gate.conditions[]` entry must be `satisfied`, cite
evidence already listed by the skill, and explain a claim supported by that
evidence. `distinct_value_from` records adjacent candidates considered during
pruning. Runtime-fixed memory skills may cite an empty evidence list only when
the condition names the guaranteed generated runtime.

`evidence_ids` MUST resolve to `evidence[]`. `source_paths` is the de-duplicated target-relative subset used by the skill. Every `related_skills` and `nearest_siblings[].name` MUST exist in the same plan. A skill contract cannot rely on another skill to supply its purpose, inputs, procedure, verification, output, or failure behavior.

Each rejected catalog candidate is recorded so inventory pruning is auditable:

```json
{
  "candidate_id": "caching-strategy",
  "name": "caching-strategy",
  "category": "specialty",
  "reason": "No confirmed cache usage or invalidation ownership",
  "missing_evidence": ["runtime cache calls", "write-path invalidation"]
}
```

`candidate_id` and `catalog` must resolve exactly to the registry entry and its
real Markdown anchor. This prevents a plan from inventing a catalog reference
or silently omitting candidates from the selected/rejected inventory.

The runtime-fixed memory quartet may have empty `evidence_ids`/`source_paths` during synthesis because its necessity comes from runtime files that `memory-seed` is contractually guaranteed to create in the same generation run. Its `necessity_rationale` and `writes` must name that dependency. The authored skills must refer to those future files by target-relative paths such as `memory-bank/scripts/context.py`, never by generator task/staging paths.

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

### 11.1 Skills To Generate
Inventory is evidence-gated across all categories. There are no fixed category counts. The memory quartet is the sole exception because the generated runtime always exists.

- Plan file: `tasks/TASK-{N}/skill-generation-plan.json`
- Selected: one line per skill: `[name]` (`[category]`) - `[necessity rationale]` - evidence: `[EV ids and target-relative paths]`.
- Rejected candidates: one line per considered catalog candidate: `[name]` - skipped because `[selection trigger or required evidence not satisfied]`.
- Runtime-fixed: `memory-bank`, `project-brain`, `checkpoint`, `memory` - justified by the always-installed memory runtime; each still has its own complete contract.
- Inventory check: `[skills.length]` selected entries = `[skills.length]` complete JSON contracts.

### 11.2 Agents & Commands Preview
- Skill count breakdown: derive all category counts from the JSON inventory and sum them to `skills.length`; categories with zero entries remain zero. Do not use baseline constants.
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
- `skill-forge`: `skill-generation-plan.json` plus the exact profile/evidence slices referenced by each contract; it authors one skill or a small sibling group at a time.
- `agent-forge`/`command-forge`: the final skill list from `skill-forge` + section 1 (which editions); section 11.2's counts are what the user was shown to expect.
- `hook-forge`: sections 2, 4, 5, 6 + section 1 -> the four enforcement hooks tuned to real tooling/risks plus the two working-memory hooks calling `memory-bank/scripts/context.py`, for the selected editions only.
- `memory-seed`: section 12 -> the authoritative, pre-reviewed list of cohesive concepts to seed, each already cited to canonical source files; plus section 2's confirmed framework -> the framework slug written into `project-brain/config/runtime.json` (`generic` when none is confirmed).

## Validation Rules (profile-synthesizer self-checks)

- Section 1 MUST list at least one edition and MUST cite the interview as its source (never assumed).
- Every line in sections 2-8 (including 3.1 and 3.2) MUST carry a confidence tag; every section 8 finding MUST also carry source type.
- Section 8.5 MUST distinguish discovered statuses from evidenced transitions; section 8.6 MUST state permission-matrix completeness; section 8.8 MUST distinguish risk indicators from documented severity/approval.
- Section 11.1 MUST equal JSON `skills[]` exactly. Every selected candidate satisfies its reference contract; every rejected candidate records the missing trigger/evidence. Only the memory quartet is unconditional.
- JSON MUST satisfy the field, uniqueness, reference, canonical-path, and completeness rules above. Grouped summaries are invalid.
- Section 11.2's agent/command counts MUST be arithmetically consistent with `skills.length`, dynamic category counts, and section 1's selected editions.
- Section 12 MUST list cohesive durable concepts composed only from `confirmed` facts across sections 2-8, MUST cite canonical sources, and MUST NOT include any `inferred`/`unknown` fact, raw incident data, customer data, or full copied source documents.
- No secrets or credential values may appear anywhere in the profile.
- A `confirmed` integration MUST cite runtime wiring, not merely a `composer.json` entry.
