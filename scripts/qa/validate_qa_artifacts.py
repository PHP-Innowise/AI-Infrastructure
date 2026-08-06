#!/usr/bin/env python3
"""Validate ledger, reconstructed workbook, and deterministic evidence artifacts."""

from __future__ import annotations

import argparse
import sys
from collections import Counter
from pathlib import Path
from typing import Any, Dict, Iterable, List, Mapping, Optional, Set

from build_sha256_manifest import verify_manifest
from common import (
    ALL_RUN_IDS,
    CODEX_DEFECT_NAMESPACE,
    CODEX_RUN_IDS,
    EVIDENCE_ROOT,
    FINAL_DISPOSITIONS,
    ROOT,
    QaError,
    confined_manifest_artifact,
    load_json,
    load_openpyxl,
    parse_sha256_manifest,
)
from reconstruct_workbook import SHEET_SPEC, nonempty_data_rows
from run_tc_ai import assertion_summary, final_status, validate_catalog


EXPECTED_RESULTS = {
    "Pass": 52,
    "Partial": 15,
    "Fail": 2,
    "Blocked": 6,
}


def validate_schema_instance(
    instance: Any,
    schema_path: Path,
    label: str,
) -> List[str]:
    """Validate one artifact with Draft 2020-12, including format checks."""
    try:
        from jsonschema import Draft202012Validator, FormatChecker
        from jsonschema.exceptions import SchemaError
    except ImportError as exc:
        raise QaError(
            "jsonschema is required for QA artifact validation. Install "
            "requirements-qa.txt"
        ) from exc
    schema = load_json(schema_path)
    try:
        Draft202012Validator.check_schema(schema)
    except SchemaError as exc:
        return ["{0}: invalid schema: {1}".format(schema_path.name, exc.message)]
    validator = Draft202012Validator(schema, format_checker=FormatChecker())
    errors: List[str] = []
    for error in sorted(
        validator.iter_errors(instance),
        key=lambda item: tuple(str(part) for part in item.path),
    ):
        location = ".".join(str(part) for part in error.absolute_path) or "<root>"
        errors.append("{0} schema violation at {1}: {2}".format(label, location, error.message))
    return errors


def validate_strict_schema_documents(schema_root: Path) -> List[str]:
    errors: List[str] = []
    for name in (
        "disposition-ledger.schema.json",
        "run-evidence.schema.json",
        "tc-ai-case.schema.json",
    ):
        path = schema_root / name
        try:
            schema = load_json(path)
        except QaError as exc:
            errors.append(str(exc))
            continue
        if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
            errors.append("{0}: unsupported or missing $schema".format(name))
        if schema.get("additionalProperties") is not False:
            errors.append("{0}: root additionalProperties must be false".format(name))
        if not schema.get("required"):
            errors.append("{0}: root required list is missing".format(name))
    return errors


