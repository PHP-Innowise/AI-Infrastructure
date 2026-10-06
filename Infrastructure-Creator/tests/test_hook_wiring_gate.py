#!/usr/bin/env python3
"""The bootstrap-verifier wiring gate accepts only root-anchored hook wiring.

Claude Code and Codex run a hook command in the session's current directory.
A bare ".claude/hooks/x.sh" therefore exits 127 as soon as that directory is a
subdirectory, and both hosts treat every exit other than 2 as non-blocking: the
guardrail silently stops. hook-forge wires Claude hooks through the
"${CLAUDE_PROJECT_DIR}" placeholder and Codex hooks through a fixed root-finding
launcher; these tests hold the verifier, the forge's instructions and this
edition's own wiring to that one contract, and prove the forms actually run
from a nested directory.

Run from this directory: python3 test_hook_wiring_gate.py
"""

from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
SPEC = importlib.util.spec_from_file_location(
    "validate_generated_wiring", SCRIPTS / "validate_generated.py"
)
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)

HOOK_FORGE = ROOT / ".agents/skills/hook-forge/SKILL.md"
BASH = shutil.which("bash") or "/bin/bash"
CLAUDE_ROOT = '"${CLAUDE_PROJECT_DIR}"/'
SCRIPT = "bash-validator.sh"


def claude_command(script: str) -> str:
    return f"{CLAUDE_ROOT}.claude/hooks/{script}"


def codex_command(script: str) -> str:
    return validator.CODEX_HOOK_LAUNCHER + script


class WiringFixture(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="hook-wiring-gate-")
        self.addCleanup(temporary.cleanup)
        # A space in the path proves the quoting, not just the resolution.
        self.target = Path(temporary.name).resolve() / "target project"
        self.target.mkdir()
        self.files: dict = {}

    def hook(self, rel: str, body: str = "#!/bin/sh\nexit 0\n", *, owned: bool = True,
             executable: bool = True) -> Path:
        path = self.target / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755 if executable else 0o644)
        if owned:
            self.files[rel] = {"sha256": "x"}
        return path

    def wire(self, edition: str, commands: list) -> None:
        wiring_rel = validator.EDITION_HOOK_WIRING[edition][1]
        document = {"hooks": {"PreToolUse": [{"hooks": [
            {"type": "command", "command": command} for command in commands
        ]}]}}
        path = self.target / wiring_rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(document), encoding="utf-8")
        self.files[wiring_rel] = {"sha256": "x"}

    def wiring_errors(self, edition: str) -> list:
        if edition == "codex":
            config = self.target / ".codex/config.toml"
            config.write_text("[features]\nhooks = true\n", encoding="utf-8")
            self.files[".codex/config.toml"] = {"sha256": "x"}
        errors: list = []
        validator.validate_hook_wiring(self.target, [edition], self.files, errors)
        return errors


