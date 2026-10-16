#!/usr/bin/env python3
"""Where the accelerator's tooling, its project and its state live.

Installed, the accelerator is copied into the project, so one directory - the
consuming project's root - holds all three: the tooling (policy, skills,
runtime scripts, schemas), the project's own files, and the accelerator's
state (Project Brain records, Memory Bank chunks, the local index). Every
helper below then answers with that one directory, and every key and path is
exactly what it was before attached mode existed.

Attached, the accelerator stays in its own clone and is lent to a project for
a session (the Harness, or scripts/accelerator_attach.py); nothing is copied
into the project. The launcher exports three variables and the roots differ:

- tooling: ACCELERATOR_HOME, the edition directory of the clone this file
  belongs to;
- state: ACCELERATOR_STATE_DIR - the accelerator's private directory for that
  project, holding project-brain/ and memory-bank/; it is the `--root`;
- project: ACCELERATOR_PROJECT_DIR - the project's own working tree, where
  Git runs and which project documents and citations are read from.

The variables bind only the runtime copy they name: ACCELERATOR_HOME must be
this file's edition and ACCELERATOR_STATE_DIR the state root in use. A copy
installed in some other project therefore keeps its own layout even when a
shell exported the variables for an attached session elsewhere.

A document or citation key says which root it belongs to by its shape. An
absolute path is taken as written; tooling documents are keyed that way, so
the accelerator's AGENTS.md cannot collide with a project's own. A key under
project-brain/ or memory-bank/ is accelerator state. Any other key is a path
in the project.
"""

from __future__ import annotations

import contextlib
import functools
import os
import stat
import stat as stat_module
import subprocess
import time
from pathlib import Path
from typing import NamedTuple, Optional


TOOLING_ROOT = Path(__file__).resolve().parents[2]
HOME_VARIABLE = "ACCELERATOR_HOME"
STATE_VARIABLE = "ACCELERATOR_STATE_DIR"
PROJECT_VARIABLE = "ACCELERATOR_PROJECT_DIR"
STATE_PREFIXES = ("project-brain/", "memory-bank/")
# What an attached state directory holds of the accelerator's: the runtime's
# layout and the launcher's record and launch files.
STATE_ENTRIES = ("project-brain", "memory-bank", "launch", "accelerator-attach.json")
# Windows reports symbolic links and junctions as reparse points with these
# tags (IO_REPARSE_TAG_SYMLINK, IO_REPARSE_TAG_MOUNT_POINT).
LINK_REPARSE_TAGS = (0xA000000C, 0xA0000003)


# A named tuple, not a dataclass: `dataclasses` imports `inspect` (and with
# it ast, dis and tokenize), about 9 ms of every hook's start for this one
# class.
class Roots(NamedTuple):
    tooling: Path
    project: Path
    state: Path

    @property
    def attached(self) -> bool:
        return self.project != self.state


def _configured(variable: str) -> Optional[Path]:
    value = os.environ.get(variable, "").strip()
    if not value:
        return None
    return Path(value).expanduser()


def _names_this_copy() -> bool:
    home = _configured(HOME_VARIABLE)
    try:
        return home is not None and home.resolve() == TOOLING_ROOT
    except OSError:
        return False


def linked_state_entry(state: Path) -> Optional[Path]:
    """The state directory, or an accelerator entry in it, that is a symbolic link.

    A state directory standing as a link took the project's memory - and
    the launcher's record and Claude's system prompt - to wherever it
    pointed, the project itself included; a link at one of its entries took
    that part. Looked at before the runtime writes anything: the state is
    its owner's alone (0700), and the launcher makes it without following a
    link (scripts/accelerator_attach.py), so nobody else can put one there
    afterwards. Directories above the state are the person's own setting.
    """
    for path in (state, *(state / name for name in STATE_ENTRIES)):
        try:
            status = os.lstat(path)
        except OSError:
            continue
        if stat.S_ISLNK(status.st_mode) or getattr(status, "st_reparse_tag", 0) in LINK_REPARSE_TAGS:
            return path
    return None


