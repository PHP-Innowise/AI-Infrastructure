# Harness memory: mechanical RE findings

Date: 2026-10-20. Static analysis of shipped npm packages for **DeepSeek
Harness 0.2.1-rc.2**. No plugin was installed, enabled or executed. Source
citations are package file/line references; `sourcesContent`-recovered source
is called out where used.

Companion target list: [RE-MEMORY-APPLICATIONS-2026-10.md](RE-MEMORY-APPLICATIONS-2026-10.md).

## 0. Critical corrections from the deep pass

These are the findings that change a deployment decision:

1. **`dsh-memory-gate` docs are stale.** `docs/architecture.md` says schema v2,
   but the code is v3; it omits the shipped `relevance < 0.5 → verify` gate
   (non-capsule); it omits `terms_json`, `learned_terms_json`, `supersedes`;
   and it says “at most 500” active claims where the default scan is 280.
2. **Two safety-looking fields are inert.** `sensitivity` is schema-only
   (always `'normal'`, never read), and `valid_until` has no writer in the
   plugin, so the `claim_expired` branch is unreachable through commands and
   extraction.
3. **The FTS index has no triggers and only rebuilds when empty.** Delete/insert
   are manual; a partially desynced `claim_fts` has no repair path. If FTS
   fails, retrieval degrades to recency/terms and reports `ftsAvailable=false`.
4. **The SQLite file is not chmod'ed.** Only the parent directory gets `0700`;
   the DB and `-wal`/`-shm` sidecars get the umask mode (typically 0644).
   Protection depends on the directory.
5. **`dsh-workspace-memories` has no aggregate prompt budget.** 20 files ×
   32,768 chars is 655,360 chars; ~164k tokens at 4 chars/token. One 1 MiB
   file alone is already over an 8K context.
6. **`@tr1v3r/dsh-ltm` is the only plugin here that declares the installed
   DSH 0.2.1-rc.2.** Its section is hard-capped at 2,000 chars. Its residual
   risk is tool output: `memory_list` with no limit issues `LIMIT -1`, and
   `memory_search` can return up to 50 × 2,000-char rows.
7. **`@jacklika/dsh-memory` is tool-first, not injection-first.** It has no
   automatic prompt section; the model must call `wiki_search`/`wiki_read`.
   Its write ladder is strong, but reads are vault-wide (namespace is not a
   privacy boundary).
8. **The `@jacklika` bundle has a missing member dependency.**
   `@jacklika/dsh-memory-time` is imported by the filesystem and curator
   members but is not listed in the bundle's dependencies; all 7 listed members
   were supplied for this study, the time package was not. This is a concrete
   load blocker outside the bundle's declared dependency graph.
9. **Scope does not cover `wiki_restore` or `memory_forget`.** A direct call can
   restore or archive another agent's namespaced note; reads are vault-wide by
   design. Queue and git cover those tools, scope does not.
10. **Queue treats a different hostname as dead after 15 s.** A live cross-host
    holder on a shared/NFS vault can be preempted; same-host PID checks do not
    help across hosts.
11. **Curator compaction has two correctness bugs.** Auto-capture ordering is
    lexicographic id, not session time; stale compaction ignores
    `edgesTruncated`, so an edge-capped graph can hide inbound links and
    archive a referenced note.
