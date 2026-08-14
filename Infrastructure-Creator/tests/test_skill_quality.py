#!/usr/bin/env python3
"""Regression tests for deterministic semantic skill-quality validation."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
VALIDATOR_PATH = (
    ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_skill_quality.py"
)
CASES_PATH = ROOT / "tests/fixtures/skill-quality/cases.json"
SPEC = importlib.util.spec_from_file_location("validate_skill_quality", VALIDATOR_PATH)
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)

SHARED_SAFETY = (
    "Never expose credentials or customer payloads. Stop and report sanitized "
    "context when required evidence contains secrets."
)


def skill_markdown(
    name: str,
    description: str,
    purpose: str,
    evidence: str,
    procedure: list[str],
    verification: str,
    outputs: str,
    failure: str,
) -> str:
    steps = "\n".join(f"{index}. {step}" for index, step in enumerate(procedure, 1))
    return f"""---
name: {name}
description: {description}
---

# {name}

## Purpose

{purpose}

## Project Evidence / Inputs

{evidence}

## Procedure / Process

{steps}

## Verification

{verification}

## Outputs

{outputs}

## Guardrails / Failure Handling

{failure}

{SHARED_SAFETY}
"""


class SkillQualityFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="skill-quality-")
        self.base = Path(self.temporary.name)
        self.target = self.base / "target"
        self.skills = self.base / "staging" / "skills"
        self.task_dir = self.base / "tasks" / "TASK-001"
        self.references = self.base / "references"
        self.target.mkdir()
        (self.target / "reports").mkdir()
        self.skills.mkdir(parents=True)
        self.task_dir.mkdir(parents=True)
        self.references.mkdir()
        (self.references / "test-catalog.md").write_text(
            "# Test Catalog\n\n"
            "## Firebase Services\nEvidence scope procedure decision verification "
            "output failure sibling negative.\n\n"
            "## Availability Contract Review\nEvidence scope procedure decision "
            "verification output failure sibling negative.\n\n"
            "## Generic Cache\nEvidence scope procedure decision verification "
            "output failure sibling negative.\n",
            encoding="utf-8",
        )
        self.registry_path = self.references / "candidate-registry.json"
        self.registry_path.write_text(
            json.dumps(
                {
                    "schema_version": "1.0",
                    "catalog_version": "2.5.0",
                    "candidates": [
                        {
                            "id": "firebase-services",
                            "catalog": "test-catalog.md#firebase-services",
                            "category": "integration",
                            "mode": "static",
                        },
                        {
                            "id": "availability-contract-review",
                            "catalog": "test-catalog.md#availability-contract-review",
                            "category": "domain-review",
                            "mode": "static",
                        },
                        {
                            "id": "generic-cache",
                            "catalog": "test-catalog.md#generic-cache",
                            "category": "integration",
                            "mode": "static",
                        },
                    ],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        self.addCleanup(self.temporary.cleanup)

        (self.task_dir / "infra-scan-project-profile.md").write_text(
            "# Approved profile\n", encoding="utf-8"
        )
        self.write_target(
            "config/firebase.php",
            "<?php // Firebase initialization, messaging client, and runtime boundary.\n",
        )
        self.write_target(
            "domain/availability.php",
            "<?php // Availability sampled-date invariant and cancellation transition.\n",
        )
        self.write_target(
            "composer.json",
            '{"require":{"php":"^8.3"},"autoload":{"psr-4":{"App\\\\":"src/"}}}\n',
        )
        self.plan = self.base_plan()
        self.skill_texts = self.good_skill_texts()
        self.write_fixture()

    def write_target(self, relative: str, content: str) -> None:
        path = self.target / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def base_plan(self) -> dict:
        return {
            "schema_version": "1.0",
            "catalog_version": "2.5.0",
            "target_root": ".",
            "profile": "tasks/TASK-001/infra-scan-project-profile.md",
            "evidence": [
                {
                    "id": "firebase-runtime",
                    "path": "config/firebase.php",
                    "source_type": "configuration",
                    "authority": "Declares the Firebase runtime boundary",
                    "confidence": "confirmed",
                    "fingerprint": "sha256:"
                    + hashlib.sha256(
                        (self.target / "config/firebase.php").read_bytes()
                    ).hexdigest(),
                    "supported_claims": [
                        "Firebase initialization and messaging form a runtime boundary"
                    ],
                },
                {
                    "id": "availability-rules",
                    "path": "domain/availability.php",
                    "source_type": "domain code",
                    "authority": "Implements availability state behavior",
                    "confidence": "confirmed",
                    "fingerprint": "sha256:"
                    + hashlib.sha256(
                        (self.target / "domain/availability.php").read_bytes()
                    ).hexdigest(),
                    "supported_claims": [
                        "Availability has sampled-date and cancellation invariants"
                    ],
                },
            ],
            "skills": [
                {
                    "name": "firebase-services",
                    "category": "integration",
                    "kind": "project-derived",
                    "phase": "implementation",
                    "necessity_rationale": "Protect Firebase messaging runtime boundaries.",
                    "selection_gate": {
                        "catalog": "test-catalog.md#firebase-services",
                        "candidate_id": "firebase-services",
                        "candidate": "firebase-services",
                        "conditions": [
                            {
                                "requirement": "Firebase initialization and messaging runtime boundary exists",
                                "evidence_ids": ["firebase-runtime"],
                                "status": "satisfied",
                                "explanation": "Configuration confirms the Firebase messaging boundary.",
                            }
                        ],
                        "distinct_value_from": ["availability-contract-review"],
                    },
                    "triggers": {
                        "positive": ["change Firebase messaging or initialization"],
                        "negative": ["review availability domain behavior"],
                    },
                    "evidence_ids": ["firebase-runtime"],
                    "source_paths": ["config/firebase.php"],
                    "owned_scope": ["Firebase integration transport"],
                    "excluded_scope": ["availability domain behavior"],
                    "required_procedure_roles": [
                        {"role": "inspect", "requirements": ["inspect initialization"]},
                        {"role": "trace", "requirements": ["trace message transport"]},
                        {"role": "failures", "requirements": ["classify provider failures"]},
                    ],
                    "decision_points": [
                        {
                            "question": "Which provider failures may retry?",
                            "branches": ["choose retry only for transient provider failures"],
                        }
                    ],
                    "verification": ["Exercise initialization and messaging release checks."],
                    "output_contract": ["Produce a Firebase boundary report with release evidence."],
                    "failure_handling": ["Stop when provider boundary evidence is unavailable."],
                    "related_skills": ["availability-contract-review"],
                    "nearest_siblings": [
                        {
                            "name": "availability-contract-review",
                            "boundary": "Firebase transport is owned here; availability domain behavior is deferred.",
                        }
                    ],
                    "writes": ["reports/firebase-boundary.md"],
                    "fixed_blocks": [
                        {
                            "id": "safety.no-secrets",
                            "version": "1.0",
                            "content": SHARED_SAFETY,
                        }
                    ],
                },
                {
                    "name": "availability-contract-review",
                    "category": "domain-review",
                    "kind": "domain",
                    "phase": "review",
                    "necessity_rationale": "Protect availability sampled-date and cancellation invariants.",
                    "selection_gate": {
                        "catalog": "test-catalog.md#availability-contract-review",
                        "candidate_id": "availability-contract-review",
                        "candidate": "availability-contract-review",
                        "conditions": [
                            {
                                "requirement": "Availability sampled-date and cancellation invariants exist",
                                "evidence_ids": ["availability-rules"],
                                "status": "satisfied",
                                "explanation": "Domain code confirms sampled-date and cancellation invariants.",
                            }
                        ],
                        "distinct_value_from": ["firebase-services"],
                    },
                    "triggers": {
                        "positive": ["review availability state or cancellation behavior"],
                        "negative": ["change Firebase messaging transport"],
                    },
                    "evidence_ids": ["availability-rules"],
                    "source_paths": ["domain/availability.php"],
                    "owned_scope": ["availability domain behavior"],
                    "excluded_scope": ["Firebase integration transport"],
                    "required_procedure_roles": [
                        {"role": "enumerate", "requirements": ["enumerate sampled dates"]},
                        {"role": "transitions", "requirements": ["trace cancellation transitions"]},
                        {"role": "failures", "requirements": ["inspect partial failures"]},
                    ],
                    "decision_points": [
                        {
                            "question": "Is the transition valid?",
                            "branches": ["reject transitions that violate the sampled-date invariant"],
                        }
                    ],
                    "verification": ["Test sampled dates, cancellation, and partial failure states."],
                    "output_contract": ["Produce an availability invariant review with failing scenarios."],
                    "failure_handling": ["Mark conflicting state authority unresolved and stop approval."],
                    "related_skills": ["firebase-services"],
                    "nearest_siblings": [
                        {
                            "name": "firebase-services",
                            "boundary": "Availability behavior is owned here; Firebase transport is deferred.",
                        }
                    ],
                    "writes": ["reports/availability-review.md"],
                    "fixed_blocks": [
                        {
                            "id": "safety.no-secrets",
                            "version": "1.0",
                            "content": SHARED_SAFETY,
                        }
                    ],
                },
            ],
            "rejected_candidates": [
                {
                    "candidate_id": "generic-cache",
                    "name": "generic-cache",
                    "category": "integration",
                    "reason": "No confirmed cache runtime wiring",
                    "missing_evidence": ["cache initialization and call sites"],
                }
            ],
        }

    def good_skill_texts(self) -> dict[str, str]:
        return {
            "firebase-services": skill_markdown(
                "firebase-services",
                "Use when changing Firebase initialization, messaging, or provider failure handling.",
                "Protect the Firebase messaging runtime boundary established in config/firebase.php.",
                "Read config/firebase.php and identify the initialized Firebase client and transport.",
                [
                    "Inspect initialization before selecting the configured Firebase client.",
                    "Trace message transport from the application adapter to the provider API.",
                    "Classify provider failures as transient, permanent, or configuration faults.",
                    "Choose retry only for transient provider failures; never retry invalid payloads.",
                    "Keep availability-contract-review responsible for availability domain behavior.",
                ],
                "Exercise initialization and messaging release checks with one success, one transient failure, and one invalid payload.",
                "Produce a Firebase boundary report with release evidence and unresolved provider risks.",
                "On unavailable provider documentation, stop the release check and record the missing boundary evidence.",
            ),
            "availability-contract-review": skill_markdown(
                "availability-contract-review",
                "Use when reviewing availability state, sampled dates, cancellation, or partial failures.",
                "Protect availability sampled-date and cancellation invariants encoded in domain/availability.php.",
                "Read domain/availability.php and enumerate its state, permission, and failure rules.",
                [
                    "Enumerate sampled dates and preserve the one-identifier invariant for each result.",
                    "Trace cancellation transitions and reject a forbidden terminal-to-active transition.",
                    "Inspect partial failures without converting successful dates into failed results.",
                    "Reject transitions that violate the sampled-date invariant or bypass permissions.",
                    "Leave Firebase transport analysis to firebase-services rather than expanding domain ownership.",
                ],
                "Test sampled dates, cancellation, and partial failure states across permitted roles.",
                "Produce an availability invariant review with failing scenarios and transition evidence.",
                "If state authority conflicts, mark the invariant unresolved and do not approve the transition.",
            ),
        }

    def write_fixture(self) -> None:
        self.plan_path = self.task_dir / "plan.json"
        self.plan_path.write_text(
            json.dumps(self.plan, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        for name, text in self.skill_texts.items():
            path = self.skills / name / "SKILL.md"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")

    def rewrite(self) -> None:
        for path in self.skills.glob("*/SKILL.md"):
            path.unlink()
        self.write_fixture()

    def codes(self) -> list[str]:
        return [
            diagnostic.code
            for diagnostic in validator.validate(
                self.skills, self.plan_path, self.target, self.registry_path
            )
        ]

    def plan_diagnostics(self) -> list:
        return validator.validate_plan(
            self.plan_path, self.target, self.registry_path
        )

    def use_schema_1_1(self) -> None:
        self.plan["schema_version"] = "1.1"
        ownership_ids = [
            f"skill.{skill['name']}" for skill in self.plan["skills"]
        ]
        for index, skill in enumerate(self.plan["skills"]):
            skill["ownership"] = [
                {
                    "id": ownership_ids[index],
                    "mode": "exclusive",
                    "description": skill["owned_scope"][0],
                    "paths": list(skill["writes"]),
                }
            ]
            sibling = skill["nearest_siblings"][0]
            sibling["role"] = "primary" if index == 0 else "defer"
            sibling["ownership_ids"] = ownership_ids
        self.rewrite()

    def use_schema_1_2(self) -> None:
        self.use_schema_1_1()
        self.plan["schema_version"] = "1.2"
        for skill in self.plan["skills"]:
            name = skill["name"]
            evidence_id = skill["evidence_ids"][0]
            source = skill["source_paths"][0]
            claim = next(
                item["supported_claims"][0]
                for item in self.plan["evidence"]
                if item["id"] == evidence_id
            )
            skill["owned_scope"].append(claim)
            sibling = skill["nearest_siblings"][0]["name"]
            read_only = name == "availability-contract-review"
            skill["capability"] = {
                "mode": "read-only" if read_only else "workspace-write",
                "summary": (
                    "Reviews availability behavior without modifying target files"
                    if read_only
                    else "May write the bounded Firebase boundary report"
                ),
            }
            if read_only:
                skill["writes"] = []
                skill["ownership"][0]["paths"] = [source]
            for decision_index, decision in enumerate(skill["decision_points"]):
                decision["id"] = f"decision-{decision_index + 1}"
            skill["procedure_steps"] = [
                {
                    "id": "trace-confirmed-claim",
                    "action": f"Inspect and trace {claim} from {source}",
                    "evidence_ids": [evidence_id],
                    "path_refs": [source],
                    "decision_refs": [skill["decision_points"][0]["id"]],
                    "expected_outcome": f"The review establishes that {claim}",
                    "failure_branch": "Stop and report the unresolved evidence conflict",
                }
            ]
            skill["verification"] = [
                {
                    "id": "verify-confirmed-claim",
                    "mode": "manual" if read_only else "command",
                    "instruction": f"Assert from the cited source that {claim}",
                    "command": None if read_only else f"php -l {source}",
                    "prerequisites": ["The cited source is available locally"],
                    "safe_scope": "Local static source inspection without network access",
                    "mutation_class": "none",
                    "network_class": "none",
                    "expected_result": f"The check confirms {claim}",
                    "failure_result": f"Any contradiction to {claim} blocks approval",
                    "skip_condition": "The cited source or local PHP executable is unavailable",
                    "skip_reporting": "Report SKIPPED and identify the unverified claim",
                }
            ]
            verification_text = (
                f"{skill['verification'][0]['instruction']}. "
                f"{skill['verification'][0]['expected_result']}."
            )
            self.skill_texts[name] = self.skill_texts[name].replace(
                "## Outputs",
                f"{verification_text}\n\n## Outputs",
            )
            skill["output_contract"] = [
                f"Produce a cited report confirming or rejecting: {claim}"
            ]
            skill["integration_safety"] = {
                "network_policy": "mock-only",
                "test_double_strategy": "Use local fixtures and a fake provider client",
                "environment": "local",
                "authorization_required": False,
                "rollback": "Remove only the bounded report created by this skill",
                "sanitization": "Exclude credentials, customer payloads, and provider errors",
            }
            skill["path_contracts"] = [
                {
                    "path": source,
                    "access": "read",
                    "classification": "required-existing",
                    "evidence_ids": [evidence_id],
                }
            ]
            for write in skill["writes"]:
                skill["path_contracts"].append(
                    {
                        "path": write,
                        "access": "write",
                        "classification": "creatable",
                        "evidence_ids": [evidence_id],
                    }
                )
            skill["evidence_anchors"] = [
                {
                    "evidence_id": evidence_id,
                    "claim": claim,
                    "anchor": f"{source}:L1",
                    "procedure_step_ids": ["trace-confirmed-claim"],
                    "verification_ids": ["verify-confirmed-claim"],
                }
            ]
            skill["routing_cases"] = [
                {
                    "prompt": skill["triggers"]["positive"][0],
                    "expected_primary": name,
                    "permitted_secondary": [],
                    "forbidden_skills": [sibling],
                    "rationale": "The prompt matches this skill's positive scope",
                    "evidence_ids": [evidence_id],
                },
                {
                    "prompt": skill["triggers"]["negative"][0],
                    "expected_primary": sibling,
                    "permitted_secondary": [],
                    "forbidden_skills": [name],
                    "rationale": "The prompt belongs to the adjacent owner",
                    "evidence_ids": [evidence_id],
                },
            ]
        self.plan["critical_invariants"] = [
            {
                "id": f"confirmed.{skill['name']}",
                "statement": next(
                    item["supported_claims"][0]
                    for item in self.plan["evidence"]
                    if item["id"] == skill["evidence_ids"][0]
                ),
                "evidence_ids": list(skill["evidence_ids"]),
                "skill_names": [skill["name"]],
                "assertions": [
                    {
                        "skill_name": skill["name"],
                        "verification_id": "verify-confirmed-claim",
                    }
                ],
            }
            for skill in self.plan["skills"]
        ]
        roster = [
            {
                "skill": skill["name"],
                "agent": f"{skill['name']}-agent",
                "phase": skill["phase"],
                "writes": bool(skill["writes"]),
            }
            for skill in self.plan["skills"]
        ]
        writers = [item["agent"] for item in roster if item["writes"]]
        reviewers = [item["agent"] for item in roster if not item["writes"]]
        self.plan["flow_contracts"] = {
            "roster": roster,
            "flows": [
                {
                    "name": "flow-feature",
                    "required_code_review": None,
                    "stages": [
                        {
                            "phase": "implementation",
                            "agents": writers,
                            "parallel": False,
                            "checkpoint": True,
                        },
                        {
                            "phase": "verification",
                            "agents": reviewers,
                            "parallel": False,
                            "checkpoint": True,
                        },
                    ],
                },
                {
                    "name": "flow-review",
                    "required_code_review": None,
                    "stages": [
                        {
                            "phase": "verification",
                            "agents": reviewers,
                            "parallel": False,
                            "checkpoint": True,
                        }
                    ],
                },
            ],
        }
        self.rewrite()

    def write_agent_wrappers(self) -> Path:
        agents = self.base / "staging" / "agents"
        agents.mkdir()
        for contract in self.plan["skills"]:
            name = contract["name"]
            sibling = contract["nearest_siblings"][0]
            positive = contract["triggers"]["positive"][0]
            negative = contract["triggers"]["negative"][0]
            output = contract["output_contract"][0]
            text = f"""---
