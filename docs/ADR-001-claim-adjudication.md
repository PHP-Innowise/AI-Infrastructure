> Решение: «LLM-судья» НЕ добавляется в `validate_skill_quality.py`. Поставляемый
> рантайм акселератора остаётся stdlib-only, детерминированным и офлайновым;
> семантическая адъюдикация claim (в первую очередь полярность — claim,
> утверждающий обратное процитированному коду) переносится в проект-компаньон
> `harness/` как линза Stage D. Взамен в самом гейте реализовано детерминированное
> сужение: поддержка claim теперь требует пересечения по *отличительной* лексике,
> а не двух любых токенов; калибровка — измерением, а не на глаз. Статус:
> принято и реализовано 2026-08-17 (детерминированная часть); линза в harness —
> предложение, не реализована.

# ADR-001 — Where claim adjudication belongs

**Status.** Accepted, 2026-08-17. The deterministic half is implemented in this
commit; the harness lens is a proposal with an implementation path, not code.

**Context files.**
`Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_skill_quality.py`
(the gate), `Infrastructure-Creator/tests/test_skill_quality.py` (its suite),
`harness/` (the Stage D companion), `docs/AGENT-ORCHESTRATION-DESIGN.md`
(why that companion exists at all).

---

## 1. The problem the deterministic gate does not solve

A schema 1.2 plan binds every generated skill to evidence, and every evidence
entry carries `supported_claims`: sentences the cited file (or the cited
`line_range` slice of it) is asserted to support. That binding is the
accelerator's whole defence against a generated skill that sounds authoritative
about code it never read.

Before this change the binding was a bag-of-words test. At
`validate_skill_quality.py:3195` (pre-change numbering) the gate computed the
meaningful tokens of the cited range and the meaningful tokens of the claim,
subtracted five service words (`project`, `confirmed`, `supports`, `uses`,
`runtime`), and required an intersection of two tokens — one, for a claim of
three tokens or fewer — before emitting `EVIDENCE_CLAIM_UNSUPPORTED`.

Two distinct failure modes follow from that shape, and only one of them is
fixable deterministically.

**(a) Common lexicon grounds anything.** Measured over 3997 real PHP files
sampled from the Symfony 7 and Laravel vendor trees in this repository, `class`
appears in 75% of files, `function` in 77%, `public` in 74%, `string` in 56%,
`array` in 43%. Two of those constitute "support". So the claim

> The public class exposes a private function that returns a string value from
> the configuration array

was grounded by any PHP file in any project. This one is a lexical problem and
has a lexical answer — see §4.

**(b) Negation is not modelled, and cannot be.** Support is set intersection.
Set intersection is blind to polarity, quantifier and scope. A claim that
asserts the exact opposite of the code it cites shares *all* of the code's
vocabulary and therefore scores maximally:

| Cited code | Claim | Score under the shipped rule | Verdict |
|---|---|---|---|
| `->andWhere('entry.tenantId = :tenant')` … `->setFirstResult($offset)` | "LedgerEntryRepository binds the tenant predicate as a parameter before applying offset and limit" | 9 tokens, 5 distinctive matched, 4 project-specific matched | grounded (true) |
| the same lines | "LedgerEntryRepository applies offset and limit **before** the tenant predicate parameter is bound" | 9 tokens, 5 distinctive matched, 4 project-specific matched | grounded (false) |

The two scores are not merely close, they are equal — measured against the
lexicons this change ships, not against the weaker rule it replaces. The same
holds for a middleware claim and its negation (9/3/3 honest against 12/4/4
inverted, both passing). No token-set rule, no threshold on that rule, and no
enlargement of the stopword lists changes this: the two claims are the same
multiset of words with one connective moved. Reading which one the code
supports is semantic adjudication.

## 2. Options considered

### Option A — an LLM judge inside `validate_skill_quality.py`

Call a model at the claim check: hand it the cited range and the claim, ask
whether the code supports, contradicts, or is silent about it.

- **Runtime contract.** The shipped accelerator runtime is stdlib-only and
  dependency-free by policy, and that policy is load-bearing rather than
  aesthetic: `validate_skill_quality.py` runs inside a generated project on
  whatever Python the developer has. An HTTP client, an SDK, or a key-loading
  path in this file makes the gate part of the dependency surface of every
  generated project.
- **Determinism.** `tests/test_skill_quality.py:1600`
  (`test_json_cli_is_nonzero_and_byte_deterministic`) runs the CLI twice as a
  subprocess and asserts `stdout` and `stderr` are byte-identical, with
  diagnostics sorted by `(severity, code, message)`. A sampled judge breaks
  that test by construction — and the test is not incidental: byte-stable
  output is what lets a generation be re-verified, diffed and audited.
- **CI reality.** The gate runs in `.github/workflows/ci.yml` under
  `infrastructure-creator-reliability` on stock Python 3.9 with no
  `pip install` step, no network calls and no secrets. A judge there either
  fails every run (no key, no network) or is written to fail *open* on any
  transport error — and a gate that silently passes everything when the network
  hiccups is worse than the honest bag-of-words gate it replaced, because the
  audit trail then claims a check that did not happen.
