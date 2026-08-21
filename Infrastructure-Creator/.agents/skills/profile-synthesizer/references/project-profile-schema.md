# Project Profile Schema

The profile and its machine-readable skill plan are the hand-off artifacts between Phase 1 (scan) and Phase 2 (generate). `profile-synthesizer` writes both; every forge reads them after re-validating evidence against current target files.

- Human review: `tasks/TASK-{N}/infra-scan-project-profile.md`
- Machine contract: `tasks/TASK-{N}/skill-generation-plan.json`

Every factual line MUST carry a confidence tag - `(confirmed - path:L#)`, `(inferred - reason)`, or `(unknown)` - so generated artifacts remain auditable back to evidence. Behavioral findings in section 8 MUST additionally name the source type (`spec/ADR`, `test`, `database constraint`, `workflow configuration`, `authorization rule`, `domain code`, `application code`, `configuration`, or `interview answer`) because confirmed implementation and confirmed policy are not equal authorities.

## Machine-Readable Skill Plan

The JSON top level MUST contain exactly these required members (extensions require a schema-version change):

```json
{
  "schema_version": "1.6",
  "catalog_version": "2.5.0",
  "target_root": "/absolute/path/to/target",
  "profile": "tasks/TASK-001/infra-scan-project-profile.md",
  "evidence": [],
  "preexisting_team_skills": [],
  "skills": [],
  "rejected_candidates": [],
  "critical_invariants": [],
  "flow_contracts": {"roster": [], "flows": []}
}
```

Top-level membership is exact. New plans use `schema_version": "1.6"`, which
adds the typed `preexisting_team_skills` collision contract. Schema 1.5 added
the rejection `disposition` field. 1.5 and earlier stay readable for audit
diagnostics and are ineligible for publication.
Schemas `1.0` through `1.5` remain readable for plan-only audit diagnostics
and produce a nonblocking `PLAN_SCHEMA_MIGRATION` warning. They are publication
ineligible: full or partial authored-skill validation emits blocking
`LEGACY_PLAN_PUBLICATION_INELIGIBLE`. New synthesis runs MUST emit 1.6.

**1.6 adds one top-level merge collision contract.** Each
`preexisting_team_skills[]` record exists only when a selected-edition skill
path already resolves to a team-owned symlink that merge mode must preserve.
It pins the original name/candidate, edition, target-relative symlink path,
literal link target and SHA-256, every protected regular file and SHA-256, and
a distinct `generated_alias`. Its fixed values are `kind:
preexisting-team`, `mode: merge`, `ownership: team`, and `publication:
watch-only`. The aliased generated skill carries
`preexisting_team_replacement: <original-name>` and remains fully subject to
the normal evidence, semantic, routing, and generated-path checks. The
protected path and files enter the watch plan only; they never enter staging,
publication, write-plan, or manifest ownership. Symlink safety is not relaxed
for generated or owned paths.

**1.3 changed nested shapes only** - a role entry names the evidence and the
procedure step that carry it; an executable verification records what its
command does on the unmodified target; an evidence entry may state an absence.

**1.4 adds two skill members** and leaves the top level alone: `claim_ids`,
naming the reconciled claims from `project-claims.json` this skill rests on, and
`evidence_dispositions`, the record of evidence inside the skill's own declared
paths that it deliberately does not use. Synthesis reads the whole ledger and
writes one contract at a time, so a passed-over finding used to leave no trace:
the plan looked identical whether the author judged it irrelevant or never saw
it. Evidence inside the skill's own `path_contracts` or `ownership` paths that is
neither cited nor ruled out is `SKILL_EVIDENCE_UNDISPOSED`. Measured on five real
plans, that is a median of 0-3 undecided items per skill and 4-47 per plan, so
the obligation grows with what a skill claims rather than with the ledger.
`catalog_version` must equal
`skill-forge/references/candidate-registry.json`. Every registry candidate must
appear in exactly one disposition: a selected skill's `selection_gate` or
`rejected_candidates` (family candidates may select multiple concrete skills).
`profile` must identify the sibling Profile in the same
`tasks/TASK-{N}/`. By contrast, evidence paths and generated-skill source
citations MUST be canonical paths relative to `target_root`; they MUST NOT be
absolute, contain `..`, or point into `tasks/TASK-{N}/`.

Each `evidence[]` entry is:

```json
{
  "id": "EV-0001",
  "path": "composer.json",
  "source_type": "configuration",
  "authority": "Declares runtime dependencies and PHP constraints",
  "confidence": "confirmed",
  "line_range": {"start": 8, "end": 19},
  "fingerprint": "sha256:<digest>",
  "supported_claims": ["Laravel is a runtime dependency"]
}
```

An evidence entry may instead record an **absence** - something the target does
not do, such as an analyser that is configured and invoked from nowhere. The
entry then carries `absence` with exactly `subject`, `search`, and
`accounted_matches`, and no `path`, `url`, `fingerprint`, or `line_range`:

```json
{
  "id": "EV-0031",
  "absence": {
    "subject": "No CI workflow or composer script invokes PHPStan",
    "search": "grep -rn phpstan .github composer.json",
    "accounted_matches": ["composer.json"]
  },
  "source_type": "absence",
  "authority": "The only match declares the dependency; nothing runs it",
  "confidence": "confirmed",
  "supported_claims": ["PHPStan is installed but never invoked"]
}
```

The search must be one this gate can resolve literally, and its resolution must
equal `accounted_matches` exactly: a match outside the set is
`EVIDENCE_ABSENCE_CONTRADICTED`, and an accounted path that stops matching is
`EVIDENCE_ABSENCE_STALE`. What the accounted matches *mean* stays the author's
judgement, but the file set is pinned, so the finding goes stale loudly when the
target changes. Claims on an absence entry are grounded against the search, not
against the subject line, which would be circular.

Otherwise exactly one of `path` or `url` is required. `id`, `source_type`, `authority`,
`confidence`, and non-empty `supported_claims` are required. Repository-path
evidence also requires the current `sha256:<digest>` fingerprint; `line_range`
is optional but may not exceed the source. URLs rely on their recorded
authority/version and do not require a local fingerprint.