def long_path(path: Path) -> Path:
    """`path` with any Windows 8.3 short name spelled long; elsewhere `path`.

    Path.resolve() spells a short name such as RUNNER~1 long while it resolves
    links, so a root spelled short never equalled its resolution and was
    refused as a link. GetLongPathNameW expands short names and keeps every
    link, so its answer still differs from resolve() where a link is.
    """
    if os.name != "nt":
        return path
    import ctypes
    from ctypes import wintypes
    expand = ctypes.WinDLL("kernel32", use_last_error=True).GetLongPathNameW
    expand.argtypes = (wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD)
    expand.restype = wintypes.DWORD
    text = str(path)
    size = expand(text, None, 0)
    if not size:
        return path
    buffer = ctypes.create_unicode_buffer(size)
    if not expand(text, buffer, size):
        return path
    return Path(buffer.value)


def change_time_ns(image: os.stat_result) -> int:
    """st_ctime_ns as a path's stat and an open file's fstat both report it.

    Python 3.12+ on Windows gives os.stat() and os.lstat() the creation time
    as st_ctime_ns and os.fstat() the last change time, so a file nobody
    touched compared unequal to itself. Both report the creation time as
    st_birthtime_ns there, as earlier Pythons do st_ctime_ns; elsewhere
    st_ctime_ns is the inode change time in both.
    """
    if os.name == "nt":
        birth = getattr(image, "st_birthtime_ns", None)
        if birth is not None:
            return birth
    return image.st_ctime_ns


def configuration_error() -> Optional[str]:
    """Why the attached variables cannot be honoured, or None.

    Only checked when they name this copy: a variable meant for another
    copy is somebody else's session, not a mistake here.
    """
    if not _names_this_copy():
        return None
    state = _configured(STATE_VARIABLE)
    project = _configured(PROJECT_VARIABLE)
    if state is None or project is None:
        return (
            f"{HOME_VARIABLE} attaches this accelerator, so {STATE_VARIABLE} "
            f"and {PROJECT_VARIABLE} must both be set"
        )
    if not state.is_absolute() or not project.is_absolute():
        return f"{STATE_VARIABLE} and {PROJECT_VARIABLE} must be absolute paths"
    if not project.is_dir():
        return f"{PROJECT_VARIABLE} is not an existing directory: {project}"
    linked = linked_state_entry(state)
    if linked is not None:
        return (
            f"{linked} is a symbolic link; the attached state ({STATE_VARIABLE}) "
            "is never reached through one"
        )
    try:
        if project.resolve() == state.resolve():
            return f"{STATE_VARIABLE} must not be the project itself"
        state.resolve().relative_to(project.resolve())
    except ValueError:
        return None
    except OSError as error:
        return f"Cannot resolve the attached layout: {error}"
    return f"{STATE_VARIABLE} must lie outside the project it serves"


def default_state_root() -> Path:
    """The `--root` default: the attached state directory, else this install."""
    if _names_this_copy():
        state = _configured(STATE_VARIABLE)
        if state is not None and state.is_absolute():
            return state
    return TOOLING_ROOT


@functools.lru_cache(maxsize=32)
def _roots(repository: str, home: str, state: str, project: str) -> Roots:
    root = Path(repository)
    if home and state and project:
        try:
            home_path = Path(home).expanduser().resolve()
            state_path = Path(state).expanduser()
            project_path = Path(project).expanduser()
            if (
                home_path == TOOLING_ROOT
                and state_path.is_absolute()
                and project_path.is_absolute()
                and state_path.resolve() == root.resolve()
                and project_path.is_dir()
                and project_path.resolve() != root.resolve()
            ):
                return Roots(TOOLING_ROOT, project_path, root)
        except OSError:
            pass
    return Roots(root, root, root)


