# Public response contracts for memory task evaluation

Maintainer-only opt-in helper for new studies. Python 3.9+ and standard library only.
The module works entirely on in-memory values. Model execution, file access,
storage and default activation are outside this module. The maintainer-only
task-complete stand can explicitly adopt v2 through `memory_response_intake.py`;
it is not enabled for existing task sets or installed memory runtimes.
Existing studies and grades remain unchanged; v2 is an explicit future-only
choice, with no change to installed memory, retrieval or defaults.

## Public contract

`memory-response-contract-v1` has exactly five keys:

| Key | Value |
| --- | --- |
| `schema` | `memory-response-contract-v1` |
| `case_id` | Nonblank valid UTF-8 string, at most 128 code points, without NUL |
| `marker` | Fixed ASCII `MEMORY_TASK_ANSWER` |
| `answer_fields` | 1–32 flat fields, each with exactly `type` and boolean `nullable` |
| `limits` | Every named limit below, as an integer |

Supported answer types are `boolean`, `integer`, `string`. Field names are
nonblank valid UTF-8 strings of at most 64 code points without NUL. Expected
factual answers, semantic labels and arbitrary validation code are not contract
keys. Boolean values are distinct from integers.

| Public limit | Configuration admission range |
| --- | --- |
| `max_messages` | 1–64 |
| `max_message_bytes` | 1–262144 |
| `max_total_bytes` | 1–1048576 |
| `max_answer_string_chars` | 0–65536 |
| `integer_abs_max` | 1–9007199254740991 |
| `min_citations`, `max_citations` | 0–16 each |
| `max_citation_span_lines` | 1–10000 |
| `max_citation_quote_chars` | 1–65536 |
| `max_citation_path_chars` | 1–1024 |
| `min_explanation_chars`, `max_explanation_chars` | 0–65536 each |

Minimums cannot exceed maximums; total message bytes cannot be below the
per-message byte limit. These ranges bound admitted configurations. Actual
response checks use their explicitly supplied values. `../tests/test_memory_response_contract.py`
contains a complete small public contract and sample response.

`render_contract` renders the exact validated public object plus the fixed
format rules used by `check_response`. A future controller's sealed effective
prompt can incorporate this text. Prompt submission and capture custody are
caller responsibilities, not attestations made by this module.

An answer attempt has the marker on its first line, followed by one plain JSON
object with exactly `case_id`, `answer`, `citations`, `explanation`. Fences and
surrounding prose are outside this format. LF and CRLF marker endings work;
their byte hashes remain different.

The answer has exactly the declared fields and types/nullability. Integer
magnitude uses `integer_abs_max`; strings use `max_answer_string_chars`.
Each citation has exactly `path`, `start_line`, `end_line`, `quote`. Path and
quote are nonempty strings within the declared limits. Positive line numbers
are bounded by `integer_abs_max`; end is at least start, and the inclusive span
is bounded. Explanation length is bounded separately. Character lengths are
Unicode code points; message/transcript limits are exact UTF-8 bytes.

JSON parsing rejects duplicate keys, nonfinite values, exponent overflow and
strings that cannot encode as UTF-8. Shape checking does not establish source
authenticity, contiguous quotes, explanation substance, factual truth or valid
unknowns. Arbitrary custom graders may still impose undisclosed constraints;
their complete disclosure is not mechanically guaranteed here. Representation
equivalences such as URL versus ticket ID are not inferred or normalized.

Version 1 remains usable for shape-only checks. It does not disclose a private
grader's eligible source corpus, even when that grader accepts only an initial
snapshot. Readable runtime-created catalogs can therefore satisfy v1 shape
while falling outside a caller's private corpus. Do not describe v1 as solving
that disclosure problem, add corpus fields to old v1 objects, or regrade frozen
studies with the new version.

## Version 2 citation corpus

`memory-response-contract-v2` retains the five v1 fields and adds exactly
`citation_corpus`. This object has exactly `schema` (the literal
`memory-citation-corpus-v1`) and `files`, a complete list of entries with exactly:

| Entry key | Meaning |
| --- | --- |
| `path` | Exact canonical relative POSIX path |
| `sha256` | 64 lowercase hex digits over original bytes |
| `bytes` | Exact byte size, an integer distinct from boolean |

