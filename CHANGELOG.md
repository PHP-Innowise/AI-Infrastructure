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

- **The Kit 3 risk registry (`install/open-source-kit/registry/`) and
  `scripts/validate_registry.py`.** A curated list of other people's
  repositories is not something we can own or be accountable for on an
  outstaff engagement; four things are, and they are what the registry
  records: what was checked and found, isolation, measurement, and removal
  without residue. The tool list is a consequence of the gates rather than a
  decision taken ahead of them.
  - **The registry describes; it does not forbid.** Nothing in it refuses a
    tool. The selector installs nothing either way - it records a choice and
    prints a command a human runs - so a refusal would only block writing the
    choice down, and an install that happened anyway would then be missing
    from the audit trail entirely. What it produces is a dossier: what was
    checked, what was found, and what would close each open item. The team
    decides.
  - Twelve gates per candidate. Eight are **binary**, answered
    `pass`/`fail`/`unknown` - `license`, `data_egress`, `pinning`, `uninstall`,
    `collisions`, `auto_update`, `maintenance_ownership`, `measurability`.
    Four are **scored** 0-5. They summarise as `clear`, `open_questions`, or
    `known_risks`; `unknown` is reported as its own state rather than folded
    into `clear`, because absence of evidence is not evidence of safety.
  - The status is stored for review but never trusted. The validator
    recomputes it from the gates and fails when the two disagree, so a stored
    `clear` cannot outrank a failing gate. The cross-check runs both ways - a
    status may be neither kinder nor graver than its gates.
  - `.kit3-manifest.json` carries the status into the client project under
    `review`, so what review found is visible in a diff long after the
    terminal output is gone. It also records `install_method`,
    `install_guidance` and `risk_notes`: how something was installed is part
    of its risk - `curl | bash` is not `git clone` - and an audit trail that
    answers "what" but not "how" leaves that in terminal scrollback. The field
    is `guidance`, not `command`, because the tool knows what it proposed and
    not what a human actually ran.
  - **`scripts/kit_fetcher.py` and `install_open_source_kit.py --refresh`.** A
    stored `verified_command` is a snapshot from whenever someone last read
    that README by hand and goes stale. `--refresh` checks it against the
    README as it stands right now, before recording anything: it fetches the
    repo's current metadata and README from the GitHub API and looks for
    fenced code blocks near an install-related keyword. It never guesses -
    exactly one candidate is used and recorded as
    `install_guidance_source: "live_fetch"` with `install_guidance_fetched_at`;
    zero or several candidates, or any network failure, falls back to the
    stored guidance unchanged, because picking among several would present a
    guess as a fact. Opt-in, not the default: it makes a run network-dependent
    and non-deterministic, and unauthenticated GitHub API calls are
    rate-limited to 60/hour - browsing, testing, and recording a selection
    offline stay exactly as fast and reliable as they always were.
    Discovery-index entries are skipped, since "install" is not a concept for
    a curated list. 16 tests exercise the fetcher fully mocked (no live
    network in the suite), plus 5 more that patch `kit_fetcher.refresh`
    in-process to prove the selector's fallback behavior end to end -
    including one asserting `kit_fetcher.refresh` is never even called
    without the flag. Live-verified against this catalog while writing the
    fetcher: correctly caught a real GitHub rate-limit error and fell back
    rather than crashing or writing nothing.
  - `intersection_map` is mandatory and may not be empty. The hidden cost of
    Kit 3 is not context, it is two systems doing one job, and an entry with
    no intersection analysis is unexamined whatever its gates say. Each row is
    typed `duplicates`/`replaces`/`conflicts`/`complements`.
  - A gate that is `fail` or `unknown` must carry `resolves_by`, so a finding
    is a work item rather than a dead end. Unmeasured cost scores 0 and may
    not coexist with a passing `measurability` gate.
  - Star count is excluded from scoring by construction. Inflated counts are
    recorded in `trust_signals.disqualified_signals` as findings, never scored:
    an implausible count is evidence the signal is fake, not that the code is.
  - Two worked entries, both `known_risks` on recorded evidence rather than
    taste. `graphify`'s PreToolUse hook occupies the same interception point
    as `local-context.sh` with no precedence resolution recorded, and it
    duplicates the Local Context Engine's retrieval job. `ohmyclaude`'s
    19-agent self-orchestrating architecture is what `subagent-gate.sh`,
    `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH=1` and the denied
    `Agent(Explore|Plan|general-purpose)` entries refuse by construction - it
    will not run under our accelerator without an explicit roster-whitelist
    decision. Both also carry unresolved `measurability` and
    `maintenance_ownership`.
  - 38 tests cover all three status paths and every enforcement property,
    including four defects found by adversarial probing before release: a
    malformed gate crashed the validator with `AttributeError` instead of
    reporting it (a traceback in CI reads as broken tooling rather than as bad
    input); `score: true` passed the 0-5 check because `bool` subclasses `int`;
    and whitespace-only `evidence`, `resolves_by` and `intersection_map`
    fields satisfied a truthiness test while committing a reviewer to nothing.
  - Six of fifteen catalog entries (`obra-superpowers`, `caveman`, `graphify`,
    `ohmyclaude`, `claude-code-templates`, `claude-plugins-community`) carry a
    `verified_command` checked against the tool's own current README, plus
    `command_verified_date` recording when. The other nine keep the generic
    `install_type` guidance instead of a fabricated single answer, because no
    one unambiguous command exists yet for them (a 93-plugin marketplace, a
    93-server MCP collection, a teaching repo with no install script) or none
    has been verified. `graphify`'s command names the PyPI package as
    `graphifyy` (double y) against the CLI it installs, `graphify` (single y) -
    checked against PyPI on 2026-09-01: description and maintainer match
    Graphify-Labs/graphify, a taken-name workaround, not a typosquat.
    `.kit3-manifest.json` now carries whichever command actually printed, so
    the audit trail states the real install step, not only that one was
    proposed.

- **`document_links`, a reverse index from source path to the documents that
  cite it, with `context.py links` and `retrieve --path` (roadmap H4-02,
  H4-03).** Two durable chunks whose only connection was a `sources[]` entry
  naming the same file were unreachable from each other and from the source's
  own words: a query phrased in the source's vocabulary returned the source
  document and **neither chunk — 0 of 2**, while a control query using words
  the chunks share returned 2 of 2. The link the two chunks already asserted
  was sitting unread in their own frontmatter.
  - `document_links(path, ref_path, ref_kind)` is built during indexing from
    the `sources` of durable chunks, Brain records and handoffs, with an index
    on `ref_path`. On the same fixture the link route returns **2 of 2**.
  - `context.py links --path P [--prefix]` reports the citations. It joins
    `document_metadata` and applies the full runtime filter; `search_documents`
    deliberately does not, and a `links` modelled on `search` would have
    republished restricted records.
  - `retrieve --path P` (repeatable, also on `context`) delivers them in the
    same capsule, budget and manifest. Path-linked items lead their layer,
    capped at `PATH_LINK_LIMIT`, and carry `selection: "path-link"` in the
    manifest with no `match` and no `rank` — `match` says how the *query*
    matched, and nothing lexical selected these. `selection` is added to
    `retrieval-manifest.schema.json`, which is `additionalProperties: false`;
    the strict top-level key set is untouched, so no manifest version bump.
  - Injection happens strictly after `matched_layers`/`no_match` are computed,
    so the capsule's `no-match:` line and `gate.signals.no_match` keep meaning
    "the query found nothing in this layer". A layer can report `no-match` and
    still deliver a path-linked document on the same turn.
  - A `related:` field on chunks was considered and **not** built: it repairs
    the same case only if a human authors an edge for every pair, and nothing
    in the repository authors chunk edges. Recorded in
    `docs/CONTEXT-AND-MEMORY.md` beside the embeddings negative.
  - `ref_kind` carries only `source`. `fingerprint` is dead by construction
    (`sources_are_fresh` requires the fingerprint path set to equal the
    sources path set) and a task's `files[]` is Git churn in the wrong path
    frame — every live task here lists twenty entries led by phpunit cache and
    vendored JavaScript, written relative to the Git toplevel rather than the
    repository root.
  - When the table is absent the runtime drops `document_source_state` so the
    first index after an upgrade re-reads every candidate. `discover_documents`
    returns only *changed* documents, so populating it incrementally would
    leave it complete for whatever happened to change next while claiming to
    be complete.

- **Project Brain records render a `## Sources` block (roadmap H4-03).** The
  index reads bodies, not frontmatter, so a finding created with
  `--source app/Billing/InvoiceTotal.php` was not reachable by the one string
  a reader is most likely to search for. Existing records keep their current
  body until their next mutation rewrites them; nothing verifies an on-disk
  body against `_record_body`, and `source_hash` is recomputed at index time.

- **`codebase/**/*.md` is indexed recursively (roadmap H4-05).** The pattern
  was single-level, so a codebase map filed into per-module directories — the
  shape a map big enough to be worth writing takes — was indexed at its top
  level and nowhere else. A strict superset of what was indexed before.


