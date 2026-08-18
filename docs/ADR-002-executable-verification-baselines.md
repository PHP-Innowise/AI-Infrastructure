# ADR-002: Executable verification baselines

> **Кратко.** Скилл объявляет проверку `eslint assets` и заявляет «exits zero». На
> неизменённом проекте она даёт 189 ошибок, то есть скилл выдаёт блокирующую находку
> при каждом прогоне. Гейт этого не видит и увидеть не может: чтобы узнать результат
> `eslint`, его надо запустить, а гейту исполнять команды запрещено. Предлагается
> записывать наблюдение при авторстве — тот, кто пишет план, всё равно обязан
> запустить команду, чтобы написать правдивое ожидание, — и сверять заявление с
> записью статически. Ожидание при этом становится разностным, а не абсолютным.
> Пример в схеме, который учит абсолютной форме, переписывается вместе с правилом.

**Status:** accepted, implemented as schema 1.3 (see section 7 for what changed
between the proposal and the implementation)
**Supersedes:** nothing
**Related:** [ADR-001](ADR-001-claim-adjudication.md) — the same shape of problem
(a claim the deterministic gate cannot settle) resolved a different way.

## 1. What is broken

A generated skill carries verification entries. One of them, from the fourth
end-to-end run against a real Symfony project:

```
skill:      coder-frontend
id:         lint-frontend-sources
command:    node_modules/.bin/eslint assets
expected:   "ESLint exits zero, so the touched sources satisfy the indent and
             vue/html-indent rules set to 4 ..."
failure:    "Any reported error blocks completion"
```

Executed against the unmodified target:

```
exit=1
205 problems (189 errors, 16 warnings)
```

The skill therefore raises a blocking finding on clean code every time it runs.
This is the same defect that got `content-publication-review` rejected in the
third run — there in a `grep`, here in an executable command.

**The defect is taught, not accidental.** `project-profile-schema.md` gives this
exemplar for a verification entry:

```json
"command": "vendor/bin/phpunit tests/Feature/ExampleTest.php",
"expected_result": "The focused test exits zero and reports no failed assertions"
```

Any author following the schema will write the absolute form. On a project whose
suite or linter does not currently pass — which is most projects — the absolute
form is false the moment it is written.

## 2. Why the gate cannot close this

`validate_skill_quality.py` grades search-shaped commands by resolving them with
its own file walk. That works because a literal search over the target is a pure
function of the target's bytes.

`eslint assets`, `bin/phpunit`, `bin/console lint:twig` are not. Their result
depends on the installed toolchain, the tool's configuration, the container, the
database. There is no static resolution; there is only execution.

Execution is not available to the gate, and the reasons are load-bearing rather
than stylistic:

- it is stdlib-only and dependency-free, so it can ship inside any accelerator;
- its JSON output is byte-stable, pinned by a double-run test;
- it runs in an offline CI with no secrets and no target toolchain installed;
- it is fail-closed, and a command that hangs or mutates would make it neither.

Extending static resolution to more command shapes (option A) buys the cheap
cases and leaves every real tool untouched. Executing inside the gate (option B)
costs all four properties above and still fails in CI, where the target's PHP and
node toolchains do not exist.

## 3. Decision

**Record the observation where it is already being made, and let the gate compare.**

The agent authoring the plan holds the target and must run the command anyway to
write a truthful expectation. Today that run is invisible: only its conclusion is
recorded, and nothing distinguishes a conclusion drawn from a real run from one
drawn from optimism.

Each `mode: command` verification whose command is not statically resolvable gains
a recorded baseline: what the command does on the unmodified target. The gate then
performs a static consistency check:

1. a non-resolvable command with **no** recorded baseline is an error — an
   expectation may not be declared for a command nobody observed;
2. an expectation asserting success while the recorded baseline records failure is
   an error — this is exactly the `eslint` defect, and it becomes mechanical;
3. the baseline is recorded against a target fingerprint, so a stale baseline is
   detectable the same way stale evidence already is.

