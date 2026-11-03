# Memory Evaluation Stand

## Declared response contracts for future studies

The maintainer-only [response contract helper](MEMORY-RESPONSE-CONTRACT.md)
renders and checks the same declared answer/citation limits, keeps the last
explicitly marked attempt, and binds independent review to the exact message,
slot, source image, verifier and protocol. A positive review never rescues a
known machine failure. The caller retains immutable current-run custody and
checks source truth in its private oracle.

New private controllers may opt into this pure helper. The task-complete stand
also supports explicit per-task `response_contract` v2, with a sealed common
citation snapshot, complete stdin prompt and isolated same-byte evidence check.
Tasks without that field retain legacy behavior. Preflight references against
both shape and literal source bytes. Equivalent identifier forms or exact
required forms belong in the public task; custom grader disclosure remains
author-supplied. Existing frozen studies retain their original grades.

## Experimental whole-task arms

The opt-in `task-complete` endpoint also accepts `selective`, `consolidate`
and `consolidate-verify`, each paired with one baseline in its own sealed run.
These arms are refused by legacy first-touch runs. No installed profile,
ranking, admission rule or write-path default is changed.

`selective` starts with recall and wraps exactly one existing prompt hook.
Bounded versioned RU/EN cues select the existing full profile only for that
request. Version 2 distinguishes historical-state inquiries (including past
limits without decision words), historical decisions, current-state questions,
procedural requests and continuations. Mixed past/current inquiries can request
history; ordinary requests stay on recall. Version 1 rules remain supported.
These are purpose cues, not semantic understanding or an environment filter.
The wrapper sanitizes a copy for routing but
forwards the exact original payload, cwd, task, host, session and transcript to
the original hook. It never bypasses the gate, broadens own-save membership,
calls MCP, appends a second capsule or retries a failed hook. Routing failure
falls back to recall before invoking the hook once.

```bash
infra-engine eval memory-outcome-ab seal --set /private/tasks.json --verifiers /private/verifiers.json --design /private/design.json --out /private/selective-protocol.json --cache /separate/corpus-cache --model MODEL --arms recall,selective
infra-engine eval memory-outcome-ab run --endpoint task-complete --set /private/tasks.json --verifiers /private/verifiers.json --protocol /private/selective-protocol.json --out /private/selective-result.json --cache /separate/corpus-cache --model MODEL --arms recall,selective
```

The wrapper and public rules are copied into the run's corpus; a hook command
never refers to a tooling path the agent's allowlist cannot read. Metadata-only
router observations are transient under `memory-bank/local/experiments`, then
validated/exported into the private result after the agent exits. Digests bind
router, sanitizer and rules. Missing, unsafe or mismatched receipts give unknown
treatment attribution, not an excluded successful pair. Receipts from a writable
corpus are observations, **not authenticated proof of producer honesty**.
No raw query, session ID or transcript path is included. Optional controller-only
Boolean `needs_history` labels give TP/FP/FN/TN routing diagnostics; absent labels
remain unknown and never reach the router. Explicit retrieve and all policy/tool
availability stay equal in all arms. The router does not provide a new environment
admission filter: scoped ambiguity must not be silently converted into a fact.

The consolidation arms reproduce the existing pipeline on separate copies at
`seal`, validate them, and bind each actual public treatment image and Git store
before model execution. Derived indexes are rebuilt for the treatment and the
same screened recent-file state is restored from baseline; no private locks or
unrelated local caches are copied. Every repeat starts from that frozen image; new summary
UUIDs are not regenerated per repeat. Baseline and treatment differ in the two
declared consolidation flags and their resulting memory, with promotion settings
otherwise preserved. The mechanical-only arm is not automatic verification.
Independent resolution of each flag needs its own held-out result. See
[the lifecycle pilot contracts](MEMORY-WRITE-PATH.md).

Completion reports validate the embedded protocol seal and exact planned
membership. A result violating the sealed no-retry policy cannot improve the
quality verdict; observed attempt costs remain visible. Known agent calls/time
and input/output/cached subsets are reported separately; none establishes full
provider-attempt economics. Real held-out measurements of these new arms have
not been established by public fixture tests.

`infra-engine eval memory-eval` measures what the prompt-time memory hook delivers,
on each project **as it was when the prompt was written**. It is a
repository-level maintainer tool: it is never installed into a project, it
calls no model by default and no network, and CI runs only its tests
(`tests/test_memory_eval.py`), never the stand itself.

Source-linked expansion runs automatically without a runtime flag. Each
successful item keeps only safe `source_links` counters from the local manifest
before its corpus is removed. The repository-only `infra-engine eval memory-graph-pilot`
compares a committed baseline with the working tree using the same scorer and
inputs; its baseline contains all four installable editions so `--edition auto`
works for real projects too. Runtime version, materialization and scorer digest
identify each run.
In addition to the existing Boolean `answer_in_text`, passage counts measure
all distinct normalized answers that existed in the prompt-time source bytes
and were actually delivered. The summary's `answer_passage_recall` is their
micro ratio; a legacy result without these counts reports them unavailable.
No answer text is added to result JSON.

Runtime timeouts must be finite and positive. Measured refresh/index durations
must be finite nonnegative numbers; booleans, strings and overflowing numbers
are rejected. Missing or null legacy timings remain unavailable, while zero is
a valid measurement. Invalid optional runtime phase values are omitted; malformed
known timing fields in a saved result are rejected when reporting. Opaque legacy
metadata keeps its existing decoding rules. New results are serialized as finite
UTF-8 JSON before any output file is created, then replace an owner-only file
atomically; a failed serialization or write preserves the previous result.

## Why it exists

The read hook (`working-memory-read.sh`) puts a Task Capsule - excerpts of
the project's memory - in front of every prompt. Earlier measurements on real
prompts indexed each project's *current* state. Knowledge written after a
prompt, often about that prompt's own work - the finding its session
resolved, the chunk promoted from it - was then "retrieved" for it. That
leakage from the future inflated the useful turns three to four times. Every
change to retrieval must be measured on the project as it was at the prompt;
this stand rebuilds that state, overlays the runtime under test from this
clone, runs the refresh the hook runs, and scores the result against curated
judgments. Record who authored the labels when that evidence is available;
existing labels with unknown provenance must not be described as human judgments.

## Accepting a retrieval change

A labelled set that steered a change cannot also accept it. In October 2026
four retrieval changes were tuned on the same 12-prompt set, reported it
unchanged or improved, and still broke behaviour the set never exercised:
- version and standard names ("PHP 8.2", "UTF-8", "PSR-4") were read as
  tickets and withheld the chunks that mentioned them;
- a Keep a Changelog preamble replaced the newest entry in the quote;
- Russian aliases replaced the words a Russian document is found by;
- a request naming any unknown ticket admitted two neighbouring events.

So:

- The set a change was tuned on is a smoke test. It shows that the change does
  what it was written for, not that it generalises.
- Acceptance needs a held-out measurement written down before it runs:
  another project, or the same project's later period. The metric, the
  threshold and the one variant under test are fixed in advance, with a
  digest of that note. A variant whose ties are broken by hash order is not
  one variant; fix the tie-break first.
- Every behaviour change carries a test that fails on the previous commit.
  Such a test uses inputs outside the tuning set's vocabulary: version and
  standard names, another changelog format, another language, a Brain with
  several owners.
- A write-path mutation that has not been measured (consolidation, automatic
  verification) ships switched off. Turning it on needs a run on a copy of a
  real Brain.
- Insufficient and negative held-out results are reported as they are.
  "Unchanged on the stand" is not evidence that a change is safe.

