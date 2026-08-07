# Infrastructure-Creator

> **For enforceable agent policy rules, see [AGENTS.md](AGENTS.md).**

A generator, not an accelerator. Infrastructure-Creator *builds* a bespoke accelerator for a specific **PHP project** you point it at - by scanning it, researching its actual dependencies, asking you the few things it could not determine on its own, and generating a working `AGENTS.md`, skills, agents, commands, hooks, and a seeded `memory-bank/` straight into that project's root, for only the AI tool(s) your team uses.

It is a standalone, self-contained tool with no bundled reference accelerator
and no dependency on another project. Any AI assistant that can read repository
instructions and follow the documented workflows can operate it; native
integration directories are optional convenience layers, not a requirement for
the scan-and-generate process.

## Why This Exists

Generic PHP boilerplate covers common stacks in the abstract, but real projects have a specific layer on top that no template can anticipate: a specific payment provider, a specific queue, a home-grown service topology, a specific CI/CD setup, internal conventions, and so on. Infrastructure-Creator looks at *your actual PHP project* and generates that missing, specific layer instead of asking you to hand-write it.

## Load and Run (Quick Start)

These are workflows for an AI assistant, not terminal executables. Invoke them
through whatever command, skill, prompt, or agent mechanism your assistant
supports.

1. Open or clone the repository containing `Infrastructure-Creator/`.
2. Obtain the existing target project's absolute filesystem path using your
   editor, file manager, or terminal.
3. Ask your AI assistant:

   ```text
   Run infra-scan against the existing target project at
   "/absolute/path/to/my-php-app".
   ```

   Keep the path in quotes when any directory name contains spaces. The absolute
   path avoids ambiguity between the visible project root and the assistant's
   working directory.
4. Read `Infrastructure-Creator/tasks/TASK-{N}/infra-scan-project-profile.md`
   (or `tasks/TASK-{N}/...` when `Infrastructure-Creator/` itself is the
   workspace). Fix anything wrong before generation.
5. Ask the assistant to generate from the reviewed profile:

   ```text
   Run infra-generate against
   "/absolute/path/to/my-php-app".
   ```

6. Open the target project. Its new `AGENTS.md`, selected AI-tool edition(s),
   and `memory-bank/` are ready to use.

In a hurry and you trust the scan, ask the assistant to
`Run infra-build against "/absolute/path/to/my-php-app".` It chains scan ->
generate and pauses only for a blocking ambiguity or collision.

**Target isn't PHP?** `infra-scan` will tell you rather than silently failing -
see "Non-PHP Targets" below.

## Two-Phase Workflow

```text
infra-scan <path-to-php-project>          (read-only; never writes into the target)
   -> seven PHP scanners (parallel): stack, architecture,
      integrations, infra/ops, security/compliance, conventions, domain behavior
   -> stack-researcher (web research grounded in the real composer dependencies)
   -> clarifying-interview (asks only what evidence could not settle,
      including which AI tool(s) the target team uses)
   -> profile-synthesizer -> tasks/TASK-{N}/infra-scan-project-profile.md
      (incl. behavioral contract, per-skill descriptions, counts, memory preview)

   <-- REVIEW THE PROFILE (what you read here is what infra-generate will build) -->

infra-generate <path-to-php-project>       (the only step that writes into the target)
   -> re-validates the profile against the target's current files
   -> forges (parallel): policy-forge, skill-forge, hook-forge, memory-seed
   -> agent-forge + command-forge (need the final skill list)
   -> skill-flow-composer, then version stamp + .infra-manifest.json, then bootstrap-verifier
   -> Target now has its own working AGENTS.md + selected edition(s) + memory-bank/
      + .infra-manifest.json (so a later `infra-update <path>` can upgrade it safely)
```

## Non-PHP Targets

Infrastructure-Creator only generates PHP accelerators directly - but it does not silently fail on a non-PHP target either. When `infra-scan` finds no PHP evidence, it checks for a *recognizable* non-PHP stack (Flutter/Dart, Node.js, Python, Go, Ruby, Java/Kotlin, .NET, Rust, Swift, or similar, detected from real manifest files like `pubspec.yaml`, `package.json`, `go.mod`, etc.):

