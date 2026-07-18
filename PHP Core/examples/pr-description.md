# Example Pull Request Description

## Summary

- Added invitation-only registration endpoints for native PHP API clients.
- Added input-DTO validation, explicit authorization, PDO persistence, and JSON serializer responses.
- Added tests for successful registration, validation failure, authorization failure, and expired invitations.

## Test Plan

- [x] `composer validate --strict`
- [x] `composer test` (or `vendor/bin/phpunit --filter=InvitationRegistrationTest`)
- [x] `composer lint` (php-cs-fixer / phpcs)
- [x] `composer analyse` (phpstan / psalm)

## Risk Notes

- Migration adds an `invitations` table and should run before the feature is enabled.
- Invitation tokens must remain high entropy and must not be logged.
