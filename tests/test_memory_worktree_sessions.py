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
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
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
        # A runner is slower than a laptop, and the first refresh in a
        # worktree builds its index: under the hooks' own 5-second budget
        # this would be a timing test (as in test_windows_memory_hooks).
        cls.env["CONTEXT_HOOK_BUDGET"] = "60"
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
        with closing(sqlite3.connect(database)) as connection:
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
            with closing(sqlite3.connect(database)) as connection:
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
        # The state named otherwise than the hook's shell names the root - C:\\... where
        # Git Bash says /c/..., or with a trailing separator - is still this installation.
        for state in (str(self.main), str(self.main) + os.sep):
            with self.subTest(state=state):
                done = subprocess.run([BASH, (self.main / ".claude" / "hooks" / "local-context.sh").as_posix()],
                                      input=json.dumps(payload), cwd=self.worktree,
                                      env=dict(self.shared, ACCELERATOR_STATE_DIR=state),
                                      capture_output=True, text=True, timeout=120)
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertIn("Branch: feature/SHOP-9-refunds", done.stdout)
                # The root as the hook's shell spells it: /c/Users/... in Git Bash.
                self.assertRegex(done.stdout,
                                 r"Accelerator: the installation in \S*/shop, whose memory this worktree shares")
                self.assertNotIn("attached from", done.stdout)
                self.assertIn("the memory-bank/ and project-brain/ paths the policy and skills name are there",
                              done.stdout)
                # The worktree has no memory-bank/ of its own: the files are
                # named where they are, the memory CLI too.
                self.assertNotIn(" Read memory-bank/README.md", done.stdout)
                self.assertRegex(done.stdout, r"Memory CLI: this worktree has none - run the commands the policy "
                                              r"and skills name with `\S+ \S*/shop/memory-bank/scripts/context\.py`")

    @unittest.skipIf(BASH is None, "no bash to run hooks with")
    def test_a_session_started_below_the_root_hears_where_the_cli_is(self):
        # Codex runs a hook in the session's folder; the dispatcher runs
        # Claude Code's in the root. Either way the payload names the folder.
        hook = (self.main / ".claude" / "hooks" / "local-context.sh").as_posix()
        for where, cwd in (("src", self.main / "src"), ("the root through the dispatcher", self.main)):
            with self.subTest(hook_in=where):
                payload = {"session_id": "below", "hook_event_name": "SessionStart", "source": "startup",
                           "cwd": str(self.main / "src")}
                done = subprocess.run([BASH, hook], input=json.dumps(payload), cwd=cwd,
                                      env=dict(self.env, CLAUDE_PROJECT_DIR=str(cwd)), capture_output=True,
                                      text=True, timeout=120)
                self.assertEqual(done.returncode, 0, done.stderr)
                named = re.search(r"Memory CLI: this session started below the project root - run the commands "
                                  r"the policy and skills name with `([^`]+)`", done.stdout)
                self.assertIsNotNone(named, done.stdout)
                ran = subprocess.run([BASH, "-c", named.group(1) + " status --json"], cwd=self.main / "src",
                                     env=self.env, capture_output=True, text=True, timeout=120)
                self.assertEqual(self.layout(ran), ("installed", self.main))
        payload = {"session_id": "root", "hook_event_name": "SessionStart", "source": "startup",
                   "cwd": str(self.main)}
        done = subprocess.run([BASH, hook], input=json.dumps(payload), cwd=self.main,
                              env=dict(self.env, CLAUDE_PROJECT_DIR=str(self.main)), capture_output=True,
                              text=True, timeout=120)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertNotIn("Memory CLI:", done.stdout)

    def hook(self, name, payload, cwd, **extra):
        """One of the root's own hooks, as Claude Code runs it for a session
        that started in the root: CLAUDE_PROJECT_DIR names the root, the
        process and the input's cwd are where the session works now."""
        return subprocess.run([BASH, (self.main / ".claude" / "hooks" / name).as_posix()],
                              input=json.dumps(payload), cwd=cwd,
                              env=dict(self.env, CLAUDE_PROJECT_DIR=str(self.main), **extra),
                              capture_output=True, text=True, timeout=120)

    def save_as_asked(self, stop, cwd, title):
        """Run the save step a Stop hook asked for, as the agent's shell does:
        in the session's folder, with none of the hooks' variables."""
        decision = json.loads(stop.stdout)
        request = decision.get("reason") or decision.get("hookSpecificOutput", {}).get("additionalContext", "")
        command = re.search(r"exact command: (cd [^\n]*?--json(?: && cd [^\n]*)?)\n", request)
        self.assertIsNotNone(command, request[:400])
        result = json.dumps({"progress": f"{title}.", "verified": True,
                             "learnings": [{"type": "finding", "title": title,
                                            "consequence": "A session that entered a worktree saves under its branch.",
                                            "sources": ["src/Invoice.php"]}]})
        done = subprocess.run([BASH, "-c", command.group(1).replace("'<JSON>'", shlex.quote(result))], cwd=cwd,
                              env=self.env, capture_output=True, text=True, timeout=120)
        self.assertEqual(done.returncode, 0, done.stderr + done.stdout)
        return command.group(1)

    @unittest.skipIf(WINDOWS or (hasattr(os, "geteuid") and os.geteuid() == 0),
                     "a folder without write permission stops neither Windows nor root")
    def test_a_command_a_sandbox_holds_for_a_worktree_is_made_for_it(self):
        # A sandbox that lets a command write only its own folder refuses the
        # memory's: the command waits for the next hook (the write queue). A
        # session that entered a worktree names it (--project), as its save
        # step does; the waiting command keeps the name, and the hook's turn
        # in that worktree - the roots' variables as the hooks export them -
        # makes it there, under the worktree's branch.
        entered = self.main / ".claude" / "worktrees" / "queued"
        git(self.main, "worktree", "add", "-q", str(entered), "-b", "worktree-queued")
        self.addCleanup(git, self.main, "worktree", "remove", "--force", str(entered))
        spool = tempfile.TemporaryDirectory(prefix="queue-")
        self.addCleanup(spool.cleanup)
        env = dict(self.env, TMPDIR=spool.name)

        def cli(*arguments, **extra):
            return subprocess.run([sys.executable, str(self.main / "memory-bank" / "scripts" / "context.py"),
                                   *arguments], cwd=self.main, env=dict(env, **extra), capture_output=True,
                                  text=True, timeout=120)

        started = cli("--project", str(entered), "start", "--task-id", "worktree-queued", "--goal", "Queued", "--json")
        self.assertEqual(started.returncode, 0, started.stderr)
        locked = [self.main / "project-brain" / "local", self.main / "memory-bank" / "local"]
        modes = [folder.stat().st_mode for folder in locked]

        def unlock():
            for folder, mode in zip(locked, modes):
                folder.chmod(stat.S_IMODE(mode))

        self.addCleanup(unlock)
        for folder in locked:
            folder.chmod(0o555)
        done = cli("--project", str(entered), "update", "--task-id", "worktree-queued", "--revision", "auto",
                   "--progress", "Made in the worktree", "--json")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(json.loads(done.stdout)["queued"], done.stdout)
        (record,) = Path(spool.name).glob("ai-accelerator-memory-queue-*/command-*.json")
        self.assertIn("--project=" + str(entered), json.loads(record.read_text(encoding="utf-8"))["argv"])
        unlock()
        turned = cli("turn", "--task-id", "worktree-queued", "--flush-after", "5", "--json",
                     ACCELERATOR_HOME=str(self.main), ACCELERATOR_STATE_DIR=str(self.main),
                     ACCELERATOR_PROJECT_DIR=str(entered))
        self.assertEqual(turned.returncode, 0, turned.stderr)
        self.assertEqual([], list(Path(spool.name).glob("ai-accelerator-memory-queue-*/command-*.json")))
        tasks = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                 for path in (self.main / "project-brain" / "dynamic" / "tasks").glob("*.md")]
        self.assertTrue(any(task["external_id"] == "worktree-queued" and task.get("progress") == "Made in the worktree"
                            for task in tasks), [task["external_id"] for task in tasks])

    @unittest.skipIf(BASH is None, "no bash to run hooks with")
    def test_a_session_that_entered_a_worktree_keeps_its_memory_there(self):
        # Claude Code's EnterWorktree makes .claude/worktrees/<name> on a
        # branch of its own and moves the session in; the root's hooks keep
        # running, with the input's cwd the worktree.
        entered = self.main / ".claude" / "worktrees" / "entered"
        git(self.main, "worktree", "add", "-q", str(entered), "-b", "worktree-entered")
        self.addCleanup(git, self.main, "worktree", "remove", "--force", str(entered))
        for turn, (asked_in, title) in enumerate(((self.main, "Entering keeps the turn"),
                                                  (entered, "Entered worktrees keep their memory"))):
            with self.subTest(turn=turn):
                # The first turn is asked in the root and enters the worktree;
                # the next one is asked and ends there.
                session = {"session_id": "entered-session"}
                done = self.hook("working-memory-read.sh", dict(session, cwd=str(asked_in),
                                 hook_event_name="UserPromptSubmit", prompt="How are invoices numbered?"), asked_in)
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertNotIn("Memory not consulted", done.stdout)
                if asked_in == entered:
                    self.assertRegex(done.stdout, re.compile(r"^working: (worktree-entered|not recorded yet)", re.M))
                    self.assertNotIn("SHOP-7", done.stdout)
                stop = self.hook("working-memory-write.sh", dict(session, cwd=str(entered), hook_event_name="Stop",
                                 stop_hook_active=False, last_assistant_message="Checked."), entered)
                self.assertEqual(stop.returncode, 0, stop.stderr)
                command = self.save_as_asked(stop, entered, title)
                self.assertIn("--project " + shlex.quote(str(entered)), command)
                done = self.hook("working-memory-write.sh", dict(session, cwd=str(entered), hook_event_name="Stop",
                                 stop_hook_active=True, last_assistant_message="Saved."), entered)
                self.assertEqual(done.returncode, 0, done.stderr)
        tasks = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                 for path in (self.main / "project-brain" / "dynamic" / "tasks").glob("*.md")]
        self.assertTrue(any(task["external_id"] == "worktree-entered" and task.get("progress") for task in tasks))
        findings = [path.read_text(encoding="utf-8")
                    for path in (self.main / "project-brain" / "dynamic" / "findings").glob("*.md")]
        for title in ("Entering keeps the turn", "Entered worktrees keep their memory"):
            self.assertTrue(any(title in finding for finding in findings), title)
        # After a compaction the session starts again there: the banner is the worktree's.
        done = self.hook("local-context.sh", {"session_id": "entered-session", "cwd": str(entered),
                                              "hook_event_name": "SessionStart", "source": "compact"}, entered)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("Branch: worktree-entered", done.stdout)
        self.assertRegex(done.stdout, r"Accelerator: the installation in \S*/shop, whose memory this worktree shares")
        self.assertEqual(git(entered, "status", "--porcelain", "--ignored"), "")

    @unittest.skipIf(BASH is None, "no bash to run hooks with")
    def test_a_worktree_with_its_own_installation_is_still_left_to_it(self):
        own = self.main / ".claude" / "worktrees" / "installed"
        git(self.main, "worktree", "add", "-q", str(own), "-b", "worktree-installed")
        self.addCleanup(git, self.main, "worktree", "remove", "--force", str(own))
        runtime = own / "memory-bank" / "scripts" / "context.py"
        runtime.parent.mkdir(parents=True)
        runtime.write_text("# Its own runtime\n", encoding="utf-8")
        done = self.hook("working-memory-read.sh", {"session_id": "own", "cwd": str(own),
                                                    "hook_event_name": "UserPromptSubmit",
                                                    "prompt": "How are invoices numbered?"}, own)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("Memory not consulted", done.stdout)

    def test_the_cli_takes_a_served_worktree_and_refuses_any_other_folder(self):
        cli = [sys.executable, str(self.main / "memory-bank" / "scripts" / "context.py")]
        done = subprocess.run([*cli, "--project", str(self.worktree), "status", "--json"], cwd=self.main,
                              env=self.env, capture_output=True, text=True, stdin=subprocess.DEVNULL)
        self.assertEqual(self.layout(done), ("shared", self.worktree))
        done = subprocess.run([*cli, "--project", str(self.main), "status", "--json"], cwd=self.main,
                              env=self.env, capture_output=True, text=True, stdin=subprocess.DEVNULL)
        self.assertEqual(self.layout(done), ("installed", self.main))
        outside = Path(self.temporary.name).resolve() / "elsewhere"
        outside.mkdir(exist_ok=True)
        done = subprocess.run([*cli, "--project", str(outside), "status", "--json"], cwd=self.main,
                              env=self.env, capture_output=True, text=True, stdin=subprocess.DEVNULL)
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("--project names neither", done.stderr)

    def plain(self, *arguments, cwd, **extra):
        """The installation's CLI run in `cwd` with none of the dispatcher's
        variables: a Codex command, the memory MCP server, a terminal."""
        return subprocess.run([sys.executable, str(self.main / "memory-bank" / "scripts" / "context.py"),
                               *arguments], cwd=cwd, env=dict(self.env, **extra), capture_output=True,
                              text=True, stdin=subprocess.DEVNULL)

    def layout(self, done):
        self.assertEqual(done.returncode, 0, done.stderr)
        workspace = json.loads(done.stdout)["workspace"]
        return workspace["layout"], Path(workspace["project"])

    def scratch_worktree(self, name):
        """A worktree of the main checkout for one test, removed after it."""
        folder = Path(self.temporary.name).resolve() / name
        git(self.main, "worktree", "add", "-q", str(folder), "-b", f"feature/SHOP-{name}")
        self.addCleanup(git, self.main, "worktree", "remove", "--force", str(folder))
        return folder

    def test_the_installations_cli_run_in_the_worktree_finds_the_worktree_by_itself(self):
        for folder in (self.worktree, self.worktree / "src"):
            with self.subTest(folder=folder):
                self.assertEqual(self.layout(self.plain("status", "--json", cwd=folder)),
                                 ("shared", self.worktree))
        # The main checkout stays the root, and so does a Claude Code
        # session there whose shell moved into the worktree: its hooks get
        # CLAUDE_PROJECT_DIR, which names the session's own project.
        for folder, extra in ((self.main, {}), (self.main / "src", {}),
                              (self.worktree, {"CLAUDE_PROJECT_DIR": str(self.main)})):
            with self.subTest(folder=folder, **extra):
                self.assertEqual(self.layout(self.plain("status", "--json", cwd=folder, **extra)),
                                 ("installed", self.main))

    def test_a_result_saved_from_the_worktree_without_the_variables_lands_under_its_branch(self):
        folder = self.scratch_worktree("12-plain")
        # A file only the worktree's branch has: read from the main checkout,
        # the citation would name nothing.
        (folder / "src" / "Refund.php").write_text("<?php\nfinal class Refund {}\n", encoding="utf-8")
        git(folder, "add", "src/Refund.php")
        git(folder, "commit", "-qm", "Refund")
        request = Path(self.temporary.name) / "plain-result.json"
        request.write_text(json.dumps({
            "progress": "Plain refunds checked.", "verified": True,
            "learnings": [{"type": "finding", "title": "Plain refunds keep the invoice date",
                           "consequence": "A refund made without the variables keeps its invoice's date.",
                           "sources": ["src/Refund.php"]}]}), encoding="utf-8")
        done = self.plain("start", "--task-id", "feature/SHOP-12-plain", "--goal", "Date refunds", "--json",
                          cwd=folder)
        self.assertEqual(done.returncode, 0, done.stderr)
        done = self.plain("record-result", "--task-id", "feature/SHOP-12-plain", "--result-id", "plain-1",
                          "--revision", "auto", "--input", str(request), "--json", cwd=folder / "src")
        self.assertEqual(done.returncode, 0, done.stderr)
        tasks = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                 for path in (self.main / "project-brain" / "dynamic" / "tasks").glob("*.md")]
        self.assertIn("feature/SHOP-12-plain", [task["external_id"] for task in tasks])
        findings = (self.main / "project-brain" / "dynamic" / "findings").glob("*.md")
        self.assertTrue(any("Plain refunds keep the invoice date" in path.read_text(encoding="utf-8")
                            for path in findings))
        self.assertEqual(git(folder, "status", "--porcelain", "--ignored"), "")

    def test_a_learning_in_the_worktree_may_cite_the_installations_own_files(self):
        # Retrieval reads specs/ and AGENTS.md from the main checkout when
        # the worktree has none (resolve); a citation of them is checked there too.
        folder = self.scratch_worktree("14-cites")
        variables = {"ACCELERATOR_HOME": str(self.main), "ACCELERATOR_STATE_DIR": str(self.main),
                     "ACCELERATOR_PROJECT_DIR": str(folder)}
        request = Path(self.temporary.name) / "cites-result.json"
        request.write_text(json.dumps({
            "progress": "Specifications read.", "verified": True,
            "learnings": [{"type": "decision", "title": "Refund specifications live with the installation",
                           "consequence": "The manifest lists where the refund specification goes.",
                           "sources": ["specs/MANIFEST.md", "AGENTS.md"]}]}), encoding="utf-8")
        done = self.plain("start", "--task-id", "feature/SHOP-14-cites", "--goal", "Specify refunds", "--json",
                          cwd=folder, **variables)
        self.assertEqual(done.returncode, 0, done.stderr)
        done = self.plain("record-result", "--task-id", "feature/SHOP-14-cites", "--result-id", "cites-1",
                          "--revision", "auto", "--input", str(request), "--json", cwd=folder, **variables)
        self.assertEqual(done.returncode, 0, done.stderr)
        decisions = (self.main / "project-brain" / "dynamic" / "decisions").glob("*.md")
        self.assertTrue(any("Refund specifications live with the installation" in path.read_text(encoding="utf-8")
                            for path in decisions))
        self.assertEqual(git(folder, "status", "--porcelain", "--ignored"), "")

    def test_a_git_file_naming_a_worktree_git_does_not_know_is_not_served(self):
        # A .git file naming a worktree whose back-reference Git removed (or
        # never wrote) is no worktree of this installation.
        folder = Path(self.temporary.name).resolve() / "ghost"
        (folder / "src").mkdir(parents=True, exist_ok=True)
        (folder / ".git").write_text(f"gitdir: {(self.main / '.git' / 'worktrees' / 'ghost').as_posix()}\n",
                                     encoding="utf-8")
        self.assertEqual(self.layout(self.plain("status", "--json", cwd=folder / "src")), ("installed", self.main))

    @unittest.skipIf(WINDOWS or (hasattr(os, "geteuid") and os.geteuid() == 0),
                     "a folder without write permission stops neither Windows nor root")
    def test_a_lookup_the_sandbox_keeps_from_the_index_builds_one_of_its_own(self):
        # As a sandbox that lets a command write only the worktree - Codex's
        # workspace-write there - leaves the main checkout's state alone.
        folder = self.scratch_worktree("15-sandbox")
        # The task exists, so a lookup publishes what it delivered.
        done = self.plain("start", "--task-id", "feature/SHOP-15-sandbox", "--goal", "Look refunds up", "--json",
                          cwd=folder)
        self.assertEqual(done.returncode, 0, done.stderr)
        (self.main / "memory-bank" / "local" / "worktrees").mkdir(parents=True, exist_ok=True)
        for top in (self.main / "memory-bank", self.main / "project-brain"):
            for directory, _, files in os.walk(top):
                for place in (Path(directory), *(Path(directory) / name for name in files)):
                    mode = stat.S_IMODE(place.stat().st_mode)
                    place.chmod(mode & ~0o222)
                    self.addCleanup(place.chmod, mode)
        temporary = Path(self.temporary.name) / "sandbox-tmp"
        temporary.mkdir()
        done = self.plain("status", "--json", cwd=folder, TMPDIR=str(temporary))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(json.loads(done.stdout)["database"].startswith(str(temporary.resolve())))
        done = self.plain("retrieve", "saffron checkout", "--task-id", "feature/SHOP-15-sandbox", "--json",
                          cwd=folder, TMPDIR=str(temporary))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("README.md", done.stdout)
        # A write needs the state itself: not made somewhere else, it waits
        # for the session's next hook, which writes the state.
        done = self.plain("update", "--task-id", "feature/SHOP-15-sandbox", "--revision", "auto",
                          "--progress", "Nothing.", "--json", cwd=folder, TMPDIR=str(temporary))
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(json.loads(done.stdout)["queued"])
        self.assertEqual(1, len(list(temporary.glob("ai-accelerator-memory-queue-*/command-*.json"))))
        self.assertFalse(list(temporary.rglob("*.md")))

    def test_a_worktree_with_an_installation_of_its_own_is_not_borrowed(self):
        folder = self.scratch_worktree("13-own")
        own = folder / "memory-bank" / "scripts" / "context.py"
        own.parent.mkdir(parents=True)
        own.write_text("# This worktree's own runtime\n", encoding="utf-8")
        self.assertEqual(self.layout(self.plain("status", "--json", cwd=folder)), ("installed", self.main))

    def test_the_memory_mcp_server_of_a_worktree_session_serves_the_worktree(self):
        # A worktree inside the main checkout finds its MCP configuration
        # above it: Claude Code's bootstrap walks up from CLAUDE_PROJECT_DIR,
        # Codex's from the working directory, which is all Codex gives.
        server = json.loads((self.main / ".mcp.json").read_text(encoding="utf-8"))["mcpServers"]
        bootstrap = next(iter(server.values()))["args"][:-1]
        for host, extra in (("claude", {"CLAUDE_PROJECT_DIR": str(self.worktree)}), ("codex", {})):
            with self.subTest(host=host):
                self.serves_the_worktree([*bootstrap, host], dict(self.env, **extra))

    def serves_the_worktree(self, bootstrap, environment):
        messages = [
            {"jsonrpc": "2.0", "id": 0, "method": "initialize",
             "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": {"name": "test", "version": "1"}}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
             "params": {"name": "memory_status", "arguments": {}}},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
             "params": {"name": "memory_retrieve",
                        "arguments": {"task_id": "feature/SHOP-9-refunds", "query": "saffron checkout"}}},
        ]
        done = subprocess.run([sys.executable, *bootstrap],
                              input="".join(json.dumps(message) + "\n" for message in messages),
                              cwd=self.worktree, env=environment, capture_output=True, text=True, timeout=120)
        self.assertEqual(done.returncode, 0, done.stderr)
        responses = {item["id"]: item["result"] for item in map(json.loads, done.stdout.splitlines())}
        self.assertFalse(responses[1]["isError"], responses[1])
        workspace = responses[1]["structuredContent"]["workspace"]
        self.assertEqual((workspace["layout"], Path(workspace["project"])), ("shared", self.worktree))
        self.assertFalse(responses[2]["isError"], responses[2])
        capsule = json.dumps(responses[2]["structuredContent"])
        self.assertIn("README.md", capsule, "the worktree's own README is found")
        self.assertEqual(git(self.worktree, "status", "--porcelain", "--ignored"), "")


