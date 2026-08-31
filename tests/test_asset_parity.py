#!/usr/bin/env python3
"""Regression tests for scripts/asset_parity.py.

The point of the checker is that it fails on real drift, so every test
here injects one concrete kind of drift into a synthetic repository and
asserts the finding. A parity gate that only ever passes is the defect it
was written to prevent, one level up.

The fixture builds a miniature monorepo - one canonical edition plus a
memory-seed asset tree - in a temporary directory, so nothing inside the
repository is read or written. One test runs the real asset tree to
assert the shipped state is clean.

Run: python3 -m unittest tests.test_asset_parity
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

import asset_parity  # noqa: E402


RUNTIME_JSON = {
    "automatic_completion": False,
    "automatic_promotion": True,
    "canonical_edition": ".agents",
    "framework": "laravel",
    "mode": "governed",
}

RUNTIME_TEMPLATE = {
    "automatic_completion": True,
    "automatic_promotion": True,
    "canonical_edition": "{{CANONICAL_EDITION}}",
    "framework": "{{TARGET_FRAMEWORK}}",
    "mode": "governed",
}

RUNTIME_CONTRACT = {
    "schema_version": "1.0",
    "contract_id": "context-brain-runtime",
    "path_contracts": {
        "required_skeleton": [
            "memory-bank/scripts/context.py",
            "memory-bank/scripts/telemetry.py",
            "project-brain/PROTOCOL.md",
            "project-brain/schemas/",
        ]
    },
}


class AssetParityFixture(unittest.TestCase):
    """A synthetic repository whose asset starts out in perfect parity."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="asset-parity-test-")
        self.repo = Path(self.temporary.name)
        self.edition = self.repo / "Laravel"
        self.asset = (
            self.repo
            / "Infrastructure-Creator"
            / ".agents"
            / "skills"
            / "memory-seed"
            / "assets"
        )
        self.build()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write(self, root: Path, rel: str, content: str) -> Path:
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def build(self) -> None:
        """Both sides of a byte-identical pair, plus the justified exceptions."""
        core = {
            "memory-bank/scripts/context.py": "# context\n",
            "memory-bank/scripts/telemetry.py": "# telemetry\n",
            "memory-bank/templates/chunk.md": '{"id": "MEM-YYYYMMDD-xxxxxxxx"}\n',
            "project-brain/PROTOCOL.md": "# protocol\n",
            "project-brain/README.md": "# brain\n",
            "project-brain/.gitignore": "local/\n",
            "project-brain/schemas/handoff.schema.json": "{}\n",
            "project-brain/scripts/validate.py": "# validate\n",
            "project-brain/templates/task.md": "task\n",
            "project-brain/indexes/active.json": "[]\n",
            "project-brain/config/providers.json": "{}\n",
            "project-brain/config/telemetry.json": "{}\n",
            "project-brain/dynamic/tasks/.gitkeep": "\n",
        }
        for edition_rel, content in core.items():
            self.write(self.edition, edition_rel, content)
            asset_rel = asset_parity.asset_path_for(edition_rel)
            assert asset_rel is not None, edition_rel
            self.write(self.asset, asset_rel, content)

        # Justified one-sided files.
        self.write(
            self.edition,
            "project-brain/config/runtime.json",
            json.dumps(RUNTIME_JSON, indent=2) + "\n",
        )
        self.write(
            self.asset,
            "project-brain/config/runtime.json.template",
            json.dumps(RUNTIME_TEMPLATE, indent=2) + "\n",
        )
        self.write(
            self.asset,
            "runtime-contract.json",
            json.dumps(RUNTIME_CONTRACT, indent=2) + "\n",
        )
        # Edition-only by design: must not be demanded of the asset.
        self.write(self.edition, "memory-bank/README.md", "# per-target prose\n")
        self.write(self.edition, "memory-bank/tests/test_context.py", "# tests\n")
        self.write(self.edition, "project-brain/tests/test_runtime.py", "# tests\n")
        self.write(self.edition, "project-brain/.install/active.json", "[]\n")

    def findings(self) -> list[dict[str, str]]:
        return asset_parity.collect_findings(self.repo, self.asset)

    def reasons(self) -> str:
        return "\n".join(f"{f['path']}: {f['reason']}" for f in self.findings())


class CleanFixtureTests(AssetParityFixture):
    def test_matched_asset_reports_no_findings(self) -> None:
        self.assertEqual(self.findings(), [], self.reasons())

    def test_edition_only_paths_are_not_demanded_of_the_asset(self) -> None:
        # README.md, both test suites and .install/ exist only in the edition
        # and are listed in EDITION_ONLY; none may surface as a finding.
        self.assertNotIn("README", self.reasons())
        self.assertNotIn("tests", self.reasons())
        self.assertNotIn(".install", self.reasons())

    def test_pycache_is_ignored_on_both_sides(self) -> None:
        self.write(self.asset, "scripts/__pycache__/context.cpython-39.pyc", "x")
        self.write(
            self.edition, "memory-bank/scripts/__pycache__/context.cpython-39.pyc", "x"
        )
        self.assertEqual(self.findings(), [], self.reasons())


