"""Codex in a linked worktree of an installed project: keep it working.

Git gives a worktree only what it tracks, and an installation is often kept
out of a project's history. Codex reads a project's .codex/ only in the
folders between the session's folder and the nearest .git, which in a
worktree is the worktree's own, so a session started in a worktree of an
installed project ran none of the project's hooks: no memory, no guards -
Codex 0.164 in a worktree inside the project, its hooks trusted, ran none.
Run by the session-start hook of a Codex session in the project root, this
keeps a small dispatcher in the person's own Codex folder - the
codex_dispatch.sh and codex_dispatch.py beside this file - with one entry per
project hook in their hooks.json, one more for AGENTS.md, and lists this
project for it. The dispatcher then runs this project's hooks for a session
in a worktree of it, and does nothing anywhere else: Codex itself runs them
in the root and in the folders below it.

Codex runs a hook only once the person has trusted its definition - a hash
it keeps in config.toml, written by its own hook review (/hooks in its
terminal, which offers "Trust all" when a session starts; Settings > Hooks
in the app). This never writes that record, nor config.toml at all: the
entries wait for the person's review, which a changed definition asks for
again, so they are written the same way every time and appended after the
person's own, whose keys - their position - stay. An edit of the scripts
they run needs no new review.

It writes only into the Codex folder of the Codex that started this hook -
the session's transcript must live there - and nothing when all is already
in place. A hooks.json it cannot read as a JSON object is left as it is, and
nothing is written for a person who keeps hooks in config.toml, where Codex
would warn at every start that hooks come in two forms; nothing again once
the person has put a file named `off` into the dispatcher's folder
(ai-accelerator/ in the Codex folder). Not on Windows, where Codex does not
run a hook's command with bash.

The installer runs it with --install when a person installs in a terminal,
so a first session in a worktree has the project's hooks too, once trusted;
it then writes into the Codex folder this machine already has, if there is
one, and reads no session input.

Usage: codex_worktrees.py < SESSION-START-INPUT
       codex_worktrees.py --install
"""
from __future__ import annotations

import json
import os
import re
import shlex
import stat
import sys
from pathlib import Path

from claude_subfolders import exclusive, installed_version, write_if_changed
from codex_dispatch import DISPATCH_VERSION, POLICY_LIMIT

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FOLDER = "ai-accelerator"
DISPATCHER = "ai-accelerator-dispatch.sh"
HELPER = "ai-accelerator-dispatch.py"
# The script a project hook's command runs: its last word, a name in
# .codex/hooks/ the dispatcher accepts.
SCRIPT = re.compile(r"([A-Za-z0-9][A-Za-z0-9.-]*\.sh)\W*$")
# What an entry keeps of the project hook it stands for.
KEPT = {"timeout": int, "additionalContextLimit": int, "statusMessage": str}


def codex_home() -> Path:
    folder = os.environ.get("CODEX_HOME")
    return Path(folder) if folder else Path.home() / ".codex"


def own_session(payload: object, home: Path) -> bool:
    """The hook was started by a Codex that keeps its transcripts in `home`:
    a test or a script feeding the hook gets nothing written."""
    transcript = payload.get("transcript_path") if isinstance(payload, dict) else None
    if not isinstance(transcript, str) or not os.path.isabs(transcript):
        return False
    sessions = os.path.normcase(os.path.realpath(home / "sessions"))
    return os.path.normcase(os.path.realpath(transcript)).startswith(sessions + os.sep)


def wanted_entries(hooks: Path, dispatcher: Path) -> list[tuple[str, dict]]:
    """The dispatcher's entry, one group each, for every hook script in the
    project's .codex/hooks.json, with the group's matcher and the hook's
    timeout and context limit, then the one for AGENTS.md."""
    data = json.loads(hooks.read_text(encoding="utf-8"))
    command = shlex.quote(str(dispatcher))
    entries: list[tuple[str, dict]] = []
    for event, groups in (data.get("hooks") or {}).items():
        for group in groups if isinstance(groups, list) else []:
            for hook in group.get("hooks") or [] if isinstance(group, dict) else []:
                found = SCRIPT.search(hook.get("command") or "") if isinstance(hook, dict) else None
                if not found or not (hooks.parent / "hooks" / found.group(1)).is_file():
                    continue
                entry: dict = {"type": "command", "command": f"{command} {found.group(1)}"}
                entry.update((name, hook[name]) for name, kind in KEPT.items()
                             if type(hook.get(name)) is kind)
                one: dict = {"hooks": [entry]}
                if isinstance(group.get("matcher"), str) and group["matcher"]:
                    one = {"matcher": group["matcher"], **one}
                entries.append((event, one))
    if entries and (ROOT / "AGENTS.md").is_file():
        entries.append(("SessionStart", {"hooks": [{"type": "command", "command": f"{command} policy",
                                                    "timeout": 10, "additionalContextLimit": POLICY_LIMIT}]}))
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


