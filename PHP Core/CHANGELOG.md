# Changelog

Shared-core history - the Python memory/context core
(`memory-bank/scripts`, `memory-bank/tests`, `project-brain/`), the
tool hooks, and the mirror machinery common to the Laravel, Symfony
and PHP Core editions - is recorded once in the root
[`CHANGELOG.md`](../CHANGELOG.md). This file records only
PHP Core-specific changes. The edition's release version is the
`VERSION` file beside this changelog; the SessionStart hook prints
it at the top of every session.

## Unreleased

### Changed

- Memory promotion policy now matches runtime configuration: eligible verified
  terminal records may be applied unattended only when automatic promotion is
  enabled and remain explicitly marked as unreviewed; otherwise an independent
  human review is required.

### Added

- **Workflow skills now produce and retire durable knowledge, and the
  procedural rules gained a lifecycle** (roadmap H2-02, H2-03, H2-04, H2-05,
  H2-07; the engine changes behind them are in the root changelog).
  - `memory-bank` skill: retiring a chunk is `context.py bank-retire`, not a
    hand edit of frontmatter on both sides of a replacement link, and a chunk
    written by hand is followed by `bank-reverify` so it can later notice that
    what it cites has moved on. The two questions `valid_to` and
    `superseded_by` answer are spelled out, because the field was named in no
    skill and an agent following policy honestly set a status and never wrote
    a date.
  - `/flow-review` materializes each confirmed finding as a `finding` record —
    from the orchestrator after synthesis, not from the review agents, which
    stay read-only so the stage keeps exactly one write-capable agent. Until
    now a review wrote its findings into the task's `--progress`, and the
    `task` record type can never be promoted, so nothing a review produced
    could ever become durable memory.
  - `systematic-debugger` records its confirmed root cause directly (it is
    already declared write-capable); `code-reviewer` and `security-reviewer`
    gained the opposite instruction, so the division is stated where each
    skill is read rather than inferred.
  - `DOD.md`, Standard tier: every confirmed review or debugging finding
    exists as a `finding` record — resolved, or explicitly deferred.
  - `reflect` skill and `STABILIZATION.md`: the rule template gained
    `Retired:` and `Superseded-by:` and the file gained a `## Retired rules`
    section, and the skill now takes an explicit add / overwrite / retire
    decision before writing. A pillar that can only add runs out of budget,
    because `AGENTS.md` is paid on every session and gated in CI.
  - `.accelerator-policy-lock.json` ships with the edition: a manifest of
    sha256 digests over the surface the model reads, plus each agent's
    declared model. The session banner prints its short digest beside the
    version, because a version that moves once a release says nothing about
    whether the prompt layer changed.

- Agent `<example>` blocks moved out of `description:` frontmatter into a
  `## Selection examples` body section. An agent's description is loaded into
  the orchestrator's context on every session, spawned or not, and the
  examples were about two thirds of those bytes while teaching the selector
  what the surrounding prose already says. Nothing was deleted - the blocks
  sit verbatim in the body, where a reader still finds them and a session no
  longer pays for them. Measured with cl100k: the agent listing drops from
  5262 t to 1489 t per session, and the edition's whole startup surface
  from 11101 t to 7322 t.

- Spec-driven development: the `sdd` skill and the `/sdd` flow command.
  `/sdd` runs specify -> design -> **checkpoint** -> task breakdown ->
  **checkpoint** -> execute task by task -> tests -> parallel review ->
  verify, spawning roster agents from the main conversation like the other
  flows. What makes it spec-driven rather than a second feature flow is that
  no stage completes without its artifact: the spec and the design land in
  `specs/` and are registered in `specs/MANIFEST.md`, the task breakdown lands
  in `tasks/<TASK-ID>/writing-plans-plan.md` as a checkbox list, and execution
  marks a box and records a `context.py update` per task. The spec carries
  user scenarios, numbered acceptance criteria and an edge-case table; every
  task cites the criteria it serves, each coder capsule carries those criteria
  verbatim rather than just the spec's path, and tests are written per
  criterion. Where a criterion cannot be met as written the flow stops and
  amends the spec instead of quietly redefining done. That citation chain is
  what keeps implementation tied to the spec, and the artifacts are what make a
  run resumable - `/sdd` reads the artifacts, works out which phase is done,
  and continues at the first gap instead of restarting. It drives the
  previously inert `specs/MANIFEST.md` scaffold, adds no new agent, and does
  not integrate: `/finishing-branch` stays a separate step. `/sdd` is listed
  among the sanctioned flow commands in AGENTS.md, so the subagent gate
  treats it as one.