def roots(repository: Path) -> Roots:
    """The three roots for a runtime whose state root is `repository`.

    Attached mode needs all three variables, ACCELERATOR_HOME naming this copy
    and ACCELERATOR_STATE_DIR naming `repository`; anything else is the
    installed layout, so a stray or stale variable can never move an
    installed project's state.
    """
    return _roots(
        str(repository),
        os.environ.get(HOME_VARIABLE, "").strip(),
        os.environ.get(STATE_VARIABLE, "").strip(),
        os.environ.get(PROJECT_VARIABLE, "").strip(),
    )


def project_root(repository: Path) -> Path:
    return roots(repository).project


def tooling_root(repository: Path) -> Path:
    return roots(repository).tooling


def is_attached(repository: Path) -> bool:
    return roots(repository).attached


def is_state_key(key: str) -> bool:
    return key.startswith(STATE_PREFIXES)


def resolve(repository: Path, key: str) -> Path:
    """The file a document or citation key names; a `#anchor` is ignored."""
    relative = key.split("#", 1)[0]
    layout = roots(repository)
    if Path(relative).is_absolute():
        return Path(relative)
    if layout.attached and is_state_key(relative):
        return layout.state / relative
    return layout.project / relative


def key_for(repository: Path, base: Path, path: Path) -> str:
    """The key for `path`, found under `base`: relative, or absolute for
    attached tooling, which shares no root with the project."""
    layout = roots(repository)
    if layout.attached and base == layout.tooling:
        return path.as_posix()
    # path.relative_to(base).as_posix() by slicing the strings when `path` is
    # spelled under `base`: pathlib's version was half of a prompt's source
    # walk. Any other spelling takes pathlib's answer, or its ValueError.
    # On Windows the drive decides too: \\srv\a.md is the share a.md on the
    # server srv, not a file under \\srv.
    text, prefix = str(path), str(base)
    if (len(text) > len(prefix) + 1 and text.startswith(prefix)
            and text[len(prefix)] == os.sep and not prefix.endswith(os.sep)
            and path.drive == base.drive):
        relative = text[len(prefix) + 1:]
        return relative.replace(os.sep, "/") if os.sep != "/" else relative
    return path.relative_to(base).as_posix()


def contains(repository: Path, path: Path) -> bool:
    """Whether `path` lies in a root a key may name: the installed root, or
    in attached mode the project, the state or the tooling."""
    layout = roots(repository)
    try:
        resolved = path.resolve()
    except OSError:
        return False
    for base in dict.fromkeys((layout.project, layout.state, layout.tooling)):
        try:
            resolved.relative_to(base.resolve())
        except (OSError, ValueError):
            continue
        return True
    return False


def describe(repository: Path) -> dict[str, Optional[str]]:
    """The layout as status reports it: which mode, and where each root is."""
    layout = roots(repository)
    return {
        "layout": "attached" if layout.attached else "installed",
        "tooling": str(layout.tooling),
        "project": str(layout.project),
        "state": str(layout.state),
    }


class ContextError(Exception):
    """A safe, user-facing context-engine error."""


class SourceGitError(ContextError):
    """Unknown source policy must not become an empty ignore set or warning."""


_SOURCE_GIT_ERROR = "Git source context unavailable"


def _source_git_environment() -> dict[str, str]:
    # Keep ordinary protected/system/global/repository policy, including global
    # excludesFile and safe.directory. Remove command environment overrides and
    # interactive helpers only in this child; never edit trust configuration.
    # A GIT_DIR, GIT_INDEX_FILE or discovery knob the person's shell exports is
    # dropped rather than refused: every probe runs `git -C <project>`, so it
    # finds the project's own repository, which is the policy that applies to
    # the project's files. Refusing cost the whole turn its memory.
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith("GIT_")}
    environment.update(GIT_OPTIONAL_LOCKS="0", GIT_TERMINAL_PROMPT="0")
    return environment


def _git_directory(marker: Path) -> bool:
    """Whether a `.git` directory is a repository, by Git's own test.

    Git skips a `.git` directory that has no HEAD, objects and refs and keeps
    looking further up - an empty `~/.git` or `/tmp/.git` left behind by some
    tool makes nothing below it a repository. Discovery here must agree, or a
    project outside any repository is mistaken for one whose policy cannot be
    read.
    """
    try:
        head = (marker / "HEAD").lstat()
        return (stat_module.S_ISREG(head.st_mode) or stat_module.S_ISLNK(head.st_mode)) \
            and (marker / "objects").is_dir() and (marker / "refs").is_dir()
    except FileNotFoundError:
        return False


