"""Tool targets and the receipt of a native launch for the run view; standard library only.

`providers.normalize_event(..., targets=True)` attaches what a tool call works on to
its label. The session pump moves those targets onto the stored event within a budget
of their own: the launch's 5000-event / 4 MiB failure limit keeps counting the plain
label, so recording targets never changes whether a launch fails. Paths become
project-relative, a path outside the project keeps only its basename, and commands,
patterns and plan text are redacted. File contents, diffs, tool results and command
output never reach this module.
"""
from __future__ import annotations

from collections import deque
import hashlib
import json
import ntpath
import os
from pathlib import Path, PurePath, PureWindowsPath
import posixpath
import re

from . import filesystem  # noqa: F401 -- puts scripts/ on the import path
from .providers import command_head
from ai_system_lib import SECRET

# Targets stored per launch; past either cap, or once the launch nears its own limit, labels go on alone.
ENRICH_BYTES = 1024 * 1024
ENRICH_EVENTS = 4000
STOP_COUNT = 4500
STOP_BYTES = int(3.5 * 1024 * 1024)
# Events outside the pump's count: plans, compaction dividers and the one limited notice.
PLAN_EVENTS = 100
COMPACTION_EVENTS = 20
EXTRA_EVENTS = PLAN_EVENTS + COMPACTION_EVENTS + 1
EXTRA_BYTES = 256 * 1024
# What the store adds to each extra event (launch_id and at), counted against EXTRA_BYTES.
STORED_OVERHEAD = 96
PLAN_ITEMS = 30
# Plan items kept for merge updates; a plan longer than this keeps its count, not its tail.
PLAN_STATE = 200
PLAN_STATUSES = ('done', 'active', 'pending', 'cancelled')
DETAIL_LIMIT = 300
# Receipt lists: opened and edited paths, then search patterns and the latest commands.
LIST_LIMIT = 40
RECENT_LIMIT = 12
LIMITED = {'kind': 'status', 'targets': 'limited',
           'text': 'Step details are not recorded after this point; steps are still counted.'}
