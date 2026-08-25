#!/usr/bin/env python3
"""Regression tests for scripts/policy_lock.py.

Each test injects one change the existing gates are blind to and asserts the
lock catches it. The two that matter most are the ones the roadmap opens by
citing: a mistyped `model:` value, which no other check in the repository
looks at, and an edit to an agent's body, which is that agent's prompt and
which passes `mirrors`, `context_budget` and `check_stabilization` once the
mirrors have been regenerated.

The fixtures build a miniature edition inside a temporary Git repository,
because the enumerator reads the index rather than the filesystem - that is
the property that keeps the gitignored, per-turn Cursor rule render out of
the lock, so the tests have to exercise it rather than route around it.

Run: python3 -m unittest tests.test_policy_lock
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import policy_lock  # noqa: E402


AGENT = """---
name: coder
description: Implement backend code.
model: sonnet
---

Follow the layered architecture.
"""


class PolicyLockFixture(unittest.TestCase):
    EDITION = "Symfony"

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="policy-lock-test-")
        self.root = Path(self.temporary.name)
        subprocess.run(
            ["git", "init", "--quiet", str(self.root)], check=True, capture_output=True
        )
        self.edition = self.root / self.EDITION
        self.write("AGENTS.md", "# Policy\n\n- MUST do the thing.\n")
        self.write(".claude/DOD.md", "# DOD\n")
        self.write(".claude/agents/coder-agent.md", AGENT)
        self.write(".claude/commands/flow-review.md", "# Flow: Review\n")
        self.write(".agents/skills/coder/SKILL.md", "# Coder\n")
        self.write(".agents/skills/coder/references/patterns.md", "# Patterns\n")
        self.write(".claude/settings.json", '{"hooks": {}}\n')
        self.write("VERSION", "2.0.0\n")
        self.stage()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write(self, relative: str, content: str) -> Path:
        path = self.edition / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def stage(self) -> None:
        subprocess.run(
            ["git", "-C", str(self.root), "add", "-A"], check=True, capture_output=True
        )

    def build(self) -> dict:
        return policy_lock.build_lock(self.EDITION, self.root)

    def write_lock(self) -> dict:
        lock = self.build()
        policy_lock.lock_path(self.EDITION, self.root).write_text(
            json.dumps(lock, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        self.stage()
        return lock

    def findings(self) -> list[str]:
        stored = json.loads(
            policy_lock.lock_path(self.EDITION, self.root).read_text(encoding="utf-8")
        )
        current = self.build()
        return policy_lock.invalid_models(
            self.EDITION, current
        ) + policy_lock.compare(self.EDITION, current, stored)


class SurfaceEnumerationTests(PolicyLockFixture):
    def test_the_lock_covers_the_files_the_model_reads(self) -> None:
        lock = self.build()
        self.assertEqual(
            {
                "AGENTS.md",
                ".claude/DOD.md",
                ".claude/agents/coder-agent.md",
                ".claude/commands/flow-review.md",
                ".agents/skills/coder/SKILL.md",
                ".agents/skills/coder/references/patterns.md",
                ".claude/settings.json",
            },
            set(lock["files"]),
            lock["files"],
        )

    def test_a_skill_reference_file_is_covered_not_only_the_skill_body(self) -> None:
        # An agent reads the references under a skill exactly as it reads the
        # body, and a SKILL.md-only glob would leave them unlocked.
        self.assertIn(".agents/skills/coder/references/patterns.md", self.build()["files"])

    def test_an_untracked_file_never_enters_the_lock(self) -> None:
        # The per-turn Cursor rule render is gitignored and rewritten every
        # turn; enumerating from disk would bake it in and make --check fail
        # for ever.
        self.write(".cursor/rules/working-memory.mdc", "rendered at turn 1\n")
        self.assertNotIn(".cursor/rules/working-memory.mdc", self.build()["files"])

    def test_the_lock_does_not_hash_itself(self) -> None:
        self.write_lock()
        self.assertNotIn(policy_lock.LOCK_NAME, self.build()["files"])
        self.assertEqual([], self.findings())

    def test_release_is_recorded_but_is_not_the_identity(self) -> None:
        # A surface change does not require a VERSION bump; it requires the
        # lock to be regenerated. The two move independently on purpose.
        before = self.build()
        self.write("VERSION", "2.1.0\n")
        self.stage()
        after = self.build()
        self.assertNotEqual(before["release"], after["release"])
        self.assertEqual(before["policy_digest"], after["policy_digest"])


class DriftDetectionTests(PolicyLockFixture):
    def test_an_agent_body_edit_is_caught(self) -> None:
        # The case every other gate misses once mirrors are regenerated: the
        # body is the agent's prompt.
        self.write_lock()
        self.write(
            ".claude/agents/coder-agent.md",
            AGENT + "\nAn instruction nobody reviewed.\n",
        )
        self.stage()
        self.assertIn(".claude/agents/coder-agent.md: content changed", self.findings())

    def test_a_policy_edit_is_caught(self) -> None:
        self.write_lock()
        self.write("AGENTS.md", "# Policy\n\n- MUST do something else.\n")
        self.stage()
        self.assertIn("AGENTS.md: content changed", self.findings())

    def test_a_new_surface_file_is_caught(self) -> None:
        self.write_lock()
        self.write(".agents/skills/refactorer/SKILL.md", "# Refactorer\n")
        self.stage()
        self.assertTrue(
            any("absent from the lock" in finding for finding in self.findings())
        )

    def test_a_removed_surface_file_is_caught(self) -> None:
        self.write_lock()
        (self.edition / ".claude/DOD.md").unlink()
        self.stage()
        self.assertIn(
            ".claude/DOD.md: in the lock, absent from the surface", self.findings()
        )

    def test_a_matching_surface_reports_nothing(self) -> None:
        self.write_lock()
        self.assertEqual([], self.findings())


class ModelAllowlistTests(PolicyLockFixture):
    def test_a_mistyped_model_is_reported(self) -> None:
        self.write_lock()
        self.write(".claude/agents/coder-agent.md", AGENT.replace("sonnet", "sonet"))
        self.stage()
        self.assertTrue(
            any("is not one of" in finding for finding in self.findings())
        )

    def test_write_refuses_to_record_a_mistyped_model(self) -> None:
        # Validating only on --check lets a regeneration launder the typo and
        # then agree with itself, which defeats the point of validating it.
        self.write(".claude/agents/coder-agent.md", AGENT.replace("sonnet", "sonet"))
        self.stage()
        current = self.build()
        self.assertTrue(policy_lock.invalid_models(self.EDITION, current))

    def test_the_allowlist_is_per_edition(self) -> None:
        # The three ready editions use haiku for four narrow agents;
        # Infrastructure-Creator's own agent-forge skill requires opus or
        # sonnet and it ships no haiku agent.
        self.assertIn("haiku", policy_lock.MODEL_ALLOWLIST["Symfony"])
        self.assertNotIn("haiku", policy_lock.MODEL_ALLOWLIST["Infrastructure-Creator"])


class ShippedLockTests(unittest.TestCase):
    def test_every_shipped_edition_matches_its_lock(self) -> None:
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts" / "policy_lock.py"), "--check"],
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_every_shipped_agent_declares_an_allowed_model(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts" / "policy_lock.py"),
                "--check",
                "--json",
            ],
            text=True,
            capture_output=True,
        )
        report = json.loads(result.stdout)
        for edition, entry in report.items():
            with self.subTest(edition=edition):
                self.assertEqual([], entry["findings"])
                self.assertGreater(entry["files"], 0)


if __name__ == "__main__":
    unittest.main()
