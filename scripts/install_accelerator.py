#!/usr/bin/env python3
"""Validate inventories and copy a ready-made accelerator without data loss."""

from __future__ import annotations

import argparse
import errno
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
    # Durable memory captured while building one project. The production
    # index override (memory-bank/.install/INDEX.md) ships an empty table, so
    # a clean install is meant to carry no chunks at all — MEM-0001 was listed
    # exactly above because it was the only chunk that existed when this list
    # was written. Installing a project's own chunks fails validation twice
    # over: they are absent from the shipped INDEX.md, and their cited sources
    # (Task/app/...) are not installed.
    "memory-bank/chunks/**",
    "project-brain/tests/**",
    # Governed task runtime written while working on one project. The shipped
    # project-brain/indexes/active.json is `[]`, so installing another
    # project's tasks makes that index stale on arrival. Matched by file type
    # rather than by directory so the .gitkeep placeholders that create the
    # runtime's directory structure still ship.
    "project-brain/control/handoffs/*.md",
    "project-brain/control/messages/*.jsonl",
    "project-brain/control/retrieval-manifests/*.json",
    "project-brain/dynamic/*/*.md",
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
CODEX_CONFIG = '.codex/config.toml'
MCP_CODEX_BEGIN = b'# BEGIN HARNESS MEMORY MCP'
MCP_CODEX_BLOCK = re.compile(rb'# BEGIN HARNESS MEMORY MCP\n.*?# END HARNESS MEMORY MCP\n?', re.S)


def codex_memory_merge(source: bytes, destination: Path) -> bool:
    """Merge only the memory block, and only into the accelerator's own config.

    The shipped config also turns Codex hooks on and its built-in agents off;
    appending the block to a client's config would drop those silently, so a
    client-owned file stays an ordinary collision.
    """
    if MCP_CODEX_BEGIN not in source:
        return False
    if not destination.is_file():
        return True
    return MCP_CODEX_BLOCK.sub(b'', destination.read_bytes()) == MCP_CODEX_BLOCK.sub(b'', source)


def load_memory_mcp(root: Path, edition: str):
    """The edition's MCP configuration merger, from the source clone.

    Install and sync merge with the release's own code, so the entry they
    write and the entries they recognise as the accelerator's are that
    release's.
    """
    spec = importlib.util.spec_from_file_location(
        'accelerator_memory_mcp_config',
        root / edition_path(edition) / 'memory-bank/scripts/mcp_config.py',
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
                    # Both or neither: a module kept without its Python sent
                    # the next configuration into merge() with None.
                    module = load_memory_mcp(root, edition)
                    mcp, python = module, module.python_command()
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
    # Every project file this run replaces, with what it held: a run that
    # fails part way puts them back (put_back).
    replaced: list[tuple] = []
    try:
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
                relative = resolved_destination.relative_to(target).as_posix()
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
                    if content is None:
                        resolved_destination.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(source, resolved_destination)
                        if executable:
                            os.chmod(resolved_destination, EXECUTABLE_INSTALL_MODE)
                    else:
                        # A merge holds the project's own content: written
                        # whole beside the file and renamed over it.
                        replace_project_file(target, (component, path), relative, content, replaced)
                print(f"{label}\t{component}\t{path}\t{relative}")
                continue
            current_action = "WOULD_OVERWRITE" if dry_run and path in overwrite_paths else action
            if not dry_run:
                if path in overwrite_paths or path in MCP_CONFIG_FILES:
                    # Replacing a project file, or writing a configuration a
                    # later merge has to parse: never left half written.
                    info = source.stat()
                    replace_project_file(
                        target, (component, path), path, source.read_bytes(), replaced,
                        EXECUTABLE_INSTALL_MODE if executable else stat.S_IMODE(info.st_mode),
                        (info.st_atime_ns, info.st_mtime_ns),
                    )
                else:
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, destination)
                    if executable:
                        os.chmod(destination, EXECUTABLE_INSTALL_MODE)
                if path in overwrite_paths:
                    current_action = "OVERWRITE"
            print(f"{current_action}\t{component}\t{path}")
    except BaseException:
        put_back(target, replaced)
        raise
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
# The file whose presence makes a folder an installed accelerator.
SYNC_RUNTIME_MARKER = "memory-bank/scripts/context.py"
# The accelerator's runtime: code every memory fix lives in. A project that
# runs an old copy gets none of them - on six real installations none carried
# the fixes of the previous week - so a local edit here is backed up and
# replaced rather than left to keep the project on the old behaviour.
CORE_RUNTIME_PATTERNS = (
    "memory-bank/scripts/*.py",
    "project-brain/scripts/*.py",
    "project-brain/schemas/*.json",
)
RUNTIME_SYNC_PATTERNS = CORE_RUNTIME_PATTERNS + (
    ".claude/hooks/working-memory-*.sh",
    ".codex/hooks/working-memory-*.sh",
    ".cursor/hooks/working-memory-*.sh",
)
# What a release's files lean on. Its skills, commands, hooks and memory
# server run its own runtime (a skill calls a `context.py` subcommand the
# release added, the memory server imports a function the release added to
# `brain_runtime.py`), and a tool runs only the hooks its wiring names. A
# project whose Git tracks its install kept the runtime and the wiring at the
# release it committed while the sync added the newer release's files: the
# agent saw /context-save, which failed with "invalid choice: 'context-save'",
# and a memory server that crashed on import. So when a runtime file stays
# at the project's version, nothing else of the release is written either;
# when a tool's wiring stays, neither are the new hooks it would run; and when
# a new hook is not written, neither is the wiring that would run it. Each
# such file is reported under `kept` as held back, and the report as partial.
HOOK_WIRING = {
    ".claude/settings.json": ".claude/hooks/",
    ".cursor/hooks.json": ".cursor/hooks/",
    ".codex/hooks.json": ".codex/hooks/",
}
HELD_BACK = "held back"
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
        present = len(
            confined_files(target, (path for paths in data["installed"].values() for path in paths))
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


def is_core_runtime_file(path: str) -> bool:
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in CORE_RUNTIME_PATTERNS)


def hook_wiring_of(path: str) -> str | None:
    """The wiring file that runs the hook script at `path`, if it is one."""
    for wiring, folder in HOOK_WIRING.items():
        if path.startswith(folder):
            return wiring
    return None


def source_commit(root: Path) -> str | None:
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True
        )
    except OSError:
        return None
    return result.stdout.strip() or None if result.returncode == 0 else None


