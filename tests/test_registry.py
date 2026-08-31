#!/usr/bin/env python3
"""Tests for the Kit 3 admission registry and its validator."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import unittest
from pathlib import Path

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
        "id": "fixture",
        "catalog_id": "obra-superpowers",
        "name": "fixture/tool",
        "url": "https://example.invalid/fixture",
        "reviewed_date": "2026-08-31",
        "reviewed_by": "tests",
        "verdict": "approved",
        "verdict_summary": "fixture",
        "tier": 2,
        "default_state": "disabled",
        "install": {
            "method": "reference-clone",
            "pinned_ref": "deadbeef",
            "namespace_prefix": "fx-",
            "command": "git clone ...",
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


class VerdictComputationTests(unittest.TestCase):
    def test_all_pass_is_approved(self) -> None:
        gates = {name: {"status": "pass"} for name in registry.BINARY_GATES}
        self.assertEqual(registry.compute_verdict(gates), "approved")

    def test_any_unknown_is_blocked(self) -> None:
        gates = {name: {"status": "pass"} for name in registry.BINARY_GATES}
        gates["uninstall"]["status"] = "unknown"
        self.assertEqual(registry.compute_verdict(gates), "blocked")

    def test_any_fail_is_rejected(self) -> None:
        gates = {name: {"status": "pass"} for name in registry.BINARY_GATES}
        gates["collisions"]["status"] = "fail"
        self.assertEqual(registry.compute_verdict(gates), "rejected")

    def test_fail_outranks_unknown(self) -> None:
        gates = {name: {"status": "pass"} for name in registry.BINARY_GATES}
        gates["uninstall"]["status"] = "unknown"
        gates["license"]["status"] = "fail"
        self.assertEqual(registry.compute_verdict(gates), "rejected")


class EntryValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.catalog_ids = registry.load_catalog_ids()

    def check(self, entry: dict) -> list[str]:
        return registry.validate_entry(entry, self.catalog_ids)

    def test_valid_fixture_passes(self) -> None:
        self.assertEqual(self.check(valid_entry()), [])

    def test_stored_verdict_cannot_outrank_a_failing_gate(self) -> None:
        entry = valid_entry()
        entry["binary_gates"]["license"] = {
            "status": "fail",
            "evidence": "non-commercial",
            "resolves_by": "fork under a compatible license",
        }
        errors = self.check(entry)
        self.assertTrue(any("disagrees with the gates" in e for e in errors), errors)

    def test_blocked_verdict_is_accepted_when_gates_agree(self) -> None:
        entry = valid_entry()
        entry["verdict"] = "blocked"
        entry["binary_gates"]["data_egress"] = {
            "status": "unknown",
            "evidence": "source not read",
            "resolves_by": "read the install script",
        }
        self.assertEqual(self.check(entry), [])

    def test_non_passing_gate_requires_resolves_by(self) -> None:
        entry = valid_entry()
        entry["verdict"] = "blocked"
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

    def test_unknown_gate_status_is_rejected(self) -> None:
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
        entry["verdict"] = "blocked"
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

    def test_non_string_evidence_is_rejected(self) -> None:
        entry = valid_entry()
        entry["binary_gates"]["license"]["evidence"] = 42
        errors = self.check(entry)
        self.assertTrue(any("no evidence" in e for e in errors), errors)

    def test_verdict_is_case_sensitive(self) -> None:
        entry = valid_entry()
        entry["verdict"] = "APPROVED"
        errors = self.check(entry)
        self.assertTrue(any("verdict must be one of" in e for e in errors), errors)

    def test_understated_verdict_is_caught_too(self) -> None:
        """The check runs both ways - a verdict may not be harsher than the gates either."""
        entry = valid_entry()
        entry["verdict"] = "rejected"
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
        catalog_ids = registry.load_catalog_ids()
        for entry in self.entries:
            self.assertEqual(
                registry.validate_entry(entry, catalog_ids),
                [],
                msg=f"{entry['id']} failed validation",
            )

    def test_every_entry_is_in_the_catalog(self) -> None:
        catalog = json.loads(CATALOG.read_text(encoding="utf-8"))
        catalog_ids = {entry["id"] for entry in catalog["resources"]}
        for entry in self.entries:
            self.assertIn(entry["catalog_id"], catalog_ids)

    def test_unapproved_entries_default_to_disabled(self) -> None:
        for entry in self.entries:
            if entry["verdict"] != "approved":
                self.assertEqual(
                    entry["default_state"],
                    "disabled",
                    msg=f"{entry['id']} is {entry['verdict']} but defaults to enabled",
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
