#!/usr/bin/env python3
"""Tests for the Kit 3 (open-source) catalog and its selector script."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
import unittest.mock
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SELECTOR = ROOT / "scripts" / "install_open_source_kit.py"
CATALOG = ROOT / "install" / "open-source-kit" / "resources.json"
TIMEOUT = 30

# Loaded in-process (not via subprocess) so --refresh tests can patch
# kit_fetcher.refresh directly. The subprocess-based `run()` below still
# exercises the real CLI end to end for every other test in this file.
_spec = importlib.util.spec_from_file_location("install_open_source_kit", SELECTOR)
iosk = importlib.util.module_from_spec(_spec)
sys.modules["install_open_source_kit"] = iosk
_spec.loader.exec_module(iosk)

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
    "python-cli",
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

    def test_verified_command_carries_a_verification_date(self) -> None:
        """A command checked against a README goes stale; the date says how stale."""
        for entry in self.resources:
            if entry.get("verified_command"):
                self.assertIn(
                    "command_verified_date", entry,
                    msg=f"{entry['id']} has verified_command with no verification date",
                )

    def test_discovery_index_entries_have_no_tool_compatibility(self) -> None:
        for entry in self.resources:
            if entry["category"] == "discovery-index":
                self.assertEqual(entry["tool_compatibility"], [])


class SelectorCliTests(unittest.TestCase):
    def test_malformed_catalog_fails_without_writing(self) -> None:
        resource = json.loads(CATALOG.read_text())["resources"][0]
        cases = (
            [],
            {"resources": [None]},
            {"resources": [dict(resource, id="../escape")]},
            {"resources": [dict(resource, install_type="unknown")]},
            {"resources": [dict(resource, verified_command=["not text"])]},
            {"resources": [dict(resource, tool_compatibility="claude-code")]},
            {"resources": [resource, resource]},
        )
        for data in cases:
            with self.subTest(data=data), tempfile.TemporaryDirectory() as target:
                catalog = Path(target) / "resources.json"
                catalog.write_text(json.dumps(data))
                result = run("--resources", str(catalog), "--list")
                self.assertEqual(result.returncode, 1)
                self.assertNotIn("Traceback", result.stderr)
                self.assertFalse((Path(target) / iosk.MANIFEST_NAME).exists())

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

    def test_empty_pin_cannot_erase_an_existing_pin(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            self.assertEqual(run("--select", "obra-superpowers", "--target", target,
                                 "--pin", "obra-superpowers=deadbeef").returncode, 0)
            path = Path(target) / iosk.MANIFEST_NAME
            original = path.read_bytes()
            result = run("--select", "obra-superpowers", "--target", target,
                         "--pin", "obra-superpowers= ")
            self.assertEqual(result.returncode, 1)
            self.assertEqual(path.read_bytes(), original)

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
            expected = next(item for item in iosk.load_resources(iosk.DEFAULT_RESOURCES) if item["id"] == "caveman")
            self.assertEqual(expected["risk_notes"], entry["risk_notes"])

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


    def test_invalid_manifest_structure_fails_without_changes(self) -> None:
        cases = (
            ([], "JSON object"),
            ({"schema_version": 1, "kit": "open-source-kit", "entries": []}, "entries"),
            (
                {
                    "schema_version": 1,
                    "kit": "open-source-kit",
                    "entries": {"obra-superpowers": []},
                },
                "obra-superpowers",
            ),
        )
        for data, expected in cases:
            with self.subTest(data=data), tempfile.TemporaryDirectory() as target:
                path = Path(target) / iosk.MANIFEST_NAME
                original = json.dumps(data)
                path.write_text(original, encoding="utf-8")

                result = run("--select", "obra-superpowers", "--target", target)

                self.assertEqual(result.returncode, 1)
                self.assertIn(expected, result.stderr)
                self.assertNotIn("Traceback", result.stderr)
                self.assertNotIn("SELECTED", result.stdout)
                self.assertEqual(path.read_text(encoding="utf-8"), original)

    def test_invalid_manifest_identity_fails_without_changes(self) -> None:
        cases = (
            ({"schema_version": 99, "kit": "open-source-kit", "entries": {}}, "schema_version"),
            ({"schema_version": True, "kit": "open-source-kit", "entries": {}}, "schema_version"),
            ({"schema_version": 1, "kit": "different-tool", "entries": {}}, "kit"),
        )
        for data, expected in cases:
            with self.subTest(data=data), tempfile.TemporaryDirectory() as target:
                path = Path(target) / iosk.MANIFEST_NAME
                original = json.dumps(data)
                path.write_text(original, encoding="utf-8")

                result = run("--select", "obra-superpowers", "--target", target)

                self.assertEqual(result.returncode, 1)
                self.assertIn(expected, result.stderr)
                self.assertNotIn("Traceback", result.stderr)
                self.assertNotIn("WROTE", result.stdout)
                self.assertEqual(path.read_text(encoding="utf-8"), original)

    def test_registry_status_is_recomputed_before_manifest_snapshot(self) -> None:
        resource = next(
            entry
            for entry in json.loads(CATALOG.read_text(encoding="utf-8"))["resources"]
            if entry["id"] == "graphify"
        )
        registry = json.loads(
            (iosk.REGISTRY_DIR / "graphify.json").read_text(encoding="utf-8")
        )
        registry["status"] = "clear"

        with tempfile.TemporaryDirectory() as registry_dir:
            Path(registry_dir, "graphify.json").write_text(
                json.dumps(registry), encoding="utf-8"
            )
            manifest = {"entries": {}}
            with unittest.mock.patch.object(iosk, "REGISTRY_DIR", Path(registry_dir)):
                with self.assertRaises(iosk.KitError):
                    iosk.apply_selection(manifest, [resource], {}, "2026-08-31")
            self.assertEqual(manifest, {"entries": {}})

    def test_registry_identity_is_bound_to_the_selected_resource(self) -> None:
        resource = next(
            entry
            for entry in json.loads(CATALOG.read_text(encoding="utf-8"))["resources"]
            if entry["id"] == "graphify"
        )
        original = json.loads(
            (iosk.REGISTRY_DIR / "graphify.json").read_text(encoding="utf-8")
        )
        for field in ("id", "catalog_id", "name", "url"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as registry_dir:
                registry = dict(original)
                registry[field] = "different-resource"
                Path(registry_dir, "graphify.json").write_text(
                    json.dumps(registry), encoding="utf-8"
                )
                with unittest.mock.patch.object(iosk, "REGISTRY_DIR", Path(registry_dir)):
                    with self.assertRaises(iosk.KitError):
                        iosk.apply_selection(
                            {"entries": {}}, [resource], {}, "2026-08-31"
                        )

    def test_registry_install_path_is_bound_to_the_selected_resource(self) -> None:
        resource = next(
            entry
            for entry in json.loads(CATALOG.read_text(encoding="utf-8"))["resources"]
            if entry["id"] == "graphify"
        )
        for field, value in (
            ("install_type", "npx-cli"),
            ("verified_command", "run a different installer"),
        ):
            with self.subTest(field=field):
                changed = dict(resource)
                changed[field] = value
                with self.assertRaises(iosk.KitError):
                    iosk.apply_selection(
                        {"entries": {}}, [changed], {}, "2026-08-31"
                    )

    def test_registry_path_cannot_escape_through_a_resource_id(self) -> None:
        resource = next(
            entry
            for entry in json.loads(CATALOG.read_text(encoding="utf-8"))["resources"]
            if entry["id"] == "graphify"
        )
        registry = json.loads(
            (iosk.REGISTRY_DIR / "graphify.json").read_text(encoding="utf-8")
        )
        with tempfile.TemporaryDirectory() as workspace:
            root = Path(workspace)
            registry_dir = root / "registry"
            registry_dir.mkdir()
            forged_id = "../forged"
            forged_resource = dict(resource, id=forged_id)
            registry["id"] = forged_id
            registry["catalog_id"] = forged_id
            (root / "forged.json").write_text(json.dumps(registry), encoding="utf-8")

            with unittest.mock.patch.object(iosk, "REGISTRY_DIR", registry_dir):
                with self.assertRaises(iosk.KitError):
                    iosk.registry_status(forged_resource)

    def test_malformed_registry_entry_fails_as_a_kit_error(self) -> None:
        resource = next(
            entry
            for entry in json.loads(CATALOG.read_text(encoding="utf-8"))["resources"]
            if entry["id"] == "graphify"
        )
        for registry in ([], None, {"status": "clear"}):
            with self.subTest(registry=registry), tempfile.TemporaryDirectory() as registry_dir:
                Path(registry_dir, "graphify.json").write_text(
                    json.dumps(registry), encoding="utf-8"
                )
                with unittest.mock.patch.object(iosk, "REGISTRY_DIR", Path(registry_dir)):
                    try:
                        iosk.apply_selection(
                            {"entries": {}}, [resource], {}, "2026-08-31"
                        )
                    except Exception as error:  # assertion below checks the boundary type
                        self.assertIsInstance(error, iosk.KitError)
                    else:
                        self.fail("malformed registry entry was accepted")

    def test_manifest_symlink_is_rejected_without_touching_its_target(self) -> None:
        with tempfile.TemporaryDirectory() as workspace:
            root = Path(workspace)
            target = root / "target"
            target.mkdir()
            victim = root / "victim.json"
            original = '{"outside": true}\n'
            victim.write_text(original, encoding="utf-8")
            manifest_path = target / iosk.MANIFEST_NAME
            manifest_path.symlink_to(victim)

            result = run("--select", "obra-superpowers", "--target", str(target))

            self.assertEqual(result.returncode, 1)
            self.assertIn("symlink", result.stderr)
            self.assertTrue(manifest_path.is_symlink())
            self.assertEqual(victim.read_text(encoding="utf-8"), original)

    def test_write_boundary_rejects_a_symlink_created_after_load(self) -> None:
        with tempfile.TemporaryDirectory() as workspace:
            root = Path(workspace)
            manifest_path = root / iosk.MANIFEST_NAME
            manifest = iosk.load_manifest(manifest_path)
            victim = root / "victim.json"
            original = '{"outside": true}\n'
            victim.write_text(original, encoding="utf-8")
            manifest_path.symlink_to(victim)
            writer = getattr(iosk, "write_manifest", None)

            self.assertIsNotNone(writer, "selector has no safe write boundary")
            with self.assertRaises(iosk.KitError):
                writer(manifest_path, manifest)
            self.assertEqual(victim.read_text(encoding="utf-8"), original)

    def test_target_directory_swap_cannot_redirect_the_manifest_write(self) -> None:
        with tempfile.TemporaryDirectory() as workspace:
            root = Path(workspace)
            target = root / "target"
            moved_target = root / "moved-target"
            outside = root / "outside"
            target.mkdir()
            outside.mkdir()
            apply_selection = iosk.apply_selection

            def swap_target(*args, **kwargs):
                target.rename(moved_target)
                target.symlink_to(outside, target_is_directory=True)
                return apply_selection(*args, **kwargs)

            argv = [
                str(iosk.__file__),
                "--select",
                "obra-superpowers",
                "--target",
                str(target),
            ]
            with (
                unittest.mock.patch.object(sys, "argv", argv),
                unittest.mock.patch.object(iosk, "apply_selection", side_effect=swap_target),
                unittest.mock.patch("sys.stdout", new=io.StringIO()),
                unittest.mock.patch("sys.stderr", new=io.StringIO()),
            ):
                result = iosk.main()

            self.assertEqual(result, 1)
            self.assertFalse((outside / iosk.MANIFEST_NAME).exists())

    def test_ancestor_swap_before_open_cannot_redirect_the_manifest_write(self) -> None:
        with tempfile.TemporaryDirectory() as workspace:
            root = Path(workspace)
            ancestor = root / "ancestor"
            target = ancestor / "target"
            moved_ancestor = root / "moved-ancestor"
            outside = root / "outside"
            outside_target = outside / "target"
            target.mkdir(parents=True)
            outside_target.mkdir(parents=True)
            open_target_directory = iosk.open_target_directory

            def swap_ancestor(path):
                ancestor.rename(moved_ancestor)
                ancestor.symlink_to(outside, target_is_directory=True)
                return open_target_directory(path)

            argv = [
                str(iosk.__file__),
                "--select",
                "obra-superpowers",
                "--target",
                str(target),
            ]
            with (
                unittest.mock.patch.object(sys, "argv", argv),
                unittest.mock.patch.object(
                    iosk,
                    "open_target_directory",
                    side_effect=swap_ancestor,
                ),
                unittest.mock.patch("sys.stdout", new=io.StringIO()),
                unittest.mock.patch("sys.stderr", new=io.StringIO()),
            ):
                result = iosk.main()

            self.assertEqual(result, 1)
            self.assertFalse((outside_target / iosk.MANIFEST_NAME).exists())

    def test_existing_ancestor_symlink_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as workspace:
            root = Path(workspace)
            real_target = root / "real" / "target"
            alias = root / "alias"
            real_target.mkdir(parents=True)
            alias.symlink_to(root / "real", target_is_directory=True)

            result = run(
                "--select",
                "obra-superpowers",
                "--target",
                str(alias / "target"),
            )

            self.assertEqual(result.returncode, 1)
            self.assertFalse((real_target / iosk.MANIFEST_NAME).exists())

    def test_hard_linked_manifest_does_not_overwrite_the_other_link(self) -> None:
        with tempfile.TemporaryDirectory() as workspace:
            root = Path(workspace)
            target = root / "target"
            target.mkdir()
            victim = root / "victim.json"
            original = json.dumps(
                {"schema_version": 1, "kit": "open-source-kit", "entries": {}},
                sort_keys=True,
            ) + "\n"
            victim.write_text(original, encoding="utf-8")
            manifest_path = target / iosk.MANIFEST_NAME
            os.link(victim, manifest_path)

            result = run("--select", "obra-superpowers", "--target", str(target))

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(victim.read_text(encoding="utf-8"), original)
            self.assertIn(
                "obra-superpowers",
                json.loads(manifest_path.read_text(encoding="utf-8"))["entries"],
            )


def _run_quietly(argv: list[str]) -> int:
    """Call iosk.main() without letting its prints reach the test runner's stdout."""
    with contextlib.redirect_stdout(io.StringIO()):
        return iosk.main(argv)


