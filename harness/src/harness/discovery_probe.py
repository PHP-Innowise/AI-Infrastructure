"""Boundary checks inside the AI discovery sandbox, run before a scan on Windows.

Usage: discovery_probe.py PATHS WORKSPACE EVIDENCE

PATHS lists, one per line, every original file and folder the scan must not
read. After a line `--readable` come evidence copies it must read. A denied
path is only opened, never read, and nothing outside the workspace changes.
"""
import os
from pathlib import Path
import sys
import uuid

UNREADABLE = (PermissionError, FileNotFoundError, NotADirectoryError)


def readable(path):
    """True when the sandbox can list this folder or open this file for reading."""
    if os.name == 'nt' and not path.startswith('\\\\?\\'):
        path = '\\\\?\\' + path  # A long path must not look unreadable merely for its length.
    try:
        os.listdir(path)
        return True
    except UNREADABLE:
        pass
    try:
        with open(path, 'rb'):
            return True
    except (*UNREADABLE, IsADirectoryError):
        return False


def main():
    paths, workspace, evidence = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    denied, allowed, target = [], [], None
    for line in paths.read_text(encoding='utf-8').splitlines():
        if line == '--readable':
            target = allowed
        elif line:
            (denied if target is None else allowed).append(line)
    exposed = [path for path in denied if readable(path)]
    if exposed:
        print(f'The sandbox could read {len(exposed)} of {len(denied)} original files and folders, '
              f'for example {exposed[0]}.', flush=True)
        return 1
    if not all(readable(path) for path in allowed):
        print('The sandbox could not read the captured evidence.', flush=True)
        return 1
    scratch = workspace / '.harness-tmp' / ('probe-' + uuid.uuid4().hex)
    scratch.write_text('workspace probe')
    scratch.unlink()
    written = evidence / ('probe-' + uuid.uuid4().hex)
    try:
        written.write_text('evidence probe')
    except OSError:
        pass
    else:
        written.unlink()
        print('The sandbox could change the captured evidence.', flush=True)
        return 1
    print(f'AI discovery sandbox boundaries verified for {len(denied)} original files and folders.', flush=True)
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except Exception as error:  # noqa: BLE001 - any failure leaves the boundary unverified
        print('The AI discovery sandbox boundary could not be verified: ' + type(error).__name__, flush=True)
        raise SystemExit(1)
