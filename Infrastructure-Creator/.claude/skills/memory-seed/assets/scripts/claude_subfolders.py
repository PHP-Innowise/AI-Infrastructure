"""Claude Code in a folder below an installed project: keep it working.

Claude Code reads a project's settings - its hooks and permission rules -
only from the folder a session starts in, so a session started in src/ of an
installed project ran none of the project's hooks: no memory, no guards. Run
by the session-start hook of a session in the project root, this keeps a small
dispatcher in the person's own Claude Code configuration folder - the
claude_dispatch.sh and claude_dispatch.py beside this file - with one entry
per project hook in their settings.json, and lists this project for it. The
dispatcher then runs this project's hooks for a session started below it,
and does nothing anywhere else. Further session-start entries bring such a
session the project's AGENTS.md, which its .claude/CLAUDE.md imports from
outside the session's folder.

It writes only into the configuration folder of the Claude Code that started
this hook - the session's transcript must live there - and nothing when all
is already in place. A settings.json it cannot read as a JSON object is left
as it is. Never on Windows, whose Git Bash paths the dispatcher does not map,
and never again once the person has put a file named `off` into the
dispatcher's folder (ai-accelerator/ in the configuration folder).

Usage: claude_subfolders.py < SESSION-START-INPUT
"""
from __future__ import annotations

import json
import os
import shlex
import stat
import sys
import tempfile
from pathlib import Path

from claude_dispatch import POLICY_PARTS

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FOLDER = "ai-accelerator"
DISPATCHER = "ai-accelerator-dispatch.sh"
HELPER = "ai-accelerator-dispatch.py"
HOOK_PREFIX = '"${CLAUDE_PROJECT_DIR}"/.claude/hooks/'


def config_folder() -> Path:
    folder = os.environ.get("CLAUDE_CONFIG_DIR")
    return Path(folder) if folder else Path.home() / ".claude"


def own_session(payload: object, config: Path) -> bool:
    """The hook was started by a Claude Code that keeps its transcripts in
    `config`: a test or a script feeding the hook gets nothing written."""
    transcript = payload.get("transcript_path") if isinstance(payload, dict) else None
    if not isinstance(transcript, str) or not os.path.isabs(transcript):
        return False
    projects = os.path.realpath(config / "projects")
    return os.path.realpath(transcript).startswith(projects + os.sep)


def wanted_entries(settings: Path, dispatcher: Path) -> list[tuple[str, dict]]:
    """The dispatcher's entry, one group each, for every hook script in the
    project's settings, with the script's matcher and timeout, then those
    for the parts of AGENTS.md."""
    data = json.loads(settings.read_text(encoding="utf-8"))
    entries: list[tuple[str, dict]] = []
    for event, groups in (data.get("hooks") or {}).items():
        for group in groups if isinstance(groups, list) else []:
            for hook in group.get("hooks") or [] if isinstance(group, dict) else []:
                command = hook.get("command") if isinstance(hook, dict) else None
                if not isinstance(command, str) or not command.startswith(HOOK_PREFIX):
                    continue
                name = command[len(HOOK_PREFIX):]
                if "/" in name or not name.endswith(".sh") or not name.replace("-", "").replace(".", "").isalnum():
                    continue
                entry: dict = {"type": "command", "command": f"{shlex.quote(str(dispatcher))} {name}"}
                if isinstance(hook.get("timeout"), int):
                    entry["timeout"] = hook["timeout"]
                one: dict = {"hooks": [entry]}
                if isinstance(group.get("matcher"), str) and group["matcher"]:
                    one = {"matcher": group["matcher"], **one}
                entries.append((event, one))
    if (ROOT / "AGENTS.md").is_file():
        for part in range(1, POLICY_PARTS + 1):
            command = f"{shlex.quote(str(dispatcher))} policy-{part}"
            entries.append(("SessionStart", {"hooks": [{"type": "command", "command": command, "timeout": 10}]}))
    return entries


