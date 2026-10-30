# Changelog

- Restore only the 89 historical `Task/` assignment files (epics, backlogs and design inputs); keep applications and generated project work untracked and all `Task/` material excluded from installed accelerator payloads.

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

### A capsule's health line counts the bytes it printed — 2026-10-30

- The capsule shape in `refresh-health.ndjson` gains `bytes`: the size of
  the capsule as printed, by `refresh` and by a printing `hook-context`
  (`None` for `--json`). Its `tokens` estimate counts documents only, so a
  recall or navigation capsule of working state and recent files read 0
  whatever it printed. Counts only, as before.
- `memory_eval.py funnel` prints the median printed bytes beside the median
  tokens, and `memory_outcome_ab.py` records each run's `capsule_bytes` and
  prints their mean among the arm's costs: the sealed ticket run promises to
  report what its capsules cost, and the document estimate cannot say.

### Chat snapshots leave out the Codex context the turn reader leaves out — 2026-10-30

- `context_continuity.py` kept as the person's words four envelopes Codex
  writes into the user role that `context.py`'s turn reader takes for Codex's
  own since 8b20d5f1: the plugins it recommends (`recommended_plugins`), the
  user instructions beside AGENTS.md (`user_instructions`), an app's notice
  that blocks edits (`external_codex_apps_writing_block_edits`) and a
  subagent's report (`subagent_notification`). They came into every chat
  snapshot that held them. A test now holds the two lists together and names
  where they differ on purpose: the answer to the agent's question continues
  a turn and is still the person's, and a notification starts a turn and is
  not.

### `realized` reads a retrieve's answer inside Codex's output wrapper — 2026-10-30

- Codex heads what a command printed with its own lines ("Script completed"
  or "Script running with cell ...", the wall time, "Output:"): every one of
  the 75 retrieves in next's September sessions came back so, and a `--json`
  answer then did not start with `{` and was not read. `realized` now reads
  past those lines, and of several JSON documents one call printed (`start
  --json` before the retrieve) takes the one with memory layers. The counts
  of 2026-10-30 hold: the 17 JSON answers held one capsule.

### A verified record keeps the versions that were checked, and says who checked — 2026-10-30

- `update_record` took every source fingerprint anew on any write, so a
  `brain-update --progress` that only tidied a verified finding's wording
  moved its fingerprints to the current files: retrieval stopped marking it
  "cited file changed", and a re-promotion carried the new digests into the
  chunk, for a claim nobody had checked again. A verified finding, bug,
  incident or decision now keeps the fingerprints taken when it was checked
  through every write that checks nothing (progress, next steps, a lifecycle
  move), and only a source cited for the first time is fingerprinted then.
  An observed record follows its sources as before.
- `brain-create --authority verified` and `brain-update --authority
  verified` recorded nobody as the checker; the one real knowledge corpus in
  a client project was written that way. They now record `agent` unless
  `--attestation person` says otherwise. `brain-update --reverify` checks a
  verified claim again: its fingerprints are taken anew and the ledger says
  who checked. A record created verified writes its attestation into an
  `observed` -> `verified` ledger entry, which every runtime's validator
  accepts.
- A person's verified claim is revised only with attestation `person`, as
  `record-result` already held it: before, a `brain-update --progress` from
  an agent rewrote it silently.
- The Harness passes `--attestation=person` for what a person does on its
  project and session pages, where the runtime declares
  `BRAIN_ATTESTATION_FLAG` near the top of `context.py`; an older runtime is
  not handed the flag.

### The maintainer tools measure the retrieves agents make and what a save costs — 2026-10-30

- `memory_eval.py realized` reads every explicit retrieve in the sessions it
  reads - `context.py retrieve` through a shell and the Memory MCP's
  `memory_retrieve`, in a capsule turn or not - pairs each call with its
  answer by the host's call ID and counts, per host and kind, the documents
  the answers named and how many a later tool call opened, the task records
  they pointed at, and as a baseline the documents opened that no retrieve
  named. In next's Codex sessions since 2026-09-01 (75 calls in 36 sessions)
  the answers named 25 task records, 9 opened afterwards, and 4 findings and
  chunks, none opened: the one memory path agents were seen using is the
  working state, and no tool read it before.
- A capsule's delivered documents are the items it lists as memory, in the
  current form (`- memory|history|skill|policy <path> — <title>`) or the
  September one (a `semantic:` line with the items indented under it, and
  `weak-match:`). Paths on the other lines - recent and ticket files, the
  task's sources, the last turn, an excerpt - were counted as delivered
  (next, September: 65 items, now 54); the `task record:` pointer has its own
  row, and a task's record is the kind `task-record`, apart from `brain`.
- A Codex capsule after the prompt hook's "Previous native memory result was
  not saved completely." was no turn at all; it is now, and so is one Codex
  marks `hooks.additional_context`. A developer message Codex marks as
  something else is no capsule whatever its first word, and "Memory refresh
  unavailable" opens a turn without a capsule.
- The native save cost line adds, where the host records them, the save's and
  the answer's input tokens and how many came from the cache (Claude Code:
  fresh, cache-written and cache-read). On the 43 saves of 2026-10-24 on
  copies of two client projects the median save re-read 176,782 input tokens,
  175,616 cached, beside its 662 output tokens and 17.1 s.
- `memory_outcome_ab.py`: the `capsule` arm names `capsule_profile: full`. It
  left the profile to the corpus, and since the editions ship `recall`
  (2026-10-22) it would have measured recall under its old name. `report`
  prints each arm's cost outside the verdict: mean seconds, on how many runs
  Codex reported usage (never on a run stopped at the gold touch), the mean
  tokens over those, and the documents' token estimate.
- `docs/MEMORY-EVAL.md` says the stand counts calls to the first touch of a
  changed file and does not see whether the task got done.

### A ticket files line in the capsule, an experiment switched off — 2026-10-30

- `ticket_files: true` in `runtime.json` (or `CONTEXT_TICKET_FILES=1`; `0`
  turns a configured one off) adds `ticket files (CMS-741): ...` beside the
  recent files: the files of the newest commits on any ref whose message names
  a ticket of the request, or of the task's ID upper-cased, matched in any
  case as a word of its own; present, kept as recent files are, not named in
  the request and not already shown; ten at most within 720 characters. Off
  by default: it changes what a capsule spends, so it waits for a task-outcome
  measurement (`docs/MEMORY-EVAL.md`, "Accepting a retrieval change").
- Why this one: on 155 cold starts of four client projects (before
  2026-10-07) it was the most precise list - 9.4% of the files it showed were
  changed by the turn, against 6.1% of the recent files - and found a changed
  file in 14 tasks the recent files missed; with co-change of the files a
  prompt names, 23 (55 to 78 tasks) at the width of twenty recent files (68).
  Co-change of the recent files themselves did no better than ten more recent
  files and is not built.
- Cost: `git log --grep` adds 35 to 60 ms a prompt on repositories of 4,000
  to 5,000 commits (the largest client clone's refresh, median 83 to 143 ms).
  A Git failure or its two-second timeout leaves the line out.
- The capsule shape in `refresh-health.ndjson` counts `ticket_files`, and
  `memory_eval.py funnel` has "with ticket files".
- `scripts/memory_outcome_ab.py`: arm `ticket` (recall plus the line); `sample
  --require-ticket-commits` keeps the cold starts it can act on; a task records
  its session's `branch`, and the corpus branch keeps that ticket
  (`ab/<id>-CMS-741`); `report --compare recall,ticket` counts the pairs whose
  capsule carried the line. The measurement is sealed before it runs (SHA-256
  `ff43fdbad191b748feacfce2a2a087d3bfdabb71108804ed250d2fdca9cd1a6f`):
  `recall` against `ticket` on the same projects' cold starts from
  2026-10-31, drawn once at least 30 exist. On 8 corpora of the tuning set the
  ticket arm's capsule showed the line and the recall arm's did not.

### A Codex turn begins where its rollout says, not at Codex's own context — 2026-10-30

- The Stop hook keeps the files of the transcript's latest turn, which began
  after the last user message the runtime took for a prompt. Codex also
  writes its own context in the user role - its `AGENTS.md` instructions and
  recommended plugins, an app page, a skill's text, a Stop hook's
  continuation, a subagent's report, the answer to its question, a stopped
  turn's note - and each was taken for a prompt: on the 3,099 rollouts of
  the maintainer machine, 229 of 6,298 Codex turns with calls lost the calls
  made before one, 5,307 of 131,774. `_transcript_prompt` now leaves out a
  message that is wholly such context (the instructions, or one of those
  envelopes). Context around the person's own request (`# In app browser:`
  or `# Files mentioned by the user:` above `## My request`,
  `<in-app-browser-context>` with the request after it) and a turn Codex
  starts itself (a notification, a heartbeat) still start a turn.
- A Codex turn begins at the rollout's own `task_started`: the 29 turns the
  prompt rule alone still cut were a message the person sent while the turn
  ran, which Codex adds to that turn. Without the event in the window
  (Claude Code, a turn longer than the window) the turn begins at its prompt
  as before.
- `scripts/memory_outcome_ab.py` had its own test for a Codex prompt, which
  also dropped context around the person's request: 484 such prompts on the
  maintainer machine. It now takes the runtime's and leaves out only a turn
  Codex starts itself. Before 2026-10-07 the four clients give 268 Codex
  recall sources (242 before) and still 38 Codex cold starts, 6 of them
  different: the old test had passed over the session's first prompt.

### The evaluation tools read Codex's compressed rollouts and draw Codex tasks — 2026-10-30

- Codex keeps a finished rollout as `rollout-*.jsonl.zst`: 3,040 of the
  3,099 on the maintainer machine, the four client projects' 1,837 Codex
  sessions among them. `scripts/memory_eval.py` read only `*.jsonl`, so
  `realized` reported those clients' Codex history as empty. It now reads
  compressed rollouts through Python 3.14's `compression.zstd`, or else the
  `zstd` command, and says how many it could not read without either.
- With them, over the four clients: Claude Code, 386 sessions, 1 of 289
  delivered documents opened, no `context.py` call by an agent; Codex, 1,837
  sessions, 25 of 67 opened, 474 `context.py` calls (66 of them `retrieve`).
  "Delivered documents go unused" held for Claude Code alone.
- `scripts/memory_outcome_ab.py`: `sample --hosts claude,codex` and
  `recall-sources --hosts` draw from Codex rollouts too, compressed ones
  included; the default stays `claude`, so a sealed sample draws the same
  tasks. Each task records `seed_hosts`, the sessions its recent files are
  seeded from (`run --codex-root`); a task from before them seeds from
  Claude Code alone.
- A Codex prompt is what a person wrote: the stand reads a Codex message's
  text, and a message Codex puts in the user role (`AGENTS.md`
  instructions, recommended plugins, notifications, a hook's continuation
  prompt) no longer starts a turn. 2,310 of the 3,099 rollouts began with
  one; before the fix the four clients gave 15 Codex cold starts, after it
  38 (37 with 1-10 gold files, none in an earlier sample), and 242 Codex
  recall sources beside Claude Code's 105.

### A memory funnel across projects, from their journals — 2026-10-30

- `scripts/memory_eval.py funnel --project A --project B [--since DAY] [--json]`
  adds up how far memory gets in several projects at once, from their
  journals alone: the prompt hook's runs, timeouts and failures; the capsules
  built, how many carried the working state, recent files or documents, by
  profile, and their median tokens; the saves armed, requested, made, refused,
  failed or omitted, with learnings written and chunks promoted; and the task
  records, findings, decisions and bank chunks the project keeps. It reads no
  transcript, path, query or record text and labels a project by its folder
  name, so a teammate can send the result; `realized` stays the transcript
  side.
- A `refresh` or `hook-context` line in `refresh-health.ndjson` now carries
  the capsule's shape as counts: `{"profile", "working", "recent_files",
  "documents", "tokens"}`. A line from an older runtime has none and the
  funnel counts it as `unknown`.
- Run over the four client projects after syncing them to this stack on
  2026-10-30: since 2026-09-01, 640 prompt hook runs (5 timed out), 663
  capsules with no shape recorded, no save journal yet, 56 task records,
  77 findings and decisions, 5 bank chunks.

### Each turn mends a bank index that a stopped write left behind — 2026-10-30

- Why: a promotion, a retire or a re-verify writes the chunk first and
  `memory-bank/INDEX.md` second. Its snapshot restore covers an `Exception`
  only. A host that stopped the write between the two, by Ctrl-C, a budget
  or a kill, left a chunk the index did not name, or named as it had been.
  The bank's `validate.py`, which a team may run in CI, then failed until
  the next bank write. Found by a longer lifecycle campaign: 24 installed
  seeds of 120 steps with stops and lock waits. The last save of seed
  13018 was stopped by SIGINT between the two writes.
- What changes: the turn checkpoint each Stop hook runs (`context.py
  turn`) first compares the index's time with its newest chunk's. When a
  chunk is newer, it writes the index again through `reindex_bank`, which
  takes the lock the stopped write held and rewrites only what differs. An
  index Git checked out after its chunks, as a pull does, is only marked as
  current, so later turns read the chunks' times and nothing more.
- Tests: `tests/test_memory_native_capture.py` drops a promoted chunk's row
  from the index and makes the index older than the chunk. One turn
  restores the row and the bank validates. Before, the row stayed missing.

### A save the Memory MCP refuses names its code — 2026-10-30

- Why: the Memory MCP answers a refused tool call with fixed text and,
  as its comment says, with the typed code, the one thing an agent can act
  on and never the submitted content. It found the code only when the error
  message was itself a code. A refused save carries its code apart from
  the message (`ResultRefused.code`), so every refusal of
  `memory_record_result` read "Memory operation failed" with no reason. A
  progress too long, a pasted log, a missing source and half a character all
  looked the same. Found by calling the installed server with refused saves.
- What changes: the server takes the code through `refusal_code`, as the
  native save's journal does. The answer names it, for example
  `(text_limit)`, and still never the submitted text.
- Tests: `tests/test_memory_mcp.py` makes three refused saves and finds
  each code, without the submitted text, in the answer. Before, none of
  the three named its code.

### A bisect keeps the branch it started from as the task — 2026-10-30

- Why: a bisect tests commits on a detached HEAD. A detached HEAD names no
  task except during a rebase and in a worktree Codex made. So during a
  bisect, on Claude Code, Codex and Cursor alike, the prompt hook delivered
  no capsule at all, not even the working state, and the Stop hook asked
  for no save until `git bisect reset`. Yet what those turns find, such as
  the commit that broke something, is about the branch the bisect started
  from. Found by running whole turns through a bisect.
- What changes: the hooks' `rebased_branch` and the runtime's
  `_bisected_branch` read `BISECT_START`, which Git keeps in the worktree's
  own Git directory until the reset. A branch name there is the task, and
  the runtime admits it only while the bisect runs, as it admits a rebase's
  branch. A commit ID there (a bisect started on a detached HEAD) names no
  task. `status` reports the task with the source `git-bisect`. The
  generator's hook fragment (`hook-forge`) carries the same lines, and a
  test holds it to the editions' hooks. Those lines take the generator's
  skill bodies to 307,171 bytes, past their 306,800-byte ceiling, so the
  ceiling in `scripts/token_budget.json` rises to 307,300.
- Tests:
  - `tests/test_memory_git_reuse.py` refreshes the capsule and reads
    `status` during a bisect. Before, the refresh failed with "Inferred task
    branch changed or unavailable".
  - `memory-bank/tests/test_hooks.py` runs the prompt and Stop hooks of
    every host during a bisect. Before, they named no task.
  - `tests/test_memory_prompt_generator.py` compares the documented
    resolver with every edition's hooks.

### The agent channel of a branch two task records claim is the one its capsule serves — 2026-10-30

- Why: when two copies of a branch each start its task record and then
  merge, the task ID names both records. The capsule, the turn checkpoint
  and the native save already settle on one of them
  (`served_task_record`). `msg-send`, `msg-read` and `msg-dispatch` still
  asked for the task ID alone. In a clone without a binding (a fresh clone,
  or one whose local index was rebuilt) they failed with "identifier is
  ambiguous". An orchestrator could then read none of its subagents'
  results, and the dispatch hook reported each completion lost, until a
  person cancelled a twin. Found by a random sequence of memory commands
  with pasted texts.
- What changes: the channel commands take the record the capsule serves.
  When there is none, they fall back as before to the binding, then to
  the task ID, so a finished task's journal stays readable.
- Tests: `project-brain/tests/test_runtime.py` sends, dispatches and reads
  on a branch two records claim, from a clone without a binding. Before,
  `msg-send` failed as ambiguous.

### Text holding half of a character is refused by name — 2026-10-30

- Why: a JSON `\ud83d` with no partner, such as an emoji cut in two by a
  tool that truncates text, decodes to a lone surrogate. So does a byte
  that is not UTF-8 in a command argument. No UTF-8 file or SQLite text
  holds one, so a save failed on a `UnicodeEncodeError` traceback while
  hashing the request. The journal recorded that as `unclassified`, and
  `msg-send` and `record` failed the same way. Found by saving texts
  that agents paste (Unicode line separators, conflict markers, fences,
  control and bidi characters) and checking `validate`, the capsule and
  search after each save. This was the only failure.
- What changes: such text is refused before anything is written. A save
  is refused with the new code `memory-result-v1:text_unicode` (listed in
  `docs/MEMORY-INTEGRATION.md`). A Brain write or channel message is
  refused through `guard_shared_text`, and a lightweight write through
  `reject_secrets`. Each says the text "holds half of a character" and asks
  for the whole character or none.
- Tests: `tests/test_memory_native_capture.py` adds the case to its
  refusal table (before: a traceback and no code).
  `memory-bank/tests/test_channel.py` and `test_context.py` send a message
  and record an episode with a byte that is not UTF-8 (POSIX only, since
  Windows arguments are Unicode).

### A channel message holding a Unicode line separator no longer breaks the channel — 2026-10-30

- Why: JSON leaves U+2028, U+2029 and U+0085 unescaped inside a string,
  and the agent channel's readers split the journal with `str.splitlines`,
  which also splits a line at them. `msg-send` accepted such a message,
  for example text pasted from a PDF or a web page. After that, every
  `msg-read` failed on the journal and so did `validate`, for the whole
  project and for good.
- What changes: the journal is split at newlines alone (`journal_lines`),
  in `msg-read`, `msg-send`'s sequence count and `validate`. The three
  characters are written escaped (` `), so a runtime from before this
  change, or a tool that splits the same way, reads new entries too.
- Tests: `memory-bank/tests/test_channel.py` sends a message holding all
  three and reads and validates it as written. It then does the same with
  the characters written raw, as an earlier runtime wrote them. Before,
  reading failed with "line 1 is not valid JSON".

### A channel message a full disk cuts short is taken back — 2026-10-30

- Why: a full disk takes part of an append. When `msg-send` failed that way,
  the part it had written stayed at the end of the task's agent channel
  (`project-brain/control/messages/<task>.jsonl`, tracked in Git). Once the
  disk had room again, every `msg-read` called the journal corrupt and
  `validate` failed on it ("Unterminated string"). The next message was
  appended after the broken part, so the two shared one line for good. Found
  by running the memory's commands and hooks on a nearly full tmpfs; the
  hooks, saves, checkpoints and the index all failed cleanly there and
  recovered.
- What changes: an append that fails is truncated back to where the file
  ended, under the mutation lock it already held. A line that an earlier
  append left cut short is named, in `msg-read`, `validate` and `msg-send`,
  as cut short by an append that stopped part-way, with the advice to
  remove it; `msg-send` no longer appends after it. A last line that is whole
  but lacks its newline gets one before the next entry. `validate` names a
  merge conflict in a channel as `msg-read` does.
- Tests: `memory-bank/tests/test_channel.py` cuts an append short with a
  file size limit (`RLIMIT_FSIZE`, POSIX only). Before, the journal kept 300
  bytes of it and failed validation; now it is unchanged. It also reads,
  validates and appends to a journal left cut short, and validates one left
  in a conflict.

### A file named outside UTF-8 no longer stops every turn's checkpoint — 2026-10-30

- Why: `git status` gives a file's name as bytes. A name that is not UTF-8,
  such as one from a tree unpacked from another system's archive, came back
  with lone surrogates, and SQLite refused them in the turn buffer. While
  such a file was changed or untracked, every `context.py turn` failed. The
  Stop hook's checkpoint runs `turn` and always exits 0, so no turn was
  buffered or flushed, the capsule's recent files stopped, and nothing said
  so.
- What changes: such a name keeps its odd bytes as `\xNN` escapes
  (`caf\xe9.php`), in the buffer and in the task's file list.
- Tests: `memory-bank/tests/test_context.py` flushes a turn with an
  untracked `caf\xe9.txt`. It runs on Linux only, because macOS and Windows
  keep no such name. Before, the turn exited 1 with a `UnicodeEncodeError`.

### An index, a chunk or a channel left in a merge conflict says so — 2026-10-30

- Why: every promotion rewrites the generated `memory-bank/INDEX.md`, so two
  clones of a branch that both promoted leave it in a merge conflict on pull.
  The bank's `validate.py` read that as a duplicate row and every chunk
  "missing from INDEX.md", and nothing said what had happened; a team that
  runs it in CI saw a failure about all of its memory.
- What changes: an index holding Git conflict markers is one error, saying
  the index is generated from the chunks and that `context.py reindex-bank`
  writes it again (any bank write does: a save that promotes, `reindex-bank`,
  a retire); the per-chunk index checks wait for a readable index.
- A chunk two clones both re-verified or retired conflicts in its
  frontmatter; it read as "frontmatter is not valid JSON-compatible YAML".
  It now says it holds an unresolved merge conflict, to keep one side or
  merge both.
- A task's agent channel (`project-brain/control/messages/<task>.jsonl`),
  which two clones both appended to, conflicts at its end; `msg-read` and
  `msg-dispatch` called it "corrupt". They now say the line holds an
  unresolved merge conflict. The subagent gate's write lock never depended
  on it: the dispatch hook releases the lock before it journals.
- Tests: `memory-bank/tests/test_validate.py` conflicts an index row (one
  error naming the conflict and `reindex-bank`; before, two that did not)
  and a chunk's `last_verified` (the error names the conflict; before, the
  JSON parser's position); `memory-bank/tests/test_channel.py` conflicts a
  channel's last line (before: "corrupt").

### A learning's citation is kept as the file is spelled — 2026-10-30

- Why: a citation was stored as the agent wrote it. `src/a.php`,
  `./src/a.php` and `src//a.php` were three keys for one file, and on a file
  system that ignores case - macOS's, Windows' - `SRC/a.php` was found,
  stored and committed, then failed validation in every clone on Linux (the
  chunk left retrieval there). `src\a.php`, as an agent on Windows writes
  it, was refused with "use project-relative source paths", which it is.
- What changes: `memory_results.check_source` returns the path as it is
  spelled on disk - each part the entry of that name, else the one entry
  that differs only in case - and a result's sources are stored that way,
  two spellings of one file once. A citation not found as written is
  refused, or kept as written where the file is not required, as before. A
  backslash is refused with the path written with forward slashes.
- Tests: `tests/test_memory_native_capture.py` saves learnings citing
  `./rules.md`, `.//rules.md` and `RULES.md` (stored `rules.md`) and refuses
  `docs\rules.md` naming `docs/rules.md`; it fails on the check before.

### A task record left in a merge conflict is not taken for no task — 2026-10-30

- Why: two clones of a branch that both checkpointed its task conflict in
  that record and its handoff when one pulls the other. A record that fails
  to parse kept the identity it claims only while its frontmatter still
  parsed, and with conflict markers it did not: a lookup by the branch said
  "not found". A clone without a binding was told "working: not recorded
  yet", and the save its Stop then asked for made a second task for the
  branch beside the one being merged - a twin once the conflict was
  resolved. Found with two clones on `main` that had synced, each saving,
  then a pull.
- What changes: a record's claimed identity is read from either side of an
  unresolved Git merge conflict (`brain_runtime.merge_sides`), so a lookup
  says the record is invalid, and the error says why: "unresolved Git merge
  conflict". The capsule of a clone without a binding says so as the bound
  clone's always did, leading with the file ("Unresolved Git merge conflict
  in project-brain/dynamic/tasks/<uuid>.md: keep one side or merge both"),
  and a native Stop asks no save until the file is repaired (`skipped`
  `task-unreadable`, documented in `docs/MEMORY-INTEGRATION.md` with the
  `interrupted` refusal).
- Tests: `project-brain/tests/test_runtime.py` conflicts a task record: the
  lookup is not "not found", the capsule names the conflict and the file,
  and a checkpoint makes no second task. `tests/test_memory_native_capture.py`:
  the Stop asks no save and makes no task. Both fail before.

### A branch two task records claim is saved where its capsule says — 2026-10-30

- Why: two copies of a branch - two people on `main`, a branch worked on
  from two machines - each start the branch's task record before they
  sync, and once they merge the task ID names both. The capsule already
  settles on one (the clone's binding, else the earliest created active
  record) and says so, but a native Stop and save asked for the task ID
  alone and failed as ambiguous: the Stop hook journaled an unclassified
  error and exited 1, and no save was asked on that branch, in any clone,
  until a person cancelled a twin. Found by two clones on `main`, each
  saving, then a third clone.
- What changes: where the task ID is ambiguous, the native Stop and save
  take the record `context.served_task_record` names - the one the capsule
  serves and the turn checkpoint writes - by its UUID. Anything else that
  makes the ID ambiguous (an unreadable record claiming it) keeps the
  refusal.
- Tests: `tests/test_memory_native_capture.py` adds a twin created before
  the first record; a clone bound to the first saves into it, and a clone
  without a binding saves into the earlier twin, which its capsule names.
  Both fail on the native path before ("identifier is ambiguous").
- The capsule's advice now names the same twin in every clone: never the
  earliest created record, which a clone without a binding settles on.
  Each clone named "the other" record before, so the clone bound to the
  later twin and one bound to the earlier one cancelled one each - both -
  and the branch was left with no active task. Cancelled, the later twin's
  clone follows the earliest (its binding's successor).
  `project-brain/tests/test_runtime.py` binds a clone to the later twin:
  it is told to cancel its own, and then works with the earliest; on the
  advice before, it was told to cancel the earliest.
- Not changed: the conflicts a merge leaves in the derived
  `project-brain/indexes/active.json` and `memory-bank/INDEX.md`.

### A line cut inside a character no longer stops the refresh health series — 2026-10-29

- Why: the prompt hook appends one line per refresh to
  `memory-bank/local/refresh-health.ndjson` and trims it to its retention.
  Both the trim and `context.py health` read the file as strict UTF-8, so
  one line a full disk or a crash cut inside a character (a Cyrillic branch
  name, say) made `health` fail with `UnicodeDecodeError` and stopped every
  later trim: the series grew for good, and every prompt read it whole.
- What changes: both read past such a line - it is one bad line, which the
  reader skips as it skips one that is not JSON - and the trim replaces the
  file whole (`atomic_write_bytes`) instead of rewriting it in place, which a
  crash could leave empty.
- Tests: `tests/test_memory_health_notices.py` reads a series with such a
  line (two turns counted, `health` answers) and appends to one longer than
  its retention (trimmed, valid UTF-8). Both fail on the reader before.

### The lifecycle fuzzer runs hooks behind another session's lock — 2026-10-29

- Why: the lost save fixed below needed a hook's own budget to run out
  while the save waited on another session's lock. `--interrupt` stops a
  hook at a random moment, never while it waits, and its 36 seeds found
  nothing; the loss was found by reading the drain.
- What changes: `scripts/memory_lifecycle_fuzz.py --contention SHARE` runs
  that share of the turns outside a burst and not interrupted with their
  prompt hook or the Stop after the save behind another session's hold on
  the runtime's lock (1.2-2.5 s) and under a one-second hook budget. A save
  a sandbox held whose Stop ran out of its budget must be made by the
  session's next prompt hook. A hook out of its budget may say so, once
  each: the Stop's journal line with `timeout`'s 124, a refresh reported
  unavailable. A degraded refresh is counted under the fuzzer's lock: with
  16 sessions in a burst, increments lost between threads read as refreshes
  that failed (3 of 8 in one run).
- Found: on the drains before the fix below, 8 of 12 seeds (60 steps,
  `--contention 0.4`) lost a save a sandbox held; on the fix none did, and
  12 such saves were made by the next prompt hook.
- Tests: the `memory-lifecycles` job's interrupted seed is now 24 with
  `--interrupt 0.3 --contention 0.3`: on the drains before the fix it loses
  its held save.

### A save or checkpoint a sandbox held survives a hook its budget stops — 2026-10-29

- Why: a save the agent's sandbox could not write waits in the spool, and a
  checkpoint (`update`, `turn --flush`) in the command queue, until a hook
  outside the sandbox makes it. Both drains removed the file before the save
  or the command ran ("one attempt each"). The hooks run them under
  `timeout` with a 5 s budget, so one that waited on another session's
  runtime lock - an index build, a long flush - was ended with its file
  already gone: the save or checkpoint was lost, and nothing was journaled.
  Reproduced with the lock held and a 1 s budget: 3 of 3 spooled saves and
  2 of 2 queued checkpoints never made, by that hook or any later one.
- What changes: a drain renames the file for its attempt and removes it
  once the save or command has answered, whatever it said, so an attempt a
  budget or a host cuts off leaves it to the next hook, which begins it
  again. A save that had landed is only repeated - the request's token keys
  it - and a waiting `update` is made once unless its hook was stopped after
  the command finished and before the file went. After three beginnings the
  save or command is given up as refused with code `interrupted`; a
  command's refusal is named in the next capsule, as other refusals are.
  While it works a drain holds a lock file beside the files, taken without
  waiting and let go by the system however the drain ends: the queue is the
  project's, so two sessions' Stops drain it at once, and the second took a
  file the first was still making - waiting on the lock - for a stopped
  drain's and made the command twice. Now the second leaves the queue to
  the first. The lock file is made only when there is something to drain.
- Tests: `tests/test_memory_native_capture.py` holds the runtime lock while
  a Stop drains a spooled save, ends the Stop's process group as `timeout`
  does, and has the next prompt hook make the save; a save three hooks
  began is given up as `interrupted`. `project-brain/tests/test_runtime.py`
  (`SandboxQueueTest`) does the same for a queued `update`, checks that the
  refusal reaches the capsule, and has two Stops drain one queued `update`
  at once while the lock is held: made once, one revision. Each fails on the
  drains before (the last one on the drain without its lock).
- Not covered: a drain whose hook runs Python without `timeout` (macOS
  without coreutils) is in the hook's own process group; a host that kills
  that group mid-drain is covered by the same rename, a machine that loses
  power by nothing new.

### The lifecycle fuzzer stops hooks and saves part-way — 2026-10-29

- Why: a host stops a hook at its timeout (Claude Code gives the Stop hooks
  8 s), when its window closes and at Ctrl+C, and a save, a turn buffer or
  an index write can be cut anywhere. The fuzzer only ever let a hook finish.
- What changes: `scripts/memory_lifecycle_fuzz.py --interrupt SHARE` stops
  one step of that share of the turns outside a burst - the prompt hook,
  the Stop hook, the save or the Stop after it - by SIGKILL to the hook's
  process group, SIGTERM to its shell alone or SIGINT to the group, after up
  to 0.3 s. A save stopped part-way may or may not land; a save a sandbox
  held whose Stop was stopped must be made by the session's next prompt
  hook. Off by default, so every other seed takes the steps it always took.
- Found: nothing in the runtime. 36 seeds with a third of their turns
  stopped (24 installed at 70 steps, 12 attached at 60): 1078 saves, 224
  hooks and 55 saves stopped, no violation. The spool's drain removes a held
  save's file before it makes the save, but the hooks run Python under
  `timeout`, which leads a process group of its own, so a kill of the
  hook's group does not reach it: across 36 kills 1.5 ms apart over the
  drain, every save was made by that Stop or by the next prompt hook.
  Without `timeout` (macOS without coreutils) the drain runs in the hook's
  group, and a group kill between its two steps would lose that save.
- Tests: `tests/test_memory_lifecycle_fuzz.py` runs seed 6 with
  `--interrupt 0.3` in the `memory-lifecycles` job: a prompt hook, a Stop
  hook, a save and the Stop after a save stopped, by each of the three
  signals, most of them while they ran.

### A commit in a worktree Codex made no longer refuses the save — 2026-10-29

- Why: a native save is pinned when the Stop asks for it: to the branch,
  and on a detached HEAD to the commit. A worktree Codex makes stays
  detached, and the thread that owns it names the task
  (`codex-thread/<thread>`), so a commit there - the person's, or another
  session's - moved the pin and refused the save, learnings and all. The
  lifecycle fuzzer found it once the two worktree stacks met: 2 of 96 seeds
  at 80 steps, one of them a save the sandbox held that its Stop then could
  not make.
- What changes: on a detached HEAD whose Git directory holds a
  `codex-thread.json` naming its owner, the save is pinned to that thread,
  read by the patterns `context.py` reads it by and in the same `git
  rev-parse` call. A worktree handed to another thread refuses the save, as
  a branch made there does. A stopped rebase and any other detached HEAD
  keep the commit, and their pin is the same value as before.
- Tests: `tests/test_memory_native_capture.py` commits in such a worktree
  while the save is written, and the save is made; on the old pin it is
  refused as `scope-changed`. A worktree handed to another thread refuses
  it; on the old pin that save was made. The two fuzzer seeds pass.

### The lifecycle fuzzer holds saves in a sandbox, as Claude Code's does — 2026-10-28

- Why: a save a sandbox cannot write waits in the spool, and the session's
  next hook makes it. The benches drive that once per run; the fuzzer
  never did, between other sessions' saves, commits and branch moves.
- What changes: where `bwrap` is, `scripts/memory_lifecycle_fuzz.py` runs a
  fifth of the saves outside a burst under bubblewrap, everything read-only
  but the temporary folder, as Claude Code's sandbox on Linux. The save
  must say `spooled`, and the Stop after it must make it under the branch's
  task. Read-only memory folders alone are no stand-in for that sandbox:
  the save opens the index, whose file they leave writable, before it
  takes its lock, and fails on the database's journal. The CI job
  `memory-lifecycles` installs bubblewrap and lifts the runner's AppArmor
  restriction, as `system-orchestration` does; its test asserts a held
  save where `bwrap` is. The draw is apart from the run's own random
  stream, so the seeds keep their steps.
- Tests: seeds 1 and 2 at 60 steps hold 7 and 9 saves, all made by the Stop
  after them, with no violation; `tests.test_check` accepts the job's
  setup lines.

### The memory hooks' Python imports nothing from the project's folder — 2026-10-28

- Why: the prompt hook, the Stop hook and the session banner run Python
  snippets in the session's folder, and `python -c` puts that folder first
  on `sys.path`. A project's own `json.py` was imported before the standard
  one, on every prompt. Measured: a `json.py` in the project root wrote its
  marker, and the hook went on as usual. Attached - the Harness lends the
  edition to any folder it opens - that is code the person never trusted.
- What changes: those snippets run with `-I`: no session folder, no
  `PYTHON*` variables, no user site. The validators get the same in their
  own change, from `main`. The runtime's CLI is untouched: run as a script,
  it imports from its own folder.
- Tests: `PythonResolutionTest` runs the three hooks with a `json.py` in the
  session's folder. It is not imported, and without `-I` the test fails. The
  Windows banner test now sets the stream at the top of the program
  `python` runs, which `-I` leaves alone, where `PYTHONPATH` it ignores.

### A session that entered a worktree keeps the root's permission rules, measured — 2026-10-28

- Why: the worktree stack said, as a limit it had not measured, that after
  EnterWorktree the root's permission rules decide by the root's paths, so
  the worktree's `secrets/` and `.env` would go unguarded.
- What changes: `scripts/claude_code_bench.py` takes `--enter-worktree` with
  `--permissions`. A session in the root enters a worktree, tries the reads,
  writes, commands and fetches of `--permissions` on the worktree's files,
  and passes when each gets the decision it gets in the root. The report
  carries what EnterWorktree answered. `docs/MEMORY-INTEGRATION.md` drops
  the limit and says what was measured.
- Tests: Claude Code 2.1.278 offline. EnterWorktree answered "The session is
  now working in the worktree", and 19 of 19 decisions matched the root's:
  the worktree's `.env`, `.env.local` and `secrets/` denied, `rm -rf`,
  `sudo` and `chmod 777` denied, its own files read. Claude Code takes `./`
  in a rule from the session's folder, which EnterWorktree moves.

### The Codex bench passes a read-only turn that asks for no save — 2026-10-28

- Why: `scripts/codex_bench.py --sandbox read-only` shows that the Stop skips
  the save request where the turn can write nothing, but its verdict was
  still "saved", so the run that showed the right thing exited 1.
- What changes: under `--sandbox read-only` the run passes when the stand-in
  got no save step and the journal says `stop:skipped:read-only-sandbox`.
- Tests: on Codex 0.163 offline the run exits 0 with exactly those two
  events; the workspace-write runs are unchanged.

### The lifecycle fuzzer drives Codex in worktrees and held checkpoints — 2026-10-28

- Why: the fuzzer drove Claude Code in worktrees, but Codex only in the root
  and below it, never a worktree Codex makes, and never the explicit memory
  commands. That is the stack carried onto the worktree stack the same day,
  which its own tests covered one piece at a time.
- What changes: `scripts/memory_lifecycle_fuzz.py` also runs Codex sessions
  in worktrees, through the Codex dispatcher as a root Codex session sets it
  up. It makes worktrees as Codex does: on a detached HEAD, the owning
  thread in `codex-thread.json`, so the task is `codex-thread/<thread>`. And
  it makes checkpoints as the checkpoint skill does, half of them under
  read-only memory folders. A checkpoint a sandbox held must be made by the
  session's Stop; one never made, or refused, is a violation.
  `tests/test_memory_lifecycle_fuzz.py` asserts the new coverage on seeds 1
  and 2, and its attached seed is 9 (4 no longer reached a Codex session in
  the root; of 18 attached seeds none had a violation, 5 cover every path
  the test names); `scripts/README.md` and `docs/CI.md` say what it drives.
- Tests: seeds 1 to 3 at 60 steps, and a campaign of 24 seeds at 80 steps
  over the four editions, a quarter attached: 936 saves, 29 Codex worktree
  sessions, 23 worktrees Codex made, 23 checkpoints held and made, no
  violation.

### The session banner reads the index Windows names — 2026-10-28

- Why: `local-context.sh` takes its status fields from Python's `print`,
  which on Windows ends the line with `\r\n`. The last field, the index's
  path, kept the `\r` and named no file, so the banner of every session on
  Windows said `index=missing` over an index that was there.
- What changes: the four editions' `local-context.sh` write the fields as
  UTF-8 bytes, as the memory hooks' snippets do.
- Tests: `PythonResolutionTest` runs the banner with `python` writing its
  stream as on Windows (`sitecustomize`: cp1252, `\r\n`) and a status that
  names an existing index; on the old hook it reads `index=missing`.

### Codex in worktrees and the explicit memory commands, on the worktree stack — 2026-10-28

- Why: two stacks grew apart from the same base. One served Codex started in
  a worktree, and in a worktree Codex made on a detached HEAD, and had the
  explicit memory commands work there (a write a sandbox refuses waits for
  the next hook, the checkpoint names the hooks' task); the other served
  Claude Code's `EnterWorktree`, saves another session or a commit overtook,
  and attached sessions entering worktrees. Each was tested apart.
- What changes: the first stack's fifteen commits are carried onto the second
  (the commits both had are kept once). Where they met: a Codex save step in
  the shared layout names the roots' variables before the command and the
  worktree after `--root` (`--project`); the Stop hook takes the served
  worktree first and then the task - the branch, a rebase's branch, the
  thread that owns a worktree Codex made; `claude_code_bench.py` takes
  `--enter-worktree` and `--cli checkpoint` both. A command the sandbox holds
  for a worktree keeps the name (`--project=<worktree>` in what waits), so it
  is made for the worktree it was asked for. `docs/MEMORY-INTEGRATION.md` no
  longer says Codex started in a worktree runs no hooks.
- The generator's skill bodies: each stack added to `hook-forge` within the
  306,200-byte ceiling, together 306,727; the ceiling rises to 306,800.
- Tests: `SharedWorktree` holds an update for an entered worktree under a
  sandbox and makes it at the hook's turn there; without the name in what
  waits it fails.

### The lifecycle fuzzer runs as a CI job of its own — 2026-10-28

- Why: as a step of the `memory` job it took about three minutes on a
  hosted runner, and with it that job's Python 3.x leg ran into its
  15-minute limit and was cancelled on two pull requests (15m7s and
  15m16s; 9 to 11 minutes before the step).
- What changes: `tests.test_memory_lifecycle_fuzz` runs in a job of its own,
  `memory-lifecycles`, on the same two Pythons with a 10-minute limit, beside
  `memory`; `scripts/check.py` has the group of the same name, and
  `docs/CI.md` and `scripts/README.md` say where it runs.
- Tests: `tests.test_check` (40 tests) holds `check.py` to the workflow.

### The hooks read their input as bytes on Windows, and name the drive as Windows does — 2026-10-28

- Why: the Windows CI job ran the tests of a session that entered a
  worktree for the first time and failed them. The prompt and Stop hooks
  hand their JSON input to Python, which writes back one field per line;
  on Windows a text stream writes `\n` as `\r\n`, so every field bash cut
  kept a `\r`. The session's folder was no folder (an entered worktree
  was never seen), the transcript path no path (the transcript's recent
  files were never read there), and the Stop hook's continuation flag read
  `1\r`, so a save's continuation checkpointed the turn a second time.
  Python also decoded the input by the ANSI code page. And the dispatcher
  compared the session's folder with a listed root case-sensitively for an
  entered worktree's allow: the list's `C:\` reads `/C/` there, the
  folder `/c/`. With those fixed, the save step named the worktree
  `c:\...` beside a root `C:\...`: the hooks and the dispatcher took
  the drive letter from Git Bash's `/c/` as it was.
- What changes: both snippets read their input as bytes and write their
  fields as UTF-8 bytes; the dispatcher matches the root by a pattern, as
  its other comparisons do (`nocasematch` on Windows). Where the hooks
  and the dispatcher turn `/c/` into `C:`, the letter is upper case, as
  Windows writes it (`claude_dispatch.py`'s `native_path` already said
  so). Version 4 has not shipped: its fingerprint is recorded again.
- Tests: the Windows CI job's `tests.test_memory_worktree_sessions` and
  `tests.test_memory_claude_subfolders`, which failed (four failures, one
  error), then one: the drive letter, and on the runner the first
  refresh in the worktree, which builds its index, over the hooks'
  5-second budget. The test gives the hooks 60 seconds, as
  `tests.test_windows_memory_hooks` does. Not run on Windows locally.

### A launched session keeps its memory in a worktree it enters, and its shell where it was — 2026-10-27

- Why: a Claude Code session launched with the edition lent from the
  clone (attached: the Harness's native launches, `accelerator_attach.py
  run claude`) that called `EnterWorktree` read "Memory not consulted"
  from then on: the hooks served an entered worktree only for an
  installation in the root. Driving it showed a second fault in every
  attached session: the save step's `cd <clone>` - the clone is a working
  directory the launch adds - left Claude Code's shell in the clone, so
  every command the agent ran after a save, for the rest of a session a
  person drives, ran in the clone, and the Stop that settles the save no
  longer saw an entered worktree.
- What changes: the lent hooks serve a worktree an attached session
  entered with the project's one state, which keeps the project and all
  its worktrees: `ACCELERATOR_PROJECT_DIR` names the worktree, the home
  and the state stay as launched. `context.py --project` takes, attached,
  a checkout of the project's repository at the project's place in it
  with no runtime of its own (`workspace_roots.attached_worktree`). A
  save step asked in an entered worktree names it and pipes without
  `--input -`, attached as installed. An attached Claude Code save step
  that goes through ends by going back to the folder the session was in
  (`&& cd <folder>`), and a refused one keeps its exit status; `cd -` is
  refused in an entered worktree.
- Benches: `scripts/claude_code_bench.py --attached --enter-worktree` (now
  allowed together) has the stand-in enter a worktree and save there: on
  Claude Code 2.1.278 in default, acceptEdits, auto, dontAsk and
  bypassPermissions, and on 2.1.299 in the desktop app's mode, the save
  landed under the worktree's branch in the project's state with no
  permission prompt, the Stop after it saw it saved, and project and
  worktree stayed clean; `--attached`, `--enter-worktree` and `--worktree`
  still pass. Offline probes of the binary: after the step's `cd` into the
  clone, `pwd` answered the clone; after `&& cd <folder>`, the folder.
- Tests: `tests/test_memory_worktree_sessions.py` drives an attached
  session that enters a worktree through two turns and the save step;
  before the change its Stop asked for no save and its prompt read
  "Memory not consulted". The lifecycle fuzzer lets attached sessions
  enter worktrees: 24 seeds of 60 steps in each layout pass (attached: 699
  saves, 29 worktrees entered); a first version that went back with `;`
  hid refused saves behind the `cd`'s exit status, which it caught.
  `tests/test_memory_lifecycle_fuzz.py` runs seed 4 attached, whose root
  session enters a worktree.

### Two refreshes that find one new document both index it — 2026-10-27

- Why: a refresh read which documents the index held without the write
  lock, and wrote its changes in a transaction begun later. When two
  sessions' prompt hooks refreshed at once after a document appeared -
  most often the Brain record one of them had just saved - the second
  inserted the rows the first had committed meanwhile:
  "UNIQUE constraint failed: document_metadata.path". Its capsule went out
  with the working state alone (procedural, semantic and episodic
  `failed`), and the hook's health line recorded exit 1. The lifecycle
  fuzzer's new health check found it: 45 of 120 runs of 80 steps.
- What changes: before it writes, `index_documents` takes the write lock
  and reads again what the index holds, so the rows it drops and the rows
  it writes are one transaction's. A refresh that finds nothing to change
  still decides that without the lock, so a prompt is not held up behind
  another session's write; one that another session's lock keeps past its
  share of the hook's budget still falls back to the working state and
  says so, as before.
- `scripts/memory_lifecycle_fuzz.py` reads the prompt hook's health lines
  and Cursor's rule: a refresh that exited non-zero without the degraded
  capsule that says why, or timed out, fails the run; a degraded one is
  counted (`refresh degraded`).
- Tests: `memory-bank/tests/test_context.py` runs two incremental
  refreshes that meet at that read; on the indexer before the change one
  of them fails with the constraint, 3 times in 3.

### A save survives another session's writes and a commit — 2026-10-26

- Why: a native save is pinned when the Stop asks for it, and two things
  that happen in the seconds the agent takes to write it refused it. The
  task's revision moved - the other session's save, the turn checkpoint a
  Stop hook writes every fifth turn of a checkout or, in a new project, the
  other session creating the branch task - or a commit landed: the request
  was pinned to the HEAD commit, so the person's commit, or another
  session's on the branch, refused it as `native-result:scope-changed`.
  Nothing of the save was written, its learnings included, and the agent
  was told to report the held save to a person who had already read the
  answer; a save a moment later replaced the other's progress all the same.
  Neither was driven by any test: the new lifecycle fuzzer found both.
- What changes: under the save's lock, where nothing else writes, a save
  made within ten minutes of its request (`IN_FLIGHT_SECONDS`, the window
  in which a prompt sent meanwhile treats it as still being written) takes
  the task's current revision, as the Harness retries its own, and a task
  created since the request takes it as well; the journal records it as
  `saved` with code `rebased`. Its progress and next steps replace the
  other session's, and its learnings are recorded as any are. A later
  save - spooled until a hook long after - still yields to what was
  written since, and a task replaced since the request (completed and
  started again) still refuses it with `native-result:task-changed`. The
  request is pinned to its branch; to its commit only on a detached HEAD,
  as in the middle of a rebase, and a branch switched in between still
  refuses it. `record-result` keeps its revision fence.
  `docs/MEMORY-INTEGRATION.md` records the decision this replaces.
- `scripts/memory_lifecycle_fuzz.py` (new) installs PHP Core into a
  scratch project and drives random steps from a seed through the real
  hooks: turns of Claude Code sessions in the root, below it and in
  worktrees (through the dispatcher), a root session entering a worktree,
  Codex and Cursor sessions, bursts of turns at once, saves made, skipped
  and ignored, commits, branches made, renamed, detached and moved under a
  pending save, a rebase stopped on a conflict and then continued or
  aborted, worktrees added and removed. Every hook must exit 0 with no
  error in the native journal, every save land in its branch's task (in a
  stopped rebase, the branch being rebased), a sample of the saved
  learnings be found by `search` from the root and from every worktree
  left, no index of a removed worktree outlive the next turn checkpoint,
  Project Brain and the Memory Bank validate and no worktree be written
  into. `--edition` runs it on Laravel, Symfony or WordPress; `--attached`
  launches every session as the Harness does, the edition lent from the
  clone and the state apart, and neither the project nor the clone may be
  written into. Bursts put two sessions in one folder too. On 24 seeds of
  60 steps, installed and attached, nothing failed, nor on seed 1 of the
  other editions; before the change, seeds lost saves to both refusals.
- Tests: `tests/test_memory_native_capture.py` - two sessions saving in the
  same seconds, a turn checkpoint in between, a task the other session
  created and a commit made while the save is written, each failing
  without the change, and a replaced task, a save delayed past ten minutes
  and a moved detached HEAD still refused. `tests/test_memory_lifecycle_fuzz.py`
  runs seeds 1 and 2, and seed 2 attached, in a step of its own in the
  `memory` job (about two minutes); on the runtime before the change seed
  1 fails by the first refusal and seed 2 by the second.

### A session that entered a worktree keeps its memory there — 2026-10-26

- Claude Code's `EnterWorktree` moves a session that started in the project
  root into `.claude/worktrees/<name>`, on a branch of its own. The root's
  own hooks keep running there - `CLAUDE_PROJECT_DIR` the root, the input's
  `cwd` the worktree - and saw another checkout of the repository: from
  that turn on the session got no capsule and saved nothing. They now serve
  a worktree that has none of the installation as the dispatcher serves a
  session started in it (shared layout, the worktree's branch, its banner
  after `/clear` or a compaction); one with an installation of its own is
  left to it, as before, and so is a Cursor session there, whose capsule
  would be a rule written into the worktree.
- On Windows the hooks hand the runtime the root and the worktree as
  Windows spells them (`c:/...`), as the dispatcher does: Git Bash's `/c/...`
  read as a folder on the current drive, and the runtime would have taken
  the root's own layout and task.
- The turn that enters the worktree was armed by its prompt in the root:
  its Stop arms the request in the worktree (`native-stop --entered`), so
  that turn's work - often all of it - is saved under the worktree's branch.
- The save step names the worktree (`context.py --project <worktree>`, a
  global option taken only for a worktree the installation serves), since
  run from the root it shows none of it, and in a worktree pipes the
  result without naming the descriptor: once a session has entered a
  worktree, Claude Code refuses "a program fed a descriptor it names"
  (`--input -`) in every permission mode. `native-result` reads stdin when
  given neither `--input` nor `--skip`.
- Claude Code asks before a command that leaves an entered worktree, as the
  save step's `cd <root>` does. The dispatcher (version 4) gives the memory
  CLI alone the allow it has in the root there; nothing else, and nothing
  for a session in the root itself, whose every Bash call it now reads
  without starting a process (1.7 ms a call, as with version 3).
- `tests/test_memory_worktree_sessions.py` runs the hooks as Claude Code does
  after `EnterWorktree`, for the entering turn and the next one, and the
  save step from the worktree; `tests/test_memory_claude_subfolders.py` the
  dispatcher's allow and what it refuses. `scripts/claude_code_bench.py
  --enter-worktree` has the stand-in enter a worktree first: 2.1.278, and
  2.1.299 in the desktop app's mode, saved under the worktree's branch with
  no permission prompt and the worktree left clean. A first session's own
  setting up of the dispatcher made 2.1.278 reread its settings in the
  worktree, which has no hooks; the bench now enters after a first session.

### An attached project's worktree keeps an index of its own — 2026-10-26

- The Harness keeps one state for a project and all its checkouts, and the
  runtime kept one index in it. The checkouts' documents differ, so a lookup
  after one in the other checkout rebuilt the index: 580 ms against 250 ms a
  lookup on 400 notes, at every switch between the project and a worktree.
  An attached project's linked worktree now keeps its index in
  `memory-bank/local/worktrees/<hash>/` of the state, as an installation's
  worktree does in the installation, named and removed the same way; the
  project's own checkout keeps the index where it was.
  `workspace_roots.linked_worktree` recognises the worktree as the
  dispatcher does. `tests/test_memory_worktree_sessions.py` (`AttachedWorktree`)
  fails without the change.

### Explicit memory commands in a worktree: writes a sandbox refuses wait, the checkpoint names the hooks' task — 2026-10-26

- The memory commands an agent runs itself - the policy's stage-boundary
  `update`, the `checkpoint` skill's `turn --flush` and `update` - failed in
  a worktree the installation serves. With the CLI named by its path in the
  root (entries below), three things still stood in the way. Codex's
  workspace-write sandbox writes only the worktree, `/tmp` and `$TMPDIR`:
  under Codex 0.164's own sandbox (`codex sandbox -P :workspace`) `update`
  failed at the Brain's lock (`Read-only file system:
  project-brain/local/runtime.lock`) and `turn --flush` at the index
  (`attempt to write a readonly database`), and Claude Code's sandbox does
  the same there. The skill stopped on a detached HEAD, where the hooks name
  `codex-thread/<thread>` or a rebase's branch. And on a task with nothing
  buffered yet - a new branch, a worktree's first answer, in the root too -
  `turn --flush` made no task, so the `update` after it was refused
  ("Working task not found").
- **A write a sandbox refuses waits.** An `update` or a `turn --flush` that
  cannot write the Brain's lock folder (governed) and the index's is checked
  as far as nothing has to be written - the task ID, a changed field, the
  secret and personal-data refusals - and waits, owner-only, in
  `ai-accelerator-memory-queue-<uid>` in the first temporary folder the
  sandbox writes (`$TMPDIR`, `/tmp`, Claude Code's `claude-<uid>`: the
  save's spool folders, now `workspace_roots.spool_directories`). It answers
  `queued` and exits 0. The next turn a hook records - this turn's Stop,
  outside the sandbox - makes what waits for its project (the state it
  writes and the checkout it reads), oldest first, after the turn and
  before the Stop's save request. Each is journaled (`native-results.ndjson`,
  stage `queue`); one refused is named in the next capsule's last-turn line.
  Only these two commands, rebuilt from their own options, for this project,
  younger than a day; a file naming anything else, or a link, is refused
  unread, and a command a drain runs fails rather than waiting again. A
  drain with nothing waiting costs 0.07 ms a turn.
- **The checkpoint names the hooks' task.** Its step 1 takes
  `CONTEXT_TASK_ID`, otherwise the branch, and on a detached HEAD the
  `automatic_memory.task_identity.task_id` of `context.py status --json`,
  which now names a rebase's branch too (source `git-rebase`) where it told
  the person to set `CONTEXT_TASK_ID`. The lightweight checkpoint and the
  generator's (`skill-forge/references/php-process-skills.md`) resolve it
  the same way; the skill reports a `queued` command as made when the turn
  ends. It and the `memory` skill took the mode from
  `project-brain/config/runtime.json` under `git rev-parse --show-toplevel`,
  which a worktree the installation serves, or a project an accelerator is
  lent to, does not hold; both take the `mode` `status --json` reports now.
- **An explicit flush makes the task**, as the memory MCP server's
  checkpoint already did; the Stop hook never flushes explicitly, so
  visiting a branch still mints nothing, and a completed task is not made
  again.
- Offline, scripted model, the same stand-in before and after
  (`--cli checkpoint` in `scripts/claude_code_bench.py` and
  `scripts/codex_bench.py`, `scripts/bench_checkpoint.py`): it makes a
  checkpoint as the installed skill says - step 1, `turn --flush`,
  `update` - with the memory CLI the session was told of, else the
  policy's. Before (912a0bbf): in `.claude/worktrees/<name>` under Claude
  Code 2.1.299 and in a worktree inside and beside the root under Codex
  0.164, the flush failed (`can't open file
  '<worktree>/memory-bank/scripts/context.py'`); in a worktree Codex made,
  the skill stopped on the detached HEAD. The save after the answer worked
  in all four. After: every checkpoint landed in the root's Project
  Brain under the task of the checkout it ran in - the worktree's branch,
  or `codex-thread/<thread>` - in those four, in Claude Code's desktop
  mode (no permission prompt), with its sandbox on, under the Codex
  app-server (no approval asked) and in the project root of a fresh
  install of either host, where the `update` had been refused ("Working
  task not found"). Under either sandbox both writes answered `queued`
  and the Stop made both (journal `queue made` twice); the save after
  the answer, the capsule, the policy and a clean worktree held in every
  run.
- `project-brain/tests/test_runtime.py` (`SandboxQueueTest`, read-only
  folders standing in for the sandbox; an explicit flush on a new branch)
  and `memory-bank/tests/test_checkpoint.py` (the prompt hook's `--task-id`
  against the skill's step 1 for a branch, `CONTEXT_TASK_ID`, both rebase
  backends, both owner records and a worktree Codex made);
  `tests/test_host_benches.py` holds the stand-in. Each of twelve
  pieces taken out fails a test: the rebase in `status`, the skill's step 1
  and its `queued` line, the queue, the drain, the command allow-list, the
  capsule line, the drain's no-wait guard, the queue-time refusals, the
  explicit flush, and each skill's mode from `status`.
- Not covered: the Codex app was not driven (the bench makes its worktree
  as the app does); Cursor in a worktree still runs no hooks; `refresh`,
  which the `memory` skill runs, reports its layers failed in such a sandbox
  - the hooks refresh the index on every prompt.

### A sandboxed lookup of a task publishes its manifest locally — 2026-10-26

- A lookup that the sandbox keeps from the index (see the entry below) still
  failed for a task that exists: `retrieve` publishes what it delivered to
  `project-brain/control/retrieval-manifests` under the Project Brain's
  lock, which the sandbox does not let it take. Claude Code 2.1.299 in the
  desktop app's mode, with the sandbox on and started in `src/`, got
  `[Errno 30] Read-only file system` on `project-brain/local/runtime.lock`.
  Such a lookup is now `--ephemeral`: its manifest is the local one,
  written when it can be. `tests/test_memory_worktree_sessions.py` repeats
  it with a task and the state read-only, and fails without the change;
  `claude_code_bench.py --subfolder src --cli --sandbox` passes on 2.1.278
  and on 2.1.299 in the desktop app's mode.

### A session below the root hears where the memory CLI is — 2026-10-26

- A session started below the project root runs its commands in its own
  folder, where the policy's `python3 memory-bank/scripts/context.py` names
  nothing. `local-context.sh` now names the CLI by its path in the root
  there too, as it does in a worktree. It reads the session's folder from
  its input's `cwd`, since the dispatcher runs it in the root and Codex in
  the session's folder. A session in the root hears nothing new, and where
  its input names the root as the hook spells it, the hook starts no
  process to compare them.
- A worktree's banner also says that the `memory-bank/` and
  `project-brain/` paths the policy and skills name are in the root: the
  `checkpoint` skill, for one, reads `project-brain/config/runtime.json`
  from the checkout it is in.
- `tests/test_memory_worktree_sessions.py` runs the hook both ways - in the
  session's folder, as Codex does, and in the root, as the dispatcher
  does - and runs the command it names from `src/`. `claude_code_bench.py
  --cli` now takes `--subfolder` alone: 2.1.278, and 2.1.299 in the desktop
  app's mode, ran it from `src/` without a permission prompt for the
  root's branch.

### The memory CLI in a worktree finds the worktree by itself — 2026-10-26

- The policy and the skills name the memory CLI as `python3
  memory-bank/scripts/context.py`, which a worktree of an installation Git
  does not track has none of. A worktree session's banner now names it by
  its path in the root - bare, or in single quotes where the path needs
  them, the form the dispatcher's allow for the memory CLI takes (a
  double-quoted path was asked about, since bash expands inside it).
- Run in the worktree without the dispatcher's variables - a Codex
  command, the memory MCP server, a terminal - the root's runtime took the
  installed layout: the main checkout's branch and index, and a citation of
  a file only the worktree's branch has was refused ("Source is missing").
  `workspace_roots.worktree_project` now finds the shared layout from where
  the run is (`CLAUDE_PROJECT_DIR`, which Claude Code gives its hooks and
  MCP servers, else the working directory), with the rules the dispatcher
  applies: a `.git` file naming `<main>/.git/worktrees/<name>` that keeps
  Git's back-reference, this installation inside `<main>`, and no runtime
  of the worktree's own. A session in the main checkout keeps the root
  after its shell moved into a worktree inside it.
- The memory MCP server of a worktree inside the root started for the root
  (and refused to start once the worktree was its project: it checked the
  installation's own files there). It checks them under the installation
  and gives the runtime it starts the worktree's layout, through Claude
  Code's bootstrap and Codex's.
- In a worktree, a learning citing the installation's own files - `specs/`,
  `AGENTS.md`, skills - was refused, though retrieval reads them from the
  main checkout; a citation is checked where `resolve` reads it
  (`workspace_roots.citation_base`).
- Under a sandbox that lets a command write only its own folder - Codex's
  workspace-write in a worktree or below the root, Claude Code's - every
  CLI command failed on the index, a lookup too: Codex 0.164 in a worktree
  got `[Errno 30] Read-only file system`. A lookup (`status`, `retrieve`,
  `context`, `search`) that cannot write the index where the hooks keep it
  builds one of its own in the user's temporary folder
  (`ai-accelerator-index-<uid>`); a write is still refused there.
- `tests/test_memory_worktree_sessions.py` covers each piece (ten reverted
  pieces each fail a test), a root whose path has a space included.
  `scripts/claude_code_bench.py --worktree --cli` has the stand-in run the
  CLI as the banner names it: 2.1.278, and 2.1.299 in the desktop app's
  mode in the worktree and in `src/` of it, ran it without a permission
  prompt for the worktree's task. The bench's desktop verdict no longer
  drops a permission prompt in `--subfolder` and `--worktree` runs.

### A worktree's state spelled another way is still its installation — 2026-10-26

- The Windows memory job on #66 failed twice in
  `tests/test_memory_worktree_sessions.py`. `local-context.sh` told an
  attached project from a worktree by comparing ACCELERATOR_STATE_DIR with
  its own root as text: Git Bash spells the root `/c/...`, the variable
  spelled it `C:\...`, and the banner said "attached from" where the
  worktree shares the installation's memory. The hook now compares where
  the two lead (`pwd -P`, as it already did for ACCELERATOR_HOME) and, when
  the state is this installation, uses the root's spelling; on Linux a
  trailing separator showed the same mistake and is now part of the test.
  The test's cleanup could not remove the worktree's index because the
  test's own SQLite connections stayed open: `with sqlite3.connect(...)`
  ends a transaction, not the connection, and on Windows an open database
  cannot be deleted. They are closed with `contextlib.closing` now.

### A worktree Codex made names its task by its owning thread — 2026-10-26

- `codex --worktree` and `codex exec --worktree` (0.164) check the session's
  `HEAD` out - committed work only - in `$CODEX_HOME/worktrees/<4 hex>/<repo>`
  on a detached HEAD and write the thread that owns the checkout into
  `codex-thread.json` of its Git directory (`{"version":1,"ownerThreadId":
  "<UUID>"}`, the session's own ID). The Codex app makes its worktrees the
  same way - `git worktree add --detach`, the record one key a line - and a
  branch comes there only by an act: its "Create branch" (`codex/` and the
  title's first words), the agent, told to name its branches `codex/...`, or
  "Move to worktree"; "Hand off to local" moves the work to a branch in the
  main checkout and detaches the worktree. A detached HEAD named no task, so
  a session there, the root's hooks run for it by the dispatcher, got neither
  capsule nor save (offline, real Codex 0.164: hooks ran, no journal line).
- The hooks now name such a checkout's task `codex-thread/<UUID>`, after
  `CONTEXT_TASK_ID`, the branch and a rebase's branch:
  `[ -n "$TASK_ID" ] || TASK_ID=$(codex_thread_task)` follows the rebase
  line in every copy of `working-memory-read.sh` and
  `working-memory-write.sh`. Any other detached HEAD still names none. The
  thread, not the folder: it is Codex's own link between thread and
  checkout, unique, and the same for the checkout's life; the folder's four
  hex digits, a name the person gives it and Git's name for the worktree
  come back for other work once a worktree is removed. A branch made there
  names the task from the next hook on; the thread's task is not carried
  over, since the branch travels - to the main checkout at "Hand off to
  local", to a pull request - and the record does not. The hooks pass the
  thread's task as the inferred identity (`--expected-branch`), and the
  runtime delivers its capsule only while `HEAD` stays detached and the
  record still names that thread (`_admit_inferred_task_branch`), as it
  admits a rebase's branch only while the rebase runs. `context.py status`
  reports it (`automatic_memory.task_identity.source` `codex-thread`) where
  it told the person to set `CONTEXT_TASK_ID`; the first capsule, before the
  task is recorded, names it, since no branch does; its goal reads "Work of
  the Codex thread that owns its worktree", where the thread's ID read as
  words came apart at its hyphens. The runtime reads the record by the hooks'
  two patterns, so the two never name different tasks.
- Offline, real Codex 0.164, scripted model, isolated `CODEX_HOME`:
  `scripts/codex_bench.py --worktree codex` (new: `codex exec --worktree`
  makes the worktree) passed - capsule, every line of `AGENTS.md`, a save
  through the spool under the measured session's own `codex-thread/<UUID>`,
  of a learning citing a file only that worktree had, the worktree still
  detached and clean - and so did `--subfolder src` and `--app-server`, where
  the bench makes the worktree as the app does (no approval asked); with the
  hook line taken out the same run got no capsule and no save.
  `--worktree beside` still saves under its branch.
  `memory-bank/tests/test_hooks.py` (`CodexWorktreeOwnerTest`: the CLI's and
  the app's record for each host, the order, twelve refused records, the
  hooks and the runtime reading the same records) and
  `project-brain/tests/test_runtime.py` (`CodexThreadTaskTest`: admission,
  refusal once a branch, another owner or no record, `status`, the capsule
  and the goal) fail on each of eight reverted pieces. The generator's
  memory hook contract (`validate_generated.py`), which the shipped hooks
  must pass, admits the line right after the rebase one only, and hook-forge
  documents the resolver (`tests/test_memory_prompt_generator.py`).
- Not covered: the Codex app was not driven - its worktrees were read from
  its code. When the app hands the record to another thread (it archives the
  owner while another thread still works there), that thread's task starts
  empty. The `checkpoint` skill still stops on a detached HEAD, and the
  explicit `context.py` commands find no runtime in a worktree without the
  installation, branch or not.

### Codex started in a worktree of the project — 2026-10-26

- Codex reads a project's `.codex/` - `hooks.json` and `config.toml` - and
  `AGENTS.md` only in the folders between the session's folder and the
  nearest `.git`, which in a linked worktree is the worktree's own, inside
  the project too. A worktree of an installation Git does not track has none
  of them: Codex 0.164 started in a worktree of an installed project, the
  main checkout's hooks approved, ran none of them - no capsule, no save, no
  guard. A Codex session in the root now sets up, in the background from the
  Codex copy of `local-context.sh`, a dispatcher in the person's own Codex
  folder (`memory-bank/scripts/codex_worktrees.py`, the scripts
  `codex_dispatch.sh` and `.py` copied to `$CODEX_HOME/ai-accelerator/`):
  one entry per hook of the project's `.codex/hooks.json` in
  `$CODEX_HOME/hooks.json`, with its matcher, timeout and context limit, and
  one named `policy`. The installer run in a terminal does it at once
  (`CODEX worktrees <folder>`). The person's own entries keep their place -
  Codex keys an approval by the file and the entry's position - and the
  dispatcher's are written the same way every time, since Codex asks again
  for a changed definition but not for an edited script. Nothing goes into
  `config.toml`: Codex runs the entries only once the person trusts them in
  its own review (the terminal's "Trust all and continue" at start, `/hooks`,
  Settings > Hooks in the app; `codex exec` skips them silently). Nothing is
  written either for hooks kept in `config.toml`, where Codex would warn at
  every start that the layer has two forms, for a `hooks.json` that is not an
  object, once `ai-accelerator/off` exists, for a hook another program
  started, or on Windows.
- The dispatcher serves only a session in a linked worktree of a listed
  root, in the root's folder of it, without an installation of its own: it
  runs the root's `.codex/hooks/<script>` there with `ACCELERATOR_HOME` and
  `ACCELERATOR_STATE_DIR` naming the root and `ACCELERATOR_PROJECT_DIR` the
  worktree, the shared layout Claude Code's dispatcher uses. In the root and
  below it Codex runs the project's own hooks, so there - and in every other
  project and in a launcher's attached session - it exits after a string
  match. `policy` prints the root's `AGENTS.md` whole
  (`additionalContextLimit` 32,768 tokens; Codex's default keeps 2,500) at a
  new, cleared or compacted start, not on resume, whose history replays it,
  and not where the worktree has an `AGENTS.md` of its own.
- `native_results`: Codex has no file a hook can give the session's
  commands variables through, so in the shared layout its save step names
  the three roots in the command. Without them the save checked the main
  checkout's files and refused a learning citing the branch's own file
  (`stop:failed:unsaved`).
- Offline with the real Codex 0.164 and a scripted model, an isolated
  `CODEX_HOME`: the root session set up eleven entries and wrote no
  approval; a worktree session before they were approved ran no hook; after
  "Trust all" a session in `.claude/worktrees/<name>`, in
  `$CODEX_HOME/worktrees/<id>/project` and in `src/` of one got the capsule
  and every line of `AGENTS.md`, and its save went through the spool into the
  root's Project Brain under the worktree's branch, with an index of its own
  and the worktree left clean - through `codex exec` in all four editions,
  and in PHP Core also through `codex app-server` as the desktop app drives
  it (no approval asked) and the terminal with a question typed during the
  save.
  On synced copies of the four client installations the rollout rehearsal
  (`--hosts codex --worktree`, which now rehearses Codex in a worktree too)
  saved under the worktree's branch, recalled it and left the worktree clean
  under both profiles, 8 of 8, and the real Codex passed the bench in a
  worktree of two of them (inside and beside). Where the dispatcher does not
  serve, an entry costs 2.3 ms, 1.2 ms more than an empty hook.
  `scripts/codex_bench.py --worktree [inside|beside]` and `--app-server`
  repeat it; `tests/test_memory_codex_worktrees.py` (28 cases, CI's memory
  job) covers the setup, the dispatcher, the policy and the save step, and
  each of nine reverted pieces fails it. The bench's stand-in now opens its
  stream before it writes: Codex 0.164's terminal, after an earlier session
  in the same `CODEX_HOME`, cut a response silent for 8 seconds.
- Not covered: Codex's own `--worktree` starts on a detached HEAD, which
  named no task (see the entry above); the desktop app's worktrees were not
  run; Cursor in a worktree without the installation still runs no hooks.

### Claude Code's sandbox below the root or in a worktree saves through the spool — 2026-10-26

- Claude Code's Bash sandbox lets a command write only the folder its
  session started in and claude-<uid> under its own temporary directory
  (CLAUDE_CODE_TMPDIR, else the system's), which it gives the command as
  TMPDIR. A save from a session below the project root or in a worktree
  cannot write the project there, so it was spooled into that folder - and
  the next hook, which runs outside the sandbox with the temporary
  directory itself, never looked in it: the turn ended `stop` `failed`
  `unsaved` (Claude Code 2.1.299, offline, with the sandbox on and the
  project on disk). `native_results._spool_dirs` now looks in claude-<uid>
  under each temporary directory too; the command still takes the first
  folder it can write, its own TMPDIR. With it, both sessions saved
  (`saved` `spooled`, then `recovery-saved`). A session in the root, which
  the sandbox lets write the project, saves as before. The attached layout
  - the Harness lending an edition, its state kept outside the project -
  had the same loss under the sandbox, every answer, and saves through the
  spool now too (`--sandbox --attached` fails without the change).
  `tests/test_memory_native_capture.py` repeats it with the two TMPDIRs;
  dropping the new folders fails it. `scripts/claude_code_bench.py
  --sandbox` turns the sandbox on in the bench (the project on disk under
  `/var/tmp`, Claude Code started without the namespace's capabilities):
  on 2.1.278 and 2.1.299 the root saved as before, and below it, in a
  worktree and in the desktop app's mode the save went through the spool.

### Claude Code started in a worktree of the project — 2026-10-25

- Git gives a linked worktree only what it tracks, and none of the four
  real installations checked was tracked: Claude Code's own worktrees
  (`claude --worktree`, the desktop app's `.claude/worktrees/<name>`)
  copy `settings.local.json` and `.worktreeinclude` files, not the
  installation. The real Claude Code 2.1.278 started in such a worktree
  ran no project hook - no project context, no `AGENTS.md`, no capsule,
  no save - and 12 of the person's 90 Claude Code sessions in October
  started in one of its worktrees.
- The dispatcher (`claude_dispatch.sh`, version 3) now serves a session
  started in a linked worktree of a listed root's repository that has no
  installation of its own: it finds the worktree by its `.git` file,
  which names `<main checkout>/.git/worktrees/<name>`, a folder Git keeps
  with a `gitdir` file, without starting a process; a project installed
  in a folder of a monorepo is that folder of the worktree. It runs the
  root's hooks in the worktree with `ACCELERATOR_HOME` and
  `ACCELERATOR_STATE_DIR` naming the root and `ACCELERATOR_PROJECT_DIR`
  the worktree; `environment` gives the session's commands the same
  three variables, so the save step serves the worktree; `permissions`
  decides for the worktree's files by the root's rules, a deny or ask
  also covering the root's own files from there; the policy comes in
  parts whatever Claude Code was told about imports. A forged `.git`
  file, a submodule, a worktree of another repository or one with an
  installation of its own is left alone, and so is a session a launcher
  lends an accelerator to (`ACCELERATOR_HOME` set: the Harness opening a
  worktree of an installed project, which has none of the installation,
  would otherwise run every hook twice). A root the session's folder is
  below, in its own checkout or one around it, comes first: an installed
  worktree of an installed project is served as itself whatever the
  lengths of the two roots - found by a test, before it shipped.
- The dispatcher's permissions entry, which runs before every tool call
  below the root or in a worktree, starts a third faster: `shutil`,
  `subprocess` and `urllib.parse` are imported where they are used, and the
  helper runs isolated and without site (`-I -S`), so a person's
  `PYTHONPATH` cannot shadow the standard library either - 32 ms a call
  before, 22 ms after. The guard, `bash-validator.sh`, now runs there as in
  the root, without the helper, unless the command names `context.py`:
  the helper only adds the allow the project's rules give the memory CLI,
  and its Python start cost every other Bash call five times the guard's
  own - 37 ms before, 7.5 ms after; a block is the guard's as before.
- The runtime's shared layout (`workspace_roots.py`): the three variables
  with the state root equal to the tooling root - an installation lent
  to its own worktree. The Project Brain and Memory Bank are the root's,
  keyed as there (`key_for`); the worktree is the project, whose branch
  names the task; the index is the worktree's own,
  `memory-bank/local/worktrees/<hash>/context.db` (`default_database`);
  the accelerator's policy, skills, `specs/` and `tasks/` are indexed
  from the root, where the worktree has no copy (`source_scan_plan`), and
  a citation of them by the root's key resolves there (`resolve`). The
  installed state keeps its permissions: no owner-only `umask`, no
  `ensure_attached_state`. A file named `worktree` beside the index names
  the worktree (`remember_worktree_index`), and a checkpoint removes the
  index of a worktree that is gone (`prune_worktree_indexes`, beside the
  consolidation and compaction it runs with).
- The hooks tell a worktree from the root by `PROJECT_DIR`, not
  `STATE_DIR`: `local-context.sh` describes the worktree, names the
  installation whose memory it shares and the Memory Bank's files by
  their full path, and `working-memory-read.sh` (with its mirrors' rule in
  `context_retrieval.py`) gives the advice to build the index by hand only
  in the root, where the command it names works.
- Offline with the real Claude Code 2.1.278 and 2.1.299 (the version the
  desktop app bundles; without the dispatcher a worktree session there got
  nothing either), in `.claude/worktrees/<name>`
  and in a folder of a worktree beside the project: project context,
  policy, a capsule with the worktree's own task, and the save into the
  root's Brain under the worktree's branch; 280 randomized permission
  calls in seven rule sets decided in a worktree for its files as in the
  root for the root's. `scripts/memory_rollout_rehearsal.py --worktree`
  runs the hooks through the copy's dispatcher in a worktree of each copy,
  the save with the `environment` entry's variables: on the four client
  installations, under both profiles, every save landed under the
  worktree's branch, was recalled, and left the worktree clean
  (`tests/test_memory_rollout_rehearsal.py` covers it).
  `scripts/claude_code_bench.py --desktop` drives a session as the
  desktop app drives Claude Code (stream-json, the initialize request,
  `--setting-sources user,project,local`, permission prompts over stdio,
  each refused and listed) and fails on any prompt: in the root, below it
  and in a worktree, 2.1.278 and 2.1.299 asked none; with the memory CLI's
  allow taken out it lists the save's command. Left without a mode, 2.1.299
  under those flags decides an ask in auto mode, which needs a classifier
  no offline run has, so the bench names the mode as the app does.
  `scripts/claude_code_bench.py --worktree` (and
  `--worktree --subfolder src`) repeats the host run and passes only when
  the save went to the worktree's branch, the worktree got its own index
  and stayed clean. `tests/test_memory_claude_subfolders.py`
  (`Worktrees`) and the new `tests/test_memory_worktree_sessions.py`, in
  CI's memory job, cover both halves; mutating any of the changes fails
  them.
- Not covered: a session that enters a worktree after starting in the
  root (`EnterWorktree`) keeps the root's hooks, which leave the memory
  out there, as before; Codex and Cursor started in such a worktree run
  no hooks; the memory MCP server, found by the nearest `.mcp.json`,
  serves a worktree inside the root as the root. Documented in
  `docs/MEMORY-INTEGRATION.md` ("Claude Code
  started in a worktree"), `docs/TROUBLESHOOTING.md` item 11 and
  `install/README.md`.

### The spool's journal line, as it lands — 2026-10-25

- Documentation: below the project root Codex's sandbox cannot write the
  project's journal either, so the `save` `spooled` line of 2a31787a is
  lost there and the record is the next hook's `saved` with code
  `spooled` (`docs/MEMORY-INTEGRATION.md`, `docs/TROUBLESHOOTING.md`).
  Found by the Codex bench started below the root.

### Command rules below the project root reach as far as the root's — 2026-10-25

- Why: a differential run - random deny rules in the project's
  `settings.local.json`, random reads, writes and commands, tried in a root
  session and in one started in `src/` of the real Claude Code 2.1.278 -
  found commands the root refused that the dispatcher let through below it:
  `env rm -rf x`, `... | xargs rm -rf`, `if true; then rm -rf x; fi`,
  `{ rm -rf x; }`, `for ...; do rm -rf $d; done`. It also refused two the
  root allows: `command -v sudo` and `xargs -I{} rm -rf {}`.
- What changes: the dispatcher takes from Claude Code's own code what it
  looks through - `time`, `nohup`, `timeout`, `nice`, `stdbuf`, `env`,
  `command`, `builtin`, `noglob` and a bare `xargs` for any rule, `sudo`,
  `doas`, `exec`, `setsid`, `ionice`, `pkexec`, `runuser` and `eval` too,
  each named without a path, with their options, leading assignments and
  the keywords `if`, `then`, `else`, `elif`, `do`, `while`, `until`, `!`,
  `{` - and
  matches each rule against every form of the command along the way, so
  `Bash(sudo:*)` still refuses `timeout 5 sudo ls`. `command -v` and an
  `xargs` with flags, `busybox`, `coproc` and a wrapper named by its path
  are not looked through, as in the root.
- Tests: the cases above and twenty more in
  `tests/test_memory_claude_subfolders.py`; the differential runs (scratch,
  not in CI) afterwards found no command refused in the root and let
  through below it. Claude Code also reads the root's `settings.local.json`
  in a session below it, resolving its paths from that session's folder:
  stricter there, never looser (`docs/MEMORY-INTEGRATION.md`).

### The Codex bench starts below the project root — 2026-10-25

- `scripts/codex_bench.py --subfolder DIR` starts Codex in DIR below the
  project root, in `codex exec` or, with `--queued`, in its terminal: the
  save goes through the spool of 2a31787a and the run passes when the
  journal says `saved` with code `spooled` and the recovery was seen. Its
  work folder is on disk under `~/.cache/ai-infrastructure-benches/`:
  Codex 0.163's bubblewrap sandbox mounts its own `/dev` and writes `/tmp`,
  so a project in `/dev/shm` was invisible to a command started below its
  root (`can't open file .../context.py`) and one in `/tmp` writable. The
  stand-in reads its file from the folder Codex starts in.
- Offline on Codex 0.163: from `src/` in `codex exec` and in the terminal
  with a second question typed during the save (`previous-in-flight`), the
  save landed through the spool; nothing said it was not saved.
  `claude_code_bench.py`'s check of a queued save counts a spooled one as
  landed. The sandbox below the root writes no journal either, so the
  `save` `spooled` line is never there; the next hook's `saved` line is.

### Below the project root on Windows too; the dispatcher keeps its newest copy — 2026-10-25

- Why: Claude Code on Windows runs hook commands with Git Bash, and the
  dispatcher for sessions started below a project's root stayed off there:
  its list holds roots as Python writes them (`C:\Users\me\shop`) while
  Git Bash shows the session's folder as `/c/Users/me/shop`, a bare `bash`
  started from Python is WSL's launcher, `python3` is often missing or the
  Store's stand-in, and Windows names ignore case. And one dispatcher
  serves every project of the person: a session in the root of a project
  with an older release put an older dispatcher back.
- What changes: the dispatcher compares roots in Git Bash's form without
  regard to case, walks up by length so that a difference in case cannot
  keep the walk from ending, runs the project's hooks with the bash it runs
  in, and finds Python as the hooks do (`python`, `py -3`, `python3`). Its
  helper runs a hook with Git for Windows' bash, compares paths and rule
  paths without regard to case and across drives, and looks the import
  approval up under the key Claude Code uses there (forward slashes). The
  setup names the dispatcher with forward slashes, locks with `msvcrt` and
  skips the owner check Windows lacks; the installer sets it up there too.
  `DISPATCH_VERSION` keeps a newer dispatcher in place, and a fingerprint
  test makes any change to its scripts raise it. The policy parts come only
  where the project's `CLAUDE.md` imports `AGENTS.md`.
- Tests: `tests/test_memory_claude_subfolders.py` runs in the Windows CI
  job too, with Git Bash, skipping there only what needs symbolic links,
  file modes, a pseudo-terminal or a JSON reader for `bash-validator.sh`.
  Its first run there found the setup rewriting its files at every start
  (Windows has no file modes to match) and dropping a root when six
  projects started together (the dispatcher's files were written outside
  the lock, and Windows refuses to replace a file another process reads):
  every write is now under the lock, compared by content alone there, and
  tried again for a moment when Windows refuses it. Claude Code itself was
  not run on Windows.

### Below the project root Claude Code gets the project's permission rules and env — 2026-10-25

- Why: the dispatcher of 9f854412 ran the project's hooks for a Claude Code
  session started below the root, but Claude Code applies the project's
  permission rules and `env` only to a session started in it. Offline on
  2.1.278, a session in `src/` in `bypassPermissions` read and edited
  `.env`, read `secrets/` and ran `rm -rf`, `sudo` and `chmod 777`, all of
  which the root denies; in `default` it was asked about reads, writes, `php`
  and a fetch the project allows; and its commands lacked `APP_ENV`. The
  dispatcher was set up only by a first session in the root.
- What changes: a `permissions` entry answers every tool call below a listed
  root with the decision the project's `settings.json` and
  `settings.local.json` make in the root - a deny or an ask wherever they
  would, an allow only for a file inside the root or a single plain command,
  and none against a deny or ask of the person's own or the managed
  settings. Read and Edit paths match as Claude Code 2.1.278 matches them in
  the root, probed with the real binary: gitignore patterns from the root,
  where a trailing `/` or `/**` names a folder at any depth and a pattern
  with another `/` is anchored. Bash rules count for every command of a
  compound one, Agent rules by subagent type, WebFetch ones by domain. An
  `environment` entry writes the project's `env`, but the `CLAUDE_` and
  `ANTHROPIC_` variables, into the file Claude Code names in
  `CLAUDE_ENV_FILE`. The installer, run from the command line in a terminal
  (stdin and stderr a terminal, `| tee` included), sets the dispatcher up at
  once and says so on stderr; a test, a bench, the Harness or a call into the
  module never writes outside the project. Windows stays without it: its
  hooks run through Git Bash, with paths, Python and `bash` this could not
  be checked against here.
- Tests: `tests/test_memory_claude_subfolders.py`, now 38 cases: the path
  patterns as probed, the editions' rules decided below the root as in it,
  local rules, the person's own rules, links, the dispatcher's answer, the
  env export, and the install from a terminal, through `tee`, from a script,
  as a module call and without a configuration folder. Mutations of the
  matching fail them. `scripts/claude_code_bench.py --permissions` tries
  nineteen tool calls in the root and in `src/`: the same decisions in
  `default`, `acceptEdits` and `bypassPermissions` and in all four editions,
  the same `printenv`; without the `permissions` entry ten differ.

### No permission rule Claude Code warns about at every start — 2026-10-25

- Why: Claude Code 2.1 checks file permissions with `Edit(path)` and
  `Read(path)` rules only. The editions' `.claude/settings.json` also
  carried `Write(**)` and `Glob(**)` in `allow` and `Write(.env)`,
  `Write(.env.*)` (WordPress also `Write(wp-config*.php)`) in `deny`, which
  match nothing, and its terminal printed four warnings about them at every
  start - text the person was shown and had nothing to do about. The
  attached launch (`scripts/accelerator_attach.py`) added a fifth,
  `Write(<clone>/**)` beside `Edit(<clone>/**)`.
- What changes: those rules are gone from the four editions and the
  attached launch; Infrastructure-Creator's own settings deny the same
  secret paths with `Edit(...)` rules (its `CHANGELOG.md`). `Edit(**)`,
  `Read(**)`, `Edit(.env)` and the rest already covered every one, so
  nothing allowed or denied changes. The editions' policy locks were
  rewritten.
- Tests: `tests/test_hook_wiring.py` refuses a `Write`, `MultiEdit`,
  `NotebookEdit` or `Glob` rule with a path in any edition's or the
  generator's settings, and fails on the old ones. Offline on Claude Code
  2.1.278: the terminal of an installed project starts with no warning;
  a `Write` to `.env` in `acceptEdits` mode is refused in an edition and in
  the generator, before and after.

### Claude Code started below the project root runs the project's hooks — 2026-10-25

- Why: Claude Code reads `.claude/settings.json` - the hooks and the
  permission rules - only from the folder a session starts in. Started in
  `src/` of an installed project, it ran no capsule, no save and no guard,
  and `.claude/CLAUDE.md`'s `@../AGENTS.md` became an import from outside
  the working directory, which it asks about in a terminal (No by default)
  and leaves out under `-p`. Nothing was to be asked of the person.
- What changes: `local-context.sh`, in a session in the root, runs
  `memory-bank/scripts/claude_subfolders.py` in the background. It copies
  `claude_dispatch.sh` and `claude_dispatch.py` into the person's Claude
  Code configuration folder (`~/.claude/ai-accelerator/`, or under
  `$CLAUDE_CONFIG_DIR`), lists the project root there, and adds to their
  `settings.json` one entry per project hook (same matcher and timeout) and
  four session-start entries `policy-1` to `policy-4`, keeping everything
  else. It writes only for the Claude Code whose transcript folder holds the
  session, nothing when all is in place, never on Windows, and never once a
  file named `off` is in that folder. The dispatcher runs a listed root's
  hook for a session started below it in the same checkout, with
  `CLAUDE_PROJECT_DIR` set to the root, and exits at once anywhere else.
  For `bash-validator.sh` it adds the allow the project's rules give the
  memory CLI in the root. The `policy-N` entries bring `AGENTS.md` in pieces
  cut between sections, each under the 10,000 characters Claude Code 2.1
  keeps in the conversation (a longer hook output goes to a file of which
  the model sees 2,000), and nothing when Claude Code imports the file
  itself - the dialog's Yes, filed under the repository's root. The
  shared-core manifest now names `memory-bank/scripts/*.sh`, and the
  generator's asset carries the shell dispatcher; its skill-body ceiling
  in `scripts/token_budget.json` rises to 306,200 bytes for the two
  instructions that seed it (Infrastructure-Creator `CHANGELOG.md`).
- Tests: `tests/test_memory_claude_subfolders.py`, 27 cases: pieces of
  every edition's policy and the parts' limit; the import approval under the
  repository's key and under a linked worktree's main checkout; the memory
  CLI recognised in five forms and 16 commands that are not; the setup once,
  beside the person's own settings, through a linked `settings.json`, for six
  projects starting together, dropping a removed project, and not for a
  settings file it cannot read, a linked folder, another program's hook or
  the `off` file; the dispatcher in the root, below an unlisted project, in a
  nested repository, in a project installed inside a larger checkout,
  outside Git and under an ASCII locale; the edition's real
  `bash-validator.sh` (a save naming DROP TABLE allowed, `rm -rf /` blocked)
  and the real `local-context.sh` setting it up. Mutations of the dispatcher
  and of the list's lock fail them. In a folder of no listed project the
  dispatcher costs 1.1 ms a hook, against 0.9 ms for `bash` doing nothing.
  Offline, Claude Code 2.1.278 started in `src/` got the capsule, every line
  of `AGENTS.md` and saved, in the four editions and in default,
  `acceptEdits`, `dontAsk`, `bypassPermissions` and `auto` modes, after
  `--resume` too; in its terminal, with No and with Yes to the import
  dialog, the policy came once. `scripts/claude_code_bench.py --subfolder
  src` runs it.

### Codex started below the project saves through its next hook — 2026-10-25

- Why: Codex started in a subdirectory of the project runs a workspace-write
  sandbox that writes only below that subdirectory. The save after every
  answer failed with "Read-only file system" at the runtime lock, and since
  5695f5e8 it was not asked for at all (`outside-sandbox`): such a session
  saved nothing, and the person was not to be asked to restart anywhere.
- What changes: the Stop asks for the save whenever the sandbox writes a
  temporary directory, as Codex's does (`/tmp`, `$TMPDIR`). The save
  command, finding the project read-only (EROFS, EACCES or EPERM on the
  Brain), checks the request and the result as a save does - a malformed
  result is refused while the agent can still correct it - and leaves it
  in a folder of this user's there (`ai-accelerator-native-spool-<uid>`,
  owner-only, journal `save` `spooled`), answering `{"state": "spooled"}`.
  The session's next hook - the continuation's Stop, or else the next
  prompt - runs outside the sandbox, takes the file (one attempt each) and
  saves it (`saved` with code `spooled`). Only a sandbox that writes
  neither the Brain nor a temporary directory is skipped as
  `outside-sandbox`. Codex's `tmpdir` entry is `$TMPDIR`, or on Windows,
  where that is rarely set, the user's temporary folder (`TEMP`, `TMP`):
  the Windows CI found the save not asked for there.
- Tests: `test_a_save_the_sandbox_cannot_write_is_made_by_the_next_hook`
  (the project made read-only; a malformed result refused at once, the
  result spooled, then saved by the continuation's Stop and, in a second
  case, by the next prompt), failing without the change, and a
  temporary-directory case in the outside-sandbox test. On the real host,
  Codex 0.163 started from `src`: `requested`, `saved` `spooled`,
  `recovery-saved`.

### The host benches type a question while the save is written — 2026-10-25

- `scripts/claude_code_bench.py --queued SECONDS` and `scripts/codex_bench.py
  --queued SECONDS` run the host in its terminal through a pseudo-terminal:
  the stand-in takes SECONDS to write the save, and once it has begun a
  second question is typed. The run passes when the save in flight still
  lands and nothing says it was not saved. On the runtime before the
  in-flight fix (afb80236) Claude Code's run fails with the save refused as
  `request-unavailable`; on this branch both hosts pass. The Codex terminal
  runs commands through the app-server daemon under `~/.codex/packages`,
  which the isolated `CODEX_HOME` links to; `--queued` takes no `--attached`
  there. A question the stand-in never received - the terminal of a large
  project was not ready yet - is typed again, up to three times, and the
  screen is kept in the work directory. On copies of next, scb,
  bauherrenmappe and aea-partner (`--target ... --sync`) both hosts pass.
  Codex's terminal leaves its app-server daemon (and, in code mode, its
  host) running after the run, writing to the run's `CODEX_HOME`: the
  Codex bench now stops every process whose `CODEX_HOME` is the run's and
  reports how many (`helpers_stopped`); fifty-odd were left from earlier
  runs on this machine.
  Tests without the hosts: the stand-in waits over the first save only,
  and what counts as passing.

### The memory CLI may name SQL past bash-validator — 2026-10-25

- Why: every shipped `bash-validator.sh` refused the accelerator's own
  memory CLI whenever what it was handed named destructive SQL. A save
  whose progress or learning said "the rollback runs DROP TABLE invoices"
  was blocked as `destructive SQL - DROP TABLE/DATABASE/SCHEMA` - that
  answer's native save was lost, in Claude Code's `echo -E '<JSON>' |`
  form and in the `| (cd ROOT && ...)` form Codex and Cursor are given -
  and so were `context.py retrieve --query 'rollback DROP TABLE'` and
  `record-result` with such text. The SQL rules leave searches and git
  alone; the text piped into the CLI was scanned as if it fed a SQL client.
- What changes: a command that runs nothing but the memory CLI
  (`python3`/`python`/`py -3` `memory-bank/scripts/context.py`, any
  directory before either) and `cd`, `cat`, `echo` or `printf`, in any
  pipe, group or heredoc, is left alone by the SQL rules: nothing in it can
  run the text. Any other program in the command - a client the CLI's
  output is piped into, one run beside it or in a substitution, Python that
  is not the CLI - and the rules apply as before. Generic section,
  byte-identical in the four editions and Infrastructure-Creator; mirrors
  regenerated; the editions' `.claude/hooks/README.md` lists the exception.
- Tests: eight corpus cases in `tests/fixtures/bash-validator-corpus.json`
  (four allowed, four still refused) and the `memory-cli` decision.

### A prompt sent while the save is written no longer cancels it — 2026-10-25

- Why: offline in the terminals of Claude Code 2.1.278 and Codex 0.163,
  with a scripted model that took 8 seconds to write the save, a second
  question sent while it did went into the running turn, and the host ran
  the prompt hook at once (Claude Code's transcript: `queue-operation`
  `enqueue`, then `absorbed_mid_turn`; Codex's rollout: the message under
  the same `turn_id`). The hook re-armed the session: the save in flight
  was refused as `native-result:request-unavailable`, and the agent read
  "Previous native memory result was not saved completely." in the middle
  of saving it. With a real model the save takes seconds after every
  visible answer, the time in which a person types the next question. The
  Claude desktop app mostly runs the prompt hook only when a queued prompt
  starts, after the turn: in 509 local desktop sessions 26 of 2,456 human
  prompts (1.1 %) were sent while the agent still worked.
- What changes: a prompt that finds the last request asked and unsettled,
  less than ten minutes old, moves it aside to a second file of the same
  session and journals `armed` `previous-in-flight` without a warning. The
  save still lands there (`native-result` reads either file by its token),
  the continuation's Stop settles it, and the prompt after reports it as
  any other request: quiet when saved, the warning when not, "not asked
  again for a day" when the person refused the save command. Codex's
  continuation prompt is recognised by either request's token. An older
  request is reported at once, as before. The queued prompt's own answer is
  saved with the next answer's save. `memory_eval.py realized` does not
  count `previous-in-flight` or `previous-unreadable` as armed after an
  unsaved result. A save that lands after its request was moved aside is
  journaled `saved` (or `omitted`) with code `in-flight`, so `realized`
  counts how often the case comes up and how often the save still lands.
- Tests: `test_a_prompt_sent_while_the_save_is_written_does_not_cancel_it`,
  `test_a_request_moved_aside_is_reported_at_the_next_prompt_when_unsaved`,
  `test_a_request_asked_long_ago_is_reported_at_once_and_codex_continues_one_moved_aside`,
  `test_an_armed_prompt_that_printed_no_warning_is_not_after_an_unsaved_one`.

### A save request this runtime cannot read is replaced at the next prompt — 2026-10-25

- Why: a session's request file that this runtime refuses as
  `native-result:invalid-state` - one another version of the runtime wrote
  (an older one does not know the `refused` and `declined` phases), or one
  cut short - refused every later prompt and Stop of that session. The
  session saved nothing more, and only the journal said so.
- What changes: the prompt replaces such a request with a new one and
  journals `armed` `previous-unreadable`, without the "not saved
  completely" warning, since whether it was saved is unknown. Storage that
  is not a plain file stays `unsafe-storage` and is never replaced.
- Tests: `test_a_state_this_runtime_cannot_read_is_replaced_at_the_next_prompt`
  (another version's phase, a file cut short, a directory in its place),
  failing without the change.

### Claude Code started below the project root runs none of its hooks — 2026-10-25

- Documentation only (`docs/TROUBLESHOOTING.md`, `docs/MEMORY-INTEGRATION.md`
  known limits). Offline on Claude Code 2.1.278, started in `src/` of an
  installed project (trusted, and with `src/` trusted too): no hook ran - no
  capsule, no save, no `bash-validator.sh` - while skills, commands and
  agents were found above. It reads `.claude/settings.json` only from the
  folder it starts in, and `.claude/CLAUDE.md`'s `@../AGENTS.md` became an
  external import: a dialog in a terminal (default "No"), left out under
  `-p`. Codex 0.163 from a subdirectory runs the project's hooks. Claude
  Code's own Bash sandbox (bubblewrap) started in the root let the save
  write. The editions' `README.md`, installed as `ACCELERATOR.md`, opens
  Quick Start with where to start the tools.

### A Codex session that cannot write the Brain is not asked to save — 2026-10-24

- Why: offline on Codex 0.163, a session started in a subdirectory of the
  project (`codex exec -C project/src`) runs in a workspace-write sandbox
  that writes only that subdirectory. The save after every answer failed
  with "Read-only file system" on `project/project-brain`, journaled as
  `failed` `unsaved`: two model calls for nothing each time.
- What changes: the Stop reads the turn's `file_system_sandbox_policy` from
  the rollout's `turn_context`, as it reads Plan mode and the sandbox type.
  When the most specific entry over the Brain does not grant write, the save
  is not asked for and the journal says `skipped` `outside-sandbox`. An
  unrestricted policy, or none, asks as before; the attached launch's state
  directory is an added writable root and saves. A policy that, read this
  way, would not let the turn write its own working directory is in a form
  the runtime does not know (a later Codex, another platform's paths), and
  the save is asked for as before rather than dropped.
- Tests: `test_a_codex_workspace_that_cannot_write_the_brain_is_not_asked_to_save`
  (a subdirectory, a Brain carved out read-only, the project root, an added
  root, a policy that denies the turn its own directory, no working
  directory, the project and a subdirectory reached through a link),
  failing without the change. On the real host, started from `src`:
  `skipped` `outside-sandbox`; the bench's default and attached runs save,
  and a read-only sandbox is still `read-only-sandbox`.

### Cursor keeps the full save step on every answer — 2026-10-24

- Why: a later save step points back to the full one while the transcript
  holds it after its last compaction record. Cursor's transcript, when it
  keeps one, and how it records a summarized conversation could not be
  checked here, so nothing tells that a short step is safe there.
- What changes: Cursor gets the full step every time, as before the short
  step. Claude Code and Codex are unchanged. The known limits in
  `docs/MEMORY-INTEGRATION.md` add that a save step after `/rewind` past
  the conversation's first full step still points back to it.
- Tests: `test_cursor_gets_the_full_step_every_time`, failing without the
  change.

### After a Codex compaction the save step is full again — 2026-10-24

- Why: offline on Codex 0.163, with an automatic compaction before the third
  turn, that turn's save step came short, pointing back to rules the model
  no longer held. Codex's `compacted` record carries a `retained_context`
  with the earlier steps in it, and the check found the full step's text
  there, after the record's marker. The model's history after the
  compaction (`replacement_history`) held no full step.
- What changes: the full step counts only after the compaction record's own
  line.
- Tests: `test_the_full_step_comes_back_after_a_compaction_or_without_a_transcript`
  carries a compaction record with the full step inside it, and fails without
  the change. Offline, three turns around the compaction: Codex 2,522, then
  1,619, then 2,522 characters again; Claude Code around a `/compact` 3,149,
  2,246, 3,149. Every turn saved.

### A capsule request cuts each section into sentences once — 2026-10-24

- Why: after the masking cache, the next cost in `refresh` on a synced copy
  of `next` was cutting sections into the sentences an excerpt window moves
  by: 369 `_excerpt_units` calls per request, the same sections cut again on
  each of the three excerpt passes.
- What changes: the units of a section are held per its lines in a bounded
  cache (2,048 sections); each caller gets its own list. They depend on the
  lines alone.
- Measured on the copy, 24 interleaved first prompts of fresh sessions per
  side, with the masking cache in both: the prompt hook's median went from
  280.5 to 238 ms (−42.5 ms, −15 %), every output byte for byte the same.
  With the masking cache, a first prompt went from about 311 to 238 ms.

### A capsule request masks each document image once — 2026-10-24

- Why: profiling the prompt hook on a synced copy of `next` showed masking
  secrets in about a fifth of `refresh`. One request cuts the capsule's
  excerpts three times over (for the manifest, the text and the output), and
  each pass screened every document image again: 127 `mask_secrets` calls
  for about 30 distinct texts.
- What changes: `mask_secrets` is held per text and secret policy in a
  bounded cache (128 texts). The key carries `secret_policy_fingerprint()`,
  so a pattern added since masks afresh, and a policy holding a scanner that
  is no compiled pattern is not cached at all. A ValidationError is never
  held.
- Measured on the copy, 24 interleaved first prompts of fresh sessions per
  side: the prompt hook's median went from 311.5 to 286.5 ms (−25 ms, −8 %),
  every output byte for byte the same. Prompts whose items repeat do no
  excerpt work and are unchanged.
- Tests: `MaskSecretsTest.test_masking_is_held_per_text_and_secret_policy`;
  the secret-policy delivery suites, which change the patterns between
  calls, failed against a cache keyed on the text alone.

### The policy asks for an explicit save only where no Stop request comes — 2026-10-24

- Why: each edition's `AGENTS.md` said "MUST finish authorized work with
  progress/next steps and source-backed reusable findings/decisions (Harness
  `memory-draft`, `memory_record_result` MCP, or `context.py
  record-result`)", and the Memory MCP server asked for
  `memory_record_result` "before finishing authorized work". The Stop hook
  asks for the `native-result` save after every answer, so an agent that
  follows the policy saves each answer twice. On a synced copy of a client
  project, gpt-6.1-sol on Codex 0.163 ran `context.py record-result` itself
  in all 12 answers of a 12-answer plan (13 calls; the run's own count of 10
  missed `Result already recorded`), and a closing `context.py update` in 7.
  The native save then saved each answer again, with `learnings: []` every
  time.
- What changes: the save rule in `AGENTS.md` opens with what not to do:
  MUST NOT save an answer's result yourself - no `record-result`,
  `memory_record_result` or closing `update` - where Stop requests it. The
  Stop bullet names where that is: governed Claude Code, Codex and Cursor
  sessions running the hooks, outside the Harness, unless
  `native_result_request` is `off`. The stage-boundary `update` rule leaves
  out an answer's end that Stop's request saves. Where no request comes - a
  Harness session's `memory-draft` block, lightweight mode,
  `native_result_request: off`, a host without the hooks - the explicit save
  is still required before finishing. Plan mode and a read-only sandbox get
  no request and stay read-only scopes. "Report save failures; claim saves
  only after success" stays. The Memory MCP server's instructions, the
  `memory_record_result` description, `memory-bank/MCP.md` and the
  generator's `policy-forge` say the same.
- Measured live on copies of that project, same model and prompts, one
  copy per session: under the old policy `record-result` ran in 12 of 12
  answers and `update` in 7, and a same-day control answer saved explicitly
  too. A first rewording that kept "MUST save" in front and made the
  prohibition conditional did not change a session's first answer (1 of 1
  still saved). The rule as shipped: `record-result` in 2 of 12 answers (one
  a first answer, one beside the rule a person asked to keep), `update` in
  0; all 12 native saves landed on the first try, and 5 of them carried
  learnings, against 0 of 12 before.
  Asked to keep a rule, the agent re-verified a chunk, created a finding or
  edited `AGENTS.md`, as before. `native_result_request: off` is not visible
  to an agent unless it reads `runtime.json`.
- Budget: `AGENTS.md` grows 379 bytes in each edition, about 80 tokens per
  session; the four `agents_md_bytes` ceilings rise by that much, and the
  generator's `body_bytes` by 745 for `policy-forge`. The policy locks were
  rewritten.
- Tests: `NativeSaveRequestPolicyTest` in `tests/test_framework_semantics.py`
  holds the rule in every edition whose Stop hook asks for the save, and in
  the MCP server; `tests/fixtures/memory_native_roundtrip_public.json`
  carries the server's new instructions.

### A save step points back to the rules the conversation holds — 2026-10-24

- Why: every answer carried the whole save step, about 2,600 characters, of
  which the rules (the JSON fields, the schema example, what belongs in
  learnings) were about 1,150. The conversation already held them from the
  first step. Claude Code's terminal showed the whole step after every
  answer.
- What changes: the Stop reads the transcript's last 2 MiB. When a full step
  is there after the last compaction record, the step carries its commands,
  its ending, its refusal rules and a pointer back to the full step, 903
  characters shorter. After a compaction, or without a readable transcript,
  the full step comes back. `"native_result_instruction": "full"` (or
  `CONTEXT_NATIVE_RESULT_INSTRUCTION=full`) sends it every time. The first
  step of a conversation is byte for byte what it was.
- Tests: `NativeInstructionTests` in `tests/test_memory_native_capture.py`,
  failing without the change. Offline on Claude Code 2.1.278, three turns of
  one session: 3,117 characters, then 2,214 twice, all three saved. On Codex
  0.163, `exec` then `exec resume`: 2,516 then 1,613, both saved.

### The Claude Code bench runs attached, as the Harness launches it — 2026-10-24

- Why: the Harness lends an edition from the clone without copying it
  (`accelerator_attach.py`): `--add-dir`, one `--settings` with the edition's
  permissions and hooks, and the accelerator's state outside the project.
  The bench only ran installed editions, so that launch, and whether its save
  passes Claude Code's permission check, had never run offline.
- What changes: `claude_code_bench.py --attached` runs a fresh project with
  nothing installed and that overlay, leaving the project untrusted, and
  reads the journal from the state directory. Offline on Claude Code 2.1.278
  it saves, under `default` and `dontAsk` alike: permissions from
  `--settings` count without the trust dialog, and the `cd` into the
  added directory passes.
- Tests: `AttachedTest` in `tests/test_host_benches.py`.
- `codex_bench.py --attached` does the same for Codex: the edition's hooks and
  instructions as `-c` options, the state as an added directory, and the
  hooks trusted as `accelerator_attach.py trust-codex-hooks` records them.
  Offline on Codex 0.163 it saves. `CodexAttachedTest` holds it.

### The rollout rehearsal says when your Claude Code does not trust a target — 2026-10-24

- Why: the rehearsal is the step before a rollout, and its copies run with a
  HOME of their own. On `next` it passed while your own Claude Code had never
  accepted the project's trust dialog, so `claude -p` there would refuse the
  save after every answer.
- What changes: with Claude Code among the hosts,
  `memory_rollout_rehearsal.py` reports `claude_trust` per target, read as
  the Claude Code bench reads it, and prints a line when the dialog was never
  accepted. The exit status is unchanged.
- Tests: `tests/test_memory_rollout_rehearsal.py` runs it against a Claude
  Code configuration that never trusted the project.

### The Claude Code bench says when your Claude Code does not trust the target — 2026-10-24

- Why: the bench runs in a HOME of its own that trusts the copy, so it passed
  on `next`, whose trust dialog your own Claude Code never accepted. There
  `claude -p` refuses the save after every answer.
- What changes: with `--target`, `claude_code_bench.py` reads your own Claude
  Code configuration (`~/.claude.json`, or `$CLAUDE_CONFIG_DIR`) for that
  project's `hasTrustDialogAccepted` and nothing else. When the dialog was
  never accepted, it says so and how to fix it. The run's outcome is
  unchanged.
- Tests: `PersonTrustTest` in `tests/test_host_benches.py`.

### A session whose host refused the save is not asked again for a day — 2026-10-24

- Why: offline on Claude Code 2.1.278, `claude -p` in a project whose trust
  dialog was never accepted ran the hooks but ignored the project's allow
  rules. The save command was refused after every answer ("requires
  approval"): two wasted model calls and a "not saved completely" warning on
  each prompt. The same happens when a person keeps saying no or a rule or
  auto mode denies the command. Of the four client projects, `next` has never
  had its trust accepted.
- What changes: the recovery Stop reads the transcript's last 512 KiB for
  the save command's result. When Claude Code recorded a refusal there
  (`toolDenialKind` `user-rejected`, `permission-rule` or `automode-blocked`;
  not `interrupted`, `cancelled` or auto mode's classifier failing), the
  journal says `failed` `host-refused`.
  The next prompt prints the usual warning once, and the session is not
  asked again for a day (`prompt` `skipped` `host-refused`). A new session is
  asked again. A person who answers No in Claude Code's terminal dialog ends
  the turn before any Stop runs (seen offline: the transcript records
  `user-rejected`, no recovery Stop follows), so the next prompt reads the
  refusal from the transcript and does the same. It also catches a refusal
  the recovery Stop missed: Claude Code writes its transcript in batches, and
  a denial made 10 ms before that Stop was not on disk yet.
- Tests: three in `tests/test_memory_native_capture.py`. The two refusal
  tests fail without the change; the third holds that an interrupted or
  plain unsaved request stays `unsaved`. On the real host, a turn after the
  refusal has no save step, after `claude -p` in an untrusted project and
  after No in the terminal dialog alike.

### The host benches run the WordPress edition — 2026-10-24

- Why: `claude_code_bench.py` and `codex_bench.py` handed the installer the
  edition's directory, `Cms/wordpress`, where it takes the name `WordPress`.
  `--edition WordPress` failed before Claude Code or Codex started.
- What changes: both benches hand the installer the edition's name.
- Tests: `EditionTest` in `tests/test_host_benches.py` holds the name `main()`
  hands on, failing on the old code, and installs WordPress through
  `prepare()`. Offline, both benches pass on WordPress.

### The Claude Code bench runs a permission mode, as your own uid — 2026-10-24

- Why: inside `unshare -rn` the bench ran Claude Code as root, and Claude
  Code refuses `bypassPermissions` to root ("cannot be used with root/sudo
  privileges"). The bench also ran only the default mode, while the
  Harness fleet worker runs `dontAsk`.
- What changes: `scripts/claude_code_bench.py --mode` takes `default`,
  `acceptEdits`, `auto`, `dontAsk`, `bypassPermissions` or `plan`. The
  namespace maps your own uid where `unshare` can (util-linux 2.38 and
  later) and falls back to `unshare -rn`. In `plan` the run passes when no
  save was asked for and the journal records `plan-mode`.
- Tests: `NamespaceTest` in `tests/test_host_benches.py`. Offline on Claude
  Code 2.1.278 every mode passes.

### A save another session overtook says to report it — 2026-10-24

- Why: two Claude Code sessions on one branch, driven offline at once with
  different results: the first save moved the task's revision, and the
  second was refused with the bare `memory-result-v1:prepared_stale_no_write`.
  In a new project the second was refused with `native-result:task-changed`,
  because the first save had created the branch task. Nothing was written
  either way, as designed, but the agent read only a code.
- What changes: on the `native-result` path both refusals say what happened
  and what to do, for example "another session or a person saved the task
  after this save was requested; nothing was written: report the held save,
  do not retry it". The journal keeps the codes. `record-result`,
  which the Harness reads by its exact text, is unchanged.
- Tests: two in `tests/test_memory_native_capture.py`, one per refusal; both
  fail without the change.

### A raw newline in a save names the echo that made it — 2026-10-24

- Why: driven offline on Claude Code 2.1.278 under zsh, a save piped with a
  plain `echo` instead of the command's `echo -E` was refused with "inside a
  string, write a newline as \n". The agent had written `\n`: zsh's `echo`
  turned it into a line break. The hint pointed at the JSON, not at the pipe.
- What changes: the hint adds "(a plain echo in zsh turns \n into a line
  break: pipe with echo -E)". The journal code is unchanged.
- Tests: `test_a_refused_input_says_what_to_correct` checks the whole hint.
  Offline, a stand-in model that pipes with a plain `echo` first and retries
  as told after the refusal saves on the second call.

### A compaction after a new conversation's first turn hands its memory again — 2026-10-24

- Why: Claude Code writes a new conversation's transcript only after its
  first prompt hook has run (seen on 2.1.278: the file did not exist at
  `SessionStart` or at the first `UserPromptSubmit`). That turn kept no
  transcript position, so a `/compact` or an automatic compaction before the
  second turn went unseen. The second turn's capsule left out what the first
  had handed and said `memory: 1 item(s) handed earlier in this conversation
  still apply`, while the conversation held only the summary.
- What changes: a transcript not written yet is a position, its start. The
  next turn reads the file from there and sees the compaction record. Codex
  0.163 already has its rollout at `SessionStart`, so nothing changes for it.
- Tests: `test_a_transcript_written_after_the_first_turn_still_shows_a_compaction`
  in `project-brain/tests/test_runtime.py`, with and without a compaction. It
  fails without the change. Offline on Claude Code 2.1.278: after a first turn
  that handed a learning, `/compact` and a second turn, the capsule carries
  the learning again.

### The session banner counts the accelerator's own files apart — 2026-10-24

- Why: the session-start banner printed `Branch: … (N uncommitted changes)`
  from `git status --short`. In a project that neither commits nor ignores
  the accelerator, that count included `.claude/`, `memory-bank/`,
  `AGENTS.md` and the rest on every session:
  - on `next`, 8 of 8 entries were the accelerator's;
  - on `scb`, 14 of 19;
  - on `bauherrenmappe`, 1 of 1.

  The model was told about uncommitted work nobody had done.
- `context.py status --json` reports `uncommitted` (the team's entries) and
  `accelerator_uncommitted` (the accelerator's install as its install record
  or the generator's manifest says it was written, and the memory the
  runtime writes). Both come from one `git status`, with the filter
  `changed_paths` already used, now shared as `accelerator_owned`. Outside
  Git both fields are left out.
- The four editions' `local-context.sh` print the branch line after the
  status call: `Branch: … (0 uncommitted changes; 16 more are the
  accelerator's own files)`. Without Python, or without the fields, it keeps
  the plain Git count. The Codex and Cursor mirrors follow.
- Test (`test_memory_lifecycle.py`): on a fresh install the banner reports 0
  uncommitted changes and the accelerator's entries apart, and 1 after an
  edit. It fails on the old hook.

### A refused save says what to correct — 2026-10-23

- Why: the agent gets one retry after a refused save, and two common
  refusals did not say what to fix.
  - An input that was not one JSON object came back as the bare code
    `native-result:invalid-input`. This is what a shell's `echo` causes when it
    turns a JSON string's `\n` into a real newline, and what any malformed or
    doubled object causes.
  - A learning with `verified` inside it got "Each learning needs type, title,
    consequence and sources", which does not name the extra key.
- The input refusal keeps its journal code, through the error's `code`, and
  now says why:
  - the JSON error with its line and column;
  - for a control character, how to write a newline or a tab inside a string;
  - "more than one JSON object", or the kind of value sent instead of an
    object.
- The learning refusal keeps its sentence and adds the missing and the
  unexpected fields (at most five, each cut to 40 characters). It calls
  `verified` a top-level boolean. The same validation serves `record-result`
  and the MCP tool.
- "Nothing to record" says what a result needs (a progress summary, next
  steps or a checked learning), and the native save's bare
  `native-result:nothing-to-record` adds that `--skip` is the way out.
  Conflicting learnings name the title they share.
- A missing or linked source is named in its refusal, without its `#`
  anchor and cut to 200 characters, so an agent citing several files knows
  which to fix.
- Test (`NativeFunnelJournalTests`): four malformed inputs each journal
  `native-result:invalid-input` and print their reason; `verified` inside a
  learning and a missing `sources` are named. It fails on the old runtime.

### The host benches are tested without the hosts — 2026-10-23

- `scripts/claude_code_bench.py` and `scripts/codex_bench.py` run only where
  Claude Code and Codex are installed, and CI has neither. A change to the
  save step's wording could break their command search unnoticed.
- `tests/test_host_benches.py`, in the `installation` job, covers:
  - each bench finds the save command in the step the runtime writes for
    Claude Code, Codex and Cursor;
  - the Claude stand-in reads, answers, saves through `echo -E` and ends;
  - the Codex stand-in reads Codex's XML-escaped `<hook_prompt>` unescaped,
    and runs commands through code mode's `exec` script when there is no
    tool list;
  - a target the rollout rehearsal refuses is refused.

### Codex's Plan mode and read-only sandbox are not asked to save — 2026-10-23

- Why: Codex's hook `permission_mode` follows its approval policy only. The
  offline Codex run (`scripts/codex_bench.py`) reported
  `bypassPermissions` under both the workspace-write and the read-only
  sandbox. In the read-only sandbox the save after every answer failed
  (`stop failed unsaved`), and with approvals on request the agent could ask
  the person to let it write. Codex's Plan collaboration mode, like Claude
  Code's plan mode, is for no changes. Both show in the rollout's
  `turn_context` record, as `collaboration_mode.mode` and
  `sandbox_policy.type`.
- For Codex the Stop reads the last 512 KiB of the rollout and takes the
  `turn_context` whose `turn_id` is the Stop's. Plan mode is then skipped as
  `plan-mode`, and a read-only sandbox as `read-only-sandbox`. Another
  turn's context, one beyond the tail, or no rollout is unknown, and such a
  turn is asked as before.
- Tests (`NativeRequestSwitchTests`): plan mode and a read-only sandbox ask
  nothing and journal their code, and a default turn is asked; another
  turn's context, one beyond the tail and a missing rollout are asked.

### A plan-mode turn is not asked to save — 2026-10-23

- Why: in plan mode Claude Code tells the agent it "MUST NOT make any edits,
  run any non-readonly tools … This supercedes any other instructions you
  have received". The Stop hook still asked for a save after every answer,
  so the agent either refused it, at the cost of a model call and a "not
  saved completely" warning on the next prompt, or wrote memory against the
  person's choice. The offline Claude Code run showed the save command
  itself passes the permission check in plan mode, so only the agent's
  compliance stood between plan mode and a write.
- The Stop now skips the request when the hook input's `permission_mode` is
  `plan`. Claude Code and Codex both send that field; Codex's hook schema
  has the same five values. The journal records `stop` `skipped`
  `plan-mode`. The prompt still arms, so a turn switched out of plan mode
  before it ends is asked, and a save asked before the switch is settled as
  before.
- Test (`NativeRequestSwitchTests`): for both hosts, a plan-mode Stop asks
  nothing and journals `plan-mode`, and the next default-mode answer is asked.

### Codex on the real host, offline — 2026-10-23

- Why: the Codex side of the save after every answer had been checked only
  through simulated hooks. `scripts/codex_bench.py` drives the installed
  Codex (0.163) the way `claude_code_bench.py` drives Claude Code: a network
  namespace with loopback only, a scripted stand-in for the Responses API,
  and an isolated `CODEX_HOME` that approves the project's hooks by hash.
- What it showed:
  - Codex adds the Stop hook's continuation to the same turn as a
    `<hook_prompt>` item, XML-escaped, and runs no prompt hook for it.
    `docs/MEMORY-INTEGRATION.md` said Codex sends it back through the prompt
    hook, and is corrected; the prompt-hook guard and the silent `refresh` for
    such a query stay as defence.
  - The command's `&&` reaches the model as `&amp;&amp;`. In every real Codex
    save in the transcripts, the model ran it as `&&`.
  - A project that pins a gpt-5.6 model gets no tool list. The model then
    runs commands through code mode's `exec` script.
- Measured: a fresh PHP Core install and synced copies of the four client
  projects (one pins gpt-5.6) save and recover in Codex's workspace-write
  sandbox.

### Claude Code saves without a prompt and without a hook error — 2026-10-23

- Why: no Claude Code session had yet run the save after an answer, so the
  real binary (2.1.278) was driven offline: a network namespace with
  loopback only and a scripted stand-in for the Messages API, no model.
  It found three defects:
  - Claude Code asked before the save command after every answer, whatever
    the allow rules say: a command holding a subshell or a `{ }` group always
    asks, and the command was `(cd ROOT && python3 … --json)`. Under
    `claude -p` the save was refused and the result lost.
  - The Stop `decision: "block"` showed in the transcript as "Stop hook
    error" and raised "Stop hook error occurred · ctrl+o to see" after every
    answer.
  - The model got the step twice: as "Stop hook feedback" and again as a
    blocking-error message, about 2,600 characters a turn.
- Claude Code's Stop now answers with `hookSpecificOutput.additionalContext`
  (documented for Stop since 2.1.163). It continues the turn through the
  same `stop_hook_active` and continuation cap, is labelled hook feedback,
  and reaches the model once.
- Its command is `cd ROOT && echo -E '<JSON>' | python3
  memory-bank/scripts/context.py … --input - --json`, the JSON written in
  place of `<JSON>`, and the skip has its own line. Offline the `cd`, `echo`
  and the allowed CLI pass without a prompt. Rejected:
  - a heredoc: Claude Code asks before one whose body holds braces and
    quotes;
  - `printf '%s'`: Claude Code asks before a `printf` operand holding `[`
    with `$` or a backtick, as a JSON with code in it does;
  - plain `echo`: Claude Code's Bash tool runs zsh unless `$SHELL` is bash,
    and zsh's `echo` turned the JSON's `\n` into a newline, so the save was
    refused. `-E` keeps it, in zsh as in bash.
- Codex and Cursor get the same text, byte for byte, as before.
- `memory_rollout_rehearsal.py` and the lifecycle test run either form.
  `memory_eval.py` already finds the request in the new transcript shape;
  a test now holds it.
- Measured offline, one answered turn: the save ran and the second Stop saw
  the recovery; the continuation fell from 5,250 to 3,141 characters in one
  message; the stream raised no error notification.
- Tests (`ClaudeSaveStepTests`): the Stop output per host; the command holds
  no subshell, group, heredoc, `$`, backtick or `;` outside quotes; it keeps
  a JSON with backticks, `$`, newlines, tabs, backslashes, quotes and
  Cyrillic verbatim under bash and zsh from a subdirectory; the skip runs as
  given. All four fail on the old runtime.
- `scripts/claude_code_bench.py` repeats the offline run against the
  installed Claude Code: one answered turn of an installed edition, the save
  step followed as written with such a JSON, and a report of whether the
  save ran, how many messages carried the step and whether the transcript
  recorded a hook error. With `--target DIR --sync` it runs in a synced copy
  of an installed project. On copies of the four client projects synced to
  this change, all four saved without a prompt or a hook error. On `next` as
  installed today, the save was refused. Linux only (user and network
  namespaces). It is not a CI gate, as CI has no Claude Code; run it when
  Claude Code updates and before a rollout.

### Cursor's next request keeps the rule its last prompt wrote — 2026-10-23

- Why: Cursor reads its rule before the prompt hook runs, so the rule a
  prompt writes is what the next request carries. Its save continuation
  comes back as a prompt, and the prompt hook rendered a capsule for the
  instruction's words into the rule. On a project where the last real prompt
  had retrieved the learning it asked about, the continuation replaced that
  memory with `no-match`, and the person's next request carried that rule.
- The prompt hook of every host now exits after arming when the prompt is
  the save continuation, waiting for the background arm first: Cursor's rule
  stays the last real prompt's, and Codex skips the refresh process
  `refresh` would have spent saying nothing (the runtime's own check from
  the previous entry stays, for a hook that still calls it). Claude Code's
  continuation never arrives as a prompt. The block sits after the arm in the
  canonical hook, so the Codex and Cursor mirrors carry it unchanged.
- Test (`test_memory_lifecycle.py`): on a full PHP Core install, the rule
  holding the learning the second prompt retrieved is unchanged, byte for
  byte, after the continuation; it fails without the change.

### A task followed across sessions, a commit and a branch, in CI — 2026-10-23

- Why: the rollout rehearsal runs one turn and the next session's recall,
  and the hook tests each cover one piece. A task lives longer: a learning,
  then a progress and a next step, a commit, a new session, another branch
  with its own task, and the way back. Run on copies of the four clients
  through the installed hooks it held, so it is now a regression test.
- `tests/test_memory_lifecycle.py` installs PHP Core into a small shop and
  follows that life through Claude Code's hooks (a session a person reads)
  and Codex's (the save continuation comes back through the prompt hook):
  the second session sees the progress, the next step and the learning; the
  new branch starts its own task; back on the first branch its own progress
  returns and the other's does not; three saves journaled; `validate`
  passes. With the continuation fix reverted the Codex case fails. It runs
  in the step that holds the native roundtrip and capture suites.

### Claude Code shows what the save step is, not its marker — 2026-10-23

- Why: Claude Code shows the first lines of a Stop hook's reason under "Ran
  N stop hooks" after every answer (up to five lines in 2.1.278), and the
  save request opened with `INTERNAL_NATIVE_MEMORY_RESULT <token>`: the
  first thing a person saw after each answer was an opaque marker.
- For Claude Code the marker now closes the request, and its first line is
  the instruction's own "This is an internal continuation: ...". Codex and
  Cursor send the continuation back as a prompt and recognise it by the
  marker that opens it, so theirs is unchanged. `memory_eval.py realized`
  finds the marker anywhere in the record, as before.
- Test (`NativeAnswerTests`): Claude's request opens with the instruction and
  ends with the marker; Codex's and Cursor's open with the marker.

### A Codex save continuation gets no second capsule — 2026-10-23

- Why: Codex sends the Stop hook's save continuation back through
  UserPromptSubmit as a new prompt, so the prompt hook built a capsule for
  the instruction's own words. The turn's prompt had already brought the
  capsule: on a copy of a client the continuation's hook took 169 ms and put
  1.4 KB of repeated working state, recent files and last-turn telemetry
  into the save step, and where the project has documents, the instruction's
  words ("progress", "learnings", "sources") would search them too.
- `refresh --host codex` (and `--host claude`, whose continuation never
  arrives as a prompt) returns at once, silently, for a query opened by the
  continuation's marker and challenge token (`NATIVE_CONTINUATION_PROMPT`);
  the hook then stays silent, as for any refresh with nothing to say. The
  arm still recognises the continuation and journals it. Cursor's rule is
  left to its own refresh.
- Test: the Codex prompt hook prints nothing for a real continuation and
  the journal records it, while the same runtime still answers Cursor and a
  real prompt; it fails without the change.

### A promoted save's memory files are not the task's work — 2026-10-23

- Why: `changed_paths` drops what the runtime writes itself, so that a task
  does not record its own churn as the user's work, but it knew only the
  Project Brain's runtime directories. A save with a learning promotes it:
  the runtime writes a chunk under `memory-bank/chunks/` and rewrites
  `memory-bank/INDEX.md`. In a project that commits its memory - the
  arrangement the editions' documentation recommends - every such save left
  both changed for the next turn: on a copy of a client with its memory
  committed, one save wrote five chunks (the learning and four findings
  promoted with it) and the next turn counted `memory-bank/chunks/` and
  `memory-bank/INDEX.md` as its work. A turn that only saved then counted
  toward a checkpoint, and the chunks became the task's newest files.
- `TURN_RUNTIME_MEMORY` names `memory-bank/chunks/`, `memory-bank/INDEX.md`
  and `memory-bank/local/`; `changed_paths` skips them as it skips the
  Brain's directories, and a checkpoint flush drops them from turns buffered
  before. The recent-files line read from transcripts already left out every
  `.md` file, so it named none of them.
- None of the four clients is affected today: two ignore their memory and two
  neither commit nor ignore the accelerator, whose collapsed `memory-bank/`
  the install record already leaves out.
- Tests (`AutomaticWorkingMemoryTest`): a promoted save's chunk and index are
  not a turn's work while a real change beside them is; a flush drops them
  from a turn buffered before. Both fail without the change.

### The prompt hook arms the save request beside the capsule's refresh — 2026-10-23

- Why: the prompt hooks ran two Python processes one after the other:
  `native-prompt`, which arms the save request, then `refresh`, which builds
  the capsule. The first is a few milliseconds of work behind about 50 ms of
  interpreter start and imports: 63 ms of a 182 ms hook on a copy of scb.
- Claude Code's and Codex's hooks now run `native-prompt` in the background
  beside `refresh`, its output in a private temporary file; the hook waits
  for it when the refresh returns and prints its line before anything else,
  as before. Without a temporary file the request is still armed and only
  that line is lost. The Stop hook keeps its order: `native-stop` pins the
  task revision a checkpoint flush in `turn` may have just moved.
- The two shared one thing: on an unsaved previous result the arm wrote
  `native_capture_incomplete` into the last-turn report, which the refresh
  reads into the capsule. `native-prompt` now writes it for Cursor only.
  Claude Code and Codex get the notice from the hook's own first line, as
  they did; the capsule line that repeated it is gone instead of appearing
  when the arm happened to finish first. Cursor's hook answers in JSON only,
  so its notice is that capsule line: its mirror rule keeps the arm before
  the refresh (`_WM_NATIVE_ARM_CURSOR` in `context_retrieval.py`).
- The generator's hook contract (`native_capture_hook_contract` in
  `validate_generated.py`) accepts the arm beside the refresh too, when its
  PID is taken at once and awaited; generated hooks keep the sequential
  form hook-forge describes.
- Measured on synced copies of scb (prompt hook, ABBA, 30 rounds): 209 →
  147 ms, paired median −62 ms [−63.6; −59.9]. The worst case of the hook is
  one 5 s budget instead of two against Claude Code's 8 s timeout. The
  rehearsal on the four clients saved and recalled 16 of 16.
- Tests: each host gets the unsaved-result notice by one fixed path (Claude
  Code and Codex: the hook's first line, no report flag, no temporary file
  left; Cursor: the rule written in the same turn), failing without the
  flag change; the generator contract refuses an arm in the background
  without its PID or its `wait`.

### A session a person reads is not asked to repeat its answer after the save — 2026-10-23

- Why: after every answer in governed mode the Stop hook asks the agent to
  save its result, and since 2026-10-21 to return its substantive answer
  again afterwards. A headless run (`codex exec`, `claude -p`) needs that: its
  caller gets only the final message. In a session a person reads as it goes,
  the answer is already on screen above the continuation, and the repeat
  doubled it: on the nine Codex runs that had the instruction, the message
  after the save was 93 % of the answer's length (21 % on the twelve before
  it), and the continuation cost +408 output tokens against +360.
- `native_result_answer` in `runtime.json` (or `CONTEXT_NATIVE_RESULT_ANSWER`):
  `auto`, the default, ends a session its host marks as attended with no
  further text or one short line, and repeats everywhere else; `repeat`
  always repeats; `brief` never does. An unknown value repeats and the
  journal names it; the save is asked for either way.
  - Claude Code: `CLAUDE_CODE_SESSION_ATTENDED=1` (Claude Code 2.1.278 sets it
    for the processes of an attended session) and an entrypoint of `cli`,
    `claude-vscode` or `claude-desktop`. A `claude -p` started from such a
    session inherits the entrypoint, but it is a child session, which Claude
    Code never marks attended.
  - Codex: the rollout's first line names the session's source: `vscode`
    (the app, the IDE extension and the terminal UI) or `cli` end briefly;
    `exec` and subagents repeat.
  - Cursor and anything unmarked repeat, as before.
- The journal's `requested` line carries `answer` (`brief`/`repeat`).
  `memory_eval.py realized` counts the brief requests in the funnel and
  splits the transcripts' continuation costs by how each request asked to
  end (`brief`, `repeat`, `earlier`), read from the request's own words, so
  the change reads as a before and after on real sessions.
- Tests (`NativeAnswerTests`): an attended Claude session in each of the
  three interfaces ends briefly; a child of a desktop session, `claude -p`,
  a Slack session and an unmarked one repeat; a Codex session ends briefly
  only for a `vscode` or `cli` rollout (not `exec`, a subagent, a first line
  past the bound or a missing rollout); the setting and its variable decide,
  and an unknown value keeps the save. The native-capture fixture drops the
  marks a suite run from a Claude Code session would inherit.

### A citation check resolves its source once and the roots once a pass — 2026-10-23

- Why: every prompt checks each Memory Bank chunk's citations again, and
  each Brain record's, because a cited file can go away while the chunk
  stays. Each check resolved the source, then `workspace_roots.contains`
  resolved it a second time, resolved the project root again, and pathlib's
  `relative_to` walked the path's parents. On a copy of a client grown to
  1,200 chunks by 600 native saves, the prompt hook took 385 ms instead of
  208. Profiled at 200 chunks, these checks were three quarters of the
  eligibility pass.
- `workspace_roots.within` answers `contains` for a path its caller has
  already resolved, against `resolved_roots`. The bank validator's source
  facts and the Brain's cited-file check resolve the roots once per pass,
  in the memo they already keep for sources, and `source_fingerprints` once
  per call. The spelling decides first, as in `key_for`; any other
  spelling takes pathlib's answer, so a neighbour such as `<root>-old` is
  still outside. `contains` keeps its contract for every other caller.
- Measured on that copy (prompt hook, ABBA): at 650 chunks 373 → 326 ms,
  paired median −41 ms [−42.9; −37.4]; at 1,200 chunks 396 → 336 ms,
  −55 ms [−56.5; −52.0]. The capsules and the retrieved documents are
  identical under `full` and `recall`; only the gate's diagnostic
  `top_score` moves in its sixth digit, as it does between two runs of one
  copy.
- Tests (`test_memory_brain_scale.py`): a pass resolves the roots once
  however many files its chunks cite; `within` answers as pathlib does,
  neighbour included; a citation that leads out of the project by a link
  still takes its chunk out. The two tests that counted citation checks
  count `within` now.

### The accelerator's block in .gitignore and .gitattributes follows the release — 2026-10-23

- Why: `--sync` kept `.gitignore` and `.gitattributes` whole as the
  project's own files, including the block between the accelerator's
  `# BEGIN/END ACCELERATOR MANAGED ENTRIES` markers. A client installed
  before the client `.gitattributes` override therefore kept the edition's
  own block, which marks every skill and hook `-diff linguist-generated`. An
  edited hook - a safety control - showed as "Binary files differ" in review,
  and no sync, `--update-tracked` included, ever replaced it. The rehearsal
  of the rollout found it on scb.
- The sync now rebuilds that block from the release with the merge an
  install uses (`merge_additive_file`): the project's own lines stay, and a
  directive they already hold is not repeated. A tracked file waits for
  `--update-tracked` as before, and a file without the block stays the
  project's.
- Test (`InstallSyncTest`): an old `-diff` block is replaced by the release's
  `*.sh text eol=lf` while the project's line stays; a tracked file changes
  only with `--update-tracked`; a file without the block is kept. It fails
  without the change.

### A memory line the hooks cannot write is lost quietly — 2026-10-23

- Why: the prompt hook appends a health line and the Stop hook a journal line
  with `>> file 2>/dev/null`. Bash opens redirections left to right, so in a
  project whose `memory-bank/local` was read-only, every turn printed
  "Permission denied" before stderr reached `/dev/null`. The rehearsal on a
  copy of a client showed it; the hooks themselves still exited 0 and served.
- Both lines now redirect stderr first. The Claude Code, Codex and Cursor
  copies of `working-memory-read.sh` and `working-memory-write.sh` change in
  every edition, through `scripts/core_sync.py --write` and the mirrors. The
  generator's hook-forge fragment and the verifier line it is held to
  (`validate_generated.py`) take the same order, so generated Stop hooks
  stay quiet too; the generator's policy lock was rewritten.
- Test (`ReadOnlyStateTest` in `test_hooks.py`): with a read-only
  `memory-bank/local`, both hooks of each host exit 0, print no "Permission
  denied" and write nothing. It fails without the change; it is skipped
  under root.

### The cost of the save after every answer, read from the transcripts — 2026-10-23

- Why: the native journal counts the Stop hook's save requests, but whether
  to keep `native_result_request: every-turn` turns on what each request
  costs. Only the session transcript shows that, and nothing read it.
- `scripts/memory_eval.py realized` also prints, per host, the save requests
  in the project's Claude Code transcripts and Codex rollouts. As medians:
  the seconds and output tokens from each request to the next human prompt,
  and those of the answer before it.
  - A request is found by its `INTERNAL_NATIVE_MEMORY_RESULT <token>` marker
    wherever the host recorded it, never in the agent's own messages.
  - Claude Code's tokens are each response's `usage`, counted once per
    response; Codex's are the cumulative `token_count`.
  - `--json` carries the same under `continuations`.
  - On the 21 Codex runs of 16-19 October that made one request each: +9.9 s
    and +383 output tokens after a 17.5 s answer of 701 tokens.
- Tests (`SaveContinuationCostTest` in `tests/test_memory_eval.py`): a Codex
  request and a Claude Code request cost exactly the time and the tokens to
  the next prompt; a response split into several records counts once; a
  mention of the marker in a later answer is not a request.

### A generated project's manifest marks its accelerator files too — 2026-10-23

- Why: a checkpoint leaves out the accelerator's own untracked files by the
  install record, which only `install_accelerator.py` writes. A project the
  generator built has no such record, so while its team had neither
  committed nor ignored the result, `.claude/`, `AGENTS.md` and the rest -
  and `.infra-manifest.json` itself - still counted as the task's work on
  every turn.
- Where there is no install record, `installed_footprint` reads the
  generator's `.infra-manifest.json` (sha256 of every file it owns). A file
  is the generator's while its sha256 still matches. The manifest itself
  never counts.
- The core change was made in `PHP Core/` and written to the other editions,
  the generator asset (so generated projects get it) and the mirrors by
  `scripts/core_sync.py --write`; the generator's policy lock was rewritten.
- Test (`AutomaticWorkingMemoryTest`): an untracked generated install
  changes nothing, and an edited generated file is reported. It fails
  without the change.

### The save after every answer can be switched off — 2026-10-22

- Why: in governed mode every answer ends with the Stop hook asking the agent
  to save its result, and that continuation is another model call. On 21
  Codex runs with one native save each (16-19 October), it cost a median of
  10 seconds, 383 output and 59k mostly cached input tokens: about half of
  the answer's own time and tokens. No client has worked under it yet, and a
  team that finds it too slow or costly had no way out but lightweight mode,
  which gives up governance altogether.
- `native_result_request` in `project-brain/config/runtime.json` (or
  `CONTEXT_NATIVE_RESULT_REQUEST`): `every-turn`, the default and what ran
  before, or `off`. Off, no prompt is armed and no Stop asks, including a
  Stop whose prompt was armed before the switch. Each such prompt and Stop
  is journaled as `skipped` with the code `switched-off`, and results still
  reach memory through `record-result`, the MCP tool or the Harness. Any
  other value refuses the arming with `native-result:invalid-request-setting`.
  No edition ships the key, and `--sync` never changes `runtime.json`.
  `docs/TROUBLESHOOTING.md` ("The Agent Saves After Every Answer") explains
  what the step is, its cost and the switch.
- The core change was made in `PHP Core/` and written to the other editions,
  the generator asset and the mirrors by `scripts/core_sync.py --write`; the
  generator's policy lock was rewritten for its asset copies.
- Tests (`NativeRequestSwitchTests` in `tests/test_memory_native_capture.py`):
  - switched off, Codex, Claude Code and Cursor arm nothing and ask nothing,
    with two `switched-off` lines each;
  - a prompt armed before the switch asks nothing;
  - the variable overrides the setting;
  - an unknown value is refused and journaled.

  Each fails without the change.

### An install records what it wrote — 2026-10-22

- Why: the record of the accelerator's own files
  (`memory-bank/local/accelerator-install.json`) was written by `--sync`
  only. Until a project's first sync, the memory runtime could not tell the
  untracked install from the task's work. A fresh install's Cursor rule
  opened with "warming: task checkpoint pending" on a project where nothing
  had changed. The first sync also had to recognise untouched files from the
  clone's history, and without that history (a source from an archive) an
  untouched `ACCELERATOR.md` stayed behind as edited.
- `install_accelerator.py` now writes the record after an install. It names
  every file the install leaves holding the release byte for byte (copied,
  already identical, or given its executable bit) and keeps the entries of a
  record the project already has. A merge into the project's own file
  (`AGENTS.md`, `.gitignore`, an MCP configuration) is not named. A dry run
  writes nothing, and a record that cannot be read or written safely is left
  to the next sync. The Harness applies only the reviewed files of its
  staged install and leaves the record to its first sync.
- Tests (`tests/test_installation.py`):
  - an install records the runtime by its blob id and names neither its
    merged `AGENTS.md` nor its merged `.mcp.json`; a dry run leaves the
    record as it was;
  - the first sync takes the recorded `ACCELERATOR.md` to the next release
    without the clone's history. A copy from an install made before records
    still stays as edited; that test now removes the record first.
  - a dry-run sync leaves the install's record unchanged;
  - `tests/test_harness_setup.py` compares the Harness's and the CLI's file
    modes outside `memory-bank/local`, where only the CLI's record lies.

### A client's sync rehearsed on a copy — 2026-10-22

- Why: `--sync` updates an installed project in place, and whether the
  memory hooks still saved and recalled afterwards showed only in the first
  real session there. Rehearsing it on copies of the four client
  installations by hand found that the sync leaves `runtime.json` - and so
  the capsule profile - as it was, holds back Codex's new hook while
  `.codex/hooks.json` waits for re-approval, and that a project whose
  accelerator is untracked fed `.claude/`, `memory-bank/` and the rest into
  its tasks' files.
- `scripts/memory_rollout_rehearsal.py --target DIR` repeats that rehearsal.
  It copies the project without dependency and worktree trees and syncs the
  copy. Then, for each host (Claude Code, Codex; Cursor on request) and
  capsule profile on a copy of its own, it runs the installed session-start,
  prompt and Stop hooks, the save the Stop request names, the closing Stop
  and a new session's prompt. It reports the
  sync, the profile `runtime.json` selects, every hook's exit and time, the
  save or its refusal code, the native journal and whether the next capsule
  recalled the save. No model runs, HOME and TMPDIR stay in the work
  directory, and the project is never written. A link out of the project is
  left out of the copy, and a Git worktree or submodule is refused.
- Tests (`tests/test_memory_rollout_rehearsal.py`, `installation` job; also
  in `scripts/check.py`): on a fresh PHP Core install, Claude Code, Codex
  and Cursor under `config` and `recall` each save and recall, the project is byte for
  byte unchanged, the copy leaves out `node_modules` and a link out of the
  project but keeps a link inside it, and this clone, a folder without an
  install and a worktree are refused.

### A checkpoint leaves out the accelerator's own untracked install — 2026-10-22

- Why: a project that neither commits nor ignores the accelerator shows
  `.claude/`, `.codex/`, `memory-bank/`, `specs/`, `AGENTS.md`, `VERSION` and
  the rest as untracked in `git status` on every turn, and `changed_paths`
  took them for the task's work. On copies of the four client installations
  83 of the 223 files the tasks of one client recorded were those (8 of 60 on
  another); its capsule offered `memory-bank/` as a recent file; and a turn
  that changed nothing still buffered a turn delta, so read-only sessions
  reached the five-turn checkpoint and showed a warming line.
- `changed_paths` now drops what the install record
  (`memory-bank/local/accelerator-install.json`, written by `--sync`) says the
  accelerator wrote and nobody has touched since: an untracked directory it
  installed into, and a file whose bytes still have the recorded Git blob id.
  A file the team has edited differs from the record and stays a change;
  without a record - attached, generated, or never synced - nothing changes.
- A checkpoint flush applies the same test to the turns it consolidates:
  turns buffered before the update still name the install, and the first
  flush after it would otherwise have made them the task's newest files.
- The core change was made in `PHP Core/` and written to the other editions,
  the generator asset and the mirrors by `scripts/core_sync.py --write`; the
  generator's policy lock was rewritten for its asset copies.
- Tests (`AutomaticWorkingMemoryTest`): an untouched untracked install changes
  nothing, an edited installed file and the project's own work stay, no record
  keeps the old report; a turn in such a project buffers no delta and the
  branch has no context; a flush leaves the install a buffered turn named
  out of the task's files. Each fails without the change.

### The automatic native save funnel, counted from disk — 2026-10-22

- Why: why installed projects held no saved result had to be reconstructed
  from transcripts, and once sessions run the disk still could not say how
  far a save got. The challenge file is rewritten every turn and pruned to
  128. A refused save's reason reached only the agent's tool output, and the
  recovery Stop then marked it `failed` without one. `native_capture_incomplete`
  lives one turn. The Stop hook left no trace when its save request failed
  or was killed.
- `native_results.py` appends one metadata line per prompt, Stop and save to
  ignored `memory-bank/local/native-results.ndjson`: `at`, `host`, `stage`,
  `event` and a fixed `code` - armed (with the phase the last request ended
  in when the hook warns), continuation, requested or the reason a Stop asks
  nothing (`not-governed`, `capsule-delivered`, `no-session-id`,
  `not-stop-event`, `not-armed`, `task-completed`, `recovery-<phase>`, ...),
  `failed` (`unsaved` or `incomplete`), held, saved/held with the learnings
  written and chunks promoted, omitted, or `refused` with the refusal's code.
  No payload, result, path, task or session. A call writes exactly one line;
  an input refused before `save()` runs is journaled from `read_input`, so
  `context.py` passes the request to it (two call lines).
- `memory_results.py` refusals carry machine codes (`ResultRefused.code`,
  `memory-result-v1:source_missing`, `:unverified`, `:learning_fields`,
  `:schema`, `:nothing`, `:sensitive`, `:stale_revision`, ...) while the
  message - the text the agent corrects and the Harness matches - is
  unchanged; `refusal_code()` gives any refusal its code.
- `working-memory-write.sh` appends the save request's exit status after it
  runs (`stop-hook` `exited`, 124 when the budget killed it), or
  `capsule-delivered` when a Harness turn skips it - written by the shell,
  like the prompt hook's health line, because a killed runtime cannot.
  Never through a linked journal file, nor through a link in an attached
  state. The Codex and Cursor copies name their own host.
- The journal keeps eight lines per `refresh_health_retention` record (a
  fully exercised turn writes about six), trimmed in batches so an append
  stays one write.
- `scripts/memory_eval.py realized` prints the funnel per host - prompt hook
  runs, armed, requested, saved, held, omitted, failed, refusals by code,
  learnings written, chunks promoted - and the prompt hook's
  `refresh-health.ndjson` lines for all hosts; `--state DIR` reads an
  attached project's state.
- The two Cursor guards that kept every clock out of the rule-rendering hooks
  - `test_render_carries_no_per_turn_invalidator` and the lint job's textual
  grep (`ci.yml`, `scripts/check.py`) - now admit the one on the journal
  line, which stays on one physical line: it writes local metadata, and no
  rule is made from it.
- The core change was made in `PHP Core/` and written to the other editions,
  the generator asset and the mirrors by `scripts/core_sync.py --write`; the
  generator's policy lock was rewritten for its asset copies.
- Generated projects write the Stop hook's line too: Infrastructure-Creator's
  hook-forge documents this Stop block as a mandatory fragment, and
  bootstrap-verifier requires it in a generated `working-memory-write.sh`
  once the seeded runtime journals (see `Infrastructure-Creator/CHANGELOG.md`).
  The generator's skill-body ceiling in `scripts/token_budget.json` rises by
  the 3,133 bytes that text added, to 302,433.
- Tests (`tests/test_memory_native_capture.py`, `NativeFunnelJournalTests`,
  and `tests/test_memory_eval.py`): every prompt, Stop and save path writes
  exactly one line with its code and no content; the Stop hook's line on
  success, failure, timeout and a Harness skip for Claude, Codex and Cursor;
  the bounded window; no write through a linked journal; the realized
  funnel and its text report. Each fails without the change, except the two
  link guards, which hold the implementation to its refusal.

### A path written out under the project root is found — 2026-10-22

- A prompt that names a file by its absolute path seeds the documents linked
  to it (`prompt_path_seeds`). The path was found by one pattern for the whole
  of it, which stopped at a space: under a root such as
  `/Users/ann/My Projects/shop` every such path was lost, and
  `test_prompt_paths_are_extracted_normalised_and_screened` fails whenever the
  temporary directory holds a space. The root is known, so a path under it is
  found by that prefix now; the character before the prefix is checked as the
  old pattern did.
- On Windows the root is also looked for as Windows writes it,
  `C:\Users\...\shop\src\...`; only its slash form was. On other systems
  the two forms are one and nothing changes.
- `test_a_path_under_a_root_with_a_space_is_found` and
  `test_a_windows_path_under_the_root_is_found` fail on `1b010289`.

### A chunk written today is served west of UTC — 2026-10-22

- The runtime writes chunk dates as UTC calendar dates - promotion,
  re-verification and supersession do, and an ID is minted from today's UTC
  date - but `validate.py` judged `last_verified` against the machine's local
  date. West of UTC the UTC date is the local tomorrow every evening (in New
  York from 20:00, in San Francisco from 17:00, summer time): every chunk
  promoted or re-verified then was "in the future", failed validation and
  left retrieval until local midnight. With the local calendar a day behind
  UTC, 120 of the 722 tests in `project-brain/tests/test_runtime.py` failed;
  now none does. A date is in the future only past today by both calendars
  (`validate.latest_today`).
- `bank-quarantine` dated its `valid_to` by the local calendar, unlike every
  other date the runtime writes; east of UTC after midnight that was the UTC
  tomorrow, and `test_quarantine_stops_serving_and_closes_validity` failed
  there every night between local and UTC midnight (00:00-03:00 in EEST,
  where the night check of this branch met it). It writes the UTC date
  now, and the test runs with the local calendar a day ahead
  (`calendar_shifted`, a 24-hour POSIX TZ offset), so it fails on a
  regression at any hour on a POSIX runner.
- `ChunkCalendarTest` and the quarantine test fail on `256c1c4b`.
- Not changed: the overdue-review, past-`valid_to` and terminal-chunk
  comparisons still read the local date. They move a boundary by hours once
  in a chunk's life rather than hiding the day's new chunks.

### A Windows session's PowerShell commands name their files — 2026-10-22

- Claude Code can run shell commands through its `PowerShell` tool on
  Windows (code.claude.com/docs/en/tools-reference): on by default without
  Git Bash, and with Git Bash for claude.ai and Console accounts. Its input
  has the Bash tool's `command`, but the transcript reader took only `Bash`,
  so a session that used it recorded none of the files its commands read -
  the main source of the `recent files` line. `PowerShell` calls are now read
  with the PowerShell dialect the Codex Windows path already uses.
- Where it applies: the editions' settings deny scoped `Bash(...)` rules, and
  on Windows with Git Bash - which the hooks need - Claude Code then turns
  the PowerShell tool off for the session unless
  `CLAUDE_CODE_USE_POWERSHELL_TOOL=1` or a `PowerShell(...)` rule turns it
  on. This reaches the sessions that turn it on.
- `test_claude_powershell_tool_calls_name_their_files` fails on `8caa2869`.
- Not changed, needs a decision: `bash-validator.sh` is wired on `Bash` only,
  while the documentation asks hooks that inspect shell commands to match
  `Bash|PowerShell`, so a session with the PowerShell tool on passes around
  it; and the memory CLI is allowed only as `Bash(...)` rules.

### A Codex chat snapshot no longer carries the Stop hook's continuation — 2026-10-22

- Codex 0.162 and later write a Stop hook's continuation into the
  conversation as a `<hook_prompt hook_run_id="...">` user message.
  `context_continuity.py` leaves the context Codex injects out of a snapshot,
  and its comment names hook prompts among it, but the envelope list did not:
  the native result's continuation - the request to save the turn, some 2,600
  characters - went into every Codex snapshot taken where native capture ran.
  On this machine's October rollouts (all in test and evaluation projects;
  none in the Desktop projects) that was 21 of 21 snapshots, a median 69 % of
  their text (53-80 %) and half of their user messages. `hook_prompt` is now
  in `CODEX_INJECTED_ENVELOPES`, and `docs/CONTEXT-HANDOFF.md` names it among
  the envelopes a projection drops.
- `test_a_stop_hook_continuation_is_not_part_of_the_chat` fails on `7df1da52`.
- Seen on the way, not changed: Codex 0.162.1 and 0.163.0 run no
  `UserPromptSubmit` hook for that continuation (in those 19 projects, 22
  user messages, 22 hook prompts and 21 read-hook health lines; a capsule
  followed 21 of 36 real prompts and none of the 21 continuations), and 20 of
  21 saves passed on a runtime that recognises only a bare continuation. The
  comment in `native_results.begin` that Codex submits the Stop reason through
  `UserPromptSubmit` describes earlier versions.

### The miss audit names every exclusion the runtime writes — 2026-10-22

- `scripts/memory_miss_audit.py` maps a manifest exclusion with reason
  `ticket-mismatch`, `invalid` or `explicit-context-only` to
  `runtime-exclusion`, and `path-link-limit` beside `file-link-limit` to
  `automatic-link-limit`. They read as `unknown`: on the 327-prompt implicit
  set, 43 of the 50 changelogs `main` withholds (the ticket filter: a changelog
  naming only other tickets than the request's) showed as unexplained misses;
  with the map they are runtime exclusions, the other 7 capsule losses.
- `test_every_manifest_exclusion_the_runtime_writes_has_a_stage` fails on
  `a54143b8`.

### A Codex turn records its files again — 2026-10-22

Codex 0.162 and later run every tool from one JavaScript cell, the custom
tool `exec`: a shell command is `tools.exec_command({cmd: "...", workdir:
"..."})`, a patch `tools.apply_patch("*** Begin Patch ...")`. The Stop hook's
transcript reader knew Codex's `function_call`, `local_shell_call` and the
`apply_patch` custom tool, none of which those versions write, so a Codex turn
recorded none of its files and the next session's `recent files` line stayed
empty. On this machine's 154 October rollouts (Codex Desktop and the TUI,
0.162 and 0.163) the reader found 0 files in 650 turns.

- `_transcript_calls` reads an `exec` cell's `tools.exec_command` and
  `tools.apply_patch` calls: their string literals - double, single or
  backtick quoted, `String.raw` included, JavaScript escapes decoded - go
  through the same shell and patch readers as before, from the cell's
  `workdir` when it is an absolute literal. A command built from a variable or
  a `${}` substitution names nothing the text can show and is passed over.
- On the same rollouts 42 turns record 90 files; most of those sessions worked
  in other checkouts and in `~/.codex`, whose files are not the project's.
- `test_codex_exec_cells_name_their_files` in
  `project-brain/tests/test_runtime.py` fails on `f6876f7d`.

### A session in another worktree is not served this checkout's memory — 2026-10-22

A Claude Code desktop session in its own worktree - and any session after
`EnterWorktree` - runs the hooks installed in the checkout it started from:
`${CLAUDE_PROJECT_DIR}` stays there, and the session's own checkout arrives
only in the input's `cwd` (code.claude.com/docs/en/worktrees, "What worktrees
share with the main checkout"). The memory hooks took their project and branch
from their own location, so such a session was handed the other branch's task
as its state, its turns were recorded into that task (`turn`, `native-stop`),
and its chats were captured as that branch's snapshots - a new worktree session
could even claim the merge prepared for that branch's next task. In the client
projects the accelerator is untracked, so a worktree holds no memory of its own.

- `working-memory-read.sh` and `working-memory-write.sh` read `cwd` with their
  other fields. When it is another worktree of the repository (another top
  level, the same Git common directory), the read hook prints that memory was
  not consulted, and the write hook records nothing; Cursor's read hook only
  answers `{"continue": true}`. `context_continuity.py` makes the same check
  and neither captures nor restores. A `cwd` equal to the project costs no Git
  call; a subdirectory of the project or another repository changes nothing.
- Tests: `OtherWorktreeTest` in `memory-bank/tests/test_hooks.py` (all three
  hosts) and `test_a_session_in_another_worktree_neither_captures_nor_claims_a_merge`
  fail on `a9277c8f`.
- `hook-forge` describes the check for generated projects; the hooks README of
  each edition names it. `scripts/token_budget.json` raises the
  Infrastructure-Creator `body_bytes` ceiling from 299,300 to 300,000 for that
  752-byte requirement: the skill bodies were 151 bytes under it.
- Serving such a session with its own branch's task and files (`--checkout`
  through the runtime's branch observations) is a separate change.

### A person's refresh asks for the full capsule on Windows too — 2026-10-22

- Harness read the `CAPSULE_PROFILE_FLAG` marker through `O_NOFOLLOW`, as it
  reads the source-hint and provenance markers, and Windows has no
  `O_NOFOLLOW`: there the marker counted as absent, and a person's knowledge
  `refresh` ran under the shipped `recall` and searched only the task's own
  saves. The marker only widens that refresh, so without `O_NOFOLLOW` it is
  read from a regular file that is not a link.
- `test_an_explicit_refresh_searches_everything_without_o_nofollow` fails on
  `46db067e`.

### The editions ship the recall capsule — 2026-10-22

- The four editions' `project-brain/config/runtime.json` and the generator's
  `runtime.json.template` set `capsule_profile: recall`: the automatic capsule
  carries the working state, the recent files and what the task's own saves
  recorded. The code's default stays `full`, so a project whose `runtime.json`
  names no profile keeps the capsule it had. An explicit `retrieve`, MCP
  `memory_retrieve`, Harness's explicit `refresh` and the LongMemEval bridge
  still search everything.
- The third task-outcome measurement sealed three conditions
  (`docs/MEMORY-EVAL.md`). On recall sets `recall` was not worse than `full`;
  on cold starts it was navigation byte for byte, and navigation had reached
  the changed file in 2.92 fewer calls than `full` on 30 client tasks. The
  third condition - every suite that saves a result and expects the next
  capsule to carry it stays green with `recall` shipped - failed on the two
  blockers the next two entries fix. Run again with `recall` shipped,
  `check.py` passed every job it ran.
- Tests that check a layer `recall` leaves out on purpose pin `full` in their
  fixtures: the shared document in the Cursor rule, the document beside
  broken UTF-8, Hindsight's source hints, the Git calls and the working
  record re-read on the full search path, the membership index's full capsule
  render, and Harness's first-message document, funnel diagnosis, ordinary
  receipt and opened-memory report.
- `AGENTS.md` in the four editions, `README_EN.md`, `README_RU.md`,
  `harness/README.md` and `docs/CONTEXT-AND-MEMORY.md` say what the automatic
  capsule carries.

### Harness works under a capsule without documents — 2026-10-22

Under the navigation and recall profiles a capsule may carry no document -
recall's whenever none of the task's own saves matches - and Harness assumed
one in three places.

- **Automatic memory.** `3ea0b262` let a capsule with no document through
  only off the local transport. On POSIX, where Harness asks the runtime for
  a source proof and runs the turn ephemeral, such a capsule still went to
  the proof check, and the runtime sends no proof when there is no document
  to prove: every such turn failed `prepare` with "The local source
  provenance is unavailable or invalid". A capsule with no document and no
  proof needs no fingerprints on either path now; a proof that does arrive is
  checked as before, and a document without one is still refused.
- **A person's refresh.** The knowledge `refresh` action ran under the
  project's automatic profile. It is an explicit request, like MCP
  `memory_retrieve`, and passes `--capsule-profile full` when the runtime
  takes it - `context.py` names that with a `CAPSULE_PROFILE_FLAG` marker, read
  the way Harness reads the source-hint and provenance markers.
- **The LongMemEval bridge** measures retrieval and asks `assemble_capsule`
  for the full capsule when the runtime knows the profile.
- `test_no_document_and_no_proof_need_no_fingerprints`,
  `test_an_explicit_refresh_searches_whatever_the_automatic_capsule_carries`
  and `test_the_bridge_measures_the_full_capsule_whatever_the_project_ships`
  fail on `60be9fb8`.

### A Memory Bank chunk is quoted by its rule, not its Sources footer — 2026-10-22

- `excerpt_section` skipped a citation-only `## Sources` footer for linked
  selections only. A chunk found by its own words - a request naming the
  file it cites - was quoted by that footer: the paths without the rule.
- The Harness recall suite showed it under the recall capsule: the corrected
  learning reached the next message as a list of sources. Under `full` the
  same chunk was quoted the same way, and the learning arrived only because
  the cited file came along.
- Every Memory Bank chunk is held to the linked selections' rule now; a
  request about the sources still gets them, and a distinctive pointer still
  carries no excerpt.
- `test_a_chunk_found_by_its_own_words_quotes_its_rule_too` replaces
  `test_lexical_selection_and_distinctive_pointer_are_unchanged`, which pinned
  the old quote, and fails on `1d4f4af5`.
- The 327-prompt stand under `full` does not move: the same documents, 217
  useful, the same capsule characters on every prompt.

### A recall capsule: navigation plus the task's own saves — 2026-10-22

- `capsule_profile: recall` (also `CONTEXT_CAPSULE_PROFILE` and
  `refresh --capsule-profile recall`) is the navigation capsule plus what the
  task's own saves recorded, searched by the request among themselves. The
  default stays `full`, and an explicit `retrieve` or MCP `memory_retrieve`
  still searches everything.
- What counts as the task's own:
  - a finding or decision whose name is the one a result of this task gives
    a learning (`record-result`, `native-result`, MCP
    `memory_record_result`): the name is computed again from the task's UUID
    and the record's type, title and sources;
  - a finding or decision whose name starts with the task's external ID or a
    ticket it names: a Harness draft (`<task>-finding-<hex>`), or findings an
    agent filed as `CMS-739-review-4` while on `feature/CMS-739-…`;
  - the Memory Bank chunk any of these was promoted to, found by the record
    it cites, also after compaction archives the record. A saved finding is
    resolved as it is written and no resolved finding is indexed, so for a
    finding the chunk is what the capsule can deliver.
- With none of them indexed the capsule is navigation's: no search and no
  delivery record. Recorded history, path and file links, related knowledge
  and source hints are left out; a conflict partner is still delivered.
  Lightweight mode has no governed saves, so there `recall` is navigation.
- `scripts/memory_outcome_ab.py`:
  - a `recall` arm;
  - recall sets: `recall-sources` lists first turns that answered from files
    they read, `recall-set` checks the learning and the follow-up question
    written for each against it, and `run` records the save on the corpus
    branch's task through `record-result` before any arm;
  - two regimes: the question a minute after the source turn (`next`), or at
    the first later session whose recent files no longer show the files the
    answer named (`later`);
  - the recall metric: calls to the first touch of a file the save cites, or
    to an answer naming one; `report --metric`;
  - each run records whether the hook delivered the save;
  - `--exclude-set` also takes a sample this tool made;
  - `recall-set` refuses a learning that cites a file the corpus at the
    question lacks - a file the source turn read before it was committed -
    which `record-result` would refuse when the save is planted;
  - a later question's task ID includes its source: several sources asked at
    the same later session shared one ID, and `run`, which now refuses a set
    with a repeated ID, kept only the last of their results.
- The measurement, under a rule sealed before the run
  (`docs/MEMORY-EVAL.md`, "The third measurement"):
  - On 30 recall scenarios in two regimes, `recall` was as good as `full`:
    - next: 0.00 calls [−0.26, +0.28];
    - later: −0.21 [−0.42, −0.02], and −0.11 [−0.32, +0.11] with four
      results recovered.
  - The hook delivered the save in every pair.
  - On 14 cold starts the `recall` capsule equals navigation's byte for byte.
  - With `recall` shipped, the three runtime recall suites navigation breaks
    pass. But the Harness recall suite fails: a promoted chunk is quoted by its
    `Sources` footer, which `full` hides by delivering the cited file beside
    it. The Harness's automatic memory also fails on a capsule without a
    manifest. `recall` therefore stays opt-in and the default stays `full`.
- The core change was made in `PHP Core/` and written to the other editions,
  the generator asset and the mirrors by `scripts/core_sync.py --write`; the
  generator's policy lock was regenerated.
- Tests: a saved result reaches the next capsule under `recall` and the bank's
  other matching chunk does not; another task's save does not, and without
  its own the capsule is navigation's; records filed under the task or its
  ticket count and a longer ticket number does not; a promoted save is still
  found after compaction; the refresh flag selects `recall`; an explicit
  retrieval still searches.

### A navigation-only capsule, behind an experiment switch — 2026-10-21

- `capsule_profile` in `runtime.json` (or `CONTEXT_CAPSULE_PROFILE`) chooses
  what the automatic capsule carries:
  - `full`, the default, is unchanged;
  - `navigation` keeps the working state and the recent files and neither
    searches nor records a delivery.
- Only the automatic entry points read the switch (`refresh` and
  `hook-context`); an explicit `context.py retrieve` still searches. An
  unknown value leaves the turn without a capsule and names the setting, as a
  bad retrieval gate does.
- Harness automatic memory accepts a capsule that carries no document. It
  refused the turn ("no valid source manifest") whenever the runtime wrote
  no manifest, which `navigation` never writes; a capsule with documents and
  no manifest is still refused.
- `refresh --capsule-profile full|navigation` overrides the environment and
  the config for one call. The MCP `memory_retrieve` tool reaches the capsule
  through `refresh`, but it is the agent's own request, so it passes `full`;
  the evaluation stand pins `full` in its runtime environment because it
  measures document retrieval.
- Why a switch and not a default:
  - The second, three-arm measurement (30 fresh client tasks, rule sealed
    before the run) found that the navigation capsule reached the file a task
    changed in 2.92 fewer calls than the full one (95% CI [−6.25, −0.40]); the
    full capsule against none again showed no difference.
  - Shipping navigation would also turn off what the hook delivers through
    the document layers: recall of a saved result in the next session, source
    hints, documents in the Cursor rule. Twenty-three suites pin those, and
    the measurement does not cover recall.
  - The default therefore stays `full`; `navigation` is opt-in.
- `scripts/memory_outcome_ab.py` gains the matching arm:
  - `navigation` runs the memory prompt hook with the profile set in the
    corpus;
  - a hook run is recognised by its `refresh-health.ndjson` line, since the
    profile writes no manifest;
  - `report --compare BASELINE,TREATMENT` compares any two arms;
  - each run records the token estimate of the documents the hook delivered.
- The core change was made in `PHP Core/` and written to the other editions,
  the generator asset and the mirrors by `scripts/core_sync.py --write`.
- Tests: the default capsule carries a matching chunk; `navigation` drops it
  and keeps the `working:` line; the environment overrides the config; an
  explicit retrieval still finds the chunk; an unknown profile degrades
  without failing the turn.

### Task outcomes, measured: capsule on against off — 2026-10-21

- `scripts/memory_outcome_ab.py` measures what the stand cannot: whether the
  automatic capsule changes what an agent does.
  - It rebuilds a project as of a real task's first prompt and runs a
    headless Codex session twice: without hooks, and with the memory prompt
    hook alone.
  - It counts the tool calls before the agent first touches a file the
    original session changed.
  - Runs stop at that touch, so a pair costs the start of two sessions.
  - Sets and results stay outside the clone; sessions are ephemeral.
- First measurement, under a rule sealed before the run:
  - 30 client tasks, two repeats per arm;
  - mean calls 17.10 without the capsule, 17.13 with it, 95% CI of the
    difference [−2.57, +2.65]: **no difference shown**;
  - each arm touched gold on 18 tasks;
  - the capsule took fewer calls on 15 tasks and more on 5 (sign test
    p 0.041), and about one call fewer where both arms reached the target
    (exploratory).
  - The document layers have not shown an effect on the outcome; a capsule
    reduced to working state and navigation is the next candidate, to be
    measured the same way.
- The memory-stand CI step now also runs the four test modules the
  2026-10-20/21 commits added without wiring them: rerank pilot, sqlite-vec
  pilot, fact consolidation and the history series pilot.

### The memory core has one source — 2026-10-21

- `scripts/core_sync.py --write` copies the memory core from `PHP Core/` to
  Laravel, Symfony and WordPress, removing a core file that was removed from
  `PHP Core/`. It then runs `asset_parity.py --write` and
  `build_mirrors.py --write`, so one command carries a core change to every
  copy, the generator asset and the tool mirrors. `--check` lists the copies
  that differ.
- The file set is the runtime's own `CROSS_EDITION_CORE_MANIFEST` minus
  `CROSS_EDITION_ALLOWED_DRIFT`; the framework-shaped `bash-validator.sh` and
  `local-context.sh` stay each edition's own.
- `.gitattributes` marks those copies and the memory-seed asset
  `linguist-generated`, so a pull request shows a core change once instead of
  five to seven times. `PHP Core/`, the framework hooks and the asset's own
  template and contract stay reviewed.
- The editions stay self-contained: the installer, attached mode and each
  edition's own test suites still find the whole runtime inside the edition.
  Removing the copies outright would have broken all three.
- Tests: the plan and writer on a synthetic four-edition tree, the real
  repository's copies, and the attribute marking through `git check-attr`
  (`tests/test_core_sync.py`, in the `parity` job).

### Review of the consolidation and ticket work — 2026-10-21

Fixes for what a review of the twelve unpushed commits of 2026-10-20/21 found,
each reproduced through the CLI on a temporary repository. Every behaviour
change below has a test that fails on `a47e1eb8`.

- **Inventories.** `install/inventories/*.json` carry
  `memory-bank/tests/test_identifier_terms.py`; `--verify-inventories` is
  VERIFIED again and the `installation` job passes.
- **A ticket is `PREFIX-123`, not "PHP 8.2".** A space no longer joins a
  prefix and a number, a number that runs on into a decimal is a version, and
  a standard's prefix - `UTF-8`, `PSR-4`, `SHA-256`, `ISO-8601`, … - is never a
  ticket (`brain_runtime.ticket_ids`, shared by consolidation and retrieval).
  The index also keeps the prefixes the project files its work under - its
  tasks' IDs and the tasks its completion events name (`ticket-prefixes` in
  `index_state`) - and, once there are any, retrieval counts only those, so
  `GPT-4` or `COVID-19` is not a ticket where the work is `CMS-741`. A request
  about Composer "on PHP 8.2" no longer withholds the chunk naming PSR-4, and
  in task `CMS-741` the chunk about UTF-8 exports is delivered again.
- **Consolidation keeps what the events said.** The finding carries every
  event on a line of its own - what was done, the outcome and its
  `Verification:` - not only the latest event's topic. Retrieval serves no
  resolved finding, so the events stay `recorded` until the finding's current
  revision is an active Memory Bank chunk; only then does a pass supersede
  them. A fresh clone after the flush answers "How are invoice totals rounded
  for CMS-741?" from the rounding event (observed) or from the chunk
  (verified and promoted); before, both events had left retrieval.
- Consolidation no longer supersedes events "into" another resolved finding of
  the ticket - active or archived - that does not carry their text: they stay
  recorded and the ticket is reported `existing`.
- The actor of a supersession is the operator, never the event's owner;
  another owner's events are skipped and left to that owner.
- `all_verified` counts events with a `Verification:` line: two events that
  both say "unit tests passed" verify the finding.
- Events that reach a ticket after its finding was written are added to it
  (`updated`) instead of staying "already exists" for good; a verified finding
  takes only verified events, re-attested by the agent, and an archived one is
  followed by a new finding. A pass reads the archive and the promotion ledger
  once, and not at all when every ticket's finding covers its events - it read
  the archive once per such ticket on every flush. At 40 such tickets and
  2,000 archived records a pass took 540 ms and now takes 12 ms.
- **Consolidation is opt-in.** `load_config` defaults
  `automatic_consolidation` and `automatic_consolidation_verification` to
  `false`, like promotion and compaction, and the four editions' `runtime.json`
  and the generator's `runtime.json.template` now ship them `false` too: the
  pass rewrites the team's tracked Brain on its first flush (27 of 28 events on
  the evaluated project) and a verified summary becomes an agent-attested chunk
  no person reviewed, and the redesigned pass has not been measured on a real
  project yet. The `consolidate` command is unchanged as the explicit replay.
- The same commits had copied PHP Core's `runtime.json` over the other
  editions: Laravel, Symfony and WordPress are `laravel`, `symfony` and
  `wordpress` again, and Symfony's tool-owned Cursor `skill-creator` command
  and agent, which `build_mirrors --write` had overwritten once the framework
  read `php-core`, are restored with their `.gitattributes` lines.
- **The changelog quote is its newest version.** A Keep a Changelog file was
  quoted from its preamble ("All notable changes …"); the head quote now skips
  the prose under the document title and names an entry whose heading has no
  prose of its own with its subsection: `[1.2.0] - 2026-10-01 › Fixed`.
- **Russian aliases add, they do not replace.** `query_tokens` keeps the
  Russian word beside its English term, so the chunk "Поля сущности счёта" is
  found by "Какие поля у сущности?" again, and each alias matches whole word
  forms: `логика`/`логин` are not log, `старт`/`стараться` not legacy,
  `действительно`/`действие` not validity, `полезно` not field.
- **Event slot.** Two events share the history slot only when events carry
  the ticket the request names; a ticket no event carries, or "PHP 8.3", left
  two neighbours there. The slot again weighs the halving for a changed
  source: a fresh 9.6 beats a 12.5 halved to 6.3.
- **Provenance is screened at indexing.** An auto-promoted chunk is checked
  for instruction-like text when it is indexed, under the same screened hash
  as the secret scan, instead of being read and scanned on every refresh. A
  warm `retrieve` with 1,000 such chunks took 359 ms on `a47e1eb8` and 284 ms
  before the check existed; it takes 238 ms now. A changed pattern set
  (`provenance-policy-fingerprint`) screens every chunk again.
- `bank-audit` without `--provenance` no longer prints a provenance line it
  never counted.
- The capsule's episodic layer holds three items - the changelog and up to two
  recorded events - since the two-event slot: `AGENTS.md` (four editions), the
  `project-brain` skill, `PROTOCOL.md`, the READMEs, `docs/OPERATIONS.md` and
  `docs/CONTEXT-AND-MEMORY.md` said two, and the Harness capsule validator
  refused a third item.
- Documentation: `docs/MEMORY-WRITE-PATH.md`, `docs/OPERATIONS.md` and
  `docs/CONTEXT-AND-MEMORY.md` describe the consolidation above and no longer
  call it on by default; `docs/RE-MEMORY-APPLICATIONS-2026-10.md` no longer
  says nothing was installed while its section 11 records the `dsh-ltm`
  deployment.

### A same-ticket event series pilot — 2026-10-21

- `CONTEXT_EVENT_SERIES=1` is an opt-in experiment: when the request names no
  ticket, the history slot may take up to two events from the largest
  same-ticket series instead of exactly one. The default remains one event.
- On the real 12-prompt stand the pilot changed selected recall 52.6% -> 55.3%
  and useful paths 19 -> 20, with answer passage recall 34.2% -> 36.8% and the
  same two noise-only turns. That is too small to change the default; the
  switch stays off and documented as a pilot.
- A distinct-ticket variant and a plain top-two variant were also measured.
  The first added only a noise document; the second changed nothing because
  ticket-bearing prompts already had a two-event limit. Both were removed
  rather than left as unused switches.
- History metrics now tag an entry by the indexed `brain-event` kind rather
  than only by its funnel stage. This corrected `history_paths` from 15 to 26
  and showed the real baseline: 13 of 26 event paths delivered (50%), not
  2 of 15. `history-event-limit` remains 15 -> 14 under the pilot.

### The miss audit keeps dynamic history visible — 2026-10-21

- `memory_miss_audit.py` now treats an unavailable Git source context during
  history admission as `unknown-history` instead of raising and erasing every
  judged path in the prompt. A per-path guard keeps the stages already
  established when one path cannot be resolved.
- `memory_eval` reports `history_paths`, `history_delivered` and
  `history_recall` beside candidate/selected recall. On the real 12-prompt
  stand this made the loss visible: 15 judged paths were dynamic history
  records, only 2 were delivered, while semantic candidate recall was 95.7%
  and selected recall 87.0%.
- `docs/MEMORY-EVAL.md` explains that candidate/selected shares cover
  semantic documents and history has its own share. The report prints all
  three, and paired deltas format the new share as a percentage.
- `memory_eval --audit-git` seeds a disposable Git snapshot for history
  admission when the evaluation corpus has no usable `.git`. It is diagnostic
  only, and the source-policy deadline still applies.
- History admission now receives the database connection, so the event filter
  no longer fails with `Git source context unavailable` in a seeded or
  attached corpus. On the real 12-prompt stand all 38 judged paths classified:
  candidate recall 97.4%, selected recall 52.6%, and 15 paths lost to
  `history-event-limit`; `unknown_history` disappeared.
- Answer-presence comparison ignores Markdown heading lines, and a labelled
  passage with no non-heading body is reported as `heading-only` and excluded
  from answer passages rather than counted as a retrieval miss. The audit and
  the evaluator use the same normalization. On the real stand 7 labels were
  heading-only; answer passage recall is 41.9% (13/31) instead of 34.2%
  (13/38). This is label hygiene, not a claimed retrieval gain.

### A reranker pilot before a runtime change — 2026-10-21

- `scripts/memory_rerank_pilot.py` compares the runtime's normal candidate
  order with a deterministic path/title feature reranker on a local index and
  judged queries. Path is unindexed in FTS5, so the feature gives a file name
  a bounded voice; the base BM25/authority score still dominates.
- The pilot is read-only, stdlib-only and never imported by the runtime. It
  reports per-query baseline/reranked useful paths and an aggregate, so a
  positive result is evidence for integration rather than a default change.
- `docs/MEMORY-RERANK-PILOT.md` explains the weights, the candidate window and
  why a low `candidate_recall` is out of scope. `tests/test_memory_rerank_pilot.py`
  covers feature scoring, promotion, metrics and malformed input.

### Decision templates keep the options that lost — 2026-10-21

- The Project Brain `decision` template and the durable chunk template now
  carry a `Rejected Alternatives` section. A future task that proposes the
  same option again can find why it was rejected; a fact or constraint deletes
  the section. This is authoring guidance, not a new admission rule: the
  existing promotion and retrieval rules are unchanged.
- The generator's memory-seed asset carries both templates.

### The memory stand separates candidate recall from selection — 2026-10-21

- `memory_miss_audit.py` now builds its query through the runtime's
  `expand_query_tokens`, so a judged path is diagnosed against the same
  identifier-aware vocabulary the capsule actually searched. An audit that
  used raw prompt tokens described a different retrieval from the one under
  test.
- `memory_eval` derives `candidate_recall` and `selected_recall` from the
  existing stage funnel. The denominator is the labelled paths the index could
  have returned (absent, not-indexed and unknown are excluded). A low candidate
  share points at query/index work; a high candidate share with a low selected
  share points at ranking, admission or budget.
- The report prints both shares and their paired deltas, and names the larger
  loss as `candidate-bound`, `selection-bound`, `mixed` or `none measured`.
  `MEMORY-EVAL.md` explains how to read them.
- Tests: a stage fixture with selected, relevance-drop, no-candidate,
  absent-source and unknown paths; a four-word identifier reaches the audit
  through the shared expansion; the report carries both rows. The full
  memory-evaluation suite is 93 tests.

### CamelCase identifiers and short continuations reach the index — 2026-10-20

- The local context index now carries a derived `identifier_terms` table: a
  document naming `getUserById` is reachable from "get user by id". The table
  stores only `(path, identifier, expansion)`; it is rebuilt incrementally for
  changed documents, once on an older index, and never becomes a second source
  of truth. `search`, capsule candidates, marked excerpts, episode search and
  excerpt weights all use the same expansion.
- Expansion is deliberately one-directional: words in the request gain the
  identifier the index spells, but an identifier is never expanded into its
  words. The reverse admitted every document sharing two common subwords and
  displaced the prompt-link upgrade that names a class. An opaque
  `SkylarkEpisode695` therefore does not match every unrelated
  `SkylarkSignal842` on the shared prefix. Ordinary words, short tokens and
  all-caps acronyms without a boundary produce no row.
- A short imperative continuation ("fix it", "do it") now borrows the active
  task's goal and next steps for governed retrieval. The prompt must name an
  explicit action verb; bare acknowledgements ("continue", "ok", "sure", "go
  ahead", "thanks") keep the existing no-injection behaviour, so a touch link
  still needs a shared informative term. The manifest records
  `query_source: continuation` and the combined query it actually searched.
- Tests: identifier forms for camelCase, snake_case, namespace and dotted
  names; the words-to-identifier direction through `search`; a negative test
  that an identifier does not expand into shared subwords; incremental
  replacement, document removal and an older-index rebuild; and an imperative
  continuation that reaches the task's specification. The full PHP Core
  Project Brain suite is 680.

### A Russian technical query reaches the English index — 2026-10-20

- `query_tokens` now applies a small, prefix-based seed glossary for common
  Russian software terms (path, log, drop, unknown, properties, entity, field,
  nested, list, ttl, preview, legacy, admin, …). It rewrites only Cyrillic
  words; English queries, code identifiers and tickets are untouched. The
  duplicated local `fts_query` in `context.py` now delegates to the aliasing
  implementation.
- On the real project's three Russian prompts that previously returned an empty
  selection, the same corpus now returns 2–3 semantic documents and the
  changelog; the TTL-preview prompt reaches the exact chunk
  `MEM-20260915-77964094-upgrade-note-overstates-legacy-preview-ttl-configurability`.
- The 12-prompt English stand is unchanged: 26 judged-useful documents, 10
  noise, 18/37 answer passages, 11/12 useful turns.
- Tests: the mapping produces path/written/log/drop/unknown/properties/entity
  and an in-memory English document is found from the Russian question; the
  full PHP Core Project Brain suite is 678.

### Every-event evidence verifies a consolidated finding — 2026-10-20

- With `automatic_consolidation_verification` (on in the shipped config), a
  consolidated finding becomes `verified` only when **every** source event
  recorded a `Verification:` line and at least one local source was
  fingerprinted. The authority transition is agent-attested, like the existing
  Harness save path; a finding without that evidence stays observed.
- This lets the same turn boundary's automatic promotion turn the verified
  summary into a durable chunk; an observed summary remains only in the Brain.
- Tests: two verified events produce a verified finding, two unverified events
  do not; the full PHP Core Project Brain suite is 678.

### A consolidated summary names its ticket's existing findings — 2026-10-20

- The summary's `progress` now lists the external id and UUID of every finding
  that names the same ticket, including an open/investigating one, after the
  event ids and verification. The capsule carries the link in text without a
  schema change: the reviewer can follow the specific review finding from the
  summary, and the two records keep their distinct scope.
- Tests: an open `CMS-741-REVIEW` finding is named by the new summary and keeps
  its status; the full PHP Core Project Brain suite is 678.

### A resolved ticket finding suppresses a duplicate summary — 2026-10-20

- If a ticket already has a `resolved` finding, `consolidate` no longer creates
  a second `CONSOLIDATED-*` conclusion. It transitions the ticket's completed
  events to `superseded` with the existing finding named in the reason and
  reports the ticket as `existing`. An open/investigating finding is a specific
  work item, not a conclusion, so it does not suppress the summary.
- Tests: the existing resolved finding survives untouched, no duplicate is
  created, and both events are superseded; the full PHP Core Project Brain suite
  is 674.

### Automatic compaction keeps unverified consolidated findings active — 2026-10-20

- The turn boundary creates a consolidated finding as `resolved`/`observed`,
  then the same boundary's automatic compaction archived it immediately because
  `resolved` is terminal. On a real-data copy all seven new findings landed in
  `archive/finding` and no capsule could reach them. `compact` and
  `archivable_records` now retain a `CONSOLIDATED-*` finding until its authority
  is `verified`; superseded events still archive normally.
- On a fresh copy, `turn --flush` created seven findings, superseded 27 events,
  left all seven in `dynamic/findings`, archived none of them, and `validate`
  passed. The manual `consolidate --apply` path already left them active; this
  closes the automatic path.
- Tests: the automatic boundary with `compaction_threshold=1` keeps the
  unverified finding in `dynamic/` and out of the archive; the full PHP Core
  Project Brain suite is 673.

### Two same-ticket events share the history layer — 2026-10-20

- When the request names a ticket, the history layer admits the two
  highest-ranked events that carry it instead of one; without a ticket it keeps
  one. On the 12-prompt stand this delivered 18/37 answer passages instead of
  15/37 (40.5% → 48.6%), 26 judged-useful paths instead of 21, with noise
  unchanged at 10 and no unjudged documents after both new events were graded
  1 (same ticket / same subject). The governed
  capsule contract now keeps the changelog plus two event/episode slots.
- `_under_git` asks Git whether a `.git` entry is a usable repository instead of
  trusting the entry's existence. A stale or empty `.git` above a plain project
  made `context-save` fail with "cannot verify source ignore rules"; the
  outside-Git handoff test now covers that case.
- Applying automatic consolidation to a copy of the evaluated project with its
  cited sources created seven findings from 27 completed events, superseded all
  27, blocked none, and passed `validate`. A generic latest goal no longer
  becomes the finding title: `CMS-744: completed Verification: ...` became
  `CMS-744: Remove legacy unsigned ACL headers and preserve SDK presigner
  behavior`.
- Tests: a named ticket delivers both of its events, the governed contract
  keeps three history items, and a generic latest goal uses the descriptive
  event title; the full PHP Core Project Brain suite is 678.

### Memory maintenance runs on the turn boundary — 2026-10-19

- `automatic_consolidation` is on in the shipped config (and in the generator's
  `runtime.json.template`). At the same flush boundary that promotes and
  compacts, the runtime collapses a ticket's completed-event series into one
  observed finding before promotion, so the user does not run `consolidate`
  for normal work. The command remains as the explicit replay/backfill.
- An `auto-promoted` chunk whose title/body is instruction-like is now withheld
  from retrieval automatically, because no person read it. A remote citation is
  reported by `bank-audit --provenance` but is not withheld by itself: a pinned
  source URL is legitimate evidence, and withholding every one lost a measured
  useful chunk. Manually reviewed chunks are not withheld.
- `automatic_consolidation: false` restores the previous event-only behavior.
- The turn result reports `consolidation` (`enabled`, `created`, `superseded`,
  `blocked`, `tickets`, `error`) beside promotion and compaction; a maintenance
  failure is reported there rather than failing the checkpoint.
- Tests: automatic boundary consolidation, disabled-setting behavior,
  idempotent second boundary, shipped-config flag, and automatic withholding
  of an auto-promoted instruction-like chunk.

### Memory write path: consolidation, provenance and deterministic facts — 2026-10-19

- `context.py consolidate --ticket T` (dry-run unless `--apply`) turns two or
  more completed events for one ticket into one `resolved`/`observed` finding.
  The finding cites the events' local existing sources, keeps their IDs and
  `Verification:` lines in `progress`, and is idempotent. The source events are
  transitioned to `superseded` by their own owner and are left for `compact` to
  archive; private and restricted events are not folded into a team finding.
- Automatic promotion now refuses a record whose cited source is `http(s)://`
  or whose title/content is instruction-like (`ignore previous instructions`,
  `system prompt`, chat-template control tokens). A person can still review and
  promote such a record; the gate is only on the unreviewed path.
- `bank-audit --provenance` adds active auto-promoted chunks, chunks citing a
  remote source, and instruction-like chunks. `bank-quarantine --id ...` closes
  a chunk's validity at today and archives it, so a suspected poisoned or unsafe
  memory stops being served immediately.
- `scripts/memory_fact_consolidation.py` implements the deterministic
  conflict-resolution recipe: group candidates by fact, take the maximum
  explicit serial/version/revision, break an equal version on `updated_at`, and
  report a true tie as ambiguous. It is not wired into the capsule: records
  that declare each other in `conflicts` remain shown together by contract.
  The known-answer fixture is `tests/fixtures/fact-consolidation.json`. When two
  records that declare each other in `conflicts` are both selected, the capsule
  now names the partner paths in `conflict`, beside the existing
  `match: conflict`; neither record is dropped.
- `scripts/memory_sqlite_vec_pilot.py` probes local `sqlite-vec`/`sqlite-lembed`
  assets and fuses lexical and dense path lists with RRF. It is maintainer-only,
  never imported by the runtime, fails closed, and adds no default dependency;
  see `docs/MEMORY-SQLITE-VEC-PILOT.md` and `docs/MEMORY-WRITE-PATH.md`.
- Tests: five event-consolidation cases, provenance audit/quarantine, the
  provenance problem gate, the fact-consolidation fixture and the fail-closed
  pilot/RRF suite; the full PHP Core Project Brain suite is 671 tests.

### A shared class name no longer anchors a weak match — 2026-10-19

- A document whose only query match is a name-shaped word is admitted as
  `distinctive`; the anchor test treated any CamelCase word as a name. On the
  current 12-prompt run three chunks about neighbouring tickets filled the
  capsule for a FileSubscriber cleanup prompt because they share the
  `EventSubscriber` directory in their sources - that word appears in five
  documents. A name-shaped term now anchors only when it appears in at most
  three documents or carries a digit: the shared directory stops admitting,
  while `loggableName` in one document still does. On the same run the three
  chunks are gone, the prompt's `CHANGELOG.md` takes the room, and the useful
  turn remains useful.
- Recorded history's floor was the weight of a term appearing in 1.5
  documents, so an event naming a ticket could never be reached when the
  ticket's prefix was common: six CMS-741 events matched both query concepts
  and none passed. A match that includes a name-shaped term now clears the
  coverage floor, while a match of ordinary words keeps the strict floor, so
  a weak event still leaves the slot empty (the weak-event contract test
  passes). The CMS-741 turn gains its event and becomes useful.
- The recorded-history slot now prefers the event that carries the ticket the
  request names, when one is admitted. On `next-9293f3` (CMS-741) the
  higher-scored CMS-740 event (8.22) led the slot while five CMS-741 events
  followed; the CMS-741 event `805d009f` now takes the slot, and the CMS-740
  event leaves. A request with no exact-ticket event keeps its current event,
  so the ACL prompts helped by a CMS-743 event beside a CMS-735/753 request
  are unchanged.
- A durable chunk or finding that names only another ticket is withheld. On
  `next-9293f3` the finding `903cd91e` (CMS-739) and on `next-3b2ec4` the
  chunk `7cc0d43f` (CMS-739) were graded noise beside a CMS-741/735 request;
  both are gone, with no useful document lost. A chunk with no ticket (the
  TTL note in 5b9f61/6a4bf1) and a finding with the request's ticket are kept.
- A changelog is kept when the request names it or it mentions a ticket the
  request names; otherwise a request with a ticket the changelog lacks no
  longer receives it. `next-32c2e4` (CMS-741) and `next-53eabd` (CMS-740)
  lost a changelog that named neither, while `next-9293f3` keeps it because
  the request names `CHANGELOG.md`, and 1145cc/3b2ec4/b548e8/5b9f61 keep it
  because it names their ticket.
- The working task supplies the ticket when the request names none: its
  external id, title and goal are read in memory only. The ephemeral
  evaluation has no task, so this is covered by its unit test rather than by
  the 12-prompt run.
- Measured on the same 12 prompts, with every delivered document graded by an
  agent (not a person): useful turns rose from 10 to 12, useful documents from
  18/35 to 21/31 (51.4% to 67.7%), answer passages from 13 to 16 of 38 (34.2%
  to 42.1%) and answer-in-text from 7 to 10 of 12; noise fell from 17 to 10
  documents. Warm refresh p50/p95 was 0.38/0.50 s and the largest capsule
  3,193 characters. The anchor cap alone gives 11 useful turns and 14/38; the
  anchor-specific history floor adds the last turn and 15/38; the exact-ticket
  preference adds one useful document and 16/38; the two ticket filters
  remove four noise documents without losing a useful one.

### A changelog's excerpt starts at its newest entry — 2026-10-19

- A document of kind `changelog` now quotes the first section that has a body,
  and that section's opening, instead of the window its shared words picked.
  On a conflict notice that names `CHANGELOG.md`, "web editor" and "command
  line" matched an older entry about an entity menu and an editor; the newest
  entry, which names the release the conflict is in, was never shown. The
  same choice displaced the newest entry on review prompts whose vocabulary
  recurs throughout the file.
- The head window fills its allowance from the next sentence too: a changelog
  entry is one long list item, and stopping at the first sentence cut the
  answer that the entry carried. The window stays within its layer's limit,
  and a head-preferring quote finds the first body section directly rather
  than scoring every section of a file that can run past a megabyte; warm
  refresh p50/p95 stayed around half a second (0.52/0.55 s against 0.48/0.53 s
  before).
- Measured on the 12-prompt labelled set with the answer passages normalised
  to the entry body (the old labels carried the previous renderer's heading
  prefix, which the current format no longer repeats): answer passages
  delivered rose from 7 to 13 of 38 (18.4% to 34.2%) and `answer in the
  capsule text` from 5 to 7 of 12, with the same 7 useful and 3 noise-only
  turns, the same 2.92 delivered documents per turn and the same 22 graded
  noise documents. The largest capsule grew from 3,172 to 3,193 characters,
  under the 3,600 ceiling. Against the unnormalised labels the change is
  neutral (7 of 38), because those passages include the heading prefix and
  the full entry; the measurement artifact is recorded here rather than
  fixed by re-adding headings to the rendered body.
- The 12-prompt labels were written against the older runtime's event-heavy
  delivery, and the current runtime deliberately caps recorded history at one
  event, so comparing its answer recall to the older 28-of-38 score is not
  like for like. The 15 ungraded documents were graded in the local corpus by
  an agent, not a person; fresh human labels for the delivered set remain the
  open measurement.
- The excerpt pilot's stand-in for `excerpt_section`
  (`scripts/stateful_pilot.py`) takes `prefer_head` and leaves a changelog's
  head to the runtime instead of a scored section: without it every refresh
  under the pilot failed with a TypeError, and the memory evaluation stand
  went red. `tests/test_memory_onnx_pilot.py::ChangelogHeadPilotTests`.

### The sync keeps ACCELERATOR.md at the release — 2026-10-18

- `scripts/install_accelerator.py --sync` follows the edition README that
  `--merge-existing` installed as `ACCELERATOR.md` beside a project's own
  `README.md`, by the rules of every other accelerator file: it records the
  copy in `memory-bank/local/accelerator-install.json`, updates it while it
  holds a README the edition shipped or what the last sync wrote, and keeps
  one the team edited, listed under `kept` (a copy the project's Git tracks
  waits for `--update-tracked`). The sync used to skip it and never record
  it, so once a release changed the README, adding a tool to a synced
  project (`--edition Symfony --tool claude --merge-existing`) was refused
  with `accelerator-readme-exists`. On four client projects synced to
  Symfony 2.0.0 that was the only collision left, and their copy was the
  README the edition shipped from 2026-08-12 to 2026-10-06. Copies from
  syncs before this change have no record and are recognised from the
  clone's Git history of the edition README; without that history (a source
  from an archive) an older copy stays, reported as edited. The sync writes
  no `README.md` and adds no `ACCELERATOR.md`: an absent one was removed,
  or the project had no README and got the edition's as `README.md`.
  `tests/test_installation.py::AcceleratorReadmeSyncTest`,
  `SyncOfAnOlderReleaseTest::test_a_tool_added_after_the_sync_finds_the_accelerator_readme_current`.

### Recent files from what the agent read and changed — 2026-10-18

- The Stop hook hands `turn --transcript` the transcript the host names. The
  runtime keeps the project files the turn read or changed - tool-call paths
  and the words of shell commands (`cat`, `sed -n`, `grep`, `git diff`,
  `git add`, redirections) that name an existing project file, changed
  first - as the project's recent files in local state, newest first, 64 at
  most. Commands, tool results and file contents are never kept;
  dependencies, prose, ignored and secret-looking paths are left out; an
  unreadable transcript records nothing and never fails the turn. Claude Code
  and Codex transcripts are read; a host that names none keeps the old line.
- The capsule's `recent files:` line shows ten - the project's, then the
  task's changed files they do not name, whole paths within 720 characters -
  and shows them before the task is recorded too. On 1,940 real Claude Code
  turns of four projects, ten such files named at least one file the agent
  then read or changed on 75% of new sessions' first turns (held-out, 140),
  against 58% for the task's five changed files (+17 points, 95% CI 10-24);
  the share of the needed files named rose from 31% to 54%. The reads came
  almost wholly through shell commands: the Read tool's paths alone named
  56%, no better than before.
- Review fixes. A shell command is read with its quoting: split on `|` and
  `;` as text, a quoted grep alternation, sed script or jq filter lost its
  file (`rg "a|b" x.php`, `sed -n '1,20p;30,40p' x.php`). A redirection
  names its file however it is written (`>out`, `&>out`, `>&out`,
  `2>&1 >out`); `cd` and `git -C` move the words after them, a subshell's
  `cd` ending with it; a here-document's body is data; `sed --in-place` and
  `perl -pi` change their file, `cp` and `mv` only their last operand; and
  Codex's PowerShell and cmd commands on native Windows are read. A turn now
  starts at any user message but a local command's output: a slash command,
  pasted text and a background task's `<task-notification>` (132 of 416 on
  400 recent Claude Code transcripts went on to call tools) had joined the
  turn before, whose files then came back as the newest. The task's changed
  files the project's push off the line are counted in the compaction line
  (they had gone without a word), and a lightweight capsule shows the line
  too. `RecentFilesShellCommandTest` and `RecentProjectFilesTest` in
  `project-brain/tests/test_runtime.py`.
- `turn` reads the transcript after the turn's own record, flush and
  promotion, and stops reading its calls after a second, keeping the files
  found: read first, a long transcript on a slow disk could run the Stop
  hook out of its five seconds and lose the checkpoint itself. Each checkout
  of an attached project keeps its own recent files (an installed project
  keeps them in its checkout's database, as before): attached checkouts
  share one state, and parallel sessions in two worktrees named each other's
  files. Excerpt growth reserves the whole `recent files:` line while the
  capsule carries one, so a grown excerpt no longer makes the renderer cut
  the later ones. `test_attached.py::AttachedRecentFilesTest`,
  `RecentFilesExcerptRoomTest`.

### Excerpts grow into the capsule's room — 2026-10-17

- The first and second delivered excerpts whose section runs on past their
  window grow into the room the capsule leaves, up to 1,800 and 1,200
  characters; the rendered capsule still stays within 3,600. On the labelled
  prompts a capsule used 600-2,200 of its characters while a delivered
  document's answer sat just past the window. Answer passages delivered rose
  from 9 to 11 of 33 and from 3 to 5 of 7, with the same useful, noise-only
  and silent turns; the median capsule grew from about 880 to 1,570-1,830
  characters. The room is reckoned from a fixed reserve for the head, so a
  document's window for a request is the same on every turn and repeat
  detection still recognises it.
- `memory_eval.py` also counts the answer passages the runtime can reach -
  present in the text the index holds for the document - and those in
  documents longer than it marks whole (64 KB). On the labelled sets every
  existing passage was reachable.

### Native final-answer preservation — 2026-10-17

- Keep routine native memory capture silent and preserve the original substantive answer, source citations and requested format after successful or skipped capture. A save acknowledgement must not replace the task result consumed as the last agent message. Refused saves retain their warning alongside the answer.
- Reproduced against an actual Codex Stop continuation; empty reusable learnings remain valid and do not create findings merely to pass a lifecycle probe.

### A pause of a second no longer costs a query its memory — 2026-10-16

- `refresh`, `retrieve`, `context`, `hook-context` and `index` ask Git for the
  project's source frame before anything else when they have no branch to
  admit - an explicit `CONTEXT_TASK_ID`, an agent's own `retrieve`, a detached
  HEAD - and gave that one `git rev-parse` one second, half of what every
  other Git source check has. A windows-latest runner in a burst of slowness
  failed a refresh with "Git source context unavailable": that check was the
  refresh's only Git process, and the whole test took 2.17 s, too short for
  any two-second check to have run out. On 8 runners over 30 minutes the
  check never took more than 535 ms in about 24,500 runs, but on three of
  the machines a 5 ms sleep in the probe woke up to 963 ms late, and one
  second left no margin. The check now has the same two seconds; a Git that
  does not answer within them still ends the command with that error.

### Automatic promotion apply admission — 2026-10-16

- Recheck configured near-duplicate admission inside automatic promotion apply, including stalled retries. A chunk left by an interrupted apply cannot be duplicated under a later date; the original chunk and authoritative records remain untouched. A valid revision still excludes its recorded predecessor.
- Add a date-bound interruption/replay regression and public process-exit controls. This prevents a later normal write from duplicating knowledge; it does not adopt an unproven chunk or promise arbitrary crash recovery.

### Correctable native result preflight — 2026-10-16

- Reject conflicting learning identities during normalization, before native task provisioning or digest binding. A refused first payload can be corrected in the same continuation; identical duplicates still save once, while genuinely begun results retain their exact-payload and canonical-proof guards.
- Cover existing and unprovisioned tasks, normalized identity conflicts and duplicate save/replay through the public CLI. Apply the shared JSON contract across all editions and generator assets.

### Whole-planned memory delivery — 2026-10-16

- Persist the full selected logical plan before evaluation and freeze source opportunity before warm-up. Completed refresh failures retain their source denominator; interruptions preserve in-flight evidence without changing terminal-only successful-intersection metrics.
- Report supplementary unit delivery and passage recall only for complete, internally consistent plans with known source opportunities. Legacy, malformed, missing-source and interrupted evidence cannot produce a final rate; no whole-planned paired delta or task-success/cost claim is inferred. Add actual producer, timeout, interruption, checkpoint-failure, frozen-source and malformed-evidence regressions.

### Recorded memory-evaluation compatibility — 2026-10-16

- Classify comparison metadata before numeric reports: matching recorded inputs/source conditions and budgets retain paired gains; known mismatches withhold quality/latency deltas and gains; missing or malformed legacy evidence is explicitly diagnostic. Runtime treatment versions may differ. Preserve the existing successful-intersection arithmetic and visible coverage losses.
- Keep malformed legacy headers readable and treat out-of-range UTC conversions as unknown timestamps. Add mismatch, typed/absent evidence, runtime-treatment and real graph-pilot report regressions; document that recorded compatibility does not prove corpus-body identity or whole-planned task outcomes.

### Native exact-result recovery — 2026-10-16

- An already-started native save can finish through the canonical guarded retry after its recovery Stop marks the challenge failed. Completed saves replay without requiring a still-present citation or active task alias, including after archival and alias reuse.
- Native retries require the original content, pinned task UUID/revision and canonical result evidence; missing evidence cannot turn a retry into a first write. Fresh saves still check current sources before task creation. Add interruption, archive, disposable receipt loss and refusal regressions; the single automatic continuation remains bounded.

### Memory suites on native Windows — 2026-10-16

- The 23 memory regression suites that failed on their first native Windows
  run pass there, in a new `windows-memory` job: with them `windows-harness`
  took 12 to 14.5 of its 15 minutes. Where a Windows user hit the same
  failure, the runtime is fixed:
  - Python 3.12+ on Windows gives `os.stat()` and `os.lstat()` the creation
    time as `st_ctime_ns` and `os.fstat()` the last change time, so every
    check that an open file is still the one its path names refused the same
    unchanged file: learning images were never recorded, and the save
    frontier and the Cursor continuation topic were always unavailable. Those
    checks compare `workspace_roots.change_time_ns()`.
  - `validate.py` compared each Memory Bank index row (`chunks/<file>`) with a
    backslash path: on Windows every chunk failed validation and every
    automatic promotion was refused.
  - The memory MCP server and its registration script refused a project root
    spelled with 8.3 short names (`C:\Users\RUNNER~1\...`) as a symlink
    root. Short names are now expanded with `GetLongPathNameW`, which keeps
    links, before the check.
  - Source hints abstained on Windows, which has no `O_NOFOLLOW` or `dir_fd`;
    they now read through the NT handles the Cursor rule already uses.
  - `workspace_roots.key_for` no longer files `\\srv\a.md` - the share `a.md`
    on the server `srv` - under `\\srv`.
- Where only the tests were not portable, the tests are fixed: resolved
  temporary roots, hooks run by Git Bash (`tests/hook_shell.py`: a bare
  `bash` is WSL's launcher in System32), POSIX manifest keys and Git config
  paths, LF fixture bytes, closed SQLite connections, runtime modules from a
  temporary copy removed from `sys.modules` (the Windows jobs run their
  suites in one process), and a churn and a branch-race fixture that hold
  with Windows' coarse file times and Git Bash's own `PATH`.

### Native Windows CI — 2026-10-16

- The Windows hook test gives Git Bash each hook command in a file. Passed as
  `bash -c` from Python, `"${CLAUDE_PROJECT_DIR}"/...` reached Git Bash without
  its quotes and failed to parse, so the test stopped before any hook ran.
- `windows-harness` runs the memory suites that pass on native Windows. The 23
  that failed on their first native run - 8.3 short temp paths (`RUNNER~1`,
  which the memory MCP server then refuses as a symlink root), SQLite files
  still open at cleanup, Cursor rule custody, generator lint paths - run on
  Linux only, named in `scripts/check.py`, until they are ported.

### Automatic structured native results — 2026-10-16

- Native prompt/Stop hooks coordinate one same-agent structured save, including clean research tasks without Git edits, through the existing guarded result contract. No raw transcript is adopted; task completion and person review stay explicit.
- Challenges pin owner/session/task revision, checkout and runtime; repeats are idempotent, recovery checkpoints are suppressed, conflicts/partial outcomes remain held and the next capsule reports failures. Cursor follow-ups are capped at one.
- Add real-hook roundtrip, replay, schema/source/privacy, stale scope, corrupt metadata and interruption regressions; include them in the standard check runner.

### Memory fixes after the assessment — 2026-10-16

- Automatic memory preserves manual corrections when a learning-image proof
  is missing or could not be written; deliberate corrections still accept an
  explicit current revision. Promotion duplicate checks retain Unicode words,
  so independent Cyrillic findings from one source both reach the bank.
  Native Stop hooks use the branch being rebased for checkpoints, consistently
  with prompt reads, across Claude Code, Cursor and Codex.
- Failed or interrupted automatic promotion leaves an owner-scoped ignored
  retry marker. A due governed turn retries one record after a 30-second delay,
  without requiring a Git delta or creating a task. Retries rotate past failed
  records, obey existing source/owner/configuration guards, and drain blocked
  duplicate work; schedule-write failures do not trigger repeated full scans.
  Source traversal retains the existing promotion implementation and hook
  timeout. Markerless failures from older runtimes still need a normal flush
  or an explicit replay. Regression coverage exercises save conflicts,
  Unicode promotion, rebase adapters and bounded clean-turn recovery.

### Memory reliability round — 2026-10-15/16

- A reply with no word in it - `?`, an emoji - no longer fills the turn with
  the operator's report. It passed the sanitizer, failed the word check
  later, and its prompt got four lines of layer report and `Capsule
  unavailable: Search query must contain a word`, often on the turn that
  approves a plan. Such a prompt is now withheld like one the sanitizer left
  empty: the turn is silent, and Cursor keeps its rule. Tests:
  `project-brain/tests/test_runtime.py` (`WordlessPromptTest`).

- A branch whose task is finished says so, and the way on works. Work that
  went on after the task was completed - review fixes, say - is not recorded,
  by design, yet its prompts read `working: not recorded yet (the task starts
  at the 5-turn checkpoint)`, though no checkpoint would start one, and the
  advice to start a new task was refused: the finished record keeps the ID
  until compaction archives it, and automatic compaction waits for five
  finished records. The capsule now opens with `working: closed (this
  branch's task is finished; turns are not recorded until a new one starts:
  start a new task: context.py start --task-id <id> --goal "...")`, without
  the warming line, and the turn report says the same; `start`, refused only
  by finished records of that ID, now moves those to the archive - no other
  record - and creates the task (`compact(only_external_id=...)`). Tests:
  `project-brain/tests/test_runtime.py` (`ClosedBranchTaskTest`).

- Knowledge that stops being retrieved because its cited file moved is no
  longer silent. A Memory Bank chunk citing a file renamed with `git mv` (or
  deleted) is left out of every capsule, while the session banner went on
  counting it as active. The summary now adds `N active chunk(s) cite a file
  that is gone and are not retrieved; memory-bank/scripts/validate.py names
  them.`, and `validate.py` follows Git's renames to say where each such file
  went (`source path does not exist: AGENTS.md - Git moved it to POLICY.md`);
  the checks on every prompt start no Git for it. Tests:
  `memory-bank/tests/test_validate.py`.

- A branch renamed with `git branch -m` keeps its task. Its prompts read
  `working: not recorded yet`, and its fifth turn recorded a second task
  beside the one holding the branch's progress and next steps. Git moves the
  reflog with the branch and notes `Branch: renamed refs/heads/<old> to
  refs/heads/<new>`; when the branch the checkout is on has no active task
  and an earlier name has one, the commands the hooks run (refresh,
  hook-context, turn, cursor-rule, msg-dispatch) work with that task, and the
  capsule says `This branch was renamed from <old>; its task continues here.`
  The reflog is read as a file, so a branch never renamed costs nothing
  more; a copy of a branch and a finished earlier task start their own, the
  record keeps its ID, and an explicit `start` for the new name still
  records a new task. Tests: `project-brain/tests/test_runtime.py`
  (`RenamedBranchTaskTest`).

- Several sessions on one checkout no longer cost each other the working
  state. A prompt's capsule reads the task record, retrieves, and checks
  before delivery that the record is still the one it read; another
  session's turn checkpoint rewriting it in between withdrew the working
  state, and the capsule said only `working: unavailable`. With four sessions
  checkpointing every turn, 22 of 48 prompts lost it. The record is now read
  again just before the capsule is signed and packed, and its newest state is
  delivered (a query derived from the task keeps the state it came from); a
  rewrite in the last moments still withdraws it, and the capsule then says
  so and names the record to read. 47 of 48 such prompts keep their working
  state. Tests: `tests/test_memory_brain_scale.py`
  (`ConcurrentCheckpointTests`).

- A large Brain no longer costs every prompt a read of every record. Each
  prompt is a new process, and each one read, parsed and validated every
  Brain record again - with 3,133 records most of the prompt. The images the
  runtime already kept for one process are now carried to the next in the
  ignored `project-brain/local/record-images.cache`: keyed as before by each
  file's identity (device, inode, size, modification and change times), tied
  to the validator's file and the Python that wrote them, and holding only
  this project's records. A record changed since is read again; one changed
  within two seconds of a save is left out, since a coarse clock could give a
  second edit the same times; anything unreadable is a miss, never an error;
  and a refresh that promises no filesystem writes (read-only or degraded)
  writes none. A prompt on 3,133 records went from 441 to 325 ms, on 524
  records from 222 to 204 ms; a turn checkpoint is unchanged. Tests:
  `tests/test_memory_brain_scale.py` (`BrainScaleTests`).

- A rebase no longer leaves the turn without memory. It detaches HEAD on the
  commits it replays, so the prompt hook found no branch and named no task -
  exactly while a turn asks about the conflict the rebase stopped on. The
  read hook now falls back to the branch being rebased (Git's
  `rebase-merge/head-name`, or `rebase-apply/` for the apply backend, in the
  worktree's own Git directory), and the runtime admits that branch as the
  inferred one only while the rebase is in progress: another branch's name
  there, or a detached HEAD with no rebase, is still refused. The capsule
  then carries the task and the conflicted passage. Turn checkpoints still
  wait for the symbolic branch. Checked in a checkout and a linked worktree;
  the generator's hook contract admits the one fallback line. Tests:
  `tests/test_memory_git_reuse.py`, `memory-bank/tests/test_hooks.py`
  (`RebaseIdentityTest`).

- A branch whose name Git takes but a task ID does not - an apostrophe
  (`fix/user's-login`), `@`, parentheses, more than 128 characters - no
  longer turns memory off. Every prompt there said `Capsule unavailable: Task
  ID must use letters, digits...` and no turn was ever recorded. The branch
  the checkout is on now gets a task ID derived from its name
  (`fix/user-s-login-b813ac24`: refused characters become `-`, the name is cut
  to fit, eight hex digits of its SHA-256 keep such branches apart), wherever
  the hooks or a person name the branch: reads, checkpoints, Cursor's rule,
  subagent dispatch and the readiness report. Any other ID the grammar
  refuses is still refused. Tests: `project-brain/tests/test_runtime.py`
  (`AnyScriptTaskIdTest`).

- Memory works on native Windows with the python.org installer. The hooks
  ran only `python3`, which that installer does not provide: Git Bash then
  finds nothing, or the Microsoft Store stub that only offers to install
  Python, so memory was neither read nor written - silently, or with the
  stub's message on every prompt. Every hook that runs Python
  (`working-memory-read.sh`, `working-memory-write.sh`,
  `context-continuity.sh`, `subagent-dispatch.sh`, `local-context.sh` and
  Cursor's rule maintenance) now finds it with one `resolve_python`
  function: on Windows it tries `python`, `py -3` and `python3` and takes the
  first that is Python 3.9+, the order the memory MCP launcher already uses;
  elsewhere a `python3` on `PATH` is taken without a probe, so the common
  case starts no extra interpreter. On Windows the name found is remembered
  per user (`$TMPDIR/ai-accelerator-python-<uid>`) and taken again while the
  shell finds it at the same path - read from bash's own lookup table, no
  process - so the probe runs once, not in every hook. With no Python at all the prompt hooks
  stay silent as before, and the session banner now says memory is off
  (`Context governance: unavailable - no Python 3.9+ found`) instead of
  nothing. The memory CLI the policy has the agent run as `python3
  memory-bank/scripts/context.py` failed there too, and under `claude -p`
  (the Harness runs it so) a `python ...` retry was refused as not allowed:
  `.claude/settings.json` now allows the CLI under `python` and `py -3` as
  well, `.cursor/cli.json` under `python`, and where Python has another name
  the session banner says which (``Python here is `py -3`, not python3``).
  Attached on Windows, the policy preamble names the runtime as `python`.
  The generator's hook contract (hook-forge,
  `validate_generated.py`) takes the resolved call and still accepts hooks
  that call `python3`. Tests: `memory-bank/tests/test_hooks.py`
  (`PythonResolutionTest`); checked live on an installed project with
  `OS=Windows_NT`, a Store-stub `python3` and the interpreter answering only
  to `python`, then only to `py -3`. On native Windows,
  `tests/test_windows_memory_hooks.py` (CI job `windows-harness`) installs
  PHP Core and runs the session, prompt and Stop hooks through Git Bash,
  with a real `python.exe` and no `python3` on `PATH`.

- A large Memory Bank no longer slows every prompt. Each prompt checks the
  chunks again, since a citation can go away while a chunk stays, and it
  also screened every unchanged chunk for secrets (two dozen patterns over
  the whole file, about 0.4 ms a chunk) and resolved every citation again
  chunk by chunk: with 2,045 chunks a prompt took 1.8 s, 1.5 s of it in that
  pass, and auto-promotion keeps adding chunks. A chunk whose text still
  hashes to what an index screened and stored under the same secret policy
  is no longer screened again - other text is, even behind an unchanged
  stat, and a changed policy rebuilds the index - and within one pass each
  cited source is looked at once; citations are still checked on every
  prompt. That prompt now takes 0.35 s, and an ordinary project's prompt
  went from 260 to 221 ms. Tests: `tests/test_memory_brain_scale.py`
  (`MemoryBankScaleTests`).

- A damaged local database no longer stops memory for good. When SQLite
  reported `memory-bank/local/context.db` damaged ("database disk image is
  malformed", "file is not a database") every prompt and every checkpoint
  failed until a person moved the file. In governed mode, where all it holds
  is derived, the hook commands now move it and its journals aside to
  `context.db.quarantine-<UTC time>`, build a new one and run once more; the
  capsule says so, the branch's task is read from the Brain at once and bound
  again at the next checkpoint, and only the turns buffered since the last
  checkpoint are lost. Lightweight mode, whose working tasks live only in
  that file, keeps the manual recovery in `docs/TROUBLESHOOTING.md`. Tests:
  `tests/test_memory_orphaned_task.py`.

- A prompt with a pasted log keeps its memory. The read hook passes the
  prompt to the CLI as one argument, and Linux refuses one over 128 KiB
  (Windows a whole command line over 32,767 characters): a prompt past that
  failed the refresh with "Argument list too long" and the turn got no
  memory. The hook now keeps the first 16,000 and the last 8,000 characters
  of a prompt over 24,000 - the question usually stands at one end - and
  the query is distilled from those as before. Test:
  `tests/test_memory_prompt_read.py`.

- Tests: `tests/test_memory_onnx.py` no longer fails when a worker process is
  reaped between the open and the read of its `/proc/<pid>/stat` (the read
  raises ESRCH); a full check hit that race once.

- Claude Code and Codex: on a checkout that names no branch - a detached
  HEAD in a rebase or a bisect - the prompt hook has no task to read and
  only keeps the index current, and the refresh's operator report ("procedural:
  updated (94 documents)", the phase timings) reached the model as noise on
  every prompt. A hook's refresh without a task and a query now prints
  nothing when every layer refreshed; a failure still prints, and `/memory`
  (`refresh` without `--host`) keeps its report. Test:
  `tests/test_memory_prompt_read.py`.

- A project with tens of thousands of documents keeps its memory. The ignore
  check asked `git check-ignore` about every candidate document, and Git
  looks each path up against the index - about a tenth of a millisecond a
  path in a 20,000-file checkout - so 20,000 Markdown files outlasted the
  check's two seconds and every refresh failed with "Git source context
  unavailable": no capsule, no knowledge, on every prompt. Above 256 paths
  the rules are now asked without the index (`--no-index`, milliseconds),
  and the tracked files among the paths they name - which Git never counts
  as ignored - are taken out with `ls-files`, only when there are any: one
  call for the directories they lie in when they are many (generated pages
  under an ignored folder), not a call per batch of paths.
  The answer is the same; on that project a prompt's refresh takes 0.44 s
  warm and 2.0 s cold. Tests: `tests/test_memory_git_reuse.py`
  (`ManyDocumentsIgnoreTests`).

- A long document no longer holds the prompt. FTS5 marked the whole of a
  delivered document - about a third of a second a megabyte - and a prompt
  marked each delivered item several times (its identity, its excerpt, the
  render), so a question that matched a 20 MB reference under `docs/` held
  the prompt hook for minutes: past its five seconds the turn got no memory
  at all, and a 2 MB document took about 7 s. A document longer than 65,536
  characters is now marked only in its answering stretch - the section, from its
  heading, where most distinct words of the query meet, or within a longer
  section the window where they do - and one command marks a document for a
  query once. The document is still matched and ranked over all of its text.
  Such a prompt now takes about 1.6 s with the 20 MB document and 0.3 s with
  the 2 MB one. Tests: `tests/test_memory_brain_scale.py`
  (`LongDocumentTests`).

- A branch named in any script gets memory. The task ID is the branch name,
  and it had to be ASCII: on `feature/счёт-экспорт` every prompt said
  "Capsule unavailable: Task ID must use letters, digits, '.', '_', '/', or
  '-'" and delivered nothing, and no turn was recorded; so did
  `fix/#123-crash` and `feature/c++-parser`. Task IDs now take letters and
  digits of any script (and combining marks) with `.`, `_`, `/`, `#`, `+`
  and `-`, in the runtime, the memory MCP server, the Cursor rule's receipt
  and the Harness; whitespace and other punctuation stay refused. Test:
  `PHP Core/project-brain/tests/test_runtime.py` (`AnyScriptTaskIdTest`).

- Two task records for one branch no longer stop that branch's memory.
  Two copies of a branch that each started its task - two people, or one
  person on two machines - leave two active records with one task ID once
  they merge. A clone without a binding for it failed every checkpoint as
  ambiguous, which the Stop hook hides, and its prompts showed the task as
  not recorded. Now a machine without a binding reads, and binds to at its
  next checkpoint, the earliest-created active record, so every clone settles
  on the same one; the capsule names the other record on every prompt with
  the command that keeps one (`brain-update --record-id <other> --revision
  auto --transition cancelled`); and a machine whose bound record was
  cancelled that way follows the task ID to the record that is left instead
  of dropping its turns as work on a closed task. A finished task with no
  active record beside it keeps its binding, as before. A fresh clone or a
  second machine also reads the branch's existing task from its first prompt
  and in Cursor's session rule (`hook-context`), where it showed "working:
  not recorded yet" until its first checkpoint; reading binds nothing.
  The index pass notes such task IDs (`task-twins` in the index state), so
  the check costs no extra pass over the Brain. `docs/TROUBLESHOOTING.md`
  has the steps. Tests: `PHP Core/project-brain/tests/test_runtime.py`
  (`MultiMachineContinuityTest`).

- After a merge of two branches that both added Project Brain records,
  `project-brain/indexes/active.json` holds conflict markers or one side's
  list. `validate` said "/abs/path/project-brain/indexes: Expecting value:
  line 1 column 1" and `compact` refused, claiming that "active records are
  invalid". Validation now names the index file by its repository path, says
  what is wrong with it and that the next Brain write rebuilds it, and
  compaction - which rebuilds the indexes - goes ahead when they are the only
  problem; an invalid record still stops it. Test:
  `PHP Core/project-brain/tests/test_runtime.py`.

- The automatic checkpoint names when its first turn began to the minute
  ("Auto-checkpoint: 5 turn(s) since 2026-10-15 20:02 UTC"), not as
  `2026-10-15T20:02:04.002114+00:00`: the capsule repeats the line on every
  prompt of the task, and the seconds, microseconds and offset told a reader
  nothing more.

- Harness: a capsule that does not match the session's linked task now says
  why when the runtime said it - "Retrieved context does not match the
  linked task and revision" carries the capsule's warnings, such as the task
  record that cannot be read. Test: `tests/test_harness_task_context.py`.

- A prompt no longer rewrites the index when the Project Brain did not
  change. The Brain rows are rebuilt from the records on every refresh, and
  every refresh used to delete and insert all of them again and stamp a new
  index generation, which also threw away the token frequencies cached
  against it; now a digest of the rows is stored with them and an unchanged
  set leaves the tables and the generation as they are. Within one pass
  over the records each cited file is looked up once, however many records
  cite it. A document's key is cut from the path string instead of by
  `pathlib` (half of the source walk), and `workspace_roots` no longer
  imports `dataclasses`, which brought `inspect`, `ast` and `dis` into every
  runtime start for one class. On a 524-record Brain the index phase of a
  prompt went from about 67 to 55 ms, and the prompt hook, measured
  interleaved over 31 prompts each, from 257 to 239 ms on Python 3.14 and
  from 301 to 270 ms on 3.9. Tests: `tests/test_memory_brain_scale.py`
  (`BrainIndexStabilityTests`, `SourceKeyTests`).

- Cursor: a `.cursor` or `.cursor/rules` that is a symbolic link - a rules
  folder shared between projects - is still never written through, since the
  capsule would reach every project sharing it, but the hooks now say so:
  "Cursor memory rule was not updated: linked_directory." instead of the
  generic `custom_or_unavailable`, and `cursor-rule` reports the reason in its
  JSON. Keep `.cursor/rules` a directory and link shared rule files inside it.

- Cursor: a rule written by the releases of 2026-08-03..05 - frontmatter
  "Task Capsule as of end of previous turn" or "warming up, no Task Capsule
  rendered yet" - is generated output, not a person's rule: it is copied to
  `memory-bank/local/cursor-rule-edits/` like an edited rule and the slot is
  written again. Before, such a file stayed in the slot after an upgrade and
  Cursor sent its August capsule with every request.

- Claude Code and Codex: a refresh stopped at its budget now adds "If this
  repeats on every prompt, build the index once outside the hook: python3
  memory-bank/scripts/context.py index" in an installed project. A first
  index of a large documentation tree (about 19 MB of Markdown on a loaded
  machine) does not fit a prompt's five seconds, and an index stopped there
  starts over on the next prompt.

- Cursor: a prompt whose capsule selected nothing no longer becomes the
  topic a "yes", "да" or "go on" continues. A short reply ("thanks, looks
  good") took the topic's place, and every whitelisted continuation after it
  searched by that reply and delivered nothing. Such a prompt - a reply or a
  question nothing matched - now clears the topic without reviving the one
  before it, and the next prompt that selects something sets it again. Test:
  `tests/test_cursor_continuation_state.py`.

- Cursor: a `safe.directory` in `$XDG_CONFIG_HOME/git/config` trusts the
  project for the rule's custody as it does for Git. The trust check read
  `git config --global`, which reads only `~/.gitconfig` when that file
  exists; it now reads every file with its scope and keeps the system and
  global values, so the rule was neither written nor retired on such setups.

- A working task whose record is unreadable (bytes that are not UTF-8, say)
  keeps its binding: the lookup now says "invalid" - the file's name is the
  record's UUID - instead of "not found", which would have let the next
  checkpoint record a duplicate beside it. The read serves the turn's other
  memory with a warning that names the record by its repository-relative path
  and cause, and the capsule says "working: unavailable (the task's record
  cannot be read)" rather than "not recorded yet"; an invalid record cost the
  turn its whole capsule before. An invalid record's absolute path no longer
  reaches prompts or messages.
- `check.py` drops Git's repository selectors (`GIT_DIR`, `GIT_INDEX_FILE`,
  `GIT_WORK_TREE`, ...) from the commands it runs: from a pre-commit hook they
  name the commit being made, and a test's `git add` in its own temporary
  repository wrote into that index. The Cursor rule tests prepare their
  fixtures in a clean environment too.

- CI: the memory runtime's regression steps left the `installation` job for a
  `memory` job of their own, which runs beside it on Python 3.9 - the floor the
  runtime is declared for, where no memory test ran before - and on the
  current release. `installation` had reached about 1000 s of its 20 minutes.
  The memory job also runs eight test modules no workflow ran: the Harness
  capsule economy, cost observation, memory metadata, native diagnostics,
  reviewed and unattended capsule JSON and SDD modules, and promotion
  admission. All 1322 tests of the job pass on Python 3.9.

- Windows: the Brain refuses only reparse points that redirect a path - a
  symbolic link or a junction, or one whose tag cannot be read. A OneDrive
  placeholder or a deduplicated file is the file itself with its storage
  elsewhere; any reparse attribute used to make the whole Brain unavailable on
  such volumes. Save evidence (receipts, attempts, learning images, the save
  frontier) follows the same rule; the Cursor rule's custody stays strict.

- A result saved before the runtime keyed everything by task UUID replays
  after an upgrade instead of being refused for good (`receipt_unavailable`).
  Those local receipts named the task as the caller did, usually by its
  branch alias; they sit under the same UUID key with the same digest, so the
  task's current alias is accepted there and the replay rewrites the receipt
  with the UUID.

- A project under a directory whose name is not UTF-8 (a Latin-1 home or
  share) keeps its memory. Brain paths are checked from `project-brain` down
  - they become record keys and index entries - and handoffs check the paths
  below the repository they store; the machine's own directories above the
  project are never stored and are no longer checked. The Git frame is read
  with the filesystem's own codec. Before, one such directory name switched
  the Brain and handoffs off for every command and the prompt hook answered
  "Git source context unavailable". A handoff transcript's file name, which
  is never stored, is no longer refused either.

- A working task whose uncommitted record `git stash -u` or `git clean` swept
  away comes back. Every later Stop failed with "record not found" while
  `status` reported memory as active, and the prompt hook delivered nothing at
  all. The read now serves the turn as for a task not recorded yet - knowledge
  included - with a warning naming the gone record, and the next checkpoint
  drops the dead binding and records the task again with the turns buffered
  meanwhile. A record that is only invalid keeps its binding and its refusal.

- Git starts per prompt: 12 became 6 on a prompt that delivers a project
  document (5 when it delivers none). The inferred branch is asked once; the
  two checks before delivery compare the checkout's HEAD - and the reftable
  list - with the snapshot taken before that answer and ask Git again only
  when either is a new file. The first ignore check of a command still
  confirms its frame with Git before and after its query; later checks in the
  same command reuse that frame while every file that decides it (the
  directories from the project to its top, each `.git` marker, the admin
  directory, its config and commondir) keeps its identity, and confirm it
  with Git again when one moved. A branch switched mid-refresh and a
  repository retargeted during a reused check are refused as before. On
  Windows, where each Git start costs tens of milliseconds, this is most of a
  prompt's Git time.

- Cursor: the Stop hook no longer prints "Cursor memory rule was not updated:
  custom_or_unavailable." on every turn that replaces a session or turn rule.
  Its check for a held prompt rule is a question - "preserved" there means
  another rule is in place for the write that follows - and the write, keep
  or retire after it reports the rule's real outcome. TOOL-INTEGRATIONS gives
  the maintenance budget the hooks use, three seconds, not one.

### Memory repairs and resumable saves — 2026-10-15

- Brain reads at scale: a command reads each Project Brain record once. The
  runtime keeps every valid record's image for the rest of the process, keyed
  by the file's identity (device, inode, size, modification and change
  times), and hands each pass its own copy; the save frontier's file hashes
  and the Memory Bank's secret scan are remembered the same way, and listing
  works on strings rather than Path objects. A `record-result` with two new
  learnings on a 500-record Brain went from about 820 to 300 ms - it walked
  every record thirty times. A directory a writer changes while it is being
  listed is listed again (up to six times) instead of failing the reader with
  "Project Brain inventory unavailable", and a writer's temporary file that
  disappears before it is looked at is no longer an error.

- Memory saves: a save interrupted at any point and repeated with the same
  `result_id` now finishes. It resumes at an unchanged frontier, or runs
  again from the start with every effect idempotent against the current
  records - a learning is created only when absent, finished only when it is
  exactly this save's open record, revised only under the usual proofs, and
  the task takes only progress it does not already hold. Harness
  `replay-result` finishes a started attempt the same way; without one a
  replay is still never a first write. `attempt_incomplete` is left for stage
  records that disagree with the records. MCP texts, `MCP.md`,
  `MEMORY-INTEGRATION.md` and policy-forge say so.
- Save evidence in the shared Brain: `control/result-attempts/<task UUID>/`
  holds one file per task, result ID and request content, so a teammate's
  clone that reuses a result ID for other content makes its own save instead
  of being refused, and two clones' files never meet in a merge. A finished
  save keeps a proof under 400 bytes (the whole attempt was kept, 3.5 times
  the size), the newest 64 per task; compaction removes an archived task's
  folder, the `control/result-learning-images/` of archived learnings and
  files of the earlier flat layout. MCP names the `result_id_reused` and
  `stale_task_revision` refusals. Both folders are documented in the Brain
  README, `MCP.md`, `MEMORY-INTEGRATION.md` and the generator's runtime
  contract.
- A Cursor rule a person edited is preserved: before the rule is regenerated
  or retired, an edited generation is copied to
  `memory-bank/local/cursor-rule-edits/` (newest 20), and temporary files an
  interrupted write left are swept.
- Prompt hook cost on a 500-record Brain is back to the evening level (221 ms
  against 225 ms; it was 23% slower). The Brain is read once per pass -
  `iter_record_images` hands the index the very image it validated, and the
  archive test no longer walks `path.parents`; the metadata Unicode check
  runs only for frontmatter with a `\u` escape, as one `json.dumps` instead
  of a Python walk; the hooks run the new `memory-bank/scripts/context_cli.py`,
  which runs `context.py` from cached bytecode (run as a script it was
  compiled again on every prompt). `context.py` names itself in usage text.
- The Harness runs the installed runtime through `context_cli.py` too,
  falling back to `context.py` in a runtime without it: each message makes
  several runtime calls (rebind, refresh, Cursor's checkpoint and rule
  retirement), and each compiled `context.py` again. With the runtime cached
  (next entry), a refresh on a 500-record Brain, with the Harness's arguments
  and environment, takes 226 ms instead of 258 ms. Behind the process guard,
  which looked for the runtime's exit every 50 ms, one call gained 0 or 50 ms;
  the guard now waits for the exit itself (see the process guard entry below).
  Infrastructure-Creator's generated hooks take the same entry (see its
  changelog).
- `context_cli.py` writes the runtime's bytecode cache even when the caller
  asks Python for none. Claude Desktop starts Claude Code with
  `PYTHONDONTWRITEBYTECODE=1`, which every hook inherits, and the Harness sets
  it on each runtime call, so neither ever wrote the cache and the whole
  runtime was compiled again on every call. A path hook now gives the
  loader's own directory Python's finder with a loader that caches; every
  other import keeps the caller's setting. `memory-bank/scripts/__pycache__/`
  is ignored in installed projects (the editions' `.gitignore`), in the clone
  the Harness attaches and in generated projects (memory-seed's root-ignore
  requirement). Under `PYTHONDONTWRITEBYTECODE=1` on a 500-record Brain the
  Claude prompt hook takes 305 ms instead of 429 ms from the second call on
  (faster in 60 of 60 paired runs; the first call, which writes the cache,
  costs what every call did) - as much as where bytecode is allowed, where
  both loaders take 304-306 ms - and a Harness refresh through
  `KnowledgeManager.run` 300 ms instead of 422 ms (40 of 40). A runtime
  without the loader, which the Harness calls directly, still writes nothing.
  `test_context.py` and the Harness loader test check that the cache appears,
  in the runtime's directory only.

### Harness process guard — 2026-10-15/16

- The Harness process guard (POSIX) waits for its native process's exit as an
  event instead of looking every 50 ms: a pidfd in the selector that already
  holds the owner's watchdog on Linux 5.3+, elsewhere (macOS, older kernels,
  a sandbox that refuses pidfds) SIGCHLD through Python's signal wakeup pipe,
  which carries SIGTERM and SIGINT too - a handled signal alone resumes a
  blocking select. The owner reads EOF only when the guard exits, so every
  guarded call (each memory-runtime call of a message, guarded Git, commands,
  skills) waited up to 50 ms after the native process had finished. A short
  call's EOF now reaches the owner 3.5 ms after the native exit instead of
  48 ms (what is left is the guard interpreter's own exit), and the whole call
  takes 26 ms instead of 70 ms. A refresh on a 500-record Brain through
  `KnowledgeManager.run` costs 23 ms over the same call without a guard
  instead of 44 ms, 21 ms less per call (paired medians of 60 interleaved
  runs), and no longer lands on the 50 ms grid. Owner loss and cancel keep
  SIGTERM to the group, `TERMINATION_GRACE`, then SIGKILL, and still never
  return while a stubborn descendant may live in the group; they end at the
  deadline, not at the next tick. Exit statuses (128+N for a signal) and the
  Windows Job Object path are unchanged. `test_harness_process_guard` counts
  the guard's wakeups while its child sleeps (8 in 0.4 s before, none now)
  and runs the exit, signal-status and stop tests with and without a pidfd.
- The Harness process guard (POSIX) also starts and exits faster. Every
  guarded call waits for the guard's interpreter twice: before its native
  process starts, and after it ends, since the owner reads EOF only once the
  guard is gone. `process_runtime.launch_guarded` starts the guard with `-S`
  (it needs only the standard library), the guard imports `ctypes` only in
  its Windows functions, and once its native process has ended it flushes
  stdout/stderr and leaves through `os._exit` instead of interpreter
  finalization. Paired medians of 120 interleaved runs on Python 3.14: `-S`
  saves 1.1 ms a call, `ctypes` 2.0 ms, `os._exit` 2.9 ms (EOF reaches the
  owner 0.9 ms after the native exit instead of 4.1 ms). A short guarded call
  takes 22.1 ms instead of 27.9 ms (6.4 ms without a guard), on Python 3.9
  20.2 instead of 24.5 ms. A refresh on a 500-record Brain through
  `KnowledgeManager.run` costs 15.9 ms over the same call without a guard
  instead of 21.5 ms (100 runs). `-I` was left out: it ignores
  `PYTHONCOERCECLOCALE`, so under the C locale the guard would hand its native
  process an `LC_CTYPE` the owner did not give it. The Windows launch and
  exit, waiting, stopping and exit statuses are unchanged. The guard tests
  start it with `-S` as the Harness does; new tests check that a start
  failure still exits 127 with its one line, that the POSIX guard imports no
  `ctypes`, and that a `sitecustomize` on `PYTHONPATH` runs in the native
  process but not in the guard.
- The Harness process guard (POSIX) starts its native process with
  `os.posix_spawn` and waits with `select.poll` instead of importing
  `subprocess` and `selectors`, which with what they pull in (`threading`,
  `collections`, `contextlib`, `warnings`, and `locale` on Python 3.14) were
  about a third of the guard's start; every guarded call waits for it. The
  native process gets what `Popen(close_fds=True, pass_fds=(lock,))` gave it:
  stdin, stdout, stderr and the runner lock and nothing else (posix_spawn
  closes only close-on-exec descriptors, so the guard marks every other one
  it holds, listed from `/proc/self/fd`, on macOS `/dev/fd`); SIGPIPE and
  SIGXFSZ, which Python ignores, at default; and the executable Popen would
  find on PATH - past an entry it cannot run and never through a shell,
  where `posix_spawnp` gives up on glibc and runs `/bin/sh` on macOS. A
  `stat` skips an absent PATH entry before a spawn is tried (each refused
  spawn costs ~0.05-0.1 ms; 0.6 ms a call for the 12 entries ahead of
  `/usr/bin` on the test machine). A start failure still exits 127 with its
  one line. Where posix_spawn might not report a failed exec - anything but
  macOS and Linux with glibc 2.24+, as `subprocess` itself decides - or
  `/proc` is not mounted, Popen still starts it. Waiting, stopping, exit
  statuses, `-S`, `os._exit` and the Windows path are unchanged. Paired
  medians of 150 interleaved runs on Python 3.14: a short guarded call takes
  17.3 ms instead of 22.2 ms (6.3 ms without a guard), 17.7 instead of 22.5
  when its executable is looked up on PATH (120 runs); on Python 3.9 16.1
  instead of 19.4 ms (120 runs). A refresh on a 500-record Brain through
  `KnowledgeManager.run` costs 11.0 ms over the same call without a guard
  instead of 15.9 ms (100 runs; on Python 3.9 9.6 instead of 11.8, 80 runs).
  New tests compare the guard with Popen on PATH lookups and their failures,
  and check that the native process holds only descriptors 0-2 and the lock
  (also one the guard got non-inheritable), starts with SIGPIPE and SIGXFSZ
  at default, and that a whole POSIX run imports none of `subprocess`,
  `selectors`, `threading` and `ctypes`. The start-failure, PATH, descriptor
  and signal tests run both through posix_spawn and through the Popen
  fallback.
- The Harness process guard (POSIX) starts in half the time.
  `process_runtime.launch_guarded` starts it through the new
  `harness/src/harness/process_guard_cli.py`, which runs `process_guard.py`
  from cached bytecode: run as a script, the guard, its Windows code
  included, was compiled again on every guarded call. As `context_cli.py`
  does for the memory runtime, the launcher writes that cache even under
  `PYTHONDONTWRITEBYTECODE=1`, which the Harness sets for its runtime, setup
  and result calls - for `process_guard.py` only; what the guard imports
  keeps the caller's setting (`harness/.gitignore` ignores `__pycache__/`).
  The guard imports `_signal` instead of `signal`, whose IntEnum wrappers
  brought in `enum`; `posix` instead of `os`, whose `os.py` brought in `abc`,
  `_collections_abc` and `posixpath` (PATH is searched in bytes, as Popen
  searches it); and no `__future__` (annotations Python 3.9 cannot evaluate
  are strings). The launcher takes `SourceFileLoader` from the importlib
  module Python loaded at startup: importing `importlib` cost 0.1-0.5 ms,
  and `from importlib.machinery import ...` alone imported `warnings` on
  Python 3.14, through that module's `__getattr__`. Beyond the interpreter's
  own start the guard now imports `stat` and `select`. Paired medians of 200
  interleaved runs on Python 3.14: a short guarded call takes 10.8 ms
  instead of 15.5 ms (5.6 ms without a guard; the guard's own start 4.8 ms
  instead of 9.5), 11.0 instead of 15.7 when its executable is looked up on
  PATH (150 runs); on Python 3.9 10.9 instead of 14.4 ms (150 runs). Each
  change taken back alone (300 runs on Python 3.14): the cache is worth
  2.1 ms, `signal` 1.7, `os` 1.3, `importlib` 0.1, `__future__` under 0.1.
  A refresh on a 500-record Brain through `KnowledgeManager.run` costs
  5.7 ms over the same call without a guard instead of 11.2 ms (100 runs;
  on Python 3.9 4.7 instead of 8.8, 80 runs).
  Windows still starts `process_guard.py` itself, and its code is
  unchanged, as are waiting, stopping, exit statuses, `-S`, `os._exit` and
  what posix_spawn or Popen hands the native process. New tests check that
  the launcher caches the guard under `PYTHONDONTWRITEBYTECODE` and nothing
  else, and runs it from the cache (after an edit of the same size and
  modification time it still prints the line it was compiled with), that
  `launch_guarded` starts it that way, that the guard imports none of `os`,
  `signal`, `enum`, `importlib`, `warnings` and `__future__`, and that a
  non-ASCII executable in a non-ASCII PATH entry is found as Popen finds
  it. The guard tests start it through the launcher, as the Harness does,
  and the in-process import test counts only what the guard adds: on
  Python 3.11 `runpy` itself imports `threading`, which failed it before.
- The Harness process guard starts from cached bytecode on Windows too:
  `process_runtime` starts `process_guard_cli.py` there as well, instead of
  `process_guard.py`, which run as a script was compiled again on every
  guarded call. The launcher now finds the guard beside itself whichever
  slash ends its folder: Windows hands it a path with backslashes, and the
  launcher, which does without `os.path`, looked only for a slash, so it
  would have looked for `process_guard.py` in the working directory.
  Through the launcher the guard is the same script - `__main__`, the same
  argv, `sys.argv[0]` naming `process_guard.py`, its folder first on
  `sys.path` (where the Windows guard imports `windows_commands`) - and its
  `sys.exit` still ends the interpreter normally, so the final close of the
  Job Object handle still ends the tree. The guard's Windows code, the
  duplicated watchdog and lock handles in the `STARTUPINFO` handle list,
  `CREATE_NEW_PROCESS_GROUP` and the start without `-S` are unchanged;
  under `PYTHONDONTWRITEBYTECODE` only the guard's own cache is written, as
  on POSIX. Not measured on Windows (no machine at hand). In the Windows
  launch shape on Linux - no `-S`, the same failing start for both - paired
  medians of 300 interleaved runs: the launcher saves 1.85 ms a call on
  Python 3.14, 1.81 on 3.12, 1.43 on 3.11 and 1.33 on 3.9 (within 0.1 ms
  under `PYTHONDONTWRITEBYTECODE=1`; faster in 297-300 of 300 runs).
  Compiling the guard takes 2.4 ms on 3.14 (2.0 on 3.12, 1.5 on 3.9), the
  launcher 0.2 ms, reading the guard from its cache 0.1 ms. These figures
  leave out what Windows file access adds: one more stat (the guard) and
  one more open (its cache) per call. Where the cache cannot be written - a
  read-only folder; the Harness's own clone, which it updates in place, is
  writable - a call costs 0.7-1.1 ms more than before, on POSIX too: the
  guard is still compiled, now through `compile()`, whose first call in a
  process builds the AST node types (~0.6 ms; a script run skips that).
  `test_guard_runs_from_cached_bytecode_the_caller_did_not_ask_for` now
  runs in the windows-harness job too (exactly one cache file, the guard's,
  not `windows_commands`), and a new test on every platform checks that the
  launcher names the guard beside it for a `__file__` whose folders end
  with backslashes, slashes or both; the previous launcher fails it.

### Fixes after the third memory review — 2026-10-15

- Harness Hindsight/LongMemEval: the export screen refuses camel and kebab
  credential keys, `token`, `pass`, `pwd`, `passphrase`, `salt`, `APP_KEY`,
  `encryption_key`, unquoted `.env`/YAML values, literal `env()` and
  `os.getenv` defaults and SQL `IDENTIFIED BY`/`PASSWORD '...'`; the forward
  scanner decodes `\u{...}` code points, keeps an unknown `\u` literal in
  PHP, Markdown, text and SQL sources and never pairs a quote across a line
  break. The LongMemEval projection rewinds its directory descriptor before
  each scan: btrfs returned the previous scan's list and the instance fenced
  itself with `source_changed`. Recall counts `socket.timeout` as a timeout on
  Python 3.9, and the journal test removes its SQLite authorizer the way
  Python before 3.11 needs.
- Tests and CI: `test_memory_result_evidence` and `test_hindsight_export_gaps`
  run in CI and `check.py`; `test_memory_optional_publication` no longer
  leaves its temporary runtime modules in `sys.modules`, which failed 39 tests
  of the Windows harness job. The root `tasks/` (agent journals with absolute
  paths) is ignored, and the installer inventories carry `context_cli.py`.

- Remove the Claude/Codex prompt supervisor from the read hooks; they call
  `context.py refresh` directly again under the one `CONTEXT_HOOK_BUDGET`.
  The supervisor refused any budget above 5 seconds and any prompt above
  8 KB, so a pasted log or a raised budget left the turn with no memory at
  all, not even working state; it hid the runtime's own error behind its
  JSON report, gave the refresh only 70% of the budget and cost an
  interpreter start per prompt, while its read-only fallback rarely
  delivered. The hook now drops a capsule cut off at the budget (status 124)
  instead. `workspace_roots.py prompt-supervise`, `context.py --prompt-wire`,
  their tests and the generator's supervised profile go with it; generated
  hooks that still call the supervisor are rejected.
- An ignored file a record cites no longer withholds the record or the task's
  working state. Ignore rules still decide what is indexed and served from
  the cache (a document's own path); a task citing `vendor/...` lost its
  progress and next steps for good, and knowledge citing the accelerator's
  own ignored specs or skills left every capsule.
- One unreadable or schema-invalid Project Brain record is skipped, as before
  the overnight change, instead of stopping memory for every task, `get` of
  unrelated tasks and `validate`'s own report. `validate` names each skipped
  record and its cause; a lookup whose identifier only an invalid record
  claims refuses by name (or as ambiguous) rather than reporting absence, so
  no duplicate is created beside it.
- Git source checks: a GIT_DIR, GIT_INDEX_FILE or discovery variable in the
  environment is dropped from the probe instead of disabling memory; a `.git`
  directory without HEAD, objects and refs (an empty `~/.git` or `/tmp/.git`)
  is skipped as Git itself skips it; the inferred-branch check is one Git
  start instead of three, so a prompt starts 7 Git processes rather than 13,
  and the probes allow 2 seconds instead of 0.1-1.
- Refresh waits for SQLite write admission (2 s, or a quarter of the hook
  budget under a hook) and then for busy writers (5 s, or half the budget)
  instead of 50 ms for the whole refresh, which lost a parallel session's
  knowledge in a third of turns.
- Delivery: a Memory Bank chunk citing a directory (`src/Billing`) is no
  longer withheld as source-missing; `retrieve`/`context` answer with the
  knowledge on a branch the first checkpoint has not provisioned yet; the
  degraded read-only refresh keeps working state for an edited or ignored
  citation, as the full refresh does. Repeated record validation and
  frontmatter text checks are remembered per exact file text within a run.
- Record results: an `expected_revision` on a learning that does not exist yet
  creates it instead of holding it as a "kept" conflict; a person's attested
  correction of a closed learning applies; an agent may revise its own
  learning recorded before learning images were kept; an open learning that
  an interrupted save created with exactly this content is finished by the
  next result instead of staying open and unpromoted. MCP errors name the
  typed result code and ask for current, not original, revisions. File-image
  checks compare identity fields without st_atime, which reading the file
  itself moves.
- Cursor: a generated rule is recognized as registered when Git ignores the
  slot through `.git/info/exclude` or a global excludes file, not only a
  root `.gitignore` line; retire never unlinks `cursor-rule.json` relative to
  the process directory and no longer reports a false incomplete; an
  unreadable receipt with no rule no longer blocks the next write; short
  acknowledgements ("sure", "go ahead", "давай") keep the rule's topic.
- `install_accelerator.py --sync` identifies a PHP Core installation (its file
  names tie with Laravel and Symfony; the larger share of the edition's own
  inventory now decides), so the update path - and the Harness's own sync on
  opening a project - reaches PHP Core projects. A project holding only a copy
  of the memory runtime, with no tool's files, is not an installation and is
  no longer filled with one.
- Tests: fixtures updated to the image-carrying capsule contract and to the
  receipts-after-publication order; a fixed 2026-11-13 review date that would
  have failed CI from 14 November moved to 2099; imported temporary runtimes
  leave `sys.modules`; the stale Hindsight case for unescaped JSON strings is
  dropped (the forward screen no longer decodes them).

### Memory review remediation — 2026-10-14/15

- Aggregate allowlisted omitted-source notice occurrences in health JSON/text after windowing, with explicit journal-record counts and no source identities; preserve legacy output when notices are absent.

- Add fresh actual CLI/MCP save-to-promotion-to-recall integration with complete chunk/digest/INDEX oracles, source presence lifecycle, ordinary decode isolation, private request refusal and byte-identical Claude/Codex hook delivery. Keep provider/token claims unmeasured.

- Request Cursor refresh JSON after the prompt-query conditional, including empty/missing/non-string inputs, so native rule publication cannot silently keep a same-task stale rule after parsing operator text. Remove the generator native-profile exception; preserve Claude/Codex text output and native custom/link custody.

- Refuse malformed Brain inventory paths before lookup/yield, direct reads and held-text parsing; pre-admit managed handoff paths outside per-file catches. Guard portable handoff paths before affected reads, writes and projection. Add fresh preservation, no-false-absence and complete Unicode roundtrip controls.

- Reject malformed UTF-8 policy keys in direct/supplied fingerprints and light/full/cross-edition parity. Validate reverse inventories before partial-install shortcuts; preserve normal Unicode digests, drift shapes, ignored files and valid partial installs. Add fresh fail-closed and actual CLI controls.

- Isolate ordinary invalid UTF-8 document contents and path keys before Git/SQLite; retain healthy Unicode neighbors and report count-only, nonfatal source notices through refresh, capsules and health records. Keep malformed skill policy paths strict. Turn Memory Bank INDEX/chunk decode errors into fixed diagnostics, including second-read races, and keep summaries available.

- Keep rich memory rendering, section identities and window proposals on admitted indexed bodies through packing, printing and refresh. Carry immutable images outside public JSON, including lightweight packets; detached capsules use admitted snippets. A moved cache is highlighted through isolated in-memory FTS over held bytes, and shared sinks match first-row dedup ownership.

- Keep verified existing cost bookings idempotent after unit closure and before final freeze. New reservations remain refused; malformed stored tickets cannot be repaired by replay. Add independent supplied-observation arithmetic and concurrency controls.

- Bind candidate policy images to the same SQL row as their highlighted/plain excerpt. Missing or changed row identities cannot be rebound by a later cache read; path/hint/source-graph excerpts reuse their already-admitted body. Add fresh native policy-refresh race and supplied channel controls.

- Reapply the current secret-recognition policy to full indexed images and returned paths at cached search and governed delivery, including after packing. Preserve safe masked/template records and the five-field raw-search contract; retain the source-hint byte cap before image hydration.

- Recheck declared Memory Bank expiry in cached FTS search and governed delivery after packing; boundary-day knowledge remains valid, expired chunks leave manifests and receipt identities without waiting for refresh.

### Final memory source-image custody — 2026-10-15

- Recheck delivered ordinary/Brain content and full local citation existence
  after packing, with fresh privacy/owner/authority configuration. Preserve
  initial citation-drift warnings and original scores; withhold newly changed
  or restored drift sets rather than expand already packed text.
- Revalidate full semantic working-record images and withdraw dependent
  queries/views when tasks change or disappear. Read final working records
  through admitted regular descriptors; POSIX FIFO replacement cannot block
  open. Preserve active cancellation through single-attempt cleanup failures.
- Recompute receipts after ordinary packing omissions. Add fresh Wren/Lark/
  Swan controls with native temporary CLI setup, assembly/proof checks and
  supplied dual-faults. This is point-in-time custody, not atomic filesystem
  stability, complete eligibility rescreening, client or provider validation.

### Cached memory source admission — 2026-10-15

- Recheck current project Git ignore policy for cached CLI/Harness search,
  governed candidates and all declared local citations, including legacy
  chunks without digests. Preserve lexical Git names, attached roots,
  protected configuration and ordinary accelerator document exemptions.
- Batch citation/hint admission and recheck selected views after packing.
  Check full working citations before display clipping; withdraw dependent
  automatic queries/history and all packed episode views when sources close.
  Unknown Git policy remains a typed failure, including read-only refresh.
- Add fresh CLI, MCP, SQLite, global-ignore, symlink, working and interruption
  boundary controls. Local bytes/receipts are not measured provider savings;
  real Windows, live clients, remote CI and semantic benchmarks remain unrun.

### Generated memory task identity — 2026-10-15

- Specify inferred-only branch markers in generated prompt, Cursor turn and
  session readers. Explicit tasks remain unmarked. Add a manifest-owned
  canonical fragment lint for identity, relay and both renderer call arms;
  reject missing guards, data-only spoofs and unsupported array rewrites.
- Reject delimiter-prefix, arithmetic/data, Unicode-comment, quote-adjacency
  and selector-mutation spoofing; preserve literal-data continuations and
  require Cursor JSON before dispatch. Admit the finite installed hook profile
  without relaxing inference guards.
- Add fresh structural and trusted argv-recorder fixtures. Structural lint
  does not execute generated hooks or prove arbitrary shell control flow,
  actual AI generation, live delivery or whole-hook deadlines.

### Portable memory CLI streams — 2026-10-15

- Configure standard stdout/stderr wrappers for UTF-8 and untranslated LF,
  including diagnostic paths. Preserve explicit CR characters and buffered
  bytes written before startup. Custom streams retain the two-keyword API;
  ordinary configuration errors are best effort and cancellation propagates.
- Add fresh exact-byte stream and actual CLI diagnostic controls to local
  checks and both CI platform mappings. Supplied CRLF profiles on Linux do
  not establish a native Windows or live-client pass.

### Inferred task branch identity — 2026-10-15

- Preserve whether automatic read hooks inferred their task from a branch or
  received an explicit task ID. Mark inferred prompt and Cursor session/Stop
  reads with a private expected-branch value; explicit cross-branch recall
  remains supported, including requests without a committed HEAD.
- Revalidate the protected full symbolic branch before state work, after
  packing/schema work before manifest and delivery receipt publication, and
  before output. Read-only delivery checks after rendering; branch failures
  stay explicit rather than becoming warnings or valid no-context outcomes.
- Add fresh native Claude/Codex branch-movement reproductions, positive saved
  recall and callback/transport controls. Checks are point observations:
  write attribution, atomic branch epochs, downstream Cursor publication and
  whole-hook deadline guarantees remain separate work.

### Protected project source-ignore context — 2026-10-15

- Reject inherited repository/index/object/discovery selectors in source-ignore
  checks and read entry points. Strip Git command-environment overrides while
  preserving ordinary global exclusions and Git's protected trust policy.
- Observe the actual project top, prefix and Git directory, pin the ignore
  query to that frame and rediscover afterward. Unknown markers, failed probes
  and malformed responses refuse delivery instead of becoming empty ignores.
- Apply one deadline across each operation and propagate source-policy failure
  through refresh. Early read admission precedes state creation; ordinary
  accelerator exemptions remain separate from working-source citations.
- Add fresh Git, transport and real CLI/Claude/Codex hook controls. These are
  local finite profiles; no atomic filesystem snapshot, general branch
  authority, hard capture-memory bound or whole-hook deadline is established.

### Strict optional runtime configuration — 2026-10-15

- Distinguish ordinary missing configuration from an unobservable or invalid
  entry. Directories, special files, dangling links, failed reads and invalid
  UTF-8 no longer enable defaults or produce an `absent` fingerprint.
- Share strict configuration text observation between the loader and index
  fingerprint. Preserve valid dictionary overlays, readable links and exact
  newline hashes; close the owned descriptor once on errors and cancellation.
  Resolve known Windows link/reparse targets before accepting parent absence.
- Add fresh full-value and fingerprint controls plus real governed CLI/hook
  refusal checks. POSIX FIFO replacement is guarded; native Windows,
  cross-call snapshots and general cold-refresh deadlines remain unproved.

### Assigned credential prefix search — 2026-10-15

- Avoid repeatedly searching underscore-prefix suffixes for the exact known
  assigned-credential pattern. Preserve the full key grammar, original value
  validation, consumed spans, masking passes and policy fingerprint; changed
  patterns and string subclasses retain the original search methods.
- Add fresh complete mask/span oracles and actual lightweight CLI-save,
  Claude cold-hook and Codex warm-hook checks. A fixed public 192 KiB lookup
  corpus that timed out now delivers saved progress and a complete index.
  Value lookahead and general cold-refresh liveness remain separate limits.

### Cold document screening cost — 2026-10-15

- Skip secret-pattern scans only when an exact known pattern cannot match:
  its mandatory literal is absent. Keep masking outputs, privacy checks,
  policy fingerprints and whole-index publication unchanged; unknown or
  replaced patterns retain full scanning.
- Add fresh complete masking and native hook/index assertions, including
  Unicode and custom pattern compatibility. This reduces screening work;
  oversized or slow cold refreshes still have no durable scan checkpoint.

### Public OpenAPI declaration export — 2026-10-15

- Recognize a narrow whole-JSON OpenAPI 3.1.1 HTTP bearer declaration by its
  actual structural source position. Keep credential maps, unrelated objects,
  nested decoded documents, duplicate keys and unknown syntax refused.
- Preserve original approved source, request, packet and journal bytes. Add
  fresh complete byte oracles and offline constructor, planner, SQLite journal
  and protected-file capture checks; remote ingestion remains unavailable.

### Retained memory citation admission — 2026-10-14

- Revalidate current chunk eligibility before retaining cached durable memory.
  An unchanged legacy or partially fingerprinted chunk is removed from the
  index when any local citation disappears or escapes the supported roots.
  Preserve ordinary document reuse and declared historical hashes. Add fresh
  CLI/SQLite eviction, surviving citation and UTF-8 regression coverage.

### Optional local memory publication — 2026-10-14

- Confirm a retrieval manifest before recording its delivery. A failed local
  publication preserves fresh context without a manifest reference or new
  ordinary receipt pair; required governed publication still fails closed.
- Allow supported Harness helpers to return bounded source provenance from
  that same invocation, with exact task, capsule and visible-source bindings.
  Keep required approval/result journals and physical-manifest checks.
- Retire trace descriptor ownership before cleanup, drain remaining handles
  once and preserve primary errors. Add fresh actual CLI/Harness, privacy,
  partial-write and controlled descriptor regressions.

### Read-only memory startup — 2026-10-14

- Bound refresh SQLite busy waits and preserve freshly validated working task
  delivery when startup writes are unavailable. Existing compatible rollback
  state is read without cached search or delivery publication; unsupported or
  unknown state remains explicitly incomplete. Add temporary CLI, hook,
  schema, identity, privacy and real permission regression coverage.

### Hindsight forward screening and cancellation — 2026-10-14

- Discover JSON string spans in one forward pass; ordinary strings no longer
  consume decoded-view capacity. Keep raw credential refusal, valid Unicode,
  changed-view/depth/byte ceilings and exact retained source/request bytes.
- Propagate cooperative cancellation through public export/plan revalidation
  and current journal admission before commit. Init-only callbacks remain
  unstored; manual validation and dataclass replacement retain current guards.
- Add fresh byte oracles, indexed-inspection/decoder controls, a fixed standalone
  hang guard and actual SQLite rollback regressions. Remaining regex/decoder
  work is nonpreemptible; no whole-screen linearity or hard-deadline claim.

### Hindsight historical integrity and current admission — 2026-10-14

- Keep intact ingestion history readable for diagnostics, terminal settlement
  and supplied reconciliation after current content/path rules tighten.
  Preserve canonical envelopes, identities, source/body/packet hashes, bounds
  and manifest correspondence independently of mutable admission rules.
- Re-admit only the selected frozen request before body access or a new attempt;
  public export/prepare remain screened. Clean replacements retain blocked
  prior bindings for reconciliation without authorizing remote mutations.
- Add fresh public whole-byte oracles and SQLite regressions for policy drift,
  selected dispatch, unchanged frontiers, forged exports and rehashed corruption.
  Remote transport, provider costs and source authority remain unmeasured.

### Native memory source resilience — 2026-10-14

- Report native UTF-8, JSON, metadata and inventory failures without treating
  unknown governance as an absent task. Creation, result, promotion and MCP
  callers use typed absence from a complete validated catalogue.
- Isolate invalid derived handoffs while preserving independent valid memory;
  abort native authority failures before publishing a partial index. Validate
  the captured second image and Unicode metadata, retain actual task paths,
  and render fallback warnings once.
- Add fresh copied-helper CLI and stdio MCP regressions for corruption,
  incomplete scans, archival, save/recall and Unicode. Synchronize edition and
  generator runtime assets. Local validation does not establish Windows,
  live-client behavior or provider-token savings.

### Cursor persistent-rule continuity — 2026-10-14

- Keep a bounded, policy/task/branch-scoped query-only topic for referential Cursor replies; refresh current canonical sources on every delivery and transfer only to an internally created checkpoint generation. Ordinary CLI/Claude/Codex/Harness profiles keep their previous behavior.
- Use one stable persistent-rule render through selection, manifest and final text/JSON; suppress routine checkpoint/report counts while retaining substantive progress and failures.
- Bound optional cache UTF-8 transport, reject malformed timestamps/JSON and abstain immediately on SQLite write contention without committing caller work. Add fresh local continuation, source-change, failure, projection and ownership regressions; native clients/Windows/provider-token savings remain unmeasured.

- Run session-hook cache/counter/validator tests from disposable copied
  runtimes. A temporary cwd alone does not isolate hooks that derive their
  root from the script location; actual private health writes are asserted.

### Memory result saves, Cursor custody and Harness bindings — 2026-10

- Suppress current metadata and mutations for existing learnings outside the
  public/team save privacy scope, including archived restatements and completed
  replay. Return only the sanitized incoming learning and a fixed held outcome;
  preserve independent progress and facts through CLI/MCP/Harness.
- Restore automatic result saving for hook-updated tasks and ordinary rewording
  of an unchanged runtime-owned learning. Protect manual/person/archived/type
  conflicts individually while saving independent progress and facts; resume
  only new, exactly observed source/scope/domain frontiers after interruption.
- Recover Cursor's registered generated rule after copy, touch, cache loss and
  an older branch writer. Keep physical identities as current-operation guards,
  honor protected Git trust narrowly, reconcile the old exact two-link window,
  and use Linux no-clobber rename when supported. Generated hooks delegate to
  native maintenance with fixed diagnostics; custom deviations remain intact.
- Add actual copied-helper Harness interruption/recovery controls and portable
  UTF-8/root-rename tests. Local checks do not establish native Windows runtime,
  AI provider tokens, client rule-loading order or semantic-quality savings.
- Fence native memory result attempts before shared effects and retain complete
  proof outside disposable local receipts; refuse unknown partial retries, stale
  implicit corrections and generic retry rebasing. Deliberate corrections carry
  their original learning revision through CLI/MCP/Harness.
- Reconcile recorded Cursor custody after a physical checkout rename, preserving
  held capsule bytes and refusing copied/foreign/orphan custody. Add fresh Opal
  result-fence/rename regressions and exclude project attempt metadata from install.

- Cursor continuation prompts now re-query current governed task context, preferring an explicit goal while rendering fresh progress and next steps. Privacy-modified/withheld requests and meaningful new queries retain their ordinary behavior. Known empty enforced selections replace stale selected facts with current working state; repeat/legacy skips preserve the owned rule. Added fresh actual-helper CLI regressions and matching local/Linux/Windows CI entries; unknown legacy ownership still requires exact-image operator approval.

- Cursor working-memory rules now use prospective local custody receipts:
  exact byte/identity and Git checks protect custom, edited, tracked and linked
  files; installed POSIX maintenance shares a nonblocking anchored Brain lock.
  Harness capsule delivery attempts guarded retirement before Cursor startup.
  Identical complete renders remain unchanged; changed excerpts refresh even
  under the same document labels. Fixed partial-failure metadata and runtime
  I/O accounting distinguish successful retirement from incomplete custody.
  Attached clones and unsupported platforms preserve their rule files; no
  native loading, cache or provider-token benefit is claimed.

- Invalidate cached task-file membership on the effective Git index, shared index,
  worktree/prefix and supported object-resolution generation. Preserve warm
  batched-probe reuse; unavailable or changed captures clear cached proof without
  changing citation links or ordinary document writes. Add index-only staging,
  removal, split/alternate index and full capsule/manifest regressions.

- Freeze native-validated automatic Harness memory requests in a separate private
  table for their original session/generation/event. Add explicit prepare/replay
  CLI operations, receipt-only retries, immutable omissions, current scope guards
  and complete transient request-file cleanup. Keep manual and unsupported fresh
  saves compatible; unknown partial writes and post-final replay stay incomplete.

- Preserve exact receipted result replay through CLI and MCP after task
  archival or citation deletion, with current owner/privacy checks and no
  rewrite of later corrections. New writes retain live-source validation;
  archived tasks require a matching receipt. Resolve CLI auto revisions
  under the mutation lock and keep MCP revisions strictly numeric.

- Build Project Brain retrieval rows from one coherent text image: content,
  privacy, lifecycle and source hash now describe the same record or handoff.
  Failed index refreshes deliver only fresh working state, retain diagnostics,
  and skip stale retrieval, frequency lookup and delivery receipts.
- Honor the explicit trailing verification attestation even when its reason
  quotes another marker; keep Memory MCP serving after escaped surrogate IDs
  while preserving ordinary Unicode output.
- Isolate each Harness launch from inherited memory task IDs. Validate the
  task and embedded timestamp of bounded Cursor checkpoint reports instead
  of file mtime, using portable paths. Observe recovery compaction before
  event normalization so the context history records its divider once.

### LongMemEval and Hindsight evaluation pilots — 2026-10

- Keep the pinned LongMemEval API test image byte-identical while explicitly
  deferring its newer annotations in offline compiler controls on Python 3.9.
  This selected-source test profile does not certify the official environment.

- Memory: strengthen offline public Hindsight export screening for literal
  PHP/WordPress/Composer credentials and escaped JSON tokens; accompany pinned
  LongMemEval source fixtures with their upstream license and extraction notices.
  Cursor installed runtimes now include rooted NT handle maintenance and
  no-clobber publication, with native Windows acceptance in CI and abort cleanup
  controls. Linux validation does not certify Windows kernel behavior.

- Harness: add fixed two-slot memory-accounting controls for cancelled and
  unlaunched queries, preserving known subtotals, incomplete quality denominators
  and null full provider costs through construction and repeated finalization.

- Harness: add isolated real temporary descriptor ownership controls for
  LongMemEval adapter cleanup, including retained original handles and immediate
  descriptor reuse. Supplied faults are not actual OS-failure or token evidence.

- Harness: retain privacy-safe fixed projection guard labels per failed local
  operation through a detached optional accessor; preserve task exceptions and
  existing snapshot/receipt/cost schemas. Unknown labels, root causes and provider
  costs remain unmeasured.

- Fix memory regression boundaries: isolate LongMemEval test output in temporary
  projects, retain private Harness bindings without a lifetime admission quota,
  sanitize fixed Cursor Git probes, preserve enforced skipped context, recover
  owned absent rules, and exclude invalid UTF-8 documents per source. Restore
  CI/local gate parity and receipt privacy assertions after manifest validation.

- Classify isolated LongMemEval controller errors by their actual exception type,
  without consulting a supplied `__class__` property. Preserve cancellation and
  primary errors across queries, callback dispatch, construction and finalization;
  keep ordinary failure and partial-construction semantics unchanged.

- Keep isolated LongMemEval diagnostic clock failures from replacing query or
  cleanup errors. Clear delivery metadata when a new timer abort prevents return,
  retain unknown elapsed time, and attempt owned teardown before propagating an
  observer start-clock interruption. Repeated close stays once-only.

- Drain registered directory-descriptor close attempts in the isolated LongMemEval
  adapter after individual cleanup errors. Preserve construction primary errors,
  report the first owner-close error, and never retry retired descriptors or adopt
  an already-closed ambiguous registry. Attempt bookkeeping does not prove closure.

- Fence further insert/query work in the isolated LongMemEval adapter when a
  final inserted-file close fails. Retain already-published state and the same
  exception; do not retry an uncertain descriptor or change teardown behavior.

- Forward optional custody-failure stage metadata through the isolated
  LongMemEval observer and registered bridge. Canonical bounded dispatch works
  after close without changing query hooks, frozen views or cost accounting;
  missing/unsupported observations do not imply source validity or freshness.

- Add optional fixed custody-failure stage metadata to the isolated LongMemEval
  adapter. Existing guards, fencing and I/O remain unchanged; detached metadata
  identifies a source region, with root causes and provider costs unmeasured.

- Add an isolated selected-upstream async reader/synchronous judge controller,
  binding actual SDK argument proxies and supplied usage to owned tickets across
  awaits, errors and cancellations. Legacy zero-filled reader counters are kept
  only for compatibility; physical/provider coverage and full costs stay unknown.
  Fresh offline controls leave automatic production memory unchanged.

- Add a separate isolated Chat receipt controller: budget and seal exact injected
  request bodies, preserve captured usage through decoder/validation failures,
  bind attempts to owned tickets, and keep judge requests in a post-gate journal.
  Physical/provider/SDK coverage and billing remain unknown; fresh offline
  controls do not claim token savings or change automatic production memory.

- Add bounded supplied ChatCompletion usage capture before answer decoding for the
  isolated memory benchmark. Missing counters stay unknown; paid supplied usage
  survives decoder failures/cancellation, without provider/attempt/billing claims.
  Fresh offline controls include pinned selected reader/judge code with fake SDK
  values; production memory, defaults and the closed lifecycle remain unchanged.

- Add a local LongMemEval lifecycle controller with fixed-to-native query binding,
  one-use reader/judge callback tickets, whole-envelope byte selection and an
  immutable construction gate before evaluator lookup. Frozen native scalar
  observations join once; retries, cancellation and missing horizon remain
  accounted. Fresh public controls keep hidden provider coverage/full cost unknown;
  this modified text reader is separate from the official multimodal runtime.

- Register the isolated FTS5 states adapter through a pinned LongMemEval Memory
  API contract with strict direct configuration, invocation-bound ordinal hooks,
  bounded replay, pre-work cancellation and explicit cleanup/persistence refusal.
  Fresh public controls execute the unchanged stdlib API prefix and native FTS5;
  the full official environment, reader/judge costs and token benefit remain pending.

- Add a separate fixed-horizon local observation wrapper for the LongMemEval
  text-projection pilot, connecting native FTS5 operations to the cost ledger.
  Ordinal receipts retain failure/cancellation and unknown usage; disjoint local
  timings and exact JSON byte proxies include insertion replay and teardown.
  Fresh public integration controls keep reader/judge, billing and provider token
  benefit unmeasured; production automatic memory and runtime defaults unchanged.

- Add an experimental fixed-horizon build/query/reader/judge cost ledger with
  private physical-attempt identities, independent completeness, inclusive cache
  accounting and conservative unknown/conflict handling. Fresh literal oracles
  test full scopes and cost-per-correct arithmetic; runtime/provider integration
  and actual token benefit remain pending.

- Add an isolated LongMemEval-V2 public-states text projection with exclusive
  corpus ownership, strict admission and native automatic FTS5 refresh/packing/
  rendering. Fresh synthetic tests keep evaluator labels and image bodies out;
  upstream registration, multimodal accuracy and full provider costs are pending.

- Add a private offline Hindsight ingestion journal: frozen public request
  bodies, monotonic desired generations, committed one-owner reservations and
  cross-version ambiguous-attempt barriers. Supplied document observations bind
  a fresh local challenge and complete history; HTTP404 never proves global
  absence. Suggestions authorize no upload/delete/retry and retain unknown costs.
  No service executor, default wiring or provider-token benefit is claimed.

- Add an offline operator-approved public-source export and Hindsight ingestion
  draft planner. Exact scoped approvals, anchored POSIX custody, raw UTF-8 hashes,
  privacy screening and whole-payload limits precede returned images. Version
  changes retire current catalogue membership; prior input never proves upload
  or remote no-op. All current request bodies remain available for reconciliation.
  No executor, live upload or provider-token savings are claimed.

- Add bounded internal source hints for optional remote recall: native exact
  indexed-source hydration, raw-byte and normalized hash checks, POSIX custody,
  ordinary guards and final packing own delivery. Explicit local project/bank/
  workspace routes prevent cross-project outbound queries. Separate remote
  diagnostics retain unknown costs on local failure; FTS5 and provider defaults
  remain unchanged. Fresh public whole-caller tests cover guards, cancellation,
  history priority and delivered-only manifests/receipts. No live service or
  provider-token benefit is claimed.

- Add an isolated stdlib Hindsight recall pilot with registered team/project
  scopes, bounded direct HTTP transport, locally admitted source-byte evidence,
  whole-envelope budgets and exact local fallback. Remote prose and arbitrary
  diagnostics cannot enter context; unknown costs remain null. New public
  offline/loopback controls cover corrections, deletion, isolation and failures.
  Automatic Harness/native routing, ingestion and live token savings remain
  pending; local FTS5 defaults and Python runtime requirements are unchanged.

### Earlier unreleased changes

- Keep uppercase `Task/` client applications, specifications and project work
  local instead of versioning them in the accelerator repository. Regenerate
  source inventories without changing installed payloads; use synthetic client
  inputs to test installer exclusion and collection opt-in. Lowercase `tasks/`
  workflow scaffolds remain versioned and installed.

- Align Harness integration assertions with lossless compact capsule delivery,
  independent cost receipts and runtime-confirmed repeat saves. Preserve full
  capsule and warning checks, preview/delivery meter separation, refusal before
  provider launch, and persisted learning identity across follow-ups.

- Harness knowledge metadata discovers and selects banks without reading unused Markdown previews. Public document browsing, bank order/selection and script/config/Brain checks stay unchanged; metadata no longer inherits incidental preview-scan failures.

- Harness compacts lossless capsule JSON only for actual ordinary, Fleet and
  Clash delivery when no rendered text is supplied, including reviewed sessions.
  Review previews and existing helper/API formatting stay compatible; an additive
  delivery meter keeps displayed estimates aligned with the inserted prompt.
  This reduces UTF-8 bytes and characters, without claiming provider-token savings.

- Harness uses lossless compact JSON for unattended prompt capsules when the
  runtime provides no rendered text. Preserve every field and scalar, the
  whole reference warning, reviewed/default JSON and rendered/native paths;
  prompt attribution, notices and cost observations measure the same format.
  Separator-byte reduction does not establish provider-token savings.

- Generated accelerators now carry the same automatic-promotion admission
  fairness as the ready-made editions: duplicate candidates do not consume
  new-attempt slots, failed attempts still do, and stalled proposals retry
  separately. Synchronize the generator runtime and its tool mirrors.

- Project Brain serializes active-alias lookup and record creation under the
  repository mutation lock, preventing concurrent creators from writing two
  UUID records with the same alias. Preserve cross-type active uniqueness,
  archived-alias reuse, source-error precedence and canonical write rollback.

- Harness shares one trusted capsule envelope across prompt insertion, memory
  attribution and constructed-cost observations. Its leading reference-data and
  canonical-source warning is 21 ASCII bytes/characters shorter; rendered bodies
  and compact-JSON capsule statistics are unchanged. This is a constructed-text
  reduction, not a measurement of provider tokens or model quality.

- Harness full-cost receipts add bounded native boundary diagnostics for main
  and recovery attempts: observed terminal/error occurrences, fixed stop/read
  signals, and separate guarded-process codes before and after cleanup. Unknown
  causes remain unmeasured; stderr and error text are not captured in these
  diagnostics. Observation failure preserves work and existing cost accounting.

- Automatic durable-memory promotion counts new nonduplicate proposal attempts
  against its batch limit, so a stable prefix of duplicate records cannot starve
  later independent facts. Failed attempts still consume admission; duplicate
  guards, stalled retries, source eligibility and supersession stay in force.
  `skipped` now counts the uninspected tail. Duplicate scanning can visit the
  whole candidate set; the limit is a write-admission cap, not a CPU bound.

- Harness records a generation-owned `full_cost` receipt for ordinary native
  turns: main and recovery provider usage, attributed USD, constructed prompt
  parts, local runtime byte counts and retries, and elapsed recall/save/checkpoint
  work. Missing or incomplete observation stays unknown; payload text and raw
  tool identities are not retained. Observation failures preserve the task,
  and cancellation/resend cannot attach an old turn's cost or writes to its successor.

- **Keep distinct automatic memory claims and reject conflicting drafts.**
  Within the parsed, bounded draft, type/title/exact source sets identify a
  learning. Identical copies deduplicate; conflicting consequences omit the
  whole group with a count-only notice. Independent claims and progress save
  normally. Manual submissions and the durable writer retain their own rules.

- **Keep automatic memory corrections across Harness runs.** Session title
  markers no longer discard changed consequences, new sources or unfinished
  promotion retries before the durable result writer. The writer retains
  ownership, attestation, replay and no-op checks; same-draft filtering stays
  bounded. Older command-chain runtimes still have no cross-run idempotency.

- **Preserve winning excerpt evidence when retaining a qualifier.** A colon
  introducer may replace only trailing context that leaves the selected
  window's evidence score intact; tight limits cannot turn a matching answer
  into an evidence-free introducer. Preserve capsule limits and retrieval defaults.

- **Reject malformed or conflicting passage labels before evaluation.**
  Validate document objects, string lists and Boolean usefulness, and refuse
  conflicting normalized skill aliases. Document that literal passage
  delivery measures supporting coverage rather than final-answer correctness.

- **Close file-link test fixture databases.** Register SQLite cleanup as soon
  as each fixture connection opens, preserving the original assertions and
  borrowed helper ownership. Runtime behavior and retrieval defaults are unchanged.

- Automatically retain a separate `ordinary_receipt` status observation in
  refresh JSON, local trace/health metadata, evaluator refresh metadata and
  Harness prepared context. Strict scalar validation and an eight-field health
  summary keep missing evidence unknown; each sibling fails independently.
  Preserve paired receipt transactions and cleanup, capsule/prompt content,
  scores, token meters and Python/FTS5/optional ONNX defaults. Status records
  do not establish refusal causes, durability, emission or host ACK.

- **Close context-usage test databases after their transactions.** Two SQLite
  fixtures now preserve commit/rollback before closing their connections,
  eliminating allocation-traced warnings without changing runtime behavior
  or the tests' behavioral assertions.

- **Unwind interrupted ordinary memory bookkeeping.** Cleanup is armed
  before transaction or savepoint acquisition and retries once if a
  cancellation interrupts cleanup. Caller transactions and connection
  ownership, original exceptions and busy timeouts are preserved.
  Completed commits remain durable after acknowledgement failure.

- **Keep automatic memory diagnosis through its consumers.** Health JSON
  aggregates fixed counters over at most 500 selected records, retaining
  unknowns and lower bounds. The evaluator and Harness retain validated
  invocation observations outside capsules, prompts, scoring and approval
  material; invalidated prepared context clears its observation. Missing or
  failed metadata leaves usable memory and legacy health output intact.

- **Close interrupted memory database initialization.** The context engine
  now releases its owned SQLite connection on cancellation during acquisition,
  migration and the successful return boundary. Original errors and completed
  commits retain their behavior; failed cleanup still attempts both rollback
  and close. Ordinary successful callers retain connection ownership.

- **Retain memory selection diagnosis without manual flags.** Refresh JSON
  and existing local health records now carry a fixed metadata summary bound
  to that turn's complete rendered trace. Missing, unsupported, stale or
  partially read evidence stays unavailable; source coverage and consumption
  remain unknown. Capsule text, selection and ordinary bookkeeping are preserved.

- **Diagnose memory selection stages automatically.** Local traces now contain
  fixed privacy-filtered occurrence counters and observed budget, repeat,
  gate and pack boundaries. Unseen source coverage and external consumption
  remain unknown. If the new child exceeds the trace byte cap, the original
  trace and rendered-window metadata are preserved without it.

- **Keep memory evaluation timings finite.** Invalid ordinary timeouts and
  measured durations fail before setup or aggregation; optional malformed
  runtime phases are omitted. Missing legacy timings remain unavailable.
  Result export validates finite UTF-8 JSON before writing and atomically
  replaces an owner-only file with cleanup on failure.

- **Preserve memory bookkeeping across concurrent retrievals.** Baseline and
  conversation receipts now update together from current SQLite maps. Stale
  target snapshots cannot overwrite a newer turn or undo a compaction reset;
  other sessions and baseline keys survive. Read-only, busy and failed writes
  keep ordinary retrieval available, and caller transactions retain ownership.

- **Support a strictly pinned optional Jina worker profile.** Versioned local
  manifests select the explicit two-input/pad-one adapter; worker-owned asset
  pins and CPU/runtime/tokenizer/float32 checks reject incompatible profiles.
  Schema-less MiniLM manifests retain the existing route. Worker timeout
  overrides are capped at five seconds, with the tighter caller deadline and
  cleanup reserve preserved. Default FTS5, one-attempt scoring and full lexical
  capsule/session-receipt fallback remain.

- **Observe suppressed session windows without changing novelty.** Automatic
  local traces compare a screened pre-novelty proposal with the prior final
  rendered body under the same delivery identity. Bounded session companions
  point to existing trace UUIDs; original receipt tuples and four-turn expiry
  remain. Legacy/pruned/corrupt traces stay unknown, protected records stay
  unavailable, and diagnostic failures cannot change the capsule or gate.
  Different fingerprints signal exposure differences, not confirmed fact loss.

- **Admit optional excerpt alternatives before scoring.** Screened windows
  must fit the existing structured snippet cap and the selected item's unchanged
  token estimate before consuming worker slots. An oversized alternative cannot
  displace a feasible quote; the lexical choice and complete fallback remain.

- **Keep optional ONNX quotes and delivery receipts coherent.** The maintainer
  pilot prepares screened excerpts before session novelty and verifies the
  existing packer's membership before recording delivery. Structured snippets
  match the chosen quote, citation-only footers keep the lexical safeguards,
  and a rejected projection retries the complete lexical selection once
  without another model call. The late-render callable remains lexical-only.
  Worker startup, pipe I/O and cleanup share one deadline; POSIX groups are
  cleaned up even after their leader exits, and Linux workers guard parent
  death. The evaluator binds the optional invocation to its configured timeout.
  Default retrieval remains FTS5 and requires no ML dependencies.

- **Reuse citation normalization without caching freshness.** Each retrieval
  filter keeps a bounded cache of exact freshly read source bytes and their
  normalized SHA-256 digest. Every citation still performs path validation and
  a fresh file read; changed, deleted and unreadable sources retain the same
  handling, and final validation creates a new cache. Source-change warnings,
  ranking, FTS5 selection and capsule limits remain unchanged.

- **Deliver the rule in source-linked memories.** Linked records quote
  substantive sections instead of generated citation-only Sources/References
  footers. Explicit footer requests, annotated references and source-only
  documents remain readable. Corrected automatic excerpts are delivered
  immediately in existing sessions; headings receive the same privacy screen
  as excerpt bodies. Candidate selection and capsule limits are unchanged.

- **Explain memory selection and final delivery without enlarging capsules.**
  Governed retrieval records bounded metadata in ignored local trace sidecars,
  including lexical admission, linked-document filters, budgets, layer limits
  and the final rendered section/body hashes. Optional `--trace-path` watches
  a document without changing search. Protected metadata stays excluded;
  diagnostic storage failures leave retrieval and delivery unchanged.

- **Preserve attached Codex policy and hooks with nested CLI overrides.**
  Direct and Harness overlay launches collect `-c`/`--config` arguments into
  the root CLI scope so an `exec` or `resume` option cannot silently discard
  accelerator instructions and hooks. Caller override order, literal `--`
  arguments and Node launcher prefixes are preserved.

- **Independent evidence for history admission.** Porter variants of the
  same word no longer satisfy the two-concept coverage rule for Project Brain
  events or local episodes. A query about one concept remains valid. The raw
  search window, ranking weights, history slots and capsule limits stay the
  same. Six regressions cover admission and the preserved window in all four
  editions; generator seed assets carry the same correction.

- **Opt-in memory miss diagnosis and isolated ONNX excerpt experiment.**
  The maintainer evaluation stand can trace judged paths through the main
  FTS window, recorded-history admission/selection, policy, budgets and actual
  answer delivery, using the same prompt-time clock. Diagnostic reports keep
  skill routing separate from project knowledge. A separately provisioned,
  hash-checked CPU worker can rank bounded excerpts of already selected
  documents; missing dependencies/assets, timeout, busy or invalid output
  keep the exact lexical excerpts. Worker inputs receive the renderer's
  privacy screen, and changes that alter packed path selections are refused.
  Normal installed runtimes and their Python requirements are unchanged;
  optional ML packages are not installed by CI. Focused stdlib tests cover
  these diagnostic and fallback contracts.

- **`--phase quality` is accepted.** The Phase Map in `SKILL FLOW.md` names
  the reviewing and testing row "Quality", and `update --phase` /
  `brain-update --phase` rejected that word; it is now an input alias stored
  as `verification`, next to `review`. Found while merging the 2026-08-05
  QA-fix branch, whose own phase change was superseded. `PROTOCOL.md`, the
  `--phase` help and the round-trip test in `project-brain/tests` list it;
  the generator asset follows.

- **Merge chats into a new task, and hand curated context to another one.**
  Ported from the `codex/context-handoff` branch onto the automatic memory.
  - `context-save` / `context-load` skills and `/context-save`, `/context-load`
    commands in all four editions, backed by `memory-bank/scripts/context_handoff.py`
    and `context.py context-save|context-load`: a curated `summary`, `topic` or
    `full` handoff (an explicitly exported visible conversation kept verbatim)
    with provenance and file fingerprints, written under `tasks/` and loaded
    with branch/commit/file drift. Handoffs never write Project Brain, Memory
    Bank or SQLite and are excluded from indexing even when renamed. Curated
    fields refuse personal data as well as secrets (`sensitive_label`, the
    policy of every authored memory). In an attached session paths resolve in
    the project and a handoff is never written into the accelerator state.
  - `context-continuity.sh`, one argument-free root-anchored hook per tool on
    session start, prompt and end of turn (Claude `SessionStart`,
    `UserPromptSubmit`, `Stop`; Codex the same through the root-finding
    launcher; Cursor `sessionStart`, `beforeSubmitPrompt`, `afterAgentResponse`),
    dispatches on the payload's `hook_event_name` to
    `memory-bank/scripts/context_continuity.py`. It keeps each chat's visible
    prompts and final answers in ignored `.context-handoff/` (the launcher's
    state directory when attached), and `context-load merge` freezes 2-8 chosen
    chats into an attributed archive that the next new session of the chosen
    client receives as a bounded preview (6,000 bytes). The branch's automatic
    replay of up to eight recent chats into every new session was not taken:
    the Task Capsule already carries a branch's work into the next session, so
    only an explicitly prepared merge is delivered. Locking works on Windows
    (`msvcrt`) as well as POSIX. `CONTEXT_CONTINUITY_DISABLED=1` turns
    snapshots off; `CONTEXT_CONTINUITY_RESTORE_DISABLED=1` skips delivery.
  - Harness **Merge chats** (the **Merge** button beside the session list):
    2-8 inactive ordinary chats of a project start a fresh Codex, Claude or
    Cursor task with a frozen, source-attributed archive of their
    Harness-visible messages (private state archive and SQLite record, retry
    by request identity, restart of an interrupted merged task, bounded
    storage, orphan cleanup at start). The merged task keeps the automatic
    memory of any new session (the branch refused a Brain link); a Claude
    merged task cannot start with a `/` command, which would skip the merged
    context. Its launch sets `CONTEXT_CONTINUITY_RESTORE_DISABLED=1` so a
    native merge prepared for that client is not consumed. `create()` now
    commits the session row, its settings, a merge archive and the first
    message together, and a native launch whose prompt cannot be built (a
    changed archive, a missing workspace) fails with its reason instead of
    "Run failed (SessionError)". CI's Harness step (and `scripts/check.py`)
    runs the new `tests.test_harness_merge`.
  - Infrastructure Creator seeds `context_handoff.py` and
    `context_continuity.py`, generates the two skills and the hook, and the
    bootstrap verifier checks the hook's per-event wiring in the one
    root-anchored form per hook.
  - The fine-grained GitHub token pattern the branch added was already in
    `automatic_query.py`, and mirror executable bits were already kept by
    `build_mirrors.py`.
  - The context-budget ceilings in `scripts/token_budget.json` rise by the
    growth this brings and no more: `agents_md_bytes` by 419 bytes in each PHP
    edition (Laravel 12135, Symfony 16069, PHP Core 11959, WordPress 16090),
    `frontmatter_bytes` by 604 in Laravel, Symfony and PHP Core, `body_bytes`
    by 7,765 in Laravel, Symfony and WordPress, the PHP Core skill count from
    37 to 39, and Infrastructure-Creator's `body_bytes` by 4,231 to 281856.
    PHP Core's `AGENTS.md` and the generator's skill bodies keep the 3 bytes
    of headroom they had before.

- **Memory MCP: a message nested past 64 levels is a parse error on every
  Python.** The server relied on the JSON parser's recursion limit, which now
  follows the C stack; a newer Python parsed 60,000 levels and answered
  "invalid request" (installation CI). The depth is checked before parsing,
  strings aside.

- **A Windows sync can create files.** The installer's descriptor-relative
  `create_file` asked `NtCreateFile` for synchronous I/O without naming
  `SYNCHRONIZE` in the access mask; the call checks the requested mask before
  generic rights are mapped and answered "The parameter is incorrect", so no
  new file could be written by a Windows `--sync`. The two desktop-app tests
  that read a login shell's POSIX PATH are skipped on Windows, where the
  application does not read one.

- **The Task Capsule carries no skill.** In real sessions of the five
  evaluated projects agents took none of the 71 skills a capsule named (no
  Skill call, no `SKILL.md` read on those turns), and used accelerator skills
  on 1.1% of Claude turns at all; every host lists its skills and loads its
  instruction files itself. The procedural line only took room from
  answer-bearing excerpts. Ranking still picks one and the manifest records it
  as `host-listed`; `search` still returns skills; lightweight mode follows.
  `AGENTS.md` (four editions), the `project-brain` skill, `PROTOCOL.md` and the
  READMEs say so.

- **The memory evaluation stand reads project knowledge and skill routing
  apart.** Agents in real sessions followed none of the 71 skills a capsule
  named and used accelerator skills on 1.1% of Claude turns, while skills were
  53 of the 66 useful paths the core set missed. The summary adds
  `knowledge_*` turn classes and paths with skills left out, and `skill_*`
  routing outcomes (useful one, another one, none; noise or silence where
  none was useful), computed from each item's paths so older results have
  them too.

- **Related knowledge gives its place back first.** When the capsule's JSON
  passes 8,000 characters or its text 3,600, an item a link brought in
  (`source-link`, `prompt-link`, `touch-link`) is dropped before history the
  query found; the JSON limit used to drop the changelog first, and on one
  evaluated prompt a linked record pushed out the useful changelog entry.

- **File edges: the files a task touched link it to the work that follows.**
  An active Brain task's `files[]` (Git-tracked, screened) become
  `document_links` rows of kind `file`. Automatic retrieval seeds them with
  the paths a request names - written out, as a PHP class through
  `composer.json` PSR-4, or as a bare file or class name - and the files the
  branch touched on its last turns, and may add one linked document after the
  strong matches, marked `prompt-link` or `touch-link`. A hub file (more than
  five links) reaches nothing; a document reached only through touched files
  must share an informative query term. `--path`, MCP `paths` and source-link
  expansion still read citations only; `links` also shows touches. The manifest
  schema's `selection` gains the two values.

- **A Brain record whose cited file was edited stays retrievable, marked.**
  Any edit to a cited file used to evict the record at the next index -
  accepted decisions, open findings and events alike - and its source links
  with it; on the evaluation 77 record admissions with sources were all
  excluded this way. The record is now indexed, and retrieval marks it
  `source_changed` and ranks it at half its score, as it already did chunks.
  Only a citation that no longer exists (`source-missing`) or was never
  digested (`source-undigested`) keeps a record out, checked by existence
  alone so the per-turn rebuild hashes no cited file. `validate` says which
  case each warning is, and the capsule's mark names one file and counts the
  rest.

- **Recorded history has its own slot and its own search.** The capsule's
  episodic layer carries the changelog and one Project Brain event or local
  episode; they used to share one slot, which the changelog held on every
  evaluated turn that had an event while 19 answers sat in events never
  delivered. Events are found by `history_query`: every informative term of
  the whole request, matched against the event's subject, admitted when it
  covers two terms whose weight equals at least two terms unique in the index -
  no tuned constant. Events leave the main query's window and floor. A local
  episode follows the same rule and now renders its outcome. A delivered event
  no longer costs the third semantic item. The governed contract is 1/3/2
  (lightweight mode keeps one history item); `AGENTS.md`, the project-brain
  skill, `PROTOCOL.md` and READMEs say so, and the Harness accepts two history
  items.

- **A skill's sub-files no longer take the capsule's procedural slot.**
  References, agents, rules files, an `AGENTS.md` inside a skill and
  `SKILL FLOW.md` were useful 1 time in 185 judgments against 71 in 627 for
  `SKILL.md`. When one ranks first the slot stays empty - the next skill down
  was useful on none of the 19 replayed turns - and the manifest records
  `skill-subfile`. Lightweight mode applies the same rule. They remain
  searchable with `search`.

- **A document with a detected credential is masked in the index, not
  dropped.** One placeholder-shaped development credential in a code block
  (`password: admin`) used to take a whole README out of retrieval; on the
  121-prompt evaluation it was the only useful document of seven holdout
  prompts. The indexer now replaces each value a secret pattern recognises
  with `[redacted: <label>]`, keeping the key, and excludes the document only
  when masking does not converge. The stored source hash stays the file's, so
  governed retrieval does not call a masked document stale. `index --json`
  reports `redacted` paths with a count of values. A change to the secret
  patterns re-reads every document once. Memory Bank chunks with a likely
  secret are still excluded whole.

- **The memory evaluation stand restores archived records and counts each
  mechanism.** Archived Brain records that existed at the prompt are restored
  from the working tree under the as-of rule already used for active records,
  so a promoted chunk citing its archived source validates as it does in the
  project (no evaluated project commits its archive). Items carry counts of
  item kinds per layer, link selections, changed-citation items, skill
  sub-files in the procedural slot and secret-pattern matches in the capsule;
  `refresh.index` counts indexed documents, link rows by kind and indexed
  documents matching a secret pattern. Results are written owner-only.

- **A governed retrieval manifest that carries source-link counters
  validates.** Manifests now declare schema version 4, which is version 3 plus
  `source_links`. Version 3 had shipped with an exact key set, so every
  governed manifest written since the counters were added failed
  `context.py validate` ("retrieval manifest does not match strict schema").
  Versions 1-3 keep validating; a version 3 manifest that carries the counters
  is still rejected.

- **Automatic excerpts keep the evidence they score, and related knowledge can
  share a current code source.** Adjacent FTS highlights retain each term's
  weight; exact section-score ties prefer body evidence; oversized sentences
  select a bounded matching stretch and are scored only on visible terms.
  Strong semantic matches also reach indexed neighbours independently citing
  the same current canonical source fingerprint. The source itself need not be
  indexed. Private/derived paths, missing or changed fingerprints, symlinks and
  large sources do not establish this relation. Indexed per-source reads cap
  distinct neighbours without materializing duplicate citation products;
  at most 32 source references per anchor can establish the relation. Runtime filters, conflict checks, direct-hit
  priority and nonrecursive budgets still apply. No user setting is introduced.

- **Source-linked knowledge is retrieved automatically.** Every normal
  capsule retrieval follows one existing citation edge from at most two strong,
  allowed semantic matches, examines at most 32 rows and proposes at most three
  neighbours. There is no graph activation mode or user setting. Hooks, Memory
  MCP, Harness, first-turn and lightweight retrieval use the common filtering
  and bounded delivery pipeline. Direct matches keep priority, promoted graph
  chunks cannot suppress their direct source and conflicting neighbours are
  excluded. Manifests carry safe counters without a mode. Repository-only
  `memory_graph_pilot.py` compares committed baseline and working-tree runtime
  versions with the same scorer and synthetic inputs, including future and
  irrelevant-citation controls. No Neo4j dependency is introduced.

- **Historical memory evaluation withholds private source files before reading
  them.** Git-tree materialization and current-worktree copies apply the shared
  source denylist to environment files, credentials, private-key/database
  suffixes and dependency/cache paths. The new public-tree cache namespace
  prevents reuse of older unfiltered snapshots. Project-owned memory remains
  eligible. The comparison baseline includes all four installable editions,
  allowing automatic edition detection on real project inputs. Regression
  coverage proves private blob IDs are never read and private worktree files
  are never copied. Evaluation documentation distinguishes curated labels,
  delivery, successful opens and unknown knowledge use.


- **Third review of the unified memory branch: replays, a person's correction, revised knowledge in the bank, promotion runs at once, and more secrets.** Each finding was reproduced first; every regression test fails on the previous code.
  - **A delayed replay no longer rolls back a later revision.** A result whose receipt matched still went through the revision step, so saving A (X), then B (Y) of the same learning, then retrying A put X back and raised the record's revision. A proven replay now leaves every learning as it is - reported `existing`, `differs` where a later save revised it, `missing` where the record was removed - and only retries promotion; `replayed` is true for it.
  - **A person's correction is theirs.** A revision recorded with `attestation: person` updated the consequence but kept the agent's mark from the first verification, because the runtime recorded attestation only on the observed -> verified step, so the next agent could overwrite it. `update_record(..., reverify=True)` now verifies a revised claim again in the record's ledger - another observed -> verified entry, the one authority edge every runtime's validator accepts - and `record_result` uses it for every revision, so `record_attestation` reads who checked the current claim.
  - **Revised knowledge reaches the Memory Bank.** A record promoted once was excluded from promotion for good, so a corrected consequence stayed in Project Brain while the bank kept serving the old one, marked only "source changed". A record revised since its automatic promotion is promoted again and its chunk supersedes the earlier one in the same write (`superseded`, `superseded_by`, `valid_to`; the new chunk lists it in `supersedes` and takes an identifier hashed from the record revision). A chunk a person reviewed, or one a person already retired, is not replaced: the revision is reported as blocked with what to do.
  - **Promotion runs at the same time write one proposal and one chunk.** Choosing a record, proposing it and applying the proposal were separate locked steps, so two runs at once - a Stop hook and a Harness save - both proposed the same record; one chunk was written and the other proposal stayed `reviewed`, refused on every retry or, on a later day, written as a second chunk. `auto_promote` now holds the mutation lock for the whole step, never proposes a record whose automatic promotion is still open, and withdraws one that can never apply - its record moved on, moved, was removed, or another promotion already brought that revision in - as `rejected` by nobody, with the reason under `conflicts`, instead of retrying it for good (`closed` in the result).
  - **More secret formats, and a customer's number without a colon.** The shared patterns now know Slack tokens and webhooks, Discord webhooks, Telegram bot tokens, GitHub fine-grained and GitLab tokens, AWS temporary keys, Google API and OAuth tokens, Stripe restricted keys, npm, PyPI, Hugging Face, SendGrid, Shopify and DigitalOcean tokens, every private-key armour (DSA, encrypted, PGP), and `AccountKey=`/`private_key=`/`refresh_token=` assignments; placeholders without a digit (`xoxb-your-token`) do not count. "customer ID 10492", "customer #10492", "patient no. 77", "ID клиента 10492" and a bare "customer 10492" are personal data: a write or a direct query is refused, and an automatic query keeps the label and loses the number. A Harness session's goal, task ID and records no longer carry either. No new match in this repository or in six client projects' Project Brain, Memory Bank, docs, specs and tasks.
  - **`memory_record_result` refuses a progress that is not text.** The schema says text, but `null` was read as "leave the task's progress alone" and the learnings were recorded with it; a client's error is now refused before anything is written, as `memory_checkpoint` refuses a goal that is not text.

- **Third review of the unified memory branch: the gate defers to the conversation, the JSON capsule quotes the body, and the checkpoint checks its goal.** Each finding was reproduced first; every regression test fails on the previous code.
  - **A new conversation is handed what it was never handed.** The retrieval gate's repeat baseline is the task's (task, host, entry point) and every conversation shares it: under `--gate enforce` a second conversation asking what the first had asked was a `repeat-retrieval` and lost its whole capsule, although its own delivery record held nothing. The same rule withheld what one conversation's record hands again after a compaction or past the novelty window, on that turn and on every later one asking the same, since a withheld turn records nothing as handed. For a turn with a session ID the record, which has already left out what the conversation holds, is now the one notion of "seen": what would be a repeat retrieves as `not-held-by-conversation`, a turn whose every item is held still skips as `empty-after-filter`, and a caller without a session ID keeps the task's baseline (`gate_decision(conversation=...)`; `docs/CONTEXT-AND-MEMORY.md`).
  - **The JSON capsule quotes the finding, not the frontmatter.** A ranked candidate's snippet was FTS `snippet()` over the whole file, which centres on the densest run of matches; in a promoted chunk that is the JSON frontmatter, whose `title` repeats the heading, so after promotion the Memory MCP's `semantic[].snippet` for "cobalt allocation" was `"supersedes": [], "tags": [...]` cut before the finding, and the token estimate, the budget and the manifest counted those keys while the rendered capsule quoted the finding. The snippet is now chosen the way the rendered excerpt is - the document marked by the words that selected it, the section carrying most of them, the 320-character window where they are (`_quote_candidates`, `SNIPPET_WINDOW_CHARS`) - and `estimated_tokens` is of that text. One `highlight()` over the candidates that survive relevance replaces a `snippet()` computed for every match: over six requests on an index of the PHP Core edition, ranking takes 1.3 ms longer on average and 3.6 ms at most. A section the query never matched - a document matched only in its frontmatter - gives its opening prose, as `quoted_section` promises; the window took its last sentence, in the rendered capsule too.
  - **`memory_checkpoint` checks the goal it was given.** The goal was checked only when the call created the task, so `goal: 42` - or `null`, `""`, a list - was refused for a new task and accepted, flushing a checkpoint, for an existing one. A goal passed is now checked before the task is looked up. The other tools already check every argument before branching on state: `memory_retrieve` its query and paths whether or not the task exists, `memory_record_result` the whole result before it reads the task.
  - Tests: `PHP Core/project-brain/tests/test_runtime.py` (`DeliveryTest`: two conversations, the novelty window and a compaction under enforce; a promoted chunk's JSON snippet and estimate; a match only in frontmatter; an unmatched section's window), `tests/test_memory_mcp.py` (the cobalt allocation retrieval after promotion; seven malformed goals against a new and an existing task).

- **Third review, Harness: a Codex memory recovery reaches the launch's fill, an update follows a remote with a slash in its name, and the Codex instruction budget has a ceiling.** Each finding was reproduced first; every regression test fails on the previous code.
  - A Codex launch's fill was settled from its rollout before the memory recovery ran, and the recovery's events then moved only the cache share: with the launch at 12,000 input tokens and the recovery at 24,000, the launch kept an end of 12,000 and one call, although the recovery prompt was already in its ledger, and a compaction during the recovery got no divider. The recovery resumes the same thread, so its calls and compactions follow in the same rollout; the Harness now settles the rollout again once the recovery has run. `CodexLive.settle` goes on from the end of the tail it read, last line included, not from where the live reads stopped, so that second settle adds only what the recovery appended instead of counting the tail's lines again.
  - The update check split the followed branch's short name at its first slash, so a remote named `team/main` was fetched as the remote `team` and the branch `main/main`, and every check failed. The remote and the branch ref now come from `branch.<name>.remote` and `branch.<name>.merge` and reach `git fetch` as separate arguments after `--`; the fast-forward-only merge and every refusal are unchanged.
  - The Codex instruction budget grew with the `AGENTS.md` size without bound - a sparse 2 GiB file asked for `project_doc_max_bytes=2147516416` - and that size was read through a link, whose target can be any file. The budget now stops at 256 KiB (`CODEX_DOC_CEILING`: the largest `AGENTS.md` this repository ships is 17,251 B and `scripts/token_budget.json` lets none pass 17,971 B, so with the 32 KiB left for the rest of the chain a project's own instructions keep about 200 KiB in front of the policy block), and only a regular file raises it: a linked `AGENTS.md` or override leaves Codex's own budget. `docs/TOOL-INTEGRATIONS.md` says so.

- **A third review: the System planner reads only what can still fit, and the stand scores only what it measured.** Each finding was reproduced first; every regression test fails on the previous code.
  - **The planner read every declared source to fill a context that was already full.** `ai_system.py plan` read each source in full before asking whether it fit: 100 sources of 64 KiB under the default 8,000 characters were all read for the 3 that fit, and the declared maximum - 500 services of 100 sources of 256 KiB - is about 12.2 GiB of reading for one plan. A source is now read only while it can still fit: when the context has no room left for even an empty file's entry, or the file's size rules it out (a usable file of N bytes holds at least N/4 characters), it is omitted with `reason: budget` without being read. Reading for one plan is also bounded, at 2,000 files and 32 MiB with memory citations: past that the remaining sources are omitted with `reason: read_limit` and the plan warns `source_read_limit`. Short of the limit the context is the one reading everything packed - 600 randomized plans against the previous planner gave the same contexts and omissions - and packing no longer re-encodes the whole context per source. A source left unread has no fingerprint, so `verify` does not report it changed; whether it was left out depended only on the sources that were read and its size (`docs/AI-SYSTEM-ORCHESTRATION.md`).
  - **The stand scored a refresh that failed.** The memory runtime prints its JSON and exits 1 when a memory layer failed to update; `scripts/memory_eval.py` scored any JSON it could parse, so that turn counted as evaluated and silent - in the denominators and the latency - and a runtime that broke indexing read as a quiet one. A refresh is now scored only when it exited 0: otherwise the prompt is skipped as `refresh-error`, or `refresh-timeout` past `--timeout` whatever it printed, and the item keeps the exit status, the warning count, the stderr tail and, new, each layer's state (`updated`, `failed`, or `other` for anything else).
  - **A/B deltas compared different prompts.** `report --compare` subtracted each run's totals, so a prompt useful in A and skipped in B showed `useful -1` while the paired diff, rightly, showed no loss. Each measure's delta is now over the prompts both runs evaluated, `-` when there are none; `prompts`, `evaluated` and `skipped` still compare the whole runs. When the runs evaluated different prompts the column reads `paired delta`, a note gives the counts, and the paired diff names the ids evaluated in only one run.
  - **Grades outside 0, 1 and 2.** A judgment of 3, -1 or 10^30 was taken and counted as useful or noise; one of `true`, `1.5`, `2.0`, `"2"` or `null` was dropped silently and its document counted unjudged, and so was a prompt whose judgments were not an object. `run` now stops before any work with an error naming the prompt, the path and the value (exit 2) (`docs/MEMORY-EVAL.md`). A prompt's `_note` - a reviewer's note as text, which real judgment sets carry - is the one key read past; a stricter loader without it refused the whole tuning set.
  - Tests: `tests/test_ai_system.py` (`test_planner_reads_no_source_that_cannot_fit`: the reviewer's 100 sources in one service and spread over ten; `test_planner_stops_at_its_read_allowance`: refused sources by bytes, missing ones by files; `test_sources_left_unread_could_not_have_fit`: a budget sweep against reading everything, and the exact boundary of a four-byte-character file), `tests/test_memory_eval.py` (`RefreshOutcomeTest`: the runtime replaced by one that exits 1 or times out after printing JSON, layer states never client text, and the real runtime's layers in the end-to-end run; `test_compare_deltas_are_over_the_prompts_evaluated_in_both`; `test_judgments_take_grades_0_1_2_and_refuse_anything_else`; `test_run_refuses_judgments_out_of_range_before_any_work`).

- **No write follows a link swapped into a project on Windows, or into an attached state anywhere.** Two findings of a third review, each reproduced first; every regression test fails on the previous code.
  - **A Windows sync wrote outside the project.** Native Windows has no descriptor-relative calls in Python, so the sync looked at each folder with `lstat` and then wrote by the path, and a `.cursor` swapped for a link in between sent `.cursor/mcp.json` to the link's target (reproduced on the same fallback under Linux). The installer's walk now goes through one set of calls: folder descriptors on POSIX, unchanged, and on Windows NT handles (`_HandleCalls`) - each component opened with `NtCreateFile` relative to the folder before it, never through a reparse point, the temporary file created in and renamed within the last folder's handle, handles that do not share delete. A path write under held handles was not enough: holding an empty folder does not stop another process turning it into a junction in place, which needs only `FILE_WRITE_ATTRIBUTES`. The calls are a copy of `scripts/portable_fs.py`'s, because the Harness runs the installer alone from a staging folder. Where neither descriptors nor handles are available nothing is written: the sync reports each file in `kept` (`NO_SAFE_WRITE`) and an install refuses the files it would replace or merge. Tests: `tests/test_installation.py` (`SyncWithoutDescriptorsTest`: the swap on the fallback, every write reported, an install's merge refused; `SyncThroughStandInCallsTest`: the walk through injected calls in the handles' place, with `.cursor` moved and removed at the temporary file, and no open by path below the project), `tests/test_portable_fs.py` (`InstallerHandleCallsTests`, native Windows only: write, read, replace and times by handle, a junction refused, a held folder that cannot be renamed).
  - **An attached state that was a link took the launcher's writes, and the memory, through it.** `prepare_state` accepted `<state base>/attached/<project>` as a symbolic link and wrote `accelerator-attach.json` - and `claude_overlay` `launch/claude-system-prompt.md` - wherever it pointed, the project itself included; the runtime then laid out its Project Brain and Memory Bank there, and the prompt hook appended `memory-bank/local/refresh-health.ndjson` through it even when the runtime refused. The launcher now opens the base as named and `attached/` and the project's directory one inside the other without following a link (`portable_fs`: `O_NOFOLLOW` on POSIX, rooted handles on Windows), creates them owner-only, and reads and writes its record and the system prompt through the held directory's descriptor; a link at either directory, at `launch/` or at either file is refused (`AttachError`, so the Harness reports that the accelerator cannot be prepared), and `env` refuses it too. The runtime refuses a state directory - or its `project-brain`, `memory-bank`, `launch` or record - that is a link (`workspace_roots.linked_state_entry`, checked before anything is laid out), and the prompt hook appends nothing through one. Tests: `tests/test_accelerator_attach.py` (`LinkedStateTests`, and `run`/`env` on a linked state), `memory-bank/tests/test_attached.py` (a linked state and a linked memory bank refused, the prompt hook writing nothing through a linked state).

- **Second review of the unified memory branch: secrets, revised learnings, one source policy, and the stand.** Each finding was reproduced first; every regression test fails on the previous code.
  - HTTP credentials are secrets everywhere the shared patterns apply - write refusal, direct-query refusal, automatic-query cutting: `Authorization: Bearer|Token|Digest <token with a digit>`, `Authorization: Basic <base64>` and a bare `Bearer <20+ characters with a digit>`; placeholders (`YOUR_TOKEN`, `<token>`, `$TOKEN`) are not. No match in this repository or in four client projects' documents and memory.
  - A learning restated with a different consequence is no longer reported as already there: an agent revises what an agent attested (`updated`); what a person verified, or nobody said who, another owner's record and an archived one are left as recorded and reported as `differs`/`archived-differs`, and the result is not a replay. The Harness notice says what was saved, what was already in memory and what was kept as recorded.
  - Which paths a learning may cite - secrets, keys, environment files, dependencies, runtime state and derived memory refused - is one function, `source_path_problem`, in the module the runtime and the Harness share byte for byte; the Harness applies it before choosing a write path, so an older runtime's command chain no longer accepts what `record-result` refuses.
  - A damaged session-repeat record (a list where items belong, a turn that is not a number) is normalised away, costing at most one repeat instead of the turn's retrieval.
  - The stand: a document's own edit time decides whether it changed after the prompt, whatever its mtime; a skill counts as existing only where the edition under test installed it; a run that evaluated nothing exits 1.

- **Attached mode: first runs that start together, a private state, a relative state base.** Three findings of a second independent review, each reproduced first and covered by a regression test.
  - **A first run never takes a seed another one is still writing.** The runtime laid out an empty attached state without a lock and wrote each seed in place, so a second run started at the same moment - Claude Code runs the hooks of one event in parallel - could find `project-brain/config/runtime.json` present but half-written and fail with `Invalid Project Brain configuration`; a half-written seed index made `validate` report the state invalid. `ensure_attached_state` now lays out a state that lacks anything under the mutation lock, whose directory it makes first so the lock works in an empty state, and checks again once the lock is held; each seed is written beside its place and renamed into it (`atomic_write_bytes`, which `atomic_write` now uses). A complete state is still checked without the lock.
  - **The attached state is owner-only, whatever the umask.** Under umask 022 the launcher and the runtime left the state directory, its subdirectories, the launcher's marker and the SQLite index readable by every local user. `scripts/accelerator_attach.py` now creates the state directory and any parent it lacks 0700 and writes its marker and Claude's system prompt 0600, whole; the runtime makes its layout 0700, and an attached `context.py` run sets umask 077 for itself, so records, the index database and its journals, logs, receipts - and an `export` bundle - are 0600. A state from before the rule is tightened where it is the accelerator's - its own entries with everything under them, and the state directory when it holds nothing else - never through a link, never another user's file, and nothing above the state. POSIX only; an installed state keeps the project's permissions.
  - **A relative `--state-base` works.** `run` created the state relative to the directory it was started in, then exported the same relative `ACCELERATOR_STATE_DIR` from the project, which the runtime refuses, so memory did nothing for the whole session. `state_directory` now makes the base absolute from where the launcher runs, before anything is prepared or exported, for `run`, `env` and the default base alike.
- **The installer's writes survive a link swapped in mid-sync and a disk that fills mid-merge.** Two findings of a second review of `scripts/install_accelerator.py`, each reproduced first and with a regression test.
  - **A folder swapped for a link during a sync.** The sync looked at every folder of a path with `lstat` and then wrote by the path, so a process that could write to the project and replaced `.cursor` with a link between the look and the temporary file sent `.cursor/mcp.json` to the link's target; the sync's record went the same way, and a backup of a local edit aborted the whole sync. Every read, write, backup and the record now walk the path by folder descriptors (`openat` with `O_NOFOLLOW`, `mkdirat`, `fstatat`), create, write and rename the temporary file through the last folder's descriptor, and check before the rename that the path still leads to that folder: a folder moved or deleted and replaced by a link meanwhile leaves the file alone, removes the temporary file and is reported in `kept` with the link. The Harness's own put-back of the Codex wiring it could not get approved goes through the same call and records a failed put-back in `codex_trust.not_restored`. Native Windows has no descriptor-relative calls in `os.supports_dir_fd`; there the component checks stay as before, which refuse a link present when the sync starts but leave the moment between check and write open (comment in the installer; `scripts/README.md`, `harness/README.md`). A sync takes as long as before (Laravel with all three tools on btrfs: no-op 45 → 44 ms, a release re-adding 30 files 45 → 43 ms); only the files whose loss a crash could not undo - a backup of a locally edited runtime file, a merge into the project's own content - are flushed to disk first, about 3 ms each on btrfs.
  - **A merge interrupted part way.** `--merge-existing` wrote a merge over the project's file in place: a disk that filled after the first byte left `{` where a team's `.mcp.json` had been, and every later install refused with `cannot-merge:memory-mcp-configuration`. A file an install replaces (merges, `--overwrite` collisions) and every MCP configuration it writes are now written beside the file, flushed, and renamed over it with the replaced file's mode, as `mcp_config.apply()` does; an install that fails part way puts back, newest first, the files it had already replaced (`RESTORED`; `NOT_RESTORED` for one changed after the install wrote it, which stays) and exits 1. Files it created stay; a rerun finds them identical.
  - Tests: `tests/test_installation.py` (`SyncRaceTest`: `--sync` with `.cursor`, the backup folder and the record folder swapped at the temporary file, each moved and deleted; `SyncWithoutDescriptorsTest`: the Windows fallback on Linux; `InterruptedMergeTest`: a full disk in the `.mcp.json` merge, a failed install putting back four merged files with their modes and times, a file changed meanwhile left alone), `tests/test_harness_accelerators.py` (a swapped `.claude` and record folder through `keep_current`, the Codex wiring put-back).
- **A second review of the Harness: an update admits no run its restart would interrupt, a long Codex turn keeps its context numbers, and hook memory redraws.** Each finding was reproduced first and has a regression test.
  - **Update and restart.** An update counted the runs a restart would interrupt and then fetched and merged with nothing holding new ones back: a session created during the merge was queued, the update restarted the Harness, and the restart marked it interrupted. The count and a hold on new runs are now one step under the store lock every run is queued under (`Sessions.hold_runs`), and every way a run starts asks before it writes anything (`Sessions.admit`: a new session, a follow-up, a prepared context, a Fleet resume or decision, a Harness or target check, Infrastructure Creator, System Orchestration, AI discovery), so it is refused with a note to start it once the page has reloaded. The hold lasts through the restart, which now follows the reply even when the page went away; an update that fails or finds nothing to apply, or a server that cannot restart itself, lets runs start again (`Updates.resume`), and a second update while one waits for its restart is refused rather than lifting the hold.
  - **A Codex turn longer than the rollout tail.** Once a Codex launch ended, the Harness read the rollout's last 4 MiB and let that tail replace the fill the live reads had counted: a turn that started at 100,000 tokens, compacted and ended at 30,000 was stored as 30,000 / 30,000 / 30,000 over one call with no compaction. The live reader's fill is now the launch's own: once the launch has ended it reads on, and the tail only adds the lines past those reads, each known by its place in the file (`CodexLive.settle`, `providers.codex_rollout_end`), so no line counts twice and nothing counted is replaced. `context_usage.codex_fill` is gone.
  - **Usage › Context draws a hook-only change.** The view skips a poll that changed nothing it shows, but its key left out the hook memory, the ledger a memory follow-up adds to, the start, the peak and a compaction's numbers: a second hook's output, or a follow-up's prompt, kept the old numbers on screen. The view now keys on the launch fields it draws from, and a turn's column on everything the column draws (`contextColumnKey`).
  - Tests: `tests/test_harness_web.py` (a session and a follow-up sent while the update waits at a barrier between its busy check and its merge; an update that fails, finds nothing, or cannot restart; the Context view and column keys, in Node), `tests/test_harness_updates.py`, `tests/test_harness_sessions.py` (every way in refused while held, nothing written), `tests/test_harness_context_usage.py` (a launch whose rollout exceeds the tail, end to end; live reads that stop short of a 5 MiB rollout), `tests/test_harness_run_activity.py`.
- **An independent review of the unified memory branch: the write path, the MCP boundary and the stand.** Each finding was reproduced first and has a regression test.
  - A quoted credential goes from an automatic query whole: `password='alpha beta gamma'` used to leave `beta gamma'`, which reached the automatic task goal stored in Git and the retrieval manifest (`QUOTED_CREDENTIAL`, cut to the closing quote or the end of the line).
  - `record-result` resolves a learning's sources in the project the way fingerprints do (`workspace_roots`), so an attached project can record knowledge about its own files; it refused every one.
  - A result's keys - its digest, its receipt, its learnings' IDs - come from the task's UUID, so the external ID and the UUID are one namespace; a replay through the other name wrote a second copy and bypassed the different-content check.
  - An archived learning is found again (`include_archive`) and never written a second time or reopened; a stale revision now refuses any new result, learnings alone included; a replay runs promotion again, so an attempt interrupted after its records reached the Memory Bank all the same; promoting an archived agent-attested record keeps its `agent-attested` tag.
  - The public writer refuses progress, steps and learnings that open like a pasted log or transcript (`stderr:`, `user:`), as the query path cuts them.
  - The Memory MCP answers a message nested past the parser's depth with a parse error and stays up; it used to exit.
  - The stand's as-of corpus leaves out a working-tree memory document edited after the prompt (`--updated-after drop`, the default; `keep` marks the turn contaminated), and skills count in the could-answer ceiling as they do in answer-in-text.
- **The capsule quotes by the index's own words, and counts as handed only what it showed.** Three findings of a review of the unified memory runtime, each reproduced first; ranking and admission are unchanged.
  - **Excerpt terms are the index's terms.** The excerpt weighed a marked word by a suffix rule of its own while FTS5 had matched it with Porter: "classes" read as "class" but "class" as "clas", so the rare word that answered a question about classes took the lowest weight and a section sharing two common words with the request was quoted instead - likewise policy/policies, business/businesses, stop/stopped. Request words and marked words are now both read by the index's tokenizer, through an in-memory FTS5 table's vocabulary (`term_forms`; `TOKENIZER` moves to `context_retrieval.py`); a SQLite without `fts5vocab` reads both casefolded.
  - **A recorded episode is not handed twice.** Local episodes took no part in a conversation's repeat record: two identical `refresh --session-id` turns both handed the same episode and counted no repeat. An episode is now recorded by its id at the hash of its content, in the same record and with the same compaction reset as documents, so an edited one is handed again; the manifest excludes a repeat as `episode:<id>`, `delivered-this-session`.
  - **Only what the capsule shows counts as handed.** `retrieve()` wrote the conversation's record and the manifest before the 8,000-character JSON contract and the 3,600-character rendered text dropped what did not fit, so an item the model never saw was left out of the next turns as one that "still applies" and counted as delivered in `retrieval-stats`. `retrieve(pack=...)` now packs the capsule first - the contract, and for a rendered capsule the text's ceiling (`shown_in_render`) - and writes the record, the manifest's selection and the token estimates from what it shows; the rest is excluded as `capsule-limit`, counted in `omitted`, and handed on a later turn. A `refresh` with a query and a printed `context`, `retrieve` or `hook-context` capsule count their rendered text, `--json` its JSON, and the JSON of a `refresh` lists the items its `capsule_text` shows.
  - Tests: `project-brain/tests/test_runtime.py` (`DeliveryTest`: a plural and its singular weigh the same; a recorded episode once per revision, and again after compaction; an item left out by the JSON contract or by the rendered text is handed on the next turn; a capsule keeps only the items its text shows).
- **The Harness measures a run's memory on what the prompt carried and what the run was seen to read.** "M opened by the agent" matched the delivered items against the receipt's list of opened paths, which stops at 40 and takes a Read at its start: a delivered file read after 40 others counted as unopened, a Read that failed counted as opened, and a Codex run that read the file with `sed` - Codex reads only through its shell - reported 0. The launch's ledger now matches each read that succeeded against the delivered memory itself (`memory_draft.watch`: its path, any tool's copy of a skill, a Skill load by name), however many paths the run view lists, and the receipt keeps the counts (`memory`). Shell commands are never parsed for what they read: when they, Codex helper agents or, for an attached accelerator's memory, reads outside the project also ran, the line says "at least N opened"; when they were the run's only way to read, "opening unknown", `opened_delivered_sources` is null and the event's `opened_observation` says which. Usage › Context counted the capsule JSON the prompt no longer carries - 2,383 characters of memory parts on a real unattended turn that inserted 463; the parts now come from the rendered text's item lines (a chunk is Memory bank, a Brain record or episode Project Brain, any other path rules and docs, each with the excerpt under it), through one parser (`context_usage.capsule_text_parts`) that also reads a Claude hook's output, which had charged every current item line to Project Brain. A prompt that carries JSON (a reviewed session, a runtime that renders no text) keeps the JSON parts. Tests: `tests/test_harness_run_activity.py`, `tests/test_harness_memory_draft.py`, `tests/test_harness_context_usage.py` (the PHP Core runtime's own `render_capsule_lines`), `tests/test_harness_task_context.py` (real refresh; Claude and Codex launches).
- **An attached project is one project in the browser and in the terminal, and what the composer lists for it is what a message reaches.** The browser registered a folder in the spelling it was given, so `//srv/shop` - POSIX lets `//` name the root - became a second project beside `/srv/shop`, with its own ID and attached state directory, while `scripts/accelerator_attach.py` resolved both to `/srv/shop`: the folder's Project Brain and Memory Bank split by interface and spelling, and the `//` registration could not start a session. Both now take the ID from `accelerator_attach.canonical_project` (`~`, symbolic links, `.`, `..` and repeated or trailing separators resolved), and the browser stores that spelling; it still refuses `..` and links, which the terminal resolves to the same folder and state. On Windows the ID comes from the resolved path as well, as the terminal's always did, while the stored path stays as given. A registration saved before keeps its ID. The composer listed an attached edition's Claude Code commands (its probe passes the edition's `--add-dir`), Codex skills and Cursor skills, but routing read the catalogue without them: a listed Claude Code command went as plain text with the Harness context, and a Codex `$skill` or Cursor `/skill` got no request. Listing and routing now share one catalogue (`Catalog.catalogue`). Tests: `tests/test_accelerator_attach.py` (`ProjectIdentityTests`), `tests/test_harness_accelerators.py` (every spelling against the terminal's state), `tests/test_harness_commands.py` (`AttachedCatalogTests`: routing for Claude Code, Codex and Cursor).
- **Keeping an installed project current stays inside the project, and its memory server follows the release beside a team's servers.** Two review findings on `scripts/install_accelerator.py --sync`, which the Harness runs by itself when it opens a project.
  - **Symbolic links.** Only a path's last component was checked. A link standing in for `.cursor` took the release's `.cursor/README.md` outside the project; one for `memory-bank` or `memory-bank/local` replaced a runtime file outside it and wrote the backup of its local edit and the sync's record there. Every read, write and backup, and the record, now walk the path from the project one folder at a time without following links (`lstat`; Windows junctions too), create missing folders one level at a time and write through an exclusively created temporary file. A path a link or a non-folder stands on is left alone and named in `kept` with the link; a runtime edit whose backup cannot stay inside is not replaced; a runtime reached only through a link is an error, as the Harness, which reads it without following links, already treated it. The Harness reads and restores `.codex/hooks.json` with the same helpers, so wiring behind a link is neither rewired nor approved. The probes over whole inventories look at each folder once, so a sync takes as long as before.
  - **MCP configurations.** Install merges `.mcp.json`, `.cursor/mcp.json` and the managed block of `.codex/config.toml`, so a project that keeps its own server never held the release's bytes, and sync reported the file as edited forever: a launcher fix never reached it. Sync now merges with the release's `mcp_config.merge`. An untouched or missing file gets the release's file as this machine's install writes it; a file holding the project's servers gets only the memory server's entry or block replaced ("memory server updated") and is not recorded as an untouched release, so the next release merges into it again instead of replacing it. A malformed configuration, another server under the memory server's name, an edited managed block, or a key appended after the block (TOML of the memory server's own table) is kept with the merger's reason; a Codex configuration without the managed block stays the project's, as install treats it.
  - Install no longer fails with a `TypeError` when the Python probe fails for one MCP configuration and the next is merged without an interpreter.
  - Tests: `tests/test_installation.py` (`InstallSyncTest`: a linked `.cursor`, a linked `memory-bank`, backups and the record behind a linked `memory-bank/local`, each as a dry run and a real one; `McpConfigSyncTest`: two releases of the memory server beside a team's server for Claude, Cursor and Codex, registration into a team-only configuration, refusals), `tests/test_harness_accelerators.py` (the same links through `keep_current` and `_sync_installed`; Codex wiring behind a link).

- **The Harness counts the delivered memory the agent opened.** After a run it compares the capsule's items with the files the run's own tools read (the launch receipt's `opened_paths`; a skill counts whichever tool's copy was read) and says "N delivered, M opened by the agent" in the conversation, beside the agent's own `used_memory` report when there is one; the memory event carries `opened_delivered_sources`. Delivery alone said nothing - pointers handed to agents on real installations were opened once in 290 deliveries - and a report is the agent's word; opening is observed. Neither proves a claim was used, and the notice says so. Tests: `tests/test_harness_memory_draft.py`, `tests/test_harness_task_context.py`.

- **One replayable write path for a run's result.** `memory-bank/scripts/memory_results.py` (`record_result`) is what the Memory MCP's `memory_record_result`, the Harness's save at the end of a run and the new `context.py record-result` all call. A learning's record is named by its content (task, type, title, sources): a retry, a replay after a crash between writes, or a later result restating the learning finds that record - and closes it if an earlier attempt stopped half way, leaving one that has moved on since (under investigation, superseded) as it is - instead of writing a second one. The Harness's chain of commands drew a random ID per attempt and wrote a duplicate after any partial failure; the MCP wrote two shared events per save and, after a partial one, refused every retry until a person inspected Project Brain. The task update goes last: a local receipt (`project-brain/local/results/`), or the task already holding the result's progress and steps, marks a replay, and the same result ID with other content is refused. A stale revision is refused before anything is written; the Harness re-reads the task once and replays. A project whose runtime predates `record-result` still gets the Harness's previous chain.
  - **Who checked it.** An unattended save's verification carries `[attestation:agent]` in its ledger entry (`record_attestation`; older unattended saves are recognised by their "agent-attested" reason), a chunk promoted from such a record is tagged `agent-attested`, the index keeps the attestation (`document_metadata.attestation`, one full re-read when the column appears), and the capsule marks the item "agent-attested, not reviewed by a person". No record field was added, so a runtime that predates it still accepts the record.
  - **A rare term in a small index.** A term in one document always counts as rare now: below ten documents the 10% share rounded under one, and an identifier only one document carried could not admit it. Real installations index about a hundred documents, mostly skills, so this matters where skills are missing from the index.
  - Tests: `project-brain/tests/test_runtime.py` (`RecordResultTest`, `SmallIndexRarityTest`), `tests/test_memory_mcp.py` (a half-finished save finished by its replay; a stale revision writes nothing), `tests/test_harness_task_context.py` (a repeated save writes each learning once; a task moved on by a checkpoint; a runtime without `record-result`).
- **Memory retrieval is measured on each project as it was when the prompt was written.** Earlier counts on real prompts indexed every project's current state, so knowledge written after a prompt - often about that prompt's own work - was credited to the capsule, three to four times over. `scripts/memory_eval.py` is the stand to measure every retrieval change on: repository-level, never installed, no network, standard library and Python 3.9+.
  - `run`, per prompt and without writing into the project: the newest commit at or before the prompt, read blob by blob (`git archive` would apply `export-ignore`); Brain records, handoffs and chunks created later dropped, and records and chunks that existed uncommitted added - by `created_at`, by the chunk's name date narrowed by the promotion that wrote it, or by a modification time before the prompt; the edition under test overlaid from this clone's inventory the way the installer copies it, without replacing project-owned state (`--edition auto`, the default, picks it per project from the install manifest, the policy lock or the framework markers attached mode reads); the corpus committed on the project's own history, borrowed from a clone in the cache because Git would touch the project's objects; the runtime's clock pinned to the prompt, so review dates and recency are the prompt's; `context.py index`, then the hook's `refresh --sanitize --ephemeral --json`, timed. `--as-of now` keeps the old measurement for comparison.
  - The score per prompt: delivered, useful, noise and unjudged paths; the judged-useful documents that existed; whether a judged answer passage is in the capsule text; the turn class (useful, noise-only, unjudged-only, silent). Results carry ids, paths and counts, never prompt, capsule or document text; a stderr line quoting the prompt is withheld.
  - `report` prints useful turns, useful among could-help, answer in the capsule text, noise-only and silent turns, mean items and latency p50/p95, side by side with a paired per-prompt diff under `--compare`, and lists unjudged deliveries under `--show-unjudged`. `realized` counts, from Claude Code transcripts and Codex rollouts, the delivered items agents touched or mentioned, by kind, their own retrieve calls, and the documents they opened that were not delivered.
  - `docs/MEMORY-EVAL.md`; `tests/test_memory_eval.py` (installation job) runs one real PHP Core refresh on a synthetic project whose files and `.git` must come out untouched.

- **The two memory branches are one.** `fix/memory-reaches-projects` and the Memory MCP work had each built the same capsule changes in their own code; this keeps one of each.
  - Prompt sanitizing (`memory-bank/scripts/automatic_query.py`, shared byte for byte with the Harness): secrets, personal data and transcript role prefixes are cut out and the words around them stay - refusing the whole prompt cost 35 of 60 first prompts on one real project their memory - and a private key goes with its body, not only its header line. A labelled name or address still takes the rest of the text with it, and a prompt with nothing technical left gets no capsule instead of one retrieved on the task goal or the branch name (`refresh --sanitize`, `query_withheld`).
  - The relative score floor stays where `fix/memory-reaches-projects` measured it, before the delivery filters. The MCP side had moved it after them, so that a task's own record or a host-loaded file would not set the bar; on the as-of-date stand (below) that delivered one more noise-only turn in each set and nothing more useful, so it was not kept.
  - The rendered capsule keeps the quoting windows and the section labels, and adds the MCP side's bounds - task, goal, warnings, last turn and paths are capped, so working state cannot crowd out the ceiling - and screens excerpts for personal data like the query.
  - `validate.py` finds `automatic_query.py` when it is loaded by path (the orchestration library does), and says the module is missing instead of failing on import. Generated projects must carry `automatic_query.py`, `mcp_server.py` and `mcp_config.py` (`bootstrap-verifier`).
  - The Cursor read hook stands down during the Harness's memory recovery, like the Claude and Codex hooks.

- **Memory MCP for Claude Code, Codex and Cursor.** A dependency-free, project-scoped server any of the three clients can call to retrieve, checkpoint and record results, registered by the installer and the generator; the Harness recovers a missing memory draft once.
  - Require targeted memory retrieval after compaction/scope changes and source-backed result recording at authorized work boundaries across the four editions and generated policy.
  - Register Memory MCP automatically for selected clients during installation/generation, preserving other servers, refusing foreign name conflicts, detecting Python (including native Windows), and using portable launchers across nested directories/worktrees. Harness setup previews and publishes these supported merges.
  - Bundle a dependency-free, project-scoped Memory MCP for Claude Code, Codex and Cursor, with warming/source-linked retrieval, first-turn task provisioning, revision-checked result recording (replayable, see "One replayable write path" above), source/actor boundaries, and existing automatic promotion.
  - Accept native MCP request metadata during tool discovery and calls, while refusing malformed metadata; verify discovery and direct calls against the actual Codex app-server.
  - Recover a missing/invalid Harness memory draft once through a read-only continuation of the same native session, within shared budgets. Preserve cancellation and native identity, suppress memory hooks during recovery, include its usage/context counts, and distinguish delivered pointers from agent-reported use.
  - Add protocol, persistence, recovery and isolation regressions and per-client connection instructions in docs/MEMORY-INTEGRATION.md.

- **The capsule quotes the passage that answers, a rare word alone no longer admits a document, and repeats are counted by section.** From a review of the runtime, measured on the 61 graded real first prompts (used for tuning) and confirmed once on 60 new ones judged blind (holdout). For both, the passages that carry each useful document's value were labelled, so "the delivered text holds the answer" is measured, not assumed.
  - **Excerpts.** A document's matches are marked by FTS5 `highlight()` with the index's own tokenizer - a request about a "session" now finds the section that says "Sessions" - and weighted by their rarity in the index. The section with the most weight is quoted, from a window of whole sentences that starts where the matches are instead of from the section's top. List items are split into sentences, since a changelog entry runs to a kilobyte. A list item keeps the line that introduces it. Windows are 800/600/400 characters by rank and 600 for a history item (were 600/400/250). The item line names the section: `- memory specs/auth.md § Sessions — Auth`. Tuning set: the delivered text held an answer passage for 20 of 36 delivered useful documents instead of 12, in 14 turns instead of 11. Holdout: 19 turns instead of 16 (7 gained, 4 lost), 21 of 51 documents instead of 17 of 53. The capsule grew from 960 to 1,168 characters on average (holdout), about 50 tokens a turn.
  - **Admission.** A document matched by a single rare term is admitted only when that term is an anchor: shaped like an identifier (a digit, `CamelCase`, `snake_case`) or a word of the document's own path or title (`is_anchor`). In an index of about a hundred documents, a word in ten of them counts as rare, and without an anchor such matches were noise in six of seven judged cases. Skills keep the old rule, because a capsule never delivers a weak skill and explicit retrieval and the routing floor rank them. Tuning set: 14 fewer delivered items (172), no useful one lost. Holdout: 7 fewer (145), 5 of them noise and 2 useful.
  - **Repeats.** A conversation is not handed the same section of a document again within four turns. The key is `path#section`, and the revision is the section's hash, so a later question answered by another section still gets that section. `refresh --transcript` takes the transcript path the host gives the hook (`transcript_path`; the read hook passes it with `--session-id`). A Claude Code `compact_boundary` record or a Codex `compacted` record written there since the last turn resets what counts as handed, because after compaction the conversation holds only a summary. The Cursor rule, which is replaced only when a prompt brings an item it does not hold, now compares sections.
  - **Unchanged, and why.** Useful turns stayed 33 of 61 and 39 of 60, and turns delivering only noise stayed at 7 → 8 and 12 → 12. Admission is not the lever for them: on the tuning set, 5 of the 7 noise-only turns had a useful document in the pool that ranking did not deliver. That is a ranking problem, for a reranker or hybrid retrieval to be measured against. Russian prompts against English documents were 1 of 385 real first prompts, so no translation dictionary was added.
  - Tests: `project-brain/tests/test_runtime.py` (`DeliveryTest`: the answering stretch, words matched as the index matches them, another section of a handed document, a compacted conversation, a rare word admitting a document only when it names something), `memory-bank/tests/test_hooks.py` (the transcript reaches the CLI, the Cursor rule updates on a new section).

- **Memory reaches the projects and says something when it speaks.** Measured on six real installations: none carried the previous week's fixes, about 98% of turns went through the native Claude/Codex hooks rather than the Harness, the hook capsule named paths and titles and never the answer, ignore rules had removed every skill from three indexes, 68 of 70 resolved findings in one project had never become durable memory, and the capsule was never empty.
  - **Installed projects follow the clone.** `scripts/install_accelerator.py --sync --target <project>` (and `sync_installation()`) brings an installed accelerator up to the clone without a question: files nobody edited (the bytes of some released version, or what the last sync wrote, recorded in `memory-bank/local/accelerator-install.json`), files a release added to a component the project has, and the managed blocks of `AGENTS.md` and `.claude/CLAUDE.md`. The runtime (`memory-bank/scripts`, `project-brain/scripts`, `project-brain/schemas`, the working-memory hooks) also replaces a local edit, which is backed up under `memory-bank/local/accelerator-sync/`. Never written: anything the project's Git tracks, seeded state, and - except for a caller that approves it again (below) - `.codex/hooks.json`, whose hook trust is a hash of its definitions; each is reported. The Harness runs it for every registered installed project at start, on registration and before a session's memory, once per clone version, and says in the conversation when a project was updated (`Accelerators.keep_current`, table `accelerator_syncs`). `tests/test_installation.py::InstallSyncTest`, `tests/test_harness_accelerators.py::InstalledAcceleratorSyncTests`.
  - **The capsule carries the answer.** The rendered capsule - what every host reads - gives each project-knowledge item the heading section of the document that shares most terms with the request (600/400/250 characters by rank), puts project knowledge first and at most one skill (a strong match, by name; hosts load their own skill catalogue, and agents opened 0 of 148 skill pointers), marks weak matches and a cited file changed since verification on the item, and stays under 3,600 characters, below Codex's 4,000. `refresh --json` adds `capsule_text`; the Harness inserts it instead of the JSON, which held every item three times. A promoted chunk and the record it came from no longer take two slots (`promoted-to-chunk`). The read hook prints no banner and passes the conversation's `session_id`; a conversation is not handed the same item again for 4 turns, and the capsule says how many earlier items still apply (`refresh --session-id`). On a per-turn refresh the layer and timing report prints only when something is wrong or slow. The delivered-capsule contract is 1/3/1 (PROTOCOL.md, `project-brain` skill, docs).
  - **Pasted logs and addresses no longer cost the turn its memory.** The hooks and the Harness pass `refresh --sanitize`: secrets, personal data and transcript-style prefixes (`stderr:`, `user:`) are cut out of the prompt instead of the whole prompt being refused - 35 of 60 real first prompts on one project used to get no memory for that reason. A query a person passes without the flag is still refused whole.
  - **The capsule can be silent.** Function words and conversation ("thanks", "ok", "looks good", "спасибо") never count as evidence of relevance (`EVIDENCE_STOPWORDS`), and a candidate under 30% of the best score in its layer is left out (`RELATIVE_SCORE_FLOOR`). Retrieval bench (Symfony / PHP Core): chit-chat turns injecting anything .81 -> .52/.67, irrelevant developer turns .84 -> .75; ranking (hit@1/hit@2) unchanged, non-skill documents reaching the capsule .40/.50 -> .60/.70, skills reaching it lower with one slot (golden-en .71/.78 -> .59/.54). Replayed against copies of five real installations (61 real first prompts with graded relevance), turns with a useful item rose from 29 to 33, useful items from 36 to 45, and every one now arrives with its text.
  - **Accelerator sources survive ignore rules.** Skills, `AGENTS.md`, `memory-bank/chunks/`, `specs/` and `tasks/` are indexed even when the project ignores them; a folder that is not a repository has no ignore rules instead of failing every refresh. On copies of two real installations the index went from 12 and 18 documents to 120 and 116.
  - **Written knowledge stays.** A record reaching a promotable state is promoted by the `brain-update` that resolves it, while its citations still say what was verified. A cited file edited afterwards no longer evicts a chunk or a record: it is served marked `source_changed` at half its score, and leaves only when the file is gone (`source-missing`). Automatic promotion of a record whose citation changed after verification proceeds, and the chunk keeps the digests taken at verification. `validate` lists such records as warnings instead of failing. Turns on a branch whose task was completed are dropped with a note instead of failing every flush, which had stopped promotion and compaction.
  - **Claude can use its memory.** Every edition's `.claude/settings.json` allows `python3 memory-bank/scripts/context.py`, which `-p` sessions refused before; the sync above delivers `.claude/CLAUDE.md` (`@../AGENTS.md`) to installations made without it.
  - Windows: the runtime prints UTF-8 whatever the console code page, and the Harness runs it with `PYTHONUTF8=1`; the first '…' used to fail every capsule there.
  - **Codex runs the memory hooks and reads the whole policy.** Codex runs a project hook only after its definition was approved, and on two of five real installations nobody had approved them: no memory hook ran in 0 of 39 sessions of one project. With Codex available, the Harness approves the accelerator's own hook definitions - the event and command the edition ships, running a script byte-identical to the clone's - in the user's Codex config, the record Codex's own `/hooks` review writes (`accelerator_attach.trust_installed_codex_hooks`; for an attached project, the hooks its launches lend). A team's hook, an edited definition or an edited script is left for review, and `HARNESS_CODEX_HOOK_TRUST=0` turns the approval off. Because it approves in the same pass, the Harness sync rewrites an untouched `.codex/hooks.json` (`sync_installation(rewire_codex=True)`; from a terminal `install_accelerator.py --sync --rewire-codex`, then approve in `/hooks`), and puts the approved wiring back when Codex cannot approve the new one. `.codex/config.toml` is no longer treated as trust-bound and follows the release: it sets `project_doc_max_bytes = 131072`, and the Harness passes the same for a launch whose `AGENTS.md` exceeds Codex's 32 KiB default - past it, Codex cut the accelerator's policy block, which comes last.
  - **Cursor gets the capsule for the prompt it is about to answer.** Cursor's `beforeSubmitPrompt` can only let a prompt through or stop it, and the context `sessionStart` returns is often dropped on the first message, so Cursor saw the working state of the last stop and nothing retrieved for the request. The new `.cursor/hooks/working-memory-read.sh`, generated from the Claude read hook and wired on `beforeSubmitPrompt`, retrieves for the prompt and renders `capsule_text` into the `alwaysApply` rule `.cursor/rules/working-memory.mdc` before the request leaves; it always answers `{"continue": true}` and never passes `--session-id`, because the rule is re-sent whole with every request. Cursor puts rules at the start of the model's context, where a change costs the conversation its cached prefix, so the rule is replaced only when a prompt retrieved an item it does not hold yet. The stop hook's branch render leaves a rule the prompt hook wrote for the same task, so when Cursor reads its rules before the hook runs, a request carries the previous prompt's capsule rather than one rebuilt from the task alone - which of the two orders Cursor uses has not been verified on a live client. Cursor also runs the Claude Code hooks it finds: under a Cursor payload (`cursor_version`) the Claude copies of the read and write hooks stand down where the project has its own `.cursor/hooks.json`, instead of every turn being retrieved and checkpointed twice. The Cursor stop and session renders skip a turn whose capsule the Harness delivered (`CONTEXT_CAPSULE_DELIVERED=1`). `.cursor/cli.json` allows `python3 memory-bank/scripts/context.py` in `cursor-agent`. `cursor-agent --print` fires no stop hook, so the Harness runs the turn checkpoint after a Cursor run unless a stop hook wrote one during it (`TaskContext.checkpoint`), and it no longer resends excerpts of the `AGENTS.md` and `CLAUDE.md` the Cursor CLI loads by itself. `memory-bank/tests/test_hooks.py` (`CursorPromptHookTest`), `tests/test_harness_accelerators.py` (`InstalledCodexHookTrustTests`, `InstalledCodexSyncTests`, `AttachedCodexHookTrustTests`), `tests/test_harness_providers.py::CodexInstructionBudgetTests`, `tests/test_harness_task_context.py`, `tests/test_installation.py::InstallSyncTest`. `memory-bank/tests/test_parity.py` now requires the Cursor mirror of the read hook, and `test_memory.py` holds `AGENTS.md` to the delivered contract (one Procedural, three Semantic, one Episodic, 3,600 characters).

- **Use the accelerator in a project without copying anything into it (attached mode).** Clone this repository once and choose the project folder in the Harness's **Sessions** (as DeepSeek Harness chooses a workspace), or run `scripts/accelerator_attach.py run claude|codex|cursor` in the project: the edition the project's files point to is lent from the clone for the session, and `git pull` updates every attached project. See [docs/ATTACHED-MODE.md](docs/ATTACHED-MODE.md).
  - Core: `memory-bank/scripts/workspace_roots.py` separates the tooling, the project and the accelerator's state, which used to be one root. Installed, they are still one directory and every key and path is unchanged. Attached (`ACCELERATOR_HOME`, `ACCELERATOR_STATE_DIR`, `ACCELERATOR_PROJECT_DIR`, binding only the runtime copy they name), policy and skills are indexed in place from the clone under absolute keys, project documents, citations and Git come from the project, and Project Brain, the Memory Bank and the index live in the state directory, which the runtime lays out on first use. `status --json` reports the layout. `validate.py` still runs copied on its own.
  - Hooks keep `ROOT_DIR` as the tooling root and add `PROJECT_DIR` and `STATE_DIR`: task IDs and write-agent locks follow the project's branch, refresh-health and `msg-dispatch` write the state, `local-context.sh` describes the project and names the clone and the state, and Cursor's working-memory rule is not written into a shared clone (the `_WM_DELIVERY_*_CURSOR` mirror blocks).
  - `scripts/accelerator_attach.py`: edition detection (`composer.json`/`composer.lock`, `artisan`, `bin/console`, WordPress markers), the per-project state directory shared with the Harness, and the per-tool overlay - Claude Code `--add-dir` + `--append-system-prompt-file` + merged `--settings` (hooks with absolute paths, edits of the clone denied), Codex `developer_instructions` (policy and skill catalogue) + `-c hooks.*` + `--add-dir <state>`, Cursor `--plugin-dir` - plus `trust-codex-hooks`, which records the hook approvals Codex requires.
  - Harness: the project is chosen in **Sessions** - the **Project** chip, **Choose a project folder** on an empty session, or the last entry of the sidebar **Project** selector - and **Projects & Setup** is gone (installing files into a project moves to **Accelerators › Install into project**). A chosen folder is registered and attached automatically when it has no accelerator of its own; started from the clone without `--project`, the server registers no project and asks for one. Native sessions, Clash and Fleet apply the overlay; the `/` and `$` menus list the edition's commands and skills; Knowledge, Memory use and session memory read the attached state with the clone's runtime; the **Project** chip shows, switches and detaches the attachment and offers the Codex hook approval. New `tests/test_harness_accelerators.py` and `tests/test_accelerator_attach.py` (CI `installation` job and `scripts/check.py`).
  - `scripts/install_accelerator.py` excludes the attach-only `.cursor-plugin/` from installs; inventories regenerated (`workspace_roots.py` ships with the runtime).
- **Install the accelerator as a desktop application.** `./accelerator-app install`, run once in the clone (in Git Bash on Windows), puts **AI Accelerator**, with a hare icon, among the installed applications. On Linux it writes an XDG desktop entry and hicolor icons, on macOS `~/Applications/AI Accelerator.app`, and on Windows a Start menu shortcut plus an entry under **Settings › Apps › Installed apps** with Uninstall. A click starts the Harness from the clone if it is not running and opens it in the browser. The entry runs the clone, so `git pull` updates the application. A desktop launch lacks the shell profile's `PATH`, where the agent CLIs and often Python itself are. So the entry names the Python that ran `install` (the launcher falls back to `python3` and `python`), `install` records the shell's `PATH`, and `open` puts that `PATH` first. A failed start shows as a desktop notification (a message box on Windows) and in `launcher.log`. `open`, `status`, `stop` and `uninstall` complete the command, and uninstalling keeps projects and memory.
  - `harness/src/harness/desktop_app.py` (standard library) builds the Windows ICO and macOS ICNS from the PNGs at install time. The root launcher `accelerator-app` joins the lint and file-mode gates.
  - The icon is `harness/web/icons/ai-accelerator.svg`, with PNG sizes rendered from it. The Harness serves it as the browser tab icon and the sidebar and welcome logo; run-state badges are drawn over the hare.
  - New `tests/test_desktop_app.py` runs in CI's `installation` job and in `scripts/check.py`. On `windows-harness` it also creates a real Start menu shortcut with PowerShell, in a temporary folder.
- **The application updates itself, as desktop applications do.** The server checks the branch the clone follows (`main` on origin for a user's clone) at start, hourly, and when the page comes back, with one `git fetch`. While the clone lacks commits, an **Update** button sits at the right of the page header. A click:
  1. fast-forwards the clone;
  2. rewrites the installed desktop application from it;
  3. restarts the server on the new code through a `relaunch` process, which starts the same `serve` command once the old server has let go of its state directory;
  4. reloads the page when the new server answers.

  Only a fast-forward is applied. A clone with commits of its own, a local change in the way, or runs in progress hold the update back and say why. `./accelerator-app update` and `harness-server update` do the same from a terminal, and `HARNESS_UPDATE_CHECK=0` turns the checks off. The new module is `harness/src/harness/updates.py`, with `tests/test_harness_updates.py` against a local remote.
- **No reinstall after installing a CLI or moving the clone.** A desktop launch now reads the `PATH` the login shell sets up (`$SHELL -ilc`, as VS Code does; bash, zsh and fish) before it starts the server, so a CLI installed since `install` is found. The `PATH` `install` recorded remains the fallback. A clone started from a new folder after the old one is gone takes the installed application with it. The install message no longer asks to run `install` again.
- **`@` file mentions and terminal-only commands in the composer.** The Harness runs the CLIs without their terminals (`claude -p`, `codex exec`, `cursor-agent --print`), so `@` opened nothing. `/` lacked every command print mode refuses: Claude Code lists 60 commands there, and none of `/status`, `/resume`, `/memory` or `/help`; its `/cost` is an alias of its own `/usage`.
  - `@` where a word starts now searches the workspace's files and folders, in every workflow and for every provider. Inside Git the search covers what Git lists (tracked, plus untracked files that are not ignored); outside Git it walks the tree, skipping dependencies. Only names are read.
  - Tab or Enter inserts `@path`; a folder stays open on what is inside it. The new module is `harness/src/harness/mentions.py`, served by `GET /api/projects/<id>/files` and `GET /api/sessions/<id>/files`, which need the page token.
  - `/status`, `/cost`, `/diff`, `/resume`, `/memory`, `/login` and `/help` now open the matching Harness view in every CLI: Usage, Changes, the session list, Knowledge, the sign-in state, or the command list. A command or alias of the same name from the CLI or the project wins: Claude Code's `/cost` stays its `/usage`, and an accelerator's `/memory` still reaches the CLI.
  - Their answers appear in a status line under the message field. New `tests/test_harness_mentions.py` (CI `installation` job, `scripts/check.py`), plus composer and route tests in `tests/test_harness_web.py` and `tests/test_harness_commands.py`.
- **The conversation has no Record result & durable memory panel.** It sat under every linked session's output, including during the run, and with automatic memory every session is linked. Its markup, about 150 lines of script and its styles are gone.
  - What it did: a session opened for review now saves its run's memory draft when the run completes, like every other session. Every launch gets the same 825-character instruction, and the review checkbox reads **Review context by hand**.
  - A held-back learning is reported as staying in Project Brain. Task completion and records by hand are in **Knowledge › Project Brain**, and manual Memory Bank proposals use `context.py promote-propose`/`promote-review`/`promote-apply`.
  - The session API keeps its memory-save and record routes.
- **The Harness says which provider is not signed in, and how to sign it in.** A first session with the default provider failed with the CLI's "Failed to authenticate: OAuth session expired and could not be refreshed" followed by a general "Provider did not complete successfully". **Sessions** now asks each found CLI whether it is signed in, with no model call: `claude auth status --json`, `codex login status`, `cursor-agent status --format json`. These answers are cached for 30 seconds and asked again when the page regains focus.
  - A signed-out provider is listed as `… · not signed in`, and a note names its sign-in command (`claude auth login`, `codex login`, `cursor-agent login`, or the executable's path when it is not on `PATH`) with **Use <provider>** for one that is signed in.
  - A run the CLI refuses for its account ends with an error naming the provider and the command, instead of the general failure.
  - The check is advisory: a CLI using another backend or a key in the environment reports `unknown`, and no run is refused. The route `GET /api/providers/sign-in` needs the page token, since it starts native CLIs.
  - Tests are in `tests/test_harness_providers.py`, `tests/test_harness_sessions.py` (a fixture of the real expired-OAuth stream) and `tests/test_harness_web.py`.
- The edition policies and the `checkpoint`/`memory` skills describe the automatic memory as it runs (see each edition's CHANGELOG): `AGENTS.md` loses about 3.3 KB of contradictory memory rules per edition, `scripts/token_budget.json` tightens `agents_md_bytes` to the new sizes and raises PHP Core's `body_bytes` for the governed `checkpoint` workflow, and `memory-bank/tests/test_checkpoint.py`/`test_memory.py` assert the new contract.
- **A reinstall keeps the state the project owns.** `tasks/.task-counter`, `specs/MANIFEST.md`, `memory-bank/INDEX.md`, `project-brain/indexes/active.json`/`archive.json` and `project-brain/config/runtime.json` shipped as ordinary installer files: after a single `context.py start` a `--merge-existing` reinstall refused with a collision, and `--overwrite` reset the task counter to 1, so TASK-NNN numbers repeated and `project-brain validate` broke. `install_accelerator.py` now treats them as `SEED_ONLY_PATHS`: an existing copy is kept under every mode and reported as `KEPT` (configuration keys a newer release adds fall back to the runtime defaults). Harness Setup previews them as `kept`, outside the change count. `tests/test_installation.py` reinstalls over a used project in both modes and validates it; `tests/test_harness_setup.py` covers the preview.
- **`scripts/check_php_snippets.py` lints fragments, not only whole files.** It checked 93 of 363 fenced PHP blocks; the rest were skipped as fragments, and the Laravel and Symfony snippets that fatal on their declared versions were all among them. A fragment is now linted as statements behind an added `<?php` tag, then as class members, and raw only when it carries its own `<?php`/`<?=` tag (raw untagged text is inline HTML to `php -l`, which would pass anything). A block that is deliberately not valid PHP alone is fenced ` ```php fragment` and counted. 366 blocks are linted now, and the output gives counts per mode. New `tests/test_check_php_snippets.py`, run in CI's `lint` job, proves a broken untagged fragment fails. `tests/test_framework_semantics.py` adds `FrameworkApiCurrencyTest`, a denylist of the patterns the audit found (Filament `?string $navigationIcon`, `$this->authorize()` without `AuthorizesRequests`, an undefined `'private'` disk, claims that dispatch inside a transaction is deferred, `EventServiceProvider.php`/`#[AsListener]`, and Symfony voters without `?Vote $vote = null`).
- CI's `lint` job runs the Symfony `skill-creator` adapter tests (`.agents` and `.cursor` copies), which are excluded from installs and ran nowhere; `scripts/check.py` mirrors the step.
- **Prompt-time retrieval works before the branch's governed task exists.** In the default governed mode the task is created at the first checkpoint, after several file-changing turns, and a read-only session never gets one; until then every `UserPromptSubmit` capsule was a bare `Capsule unavailable: Working task not found` warning, although the runtime filters do not depend on a task. `retrieve()` now accepts no task: it keeps the privacy, owner, authority and freshness filters, excludes no own record, and writes its manifest to ignored local state only (the manifest schema's `task_id`/`task_revision` may be null; governed manifests still require a task). `refresh` uses it, and the capsule renders `working: not recorded yet (the task starts at the 5-turn checkpoint)` above the layers. An explicit `retrieve --task-id` still refuses a missing task. `msg-dispatch` provisions the branch task itself, so a subagent completion on a new branch is recorded instead of failing into `/dev/null` in `subagent-dispatch.sh`. The test that pinned `capsule is None` now asserts the layers.
- **Project Brain fingerprints and writes are line-ending neutral.** `fingerprint()` hashed raw bytes, so a colleague's `core.autocrlf=true` checkout of the same commit marked every record citing a text file `source fingerprint is stale` while Git saw no change. Text is now hashed with CRLF folded to LF (an LF file's digest is unchanged, so stored fingerprints need no migration; a NUL byte keeps binary data raw). Record, handoff, message and telemetry writes pass `newline="\n"`, so the runtime no longer writes CRLF into Git-tracked files on Windows. `LineEndingFingerprintTest` covers a CRLF checkout, the LF digest and binary sources.
- **LF everywhere, and a client `.gitattributes` that keeps hooks reviewable.** The repository had no line-ending policy: a clone with `core.autocrlf=true` got CRLF hooks, which bash refuses (`syntax error near unexpected token $'in\r'`), and the byte-comparing `build_mirrors --check` and `parity` reported 355 and 66 false drifts. A root `.gitattributes` pins `* text=auto eol=lf`, marks the binary types the repository tracks, and leaves client-supplied `*/Task/**` material as delivered (`-text`); every other tracked file was already LF. Installs no longer copy the edition's generated `.gitattributes`, which marks all 239 mirror paths `-diff` (13 hooks among them) and so showed an edited hook as "Binary files differ" in a client's review: `PRODUCTION_SOURCE_OVERRIDES` installs `<edition>/.install/gitattributes` instead, one `*.sh text eol=lf` directive merged additively into an existing file. `tests/test_file_modes.py` fails on any CRLF in the index outside `*/Task/`, and the installation tests assert the installed attributes. install/README.md says how to remove the old `-diff` lines from an earlier install.
- **The policy loads in Claude Code even when the project has its own `CLAUDE.md`.** Claude Code reads `AGENTS.md` by itself only while no `CLAUDE.md`, `.claude/CLAUDE.md` or `CLAUDE.local.md` exists, and only from v2.1.277; a project with any of them (Laravel Boost writes one, and the Laravel README recommends Boost) never saw `AGENTS.md`. Every edition now ships `.claude/CLAUDE.md` importing `@../AGENTS.md` (installer `claude` component; inventories and policy locks regenerated, and `.claude/CLAUDE.md` joins the policy-lock surface). `install_accelerator.py --merge-existing` appends the import as a managed block to an existing `.claude/CLAUDE.md` (`MANAGED_POLICY_FILES`), and without the flag reports the collision. In the shared core, `host_loaded_paths()` knows all three Claude instruction files, follows `@` imports from each of them, and counts `AGENTS.md` as host-loaded when none exists, so the capsule stops spending a procedural slot on a policy the host already loaded. `docs/TOOL-INTEGRATIONS.md` adds the `/context` → **Memory files** check.
- `context.py parity` passes on single-tool installs. `--tool claude` and `--tool cursor` ship no `.agents/skills/`, so every skill was reported as "absent from canonical" (85 paths in Laravel), and the `.cursor/README.md` and `.codex/README.md` that every install carries made those directories look like installed mirrors missing all their governance documents. The check the project-brain skill tells agents to run failed on every such install, and TROUBLESHOOTING forbade the obvious workaround of editing `canonical_edition`. `effective_canonical_edition()` in `context_retrieval.py` now compares against the first installed tree when the configured one is absent, and parity names the stand-in (`.claude canonical; configured .agents is not installed here`); `full_mirror_drift` skips a class for a mirror directory that holds none of its files, while a mirror that holds some of them still reports the rest. `tests/test_installation.py` runs `parity` in every tool selection's smoke test, and `memory-bank/tests/test_parity.py` covers both install shapes and that partial drift is still caught. ADOPTION §8 and TROUBLESHOOTING describe the single-tool case.
- **Secrets and personal data are refused on the governed write path, and the secret patterns stop refusing ordinary PHP.** Project Brain records, handoffs and agent messages are Git-tracked and repeated into Task Capsules, yet only the lightweight path and the capsule query checked what was written: a governed `start` whose goal carried an email or a phone number stored it, `brain-create --goal 'ghp_...'` stored a token that `validate` passed and `retrieve` served to another task, and `promote-auto` (on by default) copied an email into a Memory Bank chunk and `INDEX.md`. `brain_runtime` now refuses, through one `guard_shared_text()`, the secrets and personal data a call introduces in `create_record` (title, goal, conflicts), `update_record` (progress, auto checkpoint, next steps, reason, conflicts) and `append_message`; the message names the kind (`a possible GitHub token`, `personal data (email address)`), never the value. Only new text is checked, so a record written before stays updatable. Promotion refuses a record whose content carries either kind, and `validate_repository` reports a stored secret (personal data in old records is not an error). The personal-data patterns moved from `context.py` to `validate.py` (`PRIVATE_PATTERNS`, `sensitive_label()`) so both paths share them. The `assigned credential` pattern missed the keys PHP projects actually use (`DB_PASSWORD=`, `MAIL_PASSWORD=`, `AWS_SECRET_ACCESS_KEY=`: the `\b` anchor failed after `_`) and flagged code and config (`$password = $request->validated(...)`, `secret: '%env(APP_SECRET)%'`), which refused capsule queries and dropped the Laravel architect skill from every Laravel index over `php artisan down --secret=...`. It now takes snake/UPPER_SNAKE prefixes, stays on one line, needs four value characters, and skips variables, env/config lookups, templates, placeholders, masks, validation rules, code expressions and bare numbers. New patterns: a Laravel `APP_KEY=base64:...` and credentials in a URL or DSN (`mysql://app:pw@db`, `redis://:pw@cache`, `https://user:token@host`), minus documentation placeholders such as Symfony's `!ChangeMe!`. Tests: a pattern table in `memory-bank/tests/test_validate.py`, `SharedTextGuardTest` in `project-brain/tests/test_runtime.py`, and `ShippedContentIndexTest` in `tests/test_framework_semantics.py`, which indexes each edition and fails on any document excluded as a secret.
- Fix hooks failing open whenever the session's working directory is not the project root. Every edition wired its hooks as bare relative paths (`.claude/hooks/<script>.sh`, `.codex/hooks/<script>.sh`). Claude Code and Codex run a hook command in the session's current directory, so from a subdirectory the shell exited 127, and both hosts treat that as non-blocking: `bash-validator.sh` silently stopped blocking force-pushes and the other hooks stopped running. Claude Code wiring now uses the documented `"${CLAUDE_PROJECT_DIR}"/.claude/hooks/<script>.sh`. Codex wiring now uses a fixed `sh -c` launcher. It walks up from the session's directory to the nearest directory holding `.codex/hooks.json` (the project that declared the hook) and runs that project's `.codex/hooks/<script>.sh`. A script missing there exits 127 instead of running a same-named script from an ancestor such as `~/.codex/hooks/`. Codex exports no project-root variable and runs hooks through `$SHELL -lc`, so the launcher works from a subdirectory, without Git, inside a larger repository, and under bash, zsh or fish. Cursor wiring is unchanged because Cursor runs project hooks from the project root. The edition hook READMEs document the forms. New `tests/test_hook_wiring.py`, run in CI, runs every wired command from a nested directory of each edition and requires the force-push block (exit 2) through the wiring. Codex records hook trust per definition hash, so users must re-trust the changed hooks once in `/hooks`. `tests/test_hook_wiring.py` also proves the launcher never runs an ancestor's script, and that the generator's wiring gate accepts each ready-made edition's complete wiring.
- Hook scripts keep their executable bit through mirroring and installation. The Cursor and Codex copies of `subagent-dispatch.sh` were 100644 in all four editions while their `.claude` canon was 100755. Cursor runs that hook as a direct command (`subagentStop`), so every installed project got exit status 126, and the write-agent lock taken by `subagent-gate` was never released until its 30-minute TTL. The eight files are now 100755.
- `scripts/build_mirrors.py`: a mirror's executable bit now equals its canonical file's. The bit is read from the Git index first, and from the filesystem only for an untracked path, so a Windows or `core.fileMode=false` checkout gets the same answer. `--check` reports `mode differs from canon`. `--write` sets the bit on disk and, for a tracked mirror, in the index (mode only, never content), also when the bytes already match. This also removed a stale +x from Infrastructure-Creator's `.claude` and `.cursor` copies of `memory-seed/assets/scripts/validate.py`.
- `scripts/install_accelerator.py`: every `hooks/*.sh`, and every file that is 100755 in the source Git index (or executable on disk without a Git checkout), is installed with mode 0755 whatever the working tree says. A file already identical in the target keeps its content. If it must be executable and isn't, the installer adds only the bit (`chmod +x`) and reports it as `FIX_MODE` (`WOULD_FIX_MODE` with `--dry-run`). This happens under every collision mode because it destroys nothing. A refused run still writes nothing, modes included.
- `scripts/asset_parity.py`: compares the executable bit between the memory-seed asset and the canonical edition, and `--write` fixes it. Before, `copy2` copied the working-tree bit only.
- `scripts/file_modes.py`: shared helper for executable bits, read from the Git index first and written to disk and to the index. Used by `build_mirrors.py` and `asset_parity.py`.
- `tests/test_file_modes.py`, run by the CI `mirrors` job. These must be 100755 in the Git index: every tracked `*.sh` inside a `hooks/` directory, every hook script a tracked `.claude/settings.json`, `.cursor/hooks.json` or `.codex/hooks.json` runs, and every repository-root launcher with a shebang.
- **bash-validator parses commands instead of grepping them, and a shared corpus proves it.** Destructive forms that agents actually emit got past the old combined regex: `git push origin +main`, `git -C . reset --hard`, a backslash-newline continuation, `rm -r -f /`, `git commit -n`, `bin/console d:d:d --force`, `php artisan migrate:fr`, `wp db clean`, `wp site empty`, `secrets:reveal`, `config:show database`. Read-only commands were blocked, including `git branch -d` (the old `grep -i` made `-D` case-insensitive) and `grep "DROP TABLE" migrations/`. Every edition's `.claude/hooks/bash-validator.sh` (and its generated `.cursor`/`.codex` copies) now carries one generic section, byte-identical between explicit markers and shared with Infrastructure-Creator. It is a pure-bash parser with no fork per rule. It joins continuations, removes quotes, splits on `;` `&&` `||` `|` `&`, subshells and newlines, recurses into `$(...)`, backticks, `sh`/`bash -c`, `su -c`, `eval` and heredocs fed to a shell, and strips `VAR=value`, wrappers (`command`, `exec`, `sudo`, `env`, `nohup`, `time`, `timeout`, `xargs`, ...) and binary paths. It also looks through launchers that have no rules of their own (`docker compose exec`, `ddev`, `lando`, `ssh`). The section holds case-sensitive git, rm, gh and composer rules (git rules accept global options and any flag order; force push includes clustered `-f`, `--mirror`, `+refspec` and, deliberately, `--force-with-lease`), case-insensitive SQL rules that skip read-only searches and commit messages, and `.env` read rules. Framework rules follow in `BV_FRAMEWORK_RULES`; console rules match every Symfony Console abbreviation of a dangerous command. New secret-printing rules, checked against the official docs: Laravel `env:decrypt` and `config:show` of credential-bearing config files, Symfony `secrets:reveal`, `secrets:list --reveal`/`-r`, `secrets:decrypt-to-local`, `debug:dotenv` and `debug:container --env-vars`, WP-CLI `wp config get`/`list` of credentials. A block names its rule category and never the command body. Fail-open without a JSON extractor is unchanged. `tests/fixtures/bash-validator-corpus.json` (323 cases) runs through all 15 shipped copies, each with its host's payload shape, in `tests/test_bash_validator_corpus.py`, which also asserts the generic section is identical everywhere. Latency on a typical command is unchanged in the editions (0.92-1.05x). Each hooks README now says the validator is a guard against accidental destruction, not a sandbox.
- Add `scripts/check_routes.py`, a CI gate (run by the `mirrors` job) that proves every wiring and routing reference resolves in each edition and in Infrastructure-Creator. Hook wiring: every name under `(.claude|.cursor|.codex)/hooks/` in `.claude/settings.json`, `.cursor/hooks.json` or `.codex/hooks.json` (whatever the command prefix), and every Codex launcher `$1`, must be a `*.sh` script that exists, is tracked, and carries index mode 100755; a hook script no wiring file references is a warning. Routing: command `spawns`/`flow-next`/`flow-alternatives`/`stages[].agents`, `subagent_type`, `` `/x` `` spans, skill paths, agent `name`/`invokes`, every `SKILL FLOW.md`, and `AGENTS.md` references must resolve in their own tool (the Codex flow accepts skills only: Codex has no command layer). Reachability: every skill is named by a command, agent or flow. Deliberate exceptions live in `scripts/check_routes_allowlist.json`, each with a mandatory reason; stale entries warn. Regression tests: `tests/test_check_routes.py`.
- Add `scripts/check.py`, one local entry point for what CI runs. It has one group per job in `.github/workflows/*.yml`, each the job's `run:` steps in order. Jobs and matrix legs run in parallel and the steps of one job or leg run in order. Every command reports PASS/FAIL/SKIP with its duration, each failure prints its last 60 lines and the path of its log, a summary table with wall times closes the run, and any failure exits 1. Shell steps run as GitHub runs them (`bash -eo pipefail`). `python3` inside every command, including interpreters the tests start, is the interpreter running the script, and `python3.9` runs the job pinned to it. A tool the runner provides but the machine lacks (shellcheck, php, pwsh, bwrap, python3.9, the harness venv) skips with the reason, or fails under `--strict`. `tests/test_check.py` reads the workflows with a stdlib YAML subset reader and fails when a CI command has no entry in `check.py` or an entry is no longer in CI. Runner provisioning (apt-get, sysctl, venv creation, pip install) is excluded through an explicit list. It replaces the documented copy-paste loops, which exited 0 when a test failed.
- `scripts/check_routes.py` exposes `wired_hook_scripts()`, the one parser for every wiring form the editions ship (a bare or `"${CLAUDE_PROJECT_DIR}"`-anchored `.<tool>/hooks/<name>` (any name, so the caller can reject one that is not an existing `*.sh` script), and the Codex launcher that passes the script name as `$1`). The gate, `tests/test_file_modes.py` and `tests/test_installation.py` share it, so a Codex hook is checked for existence and mode like the others. CI now also runs `tests/test_bash_validator_corpus.py` (mirrors job) and `tests/test_check.py` (lint job).
- **bash-validator: review fixes for bypasses, false positives and a fail-open crash.** Generic section, identical in all copies. Shell option clusters that take a value (`bash -euo pipefail -c ...`, `-oe pipefail`, `+eo pipefail`) no longer hide the `-c` string or the heredoc. A quoted command line given to a known launcher (`ssh host '...'`, `ddev exec "..."`, `vagrant ssh -c`/`lando ssh -c "..."`, `gcloud ... --command=`) is parsed as a command. So is a heredoc or here-string fed to a shell through a launcher (`docker exec -i app bash`, `kubectl exec -i pod -- bash`, `ssh host bash -s`) or to `ssh HOST` with no remote command, and so is `git submodule foreach`. A quoted argument of any other program is still text. DELETE with no WHERE, and TRUNCATE without TABLE, are refused only when the input runs SQL (`mysql`, `psql`, `sqlite3`, `wp db query`/`cli`, `dbal:run-sql`, `doctrine:query:sql`, `artisan tinker`/`db`): test names, issue and PR titles and bodies, `sed -i` and file-writing heredocs that say "delete from" pass again. DROP, TRUNCATE TABLE and an always-true WHERE (`1=1`, `true`, and now a bare `1`) are refused everywhere, as before. New: `dropdb`, `mysqladmin drop`, `git config` writing or unsetting `core.hooksPath`, and `git --config-env core.hooksPath=...`. `git log -S`/`git grep -e` searching for `--no-verify` passes. `.env` readers now include `grep`/`egrep`/`fgrep`/`rg`/`ag`/`ack`/`awk`/`cut`; a search's pattern operand (`grep -rn .env src/`) and `-q`/`-l`/`-L`/`-c` are not reads, and `sed -Ei`/`-ni` in-place edits pass. Console rules see options placed before the command name (`bin/console --reveal secrets:list`, `php artisan --force db:seed`). A `-x` item matches inside a short-option cluster. An option's value is not taken for the command's argument (`--env production database`, `--em customer prev`), using per-edition `BV_CONSOLE_VALUES`/`BV_CONSOLE_SHORT_VALUES`. Sail's `art`/`a` aliases count as artisan. A `php <script>` that is not a listed entry yields only namespaced command names, so `php vendor/bin/phpunit --filter rollback` is a test run. The parser reads through a 1 KB window and builds long words in two parts. Parse time is now linear: 100 KB of dense shell syntax takes about 2 s instead of 5-9 s, and a 127 KB heredoc write about 75 ms instead of 190 ms. Nesting deeper than any real command (`BV_MAX_NEST`) is refused as `command too complex`, where it used to crash bash with SIGSEGV, which every host treated as allow. Latency on a typical command is 5.7-6.2 ms, up from 5.1-5.5 ms. The corpus grows from 210 to 323 cases with new decisions (delete-from, core.hooksPath, launchers, nesting), and `tests/test_bash_validator_corpus.py` gains a parse-cost test that asserts linear growth.
- Harness Setup repairs a hook whose content matches the source but lacks its executable bit. The preview shows it as `fix-mode`, counts it as a change and names the mode change. Install changes only the mode, in place, and the receipt records the repaired mode. Before, the preview reported 0 changes and the receipt recorded 0644 as verified, while the hook kept exiting 126.
- Harness Setup stages every source file that is 100755 in the source Git index with its executable bit, so it installs the same modes as `scripts/install_accelerator.py`. Before, on a checkout without filesystem modes (`core.fileMode=false`, or a Windows-made checkout), skill scripts such as `skill-creator/scripts/init_skill.py` installed 0644 through the Harness but 0755 through the CLI.
- `scripts/file_modes.py`: an intent-to-add index entry (`git add -N`) now counts as untracked. Before, `build_mirrors.py --write` and `asset_parity.py --write` repaired its mode through `update-index --index-info`, which staged the empty blob it records in place of the file and dropped the flag, so a plain `git commit` shipped a 0-byte mirror.
- `scripts/check_routes.py` now resolves references the way the runtime does. `subagent_type`, `stages[].agents`, prose spawns in command bodies and `AGENTS.md` "`x` agent" must equal an agent's frontmatter `name` as the host and `subagent-gate.sh` read it; the file stem and the `<name>-agent` alias stay valid for `spawns` only, and an agent name the gate reads differently (quoted) is an error. Command bodies (Claude and Cursor) also check "spawn `x`", "Spawn the `x` agent", "Spawn `x` with", a line opening "Spawn x agent", and "invoke the x skill". `SKILL FLOW.md` checks also cover bare-name steps of fenced diagrams (a whole-step rule that does not read diagram prose as steps) and backticked names followed by `<arg>`/`[ctx]`. Hook wiring treats every name under `.<tool>/hooks/` and every launcher `$1` as a reference, and one that is not an existing `*.sh` script (for example a typo that drops `.sh`) is an error, not an unwired-script warning. A non-UTF-8 file is an error finding (exit 1, valid `--json`) instead of a traceback; a malformed allowlist entry is reported and silences nothing; `--json` prints `{"error": ...}` on an exit-2 failure.
- `scripts/check.py`: SIGTERM and SIGHUP now stop a run the way Ctrl-C does. Every command runs in a session of its own, so `kill`, `timeout`, a closed terminal or an IDE stop button used to end check.py alone, leaving its commands running as orphans and `/tmp/check-py-*` behind. Now each running command's process group is terminated, the temporary directory is removed, and the run exits 130. A signal ignored at start (`nohup`) stays ignored, and a second signal no longer cuts the cleanup short.
- `scripts/check.py`: a guard no longer hides a step CI runs. An entry whose guarded files are absent still skips as "not present" where its workflow has no such step. Where the workflow does run the step, the skip says CI fails there, and `--strict` fails it. `tests/test_check.py` fails on it in either mode. Before, renaming `docs/examples/ai-system/system.json` skipped all six system-orchestration commands, even under `--strict`, and the drift test stayed green.
- `scripts/check.py`: the changelog group no longer passes when `check_core_changelog.sh` cannot resolve a merge base. The script then exits 0 without checking anything. An explicit `--base` that does not resolve (a typo, an unfetched branch) now fails. A missing default base (`origin/main`, or a local `main`) skips with the reason, or fails under `--strict`.
- `tests/test_check.py` compares each job's `actions/setup-python` version, leg by leg, with the interpreter `scripts/check.py` pins for that leg. Moving a pin or adding or removing a matrix leg in `ci.yml` now fails the drift test. `UNMIRRORED_PYTHON` names the two pins check.py deliberately does not reproduce, with the reason for each: harness-fleet's 3.11 and windows-harness's 3.13.
- `memory-bank/tests/test_hooks.py` (Laravel, Symfony, PHP Core, WordPress): `test_body_horizontal_rule_cannot_declare_writes` now writes its parser-trap agent into a throwaway repository with its own copy of `subagent-gate.sh`, not into the edition's `.claude/agents/`. `scripts/check.py` runs the tests beside the mirror, parity and route gates in one checkout, and those gates failed now and then on the unmirrored agent while it was briefly present. A run stopped at that moment left the file in the tree. The new `FixtureIsolationTest` fails if the fixture is written inside the edition again.

- Harness: **Existing Git worktree** as a third workspace. A new session can run
  in a checkout that Git lists for the project's repository: one made outside
  the Harness, for example by a script that also prepares the task's
  environment. The picker lists every other checkout except the project's own
  folder and anything inside the runner's state, such as session and Delivery
  check worktrees, and chooses none for you. The session uses the checkout as it
  is, with its branch, uncommitted files and environment. An entry's ID covers
  its path and branch, so a folder reused for another branch needs a refresh.
  Follow-ups accept a switched branch, but refuse a checkout that Git no longer
  lists, that became a link or that belongs to another repository. New:
  `GET /api/projects/{id}/worktrees`, `workspace: "existing-worktree"` with a
  listing `worktree_id` on session creation. The Creator still offers only the
  project folder or a new worktree.

- Harness: **slash commands** work as in the CLI behind the session.
  - In a Workspace session, `/` at the start of a message lists the CLI's own commands, with keyboard
    selection. Tab inserts the name; Enter inserts it and runs a command that needs no argument.
  - Claude Code's list is the CLI's own, from an `initialize` control request to `claude -p` in the
    workspace (no turn, no model call): built-ins that work in print mode, project, personal and plugin
    commands and skills; the probe runs no hooks. A message that starts with one goes to Claude Code
    exactly as typed, without Harness memory, task or instructions. Sessions with reviewed memory refuse
    commands.
  - `/clear` (`/reset`, `/new`), `/model` and `/effort` act on the page when typed alone or with one
    value, and the API refuses them in a message.
  - A Claude message that starts with `/` but is not one of the CLI's commands is marked as the user's
    text, so the CLI does not run it.
  - Codex reads messages as text, so the Harness does what the Codex app would. `$name` anywhere names
    one of Codex's own skills (`codex app-server` `skills/list`) and gets an explicit request.
    `/prompts:name` expands a custom prompt, `/init` asks for an AGENTS.md guide, and `/new`, `/model`,
    `/diff` and `/status` act on the page.
  - New: `harness/src/harness/commands.py`, `harness/web/composer-commands.js`,
    `GET /api/projects/{id}/commands?provider=` (optional `worktree=`) and
    `GET /api/sessions/{id}/commands`. Both need the page's token, and the page now sends it with
    every request.

- System Orchestration judges a memory chunk's `review_after` and `valid_to`
  by the local calendar, the one the native Memory Bank contract it applies
  uses. It read the UTC date, so west of UTC a chunk valid through today was
  dropped from plans in the local evening while the Memory Bank still served
  it. Two tests no longer fail around local midnight: the memory expiry test
  (UTC date against the contract's local one) and the Memory use history test
  (two retrievals an hour apart crossed into the previous day before 01:00).

- Harness: **Run view**. While a Workspace, Plan, Review or SDD turn runs, a
  strip above the composer shows what the agent is doing now: the tool, its
  target and how long the call has been open, or *Model's turn*. It also shows
  counts of files, plan items, checks and failed steps. A Harness check no
  longer reads as "Claude is running". **Calm** is the default. **Detailed**
  opens three tabs:
  - **Files:** a map of the files named in tool calls.
  - **Plan:** the agent's own todo list, with what was added or dropped.
  - **Commands:** PHP checks as runs per target, a result marked unknown when a
    pipe or a later command hides it, fixers, and **Run in Harness**.

  Each step in the conversation names its target. A run receipt follows each
  turn, a compaction leaves a divider, and the tab title and icon carry the
  state. Notifications are opt-in and private.

  For native launches `providers.normalize_event(..., targets=True)` attaches
  each tool's canonical name, call ID, state, project-relative path (a file
  outside the project keeps only its name), redacted detail (a command's first
  line without a heredoc body; an MCP tool's name, never its arguments), `not_run`
  outcome, Codex exit code and file changes, and Claude helper parent. These
  targets live in `harness/src/harness/run_activity.py` (`Enricher`,
  `RunLedger`, `redact_command`), along with plan events, compaction dividers
  and one limited notice. They sit outside the launch's 5000-event / 4 MiB
  failure limit and under caps of their own, so recording them never changes
  whether a launch fails.

  Other server changes:
  - Every stored event gets a millisecond `at`.
  - The closing status carries `outcome`.
  - `GET /api/sessions/<id>` returns the newest `launch` of any kind.
  - `launches.receipt` is also saved when a launch hits its output limit, is
    cancelled or cannot start.
  - `/results?diff=names` lists changed files without the diff.
  - System runs' agent panels use the same relative paths and redaction.

  File contents, diffs, tool output and reasoning are never stored.
  `tests.test_harness_run_activity` joins the CI Harness job.

- Harness: project memory runs by itself. Every new session uses it and
  nothing waits for a person: the first message names a new Brain task, every
  message is the retrieval query for its own turn (`context.py refresh` with
  the provider as host, manifest in the ignored local store), the capsule goes
  straight into the launch, and when a run completes its `memory-draft` is
  saved — progress, next steps, and each learning written as observed and
  raised to verified with the reason "agent-attested, not reviewed by a
  person", then `promote-auto` once. Learnings citing no file in the workspace
  or already saved from the session are left out; a retrieval or save failure
  costs the turn its memory, never the turn, and the conversation shows a line
  for what each turn carried and saved. Review by hand (prepare, approve, Save
  to memory) is an opt-in, and sessions linked before this keep it. A project
  without a governed runtime starts its sessions with its reference files
  instead of refusing them. Memory use names these retrievals Harness, and the Claude and
  Codex read hooks, which call `refresh`, by their host instead of CLI.

- Harness: the **Project context** chip and the **Use project memory** checkbox
  are gone. Project memory is always on and also decides what the project
  contributes: a turn whose prompt carries a memory capsule no longer also gets
  the reference-file excerpts; without a capsule (no governed
  runtime, or retrieval failed) they stand in, sent once per native
  conversation instead of on every resume, and leave out the instruction file
  the provider loads by itself (`CLAUDE.md` for Claude, `AGENTS.md` for Codex).

- The Claude and Codex read hooks (`working-memory-read.sh`) stand down when
  `CONTEXT_CAPSULE_DELIVERED=1`. The Harness sets it when it has already put
  the turn's capsule into the prompt, retrieved for the message alone; the hook
  would otherwise distill a second capsule from that whole prompt and spend the
  turn's memory budget twice. The Stop hook still checkpoints the task.

- Harness: **Save to memory** after a linked run. Agents left to policy almost
  never record what a run established — 0 memory commands in ~1,850 prompts on
  four real installations — so a linked launch's prompt now asks the agent to
  close its final reply with a `memory-draft` block (progress, up to three next
  steps, up to three learnings with source files). Nothing is written from it:
  the session page shows the draft as an editable form, and only on Save, with
  the person confirming each kept learning against its sources, does the
  Harness run the runtime's own commands — `brain-update` on the task (next
  steps replaced), `brain-create` + `brain-update` for each learning as a
  verified, resolved finding or accepted decision, then `promote-auto` once.
  Sources must be regular files in the session workspace; a failed command
  reports what was already saved. `harness/src/harness/memory_draft.py`,
  `POST /api/sessions/<id>/memory`; `brain_info` carries `memory_draft`.

- `context.py promote-auto` runs automatic promotion now, under the same rules
  as a turn boundary, and does nothing unless `automatic_promotion` is enabled.
  Knowledge recorded deliberately no longer waits for `--flush-after` turns.

- The automatic checkpoint says what the work was. It listed only how many
  turns and files a flush covered, and agents almost never write progress
  themselves — on four real installations 23 of 24 tasks held nothing else. A
  flush now appends the branch's newest commit subjects, newest first (at most
  five, 80 characters each, `(+N more)`), counted from the merge base with the
  default branch; merges and any subject a secret or privacy gate would refuse
  are left out, never failing the flush. A turn whose HEAD moved also counts
  as work even with a clean tree: a turn that ended in a commit used to read as
  a turn with nothing in it, so committed work never reached the buffer. The
  HEAD each task last saw is kept in local state (`turn-heads`, bounded).

- A Memory Bank write is refused only for what it introduces. Promotion,
  compaction, `bank-reverify` and `bank-retire` validated the whole bank
  afterwards and rolled back on any error, so one chunk passing its review
  date failed every later write in the repository, and two overdue chunks each
  blocked the one command that would repair the other — reproduced: neither
  could be re-verified and only hand-editing got out. Writes now compare the
  bank with itself before the write (`brain_runtime.bank_write_errors`) and
  still hold the chunk they produce or attest to the full contract, so a
  re-verify of a chunk whose source is gone is refused as before. An overdue
  chunk still leaves retrieval and `validate.py` still reports it; `refresh`
  now says so on every turn (`memory review: N chunk(s) overdue…`,
  `overdue_review` in `--json`). Validator messages name paths from the
  bank's parent instead of the machine's absolute path, which had reached the
  Task Capsule through a refused promotion's reason. The Harness memory demo
  no longer stalls its two late automatic promotions.

- Compaction no longer waits for every cited file to stand still. It validated
  the Brain with source freshness included, so one record whose cited file
  changed — an accepted decision citing a living spec, a finding citing the
  code it was about — refused every later compaction, manual and automatic,
  with `Compaction refused because active records are invalid`. Freshness is
  now left out of both of compaction's validations
  (`validate_repository(..., check_freshness=False)`): moving terminal records
  can neither cause nor cure it, and the documented contract was already that
  compaction skips stale records. `validate` still reports them, retrieval and
  promotion still skip them, and the refusal now says how many problems it
  found.

- The rendered capsule says where the work stopped. Claude Code, Codex and
  Cursor read the plain capsule, which printed only `working: <task> — <goal>`:
  progress and next steps never reached the model, and "continue where we left
  off" retrieved skill pointers instead. It now adds bounded `phase:`,
  `progress:`, `next:`, `recent files:` and `task record:` lines. The capsule
  keeps the three newest next steps and the newest eight files (it kept the
  oldest eight), a file touched again moves to the end of the task's list,
  and `update`/`brain-update --replace-next-steps` lets a finished step leave
  the list — it was append-only.

- Capsule slots no longer point at what the model already has. The task's own
  record and handoff leave as `working-task`; with `--host claude` or
  `--host codex` the instruction files that host loads itself leave as
  `host-loaded` (`CLAUDE.md` and its `@path` imports, `AGENTS.md`). On real
  installations `CLAUDE.md` held a procedural slot on 88 of 114 and 150 of 158
  Claude Code turns, and the task's own record a semantic slot on 13–20%.

- Run System Orchestration on native Windows:
  - System files, run journals and editor saves go through the portable
    filesystem layer, whose rooted handles refuse junctions. Windows refuses to
    replace a file another program holds open, for example while the Harness
    reads a run, so a save retries for up to 2 seconds.
  - Every worker and Brain call starts under the Harness process guard, whose
    Windows job ends the whole process tree. A worker CLI installed by npm
    starts through Node.js directly, never through its `.cmd` launcher.
  - Run and workspace locks are share-deny opens. The workspace lock folder
    gets an ACL that admits only the current user and SYSTEM.
  - Brain calls, the System runner and `scripts/ai_system.py` exchange UTF-8,
    so a task written in Russian also works on Windows.
  - AI discovery (**Fill with AI**) runs in Codex's elevated sandbox, as Creator
    phases do. Its permission profile is an allow-list: the platform minimum,
    the CLI runtime, Python, the run folder and the provider's account state,
    with every selected service folder denied outright. Codex denies a folder
    with an inherited Windows ACE, which a file with its own allow entry
    escapes. So before every scan a probe inside the same sandbox opens each
    original file and folder, up to 100,000, and the scan starts only if none
    is readable. A Codex agent inside drops its own nested sandbox, as Creator's
    does.
  - The new `tests.test_ai_system_portable` runs on Linux and in the
    `windows-harness` job, including a run with an npm-installed Codex fixture.
    `tests.test_harness_system_discovery` checks the scan profile, and with a
    local Codex CLI enforces it through Codex's Linux sandbox. The new
    `tests.test_windows_discovery` runs it in the real elevated sandbox, in the
    manual Windows sandbox workflow.
- Join the native Windows Harness (PR #35) with the redesigned page and System
  Orchestration (PR #36):
  - The redesigned page keeps its split scripts. Creator names the missing
    isolation as the server reports it.
  - Memory use and the live Codex fill read files through the portable
    filesystem layer.
  - `scripts/ai_system_execution.py` imported `fcntl`, which kept the server
    from starting on native Windows. It no longer does.
- Let the browser keep the Harness page files. The page names each style and
  script by its content hash, so a repeat visit loads none of them again (83 KB
  instead of 651 KB here). That address is served `immutable`; any other request
  for a page file revalidates by ETag (304), and API answers stay `no-store`. The
  session list in `/api/bootstrap` and `/api/sessions` now carries summaries
  (`Sessions.summaries()`, one query), so 200 sessions with capsules weigh 71 KB
  instead of 1.96 MB. Opening a session takes its settings from the full record
  the first poll brings.

- Finish the Harness memory views:
  - **Retrieval history.** Memory use keeps a numbers-only daily rollup of
    retrievals in the Harness database (`retrieval_days`, with
    `retrieval_seen` so each manifest counts once). Retrieval history therefore
    outlives the newest manifests a project keeps. The rollup is folded when
    the view reads or a session launch ends, and is shown in the Selected
    breakdown and the chunk card.
  - **Live Codex fill.** A running Codex launch's fill is read from its rollout
    as it grows (`context_usage.CodexLive`, every two seconds; the rollout path
    comes from `providers.codex_rollout`).
  - **Startup context per edition.** Accelerators › Kit 2 lists the exact
    startup bytes per edition against their CI ceilings
    (`GET /api/accelerators/startup`, measured by `scripts/context_budget.py`).
  - **Calibration citation.** The citation in `context_budget.py` (and in the
    CI comment) points at `docs/TOKEN-ECONOMY-RESEARCH.md` as of `9435dfc1^`.
    `9435dfc1` removed the file, which left it dangling.

- Add **Sessions › Usage › Context** to the Harness: how full each turn left the
  agent's context window, how much of it was memory the Harness sent, how the
  turn grew it and when compaction cleared it. Each launch now records a
  numbers-only `context` column in `launches` (migrated in place; older launches
  read as not recorded). The column holds the prompt ledger built in
  `Sessions._prompt` (capsule by memory kind, project excerpts sent of their
  full size, attachments, instructions and the message) and the fill that
  `context_usage.ContextTracker` reads from the provider's own usage. Claude
  calls are counted once per message ID, subagents are left out, and compaction
  comes from `compact_boundary`; Codex fill is read from the thread's rollout at
  the end of the launch, and Cursor reports none. Session launches of Claude add
  `--include-hook-events`, and only the character counts of hook output are
  kept. Fleet and Clash launch rows name the memory prefix and how many agents
  received it. `Sessions.get` adds `context_last` for a composer meter that
  appears once the latest turn fills half its window or compacts.

- Add **Knowledge › Memory use** to the Harness. The new first Knowledge tab
  follows one knowledge root through Project Brain, promotion, the Memory bank
  and the last 200 retrievals, in fixed-step bands for a 7-, 30- or 90-day
  window or all time. Rows flag chunks past their review date, chunks whose
  cited files changed, and stalled automatic promotions. A retrieval strip and a
  review horizon each have a keyboard-navigable detail card and a table twin. On
  a return visit the tab shows and replays only what changed since the last one,
  and reduced motion shows a `+N` mark instead.
  `harness/src/harness/memory_use.py` reads project files in process, without
  the knowledge lock and without running project code. Only counts, dates, chunk
  IDs and chunk titles leave the server; Brain titles and bodies, queries and
  non-chunk paths do not. A Harness freshness re-check counts with its
  retrieval. `POST …/memory-use/check` runs the installed runtime's
  `promotable_records` and `memory_eligibility` on a click and returns 409 while
  the lock is busy; the knowledge lock's busy error is now `KnowledgeBusy`
  (HTTP 409). `tests/harness_memory_demo.py` builds a year of history through the
  copied runtime's own API.

- Show what memory adds where a Harness launch is decided. The linked Brain
  task card shows the capsule against its 8,000-character cap as one bar split
  into Project Brain, Memory bank and Rules & docs, with the items the runtime
  dropped to fit and the characters repeated by the capsule's selected and
  category views; the note beside **Run with this context** estimates what the
  capsule adds to the turn. The counts come from `sessions.capsule_meter`,
  measured on the server the way the runtime measures the cap: the browser
  writes a float such as `4e-06` as `0.000004`, so its own count would drift.
  `task_context` checks the cap against the same `CAPSULE_LIMIT`. The
  **Project context** chip shows what its excerpts add to each launch, with the
  3,000-byte excerpt size from `bootstrap.runtime.context_excerpt_bytes`.

- Count each Harness launch's own Claude spend. From Claude Code 2.1.277 a
  resumed session's result reports the session's whole spend in
  `total_cost_usd`, and the Harness added those totals launch by launch: a
  resumed Claude session's Usage totals, its "Usage · $" label and the launch
  USD cap counted earlier launches again, and Clash turns did the same.
  `providers.RunCost` now keeps each launch's and each Clash turn's growth over
  the native session's last reported total (`sessions.cost_totals`), read per
  run from the CLI version in the init event; older CLIs keep their per-run
  cost. When the share cannot be told the cost stays unknown, never 0, and the
  launch records the raw report as `cost_usd_reported`. Launches recorded before
  this change keep their totals. The agents panel no longer turns an unreported
  token count or cost into 0.

- Lay the motion and number foundation for the memory views. Motion tokens
  (fast, base, slow, travel, stagger, pulse and three easings) replace the
  literal durations, the two pulses share one keyframe, and a guard rejects
  literal durations and `cubic-bezier()` in rules on both themed pages. Reduced
  motion now also stops `::before` and `::after` animations (the agents panel's
  live dot kept pulsing), and scripted scrolls follow it through
  `scrollMotion()`. Six memory and context colour tokens join both themes,
  guarded at 3:1 on both surfaces. `fmt` gives numbers one grammar (exact, ≈,
  ≤ / ≥ / +, —) and the Usage totals use it: unknown totals read — and costs
  read as the CLIs' estimates. `keyedRender` keeps launch and check details open
  across result polls; before, every poll collapsed them.

- Bring System Orchestration (PR #36) onto the Harness design. It is a sidebar
  section with two tabs: **Services** (system file, contract map, service cards
  and the editor with **Fill with AI**) and **Changes** (a change selector, the
  new-change form with starting services as chips, and the selected change as
  **Plan · Review · Run · Receipts** steps with its launch, recovery, agents and
  receipts). It follows the sidebar **Project** instead of its own project
  select; **Choose system folder…** adds a folder and makes it the working
  project in every view. The map is drawn at its natural size in theme colors,
  so it reads the same in the dark theme. Available services and completed
  receipts carry no label; native task references and the runner log sit behind
  toggles. Saving the system closes the editor onto the updated map. The launch
  note names edit mode's writes in each service's current checkout, recovery
  names the provider that resumes, starting services carry their dependency
  warnings and the plan lists its warnings. Runner transcripts of system changes
  and AI scans in **Sessions** point back to System Orchestration. The
  screenshots in `docs/AI-SYSTEM-ORCHESTRATION.md` show the new layout.

- Split the Harness page: markup stays in `harness/web/index.html`, styles move
  to `app.css` and the script to five classic files (`app-core.js`,
  `app-knowledge.js`, `app-setup.js`, `app-skills.js`, `app-creator.js`) that
  load in order with no build step. The code moved unchanged apart from
  indentation. The server serves only the files named in `ASSETS`, read at
  start with the page; a test checks that the page and the list agree.

- Finish the Harness redesign plan. One **Project** selector in the sidebar
  drives every view (the per-view project selects are gone from the screen); an
  open session of another project yields to a new draft. Results splits into
  **Changes · Checks · Usage** tabs next to Conversation, with counts, and shows
  Worktree delivery only for worktree sessions. The Creator shows a run's phase
  as five steps and keeps the new-run form behind **New run**. Skills lists one
  row per installed skill with tool chips (36 rows instead of 108 for the demo
  project), compacts the catalog, and keeps the selection and **Preview
  installation** in a bar at the bottom.

- Regroup the Harness into five sections: Sessions, Knowledge (Project Brain,
  Memory bank, Context files), Skills (Library, Create skill), Accelerators
  (Overview, Infrastructure Creator, Open Source Kit) and Projects & Setup.
  Tabs replace the separate entries, every view has its own `#/view` address,
  and Results & verification becomes a tab of the open session. Slogan headings
  give way to one title per view, and the embedded Kit 3 catalog drops its own
  header, hero and footer. Setup readiness now separates the constant `scope`
  notes from real `diagnostics`, so only problems are highlighted. Copy across
  Setup, Creator, Skills, Knowledge and Results is cut to the decision point;
  the README gains interface copy rules. Fixes: Creator no longer posts a
  `null` thinking effort when a run's provider differs from the form; Setup
  hides a used preview after installing and no longer preselects an edition;
  Create skill shows what is missing when clicked; Recent sessions keeps focus
  while a session runs; Memory documents open on their text, with front matter
  under Metadata.

- Calm the Harness Sessions screen. The launch fields sit in one row, and Mode
  appears only when the workflow leaves it open. Optional settings (helpers,
  Clash, workspace, Brain task, budgets, separate models) become chips that
  show their current value and open one panel at a time. An open session
  collapses to a single summary line with **Next-turn settings**. Fleet review
  ticks its helpers itself, the SDD slug waits for input before showing an
  error, and hints, budget, Clash and composer copy keep only what matters at
  the moment of decision. Consecutive activity steps fold into one row, and a
  failed run shows one error card. Both Harness pages now take font size,
  weight, line height and radius from scale tokens, with spacing on a 4px grid;
  `tests.test_harness_web` enforces this. One field style replaces seven
  copies, and muted text and focus rings gain contrast.

- Add a dark theme to the Harness browser workspace: a System / Light / Dark
  switch at the bottom of the sidebar, stored in the browser and applied before
  first paint, so a dark page never flashes white; **System** follows the OS
  setting live. The Kit 3 catalog follows the same choice inside the Harness and
  the OS theme when served alone. Both stylesheets now take every color from
  paired light/dark tokens, enforced by `tests.test_harness_web`; the Setup
  preview's collision label uses the error color instead of an undefined
  `--red`.

- Show a live **Agents** panel for System Orchestration runs and AI scans: one
  card per contract, service, verification or discovery agent with state, time,
  granted folders, tool calls, tokens and changed files, plus a timeline of its
  messages, reasoning summaries, plans and tool targets. Workers stream native
  output line by line; Claude workers now use `stream-json` with the same
  `--json-schema` terminal `structured_output`. Activity is display-only,
  redacted, bounded per agent and launch, and never stores file contents, diffs,
  command output or prompts. Run details list each launch; runner status no
  longer disappears behind the first page of events.
- Add `--access all` (browser default: **All selected services**) so every
  system worker can read all selected service folders and, in edit mode, change
  files in any of them; the system folder stays read-only. Claude receives the
  folders through `--add-dir`, Codex through writable roots; Cursor is limited to
  the own-service scope. Cross-service changes are reported as
  `<service-id>/<path>` and adopted only when reported. Contract and verification
  workers now read every selected service under Claude in both scopes. The scope
  is pinned in the journal; older journals resume with the own-service scope.
- Fix intermittent HTTP 400 responses while polling a live system run: a journal
  file being published by link-then-unlink is read on the next poll instead.
- Replace the opaque `Native AI discovery CLI failed` with the actual cause: an
  expired or missing CLI login (with `claude auth login`, `codex login` or
  `cursor-agent login`), a sandbox that cannot start (with AppArmor and
  user-namespace hints), a CLI that cannot start inside the sandbox, a timeout or
  a rate limit. Blocked system dispatches show the same CLI reason on the agent
  card and in the run log. Probe bubblewrap before queueing a scan and warn in the
  editor. Document troubleshooting, including an AppArmor profile for bubblewrap,
  and enable user namespaces for the discovery tests on GitHub's Ubuntu runners.

- Automatically fill Harness system/service forms through Codex, Claude or
  Cursor from bounded, filtered evidence snapshots. Show sources/uncertainties,
  validate every service and contract, retain unknown ownership and incomplete
  dependency coverage, and require fresh evidence at preview and final save.
  Use the existing serialized queue/watchdog for cancellation/restart; isolate
  source reads from original projects while allowing native CLI runtime/account
  state. Cover real subprocess adapters and the visible browser form flow.

- Create and edit system orchestration in Harness through a folder browser and
  forms for service ownership, capabilities, contracts, dependencies and context
  sources. Register selected folders and generate metadata without JSON input.
  Validate the full graph before saving, reject stale forms/files, preserve file
  permissions and recover interrupted multi-file saves with a durable journal.
  Block metadata writes/recovery during active sessions; retain detected external
  edits as conflicts. Cover localhost API, persistence and failure paths.

- Support Claude Code and Cursor Agent for system workers in the CLI and Harness
  UI alongside Codex. Share native permission/delegation flags, validate Claude
  structured output and Cursor terminal reports, bound Cursor prompt bytes,
  and pin provider/executable across recovery. Preserve legacy Codex requests.
  Cover native reports, modes, retry and cancellation with deterministic CLIs.

- Integrate system orchestration into Harness UI: declared service graphs,
  capabilities and memory ownership, reviewed context/impact plans, sequential
  Codex execution through the shared queue, live receipts/native task references,
  cancellation and explicit recovery. Derive filesystem access from registered
  projects; preserve pending launch identity across crashes and stop detached
  worker trees through nested watchdogs. Cover real HTTP/native Brain paths on
  Python 3.9 and current Python using deterministic provider fixtures.

- Add an optional stack-neutral system planner (`scripts/ai_system.py`) for
  service passports, declared contract impact, globally budgeted context,
  Mermaid maps and cross-service plans. Preserve local Brain/Memory ownership,
  require explicit access to external roots, fingerprint sources/commits and
  detect changed or newly available inputs. Include a synthetic three-service
  example and stdlib regression coverage. Add explicit sequential Codex/custom
  adapter execution, native system/service Brain tasks, structured terminal
  receipts, source checkpoints and crash-safe reconciliation. Require explicit
  retries for ambiguous writes; complete tasks only after reported cross-service
  verification and preserve knowledge handoff without promoting raw context.

- Fix native Windows Harness guard shutdown and private-state ownership on
  elevated runners. Reject non-UTF-8 Creator plans with a controlled error and
  make pipe and plan fixtures independent of Windows newline/encoding defaults.

- Add native Windows/Git Bash Harness runtime support: working Python fallback,
  Job Object process cleanup, threaded bounded pipe reads, retained admission
  locks, rooted no-reparse filesystem operations and private state ACLs.
  Launch supported npm CLI shims directly through Node.js, support Windows
  Fleet venv paths and Project Brain process locks, and add a native Windows
  acceptance CI job. Creator also supports native Windows through Codex elevated
  permission profiles, with read-only project/control roots, per-phase boundary
  probes and disposable installed-runtime validation copies.
- Let a Harness Creator re-launch change how the run executes, and stop losing
  a phase to its own stream budget. Three defects found while scanning a real
  project; each one alone could end a run that had already done its work.
  - **Re-launching a phase applies the current model, thinking effort and
    additional-agent settings.** `act()` now accepts a `routing` object and
    applies it before `rescan`, `revise` and `generate`; the browser sends the
    form's values and mirrors a run's settings into the form when a different
    run is selected. Before this, `_launch` always read the values frozen at
    `start()`: a corrected helper count (or model, or effort) was silently
    ignored and the stale configuration ran again - including the one that had
    just failed. Provider, tools and operation stay frozen; they define what the
    run is, not how the next launch executes.
  - **Cancelling a phase is no longer refused as stale.** The cancel branch runs
    before the revision guard and always reports the live state: a background
    poll can advance the revision between rendering the button and clicking it
    (the phase ends on its own), after which cancel failed with "This Creator
    view is stale" or silently did nothing. Cancellation during `apply` and
    `rollback` stays refused, as before.
  - **The phase output budget scales with the helpers that share the stream,
    and exhausting it no longer discards completed work.** The limit was a fixed
    3 MiB regardless of helper count, so a multi-helper scan spent it on the
    helpers' own events and died holding a half-written profile; it is now
    3 MiB plus 3 MiB per helper, capped at 96 MiB, and unchanged at 3 MiB when
    additional agents are disabled. On exhaustion the runner stops reading the
    stream, asks the agent to finish, and keeps every artifact already written -
    the canonical verification that follows decides whether the phase passed,
    instead of the limit ending it outright.

- Run Harness Creator phases on macOS through the built-in `sandbox-exec`
  (Seatbelt) profile when bubblewrap is absent: writes are allowed only in the
  run workspace, a private temporary directory, the per-user temporary space and
  the provider's account directory; the target and a worktree's Git common
  directory stay denied, and the disposable runtime-cache check re-allows the
  project's ignored `memory-bank/local` in place. Creator availability and the
  start check accept either isolation tool; the Linux bubblewrap policy is
  unchanged.

- Add the Harness **Clash with a challenger** option for Workspace and Review
  sessions: the selected provider implements (Edit mode) or reviews (Plan mode)
  while a different provider challenges the result, alternating for bounded
  rounds until the challenger accepts. A stdlib runner executes one
  native turn per participant in the session workspace, parses each fenced JSON
  verdict, keeps an objection/finding ledger with the exchange behind every
  item, streams turns labelled by provider and role, and saves the ledger, usage
  and a Markdown report with the session. Follow-ups with the box ticked start
  the next cycle with both native sessions resumed; the option can be switched
  on or off between turns, and additional agents and Ultracode stay disabled.

- Treat empty Harness time budgets as uncapped instead of silently applying the
  server's 900-second timeout. Show unlimited time in forms and per-agent shares,
  report explicit time limits in errors, and retain cancellation/watchdog cleanup.

- Allow changing helper count and enabling/disabling delegation between Harness
  session turns; validate and persist follow-up settings while retaining each
  launch's original settings and recalculating per-agent budget shares.

- Make the selected Harness helper count mandatory per turn and Creator model
  phase; allow batches under lower native concurrency. Count distinct native
  receipts, merge Codex stream/journal evidence, and show confirmed/required
  launches with shortfall or excess instead of confirming a single helper.

- Attach files to new Harness Sessions and follow-up messages, retain downloads
  in history, and pass verified local file references to native agent tools.

- Preserve Harness Sessions project preferences across reloads and project
  switches, and reopen the selected session using its server-side settings.

- Add a draggable vertical size control for Harness AI output, with keyboard
  resizing and double-click reset while keeping the message composer accessible.

- Recognize Codex V2 helper lifecycle events in Harness activity and delegation
  verification, fixing false "Helper launch not confirmed" notices after real
  subagent work. When exec JSON omits those events, check bounded native journal
  metadata for the same session, workspace and current turn. Preserve V1 spawn
  receipts and reject malformed or stale activity evidence.

- Require at least one useful native helper when additional agents are enabled
  and delegation is available, across Harness sessions and Creator phases;
  allow Claude Agent/Task calls and show per-turn native delegation evidence.

- Add a local project folder picker to Harness Projects & Setup, with navigation,
  hidden-folder visibility and bounded recursive name search before registration.

- Pass the linked Harness Brain task ID to native hooks, overriding an unrelated
  inherited task identity instead of falling back to the workspace branch.

- Fix memory status consolidation counters after a promotion by unpacking the
  promotion iterator records; include a regression check for applied counts.

- Fix Harness SDD artifact names to pass accelerator naming hooks, retain legacy
  documents, and focus the Brain progress field after selecting a record. Report
  native USD overspending explicitly and reject over-budget CLI review results.

- Optional Harness planning/editing model selection, with independent thinking
  effort, SDD phase routing, explicit Workspace mode changes and launch history.

- Harness SDD workflow: select and resume explicit development phases in a native
  session, inspect specification documents, validate phase prerequisites, and
  retain phase settings in launch history with existing workspace and budget controls.

- Harness per-agent budget calculator for sessions, Fleet and Creator: convert
  money/tokens into shared totals, account for concurrent time and Fleet waves,
  show equal planning shares and retain the launch allocation in usage history.

- Harness target checks: run a selected command against the prepared delivery
  commit, retain logs and branch/SHA evidence, and gate Apply on successful,
  unchanged verification. Reuses the session queue, cancellation and timeouts;
  the UI defaults to requiring a check and restores previews after refresh.

- Harness Worktree delivery: select whole files, preview an immutable commit,
  preserve unselected staging and edits, then rehearse and apply one commit to a
  local target branch. Stale source/target previews and dirty/conflicting targets
  are refused; commit index recovery retains ownership checks after interruption.

- Harness Results & Verification: inspect baseline Git diffs, run explicit local
  checks with bounded output/time and cancellation, retain independent exit/status
  evidence and detect changed diffs. Persist per-launch usage and aggregate known
  money, tokens and time across follow-ups and Creator phases without treating
  unknown usage as zero or rewriting the agent outcome.

- Harness UI: persist and edit USD, reported-token and elapsed-time budgets for sessions, Fleet and Creator model phases; show last-launch usage, reject stale/active edits, preserve Fleet reservations and keep publication outside model budgets.

- Harness UI: add Infrastructure Creator scan/review/generate/apply and manifest-aware update runs, native provider/model/effort/agent/worktree settings, independent canonical validation, file decisions and durable publication recovery.

### Added

- Harness Skills records sources, immutable upstream commits and file hashes;
  provides source refresh, update/removal diffs and single-use apply actions.
  Local edits, extra files and unsafe paths block mutation. Tracking survives
  restart, includes locally created skills, and can adopt exact legacy copies.
- Harness Projects & Setup registers existing projects without a server restart,
  persists browser-added paths, reports readiness and installs selected edition
  tools through an immutable file preview. Reuses the accelerator installer's
  inventory and root-file merge rules, checks for source/target changes before
  applying, and reports collisions or partial filesystem failures explicitly.
- Browser sessions can bind a Project Brain task in their actual workspace,
  preview retrieved context before each turn and refuse stale context at launch.
  Explicit outcomes retain task revision checks; verified findings and decisions
  can be proposed, independently reviewed and applied to Memory Bank without
  automatically storing transcripts or promoting task records.
- Harness Project Brain browser and Memory Bank operations backed by the installed
  context CLI: indexed search, audit, validation, index maintenance, attestation,
  retirement, task/record lifecycle, compaction and privacy-filtered ZIP export.
  Browsing stays read-only; runtime mutations retain ownership and revision checks.
- Browser Fleet review runs the existing LangGraph graph with selected reviewers,
  native Claude/Codex/Cursor adapters, offline rehearsal, stage/reviewer status,
  persisted checkpoints, report approval/rejection and Markdown download.
  Reuses workspace isolation and Project Brain audit integration. Parallel budget
  shares and durable browser reservations prevent retries from reusing unknown
  spend; failed workers remain retryable failures instead of code findings.
- Harness project Git status and project/worktree execution selection. New
  worktrees use committed HEAD and a new branch, preserve the original checkout,
  and retain their execution directory for native session continuation.
- Harness Create skill section with an offline Markdown editor, project/tool
  selection and generated `SKILL.md` preview. Validates skill names and field
  sizes, quotes YAML metadata, and reuses the existing immutable installation
  preview and collision-safe writer.
- Harness Skills browser: select individual skills from catalog sources and
  install into registered Claude, Codex or Cursor projects after a file preview.
  Uses the existing pinned Vercel manager in private staging, preserves existing
  files, rechecks conflicts and paths, and lists installed skills. Adds Vercel,
  Anthropic and Supabase skill sources to the shared Kit 3 catalog.
- Browser Harness Memory bank viewer with project/bank selection, a filterable
  document list and read-only Markdown content. Fixed bank discovery includes
  the four source editions; bounded reads reject traversal, symbolic links,
  hard links and nonregular files without executing project memory scripts.
- Browser Harness model picker with custom IDs and per-model thinking effort
  choices. Model and effort can change between turns in the same native
  session; active runs and the other session settings remain fixed. Native
  Claude/Codex controls, Cursor model-variant guidance, local model metadata,
  validation and a nullable history migration avoid silently ignored options.
  Claude choices include explicit Fable/Opus/Sonnet/Haiku versions and clearly
  labelled auto aliases so users can choose a version instead of a moving alias.
  Claude Ultracode is an explicit workflow choice requiring additional agents;
  its helper count is labelled advisory, and ordinary resumed turns disable
  inherited Ultracode/workflows to preserve their agent controls.
- Browser Harness sessions can enable or disable additional agents and set
  a concurrent helper limit of 1–40. Settings persist through resume and
  restart; existing history migrates with helpers disabled. Codex and Claude
  receive native per-invocation controls; Cursor's instruction-only behavior
  and Claude's ultracode exception are visible in the UI.
- General browser Harness with a standalone localhost server (`./harness-server
  start/status/stop`), registered projects and native Claude Code, Codex and
  Cursor Agent sessions. SQLite history, resumable conversations, normalized
  live events, plan/edit permissions, workflow/context options, cancellation,
  bounded queue and timeouts use the Python standard library. Kit 1 project
  assessment, Kit 2 installer previews and the existing Kit 3 catalog live
  within the same workspace. Host/Origin/token checks protect local API
  mutations; the separate LangGraph batch runner remains optional.
- Kit 3 now has a searchable static web catalog and `./kit3` CLI. Search,
  client/category/review filters, resource details and copyable commands reuse
  the existing catalog and validated dossiers. `build` produces deployable
  static HTML; `serve` previews it on localhost without exposing the repository.
- Agent Skills add/list/update/remove delegate to pinned Vercel Skills 1.5.23.
  Updates explicitly select project scope by default; `--dry-run` runs no
  subprocess. Native installation state remains separate from `record` and
  `.kit3-manifest.json`. Non-skill resources retain manual guides.
- Kit 3 manifest and registry protections from the prior review: safe
  descriptor-relative atomic writes, strict identity/schema checks and
  recomputed review status. README refresh candidates remain advisory and
  never replace reviewed guidance. CI includes the fetcher and new CLI/site
  checks; catalog descriptions no longer treat popularity as a safety finding.


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

- **Capsule compaction now reports itself, and keeps both ends of a progress
  narrative** (from `feat/failure-localization-harness`, ported onto the
  rendered capsule). `enforce_capsule_budget` and the governed contract counted
  some of what they removed and nothing rendered the counters, so a capsule
  that had lost a constraint reached the prompt looking complete - the failure
  the interaction taxonomy calls context rationale erosion, blamed on the
  harness rather than on the model that later "optimized" the decision away.
  The rendered capsule (`render_capsule_lines`, the text every host puts in
  front of the model) gains a `compaction:` line after the warnings, naming
  every omitted section and instructing a re-read of the cited source; it is
  absent when nothing was omitted. Budget drops of semantic and episodic
  results and of the governed last-turn report are counted, and the governed
  contract adds what its projection limits hide (next steps, files, sources)
  to any count the working state already carried instead of replacing the
  counters. At the rendered ceiling the line yields after the excerpts and
  before any entry, so the drops it reports never cost the capsule one more
  item and `shown_in_render` still records exactly what the text shows.
  Progress truncation keeps a head and a tail (`truncate_progress`) rather
  than the tail alone: the opening of a narrative carries the constraint, the
  end the current state. `Capsule unavailable` warnings in the refresh report
  now state the consequence - no task context was assembled. Four new tests
  in `memory-bank/tests/test_context.py`; the one that pinned the tail-only
  cut now pins both ends.

- **Hooks no longer swallow their own failures.** `working-memory-read.sh`
  sent the refresh's stderr to `/dev/null` and exited silently whenever it
  printed nothing, so a crashed or timed-out memory refresh and a quiet one
  were indistinguishable from inside the turn. When the refresh exits non-zero
  without a report it now prints why (`exceeded its Ns budget` for 124,
  otherwise the exit status and the last line of the error, bounded to 160
  characters) and `Working memory was NOT consulted this turn`; a refresh that
  succeeds with nothing to say - a prompt the sanitizer emptied - stays
  silent, and the health line is still written first. The block is the
  `_WM_DELIVERY_PROMPT` text the Cursor mirror replaces, so the Cursor copy
  keeps answering `beforeSubmitPrompt` with JSON only. `subagent-dispatch.sh`
  wrote the completion journal's errors to `/dev/null` while the flow commands
  told the orchestrator that completions are recorded automatically; a failed
  write is now reported on stdout and stderr, and the Symfony flow commands
  tell the orchestrator to record the completion itself rather than read the
  gap as an unfinished agent. A missing governed task or a lightweight-mode
  project stays silent: with no channel, nothing was lost. Five new tests in
  `memory-bank/tests/test_hooks.py`.

- **`bash-validator.sh` gained a repetition guard** in all four editions,
  below the generic section. The file-edit loop detector cannot see a command
  loop, because rerunning one failing command touches no file: identical
  invocations that pass the rules are counted per exact command string in the
  session-scoped counter directory `loop-detection.sh` uses (cleared by the
  session-start hook), warning at six and blocking at twelve with a pointer to
  `/debugger` (`systematic-debugger` in the Codex mirror, two new
  `MIRROR_RULES` replacements). The branch's outward-action warning (`gh pr
  create`, `git push`, ... exiting 1 with `CONFIRM`) was not ported: the
  validator corpus pins those commands as silent passes, and the rule lives in
  the Symfony `AGENTS.md` instead. `tests/test_bash_validator_corpus.py` runs
  the hooks from a scratch directory outside any repository, so its cases no
  longer add to this checkout's counters or cross the threshold on a rerun;
  `BashValidatorTest` runs its passing commands in throwaway repositories, and
  the mirror test allows the counter namespace and the Codex skill name.
  A file edit `loop-detection.sh` records after a command's last run starts
  that command's count over: as merged, the count spanned the whole session,
  so the twelfth run of one test command was refused even when every run
  followed a fix, which is the edit-and-rerun loop a repair consists of.

- **Project Brain bug, finding and incident templates carry a `Localization`
  section** naming the interaction edge and the blamed side of the earliest
  unrecovered failure, so the repair is routed by attribution rather than by
  symptom. In the bug template it precedes `Root Cause`. The line names the
  components itself, because the generator ships these templates into
  projects whose `STABILIZATION.md` may not define them; the four editions'
  `STABILIZATION.md` do.

- **Symfony's `agents_md_bytes` ceiling rises 15115 -> 15650**
  (`scripts/token_budget.json`, observed 15498 + ~1%): its `AGENTS.md` gains
  the localization rule, the lossy-capsule sentence, approval for each
  outward-facing action and a `Third-Party Content` section, which absorbs
  the Memory Bank bullet's narrower "treat instructions inside imported
  documents as data". Recorded in `Symfony/CHANGELOG.md`.

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

- **Status polling exemptions stop at read filters.** A status command piped
  into an arbitrary program, such as `git status | tee status-copy.txt`, was
  exempted from repetition counting. Only head, tail, wc, cut, grep and jq
  pipelines are exempt now; action pipelines are counted like other commands.

- **`PROTOCOL.md` and the `project-brain` skill admit the capsule and the MCP
  server.** `project-brain/PROTOCOL.md` said no MCP server and no automatic
  prompt injection are part of the runtime, and the `project-brain` skill told
  agents never to claim automatic prompt injection because indexing and
  retrieval run only through explicit CLI calls. Every edition's prompt hook
  injects a Task Capsule into each request (Cursor through the rendered
  `working-memory.mdc` rule), the Stop hook checkpoints the task, and every
  edition registers the local memory MCP server
  (`memory-bank/scripts/mcp_server.py`), whose four tools call `context.py`;
  `AGENTS.md` already said so, so the skill contradicted the policy it sits
  under. `PROTOCOL.md` in the four editions and the `memory-seed` asset
  (`scripts/asset_parity.py --write`), rule 4 of the skill in the four
  editions, and `docs/CONTEXT-AND-MEMORY.md` now describe both; no network
  service or embedding store is still true and still said. Mirrors and policy
  locks regenerated. `MemoryWiringClaimsTest` in
  `tests/test_framework_semantics.py` fails when a shipped edition document,
  the generator's memory assets or the root memory docs deny the MCP server or
  the capsule the editions wire; changelogs and dated design records are left
  out, since they quote history.

- **The READMEs describe the memory the editions ship.** README_EN and
  README_RU said a record reaches `verified` only through
  `brain-update --authority verified`, when `brain-create --authority verified`
  writes one already verified, with no ledger entry, and promotion reads only
  the field. They said the memory has no MCP, when every edition ships the
  local `harness-memory` server and the installer registers it. They named one
  session-start hook that never injects context and left out
  `context-continuity.sh`, which keeps every chat's visible prompts and answers
  in `.context-handoff/` and delivers a prepared merge at session start. And
  they promised about five percent of context-budget headroom where several
  ceilings sit a few bytes above the measured value. Both READMEs now say what
  the code does: ceilings were raised inconsistently, some refit to the
  observed value plus five percent and others by exactly one change's growth,
  so headroom runs from a few bytes to just under five percent.
  `docs/SECURITY.md` and `docs/TOOL-INTEGRATIONS.md` no longer say the shipped
  Codex configuration has no MCP server; `docs/CI.md`, the `--headroom` entry
  in `scripts/README.md` and the `context_budget.py` docstrings no longer
  promise the headroom; and `install/README.md` names the chunk and
  Brain-runtime exclusions a seed chunk would fall under.

- **This changelog's history matches the code.** The 2026-08-12 entry said the
  installer was `main`'s version unchanged and that project chunks and Brain
  runtime stayed installed; the merge that wrote it had added
  `memory-bank/chunks/**` and four Brain-runtime patterns to
  `EXCLUDED_PATH_PATTERNS`, and its counts predated them. It now records the
  patterns and the committed inventory's counts. The 2026-08-10 entry, whose
  `PROJECT_WORK_AREAS` rule never reached this line, is marked superseded. The
  QA artifact integrity tooling entry, which a merge had placed by context
  inside `## 2.0.0`, moves to Unreleased: 2.0.0 has no `scripts/qa`. The
  context-handoff entry records the budget ceilings it raised.

- **`install_accelerator.py --sync` no longer applies part of a release over
  the runtime or hook wiring it keeps.** The sync, which the Harness runs by
  itself on every installed project, writes no file the project's Git tracks
  but did add the files a newer release brought. On a committed install of
  the release before the context handoff it added the `context-save` and
  `context-load` skills and commands, `context_handoff.py`,
  `context_continuity.py` and the continuity hooks, and kept the older
  `context.py`, `.claude/settings.json`, `.cursor/hooks.json` and
  `.codex/hooks.json`: `/context-save` failed with "invalid choice:
  'context-save'", no tool ran the new hooks, and the memory server it added
  beside an older `brain_runtime.py` crashed on import. Now every write is
  planned first and the release is applied only as far as what it runs
  follows (`HOOK_WIRING`, `apply_release`): while a runtime file
  (`memory-bank/scripts`, `project-brain/scripts`, `project-brain/schemas`)
  stays at the project's version, nothing else of the release is written; a
  tool's new hooks wait for its wiring, and the wiring for a new hook that
  cannot be written; the runtime is written first, and a part of it that
  fails holds back the rest. Each such file is listed under `kept` with a
  reason starting `held back:`, and the report's new `partial` field says the
  project is not at the release (null otherwise). A committed install takes
  a release with the new `--sync --update-tracked`, which a person runs (the
  Harness never does): it writes the files the project's Git tracks by the
  same rules, so an edited file, the project's own `README.md` and
  `.gitignore`, the text around `AGENTS.md`'s managed block, seeded state
  and the tools it did not install stay, and leaves the diff to review and
  commit; `partial` names it when tracking held the release back.
  `scripts/README.md`, `docs/ADOPTION.md` and `harness/README.md` say so,
  and that a reinstall with `--overwrite` is not an upgrade path (it
  replaced those files and, without the original `--tool`, added every
  tool). The Harness now tells the conversation when a sync held files back,
  a sync that wrote nothing included, and `Accelerators.last_sync` carries
  `partial`; when it puts Codex's approved wiring back because it cannot
  approve the new one, `partial` says the release's Codex hooks run only
  after the `/hooks` review. Tests: `tests/test_installation.py`
  (`SyncOfAnOlderReleaseTest`: a clone publishes the release before the
  handoff and then the current one; a committed install of the older one
  takes nothing, as a dry run and with `--rewire-codex`, and is pointed to
  `--update-tracked`; with it, a committed Claude-only install that has its
  own README, ignore rules, team rule and an edited skill takes the release
  and keeps all four, and its dry run writes nothing; an uncommitted one
  takes everything; new hooks wait for tracked Claude wiring and for Codex
  wiring awaiting approval; the wiring waits for a hook behind a link.
  `InstallSyncTest`: a runtime whose backup cannot be written now holds back
  the rest), `tests/test_harness_accelerators.py` (the held-back notice, and
  `partial` after Codex's wiring is put back).

- **The installation job has room to finish.** On PR #44 it ran 577 s of its
  600 s limit on a hosted runner: the per-edition install checks doubled with
  the context handoff, and its harness step varies by more than 100 s between
  runs. `timeout-minutes` is now 20 in `.github/workflows/ci.yml` and
  `scripts/check.py`, and `tests/test_check.py` (`test_timeouts_match_ci`)
  fails when a group's limit differs from its job's.

- **The merge archive's Windows file calls run in CI.** `harness.chat_merge`
  creates, reads, removes and recovers a merged chat's archive through folder
  handles, NT handles on Windows, but its tests ran only in the Linux job:
  they build a whole in-process Harness store and assert POSIX modes and
  links. `tests/test_harness_merge_archive.py` drives the same functions
  against the record table alone, without POSIX-only assertions and with a
  junction where POSIX uses a link, and runs in the `windows-harness` job and
  in the Linux harness step.

- **CLAUDE.md and the QA guides name the venv the `qa-tooling` check uses.**
  CLAUDE.md still called the repository's tooling standard-library only and
  gave no recipe for `scripts/qa/.venv`, and `docs/qa/` built the venv at
  `/tmp/accelerator-qa-venv`, where `scripts/check.py` never looks, so the QA
  checks were skipped before a push. CLAUDE.md now names both venv
  exceptions and has a QA block beside the harness one, and the QA guides use
  `scripts/qa/.venv`.

- **The `compaction:` line reports what the rendered capsule cuts, and the
  ceiling no longer removes it.** The line counted only what the JSON budget
  and the governed contract dropped, while the rendered text - the only form
  Claude Code, Codex and Cursor read - cut again: progress to its first 399
  characters, five of the task's files, the goal and each next step past
  their bounds, the last of five long paths in half, and every working source.
  A 1,100-character progress therefore reached the model without the step it
  ended on and with no compaction line, and a task with 20 files showed 5 and
  reported 12 omitted. `rendered_working_state` now returns those cuts and
  the line adds them to the JSON counters (20 files: 15 omitted); progress
  keeps its opening and its end, as `truncate_progress` does for the JSON;
  path lines stop at a whole path; and a `sources:` line shows the task's
  cited sources, the ones the line asks the model to re-read. At the
  3,600-character ceiling the line used to go whole before any entry, so a
  capsule whose JSON had dropped results read as complete exactly when it was
  tightest; it now gives up its counts for a fixed `compaction: lossy view`
  marker, which is never dropped and goes in before the first entry the
  ceiling drops. The marker costs about 100 characters in a capsule the
  ceiling cuts; the drop order (excerpts, the line's counts, skills, related
  knowledge, the weakest knowledge) and the 8,000/3,600 limits are unchanged,
  and `shown_in_render` still records exactly what the text shows. Tests in
  `memory-bank/tests/test_context.py` and the `WorkingStateCapsuleTest`,
  `RelatedItemBudgetTest` and `DeliveryTest` cases in
  `project-brain/tests/test_runtime.py`; the generator asset follows.

- **The repetition guard and the edit counter no longer trust a counter
  directory someone else made.** Both kept their counts in
  `/tmp/<host>-loop-detection-<repo-key>/`, a predictable name they accepted
  whoever had created it, and read and wrote the counters through symbolic
  links: another user on a shared host could plant a counter of 11 that
  blocked a first `php artisan test`, or link `cmd-<md5 of "git status">` to
  `~/.bashrc` and have the next `git status` overwrite it with a number.
  `loop-detection.sh` also fed a stored count to shell arithmetic, so a planted
  `a[$(cmd)]` ran `cmd`, and in the guard a stored `08` aborted the arithmetic
  and turned the guard off for that command. The counters now live in
  `${TMPDIR:-/tmp}/<host>-loop-detection-<uid>-<repo-key>/`, created with mode
  700; one that is a symbolic link or belongs to another user turns the count
  off, a counter that is a symbolic link is never read or written, and a count
  is accepted only as digits, read in base 10. The same in all four editions
  and all three hosts.

- **A command's repetition count is its session's, and polling is not a
  loop.** The count was keyed by the command and the checkout alone: every
  Claude Code, Codex or Harness session in one checkout added to it, and a new
  session's start wiped the counts of every session still running there. Counters are now keyed by the payload's `session_id` (Cursor
  `conversation_id`), and `local-context.sh` deletes only its own session's
  counters and those untouched for a day; the edit counts of
  `loop-detection.sh` are per session the same way. A read-only status query
  standing alone - `gh pr checks/status/view`, `gh run list/view/watch`,
  `git status`, `docker [compose] ps/logs`, `kubectl get/describe/logs`,
  `tail`, optionally after `sleep N &&` and piped into filters - is no longer
  counted: waiting for CI was refused on the twelfth poll. Chained with
  anything else it counts as before.

- **An edit by Codex, or by Claude Code's `Write`, restarts a command's
  repetition count.** The count restarts when `loop-detection.sh` recorded an
  edit since the command last ran, but Codex edits through `apply_patch`,
  whose payload has no `file_path`, and Claude Code ran the hook for `Edit`
  only. On Codex a fix-and-rerun loop was refused from its twelfth run, and
  on Claude Code whenever the fix was a `Write`. `loop-detection.sh` now
  counts every file an `apply_patch` call names on its `*** Add File:`,
  `*** Update File:` and `*** Move to:` lines, and a `NotebookEdit`'s
  `notebook_path`; `.claude/settings.json` wires it for
  `Edit|Write|MultiEdit|NotebookEdit`.

- **The repetition and edit-loop warnings reach the model on Claude Code and
  Codex.** Both warned on stderr or stdout with exit 1, which Claude Code and
  Codex treat as a non-blocking hook error shown to the user only, so the
  agent first met either guard at its block. Their Claude and Codex copies
  now exit 0 with `hookSpecificOutput.additionalContext`, which both hosts
  add to the model's context next to the tool result, as their hook
  documentation says. Cursor documents no such channel for `afterFileEdit` or
  for a shell command it lets run (`agent_message` accompanies a denial), so
  its copies still warn with exit 1 and the warning stays the user's; the
  hook READMEs and `docs/TOOL-INTEGRATIONS.md` say so.

- **Cursor no longer serves another task's working memory after a failed
  render.** The stop and session-start hooks removed the rule
  `.cursor/rules/working-memory.mdc` only when `hook-context` said the branch
  had no context (status 3); a failure, a timeout, a broken render or an
  enforce-mode skip kept the rule whichever task it named, and the prompt
  hook never touched it without a capsule. After `git switch` to a branch
  whose render kept failing - a name with `#` is enough, because the task ID
  is refused - Cursor sent the previous branch's capsule with every prompt.
  All three hooks now keep a rule they did not rewrite only when its header
  names the current task, and remove it otherwise; an enforce-mode skip still
  never empties the current task's rule.

- **A lost subagent completion reaches the orchestrator and names its error.**
  `subagent-dispatch.sh` reported a failed `msg-dispatch` on stdout and stderr
  from `SubagentStop`, where Claude Code shows a hook's output to nobody but
  the debug log, and the report quoted the first 160 characters of the error:
  for an uncaught exception, `Traceback (most recent call last):` and the
  install path. The report now quotes the error's last non-blank line, as
  `working-memory-read.sh` does, and on Claude Code it is also left in the
  ignored `memory-bank/local/unrecorded-completions`; the same script, wired
  on `PostToolUse` for `Agent|Task` (the event Claude Code documents for
  adding context to the parent after a subagent returns), hands it to the
  orchestrator once as `additionalContext`. Cursor's report stays in its
  hooks output.

- **The hook READMEs describe the repetition guard as it runs.** The Cursor
  READMEs listed only exit codes 0 and 2 for a validator that exits 1 on its
  warning, and no README said that an edit restarts the count; they now give
  each host's exit codes and warning channel, the edit reset, polling, the
  session keying and the counter directory.

- **Harness merge storage can be freed: a merged task's saved context can be
  deleted.** Saved merge archives were capped at 128 and 64 MiB for the whole
  state directory, and nothing ever removed one, so after the 128th merge
  every new merge was refused for good. A finished merged task now offers
  **Delete saved context** in the panel above the composer, which asks for
  confirmation before it deletes (`POST
  /api/sessions/<id>/delete-merge-context`). It empties the saved bundle and
  removes the archive folder; the task, its conversation, its source list,
  the titles of the chats it was about and the original chats stay. Only
  saved copies count towards the quota, so the storage is free at once, the
  refusal says how to free it, and a nested merge of such a task records
  `inherited_context_deleted` and those titles instead of the copy. A deleted
  copy cannot restart or start a first launch. Unknown files are never
  removed: a folder that also holds files an agent wrote there is left in
  place and named in the conversation, and a later start removes it only
  once it holds nothing but the archive.

- **Leaving a Harness merge draft keeps the project's new-session defaults.**
  `selectSession()` ended the draft before saving the form, so the values
  the draft forces (native workflow, project folder, Clash off) replaced the
  project's saved defaults after every successful merge, or when a history
  item was opened during a draft. It now saves first, while the draft guard
  still skips them, as `newSession()` already did.

- **The Merge picker no longer lists System Orchestration runs and AI
  scans.** The picker filters the session list, which holds summaries, and
  the summaries had no `system_run` or `system_discovery`; such runs were
  offered and only refused on submit. `Sessions.SUMMARY_FIELDS` now carries
  both.

- **Restart merged task rewrites a missing or changed archive from its
  record.** A first launch refuses an archive file that no longer matches the
  database record, and Restart requeued the same failing launch indefinitely.
  `restart_merge()` now rewrites the file from the record first (through
  rooted descriptors; a link in the folder's place is refused, not followed)
  and says so in the conversation.

- **A merged task's project memory names the merged chats.** With automatic
  memory, the first message became the Brain task goal and the first turn's
  query, and for most merges that is the prefilled "Continue the work from
  these chats..." with no subject. Memory now leaves the prefilled
  instruction out. An instruction the person wrote still leads the goal and
  the task ID, with the chats' titles after it, so long titles cannot push it
  out of the 200-character goal; when only the prefilled instruction is
  there, the goal starts with the titles and the task ID reads like
  `harness/merged-design-the-payment-retry-...`. The query also carries each
  chat's first request. A merged chat is named by the chats it merged, also
  after its saved context was deleted. The first message itself stays as
  written.

- **A chat capture and a merge delivery stay inside the hook budget however
  long the branch's chats are.** Every prompt and stop re-read all of the
  branch's snapshots and ran the full secret scan over each (up to eight
  times 4 MiB), and scanned the new text twice; delivering a merge re-scanned
  every frozen source of an archive of up to 32 MiB. Eight chats of 3.5 MB
  took about 9 s per capture and per delivery, so `timeout 5` killed the
  capture after it wrote its snapshot but before it evicted one - the
  8-per-branch cap stopped holding and each later turn cost more - and killed
  the delivery before it claimed the pending merge, which then cost every new
  session 5 s and delivered nothing. Each turn's new text is now scanned once;
  snapshots and archives are read back by the sha256 recorded when their text
  passed the scan; eviction runs first, from file modification times, without
  opening a snapshot. `--event merge` (a command, no hook budget) scans its
  sources again under the current patterns before freezing them, and the few
  kilobytes a new task receives, and each `list` preview, are scanned once
  more. Both paths now take about 0.1-0.2 s beside eight 3.5 MB chats; the
  continuity tests time them against half the default budget.

- **`.context-handoff/` keeps itself out of Git.** `install_accelerator.py
  --sync`, which the Harness runs for every installed project, wires the
  continuity hook into a project installed before it existed but keeps that
  project's own `.gitignore`, so the visible chat text it then captured showed
  up as untracked files a `git add -A` would commit. The store now writes its
  own `.gitignore` containing `*` when it is created, or on the next capture
  into a store that predates it, and never writes through a symlink there.

- **Chat snapshots are bounded across branches, not only per branch.**
  Eviction looked only at the current branch's valid snapshots, so those of
  deleted branches, of a checkout that had moved (its `repository_id` no longer
  matched) and of a pattern added since stayed on disk for good, as did a
  temporary file of a write the hook budget killed. Each capture now keeps 64
  snapshots per checkout and drops any idle for 30 days, besides eight per
  branch; the hook READMEs and `docs/CONTEXT-HANDOFF.md` state the bounds.

- **A Codex chat's snapshot starts with what the person typed.** Codex
  records the project's AGENTS.md (`# AGENTS.md instructions for ...`),
  `<environment_context>`, skills, hook prompts and notifications as
  user-role messages, and the projection kept them as `User:` text: every
  Codex chat of a project opened with the same AGENTS.md text, so the `list`
  previews the `context-load merge` skill selects from were identical, and a
  merged preview spent its first third of each source on policy. Such parts
  (known context envelopes or either AGENTS.md header) are dropped now; a
  `<pasted_content>` paste is the person's and stays.

- **`list` and `merge` fail explicitly when chat continuity is disabled.**
  With `CONTEXT_CONTINUITY_DISABLED` set, or a state root that does not exist,
  both returned 0 with no output, so an agent following the skill reported a
  merge as prepared that was never written. They now exit 1 with `Context merge
  failed:` and the reason, as `docs/CONTEXT-HANDOFF.md` promised; the hook
  events stay silent.

- **A committed handoff loads in a CRLF checkout.** Git for Windows' default
  `core.autocrlf=true` checks a committed `tasks/TASK-NNN/context-save-*.md`
  out with CRLF, and `context-load` refused it as "missing JSON frontmatter".
  A handoff that does not validate as read is now checked once more with CRLF
  turned back into LF, and its digests decide. A new handoff containing a
  CRLF transcript stores its exact text as a JSON string (version 2), so
  commit-time normalization cannot change its bytes; version 1 remains readable.

- **`context-save --topic` and `--task-id` refuse personal data.** Both went
  into the tracked handoff after only the secret scan, while the curated
  fields beside them also refuse personal data; an email address in the topic
  was saved. They are screened like the curated fields, on save and on load.

- **`context-save` works in a project outside Git.** Every cited file was
  checked with `git check-ignore`, which exits 128 outside a repository, so a
  handoff citing any file failed with "cannot verify source ignore rules" -
  in a project without Git and when loading a handoff from a copy without
  `.git`. Without Git metadata there are no ignore rules to honour and the
  files are fingerprinted directly; where `.git` exists, a check that cannot
  run still refuses.

- **QA run evidence is complete on a detached checkout.** `run_tc_ai`
  recorded `git branch --show-current` as the environment's `branch`, which is
  empty where CI checks a pull request out as a detached merge commit, so the
  evidence validator rejected `environment.json` as incomplete and the
  `qa-tooling` job failed on its first pull request. A detached checkout is now
  recorded as `detached at <commit>`, with a regression test.

- **A project built inside an edition keeps its work under `Task/`.** The
  client-application branches built an application inside `Laravel/` and
  `Symfony/` and left its derived specs, `codebase/` map, five memory chunks
  and governed Brain records in the editions' own `specs/`, `codebase/`,
  `memory-bank/chunks/` and `project-brain/`. There the editions' hooks and
  index read them as the edition's memory (the Symfony session hook counted
  six chunks and reported `brain-validation=invalid`), the installer listed
  the specs as distribution files, the Laravel index masked three values in a
  schema spec, and two of its PHP snippets failed the lint. The 52 files now
  sit at the same relative paths under `<Edition>/Task/project-work/`, with
  the project's `specs/MANIFEST.md` and `.memory-counter` beside them; the
  editions' own copies are back to their seeds. `scripts/check_php_snippets.py`
  no longer lints `Task/` material, which the installer already excludes.

- **The QA tooling job runs locally, and its evidence names the interpreter
  in a repository venv.** `qa-tooling` came in from the QA branch with a
  `pip install` into the runner's own Python, which `scripts/check.py` cannot
  mirror without installing third-party packages into the user's interpreter.
  It now builds `scripts/qa/.venv` from the pinned `requirements-qa.txt`, as
  `harness-fleet` builds `harness/.venv`; `check.py` has a `qa-tooling` group
  that skips with the venv recipe when the venv is absent and fails under
  `--strict`. `run_tc_ai.sanitized_argv` replaced the repository root before
  the interpreter, so an interpreter inside the repository was recorded as
  `{root}/...`; the longest path is now replaced first.

- **A completion whose commit fails no longer leaves its event behind.**
  `complete` writes the Git-tracked `event` before the SQLite commit, and the
  rollback restored only files that existed beforehand: the task, its handoff
  and the indexes. A commit that failed on disk or I/O therefore reopened the
  task but kept a "Completed ..." event outside the restored index, so
  `validate` reported the active index as stale, and a retried completion wrote
  no event of its own (`event_id: null`) because the orphan already held its
  external ID. The created event is now part of the same compensation. A
  regression test fails the commit once the event exists and checks that no
  event remains, the task, handoff and indexes are byte-identical, `validate`
  is clean, and a retry writes exactly one event.

- **Prompt distillation now preserves terms that exist only in local
  episodes.** The prompt hook previously ranked words against indexed files
  alone, so a natural prompt containing one repository word plus a distinctive
  episode-only phrase could discard the phrase before the episode search ran.
  Distillation now measures the combined searchable corpus, and an end-to-end
  `record` -> `refresh --query` regression covers the real prompt path.

- **The memory lifecycle now passes end to end through the public CLI and a
  clean installed runtime.** A governed finding can move from observed to
  verified and resolved, be promoted automatically into a source-digested
  `MEM-*` chunk, complete its task into an episode, and be retrieved from a
  different task. The same smoke now installs every edition for all three
  supported tool surfaces and exercises `start`, source-backed `brain-create`,
  indexing, and `links --path` under a hostile project `.gitignore`.
  - Promotion discovery and apply share one eligibility check for record type,
    terminal state, verified authority, privacy, useful content, and source
    freshness. Apply no longer admits a stale, empty, or provenance-tampered
    reviewed proposal; human and automatic outcomes are bound to their review
    modes, while legacy applied v1 records remain readable. The `complete`
    status counter also reads the `(path, promotion)` iterator correctly.
  - Retrieval manifests are v3 and attribute the real host and entry point.
    Repeat detection is isolated per task/host/entry point and includes source
    hashes, task revision, and local episode content, preventing stale content
    and Cursor task updates from false `repeat-retrieval` skips. Local episodes
    enter the gate before its decision, consume the same token budget, and are
    reported by count and token cost without persisting their id or body.
    Reports separate shadow `would_skip` from enforce `withheld`, split slices
    by mode, and aggregate token estimates and per-layer no-match signals.
  - Automatic checkpoints now emit edition-relative paths and collapse large
    untracked directories instead of expanding vendor/cache trees. Subagent
    write-lock tests use isolated directories; explicit overrides must be safe
    absolute writable directories, and an invalid cleanup override cannot
    suppress the completion journal.
  - `memory-probes.json` is part of cross-edition parity, and every edition's
    existing `MEM-0001` was reverified with current source digests.

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

### 2026-08-12 the Symfony inventory, regenerated under schema 2

This entry and the next come from the client-application branches, which
built a project inside `Symfony/` and `Laravel/`; they reached this history
when those branches were merged into the integration of all branches on
2026-10-10. Corrected on 2026-10-10: as first written, this entry said the
installer was `main`'s version unchanged and that project chunks and Brain
records stayed installed, while the merge that wrote it had added the
exclusions below, as its commit message says, and its inventory counts were
taken before it did.

- `scripts/install_accelerator.py` takes `main`'s version instead of this
  branch's own. The `PROJECT_WORK_AREAS` prefix list this branch added on
  2026-08-10 solved the same problem — a project built inside an edition
  should not have to be listed as a distribution file — and `main`'s
  `excluded_tracked_paths` solves it better: the exclusions are recorded in
  the inventory itself and validated on load, rather than implied by a tuple
  in the script, and `Task/**` covers what `Task/app/` covered. Two mechanisms
  for one rule would be worse than either, so the earlier one is removed
  rather than merged alongside.
- `main`'s `EXCLUDED_PATH_PATTERNS` gains two groups of patterns, each
  `main`'s own intent applied to a tree `main` never had.
  `memory-bank/chunks/**`: the production index override
  (`memory-bank/.install/INDEX.md`) ships an empty table, so a clean install
  is meant to carry no chunks, and `MEM-0001` was
  excluded by exact path only because it was the one chunk `main` had; the
  project's own chunks failed validation twice once installed, absent from
  the shipped `INDEX.md` and citing sources under `Task/app/` that are not
  installed. The governed Brain runtime, matched by file type so the
  `.gitkeep` placeholders that create its directories still ship:
  `project-brain/control/handoffs/*.md`,
  `project-brain/control/messages/*.jsonl`,
  `project-brain/control/retrieval-manifests/*.json` and
  `project-brain/dynamic/*/*.md`; the shipped
  `project-brain/indexes/active.json` is `[]`, so another project's tasks
  would make it stale on arrival.
- `install/inventories/symfony.json` regenerated for this branch's tree: 634
  installed, 811 excluded. The 760 files of `Task/app/` land in
  `excluded_tracked_paths`, which is what the 2026-08-10 entry was after.
- Still recorded as installed: the project's derived `specs/` and its
  `codebase/` map, 22 files that match no exclusion pattern. They left the
  payload on 2026-10-10, when the integration moved them under
  `Task/project-work/` (see "A project built inside an edition keeps its work
  under `Task/`" under Fixed above).

### 2026-08-10 installation inventories exclude project work — superseded

Superseded on 2026-08-12 by the entry above: `scripts/install_accelerator.py`
took `main`'s `excluded_tracked_paths` instead, and the `PROJECT_WORK_AREAS`
rule described here is not in the code. Until the integration moved them under
`Task/project-work/`, the derived `specs/` and `codebase/` this rule skipped
were recorded as installed. Kept as the record of what the branch tried.

- `scripts/install_accelerator.py` no longer requires an inventory entry for
  files an edition accumulates while a project is built inside it: `Task/app/`,
  `codebase/`, derived `specs/`, project memory chunks, and the governed brain
  runtime under `project-brain/control/` and `project-brain/dynamic/`. The
  inventory's own scope already read "excludes runtime, local, and user state";
  the discovery step now matches it. Listing such files instead would make the
  installer copy one project's application into every consumer's tree.
- A path in those areas is skipped only when the inventory does not list it, so
  the seeds that genuinely ship — `specs/MANIFEST.md`, the `.gitkeep`
  placeholders, the starter memory chunk, the empty brain indexes — stay
  verified, deleting one still fails, and an unlisted file outside those areas
  is still reported. The trade-off is recorded in the module: a distribution
  file mistakenly placed under one of those prefixes is no longer caught here.

### 2026-08-06 QA artifact integrity tooling

Made on the side branch `fix/updates-after-tests` and not part of the 2.0.0
release, which has no `scripts/qa`; it reached this history when that branch
was merged into the integration of all branches on 2026-10-10. The
`qa-tooling` job's repository venv and `check.py` group are under Fixed above.

- Added strict schemas and fail-closed builders for the 75-run disposition
  ledger and reconstructed workbook. Builders require the complete,
  checksum-verified historical package, use composite run/case/defect
  identities, namespace only the four Codex defects, and never emit partial
  or fabricated historical artifacts.
- QA artifact validation now applies Draft 2020-12 schemas to the catalog,
  ledger, and run evidence, confines checksum entries to their evidence root,
  binds run/case/classification identity, cross-checks assertion status and
  ancillary evidence documents, ties each command exit code to a unique
  assertion, rejects malformed/duplicate checksum entries and evidence
  symlinks, and requires a complete consistent package before approving
  reproduced passes. CI validates the committed ledger and reconstructed
  workbook.
- Added the data-driven `TC-AI-001..018` runner with deterministic command
  mappings, focused-regression discovery gates, explicit native-host skips,
  split command streams, immediate exit codes, assertion arithmetic, Git
  snapshots, provenance, inventories, and per-run SHA-256 manifests. Optional
  local tools such as ShellCheck are now recorded as explicit skips when absent
  instead of being misclassified as accelerator failures. Embedded shell
  commands use the pinned QA interpreter, while generated evidence redacts
  machine-local repository and interpreter paths.
- Added a reproducible disposition-policy overlay that assigns approved owner
  labels, proposes package-backed historical Pass dispositions, binds
  checksum-addressed Codex rerun evidence, and records the
  `evidence-backed-local-v1` approval boundary. Only checksum-backed,
  defect-free historical Passes and fully passed Codex reruns are approved;
  historical non-passes, defect-bearing historical Passes, skipped/failed
  checks, native observations, and external findings remain pending.
- Added synthetic QA tooling tests, CI coverage, minimal non-client fixtures,
  and release validation for exact IDs/counts, workbook tables/formulas,
  evidence checksums, RUN-057 withholding, and final-disposition strictness.

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

### 2026-08-03 QA defect fixes (TC-066, TC-021/TC-061/DEF-007) — superseded

Superseded before release. These fixes were made on the side branch
`merge/context-brain-reconciliation` (2026-08-05) and reached this history
only when that branch was merged into the integration of all branches on
2026-10-10. Their code was not kept: the 2026-08-06 hook and installation
hardening and context-runtime remediation above fixed the same defects
differently (builtin stdin reads, direct queries refused before any state is
touched, a five-phase stored vocabulary with aliases, a pre-provision warming
capsule for Cursor), and the regression tests named below do not exist in
that form. Kept as the record of what the QA run found.

- **`refresh --query` privacy gate now screens the raw prompt (TC-066)** -
  the refresh path distilled the raw prompt into retrieval terms before the
  capsule privacy gate ran, and distillation tokenizes away the very
  characters the private-data patterns match on (`@`, dots, `user:`
  prefixes), so a secret-like raw prompt bypassed the refusal that
  `retrieve`/`context` already gave and its distilled tokens were persisted
  in the retrieval manifest. `context.py refresh` now applies
  `reject_capsule_privacy` to the raw `--query` text next to the existing
  secret gate, before any distillation: a rejected prompt is never
  distilled, no retrieval runs, and no manifest is written, while the layer
  refresh itself still succeeds with a "Capsule unavailable" warning.
  Applied to `Laravel/memory-bank/scripts/context.py` and propagated
  byte-identically to the Symfony and PHP Core copies in the core-sync step.
  Regression test: `test_refresh_rejects_private_raw_query_before_distillation`
  in `memory-bank/tests/test_context.py`.
- **`--phase` accepts the documented skill vocabulary (TC-021/TC-061/DEF-007)** -
  `context.py update --phase` and `brain-update --phase` accepted only the
  four stored values (`understanding`, `planning`, `execution`,
  `finalization`) with no help text, while the skills' own Phase Map names
  the delivery steps Understanding / Planning / Implementation / Quality and
  the verify skill calls its stage "verification" - so agents following the
  skills were rejected (TC-021 `verification`, TC-061 `implementing`). The
  CLI now accepts `implementation`, `quality`, and `verification` as
  documented aliases that are recorded as `execution`, the canonical phase
  those same skills declare in their frontmatter; `--phase` help states the
  normalization, and `project-brain/PROTOCOL.md` documents the vocabulary,
  the alias mapping, the deliberate absence of `utility`, and the absence of
  phase-ordering constraints. Stored records and the dynamic-record/handoff
  schema enums are unchanged, so existing records stay valid byte for byte.
  Regression tests in `memory-bank/tests/test_context.py`: every documented
  CLI value maps to its stored canonical phase, all twelve ordered
  canonical-phase transitions are exercised, the alias path works through
  `brain-update` on the task record, and non-vocabulary values (TC-061's
  `implementing`) are still rejected with the full documented list.

### 2026-08-03 QA defect fixes (DEF-001/TC-004, TC-030) — superseded

Superseded likewise; see the note under the previous heading.

- **Cursor cold start gets a warming-up rule (DEF-001/TC-004)** - the Cursor
  mirrors of the Stop and sessionStart hooks used to render
  `.cursor/rules/working-memory.mdc` only when the capsule render produced
  output, and the working task auto-provisions only on the flush boundary
  (`--flush-after`, default 5), so the first turns of a fresh Cursor session
  ran with no working-memory rule at all. The `_WM_DELIVERY_*_CURSOR`
  transforms in `memory-bank/scripts/context_retrieval.py` now render a
  warming-up placeholder on a cold start (task id, provisioning note, and
  the last turn report from `memory-bank/local/` when present). A capsule
  already rendered for the current task is never downgraded to a placeholder
  (one turn stale beats empty), while a rule left over from a different task
  is replaced instead of being served as the current task's memory.
  Governed-memory semantics are untouched: provisioning still happens only
  at flush time, and `context.py context` still requires a bound task.
  Regression tests: the `CursorCapsuleRenderTest` cold-start/placeholder
  cases in `memory-bank/tests/test_hooks.py`.
- **`bash-validator` reads stdin without external `cat` (TC-030)** - the
  validator read its payload with `INPUT=$(cat)` before the JSON-extractor
  guard, so on a system without `cat` in PATH the payload was lost, every
  command (including destructive ones) passed silently, and the fail-open
  "no JSON extractor available" warning could never fire. Stdin is now read
  with the bash builtin (`IFS= read -r -d '' INPUT`) in every edition's
  canon and mirrors, so validation and the fail-open warning survive
  `cat`-less systems. Regression tests: the restricted-PATH cases in
  `memory-bank/tests/test_hooks.py` (per edition) and
  `Infrastructure-Creator/tests/test_hooks.py`.

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
