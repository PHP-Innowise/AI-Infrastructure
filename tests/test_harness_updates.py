"""Updates of the clone, as a desktop application offers them: checked against a local remote, never the network."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness" / "src"))
from harness import updates  # noqa: E402
from harness.sessions import SessionError  # noqa: E402


def git(root, *args):
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update(GIT_AUTHOR_NAME="Fixture", GIT_AUTHOR_EMAIL="fixture@example.com",
                       GIT_COMMITTER_NAME="Fixture", GIT_COMMITTER_EMAIL="fixture@example.com")
    return subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True, text=True, env=environment).stdout.strip()


@unittest.skipUnless(shutil.which("git"), "needs git")
class UpdateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        self.remote, self.clone, self.other = root / "remote.git", root / "clone", root / "other"
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(self.remote)], check=True)
        subprocess.run(["git", "clone", "-q", str(self.remote), str(self.other)], check=True, capture_output=True)
        git(self.other, "checkout", "-q", "-b", "main")
        self.publish("README.md", "First\n", "Start the accelerator")
        subprocess.run(["git", "clone", "-q", str(self.remote), str(self.clone)], check=True, capture_output=True)
        self.updates = updates.Updates(self.clone)

    def publish(self, name, text, subject):
        (self.other / name).write_text(text, encoding="utf-8")
        git(self.other, "add", name)
        git(self.other, "commit", "-q", "-m", subject)
        git(self.other, "push", "-q", "origin", "main")

    def test_a_clone_that_has_everything_is_current(self):
        status = self.updates.check()
        self.assertEqual(("current", "main", "origin/main", 0), (status["state"], status["branch"], status["upstream"], status["behind"]))
        self.assertEqual(git(self.clone, "rev-parse", "--short", "HEAD"), status["version"])

    def test_new_commits_on_the_followed_branch_are_offered_and_applied_by_fast_forward(self):
        self.publish("Billing.php", "<?php\n", "Add billing")
        self.publish("Cart.php", "<?php\n", "Add the cart")
        status = self.updates.check()
        self.assertEqual(("available", 2), (status["state"], status["behind"]))
        self.assertEqual(["Add the cart", "Add billing"], [commit["subject"] for commit in status["commits"]])
        self.assertEqual("2 new changes on origin/main.", status["detail"])
        result = self.updates.apply()
        self.assertEqual((git(self.other, "rev-parse", "--short", "HEAD"), 2), (result["to"], result["changes"]))
        self.assertEqual(git(self.other, "rev-parse", "HEAD"), git(self.clone, "rev-parse", "HEAD"))
        self.assertTrue((self.clone / "Cart.php").is_file())
        self.assertEqual("updated", self.updates.status()["state"])
        self.updates.resume()  # what a server that does not restart does
        with self.assertRaisesRegex(SessionError, "already up to date"):
            self.updates.apply()

    def test_a_clone_with_commits_of_its_own_is_reported_and_left_alone(self):
        self.publish("Billing.php", "<?php\n", "Add billing")
        (self.clone / "Local.php").write_text("<?php\n", encoding="utf-8")
        git(self.clone, "add", "Local.php")
        git(self.clone, "commit", "-q", "-m", "A local change")
        before = git(self.clone, "rev-parse", "HEAD")
        status = self.updates.check()
        self.assertEqual(("diverged", 1, 1), (status["state"], status["ahead"], status["behind"]))
        self.assertIn("update it with Git", status["detail"])
        with self.assertRaisesRegex(SessionError, "commit of its own"):
            self.updates.apply()
        self.assertEqual(before, git(self.clone, "rev-parse", "HEAD"))

    def test_local_changes_in_the_way_fail_the_update_and_keep_the_clone(self):
        self.publish("README.md", "Second\n", "Rewrite the readme")
        (self.clone / "README.md").write_text("My notes\n", encoding="utf-8")
        before = git(self.clone, "rev-parse", "HEAD")
        with self.assertRaisesRegex(SessionError, "clone is unchanged"):
            self.updates.apply()
        self.assertEqual((before, "My notes\n"), (git(self.clone, "rev-parse", "HEAD"), (self.clone / "README.md").read_text()))
        self.assertEqual("failed", self.updates.status()["state"])

    def test_runs_in_progress_hold_the_update_back(self):
        self.publish("Billing.php", "<?php\n", "Add billing")
        released = []
        busy = updates.Updates(self.clone, hold=lambda: 2, release=lambda: released.append(True))
        with self.assertRaisesRegex(SessionError, "2 runs are in progress"):
            busy.apply()
        self.assertNotEqual(git(self.other, "rev-parse", "HEAD"), git(self.clone, "rev-parse", "HEAD"))
        self.assertEqual([], released)  # nothing was held, so nothing is let go

    def test_new_runs_stay_held_from_the_busy_check_until_the_restart_or_a_failure(self):
        calls = []
        held = updates.Updates(self.clone, hold=lambda: calls.append("hold") or 0, release=lambda: calls.append("release"))
        # Already current, diverged, or local changes in the way: the update fails and runs may start again.
        with self.assertRaises(updates.UpToDate):
            held.apply()
        self.assertEqual(["hold", "release"], calls)
        self.publish("README.md", "Second\n", "Rewrite the readme")
        (self.clone / "README.md").write_text("My notes\n", encoding="utf-8")
        with self.assertRaisesRegex(SessionError, "clone is unchanged"):
            held.apply()
        self.assertEqual(["hold", "release"] * 2, calls)
        (self.clone / "README.md").write_text("First\n", encoding="utf-8")
        # Applied: the hold outlives apply() for the restart the caller hands over to, and a second update is refused
        # without touching it.
        calls.clear()
        held.apply()
        self.assertEqual(["hold"], calls)
        with self.assertRaisesRegex(SessionError, "restarting"):
            held.apply()
        self.assertEqual(["hold"], calls)
        # A server that does not restart lets runs start again, once.
        held.resume()
        held.resume()
        self.assertEqual(["hold", "release"], calls)

    def test_a_clone_that_follows_nothing_or_cannot_reach_its_remote_says_why(self):
        git(self.clone, "checkout", "-q", "-b", "experiment")
        status = self.updates.check()
        self.assertEqual("unavailable", status["state"])
        self.assertIn("experiment follows no remote branch", status["detail"])
        git(self.clone, "checkout", "-q", "--detach")
        self.assertIn("not on a branch", self.updates.check()["detail"])
        # main follows origin's main even without the tracking configuration.
        git(self.clone, "checkout", "-q", "main")
        git(self.clone, "branch", "-q", "--unset-upstream")
        self.assertEqual(("current", "origin/main"), (self.updates.check()["state"], self.updates.status()["upstream"]))
        git(self.clone, "remote", "set-url", "origin", str(self.remote.parent / "missing.git"))
        status = self.updates.check()
        self.assertEqual("failed", status["state"])
        self.assertIn("Could not check origin/main", status["detail"])
        with tempfile.TemporaryDirectory() as plain:
            self.assertEqual("unavailable", updates.Updates(plain).check()["state"])

    def test_a_remote_whose_name_has_a_slash_is_checked_and_applied(self):
        # Git allows a slash in a remote's name. Split at its first slash, team/main/main named a remote team, which
        # does not exist, and a branch main/main, so every check failed.
        git(self.clone, "remote", "rename", "origin", "team/main")
        self.publish("Billing.php", "<?php\n", "Add billing")
        status = self.updates.check()
        self.assertEqual(("available", 1, "team/main/main"), (status["state"], status["behind"], status["upstream"]))
        self.assertEqual("1 new change on team/main/main.", status["detail"])
        self.updates.apply()
        self.assertEqual(git(self.other, "rev-parse", "HEAD"), git(self.clone, "rev-parse", "HEAD"))
        # A fast-forward still: a commit of the clone's own is reported and the clone left as it is.
        self.updates.resume()
        self.publish("Cart.php", "<?php\n", "Add the cart")
        (self.clone / "Local.php").write_text("<?php\n", encoding="utf-8")
        git(self.clone, "add", "Local.php")
        git(self.clone, "commit", "-q", "-m", "A local change")
        before = git(self.clone, "rev-parse", "HEAD")
        with self.assertRaisesRegex(SessionError, "commit of its own"):
            self.updates.apply()
        self.assertEqual(before, git(self.clone, "rev-parse", "HEAD"))

    def test_a_page_that_comes_back_checks_once_the_last_check_is_old(self):
        with patch.object(updates.Updates, "check") as check:
            self.updates.poke()
            self.updates.poke()
            time.sleep(.1)
        self.assertEqual(1, check.call_count)
        self.updates.checked = time.monotonic() - updates.PAGE_CHECK_SECONDS - 1
        with patch.object(updates.Updates, "check") as check:
            self.updates.poke()
            time.sleep(.1)
        self.assertEqual(1, check.call_count)

    def test_the_application_entry_is_rewritten_only_when_installed_from_this_clone(self):
        from harness import desktop_app
        with patch.object(desktop_app, "load_config", return_value={}), patch.object(desktop_app, "install") as install:
            self.assertEqual([], updates.refresh_application())
        install.assert_not_called()
        with patch.object(desktop_app, "load_config", return_value={"clone": str(desktop_app.ROOT)}), \
                patch.object(desktop_app, "installed_entry", return_value=Path("/x.desktop")), \
                patch.object(desktop_app, "install", return_value=[Path("/x.desktop")]):
            self.assertEqual(["/x.desktop"], updates.refresh_application())


if __name__ == "__main__":
    unittest.main()
