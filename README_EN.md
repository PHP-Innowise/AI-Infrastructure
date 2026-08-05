English | [Русский](README_RU.md)

# PHP AI Accelerators

A collection of ready-to-use accelerators for AI agents working in PHP
projects, plus a generator that builds an accelerator from the actual
structure of a specific project. Each edition combines policies, commands,
agents, skills, quality checks, and documentation conventions. It does not
replace the project's code, configuration, tests, or specifications.

## What Is in This Repository

~~~text
AI-Infrastructure/
├── Laravel/                  # ready-to-use Laravel edition
├── Symfony/                  # ready-to-use Symfony edition
├── PHP Core/                 # ready-to-use native PHP edition
├── Infrastructure-Creator/   # generator for a specific project
├── scripts/                  # monorepo tooling (mirror build, repo checks)
├── docs/                     # shared documentation, including docs/CI.md
├── CHANGELOG.md              # shared-core history
└── .github/workflows/ci.yml  # CI: tests, parity, mirrors, lint, links, budget
~~~

- [Laravel/](Laravel/README.md) — a Laravel-focused edition covering
  Eloquent, queues, events, notifications, Filament, and package development.
- [Symfony/](Symfony/README.md) — a Symfony-focused edition with pragmatic
  Controller → Service → Repository boundaries, Doctrine, Messenger, API
  Platform, voters, Forms, and Symfony UX.
- [PHP Core/](PHP%20Core/README.md) — a framework-neutral foundation for
  Composer and PSR projects, PDO, and explicit application boundaries.
- [Infrastructure-Creator/](Infrastructure-Creator/README.md) — not an
  edition to copy, but a generator that inspects a target project and creates
  a suitable set of agent policies and workflows.

The first three directories are independent, ready-to-use editions.
`Infrastructure-Creator/` solves a different problem: it generates a new
edition based on the components, integrations, architecture, and CI/CD of a
given PHP project.

`scripts/`, `docs/`, the root `CHANGELOG.md`, and CI serve the monorepository
itself and are not copied into a project along with an edition. Each edition
carries its released version in a `VERSION` file (currently Laravel 1.4.3,
Symfony 1.3.1, PHP Core 1.2.1, Infrastructure-Creator 1.4.0), which the
session-start hook prints.

## Which Edition Should You Choose?

| Edition | Baseline platform | Choose it when |
| --- | --- | --- |
| [Laravel/](Laravel/README.md) | Laravel 12 / 13, PHP 8.2+ (PHP 8.3+ for Laravel 13) | The project already uses Laravel, Eloquent, Artisan, Sanctum, queues, or the Laravel ecosystem. |
| [Symfony/](Symfony/README.md) | Symfony 7.4 LTS on PHP 8.2+, or Symfony 8.1 on PHP 8.4+ | The project uses Symfony, Doctrine, Messenger, API Platform, voters, and conventional Symfony boundaries. |
| [PHP Core/](PHP%20Core/README.md) | Native PHP 8.2+ | A regular PSR project, a microframework, or a framework without a dedicated edition. |

Use the matching directory when the project already runs on Laravel or
Symfony. Use `PHP Core/` for other cases; it does not impose a particular ORM,
router, or dependency-injection container.

## How to Add an Accelerator to a Project

AI tools look for their files from the workspace root. There are two supported
ways to connect an edition:

1. Open the selected edition directory as the workspace root and keep the real
   application alongside or inside it.
2. Copy the selected directory's contents — including `.claude/`, `.cursor/`,
   `.agents/`, `.codex/`, `AGENTS.md`, and the documentation — into the root
   of the real project.

Opening this monorepository's root does not activate a nested edition by
itself. Claude Code, Cursor, and Codex do not automatically search
`Laravel/`, `Symfony/`, or `PHP Core/` for configuration.

## Shared Architecture: Command → Agent → Skill

All three editions use the same workflow model, adapted to their stack:

~~~text
User request
     ↓
Command selects an Agent
     ↓
Agent executes one Skill in an isolated context
     ↓
Result, concise context, and next steps
~~~

- A Command routes the user's intent to the appropriate workflow.
- An Agent stays thin: it executes one skill and stops, keeping decisions
  observable and under user control.
