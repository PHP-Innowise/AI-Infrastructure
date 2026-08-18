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
   -> profile-synthesizer -> human Project Profile
      + machine-readable skill-generation-plan.json
      (one evidence/ownership/procedure/routing contract per proposed skill)

   <-- REVIEW THE PROFILE (what you read here is what infra-generate will build) -->

infra-generate <path-to-php-project>
   -> validates complete schema 1.4 evidence, operational safety, ownership,
      routing, invariants, path authority, and necessity
   -> builds policy/hooks/memory and evidence-scoped skill batches in staging
      (partial batches are allowed; the final complete gate is mandatory)
   -> semantic gate: evidence-anchored procedures, concrete safe verification,
      provider policy, capabilities, outputs, scope, distinctness, flow parity
   -> only then generates agents, commands, and adaptive flows
   -> verifies the complete staged bundle
   -> centrally composes approved shared .gitignore requirements
   -> publishes explicit write/watch paths with rollback, manifest last,
      and verifies again
   -> Target now has its own working AGENTS.md + selected edition(s) + memory-bank/
      + .infra-manifest.json (so a later `infra-update <path>` can upgrade it safely)
```

## Non-PHP Targets

Infrastructure-Creator only generates PHP accelerators directly - but it does not silently fail on a non-PHP target either. When `infra-scan` finds no PHP evidence, it checks for a *recognizable* non-PHP stack (Flutter/Dart, Node.js, Python, Go, Ruby, Java/Kotlin, .NET, Rust, Swift, or similar, detected from real manifest files like `pubspec.yaml`, `package.json`, `go.mod`, etc.):

- **Recognized:** it offers to build `stack-adapter` - a brand-new, fully independent sibling generator, `Infrastructure-Creator-[Stack]/`, next to this folder. The sibling carries the same evidence contracts, semantic gates, staged publication, and regression fixtures across all three editions.
- **Not recognized at all:** it reports the target out of scope, same as before.

### Quick Guide: Building A Sibling Generator

Your project isn't PHP (Flutter, Node.js, Python, Go, or similar) but you still want the same kind of bespoke, discovery-driven accelerator? Here's the whole path, start to finish:

1. **Point at your project, same as always.** Obtain its absolute path and ask your AI assistant to `Run infra-scan against "/absolute/path/to/my-flutter-app".`
2. **Let it detect the stack.** No `composer.json`/`*.php` found, so it checks for a recognizable manifest (`pubspec.yaml`, `package.json`, `go.mod`, etc.) instead of just giving up.
3. **Confirm the offer.** It asks once: *"This uses Flutter/Dart, not PHP - want me to build `Infrastructure-Creator-Flutter`, an independent sibling generator for it?"* Say yes.
   - Already certain you need this and don't want to go through `infra-scan` first? Ask the assistant to `Run infra-adapt against "/absolute/path/to/my-flutter-app".`
4. **Wait for it to build.** `stack-adapter` re-authors the generator and all six ecosystem contract catalogs, copies the stack-agnostic quality gates, then rejects stub references, mechanical substitutions, and failed semantic fixtures before reporting success.
5. **Check the report.** It tells you the new generator's path (e.g. `../Infrastructure-Creator-Flutter/`) and whether self-verification passed. If it flags a problem, don't proceed until that's resolved.
6. **Switch workspaces.** Open `Infrastructure-Creator-Flutter/` (the new folder) as its own workspace - separate from both this generator and your target project.
7. **Use it exactly like this one.** Ask the assistant to run `infra-scan` against the absolute target path, review the profile, then ask it to run `infra-generate` (or `infra-build` for the one-shot) against the same path. From this point on, everything works the same as the PHP flow above - just for Flutter.

One confirmation, one wait, then a brand-new generator ready to use for that stack.

## What Gets Generated

The generator always creates the shared, tool-neutral infrastructure and adds
only the native integration directories selected during `clarifying-interview`:

- `AGENTS.md`, `DOD.md`, `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md` - policy tailored to what was found.
- An evidence-gated custom PHP skill set: catalog names are candidates, not quotas. A skill is generated only when it has distinct project-backed selection, scope, procedure, output, and routing value.
  Every selected skill also carries concrete non-mutating verification (or a
  bounded manual assertion), expected pass/fail/skip behavior, exact local
  evidence anchors, capability and path contracts, and claim-linked critical
  invariants. Provider skills are non-networked by default.
  - **Architecture, design, and frontend** skills exist only where the detected structure and UI surface justify a separate operational workflow.
  - **Process and universal PHP** skills are selected and adapted to the target's actual conventions/tooling instead of being emitted as a fixed list. The memory quartet (`memory-bank`, `project-brain`, `checkpoint`, `memory`) remains because the generator always installs that runtime.
  - **Framework-specialty** (evidence-gated, one per confirmed pattern) - e.g. ORM patterns, migration safety, async/queue jobs, event-boundary review, caching strategy, file storage, auth scaffolding, form/validator design, admin panel, console commands, test-data factories - generated only where the scan found real evidence, never speculatively.
  - **Integrations** (one per detected package/service) - e.g. a payment-integration skill if a Stripe SDK was found, a queue skill if a Redis/SQS worker was found.
  - **Domain** (0 or more, evidence-gated) - one bounded-context review skill only when multiple confirmed rules create a coherent purpose, e.g. `billing-rules-review`; never one skill per rule/entity/status.
- Matching agents with project-specific positive/negative routing and sibling deferrals, plus thin commands (Claude/Cursor only; Codex invokes skills directly).
- Two adaptive orchestrator commands for Claude/Cursor: `flow-feature` and `flow-review` select specialists only when the current request/change intersects their validated contract. Available domain or integration reviewers are not run unconditionally.
- Eight hook roles in three groups: four evidence-tuned enforcement hooks (`local-context.sh`, `bash-validator.sh`, `file-naming-validator.sh`, `loop-detection.sh`), two automatic-memory hooks (`working-memory-read.sh`, `working-memory-write.sh`), and two orchestration hooks (`subagent-gate.sh`, `subagent-dispatch.sh`) for roster enforcement, write serialization, and completion recording. Cursor omits `working-memory-read.sh` and renders the previous turn's capsule into an ignored `alwaysApply` rule; Codex ships the dispatch hook unregistered because multi-agent execution is disabled.
- A seeded `memory-bank/` whose chunks represent cohesive durable confirmed concepts, link canonical sources, and are operated through the generated `memory-bank` skill - plus the dependency-free context-brain runtime under `memory-bank/scripts/` (`context.py`, `brain_runtime.py`, `context_retrieval.py`, `validate.py`) and the governed `project-brain/` control-plane skeleton it drives.
- A `SKILL FLOW.md` built from the skills that were actually generated, not a template.
- The upgrade contract: a version-stamp comment on a generated `AGENTS.md` and a `.infra-manifest.json` at the target root (generator version, source profile, and sha256 of every path in the explicit generation write plan) - this is what makes `infra-update` possible later. Manifest membership, not location under a managed root, exclusively defines generator ownership. Commit it with the rest of the accelerator.

None of this is a surprise at generation time: the profile gives the human
preview, while `skill-generation-plan.json` records the complete per-skill
evidence, ownership, procedure, verification, output, and routing contract.
Each selected skill carries satisfied claim-backed selection conditions;
rejected catalog candidates record their reason and missing evidence.
Unsupported or overlapping candidates are pruned before generation.
Schema 1.0 through 1.3 plans remain readable for audit and migration diagnostics
but cannot be generated or published: operational safety data cannot be
inferred. New profiles use schema 1.4 with typed procedures and verification,
provider safety, path authority, critical invariant coverage, evidence anchors,
routing fixtures, one canonical flow graph, each catalog obligation wired to the
evidence and step that carry it, a recorded baseline for every executable check,
evidence that may state an absence, the reconciled claims each skill rests on,
and a recorded decision on every piece of evidence inside a skill's own paths.

## Upgrading A Generated Accelerator

The generator keeps improving after your accelerator was generated. `infra-update` brings an existing target up to the current generator version **without losing anything your team changed**:

```text
infra-update ../my-php-app
```

How it stays safe:

1. Every `infra-generate` run builds and validates the complete accelerator in task staging, rechecks target publication and watch-only baseline hashes, then publishes an explicit path plan with rollback. `.infra-manifest.json` is copied last and `AGENTS.md` is stamped only when generated.
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
├── .claude/                  # Claude Code agents/commands and mirrored skills
├── .cursor/                  # Cursor edition
├── .codex/  .agents/         # Codex config/hooks/docs + canonical skills tree
├── specs/                    # This tool's own living specs
├── tasks/                    # One TASK-{N}/ per scan+generate run against a target
└── examples/                 # Illustrative sample outputs
```

