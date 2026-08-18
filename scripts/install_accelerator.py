#!/usr/bin/env python3
"""Validate inventories and copy a ready-made accelerator without data loss."""

from __future__ import annotations

import argparse
import fnmatch
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
ADDITIVE_FILES = {".gitattributes", ".gitignore"}
PRODUCTION_SOURCE_OVERRIDES = {
    "memory-bank/INDEX.md": "memory-bank/.install/INDEX.md",
}
EXCLUDED_EXACT_PATHS = {
    "CHANGELOG.md",
    "examples/context-summary.md",
    "examples/pr-description.md",
    "memory-bank/.install/INDEX.md",
    "memory-bank/.memory-counter",
    "memory-bank/chunks/MEM-0001-cross-edition-sync.md",
}
EXCLUDED_PATH_PATTERNS = (
    "Task/**",
    "examples/completed-task/**",
    "memory-bank/tests/**",
    "project-brain/tests/**",
    "*/skills/skill-creator/tests/**",
)
AGENTS_BEGIN = "<!-- BEGIN ACCELERATOR MANAGED POLICY -->"
AGENTS_END = "<!-- END ACCELERATOR MANAGED POLICY -->"
ENTRIES_BEGIN = "# BEGIN ACCELERATOR MANAGED ENTRIES"
ENTRIES_END = "# END ACCELERATOR MANAGED ENTRIES"


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
    if data.get("schema_version") != 2 or data.get("edition") != edition:
        raise InventoryError(f"{path}: unsupported schema or edition")
    if tuple(data) != (
        "schema_version",
        "inventory_version",
        "edition",
        "release",
        "scope",
        "installed",
        "excluded_tracked_paths",
        "source_overrides",
    ):
        raise InventoryError(f"{path}: unsupported or unordered inventory fields")
    components = data.get("installed")
    if not isinstance(components, dict) or tuple(components) != COMPONENTS:
        raise InventoryError(f"{path}: installed components must be ordered as {COMPONENTS}")
    seen: set[str] = set()
    for component, paths in components.items():
        if not isinstance(paths, list) or paths != sorted(paths) or len(paths) != len(set(paths)):
            raise InventoryError(f"{path}: {component} paths must be sorted and unique")
        for value in paths:
            if not is_safe_relative_path(value) or value in seen:
                raise InventoryError(f"{path}: invalid or duplicate path: {value!r}")
            seen.add(value)

    excluded = data.get("excluded_tracked_paths")
    if (
        not isinstance(excluded, list)
        or excluded != sorted(excluded)
        or len(excluded) != len(set(excluded))
    ):
        raise InventoryError(f"{path}: excluded paths must be sorted and unique")
    for value in excluded:
        if not is_safe_relative_path(value) or value in seen:
            raise InventoryError(f"{path}: invalid or overlapping exclusion: {value!r}")

    overrides = data.get("source_overrides")
    if not isinstance(overrides, dict) or list(overrides) != sorted(overrides):
        raise InventoryError(f"{path}: source overrides must be an ordered object")
    excluded_set = set(excluded)
    for destination, source in overrides.items():
        if (
            not is_safe_relative_path(destination)
            or not is_safe_relative_path(source)
            or destination not in seen
            or source not in excluded_set
            or destination == source
        ):
            raise InventoryError(
                f"{path}: invalid source override: {destination!r} -> {source!r}"
            )
    return data


def is_safe_relative_path(value: object) -> bool:
    if not isinstance(value, str) or not value or "\\" in value:
        return False
    pure = PurePosixPath(value)
    return (
        not pure.is_absolute()
        and ".." not in pure.parts
        and value == pure.as_posix()
        and value not in {".", ""}
    )


def selected_files(data: dict, tools: list[str]) -> list[tuple[str, str]]:
    selected = {"shared", *tools}
    return [
        (component, path)
        for component in COMPONENTS
        if component in selected
        for path in data["installed"][component]
    ]


