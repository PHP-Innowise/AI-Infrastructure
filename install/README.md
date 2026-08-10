# Accelerator Installer

`scripts/install_accelerator.py` installs a ready-made Laravel, Symfony, or PHP
Core accelerator into an existing project from a versioned file inventory. It
copies the contents of the selected edition into the target root and refuses
collisions before writing anything.

Use this installer for ready-made editions. For projects with substantial
custom architecture, integrations, or internal conventions, use
[Infrastructure-Creator](../Infrastructure-Creator/README.md).

## Prerequisites

- Clone or check out the accelerator repository separately from the target.
- Use Python 3.9 or newer.
- Put the target project in a clean, recoverable Git state.
- Back up existing AI configuration, policies, Memory Bank, and Project Brain
  data.
- Run commands below from the accelerator repository root.

The installer does not install PHP, Composer, framework packages, AI clients,
or other dependencies.

## 1. Choose an Edition

- `Laravel` for Laravel projects.
- `Symfony` for Symfony projects.
- `PHP Core` for native Composer/PSR projects, microframeworks, or frameworks
  without a dedicated edition.

Use the edition supported by evidence in the target project's `composer.json`
and lock file.

## 2. Verify the Distribution Inventories

```bash
python3 scripts/install_accelerator.py --verify-inventories
```

The command must report `VERIFIED` for Laravel, Symfony, and PHP Core. An
inventory mismatch means the source checkout is incomplete or its distribution
files changed without an inventory update.

## 3. Run a Dry Run

```bash
TARGET="/path/to/existing-project"

python3 scripts/install_accelerator.py \
  --edition "PHP Core" \
  --target "$TARGET" \
  --tool cursor \
  --dry-run
```

Available tool values are:

- `--tool claude` for `.claude/`;
- `--tool cursor` for `.cursor/`;
- `--tool codex` for `.agents/` and `.codex/`.

Repeat `--tool` to select multiple integrations. Omit it to install all three.
Shared policy, workflow, Memory Bank, and Project Brain files are included with
every selection.

Review all `WOULD_COPY` lines. If the target contains any selected path, the
installer prints `COLLISION`, returns a nonzero exit code, and copies nothing.

## 4. Resolve Collisions

Do not use `--overwrite` for normal adoption. Review every conflict and choose
one of these actions:

1. Keep the existing project file.
2. Merge compatible accelerator behavior into it manually.
3. Rename and update references when the AI tool supports that layout.
4. Abort the installation.

Never replace existing project policy, hooks, skills, Memory Bank chunks,
Project Brain records, or `.gitignore` without a semantic review.

## 5. Install and Save the Transcript

After obtaining a collision-free dry run, repeat the same command without
`--dry-run`:

```bash
python3 scripts/install_accelerator.py \
  --edition "PHP Core" \
  --target "$TARGET" \
  --tool cursor |
  tee "/safe/backup/path/accelerator-install-transcript.txt"
```

Keep the `COPY` transcript. It identifies files created by the installation and
is required for safe rollback. A final `COMPLETE` line reports the edition,
selected tools, and copied file count.

## 6. Validate the Installation

From the target project root:

```bash
python3 memory-bank/scripts/validate.py
python3 project-brain/scripts/validate.py --root .
python3 memory-bank/scripts/context.py status --json
python3 memory-bank/scripts/context.py validate --json
python3 memory-bank/scripts/context.py index
git status --short
```

Also run the target project's normal Composer validation, tests, formatter,
static analysis, and framework checks. Confirm that
`memory-bank/local/context.db` is ignored and that the selected AI client loads
its skills and hooks.

## Infrastructure-Creator: Generate a Custom Installation

Use Infrastructure-Creator instead of the ready-made installer when the target
has substantial custom architecture, integrations, domain rules, internal
conventions, or specialized CI/CD. Infrastructure-Creator remains outside the
target project and generates a bespoke accelerator from evidence found in that
project.

Do not run `scripts/install_accelerator.py` for this path. Infrastructure-Creator
is operated by an AI assistant through its own workflows; `infra-scan`,
`infra-generate`, `infra-build`, and `infra-update` are not terminal
executables.

### What to Copy or Clone

