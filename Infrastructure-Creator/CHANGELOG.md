# Changelog

All notable changes to Infrastructure-Creator are documented here. Format loosely follows [Keep a Changelog](https://keepachangelog.com/); versions are for this generator tool, not for anything it generates. The released version has a single machine-readable source: the root `VERSION` file - profiles, `AGENTS.md` stamps, and `.infra-manifest.json` files all read it.

## Unreleased

### Fixed

- `CommandAnalyzer` raised on the stock Symfony skeleton, so the quality gate
  could not run at all on a normal Symfony target. Flex writes `auto-scripts`
  as an object keyed by command (`{"cache:clear": "symfony-cmd"}`), and
  `_normalise_script_commands` accepted only a string or a string list - it
  raised, `validate_skill_quality.py` turned that into
  `COMMAND_ANALYZER_UNAVAILABLE`, and every skill in the inventory failed.
  All seven real Symfony projects available for measurement carry the object
  form, so this was not an edge case: it made generation impossible on the
  generator's own primary target ecosystem. The object form is now read, with
  the command reconstructed from the key according to its handler
  (`symfony-cmd` -> `bin/console <key>`, `php-script` -> `php <key>`), and a
  script object with non-string values is still rejected. Found by running
  the pipeline end to end against a real project rather than a fixture.

- The skill-quality gate held the *shape* of a generated skill - contract
  completeness, candidate registry, sha256 fingerprints, path existence - and
  lexical duplication, but not its *meaning*. Seven reproduced ways to pass it
  with an unusable or unsafe skill are now closed; the entries below describe
  each one. Throughout, calibration was measured against honest corpora rather
  than guessed, because a gate that fails a good-faith generation breaks the
  generator outright, which is worse than the miss it closes.
- Command safety stopped at the plan's `verification[].command`, so a
  generated `SKILL.md` could hand the agent `rm -rf var/cache/dev` or
  `curl -X POST https://billing.internal/api/invoices/void` in its prose and
  still pass. Skill bodies are now analyzed too: fenced `bash`/`sh`/`shell`/
  `console` blocks strictly (transcript prompts stripped, continuations
  joined, comments dropped) and inline backtick spans permissively (a segment
  counts only when it names a known executable *and* carries an argument, and
  a bare English-word executable only when path-qualified). Chains are split
  at quote-aware separators and classified leaf by leaf, homoglyph spaces are
  folded first, and the argument of a code host (`bash -c`, `php -r`) is
  re-read as a nested command. Destructive, workspace-mutating,
  provider/network, or `sudo` behavior blocks (`SKILL_BODY_COMMAND_RISK`);
  attestability-only findings (unknown executable, unresolved alias,
  tokenization) do not, because prose legitimately carries placeholders and
  sample output. That split is the measured one: on 93 hand-written
  accelerator skills the naive "anything not provably safe" policy fired ~100
  times on the 48 Symfony skills alone and was almost entirely noise
  (`<number>` placeholders, heredoc fragments, JSON payload strings, `EOF`),
  while the shipped policy fires 64 times and every hit is a real mutating or
  networked command.
  Two defects in that first cut were found by review and closed before it
  shipped. Prose polarity is now read: a guardrail names the command it
  forbids, so `Never run \`rm -rf var/\`` - the safest sentence a skill can
  carry - was the one sentence that failed the gate. A backtick span whose
  own clause carries a prohibition (`never`, `do not`, `must not`, `avoid`,
  `instead of`, `tempted to`, ...) is read as a mention, not a prescription;
  the window stops at the previous clause boundary so one prohibition cannot
  silence a section, a lone hyphen is deliberately not a boundary (it would
  truncate the window at "read-only" and hand the false positive back), and a
  fenced shell block stays an instruction whatever the surrounding prose
  claims. Second, the segment walk was LIFO against a 256-segment cap, so a
  chain longer than the cap dropped its *head* - exactly where padding hides
  a dangerous command; `rm -rf var/important && <300 harmless echoes>`
  validated clean. The walk is now breadth-first over the written order, and
  reaching the cap is itself reported (`SKILL_BODY_COMMAND_UNSCANNED`)
  instead of silently truncating: an unscanned tail is an unproven command,
  and unproven fails closed.
- `evidence_anchors[].anchor` was validated for shape only, so an anchor could
  cite line 7400 of a 60-line file or a symbol that does not exist in the
  class it names. Anchors now resolve against the cited file - `L` ranges must
  run forwards and lie inside its real bounds (`EVIDENCE_ANCHOR_RANGE`), and a
  `symbol:` anchor's last segment must occur in it
  (`EVIDENCE_ANCHOR_SYMBOL_ABSENT`, case-insensitive whole-identifier match so
  `Ns\Class::member` resolves through `member`) - the same bound
  `evidence[].line_range` already carried. An unreadable source is reported
  (`EVIDENCE_ANCHOR_UNRESOLVABLE`), never raised.
- Ownership was only ever compared writes-against-writes, so a skill could
  write into a zone another skill holds under `mode: exclusive` whenever that
  owner was read-only - which, by contract, every reviewer is
  (`OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT`). `shared`/`composed` zones keep their
  own rules. Separately, a declared primary/defer precedence silenced overlap
  unconditionally, so two skills could claim the same `owned_scope` entry,
  `ownership[].description`, or positive trigger *word for word* and still
  pass. Verbatim repetition is now `BOUNDARY_NOT_SEPARATING` regardless of
  precedence - a boundary that repeats itself divides nothing - while nested
  or otherwise partial overlap under an explicit precedence stays legitimate.
- A skill could carry the project's real paths, classes and constants in every
  sentence and still instruct nothing, passing every lexical traceability
  check. Procedure steps are now read structurally: a step must command an
  action (`SKILL_STEP_NOT_OPERATIONAL`) and must not open by deferring to
  reflection - "consider", "bear in mind", "form an opinion"
  (`SKILL_STEP_HEDGED`); the procedure as a whole must name a concrete anchor
  (`SKILL_PROCEDURE_UNANCHORED`); and verification must state a check rather
  than an impression (`SKILL_VERIFICATION_NOT_FALSIFIABLE`,
  `SKILL_VERIFICATION_NOT_OPERATIONAL`). The plan is read the same way
  (`PROCEDURE_STEP_NOT_OPERATIONAL`, `PROCEDURE_STEP_UNANCHORED`,
  `VERIFICATION_NOT_FALSIFIABLE`). The `GENERIC_PHRASES` blacklist is demoted
  to a secondary net, since one paraphrase defeats it. Calibration is
  deliberately loose where it can only cost honest skills: the verb set is
  open and matched anywhere in the step, mid-sentence hedging next to a real
  action stays legitimate, and the anchor is required per procedure and never
  per step, so honest branch and delegation steps survive.
- The action-verb set that gate reads against was too narrow on arrival and
  rejected honest instruction: 5 of the 37 procedure steps in the adversarial
  harness's *honest* five-skill baseline were failed as commanding no action
  ("Bound delivery at three attempts.", "Downgrade only on a permanent
  delivery boundary error.", "Not this skill: recompute the anniversary
  anchor of a subscription cycle."), and a step reading "Accept transitions
  that revive a cancelled slot" was reported as inert rather than as wrong.
  The set is widened by two stated rules - a verb enters when a measured
  honest step used it, and a verb whose counterpart is already listed
  ("upgrade"/"downgrade", "compute"/"recompute", "reject"/"accept",
  "stop"/"start", "bind"/"bound", "modify"/"change") enters with it - stopping
  at words that ordinarily read as adjective or noun, so "close" stays out and
  "the closed invoice" keeps reading as a bare noun list. The honest baseline
  is clean again at 0 findings over 271 distinct steps of every corpus
  available.
- Skill deduplication in `validate_skill_quality.py` was advertised as
  semantic but compared normalized lines for *equality*, so it only ever saw
  byte-identical prose - and since a plan-conforming `SKILL.md` must carry its
  own claim, paths and neighbours' names, those mandated differences diluted
  the score below the fail lines even for a byte clone of the procedure
  (measured on the fixture: clone line 0.500 / token 0.680 against thresholds
  0.70 / 0.80; one template with the project's nouns substituted 0.100 /
  0.351; the same template differing only in `,`->`;` and `and`->`plus`
  0.000 / 0.324 - indistinguishable from two honestly different skills).
  A second pass now compares a *skeleton*: backticked spans, paths, CamelCase
  and `UPPER_SNAKE` identifiers, dotted ids, PHP variables and calls, numbers
  and the inventory's other skill names collapse to one placeholder, fenced
  code and approved fixed blocks are dropped, punctuation and interchangeable
  connectives are folded, and lines that are almost entirely project identity
  are ignored; what remains is scored by one-to-one line matching plus token
  trigrams (`SKILL_TEMPLATE_REUSE`, error at 0.38 / 0.26, warning at
  0.28 / 0.20). Thresholds are measured, not guessed: across 2371 pairs of
  honest hand-written skills the worst pair scores 0.350 / 0.154 and is a
  deliberately parallel scanner family, while the three duplicates above
  score 0.400 / 0.329, 0.400 / 0.311 and 0.500 / 0.471. `REPEATED_BLOCK`'s
  three-consecutive-identical-lines rule drops to two adjacent skeleton lines
  as `SKILL_TEMPLATE_BLOCK`, kept a *warning* because on the same honest
  corpus it fires five times on legitimately shared policy sentences.
- An `evidence[].supported_claims` entry was "grounded" by a bag-of-words
  overlap with the cited range: two shared tokens (one for a claim of three
  tokens or fewer) after subtracting five service words. `class` occurs in
  75% of real PHP files, `function` in 77%, `public` in 74%, `string` in 56%,
  so an invented claim - "the public class exposes a private function that
  returns a string value from the configuration array" - was grounded by any
  PHP file it pointed at. Support now has to come from vocabulary that
  distinguishes *that* range: a claim sharing nothing outside PHP-keyword and
  licence-header lexicon is `EVIDENCE_CLAIM_UNSUPPORTED` (error), and one
  sharing nothing outside software-English boilerplate (`service`, `method`,
  `value`, `result`, `config`) while at least 65% of its own vocabulary is
  such boilerplate is `EVIDENCE_CLAIM_GENERIC_SUPPORT` (warning). The cited
  side is matched with compound identifiers split into their parts, so
  `publishReminder` grounds an honest claim about the "reminder" the file
  never writes on its own. Both lexicons are unedited document-frequency cuts
  (share >= 0.40 and >= 0.04) over 3997 real PHP files from the Symfony and
  Laravel vendor trees, measured in that same identifier-split token space.
  Calibration on 1485-2190 honest docblock/code pairs from those trees: the
  error tier costs 0.18-0.27% false positives and catches 20-45% of
  generic-lexicon fabrications, the warning tier costs 2.3-3.6% and takes the
  pair to 87-99%; on the harness's honest control claims the margin is 3-10
  project-specific tokens matched against a threshold of one. The second tier
  warns rather than blocking because at that false-positive rate a failed
  generation would cost more than the miss.
  What no lexical rule can reach is *polarity*: a claim asserting the opposite
  of the code it cites shares all the same vocabulary. That gap, the options
  weighed for it (an LLM judge inside the gate, doing nothing, this
  deterministic narrowing plus adjudication in the repo-root `harness/`), and
  the decision are recorded in `docs/ADR-001-claim-adjudication.md`.
- Six contradictions between the canonical LLM-prompt documents and the
  shipped validators, each capable of steering an obedient agent into a
  blocking gate or leaking non-neutral fixture data:
  - `infra-generate` step 2 demanded a top-level `routing_cases[]` in
    `skill-generation-plan.json`, but the schema's top-level membership is
    exact and `validate_skill_quality.py` blocks any extra field with
    `PLAN_FIELD_UNKNOWN`; the step now requires the per-skill
    `routing_cases[]` (`skills[].routing_cases`) the validator actually
    checks.
  - `command-forge` step 8 read as an in-skill instruction to run
    `validate_flow_contracts.py`, yet `skill-flow-composer` runs after
    command-forge, so `SKILL FLOW.md` cannot exist and the validator exits 1
    on the missing artifact; the step now states compiled-graph validation
    is orchestrator-owned (`infra-generate` step 8 after the composer), must
    not run inside command-forge, and is recorded as pending that gate.
  - The `command-forge` frontmatter guardrail ("Cursor command only `name`,
    `description`") carried no flow-command exception while step 7 and the
    flow guardrail require `flow` + ordered `stages` frontmatter and one
    fenced `json flow-contract` block in every selected command-carrying
    edition; the guardrail now names flow commands as the sole exception, so
    a literal reading no longer guarantees failing
    `validate_flow_contracts.py`.
  - The `project-profile-schema.md` exemplar skill stamped
    `"phase": "execution"`, outside the fixed vocabulary (`understanding`,
    `planning`, `implementation`, `verification`, `finalization`) that flow
    stages and the composer's Phase Map accept; the example now uses
    `implementation`.
  - Adjacent `AGENTS.md` bullets contradicted each other on schema 1.1
    approvability; both now state the policy DOD.md, the schema doc, and
    `LEGACY_PLAN_PUBLICATION_INELIGIBLE` enforce: only a schema 1.2 plan is
    approvable (1.2 carries the ownership/routing structures introduced in
    1.1), and legacy 1.0/1.1 plans stay audit-readable but must be
    re-synthesized.
  - The `critical_invariants` example in `project-profile-schema.md` leaked
    a real project's domain (`content-job.failure-terminal` /
    `contentjobs-lifecycle-review`); it now uses the neutral acme-billing
    fixture family (`invoice.paid-immutable`, `billing-rules-review`) like
    every other example, preserving the example's structure.

  Pinned by the new `tests/test_doc_contracts.py`, which parses the
  documents and cross-checks them against the validators' actual field
  sets, phase vocabulary, and diagnostics instead of trusting prose.

- `analyze_commands.py` no longer fails open: unknown executables, `sudo`/
  `env`/`timeout`/`nice`/`nohup`/`stdbuf` wrappers (unwrapped by basename),
  `xargs` and `$VAR` indirection, clustered `-lc` interpreter flags, shell
  invocations of script files, git global options before the subcommand,
  `php bin/console` / `symfony console` database commands, and
  `npm i`/`ci`/`npx`/`dlx`-family runners all classify or block instead of
  reporting `verification_safe=true`. Blocked commands now carry an explicit
  `verification_blocker` category instead of `non_mutating`. False positives
  fixed (`ruff check .`, `gofmt -l .`, `pytest -W error`); alias expansion
  is bounded (512 walks, fail-closed `EXPANSION_LIMIT`); unknown bare
  `yarn`/`pnpm`/`bun` scripts fail closed as `UNKNOWN_ALIAS`; direct
  `composer <builtin>` classifies the builtin, not a shadowing script.
- `publish_staging.py` refuses to write outside its contract: publication
  plans must be members of the staged manifest, removal plans members of
  the target manifest, and non-ownable runtime state (memory-bank/
  project-brain) is rejected before any mutation. Rollback now removes
  directory chains publication created and, using post-publish content
  hashes recorded in the journal, skips (and reports) files edited by third
  parties instead of silently clobbering them. `classify_update` no longer
  reports the staged `.infra-manifest.json` as a `new-file-collision`, and
  malformed manifest/decision entries raise clean `OwnershipError`s.
- `merge_gitignore.py` emits positive patterns before `!` negations so
  re-includes survive git's last-match-wins, and a requirement that would
  silently override a team `!entry` (or vice versa) is now a reported
  conflict requiring an explicit decision.
- `validate_skill_quality.py` diagnoses wrong-typed plan fields instead of
  crashing (five reproduced crash sites; `--json` always emits its
  payload), accepts the shipped five-key `candidate-registry.json`
  (previously every default-registry `--skill-plan` run failed
  `REGISTRY_INVALID`), no longer corrupts similarity metrics via the
  `profile` normalization regex, and flags per-trigger routing collisions
  that whole-set Jaccard diluted below threshold. Also fixed: unescaped
  skill names in regexes, `_globs_intersect` false positives on disjoint
  patterns, glob matches escaping the target through symlinks, the dead
  pre-`resolve()` symlink guard, trailing sentence punctuation in
  traceability tokens, and YAML folded/literal description parsing.
- `validate_flow_contracts.py` reports structured errors instead of
  raising on malformed routing fields and null stage `agents`, and command
  file-set discovery follows declared flow names instead of a hardcoded
  `flow-*.md` glob. `validate_generated.py` reports one placeholder error
  per file/pattern. `validate_reference_catalogs.py` rejects empty
  candidate registries and reports unreadable catalogs per file instead of
  crashing.
- Memory readiness in the seeded `context.py` distinguishes transient git
  probe failures (`git-probe-failed`) from detached HEAD and makes
  `unborn-head` genuinely detectable; the workflow-smoke repeatability hash
  now covers recomputed outputs instead of constants.
- CI: the reliability job is pinned to Python 3.9 (its 3.x leg duplicated
  the tests-matrix and mirrors jobs), and `build_mirrors.py --check` now
  detects stale mirrors of deleted canonical `only`-class files (pinned by
  the new root `tests/test_build_mirrors.py`).

## [2.5.0] - 2026-08-14

### Added

- Machine-readable `skill-generation-plan.json` with a validated evidence
  ledger and one complete necessity, scope, procedure, verification, output,
  failure, write, and routing contract per proposed skill.
- Dependency-free semantic gates for plan/evidence integrity, fingerprints,
  contract-to-skill traceability, target-relative citations, ownership
  collisions, routing ambiguity, substantive duplication, approved versioned
  fixed blocks, agent routing, and conditional specialist flows.
- Transactional staged publication with explicit publication/removal plans,
  target-drift detection, manifest-last writes, team-file preservation, and
  rollback for generation and updates.
- Reference-catalog validation and regression fixtures for generic/duplicated
  skills, paraphrased filler, invalid or irrelevant evidence, stale
  fingerprints, scope/routing collisions, reference stubs, merge preservation,
  and failed-publication rollback.
- Early complete-inventory validation, safe partial skill-batch validation, and
  schema 1.1 structured ownership/reciprocal routing with schema 1.0 migration.
- Deterministic shared root `.gitignore` requirement composition, additive
  decision metadata, and watch-only target drift protection.
- Exact immutable-memory placeholder declarations and deterministic workflow,
  parity, shared-file, publication, and rollback regression coverage.
- Publication-gated schema 1.2 operational contracts for typed procedures,
  concrete verification, provider safety, path authority, critical invariants,
  evidence anchors, realistic routing cases, and canonical flow graphs.
- Dependency-free command risk analysis for Composer/npm aliases and explicit
  automatic-memory readiness reporting.

### Changed

- Skill inventory selection is evidence-gated instead of quota-driven. Only the
  memory quartet remains unconditional because its runtime is always installed.
- `skill-forge` authors one skill or a small bounded sibling group at a time
  from the minimum evidence slice; line padding and grouped category generation
  are invalid.
- Agents derive positive/negative selection, sibling deferrals, expected
  results, and write capability from validated contracts. `flow-feature` and
  `flow-review` route specialists conditionally rather than selecting every
  available reviewer.
- `infra-generate` and `infra-update` verify the complete staged bundle before
  publication and run the full gate again on the published target.
- Forges declare root-ignore requirements instead of mutating `.gitignore`;
  generation/update orchestration is the sole composer and requires explicit
  approval when a pre-existing team file needs appended entries.
- Legacy schema 1.0/1.1 plans remain audit-readable but require re-synthesis
  before generation or publication; safety-critical fields are never guessed.
- Skills, wrappers, `SKILL FLOW.md`, and executable flows compile from one
  validated routing/flow contract and must preserve every material adjacency.
- `stack-adapter` must carry the complete contract compiler, semantic/reference
  validators, staging/rollback helper, fixtures, and synthetic rehearsal into
  every future sibling generator.

### Breaking

- Profiles created before 2.5.0 do not contain per-skill generation contracts.
  Re-run `infra-scan` before generation/update instead of reconstructing
  authority from previously generated prose.

### Orchestration and context changes

- **Generated accelerators reach orchestration parity with the hand-built
  editions.** The seeded context-brain runtime was a stale fork — no agent
  channel at all, no `hook-context`/`rebind`/`reindex-bank`, and a
  four-value phase vocabulary the engine stopped using — so
  `memory-seed`'s bundled `scripts/` and `project-brain/` skeleton are
  re-synced from the canonical runtime, gaining `msg-send` / `msg-read` /
  `msg-dispatch`, `capsule --validate`, `update --actor`, the forward-only
  phase guard, `message.schema.json`, and `control/messages/`. On top of
  that: `agent-forge` emits `writes: true` for write-capable agents in both
  editions (without it the generated gate's serialization never engages),
  `command-forge` generates `flow-feature` and `flow-review` composed only
  of agents the run actually produced, `hook-forge` adds
  `subagent-dispatch.sh` with its `SubagentStop`/`subagentStop` wiring
  (shipped unregistered on Codex) plus the gate's write serialization,
  `policy-forge` writes an Orchestration section into the generated
  AGENTS.md, and `bootstrap-verifier` / `validate_generated.py` enforce the
  eight-hook contract and the flow contract — stages naming generated
  agents, at most one write-capable agent per parallel stage, a checkpoint
  in every multi-stage flow.

- **Generated accelerators now restrict subagents to their own roster.**
  `hook-forge` produces a seventh hook, `subagent-gate.sh` - three
  tool-owned variants (Claude PreToolUse exit codes against the generated
  `.claude/agents` roster, Cursor `subagentStart` permission JSON with
  `failClosed`, Codex spawn_agent-family deny) - and wires the
  configuration half per edition: `Agent(...)` deny rules plus the
  built-in-agent env keys in `.claude/settings.json`, `subagentStart` in
  `.cursor/hooks.json`, `multi_agent = false` and `[agents] enabled =
  false` in `.codex/config.toml`. `policy-forge` adds a Subagents section
  to the generated `AGENTS.md`; `bootstrap-verifier` and
  `validate_generated.py` enforce the seven-hook contract (the gate is
  exempt from byte-identity by design).

- `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH=1` joins `.claude/settings.json`:
  the generator's flows orchestrate from the main conversation, so nested
  subagent trees add cost without oversight. The tool-owned
  `subagent-gate.sh` copies are refreshed to the monorepo's Stage C
  versions; the new write-serialization logic is dormant here until an
  agent declares `writes: true`.

### Additional additions included in this release

- Agent `<example>` blocks moved out of `description:` frontmatter into a
  `## Selection examples` body section, and `agent-forge` now requires the
  same of every accelerator it generates. A description is loaded into the
  orchestrator's context on every session whether or not the agent is spawned,
  and the embedded examples were about two thirds of those bytes; the forge
  previously prescribed embedding them, so each generated accelerator
  inherited the cost. The generator's own 23 agents shed 15,866 bytes of
  description. Nothing is lost - the blocks move verbatim into the body.

- **The generator's own tool configs now restrict subagent spawning to the
  project roster.** A tool-owned `subagent-gate.sh` joins each hooks
  directory — the one deliberate exception to the byte-identical hooks
  invariant, skipped in `mirror_rules.py`, because each host exposes a
  different gate contract (Claude PreToolUse exit codes, Cursor
  `subagentStart` permission JSON, Codex `spawn_agent` deny). Configuration
  backs the hooks: `Agent(...)` deny rules and
  `CLAUDE_CODE_DISABLE_EXPLORE_PLAN_AGENTS=1` in `.claude/settings.json`,
  `[features] multi_agent = false` plus `[agents] enabled = false` in
  `.codex/config.toml`, a `subagentStart` entry in `.cursor/hooks.json`, the
  `subagent-policy.mdc` Cursor rule, and an AGENTS.md "Subagents" section.
  Covered by `SubagentGateTest` in `tests/test_hooks.py`.

## [2.0.0] - 2026-08-07

### Changed

- **Manifest membership is now the only ownership authority.** Full and merge
  generation use an explicit write plan instead of walking managed target
  roots; `bootstrap-verifier` validates and scans placeholders only in tracked
  generated files, leaving unmanifested team policies, skills, hooks, memory,
  and Project Brain files untouched. The dependency-free ownership helper also
  provides deterministic update classification, legacy-manifest refusal, and
  caller-relative/absolute path handling, with standard-library regression
  coverage including a generated ownership lifecycle fixture.
- **Hooks joined the hardened generation.** `bash-validator` extracts the
  command through the jq/php/python3 chain instead of a greedy `sed` (a
  payload that fails to parse is no longer matched raw, which ends false
  blocks on prose mentioning dangerous commands) and warns on stderr when no
  extractor is available; `loop-detection` and `local-context` namespace
  their counters by a repository hash and reset them on SessionStart; the
  `.codex` hooks exit early on unrelated tool names, so validators no longer
  run - or count edits - on read-only tool calls. The hook layer gained its
  first regression suite (`tests/test_hooks.py`).
- **Canonical hook input no longer depends on `cat`.** Bash builtins consume
  complete multiline payloads before extractor selection; an extractor-less
  environment produces exactly one sanitized fail-open warning, and blocked
  diagnostics never disclose the command body.
- **Mirror rules became declarative.** The generator's own
  `.claude`/`.cursor`/`.codex` mirrors are regenerated by the monorepo's
  `scripts/build_mirrors.py` from the rule table in `mirror_rules.py`
  instead of being copied by hand.
- **Generated Cursor targets receive the Task Capsule.** `hook-forge` wires
  the same `alwaysApply` rule delivery (`.cursor/rules/working-memory.mdc`,
  gitignored in the target) that the flagship editions use, so Cursor
  installations of generated accelerators read working memory one turn
  behind instead of not at all.

## [1.4.0] - 2026-08-02

### Added

- **The generator's output is now upgradeable in place.** The new root `VERSION`
  file is the single source of the generator's version. `infra-generate` now
  finishes by stamping the first line of the target's `AGENTS.md`
  (`<!-- Generated by Infrastructure-Creator vX.Y.Z | TASK-N | date -->`) and
  writing `.infra-manifest.json` at the target root: generator version, source
  profile/task, selected editions, and the sha256 of every generator-owned
  file (runtime state - memory chunks, `INDEX.md`, counters, `local/` dirs,
  Project Brain records and indexes - is deliberately untracked; it belongs to
  the target team from the moment it is seeded). A new 23rd skill,
  `infra-update` (+ `/infra-update` command + agent, the fifth sanctioned
  orchestrator), consumes that manifest: it re-validates the profile,
  regenerates into a staging directory inside this workspace, and then applies
  a strict three-set triage - files whose hash still matches the manifest are
  safely replaced with their new versions; files the team edited (or deleted)
  are never overwritten and land in a "requires decision" report with
  three-way context (as generated / as the current generator would write it /
  as it is in the target); files with no manifest record are never touched.
  Every update run ends by rewriting the manifest and re-running
  `bootstrap-verifier`. Targets generated before manifests existed (v1.3.x
  and earlier) make `infra-update` abort with legacy-target recovery options
  instead of guessing file ownership. `bootstrap-verifier` /
  `validate_generated.py` now verify the contract itself: manifest presence,
  schema, self-exclusion, no state tracking, every tracked file existing with
  a matching hash, full coverage of the generated surface, the `AGENTS.md`
  stamp agreeing with the manifest, equality with the generator's `VERSION`
  right after a run, and a hash refresh after any content auto-fix.
- **Generated accelerators now receive the full context-brain layer**, closing
  the gap to the monorepo's hand-built PHP editions:
  - `memory-seed` bundles and copies verbatim the dependency-free runtime
    (`context.py`, `brain_runtime.py`, `context_retrieval.py`, `validate.py`
    into `memory-bank/scripts/`) and the governed `project-brain/` skeleton
    (`PROTOCOL.md`, `README.md`, `.gitignore`, five schemas, nine record/control
    templates, `scripts/validate.py`, empty indexes, config, and `.gitkeep`-held
    record directories). `config/runtime.json` is materialized from
    `runtime.json.template` with a single `{{TARGET_FRAMEWORK}}` substitution -
    the target's confirmed framework slug, or `generic` when none is confirmed.
    Unknown slugs are safe by design: the runtime's only framework-keyed lookup
    (`MIRROR_RULES.skip_by_framework`) yields no exemptions for unknown keys,
    and `brain_runtime.py` defaults to `generic`.
  - `hook-forge` now specifies six hooks instead of four: the two working-memory
    hooks (`working-memory-read.sh` on `UserPromptSubmit`, calling
    `context.py refresh` with a bounded ephemeral query; `working-memory-write.sh`
    on `Stop`, calling `context.py turn --flush-after`) join the four enforcement
    hooks, with wiring for Claude (`settings.json`, second timeouts), Cursor
    (`hooks.json` `stop` only - Cursor has no prompt-time event, a documented
    divergence), and Codex (`hooks.json` with `additionalContextLimit`, plus
    `config.toml`). Loop-detection tracking is namespaced per repository by a
    hash of the repo root.
  - `skill-forge` generates 18 process/workflow skills instead of 15: the new
    `project-brain`, `checkpoint`, and `memory` cards complete the memory
    quartet around `memory-bank`, authored against the shipped runtime per the
    new "Memory Quartet" contract in `references/php-process-skills.md`.
  - `bootstrap-verifier`/`validate_generated.py` gained delivery smoke checks:
    `context.py status` and `context.py validate` must exit 0 in the generated
    tree; the per-edition hook set must be complete; every hook wiring file may
    reference only existing, executable scripts (closing the "dead hooks" bug
    class); the `project-brain/` skeleton and substituted `runtime.json` are
    verified; and the memory quartet skills (plus wrappers) are required per
    edition. The seeded bank's validator is now invoked with its positional
    bank argument, matching the shipped validator's CLI.
  - The Project Profile schema's sections 11.1/11.2/12, consumption contract,
    and validation rules now describe the honest delivery: 18 process skills,
    the two-root memory layer, and the framework-slug substitution.

### Fixed

- **The generator's own `bash-validator.sh` allowed irreversible GitHub
  operations.** It blocked broad `rm -rf`, force-push, hard reset, `--no-verify`,
  `.env` access, and SQL `DROP`, but not `gh repo delete`, `gh repo archive`,
  `gh issue delete`, `gh release delete`, or `gh api ... DELETE` - all of which
  the three PHP editions block. This generator only ever reads a target project,
  so none of them has a legitimate use here. Added them as `case` rules,
  matching the script's existing style; its shell-glob matching never had the
  `grep --` defect the PHP editions did, so the rest is unchanged.
- **`hook-forge` specified broken Claude wiring, so every generated accelerator
  inherited dead hooks.** Step 6 told the generator to emit
  `echo '$TOOL_INPUT' | <script>` with millisecond timeouts, and assigned
  loop-detection to `PreToolUse` and file-naming-validator to `PostToolUse` -
  the reverse of this generator's own `.claude/settings.json`. Claude Code
  delivers the payload on stdin, so the `echo` form passes the literal string
  `$TOOL_INPUT` and the hook exits 0 on every call; `bootstrap-verifier` only
  checks `bash -n` and the executable bit, so it reported success anyway.
  Corrected the wiring spec (bare script path, second timeouts, correct
  event/matcher pairs) and added guardrails requiring `grep -Eqi --` and JSON
  decoding rather than `sed` scraping in every generated `bash-validator.sh`.
- **This generator's own hooks were wired the same broken way.** Applied the
  same fix to `.claude/settings.json`: bare script paths, timeouts in seconds.
  Its `bash-validator.sh` matches with shell `case` globs rather than `grep`,
  so the script itself was unaffected and is unchanged.

## [1.3.5] - 2026-07-27

### Added

- New Claude `domain-behavior-scanner` skill and agent, expanding Phase 1 from six to seven scanners. It discovers project-specific sources of truth, domain vocabulary, core entities, business invariants, proven lifecycle transitions, roles/permissions, audit obligations, high-risk workflows, critical regression scenarios, sanitized incident lessons, and cohesive domain-skill candidates.
- Behavioral findings preserve both confidence and source type and surface contradictions; statuses are not treated as transitions, observed enforcement is not treated as a complete permission matrix, and risk indicators do not invent severity/owners/approval.
- Project Profile schema now includes section 8 (Domain & Behavioral Contract), section 11 generation preview, and section 12 cohesive memory preview.
- New `php-domain-behavior.md` generation reference. Existing requirements/architecture/API/database/testing/review/security/debugging/documentation skills are enriched first; separate domain skills are generated only for cohesive confirmed bounded contexts.
- The operational `memory-bank` skill is now always generated and counted alongside the shared bank created by `memory-seed`.
- Memory seeding now groups confirmed facts into cohesive concepts and links canonical sources instead of producing one small chunk per fact or duplicating specs/schemas/tests.

### Changed

- Mirrored the complete 22-skill implementation to Cursor (`.cursor/skills`) and Codex (`.agents/skills`), including the domain scanner, profile schema, generation references, memory behavior, and verifier updates.
- Added the matching reduced-frontmatter Cursor `domain-behavior-scanner` agent and updated Cursor orchestration commands/rules. Codex continues to invoke skills directly without agents or commands.
- `bootstrap-verifier` now enforces AI-tool selection directly in its Python validator: every selected edition root must exist, while any unselected `.claude`, `.cursor`, `.agents`, or `.codex` root causes generation verification to fail.

## [1.3.0] - 2026-07-21

### Added

- `skill-forge` now generates a much broader, still 100% evidence-driven skill set for the target, in six groups instead of three: **architecture** (1, unchanged), **design & interaction** (3, always: `architecture-implementer`, `api-designer`, `database-designer`), **frontend** (0 or 5, conditional on a detected rendering/asset layer: `frontend-design`, `coder-frontend`, `wcag-accessibility`, `web-design-guidelines`, `browser-verify`), **process & workflow** (14, always, framework-agnostic: `requirements-analyst`, `researcher`, `brainstorming`, `council`, `writing-plans`, `using-git-worktrees`, `systematic-debugger`, `refactorer`, `dependency-manager`, `review-pr`, `finishing-branch`, `documentation-generator`, `skill-creator`, `reflect`), **universal PHP** (7, unchanged), and **framework-specialty** (evidence-gated: ORM patterns, migration safety, async/queue jobs, event-boundary review, notification delivery, caching strategy, file storage, auth scaffolding, form/validator design, admin panel, declarative API resources, console commands, repository review, container review, test-data factories, package authoring), plus integrations (unchanged).
- This closes the gap identified by auditing the Laravel/Symfony/PHP-Core accelerators in this monorepo: a generated target previously received only an architecture skill + 7 universal skills + integrations, far narrower than the ~40-skill hand-built accelerators. The new framework-specialty catalog generalizes the useful Laravel-only (`eloquent`, `queues-jobs`, `caching`, `events-notifications`, `file-storage`, `console-scheduler`, `auth-scaffolding`, `filament`, `package-developer`) and Symfony-only (`doctrine-migration-designer`, `messenger-designer`, `event-subscriber-designer`, `security-voter-designer`, `form-validator-designer`, `fixture-factory-generator`, `console-command-coder`, `container-reviewer`, `repository-reviewer`, `api-platform-designer`) skills into signal-named, framework-neutral equivalents keyed to real scan evidence rather than to which framework folder they came from.
- New Project Profile section 3.1 ("Framework-Specialty Signals") and 3.2 ("Frontend Presence"), populated by an expanded `architecture-scanner` that now also detects ORM/data-access pattern, migration tooling, async/queue mechanism, event/notification patterns, caching/storage usage, auth/authorization scaffolding, form/validator design, admin panels, declarative API frameworks, console commands, repository/DI-container style, test factories, package-vs-application nature, and rendering/frontend-asset presence.
- Two new bundled references for `skill-forge`: `references/php-process-skills.md` (the 14 always-generated process/workflow skills) and `references/php-specialty-skills.md` (the evidence-gated specialty catalog, mapped from section 3.1 signals to generated skills). `references/php-frameworks.md` gained "Design & Interaction Skills" and "Frontend Skills" sections.
- Section 10.1 of the Project Profile now lists all six skill groups explicitly (with the group-by-group count breakdown carried into 10.2's arithmetic), and `infra-scan`'s "What Will Be Generated" summary reports the full group breakdown, not just architecture/universal/integrations.
- New DOD checks (Tier 1 and Tier 2) verifying sections 3.1/3.2 and that the fixed design/process groups and the conditional frontend/specialty groups were generated correctly.
- `examples/infra-scan-project-profile-example.md` rewritten end to end for the new sections and the resulting 43-skill total for the acme-billing example (up from 15).

### Fixed

- Removed the unintentional overlap between the universal `debugging` skill and the process skill `systematic-debugger` (both previously could restate the same "root-cause-first" debugging content). Their scope is now explicitly split and cross-referenced: `debugging` owns only the target's real tools/log locations ("where to look"); `systematic-debugger` owns the tool-agnostic investigative methodology ("how to look") and names zero target-specific tools. New DOD check verifies neither duplicates the other.
- Audited every remaining pair of generated skills for the same kind of overlap and fixed three more, all with the same scope-split-plus-cross-reference pattern (no skill counts changed):
  - `database-designer` (design & interaction, schema/table/index/migration design) vs. `orm-patterns` (specialty, how application code uses that schema through the ORM - relationships-as-used-in-code, casts, scopes, eager loading). Previously `database-designer`'s own reference text mentioned "Eloquent relationships," directly overlapping `orm-patterns`.
  - `performance` (universal, measure-first hot-path workflow) vs. `caching-strategy` (specialty, cache-aside correctness/invalidation). `performance` now names caching only as one possible hot-path lever and defers to `caching-strategy` for correctness depth when that specialty skill is generated.
  - `api-designer` (design & interaction, hand-rolled routes/controllers) vs. `api-platform-design` (specialty, declarative API resource frameworks). `api-designer` now explicitly narrows to any remaining hand-rolled endpoints and defers resource-level design to `api-platform-design` when a declarative framework is the target's primary/sole API mechanism.
  - DOD's Tier 2 check was broadened from just the `debugging`/`systematic-debugger` pair to cover all four scope-split pairs.

## [1.2.0] - 2026-07-21

### Added

- The Project Profile's "Generation Notes" section is now split into 10.1 (Skills To Generate), 10.2 (Agents & Commands Preview), and 10.3 (Non-PHP Neighbors). 10.1 now requires a one-line, target-specific description for every proposed skill (naming the real package/pattern/tool found) instead of a bare skill name; 10.2 states the exact agent and command counts implied by the selected edition(s).
- New Project Profile section 11 ("Memory Bank Preview") - `profile-synthesizer` now previews every memory-bank chunk `memory-seed` will seed (planned ID, title, type, source) before the user ever runs `infra-generate`, using the same confirmed-only selection rule `memory-seed` applies.
- `memory-seed` now treats the profile's section 11 as its authoritative seed plan and reports any drift between the preview and what it actually seeds, rather than silently reconciling differences.
- `infra-scan`'s final report now surfaces a "What Will Be Generated" summary (skill/agent/command counts and memory-bank chunk count) alongside the existing confidence summary, so the scope of a generation run is visible without opening the full profile.
- New DOD checks (Tier 1 and Tier 2) verifying the enriched profile sections and that seeded memory-bank chunks match their preview.
- `examples/infra-scan-project-profile-example.md` rewritten to demonstrate the new sections 10.1-10.3 and 11 end to end.

## [1.1.0] - 2026-07-20

### Added

- Self-adaptation: when `infra-scan` finds no PHP evidence, it now probes for a recognizable non-PHP stack (Flutter/Dart, Node.js, Python, Go, Ruby, Java/Kotlin, .NET, Rust, Swift, or similar, via a generic manifest-signal table) instead of only reporting out of scope.
- New `stack-adapter` skill (21st skill) - a meta-generator that, with explicit user confirmation, researches the detected stack live and builds a brand-new, fully independent sibling generator (`Infrastructure-Creator-[Stack]/`, a sibling folder next to this one) with the identical 21-skill, three-edition architecture (including its own re-authored copy of `stack-adapter`), freshly authored for that stack. Zero mentions of PHP or of this generator appear in the sibling's own content.
- New `infra-adapt <target-path>` entry point (command + agent, mirrored Claude/Cursor) for invoking `stack-adapter` directly, without going through `infra-scan`'s auto-detection first.
- `stack-adapter` is documented as the fourth sanctioned Orchestration Exception in `AGENTS.md`, alongside `infra-scan`, `infra-generate`, and `infra-build`.
- New Golden Principle ("Honest Scope Over Silent Failure or Scope Creep") and a new DOD tier (Tier 4 - Stack Adaptation) covering verification of a self-generated sibling.

## [1.0.0] - 2026-07-18

### Added

- Initial release of Infrastructure-Creator: a standalone, PHP-only meta-accelerator that scans a target PHP project and generates a bespoke accelerator directly into it, for only the AI tool(s) the target team selects.
- Two-phase workflow with a human review checkpoint:
  - `infra-scan <target>` - six parallel PHP scanners (stack, architecture, integrations, infra/ops, security/compliance, conventions), grounded web research, a minimal clarifying interview (including the mandatory AI-tool-selection question), and synthesis into one reviewable Project Profile.
  - `infra-generate <target>` - policy/skill/agent/command/hook forges plus memory seeding, flow composition, and a final verification gate; writes only the selected edition(s).
  - `infra-build <target>` - optional one-shot that chains scan then generate, pausing only on blocking ambiguity or a collision.
- 20 skills total, following standard `SKILL.md`/agent/command frontmatter conventions.
- The generator itself ships in three editions - Claude Code (`.claude/`), Cursor (`.cursor/`), and Codex (`.agents/skills` + `.codex/`) - so it runs natively from whichever tool the operator uses.
- Bundled, PHP-specialized reference material: framework detection signals, an integration catalog, architecture patterns, the Project Profile schema, a dependency-free generated-output validator, and the memory-bank validator + chunk template.
- Safety guardrails: workspace-boundary enforcement, a collision guard for pre-existing target accelerators, evidence-cited findings, and a strict no-secrets policy.