name: {name}-agent
description: "Use when {positive}."
invokes: {name}
writes: true
---
# {name} Agent
## Role
Own the bounded request and defer adjacent work.
## Selection examples
Positive: {positive}.
Negative: {negative}; defer to {sibling['name']}.
Boundary: {sibling['boundary']}
## Instructions
Invoke exactly `{name}` and stop.
## Output Format
{output}
"""
            (agents / f"{name}-agent.md").write_text(text, encoding="utf-8")
        return agents

    def write_flow_commands(self) -> Path:
        commands = self.base / "staging" / "commands"
        commands.mkdir()
        routing_lines = []
        review_lines = []
        for contract in self.plan["skills"]:
            name = contract["name"]
            positive = contract["triggers"]["positive"][0]
            negative = contract["triggers"]["negative"][0]
            routing_lines.append(
                f"- `{name}-agent`: select when {positive}; do not select when {negative}."
            )
            review_lines.append(
                f"- `{name}-agent`: write-capable; skip review. Positive scope: "
                f"{positive}; negative scope: {negative}."
            )
        for name, lines in (
            ("flow-feature", routing_lines),
            ("flow-review", review_lines),
        ):
            text = f"""---
flow: {name}
stages:
  - phase: understanding
    agents: [core-agent]
    parallel: false
    checkpoint: false
