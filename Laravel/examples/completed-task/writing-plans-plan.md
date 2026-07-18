# Invitation Registration Implementation Plan

## Goal

Allow trainers to invite players or parents to register through a Laravel API flow with validation, authorization, and test coverage.

## Proposed Laravel Design

- Route: `POST /api/invitations` in `routes/api.php`.
- Controller: `app/Http/Controllers/Api/InvitationController.php`.
- Request: `app/Http/Requests/StoreInvitationRequest.php`.
- Resource: `app/Http/Resources/InvitationResource.php`.
- Model: `app/Models/Invitation.php`.
- Policy: `app/Policies/InvitationPolicy.php`.
- Migration: `database/migrations/*_create_invitations_table.php`.
- Action: `app/Actions/CreateInvitation.php` if creation includes token generation and notification dispatch.

## Implementation Steps

1. Add the migration, model, factory, and policy.
2. Add `StoreInvitationRequest` with validation and authorization.
3. Add controller and API route.
4. Add `InvitationResource`.
5. Add feature tests for success, validation, authorization, and duplicate/expired cases.
6. Run Laravel verification checks.

## Verification

```bash
composer validate --strict
php artisan test
vendor/bin/pint --test
vendor/bin/phpstan analyse
```

## Risks

- Invitation tokens must be generated securely.
- Email dispatch should be queued if it affects request latency.
- Policy checks must be server-side; hiding UI actions is not authorization.
