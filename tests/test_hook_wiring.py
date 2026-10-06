"""Hook wiring must reach the hook scripts from any working directory.

Claude Code and Codex run a hook command in the session's current directory.
Wired as a bare relative path (".claude/hooks/bash-validator.sh"), a hook is
found only while that directory is the project root; from a subdirectory the
shell exits 127, and both hosts treat every exit other than 2 as
non-blocking - the force-push guard silently lets the command through.

Every edition (the four ready-made editions and Infrastructure-Creator) is
checked here in a temporary copy of its hook layer:

- every command in .claude/settings.json and .codex/hooks.json that references
  a hooks/*.sh script uses the root-anchored form and runs from a nested
  directory without exit 126/127;
- the bash-validator entry blocks `git push --force origin main` (exit 2)
  through the wiring, not by calling the script directly;
- the Codex launcher also works without Git and when the project is nested
  inside a larger repository (Infrastructure-Creator's real layout), and under
  every login shell available here, because Codex runs it with `$SHELL -lc`.

Cursor is out of scope: it runs project hooks from the project root, so its
bare `.cursor/hooks/<script>.sh` wiring is correct as is.

The copy carries the hook layer only, not the memory runtime: the memory hooks
then take their documented "no CLI" exit 0 instead of indexing, which keeps the
suite fast and free of side effects while still proving each script was found.

Run: python3 -m unittest tests.test_hook_wiring
"""

from __future__ import annotations

import importlib.util
import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EDITIONS = ("Laravel", "Symfony", "PHP Core", "Cms/wordpress", "Infrastructure-Creator")
WIRING = {"claude": ".claude/settings.json", "codex": ".codex/hooks.json"}
# The hook layer a wired command can touch; everything else stays out.
HOOK_LAYER = (
    ".claude/settings.json",
    ".claude/hooks",
    ".claude/agents",
    ".codex/hooks.json",
    ".codex/hooks",
    ".codex/config.toml",
)
BASH = shutil.which("bash") or "/bin/bash"
TIMEOUT = 20

CLAUDE_ROOT = '"${CLAUDE_PROJECT_DIR}"/'
CODEX_LAUNCHER = (
    "sh -c 'd=$(pwd); until [ -z \"$d\" ] || [ -f \"$d/.codex/hooks/$1\" ]; "
    "do d=${d%/*}; done; exec \"$d/.codex/hooks/$1\"' sh "
)
SCRIPT_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*\.sh")
# A hooks path that starts a shell word: bare, "./"-prefixed, or behind an
# interpreter. Anchored forms put "/" or a placeholder in front of it.
BARE_HOOK_PATH = re.compile(r"""(?:^|[\s'"])(?:\./)?\.(?:claude|codex)/hooks/\S*\.sh""")

FORCE_PUSH = json.dumps(
    {
        "session_id": "hook-wiring-test",
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "git push --force origin main"},
    }
)

GENERATOR_VALIDATOR = (
    ROOT
    / "Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_generated.py"
)


def load_generator_validator():
    spec = importlib.util.spec_from_file_location("hook_wiring_validator", GENERATOR_VALIDATOR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def wired_hooks(document: dict) -> list:
    """(event, command) for every command handler that runs a hooks/*.sh."""
    found = []
    for event, groups in document.get("hooks", {}).items():
        for group in groups:
            for handler in group.get("hooks", []):
                command = handler.get("command", "")
                if "hooks/" in command and re.search(r"\.sh\b", command):
                    found.append((event, command))
    return found


def script_of(tool: str, command: str) -> str:
    """The hook script a canonical command runs, relative to the project."""
    prefix, hooks_dir = (
        (CLAUDE_ROOT, "") if tool == "claude" else (CODEX_LAUNCHER, ".codex/hooks/")
    )
    if not command.startswith(prefix):
        raise AssertionError(f"{tool} hook is not in the root-anchored form: {command}")
    return hooks_dir + command[len(prefix):]


def clean_env(**extra: str) -> dict:
    """The caller's environment minus anything that would mask the bug.

    A test launched from inside Claude Code inherits CLAUDE_PROJECT_DIR, and
    one launched from a Git hook inherits GIT_DIR; either would make a broken
    wiring look correct.
    """
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("GIT_")
        and key not in ("CLAUDE_PROJECT_DIR", "CONTEXT_TASK_ID")
    }
    env.update(extra)
    return env


