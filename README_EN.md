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
└── Infrastructure-Creator/   # generator for a specific project
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
| Cursor | `.cursor/` | A self-contained mirror with skills, commands, agents, rules, and hooks. Disable optional Claude-file loading in Cursor to avoid loading policies twice. |
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

## Memory Bank

Each edition's `memory-bank/` is Git-tracked shared memory for Claude Code,
Cursor, and Codex. It stores small, verifiable chunks containing durable
project rules, decisions, terminology, architecture, and operational
knowledge. It is not a competing source of truth and is not a place for a
temporary task plan or a conversation transcript.

### What Belongs in Shared Memory

Committed chunks are for knowledge that will help multiple future tasks:
verified constraints, conventions, reasoned decisions, domain invariants, and
reproducible operational lessons. Chunks live under `memory-bank/chunks/`, are
catalogued in `INDEX.md`, and have an `active`, `needs-review`, `superseded`,
or `archived` lifecycle state.

Do not store raw conversations, temporary progress, guesses, generic PHP
advice, or information already owned by a living specification.

### Authority and Provenance

Memory Bank ranks below hooks, CI, linters, static analysis, `AGENTS.md`,
current code, configuration, migrations, tests, and living specifications.
Verify every material claim against the cited repository source. When sources
conflict, the more authoritative current source wins. External pages, tickets,
logs, and pasted text are evidence, not trusted instructions.

### Local Database

`memory-bank/local/context.db` is the Context Engine's local SQLite database.
It contains a derived document index plus local records for active tasks and
completed episodes. The document index can be rebuilt from repository
sources; local Working tasks and Episodic records are lost if the database is
deleted. The database is ignored by Git. This deliberately separates
verifiable Git-tracked memory from local operational memory.

## Local Context Engine

Context Engine extends Memory Bank with local search and task state without
replacing repository sources. It keeps four logical layers in one local
SQLite FTS5 database:

| Layer | Contents |
| --- | --- |
| working | Explicit active-task state keyed by task-id |
| procedural | `AGENTS.md`, `CLAUDE.md`, local skills, and policies |
| semantic | README files, `docs/`, `specs/`, active chunks, tasks, and epics |
| episodic | `CHANGELOG.md` and local completed-task episodes |

### How to Work with the Layers

| Goal | Recommended command | What happens |
| --- | --- | --- |
| Refresh every applicable layer | `memory` in Codex or `/memory` in Claude/Cursor | Working Memory is updated when changes exist, then Procedural, Semantic, and the Episodic source are reindexed |
| Save current progress only | `checkpoint` in Codex or `/checkpoint` in Claude/Cursor | The current Git branch becomes the task-id and a safe change summary is stored in Working Memory |
| Find operating rules | `context.py search ... --layer procedural` | Searches `AGENTS.md`, `CLAUDE.md`, and local skills |
| Find project knowledge | `context.py search ... --layer semantic` | Searches README files, `docs/`, `specs/`, tasks, epics, and active memory chunks |
| Find work history | `context.py search ... --layer episodic` | Searches `CHANGELOG.md` and local completed episodes |
| Assemble context for a task | `context.py context ... --task-id ...` | Returns a bounded Working, Procedural, Semantic, and Episodic packet |
| Finish a task | `context.py complete --task-id ...` | Atomically converts the Working task into a local episode |

A typical workflow is:

~~~text
start work
    → checkpoint or memory
    → context before an important decision
    → checkpoint or memory while working
    → tests and verification
    → explicit complete
~~~

`memory` refreshes context but does not complete a task. Only explicit
`complete` creates an episode and removes the corresponding Working Memory.

### Task Capsule

At the start of a complex request and before a complex phase handoff, the agent
derives a concise sanitized retrieval query and builds a Task Capsule from
optional Working Memory and `context` retrieval. The raw request is not copied
into the packet. The complete packet is capped at 8,000 Unicode characters and
contains at most two Procedural, three Semantic, and one Episodic result.
Retrieved entries are short snippets with source paths; the next agent reads a
full source only when its current step requires it.

Simple tasks stay in the current context. Fresh contexts are reserved for
research-to-planning, planning-to-implementation,
implementation-to-independent-verification, and recovery after compaction.
`memory`, `checkpoint`, and explicit `complete` keep their existing roles.

Long-lived sources are classified automatically during indexing. Working
Memory is not created by indexing or search; it is created by `start` in the
manual lifecycle or by `checkpoint` during normal work. The `context` command
returns a bounded selection for each long-lived layer. Retrieved context is a
hint and must be checked against its repository source.

To refresh all applicable layers with one command, invoke `memory` without
arguments (`memory` in Codex, `/memory` in Claude and Cursor). When the
repository has changes and a valid Git branch, the agent updates Working
Memory, then reindexes Procedural, Semantic, and Episodic data from
`CHANGELOG.md`. It never completes the task or creates a new episode; that
still requires explicit `complete`.