_CALL = re.compile(r'[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}')
# Bidi overrides would let a path or command display as something else; controls would break the line.
_BIDI = re.compile('[\u202a-\u202e\u2066-\u2069]')
_CONTROL = re.compile('[\x00-\x1f\x7f]')
_VALUE = r'("[^"]*"|\'[^\']*\'|[^\s"\'`;|&)]+)'
_REDACTIONS = (
    # --password=x, --db-password x, --token=x, --api-key x; never --filter=Password.
    (re.compile(r'(?i)((?<![\w-])--?(?:[a-z0-9]+[-_])*(?:password|passwd|pass|pwd|secret|token|api[-_]?key|access[-_]?key)'
                r'(?:=|\s+))(?!-)' + _VALUE), r'\1[redacted]'),
    # DB_PASSWORD=x, GITHUB_TOKEN=x, STRIPE_SECRET=x, API_KEY=x: an environment name that ends in a secret word.
    (re.compile(r'(?<![\w-])([A-Z0-9_]*(?:TOKEN|SECRET|PASSWORD|PASSWD|PWD|PASS|KEY|CREDENTIALS?)=)' + _VALUE), r'\1[redacted]'),
    (re.compile(r'(?i)(?<![\w-])([a-z0-9_]*(?:password|passwd|pwd|secret|token|api_?key|access_?key|private_?key)=)' + _VALUE),
     r'\1[redacted]'),
    (re.compile(r'(?i)(\bauthorization:\s*(?:bearer|basic|token|digest)?\s*)([^\s"\']+)'), r'\1[redacted]'),
    # -H "X-Api-Key: abc", --header 'Private-Token: abc': a request header that carries a key.
    (re.compile(r'(?i)((?<!\S)(?:-H|--header)[=\s]*["\']?\s*(?:x-)?(?:api[-_]?key|auth[-_]?token|access[-_]?token|private[-_]?token)'
                r':\s*)([^\s"\']+)'), r'\1[redacted]'),
    # sshpass -p secret, redis-cli -a secret, docker login -p secret: a password given as the next argument.
    (re.compile(r'(?i)(\bsshpass(?:\.exe)?(?:\s+-(?!p\b)\S+)*\s+-p)\s+(?!-)(\S+)'), r'\1 [redacted]'),
    (re.compile(r'(?i)(\bredis-cli(?:\.exe)?\b[^|;&\n]*?\s-a)\s+(?!-)(\S+)'), r'\1 [redacted]'),
    (re.compile(r'(?i)(\b(?:docker|podman)(?:\.exe)?\s+login\b[^|;&\n]*?\s-p)\s+(?!-)(\S+)'), r'\1 [redacted]'),
    (re.compile(r'\b(Bearer\s+)[A-Za-z0-9._~+/=-]{12,}'), r'\1[redacted]'),
    # https://user:secret@host and curl -u user:secret
    (re.compile(r'(\b[A-Za-z][A-Za-z0-9+.-]*://[^/\s:@]+:)[^/\s@]+(@)'), r'\1[redacted]\2'),
    (re.compile(r'((?<!\S)(?:-u|--user)\s+[^\s:]+:)(\S+)'), r'\1[redacted]'),
    (re.compile(r'\b(?:gh[pousr]_[A-Za-z0-9]{16,}|github_pat_[A-Za-z0-9_]{20,}|(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{8,}'
                r'|xox[abposr]-[A-Za-z0-9-]{8,}|glpat-[A-Za-z0-9_-]{16,})'), '[redacted]'),
    (re.compile(r'\beyJ[A-Za-z0-9_-]{8,}(?:\.[A-Za-z0-9_-]+){0,2}'), '[redacted]'),
)
# mysql -psecret: an attached -p value is a password only for these clients (`mysql -p dbname` prompts, so a separate
# value stays; sshpass takes its separate value in _REDACTIONS).
_ATTACHED_PASSWORD = re.compile(r'(?:^|[\s/;&|(])(?:mysql[\w-]*|mariadb[\w-]*|sshpass)(?:\.exe)?\b')
_ATTACHED_VALUE = re.compile(r'(?<!\S)-p(?![\s-])\S+')
READ, EDIT, DELETE, SEARCH = {'read'}, {'edit', 'write', 'multiedit', 'notebookedit'}, {'delete'}, {'grep', 'glob', 'ls'}
COMMAND, HELPER = {'bash', 'shell'}, {'task', 'agent'}
# Claude Code's Skill tool hands the agent a skill's SKILL.md; the call names the skill.
SKILL = 'skill'


def redact_command(text):
    """A command, pattern or plan line with passwords, tokens and keys replaced by [redacted]."""
    if not isinstance(text, str):
        return text
    text = SECRET.sub('[redacted]', text)
    for pattern, replacement in _REDACTIONS:
        text = pattern.sub(replacement, text)
    if _ATTACHED_PASSWORD.search(text):
        text = _ATTACHED_VALUE.sub('-p[redacted]', text)
    return text


def _flavor(root):
    windows = isinstance(root, PureWindowsPath) if isinstance(root, PurePath) else os.name == 'nt'
    return ntpath if windows else posixpath


def _full(path, root):
    """A tool's path as an absolute, normalized path under the launch cwd's flavor."""
    flavor = _flavor(root)
    return flavor.normpath(flavor.join(str(root), path.strip()))


def relative(path, root, alias=None):
    """(project-relative POSIX path, outside) of a tool's path; never an absolute path.

    `root` is the launch cwd (the project or its worktree) and takes relative paths;
    `alias` is another name of the same folder, such as the session's project path.
    The root itself is ''. Outside both only the basename is kept, with outside True.
    Windows roots (a PureWindowsPath, or any root on Windows) compare without case.
    """
    if not isinstance(path, str) or not path.strip():
        return None, False
    flavor = _flavor(root)
    full = _full(path, root)
    for base in (root, alias):
        if base is None or not str(base):
            continue
        base = flavor.normpath(str(base))
        prefix = base if base.endswith(flavor.sep) else base + flavor.sep
        key = flavor.normcase(full)
        if key == flavor.normcase(base):
            return '', False
        if key.startswith(flavor.normcase(prefix)):
            return full[len(prefix):].replace('\\', '/') if flavor is ntpath else full[len(prefix):], False
    return flavor.basename(full), True