def key(event: str, group: object, dispatcher: Path):
    """(event, matcher, script) of a dispatcher entry, or None for any other."""
    hooks = group.get("hooks") if isinstance(group, dict) else None
    marker = shlex.quote(str(dispatcher)) + " "
    if not isinstance(hooks, list) or len(hooks) != 1 or not isinstance(hooks[0], dict):
        return None
    command = hooks[0].get("command")
    if not isinstance(command, str) or not command.startswith(marker):
        return None
    return event, group.get("matcher", ""), command[len(marker):]


def merged(user: dict, wanted: list[tuple[str, dict]], dispatcher: Path) -> dict:
    """The user's settings with this project's dispatcher entries: one for the
    same hook and matcher replaced, those other projects added kept, nothing
    else touched."""
    result = json.loads(json.dumps(user))
    hooks = result.setdefault("hooks", {})
    for event, group in wanted:
        present = hooks.setdefault(event, [])
        same = key(event, group, dispatcher)
        for index, existing in enumerate(present):
            if key(event, existing, dispatcher) == same:
                present[index] = group
                break
        else:
            present.append(group)
    return result


def write_if_changed(path: Path, content: bytes, mode: int) -> bool:
    try:
        if path.read_bytes() == content and stat.S_IMODE(path.stat().st_mode) == mode:
            return False
    except OSError:
        pass
    descriptor, name = tempfile.mkstemp(prefix=f".{path.name}.", dir=str(path.parent))
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(content)
        os.chmod(name, mode)
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)
    return True


def main() -> int:
    if os.name == "nt":
        return 0
    try:
        payload = json.loads(sys.stdin.buffer.read(1024 * 1024) or b"{}")
    except ValueError:
        return 0
    config = config_folder()
    settings = ROOT / ".claude" / "settings.json"
    if not own_session(payload, config) or not settings.is_file() or not config.is_dir():
        return 0
    folder = config / FOLDER
    if os.path.lexists(folder / "off"):
        return 0
    folder.mkdir(mode=0o700, exist_ok=True)
    info = folder.lstat()
    if not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode) or info.st_uid != os.getuid():
        return 0
    dispatcher = folder / DISPATCHER
    write_if_changed(dispatcher, (HERE / "claude_dispatch.sh").read_bytes(), 0o755)
    write_if_changed(folder / HELPER, (HERE / "claude_dispatch.py").read_bytes(), 0o644)
    try:
        wanted = wanted_entries(settings, dispatcher)
    except (OSError, ValueError, AttributeError):
        return 0
    # A settings.json linked from elsewhere - a dotfiles checkout - is
    # written where it lives, and stays linked.
    user_settings = Path(os.path.realpath(config / "settings.json"))
    import fcntl
    with open(folder / ".lock", "a") as lock:
        # Sessions of two projects starting together each add their root and
        # write the same entries: the lock keeps one from reading what the
        # other has half written, or dropping its root.
        fcntl.flock(lock, fcntl.LOCK_EX)
        projects = folder / "projects"
        try:
            listed = projects.read_text(encoding="utf-8").splitlines()
        except OSError:
            listed = []
        # A project removed since is dropped; this one is listed once.
        kept = [line for line in dict.fromkeys(listed)
                if os.path.isfile(os.path.join(line, "memory-bank", "scripts", "context.py"))]
        if str(ROOT) not in kept:
            kept.append(str(ROOT))
        write_if_changed(projects, ("\n".join(kept) + "\n").encode(), 0o644)
        try:
            user = json.loads(user_settings.read_text(encoding="utf-8")) if user_settings.exists() else {}
        except (OSError, ValueError):
            return 0
        if not isinstance(user, dict) or not isinstance(user.get("hooks", {}), dict):
            return 0
        updated = merged(user, wanted, dispatcher)
        if updated != user:
            mode = stat.S_IMODE(user_settings.stat().st_mode) if user_settings.exists() else 0o644
            write_if_changed(user_settings, (json.dumps(updated, indent=2, ensure_ascii=False) + "\n").encode(), mode)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:  # a session's start is never failed by this
        sys.exit(0)
