# Changelog

All notable changes to Infrastructure-Creator are documented here. Format loosely follows [Keep a Changelog](https://keepachangelog.com/); versions are for this generator tool, not for anything it generates. The released version has a single machine-readable source: the root `VERSION` file - profiles, `AGENTS.md` stamps, and `.infra-manifest.json` files all read it.

## Unreleased

### Breaking

- **Plan schema 1.4.** Two skill members, top level untouched again. Schema 1.3
  joins 1.0-1.2 as readable-for-audit and publication-ineligible; new synthesis
  emits 1.4.
  - `claim_ids` names the reconciled claims from `project-claims.json` a skill
    rests on. With it, the lost-invariant check stops guessing at wording: a
    skill answers the question outright, and a claim reconciliation never made
    is `CLAIM_ID_UNKNOWN`.
  - `evidence_dispositions` records the evidence inside a skill's own declared
    paths that it deliberately does not use. Synthesis reads the whole ledger
    and writes one contract at a time, so a passed-over finding left no trace -
    the plan looked identical whether the author judged it irrelevant or never
    saw it. Evidence inside the skill's own `path_contracts` or `ownership`
    paths that is neither cited nor ruled out is `SKILL_EVIDENCE_UNDISPOSED`.
    Measured on five real plans before release: a median of 0-3 undecided items
    per skill and 4-47 per plan, because the obligation grows with what a skill
    claims rather than with the size of the ledger.
  - The 36-skill honest corpus was migrated with it and still passes with zero
    blocking diagnostics, which is what the corpus is for.

- **Plan schema 1.3.** One migration, three nested shapes; the top level and the
  skill field set are 1.2's, so a 1.2 plan differs from a 1.3 plan only inside
  `required_procedure_roles[]`, `verification[]`, and `evidence[]`. Schema 1.2
  joins 1.0 and 1.1 as readable-for-audit and publication-ineligible; new
  synthesis emits 1.3.
  - *A catalog obligation now names what carries it.* A role entry has exactly
    `role`, `requirements`, `evidence_ids`, and `procedure_step_ids`, and both
    reference sets must resolve inside the skill. Declaring an obligation was
    not carrying it: 1.2 accepted a role plus a sentence, so the field could be
    satisfied by writing it down.
  - *Three or more obligations discharged by one step is `PROCEDURE_ROLE_COLLAPSED`* -
    the "one general inspection step" shape the readiness criteria name.
    Calibrated before release on both honest corpora: 0 of 45 skills across four
    real runs, against 37 of 39 in an externally authored plan. Median procedure
    length is 5 steps in our runs and 1 in theirs, so the rule separates a
    template from honest work instead of taxing it. Only the unambiguous form
    blocks; any ratio beyond it would be calibrated on a corpus that does not
    exist yet.
  - *An executable check records what its command already does* ([ADR-002]).
    `verification[].baseline` carries the command, the observation, and one of
    `passing` / `failing` / `failing-remediated`. The gate cannot run `eslint`:
    the result is a function of the installed toolchain, not of the target's
    bytes, and executing would cost the gate its dependency-freedom, byte-stable
    output, offline CI, and fail-closed behaviour. So the observation is recorded
    by the agent that has to run it anyway, and the gate compares two strings.
    Promising outright success against a `failing` baseline is
    `VERIFICATION_BASELINE_CONTRADICTED` - the measured defect: a generated skill
    declared `eslint assets` exits zero on a target where it reports 189 errors,
    so it raised a blocking finding on untouched code on every run. Where the
    command is a literal search the gate resolves it and cross-checks the
    recorded baseline rather than trusting it.
  - *Evidence may state an absence.* "PHPStan is installed and invoked from
    nowhere" could not enter the ledger at all, because every entry needed a path
    and a fingerprint. An `absence` entry carries a subject, a literal search the
    gate resolves itself, and the matches it accounts for; the resolution must
    equal that set exactly, so the negative is pinned to a file set and goes
    stale loudly rather than being asserted once.
  - Migration cost, measured: the checks that now require a baseline number 24,
    7, 4, 13, and 6 across the five plans. Every command expectation in all five
    is written in the absolute form and none in the differential one, which is
    what the schema's own exemplar taught - so the exemplar was rewritten with
    the rule, and its procedure expanded to one step per obligation rather than
    six obligations on one step.

[ADR-002]: ../docs/ADR-002-executable-verification-baselines.md

### Added

- **`infra-validate` - a mandatory content review-and-repair phase between the
  wrappers and publication, with a deterministic gate that refuses to take the
  reader's word for having looked.** The mechanical validators prove form -
  evidence resolves, skeletons differ, routing parses - and the adversarial
  plan review covers the plan; nothing systematically read the *content* of
  every generated file. `bootstrap-verifier`'s own text admitted the gap: the
  evidence-to-content bar for `AGENTS.md`, `DOD.md`, the hooks, and the seeded
  memory "applies here by hand, because no gate enforces it yet".
  Now it does. `infra-validate` (the sixth sanctioned orchestrator) partitions
  the publication plan into lanes - skills, wrappers, policy, hooks, memory,
  the run's own profile and plan - and fans out parallel read-only
  `content-reviewer` instances (the 25th skill), each judging every file on
  four dimensions the deterministic gates provably cannot ask: uniqueness
  (identity-erasure: scaffolding that would read identically against another
  PHP repository was copied, not generated), completeness (executable end to
  end without its author), accuracy (claims match the current target), and
  coherence (siblings agree). Blocking findings are repaired through the
  owning forge as contract amendments - never in place, never by invention;
  what evidence cannot settle escalates - bounded at two rounds, with
  `validate_skill_quality.py` and `validate_generated.py` re-run after any
  repair. The review record (`infra-validate-review.json`) is held by the new
  `validate_content_review.py`: full surface coverage, all dimensions
  answered, verbatim exemptions limited to the sanctioned stack-agnostic
  runtime, no open blockers, escalations, or accepted blocking findings,
  reviewer independence, and the repair-round bound - stdlib-only,
  fail-closed, byte-stable, like its siblings. `infra-generate` runs the phase
  as step 9 (before the manifest, so repairs never invalidate hashes),
  `infra-update` runs it over its update staging, `infra-build` treats an
  escalation as a checkpoint, `bootstrap-verifier` requires the record next to
  the scan-coverage and plan-review gates, and standalone
  `/infra-validate <target>` reviews a published accelerator through its
  manifest and publishes repairs transactionally with a manifest hash refresh.
  Sixteen regression tests pin the gate (`tests/test_content_review.py`).
- **The plan is damaged on purpose before it is published.** A plan that passes
  every gate proves one half of the bargain - that the rules are satisfiable. It
  says nothing about the other half: whether the rules would have noticed had the
  plan been worse. A gate can decay in ways nothing reports - a rule made
  conditional on a field that stopped being emitted, a pattern that stopped
  matching after a rename, an exemption that widened - and every plan still
  passes, which is what a reviewer reads.
  `scripts/validate_plan_mutations.py` damages the plan one way at a time and
  requires the gate to object by name: a dropped catalog obligation, every
  obligation collapsed onto one step, a lost verification baseline, an
  expectation promising success against a failing one, a runtime check promising
  zero where the contract declares otherwise, a routing fixture naming its own
  answer, a skill that stops naming its claims, and one that stops ruling out the
  evidence inside its own paths. `MUTATION_UNCAUGHT` means a rule stopped firing,
  and the plan may not be published on it.
  The damages are derived from the plan rather than hardcoded, so the step
  travels to any target, and a damage the plan has no shape for is reported as
  `MUTATION_NOT_APPLICABLE` - a warning, deliberately visible, because a silently
  skipped check reads exactly like a passing one. `MUTATION_CONTROL_DIRTY` says
  the plan was not clean to begin with, so the check proves nothing.
  Verified on both corpora: 8 of 8 damages exercised and caught on a plan built
  from a real Symfony project, and 5 of 8 on the synthetic 36-skill corpus, whose
  three inapplicable damages are named rather than dropped. The mutated plans go
  to a scratch copy, and a test asserts the real plan's bytes are untouched.

- **A runtime command is now graded against its contract, closing the one hole
  the fifth end-to-end run found in ADR-002.** The memory quartet verifies itself
  with the seeded runtime, and on a first generation that runtime does not exist
  on the target yet - the generation installs it. So a baseline "recorded on the
  unmodified target" was either impossible or, as in that run, taken on a
  different tree and explained in prose no gate could check.
  The runtime is fixed and shipped by this generator, so its behaviour belongs to
  `runtime-contract.json`, which now declares what each command's exit codes
  mean, read out of the scripts themselves rather than from one observation.
  A runtime-fixed skill needs no baseline for such a command; its expectation may
  not promise the command succeeds outright when the contract declares a nonzero
  exit; and a baseline recorded anyway - which an update legitimately can, since
  by then the runtime exists - may not say what the contract does not declare.
  Calibrated on all five runs: 2 of 23 runtime checks fire, both of them skills
  promising `context.py validate` exits zero, which the contract declares it does
  not whenever an index is stale - the state this repository is in right now.
  A test also pins the two lists together, so a command may not be attested
  read-only without declaring what its exits mean.

- **A narrower disposition now overrules a broader one.** Running the discovery
  gate against a real Symfony target on the first end-to-end run of schema 1.4
  reported `config` as "both covered and forbidden": the architecture scanner had
  covered the configuration tree, and the security scanner had marked
  `config/jwt` not-permitted. That pair is how a scan says the right thing -
  everything in config except the signing keys - and reading it as a
  contradiction would force every scanner to enumerate a tree file by file,
  which is exactly what the glob surfaces exist to avoid. The most specific
  statement about a path now wins; what stays blocking is two statements at the
  same specificity, and a narrow claim to have read inside something broader
  that forbids it.

- **Discovery now records what it did *not* read.** A scan that missed a
  subsystem and a scan that covered it produce the same artifact - a list of
  what was found - so the omission is invisible until a generated skill turns
  out not to know the subsystem exists. Each scanner now writes a third
  artifact, `<scanner>-coverage.json`, giving every surface it saw one of four
  dispositions: `covered`, `excluded`, `truncated`, `not-permitted`.
  `scripts/validate_scan_coverage.py` holds those records against the target and
  against each other, and blocks a surface nobody dispositioned, coverage
  claimed with no evidence inside it, evidence cited from outside what a scanner
  says it read, two scanners disagreeing about a forbidden surface, and any
  claim to have covered a secret-bearing file. Truncations are warnings by
  design and belong in the confidence summary verbatim - a cap nobody sees reads
  as full coverage.
  The secrets rule becomes mechanical rather than advisory, and is enforced
  without ever opening the file: a test asserts the gate never reads a path
  whose disposition it is judging.
  Same invariants as the other gates here: standard library only, no execution,
  no network, byte-stable JSON, fail-closed. Measured on three real targets
  before release - 12, 23, and 64 surfaces requiring a disposition, most of them
  collapsible into `dir/**` entries, so a coverage record is on the order of
  twenty lines per scanner rather than one per file.
  This is also where a scanner's "sibling report absent" note is revisited: by
  reconciliation time the parallel siblings have landed, so a claim made against
  a missing neighbour is confirmed or withdrawn rather than left standing.

- **The regeneration baseline sees past skills and files.** A second generation
  could keep every skill and every evidence path and still drop the invariant
  that made one of them worth generating, or the ownership that kept two of them
  from colliding - and the comparison reported "no coverage lost".
  `--baseline-plan` now also names every dropped critical invariant, owned path,
  and module, one warning each, with the summary repeating the names rather than
  a count.
  What it deliberately does *not* compare is identifiers. Measured across three
  consecutive real regenerations of one target, comparing invariant, ownership
  and verification *ids* reported 5-12, 6-8 and 34-42 losses per run, nearly all
  of them the same thing renamed; a rule that cries forty times is read zero
  times. An invariant is therefore compared by what it says - two statements are
  the same rule when they share two meaningful words, measured at 0, 1 and 2
  drops per regeneration against 2/2/4 at three words and 2/5/7 at four - and
  ownership by the paths it holds. A file at the repository root is not a
  module, so `composer.json` stopped being reported as one.

- **A routing fixture may no longer name the skill it expects to win.**
  "Route architecture-implementer work to architecture-implementer" tests string
  matching, not routing: no arrangement of skills could get it wrong, so it
  proves nothing about whether the boundaries hold, while looking like a routing
  test in every count and report. `ROUTING_CASE_TAUTOLOGICAL` blocks it.
  Calibrated on both corpora: 0 of 148 fixtures across four real runs, and 183 of
  183 non-runtime fixtures in an externally authored plan - 40 of those verbatim.
  Runtime-fixed skills are exempt: a prompt about reloading memory cannot avoid
  saying "memory" without becoming artificial.
  The first implementation was wrong in the safe direction and the tests caught
  it: the tokenizer keeps compound identifiers whole, so `firebase-services` in a
  prompt did not match the skill named `firebase-services`. Both sides are now
  spoken aloud before comparison.

- **Seven parallel scanners now produce one reconciled claim set.** Each stated
  its findings in prose inside its own ledger; nothing merged them, so nothing
  noticed when two contradicted each other, and nothing noticed when an
  invariant one of them confirmed never reached the plan. Reconciliation
  promotes those claims into `project-claims.json` - typed by class
  (`invariant`, `capability`, `convention`, `risk`, `integration`, `command`),
  priority and status, each naming the evidence it came from and the scanners
  that made it. Nothing there is a new finding: a claim resting on evidence no
  ledger carries is `CLAIM_EVIDENCE_UNKNOWN`.
  Unresolved contradictions are always reported, and block when they touch a
  high-priority invariant - a skill cannot be told to honour something discovery
  is still arguing about.
  `CLAIM_INVARIANT_LOST` reports a high-priority invariant discovery confirmed
  and the plan does not carry. Dropping one may be right; leaving no trace of
  the decision is what makes it indistinguishable from an oversight.
  Calibrated on four real runs before release: 9, 12, 7 and 8 high-priority
  invariants, and 0 of the 36 lost - the rule does not tax honest work, it
  catches the one that goes missing.
  Designing it corrected the join once: `profile-synthesizer` renumbers evidence
  when it merges seven ledgers, so matching discovery to the plan by evidence id
  would have reported every honest plan as having lost every invariant. The join
  is the cited source - a path, a URL, or an absence subject - which survives the
  merge.

- **The plan must now survive an adversarial review, and the review must show
  its work.** A deterministic gate proves a contract is well formed and its
  evidence resolves; it cannot ask whether the skill would be picked for a
  request nobody has written yet, whether the contract survives having its nouns
  removed, or whether the prescribed command really does what the plan says.
  Those need a reader - and what a gate can do is refuse to take the reader's
  word for having looked. `skill-plan-quality-report.json` records eight answers
  per selected skill, and `scripts/validate_plan_review.py` holds it against the
  plan: every skill answered once, no dimension skipped, no fixture prompt that
  names the skill it expects to win, and no open blocker.
  `verification_realism` must list the commands the reviewer actually ran, and
  they must be the ones the plan prescribes. That one is not ceremony: in the
  third preserved run a skill's broken verification was found only by the judge
  that executed it and missed by the judge that read it.
  The review must also declare itself independent of the author, because a
  contract's author is the worst judge of whether it is distinguishable from a
  template.
  `infra-generate` refuses to start until it passes.

### Verified

- **The forge must render the approved contract, not a summary of it.** The
  trace checks graded a whole contract member as one bag of words, so an
  authored skill could satisfy `required_procedure_roles` by echoing two words
  from any one role while dropping the other five. Rendering is now checked per
  member: an approved procedure step or catalog obligation that never reaches
  the page is `SKILL_STEP_NOT_RENDERED` / `SKILL_ROLE_NOT_RENDERED`.
  Measured on 45 real authored skills before release - 0 of 191 planned steps
  and 0 of 135 obligations untraceable - so the bar costs honest work nothing
  and only catches what was dropped between approval and authoring.

- **The regression catalog is now an index of coverage that exists.** Every rule
  shipped this cycle has a named case in
  `tests/fixtures/skill-quality/cases.json`, and a new test fails if any
  catalogued code is not actually asserted by some test - a catalog that lists
  coverage it does not have is the same failure as a scan that records what it
  found and not what it missed. Seventeen cases added, including the four
  calibration entries that pin what must stay clean.
  The large corpus also carries mutation tests now: dropping a claim reference,
  a catalog obligation, an obligation's wiring, a recorded baseline, or an
  invariant's only assertion is each caught by name.

- **Three project shapes, not one.** A corpus of one shape proves the gate
  admits one shape: the rules that never fire on a modular application - shared
  ownership, provider capability, an approved network policy - would be free to
  stay miscalibrated forever. The corpus now also builds a tenant shape (a
  central store and a per-tenant one sharing paths, 9 shared ownership entries)
  and a provider shape (18 external-side-effect skills under
  `sandbox-with-approval`), and both validate with zero blocking diagnostics.
  Writing them produced a false green worth recording: the project shape was
  shadowed by a local variable of the same name inside the builder, so all three
  shapes built the identical plan and three new tests passed while proving
  nothing. It was caught by looking at the artifact rather than at the passing
  test, and a test now asserts the three plans actually differ.

- **An honest thirty-six skill plan validates with zero blocking diagnostics.**
  Every threshold in this gate was calibrated on plans of nine to thirteen
  skills, because that is what our runs produce; the failure that costs most is
  not a missed defect but a false rejection at scale, and nothing in the suite
  would have noticed one. `tests/test_large_plan_calibration.py` now builds a
  synthetic thirty-six skill plan - eighteen subject areas, each with the skill
  that changes it and the skill that reviews it, drawn from separate procedure
  and verification pools - and asserts it passes clean.
  Writing it found seven defects, all in the corpus rather than the gate, which
  is the outcome that makes it worth keeping: a reviewer inheriting a
  write-oriented step, a writer verified only by a text search, an exclusive
  path swallowed by a sibling's write glob, eighteen write-capable agents in one
  parallel stage. 124 blocking diagnostics on the first run, 0 on the seventh.
  The collapsed twin - the same plan with every obligation discharged by one
  step, which is the shape an externally authored plan of this size actually
  had - is rejected 36 times over.
  The similarity signal is pinned here too: 54 warnings across 36 skills, every
  one naming a module's workflow beside its own review, and no unrelated pair.
  That is the same pass that produced 1642 warnings on the external plan, and
  the reason it stays a signal rather than a gate.
  The corpus is synthetic and carries no client name or content.

- Independent measurement of the two gate additions above (search grading and
  `--baseline-plan`), against the question that decides whether a gate is worth
  having: does it catch the real defect without failing honest work?
  *Catches the real thing.* Run 2's `content-publication-review` is reported as
  exactly one error naming the two surplus files
  (`src/Entity/Article.php, src/Entity/Job.php`); real `grep -rn "setState" src`
  on the target answers those two plus the importer, so the claim that *only*
  the importer prints is false on the unchanged checkout. Run 1 scopes the same
  question to `grep -n setState src/Importer/WordpressArticleImporter.php` and
  stays PASS at zero diagnostics - the rule separates the two runs on wording,
  not on skill name.
  *Names what was lost.* Run 2 against run 1 as baseline names
  `security-review` together with its five baseline source paths, and names all
  eleven dropped evidence paths individually; the summary line repeats the
  names rather than a count.
  *Does not fail honest work.* An independent honest skill set was built on an
  unrelated domain (courier dispatch) with eleven verifications spanning: a
  search with no exclusivity claim; a true exclusivity claim; `-i`, `-w` and
  `--include=` flags (the `--include` case carries a `.css` decoy holding the
  same token, which the engine correctly excludes); a pipeline; `-A3` context;
  an `-E` regular expression; a `skip_condition` covering a genuinely absent
  path; and the non-mutating `vendor/bin/phpunit` and `php -l`. The engine's
  matched-file sets were checked against real `grep` and agree exactly. In all
  three validation modes (`full`, `--plan-only`, `--allow-partial-skills`) the
  result is PASS, 0 errors, 0 warnings. Four honest phrasings of an exclusivity
  claim - full paths, basenames, class names, prose subjects, and a Russian
  wording - are all clean, while the same command with an expectation naming
  one of two matched files fires.
  *Never executes.* No `subprocess`, `os.system`, `popen`, `exec` or
  `shell=True` appears anywhere in the validator; the added code reaches the
  target only through `Path.iterdir`, `stat`, and `read_bytes`, and declines
  every form it cannot model rather than running it.
  *No baseline flag, no change.* With `--baseline-plan` absent, `--json` output
  is byte-identical to the pre-change gate on every honest input measured
  (run 1 in full mode, and the dispatch set in all three modes); the
  `coverage_baseline` key is absent. The single behavioural difference across
  every input measured is the one true positive on run 2. Three repeat runs of
  the baseline command are byte-identical.
  One pre-existing limit was separated out and is *not* attributable to this
  work: a verification whose command contains a pipe is refused as
  `COMMAND_RISK_BLOCKED (SHELL_COMPOSITION)` by `analyze_commands.py`, which
  this change does not touch; the pre-change gate refuses the identical command
  with the identical diagnostic.

### Fixed

- **Two shipped gates required mutually exclusive encodings of the same
  frontmatter, so no generated flow command could pass both.** `validate_generated.py`
  reads a flow's stages as inline mappings - `- { phase: ..., agents: [...] }`,
  the form every shipped edition writes - while `validate_flow_contracts.py`
  parsed only the block form. Measured on the reference command a shipped
  edition carries: the flow-contract parser saw 0 stages where the publication
  gate saw 8, and on the block form the counts inverted. Since `infra-generate`
  runs both, generation could not have completed on any target.
  Found by building a publishable bundle for the first time rather than by
  reading either gate. The flow-contract parser now reads the inline form as
  well, treating a named checkpoint as a checkpoint - the stage stops either
  way - and a test pins the two parsers to the same count on both forms.


- A selected candidate could ignore every obligation its catalog places on it.
  `required_procedure_roles` existed in schema 1.2, but nothing said what
  belonged there, so both measured plans filled it with one universal trio -
  `load-evidence`, `execute`, `verify` - for every skill: 9 of 9 in our fourth
  run and 35 of 35 in an external 39-skill plan. A database designer, a WCAG
  reviewer and a Marketo integration all declared the same three roles, while
  the catalog's real obligations (derive constraints from invariants, design
  indexes from evidenced access paths, cover denied paths) appeared nowhere.
  A trio that describes every skill ever written describes none of them.
  `candidate-registry.json` gains an optional `roles` array carrying the
  catalog's obligations in machine-readable form, extracted from the catalog
  prose rather than invented, and a selected candidate that fails to cover them
  is `CATALOG_ROLE_UNCOVERED`. The field is optional on purpose: obligations are
  filled in tranches, and a candidate nobody has described yet stays unchecked
  instead of blocking generation. Thirteen candidates are described in this
  first tranche, chosen to cover both measured corpora.
  The schema exemplar taught the defect and is rewritten: it now shows the six
  real obligations of `testing` instead of the trio. This is the third defect
  this cycle traced to an example in the documentation rather than to the code.
  Calibrated on both corpora: the rule fires on every skill whose candidate
  declares roles - 4 of 4 in our plan, 12 of 12 in the external one - which is
  the universal defect, not a false positive.
