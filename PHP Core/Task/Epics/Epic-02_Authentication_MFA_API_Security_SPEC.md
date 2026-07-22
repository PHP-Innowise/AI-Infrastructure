# Epic 02: Authentication, MFA & API Security

## Outcome

Active users authenticate through a hardened password and MFA flow, can manage their credentials safely, and receive scoped API tokens only after successful authentication.

**Scope status:** Existing baseline from Project history.

## Capability tasks

### E02-T01 — Password login and session safety

- [ ] Authenticate by email and password without revealing whether an account exists.
- [ ] Deny login for inactive users.
- [ ] Support Remember Me with secure, HTTP-only, same-site cookies.
- [ ] Rotate the session after successful authentication.
- [ ] Throttle repeated login attempts by account and request origin.

### E02-T02 — Password lifecycle

- [ ] Provide password reset through a time-limited, single-use flow.
- [ ] Let authenticated users change their password after current-password verification.
- [ ] Revoke or rotate affected sessions and long-lived credentials after a password change.
- [ ] Apply the configured password policy at every password-setting boundary.

### E02-T03 — WebAuthn and TOTP MFA

- [ ] Support WebAuthn credentials and TOTP as local second factors.
- [ ] Bind enrollment and verification to the authenticated user and current challenge.
- [ ] Store TOTP secrets encrypted at rest.
- [ ] Apply rate limits to registration, enrollment, and verification attempts.
- [ ] Require MFA enrollment for roles covered by the deny-by-default policy, including authority access.

### E02-T04 — MFA device trust and recovery

- [ ] Allow a user to remember a successfully verified device.
- [ ] Sign, expire, rotate, and revoke remembered-device tokens.
- [ ] Invalidate trust when security-sensitive account state changes.
- [ ] Keep a recovery path that does not bypass identity verification or tenant authorization.

### E02-T05 — MFA encryption-key operations

- [ ] Validate the primary encryption key during application boot.
- [ ] Fail closed when the configured key is malformed.
- [ ] Support key identifiers and previous keys during controlled rotation.
- [ ] Provide a command to validate, generate, and re-encrypt stored MFA secrets.
- [ ] Never print keys or decrypted secrets to logs or command output.

### E02-T06 — Stateless API authentication

- [ ] Issue bearer access and refresh credentials only after password and required MFA checks pass.
- [ ] Scope API authorization to the same roles, Mandant, and access grants as browser access.
- [ ] Rotate refresh credentials and reject replayed, expired, or revoked tokens.
- [ ] Avoid mutating shared login state in tests or long-running processes.

## Acceptance criteria

- Inactive users and users missing mandatory MFA cannot obtain a session or bearer token.
- Password, MFA, and token endpoints remain rate-limited and non-enumerating.
- A remembered device expires and can be invalidated without disabling MFA itself.
- Invalid MFA encryption configuration stops normal application boot before encrypted data is used.
- API tokens never bypass Mandant or object-level access rules.

## Commit evidence

- `5e5bcd0` — password reset support.
- `69fecc9` — active-user login gate.
- `ee21d67`, `5f88213` — secure password login and Remember Me review fixes.
- `0a9ddf5` — authenticated password change.
- `b205589`, `0997426`, `0002cbd` — WebAuthn/TOTP MFA and rate-limit hardening.
- `6678978` — bearer token issuance after MFA.
- `98a7dd8` — remembered MFA device.
- `f71c128` — login throttling.
- `daeb837` — MFA key validation and rotation.
- `4080e38` — MFA enrollment gate and authority-role policy.
- `7a7d794` — login-test isolation, encrypted mapping, cookie flags, and MFA cleanup.

## Dependencies

- Epic 01 supplies users, roles, Mandants, and access grants.
- Epic 08 supplies secure environment configuration and deployment checks.

## Excluded

- Passwordless-only login.
- Social login and enterprise SSO.
- SMS as a second factor.

