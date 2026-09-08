"""Bounded, read-only folder discovery for the local project picker."""
from collections import deque
import os
from pathlib import Path
import time

from harness.sessions import Sessions, SessionError, open_project_path

MAX_ENTRIES = 20000
MAX_RESULTS = 200
MAX_DEPTH = 8
MAX_SECONDS = 1.5
SEARCH_SKIP = {'.git', 'node_modules', 'vendor', '__pycache__', '.venv', 'venv'}


def browse_projects(data):
    if not isinstance(data, dict) or set(data) - {'path', 'query', 'hidden'}:
        raise SessionError('Folder browsing accepts only path, query and hidden.')
    query = data.get('query', '')
    hidden = data.get('hidden', False)
    if (not isinstance(query, str) or len(query) > 100
            or any(ord(c) < 32 or ord(c) == 127 for c in query)
            or type(hidden) is not bool):
        raise SessionError('Enter a folder name of up to 100 characters and a boolean hidden option.')
    root = Sessions._project_folder(data.get('path', str(Path.home())))
    query = query.strip().casefold()
    queue = deque([(root, 0)])
    entries, examined, skipped, truncated = [], 0, 0, False
    deadline = time.monotonic() + MAX_SECONDS
    # ponytail: bounded synchronous scan; narrow the root when limits are reached.
    while queue:
        if examined >= MAX_ENTRIES or len(entries) >= MAX_RESULTS or time.monotonic() >= deadline:
            truncated = True
            break
        folder, depth = queue.popleft()
        try:
            descriptor = open_project_path(folder, '.', directory=True)
            try:
                with os.scandir(descriptor) as children:
                    for child in children:
                        if examined >= MAX_ENTRIES or len(entries) >= MAX_RESULTS or time.monotonic() >= deadline:
                            truncated = True
                            break
                        examined += 1
                        if not hidden and child.name.startswith('.'):
                            continue
                        path = folder / child.name
                        if len(str(path)) > 1024 or any(ord(c) < 32 or ord(c) == 127 for c in child.name):
                            continue
                        try:
                            if not child.is_dir(follow_symlinks=False):
                                continue
                        except OSError:
                            skipped += 1
                            continue
                        if not query or query in child.name.casefold():
                            entries.append({'name': child.name, 'path': str(path),
                                            'relative': str(path.relative_to(root))})
                        if query and child.name not in SEARCH_SKIP:
                            if depth + 1 < MAX_DEPTH:
                                queue.append((path, depth + 1))
                            else:
                                truncated = True
            finally:
                os.close(descriptor)
        except OSError:
            if folder == root:
                raise SessionError('This folder is no longer accessible. Choose another folder.') from None
            skipped += 1
    entries.sort(key=lambda entry: (entry['name'].casefold(), entry['path']))
    return {'path': str(root), 'parent': str(root.parent) if root.parent != root else None,
            'entries': entries, 'search': bool(query), 'truncated': truncated, 'skipped': skipped}
