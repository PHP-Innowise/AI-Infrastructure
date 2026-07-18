# Infrastructure-Creator

> **For enforceable agent policy rules, see [AGENTS.md](AGENTS.md).**

A generator, not an accelerator. Infrastructure-Creator *builds* a bespoke accelerator for a specific **PHP project** you point it at - by scanning it, researching its actual dependencies, asking you the few things it could not determine on its own, and generating a working `AGENTS.md`, skills, agents, commands, hooks, and a seeded `memory-bank/` straight into that project's root, for only the AI tool(s) your team uses.

It is a standalone, self-contained tool: no bundled reference accelerator, no dependency on any other project. You run it from **Claude Code**, **Cursor**, or **OpenAI Codex** - whichever you already use - because the generator itself ships in all three editions.

## Why This Exists

Generic PHP boilerplate covers common stacks in the abstract, but real projects have a specific layer on top that no template can anticipate: a specific payment provider, a specific queue, a home-grown service topology, a specific CI/CD setup, internal conventions, and so on. Infrastructure-Creator looks at *your actual PHP project* and generates that missing, specific layer instead of asking you to hand-write it.

## Load and Run (Quick Start)

1. Open or clone this folder as its own workspace, separate from the PHP project you want to generate for.
2. Have the target project available as a sibling directory (or note its path).
3. From your AI tool, run the scan against the target:
   - `infra-scan ../my-php-app`
4. Glance at `tasks/TASK-{N}/infra-scan-project-profile.md`. Fix anything the scan got wrong (there is rarely much - the interview already asked about the ambiguous parts, including which AI tool your team uses).
5. Generate:
   - `infra-generate ../my-php-app`
6. Open the target project. Its new `AGENTS.md`, the AI-tool edition(s) you selected, and `memory-bank/` are ready to use.

In a hurry and you trust the scan? Run the one-shot: `infra-build ../my-php-app` chains scan -> generate and only pauses if it hits a blocking ambiguity or a collision.

## Two-Phase Workflow

```text
infra-scan <path-to-php-project>          (read-only; never writes into the target)
   -> six PHP scanners (parallel): stack, architecture, integrations, infra/ops,
      security/compliance, conventions
   -> stack-researcher (web research grounded in the real composer dependencies)
   -> clarifying-interview (asks only what evidence could not settle,
      including which AI tool(s) the target team uses)
   -> profile-synthesizer -> tasks/TASK-{N}/infra-scan-project-profile.md

   <-- REVIEW THE PROFILE (quick glance; edit if anything is wrong) -->

infra-generate <path-to-php-project>       (the only step that writes into the target)
   -> re-validates the profile against the target's current files
   -> forges (parallel): policy-forge, skill-forge, hook-forge, memory-seed
   -> agent-forge + command-forge (need the final skill list)
   -> skill-flow-composer, then bootstrap-verifier
   -> Target now has its own working AGENTS.md + selected edition(s) + memory-bank/
```

## What Gets Generated

For the selected AI-tool edition(s) only (Claude Code / Cursor / Codex - chosen during `clarifying-interview`):

- `AGENTS.md`, `DOD.md`, `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md` - policy tailored to what was found.
- A custom PHP skill set - one per detected domain/integration (e.g. a payment-integration skill if a Stripe SDK was found, a queue skill if a Redis/SQS worker was found) plus adapted universal skills (architecture, coding, testing, review, security, performance, release) for the target's actual PHP framework.
- Matching agents and commands (commands only for editions with a command layer; Codex invokes skills directly).
- Hooks (`local-context.sh`, `bash-validator.sh`, `file-naming-validator.sh`, `loop-detection.sh`) tuned to the target's real tooling and destructive-command risks.
- A seeded `memory-bank/` whose initial chunks are strictly the confirmed findings from the scan, each cited to the real file that proves it.
- A `SKILL FLOW.md` built from the skills that were actually generated, not a template.

## Directory Structure

```
Infrastructure-Creator/
├── AGENTS.md                 # Shared generator policy
├── README.md  CHANGELOG.md   # This file + change history
├── .claude/                  # Claude Code edition of the generator (source of truth)
├── .cursor/                  # Cursor edition
├── .codex/  .agents/         # Codex config/hooks/docs + Codex skills tree
├── specs/                    # This tool's own living specs
├── tasks/                    # One TASK-{N}/ per scan+generate run against a target
└── examples/                 # Illustrative sample outputs
```

The generator has no `memory-bank/` of its own - it seeds one for the target.

## Prerequisites

- Read access to the target PHP project's source tree (`composer.json`, config, `*.php`, CI/CD, IaC).
- Internet access for `stack-researcher`'s web-research pass (falls back to internal-only findings and flags the gap if unavailable).
- Python 3 (dependency-free) for `bootstrap-verifier`'s structural checks and the seeded `memory-bank/scripts/validate.py`.

## Verification

`bootstrap-verifier` runs automatically at the end of `infra-generate` and checks: frontmatter validity across all generated skills/agents/commands, cross-references pointing only at skills that actually exist, hook scripts pass `bash -n` and carry the executable bit, the seeded memory bank passes its own validator, and no template placeholders remain. Treat a failed `bootstrap-verifier` run as generation not being done yet.
