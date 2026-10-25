"""A worktree served by its main checkout's installation: the shared layout.

A linked worktree has none of an installation Git does not track, so Claude
Code started there ran nothing of it. The person's dispatcher runs the main
checkout's hooks for such a session with ACCELERATOR_HOME and
ACCELERATOR_STATE_DIR naming the main checkout and ACCELERATOR_PROJECT_DIR
the worktree (memory-bank/scripts/workspace_roots.py): the state is the main
checkout's and keeps its keys, the worktree is the project whose branch
names the task, and the index is the worktree's own, because the two
checkouts' documents differ.
"""
import importlib.util
import json
import os
from pathlib import Path
import re
import sqlite3
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tests.hook_shell import BASH

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = os.name == "nt"


def git(folder, *args):
    return subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                           "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args],
                          cwd=folder, check=True, capture_output=True, text=True).stdout


class SharedWorktree(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="shared-worktree-")
        base = Path(cls.temporary.name).resolve()
        cls.main = base / "shop"
        (cls.main / "src").mkdir(parents=True)
        (cls.main / "composer.json").write_text('{"name": "acme/shop"}\n', encoding="utf-8")
        (cls.main / "README.md").write_text("# Shop\n\nThe saffron checkout settles every order.\n",
                                            encoding="utf-8")
        (cls.main / "src" / "Invoice.php").write_text("<?php\nfinal class Invoice {}\n", encoding="utf-8")
        git(cls.main, "init", "-q", "-b", "feature/SHOP-7-invoices")
        git(cls.main, "add", ".")
        git(cls.main, "commit", "-qm", "Shop")
        cls.env = {key: value for key, value in os.environ.items()
                   if not key.startswith(("CLAUDE", "ACCELERATOR_", "CONTEXT_", "GIT_"))}
        cls.env["PYTHONDONTWRITEBYTECODE"] = "1"
        done = subprocess.run([sys.executable, str(ROOT / "scripts" / "install_accelerator.py"), "--edition",
                               "PHP Core", "--target", str(cls.main), "--merge-existing"],
                              cwd=ROOT, env=cls.env, capture_output=True, text=True, stdin=subprocess.DEVNULL)
        if done.returncode:
            raise AssertionError(done.stdout[-2000:] + done.stderr[-2000:])
        cls.worktree = cls.main / ".claude" / "worktrees" / "refunds"
        git(cls.main, "worktree", "add", "-q", str(cls.worktree), "-b", "feature/SHOP-9-refunds")
        cls.shared = dict(cls.env, ACCELERATOR_HOME=str(cls.main), ACCELERATOR_STATE_DIR=str(cls.main),
                          ACCELERATOR_PROJECT_DIR=str(cls.worktree))
        cls.brain_mode = stat.S_IMODE((cls.main / "project-brain").stat().st_mode)

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def cli(self, *arguments, env=None):
        return subprocess.run([sys.executable, str(self.main / "memory-bank" / "scripts" / "context.py"),
                               *arguments], cwd=self.worktree, env=env or self.shared, capture_output=True,
                              text=True, stdin=subprocess.DEVNULL)

    def installed_roots(self):
        """The installed copy's workspace_roots, under a name of its own."""
        spec = importlib.util.spec_from_file_location(
            "workspace_roots_of_the_installation", self.main / "memory-bank" / "scripts" / "workspace_roots.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_the_worktree_has_an_index_of_its_own_beside_the_shared_state(self):
        main_index = self.main / "memory-bank" / "local" / "context.db"
        before = main_index.stat().st_mtime_ns if main_index.exists() else None
        done = self.cli("status", "--json")
        self.assertEqual(done.returncode, 0, done.stderr)
        status = json.loads(done.stdout)
        self.assertEqual((status["workspace"]["layout"], Path(status["workspace"]["project"])),
                         ("shared", self.worktree))
        database = Path(status["database"])
        self.assertEqual(database.parent.parent, (self.main / "memory-bank" / "local" / "worktrees").resolve())
        self.assertRegex(database.parent.name, r"^[0-9a-f]{16}$")
        done = self.cli("index", "--json")
        self.assertEqual(done.returncode, 0, done.stderr)
        with sqlite3.connect(database) as connection:
            paths = {row[0] for row in connection.execute("SELECT path FROM documents")}
        # The worktree's own documents by their keys, the installation's
        # policy and skills from the main checkout by their full path.
        self.assertIn("README.md", paths)
        self.assertIn((self.main / "AGENTS.md").as_posix(), paths)
        self.assertTrue(any(path.startswith(self.main.as_posix() + "/") and "/skills/" in path for path in paths))
        # Its specs and tasks too, which Git gives the worktree none of.
        self.assertTrue(any(path.startswith(self.main.as_posix() + "/specs/") for path in paths), paths)
        self.assertFalse(any(path.startswith(self.main.as_posix() + "/memory-bank/") for path in paths))
        after = main_index.stat().st_mtime_ns if main_index.exists() else None
        self.assertEqual(before, after, "the main checkout's own index is left alone")
        # The worktree's own copy of one of the installation's files wins.
        own = self.worktree / "specs" / "MANIFEST.md"
        own.parent.mkdir()
        own.write_text("# The worktree's specifications\n", encoding="utf-8")
        try:
            done = self.cli("index", "--json")
            self.assertEqual(done.returncode, 0, done.stderr)
            with sqlite3.connect(database) as connection:
                paths = {row[0] for row in connection.execute("SELECT path FROM documents")}
        finally:
            own.unlink()
            own.parent.rmdir()
        self.assertIn("specs/MANIFEST.md", paths)
        self.assertNotIn((self.main / "specs" / "MANIFEST.md").as_posix(), paths)

    def test_keys_and_citations_resolve_where_each_file_lives(self):
        roots = self.installed_roots()
        with mock.patch.dict(os.environ, self.shared):
            layout = roots.roots(self.main)
            self.assertTrue(layout.shared and layout.attached)
            self.assertEqual(layout.project, self.worktree)
            chunk = self.main / "memory-bank" / "chunks" / "MEM-x.md"
            self.assertEqual(roots.key_for(self.main, layout.state, chunk), "memory-bank/chunks/MEM-x.md")
            self.assertEqual(roots.key_for(self.main, layout.tooling, self.main / "AGENTS.md"),
                             (self.main / "AGENTS.md").as_posix())
            self.assertEqual(roots.key_for(self.main, layout.project, self.worktree / "README.md"), "README.md")
            # A record of the main checkout cites the installation's files as
            # its own keys; the worktree has none of them.
            self.assertEqual(roots.resolve(self.main, "AGENTS.md"), self.main / "AGENTS.md")
            self.assertEqual(roots.resolve(self.main, "specs/MANIFEST.md"), self.main / "specs" / "MANIFEST.md")
            self.assertEqual(roots.resolve(self.main, "src/Invoice.php"), self.worktree / "src" / "Invoice.php")
            self.assertEqual(roots.resolve(self.main, "project-brain/config/runtime.json"),
                             self.main / "project-brain" / "config" / "runtime.json")
            own = self.worktree / "AGENTS.md"
            own.write_text("# The worktree's own\n", encoding="utf-8")
            try:
                self.assertEqual(roots.resolve(self.main, "AGENTS.md"), own)
            finally:
                own.unlink()
        self.assertFalse(roots.roots(self.main).attached, "without the variables it is the installed layout")

    def test_a_result_saved_in_the_worktree_lands_in_the_shared_brain_under_its_branch(self):
        request = Path(self.temporary.name) / "result.json"
        request.write_text(json.dumps({
            "progress": "Refunds checked.", "verified": True,
            "learnings": [{"type": "finding", "title": "Refunds reuse the invoice number",
                           "consequence": "A refund cites the invoice it reverses.",
                           "sources": ["src/Invoice.php"]}]}), encoding="utf-8")
        done = self.cli("start", "--task-id", "feature/SHOP-9-refunds", "--goal", "Number refunds", "--json")
        self.assertEqual(done.returncode, 0, done.stderr)
        done = self.cli("record-result", "--task-id", "feature/SHOP-9-refunds", "--result-id", "refund-1",
                        "--revision", "auto", "--input", str(request), "--json")
        self.assertEqual(done.returncode, 0, done.stderr)
        tasks = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                 for path in (self.main / "project-brain" / "dynamic" / "tasks").glob("*.md")]
        self.assertIn("feature/SHOP-9-refunds", [task["external_id"] for task in tasks])
        findings = (self.main / "project-brain" / "dynamic" / "findings").glob("*.md")
        self.assertTrue(any("Refunds reuse the invoice number" in path.read_text(encoding="utf-8")
                            for path in findings))
        # Installed state keeps the project's own permissions, and nothing
        # is written into the worktree.
        self.assertEqual(stat.S_IMODE((self.main / "project-brain").stat().st_mode), self.brain_mode)
        self.assertEqual(git(self.worktree, "status", "--porcelain", "--ignored"), "")

    def test_the_index_of_a_removed_worktree_goes_at_the_next_checkpoint(self):
        gone = self.main / ".claude" / "worktrees" / "short-lived"
        git(self.main, "worktree", "add", "-q", str(gone), "-b", "feature/SHOP-11-short-lived")
        done = self.cli("status", "--json", env=dict(self.shared, ACCELERATOR_PROJECT_DIR=str(gone)))
        self.assertEqual(done.returncode, 0, done.stderr)
        index = Path(json.loads(done.stdout)["database"]).parent
        self.assertEqual((index / "worktree").read_text(encoding="utf-8"), os.path.realpath(gone))
        kept = Path(json.loads(self.cli("status", "--json").stdout)["database"]).parent
        git(self.main, "worktree", "remove", "--force", str(gone))
        invoice = self.main / "src" / "Invoice.php"
        original = invoice.read_text(encoding="utf-8")
        invoice.write_text(original + "// numbered per tenant\n", encoding="utf-8")
        try:
            done = subprocess.run([sys.executable, str(self.main / "memory-bank" / "scripts" / "context.py"), "turn",
                                   "--task-id", "feature/SHOP-7-invoices", "--flush", "--json"],
                                  cwd=self.main, env=self.env, capture_output=True, text=True,
                                  stdin=subprocess.DEVNULL)
            self.assertEqual(done.returncode, 0, done.stderr)
        finally:
            invoice.write_text(original, encoding="utf-8")
        self.assertFalse(index.exists(), "the removed worktree's index is gone")
        self.assertTrue((kept / "worktree").is_file(), "a worktree that is still there keeps its index")

    @unittest.skipIf(BASH is None, "no bash to run hooks with")
    def test_the_prompt_hook_in_the_worktree_works_for_its_own_branch(self):
        payload = {"session_id": "worktree-session", "hook_event_name": "UserPromptSubmit",
                   "prompt": "How are refunds numbered?", "cwd": str(self.worktree)}
        done = subprocess.run([BASH, (self.main / ".claude" / "hooks" / "working-memory-read.sh").as_posix()],
                              input=json.dumps(payload), cwd=self.worktree, env=self.shared,
                              capture_output=True, text=True, timeout=120)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertNotIn("Memory not consulted", done.stdout)
        self.assertNotIn("Memory refresh unavailable", done.stdout)
        self.assertRegex(done.stdout, re.compile(r"^working: (feature/SHOP-9-refunds|not recorded yet)", re.M))
        self.assertNotIn("SHOP-7", done.stdout)

    @unittest.skipIf(BASH is None, "no bash to run hooks with")
    def test_the_session_start_names_the_worktrees_branch_and_where_its_memory_is(self):
        payload = {"session_id": "worktree-start", "hook_event_name": "SessionStart", "source": "startup",
                   "cwd": str(self.worktree)}
        done = subprocess.run([BASH, (self.main / ".claude" / "hooks" / "local-context.sh").as_posix()],
                              input=json.dumps(payload), cwd=self.worktree, env=self.shared,
                              capture_output=True, text=True, timeout=120)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("Branch: feature/SHOP-9-refunds", done.stdout)
        # The root as the hook's shell spells it: /c/Users/... in Git Bash.
        self.assertRegex(done.stdout, r"Accelerator: the installation in \S*/shop, whose memory this worktree shares")
        self.assertNotIn("attached from", done.stdout)
        # The worktree has no memory-bank/ of its own: the files are named
        # where they are.
        self.assertNotIn(" Read memory-bank/README.md", done.stdout)


if __name__ == "__main__":
    unittest.main()
