"""Independent deterministic checks over finished task artifacts (stdlib only)."""
from __future__ import annotations

import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Any, Dict

from memory_outcome_isolation import IsolationError, verifier_command

MAX_SNAPSHOT_BYTES = 512 * 1024 * 1024
MAX_SNAPSHOT_FILES = 100000
MAX_ANSWER_BYTES = 1024 * 1024


def file_digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open('rb') as stream:
        while True:
            block = stream.read(1024 * 1024)
            if not block:
                break
            value.update(block)
    return value.hexdigest()


def digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'),
                                     ensure_ascii=False, allow_nan=False).encode('utf-8')).hexdigest()


def snapshot_digest(root: Path) -> str:
    """Content-address regular source artifacts; reject links and special files.

    Git/local memory state is private controller metadata, not part of a patch.
    Restricting links deliberately refuses escaping and race-prone candidate
    snapshots instead of dereferencing something outside the disposable tree.
    """
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
    images = []
    for raw in case.get('read_roots') or []:
        path = Path(raw)
        if path.is_symlink() or not path.exists():
            raise ValueError('unsafe verifier artifact')
        images.append((str(path.resolve()), snapshot_digest(path) if path.is_dir()
                       else file_digest(path)))
    for raw in case.get('argv') or []:
        path = Path(raw)
        if path.is_absolute() and path.is_file():
            images.append((str(path.resolve()), file_digest(path)))
    return digest([case, images])


def verify_task(snapshot: Path, answer_artifact: Path, trusted_case: Dict[str, Any]) -> Dict[str, Any]:
    started = time.monotonic()
    result = {'status': 'unavailable', 'correct': None, 'verifier_digest': None,
              'artifact_digest': None, 'seconds': None}
    try:
        result['verifier_digest'] = verifier_digest(trusted_case)
        image = snapshot_digest(snapshot)
        if answer_artifact.stat().st_size > MAX_ANSWER_BYTES:
            return result
        answer = answer_artifact.read_bytes()
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
            receipt = json.loads(raw_receipt)
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
