# Reverse engineering: memory applications for DeepSeek Harness

Research date: 2026-10-20. Scope: **DeepSeek Harness (DSH) 0.2.1-rc.2** and
memory plugins that extend harness behavior.

No plugin was installed, enabled, or executed. Downloads are npm tarballs into
`/tmp`; REA parsed the shipped JavaScript as inert text. Local DSH packages were
inventory-only.

Mechanical evidence (schemas, thresholds, feedback math, token budgets,
compatibility): [RE-HARNESS-MEMORY-MECHANICS-2026-10.md](RE-HARNESS-MEMORY-MECHANICS-2026-10.md).

## 1. What “memory for Harness” means

The installed DSH tree has session persistence, compaction, checkpoints,
instructions and session search, but no durable cross-session memory plugin:

```text
@deepseek-ai/dsh-session-*       session log / format / query / persistence
@deepseek-ai/dsh-compaction-*    context compaction
@deepseek-ai/dsh-agent-instructions  AGENTS.md / instruction loading
no @deepseek-ai/dsh-*memory* package
```

The plugin ecosystem fills that gap at four different extension points:

| Extension point | What it can do | Examples below |
|---|---|---|
| **Prompt section** | Inject durable text into every session/step | workspace memories, index, pinned claims |
| **Model tools** | Let the model write/search/forget/feedback | `memory_save`, `memory_search`, `/memory` |
| **Harness events** | Capture `user/message`, `turn/end`, `agent/pre-step` | automatic extraction, pre-step recall |
| **Bundle / profile patch** | Compose multiple plugins and storage backends | Obsidian vault + git + curator |

A memory plugin can therefore be as small as “one prompt section + three tools”
or as large as a write-coordinated vault with git history and graph traversal.

## 2. Ranked concrete applications

Ranking favors: a real DSH-native integration, a shipped artifact that can be
statically inspected, a distinctive memory mechanism, and a clear next RE
question.

| Rank | Application | Version | Storage / mechanism | DSH 0.2.1-rc.2 | Why RE it |
|---:|---|---|---|---|---|
| 1 | **dsh-memory-gate** | 0.15.0 | Local SQLite + FTS5; Claim → Belief → Decision → Consumption; `use/verify/ignore`; feedback updates belief | **Not verified**: declared through 0.2.1-alpha.2 | The only plugin found that makes **retrieved ≠ injected** explicit and learns from post-use feedback |
| 2 | **@tr1v3r/dsh-ltm** | 0.3.3 | Local SQLite; CJK-aware FTS + char n-gram rerank; scopes; dedupe; expiry; prompt budget | **Explicitly supports 0.2.1-rc.1/rc.2** | Best current-compatible deterministic memory; good schema/ranking target |
| 3 | **dsh-workspace-memories** | 1.1.2 | Per-workspace Markdown files; prompt-section injection; `save_workspace_memory`; sidebar panel | No declared compatibility; simple API | Cleanest “memory outside the repo” design and easiest full source read |
| 4 | **@jacklika/dsh-memory** | 0.1.7-rc.40 | Obsidian-compatible vault; per-agent write namespaces; serialized writes; git history; graph; curator/conflict tools | Peer range includes `0.2.x`; **published bundle lacks `@jacklika/dsh-memory-time` in its dependency list** | Most ambitious write path: queue, git, version checks, graph |
| 5 | **@fooxe/dsh-memory** | 0.13.10 | Markdown user/project scopes; session-start index; touch activation by read/edit path; panel | Peer tools range likely includes current | File-memory UX and path-triggered activation; bounded 200 lines / 25 KB index |
| 6 | **dsh-memory** | 0.1.0 | One SQLite file + FTS5; `memory_write/search/forget`; recall section | **Not compatible**: peers 0.1.0-rc.6 | Minimal baseline; useful to compare a 3-tool design |
| 7 | **bailian-memo-dsh** | 2.5.0 | Cloud Bailian Memory; auto recall; silent curation; remember/forget; consent/status | Peer 0.2.1-alpha.1; current not verified | Harness-native cloud memory with auto-recall and curation |
| 8 | **dsh-experience-memory** | 0.3.1 | Global “hot” memory + experience library; FTS5; WebUI cards | Need compatibility check | Two-tier hot/experience design |
| 9 | **@jipika/dsh-memory** | 0.6.0 | Sharded Markdown bodies + live-generated index (`shard · count · …`) | Need compatibility check | Index-only context; sharded bodies |
| 10 | **agentmemory** | 0.9.21 | Local memory server (MCP/hooks); BM25/vector/graph; 12 hooks; 53 MCP tools | Harness-agnostic, not DSH-native | Reference architecture for hook→observation→memory and retention |

