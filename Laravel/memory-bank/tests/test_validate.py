#!/usr/bin/env python3
"""Tests for the dependency-free memory-bank validator."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import tempfile
import unittest
from datetime import date, timedelta
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[1] / "scripts" / "validate.py"
REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("memory_bank_validate", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("Cannot load memory-bank validator")
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


class MemoryBankValidatorTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="memory-bank-test-")
        self.repository = Path(self.temporary.name)
        self.bank = self.repository / "memory-bank"
        self.bank.mkdir()
        self.bank.joinpath("README.md").write_text("# Memory Bank\n", encoding="utf-8")
        self.bank.joinpath("INDEX.md").write_text(self.index([]), encoding="utf-8")
        self.bank.joinpath(".memory-counter").write_text("1\n", encoding="utf-8")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    @staticmethod
    def index(rows: list[str]) -> str:
        return (
            "# Memory Index\n\n"
            "| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n"
            "| --- | --- | --- | --- | --- | --- | --- | --- |\n"
            + "\n".join(rows)
            + "\n"
        )

    def add_chunk(self, body: str = "Verified reusable context.\n") -> None:
        self.repository.joinpath("AGENTS.md").write_text("# Policy\n", encoding="utf-8")
        today = date.today()
        metadata = {
            "id": "MEM-0001",
            "title": "Layering convention",
            "type": "convention",
            "status": "active",
            "scope": ["application"],
            "tags": ["laravel", "architecture"],
            "created": today.isoformat(),
            "last_verified": today.isoformat(),
            "review_after": (today + timedelta(days=365)).isoformat(),
            "sources": ["AGENTS.md"],
            "supersedes": [],
            "superseded_by": None,
        }
        chunks = self.bank / "chunks"
        chunks.mkdir()
        chunks.joinpath("MEM-0001-layering-convention.md").write_text(
            f"---\n{json.dumps(metadata, indent=2)}\n---\n\n# Layering Convention\n\n{body}",
            encoding="utf-8",
        )
        self.bank.joinpath("INDEX.md").write_text(
            self.index(
                [
                    f"| MEM-0001 | Layering convention | convention | application | laravel, architecture | active | {today.isoformat()} | chunks/MEM-0001-layering-convention.md |"
                ]
            ),
            encoding="utf-8",
        )
        self.bank.joinpath(".memory-counter").write_text("2\n", encoding="utf-8")

    def update_chunk_metadata(self, **changes: object) -> None:
        chunk = self.bank / "chunks" / "MEM-0001-layering-convention.md"
        metadata, body = chunk.read_text(encoding="utf-8")[4:].split("\n---\n", 1)
        decoded = json.loads(metadata)
        decoded.update(changes)
        chunk.write_text(
            f"---\n{json.dumps(decoded, indent=2)}\n---\n{body}",
            encoding="utf-8",
        )

    def add_valid_supersession(self) -> None:
        self.add_chunk()
        today = date.today().isoformat()
        self.update_chunk_metadata(status="superseded", superseded_by="MEM-0002")
        old_chunk = self.bank / "chunks" / "MEM-0001-layering-convention.md"
        old_metadata = json.loads(old_chunk.read_text(encoding="utf-8")[4:].split("\n---\n", 1)[0])
        replacement = {
            **old_metadata,
            "id": "MEM-0002",
            "title": "Updated layering convention",
            "status": "active",
            "supersedes": ["MEM-0001"],
            "superseded_by": None,
        }
        self.bank.joinpath("chunks/MEM-0002-updated-layering.md").write_text(
            f"---\n{json.dumps(replacement, indent=2)}\n---\n\n# Updated Layering\n",
            encoding="utf-8",
        )
        self.bank.joinpath("INDEX.md").write_text(
            self.index(
                [
                    f"| MEM-0001 | Layering convention | convention | application | laravel, architecture | superseded | {today} | chunks/MEM-0001-layering-convention.md |",
                    f"| MEM-0002 | Updated layering convention | convention | application | laravel, architecture | active | {today} | chunks/MEM-0002-updated-layering.md |",
                ]
            ),
            encoding="utf-8",
        )
        self.bank.joinpath(".memory-counter").write_text("3\n", encoding="utf-8")

    def test_empty_initialized_bank_is_valid(self) -> None:
        self.assertEqual([], VALIDATOR.validate_bank(self.bank))

    def test_indexed_source_backed_chunk_is_valid(self) -> None:
        self.add_chunk()

        self.assertEqual([], VALIDATOR.validate_bank(self.bank))

    def test_all_session_hooks_report_json_frontmatter_status(self) -> None:
        self.add_chunk('```json\n"status": "needs-review",\n```\n')

        for edition in (".claude", ".cursor", ".codex"):
            with self.subTest(edition=edition):
                result = subprocess.run(
                    ["bash", str(REPOSITORY_ROOT / edition / "hooks" / "local-context.sh")],
                    cwd=self.repository,
                    text=True,
                    capture_output=True,
                    check=True,
                )
                self.assertIn(
                    "Memory bank: 1 chunks (1 active, 0 needs review).",
                    result.stdout,
                )

    def test_chunks_directory_rejects_unexpected_file(self) -> None:
        chunks = self.bank / "chunks"
        chunks.mkdir()
        chunks.joinpath("invalid-name.md").write_text("Untracked memory.\n", encoding="utf-8")

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("unexpected chunk entry" in error for error in errors))

    def test_chunks_directory_rejects_nested_directory(self) -> None:
        chunks = self.bank / "chunks"
        chunks.mkdir()
        chunks.joinpath("nested").mkdir()

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("unexpected chunk entry" in error for error in errors))

    def test_secret_like_value_is_rejected_without_echoing_it(self) -> None:
        fake_token = "ghp_" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"
        self.add_chunk(f"access_token={fake_token}\n")

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("possible GitHub token" in error for error in errors))
        self.assertFalse(any("ABCDEFGHIJKLMNOPQRSTUVWXYZ" in error for error in errors))

    def test_openai_style_token_is_rejected_without_echoing_it(self) -> None:
        fake_token = "sk-proj-" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"
        self.add_chunk(f"{fake_token}\n")

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("possible OpenAI-style token" in error for error in errors))
        self.assertFalse(any("ABCDEFGHIJKLMNOPQRSTUVWXYZ" in error for error in errors))

    def test_counter_must_advance_past_allocated_ids(self) -> None:
        self.add_chunk()
        self.bank.joinpath(".memory-counter").write_text("1\n", encoding="utf-8")

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("counter must be greater" in error for error in errors))

    def test_local_source_must_stay_inside_repository(self) -> None:
        self.add_chunk()
        self.update_chunk_metadata(sources=["../"])

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("escapes the repository" in error for error in errors))

    def test_active_chunk_cannot_pass_after_review_date(self) -> None:
        self.add_chunk()
        verified = date.today() - timedelta(days=2)
        self.update_chunk_metadata(
            created=verified.isoformat(),
            last_verified=verified.isoformat(),
            review_after=(date.today() - timedelta(days=1)).isoformat(),
        )

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("overdue for review" in error for error in errors))

    def test_index_scope_must_match_chunk_metadata(self) -> None:
        self.add_chunk()
        index = self.bank.joinpath("INDEX.md")
        index.write_text(
            index.read_text(encoding="utf-8").replace(
                "| convention | application |",
                "| convention | infrastructure |",
            ),
            encoding="utf-8",
        )

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("scope differs from chunk metadata" in error for error in errors))

    def test_chunk_cannot_supersede_itself(self) -> None:
        self.add_chunk()
        self.update_chunk_metadata(supersedes=["MEM-0001"])

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("cannot supersede itself" in error for error in errors))

    def test_bidirectional_supersession_is_valid(self) -> None:
        self.add_valid_supersession()

        self.assertEqual([], VALIDATOR.validate_bank(self.bank))

    def test_supersession_requires_replacement_backlink(self) -> None:
        self.add_valid_supersession()
        replacement = self.bank / "chunks" / "MEM-0002-updated-layering.md"
        metadata, body = replacement.read_text(encoding="utf-8")[4:].split("\n---\n", 1)
        decoded = json.loads(metadata)
        decoded["supersedes"] = []
        replacement.write_text(
            f"---\n{json.dumps(decoded, indent=2)}\n---\n{body}",
            encoding="utf-8",
        )

        errors = VALIDATOR.validate_bank(self.bank)

        self.assertTrue(any("must include this ID in supersedes" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