class RefreshTests(unittest.TestCase):
    """--refresh, tested in-process against the mocked kit_fetcher.refresh.

    `run()` above spawns a subprocess, so a mock in this test process would
    never reach it - these tests call `iosk.main()` directly instead, which is
    why this file loads install_open_source_kit.py via importlib rather than
    only shelling out to it.
    """

    def test_refresh_keeps_one_candidate_advisory(self) -> None:
        fake_meta = iosk.kit_fetcher.RepoMetadata(1, 1, "2026-09-01T00:00:00Z", 0, "MIT")
        candidate = iosk.kit_fetcher.Candidate(
            context="## Install", command="npx real-thing install"
        )
        with tempfile.TemporaryDirectory() as target:
            with unittest.mock.patch.object(
                iosk.kit_fetcher, "refresh", return_value=(fake_meta, [candidate], [])
            ):
                result = _run_quietly(
                    ["--select", "obra-superpowers", "--target", target, "--refresh"]
                )
            self.assertEqual(result, 0)
            entry = json.loads(
                (Path(target) / ".kit3-manifest.json").read_text()
            )["entries"]["obra-superpowers"]
            self.assertIn("/plugin install superpowers@", entry["install_guidance"])
            self.assertEqual(entry["install_guidance_source"], "verified_command")
            self.assertIsNone(entry["install_guidance_fetched_at"])
            self.assertEqual(entry["readme_refresh"]["candidates"], [
                {"context": candidate.context, "command": candidate.command}
            ])
            self.assertTrue(entry["readme_refresh"]["advisory"])

    def test_readme_candidate_does_not_inherit_existing_review(self) -> None:
        resource = next(entry for entry in iosk.load_resources(CATALOG) if entry["id"] == "graphify")
        candidate = iosk.kit_fetcher.Candidate("install", "curl https://example.invalid/new | sh")
        result = iosk.apply_selection(
            {"entries": {}}, [resource], {}, "2026-09-05",
            {"graphify": (None, [candidate], [])},
        )["entries"]["graphify"]
        self.assertEqual(result["install_guidance"], resource["verified_command"])
        self.assertEqual(result["review"], {"status": "known_risks", "reviewed": True})
        self.assertNotIn("review", result["readme_refresh"])
        self.assertTrue(result["readme_refresh"]["advisory"])

    def test_refresh_does_not_replace_a_generic_template(self) -> None:
        resource = next(entry for entry in iosk.load_resources(CATALOG)
                        if entry["id"] == "mcp-servers-official")
        resource.pop("verified_command", None)
        candidate = iosk.kit_fetcher.Candidate("install", "run an unknown installer")
        guidance, source, notes = iosk.resolve_guidance(resource, (None, [candidate], []))
        self.assertEqual(guidance, iosk.guidance_for(resource))
        self.assertEqual(source, "generic_template")
        self.assertIn("advisory", " ".join(notes))

    def test_refresh_falls_back_when_candidates_are_ambiguous(self) -> None:
        """Picking among several would present a guess as a fact - the one thing
        this whole registry exists to refuse to do."""
        c1 = iosk.kit_fetcher.Candidate(context="## npm", command="npm install -g x")
        c2 = iosk.kit_fetcher.Candidate(context="## pipx", command="pipx install x")
        with tempfile.TemporaryDirectory() as target:
            with unittest.mock.patch.object(
                iosk.kit_fetcher, "refresh", return_value=(None, [c1, c2], [])
            ):
                _run_quietly(["--select", "obra-superpowers", "--target", target, "--refresh"])
            entry = json.loads(
                (Path(target) / ".kit3-manifest.json").read_text()
            )["entries"]["obra-superpowers"]
            self.assertNotEqual(entry["install_guidance_source"], "live_fetch")
            self.assertIn("/plugin install superpowers@", entry["install_guidance"])

    def test_refresh_falls_back_on_network_failure(self) -> None:
        with tempfile.TemporaryDirectory() as target:
            with unittest.mock.patch.object(
                iosk.kit_fetcher, "refresh",
                return_value=(None, [], ["boom: no network"]),
            ):
                result = _run_quietly(
                    ["--select", "obra-superpowers", "--target", target, "--refresh"]
                )
            self.assertEqual(result, 0)  # a refresh failure is not a run failure
            entry = json.loads(
                (Path(target) / ".kit3-manifest.json").read_text()
            )["entries"]["obra-superpowers"]
            self.assertEqual(entry["install_guidance_source"], "verified_command")

    def test_discovery_index_entries_are_not_refreshed(self) -> None:
        """"Install" is not a concept for a curated list - refreshing one is wasted work."""
        with tempfile.TemporaryDirectory() as target:
            with unittest.mock.patch.object(iosk.kit_fetcher, "refresh") as mocked:
                _run_quietly(
                    ["--select", "awesome-claude-code", "--target", target, "--refresh"]
                )
            mocked.assert_not_called()

    def test_without_refresh_flag_kit_fetcher_is_never_called(self) -> None:
        """The default path stays exactly as network-free as it always was."""
        with tempfile.TemporaryDirectory() as target:
            with unittest.mock.patch.object(iosk.kit_fetcher, "refresh") as mocked:
                _run_quietly(["--select", "obra-superpowers", "--target", target])
            mocked.assert_not_called()


if __name__ == "__main__":
    unittest.main()
