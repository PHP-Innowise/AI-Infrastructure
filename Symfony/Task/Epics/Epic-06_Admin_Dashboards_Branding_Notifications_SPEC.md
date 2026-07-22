# Epic 06: Admin, Dashboards, Branding & Notifications

## Outcome

Each role receives a usable Project administration experience with relevant dashboards, correct domain navigation, tenant branding, translations, and operational notifications.

**Scope status:** Existing baseline from Project history.

## Capability tasks

### E06-T01 — Domain administration

- [ ] Provide admin list, detail, create, and update views for supported Project entities.
- [ ] Use tenant-scoped selectors and authorization-aware nested views.
- [ ] Provide dedicated views for BuildingRegister, components, defects, inspections, requirements, decisions, changes, projects, processes, tasks, documents, users, and grants.
- [ ] Keep navigation organized around Project rather than the inherited starter domain.

### E06-T02 — Building and governance views

- [ ] Show the building detail page and Unit tree.
- [ ] Surface governance, defects, requirements, and typed Change records.
- [ ] Support inline classification and external-reference selection where the owner form needs it.
- [ ] Use local time for user-facing dashboard dates.

### E06-T03 — Role dashboards

- [ ] Route users to a landing view appropriate to their role.
- [ ] Show KPIs and deadline tables using only authorized tenant data.
- [ ] Keep authority, platform administration, tenant administration, and operational views distinct.
- [ ] Treat dashboard filtering as presentation on top of server-side authorization.

### E06-T04 — White-label tenant resolution

- [ ] Resolve the Mandant from the configured host.
- [ ] Apply Mandant name, logo, colors, and related branding to the login and admin experience.
- [ ] Reject unknown or ambiguous hosts safely.
- [ ] Prevent host selection from granting access to another Mandant's data.

### E06-T05 — Notifications and messages

- [ ] Send transactional notifications for relevant account and deadline events.
- [ ] Keep templates consistent with Project branding and translations.
- [ ] Avoid duplicate sends when commands or workers retry.
- [ ] Do not include confidential document content in notifications unless the recipient is authorized and the channel is approved.

### E06-T06 — Internationalization and fixtures

- [ ] Maintain contract-aligned English field names with German user-facing descriptions.
- [ ] Keep menu labels, forms, validation messages, and notifications translatable.
- [ ] Provide realistic non-sensitive development fixtures and valid placeholder media.

## Acceptance criteria

- Every role lands on an authorized, relevant dashboard.
- Building, Unit, governance, defect, requirement, and change views are navigable without cross-tenant leakage.
- Host-based branding changes presentation but never authorization.
- User-facing dates use the intended local timezone.
- Retried notification jobs do not send duplicates.

## Commit evidence

- `d7c87ce` — Nuxt admin views adapted to the Project domain.
- `e21fc19`, `d2e6321` — dashboard KPIs, deadline tables, route fixes, and fixtures.
- `2e3e860` — building detail and Unit tree.
- `6bf5a6c`, `e68b607`, `37b3610` — audit navigation, dashboard links/menu, and local-time fixes.
- `55f0046` — setup, password command, and notification-template polish.
- `9cc18da`, `e2fdc01` — governance administration and inline pickers.
- `955254d` — role-based dashboards.
- `e511c9e` — host-based Mandant branding and resolution.
- `bcaeb98` — valid fixture media.

## Dependencies

- Epics 01–05 provide the domain, authorization, workflows, and documents shown here.
- Epic 08 supplies mail and frontend build/deployment.

## Excluded

- Visual page builder or per-user theme editor.
- Marketing website/content management.
- Push and SMS notifications not present in the history.