**The expectation becomes differential.** Once the baseline is on the record, the
correct phrasing for a change-verification writes itself: not "eslint exits zero",
but "no error outside the recorded baseline". That is what verifying a change has
always meant; the absolute form was a mis-specification that happened to be
checkable only on green-field projects. The schema exemplar is rewritten with the
differential form, since it is the source of the pattern.

## 4. What this does and does not buy

**Buys.** The `eslint` class of defect becomes a mechanical error at generation
time. The gate stays static, offline and byte-stable — it compares two recorded
strings, it does not run anything. The cost lands on the author, who was supposed
to do the work regardless.

**Does not buy.** A fabricated baseline still passes. This ADR converts an
*unstated* assumption into a *stated, falsifiable* one: a wrong baseline is now a
false statement on the record, checkable by a human or by the optional runner
below, rather than an omission nobody can point at. That is the same bargain
[ADR-001](ADR-001-claim-adjudication.md) struck for claims, and it is the ceiling
of what a deterministic gate can reach.

**Optional companion.** Re-executing recorded baselines later — to catch drift
after the project changes — belongs outside the gate, in `harness/`, on the
precedent already set: *companion, not a component*. It is not required for this
decision and is not part of it.

## 5. Cost of implementation

| Surface | Change |
|---|---|
| `project-profile-schema.md` | one field per verification; exemplar rewritten to the differential form |
| `validate_skill_quality.py` | exact-field-set update plus the three consistency rules of section 3 |
| scanners / `skill-forge` | must run the command once and record what it did — the real cost, and the honest one |
| existing plans | the field is required only for non-resolvable `mode: command` entries, so search-only plans are unaffected |
| CI | unchanged: the new check is a string comparison |

The field-set is validated by exact equality, so this is a breaking schema change
for plans that already carry executable verifications. All four preserved runs
would need the field added before they validate again.

## 6. What was rejected

- **Extend static resolution to real tools.** Impossible in general: the result is
  a function of the installed toolchain, not of the target's bytes.
- **Execute inside the gate.** Costs dependency-freedom, byte-stability, offline
  CI and fail-closed behaviour, and does not work in CI regardless.
- **Do nothing and document the limit.** The defect is not rare: it appeared in
  three of four runs, in two different command shapes, and the schema's own
  exemplar teaches it.

## 7. What changed when this was implemented

Three deviations from section 3, each decided while writing the rules rather
than before.

**The state fingerprint was dropped.** Section 3 asked for a target fingerprint
so a stale baseline would be detectable the way stale evidence is. There is
nothing honest to hash. The result of `eslint assets` or `bin/phpunit` depends on
the installed toolchain, the container, and the database - none of which are in
the target's bytes - so a hash over the scanned paths would report "unchanged"
for a baseline invalidated by a dependency bump. A fingerprint that is wrong in
the direction of confidence is worse than none. Staleness is instead surfaced
where it already was: a baseline is re-recorded whenever the plan is
re-synthesized, and the existing regeneration-baseline diagnostics report a plan
that changed without one.

**`failing-remediated` was added, because rule 2 as written had a false
rejection in it.** A skill whose declared job is to eliminate the recorded
failures is *right* to promise the command exits zero. Rule 2 would have
rejected it. The distinction is made structurally rather than lexically - a
third `outcome` value the author states outright - so the lexical pattern only
has to be sensitive, and a reviewer can see and challenge the claim. A read-only
skill may not declare it.

**A cross-check the proposal did not have.** Where the command is a literal
search, this gate resolves it itself, so the recorded baseline is compared
against that resolution instead of being trusted, and a contradiction is an
error. That is the one place a fabricated baseline is mechanically detectable,
and it costs nothing: the machinery was already there for grading search
expectations.

**Measured before release.** Across four real runs and one externally authored
39-skill plan, the checks that would now require a baseline number 24, 7, 4, 13,
and 6. Every command expectation in all five plans is written in the absolute
form and none in the differential form - 16 absolute against 0 differential -
which is what section 1 predicted from the exemplar, and why the exemplar was
rewritten along with the rule.
