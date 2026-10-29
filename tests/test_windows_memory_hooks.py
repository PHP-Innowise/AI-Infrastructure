"""The memory hooks on native Windows, started the way Claude Code starts them.

Claude Code on native Windows runs each hook command with Git Bash
(`<Git>\\bin\\bash.exe -c <command>`). The python.org installer gives that
shell `python` and `py`, but no `python3`, so every memory hook resolves its
interpreter: python, then py -3, then python3, and it remembers the one it
found in ai-accelerator-python-<uid> in TMPDIR. PythonResolutionTest in
PHP Core/memory-bank/tests/test_hooks.py simulates that on POSIX, with
OS=Windows_NT and shell stand-ins. The parts only Windows has are checked
here: Git Bash's path conversion and hash table, a real python.exe, and an
edition installed under a Windows path. The windows-latest runner runs this
module; elsewhere it skips.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "scripts" / "install_accelerator.py"
TASK = "feature/invoice-export"
GOAL = "Invoice export to CSV"
WORKING = f"working: {TASK} — {GOAL}"
PROMPT = {"prompt": "How does the invoice export work?"}
# A runner is slower than a laptop, and the first refresh builds the index.
# Under the hooks' own 5-second budget this would be a timing test.
BUDGET = "60"
HOOK_TIMEOUT = 180
# Every name the hooks could take Python by, in every spelling that Git Bash
# or Windows finds a program by.
PYTHON_NAMES = tuple(name + suffix for name in ("python", "python3", "py")
                     for suffix in ("", ".exe", ".com", ".bat", ".cmd"))
# jq and php, which the validators decode a call with before Python.
JSON_TOOLS = tuple(name + suffix for name in ("jq", "php")
                   for suffix in ("", ".exe", ".bat", ".cmd"))
# A remembered-interpreter file this old was not written by a later hook run;
# a hook that writes it again gives it that run's time.
LONG_AGO = 1_000_000_000


def git_bash() -> str | None:
    """Git for Windows' bin\\bash.exe, the shell Claude Code starts hooks with.

    CLAUDE_CODE_GIT_BASH_PATH names it for Claude Code and is honoured here.
    Otherwise it is found beside the git on PATH. A bare `bash` on PATH can be
    WSL's launcher in System32, which is not the shell the hooks run in.
    """
    configured = os.environ.get("CLAUDE_CODE_GIT_BASH_PATH")
    if configured:
        return configured if Path(configured).is_file() else None
    git = shutil.which("git")
    for folder in Path(git).resolve().parents if git else ():
        if (folder / "bin" / "bash.exe").is_file():
            return str(folder / "bin" / "bash.exe")
    return None


def without_python(path: str) -> list[str]:
    """PATH's folders, every folder that holds a Python left out."""
    return [folder for folder in path.split(os.pathsep)
            if folder and not any(os.path.lexists(os.path.join(folder, name)) for name in PYTHON_NAMES)]