- Opt-in orchestration flows: `/flow-feature` (requirements -> architecture
  -> plan -> checkpoint -> code -> tests -> parallel review -> verify ->
  checkpoint -> finishing-branch) and `/flow-review` (three read-only review
  agents in parallel, one synthesized report). The main conversation is the
  orchestrator: it spawns only roster agents, passes each a bounded
  delegation capsule, and pauses at declared checkpoints. AGENTS.md gains
  the "Orchestration (Flows, SCOPED)" section; SKILL FLOW.md documents the
  flows. Cursor mirrors are generated; Codex keeps its sequential skill
  flow by design.


## 2.0.0 - 2026-08-07

### Shared context runtime

- Adopted the 2026-08-06 shared-core context remediation documented in the
  root changelog: pre-index privacy rejection, canonical task phases, bounded
  2/3/1 capsules, explicit completion, Cursor warming continuity, retrieval
  quality gates, and metadata-only telemetry.
- Adopted the shared builtin hook input hardening and deterministic,
  selected-tool installation inventory/clean-install verification documented in
  the root changelog.

### Fixed

- **`bash-validator.sh` never blocked anything, and the wiring never reached
  it.** Two independent defects, both silent. The hook scraped the command out
  of the tool-input JSON with `sed` on `"[^"]*"`, so any command containing an
  escaped quote (`php -r "echo 1;" && git push --force`) was truncated past the
  destructive half and passed. Patterns were then handed to `grep -Eqi
  "$PATTERN"` without `--`, so the leading-dash pattern `--no-verify` was parsed
  as a grep option: `git commit --no-verify` printed `grep: unrecognized option`
  to the transcript and exited 0 on every single command. Separately,
  `.claude/settings.json` wired all three PreToolUse/PostToolUse hooks as
  `echo '$TOOL_INPUT' | <script>`, which feeds the hook the literal string
  `$TOOL_INPUT` rather than the payload Claude Code already delivers on stdin,
  and used millisecond `timeout` values (`5000`/`8000`/`10000`) where the field
  is seconds. Adopted the Symfony edition's JSON-decoding extractor
  (jq -> php -> python3), `grep -Eqi --`, and stderr reporting; the wiring is now
  a bare script path with second timeouts. Verified by running both versions
  against the same payloads: `git commit --no-verify` now exits 2 instead of 0.
- **Framework-specific schema rules matched prose and read-only searches.** This
  edition assumes no framework, but carried unanchored Laravel leftovers:
  `migrate.*(:|--)?(fresh|reset|refresh|rollback)`, `db:wipe`, and `schema:drop`
  matched `grep -rn migrate:refresh docs/` and
  `git commit -m "migrate to the new refresh flow"` once the extractor started
  reporting whole commands. Consolidated them into one rule that fires only when
  a PHP console runner (`php`, `artisan`, `console`, `phinx`,
  `doctrine-migrations`) actually invokes the destructive subcommand, so
  `vendor/bin/phinx migrate:rollback` blocks while
  `cat docs/migrate:fresh-notes.md` passes.

## 1.2.1 - 2026-07-18

### Fixed

- **Stale branch-based wording left over from the pre-monorepo layout** - `AGENTS.md`, `README.md` intro, `.cursor/rules/accelerator-workflow.mdc`, `GOLDEN-PRINCIPLES.md` (all three editions), `local-context.sh` (all three editions), and the `architect`, `brainstorming`, and `coder` skills (all three editions) still said things like "framework-specific behavior lives in dedicated branches" / "switch to the matching accelerator branch", which stopped being accurate once the accelerators were merged into sibling `Laravel/` / `Symfony/` / `PHP Core/` folders in one repo. Reworded all of these to point at the sibling `Laravel/`/`Symfony/` folders instead of branches, consistent with the root `README.md`. (Legitimate git-branch mentions, like `using-git-worktrees`'s "return work to the main branch" referring to the consuming project's own Git history, were left untouched.)

## 1.2.0 - 2026-07-18

### Added

