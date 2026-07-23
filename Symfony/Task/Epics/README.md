# Project Epic Guide

This directory is a reusable capability backlog for a tenant-aware business application. The epic files describe outcomes, implementation tasks, acceptance criteria, dependencies, and reference-history evidence. They are not a framework-specific implementation plan or a claim that every consuming project already provides the listed capabilities.

## Start here

1. Read [Epic_Areas_Plan.md](Epic_Areas_Plan.md) for scope, dependencies, delivery order, and shared acceptance criteria.
2. Choose the epic that owns the capability instead of copying a task into several epics.
3. Verify each checkbox against the consuming project's code, configuration, migrations, and tests.
4. Mark each task as implemented, planned, or not applicable.
5. Replace reference commit evidence with links or commit identifiers from the consuming project.
6. Implement prerequisites before dependent epics and verify the epic's acceptance criteria end to end.

An **existing baseline** means the capability existed in the reference history. It must still be verified in the consuming project. **New planned scope** means the capability was intentionally added to the backlog and requires implementation.

## Epic index

| Epic | General capability | Use it for |
|---|---|---|
| [01 — Tenant, Users & Access](Epic-01_Tenant_Users_Access_SPEC.md) | Identity, ownership, and authorization boundaries | Organizations, users, roles, contacts, and explicit object access |
| [02 — Authentication, MFA & API Security](Epic-02_Authentication_MFA_API_Security_SPEC.md) | Account and API security | Passwords, sessions, MFA, reauthentication, bearer tokens, and revocation |
| [03 — Building Register, Inspections, Defects & Governance](Epic-03_Building_Register_Inspections_Defects_Governance_SPEC.md) | Core regulated domain | Aggregate roots, owned assets, inspections, defects, decisions, and auditability |
| [04 — Projects, Processes, Tasks & Deadlines](Epic-04_Projects_Processes_Tasks_Deadlines_SPEC.md) | Operational workflow | Projects, phases, work items, responsibilities, statuses, and deadlines |
| [05 — Documents, Versioning, OCR, Search & Export](Epic-05_Documents_Versioning_OCR_Search_Export_SPEC.md) | Document lifecycle | Confidential files, versions, extraction, search, historical state, and export |
| [06 — Admin, Dashboards, Branding & Notifications](Epic-06_Admin_Dashboards_Branding_Notifications_SPEC.md) | Administration experience | CRUD screens, navigation, dashboards, tenant branding, messages, and translations |
| [07 — Stripe Platform Subscriptions & Billing](Epic-07_Stripe_Platform_Subscriptions_Billing_SPEC.md) | Optional platform billing | Hosted subscription checkout, portal, invoices, webhooks, and entitlements |
| [08 — Infrastructure, Storage, Search & Deployment](Epic-08_Infrastructure_Storage_Search_Deployment_SPEC.md) | Runtime and delivery | Application hosting, data services, storage, search, workers, schedules, and CI/CD |

## Generalize the domain

Translate domain names to the consuming project's language while preserving their responsibility and security boundary.

| Project term | General concept |
|---|---|
| Mandant | Tenant, organization, account, or workspace |
| BuildingRegister | Root business record or aggregate |
| BuiltStructure, Building, Unit | Hierarchical tenant-owned assets or entities |
| Project, Process, Task | Work hierarchy and execution units |
| Document, Classification, ExternalReference | Content, metadata, and integration references |
| Platform, tenant, operational, and authority roles | The consuming project's role and permission matrix |
| Stripe subscription | Optional tenant-level platform entitlement |

Do not copy names, roles, vendors, or storage choices when the target domain uses different concepts. Do preserve server-side tenant isolation, authorization, auditability, confidential-data controls, idempotency, and failure behavior.

## Adapt the backlog

- Remove tasks unsupported by real requirements; do not keep them for possible future use.
- Add a capability to the closest existing epic before creating another epic.
- Replace Symfony, Nuxt, MySQL, S3, Meilisearch, Jenkins, or Stripe details when the selected framework or platform provides an equivalent.
- Keep task wording testable: identify the actor, protected resource, allowed behavior, and highest-risk failure.
- Treat commit evidence as provenance, not acceptance proof.
- Keep billing optional unless subscriptions are part of the product scope.

## Completion rule

An epic is complete only when its applicable tasks and acceptance criteria are verified in the consuming project. UI hiding does not replace authorization, search indexes do not replace database ownership checks, and external-service redirects or callbacks do not replace verified server-side state.

The `Laravel`, `PHP Core`, and `Symfony` copies of this directory intentionally contain the same planning material. Framework-specific implementation belongs in the selected accelerator; changes to the shared epic backlog must remain synchronized across all three copies.
