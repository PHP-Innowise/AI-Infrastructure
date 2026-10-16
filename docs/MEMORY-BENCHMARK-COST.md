# Supplied memory benchmark cost ledger

`harness/src/harness/memory_benchmark_cost.py` records a fixed benchmark horizon
across **build**, **query**, **reader** and **judge**. It is optional experimental
accounting, with no default Harness routing, model calls or service authorization.
Existing native cost observation and RunCost/budgets remain unchanged. The local
runtime stays Python 3.14/FTS5; upstream benchmark dependencies remain separate.

This ledger consumes supplied observations. It does not discover provider work,
audit an invoice, judge answers or authorize execution. A completed SDK response
does not establish coverage of its hidden retries or a memory service's extraction,
embedding, consolidation, refresh, storage or other billable work. The future
runtime adapter must observe those costs or declare coverage incomplete.

## Collection contract

```python
from harness.memory_benchmark_cost import Ledger

ledger = Ledger(horizon=2)
ticket = ledger.begin_call('reader', 0, 'physical-call-id', 'physical-attempt-id')
ledger.finish_call(ticket, outcome='succeeded', usage={
    'input_tokens': 100, 'output_tokens': 20, 'total_tokens': 120,
    'cached_input_tokens': 60, 'reasoning_output_tokens': 5,
})
ledger.close_unit('reader', 0, coverage='complete', model_work='reported_calls',
                  billing_complete=False, local_ns=0, constructed_bytes=2400)
snapshot = ledger.snapshot()  # all other units are still unknown
```

Build has ordinal `None`; the other phases require each ordinal `0..H-1`.
Book every physical attempt before work. The call/attempt pair binds one immutable
scope across the entire ledger. Logical SDK calls are insufficient identities if
their retry attempts are unobserved. Same-scope booking replay returns the same
ticket; it does not grant permission to repeat physical work. Cross-scope reuse,
foreign/reconstructed/mutated tickets and new reservations after closure refuse.

An exact known booking may replay after its unit closes, before final freeze:
the same immutable scope and intact stored ticket return without adding an
attempt or changing cost totals. Pending or conflicted reports stay unknown;
booking replay cannot complete or repair them. Freeze and observation-budget
checks precede replay, and a new retry identity remains new membership.

The only usage profile is `inclusive-input-output-v1`: input already includes
cached input, and output includes reasoning. Raw Claude/native provider events
require a separate verified normalization; they cannot be passed as generic
compatible usage. Cache/reasoning are subsets, never added to totals. Valid paired
input/output can establish a derived report total when `total_tokens` is missing.
A supplied total must equal both components, or at least contain each known
component of a partial report. Contradictory/invalid usage is unusable.

Reports accept only fixed counter keys. Missing usage is unknown; explicit valid
zero usage differs from absence. Failed and cancelled reported attempts contribute
cost. Exact normalized receipts count once; conflicting receipts exclude the
whole attempt's usable observations. Unit closure freezes membership. Pending
reports remain unknown; a new/changed late report is refused and invalidates the
unit. Exact known replay before final freeze is allowed. Differing unit closure
reports remove its contradictory local timing/byte observations, independently
of conflicts in attempt reports. Final freeze rejects mutations; returned nested
snapshots are independent copies.

`none_observed_local` is an explicit caller attestation allowed only for build or
query with zero attempts. It cannot establish hidden remote work absence, and
reader/judge cannot use it. Empty `reported_calls` is refused. All contributing
units must close with complete coverage before a complete scope can be reported.

## Reading results

- **Product** includes build + query + reader. **Experiment** additionally
  includes judge. The fixed horizon charges build once, with per-planned-query
  amortization only when the named cost is complete.
- `known_tokens` is the subtotal of valid **paired** input/output reports. The
  independent `known_input_tokens` / `known_output_tokens` may include partial
  reports and need not sum to it. Zero in a known subtotal means an empty usable
  subtotal, not zero real cost. Complete totals remain `None` until covered.
- Token, cache, reasoning, billing, timing and byte completeness are separate.
  Missing cache detail need not invalidate known total tokens; complete tokens
  do not establish complete billing. `usd_micros` uses integer millionths of USD,
  with fixed `provider_reported` or `invoice_allocated` attribution. Complete
  billing also needs caller-attested unit coverage. Numeric unattributed amounts
  remain known observations with no complete USD total. This is not total cost
  of ownership: unobserved CPU/infrastructure/subscriptions are outside profile.
- `component_ns` sums supplied component durations, including local overhead;
  it is not wall-clock latency, p50 or p95. `constructed_bytes` sums peer-unit
  serialization proxies, not actual source reads, network bytes or provider
  tokens. Missing fields remain unknown rather than becoming free work.
- Evaluator grades are unique fixed ordinals and exact `bool`/`None`, with no
  answer/gold text stored. `None` is ungraded. Conflicting grades invalidate that
  ordinal. Per-correct ratios additionally need complete grading over the entire
  planned horizon and positive correct count. Missing/zero correct counts never
  produce a misleading zero cost. These are supplied grades, not verified QA.

The ledger bounds H to 512, physical attempts to 4,096, mutation observations to
65,536, each identifier to 256 characters, counters/derived sums to signed-64-bit
nonnegative values and serialized snapshots to 256 KiB. Lower configured limits
may make a workload unrepresentable: overflow stays explicit incomplete/refusal,
never truncation to a successful total. This does not bound the actual model or
service. IDs are privately keyed; no raw IDs, hashes, tickets, prompts, callback
dictionaries, error prose, model names, URLs or credentials enter exports.

Fresh public arithmetic inputs and complete independent oracles precede code and
calls. Run the new local suite with:

```bash
python3 -m unittest tests.test_memory_benchmark_cost
```

This suite validates accounting semantics. Actual upstream integration, physical
attempt coverage, hidden work, provider/model behavior, invoice completeness,
quality and net token savings remain pending. Do not substitute these arithmetic
fixtures for a separately sealed real benchmark with private evaluator gold.