---
# {name}
## Specialist Routing
{chr(10).join(lines)}
"""
            (commands / f"{name}.md").write_text(text, encoding="utf-8")
        return commands

    def make_duplicate_pattern(self) -> None:
        for entry in self.plan["skills"]:
            name = entry["name"]
            source = entry["source_paths"][0]
            sibling = entry.get("related_skills", entry.get("nearest_siblings"))[0]
            entry["necessity_rationale"] = "Inspect configured behavior safely."
            entry["required_procedure_roles"] = [
                "review context",
                "inspect implementation",
                "apply changes",
            ]
            entry["decision_points"] = ["stop when required evidence is missing"]
            entry["verification"] = "Run configured checks and review changes."
            entry["output_contract"] = "Produce a change report and verification evidence."
            self.skill_texts[name] = skill_markdown(
                name,
                f"Use when performing the planned {name} workflow.",
                "Inspect configured behavior safely and keep the requested change bounded.",
                f"Read {source} before making decisions for this generated workflow.",
                [
                    "Review context and identify the files relevant to the requested change.",
                    "Inspect implementation and compare it with the approved project profile.",
                    "Apply changes in small steps while preserving existing project conventions.",
                    "Run configured checks and review changes for unintended side effects.",
                    f"Stop when required evidence is missing and hand adjacent work to {sibling}.",
                ],
                "Run configured checks and review changes before reporting completion.",
                "Produce a change report and verification evidence for the completed workflow.",
                "Stop when required evidence is missing, preserve existing behavior, and report the blocker.",
            )
        self.rewrite()


class SkillQualityTest(SkillQualityFixture):
    def test_fixture_catalog_covers_required_regressions(self) -> None:
        cases = json.loads(CASES_PATH.read_text(encoding="utf-8"))
        self.assertEqual(
            set(cases),
            {
                "duplicate-shared-template",
                "paraphrased-empty",
                "lexically-unique-empty",
                "invalid-evidence",
                "irrelevant-evidence-reuse",
                "missing-evidence",
                "unsupported-evidence-claim",
                "scope-collision",
                "ambiguous-routing",
                "unjustified-inventory-growth",
                "registry-coverage-bypass",
                "nonconforming-agent-file",
                "good-distinct-shared-safety",
                "deterministic-output",
                "schema-1.2-valid",
                "legacy-publication-ineligible",
                "generic-verification",
                "provider-safety",
                "read-only-mutation",
                "path-contract-classification",
                "critical-invariant-coverage",
                "claim-traceability",
                "evidence-anchor",
                "routing-case-coverage",
                "flow-contract-shape",
            },
        )

    def test_good_distinct_skills_with_shared_approved_safety_pass(self) -> None:
        self.assertEqual(
            self.codes(),
            [
                "LEGACY_PLAN_PUBLICATION_INELIGIBLE",
                "PLAN_SCHEMA_MIGRATION",
            ],
        )

    def test_schema_1_0_is_accepted_with_exactly_one_migration_warning(self) -> None:
        diagnostics = self.plan_diagnostics()
        migration = [
            item for item in diagnostics if item.code == "PLAN_SCHEMA_MIGRATION"
        ]
        self.assertEqual(len(migration), 1)
        self.assertEqual(migration[0].severity, "warning")
        self.assertFalse(any(item.severity == "error" for item in diagnostics))

    def test_schema_1_1_remains_auditable_with_migration_warning(self) -> None:
        self.use_schema_1_1()
        diagnostics = self.plan_diagnostics()
        self.assertIn("PLAN_SCHEMA_MIGRATION", [item.code for item in diagnostics])
        self.assertFalse(any(item.severity == "error" for item in diagnostics))

    def test_schema_1_2_typed_contract_passes_plan_and_authored_validation(self) -> None:
        self.use_schema_1_2()
        self.assertEqual(self.plan_diagnostics(), [])
        self.assertEqual(self.codes(), [])

    def test_legacy_plan_is_audit_only_and_publication_ineligible(self) -> None:
        self.assertFalse(any(item.severity == "error" for item in self.plan_diagnostics()))
        diagnostics = validator.validate(
            self.skills, self.plan_path, self.target, self.registry_path
        )
        legacy = [
            item
            for item in diagnostics
            if item.code == "LEGACY_PLAN_PUBLICATION_INELIGIBLE"
        ]
        self.assertEqual(len(legacy), 1)
        self.assertEqual(legacy[0].severity, "error")

    def test_schema_1_2_rejects_generic_verification_and_bad_step_shape(self) -> None:
        self.use_schema_1_2()
        first = self.plan["skills"][0]
        first["verification"][0]["instruction"] = "Run the appropriate checks."
        first["procedure_steps"][0].pop("expected_outcome")
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("GENERIC_VERIFICATION", codes)
        self.assertIn("PROCEDURE_STEP_INVALID", codes)

    def test_schema_1_2_enforces_provider_safety_and_read_only_language(self) -> None:
        self.use_schema_1_2()
        integration, review = self.plan["skills"]
        integration["integration_safety"]["network_policy"] = "approved-live"
        integration["integration_safety"]["authorization_required"] = False
        integration["integration_safety"]["test_double_strategy"] = (
            "Call the production provider directly"
        )
        review["procedure_steps"][0]["action"] = (
            "Edit availability state and write a corrected transition"
        )
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("PROVIDER_AUTHORIZATION_REQUIRED", codes)
        self.assertIn("PROVIDER_SAFE_DEFAULT_MISSING", codes)
        self.assertIn("READ_ONLY_PROCEDURE_MUTATION", codes)

    def test_schema_1_2_external_side_effect_requires_approved_policy(self) -> None:
        self.use_schema_1_2()
        skill = self.plan["skills"][0]
        skill["capability"]["mode"] = "external-side-effect"
        skill["integration_safety"]["network_policy"] = "forbidden"
        skill["integration_safety"]["authorization_required"] = False
        self.rewrite()
        self.assertIn(
            "EXTERNAL_SIDE_EFFECT_UNAUTHORIZED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_schema_1_2_enforces_path_shape_and_classification(self) -> None:
        self.use_schema_1_2()
        contract = self.plan["skills"][0]["path_contracts"][0]
        contract["path"] = "missing/provider.php"
        contract["classification"] = "required-existing"
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("PATH_EXISTING_MISSING", codes)
        self.assertIn("PATH_CONTRACT_MISSING", codes)

    def test_schema_1_2_enforces_invariant_and_claim_traceability(self) -> None:
        self.use_schema_1_2()
        first = self.plan["skills"][0]
        self.plan["critical_invariants"][0]["statement"] = (
            "A terminal provider failure preserves the quarantined delivery state"
        )
        first["output_contract"] = ["Produce an unrelated summary."]
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("CRITICAL_INVARIANT_COVERAGE", codes)
        self.assertIn("CLAIM_TRACEABILITY_MISSING", codes)

    def test_schema_1_2_enforces_evidence_anchor_and_routing_coverage(self) -> None:
        self.use_schema_1_2()
        first = self.plan["skills"][0]
        first["evidence_anchors"][0]["anchor"] = "wrong.php:L1"
        first["routing_cases"] = first["routing_cases"][:1]
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("EVIDENCE_ANCHOR_LOCATION", codes)
        self.assertIn("ROUTING_CASE_COVERAGE", codes)

    def test_schema_1_2_enforces_flow_contract_shape_and_coverage(self) -> None:
        self.use_schema_1_2()
        self.plan["flow_contracts"]["flows"][0]["stages"][0] = {
            "phase": "implementation",
            "agents": ["unknown-agent"],
            "parallel": False,
            "checkpoint": "yes",
        }
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("FLOW_CONTRACT_INVALID", codes)

    def test_plan_only_gate_passes_before_skills_are_authored(self) -> None:
        command = [
            sys.executable,
            str(VALIDATOR_PATH),
            "--plan",
            str(self.plan_path),
            "--target",
            str(self.target),
            "--registry",
            str(self.registry_path),
            "--plan-only",
            "--json",
        ]
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr or result.stdout)
        self.assertTrue(json.loads(result.stdout)["valid"])

    def test_duplicate_shared_template_fails(self) -> None:
        self.make_duplicate_pattern()
        codes = self.codes()
        self.assertIn("SKILL_SIMILARITY", codes)
        self.assertIn("REPEATED_BLOCK", codes)

    def test_paraphrased_empty_skill_fails_without_word_count_dependency(self) -> None:
        self.skill_texts["firebase-services"] = """---
