---
name: security-reviewer
description: Audit Laravel changes for security risk against the OWASP Top 10. Use before merging security-sensitive changes (auth, input handling, SQL, file uploads, sessions, secrets) or on request. Triggers on "security review", "is this safe", "vulnerability", "OWASP", "audit security".
phase: execution
flow-next: verify
flow-alternatives: [coder, code-reviewer, debugger]
related: [code-reviewer, coder, dependency-manager, architect, file-storage, auth-scaffolding]
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

Identify the attack surface of the change: which inputs cross a trust boundary (HTTP params/body/headers/cookies, CLI args via Artisan commands, files, external API responses, queue job payloads), what they reach (query builder/Eloquent, filesystem, shell, Blade templates, deserialization), and who is allowed to do it (guest, authenticated user, specific role/ability).

## OWASP Top 10 Checklist (Laravel)

### A01 Broken Access Control
- Every state-changing/sensitive action authorized via a Policy or Gate (`$this->authorize()`, `can:` middleware), not just hidden UI?
- Object ownership checked (no IDOR: `/orders/{id}` reachable by other users) — does the Policy compare the resource owner to the authenticated user?
- Route model binding scoped correctly; no auth logic relying solely on client-controlled route params?
- Sanctum token abilities/scopes (`tokenCan('...')`) checked where a token should not have full account access?

### A02 Cryptographic Failures
- Passwords hashed via the `Hash` facade (bcrypt/argon2 per `config/hashing.php`), verified with `Hash::check()` (never md5/sha1/plaintext)?
- Secrets/tokens generated with `Str::random()`/`random_bytes()` (never `rand()`/`uniqid()`)?
- `APP_KEY` set and never committed; encrypted data uses the `Crypt` facade, not hand-rolled crypto?
- Session cookies secure per `config/session.php` (see A07).

### A03 Injection
- Queries built via Eloquent/query builder with bound parameters; any `DB::raw()`/`whereRaw()`/`selectRaw()` with interpolated (non-bound) user input?
- No `exec`/`shell_exec`/`system`/`proc_open` with untrusted input (or `escapeshellarg`/`escapeshellcmd` applied)?
- Blade output escaped by default (`{{ }}`); any `{!! !!}` rendering untrusted/user-supplied data (XSS)?
- No untrusted input in dynamic `View::make()`/`include` paths or dynamic class/function calls.

### A04 Insecure Design
- Rate limiting (`throttle:` middleware, custom `RateLimiter::for()`) on auth, password reset, and expensive endpoints?
- Sensitive workflows have server-side invariants (Policy checks, Form Request rules), not just UI guards or disabled buttons?
- Unauthenticated sensitive links (email verification, password reset, one-off downloads) use `URL::signedRoute()`/`hasValidSignature()` rather than guessable IDs?

### A05 Security Misconfiguration
- `APP_DEBUG=false` and `APP_ENV=production` in production; stack traces not leaked to users?
- Telescope/Debugbar/`horizon` dashboards gated behind auth or disabled in production (`TelescopeServiceProvider::gate()`)?
- CORS (`config/cors.php`) scoped to known origins, not `*` for authenticated APIs?
- Correct file/storage permissions; no world-writable secrets or `.env` committed.

### A06 Vulnerable & Outdated Components
- Run `composer audit`; triage advisories in dependencies.
- Unmaintained/abandoned packages flagged; Laravel/PHP versions within supported LTS windows.

### A07 Identification & Authentication Failures
- Session regenerated on login/privilege change (`$request->session()->regenerate()`, handled by Laravel's auth guards by default — verify it wasn't bypassed)?
- CSRF protection intact for state-changing web requests: `VerifyCsrfToken` middleware active, `@csrf` present in Blade forms?
- Cookie flags secure per `config/session.php` (`secure`, `http_only`, `same_site`)?
- No session fixation; logout invalidates the session (`Auth::logout()` + session invalidation).
- Sanctum SPA/token guards configured correctly (`stateful` domains, expiration).

### A08 Software & Data Integrity Failures
- No `unserialize()` of untrusted data (use JSON, or Laravel's queue/cache serialization which is not user-controlled)?
- Uploaded files validated by type/size (`'file' => 'mimes:...|max:...'`) and stored via `Storage::disk(...)->store()` with a non-executable, ideally non-public disk, never trusted by extension alone? (See the `file-storage` skill for implementing this correctly — MIME-sniffing, generated filenames, signed/temporary URLs.)
- Mass assignment guarded: model `$fillable` allow-lists sensitive attributes (`is_admin`, `role`) out, or they're excluded from request `$validated` before `create()`/`update()`.

### A09 Logging & Monitoring Failures
- Security-relevant events logged (auth failures, authorization denials) via `Log::channel(...)` without logging secrets/PII/tokens/passwords?
- Telescope data pruned/restricted in production so it doesn't become a sensitive-data sink.

### A10 Server-Side Request Forgery (SSRF)
- Outbound requests to user-supplied URLs (`Http::get($userUrl)`) validated/allowlisted; internal/metadata endpoints blocked?

## Tooling Support

Automated tools augment (never replace) manual review:

- `composer audit` for known-vulnerable dependencies; consider `roave/security-advisories` to block installs of packages with known CVEs.
- Static analysis: PHPStan with Larastan (`nunomaduro/larastan`) at a high level catches unsafe nullable access and type mismatches that often mask security bugs; Psalm `--taint-analysis` for deeper taint tracing of untrusted input into sinks (SQL, `echo`, `exec`) if configured.
- `php artisan about` to quickly confirm environment (`APP_ENV`, `APP_DEBUG`, drivers) matches production expectations.
- Grep for dangerous sinks as a fast first pass: `eval(`, `exec(`, `shell_exec(`, `system(`, `unserialize(`, `DB::raw(`, `whereRaw(`, `{!!`, `md5(`/`sha1(` for passwords.
- Verify secure session/cookie config in `config/session.php` (`secure`, `http_only`, `same_site`) and that `APP_DEBUG=false` in the deployed `.env` (never read the actual `.env`; check `config/app.php` defaults and deployment docs instead).

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
### [Critical] SQL injection in ReportController::search()
- Location: `app/Http/Controllers/ReportController.php:42`
- Issue: `$term` interpolated into `DB::raw()` instead of using a bound parameter.
- Exploit: `'; DROP TABLE users; --`
- Fix: use `whereRaw('name LIKE ?', ["%{$term}%"])` or the query builder's `where('name', 'like', "%{$term}%")`.

## Dependency Audit
- `composer audit`: [result / advisories]

## Summary
- Critical: N, High: N, Medium: N, Low: N
- Overall: [BLOCK / fix-then-merge / acceptable]
```

If nothing is found, state so and note the residual risk and what was not covered.

## Guardrails

- Never print or exfiltrate real secrets found; report the location and remediation only.
- Never read, print, or commit `.env` contents — reason about config via `config/*.php` defaults and deployment docs.
- Do not exploit against live systems; reason statically and with local tests.

## Final Output

Return findings by severity with fixes, `composer audit` result, an overall verdict, Context Summary, and next step (`/coder` to fix, then `/verify`).
