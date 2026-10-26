#!/usr/bin/env python3
"""Contract tests for authority-aware checkpoints."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Optional


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SKILL_PATHS = tuple(
    f"{tool}/skills/checkpoint/SKILL.md"
    for tool in (".agents", ".claude", ".cursor")
)
BASH = shutil.which("bash") or "/bin/bash"


class CheckpointIntegrationTest(unittest.TestCase):
    def test_tool_skills_are_byte_identical(self) -> None:
        contents = [
            REPOSITORY_ROOT.joinpath(path).read_bytes() for path in SKILL_PATHS
        ]
        self.assertEqual(contents[0], contents[1])
        self.assertEqual(contents[0], contents[2])

    def test_governed_mode_never_creates_duplicate_task_authority(self) -> None:
        """Governed mode records into the one branch task the hooks keep -
        flushing the buffered turns, then a revision-checked update - instead
        of declining, which left the skill contradicting the hooks."""
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(encoding="utf-8")
        self.assertIn("If its mode is\n   `governed`", skill)
        self.assertIn("context.py turn --task-id <task> --flush", skill)
        self.assertIn("--revision auto", skill)
        self.assertIn("`CONTEXT_TASK_ID`, otherwise", skill)
        self.assertIn("MUST NOT create a second task authority in governed mode", skill)
        self.assertLess(
            skill.index("## Authority Gate"),
            skill.index("## Governed Workflow"),
        )
        self.assertLess(
            skill.index("## Governed Workflow"),
            skill.index("## Lightweight Workflow"),
        )

    def test_lightweight_checkpoint_preserves_safety_and_provenance(self) -> None:
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(encoding="utf-8")
        required = (
            "--mode lightweight",
            "git status --porcelain=v1 -z --untracked-files=all",
            "git --literal-pathspecs diff --no-ext-diff --no-textconv --no-renames -- <safe-paths>",
            "git --literal-pathspecs diff --cached --no-ext-diff --no-textconv --no-renames -- <safe-paths>",
            "one `--file` argv value per exact changed path",
            "preserving leading and\n   trailing whitespace",
            "MUST NOT run unscoped `git diff`, `--stat`, or `--numstat`",
            "MUST NOT run `complete`, `record`, or `clear`",
        )
        for text in required:
            with self.subTest(text=text):
                self.assertIn(text, skill)

    def test_commands_accept_no_arguments(self) -> None:
        for tool in (".claude", ".cursor"):
            command = REPOSITORY_ROOT.joinpath(
                tool, "commands/checkpoint.md"
            ).read_text(encoding="utf-8")
            self.assertIn("accepts no arguments", command)
            self.assertIn(f"{tool}/skills/checkpoint/SKILL.md", command)


@unittest.skipUnless(shutil.which("git"), "Git")
class CheckpointTaskTest(unittest.TestCase):
    """The checkpoint records into the task the hooks name. It took the
    branch alone and stopped on a detached HEAD, where the hooks name the
    branch a rebase in progress rewrites, or codex-thread/<thread> in a
    worktree Codex made for a thread: there a checkpoint saved nothing.
    Each case runs the prompt hook, which hands its task to the runtime as
    --task-id, and resolves the skill's step 1 as its text says - including
    the task `context.py status --json` reports on a detached HEAD."""

    OWNER = "01a1776d-b888-7ac2-8a1b-9557bdbd1dad"
    CLI_RECORD = '{"version":1,"ownerThreadId":"%s"}' % OWNER
    APP_RECORD = '{\n  "version": 1,\n  "ownerThreadId": "%s"\n}\n' % OWNER

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="checkpoint task ")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.project = self.base / "project"
        self.project.mkdir()
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("GIT_", "CONTEXT_", "ACCELERATOR_"))}
        self.git("init", "-q", "-b", "feature/ledger")
        self.git("commit", "-q", "--allow-empty", "-m", "base")
        # The prompt hook, installed with a CLI that only records how it
        # was called.
        hooks = self.project / ".claude/hooks"
        hooks.mkdir(parents=True)
        shutil.copy(REPOSITORY_ROOT / ".claude/hooks/working-memory-read.sh", hooks)
        scripts = self.project / "memory-bank/scripts"
        scripts.mkdir(parents=True)
        self.calls = self.base / "argv.ndjson"
        (scripts / "context.py").write_text(
            "import json, pathlib, sys\n"
            f"with pathlib.Path({str(self.calls)!r}).open('a') as out:\n"
            "    out.write(json.dumps(sys.argv[1:]) + '\\n')\n", encoding="utf-8")

    def git(self, *arguments: str, where: Optional[Path] = None) -> str:
        return subprocess.run(["git", "-C", str(where or self.project), "-c", "user.email=c@example.invalid",
                               "-c", "user.name=C", "-c", "commit.gpgsign=false", *arguments],
                              check=True, capture_output=True, text=True, env=self.env).stdout.strip()

    def hooks_task(self, checkout: Path, explicit: Optional[str]) -> Optional[str]:
        """The task the prompt hook hands the runtime for `checkout`."""
        env = dict(self.env, **({"CONTEXT_TASK_ID": explicit} if explicit else {}))
        if checkout != self.project:
            # A worktree the project's installation serves: the dispatcher
            # names the roots, as for Claude Code and Codex there.
            env.update(ACCELERATOR_HOME=str(self.project), ACCELERATOR_STATE_DIR=str(self.project),
                       ACCELERATOR_PROJECT_DIR=str(checkout))
        self.calls.unlink(missing_ok=True)
        done = subprocess.run([BASH, str(self.project / ".claude/hooks/working-memory-read.sh")],
                              input=json.dumps({"session_id": "checkpoint", "cwd": str(checkout),
                                                "prompt": "Where do refunds stand?"}),
                              capture_output=True, text=True, cwd=str(checkout), env=env, timeout=60)
        self.assertEqual(0, done.returncode, done.stderr)
        calls = [json.loads(line) for line in self.calls.read_text(encoding="utf-8").splitlines()]
        [refresh] = [arguments for arguments in calls if arguments[0] == "refresh"]
        return refresh[refresh.index("--task-id") + 1] if "--task-id" in refresh else None

    def skill_task(self, checkout: Path, explicit: Optional[str]) -> tuple:
        """The Governed Workflow's step 1, as written: CONTEXT_TASK_ID, else
        the branch, else what status reports on a detached HEAD."""
        if explicit:
            return explicit, None
        branch = subprocess.run(["git", "-C", str(checkout), "symbolic-ref", "--quiet", "--short", "HEAD"],
                                capture_output=True, text=True, env=self.env)
        if branch.returncode == 0:
            return branch.stdout.strip(), None
        status = subprocess.run([sys.executable, str(REPOSITORY_ROOT / "memory-bank/scripts/context.py"),
                                 "--root", str(checkout), "status", "--json"],
                                capture_output=True, text=True, env=self.env, timeout=120)
        self.assertEqual(0, status.returncode, status.stderr)
        identity = json.loads(status.stdout)["automatic_memory"]["task_identity"]
        return identity["task_id"], identity["remediation"]

    def record(self, checkout: Path, text: str) -> None:
        directory = Path(self.git("rev-parse", "--absolute-git-dir", where=checkout))
        (directory / "codex-thread.json").write_text(text, encoding="utf-8")

    def rebase(self, checkout: Path, state: str, branch: str) -> None:
        directory = Path(self.git("rev-parse", "--absolute-git-dir", where=checkout))
        (directory / state).mkdir()
        (directory / state / "head-name").write_text(f"refs/heads/{branch}\n", encoding="utf-8")

    def test_the_skill_takes_a_detached_heads_task_from_status(self) -> None:
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(encoding="utf-8")
        governed = skill[skill.index("## Governed Workflow"):skill.index("## Lightweight Workflow")]
        step = governed[governed.index("1. "):governed.index("2. ")]
        for text in ("`CONTEXT_TASK_ID`, otherwise", "`git symbolic-ref --quiet --short HEAD`",
                     "On a detached HEAD", "`automatic_memory.task_identity.task_id`",
                     "`python3 memory-bank/scripts/context.py status --json`", "codex-thread/<thread>",
                     "rather than inventing an ID"):
            with self.subTest(text=text):
                self.assertIn(" ".join(text.split()), " ".join(step.split()))
        # The lightweight checkpoint resolves its task the same way.
        lightweight = skill[skill.index("## Lightweight Workflow"):skill.index("## Safety")]
        self.assertIn("as step 1 of the Governed Workflow does", " ".join(lightweight.split()))
        self.assertNotIn("detached-HEAD error", skill)

    def test_the_skill_names_the_task_the_hooks_name(self) -> None:
        thread = "codex-thread/" + self.OWNER
        worktree = self.base / "codex-home/worktrees/b3c4/project"
        cases = (
            ("branch", lambda: None, self.project, None, "feature/ledger"),
            ("branch beside a record", lambda: self.record(self.project, self.CLI_RECORD), self.project,
             None, "feature/ledger"),
            ("CONTEXT_TASK_ID", lambda: self.git("checkout", "-q", "--detach"), self.project, "TASK-9", "TASK-9"),
            ("rebase-merge", lambda: self.rebase(self.project, "rebase-merge", "feature/refunds"), self.project,
             None, "feature/refunds"),
            ("rebase-apply", lambda: self.rebase(self.project, "rebase-apply", "feature/refunds"), self.project,
             None, "feature/refunds"),
            ("a thread's record, the CLI's", lambda: self.record(self.project, self.CLI_RECORD), self.project,
             None, thread),
            ("a thread's record, the app's", lambda: self.record(self.project, self.APP_RECORD), self.project,
             None, thread),
            ("a rebase beside a thread's record", lambda: (self.record(self.project, self.CLI_RECORD),
                                                           self.rebase(self.project, "rebase-merge", "codex/refunds")),
             self.project, None, "codex/refunds"),
            ("a worktree Codex made", lambda: (self.git("worktree", "add", "-q", "--detach", str(worktree)),
                                              self.record(worktree, self.APP_RECORD)), worktree, None, thread),
        )
        for label, arrange, checkout, explicit, task in cases:
            with self.subTest(case=label):
                self.git("checkout", "-q", "--detach", "feature/ledger")
                if label.startswith("branch"):
                    self.git("checkout", "-q", "feature/ledger")
                try:
                    arrange()
                    self.assertEqual(task, self.hooks_task(checkout, explicit))
                    self.assertEqual((task, None), self.skill_task(checkout, explicit))
                finally:
                    for place in (self.project, *([worktree] if worktree.exists() else [])):
                        directory = Path(self.git("rev-parse", "--absolute-git-dir", where=place))
                        (directory / "codex-thread.json").unlink(missing_ok=True)
                        for state in ("rebase-merge", "rebase-apply"):
                            shutil.rmtree(directory / state, ignore_errors=True)

    def test_any_other_detached_head_stops_with_the_remediation(self) -> None:
        self.git("checkout", "-q", "--detach")
        self.assertIsNone(self.hooks_task(self.project, None))
        task, remediation = self.skill_task(self.project, None)
        self.assertIsNone(task)
        self.assertIn("CONTEXT_TASK_ID", remediation)


if __name__ == "__main__":
    unittest.main()