class DriftDetectionTests(AssetParityFixture):
    def test_content_drift_in_a_core_module_is_reported(self) -> None:
        self.write(self.edition, "memory-bank/scripts/context.py", "# context v2\n")
        self.assertIn("scripts/context.py: content differs", self.reasons())

    def test_content_drift_in_a_seeded_template_is_reported(self) -> None:
        self.write(self.asset, "templates/chunk.md", '{"id": "MEM-0000"}\n')
        self.assertIn("templates/chunk.md: content differs", self.reasons())

    def test_new_canonical_module_missing_from_the_asset_is_reported(self) -> None:
        self.write(self.edition, "memory-bank/scripts/gate.py", "# gate\n")
        self.assertIn("missing from the asset", self.reasons())
        self.assertIn("scripts/gate.py", self.reasons())

    def test_new_canonical_schema_missing_from_the_asset_is_reported(self) -> None:
        self.write(self.edition, "project-brain/schemas/gate.schema.json", "{}\n")
        self.assertIn("project-brain/schemas/gate.schema.json", self.reasons())

    def test_asset_file_with_no_canonical_counterpart_is_reported(self) -> None:
        self.write(self.asset, "scripts/invented.py", "# invented\n")
        self.assertIn("no counterpart at Laravel/", self.reasons())

    def test_unmappable_asset_file_is_reported_rather_than_skipped(self) -> None:
        self.write(self.asset, "extras/notes.md", "notes\n")
        self.assertIn("maps onto no canonical path", self.reasons())

    def test_runtime_setting_absent_from_the_template_is_reported(self) -> None:
        settings = dict(RUNTIME_JSON, retrieval_gate="shadow")
        self.write(
            self.edition,
            "project-brain/config/runtime.json",
            json.dumps(settings, indent=2) + "\n",
        )
        reasons = self.reasons()
        self.assertIn("runtime.json.template", reasons)
        self.assertIn("retrieval_gate", reasons)

    def test_template_key_absent_from_the_edition_is_reported(self) -> None:
        settings = dict(RUNTIME_TEMPLATE, removed_setting=1)
        self.write(
            self.asset,
            "project-brain/config/runtime.json.template",
            json.dumps(settings, indent=2) + "\n",
        )
        self.assertIn("removed_setting", self.reasons())

    def test_contract_requiring_a_module_the_asset_lacks_is_reported(self) -> None:
        (self.asset / "scripts" / "telemetry.py").unlink()
        (self.edition / "memory-bank" / "scripts" / "telemetry.py").unlink()
        reasons = self.reasons()
        self.assertIn("runtime-contract.json requires", reasons)
        self.assertIn("telemetry.py", reasons)


class SyncTests(AssetParityFixture):
    def test_write_repairs_content_drift_and_missing_files(self) -> None:
        self.write(self.edition, "memory-bank/scripts/context.py", "# context v2\n")
        self.write(self.edition, "memory-bank/scripts/gate.py", "# gate\n")
        asset_parity.sync(self.repo, self.asset, self.findings())
        self.assertEqual(self.findings(), [], self.reasons())
        self.assertEqual(
            (self.asset / "scripts" / "context.py").read_text(encoding="utf-8"),
            "# context v2\n",
        )

    def test_write_refuses_findings_that_need_a_human_decision(self) -> None:
        self.write(self.asset, "extras/notes.md", "notes\n")
        asset_parity.sync(self.repo, self.asset, self.findings())
        # An unmappable asset file has no canonical source to copy from, so it
        # must survive --write as an unresolved finding rather than vanish.
        self.assertIn("maps onto no canonical path", self.reasons())
        self.assertTrue((self.asset / "extras" / "notes.md").is_file())


class ShippedAssetTests(unittest.TestCase):
    """The repository's own asset tree, through the real CLI."""

    def test_shipped_asset_is_in_parity(self) -> None:
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts" / "asset_parity.py"), "--check"],
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_json_mode_is_machine_readable(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts" / "asset_parity.py"),
                "--check",
                "--json",
            ],
            text=True,
            capture_output=True,
        )
        payload = json.loads(result.stdout)
        self.assertEqual(payload["findings"], [])
        self.assertIn(payload["canonical_edition"], asset_parity.CANONICAL_EDITIONS)


if __name__ == "__main__":
    unittest.main()