- **`memory-bank`** - ported the indexed, cross-session, source-verified project-memory system from the Symfony edition, adapted for framework-agnostic native PHP instead of copied verbatim:
  - New root `memory-bank/` store: `README.md` (contract/lifecycle), `INDEX.md`, `.memory-counter`, `chunks/MEM-0001-cross-edition-sync.md` (seed chunk documenting the Claude/Cursor/Codex mirroring convention), `templates/chunk.md`, `scripts/validate.py` (dependency-free structural validator), and `tests/test_validate.py`.
  - New `memory-bank` skill/command/agent across `.claude/`, `.cursor/`, and `.agents/skills` (Codex has no command/agent layer, per convention). The native-PHP memory categories replace Symfony's Controller -> Service -> Repository/Messenger wording with generic entry-point/service/data-access-gateway boundaries and worker/queue operational lessons, since no framework or persistence layer is assumed here.
  - Wired into `AGENTS.md` (hierarchy of sources of truth, file naming, agent behavior, and a new "Memory Bank" policy section), `SKILL FLOW.md` (utility row + shortcut + Context Handoff line), `README.md` (What This Is, directory structure, Quick Start table, and a new "Memory Bank" section), `.cursor/rules/accelerator-workflow.mdc`, and each edition's `README.md` (skill count bumped to 31).
  - `local-context.sh` (all three editions) now reports memory-bank chunk counts at session start via `scripts/validate.py --summary`, and lists `memory-bank/` in the project structure scan. Also corrected the Codex hook's structure-scan marker from a bare `.codex` to `.agents .codex`.
  - `.gitignore` updated to ignore `memory-bank/local/`, `.idea/`, and Python tooling caches, matching the Symfony edition.
- All 14 validator tests (including the cross-edition hook-integration test) pass against the new store.

## 1.1.0 - 2026-07-09

### Added

- **Cursor edition** - full self-contained `.cursor/` mirror so Cursor users get the accelerator without enabling the opt-in "read `.claude`" setting: 30 skills (`.cursor/skills/`), 28 agents (`.cursor/agents/`), 27 commands converted to Cursor `name`/`description` frontmatter (`.cursor/commands/`), `hooks.json` + scripts with events translated to Cursor's model (`sessionStart`, `beforeShellExecution`, `afterFileEdit`), three always-on/PHP-scoped rules (`.cursor/rules/*.mdc`), DOD/principles/stabilization docs, and a README documenting the double-loading caveat.
- **Codex edition** - Codex-idiomatic layout: 30 skills in `.agents/skills/` (the path Codex discovers), plus `.codex/` holding `config.toml` (enables hooks, MCP placeholder), `hooks.json` + scripts using Codex's Claude-compatible event schema (`SessionStart`, `PreToolUse`, `PostToolUse`), DOD/principles/stabilization docs, and READMEs (`.codex/README.md`, `.agents/README.md`) explaining Codex's model (custom prompts deprecated -> skills; trust requirement).
- **Best-practice depth across skills** - compact, high-signal sections added to:
  - `coder`: modern PHP 8.x idioms, typed exception hierarchy/error handling, PSR-3 logging, configuration, PSR-15 middleware.
  - `test-generator`: AAA, test-double taxonomy, data providers, determinism, coverage targets, mutation testing.
  - `api-designer`: versioning & deprecation, idempotency keys, content negotiation, cursor vs offset pagination.
  - `database-designer`: transaction isolation/deadlocks/locking, expand-contract zero-downtime migrations, soft delete/auditing, correct data types.
  - `architect`: when-not-to-add-a-layer (YAGNI), ports & adapters, concurrency/idempotency, failure/resilience.
  - `code-reviewer`: review-conduct practices (severity labels, actionable feedback, scope).
  - `security-reviewer`: tooling support (`composer audit`, Psalm/PHPStan taint analysis, dangerous-sink grep, secure defaults).
  - `refactorer`: code-smell -> refactoring catalog and the strangler-fig pattern.
  - `writing-plans`: plan-quality practices (safe sequencing, incremental delivery, reversibility).
  - `frontend-design` / `coder-frontend`: frontend best practices and context-aware output escaping + asset hygiene.
  - `documentation-generator`: docs-with-code, runnable examples, Keep a Changelog + SemVer.
  - `using-git-worktrees`: branch and commit hygiene.

### Changed

- **Disambiguated overlapping skills/agents** with explicit "Scope Boundary" sections and tightened descriptions: `coder` (behavior-changing) vs `refactorer` (behavior-preserving) vs `architecture-implementer` (scaffolding); `code-reviewer` (local, broad) vs `security-reviewer` (OWASP-only) vs `review-pr` (remote GitHub PR); `web-design-guidelines` (general UX) vs `wcag-accessibility` (accessibility only).

### Fixed

- **`coder` skill examples corrected** to practice what they preach: replaced the non-existent PSR-7 `->send()` with a proper SAPI emitter; reworked the transactional use case to depend on a repository interface + transaction boundary instead of injecting raw `PDO` into the application layer; added an explicit authorization check in the controller example; flagged illustrative helper imports; and added a `UNIQUE(email)` note to close the check-then-insert race.

## 1.0.0

### Added

- Added claude agents, commands, hooks and skills
- Created a Laravel-first PHP accelerator based on the universal workflow model.
- Added PHP/Laravel onboarding, policy, Definition of Done, and manually authored backend/API/testing guidance.
- Preserved framework-neutral workflow assets, task/spec conventions, product epics, design assets, accessibility guidance, and command-agent-skill flow.
