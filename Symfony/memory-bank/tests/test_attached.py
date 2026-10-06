#!/usr/bin/env python3
"""Attached layout: the accelerator stays in its clone, the project gets nothing.

The runtime under test is this edition's own: in attached mode it indexes the
edition's policy and skills in place, reads the project's documents and Git,
and keeps every piece of accelerator state in a separate directory. These
tests prove the three roots stay apart and that the installed layout - every
other test in this suite - is untouched by the variables.
"""

from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


EDITION = Path(__file__).resolve().parents[2]
SCRIPTS = EDITION / "memory-bank" / "scripts"
CONTEXT_SCRIPT = SCRIPTS / "context.py"
sys.path.insert(0, str(SCRIPTS))
try:
    import brain_runtime as brain
    import workspace_roots
finally:
    sys.path.pop(0)


def git(directory: Path, *arguments: str) -> str:
    return subprocess.run(
        ["git", "-C", str(directory), *arguments],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


class AttachedProject(unittest.TestCase):
    """A Git project with no accelerator files, and an empty state directory."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="attached-layout-test-")
        root = Path(self.temporary.name)
        self.project = root / "shop"
        self.state = root / "state" / "shop"
        self.project.mkdir()
        git(self.project, "init", "--quiet")
        git(self.project, "checkout", "--quiet", "-b", "feature/saffron-checkout")
        files = {
            "README.md": "# Shop\n\nThe saffron checkout flow settles every order.\n",
            "AGENTS.md": "# Shop rules\n\nUse the vermilion naming rule for services.\n",
            "docs/payments.md": "# Payments\n\nRefunds follow the cobalt ledger rule.\n",
            "app/Order.php": "<?php\n\nfinal class Order\n{\n}\n",
        }
        for name, content in files.items():
            path = self.project / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        git(self.project, "add", "-A")
        git(
            self.project,
            "-c", "user.email=test@example.invalid",
            "-c", "user.name=Test",
            "commit", "--quiet", "-m", "Initial shop",
        )
        self.environment = {
            **os.environ,
            "ACCELERATOR_HOME": str(EDITION),
            "ACCELERATOR_STATE_DIR": str(self.state),
            "ACCELERATOR_PROJECT_DIR": str(self.project),
        }
        for variable in ("CONTEXT_TASK_ID", "CONTEXT_CAPSULE_DELIVERED"):
            self.environment.pop(variable, None)
        self.tooling_database = EDITION / "memory-bank" / "local" / "context.db"
        self.tooling_database_before = self._stat(self.tooling_database)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    @staticmethod
    def _stat(path: Path):
        try:
            status = path.stat()
        except OSError:
            return None
        return status.st_mtime_ns, status.st_size

    def run_cli(self, *arguments: str, environment=None) -> subprocess.CompletedProcess[str]:
        # No --root: an attached runtime takes its state root from the
        # environment, as the hooks run it.
        return subprocess.run(
            [sys.executable, str(CONTEXT_SCRIPT), *arguments],
            cwd=self.project,
            env=environment or self.environment,
            capture_output=True,
            text=True,
        )

    def indexed_paths(self) -> set[str]:
        database = self.state / "memory-bank" / "local" / "context.db"
        with sqlite3.connect(database) as connection:
            return {row[0] for row in connection.execute("SELECT path FROM documents")}

    def assert_project_untouched(self) -> None:
        self.assertEqual("", git(self.project, "status", "--porcelain", "--ignored"))



class AttachedLayoutTest(AttachedProject):
    def test_index_reads_each_document_from_its_own_root(self) -> None:
        result = self.run_cli("index", "--json")
        self.assertEqual(0, result.returncode, result.stderr)

        paths = self.indexed_paths()
        # Tooling is keyed by absolute path, so it cannot collide with the
        # project's own policy file of the same name.
        self.assertIn((EDITION / "AGENTS.md").as_posix(), paths)
        self.assertIn("AGENTS.md", paths)
        self.assertTrue(
            any(path.startswith(EDITION.as_posix() + "/") and "/skills/" in path for path in paths),
            "the edition's skills are indexed in place",
        )
        self.assertIn("README.md", paths)
        self.assertIn("docs/payments.md", paths)
        # The accelerator's own README and changelog describe the
        # accelerator, not the work, and stay out of an attached index.
        self.assertNotIn((EDITION / "README.md").as_posix(), paths)
        self.assertNotIn((EDITION / "CHANGELOG.md").as_posix(), paths)

    def test_state_lives_outside_the_project_and_the_clone(self) -> None:
        result = self.run_cli("status", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        status = json.loads(result.stdout)

        self.assertEqual("attached", status["workspace"]["layout"])
        self.assertEqual(str(self.project), status["workspace"]["project"])
        self.assertEqual(
            (self.state / "memory-bank" / "local" / "context.db").resolve(),
            Path(status["database"]).resolve(),
        )
        for directory in brain.ATTACHED_STATE_DIRECTORIES:
            self.assertTrue((self.state / directory).is_dir(), directory)
        self.assertTrue((self.state / "project-brain/config/runtime.json").is_file())
        self.assertEqual(
            (EDITION / "memory-bank/.install/INDEX.md").read_bytes(),
            (self.state / "memory-bank/INDEX.md").read_bytes(),
        )
        # Tooling is read in place, never copied into the state.
        self.assertFalse((self.state / "project-brain/schemas").exists())
        self.assertFalse((self.state / "memory-bank/scripts").exists())
        self.assert_project_untouched()
        self.assertEqual(self.tooling_database_before, self._stat(self.tooling_database))

    def test_search_returns_project_text_and_tooling_paths(self) -> None:
        self.assertEqual(0, self.run_cli("index").returncode)

        result = self.run_cli("search", "saffron checkout git", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        found = [item["path"] for item in json.loads(result.stdout)["documents"]]
        self.assertIn("README.md", found)
        self.assertTrue(
            any(path.startswith(EDITION.as_posix() + "/") for path in found),
            "skills from the clone carry their absolute path",
        )

        result = self.run_cli("search", "vermilion naming", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        found = [item["path"] for item in json.loads(result.stdout)["documents"]]
        self.assertIn("AGENTS.md", found)
        self.assert_project_untouched()

    def test_turn_records_project_changes_in_the_state(self) -> None:
        (self.project / "app" / "Order.php").write_text(
            "<?php\n\nfinal class Order\n{\n    public int $total = 0;\n}\n",
            encoding="utf-8",
        )
        result = self.run_cli(
            "turn", "--task-id", "feature/saffron-checkout", "--flush", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(
            (self.state / "memory-bank/local/last-turn-report.json").read_text(
                encoding="utf-8"
            )
        )
        self.assertEqual(1, report["files"])
        self.assertTrue(report["flushed"])
        tasks = list((self.state / "project-brain/dynamic/tasks").glob("*.md"))
        self.assertEqual(1, len(tasks))
        # The only change in the project is the user's own edit.
        self.assertEqual(
            " M app/Order.php\n", git(self.project, "status", "--porcelain", "--ignored")
        )

    def test_citations_are_fingerprinted_in_the_project(self) -> None:
        with mock.patch.dict(os.environ, self.environment, clear=True):
            self.state.mkdir(parents=True)
            self.assertTrue(workspace_roots.is_attached(self.state))
            fingerprint = brain.fingerprint(self.state, "app/Order.php")
            record = {"sources": ["app/Order.php"], "source_fingerprints": [fingerprint]}
            self.assertTrue(brain.sources_are_fresh(self.state, record))

            (self.project / "app" / "Order.php").write_text("<?php\n", encoding="utf-8")
            self.assertFalse(brain.sources_are_fresh(self.state, record))
            with self.assertRaises(brain.BrainError):
                brain.fingerprint(self.state, "../../outside.txt")

    def test_variables_naming_another_copy_leave_the_installed_layout(self) -> None:
        environment = {**self.environment, "ACCELERATOR_HOME": str(self.project)}
        with mock.patch.dict(os.environ, environment, clear=True):
            self.assertIsNone(workspace_roots.configuration_error())
            self.assertEqual(workspace_roots.TOOLING_ROOT, workspace_roots.default_state_root())
            layout = workspace_roots.roots(EDITION)
            self.assertFalse(layout.attached)
            self.assertEqual(EDITION, layout.project)

    def test_incomplete_or_unsafe_variables_are_refused(self) -> None:
        cases = {
            "missing project": {"ACCELERATOR_PROJECT_DIR": ""},
            "relative state": {"ACCELERATOR_STATE_DIR": "state"},
            "state inside project": {
                "ACCELERATOR_STATE_DIR": str(self.project / ".accelerator")
            },
        }
        for label, overrides in cases.items():
            with self.subTest(label):
                result = self.run_cli(
                    "status", "--json", environment={**self.environment, **overrides}
                )
                self.assertNotEqual(0, result.returncode)
                self.assertIn("ACCELERATOR_", result.stderr)
                self.assert_project_untouched()


class AttachedHooksTest(AttachedProject):
    """The edition's own hooks, run from the clone against the project."""

    def run_hook(self, name: str, payload: dict) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(EDITION / ".claude" / "hooks" / name)],
            input=json.dumps(payload),
            cwd=self.project,
            env={**self.environment, "CONTEXT_HOOK_BUDGET": "120"},
            capture_output=True,
            text=True,
            timeout=300,
        )

    def test_session_start_names_the_clone_and_the_state(self) -> None:
        result = self.run_hook("local-context.sh", {"hook_event_name": "SessionStart"})
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(f"Accelerator: attached from {EDITION}", result.stdout)
        self.assertIn(str(self.state), result.stdout)
        self.assertIn("Branch: feature/saffron-checkout", result.stdout)
        self.assert_project_untouched()

    def test_prompt_and_stop_hooks_write_only_the_state(self) -> None:
        result = self.run_hook(
            "working-memory-read.sh", {"prompt": "Explain the saffron checkout flow"}
        )
        self.assertEqual(0, result.returncode, result.stderr)
        health = (self.state / "memory-bank/local/refresh-health.ndjson").read_text(
            encoding="utf-8"
        )
        self.assertIn('"hook_status":0', health)
        self.assert_project_untouched()

        (self.project / "app" / "Order.php").write_text(
            "<?php\n\nfinal class Order\n{\n    public int $total = 0;\n}\n",
            encoding="utf-8",
        )
        result = self.run_hook("working-memory-write.sh", {"hook_event_name": "Stop"})
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(
            (self.state / "memory-bank/local/last-turn-report.json").read_text(
                encoding="utf-8"
            )
        )
        # The task comes from the project's branch, not the clone's.
        self.assertEqual("feature/saffron-checkout", report["task_id"])
        self.assertEqual(1, report["files"])
        self.assertEqual(
            " M app/Order.php\n", git(self.project, "status", "--porcelain", "--ignored")
        )
        self.assertEqual(self.tooling_database_before, self._stat(self.tooling_database))


if __name__ == "__main__":
    unittest.main()