Keep two separate directories:

```text
/workspaces/
├── Infrastructure-Creator/   # generator workspace
└── my-php-project/           # target project
```

Choose one of these setups:

- recommended: use a clean clone of this complete repository and open its
  `Infrastructure-Creator/` directory as the AI workspace; or
- export the version-controlled contents of the complete
  `Infrastructure-Creator/` directory to another location outside the target
  and open that directory.

Copy the whole `Infrastructure-Creator/` directory, including its `AGENTS.md`,
`VERSION`, `.claude/`, `.cursor/`, `.agents/`, `.codex/`, `specs/`, `tasks/`,
and bundled skill assets. Do not copy only one hidden AI-tool directory:
generation workflows depend on shared policy, scanners, schemas, templates,
verification scripts, and the generator version.

Do not carry over `.git/`, caches, local scratch files, or `tasks/TASK-*`
outputs from previous generator runs. A clean checkout keeps the task counter,
templates, and distribution files without mixing evidence from another target.

Do **not**:

- copy `Infrastructure-Creator/` into the target project;
- point the generator at its own directory;
- copy the ready-made `Laravel/`, `Symfony/`, or `PHP Core/` edition when using
  this custom-generation path.

### Run the Scan

1. Obtain the target PHP project's absolute path.
2. Open `Infrastructure-Creator/` in the AI tool that will run the generator.
3. In Claude Code or Cursor, run:

   ```text
   /infra-scan "/absolute/path/to/my-php-project"
   ```

   In Codex, ask it to use the `infra-scan` skill against the same path.

`infra-scan` is read-only on the target. It allocates the next
`Infrastructure-Creator/tasks/TASK-{N}/` directory and writes the scan evidence
there, never into the target project.

### Scan Report and Human Review

The main scan report and required human-review checkpoint is:

```text
Infrastructure-Creator/tasks/TASK-{N}/infra-scan-project-profile.md
```

This Project Profile is the contract consumed by `infra-generate`. It
summarizes:

- detected PHP and framework versions, dependencies, entry points, and tooling;
- architecture, module boundaries, persistence, queues, integrations, and
  frontend signals;
- CI/CD, infrastructure, security, compliance signals, and project
  conventions;
- domain vocabulary, invariants, state transitions, permissions, audit
  obligations, risky workflows, and critical regression scenarios;
- confirmed, inferred, unknown, and contradictory findings with source
  evidence and confidence;
- selected AI tools and expected skill, agent, command, and hook counts;
- the exact proposed generated infrastructure and initial Memory Bank preview;
- open questions, unavailable research, and expected generation collisions.

The same task directory keeps the detailed evidence behind that report:

```text
tasks/TASK-{N}/
├── stack-scanner-findings.md
├── architecture-scanner-findings.md
├── integration-scanner-findings.md
├── infra-ops-scanner-findings.md
├── security-compliance-scanner-findings.md
├── conventions-scanner-findings.md
├── domain-behavior-scanner-findings.md
├── stack-researcher-findings.md
├── clarifying-interview-questions.md
├── clarifying-interview-answers.md
└── infra-scan-project-profile.md          # main report
```

Read the complete Project Profile, not only its summary. Pay particular
attention to the behavioral contract, proposed generated infrastructure,
Memory Bank preview, selected AI tools, low-confidence findings, contradictions,
and collision notes. Correct the report before generation if anything is
wrong. Running `infra-scan` alone does not install or modify anything in the
target.

### Generate from the Approved Report

After reviewing and approving the scan report, generate the accelerator:

```text
/infra-generate "/absolute/path/to/my-php-project"
```

In Codex, invoke the `infra-generate` skill by name.

`infra-generate` revalidates the approved profile and its cited evidence
against the current target before writing. If the target changed since the
scan, the generator reports the drift instead of silently consuming stale
findings. It also stops for an explicit overwrite/merge/abort decision when
accelerator files already exist, generates only the selected AI-tool
integrations, and runs `bootstrap-verifier` before reporting success.

After generation, review both:

```text
Infrastructure-Creator/tasks/TASK-{N}/infra-generate-report.md
/absolute/path/to/my-php-project/.infra-manifest.json
```

