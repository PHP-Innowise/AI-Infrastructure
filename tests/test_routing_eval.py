#!/usr/bin/env python3
"""Regression tests for scripts/routing_eval.py.

The script invokes a model, so what is testable offline is the part that
decides what a run *meant*: the three-way label, the refusal to record a
baseline from a dry run, and the shape of the fixture the roster is scored
against. `--dry-run` scores perfectly by construction — it feeds the expected
answer back — so it proves the plumbing and nothing about routing, and these
tests exist so that the scoring is not taken on the same trust.

Run: python3 -m unittest tests.test_routing_eval
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import routing_eval  # noqa: E402


class LabelTests(unittest.TestCase):
    def test_the_expected_skill_is_a_hit(self) -> None:
        self.assertEqual("hit", routing_eval.label("coder", "coder"))

    def test_nothing_selected_is_a_miss(self) -> None:
        # A miss means the description is too narrow.
        self.assertEqual("miss", routing_eval.label("coder", None))

    def test_another_skill_is_wrong_not_a_miss(self) -> None:
        # A wrong means the description overlaps a neighbour. The two are
        # repaired in opposite directions, which is why they are separate
        # labels rather than one "fail".
        self.assertEqual("wrong", routing_eval.label("coder", "refactorer"))

    def test_a_restraint_case_passes_only_when_nothing_triggers(self) -> None:
        self.assertEqual("hit", routing_eval.label(None, None))
        self.assertEqual("wrong", routing_eval.label(None, "coder"))


class FixtureTests(unittest.TestCase):
    def test_every_edition_ships_routing_cases(self) -> None:
        for edition in routing_eval.EDITIONS:
            with self.subTest(edition=edition):
                fixture = routing_eval.load_cases(edition)
                self.assertGreaterEqual(len(fixture["cases"]), 10, edition)

    def test_every_expected_skill_exists_in_that_edition(self) -> None:
        # A fixture naming a skill the edition does not ship would score a
        # permanent miss and look like a routing defect.
        for edition in routing_eval.EDITIONS:
            present = {
                path.name
                for path in (ROOT / edition / ".agents" / "skills").iterdir()
                if path.is_dir()
            }
            for case in routing_eval.load_cases(edition)["cases"]:
                if case["expect"] is not None:
                    with self.subTest(edition=edition, expect=case["expect"]):
                        self.assertIn(case["expect"], present)

    def test_every_edition_carries_restraint_cases(self) -> None:
        # A roster eval without a request that should trigger nothing measures
        # only eagerness.
        for edition in routing_eval.EDITIONS:
            with self.subTest(edition=edition):
                cases = routing_eval.load_cases(edition)["cases"]
                self.assertGreaterEqual(
                    sum(1 for case in cases if case["expect"] is None), 2
                )

    def test_each_case_records_why_it_is_a_boundary(self) -> None:
        for edition in routing_eval.EDITIONS:
            for case in routing_eval.load_cases(edition)["cases"]:
                with self.subTest(edition=edition, request=case["request"][:30]):
                    self.assertTrue(case["rationale"].strip())


class BaselineTests(unittest.TestCase):
    def test_a_baseline_is_tied_to_the_policy_digest(self) -> None:
        # A stale baseline has to be mechanically visible; the digest is what
        # makes it so.
        for edition in routing_eval.EDITIONS:
            with self.subTest(edition=edition):
                self.assertRegex(routing_eval.policy_digest(edition), r"^[0-9a-f]{64}$")

    def test_a_dry_run_may_not_be_recorded_as_a_baseline(self) -> None:
        # It scores perfectly by construction; recording it would publish a
        # pass rate of 1.0 that means nothing.
        self.assertEqual(
            2, routing_eval.main(["--dry-run", "--write-baseline", "--edition", "Symfony"])
        )

    def test_a_dry_run_scores_and_reports_without_a_model(self) -> None:
        result = routing_eval.evaluate(
            "Symfony", runs=2, model=None, timeout=1, dry_run=True
        )
        self.assertTrue(result["dry_run"])
        self.assertEqual(0, result["labels"]["miss"])
        self.assertEqual(0, result["labels"]["wrong"])
        self.assertEqual(len(result["per_case"]) * 2, result["labels"]["hit"])


if __name__ == "__main__":
    unittest.main()
