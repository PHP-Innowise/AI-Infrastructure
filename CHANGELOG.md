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

- `scripts/install_accelerator.py` reverts to `main`'s version. The
  `PROJECT_WORK_AREAS` prefix list this branch added on 2026-08-10 solved the
  same problem — a project built inside an edition should not have to be
  listed as a distribution file — and `main`'s `excluded_tracked_paths` solves
  it better: the exclusions are recorded in the inventory itself and validated
  on load, rather than implied by a tuple in the script, and `Task/**` covers
  what `Task/app/` covered. Two mechanisms for one rule would be worse than
  either, so the earlier one is removed rather than merged alongside.
- `install/inventories/symfony.json` regenerated for this branch's tree: 645
  installed, 800 excluded. The 760 files of `Task/app/` land in
  `excluded_tracked_paths`, which is what the 2026-08-10 entry was after.
- Left as `main` classifies them, and flagged rather than changed here: the
  derived `specs/`, `codebase/`, `memory-bank/chunks/` and the governed brain
  runtime under `project-brain/control/` and `project-brain/dynamic/` match no
  exclusion pattern, so they are recorded as installed — 35 files of one
  project's own output that an installation would copy into a consumer's tree.
  Extending `EXCLUDED_PATH_PATTERNS` would fix it in one line, but that is
  shared-core policy belonging to whoever reworked this module, not to a pull
  request about the Symfony application.

### 2026-08-10 installation inventories exclude project work

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
