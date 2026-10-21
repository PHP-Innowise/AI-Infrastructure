# Document reranker pilot

`scripts/memory_rerank_pilot.py` is an optional, source-only experiment. It
does not change the runtime, the index, or CI: it compares the runtime's normal
candidate order with a deterministic document-feature reranker on the same
`context.db` and the same judged queries.

The reranker is deliberately small. A candidate keeps its BM25/authority
score, multiplied by at most `1.45` when the query's words also appear in its
path (`0.15` per hit, capped at three) and title (`0.10` per hit, capped at
two). Path is unindexed in FTS5, so this is the only signal that can reward a
file name such as `context_retrieval.py`; a model is not involved.

## Run

```bash
python3 scripts/memory_rerank_pilot.py \
  --database path/to/memory-bank/local/context.db \
  --queries queries.json \
  --judgments judgments.json \
  --limit 3 \
  --json
```

`queries.json` is `[{"id": "q1", "query": "..."}]` or `{"q1": "..."}`.
`judgments.json` uses the same shape as the evaluation stand:
`{"q1": {"docs/a.md": 2, "docs/b.md": 0}}`; grades above zero are useful.
`--runtime` selects another edition's `memory-bank/scripts` directory; the
default is PHP Core.

The pilot reads the database in read-only mode, prints no document bodies, and
writes nothing. It fails closed when the runtime, database, queries or grades
are unavailable.

## Read the result

For each query it reports the top-k useful/noise counts and paths for the
baseline and the reranked list. The aggregate is the sum of useful paths, not
a claim that the reranker is better:

- if `reranked_useful > baseline_useful` on a representative labeled set, the
  feature is a candidate for runtime integration;
- if it is equal or worse, keep the runtime unchanged.

The pilot shares the candidate window (100) and the runtime's admission rules;
it cannot rescue a path that never became a candidate. Use
`memory_eval --diagnose-misses` first: a high `selected_recall` with low
delivery means ranking, while a low `candidate_recall` means the query/index
side. This pilot is for the ranking branch only.

## Limits

Path and title overlap are weak features. They do not read body text, do not
model synonyms, and cannot resolve a tie between two equally named files. A
positive result here is evidence for a document feature, not for a learned
reranker; a cross-encoder or a larger feature set needs its own measured pilot.
