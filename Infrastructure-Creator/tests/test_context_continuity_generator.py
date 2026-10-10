#!/usr/bin/env python3
"""The generator gate holds generated chat-snapshot hooks to one contract.

`context-continuity.sh` is one script for every lifecycle event it serves:
the client names the event in its payload and `context_continuity.py --event
hook` acts on it - a session start delivers a merge prepared with
`context-load merge`, a prompt or a final answer is captured. So the wiring
carries no argument, uses each edition's root-anchored hook-forge form, and
must be present on every one of those events; the script must call the
runtime with its own host. These tests hold bootstrap-verifier, hook-forge's
instructions and the runtime contract to that.

Run from this directory: python3 test_context_continuity_generator.py
"""

from __future__ import annotations

import importlib.util
import json
import re
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
SPEC = importlib.util.spec_from_file_location(
    "validate_generated_continuity", SCRIPTS / "validate_generated.py"
)
validator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validator)

HOOK_FORGE = ROOT / ".agents/skills/hook-forge/SKILL.md"
CONTRACT = ROOT / ".agents/skills/memory-seed/assets/runtime-contract.json"
HOOK = validator.CONTINUITY_HOOK


def form(edition: str, script: str) -> str:
    """hook-forge's exact command for a script on an edition."""
    if edition == "claude":
        return f'"${{CLAUDE_PROJECT_DIR}}"/.claude/hooks/{script}'
    if edition == "codex":
        return validator.CODEX_HOOK_LAUNCHER + script
    return f".cursor/hooks/{script}"


def adapter(edition: str) -> str:
    return (
        "#!/bin/bash\n"
        'ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd) || exit 0\n'
        'RUNTIME="$ROOT_DIR/memory-bank/scripts/context_continuity.py"\n'
        f'python3 "$RUNTIME" --root "$ROOT_DIR" --host {edition} --event hook 2>/dev/null\n'
        "exit 0\n"
    )


class ContinuityFixture(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="continuity-wire-")
        self.addCleanup(temporary.cleanup)
        # A space in the path proves the quoting, not just the resolution.
        self.target = Path(temporary.name).resolve() / "target project"
        self.target.mkdir()

    def fixture(self, edition: str, *, body: str | None = None) -> tuple[dict, Path]:
        """A target whose wiring runs the adapter on every event it serves."""
        hooks_rel, wiring_rel = validator.EDITION_HOOK_WIRING[edition]
        script = self.target / hooks_rel / HOOK
        script.parent.mkdir(parents=True, exist_ok=True)
        script.write_text(adapter(edition) if body is None else body, encoding="utf-8")
        script.chmod(0o755)
        document = {
            "hooks": {
                event: [{"hooks": [{"type": "command", "command": form(edition, HOOK)}]}]
                for event in validator.CONTINUITY_EVENTS[edition]
            }
        }
        wiring = self.target / wiring_rel
        wiring.parent.mkdir(parents=True, exist_ok=True)
        wiring.write_text(json.dumps(document), encoding="utf-8")
        files = {wiring_rel: {}, f"{hooks_rel}/{HOOK}": {}}
        if edition == "codex":
            config = self.target / ".codex/config.toml"
            config.write_text("[features]\nhooks = true\n", encoding="utf-8")
            files[".codex/config.toml"] = {}
        return files, wiring

    def wiring_errors(self, edition: str, files: dict) -> list:
        errors: list = []
        validator.validate_hook_wiring(self.target, [edition], files, errors)
        validator.validate_continuity_wiring(self.target, [edition], files, errors)
        return errors

    def runtime_errors(self, edition: str, files: dict) -> list:
        errors: list = []
        validator.validate_hooks(self.target, [edition], files, errors)
        return [error for error in errors if HOOK in error]


