#!/usr/bin/env python3
"""Regression tests for the optional coverage baseline (--baseline-plan).

Two generations of the same target may legitimately re-compose the inventory,
so lost coverage is reported as warnings; the contract under test is that the
lost skills and lost evidence paths are always NAMED, never hidden behind a
count, and that a missing or malformed baseline is a diagnostic, not a
traceback.
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
    ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_skill_quality.py"
)
SPEC = importlib.util.spec_from_file_location(
    "validate_skill_quality_baseline", VALIDATOR_PATH
)
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)

REGISTRY = ROOT / ".agents/skills/skill-forge/references/candidate-registry.json"


def _plan(skills: list[dict], evidence: list[dict]) -> dict:
    return {
        "schema_version": "1.2",
        "catalog_version": "0",
        "target_root": ".",
        "profile": {},
        "evidence": evidence,
        "skills": skills,
        "rejected_candidates": [],
        "critical_invariants": [],
        "flow_contracts": {},
    }


BASELINE_PLAN = _plan(
    skills=[
        {
            "name": "security-review",
            "source_paths": [
                "config/packages/security.yaml",
                "templates/article/show.html.twig",
            ],
        },
        {"name": "testing", "source_paths": ["phpunit.xml.dist"]},
    ],
    evidence=[
        {"id": "EV-1", "path": "src/Controller/PageController.php"},
        {"id": "EV-2", "path": "phpunit.xml.dist"},
    ],
)
CURRENT_PLAN = _plan(
    skills=[{"name": "testing", "source_paths": ["phpunit.xml.dist"]}],
    evidence=[{"id": "EV-2", "path": "phpunit.xml.dist"}],
)


class CoverageBaselineTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def _write(self, name: str, payload) -> Path:
        path = self.root / name
        if isinstance(payload, str):
            path.write_text(payload, encoding="utf-8")
        else:
            path.write_text(json.dumps(payload), encoding="utf-8")
        return path

    def test_lost_skill_and_paths_are_named(self) -> None:
        baseline = self._write("baseline.json", BASELINE_PLAN)
        current = self._write("plan.json", CURRENT_PLAN)
        diagnostics, report = validator.compare_coverage_baseline(current, baseline)
        self.assertTrue(report["compared"])
        self.assertEqual(report["lost_skills"], ["security-review"])
        self.assertEqual(
            report["lost_source_paths"],
            [
                "config/packages/security.yaml",
                "src/Controller/PageController.php",
                "templates/article/show.html.twig",
            ],
        )
        dropped = [
            item for item in diagnostics if item.code == "BASELINE_SKILL_DROPPED"
        ]
        self.assertEqual(len(dropped), 1)
        self.assertIn("security-review", dropped[0].message)
        # The dropped skill's own cited paths travel with it, so a reader can
        # see what stopped being read without opening the baseline plan.
        self.assertIn("templates/article/show.html.twig", dropped[0].message)
        named_paths = {
            item.message.split(":", 1)[0]
            for item in diagnostics
            if item.code == "BASELINE_COVERAGE_DROPPED"
        }
        self.assertEqual(named_paths, set(report["lost_source_paths"]))

    def test_lost_coverage_is_warning_not_error(self) -> None:
        baseline = self._write("baseline.json", BASELINE_PLAN)
        current = self._write("plan.json", CURRENT_PLAN)
        diagnostics, _ = validator.compare_coverage_baseline(current, baseline)
        self.assertTrue(diagnostics)
        self.assertEqual({item.severity for item in diagnostics}, {"warning"})

    def test_identical_plan_loses_nothing(self) -> None:
        baseline = self._write("baseline.json", BASELINE_PLAN)
        current = self._write("plan.json", BASELINE_PLAN)
        diagnostics, report = validator.compare_coverage_baseline(current, baseline)
        self.assertEqual(diagnostics, [])
        self.assertTrue(report["compared"])
        self.assertEqual(report["lost_skills"], [])
        self.assertEqual(report["lost_source_paths"], [])
        self.assertEqual(
            validator._coverage_baseline_summary(report),
            "coverage baseline: no coverage lost",
        )

    def test_added_coverage_is_not_reported_as_loss(self) -> None:
        baseline = self._write("baseline.json", CURRENT_PLAN)
        current = self._write("plan.json", BASELINE_PLAN)
        diagnostics, report = validator.compare_coverage_baseline(current, baseline)
        self.assertEqual(diagnostics, [])
        self.assertEqual(report["lost_skills"], [])

    def test_coverage_unions_evidence_paths_and_source_paths(self) -> None:
        names, paths = validator._plan_coverage(BASELINE_PLAN)
        self.assertEqual(names, ["security-review", "testing"])
        self.assertIn("src/Controller/PageController.php", paths)  # evidence only
        self.assertIn("config/packages/security.yaml", paths)  # source_paths only

    def test_malformed_baseline_is_a_diagnostic(self) -> None:
        baseline = self._write("baseline.json", "{not json")
        current = self._write("plan.json", CURRENT_PLAN)
        diagnostics, report = validator.compare_coverage_baseline(current, baseline)
        self.assertEqual([item.code for item in diagnostics], ["BASELINE_PLAN_UNREADABLE"])
        self.assertEqual(diagnostics[0].severity, "warning")
        self.assertFalse(report["compared"])
        self.assertEqual(report["lost_skills"], [])
        summary = validator._coverage_baseline_summary(report)
        self.assertIn("NOT COMPARED", summary)
        self.assertNotIn("no coverage lost", summary)

    def test_missing_baseline_file_is_a_diagnostic(self) -> None:
        current = self._write("plan.json", CURRENT_PLAN)
        diagnostics, report = validator.compare_coverage_baseline(
            current, self.root / "absent.json"
        )
        self.assertEqual([item.code for item in diagnostics], ["BASELINE_PLAN_UNREADABLE"])
        self.assertFalse(report["compared"])

    def test_baseline_root_of_wrong_type_is_a_diagnostic(self) -> None:
        baseline = self._write("baseline.json", ["not", "an", "object"])
        current = self._write("plan.json", CURRENT_PLAN)
        diagnostics, report = validator.compare_coverage_baseline(current, baseline)
        self.assertEqual([item.code for item in diagnostics], ["BASELINE_PLAN_UNREADABLE"])
        self.assertFalse(report["compared"])

    def test_unreadable_current_plan_is_reported_as_not_compared(self) -> None:
        baseline = self._write("baseline.json", BASELINE_PLAN)
        current = self._write("plan.json", "}broken")
        diagnostics, report = validator.compare_coverage_baseline(current, baseline)
        self.assertEqual([item.code for item in diagnostics], ["BASELINE_COMPARE_SKIPPED"])
        self.assertFalse(report["compared"])

    def test_malformed_plan_fields_do_not_raise(self) -> None:
        baseline = self._write(
            "baseline.json",
            {"skills": "nope", "evidence": [None, {"path": 7}, {"path": "kept.yaml"}]},
        )
        current = self._write("plan.json", {"skills": [None, {"name": 3}]})
        diagnostics, report = validator.compare_coverage_baseline(current, baseline)
        self.assertTrue(report["compared"])
        self.assertEqual(report["lost_source_paths"], ["kept.yaml"])
        self.assertEqual({item.severity for item in diagnostics}, {"warning"})

    def test_comparison_is_deterministic(self) -> None:
        shuffled = _plan(
            skills=list(reversed(BASELINE_PLAN["skills"])),
            evidence=list(reversed(BASELINE_PLAN["evidence"])),
        )
        baseline_a = self._write("a.json", BASELINE_PLAN)
        baseline_b = self._write("b.json", shuffled)
        current = self._write("plan.json", CURRENT_PLAN)
        first, report_a = validator.compare_coverage_baseline(current, baseline_a)
        second, report_b = validator.compare_coverage_baseline(current, baseline_b)
        self.assertEqual([item.as_dict() for item in first], [item.as_dict() for item in second])
        self.assertEqual(report_a, report_b)


class CoverageBaselineCliTest(unittest.TestCase):
    """The flag has to be inert when absent and loud when present."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)
        self.baseline = self.root / "baseline.json"
        self.baseline.write_text(json.dumps(BASELINE_PLAN), encoding="utf-8")
        self.plan = self.root / "plan.json"
        self.plan.write_text(json.dumps(CURRENT_PLAN), encoding="utf-8")

    def _run(self, *extra: str) -> str:
        stream = io.StringIO()
        argv = [
            "--plan",
            str(self.plan),
            "--target",
            str(self.root),
            "--registry",
            str(REGISTRY),
            "--plan-only",
        ]
        with contextlib.redirect_stdout(stream), contextlib.redirect_stderr(io.StringIO()):
            validator.main(argv + list(extra))
        return stream.getvalue()

    def test_absent_flag_changes_nothing(self) -> None:
        output = self._run()
        self.assertNotIn("coverage baseline", output)
        self.assertNotIn("BASELINE_", output)
        payload = json.loads(self._run("--json"))
        self.assertNotIn("coverage_baseline", payload)

    def test_summary_names_the_lost_skill_and_paths(self) -> None:
        output = self._run("--baseline-plan", str(self.baseline))
        summary = [
            line for line in output.splitlines() if line.startswith("coverage baseline:")
        ]
        self.assertEqual(len(summary), 1)
        self.assertIn("security-review", summary[0])
        self.assertIn("templates/article/show.html.twig", summary[0])
        self.assertIn("BASELINE_SKILL_DROPPED", output)

    def test_json_payload_carries_named_losses(self) -> None:
        payload = json.loads(
            self._run("--json", "--baseline-plan", str(self.baseline))
        )
        self.assertEqual(payload["coverage_baseline"]["lost_skills"], ["security-review"])
        self.assertIn(
            "templates/article/show.html.twig",
            payload["coverage_baseline"]["lost_source_paths"],
        )
        codes = {item["code"] for item in payload["diagnostics"]}
        self.assertIn("BASELINE_SKILL_DROPPED", codes)

    def test_lost_coverage_does_not_flip_the_exit_code(self) -> None:
        stream = io.StringIO()
        argv = [
            "--plan",
            str(self.plan),
            "--target",
            str(self.root),
            "--registry",
            str(REGISTRY),
            "--plan-only",
        ]
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(stream):
            without = validator.main(argv)
            with_baseline = validator.main(argv + ["--baseline-plan", str(self.baseline)])
        self.assertEqual(without, with_baseline)

    def test_broken_baseline_does_not_traceback_through_the_cli(self) -> None:
        broken = self.root / "broken.json"
        broken.write_text("{", encoding="utf-8")
        output = self._run("--baseline-plan", str(broken))
        self.assertIn("coverage baseline: NOT COMPARED", output)



