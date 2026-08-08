# Changelog

Shared-core history for the PHP AI Accelerators monorepo. The shared
core is what the `Laravel/`, `Symfony/` and `PHP Core/` editions have
in common: the Python memory/context core (`memory-bank/scripts/`,
`memory-bank/tests/`, `memory-bank/templates/`), Project Brain
(`project-brain/` protocol, config, schemas, scripts, tests), the tool
hooks (`.claude/hooks/` and their generated `.cursor`/`.codex`
mirrors), the mirror machinery (`MIRROR_RULES` in
`memory-bank/scripts/context_retrieval.py`, executed by
`scripts/build_mirrors.py`), and the repository-level tooling in
`scripts/`.

A change to a shared-core file must land together with an entry here;
CI enforces this on pull requests via
`scripts/check_core_changelog.sh`. Framework-specific history stays in
each edition's own changelog (`Laravel/CHANGELOG.md`,
`Symfony/CHANGELOG.md`, `PHP Core/CHANGELOG.md`;
`Infrastructure-Creator/CHANGELOG.md` covers the generator). Each PHP
edition also carries a `VERSION` file naming its latest released
changelog section; bump it in the same commit as the release entry.

The Unreleased section below was consolidated from the three edition
changelogs when this file was introduced; entries that describe one
edition's own files remain in that edition's changelog.

## Unreleased

### Added

- **A context-collection command for external models
  (`scripts/collect_context.py`), wrapping the optional `code2prompt`
  CLI.** Invoked as `/collect <scope> [options]` in Claude Code, from a
  new repository-root `.claude/commands/collect.md`, or as `./collect` in
  a shell — a one-line `exec` wrapper over the now-executable
  `scripts/collect_context.py`, so arguments and exit status pass through
  unchanged. A bare invocation lists the scopes. Both entry points sit at
  the repository root, outside every edition, so an installed accelerator
  never carries a command for a tool it does not ship, and
  `build_mirrors.py` — which walks only the four edition directories —
  never sees them. The slash command caps its injected output and tells
  the model to report the summary without opening the bundle, which
  exists for a model that cannot see the checkout. Because `collect`
  carries no `.sh` extension, the `lint` job now lists it by name
  alongside `*.sh`, keeping every tracked shell file inside `bash -n` and
  `shellcheck`. Nine scopes — `edition`, `skills`, `core`, `hooks`, `tooling`,
  `docs`, `harness`, `diff`, `custom` — package a chosen slice of the
  repository into one bundle in the ignored `/.c2p/`, with a manifest
  recording the exact patterns, counts and binary version. Phase 1 of
  `docs/CODE2PROMPT-INTEGRATION-PLAN.md`, and deliberately the whole of
  it: developer-local, zero runtime tokens saved, never a blocking gate.
  The wrapper exists because the bare CLI is unsafe in this checkout,
  and each guard answers a behaviour verified on 4.3.0: `.git` is always
  excluded (the root with `--hidden` and no include reads 2,213 files,
  `.git/config` among them); the client-owned `Task/` specifications are
  excluded unless `--with-task` (collecting `Laravel/**/*.md` without it
  adds ~87k tokens of named-client material); the generated
  `.claude`/`.cursor`/`.codex` mirrors are excluded unless
  `--with-mirrors`; every run executes in an empty directory with
  `XDG_CONFIG_HOME` redirected, because a `.c2pconfig` in the working
  directory is auto-loaded with no opt-out; and a pattern matching
  nothing becomes an error instead of upstream's silent empty bundle.
  Two upstream traits are absorbed rather than documented away: `*`
  crosses directory separators there, so scopes declare top-level wants
  separately and the wrapper expands them with Python's own glob.
  `tests/test_collect_context.py` runs in CI without the binary; its
  `test_containment` fails if `code2prompt` is ever referenced from an
  edition or the installer. Token counts are cl100k (OpenAI BPE) and are
  labelled everywhere as not a count of Claude tokens.

