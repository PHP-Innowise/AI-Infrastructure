#!/usr/bin/env python3
"""Regression coverage for generated automatic-continuity hook wiring."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(SCRIPTS))

import validate_generated as validator  # noqa: E402


class ContextContinuityGeneratorTest(unittest.TestCase):
    def setUp(self) -> None:
        self._temporary = tempfile.TemporaryDirectory(prefix="continuity-wire-")
        self.addCleanup(self._temporary.cleanup)
        self.target = Path(self._temporary.name)

    def fixture(self, edition: str) -> tuple[dict, str]:
        hooks_rel, wiring_rel = validator.EDITION_HOOK_WIRING[edition]
        commands = validator.CONTINUITY_WIRING[edition]
        document = {
            "hooks": {
                event: [{"command": command}]
                for event, command in commands.items()
            }
        }
        wiring = self.target / wiring_rel
        wiring.parent.mkdir(parents=True, exist_ok=True)
        wiring.write_text(json.dumps(document), encoding="utf-8")
        adapter_rel = f"{hooks_rel}/context-continuity.sh"
        adapter = self.target / adapter_rel
        adapter.parent.mkdir(parents=True, exist_ok=True)
        adapter.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        adapter.chmod(0o755)
        files = {wiring_rel: {}, adapter_rel: {}}
        if edition == "codex":
            config_rel = ".codex/config.toml"
            config = self.target / config_rel
            config.write_text("[features]\nhooks = true\n", encoding="utf-8")
            files[config_rel] = {}
        return files, wiring_rel

    def validate(self, edition: str, files: dict) -> list[str]:
        errors: list[str] = []
        validator.validate_hook_wiring(self.target, [edition], files, errors)
        return errors

    def test_every_native_lifecycle_wires_capture_and_restore(self) -> None:
        for edition in validator.CONTINUITY_WIRING:
            with self.subTest(edition=edition):
                files, _ = self.fixture(edition)
                self.assertEqual([], self.validate(edition, files))

    def test_missing_or_wrong_event_wiring_is_rejected(self) -> None:
        for edition, requirements in validator.CONTINUITY_WIRING.items():
            for event, command in requirements.items():
                with self.subTest(edition=edition, event=event):
                    files, wiring_rel = self.fixture(edition)
                    wiring = self.target / wiring_rel
                    document = json.loads(wiring.read_text(encoding="utf-8"))
                    document["hooks"][event][0]["command"] = command.replace(
                        "capture", "restore"
                    )
                    if command.endswith("restore"):
                        document["hooks"].pop(event)
                    wiring.write_text(json.dumps(document), encoding="utf-8")
                    errors = self.validate(edition, files)
                    self.assertIn(
                        f"[{edition}] {wiring_rel}: context-continuity "
                        f"{command.rsplit(' ', 1)[1]} is not wired on {event}",
                        errors,
                    )


if __name__ == "__main__":
    unittest.main()
