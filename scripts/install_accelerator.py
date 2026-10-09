#!/usr/bin/env python3
"""Validate inventories and copy a ready-made accelerator without data loss."""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import importlib.util
import json
import os
import re
import shutil
import stat
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent
EDITION_PATHS = {
    "Laravel": Path("Laravel"),
    "Symfony": Path("Symfony"),
    "PHP Core": Path("PHP Core"),
    "WordPress": Path("Cms/wordpress"),
}
EDITIONS = tuple(EDITION_PATHS)
TOOLS = ("claude", "cursor", "codex")
COMPONENTS = ("shared", *TOOLS)
INVENTORY_DIR = ROOT / "install" / "inventories"
ADDITIVE_FILES = {".gitattributes", ".gitignore"}
# Files a project may already have whose accelerator content lives in one
# replaceable managed block. `.claude/CLAUDE.md` carries the `@../AGENTS.md`
# import: Claude Code stops reading AGENTS.md by itself as soon as any
# CLAUDE.md exists, so the import is what loads the policy in a project that
# has its own CLAUDE.md, and an existing `.claude/CLAUDE.md` keeps its
# content with the import appended.
MANAGED_POLICY_FILES = {"AGENTS.md", ".claude/CLAUDE.md"}
# Files the accelerator seeds once and the project owns from then on: the
# runtime rewrites the indexes and the task counter, and the team edits the
# spec manifest and the runtime configuration. A reinstall over a project that
# had used the accelerator collided on them, and `--overwrite` reset the task
# counter to 1, so TASK-NNN numbers repeated and `project-brain validate`
# broke. An existing copy is kept under every mode and reported as KEPT;
# configuration keys a newer release adds fall back to the runtime defaults.
SEED_ONLY_PATHS = {
    "memory-bank/INDEX.md",
    "project-brain/config/runtime.json",
    "project-brain/indexes/active.json",
    "project-brain/indexes/archive.json",
    "specs/MANIFEST.md",
    "tasks/.task-counter",
}
# Files the accelerator's own runtime rewrites in this repository, which must
# still install in their pristine state. A developer who has run a task here
# carries a Brain index listing that task's records; those records are this
# repository's, not the target's, and copying the index without them installs
# an index pointing at files that do not exist - which the target's own
# validator then reports as stale. The memory index has the same shape of
# problem and set the precedent.
PRODUCTION_SOURCE_OVERRIDES = {
    # The edition's own .gitattributes marks every generated mirror `-diff`
    # for this repository's reviews. In a client project nothing regenerates
    # those mirrors, and the same marking showed an edited hook - a safety
    # control - as "Binary files differ" in review. The client gets the one
    # attribute it needs: LF for the hook scripts bash runs.
    ".gitattributes": ".install/gitattributes",
    "memory-bank/INDEX.md": "memory-bank/.install/INDEX.md",
    "project-brain/indexes/active.json": "project-brain/.install/active.json",
    "project-brain/indexes/archive.json": "project-brain/.install/archive.json",
}
EXCLUDED_EXACT_PATHS = {
    ".install/gitattributes",
    "CHANGELOG.md",
    "examples/context-summary.md",
    "examples/pr-description.md",
    "memory-bank/.install/INDEX.md",
    "memory-bank/.memory-counter",
    "project-brain/.install/active.json",
    "project-brain/.install/archive.json",
    "memory-bank/chunks/MEM-0001-cross-edition-sync.md",
}
EXCLUDED_PATH_PATTERNS = (
    "Task/**",
    "examples/completed-task/**",
    "memory-bank/tests/**",
    "project-brain/tests/**",
    "*/skills/skill-creator/tests/**",
    # Loads the edition in place for an attached Cursor session; an installed
    # project has its own .cursor tree and no use for it.
    ".cursor-plugin/**",
)
AGENTS_BEGIN = "<!-- BEGIN ACCELERATOR MANAGED POLICY -->"
AGENTS_END = "<!-- END ACCELERATOR MANAGED POLICY -->"
ENTRIES_BEGIN = "# BEGIN ACCELERATOR MANAGED ENTRIES"
ENTRIES_END = "# END ACCELERATOR MANAGED ENTRIES"
# Mode of an installed executable. Claude Code, Cursor and Codex run every
# wired hook as a direct command, so a hook installed without the bit exits
# 126 and its effect - a lock released, a command blocked - never happens.
EXECUTABLE_INSTALL_MODE = 0o755
MCP_CONFIG_FILES = {'.mcp.json', '.cursor/mcp.json', '.codex/config.toml'}
MCP_CODEX_BLOCK = re.compile(rb'# BEGIN HARNESS MEMORY MCP\n.*?# END HARNESS MEMORY MCP\n?', re.S)


