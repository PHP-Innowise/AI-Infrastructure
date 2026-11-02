"""Independent deterministic checks over finished task artifacts (stdlib only)."""
from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path
import stat
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Any, Dict

from memory_outcome_isolation import IsolationError, verifier_command
from memory_native_ab_io import ordinary, read_file
from memory_pilot_support import strict_json

MAX_SNAPSHOT_BYTES = 512 * 1024 * 1024
MAX_SNAPSHOT_FILES = 100000
MAX_ANSWER_BYTES = 1024 * 1024


def file_digest(path: Path) -> str:
    return hashlib.sha256(read_file(path, MAX_SNAPSHOT_BYTES)[0]).hexdigest()


def digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'),
                                     ensure_ascii=False, allow_nan=False).encode('utf-8')).hexdigest()


def snapshot_digest(root: Path) -> str:
    """Content-address regular source artifacts; reject links and special files.

    Git/local memory state is private controller metadata, not part of a patch.
    Restricting links deliberately refuses escaping and race-prone candidate
    snapshots instead of dereferencing something outside the disposable tree.
    """
    root = ordinary(root)
    if not root.is_dir():
        raise ValueError('candidate must be an ordinary existing directory')
    result, size = [], 0
    for base, dirs, files in os.walk(root, followlinks=False):
        folder = Path(base)
        dirs[:] = sorted(name for name in dirs if name not in ('.git', '__pycache__'))
        for name in [*dirs, *sorted(files)]:
            path = folder / name
            if path.is_symlink():
                raise ValueError('candidate symlink refused')
            if path.is_file():
                size += path.stat().st_size
                if size > MAX_SNAPSHOT_BYTES or len(result) >= MAX_SNAPSHOT_FILES:
                    raise ValueError('candidate snapshot exceeds its bounded profile')
                result.append((path.relative_to(root).as_posix(), file_digest(path)))
            elif not path.is_dir():
                raise ValueError('candidate special file refused')
    return digest(sorted(result))


def verifier_digest(case: Dict[str, Any]) -> str:
    """Bind the rubric AND implementation/label bytes, never argv alone."""
    if (not isinstance(case, dict) or not isinstance(case.get('read_roots', []), list)
            or not isinstance(case.get('argv', []), list)
            or any(not isinstance(raw, str) for raw in [*case.get('read_roots', []), *case.get('argv', [])])):
        raise ValueError('invalid verifier artifacts')
    images = []
    for raw in case.get('read_roots') or []:
        path = Path(raw)
        if path.is_symlink() or not path.exists():
            raise ValueError('unsafe verifier artifact')
        images.append((str(path.resolve()), _oracle_digest(path)))
    for index, raw in enumerate(case.get('argv') or []):
        path = Path(raw)
        if path.is_absolute() and path.is_file():
            # The host interpreter may be a normal system symlink; private
            # implementation and label paths must remain ordinary artifacts.
            path = path.resolve() if index == 0 else path
            images.append((str(path.resolve()), _oracle_digest(path)))
    return digest([case, images])


def _oracle_file_image(path: Path, limit: int):
    before = path.lstat()
    if stat.S_IMODE(before.st_mode) & 0o7000:
        raise ValueError('special oracle permission bits refused')
    raw, mode = read_file(path, limit)
    after = path.lstat()
    identity = lambda item: (item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns,
                             item.st_ctime_ns, item.st_mode)
    if identity(before) != identity(after):
        raise ValueError('oracle artifact changed while reading')
    return raw, mode


def _oracle_digest(root: Path) -> str:
    """Seal all readable oracle artifacts, including hidden labels and modes."""
    root = ordinary(root)
    if not root.is_dir():
        raw, mode = _oracle_file_image(root, MAX_SNAPSHOT_BYTES)
        return digest(['file', hashlib.sha256(raw).hexdigest(), mode])
    result = [('.', 'directory', stat.S_IMODE(root.stat().st_mode))]
    total = 0
    def fail(error):
        raise error
    for base, directories, files in os.walk(root, followlinks=False, onerror=fail):
        folder = Path(base)
        directories.sort()
        for name in [*directories, *sorted(files)]:
            if len(result) >= MAX_SNAPSHOT_FILES:
                raise ValueError('oracle artifact count exceeded')
            path = folder/name
            image = path.lstat()
            relative = path.relative_to(root).as_posix()
            if stat.S_ISDIR(image.st_mode):
                result.append((relative, 'directory', stat.S_IMODE(image.st_mode)))
            else:
                raw, mode = _oracle_file_image(path, MAX_SNAPSHOT_BYTES - total)
                total += len(raw)
                result.append((relative, 'file', hashlib.sha256(raw).hexdigest(), mode))
    return digest(sorted(result))