- **Blast radius.** A false "contradicts" from a judge blocks publication of a
  correct plan. The gate is blocking; the generator user has no override.

### Option B — do nothing

Keep the two-token rule, document the gap. Cheap and honest, but it leaves (a)
open, and (a) is trivially exploitable: a fabricating agent never has to invent
vocabulary, it just has to write in PHP-flavoured English.

### Option C — deterministic narrowing here, adjudication in `harness/`

Fix what is lexical with a lexical rule calibrated by measurement (§4); move
semantic adjudication out of the generation step and into the repo-root
`harness/`, which already exists precisely for work that needs a model, a
budget and a wall clock (§6).

The precedent is explicit. `harness/README.md`: the harness is *"a deliberate
companion, not a component"* — it lives outside the editions, the installer
never ships it, the inventories never list it, *"no edition requires or depends
on it, and the accelerator's own runtime stays stdlib-only. This directory
(with its own venv) is where the LangGraph dependency is allowed to live."*
Stage D in `docs/AGENT-ORCHESTRATION-DESIGN.md` is the same boundary stated as
design.

## 3. Decision

**Option C.**

1. No model call is added to `validate_skill_quality.py` or to any other shipped
   runtime script. The generation-time gate stays stdlib-only, deterministic and
   offline.
2. The lexical half of the problem is fixed in the gate, with thresholds derived
   from measurement over a real corpus rather than chosen by eye, and with the
   weaker tier emitting a **warning** because its measured false-positive rate
   does not justify blocking a generation.
3. Semantic adjudication of claims — polarity, scope, quantifier, "the code says
   something adjacent but not this" — is a **harness lens**, run out of band
   against a produced plan, never as a step of producing one.

## 4. What was implemented in the gate

The claim check now runs three conditions in sequence over the cited text
(`line_range` slice when declared, whole file otherwise):

| Condition | Diagnostic | Severity |
|---|---|---|
| fewer than the required plain-token overlaps (unchanged) | `EVIDENCE_CLAIM_UNSUPPORTED` | error |
| claim shares nothing outside PHP-keyword / licence-header lexicon | `EVIDENCE_CLAIM_UNSUPPORTED` | error |
| claim shares nothing outside software-English lexicon, **and** ≥65% of the claim's own vocabulary is that lexicon | `EVIDENCE_CLAIM_GENERIC_SUPPORT` | warning |

Three details matter for anyone re-deriving the numbers:

- **The lexicons are measurements, not opinions.** Both are raw
  document-frequency cuts — no hand curation — over 3997 real PHP files
  sampled from the `Task/app/vendor` dependency trees of the Symfony and
  Laravel editions (installed working-tree dependencies, not committed here):
  share ≥ 0.40 (26 tokens) for the language tier, share ≥ 0.04 (284 tokens)
  for the common tier. The cut is unedited, author names and hostnames
  included: licence headers make `fabien` exactly as uninformative as `class`.
- **DF is measured in the token space the matcher uses.** The cited side is
  expanded by splitting compound identifiers (`publishReminder` → `publish`,
  `reminder`), so the lexicon is measured the same way. This is not cosmetic:
  the words that most cheaply ground a fabricated claim are exactly the ones
  that move across the cut. Measured whole-token, `service` is 0.039,
  `configuration` 0.037, `handler` 0.032 — all outside a 0.04 cut; measured in
  the space the matcher actually sees they are 0.047, 0.057 and 0.055, inside
  the common tier where they belong.
- **Identifier expansion protects the honest side.** Honest prose writes
  "failure description" where code writes `failureDescription`. Without the
  split, honest claims lose their only distinctive overlap: measured on 2500
  docblock/code pairs, splitting cuts the false-error rate from 0.55–0.88% to
  0.18–0.27% and costs 0.10–0.34 points of fabrication detection, because the
  parts an identifier contributes (`value`, `name`, `type`) are themselves
  common lexicon.

**Measured cost and benefit.** Honest corpus: 2500 docblock summaries paired
with the code that follows them (the docblock text itself excluded from the
citation), of which 1485–2190 pass the pre-existing rule and are therefore at
risk of a *new* rejection. Fabricated corpus: generic-lexicon claims attached to
those same ranges.

| Cited span | honest error | honest warning | fabrications caught |
|---|---|---|---|
| 15 lines | 0.27–0.32% | 3.6–6.2% | 99.1% |
| 25 lines | 0.18–0.50% | 3.5–6.0% | 98.5% |
| 40 lines | 0.22–0.47% | 3.3–5.8% | 98.7% |
| whole file | 0.00–0.18% | 2.3–6.8% | 87.1–99.6% |

