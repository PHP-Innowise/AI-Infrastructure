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

The target project path is a **required** argument; never assume the current working directory is the target. Operate strictly read-only within it.

**Secrets rule** (identical in `integration-scanner`, full text in the contract): never open, read, print, or fingerprint `.env`/`.env.*` or any credential store, and never record a value. Environment-variable *names* may be cited when they come from a non-secret committed source - `config/**`, container/CI config, deploy scripts, committed `.env.example`/`*.template` files, and `env()`/`getenv()`/`$_ENV` call sites. Existence of a secret file may be recorded; its contents may not.

## Outputs (MANDATORY)

Per run: exactly one report `tasks/TASK-{NNN}/security-compliance-scanner-findings.md` and exactly one evidence ledger `tasks/TASK-{NNN}/security-compliance-scanner-evidence.json`, both shaped by `stack-scanner/references/scan-evidence-contract.md` - read it first. Never write into the target.

## Process

1. **Detect the auth pattern** from packages plus middleware/guards/config, citing evidence:
   - Session: framework session config, session-based login controllers/guards.
   - Token/API: `laravel/sanctum`, `laravel/passport`, `lexik/jwt-authentication-bundle`, `tymon/jwt-auth`, `firebase/php-jwt`, API middleware.
   - OAuth: `league/oauth2-*`, `socialite`, `knpuniversity/oauth2-client-bundle`.
   - SAML: `onelogin/php-saml`, `simplesamlphp/*`, SAML config files.
2. **Detect the secrets-handling approach without opening a secret file.** Record that `.env`/`.env.*` exist (existence only), then list the key *names* obtainable from the non-secret committed sources named in the secrets rule above - `env()`/`getenv()`/`$_ENV`/`%env()%` call sites, `config/**`, container/CI config, deploy scripts, and committed `.env.example`/`*.template` files. Also record `config/secrets/` (Symfony vault) and vault SDKs (`hashicorp/vault-*`, AWS/GCP/Azure secret managers). A key knowable only from a real `.env` stays `unknown`; never open or echo a value.
3. **Detect existing security tooling.** `composer audit` in CI, `roave/security-advisories` in `require-dev`, `enlightn/enlightn`, `enlightn/security-checker`, Psalm taint analysis (`--taint-analysis`/config), and PHPStan security rules/extensions. Cite the config or CI line.
4. **Detect textual compliance mentions ONLY.** Grep docs/config/README for GDPR/PCI(-DSS)/HIPAA/SOC 2/ISO 27001 strings and report the mention with location. Do **not** assert or evaluate actual compliance - report only that the text appears.
5. **Capture security test topology and invariants.** Locate authentication, authorization, tenant/object-ownership, validation, audit-integrity, and redaction tests; record suite/root, focused repository command, fixture/fake prerequisites, denied-path assertion, and bounded anchor. Map each confirmed high-priority security invariant ID from domain findings to a concrete test/assertion owner, while preserving incomplete coverage as a gap.
6. **Note relevant hardening signals** if trivially visible (CSRF config, security headers middleware, encryption config presence) as `inferred` unless directly configured. External security/compliance standards remain review requirements, not confirmed target behavior, unless target policy explicitly adopts them.
7. **Map material adjacency.** Record when local authorization, provider identity mechanics, domain permission outcomes, audit behavior, and general code review have different primary owners; include positive, negative, ambiguous, and cross-domain routing cases.
8. **Mark confidence** per finding: `confirmed` (direct evidence), `inferred` (indirect signal), or `unknown`. Never present a guess as fact.

## Report Structure

Follow the `security-compliance-scanner` report template in appendix A of `stack-scanner/references/scan-evidence-contract.md`. Every factual line carries its confidence tag and its evidence id.

## Guardrails

- MUST cite a real file path (and line where practical) for every finding, and MUST emit both artifacts with contract-shaped evidence records (target-relative path, `sha256:` fingerprint, supported claims).
- MUST operate read-only on the target and follow the shared secrets rule: key names from non-secret committed sources only; never a value, and never `.env` itself.
- MUST report compliance strings as textual mentions only; MUST NOT assert the project is compliant with any standard.
- MUST report absent tooling as `N/A - not configured` rather than assuming a default.
- MUST NOT elevate a hardening signal, package capability, or external standard into confirmed target policy without a bounded target-policy anchor.
- MUST preserve missing denied-path coverage and unowned high-priority invariants as synthesis gaps.
- MUST NOT deep-dive framework identity, integrations, infra, or conventions - those belong to their own scanners.

## Final Output

Return both artifact paths (report and evidence ledger), the detected auth pattern, the secrets-handling approach (no values), the security tooling inventory, any compliance mentions (flagged as textual only), and a one-line confidence summary. Suggest `profile-synthesizer` as the next step.