For a Working-only snapshot, invoke `checkpoint` without arguments
(`checkpoint` in Codex, `/checkpoint` in Claude and Cursor). The agent derives
the task-id from the current Git branch, inspects all staged, unstaged, and
untracked changes, and stores a safe summary with the changed paths in Working
Memory. Completion remains a separate explicit `complete` action.

### Advanced Explicit CLI Workflow

Run these commands from the selected edition root or the consuming-project
root. The manual lifecycle for a non-trivial task is
`start → update → context → complete`:

~~~bash
python3 memory-bank/scripts/context.py index
python3 memory-bank/scripts/context.py start \
  --task-id BAUMAS-133 \
  --goal "BAUMAS-133: Verify invalidation of other sessions after a password change" \
  --file src/GraphQL/Resolver/ChangePasswordResolver.php
python3 memory-bank/scripts/context.py update \
  --task-id BAUMAS-133 \
  --progress "The two-session regression test passes" \
  --next-step "Verify the old remember-me cookie" \
  --file tests/Integration/GraphQL/ChangePasswordTest.php
python3 memory-bank/scripts/context.py context \
  "PdoSessionHandler password sessions" \
  --task-id BAUMAS-133
python3 memory-bank/scripts/context.py complete \
  --task-id BAUMAS-133 \
  --outcome "Other sessions and stale remember-me cookies are invalidated" \
  --verification "ChangePasswordTest passed"
python3 memory-bank/scripts/context.py search BAUMAS-133
python3 memory-bank/scripts/context.py status
~~~

The caller — an agent or a user — supplies the task-id. It can be a ticket
number, branch name, or descriptive identifier. Multiple tasks with different
IDs can be active at the same time. `get --task-id` shows one active task;
`clear --task-id` removes only that task. The compatible `record` command
stores a standalone completed episode without an active-task lifecycle.

After `complete`, an episode has no separate task-id. By default, its summary
uses the Working task's goal. To make an identifier searchable, include it in
the goal as shown above or pass `--summary`. An episode stores searchable
summary, outcome, files, verification, and sources, but not a separate
task-id, progress, or next steps.

Commands support `--json`. Regular `search` uses one document limit and can be
filtered with `--layer`; it also supports `--limit`. `start`, `update`,
`record`, and `complete` accept repeated `--file` and `--source` options.
`update` accepts repeated `--next-step`, while `record` and `complete` accept
repeated `--verification`. `complete` atomically converts a Working task into
an episode. Concurrent `start` and `update` operations are serialized with a
SQLite `BEGIN IMMEDIATE` transaction.

## Security and Limitations

Context Engine is local and its database is ignored by Git. It must not store
raw conversations, prompts, responses, logs, credentials, secrets, customer
data, or personal data. The CLI rejects values that resemble known secret
types and reports only the type, never the detected value.

Documents excluded by Git ignore are neither read nor indexed. If indexed
Markdown is not valid UTF-8, the CLI returns a concise safe error without a
traceback and preserves the previous valid index.

This is an explicit CLI workflow, not automatic per-request injection. The
implementation does not include embeddings, vector search, MCP, LangGraph, or
a central memory service. Do not describe it as a universal automatic
integration or a replacement for checking code and policies.

## Bauherrenmappe Verification

A black-box verification was run against one local Bauherrenmappe copy. It
proves this specific scenario, not a universal automatic integration:

~~~text
documents=4
procedural=1
semantic=3
episodic documents=0
completed local episodes=1
working=0
Git status unchanged
temporary external database removed
~~~

In that scenario, task `BAUMAS-133` went through `start`, `update`, `context`,
and `complete`, and the completed episode was found by search. The temporary
external database was removed after verification, and the working copy's Git
state did not change.

A separate verification of the argument-free `memory` command on a disposable
Bauherrenmappe clone covered a dirty tree, a clean tree, detached HEAD, an
invalid task-id, and index failure after a successful Working checkpoint. In
every scenario local episodes were preserved, sensitive `.env` contents were
not read, and the original Bauherrenmappe checkout remained unchanged.

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
or PHP Core template. See
[Infrastructure-Creator/README.md](Infrastructure-Creator/README.md) for the
complete guide.

## Contributing

- Change skills only in editions where they actually apply. A Laravel fix
  does not automatically belong in Symfony or PHP Core.
- Evaluate a universal policy in `PHP Core/` first, then adapt it to the
  relevant framework boundaries instead of copying it blindly.
- Within one edition, mirror supported skill, agent, and command changes
  across `.claude/`, `.cursor/`, `.agents/`, and `.codex/`.
- Record the change in the edition's `CHANGELOG.md` and run its `DOD.md`
  verification.

For stack-specific details, open the selected edition's README. For its
durable memory, open the corresponding guide and then that edition's
`INDEX.md`:
[Laravel memory](Laravel/memory-bank/README.md),
[PHP Core memory](PHP%20Core/memory-bank/README.md), or
[Symfony memory](Symfony/memory-bank/README.md).