def codex_memory_merge(source: bytes, destination: Path) -> bool:
    """Merge only the memory block, and only into the accelerator's own config.

    The shipped config also turns Codex hooks on and its built-in agents off;
    appending the block to a client's config would drop those silently, so a
    client-owned file stays an ordinary collision.
    """
    if b'# BEGIN HARNESS MEMORY MCP' not in source:
        return False
    if not destination.is_file():
        return True
    return MCP_CODEX_BLOCK.sub(b'', destination.read_bytes()) == MCP_CODEX_BLOCK.sub(b'', source)


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


def edition_path(edition: str) -> Path:
    """Return the repository-relative source directory for a public edition."""
    try:
        return EDITION_PATHS[edition]
    except KeyError as error:
        raise InventoryError(f"unsupported edition: {edition}") from error


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
    source_dir = edition_path(edition).as_posix()
    command = ["git", "ls-files", "-z", "--cached", "--", source_dir]
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
    prefix = source_dir + "/"
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
            f"{edition}: no tracked files under {root / edition_path(edition)}; "
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
    if path == '.mcp.json' or path.startswith(".claude/"):
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


def is_hook_script(path: str) -> bool:
    """A `*.sh` file inside a `hooks/` directory, which a client runs directly."""
    pure = PurePosixPath(path)
    return pure.suffix == ".sh" and "hooks" in pure.parts[:-1]


def source_executable_bits(root: Path, edition: str) -> dict[str, bool] | None:
    """The executable bits the source Git index records for one edition.

    Keyed by edition-relative path. The index, not the working tree, is the
    authority: a checkout made on Windows or with `core.fileMode=false` has
    no executable bit on disk, and copying its working tree would install
    every hook non-executable. Returns None without a Git checkout - a
    source extracted from an archive, or the standalone copy of this script
    the Harness runs against a staged source - and the caller then falls
    back to the filesystem bit. The Harness calls this on its real source
    and stages every index-executable file with the bit, so that fallback
    reaches the same decision as a direct run.

    Deliberately self-contained rather than shared with
    `scripts/file_modes.py`: the Harness copies this one file on its own.
    """
    source_dir = edition_path(edition).as_posix()
    command = ["git", "ls-files", "--stage", "-z", "--", source_dir]
    try:
        result = subprocess.run(command, cwd=str(root), capture_output=True)
    except OSError:
        return None
    if result.returncode != 0:
        return None
    prefix = source_dir + "/"
    bits: dict[str, bool] = {}
    for record in result.stdout.split(b"\0"):
        meta, tab, raw = record.partition(b"\t")
        fields = meta.split()
        if not tab or len(fields) != 3:
            continue
        value = raw.decode("utf-8", "surrogateescape")
        if value.startswith(prefix):
            bits.setdefault(value[len(prefix) :], fields[0] == b"100755")
    return bits


def installs_executable(
    path: str, source: Path, source_path: str, bits: dict[str, bool] | None
) -> bool:
    """Whether the installed copy of ``path`` must be executable.

    Hook scripts always are. Any other file follows its source: the index
    bit when the source checkout records one, otherwise the filesystem bit
    (where the filesystem has one - Windows reports none for a script).
    """
    if is_hook_script(path):
        return True
    if bits is not None and source_path in bits:
        return bits[source_path]
    return os.name != "nt" and bool(source.stat().st_mode & stat.S_IXUSR)


def lacks_executable_bit(path: Path) -> bool:
    return os.name != "nt" and not path.stat().st_mode & stat.S_IXUSR


def add_executable_bit(path: Path) -> None:
    """`chmod +x` on a file already in place: execute for the owner, and for
    group and others wherever they may read it. Nothing else changes."""
    current = stat.S_IMODE(path.stat().st_mode)
    os.chmod(path, current | stat.S_IXUSR | ((current & 0o044) >> 2))


