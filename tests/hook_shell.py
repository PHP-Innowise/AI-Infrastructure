"""The bash that hosts start hook scripts with, for tests that run hooks."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
from typing import Optional


def hook_bash() -> Optional[str]:
    """Git for Windows' bin\\bash.exe on Windows, otherwise the bash on PATH.

    Claude Code, Cursor and Codex start hooks with Git Bash on Windows.
    CLAUDE_CODE_GIT_BASH_PATH names it for Claude Code; otherwise it is found
    beside the git on PATH. A bare "bash" there starts WSL's launcher in
    System32, which CreateProcess searches before PATH, so ['bash', hook] ran
    no hook at all.
    """
    if os.name != "nt":
        return shutil.which("bash")
    configured = os.environ.get("CLAUDE_CODE_GIT_BASH_PATH")
    if configured:
        return configured if Path(configured).is_file() else None
    git = shutil.which("git")
    for folder in Path(git).resolve().parents if git else ():
        if (folder / "bin" / "bash.exe").is_file():
            return str(folder / "bin" / "bash.exe")
    return None


# Found once, before a test narrows the environment it runs hooks with.
BASH = hook_bash()
