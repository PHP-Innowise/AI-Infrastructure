---
name: api-platform-designer
description: Design API Platform resources, providers, processors, DTOs, security, serialization, filters, pagination, and OpenAPI behavior.
phase: planning
flow-next: coder
flow-alternatives: [api-designer, security-reviewer]
---

# Symfony API Platform Designer

Use only when the project uses API Platform.

Design:

- Resource metadata and operations.
- Input/output DTOs.
- Providers and processors that delegate to repositories/services.
- Security expressions and voters.
- Serializer groups and sensitive-field exposure.
- Filters, pagination, validation, errors, and OpenAPI docs.

## Workflow

1. Confirm the installed API Platform and Symfony versions from Composer metadata.
2. Inventory existing resources, operations, providers, processors, serialization groups, filters, and exception mappings.
3. Decide whether the public contract uses entities or dedicated input/output DTOs. Prefer DTOs when entity shape, write rules, or versioning differ from the API contract.
4. Define each operation's URI, method, input, output, validation, security, provider, processor, status code, cache behavior, and OpenAPI description.
5. Keep providers query-focused and processors orchestration-focused: providers call repositories/query services; processors authorize where required and delegate writes to application services.
6. Specify pagination limits, stable sorting, filter allowlists, N+1 prevention, and collection authorization.
7. Map validation, domain, authorization, not-found, conflict, and infrastructure failures to stable problem responses.
8. Plan functional contract tests and focused provider/processor tests.

Compare resource/DTO/provider/processor separation with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). Do not expose an entity merely to avoid mapping, and do not introduce DTOs that duplicate an already-safe stable public contract without benefit.

## Security And Performance

- Never expose sensitive entity fields through broad normalization groups.
- Separate read and write groups; prevent client-controlled ownership, roles, tenant IDs, audit fields, and workflow state.
- Apply object security to item operations and explicit collection scoping to collection providers.
- Bound pagination and expensive filters; align repository queries with indexes and eager-loading needs.
- Treat GraphQL, Mercure, file fields, and custom operations as separate threat and performance surfaces when enabled.

## Output

Produce an operation matrix, DTO/resource design, provider/processor responsibilities, security rules, error contract, query implications, test plan, and OpenAPI changes. Record durable API decisions in `specs/`.
