#!/usr/bin/env python3
"""Regression tests for scripts/check_php_snippets.py.

The gate lints fragments by wrapping them; the one shape it must never use is
raw untagged text, which `php -l` accepts as inline HTML whatever it holds.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import check_php_snippets  # noqa: E402

FENCE = "`" * 3


@unittest.skipUnless(shutil.which("php"), "php is not installed")
class SnippetGateTest(unittest.TestCase):
    def check(self, blocks: dict) -> tuple:
        with tempfile.TemporaryDirectory(prefix="snippet-gate-") as raw:
            root = Path(raw)
            body = "".join(
                f"{FENCE}php{info}\n{code}\n{FENCE}\n\n" for info, code in blocks.items()
            )
            (root / "doc.md").write_text(body, encoding="utf-8")
            subprocess.run(["git", "init", "--quiet"], cwd=root, check=True)
            subprocess.run(["git", "add", "doc.md"], cwd=root, check=True)
            return check_php_snippets.check(root)

    def test_every_shape_is_linted_and_counted(self) -> None:
        counts, failures = self.check({
            "": "<?php\n\nfunction ok(): int { return 1; }",
            " ": "$total = array_sum([1, 2]);",
            "  ": "public function handle(): void\n{\n}",
            "   ": "<p><?= htmlspecialchars($name) ?></p>",
            " fragment": "'guards' => [",
        })
        self.assertEqual([], failures)
        self.assertEqual(1, counts["complete"])
        self.assertEqual(1, counts["statements"])
        self.assertEqual(1, counts["class members"])
        self.assertEqual(1, counts["template"])
        self.assertEqual(1, counts["declared fragment"])

    def test_broken_untagged_fragment_fails(self) -> None:
        _, failures = self.check({"": "$x = ;"})
        self.assertEqual(1, len(failures))
        self.assertIn("no shape parses", failures[0])

    def test_broken_complete_file_fails(self) -> None:
        _, failures = self.check({"": "<?php\nfunction broken( {"})
        self.assertEqual(1, len(failures))

    def test_client_task_material_is_not_linted(self) -> None:
        with tempfile.TemporaryDirectory(prefix="snippet-gate-") as raw:
            root = Path(raw)
            client = root / "Laravel" / "Task" / "project-work" / "specs"
            client.mkdir(parents=True)
            (client / "schema.md").write_text(f"{FENCE}php\n$x = ;\n{FENCE}\n", encoding="utf-8")
            (root / "doc.md").write_text(f"{FENCE}php\n$total = 1;\n{FENCE}\n", encoding="utf-8")
            subprocess.run(["git", "init", "--quiet"], cwd=root, check=True)
            subprocess.run(["git", "add", "--all"], cwd=root, check=True)
            counts, failures = check_php_snippets.check(root)
        self.assertEqual([], failures)
        self.assertEqual(1, counts["statements"])


if __name__ == "__main__":
    unittest.main()
