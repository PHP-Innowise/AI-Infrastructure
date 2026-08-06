#!/usr/bin/env python3
"""Reconstruct the authoritative 75-run workbook without altering either input."""

from __future__ import annotations

import argparse
import copy
import os
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, Iterable, List, Mapping, Sequence, Tuple

from common import (
    CODEX_CASE_IDS,
    CODEX_DEFECT_NAMESPACE,
    CODEX_RUN_IDS,
    CURRENT_WORKBOOK,
    EVIDENCE_ROOT,
    HISTORICAL_RUN_IDS,
    RECONSTRUCTED_WORKBOOK,
    QaError,
    load_json,
    load_openpyxl,
    records_from_json,
    require_primary_package,
    sha256_file,
    utc_now,
    verify_sha256_manifest,
)


SHEET_SPEC = {
    "Test Cases": (92, 18, "A1:R93", "tblTestCases"),
    "Test Runs": (75, 30, "A1:AD76", "tblTestRuns"),
    "Quality Metrics": (71, 19, "A1:S72", "tblQualityMetrics"),
    "Efficiency & Tokens": (75, 24, "A1:X76", "tblEfficiencyTokens"),
    "Context & Brain": (44, 26, "A1:Z45", "tblContextBrain"),
    "Defects": (13, 22, "A1:V14", "tblDefects"),
}

HISTORICAL_COUNTS = {
    "Test Cases": 74,
    "Test Runs": 57,
    "Quality Metrics": 53,
    "Efficiency & Tokens": 57,
    "Context & Brain": 26,
    "Defects": 9,
}


def nonempty_data_rows(sheet: Any) -> List[int]:
    return [
        row
        for row in range(2, sheet.max_row + 1)
        if any(sheet.cell(row, column).value not in (None, "") for column in range(1, sheet.max_column + 1))
    ]


def header_map(sheet: Any, limit: int) -> Dict[str, int]:
    result: Dict[str, int] = {}
    for column in range(1, limit + 1):
        value = sheet.cell(1, column).value
        if value not in (None, ""):
            result[str(value).strip()] = column
    return result


def row_identity(sheet: Any, row: int) -> str:
    value = sheet.cell(row, 1).value
    return "" if value is None else str(value).strip()


def verify_historical_snapshot(workbook: Any) -> None:
    for sheet_name, expected_count in HISTORICAL_COUNTS.items():
        if sheet_name not in workbook.sheetnames:
            raise QaError(
                "Historical workbook is missing required sheet: {0}".format(sheet_name)
            )
        rows = nonempty_data_rows(workbook[sheet_name])
        if len(rows) != expected_count:
            raise QaError(
                "Historical {0} must contain {1} data rows; found {2}".format(
                    sheet_name, expected_count, len(rows)
                )
            )
    run_ids = {
        row_identity(workbook["Test Runs"], row)
        for row in nonempty_data_rows(workbook["Test Runs"])
    }
    if run_ids != set(HISTORICAL_RUN_IDS):
        raise QaError("Historical workbook does not contain exactly RUN-001..RUN-057")


def verify_manifests(paths: Mapping[str, Path]) -> None:
    historical_runs = records_from_json(
        load_json(paths["manifests/historical-runs.json"]),
        ("runs", "records", "rows", "data"),
    )
    ids = set()
    for record in historical_runs:
        run_id = (
            record.get("Run ID")
            or record.get("run_id")
            or record.get("RunID")
        )
        if run_id:
            ids.add(str(run_id))
    if len(historical_runs) != 57 or ids != set(HISTORICAL_RUN_IDS):
        raise QaError(
            "historical-runs.json must independently contain RUN-001..RUN-057"
        )
    historical_cases = records_from_json(
        load_json(paths["manifests/historical-testcases.json"]),
        ("test_cases", "cases", "records", "rows", "data"),
    )
    if len(historical_cases) != 74:
        raise QaError("historical-testcases.json must contain exactly 74 records")
    # Parsing this file is mandatory even though the snapshot remains authoritative
    # for formatting and values.
    load_json(paths["manifests/historical-othersheets.json"])


