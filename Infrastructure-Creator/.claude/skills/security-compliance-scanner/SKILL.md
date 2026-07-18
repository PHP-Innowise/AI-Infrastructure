---
name: security-compliance-scanner
description: Detect a PHP target's authentication pattern, secrets-handling approach, existing security tooling, and textual compliance mentions from real evidence - without reading or printing secret values, and without asserting actual compliance. Use as Phase 1 discovery input to profile-synthesizer. Triggers on "scan security", "detect auth pattern", "how does this PHP app handle secrets", "security-compliance-scanner".
phase: discovery
flow-next: profile-synthesizer
flow-alternatives: [profile-synthesizer]
related: [stack-scanner, architecture-scanner, integration-scanner, infra-ops-scanner, conventions-scanner, infra-scan]
---

# Security Compliance Scanner

## Overview

Read-only reconnaissance of a PHP target's security posture: the authentication pattern in use, how secrets are handled (approach only, never values), what security tooling already runs, and any textual compliance mentions. The output gives `profile-synthesizer` an evidence-backed security picture without ever reading a secret value or claiming the project is actually compliant with any standard.

The target project path is a **required** argument (e.g. "scan security of ../my-php-app"). Never assume the current working directory is the target. Operate strictly read-only within that path; never write to it, and never read or print the contents of `.env`/secrets - detect only that they exist and how they are referenced.

## Generated File Naming Convention (MANDATORY)

Write exactly one findings file into the current run's task directory: `tasks/TASK-{N}/security-compliance-scanner-findings.md`. Never write into the target.

## Process

1. **Detect the auth pattern** from packages plus middleware/guards/config, citing evidence:
   - Session: framework session config, session-based login controllers/guards.
   - Token/API: `laravel/sanctum`, `laravel/passport`, `lexik/jwt-authentication-bundle`, `tymon/jwt-auth`, `firebase/php-jwt`, API middleware.
   - OAuth: `league/oauth2-*`, `socialite`, `knpuniversity/oauth2-client-bundle`.
   - SAML: `onelogin/php-saml`, `simplesamlphp/*`, SAML config files.
2. **Detect the secrets-handling approach WITHOUT reading values.** Note `.env`/`.env.example` presence and *which keys* are referenced via `env()`/`getenv()`/`$_ENV` (key names only), `config/secrets/` (Symfony vault), and vault SDKs (`hashicorp/vault-*`, AWS/GCP/Azure secret-manager SDKs). Never open or echo secret values.
3. **Detect existing security tooling.** `composer audit` in CI, `roave/security-advisories` in `require-dev`, `enlightn/enlightn`, `enlightn/security-checker`, Psalm taint analysis (`--taint-analysis`/config), and PHPStan security rules/extensions. Cite the config or CI line.
4. **Detect textual compliance mentions ONLY.** Grep docs/config/README for GDPR/PCI(-DSS)/HIPAA/SOC 2/ISO 27001 strings and report the mention with location. Do **not** assert or evaluate actual compliance - report only that the text appears.
5. **Note relevant hardening signals** if trivially visible (CSRF config, security headers middleware, encryption config presence) as `inferred` unless directly configured.
6. **Mark confidence** per finding: `confirmed` (direct evidence), `inferred` (indirect signal), or `unknown`. Never present a guess as fact.

## Output Template

```markdown
# Security Compliance Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Authentication Pattern
- [session/token/OAuth/SAML + package] (confirmed - evidence path:L#)

## Secrets Handling (approach only - no values read)
- [.env keys referenced / vault SDK / config/secrets] (confirmed - path:L#)

## Security Tooling
- [composer audit / roave/security-advisories / enlightn / psalm taint / phpstan rules or "N/A - not configured"] (confirmed - path:L#)

## Compliance Mentions (textual only - not an assertion of compliance)
- [GDPR/PCI/HIPAA/... : location] for each; "None detected" if absent

## Hardening Signals
- [CSRF / headers / encryption config] (confirmed/inferred - path)

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Guardrails

- MUST cite a real file path (and line where practical) for every finding.
- MUST operate read-only on the target; MUST NEVER read or print secret VALUES - detect only the handling approach and key names.
- MUST report compliance strings as textual mentions only; MUST NOT assert the project is compliant with any standard.
- MUST report absent tooling as `N/A - not configured` rather than assuming a default.
- MUST NOT deep-dive framework identity, integrations, infra, or conventions - those belong to their own scanners.

## Final Output

Return the findings file path, the detected auth pattern, the secrets-handling approach (no values), the security tooling inventory, any compliance mentions (flagged as textual only), and a one-line confidence summary. Suggest `profile-synthesizer` as the next step.
