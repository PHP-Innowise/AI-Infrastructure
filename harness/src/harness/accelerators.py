"""Accelerator editions lent to registered projects from this clone.

A project registered in the browser is attached to the edition its own files
point to (composer.json, artisan, bin/console, WordPress headers); nothing is
copied into it. Sessions then launch the native CLI with the edition from this
clone - its policy, skills, agents, commands and hooks - and keep the
accelerator's state for the project under this server's state directory, at
`attached/<project id>`, where `scripts/accelerator_attach.py` finds the same
memory from a terminal. A project with an installed accelerator keeps using
its own files; attaching is for every other project.

An installed copy is kept current instead: once per version of this clone,
`install_accelerator.sync_installation` brings the project's untouched
accelerator files and its runtime up to the clone, so every session in the
project - including the native ones Harness never sees - runs today's
memory. Nothing a person owns is written (see that function).

Codex runs a project's hooks only after their definitions were approved, so
the accelerator's memory hooks never ran where nobody approved them in /hooks.
With Codex available, the same pass approves the accelerator's own hook
definitions - installed or lent - in the user's Codex config, the record
Codex's own review writes; a team's hook or an edited one is left for review.
HARNESS_CODEX_HOOK_TRUST=0 turns this off.
"""

from __future__ import annotations

import dataclasses
import hashlib
import json
import os
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any, Optional

from .sessions import SessionError, now, read_context

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts'))
import accelerator_attach as attach  # noqa: E402
import install_accelerator as installer  # noqa: E402

TABLE = '''CREATE TABLE IF NOT EXISTS project_accelerators (
    project_id TEXT PRIMARY KEY, edition TEXT NOT NULL, attached_at TEXT NOT NULL)'''
# The clone version each installed project was last brought up to, and what
# that sync did.
SYNC_TABLE = '''CREATE TABLE IF NOT EXISTS accelerator_syncs (
    project_id TEXT PRIMARY KEY, source TEXT NOT NULL, synced_at TEXT NOT NULL, report TEXT NOT NULL)'''
INSTALLED_MARKER = 'memory-bank/scripts/context.py'
# How long the clone's version is trusted before Git is asked again.
SOURCE_VERSION_TTL = 30


