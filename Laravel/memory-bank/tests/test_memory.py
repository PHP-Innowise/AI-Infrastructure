#!/usr/bin/env python3
"""Contract tests for authority-aware unified memory."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SKILL_PATHS = tuple(
    f"{tool}/skills/memory/SKILL.md"
    for tool in (".agents", ".claude", ".cursor")
)
VALIDATOR_PATHS = tuple(
    f"{tool}/skills/skill-creator/scripts/quick_validate.py"
    for tool in (".agents", ".claude", ".cursor")
)


class MemoryIntegrationTest(unittest.TestCase):
    def run_validator(
        self, validator_path: str, skill_path: Path
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(REPOSITORY_ROOT / validator_path), str(skill_path)],
            capture_output=True,
            check=False,
            cwd=REPOSITORY_ROOT,
            text=True,
        )

    def test_tool_skills_are_byte_identical(self) -> None:
        contents = [
            REPOSITORY_ROOT.joinpath(path).read_bytes() for path in SKILL_PATHS
        ]
        self.assertEqual(contents[0], contents[1])
        self.assertEqual(contents[0], contents[2])

    def test_unified_memory_preserves_governed_authority(self) -> None:
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(encoding="utf-8")
        required = (
            "AI skill/command name, not a shell executable",
            "project-brain/config/runtime.json",
            "Project Brain as the only authority",
            "context.py validate",
            "working: governed",
            ".agents/skills/checkpoint/SKILL.md",
            "explicitly configured lightweight mode",
            "does not invoke or chain another skill",
            "context.py index --json",
            "context.py status --json",
            "procedural: updated | failed",
            "semantic: updated | failed",
            "episodic: updated | failed",
            "MUST NOT run `complete`, `record`, `clear`",
        )
        for text in required:
            with self.subTest(text=text):
                self.assertIn(text, skill)

    def test_commands_accept_no_arguments(self) -> None:
        for tool in (".claude", ".cursor"):
            command = REPOSITORY_ROOT.joinpath(
                tool, "commands/memory.md"
            ).read_text(encoding="utf-8")
            self.assertIn("accepts no arguments", command)
            self.assertIn(f"{tool}/skills/memory/SKILL.md", command)

    def test_validators_accept_flow_metadata_and_reject_bad_names(self) -> None:
        for validator_path in VALIDATOR_PATHS:
            tool = Path(validator_path).parts[0]
            result = self.run_validator(
                validator_path,
                REPOSITORY_ROOT / tool / "skills/memory",
            )
            self.assertEqual(0, result.returncode, result.stdout + result.stderr)

            with tempfile.TemporaryDirectory() as directory:
                skill = Path(directory) / "skill"
                skill.mkdir()
                skill.joinpath("SKILL.md").write_text(
                    "---\n"
                    "name: memory\n"
                    "description: Validate project flow metadata.\n"
                    "phase: utility\n"
                    "flow-next: memory--bank\n"
                    "flow-alternatives: [project-brain]\n"
                    "---\n",
                    encoding="utf-8",
                )
                invalid = self.run_validator(validator_path, skill)
            self.assertNotEqual(0, invalid.returncode)

    def test_skill_flow_uses_exact_discovery_names(self) -> None:
        expected = {
            ".agents": "`checkpoint`, `memory`",
            ".claude": "`/checkpoint`, `/memory`",
            ".cursor": "`/checkpoint`, `/memory`",
        }
        for tool, names in expected.items():
            flow = REPOSITORY_ROOT.joinpath(
                tool, "skills/SKILL FLOW.md"
            ).read_text(encoding="utf-8")
            self.assertIn(names, flow)


if __name__ == "__main__":
    unittest.main()