def verify_task(snapshot: Path, answer_artifact: Path, trusted_case: Dict[str, Any]) -> Dict[str, Any]:
    started = time.monotonic()
    result = {'status': 'unavailable', 'correct': None, 'verifier_digest': None,
              'artifact_digest': None, 'seconds': None}
    try:
        result['verifier_digest'] = verifier_digest(trusted_case)
        image = snapshot_digest(snapshot)
        answer, _ = read_file(answer_artifact, MAX_ANSWER_BYTES)
        result['artifact_digest'] = digest([image, hashlib.sha256(answer).hexdigest()])
        mode = trusted_case.get('mode')
        if mode == 'answer':
            if set(trusted_case) != {'mode', 'equals'} or not isinstance(trusted_case['equals'], str):
                return result
            correct = answer.decode('utf-8').strip() == trusted_case['equals'].strip()
        elif mode == 'patch':
            if set(trusted_case) != {'mode', 'argv', 'timeout', 'read_roots'}:
                return result
            argv, timeout, roots = trusted_case['argv'], trusted_case['timeout'], trusted_case['read_roots']
            if (not isinstance(argv, list) or not argv or any(not isinstance(s, str) or not s for s in argv)
                    or type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0
                    or not isinstance(roots, list) or any(not isinstance(s, str) for s in roots)):
                return result
            # Ordinary candidate-owned unittest/pytest discovery is not an
            # independent oracle. Require an explicit external implementation
            # whose bytes are part of the sealed read roots.
            external = []
            # Known executable forms only: a sealed native oracle executable,
            # or the Python interpreter followed immediately by its script.
            program = argv[1:2] if Path(argv[0]).resolve() == Path(sys.executable).resolve() else argv[:1]
            for raw in program:
                path = Path(raw)
                if (path.is_absolute() and path.is_file() and not path.is_symlink()
                        and snapshot.resolve() not in path.resolve().parents
                        and any(path.resolve() == Path(root).resolve()
                                or Path(root).resolve() in path.resolve().parents for root in roots)):
                    external.append(path)
            if not external:
                return result
            # Source/candidate tests are never substituted for the private oracle.
            with tempfile.TemporaryDirectory(prefix='outcome-verify-', dir=snapshot.parent) as temporary:
                candidate = Path(temporary) / 'candidate'
                shutil.copytree(snapshot, candidate, symlinks=True, ignore=shutil.ignore_patterns('.git', '__pycache__'))
                if snapshot_digest(candidate) != image:
                    return result
                command = verifier_command(candidate, argv, roots)
                with tempfile.TemporaryFile() as output:
                    checked = subprocess.run(command, cwd=candidate, env={}, stdin=subprocess.DEVNULL,
                                             stdout=output, stderr=subprocess.DEVNULL, timeout=timeout)
                    output.seek(0)
                    raw_receipt = output.read(65537)
            if checked.returncode != 0 or len(raw_receipt) > 65536:
                return result
            receipt = strict_json(raw_receipt)
            if (not isinstance(receipt, dict) or set(receipt) != {'schema', 'completed', 'correct'}
                    or receipt['schema'] != 'task-outcome-verifier-v1'
                    or receipt['completed'] is not True or type(receipt['correct']) is not bool):
                return result
            correct = receipt['correct']
        else:
            return result
        if verifier_digest(trusted_case) != result['verifier_digest']:
            return result
        result.update(status='passed' if correct else 'failed', correct=correct)
    except (OSError, ValueError, TypeError, UnicodeError, IsolationError, subprocess.TimeoutExpired):
        pass
    finally:
        result['seconds'] = round(time.monotonic() - started, 4)
    return result