def merged(user: dict, wanted: list[tuple[str, dict]], dispatcher: Path, replace: bool = True) -> dict:
    """The person's hooks with this project's dispatcher entries: one for the
    same hook and matcher replaced where it stands - unless a newer release's
    dispatcher wrote it - one missing appended after the rest, nothing else
    touched."""
    result = json.loads(json.dumps(user))
    hooks = result.setdefault("hooks", {})
    for event, group in wanted:
        present = hooks.setdefault(event, [])
        same = key(event, group, dispatcher)
        for index, existing in enumerate(present):
            if key(event, existing, dispatcher) == same:
                if replace:
                    present[index] = group
                break
        else:
            present.append(group)
    return result


def toml_hooks(config: Path) -> bool:
    """Whether the person's config.toml defines hooks of its own - more than
    the trust records under hooks.state. Codex loads both forms and warns at
    every start that the layer has two. One it cannot read counts as yes."""
    try:
        text = config.read_text(encoding="utf-8")
    except FileNotFoundError:
        return False
    except (OSError, UnicodeError):
        return True
    try:
        import tomllib
    except ImportError:  # Python 3.9 and 3.10: the lines that could define one
        table = None
        for line in text.splitlines():
            found = re.match(r"\s*\[\[?\s*([^\]]*?)\s*\]", line)
            if found:
                table = re.sub(r"[\s\"']", "", found.group(1))
                if table.startswith("hooks.") and not re.match(r"hooks\.state(\.|$)", table):
                    return True
                continue
            setting = re.match(r"\s*([A-Za-z0-9_.\"'-]+?)\s*=", line)
            name = re.sub(r"[\"']", "", setting.group(1)) if setting else ""
            if (table is None and re.match(r"hooks(\.|$)", name) and not re.match(r"hooks\.state(\.|$)", name)
                    or table == "hooks" and name and not re.match(r"state(\.|$)", name)):
                return True
        return False
    try:
        hooks = tomllib.loads(text).get("hooks")
    except tomllib.TOMLDecodeError:
        return True
    return isinstance(hooks, dict) and any(name != "state" for name in hooks)


def main(argv: list) -> int:
    if os.name == "nt":
        return 0
    home = codex_home()
    if argv[1:] != ["--install"]:
        try:
            payload = json.loads(sys.stdin.buffer.read(1024 * 1024) or b"{}")
        except ValueError:
            return 0
        if not own_session(payload, home):
            return 0
    hooks = ROOT / ".codex" / "hooks.json"
    if not hooks.is_file() or not home.is_dir():
        return 0
    folder = home / FOLDER
    if os.path.lexists(folder / "off") or toml_hooks(home / "config.toml"):
        return 0
    dispatcher = folder / DISPATCHER
    try:
        wanted = wanted_entries(hooks, dispatcher)
    except (OSError, ValueError, AttributeError, TypeError):
        return 0
    if not wanted:
        return 0
    folder.mkdir(mode=0o700, exist_ok=True)
    info = folder.lstat()
    if not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode) or info.st_uid != os.getuid():
        return 0
    # A hooks.json linked from elsewhere - a dotfiles checkout - is written
    # where it lives, and stays linked.
    user_hooks = Path(os.path.realpath(home / "hooks.json"))
    with exclusive(folder / ".lock"):
        # Sessions of two projects starting together each add their root and
        # write the same files: the lock keeps one from reading what the
        # other has half written, or dropping its root. Another project's
        # newer release put a newer dispatcher here: it stays, and so do the
        # entries it wrote.
        newer = installed_version(folder / HELPER) > DISPATCH_VERSION
        if not newer:
            write_if_changed(dispatcher, (HERE / "codex_dispatch.sh").read_bytes(), 0o755)
            write_if_changed(folder / HELPER, (HERE / "codex_dispatch.py").read_bytes(), 0o644)
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
            user = json.loads(user_hooks.read_text(encoding="utf-8")) if user_hooks.exists() else {}
        except (OSError, ValueError):
            return 0
        if not isinstance(user, dict) or not isinstance(user.get("hooks", {}), dict) \
                or not all(isinstance(groups, list) for groups in user.get("hooks", {}).values()):
            return 0
        updated = merged(user, wanted, dispatcher, replace=not newer)
        if updated != user:
            mode = stat.S_IMODE(user_hooks.stat().st_mode) if user_hooks.exists() else 0o644
            write_if_changed(user_hooks, (json.dumps(updated, indent=2, ensure_ascii=False) + "\n").encode(), mode)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except Exception:  # a session's start is never failed by this
        sys.exit(0)