Retrieval takes fixes; a heuristic is accepted only on a task-outcome
measurement of its own: paired runs of the stand (below) on held-out tasks,
written down and sealed before they run. The first one, on 2026-10-21, found
the full capsule no better than none, and the navigation profile faster to
the changed file, though it never delivers a saved result; `recall`
(navigation plus the task's own saves) became the default after its own run
the next day. That did not end the rule: every heuristic after it waits for
its own run, as the `ticket` arm does. The stand counts the calls
before the first touch of the file the task changed. It stops a run at that
touch, so it does not see whether the task got done. The opt-in `task-complete`
endpoint below adds full-length runs and independent checking; its public
fixture tests do not establish an efficacy result on real held-out tasks. The measures
before it score relevance labels, and real use has not shown that relevant
delivery changes what an agent does. In September one of 287 delivered
documents was opened.
- A **fix** makes the runtime do what its contract already says. Examples: a
  crash, a secret or privacy leak, a filter that withholds what its own
  documentation keeps, a lost write, a performance regression.
- A **heuristic** changes what a capsule selects, ranks or spends. Examples:
  a new boost, floor, slot, alias, expansion or budget rule.
- Fixes merge with their tests. Heuristics wait for the outcome measurement,
  and an experiment switch must leave the default unchanged.

### The `ticket` arm: a sealed measurement

The ticket files line was chosen on the 155 cold starts before 2026-10-07 of
four client projects: beside the ten recent files it found a file the turn
changed in 14 more tasks, and with co-change of the files a prompt names in 23
more (55 to 78), at the width of twenty recent files (68). That set steered
the design, so it is a smoke test (78 of its tasks pass
`--require-ticket-commits`). The measurement is written down before it runs:
`recall` against `ticket` on the same projects' cold starts from 2026-10-31,
drawn once at least 30 exist, with the stand's verdict rule. The note is kept
outside the clone (it names the projects); its SHA-256 is
`ff43fdbad191b748feacfce2a2a087d3bfdabb71108804ed250d2fdca9cd1a6f`. Until
it reads `ticket-helps`, `ticket_files` stays off.

### The every-turn save: a field check

The save request after every answer is a write-path cost that was never
weighed against a benefit: on 43 saves on copies of two client projects it
took a median 17.1 s after an answer of 70.5 s, and in real sessions the
knowledge it writes was not seen opened (`realized`, above). The four client
projects run it since 2026-10-30 - but from 17:01 that day a Harness sync
put them on `main`'s runtime, which refuses their `recall` profile, so until
they are synced again their prompts carry no capsule and none of their
turns counts toward the 50 below. What to read from their sessions, when,
and what follows was written down before any of them ran it: the field
counts of `funnel` and `realized --traces` from 2026-10-31, on the first
Monday with 50 capsule turns for each of Claude Code and Codex (at the latest
2027-02-01); if no knowledge item was opened on either host while the save
costs a fifth of the answer's time or more on one, an outcome A/B of the
every-turn save on continuation tasks comes next, before the default moves.
The note is kept outside the clone (it names the projects); its SHA-256 is
`83cf101c527e9341aa42c525d6cc6a0262f794ca1102cea39237cd40524edcee`.

`infra-engine eval memory-eval field-check --project <each> --since 2026-10-31`
computes what the rule reads, per host, from the same transcripts `realized`
reads: the capsule turns; K, the knowledge items (kinds `brain` and `chunk`)
a later tool call opened, from capsules and explicit retrieves together; T,
the task records opened, pointers and returned items together; S, the median
save seconds over the median answer's; and which branch of the rule each
host meets, or that a host is still below 50 capsule turns. Beside them, not
part of the rule, it gives the Claude Code hook runs recorded and how many
were not found: a hook that is not found decides the rule silently - and,
per host, the prompt-hook turns whose refresh assembled no capsule, which
are not capsule turns, so a host short of 50 may be short for them. Counts
only. On
the four projects' sessions from 2026-09-01 - before any save step ran there -
it reads K = 0 on both hosts, T = 1 for Claude Code and 9 for Codex: the
working-state path first.

## What a run does, per prompt

Nothing is written into the project. All work happens in the cache (default
`<system temp>/memory-eval-<uid>`, mode 0700).

1. **Files as of the prompt.** The commit is the newest one reachable from the
   project's current `HEAD` with a committer date at or before the prompt's
   `ts` (`git rev-list -1 --before=<ts> HEAD`); none means the prompt is
   skipped as `no-history`. Its files are read blob by blob (`git ls-tree` +
   `git cat-file --batch`) into a per-tree cache shared by every prompt on
   that tree, then copied into a per-prompt corpus. `git archive` is not used:
   it applies the tree's `export-ignore` and `export-subst` attributes, which
   PHP packages commonly set on `docs/` and `tests/`. Symlinks and submodules
   are not materialized. Environment files, credentials, private-key/database
   suffixes and dependency/cache directories follow the canonical automatic-query
   source denylist: they are withheld before blob reads and before working-tree
   copies, including `--as-of now`. Public trees use the `public-trees-v1` cache
   namespace so an older unfiltered tree cannot be reused. Project Brain and
   Memory Bank remain eligible; they are the state this stand measures.
2. **Memory as of the prompt.** Client projects often do not commit their
   memory, so the stand keeps what the commit has, drops what was created
   after the prompt, and adds what existed then in the working tree only:
   - Project Brain records (`project-brain/dynamic/**`) carry `created_at`;
     handoffs (`project-brain/control/handoffs/`) are dropped by it too but
     taken from Git only, because the working tree holds today's rolling text.
   - Memory Bank chunks (`memory-bank/chunks/MEM-YYYYMMDD-xxxxxxxx-*.md`) are
     placed by the date in their name, narrowed to the minute by the
     promotion record that wrote them (`project-brain/control/promotions/`,
     between its `created_at` and its `updated_at`); a legacy name falls back
     to the front matter's `created`.
   - A working-tree file whose modification time is at or before the prompt
     existed then exactly as it is now, whatever its metadata says.
   - What cannot be placed - a chunk written on the prompt's own day with no
     promotion record and a later modification time - is left out and
     counted as `undetermined`.
   - Archived Brain records (`project-brain/archive/**`, never archived
     handoffs) follow the same rule, counted under `archive_*`. The runtime
     never indexes or renders them; a promoted chunk cites the record it came
     from and fails validation without it, as it would not in the project.
     No project commits its archive, so the working tree's copy is the only
     source; one that is still active in `project-brain/dynamic/` at the
     prompt's commit is skipped (`archive_still_dynamic`). An archived body
     kept under `--updated-after keep` marks the turn contaminated.
   If the commit lacks `project-brain/config/runtime.json`, the working tree's
   copy is used (configuration such as a retrieval gate, not knowledge).
3. **The runtime under test.** The edition comes from `--edition`: `auto` (the
   default) reads, per project, the installer's sync manifest
   (`memory-bank/local/accelerator-install.json`), then the project's
   `.accelerator-policy-lock.json`, then the framework markers attached mode
   uses (`infra-tooling::attach`: `composer.json`/`composer.lock`,
   `artisan`, `bin/console`, WordPress markers), else PHP Core; a name or an
   edition directory forces one edition for every prompt. Every path any
   edition's inventory (`install/inventories/*.json`) installs is removed
   from the corpus - an older install may be committed - and the chosen
   edition is copied in the way `infra-engine install` copies it
   (source overrides, executable bits). Project-owned state is never
   replaced: `project-brain/dynamic/`, `archive/`, `control/`,
   `memory-bank/chunks/`, `memory-bank/INDEX.md`, `runtime.json` and the other
   seed-only files; `.gitignore`/`.gitattributes` and
   `AGENTS.md`/`.claude/CLAUDE.md` are merged as the installer merges them (an
   older accelerator copy of `AGENTS.md`, recognised by its first line, is
   replaced); a project `README.md` stays. Local state
   (`memory-bank/local/`, `project-brain/local/`) starts empty.
4. **A real checkout.** The corpus is committed with a fixed identity and the
   prompt's date, on branch `eval/<id>` (and `main`), with the reconstructed
   commit as parent. The history is borrowed through `objects/info/alternates`
   from a clone of the project kept in `<cache>/history/` - not from the
   project itself, whose objects Git would "freshen" (touch) on every
   `git add`. Ignore rules, the default-branch probe and the codebase map's
   drift check (`git rev-list --count <mapped>..HEAD`) behave as in the
   project.
5. **The clock.** With `--clock as-of` (the default) the runtime's
   `date.today()` and `datetime.now()` read the prompt's instant (a
   `sitecustomize` shim on `PYTHONPATH`), so review dates, validity windows
   and recency decay are those of the prompt and a result does not drift with
   the calendar. Git and file timestamps stay real.
6. **The refresh.** `memory-bank/bin/infra-engine memory index` warms the index first (a hook's index is
   warm); then `memory-bank/bin/infra-engine memory refresh --host <host> --query <prompt> --task-id
   eval/<id> --ephemeral --sanitize --json` is timed. The environment is
   minimal: no attached-mode variables, no task override, no caller Git
   configuration, `PYTHONHASHSEED=0`, `TZ=UTC`.
7. **The score.** See [What the numbers mean](#what-the-numbers-mean). Only a
   refresh that exited 0 is scored. One that ran past `--timeout` is skipped
   as `refresh-timeout`; one that exited non-zero, or printed no JSON, as
   `refresh-error` - the runtime prints its JSON and exits 1 when a memory
   layer failed to update, and scoring that turn would count a broken
   runtime as a silent one. Either way the item keeps the refresh's
   diagnostics, and the prompt counts in no measure and no latency.

`--as-of now` skips steps 1, 2 and 5 and copies the current working tree
(what Git lists plus the memory trees, without `.git` and local state): the
old, leaky measurement, kept for comparison. Its provenance counts
`future_present`, the documents the prompt could not have seen.

## Running it

Run from the repository root. Keep sets, judgments, passages and results
outside the repository.

```bash
# Baseline: the runtime as committed in this clone.
infra-engine eval memory-eval run \
  --set ~/memory-eval/set.json --judgments ~/memory-eval/judgments.json \
  --passages ~/memory-eval/passages.json --projects-root ~/Desktop \
  --out ~/memory-eval/results/baseline.json

# After a retrieval change in an edition: the same command, another --out.
infra-engine eval memory-eval run ... --out ~/memory-eval/results/change.json

# Side by side, with the per-prompt diff; then what still needs a judgment.
infra-engine eval memory-eval report ~/memory-eval/results/baseline.json \
  --compare ~/memory-eval/results/change.json
infra-engine eval memory-eval report ~/memory-eval/results/change.json --show-unjudged

# How much the old measurement leaked, on the same set.
infra-engine eval memory-eval run ... --as-of now --out ~/memory-eval/results/leaky.json

# Do agents use what the hook delivered? Aggregates only.
infra-engine eval memory-eval realized --project ~/Desktop/next --since 2026-09-01

# How far memory gets in several projects at once, from their journals alone.
infra-engine eval memory-eval funnel --project ~/Desktop/next --project ~/Desktop/scb --since 2026-10-30
```

| `run` option | Meaning |
|---|---|
| `--set FILE` | The prompts (below). |
| `--judgments FILE` | Grades per prompt and path. Required; `{}` is valid and leaves every delivery unjudged. |
| `--passages FILE` | Optional answer passages, for `answer_in_text`. |
| `--projects-root DIR` | Where a relative `project` lives (`DIR/<project>`); an absolute `project` is used as is. |
| `--edition auto\|NAME\|DIR` | `auto` per project (default), or one edition of this clone for every prompt: `"PHP Core"`, `Laravel`, `Symfony`, `WordPress` / `Cms/wordpress`. |
| `--out FILE` | The result; rewritten after every prompt, so an interrupted run keeps what it measured. |
| `--host claude\|codex\|cursor` | Passed to `refresh --host` (default `claude`). |
| `--ids A,B` / `--limit N` | Evaluate only these ids / the first N selected. |
| `--as-of prompt\|now` | As of each prompt (default) or the current tree. |
| `--clock as-of\|real` | Pin the runtime's clock to the prompt (default) or leave it alone. |
| `--updated-after drop\|keep` | A working-tree memory document created before the prompt but edited after it: leave it out (default, the strict as-of corpus) or copy today's body and mark the turn contaminated. |
| `--cache DIR` | Work and cache directory; refused inside this clone. |
| `--diagnose-misses` | Add opt-in judged-path stage and answer-presence counters; default JSON stays unchanged. |
| `--audit-git` | Diagnostic only: seed a temporary Git snapshot in each evaluation corpus so history admission can be audited when the archive has no usable `.git`. |
| `--onnx-excerpts DIR` / `--onnx-worker FILE` / `--onnx-ort-library FILE` | Maintainer-only excerpt pilot with local assets, the explicit native worker and selected ORT library; default is off. |
| `--onnx-timeout SECONDS` | Worker budget including startup and cleanup (default and maximum 5); larger requests are capped and the remaining evaluator timeout can tighten it; failure retains lexical excerpts. |
| `--timeout SECONDS` | Per runtime call (default 30). A refresh past it skips the prompt as `refresh-timeout`. |
| `--keep` | Keep each prompt's corpus and record its path in the item, for debugging. |

`run` prints the report of its result; it exits 0 when at least one prompt
was evaluated, 1 when none was, 2 on a usage error.

`report A --compare B` prints each result's own numbers side by side and
classifies recorded comparison conditions before the numeric table. The
`recorded-comparison-v1` check compares input/oracle hashes, host, clock,
historical mode, timeout, and each shared successful unit's project, timestamp,
commit, actual edition, history linkage and reconstruction counters. Declared
scorer and optional worker budgets must also have matching valid evidence.
Runtime versions, edition digests and materialization may differ as the
intended treatment; they do not by themselves invalidate a comparison.

- `compatible recorded metadata`: ordinary numeric deltas and paired gains.
- `incompatible`: known condition mismatches; quality/latency/mechanism deltas
  and paired gains are withheld. Individual columns and coverage counts remain.
- `unknown`: missing or malformed comparison evidence in otherwise readable
  legacy results; explicitly labelled ID-overlap diagnostics remain readable,
  without a strict paired claim. Existing result-shape and timing validation
  still refuses invalid files or timing budgets.

A known mismatch takes precedence over unrelated missing evidence. Explicit
`passages_sha256: null` on both sides records the same absent passage input;
missing keys do not. Contaminated historical copies remain uncertain. Existing
`as-of now` results omit the contamination field and therefore cannot establish
strict recorded compatibility without that evidence. A malformed timestamp,
including timezone overflow outside the representable UTC range, stays unknown.
Reasons name fields rather than echoing their supplied values.

Quality and latency deltas retain their existing **successful-intersection**
meaning: only prompts evaluated successfully in both runs contribute. A timeout
does not become a successful quality/latency observation. `prompts`, `evaluated`
and `skipped` compare all items recorded in each file, and the report names ids
successful in only one run. Inspect those coverage losses alongside the
conditional diagnostics. This table is not a whole-planned outcome endpoint:
interrupted legacy files may lack unattempted units or source opportunities.
Recorded compatibility also does not prove identical historical corpus bodies,
task success, total model cost or complete experiment execution.

New runs separately report `whole-planned` delivery using the
`planned-delivery-v1` manifest. The selected logical IDs and input digests are
saved before the first evaluation. Each unit's useful-path, answer-path and
distinct normalized answer-passage opportunity is frozen and checkpointed
before index warm-up. The same private passage set is used for scoring. Result
JSON retains counts and SHA-256 bindings, never the prompt or answer bodies.
Terminal-only `items` and their existing `summary` keep their prior meaning.

The supplementary **unit delivery** rate is the fraction of planned units with
known useful source material that delivered a useful path. **Passage recall**
is total delivered answer passages divided by the frozen source passages.
A completed refresh timeout/error contributes zero delivery while retaining its
known opportunity. For two useful units with passage ceilings 3+1, delivery of
the first and timeout of the second gives 50% unit delivery and 75% passage
recall; reversing the timeout gives 50% and 25%. These measure local hook
delivery, not source verification, model use, task success or total token cost.

Final rates require a finished, noninterrupted run, a terminal result for every
selected ID and known source opportunity for every unit. Missing projects,
missing history and invalid timestamps remain visible `source_excluded` units
with unknown opportunity; they do not silently shrink the planned denominator.
Incomplete runs show terminal/source-known coverage, truly unattempted units
and units without a terminal result, with final rates unavailable. A
`runtime-admitted` checkpoint precedes warm-up; it is not proof of a physical
call or its cost. Interruption inside warm-up preserves its opportunity and
marks that unit interrupted. A failed checkpoint prevents subsequent runtime
calls. Empty opportunity yields an unavailable rate rather than fabricated 0%.

Legacy results without a manifest, malformed counts or inconsistent item/plan
bindings report `unknown`. Recorded consistency is not proof of producer
honesty or identical source bytes. Whole-planned values are individual run
figures with **no paired delta**: the comparison compatibility check above
examines shared successful units and does not establish matching opportunities
for failed units. Physical retries do not create additional logical IDs; this
stand has no retry or complete model-attempt cost ledger.
Passage-count evidence uses strict integers from 0 to 2^53−1 and delivery cannot
exceed its source ceiling. Invalid counts are unavailable in the conditional
table too; otherwise valid successful-intersection arithmetic is unchanged.
Invalid UTF-8 inputs fail before runtime and preserve the preceding output.

`realized` reads Claude Code transcripts (`--claude-root`, default
`~/.claude/projects`) and Codex rollouts (`--codex-root`, default
`~/.codex/sessions`). Claude Code folders are the project path with every
non-alphanumeric character replaced by `-`, plus its `--claude-worktrees-*`
variants; another folder with the same prefix is read only when its sessions
ran in the project or in a worktree of the same repository (otherwise it is
counted as skipped - `next-gen` is not `next`). A turn starts at the prompt
hook's output (a `hook_success` `UserPromptSubmit` attachment; in Codex, a
developer message that starts like a capsule, which a Codex of October 2026
or later also marks `hooks.additional_context`) and ends at the next human
prompt. The hook may print "Previous native memory result was not saved
completely." ahead of the capsule; until 2026-10-30 such a Codex turn was
not counted at all. A hook that printed "Memory refresh unavailable" opened
a turn without a capsule. `--json` prints the numbers as JSON, `--paths`
adds per-path counts.