def git_init(path: Path) -> None:
    subprocess.run(
        ["git", "init", "-q", str(path)], check=True, env=clean_env(), timeout=TIMEOUT
    )


def copy_hook_layer(edition: str, destination: Path) -> Path:
    source = ROOT / edition
    for rel in HOOK_LAYER:
        origin = source / rel
        if origin.is_dir():
            shutil.copytree(origin, destination / rel)
        elif origin.is_file():
            (destination / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(origin, destination / rel)
    return destination


class HookWiringTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.wiring = {}
        for edition in EDITIONS:
            for tool, rel in WIRING.items():
                document = json.loads((ROOT / edition / rel).read_text(encoding="utf-8"))
                cls.wiring[(edition, tool)] = wired_hooks(document)

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="hook-wiring-")
        self.addCleanup(temporary.cleanup)
        # A space in every path proves the quoting, not only the resolution.
        self.base = Path(temporary.name).resolve() / "with space"
        self.base.mkdir()
        self.tmp = self.base / "tmp"
        self.tmp.mkdir()

    def run_command(self, command: str, cwd: Path, payload: str, *, shell=None,
                    claude_root: Path | None = None) -> subprocess.CompletedProcess:
        extra = {"TMPDIR": str(self.tmp)}
        if claude_root is not None:
            extra["CLAUDE_PROJECT_DIR"] = str(claude_root)
        return subprocess.run(
            shell or [BASH, "-c", command],
            cwd=str(cwd),
            input=payload,
            capture_output=True,
            text=True,
            env=clean_env(**extra),
            timeout=TIMEOUT,
        )

    def edition_copy(self, edition: str, parent: Path, *, git: bool) -> tuple:
        project = copy_hook_layer(edition, parent / "project copy")
        if git:
            git_init(project)
        nested = project / "src" / "deep dir"
        nested.mkdir(parents=True)
        return project, nested

    def bash_validator(self, edition: str, tool: str) -> str:
        commands = [
            command
            for event, command in self.wiring[(edition, tool)]
            if event == "PreToolUse" and command.endswith("bash-validator.sh")
        ]
        self.assertEqual(len(commands), 1, f"{edition} {tool}: {commands}")
        return commands[0]

    # -- shape --------------------------------------------------------------

    def test_every_hook_command_is_root_anchored(self) -> None:
        for (edition, tool), hooks in self.wiring.items():
            with self.subTest(edition=edition, tool=tool):
                self.assertTrue(hooks, "no hook commands wired")
                for _event, command in hooks:
                    self.assertIsNone(
                        BARE_HOOK_PATH.search(command),
                        f"bare relative hooks path: {command}",
                    )
                    script = script_of(tool, command)
                    self.assertTrue(
                        SCRIPT_NAME.fullmatch(script.rsplit("/", 1)[-1]), command
                    )
                    path = ROOT / edition / script
                    self.assertTrue(path.is_file(), f"dead hook: {script}")
                    self.assertTrue(os.access(path, os.X_OK), f"not executable: {script}")

    def test_generator_gate_accepts_the_shipped_wiring(self) -> None:
        """Ready-made editions and generated targets share one wiring contract."""
        validator = load_generator_validator()
        for (edition, tool), hooks in self.wiring.items():
            for _event, command in hooks:
                with self.subTest(edition=edition, tool=tool, command=command):
                    scripts, unanchored = validator.resolve_wired_scripts(tool, command)
                    self.assertEqual(unanchored, [])
                    self.assertEqual(scripts, [script_of(tool, command)])

    # -- behaviour ----------------------------------------------------------

    def test_every_wired_hook_runs_from_a_nested_directory(self) -> None:
        for edition in EDITIONS:
            parent = self.base / edition.replace("/", "-")
            parent.mkdir()
            project, nested = self.edition_copy(edition, parent, git=True)
            for tool in WIRING:
                for event, command in self.wiring[(edition, tool)]:
                    with self.subTest(edition=edition, tool=tool, event=event,
                                      command=command[-40:]):
                        payload = json.dumps(
                            {
                                "session_id": "hook-wiring-test",
                                "hook_event_name": event,
                                "cwd": str(nested),
                                "tool_name": "Read",
                                "tool_input": {},
                            }
                        )
                        result = self.run_command(
                            command, nested, payload,
                            claude_root=project if tool == "claude" else None,
                        )
                        self.assertNotIn(result.returncode, (126, 127), result.stderr)

    def test_force_push_is_blocked_through_the_wiring_from_a_subdirectory(self) -> None:
        for edition in EDITIONS:
            parent = self.base / edition.replace("/", "-")
            parent.mkdir()
            project, nested = self.edition_copy(edition, parent, git=True)
            for tool in WIRING:
                with self.subTest(edition=edition, tool=tool):
                    result = self.run_command(
                        self.bash_validator(edition, tool), nested, FORCE_PUSH,
                        claude_root=project if tool == "claude" else None,
                    )
                    self.assertEqual(result.returncode, 2, result.stderr)

    def test_codex_launcher_needs_neither_git_nor_a_git_root_project(self) -> None:
        for edition in EDITIONS:
            command = self.bash_validator(edition, "codex")
            without_git = self.base / f"{edition.replace('/', '-')} plain"
            without_git.mkdir()
            _project, nested = self.edition_copy(edition, without_git, git=False)
            # Infrastructure-Creator's own layout: the project is a
            # subdirectory of a larger repository, so the Git top level is
            # the wrong root.
            monorepo = self.base / f"{edition.replace('/', '-')} monorepo"
            monorepo.mkdir()
            git_init(monorepo)
            (monorepo / "package").mkdir()
            _inner, inner_nested = self.edition_copy(
                edition, monorepo / "package", git=False
            )
            for label, cwd in (("no git", nested), ("nested in a repo", inner_nested)):
                with self.subTest(edition=edition, layout=label):
                    result = self.run_command(command, cwd, FORCE_PUSH)
                    self.assertEqual(result.returncode, 2, result.stderr)

    def test_codex_launcher_is_independent_of_the_login_shell(self) -> None:
        """Codex runs hooks with `$SHELL -lc`; any common shell must work."""
        shells = [name for name in ("sh", "dash", "bash", "zsh", "fish") if shutil.which(name)]
        self.assertIn("sh", shells)
        _project, nested = self.edition_copy("Laravel", self.base, git=True)
        command = self.bash_validator("Laravel", "codex")
        for name in shells:
            with self.subTest(shell=name):
                result = self.run_command(
                    command, nested, FORCE_PUSH, shell=[shutil.which(name), "-c", command]
                )
                self.assertEqual(result.returncode, 2, result.stderr)

    def test_claude_placeholder_runs_under_sh_as_claude_code_invokes_it(self) -> None:
        """Claude Code passes shell-form hooks to `sh -c` on macOS and Linux."""
        project, nested = self.edition_copy("PHP Core", self.base, git=True)
        command = self.bash_validator("PHP Core", "claude")
        result = self.run_command(
            command, nested, FORCE_PUSH, shell=["sh", "-c", command], claude_root=project
        )
        self.assertEqual(result.returncode, 2, result.stderr)

    def test_the_bare_relative_form_is_the_failure_this_guards(self) -> None:
        """Proof the fixture reproduces the bug: 127 is a silent pass."""
        _project, nested = self.edition_copy("Laravel", self.base, git=True)
        result = self.run_command(".claude/hooks/bash-validator.sh", nested, FORCE_PUSH)
        self.assertEqual(result.returncode, 127)


if __name__ == "__main__":
    unittest.main()
