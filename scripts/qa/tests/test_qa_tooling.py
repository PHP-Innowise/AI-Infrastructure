from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path


QA_SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = QA_SCRIPTS.parents[1]
sys.path.insert(0, str(QA_SCRIPTS))

from build_disposition_ledger import composite_defect_map  # noqa: E402
from build_sha256_manifest import render_manifest, verify_manifest  # noqa: E402
from apply_disposition_decisions import apply_decisions  # noqa: E402
from common import QaError, load_json, verify_sha256_manifest  # noqa: E402
from reconstruct_workbook import reconstruct, sanitize_machine_paths  # noqa: E402
from run_tc_ai import (  # noqa: E402
    expand_argv,
    run_case,
    sanitized_argv,
    validate_catalog,
)
from validate_qa_artifacts import (  # noqa: E402
    validate_run_evidence,
    validate_schema_instance,
    validate_strict_schema_documents,
)


class QaToolingTests(unittest.TestCase):
    def test_schemas_are_strict_draft_2020_12(self) -> None:
        self.assertEqual(
            [],
            validate_strict_schema_documents(ROOT / "schemas" / "qa"),
        )

    def test_catalog_has_exact_18_case_identity(self) -> None:
        cases = validate_catalog(load_json(ROOT / "qa" / "tc-ai" / "cases.json"))
        self.assertEqual(18, len(cases))
        self.assertEqual(18, len({case["run_id"] for case in cases}))
        self.assertEqual(18, len({case["test_case_id"] for case in cases}))

    def test_catalog_unknown_property_is_rejected_by_schema(self) -> None:
        catalog = load_json(ROOT / "qa" / "tc-ai" / "cases.json")
        catalog["unexpected"] = True
        errors = validate_schema_instance(
            catalog,
            ROOT / "schemas" / "qa" / "tc-ai-case.schema.json",
            "catalog",
        )
        self.assertTrue(any("Additional properties" in error for error in errors))

    def test_shell_commands_use_pinned_python_and_evidence_redacts_paths(self) -> None:
        argv = expand_argv(
            [
                "bash",
                "-c",
                "python3 -m unittest; {python_shell} -V; cd {root_shell}",
            ],
            ROOT,
        )
        self.assertNotIn("python3 -m unittest", argv[2])
        self.assertIn(sys.executable, argv[2])
        self.assertIn(str(ROOT), argv[2])

        sanitized = sanitized_argv(argv, ROOT)
        joined = " ".join(sanitized)
        self.assertNotIn(sys.executable, joined)
        self.assertNotIn(str(ROOT), joined)
        self.assertIn("{python}", joined)
        self.assertIn("{root}", joined)

    def test_duplicate_defect_composite_is_rejected(self) -> None:
        duplicate = {
            "Run ID": "RUN-004",
            "Test Case ID": "TC-004",
            "Defect ID": "DEF-001",
        }
        with self.assertRaisesRegex(QaError, "Duplicate defect composite"):
            composite_defect_map([duplicate, duplicate], "synthetic")

    def test_missing_package_refuses_without_ledger_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_name:
            temporary = Path(temporary_name)
            from build_disposition_ledger import build_ledger

            with self.assertRaisesRegex(QaError, "incomplete"):
                build_ledger(
                    temporary / "missing-package",
                    ROOT / "Accelerator TestCases.xlsx",
                )

    def test_workbook_refusal_leaves_no_partial_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_name:
            temporary = Path(temporary_name)
            output = temporary / "Accelerator TestCases.reconstructed.xlsx"
            with self.assertRaisesRegex(QaError, "incomplete"):
                reconstruct(
                    temporary / "missing-package",
                    ROOT / "Accelerator TestCases.xlsx",
                    output,
                )
            self.assertFalse(output.exists())
            self.assertEqual([], list(temporary.glob("*.xlsx")))

    def test_generated_workbook_redacts_machine_local_repository_paths(self) -> None:
        from openpyxl import Workbook

        workbook = Workbook()
        sheet = workbook.active
        sheet["A1"] = "/home/tester/Desktop/AI-Infrastructure/status"
        sheet["A2"] = "/Users/tester/work/accelerator-php/docs/source.json"
        sheet["A3"] = "=SUM(B1:B2)"

        sanitize_machine_paths(workbook)

        self.assertEqual("{repository}/status", sheet["A1"].value)
        self.assertEqual("{repository}/docs/source.json", sheet["A2"].value)
        self.assertEqual("=SUM(B1:B2)", sheet["A3"].value)

    def test_checksum_manifest_cannot_escape_or_follow_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_name:
            base = Path(temporary_name)
            evidence = base / "evidence"
            evidence.mkdir()
            outside = base / "outside.txt"
            outside.write_text("outside\n", encoding="utf-8")
            (evidence / "linked.txt").symlink_to(outside)
            manifest = evidence / "SHA256SUMS.txt"
            digest = "a" * 64
            manifest.write_text(
                "{0}  ../outside.txt\n{0}  {1}\n{0}  linked.txt\n".format(
                    digest, outside
                ),
                encoding="utf-8",
            )

            errors = verify_sha256_manifest(manifest, evidence)

            self.assertEqual(3, len(errors))
            self.assertTrue(any("Unsafe checksum entry" in error for error in errors))
            self.assertTrue(any("contains a symlink" in error for error in errors))

            malformed_manifests = (
                "one-token-line\n",
                "{0}  file.txt\n{0}  file.txt\n".format("a" * 64),
                "{0}  file.txt\n".format("z" * 64),
            )
            for content in malformed_manifests:
                with self.subTest(content=content):
                    manifest.write_text(content, encoding="utf-8")
                    with self.assertRaises(QaError):
                        verify_manifest(evidence, manifest)

    def test_malformed_evidence_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_name:
            evidence = Path(temporary_name)
            for name in (
                "case.json",
                "environment.json",
                "commands.json",
                "exit-codes.json",
                "assertions.json",
                "inventory.json",
                "provenance.json",
            ):
                (evidence / name).write_text("{}\n", encoding="utf-8")
            (evidence / "run.json").write_text(
                json.dumps(
                    {
                        "run_id": "RUN-20260803-001",
                        "commands": [{"id": "bad", "exit_code": "zero"}],
                        "assertions": {
                            "executed": 2,
                            "passed": 1,
                            "failed": 0,
                            "skipped": 0,
                            "items": [
                                {
                                    "id": "one",
                                    "status": "passed",
                                    "message": "ok",
                                }
                            ],
                        },
                        "native": {"required": False, "status": "not-applicable"},
                    }
                ),
                encoding="utf-8",
            )
            for name in ("git-before.txt", "git-after.txt", "SHA256SUMS.txt"):
                (evidence / name).write_text("", encoding="utf-8")
            errors = validate_run_evidence(evidence, "RUN-20260803-001")
            self.assertTrue(any("assertion arithmetic" in error for error in errors))
            self.assertTrue(any("exit code" in error for error in errors))

    def test_unavailable_native_case_is_skipped_never_passed(self) -> None:
        catalog = validate_catalog(load_json(ROOT / "qa" / "tc-ai" / "cases.json"))
        case = next(
            item for item in catalog if item["run_id"] == "RUN-20260803-018"
        )
        with tempfile.TemporaryDirectory() as temporary_name:
            destination = run_case(
                case,
                ROOT,
                Path(temporary_name),
                available_hosts=set(),
            )
            run = load_json(destination / "run.json")
            self.assertEqual("skipped", run["status"])
            self.assertEqual("skipped", run["native"]["status"])
            self.assertEqual(0, run["assertions"]["passed"])
            self.assertGreaterEqual(run["assertions"]["skipped"], 1)
            self.assertEqual(
                [],
                validate_run_evidence(destination, "RUN-20260803-018"),
            )

    def test_unavailable_optional_executable_is_skipped_never_failed(self) -> None:
        case = {
            "run_id": "RUN-20260803-008",
            "test_case_id": "TC-AI-008",
            "title": "Synthetic optional executable",
            "classification": "deterministic",
            "commands": [
                {
                    "id": "optional-tool",
                    "argv": ["definitely-unavailable-qa-tool"],
                    "cwd": ".",
                    "requires_executable": "definitely-unavailable-qa-tool",
                }
            ],
            "required_test_patterns": [],
            "native_requirement": None,
            "blocker": None,
        }
        with tempfile.TemporaryDirectory() as temporary_name:
            destination = run_case(
                case,
                ROOT,
                Path(temporary_name),
                available_hosts=set(),
            )
            run = load_json(destination / "run.json")
            self.assertEqual("skipped", run["status"])
            self.assertEqual(0, run["assertions"]["failed"])
            self.assertEqual(1, run["assertions"]["skipped"])
            self.assertEqual([], run["commands"])
            self.assertEqual(
                [],
                validate_run_evidence(destination, "RUN-20260803-008", case),
            )

    def write_complete_run_evidence(
        self,
        evidence_root: Path,
        run_id: str,
        expected_case: dict,
        status: str,
    ) -> None:
        run_path = evidence_root / run_id
        run_path.mkdir(parents=True)
        assertion_status = "passed" if status == "passed" else "skipped"
        commands = []
        if status == "passed":
            (run_path / "stdout").mkdir()
            (run_path / "stderr").mkdir()
            (run_path / "stdout/check.txt").write_text("ok\n", encoding="utf-8")
            (run_path / "stderr/check.txt").write_text("", encoding="utf-8")
            commands.append(
                {
                    "id": "synthetic-check",
                    "argv": ["python3", "-V"],
                    "cwd": ".",
                    "started_at": "2026-08-06T08:00:00Z",
                    "ended_at": "2026-08-06T08:00:01Z",
                    "exit_code": 0,
                    "stdout_path": "stdout/check.txt",
                    "stderr_path": "stderr/check.txt",
                }
            )
        run = {
            "schema_version": "1.0.0",
            "run_id": run_id,
            "test_case_id": expected_case["test_case_id"],
            "classification": expected_case["classification"],
            "started_at": "2026-08-06T08:00:00Z",
            "ended_at": "2026-08-06T08:00:01Z",
            "status": status,
            "commands": commands,
            "assertions": {
                "executed": 1,
                "passed": 1 if status == "passed" else 0,
                "failed": 0,
                "skipped": 0 if status == "passed" else 1,
                "items": [
                    {
                        "id": (
                            commands[0]["id"] if commands else "synthetic-evidence"
                        ),
                        "status": assertion_status,
                        "message": "Complete synthetic policy fixture",
                    }
                ],
            },
            "native": {
                "required": False,
                "status": "not-applicable",
                "reason": None,
            },
        }
        (run_path / "run.json").write_text(
            json.dumps(run),
            encoding="utf-8",
        )
        documents = {
            "case.json": expected_case,
            "environment.json": {
                "branch": "main",
                "captured_at": "2026-08-06T08:00:01Z",
                "executable": "python3",
                "platform": "synthetic",
                "python": "3.9",
                "repository_commit": "a" * 40,
                "repository_root": str(ROOT),
            },
            "commands.json": commands,
            "exit-codes.json": {
                command["id"]: command["exit_code"] for command in commands
            },
            "assertions.json": run["assertions"],
            "inventory.json": {
                "files": [
                    "run.json",
                    "case.json",
                    "environment.json",
                    "commands.json",
                    "exit-codes.json",
                    "assertions.json",
                    "git-before.txt",
                    "git-after.txt",
                    *(
                        command[field]
                        for command in commands
                        for field in ("stdout_path", "stderr_path")
                    ),
                ],
                "repository_status_changed": False,
            },
            "provenance.json": {
                "catalog": "qa/tc-ai/cases.json",
                "generated_at": "2026-08-06T08:00:01Z",
                "historical_claim_used_as_pass_evidence": False,
                "repository_commit": "a" * 40,
                "runner": "scripts/qa/run_tc_ai.py",
            },
        }
        for name, document in documents.items():
            (run_path / name).write_text(
                json.dumps(document),
                encoding="utf-8",
            )
        (run_path / "git-before.txt").write_text("clean\n", encoding="utf-8")
        (run_path / "git-after.txt").write_text("clean\n", encoding="utf-8")
        manifest = run_path / "SHA256SUMS.txt"
        manifest.write_text(render_manifest(run_path, manifest), encoding="utf-8")

    def test_run_evidence_binds_identity_status_documents_and_regular_files(self) -> None:
        catalog = load_json(ROOT / "qa" / "tc-ai" / "cases.json")
        expected_case = next(
            case for case in catalog["cases"]
            if case["run_id"] == "RUN-20260803-001"
        )

        with tempfile.TemporaryDirectory() as temporary_name:
            evidence_root = Path(temporary_name)
            self.write_complete_run_evidence(
                evidence_root, expected_case["run_id"], expected_case, "passed"
            )
            run_path = evidence_root / expected_case["run_id"]
            run = load_json(run_path / "run.json")
            run["test_case_id"] = "TC-AI-018"
            (run_path / "run.json").write_text(json.dumps(run), encoding="utf-8")
            manifest = run_path / "SHA256SUMS.txt"
            manifest.write_text(render_manifest(run_path, manifest), encoding="utf-8")
            errors = validate_run_evidence(run_path, expected_case["run_id"])
            self.assertTrue(any("identity mismatch" in error for error in errors))

        with tempfile.TemporaryDirectory() as temporary_name:
            evidence_root = Path(temporary_name)
            self.write_complete_run_evidence(
                evidence_root, expected_case["run_id"], expected_case, "passed"
            )
            run_path = evidence_root / expected_case["run_id"]
            run = load_json(run_path / "run.json")
            run["assertions"]["items"][0]["status"] = "failed"
            run["assertions"].update({"passed": 0, "failed": 1})
            (run_path / "run.json").write_text(json.dumps(run), encoding="utf-8")
            (run_path / "assertions.json").write_text(
                json.dumps(run["assertions"]), encoding="utf-8"
            )
            manifest = run_path / "SHA256SUMS.txt"
            manifest.write_text(render_manifest(run_path, manifest), encoding="utf-8")
            errors = validate_run_evidence(run_path, expected_case["run_id"])
            self.assertTrue(any("status does not match" in error for error in errors))

        with tempfile.TemporaryDirectory() as temporary_name:
            evidence_root = Path(temporary_name)
            self.write_complete_run_evidence(
                evidence_root, expected_case["run_id"], expected_case, "passed"
            )
            run_path = evidence_root / expected_case["run_id"]
            run = load_json(run_path / "run.json")
            run["commands"][0]["exit_code"] = 1
            (run_path / "run.json").write_text(json.dumps(run), encoding="utf-8")
            (run_path / "commands.json").write_text(
                json.dumps(run["commands"]), encoding="utf-8"
            )
            (run_path / "exit-codes.json").write_text(
                json.dumps({run["commands"][0]["id"]: 1}),
                encoding="utf-8",
            )
            manifest = run_path / "SHA256SUMS.txt"
            manifest.write_text(render_manifest(run_path, manifest), encoding="utf-8")
            errors = validate_run_evidence(run_path, expected_case["run_id"])
            self.assertTrue(
                any("exit code disagrees with assertion" in error for error in errors)
            )

        with tempfile.TemporaryDirectory() as temporary_name:
            evidence_root = Path(temporary_name)
            self.write_complete_run_evidence(
                evidence_root, expected_case["run_id"], expected_case, "passed"
            )
            run_path = evidence_root / expected_case["run_id"]
            (run_path / "commands.json").write_text("{}\n", encoding="utf-8")
            manifest = run_path / "SHA256SUMS.txt"
            manifest.write_text(render_manifest(run_path, manifest), encoding="utf-8")
            errors = validate_run_evidence(run_path, expected_case["run_id"])
            self.assertTrue(any("commands.json does not match" in error for error in errors))

        with tempfile.TemporaryDirectory() as temporary_name:
            evidence_root = Path(temporary_name)
            self.write_complete_run_evidence(
                evidence_root, expected_case["run_id"], expected_case, "passed"
            )
            run_path = evidence_root / expected_case["run_id"]
            commands = run_path / "commands.json"
            outside = evidence_root / "outside.json"
            outside.write_text(commands.read_text(encoding="utf-8"), encoding="utf-8")
            commands.unlink()
            commands.symlink_to(outside)
            errors = validate_run_evidence(run_path, expected_case["run_id"])
            self.assertTrue(any("symlinked commands.json" in error for error in errors))

    def test_disposition_policy_preserves_external_and_native_pending_records(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary_name:
            repository = Path(temporary_name)
            catalog_path = repository / "qa" / "tc-ai" / "cases.json"
            catalog_path.parent.mkdir(parents=True)
            catalog = load_json(ROOT / "qa" / "tc-ai" / "cases.json")
            catalog_path.write_text(json.dumps(catalog), encoding="utf-8")
            cases = {case["run_id"]: case for case in catalog["cases"]}
            evidence_root = repository / "docs" / "qa" / "evidence"
            for number in range(1, 19):
                run_id = "RUN-20260803-{0:03d}".format(number)
                status = "skipped" if number == 18 else "passed"
                self.write_complete_run_evidence(
                    evidence_root,
                    run_id,
                    cases[run_id],
                    status,
                )

            records = []
            for number in range(1, 58):
                run_id = "RUN-{0:03d}".format(number)
                records.append(
                    {
                        "run_id": run_id,
                        "source_group": "historical-57",
                        "historical_result": "Pass",
                        "evidence_path": "logs/{0}.log".format(run_id),
                        "native_client_requirement": "none",
                        "review_status": "pending",
                    }
                )
            records[42]["namespaced_defect_id"] = "DEF-006"
            records[1]["native_client_requirement"] = "claude-code"
            for number in range(1, 19):
                records.append(
                    {
                        "run_id": "RUN-20260803-{0:03d}".format(number),
                        "source_group": "codex-18",
                        "historical_result": "Pass",
                        "review_status": "pending",
                    }
                )
            decisions = {
                "accelerator_owner": "Accelerator Team",
                "external_owner": "External Client Team",
                "historical_pass_policy": "propose-accepted-historical-pass",
                "historical_regressions": {
                    "DEF-006": ["test_direct_queries_reject_before_mutation"]
                },
                "codex_evidence_root": "docs/qa/evidence",
                "external_runs": ["RUN-053", "RUN-056", "RUN-057"],
                "approval_policy": {
                    "id": "evidence-backed-local-v1",
                    "eligible_review_status": "approved",
                    "ineligible_review_status": "pending",
                },
            }
            updated = apply_decisions(
                {"records": records},
                decisions,
                repository_root=repository,
            )
            by_id = {record["run_id"]: record for record in updated["records"]}
            self.assertEqual(
                "accepted-historical-pass", by_id["RUN-001"]["final_disposition"]
            )
            self.assertEqual("approved", by_id["RUN-001"]["review_status"])
            self.assertIsNone(by_id["RUN-002"]["final_disposition"])
            self.assertEqual("pending", by_id["RUN-002"]["review_status"])
            self.assertEqual(
                "native-client-closure",
                by_id["RUN-002"]["remediation_workstream"],
            )
            self.assertIsNone(by_id["RUN-043"]["final_disposition"])
            self.assertEqual("pending", by_id["RUN-043"]["review_status"])
            self.assertEqual(
                "post-remediation-verification",
                by_id["RUN-043"]["remediation_workstream"],
            )
            self.assertEqual(
                ["test_direct_queries_reject_before_mutation"],
                by_id["RUN-043"]["required_regressions"],
            )
            self.assertIsNone(by_id["RUN-053"]["final_disposition"])
            self.assertEqual("External Client Team", by_id["RUN-053"]["owner"])
            self.assertEqual(
                "verified-unmodified-pass",
                by_id["RUN-20260803-001"]["final_disposition"],
            )
            self.assertEqual(
                "approved", by_id["RUN-20260803-001"]["review_status"]
            )
            self.assertIsNone(by_id["RUN-20260803-018"]["final_disposition"])
            self.assertEqual("pending", by_id["RUN-20260803-018"]["review_status"])

            (
                evidence_root
                / "RUN-20260803-001"
                / "stdout"
                / "check.txt"
            ).unlink()
            with self.assertRaisesRegex(QaError, "incomplete or invalid"):
                apply_decisions(
                    {"records": records},
                    decisions,
                    repository_root=repository,
                )


if __name__ == "__main__":
    unittest.main()