- **Recognized:** it offers to build `stack-adapter` - a brand-new, fully independent sibling generator, `Infrastructure-Creator-[Stack]/`, next to this folder. That architecture contains 23 skills, including domain-behavior discovery, across all three editions.
- **Not recognized at all:** it reports the target out of scope, same as before.

### Quick Guide: Building A Sibling Generator

Your project isn't PHP (Flutter, Node.js, Python, Go, or similar) but you still want the same kind of bespoke, discovery-driven accelerator? Here's the whole path, start to finish:

1. **Point at your project, same as always.** Obtain its absolute path and ask your AI assistant to `Run infra-scan against "/absolute/path/to/my-flutter-app".`
2. **Let it detect the stack.** No `composer.json`/`*.php` found, so it checks for a recognizable manifest (`pubspec.yaml`, `package.json`, `go.mod`, etc.) instead of just giving up.
3. **Confirm the offer.** It asks once: *"This uses Flutter/Dart, not PHP - want me to build `Infrastructure-Creator-Flutter`, an independent sibling generator for it?"* Say yes.
   - Already certain you need this and don't want to go through `infra-scan` first? Ask the assistant to `Run infra-adapt against "/absolute/path/to/my-flutter-app".`
4. **Wait for it to build.** `stack-adapter` re-authors all 23 skills, including the new stack's own domain-behavior scanner, then mirrors and verifies the sibling.
5. **Check the report.** It tells you the new generator's path (e.g. `../Infrastructure-Creator-Flutter/`) and whether self-verification passed. If it flags a problem, don't proceed until that's resolved.
6. **Switch workspaces.** Open `Infrastructure-Creator-Flutter/` (the new folder) as its own workspace - separate from both this generator and your target project.
7. **Use it exactly like this one.** Ask the assistant to run `infra-scan` against the absolute target path, review the profile, then ask it to run `infra-generate` (or `infra-build` for the one-shot) against the same path. From this point on, everything works the same as the PHP flow above - just for Flutter.

One confirmation, one wait, then a brand-new generator ready to use for that stack.

## What Gets Generated

The generator always creates the shared, tool-neutral infrastructure and adds
only the native integration directories selected during `clarifying-interview`:

- `AGENTS.md`, `DOD.md`, `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md` - policy tailored to what was found.
- A full, custom PHP skill set in eight groups - not just a handful of generic skills:
  - **Architecture** (1) - grounded in the detected pattern (monolith/modular-monolith/microservices/event-driven).
  - **Design & interaction** (3, always) - `architecture-implementer`, `api-designer`, `database-designer`, shaped to the target's real scaffolding tooling, API shape, and persistence layer.
  - **Frontend** (0 or 5, only if a rendering/asset layer exists) - `frontend-design`, `coder-frontend`, `wcag-accessibility`, `web-design-guidelines`, `browser-verify`.
  - **Process & workflow** (18, always, framework-agnostic) - fourteen workflow mechanics plus the memory quartet (`memory-bank`, `project-brain`, `checkpoint`, `memory`) that operates the seeded memory layer.
  - **Universal PHP** (7) - `coding`, `testing`, `code-review`, `security-review`, `performance`, `release`, `debugging`, adapted to the target's actual PHP framework/version/tooling.
  - **Framework-specialty** (evidence-gated, one per confirmed pattern) - e.g. ORM patterns, migration safety, async/queue jobs, event-boundary review, caching strategy, file storage, auth scaffolding, form/validator design, admin panel, console commands, test-data factories - generated only where the scan found real evidence, never speculatively.
  - **Integrations** (one per detected package/service) - e.g. a payment-integration skill if a Stripe SDK was found, a queue skill if a Redis/SQS worker was found.
  - **Domain** (0 or more, evidence-gated) - one bounded-context review skill only when multiple confirmed rules create a coherent purpose, e.g. `billing-rules-review`; never one skill per rule/entity/status.
