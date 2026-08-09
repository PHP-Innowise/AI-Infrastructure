# Project: Symfony Layered Architecture Accelerator

An AI-assisted development accelerator for Symfony 7.4 LTS and Symfony 8.1 projects. It provides native Claude Code, Cursor, and Codex workflows centered on pragmatic Controller -> Service -> Repository architecture and Symfony conventions.

## Specs Index

| File | Purpose | Depends On | Last Updated |
|------|---------|------------|--------------|
| architect-architecture.md | System design, components, data flow | - | - |
| api-designer-spec.md | Endpoints, schemas, authentication | architect-architecture | - |
| frontend-design-spec.md | Pages, components, state management | architect-architecture, api-designer-spec | - |
| docs-generator-implementation.md | Build process, deployment, tooling | - | - |

## Key Decisions

- Target Symfony 7.4 LTS and Symfony 8.1 while detecting each consuming project's installed versions.
- Use `.agents/skills` as the configured canonical source for shared skill parity, mirror Claude and Cursor semantics natively, and keep Codex support files under `.codex`.
- Enforce Controller -> Service -> Repository pragmatically, without requiring pass-through layers or interfaces without a real boundary.

## Tech Stack

- PHP 8.2+ for Symfony 7.4 LTS; PHP 8.4+ for Symfony 8.1.
- Symfony components and conventions, Doctrine ORM/Migrations, Symfony Security, Messenger, Forms, Validator, Serializer, Twig, and Symfony UX as installed by the consuming project.

---

## PracticePerfect Specs Index

Client-derived specs for the PracticePerfect platform built under `Task/app/`.
Source epics live in `Task/Epics/` and are read-only client-owned material;
these files derive from them and cite them by file, heading text and line
number. Build order derives from each epic's own "Required Before This Epic":
01 -> 02 -> {03, 04} -> 05 -> {06, 07, 08}.

| File | Purpose | Depends On | Last Updated |
|------|---------|------------|--------------|
| requirements-analyst-epic-01-user-management-spec.md | Epic-01 user management, auth, roles, parent-child | - | 2026-08-09 |
| requirements-analyst-epic-02-event-management-spec.md | Epic-02 events, RSVP, capacity, coach assignment, attendance | epic-01 | 2026-08-09 |
| requirements-analyst-epic-03-crm-players-spec.md | Epic-03 CRM, segmentation, labels, flags, notes, Quick View | epic-01, epic-02 | 2026-08-09 |
| requirements-analyst-epic-04-lp-content-spec.md | Epic-04 Learn/Practice playlists, drills, paywall, progress | epic-01, epic-02 | 2026-08-09 |
| requirements-analyst-epic-05-payments-tokens-spec.md | Epic-05 Stripe Connect, tokens, subscriptions, refunds, fees | epic-01, epic-02, epic-04 | 2026-08-09 |
| requirements-analyst-epic-06-marketing-growth-spec.md | Epic-06 referrals, rewards, coupons, campaign analytics | epic-01, epic-05 | 2026-08-09 |
| requirements-analyst-epic-07-super-admin-spec.md | Epic-07 impersonation, feature toggles, audit log, Event Master | epic-01, epic-02, epic-05 | 2026-08-09 |
| requirements-analyst-epic-08-forms-registration-spec.md | Epic-08 camp/evaluation forms, external registration, conversion | epic-01, epic-05 | 2026-08-09 |
| requirements-analyst-open-questions.md | Consolidated question register; Section A blocks Phase 2 design | all epic specs | 2026-08-09 |
| architect-architecture.md | Platform shape, tenancy, modules, layering, authorization | all epic specs | 2026-08-09 |
| api-designer-spec.md | ~130 HTTP routes across nine modules, voters, Stripe webhook contract, public tenant resolution, impersonation | architect-architecture, all epic specs | 2026-08-09 |
| frontend-design-spec.md | CSS token layers, Twig hierarchy, component inventory, ~65 screens, WCAG 2.2 AA contract | architect-architecture, DESIGN_TOKENS.md | 2026-08-09 |
| database-designer-schema.md | 58 tables (18 global, 40 trainer-scoped), RLS policies, ledger invariants, fee rounding, migration order | architect-architecture, all epic specs | 2026-08-09 |
| council-sharelink-tenant-resolution.md | Verdict on the ShareLink tenant-resolution gap; amendments applied to architect-architecture | architect-architecture, api-designer-spec | 2026-08-09 |

---

*This manifest is updated automatically by architect, api-designer, and frontend-design skills.*
*See `../spec-desc.md` for specification structure guidelines.*
