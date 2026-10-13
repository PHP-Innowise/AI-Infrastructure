"""Safe Windows argv helpers; never execute a batch shim through ``cmd.exe``."""
from __future__ import annotations

import json
import os
from pathlib import Path, PureWindowsPath
import shlex
import shutil
import stat


WINDOWS = os.name == "nt"

# npm generates .cmd launchers for these packages.  The executable name and
# package metadata must agree before bypassing the untrusted batch program.
NPM_SHIMS = {
    "codex": ("@openai/codex", "codex"),
    "claude": ("@anthropic-ai/claude-code", "claude"),
    "npm": ("npm", "npm"),
    "npx": ("npm", "npx"),
}


def _reparse(path: Path) -> bool:
    try:
        info = os.stat(path, follow_symlinks=False)
    except OSError:
        return True
    attribute = getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)
    return path.is_symlink() or bool(getattr(info, "st_file_attributes", 0) & attribute)


def _relative_file(root: Path, value: object) -> Path:
    if not isinstance(value, str) or not value or "\\" in value:
        raise ValueError("Unsupported Windows npm launcher.")
    relative = Path(value)
    if (relative.is_absolute() or PureWindowsPath(value).is_absolute()
            or ".." in relative.parts or relative == Path(".")):
        raise ValueError("Unsupported Windows npm launcher.")
    path = root / relative
    try:
        path.relative_to(root)
    except ValueError as error:
        raise ValueError("Unsupported Windows npm launcher.") from error
    if _reparse(root):
        raise ValueError("Unsupported Windows npm launcher.")
    current = root
    for part in relative.parts:
        current /= part
        if _reparse(current):
            raise ValueError("Unsupported Windows npm launcher.")
    if not path.is_file():
        raise ValueError("Unsupported Windows npm launcher.")
    return path


def _node_executable() -> Path:
    value = shutil.which("node.exe") or shutil.which("node")
    if not value:
        raise ValueError("Node.js is required for this Windows npm launcher.")
    path = Path(value)
    if path.suffix.lower() != ".exe" or _reparse(path) or not path.is_file():
        raise ValueError("Node.js is required for this Windows npm launcher.")
    return path


def _npm_prefix(command: str, provider: str | None) -> list[str]:
    shim = Path(command)
    name = shim.stem.lower()
    configured = NPM_SHIMS.get(name)
    if configured is None or (provider is not None and provider != name):
        raise ValueError("Unsupported Windows batch launcher.")
    package_name, bin_name = configured
    if _reparse(shim) or _reparse(shim.parent):
        raise ValueError("Unsupported Windows npm launcher.")
    package = shim.parent
    for part in ("node_modules", *package_name.split("/")):
        package /= part
        if _reparse(package):
            raise ValueError("Unsupported Windows npm launcher.")
    metadata = package / "package.json"
    if _reparse(metadata):
        raise ValueError("Unsupported Windows npm launcher.")
    try:
        data = json.loads(metadata.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("Unsupported Windows npm launcher.") from error
    bins = data.get("bin") if isinstance(data, dict) else None
    if (not isinstance(data, dict) or data.get("name") != package_name
            or not isinstance(bins, dict) or bin_name not in bins):
        raise ValueError("Unsupported Windows npm launcher.")
    target = _relative_file(package, bins[bin_name])
    if target.suffix.lower() == ".exe":
        return [str(target)]
    if target.suffix.lower() not in {".js", ".cjs"}:
        raise ValueError("Unsupported Windows npm launcher.")
    return [str(_node_executable()), str(target)]


def command_argv(command: list[str], provider: str | None = None) -> list[str]:
    """Return a direct argv, resolving only allowlisted npm batch wrappers."""
    if not isinstance(command, list) or not command or any(not isinstance(value, str) or "\0" in value for value in command):
        raise ValueError("A native executable is required.")
    executable = command[0]
    if WINDOWS:
        executable = shutil.which(executable) or executable
    if not WINDOWS or Path(executable).suffix.lower() not in {".cmd", ".bat"}:
        return [executable, *command[1:]]
    return [*_npm_prefix(executable, provider), *command[1:]]


def split_command(command: str) -> list[str]:
    """Parse one direct command using the host platform's argv rules."""
    if not WINDOWS:
        return shlex.split(command)
    import ctypes

    argc = ctypes.c_int()
    shell32 = ctypes.WinDLL("shell32", use_last_error=True)
    parse = shell32.CommandLineToArgvW
    parse.argtypes = (ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_int))
    parse.restype = ctypes.POINTER(ctypes.c_wchar_p)
    values = parse("harness-check.exe " + command, ctypes.byref(argc))
    if not values:
        raise ValueError("Check command quoting.")
    try:
        return [values[index] for index in range(1, argc.value)]
    finally:
        free = ctypes.WinDLL("kernel32", use_last_error=True).LocalFree
        free.argtypes = (ctypes.c_void_p,)
        free.restype = ctypes.c_void_p
        free(values)
