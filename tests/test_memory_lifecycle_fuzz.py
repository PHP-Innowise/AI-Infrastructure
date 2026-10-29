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
    # rebase stopped on a conflict, then continued and aborted. On the
    # runtime before saves followed the task's current revision and the
    # branch rather than the commit, seed 1 lost saves to the first and
    # seed 2 to the second.
    SEEDS = (1, 2)
    # Launched as the Harness launches them: Claude Code and Codex in the
    # root and in worktrees, a root session entering one, two at once in
    # one folder, the state apart.
    ATTACHED_SEED = 4

    def fuzz(self, *arguments):
        done = subprocess.run([sys.executable, str(FUZZ), *arguments, "--steps", "60"], cwd=ROOT,
                              capture_output=True, text=True, timeout=900, stdin=subprocess.DEVNULL)
        self.assertEqual(0, done.returncode, done.stdout[-4000:] + done.stderr[-2000:])
        summary = json.loads(done.stdout)
        self.assertEqual([], summary["violations"])
        self.assertGreater(summary["saves"], 10)
        return summary

    def test_random_lifecycles_keep_every_save_in_its_branch_task(self):
        seen, events = set(), set()
        for seed in self.SEEDS:
            with self.subTest(seed=seed):
                summary = self.fuzz("--seed", str(seed))
                seen.update(name.split()[0] for name in summary["sessions"])
                events.update(summary["events"])
        self.assertEqual({"claude", "codex", "cursor"}, seen)
        self.assertLessEqual({"burst", "worktree entered", "commit during a save", "rebase stopped",
                              "rebase continued", "rebase aborted"}, events)

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