class BaselineDimensionTest(unittest.TestCase):
    """A regeneration can keep every skill and every file and still lose the
    invariant that made one of them worth generating, or the ownership that kept
    two of them from colliding. The old comparison called that "no coverage
    lost".

    Identifiers are deliberately not compared: across three consecutive real
    regenerations of one target, comparing invariant, ownership and verification
    ids reported 5-12, 6-8 and 34-42 losses, nearly all of them the same thing
    renamed.
    """

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def rich(self, *, invariants, owned, module) -> dict:
        plan = _plan(
            skills=[
                {
                    "name": "testing",
                    "source_paths": [f"{module}/Kernel.php"],
                    "ownership": [
                        {"id": "testing.suite", "paths": list(owned)}
                    ],
                }
            ],
            evidence=[{"id": "EV-2", "path": f"{module}/Kernel.php"}],
        )
        plan["critical_invariants"] = [
            {"id": f"inv-{index}", "statement": statement}
            for index, statement in enumerate(invariants)
        ]
        return plan

    def compare(self, baseline: dict, current: dict):
        base = self.root / "baseline.json"
        cur = self.root / "plan.json"
        base.write_text(json.dumps(baseline), encoding="utf-8")
        cur.write_text(json.dumps(current), encoding="utf-8")
        return validator.compare_coverage_baseline(cur, base)

    def test_each_lost_dimension_is_named_individually(self) -> None:
        baseline = self.rich(
            invariants=[
                "A published article never returns to draft",
                "Every audit entry is append-only once written",
            ],
            owned=["tests/**", "fixtures/**"],
            module="src",
        )
        current = self.rich(
            invariants=["A published article never returns to draft"],
            owned=["tests/**"],
            module="lib",
        )
        diagnostics, report = self.compare(baseline, current)
        self.assertEqual(
            report["lost_invariants"],
            ["Every audit entry is append-only once written"],
        )
        self.assertEqual(report["lost_owned_paths"], ["fixtures/**"])
        self.assertEqual(report["lost_modules"], ["src"])
        for code, member in (
            ("BASELINE_INVARIANT_DROPPED", "append-only"),
            ("BASELINE_OWNERSHIP_DROPPED", "fixtures/**"),
            ("BASELINE_MODULE_DROPPED", "src"),
        ):
            named = [item for item in diagnostics if item.code == code]
            self.assertEqual(len(named), 1, code)
            self.assertIn(member, named[0].message)
            self.assertEqual(named[0].severity, "warning")

    def test_a_restated_invariant_is_not_reported_as_lost(self) -> None:
        """Runs reword freely; comparing the words would cry at every run."""
        baseline = self.rich(
            invariants=["A published article never returns to draft"],
            owned=["tests/**"],
            module="src",
        )
        current = self.rich(
            invariants=["Once published, an article cannot go back to draft"],
            owned=["tests/**"],
            module="src",
        )
        _, report = self.compare(baseline, current)
        self.assertEqual(report["lost_invariants"], [])

    def test_a_root_file_is_never_reported_as_a_lost_module(self) -> None:
        baseline = _plan(
            skills=[{"name": "testing", "source_paths": ["composer.json"]}],
            evidence=[{"id": "EV-1", "path": "composer.json"}],
        )
        current = _plan(skills=[{"name": "testing", "source_paths": []}], evidence=[])
        _, report = self.compare(baseline, current)
        self.assertEqual(report["lost_modules"], [])

    def test_the_summary_names_every_lost_member_not_a_count(self) -> None:
        baseline = self.rich(
            invariants=["Every audit entry is append-only once written"],
            owned=["fixtures/**"],
            module="src",
        )
        current = self.rich(invariants=[], owned=[], module="src")
        _, report = self.compare(baseline, current)
        summary = validator._coverage_baseline_summary(report)
        self.assertIn("append-only", summary)
        self.assertIn("fixtures/**", summary)

    def test_an_unchanged_plan_loses_nothing(self) -> None:
        plan = self.rich(
            invariants=["Every audit entry is append-only once written"],
            owned=["fixtures/**"],
            module="src",
        )
        _, report = self.compare(plan, plan)
        self.assertEqual(
            validator._coverage_baseline_summary(report),
            "coverage baseline: no coverage lost",
        )


if __name__ == "__main__":
    unittest.main()
