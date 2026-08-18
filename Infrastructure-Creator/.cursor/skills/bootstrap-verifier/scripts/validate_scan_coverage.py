#!/usr/bin/env python3
"""Validate that discovery accounted for every surface of the target.

A scan that misses a module is indistinguishable from a scan that covered it,
because the only thing either produces is a list of what was found. The
generated accelerator then looks complete while carrying nothing about the part
nobody looked at, and the omission surfaces months later as a skill that does
not know a subsystem exists.

So each scanner writes, beside its report and its ledger, a coverage artifact
that gives every surface it saw one of four dispositions - `covered`,
`excluded`, `truncated`, `not-permitted` - and this validator holds the
artifacts against the target and against each other.

Invariants shared with the other gates in this directory: standard library
only, no execution, no network, byte-stable JSON output, and fail-closed - a
surface this validator cannot resolve is reported, not assumed clean. It never
opens a file it is checking the disposition of, so the secrets rule is enforced
without ever reading a secret.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

DISPOSITIONS = {"covered", "excluded", "truncated", "not-permitted"}
# Dispositions under which the scanner did look at some of the surface, so
# evidence from it is expected rather than contradictory. A truncated scan
# stopped partway; what it read before stopping is still evidence.
SCANNED_DISPOSITIONS = {"covered", "truncated"}
COVERAGE_FIELDS = {"surface", "kind", "disposition", "reason", "evidence_ids"}
SURFACE_KINDS = {"file", "tree"}

# VCS internals are the one thing exempt from an explicit disposition: they are
# not the target's own code, and the secrets rule forbids reading them anyway.
# Everything else - `vendor/`, `node_modules/`, `var/` - costs one honest line
# saying it was deliberately left out, which is the whole point of the artifact.
BUILTIN_EXEMPT = (".git",)

# Surfaces whose contents may never be read. A disposition of `covered` on any
# of these is a claim to have done what the contract forbids.
SECRET_SURFACE_PATTERNS = (
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.jks",
    "id_rsa",
    "id_ed25519",
    "*.keystore",
    "auth.json",
)

# Directories whose immediate children are separately meaningful surfaces: a
# module, a test family, an integration. Listing `src/` as covered without
# saying anything about `src/Legacy` is the omission this catches.
MODULE_CONTAINERS = (
    "src",
    "app",
    "apps",
    "lib",
    "libs",
    "modules",
    "packages",
    "bundles",
    "tests",
    "test",
    "config",
)
SURFACE_BUDGET = 4000


@dataclass(frozen=True)
class Diagnostic:
    severity: str
    code: str
    message: str


def _diag(
    diagnostics: list[Diagnostic], code: str, message: str, severity: str = "error"
) -> None:
    diagnostics.append(Diagnostic(severity, code, message))


def _is_nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _is_string_list(value: Any, *, allow_empty: bool = True) -> bool:
    if not isinstance(value, list) or (not allow_empty and not value):
        return False
    return all(_is_nonempty_string(item) for item in value)


def _matches(surface: str, kind: str, path: str) -> bool:
    """Whether one declared surface accounts for one target path."""
    surface = surface.strip().rstrip("/")
    if surface in {"**", "*"}:
        return True
    if surface.endswith("/**"):
        prefix = surface[:-3]
        return path == prefix or path.startswith(prefix + "/")
    if kind == "tree":
        return path == surface or path.startswith(surface + "/")
    if any(char in surface for char in "*?["):
        return fnmatch.fnmatch(path, surface)
    return path == surface


def _is_secret_surface(surface: str) -> bool:
    name = surface.strip().rstrip("/").split("/")[-1]
    return any(fnmatch.fnmatch(name, pattern) for pattern in SECRET_SURFACE_PATTERNS)


def target_surfaces(target: Path, diagnostics: list[Diagnostic]) -> list[str]:
    """Every surface of the target that requires a disposition.

    Depth one throughout, plus the immediate children of the directories that
    hold modules, test families, and configuration - the granularity at which
    "the scan missed a subsystem" is a statement someone can act on.
    """
    surfaces: list[str] = []
    try:
        entries = sorted(target.iterdir(), key=lambda item: item.name)
    except OSError as error:
        _diag(diagnostics, "SCAN_TARGET_UNREADABLE", f"target is unreadable: {error}")
        return []
    for entry in entries:
        if entry.name in BUILTIN_EXEMPT or entry.is_symlink():
            continue
        surfaces.append(entry.name)
        if entry.is_dir() and entry.name in MODULE_CONTAINERS:
            try:
                children = sorted(entry.iterdir(), key=lambda item: item.name)
            except OSError as error:
                _diag(
                    diagnostics,
                    "SCAN_TARGET_UNREADABLE",
                    f"{entry.name} is unreadable: {error}",
                )
                continue
            for child in children:
                if child.is_symlink():
                    continue
                surfaces.append(f"{entry.name}/{child.name}")
        if len(surfaces) > SURFACE_BUDGET:
            _diag(
                diagnostics,
                "SCAN_TARGET_TOO_LARGE",
                f"target exposes more than {SURFACE_BUDGET} surfaces; discovery "
                "coverage cannot be established",
            )
            return []
    return surfaces


def _load(path: Path, diagnostics: list[Diagnostic]) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        _diag(
            diagnostics,
            "SCAN_ARTIFACT_UNREADABLE",
            f"{path.name} is not readable JSON: {error}",
        )
        return None
    if not isinstance(value, dict):
        _diag(
            diagnostics,
            "SCAN_ARTIFACT_UNREADABLE",
            f"{path.name} root must be an object",
        )
        return None
    return value


def _read_coverage(
    path: Path, diagnostics: list[Diagnostic]
) -> tuple[str, list[dict[str, Any]]]:
    document = _load(path, diagnostics)
    if document is None:
        return "", []
    scanner = document.get("scanner")
    if not _is_nonempty_string(scanner):
        _diag(
            diagnostics,
            "SCAN_COVERAGE_INVALID",
            f"{path.name} must name the scanner that produced it",
        )
        return "", []
    scanner = scanner.strip()
    if set(document) != {"scanner", "target_root", "surfaces"}:
        _diag(
            diagnostics,
            "SCAN_COVERAGE_INVALID",
            f"{scanner} coverage must define exactly scanner, target_root, "
            "and surfaces",
        )
        return scanner, []
    entries = document.get("surfaces")
    if not isinstance(entries, list) or not entries:
        _diag(
            diagnostics,
            "SCAN_COVERAGE_INVALID",
            f"{scanner} coverage must declare a non-empty surfaces array",
        )
        return scanner, []
    valid: list[dict[str, Any]] = []
    for index, entry in enumerate(entries):
        if (
            not isinstance(entry, dict)
            or set(entry) != COVERAGE_FIELDS
            or not _is_nonempty_string(entry.get("surface"))
            or entry.get("kind") not in SURFACE_KINDS
            or entry.get("disposition") not in DISPOSITIONS
            or not _is_nonempty_string(entry.get("reason"))
            or not _is_string_list(entry.get("evidence_ids"))
        ):
            _diag(
                diagnostics,
                "SCAN_COVERAGE_INVALID",
                f"{scanner} surfaces[{index}] must declare surface, kind, "
                "disposition, reason, and evidence_ids",
            )
            continue
        surface = str(entry["surface"]).strip()
        if surface.startswith("/") or ".." in Path(surface).parts:
            _diag(
                diagnostics,
                "SCAN_SURFACE_UNSAFE",
                f"{scanner} declares a surface outside the target: {surface}",
            )
            continue
        valid.append(entry)
    return scanner, valid


def _ledger_paths(path: Path, diagnostics: list[Diagnostic]) -> dict[str, str]:
    """Evidence id to cited path, for the ledger beside a coverage artifact."""
    document = _load(path, diagnostics)
    if document is None:
        return {}
    cited: dict[str, str] = {}
    for entry in document.get("evidence") or []:
        if not isinstance(entry, dict) or not _is_nonempty_string(entry.get("id")):
            continue
        # A URL entry cites external authority and an absence entry cites a
        # search; neither occupies a surface of this target.
        if _is_nonempty_string(entry.get("path")):
            cited[entry["id"].strip()] = str(entry["path"]).strip()
    return cited


def validate(target: Path, task_dir: Path) -> list[Diagnostic]:
    diagnostics: list[Diagnostic] = []
    coverage_files = sorted(task_dir.glob("*-coverage.json"))
    if not coverage_files:
        _diag(
            diagnostics,
            "SCAN_COVERAGE_MISSING",
            f"no scanner coverage artifact in {task_dir.name}; discovery "
            "completeness cannot be established",
        )
        return diagnostics

    surfaces = target_surfaces(target, diagnostics)
    accounted: dict[str, list[tuple[str, str]]] = {name: [] for name in surfaces}
    seen_scanners: set[str] = set()

    for coverage_path in coverage_files:
        scanner, entries = _read_coverage(coverage_path, diagnostics)
        if not scanner:
            continue
        if scanner in seen_scanners:
            _diag(
                diagnostics,
                "SCAN_COVERAGE_DUPLICATE",
                f"more than one coverage artifact claims to be {scanner}",
            )
            continue
        seen_scanners.add(scanner)
        ledger_path = task_dir / f"{scanner}-evidence.json"
        if not ledger_path.is_file():
            _diag(
                diagnostics,
                "SCAN_LEDGER_MISSING",
                f"{scanner} declares coverage but wrote no evidence ledger",
            )
            cited: dict[str, str] = {}
        else:
            cited = _ledger_paths(ledger_path, diagnostics)

        scanned_surfaces = [
            entry
            for entry in entries
            if entry["disposition"] in SCANNED_DISPOSITIONS
        ]
        for entry in entries:
            surface = str(entry["surface"]).strip()
            disposition = str(entry["disposition"])
            if disposition == "covered" and _is_secret_surface(surface):
                _diag(
                    diagnostics,
                    "SCAN_SECRET_COVERED",
                    f"{scanner} claims to have covered a secret-bearing "
                    f"surface: {surface}. Existence may be recorded; contents "
                    "may not.",
                )
            if disposition == "covered":
                supporting = [
                    identifier
                    for identifier in entry["evidence_ids"]
                    if identifier in cited
                    and _matches(surface, str(entry["kind"]), cited[identifier])
                ]
                if not supporting:
                    _diag(
                        diagnostics,
                        "SCAN_COVERED_WITHOUT_EVIDENCE",
                        f"{scanner} reports {surface} covered but its ledger "
                        "carries no evidence inside it",
                    )
            elif disposition not in SCANNED_DISPOSITIONS and entry["evidence_ids"]:
                _diag(
                    diagnostics,
                    "SCAN_DISPOSITION_EVIDENCE_CONFLICT",
                    f"{scanner} marks {surface} {disposition} yet cites "
                    "evidence from it",
                )
            if disposition == "truncated":
                _diag(
                    diagnostics,
                    "SCAN_SURFACE_TRUNCATED",
                    f"{scanner} stopped short of {surface}: {entry['reason']}",
                    "warning",
                )
            for name in surfaces:
                if _matches(surface, str(entry["kind"]), name):
                    accounted[name].append((scanner, disposition))

        for identifier, cited_path in sorted(cited.items()):
            if not any(
                _matches(str(entry["surface"]).strip(), str(entry["kind"]), cited_path)
                for entry in scanned_surfaces
            ):
                _diag(
                    diagnostics,
                    "SCAN_EVIDENCE_OUTSIDE_COVERAGE",
                    f"{scanner} cites {identifier} from {cited_path}, which "
                    "none of the surfaces it says it read contains",
                )

    for name in surfaces:
        dispositions = accounted[name]
        if not dispositions:
            _diag(
                diagnostics,
                "SCAN_SURFACE_UNACCOUNTED",
                f"no scanner gave {name} a disposition; a surface nobody "
                "looked at is not the same as one nobody needed",
            )
            continue
        kinds = {disposition for _, disposition in dispositions}
        if "covered" in kinds and "not-permitted" in kinds:
            owners = ", ".join(
                f"{scanner}:{disposition}" for scanner, disposition in dispositions
            )
            _diag(
                diagnostics,
                "SCAN_DISPOSITION_CONFLICT",
                f"{name} is both covered and forbidden: {owners}",
            )
    return diagnostics


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--task-dir", required=True)
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    diagnostics = validate(Path(args.target).resolve(), Path(args.task_dir))
    errors = [item for item in diagnostics if item.severity == "error"]
    if args.as_json:
        print(
            json.dumps(
                {
                    "valid": not errors,
                    "diagnostics": [
                        {
                            "severity": item.severity,
                            "code": item.code,
                            "message": item.message,
                        }
                        for item in diagnostics
                    ],
                },
                indent=2,
                sort_keys=True,
            )
        )
    else:
        for item in diagnostics:
            stream = sys.stderr if item.severity == "error" else sys.stdout
            print(f"{item.severity.upper()}: {item.code}: {item.message}", file=stream)
        if not diagnostics:
            print("discovery coverage OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