def without_managed_block(text: str, begin: str, end: str) -> str:
    """Remove one complete installer-managed block before rebuilding it."""
    begin_count = text.count(begin)
    end_count = text.count(end)
    if begin_count != end_count or begin_count > 1:
        raise InventoryError("existing managed block is malformed")
    if begin_count == 0:
        return text.rstrip()
    before, remainder = text.split(begin, 1)
    _, after = remainder.split(end, 1)
    return (before.rstrip() + "\n" + after.lstrip()).rstrip()


def merge_additive_file(existing: str, source: str) -> str:
    """Preserve project entries and add missing accelerator directives."""
    base = without_managed_block(existing, ENTRIES_BEGIN, ENTRIES_END)
    existing_entries = {
        line.strip()
        for line in base.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }
    additions = [
        line.rstrip()
        for line in source.splitlines()
        if line.strip()
        and not line.lstrip().startswith("#")
        and line.strip() not in existing_entries
    ]
    if not additions:
        return base + ("\n" if existing.endswith("\n") else "")
    prefix = base + "\n\n" if base else ""
    return (
        prefix
        + ENTRIES_BEGIN
        + "\n"
        + "\n".join(additions)
        + "\n"
        + ENTRIES_END
        + "\n"
    )


def merge_agents_file(existing: str, source: str) -> str:
    """Keep project policy first and maintain one replaceable accelerator block."""
    base = without_managed_block(existing, AGENTS_BEGIN, AGENTS_END)
    prefix = base + "\n\n" if base else ""
    return prefix + AGENTS_BEGIN + "\n" + source.rstrip() + "\n" + AGENTS_END + "\n"


def discover_distribution_files(root: Path, edition: str) -> list[str]:
    """Return the Git-tracked distribution files of one edition.

    The inventory is a closed contract over what the repository tracks, so the
    file set comes from the index rather than from a working-tree scan. An
    untracked working tree may hold a client application, build caches, local
    databases or secrets; scanning the disk would publish that material into
    the inventories, which are shipped. Staging (`git add`) is enough to make a
    new distribution file visible here - a commit is not required - and both
    `--write-inventories` and `--verify-inventories` read the same index, so a
    working tree that is dirty with untracked files can neither change an
    inventory nor fail its verification.

    Without a Git checkout there is no way to tell distribution files from
    client data, so this raises instead of guessing from the filesystem.
    """
    command = ["git", "ls-files", "-z", "--cached", "--", edition]
    try:
        result = subprocess.run(command, cwd=str(root), capture_output=True)
    except OSError as error:
        raise InventoryError(
            f"{edition}: cannot run git ({error}); inventories require a Git checkout"
        ) from error
    if result.returncode != 0:
        reason = result.stderr.decode("utf-8", "replace").strip().splitlines()
        raise InventoryError(
            f"{edition}: cannot list tracked files under {root}"
            + (f": {reason[-1]}" if reason else "")
            + "; inventories are generated from a Git checkout only"
        )
    prefix = edition + "/"
    # Unmerged index entries repeat a path once per stage; distinct paths are
    # what the inventory records.
    paths = set()
    for raw in result.stdout.split(b"\0"):
        if not raw:
            continue
        value = raw.decode("utf-8")
        if value.startswith(prefix):
            paths.add(value[len(prefix) :])
    if not paths:
        raise InventoryError(
            f"{edition}: no tracked files under {root / edition}; "
            "--source-root must point at the Git checkout holding the editions"
        )
    return sorted(paths)


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


def is_source_only(path: str) -> bool:
    return path in EXCLUDED_EXACT_PATHS or any(
        fnmatch.fnmatchcase(path, pattern) for pattern in EXCLUDED_PATH_PATTERNS
    )


def build_inventory(root: Path, edition: str) -> dict:
    components = {component: [] for component in COMPONENTS}
    excluded: list[str] = []
    for path in discover_distribution_files(root, edition):
        if is_source_only(path):
            excluded.append(path)
        else:
            components[component_for(path)].append(path)
    version_file = root / edition / "VERSION"
    release = version_file.read_text(encoding="utf-8").strip()
    return {
        "schema_version": 2,
        "inventory_version": 2,
        "edition": edition,
        "release": release,
        "scope": "closed tracked-file contract for production installation",
        "installed": components,
        "excluded_tracked_paths": excluded,
        "source_overrides": dict(sorted(PRODUCTION_SOURCE_OVERRIDES.items())),
    }


