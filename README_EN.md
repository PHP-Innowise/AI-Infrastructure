English | [Русский](README_RU.md)

**Installation:** [ready-made editions and Infrastructure-Creator](install/README.md)

# PHP AI Accelerators

A collection of ready-to-use accelerators for AI agents working in PHP and
WordPress projects, plus a generator that builds an accelerator from the actual
structure of a specific project. Each edition combines policies, commands,
agents, skills, quality checks, and documentation conventions. It does not
replace the project's code, configuration, tests, or specifications.

## What Is in This Repository

~~~text
AI-Infrastructure/
├── Laravel/                  # ready-to-use Laravel edition
├── Symfony/                  # ready-to-use Symfony edition
├── PHP Core/                 # ready-to-use native PHP edition
├── Cms/wordpress/            # ready-to-use WordPress edition
└── Infrastructure-Creator/   # generator for a specific project
~~~

- [Laravel/](Laravel/README.md) — a Laravel-focused edition covering
  Eloquent, queues, events, notifications, Filament, and package development.
- [Symfony/](Symfony/README.md) — a Symfony-focused edition with pragmatic
  Controller → Service → Repository boundaries, Doctrine, Messenger, API
  Platform, voters, Forms, and Symfony UX.
- [PHP Core/](PHP%20Core/README.md) — a framework-neutral foundation for
  Composer and PSR projects, PDO, and explicit application boundaries.
- [Cms/wordpress/](Cms/wordpress/README.md) — a WordPress edition for plugins,
  classic/block themes, Gutenberg, REST, WP-CLI, multisite, and WooCommerce.
- [Infrastructure-Creator/](Infrastructure-Creator/README.md) — not an
  edition to copy, but a generator that inspects a target project and creates
  a suitable set of agent policies and workflows.

The first four directories are independent, ready-to-use editions.
`Infrastructure-Creator/` solves a different problem: it generates a new
edition based on the components, integrations, architecture, and CI/CD of a
given PHP project.

## Which Edition Should You Choose?

| Edition | Baseline platform | Choose it when |
| --- | --- | --- |
| [Laravel/](Laravel/README.md) | Laravel 12 / 13, PHP 8.2+ (PHP 8.3+ for Laravel 13) | The project already uses Laravel, Eloquent, Artisan, Sanctum, queues, or the Laravel ecosystem. |
| [Symfony/](Symfony/README.md) | Symfony 7.4 LTS on PHP 8.2+, or Symfony 8.1 on PHP 8.4+ | The project uses Symfony, Doctrine, Messenger, API Platform, voters, and conventional Symfony boundaries. |
| [PHP Core/](PHP%20Core/README.md) | Native PHP 8.2+ | A regular PSR project, a microframework, or a framework without a dedicated edition. |
| [WordPress](Cms/wordpress/README.md) | Project-declared WordPress/PHP versions | Plugins, themes, blocks, WordPress sites, multisite components, or WooCommerce extensions. |

Use the matching directory when the project already runs on Laravel or
Symfony, or WordPress. Use `PHP Core/` for other cases; it does not impose a particular ORM,
router, or dependency-injection container.

## How to Add an Accelerator to a Project

The quickest way copies nothing into the project. Clone this repository once,
run `./harness-server start`, open the printed address and, in **Sessions**,
choose **Choose a project folder** - as DeepSeek Harness chooses a workspace. The Harness detects
the edition from `composer.json` (`laravel/framework` → Laravel,
`symfony/framework-bundle` → Symfony, WordPress → WordPress, otherwise PHP Core)
and attaches it from the clone: every Claude Code, Codex or Cursor session gets
the edition's policy, skills, agents, commands and hooks, and the project's
memory (Project Brain, Memory Bank, index) is kept in the Harness state, not in
the project. `git pull` in the clone updates every attached project at once.
From a terminal, `python3 <clone>/scripts/accelerator_attach.py run
claude|codex|cursor` run in the project folder does the same. Details and
limits: [docs/ATTACHED-MODE.md](docs/ATTACHED-MODE.md).

