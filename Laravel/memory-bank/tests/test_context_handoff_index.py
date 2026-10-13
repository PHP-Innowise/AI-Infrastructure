#!/usr/bin/env python3
"""Portable conversation snapshots must stay out of automatic retrieval."""

from __future__ import annotations

import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts/context.py"
SNAPSHOT = '---\n{"type": "context-handoff", "schema_version": 1}\n---\n\nUser: violet transcript\n'


class HandoffIndexTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="handoff-index-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        (self.root / "docs").mkdir()
        (self.root / "tasks/TASK-001").mkdir(parents=True)
        (self.root / "docs/guide.md").write_text("# Guide\nCurrent cobalt contract.\n")

    def index(self, *args):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.root),
             "--mode", "lightweight", "index", "--json", *args],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def indexed_paths(self):
        with sqlite3.connect(self.root / "memory-bank/local/context.db") as db:
            return [row[0] for row in db.execute("SELECT path FROM documents ORDER BY path")]

    def test_snapshots_and_renamed_copies_are_never_indexed(self):
        paths = ["tasks/TASK-001/context-save-snapshot.md", "docs/copied-history.md"]
        for path in paths:
            (self.root / path).write_text(SNAPSHOT)
        for options in [(), ("--incremental",)]:
            result = self.index(*options)
            self.assertEqual(["docs/guide.md"], self.indexed_paths())
            excluded = {item["path"]: item["reason"] for item in result["excluded"]}
            for path in paths:
                self.assertEqual("explicit-context-only", excluded[path])

    def test_incremental_index_removes_a_source_replaced_with_a_snapshot(self):
        source = self.root / "docs/history.md"
        source.write_text("# Work\nPreviously ordinary project documentation.\n")
        self.index()
        self.assertIn("docs/history.md", self.indexed_paths())
        source.write_text(SNAPSHOT)
        self.index("--incremental")
        self.assertEqual(["docs/guide.md"], self.indexed_paths())

    def test_documentation_with_an_embedded_example_still_indexes(self):
        (self.root / "docs/example.md").write_text("# Format example\n\n```markdown\n" + SNAPSHOT + "```\n")
        self.index()
        self.assertEqual(["docs/example.md", "docs/guide.md"], self.indexed_paths())


if __name__ == "__main__":
    unittest.main()