class ContinuityWiringTest(ContinuityFixture):
    def test_every_event_in_hook_forge_form_passes(self) -> None:
        for edition in validator.EDITION_HOOK_WIRING:
            with self.subTest(edition=edition):
                files, _ = self.fixture(edition)
                self.assertEqual([], self.wiring_errors(edition, files))

    def test_the_events_are_each_hosts_start_prompt_and_answer(self) -> None:
        self.assertEqual(
            {
                "claude": ("SessionStart", "UserPromptSubmit", "Stop"),
                "codex": ("SessionStart", "UserPromptSubmit", "Stop"),
                "cursor": ("sessionStart", "beforeSubmitPrompt", "afterAgentResponse"),
            },
            dict(validator.CONTINUITY_EVENTS),
        )
        for edition in validator.EDITION_HOOK_WIRING:
            self.assertIn(HOOK, validator.REQUIRED_HOOKS[edition])
            self.assertIn(HOOK, validator.WIRED_HOOKS[edition])

    def test_a_missing_event_is_rejected(self) -> None:
        for edition, events in validator.CONTINUITY_EVENTS.items():
            for event in events:
                with self.subTest(edition=edition, event=event):
                    files, wiring = self.fixture(edition)
                    document = json.loads(wiring.read_text(encoding="utf-8"))
                    document["hooks"].pop(event)
                    wiring.write_text(json.dumps(document), encoding="utf-8")
                    errors = self.wiring_errors(edition, files)
                    self.assertEqual(len(errors), 1, errors)
                    self.assertIn(f"does not run {HOOK} on {event}", errors[0])

    def test_a_positional_mode_argument_is_not_hook_forge_form(self) -> None:
        """The older `context-continuity.sh restore|capture` wiring is refused."""
        for edition, events in validator.CONTINUITY_EVENTS.items():
            with self.subTest(edition=edition):
                files, wiring = self.fixture(edition)
                document = json.loads(wiring.read_text(encoding="utf-8"))
                start = events[0]
                command = document["hooks"][start][0]["hooks"][0]
                command["command"] = form(edition, HOOK) + " restore"
                wiring.write_text(json.dumps(document), encoding="utf-8")
                errors = self.wiring_errors(edition, files)
                # The command itself is refused (a Claude/Cursor form with a
                # trailing word, or a Codex launcher whose name is not a
                # script), and the start event is left without the hook.
                wiring_errors: list = []
                validator.validate_hook_wiring(self.target, [edition], files, wiring_errors)
                self.assertTrue(wiring_errors, errors)
                self.assertTrue(
                    any(f"does not run {HOOK} on {start}" in error for error in errors),
                    errors,
                )

    def test_a_bare_or_git_root_codex_path_is_refused(self) -> None:
        """Neither the bare path nor `$(git rev-parse --show-toplevel)` is the launcher."""
        for command in (
            f".codex/hooks/{HOOK}",
            f'"$(git rev-parse --show-toplevel)/.codex/hooks/{HOOK}"',
        ):
            with self.subTest(command=command):
                files, wiring = self.fixture("codex")
                document = json.loads(wiring.read_text(encoding="utf-8"))
                document["hooks"]["Stop"][0]["hooks"][0]["command"] = command
                wiring.write_text(json.dumps(document), encoding="utf-8")
                errors = self.wiring_errors("codex", files)
                self.assertTrue(errors)
                self.assertTrue(
                    any(f"does not run {HOOK} on Stop" in error for error in errors),
                    errors,
                )

    def test_wired_next_to_other_hooks_on_the_same_event_passes(self) -> None:
        """Appended after an event's existing hooks, as hook-forge step 9 wires it."""
        files, wiring = self.fixture("cursor")
        read = self.target / ".cursor/hooks/working-memory-read.sh"
        read.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        read.chmod(0o755)
        files[".cursor/hooks/working-memory-read.sh"] = {}
        document = json.loads(wiring.read_text(encoding="utf-8"))
        document["hooks"]["beforeSubmitPrompt"] = [
            {"command": ".cursor/hooks/working-memory-read.sh", "timeout": 8},
            {"command": f".cursor/hooks/{HOOK}", "timeout": 8},
        ]
        wiring.write_text(json.dumps(document), encoding="utf-8")
        self.assertEqual([], self.wiring_errors("cursor", files))


class ContinuityRuntimeCallTest(ContinuityFixture):
    def test_the_adapter_calls_the_runtime_with_its_host_and_event_dispatch(self) -> None:
        for edition in validator.EDITION_HOOK_WIRING:
            with self.subTest(edition=edition):
                files, _ = self.fixture(edition)
                self.assertEqual([], self.runtime_errors(edition, files))

    def test_a_wrong_host_a_mode_or_another_runtime_is_rejected(self) -> None:
        cases = {
            "wrong host": adapter("claude").replace("--host claude", "--host cursor"),
            "positional mode": adapter("claude").replace("--event hook", "--event capture"),
            "no runtime": "#!/bin/bash\nexit 0\n",
        }
        for label, body in cases.items():
            with self.subTest(case=label):
                files, _ = self.fixture("claude", body=body)
                errors = self.runtime_errors("claude", files)
                self.assertEqual(len(errors), 1, errors)
                self.assertIn("--host claude --event hook", errors[0])


class ContinuityDocumentationTest(unittest.TestCase):
    def test_hook_forge_wires_no_mode_argument_and_no_git_root_path(self) -> None:
        text = HOOK_FORGE.read_text(encoding="utf-8")
        self.assertIsNone(
            re.search(r"context-continuity\.sh[`\"']?\s+(?:restore|capture)\b", text)
        )
        self.assertNotIn("git rev-parse --show-toplevel", text)
        self.assertIn("--event hook", text)
        self.assertIn("`afterAgentResponse` (context-continuity, 8)", text)
        self.assertIn("context-continuity, with `additionalContextLimit: 6000`", text)

    def test_the_contract_delivers_only_a_prepared_merge(self) -> None:
        contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
        commands = contract["commands"]
        self.assertEqual(
            [
                "python3 memory-bank/scripts/context_continuity.py --root ROOT "
                "--host CLIENT --event hook"
            ],
            commands["automatic_continuity"],
        )
        merge_forms = commands["chat_merge"]
        self.assertTrue(any("--event list" in form for form in merge_forms))
        self.assertTrue(any("--event merge --source-session" in form for form in merge_forms))
        for form_text in commands["automatic_continuity"] + merge_forms:
            self.assertIn(form_text, commands["purposes"])
            self.assertIn(form_text, commands["outcomes"])
        continuity = contract["automatic_continuity"]
        self.assertIn("only a merge explicitly prepared", continuity["restore"])
        self.assertNotIn("up to eight", continuity["restore"])
        self.assertIn("hook_event_name", continuity["dispatch"])
        self.assertIn(
            "memory-bank/scripts/context_continuity.py",
            contract["path_contracts"]["required_skeleton"],
        )


if __name__ == "__main__":
    unittest.main()
