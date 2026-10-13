#!/usr/bin/env python3
"""Build the authoritative working or release 75-run disposition ledger."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any, Dict, Iterable, List, Mapping, Optional, Tuple

from common import (
    CODEX_CASE_IDS,
    CODEX_DEFECT_NAMESPACE,
    CODEX_RUN_IDS,
    CURRENT_WORKBOOK,
    EVIDENCE_ROOT,
    FINAL_DISPOSITIONS,
    HISTORICAL_RUN_IDS,
    QaError,
    atomic_write_json,
    ensure_unique,
    load_json,
    load_openpyxl,
    parse_sha256_manifest,
    pick,
    records_from_json,
    repository_commit,
    repository_relative,
    require_primary_package,
    sha256_file,
    utc_now,
    verify_sha256_manifest,
)


def worksheet_records(workbook: Any, sheet_name: str) -> List[Dict[str, Any]]:
    if sheet_name not in workbook.sheetnames:
        raise QaError("Workbook is missing required sheet: {0}".format(sheet_name))
    sheet = workbook[sheet_name]
    rows = list(sheet.iter_rows(values_only=True))
    if not rows:
        raise QaError("Workbook sheet is empty: {0}".format(sheet_name))
    headers = [str(value).strip() if value is not None else "" for value in rows[0]]
    records: List[Dict[str, Any]] = []
    for values in rows[1:]:
        if not any(value not in (None, "") for value in values):
            continue
        records.append(
            {
                headers[index]: value
                for index, value in enumerate(values)
                if index < len(headers) and headers[index]
            }
        )
    return records


def normalize_run(record: Mapping[str, Any]) -> Dict[str, Any]:
    return {
        "run_id": str(pick(record, "run_id", "run id", default="")).strip(),
        "test_case_id": str(
            pick(record, "test_case_id", "test case id", default="")
        ).strip(),
        "historical_result": str(
            pick(record, "result", "historical_result", default="")
        ).strip(),
        "framework": str(pick(record, "framework", default="Unknown")).strip(),
        "tool": str(pick(record, "ai_tool", "tool", default="Unknown")).strip(),
        "tested_commit": str(
            pick(record, "git_commit", "tested_commit", default="Unknown")
        ).strip(),
        "original_defect_id": optional_string(
            pick(record, "defect_id", "defect id")
        ),
        "evidence_path": optional_string(
            pick(record, "evidence_link", "evidence path")
        ),
        "raw": dict(record),
    }


def optional_string(value: Any) -> Optional[str]:
    if value in (None, ""):
        return None
    return str(value).strip()


def normalize_defect(record: Mapping[str, Any]) -> Dict[str, Any]:
    return {
        "run_id": optional_string(pick(record, "run_id", "run id")),
        "test_case_id": optional_string(
            pick(record, "test_case_id", "test case id")
        ),
        "defect_id": optional_string(pick(record, "defect_id", "defect id")),
        "raw": dict(record),
    }


def composite_defect_map(
    records: Iterable[Mapping[str, Any]], source_name: str
) -> Dict[Tuple[str, str, str], Dict[str, Any]]:
    result: Dict[Tuple[str, str, str], Dict[str, Any]] = {}
    for source in records:
        defect = normalize_defect(source)
        if not all(
            defect[field] for field in ("run_id", "test_case_id", "defect_id")
        ):
            continue
        key = (
            str(defect["run_id"]),
            str(defect["test_case_id"]),
            str(defect["defect_id"]),
        )
        if key in result:
            raise QaError(
                "Duplicate defect composite in {0}: {1}".format(source_name, key)
            )
        result[key] = defect
    return result


def evidence_location(raw_path: Optional[str]) -> Optional[str]:
    if not raw_path:
        return None
    normalized = raw_path.replace("\\", "/")
    if normalized.startswith("Accelerator-TestEvidence/"):
        normalized = "logs/original/" + normalized.rsplit("/", 1)[-1]
    if normalized.endswith("TC-074.log"):
        return "transcripts/RUN-057-TC-074-sanitized.md"
    return normalized


def native_requirement(run: Mapping[str, Any]) -> str:
    if run["run_id"] == "RUN-20260803-018":
        return "multiple"
    tool = str(run["tool"]).lower()
    if "cursor" in tool:
        return "cursor"
    if "codex" in tool:
        return "codex"
    if "claude" in tool:
        return "claude-code"
    return "none"


def historical_records(paths: Mapping[str, Path]) -> List[Dict[str, Any]]:
    payload = load_json(paths["manifests/historical-runs.json"])
    records = records_from_json(payload, ("runs", "records", "rows", "data"))
    normalized = [normalize_run(record) for record in records]
    actual = [record["run_id"] for record in normalized]
    if len(normalized) != 57 or set(actual) != set(HISTORICAL_RUN_IDS):
        raise QaError(
            "historical-runs.json must contain exactly RUN-001..RUN-057 once"
        )
    ensure_unique(normalized, ("run_id",))
    return normalized


def codex_records(workbook_path: Path) -> Tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    openpyxl = load_openpyxl()
    workbook = openpyxl.load_workbook(workbook_path, data_only=False, read_only=False)
    runs = [
        normalize_run(record)
        for record in worksheet_records(workbook, "Test Runs")
        if str(record.get("Run ID", "")).startswith("RUN-20260803-")
    ]
    cases = [
        record
        for record in worksheet_records(workbook, "Test Cases")
        if str(record.get("Test Case ID", "")).startswith("TC-AI-")
    ]
    if len(runs) != 18 or set(record["run_id"] for record in runs) != set(
        CODEX_RUN_IDS
    ):
        raise QaError(
            "Current workbook must contain exactly RUN-20260803-001..018"
        )
    if len(cases) != 18 or set(str(row["Test Case ID"]) for row in cases) != set(
        CODEX_CASE_IDS
    ):
        raise QaError("Current workbook must contain exactly TC-AI-001..018")
    ensure_unique(runs, ("run_id",))
    defects = worksheet_records(workbook, "Defects")
    return runs, defects


def release_errors(records: Iterable[Mapping[str, Any]]) -> List[str]:
    errors: List[str] = []
    for record in records:
        identity = record["identity"]
        disposition = record.get("final_disposition")
        if disposition not in FINAL_DISPOSITIONS:
            errors.append("{0}: final_disposition is incomplete".format(identity))
        for field in ("final_evidence_path", "owner"):
            if not record.get(field):
                errors.append("{0}: {1} is required".format(identity, field))
        if record.get("review_status") not in {"approved", "rejected", "deferred"}:
            errors.append("{0}: review_status is not final".format(identity))
    return errors


def build_ledger(
    evidence_root: Path,
    workbook_path: Path,
    release: bool = False,
) -> Dict[str, Any]:
    paths = require_primary_package(evidence_root)
    checksum_errors = verify_sha256_manifest(paths["SHA256SUMS.txt"], evidence_root)
    if checksum_errors:
        raise QaError(
            "Evidence package checksum verification failed: {0}".format(
                "; ".join(checksum_errors)
            )
        )
    historical = historical_records(paths)
    codex, workbook_defects = codex_records(workbook_path)

    historical_other_sheets = load_json(
        paths["manifests/historical-othersheets.json"]
    )
    defect_matrix = historical_other_sheets.get("Defects")
    if (
        not isinstance(defect_matrix, list)
        or len(defect_matrix) != 10
        or not isinstance(defect_matrix[0], list)
    ):
        raise QaError(
            "historical-othersheets.json must contain one Defects header and "
            "nine historical defect rows"
        )
    defect_headers = defect_matrix[0]
    historical_defect_records = [
        {
            str(header): row[index] if index < len(row) else None
            for index, header in enumerate(defect_headers)
        }
        for row in defect_matrix[1:]
    ]
    historical_defects = composite_defect_map(
        historical_defect_records, "manifests/defects.json"
    )
    codex_defects = composite_defect_map(workbook_defects, "current workbook")

    checksums = parse_sha256_manifest(paths["SHA256SUMS.txt"])
    output_records: List[Dict[str, Any]] = []
    for source_group, source_records in (
        ("historical-57", historical),
        ("codex-18", codex),
    ):
        for run in source_records:
            original_defect = run["original_defect_id"]
            namespaced_defect = original_defect
            conflicts: List[str] = []

            if source_group == "codex-18":
                matching = [
                    key
                    for key in codex_defects
                    if key[0] == run["run_id"] and key[1] == run["test_case_id"]
                ]
                if run["run_id"] == "RUN-20260803-013":
                    matching = [
                        ("RUN-20260803-013", "TC-AI-013", "DEF-003")
                    ]
                    conflicts.append(
                        "Current Test Runs defect cell is blank; the rejected "
                        "Defects row supplies original DEF-003."
                    )
                if original_defect:
                    expected_key = (
                        run["run_id"],
                        run["test_case_id"],
                        original_defect,
                    )
                    if expected_key not in codex_defects:
                        raise QaError(
                            "Codex defect has no composite match: {0}".format(
                                expected_key
                            )
                        )
                    matching = [expected_key]
                if len(matching) > 1:
                    raise QaError(
                        "Codex run has ambiguous composite defect matches: {0}".format(
                            run["run_id"]
                        )
                    )
                if matching:
                    original_defect = matching[0][2]
                    try:
                        namespaced_defect = CODEX_DEFECT_NAMESPACE[matching[0]]
                    except KeyError as exc:
                        raise QaError(
                            "Unexpected Codex defect composite: {0}".format(matching[0])
                        ) from exc
            elif original_defect:
                key = (run["run_id"], run["test_case_id"], original_defect)
                if key not in historical_defects:
                    raise QaError(
                        "Historical defect has no composite manifest match: {0}".format(
                            key
                        )
                    )

            relative_evidence = evidence_location(run["evidence_path"])
            evidence_hash = checksums.get(relative_evidence or "")
            evidence_status = (
                "withheld-sensitive"
                if run["run_id"] == "RUN-057"
                else (
                    "available-original"
                    if relative_evidence and evidence_hash
                    else (
                        "workbook-recorded"
                        if source_group == "codex-18"
                        else "missing"
                    )
                )
            )
            identity_defect = namespaced_defect or "NONE"
            output_records.append(
                {
                    "identity": "{0}|{1}|{2}".format(
                        run["run_id"], run["test_case_id"], identity_defect
                    ),
                    "run_id": run["run_id"],
                    "test_case_id": run["test_case_id"],
                    "historical_result": run["historical_result"],
                    "framework": run["framework"],
                    "tool": run["tool"],
                    "tested_commit": run["tested_commit"],
                    "source_group": source_group,
                    "source_evidence_status": (
                        "package-verified"
                        if source_group == "historical-57"
                        else "workbook-recorded"
                    ),
                    "original_defect_id": original_defect,
                    "namespaced_defect_id": namespaced_defect,
                    "evidence_status": evidence_status,
                    "evidence_path": relative_evidence,
                    "evidence_sha256": evidence_hash,
                    "classification": None,
                    "remediation_workstream": None,
                    "required_regressions": [],
                    "native_client_requirement": native_requirement(run),
                    "final_disposition": None,
                    "final_evidence_path": None,
                    "owner": None,
                    "review_status": "pending",
                    "provenance": {
                        "sources": [
                            (
                                "manifests/historical-runs.json"
                                if source_group == "historical-57"
                                else repository_relative(workbook_path)
                            )
                        ],
                        "conflicts": conflicts,
                    },
                }
            )

    ensure_unique(output_records, ("run_id",))
    ensure_unique(output_records, ("identity",))
    if len(output_records) != 75:
        raise QaError("Ledger generation did not produce exactly 75 records")
    errors = release_errors(output_records) if release else []
    if errors:
        raise QaError("Release ledger validation failed: {0}".format("; ".join(errors)))

    source_checksums = {
        relative: sha256_file(path) for relative, path in paths.items()
    }
    source_checksums[repository_relative(workbook_path)] = sha256_file(workbook_path)
    return {
        "schema_version": "1.0.0",
        "generated_at": utc_now(),
        "repository_commit": repository_commit(),
        "release_ready": release,
        "source_checksums": source_checksums,
        "records": sorted(output_records, key=lambda record: record["run_id"]),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-root", type=Path, default=EVIDENCE_ROOT)
    parser.add_argument("--workbook", type=Path, default=CURRENT_WORKBOOK)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("docs/qa/75-run-disposition.json"),
    )
    parser.add_argument("--release", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        ledger = build_ledger(
            args.evidence_root.resolve(),
            args.workbook.resolve(),
            release=args.release,
        )
        atomic_write_json(args.output.resolve(), ledger)
    except QaError as exc:
        print("QA ledger build refused: {0}".format(exc), file=sys.stderr)
        return 2
    print("Wrote {0} validated ledger records to {1}".format(
        len(ledger["records"]), args.output
    ))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
