#!/usr/bin/env python3
"""Contract tests for the argument-free unified memory agent command."""

from __future__ import annotations

import subprocess
import sys
import tempfile
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
VALIDATOR_PATHS = tuple(
    f"{tool}/skills/skill-creator/scripts/quick_validate.py"
    for tool in (".claude", ".cursor", ".agents")
)


class MemoryIntegrationTest(unittest.TestCase):
    def run_validator(self, validator_path: str, skill_path: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(REPOSITORY_ROOT / validator_path), str(skill_path)],
            capture_output=True,
            check=False,
            cwd=REPOSITORY_ROOT,
            text=True,
        )

    def test_skill_validators_accept_project_metadata(self) -> None:
        for validator_path in VALIDATOR_PATHS:
            tool = Path(validator_path).parts[0]
            for skill_name in ("memory", "checkpoint"):
                with self.subTest(validator=validator_path, skill=skill_name):
                    result = self.run_validator(
                        validator_path,
                        REPOSITORY_ROOT / tool / "skills" / skill_name,
                    )
                self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_skill_validators_reject_malformed_flow_names(self) -> None:
        for validator_path in VALIDATOR_PATHS:
            for value in ("-memory", "memory-", "memory--bank"):
                for field in ("flow-next", "flow-alternatives"):
                    with self.subTest(
                        validator=validator_path, field=field, value=value
                    ):
                        flow_next = f'"{value}"' if field == "flow-next" else "null"
                        alternatives = (
                            f'["{value}"]'
                            if field == "flow-alternatives"
                            else '["checkpoint"]'
                        )
                        with tempfile.TemporaryDirectory() as directory:
                            skill_path = Path(directory) / "skill"
                            skill_path.mkdir()
                            skill_path.joinpath("SKILL.md").write_text(
                                "---\n"
                                "name: memory\n"
                                "description: Use when testing validator metadata.\n"
                                "phase: utility\n"
                                f"flow-next: {flow_next}\n"
                                f"flow-alternatives: {alternatives}\n"
                                "---\n",
                                encoding="utf-8",
                            )
                            result = self.run_validator(validator_path, skill_path)
                        self.assertNotEqual(
                            0, result.returncode, result.stdout + result.stderr
                        )

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
            "Working Memory procedure fails",
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
            skill.index("Working Memory procedure fails"),
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

    def test_policy_separates_memory_checkpoint_and_completion(self) -> None:
        policy = REPOSITORY_ROOT.joinpath("AGENTS.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("argument-free `memory` skill", policy)
        self.assertIn("all four local context layers", policy)
        self.assertIn("referenced procedure", policy)
        self.assertIn("MUST NOT invoke or chain", policy)
        self.assertIn("explicit `complete`", policy)

    def test_readmes_document_the_short_and_explicit_flows(self) -> None:
        memory_readme = REPOSITORY_ROOT.joinpath(
            "memory-bank/README.md"
        ).read_text(encoding="utf-8")
        accelerator_readme = REPOSITORY_ROOT.joinpath("README.md").read_text(
            encoding="utf-8"
        )
        workspace_root = REPOSITORY_ROOT.parent
        workspace_readme = workspace_root.joinpath("README.md").read_text(
            encoding="utf-8"
        )
        english_readme = workspace_root.joinpath("README_EN.md").read_text(
            encoding="utf-8"
        )
        russian_readme = workspace_root.joinpath("README_RU.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("AI command `memory`", memory_readme)
        self.assertIn("Use `memory` in Codex or `/memory`", accelerator_readme)
        self.assertIn("[English](README_EN.md)", workspace_readme)
        self.assertIn("[Русский](README_RU.md)", workspace_readme)
        self.assertIn("`memory` без", russian_readme)
        self.assertIn("invoke `memory` without", english_readme)
        for document in (
            memory_readme,
            accelerator_readme,
            english_readme,
            russian_readme,
        ):
            self.assertIn("checkpoint", document)
            self.assertIn("complete", document)

    def test_policy_and_docs_define_hybrid_task_capsules(self) -> None:
        policy = REPOSITORY_ROOT.joinpath("AGENTS.md").read_text(encoding="utf-8")
        required_policy = (
            "Task Capsule",
            "8,000 Unicode characters",
            "two Procedural",
            "three Semantic",
            "one Episodic",
            "MUST NOT pass the parent conversation",
            "start of a complex request",
            "research to planning",
            "planning to implementation",
            "implementation to independent verification",
            "simple task",
            "explicit `complete`",
        )
        for text in required_policy:
            with self.subTest(policy=text):
                self.assertIn(text, policy)

        for tool in (".agents", ".claude", ".cursor"):
            flow = REPOSITORY_ROOT.joinpath(
                tool, "skills", "SKILL FLOW.md"
            ).read_text(encoding="utf-8")
            with self.subTest(tool=tool):
                self.assertIn("## Task Capsule Handoff", flow)
                self.assertIn("work completed", flow)
                self.assertIn("verification evidence", flow)
                self.assertIn("unresolved blockers", flow)
                self.assertIn("must not preload every cited source", flow)

        documents = (
            REPOSITORY_ROOT / "README.md",
            REPOSITORY_ROOT / "memory-bank/README.md",
            REPOSITORY_ROOT.parent / "README_EN.md",
            REPOSITORY_ROOT.parent / "README_RU.md",
        )
        for path in documents:
            text = path.read_text(encoding="utf-8")
            with self.subTest(document=path):
                self.assertIn("Task Capsule", text)
                self.assertIn("8", text)
                self.assertIn("Procedural", text)
                self.assertIn("Semantic", text)
                self.assertIn("Episodic", text)

    def test_skill_flow_discovers_memory(self) -> None:
        expected = {
            ".claude": "`/checkpoint`, `/memory`",
            ".cursor": "`/checkpoint`, `/memory`",
            ".agents": "`checkpoint`, `memory`",
        }
        for tool, entry in expected.items():
            with self.subTest(tool=tool):
                flow = REPOSITORY_ROOT.joinpath(
                    tool, "skills", "SKILL FLOW.md"
                ).read_text(encoding="utf-8")
                self.assertIn(entry, flow)


if __name__ == "__main__":
    unittest.main()
