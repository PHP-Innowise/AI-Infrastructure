# SkillCache pilots

Three runs of [the SkillCache evaluation](SKILLCACHE-EVAL.md) against a real
provider, all on 21 November 2026 with Claude Sonnet 5 as compiler and
executor. Each was sealed before it started and judged by isolated verifiers.
They are the first measurements of what a warm turn costs and whether its work
holds. They enable nothing: every SkillCache mode stays opt-in and `off` by
default.

## What was asked

For procedural skills, does a warm turn (a compiled capsule run by the
executor) cost less than a cold turn while its work is still accepted?

Every run used the arms `cold` and `skillcache_executor` and the probes `exact`
(the observed request again) and `instance` (the same skill on other input). A
task is one skill with a small Git project; every attempt ran in a fresh copy
of it, in the Harness's own sandbox, and one verifier per task judged what the
attempt left, knowing which probe it judged. The correctness margin was 0.10
in all three.

| Run | Build | Tasks | What the tasks are |
| --- | --- | ---: | --- |
| A | `d32aa682` | 8 | Reports from routes, config files, language files, migrations, a log and source comments; a release cut; a contact clean-up |
| B | `6a52083f` | 10 | Small computations over JSON cases with a JSON result |
| C | `f45fd46e` | 8 | Reports from a lock file, an OpenAPI document, PHP sources, a SQL seed, a JUnit report and an access log; a flag diff; an invoice ledger |

Run A ran before Gate 3 compared a replay with the observed result. Runs B and
C ran with that comparison, on tasks none of which is in run A; B and C share
no task either. The build is the commit whose tree the pilot driver was built
from. In run C the engine that sealed, ran and reported was the build of
`6a52083f`; the two commits differ only in how a graph's folder is removed.

## What came out

| | A | B | C |
| --- | ---: | ---: | ---: |
| Capsules qualified, all in the first round | 8 of 8 | 10 of 10 | 8 of 8 |
| Cold attempts accepted | 16 of 16 | 20 of 20 | 15 of 16 |
| Warm attempts accepted | 14 of 16 | 20 of 20 | 16 of 16 |
| Cold turn, list price | $0.183 | $0.142 | $0.191 |
| Warm turn, list price | $0.124 | $0.136 | $0.141 |
| Cold turn, wall time | 28.6 s | 17.4 s | 35.0 s |
| Warm turn, wall time | 11.2 s | 15.5 s | 16.4 s |
| Model calls, cold and warm | 5.6 and 2.6 | 5.45 and 4.4 | 5.75 and 3.6 |
| Output tokens, cold and warm | 2,048 and 389 | 843 and 727 | 2,471 and 833 |
| Saving per accepted reuse | $0.059 | $0.006 | $0.051 |
| Its 95% interval | $0.046 to $0.073 | −$0.015 to $0.018 | $0.043 to $0.058 |
| Compilation, per capsule | $0.304 | $0.279 | $0.305 |
| Observed turn, per capsule | $0.190 | $0.146 | $0.188 |
| Break-even reuses, observation charged | 9 | 75 | 10 |
| Quality against cold | `not-shown` | `non-inferior` | `non-inferior` |
| Sealed verdict | `not-accepted` | `no-saving-shown` | `saves` |

Every warm attempt of the three runs applied its capsule, and no program step
was handed back to the agent.

## What it says

- **A warm turn is not cheaper by itself.** It saves what the cold turn would
  have spent on the task: model calls and output. Where a cold turn takes five
  or six calls and writes two thousand tokens (A and C), a warm turn costs a
  quarter to a third less and takes less than half the time. Where the cold
  turn is already short (B, 843 output tokens), the saving is within the noise
  and a capsule does not pay for its compilation.
- **Most of a turn's price is not the task.** Every attempt started in a fresh
  folder, so the provider's prompt cache was written anew: about 25,000 tokens
  at the one-hour rate, $0.10 of a $0.14 warm turn in run C. SkillCache does
  not touch that part. A project whose sessions find a warm cache was not
  measured.