Each `skills[]` entry is one complete, independently actionable contract:

```json
{
  "name": "testing",
  "category": "universal",
  "kind": "project-adapted",
  "phase": "implementation",
  "capability": {
    "mode": "workspace-write",
    "summary": "May add or update tests within the declared write paths"
  },
  "necessity_rationale": "The target has an evidenced test suite and project-specific test conventions that require operational guidance.",
  "selection_gate": {
    "catalog": "php-frameworks.md#testing",
    "candidate_id": "testing",
    "candidate": "testing",
    "conditions": [
      {
        "requirement": "A configured target test suite and conventions exist",
        "evidence_ids": ["EV-0007", "EV-0008"],
        "status": "satisfied",
        "explanation": "The cited PHPUnit config and base test case prove distinct operational guidance."
      }
    ],
    "distinct_value_from": ["coding", "systematic-debugger"]
  },
  "triggers": {
    "positive": ["A change requires tests under tests/Feature"],
    "negative": ["The request is only to investigate a production symptom"]
  },
  "evidence_ids": ["EV-0007", "EV-0008"],
  "claim_ids": ["CLM-0004", "CLM-0011"],
  "evidence_dispositions": [
    {"evidence_id": "EV-0021", "disposition": "excluded", "reason": "A fixture factory under tests/, owned and documented by fixture-factory-generator"}
  ],
  "source_paths": ["phpunit.xml", "tests/TestCase.php"],
  "owned_scope": ["Select and implement tests using the target's suite structure"],
  "excluded_scope": ["Root-cause investigation", "Release orchestration"],
  "ownership": [
    {
      "id": "testing.test-suite",
      "mode": "exclusive",
      "description": "Test selection and implementation in the target suite",
      "paths": ["tests/**"]
    }
  ],
  "required_procedure_roles": [
    {"role": "select-suite-and-focused-scope", "requirements": ["Read the cited PHPUnit configuration and pick the suite the change belongs to"], "evidence_ids": ["EV-0007"], "procedure_step_ids": ["inspect-test-topology"]},
    {"role": "derive-critical-scenarios-from-invariants", "requirements": ["Turn each intersecting high-priority invariant into a named case"], "evidence_ids": ["EV-0008"], "procedure_step_ids": ["name-cases-from-invariants"]},
    {"role": "cover-denied-and-forbidden-paths", "requirements": ["Assert the transitions and permissions the target refuses"], "evidence_ids": ["EV-0008"], "procedure_step_ids": ["assert-refused-transitions"]},
    {"role": "arrange-fixtures-and-test-doubles", "requirements": ["Build rows through the target's own factories and fakes"], "evidence_ids": ["EV-0008"], "procedure_step_ids": ["arrange-through-target-factories"]},
    {"role": "execute-the-evidenced-focused-command", "requirements": ["Run the command the scan proved, not a composed one"], "evidence_ids": ["EV-0007"], "procedure_step_ids": ["run-the-evidenced-command"]},
    {"role": "assert-expected-and-forbidden-outcomes", "requirements": ["State what must hold and what must never appear"], "evidence_ids": ["EV-0008"], "procedure_step_ids": ["assert-refused-transitions", "run-the-evidenced-command"]}
  ],
  "procedure_steps": [
    {
      "id": "inspect-test-topology",
      "action": "Inspect the PHPUnit configuration and base test case before selecting a test level",
      "evidence_ids": ["EV-0007"],
      "path_refs": ["phpunit.xml", "tests/TestCase.php"],
      "decision_refs": ["choose-test-level"],
      "expected_outcome": "The selected unit, integration, or feature boundary matches the target suite",
      "failure_branch": "Stop and report unresolved test authority when no branch is evidenced"
    },
    {
      "id": "name-cases-from-invariants",
      "action": "Name one case per intersecting invariant from section 8.4 of the Profile",
      "evidence_ids": ["EV-0008"],
      "path_refs": ["tests/TestCase.php"],
      "decision_refs": ["choose-test-level"],
      "expected_outcome": "Every intersecting invariant has a named case",
      "failure_branch": "Report the invariant that no case can reach and stop"
    },
    {
      "id": "assert-refused-transitions",
      "action": "Assert the transitions and permissions the target refuses, not only the ones it allows",
      "evidence_ids": ["EV-0008"],
      "path_refs": ["tests/TestCase.php"],
      "decision_refs": ["choose-test-level"],
      "expected_outcome": "Each denied path fails for the target's own reason",
      "failure_branch": "Report the refusal the suite cannot express and stop"
    },
    {
      "id": "arrange-through-target-factories",
      "action": "Arrange rows through the target's own factories and fakes rather than literal fixtures",
      "evidence_ids": ["EV-0008"],
      "path_refs": ["tests/TestCase.php"],
      "decision_refs": ["choose-test-level"],
      "expected_outcome": "Arrangement uses the factories the scan proved",
      "failure_branch": "Report the missing factory and stop rather than inlining data"
    },
    {
      "id": "run-the-evidenced-command",
      "action": "Run the focused command the scan proved against the changed file only",
      "evidence_ids": ["EV-0007"],
      "path_refs": ["phpunit.xml"],
      "decision_refs": ["choose-test-level"],
      "expected_outcome": "The focused command runs and its output is recorded",
      "failure_branch": "Report SKIPPED with the missing dependency when the command cannot run"
    }
  ],
  "decision_points": [
    {"id": "choose-test-level", "question": "Which test level protects this behavior?", "branches": ["unit", "integration", "feature/end-to-end"]}
  ],
  "verification": [
    {
      "id": "run-focused-tests",
      "mode": "command",
      "instruction": "Run the focused PHPUnit file that covers the changed behavior",
      "command": "vendor/bin/phpunit tests/Feature/ExampleTest.php",
      "baseline": {
        "command": "vendor/bin/phpunit tests/Feature/ExampleTest.php",
        "observed": "exit 1; 42 passed, 3 failed (ExampleTest::testLegacyImport and 2 others)",
        "outcome": "failing"
      },
      "prerequisites": ["Local dependencies are installed"],
      "safe_scope": "Local test process with no provider or production access",
      "mutation_class": "none",
      "network_class": "none",
      "expected_result": "No failure outside the three recorded in the baseline, and every case named for this change passes",
      "failure_result": "Any new failure, or a recorded failure that changes shape, blocks completion",
      "skip_condition": "vendor/bin/phpunit is absent or the required local dependency is unavailable",
      "skip_reporting": "Report SKIPPED with the missing dependency and the unverified assertion"
    }
  ],
  "output_contract": ["Changed tests or a test plan", "Commands and results"],
  "failure_handling": ["Stop when required tooling is unavailable and report N/A", "Do not weaken assertions to force a pass"],
  "integration_safety": {
    "network_policy": "forbidden",
    "test_double_strategy": "Use local fixtures and framework fakes",
    "environment": "local",
    "authorization_required": false,
    "rollback": "Revert only files changed by this skill when verification fails",
    "sanitization": "Exclude credentials, customer payloads, and unsanitized provider errors"
  },
  "path_contracts": [
    {
      "path": "phpunit.xml",
      "access": "read",
      "classification": "required-existing",
      "evidence_ids": ["EV-0007"]
    },
    {
      "path": "tests/**",
      "access": "write",
      "classification": "creatable",
      "evidence_ids": ["EV-0008"]
    }
  ],
  "evidence_anchors": [
    {
      "evidence_id": "EV-0007",
      "claim": "The target has a configured PHPUnit feature-test suite",
      "anchor": "phpunit.xml:L8-L19",
      "procedure_step_ids": ["inspect-test-topology"],
      "verification_ids": ["run-focused-tests"]
    }
  ],
  "routing_cases": [
    {
      "prompt": "Add a feature test for the changed HTTP behavior",
      "expected_primary": "testing",
      "permitted_secondary": [],
      "forbidden_skills": ["systematic-debugger"],
      "rationale": "The request changes tests in the owned target suite",
      "evidence_ids": ["EV-0007", "EV-0008"]
    },
    {
      "prompt": "Investigate an unexplained production failure before an expected assertion is known",
      "expected_primary": "systematic-debugger",
      "permitted_secondary": [],
      "forbidden_skills": ["testing"],
      "rationale": "Root-cause investigation precedes test implementation",
      "evidence_ids": ["EV-0007"]
    }
  ],
  "related_skills": ["coding"],
  "nearest_siblings": [
    {
      "name": "systematic-debugger",
      "role": "defer",
      "ownership_ids": ["testing.test-suite"],
      "boundary": "Defer root-cause investigation; testing protects expected behavior."
    }
  ],
  "writes": ["tests/**"],
  "fixed_blocks": [
    {"id": "safety.no-secrets", "version": "1.0", "content": "Exact approved text"}
  ]
}
```