def inventory_paths(data: dict) -> set[str]:
    return {
        *(path for paths in data["installed"].values() for path in paths),
        *data["excluded_tracked_paths"],
    }


DELTA_SAMPLE = 10


def report_delta(label: str, paths: set[str]) -> None:
    """Name what changed, so a wrong inventory is visible before it is committed."""
    if not paths:
        return
    listed = sorted(paths)
    for path in listed[:DELTA_SAMPLE]:
        print(f"\t{label}\t{path}")
    if len(listed) > DELTA_SAMPLE:
        print(f"\t{label}\t... and {len(listed) - DELTA_SAMPLE} more")


def write_inventories(root: Path, destination: Path | None = None) -> None:
    """Generate every edition inventory into ``destination``.

    ``destination`` defaults to the checkout's own ``install/inventories``.
    Pointing it elsewhere lets a caller regenerate and compare without writing
    into the repository under test.
    """
    if destination is None:
        destination = root / "install" / "inventories"
    generated = {
        edition: build_inventory(root, edition) for edition in EDITIONS
    }
    destination.mkdir(parents=True, exist_ok=True)
    for edition in EDITIONS:
        path = destination / inventory_path(edition).name
        previous: set[str] = set()
        if path.is_file():
            try:
                previous = inventory_paths(json.loads(path.read_text(encoding="utf-8")))
            except (ValueError, KeyError, AttributeError, TypeError):
                # An unreadable predecessor is not a reason to refuse to write
                # a correct successor; it only costs the delta.
                previous = set()
        data = generated[edition]
        path.write_text(
            json.dumps(data, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        current = inventory_paths(data)
        added, removed = current - previous, previous - current
        try:
            label = path.relative_to(root).as_posix()
        except ValueError:
            label = path.as_posix()
        print(
            f"WROTE\t{label}"
            f"\t{len(current)} path(s)\t+{len(added)}\t-{len(removed)}"
        )
        report_delta("+", added)
        report_delta("-", removed)


def verify_inventory(root: Path, edition: str) -> None:
    data = load_inventory(edition, root)
    actual = discover_distribution_files(root, edition)
    expected = sorted(
        path for paths in data["installed"].values() for path in paths
    )
    excluded = data["excluded_tracked_paths"]
    classified = sorted((*expected, *excluded))
    missing_sources = [
        path for path in expected if not (root / edition / path).is_file()
    ]
    override_sources = sorted(set(data["source_overrides"].values()))
    missing_override_sources = [
        path for path in override_sources if not (root / edition / path).is_file()
    ]
    if actual != classified or missing_sources or missing_override_sources:
        unclassified = sorted(set(actual) - set(classified))
        stale_installed = sorted(set(expected) - set(actual))
        stale_excluded = sorted(set(excluded) - set(actual))
        details = [
            *(f"UNCLASSIFIED\t{edition}/{path}" for path in unclassified),
            *(f"STALE_INSTALLED\t{edition}/{path}" for path in stale_installed),
            *(f"STALE_EXCLUSION\t{edition}/{path}" for path in stale_excluded),
            *(f"MISSING_SOURCE\t{edition}/{path}" for path in missing_sources),
            *(
                f"MISSING_OVERRIDE_SOURCE\t{edition}/{path}"
                for path in missing_override_sources
            ),
        ]
        raise InventoryError("\n".join(details) or f"{edition}: inventory mismatch")
    print(
        f"VERIFIED\t{edition}\tinstalled={len(expected)}"
        f"\texcluded={len(excluded)}"
    )


def install(
    root: Path,
    edition: str,
    target: Path,
    tools: list[str],
    dry_run: bool,
    overwrite: bool,
    merge_existing: bool,
) -> int:
    data = load_inventory(edition, root)
    files = selected_files(data, tools)
    collisions: list[tuple[str, str, str]] = []
    resolutions: dict[str, tuple[str, Path, bytes | None]] = {}
    for component, path in files:
        source_path = data["source_overrides"].get(path, path)
        source = root / edition / PurePosixPath(source_path)
        destination = target / PurePosixPath(path)
        if destination.is_symlink():
            collisions.append((component, path, "symlink"))
            continue
        if destination.exists():
            if not destination.is_file():
                collisions.append((component, path, "existing-non-file"))
                continue
            if source.read_bytes() == destination.read_bytes():
                resolutions[path] = ("unchanged", destination, None)
                continue
            if merge_existing and path in ADDITIVE_FILES:
                try:
                    merged = merge_additive_file(
                        destination.read_text(encoding="utf-8"),
                        source.read_text(encoding="utf-8"),
                    ).encode("utf-8")
                except (UnicodeError, InventoryError) as error:
                    collisions.append((component, path, f"cannot-merge:{error}"))
                    continue
                action = "unchanged" if merged == destination.read_bytes() else "merge"
                resolutions[path] = (action, destination, merged)
                continue
            if merge_existing and path == "AGENTS.md":
                try:
                    merged = merge_agents_file(
                        destination.read_text(encoding="utf-8"),
                        source.read_text(encoding="utf-8"),
                    ).encode("utf-8")
                except (UnicodeError, InventoryError) as error:
                    collisions.append((component, path, f"cannot-merge:{error}"))
                    continue
                action = "unchanged" if merged == destination.read_bytes() else "merge"
                resolutions[path] = (action, destination, merged)
                continue
            if merge_existing and path == "README.md":
                alternate = target / "ACCELERATOR.md"
                if alternate.is_symlink() or (alternate.exists() and not alternate.is_file()):
                    collisions.append((component, path, "accelerator-readme-obstruction"))
                elif alternate.exists() and alternate.read_bytes() != source.read_bytes():
                    collisions.append((component, path, "accelerator-readme-exists"))
                elif alternate.exists():
                    resolutions[path] = ("unchanged", alternate, None)
                else:
                    resolutions[path] = ("copy-as", alternate, None)
                continue
            collisions.append((component, path, "existing-file"))
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
        source_path = data["source_overrides"].get(path, path)
        source = root / edition / PurePosixPath(source_path)
        destination = target / PurePosixPath(path)
        if not source.is_file():
            raise InventoryError(f"source file missing: {source}")
        if path in resolutions:
            resolution, resolved_destination, content = resolutions[path]
            if resolution == "unchanged":
                print(f"UNCHANGED\t{component}\t{path}")
                continue
            label = {
                "merge": "WOULD_MERGE" if dry_run else "MERGE",
                "copy-as": "WOULD_COPY_AS" if dry_run else "COPY_AS",
            }[resolution]
            if not dry_run:
                resolved_destination.parent.mkdir(parents=True, exist_ok=True)
                if content is None:
                    shutil.copy2(source, resolved_destination)
                else:
                    resolved_destination.write_bytes(content)
            relative = resolved_destination.relative_to(target).as_posix()
            print(f"{label}\t{component}\t{path}\t{relative}")
            continue
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
    parser.add_argument(
        "--inventory-out",
        type=Path,
        help="with --write-inventories, write the generated inventories to this "
        "directory instead of the checkout's install/inventories",
    )
    parser.add_argument("--target", type=Path)
    parser.add_argument("--tool", action="append", choices=TOOLS)
    parser.add_argument("--dry-run", action="store_true")
    collision_mode = parser.add_mutually_exclusive_group()
    collision_mode.add_argument("--overwrite", action="store_true")
    collision_mode.add_argument(
        "--merge-existing",
        action="store_true",
        help="safely merge supported root files and refuse all other conflicts",
    )
    args = parser.parse_args()
    if args.edition and args.target is None:
        parser.error("--target is required with --edition")
    if args.inventory_out is not None and not args.write_inventories:
        parser.error("--inventory-out requires --write-inventories")
    return args


def main() -> int:
    args = parse_args()
    root = args.source_root.resolve()
    try:
        if args.write_inventories:
            destination = (
                args.inventory_out.expanduser().resolve()
                if args.inventory_out is not None
                else None
            )
            write_inventories(root, destination)
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
            args.merge_existing,
        )
    except (InventoryError, OSError, subprocess.CalledProcessError) as error:
        print(f"install-accelerator: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
