# Project Epic Areas Plan

## Purpose

This plan replaces the previous sports/training backlog with Project work derived from feature-bearing commits reachable from the repository `HEAD`, plus one explicitly new Stripe platform-subscription epic.

Merge-only commits, source-control cleanup, and review-only fixes are represented by the feature they complete rather than repeated as separate work items.

## Status legend

- **Existing baseline** — implemented or established by the Project history and documented here as the product capability baseline.
- **New planned scope** — approved for the backlog but not found in Project history.

## Epic map

| Epic | Area | Status | Primary result |
|---|---|---|---|
| 01 | Tenant, Users & Access | Existing baseline | Tenant isolation, registration, roles, users, contacts, and explicit grants |
| 02 | Authentication, MFA & API Security | Existing baseline | Hardened password/MFA sessions and scoped bearer tokens |
| 03 | Building Register, Inspections, Defects & Governance | Existing baseline | Contract-aligned digital building-register domain |
| 04 | Projects, Processes, Tasks & Deadlines | Existing baseline | Authorized operational work and phase/deadline handling |
| 05 | Documents, Versioning, OCR, Search & Export | Existing baseline | Confidential, revision-proof, searchable document lifecycle |
| 06 | Admin, Dashboards, Branding & Notifications | Existing baseline | Role-focused, tenant-branded administration experience |
| 07 | Stripe Platform Subscriptions & Billing | New planned scope | Mandant subscription Checkout, portal, invoices, webhooks, and entitlements |
| 08 | Infrastructure, Storage, Search & Deployment | Existing baseline + Stripe support | Reliable runtime and delivery platform |

## Dependency flow

```text
Epic 01 Tenant and access
  ├── Epic 02 Authentication and API security
  ├── Epic 03 Building-register domain
  │     ├── Epic 04 Projects, processes, tasks and deadlines
  │     └── Epic 05 Documents, OCR, search and history
  ├── Epic 06 Admin, dashboards, branding and notifications
  └── Epic 07 Stripe platform subscriptions

Epic 08 Infrastructure supports every epic.
```

## Delivery order

The history-derived epics describe the current baseline and should be used to verify or reconstruct that baseline in another implementation. For new work, deliver Stripe in this order:

1. Reconfirm Epic 01 Mandant and billing-administrator authorization.
2. Add Stripe billing identity and allow-listed plans.
3. Add hosted Checkout and Customer Portal.
4. Add signed, idempotent webhook processing and local subscription projection.
5. Enforce entitlements while retaining the billing-management path.
6. Add invoice/status UI, monitoring, and resynchronization operations.

## Commit-history coverage

| History area | Covered by |
|---|---|
| Initial Project domain, Mandant, voter/defaults | Epics 01 and 03 |
| Search index, contacts, deadlines, fixtures, translations | Epics 01, 04, 05, and 06 |
| Dashboard, building detail, menus, overlay positioning, local dates, notifications | Epic 06 |
| Contract enums, names, translations, classifications, external references | Epic 03 |
| Person/Company split | Epics 01 and 03 |
| BuildingComponent, BuildingDefect, InspectorRequirement, Inspection | Epic 03 |
| BuildingRegister and re-parented Documents/Components/Projects | Epics 03, 04, and 05 |
| Governance entities, defect remediation, typed Change hierarchy | Epic 03 |
| Document interoperability and inline pickers | Epics 03, 05, and 06 |
| Registration, inactive activation, active-user login | Epics 01 and 02 |
| Role dashboards | Epic 06 |
| Multi-tenancy and object/project grants | Epic 01 |
| Password login/reset/change and Remember Me | Epic 02 |
| WebAuthn/TOTP MFA, rate limits, remembered device, enrollment gate, key rotation | Epic 02 |
| Stateless bearer token after MFA, rotating refresh-token families, reuse detection, and family revocation | Epic 02 |
| Project phases/status transitions and contract fields | Epic 04 |
| Per-Mandant host branding, private host-owned logo delivery, and protected mail sender identity | Epic 06 |
| Nested BuiltStructure creation with role-gated writes and server-assigned Mandant defaults | Epics 01 and 03 |
| Shared current-password reauthentication, throttling, and security audit events | Epic 02 |
| Confidential document policy | Epic 05 |
| Document versioning, OCR, metadata/full-text search, as-of reconstruction | Epic 05 |
| S3, Meilisearch, cron, supervisor, Jenkins, DigitalOcean deployment | Epic 08 |
| Stripe platform subscription billing | Epic 07 — new scope |

## Shared acceptance criteria

- Every tenant-owned operation enforces Mandant isolation on the server.
- Object/project access grants are honored consistently across admin, API, search, export, and download paths.
- Confidential documents do not leak through metadata, counts, nested fields, search, or storage URLs.
- Security-sensitive flows fail closed, are rate-limited where exposed, and never log secrets.
- New nested tenant-owned records inherit their Mandant server-side and cannot cross tenant boundaries.
- Branding files and platform mail sender identity cannot be selected or changed across Mandant or role boundaries.
- Refresh-token rotation, reuse detection, and family revocation invalidate related bearer access consistently.
- History-derived items are not presented as new Stripe functionality.
- Stripe billing belongs to the Mandant platform subscription, not construction projects, contractors, tasks, or token wallets.
- Each framework mirror contains the same filenames and byte-identical content.

## Explicitly outside this plan

- Sports, players, trainers, events, tournaments, token wallets, and marketing-growth features from the replaced files.
- Stripe Connect, marketplace payments, contractor invoices, construction milestone payments, and metered billing.
- Speculative workflow, BIM, SSO, SMS, push-notification, or infrastructure expansions not supported by Project history.