- A Skill contains the workflow itself: checks, examples, decision criteria,
  and output format.
- Hooks and policy files enforce security, naming, and verification
  conventions.

In every edition, `tasks/` contains temporary, skill-prefixed task documents,
while `specs/` contains durable living specifications registered in
`specs/MANIFEST.md`.

Claude Code and Cursor normally route a command to an agent. Codex has no
separate command layer: a skill is invoked by name or selected by Codex from
`.agents/skills/`.

## Supported AI Tools

Each edition mirrors its workflow for three tools so their files do not
conflict:

| Tool | Reads | Practical meaning |
| --- | --- | --- |
| Claude Code | `.claude/` | The source edition with agents, commands, hooks, skills, and settings. |
| Cursor | `.cursor/` | A self-contained mirror with skills, commands, agents, rules, and hooks. Disable optional Claude-file loading in Cursor to avoid loading policies twice. Cursor cannot add context to a prompt directly, so the Task Capsule arrives through the `.cursor/rules/working-memory.mdc` rule (`alwaysApply: true`): the turn-end and session-start hooks re-render it, which makes the capsule one turn stale. On a cold start the hooks render a warming-up placeholder instead of nothing. The file is local state and is listed in the edition's `.gitignore`. |
| Codex | `.agents/skills/` and `.codex/` | Skills live in `.agents/skills/`; `.codex/` contains configuration, hooks, and references. There is no separate command layer. |

The selected edition's `AGENTS.md` is executable policy for that stack. Its
README documents the full directory layout, prerequisites, command catalog,
and verification workflow.

## How the Editions Differ

The shared process stays the same, while each edition adds only capabilities
that belong to its real stack:

- Laravel adds Filament, Eloquent, jobs and queues, events and notifications,
  authentication, cache, Artisan scheduling, file storage, and
  Composer/Laravel package workflows.
- Symfony adds API Platform, Doctrine migrations, event subscribers, Forms
  and validation, authorization rules, Messenger, console commands, fixtures,
  Controller/Service/Repository boundaries, the DI container, and
  Twig/Symfony UX.
- PHP Core keeps the smallest shared foundation: architecture, APIs,
  databases, implementation, testing, quality and security review,
  performance, dependencies, debugging, and releases without assuming a
  framework.

Do not synchronize every change mechanically across editions. First verify
that the change is meaningful for the target stack.

## Project Brain, Local Context Engine, and Memory Bank

Every edition combines three separate components:

- **Project Brain — what is happening now.** It is the Git-tracked shared
  authority for active tasks and handoffs plus findings, bugs, incidents,
  decisions, and events. Records use ownership, privacy, authority, source
  fingerprints, revisions, lifecycle transitions, and explicit conflicts.
- **Local Context Engine — how relevant sources are found.** It indexes
  eligible policy, skills, project documentation, specifications, Project
  Brain records, handoffs, active Memory Bank chunks, and history in ignored
  SQLite. Retrieval uses FTS5/BM25, privacy/authority/freshness filtering,
  bounded snippets, token budgets, and retrieval manifests.