def validate_ledger(
    ledger: Mapping[str, Any],
    repository_root: Path,
    evidence_root: Path,
    release: bool,
) -> List[str]:
    errors: List[str] = []
    allowed_root = {
        "schema_version",
        "generated_at",
        "repository_commit",
        "release_ready",
        "source_checksums",
        "records",
    }
    extra = set(ledger) - allowed_root
    if extra:
        errors.append("ledger has unknown properties: {0}".format(sorted(extra)))
    records = ledger.get("records")
    if not isinstance(records, list):
        return errors + ["ledger records must be an array"]
    run_ids = [record.get("run_id") for record in records if isinstance(record, dict)]
    if len(records) != 75 or set(run_ids) != set(ALL_RUN_IDS):
        errors.append("ledger must contain exactly the authoritative 75 Run IDs")
    if len(set(run_ids)) != len(run_ids):
        errors.append("ledger contains duplicate Run IDs")
    identities = [record.get("identity") for record in records]
    if len(set(identities)) != len(identities):
        errors.append("ledger contains duplicate composite identities")
    results = Counter(record.get("historical_result") for record in records)
    if dict(results) != EXPECTED_RESULTS:
        errors.append(
            "historical result arithmetic mismatch: {0}".format(dict(results))
        )

    expected_namespace = {
        key[:2]: value for key, value in CODEX_DEFECT_NAMESPACE.items()
    }
    seen_namespaced = set()
    for record in records:
        if not isinstance(record, dict):
            errors.append("ledger records must be objects")
            continue
        run_id = record.get("run_id")
        case_id = record.get("test_case_id")
        defect_id = record.get("namespaced_defect_id")
        expected = expected_namespace.get((run_id, case_id))
        if expected:
            if defect_id != expected:
                errors.append(
                    "{0}: expected namespaced defect {1}".format(run_id, expected)
                )
            seen_namespaced.add(defect_id)
        elif run_id in CODEX_RUN_IDS and defect_id and str(defect_id).startswith(
            "DEF-00"
        ):
            errors.append("{0}: unnamespaced Codex defect".format(run_id))
        identity_defect = defect_id or "NONE"
        expected_identity = "{0}|{1}|{2}".format(run_id, case_id, identity_defect)
        if record.get("identity") != expected_identity:
            errors.append("{0}: composite identity mismatch".format(run_id))

        evidence_path = record.get("evidence_path")
        evidence_sha = record.get("evidence_sha256")
        if evidence_path:
            path = Path(str(evidence_path))
            if path.is_absolute() or ".." in path.parts:
                errors.append("{0}: evidence path is not repository-relative".format(run_id))
            candidates = [repository_root / path, evidence_root / path]
            existing = next((candidate for candidate in candidates if candidate.is_file()), None)
            if evidence_sha and existing:
                from common import sha256_file

                if sha256_file(existing) != evidence_sha:
                    errors.append("{0}: evidence checksum mismatch".format(run_id))
            elif release and record.get("evidence_status") not in {
                "withheld-sensitive",
                "workbook-recorded",
            }:
                errors.append("{0}: evidence path does not resolve".format(run_id))

        if release:
            if record.get("final_disposition") not in FINAL_DISPOSITIONS:
                errors.append("{0}: final disposition is incomplete".format(run_id))
            if not record.get("final_evidence_path"):
                errors.append("{0}: final evidence path is required".format(run_id))
            if not record.get("owner"):
                errors.append("{0}: owner is required".format(run_id))
            if record.get("review_status") not in {
                "approved",
                "rejected",
                "deferred",
            }:
                errors.append("{0}: review status is not final".format(run_id))
        if str(record.get("final_disposition", "")).lower() == "unknown":
            errors.append("{0}: unknown final disposition is forbidden".format(run_id))

    if seen_namespaced != set(expected_namespace.values()):
        errors.append("not all four Codex defects are represented")
    run_057 = next(
        (record for record in records if record.get("run_id") == "RUN-057"), None
    )
    if not run_057 or run_057.get("evidence_status") != "withheld-sensitive":
        errors.append("RUN-057 must represent the withheld sensitive artifact")
    checksum_path = evidence_root / "SHA256SUMS.txt"
    if checksum_path.is_file():
        checksum_text = checksum_path.read_text(encoding="utf-8")
        if "WITHHELD/TC-074.log" not in checksum_text:
            errors.append("SHA256SUMS.txt lacks WITHHELD/TC-074.log")
    elif release:
        errors.append("evidence package SHA256SUMS.txt is missing")
    return errors


