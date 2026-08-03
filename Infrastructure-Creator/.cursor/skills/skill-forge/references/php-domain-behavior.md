# PHP Domain-Behavior Generation Reference

Used by `skill-forge`, `policy-forge`, and `memory-seed` to turn profile section 8 into useful target infrastructure without inventing business behavior or multiplying shallow skills.

## Core Rule

Behavioral findings enrich existing skills first. Generate a separate domain skill only when section 8.11 identifies one bounded context with:

1. multiple coherent `confirmed` rules;
2. canonical source paths;
3. a distinct review purpose not already owned by a universal/specialty skill;
4. enough content to guide real decisions, not merely restate facts.

Never generate one skill per rule, role, entity, status, risk, test, or table.

## Enrichment Map

- `requirements-analyst`: load project vocabulary, sources of truth, known invariants, lifecycle rules, roles/permissions, audit obligations, forbidden/high-risk changes, and critical acceptance scenarios before decomposing requirements.
- Architecture skill / `architecture-implementer`: preserve bounded-context ownership and identify which layer owns each invariant, transition, permission, and side effect.
- `api-designer`: enforce confirmed permissions, lifecycle guards, source-authority rules, idempotency/external-contract behavior, and error responses without exposing client-controlled state transitions.
- `database-designer`: map confirmed integrity invariants to constraints/indexes/transaction boundaries where appropriate; never infer business rules from schema shape alone.
- `testing`: require confirmed critical scenarios, denied paths, allowed/forbidden transitions, audit emission, retries/idempotency, and historical regression rules when the changed scope intersects them.
- `code-review`: identify which invariants, permissions, transitions, audit obligations, sources of truth, and critical scenarios a diff can affect.
- `security-review`: verify object-level/tenant ownership, authorization enforcement at every entry point, sensitive-state transitions, audit integrity, and non-logging of sensitive values.
- `debugging`: point to confirmed incident-prevention rules, runbooks, and operational sources without copying raw incident content.
- `documentation-generator`: preserve project-specific authority and update the canonical source instead of creating competing documentation.

Every enrichment must cite section 8's canonical source. If section 8 says `unknown`, the generated skill must ask/verify rather than state a rule.

## Evidence-Gated Domain Skill Shape

A domain skill such as `billing-rules-review` should contain:

- bounded-context purpose and trigger examples;
- canonical sources and their authority scope;
- confirmed vocabulary and central entities;
- critical invariants;
- proven lifecycle transitions and guards (statuses alone are not transitions);
- observed permissions and matrix-completeness caveat;
- audit obligations;
- high-risk/forbidden behavior only where governance is documented;
- critical regression scenarios;
- a review procedure that traces a proposed change through all affected rules;
- explicit unknowns/contradictions that require human confirmation.

Its frontmatter `related` should point to generated `requirements-analyst`, `code-review`, `testing`, `security-review`, `memory-bank`, and relevant architecture/integration/specialty skills.

## Operational `memory-bank` Skill (Always Generated)

`memory-seed` creates the shared files; `memory-bank` is the day-to-day operational skill that makes them usable. Generate `memory-bank` for every target in every selected edition and include it in skill/agent/command arithmetic. It is one quarter of the memory quartet - `project-brain`, `checkpoint`, and `memory` are its governed/manual companions, contracted in `references/php-process-skills.md` ("The Memory Quartet"); keep the boundary crisp: `memory-bank` owns durable reusable knowledge and approved-promotion application, never active task state.

Required modes:

- **retrieve**: search `memory-bank/INDEX.md`, then read only relevant active chunks and revalidate canonical sources before relying on them;
- **capture**: create one cohesive durable concept from confirmed evidence; never copy a full spec/schema/test inventory;
- **update/supersede**: never rewrite history silently; update verification metadata or supersede a stale chunk with links;
- **audit**: run `memory-bank/scripts/validate.py`, check source existence/current truth, and report stale/conflicting chunks;
- **initialize**: only when the bank is absent; generation normally delegates this to `memory-seed`.

Authority and safety:

- policy/current canonical specs/code/migrations/tests outrank memory;
- memory is indexed context, not a second source of truth;
- no secrets, credentials, customer data, raw logs, or incident payloads;
- one cohesive concept per chunk, with sources and consequences;
- distinguish confirmed repository evidence from interview answers;
- surface contradictions rather than resolving them silently.

## Memory Selection

Seed or capture cohesive concepts such as:

- project-specific source authority;
- a bounded-context invariant set;
- one lifecycle with confirmed guards and consequences;
- one permission/ownership rule set for a named surface;
- one audit obligation;
- one integration contract;
- one sanitized incident-prevention rule.

Do not seed:

- one chunk per table, class, status, permission row, or test;
- copied OpenAPI/schema/spec content;
- raw incident narratives or production data;
- inferred/unknown behavior presented as fact.