- **Cross-edition parity now covers the hook surface, and a new
  `scripts/asset_parity.py` covers the generator asset (roadmap H1-01).**
  The memory core has four copies, not three: the three PHP editions plus
  `Infrastructure-Creator/.agents/skills/memory-seed/assets/`, which is
  copied verbatim into every project the generator builds. Nothing
  compared that fourth copy, and the hooks — the only automatic entry
  into the engine — were outside `CROSS_EDITION_CORE_MANIFEST` entirely,
  so both drifted while CI stayed green.
  - `CROSS_EDITION_CORE_MANIFEST` gains `.claude/hooks/*.sh`.
    `bash-validator.sh` (this framework's destructive commands) and
    `local-context.sh` (this framework's session-start detection) are
    framework surface and are exempted by name in
    `CROSS_EDITION_ALLOWED_DRIFT`; every other hook is engine and must
    match. `subagent-gate.sh` needs no exemption — it is already
    byte-identical across the editions, and its `MIRROR_RULES.skip`
    entry governs the `.claude`→`.cursor`/`.codex` axis inside one
    edition, which is a different question.
  - `scripts/asset_parity.py --check|--write` compares the asset against
    the canonical edition in both directions: drifted bytes, canonical
    files the asset never received, and asset files that map to no
    canonical path (so a new asset file cannot become silently
    unchecked). `runtime.json.template` cannot match byte-for-byte, so it
    is compared by JSON key set against the edition's `runtime.json` — a
    new runtime setting now either reaches generated projects or fails
    the gate. Deliberate one-sided files are listed with their reason in
    `ASSET_ONLY` / `EDITION_ONLY`.
  - CI: the `parity` job runs both the asset check and
    `tests/test_asset_parity.py` once, on the `Laravel` leg.

- Added a first-class WordPress accelerator at `Cms/wordpress` with canonical
  WordPress policy and skills for plugins, classic/block themes, Gutenberg,
  actions/filters, content modeling, REST, WP-CLI, multisite, WooCommerce and
  background processing; integrated it into mirrors, installation inventories,
  context budgets, cross-edition runtime parity, CI and documentation.

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

- **CI now syntax-checks the PHP the accelerator actually ships.** The
  repository tracks no `.php` file, so no job had ever parsed a line of PHP -
  yet its skills, examples and DOD carry 363 fenced PHP blocks, and those
  blocks are what an agent copies when it writes code. A broken one propagates
  into generated applications while every existing job stays green. The new
  `scripts/check_php_snippets.py`, wired into the `lint` job, extracts each
  block from tracked Markdown and runs `php -l` over the 93 that begin with
  `<?php` and therefore claim to be whole files; the remaining 270 fragments
  are counted and reported rather than linted, so the step never overstates
  its coverage. All 93 pass today. `--require-php` makes a runner that lost
  its preinstalled `php` fail rather than report a silent pass.

### Changed

- **Integration of the WordPress edition with the H1–H4 memory work.** The two
  branches merged cleanly as text and would have shipped a broken engine: the
  new edition carried its own copy of the five core modules taken from `main`,
  so four of them were behind, and `parity --cross-edition` — which the
  WordPress branch itself extended to cover the edition — reported ten
  divergences. Reconciled by bringing the edition to canon rather than by
  relaxing the gate.
  - Four core modules, three shared test files, `PROTOCOL.md`, the retrieval
    manifest schema and `project-brain/tests/test_runtime.py` synchronised.
  - `.claude/hooks/working-memory-read.sh` was the pre-H1-01 variant that
    truncates the prompt with a regex; replaced with the canonical one, which
    passes the raw prompt to the CLI where distillation and the secret gate
    live. Left as it was, the edition would have asked the core a different
    question than its siblings — the exact drift H1-01 exists to prevent.
  - `memory-probes.json` copied to the edition: it is byte-identical across
    the other three, so it is engine data rather than framework data.
  - `SkillRoutingTest` now skips explicitly, naming the gap, when an edition
    ships no `skill-routing-golden.json`. The guard stays after WordPress got
    its own set, because the generator ships this test into projects that have
    no roster fixture at all.
  - WordPress gained both routing fixtures. `skill-routing-golden.json` holds
    18 boundary cases over its roster and a floor **measured at 17 of 18** —
    the one miss is "write tests for the checkout extension", which reaches
    `woocommerce` because "checkout" is that skill's strongest term. The
    comment warns against reading 17/18 against Symfony's 10/16: WordPress's
    domain skills carry far more distinctive vocabulary than Symfony's
    overlapping pairs, so the two numbers are not comparable.
    `.agents/evals/routing.json` holds 15 cases including two restraint ones,
    and `Cms/wordpress` joins `EDITIONS` in `routing_eval.py` in the same
    change, as that file's own note required.
  - `policy_lock.py`, `check_stabilization.py` and `routing_eval.py` all
    learned the edition; it ships 41 agents, its own stabilization rules and
    now its own routing cases.
  - Mirrors, the generator asset and all four installation inventories
    regenerated; the WordPress inventory gains its policy lock.


- **Infrastructure-Creator's context-budget ceilings were raised for the
  content review-and-repair phase** (`scripts/token_budget.json`, observed
  values + ~5%): the generator gained the `infra-validate` and
  `content-reviewer` skills with their agent wrappers and the
  `/infra-validate` command, growing the edition's startup surface and skill
  bodies. The change that justifies the growth is recorded in
  `Infrastructure-Creator/CHANGELOG.md`.

- **The ready-made accelerator installer can now adopt standard existing
  project root files without destructive overwrites.** The new
  `--merge-existing` mode preserves project `.gitignore`, `.gitattributes`, and
  `AGENTS.md` content while adding marked accelerator-managed blocks, keeps an
  existing `README.md` and installs accelerator documentation as
  `ACCELERATOR.md`, treats byte-identical files as unchanged, and continues to
  abort atomically on every unsupported collision. The installation and
  adoption guides document preview, transcript, and rollback behavior, with
  tests covering safe merging, idempotence, and unsupported conflicts.

- **Ready-made installations now have an explicit production boundary.**
  Schema-v2 inventories classify every tracked edition file as installed or
  source-only, reject unclassified/stale/overlapping metadata, and use a
  declared source override to install a clean Memory Bank index. Client
  `Task/` specifications, maintainer changelogs and memory, regression suites,
  retired counters, and generic examples remain available in the source
  repository but no longer enter consuming projects. Standalone payload tests
  now validate their own Markdown links without repository allowlists. The
  installation, operations, context/memory, orchestration, CI, troubleshooting,
  token-research, edition, and scripts documentation was reconciled with the
  implemented runtime and current production contract.

- **The context budget now gates the whole startup surface, not the half of
  it that was easy to measure.** `scripts/context_budget.py` grew two
  categories, `command_bytes` and `agent_bytes`, because a measurement of what
  a session actually pays found that commands and the agent roster were
  **62 % of Laravel's startup cost and gated nowhere**: the agent listing
  alone was 7,405 t against 2,455 t for the skill descriptors the gate did
  watch. Both listings are `name` + `description`, derived from the first body
  paragraph when frontmatter declares none, exactly as Claude Code derives it;
  that derivation was validated against this repository's own mirror
  generator, which implements the same rule for Cursor - 43 of 45 Laravel
  commands byte-identical, the two exceptions being the pinned
  `description_overrides`. The new bytes-per-token ratios, 5.11 and 5.16, were
  measured with real cl100k rather than estimated. `startup_tokens()` now sums
  the four startup categories instead of `AGENTS.md` + full frontmatter, which
  over-stated the listing surface by roughly 1.5x, and a skill declaring
  `disable-model-invocation: true` is excluded from `descriptor_bytes` because
  Claude Code does not put its description in context - closing the
  over-statement noted when `/sdd` took that flag.

  Both listings are measured on the `.claude` tree, the richest of the three
  tool surfaces, so gating it bounds the others rather than tracking each.
  Cursor carries the same files with a few deliberately condensed for it;
  **Codex carries neither** - `.codex/` holds only `config.toml`, the
  governance documents, `hooks/` and `hooks.json`, and `agent-forge` forbids
  writing an agent there - so for a Codex session these two categories
  over-state the cost by their whole value, leaving `AGENTS.md` plus the skill
  descriptors. Recorded in the script, in the ceiling file and in
  [`docs/CI.md`](docs/CI.md) rather than left for a reader to rediscover.

  **Every startup category now counts what a tool renders, not what the file
  says.** A frontmatter field contributes its value with the key, the colon,
  surrounding quotes and their backslash escapes removed and wrapped lines
  folded, which is how the model receives it; `frontmatter_bytes` stays the
  deliberate exception as a file fact. This started as a noticed wart - the
  old code counted the whole `description: "..."` entry, some 14 bytes per
  file above the rendered text - and comparing the Claude and Cursor listings
  made it visible as a phantom 645-byte "difference" that was only Cursor's
  generated `description:` keys. Fixing it moves `descriptor_bytes` too, which
  had counted `name:` and `description:` entries since it was written. All
  three listing ratios were re-measured on the rendered text (descriptor
  4.99 -> 4.91, command 5.11 -> 5.10, agent 5.16 -> 5.14; the 4.99 came from
  research taken on key-inclusive text and no longer described what is
  counted), and the three ceilings were refit to observed + 5 % - keeping them
  would have handed out phantom headroom created by changing the ruler. The
  report's estimate now lands within 0.1 % of a direct cl100k count of the
  same payload.

- **`sdd` is user-invoked only (`disable-model-invocation: true`).** Claude
  Code documents that this keeps a skill's description *out of context
  entirely* rather than merely blocking automatic loading, which turns the
  startup cost of a rarely-used skill from "halved by a shorter description"
  into zero. The description stays short anyway - it is what the `/` menu
  shows - and now also states how to invoke it, since the model will no longer
  surface the skill on its own. The second effect is the reason to want this
  independently of bytes: a multi-agent, multi-session flow should not begin
  because a prompt looked spec-shaped. Note the consequence for the gate:
  `scripts/context_budget.py` measures files, not runtime behaviour, so it
  still counts these 172 bytes per edition against `descriptor_bytes` even
  though Claude Code no longer loads them. The gated figure now over-states
  the real startup cost by exactly this skill's descriptor. Cursor and Codex
  receive the key through the mirrors and may ignore it; the flow is driven by
  the `/sdd` command in every tool regardless.

- **Body-size ceilings refit again after the `sdd` skill, and only those.**
  `scripts/token_budget.json` `body_bytes` moves to observed + 5 % — Laravel
  370,217, Symfony 181,612, PHP Core 208,474 — while the descriptor and
  frontmatter ceilings stay where they are. The two categories are paid at
  different times, and `/sdd` is the case that makes the difference visible:
  its body is 6,031 B (~1,426 t) charged only when the flow runs, perhaps once
  in a project's life, but its descriptor is charged on **every** session of
  that edition, whether or not the flow is ever used. At the measured
  descriptor ratio that is ~68 t per session against ~1,426 t once, so the
  descriptor overtakes the body after about 20 sessions: a rarely-invoked
  skill is the *worst* case for the startup surface, not a cheap one. Raising
  the body ceiling therefore costs approximately nothing, and raising the
  descriptor ceiling would buy a permanent startup tax in every consuming
  project — so instead the `sdd` description was cut from ~330 B to 172 B by
  dropping the trigger phrases, which exist to win automatic skill selection
  that a deliberately typed `/sdd` does not need. That returns 158 B per
  edition per session and leaves the descriptor headroom at 426 B (Laravel),
  328 B (Symfony) and 292 B (PHP Core) without moving a ceiling. Skill-count
  ceilings are also untouched: 36 × 1.05 rounds to PHP Core's existing 37, so
  its single remaining slot is what the policy prescribes rather than an
  accounting artefact.

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

- **Browser verification now walks journeys, and reports defects instead of
  repairing them.** Three rounds of manual testing on a completed application
  found 15 defects that a 419-of-419 acceptance-criteria suite and the
  agent's own browser pass had both stayed green through, and the skill's
  shape explains why: every checklist item asked "on this page, does X work",
  while every one of the 15 needed a role walked from its first action. A
  parent with a zero balance pressing "Register & Pay" got an HTTP 500; the
  page it sat on loaded perfectly. `browser-verify/SKILL.md` gained a
  mandatory `What To Walk` section - roles enumerated, each role's first
  action named, ordered by blast radius rather than by proximity to the diff,
  each journey walked empty as well as populated - and calls out the two
  classes a page check cannot see: the feature that saves and never renders,
  and the mechanism behind the button. It also gained a `Defect Report` output
  contract, because the previous "verified flows, evidence, blockers" had
  nowhere for "the feature works and is invisible" to land. The screenshot
  bound became per-journey rather than per-verification, and exhausting the
  budget now requires naming the journeys left unwalked. In
  `browser-verify-agent.md`, the instruction "If issues found: fix code, wait
  for hot-reload, re-verify (max 3 attempts)" is gone: a verifier that repairs
  what it finds hands back a green report and no list, and the list is the
  deliverable.

- **The debugger sweeps the root cause across the codebase before calling a
  bug fixed.** `EntityManager::wrapInTransaction()` closes the manager on any
  exception, expected ones included, so six services handed their controller a
  dead manager while reporting an ordinary rejection - one shape, six sites,
  and fixing the reported one left five live. Phase 4 of
  `systematic-debugger/SKILL.md` gained a `Sweep The Root Cause` step: search
  for the mechanism rather than the symptom, report every other site with file
  and line even when fixing it is out of scope, and treat that list as the
  requester's scope decision rather than a "while I'm here" improvement the
  same phase forbids. `project-brain/templates/bug.md` gained `Root Cause`,
  `Same Shape Elsewhere` and `Guard` sections so the sweep and the regression
  test survive in the record; an empty sweep section means the search ran.

- **The Definition of Done and the test-data skills now name the
  order-dependence a green suite hides.** A new test signed in as a seeded
  fixture account and failed, because an earlier password-reset test had
  permanently changed that fixture's password - a result that depended on
  suite order and surfaced as an unrelated assertion. Each edition's
  `.claude/DOD.md` now requires the whole suite in one run, states that a
  `--filter`/`--group` run is a debugging aid to be reported as filtered, and
  adds an item requiring tests to own the rows they assert on. The
  `test-generator` skills and Symfony's `fixture-factory-generator` carry the
  same rule where the test is written: a seeded fixture is a read-only prop,
  and anything a test authenticates as or mutates is minted by that test.

- **`docs/SECURITY.md` states what the write-agent gate does not guarantee.**
  The policy-versus-enforcement table gained a row for it: an advisory,
  machine-local `/tmp` lock that does not serialize two containers on the same
  branch, expires after `SUBAGENT_WRITE_LOCK_TTL_MINUTES`, gates subagent
  spawns rather than the main conversation's own edits, and fails open without
  a JSON extractor, without a readable roster, or - for the check-and-take
  race - without `flock`. Each `.claude`/`.cursor` hooks README carries the
  same limits next to the serialization it already documented.

- **`Infrastructure-Creator.body_bytes` in `scripts/token_budget.json` refit
  to 232069 B** (observed 221018 B + 5 %, the ceiling file's own policy), and
  only that ceiling. The growth is 901 B of `bootstrap-verifier/SKILL.md`
  documenting two new gate checks - the offline resolution of a literal
  `grep` in `verification[].command` and the optional `--baseline-plan`
  coverage comparison - which carried the edition 577 B past its previous
  220441 B. Trimming was measured first and rejected on the numbers: the two
  documentation blocks are 901 B in total, so closing a 577 B overrun inside
  them would have deleted about two thirds of what they document, including
  diagnostic identifiers the report text is grepped for. `body_bytes` is paid
  per skill invocation, not at startup, and the startup categories
  (`agents_md_bytes`, `descriptor_bytes`, `command_bytes`, `agent_bytes`),
  `frontmatter_bytes` and `skills` are unchanged and stay where they are.

### Fixed

- **Promotion carries a record's citations through to the chunk.** Found by
  installing the edition into a real Symfony project rather than by a fixture:
  `apply_promotion` recorded only the source records, so two findings about one
  design document promoted into two chunks that shared no source at all. The
  chunk named the record, the record named the document, and `links` and
  `retrieve --path` are one hop by design — so the document reached the records
  and never the knowledge derived from them. The hand-written fixture had two
  chunks citing one document, a shape the automatic pipeline never produces.
  - Only `sources` is inherited, never `files`: a record's `files` is Git churn
    in the Git-toplevel frame, and merging it would put code paths under
    `chunk_source_digests` where the next edit evicts the chunk.
  - A citation whose file no longer exists is dropped rather than carried. It
    is reachable — delete the file between review and apply — and without the
    guard `validate_metadata` rejects the new chunk and fails a promotion that
    has nothing to do with that file.

- **A source tree lost entirely to `.gitignore` is now reported.** Same real
  installation: the project's own `.gitignore` carried a bare `docs` entry, so
  `docs/**/*.md` matched two design documents on disk and silently indexed
  neither. The index held 99 documents — 96 of them the accelerator's own
  skills, one the project's `README.md`. Discovery now emits one exclusion per
  pattern that lost every candidate, named for the pattern with the reason
  `pattern-all-git-ignored`; it surfaces wherever excluded paths already do,
  including the per-turn hook line. Individual ignored files stay silent as
  before — Symfony alone contributes hundreds.


- **A promoted chunk declares what it was promoted from (roadmap H4-04).**
  `apply_promotion` wrote `type: "decision"` whatever it promoted, so a
  resolved bug and a closed incident both entered the Memory Bank claiming to
  be decisions, and `INDEX.md`, `bank-audit` and every reader keyed on type
  were reading a value nothing had chosen. `PROMOTION_TYPE_BY_RECORD` maps the
  four promotable record types to `constraint`, `operations`, `domain` and
  `decision`; every value is in `validate.ALLOWED_TYPES`, and `decision`
  remains the fallback. It sits beside `PROMOTABLE_STATES` so the two stay
  visibly coupled.

- **A snippet for a candidate the query did not match is prose, not
  frontmatter.** `_conflict_candidates` took `substr(content, 1, N)`, and a
  durable chunk opens with a JSON metadata block long enough to fill the whole
  snippet allowance — so a conflict partner was delivered as a wall of quoted
  keys and digests. Candidates selected by the query escaped this because FTS
  `snippet()` centres on the match. `_body_snippet` skips a leading
  frontmatter block and is used by conflict partners and path links alike.


- **The bank validator distinguishes a broken chunk from a retired one
  (roadmap H1-07).** `validate_bank` returned a flat list and `main()` exited
  `1` on any element, so deleting a cited file — dropping a controller,
  regenerating a schema — failed Definition of Done on an unrelated task for a
  chunk that had been correctly archived. The engineer's only outs were to
  fake `sources` or delete the record, both of which destroy the provenance
  the bank exists for.
  - `validate_bank_report` returns `(errors, warnings)`; `validate_bank` keeps
    its list-of-errors signature and delegates, so every existing caller in
    `brain_runtime.py` and the tests is untouched. A terminal chunk
    (`superseded`, `archived`, or past its `valid_to`) whose cited source has
    been deleted is a warning; an active chunk with the same defect is still
    an error, because a chunk that still answers questions must still be able
    to show where the answer came from. A source path escaping the repository
    stays fatal in every status.
  - `validate_metadata` takes an optional warnings sink. Without one its
    behaviour is byte-for-byte unchanged, which is what keeps `active_memory`
    — the retrieval gate — strict rather than quietly admitting active chunks
    with deleted sources into the index.
  - The cascade message is fixed: an index row whose chunk is on disk but
    failed validation now reads `index row retained for invalid chunk`
    instead of `points to a missing chunk`, which sent the reader looking for
    a file that was sitting right there. It reproduced for *any* metadata
    error, not just deleted sources. The row is matched against the ids seen
    on disk, falling back to the filename, because a chunk can fail before
    its own id is readable.

- **Cursor now retrieves on the task, not on the branch name (roadmap
  H1-06).** Cursor has no prompt-submit event, so its one automatic memory
  entry point hands over a task identifier — in practice a branch name — and
  the governed path tokenized that slug and retrieved on it. Measured: branch
  `main` filled both procedural slots with `wcag-accessibility` rules matched
  on the word "main"; `chore/accelerator-hardening` returned a release skill.
  The enrichment that fixes this already existed and was used only on the
  lightweight path.
  - `hook_capsule_query` builds the query from the task behind the binding —
    goal, manual progress, next steps, file stems — via the existing
    `build_capsule_query`. The identifier still leads, so branch tokens are
    kept, but a second informative term raises `required_coverage` from 1 to
    2 and documents whose only hit is the slug stop qualifying.
  - Two exclusions are deliberate. The automatic checkpoint is a sentence of
    turn counts and an ISO timestamp with no topical content that would spend
    a third of the 32-token budget and change the query on every flush. An
    auto-provisioned goal is the branch slug re-cased, so using it would
    reintroduce the same noise under another name; it is detected by
    comparing against `derive_goal`, which builds that form, rather than by
    matching a literal suffix.
  - When nothing substantive remains, or the task cannot be read, the bare
    identifier is used rather than failing — Cursor would otherwise keep
    serving the previous rule with no signal. The capsule prints `query: from
    task goal` or `query: from branch name only` after the `working:` render
    marker, and the manifest records the same distinction as `query_source`.
  - `docs/TOOL-INTEGRATIONS.md` described the Cursor difference as one turn of
    staleness only; the query source was the larger half and is now stated.

- **The signals a retrieval gate will need are now persisted, and the
  observation window is configurable (roadmap H1-05).** Everything needed to
  decide whether a turn should retrieve at all is already computed during a
  retrieval and was discarded at the end of it: `adjusted_score` never left
  `retrieve()`, rank existed only as list position, phase timings printed only
  under `--json` on a branch the hook never takes, and nothing recorded where
  the query came from. Horizon H3 has nothing to measure without them.
  - Manifest `selected[]` entries gain `score` (the adjusted relevance) and
    `rank` (position in the lexically ranked list, stamped in `_candidates`
    before any filter — a position in the post-filter list would say where a
    survivor landed, not how well it answered). A conflict partner is pulled
    in by record id and never ranked, so it reports `rank: null` and an
    explicit `score: 0.0` rather than inheriting a relevance it never earned.
  - The manifest gains `query_source` (`prompt` | `task` | `task-id` |
    `explicit`, threaded from each dispatch site because neither `retrieve()`
    nor `assemble_capsule()` can infer it from a bare query string) and
    `phase_seconds` for stat, index and retrieval. `retrieve()` times its own
    body rather than reusing the CLI's `retrieval_seconds`, which measures a
    strictly larger interval; a phase that did not run this turn reports
    `null` rather than `0.0`, which would claim an instantaneous index.
  - `local_manifest_retention` moves from a module constant to
    `project-brain/config/runtime.json` (default 200). It is floored at 1:
    zero or a negative value would delete the manifest the current retrieval
    just wrote while the returned capsule still advertised its path.
  - Plain `refresh` now prints `phases: stat Xms index Yms retrieval Zms`.
    The hook echoes that report verbatim and never passes `--json`, so this
    is the first time the number that says whether a turn is approaching its
    five-second budget is visible where it can be acted on. It is printed in
    the `refresh` branch, not in `print_capsule`, whose first line must stay
    the `working:` render marker the Cursor hooks key on.
  - **Manifest schema version 2.** `brain_runtime.validate_repository`
    duplicates the manifest key set as a literal and compares it with strict
    equality, so any new top-level field would have invalidated every
    manifest a consuming project had already written — a red Definition of
    Done on upgrade, for a file nothing re-reads. The key set is now selected
    by the manifest's own declared version: a version 1 manifest keeps
    validating, and a manifest claiming version 2 must carry what version 2
    promises. `schema_version` accepts `[1, 2]`; the new fields are declared
    explicitly, since every Brain schema is `additionalProperties: false`.

- **The capsule now says how well it matched, and when it matched nothing
  (roadmap H1-04).** A capsule that retrieved noise and a capsule that
  retrieved an answer looked identical from inside a turn, and a capsule that
  found nothing looked identical to one that was never consulted. Any policy
  rule of the form "refuse when there is no data" was therefore asking the
  model to observe something the harness never reported — exactly what
  `STABILIZATION.md` forbids.
  - `match_strength` names which of the two admission routes `is_relevant`
    took: `covered` (the document carried the required number of distinct
    query terms) or `distinctive` (it did not, and was admitted on rarity
    alone). The threshold is `required_coverage(tokens)`, which is 1 for a
    single-term query — so a one-term query is always `covered`, correctly:
    there is no weaker way to match it. A conflict partner is pulled in by
    record id rather than by the query and is recorded as `conflict`, which
    also keeps it from tripping the projections below.
  - The label is set on both retrieval paths — `_candidates` for governed
    retrieval and `search_documents` for the lightweight one — and named in
    all three key whitelists (`manifest["selected"]`, the capsule `groups`
    projection, the returned `selected`), any one of which would otherwise
    have swallowed it silently.
  - The capsule is zero-sum against an 8,000-character ceiling and serializes
    each selected item up to three times, so `covered` is carried by its
    absence: the manifest records the verdict for every item, the capsule
    spends characters only on the one that needs a caveat. `print_capsule`
    renders it as `weak-match: <path>`.
  - `no-match: <layers>` names the layers where no candidate passed the
    relevance test. It is computed before any privacy, authority, lifecycle,
    budget or layer-limit filter runs, because a layer emptied by a filter is
    not a layer memory had nothing for — deriving it from an empty rendered
    list would have re-created the very conflation the line exists to remove.
    In the manifest the same distinction reads as an empty `selected` next to
    an empty `excluded`.
  - Both lines are printed after the opening `working:` line, which the Cursor
    hooks require as the render marker.
  - `match` is declared in `retrieval-manifest.schema.json` with an enum but
    deliberately left out of `required`, so manifests written by an earlier
    build stay valid in a consuming project.

- **One document can no longer hold two slots of the same capsule
  (roadmap H1-03).** The governed capsule builds its semantic layer from
  retrieval categories and its episodic layer from the layer column, and the
  two taxonomies disagree: `category_for` has no `changelog` branch, so
  CHANGELOG.md — the only episodic document a repository ships — was category
  `evidence` and layer `episodic` at once. It took one of the three semantic
  slots and the single episodic slot together, so a sixth of a capsule bounded
  at 2+3+1 went to a duplicate while the degradation ladder dropped real
  content to stay under 8,000 characters.
  - `retrieve()` now excludes episodic-layer documents from the semantic
    ranking *before* the 2+3 truncation rather than after it. Filtering
    afterwards would subtract without replacing — the freed slot would be
    lost instead of going to the next ranked candidate — and would leave the
    written manifest asserting a selection the delivered capsule no longer
    matched.
  - A document held back because its own layer will carry it is now excluded
    with reason `episodic-layer` rather than `layer-limit`; the manifest
    schema takes a free-form reason string, so nothing else changed.
  - `deduplicate_capsule_layers` is a layer-preserving guard in
    `assemble_capsule`, added so a future layer source cannot reintroduce the
    collision unnoticed. It resolves a collision by ownership — a document
    goes to the capsule layer its own `layer` field names — rather than by
    first-come priority, which would have let a semantic copy empty the one
    episodic slot. It removes nothing today.
  - The lightweight path never had the collision (it queries each layer
    separately) and is now held to the same asserted contract as the governed
    one.

- **Retrieval quality is measured rather than asserted (roadmap H3-03,
  H3-04, H3-06), and one proposed mechanism is recorded as refuted
  (H3-05).**
  - **Memory probes with declared categories** (`memory-probes.json`, twelve
    cases across recall / update / restraint / reasoning, and a deterministic
    gate). The old fixture expressed only `{query, expected[]}`, so negatives
    had to be hardcoded in a test body and a restraint case could not be
    written at all. UPDATE cases exercise the retire path end to end: a chunk
    is written, retired with `bank-retire`, replaced, and the retired one must
    not be delivered. Cases seed and clear their own corpus — without that a
    later case retrieved an earlier one's documents and the gate stopped
    measuring what it named.
  - The RESTRAINT predicate is deliberately narrower than proposed. The
    roadmap's version would have declared a documented, deliberate behaviour a
    defect: retrieval surfaces the least-bad lexical match rather than nothing,
    on the recorded ground that a hidden answer costs more than a spurious one.
    So restraint is asserted where the claim is unambiguous — a subject absent
    from the corpus must produce `no-match` — and the single-rare-term case
    asserts the checkable thing instead: the selection is *marked*
    `distinctive`.
  - **A skill-routing floor** (`skill-routing-golden.json` plus a gate that
    copies the edition's real skill tree). `acceptable` is a set, not one
    slug, because several requests have two defensibly correct answers and
    asserting one would measure the fixture author's taste. `min_top2` is the
    measured value — Laravel 12/15, Symfony 10/16, PHP Core 13/14 — recorded
    as a regression floor and explicitly not a quality bar. Routing is poor
    today; the number exists so it cannot quietly get worse.
  - **A roster routing eval** (`routing.json` per edition, `routing_eval.py`).
    The unit is the *skill*, not the agent: agent name equals skill name for
    every roster entry, so a name-based detector cannot tell them apart, and
    the skill layer is the only one Codex shares. The capsule stays on,
    because a baseline without it would describe a configuration nobody runs.
    `hit` / `miss` / `wrong` stay separate because a miss means the
    description is too narrow and a wrong means it overlaps a neighbour.
    Baselines are keyed to `policy_digest`, and `--dry-run` refuses to write
    one — it scores perfectly by construction and would publish a meaningless
    1.0. No baseline is recorded here: producing one invokes a model.
  - `docs/EXTENDING.md`: a pull request changing any `description:`, or the
    composition of the roster, attaches the routing delta.
  - **H3-05's descriptor filter is refuted, not implemented.** Both readings
    of "admit a skill only by the distinctive route" were simulated against
    real indexes: gating on match strength drops the right skill and keeps the
    wrong one ("map the codebase" loses `codebase-mapper`, retains
    `researcher`), and gating on distinctive-path membership keeps the
    proposal's own showcase failures. A skill's `description` is indexed as
    the `summary` column at weight 8, so correct and incorrect skill matches
    arrive through the same channel and no filter over it separates them. The
    negative is recorded in `docs/CONTEXT-AND-MEMORY.md` beside the embeddings
    negative, with the two measurements that should decide any future remedy.

- **The retrieval manifests are read, and turn health accumulates (roadmap
  H3-02).** The prompt hook is the most frequent event in the system and its
  honesty lived exactly one turn: a timeout, a slow index phase or a capsule
  that dropped content was visible in that turn's report and nowhere
  afterwards. None of the CLI's subcommands read a manifest — they were
  written and died — and `telemetry.py` had been written, disabled, covered by
  five tests and never called from anywhere.
  - `context.py retrieval-report [--since N] [--scope] [--json]` aggregates
    turns, empty selections, exclusion reasons, gate decisions and skip
    reasons, match strengths, phase percentiles and the twenty most-selected
    paths. `--since` counts records, not days, and says so.
  - It reports the top score **twice** — best candidate before any filter, and
    best delivered — because those diverge exactly when the best match was
    withheld, which is the case the report exists to surface. Picking one for
    the reader would hide it.
  - Version 1 manifests, which carry no gate, timings, query source or
    per-item score, are counted rather than skipped, and every metric states
    how many manifests could answer it. On a real corpus that is most of them.
  - `context.py health [--window N] [--json]` aggregates a new append-only
    `memory-bank/local/refresh-health.ndjson`, bounded by
    `refresh_health_retention` in `runtime.json` — a window, so a runtime key
    beside `local_manifest_retention` for the reason H1-05 recorded, not a
    module constant. It reads only the health series: the manifest's
    `retrieval` phase and a refresh's `retrieval` phase are different
    intervals that H1-05 split deliberately.
  - Records are appended by `refresh`, by `hook-context` (Cursor's read path
    never enters the refresh branch and would otherwise be invisible), and by
    `working-memory-read.sh` itself, which writes its own status line **before**
    the early exit — on a timeout the Python process is killed before it can
    write anything, and that is the one case worth measuring. The hook had no
    status capture at all; the roadmap's `$STATUS` did not exist.
  - `emit_refresh_telemetry` gives `telemetry.py` its first production caller,
    guarded against the three ways its contract breaks a turn: it reads its
    config file before checking whether it is enabled and raises when that
    file is absent; `task_id` must be a UUID while the refresh branch has a
    branch name; and `context_tokens` exists only on a capsule that a refresh
    without `--query` never builds. Imported lazily, wrapped, and validated by
    the caller so `telemetry.py` keeps its zero coupling to Project Brain.
  - Privacy: neither report nor the health series carries the query text, in
    either output mode — the roadmap asked for this on `--json` only, but the
    text report and the NDJSON are the same leak.

- **The surface the model reads now has a content identity (roadmap
  H3-01).** It was gated on volume (`context_budget.py --check`) and on mirror
  agreement (`parity`, `mirrors`) and on nothing that said what it *is*.
  Measured consequences: `model: sonet` in an agent's frontmatter passed every
  job — the mirror builder drops that line whatever it says, and the budget
  counts only the stem and the description, so no byte moves; and once mirrors
  are regenerated, an unreviewed instruction appended to an agent's *body* —
  which is its prompt — passes `mirrors`, `context_budget` and
  `check_stabilization` alike.
  - `scripts/policy_lock.py --check|--write` writes
    `<edition>/.accelerator-policy-lock.json`: sha256 per surface file, each
    agent's declared model, and a `policy_digest` over the sorted manifest.
  - **Whole files, never projections.** Hashing only frontmatter would store a
    digest under a path key that is not `sha256(path)`, turning every
    independent check into a false positive — the exact failure the lock
    exists to remove.
  - **Enumerated from Git, never from disk**, because `.cursor/rules/` holds a
    gitignored render that changes every turn and a filesystem glob would bake
    it into the lock permanently.
  - The model allowlist is enforced on `--write` as well as `--check` —
    otherwise a regeneration launders the typo and then agrees with itself —
    and is per edition, since Infrastructure-Creator ships no haiku agent and
    its own `agent-forge` skill requires opus or sonnet.
  - The lock lives inside the edition rather than under `install/`, so the
    installer classifies it as a shared component with no code change and it
    lands beside `VERSION` — the only place a session hook can find it without
    knowing its edition name. Inventories regenerated.
  - `local-context.sh` prints `Accelerator version: 2.0.0 (policy b3dbfccb)`.
    It prints the *recorded* digest, which is honest here — CI refuses a
    surface change without regenerating the lock — and as stale as the version
    line in a project that edited a skill locally; that limit is stated in the
    code. `skill_tree_fingerprint` was deliberately not reused: it is computed
    from mtime and size, which makes it a cache-invalidation key rather than a
    statement about content.
  - `check_core_changelog.sh` gained a second rule: a change to an edition's
    model-facing surface requires an entry in that edition's own changelog. A
    single ERE could not express it — the two rules have different targets —
    so the script now runs both and reports both. The three edition changelogs
    were back-filled for the H1/H2 skill, DOD, command and STABILIZATION
    changes, which had been recorded only in this file.

- **The stabilization rules are validated against the template they declare
  (roadmap H2-08).** `STABILIZATION.md` states the shape a rule takes and
  nothing checked that the rules took it — the contract rested entirely on the
  discipline of whoever wrote the last one, in a repository that gates
  markdown links, JSON syntax, PHP snippets and byte budgets.
  - `scripts/check_stabilization.py` validates the required fields, that
    `Rule` states an obligation rather than a preference, that `Enforcement`
    names something, that dates are ISO, that a `Retired` rule sits under
    `## Retired rules` and vice versa, that `Superseded-by` resolves within
    the file, and that `Evidence` — optional — is a UUID naming a Project
    Brain record on disk. Wired into the `lint` job with its regression tests,
    following the `context_budget.py` precedent for stdlib policy checks.
  - Infrastructure-Creator is reported as **skipped**, not passed: it writes
    the cycle as prose and carries no rule blocks, and a validator that
    silently approves a file it never examined is the defect it exists to
    prevent.
  - A parser bug the tests caught, worth recording because it is the exact
    case being validated: the field regex used `\s*`, which matches a
    newline, so an empty `**Enforcement:**` swallowed the line break and
    adopted the next line as its value — the check would have passed the one
    thing it was written to catch.
  - **Most of this item's premise does not exist.** It describes
    `STABILIZATION.md` as already localizing failures with `Edge`/`Blame`
    fields over a nine-component vocabulary, already declaring an "earliest
    unrecovered failure" rule and a `Blame`-to-`Enforcement` routing rule, and
    Symfony as already rewritten to carry them. `Blame`, `Edge`, "unrecovered"
    and the nine-component vocabulary appear nowhere in the repository except
    in the roadmap paragraph proposing to validate them. Steps 2, 3 and 6 were
    therefore not implemented: authoring a failure-localization taxonomy is a
    design decision for a human, not something to infer from a description of
    a document that was never written.

- **A procedural rule can now be retired, and the budget it frees is
  visible (roadmap H2-07).** `/reflect` could only ever add: the rule template
  carried `Added:` and nothing else, there was no notion of a rule that had
  stopped applying and no way to take one back out. `AGENTS.md` is paid on
  every session of an edition and gated in CI, so the only growing channel of
  procedural memory ran toward a wall with nothing to give back.
  - The rule template gains `Retired:` and `Superseded-by:`, in both places it
    lives — `.claude/STABILIZATION.md` and the `reflect` skill, which
    duplicate it and would otherwise fork on the first edit. `STABILIZATION.md`
    gains a `## Retired rules` section: a retired rule is kept for the reason a
    superseded memory chunk is kept, and that file is not part of the
    per-session budget.
  - `/reflect` gains an explicit add / overwrite / retire decision before it
    writes, with the search that finds what a new rule might replace. Two
    limits are stated rather than papered over: the index must be refreshed
    first or `search` silently returns nothing and the decision degrades to
    `add`; and the procedural layer covers `AGENTS.md`, `CLAUDE.md` and skill
    bodies only, so candidates in `GOLDEN-PRINCIPLES.md` and
    `STABILIZATION.md` have to be found by reading them.
  - It also now names the canonical tree to write into and what to do in a
    consuming project, where the mirror builder is not installed: make the
    identical edit in every tool tree present, or the retire manufactures a
    governance-document parity failure in the project it was meant to help.
  - `context_budget.py --headroom` prints the bytes remaining under each
    ceiling, tightest first. The roadmap said this mode was unnecessary
    because `--check` already prints the numbers; it does not — it prints
    `ok`/`FAIL` and nothing else, which is why its own readiness criterion
    could not be met without it.
  - **No 5 % warning was added, deliberately.** `token_budget.json` sets every
    ceiling at the observed value plus about five per cent, so headroom is
    ~4.8 % of the ceiling by construction: a 5 % warning fires on all 28
    edition×category pairs at once, which is the same as no warning. Saying so
    is more useful than shipping an alarm that is always on.
  - The premise figure was already dead: Symfony's `agents_md_bytes` headroom
    is 847 B (4.72 %), not 137 B — the H1-01 status block had recorded the
    same correction. `Symfony.body_bytes` did have to rise, to cover the skill
    text this horizon added across H2-02, H2-03, H2-04 and this item; the
    ceiling was re-baselined to the documented observed + 5 %.

- **The episodic layer gets a Git-tracked source, and stops bypassing the
  filters (roadmap H2-06).** The layer existed as a taxonomy, a table, a CLI
  verb and a record type, and in governed mode had exactly one Git-tracked
  document: the changelog. Completed work lived only in the disposable local
  database, so the one layer meant to hold "what happened here" could not
  survive a fresh clone.
  - `complete` now writes an `event` record beside the local episode, under
    the same lock and rollback. Best-effort by construction: a task that
    completed is never reopened because its episode could not be written.
    The outcome and verification go in the record's `goal` — `create_record`
    hard-codes empty progress and `update_record` refuses to mutate an event,
    so a create-then-update pair is not available. The external id derives
    from the task UUID rather than its slug, because branch names are reused
    and events are never archived, so a slug-derived id would collide the
    second time round and fail the completion. Cited sources are filtered to
    files that still exist, since `create_record` fingerprints every one.
  - `event` — and only `event` — maps to the episodic layer. The roadmap also
    proposed `incident`, which would be harmful: an open incident is active,
    urgent, promotable content, and the single episodic slot would take it out
    of the runtime filters, the budget, the manifest, and the conflict-partner
    fetch.
  - **`retrieve()` now owns the episodic slot.** It was assembled by the
    caller from a `search_documents` query that never joins
    `document_metadata`, so it applied no privacy, owner, authority, lifecycle
    or freshness filter and never reached the manifest. Harmless while the
    only episodic document was the changelog; a hole on the per-prompt hook
    path the moment governed records live there. Ranking it with everything
    else also closes the audit gap and makes H1-03's `episodic-layer`
    exclusion reason literally true, and `synchronize_views` now spans all
    three layers so the manifest describes every delivered document.
  - The roadmap's step 1 — indexing `project-brain/archive/` — was dropped as
    inert: every archived record is terminal by contract and
    `record_is_eligible` rejects terminal statuses, so the one-line change
    adds zero documents and only grows an exclusion tally. Carving archived
    records out of that check would make the manifest's advertised
    `filters.active_only: true` a lie, which is a decision this item is not
    the place to take.
  - Two existing tests were tightened rather than worked around: the manifest
    is now asserted to describe every layer the capsule delivers, episodic
    included, and the changelog is asserted to arrive through the governed
    path instead of being excluded from it.

- **Two records saying one thing no longer become two chunks (roadmap
  H2-05).** `noop` existed by record identity — the same record cannot be
  promoted twice — and in reviewed mode as an explicit rejection. The case
  with no executor was two *different* records carrying the same consequence:
  promotion never read the bank it writes into, so the policy "update an
  existing chunk instead of creating a near duplicate" had nothing enforcing
  it on the automatic path. With H2-04 giving the pipeline producers, that
  stopped being latent.
  - `auto_promote` now compares each candidate's promotion content against
    the active chunks and blocks a near duplicate with
    `near-duplicate of MEM-...; merge or supersede first` — through the
    existing `blocked` channel, so it reaches the last-turn report and the
    capsule, and names the chunk so the block reads as an instruction.
  - **The check runs inside the promotion loop, not before it.** One flush
    promotes up to five records, so a single review producing several findings
    that say the same thing — the realistic case, and the one the roadmap
    describes — is exactly what a pre-loop snapshot cannot see: it would
    compare only against chunks that existed before the run.
  - Neither key the roadmap proposed to match on works. A chunk's `sources`
    holds the *source record's* file path, so two chunks promoted from two
    records can never share one; and promotion hardcodes chunk `type` to
    `decision` regardless of the record type. Similarity is measured on what
    actually carries the conclusion: the title plus the body.
  - Similarity is 3-gram Jaccard over a fixed stop list, not the retrieval
    layer's corpus-adaptive vocabulary. That vocabulary is derived from
    `memory-bank/local/`, which is git-ignored and disposable, and letting it
    decide would make the content of the tracked Memory Bank depend on local
    state — two machines on the same commit would promote different sets.
    Whole-set Jaccard was avoided because this repository has already recorded
    it diluting below any usable threshold.
  - Digits are kept at any length. They are often the only thing separating
    two findings — an error code, a version, an ordinal — and dropping them as
    too short made texts identical that no reader would confuse.
  - `bank_duplicate_ratio` (default 0.6) in `runtime.json`, beside
    `automatic_promotion` and `compaction_threshold`. `bank-audit` reports
    duplicate pairs among existing chunks.

- **The consolidation pipeline has producers, and its emptiness is now
  reportable (roadmap H2-04).** Promotion is built end to end and well
  tested — flush-boundary distillation, narrow criteria, a blocked-with-reason
  report — and had received nothing in nine days of use, because no workflow
  produced records it could promote. `/flow-review` writes its findings into
  the task's `--progress`, and the `task` record type is absent from
  `PROMOTABLE_STATES`, so a review's output could never become a candidate.
  - `/flow-review` now materializes each confirmed finding as a `finding`
    record — **from the orchestrator, after synthesis**, not from the review
    agents. Making a reviewer write-capable would put a second write-capable
    agent in a stage the subagent gate serializes behind a TTL lock, which
    that command explicitly forbids. Creating records after synthesis also
    means nothing dropped as unevidenced becomes durable memory.
  - `systematic-debugger` is already declared `writes: true` and records its
    confirmed root cause directly. `code-reviewer` and `security-reviewer`
    gained the opposite instruction — report, do not record — so the division
    is stated where each skill is read rather than inferred.
  - The commands in the roadmap did not exist: there is no `--status` flag and
    no positional record id. The two real edges are
    `brain-update --record-id <id> --revision auto --authority verified` and a
    second call with `--transition resolved`. Both are written down with
    `--progress`, which is not optional bookkeeping but the record's only
    content — resolving without it produces a record blocked as carrying
    nothing beyond its own title, which is the same dead end as never
    creating it.
  - `DOD.md` Standard tier: every confirmed review or debugging finding exists
    as a `finding` record, resolved or explicitly deferred.
  - `context.py status` reports promotable, blocked, applied and chunk counts.
    Each is `null` (printed `unavailable`) rather than 0 when it cannot be
    established — in lightweight mode, without a `project-brain/`, or on a
    failed walk — because an absent Brain and an empty one are different
    facts. The walk degrades rather than failing: `status` is on the
    session-start hook's budget.
  - The roadmap's Stop-hook line was dropped as unimplementable where it was
    specified: `working-memory-write.sh` discards stdout and always exits 0,
    which the code says twice and the generator mandates for produced
    projects. Nothing printed there reaches anyone.

- **A durable chunk can now notice that what it cites has moved on (roadmap
  H2-03).** The provenance asymmetry was inverted: a Project Brain record
  lives for days and carries `source_fingerprints` re-checked on every
  retrieval, while a chunk lives a year or more, is auto-promoted with
  `review_after` a year out, and carried no such check at all — the schema did
  not even permit one. So the longest-lived claims were the least verified.
  - Optional `source_digests: [{path, sha256}]` on the chunk schema, written
    by promotion and by the new `bank-reverify`, validated for shape and
    required to be a subset of the chunk's own `sources`. Optional on purpose:
    requiring it would invalidate every chunk written before it existed, the
    same reason `valid_from`/`valid_to` are optional.
  - The digests cover what the chunk itself cites, not what the source record
    cited. `sources_are_fresh` compares path sets for equality, so digests of
    the record's code paths would have been permanently unequal to the chunk's
    own `sources` and every chunk would read as changed on day one.
  - `promoted_source_rewrites` now repoints digests along with citations when
    compaction archives a promoted record. Without that, the file moves,
    the digest keeps the old path, and every promoted chunk reads as
    `source-changed` forever after the first compaction.
  - `_runtime_filter` no longer restricts the freshness check to Brain
    records. A chunk whose cited file changed is excluded with reason
    `source-changed` rather than `stale`: the two words name different
    remedies — a record is refreshed by a revisioned mutation, a chunk by
    re-reading the source — and the existing `stale` runbook and its test keep
    their meaning. The check sits before the budget and the 2+3 truncation, so
    an excluded chunk frees its slot to the next candidate and the manifest
    still describes the delivered capsule.
  - `_legacy_metadata` writes the digests into `document_metadata` from the
    content the indexer already holds, rather than re-opening every chunk on
    the prompt hot path.
  - **`bank-reverify` is not optional scope.** Nothing but promotion has ever
    written chunk frontmatter, so without a re-attestation path the digests
    would be a one-way ratchet: the first change to a cited file would remove
    the chunk from retrieval permanently, leaving hand-editing as the only
    remedy — the very thing these commands replace. It recomputes digests and
    `last_verified` under the `bank-retire` transaction, and refuses
    non-active chunks so it cannot resurrect retired knowledge.
  - `bank-audit` reports active chunks with changed or missing citations,
    chunks past `review_after`, and chunks that cite local files but record no
    digests. A URL source is cited and never digested — the runtime has no
    network, so nothing is the only honest thing it can say about one.
  - Fixed in passing, and closed for good: `memory-bank/templates/chunk.md`
    had drifted — Symfony and PHP Core still handed engineers the retired
    `MEM-0000` identifier scheme that the bank's own validator had moved past.
    Templates were in neither the cross-edition manifest nor the allowed-drift
    list, so no gate could see it. `memory-bank/templates/*` is now in
    `CROSS_EDITION_CORE_MANIFEST`.

- **Retiring a chunk is now one command instead of an editing convention
  (roadmap H2-02).** Retire was designed carefully — `superseded_by` for "what
  replaced it", `valid_to` for "when it stopped being true", `archived` for
  "it ceased and nothing replaced it" — and then left to hand-editing: change
  the frontmatter on both sides of a link, regenerate the index, validate.
  Between any two of those steps the bank is invalid. The only code that ever
  wrote `valid_to` was promotion, and it always wrote `null`; the field is
  named in no skill, so an agent following policy honestly set a status and
  never wrote a date.
  - `retire_chunk` in `brain_runtime.py` follows the `apply_promotion`
    pattern: mutation lock, snapshot, atomic writes, regenerated `INDEX.md`,
    whole-bank validation, and `restore_files` on any failure. A missing
    successor is refused before the first write, so the error names the
    argument that caused it rather than surfacing later as a broken link.
  - It does not reuse `render_markdown_record`: that renders with
    `sort_keys=True` and re-expands every list, so a round trip would reorder
    frontmatter a human wrote and rewrite lines the retire never touched. A
    chunk-local parse/render keeps key order and leaves the body byte-for-byte
    intact.
  - `--reason` is recorded in the command's output only. The chunk schema is
    a closed key set, so a reason written into the frontmatter would fail
    validation and roll the whole retire back.
  - New `context.py bank-retire --id --valid-to [--superseded-by] [--reason]`,
    documented in `docs/OPERATIONS.md`, with the requirement to use it rather
    than hand-edit added to each edition's `memory-bank` skill — the same rule
    that makes the `memory` skill call the command the hook calls.
  - The readiness criterion's "the chunk is absent from the index" was
    corrected on the way in: `memory-bank/INDEX.md` must KEEP the row, because
    a chunk on disk without one is itself a validation error. What leaves is
    the retrieval index, at the next `index`/`refresh`, where the chunk is
    reported in `excluded` with its new status as the reason.
  - Fixed in passing: the Symfony and PHP Core copies of the `memory-bank`
    skill still told the agent to report "index/counter changes", naming the
    retired `.memory-counter` that the same file forbids touching. All three
    editions now say `reindex-bank`.

- **A retrieval gate decides whether a turn is worth retrieving for, and
  records the verdict (roadmap H2-01).** Restraint existed only at the
  document level — `informative_tokens`, `token_coverage`, `is_relevant` — and
  no aggregate turn-level decision existed, nor anywhere to write one. What a
  gate saves is bias rather than tokens: a pointer asserting "relevant right
  now" about the wrong file costs more than no pointer, because the agent
  opens it.
  - `gate_decision` in `context_retrieval.py` returns `{decision, mode,
    reason, signals}` and is called once the delivered selection is known —
    after the episodic-layer filter and the 2+3 truncation — because
    `selection_identical_to_previous_turn` is a claim about what the capsule
    carries. Deciding earlier would describe a set the capsule never had.
  - Two deterministic rules, both computable before a document body is
    opened: `no-match` (nothing survived relevance, so retrieving and
    skipping deliver the same thing) and `repeat-retrieval` (same distilled
    query and same selected path set as the previous turn of this task).
    The roadmap's proposed signal — a capsule repeating byte-for-byte —
    cannot occur: every call mints a fresh manifest id, and the rendered form
    carries an automatic-checkpoint sentence and a last-turn summary that
    both move on their own.
  - The previous turn is remembered as two hashes under one bounded
    `index_state` row, not one row per task: nothing prunes that table, so a
    key per task would grow for the life of the database. The write is
    best-effort against a read-only database, and a skip never overwrites the
    record — otherwise the turn after a skip would compare against nothing.
  - Signals: `informative_terms` and `top_score` were computed inside
    `_candidates` and discarded, so it now returns them alongside its
    candidates. `distinctive_matches` replaces the roadmap's
    `distinctive_terms`, which does not exist as a quantity — `token_coverage`
    decides rarity per token but returns paths — and is free from the match
    strength H1-04 already records. `top_score` is pre-filter, so a candidate
    withheld by policy stays visible, and is corpus-scaled telemetry, never a
    threshold.
  - Mode resolution follows the `configured_mode` ladder: `--gate`, then
    `CONTEXT_RETRIEVAL_GATE`, then `retrieval_gate` in `runtime.json`, then
    `shadow`. It is validated in `configured_retrieval_gate` rather than only
    by argparse, because `hook-context` never sees the flag.
  - **`shadow` is the default and is the whole point.** The decision is
    computed, written to the manifest and ignored; the turn is served either
    way. Enabling `enforce` is H3-05's decision on the data `shadow`
    produces — the same standard that kept embeddings out on measured
    evidence. `enforce` is implemented and unlit.
  - In `enforce` a skip withholds the selection, prints `gate: skipped —
    <reason>` after the `working:` render marker, and still writes a manifest
    with the verdict, because a withheld turn has to be countable or the skip
    rate H3-02 needs cannot be computed.
  - **The Cursor hazard is closed in the same change.** Both Cursor delivery
    hooks accept any stdout opening with `working:` at exit 0 and move it over
    `.cursor/rules/working-memory.mdc`, so a withheld capsule rendered
    normally would replace Cursor's only memory channel with an empty rule on
    every skipped turn. `hook-context` now exits 4 and prints nothing on a
    skip; both delivery constants document that any status other than 0 and 3
    preserves the existing rule, which is the behaviour a skip needs.
  - `gate` is declared under `properties` in
    `retrieval-manifest.schema.json` and made mandatory through the version-2
    key set in `brain_runtime`, not through the schema's `required` array:
    `validate_schema_value` has no conditional construct, so a `required`
    entry would reject version 1 manifests too. It joined version 2 rather
    than opening a version 3 because version 2 is itself unreleased — both
    land in the same change and no manifest was ever written to the
    intermediate shape.

- **A retired chunk now leaves the index on its own date, and every dropped
  document says why it was dropped (roadmap H1-02).**
  `docs/CONTEXT-AND-MEMORY.md` promised that closing a period removes a chunk
  from retrieval without deleting it. The incremental cache keyed on
  `(mtime_ns, size)` alone, so it did not: a chunk whose `valid_to` or
  `review_after` arrived while the file sat untouched was reused on every
  prompt indefinitely, and `validate.py` began failing at the same moment —
  the human saw a red Definition of Done while the agent went on reading the
  retired fact. The only automatic indexing path is the incremental one, so
  this was the state every session ran in.
  - `document_source_state` gains an `eligible_until` column holding
    `min(review_after, valid_to)`, and reuse now requires both an unchanged
    stat pair and an unexpired boundary. The table is created with
    `CREATE TABLE IF NOT EXISTS`, so `ensure_metadata_tables` adds the column
    to an existing database with the same `PRAGMA table_info` + `ALTER TABLE`
    pattern already used for `document_metadata`; a pre-migration row carries
    no boundary, cannot express expiry, and is re-validated once rather than
    trusted. No extra `stat` and no extra filesystem pass: the date comes
    from frontmatter that discovery already parses.
  - Discovery no longer drops documents on a bare `continue`. It returns an
    `excluded` list that `index_repository` merges into the channel
    `index_documents` already reports for Project Brain records, on both of
    its return paths — including the "nothing observable changed" early
    return, which is the one the per-prompt hook takes. Reasons: `retired`,
    `overdue-review`, `superseded` / `archived` / `needs-review`, `invalid`,
    `secret`. A chunk that reached its own end date is reported as retired
    rather than invalid, because "go do the review" and "go fix the
    frontmatter" are different instructions. The three structural skips
    (Git-ignored, second pattern on an owned file, mirrored skill copy) stay
    unreported — Symfony alone would contribute 279 of the last one.
  - `active_memory` returns the chunk's frontmatter instead of a bool
    (`None` when it may not be served, so every existing truthiness test
    still holds), and `memory_eligibility` exposes the reason and the
    boundary alongside it. `discover_documents` returns a fourth value.
  - Plain `index` output now prints a per-reason tally of what was excluded;
    `--json` carries the paths.

- **The four copies of the memory core were reconciled (roadmap H1-01).**
  Every divergence below was live at `cef813ee`:
  - `memory-bank/scripts/context.py`: the generator asset carried
    `_readiness_git_probe` / `automatic_memory_readiness` and their
    `status` wiring — 208 lines the three editions never received. The
    editions gain them, so all four copies are byte-identical again.
  - `memory-bank/scripts/telemetry.py` existed in the editions and not in
    the asset: generated projects were seeded with four of the five core
    modules. The asset gains it, `runtime-contract.json`'s
    `required_skeleton` now names it, and `memory-seed/SKILL.md` says
    five scripts rather than four.
  - `.claude/hooks/working-memory-read.sh`: Symfony and PHP Core cut the
    prompt to `re.findall(r"\w+", prompt)[:24]` before handing it to the
    CLI while Laravel passed it through. Truncating by position discards
    the input that `distill_capsule_query` ranks by rarity, so the same
    request produced a different capsule per edition. Both now pass the
    prompt through, and the `.codex` mirrors follow.
  - The asset's seeded content was stale against canon:
    `templates/chunk.md` still showed the pre-race-free `MEM-0000` chunk
    id, `project-brain/templates/{task,handoff}.md` lacked `phase`,
    `templates/bug.md` lacked its Root Cause / Same Shape Elsewhere /
    Guard sections, `templates/promotion.json` lacked `review_mode`, and
    `project-brain/README.md` was missing the capsule-contract, task-phase
    and promotion-mode paragraphs.
  - `project-brain/control/messages/` is required by
    `runtime-contract.json` and shipped by the asset, but no edition
    tracked a `.gitkeep` for it. All three now do, and the installation
    inventories were regenerated.

- **A developer's own Project Brain index no longer ships into an install.**
  The accelerator's runtime rewrites `project-brain/indexes/active.json` inside
  this repository whenever a task is opened here, and the records it then lists
  are this repository's own - untracked, and never part of the payload. The
  installer copies working-tree bytes for every inventory path, so a target
  received an index pointing at files it does not have, and its own
  `context.py validate` reported it stale. Measured on a working checkout:
  three clean-install subtests failed for that reason alone, with nothing wrong
  in any committed file, while a fresh clone of the same commit passed.
  `memory-bank/INDEX.md` had already met this problem and set the precedent, so
  both Brain indexes now install from a pristine
  `project-brain/.install/*.json` the same way. Inventory generation also stopped
  emitting an override for a path the edition does not install, since the
  override table is shared while editions differ. Covered by
  `tests/test_installation.py`, which dirties the index and asserts the target
  still receives `[]`.

- **The memory-bank validator can now be told which tree its chunks cite,
  so an honestly seeded bank stops failing for being staged.**
  `memory-bank/scripts/validate.py` resolved every chunk source against the
  bank's parent directory. That is right once the bank is published beside the
  project it describes, but during `infra-generate` the bank sits in a staging
  root while the files it cites live in the target - so every chunk reported
  "source path does not exist" and the publication gate refused a bundle whose
  only fault was not having been published yet. Seeding zero chunks passed;
  seeding the chunks `memory-seed` prescribes did not. `validate.py` gained
  `--source-root` (default unchanged: the bank's parent), and
  `validate_generated.py` passes the `--evidence-target` it already resolves,
  so the same bank is checked against the project it actually describes. The
  regression is covered in
  `Infrastructure-Creator/tests/test_memory_readiness.py`, which asserts both
  directions: staged-and-unaided fails, staged-and-told passes, and a published
  bank still needs no flag.

- **The write-capable agent lock no longer exempts a second instance of the
  same agent.** The gate blocked a different write-capable agent while one
  held the lock but let a same-named one through, so N concurrent `coder`
  runs all passed and each merely refreshed the lock - which is how three epic
  builds once ran at the same time against one repository. Any live holder now
  blocks. A genuine respawn is unaffected: `subagent-dispatch.sh` clears the
  lock when the holder finishes, so a fresh lock means the holder is still
  running, and a crashed run stays covered by `LOCK_TTL_MINUTES`. Applied to
  all four editions' `.claude` and `.cursor` copies by hand, because
  `subagent-gate.sh` is a documented `skip` in the hooks mirror class - the
  Codex copies block multi-agent spawning outright and hold no lock.
  `test_a_second_instance_of_the_same_agent_is_blocked` in each edition's
  `memory-bank/tests/test_hooks.py` covers it.

- **Inventory generation no longer reads the working tree.**
  `--write-inventories` discovered tracked *and* non-ignored untracked files,
  the same permissive walk `--verify-inventories` uses; one local run absorbed
  8586 untracked `vendor/` paths from a built application into an edition's
  distribution list, and the installer copies what the inventory names.
  Generation is now tracked-only, verification stays permissive - warning
  about a file not yet committed is the point there - and each written
  inventory reports its path count with the added and removed paths, so a
  wrong inventory is visible before it is committed rather than as an
  unreadable diff. Regenerating produces byte-identical output.
  `test_generation_ignores_untracked_working_tree_files` covers it.
- `scripts/build_mirrors.py` now runs its reverse stray pass over
  `only`-classes too (the governance documents: `DOD.md`,
  `GOLDEN-PRINCIPLES.md`, `STABILIZATION.md`). Previously a deleted
  canonical governance file with surviving mirrors was silently skipped:
  deleting `.claude/DOD.md` left `--check` flagging only the stale
  `.gitattributes`, and once `--write` refreshed that manifest the orphaned
  `.cursor/DOD.md` and `.codex/DOD.md` would have persisted indefinitely as
  canonical-looking files no rule accounts for. The pass examines only the
  listed names, so canonical files other classes own inside the same mirror
  directory are untouched; behavior for every other class is unchanged.
  A listed `only` entry whose canonical file does not exist is now itself a
  reported problem: previously `iter_canonical` skipped it silently, so a
  typo in the list (or a canonical file renamed after mirrors were
  generated) mirrored nothing while `--check` stayed green and the
  orphaned mirrors persisted. Pinned by the new
  `tests/test_build_mirrors.py`, which the `mirrors` CI job now runs
  alongside `--check`.

- `scripts/token_budget.json` re-baselines the Infrastructure-Creator
  ceilings, which the 2.5.0 evidence-contract work outgrew without moving
  them - the `lint` job had been red on every commit since. Per the file's
  own policy (ceiling = observed + ~5%, raised only with the change that
  justifies the growth), all six byte ceilings are re-derived from the
  current tree: `body_bytes` 176549 -> 220441 and `agents_md_bytes` 12720 ->
  15673 admit the new evidence, contract and semantic-gate procedures, while
  `descriptor_bytes` 12478 -> 10811, `agent_bytes` 10342 -> 8134 and
  `frontmatter_bytes` 18078 -> 16499 are *tightened* onto the slimming 2.5.0
  performed, so the gate keeps its grip instead of inheriting dead slack.
  Net effect on the surface that matters most: the startup total - paid on
  every session whether or not anything is invoked - is 823 B *smaller* than
  before 2.5.0 (34648 B -> 33825 B); the growth is confined to skill bodies,
  which are paid only on invocation. `skills` stays at 25.

- **`scripts/install_accelerator.py` no longer leaks untracked working-tree
  files into the shipped inventories.** `discover_distribution_files` listed
  `git ls-files --cached --others --exclude-standard`, so *anything* sitting
  in an edition directory was classified and written into
  `install/inventories/*.json` by `--write-inventories`. On a working tree
  holding a real client application under `Task/app/` that pulled roughly
  19700 lines - including `.env` and `var/cache/dev/**` - into `symfony.json`
  and `laravel.json`, files that are distributed with the repository. The same
  scan made `--verify-inventories` (the `installation` CI gate) fail with
  `UNCLASSIFIED` records on any tree with untracked files. Discovery is now
  `git ls-files --cached`: the inventory is a closed contract over the index,
  which is what "tracked-file contract" already claimed. Staging is enough to
  register a new distribution file (`git add`, no commit); an unstaged file is
  not part of the payload. Outside a Git checkout - or when the source root
  holds no tracked files for an edition - both modes now fail with an explicit
  error instead of falling back to a filesystem walk, because off the index
  there is no way to separate distribution files from client data. Writing is
  also staged in memory and only then flushed, so a failure on the third
  edition no longer leaves a half-written inventory set.

- **`--write-inventories` accepts `--inventory-out DIR`**, writing the
  generated inventories somewhere other than the checkout's
  `install/inventories`. `tests/test_installation.py` used to assert
  determinism by regenerating *in place* over the live repository and
  comparing bytes: with the leak above, running the test suite silently
  rewrote the committed inventories with client paths, and the assertion
  reported the damage only after it was done. The test now regenerates into a
  temporary directory, compares that against the committed files, and asserts
  the checkout was left untouched. A new `UntrackedSourceTest` builds a
  synthetic staged checkout, plants untracked `.env`, `Task/app/.env` and
  `Task/app/var/cache/dev/**` files in it, and pins that they reach neither
  the generated inventory nor its verification - plus that generation outside
  a Git checkout fails loudly and writes nothing.
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
