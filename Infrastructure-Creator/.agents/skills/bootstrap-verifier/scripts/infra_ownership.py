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
    generated_at: str | None = None,
) -> dict:
    """Build a manifest whose membership is exactly the explicit write plan."""
    if mode not in VALID_MODES:
        raise OwnershipError(f"mode must be one of {VALID_MODES}, got {mode!r}")
    if not re.fullmatch(r"\d+\.\d+\.\d+", generator_version):
        raise OwnershipError("generator_version must be semver")
    if not re.fullmatch(r"TASK-\d+", task):
        raise OwnershipError("task must be a TASK-<number> id")
    files = hash_write_plan(target, write_plan)
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
    staged = staged_files(staging)
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
                if staged_sha == decision.get("rejected_sha256"):
                    classification, reason = (
                        "standing-decision-honored",
                        decision.get("decision", "kept"),
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
            manifest = build_manifest(
                target,
                read_write_plan(write_plan_path),
                generator=args.generator,
                generator_version=args.version,
                task=args.task,
                profile=args.profile,
                editions=(item.strip() for item in args.editions.split(",") if item.strip()),
                mode=args.mode,
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
