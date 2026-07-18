---
name: security-voter-designer
description: Design Symfony voters, access-control rules, firewalls, role hierarchy, and authorization tests.
phase: planning
flow-next: coder
flow-alternatives: [security-reviewer, test-generator]
---

# Symfony Security Voter Designer

Design authorization:

- Subject being protected: entity, DTO, route, collection, or action.
- Attribute names and role hierarchy.
- Voter logic and abstain/deny behavior.
- Controller/service call site.
- `access_control`, firewall, authenticator, and route constraints when relevant.
- Tests for allowed, denied, anonymous, and ownership/tenant cases.

Never rely on hidden UI as authorization.

## Design Workflow

1. Identify the protected capability, subject type, caller, tenant/ownership rules, and whether access can be decided before loading an object.
2. Use stable attribute constants or enums. Define voter support narrowly so unrelated subjects abstain instead of being accidentally granted or denied.
3. Specify authentication requirements, role shortcuts, ownership/membership checks, state restrictions, and tenant boundaries.
4. Keep data loading out of the voter when possible. Controllers/providers should load the authorized subject through scoped queries where collection exposure or IDOR risk exists.
5. Decide whether denial is `403`, authentication is `401`/redirect, and sensitive resource existence should be hidden as `404`.
6. Align route rules, firewall configuration, access rules, controller attributes, API Platform expressions, and voter calls. Avoid contradictory layers.

Compare server-side voter and collection-scoping responsibilities with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). Keep voters cohesive and deterministic; do not inject the request, entity manager, or broad service locator.

## Tests

Cover anonymous, authenticated-but-denied, allowed, wrong owner/tenant, privileged role, unsupported attribute, unsupported subject, and relevant object states. Test collection scoping separately; a voter on item operations does not secure a collection query.