The report explains what was generated and whether verification passed. The
manifest is the target-side ownership record used by future updates.

### Resulting Target Structure

The exact skill set depends on the scan evidence and selected tools. The target
structure follows this model:

```text
my-php-project/
├── AGENTS.md                     # one shared, project-specific policy
├── .infra-manifest.json          # generated-file ownership and hashes
├── memory-bank/                  # durable knowledge and context runtime
│   ├── README.md
│   ├── INDEX.md
│   ├── chunks/
│   ├── scripts/
│   ├── templates/
│   └── local/                    # ignored local SQLite/runtime state
├── project-brain/                # governed active-work control plane
│   ├── PROTOCOL.md
│   ├── config/
│   ├── schemas/
│   ├── templates/
│   ├── indexes/
│   ├── control/
│   ├── dynamic/
│   ├── archive/
│   └── local/
├── .claude/                      # only when Claude Code was selected
│   ├── DOD.md
│   ├── GOLDEN-PRINCIPLES.md
│   ├── STABILIZATION.md
│   ├── settings.json
│   ├── commands/
│   ├── agents/
│   ├── skills/
│   └── hooks/
├── .cursor/                      # only when Cursor was selected
│   ├── DOD.md
│   ├── GOLDEN-PRINCIPLES.md
│   ├── STABILIZATION.md
│   ├── hooks.json
│   ├── commands/
│   ├── agents/
│   ├── skills/
│   ├── rules/
│   └── hooks/
├── .agents/                      # only when Codex was selected
│   └── skills/
└── .codex/                       # only when Codex was selected
    ├── DOD.md
    ├── GOLDEN-PRINCIPLES.md
    ├── STABILIZATION.md
    ├── config.toml
    ├── hooks.json
    └── hooks/
```

Claude Code and Cursor receive native command, agent, and skill layers. Codex
discovers skills from `.agents/skills/`; it intentionally receives no generated
command or agent-wrapper directories.

The generated `AGENTS.md`, policies, skills, hooks, and initial memory are based
on the target's actual stack and evidence rather than copied from a ready-made
edition. `.infra-manifest.json` records the generator version, source profile,
generation mode, and hashes of generator-owned files. Live Memory Bank content
and Project Brain records become target-team state and are not treated as
replaceable generator-owned runtime data.

The tree above shows the accelerator infrastructure only. Existing application
code, Composer files, framework configuration, tests, CI, and documentation
remain in place; Infrastructure-Creator does not move the application into a
new directory or install project dependencies.

### One-Step Build and Future Updates

For a faster one-step flow, use:

```text
/infra-build "/absolute/path/to/my-php-project"
```

It chains scan and generation, pausing when a blocking ambiguity or collision
requires a human decision.

To update a previously generated accelerator later, run:

```text
/infra-update "/absolute/path/to/my-php-project"
```

The update requires `.infra-manifest.json`. It regenerates into staging,
replaces only manifest-tracked files that still match their previous hashes,
and asks for a per-file decision before changing anything edited or deleted by
the target team. Files absent from the manifest are not touched.

See the
[Infrastructure-Creator quick start](../Infrastructure-Creator/README.md#load-and-run-quick-start)
and
[generated structure](../Infrastructure-Creator/README.md#what-gets-generated)
for the complete scan, generation, collision, ownership, and update contracts.

## Ready-made Installer Rollback

Use the saved transcript to remove only files created by the installer. Never
delete whole `.claude/`, `.cursor/`, `.agents/`, `.codex/`, `memory-bank/`, or
`project-brain/` directories because they may contain pre-existing project
files or active records.

## Maintainer Commands

After intentionally changing distribution files:

```bash
python3 scripts/install_accelerator.py --write-inventories
python3 scripts/install_accelerator.py --verify-inventories
python3 -m unittest tests.test_installation
```

Review generated inventory changes before committing them.

## Further Documentation

- [Full safe-adoption procedure](../docs/ADOPTION.md)
- [Tool-specific activation](../docs/TOOL-INTEGRATIONS.md)
- [Operations and health checks](../docs/OPERATIONS.md)
- [Troubleshooting](../docs/TROUBLESHOOTING.md)