def copy_cell(source: Any, target: Any) -> None:
    target.value = source.value
    if source.has_style:
        target._style = copy.copy(source._style)
    target.number_format = source.number_format
    target.font = copy.copy(source.font)
    target.fill = copy.copy(source.fill)
    target.border = copy.copy(source.border)
    target.alignment = copy.copy(source.alignment)
    target.protection = copy.copy(source.protection)
    if source.hyperlink:
        target._hyperlink = copy.copy(source.hyperlink)
    if source.comment:
        target.comment = copy.copy(source.comment)


def append_selected_rows(
    target: Any,
    source: Any,
    selected_ids: Iterable[str],
    width: int,
    defect_namespace: bool = False,
) -> None:
    selected = set(selected_ids)
    source_headers = header_map(source, source.max_column)
    target_headers = header_map(target, width)
    missing_headers = sorted(set(target_headers) - set(source_headers))
    if missing_headers:
        raise QaError(
            "{0} source is missing columns: {1}".format(
                source.title, ", ".join(missing_headers)
            )
        )
    source_rows = {
        row_identity(source, row): row for row in nonempty_data_rows(source)
    }
    if set(source_rows).intersection(selected) != selected:
        missing = sorted(selected - set(source_rows))
        raise QaError(
            "{0} is missing required rows: {1}".format(
                source.title, ", ".join(missing)
            )
        )
    for identity in sorted(selected):
        source_row = source_rows[identity]
        target_row = target.max_row + 1
        for header, target_column in target_headers.items():
            source_column = source_headers[header]
            copy_cell(
                source.cell(source_row, source_column),
                target.cell(target_row, target_column),
            )
        if source.row_dimensions[source_row].height is not None:
            target.row_dimensions[target_row].height = source.row_dimensions[
                source_row
            ].height
        if defect_namespace:
            run_id = str(target.cell(target_row, target_headers["Run ID"]).value)
            case_id = str(
                target.cell(target_row, target_headers["Test Case ID"]).value
            )
            original = str(
                target.cell(target_row, target_headers["Defect ID"]).value
            )
            key = (run_id, case_id, original)
            if key not in CODEX_DEFECT_NAMESPACE:
                raise QaError("Unexpected Codex defect composite: {0}".format(key))
            target.cell(
                target_row, target_headers["Defect ID"]
            ).value = CODEX_DEFECT_NAMESPACE[key]


def append_codex_defects(target: Any, source: Any) -> None:
    source_headers = header_map(source, source.max_column)
    target_headers = header_map(target, 22)
    required = {"Defect ID", "Run ID", "Test Case ID"}
    if not required.issubset(source_headers) or not required.issubset(target_headers):
        raise QaError("Defects sheet lacks composite identity columns")
    matched = set()
    for source_row in nonempty_data_rows(source):
        key = (
            str(source.cell(source_row, source_headers["Run ID"]).value),
            str(source.cell(source_row, source_headers["Test Case ID"]).value),
            str(source.cell(source_row, source_headers["Defect ID"]).value),
        )
        if key not in CODEX_DEFECT_NAMESPACE:
            continue
        target_row = target.max_row + 1
        for header, target_column in target_headers.items():
            copy_cell(
                source.cell(source_row, source_headers[header]),
                target.cell(target_row, target_column),
            )
        target.cell(
            target_row, target_headers["Defect ID"]
        ).value = CODEX_DEFECT_NAMESPACE[key]
        matched.add(key)
    if matched != set(CODEX_DEFECT_NAMESPACE):
        raise QaError(
            "Current workbook lacks one or more Codex defect composite rows"
        )


def namespace_codex_run_defects(sheet: Any) -> None:
    headers = header_map(sheet, 30)
    rows = {
        row_identity(sheet, row): row for row in nonempty_data_rows(sheet)
    }
    for composite, namespaced in CODEX_DEFECT_NAMESPACE.items():
        run_id, case_id, original = composite
        row = rows.get(run_id)
        if row is None:
            raise QaError("Reconstructed Test Runs is missing {0}".format(run_id))
        if str(sheet.cell(row, headers["Test Case ID"]).value) != case_id:
            raise QaError("Test case mismatch for {0}".format(run_id))
        existing = sheet.cell(row, headers["Defect ID"]).value
        if run_id != "RUN-20260803-013" and str(existing) != original:
            raise QaError("Defect mismatch for {0}".format(run_id))
        sheet.cell(row, headers["Defect ID"]).value = namespaced


