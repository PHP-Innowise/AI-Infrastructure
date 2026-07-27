#!/usr/bin/env python3
"""Integration tests for the repository-local context engine."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "context.py"


class ContextEngineTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="context-engine-test-")
        self.repository = Path(self.temporary.name)
        self.repository.joinpath("memory-bank/chunks").mkdir(parents=True)
        self.repository.joinpath("specs").mkdir()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_context(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.repository), *arguments],
            text=True,
            capture_output=True,
        )

    def write_memory(self, name: str, status: str, body: str) -> None:
        self.repository.joinpath("memory-bank/chunks", name).write_text(
            f'---\n{{"status": "{status}"}}\n---\n\n{body}\n',
            encoding="utf-8",
        )

    def test_index_makes_living_spec_searchable(self) -> None:
        self.repository.joinpath("specs/billing.md").write_text(
            "# Billing\n\nInvoice ownership stays with the tenant account.\n",
            encoding="utf-8",
        )

        indexed = self.run_context("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)

        searched = self.run_context("search", "invoice ownership", "--json")
        self.assertEqual(0, searched.returncode, searched.stderr)
        payload = json.loads(searched.stdout)
        self.assertEqual(
            ["specs/billing.md"],
            [item["path"] for item in payload["documents"]],
        )

    def test_index_includes_only_active_memory(self) -> None:
        self.write_memory(
            "MEM-0001-current-convention.md",
            "active",
            "# Current Convention\n\nUse the aurora deployment workflow.",
        )
        self.write_memory(
            "MEM-0002-old-convention.md",
            "superseded",
            "# Old Convention\n\nUse the zephyr deployment workflow.",
        )

        indexed = self.run_context("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)

        current = self.run_context("search", "aurora", "--json")
        self.assertEqual(0, current.returncode, current.stderr)
        self.assertEqual(
            ["memory-bank/chunks/MEM-0001-current-convention.md"],
            [item["path"] for item in json.loads(current.stdout)["documents"]],
        )

        old = self.run_context("search", "zephyr", "--json")
        self.assertEqual(0, old.returncode, old.stderr)
        self.assertEqual([], json.loads(old.stdout)["documents"])

    def test_recorded_episode_is_searchable(self) -> None:
        recorded = self.run_context(
            "record",
            "--summary",
            "Added the crimson retry boundary.",
            "--outcome",
            "Deployment retries are idempotent.",
            "--file",
            "src/Deployment/Retry.php",
            "--verification",
            "RetryTest passed",
            "--source",
            "specs/deployment.md",
            "--json",
        )
        self.assertEqual(0, recorded.returncode, recorded.stderr)

        searched = self.run_context("search", "crimson", "--json")
        self.assertEqual(0, searched.returncode, searched.stderr)
        episodes = json.loads(searched.stdout)["episodes"]
        self.assertEqual(1, len(episodes))
        self.assertEqual("Added the crimson retry boundary.", episodes[0]["summary"])
        self.assertEqual(["src/Deployment/Retry.php"], episodes[0]["files"])
        self.assertEqual(["RetryTest passed"], episodes[0]["verification"])
        self.assertEqual(["specs/deployment.md"], episodes[0]["sources"])

    def test_reindex_removes_deleted_document(self) -> None:
        specification = self.repository / "specs" / "temporary.md"
        specification.write_text(
            "# Temporary\n\nThe heliotrope rule is temporary.\n",
            encoding="utf-8",
        )
        first_index = self.run_context("index", "--json")
        self.assertEqual(0, first_index.returncode, first_index.stderr)
        specification.unlink()

        second_index = self.run_context("index", "--json")
        self.assertEqual(0, second_index.returncode, second_index.stderr)
        self.assertEqual(1, json.loads(second_index.stdout)["removed"])

        searched = self.run_context("search", "heliotrope", "--json")
        self.assertEqual(0, searched.returncode, searched.stderr)
        self.assertEqual([], json.loads(searched.stdout)["documents"])

    def test_record_rejects_secret_without_echoing_it(self) -> None:
        fake_token = "ghp_" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"

        recorded = self.run_context(
            "record",
            "--summary",
            f"Rotated credential {fake_token}",
            "--outcome",
            "Credential rotation completed.",
            "--json",
        )

        self.assertNotEqual(0, recorded.returncode)
        self.assertIn("possible GitHub token", recorded.stderr)
        self.assertNotIn("ABCDEFGHIJKLMNOPQRSTUVWXYZ", recorded.stderr)

        searched = self.run_context("search", "credential", "--json")
        self.assertEqual(0, searched.returncode, searched.stderr)
        self.assertEqual([], json.loads(searched.stdout)["episodes"])

    def test_status_reports_document_and_episode_counts(self) -> None:
        self.repository.joinpath("specs/status.md").write_text(
            "# Status\n\nStatus context.\n",
            encoding="utf-8",
        )
        self.assertEqual(0, self.run_context("index", "--json").returncode)
        self.assertEqual(
            0,
            self.run_context(
                "record",
                "--summary",
                "Recorded status episode.",
                "--outcome",
                "Status is visible.",
                "--json",
            ).returncode,
        )

        status = self.run_context("status", "--json")
        self.assertEqual(0, status.returncode, status.stderr)
        payload = json.loads(status.stdout)
        self.assertEqual(1, payload["documents"])
        self.assertEqual(1, payload["episodes"])
        self.assertTrue(payload["database"].endswith("memory-bank/local/context.db"))

    def test_search_rejects_unusable_query_and_limit(self) -> None:
        no_words = self.run_context("search", "!!!", "--json")
        self.assertNotEqual(0, no_words.returncode)
        self.assertIn("Search query must contain a word", no_words.stderr)

        zero_limit = self.run_context("search", "memory", "--limit", "0", "--json")
        self.assertNotEqual(0, zero_limit.returncode)
        self.assertIn("--limit must be a positive integer", zero_limit.stderr)

    def test_record_rejects_blank_required_fields(self) -> None:
        recorded = self.run_context(
            "record",
            "--summary",
            " ",
            "--outcome",
            "Completed.",
            "--json",
        )

        self.assertNotEqual(0, recorded.returncode)
        self.assertIn("Episode summary and outcome must not be empty", recorded.stderr)


if __name__ == "__main__":
    unittest.main()
