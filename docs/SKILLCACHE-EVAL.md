# SkillCache Evaluation

[SkillCache](../harness/README.md#skillcache-a-repeated-skill-as-a-compiled-plan)
replaces a repeated skill run with a compiled plan. Whether that pays is a
measurement, and this is the measurement: what a capsule costs to compile, what
each reuse saves, and whether the work is still accepted.

It decides nothing by itself. Every SkillCache mode stays opt-in, and the two
heuristics that exist behind settings (`skillcache.retrieval_by_mu` and
`skillcache.recompile_thresholds`) stay off until a run of this evaluation,
sealed before it started and made on tasks the heuristic was not tuned on, says
otherwise. That is the rule of [the memory stand](MEMORY-EVAL.md#accepting-a-retrieval-change),
applied here unchanged.

## What exists and what does not

The engine seals a protocol, runs it through a driver, and reports:

```bash
infra-engine eval skillcache-ab seal --plan plan.json --out protocol.json
infra-engine eval skillcache-ab run --protocol protocol.json --out result.json
infra-engine eval skillcache-ab report --protocol protocol.json --result result.json
```

The **driver is yours**. It is the command that makes one preparation or one
attempt with a real provider, and no driver for a real pilot ships with the
engine. Two arms can be driven with the Harness as it is: `cold` (a session
with SkillCache off) and `skillcache_executor` (a session in `warm_only`,
after an observed turn was compiled). The other four arms are ablations that
need modes the Harness does not have: a turn given the raw trace, a turn given
a written procedure, a capsule without the executor, and a capsule that was
not replayed. A protocol may name them; a driver has to provide them.

The regression that runs in CI uses a scripted driver and calls no provider
(`runtime-rust/crates/infra-cli/tests/skillcache_eval.rs`).

## The protocol

`seal` takes a plan and writes the sealed protocol once: the same document
with `sealed_at`, the digest of the verifiers' folder, and `seal_sha256` over
all of it. A run and a report recompute that digest and refuse a protocol that
no longer matches it, so nothing below can be chosen after results exist.
Unknown fields are refused.

```json
{
  "schema": "skillcache-eval-protocol-v1",
  "question": "Does a warm turn cost less than a cold one for the same accepted work?",
  "design": {
    "primary": "correctness",
    "quality_margin": 0.05,
    "minimum_tasks": 30,
    "power_rationale": "...",
    "selection": "...",
    "exclusion": "...",
    "secondary_benefit": "List price per accepted reuse.",
    "stopping_rule": "fixed-horizon",
    "unknown_policy": "insufficient"
  },
  "arms": ["cold", "skillcache_executor"],
  "probes": ["exact", "instance", "rename", "reword"],
  "matrix": [{"compile_model": "...", "executor_model": "..."}],
  "tasks": [{
    "id": "invoice-rows",
    "skill": "pdf",
    "observe": "Extract the rows of invoices/2026-q3/",
    "probes": {
      "instance": "Extract the rows of invoices/2026-q4/",
      "rename": "Extract the rows of bills/2026-q3/",
      "reword": "I need every line item from the Q3 invoice folder"
    },
    "verifier": ["sh", "{verifiers}/rows.sh"]
  }],
  "prices": {
    "<model>": {"input": 0.0, "output": 0.0, "cache_read": 0.0, "cache_write": 0.0}
  },
  "reuses": [1, 5, 20],
  "bootstrap": {"rounds": 10000, "seed": 20261120},
  "driver": {"argv": ["/path/to/driver"], "timeout_seconds": 3600},
  "verifiers": {"root": "/path/to/verifiers", "timeout_seconds": 300}
}
```

- **`design`** is the correctness design of the memory outcome evaluation,
  validated by the same code: correctness is primary, the horizon is fixed, and
  an unknown makes the answer insufficient.
- **`arms`** always include `cold`; every other arm is measured against it.
  `raw_trace` and `written_procedure` give the agent the observed trace or a
  prose procedure instead of a capsule; `skillcache_agent` gives it the capsule
  without the executor; `skillcache_executor` is the full warm turn;
  `skillcache_no_replay` is a capsule that passed Gates 1 and 2 only.
- **`probes`** say how far a request is from the one the capsule was compiled
  from. `exact` is the task's `observe` request itself. Every other probe has
  its own request in every task: `instance` (other values), `rename` (the same
  work on renamed things), `reword` (the same request in other words).
- **`matrix`** lists the model pairs. `cold` and `raw_trace` involve no
  compiler: they run once for an executor model and are shared by every cell
  with that model.
- **`prices`** are USD per million tokens for exactly the models the matrix
  names. They are sealed with the rest: a report cannot be repriced.
- **`verifiers`** is all or nothing. With it, every task names its verifier
  command and an isolated verifier judges every attempt. Without it, no task
  does, and the driver's own `accepted` is recorded, which the report says
  (`judged_by`).

## The driver

A driver gets one JSON object on standard input, answers with one JSON object
on standard output and exits with zero. It runs in the engine's process
custody with the sealed time limit and the engine's environment.

A **preparation** happens once per task and arm before that task's attempts:
observe the task's `observe` request and compile it, or whatever the arm needs.

```json
{"phase": "prepare", "seal_sha256": "...", "task": "invoice-rows", "skill": "pdf",
 "observe": "...", "arm": "skillcache_executor", "compile_model": "...", "executor_model": "..."}
```

```json
{"prepared": true, "usage": {"input_tokens": 0, "output_tokens": 0,
 "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}, "seconds": 0.0}
```

`prepared` says whether the arm got what it was preparing (a qualified
capsule). The attempts run either way: a capsule that did not qualify makes
its warm turns cold, and that is what the arm then costs.

An **attempt** is one request in one arm. The arms of one probe run in an
order drawn from the sealed seed, so that no arm is always the first on a warm
provider cache; `position` is the place in that order.

```json
{"phase": "attempt", "seal_sha256": "...", "task": "invoice-rows", "skill": "pdf",
 "observe": "...", "probe": "rename", "request": "Extract the rows of bills/2026-q3/",
 "arm": "skillcache_executor", "compile_model": "...", "executor_model": "...", "position": 1}
```

```json
{"workspace": "/abs/path/of/the/result", "accepted": true,
 "usage": {"input_tokens": 0, "output_tokens": 0,
           "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0},
 "model_calls": 0, "seconds": 0.0, "program_steps": 0, "failed_steps": 0}
```

Report only what the provider reported. A component the provider does not
give (Codex gives no cache writes and no count of model calls, Cursor gives no
tokens) is left out or `null`; it is never a zero. `model_calls` are distinct
provider messages, not stream events. `program_steps` and `failed_steps` are
the plan's program steps and how many of them were handed back to the agent.

A driver that fails, runs out of time or answers with something else has not
answered. The record is kept with its status and no measurements, and the run
goes on. A run that is interrupted writes nothing, and a sealed plan is not
resumed or run twice.

## Verifiers

With `verifiers`, the engine judges each attempt itself. The task's command
runs in the attempt's `workspace` inside the read isolation of the outcome
evaluation (Linux, bubblewrap): it sees that workspace, the sealed verifier
folder read-only at `{verifiers}`, and the system's public runtime folders. It
has no network, and it does not see the sealed protocol, the result, or
anything else of the machine. Before the first attempt the engine proves on an
empty folder that the sealed protocol is not visible from inside, and runs
nothing when that proof fails.

A verifier answers with the receipt of the outcome evaluation and exits with
zero:

```json
{"schema": "task-outcome-verifier-v1", "completed": true, "correct": true}
```

Anything else, including a missing workspace, is not a judgment: `accepted`
stays `null`. The verifiers' folder is digested when the protocol is sealed
and again before a run; a folder that changed judges nothing.

## The report

A result must hold exactly the planned preparations and attempts of its own
sealed protocol: nothing lost, added or run twice. For each cell of the
matrix and each arm the report gives:

- `planned`, `answered`, `judged`, `accepted`, and `acceptance_rate`, which is
  `null` while any planned attempt is unjudged;
- mean `tokens` per component with `coverage` (how many attempts reported
  each component, how many were fully priced, how many reported model calls,
  seconds and program steps);
- `list_price_usd`, the mean over fully priced attempts at the sealed table,
  and `list_price_usd_reported_components`, a lower bound over whatever was
  reported;
- `model_calls`, `seconds`, and `deopt_rate` (handed-back program steps over
  program steps, `null` when none were reported).

For every arm but `cold`, `preparation` gives how many preparations
succeeded and `compile_usd`, their mean price at the price of the model that
did the work. `against_cold` then answers the question:

| Field | Meaning |
| --- | --- |
| `acceptance_difference`, `..._ci95` | Per task, the arm's acceptance rate over its probes minus cold's; the interval is a bootstrap over tasks. `null` unless every planned attempt of both arms was judged and there are at least `minimum_tasks` tasks. |
| `quality` | `insufficient`, `worse` (the whole interval is below `-quality_margin`), `non-inferior` (the whole interval is at or above it), or `not-shown`. |
| `pairs`, `pairs_accepted`, `pairs_priced`, `tasks_priced` | Same task, probe and executor model in both arms; then both accepted; then both fully priced. |
| `saving_per_reuse_usd`, `saving_ci95_usd` | Cold price minus the arm's price over priced pairs, averaged within a task and then over tasks. A cheaper attempt that was not accepted saved nothing and is not in it. |
| `breakeven_reuses` | `ceil(compile_usd / saving)`. `null` when there is no saving or the compile price is unknown. |
| `net_saving_usd` | `K * saving - compile_usd` for each sealed `K`. |
| `verdict` | See below. |

The verdict applies the sealed design, correctness first:

| Verdict | When |
| --- | --- |
| `insufficient` | Quality is `insufficient`: an unknown, or too few tasks. |
| `not-accepted` | Quality is `worse` or `not-shown`. Cost is not consulted. |
| `quality-holds-cost-unknown` | Quality is `non-inferior`, but fewer than `minimum_tasks` tasks have a priced pair. |
| `no-saving-shown` | Quality is `non-inferior` and the saving interval reaches zero or below. |
| `saves` | Quality is `non-inferior` and the whole saving interval is above zero. |

The paired unit is the task. Probes of one task are not independent of each
other, so they are averaged within the task before anything is resampled.

## Accepting a change on this evidence

- A report is evidence for the sealed question. It turns nothing on.
- The task set that a capsule rule, a selection rule or a threshold was tuned
  on cannot accept it. Acceptance needs tasks held out from that tuning,
  sealed before the run, with the arms, margin, prices and horizon fixed.
- `insufficient`, `not-accepted` and `no-saving-shown` are reported as they
  are. A low price with unknown acceptance is not a saving.
- A result judged by the driver (`judged_by: "driver"`) is a rehearsal of the
  machinery. A claim about accepted work needs verifiers.
- The first pilot is an observation: it says what a default would have to
  show, and is not the reason for one.

## What the Harness itself shows

A capsule's page shows what compiling it cost, when the provider reported a
price, and how its warm turns went: how many, how many completed, how many
handed a program step back. It shows no saving and no break-even. Those come
from a sealed run of this evaluation, and until one exists they are shown as
unknown, not as zero.
