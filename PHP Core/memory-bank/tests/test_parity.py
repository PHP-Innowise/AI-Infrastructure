#!/usr/bin/env python3
"""Tests for the full mirror parity checker and the cross-edition mode.

The fixtures build a miniature edition (or monorepo) in a temporary
directory and drive the real CLI, so what is asserted here is the exact
contract a copied-out edition ships with.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "context.py"


def run_cli(repository: Path, *arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--root", str(repository), *arguments],
        text=True,
        capture_output=True,
    )


class ParityFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="parity-test-")
        self.base = Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write(self, repository: Path, rel: str, content: str) -> Path:
        path = repository / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def write_config(self, repository: Path, framework: str = "laravel") -> None:
        self.write(
            repository,
            "project-brain/config/runtime.json",
            json.dumps(
                {
                    "framework": framework,
                    "canonical_edition": ".agents",
                    "mode": "lightweight",
                }
            ),
        )


# Canonical hook exercising the documented rewrites (SKILLS_DIR, temp-file
# namespace, event header); the Cursor mirror below is what MIRROR_RULES
# derives from it.
HOOK_CANON = (
    "#!/bin/bash\n"
    "# Claude hook event: PreToolUse (Write|Edit).\n"
    'SKILLS_DIR="$ROOT_DIR/.claude/skills"\n'
    'COUNTER=/tmp/claude-loop-detection-$KEY\n'
    'echo "$SKILLS_DIR $COUNTER"\n'
)
HOOK_CURSOR = (
    "#!/bin/bash\n"
    "# Cursor hook event: afterFileEdit.\n"
    'SKILLS_DIR="$ROOT_DIR/.cursor/skills"\n'
    'COUNTER=/tmp/cursor-loop-detection-$KEY\n'
    'echo "$SKILLS_DIR $COUNTER"\n'
)

COMMAND_CANON = (
    "---\n"
    "spawns: coder-agent\n"
    "---\n"
    "Deploy the application safely.\n"
    "\n"
    "Use .claude/skills/release/SKILL.md for the checklist.\n"
)
COMMAND_CURSOR = (
    "---\n"
    "name: deploy\n"
    'description: "Deploy the application safely."\n'
    "---\n"
    "Deploy the application safely.\n"
    "\n"
    "Use .cursor/skills/release/SKILL.md for the checklist.\n"
)

AGENT_CANON = (
    "---\n"
    "name: helper-agent\n"
    "description: Helps.\n"
    "model: inherit\n"
    "invokes: helper\n"
    "phase: build\n"
    "---\n"
    "Shared agent body.\n"
)
AGENT_CURSOR = (
    "---\n"
    "name: helper-agent\n"
    "description: Helps.\n"
    "---\n"
    "Shared agent body.\n"
)

DOD_CANON = "# DOD\n\nSee .claude/GOLDEN-PRINCIPLES.md and .claude/skills/verify/.\n"
DOD_CURSOR = "# DOD\n\nSee .cursor/GOLDEN-PRINCIPLES.md and .cursor/skills/verify/.\n"
DOD_CODEX = "# DOD\n\nSee .codex/GOLDEN-PRINCIPLES.md and .agents/skills/verify/.\n"


class FullMirrorParityTest(ParityFixture):
    def build_edition(self, framework: str = "laravel") -> Path:
        repository = self.base / "edition"
        self.write_config(repository, framework)
        for edition in (".agents", ".claude", ".cursor"):
            self.write(
                repository, f"{edition}/skills/review/SKILL.md", "# review\n\nshared\n"
            )
            self.write(
                repository, f"{edition}/skills/review/helper.py", "print('shared')\n"
            )
        self.write(repository, ".claude/hooks/guard.sh", HOOK_CANON)
        self.write(repository, ".cursor/hooks/guard.sh", HOOK_CURSOR)
        self.write(repository, ".claude/commands/deploy.md", COMMAND_CANON)
        self.write(repository, ".cursor/commands/deploy.md", COMMAND_CURSOR)
        self.write(repository, ".claude/agents/helper-agent.md", AGENT_CANON)
        self.write(repository, ".cursor/agents/helper-agent.md", AGENT_CURSOR)
        self.write(repository, ".claude/DOD.md", DOD_CANON)
        self.write(repository, ".cursor/DOD.md", DOD_CURSOR)
        self.write(repository, ".codex/DOD.md", DOD_CODEX)
        return repository

    def parity(self, repository: Path) -> tuple[int, dict]:
        result = run_cli(repository, "parity", "--json")
        payload = json.loads(result.stdout) if result.stdout.strip() else {}
        return result.returncode, payload

    def drift_by_path(self, payload: dict) -> dict[str, dict]:
        return {item["path"]: item for item in payload.get("mirror_drift", [])}

    def test_conforming_edition_passes_every_class(self) -> None:
        repository = self.build_edition()
        code, payload = self.parity(repository)
        self.assertEqual(0, code, payload)
        self.assertTrue(payload["valid"])
        self.assertEqual([], payload["drift"])
        self.assertEqual([], payload["mirror_drift"])

    def test_hook_drift_is_caught_with_class_and_reason(self) -> None:
        repository = self.build_edition()
        # A hand-edited mirror: the derived rewrite is bypassed.
        self.write(
            repository, ".cursor/hooks/guard.sh", HOOK_CURSOR + "echo extra\n"
        )
        code, payload = self.parity(repository)
        self.assertEqual(1, code)
        drift = self.drift_by_path(payload)
        self.assertIn(".cursor/hooks/guard.sh", drift)
        self.assertEqual("hooks", drift[".cursor/hooks/guard.sh"]["class"])
        self.assertIn("differs from canon", drift[".cursor/hooks/guard.sh"]["reason"])

    def test_hook_mirror_must_apply_the_rewrites_not_copy_bytes(self) -> None:
        repository = self.build_edition()
        # A byte-copy of the canon is drift: the Cursor mirror must speak
        # about .cursor/skills and its own temp-file namespace.
        self.write(repository, ".cursor/hooks/guard.sh", HOOK_CANON)
        code, payload = self.parity(repository)
        self.assertEqual(1, code)
        self.assertIn(".cursor/hooks/guard.sh", self.drift_by_path(payload))

    def test_command_drift_and_stray_mirror_command_are_caught(self) -> None:
        repository = self.build_edition()
        self.write(
            repository, ".cursor/commands/deploy.md", COMMAND_CURSOR.replace("safely", "fast")
        )
        self.write(repository, ".cursor/commands/stray.md", "Mirror-only command.\n")
        code, payload = self.parity(repository)
        self.assertEqual(1, code)
        drift = self.drift_by_path(payload)
        self.assertEqual("commands", drift[".cursor/commands/deploy.md"]["class"])
        self.assertIn("no source", drift[".cursor/commands/stray.md"]["reason"])

    def test_agent_keeping_claude_frontmatter_is_caught(self) -> None:
        repository = self.build_edition()
        self.write(repository, ".cursor/agents/helper-agent.md", AGENT_CANON)
        code, payload = self.parity(repository)
        self.assertEqual(1, code)
        drift = self.drift_by_path(payload)
        self.assertEqual("agents", drift[".cursor/agents/helper-agent.md"]["class"])

    def test_governance_doc_with_unrewritten_self_links_is_caught(self) -> None:
        repository = self.build_edition()
        self.write(repository, ".codex/DOD.md", DOD_CANON)
        code, payload = self.parity(repository)
        self.assertEqual(1, code)
        drift = self.drift_by_path(payload)
        self.assertEqual("governance-docs", drift[".codex/DOD.md"]["class"])

    def test_non_markdown_skill_file_drift_is_caught(self) -> None:
        repository = self.build_edition()
        self.write(
            repository, ".cursor/skills/review/helper.py", "print('drifted')\n"
        )
        code, payload = self.parity(repository)
        self.assertEqual(1, code)
        drift = self.drift_by_path(payload)
        self.assertEqual("skills", drift[".cursor/skills/review/helper.py"]["class"])

    def test_documented_exemptions_do_not_report_drift(self) -> None:
        repository = self.build_edition()
        # skill-creator: tool-owned, three deliberate generations.
        self.write(
            repository, ".agents/skills/skill-creator/SKILL.md", "codex generation\n"
        )
        self.write(
            repository, ".claude/skills/skill-creator/SKILL.md", "claude generation\n"
        )
        # SKILL FLOW.md: .claude and .cursor share one copy; .agents adapts.
        self.write(repository, ".claude/skills/SKILL FLOW.md", "slash commands\n")
        self.write(repository, ".cursor/skills/SKILL FLOW.md", "slash commands\n")
        self.write(repository, ".agents/skills/SKILL FLOW.md", "bare skill names\n")
        # hooks README: each tool documents its own registration model.
        self.write(repository, ".claude/hooks/README.md", "settings.json\n")
        self.write(repository, ".cursor/hooks/README.md", "hooks.json\n")
        # working-memory-read.sh: Cursor has no prompt-submit hook event.
        self.write(repository, ".claude/hooks/working-memory-read.sh", HOOK_CANON)
        code, payload = self.parity(repository)
        self.assertEqual(0, code, payload)
        self.assertEqual([], payload["mirror_drift"])
        self.assertEqual([], payload["drift"])

    def test_framework_keyed_exemption_applies_only_to_that_framework(self) -> None:
        rewritten = "---\nname: skill-creator\ndescription: rewritten\n---\nCursor.\n"
        symfony = self.base / "symfony-edition"
        self.write_config(symfony, framework="symfony")
        self.write(symfony, ".claude/commands/skill-creator.md", COMMAND_CANON)
        self.write(symfony, ".cursor/commands/skill-creator.md", rewritten)
        code, payload = self.parity(symfony)
        self.assertEqual(0, code, payload)

        laravel = self.base / "laravel-edition"
        self.write_config(laravel, framework="laravel")
        self.write(laravel, ".claude/commands/skill-creator.md", COMMAND_CANON)
        self.write(laravel, ".cursor/commands/skill-creator.md", rewritten)
        code, payload = self.parity(laravel)
        self.assertEqual(1, code)
        self.assertIn(
            ".cursor/commands/skill-creator.md", self.drift_by_path(payload)
        )

    def test_skills_only_mode_ignores_non_skill_classes(self) -> None:
        repository = self.build_edition()
        self.write(
            repository, ".cursor/hooks/guard.sh", HOOK_CURSOR + "echo extra\n"
        )
        result = run_cli(repository, "parity", "--skills-only", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        payload = json.loads(result.stdout)
        self.assertTrue(payload["valid"])
        self.assertNotIn("mirror_drift", payload)


CORE_FILE = "memory-bank/scripts/context.py"


class CrossEditionParityTest(ParityFixture):
    def build_monorepo(self) -> Path:
        for name in ("Laravel", "Symfony"):
            edition = self.base / name
            self.write_config(edition)
            self.write(edition, CORE_FILE, "print('shared core')\n")
            self.write(
                edition, "memory-bank/tests/test_core.py", "def test(): pass\n"
            )
            self.write(edition, "project-brain/PROTOCOL.md", "# Protocol\n")
            self.write(
                edition, "project-brain/schemas/record.schema.json", "{}\n"
            )
            # The hooks are core too: they are the only automatic entry into
            # the engine, so the manifest holds them byte-identical except
            # for the two that speak the edition's framework.
            self.write(
                edition, ".claude/hooks/working-memory-read.sh", "#!/bin/bash\nread\n"
            )
            self.write(
                edition, ".claude/hooks/subagent-gate.sh", "#!/bin/bash\ngate\n"
            )
        return self.base / "Laravel"

    def cross(self, repository: Path) -> tuple[int, dict]:
        result = run_cli(repository, "parity", "--cross-edition", "--json")
        payload = json.loads(result.stdout) if result.stdout.strip() else {}
        return result.returncode, payload

    def test_identical_core_passes(self) -> None:
        repository = self.build_monorepo()
        code, payload = self.cross(repository)
        self.assertEqual(0, code, payload)
        self.assertTrue(payload["valid"])
        self.assertFalse(payload["skipped"])
        self.assertEqual([], payload["drift"])

    def test_substituted_core_file_is_caught(self) -> None:
        repository = self.build_monorepo()
        self.write(self.base / "Symfony", CORE_FILE, "print('forked core')\n")
        code, payload = self.cross(repository)
        self.assertEqual(1, code)
        drift = {item["path"]: item for item in payload["drift"]}
        self.assertIn(CORE_FILE, drift)
        self.assertEqual("content differs", drift[CORE_FILE]["reason"])
        self.assertEqual(["Laravel", "Symfony"], drift[CORE_FILE]["editions"])

    def test_core_file_missing_from_a_sibling_is_caught(self) -> None:
        repository = self.build_monorepo()
        (self.base / "Symfony" / "project-brain" / "PROTOCOL.md").unlink()
        code, payload = self.cross(repository)
        self.assertEqual(1, code)
        drift = {item["path"]: item for item in payload["drift"]}
        self.assertEqual("missing", drift["project-brain/PROTOCOL.md"]["reason"])
        self.assertEqual(["Symfony"], drift["project-brain/PROTOCOL.md"]["editions"])

    def test_allowed_drift_stays_quiet(self) -> None:
        repository = self.build_monorepo()
        # Edition-specific by design: README and durable-memory chunks
        # (MEM-0001 ships per-edition versions). Neither may be reported even
        # when a future manifest change makes the globs reach them.
        self.write(self.base / "Laravel", "README.md", "Laravel edition\n")
        self.write(self.base / "Symfony", "README.md", "Symfony edition\n")
        self.write(
            self.base / "Laravel",
            "memory-bank/chunks/MEM-0001-cross-edition-sync.md",
            "laravel flavor\n",
        )
        self.write(
            self.base / "Symfony",
            "memory-bank/chunks/MEM-0001-cross-edition-sync.md",
            "symfony flavor\n",
        )
        code, payload = self.cross(repository)
        self.assertEqual(0, code, payload)
        self.assertEqual([], payload["drift"])

    def test_forked_engine_hook_is_caught(self) -> None:
        repository = self.build_monorepo()
        # The defect this covers shipped for real: Symfony and PHP Core
        # truncated the prompt to 24 word characters before handing it to the
        # CLI while Laravel passed it through, so the same request produced a
        # different capsule per edition and nothing reported it.
        self.write(
            self.base / "Symfony",
            ".claude/hooks/working-memory-read.sh",
            "#!/bin/bash\nread truncated\n",
        )
        code, payload = self.cross(repository)
        self.assertEqual(1, code)
        drift = {item["path"]: item for item in payload["drift"]}
        hook = ".claude/hooks/working-memory-read.sh"
        self.assertIn(hook, drift)
        self.assertEqual("content differs", drift[hook]["reason"])

    def test_engine_hook_missing_from_a_sibling_is_caught(self) -> None:
        repository = self.build_monorepo()
        (self.base / "Symfony" / ".claude" / "hooks" / "subagent-gate.sh").unlink()
        code, payload = self.cross(repository)
        self.assertEqual(1, code)
        drift = {item["path"]: item for item in payload["drift"]}
        self.assertEqual("missing", drift[".claude/hooks/subagent-gate.sh"]["reason"])

    def test_framework_shaped_hooks_may_differ(self) -> None:
        repository = self.build_monorepo()
        # bash-validator.sh blocks this framework's destructive commands and
        # local-context.sh detects this framework's stack: both are exempt by
        # name, and the exemption must survive the hook glob reaching them.
        self.write(
            self.base / "Laravel",
            ".claude/hooks/bash-validator.sh",
            "#!/bin/bash\nartisan migrate:fresh\n",
        )
        self.write(
            self.base / "Symfony",
            ".claude/hooks/bash-validator.sh",
            "#!/bin/bash\ndoctrine:schema:drop\n",
        )
        self.write(
            self.base / "Laravel", ".claude/hooks/local-context.sh", "artisan\n"
        )
        self.write(
            self.base / "Symfony", ".claude/hooks/local-context.sh", "bin/console\n"
        )
        code, payload = self.cross(repository)
        self.assertEqual(0, code, payload)
        self.assertEqual([], payload["drift"])

    def test_standalone_edition_skips_politely(self) -> None:
        repository = self.base / "standalone"
        self.write_config(repository)
        self.write(repository, CORE_FILE, "print('core')\n")
        code, payload = self.cross(repository)
        self.assertEqual(0, code, payload)
        self.assertTrue(payload["valid"])
        self.assertTrue(payload["skipped"])
        text = run_cli(repository, "parity", "--cross-edition")
        self.assertEqual(0, text.returncode)
        self.assertIn("skipped", text.stdout)


if __name__ == "__main__":
    unittest.main()
