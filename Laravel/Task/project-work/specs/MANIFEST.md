# Project: [Project Name]

[2-3 sentence core purpose - will be filled by first task]

## Specs Index

| File | Purpose | Depends On | Last Updated |
|------|---------|------------|--------------|
| architect-architecture.md | System design, components, data flow | - | - |
| api-designer-spec.md | Endpoints, schemas, authentication | architect-architecture | - |
| frontend-design-spec.md | Pages, components, state management | architect-architecture, api-designer-spec | - |
| docs-generator-implementation.md | Build process, deployment, tooling | - | - |

## Key Decisions

[Will be populated by architect agent as decisions are made]

## Tech Stack

[Will be populated by skills as project grows]

---

## PracticePerfect Specs Index

Client-derived specs for the PracticePerfect platform, to be built under
`Task/app/`. Source epics live in `Task/Epics/` and are read-only client-owned
material. Build order derives from each epic's own "Required Before This Epic":
01 -> 02 -> {03, 04} -> 05 -> {06, 07, 08}.

**Provenance.** The files below were produced during the Symfony edition's run
and carried over here. `Laravel/Task/Epics/` is byte-for-byte identical to
`Symfony/Task/Epics/`, and the `requirements-analyst` agent definition is
byte-for-byte identical between the two editions — so re-deriving the
requirements would have produced these same documents at full cost. The
carry-over is a measured equivalence, not a shortcut.

The **Reuse** column is the part that matters. Verify it before trusting a file:
`grep -licE 'doctrine|twig|symfony'` returns 0 for every row marked *as-is* and
non-zero for every row marked *translate*.

| File | Purpose | Reuse | Last Updated |
|------|---------|-------|--------------|
| requirements-analyst-epic-01-user-management-spec.md | Epic-01 user management, auth, roles, parent-child (78 AC) | **as-is** — framework-neutral | 2026-08-09 |
| requirements-analyst-epic-02-event-management-spec.md | Epic-02 events, RSVP, capacity, coach assignment, attendance (70 AC) | **as-is** | 2026-08-09 |
| requirements-analyst-epic-03-crm-players-spec.md | Epic-03 CRM, segmentation, labels, flags, notes (70 AC) | **as-is** | 2026-08-09 |
| requirements-analyst-epic-04-lp-content-spec.md | Epic-04 playlists, drills, paywall, progress (43 AC) | **as-is** | 2026-08-09 |
| requirements-analyst-epic-05-payments-tokens-spec.md | Epic-05 Stripe Connect, tokens, subscriptions, refunds (38 AC) | **as-is** | 2026-08-09 |
| requirements-analyst-epic-06-marketing-growth-spec.md | Epic-06 referrals, rewards, coupons (33 AC) | **as-is** | 2026-08-09 |
| requirements-analyst-epic-07-super-admin-spec.md | Epic-07 impersonation, toggles, audit log (41 AC) | **as-is** | 2026-08-09 |
| requirements-analyst-epic-08-forms-registration-spec.md | Epic-08 camp/evaluation forms, conversion (46 AC) | **as-is** | 2026-08-09 |
| requirements-analyst-open-questions.md | Question register; Section A carries owner decisions A1–A12 and the fee call, all settled | **as-is** — product decisions, not technical | 2026-08-09 |
| council-sharelink-tenant-resolution.md | Verdict on the ShareLink tenant-resolution gap | **translate** — the *finding* is framework-independent and expensive to rediscover; the recommended wiring names Symfony mechanisms (`TenantResolver`, `kernel.request`) that must become Laravel middleware and a resolver binding | 2026-08-09 |
| database-designer-schema.md | 58 tables, RLS policies, ledger invariants, fee rounding, migration order | **translate** — table design, PostgreSQL RLS and the integer fee formula port directly; every Doctrine mapping note must be redone as Eloquent models and Laravel migrations | 2026-08-09 |

### Not carried over, and why

`architect-architecture.md`, `api-designer-spec.md`, `frontend-design-spec.md`
and `security-voter-designer-design.md` are deliberately absent. Their content
is Symfony mechanism rather than product decision — Doctrine filters, Voters,
Messenger, Twig, AssetMapper — and the Laravel equivalents (global scopes,
Policies and Gates, Queues, Blade, Vite) are different enough that translating
prose would be slower and less trustworthy than deriving them natively.
`Task/designs/DESIGN_TOKENS.md` also differs between editions: the Laravel copy
names Tailwind, Blade, Livewire and `resources/css/app.css`.

The tenancy *decisions* those documents record are worth carrying into the
Laravel design as inputs: five-layer enforcement with PostgreSQL Row-Level
Security, three database roles, a startup gate, an append-only token ledger with
immutability enforced at the database-privilege level, and the two global tables
the resolver must read before a tenant exists.

---

*This manifest is updated automatically by architect, api-designer, and frontend-design skills.*
*See `../spec-desc.md` for specification structure guidelines.*