class AttachedWorktree(unittest.TestCase):
    """An attached project's worktree keeps an index of its own in the state.

    The Harness keeps one state for a project and all its checkouts. With one
    index between checkouts whose documents differ, a lookup that followed
    one in the other checkout rebuilt it: 580 ms against 250 ms on 400
    notes, at every switch."""

    def test_the_worktree_indexes_apart_and_the_main_checkout_as_before(self):
        temporary = tempfile.TemporaryDirectory(prefix="attached-worktree-")
        self.addCleanup(temporary.cleanup)
        base = Path(temporary.name).resolve()
        project, state = base / "shop", base / "state"
        (project / "docs").mkdir(parents=True)
        (project / "README.md").write_text("# Shop\n\nThe saffron checkout settles every order.\n",
                                           encoding="utf-8")
        git(project, "init", "-q", "-b", "main")
        git(project, "add", ".")
        git(project, "commit", "-qm", "Shop")
        worktree = base / "shop-refunds"
        git(project, "worktree", "add", "-q", str(worktree), "-b", "feature/refunds")
        edition = ROOT / "PHP Core"
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("CLAUDE", "ACCELERATOR_", "CONTEXT_", "GIT_"))}
        env.update(PYTHONDONTWRITEBYTECODE="1", ACCELERATOR_HOME=str(edition), ACCELERATOR_STATE_DIR=str(state))

        def database(folder):
            done = subprocess.run([sys.executable, str(edition / "memory-bank" / "scripts" / "context.py"),
                                   "status", "--json"], cwd=folder, env=dict(env, ACCELERATOR_PROJECT_DIR=str(folder)),
                                  capture_output=True, text=True, stdin=subprocess.DEVNULL)
            self.assertEqual(done.returncode, 0, done.stderr)
            return Path(json.loads(done.stdout)["database"])

        self.assertEqual(database(project), state / "memory-bank" / "local" / "context.db")
        own = database(worktree)
        self.assertEqual(own.parent.parent, state / "memory-bank" / "local" / "worktrees")
        self.assertEqual((own.parent / "worktree").read_text(encoding="utf-8"), os.path.realpath(worktree))
        self.assertEqual(git(worktree, "status", "--porcelain", "--ignored"), "")


    @unittest.skipIf(BASH is None, "no bash to run hooks with")
    def test_a_session_it_launched_that_entered_a_worktree_keeps_its_memory_there(self):
        # A session the Harness started in the project, with the edition lent
        # from this clone, calls EnterWorktree: the lent hooks keep running,
        # the launch's variables naming the project and the input's cwd the
        # worktree. The project's one state takes the worktree's own task.
        temporary = tempfile.TemporaryDirectory(prefix="attached-entered-")
        self.addCleanup(temporary.cleanup)
        base = Path(temporary.name).resolve()
        project, state = base / "shop", base / "state"
        (project / "src").mkdir(parents=True)
        (project / "src" / "Invoice.php").write_text("<?php\nfinal class Invoice {}\n", encoding="utf-8")
        git(project, "init", "-q", "-b", "feature/SHOP-7-invoices")
        git(project, "add", ".")
        git(project, "commit", "-qm", "Shop")
        entered = project / ".claude" / "worktrees" / "entered"
        git(project, "worktree", "add", "-q", str(entered), "-b", "worktree-entered")
        edition = ROOT / "PHP Core"
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("CLAUDE", "ACCELERATOR_", "CONTEXT_", "GIT_"))}
        # The first refresh in the worktree builds its index: as in SharedWorktree,
        # a runner's would be a timing test under the hooks' 5-second budget.
        env.update(PYTHONDONTWRITEBYTECODE="1", ACCELERATOR_HOME=str(edition), ACCELERATOR_STATE_DIR=str(state),
                   ACCELERATOR_PROJECT_DIR=str(project), CONTEXT_HOOK_BUDGET="60")

        def hook(name, payload, cwd):
            return subprocess.run([BASH, (edition / ".claude" / "hooks" / name).as_posix()],
                                  input=json.dumps(payload), cwd=cwd,
                                  env=dict(env, CLAUDE_PROJECT_DIR=str(project)),
                                  capture_output=True, text=True, timeout=120)

        for turn, (asked_in, title) in enumerate(((project, "Entering keeps the launched turn"),
                                                  (entered, "Launched sessions keep an entered worktree"))):
            with self.subTest(turn=turn):
                session = {"session_id": "attached-entered"}
                done = hook("working-memory-read.sh", dict(session, cwd=str(asked_in),
                            hook_event_name="UserPromptSubmit", prompt="How are invoices numbered?"), asked_in)
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertNotIn("Memory not consulted", done.stdout)
                if asked_in == entered:
                    self.assertRegex(done.stdout, re.compile(r"^working: (worktree-entered|not recorded yet)", re.M))
                    self.assertNotIn("SHOP-7", done.stdout)
                stop = hook("working-memory-write.sh", dict(session, cwd=str(entered), hook_event_name="Stop",
                            stop_hook_active=False, last_assistant_message="Checked."), entered)
                self.assertEqual(stop.returncode, 0, stop.stderr)
                decision = json.loads(stop.stdout)
                request = decision.get("hookSpecificOutput", {}).get("additionalContext", "")
                command = re.search(r"exact command: (cd [^\n]*?--json(?: && cd [^\n]*)?)\n", request)
                self.assertIsNotNone(command, request[:400])
                # It names the worktree, which the session's own variables do
                # not, and pipes the result without naming the descriptor.
                self.assertIn("--project " + shlex.quote(str(entered)), command.group(1))
                self.assertNotIn("--input -", command.group(1))
                # And it goes back there: the cd into the clone, which the launch
                # adds as a working directory, kept the session's shell in it.
                self.assertTrue(command.group(1).endswith(" && cd " + shlex.quote(str(entered))), command.group(1))
                result = json.dumps({"progress": f"{title}.", "verified": True, "learnings": [
                    {"type": "finding", "title": title,
                     "consequence": "A launched session that entered a worktree saves under its branch.",
                     "sources": ["src/Invoice.php"]}]})
                saved = subprocess.run([BASH, "-c", command.group(1).replace("'<JSON>'", shlex.quote(result))],
                                       cwd=entered, env=env, capture_output=True, text=True, timeout=120)
                self.assertEqual(saved.returncode, 0, saved.stderr + saved.stdout)
                done = hook("working-memory-write.sh", dict(session, cwd=str(entered), hook_event_name="Stop",
                            stop_hook_active=True, last_assistant_message="Saved."), entered)
                self.assertEqual(done.returncode, 0, done.stderr)
        tasks = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                 for path in (state / "project-brain" / "dynamic" / "tasks").glob("*.md")]
        self.assertTrue(any(task["external_id"] == "worktree-entered" and task.get("progress") for task in tasks),
                        [task["external_id"] for task in tasks])
        findings = [path.read_text(encoding="utf-8")
                    for path in (state / "project-brain" / "dynamic" / "findings").glob("*.md")]
        for title in ("Entering keeps the launched turn", "Launched sessions keep an entered worktree"):
            self.assertTrue(any(title in finding for finding in findings), title)
        # Nothing written into the project, the worktree Claude Code made aside.
        self.assertEqual(git(entered, "status", "--porcelain", "--ignored"), "")
        self.assertEqual([line for line in git(project, "status", "--porcelain", "--ignored",
                                               "--untracked-files=all").splitlines()
                          if not line[3:].strip('"').startswith(".claude/worktrees/")], [])
        # And the CLI takes only that project's checkouts.
        other = base / "elsewhere"
        other.mkdir()
        refused = subprocess.run([sys.executable, str(edition / "memory-bank" / "scripts" / "context.py"),
                                  "--project", str(other), "status", "--json"], cwd=entered, env=env,
                                 capture_output=True, text=True, stdin=subprocess.DEVNULL)
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn("--project names neither", refused.stderr)