- **Stage D — the external batch harness joins the monorepo as a
  repo-root companion (`harness/`).** A LangGraph-based runner for
  unattended pipelines: the fleet-review graph (Send fan-out of review
  lenses -> durable interrupt approval gate -> report) over headless host
  workers (`claude -p --output-format json` without `--bare`, so the
  project's `.claude` world applies inside every worker; `codex exec
  --json`; an offline `dry-run` worker), with SQLite checkpointing, hard
  cost ceilings, and all durable state flowing through the target
  project's own `context.py` blackboard. Deliberately OUTSIDE the
  editions: never shipped by the installer, never listed in inventories,
  own venv — the shipped runtime stays stdlib-only. First real run
  reviewed the accelerator's own Stage C hooks ($1.68, 10 findings).
- **Host-native orchestration enhancers — Stage C of
  `docs/AGENT-ORCHESTRATION-DESIGN.md`.** A new canonical hook,
  `subagent-dispatch.sh` (registered on Claude Code's `SubagentStop` and
  Cursor's `subagentStop`; the Codex mirror exists but stays unregistered
  while multi-agent is off), records every subagent completion in the Stage B
  channel automatically — one sanitized line from the final assistant message
  (Cursor: `status`, since its documented `summary` field is unreliable) —
  and releases the write-agent lock. Write-capable agents are now declared
  with `writes: true` frontmatter (13 core agents plus each edition's
  framework implementers; the Cursor mirrors carry the key) and the
  subagent gates serialize them: one write-capable agent at a time per
  repository via a TTL lock (`/tmp/<tool>-write-agent-lock-<repo-key>`,
  30 min default, `SUBAGENT_WRITE_LOCK_TTL_MINUTES` override), released on
  completion or expiry; read-only agents keep running in parallel.
  `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH=1` lands in every edition's
  settings.json — the orchestrator is the main conversation, nested trees
  add cost without oversight. The Codex decision point is resolved:
  multi-agent stays off (the edition delegates through skills by design;
  hooks there are a guardrail, not a boundary). Flow commands drop the
  manual completion recording (the observer covers it) and prefer resuming
  an agent over respawning. Regression coverage: `SubagentWriteLockTest`
  and `SubagentDispatchTest` in `memory-bank/tests/test_hooks.py`.
- **Agent communication substrate — Stage B of
  `docs/AGENT-ORCHESTRATION-DESIGN.md`.** The context runtime gains a
  task-scoped agent channel: an append-only JSONL journal per task under
  `project-brain/control/messages/`, written under the global mutation lock
  and validated line-by-line (in-code rules plus the new
  `message.schema.json`) — `msg-send` / `msg-read` (recipient, `--since`, and
  type filters; no read cursor by design) and `msg-dispatch`, the
  orchestration log that fingerprints each delegation capsule (SHA-256) and
  refuses a spawn whose capsule fails the mandatory-section check exposed
  standalone as `capsule --validate` (objective, output format, tool and
  source guidance, boundaries, decisions and assumptions). Bodies are capped
  at the 8,000-character capsule bound and screened by the existing secret
  patterns; a terminal task refuses new messages but its journal stays
  readable after the binding is gone. `update` gains `--actor` (roster-slug
  attribution prefixed to progress) and a phase-order guard: phases move
  only forward unless `--allow-phase-regression` states the regression is
  deliberate. PROTOCOL.md documents the channel; the flow commands record
  their spawns and completions through it; regression coverage in the new
  `memory-bank/tests/test_channel.py` (12 tests, byte-identical across the
  PHP editions).
- **Opt-in orchestration flows — Stage A of
  `docs/AGENT-ORCHESTRATION-DESIGN.md`.** Each PHP edition ships
  `/flow-feature` and `/flow-review`: commands whose `stages:` frontmatter
  declares the agent sequence, executed by the MAIN conversation as
  orchestrator — roster agents only, one bounded delegation capsule per spawn
  (objective, output format, tool/source guidance, boundaries,
  decisions-and-assumptions), parallel stages restricted to read-only agents,
  and a mandatory pause at every declared checkpoint. AGENTS.md gains the
  "Orchestration (Flows, SCOPED)" section; SKILL FLOW.md documents the flows;
  Cursor command mirrors are generated, Codex deliberately keeps its
  sequential skill flow. The `agents_md_bytes` ceilings in
  `scripts/token_budget.json` rise to the new observed values + 5% — the
  growth is the two policy sections, per the ceiling file's stated policy.
- **Subagent spawning is now restricted to the accelerator's own roster.**
  Each edition ships a tool-owned `subagent-gate.sh` in all three hooks
  directories — a `MIRROR_RULES` `skip` entry, since each host exposes a
  different gate contract. The Claude Code gate (PreToolUse, matcher
  `Agent|Task`) allows only the frontmatter `name:`s defined in
  `.claude/agents/` and blocks the built-in agents (Explore, Plan,
  general-purpose, claude, ...); the Cursor gate (`subagentStart`, registered
  with `failClosed`) answers `{"permission": "allow"|"deny"}` against the
  `.cursor/agents/` roster, since Cursor has no setting that disables its
  built-ins; the Codex gate denies the `spawn_agent` multi-agent tool family
  outright, as the Codex edition delegates through skills only. Configuration
  backs the hooks: `Agent(...)` deny rules and
  `CLAUDE_CODE_DISABLE_EXPLORE_PLAN_AGENTS=1` in `.claude/settings.json`,
  `[features] multi_agent = false` plus `[agents] enabled = false` in
  `.codex/config.toml`, the `subagent-policy.mdc` Cursor rule, and an
  AGENTS.md "Subagents" section as the steering layer. Regression coverage:
  `SubagentGateTest` in `memory-bank/tests/test_hooks.py`.
- `scripts/build_mirrors.py` now generates and verifies a per-edition
  `.gitattributes` marking every file it produces as
  `-diff linguist-generated=true`. A generated mirror carries no information
  its canon does not, so it collapses to a stub in review diffs; measured on
  PR #11, the diff falls from 941,788 to 688,674 bytes (225,470 to 166,671
  cl100k tokens, -26.1%). The list is derived from `MIRROR_RULES`, not
  globbed: the tool directories also hold canonical files — hooks, commands,
  agents, settings, Cursor rules, `config.toml` — which are deliberately
  excluded and keep their diffs. Listed in each edition's install inventory
  as a `shared` component. `git diff --name-only` is unaffected, so
  `check_core_changelog.sh` and other name-based gates still see these files.
- `scripts/cost_attribution.py`: developer-local counterpart to
  `context_budget.py`. Reads the transcripts Claude Code writes under
  `~/.claude/projects` and reports spend weighted by billing tier, grouped by
  stratum, project, skill, MCP server, MCP tool, plugin and agent. No
  exporter, no network, no configuration; never invoked by a hook, a skill,
  or CI.

### Changed

- Body-size ceilings in `scripts/token_budget.json` refit to observed + 5 %
  after the branch's final content edits (browser-verify bounds and the
  Infrastructure-Creator forge instructions), per the ceiling file's own
  policy.
- **`context_budget.py` now measures what is actually paid.** Token
  estimates use per-class bytes-per-token ratios measured with cl100k on
  this repository's own files instead of a flat `bytes / 4`, which runs
  19–25 % high on exactly these files; the calibration reproduces the
  research's independent figures to 0.1 % (Laravel bodies 81,639 vs 81,719
  cl100k t). The gated surface is now `descriptor_bytes` (`name` +
  `description` — the text skill selection matches against) rather than the
  whole frontmatter, and skill **bodies are budgeted for the first time**:
  they are an order of magnitude above the startup surface (Laravel ~81.6k
  vs ~4.0k t) and were previously unmeasured, so the gate covered about a
  twentieth of the cost. The monorepo row is relabelled — it is what a
  monorepo checkout exposes, not a price any consuming project pays.
  Ceilings regenerated for the new categories; skill-count ceilings kept
  where they were, since tightening those is a separate decision.
- **browser-verify gained mandatory payload bounds.** Full-page snapshots
  and screenshots are the heaviest payloads in these workflows and are
  carried for the rest of the session; the skill now caps screenshots,
  prefers targeted reads over full snapshots, forbids pasting raw
  snapshot/DOM payloads into reports or Brain records, and requires closing
  the session. Roughly 1 KB of skill body, paid once per invocation, in
  front of payloads measured in tens of KB.
- **Two subagent-gate fixes from the harness self-review.** The write-agent
  lock is now taken under `flock`, so two write-capable agents spawned in
  one message can no longer both observe an unlocked state (covered by a
  concurrent regression test); and the roster parser reads only the first
  frontmatter block, so a `---` rule inside an agent's body can no longer
  declare `writes:` and change the gate's decision. Both degrade to the
  previous behaviour where `flock` or a writable guard path is missing,
  never failing closed.
- CI gains a lint step that fails when a per-turn invalidator (`date`,
  `$RANDOM`, `uuidgen`) reappears in the Cursor rule render — the cheap
  textual net in front of the behavioural test that already covers it.
- The Cursor working-memory rule no longer varies between turns when the
  context does not. `.cursor/rules/working-memory.mdc` is `alwaysApply`, so it
  is re-sent on every prompt; it previously embedded a render timestamp and
  the serialized capsule, whose `manifest` key is a fresh UUID path per call.
  Both changed every turn regardless of content. The Cursor hooks now render
  the capsule (`hook-context` without `--json`) exactly as the Claude and
  Codex hooks already did, so all three clients agree on one form. The
  JSON-parse guard that protected the serialized form is replaced by a render
  marker: a capsule is accepted only if it opens with the `working:` line, so
  a broken render still cannot replace a good rule.
- `print_capsule` reports pre-provision progress (`warming: N turn(s)
  pending`) — the one field the serialized form carried that the warning text
  did not. Emitted only while a task is unprovisioned.
- The `review-pr` skill diffs the PR locally against its merge base instead of
  calling `gh pr diff`, which renders server-side, accepts no pathspec, and
  therefore cannot apply the generated-file markers. Triages on
  `--name-only` first, and states that a stub is not a gap: review the canon
  the file was generated from, and treat a stub whose canonical source is
  absent from the diff as drift worth flagging.
- Per-edition `README.md` files gained the "Optional MCP Integrations"
  section, which previously existed only in the repository-root
  `README_EN.md` / `README_RU.md` — neither of which ships to a consuming
  project. Both root READMEs gained context rules 7-9: scope every server to
  the smallest toolset, fix the server set before a session starts because
  tool definitions are the first tier of the prompt cache, and bound what a
  server returns because a large result is re-read for the rest of the
  session.
- `docs/OPERATIONS.md` gained a Context Economy subsection under Operating
  Rules: delegate to a subagent only when it displaces roughly seven or more
  main-session turns, and compact deliberately near ~400k of context. Both
  thresholds are derived from local transcripts and are marked as such.

## 2.0.0 - 2026-08-07

### 2026-08-06 hook and installation hardening

- Hook payload capture now uses Bash builtins in every canonical validator,
  file-naming validator, and loop detector. Cat-less/extractor-less Bash
  validation fails open with exactly one sanitized warning, while block
  diagnostics disclose only the rule category and never the command body.
- Added exact versioned Laravel, Symfony, and PHP Core installation inventories
  plus a standard-library installer with selected Claude/Cursor/Codex scope,
  spaced-path support, deterministic transcripts, dry runs, complete collision
  preflight, symlink-path refusal (including the target root), and default
  overwrite refusal.
- Added the nine-way edition/tool synthetic clean-install matrix, including
  Memory Bank/Project Brain completeness, retired-file parity,
  validate/status/index smoke checks, source immutability, and sentinels proving
  no application execution or application `.env`/database access.

### 2026-08-06 context-runtime remediation

- Direct `refresh`, `retrieve`, and `context` queries now pass the original
  privacy gate before SQLite or manifest access; unsafe input fails with
  sanitized output and no side effects.
- Task phases persist one canonical five-phase vocabulary, while compatibility
  aliases normalize at the mutation boundary.
- Both context modes now deliver deterministic 2/3/1 capsules within 8,000
  serialized characters, with governed layer exclusions retained in manifests.
- Task completion is explicit and numeric-revision checked. Turn maintenance
  reports sanitized merge candidates without closing tasks; the shipped
  `automatic_completion` setting is false.
- Cursor now carries sanitized warming context through turns one to four and
  atomically replaces it with governed context on the fifth-turn boundary.
- Added frozen retrieval-quality gates, disabled-by-default metadata telemetry
  with explicit `N/A`, and focused concurrency coverage. Promotion docs now
  distinguish truthful automatic mode from independent reviewed mode.

### 2026-08-02 shared-core maintenance round (seven phases)

- **Enforcement hooks hardened and tested** - the hardened hook generation
  (jq/php/python3 JSON extraction instead of greedy `sed`, block messages on
  stderr before `exit 2`, dynamic skill-prefix discovery) now ships in every
  edition including Infrastructure-Creator's `bash-validator`; loop-detection
  counters are namespaced by a repository hash and reset on SessionStart, so
  parallel checkouts stop sharing counters and a file can no longer stay
  blocked forever; a validator that finds no JSON extractor says so on stderr
  instead of passing silently; the hook layer gained its first regression
  suites (`memory-bank/tests/test_hooks.py` per edition plus
  `Infrastructure-Creator/tests/test_hooks.py`).
- **Authority lifecycle unblocks automatic promotion** - `update_record`
  accepts the single legal authority transition `observed -> verified`
  (CAS-guarded, recorded in the transitions ledger), `brain-update
  --authority` exposes it, and the `verify` skill promotes confirmed records
  before their terminal transition - so records created as observations can
  actually reach `promotable_records` instead of staying blocked forever.
- **Turn maintenance moved to the flush boundary** - `close_merged_tasks`,
  `auto_promote` and `auto_compact` run only when the working-memory buffer
  actually flushes, not on every Stop under the 5-second hook budget;
  merged-branch detection is batched into a fixed number of
  `git for-each-ref` calls and the default branch is cached in `index_state`
  with self-healing re-detection.
- **Auto-checkpoints stopped overwriting human progress** - the turn flush
  writes to a dedicated `auto_checkpoint` field; handoffs and capsules show
  the manually recorded `progress` first with the automatic delta as a
  supplement.
- **Turn outcomes became visible** - `context.py turn` persists a compact
  report (flush results, closed-on-merge, promotions with blocking reasons,
  excluded paths, compaction errors) to
  `memory-bank/local/last-turn-report.json`, and the next capsule renders a
  "Last turn" section from it within the existing character budget.
- **Mirrors are generated, parity covers everything** -
  `scripts/build_mirrors.py` regenerates the `.claude`/`.cursor`/`.codex`
  mirrors from the canonical trees according to `MIRROR_RULES` declared in
  `context_retrieval.py` (`mirror_rules.py` for Infrastructure-Creator);
  parity checks hooks, `hooks.json`, commands, agents, the DOD family and
  `*.py` inside skills through the same rules, and `parity --cross-edition`
  verifies the shared core stays byte-identical across the three editions
  (graceful skip outside the monorepo).
- **CI and repository checks** - `.github/workflows/ci.yml` runs every
  edition's unittest suites, full and cross-edition parity,
  `build_mirrors --check`, `bash -n` plus shellcheck at error severity, JSON
  validation, the relative-link checker (`scripts/check_links.py`; all
  previously broken links fixed at the source) and the startup-budget check;
  `docs/CI.md` documents the exact local equivalents of every step.
- **Startup token budget is measured** - `scripts/context_budget.py` reports
  `AGENTS.md` and skill-frontmatter weight per edition, and `--check`
  compares against ceilings pinned in `scripts/token_budget.json` (current
  values plus five percent), so context growth surfaces as a CI regression;
  redundant prose was trimmed from `Symfony/AGENTS.md` and seventeen skill
  descriptions without dropping a single trigger term.
- **Retrieval** - query terms are distilled from the entire prompt (the most
  informative terms ranked against the index) instead of the first 24 words,
  and ranking now modulates BM25 with declared record confidence (linear)
  and a freshness half-life over `updated_at`; `Task/Epics` client planning
  material is no longer indexed as a retrieval source.
- **Memory Bank identifiers** - automatic promotion mints date-plus-source
  identifiers (`MEM-YYYYMMDD-xxxxxxxx`) instead of incrementing the tracked
  `.memory-counter`, and `INDEX.md` is regenerated deterministically from
  the chunk files, so promotions on two machines or branches no longer race
  on a global counter; legacy `MEM-NNNN` chunks stay valid and sort first.
- **Multi-machine continuity** - new `context.py rebind` command, plus
  automatic rebinding by branch (`external_id`) inside the turn flush: a
  second clone of the same branch restores its binding to the existing
  Brain record instead of failing every flush on a duplicate-record error,
  and the turn report states the restoration.
- **Hook hot-path cost** - `bash-validator` matches one combined ERE
  alternation (the per-pattern loop runs only inside a match) behind a
  fork-free command-key pre-filter; `loop-detection` and
  `file-naming-validator` exit early on irrelevant tool names; the
  SessionStart banner caches validation results per repository state, reads
  the Laravel version from `composer.lock` instead of booting the
  framework, and notifications pick `osascript`/`notify-send` by OS.
- **Cursor read path for working memory** - the Cursor mirrors of the Stop
  and sessionStart hooks render the freshest Task Capsule into a gitignored
  `alwaysApply` rule (`.cursor/rules/working-memory.mdc`) via cursor-only
  MIRROR_RULES replacements, so Cursor now receives the capsule one turn
  behind; this supersedes the "Cursor cannot receive a Task Capsule"
  limitation recorded under Fixed below.
- **Versioning and change control** - this root changelog now records
  shared-core history (consolidated from the three edition changelogs);
  each edition carries a `VERSION` file reported by its SessionStart
  banner; CI requires a root changelog entry whenever a pull request
  touches shared-core files (`scripts/check_core_changelog.sh`).
- **Repository hygiene** - `.venv/` and `.superpowers/` are ignored by the
  root `.gitignore` instead of relying on ignore files inside those
  directories; the stray `Task/designs/.gitkeep` files sitting beside real
  design assets and the empty untracked `Laravel/docs/` directory are gone.

### Added

- **`context.py export`** - writes a point-in-time, privacy-filtered bundle of
  Project Brain records and Memory Bank chunks to a directory, so accumulated
  context can be handed to another person or repository. Read-only. Records
  whose privacy is outside `allowed_privacy` never leave, and `MANIFEST.json`
  lists every included item with its ID/type/revision/sources **and every
  excluded item with its reason** - a bundle that quietly dropped records would
  read as a complete one. It also records the source commit, whether that
  installation had `automatic_promotion` on, and which chunks carry
  `auto-promoted` and were therefore never human-reviewed. Fail-closed twice: a
  non-empty destination is refused without `--force`, and one secret-pattern
  match aborts the whole export before any file is written, naming the path
  without echoing the content. There is deliberately no `import`: a bundle is a
  handoff artifact, and adopting one stays a manual act.

- **Automatic compaction** - `automatic_compaction` and
  `compaction_threshold` (default 5) let the turn-end hook archive terminal
  records in batches rather than one at a time. Within a turn the order is
  complete, promote, then archive, and archived records stay promotable, so a
  capped promotion run never loses the remainder to the archive.
- **Automatic completion on merge** - `automatic_completion` in `runtime.json`
  (enabled by default) lets the turn-end hook complete a task once its branch is
  an ancestor of the default branch, recording the episode and closing the
  handoff. The scan covers every active task, since a merge is observed after
  the branch is left. Merging the default branch into a long-running branch is
  not completion, the default branch never closes itself, and a deleted branch
  is never treated as merged. The outcome states the merge and the ancestry
  check that proved it, not a claim about the work being correct.
- **Automatic promotion to durable memory** - `automatic_promotion` in
  `runtime.json` (enabled by default) lets the turn-end hook promote resolved,
  `verified` knowledge into the Memory Bank unattended, on the same boundary as
  the working-memory flush. The runtime never pretends a human approved it:
  `reviewer` stays null, `review_mode` is `automatic`, the outcome is
  `approved-without-review`, the chunk is tagged `auto-promoted`, and
  `promote-review` refuses to sign an automatic promotion after the fact.
  Eligibility is narrow - only resolved findings/bugs, closed incidents, and
  accepted decisions, never tasks, whose checkpoint progress is not reusable
  knowledge. Sources already promoted are never promoted twice. Set the flag to
  `false` for reviewed promotion.
- **Chunk validity periods** - Memory Bank chunks may carry `valid_from` and
  `valid_to` beside `superseded_by`, which answers a different question: the
  link says what replaced a chunk, the period says when it stopped being true.
  Promotion opens an open-ended period. An active chunk may not sit past its
  `valid_to`, so closing a period removes it from retrieval without deleting
  it, and `archived` finally expresses knowledge that ceased with no successor.
  Both fields are optional, so existing chunks stay valid.
- **Task phase** - a governed task may declare `understanding`, `planning`,
  `execution`, or `finalization`, set with `update --phase` and carried into
  the handoff. Progress records what was touched; the phase records where the
  work stopped. The field is optional so records written before it stay valid,
  and only a task may carry one.
- **Requirement-to-test validation map** - `test-generator` now writes
  `tasks/TASK-NNN/test-generator-validation.md`, one row per requirement with
  its source, the test holding it, and its state. An uncovered requirement is a
  row reading `uncovered`, never an omitted row: a green suite says nothing
  about what it failed to check.
- **`codebase-mapper` skill** - writes `codebase/` documents that describe the
  application source an agent would otherwise have to read: stack,
  architecture, structure, conventions, testing, and concerns. Every claim
  cites a path, secrets are noted by existence and never opened, and each
  document is stamped with the commit and scope it was mapped from so
  retrieval can tell when it has fallen behind.
- **Codebase map freshness** - documents under `codebase/` are indexed as a
  `codebase` kind and checked against the code they describe: commits landed on
  their `mapped_scope` since their `mapped_commit` must stay within
  `codebase_map_max_drift` (default 25). A drifted map is excluded as
  `map-drift`, one without a recorded commit as `map-unverifiable`, and
  `refresh` reports the distance so a stale map is visible before it is
  silently dropped.
- **Description-weighted ranking** - each document is indexed with a `summary`
  drawn from its declared description, weighted above the body in BM25. A skill
  body is procedural prose that reads much alike across skills; the description
  states the topic. Existing databases rebuild themselves on first use.
- **Retrieval relevance** - the index now stems with `porter unicode61`, and
  capsule assembly drops corpus-common terms and requires a document to share
  more than one query term. Without stemming a security question did not
  retrieve the security skill, because its title says "Reviewer" and the query
  said "review". Explicit `search` stays broad. Existing databases rebuild
  themselves on first use; episodes are migrated, not dropped.
- **Zero-command working memory** - the turn-end hook now provisions the task on
  its first flush instead of failing until an operator ran `start`. A goal is
  derived from the branch name and states its own provenance, ticket
  identifiers survive intact, and a task that already exists is never
  overwritten. Buffered turns alone provision nothing, so visiting a branch
  mints no record; only accumulated work does.
- **Per-request memory-layer refresh** - added `context.py refresh`, which
  re-indexes procedural, semantic, and episodic memory in one incremental pass
  and reports each layer as `updated` or `failed`, optionally assembling the
  Task Capsule in the same process. The request hook runs it, so every request
  refreshes all three layers and says so; a failed layer is reported instead of
  silently narrowing the result. Working memory stays out of it: it is written
  by the turn hook, and Project Brain remains its authority in governed mode.
- **Automatic working memory** - added a `UserPromptSubmit` read hook that
  refreshes the index and emits a bounded Task Capsule per request, and a
  `Stop` write hook that buffers each turn's change set and flushes it to the
  authoritative task on a boundary. Reads and writes are split because a
  request has nothing to record yet and the end of a turn does. Both hooks are
  fail-open and time-bounded. Durable memory is populated automatically on the
  same boundary; see the automatic-promotion entry above.
- **Incremental indexing** - `index --incremental` reuses rows whose source
  modification time and size are unchanged, so a no-change refresh performs
  reads only. Secret scanning dominated a full pass, and mirrored skill copies
  that lose the dedup are no longer read at all. Governed `retrieve` now
  refreshes the index itself instead of silently reading a stale one.
- **Ephemeral retrieval manifests** - `retrieve --ephemeral` writes the same
  validated manifest to ignored local state instead of shared Git history, and
  prunes to the most recent 200. Automated retrieval requires it.
- **`--revision auto`** - resolves the current revision inside the mutation
  lock so automated writers perform a real compare-and-swap instead of omitting
  the check.
- **`context.py turn`** - buffers a per-turn working-memory delta from Git
  porcelain metadata alone and flushes a consolidated update on a boundary.
  Sensitive-looking paths and the runtime's own churn are excluded, and a
  per-flush file cap reports what it omits rather than dropping it silently.

- **Combined Project Brain + Local Context Engine architecture** - added
  governed tasks, findings, bugs, incidents, decisions, and events; revision-safe
  handoffs; ownership and conflict metadata; cross-store rollback/compensation;
  compaction; and source/revision-bound promotion proposals with independent
  human review before atomic Memory Bank application.
- **Bounded governed retrieval** - added a dependency-free SQLite FTS5/BM25
  index with privacy, ownership, authority, lifecycle, and source-freshness
  filtering, bounded snippets and token budgets, conflict retention, and
  retrieval manifests that contain metadata rather than source bodies, prompts,
  responses, or hidden reasoning.
- **Authority-aware unified memory workflow** - added `memory` and `checkpoint`
  across Claude, Cursor, and Codex discovery. Source indexing excludes
  Git-ignored files and fails safely on invalid UTF-8 without replacing the
  previous valid index.
- Added bounded Task Capsule retrieval and hybrid fresh-context handoffs for
  complex phase boundaries without adding another memory store or changing
  `memory`, `checkpoint`, or explicit `complete`.
- **Local context engine** - added a dependency-free SQLite FTS5 index for
  policy, specs, active memory, task documents, capability epics, and changelog
  history, plus gitignored summaries of completed tasks. It classifies
  procedural, semantic, episodic, and working context in one local database;
  callers supply task IDs, retrieve bounded per-layer packets, and atomically
  complete a working task into an episode. The shared `memory-bank` skill keeps
  index, search, record, and status compatible without treating local episodes
  as authoritative memory.

### Changed

- **`memory` now runs `refresh`** - the skill reports each memory layer exactly
  as the CLI returned it instead of inferring all three from one `index` exit
  status, and it runs the same command the request hook does, so the skill and
  the hook cannot drift apart. It never passes `--query`: `memory` reports
  layer health and performs no task-aware retrieval.

- **Governed context is now the default** - policy and edition documentation
  define Project Brain as shared active-work authority and SQLite as a
  disposable index plus local binding/cache. They preserve explicit
  `--mode lightweight` for non-authoritative local Working Memory, expose
  `python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID` for
  task-aware retrieval, and state that canonical project sources outrank all
  context.
- **`memory` and `checkpoint` now honor authority** - in governed mode,
  `memory` validates Brain and refreshes the index without creating task
  authority, while `checkpoint` skips local Working Memory and directs a
  revision-checked Brain/handoff update. Branch-derived checkpoints remain
  available only in explicitly configured lightweight mode.
- **Memory Bank ownership narrowed** - the `memory-bank` skill now handles only
  durable retrieval/capture/audit/supersession and application of explicitly
  human-approved promotions; active work and proposals remain in Project Brain.
- **Session hooks remain metadata-only** - startup reporting is limited to mode,
  index health/staleness, active binding count, and validation status, with no
  automatic indexing, retrieval, record printing, or prompt injection.

### Fixed

- **`parity` was permanently red, so it had stopped being a gate.** Shared
  skills named their own edition directory - `.codex/DOD.md` in `.agents`,
  `.claude/DOD.md` in `.claude` - and used two vocabularies for the same target
  (`debugger` the command vs `systematic-debugger` the skill). Every mirror
  therefore differed by construction and `parity` failed on a clean checkout.
  Rephrased the shared text neutrally, as `AGENTS.md` already does with "the
  active edition's `DOD.md`", and unified flow references on skill names - the
  only vocabulary that resolves in `.agents`, which has no command layer.
  Mirrors are now byte-identical and the gate is green from a clean checkout.
- **`parity` could only report one drifted path, and `--json` was unreachable
  when drift existed.** `assert_skill_mirror_parity` raised on `drift[0]` before
  a result was built, so repairing N files took N runs and no machine-readable
  output was available on the failure path. It now reports every path, `--json`
  returns the full list with a per-path reason, and text output goes to stderr
  with exit 1. Added coverage for the failure path, which had none.
- **`parity` ignored a file only one mirror carried.** The comparison skipped any
  logical path missing from canonical, so a stray copy in `.claude` could sit
  there indefinitely. Absent-from-canonical and missing-from-mirror are both
  drift now.
- **`skill-creator` cannot be mirrored and is now an explicit exemption.** Its
  body drives each product's own CLI (`codex exec`, `cursor-agent --print`,
  `claude -p`) with different environment variables and a different extension
  model - Cursor builds command and agent wrappers, Codex is forbidden from
  creating them. Byte-parity would mean telling a Codex user to run
  `cursor-agent`. It joins `SKILL FLOW.md` in a named, documented exemption list
  rather than being deleted on one side or silently tolerated.
- **Automatic memory was wired only for Claude Code.** `working-memory-read.sh`
  and `working-memory-write.sh` shipped in all three edition mirrors but were
  referenced only by `.claude/settings.json`, on the stated grounds that the
  other clients' event names were unverified. They are verified now. Codex
  lists `UserPromptSubmit` and `Stop` in its configuration reference, so both
  halves are wired in `.codex/hooks.json`, with `additionalContextLimit: 4000`
  on the read hook because an 8,000-character capsule exceeds the 2,500-token
  default. Cursor's `stop` is wired to the write half.
- **Cursor cannot receive a Task Capsule, and the documentation claimed it
  could.** Cursor's `beforeSubmitPrompt` returns
  `{"continue": ..., "user_message": ...}`: it allows or blocks a submission but
  cannot add context to a prompt, so a capsule printed from it is discarded.
  `.cursor/hooks/README.md` nevertheless listed `beforeSubmitPrompt:
  Working-Memory Read` as an active hook. Removed that section, deleted the
  unusable `.cursor/hooks/working-memory-read.sh` rather than leaving a script
  that can never run, and published a capability matrix in
  `docs/TOOL-INTEGRATIONS.md`. The write half is unaffected, so continuity is
  still recorded on Cursor; only retrieval into the prompt is unavailable there,
  and explicit `retrieve` still works.
- **Documentation described reviewed promotion as an invariant while the
  shipped default was automatic.** `project-brain/config/runtime.json` has
  shipped with `automatic_promotion: true`, but `PROTOCOL.md`, `docs/SECURITY.md`,
  and both READMEs stated that an independent human must approve every durable
  memory. The code was never dishonest - an automatic promotion records
  `reviewer: null`, `review_mode: automatic`, `approved-without-review`, tags the
  chunk `auto-promoted`, and `promote-review` refuses to sign one after the fact
  - so the defect was in the prose. Automatic promotion is now documented as the
  first-class default, with its cost stated plainly: durable memory is
  accumulated rather than curated, and a retrieved chunk is a pointer to its
  cited source, not a vetted fact. Reviewed promotion remains available behind
  `automatic_promotion: false`.
- **The shipped runtime config was covered by no test.** Every automation test
  calls `enable_automation()`, which overwrites `runtime.json` with the flags
  that test needs, leaving the defaults users actually get untested. Added
  `ShippedRuntimeConfigTest`, which reads the real file and fails if the
  automation flags, mode, provider, telemetry setting, or private-record
  exclusion change without being released as the behavioral change they are.
- **Automatic promotion filtered records in silence.** A decision whose cited
  source had been edited since it was written was held back by the freshness
  check and simply never appeared in durable memory, with nothing said. Every
  eligibility rule now reports the record it blocked and why.
- **Promoted chunks repeated their own title and carried no content.** The
  chunk body was the record's rendered template, so a decision produced a stub
  that stated its heading three times over `Next Steps: None`. Chunks are now
  built from the record's progress note and cited sources, and a record with
  nothing beyond its title is not promoted at all.
- **The relevance filter hid the document that answered the question.**
  Requiring two distinct query terms rewarded documents containing generic
  words and dropped a focused one matching only the single term that mattered:
  a conventions document stating how money is represented lost its slot to
  skills sharing "how" and "project". A term rare in the corpus now qualifies a
  document on its own.
- **Overlapping source patterns aborted the whole index.** Two patterns
  matching one file inserted the document twice and failed the metadata primary
  key, so any future pattern addition that overlapped an existing one would
  have broken indexing entirely. Files are now claimed by the first matching
  pattern.
- **Compaction left promoted Memory Bank chunks citing a path that no longer
  existed.** A chunk records its source record by path; archiving that record
  made the citation dangle, failed Memory Bank validation, and blocked every
  later promotion. Compaction now repoints affected chunks at the archive
  location atomically with the move, and validates the bank before committing
  to it. Present before automation; certain to occur with it.
- **A promotion that failed to apply blocked its source record for good.**
  Deduplication treated any non-rejected promotion as done, so an automatic
  promotion that stalled mid-pipeline was never retried. Stalled automatic
  promotions are now driven forward in place rather than re-proposed.
- **Merge detection completed a task the moment it was provisioned.** A branch
  with no commits of its own is already an ancestor of its target, so ancestry
  alone was not evidence of a merge. The target must also have moved ahead of
  the branch.
