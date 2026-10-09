# Memory Evaluation Stand

`scripts/memory_eval.py` measures what the prompt-time memory hook delivers,
on each project **as it was when the prompt was written**. It is a
repository-level maintainer tool: it is never installed into a project, it
calls no model and no network, and CI runs only its tests
(`tests/test_memory_eval.py`), never the stand itself.

## Why it exists

The read hook (`working-memory-read.sh`) puts a Task Capsule - excerpts of
the project's memory - in front of every prompt. Earlier measurements on real
prompts indexed each project's *current* state. Knowledge written after a
prompt, often about that prompt's own work - the finding its session
resolved, the chunk promoted from it - was then "retrieved" for it. That
leakage from the future inflated the useful turns three to four times. Every
change to retrieval must be measured on the project as it was at the prompt;
this stand rebuilds that state, overlays the runtime under test from this
clone, runs the refresh the hook runs, and scores the result against human
judgments.

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
   are not materialized.
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
7. **The score.** See [What the numbers mean](#what-the-numbers-mean).

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
| `--cache DIR` | Work and cache directory; refused inside this clone. |
| `--timeout SECONDS` | Per runtime call (default 30). A timeout skips the prompt as `refresh-timeout`. |
| `--keep` | Keep each prompt's corpus and record its path in the item, for debugging. |

`run` prints the report of its result; it exits 0 when at least one prompt
was evaluated, 1 when none was, 2 on a usage error.

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

## Data formats

**Set** - a JSON list; other keys (`session`, `cwd`, ...) are ignored:

```json
[{"id": "next-017", "project": "next", "ts": "2026-08-05T12:00:00Z", "prompt": "..."}]
```

**Judgments** - `{prompt id: {path: grade}}`, grade 0, 1 or 2; 1 and above is
useful, 0 is noise; a grade that is not an integer is ignored. Skill paths in
any tool tree (`.claude/skills/`, `.cursor/skills/`, `.codex/skills/`) count
as `.agents/skills/`.

**Passages** - `{prompt id: {path: {"useful": true, "passages": ["..."]}}}`:
the sentences that make a document useful for that prompt.

**Result** - `{"meta": ..., "items": {id: ...}, "summary": ...}`. `meta` names
the edition(s) with their release, a digest of the edition files the overlay
installs and whether the edition directory had uncommitted changes, this
clone's commit, the host, `as_of`, the clock, the SHA-256 of the set,
judgments and passages files, and the start and finish times. Each item has
`status` (`ok` or `skipped` with a `reason`), `edition` and `edition_source`,
`commit`, `provenance` (`from_git`, `from_worktree`, `dropped_future`,
`undetermined`, `updated_after`; `config_from_worktree` when the working
tree's `runtime.json` was used; under `--as-of now`, `future_present`), the
overlay counts, `history_linked`,
`refresh` (exit status, seconds, the warm-up's seconds, the runtime's phase
timings, a warning count, a stderr tail with any line quoting the prompt
withheld), and the score below. `summary` is what `report` prints.

## What the numbers mean

- **delivered** - item paths of the capsule's three layers, normalised.
  `useful_delivered` have a grade of 1 or more, `noise_delivered` a grade of
  0, `unjudged` none.
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
  could-answer" is the delivery rate against that stricter ceiling.
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

Grow the judgments with `report --show-unjudged`: it lists every delivered
`(id, path)` pair without a grade.

## Limits

- The commit is found from the project's current `HEAD`: work on a branch
  not merged into it is not seen, and history rewritten since is what it is
  now.
- Only memory is recovered from the working tree. Uncommitted code, specs and
  task documents at the prompt are not reconstructed; a record or chunk that
  existed then and was deleted since is lost.
- A record created before the prompt and edited after it carries its newer
  text (`updated_after` counts the ones that say so); a chunk re-verified
  after the prompt is refused by the pinned clock as verified "in the
  future", which errs towards less, not more.
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