class SpacedWorktree(unittest.TestCase):
    """A main checkout whose path has a space: the memory CLI the worktree's
    banner names runs as named, and the dispatcher's allow for the memory
    CLI takes it, so the session is not asked about it."""

    @unittest.skipIf(BASH is None, "no bash to run hooks with")
    def test_the_banners_cli_runs_and_is_allowed_where_the_path_has_a_space(self):
        temporary = tempfile.TemporaryDirectory(prefix="spaced-worktree-")
        self.addCleanup(temporary.cleanup)
        main = Path(temporary.name).resolve() / "my shop"
        (main / "src").mkdir(parents=True)
        (main / "composer.json").write_text('{"name": "acme/shop"}\n', encoding="utf-8")
        git(main, "init", "-q", "-b", "feature/SHOP-7-invoices")
        git(main, "add", ".")
        git(main, "commit", "-qm", "Shop")
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("CLAUDE", "ACCELERATOR_", "CONTEXT_", "GIT_"))}
        env["PYTHONDONTWRITEBYTECODE"] = "1"
        done = subprocess.run([sys.executable, str(ROOT / "scripts" / "install_accelerator.py"), "--edition",
                               "PHP Core", "--target", str(main), "--merge-existing"],
                              cwd=ROOT, env=env, capture_output=True, text=True, stdin=subprocess.DEVNULL)
        self.assertEqual(done.returncode, 0, done.stdout[-2000:] + done.stderr[-2000:])
        worktree = Path(temporary.name).resolve() / "my shop refunds"
        git(main, "worktree", "add", "-q", str(worktree), "-b", "feature/SHOP-9-refunds")
        shared = dict(env, ACCELERATOR_HOME=str(main), ACCELERATOR_STATE_DIR=str(main),
                      ACCELERATOR_PROJECT_DIR=str(worktree))
        payload = {"session_id": "spaced", "hook_event_name": "SessionStart", "source": "startup",
                   "cwd": str(worktree)}
        done = subprocess.run([BASH, (main / ".claude" / "hooks" / "local-context.sh").as_posix()],
                              input=json.dumps(payload), cwd=worktree, env=shared, capture_output=True,
                              text=True, timeout=120)
        self.assertEqual(done.returncode, 0, done.stderr)
        named = re.search(r"Memory CLI: this worktree has none - run the commands the policy and skills name with "
                          r"`([^`]+)`", done.stdout)
        self.assertIsNotNone(named, done.stdout)
        command = named.group(1) + " status --json"
        # As the agent runs it: in the worktree, without the variables.
        ran = subprocess.run([BASH, "-c", command], cwd=worktree, env=env, capture_output=True, text=True,
                             timeout=120)
        self.assertEqual(ran.returncode, 0, ran.stderr)
        workspace = json.loads(ran.stdout)["workspace"]
        self.assertEqual((workspace["layout"], Path(workspace["project"])), ("shared", worktree))
        spec = importlib.util.spec_from_file_location(
            "claude_dispatch_of_the_spaced_installation", main / "memory-bank" / "scripts" / "claude_dispatch.py")
        dispatch = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(dispatch)
        # The root as the hook's shell spells it, which the dispatcher is given.
        root = subprocess.run([BASH, "-c", 'cd "$1" && pwd', "_", str(main)], capture_output=True, text=True,
                              check=True).stdout.strip()
        self.assertTrue(dispatch.memory_command(command, root), command)


if __name__ == "__main__":
    unittest.main()