- **Memory Bank — what is remembered permanently.** It stores small,
  Git-tracked, reviewed chunks of reusable constraints, decisions, domain
  knowledge, integration contracts, and operational lessons. A new chunk is
  identified as `MEM-YYYYMMDD-xxxxxxxx` (the date plus eight hex characters
  derived from the source record's UUID), so promotions on two machines or in
  two branches never race for a shared counter; `INDEX.md` is not appended by
  hand but regenerated deterministically with `context.py reindex-bank`.
  Legacy `MEM-NNNN` chunks stay valid.

Governed mode is the default. Project Brain owns shared active-work state;
`memory-bank/local/context.db` is only a disposable index plus local
binding/cache and optional non-authoritative replay episodes. Deleting SQLite
does not delete Project Brain records, Memory Bank chunks, or canonical project
sources. Branch-derived local Working Memory exists only when
`--mode lightweight` is explicitly configured for machine-local work.

### Authority and Trust

Enforcement, policy, current specifications, code, configuration, migrations,
and tests remain canonical and outrank all retrieved context. Project Brain
coordinates current work but does not override those sources. Memory Bank is
reviewed durable knowledge, not a competing source of truth. Retrieved snippets
are discovery aids and must be verified against the cited current source.

Raw conversations, prompts, responses, hidden reasoning, logs, credentials,
secrets, customer or personal data, and unredacted incident payloads do not
belong in any of these stores. Ignored, private, unauthorized, stale,
superseded, terminal, or invalid records are excluded as applicable.

### Governed User Flow

For non-trivial work:

~~~text
prompt with a stable task ID
    → start or resume the Project Brain task
    → refresh the disposable index
    → retrieve bounded task-aware context
    → verify canonical sources
    → implement and update the revision-checked task/handoff
    → verify and complete
    → reusable knowledge is promoted automatically
      (with automatic_promotion off, promote it manually after review)
    → archive terminal records
~~~

See the complete [user task workflow example](docs/examples/USER-TASK-WORKFLOW-EXAMPLE.md)
for a step-by-step walkthrough.

The public governed retrieval command is:

~~~bash
python3 memory-bank/scripts/context.py retrieve \
  "password session invalidation" \
  --task-id BAUMAS-133
~~~

`context` remains a compatibility alias for `retrieve`; new documentation and
automation should use `retrieve`.

Alongside the explicit command, an automatic memory loop runs:

- the session-start hook prints metadata only — edition version, mode, index
  health, active binding count, and validation status; it never prints records
  or injects context. Validation results are cached per repository state, so a
  second start does not pay for them again;
- the prompt hook (`UserPromptSubmit` in Claude Code and Codex) runs
  `context.py refresh`: it incrementally refreshes the three index layers and
  injects the Task Capsule into the prompt. On Cursor the re-rendered
  `working-memory.mdc` rule serves this role (see the tool table);
- the turn-end hook (`Stop`) runs `context.py turn`: changed paths are buffered
  in ignored local state and flushed into the authoritative task every fifth
  turn. Maintenance — closing merged tasks, promotion, and compaction — runs on
  that same boundary so heavy work never happens on every turn under the hook
  timeout.

Each turn's outcome is written to `memory-bank/local/last-turn-report.json`
(what flushed, what was promoted, what was blocked and why, which paths were
excluded), and the next capsule shows a "Last turn" section — otherwise the
automatic pipeline would be invisible to the operator.

### Authority-Aware `memory` and `checkpoint`

Invoke `memory` in Codex or `/memory` in Claude/Cursor for an argument-free
refresh. In governed mode it validates Project Brain and refreshes the
disposable index without deriving a branch task, creating task authority, or
writing competing SQLite progress.

Invoke `checkpoint` in Codex or `/checkpoint` in Claude/Cursor when progress
must be captured. In governed mode it skips local Working Memory and directs
the agent to update the existing Project Brain task and handoff with the
current revision. Only explicitly configured lightweight mode may derive a
local task identifier from the Git branch and store a sanitized local Working
Memory snapshot.

Neither command completes a task, applies a promotion, or silently edits
tracked project sources.

To refresh every applicable layer in one step, invoke `memory` without
arguments (`memory` in Codex, `/memory` in Claude and Cursor). In governed
mode this validates Project Brain and refreshes the disposable index only;
it never completes a task, which still requires explicit `complete`.

### Task Capsule

At the start of a complex request and before a complex phase handoff, the agent
derives a concise sanitized retrieval query and builds a Task Capsule from
optional Working Memory and `context` retrieval. The raw request is not copied
into the packet. The complete packet is capped at 8,000 Unicode characters and
contains at most two Procedural, three Semantic, and one Episodic result.
Retrieved entries are short snippets with source paths; the next agent reads a
full source only when its current step requires it.

The retrieval query is distilled from the whole prompt rather than its opening
words: up to 24 terms are selected by rarity in the index, so a point made at
the end of a long request is not lost. Ranking modulates BM25 with the record's
declared confidence and a freshness decay over `updated_at`, so a verified
record outranks an observed one and a fresh record outranks a stale one, all
else being equal. The privacy gate screens the raw prompt before distillation:
a secret-looking query is refused outright and reaches neither retrieval nor
the retrieval manifest.

Simple tasks stay in the current context. Fresh contexts are reserved for
research-to-planning, planning-to-implementation,
implementation-to-independent-verification, and recovery after compaction.
`memory`, `checkpoint`, and explicit `complete` keep their existing roles.

### Explicit Governed CLI

Run from an edition or consuming-project root:

~~~bash
python3 memory-bank/scripts/context.py start \
  --task-id BAUMAS-133 \
  --goal "Verify invalidation of other sessions after a password change"
python3 memory-bank/scripts/context.py index
python3 memory-bank/scripts/context.py retrieve \
  "password sessions" \
  --task-id BAUMAS-133
python3 memory-bank/scripts/context.py update \
  --task-id BAUMAS-133 \
  --revision 1 \
  --progress "The two-session regression test passes" \
  --next-step "Verify the old remember-me cookie"
python3 memory-bank/scripts/context.py complete \
  --task-id BAUMAS-133 \
  --revision 2 \
  --outcome "Other sessions and stale remember-me cookies are invalidated" \
  --verification "ChangePasswordTest passed"
python3 memory-bank/scripts/context.py compact
~~~

Companion maintenance commands:

~~~bash
# restore the machine-local binding on another machine or in a fresh clone
# (the task record exists in Git, the local binding does not)
python3 memory-bank/scripts/context.py rebind --task-id BAUMAS-133

# regenerate memory-bank/INDEX.md deterministically from chunk frontmatter
python3 memory-bank/scripts/context.py reindex-bank

# check skill, hook, command, and agent mirrors inside the edition; add
# --cross-edition to check the shared core is byte-identical across editions
python3 memory-bank/scripts/context.py parity
python3 memory-bank/scripts/context.py parity --cross-edition
~~~

Automatic rebinding is built into the turn flush: when a task record exists in
Git but its local binding does not, the binding is restored by branch name and
the restoration is stated in the turn report — previously that situation
silently lost every automatic flush on a second machine.

To hand accumulated context to another person or to a team repository, write a
bundle:

~~~bash
python3 memory-bank/scripts/context.py export --destination ../context-bundle
~~~

Export is read-only. The bundle carries Project Brain records whose privacy is
allowed (`restricted` and `private` never leave) and active Memory Bank chunks.
`MANIFEST.json` lists what was included **and what was excluded with the
reason**, flags chunks tagged `auto-promoted`, and records the source commit. A
secret-pattern match aborts the whole export before any file is written. There
is no `import`: a bundle is a handoff artifact, not a second installation.

Governed cross-store mutations use revision checks and rollback/compensation so
a failed Brain or SQLite update does not leave a split authoritative state.
Promotion into Memory Bank is automatic by default: on the turn-flush boundary
resolved findings and bugs, closed incidents, and accepted decisions are
promoted without asking, and a review that did not happen is never claimed —
`reviewer` stays null, `review_mode` is `automatic`, and the chunk is tagged
`auto-promoted`. Only records whose authority is `verified` qualify. Records
are created as `observed`, and the single legal way to raise one is
`brain-update --authority verified` under a revision check, recorded in the
record's transition ledger; the `verify` skill does this for confirmed records
before their terminal status. A blocked promotion and its reason appear in the
turn report and in the next capsule. Durable
memory is therefore accumulated, not curated: treat a retrieved chunk as a
pointer to its cited source, not as a vetted fact. Set `automatic_promotion` to
`false` in `project-brain/config/runtime.json` for the reviewed sequence, where
an agent proposes source- and revision-bound knowledge and an independent human
approves it before atomic application. Lightweight mode retains the older local
`start → update → retrieve → complete` lifecycle and local episodes, all of
which are non-authoritative and lost when its SQLite database is deleted.

The implementation is local, dependency-free Python with SQLite FTS5. It does
not provide embeddings, vector search, MCP, LangGraph, a central memory
service, or automatic per-request context injection.

A three-scenario Task Capsule pressure test measured 96.4%, 97.2%, and 97.2%
fewer transferred characters while retaining each exact Working file set and
scenario-specific authoritative source. The original checkout stayed
unchanged. The capsule criteria passed, but the exact DDEV PHPUnit commands
were non-zero because of an invalid PHPUnit 10 configuration warning and, for
Branding, incompatible persisted MFA ciphertext; see the
[complete evidence report](docs/superpowers/reports/2026-07-29-task-capsule-bauherrenmappe.md).

## Infrastructure Creator

`Infrastructure-Creator/` generates an accelerator for a specific target
project; it does not replace the ready-to-use Laravel, Symfony, or PHP Core
editions. Keep it outside the target project, either in a separate workspace
or a sibling directory.

The workflow is `infra-scan → review → infra-generate`. `infra-scan` only
reads the target project and writes nothing to it; it creates a project
profile for review. During review, you can correct its conclusions.
`infra-generate` writes an accelerator only for the selected AI tools and asks
for confirmation before overwriting anything. When a separate review is not
needed, `infra-build` runs analysis and generation as one flow, stopping on
ambiguity or conflict.

The generator inspects the actual `composer.json`, framework, dependencies,
integrations, architecture, and CI/CD instead of copying a Laravel, Symfony,
or PHP Core template. A generated accelerator receives the same memory layer as
the ready-to-use editions: the context-brain runtime under
`memory-bank/scripts/`, the `project-brain/` skeleton, and the automatic-memory
hooks, including capsule delivery for Cursor.

Generated output is upgradable in place. Every `infra-generate` run writes
`.infra-manifest.json` into the target (generator version from the root
`VERSION`, the profile it consumed, and the sha256 of every generated file) and
stamps the version on the first line of the generated `AGENTS.md`. Later,
`infra-update` compares hashes: a file the team never touched is replaced with
its new version; a file the team edited is never overwritten and lands in a
"requires decision" report with three-way context; a file absent from the
manifest is left alone entirely. See
[Infrastructure-Creator/README.md](Infrastructure-Creator/README.md) for the
complete guide.

## Contributing

- Change skills only in editions where they actually apply. A Laravel fix
  does not automatically belong in Symfony or PHP Core.
- Evaluate a universal policy in `PHP Core/` first, then adapt it to the
  relevant framework boundaries instead of copying it blindly.
- Never edit a mirror by hand. Skills are canonical in `.agents/skills`; hooks,
  commands, agents, and policy documents are canonical in `.claude`. After
  editing the canon, run `python3 scripts/build_mirrors.py --write` and the
  `.claude`, `.cursor`, and `.codex` mirrors are regenerated from the
  `MIRROR_RULES` table in `memory-bank/scripts/context_retrieval.py`
  (`Infrastructure-Creator/mirror_rules.py` for the generator). A legitimate
  per-mirror difference belongs in that table as a rule or an exception, never
  as a silently diverging file.
- Record the change in the edition's `CHANGELOG.md` and run its `DOD.md`
  verification. Changes to the shared core (the memory/context core,
  Project Brain, hooks, mirror machinery, root `scripts/`) are recorded in
  the root `CHANGELOG.md` instead; CI enforces this on pull requests.

### Checks Before Handing Work Over

CI (`.github/workflows/ci.yml`) runs the same steps; their exact local
equivalents are collected in [docs/CI.md](docs/CI.md):

~~~bash
# each edition's tests (memory-bank/tests and project-brain/tests) and the generator's
python3 -m unittest discover        # from each test directory

# mirror parity inside an edition, and shared-core parity across editions
python3 memory-bank/scripts/context.py parity
python3 memory-bank/scripts/context.py parity --cross-edition

# mirrors match the canon; relative markdown links resolve; the startup
# context has not grown past its recorded ceilings
python3 scripts/build_mirrors.py --check
python3 scripts/check_links.py
python3 scripts/context_budget.py --check
~~~

The startup context budget (each edition's `AGENTS.md` plus skill frontmatter)
is measured by `scripts/context_budget.py`; the ceilings live in
`scripts/token_budget.json` and sit five percent above current values, so CI
catches a regression instead of complaining about every edit.

For stack-specific details, open the selected edition's README. For its
durable memory, open the corresponding guide and then that edition's
`INDEX.md`:
[Laravel memory](Laravel/memory-bank/README.md),
[PHP Core memory](PHP%20Core/memory-bank/README.md), or
[Symfony memory](Symfony/memory-bank/README.md).
