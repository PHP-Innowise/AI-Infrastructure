"""Updates of this clone, offered the way a desktop application offers them.

The Harness runs from a Git clone of the accelerator, so an update is the branch the clone follows moving on:
`main` on origin, for a clone made to use the accelerator. A check fetches that branch when the server starts and
then hourly, and counts the commits the clone lacks. While there are any, the page shows an Update button.
Updating fast-forwards the clone, rewrites the installed desktop application from it, and restarts the server on
the new code; the page then reloads itself. Projects attached to the clone take their edition from it, so they are
updated with it.

Only a fast-forward is ever applied. A clone with commits of its own, or local changes in the way, is reported and
left as it is.
"""
from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path
import threading
import time

from .sessions import SessionError, run_git

ROOT = Path(__file__).resolve().parents[3]
FIRST_CHECK_SECONDS = 15
CHECK_SECONDS = 3600
# A page that comes back asks for a check once the last one is this old.
PAGE_CHECK_SECONDS = 600
FETCH_SECONDS = 60
MERGE_SECONDS = 120
COMMITS_SHOWN = 20


class UpToDate(SessionError):
    """Nothing to apply: the clone already has everything its branch has."""


def _now():
    return datetime.now(timezone.utc).isoformat(timespec='seconds')


def _reason(result):
    lines = [line.strip() for line in (result.stderr or result.stdout or '').splitlines() if line.strip()]
    return lines[0][:300] if lines else f'git exited with status {result.returncode}.'


