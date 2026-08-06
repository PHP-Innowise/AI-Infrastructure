#!/usr/bin/env python3
"""Validate inventories and copy a ready-made accelerator without data loss."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent
EDITIONS = ("Laravel", "Symfony", "PHP Core")
TOOLS = ("claude", "cursor", "codex")
COMPONENTS = ("shared", *TOOLS)
INVENTORY_DIR = ROOT / "install" / "inventories"


class InventoryError(Exception):
    """An inventory or installation precondition is invalid."""


def resolve_write_target(value: Path) -> Path:
    """Resolve a target only after rejecting symlinks in the supplied path."""
    expanded = value.expanduser()
    absolute = Path(os.path.abspath(str(expanded)))
    current = Path(absolute.anchor)
    for part in absolute.parts[1:]:
        current = current / part
        if current.is_symlink():
            raise InventoryError(f"target path contains a symlink: {value}")
    return absolute.resolve()


def inventory_path(edition: str) -> Path:
    return INVENTORY_DIR / f"{edition.lower().replace(' ', '-')}.json"


def load_inventory(edition: str, root: Path = ROOT) -> dict:
    path = root / "install" / "inventories" / inventory_path(edition).name
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise InventoryError(f"cannot load {path}: {error}") from error
    if data.get("schema_version") != 1 or data.get("edition") != edition:
        raise InventoryError(f"{path}: unsupported schema or edition")
    components = data.get("components")
    if not isinstance(components, dict) or tuple(components) != COMPONENTS:
        raise InventoryError(f"{path}: components must be ordered as {COMPONENTS}")
    seen: set[str] = set()
    for component, paths in components.items():
        if not isinstance(paths, list) or paths != sorted(paths) or len(paths) != len(set(paths)):
            raise InventoryError(f"{path}: {component} paths must be sorted and unique")
        for value in paths:
            pure = PurePosixPath(value)
            if (
                not isinstance(value, str)
                or not value
                or pure.is_absolute()
                or ".." in pure.parts
                or value in seen
            ):
                raise InventoryError(f"{path}: invalid or duplicate path: {value!r}")
            seen.add(value)
    return data


def selected_files(data: dict, tools: list[str]) -> list[tuple[str, str]]:
    selected = {"shared", *tools}
    return [
        (component, path)
        for component in COMPONENTS
        if component in selected
        for path in data["components"][component]
    ]


def discover_distribution_files(root: Path, edition: str) -> list[str]:
    """Return tracked plus non-ignored pending distribution files.

    Including non-ignored pending files makes local verification useful before
    the user stages a change. In CI, the same command resolves to tracked files.
    """
    result = subprocess.run(
        [
            "git",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            edition,
        ],
        cwd=root,
        capture_output=True,
        check=True,
    )
    prefix = edition + "/"
    return sorted(
        raw.decode("utf-8")[len(prefix) :]
        for raw in result.stdout.split(b"\0")
        if raw and raw.decode("utf-8").startswith(prefix)
    )


def component_for(path: str) -> str:
    # Cross-tool layout documentation is cited by shared durable memory and is
    # therefore installed with every tool selection. It describes distribution
    # capabilities; it does not activate an unselected integration.
    if path in {
        ".agents/README.md",
        ".claude/README.md",
        ".cursor/README.md",
        ".codex/README.md",
    }:
        return "shared"
    if path.startswith(".claude/"):
        return "claude"
    if path.startswith(".cursor/"):
        return "cursor"
    if path.startswith(".agents/") or path.startswith(".codex/"):
        return "codex"
    return "shared"


def build_inventory(root: Path, edition: str) -> dict:
    components = {component: [] for component in COMPONENTS}
    for path in discover_distribution_files(root, edition):
        components[component_for(path)].append(path)
    version_file = root / edition / "VERSION"
    release = version_file.read_text(encoding="utf-8").strip()
    return {
        "schema_version": 1,
        "inventory_version": 1,
        "edition": edition,
        "release": release,
        "scope": "repository distribution files; excludes runtime, local, and user state",
        "components": components,
    }


def write_inventories(root: Path) -> None:
    destination = root / "install" / "inventories"
    destination.mkdir(parents=True, exist_ok=True)
    for edition in EDITIONS:
        path = destination / inventory_path(edition).name
        path.write_text(
            json.dumps(build_inventory(root, edition), indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        print(f"WROTE\t{path.relative_to(root).as_posix()}")


def verify_inventory(root: Path, edition: str) -> None:
    data = load_inventory(edition, root)
    actual = discover_distribution_files(root, edition)
    expected = sorted(
        path for paths in data["components"].values() for path in paths
    )
    missing_sources = [
        path for path in expected if not (root / edition / path).is_file()
    ]
    if actual != expected or missing_sources:
        missing = sorted(set(actual) - set(expected))
        stale = sorted(set(expected) - set(actual))
        details = [
            *(f"UNLISTED\t{edition}/{path}" for path in missing),
            *(f"STALE\t{edition}/{path}" for path in stale),
            *(f"MISSING_SOURCE\t{edition}/{path}" for path in missing_sources),
        ]
        raise InventoryError("\n".join(details) or f"{edition}: inventory mismatch")
    print(f"VERIFIED\t{edition}\t{len(expected)} files")


def install(
    root: Path,
    edition: str,
    target: Path,
    tools: list[str],
    dry_run: bool,
    overwrite: bool,
) -> int:
    data = load_inventory(edition, root)
    files = selected_files(data, tools)
    collisions: list[tuple[str, str, str]] = []
    for component, path in files:
        destination = target / PurePosixPath(path)
        if destination.is_symlink():
            collisions.append((component, path, "symlink"))
            continue
        if destination.exists():
            reason = "existing-file" if destination.is_file() else "existing-non-file"
            collisions.append((component, path, reason))
            continue
        parent = destination.parent
        while parent != target.parent and parent != target:
            if parent.exists() or parent.is_symlink():
                if not parent.is_dir() or parent.is_symlink():
                    collisions.append((component, path, "parent-obstruction"))
                break
            parent = parent.parent

    hard_collisions = [
        collision for collision in collisions if collision[2] != "existing-file"
    ]
    if hard_collisions or (collisions and not overwrite):
        for component, path, reason in collisions:
            print(f"COLLISION\t{component}\t{path}\t{reason}", file=sys.stderr)
        print(
            f"REFUSED\t{len(collisions)} collision(s); no files copied",
            file=sys.stderr,
        )
        return 2

    overwrite_paths = {path for _, path, _ in collisions}
    action = "WOULD_COPY" if dry_run else "COPY"
    for component, path in files:
        source = root / edition / PurePosixPath(path)
        destination = target / PurePosixPath(path)
        if not source.is_file():
            raise InventoryError(f"source file missing: {source}")
        current_action = "WOULD_OVERWRITE" if dry_run and path in overwrite_paths else action
        if not dry_run:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
            if path in overwrite_paths:
                current_action = "OVERWRITE"
        print(f"{current_action}\t{component}\t{path}")
    print(
        f"COMPLETE\t{edition}\ttools={','.join(tools)}\tfiles={len(files)}"
        f"\tdry_run={str(dry_run).lower()}"
    )
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write-inventories", action="store_true")
    mode.add_argument("--verify-inventories", action="store_true")
    mode.add_argument("--edition", choices=EDITIONS)
    parser.add_argument("--target", type=Path)
    parser.add_argument("--tool", action="append", choices=TOOLS)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--overwrite", action="store_true")
    args = parser.parse_args()
    if args.edition and args.target is None:
        parser.error("--target is required with --edition")
    return args


def main() -> int:
    args = parse_args()
    root = args.source_root.resolve()
    try:
        if args.write_inventories:
            write_inventories(root)
            return 0
        if args.verify_inventories:
            for edition in EDITIONS:
                verify_inventory(root, edition)
            return 0
        tools = list(dict.fromkeys(args.tool or TOOLS))
        return install(
            root,
            args.edition,
            resolve_write_target(args.target),
            tools,
            args.dry_run,
            args.overwrite,
        )
    except (InventoryError, OSError, subprocess.CalledProcessError) as error:
        print(f"install-accelerator: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