def _source_git_checkout(project: Path) -> Optional[Path]:
    """An unobservable/dangling marker is unknown, not a non-Git folder."""
    try:
        project = project.resolve(strict=True)
        if not stat_module.S_ISDIR(project.stat().st_mode):
            raise SourceGitError(_SOURCE_GIT_ERROR)
        for candidate in (project, *project.parents):
            marker = candidate / ".git"
            try:
                observed = marker.lstat()
            except FileNotFoundError:
                continue
            if (stat_module.S_ISLNK(observed.st_mode)
                    or getattr(observed, "st_reparse_tag", 0) in LINK_REPARSE_TAGS):
                observed = marker.stat()
            if stat_module.S_ISDIR(observed.st_mode):
                if not _git_directory(marker):
                    continue
            elif not stat_module.S_ISREG(observed.st_mode):
                raise SourceGitError(_SOURCE_GIT_ERROR)
            return candidate
        return None
    except (OSError, ValueError, RuntimeError) as error:
        raise SourceGitError(_SOURCE_GIT_ERROR) from error


def _source_git_run(project: Path, arguments: list[str], environment: dict[str, str],
                    deadline: float, payload: Optional[bytes] = None) -> subprocess.CompletedProcess:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise SourceGitError(_SOURCE_GIT_ERROR)
    command = ["git", "--no-optional-locks", "--no-pager",
               "-c", "core.hooksPath=" + os.devnull, "-c", "core.fsmonitor=false",
               "-C", str(project), *arguments]
    try:
        return subprocess.run(command, input=payload, stdout=subprocess.PIPE,
                              stderr=subprocess.DEVNULL, env=environment,
                              timeout=remaining, check=False)
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        raise SourceGitError(_SOURCE_GIT_ERROR) from error


def _source_git_frame(project: Path, environment: dict[str, str], deadline: float, *, _checkout=None, _run=None
                      ) -> Optional[tuple[Path, str, Path]]:
    checkout = (_checkout or _source_git_checkout)(project)
    if checkout is None:
        return None
    result = (_run or _source_git_run)(project, ["rev-parse", "--show-toplevel", "--show-prefix",
                                      "--absolute-git-dir"], environment, deadline)
    try:
        if result.returncode != 0 or not isinstance(result.stdout, bytes) or len(result.stdout) > 16384:
            raise SourceGitError(_SOURCE_GIT_ERROR)
        # Paths as Python names them on this filesystem: a project under a
        # directory whose name is not UTF-8 is still a project.
        lines = os.fsdecode(result.stdout).split("\n")
        if len(lines) != 4 or lines[-1] != "":
            raise SourceGitError(_SOURCE_GIT_ERROR)
        top, prefix, admin = Path(lines[0]), lines[1], Path(lines[2])
        if not top.is_absolute() or not admin.is_absolute():
            raise SourceGitError(_SOURCE_GIT_ERROR)
        top, admin = top.resolve(strict=True), admin.resolve(strict=True)
        relative = project.resolve(strict=True).relative_to(top).as_posix()
        expected_prefix = "" if relative == "." else relative + "/"
        if (top != checkout or prefix != expected_prefix
                or not stat_module.S_ISDIR(admin.stat().st_mode)):
            raise SourceGitError(_SOURCE_GIT_ERROR)
        return top, prefix, admin
    except (OSError, UnicodeError, ValueError, RuntimeError) as error:
        raise SourceGitError(_SOURCE_GIT_ERROR) from error