class RootAnchoredWiringTest(WiringFixture):
    def test_documented_forms_pass_for_every_edition(self) -> None:
        commands = {
            "claude": claude_command(SCRIPT),
            "codex": codex_command(SCRIPT),
            # Cursor runs project hooks from the project root: bare stays right.
            "cursor": f".cursor/hooks/{SCRIPT}",
        }
        for edition, command in commands.items():
            with self.subTest(edition=edition):
                self.hook(f"{validator.EDITION_HOOK_WIRING[edition][0]}/{SCRIPT}")
                self.wire(edition, [command])
                self.assertEqual(self.wiring_errors(edition), [])

    def test_unbraced_claude_placeholder_is_the_same_expansion(self) -> None:
        self.hook(f".claude/hooks/{SCRIPT}")
        self.wire("claude", [f'"$CLAUDE_PROJECT_DIR"/.claude/hooks/{SCRIPT}'])
        self.assertEqual(self.wiring_errors("claude"), [])

    def test_bare_relative_path_is_rejected_on_claude_and_codex(self) -> None:
        for edition in ("claude", "codex"):
            with self.subTest(edition=edition):
                hooks_rel = validator.EDITION_HOOK_WIRING[edition][0]
                self.hook(f"{hooks_rel}/{SCRIPT}")
                self.wire(edition, [f"{hooks_rel}/{SCRIPT}"])
                errors = self.wiring_errors(edition)
                self.assertEqual(len(errors), 1, errors)
                self.assertIn("not anchored to the project root", errors[0])

    def test_anchored_dead_hook_is_still_rejected(self) -> None:
        for edition, command in (
            ("claude", claude_command("missing.sh")),
            ("codex", codex_command("missing.sh")),
        ):
            with self.subTest(edition=edition):
                hooks_rel = validator.EDITION_HOOK_WIRING[edition][0]
                # Owned by the manifest but absent on disk.
                self.files[f"{hooks_rel}/missing.sh"] = {"sha256": "x"}
                self.wire(edition, [command])
                errors = self.wiring_errors(edition)
                self.assertEqual(len(errors), 1, errors)
                self.assertIn("wired hook does not exist", errors[0])

    def test_anchored_unknown_or_non_executable_hook_is_rejected(self) -> None:
        self.hook(".claude/hooks/team.sh", owned=False)
        self.hook(".codex/hooks/plain.sh", executable=False)
        self.wire("claude", [claude_command("team.sh")])
        self.wire("codex", [codex_command("plain.sh")])
        claude_errors = self.wiring_errors("claude")
        codex_errors = self.wiring_errors("codex")
        self.assertEqual(len(claude_errors), 1, claude_errors)
        self.assertIn("not manifest-owned: .claude/hooks/team.sh", claude_errors[0])
        self.assertEqual(len(codex_errors), 1, codex_errors)
        self.assertIn("not executable: .codex/hooks/plain.sh", codex_errors[0])

    def test_tampered_or_foreign_launchers_are_not_trusted(self) -> None:
        self.hook(f".codex/hooks/{SCRIPT}")
        self.hook(f".claude/hooks/{SCRIPT}")
        tampered = validator.CODEX_HOOK_LAUNCHER.replace("exec ", "exec bash ")
        cases = [
            # A path instead of a bare name escapes the hooks directory.
            ("codex", codex_command(f"../../elsewhere/{SCRIPT}")),
            # Any edit to the walk makes it a different, unreviewed command.
            ("codex", tampered + SCRIPT),
            # Codex exports no CLAUDE_PROJECT_DIR to project hooks.
            ("codex", claude_command(SCRIPT).replace(".claude/", ".codex/")),
            # The launcher only knows .codex/hooks; on Claude it is foreign.
            ("claude", codex_command(SCRIPT)),
        ]
        for edition, command in cases:
            with self.subTest(edition=edition, command=command):
                self.wire(edition, [command])
                self.assertNotEqual(self.wiring_errors(edition), [])

    def test_this_editions_own_wiring_passes_the_gate(self) -> None:
        """The generator ships the same forms it requires of a target."""
        files = {}
        for edition, (hooks_rel, wiring_rel) in validator.EDITION_HOOK_WIRING.items():
            files[wiring_rel] = {}
            for script in (ROOT / hooks_rel).glob("*.sh"):
                files[f"{hooks_rel}/{script.name}"] = {}
        files[".codex/config.toml"] = {}
        errors: list = []
        validator.validate_hook_wiring(ROOT, ["claude", "cursor", "codex"], files, errors)
        self.assertEqual(errors, [])

    def test_hook_forge_spells_out_the_exact_forms_the_gate_accepts(self) -> None:
        text = HOOK_FORGE.read_text(encoding="utf-8")
        self.assertIn(f"`{claude_command('<script>.sh')}`", text)
        self.assertIn(f"`{codex_command('<script>.sh')}`", text)
        self.assertIn("`.cursor/hooks/<script>.sh`", text)


class RootAnchoredWiringRunsTest(WiringFixture):
    """The accepted forms reach the hook from a nested directory."""

    PAYLOAD = json.dumps({"tool_name": "Bash", "tool_input": {"command": "x"}})

    def run_from_nested(self, command: str, *, git: bool, env: dict | None = None):
        nested = self.target / "src" / "deep dir"
        nested.mkdir(parents=True, exist_ok=True)
        if git:
            subprocess.run(["git", "init", "-q", str(self.target)], check=True)
        return subprocess.run(
            [BASH, "-c", command],
            cwd=str(nested),
            input=self.PAYLOAD,
            capture_output=True,
            text=True,
            env={**os.environ, **(env or {})},
            timeout=30,
        )

    def test_claude_placeholder_runs_from_a_subdirectory(self) -> None:
        self.hook(f".claude/hooks/{SCRIPT}", "#!/bin/sh\ncat >/dev/null\nexit 2\n")
        result = self.run_from_nested(
            claude_command(SCRIPT), git=False,
            env={"CLAUDE_PROJECT_DIR": str(self.target)},
        )
        self.assertEqual(result.returncode, 2, result.stderr)

    def test_codex_launcher_runs_from_a_subdirectory_with_and_without_git(self) -> None:
        self.hook(f".codex/hooks/{SCRIPT}", "#!/bin/sh\ncat >/dev/null\nexit 2\n")
        for git in (False, True):
            with self.subTest(git=git):
                result = self.run_from_nested(codex_command(SCRIPT), git=git)
                self.assertEqual(result.returncode, 2, result.stderr)

    def test_bare_relative_path_is_what_failed_open(self) -> None:
        """The regression the gate exists for: 127 is non-blocking."""
        self.hook(f".claude/hooks/{SCRIPT}", "#!/bin/sh\nexit 2\n")
        result = self.run_from_nested(f".claude/hooks/{SCRIPT}", git=False)
        self.assertEqual(result.returncode, 127)


if __name__ == "__main__":
    unittest.main()
