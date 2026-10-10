#!/usr/bin/env python3
"""Shared, dependency-light helpers for QA artifact tooling."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence


ROOT = Path(__file__).resolve().parents[2]
EVIDENCE_ROOT = ROOT / "docs" / "Accelerator-TestEvidence-Submission"
CURRENT_WORKBOOK = ROOT / "Accelerator TestCases.xlsx"
RECONSTRUCTED_WORKBOOK = ROOT / "Accelerator TestCases.reconstructed.xlsx"

HISTORICAL_RUN_IDS = tuple("RUN-{0:03d}".format(index) for index in range(1, 58))
CODEX_RUN_IDS = tuple(
    "RUN-20260803-{0:03d}".format(index) for index in range(1, 19)
)
CODEX_CASE_IDS = tuple("TC-AI-{0:03d}".format(index) for index in range(1, 19))
ALL_RUN_IDS = HISTORICAL_RUN_IDS + CODEX_RUN_IDS

CODEX_DEFECT_NAMESPACE = {
    ("RUN-20260803-011", "TC-AI-011", "DEF-001"): "DEF-AI-001",
    ("RUN-20260803-012", "TC-AI-012", "DEF-002"): "DEF-AI-002",
    ("RUN-20260803-013", "TC-AI-013", "DEF-003"): "DEF-AI-003",
    ("RUN-20260803-014", "TC-AI-014", "DEF-004"): "DEF-AI-004",
}

PRIMARY_PACKAGE_FILES = (
    "ACCELERATOR-TEST-EVIDENCE-REPORT.md",
    "SHA256SUMS.txt",
    "manifests/historical-runs.json",
    "manifests/historical-testcases.json",
    "manifests/historical-othersheets.json",
    "manifests/defects.json",
    "manifests/cases_map.json",
    "manifests/bwb_cases.json",
    "manifests/workbook-snapshot-20260803-2002.xlsx",
)

FINAL_DISPOSITIONS = {
    "accepted-historical-pass",
    "verified-after-remediation",
    "closed-supported-defect",
    "closed-test-definition-defect",
    "closed-environment-or-setup-artifact",
    "closed-rejected-finding",
    "verified-unmodified-pass",
    "accepted-limitation",
    "accepted-native-host-limitation",
    "external-client-finding-verified",
}


class QaError(RuntimeError):
    """An expected validation or source-availability failure."""


def utc_now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace(
        "+00:00", "Z"
    )


def load_json(path: Path) -> Any:
    try:
        with path.open("r", encoding="utf-8") as handle:
            return json.load(handle)
    except FileNotFoundError as exc:
        raise QaError("Required file is missing: {0}".format(path)) from exc
    except json.JSONDecodeError as exc:
        raise QaError("Invalid JSON in {0}: {1}".format(path, exc)) from exc


def atomic_write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    serialized = json.dumps(value, indent=2, ensure_ascii=False, sort_keys=True) + "\n"
    atomic_write_text(path, serialized)


def atomic_write_text(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=".{0}.".format(path.name), dir=str(path.parent)
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(value)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(str(temporary), str(path))
    except Exception:
        temporary.unlink(missing_ok=True)
        raise


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_primary_package(evidence_root: Path) -> Dict[str, Path]:
    paths = {
        relative: evidence_root / relative for relative in PRIMARY_PACKAGE_FILES
    }
    missing = [relative for relative, path in paths.items() if not path.is_file()]
    if missing:
        raise QaError(
            "Authoritative evidence package is incomplete; refusing to fabricate "
            "historical QA data. Missing: {0}".format(", ".join(sorted(missing)))
        )
    return paths


def repository_relative(path: Path, root: Path = ROOT) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError as exc:
        raise QaError(
            "Evidence path must be inside the repository: {0}".format(path)
        ) from exc


def git_output(
    args: Sequence[str], root: Path = ROOT, check: bool = True
) -> str:
    completed = subprocess.run(
        ["git"] + list(args),
        cwd=str(root),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if check and completed.returncode != 0:
        raise QaError(
            "git {0} failed ({1}): {2}".format(
                " ".join(args), completed.returncode, completed.stderr.strip()
            )
        )
    return completed.stdout


def repository_commit(root: Path = ROOT) -> str:
    return git_output(["rev-parse", "HEAD"], root=root).strip()


def records_from_json(value: Any, preferred_keys: Iterable[str]) -> List[Dict[str, Any]]:
    """Extract records while accepting the known manifest envelope variants."""
    if isinstance(value, list):
        records = value
    elif isinstance(value, dict):
        records = None
        for key in preferred_keys:
            candidate = value.get(key)
            if isinstance(candidate, list):
                header = value.get("header")
                if (
                    candidate
                    and isinstance(candidate[0], list)
                    and isinstance(header, list)
                ):
                    records = [
                        {
                            str(column): row[index] if index < len(row) else None
                            for index, column in enumerate(header)
                        }
                        for row in candidate
                    ]
                else:
                    records = candidate
                break
        if records is None and all(isinstance(item, dict) for item in value.values()):
            records = list(value.values())
        if records is None:
            raise QaError(
                "Manifest does not contain a supported record array ({0})".format(
                    ", ".join(preferred_keys)
                )
            )
    else:
        raise QaError("Manifest root must be an object or array")
    if not all(isinstance(record, dict) for record in records):
        raise QaError("Manifest record arrays must contain objects only")
    return list(records)


def normalized_mapping(record: Mapping[str, Any]) -> Dict[str, Any]:
    """Expose keys in lowercase snake case without discarding original values."""
    normalized: Dict[str, Any] = {}
    for key, value in record.items():
        canonical = (
            str(key)
            .strip()
            .lower()
            .replace("/", " ")
            .replace("-", " ")
            .replace("&", " and ")
        )
        canonical = "_".join(canonical.split())
        normalized[canonical] = value
    return normalized


def pick(record: Mapping[str, Any], *names: str, default: Any = None) -> Any:
    normalized = normalized_mapping(record)
    for name in names:
        key = "_".join(name.strip().lower().replace("-", " ").split())
        value = normalized.get(key)
        if value not in (None, ""):
            return value
    return default


def ensure_unique(records: Iterable[Mapping[str, Any]], fields: Sequence[str]) -> None:
    seen = set()
    for record in records:
        key = tuple(record.get(field) for field in fields)
        if key in seen:
            raise QaError(
                "Duplicate composite identity for {0}: {1}".format(
                    " + ".join(fields), key
                )
            )
        seen.add(key)


def parse_sha256_manifest(path: Path) -> Dict[str, str]:
    entries: Dict[str, str] = {}
    for number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 1)
        if len(parts) != 2 or len(parts[0]) != 64:
            raise QaError(
                "Malformed SHA-256 manifest line {0} in {1}".format(number, path)
            )
        relative = parts[1].lstrip("*")
        if relative in entries:
            raise QaError("Duplicate checksum entry: {0}".format(relative))
        entries[relative] = parts[0].lower()
    return entries


def confined_manifest_artifact(root: Path, relative: str) -> Path:
    """Resolve one manifest entry without absolute, traversal, or symlink escape."""
    supplied = Path(relative)
    if supplied.is_absolute() or ".." in supplied.parts or not supplied.parts:
        raise QaError("Unsafe checksum entry: {0}".format(relative))
    base = root.resolve()
    candidate = base
    for part in supplied.parts:
        candidate = candidate / part
        if candidate.is_symlink():
            raise QaError("Checksum entry contains a symlink: {0}".format(relative))
    try:
        candidate.resolve().relative_to(base)
    except ValueError as exc:
        raise QaError("Checksum entry escapes evidence root: {0}".format(relative)) from exc
    return candidate


def verify_sha256_manifest(path: Path, root: Optional[Path] = None) -> List[str]:
    base = root or path.parent
    errors: List[str] = []
    for relative, expected in parse_sha256_manifest(path).items():
        try:
            artifact = confined_manifest_artifact(base, relative)
        except QaError as exc:
            errors.append(str(exc))
            continue
        if relative.startswith("WITHHELD/"):
            continue
        if not artifact.is_file():
            errors.append("missing checksum target: {0}".format(relative))
        elif sha256_file(artifact) != expected:
            errors.append("checksum mismatch: {0}".format(relative))
    return errors


def load_openpyxl() -> Any:
    try:
        import openpyxl  # type: ignore
    except ImportError as exc:
        raise QaError(
            "openpyxl is required for workbook operations. Install the pinned "
            "QA dependency with: python3 -m pip install -r requirements-qa.txt"
        ) from exc
    return openpyxl
