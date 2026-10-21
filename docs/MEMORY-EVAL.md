# Memory Evaluation Stand

`scripts/memory_eval.py` measures what the prompt-time memory hook delivers,
on each project **as it was when the prompt was written**. It is a
repository-level maintainer tool: it is never installed into a project, it
calls no model by default and no network, and CI runs only its tests
(`tests/test_memory_eval.py`), never the stand itself.

Source-linked expansion runs automatically without a runtime flag. Each
successful item keeps only safe `source_links` counters from the local manifest
before its corpus is removed. The repository-only `memory_graph_pilot.py`
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

Retrieval is frozen to fixes until task outcomes are measured: a capsule
against no capsule, on replayed or real tasks, counting the calls before the
first touch of the file the task changed and whether the task got done. All
of the earlier measures score relevance labels, and real use has not shown
that relevant delivery changes what an agent does. In September one of 287
delivered documents was opened.
- A **fix** makes the runtime do what its contract already says. Examples: a
  crash, a secret or privacy leak, a filter that withholds what its own
  documentation keeps, a lost write, a performance regression.
- A **heuristic** changes what a capsule selects, ranks or spends. Examples:
  a new boost, floor, slot, alias, expansion or budget rule.
- Fixes merge with their tests. Heuristics wait for the outcome measurement,
  and an experiment switch must leave the default unchanged.

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
   uses (`scripts/accelerator_attach.py`: `composer.json`/`composer.lock`,
   `artisan`, `bin/console`, WordPress markers), else PHP Core; a name or an
   edition directory forces one edition for every prompt. Every path any
   edition's inventory (`install/inventories/*.json`) installs is removed
   from the corpus - an older install may be committed - and the chosen
   edition is copied in the way `scripts/install_accelerator.py` copies it
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
6. **The refresh.** `context.py index` warms the index first (a hook's index is
   warm); then `context.py refresh --host <host> --query <prompt> --task-id
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
python3 scripts/memory_eval.py run \
  --set ~/memory-eval/set.json --judgments ~/memory-eval/judgments.json \
  --passages ~/memory-eval/passages.json --projects-root ~/Desktop \
  --out ~/memory-eval/results/baseline.json

# After a retrieval change in an edition: the same command, another --out.
python3 scripts/memory_eval.py run ... --out ~/memory-eval/results/change.json

# Side by side, with the per-prompt diff; then what still needs a judgment.
python3 scripts/memory_eval.py report ~/memory-eval/results/baseline.json \
  --compare ~/memory-eval/results/change.json
python3 scripts/memory_eval.py report ~/memory-eval/results/change.json --show-unjudged

# How much the old measurement leaked, on the same set.
python3 scripts/memory_eval.py run ... --as-of now --out ~/memory-eval/results/leaky.json

# Do agents use what the hook delivered? Aggregates only.
python3 scripts/memory_eval.py realized --project ~/Desktop/next --since 2026-09-01
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
| `--onnx-excerpts DIR` / `--onnx-python FILE` | Maintainer-only excerpt pilot with local assets and an explicit isolated worker. Supply both; default is off. |
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
developer message that starts like a capsule) and ends at the next human
prompt. `--json` prints the numbers as JSON, `--paths` adds per-path counts.

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
- **realized** - per host: prompt-hook turns, turns with a capsule and with
  documents, delivered items the agent touched with a tool and mentioned in
  its text, by kind (skill, brain, chunk, task, spec-doc, changelog);
  `context.py retrieve` calls and Memory MCP `memory_retrieve` calls by the
  agent; and, as a baseline, documents of those kinds the agent opened that
  the capsule did not deliver.

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
- The clock pin covers Python's `datetime`; a runtime reading `time.time()`
  for dates would see the real clock.
- `<cache>/trees/` and `<cache>/history/` (a full clone per project) persist
  across runs and grow with them; delete the cache to reclaim the space.

## Privacy

Sets, judgments, passages, transcripts and results are client data. Keep them
outside this repository (`run` warns when an input or the output is inside
the clone, and refuses a cache there). A result carries prompt ids, document
paths and counts only - never prompt text, capsule text or document bodies;
the stderr tail withholds any line that quotes the prompt. `realized` prints
aggregates; document paths only with `--paths`. The cache holds full copies
of client projects: per-prompt corpora are removed as soon as the prompt is
scored (unless `--keep`), the tree cache and history clones stay until the
cache is deleted.


## Diagnosing misses

First keep Python and the runtime unchanged and run the same frozen set with
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
`scripts/memory_outcome_ab.py` measures that on tasks that really happened.

**What a pair is.**
- **The task:** the first prompt of a recorded Claude Code session whose turn
  changed at least one file the prompt does not name. Its *gold* files are
  those changed files. Each one must be:
  - a path the project's Git has known;
  - kept by the runtime's recent-files filter;
  - not part of the memory system itself.
- **The corpus:** the project rebuilt as of the prompt with this stand's
  reconstruction. Recent files are seeded from the project's earlier sessions,
  as the Stop hook would have recorded them.
- **The arms:** a headless `codex exec` session runs the same prompt twice:
  - `off`, with no hooks;
  - `capsule`, with only the memory prompt hook (`working-memory-read.sh`).

  The arms therefore differ by the Task Capsule alone. The policy's own
  `context.py retrieve` stays available in both.
- **The `navigation` arm** runs the same hook with `capsule_profile:
  navigation` in the corpus: working state and recent files, no documents.
  It writes no manifest, so its runs are recognised by the line every hook
  refresh appends to `refresh-health.ndjson`.
- **Comparing arms:** `report --compare BASELINE,TREATMENT` compares any two
  arms of a result. The rule below then reads `<treatment>-helps` /
  `<treatment>-hurts`. Each run also records the token estimate of the
  documents the hook's manifest delivered.
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

**Privacy.** Sets carry prompts and results name client paths. Both are
written owner-only and are refused inside the clone. `--events` keeps raw
sessions for a pilot and belongs in a private folder.

## Optional ONNX excerpt experiment

Use a reranker when an eligible document is already found but its ranking or
quoted window loses the answer. If it is absent from the candidate set, measure
candidate expansion with a hybrid search separately. An excerpt reranker cannot
recover an absent document, widen the history slot or enlarge the link cap.

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
The former late-render `ExcerptPilot(context, runtime, scorer)` callable remains
available as a lexical-only compatibility shim; the CLI uses `StatefulExcerptPilot`.
This is a maintainer experiment and is never included in installer inventories.

Provision the worker in a separate environment outside the checkout, using
Python 3.12+ for these pinned packages (NumPy 2.5.3 requires it; ONNX Runtime
1.31 itself needs 3.11+). Python 3.14 works without an interpreter downgrade:

```bash
python3 -m venv ~/memory-eval/onnx-venv
~/memory-eval/onnx-venv/bin/python -m pip install -r requirements-memory-onnx.txt
python3 scripts/memory_eval.py run ... --diagnose-misses \
  --onnx-excerpts ~/memory-eval/models/minilm-l6 \
  --onnx-python ~/memory-eval/onnx-venv/bin/python \
  --onnx-timeout 5 --out ~/memory-eval/results/onnx.json
