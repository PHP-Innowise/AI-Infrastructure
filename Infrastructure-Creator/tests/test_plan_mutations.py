#!/usr/bin/env python3
"""Regression tests for the pre-publication mutation step.

A plan that passes every gate proves the rules are satisfiable. It says nothing
about whether the rules would have noticed had the plan been worse - and a gate
can decay in ways nothing reports. This step damages the plan one way at a time
and requires the gate to object by name; these cases pin what it does when it
cannot, and when the plan it is handed is not clean to begin with.
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VALIDATOR_PATH = (
    ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_plan_mutations.py"
)
SPEC = importlib.util.spec_from_file_location("validate_plan_mutations", VALIDATOR_PATH)
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)

sys.path.insert(0, str(ROOT / "tests"))
import test_large_plan_calibration as corpus  # noqa: E402


class PlanMutationFixture(unittest.TestCase):
    def build(self, **kwargs):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        return corpus.build_corpus(Path(temporary.name), **kwargs)

    def run_step(self, built) -> list:
        return validator.validate(
            built["plan_path"], built["target"], built["registry_path"]
        )


class PlanMutationTest(PlanMutationFixture):
    def test_an_honest_plan_exercises_the_damages_it_has_shape_for(self) -> None:
        built = self.build()
        diagnostics = self.run_step(built)
        self.assertEqual([item for item in diagnostics if item.severity == "error"], [])
        skipped = [
            item for item in diagnostics if item.code == "MUTATION_NOT_APPLICABLE"
        ]
        # The corpus carries no failing baseline, no runtime-fixed skill, no
        # evidence disposition, and no pre-existing team skill, so four
        # damages have nothing to act on.
        self.assertEqual(len(skipped), 4)

    def test_an_inapplicable_damage_is_reported_rather_than_skipped(self) -> None:
        """A silently skipped check reads exactly like a passing one."""
        built = self.build()
        diagnostics = self.run_step(built)
        for item in diagnostics:
            if item.code == "MUTATION_NOT_APPLICABLE":
                self.assertEqual(item.severity, "warning")
                self.assertIn("unexercised", item.message)

    def test_a_plan_that_is_not_clean_makes_the_check_meaningless(self) -> None:
        built = self.build()
        plan = json.loads(built["plan_path"].read_text(encoding="utf-8"))
        plan["skills"][0]["claim_ids"] = []
        built["plan_path"].write_text(json.dumps(plan, indent=2), encoding="utf-8")
        codes = [item.code for item in self.run_step(built)]
        self.assertEqual(codes, ["MUTATION_CONTROL_DIRTY"])

    def test_an_unreadable_plan_fails_closed(self) -> None:
        built = self.build()
        built["plan_path"].write_text("{ not json", encoding="utf-8")
        codes = [item.code for item in self.run_step(built)]
        self.assertEqual(codes, ["MUTATION_PLAN_UNREADABLE"])

    def test_a_damage_the_gate_accepts_is_reported(self) -> None:
        """The failure this step exists for: a rule that stopped firing."""
        built = self.build()
        original = validator.MUTATIONS
        validator.MUTATIONS = (
            (
                "nothing is actually damaged",
                "CATALOG_ROLE_UNCOVERED",
                lambda plan, gate, roles: plan["skills"][0]["name"],
            ),
        )
        try:
            codes = [item.code for item in self.run_step(built)]
        finally:
            validator.MUTATIONS = original
        self.assertIn("MUTATION_UNCAUGHT", codes)

    def test_a_damage_caught_by_the_wrong_rule_is_reported(self) -> None:
        built = self.build()
        original = validator.MUTATIONS
        validator.MUTATIONS = (
            (
                "a skill stops naming its claims",
                "A_CODE_THAT_NEVER_FIRES",
                validator._drop_claim_references,
            ),
        )
        try:
            diagnostics = self.run_step(built)
        finally:
            validator.MUTATIONS = original
        codes = [item.code for item in diagnostics]
        self.assertIn("MUTATION_CODE_MISSING", codes)
        message = next(
            item.message for item in diagnostics if item.code == "MUTATION_CODE_MISSING"
        )
        self.assertIn("SKILL_CLAIM_IDS_INVALID", message)

    def test_the_plan_on_disk_is_never_damaged(self) -> None:
        built = self.build()
        before = built["plan_path"].read_bytes()
        self.run_step(built)
        self.assertEqual(built["plan_path"].read_bytes(), before)

    def test_the_json_report_is_byte_stable_across_runs(self) -> None:
        built = self.build()
        outputs = []
        for _ in range(2):
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                validator.main(
                    [
                        "--plan",
                        str(built["plan_path"]),
                        "--target",
                        str(built["target"]),
                        "--registry",
                        str(built["registry_path"]),
                        "--json",
                    ]
                )
            outputs.append(buffer.getvalue())
        self.assertEqual(outputs[0], outputs[1])

    def test_every_damage_names_the_rule_that_must_object(self) -> None:
        """The catalogue is the contract; a damage with no expected code would
        pass on any rejection at all."""
        gate_source = (
            ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_skill_quality.py"
        ).read_text(encoding="utf-8")
        for description, expected, _ in validator.MUTATIONS:
            self.assertTrue(description.strip())
            self.assertIn(f'"{expected}"', gate_source, expected)


if __name__ == "__main__":
    unittest.main()