Also found but lower priority: `dsh-memory-palace` 1.8.2, `@max-null/dsh-memory`
0.18.0, `@alanzhao/dsh-memory-lite` 0.4.0, `@klarkxy/dsh-memory` 0.1.7,
`dsh-memory-eternal` 0.7.0, `dsh-memory-eternal-ws` 0.8.1,
`@openviking/dsh-memory-plugin` 0.5.27.

## 3. dsh-memory-gate: the strongest design target

### Artifact and evidence

- npm `dsh-memory-gate@0.15.0`, MIT, Node >= 22.5.
- Package digest (REA): `f382147b67f1bc1f7260be7815be50744c2d15d40862c8d09aba80f80eb4cb82`.
- REA static analysis: 7 selected JS files, 8,846 AST nodes, 129 findings;
  graph partial; `ev_4220c587fc5ea3296add651cddf32a52aadf780afcc9d81a03fafa88edd1dc2f`.
- Declared compatibility: DSH `0.1.0-rc.6` through `0.2.1-alpha.2`;
  **0.2.1-rc.2 is not declared**. Use as design/reference, not as an installed
  dependency for the current Harness without an adapter test.

### Memory model

The design separates four stages that most “memory” plugins collapse:

```text
Claim → Belief → Decision → Consumption
```

- **Claim**: a scoped, sourced, revocable assertion. Scopes:
  `session:<id>`, `workspace:<sha256-prefix>` (path is not stored), `global`.
- **Evidence**: explicit claim starts at `alpha=6, beta=1`; heuristic at
  `alpha=4, beta=2`; feedback updates:
  - helped: `alpha += 1`;
  - harmful: `beta += 5`, `harmful_count += 1`;
  - stale: `beta += 2`;
  - conflict: `beta += 4`.
- **Decision**: every candidate gets `use`, `verify` or `ignore`.
- **Consumption**: human feedback (`/memory ok`, `feedback <#n> helped`) writes
  Evidence and changes future belief.

### Storage

Local SQLite + FTS5, no embeddings and no external memory API. Tables include:

- `claims` (id, scope, scope_key, kind, content, tags, state, origin,
  sensitivity, hash, source session/event, expiry, timestamps);
- `evidence` / beliefs;
- `retrieval_runs` (query hash, session, workspace, candidate count; not the raw
  prompt);
- `authority_decisions` (action, reason code, four scores);
- `injections` (actual claim ids and generated message id);
- `consumption` (human outcome and note).

The database is a **reconstructible projection and audit sidecar**; DSH session
logs remain the source of truth. `forget` is a tombstone, not hard delete.

### Retrieval and decision

Two local channels:

1. **Capsule**: up to 2 trusted explicit global preferences/constraints; does
   not require the current query to repeat their terms.
2. **Trigger**: FTS5 + normalized term rerank over current session, current
   workspace and explicit globals; the code applies a scan ceiling of
   `min(max(limit*20,100),500)` (280 at default limits).

Ranking:

```text
rank = lexical*0.62 + belief*0.23 + freshness*0.15 + scope boost + capsule boost
```

Decision order: non-active → ignore; expired → verify; harmful threshold →
ignore; relevance < 0.12 → ignore; freshness < 0.2 → verify; low belief/high
risk → verify; else use.

Modes:

- `shadow`: compute and audit, inject nothing;
- `assist` (default): inject `use`, mark `verify` as an unverified clue;
- `enforce`: inject only `use`.

Default injection bound: **3 claims / 1,200 characters**; capsule limit 2;
rolling `sessionBudgetChars=20,000` over 20 turns automatically tightens
`verify` injection. A negative-feedback rate health check can downgrade to
`shadow`.

### Feedback learning

`helped` feedback collects the query terms from recent retrieval runs
(up to 20 runs / 40 terms) and writes them as `learned_terms_json`, so the same
memory can be triggered by a different phrasing later. Only terms are stored,
not the original query text. `harmful`/`stale` lower belief and can quarantine.

### Harness integration

- Automatic extraction consumes only `source.kind=user` `user/message` after the
  matching `turn/end`; never assistant, tool or plugin messages.
- Recall runs at the first `agent/pre-step` only, with a real human message and
  not cancelled.
- Recalled content is a `createUserMessage` with
  `source: {kind:'dsh-memory-gate', form:'recall'}` and is explicitly labeled
  user memory, not a system instruction; angle brackets are escaped.
- Any retrieval/policy/audit/DB error returns the original message array
  (fail-open). If the DB cannot open, only a status command is registered.
- Workspace auto-mining is bounded: 20 sessions, 8 MiB per log, 16 MiB after
  zstd decompression; only logs whose first `session` event `cwd` matches the
  current workspace are mined.

### RE questions for dsh-memory-gate

1. Exact extraction rules for Chinese/English “remember” cues and the
   false-positive boundary.
2. Calibration of `alpha/beta`, risk baseline by claim kind, and the
   `use/verify/ignore` thresholds (the design gives defaults, not evidence for
   them).
3. Whether learned query terms cause topic drift; how to cap/forget them.
4. Correctness of the SQLite migrations (v1→v2) and crash recovery around
   WAL/audit tables.
5. How to port the same four-stage model to a DSH version whose
   `agent/pre-step`/message-source API differs (the adapter is one file,
   `harness.ts`).

## 4. dsh-workspace-memories: the clean per-workspace design

### Artifact and evidence

- npm `dsh-workspace-memories@1.1.2`, MIT, no build step.
- Package digest (REA): `511b7cc9fccb0c124a3a2580cf34f0b6cc36557c505a9ecbae1e6bb3251973e0`.
- REA static analysis: 2 JS files, 4,611 AST nodes, 17 findings;
  `ev_b5f90cf205ac8bf283122191d51215e0efe6cf638afad71a6ddd7f942f01b5fd`.
- Source is plain `index.js` + `client.js`; no source map needed.

### Behavior

- Every workspace gets one Markdown directory:
  - `home` (default): `~/.dsh/workspace-memory/<basename>-<sha256[0:8]>/`,
    outside the repository;
  - `workspace`: `<workspace>/.dsh/memory/` for team-versioned memory.
- Memory files are one `.md` per topic, max **1 MiB** each.
- Prompt injection:
  - up to **20 files**;
  - each file truncated to **32,768 characters**;
  - cached by directory signature (names, sizes, mtimes);
  - prompt section order `9800`.
- Agent tool `save_workspace_memory` writes a file; the panel lists/creates/
  edits/deletes files through exact `/api/workspace-memory/*` routes.
- File-name validation: max 100 chars, `.md`, no leading dot, no `..`, no path
  separators, no control characters.
- The panel addresses workspaces by `workspaceId`, never by an arbitrary host
  path; `storage: workspace` additionally verifies the resolved directory stays
  inside the workspace.
- Tool registration takes effect on the next turn; a new session is needed for
  the model's tool declaration to include it.

### RE questions

1. Exact prompt-section refresh behavior: the cache signature means a changed
   file with the same size and mtime could be missed; quantify the risk.
2. Panel route authorization: the plugin trusts `workspaceId`; what prevents a
   different workspace id from reading another workspace’s memory?
3. Injection budget: 20 × 32K characters is a large theoretical prompt; what
   does the real token cost look like and where should a budget cut?