def _file_identity(path: Path, *, contents: bool) -> Optional[tuple]:
    """A file's identity, and for a file whose contents matter its size and
    times too; None when it is absent. A directory's times move with every
    lock file Git writes in it, so only its identity counts."""
    try:
        image = path.lstat()
    except FileNotFoundError:
        return None
    if contents and not stat_module.S_ISDIR(image.st_mode):
        return (image.st_dev, image.st_ino, image.st_mode, image.st_size, image.st_mtime_ns, image.st_ctime_ns)
    return (image.st_dev, image.st_ino, image.st_mode)


def _frame_identity(project: Path, frame: tuple[Path, str, Path]) -> Optional[tuple]:
    """What decides a Git frame, by file identity: every directory from the
    project up to its top, each `.git` marker on the way, and the admin
    directory with its config and commondir. A retarget - a marker replaced
    or repointed, a repository created nearer, a worktree moved, core.worktree
    edited - changes at least one of them. None when it cannot be observed."""
    top, _, admin = frame
    try:
        rows = []
        resolved = project.resolve(strict=True)
        for directory in (resolved, *resolved.parents):
            rows.append(_file_identity(directory, contents=False))
            rows.append(_file_identity(directory / ".git", contents=True))
            if directory == top:
                break
        else:
            return None
        for leaf, contents in ((admin, False), (admin / "config", True), (admin / "commondir", True)):
            rows.append(_file_identity(leaf, contents=contents))
        return tuple(rows)
    except (OSError, RuntimeError, ValueError):
        return None


def branch_snapshot(project: Path) -> Optional[tuple]:
    """What a branch switch rewrites, by file identity: the checkout's HEAD -
    Git replaces it through a lock file, so it is a new file each time - and,
    under the reftable backend, the table list. None when unobservable."""
    try:
        checkout = _source_git_checkout(project)
        if checkout is None:
            return None
        marker = checkout / ".git"
        image = marker.stat()
        if stat_module.S_ISDIR(image.st_mode):
            admin = marker.resolve(strict=True)
        elif stat_module.S_ISREG(image.st_mode):
            text = marker.read_text(encoding="utf-8")
            if not text.startswith("gitdir: "):
                return None
            pointer = text[len("gitdir: "):].strip()
            admin = (Path(pointer) if os.path.isabs(pointer) else checkout / pointer).resolve(strict=True)
        else:
            return None
        head = _file_identity(admin / "HEAD", contents=True)
        if head is None:
            return None
        return (str(admin), _file_identity(marker, contents=True), head,
                _file_identity(admin / "reftable" / "tables.list", contents=True))
    except (OSError, UnicodeError, ValueError, RuntimeError, SourceGitError):
        return None


# Git frames confirmed within one command, with their identities: the frame
# a second ignore check needs is taken from here while every file that decides
# it is unchanged, instead of asking Git again. Outside a scope - and for the
# first check in one - each check still asks Git before and after its query.
_FRAME_SCOPE: Optional[dict] = None


@contextlib.contextmanager
def source_frame_scope():
    """Share confirmed Git frames among the ignore checks of one command."""
    global _FRAME_SCOPE
    previous, _FRAME_SCOPE = _FRAME_SCOPE, {}
    try:
        yield
    finally:
        _FRAME_SCOPE = previous


def command_memo(name: str) -> Optional[dict]:
    """A dictionary that lives as long as the current command's scope (see
    source_frame_scope), or None outside one: a long-lived process - the
    memory MCP server - never carries an answer from one request into the
    next."""
    scope = _FRAME_SCOPE
    if scope is None:
        return None
    return scope.setdefault(("memo", name), {})


# `git check-ignore` asks the index about every path it is given - about a
# tenth of a millisecond a path in a checkout of 20,000 files - so a project
# with that many documents outlasted the two seconds the check has, and every
# prompt's refresh failed with "Git source context unavailable". Above this
# many paths the rules are asked without the index (milliseconds), and the
# tracked files among the paths they name - which Git never counts as
# ignored - are taken out with `ls-files`, only when there are any.
IGNORE_INDEX_PATHS = 256
# Characters of path arguments one `ls-files` gets (Windows: 32,767 in all).
IGNORE_TRACKED_ARGUMENTS = 16000


