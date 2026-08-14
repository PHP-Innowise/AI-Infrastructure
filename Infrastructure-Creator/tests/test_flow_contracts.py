#!/usr/bin/env python3
"""Deterministic tests for schema 1.2 generated flow contracts."""

from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(SCRIPTS))

from validate_flow_contracts import validate  # noqa: E402


def contract_block(payload: dict) -> str:
    return (
        "```json flow-contract\n"
        + json.dumps(payload, indent=2, sort_keys=True)
        + "\n```\n"
    )


class FlowContractFixture(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="flow-contract-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.plan_path = self.root / "plan.json"
        self.skill_flow = self.root / "SKILL FLOW.md"
        self.commands = self.root / "commands"
        self.commands.mkdir()
        definitions = [
            ("alpha", "understanding", [], ["beta", "gamma", "code-review"]),
            ("beta", "implementation", ["src/beta/**"], ["alpha"]),
            ("gamma", "implementation", ["src/gamma/**"], ["alpha"]),
            ("code-review", "verification", [], ["alpha"]),
        ]
        skills = [
            {
                "name": name,
                "phase": phase,
                "writes": writes,
                "nearest_siblings": [
                    {
                        "name": sibling,
                        "role": "defer",
                        "boundary": f"Route {sibling} ownership to {sibling}.",
                        "ownership_ids": [f"skill.{sibling}"],
                    }
                    for sibling in siblings
                ],
            }
            for name, phase, writes, siblings in definitions
        ]
        all_names = {skill["name"] for skill in skills}
        for skill in skills:
            adjacent = [
                sibling["name"] for sibling in skill["nearest_siblings"]
            ]
            skill["routing_cases"] = [
                {
                    "prompt": f"Keep {skill['name']} primary for its owned scope.",
                    "expected_primary": skill["name"],
                    "permitted_secondary": [],
                    "forbidden_skills": sorted(all_names - {skill["name"]}),
                    "rationale": "The prompt matches the source skill.",
                    "evidence_ids": ["EV-routing"],
                },
                *[
                    {
                        "prompt": f"Defer this adjacent concern to {sibling}.",
                        "expected_primary": sibling,
                        "permitted_secondary": [],
                        "forbidden_skills": sorted(
                            all_names - {sibling}
                        ),
                        "rationale": "The adjacent owner is explicitly primary.",
                        "evidence_ids": ["EV-routing"],
                    }
                    for sibling in adjacent
                ],
            ]
        roster = [
            {
                "skill": skill["name"],
                "agent": f"{skill['name']}-agent",
                "phase": skill["phase"],
                "writes": bool(skill["writes"]),
            }
            for skill in skills
        ]
        feature = {
            "name": "flow-feature",
            "required_code_review": "code-review-agent",
            "stages": [
                {
                    "phase": "understanding",
                    "agents": ["alpha-agent"],
                    "parallel": False,
                    "checkpoint": True,
                },
                {
                    "phase": "implementation",
                    "agents": ["beta-agent"],
                    "parallel": False,
                    "checkpoint": False,
                },
                {
                    "phase": "verification",
                    "agents": ["code-review-agent"],
                    "parallel": False,
                    "checkpoint": False,
                },
            ],
        }
        review = {
            "name": "flow-review",
            "required_code_review": "code-review-agent",
            "stages": [
                {
                    "phase": "understanding",
                    "agents": ["alpha-agent"],
                    "parallel": False,
                    "checkpoint": True,
                },
                {
                    "phase": "verification",
                    "agents": ["code-review-agent"],
                    "parallel": False,
                    "checkpoint": False,
                },
            ],
        }
        self.plan = {
            "schema_version": "1.2",
            "skills": skills,
            "flow_contracts": {"roster": roster, "flows": [feature, review]},
        }
        self.write_artifacts()

    def write_artifacts(self) -> None:
        self.plan_path.write_text(
            json.dumps(self.plan, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        graph = self.plan["flow_contracts"]
        self.skill_flow.write_text(
            "# Skill Flow\n\n## Canonical Flow Graph\n" + contract_block(graph),
            encoding="utf-8",
        )
        for flow in graph["flows"]:
            stages = []
            for stage in flow["stages"]:
                agents = ", ".join(stage["agents"])
                stages.extend(
                    [
                        f"  - phase: {stage['phase']}",
                        f"    agents: [{agents}]",
                        f"    parallel: {str(stage['parallel']).lower()}",
                        f"    checkpoint: {str(stage['checkpoint']).lower()}",
                    ]
                )
            payload = {"roster": graph["roster"], "flow": flow}
            text = (
                "---\n"
                f"flow: {flow['name']}\n"
                "stages:\n"
                + "\n".join(stages)
                + "\n---\n"
                f"# {flow['name']}\n\n## Compiled Flow Contract\n"
                + contract_block(payload)
            )
            (self.commands / f"{flow['name']}.md").write_text(
                text, encoding="utf-8"
            )

    def errors(self) -> list[str]:
        return validate(self.plan_path, self.skill_flow, self.commands)


class FlowContractTests(FlowContractFixture):
    def test_complete_graph_with_three_siblings_and_routing_oracle_passes(self) -> None:
        self.assertEqual(self.errors(), [])

    def test_routing_oracle_rejects_any_omitted_adjacency(self) -> None:
        alpha = self.plan["skills"][0]
        alpha["routing_cases"] = [
            case
            for case in alpha["routing_cases"]
            if case["expected_primary"] != "gamma"
        ]
        self.write_artifacts()
        self.assertTrue(
            any(
                "alpha omits adjacent skills ['gamma']" in error
                for error in self.errors()
            )
        )

    def test_feature_flow_rejects_omitted_required_code_review(self) -> None:
        feature = self.plan["flow_contracts"]["flows"][0]
        feature["stages"][2]["agents"] = ["alpha-agent"]
        self.write_artifacts()
        self.assertTrue(
            any("required code-review-agent is absent" in error for error in self.errors())
        )

    def test_skill_flow_and_command_order_must_match_canonical_graph(self) -> None:
        path = self.commands / "flow-feature.md"
        text = path.read_text(encoding="utf-8")
        text = text.replace(
            "  - phase: implementation\n"
            "    agents: [beta-agent]\n"
            "    parallel: false\n"
            "    checkpoint: false\n",
            "",
            1,
        )
        path.write_text(text, encoding="utf-8")
        errors = self.errors()
        self.assertTrue(
            any("frontmatter stage order or content differs" in error for error in errors)
        )

        self.write_artifacts()
        skill_graph = copy.deepcopy(self.plan["flow_contracts"])
        skill_graph["flows"].reverse()
        self.skill_flow.write_text(contract_block(skill_graph), encoding="utf-8")
        self.assertTrue(
            any("SKILL FLOW: compiled graph differs" in error for error in self.errors())
        )

    def test_roster_and_writer_serialization_are_enforced(self) -> None:
        graph = self.plan["flow_contracts"]
        graph["roster"][0]["writes"] = True
        graph["flows"][0]["stages"][1] = {
            "phase": "implementation",
            "agents": ["beta-agent", "gamma-agent"],
            "parallel": True,
            "checkpoint": False,
        }
        self.write_artifacts()
        errors = self.errors()
        self.assertTrue(any("roster must match plan skill order" in error for error in errors))
        self.assertTrue(any("write-capable agents are not serialized" in error for error in errors))

    def test_checkpoint_and_phase_order_are_enforced(self) -> None:
        feature = self.plan["flow_contracts"]["flows"][0]
        for stage in feature["stages"]:
            stage["checkpoint"] = False
        feature["stages"][1]["phase"] = "finalization"
        self.write_artifacts()
        errors = self.errors()
        self.assertTrue(any("requires a checkpoint" in error for error in errors))
        self.assertTrue(any("phase order moves backwards" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