All shown members except `fixed_blocks` are required under schema 1.6, including non-empty
positive and negative triggers, owned and excluded scope, structured ownership,
required procedure roles and steps, decision points, structured verification,
integration safety, path contracts, evidence anchors, routing cases, output,
failure handling, siblings, and writes (use an empty array only when
`capability.mode` is `read-only`). `kind` distinguishes `runtime-fixed` from
`project-adapted`, `integration`, `specialty`, and `domain-review`.
`phase` uses the fixed flow vocabulary - `understanding`, `planning`,
`implementation`, `verification`, or `finalization` - because the canonical
flow roster copies each skill's phase verbatim and flow stages accept only
that vocabulary.
`fixed_blocks`, when used, contains explicit `id`, `version`, and exact
`content`; only that exact block is exempt from duplication checks, and it never
exempts the skill-specific procedure.

`capability.mode` is exactly `read-only`, `workspace-write`, or
`external-side-effect`. Read-only skills MUST have empty `writes` and use
inspect/evaluate/report language; mutation verbs in procedure actions are
blocking. Workspace-write skills MUST declare at least one write path.
External-side-effect skills require an explicitly authorized
`sandbox-with-approval` or `approved-live` integration policy; target writes,
when any, remain bounded by `writes` and path contracts.

Each `decision_points[]` item has exactly `id`, `question`, and
non-empty `branches`. Each `procedure_steps[]` item has exactly `id`, `action`,
`evidence_ids`, `path_refs`, `decision_refs`, `expected_outcome`, and
`failure_branch`. IDs are stable dotted/kebab lowercase, and every decision
reference resolves within the skill.
Every evidence reference is declared by the skill. Each `verification[]` item
has exactly `id`, `mode`, `instruction`, `command`, `prerequisites`,
`required_procedure_roles` carries the candidate's own obligations from
`candidate-registry.json`, not a universal trio. A candidate whose registry entry
declares `roles` must cover every one of them, or the plan fails with
`CATALOG_ROLE_UNCOVERED`: a database designer derives constraints from invariants
and indexes from evidenced access paths; a testing skill selects a suite and
covers denied paths. `load-evidence`/`execute`/`verify` describes every skill ever
written and therefore describes none - two measured plans filled this field with
exactly that trio for 9 of 9 and 35 of 35 skills.

Each role entry has exactly `role`, `requirements`, `evidence_ids`, and
`procedure_step_ids`. Declaring an obligation is not carrying it, so every role
names the evidence that grounds it and the operational step that discharges it;
both must resolve inside the skill, and a runtime-fixed skill wires steps only.
Three or more obligations discharged by one and the same step is
`PROCEDURE_ROLE_COLLAPSED` - the "one general inspection step" shape. Measured
before release: 0 of 45 skills across four real runs sit at one step, against 37
of 39 in an externally authored plan, so the rule separates a template from
honest work rather than taxing it.

