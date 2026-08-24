# Example Context Summary

## Context Summary

Implemented invitation-only registration in a WordPress plugin with a
versioned REST route, argument/response schemas, capability-protected
permission callback, prefixed invitation table and idempotent schema upgrade.
The endpoint validates input, stores only a token hash, returns stable safe
fields, and dispatches notification work through the existing background path.

## Verification

- `composer validate --strict` - PASS
- `composer test` (`vendor/bin/phpunit --filter=InvitationRegistrationTest`) - PASS
- `vendor/bin/phpcs` (WordPress Coding Standards) - PASS
- `composer analyse` - PASS
- REST unauthorized/invalid/success integration cases - PASS

## Next Steps

**Next by flow:** `/code-reviewer` - Review the implementation for correctness, authorization, validation, and persistence risks.

**Alternatives:**
- `/test-generator` - Add missing edge-case coverage.
- `/verify` - Run the full Definition of Done.