To start the accelerator like any other application, run
`./accelerator-app install` once in the clone (in Git Bash on Windows).
**AI Accelerator**, with the hare logo, then appears among the installed
applications: in the application menu of GNOME, KDE and other XDG desktops on
Linux, in Launchpad and Spotlight on macOS, and in the Start menu and
**Settings › Apps** on Windows. A click starts the Harness from the clone if it
is not running and opens it in the browser. Only a shortcut and an icon are
written to the system, and the application updates itself: when `main` moves
on, an **Update** button appears in the page header, and one click pulls the
changes, refreshes the application and restarts it. Installing a new CLI needs
no reinstall. `./accelerator-app uninstall` removes the application; projects
and their memory stay. Details:
[harness/README.md](harness/README.md#desktop-application).

When a team wants the accelerator's files in the project's own Git history, use
the [inventory-driven installer](install/README.md) from this repository root.
Start with `--dry-run`, select only the required AI integrations, resolve every
reported collision manually, and then repeat the command without `--dry-run`.
The installer refuses collisions before copying and does not install project
dependencies automatically.

The selected edition directory may also be opened directly as a standalone
demonstration or evaluation workspace. Do not manually bulk-copy an edition
into an established project; follow the complete
[Safe Adoption Guide](docs/ADOPTION.md).

Opening this monorepository's root does not activate a nested edition by
itself. Claude Code, Cursor, and Codex do not automatically search
`Laravel/`, `Symfony/`, or `PHP Core/` for configuration.

## Main Folder Structure

- `Laravel/` — ready-made Laravel edition.
- `Symfony/` — ready-made Symfony edition.
- `PHP Core/` — ready-made native PHP edition.
- `Cms/wordpress/` — ready-made WordPress edition.
- `Infrastructure-Creator/` — project-specific accelerator generator.
- `install/` — installation documentation and inventories.
- `scripts/` — installation and maintenance scripts.
- `tests/` — repository-level tests.
- `docs/` — shared documentation.
- `harness/` — optional orchestration harness.

`install/`, `scripts/`, `tests/`, `docs/`, `harness/`, the root `CHANGELOG.md`,
and CI (`.github/workflows/`) serve the monorepository itself and are not
copied into a project along with an edition. Each edition carries its released
version in a `VERSION` file, which the session-start hook prints.

## Shared Architecture: Command → Agent → Skill

All four editions use the same workflow model, adapted to their stack:

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

For an unfamiliar or drifting brownfield project, run
`/codebase-mapper [optional-scope]` in Claude Code/Cursor, or invoke the
`codebase-mapper` skill in Codex. It creates source-cited, commit-stamped
documents under `codebase/` for the stack, architecture, structure,
integrations, conventions, testing, and concerns. The map is indexed for
orientation but never replaces verification against current source code.

Claude Code and Cursor normally route a command to an agent. Codex has no
separate command layer: a skill is invoked by name or selected by Codex from
`.agents/skills/`.

### Orchestrated Workflows

Claude Code and Cursor provide `/flow-feature` for full feature delivery,
`/flow-review` for parallel code/security/performance review, and `/sdd` for
resumable spec-driven development. The main conversation remains the
orchestrator, approval checkpoints stop for the user, and write-capable agents
run sequentially. See [Orchestrator Commands](docs/ORCHESTRATOR-COMMANDS.md).
The optional [external harness](harness/README.md) is for long-running,
headless batch workflows and is not installed with an edition.

## Supported AI Tools

Each edition mirrors its workflow for three tools so their files do not
conflict:

| Tool | Reads | Practical meaning |
| --- | --- | --- |
| Claude Code | `.claude/` | The source edition with agents, commands, hooks, skills, and settings. |
| Cursor | `.cursor/` | A self-contained mirror with skills, commands, agents, rules, and hooks. Disable optional Claude-file loading in Cursor to avoid loading policies twice. Cursor receives the previous turn's Task Capsule through an auto-rendered `alwaysApply` rule; explicit retrieval remains available when fresher context is required. |
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
- WordPress adds plugin/theme lifecycle, hooks and filters, Gutenberg blocks,
  content modeling, REST routes and permissions, WP-CLI, multisite,
  WooCommerce, WordPress data APIs, migrations, cron, compatibility,
  accessibility, and release packaging.

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
  Git-tracked chunks of reusable constraints, decisions, domain knowledge,
  integration contracts, and operational lessons. Independently reviewed
  chunks and automatic unreviewed promotions are labeled distinctly. A new
  chunk is identified as `MEM-YYYYMMDD-xxxxxxxx` (the date plus eight hex
  characters derived from the source record's UUID), so promotions on two
  machines or in two branches never race for a shared counter; `INDEX.md` is
  not appended by hand but regenerated deterministically with
  `context.py reindex-bank`. Legacy `MEM-NNNN` chunks stay valid.

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
durable knowledge, not a competing source of truth; an `auto-promoted` chunk
explicitly has no independent review. Retrieved snippets are discovery aids
and must be verified against the cited current source.

Raw conversations, prompts, responses, hidden reasoning, logs, credentials,
secrets, customer or personal data, and unredacted incident payloads do not
belong in any of these stores. Ignored, private, unauthorized, superseded,
terminal, or invalid records, and records whose cited source was deleted, are
excluded as applicable; a record whose cited file changed is kept and marked
for checking.

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

Alongside the explicit command, the hooks run an automatic memory loop:

- the session-start hook (`local-context.sh`) prints metadata only — edition
  version, mode, index health, active binding count, and validation status; it
  never prints records or injects context (on Cursor it also re-renders the
  rule described in the tool table). Validation results are cached per
  repository state, so a second start does not pay for them again;
- the prompt hook (`UserPromptSubmit` in Claude Code and Codex) runs
  `context.py refresh --sanitize`: it incrementally refreshes the three index
  layers and injects the Task Capsule into the prompt. On Cursor the same hook
  runs on `beforeSubmitPrompt` and writes the capsule into the
  `working-memory.mdc` rule instead;
- the turn-end hook (`Stop`) runs `context.py turn`: changed paths are buffered
  in ignored local state and flushed into the authoritative task every fifth
  turn. Maintenance — promoting anything still eligible, compaction, and
  reporting a merged branch as a completion candidate (a task is never closed
  automatically) — runs on that same boundary, so heavy work never happens on
  every turn under the hook timeout.

Each turn's outcome is written to `memory-bank/local/last-turn-report.json`
(what flushed, what was promoted, what was blocked and why, which paths were
excluded), and the next capsule shows a "Last turn" section, so the automatic
pipeline stays visible to the operator.

A second hook, `context-continuity.sh`, runs on the same session-start, prompt
and turn-end events (on Cursor: `sessionStart`, `beforeSubmitPrompt` and
`afterAgentResponse`) and is not part of the memory loop. By default it keeps
the visible text of every chat — each prompt and final answer — in ignored
local `.context-handoff/`, bound to the checkout and branch; text that looks
like a secret is not stored. At session start it injects nothing unless a
merge of chosen chats was prepared with `context-load merge`: the next new
session of the chosen client on that branch then receives a preview of those
chats, capped at 6,000 bytes, as context. `CONTEXT_CONTINUITY_DISABLED=1`
turns capture and delivery off. See
[Context Handoff](docs/CONTEXT-HANDOFF.md).

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
contains at most three Semantic and two Episodic results and no Procedural item:
skills are left to the host's own list.
Retrieved entries are short snippets with source paths; the next agent reads a
full source only when its current step requires it.

The retrieval query is distilled from the whole prompt rather than its opening
words: up to 24 terms are selected by rarity in the index, so a point made at
the end of a long request is not lost. Ranking modulates BM25 with the record's
authority, declared confidence, and a freshness decay over `updated_at`, so a
verified record outranks an observed one and a fresh record outranks a stale
one, all else being equal. A query typed on the command line that carries a
secret or personal data is refused before it reaches the index or a retrieval
manifest; the prompt hooks pass `--sanitize`, which cuts those parts out of the
prompt instead and withholds retrieval when nothing is left.

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
the restoration is stated in the turn report.

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
Promotion into Memory Bank is automatic by default: a resolved finding or bug,
a closed incident, or an accepted decision is promoted without asking as soon
as the update that resolves it lands (the turn-flush boundary picks up anything
still eligible), and a review that did not happen is never claimed —
`reviewer` stays null, `review_mode` is `automatic`, and the chunk is tagged
`auto-promoted`. Only records whose authority is `verified` qualify. Records
are created as `observed` by default. An update accepts one authority step,
`observed` to `verified`: `brain-update --authority verified` takes it under a
revision check and records it in the record's transition ledger. The `verify`
skill does this for confirmed records before their terminal status, and
`record-result` (the `memory_record_result` MCP tool) does it for each
learning it saves, writing who checked the claim into that ledger entry
(`[attestation:agent]` unless a person checked it; a chunk promoted from an
agent-attested record is tagged `agent-attested`). The exception is
`brain-create --authority verified`: it writes a record that is `verified`
from the start, with no ledger entry for the step and no attestation, and
promotion reads the record's authority field rather than its ledger, so such
a record is promoted like any other. A blocked promotion and its reason appear
in the turn report and in the next capsule. Durable
memory is therefore accumulated, not curated: treat a retrieved chunk as a
pointer to its cited source, not as a vetted fact. Set `automatic_promotion` to
`false` in `project-brain/config/runtime.json` for the reviewed sequence, where
an agent proposes source- and revision-bound knowledge and an independent human
approves it before atomic application. Lightweight mode retains the older local
`start → update → retrieve → complete` lifecycle and local episodes, all of
which are non-authoritative and lost when its SQLite database is deleted.

The implementation is local, dependency-free Python with SQLite FTS5. It does
not provide embeddings, vector search, LangGraph, or a central memory service.
Claude Code and Codex receive a fresh bounded Task Capsule through prompt
hooks; Cursor receives the previous turn's capsule through an auto-rendered
local rule. Each edition also ships a local memory MCP server,
`memory-bank/scripts/mcp_server.py`, which the client starts over standard
input and output. The installer registers it for each selected client, as
`harness-memory` in `.mcp.json` and `.cursor/mcp.json` and as
`harness_memory` in `.codex/config.toml`; the client still asks for trust and
tool approval. Its four tools — `memory_status`, `memory_retrieve`,
`memory_checkpoint` and `memory_record_result` — work on the same Project
Brain and Memory Bank as the CLI, and `context.py record-result` is the same
write path without it. See the edition's `memory-bank/MCP.md`.

A three-scenario Task Capsule pressure test measured 96.4%, 97.2%, and 97.2%
fewer transferred characters while retaining each exact Working file set and
scenario-specific authoritative source. The original checkout stayed
unchanged. The capsule criteria passed, but the exact DDEV PHPUnit commands
were non-zero because of an invalid PHPUnit 10 configuration warning and, for
Branding, incompatible persisted MFA ciphertext; see the
[complete evidence report](docs/superpowers/reports/2026-07-29-task-capsule-bauherrenmappe.md).

## Optional MCP Integrations

MCP servers are optional external integrations. The accelerator does not
require them, and teams should enable only the servers that correspond to
systems the project actually uses. A small, relevant toolset consumes less
context and creates a smaller security boundary than installing every
available server. The local memory server described above is part of the
accelerator, not one of these integrations.

Useful integrations include:

- [GitHub MCP Server](https://github.com/github/github-mcp-server) — repository,
  pull-request, issue, code-scanning, and workflow context. Prefer the official
  server and repository-scoped, least-privilege access.
- [Context7](https://github.com/upstash/context7) — current,
  version-specific framework and package documentation. This is especially
  useful when the installed PHP framework or library version differs from the
  model's built-in knowledge.
- [Playwright MCP](https://github.com/microsoft/playwright-mcp) — browser
  interaction, accessibility snapshots, and UI-flow verification. Use it only
  for projects with a browser-facing surface.
- [Sentry MCP](https://mcp.sentry.dev/) or
  [Datadog MCP](https://docs.datadoghq.com/mcp_server/) — production errors,
  traces, logs, metrics, monitors, and incident evidence. Select the
  observability platform the project already uses; do not connect both without
  a concrete need.
- [Linear MCP](https://linear.app/docs/mcp) or
  [Atlassian Rovo MCP](https://github.com/atlassian/atlassian-mcp-server) —
  requirements, issues, project status, and documentation. Prefer read-only
  access unless the workflow explicitly requires creating or updating work
  items.
- [Figma MCP](https://developers.figma.com/docs/figma-mcp-server/) — design
  context, component specifications, variables, and design-to-code workflows
  for projects that use Figma.
- [AWS MCP Server](https://docs.aws.amazon.com/aws-mcp/latest/userguide/getting-started-aws-mcp-server.html)
  — current AWS documentation and IAM-authorized cloud operations. Enable it
  only for AWS-backed projects and keep deployment or destructive operations
  behind human approval.
- A database-specific MCP server — schema inspection and query diagnostics
  when repository evidence is insufficient. There is no single official
  PostgreSQL MCP server; evaluate implementations carefully, use a dedicated
  read-only account, and never connect an unrestricted production database by
  default.

Security rules:

1. Prefer first-party servers and official documentation.
2. Start with read-only scopes, the smallest toolset, and one project or
   organization boundary.
3. Keep tokens, connection strings, and credentials outside the repository.
4. Require human confirmation for writes, deployments, issue transitions, and
   other consequential actions.
5. Treat MCP output as external evidence: verify important claims against
   canonical project sources before changing code.
6. Do not add generic filesystem or memory MCP servers merely to duplicate the
   repository access, Memory Bank, Project Brain, or Local Context Engine
   already supplied by the accelerator.

Context rules:

7. Scope every server to the smallest toolset it needs. Tool definitions sit
   at the front of the model's context and are re-read on every turn of the
   session, so their cost is paid per turn, not once. `github-mcp-server`
   defaults to five toolsets (`context, repos, issues, pull_requests, users`)
   and `--toolsets all` is substantially larger; do not use `all`. Run with
   `--read-only` (`GITHUB_READ_ONLY=1`) unless a workflow needs writes, which
   also satisfies rule 4. Run `playwright-mcp` without `--caps` unless a
   capability is genuinely required. A server left at its widest setting can
   occupy several times the context of this accelerator's own policy and skill
   descriptions combined.
8. Decide the server set before starting a session. Tool definitions are the
   first tier of the prompt cache, ahead of the system prompt and the
   conversation, so adding or removing a server mid-session invalidates that
   cache and the accumulated context is re-established at full price.
9. What a server returns usually costs more than what it declares, because a
   large result stays in context and is re-read for the rest of the session.
   Prefer bounded, structured output, and constrain at the call site anything
   that can return a page, a log stream, or a query result set. Where an API
   has no size parameter, the only lever is a narrower request.

## Optional Developer Tooling: Context Collection

`scripts/collect_context.py` packages a chosen slice of **this repository**
into a single bundle for pasting into an external model — a review in a chat
window, a second opinion on the memory core, a diff explained to a model that
cannot see the checkout. It wraps the optional
[`code2prompt`](https://code2prompt.dev/docs/how_to/cli/) CLI and belongs to
the same category as the MCP servers above: optional, external, opt-in per
developer. It is never installed into a target project, never listed in an
inventory, and never a blocking CI gate.

In Claude Code, opened at the monorepo root, the command is `/collect`:

```text
/collect                                  # list the scopes
/collect skills --edition Laravel --dry-run
/collect core --edition Symfony           # -> .c2p/
```

In a shell, `./collect` at the repository root is the same tool — no
interpreter prefix, arguments and exit status passed straight through:

```bash
./collect                                   # list the scopes
./collect skills --edition Laravel --dry-run
./collect diff --base origin/main --stdout
```

Scopes are `edition`, `skills`, `core` and `hooks` (each takes `--edition`),
plus `tooling`, `docs`, `harness`, `diff` and `custom`. Bundles land in the
ignored `/.c2p/` next to a manifest recording the exact patterns, counts and
binary version.

`/collect` lives in the repository-root `.claude/commands/`, outside every
edition, so it travels with this monorepo and never with an installed
accelerator. It writes the bundle to a file and reports only the summary —
the bundle is meant for a model that cannot see the checkout, so loading it
into the session that produced it would spend exactly the context it was
built to move elsewhere.

The wrapper exists because a bare `code2prompt` invocation is unsafe in this
checkout. It always excludes `.git`; it excludes `Task/` unless `--with-task`,
because that directory holds client-owned product specifications; it excludes
the generated `.claude`/`.cursor`/`.codex` mirrors unless `--with-mirrors`,
since they carry no information their canon does not; and it runs in an
isolated directory, because a `.c2pconfig` in the working directory is
auto-loaded with no way to opt out. A pattern that matches nothing becomes an
error rather than upstream's silent empty bundle.

Reported token counts come from cl100k, the OpenAI BPE tokenizer the CLI
carries. **That is not a count of Claude tokens** — it is a calibrated
relative unit, useful for comparing two bundles, not for predicting a bill.
Full rationale, measurements and the decision not to adopt the `code2prompt`
MCP server or Python SDK: [docs/TOOL-INTEGRATIONS.md](docs/TOOL-INTEGRATIONS.md).

## Infrastructure Creator

`Infrastructure-Creator/` generates an accelerator for a specific target
project; it does not replace the ready-to-use Laravel, Symfony, or PHP Core
editions. Keep it outside the target project, either in a separate workspace
or a sibling directory.

The workflow is `infra-scan → review → infra-generate`. These are AI-assistant
workflows, not terminal executables. Obtain the existing target project's
absolute filesystem path and ask your assistant:

```text
Run infra-scan against the existing target project at
"/absolute/path/to/my-php-app".
```

The absolute path is reliable even when it contains spaces or the visible
project root differs from the assistant's working directory. `infra-scan` only
reads the target and creates a reviewable project profile. `infra-generate`
writes an accelerator only for selected integrations and asks before
overwriting anything. `infra-build` combines both phases.

The generator inspects the actual `composer.json`, framework, dependencies,
integrations, architecture, and CI/CD instead of copying a Laravel, Symfony,
or PHP Core template. A generated accelerator receives the same memory layer as
the ready-to-use editions: the context-brain runtime under
`memory-bank/scripts/`, the `project-brain/` skeleton, and the automatic-memory
hooks, including capsule delivery for Cursor.

Generated output is upgradable in place. Every `infra-generate` run writes
`.infra-manifest.json` into the target (generator version from the root
`VERSION`, the profile it consumed, and the sha256 of every file it generated)
and stamps the version on the first line of the generated `AGENTS.md`. Later,
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

`scripts/check.py` runs the same steps as CI (`.github/workflows/`), one group
per job; [docs/CI.md](docs/CI.md) says what each job verifies:

~~~bash
python3 scripts/check.py                               # every CI job; exit 1 on any failure
python3 scripts/check.py --list                        # every command, and what would skip here
python3 scripts/check.py --group lint --group mirrors  # only these jobs
~~~

The context budget (each edition's `AGENTS.md`, skill descriptions, commands,
agents, and skill bodies) is measured by `scripts/context_budget.py`; the
ceilings live in `scripts/token_budget.json`. That file states a policy of
about five percent headroom, but ceilings have been raised inconsistently:
some were refit to the observed value plus five percent, others raised by
exactly the growth of one change. Headroom therefore runs from a few bytes to
just under five percent of the ceiling, and where it is a few bytes one added
sentence in an `AGENTS.md` can fail CI's lint job. Trim the text or raise the
ceiling in the same change; `python3 scripts/context_budget.py --headroom`
lists what each category has left.

For stack-specific details, open the selected edition's README. For its
durable memory, open the corresponding guide and then that edition's
`INDEX.md`:
[Laravel memory](Laravel/memory-bank/README.md),
[PHP Core memory](PHP%20Core/memory-bank/README.md), or
[Symfony memory](Symfony/memory-bank/README.md).
