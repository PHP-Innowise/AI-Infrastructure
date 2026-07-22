# Epic 04: Projects, Processes, Tasks & Deadlines

## Outcome

Building-register work is organized into projects, processes, tasks, and due-date operations without breaking tenant or object-level access boundaries.

**Scope status:** Existing baseline from Project history. This epic documents the current Task capability; it does not invent a separate workflow engine.

## Capability tasks

### E04-T01 — Projects under the BuildingRegister

- [ ] Manage Projects as children of a BuildingRegister.
- [ ] Store contract-aligned project type and group identifiers.
- [ ] Scope project reads and mutations by Mandant and explicit access grants.
- [ ] Prevent a Project from moving to another tenant through ordinary updates.

### E04-T02 — Project phases and status transitions

- [ ] Represent the supported project phases and statuses.
- [ ] Permit only declared transitions.
- [ ] Reject invalid transitions consistently in admin and API paths.
- [ ] Record who changed the project status and when.

### E04-T03 — Processes

- [ ] Create and manage tenant-scoped Processes.
- [ ] Use Processes to group related operational work.
- [ ] Keep Process selection limited to records visible to the current user.
- [ ] Expose Process management in the admin and GraphQL API.

### E04-T04 — Tasks

- [ ] Create a Task with name and optional description.
- [ ] Associate a Task with an optional Process.
- [ ] Record the Task Mandant and author.
- [ ] Enforce tenant scoping on Task list, read, create, update, and delete operations.
- [ ] Expose Task administration and GraphQL CRUD through the established scoped-resolver pattern.

### E04-T05 — Deadlines and inspection/defect follow-up

- [ ] Calculate and surface relevant due dates for inspections, requirements, projects, and remediation.
- [ ] Run deadline processing through the existing console/cron path.
- [ ] Avoid duplicate notifications or repeated state changes when deadline processing reruns.
- [ ] Show upcoming and overdue items in dashboard tables.

## Acceptance criteria

- A user sees only Projects, Processes, and Tasks allowed by Mandant and grant rules.
- Invalid project status transitions are rejected without partial updates.
- A Task persists its author and tenant and may be linked to one Process.
- Deadline processing is safe to rerun and does not cross tenant boundaries.
- Overdue inspections, requirements, defects, or project work are visible from the admin dashboard where supported.

## Commit evidence

- `9becdd9` — Deadline command, search integration, fixtures, and translations.
- `e21fc19` — dashboard KPIs and deadline tables.
- `d6da9e0`, `c2fcbab` — BuildingRegister root and Project re-parenting.
- `05d633b`, `4f5876f` — project access grants.
- `96638cb`, `6e115ec` — project contract fields and engine-state fields.
- `2ab0261` — project phases and status transitions.
- Current baseline: `src/Entity/Project.php`, `Process.php`, `Task.php` and their tenant-scoped GraphQL/admin definitions.

## Dependencies

- Epic 01 supplies tenant and project grants.
- Epic 03 supplies BuildingRegister, inspections, defects, and requirements.
- Epic 06 supplies dashboards and notifications.
- Epic 08 supplies cron/worker execution.

## Excluded

- General-purpose workflow designer.
- Kanban, time tracking, recurring tasks, and task dependencies not present in the commit history.
- Construction invoice or milestone-payment workflows; platform billing belongs to Epic 07.

