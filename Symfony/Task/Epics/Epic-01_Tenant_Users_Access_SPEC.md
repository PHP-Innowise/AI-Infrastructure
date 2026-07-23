# Epic 01: Tenant, Users & Access

## Outcome

Project users can register, be activated, work only inside their Mandant, and receive explicit access to the buildings and projects they are allowed to manage.

**Scope status:** Existing baseline from Project history.

## Capability tasks

### E01-T01 — Mandant ownership and isolation

- [ ] Keep Mandant as the ownership boundary for tenant data.
- [ ] Derive tenant ownership consistently for nested records.
- [ ] Scope list, read, create, update, and delete operations to the current Mandant.
- [ ] Prevent cross-Mandant identifiers from being attached through API or admin forms.
- [ ] Restrict Mandant assignment and tenant-administration operations to authorized administrators.

### E01-T02 — Registration and activation

- [ ] Accept the registration data required to create an account.
- [ ] Create new users in an inactive state.
- [ ] Require an authorized activation step before login becomes possible.
- [ ] Keep registration and account errors non-enumerating and rate-limited.

### E01-T03 — Role and user administration

- [ ] Maintain role-based access for platform administration, tenant administration, property management, and authority users.
- [ ] Prevent tenant administrators from managing users outside their Mandant.
- [ ] Show only role-appropriate navigation and landing content.
- [ ] Preserve user status, Mandant membership, and role changes in the audit trail.

### E01-T04 — Object and project access grants

- [ ] Grant a user access to a BuiltStructure or Project explicitly.
- [ ] Apply grants to list queries, nested reads, mutations, and admin pickers.
- [ ] Reject grants when user and target belong to different Mandants.
- [ ] Allow only authorized administrators to attach or remove grants.
- [ ] Keep required building relationships intact when granting access.

### E01-T05 — Contacts

- [ ] Manage Person and Company as separate contact types.
- [ ] Associate contacts only with records visible to the current Mandant.
- [ ] Preserve contract-aligned field names and German alternative descriptions where exposed in the admin.

## Acceptance criteria

- A newly registered account cannot sign in before activation.
- A user cannot enumerate or mutate another Mandant's users, buildings, projects, tasks, documents, or grants.
- An object/project grant expands access only to its intended target and permitted nested data.
- Unauthorized users cannot assign Mandants or grants.
- Role-specific navigation never substitutes for server-side authorization.

## Commit evidence

- `e74cdc3` — remove inherited domain and introduce Mandant.
- `739ff89`, `e40b976` — Project domain entities, voter, and defaults.
- `1044dbc` — split Contact into Person and Company.
- `f181846`, `b4fa5fe` — registration and inactive-user creation.
- `994dbd8`, `a788313` — multi-tenancy and scoped visibility hardening.
- `05d633b`, `4f5876f`, `7177bbb` — per-user BuiltStructure/Project grants and administrative restrictions.

## Dependencies

- Epic 02 supplies authentication and MFA.
- Epic 03 supplies BuiltStructure and BuildingRegister ownership.
- Epic 04 supplies Project ownership.
- Epic 06 supplies role-specific admin screens.

## Excluded

- Public self-service tenant creation.
- Cross-Mandant sharing.
- Social login or external identity-provider federation.

