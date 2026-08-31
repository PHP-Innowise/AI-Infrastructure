# Invitation Registration Implementation Plan

## Goal

Allow trainers to invite players or parents to register through a WordPress API flow with validation, authorization, and test coverage.

## Proposed Design

- Route: `POST /vendor-invitations/v1/invitations` on `rest_api_init`.
- Controller: thin `InvitationController` using WordPress REST request/response contracts.
- Schema: typed arguments, bounded validation, stable response fields and error codes.
- Persistence: prefixed custom table through `$wpdb->prepare()` and a schema-version upgrade.
- Access control: object-aware capability in `permission_callback`; cookie nonce authenticates but does not authorize.
- Service: `CreateInvitation` owns token hashing, uniqueness and notification dispatch.

## Implementation Steps

1. Add an idempotent schema upgrade and unique token-hash index.
2. Add the repository and service using the plugin's established wiring.
3. Register the REST route with argument schema and permission callback.
4. Return a stable `WP_REST_Response`; never return token hashes or raw rows.
5. Add tests for success, validation, authorization, and duplicate/expired cases.
6. Run WordPress verification checks.

## Verification

```bash
composer validate --strict
composer test
vendor/bin/phpcs
composer analyse
```

## Risks

- Invitation tokens must be generated securely and never logged.
- Email/notification dispatch should be queued if it affects request latency.
- Authorization must be server-side; hiding UI actions is not authorization.
