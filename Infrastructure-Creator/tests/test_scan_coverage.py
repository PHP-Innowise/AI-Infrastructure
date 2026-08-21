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

    def test_a_forbidden_file_inside_a_covered_tree_is_not_a_contradiction(
        self,
    ) -> None:
        """"All of config is covered, except the keys nobody may read."

        Found by running the gate against a real target: reading this as a
        contradiction would force every scanner to enumerate a tree file by
        file, which is what the glob surfaces exist to avoid.
        """
        (self.target / "config/jwt").mkdir(parents=True)
        (self.target / "config/jwt/private.pem").write_text("x\n", encoding="utf-8")
        self.coverage["surfaces"].insert(
            0,
            {
                "surface": "config",
                "kind": "tree",
                "disposition": "covered",
                "reason": "Framework configuration for the application",
                "evidence_ids": ["EV-STK-0004"],
            },
        )
        self.coverage["surfaces"].append(
            {
                "surface": "config/jwt",
                "kind": "tree",
                "disposition": "not-permitted",
                "reason": "Private and public signing keys",
                "evidence_ids": [],
            }
        )
        self.ledger["evidence"].append(
            {
                "id": "EV-STK-0004",
                "path": "config/services.yaml",
                "source_type": "configuration",
                "authority": "Declares autowiring defaults",
                "confidence": "confirmed",
                "supported_claims": ["services.yaml enables autowire"],
            }
        )
        self.assertNotIn("SCAN_DISPOSITION_CONFLICT", self.codes())

    def test_claiming_to_have_read_inside_a_forbidden_tree_is_a_contradiction(
        self,
    ) -> None:
        """The other direction stays blocking: narrow claim, broad refusal."""
        (self.target / "config/jwt").mkdir(parents=True)
        (self.target / "config/jwt/private.pem").write_text("x\n", encoding="utf-8")
        self.coverage["surfaces"].append(
            {
                "surface": "config",
                "kind": "tree",
                "disposition": "not-permitted",
                "reason": "Treated as secret-bearing for this run",
                "evidence_ids": [],
            }
        )
        second = {
            "scanner": "security-compliance-scanner",
            "target_root": str(self.target),
            "surfaces": [
                {
                    "surface": "config/jwt/private.pem",
                    "kind": "file",
                    "disposition": "covered",
                    "reason": "Read the signing key",
                    "evidence_ids": ["EV-SEC-0001"],
                }
            ],
        }
        (self.task / "security-compliance-scanner-coverage.json").write_text(
            json.dumps(second, indent=2) + "\n", encoding="utf-8"
        )
        (self.task / "security-compliance-scanner-evidence.json").write_text(
            json.dumps(
                {
                    "scanner": "security-compliance-scanner",
                    "target_root": str(self.target),
                    "evidence": [
                        {
                            "id": "EV-SEC-0001",
                            "path": "config/jwt/private.pem",
                            "source_type": "configuration",
                            "authority": "Signing key",
                            "confidence": "confirmed",
                            "supported_claims": ["the key is present"],
                        }
                    ],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        self.assertIn("SCAN_DISPOSITION_CONFLICT", self.codes())

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

    def test_a_skill_naming_the_claim_carries_it_without_restating_it(self) -> None:
        """Schema 1.4 lets a plan answer the question outright."""
        self.write()
        path = self.task / "skill-generation-plan.json"
        path.write_text(
            json.dumps(
                {
                    "evidence": [],
                    "critical_invariants": [],
                    "skills": [{"name": "billing-review", "claim_ids": ["CLM-0001"]}],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        codes = [
            item.code
            for item in validator.validate(self.target, self.task, path)
            if item.severity == "error"
        ]
        self.assertEqual(codes, [])

    def test_a_skill_resting_on_a_claim_nobody_made_is_rejected(self) -> None:
        self.write()
        path = self.task / "skill-generation-plan.json"
        path.write_text(
            json.dumps(
                {
                    "evidence": [],
                    "critical_invariants": [],
                    "skills": [{"name": "billing-review", "claim_ids": ["CLM-9999"]}],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        codes = [
            item.code
            for item in validator.validate(self.target, self.task, path)
            if item.severity == "error"
        ]
        self.assertIn("CLAIM_ID_UNKNOWN", codes)

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


class ScanVolumeTest(ScanCoverageFixture):
    """A scan is complete when every surface has a disposition, not when every
    file was opened.

    Reported from a real run: scanning takes forty minutes and the reports are
    enormous. One tree disposition already accounts for everything under it -
    three entries describe a small target in full - so an enumerated coverage
    record and a report that restates the ledger are choices, and both are now
    named as choices.
    """

    def test_a_tree_disposition_accounts_for_its_children(self) -> None:
        # The honest minimum: nothing is enumerated directory by directory.
        self.assertEqual([code for code in self.codes() if code.startswith("SCAN_")], [])

    def test_an_enumerated_coverage_record_is_reported(self) -> None:
        for index in range(validator.COVERAGE_VERBOSE_LIMIT + 1):
            self.coverage["surfaces"].append({
                "surface": f"src/Module{index}/**",
                "kind": "tree",
                "disposition": "excluded",
                "reason": "enumerated one by one instead of named as a tree",
                "evidence_ids": [],
            })
        self.assertIn("SCAN_COVERAGE_VERBOSE", self.codes("warning"))

    def test_a_report_that_restates_the_ledger_is_reported(self) -> None:
        self.write()
        (self.task / "stack-scanner-findings.md").write_text(
            "\n".join(f"- line {index}" for index in range(validator.REPORT_OVERLONG_LIMIT + 5)),
            encoding="utf-8",
        )
        self.assertIn("SCAN_REPORT_OVERLONG", self.codes("warning"))

    def test_a_short_report_is_left_alone(self) -> None:
        self.write()
        (self.task / "stack-scanner-findings.md").write_text(
            "# Stack Scan\n\n- PHP 8.3 (confirmed - EV-STK-0001)\n", encoding="utf-8"
        )
        self.assertNotIn("SCAN_REPORT_OVERLONG", self.codes("warning"))


class OwnershipCoverageTest(ScanCoverageFixture):
    """What the selected skills leave unowned has to be said out loud.

    Discovery says what was read; the plan says what is owned; nothing joined
    the two. Measured on a real run: the CI pipeline the scan had just found
    belonged to nobody, and so did the stored GraphQL documents - both
    invisible, because the only question ever asked was whether a selected
    skill was justified, never whether the selection left a hole.
    """

    def plan_owning(self, paths: list[str]) -> Path:
        path = self.task / "skill-generation-plan.json"
        path.write_text(
            json.dumps(
                {
                    "evidence": [{"id": "EV-0001", "path": "src/Billing/BillingService.php"}],
                    "critical_invariants": [],
                    "skills": [
                        {
                            "name": "coding",
                            "ownership": [{"id": "app.source", "mode": "exclusive",
                                           "description": "the application", "paths": paths}],
                            "writes": paths,
                            "path_contracts": [],
                        }
                    ],
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        return path

    def declare(self, entries: list[dict]) -> None:
        (self.task / "plan-ownership.json").write_text(
            json.dumps({"target_root": str(self.target), "unowned": entries}, indent=2) + "\n",
            encoding="utf-8",
        )

    def ownership_codes(self, plan: Path) -> list[str]:
        return sorted({
            item.code
            for item in validator.validate(self.target, self.task, plan)
            if item.code.startswith("OWNERSHIP")
        })

    def test_a_plan_that_owns_everything_read_is_silent(self) -> None:
        self.write()
        plan = self.plan_owning(["**"])
        self.assertEqual(self.ownership_codes(plan), [])

    def test_a_surface_nobody_owns_is_reported(self) -> None:
        self.write()
        plan = self.plan_owning(["src/**"])
        self.assertIn("OWNERSHIP_COVERAGE_MISSING", self.ownership_codes(plan))

    def test_declaring_it_unowned_settles_it(self) -> None:
        self.write()
        plan = self.plan_owning(["src/**"])
        self.declare([
            {"surface": "composer.json", "reason": "The manifest is not this accelerator's to change"},
            {"surface": "tests/**", "reason": "No test skill was justified for this target"},
            {"surface": "config/**", "reason": "Framework configuration nobody generated a skill for"},
        ])
        self.assertEqual(self.ownership_codes(plan), [])

    def test_declaring_an_owned_surface_unowned_is_reported(self) -> None:
        self.write()
        plan = self.plan_owning(["src/**"])
        self.declare([{"surface": "src/**", "reason": "claimed unowned while a skill owns it"}])
        self.assertIn("OWNERSHIP_DECLARATION_UNKNOWN", self.ownership_codes(plan))

    def test_declaring_a_surface_nobody_read_is_reported(self) -> None:
        self.write()
        plan = self.plan_owning(["**"])
        self.declare([{"surface": "docs/**", "reason": "no scanner ever reported reading this"}])
        self.assertIn("OWNERSHIP_DECLARATION_UNKNOWN", self.ownership_codes(plan))

    def test_a_forbidden_surface_owes_no_owner(self) -> None:
        # `.env` is not-permitted, so nobody has to own it.
        self.write()
        plan = self.plan_owning(["**"])
        self.assertEqual(self.ownership_codes(plan), [])

    def test_a_malformed_declaration_is_refused(self) -> None:
        self.write()
        plan = self.plan_owning(["src/**"])
        self.declare([{"surface": "composer.json"}])
        self.assertIn("OWNERSHIP_COVERAGE_INVALID", self.ownership_codes(plan))


if __name__ == "__main__":
    unittest.main()