Each cell is a range because the figure is **not** a property of the gate alone:
it depends on how a docblock is paired with "the code it describes", and no
canonical pairing exists. The two independent measurements behind these ranges
used different pairing rules over different samples of the same vendor trees,
and the warning tier moved by roughly 2× between them while the error tier and
the detection rate stayed stable. Read the table accordingly — the error tier is
sound to well under 1%, the detection rate is sound above ~87%, and the warning
tier is only known to the nearest few percent. Anyone re-deriving these numbers
should expect their own pairing rule to move the warning column again; that is a
property of the corpus construction, not a regression. The load-bearing claim is
the *shape*: a two-order-of-magnitude gap between what the error tier costs
honest claims and what it catches.

On the honest control plans of the probe harness the margin is wide: every
control claim matches 3–10 project-specific tokens against a threshold of one,
and both controls validate at `PASS (0 errors, 0 warnings)`.

## 5. What this means for a user of the generator

**Caught now that was not before.** A claim written in PHP-flavoured English
that names nothing in the file it cites. The gate will say so, and for the
error tier it will refuse to publish.

**Still not caught — the class to plan around.** A claim that uses the file's
own vocabulary but asserts something the file does not say, most importantly its
negation. "The tenant predicate is applied *after* the offset", "an
unrecoverable message *is* retried", "a settled invoice *may* still be
allocated" — all of these will pass every deterministic tier, because they are
built from the cited code's own words.

Three practices, in the order they pay off:

1. **Narrow `line_range`.** A claim checked against ten bracketing lines is
   checked against the construct it is about; the same claim against a whole
   file is checked against everything the file happens to mention. Whole-file
   citation is where fabrication detection drops to 87% in the table above.
2. **Read the claim against the range at review time.** Polarity is the one
   thing the gate delegates to the human, and it is cheap to check when the
   range is bounded: the plan states a claim, the range is a dozen lines.
3. **Treat `EVIDENCE_CLAIM_GENERIC_SUPPORT` as a request to rewrite, not as
   noise.** A warning there says the claim carries no project-specific
   vocabulary that the range confirms. Usually the claim is true but written
   generically, and the fix is to name the class, constant or config key —
   which also makes the generated skill more useful.

## 6. Implementation path in `harness/`, if it is taken

The pieces already exist; a claim-adjudication lens is a new graph beside
`fleet_review`, not new infrastructure.

1. **Graph.** Add `harness/src/harness/graphs/claim_audit.py` mirroring
   `graphs/fleet_review.py`: `START -> scope -> [Send per claim batch] ->
   adjudicate -> collect -> gate -> record -> END`. `fleet_review` fans out over
   review lenses; this one fans out over evidence entries of one
   `skill-generation-plan.json`, batched so each worker sees a handful of claims
   and their cited ranges.
2. **Scope node.** Read the plan, resolve each `evidence[]` entry against the
   evidence target, slice `line_range`. The slice is the only context the worker
   gets — the same bound the gate uses, so the adjudication answers the same
   question the gate asks.
3. **Worker prompt.** Reuse the `CAPSULE_TEMPLATE` discipline: objective
   ("decide, for each claim, whether this range *supports*, *contradicts*, or is
   *silent about* it"), a JSON output contract mirroring `parse_findings`
   (`evidence_id`, `claim`, `verdict`, `quote`, `confidence`), and explicit task
   boundaries (report only, no file writes). Require a verbatim quote from the
   range for any `contradicts` verdict — an unquotable contradiction is a
   hallucinated one and is dropped in `collect`.
4. **Gate node.** `interrupt` with the contradiction count, exactly as
   `fleet_review` gates on findings. Nothing is written back to the plan without
   human approval: an automated verdict must never silently rewrite evidence.
5. **Budget.** `HarnessConfig.budget_usd` already enforces a hard ceiling
   between fan-out branches; claim adjudication is cheap per call and bounded by
   the plan's evidence count, so the default $5 is generous. Batches beyond the
   ceiling are reported as skipped, never dropped.
6. **Offline rehearsal.** The `dry-run` worker gives the test suite deterministic
   canned verdicts, so the graph is testable in `harness/tests/` without a host,
   a key, or tokens — the same pattern `test_graph.py` uses today.
7. **Where the result lands.** Through the project's own `context.py`
   blackboard, like every other harness node: a finding record per contradicted
   claim, auditable next to the interactive sessions' records. The generator
   itself remains unaware the harness exists.

**Explicitly out of scope for that lens.** It must not become a required step of
`infra-generate`, must not be invoked by any skill or hook, and must not be
listed in an installer inventory. The moment a generation *needs* it, the
stdlib-only boundary is gone and this ADR has been reversed rather than
extended.

## 7. Consequences

- The gate keeps its properties: no dependencies, byte-stable output, runs on
  Python 3.9 in an offline CI with no secrets.
- The lexical fabrication route is closed at a measured cost of well under 1%
  false errors (0.0–0.5% across two independent measurements); the weaker tier
  is a warning, so its cost — 2–7%, the figure most sensitive to corpus
  construction — never blocks a generation.
- Polarity remains undetected at generation time. This is a known, documented
  gap, mitigated by narrow `line_range` and human review, and addressable out of
  band by §6.
- If the harness lens is never built, nothing regresses — the accelerator is
  exactly as capable as it is today, with one fewer trivially exploitable route
  and an honest statement of what remains.
