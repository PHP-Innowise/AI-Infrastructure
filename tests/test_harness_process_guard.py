"""Real disposable processes exercise owner loss and process-group cleanup."""
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

GUARD = Path(__file__).resolve().parents[1] / "harness/src/harness/process_guard.py"


@unittest.skipUnless(hasattr(os, "killpg"), "The guard requires POSIX process groups")
class ProcessGuardTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="harness-guard-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.processes = []
        self.pipes = []
        self.addCleanup(self.cleanup_processes)

    def cleanup_processes(self):
        for descriptor in self.pipes:
            try:
                os.close(descriptor)
            except OSError:
                pass
        for process in self.processes:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=3)
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream is not None:
                    stream.close()

    def start_guard(self, code, *arguments):
        read_fd, write_fd = os.pipe()
        try:
            process = subprocess.Popen(
                [sys.executable, str(GUARD), str(read_fd), "--", sys.executable,
                 "-u", "-c", code, *arguments],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                pass_fds=(read_fd,), start_new_session=True,
            )
        finally:
            os.close(read_fd)
        self.processes.append(process)
        self.pipes.append(write_fd)
        return process, write_fd

    def wait_for(self, predicate, timeout=3):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(0.01)
        self.fail("Timed out waiting for a disposable native process")

    def close_owner_pipe(self, descriptor):
        self.pipes.remove(descriptor)
        os.close(descriptor)

    def running(self, pid):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        # A killed orphan can await reaping by the system's init process.
        status = Path(f"/proc/{pid}/stat")
        try:
            return status.read_text().rsplit(") ", 1)[1].split()[0] != "Z"
        except FileNotFoundError:
            return False

    def test_native_io_and_exit_status_pass_through_without_shell(self):
        code = "import sys; print(sys.stdin.read()); print(sys.argv[1], file=sys.stderr); sys.exit(int(sys.argv[2]))"
        literal = "$(false); --literal-value"
        for exit_code in (0, 7):
            with self.subTest(exit_code=exit_code):
                process, _ = self.start_guard(code, literal, str(exit_code))
                stdout, stderr = process.communicate(b"harmless stdin", timeout=3)
                self.assertEqual(process.returncode, exit_code)
                self.assertEqual(stdout, b"harmless stdin\n")
                self.assertEqual(stderr.decode().strip(), literal)

    def test_owner_loss_kills_stubborn_child_in_the_same_group(self):
        marker = self.root / "child.json"
        code = (
            "import json,os,pathlib,signal,sys,time; "
            "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
            "pathlib.Path(sys.argv[1]).write_text(json.dumps([os.getpid(),os.getpgrp()])); "
            "time.sleep(30)"
        )
        process, owner = self.start_guard(code, str(marker))
        self.wait_for(marker.exists)
        pid, group = json.loads(marker.read_text())
        self.assertEqual(group, process.pid)
        started = time.monotonic()
        self.close_owner_pipe(owner)
        process.wait(timeout=2)
        self.assertLess(time.monotonic() - started, 2)
        self.assertEqual(process.returncode, -signal.SIGKILL)
        self.wait_for(lambda: not self.running(pid))

    def test_cancel_kills_stubborn_descendant_after_direct_child_exits(self):
        marker = self.root / "descendant.pid"
        descendant = (
            "import os,pathlib,signal,sys,time; "
            "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
            "pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(30)"
        )
        code = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',sys.argv[1],sys.argv[2]]); time.sleep(30)"
        process, _ = self.start_guard(code, descendant, str(marker))
        self.wait_for(marker.exists)
        pid = int(marker.read_text())
        os.kill(process.pid, signal.SIGTERM)
        process.wait(timeout=2)
        self.assertEqual(process.returncode, -signal.SIGKILL)
        self.wait_for(lambda: not self.running(pid))

    @unittest.skipUnless(shutil.which("git"), "Checkout-filter regression requires Git")
    def test_worktree_timeout_kills_stubborn_checkout_filter(self):
        sys.path.insert(0, str(GUARD.parents[1]))
        try:
            from harness import sessions
        finally:
            sys.path.pop(0)
        project = self.root / "filter-project"
        project.mkdir()
        marker = self.root / "filter.json"
        helper = self.root / "filter.py"
        helper.write_text(
            "import json,os,pathlib,signal,time\n"
            "signal.signal(signal.SIGTERM,signal.SIG_IGN)\n"
            f"path=pathlib.Path({str(marker)!r})\n"
            "temporary=path.with_suffix('.tmp')\n"
            "temporary.write_text(json.dumps([os.getpid(),os.getpgrp()]))\n"
            "temporary.replace(path)\n"
            "time.sleep(30)\n"
        )
        environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                           GIT_TERMINAL_PROMPT="0")

        def git(*arguments):
            subprocess.run(
                ["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false",
                 "-C", str(project), *arguments], env=environment, check=True,
                stdin=subprocess.DEVNULL, capture_output=True, timeout=5,
            )

        git("init", "--quiet")
        git("config", "user.name", "Fixture")
        git("config", "user.email", "fixture@example.invalid")
        (project / ".gitattributes").write_text("tracked.txt filter=slow\n")
        (project / "tracked.txt").write_text("Owned checkout fixture\n")
        git("add", ".gitattributes", "tracked.txt")
        git("commit", "--quiet", "-m", "Initialize filter fixture")
        git("config", "filter.slow.smudge", shlex.join([sys.executable, str(helper)]))
        lock = os.open(self.root / "filter-runner.lock", os.O_CREAT | os.O_RDWR, 0o600)
        self.pipes.append(lock)
        try:
            with self.assertRaises(sessions.SessionError):
                sessions.run_git(
                    project, "worktree", "add", "-b", "codex/filter-fixture", "--",
                    str(self.root / "filter-worktree"), "HEAD", guard_lock=lock, timeout=1,
                )
            self.assertTrue(marker.exists(), "The real checkout filter must start before the timeout")
            pid, group = json.loads(marker.read_text())
            self.assertNotEqual(group, os.getpgrp())
            self.wait_for(lambda: not self.running(pid))
            self.assertFalse(self.running(group))
        finally:
            if marker.exists():
                _, group = json.loads(marker.read_text())
                if group != os.getpgrp():
                    try:
                        os.killpg(group, signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_immediate_sessions_restart_waits_for_crashed_owner_native(self):
        project = self.root / "project"
        project.mkdir()
        marker = self.root / "native.json"
        native = (
            "import json,os,pathlib,signal,sys,time; "
            "signal.signal(signal.SIGTERM,signal.SIG_IGN); sys.stdin.read(); "
            "path=pathlib.Path(sys.argv[1]); temporary=path.with_suffix('.tmp'); "
            "temporary.write_text(json.dumps([os.getpid(),os.getpgrp()])); "
            "temporary.replace(path); time.sleep(30)"
        )
        owner_code = r'''
import sys, time
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0, sys.argv[1])
from harness import sessions
root = Path(sys.argv[2])
with patch.object(sessions.providers, "discover_providers", return_value=[
        {"id": "codex", "available": True, "executable": sys.executable}]), \
     patch.object(sessions.providers, "build_command", return_value=[
        sys.executable, "-u", "-c", sys.argv[3], str(root / "native.json")]):
    manager = sessions.Sessions(root / "state", [root / "project"], timeout=30)
    result = manager.create({"project_id": next(iter(manager.projects)),
                             "provider": "codex", "prompt": "Disposable fake run"})
    (root / "session.id").write_text(result["id"])
    time.sleep(30)
'''
        owner = subprocess.Popen(
            [sys.executable, "-c", owner_code, str(GUARD.parents[1]), str(self.root), native],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
            start_new_session=True,
        )
        self.processes.append(owner)
        group = None
        try:
            self.wait_for(marker.exists)
            pid, group = json.loads(marker.read_text())
            self.assertNotEqual(group, owner.pid)
            self.assertTrue(self.running(pid))
            started = time.monotonic()
            os.kill(owner.pid, signal.SIGKILL)
            owner.wait(timeout=2)
            sys.path.insert(0, str(GUARD.parents[1]))
            try:
                from harness import sessions
            finally:
                sys.path.pop(0)
            with patch.object(sessions.providers, "discover_providers", return_value=[]):
                # No sleep or pre-restart polling: the inherited runner lock
                # must hold admission until the previous group is terminated.
                restarted = sessions.Sessions(self.root / "state", [project])
            try:
                # One-second guard grace plus scheduling slack.
                self.assertLess(time.monotonic() - started, 1.5)
                self.assertFalse(self.running(pid))
                self.assertFalse(self.running(group))
                sid = (self.root / "session.id").read_text()
                self.assertEqual(restarted.get(sid)["status"], "interrupted")
                self.assertTrue(any("restarted" in item.get("text", "")
                                    for item in restarted.events(sid)))
                self.assertFalse(self.running(pid))
            finally:
                restarted.close()
        finally:
            if group is not None:
                try:
                    os.killpg(group, signal.SIGKILL)
                except ProcessLookupError:
                    pass


if __name__ == "__main__":
    unittest.main()
