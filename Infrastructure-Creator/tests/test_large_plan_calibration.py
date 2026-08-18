#!/usr/bin/env python3
"""A large honest plan must pass, or every threshold here is miscalibrated.

Every blocking rule in `validate_skill_quality.py` was calibrated on plans of
nine to thirteen skills, because that is what our own runs produce. The failure
mode that costs the most is not a missed defect but a false rejection at scale:
a rule that is fair on twelve skills and impossible on thirty-six stops the
generator on exactly the projects that need it, and nothing in the suite would
notice.

So this module builds a thirty-six skill plan that is honest by construction -
distinct domains, distinct evidence, distinct procedure shapes, distinct
verification classes - and asserts it validates with zero blocking diagnostics.
The corpus is synthetic and carries no client name or content; its shape is
taken from what a large real plan looks like, not its subject matter.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import re
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VALIDATOR_PATH = (
    ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_skill_quality.py"
)
SPEC = importlib.util.spec_from_file_location(
    "validate_skill_quality_large", VALIDATOR_PATH
)
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)


# Eighteen subject areas an ordinary PHP application actually has, each with the
# vocabulary that distinguishes it. `noun` names the thing the module owns,
# `verb` the operation, `unit` the record, `rule` the invariant it protects.
DOMAINS = [
    ("catalog", "CatalogItem", "publish", "listing", "a retired listing never returns to the storefront"),
    ("pricing", "PriceBook", "quote", "quote", "a quote is refused once its price book expires"),
    ("checkout", "CheckoutSession", "confirm", "basket", "a basket cannot be confirmed twice"),
    ("payments", "PaymentIntent", "capture", "intent", "an intent is captured at most once"),
    ("refunds", "RefundRequest", "settle", "refund", "a refund never exceeds the captured amount"),
    ("shipping", "ShipmentPlan", "dispatch", "consignment", "a consignment leaves only from a stocked depot"),
    ("inventory", "StockLedger", "reserve", "reservation", "a reservation never drives stock below zero"),
    ("accounts", "AccountProfile", "activate", "profile", "a suspended profile cannot be activated in place"),
    ("permissions", "AccessRule", "grant", "grant", "a grant outlives no membership that carries it"),
    ("audit", "AuditTrail", "record", "entry", "an entry is append-only once written"),
    ("notifications", "NoticeQueue", "deliver", "notice", "a notice is delivered once per recipient"),
    ("scheduling", "SlotCalendar", "book", "slot", "two bookings never share one slot"),
    ("documents", "DocumentBundle", "seal", "bundle", "a sealed bundle rejects further pages"),
    ("imports", "ImportBatch", "ingest", "batch", "a batch is ingested exactly once per checksum"),
    ("exports", "ExportJob", "render", "job", "an export never leaves a partial file in place"),
    ("search", "SearchIndex", "reindex", "document", "a document leaves the index when it is withdrawn"),
    ("reporting", "ReportPeriod", "close", "period", "a closed period is never reopened silently"),
    ("support", "TicketThread", "escalate", "thread", "an escalated thread keeps its original author"),
]

# Procedure shapes, so structurally identical contracts do not appear thirty-six
# times over. Writers and reviewers draw from different pools: a read-only skill
# may not carry write-oriented language, and a skill that writes must be
# verified by something that exercises what it produced, not by a text search.
WRITER_SHAPES = [
    (("locate", "derive", "apply", "verify"), 2),
    (("locate", "compare", "decide", "apply", "verify"), 2),
    (("locate", "enumerate", "decide", "apply", "record", "verify"), 3),
]
REVIEW_SHAPES = [
    (("locate", "derive", "verify"), 1),
    (("locate", "enumerate", "derive", "decide", "verify"), 3),
    (("locate", "compare", "decide", "verify"), 2),
]

STEP_ACTIONS = {
    "locate": "Open {path} and locate the {unit} the {noun} {verb} request names",
    "derive": "Derive the {verb} constraints for {noun} from {path}",
    "compare": "Compare the {unit} in {path} against the {rule_short} it must hold",
    "enumerate": "Enumerate every {unit} {path} exposes before choosing one to {verb}",
    "decide": "Decide which {verb} branch {path} supports for this {unit}",
    "apply": "Apply the {verb} change to {noun} inside {path} only",
    "record": "Record the {unit} identifiers {path} reports after the {verb}",
    "verify": "Run the {module} check for the {noun} {verb} and read its result against {path}",
}
STEP_OUTCOMES = {
    "locate": "The {unit} the {noun} {verb} names is identified in {path}",
    "derive": "The {verb} constraints are stated in the terms {path} uses",
    "compare": "Every difference from {path} is written down",
    "enumerate": "Each {unit} {path} exposes is listed once",
    "decide": "One {verb} branch is chosen and its reason is stated",
    "apply": "{path} carries the {verb} change and nothing else does",
    "record": "The reported {unit} identifiers are captured verbatim",
    "verify": "The {module} check has run and its output is recorded",
}
STEP_FAILURES = {
    "locate": "Stop and report that {path} holds no such {unit}",
    "derive": "Stop and report the {verb} constraint {path} leaves undefined",
    "compare": "Stop and report the difference {path} cannot explain",
    "enumerate": "Stop and report the {unit} that {path} exposes ambiguously",
    "decide": "Stop and report that no {verb} branch is evidenced in {path}",
    "apply": "Revert {path} and report the {verb} that could not be applied",
    "record": "Stop and report the identifiers the {verb} did not return",
    "verify": "Report SKIPPED with the {module} dependency that is unavailable",
}


def build_corpus(
    root: Path, collapsed: bool = False, project_shape: str = "modular"
) -> dict:
    """Write a synthetic target and return a thirty-six skill schema 1.4 plan.

    `collapsed` produces the negative twin: the same plan with every catalog
    obligation discharged by one step, which is the shape an externally authored
    plan of this size actually had.

    `project_shape` selects what kind of project the plan describes. One shape proves
    only that the gate admits one shape; the rules that never fire on a modular
    application - shared ownership, provider capability, an authorized network
    policy - would be free to stay miscalibrated forever.

      modular   many small subject areas, exclusive ownership throughout
      tenant    a central store and a per-tenant one, sharing paths
      provider  outbound integrations under an approved sandbox policy
    """
    target = root / "target"
    task = root / "tasks/TASK-001"
    target.mkdir(parents=True)
    task.mkdir(parents=True)
    (task / "infra-scan-project-profile.md").write_text(
        "# Approved profile\n", encoding="utf-8"
    )

    evidence: list[dict] = []
    skills: list[dict] = []
    invariants: list[dict] = []
    roster: list[dict] = []
    registry_candidates: list[dict] = []

    for index in range(36):
        module, noun, verb, unit, rule = DOMAINS[index // 2]
        half = index % 2
        # Two skills per subject area: the one that changes it and the one that
        # reviews it. They own different paths and defer to each other.
        role = "implementation" if half == 0 else "review"
        name = f"{module}-{'workflow' if half == 0 else 'review'}"
        pool = WRITER_SHAPES if half == 0 else REVIEW_SHAPES
        # Named for what it is: the procedure's shape, not the project's. The
        # first version of this called it `shape` and silently shadowed the
        # parameter, so every project shape built the same plan and three tests
        # passed while proving nothing.
        procedure_shape, decision_count = pool[(index // 2) % len(pool)]
        verification_class = (
            "command" if half == 0 else ("search", "manual")[(index // 2) % 2]
        )
        source = (
            f"src/{noun}/{noun}Service.php"
            if half == 0
            else f"policy/{noun}Policy.php"
        )
        text = (
            f"<?php\n"
            f"// {noun} {role}: {rule}.\n\n"
            f"final class {noun}{'Service' if half == 0 else 'Policy'}\n"
            f"{{\n"
            f"    public function {verb}{unit.capitalize()}(string ${unit}Id): void\n"
            f"    {{\n"
            f"        $this->{module}Gateway->{verb}(${unit}Id);\n"
            f"    }}\n"
            f"}}\n"
        )
        path = target / source
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

        evidence_id = f"EV-{index:04d}"
        claim = (
            f"{noun} {verb} runs through the {module} gateway and {rule}"
            if half == 0
            else f"{noun} policy refuses a {verb} when {rule}"
        )
        evidence.append(
            {
                "id": evidence_id,
                "path": source,
                "source_type": "domain code" if half == 0 else "policy code",
                "authority": (
                    f"Implements the {module} {verb} path for a {unit}"
                    if half == 0
                    else f"Decides which {module} {verb} a {unit} may take"
                ),
                "confidence": "confirmed",
                "fingerprint": "sha256:" + hashlib.sha256(text.encode()).hexdigest(),
                "supported_claims": [claim],
            }
        )

        fields = {
            "path": source,
            "noun": noun,
            "verb": verb,
            "unit": unit,
            "module": module,
            "rule_short": rule,
        }
        steps = []
        for position, archetype in enumerate(procedure_shape):
            steps.append(
                {
                    "id": f"{archetype}-{module}-{half}",
                    "action": STEP_ACTIONS[archetype].format(**fields),
                    "evidence_ids": [evidence_id],
                    "path_refs": [source],
                    "decision_refs": [
                        f"decide-{module}-{half}-{position % decision_count}"
                    ],
                    "expected_outcome": STEP_OUTCOMES[archetype].format(**fields),
                    "failure_branch": STEP_FAILURES[archetype].format(**fields),
                }
            )
        decisions = [
            {
                "id": f"decide-{module}-{half}-{number}",
                "question": (
                    f"Which {verb} branch does the {unit} take at stage {number}?"
                ),
                "branches": [
                    f"proceed with the evidenced {module} {verb}",
                    f"stop and report the unevidenced {unit} state",
                ],
            }
            for number in range(decision_count)
        ]

        roles = [
            f"locate-the-{module}-{unit}",
            f"derive-{verb}-constraints-for-{noun}",
            f"verify-the-{module}-outcome",
        ]
        step_ids = [item["id"] for item in steps]
        if collapsed:
            wiring = [[step_ids[0]] for _ in roles]
        else:
            wiring = [[step_ids[position % len(step_ids)]] for position in range(len(roles))]
        required_roles = [
            {
                "role": role_name,
                "requirements": [
                    f"Read {source} before stating anything about the {unit}"
                    if position == 0
                    else f"Turn {rule} into a concrete {verb} condition"
                    if position == 1
                    else f"Read the {module} check output rather than assuming it"
                ],
                "evidence_ids": [evidence_id],
                "procedure_step_ids": wiring[position],
            }
            for position, role_name in enumerate(roles)
        ]

        writes = [f"src/{noun}/**"] if half == 0 else []
        capability_mode = "workspace-write" if half == 0 else "read-only"
        ownership_mode = "exclusive"
        network_policy = "forbidden"
        environment = "local"
        authorization_required = False
        if project_shape == "tenant" and half == 0:
            # A central store and a per-tenant one write the same tree through
            # different halves of it; the plan says so rather than pretending
            # the paths do not meet.
            ownership_mode = "shared" if index % 4 == 0 else "exclusive"
        if project_shape == "provider" and half == 0:
            capability_mode = "external-side-effect"
            network_policy = "sandbox-with-approval"
            environment = "sandbox"
            authorization_required = True
        if verification_class == "command":
            check = {
                "id": f"run-{module}-{half}-check",
                "mode": "command",
                "instruction": (
                    f"Run the focused {module} test for the {noun} {verb}"
                ),
                "command": f"vendor/bin/phpunit tests/{noun}Test.php",
                "baseline": {
                    "command": f"vendor/bin/phpunit tests/{noun}Test.php",
                    "observed": f"exit 0; {index + 3} assertions for {noun}",
                    "outcome": "passing",
                },
                "prerequisites": [f"The {module} suite dependencies are installed"],
                "safe_scope": f"Local {module} test process with no provider access",
                "mutation_class": "none",
                "network_class": "none",
                "expected_result": (
                    f"The {noun} case reports the {verb} of the {unit} "
                    "the evidence describes"
                ),
                "failure_result": (
                    f"A differing {verb} outcome for {noun} blocks completion"
                ),
                "skip_condition": f"vendor/bin/phpunit or the {module} suite is absent",
                "skip_reporting": f"Report SKIPPED naming the unverified {noun} {verb}",
            }
        elif verification_class == "search":
            check = {
                "id": f"find-{module}-{half}-callsite",
                "mode": "command",
                "instruction": (
                    f"Search {source} for the {noun} {verb} call under review"
                ),
                "command": f"grep -n {verb} {source}",
                "baseline": {
                    "command": f"grep -n {verb} {source}",
                    "observed": f"exit 0; the {verb} call in {source}",
                    "outcome": "passing",
                },
                "prerequisites": [f"{source} is present in the checkout"],
                "safe_scope": f"Read-only literal search of {source}",
                "mutation_class": "none",
                "network_class": "none",
                "expected_result": (
                    f"{source} answers with the {noun} {verb} call for the {unit}"
                ),
                "failure_result": f"No {verb} call in {source} blocks completion",
                "skip_condition": f"{source} is absent from the checkout",
                "skip_reporting": f"Report SKIPPED naming the missing {source}",
            }
        else:
            check = {
                "id": f"read-{module}-{half}-outcome",
                "mode": "manual",
                "instruction": (
                    f"Read {source} and confirm the {unit} state the "
                    f"{noun} {verb} leaves"
                ),
                "command": None,
                "baseline": None,
                "prerequisites": [f"{source} is available locally"],
                "safe_scope": f"Static reading of {source} with no execution",
                "mutation_class": "none",
                "network_class": "none",
                "expected_result": (
                    f"{source} leaves the {unit} in the {noun} {verb} state stated"
                ),
                "failure_result": f"Any other {unit} state in {source} blocks approval",
                "skip_condition": f"{source} cannot be read",
                "skip_reporting": f"Report SKIPPED naming the unread {source}",
            }

        sibling_index = index + 1 if half == 0 else index - 1
        sibling_module, sibling_noun, _, _, _ = DOMAINS[sibling_index // 2]
        sibling = f"{sibling_module}-{'workflow' if sibling_index % 2 == 0 else 'review'}"
        ownership_id = f"{module}.{'change' if half == 0 else 'policy'}"
        sibling_ownership = (
            f"{sibling_module}.{'change' if sibling_index % 2 == 0 else 'policy'}"
        )

        skills.append(
            {
                "name": name,
                "category": "domain-review" if half == 1 else "specialty",
                "kind": "domain-review" if half == 1 else "project-adapted",
                "phase": "verification" if half == 1 else "implementation",
                "capability": {
                    "mode": capability_mode,
                    "summary": (
                        (
                            f"May call the {module} provider sandbox for a {unit}"
                            if project_shape == "provider"
                            else f"May change the {module} {verb} path under src/{noun}"
                        )
                        if half == 0
                        else f"Reads the {module} policy and reports without writing"
                    ),
                },
                "necessity_rationale": (
                    f"{claim}, so the {module} {'change' if half == 0 else 'review'} "
                    f"needs its own operational guidance."
                ),
                "selection_gate": {
                    "catalog": f"synthetic-catalog.md#{name}",
                    "candidate_id": name,
                    "candidate": name,
                    "conditions": [
                        {
                            "requirement": (
                                f"An evidenced {module} {verb} path exists for the {unit}"
                            ),
                            "evidence_ids": [evidence_id],
                            "status": "satisfied",
                            "explanation": (
                                f"{source} proves the {module} {verb} and that {rule}."
                            ),
                        }
                    ],
                    "distinct_value_from": [sibling],
                },
                "triggers": {
                    "positive": [
                        f"{verb} a {unit} through the {module} gateway"
                        if half == 0
                        else f"judge whether a {module} {verb} is permitted for a {unit}"
                    ],
                    "negative": [
                        f"judge whether a {module} {verb} is permitted for a {unit}"
                        if half == 0
                        else f"{verb} a {unit} through the {module} gateway"
                    ],
                },
                "evidence_ids": [evidence_id],
                "source_paths": [source],
                "owned_scope": [claim],
                "excluded_scope": [
                    f"{module} policy decisions"
                    if half == 0
                    else f"{module} implementation changes"
                ],
                "ownership": [
                    {
                        "id": ownership_id,
                        "mode": ownership_mode,
                        "description": (
                            f"The {module} {verb} implementation for a {unit}"
                            if half == 0
                            else f"The {module} {verb} policy judgement for a {unit}"
                        ),
                        "paths": writes or [source],
                    }
                ],
                "claim_ids": [f"CLM-{index:04d}"],
                "evidence_dispositions": [],
                "required_procedure_roles": required_roles,
                "procedure_steps": steps,
                "decision_points": decisions,
                "verification": [check],
                "output_contract": [
                    f"Report the {noun} {verb} applied to the {unit} and its evidence"
                    if half == 0
                    else f"Report whether the {noun} {verb} is permitted for the {unit}"
                ],
                "failure_handling": [
                    f"Stop when {source} cannot answer the {verb} question",
                    f"Never widen the change beyond src/{noun}"
                    if half == 0
                    else f"Never edit {source} while reviewing it",
                ],
                "integration_safety": {
                    "network_policy": network_policy,
                    "test_double_strategy": (
                        f"Use the {module} fixtures the target already ships"
                    ),
                    "environment": environment,
                    "authorization_required": authorization_required,
                    "rollback": (
                        f"Revert src/{noun} to its prior state"
                        if half == 0
                        else f"Nothing to roll back; the {module} review writes nothing"
                    ),
                    "sanitization": (
                        f"Exclude credentials and customer {unit} payloads from output"
                    ),
                },
                "path_contracts": (
                    [
                        {
                            "path": source,
                            "access": "read",
                            "classification": "required-existing",
                            "evidence_ids": [evidence_id],
                        }
                    ]
                    + (
                        [
                            {
                                "path": f"src/{noun}/**",
                                "access": "write",
                                "classification": "creatable",
                                "evidence_ids": [evidence_id],
                            }
                        ]
                        if half == 0
                        else []
                    )
                ),
                "evidence_anchors": [
                    {
                        "evidence_id": evidence_id,
                        "claim": claim,
                        "anchor": f"{source}:L2",
                        "procedure_step_ids": [steps[0]["id"]],
                        "verification_ids": [check["id"]],
                    }
                ],
                "routing_cases": [
                    {
                        "prompt": (
                            f"{verb} a {unit} through the {module} gateway"
                            if half == 0
                            else f"judge whether a {module} {verb} is permitted for a {unit}"
                        ),
                        "expected_primary": name,
                        "permitted_secondary": [],
                        "forbidden_skills": [sibling],
                        "rationale": f"The {module} request lands in this owner's scope",
                        "evidence_ids": [evidence_id],
                    },
                    {
                        "prompt": (
                            f"judge whether a {module} {verb} is permitted for a {unit}"
                            if half == 0
                            else f"{verb} a {unit} through the {module} gateway"
                        ),
                        "expected_primary": sibling,
                        "permitted_secondary": [],
                        "forbidden_skills": [name],
                        "rationale": f"The adjacent {module} owner answers that request",
                        "evidence_ids": [evidence_id],
                    },
                ],
                "related_skills": [sibling],
                "nearest_siblings": [
                    {
                        "name": sibling,
                        "role": "primary" if half == 0 else "defer",
                        "ownership_ids": [ownership_id, sibling_ownership],
                        "boundary": (
                            f"{module} {verb} implementation is owned here; the "
                            f"{module} policy judgement is deferred."
                            if half == 0
                            else f"{module} policy judgement is owned here; the "
                            f"{module} {verb} implementation is deferred."
                        ),
                    }
                ],
                "writes": writes,
            }
        )
        invariants.append(
            {
                "id": f"{module}.{half}",
                "statement": claim,
                "evidence_ids": [evidence_id],
                "skill_names": [name],
                "assertions": [
                    {"skill_name": name, "verification_id": check["id"]}
                ],
            }
        )
        roster.append(
            {
                "skill": name,
                "agent": f"{name}-agent",
                "phase": "verification" if half == 1 else "implementation",
                "writes": bool(writes),
            }
        )
        registry_candidates.append(
            {
                "id": name,
                "catalog": f"synthetic-catalog.md#{name}",
                "category": "domain-review" if half == 1 else "specialty",
                "mode": "static",
                "roles": roles,
            }
        )

    writers = [item["agent"] for item in roster if item["writes"]]
    reviewers = [item["agent"] for item in roster if not item["writes"]]
    plan = {
        "schema_version": "1.4",
        "catalog_version": "2.5.0",
        "target_root": str(target),
        "profile": "tasks/TASK-001/infra-scan-project-profile.md",
        "evidence": evidence,
        "skills": skills,
        "rejected_candidates": [],
        "critical_invariants": invariants,
        "flow_contracts": {
            "roster": roster,
            "flows": [
                {
                    "name": "flow-feature",
                    "required_code_review": None,
                    # Write-capable agents are serialized one stage each; only
                    # the read-only reviewers may share a parallel stage.
                    "stages": [
                        {
                            "phase": "implementation",
                            "agents": [agent],
                            "parallel": False,
                            "checkpoint": True,
                        }
                        for agent in writers
                    ]
                    + [
                        {
                            "phase": "verification",
                            "agents": reviewers,
                            "parallel": len(reviewers) > 1,
                            "checkpoint": True,
                        }
                    ],
                },
                {
                    "name": "flow-review",
                    "required_code_review": None,
                    "stages": [
                        {
                            "phase": "verification",
                            "agents": reviewers,
                            "parallel": len(reviewers) > 1,
                            "checkpoint": True,
                        }
                    ],
                },
            ],
        },
    }
    registry = {
        "schema_version": "1.0",
        "catalog_version": "2.5.0",
        "candidates": registry_candidates,
    }
    plan_path = task / "skill-generation-plan.json"
    plan_path.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
    references = root / "references"
    references.mkdir(exist_ok=True)
    (references / "synthetic-catalog.md").write_text(
        "# Synthetic Catalog\n\n"
        + "".join(
            f"## {item['id']}\nEvidence scope procedure decision verification "
            "output failure sibling negative.\n\n"
            for item in registry_candidates
        ),
        encoding="utf-8",
    )
    registry_path = references / "candidate-registry.json"
    registry_path.write_text(json.dumps(registry, indent=2) + "\n", encoding="utf-8")
    return {
        "plan": plan,
        "plan_path": plan_path,
        "registry_path": registry_path,
        "target": target,
    }


class LargePlanCalibrationTest(unittest.TestCase):
    def build(
        self, collapsed: bool = False, project_shape: str = "modular"
    ) -> tuple[list, dict]:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        corpus = build_corpus(
            Path(temporary.name), collapsed=collapsed, project_shape=project_shape
        )
        diagnostics = validator.validate_plan(
            corpus["plan_path"], corpus["target"], corpus["registry_path"]
        )
        return diagnostics, corpus

    def test_the_corpus_is_large_enough_to_calibrate_on(self) -> None:
        _, corpus = self.build()
        self.assertGreaterEqual(len(corpus["plan"]["skills"]), 35)

    def test_an_honest_large_plan_validates_without_a_single_error(self) -> None:
        diagnostics, _ = self.build()
        errors = [item for item in diagnostics if item.severity == "error"]
        self.assertEqual(
            [f"{item.code}: {item.message}" for item in errors[:8]],
            [],
            f"{len(errors)} blocking diagnostics on an honest 36-skill plan",
        )

    def test_the_three_shapes_are_actually_different_plans(self) -> None:
        """Guards the bug that made this suite green while proving nothing.

        The project shape was shadowed by a local of the same name, so all
        three builders produced one plan and three passing tests said the gate
        admits three shapes. Assert the difference, not the pass.
        """
        modes = {}
        for shape in ("modular", "tenant", "provider"):
            _, corpus = self.build(project_shape=shape)
            skills = corpus["plan"]["skills"]
            modes[shape] = (
                {skill["capability"]["mode"] for skill in skills},
                {
                    item["mode"]
                    for skill in skills
                    for item in skill["ownership"]
                },
                {
                    skill["integration_safety"]["network_policy"]
                    for skill in skills
                },
            )
        self.assertIn("shared", modes["tenant"][1])
        self.assertNotIn("shared", modes["modular"][1])
        self.assertIn("external-side-effect", modes["provider"][0])
        self.assertNotIn("external-side-effect", modes["modular"][0])
        self.assertIn("sandbox-with-approval", modes["provider"][2])

    def test_a_tenant_shaped_plan_validates_too(self) -> None:
        """Shared ownership: a central store and a per-tenant one meet."""
        diagnostics, _ = self.build(project_shape="tenant")
        errors = [item for item in diagnostics if item.severity == "error"]
        self.assertEqual([f"{i.code}: {i.message}" for i in errors[:6]], [])

    def test_a_provider_shaped_plan_validates_too(self) -> None:
        """External side effects with an authorized sandbox policy."""
        diagnostics, _ = self.build(project_shape="provider")
        errors = [item for item in diagnostics if item.severity == "error"]
        self.assertEqual([f"{i.code}: {i.message}" for i in errors[:6]], [])

    def test_the_similarity_signal_only_pairs_genuinely_adjacent_skills(self) -> None:
        """Section 5's decision, held at scale: similarity stays a signal.

        Running this gate against an externally authored 39-skill plan produced
        1642 similarity warnings, 62% of them below the error thresholds, with
        unrelated skills scoring 0.21-0.51 - which is why the measurement
        refused to make similarity blocking. Here the same pass produces 54
        warnings on 36 honest skills, and every one names a module's workflow
        beside its own review: the two contracts that really are adjacent.
        """
        diagnostics, _ = self.build()
        warnings = [
            item
            for item in diagnostics
            if item.severity == "warning" and item.code == "CONTRACT_SIMILARITY_WARN"
        ]
        self.assertTrue(warnings)
        for item in warnings:
            modules = re.findall(r"([a-z]+)-(?:workflow|review)", item.message)[:2]
            self.assertEqual(len(modules), 2, item.message)
            self.assertEqual(modules[0], modules[1], item.message)

    def mutate(self, mutation) -> list:
        """Apply one deliberate loss to the honest plan and re-validate.

        A gate is only worth its false-rejection cost if the corresponding
        omission is actually caught. These mutations are the losses the merged
        plan names: a dropped claim, a dropped obligation, evidence passed over
        in silence.
        """
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        corpus = build_corpus(Path(temporary.name))
        plan = corpus["plan"]
        mutation(plan)
        corpus["plan_path"].write_text(
            json.dumps(plan, indent=2) + "\n", encoding="utf-8"
        )
        return [
            item.code
            for item in validator.validate_plan(
                corpus["plan_path"], corpus["target"], corpus["registry_path"]
            )
            if item.severity == "error"
        ]

    def test_dropping_a_skills_claim_reference_is_caught(self) -> None:
        codes = self.mutate(lambda plan: plan["skills"][3].update(claim_ids=[]))
        self.assertIn("SKILL_CLAIM_IDS_INVALID", codes)

    def test_dropping_a_catalog_obligation_is_caught(self) -> None:
        def drop(plan):
            plan["skills"][5]["required_procedure_roles"] = plan["skills"][5][
                "required_procedure_roles"
            ][:1]

        self.assertIn("CATALOG_ROLE_UNCOVERED", self.mutate(drop))

    def test_unwiring_an_obligation_from_its_step_is_caught(self) -> None:
        def unwire(plan):
            plan["skills"][7]["required_procedure_roles"][0]["procedure_step_ids"] = []

        self.assertIn("PROCEDURE_ROLE_CONTRACT_INVALID", self.mutate(unwire))

    def test_dropping_a_recorded_baseline_is_caught(self) -> None:
        def drop(plan):
            for skill in plan["skills"]:
                for check in skill["verification"]:
                    if check["mode"] == "command" and check["baseline"]:
                        check["baseline"] = None
                        return

        self.assertIn("VERIFICATION_BASELINE_MISSING", self.mutate(drop))

    def test_dropping_an_invariants_only_assertion_is_caught(self) -> None:
        def drop(plan):
            plan["critical_invariants"][2]["assertions"] = []

        codes = self.mutate(drop)
        self.assertTrue(
            [code for code in codes if code.startswith("CRITICAL_INVARIANT")],
            codes,
        )

    def test_the_same_plan_collapsed_onto_one_step_is_rejected(self) -> None:
        diagnostics, _ = self.build(collapsed=True)
        codes = [item.code for item in diagnostics if item.severity == "error"]
        self.assertIn("PROCEDURE_ROLE_COLLAPSED", codes)


if __name__ == "__main__":
    unittest.main()