def build_inventory(root: Path, edition: str) -> dict:
    components = {component: [] for component in COMPONENTS}
    excluded: list[str] = []
    for path in discover_distribution_files(root, edition):
        if is_source_only(path):
            excluded.append(path)
        else:
            components[component_for(path)].append(path)
    installed = {path for paths in components.values() for path in paths}
    version_file = root / edition_path(edition) / "VERSION"
    release = version_file.read_text(encoding="utf-8").strip()
    return {
        "schema_version": 2,
        "inventory_version": 2,
        "edition": edition,
        "release": release,
        "scope": "closed tracked-file contract for production installation",
        "installed": components,
        "excluded_tracked_paths": excluded,
        # The override table is global while editions differ: an edition that
        # ships no Project Brain has nothing to override, and naming a path it
        # does not install would make its own inventory invalid.
        "source_overrides": {
            destination: source
            for destination, source in sorted(PRODUCTION_SOURCE_OVERRIDES.items())
            if destination in installed and source in excluded
        },
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
        path for path in expected if not (root / edition_path(edition) / path).is_file()
    ]
    override_sources = sorted(set(data["source_overrides"].values()))
    missing_override_sources = [
        path for path in override_sources
        if not (root / edition_path(edition) / path).is_file()
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
    mcp, python = None, None
    for component, path in files:
        source_path = data["source_overrides"].get(path, path)
        source = root / edition_path(edition) / PurePosixPath(source_path)
        destination = target / PurePosixPath(path)
        if path in MCP_CONFIG_FILES:
            blocked_parent = False
            for parent in destination.parents:
                if parent == target: break
                if parent.is_symlink() or (parent.exists() and not parent.is_dir()):
                    collisions.append((component, path, 'parent-obstruction'))
                    blocked_parent = True
                    break
            if blocked_parent: continue
        if destination.is_symlink():
            collisions.append((component, path, "symlink"))
            continue
        if path in MCP_CONFIG_FILES and (path != '.codex/config.toml' or codex_memory_merge(source.read_bytes(), destination)):
            if destination.exists() and not destination.is_file():
                collisions.append((component, path, 'existing-non-file'))
                continue
            try:
                if mcp is None:
                    spec = importlib.util.spec_from_file_location('accelerator_memory_mcp_config',
                        root / edition_path(edition) / 'memory-bank/scripts/mcp_config.py')
                    mcp = importlib.util.module_from_spec(spec)
                    spec.loader.exec_module(mcp)
                    python = mcp.python_command()
                previous = destination.read_bytes() if destination.exists() else None
                content = mcp.merge(path, previous, source.read_bytes(), python)
            except (ValueError, UnicodeError, OSError) as error:
                collisions.append((component, path, 'cannot-merge:memory-mcp-configuration'))
                continue
            if previous is not None:
                resolutions[path] = ('unchanged' if previous == content else 'merge', destination, content)
            elif content != source.read_bytes():
                resolutions[path] = ('copy-as', destination, content)
            # Parent checks below still apply to new materialized files.
            if previous is not None:
                continue
        if destination.exists():
            if not destination.is_file():
                collisions.append((component, path, "existing-non-file"))
                continue
            if source.read_bytes() == destination.read_bytes():
                resolutions[path] = ("unchanged", destination, None)
                continue
            if path in SEED_ONLY_PATHS:
                resolutions[path] = ("kept", destination, None)
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
            if merge_existing and path in MANAGED_POLICY_FILES:
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
    executable_bits = source_executable_bits(root, edition)
    for component, path in files:
        source_path = data["source_overrides"].get(path, path)
        source = root / edition_path(edition) / PurePosixPath(source_path)
        destination = target / PurePosixPath(path)
        if not source.is_file():
            raise InventoryError(f"source file missing: {source}")
        # Normal copy semantics carry the working-tree mode, which is exactly
        # what a checkout without filesystem modes gets wrong; an executable
        # is therefore given its mode explicitly after the copy.
        executable = installs_executable(path, source, source_path, executable_bits)
        if path in resolutions:
            resolution, resolved_destination, content = resolutions[path]
            if resolution == "unchanged":
                if executable and lacks_executable_bit(resolved_destination):
                    # Identical bytes without the bit - typically a hook from
                    # an install made before executable bits were enforced.
                    # The content stays untouched and only the bit is added,
                    # which destroys nothing, so this happens under every
                    # collision mode. The Harness reads the repaired mode
                    # from its staged run and applies exactly that.
                    if not dry_run:
                        add_executable_bit(resolved_destination)
                    relative = resolved_destination.relative_to(target).as_posix()
                    label = "WOULD_FIX_MODE" if dry_run else "FIX_MODE"
                    print(f"{label}\t{component}\t{path}\t{relative}")
                    continue
                print(f"UNCHANGED\t{component}\t{path}")
                continue
            if resolution == "kept":
                print(f"KEPT\t{component}\t{path}")
                continue
            label = {
                "merge": "WOULD_MERGE" if dry_run else "MERGE",
                "copy-as": "WOULD_COPY_AS" if dry_run else "COPY_AS",
            }[resolution]
            if not dry_run:
                resolved_destination.parent.mkdir(parents=True, exist_ok=True)
                if content is None:
                    shutil.copy2(source, resolved_destination)
                    if executable:
                        os.chmod(resolved_destination, EXECUTABLE_INSTALL_MODE)
                else:
                    resolved_destination.write_bytes(content)
            relative = resolved_destination.relative_to(target).as_posix()
            print(f"{label}\t{component}\t{path}\t{relative}")
            continue
        current_action = "WOULD_OVERWRITE" if dry_run and path in overwrite_paths else action
        if not dry_run:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
            if executable:
                os.chmod(destination, EXECUTABLE_INSTALL_MODE)
            if path in overwrite_paths:
                current_action = "OVERWRITE"
        print(f"{current_action}\t{component}\t{path}")
    print(
        f"COMPLETE\t{edition}\ttools={','.join(tools)}\tfiles={len(files)}"
        f"\tdry_run={str(dry_run).lower()}"
    )
    return 0


# ---------------------------------------------------------------------------
# Keeping an installed project current
# ---------------------------------------------------------------------------

# Where a synced project records what the accelerator last wrote, so the next
# sync knows which files are still untouched. Ignored local state.
SYNC_MANIFEST = "memory-bank/local/accelerator-install.json"
SYNC_BACKUP_DIR = "memory-bank/local/accelerator-sync"
SYNC_MANIFEST_SCHEMA = 1
# The accelerator's runtime: code every memory fix lives in. A project that
# runs an old copy gets none of them - on six real installations none carried
# the fixes of the previous week - so a local edit here is backed up and
# replaced rather than left to keep the project on the old behaviour.
RUNTIME_SYNC_PATTERNS = (
    "memory-bank/scripts/*.py",
    "project-brain/scripts/*.py",
    "project-brain/schemas/*.json",
    ".claude/hooks/working-memory-*.sh",
    ".codex/hooks/working-memory-*.sh",
    ".cursor/hooks/working-memory-*.sh",
)
# Codex runs a project hook only while the stored hash of its definition
# matches, so rewriting this switches every hook off until it is approved
# again. It is rewritten only for a caller that re-approves the accelerator's
# own definitions right after (the Harness, `rewire_codex=True`); otherwise it
# is reported and left alone.
TRUST_BOUND_FILES = {".codex/hooks.json"}
# Documentation an install copied as ACCELERATOR.md when the project had its
# own README; not worth a write into the project.
SYNC_SKIPPED_FILES = {"README.md"}
# The fewest inventory files a folder must hold to count as an install of an
# edition: one stray AGENTS.md is not an installation.
SYNC_MINIMUM_PRESENT = 10


def git_blob_id(data: bytes) -> str:
    """The id Git gives these bytes, computed without Git."""
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def installed_edition(root: Path, target: Path) -> str | None:
    """The edition whose inventory the target holds the most of, if any."""
    counts = []
    for edition in EDITIONS:
        try:
            data = load_inventory(edition, root)
        except InventoryError:
            continue
        present = sum(
            1
            for paths in data["installed"].values()
            for path in paths
            if (target / PurePosixPath(path)).is_file()
        )
        counts.append((present, edition))
    counts.sort(reverse=True)
    if not counts or counts[0][0] < SYNC_MINIMUM_PRESENT:
        return None
    if len(counts) > 1 and counts[1][0] == counts[0][0]:
        return None
    return counts[0][1]


def released_blob_ids(root: Path, edition: str) -> dict[str, set[str]] | None:
    """Every blob each edition file has had in this clone's history.

    An installed file whose bytes match one of them is an accelerator file
    nobody edited, at whatever release it was installed. None without a Git
    clone (a downloaded archive): then only the sync manifest can tell.
    """
    prefix = edition_path(edition).as_posix() + "/"
    command = [
        "git", "log", "--all", "--format=", "--raw", "--no-abbrev",
        "--no-renames", "--", edition_path(edition).as_posix(),
    ]
    try:
        result = subprocess.run(command, cwd=str(root), capture_output=True)
    except OSError:
        return None
    if result.returncode != 0:
        return None
    blobs: dict[str, set[str]] = {}
    for line in result.stdout.decode("utf-8", "surrogateescape").splitlines():
        if not line.startswith(":") or "\t" not in line:
            continue
        meta, path = line.split("\t", 1)
        if not path.startswith(prefix):
            continue
        for blob in meta.split()[2:4]:
            if blob.strip("0"):
                blobs.setdefault(path[len(prefix):], set()).add(blob)
    return blobs


def tracked_paths(target: Path) -> set[str]:
    """Files the project's own Git history holds; a sync never writes them."""
    try:
        result = subprocess.run(
            ["git", "-C", str(target), "ls-files", "-z"], capture_output=True
        )
    except OSError:
        return set()
    if result.returncode != 0:
        return set()
    return {
        item.decode("utf-8", "surrogateescape")
        for item in result.stdout.split(b"\0")
        if item
    }


def is_runtime_file(path: str) -> bool:
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in RUNTIME_SYNC_PATTERNS)