`safe_scope`, `mutation_class`, `network_class`, `expected_result`,
`failure_result`, `skip_condition`, and `skip_reporting`.
`baseline` records what the command does on the **unmodified** target - exactly
`command` (identical to the check's own), `observed`, and `outcome`, one of
`passing`, `failing`, or `failing-remediated`. Manual checks and checks this gate
resolves literally may use JSON `null`; any other command check without a
baseline is `VERIFICATION_BASELINE_MISSING`, because an expectation may not be
declared for a command nobody observed. See
[ADR-002](../../../../../docs/ADR-002-executable-verification-baselines.md).

**Write the expectation differentially.** Against a `failing` baseline, promising
that the command succeeds outright is `VERIFICATION_BASELINE_CONTRADICTED`: on a
target whose linter or suite does not currently pass, "exits zero" is false the
moment it is written, and the skill then raises a blocking finding on untouched
code every time it runs. Say "no failure outside the recorded baseline" instead.
Use `failing-remediated` only when eliminating the recorded failure is this
skill's own declared job; a read-only skill may not claim it. Where the gate can
resolve the command itself, a recorded baseline is cross-checked against that
resolution rather than trusted.

**A runtime command is graded against its contract, not against the target.**
The memory quartet verifies itself with the seeded runtime, and on a first
generation that runtime does not exist on the target yet - the generation
installs it - so there is no unmodified target to observe and ADR-002's model
has no subject. The runtime is fixed and shipped by this generator, so its
behaviour belongs to `memory-seed/assets/runtime-contract.json`, which declares
what each command's exit codes mean. A runtime-fixed skill therefore needs no
baseline for such a command; its `expected_result` may not promise the command
succeeds outright when the contract declares a nonzero exit
(`RUNTIME_EXPECTATION_CONTRADICTED`), and a baseline recorded anyway - which an
update legitimately can, since the runtime exists by then - may not say what the
contract does not declare (`RUNTIME_BASELINE_CONTRADICTED`).
Measured on five real runs before release: 2 of 23 runtime checks fire, both of
them skills promising `context.py validate` exits zero, which the contract
declares it does not whenever an index is stale.

`mode` is `command` or `manual`; command checks require a concrete command,
while manual checks use JSON `null`. Generic references to an "evidenced",
"configured", "appropriate", or "narrow" command are invalid. Verification
commands MUST be non-mutating; for example, a lint script containing `--fix`
cannot be used as a verification command.
`mutation_class` is `none`, `workspace-write`, or `destructive`, and
`network_class` is `none`, `local`, or `external-provider`. Publication
verification requires mutation class `none`; external-provider checks also
require external-side-effect capability and an approved integration policy.

A routing fixture may not name the skill it expects to win.
"Route architecture-implementer work to architecture-implementer" tests string
matching, not routing: no arrangement of skills could get it wrong, so it proves
nothing about whether the boundaries hold. Write the request in the words a
person would use; `ROUTING_CASE_TAUTOLOGICAL` blocks the rest. Runtime-fixed
skills are exempt, because their names are ordinary words for what they do.
Measured before release: 0 of 148 fixtures across four real runs, and 183 of 183
non-runtime fixtures in an externally authored plan, 40 of them verbatim.

`integration_safety` is required for every skill so network behavior is
explicit. `network_policy` is `forbidden`, `mock-only`,
`sandbox-with-approval`, or `approved-live`. Integration skills default to
static inspection, fakes, fixtures, local adapters, or sandboxes. Any policy
that permits provider access requires explicit authorization; an
`approved-live` policy still requires a safe test-double default. Rollback and
sanitization are always substantive.
`environment` is `none`, `local`, `sandbox`, or `approved-live` and must agree
with the network policy.

Each `path_contracts[]` item has exactly `path`, `access`, `classification`, and
`evidence_ids`. Access is `read` or `write`; classification is
`required-existing`, `generated-runtime`, or `creatable`. Every source and
write path must have a contract.
Existing paths/globs resolve in the target. Generated paths are reserved for
`runtime-fixed` output. Creatable paths are write paths beneath an existing
parent. All paths use the normalized target-relative rules below.

Each `evidence_anchors[]` item has exactly `evidence_id`, `claim`, `anchor`,
`procedure_step_ids`, and `verification_ids`. The claim exactly matches one of
the evidence entry's `supported_claims`; the anchor begins with the canonical
path followed by a bounded line or stable symbol reference. Every skill
evidence ID has an anchor. The claim must be traceable through owned scope,
the referenced procedure step and verification, and the output contract.

Each `routing_cases[]` item has exactly `prompt`, `expected_primary`,
`permitted_secondary`, `forbidden_skills`, `rationale`, and `evidence_ids`.
Owner sets are disjoint, evidence resolves within the skill, and cases cover
selection of the current skill plus deferral to every nearest sibling.
`evidence_ids` must be non-empty and a subset of the skill's own
`evidence_ids` - with one exception that keeps the two rules consistent: a
`runtime-fixed` skill may declare no evidence at all, so its routing cases may
carry an empty `evidence_ids`. Any other skill, and any runtime-fixed skill
that does declare evidence, must cite at least one declared ID per case. A
non-empty list that is not a subset of the skill's declared evidence is
blocking for every kind.

Each `ownership[]` item has exactly `id`, `mode`, `description`, and `paths`.
The ID is stable kebab/dotted lowercase syntax within the inventory and is used
by sibling routing boundaries. `mode` is `exclusive`, `shared`, or `composed`.
An `exclusive` ID has exactly one owner. A `shared` ID may have multiple owners
but never authorizes intersecting writes. A `composed` ID may have multiple
contributors, but every intersecting owned path has exactly one writer/composer
as determined by `writes`. Reusing an ID with a different mode or substantive
description is a semantic ownership-ID conflict. Paths and writes use normalized,
target-relative `/`-separated globs: no absolute paths, `..`, backslashes,
leading `./`, duplicate separators, or trailing `/`. Glob intersection, not
literal string equality, determines write and ownership overlap.

Under schemas 1.1 and 1.2 each `nearest_siblings[]` item has exactly `name`, `role`,
`ownership_ids`, and `boundary`. `role` is `primary`, `defer`, or `fallback`.
Every relation is reciprocal, names the same ownership IDs in both directions,
and forms either `primary`/`defer` or `primary`/`fallback`. Missing reciprocals,
unknown ownership IDs, mismatched ID sets, and bilateral `primary` (or any other
role contradiction) are blocking. A valid reciprocal relation may resolve an
intentional semantic-scope overlap; intersecting write globs always receive the
separate blocking `WRITE_SURFACE_COLLISION` diagnostic.

Every non-runtime `selection_gate.conditions[]` entry must be `satisfied`, cite
evidence already listed by the skill, and explain a claim supported by that
evidence. `distinct_value_from` records adjacent candidates considered during
pruning. Runtime-fixed memory skills may cite an empty evidence list only when
the condition names the guaranteed generated runtime.

`evidence_ids` MUST resolve to `evidence[]`. `source_paths` is the de-duplicated
target-relative subset used by the skill. Every `related_skills` and
`nearest_siblings[].name` MUST exist in the same plan. A skill contract cannot
rely on another skill to supply its purpose, inputs, procedure, verification,
output, or failure behavior. Contract similarity and repeated-block warnings
compare field values only; structural JSON keys, evidence IDs, paths, and exact
versioned `fixed_blocks` are excluded.

Schema 1.2 also carries plan-level critical invariants and executable flow
shape:

```json
{
  "critical_invariants": [
    {
      "id": "invoice.paid-immutable",
      "statement": "A paid invoice is immutable except through the refund workflow",
      "evidence_ids": ["EV-0042"],
      "skill_names": ["billing-rules-review"],
      "assertions": [
        {
          "skill_name": "billing-rules-review",
          "verification_id": "assert-paid-invoice-immutable"
        }
      ]
    }
  ],
  "flow_contracts": {
    "roster": [
      {"skill": "coding", "agent": "coding-agent", "phase": "implementation", "writes": true},
      {"skill": "code-review", "agent": "code-review-agent", "phase": "verification", "writes": false}
    ],
    "flows": [
      {
        "name": "flow-feature",
        "required_code_review": "code-review-agent",
        "stages": [
          {"phase": "implementation", "agents": ["coding-agent"], "parallel": false, "checkpoint": true},
          {"phase": "verification", "agents": ["code-review-agent"], "parallel": false, "checkpoint": true}
        ]
      },
      {
        "name": "flow-review",
        "required_code_review": "code-review-agent",
        "stages": [
          {"phase": "verification", "agents": ["code-review-agent"], "parallel": false, "checkpoint": true}
        ]
      }
    ]
  }
}
```

Every `critical_invariants[]` item has exactly `id`, `statement`,
`evidence_ids`, `skill_names`, and non-empty `assertions`. Every assertion names
a selected skill and one of its structured verification IDs. IDs are unique,
evidence and skill
references resolve, and each named skill carries the invariant into both a
procedure step and a verification assertion. `flow_contracts` has exactly
`roster` and `flows`. The roster is derived in plan order from every selected
skill, agent name, phase, and write capability. Each flow has `name`,
`required_code_review`, and ordered stages containing exactly `phase`, `agents`,
`parallel`, and `checkpoint`. Both `flow-feature` and `flow-review` are required;
when `code-review` is selected, its agent is a required review stage.

Each rejected catalog candidate is recorded so inventory pruning is auditable:

```json
{
  "candidate_id": "caching-strategy",
  "name": "caching-strategy",
  "category": "specialty",
  "disposition": "absent",
  "reason": "No cache runtime wiring: config/packages/cache.yaml holds only the skeleton default and no cache client is constructed under src/",
  "missing_evidence": ["cache call sites under src/", "write-path invalidation"]
}
```

A rejection declares which kind of rejection it is, and each kind carries its
own exact fields on top of `candidate_id`, `name`, `category`, `reason` and
`disposition`:

- **`absent`** - nothing in the target holds this concern; carries
  `missing_evidence`. This is the falsifiable kind: the registry's signal for
  the candidate is run against the target, and a target that holds the surface
  refutes the rejection.
- **`consolidated`** - the concern exists and a skill this plan selects already
  owns it; carries `absorbed_by`, which must name one of those skills. It is not
  refuted by the target the way `absent` is, because it concedes the surface -
  and that concession is what gets checked. Three questions, all mechanical:
  the owner must be a selected skill (`REJECTION_ABSORBER_UNKNOWN`), the surface
  must actually be in this target (`REJECTION_CONSOLIDATION_WITHOUT_SURFACE` -
  a concern that is not here is `absent`, not absorbed), and the owner's own
  declared paths must reach it (`REJECTION_ABSORBER_OUT_OF_REACH`). Naming a
  selected skill used to be the whole test, which is how a catalog gets folded
  into one review skill: it cost a sentence and read as judgement.

Consolidating into a skill whose own scope does not reach the concern is how a
gap gets hidden behind a name, so widen that skill's `ownership` and
`owned_scope` in the same plan or record the rejection as `absent`.

```json
{
  "candidate_id": "orm-patterns",
  "name": "orm-patterns",
  "category": "specialty",
  "disposition": "consolidated",
  "reason": "Doctrine mappings live in src/Entity/, which coding owns and changes",
  "absorbed_by": "coding"
}
```

A selection condition reports one of two statuses. `satisfied` is the ordinary
one. `absent-golden` says the requirement is **not** met and the skill is
generated anyway, which only a candidate the registry marks `golden` may do: the
development loop does not wait for a profiler to be installed. Such a condition
cites an absence evidence entry - the kind carrying `absence: {subject, search,
accounted_matches}`, whose search this gate runs itself - and the skill carries
`narrow_scope`, one sentence naming what it still does and what it cannot do
until the surface exists. A skill that narrows without an absent condition is
`NARROW_SCOPE_UNEXPECTED`; one that reports an absence it cannot prove is
`ABSENT_GOLDEN_UNPROVEN`.

A rejection is a judgement about the target and is held to the target:

- **It must say where the surface was looked for.** `reason` and
  `missing_evidence` together must name at least one concrete path, glob or
  root-level file, exactly as a selection must cite where its evidence was
  found. A rejection naming nothing is `REJECTION_UNANCHORED`.
- **One sentence may not judge a whole catalog.** Sibling candidates in one
  family can honestly share a sentence ("this project renders no HTML" covers
  every frontend candidate), but the same sentence spread across categories
  judges none of them and is `REJECTION_TEMPLATED`.
- **It must rest on what the target is, never on what the current request
  wants.** The accelerator is generated once, for all later work; "no one asked
  for a review yet" is not a property of the project and is not a ground for
  dropping a skill.
- **It is checked.** Every registry candidate carries the signal whose presence
  makes "no surface here" false, and the gate runs those signals against the
  real target: a rejection the target contradicts is `REJECTION_CONTRADICTED`.
  Candidates whose trigger no repository could show (an open design question, a
  recorded agent mistake) carry no signal and say so in `falsifier_absent`.

`candidate_id` and `catalog` must resolve exactly to the registry entry and its
real Markdown anchor. This prevents a plan from inventing a catalog reference
or silently omitting candidates from the selected/rejected inventory.

### The runtime-fixed quartet: what it is measured by

`memory-bank`, `project-brain`, `checkpoint`, and `memory` carry
`kind: "runtime-fixed"` and are the only candidates the registry marks
`"mode": "runtime-fixed"`. They are generated unconditionally because
`memory-seed` installs the runtime they operate in the same generation run.

They may have empty `evidence_ids`/`source_paths` during synthesis because
their necessity comes from those runtime files rather than from target
evidence. Their `necessity_rationale` and `writes` must name that dependency.
The authored skills must refer to those future files by target-relative paths
such as `memory-bank/scripts/context.py`, never by generator task/staging
paths.

Emptiness propagates consistently: every member that cites evidence -
`selection_gate.conditions[].evidence_ids`, `path_contracts[].evidence_ids`,
`procedure_steps[].evidence_ids`, `routing_cases[].evidence_ids` - accepts an
empty list for a runtime-fixed skill, and `evidence_anchors` is required only
when evidence is actually declared. The gate likewise does not require a
runtime-fixed skill's body to quote a target evidence path.

**This is a decision, not a gap in selection.** A runtime-fixed skill
documents the generator's memory runtime, not the target's code, so project
specificity is a bar it cannot meet by construction. It is held to two
verifiable bars instead:

- **Runtime accuracy.** Every path it names under `memory-bank/` or
  `project-brain/`, and every `python3 memory-bank/scripts/*.py` command form
  it names, must exist in `memory-seed/assets/runtime-contract.json`
  (`path_contracts.required_skeleton`, `path_contracts.creatable`,
  `commands`). Forbidden invented paths, unlisted paths, unlisted
  subcommands, and unlisted flags are blocking
  (`RUNTIME_PATH_FORBIDDEN`, `RUNTIME_PATH_UNSUPPORTED`,
  `RUNTIME_COMMAND_UNSUPPORTED`).
- **No borrowed project knowledge.** A target path named in a runtime-fixed
  body is valid only when the skill declares it, through `source_paths` or a
  declared evidence anchor; anything else is blocking
  (`RUNTIME_PROJECT_CLAIM_UNSUPPORTED`).

Because the two classes are earned differently, they are reported
differently: a generated inventory is summarized as *N project skills* plus
*M runtime guides*, never as one combined count of skills "for your project".

## Adversarial Review Record

A deterministic gate proves a contract is well formed, its evidence resolves,
and its obligations are wired to steps. It cannot ask whether the skill would be
picked for a request nobody has written yet, whether the contract survives having
its nouns removed, or whether the command it prescribes really does what the plan
says. Those need a reader - one who did not write the plan.

The reader's answers go in `tasks/TASK-{NNN}/skill-plan-quality-report.json`,
which `validate_plan_review.py` holds against the plan:

```json
{
  "plan": "tasks/TASK-001/skill-generation-plan.json",
  "reviewer": "independent",
  "skills": [
    {
      "name": "testing",
      "unseen_positive_request": {"verdict": "pass", "prompt": "Add a case covering the draft-to-published transition", "note": "Selected on the suite it owns, not on its name"},
      "sibling_request": {"verdict": "pass", "prompt": "Find why the nightly publish job stopped at 3am", "note": "Deferred to the debugger, as the boundary states"},
      "ambiguous_request": {"verdict": "pass", "prompt": "The publish flow is broken, add something that catches it", "note": "Asks which is wanted rather than guessing"},
      "cross_domain_request": {"verdict": "pass", "prompt": "Cover the paid upgrade path end to end", "note": "Takes the suite half, hands the billing half over"},
      "catalog_role_coverage": {"verdict": "pass", "note": "Each obligation is a step, not a sentence"},
      "refusal_branch": {"verdict": "pass", "note": "Refuses to weaken an assertion to force a pass"},
      "verification_realism": {"verdict": "pass", "note": "Ran on the unmodified target and matched the recorded baseline", "executed": ["vendor/bin/phpunit tests/Feature/PageTest.php"]},
      "identity_erasure": {"verdict": "pass", "note": "Still recognisable with every noun removed"}
    }
  ],
  "blockers": []
}
```

Every selected skill is answered on all eight dimensions, once. A `fail` verdict
or a non-empty `blockers` list stops generation. A fixture prompt that names the
skill it expects to win is `REVIEW_PROMPT_TAUTOLOGICAL` - it tests nothing the
plan did not already assert.

**`verification_realism` must list the commands the reviewer actually ran**, and
they must be the ones the plan prescribes. This is not ceremony: in the third
preserved run, a skill's broken verification was found only by the judge that
executed it, and missed by the judge that read it. A command the review judged
from the page is `REVIEW_VERIFICATION_NOT_EXECUTED`.

## Required Structure

```markdown
# Project Profile: [target_name]

## 0. Metadata
- Target path: [absolute or relative path]
- Scanned: [date]
- Task: tasks/TASK-{N}/
- Generator version: [contents of the VERSION file at this generator's root, e.g. 1.4.0 - the single source of the generator's version; never hardcoded, never recalled from the changelog]

## 1. AI Tool Selection (MANDATORY)
- Selected editions: [one or more of: claude, cursor, codex]
- Source: clarifying-interview answer (never assumed)

## 2. PHP Stack
- PHP version: [constraint + resolved] (confidence)
- Framework: [name + version] (confidence)
- Package manager: composer (confidence)
- PSR-4 autoload map: [Namespace => path, ...] (confidence)
- Entry points: [list] (confidence)
- Test tooling: [PHPUnit/Pest + config] (confidence)
- Lint/format: [tool + config] (confidence)
- Static analysis: [PHPStan/Psalm + config] (confidence)

## 3. Architecture
- Pattern: [monolith | modular-monolith | microservices | event-driven] (confidence)
- Layering/DDD: [layered | hexagonal | ports-and-adapters | none-detected] (confidence)
- Service/module boundaries: [list] (confidence)
- Communication: [HTTP | messaging | RPC | none] (confidence)

### 3.1 Framework-Specialty Signals (drives conditional skill generation)
Each line is a scannable, generalized signal (never named after a specific framework unless that IS the evidence) - see `skill-forge/references/php-specialty-skills.md` for the full mapping from signal to generated skill:
- ORM / data-access pattern: [ORM name+version | query-builder | plain PDO] (confidence) | none
- DB migration tooling: [tool + migrations dir] (confidence) | none
- Async/queue mechanism: [queued jobs | message bus | scheduled async work + real transport] (confidence) | none
- Event listener/subscriber/observer pattern: [present + real classes/dirs] (confidence) | none
- Multi-channel notification delivery: [mail/database/broadcast/SMS notification classes] (confidence) | none
- In-app caching strategy (beyond a bare cache integration): [cache-aside usage, tags, model/query caching] (confidence) | none
- File/object storage abstraction (beyond a bare storage integration): [disk config, signed URLs, upload handling] (confidence) | none
- Auth/authorization scaffolding: [session/cookie auth + Policy/Gate/Voter-style authorization layer] (confidence) | none
- Form/validator design: [dedicated Form/Request/DTO + Validator layer with custom constraints] (confidence) | none
- Admin/back-office panel: [package + wiring] (confidence) | none
- Declarative API resource framework: [package + wiring, e.g. attribute/annotation-driven REST layer] (confidence) | none
- Custom console commands: [count + real command classes] (confidence) | none
- Repository/data-access layer: [present + real classes/dirs] (confidence) | none
- DI container configuration style: [declarative config file (services.yaml/services.php) | code-driven bindings only] (confidence)
- Test data factories/fixtures: [present + real classes/dirs] (confidence) | none
- Package vs. application: [distributable Composer library | deployed application] (confidence)

### 3.2 Frontend Presence (drives the conditional frontend skill group)
- Rendering/templating layer: [Blade | Twig | plain PHP templates | none] (confidence)
- Frontend asset build: [bundler + real config, e.g. Vite/Mix/Webpack] (confidence) | none
- Verdict: [frontend skill group applies | no UI surface - skip frontend skills] (confidence)

## 4. Integrations (per category; only those with evidence)
- Payment: [package + how wired] (confidence) | none
- Messaging/Queue: [...] | none
- Search: [...] | none
- Cache: [...] | none
- Object storage: [...] | none
- Email/SMS: [...] | none
- Auth/Identity: [...] | none
- Observability: [...] | none
- Feature flags / CDN / ML-AI / secondary DB: [...] | none

## 5. Infrastructure & Ops
- Containers: [Docker/Compose/K8s + files] (confidence) | none
- CI/CD: [provider + config files] (confidence) | none
- IaC: [Terraform/etc + files] (confidence) | none
- Deployment target: [...] (confidence) | unknown

## 6. Security & Compliance
- Auth pattern: [session | token | OAuth | SAML | none-detected] (confidence)
- Secrets handling: [.env usage, vault, ...] (confidence)
- Security tooling: [composer audit, taint analysis, ...] (confidence) | none
- Compliance mentions: [GDPR/PCI/HIPAA textual mentions only] | none

## 7. Conventions
- Code style: [tool + rules] (confidence)
- Git hooks: [captainhook/husky/...] (confidence) | none
- Docs/ADRs: [locations] (confidence) | none

## 8. Domain & Behavioral Contract (from domain-behavior-scanner)

### 8.1 Project Purpose & Domain Vocabulary
- Purpose: [short factual description] (confidence; source type - path:L#)
- [term]: [evidenced meaning] (confidence; source type - path:L#)

### 8.2 Project Sources of Truth
- [area]: [source/location] - authority: [what it governs] (confidence; source type - path:L#)
- Contradictions: [sources that disagree] (confidence; source types + paths) | none

### 8.3 Core Modules & Domain Model
- [module]: [evidenced responsibility + paths] (confidence; source type - path:L#)
- [entity]: [identifier, central relationships, ownership/lifecycle fields, integrity constraints only] (confidence; source type - path:L#)

### 8.4 Business Invariants
- [rule] - scope: [module/entity/workflow] - consequence: [what must remain true] (confidence; source type - path:L#)

### 8.5 Lifecycles & Transitions
- Statuses discovered: [entity -> statuses] (confidence; source type - path:L#)
- Confirmed transition: [entity: from -> to] - allowed when: [...] - forbidden when: [...] - side effects: [...] (confidence; source type - path:L#)
- A status list MUST NOT be presented as a transition graph without transition evidence.

### 8.6 Roles & Permissions
- [role/principal] -> [action] on [subject/resource] - restrictions: [...] - enforcement points: [...] (confidence; source type - path:L#)
- Completeness: [complete for named surface | partial | unknown] (confidence; source type - path:L#)

### 8.7 Audit Obligations
- [event] - fields required: [...] - retention/immutability: [...] (confidence; source type - path:L#) | none

### 8.8 High-Risk / Forbidden Workflows
- [workflow/action] - risk indicator: [...] - documented approval/checks: [...] (confidence; source type - path:L#) | none
- Risk indicators MUST NOT invent severity, owners, or approval requirements.

### 8.9 Critical QA / Regression Scenarios
- [scenario] - given/when/then: [...] - protected rule: [...] (confidence; source type - test/spec path:L#) | none

### 8.10 Known Risks & Incident Lessons
- [sanitized lesson] - prevention rule: [...] (confidence; source type - path:L#) | none

### 8.11 Domain Skill Candidates
- `[skill-name]` - bounded context: [...] - coherent rules covered: [...] - distinct review purpose: [...] (confidence; source paths)
- none

## 9. Research Notes (from stack-researcher)
- [Dependency: current version, key official-doc practices, source URL]

## 10. Open Items
- [Anything still `unknown` after the interview]

## 11. Generation Notes
- Pre-existing accelerator in target: [yes/no + which folders]

### 11.1 Skills To Generate
Inventory is evidence-gated across all categories. There are no fixed category counts. The memory quartet is the sole exception because the generated runtime always exists.

- Plan file: `tasks/TASK-{N}/skill-generation-plan.json`
- Selected: one line per skill: `[name]` (`[category]`) - `[necessity rationale]` - evidence: `[EV ids and target-relative paths]`.
- Rejected candidates: one line per considered catalog candidate: `[name]` - skipped because `[selection trigger or required evidence not satisfied]`.
- Runtime-fixed: `memory-bank`, `project-brain`, `checkpoint`, `memory` - justified by the always-installed memory runtime; each still has its own complete contract.
- Inventory check: `[skills.length]` selected entries = `[skills.length]` complete JSON contracts.

### 11.2 Agents & Commands Preview
- Skill count breakdown: derive all category counts from the JSON inventory and sum them to `skills.length`; categories with zero entries remain zero. Do not use baseline constants.
- Agents: one per skill listed in 11.1, generated only for selected editions with an agent layer (Claude: full frontmatter; Cursor: reduced frontmatter; Codex: none) -> state the arithmetic explicitly, e.g. "[total] skills x [E] agent-carrying selected editions = [total*E] agents".
- Commands: one per agent, generated only for selected editions with a command layer (Claude, Cursor; Codex invokes skills directly by name, no command layer) -> state the resulting count explicitly.
- If Codex is among the selected editions, say so plainly: "Codex: skills only, no agents or commands."

### 11.3 Non-PHP Neighbors (integration contracts only)
- [list] | none

## 12. Memory Bank Preview

One shared memory layer will be created at the target root, spanning two roots: `memory-bank/` (`README.md`, `INDEX.md`, `.memory-counter`, `templates/chunk.md`, `local/.gitkeep`, `chunks/`, and the dependency-free context-brain runtime `scripts/context.py` + `scripts/brain_runtime.py` + `scripts/context_retrieval.py` + `scripts/validate.py`) and the governed `project-brain/` control plane skeleton (`PROTOCOL.md`, schemas, record/control templates, config with the target's framework slug in `runtime.json`, empty indexes, and `.gitkeep`-held record directories). The automatic working-memory hooks `hook-forge` generates call this runtime; the memory quartet skills (`memory-bank`, `project-brain`, `checkpoint`, `memory`) operate it. `memory-seed` seeds one chunk per **cohesive durable concept** below, composed only from `confirmed` facts - never an `inferred` or `unknown` one. A chunk may group tightly related facts (for example, one invoice-lifecycle concept containing its confirmed statuses, transitions, guards, and audit consequence) instead of producing one tiny chunk per line. It MUST link to canonical sources rather than copying full specs, schemas, permission matrices, test inventories, or incident reports. This table is the authoritative seed plan: `memory-seed` MUST produce this same set of chunks (same count, same concepts, same sources) at generation time; any difference is drift and must be flagged.

| Planned ID | Title | Type | Source |
| --- | --- | --- | --- |
| MEM-0001 | [short title of the fact] | [architecture\|constraint\|convention\|decision\|domain\|integration\|operations] | [file:line] |
| MEM-0002 | ... | ... | ... |

**Chunks to be seeded:** [count]

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Consumption Contract (what each forge reads)

- `policy-forge`: sections 1, 2, 3, 6, 7, and confirmed high-value rules from section 8 -> the target's `AGENTS.md`/`DOD.md`/`GOLDEN-PRINCIPLES.md`/`STABILIZATION.md`.
- `skill-forge`: `skill-generation-plan.json` plus the exact profile/evidence slices referenced by each contract; it authors one skill or a small sibling group at a time.
- `agent-forge`/`command-forge`: the final skill list from `skill-forge` + section 1 (which editions); section 11.2's counts are what the user was shown to expect.
- `hook-forge`: sections 2, 4, 5, 6 + section 1 -> the four enforcement hooks tuned to real tooling/risks plus the two working-memory hooks calling `memory-bank/scripts/context.py`, for the selected editions only.
- `memory-seed`: section 12 -> the authoritative, pre-reviewed list of cohesive concepts to seed, each already cited to canonical source files; plus section 2's confirmed framework -> the framework slug written into `project-brain/config/runtime.json` (`generic` when none is confirmed).

## Validation Rules (profile-synthesizer self-checks)

- Section 1 MUST list at least one edition and MUST cite the interview as its source (never assumed).
- Every line in sections 2-8 (including 3.1 and 3.2) MUST carry a confidence tag; every section 8 finding MUST also carry source type.
- Section 8.5 MUST distinguish discovered statuses from evidenced transitions; section 8.6 MUST state permission-matrix completeness; section 8.8 MUST distinguish risk indicators from documented severity/approval.
- Section 11.1 MUST equal JSON `skills[]` exactly. Every selected candidate satisfies its reference contract; every rejected candidate records the missing trigger/evidence. Only the memory quartet is unconditional.
- JSON MUST use schema 1.2 and satisfy the typed capability, procedure,
  verification, provider-safety, path, invariant, evidence-anchor, routing-case,
  flow, ownership, reciprocal-routing, uniqueness, reference, normalized-glob,
  canonical-path, and completeness rules above. Grouped summaries are invalid.
- Section 11.2's agent/command counts MUST be arithmetically consistent with `skills.length`, dynamic category counts, and section 1's selected editions.
- Section 12 MUST list cohesive durable concepts composed only from `confirmed` facts across sections 2-8, MUST cite canonical sources, and MUST NOT include any `inferred`/`unknown` fact, raw incident data, customer data, or full copied source documents.
- No secrets or credential values may appear anywhere in the profile.
- A `confirmed` integration MUST cite runtime wiring, not merely a `composer.json` entry.
