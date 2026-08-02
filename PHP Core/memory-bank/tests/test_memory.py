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
            "context.py refresh --json",
            "context.py status --json",
            "MUST NOT pass `--query` to `refresh`",
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

        self.assertIn("AI skill `memory`", memory_readme)
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