- A skill that writes could prove itself by grepping for the text it had just
  written. Three runs against a real project shipped exactly that: `testing`
  declared `tests/**`, promised a suite invocation and verified itself with
  four searches, never running PHPUnit; `coder-frontend` and
  `media-storage-integration` did the same for the code they emit. Searching
  for your own output confirms authorship, never behaviour. A write-capable
  skill whose verifications are all text searches is now
  `WRITE_VERIFICATION_SEARCH_ONLY`. A read-only reviewer is exempt by nature -
  inspecting IS its work, and all seven reviewers of the third run stay clean -
  and a target with nothing runnable can still say so with a `manual` check,
  so the rule has an honest way out that a fabricated command does not.
- A runtime-fixed skill could attribute one runtime command's job to another
  and nothing noticed. `project-brain` stated that `parity` reports drift
  "between the governed records and the runtime index"; `parity` compares the
  canonical mirrors across editions, and it is `validate` that inspects
  governed records. The same misattribution sat in the first run too, unseen.
  `runtime-contract.json` now declares a purpose per command, copied from the
  commands' own help text so the contract is ground truth rather than
  restatement, and `RUNTIME_COMMAND_DESCRIPTION` reports a description whose
  vocabulary fits a *different* declared command strictly better than its own.
  The signal is comparative on purpose: an honest paraphrase scores no better
  against a sibling than against its own entry, so wording alone cannot trip
  it - only borrowed subject matter can. Flag variants of one command are not
  rivals (`status` and `status --json` are one operation), which was the
  difference between reporting the real defect and reporting an accurate
  description of `status` merely because it mentioned its own output.
  Severity is warning: the contract's vocabulary is small, and a wrong
  rejection would cost more than a named miss.
