#!/usr/bin/env python3
"""Regression tests for the content-review record gate.

The deterministic gates cannot read a policy document and ask whether it
would survive against a different repository, or whether a procedure is
complete enough to execute without its author. They can refuse to take the
reviewer's word for having looked.
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
    ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_content_review.py"
)
SPEC = importlib.util.spec_from_file_location(
    "validate_content_review", VALIDATOR_PATH
)
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)


class ContentReviewFixture(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.plan_lines = [
            ".claude/skills/billing-rules-review/SKILL.md",
            "AGENTS.md",
            "memory-bank/scripts/context.py",
        ]
        self.review = {
            "task": "TASK-001",
            "reviewer": "independent",
            "generation_root": "tasks/TASK-001/infra-generate-staging",
            "rounds": 0,
            "blockers": [],
            "files": [
                self.entry(
                    ".claude/skills/billing-rules-review/SKILL.md", "skills"
                ),
                self.entry("AGENTS.md", "policy"),
                {
                    "path": "memory-bank/scripts/context.py",
                    "lane": "runtime-verbatim",
                    "basis": "Stack-agnostic runtime ships byte-for-byte by design",
                },
                self.report_entry(
                    "tasks/TASK-001/infra-scan-project-profile.md"
                ),
                self.report_entry("tasks/TASK-001/skill-generation-plan.json"),
            ],
        }

    def entry(self, path: str, lane: str) -> dict:
        return {
            "path": path,
            "lane": lane,
            "uniqueness": {
                "verdict": "pass",
                "note": "Names the target's own invoice states, not a stack",
            },
            "completeness": {
                "verdict": "pass",
                "note": "Every step carries a path or a runnable command",
            },
            "accuracy": {
                "verdict": "pass",
                "note": "Cited ranges re-checked against the current target",
            },
            "coherence": {
                "verdict": "pass",
                "note": "Wrapper, command, and flow name the same boundary",
            },
            "findings": [],
        }

    def report_entry(self, path: str) -> dict:
        made = self.entry(path, "reports")
        return made

    def codes(self) -> list[str]:
        plan_path = self.base / "publication-plan.txt"
        review_path = self.base / "review.json"
        plan_path.write_text(
            "\n".join(self.plan_lines) + "\n", encoding="utf-8"
        )
        review_path.write_text(json.dumps(self.review, indent=2), encoding="utf-8")
        return [
            item.code
            for item in validator.validate(plan_path, review_path)
            if item.severity == "error"
        ]


class ContentReviewTest(ContentReviewFixture):
    def test_a_complete_review_passes(self) -> None:
        self.assertEqual(self.codes(), [])

    def test_a_published_file_nobody_reviewed_is_reported(self) -> None:
        self.plan_lines.append(".claude/hooks/bash-validator.sh")
        self.assertIn("CONTENT_REVIEW_FILE_MISSING", self.codes())

    def test_a_review_of_a_file_the_plan_does_not_publish_is_reported(self) -> None:
        self.review["files"].append(self.entry(".claude/phantom.md", "policy"))
        self.assertIn("CONTENT_REVIEW_FILE_UNKNOWN", self.codes())

    def test_a_missing_dimension_is_reported(self) -> None:
        del self.review["files"][0]["uniqueness"]
        self.assertIn("CONTENT_REVIEW_DIMENSION_MISSING", self.codes())

    def test_a_failed_verdict_blocks(self) -> None:
        self.review["files"][1]["uniqueness"] = {
            "verdict": "fail",
            "note": "Reads as any PHP repository once the nouns are removed",
        }
        self.assertIn("CONTENT_REVIEW_VERDICT_FAILED", self.codes())

    def test_an_open_blocker_blocks(self) -> None:
        self.review["blockers"] = ["The generate report counts skills that do not exist"]
        self.assertIn("CONTENT_REVIEW_BLOCKER_OPEN", self.codes())

    def test_a_review_by_the_author_is_refused(self) -> None:
        self.review["reviewer"] = "skill-forge"
        self.assertIn("CONTENT_REVIEW_NOT_INDEPENDENT", self.codes())

    def test_an_accepted_blocking_finding_is_refused(self) -> None:
        self.review["rounds"] = 1
        self.review["post_repair_gates"] = [
            "validate_skill_quality",
            "validate_generated",
        ]
        self.review["files"][0]["findings"] = [
            {
                "dimension": "completeness",
                "severity": "blocking",
                "statement": "The procedure never says how to verify the fix",
                "resolution": {
                    "action": "accepted",
                    "note": "Looked close enough",
                },
            }
        ]
        self.assertIn("CONTENT_REVIEW_BLOCKER_ACCEPTED", self.codes())

    def test_an_open_escalation_blocks_publication(self) -> None:
        self.review["files"][0]["findings"] = [
            {
                "dimension": "accuracy",
                "severity": "blocking",
                "statement": "The cited migration no longer exists in the target",
                "resolution": {
                    "action": "escalated",
                    "note": "Needs a re-scan; evidence cannot be invented",
                },
            }
        ]
        self.assertIn("CONTENT_REVIEW_ESCALATION_OPEN", self.codes())

    def test_a_repair_by_a_forge_that_does_not_own_the_lane_is_refused(self) -> None:
        self.review["rounds"] = 1
        self.review["post_repair_gates"] = [
            "validate_skill_quality",
            "validate_generated",
        ]
        self.review["files"][1]["findings"] = [
            {
                "dimension": "completeness",
                "severity": "blocking",
                "statement": "DOD names no verification commands",
                "resolution": {
                    "action": "reforged",
                    "forge": "skill-forge",
                    "round": 1,
                    "note": "Regenerated with the target's own gate commands",
                },
            }
        ]
        self.assertIn("CONTENT_REVIEW_FORGE_MISMATCH", self.codes())

    def test_repairs_without_rerun_gates_are_refused(self) -> None:
        self.review["rounds"] = 1
        self.review["files"][0]["findings"] = [
            {
                "dimension": "completeness",
                "severity": "blocking",
                "statement": "Verification section stated an impression",
                "resolution": {
                    "action": "reforged",
                    "forge": "skill-forge",
                    "round": 1,
                    "note": "Re-authored with a falsifiable check",
                },
            }
        ]
        self.assertIn("CONTENT_REVIEW_GATES_NOT_RERUN", self.codes())

    def test_rounds_beyond_the_bound_are_refused(self) -> None:
        self.review["rounds"] = 3
        self.review["post_repair_gates"] = [
            "validate_skill_quality",
            "validate_generated",
        ]
        self.assertIn("CONTENT_REVIEW_ROUNDS_EXCEEDED", self.codes())

    def test_a_verbatim_claim_outside_the_runtime_surface_is_refused(self) -> None:
        self.plan_lines.append(".claude/skills/deploys/SKILL.md")
        self.review["files"].append(
            {
                "path": ".claude/skills/deploys/SKILL.md",
                "lane": "runtime-verbatim",
                "basis": "It looked generic anyway",
            }
        )
        self.assertIn("CONTENT_REVIEW_VERBATIM_UNSANCTIONED", self.codes())

    def test_a_review_that_never_walked_the_plan_is_refused(self) -> None:
        self.review["files"] = [
            item
            for item in self.review["files"]
            if item["path"].rsplit("/", 1)[-1] != "skill-generation-plan.json"
        ]
        self.assertIn("CONTENT_REVIEW_REPORTS_MISSING", self.codes())

    def test_a_missing_review_fails_closed(self) -> None:
        plan_path = self.base / "publication-plan.txt"
        plan_path.write_text("\n".join(self.plan_lines) + "\n", encoding="utf-8")
        codes = [
            item.code
            for item in validator.validate(plan_path, self.base / "absent.json")
            if item.severity == "error"
        ]
        self.assertEqual(codes, ["CONTENT_REVIEW_UNREADABLE"])

    def test_the_json_report_is_byte_stable_across_runs(self) -> None:
        import contextlib
        import io

        plan_path = self.base / "publication-plan.txt"
        review_path = self.base / "review.json"
        plan_path.write_text("\n".join(self.plan_lines) + "\n", encoding="utf-8")
        review_path.write_text(json.dumps(self.review), encoding="utf-8")
        outputs = []
        for _ in range(2):
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                validator.main(
                    [
                        "--publication-plan",
                        str(plan_path),
                        "--review",
                        str(review_path),
                        "--json",
                    ]
                )
            outputs.append(buffer.getvalue())
        self.assertEqual(outputs[0], outputs[1])


if __name__ == "__main__":
    unittest.main()
