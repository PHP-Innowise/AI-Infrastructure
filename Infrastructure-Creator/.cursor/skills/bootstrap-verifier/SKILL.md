---
name: bootstrap-verifier
description: Run the final QA gate for a freshly generated accelerator before infra-generate is allowed to report success - validates frontmatter across every generated skill/agent/command, checks every cross-reference resolves to a real generated skill, checks every generated hook's syntax and executable bit, runs the seeded memory-bank/scripts/validate.py, and scans for leftover template placeholders. Takes a required target-project-path argument. Use as the last step of infra-generate, after skill-flow-composer. Triggers on "verify the generated accelerator", "bootstrap-verifier", "run the QA gate", "check what infra-generate produced".
phase: verification
flow-next: null
flow-alternatives: []
related: [infra-generate, skill-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer]
---

# Bootstrap Verifier

## Overview

`bootstrap-verifier` is the last step of `infra-generate`. It mechanically checks that the generated accelerator is internally consistent and immediately usable, for the selected edition(s) only. A failed run means generation is not done - it must be fixed and re-run before success is reported.

It uses the bundled `scripts/validate_generated.py` (dependency-free) plus targeted manual checks.

## Generated File Naming Convention (MANDATORY)

Writes a report to `tasks/TASK-{N}/bootstrap-verifier-report.md`. Does not write into the target.

## Process

1. **Determine the selected editions** from the profile (section 1) and the generate report.
2. **Run the validator:** `python3 scripts/validate_generated.py --target <target> --editions <selected>`. It checks:
   - Frontmatter validity across every generated `SKILL.md`, agent, and command.
   - Every `flow-next`/`flow-alternatives`/`related`/`invokes`/`spawns` reference resolves to a skill/agent that exists in that edition.
   - Every generated hook passes `bash -n` and carries the executable bit.
   - The seeded `memory-bank/` passes its own `scripts/validate.py`.
   - No template placeholders (`{skill-name}`, `TODO`, literal `YYYY-MM-DD`, `[target_name]`, `TASK-{N}`, etc.) remain.
3. **Assert edition scoping:** no unselected edition folder exists; every selected edition is present.
4. **Classify failures:**
   - Auto-fixable (e.g. missing executable bit) - fix and re-run the validator.
   - Not safely auto-fixable (e.g. a dangling cross-reference implying a forge under-produced) - escalate to the user; do not paper over it.
5. **Report** the result in `bootstrap-verifier-report.md`.

## Output Template

```markdown
# Bootstrap Verifier Report: [target_name]

**Editions checked:** [selected]
**Result:** [PASS / FAIL]

## Checks
- Frontmatter: [pass/fail]
- Cross-references: [pass/fail]
- Hooks (bash -n + exec bit): [pass/fail]
- Memory bank validate.py: [pass/fail]
- No placeholders: [pass/fail]
- Edition scoping: [pass/fail]

## Auto-fixed
- [list]

## Needs Human Attention
- [list, or "none"]
```

## Guardrails

- MUST treat any unresolved failure as "generation not done"; MUST NOT let `infra-generate` report success on failure.
- MUST NOT auto-fix anything ambiguous (e.g. rewrite a skill to satisfy a reference) - escalate instead.
- MUST confirm no unselected edition was generated.
- MUST run the seeded memory bank's own validator, not a substitute.

## Final Output

Return PASS/FAIL, the per-check results, what was auto-fixed, and what needs human attention. On PASS, `infra-generate` may report success.