4. Home-vs-workspace migration and collision behavior.
5. How the `agent/created` section interacts with resume, compaction and
   multiple agents in one workspace.

## 5. @tr1v3r/dsh-ltm: current-compatible structured memory

- npm `@tr1v3r/dsh-ltm@0.3.3`, local SQLite (`$DSH_HOME/memory/ltm.db`), no
  embedding service, no API key.
- Peer range explicitly covers DSH `0.2.1-rc.1` and `0.2.1-rc.2` — the best
  current-runtime fit among the plugins inspected.
- Tools: `memory_write`, `memory_search`, `memory_forget`, plus
  `memory_update`, `memory_confirm`, and management surfaces.
- Scopes: session/workspace/global; workspace detection uses the agent’s
  `session.header.cwd` and canonical Git common directory, so linked worktrees
  share a project scope; scope names store a short SHA-256, never absolute
  paths.
- Search: FTS5 + CJK-aware tokenization + hybrid BM25/char n-gram rerank.
- Dedupe: Jaccard ≥ `0.8` and optional cosine ≥ `0.92` mark a near-duplicate;
  writes can require `force`.
- Lifecycle: `staleAfterDays=90`; `memory_confirm` refreshes review time;
  `promptRecentCount=10`, `promptMaxChars=2000`, pinned first.
- Optional offline token budget via a pinned Hugging Face tokenizer
  (`promptMaxTokens`, `promptTokenizerPath`), both required together.

RE questions: schema/migration, CJK tokenizer quality, rerank weights,
near-duplicate semantics, scope isolation, prompt-budget truncation.

## 6. @jacklika/dsh-memory: vault, queue and git

- npm `@jacklika/dsh-memory@0.1.7-rc.40`, MIT, a **profile bundle** that mounts
  several member plugins in waterfall order.
- Vault default: `<session cwd>/.plugins/memory/`; Obsidian-compatible Markdown.
- Write ladder:
  1. `memory-scope`: private ids become `agents/<agent>/`; `shared/` passes;
  2. `memory-queue`: `mkdir` lock + heartbeat, serializes same-lane writes;
  3. `memory-git`: commits `shared/` and `agents/` writes, restores, archive
     moves inside the lock; `nestedRepo: init` keeps vault history out of the
     project repo;
  4. `tool-memory-filesystem`: `wiki_read/search/write`, atomic publication,
     optional `baseVersion` conflict detection, `wiki_history/restore`;
  5. `tool-memory-graph`: `wiki_graph`, audit, bounded page traversal;
  6. `memory-curator`: `memory_recall/capture/correct/forget/compact`.
- Frontmatter `expires` becomes an `expired` flag; expired notes are not
  auto-deleted.
- Optional semantic search is disabled until an embeddings endpoint is
  configured; `wiki_search` reports `fused` and `semantic`. When enabled it
  sends the first 8,000 chars of every note, including `archive/` and other
  agents' namespaces, to that endpoint.
- **A real load blocker:** the bundle manifest lists 7 members but both the
  filesystem and curator members import `@jacklika/dsh-memory-time`, which is
  not listed. The 7 declared member versions match; the time package must be
  supplied/resolved separately.
- Provenance fields on capture/correct: `source_session`, `commit`, `verified`.

RE questions: queue liveness/heartbeat, git history semantics, atomic publish
vs external Obsidian writes, graph snapshot validation, conflict/version
semantics, and whether `agents/<agent>/` is a real privacy boundary (README
says every agent can read the whole vault).

## 7. Other DSH plugins and harness-agnostic apps