# A sync reaches every path it reads or writes from the target one folder at
# a time, so a symbolic link is seen as itself and never followed. Git checks
# links out, and one standing in for `.cursor`, `memory-bank` or
# `memory-bank/local` took the sync's writes - its backups of a person's edits
# and its own record included - to wherever it pointed, with nobody asked: the
# Harness syncs a project when it opens it. Checking only the last component
# did not see them. A path such a link or a non-folder stands on is reported
# and left alone.
#
# Looking first and then using the path is not enough: a process that can
# write to the project could swap a folder for a link after the look, and the
# write followed it - a `.cursor` swapped between the check and the temporary
# file sent `.cursor/mcp.json` to wherever the link pointed. Where the
# platform has the descriptor-relative calls, a read or write therefore holds
# a descriptor of each folder on the way and opens the next component
# relative to it without following a link (openat with O_NOFOLLOW, mkdirat,
# fstatat); the temporary file is created, written and renamed through the
# last folder's descriptor (renameat, unlinkat). A name is resolved once, so
# a link swapped in afterwards is never followed; a folder swapped while it
# was written is found before the rename, and the file is left alone.
#
# Native Windows has none of these calls in `os.supports_dir_fd`. It used to
# look at each component with lstat and then write by the path, and the same
# swap sent `.cursor/mcp.json` outside the project there. The walk now takes
# the same steps by NT handles (_HandleCalls): each component opened relative
# to the handle of the folder before it, never through a reparse point, and
# the temporary file created in and renamed within the last folder's handle.
# Handles that do not share delete keep a folder from being renamed, but not
# from being turned into a junction in place while it is empty (that takes
# only FILE_WRITE_ATTRIBUTES, which no share mode denies): a write by path
# under held handles could still follow one, a write relative to a handle
# cannot. Where neither descriptors nor handles are available nothing is
# written (NO_SAFE_WRITE); a read there still looks at each component first.
#
# Windows reports symbolic links and junctions as reparse points; these are
# the tags of the two that name another path (IO_REPARSE_TAG_SYMLINK,
# IO_REPARSE_TAG_MOUNT_POINT).
LINK_REPARSE_TAGS = (0xA000000C, 0xA0000003)
# Whether this platform can walk a path by folder descriptors. os.replace is
# os.rename with replacement semantics and takes the same src_dir_fd and
# dst_dir_fd; os.supports_dir_fd lists it under rename only.
_DESCRIPTOR_WALK = (
    hasattr(os, "O_DIRECTORY")
    and hasattr(os, "O_NOFOLLOW")
    and hasattr(os, "fchmod")
    and {os.open, os.mkdir, os.stat, os.unlink, os.rename} <= os.supports_dir_fd
    and os.stat in os.supports_follow_symlinks
    and os.utime in os.supports_fd
)
_PROJECT_FLAGS = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
_FOLDER_FLAGS = _PROJECT_FLAGS | getattr(os, "O_NOFOLLOW", 0)
_READ_FLAGS = (
    os.O_RDONLY
    | getattr(os, "O_NOFOLLOW", 0)
    | getattr(os, "O_NONBLOCK", 0)
    | getattr(os, "O_BINARY", 0)
)
_TEMPORARY_FLAGS = (
    os.O_WRONLY
    | os.O_CREAT
    | os.O_EXCL
    | getattr(os, "O_NOFOLLOW", 0)
    | getattr(os, "O_BINARY", 0)
)
# Why a write is refused where neither descriptors nor handles can walk.
NO_SAFE_WRITE = (
    "not written: this platform cannot open a folder relative to another, "
    "so a folder swapped for a link meanwhile would take the write"
)


class _DescriptorCalls:
    """The calls a walk makes, by folder descriptors (POSIX).

    Each names one component relative to an open folder and never follows a
    link standing at it. `os` is looked up at every call, so a test that
    patches os.open sees each open the walk makes.
    """

    @staticmethod
    def open_project(target: Path) -> int:
        return os.open(os.fspath(target), _PROJECT_FLAGS)

    @staticmethod
    def open_folder(name: str, folder: int) -> int:
        return os.open(name, _FOLDER_FLAGS, dir_fd=folder)

    @staticmethod
    def make_folder(name: str, folder: int) -> None:
        os.mkdir(name, dir_fd=folder)

    @staticmethod
    def lstat(name: str, folder: int) -> os.stat_result:
        return os.stat(name, dir_fd=folder, follow_symlinks=False)

    @staticmethod
    def open_file(name: str, folder: int) -> int:
        return os.open(name, _READ_FLAGS, dir_fd=folder)

    @staticmethod
    def create_file(name: str, mode: int, folder: int) -> int:
        return os.open(name, _TEMPORARY_FLAGS, mode, dir_fd=folder)

    @staticmethod
    def fstat(descriptor: int) -> os.stat_result:
        return os.fstat(descriptor)

    @staticmethod
    def chmod(descriptor: int, mode: int) -> None:
        os.fchmod(descriptor, mode)

    @staticmethod
    def set_times(descriptor: int, times: tuple[int, int]) -> None:
        os.utime(descriptor, ns=times)

    @staticmethod
    def rename(name: str, new: str, folder: int) -> None:
        os.replace(name, new, src_dir_fd=folder, dst_dir_fd=folder)

    @staticmethod
    def unlink(name: str, folder: int) -> None:
        os.unlink(name, dir_fd=folder)

    @staticmethod
    def close(descriptor: int) -> None:
        os.close(descriptor)