Paths must be nonempty valid UTF-8, within `max_citation_path_chars` (at most
1024 code points), with no empty, `.` or `..` component, backslash, colon, C0
control or DEL. Absolute paths, drive names, URLs and UNC paths are excluded by
those rules. Files are strictly sorted by path using Unicode code-point order,
without duplicates. There is no percent decoding, case folding, Unicode
normalization, tilde expansion or filesystem resolution; names remain literal.
Existing caller privacy and safe-reader checks still apply.

The corpus allows at most 256 files, 1 MiB per file and 16 MiB in total. Its
canonical JSON is bounded to 65536 UTF-8 bytes, and the complete rendered v2
contract to 131072 bytes. An empty corpus requires both citation limits to be
zero. Admission overflow rejects the contract; it never clips or omits paths,
silently widens limits, or replaces the list with an unbound external reference.

The v2 renderer publishes the entire descriptor and fixed citation rules. Only
listed paths at the declared bytes are eligible, even when an installed,
generated, indexed or retrieved unlisted file is readable. `check_response`
checks shape and this public path membership; it does not open or verify source
bytes. Source eligibility establishes literal presence, not factual truth,
current authority, an adopted decision or human approval.

`check_citations(contract, text, sources)` is the optional pure v2 evidence
checker. `sources` must be an exact plain dictionary from **every** declared
path to original immutable `bytes`. The dictionary is copied at entry before
contract validation; later caller replacements cannot alter that check's byte
image. All paths, sizes and raw digests must match, and all bytes must decode as
strict UTF-8, without BOM removal or newline conversion. Missing or extra paths,
changed bytes, wrong types or invalid UTF-8 raise `ValueError` as invalid
evaluation context. The caller must block or report unknown, rather than count
those context failures as reader errors or fall back to host files. Calling this
function with v1 also raises `ValueError`.

With valid context, shape, membership, nonexistent ranges and inaccurate quotes
return `valid=False` with diagnostic `errors`. Lines are LF-delimited, one-based
and inclusive. A final LF creates no phantom extra line; an empty file has zero
lines. CR is preserved, including CR in CRLF; lone CR and Unicode line separators
do not start new lines. A quote must be an exact contiguous substring of its
requested original line window, excluding only that window's final LF. A
multiline CRLF quote must preserve CRLF. There is no trimming, whitespace
folding, ellipsis expansion or newline normalization. A quote elsewhere cannot
rescue an incorrect range. These checks establish no semantic truth.

## API

| Function | Purpose |
| --- | --- |
| `validate_contract(contract)` | Validated independent copy, or `ValueError` |
| `contract_digest(contract)` | Canonical public-contract SHA-256 |
| `render_contract(contract)` | Deterministic public text |
| `check_response(contract, text)` | Shape checks, plus v2 path membership; `valid` and diagnostic `errors` |
| `check_citations(contract, text, sources)` | V2 literal evidence over supplied bytes; context mismatch raises `ValueError` |
| `preflight_reference(contract, text)` | `None` for valid shape/membership; otherwise `ValueError`; no source-byte or truth verdict |
| `select_answer(contract, messages)` | Last marked attempt and exact text/hash/index |
| `make_artifact(contract, messages, context)` | Unsigned selection and identity bindings |
| `review_decision(contract, artifact, machine_correct, receipt_json=None)` | Separate machine, review and task decisions |

`messages` is a chronological list of assistant strings. Selection uses the
last exact first-line marker even when its body is malformed, lacks `case_id`
or names the wrong case. Earlier valid answers cannot rescue it. Later unmarked
save acknowledgments are ignored; unmarked intent and marker misspellings are
not inferred. Exceeded stream bounds produce `unavailable` without an earlier
answer fallback.

Selected text, zero-based index and exact UTF-8 SHA-256 are retained. The stream
digest hashes canonical JSON of all supplied strings, preserving their content
and order. Completeness of the supplied capture and host authenticity remain
unproven.

