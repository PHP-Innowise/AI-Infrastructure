# Accelerator Installer

`scripts/install_accelerator.py` installs a ready-made Laravel, Symfony, or PHP
Core accelerator into an existing project from a versioned production
inventory. It copies the selected production payload into the target root and
refuses unsupported collisions before writing anything.

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

The inventories are the executable boundary between the source editions and
the production payload. They may record source files that are deliberately
excluded from installation and may apply production-specific selection rules;
the installer, inventory verifier, and tests must agree on that resolved
payload. Treat the inventory as versioned data and review it when its schema
changes. Do not infer the installed file set by counting every tracked file in
an edition directory.

The production payload keeps the files required to operate and adapt the
accelerator: policies, native tool integrations, runtime scripts, workflow
documentation, templates, Memory Bank, Project Brain, living-spec and
lowercase `tasks/` scaffolds. Source-only research, test suites, worked
examples, and bundled uppercase `Task/` product/design material remain useful
in this repository but are not copied into a consuming project.

`Task/` and `tasks/` are intentionally different:

- uppercase `Task/` is optional client-input space. A consuming project or
  generation workflow may create and populate it when actual client
  requirements or design assets exist; the ready-made package does not seed it
  with this repository's material;
- lowercase `tasks/` is the installed operational scaffold for temporary,
  skill-prefixed `TASK-NNN/` work artifacts.

Exact payload counts are reported by the verifier and the install transcript.
They can change as the inventory evolves, so documentation does not pin a
hand-maintained total.

## 3. Run a Dry Run

```bash
TARGET="/path/to/existing-project"

python3 scripts/install_accelerator.py \
  --edition "PHP Core" \
  --target "$TARGET" \
  --tool cursor \
  --merge-existing \
  --dry-run
```

Available tool values are:

- `--tool claude` for `.claude/`;
- `--tool cursor` for `.cursor/`;
- `--tool codex` for `.agents/` and `.codex/`.

Repeat `--tool` to select multiple integrations. Omit it to install all three.
Shared policy, workflow, Memory Bank, and Project Brain files are included with
every selection. Tool selection narrows native integration trees; it does not
re-add source-only files excluded from the production payload.

`--merge-existing` handles the standard root files commonly present in an
existing project:

- identical files are reported as `UNCHANGED`;
- `.gitignore` and `.gitattributes` retain project entries and receive only
  missing accelerator directives in an installer-managed block;
- existing `AGENTS.md` retains project policy first and receives a marked,
  replaceable accelerator policy block;
- existing `README.md` remains untouched and the accelerator documentation is
  installed as `ACCELERATOR.md`.

Review `WOULD_COPY`, `WOULD_MERGE`, and `WOULD_COPY_AS` lines. Any other
existing selected path is still reported as `COLLISION`; the command returns a
nonzero exit code and writes nothing.

## 4. Resolve Collisions

Keep `--merge-existing` in the command for normal adoption. It is conservative:
unsupported collisions still require review. For each remaining conflict,
choose one of these actions:

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
set -o pipefail
python3 scripts/install_accelerator.py \
  --edition "PHP Core" \
  --target "$TARGET" \
  --tool cursor \
  --merge-existing |
  tee "/safe/backup/path/accelerator-install-transcript.txt"
```

Keep the complete transcript, including `COPY`, `MERGE`, `COPY_AS`, and
`UNCHANGED` records. It is an action log, not a restorable backup and not a
hash-based uninstall manifest. Pair it with the pre-install Git commit or
backup for safe rollback. A final `COMPLETE` line reports the edition, selected
tools, and number of selected inventory entries; that number includes
`UNCHANGED` entries and must not be interpreted as the number of files created.

When piping through `tee`, preserve the installer's exit status. In shells
without pipeline failure propagation, inspect the final status explicitly;
otherwise `tee` can succeed after the installer failed.

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

Use the saved transcript, the pre-install recovery point, and Git diff:

- remove paths recorded as `COPY` or `COPY_AS` only when they were created by
  that installation and have not since become project-owned;
- for `MERGE` records, restore the pre-install file when exact rollback is
  required. Removing only an installer-managed block is safe only after review:
  additive merges can omit accelerator directives that already existed in the
  project, and those project-owned lines must remain;
- do nothing for `UNCHANGED` records.

The installer has no automatic rollback command. A transcript records what the
successful run reported, but it does not contain previous file contents.

Never delete whole `.claude/`, `.cursor/`, `.agents/`, `.codex/`,
`memory-bank/`, or `project-brain/` directories because they may contain
pre-existing project files or active records.

## Maintainer Commands

After intentionally changing distribution files:

```bash
python3 scripts/install_accelerator.py --write-inventories
python3 scripts/install_accelerator.py --verify-inventories
python3 -m unittest tests.test_installation
```

Review generated inventory changes before committing them.

Inventories are generated from `git ls-files --cached`, so stage new
distribution files before regenerating: an unstaged file is not part of the
payload. The same rule keeps untracked working-tree content - a client
application under `Task/`, caches, `.env` files - out of the shipped
inventories and out of verification.

## Further Documentation

- [Full safe-adoption procedure](../docs/ADOPTION.md)
- [Tool-specific activation](../docs/TOOL-INTEGRATIONS.md)
- [Operations and health checks](../docs/OPERATIONS.md)
- [Troubleshooting](../docs/TROUBLESHOOTING.md)