class Updates:
    """The clone's update state, checked in the background, applied on request."""

    def __init__(self, root=ROOT, busy=None):
        self.root = Path(root)
        # How many runs a restart would interrupt; an update waits until there are none.
        self.busy = busy or (lambda: 0)
        self.lock = threading.Lock()
        self.applying = threading.Lock()
        self.state = {'state': 'unknown', 'detail': '', 'checked_at': None, 'behind': 0, 'ahead': 0, 'commits': [],
                      'branch': None, 'upstream': None, 'version': None}
        self.checked = None
        self.stopping = threading.Event()

    def _git(self, *args, timeout=30):
        return run_git(self.root, *args, timeout=timeout)

    def _value(self, *args):
        result = self._git(*args)
        return result.stdout.strip() if result.returncode == 0 else None

    def _tracking(self):
        """The remote branch the clone follows, or why it follows none."""
        branch = self._value('symbolic-ref', '--quiet', '--short', 'HEAD')
        if not branch:
            return None, 'This clone is not on a branch, so it has no branch to update from.'
        remote = self._value('config', '--get', f'branch.{branch}.remote')
        merge = self._value('config', '--get', f'branch.{branch}.merge')
        if remote and remote != '.' and merge and merge.startswith('refs/heads/'):
            return f'{remote}/{merge[len("refs/heads/"):]}', None
        # A clone of main follows origin's main whether or not the branch was set up to track it.
        if branch == 'main' and self._value('remote', 'get-url', 'origin'):
            return 'origin/main', None
        return None, f'The branch {branch} follows no remote branch, so there is nothing to update from.'

    def _compute(self, fetch):
        status = {'state': 'failed', 'detail': '', 'checked_at': _now(), 'behind': 0, 'ahead': 0, 'commits': [],
                  'branch': None, 'upstream': None, 'version': None}
        try:
            if self._value('rev-parse', '--is-inside-work-tree') != 'true':
                return {**status, 'state': 'unavailable', 'detail': 'This copy of the accelerator is not a Git clone, so it cannot update itself.'}
            upstream, reason = self._tracking()
            status.update(branch=self._value('symbolic-ref', '--quiet', '--short', 'HEAD'), version=self._value('rev-parse', '--short', 'HEAD'))
            if upstream is None:
                return {**status, 'state': 'unavailable', 'detail': reason}
            status['upstream'] = upstream
            remote, _, branch = upstream.partition('/')
            if fetch:
                fetched = self._git('fetch', '--quiet', '--no-tags', remote, branch, timeout=FETCH_SECONDS)
                if fetched.returncode != 0:
                    return {**status, 'detail': f'Could not check {upstream} for updates: {_reason(fetched)}'}
            counts = self._value('rev-list', '--left-right', '--count', f'HEAD...refs/remotes/{upstream}')
            if not counts:
                return {**status, 'detail': f'{upstream} is not known in this clone yet.'}
            ahead, behind = (int(value) for value in counts.split())
            log = self._value('log', f'-{COMMITS_SHOWN}', '--format=%h%x09%s', f'HEAD..refs/remotes/{upstream}') if behind else ''
            commits = [dict(zip(('hash', 'subject'), line.split('\t', 1))) for line in (log or '').splitlines() if '\t' in line]
            changes = f'{behind} new {"change" if behind == 1 else "changes"} on {upstream}'
            if not behind:
                return {**status, 'state': 'current', 'ahead': ahead}
            if ahead:
                return {**status, 'state': 'diverged', 'ahead': ahead, 'behind': behind, 'commits': commits,
                        'detail': f'{changes}, and this clone has {ahead} {"commit" if ahead == 1 else "commits"} of its own: update it with Git.'}
            return {**status, 'state': 'available', 'behind': behind, 'commits': commits, 'detail': changes + '.'}
        except SessionError as error:
            return {**status, 'detail': str(error)}

    def status(self):
        with self.lock:
            return {**self.state, 'commits': list(self.state['commits'])}

    def check(self, fetch=True):
        """Fetch the followed branch and count what the clone lacks. A failed check is a state, never an error."""
        status = self._compute(fetch)
        with self.lock:
            if self.state['state'] != 'updating':
                self.state = status
            self.checked = time.monotonic()
        return self.status()

    def poke(self):
        """A page came back: check in the background once the last check is old, and answer with what is known."""
        with self.lock:
            stale = self.state['state'] != 'updating' and (self.checked is None or time.monotonic() - self.checked > PAGE_CHECK_SECONDS)
            if stale:
                self.checked = time.monotonic()  # one check at a time
        if stale:
            threading.Thread(target=self.check, name='harness-update-check', daemon=True).start()
        return self.status()

    def start(self):
        """Check shortly after the server starts, then hourly."""
        def loop():
            delay = FIRST_CHECK_SECONDS
            while not self.stopping.wait(delay):
                self.check()
                delay = CHECK_SECONDS
        threading.Thread(target=loop, name='harness-updates', daemon=True).start()

    def close(self):
        self.stopping.set()

    def apply(self):
        """Fast-forward the clone to the branch it follows. The caller then restarts the server on the new code."""
        if not self.applying.acquire(blocking=False):
            raise SessionError('The update is already running.')
        try:
            running = self.busy()
            if running:
                raise SessionError(f'{running} {"run is" if running == 1 else "runs are"} in progress, and updating restarts the '
                                   'Harness. Let them finish or stop them, then update.')
            status = self._compute(True)
            if status['state'] == 'current':
                with self.lock:
                    self.state = status
                raise UpToDate('This clone is already up to date.')
            if status['state'] != 'available':
                with self.lock:
                    self.state = status
                raise SessionError(status['detail'] or 'There is no update to apply.')
            with self.lock:
                self.state = {**status, 'state': 'updating', 'detail': 'Updating…'}
            merged = self._git('merge', '--ff-only', '--quiet', f'refs/remotes/{status["upstream"]}', timeout=MERGE_SECONDS)
            if merged.returncode != 0:
                detail = f'The update could not be applied, and the clone is unchanged: {_reason(merged)}'
                with self.lock:
                    self.state = {**status, 'state': 'failed', 'detail': detail}
                raise SessionError(detail)
            version = self._value('rev-parse', '--short', 'HEAD')
            with self.lock:
                self.state = {**status, 'state': 'updated', 'version': version, 'behind': 0, 'commits': [],
                              'detail': f'Updated to {version}. The Harness is restarting.'}
            return {'from': status['version'], 'to': version, 'changes': status['behind']}
        finally:
            self.applying.release()


def refresh_application():
    """Rewrite the installed desktop application from this clone, if it is installed from here: its icon and entry
    may have changed with the update. Returns what was rewritten; a failure is reported, never raised."""
    try:
        from . import desktop_app
        config = desktop_app.load_config()
        if config.get('clone') != str(desktop_app.ROOT) or desktop_app.installed_entry() is None:
            return []
        return [str(path) for path in desktop_app.install()]
    except Exception as error:  # the update itself succeeded; the old entry still runs this clone
        return [f'The application entry was not refreshed: {error}']