Context has exactly `protocol_sha256`, `slot_id`, `verifier_sha256`,
`source_image_sha256`. Digests are lowercase 64-digit hex. Slot IDs use the same
128-code-point identity bound as case IDs. The artifact adds public case and
contract identity, stream hash, selected index and selected message hash. Source
and protocol fingerprints are supplied values, not checked through file reads.
The v2 public-contract digest includes the complete corpus descriptor. No new
artifact or review schema is needed; changing a path, digest or size invalidates
an old binding. Artifact creation checks shape/membership, not source bytes.

## Independent review

The receipt is strict JSON text up to 16384 UTF-8 bytes with exactly:

```python
{
    "schema": "independent-answer-review-v1",
    "binding": artifact["binding"],
    "reviewer_id": "independent-reviewer",
    "reviewer_kind": "agent",
    "verdict": "consistent"
}
```

Reviewer identity is nonblank UTF-8, at most 128 code points without NUL.
Claimed kind is `agent` or `human`; verdict is `consistent`, `contradiction`,
or `uncertain`. Every binding must match, including protocol, slot, case,
verifier, public contract, source, stream, selected index and selected text.
The artifact's selected text/hash/shape metadata is also rechecked.

| Machine result | Final gate |
| --- | --- |
| `False` | Always false, including missing, mismatched or uncertain review |
| `None` | Always unknown, including a positive review |
| `True`, invalid public shape | False |
| `True`, invalid artifact or missing/invalid/mismatched/uncertain review | Unknown |
| `True`, valid bound contradiction | False |
| `True`, valid shape and bound consistent review | True |

Review diagnostics remain separate from machine failures. Integer/boolean
equivalence is not used for binding indexes. Identity, reviewer independence,
kind and semantic verdict are supplied assertions. Unsigned bindings are not
authentication, human proof or semantic truth. The result always records
`authenticated=False`, `human_confirmation=False`, `semantic_truth_proven=False`,
including a claimed human reviewer.

Immutable machine-grade storage, append-only review receipts, frozen study
membership and absence of feedback/retries are future integration obligations.
This pure prototype implements none of that persistence or model governance.

## Validation

Run from the repository root:

```bash
python3 -B -m unittest tests.test_memory_response_contract
python3.9 -B -m unittest tests.test_memory_response_contract
```

The tests cover declared boundaries, boolean/integer distinctions, nullability,
Unicode bytes versus code points, CRLF identity, strict JSON, malformed last
attempts, binding fields, stale slot reuse, artifact metadata types, input
preservation and fail/unknown gates. Synthetic in-memory checks do not establish
provider behavior, production integration or task-outcome benefit.
Pre-edit v1 golden values cover rendered bytes, canonical contract/artifact
digests, selection and review behavior. V2 controls cover complete disclosure,
readable-but-unlisted files, empty corpora, source drift versus reader errors,
literal ranges/quotes, immutable inputs, fixed bounds and stale review bindings.

## Caller integration boundary

Import `scripts/memory_response_contract.py` from a private controller, or use the explicit completion-stand integration below. Seal its implementation bytes and the actual rendered effective prompt before model dispatch. Build the artifact from captured chronological assistant messages and trusted current-run context, then retain that artifact outside the agent view with an immutable association to the current slot. This in-memory helper does not establish that a supplied artifact belongs to the current process or that a supplied source digest is genuine. Never adopt an artifact and matching receipt from an unrelated run as the current slot.

Verify citation paths and quotes with safe readers against the sealed allowed source image in the independent oracle; the shape checker never opens citation paths. Define equivalent identifier representations or declare their exact required form in the public task. Custom-grader semantic requirements remain author-supplied. This helper does not change previous studies, default retrieval or memory admission.

For a new v2 caller, prepare the full runtime and index before selecting the
eligible citation corpus. A generated catalog may be included at that point, or
remain explicitly ineligible. Snapshot approved ordinary files with existing
safe readers, then build the descriptor from those exact bytes. Run both
`preflight_reference` and `check_citations` on every reference, and require the
latter's `valid=True`. Seal the descriptor, byte map, verifier implementation,
source-image association and full effective prompt before reader execution.
Do not infer source preflight from `preflight_reference` alone.

