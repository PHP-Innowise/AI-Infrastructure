"""Accelerator editions lent to registered projects from this clone.

A project registered in the browser is attached to the edition its own files
point to (composer.json, artisan, bin/console, WordPress headers); nothing is
copied into it. Sessions then launch the native CLI with the edition from this
clone - its policy, skills, agents, commands and hooks - and keep the
accelerator's state for the project under this server's state directory, at
`attached/<project id>`, where `scripts/accelerator_attach.py` finds the same
memory from a terminal. A project with an installed accelerator keeps using
its own files; attaching is for every other project.
"""

from __future__ import annotations

import dataclasses
import json
import os
import shutil
import subprocess
import sys
import threading
from pathlib import Path
from typing import Any, Optional

from .sessions import SessionError, now, read_context

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts'))
import accelerator_attach as attach  # noqa: E402

TABLE = '''CREATE TABLE IF NOT EXISTS project_accelerators (
    project_id TEXT PRIMARY KEY, edition TEXT NOT NULL, attached_at TEXT NOT NULL)'''
INSTALLED_MARKER = 'memory-bank/scripts/context.py'


class Accelerators:
    def __init__(self, sessions):
        self.sessions = sessions
        self.lock = threading.Lock()
        with sessions.lock:
            sessions.db.execute(TABLE)
            sessions.db.commit()

    # -- what a project has --------------------------------------------------

    @staticmethod
    def installed(path) -> bool:
        """The project carries its own copy, which then wins over attaching."""
        return read_context(Path(path), INSTALLED_MARKER) is not None

    def _edition(self, project_id) -> Optional[str]:
        with self.sessions.lock:
            row = self.sessions.db.execute('SELECT edition FROM project_accelerators WHERE project_id=?', (project_id,)).fetchone()
        return row['edition'] if row and row['edition'] in attach.EDITIONS else None

    def state(self, project_id) -> Path:
        return self.sessions.state_dir / 'attached' / project_id

    def get(self, project_id) -> dict[str, Any]:
        project = self.sessions.project(project_id)
        path = Path(project['path'])
        detected, evidence = (None, 'The project folder is unavailable.')
        if path.is_dir():
            detected, evidence = attach.detect_edition(path)
        edition = self._edition(project_id)
        result = {'project_id': project_id, 'mode': None, 'edition': edition, 'detected': detected, 'evidence': evidence,
                  'editions': list(attach.EDITIONS), 'home': None, 'state': str(self.state(project_id))}
        if path.is_dir() and self.installed(path):
            result['mode'] = 'installed'
        elif edition:
            result['mode'] = 'attached'
            result['home'] = str(attach.edition_directory(edition))
        return result

    # -- changing it ---------------------------------------------------------

    def attach(self, project_id, edition=None) -> dict[str, Any]:
        project = self.sessions.project(project_id)
        path = Path(project['path'])
        if not path.is_dir():
            raise SessionError('The project folder is unavailable. Restore it before attaching an accelerator.')
        if edition is None:
            edition, evidence = attach.detect_edition(path)
            if edition is None:
                raise SessionError(f'No accelerator edition fits this project ({evidence}). Choose one.')
        if edition not in attach.EDITIONS:
            raise SessionError('Choose a supported accelerator edition.')
        if self.installed(path):
            raise SessionError('This project has an installed accelerator; sessions already use its own files.')
        try:
            attach.edition_directory(edition)
        except attach.AttachError as error:
            raise SessionError(str(error)) from None
        with self.sessions.lock:
            self.sessions.db.execute(
                'INSERT INTO project_accelerators(project_id,edition,attached_at) VALUES (?,?,?) '
                'ON CONFLICT(project_id) DO UPDATE SET edition=excluded.edition,attached_at=excluded.attached_at',
                (project_id, edition, now()))
            self.sessions.db.commit()
        self._seed(project_id, edition, path)
        return self.get(project_id)

    def detach(self, project_id) -> dict[str, Any]:
        """Stop lending the edition. The state directory is kept: it is this
        project's memory, and attaching again picks it up."""
        self.sessions.project(project_id)
        with self.sessions.lock:
            self.sessions.db.execute('DELETE FROM project_accelerators WHERE project_id=?', (project_id,))
            self.sessions.db.commit()
        return self.get(project_id)

    def auto_attach(self, project_id) -> None:
        """Registration: attach the detected edition unless the project has its own."""
        try:
            path = Path(self.sessions.project(project_id)['path'])
            if self._edition(project_id) or not path.is_dir() or self.installed(path):
                return
            edition, _ = attach.detect_edition(path)
            if edition:
                self.attach(project_id, edition)
        except (SessionError, attach.AttachError, OSError):
            pass

    # -- a launch ------------------------------------------------------------

    def overlay(self, project_id, provider, cwd) -> Optional[attach.Overlay]:
        """What a native launch in `cwd` adds, or None when nothing is attached.

        `cwd` may be a worktree of the project: the session works there, while
        the memory stays the project's, shared by all its checkouts.
        """
        edition = self._edition(project_id)
        cwd = Path(cwd)
        if not edition or self.installed(cwd):
            return None
        try:
            return attach.overlay(provider, edition, cwd, self.state(project_id))
        except (attach.AttachError, OSError) as error:
            raise SessionError(f'The attached accelerator cannot be prepared: {error}') from None

    @staticmethod
    def apply(provider, command, overlay, first_turn) -> list[str]:
        """Merge an overlay into argv built by providers.build_command."""
        return attach.apply_overlay(provider, command, overlay, first_turn)

    def overlays(self, project_id, providers, cwd) -> dict[str, dict[str, Any]]:
        """Overlays for runners that launch the CLIs themselves (Fleet, Clash),
        as plain data for their request file."""
        result = {}
        for provider in dict.fromkeys(providers):
            overlay = self.overlay(project_id, provider, cwd)
            if overlay is not None:
                result[provider] = dataclasses.asdict(overlay)
        return result

    # -- knowledge -------------------------------------------------------------

    def knowledge(self, project_id, folder) -> Optional[dict[str, Any]]:
        """Where Knowledge reads an attached project's state, or None.

        `folder` is the checkout a view or session looks at - the project or one
        of its worktrees; the state is the project's, shared by all of them.
        """
        edition = self._edition(project_id)
        folder = Path(folder)
        if not edition or self.installed(folder):
            return None
        try:
            home = attach.edition_directory(edition)
        except attach.AttachError:
            return None
        state = self.state(project_id)
        return {'state': state, 'home': home, 'project': folder, 'edition': edition,
                'environment': attach.environment(edition, folder, state)}

    def _seed(self, project_id, edition, path) -> None:
        """Lay out the state once, so Knowledge has a Brain and a bank to show
        before the first session; the runtime does it itself otherwise."""
        state = self.state(project_id)
        try:
            attach.prepare_state(edition, path, state)
            subprocess.run([sys.executable, str(attach.edition_directory(edition) / 'memory-bank/scripts/context.py'),
                            'status', '--json'], cwd=state, env={**os.environ, **attach.environment(edition, path, state)},
                           stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
        except (OSError, subprocess.SubprocessError, attach.AttachError):
            pass

    # -- listings for the composer ------------------------------------------

    def home(self, project_id, cwd) -> Optional[Path]:
        """The edition lent to sessions in `cwd`, or None when it has its own."""
        edition = self._edition(project_id)
        if not edition or self.installed(cwd):
            return None
        try:
            return attach.edition_directory(edition)
        except attach.AttachError:
            return None

    def skills(self, project_id, provider) -> list[dict[str, str]]:
        """The attached edition's skills as the `$`/`/` menus list them."""
        edition = self._edition(project_id)
        if not edition:
            return []
        tree = {'claude': '.claude/skills', 'codex': '.agents/skills', 'cursor': '.cursor/skills'}.get(provider)
        return attach.skill_catalogue(edition, tree) if tree else []

    # -- Codex hook approval -------------------------------------------------

    def _codex(self) -> str:
        provider = self.sessions.providers.get('codex') or {}
        executable = provider.get('executable') if provider.get('available') else None
        executable = executable or shutil.which('codex')
        if not executable:
            raise SessionError('Codex CLI is not available on this machine.')
        return executable

    def codex_hooks(self, project_id) -> dict[str, Any]:
        info = self.get(project_id)
        if info['mode'] != 'attached':
            raise SessionError('Attach an accelerator before reviewing its Codex hooks.')
        path = Path(self.sessions.project(project_id)['path'])
        with self.lock:
            try:
                hooks = attach.codex_hook_trust(self._codex(), info['edition'], path)
            except attach.AttachError as error:
                raise SessionError(str(error)) from None
        return {'project_id': project_id, 'hooks': hooks,
                'trusted': sum(1 for hook in hooks if hook['status'] in ('trusted', 'managed')), 'total': len(hooks)}

    def trust_codex_hooks(self, project_id) -> dict[str, Any]:
        info = self.get(project_id)
        if info['mode'] != 'attached':
            raise SessionError('Attach an accelerator before trusting its Codex hooks.')
        path = Path(self.sessions.project(project_id)['path'])
        with self.lock:
            try:
                hooks = attach.trust_codex_hooks(self._codex(), info['edition'], path)
            except attach.AttachError as error:
                raise SessionError(str(error)) from None
        return {'project_id': project_id, 'hooks': hooks,
                'trusted': sum(1 for hook in hooks if hook['status'] in ('trusted', 'managed')), 'total': len(hooks)}
