"""Shared filesystem boundary for the browser and its repository tooling."""
from __future__ import annotations

import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts'))
import portable_fs as fs
from windows_security import secure_private_dir


def existing_directory(path):
    path = Path(os.path.abspath(Path(path).expanduser()))
    if os.name != "nt":
        # Retain the existing POSIX registration contract (including /tmp and
        # /var aliases on macOS); protected operations use this canonical root.
        path = path.resolve(strict=True)
    descriptor = fs.open_target_directory(path)
    fs.close(descriptor)
    return path


def same_path(left, right):
    return os.path.normcase(os.path.abspath(left)) == os.path.normcase(os.path.abspath(right))


def default_state_dir():
    if os.name == 'nt':
        return Path(os.environ.get('LOCALAPPDATA', Path.home() / 'AppData/Local')) / 'ai-infrastructure-harness'
    return Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state')) / 'ai-infrastructure-harness'
