#!/usr/bin/env python3
"""Regression tests for deterministic shared .gitignore proposals."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(SCRIPTS))

import merge_gitignore as merger  # noqa: E402


BEGIN = b"# BEGIN Infrastructure-Creator .gitignore v1"
END = b"# END Infrastructure-Creator .gitignore v1"


class GitignoreMergeTest(unittest.TestCase):
    def merge(self, existing: bytes | None, requirements: list[str]):
        return merger.merge_gitignore(existing, requirements)

    def test_absent_file_gets_sorted_deduplicated_managed_block(self) -> None:
        result = self.merge(None, ["memory-bank/local/", ".cursor-cache/", "memory-bank/local/"])
        self.assertEqual(
            result.staged_bytes,
            BEGIN
            + b"\n.cursor-cache/\nmemory-bank/local/\n"
            + END
            + b"\n",
        )
        self.assertEqual(
            result.metadata["requirements"],
            [".cursor-cache/", "memory-bank/local/"],
        )
        self.assertEqual(result.metadata["strategy"], "append-managed-block")
        self.assertEqual(
            result.metadata["original_sha256"], hashlib.sha256(b"").hexdigest()
        )
        self.assertEqual(
            result.metadata["proposal_sha256"],
            hashlib.sha256(result.staged_bytes).hexdigest(),
        )
        self.assertEqual(
            result.metadata["resolved_sha256"],
            result.metadata["proposal_sha256"],
        )

    def test_api_accepts_requirement_json_bytes(self) -> None:
        from_json = merger.merge_gitignore(
            b"vendor/\n", b'{"requirements":["cache/","vendor/"]}'
        )
        from_values = self.merge(b"vendor/\n", ["cache/", "vendor/"])
        self.assertEqual(from_json, from_values)

    def test_complete_team_file_is_byte_identical(self) -> None:
        existing = b"# team order\nvendor/\n\n/cache/\nvendor/\n"
        result = self.merge(existing, ["/cache/", "vendor/"])
        self.assertEqual(result.staged_bytes, existing)
        self.assertFalse(result.metadata["changed"])
        self.assertEqual(result.metadata["strategy"], "unchanged")

    def test_partial_file_preserves_prefix_and_missing_final_newline(self) -> None:
        existing = b"# team comment\nvendor/"
        result = self.merge(existing, ["vendor/", "memory-bank/local/"])
        self.assertTrue(result.staged_bytes.startswith(existing))
        self.assertEqual(
            result.staged_bytes,
            existing
            + b"\n\n"
            + BEGIN
            + b"\nmemory-bank/local/\n"
            + END
            + b"\n",
        )

    def test_empty_existing_file_behaves_like_absent_file(self) -> None:
        absent = self.merge(None, ["cache/"])
        empty = self.merge(b"", ["cache/"])
        self.assertEqual(empty.staged_bytes, absent.staged_bytes)
        self.assertEqual(empty.metadata, absent.metadata)

    def test_consistent_crlf_is_used_for_generated_bytes(self) -> None:
        existing = b"\xef\xbb\xbf# team\r\nvendor/\r\n"
        result = self.merge(existing, ["vendor/", "cache/"])
        self.assertTrue(result.staged_bytes.startswith(existing))
        self.assertNotIn(b"\n", result.staged_bytes.replace(b"\r\n", b""))
        self.assertEqual(result.metadata["newline"], "crlf")
        self.assertTrue(result.staged_bytes.startswith(b"\xef\xbb\xbf"))

    def test_mixed_and_newline_free_sources_use_lf(self) -> None:
        for existing in (b"a\r\nb\n", b"# no newline"):
            with self.subTest(existing=existing):
                result = self.merge(existing, ["new/"])
                appended = result.staged_bytes[len(existing) :]
                self.assertNotIn(b"\r", appended)
                self.assertEqual(result.metadata["newline"], "lf")

    def test_negated_entry_does_not_satisfy_positive_entry(self) -> None:
        existing = b"!cache/\ncache/\n"
        complete = self.merge(existing, ["!cache/", "cache/"])
        partial = self.merge(b"!cache/\n", ["cache/"])
        self.assertEqual(complete.staged_bytes, existing)
        self.assertIn(b"\ncache/\n", partial.staged_bytes)

    def test_changed_requirements_replace_only_the_managed_block(self) -> None:
        before = (
            b"# team before\nvendor/\n\n"
            + BEGIN
            + b"\nold/\n"
            + END
            + b"\n# team after\n"
        )
        first = self.merge(before, ["new/", "vendor/"])
        expected = (
            b"# team before\nvendor/\n\n"
            + BEGIN
            + b"\nnew/\n"
            + END
            + b"\n# team after\n"
        )
        self.assertEqual(first.staged_bytes, expected)
        self.assertEqual(first.metadata["strategy"], "replace-managed-block")
        second = self.merge(first.staged_bytes, ["vendor/", "new/"])
        self.assertEqual(second.staged_bytes, first.staged_bytes)
        self.assertEqual(second.metadata["strategy"], "unchanged")
        self.assertEqual(
            second.metadata["proposal_sha256"], first.metadata["proposal_sha256"]
        )

    def test_requirement_now_satisfied_by_team_is_removed_from_block(self) -> None:
        existing = (
            b"cache/\n"
            + BEGIN
            + b"\ncache/\nother/\n"
            + END
            + b"\n"
        )
        result = self.merge(existing, ["cache/", "other/"])
        self.assertEqual(result.staged_bytes.count(b"cache/"), 1)
        self.assertIn(b"\nother/\n", result.staged_bytes)

    def test_malformed_duplicate_and_unknown_markers_fail_closed(self) -> None:
        cases = (
            BEGIN + b"\nentry/\n",
            END + b"\n",
            BEGIN + b"\n" + END + b"\n" + BEGIN + b"\n" + END + b"\n",
            b"# BEGIN Infrastructure-Creator .gitignore v2\n"
            b"x/\n# END Infrastructure-Creator .gitignore v2\n",
        )
        for existing in cases:
            with self.subTest(existing=existing):
                with self.assertRaises(merger.GitignoreMergeError):
                    self.merge(existing, ["entry/"])

    def test_invalid_existing_encodings_are_rejected(self) -> None:
        for existing in (b"ok\x00bad", b"\xff"):
            with self.subTest(existing=existing):
                with self.assertRaises(merger.GitignoreMergeError):
                    self.merge(existing, ["cache/"])

    def test_requirement_injection_and_invalid_json_are_rejected(self) -> None:
        invalid_api = ["cache/\n.env", "cache/\r.env", "cache/\t.env", "\u202ebad"]
        for requirement in invalid_api:
            with self.subTest(requirement=requirement):
                with self.assertRaises(merger.GitignoreMergeError):
                    self.merge(None, [requirement])
        for raw in (b"\xff", b'{"requirements":["ok"],"extra":[]}', b"{}"):
            with self.subTest(raw=raw):
                with self.assertRaises(merger.GitignoreMergeError):
                    merger.load_requirements_json(raw)

    def test_cli_writes_staged_bytes_and_deterministic_metadata(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gitignore-merge-") as temporary:
            root = Path(temporary)
            existing = root / ".gitignore"
            requirements = root / "requirements.json"
            output = root / "staged" / ".gitignore"
            metadata = root / "proposal.json"
            existing.write_bytes(b"# team\nvendor/\n")
            requirements.write_text(
                json.dumps({"requirements": ["cache/", "vendor/"]}),
                encoding="utf-8",
            )
            command = [
                sys.executable,
                str(SCRIPTS / "merge_gitignore.py"),
                "--existing",
                str(existing),
                "--requirements",
                str(requirements),
                "--output",
                str(output),
                "--metadata",
                str(metadata),
            ]
            first = subprocess.run(command, capture_output=True, check=False)
            self.assertEqual(first.returncode, 0, first.stderr.decode())
            expected = self.merge(existing.read_bytes(), ["cache/", "vendor/"])
            self.assertEqual(output.read_bytes(), expected.staged_bytes)
            self.assertEqual(
                json.loads(metadata.read_text(encoding="utf-8")), expected.metadata
            )

            second = subprocess.run(command, capture_output=True, check=False)
            self.assertEqual(second.returncode, 0, second.stderr.decode())
            self.assertEqual(output.read_bytes(), expected.staged_bytes)
            self.assertEqual(
                json.loads(metadata.read_text(encoding="utf-8")), expected.metadata
            )

    def test_cli_unions_multiple_forge_documents_independent_of_order(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gitignore-forges-") as temporary:
            root = Path(temporary)
            first = root / "memory.json"
            second = root / "hooks.json"
            output_a = root / "a.gitignore"
            output_b = root / "b.gitignore"
            first.write_text(
                json.dumps(["memory-bank/local/", "__pycache__/"]),
                encoding="utf-8",
            )
            second.write_text(
                json.dumps([".cursor/rules/working-memory.mdc", "__pycache__/"]),
                encoding="utf-8",
            )
            base = [sys.executable, str(SCRIPTS / "merge_gitignore.py")]
            for documents, output in (
                ((first, second), output_a),
                ((second, first), output_b),
            ):
                command = list(base)
                for document in documents:
                    command.extend(["--requirements", str(document)])
                command.extend(["--output", str(output)])
                result = subprocess.run(
                    command, text=True, capture_output=True, check=False
                )
                self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output_a.read_bytes(), output_b.read_bytes())


if __name__ == "__main__":
    unittest.main()