```

Neither the evaluator nor the worker downloads models or installs dependencies.
Keep existing system/Harness interpreters and environments intact. The worker
alone imports NumPy, ONNX Runtime and tokenizers; CPU inference and tokenizer
parallelism are limited to one thread. Its budget includes startup, blocked stdin,
asset verification, import, model loading, inference and bounded cleanup, using
one absolute deadline without pipe-reader threads. Cleanup terminates the owned
POSIX process group even if its leader has exited; real Linux workers also guard
against parent death. Detached descendants outside that group and Windows
process-tree cleanup are not guaranteed. The evaluator's `--timeout` supplies
the whole invocation limit, with cooperative termination before forced kill;
the direct pilot defaults to a 30-second invocation and a 5-second worker.
Synchronous OS delays are not a universal hard deadline guarantee. Busy, missing assets/dependencies, mismatched hashes,
malformed/non-finite logits and worker errors return no scores and leave lexical
excerpts intact. Optional stats-file errors cannot fail a successful refresh.

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
[`memory_onnx.py`](../scripts/memory_onnx.py), as well as the actual local bytes.
Inert provenance fields may remain; the worker never fetches their URLs.
The [publisher model tree](https://huggingface.co/jinaai/jina-reranker-v1-tiny-en/tree/aca45de6945b5dc6399abcd2a9c55ded5dc9111f)
provides the pinned ONNX and tokenizer assets; publisher Python is not executed.

This separate profile requires the retained Python 3.14 worker and exact
NumPy 2.5.3, ONNX Runtime 1.31.0 and tokenizers 0.23.3 versions. A version
mismatch returns the ordinary dependency fallback. Its tokenizer is
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

Freeze model/assets, settings, labels, prompt-time snapshots and the scorer
before measuring. Use development queries first; only a passing configuration
may run the separate fixed replay. A reused replay is not a fresh blind holdout.
Report paired passage delivery, useful/noise paths, unexpected fallbacks and
complete refresh latency (including cold model startup); a model smoke test is
not evidence of retrieval improvement. Keep models, queries, client corpora and
result JSON outside Git. CI exercises stdlib fixtures and fake workers, without
ML downloads or dependencies.
