#!/usr/bin/env python3
"""Prevent shared-core synchronization from erasing framework specialization."""

from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
EDITIONS = (".agents", ".claude", ".cursor")
LEAKED_NATIVE_MARKERS = (
    "This flow keeps native PHP work",
    "# Native PHP Memory Bank",
    "# Native PHP Project Brain",
    "Manage durable native-PHP project memory",
    "Govern shared native-PHP task context",
    "generic PHP advice",
    "module/use-case boundaries",
    "data-access gateways",
    "worker/queue",
)
SKILL_DOCUMENTS = (
    "SKILL FLOW.md",
    "memory-bank/SKILL.md",
    "project-brain/SKILL.md",
)


class FrameworkSemanticPreservationTest(unittest.TestCase):
    def read(self, framework: str, edition: str, relative: str) -> str:
        return (ROOT / framework / edition / "skills" / relative).read_text(
            encoding="utf-8"
        )

    def assert_shared_context_updates(self, framework: str, edition: str) -> None:
        memory = self.read(framework, edition, "memory-bank/SKILL.md")
        brain = self.read(framework, edition, "project-brain/SKILL.md")

        self.assertIn("approved-without-review", memory)
        self.assertIn("reviewer: null", memory)
        self.assertIn("independently reviewed", memory)
        for phase in (
            "understanding",
            "planning",
            "implementation",
            "verification",
            "finalization",
        ):
            self.assertIn(phase, brain)
        self.assertIn("merge completion candidate is advisory", brain.lower())
        self.assertIn("2 procedural, 3 semantic, and 1 episodic", brain)
        self.assertIn("8,000 serialized characters", brain)
        self.assertIn("automatic_promotion=false", brain)
        self.assertIn("brain-create", brain)
        self.assertIn("brain-update", brain)
        self.assertIn("brain-get", brain)

    def assert_no_native_php_replacement(self, *documents: str) -> None:
        combined = "\n".join(documents)
        for marker in LEAKED_NATIVE_MARKERS:
            self.assertNotIn(marker, combined)

    def assert_not_php_core_copy(self, framework: str, edition: str) -> None:
        for relative in SKILL_DOCUMENTS:
            with self.subTest(edition=edition, relative=relative):
                self.assertNotEqual(
                    self.read(framework, edition, relative),
                    self.read("PHP Core", edition, relative),
                )

    def assert_mirror_invariants(self, framework: str) -> None:
        for relative in ("memory-bank/SKILL.md", "project-brain/SKILL.md"):
            canonical = self.read(framework, ".agents", relative)
            self.assertEqual(canonical, self.read(framework, ".claude", relative))
            self.assertEqual(canonical, self.read(framework, ".cursor", relative))
        self.assertEqual(
            self.read(framework, ".claude", "SKILL FLOW.md"),
            self.read(framework, ".cursor", "SKILL FLOW.md"),
        )
        self.assertNotEqual(
            self.read(framework, ".agents", "SKILL FLOW.md"),
            self.read(framework, ".claude", "SKILL FLOW.md"),
        )

    def test_laravel_skills_keep_laravel_routing_and_shared_updates(self) -> None:
        required_flow_markers = (
            "eloquent",
            "queues-jobs",
            "events-notifications",
            "auth-scaffolding",
            "caching",
            "console-scheduler",
            "file-storage",
            "package-developer",
            "filament",
        )
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                flow = self.read("Laravel", edition, "SKILL FLOW.md")
                memory = self.read("Laravel", edition, "memory-bank/SKILL.md")
                brain = self.read("Laravel", edition, "project-brain/SKILL.md")

                self.assertIn("Laravel work", flow)
                for marker in required_flow_markers:
                    self.assertIn(marker, flow)
                self.assertIn("# Laravel Memory Bank", memory)
                self.assertIn("Eloquent model boundaries", memory)
                self.assertIn("MEM-YYYYMMDD-xxxxxxxx", memory)
                self.assertIn("reindex-bank", memory)
                self.assertIn(".memory-counter` as retired", memory)
                self.assertIn("promote-apply --promotion-id ID", memory)
                self.assertIn("# Laravel Project Brain", brain)
                self.assertIn("generic Laravel advice", brain)
                for command in (
                    "--owner OWNER start --task-id ID --goal GOAL",
                    "update --task-id ID --revision REVISION",
                    "complete --task-id ID --revision REVISION",
                    "promote-propose --source-id UUID",
                    "context.py validate",
                    "context.py parity",
                ):
                    self.assertIn(command, brain)
                self.assert_no_native_php_replacement(flow, memory, brain)
                self.assert_shared_context_updates("Laravel", edition)
                self.assert_not_php_core_copy("Laravel", edition)

                if edition == ".agents":
                    self.assertIn("Use `project-brain`", flow)
                    self.assertNotIn("Use `/project-brain`", flow)
                else:
                    self.assertIn("Use `/project-brain`", flow)
        self.assert_mirror_invariants("Laravel")

    def test_symfony_skills_keep_symfony_routing_and_shared_updates(self) -> None:
        required_flow_markers = (
            "doctrine-migration-designer",
            "api-platform-designer",
            "security-voter-designer",
            "form-validator-designer",
            "messenger-designer",
            "event-subscriber-designer",
            "console-command-coder",
            "fixture-factory-generator",
            "architecture-boundary-reviewer",
            "repository-reviewer",
            "twig-ux-reviewer",
            "container-reviewer",
        )
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                flow = self.read("Symfony", edition, "SKILL FLOW.md")
                memory = self.read("Symfony", edition, "memory-bank/SKILL.md")
                brain = self.read("Symfony", edition, "project-brain/SKILL.md")

                self.assertIn("Symfony", flow)
                self.assertIn("Doctrine", flow)
                self.assertIn("Messenger", flow)
                self.assertIn("specs/MANIFEST.md", flow)
                self.assertIn("examples/symfony-clean-code-patterns.md", flow)
                for marker in required_flow_markers:
                    self.assertIn(marker, flow)
                self.assertIn("# Symfony Memory Bank", memory)
                self.assertIn("Controller -> Service -> Repository", memory)
                self.assertIn("Messenger worker", memory)
                self.assertIn("# Symfony Project Brain", brain)
                self.assertIn("generic Symfony advice", brain)
                self.assert_no_native_php_replacement(flow, memory, brain)
                self.assert_shared_context_updates("Symfony", edition)
                self.assert_not_php_core_copy("Symfony", edition)

                if edition == ".agents":
                    self.assertIn("using-git-worktrees", flow)
                    self.assertIn("systematic-debugger", flow)
                    self.assertNotIn("/requirements-analyst", flow)
                else:
                    self.assertIn("/git-worktrees", flow)
                    self.assertIn("/debugger", flow)
                    self.assertIn("/requirements-analyst", flow)
        self.assert_mirror_invariants("Symfony")


if __name__ == "__main__":
    unittest.main()
