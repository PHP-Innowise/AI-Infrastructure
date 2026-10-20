# Memory tools: decisions from the 13 October 2026 source study

The objective is to reduce the total cost of a correct task through automatic
memory. This study inspected current upstream documentation and source. No new
package, model, remote account or service was installed or called. Python 3.14,
local FTS5 and the existing automatic memory flow remain the defaults; ONNX
remains optional and disabled.

## Candidates and decisions

| Candidate | Concrete role | Decision and acceptance condition |
| --- | --- | --- |
| SQLite `fts5vocab` | Supplies term/document frequencies within the existing SQLite database, with no new Python dependency. | **Proceed with a separate local cost pilot.** Compare exact frequency answers and the whole refresh/query cost against the existing generation cache. Composite phrases need the existing MATCH fallback. [Official module](https://www.sqlite.org/fts5.html#the_fts5vocab_virtual_table_module) |
| Model2Vec + sqlite-vec | Static embeddings plus a derived vector index add a second bounded candidate pool for paraphrases and code concepts. | **Proceed with an optional isolated pilot.** Pin model/tokenizer revisions; measure new eligible recall and delivered answer passages. Verify Python/SQLite extension support on the actual runtime. [Model2Vec](https://github.com/MinishLab/model2vec), [sqlite-vec Python](https://alexgarcia.xyz/sqlite-vec/python.html) |
| Hindsight | Remote retain/recall with consolidation can provide team memory across clones and worktrees. | **Optional internal native source-hint bridge validated locally.** The [isolated pilot](HINDSIGHT-PILOT.md) covers registered project identity, supplied local source checks, whole-context budgets and exact fallback. Live ingestion/service readiness and total cost remain unmeasured. [Recall API](https://hindsight.vectorize.io/developer/api/recall), [billing](https://docs.hindsight.vectorize.io/billing/) |
| Graphiti | Temporal relationships can represent corrected decisions and invalidate earlier relationships. | **Keep as a challenger for correction-heavy workloads.** Pin an explicit search recipe and its model calls. Graph namespace selection must follow authorization. [Upstream](https://github.com/getzep/graphiti), [namespaces](https://help.getzep.com/v3/graphiti/core-concepts/graph-namespacing), [search](https://help.getzep.com/v3/graphiti/working-with-data/searching) |
| LLMLingua2 | Learned token selection can reduce explanatory prose. | **Defer to a protected-prose arm.** Preserve policy, canonical facts, code/config literals and verification metadata verbatim; measure omissions and compressor cost. [Upstream controls](https://github.com/microsoft/LLMLingua/blob/main/DOCUMENT.md) |
| GPTCache semantic answer reuse | Returns a stored response for a similar request. | **Stop for source-dependent repository decisions.** Similarity cannot establish current task, source, policy or permission equivalence. Exact derived-data caching needs explicit invalidation identities. [Upstream](https://github.com/zilliztech/GPTCache) |

## Integration boundaries

Project Brain and canonical project files continue to define current task state,
permissions and evidence. Every candidate channel passes the same privacy,
owner, authority, lifecycle, conflicts and freshness checks before packing.
Dense similarity needs its own admission evidence; it cannot silently inherit a
lexical coverage claim.

Derived local indexes should bind source-content hashes, model/tokenizer
revisions, passage format and policy. HEAD alone misses dirty-file changes.
Remote banks should use a registered project/team identity rather than checkout
basenames, with explicit authorized scopes and source revisions. Unknown or
unavailable evidence stays unavailable.

The Hindsight `max_tokens` limit covers fact text. Metadata and separately
requested source chunks require another complete-response budget in our
adapter. Its coding-agent integration installs local hooks; a controlled
Harness HTTP adapter is the clearer boundary for the clone-backed product.
The `harness.hindsight_recall` pilot implements bounded HTTP and a supplied-source
boundary. An operator-configured reference provider now feeds automatic
TaskContext through native source guards and packing; the default remains None.
Native hooks do not call the service. Live ingestion, service readiness and the
other proposed backend interfaces remain pending.

Model2Vec runs static embeddings and can use a base inference package without
Torch/ONNX training extras. It still adds dependencies and model assets.
sqlite-vec requires loadable SQLite extensions, which some system Python builds
disable. Package metadata does not establish runtime compatibility. Both
components stay outside the stdlib edition runtime until a separate pilot passes.

Exact excerpt selection is the first compression candidate: retain complete
relevant paragraphs, functions or configuration blocks and deduplicate overlap.
Text within retained spans remains exact; the omitted parts still require a
source fallback. FTS `snippet()` tokens and byte counts are distinct from model
tokens. [FTS5 snippet contract](https://www.sqlite.org/fts5.html#the_snippet_function)

## Proposed fresh measurements

Freeze new public source/task cases and complete expected facts before building
the candidate pools. Include precise identifiers, paraphrases, negation,
corrections, source deletion, policy changes, same-name projects and unavailable
services. Keep tuning cases separate and retain failures and abstentions.

First measure candidate recall, answer-bearing text actually delivered, stale
or unauthorized content, noise, cold/warm latency, memory use and index size.
Evaluate the whole caller, including discovery, indexing, sorting, transport,
fallbacks and cleanup. The existing native FTS whole-cycle protocol remains an
unfinished prerequisite; this study does not reopen its closed supplied pilot.

A subsequent real-session comparison must charge seed and resumed turns,
ordinary input, cache reads/writes, outputs, tool responses, embedding,
consolidation, compression, recovery and retries. Fix the provider/settings and
public source/fact protocol before execution. Compare total cost per correct
answer only when both arms have complete valid usage and a verified channel
contrast. Byte reduction and upstream benchmark accuracy do not prove that
memory saves provider tokens in this repository.

## Remaining correctness work

The current reliability increment addresses coherent Brain snapshots, failed
refresh delivery, explicit attestation, MCP Unicode IDs, Harness task identity,
checkpoint provenance and recovery compaction. Separate confirmed follow-ups:

- Harness now freezes safe submitted payloads during their original observed
  save and requires receipt-only replay without regenerating omissions. Recovery
  of interrupted first writes without receipts and any post-final/restart replay
  lifecycle remain separate contracts; legacy runtimes stay unbound.
- Task-file membership now invalidates on the Git-selected index/shared index,
  worktree prefix and supported ref generation. Unknown states use a fresh
  membership probe without caching; detected instability suppresses file edges
  for that refresh. This is refresh-boundary correctness, with added metadata
  calls and bounded index reads; net runtime/token benefit is unmeasured.
- New installed Cursor rules now receive local custody receipts. Harness
  capsule delivery attempts retirement before Cursor startup; native hooks
  share guarded maintenance and full-body freshness checks. Custom, legacy,
  edited, tracked and ambiguous files remain intact. Actual native loading
  order and net provider-token benefit remain unmeasured.

Full promotion crash recovery and a real-session net token benefit comparison
remain separate unfinished work. Old failed native/network/resource probes
retain their recorded results and are not replayed by this increment.

## Follow-up sources, 19 October 2026

A second source pass looked for tools that could change a decision above. None
is installed; the runtime stays standard-library only.

| Source | What changed | Decision |
| --- | --- | --- |
| [Deterministic memory conflict resolution](https://arxiv.org/abs/2606.01435) | The assembly step, not storage, was the bottleneck: candidate extraction plus a Python `max(serial/timestamp)` beat LLM-judged freshness (+10.8 pp in a matched pipeline; 78.0–94.8% on FactConsolidation single-hop). | **Adopt at the consolidation boundary.** `scripts/memory_fact_consolidation.py` takes the explicit maximum and reports ties. Not wired into the capsule: two records that declare each other in `conflicts` are shown together by contract, not collapsed. |
| [OWASP memory attack surface](https://genai.owasp.org/2026/05/13/memory-is-a-feature-it-is-also-an-attack-surface/) (ASI06) | Memory is a persistence layer for an attacker; a durable chunk outlives the session that injected it. | **Adopt a provenance gate.** Automatic promotion refuses external sources and instruction-like content; `bank-audit --provenance` exposes auto-promoted/external/instruction-like chunks and `bank-quarantine` stops serving one today. The reviewed path is unchanged. |
| [Anthropic Memory Stores](https://platform.claude.com/docs/en/api/beta/memory_stores) | Beta memory as a mounted directory with append-only versions, `created_by` attribution, redaction and retention. | **Design reference, not a dependency.** Versioning, attribution and redaction confirm the write-path gaps; a hosted store adds a service and a network boundary this repository does not take. |
| [Basic Memory](https://docs.basicmemory.com/start-here/what-is-basic-memory) | Local Markdown notes plus SQLite plus MCP, with observations/relations and memory URLs. | **Study, no dependency.** Closest architectural peer; its knowledge format is useful input for consolidation, but the repository already has a governed record/chunk model and must not duplicate it. |
| [OpenMemory MCP](https://mem0.ai/blog/introducing-openmemory-mcp) | A local-first memory MCP server shared by clients. | **Consumption challenger, not core.** The repository's MCP surface already exists; the missing evidence is real capsule-bearing sessions, not another server. |
| [sqlite-lembed](https://github.com/asg017/sqlite-lembed) | GGUF embeddings inside SQLite through llama.cpp. | **Optional pilot asset**, alongside Model2Vec: `scripts/memory_sqlite_vec_pilot.py` probes it and `sqlite-vec`, fails closed, and fuses path lists with RRF. Candidate-only; alpha extensions are not defaults. |
| [temporal-rag / Glacier](https://github.com/Emmimal/temporal-rag) | Validity windows, exponential decay and `supersedes` chains. | **Pattern already present.** The record/chunk model has `valid_from`, `valid_to`, `review_after` and `supersedes`; the useful increment is the consolidation boundary above, not another store. |
| [LongMemEval](https://proceedings.iclr.cc/paper_files/paper/2025/hash/d813d324dbf0598bbdc9c8e79740ed01-Abstract-Conference.html) and MemoryAgentBench | Long-term conversational memory and fact-consolidation benchmarks. | **Later measurement.** FactConsolidation has a local deterministic fixture now; a conversational benchmark does not replace the 12-prompt repository stand. |