# Above this many named paths `ls-files` is asked once, for the directories
# they lie in (two levels deep), instead of path by path in batches.
IGNORE_TRACKED_BY_DIRECTORY = 64
IGNORE_TRACKED_OUTPUT = 64 * 1024 * 1024


def _tracked_among(project: Path, frame: tuple[Path, str, Path], members: list[bytes],
                   environment: dict[str, str], deadline: float, run) -> set[bytes]:
    """Which of `members` (paths as check-ignore named them) the index tracks."""
    tracked: set[bytes] = set()
    wanted = set(members)
    names = [os.fsdecode(member) for member in members]
    if len(names) > IGNORE_TRACKED_BY_DIRECTORY:
        prefixes = sorted({"/".join(name.split("/")[:2]) if name.count("/") >= 2 else name for name in names})
        if sum(len(prefix) + 1 for prefix in prefixes) < IGNORE_TRACKED_ARGUMENTS:
            result = run(project, ["--literal-pathspecs", "--git-dir=" + str(frame[2]),
                                   "--work-tree=" + str(frame[0]), "ls-files", "-z", "--cached", "--",
                                   *prefixes], environment, deadline)
            output = result.stdout
            if (result.returncode != 0 or not isinstance(output, bytes) or len(output) > IGNORE_TRACKED_OUTPUT
                    or (output and not output.endswith(b"\0"))):
                raise SourceGitError(_SOURCE_GIT_ERROR)
            return {listed for listed in (output[:-1].split(b"\0") if output else ()) if listed in wanted}
    batch: list[str] = []
    size = 0
    for index, name in enumerate(names):
        batch.append(name)
        size += len(name) + 1
        if size < IGNORE_TRACKED_ARGUMENTS and index + 1 < len(names):
            continue
        result = run(project, ["--literal-pathspecs", "--git-dir=" + str(frame[2]),
                               "--work-tree=" + str(frame[0]), "ls-files", "-z", "--cached", "--", *batch],
                     environment, deadline)
        output = result.stdout
        # An unmerged path is listed once a stage: three times at most.
        if (result.returncode != 0 or not isinstance(output, bytes)
                or len(output) > 3 * size or (output and not output.endswith(b"\0"))):
            raise SourceGitError(_SOURCE_GIT_ERROR)
        for listed in output[:-1].split(b"\0") if output else ():
            if listed not in wanted:
                raise SourceGitError(_SOURCE_GIT_ERROR)
            tracked.add(listed)
        batch, size = [], 0
    return tracked


def _source_git_ignored(project: Path, paths: list[str], timeout: float, *,
                        _environment=None, _frame=None, _run=None) -> set[bytes]:
    if not paths:
        return set()
    environment = (_environment or _source_git_environment)()
    deadline = time.monotonic() + timeout
    frame = _frame or _source_git_frame
    scope = _FRAME_SCOPE
    key = (os.fspath(project), tuple(sorted(environment.items())))
    held = scope.get(key) if scope is not None else None
    before = held[0] if held is not None and _frame_identity(project, held[0]) == held[1] else None
    confirmed = before is not None
    if before is None:
        before = frame(project, environment, deadline)
    if before is None:
        return set()
    try:
        requested = [os.fsencode(path) for path in paths if isinstance(path, str)]
        if len(requested) != len(paths) or any(not path or b"\0" in path for path in requested):
            raise SourceGitError(_SOURCE_GIT_ERROR)
        payload = b"".join(path + b"\0" for path in requested)
        run = _run or _source_git_run
        unindexed = len(requested) > IGNORE_INDEX_PATHS
        result = run(project, ["--git-dir=" + str(before[2]),
                               "--work-tree=" + str(before[0]),
                               "check-ignore", *(["--no-index"] if unindexed else []), "--stdin", "-z"],
                     environment, deadline, payload)
        named = (result.stdout[:-1].split(b"\0") if unindexed and result.returncode == 0
                 and isinstance(result.stdout, bytes) and result.stdout.endswith(b"\0")
                 and len(result.stdout) <= len(payload) else [])
        tracked = (_tracked_among(project, before, [name for name in named if name], environment, deadline, run)
                   if named else set())
        # Rediscover from the original project, without the pinned query's Git
        # directory. This can observe retargeting; it is not an atomic snapshot.
        # A frame this command already confirmed is checked by its identity;
        # the identity is taken before Git is asked, so a retarget after it
        # shows on the next check.
        identity = _frame_identity(project, before) if scope is not None else None
        if not (confirmed and identity is not None and identity == held[1]):
            if frame(project, environment, deadline) != before:
                raise SourceGitError(_SOURCE_GIT_ERROR)
            if scope is not None and identity is not None:
                scope[key] = (before, identity)
        output = result.stdout
        # Admission is after communicate/capture. It does not bound producer
        # allocation, process cleanup, or the whole caller's wall clock.
        if not isinstance(output, bytes) or len(output) > len(payload):
            raise SourceGitError(_SOURCE_GIT_ERROR)
        if result.returncode == 1 and output == b"":
            return set()
        if result.returncode != 0 or not output or not output.endswith(b"\0"):
            raise SourceGitError(_SOURCE_GIT_ERROR)
        counts: dict[bytes, int] = {}
        for path in requested:
            counts[path] = counts.get(path, 0) + 1
        members = output[:-1].split(b"\0")
        for path in members:
            if not path or counts.get(path, 0) <= 0:
                raise SourceGitError(_SOURCE_GIT_ERROR)
            counts[path] -= 1
        return set(members) - tracked
    except (OSError, UnicodeError, ValueError) as error:
        raise SourceGitError(_SOURCE_GIT_ERROR) from error