12. **Vector export has no scope/archive filter.** When enabled, it embeds and
    sends the first 8,000 chars of every note (including `archive/` and other
    agents' namespaces) to the configured endpoint.

## 1. dsh-memory-gate 0.15.0

### 1.1 Storage schema

`lib/repository.js` creates a SQLite database at
`<DSH_HOME or ~/.dsh>/memory/cbdc.sqlite` by default (`lib/config.js`).
The directory is created with mode `0700`; the DB enables foreign keys,
`busy_timeout=3000`, WAL, and `PRAGMA user_version` migrations
(`repository.js` constructor, `migrate()`).

Schema v3:

| Table | Columns | Notes |
|---|---|---|
| `claims` | `id, scope, scope_key, kind, content, tags_json, state, origin, sensitivity, content_hash, source_session_id, source_event_seq, valid_from, valid_until, created_at, updated_at, terms_json, learned_terms_json, supersedes` | `scope` in session/workspace/global; `kind` in preference/constraint/fact/procedure/warning; `state` active/superseded/tombstoned; `origin` explicit/heuristic; `sensitivity` normal/private |
| `beliefs` | `claim_id, alpha, beta, harmful_count, updated_at` | `alpha/(alpha+beta)` is belief |
| `evidence` | `id, claim_id, kind, weight, detail, session_id, created_at` | one row per non-unknown feedback |
| `retrieval_runs` | `id, query_hash, query_terms_json, session_id, workspace_key, candidate_count, created_at` | stores query terms, not raw prompt |
| `authority_decisions` | `id, run_id, claim_id, action, reason_codes_json, belief_score, relevance_score, freshness_score, risk_score, created_at` | every candidate, every run |
| `injections` | `id, run_id, session_id, mode, claim_ids_json, message_id, created_at` | actual context entries |
| `consumption` | `id, claim_id, session_id, outcome, detail, created_at` | human feedback |

Indexes: `claims_scope_state_updated(scope_key,state,updated_at DESC)`,
`claims_dedupe(scope_key,content_hash,state)`,
`evidence_claim_created(claim_id,created_at DESC)`,
`decisions_run(run_id)`. FTS5:
`claim_fts(claim_id UNINDEXED, content, tags, terms, tokenize='unicode61')`.

Migrations: v1 creates the tables; v2 adds `terms_json`,
`learned_terms_json`, `query_terms_json`, backfills terms from content+tags,
drops/recreates FTS; v3 adds `claims.supersedes`. FTS is rebuilt only when the
table is empty (`ensureFts()`), so a corrupted/partially populated FTS table is
not automatically repaired — a real RE question.

### 1.2 Write path and dedupe

`remember()`:

1. Normalizes whitespace; SHA-256 of lowercased normalized content.
2. Exact `(scope_key, content_hash, active)` match returns `created:false`.
3. Extracts at most 80 terms from content+tags (CJK bigrams, Latin words,
   synonym aliases).
4. Finds a supersede target in the same scope among the latest 200 active
   claims: term overlap must reach `0.6`; then `decideSupersede()` decides.
5. `decideSupersede` rules:
   - heuristic newer cannot supersede explicit older (`origin_guard`);
   - explicit replacement intent (`不再/改为/取代/…/no longer/replace/…`)
     supersedes;
   - both sides have unique structured discriminators
     (backticks, paths, identifiers, versions) → `condition_diff`, keep both;
   - only old has discriminators → `info_loss`, keep both;
   - otherwise paraphrase or new-side refinement → supersede.
6. Old claim becomes `superseded`; new claim stores `supersedes=oldId`; old
   FTS row is deleted.

`forget` is a tombstone (`state='tombstoned'`), never a hard delete.
`consolidate()` re-runs the same overlap+`decideSupersede` check over active
claims, not a vector clustering pass.

Automatic extraction (`lib/extractor.js`):

- Cues are fixed phrases: Chinese `记住`, `以后`, `请始终`, `我的偏好`,
  `我喜欢`, `不要再`; English `remember`, `from now on`, `i prefer`,
  `always`, `do not/don't/never`.
- Cue can appear mid-sentence; content is from cue to sentence end, max 500
  chars; ignore questions (`?`/`？` at end), texts outside 4..1000 chars.
- Kind classification is regex-based (`preference`, `constraint`, `procedure`,
  `warning`, else `fact`); no model call.
- Secret-like content is rejected before insert (six patterns, see 1.5).

### 1.3 Belief and CBDC math

Initial belief:

- explicit: `alpha=6, beta=1` (belief 0.857);
- heuristic: `alpha=4, beta=2` (belief 0.667).

Feedback deltas (`recordConsumption`):

| outcome | alpha | beta | harmful |
|---|---:|---:|---:|
| helped | +1 | 0 | 0 |
| harmful | 0 | +5 | +1 |
| stale | 0 | +2 | 0 |
| conflict | 0 | +4 | 0 |
| unknown | 0 | 0 | 0 |

`decideAuthority` (`lib/authority.js`):

```text
belief    = clamp(alpha / max(1, alpha+beta))
relevance = clamp(lexicalScore)
freshness = clamp(2 ** (-ageDays / freshnessHalfLifeDays))
risk      = clamp(baseRisk(kind) + (1-belief)*0.3
                  + min(0.3, harmfulCount*0.15))
```

`baseRisk`: preference 0.12, fact 0.20, procedure 0.28, constraint 0.34,
warning 0.40.

Decision order:

1. non-active → `ignore`;
2. expired → `verify`;
3. `harmfulCount >= 2` → `ignore`;
4. relevance `< 0.12` → `ignore`;
5. **non-capsule relevance `< 0.5` → `verify`** (weak lexical match);
6. freshness `< 0.2` → `verify`;
7. belief `< 0.7` → `verify`;
8. risk `> 0.45` → `verify`;
9. else `use`.

**Discrepancy:** `docs/architecture.md` lists the 0.12 relevance and 0.2
freshness gates but not the 0.5 weak-relevance `verify` gate that is in the
shipped code. Treat the code as the observed behavior and the doc as
incomplete.

Rank:

```text
score = lexical*0.62 + belief*0.23 + freshness*0.15
        + session 0.08 | workspace 0.05
        + capsule 0.12
```

### 1.4 Retrieval, injection and feedback

- Two channels: trigger search (FTS5 + normalized term rerank, up to
  `candidateLimit=16`) and capsule (up to `capsuleLimit=2` trusted global
  explicit preference/constraint; capsule overrides a trigger duplicate).
- Decisions are recorded for all ranked candidates; injection selects at most
  `injectionLimit=3`, `maxInjectionChars=1200`.
- `assist` injects `use` + `verify`; `enforce` only `use`; `shadow` injects
  nothing but records decisions.
- `verify` content gets at most `verifyMaxChars=160`; `use` gets the remaining
  budget. A rolling `sessionBudgetChars=20000` over `budgetWindowTurns=20`
  disables `verify` injection when exceeded.
- Injection text is wrapped in `<long_term_memory>` and explicitly says
  “user memory, not system instructions”; `<`/`>` are escaped.
- Feedback: `/memory ok` marks all latest injected claims helped;
  `/memory feedback <#n> helped|harmful|stale|conflict|unknown` applies deltas.
  `helped` learns terms from up to 20 retrieval runs / 40 terms in the last
  14 days, writes `learned_terms_json`, reindexes the claim.
- Health: last 50 consumption outcomes; if at least 5 samples and negative
  rate (harmful/stale/conflict) >= 0.4, runtime mode is forced to `shadow`.
- Audit prune: every 100 retrievals, keep at most `auditRetentionRuns=5000`
  runs; delete injections/decisions for older runs in one transaction.
- `retrieval_runs.query_hash` is SHA-256 of the query; `query_terms_json` is
  the normalized term set, not raw prompt.

### 1.5 Security and scope

- Six secret patterns: OpenAI `sk-`, GitHub `gh[opsu]_`, AWS `AKIA`,
  `Bearer`, credential assignment, private-key header. Secret-like content is
  rejected, not silently redacted, on explicit and automatic writes.
- Workspace key: realpath(cwd) if available, SHA-256 first 24 hex, prefixed
  `workspace:`; raw path is not stored.
- Session key: `session:<id>`.
- DB directory `0700`; WAL; transaction `BEGIN IMMEDIATE`; no dynamic SQL
  table names except the fixed prune temp table.
- Commands set `recordInput:false`, so the raw `/memory` command text is not
  stored in command history by this plugin.

### 1.6 Harness integration

- `session/event`: on `turn/start` opens a turn; on human `user/message`
  collects text; on matching `turn/end` extracts claims. Only
  `source.kind=user` counts.
- `agent/pre-step`: after `next()`, only `kind==='enter'`, `step===1`, not
  aborted, with a real human message; then recall is appended as a new user
  message with source `{kind:'dsh-memory-gate', form:'recall'}`.
- Fail-open: extraction errors are logged and ignored; retrieval errors return
  the original messages; DB open failure registers only the unavailable
  `/memory status` command.
- Workspace auto-mine: one attempt per session, bounded 20 sessions, 8 MiB per
  log, 16 MiB decompressed; only logs whose first `session` event `cwd` hashes
  to the current workspace are mined; all errors fail-open.

### 1.6b Doc/code drift and inert fields

- Docs claim schema v2; code is v3 (`SCHEMA_VERSION=3`). v3 adds
  `claims.supersedes`; the migration is not described in architecture/README.
- Docs’ decision list omits the weak-relevance rule:
  `recallChannel !== 'capsule' && relevance < 0.5 → verify`. The CHANGELOG
  documents it; architecture does not.
- `search()` uses `scanLimit = min(max(limit*20,100),500)`; at defaults that is
  **280**, not 500.
- `sensitivity` is never written or read; `valid_until` has no writer, so the
  expired branch is inert in-plugin.
- FTS is trigger-free; `ensureFts()` rebuilds only when `COUNT(*) == 0`. A
  partially desynced index is not repaired.
- DB file mode is umask-derived; only the directory is `0700`.
- `tests/`, `backtest/`, `fixtures/` are not in the published tarball, so
  “64 tests”, “33 scenarios” and benchmark p95 claims are unverifiable from
  the artifact.

### 1.7 Open RE questions

1. Why is the weak-relevance `verify` gate 0.5 absent from the architecture
   doc, and was it calibrated or added after a regression?
2. Does FTS rebuild handle a non-empty but corrupt `claim_fts` table?
3. `findSupersedeTarget` picks only the highest-overlap candidate; what
   happens when it is a `condition_diff` but a lower-overlap candidate is a
   true paraphrase?
4. `recordInjection` references `retrieval_runs` without an explicit cascade;
   audit pruning deletes manually, but a crash between inserts could orphan
   rows. Verify with a crash fixture.
5. Does `learned_terms_json` ever grow the query vocabulary into noise? The
   cap is 40 terms/claim, but no decay.
6. Is the 0.4 negative-rate health threshold stable across small samples?
7. Exact DB file permissions after `mkdir` and SQLite WAL sidecars.
8. How does the plugin behave when `DSH_HOME` is relative or read-only?

## 2. dsh-workspace-memories 1.1.2

### 2.1 Injection

`agent/created` registers a prompt section `workspace-memory` with order
`9800`, text callback `buildMemoryText(dir)` (`index.js`).

- Up to `PROMPT_MAX_FILES=20` files.
- Each file truncated to `PROMPT_MAX_FILE_CHARS=32768`.
- Files are listed by name, content under `### \`<name>\``.
- Cache signature is `name:size:mtimeMs` joined by `|`; a content change that
  preserves all three is invisible.
- File size cap for writes/panel: `MAX_CONTENT_BYTES=1 MiB`.
- Storage:
  - `home`: `<DSH home>/workspace-memory/<basename>-<sha256[0:8]>/`;
  - `workspace`: `<workspace>/.dsh/memory/`, with containment check.
- Panel routes: `GET /api/workspace-memory/list|read`,
  `POST .../write|delete`; resolve `workspaceId` through
  `ctx.workspaceRegistry`, never a raw host path.
- `save_workspace_memory` tool writes a normalized `.md` name; tool
  registration is available next turn, not necessarily in the current
  prompt’s tool list.

### 2.1b Cache, auth and lifecycle details

- Header is a fixed 538-character paragraph; each file contributes
  `### \`name\``, a blank line, content.
- Cache key is sorted `name:size:mtimeMs`; a same-size edit within the same
  millisecond serves stale text. `readdirSync`/`statSync` run per model step
  per agent; the cache never evicts.
- Panel auth is two-layer: DSH's owner-Peer/Host/Origin fence, then a
  registered `workspaceId` lookup. There is no per-user ACL, no CSRF token in
  the plugin, and the list response includes the absolute storage directory.
- `agent/created` registers the section through an async `.then`; it can miss
  the first prompt assembly (unverified but plausible).
- Unregistered cwd falls back to the cwd itself for session memory, while the
  panel can only address registered workspaces.

### 2.2 Worst-case prompt cost

The section has no token/chars budget. Worst case:

```text
20 files * 32,768 chars = 655,360 chars
~164k tokens at 4 chars/token
~187k tokens at 3.5 chars/token
```

Even one maximum-size file is ~8k tokens. This is not safe for an 8k/12k
context budget. The cache avoids re-reading unchanged files, not re-emitting
them. A production deployment needs a section budget (e.g. 2–4k chars) or a
retrieval-on-demand tool instead of dumping 20 files.

### 2.3 Open RE questions

1. Does `systemPrompt.section` text get re-evaluated every request, and is the
   cache keyed only by directory signature? Confirm with a same-size,
   same-mtime write.
2. Panel route authorization: any same-origin browser client that knows a
   `workspaceId` can read/write that workspace’s memory; is that acceptable?
3. `resolveRoot`/`memoryDirFor` containment on symlinks and case-insensitive
   filesystems.
4. What happens when the workspace key collides (same basename + first 8 hash
   chars) — extremely unlikely, but no collision handling.
5. How does the section interact with compaction/resume and duplicate
   `agent/created` listeners?

## 3. @tr1v3r/dsh-ltm 0.3.3

### 3.1 Schema

`dist/scope-BuxNy74y.js`:

- SQLite DB at configured `path` (required; no default to cwd).
- Schema v2:
  - `meta(key,value)`;
  - `memories(id INTEGER PK AUTOINCREMENT, text, tags, scope, pinned,
    created_at, updated_at, last_confirmed_at, revision)`;
  - indexes `memories_recent(updated_at DESC,id DESC)`, `memories_scope(scope)`;
  - FTS5 `memories_fts(text,tags,scope, tokenize='unicode61')`.
- Writes use `BEGIN IMMEDIATE` and short synchronous transactions; the store
  can migrate v1→v2 with revision columns.

### 3.2 Retrieval

- FTS match tokens are quoted and OR-joined.
- Rerank = `0.6 * normalized BM25 + 0.4 * char n-gram cosine`;
  BM25 rank is min-max normalized because FTS5 rank is negative/lower-is-better.
- Char n-gram is 2..3 characters; cosine over frequency vectors.
- Dedupe:
  - Jaccard >= `0.8` (default) **or** char n-gram cosine >= `0.92`;
  - only within the same scope;
  - force can bypass, but stale baseVersion fails.
- Lifecycle: `last_confirmed_at`; stale after `90` days by default;
  `memory_confirm` refreshes.
- Prompt section: `promptMaxChars=2000`, `promptRecentCount=10`,
  pinned first; optional `promptMaxTokens` + `promptTokenizerPath` must be set
  together; token-bounded path counts complete candidates and never exceeds
  `promptMaxChars`.
- Scopes: session/workspace/global; workspace auto-scope by canonical Git
  common dir or cwd; absolute paths are not stored.

### 3.2b Migration, tokenization and output risk

- Schema v1 is refused on open; v1→v2 requires explicit `dsh-ltm
  upgrade-schema` with a `VACUUM INTO` backup and one `BEGIN IMMEDIATE`
  `ALTER TABLE`; no v0 path.
- FTS tokens: Latin runs lowercased; CJK ranges produce unigrams + bigrams;
  Hangul excluded; query tokens are deduped, double-quoted and OR-joined, so
  FTS5 operators in user input are literal.
- SQL fetches `max(200, searchLimitMax)` FTS candidates before rerank; beyond
  that is silent recall truncation.
- At defaults, a **single 2,000-char unpinned memory renders an empty section**
  (max renderable text ≈1,878 chars including the 76-char header and line
  overhead).
- Tool-output risk: `memory_list` without `limit` uses `LIMIT -1` (whole
  store), and `memory_search` can return up to 50 × 2,000-char rows in one
  result, bypassing the prompt budget.

### 3.3 Why it is the current-compatible target

`package.json` peers explicitly include `0.2.1-rc.1` and `0.2.1-rc.2` for
`dsh-system-prompt` and `dsh-tools`. This is the only inspected plugin that
declares the installed runtime.

### 3.4 Open RE questions

1. BM25 normalization when all FTS ranks are equal (returns 1) — does that
   over-reward a no-signal candidate?
2. Does the char n-gram cosine dominate Chinese queries because Latin BM25 is
   normalized across a small set?
3. Dedupe across revisions and pinned records: can a new duplicate supersede
   a pinned memory?
4. Exact migration v1→v2 and FTS rebuild behavior on a partially migrated DB.
5. Token counter path: what happens when the tokenizer file is missing or
   incompatible with the pinned tokenizer version?

## 4. @jacklika/dsh-memory 0.1.7-rc.40

### 4.1 Bundle order

`cordis.patch.yml` inserts, in waterfall order:

1. `memory-scope` — rewrite private write ids to `agents/<key>/<id>`;
2. `memory-queue` — serialize matching tool dispatches; optional
   cross-process `mkdir` lock, `laneArgument=id`, heartbeat;
3. `memory-git` — commit successful writes under `shared/` and `agents/`;
   `nestedRepo: init` by default;
4. `tool-memory-filesystem` — `memory_runtime_info`, `wiki_read`,
   `wiki_search`, `wiki_write`; atomic publication; optional `baseVersion`;
5. `memory-curator` — `memory_recall/capture/correct/forget/compact`;
6. `tool-memory-graph` — `wiki_graph`, audit, bounded traversal;
7. `tool-memory-vector` — disabled until an embeddings endpoint is configured.

### 4.2 Write/failure boundaries (from member READMEs)

- Scope rewrite is advisory: unlisted tools and out-of-waterfall writers can
  write any path; reads are unscoped by design.
- Queue lock is advisory too; unlisted tools/shell/Obsidian edits can race.
  `wiki_write` atomic rename prevents torn files; `baseVersion` turns lost
  updates into loud conflicts for cooperative callers.
- Git commits only successful matching writes; a failed commit fails the
  dispatch even though the note was written. The caller must re-read and pass
  the version before retrying, or an append can duplicate content.
- Archive (`memory_forget`) moves to `archive/`, excluded from search/graph by
  default; explicit reads still work.
- Per-write commits can be noisy; no fetch/pull/merge; external edits remain
  uncommitted until a sweeper/CI.

### 4.3 Open RE questions

1. Exact queue lock reclaim algorithm under a paused process and stale
   heartbeat; verify no successor lock is deleted.
2. `wiki_write` atomic publish + git commit: the note can be published but
   uncommitted; what does `wiki_history` show, and can `baseVersion` chain
   recover?
3. `memory_capture` conflict policy `ask/skip` under headless runs; how is an
   approval represented, and can it be replayed?
4. `memory_compact` stale pass needs graph edges; what happens if graph
   indexing is stale or disabled?
5. Vector search is disabled by default; what is the fusion contract when an
   endpoint fails mid-query?

### 4.4 Deeper pass: concrete failures

- **Missing member dependency:** `@jacklika/dsh-memory-time` is imported by
  `@jacklika/dsh-tool-memory-filesystem` and `@jacklika/dsh-memory-curator`
  (`lib/index.js` imports) but is absent from the bundle manifest. The 7
  declared members match the bundle's dependency versions; `memory-time` does
  not. A published install may fail to load those members unless pnpm resolves
  the transitive package; this must be verified before deployment.
- **Nested dispatch:** `memory-scope` rewrites an id by calling
  `ctx.tools.execute` again, not `next()`. The outer and inner executions both
  enter the `tools/execute` waterfall; whether post-execute/finalization runs
  twice is a host-contract question.
- **Queue cross-host reclaim:** a waiter declares a lock stale after 15 s of
  unchanged heartbeat *and* `processIsAlive(owner)` false. `processIsAlive`
  is false immediately for a different hostname, so a live cross-host holder
  on a shared vault can be preempted.
- **Git/archive failure windows:** a note can be published but uncommitted;
  the wrapper returns `COMMIT_FAILED`, and retry requires re-read + baseVersion
  or an append duplicates. A crash between archive write and original removal
  leaves both; retry then refuses because the archive exists. `wiki_restore`
  passes a model-supplied hash into `git show <hash>:<id>` without validation
  (option-injection risk; not executed).
- **Curator compact:** auto-capture order is `b.id.localeCompare(a.id)`, not
  session time. Stale compaction checks `graph.truncated` but not
  `edgesTruncated`; an edge-capped graph can hide inbound links and archive a
  referenced note. Each forget is independent, so a mid-run failure leaves
  partial archives.
- **Curator recall accounting:** it hides only auto-capture notes under
  `shared/notes/auto-capture/` or `auto-capture-<session>.md`; `total`
  subtracts only hidden hits that are inside the returned, capped page, so it
  can still count hidden notes.
- **Vector:** when enabled, embeds/sends first 8,000 chars of every note to
  the endpoint, including archive and other agents' namespaces; no
  `excludeDirs` is passed.
- **Archive glob:** code excludes archive at any directory depth, although a
  source comment says top-level only.
- **Documentation defects in the bundle:** README says curator registers
  `memory_forget`, but it does not; `memory_forget` is registered by the
  filesystem member. README also pins `0.1.7-rc.34` while the package is
  `0.1.7-rc.40`.
- **Expiry is advisory:** `expires` is parsed and exposed as `expired` on
  read/search hits, but it is not deleted or filtered from recall, graph or
  vector.
- **`baseVersion` is not CAS:** it is read-then-write, with no atomic compare;
  queue serialization makes it safe for cooperative writers, but two callers
  can read the same version before either writes.
- **Curator capture default tags:** `tags:[]` on overwrite replaces existing
  tags; provenance is stored on overwrite/new but not on append.
- **Graph keyword edges:** generated for all node pairs before `maxEdges` caps
  output; at `maxNodes:2000` this is millions of pairs (CPU/memory risk).
- **`memory-time` is absent:** both filesystem and curator import it, so the
  bundle is not self-contained in the published tarball.

## 5. Token/prompt budget comparison

Assumptions: 4 chars/token (conservative English/code mix), headers included
approximately; synthetic worst cases, not measured on DSH.

| Design | Fixed prompt budget | Worst case | Notes |
|---|---:|---:|---|
| dsh-memory-gate | 1,200 chars + header | ~355 tokens | 3 claims, 1,200 chars; 160-char verify cap; 20k session budget |
| @tr1v3r/dsh-ltm | 2,000 chars | ~500 tokens | pinned first, optional tokenizer budget |
| dsh-workspace-memories | none | **655k chars / ~164k tokens** | 20 files × 32,768; one file alone ~8k tokens |
| @jacklika/dsh-memory | 0 automatic | tool results on demand | no prompt section; model must call tools |
| agentmemory | 2,000 chars context | ~500 tokens | harness-agnostic reference |

The workspace plugin’s cache is a performance optimization, not a prompt
budget. Any DSH deployment with that plugin needs a hard section budget or a
move to retrieval-only tools.

## 6. Compatibility matrix (installed DSH 0.2.1-rc.2)

| Package | Declared | Current verdict |
|---|---|---|
| dsh-memory-gate 0.15.0 | through 0.2.1-alpha.2 | **not verified**; do not install without adapter |
| @tr1v3r/dsh-ltm 0.3.3 | 0.2.1-rc.1 / 0.2.1-rc.2 | **declared compatible** |
| dsh-workspace-memories 1.1.2 | no DSH range | simple API, but unverified |
| @jacklika/dsh-memory 0.1.7-rc.40 | peer `>=0.2.0-rc.1 <0.3.0-0` | range includes 0.2.x; still unverified runtime |
| @fooxe/dsh-memory 0.13.10 | peer tools `>=0.0.1-rc.1` | broad range; unverified |
| dsh-memory 0.1.0 | peers 0.1.0-rc.6 | **not compatible** |

## 7. Recommended next RE steps

1. **dsh-memory-gate**: port its pure `decideAuthority`,
   `extractDurableClaims`, `relations.decideSupersede`, and SQL migration into a
   standalone harness-agnostic fixture; test 100 synthetic claims/feedback
   cycles. This is the highest-value design but needs a current-DSH adapter.
2. **@tr1v3r/dsh-ltm**: run its own unit tests? No, inspect; build a synthetic
   DB and measure CJK/English rerank and prompt rendering; verify current DSH
   peer API against `dsh-tools` 0.2.1-rc.2.
3. **dsh-workspace-memories**: add a 2–4k char section budget or switch to a
   `workspace_memory_search` tool; measure actual token cost on a 20-file
   fixture.
4. **@jacklika/dsh-memory**: build a crash/concurrency matrix for
   queue→git→filesystem; verify no duplicate append after a failed commit.

## Evidence

- REA `dsh-memory-gate`:
  `ev_4220c587fc5ea3296add651cddf32a52aadf780afcc9d81a03fafa88edd1dc2f`
  (static, package digest `f382147b67f1bc1f7260be7815be50744c2d15d40862c8d09aba80f80eb4cb82`).
- REA `dsh-workspace-memories`:
  `ev_b5f90cf205ac8bf283122191d51215e0efe6cf638afad71a6ddd7f942f01b5fd`
  (static, package digest `511b7cc9fccb0c124a3a2580cf34f0b6cc36557c505a9ecbae1e6bb3251973e0`).
- Packages downloaded read-only to `/tmp/harness-memory-pkgs`,
  `/tmp/harness-memory-pkgs2`, `/tmp/jacklika-memory`; no plugin installed.