| Application | Shape | Distinctive mechanism | RE question |
|---|---|---|---|
| **@fooxe/dsh-memory** 0.13.10 | file memory, user + project scopes, panel | session-start dated index; path-touch activation (`[frontend/src]` activates after a read/edit there); 25-entry injection cap; 200 lines/25 KB index; 500 memories/scope | How touch activation and nested scope discovery stay bounded; secret refusal on all write paths |
| **dsh-memory** 0.1.0 | one SQLite file + FTS5 | three tools; recall section pinned/recency; literal-token FTS query | Minimal reference; why quoted-token AND is chosen over OR |
| **bailian-memo-dsh** 2.5.0 | cloud personal memory | auto recall at first top-level step; silent curation at turn-stopping; remember/forget; consent/status; 5 s recall deadline | Curation commit/skip policy, fail-open behavior, identity/credential binding, cloud data deletion verification |
| **dsh-experience-memory** 0.3.1 | global hot + experience library | FTS5 retrieval + WebUI cards | Hot/experience promotion and decay |
| **@jipika/dsh-memory** 0.6.0 | sharded Markdown | shard bodies; live-generated index (`shard · count · …`) | Index generation and shard routing |
| **agentmemory** 0.9.21 | MCP/hook memory server | 12 hooks; sessions→observations→memories; BM25/vector/graph RRF; retention decay | Harness-agnostic reference; how to map DSH session events to its hook model |
| **OpenCode semantic memory** | OpenCode plugin (PyPI) | semantic memory plugin for OpenCode | Compare OpenCode plugin API vs DSH prompt/tool API |
| **OpenClaw memory-lancedb** | OpenClaw plugin | LanceDB-backed memory | Local vector store integration and plugin lifecycle |
| **Hermes holographic / post-cortex** | Hermes plugins | plugin-catalog memory backends | Harness plugin contract differences |
| **Memory Bank MCP / MCP memory server / Basic Memory** | MCP servers | Markdown memory bank / knowledge graph / file knowledge base | MCP tool schema and scope semantics across harnesses |
| **OpenHands persistent memory / TencentDB-Agent-Memory** | OpenHands SDK / adapter | persistent memory for a different agent harness | Adapter shape for session/turn hooks |

## 8. Concrete RE plan with agents

The highest-value work is not “read more READMEs”; it is to answer five
mechanism questions against shipped artifacts.

### Agent A — dsh-memory-gate (design reference)

- Extract `lib/repository.js`, `lib/authority.js`, `lib/service.js`,
  `lib/harness.js`, `lib/extractor.js` and `docs/architecture.md`.
- Reconstruct the SQLite schema and migrations from `repository.js`.
- Reimplement the CBDC decision as a pure function test against the documented
  thresholds; do not run the DSH host.
- Answer: which step actually changes belief; how learned terms are selected;
  how fail-open is guaranteed.

### Agent B — dsh-workspace-memories + @tr1v3r/dsh-ltm (current-compatible)

- Compare prompt-section injection: 20×32K cache vs 2,000-char budget.
- Measure rendered section length on synthetic fixtures; do not install into
  the live profile.
- Answer: which design gives the best token/recall tradeoff for DSH 0.2.1-rc.2.

### Agent C — @jacklika/dsh-memory (write path)

- Inspect `cordis.patch.yml` and member packages for the write ladder.
- Model queue lock/heartbeat, `baseVersion` conflict behavior and git commit
  boundaries.
- Answer: which failures can leave a partially published note; whether graph
  snapshots are consistent with the vault.

### Agent D — agentmemory (harness-agnostic comparison)

- Use the shipped source maps to compare hook→observation→memory with DSH
  session events.
- Answer: which DSH events map to agentmemory’s 12 hooks, and what is lost.

## 9. What this project should borrow

1. **Usage gate before injection.** `dsh-memory-gate` is the first inspected
   app that treats retrieved and injected as different states and records
   `use/verify/ignore` plus post-use feedback. This directly addresses the
   missing consumption loop in the current engine.
2. **Per-workspace memory outside the repo.** `dsh-workspace-memories` stores
   machine-local memory in `~/.dsh/workspace-memory/<workspace-key>/` and
   supports an opt-in in-repo mode. This is a cleaner default than putting
   every memory under the project tree.
3. **Bound the index, not just the body.** The ecosystem converges on
   ~200 lines / 25 KB indexes, 10 recent entries, 2,000-character prompt
   sections, 3–20 injected items and a hard per-session budget.
