"""Executable bits as Git records them, with the filesystem as the fallback.

A hook that a client runs as a direct command - every script wired in
`.claude/settings.json`, `.cursor/hooks.json` or `.codex/hooks.json` - must be
executable, or the client gets exit status 126 and whatever the hook was
supposed to do (release a lock, block a command) silently does not happen.

The executable bit that ships is the one in the Git index. The filesystem is
not a reliable witness for it: a checkout on Windows, or any checkout with
`core.fileMode=false`, has no usable bit on disk, so a tool that reads only
the disk mirrors or copies every hook there as non-executable, and a tool that
writes only the disk changes nothing Git will commit. The tools that copy
files between trees therefore read the bit from the index first, fall back to
the filesystem only for a path the index does not know (a synthetic tree
outside any repository, a mirror created a moment ago), and when they repair
a mode they repair it in both places.

Used by `build_mirrors.py` and `asset_parity.py`. `install_accelerator.py`
reads the index itself: the Harness runs that script as a standalone copy, so
it cannot import a sibling module.

Python 3 standard library and Git only.
"""

from __future__ import annotations

import os
import stat
import subprocess
from pathlib import Path

EXECUTABLE_MODE = "100755"
REGULAR_MODE = "100644"
# Keeps one `git update-index` call comfortably inside any platform's
# command-line and pipe limits even when a whole tree needs repairing.
_INDEX_BATCH = 200


class FileModeError(Exception):
    """The Git index could not be updated."""


def _git(base: Path, *args: str, stdin: bytes | None = None):
    try:
        return subprocess.run(
            ["git", *args], cwd=str(base), input=stdin, capture_output=True
        )
    except OSError:
        return None


class FileModes:
    """Executable bits for every file under one directory.

    ``is_executable`` answers with the index bit for a tracked path and the
    filesystem bit otherwise - unless the filesystem has no trustworthy bit
    (Windows, or a repository configured with ``core.fileMode=false``), in
    which case an untracked path answers ``None``: nothing can tell, so
    callers skip the comparison instead of inventing drift.
    """

    def __init__(self, base: Path) -> None:
        self.base = base
        # ``base``-relative POSIX path -> (index mode, blob id), stage 0 only.
        self.index: dict[str, tuple[str, str]] = {}
        self.prefix = ""
        self.trust_filesystem = os.name != "nt"
        # Index mode changes queued by set_executable, written by flush.
        self._pending: list[tuple[str, str, str]] = []
        if not base.is_dir():
            return
        prefix = _git(base, "rev-parse", "--show-prefix")
        if prefix is None or prefix.returncode != 0:
            return
        self.prefix = prefix.stdout.decode("utf-8", "surrogateescape").strip("\n")
        # --full-name keeps the paths top-level relative, which is what
        # `update-index --index-info` expects; the prefix is stripped for
        # lookups so callers work in paths relative to ``base``.
        listed = _git(base, "ls-files", "--stage", "-z", "--full-name", "--", ".")
        if listed is None or listed.returncode != 0:
            return
        for record in listed.stdout.split(b"\0"):
            meta, tab, raw = record.partition(b"\t")
            fields = meta.split()
            if not tab or len(fields) != 3 or fields[2] != b"0":
                # Unmerged entries carry no single mode to compare against.
                continue
            full = raw.decode("utf-8", "surrogateescape")
            if not full.startswith(self.prefix):
                continue
            self.index[full[len(self.prefix) :]] = (
                fields[0].decode("ascii"),
                fields[1].decode("ascii"),
            )
        config = _git(base, "config", "--bool", "core.fileMode")
        if (
            config is not None
            and config.returncode == 0
            and config.stdout.strip() == b"false"
        ):
            self.trust_filesystem = False

    def relative(self, path: Path) -> str:
        return path.relative_to(self.base).as_posix()

    def is_executable(self, path: Path) -> bool | None:
        """The bit Git commits for ``path``, or None when nothing can tell."""
        entry = self.index.get(self.relative(path))
        if entry is not None:
            return entry[0] == EXECUTABLE_MODE
        if not self.trust_filesystem:
            return None
        try:
            return bool(path.stat().st_mode & stat.S_IXUSR)
        except OSError:
            return None

    def set_executable(self, path: Path, executable: bool) -> bool:
        """Make ``path`` carry the bit on disk and in the index.

        The disk is touched only where it has a trustworthy bit; the index
        only for a tracked path, and only its mode: the entry keeps the blob
        it already records, so a content change the caller made is left
        unstaged rather than smuggled into the index (which is what
        `git update-index --chmod` would do). Returns True when anything
        changed.
        """
        changed = False
        if self.trust_filesystem:
            current = stat.S_IMODE(path.stat().st_mode)
            if executable:
                # Executable wherever the file is readable, as `chmod +x`
                # under a typical umask and as Git checks it out.
                wanted = current | ((current & 0o444) >> 2)
            else:
                wanted = current & ~0o111
            if wanted != current:
                os.chmod(path, wanted)
                changed = True
        rel = self.relative(path)
        entry = self.index.get(rel)
        mode = EXECUTABLE_MODE if executable else REGULAR_MODE
        if entry is not None and entry[0] != mode:
            self._pending.append((rel, mode, entry[1]))
            changed = True
        return changed

    def flush(self) -> None:
        """Write every queued index mode change; raise FileModeError on failure."""
        while self._pending:
            batch = self._pending[:_INDEX_BATCH]
            del self._pending[:_INDEX_BATCH]
            payload = b"".join(
                f"{mode} {blob}\t{self.prefix}{rel}".encode("utf-8", "surrogateescape")
                + b"\0"
                for rel, mode, blob in batch
            )
            result = _git(self.base, "update-index", "-z", "--index-info", stdin=payload)
            if result is None or result.returncode != 0:
                reason = (
                    "git is not available"
                    if result is None
                    else result.stderr.decode("utf-8", "replace").strip()
                )
                raise FileModeError(
                    f"cannot record executable bits in the Git index: {reason}"
                )
            for rel, mode, blob in batch:
                self.index[rel] = (mode, blob)
