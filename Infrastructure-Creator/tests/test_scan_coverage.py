#!/usr/bin/env python3
"""Regression tests for the deterministic discovery-coverage gate.

A scan that missed a module and a scan that covered it produce the same
artifact - a list of what was found - so the omission is invisible until a
generated skill turns out not to know a subsystem exists. These cases pin the
four dispositions and what each of them costs to claim.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VALIDATOR_PATH = (
    ROOT / ".agents/skills/bootstrap-verifier/scripts/validate_scan_coverage.py"
)
SPEC = importlib.util.spec_from_file_location("validate_scan_coverage", VALIDATOR_PATH)
validator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validator
SPEC.loader.exec_module(validator)


class ScanCoverageFixture(unittest.TestCase):
    """A small target with the shapes a real one has: modules, tests, secrets."""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.target = self.base / "target"
        self.task = self.base / "tasks/TASK-001"
        self.task.mkdir(parents=True)
        for relative, text in (
            ("composer.json", '{"require":{"php":"^8.3"}}\n'),
            ("src/Billing/BillingService.php", "<?php // billing\n"),
            ("src/Orders/OrderService.php", "<?php // orders\n"),
            ("tests/Feature/BillingTest.php", "<?php // feature\n"),
            ("config/services.yaml", "services:\n  _defaults:\n    autowire: true\n"),
            (".env", "APP_SECRET=redacted\n"),
            ("vendor/autoload.php", "<?php // vendor\n"),
        ):
            path = self.target / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        self.coverage = self.honest_coverage()
        self.ledger = self.honest_ledger()
        self.claims = self.honest_claims()
        self.write()

    def honest_coverage(self) -> dict:
        return {
            "scanner": "stack-scanner",
            "target_root": str(self.target),
            "surfaces": [
                {
                    "surface": "composer.json",
                    "kind": "file",
                    "disposition": "covered",
                    "reason": "Declares the runtime and dependency graph",
                    "evidence_ids": ["EV-STK-0001"],
                },
                {
                    "surface": "src/**",
                    "kind": "tree",
                    "disposition": "covered",
                    "reason": "The target's own application modules",
                    "evidence_ids": ["EV-STK-0002"],
                },
                {
                    "surface": "tests/**",
                    "kind": "tree",
                    "disposition": "covered",
                    "reason": "The target's own test families",
                    "evidence_ids": ["EV-STK-0003"],
                },
                {
                    "surface": "config/**",
                    "kind": "tree",
                    "disposition": "covered",
                    "reason": "Committed configuration for the framework",
                    "evidence_ids": ["EV-STK-0004"],
                },
                {
                    "surface": ".env",
                    "kind": "file",
                    "disposition": "not-permitted",
                    "reason": "Holds secret values; existence recorded only",
                    "evidence_ids": [],
                },
                {
                    "surface": "vendor/**",
                    "kind": "tree",
                    "disposition": "excluded",
                    "reason": "Third-party code, not the target's own",
                    "evidence_ids": [],
                },
            ],
        }

    def honest_ledger(self) -> dict:
        return {
            "scanner": "stack-scanner",
            "target_root": str(self.target),
            "evidence": [
                {
                    "id": "EV-STK-0001",
                    "path": "composer.json",
                    "source_type": "configuration",
                    "authority": "Declares the runtime constraint",
                    "confidence": "confirmed",
                    "supported_claims": ["composer.json requires php ^8.3"],
                },
                {
                    "id": "EV-STK-0002",
                    "path": "src/Billing/BillingService.php",
                    "source_type": "application code",
                    "authority": "Implements the billing module",
                    "confidence": "confirmed",
                    "supported_claims": ["BillingService is the billing entry point"],
                },
                {
                    "id": "EV-STK-0003",
                    "path": "tests/Feature/BillingTest.php",
                    "source_type": "test",
                    "authority": "Feature suite for billing",
                    "confidence": "confirmed",
                    "supported_claims": ["A feature suite covers billing"],
                },
                {
                    "id": "EV-STK-0004",
                    "path": "config/services.yaml",
                    "source_type": "configuration",
                    "authority": "Declares autowiring defaults",
                    "confidence": "confirmed",
                    "supported_claims": ["services.yaml enables autowire"],
                },
            ],
        }

    def honest_claims(self) -> dict:
        return {
            "target_root": str(self.target),
            "claims": [
                {
                    "id": "CLM-0001",
                    "statement": "Billing runs through BillingService only",
                    "claim_class": "invariant",
                    "priority": "high",
                    "evidence_ids": ["EV-STK-0002"],
                    "scanners": ["stack-scanner"],
                    "status": "confirmed",
                },
                {
                    "id": "CLM-0002",
                    "statement": "The feature suite covers billing",
                    "claim_class": "capability",
                    "priority": "medium",
                    "evidence_ids": ["EV-STK-0003"],
                    "scanners": ["stack-scanner"],
                    "status": "confirmed",
                },
            ],
            "contradictions": [],
        }

    def plan_with(self, invariants: list[dict]) -> Path:
        """A plan whose evidence ids are renumbered, as a real merge does.

        `profile-synthesizer` merges seven ledgers and may renumber; the join
        between discovery and the plan therefore has to be the cited source,
        not the id. `EV-0001` here is the plan's name for `EV-STK-0002`.
        """
        path = self.task / "skill-generation-plan.json"
        path.write_text(
            json.dumps(
                {
                    "evidence": [
                        {"id": "EV-0001", "path": "src/Billing/BillingService.php"},
                        {"id": "EV-0002", "path": "config/services.yaml"},
                    ],
                    "critical_invariants": invariants,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        return path

    def write(self) -> None:
        (self.task / "project-claims.json").write_text(
            json.dumps(self.claims, indent=2) + "\n", encoding="utf-8"
        )
        (self.task / "stack-scanner-coverage.json").write_text(
            json.dumps(self.coverage, indent=2) + "\n", encoding="utf-8"
        )
        (self.task / "stack-scanner-evidence.json").write_text(
            json.dumps(self.ledger, indent=2) + "\n", encoding="utf-8"
        )

    def codes(self, severity: str = "error") -> list[str]:
        self.write()
        return [
            item.code
            for item in validator.validate(self.target, self.task)
            if item.severity == severity
        ]

    def surface(self, name: str) -> dict:
        return next(
            item for item in self.coverage["surfaces"] if item["surface"] == name
        )


class ScanCoverageTest(ScanCoverageFixture):
    def test_an_honest_coverage_artifact_passes(self) -> None:
        self.assertEqual(self.codes(), [])

    def test_a_module_nobody_dispositioned_is_reported(self) -> None:
        (self.target / "src/Legacy").mkdir()
        (self.target / "src/Legacy/LegacyImport.php").write_text(
            "<?php // legacy\n", encoding="utf-8"
        )
        # `src/**` still accounts for it - the omission has to be a surface no
        # declared entry reaches at all.
        self.assertEqual(self.codes(), [])
        (self.target / "integrations").mkdir()
        (self.target / "integrations/provider.php").write_text(
            "<?php // provider\n", encoding="utf-8"
        )
        self.assertIn("SCAN_SURFACE_UNACCOUNTED", self.codes())

    def test_claiming_coverage_without_evidence_inside_it_is_reported(self) -> None:
        self.surface("config/**")["evidence_ids"] = []
        self.assertIn("SCAN_COVERED_WITHOUT_EVIDENCE", self.codes())

    def test_evidence_from_outside_every_covered_surface_is_reported(self) -> None:
        self.ledger["evidence"].append(
            {
                "id": "EV-STK-0009",
                "path": "vendor/autoload.php",
                "source_type": "application code",
                "authority": "Composer autoloader",
                "confidence": "confirmed",
                "supported_claims": ["autoload.php registers the class map"],
            }
        )
        self.assertIn("SCAN_EVIDENCE_OUTSIDE_COVERAGE", self.codes())

    def test_claiming_to_have_covered_a_secret_is_reported(self) -> None:
        entry = self.surface(".env")
        entry["disposition"] = "covered"
        entry["evidence_ids"] = ["EV-STK-0001"]
        self.assertIn("SCAN_SECRET_COVERED", self.codes())

    def test_the_secrets_rule_is_enforced_without_reading_the_secret(self) -> None:
        """The gate never opens a file whose disposition it is checking."""
        opened: list[str] = []
        real_read = Path.read_text

        def watched(self_path, *args, **kwargs):  # noqa: ANN001
            opened.append(self_path.as_posix())
            return real_read(self_path, *args, **kwargs)

        entry = self.surface(".env")
        entry["disposition"] = "covered"
        entry["evidence_ids"] = ["EV-STK-0001"]
        self.write()
        Path.read_text = watched
        try:
            validator.validate(self.target, self.task)
        finally:
            Path.read_text = real_read
        self.assertIn("SCAN_SECRET_COVERED", self.codes())
        self.assertEqual([path for path in opened if path.endswith(".env")], [])

    def test_citing_evidence_from_a_surface_declared_excluded_is_reported(self) -> None:
        self.surface("vendor/**")["evidence_ids"] = ["EV-STK-0001"]
        self.assertIn("SCAN_DISPOSITION_EVIDENCE_CONFLICT", self.codes())

    def test_a_truncated_surface_is_visible_rather_than_silent(self) -> None:
        entry = self.surface("config/**")
        entry["disposition"] = "truncated"
        entry["evidence_ids"] = []
        entry["reason"] = "Stopped at the scan budget after 200 files"
        self.assertIn("SCAN_SURFACE_TRUNCATED", self.codes("warning"))
        self.assertEqual(self.codes(), [])

    def test_two_scanners_disagreeing_about_a_secret_is_reported(self) -> None:
        second = json.loads(json.dumps(self.coverage))
        second["scanner"] = "security-compliance-scanner"
        second["surfaces"] = [
            {
                "surface": ".env",
                "kind": "file",
                "disposition": "covered",
                "reason": "Read the runtime configuration",
                "evidence_ids": ["EV-SEC-0001"],
            }
        ]
        (self.task / "security-compliance-scanner-coverage.json").write_text(
            json.dumps(second, indent=2) + "\n", encoding="utf-8"
        )
        ledger = {
            "scanner": "security-compliance-scanner",
            "target_root": str(self.target),
            "evidence": [
                {
                    "id": "EV-SEC-0001",
                    "path": ".env",
                    "source_type": "configuration",
                    "authority": "Runtime configuration",
                    "confidence": "confirmed",
                    "supported_claims": ["the runtime declares an app secret"],
                }
            ],
        }
        (self.task / "security-compliance-scanner-evidence.json").write_text(
            json.dumps(ledger, indent=2) + "\n", encoding="utf-8"
        )
        codes = self.codes()
        self.assertIn("SCAN_DISPOSITION_CONFLICT", codes)
        self.assertIn("SCAN_SECRET_COVERED", codes)

    def test_coverage_without_its_ledger_is_reported(self) -> None:
        (self.task / "stack-scanner-evidence.json").unlink()
        codes = [
            item.code
            for item in validator.validate(self.target, self.task)
            if item.severity == "error"
        ]
        self.assertIn("SCAN_LEDGER_MISSING", codes)

    def test_a_run_with_no_coverage_artifact_fails_closed(self) -> None:
        (self.task / "stack-scanner-coverage.json").unlink()
        codes = [
            item.code
            for item in validator.validate(self.target, self.task)
            if item.severity == "error"
        ]
        self.assertEqual(codes, ["SCAN_COVERAGE_MISSING"])

    def test_a_surface_escaping_the_target_is_refused(self) -> None:
        self.coverage["surfaces"].append(
            {
                "surface": "../other-project/src",
                "kind": "tree",
                "disposition": "covered",
                "reason": "Neighbouring checkout",
                "evidence_ids": [],
            }
        )
        self.assertIn("SCAN_SURFACE_UNSAFE", self.codes())

    def test_an_untyped_surface_entry_is_refused(self) -> None:
        self.coverage["surfaces"].append(
            {"surface": "public", "disposition": "covered"}
        )
        self.assertIn("SCAN_COVERAGE_INVALID", self.codes())

    def test_the_json_report_is_byte_stable_across_runs(self) -> None:
        import io
        import contextlib

        outputs = []
        for _ in range(2):
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                validator.main(
                    [
                        "--target",
                        str(self.target),
                        "--task-dir",
                        str(self.task),
                        "--json",
                    ]
                )
            outputs.append(buffer.getvalue())
        self.assertEqual(outputs[0], outputs[1])



class ClaimReconciliationTest(ScanCoverageFixture):
    """Seven scanners state their findings in prose, in seven ledgers.

    Nothing merged them, nothing noticed when two contradicted each other, and
    nothing noticed when an invariant one of them confirmed never reached the
    plan. These cases pin the promotion that fixes that.
    """

    def test_an_honest_claim_set_passes(self) -> None:
        self.assertEqual(self.codes(), [])

    def test_a_run_without_a_claim_set_fails_closed(self) -> None:
        (self.task / "project-claims.json").unlink()
        codes = [
            item.code
            for item in validator.validate(self.target, self.task)
            if item.severity == "error"
        ]
        self.assertIn("CLAIMS_MISSING", codes)

    def test_a_claim_resting_on_evidence_no_ledger_carries_is_rejected(self) -> None:
        self.claims["claims"][0]["evidence_ids"] = ["EV-STK-9999"]
        self.assertIn("CLAIM_EVIDENCE_UNKNOWN", self.codes())

    def test_a_claim_attributed_to_a_silent_scanner_is_rejected(self) -> None:
        self.claims["claims"][0]["scanners"] = ["integration-scanner"]
        self.assertIn("CLAIM_SCANNER_UNKNOWN", self.codes())

    def test_a_repeated_claim_id_is_rejected(self) -> None:
        self.claims["claims"].append(dict(self.claims["claims"][0]))
        self.assertIn("CLAIM_ID_DUPLICATE", self.codes())

    def test_an_unresolved_contradiction_below_an_invariant_is_a_warning(self) -> None:
        """Visible, not blocking: nothing a skill must honour is in doubt."""
        self.claims["claims"][0]["claim_class"] = "convention"
        self.claims["claims"][0]["priority"] = "medium"
        self.claims["contradictions"] = [
            {
                "id": "CTR-0001",
                "claim_ids": ["CLM-0001", "CLM-0002"],
                "statement": "The suite exercises a path the convention forbids",
                "resolution": "unresolved",
            }
        ]
        self.assertIn("CONTRADICTION_UNRESOLVED", self.codes("warning"))
        self.assertEqual(self.codes(), [])

    def test_a_contradiction_touching_a_high_priority_invariant_is_blocking(
        self,
    ) -> None:
        self.claims["contradictions"] = [
            {
                "id": "CTR-0002",
                "claim_ids": ["CLM-0001", "CLM-0002"],
                "statement": "The suite exercises the path the invariant forbids",
                "resolution": "unresolved",
            }
        ]
        self.assertIn("CONTRADICTION_UNRESOLVED_INVARIANT", self.codes())

    def test_a_contradiction_naming_a_claim_that_does_not_exist_is_rejected(
        self,
    ) -> None:
        self.claims["contradictions"] = [
            {
                "id": "CTR-0003",
                "claim_ids": ["CLM-0001", "CLM-4242"],
                "statement": "Names a claim nobody made",
                "resolution": "unresolved",
            }
        ]
        self.assertIn("CONTRADICTION_CLAIM_UNKNOWN", self.codes())

    def test_an_invariant_the_plan_carries_is_accepted(self) -> None:
        self.write()
        plan = self.plan_with(
            [
                {
                    "id": "billing.entry",
                    "statement": "Billing always runs through BillingService",
                    "evidence_ids": ["EV-0001"],
                }
            ]
        )
        codes = [
            item.code
            for item in validator.validate(self.target, self.task, plan)
            if item.severity == "error"
        ]
        self.assertEqual(codes, [])

    def test_an_invariant_the_plan_dropped_is_reported(self) -> None:
        self.write()
        plan = self.plan_with(
            [
                {
                    "id": "unrelated",
                    "statement": "Exports never leave a partial file in place",
                    "evidence_ids": ["EV-0002"],
                }
            ]
        )
        codes = [
            item.code
            for item in validator.validate(self.target, self.task, plan)
            if item.severity == "error"
        ]
        self.assertIn("CLAIM_INVARIANT_LOST", codes)

    def test_an_invariant_restated_about_a_different_source_is_not_carried(
        self,
    ) -> None:
        """Sharing an evidence id is not enough; the statement must match too."""
        self.write()
        plan = self.plan_with(
            [
                {
                    "id": "billing.unrelated",
                    "statement": "The deployment pipeline requires manual approval",
                    "evidence_ids": ["EV-0001"],
                }
            ]
        )
        codes = [
            item.code
            for item in validator.validate(self.target, self.task, plan)
            if item.severity == "error"
        ]
        self.assertIn("CLAIM_INVARIANT_LOST", codes)

    def test_only_high_priority_invariants_have_to_survive(self) -> None:
        """A medium capability claim is not something the plan owes an answer."""
        self.write()
        plan = self.plan_with(
            [
                {
                    "id": "billing.entry",
                    "statement": "Billing always runs through BillingService",
                    "evidence_ids": ["EV-0001"],
                }
            ]
        )
        codes = [
            item.code
            for item in validator.validate(self.target, self.task, plan)
            if item.severity == "error"
        ]
        self.assertNotIn("CLAIM_INVARIANT_LOST", codes)


if __name__ == "__main__":
    unittest.main()