# Where a root begins: not inside a longer path or word; after an attached short flag (`-I/p/inc`) or `file://` the
# remainder keeps a leading `./`, so it still reads as a path.
_FREE_START = r'(?<![\w./\\-])'
_ATTACHED_START = r'(?:(?<=\s-[A-Za-z])|(?<=^-[A-Za-z])|(?<=file://))'
# A character that continues a path after `root/`; anything else (a space, a quote, an operator, the end) ends it.
_PATH_CHAR = r'[^\s"\'`;|&<>()]'


def strip_root(text, roots, home=None):
    """A command with each absolute root removed: `cat /p/app/x.php` reads `cat app/x.php`, `cd /p` and `cd /p/` read
    `cd .` and `cd ./`, `-I/p/inc` reads `-I./inc`. With `home`, a path under the home folder that no root covers
    reads `~/…`, so an absolute path outside the project does not show the account name."""
    if not isinstance(text, str):
        return text
    for root, name in [*((root, '.') for root in roots), *([(home, '~')] if home else [])]:
        if root is None:
            continue
        flavor = _flavor(root)
        value = flavor.normpath(str(root)).rstrip('/\\')
        if len(value) < 3:
            continue  # A filesystem or drive root would strip every absolute path.
        flags = re.IGNORECASE if flavor is ntpath else 0
        for variant in {value, value.replace('\\', '/')} if flavor is ntpath else {value}:
            for start, inner in ((_ATTACHED_START, name + '/'), (_FREE_START, '' if name == '.' else name + '/')):
                start += re.escape(variant)
                text = re.sub(start + r'[\\/]+(?=' + _PATH_CHAR + ')', inner, text, flags=flags)
                text = re.sub(start + r'[\\/]+', name + '/', text, flags=flags)
                text = re.sub(start + r'(?![\w./\\-])', name, text, flags=flags)
    return text


def _clean(value, limit):
    if not isinstance(value, str):
        return None
    value = ' '.join(_CONTROL.sub(' ', _BIDI.sub('', value)).split())
    if not value:
        return None
    return value if len(value) <= limit else value[:limit - 1] + '…'


def _size(event):
    return len(json.dumps(event, ensure_ascii=False).encode()) + STORED_OVERHEAD