- A verification could promise a result the target already contradicts, and
  the gate had no way to notice. It graded the *form* of
  `verification[].command` - safety, mutation class, falsifiable wording -
  but never asked the target what the command would actually answer. Found by
  running the pipeline twice against the same real Symfony/UniteCMS project:
  both runs passed, and the second shipped `content-publication-review` with
  `grep -rn "setState" src` whose `expected_result` claims *only* the
  Wordpress importer prints. The unchanged checkout answers three files - the
  setter declarations in `src/Entity/Article.php` and `src/Entity/Job.php`
  print too - so the skill would raise a blocking finding on every run over
  clean code.
  The gate now resolves a literal search itself, in Python, over target
  files. It never executes anything: the static, offline, secret-free
  invariant is the reason the gate cannot hang or harm, and an engine that
  shelled out to `grep` would trade that away for nothing. Three readings of
  `expected_result` follow: a search the clean target never answers is a dead
  check (`VERIFICATION_SEARCH_DEAD`); a target path the expectation names,
  inside the searched scope, that the search does not reach is
  `VERIFICATION_SEARCH_EXPECTATION`; and an exclusivity claim (`only`,
  `exclusively`, `no other`, `единственн`) that names files while the target
  answers more is `VERIFICATION_SEARCH_EXCLUSIVITY` - exactly the defect
  above.
  Calibration is deliberately timid, because a gate that fails honest
  verification is worse than the miss it closes. Only what can be modelled
  exactly is graded: `grep`/`egrep`/`fgrep`, a literal pattern, explicit
  paths, and the flags whose effect on the matched-file set is understood
  (`-r/-R`, `-n`, `-i`, `-w`, `-F`, `-E`, `-l`, `--include=`). A pipeline, a
  substitution, a regular expression, `-e`/`-v`/`-A`, a directory without
  `-r`, a path escaping the target or reached through a symlink, `rg` (whose
  ignore-file semantics this engine does not model) - all are declined, not
  failed. A `skip_condition` that admits a path may be absent absorbs its
  absence. Binary files carry no line evidence and are skipped; a scan past
  the byte/file budget reports a warning and grades nothing. An exclusivity
  claim is only ever contradicted when the search spans more than one file
  and the expectation does name some of them, so `admin-only` as a scope
  adjective and "only three lines print" both stay untouched. Both real runs
  were re-measured: the first stays PASS at zero diagnostics, the second
  reports exactly one error - the defect.

- The body-path check made its verdict depend on where the text happened to
  wrap. Creation intent was read from the physical line carrying the code
  span, but generated bodies are hard-wrapped near eighty columns, so in "a
  reviewer would add at `tests/Booking/HoldExpiryTest.php`" the verb that
  marks the path as one to be created lands on the previous line as often as
  not. Measured A/B on identical prose: unwrapped passed, wrapped failed with
  `SKILL_BODY_PATH_MISSING` - a correct instruction rejected for its line
  breaks. The context is now the paragraph. A blank line still bounds it, so
  a create verb in one instruction cannot excuse an invented path in the
  next, and that direction is pinned by its own test.

