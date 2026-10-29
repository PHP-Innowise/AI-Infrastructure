"""scripts/memory_lifecycle_fuzz.py: random session and Git lifecycles, driven
through a fresh installation's real hooks, keep what the memory promises."""
import json
import os
import shutil
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FUZZ = ROOT / "scripts" / "memory_lifecycle_fuzz.py"


@unittest.skipIf(os.name == "nt", "the hooks run under Git Bash there; the Windows memory suites cover them")
@unittest.skipUnless(shutil.which("bash") and shutil.which("git"), "needs bash and git")
class LifecycleFuzz(unittest.TestCase):
    # Seeds whose 60 steps cover, between them, every host, a session that
    # enters a worktree, bursts, a commit made while a save is written and a
    # rebase stopped on a conflict, then continued and aborted, Codex in a
    # worktree, a worktree Codex made, and checkpoints a sandbox held until
    # the session's Stop, and - where bubblewrap is - saves a sandbox held
    # until the Stop after them. On the runtime before saves followed the task's
    # current revision and the branch rather than the commit, seed 1 lost
    # saves to the first and seed 2 to the second.
    SEEDS = (1, 2)
    # Launched as the Harness launches them: Claude Code and Codex in the
    # root and in worktrees, a root session entering one, two at once in
    # one folder, the state apart.
    ATTACHED_SEED = 9
    # A third of its turns with one step stopped part-way, as a host stops a
    # hook at its timeout, a closed window or Ctrl+C: its 60 steps stop a
    # prompt hook, a Stop hook, a save and the Stop after a save, by SIGKILL
    # to the hook's process group, SIGTERM to its shell and SIGINT, Cursor's
    # and Codex's hooks among them.
    INTERRUPTED_SEED = 6

    def fuzz(self, *arguments):
        done = subprocess.run([sys.executable, str(FUZZ), *arguments, "--steps", "60"], cwd=ROOT,
                              capture_output=True, text=True, timeout=900, stdin=subprocess.DEVNULL)
        self.assertEqual(0, done.returncode, done.stdout[-4000:] + done.stderr[-2000:])
        summary = json.loads(done.stdout)
        self.assertEqual([], summary["violations"])
        self.assertGreater(summary["saves"], 10)
        return summary

    def test_random_lifecycles_keep_every_save_in_its_branch_task(self):
        seen, sessions, events = set(), set(), set()
        for seed in self.SEEDS:
            with self.subTest(seed=seed):
                summary = self.fuzz("--seed", str(seed))
                seen.update(name.split()[0] for name in summary["sessions"])
                sessions.update(summary["sessions"])
                events.update(summary["events"])
        self.assertEqual({"claude", "codex", "cursor"}, seen)
        self.assertIn("codex worktree", sessions)
        self.assertLessEqual({"burst", "worktree entered", "commit during a save", "rebase stopped",
                              "rebase continued", "rebase aborted", "codex worktree made",
                              "checkpoint held"}, events)
        if shutil.which("bwrap"):
            # A save the sandbox held, made by the Stop after it.
            self.assertIn("save held", events)

    def test_hooks_and_saves_stopped_part_way_break_nothing_after_them(self):
        summary = self.fuzz("--seed", str(self.INTERRUPTED_SEED), "--interrupt", "0.3")
        events = summary["events"]
        self.assertLessEqual({"prompt hook stopped", "Stop hook stopped", "save stopped",
                              "Stop after the save stopped", "stopped by kill", "stopped by term",
                              "stopped by int"}, set(events))
        # A stop that lands after the step ended stops nothing; most land
        # while it runs.
        self.assertGreater(events.get("stopped while running", 0), 0)
        self.assertLessEqual({"claude", "codex", "cursor"}, {name.split()[0] for name in summary["sessions"]})

    def test_attached_sessions_keep_the_project_and_the_clone_unwritten(self):
        summary = self.fuzz("--attached", "--seed", str(self.ATTACHED_SEED))
        self.assertLessEqual({"claude root", "claude worktree", "codex root", "codex worktree"},
                             set(summary["sessions"]))
        # Which of two sessions answering at once saves first is the
        # machine's: how many saves were rebased is not asserted.
        self.assertGreater(summary["events"].get("burst", 0), 0)
        self.assertGreater(summary["events"].get("worktree entered", 0), 0)


if __name__ == "__main__":
    unittest.main()
