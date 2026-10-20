# Optional local vector pool: sqlite-vec and sqlite-lembed

Status: **off by default, maintainer-only pilot.** The runtime stays on
standard-library Python and SQLite FTS5; it never imports this pilot, loads a
SQLite extension, reads a model or opens a network socket. The pilot exists so
a dense candidate pool can be measured without turning a native extension into
a dependency.

## Why

The lexical index is the admission path: coverage, anchors, source hashes and
authority decide what a capsule may carry. A dense pool can only add
**candidates** for the same admission rules to judge; it cannot admit directly.
That boundary is deliberate - the repository already recorded a negative result
for latent-semantic embeddings on a small corpus (0.76 for an absurd query
against 0.48 for a relevant one), and a second pool must earn its place.

The tools themselves are small and local:

- [sqlite-vec](https://alexgarcia.xyz/sqlite-vec/) - a loadable SQLite extension
  with a `vec0` virtual table and KNN; still alpha (`v0.1.10-alpha.4` at the
  time of writing).
- [sqlite-lembed](https://github.com/asg017/sqlite-lembed) - local text
  embeddings from GGUF models through llama.cpp, also a loadable extension.

Neither is installed or downloaded by this repository.

## Probe

Assets are provided by a maintainer, from local paths:

```bash
python3 scripts/memory_sqlite_vec_pilot.py probe \
  --vec-extension ~/models/sqlite-vec/vec0.so \
  --lembed-extension ~/models/sqlite-lembed/lembed0.so \
  --model ~/models/minilm.gguf --json
```

The result is `{"available": true, ...}` only when every file exists and both
extensions load; otherwise it is `{"available": false, "reason": "..."}` and
the command exits nonzero. Failure is data: the runtime keeps lexical order and
never silently falls back to a different scorer. The probe does not register a
model or create a table; those are the pilot's next step and must be recorded
with the model revision and hash.

## Fuse without a dependency

`fuse` combines two ranked path lists with Reciprocal Rank Fusion (RRF,
`k=60`), which needs no score scale shared between the lexical and dense pools:

```bash
python3 scripts/memory_sqlite_vec_pilot.py fuse \
  --lexical lexical.json --dense dense.json --json
```

Inputs are JSON lists of paths, best first. Duplicate paths in one list count
once. The output is a deterministic `[{path, score, ranks}]`; ties break on the
path. This is the only code that should decide a candidate order in the pilot.

## Measurement rules

Freeze the corpus, query set, labels, model/tokenizer revisions and extension
bytes before a run. Keep the dense pool candidate-only:

1. baseline: the lexical capsule as it is;
2. dense-only candidate list at a fixed depth;
3. lexical + dense fused with RRF at the same capsule budget;
4. the same admission checks (privacy, authority, lifecycle, source hashes,
   coverage/anchors) applied after either pool.

Report `answer_passage_recall`, useful/noise documents, useful/noise turns,
p50/p95 including extension load and embedding time, and the index size. A
dense candidate that only adds noise at the same recall is a negative result,
not a tuning invitation. Do not adopt an alpha extension as a default
dependency, and do not let a similarity score satisfy a lexical coverage or
authority claim.

## Tests

`tests/test_memory_sqlite_vec_pilot.py` runs in the repository's test suite and
checks fail-closed probing plus deterministic RRF. It never downloads a model,
loads a third-party extension or touches a project memory tree.
