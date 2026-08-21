#!/usr/bin/env python3
"""Regression tests for the adversarial-review record gate.

The deterministic gate cannot ask whether a skill would be picked for a request
nobody wrote down yet, or whether the command it prescribes really does what the
plan says. It can refuse to take the reviewer's word for having looked.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VALIDATOR_PATH = (
    ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_plan_review.py"
)
SPEC = importlib.util.spec_from_file_location("validate_plan_review", VALIDATOR_PATH)
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)


class PlanReviewFixture(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.plan = {
            "skills": [
                {
                    "name": "testing",
                    "verification": [
                        {
                            "id": "run-focused",
                            "command": "vendor/bin/phpunit tests/Feature/PageTest.php",
                        }
                    ],
                }
            ]
        }
        self.review = {
            "plan": "tasks/TASK-001/skill-generation-plan.json",
            "reviewer": "independent",
            "skills": [self.entry("testing")],
            "rejected": [],
            "blockers": [],
        }
        self.registry = None

    def entry(self, name: str) -> dict:
        return {
            "name": name,
            "unseen_positive_request": {
                "verdict": "pass",
                "prompt": "Add a case covering the draft-to-published transition",
                "note": "Selected on the suite it owns, not on the name",
            },
            "sibling_request": {
                "verdict": "pass",
                "prompt": "Find why the nightly publish job stopped at 3am",
                "note": "Deferred to the debugger, as the boundary states",
            },
            "ambiguous_request": {
                "verdict": "pass",
                "prompt": "The publish flow is broken, add something that catches it",
                "note": "Asks which is wanted rather than guessing",
            },
            "cross_domain_request": {
                "verdict": "pass",
                "prompt": "Cover the paid upgrade path end to end",
                "note": "Takes the suite half and hands the billing half over",
            },
            "catalog_role_coverage": {
                "verdict": "pass",
                "note": "Each obligation is a step, not a sentence",
            },
            "refusal_branch": {
                "verdict": "pass",
                "note": "Refuses to weaken an assertion to force a pass",
            },
            "verification_realism": {
                "verdict": "pass",
                "note": "Ran on the unmodified target and matched the baseline",
                "executed": ["vendor/bin/phpunit tests/Feature/PageTest.php"],
            },
            "identity_erasure": {
                "verdict": "pass",
                "note": "Still recognisable with every noun removed",
            },
        }

    def codes(self) -> list[str]:
        plan_path = self.base / "plan.json"
        review_path = self.base / "review.json"
        plan_path.write_text(json.dumps(self.plan, indent=2), encoding="utf-8")
        review_path.write_text(json.dumps(self.review, indent=2), encoding="utf-8")
        registry_path = None
        if self.registry is not None:
            registry_path = self.base / "registry.json"
            registry_path.write_text(
                json.dumps(self.registry, indent=2), encoding="utf-8"
            )
        return [
            item.code
            for item in validator.validate(plan_path, review_path, registry_path)
            if item.severity == "error"
        ]


class PlanReviewTest(PlanReviewFixture):
    def test_a_complete_review_passes(self) -> None:
        self.assertEqual(self.codes(), [])

    def test_a_selected_skill_nobody_reviewed_is_reported(self) -> None:
        self.plan["skills"].append({"name": "coding", "verification": []})
        self.assertIn("REVIEW_SKILL_MISSING", self.codes())

    def test_a_review_of_a_skill_the_plan_does_not_select_is_reported(self) -> None:
        self.review["skills"].append(self.entry("phantom"))
        self.assertIn("REVIEW_SKILL_UNKNOWN", self.codes())

    def test_a_missing_dimension_is_reported(self) -> None:
        del self.review["skills"][0]["identity_erasure"]
        self.assertIn("REVIEW_DIMENSION_MISSING", self.codes())

    def test_a_failed_verdict_blocks(self) -> None:
        self.review["skills"][0]["identity_erasure"] = {
            "verdict": "fail",
            "note": "Reads as any PHP repository once the nouns are removed",
        }
        self.assertIn("REVIEW_VERDICT_FAILED", self.codes())

    def test_an_open_blocker_blocks(self) -> None:
        self.review["blockers"] = ["Two skills claim the same write path"]
        self.assertIn("REVIEW_BLOCKER_OPEN", self.codes())

    def test_a_prompt_naming_its_own_answer_is_reported(self) -> None:
        self.review["skills"][0]["unseen_positive_request"]["prompt"] = (
            "Use testing to add a testing case"
        )
        self.assertIn("REVIEW_PROMPT_TAUTOLOGICAL", self.codes())

    def test_a_verification_judged_from_the_page_is_reported(self) -> None:
        """The defect this dimension exists for: in the third run only the
        judge that ran the command found the broken check."""
        self.review["skills"][0]["verification_realism"]["executed"] = []
        self.assertIn("REVIEW_VERIFICATION_NOT_EXECUTED", self.codes())

    def test_a_review_against_commands_the_plan_never_prescribed_is_reported(
        self,
    ) -> None:
        self.review["skills"][0]["verification_realism"]["executed"].append(
            "vendor/bin/phpunit"
        )
        self.assertIn("REVIEW_VERIFICATION_UNPLANNED", self.codes())

    def test_a_review_by_the_author_is_refused(self) -> None:
        self.review["reviewer"] = "author"
        self.assertIn("REVIEW_NOT_INDEPENDENT", self.codes())

    def test_a_record_without_the_rejected_section_is_refused(self) -> None:
        del self.review["rejected"]
        self.assertIn("REVIEW_INVALID", self.codes())

    def test_a_missing_review_fails_closed(self) -> None:
        plan_path = self.base / "plan.json"
        plan_path.write_text(json.dumps(self.plan), encoding="utf-8")
        codes = [
            item.code
            for item in validator.validate(plan_path, self.base / "absent.json")
            if item.severity == "error"
        ]
        self.assertEqual(codes, ["REVIEW_UNREADABLE"])

    def test_the_json_report_is_byte_stable_across_runs(self) -> None:
        import contextlib
        import io

        plan_path = self.base / "plan.json"
        review_path = self.base / "review.json"
        plan_path.write_text(json.dumps(self.plan), encoding="utf-8")
        review_path.write_text(json.dumps(self.review), encoding="utf-8")
        outputs = []
        for _ in range(2):
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                validator.main(
                    ["--plan", str(plan_path), "--review", str(review_path), "--json"]
                )
            outputs.append(buffer.getvalue())
        self.assertEqual(outputs[0], outputs[1])


class RejectionReviewFixture(PlanReviewFixture):
    """A plan that rejects one risk-flagged and one ordinary candidate."""

    def setUp(self) -> None:
        super().setUp()
        self.plan["rejected_candidates"] = [
            {
                "candidate_id": "migration-safety",
                "name": "migration-safety",
                "category": "specialty",
                "reason": "Creation authority is unresolved",
                "missing_evidence": ["migration ownership decision"],
            },
            {
                "candidate_id": "brainstorming",
                "name": "brainstorming",
                "category": "process",
                "reason": "No ideation procedure was evidenced",
                "missing_evidence": ["a documented discovery workflow"],
            },
        ]
        self.registry = {
            "schema_version": "1.0",
            "catalog_version": "test",
            "candidates": [
                {
                    "id": "migration-safety",
                    "catalog": "php-specialty-skills.md#migration-safety",
                    "category": "specialty",
                    "mode": "static",
                    "escalates_on_rejection": True,
                },
                {
                    "id": "brainstorming",
                    "catalog": "php-process-skills.md#brainstorming",
                    "category": "process",
                    "mode": "static",
                },
            ],
        }
        self.review["rejected"] = [
            {
                "name": "migration-safety",
                "classification": "insufficient-evidence",
                "note": "Migrations exist but no owner or allowed paths",
                "narrow_scope": "Even a read-only reviewer needs the authority "
                "decision to know which paths are in scope",
                "interview_reference": "clarifying-interview-answers.md:L40-L47",
            }
        ]


class RejectionReviewTest(RejectionReviewFixture):
    def test_a_reviewed_flagged_rejection_passes(self) -> None:
        self.assertEqual(self.codes(), [])

    def test_a_flagged_rejection_nobody_reviewed_is_reported(self) -> None:
        self.review["rejected"] = []
        self.assertIn("REJECTION_REVIEW_MISSING", self.codes())

    def test_an_unflagged_rejection_needs_no_review(self) -> None:
        # brainstorming is rejected but not flagged; its absence is fine.
        self.assertNotIn("REJECTION_REVIEW_MISSING", self.codes())

    def test_a_review_of_a_candidate_the_plan_keeps_is_reported(self) -> None:
        self.review["rejected"].append(
            {
                "name": "testing",
                "classification": "not-applicable",
                "note": "phantom entry",
            }
        )
        self.assertIn("REJECTION_REVIEW_UNKNOWN", self.codes())

    def test_a_duplicate_rejection_entry_is_reported(self) -> None:
        self.review["rejected"].append(dict(self.review["rejected"][0]))
        self.assertIn("REJECTION_REVIEW_DUPLICATE", self.codes())

    def test_an_unknown_classification_is_refused(self) -> None:
        self.review["rejected"][0] = {
            "name": "migration-safety",
            "classification": "seems-fine",
            "note": "hand-wave",
        }
        self.assertIn("REJECTION_ANSWER_INVALID", self.codes())

    def test_a_consolidation_into_an_unselected_skill_is_reported(self) -> None:
        self.review["rejected"][0] = {
            "name": "migration-safety",
            "classification": "consolidated",
            "note": "Handled elsewhere",
            "absorbed_by": ["database-designer"],
        }
        self.assertIn("REJECTION_CONSOLIDATION_PHANTOM", self.codes())

    def test_a_consolidation_into_a_selected_skill_passes(self) -> None:
        self.review["rejected"][0] = {
            "name": "migration-safety",
            "classification": "consolidated",
            "note": "The testing skill owns migration regression checks",
            "absorbed_by": ["testing"],
        }
        self.assertEqual(self.codes(), [])

    def test_an_unescalated_evidence_gap_is_reported(self) -> None:
        self.review["rejected"][0]["interview_reference"] = ""
        self.assertIn("REJECTION_NOT_ESCALATED", self.codes())

    def test_a_safety_gap_without_a_human_decision_is_reported(self) -> None:
        self.review["rejected"][0] = {
            "name": "migration-safety",
            "classification": "unresolved-safety",
            "note": "Two deploy paths contradict",
            "narrow_scope": "No safe verification baseline either way",
            "decision_reference": "",
        }
        self.assertIn("REJECTION_SAFETY_UNRESOLVED", self.codes())

    def test_a_safety_gap_with_a_recorded_decision_passes(self) -> None:
        self.review["rejected"][0] = {
            "name": "migration-safety",
            "classification": "unresolved-safety",
            "note": "Two deploy paths contradict",
            "narrow_scope": "No safe verification baseline either way",
            "decision_reference": "clarifying-interview-answers.md:L52",
        }
        self.assertEqual(self.codes(), [])

    def test_without_a_registry_no_rejection_coverage_is_required(self) -> None:
        self.registry = None
        self.review["rejected"] = []
        self.assertEqual(self.codes(), [])


if __name__ == "__main__":
    unittest.main()
