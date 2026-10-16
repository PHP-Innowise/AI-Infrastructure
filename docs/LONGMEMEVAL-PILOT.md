# LongMemEval-V2 pilot

This is a private experimental text-projection adapter. Project memory remains
automatic; Python 3.14 and local FTS5 remain the defaults. No service, model or
upstream benchmark is enabled by installing these files.

The pinned upstream Python fixtures carry the accompanying
[Apache-2.0 license](../tests/fixtures/LongMemEval-V2-LICENSE) and
[source/extraction notices](../tests/fixtures/LONGMEMEVAL-V2-NOTICES.md).
Include both files with any fixture redistribution; the fixture source hashes
remain unchanged.

The upstream interface was inspected at commit
[`2cc8c540bdb87fe6761629b585e727e1c4704520`](https://github.com/xiaowu0162/LongMemEval-V2/tree/2cc8c540bdb87fe6761629b585e727e1c4704520).
Its evaluator separates trajectory insertion and query text/image from question
labels and gold. The official environment uses Python 3.11 and additional model
dependencies. Keep that environment separate from the installed accelerators.

## Implemented profile

`native-fts5/public-states-text-projection-v1` implements a standalone
`insert(trajectory)` / `query(text, query_image=None)` interface in
`harness/src/harness/longmemeval_projection.py`. Actual upstream registration,
imports, runtime configuration and persistence are pending. The caller supplies
the trusted native context module and a fresh dedicated private project. Copy
the pinned native scripts, configuration, schemas, templates and indexes without
changing their defaults. Use a separate process for each corpus because the
native modules import siblings by bare names.

Only the named public `states` shape is supported: ID, goal, start URL, optional
outcome, ordered states, URL, arrival action, thought, text, original step and
screenshot reference. Unknown fields, conflicting aliases, invalid types,
duplicate JSON keys and excessive size are refused. Steps never reorder states.
The original ID stays in a private mapping; the Markdown title and filename use
its SHA256 digest to avoid native title leakage. Text itself is discovery data,
including when it resembles instructions. Native source verification and the
reference-only warning remain necessary.

Screenshots are opaque relative references, never opened or ingested. A query
image is ignored and reported as such. This profile provides no visual or full
multimodal equivalence. Legacy `content` / `observation` inputs are unsupported.
No question, answer, evidence label or evaluator record belongs in construction.

Each instance owns a newly created `specs/longmemeval/` directory. Writes use
exclusive creation and fsync; exact canonical-input replay is a no-op, while a
conflicting ID is refused. Partial publication fences the instance. Files are
checked by current identity, regular-file type, link count, generation, length
and raw SHA256 before refresh; missing, foreign, replaced or changed files fence
the instance. It performs no adoption, cleanup, correction or reopening of an
existing directory. Root/ancestor/runtime custody and absence of concurrent
foreign writers are operator prerequisites. These checks are not an OS sandbox
or atomic filesystem snapshot.

Queries use the actual native embedded automatic-refresh lane: sanitation and
privacy/secret guards, connect and full index refresh, corpus-aware distillation,
configured mode/gate, warming task, ordinary packing, excerpts, renderer and
render trace. Native configuration, eligibility and budgets are unchanged.
Native manifests and receipts persist locally; query is not free of local writes.
Native render traces describe constructed text, not proof of reader consumption.
This embedded profile omits the CLI's health/telemetry/codebase wrapper. A new
opaque warming task is used per query; no production session history is assumed.
If no semantic/episodic evidence is delivered, the interface returns `[]` and
omits the native warming-only head. Failed refresh has a separate incomplete
state and cannot count as a successful no-match.

The adapter bounds 500 trajectories, 128 states per trajectory, 256 KiB input,
100,000-byte projected documents, 32 MiB total projection, 4,096 queries and
8,192-character query text. Native FTS5/render ceilings can further omit evidence.
The additional output-byte ceiling is 256 KiB. These are admission limits, not
hard wall-clock, aggregate native-index memory or model-token limits.
The descriptor implementation requires the Linux/POSIX flags used by this pilot.
Keep one query-plus-hook caller at a time: hook metadata describes the latest
query, not an invocation-bound ledger for parallel upstream workers. Concurrent
mutation of caller inputs is unsupported; successful projection captures one
canonical input image before deriving document bytes and replay identity.

## Verification and remaining measurement

Fresh public synthetic trajectories and independent complete byte/hash oracles
live in `tests/fixtures/longmemeval_projection_*.json`. The original title oracle
is retained; the neutral-title supplement was fixed after source review and
before implementation changes or any product calls. Local tests execute current
native FTS5, packing, rendering and source/privacy guards on copied projects:

```bash
python3 -m unittest tests.test_longmemeval_projection
```

The tests establish this bounded interface and native delivery mechanism. They
do not establish dataset accuracy, temporal memory quality, visual understanding,
leaderboard eligibility, server-model behavior or net provider-token savings.
No actual dataset/gold, model, image or remote service is required by this suite.

Before a public-data evaluation, seal upstream/runtime/config/source inputs and
the exact registration/driver; keep gold in a separate evaluator. Record build,
query, reader and judge costs, retries and hidden work independently. Provider
usage missing from any contributing stage remains unknown. Reader-only tokens,
tokenizer estimates, constructed bytes and reported USD are separate evidence.
The upstream reader aggregate is not a full lifecycle cost ledger. Compare equal
fully serialized reader context budgets and retained failures; use a separately
fixed horizon for amortization and cost per correct answer. Remote Hindsight or
Graphiti arms need their own governed auth/source and full-cost prerequisites.