- Matching agents and commands (commands only for editions with a command layer; Codex invokes skills directly).
- Hooks: four enforcement hooks (`local-context.sh`, `bash-validator.sh`, `file-naming-validator.sh`, `loop-detection.sh`) tuned to the target's real tooling and destructive-command risks, plus the automatic working-memory pair (`working-memory-read.sh` at prompt time, `working-memory-write.sh` at turn end; Cursor gets only the write half - it has no prompt-time hook event).
- A seeded `memory-bank/` whose chunks represent cohesive durable confirmed concepts, link canonical sources, and are operated through the generated `memory-bank` skill - plus the dependency-free context-brain runtime under `memory-bank/scripts/` (`context.py`, `brain_runtime.py`, `context_retrieval.py`, `validate.py`) and the governed `project-brain/` control-plane skeleton it drives.
- A `SKILL FLOW.md` built from the skills that were actually generated, not a template.
- The upgrade contract: a version-stamp comment on a generated `AGENTS.md` and a `.infra-manifest.json` at the target root (generator version, source profile, and sha256 of every path in the explicit generation write plan) - this is what makes `infra-update` possible later. Manifest membership, not location under a managed root, exclusively defines generator ownership. Commit it with the rest of the accelerator.

None of this is a surprise at generation time: the profile you review after `infra-scan` (step 4 of the Quick Start above) already spells out a one-line description of every skill about to be written, the exact agent/command counts for your selected edition(s), and a full preview table of every memory-bank chunk `infra-generate` will seed.

## Upgrading A Generated Accelerator

The generator keeps improving after your accelerator was generated. `infra-update` brings an existing target up to the current generator version **without losing anything your team changed**:

```text
infra-update ../my-php-app
```

How it stays safe:

1. Every `infra-generate` run ends by writing `.infra-manifest.json` into the target - the generator version (from this repo's root `VERSION` file, the single version source), the profile it consumed, and the sha256 of every file in that run's explicit write plan - and stamping `AGENTS.md` only when that file was generated (`<!-- Generated by Infrastructure-Creator v1.4.0 | TASK-003 | 2026-08-02 -->`).
2. `infra-update` re-validates the profile, regenerates everything into a staging area (never into your project), then compares each file three ways: manifest hash vs. what's in your project vs. what would be generated now.
   - Hash unchanged since generation -> your team never touched it -> safely replaced with the new version.
   - Hash differs (or the file was deleted) -> it's yours now -> it goes into a "requires decision" report showing all three sides; nothing is overwritten without your explicit per-file answer.
   - Not in the manifest at all -> it was never the generator's -> never touched, never even diffed.
3. The run ends by rewriting the manifest and re-running `bootstrap-verifier`, so the next upgrade has a fresh, honest baseline.

**Legacy targets** (generated by v1.3.x or earlier) have no `.infra-manifest.json`; `infra-update` aborts with recovery options instead of guessing which files are yours - either re-run `infra-generate` through its collision guard, or (only if you are certain nothing generated was ever edited) build the manifest by hand with the recipe in `infra-generate`'s SKILL.md and update from there.

## Directory Structure

```
Infrastructure-Creator/
├── AGENTS.md                 # Shared generator policy
├── VERSION                   # Single source of the generator's version
├── README.md  CHANGELOG.md   # This file + change history
├── .claude/                  # Claude Code edition of the generator (source of truth)
├── .cursor/                  # Cursor edition
├── .codex/  .agents/         # Codex config/hooks/docs + Codex skills tree
├── specs/                    # This tool's own living specs
├── tasks/                    # One TASK-{N}/ per scan+generate run against a target
└── examples/                 # Illustrative sample outputs
```

The generator has no `memory-bank/` or `project-brain/` of its own - it seeds both for the target from the verbatim runtime bundled under `memory-seed/assets/`.

## Prerequisites

- Read access to the target PHP project's source tree (`composer.json`, config, `*.php`, CI/CD, IaC).
- Internet access for `stack-researcher`'s web-research pass (falls back to internal-only findings and flags the gap if unavailable).
- Python 3 (dependency-free) for `bootstrap-verifier`'s structural checks and the seeded `memory-bank/scripts/validate.py`.

## Verification

`bootstrap-verifier` runs automatically at the end of `infra-generate` (and `infra-update`) and checks manifest-owned frontmatter/cross-references, hooks and wiring, the generated memory/runtime surface, every manifest member's existence and hash, a tracked `AGENTS.md` stamp, and placeholders across every manifest-owned text file. Unmanifested team files are ignored in both full and merge modes. Treat a failed `bootstrap-verifier` run as generation not being done yet.
