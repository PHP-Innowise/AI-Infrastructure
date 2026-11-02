"""Small stdlib-only boundaries shared by offline memory pilots."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import stat
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
CANON = ROOT / 'PHP Core' / 'memory-bank' / 'scripts'
MAX_FILE = 4 * 1024 * 1024
MAX_TREE = 64 * 1024 * 1024
_MODULES: dict[str, Any] = {}
SKIP_PARTS = {'.git', '.ssh', '.aws', '.kube', '.venv', '__pycache__', 'node_modules',
              'vendor', '.context-handoff', '.superpowers', '.worktrees'}
SKIP_PREFIXES = ('memory-bank/local/', 'project-brain/local/', 'launch/', '.claude/worktrees/',
                 '.codex/auth.json', '.claude/settings.local.json')


class PilotError(ValueError):
    pass


def canonical_module(name: str) -> Any:
    if name not in ('brain_runtime', 'automatic_query', 'workspace_roots'):
        raise PilotError('unknown canonical module')
    if name not in _MODULES:
        spec = importlib.util.spec_from_file_location('memory_pilot_' + name, CANON / (name + '.py'))
        module = importlib.util.module_from_spec(spec)
        old = sys.dont_write_bytecode
        sys.dont_write_bytecode = True
        sys.path.insert(0, str(CANON))
        try:
            spec.loader.exec_module(module)
        finally:
            sys.path.pop(0)
            sys.dont_write_bytecode = old
        _MODULES[name] = module
    return _MODULES[name]


def digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True,
        separators=(',', ':'), allow_nan=False).encode('utf-8')).hexdigest()


def public_text(value: str) -> None:
    try: value.encode('utf-8')
    except UnicodeError as error: raise PilotError('invalid Unicode input') from error
    auto = canonical_module('automatic_query')
    brain = canonical_module('brain_runtime')
    if brain.sensitive_label(value) or any(pattern.search(value) for pattern in auto.SECRET_PATTERNS.values()):
        raise PilotError('sensitive input refused')


def relative_path(value: str, *, memory: bool = False) -> PurePosixPath:
    if (not isinstance(value, str) or not value or '\\' in value or ':' in value or '\0' in value
            or value.startswith('/') or any(part in ('', '.', '..') for part in value.split('/'))):
        raise PilotError('unsafe relative path')
    path = PurePosixPath(value)
    auto = canonical_module('automatic_query')
    if memory:
        if (auto.SOURCE_PATH_DENYLIST.search(value)
                or any(part.casefold() in auto.BLOCKED_SOURCE_PARTS for part in path.parts)
                or value.endswith(('.db', '.sqlite'))):
            raise PilotError('private path refused')
    elif auto.source_path_problem(value):
        raise PilotError('private source refused')
    return path


def _source_image(root: Path, value: str, *, memory: bool = False,
                  limit: int = MAX_FILE) -> tuple[bytes, int]:
    parts = relative_path(value, memory=memory).parts
    root = Path(root)
    if root.is_symlink() or not root.is_dir():
        raise PilotError('snapshot must be an ordinary directory')
    descriptors = []
    try:
        if os.name == 'posix':
            fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            descriptors.append(fd)
            for part in parts[:-1]:
                fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                descriptors.append(fd)
            fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
        else:
            path = root
            for part in parts:
                path = path / part
                image = path.lstat()
                if path.is_symlink() or getattr(image, 'st_file_attributes', 0) & 0x400:
                    raise PilotError('linked source refused')
            path.resolve().relative_to(root.resolve())
            fd = os.open(path, os.O_RDONLY | getattr(os, 'O_BINARY', 0))
        descriptors.append(fd)
        image = os.fstat(fd)
        if not stat.S_ISREG(image.st_mode) or image.st_size > limit:
            raise PilotError('source is not a bounded regular file')
        chunks, size = [], 0
        while True:
            chunk = os.read(fd, min(65536, limit + 1 - size))
            if not chunk:
                break
            chunks.append(chunk); size += len(chunk)
            if size > limit:
                raise PilotError('source exceeds byte limit')
        after = os.fstat(fd)
        named = (os.stat(parts[-1], dir_fd=descriptors[-2], follow_symlinks=False)
                 if os.name == 'posix' else path.lstat())
        identity = lambda item: (item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns,
                                 item.st_ctime_ns, item.st_mode)
        if identity(image) != identity(after) or identity(after) != identity(named):
            raise PilotError('source changed while reading')
        return b''.join(chunks), stat.S_IMODE(image.st_mode) & 0o777
    except (OSError, ValueError) as error:
        raise PilotError('source unavailable or unsafe') from error
    finally:
        for fd in reversed(descriptors):
            os.close(fd)


def safe_read(root: Path, value: str, *, memory: bool = False, limit: int = MAX_FILE) -> bytes:
    return _source_image(root, value, memory=memory, limit=limit)[0]


def strict_json(raw: bytes) -> Any:
    def bad(_):
        raise PilotError('nonfinite JSON refused')
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result: raise PilotError('duplicate JSON key')
            result[key] = value
        return result
    value = json.loads(raw.decode('utf-8'), parse_constant=bad, object_pairs_hook=pairs)
    json.dumps(value, ensure_ascii=False, allow_nan=False).encode('utf-8')
    return value


def _snapshot_image(root: Path) -> list[tuple[str, bytes, int]]:
    if root.is_symlink() or not root.is_dir() or root.resolve() == ROOT:
        raise PilotError('use an ordinary project snapshot, not the tooling clone')
    result, total = [], 0
    auto = canonical_module('automatic_query')
    for base, directories, files in os.walk(root, followlinks=False):
        folder = Path(base)
        directories[:] = sorted(name for name in directories if name not in SKIP_PARTS and not name.casefold().startswith('.env')
            and not (folder / name).is_symlink()
            and not getattr((folder / name).lstat(), 'st_file_attributes', 0) & 0x400
            and not ((folder / name).relative_to(root).as_posix() + '/').startswith(SKIP_PREFIXES))
        for name in sorted(files):
            value = (folder / name).relative_to(root).as_posix()
            if (name in SKIP_PARTS or name.casefold() == 'credentials.json'
                    or value == 'accelerator-attach.json' or value.startswith(SKIP_PREFIXES)
                    or auto.SOURCE_PATH_DENYLIST.search(value)
                    or any(part.casefold().startswith('.env') for part in PurePosixPath(value).parts)
                    or any(part.casefold() in auto.BLOCKED_SOURCE_PARTS for part in PurePosixPath(value).parts)
                    or Path(value).suffix.casefold() in auto.PRIVATE_SOURCE_SUFFIXES):
                continue
            path = folder / name
            if path.is_symlink() or not path.is_file():
                continue
            raw, mode = _source_image(root, value, memory=True)
            total += len(raw)
            if total > MAX_TREE or len(result) >= 20000:
                raise PilotError('snapshot exceeds pilot limits')
            result.append((value, raw, mode))
    return result


def snapshot_files(root: Path) -> list[tuple[str, bytes]]:
    return [(name, raw) for name, raw, _ in _snapshot_image(root)]


def snapshot_digest(root: Path) -> str:
    return digest([(name, hashlib.sha256(raw).hexdigest(), mode)
                   for name, raw, mode in _snapshot_image(root)])


def copy_snapshot(root: Path, destination: Path) -> str:
    files = _snapshot_image(root)
    destination.mkdir(mode=0o700)
    for name, raw, mode in files:
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('xb') as stream:
            stream.write(raw)
        path.chmod(mode)
    return digest([(name, hashlib.sha256(raw).hexdigest(), mode) for name, raw, mode in files])


def child_env() -> dict[str, str]:
    return {key: value for key, value in os.environ.items()
            if not key.startswith(('GIT_', 'ACCELERATOR_', 'CONTEXT_'))
            and key not in ('CLAUDE_PROJECT_DIR', 'PYTHONPATH', 'PYTHONHOME')}
