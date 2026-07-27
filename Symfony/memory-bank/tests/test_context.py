#!/usr/bin/env python3
"""Integration tests for the repository-local context engine."""

from __future__ import annotations

import importlib.util
import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from datetime import date, timedelta
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "context.py"
SPEC = importlib.util.spec_from_file_location("memory_bank_context", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("Cannot load context engine")
CONTEXT = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(SCRIPT.parent))
try:
    SPEC.loader.exec_module(CONTEXT)
finally:
    sys.path.pop(0)


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

    def write_memory(self, name: str, status: object, body: str) -> None:
        memory_id = "-".join(name.split("-", 2)[:2])
        today = date.today()
        self.repository.joinpath("AGENTS.md").write_text(
            "# Policy\n",
            encoding="utf-8",
        )
        metadata = {
            "id": memory_id,
            "title": name.removesuffix(".md"),
            "type": "convention",
            "status": status,
            "scope": ["application"],
            "tags": ["context"],
            "created": today.isoformat(),
            "last_verified": today.isoformat(),
            "review_after": (today + timedelta(days=365)).isoformat(),
            "sources": ["AGENTS.md"],
            "supersedes": [],
            "superseded_by": "MEM-9999" if status == "superseded" else None,
        }
        self.repository.joinpath("memory-bank/chunks", name).write_text(
            f"---\n{json.dumps(metadata, indent=2)}\n---\n\n{body}\n",
            encoding="utf-8",
        )

    def create_old_episode_database(self) -> Path:
        database = self.repository / "memory-bank/local/context.db"
        database.parent.mkdir()
        connection = sqlite3.connect(database)
        connection.executescript(
            """
            CREATE VIRTUAL TABLE episodes USING fts5(
                summary,
                outcome,
                files UNINDEXED,
                verification UNINDEXED,
                sources UNINDEXED,
                created_at UNINDEXED,
                tokenize = 'unicode61'
            );
            INSERT INTO episodes(
                rowid, summary, outcome, files, verification, sources, created_at
            ) VALUES (
                7,
                'Legacy task',
                'Legacy task completed',
                '["src/LegacyHandler.php"]',
                '["LegacyTest passed"]',
                '["specs/legacy.md"]',
                '2026-07-27T00:00:00+00:00'
            );
            """
        )
        connection.close()
        return database

    def create_old_document_database(self) -> Path:
        database = self.repository / "memory-bank/local/context.db"
        database.parent.mkdir()
        connection = sqlite3.connect(database)
        connection.executescript(
            """
            CREATE VIRTUAL TABLE documents USING fts5(
                path UNINDEXED,
                kind UNINDEXED,
                title,
                content,
                tokenize = 'unicode61'
            );
            INSERT INTO documents(path, kind, title, content) VALUES (
                'README.md', 'overview', 'Legacy', 'Legacy document content.'
            );
            CREATE VIRTUAL TABLE episodes USING fts5(
                summary,
                outcome,
                files,
                verification,
                sources,
                created_at UNINDEXED,
                tokenize = 'unicode61'
            );
            INSERT INTO episodes(
                summary, outcome, files, verification, sources, created_at
            ) VALUES (
                'Keep episode', 'Episode remains after document migration.',
                '[]', '[]', '[]', '2026-07-27T00:00:00+00:00'
            );
            """
        )
        connection.close()
        return database

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

    def test_index_classifies_layers_and_deduplicates_mirrored_skills(self) -> None:
        self.repository.joinpath("AGENTS.md").write_text(
            "# Policy\n\nUse the cobalt review procedure.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("README.md").write_text(
            "# Domain\n\nInvoices follow the amber ownership rule.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changes\n\nAdded the violet retry boundary.\n",
            encoding="utf-8",
        )
        for tool in (".agents", ".claude", ".cursor"):
            skill = self.repository / tool / "skills/review/SKILL.md"
            skill.parent.mkdir(parents=True)
            skill.write_text(
                "# Review\n\nRun the indigo verification procedure.\n",
                encoding="utf-8",
            )

        indexed = self.run_context("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)
        self.assertEqual(
            {"procedural": 2, "semantic": 1, "episodic": 1},
            json.loads(indexed.stdout)["layers"],
        )

        searched = self.run_context("search", "indigo", "--json")
        documents = json.loads(searched.stdout)["documents"]
        self.assertEqual(1, len(documents))
        self.assertEqual("procedural", documents[0]["layer"])

    def test_index_includes_common_project_documentation(self) -> None:
        self.repository.joinpath("CLAUDE.md").write_text(
            "# Policy\n\nFollow the orchid convention.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("README.md").write_text(
            "# Project\n\nThe topaz setup uses local services.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("docs/domain").mkdir(parents=True)
        self.repository.joinpath("docs/domain/rules.md").write_text(
            "# Domain Rules\n\nThe saffron approval rule is mandatory.\n",
            encoding="utf-8",
        )

        indexed = self.run_context("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)

        for term, expected_path in (
            ("orchid", "CLAUDE.md"),
            ("topaz", "README.md"),
            ("saffron", "docs/domain/rules.md"),
        ):
            with self.subTest(term=term):
                searched = self.run_context("search", term, "--json")
                self.assertEqual(0, searched.returncode, searched.stderr)
                self.assertEqual(
                    [expected_path],
                    [item["path"] for item in json.loads(searched.stdout)["documents"]],
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

    def test_index_skips_parseable_but_invalid_active_memory(self) -> None:
        self.repository.joinpath(
            "memory-bank/chunks/MEM-0003-invalid-active.md"
        ).write_text(
            '---\n{"status": "active"}\n---\n\n'
            "# Invalid Active Memory\n\nThe vermilion shortcut is unsafe.\n",
            encoding="utf-8",
        )

        indexed = self.run_context("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)

        searched = self.run_context("search", "vermilion", "--json")
        self.assertEqual(0, searched.returncode, searched.stderr)
        self.assertEqual([], json.loads(searched.stdout)["documents"])

    def test_index_skips_unhashable_type_and_status_metadata(self) -> None:
        self.write_memory(
            "MEM-0003-invalid-type.md",
            "active",
            "# Invalid Type\n\nThe vermilion shortcut is unsafe.",
        )
        invalid_type = self.repository / "memory-bank/chunks/MEM-0003-invalid-type.md"
        invalid_type.write_text(
            invalid_type.read_text(encoding="utf-8").replace(
                '"type": "convention"',
                '"type": []',
            ),
            encoding="utf-8",
        )
        self.write_memory(
            "MEM-0004-invalid-status.md",
            {"active": True},
            "# Invalid Status\n\nThe cerulean shortcut is unsafe.",
        )

        indexed = self.run_context("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)

        for term in ("vermilion", "cerulean"):
            with self.subTest(term=term):
                searched = self.run_context("search", term, "--json")
                self.assertEqual(0, searched.returncode, searched.stderr)
                self.assertEqual([], json.loads(searched.stdout)["documents"])

    def test_old_episode_schema_is_migrated_without_data_loss(self) -> None:
        database = self.create_old_episode_database()

        searched = self.run_context("search", "LegacyHandler", "--json")
        self.assertEqual(0, searched.returncode, searched.stderr)
        episodes = json.loads(searched.stdout)["episodes"]
        self.assertEqual([7], [episode["id"] for episode in episodes])

        connection = sqlite3.connect(database)
        schema = connection.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'episodes'"
        ).fetchone()[0]
        count = connection.execute("SELECT COUNT(*) FROM episodes").fetchone()[0]
        connection.close()
        self.assertNotIn("files UNINDEXED", schema)
        self.assertEqual(1, count)

    def test_old_document_schema_is_recreated_without_dropping_episodes(self) -> None:
        database = self.create_old_document_database()

        status = self.run_context("status", "--json")
        self.assertEqual(0, status.returncode, status.stderr)
        self.assertEqual(1, json.loads(status.stdout)["episodes"])

        connection = sqlite3.connect(database)
        columns = [
            row[1] for row in connection.execute("PRAGMA table_info(documents)").fetchall()
        ]
        connection.close()
        self.assertEqual(["path", "layer", "kind", "title", "content"], columns)

    def test_failed_episode_migration_rolls_back(self) -> None:
        database = self.create_old_episode_database()
        create_episode_table = CONTEXT.create_episode_table

        def fail_after_create(connection: sqlite3.Connection) -> None:
            create_episode_table(connection)
            raise sqlite3.OperationalError("simulated migration failure")

        with mock.patch.object(
            CONTEXT,
            "create_episode_table",
            side_effect=fail_after_create,
        ):
            with self.assertRaises(sqlite3.OperationalError):
                CONTEXT.connect(database)

        connection = sqlite3.connect(database)
        schema = connection.execute(
            "SELECT sql FROM sqlite_master WHERE name = 'episodes'"
        ).fetchone()[0]
        rowids = [
            row[0]
            for row in connection.execute("SELECT rowid FROM episodes").fetchall()
        ]
        connection.close()
        self.assertIn("files UNINDEXED", schema)
        self.assertEqual([7], rowids)

    def test_recorded_episode_is_searchable(self) -> None:
        recorded = self.run_context(
            "record",
            "--summary",
            "Added the crimson retry boundary.",
            "--outcome",
            "Deployment retries are idempotent.",
            "--file",
            "src/Deployment/CobaltHandler.php",
            "--verification",
            "RetryTest passed",
            "--source",
            "specs/indigo-source.md",
            "--json",
        )
        self.assertEqual(0, recorded.returncode, recorded.stderr)

        searched = self.run_context("search", "crimson", "--json")
        self.assertEqual(0, searched.returncode, searched.stderr)
        episodes = json.loads(searched.stdout)["episodes"]
        self.assertEqual(1, len(episodes))
        self.assertEqual("Added the crimson retry boundary.", episodes[0]["summary"])
        self.assertEqual(["src/Deployment/CobaltHandler.php"], episodes[0]["files"])
        self.assertEqual(["RetryTest passed"], episodes[0]["verification"])
        self.assertEqual(["specs/indigo-source.md"], episodes[0]["sources"])

        by_file = self.run_context("search", "CobaltHandler", "--json")
        self.assertEqual(0, by_file.returncode, by_file.stderr)
        self.assertEqual(1, len(json.loads(by_file.stdout)["episodes"]))

        by_source = self.run_context("search", "indigo", "--json")
        self.assertEqual(0, by_source.returncode, by_source.stderr)
        self.assertEqual(1, len(json.loads(by_source.stdout)["episodes"]))

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
        self.assertEqual(
            {"procedural": 0, "semantic": 1, "episodic": 0},
            payload["layers"],
        )
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

    def test_record_rejects_blank_optional_field(self) -> None:
        recorded = self.run_context(
            "record",
            "--summary",
            "Completed task.",
            "--outcome",
            "Task is complete.",
            "--file",
            " ",
            "--json",
        )

        self.assertNotEqual(0, recorded.returncode)
        self.assertIn("Episode file must not be empty", recorded.stderr)

    def test_nonexistent_root_is_rejected_without_creating_it(self) -> None:
        missing = self.repository / "missing"

        status = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(missing), "status", "--json"],
            text=True,
            capture_output=True,
        )

        self.assertNotEqual(0, status.returncode)
        self.assertIn("Repository root must be an existing directory", status.stderr)
        self.assertFalse(missing.exists())


if __name__ == "__main__":
    unittest.main()