name: firebase-services
description: Use when Firebase work is requested.
---
# firebase-services
## Purpose
Useful Firebase work.
## Project Evidence / Inputs
Relevant Firebase input.
## Procedure / Process
Review and act.
## Verification
Check the result.
## Outputs
Useful report.
## Guardrails / Failure Handling
Be careful.
"""
        self.rewrite()
        self.assertIn("SKILL_CONTENT_EMPTY", self.codes())

    def test_lexically_unique_but_contract_empty_skill_fails(self) -> None:
        self.skill_texts["firebase-services"] = skill_markdown(
            "firebase-services",
            "Use when broad platform coordination is requested.",
            "Coordinate a comprehensive and thoughtful platform improvement.",
            "Survey assorted documentation and available implementation material.",
            [
                "Catalogue terminology and summarize architectural observations.",
                "Compare alternatives while considering maintainability and clarity.",
                "Describe possible refinements in a detailed narrative.",
                "Discuss tradeoffs and communicate recommendations to stakeholders.",
                "Conclude with a polished summary of the exploration.",
            ],
            "Review the narrative for clarity and completeness.",
            "Produce an extensive strategic memorandum.",
            "Escalate uncertainty and continue gathering context.",
        )
        self.rewrite()
        self.assertIn("SKILL_PROCEDURE_TRACE", self.codes())

    def test_unjustified_inventory_growth_fails_selection_gate(self) -> None:
        invented = copy.deepcopy(self.plan["skills"][0])
        invented["name"] = "invented-cache"
        invented["owned_scope"] = ["invented cache operations"]
        invented["writes"] = ["reports/invented-cache.md"]
        invented["selection_gate"] = {
            "catalog": "test-catalog.md#generic-cache",
            "candidate_id": "generic-cache",
            "candidate": "invented-cache",
            "conditions": [
                {
                    "requirement": "A cache runtime is confirmed",
                    "evidence_ids": [],
                    "status": "satisfied",
                    "explanation": "No cache evidence was found.",
                }
            ],
            "distinct_value_from": [],
        }
        self.plan["skills"].append(invented)
        self.rewrite()
        self.assertIn("SKILL_SELECTION_CONDITION", self.codes())

    def test_registry_catalog_and_exact_disposition_are_enforced(self) -> None:
        registry = json.loads(self.registry_path.read_text(encoding="utf-8"))
        registry["candidates"][0]["catalog"] = "missing.md#missing"
        self.registry_path.write_text(
            json.dumps(registry, indent=2) + "\n", encoding="utf-8"
        )
        self.plan["rejected_candidates"] = []
        self.rewrite()
        codes = self.codes()
        self.assertIn("REGISTRY_CATALOG_INVALID", codes)
        self.assertIn("INVENTORY_CANDIDATE_UNACCOUNTED", codes)

    def test_invalid_evidence_path_fails(self) -> None:
        self.plan["evidence"][0]["path"] = "../outside.php"
        self.rewrite()
        self.assertIn("EVIDENCE_PATH_INVALID", self.codes())

    def test_missing_evidence_id_fails(self) -> None:
        self.plan["skills"][0]["evidence_ids"] = ["not-in-plan"]
        self.rewrite()
        self.assertIn("SKILL_EVIDENCE_UNKNOWN", self.codes())

    def test_stale_evidence_fingerprint_fails(self) -> None:
        self.plan["evidence"][0]["fingerprint"] = "sha256:" + ("0" * 64)
        self.rewrite()
        self.assertIn("EVIDENCE_FINGERPRINT", self.codes())

    def test_supported_claim_must_be_grounded_in_cited_source(self) -> None:
        self.plan["evidence"][0]["supported_claims"] = [
            "Interplanetary teleportation quantum gateway"
        ]
        self.plan["skills"][0]["selection_gate"]["conditions"][0][
            "requirement"
        ] = "Interplanetary teleportation quantum gateway exists"
        self.plan["skills"][0]["selection_gate"]["conditions"][0][
            "explanation"
        ] = "Interplanetary teleportation quantum gateway is configured"
        self.rewrite()
        self.assertIn("EVIDENCE_CLAIM_UNSUPPORTED", self.codes())

    def test_exact_schema_profile_and_evidence_bounds_are_enforced(self) -> None:
        self.plan["schema_version"] = 1
        self.plan["unexpected"] = True
        self.plan["profile"] = "other/TASK-001/profile.md"
        self.plan["evidence"][0].pop("fingerprint")
        self.plan["evidence"][1]["line_range"] = {"start": 1, "end": 99}
        self.rewrite()
        codes = self.codes()
        self.assertIn("PLAN_SCHEMA_VERSION", codes)
        self.assertIn("PLAN_FIELD_UNKNOWN", codes)
        self.assertIn("PROFILE_INVALID", codes)
        self.assertIn("EVIDENCE_FINGERPRINT_MISSING", codes)
        self.assertIn("EVIDENCE_LINE_RANGE", codes)

    def test_irrelevant_evidence_reuse_fails(self) -> None:
        self.plan["evidence"][0]["path"] = "composer.json"
        self.rewrite()
        self.assertIn("SKILL_EVIDENCE_IRRELEVANT", self.codes())

    def test_scope_ownership_collision_fails(self) -> None:
        self.plan["skills"][1]["owned_scope"] = ["Firebase integration transport"]
        self.rewrite()
        self.assertIn("SCOPE_COLLISION", self.codes())
        self.assertIn(
            "SCOPE_COLLISION", [item.code for item in self.plan_diagnostics()]
        )

    def test_ambiguous_positive_routing_fails(self) -> None:
        same = ["change project runtime boundary configuration"]
        self.plan["skills"][0]["triggers"]["positive"] = same
        self.plan["skills"][1]["triggers"]["positive"] = same
        self.plan["skills"][0]["nearest_siblings"][0]["boundary"] = "Adjacent scope differs."
        self.plan["skills"][1]["nearest_siblings"][0]["boundary"] = "Adjacent scope differs."
        self.rewrite()
        self.assertIn("ROUTING_AMBIGUITY", self.codes())
        self.assertIn(
            "ROUTING_AMBIGUITY", [item.code for item in self.plan_diagnostics()]
        )

    def test_schema_1_1_rejects_role_contradictions_and_missing_reciprocals(self) -> None:
        self.use_schema_1_1()
        self.plan["skills"][1]["nearest_siblings"][0]["role"] = "primary"
        self.rewrite()
        self.assertIn(
            "SKILL_SIBLING_ROLE_CONTRADICTION",
            [item.code for item in self.plan_diagnostics()],
        )
        self.plan["skills"][1]["nearest_siblings"] = []
        self.rewrite()
        self.assertIn(
            "SKILL_SIBLING_RECIPROCAL_MISSING",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_schema_1_1_rejects_unknown_ownership_ids(self) -> None:
        self.use_schema_1_1()
        for skill in self.plan["skills"]:
            skill["nearest_siblings"][0]["ownership_ids"] = ["unknown.owner"]
        self.rewrite()
        self.assertIn(
            "SKILL_SIBLING_OWNERSHIP_UNKNOWN",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_schema_1_1_enforces_ownership_modes_ids_and_exclusivity(self) -> None:
        self.use_schema_1_1()
        first, second = self.plan["skills"]
        second["ownership"][0] = copy.deepcopy(first["ownership"][0])
        second["ownership"][0]["mode"] = "read"
        second["ownership"][0]["id"] = "Invalid Owner"
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("SKILL_OWNERSHIP_INVALID", codes)

        self.use_schema_1_1()
        first, second = self.plan["skills"]
        second["ownership"][0] = copy.deepcopy(first["ownership"][0])
        self.rewrite()
        self.assertIn(
            "OWNERSHIP_ID_CONFLICT",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_schema_1_1_shared_ownership_never_allows_overlapping_writes(self) -> None:
        self.use_schema_1_1()
        for skill in self.plan["skills"]:
            skill["writes"] = ["reports/**"]
            skill["ownership"] = [
                {
                    "id": "reports.shared-review",
                    "mode": "shared",
                    "description": "Shared report review boundary",
                    "paths": ["reports/**"],
                }
            ]
            skill["nearest_siblings"][0]["ownership_ids"] = [
                "reports.shared-review"
            ]
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("OWNERSHIP_SHARED_WRITE_CONFLICT", codes)
        self.assertIn("WRITE_SURFACE_COLLISION", codes)

    def test_schema_1_1_composed_ownership_requires_exactly_one_writer(self) -> None:
        self.use_schema_1_1()
        for index, skill in enumerate(self.plan["skills"]):
            skill["writes"] = [
                "shared/**" if index == 0 else "reports/contributor.md"
            ]
            skill["ownership"] = [
                {
                    "id": "gitignore.composed-output",
                    "mode": "composed",
                    "description": "Composed shared output boundary",
                    "paths": ["shared/**"],
                }
            ]
            skill["nearest_siblings"][0]["ownership_ids"] = [
                "gitignore.composed-output"
            ]
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertNotIn("OWNERSHIP_COMPOSER_CONFLICT", codes)

        self.plan["skills"][1]["writes"] = ["shared/contributor.md"]
        self.rewrite()
        self.assertIn(
            "OWNERSHIP_COMPOSER_CONFLICT",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_partial_validation_suppresses_only_missing_planned_files(self) -> None:
        self.plan["skills"][1]["owned_scope"] = [
            "Firebase integration transport"
        ]
        self.rewrite()
        missing = self.skills / "availability-contract-review" / "SKILL.md"
        missing.unlink()
        complete = validator.validate(
            self.skills, self.plan_path, self.target, self.registry_path
        )
        partial = validator.validate(
            self.skills,
            self.plan_path,
            self.target,
            self.registry_path,
            allow_partial_skills=True,
        )
        self.assertIn("SKILL_FILE_MISSING", [item.code for item in complete])
        self.assertNotIn("SKILL_FILE_MISSING", [item.code for item in partial])
        self.assertIn("SCOPE_COLLISION", [item.code for item in partial])
        self.assertEqual(
            [item for item in complete if item.code != "SKILL_FILE_MISSING"],
            partial,
        )

    def test_plan_only_detects_normalized_write_glob_intersection(self) -> None:
        self.plan["skills"][1]["writes"] = ["reports/**"]
        self.rewrite()
        self.assertIn(
            "WRITE_SURFACE_COLLISION",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_partial_cli_is_mutually_exclusive_with_plan_only(self) -> None:
        command = [
            sys.executable,
            str(VALIDATOR_PATH),
            "--skills-dir",
            str(self.skills),
            "--plan",
            str(self.plan_path),
            "--target",
            str(self.target),
            "--registry",
            str(self.registry_path),
            "--plan-only",
            "--allow-partial-skills",
        ]
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 2)
        self.assertIn("not allowed with argument", result.stderr)

    def test_partial_cli_validates_existing_batch_and_complete_plan(self) -> None:
        (self.skills / "availability-contract-review" / "SKILL.md").unlink()
        command = [
            sys.executable,
            str(VALIDATOR_PATH),
            "--skills-dir",
            str(self.skills),
            "--plan",
            str(self.plan_path),
            "--target",
            str(self.target),
            "--registry",
            str(self.registry_path),
            "--allow-partial-skills",
            "--json",
        ]
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        payload = json.loads(result.stdout)
        self.assertEqual(result.returncode, 1, result.stderr or result.stdout)
        self.assertFalse(payload["valid"])
        self.assertIn(
            "LEGACY_PLAN_PUBLICATION_INELIGIBLE",
            [item["code"] for item in payload["diagnostics"]],
        )
        self.assertNotIn(
            "SKILL_FILE_MISSING",
            [item["code"] for item in payload["diagnostics"]],
        )

    def test_contract_inventory_runs_once_per_direct_validation_mode(self) -> None:
        original = validator._validate_contract_inventory
        with mock.patch.object(
            validator, "_validate_contract_inventory", wraps=original
        ) as inventory:
            validator.validate_plan(self.plan_path, self.target, self.registry_path)
            self.assertEqual(inventory.call_count, 1)
        with mock.patch.object(
            validator, "_validate_contract_inventory", wraps=original
        ) as inventory:
            validator.validate(
                self.skills, self.plan_path, self.target, self.registry_path
            )
            self.assertEqual(inventory.call_count, 1)

    def test_contract_similarity_diagnostics_are_field_aware_warnings(self) -> None:
        repeated = (
            "Run the configured narrow verification command and preserve every "
            "failure detail before approving the bounded contract."
        )
        for skill in self.plan["skills"]:
            skill["verification"] = [repeated]
        self.rewrite()
        diagnostics = self.plan_diagnostics()
        matching = [
            item
            for item in diagnostics
            if item.code
            in {"CONTRACT_SIMILARITY_WARN", "CONTRACT_REPEATED_BLOCK"}
        ]
        self.assertEqual(
            {item.code for item in matching},
            {"CONTRACT_SIMILARITY_WARN", "CONTRACT_REPEATED_BLOCK"},
        )
        self.assertTrue(all(item.severity == "warning" for item in matching))

    def test_agent_routing_and_write_flags_follow_contracts(self) -> None:
        agents = self.write_agent_wrappers()
        self.assertFalse(
            any(
                item.severity == "error"
                for item in validator.validate_agent_routing(
                agents,
                self.plan_path,
                self.target,
                require_invokes=True,
                registry_path=self.registry_path,
                )
            )
        )
        path = agents / "firebase-services-agent.md"
        path.write_text(
            path.read_text(encoding="utf-8").replace("writes: true\n", ""),
            encoding="utf-8",
        )
        codes = [
            item.code
            for item in validator.validate_agent_routing(
                agents,
                self.plan_path,
                self.target,
                registry_path=self.registry_path,
            )
        ]
        self.assertIn("AGENT_WRITES_MISMATCH", codes)
        (agents / "rogue.md").write_text(
            "---\nname: rogue-agent\n---\n# Rogue\n", encoding="utf-8"
        )
        codes = [
            item.code
            for item in validator.validate_agent_routing(
                agents,
                self.plan_path,
                self.target,
                registry_path=self.registry_path,
            )
        ]
        self.assertIn("AGENT_ROUTING_NONCONFORMING_FILE", codes)

    def test_specialists_are_conditional_in_generated_flows(self) -> None:
        commands = self.write_flow_commands()
        self.assertFalse(
            any(
                item.severity == "error"
                for item in validator.validate_flow_routing(
                    commands, self.plan_path, self.target, self.registry_path
                )
            )
        )
        path = commands / "flow-feature.md"
        text = path.read_text(encoding="utf-8").replace(
            "agents: [core-agent]", "agents: [firebase-services-agent]"
        )
        path.write_text(text, encoding="utf-8")
        codes = [
            item.code
            for item in validator.validate_flow_routing(
                commands, self.plan_path, self.target, self.registry_path
            )
        ]
        self.assertIn("FLOW_SPECIALIST_UNCONDITIONAL", codes)

    def test_json_cli_is_nonzero_and_byte_deterministic(self) -> None:
        self.make_duplicate_pattern()
        command = [
            sys.executable,
            str(VALIDATOR_PATH),
            "--skills-dir",
            str(self.skills),
            "--plan",
            str(self.plan_path),
            "--target",
            str(self.target),
            "--registry",
            str(self.registry_path),
            "--json",
        ]
        first = subprocess.run(command, capture_output=True, text=True, check=False)
        second = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertNotEqual(first.returncode, 0)
        self.assertEqual(first.stdout, second.stdout)
        self.assertEqual(first.stderr, second.stderr)
        payload = json.loads(first.stdout)
        self.assertFalse(payload["valid"])
        self.assertEqual(
            payload["diagnostics"],
            sorted(
                payload["diagnostics"],
                key=lambda item: (item["severity"], item["code"], item["message"]),
            ),
        )

    def test_plan_completeness_and_unique_identifiers(self) -> None:
        broken = copy.deepcopy(self.plan["skills"][0])
        broken.pop("output_contract")
        broken["name"] = self.plan["skills"][1]["name"]
        self.plan["skills"].append(broken)
        self.plan["evidence"].append(
            {"id": "firebase-runtime", "path": "config/firebase.php"}
        )
        self.rewrite()
        codes = self.codes()
        self.assertIn("SKILL_PLAN_INCOMPLETE", codes)
        self.assertIn("EVIDENCE_ID_DUPLICATE", codes)


if __name__ == "__main__":
    unittest.main()
