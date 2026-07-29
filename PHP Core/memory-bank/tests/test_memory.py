#!/usr/bin/env python3
"""Contract tests for the argument-free unified memory agent command."""

from __future__ import annotations

import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SKILL_PATHS = (
    ".claude/skills/memory/SKILL.md",
    ".cursor/skills/memory/SKILL.md",
    ".agents/skills/memory/SKILL.md",
)
COMMAND_PATHS = (
    (".claude/commands/memory.md", ".claude/skills/memory/SKILL.md"),
    (".cursor/commands/memory.md", ".cursor/skills/memory/SKILL.md"),
)


class MemoryIntegrationTest(unittest.TestCase):
    def test_tool_skills_are_byte_identical(self) -> None:
        contents = [
            REPOSITORY_ROOT.joinpath(path).read_bytes()
            for path in SKILL_PATHS
        ]

        self.assertEqual(contents[0], contents[1])
        self.assertEqual(contents[0], contents[2])

    def test_skill_defines_the_unified_memory_contract(self) -> None:
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(
            encoding="utf-8"
        )
        required = (
            "description: Use when",
            "AI skill/command name, not a shell executable",
            ".agents/skills/checkpoint/SKILL.md",
            "referenced procedure",
            "does not invoke or chain another skill",
            "git rev-parse --show-toplevel",
            "git status --porcelain=v1 -z --untracked-files=all",
            "git symbolic-ref --quiet --short HEAD",
            "clean tree",
            "detached HEAD",
            "invalid Context Engine task ID",
            "Working Memory failure",
            "python3 memory-bank/scripts/context.py index --json",
            "working: updated | skipped | failed",
            "procedural: updated | failed",
            "semantic: updated | failed",
            "episodic: updated | failed",
            "MUST NOT run `complete`, `record`, or `clear`",
            "ignored `memory-bank/local/context.db`",
        )

        for text in required:
            with self.subTest(text=text):
                self.assertIn(text, skill)

        self.assertLess(
            skill.index("Working Memory failure"),
            skill.index("python3 memory-bank/scripts/context.py index --json"),
        )

    def test_command_wrappers_accept_no_user_arguments(self) -> None:
        forbidden = (
            "--task-id",
            "--summary",
            "--complete",
            "--outcome",
            "--verification",
        )

        for command_path, skill_path in COMMAND_PATHS:
            with self.subTest(command=command_path):
                command = REPOSITORY_ROOT.joinpath(command_path).read_text(
                    encoding="utf-8"
                )
                self.assertIn(skill_path, command)
                self.assertIn("accepts no arguments", command)
                for option in forbidden:
                    self.assertNotIn(option, command)


if __name__ == "__main__":
    unittest.main()