# Own document custody differs from strict project citation custody. Brain
# records have their own governed admission and are not ordinary discovery.
ACCELERATOR_OWNED_FILES = frozenset({"AGENTS.md"})
ACCELERATOR_OWNED_PREFIXES = (
    ".agents/skills/", ".claude/skills/", ".cursor/skills/", ".codex/skills/",
    "memory-bank/chunks/", "specs/", "tasks/",
)


def accelerator_owned_source(name: str) -> bool:
    return name in ACCELERATOR_OWNED_FILES or name.startswith(ACCELERATOR_OWNED_PREFIXES)


def project_source_key(repository: Path, name: str) -> Optional[str]:
    """A local project key for Git; external attached state/tooling stay local.

    Resolve through the installed/attached contract, never the database path or
    process CWD. Unknown/outside keys fail closed. URL citations are not files.
    """
    if not isinstance(name, str) or not name or "\0" in name:
        raise SourceGitError(_SOURCE_GIT_ERROR)
    if name.startswith(("https://", "http://")):
        return None
    try:
        layout = roots(repository)
        named = resolve(repository, name.split("#", 1)[0])
        lexical = Path(os.path.abspath(named))
        project = layout.project.resolve()
        physical = named.resolve()
        # Decide the key's root lexically; following a project symlink cannot
        # grant attached tooling/state exemption. Git matches lexical names.
        try:
            key = lexical.relative_to(project).as_posix()
        except ValueError:
            if layout.attached and any(lexical.is_relative_to(bound.resolve())
                    and physical.is_relative_to(bound.resolve())
                    for bound in (layout.state, layout.tooling)):
                return None
            raise SourceGitError(_SOURCE_GIT_ERROR)
        if not physical.is_relative_to(project):
            raise SourceGitError(_SOURCE_GIT_ERROR)
        return key
    except (OSError, ValueError, RuntimeError) as error:
        raise SourceGitError(_SOURCE_GIT_ERROR) from error


def ignored_source_keys(repository: Path, names, *, strict: bool = False,
                        timeout: float = 2.0) -> set[str]:
    mapped = {}
    for name in names:
        if not strict and (accelerator_owned_source(name) or name.startswith("project-brain/")):
            continue
        key = project_source_key(repository, name)
        if key is not None:
            mapped[name] = key
    ignored = _source_git_ignored(project_root(repository), sorted(set(mapped.values())), timeout)
    return {name for name, key in mapped.items() if os.fsencode(key) in ignored}