A capsule hands over as memory only the items it lists: since October each
on a line `- memory|history|skill|policy <path> — <title>`, its excerpt
indented below; the September runtime put the layer on a line of its own
(`semantic:`) with each item indented under it, plus `weak-match: <path>`.
Both forms are read. The other lines name paths that are not delivered
memory - recent and ticket files, the task's sources, the last turn, an
excerpt that mentions a document - and until 2026-10-30 they were counted as
delivered; the `task record:` pointer to the task's own record is counted on
its own row (`task record pointers`). A task's record is the kind
`task-record`, apart from the Brain's findings and decisions (`brain`).

`realized` also reads every explicit retrieve in the sessions it reads -
`memory-bank/bin/infra-engine memory retrieve` (or `context`) through a shell, and the Memory MCP's
`memory_retrieve` - in a capsule turn or not, pairs each call with its answer
by the host's call ID, and reads the answer as it reads a capsule (or the
capsule's JSON, with `--json`). Per host it counts the sessions that
retrieved, the calls, the answers read and those naming a document, the
documents returned and how many a later tool call opened, and the task
records the answers pointed at and how many were opened afterwards; a
document counts once per session. By kind it gives the same, and as a
baseline the documents of that kind a session that retrieved opened without
any retrieve naming them. In next's Codex sessions since 2026-09-01 (992
read, 36 of them retrieved, 75 calls) the answers named 25 task records, 9
of them opened afterwards, 4 findings and chunks, none opened, and
`CHANGELOG.md` in 14 sessions, opened in 11. That runtime printed no `task
record:` pointer; the records came as history items. It also refused a task
not recorded yet: 14 of the 23 calls that ran retrieve alone were answered
"Working task not found", and 39 calls ran `start` first; the runtime answers
such a retrieve since 2026-10-06. Codex heads a command's output with its own
lines ("Script completed", the wall time, "Output:"), which `realized` reads
past.

`realized` also counts every hook run a Claude Code transcript records, by
event: succeeded, failed - and of those, how many failed because the command
was not found (exit 127) - and blocked. Claude Code records a tool hook's
success only when the hook printed, so a quiet `PreToolUse` reads 0 there. In
September 2026 two client projects' hooks were wired as relative
`.claude/hooks/...` commands: below the project root none was found, 3,309
times, and the Bash validator did not run there for a month. The wiring has
named `"${CLAUDE_PROJECT_DIR}"` since; this count is how such a break shows.

`realized --traces` sets the runtime's own record beside the host's. The
runtime keeps a trace of every capsule it rendered (the last 200 by default,
in `memory-bank/local/retrieval-traces`) with the digest of the text it
printed and the items it showed; the manifest of the same ID names what asked
for it. Each hook turn's capsule and each explicit retrieve's answer is
matched to the trace whose digest it reproduces, each trace once. The report
gives the renders in the window, the hook renders a transcript shows were
handed over and those none shows - a refresh stopped at its budget after it
printed, a host that dropped the capsule, a session whose transcript is not
on this machine, and every Cursor session - the retrieve answers that match
a render, and per host the hook capsules matched to a render, the items those
renders showed and how many the turn touched. Counts only. The traces exist
only where a runtime of 2026-10-13 or later rendered; a capsule from a recall
profile turn without saves of its own renders without a search and leaves no
trace. In the same sessions'
440 prompt-hook turns, 140 carried a capsule. The other 300 printed
"Capsule unavailable": "Working task not found" from the runtime of then
(the prompt refresh has served a task not yet recorded since 2026-10-06), or
an automatic-continuation conflict whose code no longer exists anywhere in
this repository. The 140 delivered 54 items, 44 of them `CHANGELOG.md` (25
opened), and none of the rest was opened. Until 2026-10-31 `realized` counted
a turn whose refresh assembled no capsule as a turn with one, since the
refresh's own layer lines come first; it now counts them apart. Over the four
client projects from 2026-09-01 they are 448 of 931 prompt-hook turns - 300
of next's 440 (Codex), 73 of scb's 147, 63 of bauherrenmappe's 216 and 12 of
aea-partner's 128 - every one in September, for those two reasons.

Read the Claude Code side of those months with one fact beside it: no Claude
Code session in the four client projects had the memory policy. Each project
keeps a `CLAUDE.md` of its own, and Claude Code reads `AGENTS.md` by itself
only where there is none. The edition's `.claude/CLAUDE.md` imports it since
2026-10-06; it reached three of the clients on 2026-10-08 and `next` on
2026-10-18, after the last Claude Code session in each (2026-09-18 to
2026-10-06), and none has run there since. Their transcripts show the
instructions Claude Code loaded: the project's own `CLAUDE.md` in 191 of 386,
`AGENTS.md` in none. Claude Code's "no retrieve,
1 of 289 delivered documents opened" measures sessions that were never told
to retrieve; Codex reads `AGENTS.md` itself, and did.
Codex keeps a finished rollout compressed, as `rollout-*.jsonl.zst` (3,040 of
3,099 on the maintainer machine in October 2026); `realized` reads those too,
through the native zstd decoder. Without
either, it says how many compressed rollouts it could not read
(`compressed_unread`), counted over every project, since which project a log
nobody could read belongs to is unknown.

