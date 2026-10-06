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

import functools
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Optional


TOOLING_ROOT = Path(__file__).resolve().parents[2]
HOME_VARIABLE = "ACCELERATOR_HOME"
STATE_VARIABLE = "ACCELERATOR_STATE_DIR"
PROJECT_VARIABLE = "ACCELERATOR_PROJECT_DIR"
STATE_PREFIXES = ("project-brain/", "memory-bank/")


@dataclass(frozen=True)
class Roots:
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
