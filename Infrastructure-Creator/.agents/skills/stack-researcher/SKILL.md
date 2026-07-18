---
name: stack-researcher
description: Run grounded, source-cited web research against the OFFICIAL documentation for each significant detected dependency/integration plus the primary PHP framework/version, so generated skills reflect accurate, current, version-appropriate PHP practice. Use in the research phase after discovery scans. Triggers on "research the stack", "verify current best practice for these packages", "check the official docs for detected dependencies", "stack-researcher".
phase: research
flow-next: clarifying-interview
flow-alternatives: []
related: [stack-scanner, integration-scanner, infra-scan, clarifying-interview]
---

# Stack Researcher

## Overview

Grounded, source-cited research against **official documentation** for the target's significant dependencies and its primary PHP framework/version. Unlike the Phase 1 scanners, this skill does not inspect the target for new evidence - it consumes prior findings and enriches them with current, version-appropriate best practice so downstream generated skills are accurate rather than guessed from stale training data.

The target project path is a **required** argument (e.g. "research the stack of ../my-php-app"). Inputs are `tasks/TASK-{N}/integration-scanner-findings.md` (significant dependencies/integrations) and `tasks/TASK-{N}/stack-scanner-findings.md` (primary framework + version). "Significant" = direct/production Composer dependencies that map to an integration category; **skip** dev-only tooling and tiny utilities. Operate read-only on the target; never read its `.env`/secrets.

## Generated File Naming Convention (MANDATORY)

This is a researcher, not a scanner, and has no special `-findings` naming exception beyond its own file. Write exactly one file into the current run's task directory: `tasks/TASK-{N}/stack-researcher-findings.md`. Never write into the target.

## Process

1. **Load prior findings.** Read `stack-scanner-findings.md` for the primary framework + resolved version, and `integration-scanner-findings.md` for detected integrations/dependencies. If either is missing, note the gap and proceed with what is available.
2. **Select significant dependencies.** Keep direct/production Composer packages that map to an integration category (database, cache, queue, HTTP client, auth, payments, mail, search, etc.). Skip `require-dev` tooling and trivial utility libraries. Record the resolved version for each (from `composer.lock` if cited in prior findings).
3. **Research each against official docs.** For the primary framework/version and each selected dependency, consult the OFFICIAL documentation for the matching major/minor version. Capture 2-4 current best-practice notes that are version-appropriate (deprecations, recommended APIs, config, security defaults).
4. **Cite one official source URL per item.** Prefer the canonical docs domain for that package/framework and the version-specific page where available.
5. **Handle offline / unreachable docs.** If web research is unavailable, fall back to internal-only findings (prior scan data + local docs) and **explicitly FLAG the gap** per affected item, marking it `unknown - offline` so downstream skills know it was not verified.
6. **Mark confidence** per note: `confirmed` (official docs cited), `inferred` (indirect/community source), or `unknown` (unverified/offline).

## Output Template

```markdown
# Stack Researcher Findings: [target_name]

**Target:** [path]  **Researched:** [date]  **Mode:** [online / offline-fallback]

## Primary Framework
- [Framework] [resolved version] (confirmed)
  - [2-4 version-appropriate best-practice notes]
  - Source: [official docs URL]

## Significant Dependencies
### [package/version]
- Resolved version: [x.y.z]
- Notes: [2-4 current best-practice notes]
- Source: [official docs URL]

(repeat per significant dependency)

## Gaps / Offline Flags
- [package: unknown - offline / docs unreachable] for each affected item; "None" if fully researched

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Guardrails

- MUST base each best-practice note on the OFFICIAL documentation and cite one source URL per item.
- MUST match research to the resolved framework/dependency VERSION, not the latest generically.
- MUST include only significant direct/production dependencies; MUST skip dev-only tooling and tiny utilities.
- MUST, when offline, fall back to internal-only findings and EXPLICITLY flag each unverified item.
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.

## Final Output

Return the findings file path, the researched framework/version, the count of significant dependencies covered, any offline/gap flags, and a one-line confidence summary. Suggest `clarifying-interview` as the next step.
