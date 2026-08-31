# Example Pull Request Description

## Summary

- Added a namespaced invitation REST route with argument and response schemas.
- Added a capability-aware permission callback, `$wpdb` persistence with
  prepared queries, and an idempotent schema-version upgrade.
- Added tests for successful registration, validation failure, authorization failure, and expired invitations.

## Test Plan

- [x] `composer validate --strict`
- [x] `composer test` (or `vendor/bin/phpunit --filter=InvitationRegistrationTest`)
- [x] `vendor/bin/phpcs` (WordPress Coding Standards)
- [x] `composer analyse` (phpstan / psalm)

## Risk Notes

- Migration adds a prefixed invitations table through a versioned, retryable upgrade.
- Invitation tokens must remain high entropy and must not be logged.