def set_table_reference(
    openpyxl: Any, sheet: Any, table_name: str, reference: str
) -> None:
    tables = list(sheet.tables.values())
    if tables:
        tables[0].ref = reference
        if tables[0].displayName != table_name:
            tables[0].displayName = table_name
            tables[0].name = table_name
        for extra in tables[1:]:
            del sheet.tables[extra.name]
    else:
        table = openpyxl.worksheet.table.Table(
            displayName=table_name, ref=reference
        )
        table.tableStyleInfo = openpyxl.worksheet.table.TableStyleInfo(
            name="TableStyleMedium2",
            showFirstColumn=False,
            showLastColumn=False,
            showRowStripes=True,
            showColumnStripes=False,
        )
        sheet.add_table(table)


def set_dashboard_formulas(workbook: Any) -> None:
    if "Dashboard" not in workbook.sheetnames:
        raise QaError("Historical workbook is missing Dashboard")
    dashboard = workbook["Dashboard"]
    formulas = {
        "B4": "=ROWS(tblTestRuns[Run ID])",
        "B5": '=COUNTIF(tblTestRuns[Result],"Pass")',
        "B6": '=COUNTIF(tblTestRuns[Result],"Partial")',
        "B7": '=COUNTIF(tblTestRuns[Result],"Fail")',
        "B8": '=COUNTIF(tblTestRuns[Result],"Blocked")',
        "B9": '=IF(B4=0,0,B5/B4)',
        "B10": '=IFERROR(COUNTIF(tblEfficiencyTokens[Successful First Attempt],"Yes")/ROWS(tblEfficiencyTokens[Run ID]),0)',
        "B11": '=IFERROR(AVERAGEIF(tblQualityMetrics[Run ID],"RUN-???",tblQualityMetrics[Overall Score]),0)',
        "B12": '=IFERROR(AVERAGEIF(tblEfficiencyTokens[Run ID],"RUN-???",tblEfficiencyTokens[Total Duration (seconds)]),0)',
        "B13": '=IF(COUNT(tblEfficiencyTokens[Total Tokens])=0,"N/A",AVERAGE(tblEfficiencyTokens[Total Tokens]))',
        "B14": '=IF(COUNT(tblEfficiencyTokens[Estimated Cost])=0,"N/A",AVERAGE(tblEfficiencyTokens[Estimated Cost]))',
        "B15": "=IFERROR(AVERAGE(tblTestRuns[Human Intervention Count]),0)",
        "B16": "=SUM(tblTestRuns[Safety Violation Count])",
        "B17": '=IFERROR(COUNTIF(tblContextBrain[Retrieval Successful],"Yes")/(COUNTIF(tblContextBrain[Retrieval Successful],"Yes")+COUNTIF(tblContextBrain[Retrieval Successful],"No")),0)',
        "B18": '=COUNTIFS(tblDefects[Severity],"Critical",tblDefects[Status],"<>Closed",tblDefects[Status],"<>Rejected")',
    }
    for coordinate, formula in formulas.items():
        dashboard[coordinate] = formula


def add_provenance_sheet(
    workbook: Any, snapshot: Path, current: Path, paths: Mapping[str, Path]
) -> None:
    if "Provenance" in workbook.sheetnames:
        del workbook["Provenance"]
    sheet = workbook.create_sheet("Provenance")
    sheet.append(["Source", "SHA-256", "Role", "Precedence"])
    sheet.append(
        [
            snapshot.as_posix(),
            sha256_file(snapshot),
            "Historical formatting, formulas, and 57-run values",
            1,
        ]
    )
    sheet.append(
        [
            "manifests/historical-runs.json",
            sha256_file(paths["manifests/historical-runs.json"]),
            "Exact historical run corroboration",
            2,
        ]
    )
    sheet.append(
        [
            current.as_posix(),
            sha256_file(current),
            "TC-AI-001..018 definitions, runs, analytics, and defects",
            3,
        ]
    )
    sheet.append(["Generated At", utc_now(), "UTC", None])
    sheet.freeze_panes = "A2"


