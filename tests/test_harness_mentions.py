"""The composer's @ search: a workspace's files and folders, by name only, as the CLIs' own @ offers them."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness" / "src"))
from harness import mentions, sessions  # noqa: E402


def paths(result):
    return [item["path"] for item in result["items"]]


class RankTests(unittest.TestCase):
    ITEMS = mentions.entries(["README.md", "composer.json", "src/Billing/Totals.php", "src/Billing/Invoice.php",
                              "src/Cart.php", "tests/BillingTest.php"])

    def rank(self, query):
        return [item["path"] for item in mentions.rank(self.ITEMS, query)]

    def test_folders_come_from_the_files_they_hold(self):
        folders = [item["path"] for item in self.ITEMS if item["kind"] == "folder"]
        self.assertEqual(["src/", "src/Billing/", "tests/"], folders)

    def test_an_empty_query_offers_the_top_of_the_tree_folders_first(self):
        self.assertEqual(["src/", "tests/", "composer.json", "README.md"], self.rank(""))

    def test_a_name_that_starts_with_the_letters_comes_first_then_a_word_in_the_path(self):
        self.assertEqual(["src/Billing/", "tests/BillingTest.php", "src/Billing/Totals.php", "src/Billing/Invoice.php"],
                         self.rank("bil"))
        self.assertEqual(["src/Billing/Totals.php"], self.rank("tot"))
        # Letters in order still match, after everything closer.
        self.assertEqual(["tests/BillingTest.php", "src/Billing/Totals.php"], self.rank("sbt"))
        self.assertEqual([], self.rank("zzz"))

    def test_a_folder_query_offers_what_is_inside_it_and_not_itself(self):
        self.assertEqual(["src/Billing/", "src/Cart.php", "src/Billing/Totals.php", "src/Billing/Invoice.php"], self.rank("src/"))
        self.assertEqual(["src/Billing/", "src/Billing/Totals.php", "src/Billing/Invoice.php"], self.rank("src/bil"))
        self.assertEqual(["src/Billing/Invoice.php", "src/Billing/Totals.php"], self.rank("SRC/BILLING/"))


class WorkspaceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def write(self, *names):
        for name in names:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("content\n", encoding="utf-8")

    @unittest.skipUnless(shutil.which("git"), "needs git")
    def test_git_lists_tracked_and_unignored_files_never_ignored_ones(self):
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        self.write(".gitignore", "src/App.php", "vendor/lib/Lib.php", "notes/todo.md", ".env")
        (self.root / ".gitignore").write_text("vendor/\n.env\n", encoding="utf-8")
        subprocess.run(["git", "-C", str(self.root), "add", ".gitignore", "src/App.php"], check=True)
        found = paths(mentions.Mentions().search(self.root, ""))
        self.assertEqual(["notes/", "src/", ".gitignore"], found)
        everything = [item["path"] for item in mentions.Mentions()._items(self.root)[0]]
        self.assertIn("notes/todo.md", everything)  # untracked, not ignored
        self.assertNotIn("vendor/lib/Lib.php", everything)
        self.assertNotIn(".env", everything)

    def test_outside_git_a_walk_skips_dependencies_and_says_when_it_stops(self):
        self.write("src/App.php", "node_modules/pkg/index.js", "vendor/lib/Lib.php", ".git/config", "docs/a.md", "docs/b.md")
        with patch.object(mentions, "_git_paths", return_value=None):
            result = mentions.Mentions().search(self.root, "")
            self.assertEqual((["docs/", "src/"], False), (paths(result), result["truncated"]))
            with patch.object(mentions, "LIMIT", 2):
                result = mentions.Mentions().search(self.root, "")
        self.assertTrue(result["truncated"])

    def test_a_listing_is_reused_for_a_while_and_names_are_all_that_is_read(self):
        self.write("src/App.php")
        index = mentions.Mentions()
        with patch.object(mentions, "_git_paths", return_value=["src/App.php"]) as listed:
            index.search(self.root, "app")
            index.search(self.root, "src/")
        self.assertEqual(1, listed.call_count)

    def test_a_query_that_is_too_long_or_holds_control_characters_is_refused(self):
        for query in ("x" * 301, "a\nb", "a\0b", None):
            with self.subTest(query=query), self.assertRaises(sessions.SessionError):
                mentions.Mentions().search(self.root, query)


if __name__ == "__main__":
    unittest.main()
