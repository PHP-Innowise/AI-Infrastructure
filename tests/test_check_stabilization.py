#!/usr/bin/env python3
"""Regression tests for scripts/check_stabilization.py.

Every test injects one malformed rule and asserts the finding. A policy
validator that only ever passes is the unverified convention it was written to
replace, so the failing direction is the one that matters here.

The fixtures build a miniature edition in a temporary directory; the shipped
documents are exercised once, through the real CLI.

Run: python3 -m unittest tests.test_check_stabilization
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import check_stabilization as validator  # noqa: E402


WELL_FORMED = """**Trigger:** A controller used raw request data directly.
**Root cause:** Guidance did not require validation before the service call.
**Rule:** MUST validate HTTP input before using it.
**Example:**
- Incorrect: `$service->create($request->all())`
- Correct: `$service->create($dto)`
**Enforcement:** `AGENTS.md`, `coder` skill.
**Added:** 2026-01-01
"""


class StabilizationFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="stabilization-test-")
        self.root = Path(self.temporary.name)
        self.edition = self.root / "Symfony"
        (self.edition / ".claude").mkdir(parents=True)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write(self, *, live: str = "", retired: str = "") -> None:
        text = "## Rule Template\n\n### Rule: [Short Name]\n\n**Trigger:** [x]\n\n"
        text += "## Examples\n\n" + live
        if retired:
            text += "\n## Retired rules\n\n" + retired
        (self.edition / ".claude" / "STABILIZATION.md").write_text(
            text, encoding="utf-8"
        )

    def findings(self) -> list[str]:
        found, _, _ = validator.check_edition("Symfony", self.root)
        return [item["message"] for item in found]

    def rule(self, name: str, body: str = WELL_FORMED) -> str:
        return f"### Rule: {name}\n\n{body}\n"


class WellFormedTests(StabilizationFixture):
    def test_a_conforming_rule_reports_nothing(self) -> None:
        self.write(live=self.rule("Validate At The Boundary"))
        self.assertEqual([], self.findings())

    def test_the_template_block_is_not_treated_as_a_rule(self) -> None:
        # The template is an example of the shape, not an instance of it.
        self.write(live=self.rule("Validate At The Boundary"))
        _, count, _ = validator.check_edition("Symfony", self.root)
        self.assertEqual(1, count)

    def test_a_prose_document_is_skipped_not_silently_passed(self) -> None:
        (self.edition / ".claude" / "STABILIZATION.md").write_text(
            "## Cycle\n\nDiagnose, then release.\n", encoding="utf-8"
        )
        found, count, skipped = validator.check_edition("Symfony", self.root)
        self.assertEqual([], found)
        self.assertEqual(0, count)
        self.assertTrue(skipped)


class MalformedRuleTests(StabilizationFixture):
    def test_a_missing_required_field_is_reported(self) -> None:
        body = WELL_FORMED.replace("**Enforcement:** `AGENTS.md`, `coder` skill.\n", "")
        self.write(live=self.rule("Unenforced", body))
        self.assertIn("missing required field Enforcement", self.findings())

    def test_a_rule_that_states_no_obligation_is_reported(self) -> None:
        # "Prefer X" is advice. A rule the harness cannot check compliance
        # against is the shape this document exists to stop.
        body = WELL_FORMED.replace(
            "**Rule:** MUST validate HTTP input before using it.",
            "**Rule:** Prefer validating HTTP input before using it.",
        )
        self.write(live=self.rule("Merely Advisory", body))
        self.assertIn(
            "Rule must state an obligation (MUST or MUST NOT)", self.findings()
        )

    def test_an_empty_enforcement_is_reported(self) -> None:
        body = WELL_FORMED.replace(
            "**Enforcement:** `AGENTS.md`, `coder` skill.", "**Enforcement:**"
        )
        self.write(live=self.rule("Nowhere Enforced", body))
        self.assertIn(
            "Enforcement must name where the rule is enforced", self.findings()
        )

    def test_a_non_iso_date_is_reported(self) -> None:
        body = WELL_FORMED.replace("**Added:** 2026-01-01", "**Added:** last Tuesday")
        self.write(live=self.rule("Undated", body))
        self.assertTrue(
            any("must be an ISO date" in message for message in self.findings())
        )

    def test_a_retired_rule_left_among_the_live_ones_is_reported(self) -> None:
        # Still policy in every reader's eyes until it is moved.
        body = WELL_FORMED + "**Retired:** 2026-02-01\n"
        self.write(live=self.rule("Retired In Place", body))
        self.assertTrue(
            any("belongs under" in message for message in self.findings())
        )

    def test_a_live_rule_in_the_retired_section_is_reported(self) -> None:
        self.write(retired=self.rule("Filed Too Early"))
        self.assertTrue(
            any("without a Retired date" in message for message in self.findings())
        )

    def test_a_retired_rule_in_the_retired_section_is_accepted(self) -> None:
        body = WELL_FORMED + "**Retired:** 2026-02-01\n"
        self.write(retired=self.rule("Properly Retired", body))
        self.assertEqual([], self.findings())

    def test_superseded_by_must_name_a_rule_in_the_same_file(self) -> None:
        body = WELL_FORMED + "**Retired:** 2026-02-01\n**Superseded-by:** Nowhere\n"
        self.write(retired=self.rule("Dangling Link", body))
        self.assertTrue(
            any("names no rule in this file" in message for message in self.findings())
        )

    def test_superseded_by_resolves_to_a_sibling_rule(self) -> None:
        retired = WELL_FORMED + (
            "**Retired:** 2026-02-01\n**Superseded-by:** Validate At The Boundary\n"
        )
        self.write(
            live=self.rule("Validate At The Boundary"),
            retired=self.rule("Older Rule", retired),
        )
        self.assertEqual([], self.findings())


class EvidenceTests(StabilizationFixture):
    RECORD = "11111111-1111-4111-8111-111111111111"

    def test_evidence_must_be_a_record_uuid(self) -> None:
        body = WELL_FORMED + "**Evidence:** FIND-1\n"
        self.write(live=self.rule("Loose Evidence", body))
        self.assertTrue(
            any("must be a record UUID" in message for message in self.findings())
        )

    def test_evidence_naming_no_record_on_disk_is_reported(self) -> None:
        body = WELL_FORMED + f"**Evidence:** {self.RECORD}\n"
        self.write(live=self.rule("Absent Evidence", body))
        self.assertTrue(
            any("names no Project Brain record" in message for message in self.findings())
        )

    def test_evidence_resolving_to_a_record_is_accepted(self) -> None:
        records = self.edition / "project-brain" / "dynamic" / "findings"
        records.mkdir(parents=True)
        (records / f"{self.RECORD}.md").write_text("record\n", encoding="utf-8")
        body = WELL_FORMED + f"**Evidence:** {self.RECORD}\n"
        self.write(live=self.rule("Backed Evidence", body))
        self.assertEqual([], self.findings())

    def test_evidence_is_optional(self) -> None:
        # Requiring it would invalidate every rule written before the field
        # existed; checked-when-present is the contract.
        self.write(live=self.rule("No Evidence Yet"))
        self.assertEqual([], self.findings())


class ShippedDocumentTests(unittest.TestCase):
    def test_the_shipped_rules_are_well_formed(self) -> None:
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts" / "check_stabilization.py")],
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_json_mode_names_what_was_skipped(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts" / "check_stabilization.py"),
                "--json",
            ],
            text=True,
            capture_output=True,
        )
        payload = json.loads(result.stdout)
        self.assertEqual([], payload["findings"])
        # A validator that silently approves a file it never examined is the
        # defect it exists to prevent, so the skip has to be visible.
        self.assertIn("Infrastructure-Creator", payload["skipped"])
        self.assertTrue(payload["checked"])


if __name__ == "__main__":
    unittest.main()
