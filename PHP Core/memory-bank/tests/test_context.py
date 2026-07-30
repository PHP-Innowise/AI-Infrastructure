#!/usr/bin/env python3
"""Integration tests for the repository-local context engine."""

from __future__ import annotations

import importlib.util
import json
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest
from datetime import date, timedelta
from pathlib import Path
from typing import Callable
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


class PrefetchedCursor:
    def __init__(self, row: object) -> None:
        self.row = row

    def fetchone(self) -> object:
        return self.row


class SynchronizedConnection:
    def __init__(
        self,
        connection: sqlite3.Connection,
        select_prefix: str,
        barrier: threading.Barrier,
    ) -> None:
        self.connection = connection
        self.select_prefix = select_prefix
        self.barrier = barrier

    def __enter__(self) -> SynchronizedConnection:
        self.connection.__enter__()
        return self

    def __exit__(
        self, exception_type: object, exception: object, traceback: object
    ) -> object:
        return self.connection.__exit__(exception_type, exception, traceback)

    def execute(
        self, statement: str, parameters: tuple[object, ...] = ()
    ) -> object:
        cursor = self.connection.execute(statement, parameters)
        if " ".join(statement.split()).startswith(self.select_prefix):
            row = cursor.fetchone()
            try:
                self.barrier.wait(timeout=1)
            except threading.BrokenBarrierError:
                pass
            return PrefetchedCursor(row)
        return cursor

    def commit(self) -> None:
        self.connection.commit()

    def rollback(self) -> None:
        self.connection.rollback()


class ContextEngineTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="context-engine-test-")
        self.repository = Path(self.temporary.name)
        subprocess.run(
            ["git", "init", "--quiet", str(self.repository)],
            check=True,
            capture_output=True,
            text=True,
        )
        self.repository.joinpath("memory-bank/chunks").mkdir(parents=True)
        self.repository.joinpath("specs").mkdir()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_context(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--root",
                str(self.repository),
                "--mode",
                "lightweight",
                *arguments,
            ],
            text=True,
            capture_output=True,
        )

    def run_governed(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.repository), *arguments],
            text=True,
            capture_output=True,
        )

    def run_concurrently(
        self,
        select_prefix: str,
        operation: Callable[[SynchronizedConnection], object],
    ) -> list[object]:
        database = self.repository / "memory-bank/local/context.db"
        ready = threading.Barrier(2)
        selected = threading.Barrier(2)
        results: list[object] = [None, None]

        def worker(index: int) -> None:
            connection = CONTEXT.connect(database)
            try:
                ready.wait()
                synchronized = SynchronizedConnection(
                    connection, select_prefix, selected
                )
                try:
                    results[index] = operation(synchronized)
                except Exception as error:
                    results[index] = error
            finally:
                connection.close()

        threads = [threading.Thread(target=worker, args=(index,)) for index in range(2)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=10)
            self.assertFalse(thread.is_alive(), "concurrent operation did not finish")
        return results

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
        self.assertEqual(".agents/skills/review/SKILL.md", documents[0]["path"])

    def test_index_skips_skill_with_secret_without_persisting_it(self) -> None:
        secret = "ghp_" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"
        skill = self.repository / ".agents/skills/review/SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text(
            f"# Review\n\nCredential: {secret}\n",
            encoding="utf-8",
        )

        indexed = self.run_context("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)
        self.assertEqual(0, json.loads(indexed.stdout)["documents"])
        self.assertNotIn("ABCDEFGHIJKLMNOPQRSTUVWXYZ", indexed.stderr)

        connection = sqlite3.connect(self.repository / "memory-bank/local/context.db")
        documents = connection.execute("SELECT path, content FROM documents").fetchall()
        connection.close()
        self.assertEqual([], documents)

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

    def test_index_skips_git_ignored_sources_but_keeps_tracked_sources(self) -> None:
        subprocess.run(
            ["git", "init", "--quiet", str(self.repository)],
            check=True,
            capture_output=True,
            text=True,
        )
        self.repository.joinpath(".gitignore").write_text(
            "docs/*.md\n",
            encoding="utf-8",
        )
        self.repository.joinpath("docs").mkdir()
        self.repository.joinpath("docs/ignored.md").write_text(
            "# Ignored\n\nThe heliotrope rule must stay out of the index.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("docs/tracked.md").write_text(
            "# Tracked\n\nThe periwinkle rule remains searchable.\n",
            encoding="utf-8",
        )
        subprocess.run(
            ["git", "add", "-f", "--", "docs/tracked.md"],
            cwd=self.repository,
            check=True,
            capture_output=True,
            text=True,
        )

        indexed = self.run_context("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)

        ignored = self.run_context("search", "heliotrope", "--json")
        self.assertEqual([], json.loads(ignored.stdout)["documents"])

        tracked = self.run_context("search", "periwinkle", "--json")
        self.assertEqual(
            ["docs/tracked.md"],
            [item["path"] for item in json.loads(tracked.stdout)["documents"]],
        )

    def test_index_does_not_read_git_ignored_sources(self) -> None:
        subprocess.run(
            ["git", "init", "--quiet", str(self.repository)],
            check=True,
            capture_output=True,
            text=True,
        )
        self.repository.joinpath(".gitignore").write_text(
            "docs/*.md\n",
            encoding="utf-8",
        )
        self.repository.joinpath("docs").mkdir()
        self.repository.joinpath("docs/:(glob) malformed\nsource.md").write_bytes(
            b"\xff"
        )

        indexed = self.run_context("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)
        self.assertEqual(0, json.loads(indexed.stdout)["documents"])

    def test_git_ignore_probe_fails_closed_on_unexpected_exit(self) -> None:
        failed = subprocess.CompletedProcess([], 2, stdout=b"", stderr=b"")

        with mock.patch.object(CONTEXT.subprocess, "run", return_value=failed):
            with self.assertRaisesRegex(
                CONTEXT.ContextError,
                "Git ignore probe failed with exit status 2",
            ):
                CONTEXT.git_ignored_paths(self.repository, ["docs/private.md"])

    def test_git_ignore_probe_fails_closed_when_git_is_unavailable(self) -> None:
        error = OSError("private probe detail")

        with mock.patch.object(CONTEXT.subprocess, "run", side_effect=error):
            with self.assertRaisesRegex(
                CONTEXT.ContextError,
                "^Git ignore probe failed$",
            ) as raised:
                CONTEXT.git_ignored_paths(self.repository, ["docs/private.md"])

        self.assertNotIn("private probe detail", str(raised.exception))

    def test_index_reports_invalid_utf8_and_preserves_previous_index(self) -> None:
        self.repository.joinpath("README.md").write_text(
            "# Project\n\nThe celadon rule remains searchable.\n",
            encoding="utf-8",
        )
        initial = self.run_context("index", "--json")
        self.assertEqual(0, initial.returncode, initial.stderr)

        self.repository.joinpath("docs").mkdir()
        self.repository.joinpath("docs/broken.md").write_bytes(b"\xff")
        failed = self.run_context("index", "--json")

        self.assertNotEqual(0, failed.returncode)
        self.assertEqual(
            "context: Source document is not valid UTF-8: docs/broken.md\n",
            failed.stderr,
        )
        self.assertNotIn("Traceback", failed.stderr)

        searched = self.run_context("search", "celadon", "--json")
        self.assertEqual(
            ["README.md"],
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

    def test_working_lifecycle_isolated_by_task_id(self) -> None:
        first = self.run_context(
            "start",
            "--task-id",
            "BAUMAS-133",
            "--goal",
            "Invalidate other password sessions.",
            "--file",
            "src/GraphQL/Resolver/ChangePasswordResolver.php",
            "--json",
        )
        second = self.run_context(
            "start",
            "--task-id",
            "BAUMAS-134",
            "--goal",
            "Add an independent audit check.",
            "--json",
        )
        self.assertEqual(0, first.returncode, first.stderr)
        self.assertEqual(0, second.returncode, second.stderr)

        updated = self.run_context(
            "update",
            "--task-id",
            "BAUMAS-133",
            "--progress",
            "Two-session regression passes.",
            "--next-step",
            "Verify remember-me invalidation.",
            "--file",
            "tests/Integration/GraphQL/ChangePasswordTest.php",
            "--json",
        )
        self.assertEqual(0, updated.returncode, updated.stderr)

        task = self.run_context("get", "--task-id", "BAUMAS-133", "--json")
        self.assertEqual(0, task.returncode, task.stderr)
        self.assertEqual(
            {
                "task_id": "BAUMAS-133",
                "goal": "Invalidate other password sessions.",
                "progress": "Two-session regression passes.",
                "next_steps": ["Verify remember-me invalidation."],
                "files": [
                    "src/GraphQL/Resolver/ChangePasswordResolver.php",
                    "tests/Integration/GraphQL/ChangePasswordTest.php",
                ],
                "sources": [],
            },
            {
                key: value
                for key, value in json.loads(task.stdout).items()
                if key not in {"created_at", "updated_at"}
            },
        )

        status = json.loads(self.run_context("status", "--json").stdout)
        self.assertEqual(2, status["working"])

        cleared = self.run_context("clear", "--task-id", "BAUMAS-133", "--json")
        self.assertEqual(0, cleared.returncode, cleared.stderr)
        status = json.loads(self.run_context("status", "--json").stdout)
        self.assertEqual(1, status["working"])
        remaining = self.run_context("get", "--task-id", "BAUMAS-134", "--json")
        self.assertEqual(0, remaining.returncode, remaining.stderr)

    def test_working_start_preserves_exact_file_path(self) -> None:
        result = self.run_context(
            "start",
            "--task-id",
            "TASK-WHITESPACE",
            "--goal",
            "Preserve exact Git paths.",
            "--file",
            " leading-and-trailing.txt ",
            "--json",
        )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(
            [" leading-and-trailing.txt "],
            json.loads(result.stdout)["files"],
        )

    def test_working_update_merges_unique_list_values(self) -> None:
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "TASK-1",
                "--goal",
                "Keep working state.",
                "--file",
                "src/Task.php",
                "--source",
                "specs/task.md",
            ).returncode,
        )

        updated = self.run_context(
            "update",
            "--task-id",
            "TASK-1",
            "--next-step",
            "Run the regression.",
            "--next-step",
            "Run the regression.",
            "--file",
            "src/Task.php",
            "--file",
            "tests/TaskTest.php",
            "--source",
            "specs/task.md",
            "--source",
            "docs/task.md",
            "--json",
        )
        self.assertEqual(0, updated.returncode, updated.stderr)
        task = json.loads(updated.stdout)
        self.assertEqual(["Run the regression."], task["next_steps"])
        self.assertEqual(["src/Task.php", "tests/TaskTest.php"], task["files"])
        self.assertEqual(["specs/task.md", "docs/task.md"], task["sources"])

    def test_concurrent_working_updates_preserve_both_additions(self) -> None:
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "TASK-1",
                "--goal",
                "Keep concurrent updates.",
            ).returncode,
        )

        additions = iter(("First synchronized step.", "Second synchronized step."))
        additions_lock = threading.Lock()

        def update(connection: SynchronizedConnection) -> object:
            with additions_lock:
                addition = next(additions)
            return CONTEXT.update_working_task(
                connection, "TASK-1", None, [addition], [], []
            )

        results = self.run_concurrently(
            "SELECT progress, next_steps, files, sources FROM working_tasks",
            update,
        )
        self.assertTrue(
            all(isinstance(result, dict) for result in results), results
        )

        task = json.loads(
            self.run_context("get", "--task-id", "TASK-1", "--json").stdout
        )
        self.assertEqual(
            ["First synchronized step.", "Second synchronized step."],
            sorted(task["next_steps"]),
        )

    def test_concurrent_duplicate_starts_return_clean_already_exists_error(self) -> None:
        def start(connection: SynchronizedConnection) -> object:
            return CONTEXT.start_working_task(
                connection, "TASK-1", "Start once.", [], []
            )

        results = self.run_concurrently(
            "SELECT 1 FROM working_tasks WHERE task_id =",
            start,
        )
        successes = [result for result in results if isinstance(result, dict)]
        errors = [result for result in results if isinstance(result, Exception)]
        self.assertEqual(1, len(successes), results)
        self.assertEqual(1, len(errors), results)
        self.assertIsInstance(errors[0], CONTEXT.ContextError)
        self.assertIn("already exists", str(errors[0]))
        self.assertFalse(any(isinstance(error, sqlite3.Error) for error in errors))

    def test_complete_moves_working_task_to_searchable_episode(self) -> None:
        started = self.run_context(
            "start",
            "--task-id",
            "BAUMAS-133",
            "--goal",
            "BAUMAS-133: Invalidate other password sessions.",
            "--file",
            "src/GraphQL/Resolver/ChangePasswordResolver.php",
            "--source",
            "specs/passwords.md",
            "--json",
        )
        self.assertEqual(0, started.returncode, started.stderr)

        completed = self.run_context(
            "complete",
            "--task-id",
            "BAUMAS-133",
            "--outcome",
            "Other sessions are invalidated.",
            "--file",
            "tests/Integration/GraphQL/ChangePasswordTest.php",
            "--file",
            "src/GraphQL/Resolver/ChangePasswordResolver.php",
            "--file",
            " src/GraphQL/Resolver/ChangePasswordResolver.php ",
            "--verification",
            "ChangePasswordTest passed",
            "--source",
            "docs/passwords.md",
            "--source",
            "specs/passwords.md",
            "--source",
            " specs/passwords.md ",
            "--json",
        )
        self.assertEqual(0, completed.returncode, completed.stderr)

        status = json.loads(self.run_context("status", "--json").stdout)
        self.assertEqual(0, status["working"])
        self.assertEqual(1, status["episodes"])

        episode = json.loads(
            self.run_context("search", "BAUMAS-133", "--json").stdout
        )["episodes"][0]
        self.assertEqual(
            "BAUMAS-133: Invalidate other password sessions.", episode["summary"]
        )
        self.assertEqual(
            [
                "src/GraphQL/Resolver/ChangePasswordResolver.php",
                "tests/Integration/GraphQL/ChangePasswordTest.php",
                " src/GraphQL/Resolver/ChangePasswordResolver.php ",
            ],
            episode["files"],
        )
        self.assertEqual(["ChangePasswordTest passed"], episode["verification"])
        self.assertEqual(
            ["specs/passwords.md", "docs/passwords.md"], episode["sources"]
        )

    def test_complete_rolls_back_episode_when_working_delete_fails(self) -> None:
        started = self.run_context(
            "start",
            "--task-id",
            "BAUMAS-133",
            "--goal",
            "BAUMAS-133: Invalidate other password sessions.",
            "--json",
        )
        self.assertEqual(0, started.returncode, started.stderr)

        database = self.repository / "memory-bank/local/context.db"
        connection = sqlite3.connect(database)
        connection.executescript(
            """
            CREATE TRIGGER fail_working_delete
            BEFORE DELETE ON working_tasks
            BEGIN
                SELECT RAISE(ABORT, 'simulated complete failure');
            END;
            """
        )
        connection.close()

        completed = self.run_context(
            "complete",
            "--task-id",
            "BAUMAS-133",
            "--outcome",
            "Other sessions are invalidated.",
            "--json",
        )
        self.assertNotEqual(0, completed.returncode)
        self.assertIn("simulated complete failure", completed.stderr)

        status = json.loads(self.run_context("status", "--json").stdout)
        self.assertEqual(1, status["working"])
        self.assertEqual(0, status["episodes"])

    def test_working_rejects_duplicate_unknown_and_invalid_tasks(self) -> None:
        self.assertEqual(
            0,
            self.run_context(
                "start", "--task-id", "TASK-1", "--goal", "First task"
            ).returncode,
        )
        duplicate = self.run_context(
            "start", "--task-id", "TASK-1", "--goal", "Duplicate task"
        )
        unknown = self.run_context(
            "update", "--task-id", "TASK-404", "--progress", "Missing"
        )
        unknown_clear = self.run_context("clear", "--task-id", "TASK-404")
        invalid = self.run_context(
            "start", "--task-id", "bad task id", "--goal", "Invalid"
        )
        empty_update = self.run_context("update", "--task-id", "TASK-1")

        self.assertIn("already exists", duplicate.stderr)
        self.assertIn("not found", unknown.stderr)
        self.assertIn("not found", unknown_clear.stderr)
        self.assertIn("Task ID must use", invalid.stderr)
        self.assertIn("requires a changed field", empty_update.stderr)

    def test_working_rejects_secret_without_echoing_it(self) -> None:
        fake_token = "ghp_" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"
        result = self.run_context(
            "start",
            "--task-id",
            "TASK-SECRET",
            "--goal",
            f"Rotate {fake_token}",
        )

        self.assertNotEqual(0, result.returncode)
        self.assertIn("possible GitHub token", result.stderr)
        self.assertNotIn("ABCDEFGHIJKLMNOPQRSTUVWXYZ", result.stderr)

    def test_working_rejects_private_or_raw_data_before_persisting_it(self) -> None:
        private = "Customer email: person@example.test"
        rejected_start = self.run_context(
            "start",
            "--task-id",
            "TASK-PRIVATE-START",
            "--goal",
            private,
        )

        self.assertNotEqual(0, rejected_start.returncode)
        self.assertIn(
            "Working task contains private or raw data; "
            "replace it with a sanitized summary",
            rejected_start.stderr,
        )
        self.assertNotIn(private, rejected_start.stdout + rejected_start.stderr)
        self.assertEqual(
            0,
            json.loads(self.run_context("status", "--json").stdout)["working"],
        )

        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "TASK-PRIVATE-UPDATE",
                "--goal",
                "Review account behavior.",
            ).returncode,
        )
        raw_handoff = "User: copied request\nAssistant: copied response"
        rejected_update = self.run_context(
            "update",
            "--task-id",
            "TASK-PRIVATE-UPDATE",
            "--progress",
            raw_handoff,
        )

        self.assertNotEqual(0, rejected_update.returncode)
        self.assertNotIn(
            raw_handoff,
            rejected_update.stdout + rejected_update.stderr,
        )
        persisted = self.run_context(
            "get",
            "--task-id",
            "TASK-PRIVATE-UPDATE",
            "--json",
        )
        self.assertEqual("", json.loads(persisted.stdout)["progress"])

    def test_working_rejects_raw_file_and_source_identifiers_before_persisting(
        self,
    ) -> None:
        raw_source = "User: copied request body"
        rejected_start = self.run_context(
            "start",
            "--task-id",
            "TASK-RAW-IDENTIFIER-START",
            "--goal",
            "Review account behavior.",
            "--source",
            raw_source,
        )

        self.assertNotEqual(0, rejected_start.returncode)
        self.assertNotIn(raw_source, rejected_start.stdout + rejected_start.stderr)
        self.assertEqual(
            0,
            json.loads(self.run_context("status", "--json").stdout)["working"],
        )

        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "TASK-RAW-IDENTIFIER-UPDATE",
                "--goal",
                "Review account behavior.",
            ).returncode,
        )
        raw_file = "Assistant: copied response"
        rejected_update = self.run_context(
            "update",
            "--task-id",
            "TASK-RAW-IDENTIFIER-UPDATE",
            "--file",
            raw_file,
        )

        self.assertNotEqual(0, rejected_update.returncode)
        self.assertNotIn(raw_file, rejected_update.stdout + rejected_update.stderr)
        persisted = self.run_context(
            "get",
            "--task-id",
            "TASK-RAW-IDENTIFIER-UPDATE",
            "--json",
        )
        self.assertEqual([], json.loads(persisted.stdout)["files"])

    def test_context_revalidates_legacy_working_raw_identifier(self) -> None:
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "TASK-RAW-IDENTIFIER-LEGACY",
                "--goal",
                "Review account behavior.",
            ).returncode,
        )
        raw_source = "User: copied request body"
        database = self.repository / "memory-bank/local/context.db"
        connection = sqlite3.connect(database)
        connection.execute(
            "UPDATE working_tasks SET sources = ? WHERE task_id = ?",
            (json.dumps([raw_source]), "TASK-RAW-IDENTIFIER-LEGACY"),
        )
        connection.commit()
        connection.close()

        packet = self.run_context(
            "context",
            "account workflow",
            "--task-id",
            "TASK-RAW-IDENTIFIER-LEGACY",
            "--json",
        )

        self.assertNotEqual(0, packet.returncode)
        self.assertIn(
            "Working task contains private or raw data; "
            "replace it with a sanitized summary",
            packet.stderr,
        )
        self.assertNotIn(raw_source, packet.stdout + packet.stderr)

    def test_working_get_and_clear_reject_secret_task_id_without_echoing_it(self) -> None:
        fake_token = "ghp_" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"

        for command in ("get", "clear"):
            with self.subTest(command=command):
                result = self.run_context(command, "--task-id", fake_token)
                self.assertNotEqual(0, result.returncode)
                self.assertIn("possible GitHub token", result.stderr)
                self.assertNotIn("ABCDEFGHIJKLMNOPQRSTUVWXYZ", result.stderr)

    def test_context_packet_retrieves_each_layer_for_working_task(self) -> None:
        self.repository.joinpath("AGENTS.md").write_text(
            "# Procedure\n\nPassword session invalidation requires a review.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("README.md").write_text(
            "# Architecture\n\nPassword session invalidation protects accounts.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changes\n\nPassword session invalidation was released.\n",
            encoding="utf-8",
        )
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "BAUMAS-133",
                "--goal",
                "Invalidate password sessions.",
            ).returncode,
        )
        self.assertEqual(
            0,
            self.run_context(
                "record",
                "--summary",
                "Password session invalidation completed.",
                "--outcome",
                "Other sessions are invalidated.",
            ).returncode,
        )
        self.assertEqual(0, self.run_context("index", "--json").returncode)

        packet = self.run_context(
            "context",
            "password session invalidation",
            "--task-id",
            "BAUMAS-133",
            "--limit",
            "2",
            "--json",
        )
        self.assertEqual(0, packet.returncode, packet.stderr)
        payload = json.loads(packet.stdout)
        self.assertEqual("BAUMAS-133", payload["working"]["task_id"])
        self.assertLessEqual(len(payload["procedural"]), 2)
        self.assertLessEqual(len(payload["semantic"]), 2)
        self.assertEqual(1, len(payload["episodic"]))
        self.assertTrue(
            all(item["layer"] == "procedural" for item in payload["procedural"])
        )
        self.assertTrue(
            all(item["layer"] == "semantic" for item in payload["semantic"])
        )
        self.assertTrue(
            all(item["layer"] == "episodic" for item in payload["episodic"])
        )
        self.assertEqual(["AGENTS.md"], [item["path"] for item in payload["procedural"]])
        self.assertEqual(["README.md"], [item["path"] for item in payload["semantic"]])
        self.assertEqual("CHANGELOG.md", payload["episodic"][0]["path"])

        unknown = self.run_context(
            "context", "password", "--task-id", "TASK-404", "--json"
        )
        invalid_limit = self.run_context(
            "context",
            "password",
            "--task-id",
            "BAUMAS-133",
            "--limit",
            "0",
            "--json",
        )
        self.assertEqual(0, unknown.returncode, unknown.stderr)
        unknown_payload = json.loads(unknown.stdout)
        self.assertIsNone(unknown_payload["working"])
        self.assertIn("Working task not found: TASK-404", unknown_payload["warnings"])
        self.assertIn("--limit must be a positive integer", invalid_limit.stderr)

    def test_context_builds_request_only_capsule_with_layer_limits(self) -> None:
        procedural_sources = {
            "AGENTS.md": "# Policy\n\nThe capsule boundary is mandatory.\n",
            "CLAUDE.md": "# Claude\n\nThe capsule boundary guides work.\n",
            ".agents/skills/alpha/SKILL.md": (
                "# Alpha\n\nThe capsule boundary applies to alpha.\n"
            ),
        }
        for relative_path, content in procedural_sources.items():
            path = self.repository / relative_path
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")

        semantic_sources = {
            "README.md": (
                "# Project\n\nThe capsule boundary describes the project.\n\n"
                + ("Background material without the search terms. " * 80)
                + "\nFULL_DOCUMENT_SENTINEL\n"
            ),
            "specs/one.md": "# One\n\nThe capsule boundary protects one.\n",
            "specs/two.md": "# Two\n\nThe capsule boundary protects two.\n",
            "specs/three.md": "# Three\n\nThe capsule boundary protects three.\n",
        }
        for relative_path, content in semantic_sources.items():
            (self.repository / relative_path).write_text(content, encoding="utf-8")

        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changes\n\nThe capsule boundary shipped.\n",
            encoding="utf-8",
        )
        self.assertEqual(0, self.run_context("index", "--json").returncode)

        packet = self.run_context(
            "context",
            "capsule boundary",
            "--limit",
            "20",
            "--json",
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        payload = json.loads(packet.stdout)
        self.assertIsNone(payload["task_id"])
        self.assertIsNone(payload["working"])
        self.assertLessEqual(len(payload["procedural"]), 2)
        self.assertLessEqual(len(payload["semantic"]), 3)
        self.assertLessEqual(len(payload["episodic"]), 1)
        self.assertIn("Working task unavailable: task ID was not supplied", payload["warnings"])
        self.assertNotIn("FULL_DOCUMENT_SENTINEL", packet.stdout)

    def test_context_uses_working_terms_and_deduplicates_paths(self) -> None:
        self.repository.joinpath("README.md").write_text(
            "# Cobalt\n\nThe cobalt invariant belongs to the account workflow.\n",
            encoding="utf-8",
        )
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "BAUMAS-133",
                "--goal",
                "Verify the cobalt invariant.",
            ).returncode,
        )
        self.assertEqual(0, self.run_context("index", "--json").returncode)

        packet = self.run_context(
            "context",
            "current phase",
            "--task-id",
            "BAUMAS-133",
            "--json",
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        payload = json.loads(packet.stdout)
        self.assertEqual("BAUMAS-133", payload["task_id"])
        self.assertEqual("BAUMAS-133", payload["working"]["task_id"])
        self.assertEqual(
            ["README.md"],
            [item["path"] for item in payload["semantic"]],
        )
        document = {
            "path": "README.md",
            "layer": "semantic",
            "kind": "overview",
            "title": "Project",
            "snippet": "Cobalt invariant.",
        }
        episode = {
            "id": 7,
            "layer": "episodic",
            "summary": "Prior work",
        }
        self.assertEqual(
            [document, episode],
            CONTEXT.deduplicate_context_items(
                [document, dict(document), episode, dict(episode)]
            ),
        )

    def test_context_projects_only_normalized_bounded_working_state(self) -> None:
        files = ["src/Feature/  Odd Name .php"] + [
            f"src/Feature/File{index}.php" for index in range(1, 10)
        ]
        sources = ["specs/Source  Zero.md"] + [
            f"specs/source-{index}.md" for index in range(1, 6)
        ]
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "CAPSULE-PROJECTION",
                "--goal",
                "  Review   account\nworkflow.  ",
                *[
                    argument
                    for file_path in files
                    for argument in ("--file", file_path)
                ],
                *[
                    argument
                    for source in sources
                    for argument in ("--source", source)
                ],
            ).returncode,
        )
        self.assertEqual(
            0,
            self.run_context(
                "update",
                "--task-id",
                "CAPSULE-PROJECTION",
                "--progress",
                "Earlier outcome.\nLatest outcome.",
                "--next-step",
                "Inspect the implementation.",
                "--next-step",
                "Run focused tests.",
                "--next-step",
                "Run final verification.",
            ).returncode,
        )

        packet = self.run_context(
            "context",
            "  Review\nbranding?! safely, now.  ",
            "--task-id",
            "CAPSULE-PROJECTION",
            "--json",
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        payload = json.loads(packet.stdout)
        self.assertEqual("Review branding safely now", payload["query"])
        self.assertEqual(
            {
                "task_id": "CAPSULE-PROJECTION",
                "goal": "Review account workflow.",
                "progress": "Earlier outcome. Latest outcome.",
                "next_steps": ["Run final verification."],
                "files": files[:8],
                "sources": sources[:4],
            },
            payload["working"],
        )
        self.assertEqual(
            {
                "working_files": 2,
                "working_next_steps": 2,
                "working_sources": 2,
                "working_progress_characters": 0,
            },
            payload["omitted"],
        )

    def test_context_revalidates_legacy_working_privacy_data(self) -> None:
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "CAPSULE-PRIVACY",
                "--goal",
                "Review the account workflow.",
            ).returncode,
        )
        database = self.repository / "memory-bank/local/context.db"
        unsafe_values = (
            "Customer email: person@example.test",
            "Customer phone: +370 600 12345",
            "User: copied request\nAssistant: copied response",
            "Customer name: Example Person",
        )

        for unsafe in unsafe_values:
            with self.subTest(unsafe=unsafe):
                connection = sqlite3.connect(database)
                connection.execute(
                    "UPDATE working_tasks SET progress = ? WHERE task_id = ?",
                    (unsafe, "CAPSULE-PRIVACY"),
                )
                connection.commit()
                connection.close()

                packet = self.run_context(
                    "context",
                    "account workflow",
                    "--task-id",
                    "CAPSULE-PRIVACY",
                    "--json",
                )

                self.assertNotEqual(0, packet.returncode)
                self.assertIn(
                    "Working task contains private or raw data; "
                    "replace it with a sanitized summary",
                    packet.stderr,
                )
                self.assertNotIn(unsafe, packet.stdout + packet.stderr)

    def test_get_revalidates_legacy_working_privacy_data_but_clear_recovers(
        self,
    ) -> None:
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "CAPSULE-LEGACY",
                "--goal",
                "Review account behavior.",
            ).returncode,
        )
        private = "Customer email: person@example.test"
        database = self.repository / "memory-bank/local/context.db"
        connection = sqlite3.connect(database)
        connection.execute(
            "UPDATE working_tasks SET progress = ? WHERE task_id = ?",
            (private, "CAPSULE-LEGACY"),
        )
        connection.commit()
        connection.close()

        retrieved = self.run_context(
            "get",
            "--task-id",
            "CAPSULE-LEGACY",
            "--json",
        )

        self.assertNotEqual(0, retrieved.returncode)
        self.assertNotIn(private, retrieved.stdout + retrieved.stderr)
        self.assertEqual(
            0,
            self.run_context(
                "clear",
                "--task-id",
                "CAPSULE-LEGACY",
            ).returncode,
        )

    def test_context_retrieves_request_matches_before_working_enrichment(
        self,
    ) -> None:
        working_terms = (
            "account workflow tenant process update review general architecture "
            "implementation verification feature change state handler service "
            "model controller database policy"
        )
        self.repository.joinpath("specs/request.md").write_text(
            "# Branding\n\nzirconbranding authoritative behavior.\n"
            + ("background " * 500),
            encoding="utf-8",
        )
        for index in range(3):
            self.repository.joinpath(f"specs/generic-{index}.md").write_text(
                f"# Generic {index}\n\n{working_terms}\n",
                encoding="utf-8",
            )
        for index in range(5):
            self.repository.joinpath(f"specs/noise-{index}.md").write_text(
                f"# Noise {index}\n\nUnrelated filler {index}.\n",
                encoding="utf-8",
            )
        self.assertEqual(
            0,
            self.run_context(
                "start",
                "--task-id",
                "CAPSULE-REQUEST",
                "--goal",
                working_terms,
            ).returncode,
        )

        packet = self.run_context(
            "context",
            "zirconbranding",
            "--task-id",
            "CAPSULE-REQUEST",
            "--json",
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        self.assertIn(
            "specs/request.md",
            [item["path"] for item in json.loads(packet.stdout)["semantic"]],
        )

    def test_context_capsule_stays_within_character_budget(self) -> None:
        self.repository.joinpath("README.md").write_text(
            "# Project\n\nCapsule budget knowledge.\n", encoding="utf-8"
        )
        long_progress = "verified progress " * 700
        long_files = [f"src/Feature/File{index:03d}.php" for index in range(200)]
        self.assertEqual(
            0,
            self.run_context(
                "start", "--task-id", "CAPSULE-1", "--goal",
                "Keep the capsule budget bounded.",
                *[argument for file_path in long_files for argument in ("--file", file_path)],
            ).returncode,
        )
        self.assertEqual(0, self.run_context(
            "update", "--task-id", "CAPSULE-1", "--progress", long_progress
        ).returncode)

        packet = self.run_context(
            "context", "capsule budget", "--task-id", "CAPSULE-1", "--json"
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        self.assertLessEqual(
            len(packet.stdout.rstrip("\n")), CONTEXT.CAPSULE_CHARACTER_LIMIT
        )
        payload = json.loads(packet.stdout)
        self.assertGreater(payload["omitted"]["working_files"], 0)
        self.assertEqual("Keep the capsule budget bounded.", payload["working"]["goal"])

    def test_capsule_budget_drops_optional_layers_before_working(self) -> None:
        capsule = {
            "query": "capsule", "task_id": "CAPSULE-2",
            "working": {
                "task_id": "CAPSULE-2", "goal": "Preserve the goal.",
                "progress": "p" * 300, "next_steps": ["Run verification."],
                "files": ["src/Required.php"], "sources": [],
                "created_at": "2026-07-29T00:00:00+00:00",
                "updated_at": "2026-07-29T00:00:00+00:00",
            },
            "procedural": [{
                "path": "AGENTS.md", "layer": "procedural", "kind": "policy",
                "title": "Policy", "snippet": "Mandatory capsule policy.",
            }],
            "semantic": [{
                "path": f"specs/{index}.md", "layer": "semantic", "kind": "spec",
                "title": str(index), "snippet": "s" * 250,
            } for index in range(3)],
            "episodic": [{
                "id": 1, "layer": "episodic", "summary": "Prior work",
                "outcome": "e" * 300, "files": [], "verification": ["Verified"],
                "sources": [], "created_at": "2026-07-29T00:00:00+00:00",
            }],
            "warnings": [], "omitted": {"working_files": 0},
        }

        with mock.patch.object(CONTEXT, "CAPSULE_CHARACTER_LIMIT", 1250):
            compacted = CONTEXT.enforce_capsule_budget(capsule)

        self.assertEqual([], compacted["episodic"])
        self.assertEqual("Preserve the goal.", compacted["working"]["goal"])
        self.assertEqual("AGENTS.md", compacted["procedural"][0]["path"])
        self.assertLessEqual(CONTEXT.capsule_character_count(compacted), 1250)

    def test_capsule_budget_preserves_latest_progress_suffix(self) -> None:
        capsule = {
            "query": "capsule",
            "task_id": "CAPSULE-LATEST",
            "working": {
                "task_id": "CAPSULE-LATEST",
                "goal": "Preserve the latest outcome.",
                "progress": ("old progress " * 200) + "LATEST_OUTCOME",
                "next_steps": ["Run verification."],
                "files": ["src/Required.php"],
                "sources": ["specs/required.md"],
            },
            "procedural": [],
            "semantic": [],
            "episodic": [],
            "warnings": [],
            "omitted": {
                "working_files": 0,
                "working_next_steps": 0,
                "working_sources": 0,
                "working_progress_characters": 0,
            },
        }

        with mock.patch.object(CONTEXT, "CAPSULE_CHARACTER_LIMIT", 500):
            compacted = CONTEXT.enforce_capsule_budget(capsule)

        self.assertTrue(compacted["working"]["progress"].startswith("…"))
        self.assertTrue(compacted["working"]["progress"].endswith("LATEST_OUTCOME"))
        self.assertGreater(compacted["omitted"]["working_progress_characters"], 0)
        self.assertLessEqual(CONTEXT.capsule_character_count(compacted), 500)

    def test_capsule_budget_drops_nonpriority_working_sources(self) -> None:
        capsule = {
            "query": "capsule",
            "task_id": "CAPSULE-SOURCES",
            "working": {
                "task_id": "CAPSULE-SOURCES",
                "goal": "Preserve the priority source.",
                "progress": "Latest outcome.",
                "next_steps": ["Run verification."],
                "files": ["src/Required.php"],
                "sources": ["specs/required.md", "specs/" + ("x" * 700) + ".md"],
            },
            "procedural": [],
            "semantic": [],
            "episodic": [],
            "warnings": [],
            "omitted": {
                "working_files": 0,
                "working_next_steps": 0,
                "working_sources": 0,
                "working_progress_characters": 0,
            },
        }

        with mock.patch.object(CONTEXT, "CAPSULE_CHARACTER_LIMIT", 500):
            compacted = CONTEXT.enforce_capsule_budget(capsule)

        self.assertEqual(["specs/required.md"], compacted["working"]["sources"])
        self.assertEqual(1, compacted["omitted"]["working_sources"])
        self.assertEqual("Latest outcome.", compacted["working"]["progress"])
        self.assertLessEqual(CONTEXT.capsule_character_count(compacted), 500)

    def test_capsule_rejects_mandatory_content_larger_than_budget(self) -> None:
        capsule = {
            "query": "capsule", "task_id": None, "working": None,
            "procedural": [{
                "path": "AGENTS.md", "layer": "procedural", "kind": "policy",
                "title": "Policy", "snippet": "mandatory " * 100,
            }],
            "semantic": [], "episodic": [], "warnings": [],
            "omitted": {"working_files": 0},
        }

        with mock.patch.object(CONTEXT, "CAPSULE_CHARACTER_LIMIT", 200):
            with self.assertRaisesRegex(
                CONTEXT.ContextError,
                "mandatory Task Capsule content exceeds 200 characters",
            ):
                CONTEXT.enforce_capsule_budget(capsule)

    def test_context_index_failure_returns_working_only_without_stale_sources(
        self,
    ) -> None:
        self.repository.joinpath("README.md").write_text(
            "# Project\n\nThe celadon source was previously indexed.\n", encoding="utf-8"
        )
        self.assertEqual(0, self.run_context("index", "--json").returncode)
        self.assertEqual(0, self.run_context(
            "start", "--task-id", "CAPSULE-3", "--goal", "Inspect celadon behavior."
        ).returncode)
        self.repository.joinpath("docs").mkdir(exist_ok=True)
        self.repository.joinpath("docs/broken.md").write_bytes(b"\xff")

        packet = self.run_context(
            "context", "celadon", "--task-id", "CAPSULE-3", "--json"
        )

        self.assertEqual(0, packet.returncode, packet.stderr)
        payload = json.loads(packet.stdout)
        self.assertEqual("CAPSULE-3", payload["working"]["task_id"])
        self.assertEqual([], payload["procedural"])
        self.assertEqual([], payload["semantic"])
        self.assertEqual([], payload["episodic"])
        self.assertIn(
            "Index refresh failed: Source document is not valid UTF-8: docs/broken.md",
            payload["warnings"],
        )
        self.assertNotIn("celadon source", packet.stdout)
        self.assertNotIn("Traceback", packet.stderr)

    def test_context_rejects_secret_query_without_echoing_it(self) -> None:
        secret = "ghp_" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"

        packet = self.run_context("context", secret, "--json")

        self.assertNotEqual(0, packet.returncode)
        self.assertIn("possible GitHub token", packet.stderr)
        self.assertNotIn("ABCDEFGHIJKLMNOPQRSTUVWXYZ", packet.stderr)

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

    def test_search_layer_filters_local_episodes(self) -> None:
        self.repository.joinpath("AGENTS.md").write_text(
            "# Procedure\n\nUse the marigold context rule.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("README.md").write_text(
            "# Domain\n\nThe marigold context rule is documented.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changes\n\nThe marigold context rule shipped.\n",
            encoding="utf-8",
        )
        self.assertEqual(0, self.run_context("index", "--json").returncode)
        self.assertEqual(
            0,
            self.run_context(
                "record",
                "--summary",
                "Marigold context work.",
                "--outcome",
                "The marigold context rule is verified.",
            ).returncode,
        )

        for layer, path in (
            ("procedural", "AGENTS.md"),
            ("semantic", "README.md"),
            ("episodic", "CHANGELOG.md"),
        ):
            with self.subTest(layer=layer):
                payload = json.loads(
                    self.run_context(
                        "search", "marigold", "--layer", layer, "--json"
                    ).stdout
                )
                self.assertEqual([path], [item["path"] for item in payload["documents"]])
                self.assertEqual(
                    1 if layer == "episodic" else 0,
                    len(payload["episodes"]),
                )

        unfiltered = json.loads(
            self.run_context("search", "marigold", "--json").stdout
        )
        self.assertEqual(1, len(unfiltered["episodes"]))

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

    def test_record_rejects_empty_optional_file(self) -> None:
        recorded = self.run_context(
            "record",
            "--summary",
            "Completed task.",
            "--outcome",
            "Task is complete.",
            "--file",
            "",
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
