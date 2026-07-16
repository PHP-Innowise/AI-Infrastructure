---
name: api-designer
description: "Design Symfony HTTP APIs: routes, request DTOs/forms, validation, authorization, response DTOs/serializers, errors, pagination, and OpenAPI/API Platform contracts."
phase: planning
flow-next: writing-plans
flow-alternatives: [coder, architect]
---

# Symfony API Designer

Design stable HTTP contracts before implementation. Use API Platform only when the consuming project already uses it or the architecture decision explicitly selects it.

## Required Context

Inspect before designing:

- `composer.json`/lock versions, routes, controllers, API Platform resources, Serializer configuration, exception listeners, and OpenAPI tooling;
- authentication/firewall configuration, voters, access rules, tenant scoping, and rate limiters;
- existing request/response DTOs, validation conventions, error format, pagination, naming, and versioning;
- services, repositories, entities, indexes, transactions, Messenger workflows, and functional tests used by adjacent endpoints.

Do not invent a second API style when the project already has a coherent contract.

## Endpoint Matrix

For every operation define:

| Concern | Required decision |
| --- | --- |
| Route | HTTP method, path, route name, format, host/version constraint |
| Input | Content type, request DTO, mapping, validation, unknown-field behavior |
| Security | Authentication, coarse access rule, voter attribute/subject, tenant scope |
| Workflow | One service/use-case method and its transaction/side-effect boundary |
| Persistence | Repository queries, locking, indexes, pagination/count cost |
| Output | Response DTO/normalizer, media type, status, headers, cache behavior |
| Failure | Stable client-safe problem code/status and field violations |
| Verification | Functional, authorization, validation, contract, and query tests |

Controllers remain adapters: map validated input, authorize, call one service, and map the result. Avoid exposing Doctrine entities as public contracts or relying on lazy-loaded associations during serialization.

## Request Contract

- Prefer typed request DTOs with `#[MapRequestPayload]`, Symfony Forms where already established, API Platform input DTOs, or an explicit project mapper.
- Distinguish absent, null, empty, and malformed values. Reject or document unknown fields consistently.
- Put syntax and shape rules in Validator constraints; keep stateful business decisions in services and database constraints.
- Prevent clients from setting ownership, tenant IDs, roles, audit fields, internal status, prices, or workflow state unless the operation explicitly permits it.
- Define upload MIME/type, size, count, storage, antivirus/content checks, authorization, and failure behavior separately.

## Authentication And Authorization

- State whether the operation is public, session-authenticated, token-authenticated, or protected by another installed authenticator.
- Define object-level voter attributes and subjects. Collection endpoints require scoped repository/provider queries; an item voter does not prevent collection leakage.
- Specify `401`, `403`, and existence-hiding `404` behavior deliberately.
- Document rate limits, login throttling, webhook signatures, replay windows, and idempotency where applicable.

## Response And Error Contract

- Use response DTOs, dedicated normalizers, documented Serializer groups, or API Platform output DTOs.
- Define field names, types, nullability, date/time/timezone format, identifier representation, links, and backwards-compatibility expectations.
- Prefer RFC 9457-style problem details or the project's established stable error envelope.
- Separate malformed body, unsupported media type, validation failure, authentication, authorization, not found, conflict, rate limit, and infrastructure failure.
- Never expose exception messages, stack traces, SQL, internal class names, secrets, or authorization-sensitive resource details.

Example error shape:

```json
{
  "type": "https://example.test/problems/validation-failed",
  "title": "Validation failed",
  "status": 422,
  "code": "validation_failed",
  "violations": [{"propertyPath": "email", "message": "This value is not valid."}]
}
```

## Collections And Compatibility

- Whitelist filters and sorts, cap page size, require deterministic ordering, and align repository queries with indexes.
- Use offset pagination for bounded stable datasets; use cursor/keyset pagination for large or frequently changing collections.
- Define count semantics, empty results, invalid cursors, filter combinations, and maximum query complexity.
- Define versioning and deprecation through the project's existing strategy. Include sunset/deprecation headers when used.
- Require idempotency keys for retry-prone create/payment/webhook operations, including key scope, retention, request fingerprinting, and replay response.
- Treat bulk operations and long-running exports as separate bounded contracts; consider Messenger plus a status resource instead of holding a request open.

## OpenAPI And API Platform

Keep route attributes, DTO schemas, Serializer metadata, validation, security, and OpenAPI descriptions consistent. When API Platform is installed, define operations, input/output classes, providers/processors, filters, pagination, security expressions/voters, and error resources explicitly; hand detailed resource design to `api-platform-designer`.

## Test Plan

Cover at minimum:

- successful request and exact response contract;
- malformed payload, invalid fields, unknown fields, and unsupported content type;
- anonymous, denied, wrong-owner/tenant, and allowed authorization paths;
- not found, uniqueness/concurrency conflict, rate limit, and idempotent replay where relevant;
- collection bounds, stable ordering, filters, pagination, N+1/query-count risk, and sensitive-field absence;
- OpenAPI/schema drift when contract tooling supports it.

Use [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md) when deciding input/output DTOs, authorization placement, query scoping, and API Platform provider/processor boundaries.

## Deliverable

Produce the endpoint matrix, request/response schemas, service and repository calls, authorization matrix, error catalog, pagination/filter rules, OpenAPI changes, compatibility/deprecation notes, and test cases. Record approved long-lived contracts in `specs/` using the required skill-prefixed filename.
