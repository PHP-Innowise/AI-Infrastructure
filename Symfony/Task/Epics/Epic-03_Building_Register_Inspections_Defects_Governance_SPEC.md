# Epic 03: Building Register, Inspections, Defects & Governance

## Outcome

Each Mandant maintains a contract-aligned digital building register containing structures, units, components, inspections, defects, requirements, classifications, references, and traceable governance decisions.

**Scope status:** Existing baseline from Project history.

## Capability tasks

### E03-T01 — Property and building structure

- [ ] Manage property parcels, addresses, geo-information, buildings, BuiltStructures, and Units.
- [ ] Require the relationships that make a building part of a valid structure.
- [ ] Allow authorized tenant administrators to create a BuiltStructure with nested Buildings while assigning missing child Mandants server-side.
- [ ] Display a building detail view with its Unit hierarchy.
- [ ] Keep all roots and descendants inside one Mandant.

### E03-T02 — BuildingRegister root

- [ ] Create one BuildingRegister root per building where required by the domain contract.
- [ ] Parent Documents, BuildingComponents, and Projects under the register.
- [ ] Preserve migrated relationships and reject orphaned or cross-register children.
- [ ] Expose register contents through tenant-scoped admin and API operations.

### E03-T03 — Contract vocabulary and references

- [ ] Use contract-aligned English field names and enum values.
- [ ] Provide German alternative descriptions for user-facing domain terms.
- [ ] Manage reusable Classification and ExternalReference value objects.
- [ ] Allow inline selection of classifications and external references on owning records.
- [ ] Cover defect severity/status, inspector role, component category, approval status, Unit usage category, and Project type/group identifier.

### E03-T04 — Components and inspections

- [ ] Manage BuildingComponents inside a BuildingRegister.
- [ ] Attach Inspections to the inspected component.
- [ ] Record inspector role, dates, results, and contract-required inspection fields.
- [ ] Prevent an inspection from referencing a component outside its Mandant/register.

### E03-T05 — Defects and remediation

- [ ] Record BuildingDefects against the relevant component.
- [ ] Track severity, status, findings, and remediation relationships.
- [ ] Link remediation work to the responsible process/project context where available.
- [ ] Surface open and overdue defects in administrative views.

### E03-T06 — Inspector requirements and decisions

- [ ] Record InspectorRequirements and their approval state.
- [ ] Capture Decisions governing requirements, remediation, and register state.
- [ ] Keep requirement and decision changes tenant-scoped and auditable.
- [ ] Provide dedicated admin views for requirements and decisions.

### E03-T07 — Change hierarchy and auditability

- [ ] Represent addition, deletion, geometry, match, property, and requirement changes as typed Change records.
- [ ] Preserve shared change metadata while exposing subtype-specific data.
- [ ] Make audit records readable from the relevant building-register context.
- [ ] Never permit audit history to be rewritten through normal CRUD screens.

## Acceptance criteria

- A BuildingRegister is the navigable root for its components, projects, and documents.
- Components, inspections, defects, requirements, decisions, and changes cannot cross Mandant or register boundaries.
- Contract enums and classifications are validated consistently in API and admin inputs.
- Nested BuiltStructure creation cannot produce tenantless or cross-Mandant Building children.
- Remediation and governance history remains traceable after ordinary record updates.
- Admin users can inspect the Unit tree, defects, requirements, decisions, and typed changes.

## Commit evidence

- `739ff89`, `e40b976` — initial domain entities and authorization/default assignment.
- `2e3e860` — building detail and Unit tree.
- `4c41615`, `ed1139b` — contract enums, Classification, and ExternalReference.
- `f9ce7d8`, `2827969`, `ff855f5` — contract field names, descriptions, translations, and enum values.
- `1044dbc` — Person and Company split.
- `b83decb`, `edd62be` — BuildingComponent, BuildingDefect, InspectorRequirement, and inspection restructuring.
- `d6da9e0`, `c2fcbab` — BuildingRegister root and child re-parenting.
- `5a082eb`, `9981285` — governance entities, remediation, and typed Change hierarchy.
- `3044a38` — document interoperability fields and classification relationship.
- `9cc18da`, `e2fdc01` — administration views and inline reference pickers.
- `96638cb`, `6e115ec`, `7177bbb` — BuiltStructure and remaining contract slots.
- `209e927` — role-gated nested Building writes and Mandant defaults for new entity graphs.

## Dependencies

- Epic 01 supplies Mandant isolation and grants.
- Epic 04 supplies project/process remediation context.
- Epic 05 supplies documents and historical reconstruction.
- Epic 06 supplies administration views.

## Excluded

- BIM authoring or 3D model editing.
- Automated defect recognition.
- Cross-Mandant building-register federation.
