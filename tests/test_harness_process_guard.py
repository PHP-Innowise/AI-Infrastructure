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
# What process_runtime.launch_guarded starts: the guard from its cached bytecode.
GUARD_CLI = GUARD.with_name("process_guard_cli.py")
# The guard as if posix (whose functions the POSIX guard calls; os takes them
# from it) lacked the named ones. Without pidfd_open it waits as on macOS or a
# kernel before 5.3 (SIGCHLD wakes it, not a pidfd); without posix_spawn it
# starts its native process through Popen, as where posix_spawn would not
# report a failed exec.
WITHOUT = ("import os,posix,runpy,sys; "
           "[module.__dict__.pop(name, None) for name in sys.argv.pop(1).split(',') for module in (os, posix)]; "
           "sys.argv[0] = sys.argv.pop(1); runpy.run_path(sys.argv[0], run_name='__main__')")
UNAVAILABLE = b"Harness process guard could not start the native process.\n"


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

    def start_native(self, native, *, pidfd=True, spawn=True, lock=None, inherited=(), script=GUARD_CLI, **options):
        read_fd, write_fd = os.pipe()
        missing = [name for name, present in (("pidfd_open", pidfd), ("posix_spawn", spawn)) if not present]
        # Without site and through its launcher, as process_runtime.launch_guarded starts it.
        guard = [sys.executable, "-S", *(["-c", WITHOUT, ",".join(missing)] if missing else []), str(script), str(read_fd)]
        if lock is not None:
            guard.extend(["--lock-fd", str(lock)])
        try:
            process = subprocess.Popen(
                [*guard, "--", *native],
                **{"stdin": subprocess.PIPE, "stdout": subprocess.PIPE, "stderr": subprocess.PIPE, **options},
                pass_fds=(read_fd, *inherited, *(() if lock is None else (lock,))), start_new_session=True,
            )
        finally:
            os.close(read_fd)
        self.processes.append(process)
        self.pipes.append(write_fd)
        return process, write_fd

    def start_guard(self, code, *arguments, pidfd=True):
        return self.start_native([sys.executable, "-u", "-c", code, *arguments], pidfd=pidfd)

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
        # Reaped between the open and the read, its stat file answers ESRCH
        # (ProcessLookupError), not a missing file: either way it is gone.
        status = Path(f"/proc/{pid}/stat")
        try:
            return status.read_text().rsplit(") ", 1)[1].split()[0] != "Z"
        except (FileNotFoundError, ProcessLookupError):
            return False

    def test_native_io_and_exit_status_pass_through_without_shell(self):
        code = "import sys; print(sys.stdin.read()); print(sys.argv[1], file=sys.stderr); sys.exit(int(sys.argv[2]))"
        literal = "$(false); --literal-value"
        for pidfd in (True, False):
            for exit_code in (0, 7):
                with self.subTest(pidfd=pidfd, exit_code=exit_code):
                    process, _ = self.start_guard(code, literal, str(exit_code), pidfd=pidfd)
                    stdout, stderr = process.communicate(b"harmless stdin", timeout=3)
                    self.assertEqual(process.returncode, exit_code)
                    self.assertEqual(stdout, b"harmless stdin\n")
                    self.assertEqual(stderr.decode().strip(), literal)

    def test_start_failure_exits_127_with_its_message(self):
        # On POSIX the guard leaves through os._exit, which skips the
        # interpreter's final flush; its one private-safe line must still arrive.
        for spawn in (True, False):
            for native in (str(self.root / "missing-native"), "harness-missing-native"):
                with self.subTest(spawn=spawn, native=native):
                    process, _ = self.start_native([native, "--private-value"], spawn=spawn,
                                                   stdin=subprocess.DEVNULL)
                    stdout, stderr = process.communicate(timeout=3)
                    self.assertEqual((process.returncode, stdout, stderr), (127, b"", UNAVAILABLE))

    def test_path_search_and_its_failures_match_popen(self):
        # On glibc posix_spawnp gives up at the first PATH entry it cannot run,
        # and on macOS it hands a file without #! to /bin/sh. Popen moves past
        # both and never runs a shell; the guard searches as Popen does.
        directories = {}
        for name, body, mode in (("plain", "echo plain\n", 0o755),  # No #!: ENOEXEC.
                                 ("denied", "#!/bin/sh\necho denied\n", 0o644),  # EACCES.
                                 ("found", "#!/bin/sh\necho found\n", 0o755),
                                 ("tööls", "#!/bin/sh\necho unicode\n", 0o755)):
            directories[name] = self.root / name
            directories[name].mkdir()
            tool = directories[name] / "harness-tool"
            tool.write_text(body)
            tool.chmod(mode)
        (directories["tööls"] / "harness-tool").rename(directories["tööls"] / "harness-tööl")
        directories["folder"] = self.root / "folder"
        (directories["folder"] / "harness-tool").mkdir(parents=True)  # A directory: EACCES.

        def search(*names):
            return os.pathsep.join([str(self.root / "missing"), *(str(directories[name]) for name in names)])

        unavailable = (127, b"", UNAVAILABLE)
        cases = (
            (["harness-tool"], search("plain", "denied", "folder", "found"), None, (0, b"found\n", b"")),
            (["harness-tool"], search("plain", "denied", "folder"), None, unavailable),
            ([str(directories["plain"] / "harness-tool")], search("found"), None, unavailable),
            (["harness-tool"], "", directories["found"], (0, b"found\n", b"")),  # An empty entry: the working directory.
            (["sh", "-c", "echo default"], None, None, (0, b"default\n", b"")),  # No PATH: os.defpath.
            (["harness-tööl"], search("tööls"), None, (0, b"unicode\n", b"")),  # Searched in bytes, as Popen does.
        )
        for native, path, cwd, expected in cases:
            environment = {key: value for key, value in os.environ.items() if key != "PATH"}
            if path is not None:
                environment["PATH"] = path
            with self.subTest(native=native, path=path, popen=True):
                try:
                    done = subprocess.run(native, env=environment, cwd=cwd, stdin=subprocess.DEVNULL,
                                          capture_output=True, timeout=5)
                    self.assertEqual((done.returncode, done.stdout, done.stderr), expected)
                except OSError:
                    self.assertEqual(unavailable, expected)
            for spawn in (True, False):
                with self.subTest(native=native, path=path, spawn=spawn):
                    process, _ = self.start_native(native, spawn=spawn, env=environment, cwd=cwd,
                                                   stdin=subprocess.DEVNULL)
                    stdout, stderr = process.communicate(timeout=3)
                    self.assertEqual((process.returncode, stdout, stderr), expected)

    def test_native_inherits_only_standard_streams_and_the_lock(self):
        # Popen(close_fds=True, pass_fds=(lock,)) is the contract. posix_spawn
        # closes only close-on-exec descriptors, so the guard marks every other
        # one it holds; the stray one arrives inheritable, as from a careless owner.
        listing = ("import os\n"
                   "def is_open(fd):\n"
                   "    try:\n"
                   "        os.fstat(fd)\n"
                   "    except OSError:\n"
                   "        return False\n"
                   "    return True\n"
                   "print(*sorted(fd for fd in map(int, os.listdir('/dev/fd')) if is_open(fd)))\n")
        lock = os.open(self.root / "runner.lock", os.O_CREAT | os.O_RDWR, 0o600)
        stray = os.open(os.devnull, os.O_RDONLY)
        self.pipes.extend((lock, stray))
        for spawn in (True, False):
            with self.subTest(spawn=spawn):
                process, _ = self.start_native([sys.executable, "-S", "-c", listing], spawn=spawn,
                                               lock=lock, inherited=(stray,))
                stdout, stderr = process.communicate(timeout=3)
                self.assertEqual((process.returncode, stderr), (0, b""))
                self.assertEqual(stdout.split(), [b"0", b"1", b"2", str(lock).encode()])

    @unittest.skipUnless(Path("/proc/self/status").exists(), "Signal dispositions need /proc")
    def test_native_starts_with_the_signals_python_ignores_at_default(self):
        # The guard is Python, which ignores SIGPIPE and SIGXFSZ; as Popen's
        # restore_signals does, its native process gets them at default (so a
        # closed pipe still ends `yes | head`). cat reports its own SigIgn mask.
        restored = 1 << (signal.SIGPIPE - 1) | 1 << (signal.SIGXFSZ - 1)
        for spawn in (True, False):
            with self.subTest(spawn=spawn):
                process, _ = self.start_native(["cat", "/proc/self/status"], spawn=spawn)
                stdout, stderr = process.communicate(timeout=3)
                self.assertEqual((process.returncode, stderr), (0, b""))
                ignored = next(line for line in stdout.decode().splitlines() if line.startswith("SigIgn:"))
                self.assertEqual(int(ignored.split()[1], 16) & restored, 0)

    def test_posix_guard_imports_no_windows_modules_and_no_subprocess(self):
        # Every guarded call waits for the guard to start and to exit; on POSIX
        # the Windows-only ctypes cost ~2 ms of that and subprocess with
        # selectors ~5 ms. Popen stays only where posix_spawn would not do.
        # Run in-process, the lock is not inheritable when the guard gets it;
        # as with pass_fds, the native process must still hold it. Counted
        # are the guard's own imports: on Python 3.11 runpy brings threading.
        code = ("import os,runpy,sys; before = set(sys.modules); guard = runpy.run_path(sys.argv[1]); "
                "read_fd, write_fd = os.pipe(); lock = os.open(sys.argv[2], os.O_CREAT | os.O_RDWR, 0o600); "
                "status = guard['run'](read_fd, [sys.executable, '-S', '-c', 'import os,sys; os.fstat(int(sys.argv[1]))', "
                "str(lock)], lock); "
                "print(status, guard['_spawn_reports_exec_errors'](), "
                "*sorted({'ctypes', 'selectors', 'subprocess', 'threading'} & (set(sys.modules) - before)))")
        imported = subprocess.run([sys.executable, "-S", "-c", code, str(GUARD), str(self.root / "runner.lock")],
                                  stdin=subprocess.DEVNULL, capture_output=True, timeout=10, check=True,
                                  start_new_session=True)
        status, direct, *modules = imported.stdout.split()
        self.assertEqual(status, b"0")
        self.assertNotIn(b"ctypes", modules)
        if direct == b"True":
            self.assertEqual(modules, [])

    def test_posix_guard_start_imports_nothing_it_can_do_without(self):
        # os.py with what it imports (~1.3 ms), signal's enums (~1.6 ms) and
        # __future__ were part of every guarded call's wait for the guard;
        # the launcher must not bring importlib (~0.1-0.5 ms) either.
        # PYTHONPROFILEIMPORTTIME lists the guard's imports; the native Python
        # ignores it under -E. Warning options and dev mode import warnings
        # at any start.
        environment = {key: value for key, value in os.environ.items()
                       if key not in ("PYTHONWARNINGS", "PYTHONDEVMODE")}
        environment["PYTHONPROFILEIMPORTTIME"] = "1"
        process, _ = self.start_native([sys.executable, "-E", "-c", ""], stdin=subprocess.DEVNULL, env=environment)
        stdout, stderr = process.communicate(timeout=5)
        self.assertEqual((process.returncode, stdout), (0, b""), stderr)
        imported = {line.rpartition("|")[2].strip() for line in stderr.decode().splitlines()
                    if line.startswith("import time:")}
        self.assertIn("select", imported)
        # Where Popen starts the native process (posix_spawn would not report
        # a failed exec there), subprocess imports all of them anyway.
        if "subprocess" not in imported:
            self.assertEqual(sorted(imported & {"os", "signal", "enum", "__future__", "importlib", "warnings",
                                                "selectors", "threading", "ctypes"}), [])

    def test_launcher_runs_the_guard_from_its_bytecode_cache(self):
        # Python compiles a file run as a script from source each time, ~2 ms
        # of every guarded call for the guard. Through its launcher the guard
        # is cached once, even under PYTHONDONTWRITEBYTECODE, which the Harness
        # and Claude Desktop set, and nothing else is cached.
        copy = self.root / "guard"
        copy.mkdir()
        for path in (GUARD, GUARD_CLI):
            shutil.copy2(path, copy / path.name)
        source, launcher = copy / GUARD.name, copy / GUARD_CLI.name
        environment = {key: value for key, value in os.environ.items() if key != "PYTHONPYCACHEPREFIX"}
        environment["PYTHONDONTWRITEBYTECODE"] = "1"

        def start_failure(script):
            process, _ = self.start_native([str(self.root / "missing-native")], script=script,
                                           env=environment, stdin=subprocess.DEVNULL)
            stdout, stderr = process.communicate(timeout=3)
            self.assertEqual((process.returncode, stdout), (127, b""))
            return stderr

        self.assertEqual(start_failure(source), UNAVAILABLE)
        self.assertEqual(list(copy.rglob("*.pyc")), [])
        self.assertEqual(start_failure(launcher), UNAVAILABLE)
        cached = [path.relative_to(copy).as_posix() for path in copy.rglob("*.pyc")]
        self.assertEqual([path.split(".", 1)[0] for path in cached], ["__pycache__/process_guard"], cached)
        # Other text of the same size and modification time: only the cached
        # guard still prints the line it was compiled with.
        status = source.stat()
        source.write_bytes(source.read_bytes().replace(b"could not start", b"COULD NOT START"))
        os.utime(source, ns=(status.st_atime_ns, status.st_mtime_ns))
        self.assertEqual(start_failure(launcher), UNAVAILABLE)
        self.assertEqual(start_failure(source), UNAVAILABLE.replace(b"could not start", b"COULD NOT START"))

    def test_native_signal_death_reports_shell_status(self):
        for pidfd in (True, False):
            with self.subTest(pidfd=pidfd):
                process, _ = self.start_guard("import os,signal; os.kill(os.getpid(), signal.SIGKILL)", pidfd=pidfd)
                process.communicate(timeout=3)
                self.assertEqual(process.returncode, 128 + signal.SIGKILL)

    @unittest.skipUnless(Path("/proc/self/status").exists(), "Wakeup counts need /proc")
    def test_guard_sleeps_until_an_event_instead_of_a_poll_tick(self):
        # The owner reads EOF only once the guard exits, so a 50 ms poll tick
        # delayed every guarded call after its native exit. Wall time varies
        # with load; the guard's wakeups while its child sleeps do not.
        def wakeups(pid):
            for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                if line.startswith("voluntary_ctxt_switches:"):
                    return int(line.split()[1])

        code = "import sys,time; print('ready', flush=True); time.sleep(float(sys.argv[1]))"
        for pidfd in (True, False):
            with self.subTest(pidfd=pidfd):
                process, _ = self.start_guard(code, "0.6", pidfd=pidfd)
                self.assertEqual(process.stdout.readline(), b"ready\n")
                before = wakeups(process.pid)
                time.sleep(0.4)
                self.assertLessEqual(wakeups(process.pid) - before, 2)
                process.communicate(timeout=3)
                self.assertEqual(process.returncode, 0)

    def test_owner_loss_kills_stubborn_child_in_the_same_group(self):
        code = (
            "import json,os,pathlib,signal,sys,time; "
            "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
            "pathlib.Path(sys.argv[1]).write_text(json.dumps([os.getpid(),os.getpgrp()])); "
            "time.sleep(30)"
        )
        for pidfd in (True, False):
            with self.subTest(pidfd=pidfd):
                marker = self.root / f"child-{pidfd}.json"
                process, owner = self.start_guard(code, str(marker), pidfd=pidfd)
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
        descendant = (
            "import os,pathlib,signal,sys,time; "
            "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
            "pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(30)"
        )
        code = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',sys.argv[1],sys.argv[2]]); time.sleep(30)"
        for pidfd in (True, False):
            with self.subTest(pidfd=pidfd):
                marker = self.root / f"descendant-{pidfd}.pid"
                process, _ = self.start_guard(code, descendant, str(marker), pidfd=pidfd)
                self.wait_for(marker.exists)
                pid = int(marker.read_text())
                started = time.monotonic()
                os.kill(process.pid, signal.SIGTERM)
                process.wait(timeout=2)
                # The direct child exits on SIGTERM; the guard still waits out
                # the grace for the descendant that ignores it.
                self.assertGreaterEqual(time.monotonic() - started, 0.9)
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