class RunLedger:
    """What a native launch's tools reported, from every tool event whatever the display budget.

    Saved as the launch's receipt. `failed` and `not_run` count commands (Bash, Shell);
    `failed_steps` counts failed calls of any tool. `created` covers Codex additions and a
    Write to a file that was not there when the call started (where that could not be read,
    a path this launch had not opened or edited); `deleted` covers Codex deletions and
    Cursor's delete tool. Path lists hold project paths in first-touch order; files outside
    the project are counted, each by its own full path, but not listed.

    `watch` is the memory the launch's prompt delivered, as what a read names (a project
    path, or `skill:<name>` for a Skill load) mapped to the delivered item; `outside` says
    that memory lives outside the project, as an attached accelerator's does. See `memory`.
    """

    def __init__(self, provider, watch=None, outside=False):
        self.provider = provider
        self.steps = self.searched = self.commands = self.failed = self.not_run = self.failed_steps = self.helpers = 0
        self.opened, self.edited, self.created, self.deleted, self.patterns = {}, {}, set(), set(), {}
        # Open calls by ID; starts without one wait per tool, oldest first.
        self.calls, self.uncalled = {}, {}
        self.last_commands = deque(maxlen=RECENT_LIMIT)
        self.plan = None
        self.limited = False
        self.watch = dict(watch) if isinstance(watch, dict) else None
        self.watch_outside = bool(outside)
        # Delivered items a read returned, however long the listed paths run; reads reported at all, and outside.
        self.watched, self.reads, self.outside_reads = set(), 0, 0

    def observe(self, label, fields, private=None):
        """Fold in one tool event: its plain label, and the display fields when the provider gave targets.

        `private` is what only the ledger sees (Enricher.take): a counting key for each path, which tells apart
        files outside the project that share a name, and whether the file a Write names was already there.
        """
        if not fields:
            # Label-only tools (an older CLI, Codex helper lifecycle) still count as steps.
            text = label.get('text') if isinstance(label.get('text'), str) else ''
            if text.endswith(': started'):
                self.steps += 1
                self.helpers += text == 'Agent activity: started'
            if label.get('ok') is False:
                self.failed_steps += 1
            return
        private = private or {}
        state, call, kind = fields.get('state'), fields.get('call'), str(fields.get('tool') or '').lower()
        if state == 'completed' and call in self.calls:
            entry = self.calls.pop(call)
        elif state == 'completed' and not call and self.uncalled.get(kind):
            # Without a call ID a completion closes the oldest open start of the same tool, as the run view pairs it.
            entry = self.uncalled[kind].popleft()
        elif state == 'completed' and not (call and kind):
            # An unpaired completion adds only its outcome: its start, if any, was already counted, and only a
            # completion that names its call and tool can stand alone.
            entry = {'kind': '', 'keys': [], 'changes': []}
        else:
            # A call first seen at its end (a Codex file change, a declined command) is still one step.
            self.steps += 1
            entry = self._start(fields, private)
            if state == 'started':
                if call:
                    self.calls[call] = entry
                elif kind:
                    self.uncalled.setdefault(kind, deque()).append(entry)
                return
        self._finish(entry, fields, private)

    @staticmethod
    def _changes(fields, private):
        """(key, listed path, kind) of each file change; a file outside the project has a private key and no listing."""
        changes = [change for change in fields.get('changes') or [] if isinstance(change, dict) and change.get('path')]
        keys = private.get('changes') or []
        keys = keys if len(keys) == len(changes) else [None] * len(changes)
        return [(key or ('outside:' if change.get('outside') else '') + change['path'],
                 None if change.get('outside') else change['path'], change.get('kind')) for change, key in zip(changes, keys)]

    def _start(self, fields, private):
        kind = str(fields.get('tool') or '').lower()
        path = None if fields.get('outside') else fields.get('path')
        key = private.get('key') or ('outside:' if fields.get('outside') else '') + (fields.get('path') or '')
        entry = {'kind': kind, 'keys': [], 'changes': self._changes(fields, private)}
        if kind in READ and fields.get('path'):
            self.opened.setdefault(key, path)
            self.reads += 1
            self.outside_reads += bool(fields.get('outside'))
            entry['read'] = path
        elif kind == SKILL and fields.get('detail'):
            self.reads += 1
            entry['read'] = 'skill:' + fields['detail']
        elif (kind in EDIT or kind in DELETE) and fields.get('path'):
            existed = private.get('existed')
            created = kind == 'write' and (existed is False if existed is not None
                                           else key not in self.opened and key not in self.edited)
            entry['keys'] = [(key, path, 'delete' if kind in DELETE else 'add' if created else 'update')]
        elif kind in SEARCH:
            self.searched += 1
            if fields.get('detail') and len(self.patterns) < 10000:
                self.patterns.setdefault(fields['detail'], None)
        elif kind in COMMAND:
            self.commands += 1
            entry['command'] = {'command': fields.get('detail') or '', 'ok': None, 'outcome': None, 'exit_code': None}
            self.last_commands.append(entry['command'])
        elif kind in HELPER and (fields.get('detail') == 'spawn_agent' if self.provider == 'codex' else self.provider == 'claude'):
            self.helpers += 1
        return entry

    def _finish(self, entry, fields, private):
        ok, outcome = fields.get('ok') is not False, fields.get('outcome')
        if 'command' in entry:
            entry['command'].update(ok=ok, outcome=outcome, exit_code=fields.get('exit_code'))
            if not ok:
                self.not_run += outcome == 'not_run'
                self.failed += outcome != 'not_run'
        if not ok:
            self.failed_steps += outcome != 'not_run'
            return
        if self.watch and entry.get('read') in self.watch:
            # Only once the tool returned the file: a failed or unanswered read opened nothing.
            self.watched.add(self.watch[entry['read']])
        changes = self._changes(fields, private) if fields.get('changes') else entry['changes']
        for key, path, kind in entry['keys'] + changes:
            self.edited.setdefault(key, path)
            if kind == 'add':
                self.created.add(key)
            elif kind == 'delete':
                self.deleted.add(key)

    def memory(self):
        """What the run's tools show of the delivered memory they read; None when the prompt delivered none.

        `opened` counts delivered items a tool returned - a Read of the file or of any
        tool's copy of a skill, a Skill load by name - counted when the call succeeded,
        however many other files the run read. `unseen` names the ways the run could read
        without saying what: shell commands, Codex helper agents (their own steps are not
        streamed) and, for memory kept outside the project, reads outside it. Without them
        the count is what was observed; beside reads the run did report it is a lower
        bound; with no reported read at all it says nothing, and is unknown. Shell commands
        are never parsed for the files they might read.
        """
        if self.watch is None:
            return None
        unseen = [name for name, count in (('shell', self.commands - self.not_run),
                                           ('helpers', self.helpers if self.provider == 'codex' else 0),
                                           ('outside', self.outside_reads if self.watch_outside else 0)) if count]
        observation = ('lower_bound' if self.reads else 'unknown') if unseen else 'observed'
        return {'delivered': len(set(self.watch.values())), 'opened': len(self.watched),
                'observation': observation, 'unseen': unseen}

    def receipt(self):
        listed = lambda paths: [path for path in paths.values() if path][:LIST_LIMIT]
        receipt = {'steps': self.steps, 'opened': len(self.opened), 'opened_paths': listed(self.opened),
                   'edited': len(self.edited), 'edited_paths': listed(self.edited),
                   'created': len(self.created), 'deleted': len(self.deleted),
                   'searched': self.searched, 'patterns': list(self.patterns)[:RECENT_LIMIT],
                   'commands': self.commands, 'failed': self.failed, 'not_run': self.not_run,
                   'failed_steps': self.failed_steps, 'last_commands': [dict(item) for item in self.last_commands],
                   'plan': self.plan, 'helpers': self.helpers, 'limited': self.limited}
        memory = self.memory()
        if memory is not None:
            # Counts only: a delivered item outside the project (an attached accelerator's) has an absolute path.
            receipt['memory'] = memory
        return receipt