4. **Deterministic local storage first.** SQLite + FTS5 is the common baseline;
   embeddings and cloud memory are optional. `@tr1v3r/dsh-ltm` proves a
   current-compatible deterministic path without a network dependency.
5. **Pin plugin compatibility per DSH release.** The installed DSH is
   `0.2.1-rc.2`; several plugins stop at `0.2.1-alpha.2` or
   `0.1.0-rc.6`. A memory plugin must declare and test a compatibility window.

## 10. Decision for the current DSH

- **Deployable now:** `@tr1v3r/dsh-ltm@0.3.3` — the only inspected plugin that
  explicitly supports DSH `0.2.1-rc.2`; local SQLite, hard 2,000-char prompt
  cap, scopes, dedupe and expiry.
- **Design reference, not deployable:** `dsh-memory-gate@0.15.0` — the best
  CBDC/feedback model, but declared only through `0.2.1-alpha.2`; port the
  pure decision/feedback logic or add a current-DSH adapter before installing.
- **Team/vault option after a dependency fix:** `@jacklika/dsh-memory` — strong
  write ladder, but the published bundle is missing
  `@jacklika/dsh-memory-time` and reads are vault-wide.
- **Simple per-workspace option after a prompt budget:** 
  `dsh-workspace-memories` — excellent storage isolation, currently unbounded
  prompt injection (20 × 32,768 chars).
- **Minimal baseline:** `dsh-memory@0.1.0` is not compatible with the installed
  DSH (peer `0.1.0-rc.6`); use it only as a small reference implementation.

## Sources

- DSH: <https://github.com/deepseek-ai/deepseek-harness>
- `dsh-memory-gate`: <https://www.npmjs.com/package/dsh-memory-gate> ·
  <https://github.com/GIT121995/dsh-memory-gate>
- `dsh-workspace-memories`: <https://www.npmjs.com/package/dsh-workspace-memories> ·
  <https://github.com/lengfengng-tech/dsh-workspace-memories>
- `@tr1v3r/dsh-ltm`: <https://www.npmjs.com/package/@tr1v3r/dsh-ltm>
- `@jacklika/dsh-memory`: <https://www.npmjs.com/package/@jacklika/dsh-memory>
- `@fooxe/dsh-memory`: <https://www.npmjs.com/package/@fooxe/dsh-memory>
- `bailian-memo-dsh`: <https://www.npmjs.com/package/bailian-memo-dsh>
- `agentmemory`: <https://github.com/rohitg00/agentmemory>
- OpenCode: <https://opencode.ai>
- OpenClaw memory plugin:
  <https://docs.openclaw.ai/plugins/memory-lancedb>
- Hermes plugins: <https://hermes-agent.nousresearch.com/docs/plugins>
- MCP memory server:
  <https://www.npmjs.com/package/@modelcontextprotocol/server-memory>
- Memory Bank MCP:
  <https://www.npmjs.com/package/@allpepper/memory-bank-mcp>
- Basic Memory: <https://docs.basicmemory.com>
- OpenHands persistent memory: <https://docs.openhands.dev/sdk/guides/persistent-memory>
- TencentDB-Agent-Memory OpenHands adapter:
  <https://github.com/TencentCloud/TencentDB-Agent-Memory/tree/main/adapters/openhands>

## Artifact notes

- npm tarballs were fetched into `/tmp/harness-memory-pkgs` and
  `/tmp/harness-memory-pkgs2`; nothing was installed into a DSH profile.
- REA evidence: `ev_4220c587fc5ea3296add651cddf32a52aadf780afcc9d81a03fafa88edd1dc2f`
  (`dsh-memory-gate`) and
  `ev_b5f90cf205ac8bf283122191d51215e0efe6cf638afad71a6ddd7f942f01b5fd`
  (`dsh-workspace-memories`). Both are static; no runtime claim.
- Local DSH is `0.2.1-rc.2`; no DSH memory plugin is installed or enabled.
