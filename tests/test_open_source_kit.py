#!/usr/bin/env python3
"""Tests for the Kit 3 (open-source) catalog and its selector script."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SELECTOR = ROOT / "scripts" / "install_open_source_kit.py"
CATALOG = ROOT / "install" / "open-source-kit" / "resources.json"
TIMEOUT = 30

REQUIRED_FIELDS = (
    "id",
    "name",
    "category",
    "url",
    "description",
    "install_type",
    "automation_depth",
    "token_footprint",
    "tool_compatibility",
    "conflict_risk",
    "license",
    "reviewed_date",
    "pinned_ref",
    "risk_notes",
)
KNOWN_INSTALL_TYPES = {
    "claude-marketplace",
    "mcp-server",
    "reference-clone",
    "npx-cli",
    "discovery-index",
}


def run(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SELECTOR), *args],
        capture_output=True,
        text=True,
        timeout=TIMEOUT,
    )


class CatalogContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.data = json.loads(CATALOG.read_text(encoding="utf-8"))
        self.resources = self.data["resources"]

    def test_catalog_is_nonempty(self) -> None:
        self.assertGreater(len(self.resources), 0)

    def test_ids_are_unique(self) -> None:
        ids = [entry["id"] for entry in self.resources]
        self.assertEqual(len(ids), len(set(ids)))

    def test_every_resource_has_required_fields(self) -> None:
        for entry in self.resources:
            for field in REQUIRED_FIELDS:
                self.assertIn(field, entry, msg=f"{entry.get('id')} missing {field}")

    def test_install_type_is_known(self) -> None:
        for entry in self.resources:
            self.assertIn(
                entry["install_type"],
                KNOWN_INSTALL_TYPES,
                msg=f"{entry['id']} has unknown install_type",
            )

    def test_verified_command_present_only_when_stated(self) -> None:
        superpowers = next(e for e in self.resources if e["id"] == "obra-superpowers")
        self.assertIn("verified_command", superpowers)

    def test_discovery_index_entries_have_no_tool_compatibility(self) -> None:
        for entry in self.resources:
            if entry["category"] == "discovery-index":
                self.assertEqual(entry["tool_compatibility"], [])


class SelectorCliTests(unittest.TestCase):
    def test_list_prints_every_resource_id(self) -> None:
        result = run("--list")
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(CATALOG.read_text(encoding="utf-8"))
        for entry in data["resources"]:
            self.assertIn(entry["id"], result.stdout)

    def test_unknown_id_fails_without_writing(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            result = run("--select", "does-not-exist", "--target", target)
            self.assertEqual(result.returncode, 1)
            self.assertIn("unknown resource id", result.stderr)
            self.assertFalse((Path(target) / ".kit3-manifest.json").exists())

    def test_dry_run_does_not_write_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            result = run(
                "--select", "obra-superpowers", "--target", target, "--dry-run"
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("WOULD_WRITE", result.stdout)
            self.assertFalse((Path(target) / ".kit3-manifest.json").exists())

    def test_select_writes_manifest_with_pin(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            result = run(
                "--select",
                "obra-superpowers",
                "--target",
                target,
                "--pin",
                "obra-superpowers=deadbeef",
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            manifest = json.loads((Path(target) / ".kit3-manifest.json").read_text())
            entry = manifest["entries"]["obra-superpowers"]
            self.assertEqual(entry["pinned_ref"], "deadbeef")
            self.assertEqual(entry["url"], "https://github.com/obra/superpowers")

    def test_second_selection_merges_rather_than_overwrites(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            run("--select", "obra-superpowers", "--target", target)
            run("--select", "mcp-servers-official", "--target", target)
            manifest = json.loads((Path(target) / ".kit3-manifest.json").read_text())
            self.assertIn("obra-superpowers", manifest["entries"])
            self.assertIn("mcp-servers-official", manifest["entries"])

    def test_select_all_covers_full_catalog(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            result = run("--select", "all", "--target", target)
            self.assertEqual(result.returncode, 0, result.stderr)
            manifest = json.loads((Path(target) / ".kit3-manifest.json").read_text())
            data = json.loads(CATALOG.read_text(encoding="utf-8"))
            self.assertEqual(
                set(manifest["entries"]), {e["id"] for e in data["resources"]}
            )

    def test_malformed_pin_fails(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            result = run(
                "--select",
                "obra-superpowers",
                "--target",
                target,
                "--pin",
                "not-a-pair",
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("ID=REF", result.stderr)

    def test_target_required_unless_listing(self) -> None:
        result = run("--select", "obra-superpowers")
        self.assertEqual(result.returncode, 2)

    # --- regressions found by code review, 2026-08-31 ---

    def test_reselecting_preserves_an_existing_pin(self) -> None:
        """A later selection without --pin must not erase a recorded ref.

        The catalog's own pinned_ref is null for every entry, so falling back
        to it silently destroyed the audit trail during the documented
        workflow: pin after install, select something else later.
        """
        with tempfile.TemporaryDirectory() as target:
            run("--select", "obra-superpowers", "--target", target,
                "--pin", "obra-superpowers=deadbeef")
            run("--select", "obra-superpowers,mcp-servers-official", "--target", target)
            manifest = json.loads((Path(target) / ".kit3-manifest.json").read_text())
            self.assertEqual(
                manifest["entries"]["obra-superpowers"]["pinned_ref"], "deadbeef"
            )

    def test_select_all_preserves_existing_pins(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            run("--select", "obra-superpowers", "--target", target,
                "--pin", "obra-superpowers=cafe123")
            run("--select", "all", "--target", target)
            manifest = json.loads((Path(target) / ".kit3-manifest.json").read_text())
            self.assertEqual(
                manifest["entries"]["obra-superpowers"]["pinned_ref"], "cafe123"
            )

    def test_a_new_pin_still_overrides_an_old_one(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            run("--select", "obra-superpowers", "--target", target,
                "--pin", "obra-superpowers=old")
            run("--select", "obra-superpowers", "--target", target,
                "--pin", "obra-superpowers=new")
            manifest = json.loads((Path(target) / ".kit3-manifest.json").read_text())
            self.assertEqual(
                manifest["entries"]["obra-superpowers"]["pinned_ref"], "new"
            )

    def test_pin_for_an_unselected_id_is_an_error(self) -> None:
        """Silently dropping it leaves the operator believing a ref was recorded."""
        with tempfile.TemporaryDirectory() as target:
            result = run("--select", "obra-superpowers", "--target", target,
                         "--pin", "typo-id=abc123")
            self.assertEqual(result.returncode, 1)
            self.assertIn("not in this selection", result.stderr)
            self.assertFalse((Path(target) / ".kit3-manifest.json").exists())

    def test_manifest_records_what_review_found(self) -> None:
        """The manifest is the audit trail, so it carries the status too.

        A warning printed to a terminal survives nothing; this is reviewable in
        the diff when the client project commits the manifest.
        """
        with tempfile.TemporaryDirectory() as target:
            run("--select", "graphify,obra-superpowers", "--target", target)
            entries = json.loads(
                (Path(target) / ".kit3-manifest.json").read_text()
            )["entries"]
            self.assertEqual(
                entries["graphify"]["review"],
                {"status": "known_risks", "reviewed": True},
            )
            self.assertEqual(
                entries["obra-superpowers"]["review"],
                {"status": None, "reviewed": False},
            )

    def test_manifest_carries_the_install_guidance(self) -> None:
        """How something was installed is part of its risk.

        `curl | bash` is not `git clone`. The manifest is the audit trail a
        client project keeps, so it has to answer "how", not only "what" -
        otherwise the answer lives in terminal scrollback and is gone.
        """
        with tempfile.TemporaryDirectory() as target:
            run("--select", "caveman,grillme", "--target", target)
            entries = json.loads(
                (Path(target) / ".kit3-manifest.json").read_text()
            )["entries"]
            self.assertEqual(entries["caveman"]["install_method"], "npx-cli")
            self.assertIn("npx", entries["caveman"]["install_guidance"])
            self.assertEqual(entries["grillme"]["install_method"], "reference-clone")
            self.assertIn("git clone", entries["grillme"]["install_guidance"])

    def test_manifest_carries_the_risk_notes(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            run("--select", "caveman", "--target", target)
            entry = json.loads(
                (Path(target) / ".kit3-manifest.json").read_text()
            )["entries"]["caveman"]
            self.assertIn("HIGH RISK", entry["risk_notes"])

    def test_verified_command_wins_over_the_generic_template(self) -> None:
        """An entry that pins its own exact command must carry that one."""
        with tempfile.TemporaryDirectory() as target:
            run("--select", "obra-superpowers", "--target", target)
            entry = json.loads(
                (Path(target) / ".kit3-manifest.json").read_text()
            )["entries"]["obra-superpowers"]
            self.assertIn("/plugin install superpowers@", entry["install_guidance"])

    def test_missing_target_directory_fails_cleanly(self) -> None:
        """One line on stderr, not a traceback after lines that read as success."""
        result = run("--select", "obra-superpowers",
                     "--target", "/nonexistent-kit3-target-12345")
        self.assertEqual(result.returncode, 1)
        self.assertIn("not an existing directory", result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertNotIn("SELECTED", result.stdout)

    def test_target_that_is_a_file_fails_cleanly(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            path = Path(target) / "a-file"
            path.write_text("not a directory", encoding="utf-8")
            result = run("--select", "obra-superpowers", "--target", str(path))
            self.assertEqual(result.returncode, 1)
            self.assertIn("not an existing directory", result.stderr)

    def test_unreviewed_pick_is_flagged(self) -> None:
        """No registry entry means nothing checked it against the gates."""
        with tempfile.TemporaryDirectory() as target:
            result = run("--select", "obra-superpowers", "--target", target, "--dry-run")
            self.assertIn("NOT REVIEWED", result.stdout)

    def test_pick_with_findings_is_flagged(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            result = run("--select", "graphify", "--target", target, "--dry-run")
            self.assertIn("KNOWN RISKS", result.stdout)


if __name__ == "__main__":
    unittest.main()