class Accelerators:
    def __init__(self, sessions):
        self.sessions = sessions
        self.lock = threading.Lock()
        self.sync_lock = threading.Lock()
        self._source = (0.0, None)
        with sessions.lock:
            sessions.db.execute(TABLE)
            sessions.db.execute(SYNC_TABLE)
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

    def summary(self, project_id) -> dict[str, Optional[str]]:
        """Mode and edition only, for project lists: no file is parsed."""
        path = Path(self.sessions.project(project_id)['path'])
        if path.is_dir() and self.installed(path):
            return {'mode': 'installed', 'edition': self._edition(project_id)}
        edition = self._edition(project_id)
        return {'mode': 'attached' if edition else None, 'edition': edition}

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
            result['sync'] = self.last_sync(project_id)
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

    # -- keeping an installed copy current -----------------------------------

    def source_version(self) -> str:
        """This clone's version: its commit, and a digest of any uncommitted
        change to what an install copies, so a working clone counts too."""
        checked, version = self._source
        if version is not None and time.monotonic() - checked < SOURCE_VERSION_TTL:
            return version
        version = installer.source_commit(ROOT) or 'no-git'
        try:
            status = subprocess.run(
                ['git', '-C', str(ROOT), 'status', '--porcelain', '-z', '--', 'install/inventories',
                 *(str(path) for path in installer.EDITION_PATHS.values())],
                stdin=subprocess.DEVNULL, capture_output=True, timeout=20)
            if status.returncode == 0 and status.stdout:
                version += '+' + hashlib.sha256(status.stdout).hexdigest()[:12]
        except (OSError, subprocess.SubprocessError):
            pass
        self._source = (time.monotonic(), version)
        return version

    def codex_for_trust(self) -> Optional[str]:
        """The Codex CLI to approve the accelerator's hooks with, or None.

        HARNESS_CODEX_HOOK_TRUST=0 turns the approval off; the hooks then wait
        for a person's review in Codex's /hooks, as they do outside the Harness.
        """
        if os.environ.get('HARNESS_CODEX_HOOK_TRUST', '').strip().lower() in ('0', 'off', 'false', 'no'):
            return None
        provider = self.sessions.providers.get('codex') or {}
        # Only the CLI discovery verified: the approvals go into that CLI's config.
        if not provider.get('available') or not provider.get('executable'):
            return None
        return provider['executable']

    def keep_current(self, project_id) -> Optional[dict[str, Any]]:
        """Bring a project's accelerator up to this clone, once per clone version.

        Installed: the project's accelerator files are synced, and the Codex
        hooks among them - whose wiring may be rewritten only when they can be
        approved again - are approved for Codex. Attached: nothing is written
        into the project, and the hooks the launches lend it are approved.
        Returns the report when one ran, else None. Never raises: a project that
        cannot be synced keeps what it has, and the report says why.
        """
        try:
            path = Path(self.sessions.project(project_id)['path'])
        except SessionError:
            return None
        if not path.is_dir():
            return None
        installed = self.installed(path)
        codex = self.codex_for_trust()
        edition = None if installed else self._edition(project_id)
        # Attached, nothing is ever written into the project: the one thing to
        # keep current is Codex's approval of the hooks the launches lend it.
        if not installed and not (edition and codex):
            return None
        version = self.source_version() + ('+codex' if codex else '')
        with self.sessions.lock:
            row = self.sessions.db.execute(
                'SELECT source FROM accelerator_syncs WHERE project_id=?', (project_id,)).fetchone()
        if row and row['source'] == version:
            return None
        with self.sync_lock:
            if installed:
                report = self._sync_installed(path, codex)
            else:
                report = {'target': str(path), 'edition': edition, 'release': None, 'changed': [], 'kept': [],
                          'backups': [], 'error': None}
                report['codex_trust'] = self._trust_attached(codex, edition, path)
            with self.sessions.lock:
                self.sessions.db.execute(
                    'INSERT INTO accelerator_syncs(project_id,source,synced_at,report) VALUES (?,?,?,?) '
                    'ON CONFLICT(project_id) DO UPDATE SET source=excluded.source,synced_at=excluded.synced_at,'
                    'report=excluded.report',
                    (project_id, version, now(), json.dumps(report, ensure_ascii=False)))
                self.sessions.db.commit()
        return report

    @staticmethod
    def _trust_attached(codex: str, edition: str, path: Path) -> dict[str, Any]:
        def untrusted(hooks):
            return sum(1 for hook in hooks if hook['status'] not in ('trusted', 'managed'))
        try:
            hooks = attach.codex_hook_trust(codex, edition, path)
            before = untrusted(hooks)
            if before:
                hooks = attach.trust_codex_hooks(codex, edition, path)
            return {'approved': before - untrusted(hooks), 'already': len(hooks) - before, 'left': [
                hook['key'] for hook in hooks if hook['status'] not in ('trusted', 'managed')]}
        except (attach.AttachError, OSError, ValueError) as error:
            return {'error': str(error)}

    def _sync_installed(self, path: Path, codex: Optional[str]) -> dict[str, Any]:
        # Read and restored as the sync writes it: never through a link inside
        # the project. Wiring reached through one is not rewired, so there is
        # nothing to approve or put back.
        wiring = '.codex/hooks.json'
        try:
            before = installer.read_confined(path, wiring)
        except (installer.InventoryError, OSError):
            before = None
        try:
            report = installer.sync_installation(ROOT, path, rewire_codex=codex is not None and before is not None)
        except (installer.InventoryError, OSError, ValueError) as error:
            return {'target': str(path), 'changed': [], 'kept': [], 'backups': [], 'error': str(error)}
        if codex and before is not None and report.get('edition') and not report.get('error'):
            try:
                report['codex_trust'] = attach.trust_installed_codex_hooks(codex, report['edition'], path)
            except (attach.AttachError, OSError, ValueError) as error:
                report['codex_trust'] = {'error': str(error)}
                # Rewired but not approved, the hooks would stop running: the
                # wiring Codex already approved goes back - never through a
                # link, one swapped in meanwhile included (write_confined).
                try:
                    if installer.read_confined(path, wiring) != before:
                        installer.write_confined(path, wiring, before)
                        report['changed'] = [item for item in report['changed']
                                             if item.get('path') != wiring]
                        report['kept'].append({'path': wiring,
                                               'reason': 'Codex could not approve the new wiring; the old one stays'})
                        # The release's new Codex hooks are on disk but the
                        # wiring that would run them is not: not at the release.
                        note = ('Codex could not approve the new hook wiring, so the approved one stays; '
                                "this release's Codex hooks run after the /hooks review")
                        report['partial'] = f"{report['partial']}; {note}" if report.get('partial') else note
                except (installer.InventoryError, OSError) as restore_error:
                    report['codex_trust']['not_restored'] = (
                        f'the approved wiring could not be put back: {restore_error}')
        return report

    def keep_all_current(self) -> None:
        """Every registered installed project, at server start."""
        for project_id in list(self.sessions.projects):
            if self.sessions.stopping.is_set():
                return
            self.keep_current(project_id)

    def last_sync(self, project_id) -> Optional[dict[str, Any]]:
        with self.sessions.lock:
            row = self.sessions.db.execute(
                'SELECT source, synced_at, report FROM accelerator_syncs WHERE project_id=?', (project_id,)).fetchone()
        if not row:
            return None
        try:
            report = json.loads(row['report'])
        except ValueError:
            report = {}
        return {'source': row['source'], 'synced_at': row['synced_at'],
                'edition': report.get('edition'), 'release': report.get('release'),
                'changed': len(report.get('changed') or []), 'kept': report.get('kept') or [],
                'backups': report.get('backups') or [], 'error': report.get('error'),
                'partial': report.get('partial'), 'codex_trust': report.get('codex_trust')}

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