def validate_workbook(path: Path) -> List[str]:
    errors: List[str] = []
    openpyxl = load_openpyxl()
    workbook = openpyxl.load_workbook(path, data_only=False)
    for sheet_name, (count, width, reference, _) in SHEET_SPEC.items():
        if sheet_name not in workbook.sheetnames:
            errors.append("workbook lacks sheet {0}".format(sheet_name))
            continue
        sheet = workbook[sheet_name]
        if len(nonempty_data_rows(sheet)) != count:
            errors.append("{0}: incorrect data-row count".format(sheet_name))
        if sheet.max_column != width:
            errors.append("{0}: incorrect column count".format(sheet_name))
        tables = list(sheet.tables.values())
        if len(tables) != 1 or tables[0].ref != reference:
            errors.append("{0}: table range must be {1}".format(sheet_name, reference))
        headers = [sheet.cell(1, column).value for column in range(1, width + 1)]
        if len(headers) != len(set(headers)):
            errors.append("{0}: duplicate headers".format(sheet_name))
    if "Provenance" not in workbook.sheetnames:
        errors.append("workbook lacks Provenance sheet")
    if "Test Runs" in workbook.sheetnames:
        run_ids = [
            str(workbook["Test Runs"].cell(row, 1).value)
            for row in nonempty_data_rows(workbook["Test Runs"])
        ]
        if set(run_ids) != set(ALL_RUN_IDS):
            errors.append("workbook Run ID set is not the authoritative 75")
    if "Dashboard" not in workbook.sheetnames:
        errors.append("workbook lacks Dashboard")
    else:
        for coordinate in (
            "B4",
            "B5",
            "B6",
            "B7",
            "B8",
            "B9",
            "B10",
            "B11",
            "B12",
            "B13",
            "B14",
            "B15",
            "B16",
            "B17",
            "B18",
        ):
            value = workbook["Dashboard"][coordinate].value
            if not isinstance(value, str) or not value.startswith("="):
                errors.append("Dashboard formula missing at {0}".format(coordinate))
            elif "#REF!" in value:
                errors.append("Dashboard formula contains #REF! at {0}".format(coordinate))
    return errors


