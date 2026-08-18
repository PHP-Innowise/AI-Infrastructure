#!/usr/bin/env python3
"""Focused regressions for manifest-relative placeholder declarations."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(SCRIPTS))

VALIDATOR_PATH = SCRIPTS / "validate_generated.py"
SPEC = importlib.util.spec_from_file_location("validate_generated", VALIDATOR_PATH)
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)


class PlaceholderContractTest(unittest.TestCase):
    def setUp(self) -> None:
        self._temporary = tempfile.TemporaryDirectory(prefix="placeholder-contract-")
        self.target = Path(self._temporary.name).resolve()
        self.addCleanup(self._temporary.cleanup)

    def validate(self, files: dict[str, str]) -> list[str]:
        manifest_files: dict[str, str] = {}
        for relative_path, content in files.items():
            path = self.target / relative_path
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
            manifest_files[relative_path] = "unused-by-placeholder-validation"

        errors: list[str] = []
        validator.validate_owned_placeholders(self.target, manifest_files, errors)
        return errors

    def test_approved_iso_date_occurrences_pass_only_in_declared_assets(self) -> None:
        errors = self.validate(
            {
                "memory-bank/templates/chunk.md": "Created: YYYY-MM-DD\n",
                "memory-bank/scripts/validate.py": 'DATE_FORMAT = "YYYY-MM-DD"\n',
            }
        )

        self.assertEqual(errors, [])

    def test_other_placeholders_still_fail_in_declared_assets(self) -> None:
        errors = self.validate(
            {
                "memory-bank/templates/chunk.md": "Created: YYYY-MM-DD\nTODO\n",
                "memory-bank/scripts/validate.py": 'DATE_FORMAT = "YYYY-MM-DD"\nFIXME\n',
            }
        )

        self.assertEqual(len(errors), 2)
        self.assertTrue(any(r"/\bTODO\b/" in error for error in errors))
        self.assertTrue(any(r"/\bFIXME\b/" in error for error in errors))
        self.assertFalse(any(r"/\bYYYY-MM-DD\b/" in error for error in errors))

    def test_repeated_placeholder_reports_one_error_per_file_and_pattern(self) -> None:
        errors = self.validate(
            {"memory-bank/templates/notes.md": "TODO\nTODO\nTODO\n"}
        )

        self.assertEqual(len(errors), 1)
        self.assertIn(r"/\bTODO\b/", errors[0])

    def test_same_iso_date_token_fails_at_every_other_path(self) -> None:
        errors = self.validate(
            {
                "memory-bank/templates/other.md": "Created: YYYY-MM-DD\n",
                "memory-bank/scripts/helper.py": 'DATE_FORMAT = "YYYY-MM-DD"\n',
            }
        )

        self.assertEqual(len(errors), 2)
        self.assertTrue(all(r"/\bYYYY-MM-DD\b/" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
