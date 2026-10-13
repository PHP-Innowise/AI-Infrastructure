#!/usr/bin/env python3
"""Contract tests for authority-aware checkpoints."""

from __future__ import annotations

import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
SKILL_PATHS = tuple(
    f"{tool}/skills/checkpoint/SKILL.md"
    for tool in (".agents", ".claude", ".cursor")
)


class CheckpointIntegrationTest(unittest.TestCase):
    def test_tool_skills_are_byte_identical(self) -> None:
        contents = [
            REPOSITORY_ROOT.joinpath(path).read_bytes() for path in SKILL_PATHS
        ]
        self.assertEqual(contents[0], contents[1])
        self.assertEqual(contents[0], contents[2])

    def test_governed_mode_never_creates_duplicate_task_authority(self) -> None:
        """Governed mode records into the one branch task the hooks keep -
        flushing the buffered turns, then a revision-checked update - instead
        of declining, which left the skill contradicting the hooks."""
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(encoding="utf-8")
        self.assertIn("If its mode is\n   `governed`", skill)
        self.assertIn("context.py turn --task-id <task> --flush", skill)
        self.assertIn("--revision auto", skill)
        self.assertIn("`CONTEXT_TASK_ID`, otherwise", skill)
        self.assertIn("MUST NOT create a second task authority in governed mode", skill)
        self.assertLess(
            skill.index("## Authority Gate"),
            skill.index("## Governed Workflow"),
        )
        self.assertLess(
            skill.index("## Governed Workflow"),
            skill.index("## Lightweight Workflow"),
        )

    def test_lightweight_checkpoint_preserves_safety_and_provenance(self) -> None:
        skill = REPOSITORY_ROOT.joinpath(SKILL_PATHS[0]).read_text(encoding="utf-8")
        required = (
            "--mode lightweight",
            "git status --porcelain=v1 -z --untracked-files=all",
            "git --literal-pathspecs diff --no-ext-diff --no-textconv --no-renames -- <safe-paths>",
            "git --literal-pathspecs diff --cached --no-ext-diff --no-textconv --no-renames -- <safe-paths>",
            "one `--file` argv value per exact changed path",
            "preserving leading and\n   trailing whitespace",
            "MUST NOT run unscoped `git diff`, `--stat`, or `--numstat`",
            "MUST NOT run `complete`, `record`, or `clear`",
        )
        for text in required:
            with self.subTest(text=text):
                self.assertIn(text, skill)

    def test_commands_accept_no_arguments(self) -> None:
        for tool in (".claude", ".cursor"):
            command = REPOSITORY_ROOT.joinpath(
                tool, "commands/checkpoint.md"
            ).read_text(encoding="utf-8")
            self.assertIn("accepts no arguments", command)
            self.assertIn(f"{tool}/skills/checkpoint/SKILL.md", command)


if __name__ == "__main__":
    unittest.main()