The caller must prove complete prompt transport without truncation and preserve
immutable grading bytes. Where listed files are visible to a reader, use exact
read-only bytes or prove that they stay unchanged. Mutable final working files
are not a substitute for the sealed map. At grading, pass that same complete map
to the shared checker; drift is an input blocker, with all failed-attempt costs
retained. Do not extend the corpus after observing answers or repair, retry or
replace a response. Semantic review remains separate, bound to the actual
hash-pinned target result, complete chronological messages and expected context.

An experiment requiring one identical public contract across arms also requires
one identical eligible citation corpus. It may choose a stable public citation
subset while memory delivery differs. If eligible corpora differ, disclose
different v2 contracts and acknowledge the prompt/context difference; never hide
arm-specific eligibility behind a common contract. The helper does not establish
that all other custom grader semantics have been disclosed. No previous study
is regraded.

## Explicit task-complete integration

`memory_outcome_ab.py seal` and `run --endpoint task-complete` opt in **per task**
when the public task contains a complete `response_contract` v2 object. Its
`case_id` must equal the task `id`. Existing task sets without that field keep
their legacy prompt, last-message capture and answer/patch verification.
The private case additionally declares:

```json
{
  "mode": "answer",
  "equals": "42",
  "response": {
    "answer_field": "result",
    "reference": "MEMORY_TASK_ANSWER\n{\"case_id\":\"public-task\",\"answer\":{\"result\":\"42\"},\"citations\":[],\"explanation\":\"Reference rationale\"}"
  }
}
```

This example requires a public contract with exactly one nonnullable string
field `result` and citation limits that permit an empty citation list. A real
reference must satisfy that task's actual shape **and** literal-source rules.
For `task_mode: patch`, retain the existing external patch oracle and set
`response.answer_field` to `null`. The response gate does not grade the factual
meaning of an explanation or other patch-answer fields: task semantics remain
the separate existing oracle's responsibility. No semantic-review assertion is
manufactured by passing literal checks.

Before sealing, the author supplies the complete approved path/raw-SHA/size
descriptor. Seal runs after base/index/save preparation. It refuses inaccessible,
linked, multiply linked, special, sensitive, invalid-UTF-8 or mismatching listed
files and invalid reference citations. It copies only those approved files into
a separate common citation image. Consolidation variants must satisfy the same
descriptor. Corpus entries cannot be extended after answers are observed.

Each slot receives a fresh read-only mount at `.memory-citation-corpus/` and
instructions to cite original relative paths against that immutable pre-task
snapshot. Both arms receive these bytes: this is extra common public context,
**not an absence-of-memory control**. Do not put treatment-only historical
knowledge in this common image when designing a memory-ablation experiment.
Existing `off` disables hooks; it does not remove saved files. A true absence
condition requires a separately designed and sealed physical memory ablation.

The complete rendered prompt (at most 2 MiB) travels through native exec's
standard input, avoiding a single-argument limit. The protocol binds its exact
SHA and byte count; the result records submitted transport, not proof of model
perception. No clipping or argument fallback is used. The full contract follows
the task instructions, so its corpus descriptor is always transmitted in full.

The stand captures at most the contract's declared assistant-message and byte
limits. Later unmarked save acknowledgments do not replace a marked answer;
a malformed later marked attempt never falls back. Overflow fails the response
gate rather than accepting a retained successful prefix. The bounded capture,
selected artifact and current protocol/slot/verifier/source associations remain
in the private result, including incomplete native attempts and failures before
grading. The capture digest and selected artifact must match the current attempt;
retaining an earlier marked answer never makes an unfinished turn complete.

After native completion, an independent no-network namespace runs the shared
checker over the same complete immutable byte map. Its exact receipt is checked
against the current captured stream and context, then the existing equals or
external patch oracle grades task correctness. Source/mode/membership/prompt
drift blocks the attempt as unknown; actual shape/path/range/quote errors are
false. Costs already incurred remain in the fixed denominator. The v2 helper and
intake implementation join the opt-in runtime seal. Private references and
expected values never enter the public snapshot or prompt.

This integration is maintainer tooling only. It does not enable installed
memory, alter retrieval defaults, authenticate an agent review, or establish
memory efficacy. The original 24-session study retains its frozen v1 inputs,
code and outcomes.

```bash
python3 -B -m unittest tests.test_memory_response_intake
```