def validate_run_evidence(
    path: Path,
    expected_run_id: str,
    expected_case: Optional[Mapping[str, Any]] = None,
) -> List[str]:
    errors: List[str] = []
    required_files = {
        "run.json",
        "case.json",
        "environment.json",
        "commands.json",
        "exit-codes.json",
        "assertions.json",
        "git-before.txt",
        "git-after.txt",
        "inventory.json",
        "provenance.json",
        "SHA256SUMS.txt",
    }
    if path.is_symlink():
        return ["{0}: evidence directory must not be a symlink".format(expected_run_id)]
    missing = sorted(
        name
        for name in required_files
        if not (path / name).is_file() or (path / name).is_symlink()
    )
    if missing:
        return [
            "{0}: missing, non-regular, or symlinked {1}".format(
                expected_run_id, ", ".join(missing)
            )
        ]
    if expected_case is None:
        catalog = validate_catalog(load_json(ROOT / "qa" / "tc-ai" / "cases.json"))
        expected_case = next(
            (case for case in catalog if case.get("run_id") == expected_run_id),
            None,
        )
    if expected_case is None:
        return ["{0}: run is absent from the TC-AI catalog".format(expected_run_id)]
    run = load_json(path / "run.json")
    errors.extend(
        validate_schema_instance(
            run,
            ROOT / "schemas" / "qa" / "run-evidence.schema.json",
            expected_run_id,
        )
    )
    if run.get("run_id") != expected_run_id:
        errors.append("{0}: run.json Run ID mismatch".format(expected_run_id))
    if run.get("test_case_id") != expected_case.get("test_case_id"):
        errors.append("{0}: catalog test-case identity mismatch".format(expected_run_id))
    if run.get("classification") != expected_case.get("classification"):
        errors.append("{0}: catalog classification mismatch".format(expected_run_id))
    case_document = load_json(path / "case.json")
    if case_document != expected_case:
        errors.append("{0}: case.json does not match the catalog".format(expected_run_id))
    assertions = run.get("assertions")
    assertion_records: List[Mapping[str, Any]] = []
    if not isinstance(assertions, dict) or not isinstance(assertions.get("items"), list):
        errors.append("{0}: malformed assertion summary".format(expected_run_id))
    else:
        assertion_records = [
            item for item in assertions["items"] if isinstance(item, dict)
        ]
        if len(assertion_records) != len(assertions["items"]):
            errors.append("{0}: assertion items are malformed".format(expected_run_id))
        recomputed = assertion_summary(assertions["items"])
        for field in ("executed", "passed", "failed", "skipped"):
            if assertions.get(field) != recomputed[field]:
                errors.append(
                    "{0}: assertion arithmetic mismatch for {1}".format(
                        expected_run_id, field
                    )
                )
        if run.get("status") != final_status(recomputed):
            errors.append(
                "{0}: run status does not match assertion outcomes".format(
                    expected_run_id
                )
            )
    commands_document = load_json(path / "commands.json")
    assertions_document = load_json(path / "assertions.json")
    exit_codes_document = load_json(path / "exit-codes.json")
    raw_commands = run.get("commands")
    command_records = (
        [command for command in raw_commands if isinstance(command, dict)]
        if isinstance(raw_commands, list)
        else []
    )
    if not isinstance(raw_commands, list) or len(command_records) != len(raw_commands):
        errors.append("{0}: run commands are malformed".format(expected_run_id))
    if commands_document != raw_commands:
        errors.append("{0}: commands.json does not match run.json".format(expected_run_id))
    if assertions_document != assertions:
        errors.append("{0}: assertions.json does not match run.json".format(expected_run_id))
    expected_exit_codes = {
        command.get("id"): command.get("exit_code")
        for command in command_records
    }
    if exit_codes_document != expected_exit_codes:
        errors.append("{0}: exit-codes.json does not match run.json".format(expected_run_id))
    assertion_ids = [
        record.get("id")
        for record in assertion_records
        if isinstance(record.get("id"), str)
    ]
    if len(assertion_ids) != len(assertion_records):
        errors.append("{0}: assertion IDs are malformed".format(expected_run_id))
    if len(assertion_ids) != len(set(assertion_ids)):
        errors.append("{0}: assertion IDs must be unique".format(expected_run_id))
    assertions_by_id = {
        record.get("id"): record
        for record in assertion_records
        if isinstance(record.get("id"), str)
    }
    command_ids = [
        command.get("id")
        for command in command_records
        if isinstance(command.get("id"), str)
    ]
    if len(command_ids) != len(command_records):
        errors.append("{0}: command IDs are malformed".format(expected_run_id))
    if len(command_ids) != len(set(command_ids)):
        errors.append("{0}: command IDs must be unique".format(expected_run_id))
    for command in command_records:
        command_id = command.get("id")
        exit_code = command.get("exit_code")
        matching_assertion = assertions_by_id.get(command_id)
        if matching_assertion is None:
            errors.append(
                "{0}: command {1} has no matching assertion".format(
                    expected_run_id, command_id
                )
            )
        elif isinstance(exit_code, int):
            expected_status = "passed" if exit_code == 0 else "failed"
            if matching_assertion.get("status") != expected_status:
                errors.append(
                    "{0}: command {1} exit code disagrees with assertion".format(
                        expected_run_id, command_id
                    )
                )
        for field in ("stdout_path", "stderr_path"):
            relative = command.get(field)
            try:
                stream = confined_manifest_artifact(path, relative or "")
            except QaError:
                stream = None
            if stream is None or not stream.is_file() or stream.is_symlink():
                errors.append(
                    "{0}: command stream is missing: {1}".format(
                        expected_run_id, relative
                    )
                )
        if not isinstance(exit_code, int):
            errors.append("{0}: command exit code is missing".format(expected_run_id))
    inventory = load_json(path / "inventory.json")
    inventory_files = inventory.get("files") if isinstance(inventory, dict) else None
    inventory_file_set = (
        set(inventory_files)
        if isinstance(inventory_files, list)
        and all(isinstance(item, str) for item in inventory_files)
        else set()
    )
    expected_inventory_files = {
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
            for command in command_records
            for field in ("stdout_path", "stderr_path")
            if isinstance(command.get(field), str)
        ),
    }
    if (
        not isinstance(inventory_files, list)
        or not expected_inventory_files.issubset(inventory_file_set)
        or not isinstance(inventory, dict)
        or not isinstance(inventory.get("repository_status_changed"), bool)
    ):
        errors.append("{0}: inventory.json is incomplete".format(expected_run_id))
    environment = load_json(path / "environment.json")
    if not isinstance(environment, dict) or not all(
        isinstance(environment.get(field), str) and environment.get(field)
        for field in (
            "branch",
            "captured_at",
            "executable",
            "platform",
            "python",
            "repository_commit",
            "repository_root",
        )
    ):
        errors.append("{0}: environment.json is incomplete".format(expected_run_id))
    provenance = load_json(path / "provenance.json")
    if (
        not isinstance(provenance, dict)
        or provenance.get("catalog") != "qa/tc-ai/cases.json"
        or provenance.get("runner") != "scripts/qa/run_tc_ai.py"
        or provenance.get("historical_claim_used_as_pass_evidence") is not False
        or not isinstance(provenance.get("repository_commit"), str)
        or not provenance.get("repository_commit")
        or not isinstance(provenance.get("generated_at"), str)
        or not provenance.get("generated_at")
    ):
        errors.append("{0}: provenance.json is incomplete".format(expected_run_id))
    native = run.get("native", {})
    if not isinstance(native, dict):
        errors.append("{0}: native evidence is malformed".format(expected_run_id))
        native = {}
    if native.get("required") and native.get("status") == "passed":
        errors.append(
            "{0}: deterministic runner may not mark native evidence passed".format(
                expected_run_id
            )
        )
    try:
        manifest_errors = verify_manifest(path, path / "SHA256SUMS.txt")
    except QaError as exc:
        manifest_errors = [str(exc)]
    errors.extend(
        "{0}: {1}".format(expected_run_id, error)
        for error in manifest_errors
    )
    return errors


