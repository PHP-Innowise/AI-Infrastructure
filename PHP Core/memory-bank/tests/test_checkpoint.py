#!/usr/bin/env python3
"""Contract tests for the argument-free checkpoint agent command."""

from __future__ import annotations

import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SKILL_PATHS = (
    ".claude/skills/checkpoint/SKILL.md",
    ".cursor/skills/checkpoint/SKILL.md",
    ".agents/skills/checkpoint/SKILL.md",
)
COMMAND_PATHS = (
    (".claude/commands/checkpoint.md", ".claude/skills/checkpoint/SKILL.md"),
    (".cursor/commands/checkpoint.md", ".cursor/skills/checkpoint/SKILL.md"),
)


class CheckpointIntegrationTest(unittest.TestCase):
    def test_tool_skills_are_byte_identical(self) -> None:
        contents = [
            REPOSITORY_ROOT.joinpath(path).read_bytes()
            for path in SKILL_PATHS
        ]

        self.assertEqual(contents[0], contents[1])
        self.assertEqual(contents[0], contents[2])

    def test_skill_defines_the_checkpoint_contract(self) -> None:
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(
            encoding="utf-8"
        )
        required = (
            "git rev-parse --show-toplevel",
            "git symbolic-ref --quiet --short HEAD",
            "git status --porcelain=v1 -z --untracked-files=all",
            "staged, unstaged, and untracked",
            "renamed, copied, and deleted",
            "memory-bank/scripts/context.py get",
            "memory-bank/scripts/context.py start",
            "memory-bank/scripts/context.py update",
            "Checkpoint work on branch",
            "MUST NOT read binary",
            "MUST NOT read `.env`",
            "MUST NOT store raw diffs",
            "MUST NOT run `complete`",
            "no current Git changes",
            "detached HEAD",
        )

        for text in required:
            with self.subTest(text=text):
                self.assertIn(text, skill)

        safe_filter = (
            "Before obtaining any diff, filter staged, unstaged, and untracked "
            "paths to safe non-sensitive text files."
        )
        excluded_paths = "Record excluded paths from porcelain metadata only."
        inspection = "Inspect the staged and unstaged diffs plus safe untracked text files only"

        self.assertIn(safe_filter, skill)
        self.assertIn(excluded_paths, skill)
        self.assertLess(skill.index(safe_filter), skill.index(inspection))

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

    def test_policy_allows_argument_free_checkpoint(self) -> None:
        policy = REPOSITORY_ROOT.joinpath("AGENTS.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("checkpoint", policy)
        self.assertIn("current Git branch", policy)
        self.assertIn("automatically creates or updates", policy)

    def test_readmes_document_checkpoint_without_hiding_completion(self) -> None:
        memory_readme = REPOSITORY_ROOT.joinpath(
            "memory-bank/README.md"
        ).read_text(encoding="utf-8")
        accelerator_readme = REPOSITORY_ROOT.joinpath("README.md").read_text(
            encoding="utf-8"
        )

        for document in (memory_readme, accelerator_readme):
            self.assertIn("checkpoint", document)
            self.assertIn("complete", document)


if __name__ == "__main__":
    unittest.main()
