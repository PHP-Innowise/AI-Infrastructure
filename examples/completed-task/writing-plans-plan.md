# Invitation Registration Implementation Plan

## Goal

Allow trainers to invite players or parents to register through a native PHP API flow with validation, authorization, and test coverage.

## Proposed Design

- Route: `POST /invitations` registered in the router / front controller.
- Handler: `src/Http/Controller/InvitationController.php`.
- Request DTO: `src/Http/Request/CreateInvitationRequest.php` (validation + normalization at the boundary).
- Serializer: `src/Http/Response/InvitationSerializer.php`.
- Domain: `src/Domain/Invitation/Invitation.php` plus an `InvitationRepository` interface.
- Persistence: `src/Infrastructure/Persistence/PdoInvitationRepository.php`.
- Access control: explicit check for `invitation.create` before acting.
- Migration: `migrations/*_create_invitations_table.php` (or reviewed SQL).
- Use case: `src/Application/AcceptInvitation.php` when creation includes token generation and notification dispatch.

## Implementation Steps

1. Add the migration and schema (UNIQUE on token/email as needed).
2. Add the domain entity + repository interface, then the PDO implementation.
3. Add `CreateInvitationRequest` with validation; authorize at the boundary.
4. Add the handler and route; return output through the serializer.
5. Add tests for success, validation, authorization, and duplicate/expired cases.
6. Run native PHP verification checks.

## Verification

```bash
composer validate --strict
composer test
composer lint
composer analyse
```

## Risks

- Invitation tokens must be generated securely and never logged.
- Email/notification dispatch should be queued if it affects request latency.
- Authorization must be server-side; hiding UI actions is not authorization.