def validate_evidence_root(path: Path, release: bool) -> List[str]:
    errors: List[str] = []
    existing = {entry.name for entry in path.iterdir() if entry.is_dir()} if path.is_dir() else set()
    if release and existing != set(CODEX_RUN_IDS):
        errors.append("release evidence root must contain exactly 18 Codex Run directories")
    for run_id in sorted(existing & set(CODEX_RUN_IDS)):
        errors.extend(validate_run_evidence(path / run_id, run_id))
    unexpected = sorted(existing - set(CODEX_RUN_IDS))
    if unexpected:
        errors.append("unexpected evidence directories: {0}".format(", ".join(unexpected)))
    return errors


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--ledger", type=Path, default=ROOT / "docs" / "qa" / "75-run-disposition.json"
    )
    parser.add_argument(
        "--workbook",
        type=Path,
        default=ROOT / "Accelerator TestCases.reconstructed.xlsx",
    )
    parser.add_argument(
        "--run-evidence", type=Path, default=ROOT / "docs" / "qa" / "evidence"
    )
    parser.add_argument("--evidence-root", type=Path, default=EVIDENCE_ROOT)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--skip-ledger", action="store_true")
    parser.add_argument("--skip-workbook", action="store_true")
    parser.add_argument("--skip-run-evidence", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    errors = validate_strict_schema_documents(ROOT / "schemas" / "qa")
    try:
        catalog = load_json(ROOT / "qa" / "tc-ai" / "cases.json")
        errors.extend(
            validate_schema_instance(
                catalog,
                ROOT / "schemas" / "qa" / "tc-ai-case.schema.json",
                "TC-AI catalog",
            )
        )
        validate_catalog(catalog)
        if not args.skip_ledger:
            if not args.ledger.is_file():
                errors.append("ledger is missing: {0}".format(args.ledger))
            else:
                ledger = load_json(args.ledger)
                errors.extend(
                    validate_schema_instance(
                        ledger,
                        ROOT / "schemas" / "qa" / "disposition-ledger.schema.json",
                        "disposition ledger",
                    )
                )
                errors.extend(
                    validate_ledger(
                        ledger,
                        ROOT,
                        args.evidence_root,
                        args.release,
                    )
                )
        if not args.skip_workbook:
            if not args.workbook.is_file():
                errors.append("reconstructed workbook is missing: {0}".format(args.workbook))
            else:
                errors.extend(validate_workbook(args.workbook))
        if not args.skip_run_evidence:
            errors.extend(validate_evidence_root(args.run_evidence, args.release))
    except (QaError, OSError, ValueError) as exc:
        errors.append(str(exc))
    if errors:
        for error in errors:
            print("ERROR: {0}".format(error), file=sys.stderr)
        return 2
    print("QA artifacts validated{0}".format(" for release" if args.release else ""))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
