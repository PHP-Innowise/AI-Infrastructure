"""Composer @-mentions: the workspace's files and folders, as the CLIs' own `@` search offers them.

Claude Code, Codex and Cursor open a file search when `@` starts a word in their terminal composer and insert the
chosen path. The Harness runs them without that terminal, so the page asks here. The paths are the workspace's
files as Git sees them (tracked, plus untracked ones `.gitignore` does not exclude), or a bounded walk outside Git,
and the folders that hold them. Only names are read, never contents. The chosen path goes into the message as
`@path`, as the CLI's own composer would put it there, and the CLI treats it as it treats any mention.
"""
from __future__ import annotations

import os
from pathlib import Path, PurePosixPath
import re
import threading
import time

from .sessions import SessionError, run_git

FRESH_SECONDS = 30
# Paths kept from one workspace; a larger tree is searched in its first LIMIT paths and says so.
LIMIT = 50_000
SHOWN = 50
QUERY_CHARACTERS = 300
GIT_SECONDS = 10
# Outside Git: the folders a walk never enters (dependencies, caches, tool state).
SKIPPED = frozenset({'.git', '.hg', '.svn', 'node_modules', 'vendor', '.venv', 'venv', '__pycache__', '.idea',
                     '.vscode', '.cache', '.pytest_cache', '.mypy_cache', 'dist', 'build'})
WORD = re.compile(r'[/._-]')


def _git_paths(root):
    try:
        result = run_git(root, 'ls-files', '-z', '--cached', '--others', '--exclude-standard', timeout=GIT_SECONDS)
    except SessionError:
        return None
    if result.returncode != 0:
        return None
    # A file that is both tracked and changed on disk is listed once.
    return list(dict.fromkeys(path for path in result.stdout.split('\0') if path))[:LIMIT]


def _walked_paths(root):
    paths = []
    for current, folders, files in os.walk(root):
        folders[:] = sorted(name for name in folders if name not in SKIPPED)
        relative = Path(current).relative_to(root)
        for name in sorted(files):
            paths.append((relative / name).as_posix())
            if len(paths) >= LIMIT:
                return paths
    return paths


def entries(paths):
    """Each file, then each folder that holds one, as `folder/`."""
    folders = {}
    for path in paths:
        parts = PurePosixPath(path).parts
        for depth in range(1, len(parts)):
            folders.setdefault('/'.join(parts[:depth]) + '/', None)
    return [{'path': path, 'kind': 'file'} for path in paths] + [{'path': path, 'kind': 'folder'} for path in folders]


def _subsequence(needle, text):
    position = 0
    for character in needle:
        position = text.find(character, position) + 1
        if not position:
            return False
    return True


def rank(items, query, limit=SHOWN):
    """The best matches first, as the CLIs' file search orders them.

    Without a query, the top of the tree: its folders, then its files. With one, a name that starts with it, then a
    word inside the path that does, then a name or a path that contains it, then a path that holds its letters in
    order. A query with a slash (`src/bil`, `src/`) first offers what is inside that folder, and a query that ends
    with one lists that folder as a directory listing does. Otherwise shorter paths win a tie.
    """
    needle = query.lower()
    if not needle:
        top = [item for item in items if '/' not in item['path'].rstrip('/')]
        return sorted(top, key=lambda item: (item['kind'] != 'folder', item['path'].lower()))[:limit]
    folder, _, name = needle.rpartition('/')
    ranked = []
    for item in items:
        path = item['path'].lower()
        if path == needle:
            continue  # the folder just typed: what is inside it is offered instead
        bare = path.rstrip('/')
        parent, _, base = bare.rpartition('/')
        if folder and parent == folder and base.startswith(name):
            order = 0
        elif folder and parent == folder and name in base:
            order = 1
        elif not folder and base.startswith(needle):
            order = 0
        elif not folder and any(word.startswith(needle) for word in WORD.split(bare)):
            order = 1
        elif needle in base:
            order = 2
        elif needle in path:
            order = 3
        elif _subsequence(needle, path):
            order = 4
        else:
            continue
        # A folder's own contents read as a listing: its folders, then its files, by name. A search prefers short paths.
        listing = bool(folder) and not name and order == 0
        ranked.append((order, bare.count('/'), (item['kind'] != 'folder', 0) if listing else (False, len(bare)), bare, item))
    ranked.sort(key=lambda entry: entry[:4])
    return [entry[4] for entry in ranked[:limit]]


class Mentions:
    """Each workspace's paths, listed at most every FRESH_SECONDS, one listing at a time per workspace."""

    def __init__(self):
        self.lock = threading.Lock()
        self.entries = {}
        self.locks = {}

    def _items(self, root):
        key = os.path.normcase(str(root))
        with self.lock:
            entry, lock = self.entries.get(key), self.locks.setdefault(key, threading.Lock())
        if entry and time.monotonic() < entry[0]:
            return entry[1], entry[2]
        with lock:
            with self.lock:
                entry = self.entries.get(key)
            if entry and time.monotonic() < entry[0]:
                return entry[1], entry[2]
            paths = _git_paths(root)
            if paths is None:
                paths = _walked_paths(root)
            items, truncated = entries(paths), len(paths) >= LIMIT
            with self.lock:
                if len(self.entries) > 32:
                    self.entries.clear()
                    self.locks = {key: lock}
                self.entries[key] = (time.monotonic() + FRESH_SECONDS, items, truncated)
            return items, truncated

    def search(self, root, query):
        if not isinstance(query, str) or len(query) > QUERY_CHARACTERS or any(ord(character) < 32 for character in query):
            raise SessionError('Invalid file search.')
        items, truncated = self._items(Path(root))
        return {'items': rank(items, query.strip().lstrip('@')), 'truncated': truncated}