The generator has no `memory-bank/` or `project-brain/` of its own - it seeds both for the target from the verbatim runtime bundled under `memory-seed/assets/`.

## Prerequisites

- Read access to the target PHP project's source tree (`composer.json`, config, `*.php`, CI/CD, IaC).
- Internet access for `stack-researcher`'s web-research pass (falls back to internal-only findings and flags the gap if unavailable).
- Python 3 (dependency-free) for `bootstrap-verifier`'s structural checks and the seeded `memory-bank/scripts/validate.py`.

## Optional MCP Integrations

MCP servers are optional external integrations. This accelerator requires
none. Enable only the servers that correspond to systems this project actually
uses: a small, relevant toolset consumes less context and creates a smaller
security boundary than installing every available server.

Commonly useful for a PHP project:

- [Context7](https://github.com/upstash/context7) — current, version-specific
  framework and package documentation. Most valuable when the installed
  framework or library version differs from the model's built-in knowledge,
  which is the one gap no repository-local index can close.
- [GitHub MCP Server](https://github.com/github/github-mcp-server) —
  repository, pull-request, issue, and workflow context. Prefer the official
  server with repository-scoped, least-privilege access.
- [Sentry](https://mcp.sentry.dev/) or
  [Datadog](https://docs.datadoghq.com/mcp_server/) — production errors,
  traces, and incident evidence. Pick the platform this project already uses;
  do not connect both without a concrete need.
- [Playwright MCP](https://github.com/microsoft/playwright-mcp) — browser
  interaction and UI-flow verification. Only for a browser-facing surface.
- [Linear](https://linear.app/docs/mcp) or
  [Atlassian Rovo](https://github.com/atlassian/atlassian-mcp-server) —
  requirements and issue context. Prefer read-only access.
- A database-specific MCP server — schema inspection and query diagnostics
  when repository evidence is insufficient. Use a dedicated read-only account
  and never point one at an unrestricted production database by default.

### Security rules

1. Prefer first-party servers and official documentation.
2. Start with read-only scopes, the smallest toolset, and one project or
   organization boundary.
3. Keep tokens, connection strings, and credentials outside the repository.
4. Require human confirmation for writes, deployments, issue transitions, and
   other consequential actions.
5. Treat MCP output as external evidence: verify important claims against
   canonical project sources before changing code.
6. Do not add generic filesystem or memory MCP servers merely to duplicate the
   repository access, Memory Bank, Project Brain, or Local Context Engine this
   accelerator already supplies.

### Context rules

Tool definitions are not free and are not paid once. They sit at the front of
the model's context and are re-read on every turn of the session, so a server
you never call still costs you on every prompt.

7. Scope every server to the smallest toolset it needs. `github-mcp-server`
   defaults to five toolsets (`context, repos, issues, pull_requests, users`)
   and `--toolsets all` is substantially larger; do not use `all`. Run with
   `--read-only` (`GITHUB_READ_ONLY=1`) unless a workflow needs writes — which
   also satisfies rule 4. Run `playwright-mcp` without `--caps` unless a
   capability is genuinely required. A server left at its widest setting can
   occupy several times the context of this accelerator's own `AGENTS.md` and
   all of its skill descriptions combined.
8. Decide the server set before starting a session. Tool definitions are the
   first tier of the prompt cache, ahead of the system prompt and the
   conversation, so adding or removing a server mid-session invalidates that
   cache and the accumulated context is re-established at full price.
9. What a server returns usually costs more than what it declares, because a
   large result stays in context and is re-read for the rest of the session.
   Prefer bounded, structured output, and constrain at the call site anything
   that can return a page, a log stream, or a query result set. Where an API
   exposes no size parameter, the only lever is a narrower request.

Verify rather than estimate: `/context` reports the MCP tools row for the
current session.

## Verification

`bootstrap-verifier` runs automatically at the end of `infra-generate` (and
`infra-update`) and checks plan-level inventory conflicts before authoring,
manifest-owned frontmatter/cross-references, hooks and wiring, the generated
memory/runtime surface, every manifest member's existence and hash, a tracked
`AGENTS.md` stamp, and placeholders across generated text. The immutable
`memory-bank/templates/chunk.md` and `memory-bank/scripts/validate.py` may carry
only their exact ISO-date format token. A shared root `.gitignore` is validated
through its declared managed requirements rather than unrelated team comments.
Unmanifested team files are ignored in both full and merge modes.
