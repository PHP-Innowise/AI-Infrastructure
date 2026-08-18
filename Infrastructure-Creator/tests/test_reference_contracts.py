#!/usr/bin/env python3
"""Regression tests for canonical and sibling-generator reference catalogs."""

from __future__ import annotations

import sys
import shutil
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(SCRIPTS))

from validate_reference_catalogs import validate  # noqa: E402


class ReferenceContractTest(unittest.TestCase):
    def test_canonical_six_catalogs_are_complete(self) -> None:
        references = ROOT / ".agents/skills/skill-forge/references"
        self.assertEqual(validate(references), [])

    def test_stub_and_source_stack_leftover_fail(self) -> None:
        with tempfile.TemporaryDirectory(prefix="reference-contract-") as temporary:
            references = Path(temporary)
            stub = "# Catalog\n\nSelection evidence for a PHP skill.\n"
            for index in range(6):
                (references / f"catalog-{index}.md").write_text(stub, encoding="utf-8")
            errors = validate(references, ["PHP"])
        self.assertTrue(any("stub" in error for error in errors))
        self.assertTrue(any("forbidden source-stack" in error for error in errors))
        self.assertTrue(any("procedure" in error for error in errors))
        self.assertTrue(any("candidate-registry" in error for error in errors))

    def test_empty_candidate_registry_is_rejected(self) -> None:
        source = ROOT / ".agents/skills/skill-forge/references"
        with tempfile.TemporaryDirectory(prefix="reference-empty-") as temporary:
            references = Path(temporary)
            for path in source.glob("*"):
                if path.is_file():
                    shutil.copy2(path, references / path.name)
            (references / "candidate-registry.json").write_text(
                '{"schema_version": "1.0", "candidates": []}\n', encoding="utf-8"
            )
            errors = validate(references)
        self.assertTrue(
            any("candidates must list at least one entry" in error for error in errors)
        )

    def test_unreadable_catalog_is_a_validation_error_not_a_crash(self) -> None:
        source = ROOT / ".agents/skills/skill-forge/references"
        with tempfile.TemporaryDirectory(prefix="reference-decode-") as temporary:
            references = Path(temporary)
            for path in source.glob("*"):
                if path.is_file():
                    shutil.copy2(path, references / path.name)
            corrupted = sorted(references.glob("*.md"))[0]
            corrupted.write_bytes(b"\xff\xfe invalid utf-8 catalog\n")
            errors = validate(references)
        self.assertTrue(
            any(
                error.startswith(f"{corrupted.name}: cannot read catalog")
                for error in errors
            )
        )

    def test_registry_catalog_anchor_must_resolve(self) -> None:
        source = ROOT / ".agents/skills/skill-forge/references"
        with tempfile.TemporaryDirectory(prefix="reference-registry-") as temporary:
            references = Path(temporary)
            for path in source.glob("*"):
                if path.is_file():
                    shutil.copy2(path, references / path.name)
            registry_path = references / "candidate-registry.json"
            text = registry_path.read_text(encoding="utf-8")
            registry_path.write_text(
                text.replace(
                    "php-frameworks.md#architecture-implementer",
                    "php-frameworks.md#missing-contract",
                    1,
                ),
                encoding="utf-8",
            )
            errors = validate(references)
        self.assertTrue(any("catalog anchor" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