`realized` also reads the project's native result journal,
`memory-bank/local/native-results.ndjson` (`--state DIR` for an attached
project's state directory), and prints how far the automatic native saves got,
per host: prompt hook runs, save requests armed (and how many followed a
request that was not saved completely), Codex continuation prompts, Stop hook
runs with their timeouts and non-zero exits, saves requested, saved, held,
omitted and failed, refused saves, errors, learnings written and chunks
promoted, then every code with its count (`save refused
memory-result-v1:unverified`, `stop skipped recovery-saved`, `stop-hook exited
124`). An exact repeat of a finished save is not counted again. The journal's
codes are fixed words; anything else in a code field counts as `other` and is
never printed. The prompt hook's lines in `refresh-health.ndjson` name no host,
so they are given once, for all hosts. A project without the journal ran a
runtime before it, or no native step at all; `docs/MEMORY-INTEGRATION.md`
lists what each line means.

`funnel` adds up several projects at once from their journals alone, so it
needs no transcripts and its output can come from a teammate's machine. Each
`--project` is a project directory, or an attached project's state directory;
its folder name labels the column and the path stays local. Per project and
in total, it counts:
- **the prompt hook**: its runs from its own lines in `refresh-health.ndjson`,
  with timeouts (`124`) and other non-zero exits;
- **the capsules**: every `refresh` and Cursor `hook-context` line, and from
  the `capsule` shape a runtime of 2026-10-30 or later records - the profile,
  the working state, how many recent files, ticket files and documents it
  carried, its token estimate - how many had each, by profile, and the median
  tokens. The estimate counts documents only, so a capsule of working state
  and recent files reads 0; the bytes the capsule printed, recorded since a
  later runtime of the same day, give the median printed size. An older
  line has no shape and counts as `unknown`. A refresh line also comes from
  an explicit request (the MCP retrieve, the CLI); a runtime of 2026-10-30 or
  later names the host that asked (`cli` for an explicit request), and the
  funnel counts capsules by host, an older line as `unknown`. A line that
  asked for a capsule and got none - a profile the runtime refused, a task ID
  it cannot read - says so (`"capsule": null`, since 2026-10-31) and counts
  as "capsules asked for, none built", not as a capsule; on older runtimes it
  wrote no shape and read as a capsule of `unknown` profile, which is how the
  four clients' capsule-less turns after 2026-10-30 17:01 would have looked;
- **the saves**: from `native-results.ndjson`, as `realized` reads it - armed,
  requested at Stop, made (saved or held), refused, failed or omitted,
  learnings written and chunks promoted, with every code and its count;
  newer save rows also distinguish unique submitted/existing/held learnings;
  absent or inconsistent selection counters remain unknown, never inferred
  from zero writes. Exact replays do not double-count the selection;
- **what the project keeps**: task records, findings and decisions, and bank
  chunks, counted as files. The `--since` window applies to the journals, not
  to these.

`install-status` answers the question before those counts: can memory run
in the project at all? Per `--project`, from the project's own files: the
commit its accelerator was last synced from and whether this clone has it
(this clone's, behind it, or not in it - newer, or another line of work);
the configured capsule profile and the profiles the project's runtime
knows; the last prompt hook run, whether the last capsule asked for was
built, and the last save step; and which instructions Claude Code loads.
Each problem is one way memory went silent in the client projects: a
profile the runtime refuses on every prompt (2026-10-30, after a sync from
an older clone), a last capsule not built, a project's own `CLAUDE.md`
without the edition's import (September), an `AGENTS.md` Codex cuts before
the policy, and Claude Code hook commands that name `.claude/` relative to
the folder a session starts in. It exits 1 when any project has a problem,
prints no record text, and `--json` gives the same as JSON. On the four
clients on the night of 2026-10-31 it named the refused `recall` profile in
each and the cut policy in one.

It prints counts and fixed codes only; `--json` gives the same numbers as
JSON. A project where the hook ran but no capsule carried a recent file, or
where saves are requested and none is made, shows where to look next;
`realized` then tells, from the transcripts on the machine that has them,
whether the agent used what was delivered.

For a real historical comparison, freeze each project's reachable Git head and
eligible working-tree memory once, preserving modification times, and point both
versions at those copies. Match the input hashes, host, selected edition,
historical commit, clock and reconstruction provenance before comparing scores.
Use a separate cache for each version. Keep a development set and a holdout set
separate in reports; do not tune against their combined score. A surviving
working-tree memory copy and today's runtime configuration cannot establish the
complete historical working tree, even when both versions use identical inputs.

## Data formats

**Set** - a JSON list; other keys (`session`, `cwd`, ...) are ignored:

```json
[{"id": "next-017", "project": "next", "ts": "2026-08-05T12:00:00Z", "prompt": "..."}]
```

**Judgments** - `{prompt id: {path: grade}}`, grade the integer 0, 1 or 2; 1
and above is useful, 0 is noise. Any other grade (`3`, `-1`, `1.5`, `true`,
`"2"`, `null`), or a prompt whose judgments are not an object, stops `run`
before it starts with an error naming the prompt and the path (exit 2): it
would otherwise count as useful or noise, or leave its document unjudged. The
one key that is not a path is `_note`, a reviewer's note as text, which is
read past.
Skill paths in any tool tree (`.claude/skills/`, `.cursor/skills/`,
`.codex/skills/`) count as `.agents/skills/`; two spellings of one path keep
the higher grade.

**Passages** - `{prompt id: {path: {"useful": true, "passages": ["..."]}}}`:
the labelled text that supports a document's usefulness for that prompt.

Each prompt must map to document objects. `passages`, when present, must be a
list of strings; an omitted or empty list has no passage labels. `useful`,
when present, must be a JSON Boolean. Invalid entries fail before creating
the evaluation corpus. Skill-path aliases must have identical entries after
path normalization; conflicting labels are rejected rather than choosing
whichever alias appears last. Validation errors do not print passage text.

Before annotating a request, distinguish facts necessary for its answer from
supporting execution guidance and related examples. Preserve the canonical
source and explain that distinction in the judgment note. For example, an
admin-test namespace review needs the test namespace, Composer mappings and
the admin suite configuration. A README's application test command supports
verification; a separate Rector suite does not establish the namespace fix.
The legacy `answer_*` counters measure literal labelled-text presence, not
necessity, model use or final-answer correctness.

**Result** - `{"meta": ..., "items": {id: ...}, "summary": ...}`; new runs also
carry `planned` and a derived `planned_delivery`. Reports recompute the latter
from the manifest and terminal items. `planned` contains `profile`, ordered
`ids` and `units`; each unit repeats its `id` and keeps `input_sha256`, `runtime_admitted`, a
`state` (`unattempted`, `preparing`, `runtime-admitted`, `finished` or
`interrupted`) and, when known, `source_opportunity` counts (`useful_paths`,
`answer_paths`, `answer_passages`). Terminal items repeat `input_sha256` and
their `planned_id`; IDs and distinct digests must match the manifest entries.
`meta` names
the edition(s) with their release, a digest of the edition files the overlay
installs and whether the edition directory had uncommitted changes, this
clone's commit, the host, `as_of`, the clock, the SHA-256 of the set,
judgments and passages files, and the start and finish times. Each item has
`status` (`ok`, or `skipped` with a `reason` such as `no-history`,
`refresh-timeout` or `refresh-error`), `edition` and `edition_source`,
`commit`, `provenance` (`from_git`, `from_worktree`, `dropped_future`,
`undetermined`, `updated_after`, `unreconstructable_updated`, the seven
`archive_*` counts; `config_from_worktree` when the working
tree's `runtime.json` was used; under `--as-of now`, `future_present`), the
overlay counts, `history_linked`,
`refresh` (exit status, seconds, the warm-up's seconds and exit status, the
runtime's phase timings, a warning count, each memory layer's state -
`updated`, `failed`, or `other` for anything else - a stderr tail with any
line quoting the prompt withheld, `source_links` counters, and `index`: the
corpus index's document count, link rows by kind and how many indexed
documents still match a secret pattern), and, when `ok`, the score below.
The score also carries mechanism counters, as fixed words and numbers only:
`layer_kinds` (item kinds per layer), `selections` (items each link
mechanism chose), `source_changed_delivered`, `skill_subfiles_delivered`
(skill files other than `SKILL.md` in the procedural slot) and
`capsule_secret_matches`. `summary` is what `report` prints; its mechanism
totals are absent (`-`) when any item predates them, never zero. Result files
are written owner-only (0600). The stand runs no hook and binds no task
(`--task-id eval/<id>`), so file links seeded by touched files never fire
there; links seeded by paths named in the prompt do, and count under
`selections`.

When the runtime supplies a valid fixed `funnel_diagnosis`, the item retains it
under `refresh.funnel_diagnosis`. Missing or malformed metadata and unexpected
validator errors omit only that child; a valid unavailable observation retains
its null counters. This historical observation does not change eligibility,
scoring, summaries, text reports or the default miss-audit behavior. Counts are
authorized occurrences or lower bounds; source coverage and consumer use remain
unknown and answer quality remains unmeasured.

A valid automatic `ordinary_receipt` sibling is retained independently under
`refresh.ordinary_receipt`. Its four scalar fields report only availability,
plan presence and the paired owner's returned `skipped`, `deferred` or
`committed` status. No-plan observations allow only `skipped`; unavailable
observations carry null plans and `unknown`. Missing or invalid metadata stays
unknown and is omitted. Ordinary validation failure omits only this child;
cancellation still propagates. It adds no score, gate, miss audit, timing or
text-report input, and does not prove durability or delivery acknowledgement.

## What the numbers mean

**Knowledge and skills are read apart.** In real sessions agents followed
none of the 71 skills a capsule named (no Skill call, no `SKILL.md` read on
those turns) and used accelerator skills on 1.1% of Claude turns; on Codex the
user picked them (`$code-reviewer`). Skills were 53 of the 66 useful paths the
core set missed. A turn counted `useful` for a skill is useful on its label,
not in what the agent did, so the summary also reports:

- `knowledge_*` - the turn classes, could-help, useful and noise paths with
  every skill path left out: what project knowledge did;
- `skill_*` - the procedural slot as a routing question. Where a
  labelled-useful skill existed: `skill_hit` (a useful one delivered),
  `skill_wrong` (another one), `skill_empty` (none), out of
  `skill_with_useful`. Where none did: `skill_silent`, `skill_noise`,
  `skill_unjudged`, out of `skill_without_useful`.

Both are computed from each item's paths, so results written before them
report them too.

- **delivered** - item paths of the capsule's three layers, normalised. The
  layers of a `refresh` hold what its `capsule_text` shows: an item the
  rendered text had no room for is left out of them. `useful_delivered` have
  a grade of 1 or more, `noise_delivered` a grade of 0, `unjudged` none.
- **existed_useful / could help** - judged-useful paths present in the
  reconstructed corpus (skills always count as present): the turn could have
  been helped.
- **answer in the capsule text** - a passage of a useful delivered document
  is, whitespace-normalised and case-folded, a substring of `capsule_text`,
  the text a model reads. Delivering a pointer is not delivering the answer.
- **answer_existed / could answer** - judged-useful documents whose labelled
  passage is already in the corpus's copy. Judgments are per file, and a file
  often existed at the prompt without the part that answered it - a changelog
  entry, a spec section written for that very work - so "could help" alone
  overstates what memory could have handed over. "Answer in text among
  could-answer" is the delivery rate against that stricter ceiling. Skills
  count like any document, read where the overlay installed them, so the
  ceiling and "answer in the capsule text" cover the same documents.
- **class** - `useful` (at least one useful item), `noise-only` (delivered,
  nothing useful, at least one judged noise), `unjudged-only`, `silent`
  (nothing delivered, or the sanitizer left nothing to search).
- **latency** - wall time of the refresh process on a warm index, p50 and p95.
- **realized** - per host: prompt-hook turns, those whose refresh assembled
  no capsule ("Capsule unavailable"), turns with a capsule and with
  documents, delivered items the agent touched with a tool and mentioned in
  its text, by kind (skill, brain, task-record, chunk, task, spec-doc,
  changelog); the capsules' task record pointers and how many the agent
  touched; `memory-bank/bin/infra-engine memory retrieve` calls and Memory MCP `memory_retrieve` calls
  by the agent in those turns; and, as a baseline, documents of those kinds
  the agent opened that the capsule did not deliver. Then the explicit
  retrieves of whole sessions, what they returned and what was opened
  afterwards; the Claude Code hook runs by event, failed and not found among
  them; and with `--traces` the renders the runtime made that a transcript
  shows were handed over, and those none shows. From the native result
  journal, per host: the
  automatic save funnel from prompt hook run to promoted chunk, and the codes
  of every skip, refusal and failure. From the transcripts, per host: how many
  save requests the Stop hook made and, as medians, the seconds and output
  tokens from each request to the next human prompt, beside those of the
  answer before it, and where the host records them the input tokens and how
  many came from the cache. The journal counts requests, but only the
  transcript shows what each one cost. A request is found by its
  `INTERNAL_NATIVE_MEMORY_RESULT <token>` marker wherever the host recorded
  it, never in the agent's own text. On 21 Codex runs (16-19 October 2026) it
  was +9.9 s and +383 output tokens after a 17.5 s answer of 701 tokens. Its
  input is the larger cost: on 43 saves on copies of two client projects
  (24 October) the median save re-read 176,782 input tokens, 175,616 of them
  cached, for 662 output tokens and 17.1 s, after an answer of 70.5 s. The
  team decides on `native_result_request` from that line. Once a runtime
  names how its request ends, the line is also split by the ending, read
  from the request's own words: `brief` (a session a person reads as it goes),
  `repeat` (the answer again) and `earlier` (a runtime that asked neither); on
  the same runs the 9 `repeat` requests cost +408 output tokens, the 12
  `earlier` ones +360. The funnel counts the requests that asked for the
  brief ending beside all of them.

`realized` records a tool's path references without checking whether the tool
successfully read that file; mentions are textual matches. Neither measures
knowledge use or final-answer quality. For confirmed opens, pair tool starts
with successful completions using the Harness activity ledger and retain its
`observed`, `lower_bound` or `unknown` status. A capsule may already carry the
answer without requiring a file read. Existing receipts without memory telemetry
establish missing observation, rather than zero use.

Grow the judgments with `report --show-unjudged`: it lists every delivered
`(id, path)` pair without a grade.

## Limits

- The commit is found from the project's current `HEAD`: work on a branch
  not merged into it is not seen, and history rewritten since is what it is
  now.
- Only memory is recovered from the working tree. Uncommitted code, specs and
  task documents at the prompt are not reconstructed; a record or chunk that
  existed then and was deleted since is lost.
- A record created before the prompt and edited after it cannot be rewound:
  its body today may hold the answer the prompt's work produced. By default
  it is left out (`unreconstructable_updated`), which errs towards less;
  `--updated-after keep` copies today's body (`updated_after`) and marks the
  turn `contaminated`, a count `report` prints. A chunk re-verified after the
  prompt is refused by the pinned clock as verified "in the future", which
  also errs towards less, not more.
- The capsule is assembled for the prompt alone: the branch's working task
  is not reconstructed and no session id is passed, so nothing is
  suppressed as already handed.
- Projects used in attached mode keep their memory in the Harness state
  directory, which the stand does not read.
- The controlled native clock supplies an explicit advancing knowledge-calendar
  offset. Proof ages, process deadlines and cancellation keep the real clock.
  A child path must preserve that typed calendar or refuse before mutation;
  ordinary commands never read a calendar override from ambient environment.
- `<cache>/trees/` and `<cache>/history/` (a full clone per project) persist
  across runs and grow with them; delete the cache to reclaim the space.

## Privacy

Sets, judgments, passages, transcripts and results are client data. Keep them
outside this repository (`run` warns when an input or the output is inside
the clone, and refuses a cache there). A result carries prompt ids, document
paths and counts only - never prompt text, capsule text or document bodies;
the stderr tail withholds any line that quotes the prompt. `realized` prints
aggregates; document paths only with `--paths`. `funnel` reads no path, query
or record text and labels a project by its folder name. The cache holds full copies
of client projects: per-prompt corpora are removed as soon as the prompt is
scored (unless `--keep`), the tree cache and history clones stay until the
cache is deleted.


## Diagnosing misses

First keep the native runtime artifact and resources unchanged and run the same frozen set with
`--diagnose-misses`. The helper receives private input over stdin, reads the
actual refresh manifest and a read-only index, and runs under the refresh's
pinned clock. It emits only supplied judged paths, fixed stage labels and
counts. Main FTS uses the explicitly pinned refresh limit of 3 (window 30).
History uses the whole screened prompt, matching the runtime's separate
history query, rather than substituting the distilled main query.

When the ranking branch is the larger loss, the
[document reranker pilot](MEMORY-RERANK-PILOT.md) compares the baseline order
with a path/title feature reranker before any runtime change.

The evaluation corpus is a copy without the project's live Git context.
`--audit-git` seeds a disposable repository for the history audit, but a large
corpus can still exceed the source-policy deadline; in that case its paths stay
`unknown-history` and `history_recall` is a lower bound. Run the history audit
against a live checkout when the exact admission verdict matters.

`CONTEXT_EVENT_SERIES=1` is an off-by-default runtime pilot: when the request
names no ticket, the history slot may take up to two events from the largest
same-ticket series. On the real 12-prompt stand it moved selected recall from
52.6% to 55.3% and answer passage recall from 34.2% to 36.8%, with the same
noise-only count; that is not enough to change the default. `memory_eval`
passes the variable through only for a stand run.

A labelled answer passage must have non-heading text. A passage that is only
`#` headings is reported as `answer passages heading-only` and excluded from
the answer-passage denominator; it is a label to rewrite, not a document the
runtime failed to quote. On the real stand 7 labels were heading-only, so the
honest recall was 41.9% (13/31) rather than 34.2% (13/38).

Interpret stages as a funnel: `absent-source` and `not-indexed` precede
retrieval; `no-fts-candidate`, `window-limit`, `relevance-drop` and `floor-drop`
precede selection. The summary reports two derived shares over the labelled
paths the index could have returned (`eligible_paths`, which excludes absent,
not-indexed and unavailable/unknown): `candidate_recall` is the share that
reached the FTS window at all, and `selected_recall` is the share that reached
the final selection, including an item later lost to the capsule budget. A low
candidate share means query/index work; a high candidate share with a low
selected share means ranking or admission work. History paths are tagged by
the indexed `brain-event` kind, whether they reached `selected`,
`history-event-limit` or `history-admission-drop`; they participate in those
two shares and have their own `history_paths`, `history_delivered` and
`history_recall`. Only an unavailable history context stays `unknown-history`
and is excluded. The report prints the larger of the two losses as
`candidate-bound`, `selection-bound`, `mixed` or `none measured`.
Manifest policy exclusions, layer/rank limits, budget and capsule drops remain
distinct. `history-admission-drop` means the recorded subject did not pass
coverage/weighted-floor admission; `history-event-limit`
means another admitted event/episode won the shared slot. `automatic-link-limit`
is the manifest's explicit file-link cap. `selected-answer-not-in-text`
includes partial delivery of a source's labelled answers. Unsupported evidence
remains `unknown` or `unknown-history`; it is never converted into a search miss.

The report separates knowledge and skills. An absent labelled path is not an
eligible prompt-time recall denominator. Per-path `eligible/indexed/delivered`
passage counts are normalized independently and can count a duplicated passage
in two paths. They do not replace the scorer's globally deduplicated per-prompt
answer denominator. Reconstructed `eval/<id>` branches lack the original
session's task, touched files and repeat-delivery state; do not infer native
session causality from a reconstructed refresh.

A labelled history path can be supporting or an alternative to the selected
event. `history-event-limit` therefore measures labelled coverage, not a proven
missing answer. Check the request's active subject and distinguish necessary
evidence, supporting alternatives and related examples before changing a
selector. Literal fragment delivery does not establish final-answer quality.

For latency comparisons, alternate baseline and candidate runs of the same
requests with the same clock and cache warmth. Host contention can affect even
the unchanged baseline; p95 values from separate batches do not by themselves
identify the CPU cost of a selector change.

## Measuring task outcomes

Every number above scores what a capsule delivers against relevance labels.
None says whether delivery changes what an agent does.
`infra-engine eval memory-outcome-ab` measures that on tasks that really happened.

**What a pair is.**
- **The task:** the first prompt of a recorded Claude Code session whose turn
  changed at least one file the prompt does not name. With `sample --hosts
  codex` (or `claude,codex`) it can also be a Codex rollout that ran in the
  project or below it, compressed ones included: the runtime's own transcript
  parsers read both. The default stays `claude`, so a sealed earlier sample
  draws the same tasks. `--hosts` also decides whose sessions seed the recent
  files: each task records them as `seed_hosts`, and a task from before them
  seeds from Claude Code alone. `recall-sources` takes `--hosts` too.
  A turn starts at what a person wrote. The runtime's prompt test leaves out
  the context Codex adds in the user role - its `AGENTS.md` instructions,
  recommended plugins, a skill's text, a hook's continuation prompt, an
  aborted turn; of 3,099 rollouts on the maintainer machine, 2,310 began
  with such a message. The stand also leaves out a turn Codex starts by
  itself: a notification, a heartbeat, a CI event, each a message that is
  wholly one envelope. Context around the person's request - `# In app
  browser:` or `# Files mentioned by the user:` above `## My request`,
  `<in-app-browser-context>` with the request after it - and a paste are the
  person's prompt. Before 2026-10-07 the four client projects hold 38 Codex
  cold starts (all with 1-10 gold files, none in an earlier sample) beside
  117 from Claude Code, and 268 Codex recall sources beside 105: most Codex
  sessions there began with a read-only review. Its *gold* files are those
  changed files. Each one must be:
  - a path the project's Git has known;
  - kept by the runtime's recent-files filter;
  - not part of the memory system itself.
- **The corpus:** the project rebuilt as of the prompt with this stand's
  reconstruction. Recent files are seeded from the project's earlier sessions,
  as the Stop hook would have recorded them.
- **The arms:** a headless `codex exec` session runs the same prompt twice:
  - `off`, with no hooks;
  - `capsule`, with only the memory prompt hook (`working-memory-read.sh`)
    and `capsule_profile: full` in the corpus.

  The arms therefore differ by the Task Capsule alone. The policy's own
  `memory-bank/bin/infra-engine memory retrieve` stays available in both. Until 2026-10-30 `capsule`
  left the profile to the corpus's own `runtime.json`; since the editions
  shipped `recall` (2026-10-22) that would have run recall under the old
  name.
- **The `navigation` arm** runs the same hook with `capsule_profile:
  navigation` in the corpus: working state and recent files, no documents.
  It writes no manifest, so its runs are recognised by the line every hook
  refresh appends to `refresh-health.ndjson`.
- **The `recall` arm** runs it with `capsule_profile: recall`: navigation plus
  the task's own saves. It writes a manifest only on a turn one of them
  matched, so a run is valid when the hook ran.
- **The `ticket` arm** is `recall` with `ticket_files: true`: the capsule also
  names the files of the earlier commits for the prompt's or the branch's
  ticket (`docs/CONTEXT-AND-MEMORY.md`). A run is valid when the hook ran;
  `report --compare recall,ticket` also says in how many pairs the line
  reached the capsule (`ticket_files_delivered`, from the health lines). A
  corpus branch keeps the ticket of the branch its session ran on
  (`ab/<id>-CMS-741`), so the runtime and the agent see it there as they did.
  `sample --require-ticket-commits` keeps the cold starts the line can act
  on: the prompt or the session's branch names a ticket, and a commit
  reachable from the corpus commit names it with a file the line would show -
  present, not named in the prompt and not among the ten recent files the
  seeded capsule shows. Each such task records its `tickets`.
- **Comparing arms:** `report --compare BASELINE,TREATMENT` compares any two
  arms of a result. The rule below then reads `<treatment>-helps` /
  `<treatment>-hurts`. Each run also records the token estimate of the
  documents the hook's manifest delivered, and the bytes the hook's capsules
  printed (`capsule_bytes`, from the health journal, None before the runtime
  recorded them).
- **Cost:** `report` prints, per arm and outside the verdict, the mean
  seconds of a run, on how many runs Codex reported its usage, the mean
  input, cached and output tokens over those, the documents' token
  estimate and the printed bytes. Codex reports usage only when a turn
  completes, so a run stopped at the first gold touch has none; the estimate
  counts documents only, so a recall or ticket capsule without one reads 0
  tokens, and its printed bytes say what it cost.
- **Repeats:** `--repeat` runs each arm several times; the order alternates by
  task and repeat.

**What is counted.**
- **The metric:** tool calls (commands, file changes, MCP and web calls)
  before the agent first touches a gold file, in a command it runs or a change
  it makes.
- **Stopping:** a run stops at that touch, at `--cap` calls or at `--timeout`.
  A run that never touched a gold file counts one past the cap.
- **Validity:** a pair counts only if the hook's own manifest (`refresh` or
  `hook-context`) shows that the capsule arm received a capsule in every repeat
  and the `off` arm received none.

**Recall sets.** A cold start has no earlier save to recall, so the metric
above cannot see recall. A recall set asks whether the agent uses what an
earlier session saved instead of investigating again.
- **The source:** a session's first turn that read project files, changed
  none, and answered at length naming files it read (`recall-sources`).
- **The scenario:** what that session would have saved - one learning citing
  files the turn read - and a question a new session asks that needs it, not
  naming the *needed* files. Both are written outside the tool for each
  source, and `recall-set` checks them: the learning is one `record-result`
  accepts, every file it cites is in the corpus at the question, the needed
  files are among its sources, and the question names none.
- **The corpus:** the project at the moment of the question, with its recent
  files. Before any arm runs, `run` records the save on the corpus branch's
  task through `record-result`, with the project's own promotion settings,
  then commits and indexes it: every arm, `off` included, starts from the
  same tree, and the agent's own `retrieve` can find the save in any arm.
- **The regime (`recall-set --regime`):**
  - `next`: the question comes a minute after the source turn ended. The
    recent files then usually show the files the turn read, so every hooked
    arm has a pointer to the subject.
  - `later`: the same save and question, asked at the start of the first
    later real session whose recent files - replayed from every turn before
    it - show none of the files the answer named. Navigation then has nothing
    to point at, and only the save can.
- **The metric (`report --metric recall`, a recall result's default):** calls
  before the agent first touches a needed file, or - a turn that finished
  without one - before an answer that names one. A run that reached neither
  counts one past the cap. Each run also records whether the hook's capsule
  delivered the save.

**The verdict.** `report` prints the mean difference capsule − off, a
bootstrap 95% interval and an exact sign test, then applies the rule:
- **insufficient:** fewer than 20 valid pairs;
- **capsule-helps:** the interval lies below zero and the capsule arm touches
  gold no less often (one task of slack);
- **capsule-hurts:** the interval lies above zero;
- **no-difference-shown:** anything else.

Write the sample, the arms and this rule down before a run, with a digest of
the note.

**Limits.**
- The working tree at the prompt is not rebuilt. A task that reviewed
  uncommitted changes may not reproduce; this hits both arms alike.
- The agent is Codex, whatever host the original session used.
- Long prompts that name components leave few calls to save, which bounds any
  effect from above.

**The first measurement (2026-10-21).**

Setup:
- 30 tasks, drawn with a fixed seed from the 62 cold starts that changed 1–10
  gold files in four client projects before 2026-10-07. The tuning set and
  the pilot were excluded.
- Runtime `fef3b217`; Codex CLI 0.163.0 with its default model.
- Two repeats of each arm per task.
- The rule above was written down and sealed by digest before the run.

Results:
- 30 valid pairs.
- Mean calls to the first gold file: 17.10 without the capsule, 17.13 with it.
- Difference +0.03, 95% CI [−2.57, +2.65]: **no-difference-shown**.
- Each arm touched gold in every repeat on 18 tasks.
- The capsule arm took fewer calls on 15 tasks and more on 5 (sign test
  p 0.041). It also ended a session without touching gold about as often as
  the other arm (22 of 60 runs against 21), and those censored runs carry the
  mean.

On the 16 tasks where every run of both arms reached gold, the capsule saved
about one call (−1.09, CI [−1.72, −0.5]). That figure is exploratory and not
part of the rule.

What follows: the document layers have not shown that they change what an
agent does. A capsule reduced to the working state and navigation is the next
candidate default, measured the same way against this one.

**The second measurement (2026-10-21): three arms.**

Setup:
- 30 fresh tasks from the same pool, with the pilot and the first sample
  left out;
- seed 2027, rule sealed before the run;
- arms `off`, `capsule` and `navigation` (`capsule_profile: navigation`), two
  repeats each.

Results:

| comparison | means | difference, 95% CI | gold reached | verdict |
|---|---|---|---|---|
| navigation against capsule | 21.80 → 18.88 | −2.92 [−6.25, −0.40] | 14 → 17 | navigation-helps |
| navigation against off | 21.67 → 18.88 | −2.78 [−6.82, +0.42] | 16 → 17 | no difference shown |
| capsule against off | 21.67 → 21.80 | +0.13 [−2.72, +3.18] | 16 → 14 | no difference shown |

The sealed rule made navigation a candidate default, but it is not the
default. Shipping it turned red 23 suites that pin what the document layers
deliver:
- a result saved in one session recalled by the next session's capsule;
- source hints;
- documents in the Cursor rule.

This metric counts calls to a file, not recall. A smaller capsule that keeps
recall needs its own arm and its own measurement before the default moves.

**The third measurement (2026-10-22): four arms and recall.**

The candidate is `capsule_profile: recall`: navigation plus the task's own
saves. The rule was sealed before the run and asked three things:
- **(a)** on recall sets, `recall` is not worse than `full`;
- **(b)** on cold starts, `recall` is navigation;
- **(c)** with `recall` shipped, every suite that saves a result and expects
  the next automatic capsule to carry it stays green.

Setup:
- **Recall sets.** 30 scenarios, seed 2028, from 105 client first turns that
  answered from files they read. The learning and the question were written
  by an AI author from each turn's own answer. Real pairs - a result saved
  through `record-result` or `native-result` and asked about later - do not
  exist in the four clients: none was ever saved there.
- **Two regimes:** `next` and `later`, the same 30 sources.
- **Arms:** `off`, `capsule`, `navigation`, `recall`, two repeats each;
  runtime `fb197740`.
- **Cold starts:** the 14 left unused by the earlier samples.

Results on the recall sets (calls to the save's subject):

| regime | comparison | pairs | means | difference, 95% CI | verdict |
|---|---|---|---|---|---|
| next | recall against capsule | 25 | 2.70 → 2.70 | 0.00 [−0.26, +0.28] | no difference shown |
| next | recall against navigation | 25 | 2.60 → 2.66 | +0.06 [−1.08, +2.02] | no difference shown |
| later | recall against capsule | 24 | 4.19 → 3.98 | −0.21 [−0.42, −0.02] | recall-helps |
| later | recall against navigation | 24 | 4.58 → 3.98 | −0.60 [−3.54, +3.21] | no difference shown |

What the rows show:
- The hook delivered the save in every pair with `recall` and with `full`,
  and never with `navigation`.
- Reached within the cap, `recall` against `capsule`: next 24 → 24, later
  23 → 23.
- **Lost results:** the `later` run lost five results to a task-ID collision,
  now fixed. A sealed amendment reran four of them. With those 28 pairs the
  first `later` row is no difference shown (−0.11 [−0.32, +0.11]): `recall` is
  as good as `full` here, not better.
- **Navigation:** against it the mean difference is not shown. By sign,
  `recall` was faster on 19 of 23 tasks without a tie (next) and 21 of 23
  (later), with a median of one call. That is exploratory: a few censored
  runs set the means, and every arm reaches the subject in two to six calls.

On cold starts, the `recall` capsule was byte-identical to navigation's on
all 14 tasks: a new branch has no saves of its own. The agent runs there
(n = 14) are insufficient by the rule. Their two identical arms differed by
+1.57 calls [+0.07, +4.32] - a reminder of why the rule wants 20 pairs.

The suites, with `recall` shipped in the four editions and the generator
template:

| shipped default | passed | failed |
|---|---|---|
| `full` | 235 | 0 |
| `navigation` | 212 | 23 |
| `recall` | 214 | 21 |

`recall` turns green the three runtime recall suites navigation breaks:
- native save, then a new session's `refresh`;
- MCP save, then CLI recall;
- the hook outputs of a saved fact.

Two blockers remain:
- **The Harness recall suite.** It expects a corrected learning in the
  next message, and the learning arrives as a chunk quoted by its
  `## Sources` footer. Under `full` the chunk is quoted the same way; the
  suite passes only because the cited source file comes along, which
  `recall` does not deliver. `excerpt_section` skips a citation-only footer
  for linked selections only.
- **Harness automatic memory requires a manifest.** A capsule without
  documents - navigation always, `recall` without a match of its own -
  fails its `prepare`.

So (a) and (b) held and (c) did not, and the default stayed `full`.

**(c) run again (2026-10-22).** Both blockers are fixed:
- `excerpt_section` skips a citation-only footer for every Memory Bank chunk,
  whatever selected it, so the corrected learning reaches the next message as
  its rule. The 327-prompt stand under `full` delivered the same documents at
  the same sizes before and after the change (217 useful).
- In Harness a capsule with no document and no source proof needs no
  fingerprints; a proof that does arrive is checked as before. Harness's
  explicit `refresh` and the LongMemEval bridge ask for `full`.

The tests that check a layer `recall` leaves out on purpose pin `full` in
their fixtures:
- the shared document in the Cursor rule;
- the document beside broken UTF-8;
- Hindsight's source hints;
- the Git calls and the working record re-read on the full search path;
- the membership index's full capsule render;
- Harness's first-message document, funnel diagnosis, ordinary receipt and
  opened-memory report.

With `recall` shipped, `check.py` passed 235 commands and failed none, the
Harness recall suite among them. The 7 it skipped need what that machine
lacked: ShellCheck, the Harness graph and QA venvs, and Windows. So (c)
holds: the four editions and the generator's template ship `recall`. The
code's default stays `full` for a project whose `runtime.json` names no
profile.

**Privacy.** Sets carry prompts and results name client paths. Both are
written owner-only and are refused inside the clone. `--events` keeps raw
sessions for a pilot and belongs in a private folder.

## Optional ONNX excerpt experiment

Use a reranker when an eligible document is already found but its ranking or
quoted window loses the answer. If it is absent from the candidate set, measure
candidate expansion with a hybrid search separately. An excerpt reranker cannot
recover an absent document, widen the history slot or enlarge the link cap.
What the pilot measures decides whether a reranker earns a task-outcome run,
never whether it ships: a default changes only on its own sealed outcome
measurement ("Accepting a retrieval change").

For a session repeat, inspect the local trace's `window_observation` before
attributing the missing answer to ranking. A `different-exposure-window`
compares the current sanitized pre-novelty proposal with the exact prior final
render body for the same novelty identity. Count comparable, identical,
different, empty-proposal and unknown-prior cases separately. Retain prior
pointer-only/shrunk state and source relation. This diagnostic is automatic,
contains no raw passages, and does not change selection or session expiry.
It measures rendered exposure, not answer presence, host receipt or model use.

The current pilot tests **excerpt selection only**. The ordinary runtime first
selects and policy-filters documents. The pilot reads only those documents'
masked indexed text, creates at most eight deterministic alternate windows per
document and 32 passages per worker call, and uses the renderer's privacy screen
before sending query/text to the worker. Automatic linked-task excerpts keep
their specialized selection. Exact ties preserve the lexical excerpt. Quotes
are prepared before session novelty, so the rendered section and its delivery
receipt agree. A chosen quote also supplies the structured snippet and token
estimate. Pure citation-only footers retain the lexical renderer's safeguards.
Alternate windows use the existing 320-character structured snippet cap.
After privacy screening, alternatives that cannot fit the item's unchanged
token estimate are excluded before counting the 32 worker passages; the lexical
option remains first. The ordinary budgets, quotas and renderer remain active;
a longer structured snippet is still defensively rejected, and if an alternate changes which paths
the renderer would retain, the complete lexical selection is recomputed once
without another worker call. A failed score attempt is not retried at rendering.
The native adapter implements the core's pre-session `ExcerptPreparer` seam. Invalid selections or changed packed membership restore the complete lexical result.
This is a maintainer experiment and is never included in installer inventories.

Provision the selected native engine, local model assets and dynamic ONNX Runtime library explicitly; no Python environment is created.

```bash
infra-engine eval memory-eval run --set prompts.json --judgments judgments.json \
  --out /private/memory-eval/onnx.json --diagnose-misses \
  --onnx-excerpts /private/memory-eval/models/minilm-l6 \
  --onnx-worker /opt/infra/bin/infra-engine \
  --onnx-ort-library /opt/onnxruntime/lib/libonnxruntime.so --onnx-timeout 5
```

The native worker verifies assets and accepts a bounded query/passages packet. Its explicit worker deadline is capped by the remaining invocation budget. Failure preserves the lexical capsule atomically; a score attempt is not retried during rendering. Optional libraries and models are never downloaded by the worker.

The legacy schema-less local asset directory needs `manifest.json` and these five files:
`model_O3.onnx`, `tokenizer.json`, `config.json`, `tokenizer_config.json`,
`special_tokens_map.json`. The manifest's `files` maps each basename to its
lowercase SHA-256 and positive byte `size`; the worker verifies every file before
importing optional packages. Record the model ID, exact upstream revision,
license and official source URLs as provenance. The model must accept BERT
`input_ids/attention_mask/token_type_ids` and return finite `[batch, 1]` logits;
other models require an explicit separate adapter. For example, use the pinned
[MiniLM model tree](https://huggingface.co/cross-encoder/ms-marco-MiniLM-L6-v2/tree/ce0834f22110de6d9222af7a7a03628121708969)
with CPU-portable O3 FP32, rather than silently substituting a GPU-oriented
optimization. Query is bounded to 1,024 characters, passages to 1,200 characters
and pair tokenization to 384 tokens.

An explicitly selected `jina-tiny-fp32` profile uses `schema_version: 1` and
`profile: "jina-tiny-fp32"` in its local manifest. Both keys are required;
unknown profiles, future versions and partial discriminators fail closed.
The profile pins `jinaai/jina-reranker-v1-tiny-en` at revision
`aca45de6945b5dc6399abcd2a9c55ded5dc9111f`, variant `onnx/model.onnx`,
`pad_token: "<pad>"`, and integer `pad_id: 1`. Its five basenames are
`model.onnx` and the same four JSON files. Each manifest hash and size must
match the worker-owned `JINA_FILES` pins in
[`memory_onnx.py`](https://github.com/PHP-Innowise/AI-Infrastructure/blob/6628d976d3fa1ca2d03ebb7d8714503a7b87b824/scripts/memory_onnx.py), as well as the actual local bytes.
Inert provenance fields may remain; the worker never fetches their URLs.
The [publisher model tree](https://huggingface.co/jinaai/jina-reranker-v1-tiny-en/tree/aca45de6945b5dc6399abcd2a9c55ded5dc9111f)
provides the pinned ONNX and tokenizer assets; publisher Python is not executed.

The historical Python profile's exact NumPy/ORT/tokenizers versions belong to its sealed artifact identity. The native worker uses the Rust tokenizers/ORT implementation and an explicit library/asset image; it must not be reported as the same historical worker or used to silently regrade an old study. Its tokenizer is
`RobertaTokenizer`: the pinned config declares pad ID zero, while the actual
`<pad>` token maps to ID one. The worker validates both declarations without
rewriting metadata and pads on the right with ID one. It supplies only
`input_ids` and `attention_mask` as int64 rank-two arrays, checks CPU-only
node metadata before inference, caps pairs at 384 tokens with longest-first
right truncation, and requires finite float32 `[batch, 1]` raw logits in
passage order. Legacy manifests retain their BERT three-input/pad-zero route.
Profile support alone does not establish retrieval quality or multilingual
reliability; Jina remains an optional maintainer experiment.

Every valid requested worker timeout is capped at five seconds before applying
the tighter remaining invocation deadline. Startup, pipe I/O and cleanup share
that one budget. Requests above five are accepted but shortened; shorter
requests remain shorter. The existing OS scheduling and process-tree limits
still apply. Any failed profile, metadata, shape, dependency or score check
keeps the complete lexical capsule and session delivery receipts.

If cleanup cannot be confirmed before that deadline, the runner retains one
pending child and its owned I/O handles, including Job custody on Windows.
Before spawning another worker, the next valid request must recover that pending
invocation within the new request's own existing deadline. Pending invocations
do not accumulate; no recovery thread or extra cleanup grace is introduced.

Freeze model/assets, settings, labels, prompt-time snapshots and the scorer
before measuring. Use development queries first; only a passing configuration
may run the separate fixed replay. A reused replay is not a fresh blind holdout.
Report paired passage delivery, useful/noise paths, unexpected fallbacks and
complete refresh latency (including cold model startup); a model smoke test is
not evidence of retrieval improvement. Keep models, queries, client corpora and
result JSON outside Git. CI exercises stdlib fixtures and fake workers, without
ML downloads or dependencies.


## Optional complete-task endpoint

### Explicit v2 response intake

A public task may declare `response_contract` v2; its private verifier then
provides `response: {reference, answer_field}`. Answer cases grade exactly one
nonnullable string field through the existing equals oracle. Patch cases use
`answer_field: null` and retain the external patch oracle. See the
[complete input, custody and failure contract](MEMORY-RESPONSE-CONTRACT.md#explicit-task-complete-integration).
Both arms receive the same read-only citation corpus. It is extra common public
context: `off` disables hooks, not physical memory. Treatment-only historical
knowledge must not leak through this image in an absence-control experiment.
Passing this integration proves evaluation plumbing, not memory benefit.

### Private delivery observations

Complete-task results additionally retain `delivery_proof`, a bounded
maintainer-only projection of new retrieval manifests in the governed
`project-brain/control/` or ignored `memory-bank/local/` directories. It names only
safe paths present in the maintainer-owned pre-run snapshot, document hashes,
frame/task UUIDs and origins (`automatic` versus `explicit`). It omits queries,
excerpts, record bodies and arbitrary metadata. New agent-created paths outside
that vocabulary remain unreported. Legacy first-touch output is unchanged.

The projection distinguishes selected documents from optional local render
traces. `trace-reported` means a matching local sidecar, not host ingestion.
`producer_authenticated: false`, `host_receipt: unknown` and
`model_use: unproven` are intentional. A matching document image can provide
its declared record ID and stored citation hashes; a changed image cannot
lend today's metadata to yesterday's delivery. Missing or partial digest coverage
for declared sources remains unknown. Current citation checks are
explicitly **post-run**, preserve historical expected hashes and establish
byte identity only. They do not reverify a claim or authenticate a reviewer.

Limits are 32 frames, 32 documents in total, 1 MiB per file, 8 MiB of reads and
64 KiB of exported JSON. Expired deadlines/budgets mark collection incomplete.
Missing traces, unsafe/hardlinked/special files, malformed JSON and unknown
metadata never become positive evidence. Byte/work bounds do not promise
preemption of a slow regular filesystem read. Optional collection failure
does not change native completion, independent grading or treatment validity.

The legacy `run` default is still `--endpoint first-touch`; its gold/recall
metrics, verdict and hook-only treatment remain unchanged. The opt-in
`task-complete` controller runs until the native turn AND CLI finish, or its
sealed cap/deadline. `completed` describes execution, never correctness.
For multiple native turns, only the last active turn can establish completion;
an unfinished continuation cannot reuse an earlier answer. Tool IDs are scoped
to each turn while calls, the cap and deadline remain cumulative. Duplicate
terminal events and items after a terminal event invalidate completion.

Prepare private inputs together in a private directory **outside this clone**,
and keep the separate corpus cache outside that directory. Add `task_mode`
(`answer` or `patch`) and an opaque `verifier_id` to each existing task-set item.
A verifier manifest has `{"cases":{"id":CASE}}`. An answer case is exactly
`{"mode":"answer","equals":"the complete expected answer"}`: whitespace at
its edges is ignored; mentioning the right file is not sufficient.

A patch case has exactly `mode: patch`, `argv`, positive finite `timeout` and
`read_roots`. Its executed oracle must be a sealed external executable or
`[absolute_python, absolute_oracle_script, ...]`, under dedicated read-only
`read_roots`, outside the candidate. Bare pytest/unittest discovery in the
candidate and Python `-c`/`-m` wrappers are not independent oracles. On exit 0,
the trusted driver must print exactly one JSON object:

```json
{"schema":"task-outcome-verifier-v1","completed":true,"correct":true}
```

The driver is trusted code: **it must isolate candidate execution from its own
process and labels**, then independently check behavior. Importing arbitrary
candidate code into the oracle's interpreter and trusting exit 0 is unsafe.
The explicit receipt detects early exit without a verdict; it is not a general
proof that an arbitrary driver cannot be subverted. Missing receipts, errors,
invalid output, changing oracle/label bytes and timeout yield `unavailable`
with `correct: null`. Patches are checked on a separate disposable copy after
the agent stops, before its corpus is removed. Gold-bearing feedback is not
sent back to the agent or used to select a better retry.
Oracle seals include every file and directory under each declared read root,
including `.git`, `__pycache__`, empty directories and ordinary permission bits.
Candidate snapshot exclusions are not label exclusions. Bounded, nonblocking
regular-file reads reject special answer/label files; duplicate receipt keys
are invalid rather than selecting the last value.
Special file permission bits are refused; only ordinary file modes are supported.

The design JSON must contain exactly `primary: correctness`, numeric
`quality_margin` in [0,1), positive integer `minimum_tasks`, nonempty
`power_rationale`, `selection`, `exclusion`, `secondary_benefit`,
`stopping_rule: fixed-horizon` and `unknown_policy: insufficient`.
Power/sample size, margin, tuning exclusions and the variant are chosen before
the real holdout, not inherited from the old navigation threshold. Example
commands (these are not run by CI and call a model only at `run`):

```bash
infra-engine eval memory-outcome-ab seal --set /private/tasks.json --verifiers /private/verifiers.json --design /private/design.json --out /private/protocol.json --cache /separate/corpus-cache --model MODEL --arms off,recall
infra-engine eval memory-outcome-ab run --endpoint task-complete --set /private/tasks.json --verifiers /private/verifiers.json --protocol /private/protocol.json --out /private/result.json --cache /separate/corpus-cache --model MODEL --arms off,recall
infra-engine eval memory-outcome-ab report /private/result.json --metric task-complete --json
```

`seal` reconstructs the prompt-time public snapshots and saves immutable base
images, config hashes per arm, runtime/CLI/input/oracle bindings, budgets and
an explicit no-retry policy before any model process. Repeat and budget options
must match between seal and run. Raw events, selective IDs and kept agent
artifacts are refused by this private endpoint. Raw final answers exist only
in a temporary private artifact outside the mounted agent corpus. Aggregates
contain counts/digests rather than prompts, expected answers or transcripts.
A seal is a content binding, not a signature authenticating its author's claims.

Linux bubblewrap with usable user namespaces is mandatory. A probe executes in
the actual mount allowlist and proves it cannot read the private manifest;
unavailable isolation never falls back to ordinary host execution. Only the
public OS/toolchain, the native CLI package, a fresh home with its read-only
`auth.json`, and that run's writable corpus are visible. The outer namespace
allows the native provider transport; the native workspace-write sandbox keeps
its tool networking policy. No account config/trust file is written. This is
not evidence of native Windows support or provider execution in CI.

For this first private endpoint, Git is rebuilt from the screened public
snapshot, with no borrowed object store or historical credential blobs. The
sealed `git_history: public-snapshot-only` is an intentional limitation:
history/ticket-dependent tasks are not faithful history replays, and the ticket
arm is refused here. Other legacy endpoints retain their historical behavior.
Candidate snapshots reject symlinks and special files and are bounded to
100,000 files / 512 MiB; answer artifacts to 1 MiB. Unsupported snapshots yield
unavailable evidence instead of dereferencing external paths or exhausting RAM.
Post-exit health/manifests use nonblocking, no-follow regular-file reads, a
1 MiB per-artifact limit and at most 1,000 manifest directory entries. Unsafe
or oversized telemetry makes treatment attribution unknown; FIFO/link artifacts
cannot block report collection. Remaining collection time is checked between
manifests; this is not a universal hard filesystem-scheduling guarantee.

Every task × arm × repeat slot is checkpointed before model work. Missing,
duplicated or changed membership is refused in reports. Interrupted/unattempted
slots and unknown grades cannot shrink the denominator into a perfect success
rate. Timeout/cap/agent failure means not completed within its budget; uncertain
infrastructure/verifier evidence stays unknown. The paired analysis groups
repeats within task; it never treats them as independent tasks or selects the
best retry. Incomplete plans/grades/treatments yield `insufficient`.

Costs are observed host-reported input/output (cached input is already included)
and elapsed time, not complete physical provider-attempt billing. All attempts
in an imported result retain their observed cost. Missing usage remains unknown,
not zero; a known subtotal is separate from covered whole-plan host usage.
Completed-turn usage is accumulated across continuations. `known_usage`,
`usage_events` and per-field turn coverage retain partial observations; `usage`
is unavailable when a required turn observation or final completion is missing.
Optional cached/reasoning fields with partial coverage remain known subtotals.
Provider totals, USD, savings and cost-per-correct stay unavailable. The existing
`harness/src/harness/memory_benchmark_cost.py` supplied-observation ledger is not
replaced or silently called complete: a CLI PID is not a physical provider
attempt, and a local deterministic verifier is not a complete zero-attempt
model judge.

Native `error` reconnect notifications are provisional: the original deadline
and cap continue to apply until `turn.failed`, CLI exit or completion. An
uncompleted native/CLI failure has an unavailable grade, rather than being
mistaken for an incorrect answer about the project. Explicit timeout/cap
exhaustion remains a failed task within its sealed budget. On systems whose
OpenSSL bundle lives at `/etc/ssl/cert.pem`, that exact public trust file is
mounted read-only alongside the certificate directory; TLS verification is
never disabled.

### Source and served tasks

Legacy authored recall sets still save onto their own branch task. New cases
may name `source_task_id` and `served_task_id`; the served ID is the corpus
branch, and the controller records both real UUIDs and refuses a claimed
cross-task pair resolving to one UUID. Newly injected `saves` are always
`save_provenance: authored`, with native result receipt digests; calling such an
injection observed/generated is refused.

An existing save may instead carry `save_provenance: observed|generated`,
`source_task_id`, `served_task_id`, earlier `saved_at`, and
`save_receipt: {path, sha256}` binding a captured native result receipt. The
source task and indexed own knowledge must already exist in the prompt-time
snapshot, and the receipt must match that actual task UUID. No missing learning
is recreated. The supplied provenance label is retained with
`authorship_verified: false`: a receipt proves a save, not model/human identity,
truth or absence of fabricated source evidence. Preserve source-session proof
privately when making a real observed/generated claim. Changing the captured
receipt after seal refuses execution.

Public roundtrip tests exercise save on A, a different UUID B, full retrieve
availability and absence from B's own recall. Their fixtures are not real
client sessions or a causal long-term benefit measurement. Choose real later
states with intervening work and without recent-file answer hints when sealing
such a holdout; moving a clock alone does not establish long-term recall.

## Native producer and fresh consumer

Trusted preparation rebuilds `memory-bank/INDEX.md` unconditionally after the
materialized admission gate is constructed, runs the separate Bank validator
(`memory-bank/bin/infra-engine bank validate`), validates Brain, then refreshes SQLite.
`memory-bank/bin/infra-engine memory validate` alone validates Brain and cannot establish Bank validity.
Any failure stops before native dispatch and retains the failed attempt and
preparation receipts. Reindexing changes only the derived catalog; it does not
re-attest, repair, promote or discard imported knowledge. A timestamp shortcut
is insufficient when a fresh blank INDEX is newer than the imported chunks.
Custom controllers must invoke `memory_native_ab.prepare_memory(trusted)` in
the same post-admission, private-environment, bounded-receipt phase, before
baseline capture and sealing. Running it in `install_runtime()` would create
local locks/indexes before the fresh-image admission check.

`infra-engine eval memory-native-ab` makes the native A→B development procedure
reproducible. It is a private maintainer tool, separate from the default
first-touch stand and from a confirmatory task-outcome comparison.

Create a UTF-8 private configuration with exactly these fields:

```json
{
  "schema": "memory-native-ab-v1",
  "project_snapshot": "/private/project-snapshot",
  "source": {
    "task_alias": "proof/source-alpha",
    "prompt": "Inspect the current retry contract. Retain one new source-backed finding if warranted."
  },
  "served": {
    "task_alias": "proof/served-omega",
    "prompt": "Find the prior retry finding using explicit retrieve, recheck its sources and cite its MEM identifier."
  },
  "model": "gpt-6.1-sol",
  "tool_cap": 40,
  "deadline_seconds": 420
}
```

Aliases must differ and be lowercase safe task names. Caps are integers
1–100; stage deadlines are integer seconds 1–900. Unknown keys, malformed or
linked inputs, detected secrets and overlapping paths are refused. No
caller-supplied executable, hook, verifier, arbitrary environment or native
configuration is admitted. The source path is a screened physical project
snapshot; links, special files and hardlinks in admitted data are refused.

```bash
infra-engine eval memory-native-ab plan --config /private/native-ab.json --out /private/native-run
infra-engine eval memory-native-ab run --run /private/native-run --stage source
infra-engine eval memory-native-ab inspect --run /private/native-run
infra-engine eval memory-native-ab run --run /private/native-run --stage served
```

Omit `--stage` to execute both stages. Planning binds the screened source
bytes **and executable modes**, prompts, budgets and canonical runtime images.
It creates owner-only `control/plan.json` and `control/state.json` with two
planned slots before execution. Source/runtime drift refuses execution.
Inspection reads existing artifacts without initializing a database or
launching subprocesses. A crash may leave a running/uncertain slot or lock;
inspection does not silently repair it, retry it or remove the prior evidence.

Each stage receives a new physical `work/source` or `work/served` tree,
independent Git metadata, native home and persistent private temporary root.
Client execution configuration and local caches/history/queues/handoffs are
excluded. Canonical executable runtime directories are replaced completely,
including unknown project modules that could shadow trusted imports.
Preparatory Git runs ignore inherited global/system configuration.
The copy is committed as a clean screened baseline before native hooks run,
so overlay files are not mistaken for task changes.

Full canonical lifecycle hooks are configured per invocation. Existing source
entries and runtime files are pinned read-only; memory/Brain/handoff mounts
pin their ancestors while permitting owned data updates. The corpus root
permits native sandbox denial stubs; unexpected public file additions block
the handoff. Native auth is a separate read-only mount and
is never copied. Read-isolation probes must deny the private plan, original
project and sibling source corpus. There is no host fallback.

A valid source capture needs an observed successful canonical native-result
command, matching task/session/request/token, normalized payload digest,
receipt and complete proof. Decisions and findings retain their distinct
accepted/resolved states; learning-only saves and progress-only saves are
distinguished. Only a newly created record and its newly promoted, fully
source-matching chunk count toward A's new durable knowledge. A promotion
sweep can also promote older records; its total is not A's learning count.

#### Mandatory snapshot admission

Planning screens the held selected image before creating the output. Each
stage screens its own held image again before copying, Git staging or context
indexing; the source copy must match the sealed source digest. B independently
screens new A knowledge even when its handoff hash matches. This gate has no
configuration/environment bypass and never edits or redacts an input.

The bounded policy rejects known credential assignments and containers,
including quoted/escaped JSON keys, YAML key/type/value carriers, literal
registry credentials, opaque `EV[...]`, private-key material and URL userinfo.
Default-looking and numeric passwords are credentials too. Exact
`[REDACTED_PRIVATE_VALUE]` / `[REDACTED]` scalars and narrowly defined whole
environment references are supported. In YAML-like colon assignments,
references must be quoted; ambiguous suffixes, folded values, comments and
unsupported credential expressions require a separately reviewed projection.
For non-JSON secret-container declarations, all `value` fields in the file
must be screened. Invalid UTF-8, NUL/C0 controls and unsupported binaries fail
closed. This is known-pattern admission, not proof that arbitrary encoded
secrets or personal information cannot exist.

After the canonical overlay, a fresh materialized gate checks every file,
required runtime/hook presence and held application bytes/modes. Canonical
trust requires exact path, bytes and ordinary mode, never a directory prefix.
Intentional installer merges are checked explicitly. Trusted preparation
creates fresh Git and index state from admitted inputs in a private HOME/TMP;
their opaque binaries are separately declared derived state. Preexisting Git
history/databases cannot acquire that trust. Their identities are fixed before
mount setup, while all other generated text remains screened.

After the trusted namespace probe creates its mountpoint directories, the
gate screens the complete actual tree, binds file/directory identities and
modes (including the private root), and verifies it immediately before model
dispatch. Unknown paths, missing files, chmod or byte changes block launch.
The external read-only provider-auth mount is an explicit transport exception;
only its empty mode-0600 mountpoint is present in the physical corpus. The gate
does not claim that the native process has no access to provider credentials.

Private `source-admission.json` / `served-admission.json` and matching verified
receipts contain policy IDs, aggregate counts and digests, without matched
values, snippets or unsafe filenames. Gate/controller bytes participate in the
runtime seal. A refusal retains the attempted slot and blocks downstream work;
it does not count as a correct answer or a correctly empty native save.

This protocol explicitly defers manual `record-result`/`memory_write` capture
until the native Stop continuation. A record written earlier in the same
session may be valid project knowledge, but does not satisfy this experiment's
native-capture criterion. `no-learning` classifies the native payload rather
than claiming the entire session produced no knowledge. Capture is elicited
under these instructions; arbitrary spontaneous extraction is not measured.

The supported observed-save encoding is a direct canonical native invocation,
optionally inside the emitted exact-corpus native engine wrapper,
with literal JSON in a single-quoted heredoc. A plain `cat` quoted-heredoc
pipeline into that exact wrapper is also accepted; extra commands, cat
options, output redirections and unquoted delimiters are not. The runner
requests that format in a generic instruction; it supplies no learning contents. Unsupported
command encodings remain unproven. Shell/interpreter paths are pinned to
trusted forms, and extracted command text is never executed by the verifier.
Receipts and sidecars remain unsigned local evidence: producer authentication,
semantic truth and human review are not established by matching hashes.

Before the handoff, A must have no unresolved native requests, spooled/queued
updates or pending turn observations. Its own completed observations may be
settled through canonical `turn --flush` under **A's** identity only, after
validated capture and healthy hooks; foreign task rows block this step.
The runner verifies chunk bytes did not change and rechecks that A is clear.
B never drains A's buffers. B's pending state remains in its private corpus;
neither database deletion nor cleanup is described as lossless.

B must resolve to a different task UUID and native session. Success requires
an A chunk in an explicit-retrieve manifest, matching post-run document and
complete citation hashes, and a render hash matching actual successful native
tool stdout. The final answer must attribute that MEM record. The collector
accepts the complete exact UTF-8 frame at the start of a successful canonical
retrieve's output, even when a later command appends text; `native_stdout_match`
distinguishes `whole` and `prefix`. Leading text, partial/changed frames and
unrelated commands remain unobserved. This does not authenticate the producer.
The collector
still reports local trace versus host/model receipt separately; observed
consultation and attribution do not establish causal necessity or correctness.

Native transport has bounded stdout/stderr, line size, answer size, turn count,
tool count and time; continuations share the original stage budget. Process
groups are cleaned even after the leader exits. Synchronous filesystem reads
have byte/work limits and deadline checks, not hard-preemptible wall time.
Raw streams and native process results are private, including results retained
when later collection fails. All completed-turn usage observations and known
subtotals are retained; missing/interrupted coverage and provider billing remain
unknown. Failed A leaves B planned but blocked. A slot is never retried under
the same plan; corrected runs need a new output and preserve prior failures.

The public summary always says `cohort_role: development-smoke`,
`natural_delayed_efficacy: false`, and leaves causal benefit/provider savings
null. Pausing between stages does not turn a predetermined B question into a
natural delayed task. The public offline adapter fixture exercises real hooks
and storage without contacting a provider; its tests are not native model runs.

For a later confirmatory comparison, collect genuinely new source sessions and
natural later questions with intervening work, exclusion/family receipts and an
independent private oracle. Exclude all development-exposed families and their
RU/EN paraphrases. Existing outcome tooling supports two arms at a time: seal
`recall↔selective` as primary and `recall↔full` as diagnostic comparisons.
Cover absent knowledge, conflict, fact update and environment changes before
reading outcomes. No eligible delayed dataset is implied by this runner.
