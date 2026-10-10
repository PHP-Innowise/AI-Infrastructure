#!/usr/bin/env python3
"""Apply reviewed owner policy and reproduced TC-AI evidence to a working ledger."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Any, Dict, Mapping

from common import (
    CODEX_RUN_IDS,
    ROOT,
    QaError,
    atomic_write_json,
    load_json,
    sha256_file,
)
from validate_qa_artifacts import validate_run_evidence


DEFAULT_LEDGER = ROOT / "docs" / "qa" / "75-run-disposition.json"
DEFAULT_DECISIONS = ROOT / "docs" / "qa" / "disposition-decisions.json"


def apply_decisions(
    ledger: Mapping[str, Any],
    decisions: Mapping[str, Any],
    repository_root: Path = ROOT,
) -> Dict[str, Any]:
    records = ledger.get("records")
    if not isinstance(records, list) or len(records) != 75:
        raise QaError("Working ledger must contain exactly 75 records")

    accelerator_owner = str(decisions.get("accelerator_owner", "")).strip()
    external_owner = str(decisions.get("external_owner", "")).strip()
    evidence_root = repository_root / str(decisions.get("codex_evidence_root", ""))
    external_runs = set(decisions.get("external_runs", []))
    approval_policy = decisions.get("approval_policy")
    historical_regressions = decisions.get("historical_regressions", {})
    if not accelerator_owner or not external_owner:
        raise QaError("Decision policy requires accelerator_owner and external_owner")
    if external_runs != {"RUN-053", "RUN-056", "RUN-057"}:
        raise QaError("External run policy must identify RUN-053, RUN-056, and RUN-057")
    if not isinstance(historical_regressions, dict) or any(
        not isinstance(defect_id, str)
        or not isinstance(patterns, list)
        or not patterns
        or not all(isinstance(pattern, str) and pattern for pattern in patterns)
        for defect_id, patterns in historical_regressions.items()
    ):
        raise QaError("Historical regression policy must map defects to test patterns")
    if (
        not isinstance(approval_policy, dict)
        or approval_policy.get("id") != "evidence-backed-local-v1"
        or approval_policy.get("eligible_review_status") != "approved"
        or approval_policy.get("ineligible_review_status") != "pending"
    ):
        raise QaError("Decision policy lacks the supported evidence-backed approval rule")

    case_catalog = load_json(repository_root / "qa" / "tc-ai" / "cases.json")
    cases = {
        case["run_id"]: case
        for case in case_catalog.get("cases", [])
        if isinstance(case, dict)
    }
    if set(cases) != set(CODEX_RUN_IDS):
        raise QaError("TC-AI catalog must contain exactly all 18 Codex runs")

    updated = dict(ledger)
    updated_records = []
    for source in records:
        if not isinstance(source, dict):
            raise QaError("Ledger records must be objects")
        record = dict(source)
        run_id = str(record.get("run_id", ""))

        if run_id in external_runs:
            record.update(
                {
                    "classification": "external-client-finding",
                    "remediation_workstream": "external-client",
                    "owner": external_owner,
                    "review_status": "pending",
                    "final_disposition": None,
                    "final_evidence_path": None,
                }
            )
        elif record.get("source_group") == "historical-57":
            record["owner"] = accelerator_owner
            defect_id = record.get("namespaced_defect_id")
            if defect_id in historical_regressions:
                record.update(
                    {
                        "classification": "accelerator-finding",
                        "remediation_workstream": "post-remediation-verification",
                        "required_regressions": historical_regressions[defect_id],
                    }
                )
            if (
                record.get("historical_result") == "Pass"
                and not defect_id
                and record.get("native_client_requirement") == "none"
                and decisions.get("historical_pass_policy")
                == "propose-accepted-historical-pass"
            ):
                original = record.get("evidence_path")
                record.update(
                    {
                        "classification": "historical-evidence-pass",
                        "remediation_workstream": "historical-evidence-acceptance",
                        "final_disposition": "accepted-historical-pass",
                        "final_evidence_path": (
                            "docs/Accelerator-TestEvidence-Submission/" + str(original)
                            if original
                            else None
                        ),
                        "review_status": "approved",
                    }
                )
            elif record.get("historical_result") == "Pass" and defect_id:
                record.update(
                    {
                        "classification": "accelerator-finding",
                        "remediation_workstream": "post-remediation-verification",
                        "final_disposition": None,
                        "final_evidence_path": None,
                        "review_status": "pending",
                    }
                )
            elif (
                record.get("historical_result") == "Pass"
                and record.get("native_client_requirement") != "none"
            ):
                record.update(
                    {
                        "classification": "native-client-verification",
                        "remediation_workstream": "native-client-closure",
                        "final_disposition": None,
                        "final_evidence_path": None,
                        "review_status": "pending",
                    }
                )
        elif record.get("source_group") == "codex-18":
            record["owner"] = accelerator_owner
            case = cases[run_id]
            run_path = evidence_root / run_id / "run.json"
            if not run_path.is_file():
                raise QaError("Missing reproduced run evidence: {0}".format(run_path))
            reproduced = load_json(run_path)
            if reproduced.get("run_id") != run_id:
                raise QaError("Reproduced evidence run ID mismatch: {0}".format(run_id))
            evidence_errors = validate_run_evidence(
                run_path.parent,
                run_id,
                case,
            )
            if evidence_errors:
                raise QaError(
                    "Reproduced evidence is incomplete or invalid for {0}: {1}".format(
                        run_id, "; ".join(evidence_errors)
                    )
                )

            relative_run = run_path.relative_to(repository_root).as_posix()
            relative_directory = run_path.parent.relative_to(repository_root).as_posix()
            record.update(
                {
                    "source_evidence_status": "newly-reproduced",
                    "evidence_status": "newly-reproduced",
                    "evidence_path": relative_run,
                    "evidence_sha256": sha256_file(run_path),
                    "classification": case["classification"],
                    "remediation_workstream": "codex-18-remediation",
                    "required_regressions": [
                        item["regex"]
                        for item in case.get("required_test_patterns", [])
                    ],
                    "final_evidence_path": relative_directory,
                    "review_status": "pending",
                }
            )
            if reproduced.get("status") == "passed":
                if run_id == "RUN-20260803-013":
                    record["final_disposition"] = "closed-rejected-finding"
                elif record.get("historical_result") == "Pass":
                    record["final_disposition"] = "verified-unmodified-pass"
                else:
                    record["final_disposition"] = "verified-after-remediation"
                record["review_status"] = "approved"
            else:
                record["final_disposition"] = None
                record["review_status"] = "pending"

        updated_records.append(record)

    updated["records"] = updated_records
    return updated


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Apply owner/disposition proposals and reproduced TC-AI evidence."
    )
    parser.add_argument("--ledger", type=Path, default=DEFAULT_LEDGER)
    parser.add_argument("--decisions", type=Path, default=DEFAULT_DECISIONS)
    parser.add_argument("--output", type=Path, default=DEFAULT_LEDGER)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        updated = apply_decisions(load_json(args.ledger), load_json(args.decisions))
        atomic_write_json(args.output, updated)
    except (OSError, QaError, ValueError) as exc:
        print("Disposition update failed: {0}".format(exc))
        return 2
    print("Applied disposition proposals to {0}".format(args.output))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