- **A capsule could be wrong and qualified.** In run A the `route-table`
  capsule dropped the middleware of half the routes in both probes. Its
  programs ended cleanly, its own check passed, and Gate 3 looked only at
  that. The observed cold turn had been right. Gate 3 now compares the files a
  replay leaves with what the observed turn left (`replay_result_mismatch`).
- **The comparison was not seen refusing anything live.** In runs B and C all
  eighteen replays matched their observation in the first round. Replaying run
  A's eight kept capsules through the new gate afterwards, without a model,
  gave six matches, the refusal of `route-table`, and one refusal of a correct
  capsule: `config-audit` wrote the same JSON values in another layout. The
  comparison is by bytes, which is stricter than the task.
- **A compiled program can be right where a cold turn is not.** The one cold
  failure of run C was a seed summary that miscounted rows; the capsule
  counted them correctly on the same request.

## What the runs found in the evaluation itself

- `launches.usage.components` took a Claude launch's output tokens from the
  usage of its stream messages, which is the count at the start of a message:
  a session that wrote 4,720 output tokens was recorded with 81. The output now
  comes from the launch's closing `result` event. With that, the driver's
  usage priced at the sealed table equals the CLI's own `total_cost_usd` to
  four decimals.
- One verifier judges every probe of its task and sees only the workspace.
  Without the probe it would have accepted, for another request, the result of
  the observed one. A verifier's command may now hold `{probe}`.
- The hooks of whoever runs a pilot fire inside its sessions: on the machine
  of these runs they added two model calls to every session and a step to the
  observed trace. The fixtures switch them off with `"disableAllHooks": true`
  in their `.claude/settings.json`, in both arms.

## What it does not say

- All twenty-six skills are procedural and were written for these runs. They
  favour compilation by construction and are not a sample of what a team asks.
- Every cell ran once. Eight or ten tasks cannot show non-inferiority within
  0.10 on a wider population, whatever the label of a single report.
- Only Claude Sonnet 5 was measured. Codex and Cursor as executors, another
  compiler, a skill with steps that need judgement, and a long-lived project
  with a warm provider cache were not.

## The sealed prices

USD per million tokens for Claude Sonnet 5, as the CLI itself prices a run at
list: input 2.00, output 10.00, cache read 0.20, cache write 2.50 for five
minutes and 4.00 for one hour. No five-minute write occurred in any run.

## Running one

A pilot is a folder of its own with four things: the tasks' fixtures and
skills beside a `driver.json` for
[the pilot driver](SKILLCACHE-EVAL.md#what-exists-and-what-does-not), the
verifiers with the pristine inputs they judge against, the plan, and a
dedicated folder for the sealed protocol.

```bash
infra-engine eval skillcache-ab seal --plan plan.json --out sealed/protocol.json
infra-engine eval skillcache-ab run --protocol sealed/protocol.json --out result.json
infra-engine eval skillcache-ab report --protocol sealed/protocol.json --result result.json
```

What the three runs taught about running one:

- Write the tasks and check the verifiers against a reference result and
  against wrong ones (nothing written, the other probe's result, a damaged
  result, a changed input) before sealing. A task that was run before the seal
  is a rehearsal task, not a pilot task.
- Take the engine and the driver as copies of the build of a committed tree:
  the seal holds the driver's digest, and a later build changes it. The driver
  is not among the workspace's default members, so it is built with
  `cargo build -p infra-harness --bin infra-harness-skillcache-driver`; without
  `-p` the build fails, and a pipeline that hides that failure copies whatever
  binary an earlier test run left.
- Do not build or test on the machine while a run goes on: wall time is one of
  its measures.
- The driver keeps a copy of the engine in the state and in the workspace of
  every session, several gigabytes for fifty sessions. A capsule's package is
  read-only, so a run folder is removed after `chmod -R u+w`.
- A provider's session limit ends a run without a result. A sealed plan is not
  resumed: the run starts again with a fresh state and a new seal over the
  same tasks, prices and analysis.

The protocols, results, reports and the kept workspaces of these runs are
local evidence (`docs/qa/evidence*/`), which the repository ignores.