def source_commit(root: Path) -> str | None:
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True
        )
    except OSError:
        return None
    return result.stdout.strip() or None if result.returncode == 0 else None


def _write_atomically(path: Path, data: bytes, executable: bool) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.sync-{os.getpid()}")
    try:
        temporary.write_bytes(data)
        if executable:
            os.chmod(temporary, EXECUTABLE_INSTALL_MODE)
        elif path.exists():
            os.chmod(temporary, stat.S_IMODE(path.stat().st_mode))
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def sync_installation(
    root: Path,
    target: Path,
    *,
    dry_run: bool = False,
    stamp: str | None = None,
    rewire_codex: bool = False,
) -> dict:
    """Bring an installed project's accelerator files up to this clone.

    Runs with no question asked, so it only writes what is safe without one:
    files nobody edited (the bytes of some released version, or of what the
    last sync wrote), files the release added to a component the project
    has, and the managed blocks of AGENTS.md and .claude/CLAUDE.md. The
    runtime is also replaced over a local edit, which is backed up first
    under memory-bank/local. Never written: anything the project's Git
    tracks, seeded state the project owns, and - unless `rewire_codex` says
    the caller re-approves it - the Codex hook wiring, whose trust is a hash
    of its definitions. Each of those is reported instead.
    """
    report: dict = {
        "target": str(target), "edition": None, "release": None, "changed": [],
        "kept": [], "backups": [], "dry_run": dry_run, "error": None,
    }
    if not (target / "memory-bank/scripts/context.py").is_file():
        report["error"] = "not an installed accelerator"
        return report
    edition = installed_edition(root, target)
    if edition is None:
        report["error"] = "the installed edition could not be determined"
        return report
    data = load_inventory(edition, root)
    report.update(edition=edition, release=data["release"])
    present = {
        component
        for component, paths in data["installed"].items()
        if component == "shared"
        or any((target / PurePosixPath(path)).is_file() for path in paths)
    }
    tracked = tracked_paths(target)
    manifest_path = target / SYNC_MANIFEST
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        known = manifest.get("files") if isinstance(manifest, dict) else None
        known = known if isinstance(known, dict) else {}
    except (OSError, ValueError):
        known = {}
    history = released_blob_ids(root, edition)
    executable_bits = source_executable_bits(root, edition)
    stamp = stamp or datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    written: dict[str, str] = {}

    def keep(path: str, reason: str) -> None:
        report["kept"].append({"path": path, "reason": reason})

    def write(path: str, payload: bytes, action: str, executable: bool, backup: bool) -> None:
        destination = target / PurePosixPath(path)
        if backup and not dry_run:
            saved = target / SYNC_BACKUP_DIR / stamp / PurePosixPath(path)
            saved.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(destination, saved)
            report["backups"].append(saved.relative_to(target).as_posix())
        if not dry_run:
            _write_atomically(destination, payload, executable)
        report["changed"].append({"path": path, "action": action})
        written[path] = git_blob_id(payload)

    for component, path in selected_files(data, [c for c in TOOLS if c in present]):
        if path in SEED_ONLY_PATHS or path in SYNC_SKIPPED_FILES:
            continue
        source_path = data["source_overrides"].get(path, path)
        source = root / edition_path(edition) / PurePosixPath(source_path)
        destination = target / PurePosixPath(path)
        payload = source.read_bytes()
        executable = installs_executable(path, source, source_path, executable_bits)
        if destination.is_symlink() or (destination.exists() and not destination.is_file()):
            keep(path, "not a regular file")
            continue
        exists = destination.is_file()
        current = destination.read_bytes() if exists else None
        if current == payload:
            written[path] = git_blob_id(payload)
            continue
        if path in tracked:
            keep(path, "tracked by the project's Git: update it through a commit")
            continue
        if path in TRUST_BOUND_FILES and exists and not rewire_codex:
            keep(path, "Codex hook wiring: a change needs re-approval in Codex /hooks")
            continue
        if not exists:
            write(path, payload, "added", executable, False)
            continue
        blob = git_blob_id(current)
        pristine = known.get(path) == blob or (
            history is not None and blob in history.get(source_path, set())
        )
        if path in MANAGED_POLICY_FILES:
            text = current.decode("utf-8", "surrogateescape")
            source_text = payload.decode("utf-8")
            if AGENTS_BEGIN in text:
                try:
                    merged = merge_agents_file(text, source_text).encode("utf-8")
                except (InventoryError, UnicodeError):
                    keep(path, "managed block is malformed")
                    continue
                if merged != current:
                    write(path, merged, "managed block updated", executable, False)
                else:
                    written[path] = git_blob_id(current)
                continue
            first = source_text.splitlines()[0] if source_text else ""
            if pristine:
                write(path, payload, "updated", executable, False)
            elif first and text.startswith(first):
                # The accelerator's own policy, edited in place: the edit is
                # kept in the backup and the current policy goes live.
                write(path, payload, "updated over a local edit", executable, True)
            elif path == ".claude/CLAUDE.md":
                write(
                    path,
                    merge_agents_file(text, source_text).encode("utf-8"),
                    "import added",
                    executable,
                    False,
                )
            else:
                keep(path, "the project's own file")
            continue
        if path in ADDITIVE_FILES:
            keep(path, "the project's own file")
            continue
        if pristine:
            write(path, payload, "updated", executable, False)
        elif is_runtime_file(path):
            write(path, payload, "updated over a local edit", executable, True)
        else:
            keep(path, "edited in the project")
    if not dry_run:
        manifest_path.parent.mkdir(parents=True, exist_ok=True)
        _write_atomically(
            manifest_path,
            (
                json.dumps(
                    {
                        "schema": SYNC_MANIFEST_SCHEMA,
                        "edition": edition,
                        "release": data["release"],
                        "source_commit": source_commit(root),
                        "synced_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
                        "files": {**known, **written},
                    },
                    indent=1,
                    sort_keys=True,
                )
                + "\n"
            ).encode("utf-8"),
            False,
        )
    return report


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write-inventories", action="store_true")
    mode.add_argument("--verify-inventories", action="store_true")
    mode.add_argument("--edition", choices=EDITIONS)
    mode.add_argument(
        "--sync",
        action="store_true",
        help="bring the accelerator installed in --target up to this clone: "
        "untouched files and the runtime only; prints a JSON report",
    )
    parser.add_argument(
        "--inventory-out",
        type=Path,
        help="with --write-inventories, write the generated inventories to this "
        "directory instead of the checkout's install/inventories",
    )
    parser.add_argument(
        "--rewire-codex",
        action="store_true",
        help="with --sync, also update an untouched .codex/hooks.json; Codex then "
        "runs the hooks only after they are approved again in /hooks",
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
    if (args.edition or args.sync) and args.target is None:
        parser.error("--target is required with --edition and --sync")
    if args.inventory_out is not None and not args.write_inventories:
        parser.error("--inventory-out requires --write-inventories")
    if args.rewire_codex and not args.sync:
        parser.error("--rewire-codex requires --sync")
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
        if args.sync:
            report = sync_installation(
                root,
                resolve_write_target(args.target),
                dry_run=args.dry_run,
                rewire_codex=args.rewire_codex,
            )
            print(json.dumps(report, indent=1, ensure_ascii=False))
            return 1 if report["error"] else 0
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