class _HandleCalls:
    """The same calls on native Windows, by NT handles.

    A component is opened with NtCreateFile relative to the handle of the
    folder before it - a RootDirectory and a one-component name - and with
    FILE_OPEN_REPARSE_POINT, so a symbolic link or junction is opened as
    itself and refused. The temporary file is created relative to the last
    folder's handle and renamed within it (FileRenameInformation with that
    handle as its RootDirectory). Handles share read and write but not
    delete. scripts/portable_fs.py, which the Harness uses, works the same
    way; this is a copy because the Harness runs this file alone, from a
    staging folder that holds no other module. The descriptors returned are
    the C runtime's, so os.write, os.fsync and os.close work on them.
    """

    FILE_READ_DATA = FILE_LIST_DIRECTORY = 0x0001
    FILE_TRAVERSE = 0x0020
    FILE_READ_ATTRIBUTES = 0x0080
    DELETE = 0x00010000
    SYNCHRONIZE = 0x00100000
    GENERIC_WRITE = 0x40000000
    FILE_SHARE_READ, FILE_SHARE_WRITE = 0x1, 0x2
    FILE_OPEN, FILE_CREATE = 1, 2
    FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE = 0x1, 0x40
    FILE_SYNCHRONOUS_IO_NONALERT, FILE_OPEN_REPARSE_POINT = 0x20, 0x00200000
    FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_DIRECTORY = 0x1, 0x10
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT = 0x80, 0x400
    OBJ_CASE_INSENSITIVE = 0x40
    FILE_RENAME_INFORMATION, FILE_DISPOSITION_INFORMATION = 10, 13
    FILE_ATTRIBUTE_TAG_INFO = 9
    OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS = 3, 0x02000000
    # A reparse point that names another path: symbolic links, junctions.
    NAME_SURROGATE = 0x20000000
    # 100-nanosecond FILETIME intervals from 1601 to the Unix epoch.
    FILETIME_EPOCH = 116444736000000000
    FOLDER_ACCESS = FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE

    def __init__(self) -> None:
        import ctypes
        import msvcrt
        from ctypes import wintypes

        class UnicodeString(ctypes.Structure):
            _fields_ = [("Length", wintypes.USHORT), ("MaximumLength", wintypes.USHORT),
                        ("Buffer", wintypes.LPWSTR)]

        class ObjectAttributes(ctypes.Structure):
            _fields_ = [("Length", wintypes.ULONG), ("RootDirectory", wintypes.HANDLE),
                        ("ObjectName", ctypes.POINTER(UnicodeString)), ("Attributes", wintypes.ULONG),
                        ("SecurityDescriptor", wintypes.LPVOID),
                        ("SecurityQualityOfService", wintypes.LPVOID)]

        class IoStatusBlock(ctypes.Structure):
            _fields_ = [("Status", ctypes.c_long), ("Information", ctypes.c_size_t)]

        class AttributeTag(ctypes.Structure):
            _fields_ = [("FileAttributes", wintypes.DWORD), ("ReparseTag", wintypes.DWORD)]

        class HandleInformation(ctypes.Structure):
            _fields_ = [("dwFileAttributes", wintypes.DWORD), ("ftCreationTime", wintypes.FILETIME),
                        ("ftLastAccessTime", wintypes.FILETIME), ("ftLastWriteTime", wintypes.FILETIME),
                        ("dwVolumeSerialNumber", wintypes.DWORD), ("nFileSizeHigh", wintypes.DWORD),
                        ("nFileSizeLow", wintypes.DWORD), ("nNumberOfLinks", wintypes.DWORD),
                        ("nFileIndexHigh", wintypes.DWORD), ("nFileIndexLow", wintypes.DWORD)]

        ntdll = ctypes.WinDLL("ntdll", use_last_error=True)
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        create = ntdll.NtCreateFile
        create.argtypes = [ctypes.POINTER(wintypes.HANDLE), wintypes.DWORD, ctypes.POINTER(ObjectAttributes),
                           ctypes.POINTER(IoStatusBlock), wintypes.LPVOID, wintypes.ULONG, wintypes.ULONG,
                           wintypes.ULONG, wintypes.ULONG, wintypes.LPVOID, wintypes.ULONG]
        create.restype = ctypes.c_long
        set_information = ntdll.NtSetInformationFile
        set_information.argtypes = [wintypes.HANDLE, ctypes.POINTER(IoStatusBlock), wintypes.LPVOID,
                                    wintypes.ULONG, wintypes.ULONG]
        set_information.restype = ctypes.c_long
        status_error = ntdll.RtlNtStatusToDosError
        status_error.argtypes = [ctypes.c_long]
        status_error.restype = wintypes.ULONG
        open_path = kernel32.CreateFileW
        open_path.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
                              wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
        open_path.restype = wintypes.HANDLE
        attribute_tag = kernel32.GetFileInformationByHandleEx
        attribute_tag.argtypes = [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD]
        attribute_tag.restype = wintypes.BOOL
        information = kernel32.GetFileInformationByHandle
        information.argtypes = [wintypes.HANDLE, ctypes.POINTER(HandleInformation)]
        information.restype = wintypes.BOOL
        set_time = kernel32.SetFileTime
        set_time.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.FILETIME),
                             ctypes.POINTER(wintypes.FILETIME), ctypes.POINTER(wintypes.FILETIME)]
        set_time.restype = wintypes.BOOL
        close_handle = kernel32.CloseHandle
        close_handle.argtypes = [wintypes.HANDLE]
        close_handle.restype = wintypes.BOOL
        self._ctypes, self._msvcrt, self._wintypes = ctypes, msvcrt, wintypes
        self._structures = (UnicodeString, ObjectAttributes, IoStatusBlock, AttributeTag, HandleInformation)
        self._create, self._set_information, self._status_error = create, set_information, status_error
        self._open_path, self._attribute_tag, self._information = open_path, attribute_tag, information
        self._set_time, self._close_handle = set_time, close_handle
        self._invalid = ctypes.c_void_p(-1).value

    # -- the handles behind the C runtime's descriptors ----------------------

    def _handle(self, descriptor: int):
        return self._wintypes.HANDLE(self._msvcrt.get_osfhandle(descriptor))

    def _last_error(self) -> OSError:
        return self._ctypes.WinError(self._ctypes.get_last_error())

    def _attributes(self, handle) -> tuple[int, int]:
        """The attributes and the reparse tag of an open handle."""
        tag = self._structures[3]()
        if not self._attribute_tag(handle, self.FILE_ATTRIBUTE_TAG_INFO, self._ctypes.byref(tag),
                                   self._ctypes.sizeof(tag)):
            raise self._last_error()
        return tag.FileAttributes, tag.ReparseTag

    @staticmethod
    def _component(name: str) -> str:
        if (not name or name in (".", "..") or any(character in name for character in "/\\:")
                or name[-1] in " ." or any(ord(character) < 32 for character in name)):
            raise OSError(errno.EINVAL, "not a single file name", name)
        return name

    def _open(self, name: str, folder: int, access: int, disposition: int, options: int,
              crt_flags: int, *, reparse: bool = False) -> int:
        """A C runtime descriptor of `name` inside `folder`, never through a reparse point.

        `reparse` lets a reparse point be opened as itself (to describe or
        delete it); otherwise one is refused with ELOOP.
        """
        ctypes, wintypes = self._ctypes, self._wintypes
        unicode_string, object_attributes, io_status_block = self._structures[:3]
        text = self._component(name)
        size = len(text.encode("utf-16-le"))
        if size > 0xFFFF:
            raise OSError(errno.ENAMETOOLONG, "file name too long", text)
        buffer = ctypes.create_unicode_buffer(text)
        unicode = unicode_string(size, size, ctypes.cast(buffer, wintypes.LPWSTR))
        attributes = object_attributes(ctypes.sizeof(object_attributes), self._handle(folder),
                                       ctypes.pointer(unicode), self.OBJ_CASE_INSENSITIVE, None, None)
        handle, status = wintypes.HANDLE(), io_status_block()
        result = self._create(
            ctypes.byref(handle), access, ctypes.byref(attributes), ctypes.byref(status), None,
            self.FILE_ATTRIBUTE_NORMAL, self.FILE_SHARE_READ | self.FILE_SHARE_WRITE, disposition,
            options | self.FILE_OPEN_REPARSE_POINT | self.FILE_SYNCHRONOUS_IO_NONALERT, None, 0,
        )
        if result < 0:
            raise ctypes.WinError(self._status_error(result) or errno.EIO)
        try:
            if not reparse and self._attributes(handle)[0] & self.FILE_ATTRIBUTE_REPARSE_POINT:
                raise OSError(errno.ELOOP, "a reparse point is not followed", text)
            return self._msvcrt.open_osfhandle(handle.value, crt_flags)
        except BaseException:
            self._close_handle(handle)
            raise

    def _set(self, descriptor: int, buffer, length: int, information_class: int) -> None:
        status = self._structures[2]()
        result = self._set_information(self._handle(descriptor), self._ctypes.byref(status), buffer,
                                       length, information_class)
        if result < 0:
            raise self._ctypes.WinError(self._status_error(result) or errno.EIO)

    # -- the calls ------------------------------------------------------------

    def open_project(self, target: Path) -> int:
        """The project as its caller names it, as os.open does on POSIX."""
        handle = self._open_path(os.fspath(target), self.FOLDER_ACCESS,
                                 self.FILE_SHARE_READ | self.FILE_SHARE_WRITE, None,
                                 self.OPEN_EXISTING, self.FILE_FLAG_BACKUP_SEMANTICS, None)
        if handle in (None, self._invalid):
            raise self._last_error()
        try:
            if not self._attributes(handle)[0] & self.FILE_ATTRIBUTE_DIRECTORY:
                raise NotADirectoryError(errno.ENOTDIR, "not a folder", os.fspath(target))
            return self._msvcrt.open_osfhandle(handle, os.O_BINARY)
        except BaseException:
            self._close_handle(handle)
            raise

    def open_folder(self, name: str, folder: int) -> int:
        return self._open(name, folder, self.FOLDER_ACCESS, self.FILE_OPEN, self.FILE_DIRECTORY_FILE,
                          os.O_BINARY)

    def make_folder(self, name: str, folder: int) -> None:
        os.close(self._open(name, folder, self.FOLDER_ACCESS, self.FILE_CREATE, self.FILE_DIRECTORY_FILE,
                            os.O_BINARY))

    def lstat(self, name: str, folder: int) -> os.stat_result:
        descriptor = self._open(name, folder, self.FILE_READ_ATTRIBUTES | self.SYNCHRONIZE, self.FILE_OPEN,
                                0, os.O_BINARY, reparse=True)
        try:
            return self.fstat(descriptor)
        finally:
            os.close(descriptor)

    def open_file(self, name: str, folder: int) -> int:
        return self._open(name, folder, self.FILE_READ_DATA | self.FILE_READ_ATTRIBUTES | self.SYNCHRONIZE,
                          self.FILE_OPEN, self.FILE_NON_DIRECTORY_FILE, os.O_RDONLY | os.O_BINARY)

    def create_file(self, name: str, mode: int, folder: int) -> int:
        # GENERIC_WRITE covers the data, the flush and the times. SYNCHRONIZE is
        # named on its own: NtCreateFile checks the requested mask for it,
        # before generic rights are mapped, whenever synchronous I/O is asked
        # for, and answers STATUS_INVALID_PARAMETER without it.
        return self._open(name, folder, self.GENERIC_WRITE | self.FILE_READ_ATTRIBUTES | self.SYNCHRONIZE,
                          self.FILE_CREATE, self.FILE_NON_DIRECTORY_FILE, os.O_WRONLY | os.O_BINARY)

    def fstat(self, descriptor: int) -> os.stat_result:
        """Status from the handle: a link (S_IFLNK) for a name-surrogate reparse point."""
        handle = self._handle(descriptor)
        attributes, tag = self._attributes(handle)
        data = self._structures[4]()
        if not self._information(handle, self._ctypes.byref(data)):
            raise self._last_error()
        reparse = bool(attributes & self.FILE_ATTRIBUTE_REPARSE_POINT)
        if reparse and tag & self.NAME_SURROGATE:
            mode = stat.S_IFLNK | 0o777
        elif attributes & self.FILE_ATTRIBUTE_DIRECTORY:
            mode = stat.S_IFDIR | 0o777
        else:
            mode = stat.S_IFREG | (0o444 if attributes & self.FILE_ATTRIBUTE_READONLY else 0o666)

        def nanoseconds(filetime) -> int:
            value = (filetime.dwHighDateTime << 32) | filetime.dwLowDateTime
            return max(0, (value - self.FILETIME_EPOCH) * 100)

        atime, mtime, ctime = (nanoseconds(data.ftLastAccessTime), nanoseconds(data.ftLastWriteTime),
                               nanoseconds(data.ftCreationTime))
        return os.stat_result(
            (mode, (data.nFileIndexHigh << 32) | data.nFileIndexLow, data.dwVolumeSerialNumber,
             data.nNumberOfLinks, 0, 0, (data.nFileSizeHigh << 32) | data.nFileSizeLow,
             atime // 1_000_000_000, mtime // 1_000_000_000, ctime // 1_000_000_000),
            {"st_atime_ns": atime, "st_mtime_ns": mtime, "st_ctime_ns": ctime,
             "st_reparse_tag": tag if reparse else 0},
        )

    @staticmethod
    def chmod(descriptor: int, mode: int) -> None:
        """Windows has no POSIX mode bits; a new file takes its folder's ACL."""

    def set_times(self, descriptor: int, times: tuple[int, int]) -> None:
        stamps = []
        for value in times:
            ticks = max(0, value // 100 + self.FILETIME_EPOCH)
            stamps.append(self._wintypes.FILETIME(ticks & 0xFFFFFFFF, ticks >> 32))
        accessed, modified = stamps
        if not self._set_time(self._handle(descriptor), None, self._ctypes.byref(accessed),
                              self._ctypes.byref(modified)):
            raise self._last_error()

    def rename(self, name: str, new: str, folder: int) -> None:
        ctypes = self._ctypes
        descriptor = self._open(name, folder, self.DELETE | self.SYNCHRONIZE | self.FILE_READ_ATTRIBUTES,
                                self.FILE_OPEN, self.FILE_NON_DIRECTORY_FILE, os.O_BINARY)
        try:
            # FILE_RENAME_INFORMATION: ReplaceIfExists, padded to a handle;
            # RootDirectory; FileNameLength; FileName.
            encoded = self._component(new).encode("utf-16-le")
            root = ctypes.sizeof(self._wintypes.HANDLE)
            length = root + ctypes.sizeof(self._wintypes.HANDLE)
            offset = length + ctypes.sizeof(self._wintypes.ULONG)
            body = ctypes.create_string_buffer(offset + len(encoded))
            body[0] = b"\x01"
            ctypes.c_void_p.from_address(ctypes.addressof(body) + root).value = self._msvcrt.get_osfhandle(folder)
            ctypes.c_ulong.from_address(ctypes.addressof(body) + length).value = len(encoded)
            ctypes.memmove(ctypes.addressof(body) + offset, encoded, len(encoded))
            self._set(descriptor, body, len(body), self.FILE_RENAME_INFORMATION)
        finally:
            os.close(descriptor)

    def unlink(self, name: str, folder: int) -> None:
        descriptor = self._open(name, folder, self.DELETE | self.SYNCHRONIZE | self.FILE_READ_ATTRIBUTES,
                                self.FILE_OPEN, self.FILE_NON_DIRECTORY_FILE, os.O_BINARY, reparse=True)
        try:
            deleting = self._ctypes.c_ubyte(1)
            self._set(descriptor, self._ctypes.byref(deleting), 1, self.FILE_DISPOSITION_INFORMATION)
        finally:
            os.close(descriptor)

    @staticmethod
    def close(descriptor: int) -> None:
        os.close(descriptor)


def _handle_calls() -> _HandleCalls | None:
    """The NT handle calls on native Windows; None anywhere else, or where they cannot load."""
    if os.name != "nt":
        return None
    try:
        return _HandleCalls()
    except (ImportError, OSError, AttributeError):
        return None


_DESCRIPTOR_CALLS = _DescriptorCalls()
_HANDLE_CALLS = _handle_calls()


def _relative_calls():
    """The calls a walk makes on this platform, or None where it has none to make.

    Read at each walk, so a test can take the descriptors away
    (`_DESCRIPTOR_WALK`) and put other calls in the handles' place.
    """
    return _DESCRIPTOR_CALLS if _DESCRIPTOR_WALK else _HANDLE_CALLS


class UnsafePathError(InventoryError):
    """A project path that must not be read or written; the message says why."""


def _is_link(info: os.stat_result) -> bool:
    return stat.S_ISLNK(info.st_mode) or getattr(info, "st_reparse_tag", 0) in LINK_REPARSE_TAGS


def _folder_problem(name: str, info: os.stat_result) -> str | None:
    """Why the component `name` cannot be a folder a read or write passes, or None."""
    if _is_link(info):
        return f"{name} is a symbolic link; the accelerator does not read or write through it"
    if not stat.S_ISDIR(info.st_mode):
        return f"{name} is not a folder"
    return None


def _relative_parts(relative: str) -> list[str] | None:
    """The components of a project-relative path, or None for anything else.

    is_safe_relative_path's rules, checked on the string - the walks below
    run for every inventory file of every edition, and a PurePosixPath per
    call was most of their time: no absolute path, no backslash, NUL, empty,
    '.' or '..' component, and on Windows no drive-relative name.
    """
    if (
        not isinstance(relative, str)
        or not relative
        or "\\" in relative
        or "\x00" in relative
        or (os.name == "nt" and ":" in relative)
    ):
        return None
    parts = relative.split("/")
    if any(part in ("", ".", "..") for part in parts):
        return None
    return parts


def confinement_problem(target: Path, relative: str) -> str | None:
    """Why `relative` must not be read or written below `target`, or None.

    None: every folder on the way that exists is a real folder, and the file
    is a regular file or absent. The walk stops at the first missing
    component; a write creates the rest one level at a time.
    """
    parts = _relative_parts(relative)
    if parts is None:
        return "not a path inside the project"
    current = os.fspath(target)
    last = len(parts) - 1
    for depth, part in enumerate(parts):
        current = os.path.join(current, part)
        try:
            info = os.lstat(current)
        except FileNotFoundError:
            return None
        except OSError as error:
            return f"{'/'.join(parts[:depth + 1])} cannot be inspected: {error.strerror or error}"
        if depth == last:
            return None if stat.S_ISREG(info.st_mode) and not _is_link(info) else "not a regular file"
        if _is_link(info) or not stat.S_ISDIR(info.st_mode):
            return _folder_problem("/".join(parts[:depth + 1]), info)
    return None


def confined_files(target: Path, paths) -> set[str]:
    """Those of `paths` that are regular files below `target`, reached without passing a link.

    For the probes that ask this of a whole inventory - which edition is
    installed, which of its tools: a folder is looked at once, however many
    files share it. Nothing is written on this answer; every read and write
    walks its own path again.
    """
    base = os.fspath(target)
    folders: dict[str, bool] = {}
    found = set()
    for relative in paths:
        parts = _relative_parts(relative)
        if parts is None:
            continue
        current, folder, reachable = base, "", True
        for part in parts[:-1]:
            current = os.path.join(current, part)
            folder = f"{folder}/{part}" if folder else part
            real = folders.get(folder)
            if real is None:
                try:
                    info = os.lstat(current)
                except OSError:
                    real = False
                else:
                    real = stat.S_ISDIR(info.st_mode) and not _is_link(info)
                folders[folder] = real
            if not real:
                reachable = False
                break
        if not reachable:
            continue
        try:
            info = os.lstat(os.path.join(current, parts[-1]))
        except OSError:
            continue
        if stat.S_ISREG(info.st_mode) and not _is_link(info):
            found.add(relative)
    return found


def confined_file(target: Path, relative: str) -> bool:
    """A regular file below `target`, reached without passing a link."""
    return relative in confined_files(target, (relative,))


def _temporary_name(name: str) -> str:
    """A name beside `name` for the bytes that will replace it."""
    return f".{name}.accelerator-{os.getpid()}-{os.urandom(4).hex()}"


def _write_all(descriptor: int, data: bytes) -> None:
    view = memoryview(data)
    while view:
        view = view[os.write(descriptor, view):]


def _read_stream(calls, descriptor: int) -> tuple[bytes, os.stat_result]:
    """The bytes and status of an opened regular file; closes it."""
    with os.fdopen(descriptor, "rb") as handle:
        info = calls.fstat(handle.fileno())
        if not stat.S_ISREG(info.st_mode):
            raise UnsafePathError("not a regular file")
        return handle.read(), info


def _entry_problem(calls, folder: int, name: str, shown: str, error: OSError) -> str:
    """Why the entry `name` of `folder` did not open as a folder."""
    try:
        info = calls.lstat(name, folder)
    except OSError:
        info = None
    # Linux says ENOTDIR for a link opened with O_DIRECTORY|O_NOFOLLOW, other
    # systems ELOOP or EMLINK: the entry itself says what it is.
    problem = _folder_problem(shown, info) if info is not None else None
    return problem or f"{shown} cannot be inspected: {error.strerror or error}"


def _open_child(calls, folder: int, name: str, shown: str, create: bool) -> int | None:
    """A descriptor of the folder `name` inside `folder`, never through a link.

    With `create` a missing folder is made first; mkdirat never follows a link
    standing at the name it creates, and the new folder is opened as any
    other. None when it is missing and `create` is false.
    """
    try:
        return calls.open_folder(name, folder)
    except FileNotFoundError:
        if not create:
            return None
    except OSError as error:
        raise UnsafePathError(_entry_problem(calls, folder, name, shown, error)) from error
    try:
        calls.make_folder(name, folder)
    except FileExistsError:
        pass
    try:
        return calls.open_folder(name, folder)
    except OSError as error:
        raise UnsafePathError(_entry_problem(calls, folder, name, shown, error)) from error


def _open_folder(calls, target: Path, parts: list[str], create: bool) -> int | None:
    """A descriptor of the folder `parts` names below `target`, or None when it is missing.

    `target` itself is opened as the caller names it; below it, each
    component is opened relative to the descriptor of the one before it, so
    no name on the way is resolved twice. Raises UnsafePathError naming a
    component that is a link or not a folder. The caller closes the result.
    """
    folder = calls.open_project(target)
    for depth, part in enumerate(parts, 1):
        try:
            child = _open_child(calls, folder, part, "/".join(parts[:depth]), create)
        finally:
            calls.close(folder)
        if child is None:
            return None
        folder = child
    return folder


def _folder_moved(calls, target: Path, parts: list[str], folder: int) -> str | None:
    """Why `parts` below `target` no longer names the folder `folder` holds, or None."""
    shown = "/".join(parts) or "the project"
    try:
        current = _open_folder(calls, target, parts, create=False)
    except UnsafePathError as error:
        return str(error)
    except OSError as error:
        return f"{shown} cannot be inspected: {error.strerror or error}"
    if current is None:
        return f"{shown} was removed while it was written"
    try:
        same = os.path.samestat(calls.fstat(current), calls.fstat(folder))
    finally:
        calls.close(current)
    return None if same else f"{shown} was replaced while it was written"


def _not_regular(calls, folder: int, name: str) -> bool:
    """The entry `name` of `folder` is a link or anything but a regular file."""
    try:
        info = calls.lstat(name, folder)
    except OSError:
        return False
    return _is_link(info) or not stat.S_ISREG(info.st_mode)


def _read_at(calls, target: Path, parts: list[str]) -> tuple[bytes, os.stat_result] | None:
    """`_read_confined` by folder descriptors or handles."""
    folder = _open_folder(calls, target, parts[:-1], create=False)
    if folder is None:
        return None
    try:
        try:
            descriptor = calls.open_file(parts[-1], folder)
        except FileNotFoundError:
            return None
        except OSError as error:
            if _not_regular(calls, folder, parts[-1]):
                raise UnsafePathError("not a regular file") from error
            raise
    finally:
        calls.close(folder)
    return _read_stream(calls, descriptor)


def _read_by_path(
    target: Path, relative: str, parts: list[str]
) -> tuple[bytes, os.stat_result] | None:
    """`_read_confined` where nothing can walk: look at each component, then open.

    Nothing is written there (NO_SAFE_WRITE), so a link swapped in between
    can show the sync another file but cannot take anything it writes.
    """
    problem = confinement_problem(target, relative)
    if problem is not None:
        raise UnsafePathError(problem)
    try:
        descriptor = os.open(os.path.join(os.fspath(target), *parts), _READ_FLAGS)
    except FileNotFoundError:
        return None
    except OSError as error:
        if error.errno == errno.ELOOP:
            raise UnsafePathError("not a regular file") from error
        raise
    return _read_stream(_DESCRIPTOR_CALLS, descriptor)


def _read_confined(target: Path, relative: str) -> tuple[bytes, os.stat_result] | None:
    """The bytes and status of a regular file below `target`; None when it is absent.

    Raises UnsafePathError for a path that must not be used.
    """
    parts = _relative_parts(relative)
    if parts is None:
        raise UnsafePathError("not a path inside the project")
    calls = _relative_calls()
    if calls is not None:
        return _read_at(calls, target, parts)
    return _read_by_path(target, relative, parts)


def read_confined(target: Path, relative: str) -> bytes | None:
    """The bytes of a regular file below `target`; None when it is absent.

    Raises UnsafePathError for a path the sync must not use.
    """
    found = _read_confined(target, relative)
    return None if found is None else found[0]


def _write_at(
    calls,
    target: Path,
    relative: str,
    parts: list[str],
    data: bytes,
    mode: int | None,
    times: tuple[int, int] | None,
    durable: bool,
) -> None:
    """`write_confined` by folder descriptors or handles."""
    try:
        folder = _open_folder(calls, target, parts[:-1], create=True)
    except FileNotFoundError as error:
        # A folder on the way went away while the next one was made in it.
        problem = confinement_problem(target, relative)
        if problem is not None:
            raise UnsafePathError(problem) from error
        raise
    name = parts[-1]
    temporary = None
    try:
        try:
            existing = calls.lstat(name, folder)
        except FileNotFoundError:
            existing = None
        if existing is not None:
            if _is_link(existing) or not stat.S_ISREG(existing.st_mode):
                raise UnsafePathError("not a regular file")
            if mode is None:
                mode = stat.S_IMODE(existing.st_mode)
        candidate = _temporary_name(name)
        # Private until it holds its bytes and the mode it ends with; a new
        # file without a mode of its own gets the umask's, as any new file.
        descriptor = calls.create_file(candidate, 0o600 if mode is not None else 0o666, folder)
        temporary = candidate
        try:
            _write_all(descriptor, data)
            if mode is not None:
                calls.chmod(descriptor, mode)
            if times is not None:
                calls.set_times(descriptor, times)
            if durable:
                os.fsync(descriptor)
        finally:
            calls.close(descriptor)
        moved = _folder_moved(calls, target, parts[:-1], folder)
        if moved is not None:
            raise UnsafePathError(moved)
        calls.rename(temporary, name, folder)
        temporary = None
    except FileNotFoundError as error:
        # The folder held was removed while it was written.
        moved = _folder_moved(calls, target, parts[:-1], folder)
        if moved is not None:
            raise UnsafePathError(moved) from error
        raise
    finally:
        if temporary is not None:
            try:
                calls.unlink(temporary, folder)
            except OSError:
                pass
        calls.close(folder)


def write_confined(
    target: Path,
    relative: str,
    data: bytes,
    mode: int | None = None,
    times: tuple[int, int] | None = None,
    *,
    durable: bool = True,
) -> None:
    """Create or replace a regular file below `target`, never through a link.

    The bytes go to a temporary file beside it, created exclusively so that a
    link planted under its name cannot take them, and are then renamed over
    the file: a write that fails or is interrupted leaves the old file whole.
    `durable` also flushes them to disk before the rename, so that a crash
    cannot leave the file empty either; the sync turns it off for files it
    can write again from the release. `mode` defaults to the replaced file's,
    and to the umask's for a new one; `times` are (atime, mtime) in
    nanoseconds. Raises UnsafePathError when a link or a non-folder stands on
    the path, when a folder on it is swapped while the file is written, and
    on a platform that can walk a path neither by descriptors nor by handles
    (see the note above `LINK_REPARSE_TAGS`).
    """
    parts = _relative_parts(relative)
    if parts is None:
        raise UnsafePathError("not a path inside the project")
    calls = _relative_calls()
    if calls is None:
        raise UnsafePathError(NO_SAFE_WRITE)
    _write_at(calls, target, relative, parts, data, mode, times, durable)


# An install that merged into a project's file wrote the merge over it in
# place: a write that failed part way - a full disk, an interrupted run -
# left a truncated `.mcp.json` (`{`) where the team's servers had been, and
# every later install refused to merge into it. A file an install replaces is
# now written whole beside it and renamed over it (write_confined), and one
# that fails part way puts back, newest first, the files it had already
# replaced. Files it created are left: they hold only the release, and the
# next run finds them identical.


def replace_project_file(
    target: Path,
    label: tuple[str, str],
    relative: str,
    data: bytes,
    replaced: list,
    mode: int | None = None,
    times: tuple[int, int] | None = None,
) -> None:
    """Write `data` as `relative` below `target` in one step; record what it replaced.

    `label` is the (component, inventory path) the install reports it under.
    A replaced file keeps its mode unless `mode` says otherwise, and goes
    into `replaced` with its bytes and status for put_back.
    """
    found = _read_confined(target, relative)
    write_confined(target, relative, data, mode, times)
    if found is not None:
        replaced.append((label, relative, found[0], found[1], data))


def put_back(target: Path, replaced: list) -> None:
    """Restore the files a failed install replaced, newest first.

    A file that no longer holds what the install wrote was changed by
    someone else since, and stays as it is. Each file is reported as
    RESTORED (stdout, the action log) or NOT_RESTORED with the reason
    (stderr); the reports follow the work, so a closed stream cannot stop it.
    """
    outcomes = []
    for (component, path), relative, previous, info, written in reversed(replaced):
        try:
            if read_confined(target, relative) != written:
                outcomes.append((component, path, relative, "changed after the install wrote it"))
                continue
            write_confined(
                target, relative, previous, stat.S_IMODE(info.st_mode),
                (info.st_atime_ns, info.st_mtime_ns),
            )
            outcomes.append((component, path, relative, None))
        except (InventoryError, OSError) as error:
            outcomes.append((component, path, relative, str(error)))
    for component, path, relative, problem in outcomes:
        try:
            if problem is None:
                print(f"RESTORED\t{component}\t{path}\t{relative}")
            else:
                print(f"NOT_RESTORED\t{component}\t{path}\t{relative}\t{problem}", file=sys.stderr)
        except OSError:
            pass


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
    has, the managed blocks of AGENTS.md and .claude/CLAUDE.md, and the
    memory server's own entry in an MCP configuration that also holds the
    project's servers. The runtime is also replaced over a local edit, which
    is backed up first under memory-bank/local. Never written: anything the
    project's Git tracks, seeded state the project owns, any path - backups
    and the sync's own record included - that a symbolic link or a
    non-folder inside the project stands on, including one swapped in while
    the sync runs (read_confined, write_confined), anything at all on a
    platform that can walk a path neither by folder descriptors nor by
    handles (NO_SAFE_WRITE), and - unless `rewire_codex` says the caller
    re-approves it - the Codex hook wiring, whose trust is a hash of its
    definitions. Each of those is reported instead.

    A release is never applied in part over what its files run (see
    HOOK_WIRING): while a runtime file stays at the project's version, the
    release's other files are held back too, and a tool's new hooks wait for
    its wiring, as the wiring waits for them. Held-back files are reported
    under `kept`, and `partial` then says why the project is not at the
    release; it is None when nothing was held back.
    """
    report: dict = {
        "target": str(target), "edition": None, "release": None, "changed": [],
        "kept": [], "backups": [], "dry_run": dry_run, "error": None, "partial": None,
    }
    # A runtime reached only through a link is not this project's: the
    # Harness, which reads it without following links, does not count it as
    # installed either.
    problem = confinement_problem(target, SYNC_RUNTIME_MARKER)
    if problem is not None or not confined_file(target, SYNC_RUNTIME_MARKER):
        report["error"] = "not an installed accelerator" + (
            f" inside the project ({problem})" if problem else ""
        )
        return report
    edition = installed_edition(root, target)
    if edition is None:
        report["error"] = "the installed edition could not be determined"
        return report
    data = load_inventory(edition, root)
    report.update(edition=edition, release=data["release"])
    existing = confined_files(target, (path for paths in data["installed"].values() for path in paths))
    present = {
        component
        for component, paths in data["installed"].items()
        if component == "shared" or any(path in existing for path in paths)
    }
    tracked = tracked_paths(target)
    record_problem = None
    try:
        recorded = read_confined(target, SYNC_MANIFEST)
        manifest = json.loads(recorded.decode("utf-8")) if recorded is not None else None
    except UnsafePathError as error:
        record_problem, manifest = str(error), None
    except (OSError, ValueError):
        manifest = None
    known = manifest.get("files") if isinstance(manifest, dict) else None
    known = known if isinstance(known, dict) else {}
    history = released_blob_ids(root, edition)
    executable_bits = source_executable_bits(root, edition)
    stamp = stamp or datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    written: dict[str, str] = {}
    # The release's MCP merger and this machine's Python, loaded on first use,
    # or why they are unavailable.
    memory_mcp: tuple | str | None = None

    def keep(path: str, reason: str) -> None:
        report["kept"].append({"path": path, "reason": reason})

    # What the release would write, in the order found: (path, payload,
    # action, executable, backup, record, durable). Written only once every
    # file has been looked at, so that a file whose runtime or wiring stays
    # behind can be held back (HOOK_WIRING).
    pending: list[tuple] = []
    # Release files the project does not have (or cannot be read safely).
    absent: set[str] = set()

    def write(
        path: str,
        payload: bytes,
        action: str,
        executable: bool,
        backup: tuple[bytes, os.stat_result] | None = None,
        record: bool = True,
        durable: bool = False,
    ) -> None:
        """Plan one write; `backup` is the content it replaces and that
        file's status, as it was read, saved first."""
        if backup is not None:
            unsafe = confinement_problem(target, f"{SYNC_BACKUP_DIR}/{stamp}/{path}")
            if unsafe is not None:
                # Without its backup a local edit would be lost: it stays.
                keep(path, f"not replaced: its backup cannot be written inside the project ({unsafe})")
                return
        pending.append((path, payload, action, executable, backup, record, durable))

    def apply(
        path: str,
        payload: bytes,
        action: str,
        executable: bool,
        backup: tuple[bytes, os.stat_result] | None,
        record: bool,
        durable: bool,
    ) -> bool:
        """Write one planned file; False when it stays as it is.

        Flushed to disk before it replaces anything only where a crash could
        lose what exists nowhere else: a backup, the only copy of a person's
        edit once the file is replaced, and a file merged with the project's
        own content (`durable`). The release's files and the sync's record are
        written again by the next sync; a flush costs about 10 ms a file on
        btrfs, so they go without one.
        """
        if backup is not None:
            saved = f"{SYNC_BACKUP_DIR}/{stamp}/{path}"
            unsafe = confinement_problem(target, saved)
            if unsafe is None and not dry_run:
                content, original = backup
                try:
                    write_confined(
                        target, saved, content, stat.S_IMODE(original.st_mode),
                        (original.st_atime_ns, original.st_mtime_ns), durable=True,
                    )
                except UnsafePathError as error:
                    unsafe = str(error)
            if unsafe is not None:
                keep(path, f"not replaced: its backup cannot be written inside the project ({unsafe})")
                return False
            if not dry_run:
                report["backups"].append(saved)
        if not dry_run:
            try:
                write_confined(
                    target, path, payload, EXECUTABLE_INSTALL_MODE if executable else None,
                    durable=durable,
                )
            except UnsafePathError as error:
                keep(path, str(error))
                return False
        report["changed"].append({"path": path, "action": action})
        if record:
            written[path] = git_blob_id(payload)
        return True

    def memory_config(path: str, previous: bytes | None, payload: bytes) -> bytes:
        nonlocal memory_mcp
        if memory_mcp is None:
            try:
                module = load_memory_mcp(root, edition)
                memory_mcp = (module, module.python_command())
            except (ImportError, OSError, SyntaxError, ValueError) as error:
                memory_mcp = str(error) or type(error).__name__
        if isinstance(memory_mcp, str):
            raise ValueError(memory_mcp)
        module, python = memory_mcp
        return module.merge(path, previous, payload, python)

    def sync_memory_config(
        path: str, current: bytes | None, payload: bytes, pristine: bool, executable: bool
    ) -> None:
        """The memory server's entry follows the release; the project's servers stay.

        Install merges these files rather than copying them, so an installed
        one holding a team's server is not the release's bytes. Treated as an
        edited file, it kept the memory server it was installed with forever.
        """
        merged_into_project = False
        try:
            # The release's file as this machine's install would write it.
            whole = memory_config(path, None, payload)
            if current is None or pristine or current == whole:
                merged = whole
            elif path == CODEX_CONFIG and MCP_CODEX_BEGIN not in current:
                # A Codex configuration without the managed block is the
                # project's own; install leaves it alone, and so does a sync.
                keep(path, "edited in the project")
                return
            else:
                merged, merged_into_project = memory_config(path, current, payload), True
        except ValueError as error:
            # Malformed, or the memory server's name taken by another server
            # or an edited managed block: nothing to merge into safely.
            keep(path, f"memory MCP configuration not merged: {error}")
            return
        unchanged = merged == current or (
            merged_into_project and path != CODEX_CONFIG and json.loads(merged) == json.loads(current)
        )
        if unchanged:
            if merged_into_project:
                keep(path, "edited in the project")
            else:
                written[path] = git_blob_id(current)
            return
        if current is None:
            action = "added"
        else:
            action = "memory server updated" if merged_into_project else "updated"
        # A merge holds the project's own servers, so it is not recorded as an
        # untouched release: the next sync merges into it again rather than
        # replacing it with the release's file.
        write(path, merged, action, executable, record=not merged_into_project, durable=merged_into_project)

    for component, path in selected_files(data, [c for c in TOOLS if c in present]):
        if path in SEED_ONLY_PATHS or path in SYNC_SKIPPED_FILES:
            continue
        source_path = data["source_overrides"].get(path, path)
        source = root / edition_path(edition) / PurePosixPath(source_path)
        payload = source.read_bytes()
        executable = installs_executable(path, source, source_path, executable_bits)
        try:
            found = _read_confined(target, path)
        except UnsafePathError as error:
            absent.add(path)
            keep(path, str(error))
            continue
        current = None if found is None else found[0]
        exists = current is not None
        if not exists:
            absent.add(path)
        if current == payload:
            written[path] = git_blob_id(payload)
            continue
        if path in tracked:
            keep(path, "tracked by the project's Git: update it through a commit")
            continue
        if path in TRUST_BOUND_FILES and exists and not rewire_codex:
            keep(path, "Codex hook wiring: a change needs re-approval in Codex /hooks")
            continue
        blob = git_blob_id(current) if exists else None
        pristine = exists and (
            known.get(path) == blob
            or (history is not None and blob in history.get(source_path, set()))
        )
        if path in MCP_CONFIG_FILES and (path != CODEX_CONFIG or MCP_CODEX_BEGIN in payload):
            sync_memory_config(path, current, payload, pristine, executable)
            continue
        if not exists:
            write(path, payload, "added", executable)
            continue
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
                    write(path, merged, "managed block updated", executable, durable=True)
                else:
                    written[path] = git_blob_id(current)
                continue
            first = source_text.splitlines()[0] if source_text else ""
            if pristine:
                write(path, payload, "updated", executable)
            elif first and text.startswith(first):
                # The accelerator's own policy, edited in place: the edit is
                # kept in the backup and the current policy goes live.
                write(path, payload, "updated over a local edit", executable, backup=found)
            elif path == ".claude/CLAUDE.md":
                write(
                    path,
                    merge_agents_file(text, source_text).encode("utf-8"),
                    "import added",
                    executable,
                    durable=True,
                )
            else:
                keep(path, "the project's own file")
            continue
        if path in ADDITIVE_FILES:
            keep(path, "the project's own file")
            continue
        if pristine:
            write(path, payload, "updated", executable)
        elif is_runtime_file(path):
            write(path, payload, "updated over a local edit", executable, backup=found)
        else:
            keep(path, "edited in the project")
    apply_release(pending, absent, report, keep, apply)
    if record_problem is not None:
        keep(SYNC_MANIFEST, f"the sync's record is neither read nor written ({record_problem})")
    elif not dry_run:
        try:
            write_confined(
                target,
                SYNC_MANIFEST,
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
                durable=False,
            )
        except UnsafePathError as error:
            keep(SYNC_MANIFEST, f"the sync's record is not written ({error})")
    return report


def apply_release(pending: list[tuple], absent: set[str], report: dict, keep, apply) -> None:
    """Write the planned files, holding back what would run against files left behind.

    `pending` holds the planned writes, `absent` the release files the project
    lacks; `keep` reports a file left as it is and `apply` writes one, False
    when it could not. See HOOK_WIRING for why a file is held back.
    """
    left = {item["path"]: item["reason"] for item in report["kept"]}
    held: dict[str, str] = {}

    def hold(path: str, reason: str) -> None:
        held.setdefault(path, f"{HELD_BACK}: {reason}")

    def hold_hooks_of(wiring: str, reason: str) -> None:
        for item in pending:
            if item[0] in absent and hook_wiring_of(item[0]) == wiring:
                hold(item[0], reason)

    stale = sorted(path for path in left if is_core_runtime_file(path))
    if stale:
        for item in pending:
            hold(item[0], f"the runtime stays at the project's version ({stale[0]}: {left[stale[0]]})")
    planned = {item[0] for item in pending}
    for wiring in HOOK_WIRING:
        if wiring in left:
            hold_hooks_of(wiring, f"the hook wiring {wiring} stays at the project's version ({left[wiring]})")
            continue
        missing = sorted(path for path in left if path in absent and hook_wiring_of(path) == wiring)
        if missing and wiring in planned:
            reason = f"a new hook it would run is not written ({missing[0]}: {left[missing[0]]})"
            hold(wiring, reason)
            hold_hooks_of(wiring, f"its wiring {wiring} is held back: {reason}")
    # The runtime first: a part of it that cannot be written holds back the
    # rest, which would otherwise mix two releases' modules. A tool's new
    # hooks before everything else, its wiring last: the wiring waits for
    # them. Where nothing can be written at all (NO_SAFE_WRITE) every write
    # is refused on its own and none is half done, so each keeps its reason.
    can_write = _relative_calls() is not None

    def rank(item: tuple) -> int:
        path = item[0]
        if is_core_runtime_file(path):
            return 0
        if path in HOOK_WIRING:
            return 3
        return 1 if path in absent and hook_wiring_of(path) is not None else 2

    order = sorted(pending, key=rank)
    held_back = 0
    for index, item in enumerate(order):
        path = item[0]
        if path in held:
            keep(path, held[path])
            held_back += 1
            continue
        if apply(*item) or not can_write:
            continue
        reason = next(entry["reason"] for entry in reversed(report["kept"]) if entry["path"] == path)
        if is_core_runtime_file(path):
            for later in order[index + 1:]:
                hold(later[0], f"the runtime is not fully written ({path}: {reason})")
        elif rank(item) == 1:
            wiring = hook_wiring_of(path)
            reason = f"a new hook it would run is not written ({path}: {reason})"
            hold(wiring, reason)
            hold_hooks_of(wiring, f"its wiring {wiring} is held back: {reason}")
    if held_back:
        report["partial"] = (
            f"{held_back} file(s) of release {report['release']} held back: what they run "
            "stays at the project's version here; each is listed under kept"
        )


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