def validate_reconstructed(workbook: Any) -> None:
    for sheet_name, (count, width, reference, table_name) in SHEET_SPEC.items():
        sheet = workbook[sheet_name]
        rows = nonempty_data_rows(sheet)
        if len(rows) != count:
            raise QaError(
                "{0} has {1} data rows; expected {2}".format(
                    sheet_name, len(rows), count
                )
            )
        if sheet.max_column != width:
            raise QaError(
                "{0} has {1} columns; expected {2}".format(
                    sheet_name, sheet.max_column, width
                )
            )
        tables = list(sheet.tables.values())
        if len(tables) != 1 or tables[0].ref != reference:
            raise QaError(
                "{0} table range must be {1}".format(sheet_name, reference)
            )
        headers = [sheet.cell(1, column).value for column in range(1, width + 1)]
        if len(headers) != len(set(headers)):
            raise QaError("{0} has duplicate column headers".format(sheet_name))
    if "Provenance" not in workbook.sheetnames:
        raise QaError("Reconstructed workbook lacks Provenance sheet")
    run_ids = [
        row_identity(workbook["Test Runs"], row)
        for row in nonempty_data_rows(workbook["Test Runs"])
    ]
    if len(set(run_ids)) != 75:
        raise QaError("Reconstructed workbook contains duplicate Run IDs")
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
            raise QaError("Dashboard formula is missing at {0}".format(coordinate))


def reconstruct(
    evidence_root: Path,
    current_path: Path,
    output_path: Path,
    force: bool = False,
) -> None:
    paths = require_primary_package(evidence_root)
    errors = verify_sha256_manifest(paths["SHA256SUMS.txt"], evidence_root)
    if errors:
        raise QaError(
            "Evidence package checksum verification failed: {0}".format(
                "; ".join(errors)
            )
        )
    verify_manifests(paths)
    snapshot_path = paths["manifests/workbook-snapshot-20260803-2002.xlsx"]
    if output_path.resolve() in {snapshot_path.resolve(), current_path.resolve()}:
        raise QaError("Output must not overwrite either source workbook")
    if output_path.exists() and not force:
        raise QaError(
            "Output already exists; pass --force to replace it: {0}".format(
                output_path
            )
        )

    openpyxl = load_openpyxl()
    historical = openpyxl.load_workbook(snapshot_path, data_only=False)
    current = openpyxl.load_workbook(current_path, data_only=False)
    verify_historical_snapshot(historical)

    append_selected_rows(
        historical["Test Cases"],
        current["Test Cases"],
        CODEX_CASE_IDS,
        18,
    )
    append_selected_rows(
        historical["Test Runs"],
        current["Test Runs"],
        CODEX_RUN_IDS,
        30,
    )
    append_selected_rows(
        historical["Quality Metrics"],
        current["Quality Metrics"],
        CODEX_RUN_IDS,
        19,
    )
    append_selected_rows(
        historical["Efficiency & Tokens"],
        current["Efficiency & Tokens"],
        CODEX_RUN_IDS,
        24,
    )
    append_selected_rows(
        historical["Context & Brain"],
        current["Context & Brain"],
        CODEX_RUN_IDS,
        26,
    )
    append_codex_defects(historical["Defects"], current["Defects"])
    namespace_codex_run_defects(historical["Test Runs"])

    for sheet_name, (_, width, reference, table_name) in SHEET_SPEC.items():
        sheet = historical[sheet_name]
        if sheet.max_column > width:
            sheet.delete_cols(width + 1, sheet.max_column - width)
        set_table_reference(openpyxl, sheet, table_name, reference)
    set_dashboard_formulas(historical)
    add_provenance_sheet(historical, snapshot_path, current_path, paths)
    validate_reconstructed(historical)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=".{0}.".format(output_path.name),
        suffix=".xlsx",
        dir=str(output_path.parent),
    )
    os.close(descriptor)
    temporary = Path(temporary_name)
    try:
        historical.save(temporary)
        reopened = openpyxl.load_workbook(temporary, data_only=False)
        validate_reconstructed(reopened)
        os.replace(str(temporary), str(output_path))
    except Exception:
        temporary.unlink(missing_ok=True)
        raise


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence-root", type=Path, default=EVIDENCE_ROOT)
    parser.add_argument("--current", type=Path, default=CURRENT_WORKBOOK)
    parser.add_argument("--output", type=Path, default=RECONSTRUCTED_WORKBOOK)
    parser.add_argument("--force", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        reconstruct(
            args.evidence_root.resolve(),
            args.current.resolve(),
            args.output.resolve(),
            force=args.force,
        )
    except (QaError, OSError, ValueError) as exc:
        print("Workbook reconstruction refused: {0}".format(exc), file=sys.stderr)
        return 2
    print("Wrote reconstructed workbook: {0}".format(args.output))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
