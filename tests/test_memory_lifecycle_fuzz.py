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
    # enters a worktree, bursts and a commit made while a save is written.
    SEEDS = (1, 9)

    def test_random_lifecycles_keep_every_save_in_its_branch_task(self):
        seen, events = set(), set()
        for seed in self.SEEDS:
            with self.subTest(seed=seed):
                done = subprocess.run([sys.executable, str(FUZZ), "--seed", str(seed), "--steps", "60"],
                                      cwd=ROOT, capture_output=True, text=True, timeout=900,
                                      stdin=subprocess.DEVNULL)
                self.assertEqual(0, done.returncode, done.stdout[-4000:] + done.stderr[-2000:])
                summary = json.loads(done.stdout)
                self.assertEqual([], summary["violations"])
                self.assertGreater(summary["saves"], 10)
                seen.update(name.split()[0] for name in summary["sessions"])
                events.update(summary["events"])
        self.assertEqual({"claude", "codex", "cursor"}, seen)
        self.assertLessEqual({"burst", "worktree entered", "commit during a save"}, events)


if __name__ == "__main__":
    unittest.main()
