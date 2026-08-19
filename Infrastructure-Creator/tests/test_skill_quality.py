#!/usr/bin/env python3
"""Regression tests for deterministic semantic skill-quality validation."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import re
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
# A multi-line evidence source, so an anchor can be pointed inside and outside
# the real bounds of the file it cites.
FIREBASE_SOURCE = """<?php
// Firebase initialization, messaging client, and runtime boundary.

final class FirebaseMessagingClient
{
    public function publishReminder(string $token): void
    {
        $this->transport->send($token);
    }
}
"""
# Three real shapes of cited source that a claim has to be gradeable against:
# a whole file that is one word, a single registration line whose subject is
# glued to a PHP member separator, and a config too short to spare a second
# reusable word.  All three are copied from a Symfony target.
PHP_VERSION_SOURCE = "8.2\n"
BUNDLES_SOURCE = """<?php

return [
    FrameworkBundle::class => ['all' => true],
    DoctrineBundle::class => ['all' => true],
];
"""
FLYSYSTEM_SOURCE = """flysystem:
  storages:
    default:
      adapter: 'aws'
      options:
        client: 'exoscale.client'
        bucket: '%env(S3_CLIENT_BUCKET)%'
        prefix: 'website'
"""


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
                    "schema_version": "1.1",
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

    def refingerprint(self, evidence_id: str) -> None:
        """Re-pin an evidence fingerprint after its source file was rewritten."""
        entry = next(
            item for item in self.plan["evidence"] if item["id"] == evidence_id
        )
        entry["fingerprint"] = "sha256:" + hashlib.sha256(
            (self.target / entry["path"]).read_bytes()
        ).hexdigest()

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
                    "reason": (
                        "No cache runtime wiring: config/cache.php is absent and "
                        "no cache client is constructed under app/"
                    ),
                    "missing_evidence": [
                        "cache initialization in config/cache.php",
                        "cache call sites under app/",
                    ],
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

    def use_schema_1_3(self) -> None:
        self.use_schema_1_2()
        self.plan["schema_version"] = "1.3"
        for skill in self.plan["skills"]:
            evidence_id = skill["evidence_ids"][0]
            source = skill["source_paths"][0]
            claim = next(
                item["supported_claims"][0]
                for item in self.plan["evidence"]
                if item["id"] == evidence_id
            )
            decision_id = skill["decision_points"][0]["id"]
            # Three obligations discharged by one step is the collapsed shape
            # the gate now rejects, so the fixture wires them across steps that
            # each carry one.
            skill["procedure_steps"].extend(
                [
                    {
                        "id": "confirm-cited-range",
                        "action": (
                            f"Inspect {source} and confirm the cited range "
                            f"that states {claim}"
                        ),
                        "evidence_ids": [evidence_id],
                        "path_refs": [source],
                        "decision_refs": [decision_id],
                        "expected_outcome": (
                            f"The cited range in {source} states {claim}"
                        ),
                        "failure_branch": "Stop and report the missing cited range",
                    },
                    {
                        "id": "report-boundary-conclusion",
                        "action": (
                            f"Trace {claim} from {source} to the reported conclusion"
                        ),
                        "evidence_ids": [evidence_id],
                        "path_refs": [source],
                        "decision_refs": [decision_id],
                        "expected_outcome": (
                            f"The report states the conclusion drawn from {source}"
                        ),
                        "failure_branch": "Stop and report the unresolved conclusion",
                    },
                ]
            )
            steps = [step["id"] for step in skill["procedure_steps"]]
            skill["required_procedure_roles"] = [
                {
                    "role": role["role"],
                    "requirements": role["requirements"],
                    "evidence_ids": [evidence_id],
                    "procedure_step_ids": [steps[index % len(steps)]],
                }
                for index, role in enumerate(skill["required_procedure_roles"])
            ]
            for check in skill["verification"]:
                check["baseline"] = (
                    None
                    if check["mode"] == "manual"
                    else {
                        "command": check["command"],
                        "observed": "exit 0; no syntax errors detected",
                        "outcome": "passing",
                    }
                )
        self.rewrite()

    def use_schema_1_4(self) -> None:
        self.use_schema_1_3()
        self.plan["schema_version"] = "1.4"
        for skill in self.plan["skills"]:
            evidence_id = skill["evidence_ids"][0]
            skill["claim_ids"] = [f"CLM-{evidence_id}"]
            # Every other skill's evidence sits outside this skill's declared
            # paths, so an honest contract needs no exclusions here.
            skill["evidence_dispositions"] = []
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
                "claim-language-lexicon-only",
                "claim-common-lexicon-only",
                "claim-compound-identifier-calibration",
                "claim-php-separator-calibration",
                "claim-single-word-source-calibration",
                "claim-short-config-calibration",
                "scope-collision",
                "ambiguous-routing",
                "unjustified-inventory-growth",
                "registry-coverage-bypass",
                "nonconforming-agent-file",
                "good-distinct-shared-safety",
                "deterministic-output",
                "schema-1.4-valid",
                "legacy-publication-ineligible",
                "generic-verification",
                "provider-safety",
                "read-only-mutation",
                "path-contract-classification",
                "plan-mutation-step",
                "critical-invariant-coverage",
                "claim-traceability",
                "evidence-anchor",
                "evidence-anchor-resolution",
                "exclusive-ownership-write",
                "verbatim-boundary",
                "routing-case-coverage",
                "flow-contract-shape",
                "skill-body-command-risk",
                "skill-body-command-calibration",
                "vacuous-operational-content",
                "operational-content-calibration",
                "noun-substituted-template",
                "repunctuated-template",
                "template-reuse-calibration",
                "body-path-existence",
                "body-path-calibration",
                "evidence-row-agreement",
                "absence-evidence-calibration",
                "absence-evidence-contradicted",
                "catalog-role-coverage",
                "contract-rendering",
                "contract-rendering-obligation",
                "claim-invariant-lost",
                "evidence-undisposed",
                "large-plan-calibration",
                "golden-plan-shapes",
                "procedure-role-collapse-calibration",
                "procedure-role-collapsed",
                "procedure-role-wiring",
                "review-verification-not-executed",
                "routing-tautology",
                "runtime-baseline-contradicted",
                "runtime-expectation-contradicted",
                "routing-tautology-calibration",
                "scan-secret-covered",
                "scan-surface-unaccounted",
                "verification-baseline-calibration",
                "verification-baseline-contradicted",
                "verification-baseline-missing",
            },
        )

    def test_every_catalogued_code_is_actually_asserted_somewhere(self) -> None:
        """The catalog is an index of covered regressions, not a wish list.

        A named case whose code no test asserts would make the catalog read as
        coverage that does not exist - the same failure mode as a scan that
        records what it found and not what it missed.
        """
        cases = json.loads(CASES_PATH.read_text(encoding="utf-8"))
        suite = "\n".join(
            path.read_text(encoding="utf-8")
            for path in sorted((ROOT / "tests").glob("test_*.py"))
        )
        missing = sorted(
            {
                case["expected"]
                for case in cases.values()
                if case.get("expected") and f'"{case["expected"]}"' not in suite
            }
        )
        self.assertEqual(missing, [])

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

    def test_schema_1_4_typed_contract_passes_plan_and_authored_validation(self) -> None:
        self.use_schema_1_4()
        self.assertEqual(self.plan_diagnostics(), [])
        self.assertEqual(self.codes(), [])

    def test_schema_1_2_is_audit_only_since_the_1_3_migration(self) -> None:
        self.use_schema_1_2()
        diagnostics = self.plan_diagnostics()
        self.assertIn("PLAN_SCHEMA_MIGRATION", [item.code for item in diagnostics])
        self.assertFalse(any(item.severity == "error" for item in diagnostics))
        authored = validator.validate(
            self.skills, self.plan_path, self.target, self.registry_path
        )
        self.assertIn(
            "LEGACY_PLAN_PUBLICATION_INELIGIBLE",
            [item.code for item in authored],
        )

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

    def test_schema_1_4_rejects_generic_verification_and_bad_step_shape(self) -> None:
        self.use_schema_1_4()
        first = self.plan["skills"][0]
        first["verification"][0]["instruction"] = "Run the appropriate checks."
        first["procedure_steps"][0].pop("expected_outcome")
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("GENERIC_VERIFICATION", codes)
        self.assertIn("PROCEDURE_STEP_INVALID", codes)

    def test_schema_1_4_enforces_provider_safety_and_read_only_language(self) -> None:
        self.use_schema_1_4()
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

    def test_schema_1_4_external_side_effect_requires_approved_policy(self) -> None:
        self.use_schema_1_4()
        skill = self.plan["skills"][0]
        skill["capability"]["mode"] = "external-side-effect"
        skill["integration_safety"]["network_policy"] = "forbidden"
        skill["integration_safety"]["authorization_required"] = False
        self.rewrite()
        self.assertIn(
            "EXTERNAL_SIDE_EFFECT_UNAUTHORIZED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_schema_1_4_enforces_path_shape_and_classification(self) -> None:
        self.use_schema_1_4()
        contract = self.plan["skills"][0]["path_contracts"][0]
        contract["path"] = "missing/provider.php"
        contract["classification"] = "required-existing"
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("PATH_EXISTING_MISSING", codes)
        self.assertIn("PATH_CONTRACT_MISSING", codes)

    def test_schema_1_4_enforces_invariant_and_claim_traceability(self) -> None:
        self.use_schema_1_4()
        first = self.plan["skills"][0]
        self.plan["critical_invariants"][0]["statement"] = (
            "A terminal provider failure preserves the quarantined delivery state"
        )
        first["output_contract"] = ["Produce an unrelated summary."]
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("CRITICAL_INVARIANT_COVERAGE", codes)
        self.assertIn("CLAIM_TRACEABILITY_MISSING", codes)

    def test_schema_1_4_enforces_evidence_anchor_and_routing_coverage(self) -> None:
        self.use_schema_1_4()
        first = self.plan["skills"][0]
        first["evidence_anchors"][0]["anchor"] = "wrong.php:L1"
        first["routing_cases"] = first["routing_cases"][:1]
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("EVIDENCE_ANCHOR_LOCATION", codes)
        self.assertIn("ROUTING_CASE_COVERAGE", codes)

    def anchor_case(self, anchor: str) -> list[str]:
        """Point the first skill's only anchor at ``anchor`` and re-validate."""
        self.plan["skills"][0]["evidence_anchors"][0]["anchor"] = anchor
        self.rewrite()
        return [item.code for item in self.plan_diagnostics()]

    def test_evidence_anchor_line_range_must_resolve_inside_the_cited_file(self) -> None:
        """An anchor is bound to the file it cites, exactly like line_range.

        Before this gate existed the anchor was checked for shape only, so
        ``config/firebase.php:L7400-L7480`` over a nine-line file passed with a
        perfectly valid prefix and a perfectly valid format.
        """
        self.use_schema_1_4()
        self.write_target("config/firebase.php", FIREBASE_SOURCE)
        self.refingerprint("firebase-runtime")
        codes = self.anchor_case("config/firebase.php:L7400-L7480")
        self.assertIn("EVIDENCE_ANCHOR_RANGE", codes)
        # The old shape checks were already satisfied; that was the whole miss.
        self.assertNotIn("EVIDENCE_ANCHOR_FORMAT", codes)
        self.assertNotIn("EVIDENCE_ANCHOR_LOCATION", codes)
        # A single line one past the end is caught for the same reason.
        self.assertIn(
            "EVIDENCE_ANCHOR_RANGE", self.anchor_case("config/firebase.php:L11")
        )
        # An inverted range bounds nothing either.
        self.assertIn(
            "EVIDENCE_ANCHOR_RANGE", self.anchor_case("config/firebase.php:L8-L3")
        )
        # Negative control: ranges that really are inside the file resolve.
        for good in (
            "config/firebase.php:L1",
            "config/firebase.php:L2-L10",
            "config/firebase.php:L4-6",
        ):
            self.assertNotIn("EVIDENCE_ANCHOR_RANGE", self.anchor_case(good), good)

    def test_evidence_anchor_symbol_must_occur_in_the_cited_source(self) -> None:
        """``symbol:`` anchors are resolved by text search, not trusted."""
        self.use_schema_1_4()
        self.write_target("config/firebase.php", FIREBASE_SOURCE)
        self.refingerprint("firebase-runtime")
        codes = self.anchor_case(
            "config/firebase.php:symbol:refundOrphanedSettlement"
        )
        self.assertIn("EVIDENCE_ANCHOR_SYMBOL_ABSENT", codes)
        self.assertNotIn("EVIDENCE_ANCHOR_FORMAT", codes)
        # A symbol that only occurs as a fragment of a longer identifier is
        # still absent: `publish` is not `publishReminder`.
        self.assertIn(
            "EVIDENCE_ANCHOR_SYMBOL_ABSENT",
            self.anchor_case("config/firebase.php:symbol:publish"),
        )
        # Negative control: present symbols resolve, including qualified forms
        # whose last segment is what actually lives in the file, and casing
        # that differs from the source.
        for good in (
            "config/firebase.php:symbol:publishReminder",
            "config/firebase.php:symbol:FirebaseMessagingClient",
            "config/firebase.php:symbol:App.Firebase.publishReminder",
            "config/firebase.php:symbol:FirebaseMessagingClient::publishReminder",
            "config/firebase.php:symbol:publishreminder",
        ):
            self.assertNotIn(
                "EVIDENCE_ANCHOR_SYMBOL_ABSENT", self.anchor_case(good), good
            )

    def test_anchor_resolution_reports_unreadable_sources_without_crashing(self) -> None:
        """A source that cannot be read yields a diagnostic, never a traceback."""
        for anchor, prefix in (
            ("config/vanished.php:L1-L4", "config/vanished.php"),
            ("reports:L1", "reports"),
            ("config/vanished.php:symbol:missingHandler", "config/vanished.php"),
        ):
            diagnostics: list = []
            validator._resolve_evidence_anchor(
                "firebase-services", anchor, prefix, self.target, diagnostics
            )
            self.assertEqual(
                [item.code for item in diagnostics],
                ["EVIDENCE_ANCHOR_UNRESOLVABLE"],
                anchor,
            )

    def test_schema_1_4_enforces_flow_contract_shape_and_coverage(self) -> None:
        self.use_schema_1_4()
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

    def set_purpose(self, name: str, purpose: str) -> None:
        original = {
            "firebase-services": "Protect the Firebase messaging runtime "
            "boundary established in config/firebase.php.",
            "availability-contract-review": "Protect availability sampled-date "
            "and cancellation invariants encoded in domain/availability.php.",
        }[name]
        self.assertIn(original, self.skill_texts[name])
        self.skill_texts[name] = self.skill_texts[name].replace(original, purpose)
        self.rewrite()

    def test_a_name_used_as_a_path_is_not_a_circular_purpose(self) -> None:
        """A skill that owns a directory of its own name must be able to say so.

        `memory-bank` cannot describe its purpose without naming
        `memory-bank/chunks/` and `memory-bank/scripts/context.py`, and a
        word-boundary match inside those paths called the honest purpose
        circular.
        """
        self.use_schema_1_4()
        self.set_purpose(
            "firebase-services",
            "Keep the Firebase transport contract in firebase-services/state/ "
            "current by running `firebase-services/scripts/probe.py` against "
            "config/firebase.php.",
        )
        self.assertNotIn("SKILL_CIRCULAR_PURPOSE", self.codes())

    def test_a_purpose_that_restates_its_own_name_is_still_circular(self) -> None:
        self.use_schema_1_4()
        self.set_purpose(
            "firebase-services",
            "Use firebase-services whenever firebase-services work is requested.",
        )
        self.assertIn("SKILL_CIRCULAR_PURPOSE", self.codes())

    def install_evidence_table(self, note: str) -> None:
        """Give both skills the same cited evidence row, verbatim."""
        table = (
            "| ID | Source | Type | Confidence | Note |\n"
            "| --- | --- | --- | --- | --- |\n"
            f"| EV-0007 | `domain/availability.php` | domain code | confirmed "
            f"| {note} |\n"
        )
        for name, text in list(self.skill_texts.items()):
            self.skill_texts[name] = text.replace(
                "\n## Procedure / Process\n", f"\n{table}\n## Procedure / Process\n"
            )
        self.rewrite()

    def test_a_shared_evidence_row_is_a_citation_not_a_repeated_block(self) -> None:
        """Two skills citing one fact must render that row identically.

        The header, its rule, and the row are a byte-identical three-line run,
        which is exactly the shape `REPEATED_BLOCK` reports - but the sameness
        is mandated by the citation, not a reused procedure.
        """
        self.use_schema_1_4()
        self.install_evidence_table(
            "Availability keeps its sampled-date invariant and its cancellation "
            "transition in one guarded block that every citing skill repeats."
        )
        self.assertNotIn("REPEATED_BLOCK", self.codes())

    def test_repeated_prose_around_a_shared_table_is_still_reported(self) -> None:
        """The exemption covers the table, never the prose beside it."""
        self.use_schema_1_4()
        shared = (
            "Hold the written boundary next to the incoming request and mark "
            "each gap you find.\n"
            "Roll the change out in small increments and re-read the touched "
            "file afterwards.\n"
            "Record every deviation from the stated rule that you decide to "
            "accept for now.\n"
        )
        for name, text in list(self.skill_texts.items()):
            self.skill_texts[name] = text.replace(
                "\n## Procedure / Process\n", f"\n{shared}\n## Procedure / Process\n"
            )
        self.install_evidence_table("Availability guards its cancellation transition.")
        self.assertIn("REPEATED_BLOCK", self.codes())

    def test_repeated_plain_table_rows_are_still_reported(self) -> None:
        """A table without evidence citations is ordinary duplicated content."""
        self.use_schema_1_4()
        rows = (
            "| Case | Expected |\n"
            "| --- | --- |\n"
            "| Sampled date repeated inside one request | reject the duplicate "
            "identifier and report the offending index |\n"
            "| Cancellation of an already terminal record | refuse the "
            "transition and keep the recorded terminal state |\n"
            "| Partial provider failure during a batch | keep the succeeded "
            "entries and report only the failed ones |\n"
        )
        for name, text in list(self.skill_texts.items()):
            self.skill_texts[name] = text.replace(
                "\n## Procedure / Process\n", f"\n{rows}\n## Procedure / Process\n"
            )
        self.rewrite()
        self.assertIn("REPEATED_BLOCK", self.codes())

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

    def claim_case(self, claim: str) -> list:
        """Attach ``claim`` to the first evidence entry over a real PHP file.

        The claim is substituted everywhere the selection gate echoes it, so
        the plan stays internally consistent - the only lie left is the
        relationship between the claim and the code it cites, which is exactly
        what a fabricating agent produces.
        """
        self.write_target("config/firebase.php", FIREBASE_SOURCE)
        self.refingerprint("firebase-runtime")
        self.plan["evidence"][0]["supported_claims"] = [claim]
        condition = self.plan["skills"][0]["selection_gate"]["conditions"][0]
        condition["requirement"] = claim
        condition["explanation"] = f"config/firebase.php confirms: {claim}"
        self.rewrite()
        return [
            item
            for item in validator.validate(
                self.skills, self.plan_path, self.target, self.registry_path
            )
            if item.code.startswith("EVIDENCE_CLAIM")
        ]

    def test_claim_made_only_of_php_language_vocabulary_is_not_grounded(self) -> None:
        """Bag-of-words overlap is trivial to satisfy with PHP keywords.

        'class' and 'function' occur in roughly three quarters of all PHP
        files, so two of them were enough to ground an invented claim against
        any cited source.  This claim has four such overlaps and still says
        nothing about the file it cites, so the distinctive-vocabulary tier
        rejects it.
        """
        claim = (
            "The public class exposes a private function that returns a string "
            "value from the configuration array"
        )
        diagnostics = self.claim_case(claim)
        self.assertEqual(
            [("error", "EVIDENCE_CLAIM_UNSUPPORTED")],
            [(item.severity, item.code) for item in diagnostics],
        )
        # The old bag-of-words test was satisfied - that was the whole miss.
        claim_tokens = validator._meaningful_tokens(claim)
        cited_tokens = validator._meaningful_tokens(FIREBASE_SOURCE)
        self.assertGreaterEqual(len(claim_tokens & cited_tokens), 2)

    def test_claim_supported_only_by_common_lexicon_is_warned_about(self) -> None:
        """One tier down: the claim does share a word, but only a boilerplate one.

        'final' is not a PHP keyword the language tier knows, so this claim
        clears the error tier; nothing project-specific in it ('whose', 'each',
        'payload') appears in the cited range.  Measured on real docblock/code
        pairs this tier costs 2.3-3.6% false positives, which is why it warns
        instead of failing the run.
        """
        diagnostics = self.claim_case(
            "The final class is a service handler whose method returns the "
            "result value for each request option payload"
        )
        self.assertEqual(
            [("warning", "EVIDENCE_CLAIM_GENERIC_SUPPORT")],
            [(item.severity, item.code) for item in diagnostics],
        )

    def test_claim_grounded_through_a_compound_identifier_is_accepted(self) -> None:
        """Honest prose names what the code spells as one identifier.

        The cited file never writes 'reminder' on its own - only
        ``publishReminder``.  Without splitting compound identifiers this
        honest claim would score zero distinctive overlap and be rejected, and
        a false rejection here blocks a correct plan.
        """
        self.assertNotIn("reminder", validator._meaningful_tokens(FIREBASE_SOURCE))
        self.assertIn("reminder", validator._cited_vocabulary(FIREBASE_SOURCE))
        self.assertEqual(
            [], self.claim_case("The class publishes each reminder string it receives")
        )

    def claim_case_over(
        self,
        relative: str,
        content: str,
        claim: str,
        line_range: dict | None = None,
    ) -> list:
        """``claim_case`` against an arbitrary cited file and range.

        The three grounding regressions below are about the *source* rather
        than the claim - a one-word file, a PHP member separator, a config too
        short to spare a second word - so the cited path has to move.
        """
        self.write_target(relative, content)
        entry = self.plan["evidence"][0]
        entry["path"] = relative
        if line_range is None:
            entry.pop("line_range", None)
        else:
            entry["line_range"] = line_range
        entry["supported_claims"] = [claim]
        self.refingerprint("firebase-runtime")
        condition = self.plan["skills"][0]["selection_gate"]["conditions"][0]
        condition["requirement"] = claim
        condition["explanation"] = f"{relative} confirms: {claim}"
        self.rewrite()
        return [
            (item.severity, item.code)
            for item in validator.validate(
                self.skills, self.plan_path, self.target, self.registry_path
            )
            if item.code.startswith("EVIDENCE_CLAIM")
        ]

    def test_php_member_separators_split_like_any_other_compound(self) -> None:
        """'::' and '->' join compounds exactly as '.' and '/' do.

        ``_tokens`` keeps ``FrameworkBundle::class`` whole, so a true claim
        naming FrameworkBundle scored zero overlap against the one line that
        registers it - the most common separator in the language reading as
        a fabrication signal.
        """
        vocabulary = validator._cited_vocabulary("FrameworkBundle::class")
        self.assertIn("frameworkbundle", vocabulary)
        self.assertIn("transport", validator._cited_vocabulary("$this->transport"))
        self.assertEqual(
            [],
            self.claim_case_over(
                "config/bundles.php",
                BUNDLES_SOURCE,
                "Project registers FrameworkBundle in every environment",
                {"start": 4, "end": 4},
            ),
        )

    def test_single_word_source_can_still_ground_a_longer_claim(self) -> None:
        """A file whose whole content is "8.2" can offer exactly one word.

        Two overlaps were required of any claim longer than three tokens, so
        `.php-version` - which the stack scanner is contractually told to cite
        as the runtime pin - made every honest claim about it impossible to
        state.  The bar now cannot exceed what the source has to give.
        """
        self.assertEqual({"8.2"}, validator._cited_vocabulary(PHP_VERSION_SOURCE))
        self.assertEqual(
            [],
            self.claim_case_over(
                ".php-version",
                PHP_VERSION_SOURCE,
                "Project PHP runtime version is pinned to 8.2 by .php-version",
            ),
        )
        # The floor is one overlap, never zero: a claim that reuses nothing
        # the one-word file says is still rejected.
        self.assertEqual(
            [("error", "EVIDENCE_CLAIM_UNSUPPORTED")],
            self.claim_case_over(
                ".php-version",
                PHP_VERSION_SOURCE,
                "Project runs on the Node 20 runtime with a Yarn workspace",
            ),
        )

    def test_short_config_grounds_a_claim_on_the_word_it_shares(self) -> None:
        """An eight-line config has one sentence of vocabulary, not two.

        Prose about the storage adapter reuses 'adapter' and paraphrases the
        rest ('storages' -> 'storage'), which is what an honest summary of a
        short file looks like.
        """
        self.assertLess(
            len(validator._cited_vocabulary(FLYSYSTEM_SOURCE)),
            2 * validator.CLAIM_SENTENCE_TOKENS,
        )
        self.assertEqual(
            [],
            self.claim_case_over(
                "config/packages/flysystem.yaml",
                FLYSYSTEM_SOURCE,
                "Media uploads go through a remote storage adapter",
            ),
        )

    def test_relaxed_bar_still_needs_distinctive_overlap(self) -> None:
        """The relaxation is a reachability fix, not a lexicon amnesty.

        A short config offers a low bar, so the two distinctive-vocabulary
        tiers are what keeps a fabricated claim out: this one clears the
        one-word bar three times over, and every word it clears it with
        ('default', 'options', 'prefix') is common lexicon that grounds
        nothing.
        """
        self.assertEqual(
            [("warning", "EVIDENCE_CLAIM_GENERIC_SUPPORT")],
            self.claim_case_over(
                "config/packages/flysystem.yaml",
                FLYSYSTEM_SOURCE,
                "The default options value returns a result for each "
                "configured prefix option",
            ),
        )

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

    def write_into_review_source(self) -> str:
        """Give the writer skill a write into the read-only neighbour's file."""
        writer, reviewer = self.plan["skills"]
        owned = reviewer["source_paths"][0]
        writer["writes"].append(owned)
        writer["path_contracts"].append(
            {
                "path": owned,
                "access": "write",
                "classification": "required-existing",
                "evidence_ids": list(writer["evidence_ids"]),
            }
        )
        return owned

    def test_exclusive_ownership_rejects_a_foreign_writer(self) -> None:
        """Exclusive ownership is exclusive against writes, not only writes.

        WRITE_SURFACE_COLLISION compares ``writes`` with ``writes``, so a
        read-only owner - whose ``writes`` is empty by contract - used to be
        writable by any neighbour that simply declared the owned file.
        """
        self.use_schema_1_4()
        owned = self.write_into_review_source()
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT", codes)
        # The writes-versus-writes comparison is still blind here: that is
        # precisely why the ownership guard has to exist.
        self.assertNotIn("WRITE_SURFACE_COLLISION", codes)
        self.assertIn(
            "OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT",
            [item.code for item in self.plan_diagnostics()],
        )
        conflicts = [
            item.message
            for item in self.plan_diagnostics()
            if item.code == "OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT"
        ]
        self.assertEqual(len(conflicts), 1)
        self.assertIn(owned, conflicts[0])

    def test_exclusive_write_guard_respects_owners_and_ownership_mode(self) -> None:
        """Negative control: legitimate write surfaces stay clean."""
        self.use_schema_1_4()
        # An owner writing its own exclusive paths is the normal case.
        self.assertNotIn(
            "OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT",
            [item.code for item in self.plan_diagnostics()],
        )
        # A non-exclusive zone is governed by the shared/composed rules, so the
        # exclusive guard must not fire on it.
        self.plan["skills"][1]["ownership"][0]["mode"] = "shared"
        self.write_into_review_source()
        self.rewrite()
        self.assertNotIn(
            "OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_declared_precedence_cannot_resolve_a_verbatim_boundary(self) -> None:
        """Precedence divides a partial overlap; it cannot divide identity.

        The reciprocal primary/defer sibling pair that schema 1.2 builds by
        default used to suppress SCOPE_COLLISION and ROUTING_AMBIGUITY even
        when the two contracts were literally the same sentence.
        """
        self.use_schema_1_4()
        first, second = self.plan["skills"]
        second["owned_scope"][0] = first["owned_scope"][0]
        second["ownership"][0]["description"] = first["ownership"][0]["description"]
        shared_trigger = first["triggers"]["positive"][0]
        second["triggers"]["positive"] = [shared_trigger]
        second["routing_cases"][0]["prompt"] = shared_trigger
        self.rewrite()
        diagnostics = self.plan_diagnostics()
        codes = [item.code for item in diagnostics]
        fields = sorted(
            item.message.split("declare the same ")[1].split(" verbatim")[0]
            for item in diagnostics
            if item.code == "BOUNDARY_NOT_SEPARATING"
        )
        self.assertEqual(
            fields,
            ["owned_scope", "ownership.description", "triggers.positive"],
        )
        # The declared precedence still suppresses the older heuristics, which
        # is exactly why the verbatim check must be independent of it.
        self.assertNotIn("SCOPE_COLLISION", codes)
        self.assertNotIn("ROUTING_AMBIGUITY", codes)

    def test_partial_overlap_under_declared_precedence_stays_legitimate(self) -> None:
        """Negative control: a real boundary with precedence is not flagged."""
        self.use_schema_1_4()
        first, second = self.plan["skills"]
        # Nested, not identical: the pair still says which side owns what.
        second["owned_scope"][0] = first["owned_scope"][0] + "/replay backlog"
        second["ownership"][0]["description"] = (
            first["ownership"][0]["description"] + " replay backlog"
        )
        second["triggers"]["positive"] = [
            first["triggers"]["positive"][0] + " replay backlog"
        ]
        second["routing_cases"][0]["prompt"] = second["triggers"]["positive"][0]
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertNotIn("BOUNDARY_NOT_SEPARATING", codes)
        self.assertNotIn("SCOPE_COLLISION", codes)

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

    def test_triggers_as_list_is_diagnosed_not_crashed(self) -> None:
        self.plan["skills"][0]["triggers"] = ["change Firebase messaging"]
        self.rewrite()
        self.assertIn(
            "SKILL_TRIGGERS_INVALID",
            [item.code for item in self.plan_diagnostics()],
        )
        self.assertIn("SKILL_TRIGGERS_INVALID", self.codes())

    def test_triggers_null_positive_is_diagnosed_not_crashed(self) -> None:
        self.plan["skills"][0]["triggers"]["positive"] = None
        self.rewrite()
        self.assertIn(
            "SKILL_TRIGGERS_INVALID",
            [item.code for item in self.plan_diagnostics()],
        )
        self.assertIn("SKILL_TRIGGERS_INVALID", self.codes())

    def test_string_line_range_is_diagnosed_not_crashed(self) -> None:
        self.plan["evidence"][0]["line_range"] = {"start": "1", "end": "5"}
        self.rewrite()
        self.assertIn(
            "EVIDENCE_LINE_RANGE",
            [item.code for item in self.plan_diagnostics()],
        )
        self.assertIn("EVIDENCE_LINE_RANGE", self.codes())

    def test_schema_1_4_string_source_paths_is_diagnosed_not_crashed(self) -> None:
        self.use_schema_1_4()
        self.plan["skills"][0]["source_paths"] = "config/firebase.php"
        self.rewrite()
        self.assertIn(
            "SKILL_PLAN_VALUE",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_non_list_nearest_siblings_is_diagnosed_not_crashed(self) -> None:
        self.plan["skills"][0]["nearest_siblings"] = None
        self.rewrite()
        self.assertIn(
            "SKILL_PLAN_VALUE",
            [item.code for item in self.plan_diagnostics()],
        )
        self.plan = self.base_plan()
        self.use_schema_1_1()
        self.plan["skills"][0]["nearest_siblings"] = None
        self.rewrite()
        self.assertIn(
            "SKILL_PLAN_VALUE",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_agents_dir_json_cli_reports_non_dict_skill_entry(self) -> None:
        agents = self.write_agent_wrappers()
        self.plan["skills"].append("rogue-entry")
        self.rewrite()
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
            "--agents-dir",
            str(agents),
            "--json",
        ]
        result = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 1, result.stderr or result.stdout)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["valid"])
        self.assertIn(
            "SKILL_PLAN_INVALID",
            [item["code"] for item in payload["diagnostics"]],
        )

    def test_profile_normalization_only_rewrites_profile_identifiers(self) -> None:
        normalize = validator._normalize_line
        self.assertEqual(
            normalize("review the profile before deploying"),
            "review the profile before deploying",
        )
        summary = normalize(
            "compare the code against the approved project profile summary"
        )
        budget = normalize(
            "compare the code against the approved project profile budget"
        )
        self.assertNotEqual(summary, budget)
        self.assertEqual(
            summary,
            "compare the code against the approved project profile summary",
        )
        self.assertEqual(
            normalize("check the profiler output"), "check the profiler output"
        )
        self.assertEqual(
            normalize("read profile-2024-alpha now"), "read profile-id now"
        )
        self.assertEqual(
            normalize("apply PROFILE_A settings"), "apply profile-id settings"
        )

    def test_shared_positive_trigger_is_ambiguous_despite_low_set_overlap(self) -> None:
        first, second = self.plan["skills"]
        first["triggers"]["positive"] = [
            "review Doctrine migrations",
            "check database schema changes",
        ]
        second["triggers"]["positive"] = [
            "review Doctrine migrations",
            "design entity relationships",
            "plan messenger transport indexes",
        ]
        first["nearest_siblings"][0]["boundary"] = "Adjacent scope differs."
        second["nearest_siblings"][0]["boundary"] = "Adjacent scope differs."
        self.rewrite()
        left_tokens = validator._meaningful_tokens(
            " ".join(first["triggers"]["positive"])
        )
        right_tokens = validator._meaningful_tokens(
            " ".join(second["triggers"]["positive"])
        )
        whole_set_score = len(left_tokens & right_tokens) / len(
            left_tokens | right_tokens
        )
        self.assertLess(whole_set_score, 0.75)
        self.assertIn(
            "ROUTING_AMBIGUITY",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_distinct_triggers_without_precedence_are_not_ambiguous(self) -> None:
        first, second = self.plan["skills"]
        first["nearest_siblings"][0]["boundary"] = "Adjacent scope differs."
        second["nearest_siblings"][0]["boundary"] = "Adjacent scope differs."
        self.rewrite()
        self.assertNotIn(
            "ROUTING_AMBIGUITY",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_shipped_candidate_registry_is_accepted_by_the_gate(self) -> None:
        registry_payload = json.loads(
            validator.DEFAULT_REGISTRY.read_text(encoding="utf-8")
        )
        diagnostics: list = []
        candidates = validator._load_registry(
            validator.DEFAULT_REGISTRY,
            registry_payload["catalog_version"],
            diagnostics,
        )
        self.assertEqual([item.code for item in diagnostics], [])
        self.assertEqual(
            set(candidates),
            {item["id"] for item in registry_payload["candidates"]},
        )

    def test_registry_with_unknown_top_level_key_is_rejected(self) -> None:
        registry = json.loads(self.registry_path.read_text(encoding="utf-8"))
        registry["unexpected"] = True
        self.registry_path.write_text(
            json.dumps(registry, indent=2) + "\n", encoding="utf-8"
        )
        self.assertIn(
            "REGISTRY_INVALID",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_regex_metacharacter_skill_name_does_not_crash(self) -> None:
        old, new = "firebase-services", "firebase-(services"
        self.plan["skills"][0]["name"] = new
        self.skill_texts[new] = self.skill_texts.pop(old).replace(old, new)
        self.rewrite()
        codes = self.codes()
        self.assertIn("SKILL_SIBLING_UNKNOWN", codes)
        agents = self.write_agent_wrappers()
        diagnostics = validator.validate_agent_routing(
            agents,
            self.plan_path,
            self.target,
            registry_path=self.registry_path,
            validate_plan_first=False,
        )
        self.assertIsInstance(diagnostics, list)

    def test_disjoint_extension_globs_do_not_collide(self) -> None:
        self.assertFalse(validator._globs_intersect("docs/*.md", "docs/*.json"))
        self.assertFalse(
            validator._globs_intersect("docs/*.md", "docs/CHANGELOG.json")
        )
        self.assertFalse(validator._globs_intersect("docs/*.md", "src/*.md"))
        self.assertTrue(validator._globs_intersect("docs/*.md", "docs/**"))
        self.assertTrue(validator._globs_intersect("docs/*.md", "docs/README.md"))
        self.assertTrue(validator._globs_intersect("docs", "docs/*.md"))
        self.assertTrue(validator._globs_intersect("docs/*", "docs/*.md"))
        # A bare wildcard-free subdirectory may denote a whole tree
        # (docs/sub/notes.md is matched by fnmatch's slash-crossing "*"),
        # so it stays a conservative collision with an ancestor-level glob.
        self.assertTrue(validator._globs_intersect("docs/*.md", "docs/sub"))
        self.assertTrue(validator._globs_intersect("docs/sub", "docs/*.md"))
        self.assertTrue(validator._globs_intersect("docs/sub/", "docs/*.md"))
        # Disjoint sibling directories still do not collide.
        self.assertFalse(validator._globs_intersect("docs/sub", "docs/other"))
        # A wildcard-free name WITH an extension stays a single file, so the
        # suffix disjointness proof still applies to it.
        self.assertFalse(
            validator._globs_intersect("docs/*.md", "docs/sub.json")
        )
        self.plan["skills"][0]["writes"] = ["reports/*.md"]
        self.plan["skills"][1]["writes"] = ["reports/*.json"]
        self.rewrite()
        self.assertNotIn(
            "WRITE_SURFACE_COLLISION",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_character_class_globs_stay_conservative(self) -> None:
        # fnmatch proves "docs/b.txt" matches both patterns, so the suffix
        # disjointness proof must not fire when a [...] class is in the tail.
        self.assertTrue(
            validator._globs_intersect("docs/[ab].txt", "docs/[bc].txt")
        )
        self.assertTrue(
            validator._globs_intersect("docs/[ab].txt", "docs/*.txt")
        )
        # A class-bearing tail is suffix-unknown; a class before the last
        # wildcard still yields a provable literal tail.
        self.assertIsNone(validator._glob_suffix("docs/[ab].txt"))
        self.assertIsNone(validator._glob_suffix("docs/*.t[xy]t"))
        self.assertEqual(validator._glob_suffix("docs/[ab]*.md"), ".md")
        # Directory-disjoint class patterns still do not collide.
        self.assertFalse(
            validator._globs_intersect("src/[ab].txt", "docs/[ab].txt")
        )

    def test_glob_path_matches_do_not_follow_symlinks_out_of_target(self) -> None:
        outside = self.base / "outside"
        outside.mkdir()
        (outside / "secret.txt").write_text("secret", encoding="utf-8")
        (self.target / "link").symlink_to(outside)
        self.assertEqual(validator._path_matches(self.target, "link/*.txt"), [])
        (self.target / "docs").mkdir()
        (self.target / "docs" / "a.txt").write_text("a", encoding="utf-8")
        self.assertEqual(
            validator._path_matches(self.target, "docs/*.txt"),
            [self.target / "docs" / "a.txt"],
        )

    def test_symlinked_skills_dir_is_rejected(self) -> None:
        link = self.base / "skills-link"
        link.symlink_to(self.skills)
        diagnostics = validator.validate(
            link, self.plan_path, self.target, self.registry_path
        )
        self.assertIn(
            "SKILLS_DIR_UNSAFE", [item.code for item in diagnostics]
        )

    def test_tokenizer_strips_sentence_punctuation(self) -> None:
        self.assertEqual(
            validator._tokens("The project uses config/services.yaml."),
            ["project", "uses", "config/services.yaml"],
        )
        self.assertIn("doctrine", validator._meaningful_tokens("uses Doctrine."))
        self.assertTrue(
            validator._contract_matches(
                ["config/services.yaml"],
                "The project uses config/services.yaml.",
            )
        )

    def test_frontmatter_supports_folded_and_literal_descriptions(self) -> None:
        fields, body = validator._parse_frontmatter(
            "---\nname: sample\ndescription: >\n  Use when reviewing\n"
            "  folded descriptions.\n---\nBody\n"
        )
        self.assertEqual(fields["name"], "sample")
        self.assertEqual(
            fields["description"], "Use when reviewing folded descriptions."
        )
        self.assertEqual(body, "Body")
        fields, _ = validator._parse_frontmatter(
            "---\ndescription: |\n  Use when needed.\n  Second line.\n---\n"
        )
        self.assertEqual(fields["description"], "Use when needed. Second line.")
        self.skill_texts["firebase-services"] = self.skill_texts[
            "firebase-services"
        ].replace(
            "description: Use when changing Firebase initialization, messaging,"
            " or provider failure handling.",
            "description: >\n  Use when changing Firebase initialization,\n"
            "  messaging, or provider failure handling.",
        )
        self.rewrite()
        codes = self.codes()
        self.assertNotIn("SKILL_DESCRIPTION", codes)
        self.assertNotIn("SKILL_FRONTMATTER_NAME", codes)

    def test_fixture_catalog_expected_codes_are_emitted_by_validator(self) -> None:
        # The catalog spans every gate in this directory, not only the plan
        # quality one: discovery coverage, claim reconciliation, and the
        # adversarial review each emit codes it now names.
        source = "\n".join(
            path.read_text(encoding="utf-8")
            for path in sorted(VALIDATOR_PATH.parent.glob("validate_*.py"))
        )
        emitted = set(re.findall(r'"([A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+)"', source))
        cases = json.loads(CASES_PATH.read_text(encoding="utf-8"))
        for name, case in sorted(cases.items()):
            if case["expected"] is not None:
                self.assertIn(
                    case["expected"],
                    emitted,
                    f"{name} expects a code the validator never emits",
                )


class SkillBodyCommandTest(SkillQualityFixture):
    """Skill prose is executed by the agent, so it must be command-analyzed.

    Before this gate existed a skill body could hand the agent a destructive or
    provider-calling command in prose and still validate clean: the analyzer
    ran only over ``plan.verification[].command``.
    """

    WRITER = "firebase-services"
    REVIEWER = "availability-contract-review"

    def setUp(self) -> None:
        super().setUp()
        self.use_schema_1_4()

    def insert(self, name: str, markdown: str) -> None:
        self.skill_texts[name] = self.skill_texts[name].replace(
            "## Verification\n\n", f"## Verification\n\n{markdown}\n\n"
        )
        self.rewrite()

    def diagnostics(self) -> list:
        return validator.validate(
            self.skills, self.plan_path, self.target, self.registry_path
        )

    def command_risks(self) -> list[str]:
        return [
            item.message
            for item in self.diagnostics()
            if item.code == "SKILL_BODY_COMMAND_RISK"
        ]

    def test_baseline_schema_1_4_bodies_stay_clean(self) -> None:
        self.assertEqual(self.codes(), [])

    def test_destructive_command_in_a_bash_block_is_blocked(self) -> None:
        self.insert(self.WRITER, "```bash\nrm -rf var/cache/dev\n```")
        risks = self.command_risks()
        self.assertEqual(len(risks), 1)
        self.assertIn(self.WRITER, risks[0])
        self.assertIn("rm -rf var/cache/dev", risks[0])
        self.assertIn("DESTRUCTIVE_FILESYSTEM", risks[0])
        self.assertEqual(
            [item.severity for item in self.diagnostics() if item.code == "SKILL_BODY_COMMAND_RISK"],
            ["error"],
        )

    def test_a_long_chain_cannot_hide_its_dangerous_head(self) -> None:
        """The analysis cap must not become the bypass.

        A LIFO walk consumed the tail first, so a chain longer than
        ``MAX_BODY_COMMAND_SEGMENTS`` silently dropped its head - precisely
        where padding would hide the dangerous command.
        """
        padding = " && ".join(
            f"echo step{index}"
            for index in range(validator.MAX_BODY_COMMAND_SEGMENTS + 40)
        )
        destructive = "rm " + "-rf"
        self.insert(
            self.WRITER, f"```bash\n{destructive} var/important && {padding}\n```"
        )
        codes = self.codes()
        self.assertIn("SKILL_BODY_COMMAND_RISK", codes)
        self.assertIn("SKILL_BODY_COMMAND_UNSCANNED", codes)
        self.assertIn(
            f"{destructive} var/important",
            " ".join(self.command_risks()),
        )

    def test_guardrail_prose_may_name_the_command_it_forbids(self) -> None:
        destructive = "rm " + "-rf"
        self.insert(
            self.WRITER,
            f"Never run `{destructive} var/` here; escalate to the team instead.",
        )
        self.assertEqual(self.codes(), [])

    def test_network_post_in_skill_prose_is_blocked(self) -> None:
        self.insert(
            self.REVIEWER,
            "Run `curl -X POST https://billing.example/api/invoices/void` first.",
        )
        risks = self.command_risks()
        self.assertEqual(len(risks), 1)
        self.assertIn(self.REVIEWER, risks[0])
        self.assertIn("EXTERNAL_OR_PROVIDER", risks[0])
        self.assertIn("https://billing.example/api/invoices/void", risks[0])

    def test_composed_prose_chain_hidden_behind_a_non_breaking_space(self) -> None:
        """The reproduced miss: composition plus a homoglyph separator."""
        self.insert(
            self.WRITER,
            "Run `rm\u00a0-rf var/cache/dev && php bin/console cache:warmup "
            "--env=prod && curl -X POST https://billing.example/api/invoices/void` "
            "before you trust the result.",
        )
        risks = self.command_risks()
        self.assertEqual(len(risks), 2)
        self.assertTrue(any("DESTRUCTIVE_FILESYSTEM" in item for item in risks))
        self.assertTrue(any("EXTERNAL_OR_PROVIDER" in item for item in risks))
        self.assertTrue(any("rm -rf var/cache/dev" in item for item in risks))

    def test_privilege_escalation_in_a_bash_block_is_blocked(self) -> None:
        self.insert(self.WRITER, "```sh\nsudo php -l config/firebase.php\n```")
        risks = self.command_risks()
        self.assertEqual(len(risks), 1)
        self.assertIn("SUDO_EXECUTION", risks[0])

    def test_identifiers_paths_and_code_in_inline_backticks_are_not_commands(
        self,
    ) -> None:
        self.insert(
            self.REVIEWER,
            "The guard lives in `InvoiceSettlementService`, is wired by "
            "`config/packages/messenger.yaml`, gated on `ROLE_TENANT_OWNER`, "
            "bounded by `retry_strategy.max_retries`, refuses when "
            "`if ($invoice->state === 'settled')` holds, and is linted with "
            "`php -l domain/availability.php`.",
        )
        self.assertEqual(self.codes(), [])

    def test_read_only_tooling_in_a_bash_block_is_not_blocked(self) -> None:
        self.insert(
            self.WRITER,
            "```bash\nvendor/bin/phpunit --filter FirebaseTransportTest\n"
            "php -l config/firebase.php\nbin/console debug:container --env=test\n```",
        )
        self.assertEqual(self.codes(), [])

    def test_unattestable_but_harmless_block_lines_do_not_block(self) -> None:
        """Verification blockers are attestability, not danger; prose is not failed for them."""
        self.insert(
            self.WRITER,
            "```console\n$ composer test\n$ ./scripts/local-check.sh\n"
            "PHPUnit 10.5.0 by Sebastian Bergmann.\nOK (3 tests, 7 assertions)\n```",
        )
        self.assertEqual(self.codes(), [])

    def test_non_shell_fenced_blocks_are_not_scanned(self) -> None:
        self.insert(
            self.WRITER,
            "```php\n<?php\nunlink($cachePath);\nmkdir($reportDir, 0o755, true);\n```",
        )
        self.assertEqual(self.codes(), [])

    def test_payload_quoted_behind_an_interpreter_is_analyzed(self) -> None:
        self.insert(
            self.WRITER,
            '```bash\nbash -c "curl -X POST https://billing.example/hook"\n```',
        )
        risks = self.command_risks()
        self.assertEqual(len(risks), 1)
        self.assertIn("EXTERNAL_OR_PROVIDER", risks[0])
        self.assertIn("curl -X POST https://billing.example/hook", risks[0])

    def test_inline_interpreter_code_flag_payload_is_analyzed(self) -> None:
        self.insert(self.WRITER, "Run `php -r 'unlink(\"var/cache/x\");'` now.")
        risks = self.command_risks()
        self.assertEqual(len(risks), 1)
        self.assertIn("DESTRUCTIVE_FILESYSTEM", risks[0])

    def test_quoted_text_of_a_non_interpreter_is_not_re_read_as_code(self) -> None:
        """Only code hosts get their arguments re-read; grep keeps its pattern."""
        self.insert(
            self.WRITER,
            '```bash\ngrep -rn "rm -rf" config/\n'
            'php bin/console lint:yaml config/firebase.php\n```',
        )
        self.assertEqual(self.codes(), [])

    def test_environment_prefix_does_not_hide_the_executable(self) -> None:
        self.insert(self.WRITER, "```bash\nAPP_ENV=prod rm -rf var/cache\n```")
        risks = self.command_risks()
        self.assertEqual(len(risks), 1)
        self.assertIn("DESTRUCTIVE_FILESYSTEM", risks[0])

    def test_body_command_diagnostics_are_deduplicated_and_sorted(self) -> None:
        self.insert(
            self.WRITER,
            "```bash\nrm -rf var/cache/dev\nrm -rf var/cache/dev\ngit push origin main\n```",
        )
        risks = self.command_risks()
        self.assertEqual(len(risks), 2)
        self.assertEqual(risks, sorted(risks))
        diagnostics = self.diagnostics()
        self.assertEqual(diagnostics, sorted(set(diagnostics)))


class VerificationAttestationTest(unittest.TestCase):
    """The runtime contract may attest, never overrule.

    The seeded memory runtime is executed through an interpreter, so
    `python3 <script> status` cannot be proven non-mutating by inspection.
    The memory quartet is unconditional and verifies itself with exactly those
    commands, so before this route existed the generator could not pass its
    own gate on any target: a real end-to-end run ended in FAIL after 19
    forge iterations.
    """

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="attest-")
        self.target = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)
        (self.target / "composer.json").write_text("{}", encoding="utf-8")
        self.analyzer = validator.CommandAnalyzer(self.target)

    def accepted(self, command: str) -> bool:
        analysis = self.analyzer.analyze(command, verification=True)
        return analysis.verification_safe or validator._verification_attested(
            command, analysis
        )

    def test_declared_read_only_commands_are_attested(self) -> None:
        for command in validator._attested_read_only_commands():
            with self.subTest(command=command):
                self.assertTrue(self.accepted(command))

    def test_the_attested_set_comes_from_the_generator_not_the_target(self) -> None:
        """A scanned project must not be able to declare its own commands safe."""
        (self.target / "runtime-contract.json").write_text(
            json.dumps({"commands": {"read_health": ["python3 evil.py wipe"]}}),
            encoding="utf-8",
        )
        self.assertNotIn(
            "python3 evil.py wipe", validator._attested_read_only_commands()
        )
        self.assertFalse(self.accepted("python3 evil.py wipe"))

    def test_mutating_forms_of_the_same_script_stay_blocked(self) -> None:
        for command in (
            "python3 memory-bank/scripts/context.py refresh",
            "python3 memory-bank/scripts/context.py start --task-id X --goal Y",
            "python3 memory-bank/scripts/context.py complete --task-id X --outcome Y",
        ):
            with self.subTest(command=command):
                self.assertFalse(self.accepted(command))

    def test_an_undeclared_script_stays_blocked(self) -> None:
        self.assertFalse(self.accepted("python3 tools/other.py status"))

    def test_attestation_cannot_launder_a_dangerous_command(self) -> None:
        """A proven risk is never waived, whatever the contract says."""
        destructive = "rm " + "-rf"
        command = f"python3 memory-bank/scripts/context.py status && {destructive} var"
        analysis = self.analyzer.analyze(command, verification=True)
        self.assertFalse(validator._verification_attested(command, analysis))
        self.assertFalse(self.accepted(command))


class BodyCommandExtractionTest(unittest.TestCase):
    """Unit-level calibration of what prose counts as a command."""

    def commands(self, body: str) -> list[tuple[str, bool]]:
        return validator._body_commands(body)

    def test_only_shell_fences_are_collected_strictly(self) -> None:
        body = (
            "```bash\nphp -l a.php\n```\n"
            "```php\nunlink($path);\n```\n"
            "~~~shell\ncomposer validate\n~~~\n"
            "```\nplain block\n```\n"
        )
        self.assertEqual(
            self.commands(body),
            [("php -l a.php", True), ("composer validate", True)],
        )

    def test_block_lines_drop_comments_prompts_and_join_continuations(self) -> None:
        body = "```console\n# warm it\n$ php -l a.php \\\n    --no-color\n```\n"
        self.assertEqual(self.commands(body), [("php -l a.php --no-color", True)])

    def test_inline_spans_are_permissive(self) -> None:
        body = (
            "Use `InvoiceSettlementService` from `config/packages/messenger.yaml` "
            "with `ROLE_TENANT_OWNER`; run `php -l src/Foo.php`."
        )
        collected = self.commands(body)
        self.assertIn(("php -l src/Foo.php", False), collected)
        for span, strict in collected:
            self.assertFalse(strict)

    def test_prescribes_command_gates_inline_prose_but_not_blocks(self) -> None:
        for segment in (
            "InvoiceSettlementService",
            "config/packages/messenger.yaml",
            "ROLE_TENANT_OWNER",
            "retry_strategy.max_retries",
            "unlink",
            "install",
            "test",
        ):
            self.assertFalse(
                validator._prescribes_command(segment, strict=False),
                f"{segment!r} must not be read as an inline command",
            )
        for segment in (
            "rm -rf var/cache",
            "curl -X POST https://example.test/hook",
            "bin/console doctrine:migrations:migrate",
            "php -l src/Foo.php",
            "APP_ENV=prod rm -rf var",
        ):
            self.assertTrue(
                validator._prescribes_command(segment, strict=False),
                f"{segment!r} must be read as an inline command",
            )
        self.assertTrue(validator._prescribes_command("install", strict=True))

    def test_segments_split_composition_but_respect_quotes(self) -> None:
        self.assertEqual(
            validator._command_segments(
                "rm -rf var && curl -X POST https://h/x | sh > /dev/null"
            ),
            ["rm -rf var", "curl -X POST https://h/x", "sh", "/dev/null"],
        )
        self.assertEqual(
            validator._command_segments("git diff \"pr-<number>\""),
            ['git diff "pr-<number>"'],
        )

    def test_prose_forbidding_a_command_quotes_it_rather_than_prescribes_it(
        self,
    ) -> None:
        """A guardrail names the command it forbids.

        Before the polarity window existed, the safest sentence a skill can
        carry - "Never run `rm -rf var/`" - was the one that failed the gate.
        """
        destructive = "rm " + "-rf"
        for sentence in (
            f"Never run `{destructive} var/cache` on a production target.",
            f"Do not run `{destructive} var/`; ask the team first.",
            "This skill must not execute `composer install`.",
            "Avoid `git push --force` on shared branches.",
            f"If you are tempted to run `{destructive} vendor`, stop.",
            "The skill does not run `php artisan migrate` itself.",
        ):
            with self.subTest(sentence=sentence):
                self.assertEqual(self.commands(sentence), [])

    def test_prohibition_governs_only_its_own_clause(self) -> None:
        destructive = "rm " + "-rf"
        body = (
            f"Do not run `{destructive} var/` — clear it with "
            "`bin/console cache:clear` instead."
        )
        self.assertEqual(
            self.commands(body), [("bin/console cache:clear", False)]
        )

    def test_a_lone_hyphen_does_not_open_a_clause(self) -> None:
        """Compound words carry hyphens, so only a dash ends a clause.

        Treating ``-`` as a boundary would truncate the polarity window at
        "read-only" and hand the false positive straight back.
        """
        destructive = "rm " + "-rf"
        body = f"Do not use the read-only shortcut `{destructive} var/` here."
        self.assertEqual(self.commands(body), [])

    def test_a_flag_inside_a_span_does_not_end_the_prohibition(self) -> None:
        """A CLI flag is not punctuation, so it cannot close a clause.

        With span contents left in the polarity window, the ``--`` of a flag
        in the first span opened a fresh clause, and the second command of a
        two-command prohibition was read as prescribed - the guardrail
        blocked the very command it forbids.
        """
        for body in (
            "Never run `bin/console doctrine:schema:update --force` or "
            "`composer install` here.",
            "Do not run `composer install --no-dev` or `php artisan migrate`.",
        ):
            with self.subTest(body=body):
                self.assertEqual(self.commands(body), [])

    def test_a_dash_after_a_flagged_span_still_ends_the_prohibition(self) -> None:
        """Blanking span contents must not blind the boundary scan to prose."""
        destructive = "rm " + "-rf"
        body = (
            f"Do not run `{destructive} var/cache --no-ansi` — clear it with "
            "`bin/console cache:clear` instead."
        )
        self.assertEqual(
            self.commands(body), [("bin/console cache:clear", False)]
        )

    def test_a_shell_fence_stays_an_instruction_despite_prose_polarity(
        self,
    ) -> None:
        destructive = "rm " + "-rf"
        body = f"Never do this:\n\n```bash\n{destructive} var/important\n```\n"
        self.assertEqual(self.commands(body), [(f"{destructive} var/important", True)])

    def test_only_code_hosts_expose_nested_arguments(self) -> None:
        self.assertEqual(
            validator._hosted_code_arguments('bash -c "curl https://h/x"'),
            ["curl https://h/x"],
        )
        self.assertEqual(
            validator._hosted_code_arguments("php -r 'unlink(\"x\");'"),
            ['unlink("x");'],
        )
        self.assertEqual(
            validator._hosted_code_arguments('grep -rn "rm -rf" config/'), []
        )
        self.assertEqual(
            validator._hosted_code_arguments("php 'unterminated"), []
        )

    def test_invisible_separators_are_folded_before_analysis(self) -> None:
        self.assertEqual(
            validator._normalize_command_text("rm\u00a0-rf\u200b var"),
            "rm -rf var",
        )


class SkillOperationalContentTest(SkillQualityFixture):
    """A skill must tell the agent to do something, not just name the project.

    Before this gate existed, a body could carry real paths, classes, and
    constants in every sentence, trace to every plan contract, and still
    prescribe nothing: "Consider X holistically", "Form an opinion",
    "Confirm that the claim still looks reasonable". Traceability is lexical,
    so all of it validated clean.
    """

    WRITER = "firebase-services"
    REVIEWER = "availability-contract-review"

    VACUOUS_STEPS = [
        "Consider config/firebase.php and the FirebaseMessagingClient "
        "initialization holistically.",
        "Take into account the messaging transport and the provider API boundary.",
        "Bear in mind the provider failures while thinking about redelivery.",
        "Which provider failures may retry? Form an opinion, keeping in mind "
        "that we choose retry only for transient provider failures.",
        "Reflect on availability domain behavior belonging to "
        "availability-contract-review; Firebase transport is owned here.",
    ]
    OPERATIONAL_STEPS = [
        "Inspect initialization in config/firebase.php and record the "
        "FirebaseMessagingClient the container builds.",
        "Trace message transport from publishReminder() to the provider API "
        "and note where the payload leaves the process.",
        "Classify provider failures as transient, permanent, or configuration "
        "faults raised from config/firebase.php.",
        "Which provider failures may retry? Choose retry only for transient "
        "provider failures; convert an invalid payload into a permanent failure.",
        "Park a permanently failed payload instead of retrying it, and keep "
        "availability-contract-review responsible for availability domain behavior.",
    ]

    def set_procedure(self, name: str, steps: list[str]) -> None:
        head, marker, rest = self.skill_texts[name].partition(
            "## Procedure / Process\n\n"
        )
        _, tail_marker, tail = rest.partition("\n\n## Verification")
        numbered = "\n".join(
            f"{index}. {step}" for index, step in enumerate(steps, 1)
        )
        self.skill_texts[name] = f"{head}{marker}{numbered}{tail_marker}{tail}"
        self.rewrite()

    def set_verification(self, name: str, text: str) -> None:
        head, marker, rest = self.skill_texts[name].partition(
            "## Verification\n\n"
        )
        _, tail_marker, tail = rest.partition("\n\n## Outputs")
        self.skill_texts[name] = f"{head}{marker}{text}{tail_marker}{tail}"
        self.rewrite()

    def test_project_specific_but_vacuous_procedure_is_rejected(self) -> None:
        self.set_procedure(self.WRITER, self.VACUOUS_STEPS)
        codes = self.codes()
        # The miss this pins: every lexical contract still traces.
        for traced in (
            "SKILL_PROCEDURE_TRACE",
            "SKILL_DECISION_TRACE",
            "SKILL_CONTENT_EMPTY",
            "SKILL_SIMILARITY",
            "SKILL_TARGET_REFERENCE",
        ):
            self.assertNotIn(traced, codes)
        self.assertIn("SKILL_STEP_HEDGED", codes)
        hedged = [
            item
            for item in validator.validate(
                self.skills, self.plan_path, self.target, self.registry_path
            )
            if item.code == "SKILL_STEP_HEDGED"
        ]
        self.assertEqual(len(hedged), len(self.VACUOUS_STEPS))
        self.assertTrue(all(item.severity == "error" for item in hedged))

    def test_step_that_names_nouns_without_commanding_an_action_is_rejected(
        self,
    ) -> None:
        steps = list(self.OPERATIONAL_STEPS)
        steps[2] = (
            "Provider failures, the messaging transport, and the "
            "initialization boundary of config/firebase.php."
        )
        self.set_procedure(self.WRITER, steps)
        codes = self.codes()
        self.assertIn("SKILL_STEP_NOT_OPERATIONAL", codes)
        self.assertNotIn("SKILL_STEP_HEDGED", codes)

    def test_procedure_without_any_concrete_anchor_is_rejected(self) -> None:
        self.set_procedure(
            self.WRITER,
            [
                "Inspect the initialization before selecting the configured client.",
                "Trace the message transport from the adapter to the provider.",
                "Classify provider failures as transient or permanent faults.",
                "Which provider failures may retry? Choose retry only for "
                "transient provider failures.",
                "Keep the availability review responsible for availability "
                "domain behavior.",
            ],
        )
        codes = self.codes()
        self.assertIn("SKILL_PROCEDURE_UNANCHORED", codes)
        self.assertNotIn("SKILL_STEP_NOT_OPERATIONAL", codes)

    def test_verification_that_accepts_an_impression_is_rejected(self) -> None:
        self.set_verification(
            self.WRITER,
            "Confirm that Firebase initialization and messaging still look "
            "reasonable and that nothing seems broken.",
        )
        codes = self.codes()
        self.assertIn("SKILL_VERIFICATION_NOT_FALSIFIABLE", codes)
        self.assertNotIn("SKILL_SECTION_MISSING", codes)

    def test_verification_that_names_no_check_is_rejected(self) -> None:
        self.set_verification(
            self.WRITER,
            "Firebase initialization and the messaging boundary of "
            "config/firebase.php: fine.",
        )
        self.assertIn("SKILL_VERIFICATION_NOT_OPERATIONAL", self.codes())

    def test_operational_project_specific_bodies_stay_clean(self) -> None:
        """Calibration: honest instruction must not be failed by this gate."""
        self.use_schema_1_4()
        self.set_procedure(self.WRITER, self.OPERATIONAL_STEPS)
        self.assertEqual(self.codes(), [])

    def test_plan_step_action_must_command_an_action(self) -> None:
        self.use_schema_1_4()
        step = self.plan["skills"][0]["procedure_steps"][0]
        step["action"] = (
            "Consider the Firebase messaging runtime boundary in "
            "config/firebase.php holistically"
        )
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("PROCEDURE_STEP_NOT_OPERATIONAL", codes)
        self.assertNotIn("PROCEDURE_STEP_INVALID", codes)

    def test_plan_step_without_path_evidence_or_anchor_is_rejected(self) -> None:
        self.use_schema_1_4()
        step = self.plan["skills"][0]["procedure_steps"][0]
        step["action"] = "Review the boundary and record the outcome"
        step["path_refs"] = []
        step["evidence_ids"] = []
        self.rewrite()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertIn("PROCEDURE_STEP_UNANCHORED", codes)
        self.assertNotIn("PROCEDURE_STEP_NOT_OPERATIONAL", codes)

    def test_plan_verification_that_accepts_an_impression_is_rejected(
        self,
    ) -> None:
        self.use_schema_1_4()
        check = self.plan["skills"][0]["verification"][0]
        check["expected_result"] = (
            "The Firebase messaging boundary still looks correct"
        )
        self.rewrite()
        self.assertIn(
            "VERIFICATION_NOT_FALSIFIABLE",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_operational_diagnostics_are_deduplicated_and_sorted(self) -> None:
        self.set_procedure(self.WRITER, self.VACUOUS_STEPS)
        self.set_procedure(self.REVIEWER, self.VACUOUS_STEPS)
        diagnostics = validator.validate(
            self.skills, self.plan_path, self.target, self.registry_path
        )
        self.assertEqual(diagnostics, sorted(set(diagnostics)))


class OperationalContentUnitTest(unittest.TestCase):
    """Unit-level calibration of the verb/anchor structure."""

    HONEST_STEPS = (
        "Trace the reminder payload through dunning_async: confirm "
        "config/packages/messenger.yaml still routes App\\Message\\DunningReminder "
        "to dunning_async and that retry_strategy.max_retries is 3.",
        "Inspect the rejected allocation branch and confirm no partial mutation "
        "happens before ForbiddenTransition is thrown.",
        "Park a poison reminder in the doctrine failure queue instead of "
        "retrying it against the broker.",
        "Which reminder payloads may be redelivered? Redeliver only payloads "
        "that carry an invoice id; convert every other failure into "
        "UnrecoverableMessageHandlingException.",
        "Leave ledger paging and tenant filtering to ledger-entry-repository-guard "
        "rather than widening scope here.",
        "Run `vendor/bin/phpunit --filter DunningReminderHandlerTest` and compare "
        "the failure to config/packages/messenger.yaml.",
    )
    VACUOUS_STEPS = (
        "Consider src/Service/InvoiceSettlementService.php and the allocation "
        "guards holistically.",
        "Take into account the settled state transition.",
        "Bear in mind the rejected allocation branch.",
        "Is the allocation permitted? Form an opinion about the settled state.",
        "Reflect on dunning redelivery belonging to dunning-transport-wiring.",
        "When the guard trips, consider the allocation history.",
    )

    def test_honest_steps_command_an_action_without_hedging(self) -> None:
        for step in self.HONEST_STEPS:
            self.assertIsNone(validator._hedge_phrase(step), step)
            self.assertTrue(validator._commands_action(step), step)
        # The anchor requirement is per procedure, never per step: an honest
        # delegation or branch step legitimately carries no path or symbol.
        self.assertFalse(
            validator._has_concrete_anchor(
                "Park a poison reminder in the doctrine failure queue instead "
                "of retrying it against the broker."
            )
        )
        self.assertTrue(
            validator._has_concrete_anchor("\n".join(self.HONEST_STEPS))
        )

    def test_vacuous_steps_are_hedged_however_specific_they_are(self) -> None:
        for step in self.VACUOUS_STEPS:
            self.assertIsNotNone(validator._hedge_phrase(step), step)
        # Naming a real file does not rescue the step.
        self.assertTrue(validator._has_concrete_anchor(self.VACUOUS_STEPS[0]))
        self.assertTrue(validator._has_concrete_anchor(self.VACUOUS_STEPS[4]))

    def test_bare_noun_lists_command_no_action(self) -> None:
        for step in (
            "Allocation rules, the closed invoice, and the outstanding balance.",
            "Source file: src/Service/InvoiceSettlementService.php.",
            "The dunning transport declaration and its redelivery budget.",
            "Firebase initialization and the messaging boundary: fine.",
        ):
            self.assertFalse(validator._commands_action(step), step)
        for step in (
            "Assert the settled state.",
            "php -l src/Service/InvoiceSettlementService.php",
        ):
            self.assertTrue(validator._commands_action(step), step)

    def test_mid_sentence_hedging_next_to_an_action_stays_legitimate(self) -> None:
        self.assertIsNone(
            validator._hedge_phrase(
                "Reject the allocation once the invoice is settled, keeping in "
                "mind the ledger paging owned by the sibling."
            )
        )

    def test_honest_imperatives_outside_the_original_verb_list_are_kept(
        self,
    ) -> None:
        """Calibration regression: these honest steps were failed as inert.

        Each line is a real step from the adversarial harness's *honest*
        five-skill baseline, which the gate must pass. They command an action
        with a verb the first draft of `ACTION_VERBS` happened to omit, so the
        gate rejected 5 of that baseline's 37 steps - the false-positive mode
        that is worse than the miss the rule closes.
        """
        for step in (
            "Bound delivery at three attempts.",
            "Bound the ninety day refundable window.",
            "Downgrade only on a permanent delivery boundary error.",
            "Not this skill: change how a charge authorisation is sent to the "
            "card provider.",
            "Not this skill: recompute the anniversary anchor of a "
            "subscription cycle.",
            # A step may be substantively wrong and still command an action;
            # "commands no action" must not become the complaint about it.
            "Accept transitions that revive a cancelled slot when the actor "
            "role is ROLE_DESK.",
        ):
            self.assertIsNone(validator._hedge_phrase(step), step)
            self.assertTrue(validator._commands_action(step), step)

    def test_counterparts_of_listed_verbs_are_listed_too(self) -> None:
        """A verb whose opposite is already accepted must be accepted too."""
        for listed, counterpart in (
            ("upgrade", "downgrade"),
            ("compute", "recompute"),
            ("subscribe", "unsubscribe"),
            ("serialize", "deserialize"),
            ("serialise", "deserialise"),
            ("modify", "change"),
            ("bind", "bound"),
            ("halt", "abort"),
            ("disallow", "allow"),
            ("reject", "accept"),
            ("stop", "start"),
        ):
            self.assertIn(listed, validator.ACTION_VERBS)
            self.assertIn(counterpart, validator.ACTION_VERBS)
        # The rule stops at words that ordinarily read as adjective or noun,
        # or the bare-noun-list reading below would collapse.
        self.assertNotIn("close", validator.ACTION_VERBS)

    def test_anchor_detection_separates_concrete_from_abstract(self) -> None:
        for text in (
            "config/packages/messenger.yaml",
            "InvoiceSettlementService::allocate()",
            "ROLE_TENANT_OWNER",
            "retry_strategy.max_retries",
            "max_retries is 3",
            "ledger-entry-repository-guard",
            "`php -l`",
            "the state is 'settled'",
        ):
            self.assertTrue(validator._has_concrete_anchor(text), text)
        for text in (
            "Inspect the initialization before selecting the configured client.",
            "Classify provider failures as transient or permanent faults.",
        ):
            self.assertFalse(validator._has_concrete_anchor(text), text)

    def test_steps_keep_sub_bullets_blocks_and_drop_the_lead_in(self) -> None:
        section = (
            "Follow the bounded procedure:\n"
            "\n"
            "1. Inspect config/firebase.php and note the client.\n"
            "   - the transport is built there\n"
            "2. Run the linter:\n"
            "\n"
            "   ```bash\n"
            "   php -l config/firebase.php\n"
            "   ```\n"
            "3. Report the finding.\n"
        )
        self.assertEqual(
            validator._procedure_steps(section),
            [
                "Inspect config/firebase.php and note the client. - the "
                "transport is built there",
                "Run the linter: php -l config/firebase.php",
                "Report the finding.",
            ],
        )

    def test_a_root_dotfile_is_a_path_not_prose(self) -> None:
        for span, expected in (
            (".eslintrc:L1-L20", ".eslintrc"),
            (".eslintrc", ".eslintrc"),
            (".php-version", ".php-version"),
            (".env.local", ".env.local"),
        ):
            self.assertEqual(validator._anchor_path(span), expected, span)

    def test_prose_is_still_not_mistaken_for_a_dotfile(self) -> None:
        for span in ("indent", "...", ".4 spaces", ".", ".."):
            self.assertIsNone(validator._anchor_path(span), span)

    def test_headings_delimit_steps_and_own_their_bullets(self) -> None:
        # The forge prescribes anchor/inspect/decision/expected per step. Read
        # flush-left those four bullets are four steps, and the anchor line -
        # a citation - commands no action.
        section = (
            "### 1. Read the suite declaration\n"
            "\n"
            "- **Anchor.** `phpunit.xml.dist:L21-L25` (EV-0020).\n"
            "- **Inspect.** The block declares one suite over tests.\n"
            "\n"
            "### 2. Run the configured suite\n"
            "\n"
            "- **Anchor.** `phpunit.xml.dist:L21-L25` (EV-0020).\n"
            "- **Inspect.** Run vendor/bin/phpunit and read its output.\n"
        )
        steps = validator._procedure_steps(section)
        self.assertEqual(len(steps), 2)
        self.assertTrue(all(validator._commands_action(step) for step in steps))
        self.assertIn("phpunit.xml.dist:L21-L25", steps[0])

    def test_heading_delimited_step_without_an_action_still_fails(self) -> None:
        section = (
            "### 1. The suite declaration\n"
            "\n"
            "- **Anchor.** `phpunit.xml.dist:L21-L25` (EV-0020).\n"
        )
        steps = validator._procedure_steps(section)
        self.assertEqual(len(steps), 1)
        self.assertFalse(validator._commands_action(steps[0]))

    def test_a_section_owns_the_content_of_its_subsections(self) -> None:
        body = (
            "## Procedure\n"
            "\n"
            "### 1. Read the suite declaration\n"
            "\n"
            "Inspect phpunit.xml.dist and place the case in the one suite.\n"
            "\n"
            "## Verification\n"
            "\n"
            "Run vendor/bin/phpunit.\n"
        )
        sections = validator._sections(body)
        procedure = validator._section(sections, ("procedure",))
        self.assertIn("phpunit.xml.dist", procedure)
        self.assertNotIn("vendor/bin/phpunit", procedure)
        self.assertIn(
            "phpunit.xml.dist",
            validator._section(sections, ("1. read the suite declaration",)),
        )

    def test_a_qualified_heading_resolves_to_its_section(self) -> None:
        sections = validator._sections(
            "## Canonical inputs\n\nEV-0020 anchors phpunit.xml.dist:L21-L25.\n"
        )
        self.assertIn(
            "EV-0020", validator._section(sections, ("project evidence", "inputs"))
        )

    def test_an_exact_heading_wins_over_a_qualified_one(self) -> None:
        sections = validator._sections(
            "## Canonical inputs\n\nqualified body\n\n## Inputs\n\nexact body\n"
        )
        self.assertEqual(
            validator._section(sections, ("project evidence", "inputs")), "exact body"
        )

    def test_an_unrelated_heading_does_not_answer_for_a_missing_section(self) -> None:
        sections = validator._sections("## Boundaries\n\nRoute judgement elsewhere.\n")
        self.assertEqual(validator._section(sections, ("verification",)), "")

    def test_unlisted_prose_procedure_is_read_as_paragraphs(self) -> None:
        section = (
            "Read config/firebase.php and record the client.\n"
            "Then trace the transport.\n"
            "\n"
            "Report the finding.\n"
        )
        self.assertEqual(
            validator._procedure_steps(section),
            [
                "Read config/firebase.php and record the client. Then trace "
                "the transport.",
                "Report the finding.",
            ],
        )

    def test_verb_forms_cover_inflections_without_stemming_nouns(self) -> None:
        for text in (
            "Inspects the guard",
            "Inspecting the guard",
            "Traced the payload",
            "Classifies the failures",
            "Verifying the transition",
        ):
            self.assertTrue(validator._has_action_verb(text), text)

    def test_verification_vocabulary_is_narrower_than_the_step_vocabulary(
        self,
    ) -> None:
        impression = "Tenant scoping before offset and limit: fine."
        self.assertTrue(validator._has_action_verb(impression))
        self.assertFalse(
            validator._has_action_verb(
                impression, validator.VERIFICATION_VERB_FORMS
            )
        )
        for text in (
            "Assert from src/Service/InvoiceSettlementService.php that a "
            "closed invoice rejects partial allocation.",
            "Run php -l on the handler and compare the exit status.",
        ):
            self.assertTrue(
                validator._has_action_verb(
                    text, validator.VERIFICATION_VERB_FORMS
                ),
                text,
            )


class SkeletonNormalizationTest(unittest.TestCase):
    """Unit-level calibration of what the skeleton erases and what it keeps.

    The skeleton exists because a plan-conforming SKILL.md is *required* to
    carry its own claim, paths and neighbour names: those mandatory
    differences are exactly what a template generator varies, and they diluted
    raw similarity below any usable threshold. Erase too little and one
    substituted noun still hides a template; erase too much and two honest
    skills collapse onto the same skeleton.
    """

    def skeleton(self, line: str, entities=()) -> str:
        return validator._skeleton_line(line, entities)

    def test_project_identity_collapses_to_one_placeholder(self) -> None:
        placeholder = validator.SKELETON_PLACEHOLDER
        for line, expected in (
            ("Read config/packages/messenger.yaml first.", "read xid first"),
            ("Read src/Service/InvoiceSettlementService.php first.", "read xid first"),
            ("The DunningReminderHandler rejects it.", "the xid rejects it"),
            ("The InvoiceSettlementService rejects it.", "the xid rejects it"),
            ("Only ROLE_TENANT_OWNER may do it.", "only xid may do it"),
            ("Only ROLE_BILLING_ADMIN may do it.", "only xid may do it"),
            ("Retry up to 3 times.", "retry up to xid times"),
            ("Retry up to 25 times.", "retry up to xid times"),
            ("Check retry_strategy.max_retries here.", "check xid here"),
            ("Check messenger.failure_transport here.", "check xid here"),
            ("Mutating $invoice->state is forbidden.", "mutating xid is forbidden"),
            ("Mutating $reminder->invoiceId is forbidden.", "mutating xid is forbidden"),
            ("Call allocate() twice.", "call xid twice"),
            ("Call publishReminder() twice.", "call xid twice"),
            ("Run `php -l src/Foo.php` now.", "run xid now"),
            ("Run `composer validate` now.", "run xid now"),
        ):
            self.assertEqual(self.skeleton(line), expected, line)
            self.assertIn(placeholder, expected)

    def test_neighbour_skill_names_are_erased_as_whole_words_only(self) -> None:
        entities = ("invoice-settlement-guard", "dunning-transport-wiring")
        self.assertEqual(
            self.skeleton("Hand invoice-settlement-guard its part.", entities),
            self.skeleton("Hand dunning-transport-wiring its part.", entities),
        )
        # A short accidental substring must not eat an ordinary word.
        self.assertEqual(
            self.skeleton("Guard the release note.", ("re",)),
            "guard the release note",
        )

    def test_punctuation_and_connectives_no_longer_change_a_line(self) -> None:
        base = "Trace the payload, mark the gap and record the deviation."
        for variant in (
            "Trace the payload; mark the gap plus record the deviation!",
            "Trace the payload -- mark the gap, and record the deviation",
            "Trace  the   payload,  mark the gap and  record the deviation.",
        ):
            self.assertEqual(self.skeleton(variant), self.skeleton(base), variant)
        # ... but real wording still separates two lines.
        self.assertNotEqual(
            self.skeleton(base),
            self.skeleton("Trace the payload, mark the gap and reject the request."),
        )

    def test_lines_that_are_almost_entirely_identity_are_not_compared(self) -> None:
        body = (
            "Read config/packages/messenger.yaml.\n"
            "See src/Service/InvoiceSettlementService.php.\n"
            "Trace the reminder payload through the transport and record every "
            "redelivery the handler accepts.\n"
        )
        lines = validator._skeleton_body(body, [], ())
        self.assertEqual(len(lines), 1)
        self.assertNotIn("read xid", lines[0])

    def test_fenced_code_is_excluded_from_the_skeleton(self) -> None:
        body = (
            "Trace the reminder payload through the transport and record the "
            "redelivery budget.\n"
            "```php\n"
            "final class Foo extends Bar\n"
            "public function handle(): bool\n"
            "```\n"
        )
        self.assertEqual(len(validator._skeleton_body(body, [], ())), 1)

    def test_bag_similarity_is_symmetric_and_bounded(self) -> None:
        left = validator.Counter("a b c d e".split())
        right = validator.Counter("a b c d f".split())
        self.assertAlmostEqual(
            validator._bag_similarity(left, right),
            validator._bag_similarity(right, left),
        )
        self.assertAlmostEqual(validator._bag_similarity(left, left), 1.0)
        self.assertAlmostEqual(validator._bag_similarity(left, validator.Counter()), 0.0)

    def test_skeleton_line_matching_is_one_to_one(self) -> None:
        """One boilerplate line may not answer for several partners."""
        repeated = "trace the reminder payload through the transport and record it"
        left = [repeated] + [f"unrelated sentence number {index} about billing states"
                             for index in range(4)]
        right = [repeated, repeated, repeated, repeated, repeated]
        self.assertAlmostEqual(
            validator._skeleton_line_similarity(left, right), 0.2
        )


class SkillTemplateReuseTest(SkillQualityFixture):
    """One prose template, two subjects: the miss this class pins.

    The lexical dedup compares normalized lines for equality, so it only ever
    saw byte-identical prose. Measured on this fixture before the skeleton
    pass existed: a byte clone scored line=0.533/token=0.442 - under the
    0.70/0.80 fail lines, so even *that* passed; the same template with the
    project's own nouns substituted scored 0.333/0.300, and the same template
    with `,` swapped for `;` and `and` for `plus` scored 0.200/0.267, i.e.
    indistinguishable from two honestly different skills at 0.200/0.069.
    That is precisely the shape of an LLM reusing one template.
    """

    WRITER = "firebase-services"
    REVIEWER = "availability-contract-review"

    TEMPLATE_STEPS = (
        "Begin at {path} and write down, line by line, what it guarantees "
        "today, so that you {role_one} before you touch anything.",
        "Hold that written description next to the incoming request and mark "
        "every single place where the two of them disagree with each other.",
        "Walk the file a second time to {role_two}, and then {role_three} "
        "without ever leaving the file you started from.",
        "{question} Apply the rule that we {branch}, and write down every "
        "deviation from that rule you decide to accept.",
        "Weigh the residual risk the request leaves behind and hand {sibling} "
        "the part that belongs to it: {boundary}",
    )
    TEMPLATE_PURPOSE = "Hold the line described by {claim}, and nothing wider."
    TEMPLATE_INPUTS = (
        "Source file: {path}, read together with the plan claim {claim}."
    )
    TEMPLATE_VERIFICATION = (
        "Assert from the cited source that {claim}; run the check twice and "
        "compare the two results before you accept either of them."
    )
    TEMPLATE_OUTPUTS = (
        "Produce a cited report confirming or rejecting: {claim}, with the "
        "line numbers that carry the decision."
    )
    TEMPLATE_FAILURE = "{failure} Do not guess the missing part."

    SLOTS = {
        "firebase-services": {
            "description": (
                "Use when changing Firebase initialization, messaging, or "
                "provider failure handling."
            ),
            "path": "config/firebase.php",
            "claim": "Firebase initialization and messaging form a runtime boundary",
            "role_one": "inspect initialization",
            "role_two": "trace message transport",
            "role_three": "classify provider failures",
            "question": "Which provider failures may retry?",
            "branch": "choose retry only for transient provider failures",
            "sibling": "availability-contract-review",
            "boundary": (
                "Firebase transport is owned here; availability domain "
                "behavior is deferred."
            ),
            "failure": "Stop when provider boundary evidence is unavailable.",
        },
        "availability-contract-review": {
            "description": (
                "Use when reviewing availability state, sampled dates, "
                "cancellation, or partial failures."
            ),
            "path": "domain/availability.php",
            "claim": "Availability has sampled-date and cancellation invariants",
            "role_one": "enumerate sampled dates",
            "role_two": "trace cancellation transitions",
            "role_three": "inspect partial failures",
            "question": "Is the transition valid?",
            "branch": "reject transitions that violate the sampled-date invariant",
            "sibling": "firebase-services",
            "boundary": (
                "Availability behavior is owned here; Firebase transport is "
                "deferred."
            ),
            "failure": "Mark conflicting state authority unresolved and stop approval.",
        },
    }

    def template_body(self, name: str, slots: dict, punctuation: bool = False) -> str:
        steps = [step.format(**slots) for step in self.TEMPLATE_STEPS]
        if punctuation:
            steps = [
                step.replace(", ", "; ").replace(" and ", " plus ").replace(".", " --", 1)
                for step in steps
            ]
        return skill_markdown(
            name,
            slots["description"],
            self.TEMPLATE_PURPOSE.format(**slots),
            self.TEMPLATE_INPUTS.format(**slots),
            steps,
            self.TEMPLATE_VERIFICATION.format(**slots),
            self.TEMPLATE_OUTPUTS.format(**slots),
            self.TEMPLATE_FAILURE.format(**slots),
        )

    def install_template(self, *, punctuation: bool = False, clone: bool = False) -> None:
        """Write both skills from one template.

        `clone` keeps the writer's own nouns in the reviewer's body (a byte
        clone of the procedure), `punctuation` re-punctuates the reviewer only.
        """
        for name, slots in self.SLOTS.items():
            used = dict(slots)
            if clone and name == self.REVIEWER:
                borrowed = dict(self.SLOTS[self.WRITER])
                borrowed["description"] = slots["description"]
                borrowed["claim"] = slots["claim"]
                borrowed["failure"] = slots["failure"]
                used = borrowed
            self.skill_texts[name] = self.template_body(
                name, used, punctuation=punctuation and name == self.REVIEWER
            )
        self.rewrite()

    def scores(self) -> tuple[float, float]:
        skeletons = {}
        for name, text in self.skill_texts.items():
            _, body = validator._parse_frontmatter(text)
            entry = next(
                item for item in self.plan["skills"] if item["name"] == name
            )
            skeletons[name] = validator._skeleton_body(
                body,
                validator._fixed_block_contents(entry.get("fixed_blocks", [])),
                frozenset(self.skill_texts),
            )
        left, right = (skeletons[name] for name in sorted(skeletons))
        return (
            validator._skeleton_line_similarity(left, right),
            validator._token_similarity(
                validator._tokens("\n".join(left)),
                validator._tokens("\n".join(right)),
            ),
        )

    def test_template_with_substituted_project_nouns_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.install_template()
        codes = self.codes()
        self.assertIn("SKILL_TEMPLATE_REUSE", codes)
        # The miss this pins: every line differs, so the lexical pass is blind.
        self.assertNotIn("SKILL_SIMILARITY", codes)
        self.assertNotIn("REPEATED_BLOCK", codes)
        line, token = self.scores()
        self.assertGreaterEqual(line, validator.SKELETON_LINE_FAIL)
        self.assertGreaterEqual(token, validator.SKELETON_TOKEN_FAIL)

    def test_template_differing_only_in_punctuation_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.install_template(punctuation=True)
        codes = self.codes()
        self.assertIn("SKILL_TEMPLATE_REUSE", codes)
        self.assertNotIn("SKILL_SIMILARITY", codes)
        self.assertNotIn("REPEATED_BLOCK", codes)

    def test_byte_identical_procedure_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.install_template(clone=True)
        codes = self.codes()
        self.assertIn("SKILL_TEMPLATE_REUSE", codes)
        self.assertIn("SKILL_TEMPLATE_BLOCK", codes)

    def test_template_reuse_is_an_error_and_the_block_pass_is_a_warning(self) -> None:
        self.use_schema_1_4()
        self.install_template(clone=True)
        severities = {
            item.code: item.severity
            for item in validator.validate(
                self.skills, self.plan_path, self.target, self.registry_path
            )
        }
        self.assertEqual(severities["SKILL_TEMPLATE_REUSE"], "error")
        self.assertEqual(severities["SKILL_TEMPLATE_BLOCK"], "warning")

    def test_honest_distinct_skills_are_never_called_a_template(self) -> None:
        """The false positive that would make the generator unusable."""
        self.use_schema_1_4()
        codes = self.codes()
        self.assertEqual(codes, [])
        line, token = self.scores()
        self.assertLess(line, validator.SKELETON_LINE_WARN)
        self.assertLess(token, validator.SKELETON_TOKEN_WARN)

    def test_verbatim_versioned_fixed_block_is_not_duplication(self) -> None:
        """An approved shared safety block may repeat word for word."""
        self.use_schema_1_4()
        block = (
            "Never paste customer billing identifiers, provider secrets, or "
            "raw transport payloads into a report; redact the identifier, "
            "keep the structural evidence, and hand back a sanitized excerpt "
            "naming the file and the line that was withheld."
        )
        for entry in self.plan["skills"]:
            entry["fixed_blocks"].append(
                {"id": "safety.redaction", "version": "2.0", "content": block}
            )
            name = entry["name"]
            self.skill_texts[name] = self.skill_texts[name].replace(
                f"\n{SHARED_SAFETY}\n", f"\n{block}\n\n{SHARED_SAFETY}\n"
            )
        self.rewrite()
        codes = self.codes()
        self.assertNotIn("SKILL_TEMPLATE_REUSE", codes)
        self.assertNotIn("SKILL_TEMPLATE_REUSE_WARN", codes)
        self.assertNotIn("SKILL_TEMPLATE_BLOCK", codes)
        self.assertEqual(codes, [])

    def test_shared_fenced_code_is_not_duplication(self) -> None:
        """Two skills may show the same framework idiom in a fence."""
        self.use_schema_1_4()
        fence = (
            "\n```php\n"
            "final class Example extends AbstractController\n"
            "{\n"
            "    public function handle(): bool\n"
            "    {\n"
            "        return $this->service->run();\n"
            "    }\n"
            "}\n"
            "```\n"
        )
        for name in self.skill_texts:
            self.skill_texts[name] = self.skill_texts[name].replace(
                "\n## Verification\n", f"{fence}\n## Verification\n"
            )
        self.rewrite()
        codes = self.codes()
        self.assertNotIn("SKILL_TEMPLATE_REUSE", codes)
        self.assertNotIn("SKILL_TEMPLATE_BLOCK", codes)

    SHARED_SCAFFOLDING = [
        "hold that written description next to the incoming request and mark the gaps",
        "roll the change out in small increments and re read the file afterwards",
        "record every deviation from the stated rule that you decide to accept",
    ]
    TRANSPORT_LINES = [
        "inspect the redelivery budget declared by the asynchronous messenger transport",
        "classify each provider outage into transient permanent or configuration faults",
        "trace how a reminder leaves the process and reaches the outbound mail gateway",
        "note which payloads consume a retry attempt and which ones are parked at once",
        "confirm the routing entry still points at the queue named in the plan claim",
        "list the callers that dispatch a reminder outside the documented entry point",
        "compare the observed retry count against the configured maximum before approval",
    ]
    SETTLEMENT_LINES = [
        "enumerate the allocation guards that protect a terminal settled invoice",
        "reject any mutation that would raise an amount already marked as final",
        "walk each branch where a partial payment updates the running balance",
        "verify that no caller bypasses the service when adjusting stored money",
        "collect the roles permitted to reopen a closed statement for correction",
        "measure how many statements reach terminal status without a matching audit row",
        "identify the currency rounding rule applied when splitting a residual amount",
    ]

    def test_partial_scaffolding_overlap_is_a_warning_not_a_failure(self) -> None:
        """The band between honest noise and a template must not block a run.

        Three shared scaffolding sentences inside otherwise unrelated bodies
        is exactly what a legitimate family of parallel skills looks like, and
        that is the shape the honest corpus tops out at.
        """
        diagnostics: list = []
        validator._compare_skeletons(
            "dunning-transport-wiring",
            self.SHARED_SCAFFOLDING + self.TRANSPORT_LINES,
            "invoice-settlement-guard",
            self.SHARED_SCAFFOLDING + self.SETTLEMENT_LINES,
            diagnostics,
        )
        self.assertEqual(
            [(item.code, item.severity) for item in diagnostics],
            [("SKILL_TEMPLATE_REUSE_WARN", "warning")],
        )

    def test_short_bodies_are_left_to_the_lexical_pass(self) -> None:
        """Below the line floor the ratio is noise, so nothing is reported."""
        diagnostics: list = []
        validator._compare_skeletons(
            "left",
            ["trace the payload and record it"] * 4,
            "right",
            ["trace the payload and record it"] * 4,
            diagnostics,
        )
        self.assertEqual(diagnostics, [])

    def test_template_diagnostics_stay_sorted_and_deduplicated(self) -> None:
        self.use_schema_1_4()
        self.install_template()
        diagnostics = validator.validate(
            self.skills, self.plan_path, self.target, self.registry_path
        )
        self.assertEqual(diagnostics, sorted(set(diagnostics)))
        again = validator.validate(
            self.skills, self.plan_path, self.target, self.registry_path
        )
        self.assertEqual(
            [(item.code, item.message, item.severity) for item in diagnostics],
            [(item.code, item.message, item.severity) for item in again],
        )


class RejectionAccountabilityTest(SkillQualityFixture):
    """A rejection must argue for itself the way a selection has to.

    Measured on a real run before this existed: 40 of 52 candidates rejected,
    all 40 carrying one identical sentence, none naming a file of the target.
    Every gate validated the selections exhaustively and nothing looked at the
    rejections, so a wrong one was invisible until a human read the list.
    """

    def reject(self, *items: dict) -> list[str]:
        self.plan["rejected_candidates"] = list(items)
        self.write_fixture()
        return [item.code for item in self.plan_diagnostics()]

    @staticmethod
    def rejection(candidate: str, category: str, reason: str, missing: list[str]):
        return {
            "candidate_id": candidate,
            "name": candidate,
            "category": category,
            "reason": reason,
            "missing_evidence": missing,
        }

    def test_an_anchored_rejection_passes(self) -> None:
        codes = self.reject(
            self.rejection(
                "generic-cache",
                "integration",
                "No cache runtime wiring: config/cache.php is absent and no cache "
                "client is constructed under app/",
                ["cache initialization in config/cache.php"],
            )
        )
        self.assertNotIn("REJECTION_UNANCHORED", codes)
        self.assertNotIn("REJECTION_TEMPLATED", codes)

    def test_a_rejection_naming_nothing_in_the_target_fails(self) -> None:
        codes = self.reject(
            self.rejection(
                "generic-cache",
                "integration",
                "No evidence in this target requires distinct operational guidance",
                ["A target surface this candidate would own"],
            )
        )
        self.assertIn("REJECTION_UNANCHORED", codes)

    def test_prose_abbreviations_are_not_anchors(self) -> None:
        # `e.g.` ends in a dot and two letters; a loose extension test would
        # read it as a file and pass an unanchored rejection.
        for text in ("Nothing here, e.g. no caching at all.", "Not applicable.",
                     "No surface, i.e. none."):
            self.assertEqual(validator._rejection_path_tokens(text), [], text)

    def test_real_paths_are_anchors_in_bare_prose(self) -> None:
        tokens = validator._rejection_path_tokens(
            "no config/packages/cache.yaml, no .eslintrc, nothing under assets/"
        )
        self.assertIn("config/packages/cache.yaml", tokens)
        self.assertIn(".eslintrc", tokens)

    def test_one_family_may_share_a_sentence(self) -> None:
        # "This project renders no HTML" is one honest judgement about every
        # frontend candidate at once, and it stays one judgement.
        shared = "No rendering layer: no resources/views/, no assets/, no package.json"
        codes = self.reject(
            self.rejection("frontend-design", "frontend", shared, ["assets/"]),
            self.rejection("wcag-accessibility", "frontend", shared, ["assets/"]),
            self.rejection("browser-verify", "frontend", shared, ["assets/"]),
        )
        self.assertNotIn("REJECTION_TEMPLATED", codes)

    def test_a_sentence_spanning_families_judges_none_of_them(self) -> None:
        shared = "No surface here: nothing under src/ requires it"
        codes = self.reject(
            self.rejection("frontend-design", "frontend", shared, ["src/"]),
            self.rejection("api-designer", "design", shared, ["src/"]),
            self.rejection("performance", "universal", shared, ["src/"]),
        )
        self.assertIn("REJECTION_TEMPLATED", codes)

    def test_two_candidates_never_trip_the_shared_sentence_rule(self) -> None:
        shared = "No surface here: nothing under src/ requires it"
        codes = self.reject(
            self.rejection("frontend-design", "frontend", shared, ["src/"]),
            self.rejection("api-designer", "design", shared, ["src/"]),
        )
        self.assertNotIn("REJECTION_TEMPLATED", codes)


class RejectionFalsifierTest(SkillQualityFixture):
    """A rejection is checked against the project it was made about.

    The registry carries, per candidate, the signal whose presence makes "no
    surface here" false. Measured on a real 52-candidate run: 25 of 40
    rejections were contradicted by the target's own files, 10 were confirmed,
    and 5 rest on things no repository can show.
    """

    def falsify(self, falsifier: dict | None, **absent: str) -> list[str]:
        registry = json.loads(self.registry_path.read_text(encoding="utf-8"))
        for candidate in registry["candidates"]:
            if candidate["id"] == "generic-cache":
                candidate["falsifier"] = falsifier
                if falsifier is None:
                    candidate["falsifier_absent"] = absent.get(
                        "reason", "nothing in a repository could show this"
                    )
        self.registry_path.write_text(
            json.dumps(registry, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        self.plan["rejected_candidates"] = [
            {
                "candidate_id": "generic-cache",
                "name": "generic-cache",
                "category": "integration",
                "reason": "No cache runtime wiring: config/cache.php is absent",
                "missing_evidence": ["cache call sites under app/"],
            }
        ]
        self.write_fixture()
        return [item.code for item in self.plan_diagnostics()]

    def write_target(self, relative: str, text: str) -> None:
        path = self.target / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def test_a_rejection_the_target_contradicts_is_reported(self) -> None:
        self.write_target("app/Cache/Warmer.php", "<?php\nuse CacheInterface;\n")
        codes = self.falsify(
            {
                "surface": "an in-app caching layer with real call sites",
                "requires": "any",
                "probes": [{"paths": ["app/**/*.php"], "pattern": "CacheInterface"}],
            }
        )
        self.assertIn("REJECTION_CONTRADICTED", codes)

    def test_a_rejection_the_target_confirms_is_left_alone(self) -> None:
        codes = self.falsify(
            {
                "surface": "an in-app caching layer with real call sites",
                "requires": "any",
                "probes": [{"paths": ["app/**/*.php"], "pattern": "CacheInterface"}],
            }
        )
        self.assertNotIn("REJECTION_CONTRADICTED", codes)

    def test_third_party_trees_do_not_establish_a_surface(self) -> None:
        # Measured: a `**/*.php` probe matched PHPUnit's own sources under
        # `bin/.phpunit` and reported a caching layer the project never wrote.
        self.write_target("vendor/acme/src/Cache.php", "<?php\nuse CacheInterface;\n")
        self.write_target("bin/.phpunit/src/Cache.php", "<?php\nuse CacheInterface;\n")
        self.write_target("node_modules/x/y.php", "<?php\nuse CacheInterface;\n")
        codes = self.falsify(
            {
                "surface": "an in-app caching layer with real call sites",
                "requires": "any",
                "probes": [{"paths": ["**/*.php"], "pattern": "CacheInterface"}],
            }
        )
        self.assertNotIn("REJECTION_CONTRADICTED", codes)

    def test_requires_all_needs_every_probe(self) -> None:
        self.write_target("app/Cache/Warmer.php", "<?php\n")
        falsifier = {
            "surface": "cache configuration together with call sites",
            "requires": "all",
            "probes": [
                {"paths": ["app/**/*.php"]},
                {"paths": ["config/cache.php"]},
            ],
        }
        self.assertNotIn("REJECTION_CONTRADICTED", self.falsify(falsifier))
        self.write_target("config/cache.php", "<?php\nreturn [];\n")
        self.assertIn("REJECTION_CONTRADICTED", self.falsify(falsifier))

    def test_at_least_counts_distinct_probes(self) -> None:
        self.write_target("app/Domain/Order.php", "<?php\n")
        falsifier = {
            "surface": "two or more named layers",
            "requires": "any",
            "at_least": 2,
            "probes": [
                {"paths": ["app/Domain/**"]},
                {"paths": ["app/Infrastructure/**"]},
            ],
        }
        self.assertNotIn("REJECTION_CONTRADICTED", self.falsify(falsifier))
        self.write_target("app/Infrastructure/Db.php", "<?php\n")
        self.assertIn("REJECTION_CONTRADICTED", self.falsify(falsifier))

    def test_a_candidate_without_a_falsifier_is_never_contradicted(self) -> None:
        self.write_target("app/Cache/Warmer.php", "<?php\nuse CacheInterface;\n")
        codes = self.falsify(None, reason="the trigger is a request, not a file")
        self.assertNotIn("REJECTION_CONTRADICTED", codes)


class FalsifierGlobTest(unittest.TestCase):
    def match(self, glob: str, path: str) -> bool:
        return bool(validator._glob_to_regex(glob).match(path))

    def test_double_star_crosses_directories(self) -> None:
        self.assertTrue(self.match("src/**/*.php", "src/a/b/C.php"))
        self.assertTrue(self.match("src/**/*.php", "src/C.php"))

    def test_single_star_stays_inside_one_segment(self) -> None:
        self.assertTrue(self.match("config/*.yaml", "config/cache.yaml"))
        self.assertFalse(self.match("config/*.yaml", "config/packages/cache.yaml"))

    def test_a_glob_anchors_at_both_ends(self) -> None:
        self.assertFalse(self.match("composer.json", "app/composer.json"))
        self.assertFalse(self.match("src/**/*.php", "src/a/b/C.phtml"))


class CatalogRoleCoverageTest(unittest.TestCase):
    """A selected candidate must carry its catalog obligations.

    Both measured plans filled `required_procedure_roles` with the same
    universal trio - load-evidence, execute, verify - for every skill: 9 of 9
    in one and 35 of 35 in the other. A trio that describes every skill ever
    written describes none of them, and the catalog's real obligations (derive
    constraints from invariants, cover denied paths) appeared nowhere.
    """

    MANDATORY = ["derive-constraints-from-invariants", "specify-migrations"]

    def diagnose(self, roles: list) -> list:
        diagnostics: list = []
        validator._validate_catalog_role_coverage(
            "database-designer",
            {"required_procedure_roles": roles},
            self.MANDATORY,
            diagnostics,
        )
        return sorted(item.code for item in diagnostics)

    def test_the_universal_trio_does_not_cover_a_candidates_obligations(self) -> None:
        self.assertEqual(
            self.diagnose([
                {"role": "load-evidence", "requirements": ["read"]},
                {"role": "execute", "requirements": ["do"]},
                {"role": "verify", "requirements": ["check"]},
            ]),
            ["CATALOG_ROLE_UNCOVERED"],
        )

    def test_declaring_every_mandatory_role_passes(self) -> None:
        self.assertEqual(
            self.diagnose([
                {"role": "derive-constraints-from-invariants", "requirements": ["x"]},
                {"role": "specify-migrations", "requirements": ["y"]},
                {"role": "load-evidence", "requirements": ["extra roles are fine"]},
            ]),
            [],
        )

    def test_a_partially_covered_candidate_is_still_reported(self) -> None:
        codes = self.diagnose(
            [{"role": "specify-migrations", "requirements": ["y"]}]
        )
        self.assertEqual(codes, ["CATALOG_ROLE_UNCOVERED"])

    def test_role_matching_ignores_case_and_padding(self) -> None:
        self.assertEqual(
            self.diagnose([
                {"role": "  Derive-Constraints-From-Invariants ", "requirements": ["x"]},
                {"role": "SPECIFY-MIGRATIONS", "requirements": ["y"]},
            ]),
            [],
        )


class RegistryRolesTest(unittest.TestCase):
    """The shipped registry must keep its declared obligations loadable."""

    def test_declared_roles_load_and_are_candidate_specific(self) -> None:
        registry = json.loads(
            (ROOT / ".agents/skills/skill-forge/references/candidate-registry.json")
            .read_text(encoding="utf-8")
        )
        with_roles = {
            item["id"]: item["roles"]
            for item in registry["candidates"]
            if item.get("roles")
        }
        self.assertTrue(with_roles, "no candidate declares catalog roles")
        universal = {"load-evidence", "execute", "verify"}
        for candidate, roles in with_roles.items():
            with self.subTest(candidate=candidate):
                self.assertTrue(all(isinstance(r, str) and r for r in roles))
                self.assertFalse(
                    universal & set(roles),
                    "the universal trio is a placeholder, not an obligation",
                )
                self.assertEqual(len(roles), len(set(roles)))

    def test_a_registry_with_declared_roles_still_loads(self) -> None:
        path = ROOT / ".agents/skills/skill-forge/references/candidate-registry.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        diagnostics: list = []
        loaded = validator._load_registry(
            path, data.get("catalog_version"), diagnostics
        )
        self.assertEqual([], [item.code for item in diagnostics])
        self.assertTrue(loaded)


class WriteVerificationTest(unittest.TestCase):
    """A skill that produces artifacts must exercise them, not grep for them.

    Two runs against a real project shipped a skill named `testing` that wrote
    to tests/** and verified itself with four greps, never invoking the suite.
    Searching for the text you just wrote proves authorship, never behaviour.
    """

    def diagnose(self, writes: list, checks: list) -> list:
        diagnostics: list = []
        validator._validate_write_verification(
            "sample-skill", {"writes": writes}, checks, diagnostics
        )
        return sorted(item.code for item in diagnostics)

    SEARCH = {"mode": "command", "command": 'grep -rn "x" src'}
    RUNNER = {"mode": "command", "command": "vendor/bin/phpunit"}
    MANUAL = {"mode": "manual", "command": ""}

    def test_a_write_capable_skill_verified_only_by_search_is_reported(self) -> None:
        self.assertEqual(
            self.diagnose(["tests/**"], [self.SEARCH, self.SEARCH]),
            ["WRITE_VERIFICATION_SEARCH_ONLY"],
        )

    def test_a_read_only_reviewer_may_verify_entirely_by_search(self) -> None:
        """Inspecting IS a reviewer's work; this must never fire on one."""
        self.assertEqual(self.diagnose([], [self.SEARCH, self.SEARCH]), [])

    def test_one_command_that_runs_something_satisfies_the_rule(self) -> None:
        self.assertEqual(
            self.diagnose(["tests/**"], [self.SEARCH, self.RUNNER]), []
        )

    def test_a_manual_check_is_an_honest_way_out(self) -> None:
        """A target with nothing runnable can still say so."""
        self.assertEqual(
            self.diagnose(["tests/**"], [self.SEARCH, self.MANUAL]), []
        )


class RuntimeCommandDescriptionTest(unittest.TestCase):
    """A runtime command may not be described as doing another one's job."""

    def diagnose(self, command: str, instruction: str, expected: str) -> list:
        diagnostics: list = []
        validator._validate_runtime_command_description(
            "project-brain",
            {"id": "check", "instruction": instruction, "expected_result": expected},
            command,
            diagnostics,
        )
        return sorted(item.code for item in diagnostics)

    PARITY = "python3 memory-bank/scripts/context.py parity --json"
    STATUS = "python3 memory-bank/scripts/context.py status"

    def test_borrowed_subject_matter_is_reported(self) -> None:
        """parity compares mirrors; validate inspects governed records."""
        self.assertEqual(
            self.diagnose(
                self.PARITY,
                "Run the runtime parity command and confirm the governed "
                "records agree with the runtime index",
                "The parity report shows no divergence between the governed "
                "records and the runtime index",
            ),
            ["RUNTIME_COMMAND_DESCRIPTION"],
        )

    def test_an_accurate_description_is_left_alone(self) -> None:
        self.assertEqual(
            self.diagnose(
                self.PARITY,
                "Run the parity command and confirm no mirror drift",
                "No canonical mirror drift is reported across the editions",
            ),
            [],
        )

    def test_a_flag_variant_is_not_a_rival(self) -> None:
        """`status` and `status --json` are one operation, not two.

        Letting them compete reported an accurate description of `status`
        merely because it mentioned the output.
        """
        self.assertEqual(
            self.diagnose(
                self.STATUS,
                "Run the runtime status command and confirm each memory layer "
                "reports its own state",
                "The status output names each memory layer and its current state",
            ),
            [],
        )


class RuntimeFixedAccountabilityTest(unittest.TestCase):
    """The runtime-fixed quartet is measured by runtime accuracy, not by
    project specificity it cannot have.

    `memory-bank`, `project-brain`, `checkpoint`, and `memory` describe the
    memory runtime `memory-seed` installs, so no target evidence is demanded
    of them. In exchange every path and command they name has to exist in
    `memory-seed/assets/runtime-contract.json`, and they may not name a target
    file they declare no evidence for.
    """

    PLAN = {
        "kind": "runtime-fixed",
        "source_paths": ["CLAUDE.md"],
        "evidence_ids": ["EV-0029"],
    }
    EVIDENCE = {"EV-0029": ("path", "CLAUDE.md")}

    def diagnose(self, body: str, plan: dict | None = None) -> list[str]:
        diagnostics: list = []
        validator._validate_runtime_fixed_body(
            "memory", plan if plan is not None else self.PLAN,
            body, self.EVIDENCE, diagnostics,
        )
        return sorted(item.code for item in diagnostics)

    def test_contract_paths_and_commands_are_accepted(self) -> None:
        body = (
            "Run `python3 memory-bank/scripts/context.py refresh`, then "
            "`python3 memory-bank/scripts/context.py status --json` and "
            "`python3 memory-bank/scripts/validate.py --summary`. Records live "
            "under `project-brain/dynamic/` and `project-brain/control/`; the "
            "index is `memory-bank/local/context.db`; the mode is read from "
            "`project-brain/config/runtime.json`."
        )
        self.assertEqual(self.diagnose(body), [])

    def test_tail_and_interior_path_fragments_are_accepted(self) -> None:
        """A skill may name `scripts/validate.py` or `control/` for short."""
        body = (
            "The durable validator is `scripts/validate.py`, chunks live in "
            "`chunks/`, the template is `templates/chunk.md`, and handoffs "
            "live under `control/`. See `memory-bank/scripts/validate.py`."
        )
        self.assertEqual(self.diagnose(body), [])

    def test_creatable_brace_group_is_expanded(self) -> None:
        body = "Task records are written to `project-brain/dynamic/tasks/`."
        self.assertEqual(self.diagnose(body), [])

    def test_the_contracts_own_brace_form_is_accepted_verbatim(self) -> None:
        # The contract writes this creatable path in brace form; a guide that
        # quotes it exactly must not be told it named an unsupported path.
        body = (
            "Records are written under "
            "`project-brain/dynamic/{tasks,findings,bugs,incidents,decisions,"
            "events}/*.md`."
        )
        self.assertEqual(self.diagnose(body), [])

    def test_a_brace_group_with_an_unsupported_member_still_blocks(self) -> None:
        body = "Records are written under `project-brain/dynamic/{tasks,rumours}/*.md`."
        self.assertEqual(self.diagnose(body), ["RUNTIME_PATH_UNSUPPORTED"])

    def test_forbidden_invented_runtime_path_blocks(self) -> None:
        body = "Checkpoints append to `memory-bank/local/checkpoints.jsonl`."
        self.assertEqual(self.diagnose(body), ["RUNTIME_PATH_FORBIDDEN"])

    def test_runtime_path_absent_from_the_contract_blocks(self) -> None:
        body = "The mode is read from `project-brain/state/mode.json`."
        self.assertEqual(self.diagnose(body), ["RUNTIME_PATH_UNSUPPORTED"])

    def test_unlisted_subcommand_blocks(self) -> None:
        body = "Run `python3 memory-bank/scripts/context.py reindex`."
        codes = self.diagnose(body)
        self.assertIn("RUNTIME_COMMAND_UNSUPPORTED", codes)

    def test_unlisted_flag_blocks(self) -> None:
        body = "Run `python3 memory-bank/scripts/context.py validate --deep`."
        self.assertEqual(self.diagnose(body), ["RUNTIME_COMMAND_UNSUPPORTED"])

    def test_declared_target_path_is_allowed(self) -> None:
        body = (
            "An empty result is explained by `CLAUDE.md`, not by a stale "
            "`memory-bank/local/context.db`."
        )
        self.assertEqual(self.diagnose(body), [])

    def test_undeclared_target_path_is_a_borrowed_project_claim(self) -> None:
        body = (
            "Capture the publication rule from `src/Entity/Article.php` into "
            "`memory-bank/chunks/MEM-001.md`."
        )
        self.assertEqual(
            self.diagnose(body), ["RUNTIME_PROJECT_CLAIM_UNSUPPORTED"]
        )

    def test_target_claim_needs_the_skill_own_evidence(self) -> None:
        """The same body passes once the skill actually declares the file."""
        body = "Capture the rule stated in `src/Entity/Article.php`."
        self.assertEqual(
            self.diagnose(body), ["RUNTIME_PROJECT_CLAIM_UNSUPPORTED"]
        )
        declared = dict(self.PLAN, source_paths=["src/Entity/Article.php"])
        self.assertEqual(self.diagnose(body, declared), [])

    def test_prose_code_spans_are_not_mistaken_for_paths(self) -> None:
        body = (
            "Report `working: skipped`, inspect `working_tasks` and "
            "`turn_deltas`, and honour schema `1.2` under "
            "`memory-bank/scripts/context.py`."
        )
        self.assertEqual(self.diagnose(body), [])

    def test_segment_runs_and_brace_expansion_are_exact(self) -> None:
        self.assertEqual(
            validator._expand_braces("a/{x,y}/b"), ["a/x/b", "a/y/b"]
        )
        self.assertTrue(
            validator._segment_run_matches(
                ["scripts", "validate.py"],
                ["memory-bank", "scripts", "validate.py"],
            )
        )
        self.assertFalse(
            validator._segment_run_matches(
                ["scripts", "chunk.md"],
                ["memory-bank", "scripts", "validate.py"],
            )
        )
        self.assertTrue(
            validator._segment_prefix_matches(
                ["project-brain", "records", "x.md"],
                ["project-brain", "records", "**"],
            )
        )
        self.assertFalse(
            validator._segment_prefix_matches(
                ["memory-bank"], ["memory-bank", "local", "checkpoints.jsonl"]
            )
        )

    def test_runtime_body_diagnostics_are_deterministic(self) -> None:
        body = (
            "Use `project-brain/state/mode.json` and "
            "`python3 memory-bank/scripts/context.py validate --deep`."
        )
        self.assertEqual(self.diagnose(body), self.diagnose(body))


class RuntimeFixedRoutingEvidenceTest(SkillQualityFixture):
    """C1: `routing_cases[].evidence_ids` cannot be required non-empty AND a
    subset of an empty declaration at the same time."""

    def blank_first_routing_evidence(self) -> list[str]:
        self.plan["skills"][0]["routing_cases"][0]["evidence_ids"] = []
        self.rewrite()
        return [item.code for item in self.plan_diagnostics()]

    def test_empty_routing_evidence_blocks_an_evidence_derived_skill(self) -> None:
        self.use_schema_1_4()
        self.assertIn("ROUTING_CASE_INVALID", self.blank_first_routing_evidence())

    def test_empty_routing_evidence_is_legal_for_a_runtime_fixed_skill(self) -> None:
        self.use_schema_1_4()
        self.plan["skills"][0]["kind"] = "runtime-fixed"
        self.assertNotIn(
            "ROUTING_CASE_INVALID", self.blank_first_routing_evidence()
        )


class SkillClassSplitTest(SkillQualityFixture):
    """The report must not merge derived skills with unconditional runtime
    guides into one 'skills for your project' number."""

    def test_split_names_both_classes(self) -> None:
        self.use_schema_1_4()
        self.plan["skills"][1]["kind"] = "runtime-fixed"
        self.rewrite()
        self.assertEqual(
            validator.skill_class_split(self.plan_path),
            {
                "project": [self.plan["skills"][0]["name"]],
                "runtime_fixed": [self.plan["skills"][1]["name"]],
            },
        )

    def test_unreadable_plan_reports_no_inventory(self) -> None:
        self.assertEqual(
            validator.skill_class_split(self.base / "absent.json"),
            {"project": [], "runtime_fixed": []},
        )


class SkillBodyPathTest(SkillQualityFixture):
    """A skill body may not send the agent to a file the target does not have.

    Plan `path_contracts` were resolved against the target, but the rendered
    prose was not, so a body could say "open `src/Security/Foo.php`" about a
    file that never existed and still pass. Prose is full of strings shaped
    like paths that are not target paths, so the reading is narrow on purpose:
    everything that cannot be told apart from an honest citation is skipped.
    """

    PLAN = {
        "kind": "project-derived",
        "source_paths": ["config/firebase.php"],
        "evidence_ids": ["firebase-runtime"],
        "path_contracts": [
            {
                "path": "config/firebase.php",
                "access": "read",
                "classification": "required-existing",
                "evidence_ids": ["firebase-runtime"],
            },
            {
                "path": "reports/firebase-boundary.md",
                "access": "write",
                "classification": "creatable",
                "evidence_ids": ["firebase-runtime"],
            },
        ],
        "writes": ["reports/firebase-boundary.md"],
    }
    EVIDENCE = {"firebase-runtime": ("path", "config/firebase.php")}

    def setUp(self) -> None:
        super().setUp()
        # A tree deep enough to separate "rooted in this project" from
        # "borrowed from somewhere else", plus the two installed trees whose
        # absence from a checkout proves nothing.
        self.write_target("src/Kernel.php", "<?php // kernel\n")
        self.write_target("src/Security/ContentVoter.php", "<?php // voter\n")
        self.write_target("tests/ExistingTest.php", "<?php // existing case\n")
        self.write_target("templates/page/show.html.twig", "<div></div>\n")
        self.write_target("vendor/autoload.php", "<?php // installed\n")
        self.write_target("var/cache/.gitkeep", "\n")

    def diagnose(self, body: str, plan: dict | None = None) -> list[str]:
        diagnostics: list = []
        validator._validate_body_paths(
            "firebase-services",
            self.PLAN if plan is None else plan,
            body,
            self.EVIDENCE,
            self.target.resolve(),
            diagnostics,
        )
        return sorted(item.code for item in diagnostics)

    def test_a_body_path_absent_from_the_target_is_reported(self) -> None:
        body = (
            "Open `src/Security/ImaginaryFirewallResolver.php` and confirm "
            "the resolver rejects the anonymous branch."
        )
        self.assertEqual(self.diagnose(body), ["SKILL_BODY_PATH_MISSING"])

    def test_creation_intent_survives_a_hard_wrap(self) -> None:
        """The verdict must not depend on where the text happened to wrap.

        Generated bodies are hard-wrapped near eighty columns, so the verb
        marking a path as one to be CREATED lands on the previous line as
        often as not. Reading only the physical line rejected an honest
        instruction whose sentence was identical but wrapped.
        """
        sentence = (
            "The suite has no expiry case; a reviewer would add at "
            "`tests/Booking/HoldExpiryTest.php` so the owner picks it up."
        )
        wrapped = sentence.replace("add at ", "add at\n", 1)
        self.assertEqual(self.diagnose(sentence), [])
        self.assertEqual(self.diagnose(wrapped), [])

    def test_creation_intent_does_not_leak_across_a_paragraph(self) -> None:
        """A blank line still bounds the intent.

        Widening the window to the paragraph must not let a create verb in
        one instruction excuse an invented path in the next.
        """
        body = (
            "A reviewer would add a regression case for the hold expiry.\n\n"
            "Open `src/Security/ImaginaryFirewallResolver.php` and confirm "
            "the resolver rejects the anonymous branch."
        )
        self.assertEqual(self.diagnose(body), ["SKILL_BODY_PATH_MISSING"])

    def test_real_target_paths_stay_clean(self) -> None:
        body = (
            "Read `config/firebase.php`, then `src/Kernel.php` and "
            "`src/Security/ContentVoter.php`; the rendered template is "
            "`templates/page/show.html.twig`."
        )
        self.assertEqual(self.diagnose(body), [])

    def test_class_names_keys_and_constants_are_not_paths(self) -> None:
        """Backslashed symbols, dotted keys, members, and bare file names."""
        body = (
            "The subject is `App\\Entity\\Page`, delivered through "
            "`Symfony\\Component\\Messenger\\Envelope`. The retry key is "
            "`retry_strategy.max_retries`, the attribute is `ROLE_ADMIN`, the "
            "constant is `CRUDEntityResolver::READ_PERMISSION`, the field is "
            "`article.content`, and the file is `security.yaml`."
        )
        self.assertEqual(self.diagnose(body), [])

    def test_placeholders_and_globs_name_a_set_not_a_file(self) -> None:
        body = (
            "Templates live at `templates/{type}/show.html.twig`, sources "
            "match `src/**/*.php`, environment overrides sit in "
            "`config/packages/{dev,prod}/doctrine.yaml`, and the entity is "
            "`src/Entity/<Name>.php` for `{id}`."
        )
        self.assertEqual(self.diagnose(body), [])

    def test_foreign_and_installed_trees_are_left_alone(self) -> None:
        """A path is judged only when it is rooted in this target's own tree.

        `app/` belongs to another framework's layout, `page/show.html.twig` is
        a Twig logical name resolved under `templates/`, and `vendor/`,
        `node_modules/`, `var/` are installed or generated rather than
        committed - none of them is evidence of an invented file.
        """
        body = (
            "Other projects put it in `app/Http/Controllers/PageController.php`. "
            "The controller renders `page/show.html.twig`. Run "
            "`vendor/bin/phpunit`, lint with `node_modules/.bin/eslint`, and "
            "inspect `var/cache/dev/AppKernelDevDebugContainer.php`."
        )
        self.assertEqual(self.diagnose(body), [])

    def test_a_path_the_skill_declares_it_writes_is_not_missing(self) -> None:
        body = "The bounded artifact of this review is `reports/firebase-boundary.md`."
        self.assertEqual(self.diagnose(body), [])

    def test_a_path_the_body_commands_into_existence_is_not_missing(self) -> None:
        """A file the skill tells the agent to create does not exist yet."""
        for line in (
            "Create `tests/AnonymousApiTest.php` beside the existing suite.",
            "Add `tests/Security/DeniedPathTest.php` to the suite.",
            "Generate `src/Security/AnonymousGuard.php` from the template.",
            "Write `src/Security/TokenAuthenticator.php` for the api firewall.",
        ):
            self.assertEqual(self.diagnose(line), [], line)

    def test_creation_verbs_do_not_swallow_their_look_alikes(self) -> None:
        """`authorization`, `additional`, `placeholder` are not creation."""
        for line in (
            "The authorization branch lives in "
            "`src/Security/ImaginaryFirewallResolver.php`.",
            "One additional check sits in `src/Security/ImaginaryAudit.php`.",
            "The placeholder is `src/Security/ImaginaryPlaceholder.php`.",
        ):
            self.assertEqual(self.diagnose(line), ["SKILL_BODY_PATH_MISSING"], line)

    def test_a_path_quoted_from_the_cited_source_is_a_report(self) -> None:
        """Configuration may declare a directory that was never created.

        `unite.yaml` in the Symfony run lists `src/Model` as a schema type
        dir although the directory does not exist. Reporting what the cited
        file says is the skill doing its job, so a string the target itself
        spells out is never called an invention.
        """
        self.write_target(
            "config/firebase.php",
            "<?php // Firebase initialization, messaging client, and runtime "
            "boundary.\n// type_dirs: src/Entity, src/Model, src/Union\n",
        )
        body = "Types are collected from `src/Entity`, `src/Model` and `src/Union`."
        self.assertEqual(self.diagnose(body), [])
        # The same three strings, from a skill that cites nothing, stay invented.
        self.assertEqual(
            self.diagnose(body, {"kind": "project-derived"}),
            ["SKILL_BODY_PATH_MISSING"] * 3,
        )

    def test_a_path_below_an_absent_directory_is_left_uncovered(self) -> None:
        """A documented miss, kept deliberately.

        `src/Security/Firewall/Resolver/` does not exist, so the fabricated
        file under it is indistinguishable from a path belonging to a layout
        this target simply does not use. Skipping is honester than guessing.
        """
        body = "Open `src/Security/Firewall/Resolver/ImaginaryResolver.php`."
        self.assertEqual(self.diagnose(body), [])

    def test_body_path_diagnostics_are_deterministic(self) -> None:
        body = (
            "Read `src/Security/ImaginaryTwo.php`, then "
            "`src/Security/ImaginaryOne.php`, then `src/Security/ImaginaryTwo.php`."
        )
        diagnostics: list = []
        validator._validate_body_paths(
            "firebase-services", self.PLAN, body, self.EVIDENCE,
            self.target.resolve(), diagnostics,
        )
        self.assertEqual(
            [item.message for item in sorted(set(diagnostics))],
            [
                "firebase-services: body sends the agent to a target path that "
                "does not exist: src/Security/ImaginaryOne.php",
                "firebase-services: body sends the agent to a target path that "
                "does not exist: src/Security/ImaginaryTwo.php",
            ],
        )

    def test_the_gate_reports_an_invented_body_path_end_to_end(self) -> None:
        self.use_schema_1_4()
        self.write_target("src/Kernel.php", "<?php // kernel\n")
        name = "firebase-services"
        self.skill_texts[name] = self.skill_texts[name].replace(
            "\n## Verification\n",
            "\nOpen `src/ImaginaryFirebaseBridge.php` and confirm the "
            "transport is bound.\n\n## Verification\n",
        )
        self.rewrite()
        self.assertIn("SKILL_BODY_PATH_MISSING", self.codes())


class SkillEvidenceRowTest(SkillQualityFixture):
    """The rendered evidence table is the skill's citation of record.

    Nothing compared it with the plan, so a row could keep a real evidence
    identifier and re-point it at a file the plan never declared. Identifier
    and path are now both held to what the plan says.
    """

    PLAN = {"evidence_ids": ["firebase-runtime"]}
    EVIDENCE = {
        "firebase-runtime": ("path", "config/firebase.php"),
        "availability-rules": ("path", "domain/availability.php"),
    }

    def diagnose(self, body: str) -> list[str]:
        diagnostics: list = []
        validator._validate_evidence_rows(
            "firebase-services", self.PLAN, body, self.EVIDENCE, diagnostics
        )
        return sorted(item.code for item in diagnostics)

    def row(self, identifier: str, anchor: str) -> str:
        return (
            "| Evidence | Anchor | Source type | Confidence | Claim |\n"
            "| --- | --- | --- | --- | --- |\n"
            f"| {identifier} | `{anchor}` | configuration | confirmed | "
            "The messaging transport is declared here. |\n"
        )

    def test_an_honest_row_is_clean(self) -> None:
        self.assertEqual(
            self.diagnose(self.row("firebase-runtime", "config/firebase.php:L1-L4")),
            [],
        )

    def test_a_symbol_anchor_resolves_to_its_path(self) -> None:
        self.assertEqual(
            self.diagnose(
                self.row(
                    "firebase-runtime",
                    "config/firebase.php:symbol:FirebaseMessagingClient",
                )
            ),
            [],
        )

    def test_a_re_pointed_anchor_is_reported(self) -> None:
        self.assertEqual(
            self.diagnose(
                self.row(
                    "firebase-runtime",
                    "config/packages/totally-made-up-firebase.yaml:L1-L5",
                )
            ),
            ["SKILL_EVIDENCE_ROW_ANCHOR"],
        )

    def test_a_row_citing_evidence_this_skill_was_not_given_is_reported(self) -> None:
        self.assertEqual(
            self.diagnose(self.row("availability-rules", "domain/availability.php:L1")),
            ["SKILL_EVIDENCE_ROW_UNDECLARED"],
        )

    def test_a_row_citing_evidence_the_plan_never_declared_is_reported(self) -> None:
        self.assertEqual(
            self.diagnose(self.row("EV-9999", "config/firebase.php:L1")),
            ["SKILL_EVIDENCE_ROW_UNDECLARED"],
        )

    def test_an_ordinary_table_is_not_an_evidence_table(self) -> None:
        body = (
            "| Case | Expected |\n"
            "| --- | --- |\n"
            "| Transient provider failure | retry once, then report |\n"
        )
        self.assertEqual(self.diagnose(body), [])

    def test_a_row_without_a_path_claims_no_location(self) -> None:
        body = (
            "| Evidence | Claim |\n"
            "| --- | --- |\n"
            "| firebase-runtime | The transport is declared. |\n"
        )
        self.assertEqual(self.diagnose(body), [])

    def test_the_gate_reports_a_re_pointed_row_end_to_end(self) -> None:
        self.use_schema_1_4()
        name = "firebase-services"
        self.skill_texts[name] = self.skill_texts[name].replace(
            "\n## Procedure / Process\n",
            "\n"
            + self.row("firebase-runtime", "config/invented-firebase.yaml:L1-L5")
            + "\n## Procedure / Process\n",
        )
        self.rewrite()
        self.assertIn("SKILL_EVIDENCE_ROW_ANCHOR", self.codes())


class SearchVerificationTest(SkillQualityFixture):
    """A search verification is graded against what the target really holds.

    The shapes below are copied from a Symfony target where a generated skill
    shipped an exclusive claim the unchanged checkout already contradicted.
    """

    SETTER = (
        "<?php\n\nclass %s\n{\n"
        "    public function setState(string $state): self\n"
        "    {\n        $this->state = $state;\n\n        return $this;\n    }\n}\n"
    )

    def setUp(self) -> None:
        super().setUp()
        self.use_schema_1_4()
        self.write_target("src/Entity/Article.php", self.SETTER % "Article")
        self.write_target("src/Entity/Job.php", self.SETTER % "Job")
        self.write_target(
            "src/Entity/Page.php",
            "<?php\n\nclass Page\n{\n    private string $title;\n}\n",
        )
        self.write_target(
            "src/Importer/WordpressArticleImporter.php",
            "<?php\n\nclass WordpressArticleImporter\n{\n"
            "    public function import(Article $article): void\n"
            "    {\n        $article->setState('published');\n    }\n}\n",
        )
        self.write_target(
            "src/Controller/PageController.php",
            "<?php\n\nclass PageController\n{\n"
            "    public function show(Page $page): Response\n"
            "    {\n        if ($page->getState() !== 'published') {\n"
            "            throw new NotFoundHttpException();\n        }\n    }\n}\n",
        )
        self.write_target(
            "src/Preprocessor/NewsPreprocessor.php",
            "<?php\n\nclass NewsPreprocessor\n{\n"
            "    public function filter(): array\n"
            "    {\n        return ['state' => 'published'];\n    }\n}\n",
        )

    def search(
        self,
        command: str,
        expected: str,
        skip_condition: str = "src/ is absent from the checkout",
    ) -> list[str]:
        check = self.plan["skills"][0]["verification"][0]
        check["mode"] = "command"
        check["command"] = command
        check["expected_result"] = expected
        check["skip_condition"] = skip_condition
        self.rewrite()
        return [
            item.code
            for item in self.plan_diagnostics()
            if item.code.startswith("VERIFICATION_SEARCH_")
        ]

    def test_an_expectation_of_no_output_is_not_a_dead_check(self) -> None:
        """"Confirm nothing remains" is a normal check, not a broken one.

        Grading an absence claim as dead inverts its meaning: the empty
        result is exactly the success the author declared.
        """
        for expected in (
            "Nothing prints, confirming no dump call survives.",
            "No output at all: the debug helper is gone from src.",
            "No matches remain, so the legacy call is fully removed.",
        ):
            with self.subTest(expected=expected):
                self.assertEqual(
                    self.search('grep -rn "var_dump" src', expected), []
                )

    def test_a_path_the_expectation_forbids_is_not_demanded(self) -> None:
        """A contrastive expectation states the rule, it does not break it.

        "Only A assigns the state; B must not appear" names B precisely so
        that B is absent. Demanding B in the output rejects the clearest
        possible phrasing.
        """
        codes = self.search(
            'grep -rn "setState" src',
            "src/Importer/WordpressArticleImporter.php assigns the state; "
            "src/Entity/Article.php must not appear.",
        )
        # The forbidden path must not be demanded. Any exclusivity grading is
        # a separate rule and is deliberately not asserted here.
        self.assertNotIn("VERIFICATION_SEARCH_EXPECTATION", codes)

    def test_a_prohibition_does_not_leak_to_a_neighbouring_clause(self) -> None:
        """The negation is bounded, so an honest expectation still grades."""
        codes = self.search(
            'grep -rn "setState" src',
            "src/Nowhere/Absent.php must not appear. "
            "src/Entity/Article.php prints its declaration.",
        )
        self.assertEqual(codes, [])

    def test_an_exclusive_claim_the_target_contradicts_is_reported(self) -> None:
        """The reproduced defect: three files answer a claim naming one."""
        codes = self.search(
            'grep -rn "setState" src',
            "Only the Wordpress importer's setState published assignment "
            "prints, so every movement still goes through the workflow registry",
        )
        self.assertEqual(codes, ["VERIFICATION_SEARCH_EXCLUSIVITY"])

    def test_the_surplus_files_are_named_in_sorted_order(self) -> None:
        check = self.plan["skills"][0]["verification"][0]
        check["mode"] = "command"
        check["command"] = 'grep -rn "setState" src'
        check["expected_result"] = (
            "Only the Wordpress importer's setState published assignment prints"
        )
        check["skip_condition"] = "src/ is absent from the checkout"
        self.rewrite()
        message = next(
            item.message
            for item in self.plan_diagnostics()
            if item.code == "VERIFICATION_SEARCH_EXCLUSIVITY"
        )
        self.assertIn("src/Entity/Article.php, src/Entity/Job.php", message)
        self.assertNotIn("WordpressArticleImporter", message)

    def test_a_search_without_an_exclusivity_claim_is_left_alone(self) -> None:
        """Calibration: the honest sibling check from the same plan."""
        self.assertEqual(
            self.search(
                'grep -rn "published" src/Controller src/Preprocessor',
                "Every changed read path prints a published comparison, and "
                "PageController::show together with the news listing builder "
                "still print theirs",
                skip_condition="src/Controller or src/Preprocessor is absent "
                "from the checkout",
            ),
            [],
        )

    def test_a_hyphenated_scope_adjective_is_not_an_exclusivity_claim(self) -> None:
        """Calibration: "admin-only" describes scope, not the printed output."""
        self.assertEqual(
            self.search(
                'grep -rn "setState" src',
                "Each entity prints its setState declaration, so the mutating "
                "attributes stay admin-only",
            ),
            [],
        )

    def test_an_exclusivity_claim_that_names_no_file_is_not_graded(self) -> None:
        self.assertEqual(
            self.search(
                'grep -rn "setState" src',
                "Only three declaration lines print, and none of them assigns a "
                "state outside a transition",
            ),
            [],
        )

    def test_a_search_the_clean_target_never_answers_is_a_dead_check(self) -> None:
        self.assertEqual(
            self.search(
                'grep -rn "setPublicationState" src',
                "Only the importer prints a direct publication state assignment",
            ),
            ["VERIFICATION_SEARCH_DEAD"],
        )

    def test_a_named_path_outside_the_output_is_reported(self) -> None:
        self.assertEqual(
            self.search(
                'grep -rn "setState" src',
                "src/Entity/Page.php prints its state assignment together with "
                "the importer",
            ),
            ["VERIFICATION_SEARCH_EXPECTATION"],
        )

    def test_a_named_path_the_search_never_covers_is_not_graded(self) -> None:
        """A path outside the searched scope is not a claim about this output."""
        self.assertEqual(
            self.search(
                'grep -rn "setState" src/Importer',
                "The importer prints its assignment; src/Entity/Job.php is "
                "reviewed by the entity skill instead",
                skip_condition="src/Importer is absent from the checkout",
            ),
            [],
        )

    def test_a_word_search_respects_word_boundaries(self) -> None:
        self.write_target(
            "src/Legacy/Alias.php",
            "<?php\n\nclass Alias\n{\n    public function unsetStateFlag(): void\n"
            "    {\n    }\n}\n",
        )
        self.assertEqual(
            self.search(
                'grep -rnw "setState" src/Legacy',
                "Only the legacy alias prints a bare setState call",
                skip_condition="src/Legacy is absent from the checkout",
            ),
            ["VERIFICATION_SEARCH_DEAD"],
        )
        self.assertEqual(
            self.search(
                'grep -rn "setState" src/Legacy',
                "The legacy alias prints its setState substring",
                skip_condition="src/Legacy is absent from the checkout",
            ),
            [],
        )

    def test_a_binary_file_is_not_line_evidence(self) -> None:
        blob = self.target / "src/Legacy/dump.bin"
        blob.parent.mkdir(parents=True, exist_ok=True)
        blob.write_bytes(b"\x00\x01setState\x00")
        self.assertEqual(
            self.search(
                'grep -rn "setState" src/Legacy',
                "The legacy dump prints a setState assignment",
                skip_condition="src/Legacy is absent from the checkout",
            ),
            ["VERIFICATION_SEARCH_DEAD"],
        )

    def test_skip_condition_absorbs_a_path_that_may_be_absent(self) -> None:
        self.assertEqual(
            self.search(
                'grep -rn "setState" src legacy',
                "Only the Wordpress importer's setState assignment prints",
                skip_condition="src/ or legacy/ is absent from the checkout",
            ),
            ["VERIFICATION_SEARCH_EXCLUSIVITY"],
        )

    def test_a_missing_path_no_skip_condition_covers_is_not_graded(self) -> None:
        self.assertEqual(
            self.search(
                'grep -rn "setState" src legacy',
                "Only the Wordpress importer's setState assignment prints",
                skip_condition="The working tree is dirty",
            ),
            [],
        )

    def test_unparseable_search_forms_are_declined(self) -> None:
        expected = "Only the Wordpress importer's setState assignment prints"
        for command in (
            'grep -rn -A3 "setState" src',
            'grep -rn -e "setState" -e "setStatus" src',
            'grep -rvn "setState" src',
            'grep -rn "set.*State" src',
            'grep -rn "setState" src | head -3',
            'grep -rn "setState" $(pwd)/src',
            'grep -rn "setState" ../src',
            'grep -rn "setState"',
            'rg -n "setState" src',
        ):
            with self.subTest(command=command):
                self.assertEqual(self.search(command, expected), [])

    def test_a_directory_searched_without_recursion_is_declined(self) -> None:
        self.assertEqual(
            self.search(
                'grep -n "setState" src',
                "Only the Wordpress importer's setState assignment prints",
            ),
            [],
        )

    def test_a_symlinked_path_is_never_followed(self) -> None:
        link = self.target / "mirror"
        try:
            link.symlink_to(self.target / "src")
        except OSError:  # pragma: no cover - platform without symlink support
            self.skipTest("symlinks unavailable")
        self.assertEqual(
            self.search(
                'grep -rn "setState" mirror',
                "Only the Wordpress importer's setState assignment prints",
                skip_condition="mirror is absent from the checkout",
            ),
            [],
        )

    def test_a_graded_search_does_not_disturb_the_rest_of_the_gate(self) -> None:
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertEqual(codes, [])


class RoleCoverageWiringTest(SkillQualityFixture):
    """Schema 1.3 wires each declared obligation to evidence and to a step.

    Schema 1.2 accepted a role plus a sentence, so an obligation could be
    discharged by writing it down. These cases pin the difference.
    """

    def roles(self, index: int = 0) -> list:
        return self.plan["skills"][index]["required_procedure_roles"]

    def test_an_untyped_role_entry_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.roles()[0] = {"role": "inspect", "requirements": ["inspect initialization"]}
        self.rewrite()
        self.assertIn(
            "PROCEDURE_ROLE_CONTRACT_INVALID",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_role_supported_by_evidence_the_skill_does_not_cite_is_rejected(
        self,
    ) -> None:
        self.use_schema_1_4()
        self.roles()[0]["evidence_ids"] = ["availability-rules"]
        self.rewrite()
        self.assertIn(
            "PROCEDURE_ROLE_EVIDENCE_UNKNOWN",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_role_naming_a_step_that_does_not_exist_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.roles()[0]["procedure_step_ids"] = ["no-such-step"]
        self.rewrite()
        self.assertIn(
            "PROCEDURE_ROLE_STEP_UNKNOWN",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_every_obligation_on_one_step_is_reported_as_collapsed(self) -> None:
        self.use_schema_1_4()
        for role in self.roles():
            role["procedure_step_ids"] = ["trace-confirmed-claim"]
        self.rewrite()
        self.assertIn(
            "PROCEDURE_ROLE_COLLAPSED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_two_obligations_sharing_one_step_are_not_called_collapsed(self) -> None:
        """The false-rejection edge: a small skill may honestly share a step."""
        self.use_schema_1_4()
        for skill in self.plan["skills"]:
            skill["required_procedure_roles"] = skill["required_procedure_roles"][:2]
            for role in skill["required_procedure_roles"]:
                role["procedure_step_ids"] = ["trace-confirmed-claim"]
        self.rewrite()
        self.assertNotIn(
            "PROCEDURE_ROLE_COLLAPSED",
            [item.code for item in self.plan_diagnostics()],
        )


class VerificationBaselineTest(SkillQualityFixture):
    """ADR-002: the gate cannot run `eslint`, so it compares what was recorded.

    The defect these pin is measured, not hypothetical: a generated skill
    declared `eslint assets` exits zero on a target where it reports 189
    errors, so the skill raised a blocking finding on untouched code on every
    run.
    """

    def check(self, index: int = 0) -> dict:
        return self.plan["skills"][index]["verification"][0]

    def test_an_executable_check_without_a_recorded_baseline_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.check()["baseline"] = None
        self.rewrite()
        self.assertIn(
            "VERIFICATION_BASELINE_MISSING",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_promising_success_against_a_failing_baseline_is_rejected(self) -> None:
        self.use_schema_1_4()
        check = self.check()
        check["baseline"]["outcome"] = "failing"
        check["baseline"]["observed"] = "exit 1; 189 errors reported"
        check["expected_result"] = "The command exits zero and reports no errors"
        self.rewrite()
        self.assertIn(
            "VERIFICATION_BASELINE_CONTRADICTED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_differential_expectation_against_the_same_baseline_passes(self) -> None:
        self.use_schema_1_4()
        check = self.check()
        check["baseline"]["outcome"] = "failing"
        check["baseline"]["observed"] = "exit 1; 189 errors reported"
        check["expected_result"] = (
            "The command reports no error outside the recorded baseline"
        )
        self.rewrite()
        self.assertNotIn(
            "VERIFICATION_BASELINE_CONTRADICTED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_baseline_recording_a_different_command_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.check()["baseline"]["command"] = "php -l composer.json"
        self.rewrite()
        self.assertIn(
            "VERIFICATION_BASELINE_COMMAND_MISMATCH",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_read_only_skill_may_not_claim_to_remediate_the_failure(self) -> None:
        self.use_schema_1_4()
        review = self.plan["skills"][1]
        source = review["source_paths"][0]
        check = review["verification"][0]
        check["mode"] = "command"
        check["command"] = f"php -l {source}"
        check["baseline"] = {
            "command": f"php -l {source}",
            "observed": "exit 1; one parse error reported",
            "outcome": "failing-remediated",
        }
        self.rewrite()
        self.assertIn(
            "VERIFICATION_BASELINE_REMEDIATION_CONFLICT",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_search_baseline_is_cross_checked_against_this_gates_resolution(
        self,
    ) -> None:
        """A recorded baseline is trusted only where the gate cannot resolve."""
        self.use_schema_1_4()
        check = self.check()
        check["command"] = "grep -rn NoSuchToken config"
        check["baseline"] = {
            "command": "grep -rn NoSuchToken config",
            "observed": "exit 0; one match",
            "outcome": "passing",
        }
        self.rewrite()
        self.assertIn(
            "VERIFICATION_BASELINE_CONTRADICTED",
            [item.code for item in self.plan_diagnostics()],
        )


class AbsenceEvidenceTest(SkillQualityFixture):
    """Schema 1.3 lets the ledger record what the target does not do.

    "PHPStan is installed and invoked from nowhere" could not be recorded at
    all: every entry needed a path and a fingerprint. The absence carries a
    search this gate resolves itself, so the negative is pinned to a file set
    rather than asserted.
    """

    def add_absence(self, **overrides) -> None:
        absence = {
            "subject": "No configuration invokes phpstan",
            "search": "grep -rn phpstan config",
            "accounted_matches": [],
        }
        absence.update(overrides)
        self.plan["evidence"].append(
            {
                "id": "phpstan-uninvoked",
                "absence": absence,
                "source_type": "absence",
                "authority": "Establishes that nothing in config runs the analyser",
                "confidence": "confirmed",
                "supported_claims": ["phpstan is invoked from nowhere in config"],
            }
        )
        self.rewrite()

    def test_a_resolved_absence_is_accepted(self) -> None:
        self.use_schema_1_4()
        self.add_absence()
        codes = [item.code for item in self.plan_diagnostics()]
        self.assertEqual([code for code in codes if code.startswith("EVIDENCE")], [])

    def test_an_absence_the_target_contradicts_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.add_absence(
            subject="No configuration mentions Firebase",
            search="grep -rn Firebase config",
        )
        self.assertIn(
            "EVIDENCE_ABSENCE_CONTRADICTED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_an_accounted_match_that_no_longer_exists_is_stale(self) -> None:
        self.use_schema_1_4()
        self.add_absence(accounted_matches=["config/phpstan.neon"])
        self.assertIn(
            "EVIDENCE_ABSENCE_STALE",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_an_absence_this_gate_cannot_resolve_is_refused(self) -> None:
        self.use_schema_1_4()
        self.add_absence(search="composer show | grep phpstan")
        self.assertIn(
            "EVIDENCE_ABSENCE_UNRESOLVABLE",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_an_absence_missing_its_search_is_refused(self) -> None:
        self.use_schema_1_4()
        self.plan["evidence"].append(
            {
                "id": "phpstan-uninvoked",
                "absence": {"subject": "No configuration invokes phpstan"},
                "source_type": "absence",
                "authority": "Establishes that nothing in config runs the analyser",
                "confidence": "confirmed",
                "supported_claims": ["phpstan is invoked from nowhere in config"],
            }
        )
        self.rewrite()
        self.assertIn(
            "EVIDENCE_ABSENCE_INVALID",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_an_absence_claim_must_name_what_was_searched_for(self) -> None:
        self.use_schema_1_4()
        self.plan["evidence"].append(
            {
                "id": "phpstan-uninvoked",
                "absence": {
                    "subject": "No configuration invokes phpstan",
                    "search": "grep -rn phpstan config",
                    "accounted_matches": [],
                },
                "source_type": "absence",
                "authority": "Establishes that nothing in config runs the analyser",
                "confidence": "confirmed",
                "supported_claims": ["The release pipeline skips staging approval"],
            }
        )
        self.rewrite()
        self.assertIn(
            "EVIDENCE_CLAIM_UNSUPPORTED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_an_absence_may_not_also_cite_a_path(self) -> None:
        self.use_schema_1_4()
        self.plan["evidence"].append(
            {
                "id": "phpstan-uninvoked",
                "path": "composer.json",
                "absence": {
                    "subject": "No configuration invokes phpstan",
                    "search": "grep -rn phpstan config",
                    "accounted_matches": [],
                },
                "source_type": "absence",
                "authority": "Establishes that nothing in config runs the analyser",
                "confidence": "confirmed",
                "supported_claims": ["phpstan is invoked from nowhere in config"],
            }
        )
        self.rewrite()
        self.assertIn(
            "EVIDENCE_LOCATION",
            [item.code for item in self.plan_diagnostics()],
        )


class RoutingTautologyTest(SkillQualityFixture):
    """A fixture that names its own answer tests nothing about routing."""

    def test_a_prompt_naming_the_expected_skill_is_rejected(self) -> None:
        self.use_schema_1_4()
        skill = self.plan["skills"][0]
        skill["routing_cases"][0]["prompt"] = (
            f"Route {skill['name']} work to {skill['name']}"
        )
        self.rewrite()
        self.assertIn(
            "ROUTING_CASE_TAUTOLOGICAL",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_prompt_describing_the_request_is_accepted(self) -> None:
        self.use_schema_1_4()
        self.assertNotIn(
            "ROUTING_CASE_TAUTOLOGICAL",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_runtime_fixed_skill_may_be_named_by_its_own_subject(self) -> None:
        """`memory` cannot describe a memory request without saying memory."""
        self.use_schema_1_4()
        skill = self.plan["skills"][0]
        skill["kind"] = "runtime-fixed"
        skill["evidence_ids"] = []
        skill["source_paths"] = []
        skill["routing_cases"][0]["prompt"] = (
            f"Reload the {skill['name']} layers and report their health"
        )
        self.rewrite()
        self.assertNotIn(
            "ROUTING_CASE_TAUTOLOGICAL",
            [item.code for item in self.plan_diagnostics()],
        )


class EvidenceDispositionTest(SkillQualityFixture):
    """Schema 1.4: evidence inside a skill's own paths needs a decision.

    Synthesis reads the whole ledger and writes one contract at a time, so a
    passed-over finding leaves no trace: the plan looks the same whether the
    author judged it irrelevant or never saw it.
    """

    def relocate_sibling_evidence(self) -> str:
        """Put the second skill's evidence inside the first skill's territory."""
        first, second = self.plan["skills"]
        contract = first["path_contracts"][0]["path"]
        directory = contract.rsplit("/", 1)[0] if "/" in contract else "config"
        first["ownership"][0]["paths"] = [f"{directory}/**"]
        first["path_contracts"][0]["path"] = f"{directory}/**"
        moved = second["evidence_ids"][0]
        entry = next(
            item for item in self.plan["evidence"] if item["id"] == moved
        )
        entry["path"] = f"{directory}/{entry['path'].rsplit('/', 1)[-1]}"
        self.write_target(entry["path"], "<?php // relocated evidence\n")
        self.refingerprint(moved)
        second["source_paths"] = [entry["path"]]
        second["path_contracts"][0]["path"] = entry["path"]
        for anchor in second["evidence_anchors"]:
            anchor["anchor"] = f"{entry['path']}:L1"
        self.rewrite()
        return moved

    def test_evidence_inside_the_declared_paths_needs_a_decision(self) -> None:
        self.use_schema_1_4()
        self.relocate_sibling_evidence()
        self.assertIn(
            "SKILL_EVIDENCE_UNDISPOSED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_ruling_it_out_with_a_reason_is_accepted(self) -> None:
        self.use_schema_1_4()
        moved = self.relocate_sibling_evidence()
        self.plan["skills"][0]["evidence_dispositions"] = [
            {
                "evidence_id": moved,
                "disposition": "excluded",
                "reason": "Availability domain behavior is the sibling's subject",
            }
        ]
        self.rewrite()
        self.assertNotIn(
            "SKILL_EVIDENCE_UNDISPOSED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_ruling_out_evidence_the_plan_does_not_carry_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.plan["skills"][0]["evidence_dispositions"] = [
            {
                "evidence_id": "no-such-evidence",
                "disposition": "excluded",
                "reason": "Not relevant to this skill",
            }
        ]
        self.rewrite()
        self.assertIn(
            "EVIDENCE_DISPOSITION_UNKNOWN",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_citing_and_ruling_out_the_same_evidence_is_rejected(self) -> None:
        self.use_schema_1_4()
        skill = self.plan["skills"][0]
        skill["evidence_dispositions"] = [
            {
                "evidence_id": skill["evidence_ids"][0],
                "disposition": "excluded",
                "reason": "Contradicts the citation above",
            }
        ]
        self.rewrite()
        self.assertIn(
            "EVIDENCE_DISPOSITION_CONTRADICTED",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_an_untyped_disposition_is_rejected(self) -> None:
        self.use_schema_1_4()
        self.plan["skills"][0]["evidence_dispositions"] = [{"evidence_id": "x"}]
        self.rewrite()
        self.assertIn(
            "EVIDENCE_DISPOSITION_INVALID",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_skill_must_name_the_claims_it_rests_on(self) -> None:
        self.use_schema_1_4()
        self.plan["skills"][0]["claim_ids"] = []
        self.rewrite()
        self.assertIn(
            "SKILL_CLAIM_IDS_INVALID",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_a_runtime_fixed_skill_rests_on_no_project_claim(self) -> None:
        self.use_schema_1_4()
        skill = self.plan["skills"][0]
        skill["kind"] = "runtime-fixed"
        skill["evidence_ids"] = []
        skill["source_paths"] = []
        skill["claim_ids"] = []
        self.rewrite()
        self.assertNotIn(
            "SKILL_CLAIM_IDS_INVALID",
            [item.code for item in self.plan_diagnostics()],
        )

    def test_schema_1_3_is_audit_only_since_the_1_4_migration(self) -> None:
        self.use_schema_1_3()
        diagnostics = self.plan_diagnostics()
        self.assertIn("PLAN_SCHEMA_MIGRATION", [item.code for item in diagnostics])
        self.assertFalse(any(item.severity == "error" for item in diagnostics))
        authored = validator.validate(
            self.skills, self.plan_path, self.target, self.registry_path
        )
        self.assertIn(
            "LEGACY_PLAN_PUBLICATION_INELIGIBLE",
            [item.code for item in authored],
        )


class ContractRenderingTest(SkillQualityFixture):
    """Section 7: the forge renders the approved contract, not a summary of it.

    The field-level trace checks grade a whole contract member as one bag of
    words, so a body could satisfy `required_procedure_roles` by echoing two
    words from any one role. An approved step that never reaches the page was
    dropped, whatever else got written.
    """

    def test_an_approved_step_that_never_reaches_the_page_is_reported(self) -> None:
        self.use_schema_1_4()
        skill = self.plan["skills"][0]
        skill["procedure_steps"].append(
            {
                "id": "sanitize-provider-payload",
                "action": (
                    "Redact the customer telephone identifiers before writing "
                    "the boundary report"
                ),
                "evidence_ids": [skill["evidence_ids"][0]],
                "path_refs": [skill["source_paths"][0]],
                "decision_refs": [skill["decision_points"][0]["id"]],
                "expected_outcome": "No telephone identifier reaches the report",
                "failure_branch": "Stop and report the unredacted identifier",
            }
        )
        self.rewrite()
        self.assertIn("SKILL_STEP_NOT_RENDERED", self.codes())

    def test_a_step_the_body_carries_is_accepted(self) -> None:
        self.use_schema_1_4()
        self.assertNotIn("SKILL_STEP_NOT_RENDERED", self.codes())
        self.assertNotIn("SKILL_ROLE_NOT_RENDERED", self.codes())

    def test_an_obligation_that_never_reaches_the_page_is_reported(self) -> None:
        self.use_schema_1_4()
        skill = self.plan["skills"][0]
        skill["required_procedure_roles"].append(
            {
                "role": "redact-customer-telephone-identifiers",
                "requirements": [
                    "Remove every subscriber telephone identifier before writing"
                ],
                "evidence_ids": [skill["evidence_ids"][0]],
                "procedure_step_ids": [skill["procedure_steps"][0]["id"]],
            }
        )
        self.rewrite()
        self.assertIn("SKILL_ROLE_NOT_RENDERED", self.codes())


class RuntimeExpectationTest(SkillQualityFixture):
    """A runtime command has no unmodified target to be observed on.

    The runtime the memory quartet verifies itself with is installed by the
    generation, so on a first run there is nothing to record a baseline
    against - ADR-002's model has no subject. The runtime is fixed and shipped
    by this generator, though, so its behaviour belongs to the contract, and
    the contract states what each exit code means.
    """

    def runtime_check(self, command: str, expected: str, baseline=None) -> list:
        self.use_schema_1_4()
        skill = self.plan["skills"][0]
        skill["kind"] = "runtime-fixed"
        skill["evidence_ids"] = []
        skill["source_paths"] = []
        skill["claim_ids"] = []
        skill["evidence_anchors"] = []
        check = skill["verification"][0]
        check["mode"] = "command"
        check["command"] = command
        check["expected_result"] = expected
        check["baseline"] = baseline
        self.rewrite()
        return [item.code for item in self.plan_diagnostics()]

    def test_promising_zero_against_a_declared_nonzero_exit_is_rejected(self) -> None:
        """The defect this closes, measured on a real run: a quartet skill
        promised `context.py validate` exits zero, and the contract declares
        exit 1 whenever an index is stale."""
        codes = self.runtime_check(
            "python3 memory-bank/scripts/context.py validate",
            "Validation exits zero and reports no schema or index error",
        )
        self.assertIn("RUNTIME_EXPECTATION_CONTRADICTED", codes)

    def test_an_expectation_that_admits_the_declared_failure_passes(self) -> None:
        codes = self.runtime_check(
            "python3 memory-bank/scripts/context.py validate",
            "Validation either passes or names each stale record, and the named "
            "records are reported rather than treated as a skill failure",
        )
        self.assertNotIn("RUNTIME_EXPECTATION_CONTRADICTED", codes)

    def test_a_runtime_command_needs_no_baseline(self) -> None:
        """On a first generation the runtime does not exist on the target yet."""
        codes = self.runtime_check(
            "python3 memory-bank/scripts/context.py status",
            "The counts for each layer are printed and recorded",
        )
        self.assertNotIn("VERIFICATION_BASELINE_MISSING", codes)

    def test_a_recorded_baseline_must_agree_with_the_contract(self) -> None:
        """An update runs against a target that already has the runtime, so a
        baseline is legitimate there - but not one the contract contradicts."""
        codes = self.runtime_check(
            "python3 memory-bank/scripts/context.py status",
            "The counts for each layer are printed and recorded",
            baseline={
                "command": "python3 memory-bank/scripts/context.py status",
                "observed": "exit 1; the runtime refused to report",
                "outcome": "failing",
            },
        )
        self.assertIn("RUNTIME_BASELINE_CONTRADICTED", codes)

    def test_a_baseline_the_contract_allows_is_accepted(self) -> None:
        codes = self.runtime_check(
            "python3 memory-bank/scripts/context.py validate",
            "Validation either passes or names each stale record",
            baseline={
                "command": "python3 memory-bank/scripts/context.py validate",
                "observed": "exit 1; named the stale project-brain index",
                "outcome": "failing",
            },
        )
        self.assertNotIn("RUNTIME_BASELINE_CONTRADICTED", codes)

    def test_every_attested_runtime_command_declares_its_outcomes(self) -> None:
        """The contract cannot be the authority for a command it says nothing
        about, so the two lists may not drift apart."""
        contract = json.loads(
            (ROOT / ".agents/skills/memory-seed/assets/runtime-contract.json")
            .read_text(encoding="utf-8")
        )
        declared = set(contract["commands"]["outcomes"])
        for command in contract["commands"]["read_health"]:
            self.assertIn(command, declared, command)


if __name__ == "__main__":
    unittest.main()
