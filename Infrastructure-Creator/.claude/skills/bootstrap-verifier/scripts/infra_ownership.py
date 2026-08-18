#!/usr/bin/env python3
"""Dependency-free ownership primitives for generated accelerators.

Manifest membership is the sole source of generator ownership. The helpers in
this module never discover ownership by walking a target's managed roots.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import time
from pathlib import Path
from typing import Iterable


MANIFEST_NAME = ".infra-manifest.json"
VALID_MODES = ("full", "merge")
VALID_DECISIONS = ("kept", "merged")
VALID_DECISION_ORIGINS = ("generated", "preexisting-team", "shared")
VALID_DECISION_STRATEGIES = ("keep", "append-requirements", "manual-merge")
STRUCTURED_DECISION_FIELDS = {
    "origin",
    "strategy",
    "proposal_sha256",
    "resolved_sha256",
    "requirements",
}
STATE_EXCLUDES = (
    "memory-bank/chunks/",
    "memory-bank/INDEX.md",
    "memory-bank/.memory-counter",
    "memory-bank/local/",
    "project-brain/indexes/",
    "project-brain/local/",
    "project-brain/archive/",
    "project-brain/control/",
    "project-brain/dynamic/",
)


class OwnershipError(ValueError):
    """Raised when ownership cannot be established safely."""


def resolve_target(value: str | Path, cwd: Path | None = None) -> Path:
    """Resolve a target after rejecting symlinks in the caller-supplied path."""
    path = Path(value).expanduser()
    if not path.is_absolute():
        path = (cwd or Path.cwd()) / path
    absolute = Path(os.path.abspath(str(path)))
    current = Path(absolute.anchor)
    for part in absolute.parts[1:]:
        current = current / part
        if current.is_symlink():
            raise OwnershipError(f"target path contains a symlink: {value}")
    return absolute.resolve()


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def normalize_relative_path(value: str | Path) -> str:
    """Return a safe POSIX target-relative path."""
    raw = Path(value)
    if raw.is_absolute() or ".." in raw.parts:
        raise OwnershipError(f"write-plan path must be target-relative: {value}")
    rel = raw.as_posix()
    while rel.startswith("./"):
        rel = rel[2:]
    if not rel or rel == ".":
        raise OwnershipError("write-plan path must not be empty")
    return rel


def confined_target_path(target: Path, value: str | Path) -> Path:
    """Return a target path only when no component can escape through a symlink."""
    root = resolve_target(target)
    rel = normalize_relative_path(value)
    candidate = root
    for part in Path(rel).parts:
        candidate = candidate / part
        if candidate.is_symlink():
            raise OwnershipError(f"target path contains a symlink: {rel}")
    try:
        candidate.resolve().relative_to(root)
    except ValueError as error:
        raise OwnershipError(f"target path escapes target root: {rel}") from error
    return candidate


def may_be_manifest_owned(rel: str) -> bool:
    if rel == MANIFEST_NAME or "__pycache__" in rel or rel.endswith(".pyc"):
        return False
    return not any(
        rel == excluded.rstrip("/") or rel.startswith(excluded)
        for excluded in STATE_EXCLUDES
    )


def read_write_plan(path: Path) -> list[str]:
    """Read one target-relative generated path per line."""
    paths: list[str] = []
    seen: set[str] = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        rel = normalize_relative_path(stripped)
        if rel not in seen:
            paths.append(rel)
            seen.add(rel)
    if not paths:
        raise OwnershipError(f"write plan is empty: {path}")
    return paths


def hash_write_plan(target: Path, write_plan: Iterable[str]) -> dict[str, str]:
    """Hash only explicit generated paths; never walk a managed root."""
    files: dict[str, str] = {}
    for value in write_plan:
        rel = normalize_relative_path(value)
        if not may_be_manifest_owned(rel):
            raise OwnershipError(f"write plan contains non-ownable runtime state: {rel}")
        path = confined_target_path(target, rel)
        if not path.is_file() or path.is_symlink():
            raise OwnershipError(f"write-plan file missing or not regular: {rel}")
        files[rel] = sha256_file(path)
    return dict(sorted(files.items()))


def hash_write_plan_sources(
    staging: Path,
    write_plan: Iterable[str],
    source_target: Path,
    source_map: dict[str, str],
) -> dict[str, str]:
    """Hash each explicit member from its declared final-content source."""
    plan = [normalize_relative_path(value) for value in write_plan]
    unknown = set(source_map) - set(plan)
    if unknown:
        raise OwnershipError(
            f"source map contains paths outside write plan: {sorted(unknown)}"
        )
    files: dict[str, str] = {}
    for rel in plan:
        if not may_be_manifest_owned(rel):
            raise OwnershipError(f"write plan contains non-ownable runtime state: {rel}")
        source = source_map.get(rel, "staging")
        if source not in {"staging", "target"}:
            raise OwnershipError(f"invalid final source for {rel}: {source!r}")
        root = source_target if source == "target" else staging
        path = confined_target_path(root, rel)
        if not path.is_file() or path.is_symlink():
            raise OwnershipError(
                f"{source} final-content source missing or not regular: {rel}"
            )
        files[rel] = sha256_file(path)
    return dict(sorted(files.items()))


def validate_decisions(decisions: dict, files: dict[str, str]) -> None:
    """Validate legacy and additive structured decision entries.

    Unknown fields are deliberately ignored so manifest v1 remains extensible.
    The presence of any structured field opts an entry into the complete
    structured contract; entries containing only the original three fields
    retain their historical behavior.
    """
    if not isinstance(decisions, dict):
        raise OwnershipError("decisions must be an object")
    if not set(decisions).issubset(files):
        raise OwnershipError("decisions contain paths outside manifest membership")
    for rel, entry in decisions.items():
        if not isinstance(entry, dict):
            raise OwnershipError(f"decision entry must be an object: {rel}")
        decision = entry.get("decision")
        if decision not in VALID_DECISIONS:
            raise OwnershipError(f"decision must be 'kept' or 'merged': {rel}")
        if not re.fullmatch(r"TASK-\d+", str(entry.get("task", ""))):
            raise OwnershipError(f"decision task must be TASK-<number>: {rel}")
        rejected = str(entry.get("rejected_sha256", ""))
        if not re.fullmatch(r"[0-9a-f]{64}", rejected):
            raise OwnershipError(
                f"decision rejected_sha256 must be a SHA-256 digest: {rel}"
            )

        if not (STRUCTURED_DECISION_FIELDS & set(entry)):
            continue
        missing = STRUCTURED_DECISION_FIELDS - set(entry)
        if missing:
            raise OwnershipError(
                f"structured decision is missing {sorted(missing)}: {rel}"
            )
        origin = entry.get("origin")
        strategy = entry.get("strategy")
        proposal = str(entry.get("proposal_sha256", ""))
        resolved = str(entry.get("resolved_sha256", ""))
        requirements = entry.get("requirements")
        if origin not in VALID_DECISION_ORIGINS:
            raise OwnershipError(f"invalid decision origin for {rel}: {origin!r}")
        if strategy not in VALID_DECISION_STRATEGIES:
            raise OwnershipError(f"invalid decision strategy for {rel}: {strategy!r}")
        if not re.fullmatch(r"[0-9a-f]{64}", proposal):
            raise OwnershipError(f"decision proposal_sha256 must be a digest: {rel}")
        if proposal != rejected:
            raise OwnershipError(
                f"proposal_sha256 must equal legacy rejected_sha256: {rel}"
            )
        if not re.fullmatch(r"[0-9a-f]{64}", resolved):
            raise OwnershipError(f"decision resolved_sha256 must be a digest: {rel}")
        if resolved != files[rel]:
            raise OwnershipError(f"resolved_sha256 must equal files[{rel!r}]")
        if (
            not isinstance(requirements, list)
            or any(not isinstance(item, str) or not item for item in requirements)
            or requirements != sorted(set(requirements))
        ):
            raise OwnershipError(
                f"decision requirements must be sorted unique strings: {rel}"
            )
        if strategy == "keep" and decision != "kept":
            raise OwnershipError(f"keep strategy requires decision 'kept': {rel}")
        if strategy != "keep" and decision != "merged":
            raise OwnershipError(
                f"{strategy} strategy requires decision 'merged': {rel}"
            )
        if strategy == "append-requirements":
            if origin != "shared":
                raise OwnershipError(
                    f"append-requirements requires shared origin: {rel}"
                )
            if not requirements:
                raise OwnershipError(
                    f"append-requirements requires at least one requirement: {rel}"
                )
        elif origin == "shared":
            if strategy != "keep" or not requirements:
                raise OwnershipError(
                    f"shared origin requires keep/append strategy and requirements: {rel}"
                )
        elif requirements:
            raise OwnershipError(
                f"requirements are only valid with shared origin: {rel}"
            )


def build_manifest(
    target: Path,
    write_plan: Iterable[str],
    *,
    generator: str,
    generator_version: str,
    task: str,
    profile: str,
    editions: Iterable[str],
    mode: str,
    decisions: dict | None = None,
    file_hashes: dict[str, str] | None = None,
    generated_at: str | None = None,
) -> dict:
    """Build a manifest whose membership is exactly the explicit write plan."""
    if mode not in VALID_MODES:
        raise OwnershipError(f"mode must be one of {VALID_MODES}, got {mode!r}")
    if not re.fullmatch(r"\d+\.\d+\.\d+", generator_version):
        raise OwnershipError("generator_version must be semver")
    if not re.fullmatch(r"TASK-\d+", task):
        raise OwnershipError("task must be a TASK-<number> id")
    normalized_plan = [normalize_relative_path(value) for value in write_plan]
    if file_hashes is not None:
        if set(file_hashes) != set(normalized_plan) or any(
            not isinstance(value, str)
            or not re.fullmatch(r"[0-9a-f]{64}", value)
            for value in file_hashes.values()
        ):
            raise OwnershipError("file_hashes must exactly match the write plan")
        files = dict(sorted(file_hashes.items()))
    else:
        files = hash_write_plan(target, normalized_plan)
    if decisions:
        validate_decisions(decisions, files)
    manifest = {
        "manifest_version": 1,
        "generator": generator,
        "generator_version": generator_version,
        "task": task,
        "profile": profile,
        "generated_at": generated_at
        or time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "editions": list(editions),
        "mode": mode,
        "files": files,
    }
    if decisions:
        manifest["decisions"] = decisions
    return manifest


def load_json_object(path: Path, label: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise OwnershipError(f"{label} unreadable or invalid: {error}") from error
    if not isinstance(value, dict):
        raise OwnershipError(f"{label} must contain a JSON object")
    return value


def write_manifest(target: Path, manifest: dict) -> Path:
    path = confined_target_path(target, MANIFEST_NAME)
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return path


def load_manifest(target: Path) -> dict:
    """Load an existing manifest or refuse a legacy target without writing."""
    path = confined_target_path(target, MANIFEST_NAME)
    if not path.is_file():
        raise OwnershipError(
            f"{MANIFEST_NAME} missing; legacy target ownership is unknown, refusing update"
        )
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise OwnershipError(f"{MANIFEST_NAME} unreadable or invalid: {error}") from error
    if not isinstance(manifest, dict):
        raise OwnershipError(f"{MANIFEST_NAME} must contain a JSON object")
    return manifest


def staged_files(staging: Path) -> dict[str, Path]:
    """Inventory staging output; staging is generator-owned by construction."""
    return {
        path.relative_to(staging).as_posix(): path
        for path in sorted(staging.rglob("*"))
        if path.is_file() and not path.is_symlink()
    }


def classify_update(target: Path, staging: Path, manifest: dict) -> list[dict]:
    """Classify manifest and staged paths without walking target roots."""
    files = manifest.get("files")
    if not isinstance(files, dict):
        raise OwnershipError(f"{MANIFEST_NAME} files must be an object")
    decisions = manifest.get("decisions", {})
    if not isinstance(decisions, dict):
        raise OwnershipError(f"{MANIFEST_NAME} decisions must be an object")
    for rel, entry in decisions.items():
        if not isinstance(entry, dict):
            raise OwnershipError(
                f"{MANIFEST_NAME} decision entry must be an object: {rel}"
            )
    staged = staged_files(staging)
    # The refreshed manifest is built inside staging before publication; it can
    # never be a member of its own files map, so it must not surface as a row.
    staged.pop(MANIFEST_NAME, None)
    results: list[dict] = []

    for rel in sorted(set(files) | set(staged)):
        target_path = confined_target_path(target, rel)
        staged_path = staged.get(rel)
        expected = files.get(rel)
        decision = decisions.get(rel)

        if expected is not None:
            if staged_path is None:
                if not target_path.is_file():
                    classification, reason = "requires-decision", "tracked-file-deleted"
                elif sha256_file(target_path) == expected:
                    classification, reason = "obsolete", "no-longer-generated"
                else:
                    classification, reason = "requires-decision", "tracked-file-modified"
            elif not target_path.is_file():
                classification, reason = "requires-decision", "tracked-file-deleted"
            elif sha256_file(target_path) != expected:
                classification, reason = "requires-decision", "tracked-file-modified"
            elif decision is not None:
                staged_sha = sha256_file(staged_path)
                proposal_sha = decision.get(
                    "proposal_sha256", decision.get("rejected_sha256")
                )
                if staged_sha == proposal_sha:
                    classification, reason = (
                        "standing-decision-honored",
                        decision.get("strategy", decision.get("decision", "kept")),
                    )
                else:
                    classification, reason = (
                        "requires-decision",
                        "generator-output-changed",
                    )
            else:
                classification, reason = "safe-update", "manifest-hash-unchanged"
        elif target_path.exists():
            classification, reason = "requires-decision", "new-file-collision"
        else:
            classification, reason = "new-file", "new-generated-path"

        results.append(
            {
                "path": rel,
                "classification": classification,
                "reason": reason,
            }
        )
    return results


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    subparsers = parser.add_subparsers(dest="command", required=True)

    manifest_parser = subparsers.add_parser("manifest")
    manifest_parser.add_argument("--target", required=True)
    manifest_parser.add_argument("--write-plan", required=True)
    manifest_parser.add_argument("--generator", default="Infrastructure-Creator")
    manifest_parser.add_argument("--version", required=True)
    manifest_parser.add_argument("--task", required=True)
    manifest_parser.add_argument("--profile", required=True)
    manifest_parser.add_argument("--editions", required=True)
    manifest_parser.add_argument("--mode", choices=VALID_MODES, required=True)
    manifest_parser.add_argument(
        "--decisions",
        help="JSON object of final keep/merge decision entries",
    )
    manifest_parser.add_argument(
        "--source-target",
        help="target root supplying explicitly retained final file content",
    )
    manifest_parser.add_argument(
        "--source-map",
        help='JSON map of write-plan path to "staging" or "target"',
    )

    classify_parser = subparsers.add_parser("classify")
    classify_parser.add_argument("--target", required=True)
    classify_parser.add_argument("--staging", required=True)

    args = parser.parse_args(argv)
    try:
        target = resolve_target(args.target)
        if not target.is_dir():
            raise OwnershipError(f"target not found: {target}")
        if args.command == "manifest":
            write_plan_path = resolve_target(args.write_plan)
            write_plan = read_write_plan(write_plan_path)
            decisions = (
                load_json_object(resolve_target(args.decisions), "decisions")
                if args.decisions
                else None
            )
            if bool(args.source_target) != bool(args.source_map):
                raise OwnershipError(
                    "--source-target and --source-map must be supplied together"
                )
            file_hashes = None
            if args.source_map:
                source_target = resolve_target(args.source_target)
                if not source_target.is_dir():
                    raise OwnershipError(f"source target not found: {source_target}")
                source_map = load_json_object(
                    resolve_target(args.source_map), "source map"
                )
                for rel, source in source_map.items():
                    decision = (decisions or {}).get(rel)
                    if source == "target" and (
                        not isinstance(decision, dict)
                        or decision.get("decision") != "kept"
                    ):
                        raise OwnershipError(
                            f"target final-content source requires a kept decision: {rel}"
                        )
                for rel, decision in (decisions or {}).items():
                    if not isinstance(decision, dict):
                        raise OwnershipError(
                            f"decision entry must be an object: {rel}"
                        )
                    if decision.get("decision") == "kept" and source_map.get(rel) != "target":
                        raise OwnershipError(
                            f"kept decision must hash the final target bytes: {rel}"
                        )
                    if decision.get("decision") == "merged" and source_map.get(
                        rel, "staging"
                    ) != "staging":
                        raise OwnershipError(
                            f"merged decision must hash staged merged bytes: {rel}"
                        )
                file_hashes = hash_write_plan_sources(
                    target,
                    write_plan,
                    source_target,
                    source_map,
                )
            manifest = build_manifest(
                target,
                write_plan,
                generator=args.generator,
                generator_version=args.version,
                task=args.task,
                profile=args.profile,
                editions=(item.strip() for item in args.editions.split(",") if item.strip()),
                mode=args.mode,
                decisions=decisions,
                file_hashes=file_hashes,
            )
            path = write_manifest(target, manifest)
            print(f"wrote {path} ({len(manifest['files'])} files)")
        else:
            manifest = load_manifest(target)
            staging = resolve_target(args.staging)
            if not staging.is_dir():
                raise OwnershipError(f"staging directory not found: {staging}")
            print(json.dumps(classify_update(target, staging, manifest), indent=2))
    except (OSError, OwnershipError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
