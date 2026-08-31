---
name: rest-api
description: Design and implement WordPress REST API routes with namespace/versioning, argument and response schemas, permission callbacks, controllers, links, pagination, errors, caching, and backward compatibility.
phase: execution
flow-next: security-reviewer
flow-alternatives: [test-generator, documentation-generator, verify]
related: [api-designer, coder, content-modeling]
---

# WordPress REST API

## Procedure

1. Read existing route registration, controllers, schemas, authentication,
   client usage, tests, and API/version compatibility requirements.
2. Register routes on `rest_api_init` under a stable vendor namespace and
   explicit version. Reuse `WP_REST_Controller` conventions when they improve
   collection/item consistency.
3. Define methods, path parameters, argument schemas, defaults, sanitizers and
   validators. Validation must enforce domain constraints; sanitization alone
   does not make input valid.
4. Every route has an intentional `permission_callback`. Check capabilities
   and object scope, not role labels. Nonces authenticate cookie-based REST
   requests but do not grant authorization.
5. Return `WP_REST_Response`, registered REST fields, or `WP_Error` with stable
   machine-readable codes, correct status, and safe details. Do not expose raw
   exceptions, SQL, paths, secrets, or private metadata.
6. Keep response schemas stable and context-aware (`view`, `edit`, `embed`).
   Bound `per_page`, filter and ordering inputs; avoid N+1 meta/term queries and
   return collection headers/links consistently.
7. Make writes atomic enough for the domain and safe to retry where clients may
   repeat requests. State caching and invalidation explicitly.
8. Test unauthenticated, unauthorized, invalid, missing, conflict, success,
   pagination, field visibility, and backward-compatibility paths through the
   REST server rather than calling callbacks only.

## Output

Return route table, schemas, permission model, response/error contracts,
compatibility notes, test evidence, Context Summary, and next step.