- The seven discovery scanners told their agents to produce something the
  evidence gate cannot accept, and four of them answered wrong on a real
  target. Found by running the pipeline end to end against a Symfony/UniteCMS
  project; none of it was visible to the tests or the fixtures, because every
  fixture already carried hand-built evidence.
  - **No scanner knew what evidence is.** The profile schema requires each
    `evidence[]` entry to carry an id, a target-relative path, a current
    `sha256:` fingerprint and non-empty supported claims, but the word
    `sha256` appeared in no scanner; `stack-scanner` never mentioned
    fingerprints or evidence ids at all. Executed literally, a scanner
    produced zero gate-eligible evidence and every agent in the run had to
    invent fingerprinting for itself. The record shape, the id namespace, the
    whole-file digest recipe, the anchor grammar and the bounded-range rule
    now live once in
    `stack-scanner/references/scan-evidence-contract.md`, which all seven
    scanners point at; the contract's member list is pinned by test to the
    validator's own allow-list and to the schema's source-type vocabulary.
  - **Report and ledger were conflated.** Each scanner mandated "exactly one
    findings file" in Markdown and specified no machine-readable form, while
    the rest of the pipeline needs structure. The two are now separate
    artifacts, both mandatory, and "exactly one" attaches to each kind:
    `<scanner>-findings.md` for the human reviewer and
    `<scanner>-evidence.json` for `profile-synthesizer` and the validators.
    The seven report templates moved into the contract's appendix A, which
    kept the change inside the context budget.
  - **The task directory was unspecified.** `TASK-{N}` let parallel scanners
    in one run create both `tasks/TASK-1/` and `tasks/TASK-001/`. The canon is
    now three-digit zero padding (as in the schema's own examples), stated in
    the contract, in all seven scanners, and in `infra-scan`, which allocates
    the directory.
  - **Cross-scanner inputs had no address.** `conventions-scanner` steps 2/8
    and `domain-behavior-scanner` steps 3/9 said "take it from stack-scanner
    findings" without saying where, so the step was unexecutable in a
    standalone run. Siblings are now addressed as
    `tasks/TASK-{NNN}/<sibling>-findings.md` / `-evidence.json`, and a missing
    sibling degrades explicitly: derive only the minimum first-hand, mark it
    `inferred`, record a `Missing sibling input:` gap line - never invent the
    sibling's verdict, never block on it.
  - **Two scanners contradicted each other about `.env`.** `integration-scanner`
    forbade reading it; `security-compliance-scanner` step 2 required listing
    its keys. Resolved once, in identical wording in both: key *names* may be
    cited from non-secret committed sources (`config/**`, container/CI config,
    deploy scripts, committed `.env.example`/templates, `env()`/`getenv()`/
    `$_ENV` call sites); values and the `.env` file itself are never read,
    recorded or fingerprinted, and a key knowable only from a real `.env`
    stays `unknown`.
  - **Four hardcoded assumptions answered wrong on the target.**
    `infra-ops-scanner` recognized containerization only by
    `Dockerfile`/`docker-compose`, so a DDEV project reported "no containers";
    `stack-scanner` read the PHP version from `composer.lock` `platform`,
    which holds the *constraint* `>=8.2.0`, while the pin lived in
    `platform-overrides`; `stack-scanner` searched for commands only in
    Composer/package-manager scripts, while `deploy.sh`
    (`git reset --hard`, `doctrine:schema:update --force` in production),
    `.ddev` hooks and three `#[AsCommand]` console commands sat outside them,
    yielding "no risky commands" from the scan the command gate exists for;
    and `integration-scanner` enumerated integrations from `composer.json`
    `require` alone, returning zero on a project whose providers arrive
    through a CMS meta-package and `config/packages/**`. Each detection step
    is now a signal *class* with a breadth checklist in the contract, and an
    absence verdict is reportable only after the whole class was searched and
    named.

- The runtime-fixed memory quartet could satisfy no consistent set of rules,
  and when it did pass it was reported as if it had been derived from the
  target. Both were found by running the pipeline end to end against a real
  Symfony/UniteCMS project.
  - **Contract contradiction.** The schema promised that `memory-bank`,
    `project-brain`, `checkpoint` and `memory` "may have empty
    `evidence_ids`/`source_paths` during synthesis", while
    `routing_cases[].evidence_ids` was validated non-empty *and* required to
    be a subset of the skill's own evidence. With no declared evidence the
    quartet could satisfy neither branch: any value failed the subset check
    and no value failed the non-empty check. Emptiness now propagates
    consistently - a `runtime-fixed` skill's routing cases may carry an empty
    `evidence_ids`, exactly like its selection conditions and path contracts
    already could. A non-empty list that is not a subset is still blocking for
    every kind.
  - **Vocabulary contradiction.** `php-process-skills.md` assigned the quartet
    (and `skill-creator`/`reflect`) the phase `utility` and five other
    candidates `execution`, neither of which exists in the flow vocabulary the
    schema and `validate_flow_contracts.py` accept
    (`understanding`/`planning`/`implementation`/`verification`/
    `finalization`). The catalog phase is copied verbatim into `phase`, so a
    plan built from those rows was unbuildable. Every row now uses the real
    vocabulary, the column documents why it is closed, and a test pins each
    catalog phase to the validator's set. The catalog also quoted a
    `context.py --mode lightweight` form that the runtime contract does not
    define; it now quotes the contract's `turn --task-id ID --flush`, and a
    test asserts every `context.py` subcommand named in catalog prose exists
    in the contract.
  - **Status of the quartet.** Measured on the real run, the four skills
    contained zero identifiers of the target, 77-85% of their lines carried no
    project token, and all four cited one shared evidence entry. That is the
    nature of the set, not a selection failure: the registry already marks
    exactly these four `"mode": "runtime-fixed"` (4 of 52), and they document
    the memory runtime `memory-seed` installs into every target rather than
    the target's code. They stay unconditional, stop being measured by a
    project specificity they cannot have, and are held to a bar they can meet.
    The gate no longer demands target evidence, source paths, or a quoted
    target path from a runtime-fixed body; instead every path it names under
    `memory-bank/`/`project-brain/` and every
    `python3 memory-bank/scripts/*.py` form it names must exist in
    `memory-seed/assets/runtime-contract.json`
    (`RUNTIME_PATH_UNSUPPORTED`, `RUNTIME_PATH_FORBIDDEN`,
    `RUNTIME_COMMAND_UNSUPPORTED`), and a target path it declares no evidence
    for is blocking (`RUNTIME_PROJECT_CLAIM_UNSUPPORTED`). Membership is
    decided on segment runs, so a skill may name `memory-bank/scripts/
    validate.py` or just `scripts/validate.py` and `control/`, but not
    `project-brain/state/mode.json`. The `mode` field already in the registry
    is the trigger; no new field was invented.
  - **Reporting.** "9 skills for your project" overstated what was derived
    from the target. `validate_skill_quality.py` now prints a `skill
    inventory:` line and emits a `skill_classes` member in `--json`, and the
    `infra-generate`/`skill-forge` output templates report two counts:
    5 project skills and 4 runtime guides, never their sum. The decision and
    its rationale are documented in `php-process-skills.md` and in the profile
    schema so the next reader does not mistake the quartet for a failure of
    selection.

- Three ways the skill-quality gate rejected honest generated content, all
  three found by running the pipeline end to end against a real Symfony
  project (UniteCMS) rather than against a fixture, and none of them visible
  to the 324 tests or the 38 fixtures.
  - The polarity window read clause boundaries out of code spans, so `--` in
    a CLI flag closed the clause. A prohibition naming two commands lost its
    force for the second: ``Never run `bin/console doctrine:schema:update
    --force` or `composer install` here.`` reported `composer install` as
    *prescribed* and failed the body as `DEPENDENCY_WRITE` - the guardrail
    blocked the very command it forbids. Span contents are now blanked (with
    offsets preserved) before the boundary and marker scan, because a
    boundary is a property of prose, not of code. Prose punctuation still
    ends a clause, a lone hyphen still does not, and a fenced shell block is
    still an instruction whatever the surrounding prose claims.
  - `SKILL_CIRCULAR_PURPOSE` fired on any skill whose name matches its own
    directory. `memory-bank`, honestly naming `memory-bank/chunks/` and
    `` `memory-bank/scripts/validate.py` `` in its Purpose, was called
    circular by a word-boundary match inside a path. A mention that is part
    of a path, or that sits inside a code span, is naming a file rather than
    restating the skill, and no longer counts; a purpose that restates its
    own name in prose is still circular.
  - `REPEATED_BLOCK` fired on the evidence table. Two skills citing one piece
    of evidence must render that row identically - the citation is the point -
    and header, rule and row form exactly the three-line identical run the
    check reports. A markdown header with its rule, and an evidence row
    (a cell that is only an evidence id, next to a cell carrying a source
    path), now break the run instead of forming a block. Nothing else was
    relaxed: duplicated prose beside a shared table, duplicated ordinary
    table rows, and the byte-identical, noun-substituted and repunctuated
    template corpora are all still reported.

- The generator could not pass its own quality gate on any target. The memory
  quartet is unconditional, its skills verify themselves with the seeded
  runtime, and every one of those commands is an interpreter invocation -
  `python3 memory-bank/scripts/context.py status` - which static analysis can
  never prove non-mutating. The gate blocked all of them as
  `INTERPRETER_EXECUTION`, so a real end-to-end run ended in FAIL after 19
  forge iterations with 7 errors, all in the quartet.
  The route out is attestation, not relaxation: `commands.read_health` in the
  generator's own `runtime-contract.json` now vouches for what the analyzer
  cannot prove. The allowance is deliberately narrow - the list is read from
  the generator's shipped asset and never from the target, so a scanned
  project cannot declare its own commands safe; it is exact-match, not a
  pattern; it covers only the `read_health` group, which the contract already
  separates from `refresh_retrieve`, `checkpoint`, `governed_task` and
  `dynamic_records`; and it waives attestability alone. A proven risk is never
  waived, so `context.py status && <destructive>` stays blocked, as do
  `refresh`, `start` and `complete` on the very same script.
  The contract was also incomplete: `memory-bank/scripts/validate.py` is in
  `required_skeleton` and is read-only (verified against the shipped source -
  no write, mkdir, unlink or commit in 388 lines), but was declared nowhere,
  so the durable validator stayed blocked. It joins `read_health`, and a new
  test asserts every declared read-only script really is read-only rather than
  trusting the declaration.

- `CommandAnalyzer` raised on the stock Symfony skeleton, so the quality gate
  could not run at all on a normal Symfony target. Flex writes `auto-scripts`
  as an object keyed by command (`{"cache:clear": "symfony-cmd"}`), and
  `_normalise_script_commands` accepted only a string or a string list - it
  raised, `validate_skill_quality.py` turned that into
  `COMMAND_ANALYZER_UNAVAILABLE`, and every skill in the inventory failed.
  All seven real Symfony projects available for measurement carry the object
  form, so this was not an edge case: it made generation impossible on the
  generator's own primary target ecosystem. The object form is now read, with
  the command reconstructed from the key according to its handler
  (`symfony-cmd` -> `bin/console <key>`, `php-script` -> `php <key>`), and a
  script object with non-string values is still rejected. Found by running
  the pipeline end to end against a real project rather than a fixture.

- The skill-quality gate held the *shape* of a generated skill - contract
  completeness, candidate registry, sha256 fingerprints, path existence - and
  lexical duplication, but not its *meaning*. Seven reproduced ways to pass it
  with an unusable or unsafe skill are now closed; the entries below describe
  each one. Throughout, calibration was measured against honest corpora rather
  than guessed, because a gate that fails a good-faith generation breaks the
  generator outright, which is worse than the miss it closes.
- Command safety stopped at the plan's `verification[].command`, so a
  generated `SKILL.md` could hand the agent `rm -rf var/cache/dev` or
  `curl -X POST https://billing.internal/api/invoices/void` in its prose and
  still pass. Skill bodies are now analyzed too: fenced `bash`/`sh`/`shell`/
  `console` blocks strictly (transcript prompts stripped, continuations
  joined, comments dropped) and inline backtick spans permissively (a segment
  counts only when it names a known executable *and* carries an argument, and
  a bare English-word executable only when path-qualified). Chains are split
  at quote-aware separators and classified leaf by leaf, homoglyph spaces are
  folded first, and the argument of a code host (`bash -c`, `php -r`) is
  re-read as a nested command. Destructive, workspace-mutating,
  provider/network, or `sudo` behavior blocks (`SKILL_BODY_COMMAND_RISK`);
  attestability-only findings (unknown executable, unresolved alias,
  tokenization) do not, because prose legitimately carries placeholders and
  sample output. That split is the measured one: on 93 hand-written
  accelerator skills the naive "anything not provably safe" policy fired ~100
  times on the 48 Symfony skills alone and was almost entirely noise
  (`<number>` placeholders, heredoc fragments, JSON payload strings, `EOF`),
  while the shipped policy fires 64 times and every hit is a real mutating or
  networked command.
  Two defects in that first cut were found by review and closed before it
  shipped. Prose polarity is now read: a guardrail names the command it
  forbids, so `Never run \`rm -rf var/\`` - the safest sentence a skill can
  carry - was the one sentence that failed the gate. A backtick span whose
  own clause carries a prohibition (`never`, `do not`, `must not`, `avoid`,
  `instead of`, `tempted to`, ...) is read as a mention, not a prescription;
  the window stops at the previous clause boundary so one prohibition cannot
  silence a section, a lone hyphen is deliberately not a boundary (it would
  truncate the window at "read-only" and hand the false positive back), and a
  fenced shell block stays an instruction whatever the surrounding prose
  claims. Second, the segment walk was LIFO against a 256-segment cap, so a
  chain longer than the cap dropped its *head* - exactly where padding hides
  a dangerous command; `rm -rf var/important && <300 harmless echoes>`
  validated clean. The walk is now breadth-first over the written order, and
  reaching the cap is itself reported (`SKILL_BODY_COMMAND_UNSCANNED`)
  instead of silently truncating: an unscanned tail is an unproven command,
  and unproven fails closed.
- `evidence_anchors[].anchor` was validated for shape only, so an anchor could
  cite line 7400 of a 60-line file or a symbol that does not exist in the
  class it names. Anchors now resolve against the cited file - `L` ranges must
  run forwards and lie inside its real bounds (`EVIDENCE_ANCHOR_RANGE`), and a
  `symbol:` anchor's last segment must occur in it
  (`EVIDENCE_ANCHOR_SYMBOL_ABSENT`, case-insensitive whole-identifier match so
  `Ns\Class::member` resolves through `member`) - the same bound
  `evidence[].line_range` already carried. An unreadable source is reported
  (`EVIDENCE_ANCHOR_UNRESOLVABLE`), never raised.
- Ownership was only ever compared writes-against-writes, so a skill could
  write into a zone another skill holds under `mode: exclusive` whenever that
  owner was read-only - which, by contract, every reviewer is
  (`OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT`). `shared`/`composed` zones keep their
  own rules. Separately, a declared primary/defer precedence silenced overlap
  unconditionally, so two skills could claim the same `owned_scope` entry,
  `ownership[].description`, or positive trigger *word for word* and still
  pass. Verbatim repetition is now `BOUNDARY_NOT_SEPARATING` regardless of
  precedence - a boundary that repeats itself divides nothing - while nested
  or otherwise partial overlap under an explicit precedence stays legitimate.
- A skill could carry the project's real paths, classes and constants in every
  sentence and still instruct nothing, passing every lexical traceability
  check. Procedure steps are now read structurally: a step must command an
  action (`SKILL_STEP_NOT_OPERATIONAL`) and must not open by deferring to
  reflection - "consider", "bear in mind", "form an opinion"
  (`SKILL_STEP_HEDGED`); the procedure as a whole must name a concrete anchor
  (`SKILL_PROCEDURE_UNANCHORED`); and verification must state a check rather
  than an impression (`SKILL_VERIFICATION_NOT_FALSIFIABLE`,
  `SKILL_VERIFICATION_NOT_OPERATIONAL`). The plan is read the same way
  (`PROCEDURE_STEP_NOT_OPERATIONAL`, `PROCEDURE_STEP_UNANCHORED`,
  `VERIFICATION_NOT_FALSIFIABLE`). The `GENERIC_PHRASES` blacklist is demoted
  to a secondary net, since one paraphrase defeats it. Calibration is
  deliberately loose where it can only cost honest skills: the verb set is
  open and matched anywhere in the step, mid-sentence hedging next to a real
  action stays legitimate, and the anchor is required per procedure and never
  per step, so honest branch and delegation steps survive.
- The action-verb set that gate reads against was too narrow on arrival and
  rejected honest instruction: 5 of the 37 procedure steps in the adversarial
  harness's *honest* five-skill baseline were failed as commanding no action
  ("Bound delivery at three attempts.", "Downgrade only on a permanent
  delivery boundary error.", "Not this skill: recompute the anniversary
  anchor of a subscription cycle."), and a step reading "Accept transitions
  that revive a cancelled slot" was reported as inert rather than as wrong.
  The set is widened by two stated rules - a verb enters when a measured
  honest step used it, and a verb whose counterpart is already listed
  ("upgrade"/"downgrade", "compute"/"recompute", "reject"/"accept",
  "stop"/"start", "bind"/"bound", "modify"/"change") enters with it - stopping
  at words that ordinarily read as adjective or noun, so "close" stays out and
  "the closed invoice" keeps reading as a bare noun list. The honest baseline
  is clean again at 0 findings over 271 distinct steps of every corpus
  available.
- Skill deduplication in `validate_skill_quality.py` was advertised as
  semantic but compared normalized lines for *equality*, so it only ever saw
  byte-identical prose - and since a plan-conforming `SKILL.md` must carry its
  own claim, paths and neighbours' names, those mandated differences diluted
  the score below the fail lines even for a byte clone of the procedure
  (measured on the fixture: clone line 0.500 / token 0.680 against thresholds
  0.70 / 0.80; one template with the project's nouns substituted 0.100 /
  0.351; the same template differing only in `,`->`;` and `and`->`plus`
  0.000 / 0.324 - indistinguishable from two honestly different skills).
  A second pass now compares a *skeleton*: backticked spans, paths, CamelCase
  and `UPPER_SNAKE` identifiers, dotted ids, PHP variables and calls, numbers
  and the inventory's other skill names collapse to one placeholder, fenced
  code and approved fixed blocks are dropped, punctuation and interchangeable
  connectives are folded, and lines that are almost entirely project identity
  are ignored; what remains is scored by one-to-one line matching plus token
  trigrams (`SKILL_TEMPLATE_REUSE`, error at 0.38 / 0.26, warning at
  0.28 / 0.20). Thresholds are measured, not guessed: across 2371 pairs of
  honest hand-written skills the worst pair scores 0.350 / 0.154 and is a
  deliberately parallel scanner family, while the three duplicates above
  score 0.400 / 0.329, 0.400 / 0.311 and 0.500 / 0.471. `REPEATED_BLOCK`'s
  three-consecutive-identical-lines rule drops to two adjacent skeleton lines
  as `SKILL_TEMPLATE_BLOCK`, kept a *warning* because on the same honest
  corpus it fires five times on legitimately shared policy sentences.
- An `evidence[].supported_claims` entry was "grounded" by a bag-of-words
  overlap with the cited range: two shared tokens (one for a claim of three
  tokens or fewer) after subtracting five service words. `class` occurs in
  75% of real PHP files, `function` in 77%, `public` in 74%, `string` in 56%,
  so an invented claim - "the public class exposes a private function that
  returns a string value from the configuration array" - was grounded by any
  PHP file it pointed at. Support now has to come from vocabulary that
  distinguishes *that* range: a claim sharing nothing outside PHP-keyword and
  licence-header lexicon is `EVIDENCE_CLAIM_UNSUPPORTED` (error), and one
  sharing nothing outside software-English boilerplate (`service`, `method`,
  `value`, `result`, `config`) while at least 65% of its own vocabulary is
  such boilerplate is `EVIDENCE_CLAIM_GENERIC_SUPPORT` (warning). The cited
  side is matched with compound identifiers split into their parts, so
  `publishReminder` grounds an honest claim about the "reminder" the file
  never writes on its own. Both lexicons are unedited document-frequency cuts
  (share >= 0.40 and >= 0.04) over 3997 real PHP files from the Symfony and
  Laravel vendor trees, measured in that same identifier-split token space.
  Calibration on 1485-2190 honest docblock/code pairs from those trees: the
  error tier costs 0.18-0.27% false positives and catches 20-45% of
  generic-lexicon fabrications, the warning tier costs 2.3-3.6% and takes the
  pair to 87-99%; on the harness's honest control claims the margin is 3-10
  project-specific tokens matched against a threshold of one. The second tier
  warns rather than blocking because at that false-positive rate a failed
  generation would cost more than the miss.
  What no lexical rule can reach is *polarity*: a claim asserting the opposite
  of the code it cites shares all the same vocabulary. That gap, the options
  weighed for it (an LLM judge inside the gate, doing nothing, this
  deterministic narrowing plus adjudication in the repo-root `harness/`), and
  the decision are recorded in `docs/ADR-001-claim-adjudication.md`.
- Six contradictions between the canonical LLM-prompt documents and the
  shipped validators, each capable of steering an obedient agent into a
  blocking gate or leaking non-neutral fixture data:
  - `infra-generate` step 2 demanded a top-level `routing_cases[]` in
    `skill-generation-plan.json`, but the schema's top-level membership is
    exact and `validate_skill_quality.py` blocks any extra field with
    `PLAN_FIELD_UNKNOWN`; the step now requires the per-skill
    `routing_cases[]` (`skills[].routing_cases`) the validator actually
    checks.
  - `command-forge` step 8 read as an in-skill instruction to run
    `validate_flow_contracts.py`, yet `skill-flow-composer` runs after
    command-forge, so `SKILL FLOW.md` cannot exist and the validator exits 1
    on the missing artifact; the step now states compiled-graph validation
    is orchestrator-owned (`infra-generate` step 8 after the composer), must
    not run inside command-forge, and is recorded as pending that gate.
  - The `command-forge` frontmatter guardrail ("Cursor command only `name`,
    `description`") carried no flow-command exception while step 7 and the
    flow guardrail require `flow` + ordered `stages` frontmatter and one
    fenced `json flow-contract` block in every selected command-carrying
    edition; the guardrail now names flow commands as the sole exception, so
    a literal reading no longer guarantees failing
    `validate_flow_contracts.py`.
  - The `project-profile-schema.md` exemplar skill stamped
    `"phase": "execution"`, outside the fixed vocabulary (`understanding`,
    `planning`, `implementation`, `verification`, `finalization`) that flow
    stages and the composer's Phase Map accept; the example now uses
    `implementation`.
  - Adjacent `AGENTS.md` bullets contradicted each other on schema 1.1
    approvability; both now state the policy DOD.md, the schema doc, and
    `LEGACY_PLAN_PUBLICATION_INELIGIBLE` enforce: only a schema 1.2 plan is
    approvable (1.2 carries the ownership/routing structures introduced in
    1.1), and legacy 1.0/1.1 plans stay audit-readable but must be
    re-synthesized.
  - The `critical_invariants` example in `project-profile-schema.md` leaked
    a real project's domain (`content-job.failure-terminal` /
    `contentjobs-lifecycle-review`); it now uses the neutral acme-billing
    fixture family (`invoice.paid-immutable`, `billing-rules-review`) like
    every other example, preserving the example's structure.

  Pinned by the new `tests/test_doc_contracts.py`, which parses the
  documents and cross-checks them against the validators' actual field
  sets, phase vocabulary, and diagnostics instead of trusting prose.

- `analyze_commands.py` no longer fails open: unknown executables, `sudo`/
  `env`/`timeout`/`nice`/`nohup`/`stdbuf` wrappers (unwrapped by basename),
  `xargs` and `$VAR` indirection, clustered `-lc` interpreter flags, shell
  invocations of script files, git global options before the subcommand,
  `php bin/console` / `symfony console` database commands, and
  `npm i`/`ci`/`npx`/`dlx`-family runners all classify or block instead of
  reporting `verification_safe=true`. Blocked commands now carry an explicit
  `verification_blocker` category instead of `non_mutating`. False positives
  fixed (`ruff check .`, `gofmt -l .`, `pytest -W error`); alias expansion
  is bounded (512 walks, fail-closed `EXPANSION_LIMIT`); unknown bare
  `yarn`/`pnpm`/`bun` scripts fail closed as `UNKNOWN_ALIAS`; direct
  `composer <builtin>` classifies the builtin, not a shadowing script.
- `publish_staging.py` refuses to write outside its contract: publication
  plans must be members of the staged manifest, removal plans members of
  the target manifest, and non-ownable runtime state (memory-bank/
  project-brain) is rejected before any mutation. Rollback now removes
  directory chains publication created and, using post-publish content
  hashes recorded in the journal, skips (and reports) files edited by third
  parties instead of silently clobbering them. `classify_update` no longer
  reports the staged `.infra-manifest.json` as a `new-file-collision`, and
  malformed manifest/decision entries raise clean `OwnershipError`s.
- `merge_gitignore.py` emits positive patterns before `!` negations so
  re-includes survive git's last-match-wins, and a requirement that would
  silently override a team `!entry` (or vice versa) is now a reported
  conflict requiring an explicit decision.
- `validate_skill_quality.py` diagnoses wrong-typed plan fields instead of
  crashing (five reproduced crash sites; `--json` always emits its
  payload), accepts the shipped five-key `candidate-registry.json`
  (previously every default-registry `--skill-plan` run failed
  `REGISTRY_INVALID`), no longer corrupts similarity metrics via the
  `profile` normalization regex, and flags per-trigger routing collisions
  that whole-set Jaccard diluted below threshold. Also fixed: unescaped
  skill names in regexes, `_globs_intersect` false positives on disjoint
  patterns, glob matches escaping the target through symlinks, the dead
  pre-`resolve()` symlink guard, trailing sentence punctuation in
  traceability tokens, and YAML folded/literal description parsing.
- `validate_flow_contracts.py` reports structured errors instead of
  raising on malformed routing fields and null stage `agents`, and command
  file-set discovery follows declared flow names instead of a hardcoded
  `flow-*.md` glob. `validate_generated.py` reports one placeholder error
  per file/pattern. `validate_reference_catalogs.py` rejects empty
  candidate registries and reports unreadable catalogs per file instead of
  crashing.
- Memory readiness in the seeded `context.py` distinguishes transient git
  probe failures (`git-probe-failed`) from detached HEAD and makes
  `unborn-head` genuinely detectable; the workflow-smoke repeatability hash
  now covers recomputed outputs instead of constants.
- CI: the reliability job is pinned to Python 3.9 (its 3.x leg duplicated
  the tests-matrix and mirrors jobs), and `build_mirrors.py --check` now
  detects stale mirrors of deleted canonical `only`-class files (pinned by
  the new root `tests/test_build_mirrors.py`).
- Claim grounding (`EVIDENCE_CLAIM_UNSUPPORTED`) blocked three *true* claims
  on the Symfony/UniteCMS run, each on the very citation the scanner contracts
  prescribe, and the rule was documented nowhere.
  - **PHP member separators were not compound joints.** `_tokens` keeps
    `FrameworkBundle::class` whole, so the cited vocabulary offered
    `framework` and `bundle` but never `frameworkbundle`, and prose naming
    the bundle scored zero overlap against the one line that registers it.
    `::` and `->` now split like `.` and `/`, and all three grading tiers
    read the same identifier-expanded vocabulary - the first tier had been
    graded on unexpanded tokens, making it stricter than the tiers meant to
    be strict.
  - **The bar could exceed what the source has to give.** Two overlapping
    words were demanded of any claim longer than three tokens, so
    `.php-version` - whose entire content is `8.2`, and which the stack
    scanner is contractually told to cite as the runtime pin - admitted no
    statable claim at all, and an eight-line `flysystem.yaml` could not
    afford a second reusable word. The requirement is now one reused word per
    sentence-worth of vocabulary the cited range actually offers, floored at
    one and never zero. Fabrication defence is unchanged: the path must
    resolve and the fingerprint must match the bytes on disk, the floor still
    forces the claim to name something the range contains, and the two
    distinctive-lexicon tiers still reject a claim grounded only in PHP or
    software-English boilerplate.
  - **The rule was untaught.** `scan-evidence-contract.md` now states it in
    the `supported_claims` bullet, with one good and one bad claim over the
    same cited file.

- **A generated skill could send the agent to a file that does not exist.**
  Found on the same Symfony/UniteCMS run: a body line reading "Open
  `src/Security/ImaginaryFirewallResolver.php` and confirm the resolver
  rejects the anonymous branch" passed with 0 errors, and so did an evidence
  row whose anchor was re-pointed from `config/packages/security.yaml` to
  `config/packages/totally-made-up-security.yaml`. The plan's
  `path_contracts` were resolved against the target all along; the *rendered
  prose* never was, and the table nobody compared with the plan is the
  skill's citation of record. Two readings were added. A bare path code span
  is now resolved against the target when it is rooted in the target's own
  tree - its first segment and its parent directory both exist - and an
  unresolvable one is `SKILL_BODY_PATH_MISSING`. A rendered evidence row must
  agree with the plan on both identifier and path
  (`SKILL_EVIDENCE_ROW_UNDECLARED`, `SKILL_EVIDENCE_ROW_ANCHOR`); `:L…` and
  `:symbol:…` anchors are reduced to their path first.
  - **Calibration is the whole difficulty, so every ambiguous class is
    skipped rather than guessed:** backslashed class names and namespaces
    (`App\Entity\Page`), dotted config keys and member references
    (`retry_strategy.max_retries`, `article.content`), constants, bare file
    names used as shorthand (`security.yaml`), globs and placeholder groups
    (`src/**/*.php`, `config/packages/{dev,prod}/doctrine.yaml`,
    `src/Entity/<Name>.php`), installed or generated trees (`vendor`,
    `node_modules`, `var`), the generated runtime roots, foreign-framework
    layouts and Twig logical names (`page/show.html.twig`) - both rejected by
    the rooted-in-the-target test - paths the skill declares in `writes` or
    classifies as creatable, a path on a line that commands its creation
    (inflections enumerated, so `authorization`, `additional` and
    `placeholder` are not creation), and any string the skill's own cited
    source spells out, which is how a real `unite.yaml` may declare
    `src/Model` for a directory that was never created.
  - **The deliberate miss:** an invented path below a directory the target
    does not have (`src/Security/Firewall/Resolver/Imaginary.php`) is
    indistinguishable from a layout this project simply does not use, so it
    is not reported. A gate that fails honest instruction is worse than the
    miss it closes.
  - Measured on the real run: 98 path spans across nine generated skills, 0
    false positives, both injected defects reported, control gate still PASS.

### Added

- `validate_skill_quality.py --baseline-plan <previous run's plan>`: an
  optional coverage baseline, because coverage silently shrank between two
  runs of the same pipeline over the same real Symfony/UniteCMS project and
  nothing said so. Run 1 shipped `security-review` carrying a live finding -
  `GET /preview` deserializing a query parameter straight into an entity.
  Run 2 re-composed the inventory, shipped no `security-review`, and moved
  those findings nowhere (`deserialize` and `|raw` appear in 0 of its 11
  skills). Both runs passed the gate: it knew nothing of any earlier run.
  With the flag, the gate reads both plans as data - no execution, the static
  offline invariant is untouched - and compares two sets: skill names, and
  the union of `evidence[].path` with every `skills[].source_paths` entry (the
  target files a run claims to have read). Everything lost is NAMED: one
  `BASELINE_SKILL_DROPPED` per absent skill, carrying the paths that skill
  cited, one `BASELINE_COVERAGE_DROPPED` per uncovered path, and a `coverage
  baseline:` summary line repeating the names (`coverage_baseline` in
  `--json`). A bare count would have hidden the one entry that mattered.
  Severity is `warning`, deliberately, and both directions were weighed:
  re-composition on regeneration is often legitimate (the target changed, a
  skill merged into a sibling), so a hard error would block honest work and
  train people to drop the flag - while silence is precisely what let the
  vulnerability slip. Naming without failing keeps the loss unmissable and the
  flag usable. An unreadable, missing, or malformed baseline is a diagnostic,
  never a traceback: `BASELINE_PLAN_UNREADABLE` plus `coverage baseline: NOT
  COMPARED (<reason>)`, so a mistyped path can never read as a clean bill of
  health. Without the flag nothing changes - no diagnostics, no summary line,
  no `coverage_baseline` key, same exit code.
  Measured on the two real runs: run 2 against run 1 names `security-review`
  and 11 lost evidence paths (including `templates/article/show.html.twig` and
  `src/Controller/ArticleController.php`); run 2 against itself reports no
  coverage lost; run 1 unchanged without the flag (PASS, 0 warnings).

### Verified

- The tightened skill-quality gate was re-measured on a domain it has never
  seen (warehouse logistics: carrier label transport and wave cutoff review),
  authored independently of the shipped fixtures. An honest two-skill
  accelerator whose guardrails *enumerate* ten forbidden commands - two in a
  single sentence, four carrying `--force`, seven of which the gate's own
  analyzer classifies as destructive or mutating - passes `--plan-only`,
  `--allow-partial-skills` and the full gate with 0 errors and 0 warnings.
  Removing only the prohibition word from those same sentences makes the gate
  report them (`SKILL_BODY_COMMAND_RISK`), so the pass is a polarity reading,
  not a skipped scan. Grip was re-confirmed for a prescribed destructive
  command (`SKILL_BODY_COMMAND_RISK`), a noun-substituted and a
  punctuation-only duplicated template (`SKILL_TEMPLATE_REUSE`,
  `REPEATED_BLOCK`), a circular purpose (`SKILL_CIRCULAR_PURPOSE`) and a
  vacuous procedure written in project vocabulary
  (`SKILL_STEP_NOT_OPERATIONAL`); and for the runtime-fixed quartet, which
  passes with no project evidence at all but is blocked when it names a
  runtime path or command absent from `runtime-contract.json`
  (`RUNTIME_PATH_UNSUPPORTED`, `RUNTIME_COMMAND_UNSUPPORTED`) or a target file
  it declares no evidence for (`RUNTIME_PROJECT_CLAIM_UNSUPPORTED`). No code
  changed; two coverage limits were recorded rather than closed:
  `docker compose down -v` and a project-specific `bin/console` command with
  `--force` are not classified as destructive by `analyze_commands.py`, and a
  bare noun-list procedure step can still read as operational when a path
  segment or a noun coincides with a listed action verb ("dispatch", "states",
  "pick", "forward").

- The two gate additions above were integrated and re-measured together, not
  only apart. Canon mirrors were rebuilt (`build_mirrors.py --write --edition
  Infrastructure-Creator`, 4 files) and `--check` passes for all four
  editions. The generator suite is 449 tests green: 417 before this round,
  +16 for the search-exclusivity engine, +16 for the coverage baseline. On the
  two real pipeline runs over the same Symfony/UniteCMS target the verdicts
  are exactly the ones the two changes predict and no others: run 1 exits 0 at
  `PASS (0 errors, 0 warnings)`, run 2 exits 1 at `FAIL (1 errors, 0
  warnings)` carrying only `VERIFICATION_SEARCH_EXCLUSIVITY` on
  `content-publication-review.assert-transition-applied`
  (`src/Entity/Article.php`, `src/Entity/Job.php`). The coverage baseline stays
  silent without `--baseline-plan`, so neither control run changed shape. The
  901 B of new documentation carried `Infrastructure-Creator.body_bytes` 577 B
  past its ceiling; the ceiling was refit per the policy in
  `scripts/token_budget.json` rather than the documentation cut, and the
  reasoning is recorded in the root changelog.

## [2.5.0] - 2026-08-14

### Added

- Machine-readable `skill-generation-plan.json` with a validated evidence
  ledger and one complete necessity, scope, procedure, verification, output,
  failure, write, and routing contract per proposed skill.
- Dependency-free semantic gates for plan/evidence integrity, fingerprints,
  contract-to-skill traceability, target-relative citations, ownership
  collisions, routing ambiguity, substantive duplication, approved versioned
  fixed blocks, agent routing, and conditional specialist flows.
- Transactional staged publication with explicit publication/removal plans,
  target-drift detection, manifest-last writes, team-file preservation, and
  rollback for generation and updates.
- Reference-catalog validation and regression fixtures for generic/duplicated
  skills, paraphrased filler, invalid or irrelevant evidence, stale
  fingerprints, scope/routing collisions, reference stubs, merge preservation,
  and failed-publication rollback.
- Early complete-inventory validation, safe partial skill-batch validation, and
  schema 1.1 structured ownership/reciprocal routing with schema 1.0 migration.
- Deterministic shared root `.gitignore` requirement composition, additive
  decision metadata, and watch-only target drift protection.
- Exact immutable-memory placeholder declarations and deterministic workflow,
  parity, shared-file, publication, and rollback regression coverage.
- Publication-gated schema 1.2 operational contracts for typed procedures,
  concrete verification, provider safety, path authority, critical invariants,
  evidence anchors, realistic routing cases, and canonical flow graphs.
- Dependency-free command risk analysis for Composer/npm aliases and explicit
  automatic-memory readiness reporting.

### Changed

- Skill inventory selection is evidence-gated instead of quota-driven. Only the
  memory quartet remains unconditional because its runtime is always installed.
- `skill-forge` authors one skill or a small bounded sibling group at a time
  from the minimum evidence slice; line padding and grouped category generation
  are invalid.
- Agents derive positive/negative selection, sibling deferrals, expected
  results, and write capability from validated contracts. `flow-feature` and
  `flow-review` route specialists conditionally rather than selecting every
  available reviewer.
- `infra-generate` and `infra-update` verify the complete staged bundle before
  publication and run the full gate again on the published target.
- Forges declare root-ignore requirements instead of mutating `.gitignore`;
  generation/update orchestration is the sole composer and requires explicit
  approval when a pre-existing team file needs appended entries.
- Legacy schema 1.0/1.1 plans remain audit-readable but require re-synthesis
  before generation or publication; safety-critical fields are never guessed.
- Skills, wrappers, `SKILL FLOW.md`, and executable flows compile from one
  validated routing/flow contract and must preserve every material adjacency.
- `stack-adapter` must carry the complete contract compiler, semantic/reference
  validators, staging/rollback helper, fixtures, and synthetic rehearsal into
  every future sibling generator.

### Breaking

- Profiles created before 2.5.0 do not contain per-skill generation contracts.
  Re-run `infra-scan` before generation/update instead of reconstructing
  authority from previously generated prose.

### Orchestration and context changes

- **Generated accelerators reach orchestration parity with the hand-built
  editions.** The seeded context-brain runtime was a stale fork — no agent
  channel at all, no `hook-context`/`rebind`/`reindex-bank`, and a
  four-value phase vocabulary the engine stopped using — so
  `memory-seed`'s bundled `scripts/` and `project-brain/` skeleton are
  re-synced from the canonical runtime, gaining `msg-send` / `msg-read` /
  `msg-dispatch`, `capsule --validate`, `update --actor`, the forward-only
  phase guard, `message.schema.json`, and `control/messages/`. On top of
  that: `agent-forge` emits `writes: true` for write-capable agents in both
  editions (without it the generated gate's serialization never engages),
  `command-forge` generates `flow-feature` and `flow-review` composed only
  of agents the run actually produced, `hook-forge` adds
  `subagent-dispatch.sh` with its `SubagentStop`/`subagentStop` wiring
  (shipped unregistered on Codex) plus the gate's write serialization,
  `policy-forge` writes an Orchestration section into the generated
  AGENTS.md, and `bootstrap-verifier` / `validate_generated.py` enforce the
  eight-hook contract and the flow contract — stages naming generated
  agents, at most one write-capable agent per parallel stage, a checkpoint
  in every multi-stage flow.

- **Generated accelerators now restrict subagents to their own roster.**
  `hook-forge` produces a seventh hook, `subagent-gate.sh` - three
  tool-owned variants (Claude PreToolUse exit codes against the generated
  `.claude/agents` roster, Cursor `subagentStart` permission JSON with
  `failClosed`, Codex spawn_agent-family deny) - and wires the
  configuration half per edition: `Agent(...)` deny rules plus the
  built-in-agent env keys in `.claude/settings.json`, `subagentStart` in
  `.cursor/hooks.json`, `multi_agent = false` and `[agents] enabled =
  false` in `.codex/config.toml`. `policy-forge` adds a Subagents section
  to the generated `AGENTS.md`; `bootstrap-verifier` and
  `validate_generated.py` enforce the seven-hook contract (the gate is
  exempt from byte-identity by design).

- `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH=1` joins `.claude/settings.json`:
  the generator's flows orchestrate from the main conversation, so nested
  subagent trees add cost without oversight. The tool-owned
  `subagent-gate.sh` copies are refreshed to the monorepo's Stage C
  versions; the new write-serialization logic is dormant here until an
  agent declares `writes: true`.

### Additional additions included in this release

- Agent `<example>` blocks moved out of `description:` frontmatter into a
  `## Selection examples` body section, and `agent-forge` now requires the
  same of every accelerator it generates. A description is loaded into the
  orchestrator's context on every session whether or not the agent is spawned,
  and the embedded examples were about two thirds of those bytes; the forge
  previously prescribed embedding them, so each generated accelerator
  inherited the cost. The generator's own 23 agents shed 15,866 bytes of
  description. Nothing is lost - the blocks move verbatim into the body.

- **The generator's own tool configs now restrict subagent spawning to the
  project roster.** A tool-owned `subagent-gate.sh` joins each hooks
  directory — the one deliberate exception to the byte-identical hooks
  invariant, skipped in `mirror_rules.py`, because each host exposes a
  different gate contract (Claude PreToolUse exit codes, Cursor
  `subagentStart` permission JSON, Codex `spawn_agent` deny). Configuration
  backs the hooks: `Agent(...)` deny rules and
  `CLAUDE_CODE_DISABLE_EXPLORE_PLAN_AGENTS=1` in `.claude/settings.json`,
  `[features] multi_agent = false` plus `[agents] enabled = false` in
  `.codex/config.toml`, a `subagentStart` entry in `.cursor/hooks.json`, the
  `subagent-policy.mdc` Cursor rule, and an AGENTS.md "Subagents" section.
  Covered by `SubagentGateTest` in `tests/test_hooks.py`.

## [2.0.0] - 2026-08-07

### Changed

- **Manifest membership is now the only ownership authority.** Full and merge
  generation use an explicit write plan instead of walking managed target
  roots; `bootstrap-verifier` validates and scans placeholders only in tracked
  generated files, leaving unmanifested team policies, skills, hooks, memory,
  and Project Brain files untouched. The dependency-free ownership helper also
  provides deterministic update classification, legacy-manifest refusal, and
  caller-relative/absolute path handling, with standard-library regression
  coverage including a generated ownership lifecycle fixture.
- **Hooks joined the hardened generation.** `bash-validator` extracts the
  command through the jq/php/python3 chain instead of a greedy `sed` (a
  payload that fails to parse is no longer matched raw, which ends false
  blocks on prose mentioning dangerous commands) and warns on stderr when no
  extractor is available; `loop-detection` and `local-context` namespace
  their counters by a repository hash and reset them on SessionStart; the
  `.codex` hooks exit early on unrelated tool names, so validators no longer
  run - or count edits - on read-only tool calls. The hook layer gained its
  first regression suite (`tests/test_hooks.py`).
- **Canonical hook input no longer depends on `cat`.** Bash builtins consume
  complete multiline payloads before extractor selection; an extractor-less
  environment produces exactly one sanitized fail-open warning, and blocked
  diagnostics never disclose the command body.
- **Mirror rules became declarative.** The generator's own
  `.claude`/`.cursor`/`.codex` mirrors are regenerated by the monorepo's
  `scripts/build_mirrors.py` from the rule table in `mirror_rules.py`
  instead of being copied by hand.
- **Generated Cursor targets receive the Task Capsule.** `hook-forge` wires
  the same `alwaysApply` rule delivery (`.cursor/rules/working-memory.mdc`,
  gitignored in the target) that the flagship editions use, so Cursor
  installations of generated accelerators read working memory one turn
  behind instead of not at all.

## [1.4.0] - 2026-08-02

### Added

- **The generator's output is now upgradeable in place.** The new root `VERSION`
  file is the single source of the generator's version. `infra-generate` now
  finishes by stamping the first line of the target's `AGENTS.md`
  (`<!-- Generated by Infrastructure-Creator vX.Y.Z | TASK-N | date -->`) and
  writing `.infra-manifest.json` at the target root: generator version, source
  profile/task, selected editions, and the sha256 of every generator-owned
  file (runtime state - memory chunks, `INDEX.md`, counters, `local/` dirs,
  Project Brain records and indexes - is deliberately untracked; it belongs to
  the target team from the moment it is seeded). A new 23rd skill,
  `infra-update` (+ `/infra-update` command + agent, the fifth sanctioned
  orchestrator), consumes that manifest: it re-validates the profile,
  regenerates into a staging directory inside this workspace, and then applies
  a strict three-set triage - files whose hash still matches the manifest are
  safely replaced with their new versions; files the team edited (or deleted)
  are never overwritten and land in a "requires decision" report with
  three-way context (as generated / as the current generator would write it /
  as it is in the target); files with no manifest record are never touched.
  Every update run ends by rewriting the manifest and re-running
  `bootstrap-verifier`. Targets generated before manifests existed (v1.3.x
  and earlier) make `infra-update` abort with legacy-target recovery options
  instead of guessing file ownership. `bootstrap-verifier` /
  `validate_generated.py` now verify the contract itself: manifest presence,
  schema, self-exclusion, no state tracking, every tracked file existing with
  a matching hash, full coverage of the generated surface, the `AGENTS.md`
  stamp agreeing with the manifest, equality with the generator's `VERSION`
  right after a run, and a hash refresh after any content auto-fix.
- **Generated accelerators now receive the full context-brain layer**, closing
  the gap to the monorepo's hand-built PHP editions:
  - `memory-seed` bundles and copies verbatim the dependency-free runtime
    (`context.py`, `brain_runtime.py`, `context_retrieval.py`, `validate.py`
    into `memory-bank/scripts/`) and the governed `project-brain/` skeleton
    (`PROTOCOL.md`, `README.md`, `.gitignore`, five schemas, nine record/control
    templates, `scripts/validate.py`, empty indexes, config, and `.gitkeep`-held
    record directories). `config/runtime.json` is materialized from
    `runtime.json.template` with a single `{{TARGET_FRAMEWORK}}` substitution -
    the target's confirmed framework slug, or `generic` when none is confirmed.
    Unknown slugs are safe by design: the runtime's only framework-keyed lookup
    (`MIRROR_RULES.skip_by_framework`) yields no exemptions for unknown keys,
    and `brain_runtime.py` defaults to `generic`.
  - `hook-forge` now specifies six hooks instead of four: the two working-memory
    hooks (`working-memory-read.sh` on `UserPromptSubmit`, calling
    `context.py refresh` with a bounded ephemeral query; `working-memory-write.sh`
    on `Stop`, calling `context.py turn --flush-after`) join the four enforcement
    hooks, with wiring for Claude (`settings.json`, second timeouts), Cursor
    (`hooks.json` `stop` only - Cursor has no prompt-time event, a documented
    divergence), and Codex (`hooks.json` with `additionalContextLimit`, plus
    `config.toml`). Loop-detection tracking is namespaced per repository by a
    hash of the repo root.
  - `skill-forge` generates 18 process/workflow skills instead of 15: the new
    `project-brain`, `checkpoint`, and `memory` cards complete the memory
    quartet around `memory-bank`, authored against the shipped runtime per the
    new "Memory Quartet" contract in `references/php-process-skills.md`.
  - `bootstrap-verifier`/`validate_generated.py` gained delivery smoke checks:
    `context.py status` and `context.py validate` must exit 0 in the generated
    tree; the per-edition hook set must be complete; every hook wiring file may
    reference only existing, executable scripts (closing the "dead hooks" bug
    class); the `project-brain/` skeleton and substituted `runtime.json` are
    verified; and the memory quartet skills (plus wrappers) are required per
    edition. The seeded bank's validator is now invoked with its positional
    bank argument, matching the shipped validator's CLI.
  - The Project Profile schema's sections 11.1/11.2/12, consumption contract,
    and validation rules now describe the honest delivery: 18 process skills,
    the two-root memory layer, and the framework-slug substitution.

### Fixed

- **The generator's own `bash-validator.sh` allowed irreversible GitHub
  operations.** It blocked broad `rm -rf`, force-push, hard reset, `--no-verify`,
  `.env` access, and SQL `DROP`, but not `gh repo delete`, `gh repo archive`,
  `gh issue delete`, `gh release delete`, or `gh api ... DELETE` - all of which
  the three PHP editions block. This generator only ever reads a target project,
  so none of them has a legitimate use here. Added them as `case` rules,
  matching the script's existing style; its shell-glob matching never had the
  `grep --` defect the PHP editions did, so the rest is unchanged.
- **`hook-forge` specified broken Claude wiring, so every generated accelerator
  inherited dead hooks.** Step 6 told the generator to emit
  `echo '$TOOL_INPUT' | <script>` with millisecond timeouts, and assigned
  loop-detection to `PreToolUse` and file-naming-validator to `PostToolUse` -
  the reverse of this generator's own `.claude/settings.json`. Claude Code
  delivers the payload on stdin, so the `echo` form passes the literal string
  `$TOOL_INPUT` and the hook exits 0 on every call; `bootstrap-verifier` only
  checks `bash -n` and the executable bit, so it reported success anyway.
  Corrected the wiring spec (bare script path, second timeouts, correct
  event/matcher pairs) and added guardrails requiring `grep -Eqi --` and JSON
  decoding rather than `sed` scraping in every generated `bash-validator.sh`.
- **This generator's own hooks were wired the same broken way.** Applied the
  same fix to `.claude/settings.json`: bare script paths, timeouts in seconds.
  Its `bash-validator.sh` matches with shell `case` globs rather than `grep`,
  so the script itself was unaffected and is unchanged.

## [1.3.5] - 2026-07-27

### Added

- New Claude `domain-behavior-scanner` skill and agent, expanding Phase 1 from six to seven scanners. It discovers project-specific sources of truth, domain vocabulary, core entities, business invariants, proven lifecycle transitions, roles/permissions, audit obligations, high-risk workflows, critical regression scenarios, sanitized incident lessons, and cohesive domain-skill candidates.
- Behavioral findings preserve both confidence and source type and surface contradictions; statuses are not treated as transitions, observed enforcement is not treated as a complete permission matrix, and risk indicators do not invent severity/owners/approval.
- Project Profile schema now includes section 8 (Domain & Behavioral Contract), section 11 generation preview, and section 12 cohesive memory preview.
- New `php-domain-behavior.md` generation reference. Existing requirements/architecture/API/database/testing/review/security/debugging/documentation skills are enriched first; separate domain skills are generated only for cohesive confirmed bounded contexts.
- The operational `memory-bank` skill is now always generated and counted alongside the shared bank created by `memory-seed`.
- Memory seeding now groups confirmed facts into cohesive concepts and links canonical sources instead of producing one small chunk per fact or duplicating specs/schemas/tests.

### Changed

- Mirrored the complete 22-skill implementation to Cursor (`.cursor/skills`) and Codex (`.agents/skills`), including the domain scanner, profile schema, generation references, memory behavior, and verifier updates.
- Added the matching reduced-frontmatter Cursor `domain-behavior-scanner` agent and updated Cursor orchestration commands/rules. Codex continues to invoke skills directly without agents or commands.
- `bootstrap-verifier` now enforces AI-tool selection directly in its Python validator: every selected edition root must exist, while any unselected `.claude`, `.cursor`, `.agents`, or `.codex` root causes generation verification to fail.

## [1.3.0] - 2026-07-21

### Added

- `skill-forge` now generates a much broader, still 100% evidence-driven skill set for the target, in six groups instead of three: **architecture** (1, unchanged), **design & interaction** (3, always: `architecture-implementer`, `api-designer`, `database-designer`), **frontend** (0 or 5, conditional on a detected rendering/asset layer: `frontend-design`, `coder-frontend`, `wcag-accessibility`, `web-design-guidelines`, `browser-verify`), **process & workflow** (14, always, framework-agnostic: `requirements-analyst`, `researcher`, `brainstorming`, `council`, `writing-plans`, `using-git-worktrees`, `systematic-debugger`, `refactorer`, `dependency-manager`, `review-pr`, `finishing-branch`, `documentation-generator`, `skill-creator`, `reflect`), **universal PHP** (7, unchanged), and **framework-specialty** (evidence-gated: ORM patterns, migration safety, async/queue jobs, event-boundary review, notification delivery, caching strategy, file storage, auth scaffolding, form/validator design, admin panel, declarative API resources, console commands, repository review, container review, test-data factories, package authoring), plus integrations (unchanged).
- This closes the gap identified by auditing the Laravel/Symfony/PHP-Core accelerators in this monorepo: a generated target previously received only an architecture skill + 7 universal skills + integrations, far narrower than the ~40-skill hand-built accelerators. The new framework-specialty catalog generalizes the useful Laravel-only (`eloquent`, `queues-jobs`, `caching`, `events-notifications`, `file-storage`, `console-scheduler`, `auth-scaffolding`, `filament`, `package-developer`) and Symfony-only (`doctrine-migration-designer`, `messenger-designer`, `event-subscriber-designer`, `security-voter-designer`, `form-validator-designer`, `fixture-factory-generator`, `console-command-coder`, `container-reviewer`, `repository-reviewer`, `api-platform-designer`) skills into signal-named, framework-neutral equivalents keyed to real scan evidence rather than to which framework folder they came from.
- New Project Profile section 3.1 ("Framework-Specialty Signals") and 3.2 ("Frontend Presence"), populated by an expanded `architecture-scanner` that now also detects ORM/data-access pattern, migration tooling, async/queue mechanism, event/notification patterns, caching/storage usage, auth/authorization scaffolding, form/validator design, admin panels, declarative API frameworks, console commands, repository/DI-container style, test factories, package-vs-application nature, and rendering/frontend-asset presence.
- Two new bundled references for `skill-forge`: `references/php-process-skills.md` (the 14 always-generated process/workflow skills) and `references/php-specialty-skills.md` (the evidence-gated specialty catalog, mapped from section 3.1 signals to generated skills). `references/php-frameworks.md` gained "Design & Interaction Skills" and "Frontend Skills" sections.
- Section 10.1 of the Project Profile now lists all six skill groups explicitly (with the group-by-group count breakdown carried into 10.2's arithmetic), and `infra-scan`'s "What Will Be Generated" summary reports the full group breakdown, not just architecture/universal/integrations.
- New DOD checks (Tier 1 and Tier 2) verifying sections 3.1/3.2 and that the fixed design/process groups and the conditional frontend/specialty groups were generated correctly.
- `examples/infra-scan-project-profile-example.md` rewritten end to end for the new sections and the resulting 43-skill total for the acme-billing example (up from 15).

### Fixed

- Removed the unintentional overlap between the universal `debugging` skill and the process skill `systematic-debugger` (both previously could restate the same "root-cause-first" debugging content). Their scope is now explicitly split and cross-referenced: `debugging` owns only the target's real tools/log locations ("where to look"); `systematic-debugger` owns the tool-agnostic investigative methodology ("how to look") and names zero target-specific tools. New DOD check verifies neither duplicates the other.
- Audited every remaining pair of generated skills for the same kind of overlap and fixed three more, all with the same scope-split-plus-cross-reference pattern (no skill counts changed):
  - `database-designer` (design & interaction, schema/table/index/migration design) vs. `orm-patterns` (specialty, how application code uses that schema through the ORM - relationships-as-used-in-code, casts, scopes, eager loading). Previously `database-designer`'s own reference text mentioned "Eloquent relationships," directly overlapping `orm-patterns`.
  - `performance` (universal, measure-first hot-path workflow) vs. `caching-strategy` (specialty, cache-aside correctness/invalidation). `performance` now names caching only as one possible hot-path lever and defers to `caching-strategy` for correctness depth when that specialty skill is generated.
  - `api-designer` (design & interaction, hand-rolled routes/controllers) vs. `api-platform-design` (specialty, declarative API resource frameworks). `api-designer` now explicitly narrows to any remaining hand-rolled endpoints and defers resource-level design to `api-platform-design` when a declarative framework is the target's primary/sole API mechanism.
  - DOD's Tier 2 check was broadened from just the `debugging`/`systematic-debugger` pair to cover all four scope-split pairs.

## [1.2.0] - 2026-07-21

### Added

- The Project Profile's "Generation Notes" section is now split into 10.1 (Skills To Generate), 10.2 (Agents & Commands Preview), and 10.3 (Non-PHP Neighbors). 10.1 now requires a one-line, target-specific description for every proposed skill (naming the real package/pattern/tool found) instead of a bare skill name; 10.2 states the exact agent and command counts implied by the selected edition(s).
- New Project Profile section 11 ("Memory Bank Preview") - `profile-synthesizer` now previews every memory-bank chunk `memory-seed` will seed (planned ID, title, type, source) before the user ever runs `infra-generate`, using the same confirmed-only selection rule `memory-seed` applies.
- `memory-seed` now treats the profile's section 11 as its authoritative seed plan and reports any drift between the preview and what it actually seeds, rather than silently reconciling differences.
- `infra-scan`'s final report now surfaces a "What Will Be Generated" summary (skill/agent/command counts and memory-bank chunk count) alongside the existing confidence summary, so the scope of a generation run is visible without opening the full profile.
- New DOD checks (Tier 1 and Tier 2) verifying the enriched profile sections and that seeded memory-bank chunks match their preview.
- `examples/infra-scan-project-profile-example.md` rewritten to demonstrate the new sections 10.1-10.3 and 11 end to end.

## [1.1.0] - 2026-07-20

### Added

- Self-adaptation: when `infra-scan` finds no PHP evidence, it now probes for a recognizable non-PHP stack (Flutter/Dart, Node.js, Python, Go, Ruby, Java/Kotlin, .NET, Rust, Swift, or similar, via a generic manifest-signal table) instead of only reporting out of scope.
- New `stack-adapter` skill (21st skill) - a meta-generator that, with explicit user confirmation, researches the detected stack live and builds a brand-new, fully independent sibling generator (`Infrastructure-Creator-[Stack]/`, a sibling folder next to this one) with the identical 21-skill, three-edition architecture (including its own re-authored copy of `stack-adapter`), freshly authored for that stack. Zero mentions of PHP or of this generator appear in the sibling's own content.
- New `infra-adapt <target-path>` entry point (command + agent, mirrored Claude/Cursor) for invoking `stack-adapter` directly, without going through `infra-scan`'s auto-detection first.
- `stack-adapter` is documented as the fourth sanctioned Orchestration Exception in `AGENTS.md`, alongside `infra-scan`, `infra-generate`, and `infra-build`.
- New Golden Principle ("Honest Scope Over Silent Failure or Scope Creep") and a new DOD tier (Tier 4 - Stack Adaptation) covering verification of a self-generated sibling.

## [1.0.0] - 2026-07-18

### Added

- Initial release of Infrastructure-Creator: a standalone, PHP-only meta-accelerator that scans a target PHP project and generates a bespoke accelerator directly into it, for only the AI tool(s) the target team selects.
- Two-phase workflow with a human review checkpoint:
  - `infra-scan <target>` - six parallel PHP scanners (stack, architecture, integrations, infra/ops, security/compliance, conventions), grounded web research, a minimal clarifying interview (including the mandatory AI-tool-selection question), and synthesis into one reviewable Project Profile.
  - `infra-generate <target>` - policy/skill/agent/command/hook forges plus memory seeding, flow composition, and a final verification gate; writes only the selected edition(s).
  - `infra-build <target>` - optional one-shot that chains scan then generate, pausing only on blocking ambiguity or a collision.
- 20 skills total, following standard `SKILL.md`/agent/command frontmatter conventions.
- The generator itself ships in three editions - Claude Code (`.claude/`), Cursor (`.cursor/`), and Codex (`.agents/skills` + `.codex/`) - so it runs natively from whichever tool the operator uses.
- Bundled, PHP-specialized reference material: framework detection signals, an integration catalog, architecture patterns, the Project Profile schema, a dependency-free generated-output validator, and the memory-bank validator + chunk template.
- Safety guardrails: workspace-boundary enforcement, a collision guard for pre-existing target accelerators, evidence-cited findings, and a strict no-secrets policy.
