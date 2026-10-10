# Epic 02: Authentication, MFA & API Security

## Outcome

Active users authenticate through a hardened password and MFA flow, can manage their credentials safely, and receive scoped, revocable API token families only after successful authentication.

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
- [ ] Let authenticated users change their password after shared, per-user-throttled current-password verification.
- [ ] Audit failed and throttled current-password checks with the user and operation, never the supplied password.
- [ ] Invalidate remembered-device trust after a password change.
- [ ] Apply the configured password policy at every password-setting boundary.

### E02-T03 — WebAuthn and TOTP MFA

- [ ] Support WebAuthn credentials and TOTP as local second factors.
- [ ] Bind enrollment and verification to the authenticated user and current challenge.
- [ ] Store TOTP secrets encrypted at rest.
- [ ] Apply rate limits to registration, enrollment, and verification attempts.
- [ ] Require MFA enrollment for roles covered by the deny-by-default policy, including authority access.
- [ ] Require current-password reauthentication before adding another factor or removing or disabling an active factor.

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

### E02-T06 — API authentication and refresh-token families

- [ ] Issue bearer access and rotating refresh credentials only after password and required MFA checks pass.
- [ ] Scope API authorization to the same roles, Mandant, and access grants as browser access.
- [ ] Rotate the refresh credential on every use without extending the original token-family expiry.
- [ ] Serialize refresh and revoke operations per family; treat reuse of any consumed refresh credential as compromise and revoke the family.
- [ ] Bind each access JWT to its owning user's active token family and give it a unique `jti`, without maintaining a separate per-JWT denylist.
- [ ] Allow only the owner or a non-impersonating platform administrator to revoke a token family, and audit revocation or reuse against the affected Mandant.
- [ ] Avoid mutating shared login state in tests or long-running processes.

## Acceptance criteria

- Inactive users and users missing mandatory MFA cannot obtain a session or bearer token.
- Password, MFA, and token endpoints remain rate-limited and non-enumerating.
- A remembered device expires and can be invalidated without disabling MFA itself.
- Invalid MFA encryption configuration stops normal application boot before encrypted data is used.
- Failed current-password checks for password and protected MFA changes are per-user rate-limited and auditable.
- At most one concurrent refresh succeeds; consumed-token reuse revokes the family and invalidates its access and refresh credentials.
- A user cannot revoke another user's token family unless acting as a non-impersonating platform administrator.
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
- `91d529a` — current-password reauthentication when adding another MFA factor.
- `ea420ac` — shared current-password throttling and audit logging.
- `911c399` — rotating refresh-token families, reuse detection, family-bound JWT revocation, and administrative family revocation.

## Dependencies

- Epic 01 supplies users, roles, Mandants, and access grants.
- Epic 08 supplies secure environment configuration and deployment checks.

## Excluded

- Passwordless-only login.
- Social login and enterprise SSO.
- SMS as a second factor.
