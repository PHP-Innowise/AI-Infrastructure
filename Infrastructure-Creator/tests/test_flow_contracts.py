#!/usr/bin/env python3
"""Deterministic tests for schema 1.2 generated flow contracts."""

from __future__ import annotations

import contextlib
import copy
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(SCRIPTS))

import validate_flow_contracts as validator  # noqa: E402
from validate_flow_contracts import main, validate  # noqa: E402


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


class RoutingContractRobustnessTests(FlowContractFixture):
    """Malformed-but-plausible plan JSON must report errors, never raise."""

    def test_nearest_siblings_none_reports_contract_error(self) -> None:
        self.plan["skills"][0]["nearest_siblings"] = None
        self.write_artifacts()
        self.assertTrue(
            any(
                "alpha.nearest_siblings must be a list" in error
                for error in self.errors()
            )
        )

    def test_nearest_siblings_string_reports_contract_error(self) -> None:
        self.plan["skills"][0]["nearest_siblings"] = "oops"
        self.write_artifacts()
        self.assertTrue(
            any(
                "alpha.nearest_siblings must be a list" in error
                for error in self.errors()
            )
        )

    def test_expected_primary_list_reports_contract_error(self) -> None:
        self.plan["skills"][0]["routing_cases"][0]["expected_primary"] = ["alpha"]
        self.write_artifacts()
        self.assertTrue(
            any(
                "alpha.routing_cases[0] routing fields must use string skill names"
                in error
                for error in self.errors()
            )
        )

    def test_permitted_secondary_dict_entries_report_contract_error(self) -> None:
        self.plan["skills"][0]["routing_cases"][1]["permitted_secondary"] = [
            {"name": "beta"}
        ]
        self.write_artifacts()
        self.assertTrue(
            any(
                "alpha.routing_cases[1] routing fields must use string skill names"
                in error
                for error in self.errors()
            )
        )

    def test_forbidden_skills_nested_list_reports_contract_error(self) -> None:
        self.plan["skills"][0]["routing_cases"][0]["forbidden_skills"] = [["beta"]]
        self.write_artifacts()
        self.assertTrue(
            any(
                "alpha.routing_cases[0] routing fields must use string skill names"
                in error
                for error in self.errors()
            )
        )

    def test_stage_agents_null_reports_error_without_crash(self) -> None:
        self.plan["flow_contracts"]["flows"][0]["stages"][2]["agents"] = None
        self.plan_path.write_text(
            json.dumps(self.plan, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        self.assertTrue(
            any(
                "invalid phase, agents, or flags" in error
                for error in self.errors()
            )
        )

    def test_sibling_unhashable_name_reports_contract_error(self) -> None:
        self.plan["skills"][0]["nearest_siblings"] = [{"name": ["beta"]}]
        self.write_artifacts()
        self.assertTrue(
            any(
                "alpha has invalid adjacent skill ['beta']" in error
                for error in self.errors()
            )
        )

    def test_skill_unhashable_name_reports_contract_error(self) -> None:
        self.plan["skills"][0]["name"] = ["alpha"]
        self.write_artifacts()
        errors = self.errors()
        self.assertTrue(
            any("has invalid adjacent skill 'alpha'" in error for error in errors)
        )
        self.assertTrue(
            any("roster must match plan skill order" in error for error in errors)
        )

    def test_stage_phase_list_reports_contract_error(self) -> None:
        feature = self.plan["flow_contracts"]["flows"][0]
        feature["stages"][0]["phase"] = ["understanding"]
        self.write_artifacts()
        self.assertTrue(
            any(
                "flows[0].stages[0]: invalid phase, agents, or flags" in error
                for error in self.errors()
            )
        )

    def test_all_crash_shapes_exit_nonzero_with_structured_errors(self) -> None:
        skills = self.plan["skills"]
        skills[0]["nearest_siblings"] = None
        skills[1]["routing_cases"][0]["expected_primary"] = ["beta"]
        skills[2]["routing_cases"][0]["permitted_secondary"] = [{"name": "alpha"}]
        skills[3]["routing_cases"][0]["forbidden_skills"] = [["alpha"]]
        self.write_artifacts()
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            exit_code = main(
                [
                    "--plan",
                    str(self.plan_path),
                    "--skill-flow",
                    str(self.skill_flow),
                    "--commands-dir",
                    str(self.commands),
                    "--json",
                ]
            )
        self.assertEqual(exit_code, 1)
        payload = json.loads(stdout.getvalue())
        self.assertFalse(payload["valid"])
        self.assertTrue(
            any(
                "alpha.nearest_siblings must be a list" in error
                for error in payload["errors"]
            )
        )
        self.assertTrue(
            any(
                "routing fields must use string skill names" in error
                for error in payload["errors"]
            )
        )

    def test_unhashable_shapes_exit_nonzero_with_structured_errors(self) -> None:
        self.plan["skills"][0]["nearest_siblings"] = [{"name": ["beta"]}]
        self.plan["skills"][1]["name"] = {"nested": "beta"}
        self.plan["flow_contracts"]["flows"][0]["stages"][0]["phase"] = [
            "understanding"
        ]
        self.write_artifacts()
        argv = [
            "--plan",
            str(self.plan_path),
            "--skill-flow",
            str(self.skill_flow),
            "--commands-dir",
            str(self.commands),
            "--json",
        ]
        outputs = []
        for _run in range(2):
            stdout = io.StringIO()
            with contextlib.redirect_stdout(stdout):
                exit_code = main(argv)
            self.assertEqual(exit_code, 1)
            outputs.append(stdout.getvalue())
        self.assertEqual(outputs[0], outputs[1])
        payload = json.loads(outputs[0])
        self.assertFalse(payload["valid"])
        self.assertTrue(
            any(
                "alpha has invalid adjacent skill ['beta']" in error
                for error in payload["errors"]
            )
        )
        self.assertTrue(
            any(
                "invalid phase, agents, or flags" in error
                for error in payload["errors"]
            )
        )


class CommandFileSetTests(FlowContractFixture):
    """Expected/actual command files derive from the declared flow names."""

    def hotfix_flow(self) -> dict:
        return {
            "name": "hotfix",
            "required_code_review": None,
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
            ],
        }

    def test_declared_flow_without_prefix_validates(self) -> None:
        self.plan["flow_contracts"]["flows"].append(self.hotfix_flow())
        self.write_artifacts()
        self.assertEqual(self.errors(), [])

    def test_missing_declared_unprefixed_flow_file_is_flagged(self) -> None:
        self.plan["flow_contracts"]["flows"].append(self.hotfix_flow())
        self.write_artifacts()
        (self.commands / "hotfix.md").unlink()
        self.assertTrue(
            any(
                "commands: expected" in error and "hotfix.md" in error
                for error in self.errors()
            )
        )

    def test_undeclared_flow_prefixed_stray_is_still_flagged(self) -> None:
        (self.commands / "flow-extra.md").write_text("# stray\n", encoding="utf-8")
        self.assertTrue(
            any(
                "commands: expected" in error and "flow-extra.md" in error
                for error in self.errors()
            )
        )




class FlowFrontmatterEncodingTest(unittest.TestCase):
    """Both gates must read the same frontmatter, or no flow can be generated.

    `validate_generated.py` reads stages as inline mappings - the form every
    shipped edition writes - while this module used to parse only the block
    form. Measured on the shipped reference command, this parser saw 0 stages
    where the other saw 8, so a flow command could satisfy one gate or the
    other and never both, and generation could not complete on any target.
    """

    INLINE = """---
flow: feature
stages:
  - { phase: understanding, agents: [requirements-analyst] }
  - { phase: verification, agents: [code-reviewer, security-reviewer], parallel: true }
  - { phase: finalization, agents: [finishing-branch], checkpoint: integration }
---

# Flow: Feature
"""
    BLOCK = """---
flow: flow-review
stages:
  - phase: verification
    agents: [security-review-agent]
    parallel: false
    checkpoint: true
---

# flow-review
"""

    def write(self, text: str) -> Path:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        path = Path(temporary.name) / "flow.md"
        path.write_text(text, encoding="utf-8")
        return path

    def test_the_shipped_inline_form_parses(self) -> None:
        flow, stages = validator._frontmatter_stages(self.write(self.INLINE))
        self.assertEqual(flow, "feature")
        self.assertEqual(len(stages), 3)
        self.assertEqual(stages[0], {"phase": "understanding",
                                     "agents": ["requirements-analyst"]})
        self.assertTrue(stages[1]["parallel"])
        self.assertEqual(stages[1]["agents"], ["code-reviewer", "security-reviewer"])
        # A named checkpoint is still a checkpoint: the stage stops.
        self.assertTrue(stages[2]["checkpoint"])

    def test_the_block_form_still_parses(self) -> None:
        flow, stages = validator._frontmatter_stages(self.write(self.BLOCK))
        self.assertEqual(flow, "flow-review")
        self.assertEqual(len(stages), 1)
        self.assertFalse(stages[0]["parallel"])

    def test_both_gates_count_the_same_stages(self) -> None:
        """The property that was broken: agreement, not either parser alone."""
        generated = importlib.util.spec_from_file_location(
            "validate_generated_for_flow",
            ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_generated.py",
        )
        module = importlib.util.module_from_spec(generated)
        sys.modules[generated.name] = module
        generated.loader.exec_module(module)
        for text in (self.INLINE, self.BLOCK):
            path = self.write(text)
            _, mine = validator._frontmatter_stages(path)
            body = path.read_text(encoding="utf-8")
            end = body.find("\n---", 3)
            theirs = sum(
                1
                for line in body[3:end].splitlines()
                if module.STAGE_RE.match(line)
            )
            if theirs:
                self.assertEqual(len(mine), theirs, text[:40])
            else:
                self.assertTrue(mine, "block form must still parse somewhere")


if __name__ == "__main__":
    unittest.main()
