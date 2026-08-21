#!/usr/bin/env python3
"""Publish an explicitly planned staged accelerator with rollback support.

The helper never walks staging or target roots.  `snapshot` records only the
publication plan plus the manifest destination.  `publish` refuses target
drift, refuses plans the manifests cannot prove safe (publication paths must
be staged-manifest members, runtime state may only seed absent paths, and
removals must be target-manifest members), copies the staged manifest last,
and rolls back immediately on an I/O failure.  The caller keeps the journal
until post-publication bootstrap verification passes; `rollback` restores it
when that verification fails, removes directory chains the publication
created, and preserves (and reports) any file a third party edited after
publication instead of clobbering it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import stat
import sys
import tempfile
from pathlib import Path

from infra_ownership import (
    MANIFEST_NAME,
    OwnershipError,
    confined_target_path,
    may_be_manifest_owned,
    normalize_relative_path,
    read_write_plan,
    resolve_target,
    sha256_file,
)


class PublicationError(ValueError):
    """Raised when staged publication cannot be proven safe."""


def planned_paths(plan: Path) -> list[str]:
    paths = read_write_plan(plan)
    if MANIFEST_NAME not in paths:
        paths.append(MANIFEST_NAME)
    return paths


def removal_paths(plan: Path | None) -> list[str]:
    if plan is None:
        return []
    paths = read_write_plan(plan)
    if MANIFEST_NAME in paths:
        raise PublicationError("removal plan must not contain the manifest")
    return paths


def file_state(root: Path, rel: str, *, allow_final_symlink: bool = False) -> dict:
    if allow_final_symlink:
        root = resolve_target(root)
        rel = normalize_relative_path(rel)
        path = root / rel
        parent = root
        for part in Path(rel).parts[:-1]:
            parent = parent / part
            if parent.is_symlink():
                raise PublicationError(f"watched path has a symlink parent: {rel}")
        try:
            parent.resolve().relative_to(root)
        except ValueError as error:
            raise PublicationError(f"watched path escapes target root: {rel}") from error
        if path.is_symlink():
            link_target = os.readlink(path)
            return {
                "state": "symlink",
                "target": link_target,
                "sha256": hashlib.sha256(link_target.encode("utf-8")).hexdigest(),
            }
    else:
        path = confined_target_path(root, rel)
    if not path.exists():
        return {"state": "missing"}
    if path.is_symlink() or not path.is_file():
        raise PublicationError(f"planned path is not a regular file: {rel}")
    return {
        "state": "file",
        "sha256": sha256_file(path),
        "mode": stat.S_IMODE(path.stat().st_mode),
    }


def build_snapshot(
    target: Path, paths: list[str], baseline_only: list[str] | None = None
) -> dict:
    baseline_only = baseline_only or []
    overlap = set(paths) & set(baseline_only)
    if overlap:
        raise PublicationError(
            "publication/removal paths overlap baseline-only paths: "
            + ", ".join(sorted(overlap))
        )
    all_paths = paths + baseline_only
    return {
        "schema_version": 1,
        "target": str(target),
        "files": {
            rel: file_state(
                target, rel, allow_final_symlink=rel in set(baseline_only)
            )
            for rel in sorted(set(all_paths))
        },
        "baseline_only": sorted(set(baseline_only)),
    }


def load_json(path: Path, label: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise PublicationError(f"{label} unreadable or invalid: {error}") from error
    if not isinstance(value, dict):
        raise PublicationError(f"{label} must contain a JSON object")
    return value


def verify_baseline(
    target: Path,
    paths: list[str],
    snapshot: dict,
    baseline_only: list[str] | None = None,
) -> None:
    baseline_only = baseline_only or []
    if snapshot.get("target") != str(target):
        raise PublicationError("baseline target does not match requested target")
    expected = snapshot.get("files")
    recorded_baseline_only = snapshot.get("baseline_only", [])
    if (
        not isinstance(recorded_baseline_only, list)
        or set(recorded_baseline_only) != set(baseline_only)
    ):
        raise PublicationError("baseline-only membership does not match watch plan")
    all_paths = paths + baseline_only
    if not isinstance(expected, dict) or set(expected) != set(all_paths):
        raise PublicationError("baseline membership does not match publication plan")
    baseline_set = set(baseline_only)
    for rel in all_paths:
        if file_state(
            target, rel, allow_final_symlink=rel in baseline_set
        ) != expected[rel]:
            raise PublicationError(f"target changed after collision review: {rel}")


def verify_staging(staging: Path, paths: list[str]) -> None:
    for rel in paths:
        path = confined_target_path(staging, rel)
        if not path.is_file() or path.is_symlink():
            raise PublicationError(f"staged publication file missing or unsafe: {rel}")


def manifest_members(root: Path, label: str) -> set[str]:
    path = confined_target_path(root, MANIFEST_NAME)
    if not path.is_file() or path.is_symlink():
        raise PublicationError(f"{label} manifest missing: {MANIFEST_NAME}")
    manifest = load_json(path, f"{label} manifest")
    files = manifest.get("files")
    if not isinstance(files, dict):
        raise PublicationError(f"{label} manifest files must be an object")
    return {normalize_relative_path(rel) for rel in files}


def verify_plan_ownership(
    target: Path, staging: Path, paths: list[str], removals: list[str]
) -> None:
    """Refuse plans the manifests cannot prove safe, before any target write.

    Every manifest-ownable publication path must be a member of the staged
    manifest's files map.  Non-ownable runtime state (memory-bank/project-brain
    live data) belongs to the target team from the moment it is seeded, so it
    may only be published into a path that does not exist yet.  Cache artifacts
    are never publishable.  Every removal must be owned by the target manifest.
    """
    staged_members = manifest_members(staging, "staged")
    caches: list[str] = []
    runtime_overwrites: list[str] = []
    unowned: list[str] = []
    for rel in paths:
        if rel == MANIFEST_NAME:
            continue
        if "__pycache__" in rel or rel.endswith(".pyc"):
            caches.append(rel)
        elif may_be_manifest_owned(rel):
            if rel not in staged_members:
                unowned.append(rel)
        elif confined_target_path(target, rel).exists():
            runtime_overwrites.append(rel)
    if caches:
        raise PublicationError(
            "publication plan contains cache artifacts: " + ", ".join(sorted(caches))
        )
    if runtime_overwrites:
        raise PublicationError(
            "publication plan would overwrite non-ownable runtime state: "
            + ", ".join(sorted(runtime_overwrites))
        )
    if unowned:
        raise PublicationError(
            "publication plan paths are missing from the staged manifest: "
            + ", ".join(sorted(unowned))
        )
    if removals:
        target_members = manifest_members(target, "target")
        unmanifested = sorted(
            rel
            for rel in removals
            if not may_be_manifest_owned(rel) or rel not in target_members
        )
        if unmanifested:
            raise PublicationError(
                "removal plan paths are not owned by the target manifest: "
                + ", ".join(unmanifested)
            )


def prepare_journal(
    target: Path, paths: list[str], journal: Path, snapshot: dict
) -> dict:
    journal = journal.resolve()
    if journal.exists():
        raise PublicationError(f"rollback journal already exists: {journal}")
    backups = journal / "backups"
    backups.mkdir(parents=True)
    for rel in paths:
        state = snapshot["files"][rel]
        if state["state"] != "file":
            continue
        source = confined_target_path(target, rel)
        backup = confined_target_path(backups, rel)
        backup.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, backup)
    created_dirs: set[str] = set()
    for rel in paths:
        parent = Path(rel).parent
        while parent.as_posix() != ".":
            ancestor = parent.as_posix()
            if not confined_target_path(target, ancestor).exists():
                created_dirs.add(ancestor)
            parent = parent.parent
    metadata = {
        "schema_version": 1,
        "target": str(target),
        "paths": paths,
        "baseline": {rel: snapshot["files"][rel] for rel in paths},
        "created_dirs": sorted(created_dirs),
        "status": "prepared",
    }
    (journal / "journal.json").write_text(
        json.dumps(metadata, indent=2) + "\n", encoding="utf-8"
    )
    return metadata


def atomic_copy(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(
        prefix=f".{destination.name}.infra-", dir=str(destination.parent)
    )
    os.close(fd)
    temporary_path = Path(temporary)
    try:
        shutil.copy2(source, temporary_path)
        os.replace(temporary_path, destination)
    finally:
        temporary_path.unlink(missing_ok=True)


def restore(target: Path, journal: Path, metadata: dict | None = None) -> None:
    metadata = metadata or load_json(journal / "journal.json", "rollback journal")
    if metadata.get("target") != str(target):
        raise PublicationError("rollback journal target mismatch")
    baseline = metadata.get("baseline")
    paths = metadata.get("paths")
    created_dirs = metadata.get("created_dirs", [])
    published = metadata.get("published")
    if (
        not isinstance(baseline, dict)
        or not isinstance(paths, list)
        or not isinstance(created_dirs, list)
        or not (published is None or isinstance(published, dict))
    ):
        raise PublicationError("rollback journal is malformed")
    conflicts: set[str] = set()
    for rel in reversed(paths):
        rel = normalize_relative_path(rel)
        destination = confined_target_path(target, rel)
        state = baseline.get(rel)
        if not isinstance(state, dict):
            raise PublicationError(f"rollback state missing: {rel}")
        if published is not None:
            # A completed publication recorded the exact content it wrote.
            # Content matching neither that record nor the baseline is a
            # third-party edit made after publication: never clobber it.
            expected = published.get(rel)
            if not isinstance(expected, dict):
                raise PublicationError(f"rollback published state missing: {rel}")
            try:
                current = file_state(target, rel)
            except (OwnershipError, PublicationError):
                conflicts.add(rel)
                continue
            signature = (current.get("state"), current.get("sha256"))
            if signature not in (
                (expected.get("state"), expected.get("sha256")),
                (state.get("state"), state.get("sha256")),
            ):
                conflicts.add(rel)
                continue
        if state.get("state") == "missing":
            if destination.exists():
                if destination.is_symlink() or not destination.is_file():
                    raise PublicationError(f"cannot remove unsafe rollback path: {rel}")
                destination.unlink()
        else:
            backup = confined_target_path(journal / "backups", rel)
            if not backup.is_file() or backup.is_symlink():
                raise PublicationError(f"rollback backup missing or unsafe: {rel}")
            atomic_copy(backup, destination)
            os.chmod(destination, int(state["mode"]))
    for rel_dir in sorted(created_dirs, reverse=True):
        directory = confined_target_path(target, rel_dir)
        if (
            directory.is_dir()
            and not directory.is_symlink()
            and next(directory.iterdir(), None) is None
        ):
            directory.rmdir()
    if conflicts:
        metadata["status"] = "rolled-back-with-conflicts"
        metadata["conflicts"] = sorted(conflicts)
    else:
        metadata["status"] = "rolled-back"
        metadata.pop("conflicts", None)
    (journal / "journal.json").write_text(
        json.dumps(metadata, indent=2) + "\n", encoding="utf-8"
    )
    if conflicts:
        raise PublicationError(
            "rollback preserved third-party edits made after publication; "
            "resolve manually: " + ", ".join(sorted(conflicts))
        )


def publish(
    target: Path,
    staging: Path,
    paths: list[str],
    snapshot: dict,
    journal: Path,
    removals: list[str] | None = None,
    baseline_only: list[str] | None = None,
) -> None:
    removals = removals or []
    baseline_only = baseline_only or []
    overlap = set(paths) & set(removals)
    if overlap:
        raise PublicationError(
            f"paths cannot be both published and removed: {', '.join(sorted(overlap))}"
        )
    watched_overlap = set(baseline_only) & (set(paths) | set(removals))
    if watched_overlap:
        raise PublicationError(
            "baseline-only paths cannot be published or removed: "
            + ", ".join(sorted(watched_overlap))
        )
    all_paths = paths + removals
    verify_plan_ownership(target, staging, paths, removals)
    verify_baseline(target, all_paths, snapshot, baseline_only)
    verify_staging(staging, paths)
    metadata = prepare_journal(target, all_paths, journal, snapshot)
    ordered = [rel for rel in paths if rel != MANIFEST_NAME] + [MANIFEST_NAME]
    try:
        for rel in ordered[:-1]:
            atomic_copy(
                confined_target_path(staging, rel),
                confined_target_path(target, rel),
            )
        for rel in removals:
            destination = confined_target_path(target, rel)
            if destination.exists():
                if destination.is_symlink() or not destination.is_file():
                    raise PublicationError(f"cannot remove unsafe planned path: {rel}")
                destination.unlink()
        atomic_copy(
            confined_target_path(staging, MANIFEST_NAME),
            confined_target_path(target, MANIFEST_NAME),
        )
        published = {
            rel: {
                "state": "file",
                "sha256": sha256_file(confined_target_path(target, rel)),
            }
            for rel in ordered
        }
        for rel in removals:
            published[rel] = {"state": "missing"}
        metadata["published"] = dict(sorted(published.items()))
    except Exception:
        restore(target, journal, metadata)
        raise
    metadata["status"] = "published-pending-verification"
    (journal / "journal.json").write_text(
        json.dumps(metadata, indent=2) + "\n", encoding="utf-8"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    snapshot_parser = subparsers.add_parser("snapshot")
    snapshot_parser.add_argument("--target", required=True)
    snapshot_parser.add_argument("--publication-plan", required=True)
    snapshot_parser.add_argument("--removal-plan")
    snapshot_parser.add_argument(
        "--watch-plan",
        help="paths checked for drift but never published, removed, or journaled",
    )
    snapshot_parser.add_argument("--output", required=True)

    publish_parser = subparsers.add_parser("publish")
    publish_parser.add_argument("--target", required=True)
    publish_parser.add_argument("--staging", required=True)
    publish_parser.add_argument("--publication-plan", required=True)
    publish_parser.add_argument("--removal-plan")
    publish_parser.add_argument(
        "--watch-plan",
        help="paths checked for drift but never published, removed, or journaled",
    )
    publish_parser.add_argument("--baseline", required=True)
    publish_parser.add_argument("--journal", required=True)

    rollback_parser = subparsers.add_parser("rollback")
    rollback_parser.add_argument("--target", required=True)
    rollback_parser.add_argument("--journal", required=True)

    args = parser.parse_args(argv)
    try:
        target = resolve_target(args.target)
        if not target.is_dir():
            raise PublicationError(f"target directory not found: {target}")
        if args.command == "rollback":
            restore(target, resolve_target(args.journal))
            print("publication rolled back")
            return 0

        plan = resolve_target(args.publication_plan)
        paths = planned_paths(plan)
        removals = removal_paths(
            resolve_target(args.removal_plan)
            if getattr(args, "removal_plan", None)
            else None
        )
        baseline_only = (
            read_write_plan(resolve_target(args.watch_plan))
            if getattr(args, "watch_plan", None)
            else []
        )
        if set(paths) & set(removals):
            raise PublicationError("publication and removal plans overlap")
        watched_overlap = set(baseline_only) & (set(paths) | set(removals))
        if watched_overlap:
            raise PublicationError(
                "watch plan overlaps publication or removal plan: "
                + ", ".join(sorted(watched_overlap))
            )
        if args.command == "snapshot":
            output = Path(args.output).expanduser().resolve()
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(
                json.dumps(
                    build_snapshot(target, paths + removals, baseline_only), indent=2
                )
                + "\n",
                encoding="utf-8",
            )
            print(
                "snapshot recorded for "
                f"{len(paths) + len(removals)} publication/removal and "
                f"{len(baseline_only)} baseline-only path(s)"
            )
            return 0

        staging = resolve_target(args.staging)
        if not staging.is_dir():
            raise PublicationError(f"staging directory not found: {staging}")
        snapshot = load_json(resolve_target(args.baseline), "baseline")
        publish(
            target,
            staging,
            paths,
            snapshot,
            resolve_target(args.journal),
            removals,
            baseline_only,
        )
        print(
            f"published {len(paths)} and removed {len(removals)} path(s); "
            "keep journal until verification"
        )
        return 0
    except (OSError, OwnershipError, PublicationError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
