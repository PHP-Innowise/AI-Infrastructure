---
name: security-reviewer
description: Audit native PHP changes for security risk against the OWASP Top 10. Use before merging security-sensitive changes (auth, input handling, SQL, file uploads, sessions, secrets) or on request. Triggers on "security review", "is this safe", "vulnerability", "OWASP", "audit security".
phase: execution
flow-next: verify
flow-alternatives: [coder, code-reviewer, debugger]
related: [code-reviewer, coder, dependency-manager, architect]
---

# Security Reviewer

## Overview

Find security defects before attackers do. Focus on exploitable risk in the change under review and the code paths it touches. Report findings by severity with a concrete fix, not vague warnings.

This complements `/code-reviewer` (which flags obvious risks) with a dedicated, deeper pass.

## Generated File Naming Convention (MANDATORY)

Any file created by this skill MUST be prefixed with `security-reviewer-`:
- Correct: `security-reviewer-findings.md`
- Incorrect: `SECURITY.md`, `audit.md`

## Scope First

Identify the attack surface of the change: which inputs cross a trust boundary (HTTP params/body/headers/cookies, CLI args, files, external API responses, queue payloads), what they reach (SQL, filesystem, shell, templates, deserialization), and who is allowed to do it.

## OWASP Top 10 Checklist (native PHP)

### A01 Broken Access Control
- Every state-changing/sensitive action authorized server-side (not just hidden UI)?
- Object ownership checked (no IDOR: `/orders/{id}` reachable by other users)?
- No auth logic relying solely on client-controlled values?

### A02 Cryptographic Failures
- Passwords hashed with `password_hash()` (bcrypt/argon2), verified with `password_verify()` (never md5/sha1/plaintext)?
- Secrets/tokens generated with `random_bytes()`/`random_int()` (never `rand()`/`uniqid()`)?
- TLS assumed for sensitive transport; secure cookie flags set?

### A03 Injection
- SQL uses PDO prepared statements with bound params (no string concatenation, no interpolated input)?
- No `exec`/`shell_exec`/`system`/`proc_open` with untrusted input (or `escapeshellarg`/`escapeshellcmd` applied)?
- Output escaped in templates with `htmlspecialchars(..., ENT_QUOTES)` (XSS); context-correct escaping for attributes/JS/URLs?
- No untrusted input in `include`/`require` paths (LFI/RFI) or dynamic class/function calls?

### A04 Insecure Design
- Rate limiting/lockout on auth and expensive endpoints?
- Sensitive workflows have server-side invariants, not just UI guards?

### A05 Security Misconfiguration
- `display_errors` off in production; stack traces not leaked to users?
- Debug endpoints/verbose logging disabled in production?
- Correct file permissions; no world-writable secrets.

### A06 Vulnerable & Outdated Components
- Run `composer audit`; triage advisories in dependencies.
- Unmaintained/abandoned packages flagged.

### A07 Identification & Authentication Failures
- `session_regenerate_id(true)` on privilege change/login; secure/httponly/samesite cookies?
- CSRF protection on state-changing web requests (token checked server-side)?
- No session fixation; logout invalidates the session.

### A08 Software & Data Integrity Failures
- No `unserialize()` of untrusted data (use JSON or `allowed_classes: false`)?
- Uploaded files validated by type/size, stored outside the web root or with execution disabled, and never trusted by extension alone?

### A09 Logging & Monitoring Failures
- Security-relevant events logged (authn/authz failures) without logging secrets/PII/tokens?

### A10 Server-Side Request Forgery (SSRF)
- Outbound requests to user-supplied URLs validated/allowlisted; internal metadata endpoints blocked?

## Tooling Support

Automated tools augment (never replace) manual review:

- `composer audit` for known-vulnerable dependencies; consider `roave/security-advisories` to block installs.
- Static taint analysis: Psalm `--taint-analysis` or PHPStan with a security/taint extension to trace untrusted input into sinks (SQL, `echo`, `exec`).
- Grep for dangerous sinks as a fast first pass: `eval(`, `exec(`, `shell_exec(`, `system(`, `unserialize(`, `extract(`, `assert(`, `md5(`/`sha1(` for passwords, string-interpolated SQL.
- Verify secure `php.ini`/cookie defaults (`session.cookie_httponly`, `session.cookie_secure`, `session.cookie_samesite`, `display_errors=0` in prod).

## Severity Ratings

| Severity | Meaning |
| --- | --- |
| Critical | Remotely exploitable, no auth (e.g. SQLi, RCE) |
| High | Exploitable with low privilege or leaks sensitive data |
| Medium | Requires unlikely conditions or limited impact |
| Low | Hardening/defense-in-depth |
| Info | Good-practice note |

## Output Template

```markdown
# Security Review: [Change]

## Scope
[Attack surface reviewed]

## Findings
### [Critical] SQL injection in UserRepository::search()
- Location: `src/Infrastructure/Persistence/UserRepository.php:42`
- Issue: `$term` concatenated into the query string.
- Exploit: `'; DROP TABLE users; --`
- Fix: use a prepared statement with a bound `:term` parameter.

## Dependency Audit
- `composer audit`: [result / advisories]

## Summary
- Critical: N, High: N, Medium: N, Low: N
- Overall: [BLOCK / fix-then-merge / acceptable]
```

If nothing is found, state so and note the residual risk and what was not covered.

## Guardrails

- Never print or exfiltrate real secrets found; report the location and remediation only.
- Do not exploit against live systems; reason statically and with local tests.

## Final Output

Return findings by severity with fixes, `composer audit` result, an overall verdict, Context Summary, and next step (`/coder` to fix, then `/verify`).