@unittest.skipUnless(os.name == "nt", "native Windows: Git Bash and a Python without python3")
class WindowsMemoryHooksTest(unittest.TestCase):
    """A session's three memory hooks against PHP Core installed into a project."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.bash = git_bash()
        # TemporaryDirectory also removes Git's read-only object files.
        shared = tempfile.TemporaryDirectory(prefix="windows-hooks-")
        cls.addClassCleanup(shared.cleanup)
        cls.shared = Path(shared.name).resolve()

    def setUp(self) -> None:
        self.assertIsNotNone(self.bash, "no Git Bash: install Git for Windows or set CLAUDE_CODE_GIT_BASH_PATH")
        temporary = tempfile.TemporaryDirectory(prefix="windows-hooks-")
        self.addCleanup(temporary.cleanup)
        base = Path(temporary.name).resolve()
        self.tmpdir = base / "tmp"
        self.tmpdir.mkdir()
        # Windows project paths often contain spaces.
        self.project = base / "client project"
        (self.project / "src").mkdir(parents=True)
        self.env = {key: value for key, value in os.environ.items()
                    if not key.upper().startswith(("GIT_", "CONTEXT_", "ACCELERATOR_", "CLAUDE_", "PYTHON"))}
        self.git("init", "-q", "-b", TASK)
        self.git("config", "user.email", "windows-hooks@example.invalid")
        self.git("config", "user.name", "Windows Hooks")
        (self.project / "src" / "Invoice.php").write_text("<?php\nfinal class Invoice {}\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "fixture")
        self.run_checked(sys.executable, str(INSTALLER), "--edition", "PHP Core",
                         "--target", str(self.project), "--tool", "claude")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "accelerator")
        self.run_checked(sys.executable, str(self.project / "memory-bank" / "scripts" / "context.py"),
                         "start", "--task-id", TASK, "--goal", GOAL, "--json")

    def run_checked(self, *command: str) -> subprocess.CompletedProcess:
        result = subprocess.run(command, cwd=self.project, env=self.env, capture_output=True, text=True,
                                encoding="utf-8", errors="replace", timeout=300)
        self.assertEqual(0, result.returncode, f"{command}\n{result.stdout}\n{result.stderr}")
        return result

    def git(self, *args: str) -> None:
        self.run_checked("git", "-C", str(self.project), *args)

    @classmethod
    def interpreter(cls, name: str) -> Path:
        """A virtual environment's Scripts folder: python.exe and no python3.exe.

        That is the shape of a python.org installation, whatever else the
        runner's own Python folder holds.
        """
        venv = cls.shared / name
        if not venv.exists():
            made = subprocess.run([sys.executable, "-m", "venv", "--without-pip", str(venv)],
                                  capture_output=True, text=True, timeout=300)
            if made.returncode:
                raise AssertionError(f"venv {name}: {made.stdout}{made.stderr}")
        scripts = venv / "Scripts"
        # A venv made by a python3.exe gets a python3.exe of its own.
        for extra in scripts.glob("python3*"):
            extra.unlink()
        return scripts

    def hook_env(self, *interpreters: Path) -> dict[str, str]:
        """The hooks' environment: these folders, then PATH without any Python."""
        return {**self.env,
                "PATH": os.pathsep.join([*map(str, interpreters), *without_python(os.environ.get("PATH", ""))]),
                "TMPDIR": str(self.tmpdir), "CLAUDE_PROJECT_DIR": str(self.project),
                "CONTEXT_HOOK_BUDGET": BUDGET}

    def bash_run(self, env: dict[str, str], script: str, payload: str = "") -> subprocess.CompletedProcess:
        # Windows gives bash.exe one command line. subprocess leaves an argument
        # without spaces unquoted, so the double quotes in `"${CLAUDE_PROJECT_DIR}"/...`
        # reached Git Bash bare and `bash -c` failed to parse: bash reads the
        # command from a file instead.
        command = self.tmpdir.parent / "command.sh"
        command.write_text(script + "\n", encoding="utf-8", newline="\n")
        return subprocess.run([self.bash, command.as_posix()], cwd=self.project, env=env, input=payload,
                              capture_output=True, text=True, encoding="utf-8", errors="replace",
                              timeout=HOOK_TIMEOUT)

    def hook(self, env: dict[str, str], event: str, script: str, payload: dict) -> subprocess.CompletedProcess:
        """`script`, run by the command that .claude/settings.json gives `event`."""
        settings = json.loads((self.project / ".claude" / "settings.json").read_text(encoding="utf-8"))
        commands = [hook["command"] for group in settings["hooks"][event] for hook in group["hooks"]]
        command = next((command for command in commands if command.endswith(f"/.claude/hooks/{script}")), None)
        self.assertIsNotNone(command, f"{event} does not run {script}: {commands}")
        result = self.bash_run(env, command, json.dumps(
            {"session_id": "windows-hooks", "hook_event_name": event, **payload}))
        self.assertEqual(0, result.returncode, f"{script}\n{result.stdout}\n{result.stderr}")
        return result

    def remembered(self, env: dict[str, str]) -> Path:
        """Where the hooks remember the interpreter they found."""
        uid = self.bash_run(env, 'printf %s "$UID"').stdout
        self.assertRegex(uid, r"^[0-9]+$")
        return self.tmpdir / f"ai-accelerator-python-{uid}"

    def assert_remembers(self, remembered: Path, name: str, folder: Path | None = None) -> None:
        self.assertTrue(remembered.is_file(),
                        f"no {remembered.name} in TMPDIR: {sorted(entry.name for entry in self.tmpdir.iterdir())}")
        found_name, _, found_path = remembered.read_text(encoding="utf-8").rstrip("\n").partition("\t")
        self.assertEqual(name, found_name)
        if folder is not None:
            # Git Bash's own spelling of the path it found, such as
            # /tmp/windows-hooks-x/venv-1/Scripts/python.
            folder_tail = re.escape(f"/{folder.parent.name}/{folder.name}/python".lower())
            self.assertRegex(found_path.lower(), rf"{folder_tail}(\.exe)?$")

    def test_a_session_reads_and_writes_memory_through_python(self) -> None:
        scripts = self.interpreter("venv-1")
        env = self.hook_env(scripts)
        # The premise: Git Bash finds python and no python3.
        python3 = self.bash_run(env, "command -v python3")
        self.assertNotEqual(0, python3.returncode, f"python3 is on the hooks' PATH: {python3.stdout}")
        self.assertEqual(0, self.bash_run(env, "command -v python").returncode, "python is not on the hooks' PATH")
        remembered = self.remembered(env)
        self.assertFalse(remembered.exists())

        session = self.hook(env, "SessionStart", "local-context.sh", {"source": "startup"})
        self.assertIn("Python here is `python`, not python3", session.stdout)
        self.assertIn("Context governance: mode=governed", session.stdout)
        self.assert_remembers(remembered, "python", scripts)

        # The prompt and Stop hooks take the remembered name. A hook that
        # probed again would also write the file again.
        os.utime(remembered, (LONG_AGO, LONG_AGO))
        prompt = self.hook(env, "UserPromptSubmit", "working-memory-read.sh", PROMPT)
        self.assertIn(WORKING, prompt.stdout)
        self.assertNotIn("Memory refresh unavailable", prompt.stdout)
        self.assertEqual(LONG_AGO, int(remembered.stat().st_mtime), "the prompt hook probed for Python again")

        report = self.project / "memory-bank" / "local" / "last-turn-report.json"
        self.assertFalse(report.exists())
        (self.project / "src" / "Invoice.php").write_text(
            "<?php\nfinal class Invoice\n{\n    public function toCsv(): string\n    {\n        return '';\n    }\n}\n",
            encoding="utf-8")
        self.hook(env, "Stop", "working-memory-write.sh", {"stop_hook_active": False})
        self.assertTrue(report.is_file(), "the Stop hook recorded no turn")
        turn = json.loads(report.read_text(encoding="utf-8"))
        self.assertEqual(TASK, turn.get("task_id"), turn)
        self.assertNotIn("error", turn)
        self.assertGreaterEqual(turn.get("files", 0), 1, turn)
        self.assertEqual(LONG_AGO, int(remembered.stat().st_mtime), "the Stop hook probed for Python again")

    def test_the_py_launcher_serves_where_there_is_no_python(self) -> None:
        launcher = shutil.which("py")
        if launcher is None:
            self.skipTest("no py launcher on this machine")
        env = self.hook_env(Path(launcher).parent)
        if any(self.bash_run(env, f"command -v {name}").returncode == 0 for name in ("python", "python3")):
            self.skipTest(f"{Path(launcher).parent} holds python as well as py")
        if self.bash_run(env, "py -3 -c 'import sys; sys.exit(sys.version_info < (3, 9))'").returncode:
            self.skipTest("py -3 starts no Python 3.9+ here")
        remembered = self.remembered(env)
        session = self.hook(env, "SessionStart", "local-context.sh", {"source": "startup"})
        self.assertIn("Python here is `py -3`, not python3", session.stdout)
        self.assert_remembers(remembered, "py")
        prompt = self.hook(env, "UserPromptSubmit", "working-memory-read.sh", PROMPT)
        self.assertIn(WORKING, prompt.stdout)

    def test_a_python_found_at_another_path_is_probed_again(self) -> None:
        first, second = self.interpreter("venv-1"), self.interpreter("venv-2")
        env = self.hook_env(first)
        remembered = self.remembered(env)
        self.assertIn(WORKING, self.hook(env, "UserPromptSubmit", "working-memory-read.sh", PROMPT).stdout)
        self.assert_remembers(remembered, "python", first)
        # Reinstalled, or another Python first on PATH now: asked again.
        env = self.hook_env(second, first)
        self.assertIn(WORKING, self.hook(env, "UserPromptSubmit", "working-memory-read.sh", PROMPT).stdout)
        self.assert_remembers(remembered, "python", second)

    def test_the_validators_check_a_russian_call_through_python(self) -> None:
        # Where Git Bash has neither jq nor php, the validators decode a call
        # with Python. They named only python3, and Python read the call by
        # the ANSI code page - a Russian word failed to decode - and wrote \n
        # as \r\n: either way the call went through unchecked.
        env = self.hook_env(self.interpreter("venv-1"))
        env["PATH"] = os.pathsep.join(folder for folder in env["PATH"].split(os.pathsep)
                                      if not any(os.path.lexists(os.path.join(folder, name))
                                                 for name in JSON_TOOLS))
        if any(self.bash_run(env, f"command -v {name}").returncode == 0 for name in ("jq", "php")):
            self.skipTest("jq or php is still on PATH")
        settings = json.loads((self.project / ".claude" / "settings.json").read_text(encoding="utf-8"))
        commands = [hook["command"] for group in settings["hooks"]["PreToolUse"] for hook in group["hooks"]]
        calls = {
            "bash-validator.sh": {"tool_name": "Bash", "tool_input": {"command": "rm -rf / # очистка\necho done"}},
            "file-naming-validator.sh": {"tool_name": "Write",
                                         "tool_input": {"file_path": "tasks/TASK-1/план.md", "content": "Черновик"}},
            "subagent-gate.sh": {"tool_name": "Agent",
                                 "tool_input": {"subagent_type": "Explore", "prompt": "Найди, где считается скидка"}},
        }
        for script, payload in calls.items():
            with self.subTest(script=script):
                command = next((command for command in commands if command.endswith(f"/.claude/hooks/{script}")), None)
                self.assertIsNotNone(command, f"PreToolUse does not run {script}: {commands}")
                result = self.bash_run(env, command, json.dumps(
                    {"session_id": "windows-hooks", "hook_event_name": "PreToolUse", **payload}, ensure_ascii=False))
                self.assertEqual(2, result.returncode, f"{script}\n{result.stdout}\n{result.stderr}")


if __name__ == "__main__":
    unittest.main()
