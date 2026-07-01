# Example Pull Request Description

## Summary

- Added invitation-only registration endpoints for Laravel API clients.
- Added Form Request validation, policy authorization, Eloquent persistence, and API Resource responses.
- Added feature tests for successful registration, validation failure, authorization failure, and expired invitations.

## Test Plan

- [x] `composer validate`
- [x] `php artisan test --filter=InvitationRegistrationTest`
- [x] `vendor/bin/pint --test`
- [x] `vendor/bin/phpstan analyse`

## Risk Notes

- Migration adds an `invitations` table and should run before the feature is enabled.
- Invitation tokens must remain high entropy and must not be logged.