def user_home():
    """The account's home folder, or None when it cannot be told."""
    try:
        return Path.home()
    except (KeyError, OSError, RuntimeError):
        return None


def compaction_text(pre, post):
    """The divider names only the sizes the CLI reported; it may report the size before without the size after."""
    if pre is not None and post is not None:
        size = f' · {pre:,} → {post:,} tokens'
    else:
        size = f' at {pre:,} tokens' if pre is not None else f' to {post:,} tokens' if post is not None else ''
    return f'Context compacted{size}. The CLI replaced earlier conversation with a summary.'


class Enricher:
    """Moves a native launch's tool targets onto its stored events within their own budget; never raises.

    `take` pops the targets from a tool label before the pump counts it and returns the
    fields to store, or None once a cap is reached; `notice` then gives the one limited
    status. `plan` and `compaction` return the rare extra events, which share 121 events
    and 256 KB per launch. The ledger sees every tool event, whatever the caps.

    Paths are relative to `root` (the launch cwd) or `alias`. Details and plan text also lose
    the `strip` roots, such as a worktree session's source checkout, whose files still count as
    outside; with `home`, a path under the home folder that no root covers reads `~/…`.
    `watch` and `outside` describe the memory the prompt delivered (RunLedger).
    """

    def __init__(self, provider, root, alias=None, strip=(), home=None, watch=None, outside=False):
        self.provider = provider
        self.root, self.alias, self.home = root, alias, home
        self.roots = [value for value in (root, alias, *strip) if value]
        self.events = self.bytes = self.extra_events = self.extra_bytes = self.plans = self.compactions = 0
        self.stopped = self.noticed = False
        self.last_plan = None
        # The last plan with its item IDs, which a merge update (Cursor's merge: true) changes by ID.
        self.plan_items, self.plan_total = None, 0
        self.ledger = RunLedger(provider, watch, outside)

    def take(self, clean, count, output_bytes):
        """The display fields for one tool label (popping its targets), or None."""
        targets = clean.pop('targets', None)
        try:
            fields, private = self._read(targets) if isinstance(targets, dict) else (None, None)
            self.ledger.observe(clean, fields, private)
            if not fields or self.stopped:
                return None
            size = len(json.dumps(fields, ensure_ascii=False).encode())
            if (self.events + 1 > ENRICH_EVENTS or self.bytes + size > ENRICH_BYTES
                    or count > STOP_COUNT or output_bytes > STOP_BYTES):
                self.stopped = self.ledger.limited = True
                return None
            self.events += 1
            self.bytes += size
            return fields
        except Exception:  # Display-only: a malformed target must never fail the launch.
            return None

    def notice(self):
        """The limited status, exactly once, after `take` has stopped."""
        if not self.stopped or self.noticed:
            return None
        self.noticed = True
        self.allow_extra(LIMITED)
        return dict(LIMITED)

    def allow_extra(self, event):
        """Whether one more event outside the pump's count fits; room for the limited notice stays reserved."""
        size = _size(event)
        reserve = 0 if self.noticed or event.get('targets') == 'limited' else 1
        if (self.extra_events + 1 > EXTRA_EVENTS - reserve
                or self.extra_bytes + size > EXTRA_BYTES - reserve * _size(LIMITED)):
            return False
        self.extra_events += 1
        self.extra_bytes += size
        return True

    def text(self, value, limit):
        """Agent text as stored: without the roots, redacted, on one line and bounded."""
        return _clean(redact_command(strip_root(value, self.roots, self.home)), limit)

    def fields(self, targets):
        """Display fields from raw targets: relative paths, root-free redacted details, bounded values."""
        return self._read(targets)[0]

    def _key(self, raw, path, outside):
        """How the ledger counts a path: the project path, or for a file outside the project a hash of its full path,
        which is never stored or shown, so two outside files with the same name stay two."""
        if not outside:
            return path
        full = _flavor(self.root).normcase(_full(raw, self.root))
        return 'outside:' + hashlib.sha256(full.encode('utf-8', 'surrogatepass')).hexdigest()[:20]

    def _existed(self, raw):
        """Whether a file is already at the path a Write names, read when its start arrives; None when it cannot be read.

        The CLI may run the tool before Harness reads its start: a file it has just created then counts as
        updated, never an existing file as created.
        """
        if _flavor(self.root) is not os.path:
            return None  # A root of another platform, as in tests, cannot be looked at.
        try:
            return os.path.lexists(_full(raw, self.root))
        except (OSError, ValueError):
            return None

    def _read(self, targets):
        """(display fields, what only the ledger sees) of one tool label's raw targets."""
        fields = {'state': 'completed' if targets.get('state') == 'completed' else 'started',
                  'ok': targets.get('ok') is not False}
        private = {}
        tool = _clean(targets.get('tool'), 120)
        if tool:
            fields['tool'] = tool
        for key in ('call', 'parent'):
            if isinstance(targets.get(key), str) and _CALL.fullmatch(targets[key]):
                fields[key] = targets[key]
        raw = targets.get('path')
        path, outside = relative(raw, self.root, self.alias)
        path = _clean(redact_command(path), 1024)
        if path:
            fields['path'] = path
            if outside:
                fields['outside'] = True
            private['key'] = self._key(raw, path, outside)
            if str(tool).lower() == 'write' and fields['state'] == 'started':
                private['existed'] = self._existed(raw)
        detail = targets.get('detail')
        # A heredoc's body or a script's later lines are file content, whichever provider sent the command.
        detail = self.text(command_head(detail) if str(tool).lower() in COMMAND else detail, DETAIL_LIMIT)
        if detail:
            fields['detail'] = detail
        if targets.get('outcome') == 'not_run':
            fields['outcome'] = 'not_run'
        if type(targets.get('exit_code')) is int:
            fields['exit_code'] = targets['exit_code']
        if isinstance(targets.get('changes'), list):
            changes, keys = [], []
            for change in targets['changes'][:20]:
                raw = change.get('path') if isinstance(change, dict) else None
                path, outside = relative(raw, self.root, self.alias)
                path = _clean(redact_command(path), 1024)
                if path:
                    kind = change.get('kind') if change.get('kind') in ('add', 'update', 'delete') else 'update'
                    changes.append({'path': path, 'kind': kind, **({'outside': True} if outside else {})})
                    keys.append(self._key(raw, path, outside))
            fields['changes'] = changes
            private['changes'] = keys
        return fields, private

    def plan(self, event):
        """A plan snapshot to store, bounded and redacted; None when it repeats the last one or the budget is spent.

        A merge update (`merge: true`) changes the last plan's items by ID and adds new ones; any other
        update replaces the plan.
        """
        try:
            incoming = []
            for item in event.get('items') or []:
                if not isinstance(item, dict):
                    continue
                ident = item.get('id') if isinstance(item.get('id'), str) and _CALL.fullmatch(item['id']) else None
                incoming.append({'id': ident, 'text': self.text(item.get('text'), 200),
                                 'status': item.get('status') if item.get('status') in PLAN_STATUSES else None})
            if event.get('merge') is True and self.plan_items is not None:
                items = [dict(item) for item in self.plan_items]
                known = {item['id']: item for item in items if item['id']}
                added = 0
                for item in incoming:
                    if item['id'] in known:
                        known[item['id']].update({key: item[key] for key in ('text', 'status') if item[key]})
                    elif item['text']:
                        items.append({**item, 'status': item['status'] or 'pending'})
                        known.update({item['id']: items[-1]} if item['id'] else {})
                        added += 1
                total = max(len(items), self.plan_total + added)
            else:
                items = [{**item, 'status': item['status'] or 'pending'} for item in incoming if item['text']]
                total = event.get('total') if type(event.get('total')) is int and event['total'] >= len(items) else len(items)
            if not items:
                return None
            self.plan_items, self.plan_total = items[:PLAN_STATE], total
            shown = [{'text': item['text'], 'status': item['status']} for item in items[:PLAN_ITEMS]]
            plan = {'kind': 'plan', 'items': shown, 'total': total}
            form = self.text(event.get('active_form'), 200)
            if form:
                plan['active_form'] = form
            self.ledger.plan = {'done': sum(item['status'] == 'done' for item in items), 'total': total}
            digest = hashlib.sha256(json.dumps(plan, sort_keys=True).encode()).hexdigest()
            if digest == self.last_plan or self.plans >= PLAN_EVENTS or not self.allow_extra(plan):
                return None
            self.last_plan = digest
            self.plans += 1
            return plan
        except Exception:  # Display-only, as in take().
            return None

    def compaction(self, compactions):
        """One divider per compaction the context tracker found since the last call."""
        result = []
        try:
            for item in compactions[self.compactions:]:
                if self.compactions >= COMPACTION_EVENTS:
                    break
                self.compactions += 1
                pre, post = ((value if type(value) is int else None) for value in (item.get('pre'), item.get('post')))
                event = {'kind': 'status', 'compaction': {'pre': pre, 'post': post}, 'text': compaction_text(pre, post)}
                if self.allow_extra(event):
                    result.append(event)
        except Exception:  # Display-only, as in take().
            pass
        return result
