"""Usage › Context: how full a launch's context window is, and how much of it is memory.

A launch records integers only. The prompt ledger counts what the Harness put in the
prompt; the tracker reads the provider's own per-call usage; hook memory is measured
from the hook's output and discarded. No prompt, file or hook text reaches the
database, only character and token counts and the fixed context-file names.
"""
from __future__ import annotations

from datetime import datetime
import json
import math
import os
import re
import stat
import time

from .filesystem import fs

# Live fill reaches the database at most this often, unless it moved by the share below.
WRITE_SECONDS = 2.0
WRITE_SHARE = .005
# Instruction files each CLI loads by itself (Cursor's CLI reads the root AGENTS.md
# and CLAUDE.md as rules), so the Harness never sends them again.
CLI_FILES = {'claude': ('CLAUDE.md', '.claude/CLAUDE.md'), 'codex': ('AGENTS.md',), 'cursor': ('AGENTS.md', 'CLAUDE.md')}
HOOK_FILES = {'claude': '.claude/settings.json', 'codex': '.codex/hooks.json', 'cursor': '.cursor/hooks.json'}
CURSOR_RULE = '.cursor/rules/working-memory.mdc'
# An item of the rendered capsule (render_capsule_lines in memory-bank/scripts/context.py):
# `- memory|history|skill|policy <path or episode id>[ § section] — title`.
_ITEM = re.compile(r'- (?:memory|history|skill|policy) (.+?) —(?: |$)')


def _count(value):
    return value if type(value) is int and value >= 0 else None


def _item_kind(label):
    path = label.split(' § ', 1)[0].strip()
    return ('bank' if path.startswith('memory-bank/chunks/') else
            'brain' if path.startswith(('project-brain/', 'episode ')) else 'rules')


def capsule_text_parts(text):
    """Characters of a rendered capsule by memory kind: what a hook printed or what the Harness inserted.

    An item line is `- memory|history|skill|policy <path> — title`, and the indented
    excerpt under it belongs to the same item; a runtime from before excerpts printed
    items as `  <path> — title`. The bank's chunks are Memory bank, Brain records,
    handoffs and local episodes are Project Brain, anything else (skills, policy,
    specs, docs) is rules and docs. Everything around the items - the working state,
    the memory heading, warnings, a header - is Project Brain, so the parts add up
    to the text's length.
    """
    parts = {'brain': 0, 'bank': 0, 'rules': 0}
    item = None
    for line in text.splitlines(keepends=True):
        match = _ITEM.match(line)
        if match:
            item = kind = _item_kind(match.group(1))
        elif item is not None and line.startswith('  '):
            kind = item
        elif line.startswith('  ') and ' — ' in line:
            item, kind = None, _item_kind(line.strip().split(' — ', 1)[0])
        else:
            item, kind = None, 'brain'
        parts[kind] += len(line)
    return parts


class ContextTracker:
    """A launch's context-window fill from the provider's per-call usage; what was not reported stays None.

    Claude: each main-thread assistant message is one model call, counted once by
    message ID; its fill is input plus cache read plus cache creation. Calls from
    subagents (a parent tool use) are not the main context. The window comes from
    the result's modelUsage, compaction from compact_boundary, hook memory from
    hook_response events (requested with --include-hook-events).
    """

    def __init__(self, provider):
        self.provider = provider
        self.start = self.end = self.peak = self.window = self.model = self.cache_share = None
        self.calls = 0
        self.messages = set()
        self.compactions = []
        self.hooks = None
        self.written_at, self.written_end = 0.0, None

    def _call(self, fill):
        self.calls += 1
        self.start = fill if self.start is None else self.start
        self.end = fill
        self.peak = fill if self.peak is None else max(self.peak, fill)

    def observe(self, event):
        """Fold one raw provider event in; True when the recorded numbers changed."""
        if not isinstance(event, dict):
            return False
        kind = event.get('type')
        if self.provider == 'claude':
            if kind == 'assistant' and event.get('parent_tool_use_id') is None and not event.get('is_meta') and not event.get('historical'):
                message = event.get('message') if isinstance(event.get('message'), dict) else {}
                usage, identity = message.get('usage'), message.get('id')
                if not isinstance(identity, str) or identity in self.messages or not isinstance(usage, dict):
                    return False
                parts = [usage.get(key) for key in ('input_tokens', 'cache_read_input_tokens', 'cache_creation_input_tokens')]
                if _count(parts[0]) is None or any(part is not None and _count(part) is None for part in parts):
                    return False
                fill = sum(part or 0 for part in parts)
                if not fill:
                    return False
                self.messages.add(identity)
                if isinstance(message.get('model'), str):
                    self.model = message['model']
                self._call(fill)
                return True
            if kind == 'system' and event.get('subtype') == 'compact_boundary':
                metadata = event.get('compact_metadata')
                if isinstance(metadata, dict) and _count(metadata.get('pre_tokens')) is not None:
                    self.compactions.append({'pre': metadata['pre_tokens'], 'post': _count(metadata.get('post_tokens')), 'call': self.calls})
                    self.peak = metadata['pre_tokens'] if self.peak is None else max(self.peak, metadata['pre_tokens'])
                    return True
                return False
            if kind == 'system' and event.get('subtype') == 'hook_response' and event.get('hook_event') in ('UserPromptSubmit', 'SessionStart'):
                if isinstance(event.get('stdout'), str) and event['stdout']:
                    found = capsule_text_parts(event['stdout'])
                    self.hooks = {name: (self.hooks or {}).get(name, 0) + value for name, value in found.items()}
                    return True
                return False
            if kind == 'result':
                windows, entry = event.get('modelUsage'), None
                if isinstance(windows, dict):
                    entry = windows.get(self.model) if self.model else None
                    known = [item.get('contextWindow') for item in windows.values()
                             if isinstance(item, dict) and _count(item.get('contextWindow'))]
                    window = entry.get('contextWindow') if isinstance(entry, dict) and _count(entry.get('contextWindow')) else max(known, default=None)
                    self.window = window or self.window
                usage = event.get('usage') if isinstance(event.get('usage'), dict) else {}
                parts = [_count(usage.get(key)) for key in ('input_tokens', 'cache_read_input_tokens', 'cache_creation_input_tokens')]
                if None not in parts and sum(parts):
                    self.cache_share = parts[1] / sum(parts)
                return True
            return False
        if self.provider == 'codex' and kind == 'turn.completed':
            usage = event.get('usage') if isinstance(event.get('usage'), dict) else {}
            cached, total = _count(usage.get('cached_input_tokens')), _count(usage.get('input_tokens'))
            if cached is not None and total:
                # Codex counts cached input inside input_tokens.
                self.cache_share = min(1.0, cached / total)
                return True
        return False

    def update(self, fill):
        """Take a Codex rollout's fill as the launch's own: Codex streams none, so the rollout is the source."""
        if not fill:
            return
        self.start, self.end, self.peak, self.calls = fill['start'], fill['end'], fill['peak'], fill['calls']
        self.window = fill.get('window') or self.window
        self.compactions = fill.get('compactions') or []

    def snapshot(self):
        return {'start': self.start, 'end': self.end, 'peak': self.peak, 'calls': self.calls or None, 'window': self.window,
                'compactions': self.compactions[:20],
                'cache_share': round(self.cache_share, 4) if self.cache_share is not None and math.isfinite(self.cache_share) else None,
                'hooks': self.hooks}

    def due(self):
        """Whether a live write is worth it: two seconds passed, or the fill moved by half a percent of the window."""
        now = time.monotonic()
        moved = (self.window and self.end is not None and self.written_end is not None
                 and abs(self.end - self.written_end) >= WRITE_SHARE * self.window)
        if now - self.written_at >= WRITE_SECONDS or moved:
            self.written_at, self.written_end = now, self.end
            return True
        return False


class CodexFill:
    """A Codex launch's fill from rollout records: token_count events and compactions inside its time window."""

    def __init__(self, since, until=None):
        self.since, self.until = since, until
        self.fills, self.window, self.compactions = [], None, []

    def feed(self, line):
        """Fold one rollout line in; True when it changed the fill."""
        try:
            record = json.loads(line)
            stamp = datetime.fromisoformat(str(record.get('timestamp', '')).replace('Z', '+00:00'))
        except (ValueError, TypeError, AttributeError, RecursionError):
            return False
        if (not isinstance(record, dict) or stamp.tzinfo is None or stamp.timestamp() < self.since
                or self.until is not None and stamp.timestamp() > self.until):
            return False
        payload = record.get('payload') if isinstance(record.get('payload'), dict) else {}
        if record.get('type') == 'event_msg' and payload.get('type') == 'token_count' and isinstance(payload.get('info'), dict):
            info = payload['info']
            last = info.get('last_token_usage') if isinstance(info.get('last_token_usage'), dict) else {}
            changed = False
            if _count(info.get('model_context_window')):
                changed, self.window = info['model_context_window'] != self.window, info['model_context_window']
            if _count(last.get('input_tokens')):
                self.fills.append(last['input_tokens'])
                for compaction in self.compactions:
                    if compaction['post'] is None and compaction['call'] == len(self.fills) - 1:
                        compaction['post'] = last['input_tokens']
                return True
            return changed
        if record.get('type') == 'compacted':
            self.compactions.append({'pre': self.fills[-1] if self.fills else None, 'post': None, 'call': len(self.fills)})
            return True
        return False

    def snapshot(self):
        if not self.fills:
            return None
        return {'start': self.fills[0], 'end': self.fills[-1],
                'peak': max([*self.fills, *(item['pre'] for item in self.compactions if item['pre'])]),
                'calls': len(self.fills), 'window': self.window, 'compactions': self.compactions[:20]}


class CodexLive:
    """A running Codex launch's rollout, read as it grows: only new complete lines on each poll.

    The rollout path is looked up through Codex's own thread index once the thread
    is known; until then, and while the index has no row, a poll costs nothing.
    """
    LIMIT = 4 * 1024 * 1024
    # Reads of at most LIMIT bytes once the launch has ended; the rollout's last 4 MiB extends past them.
    SETTLE_READS = 8

    def __init__(self, project, since):
        self.project, self.fill = project, CodexFill(since)
        self.path, self.offset, self.partial, self.retry_at = None, 0, b'', 0.0

    def poll(self, native_id):
        """Read what the rollout gained since the last poll; True when the fill changed."""
        from . import providers
        if not native_id:
            return False
        if self.path is None:
            if time.monotonic() < self.retry_at:
                return False
            opened = providers.codex_rollout(native_id, self.project)
            if not opened:
                self.retry_at = time.monotonic() + WRITE_SECONDS
                return False
            stream, self.path = opened
            with stream:
                self.offset = stream.tell()
        try:
            stream = os.fdopen(fs.open(self.path, os.O_RDONLY | fs.O_NOFOLLOW | fs.O_NONBLOCK), 'rb')
        except OSError:
            return False
        with stream:
            info = fs.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_size < self.offset:
                return False
            stream.seek(self.offset)
            data = stream.read(self.LIMIT)
        self.offset += len(data)
        lines = (self.partial + data).split(b'\n')
        self.partial = lines.pop()
        if len(self.partial) > self.LIMIT:
            self.partial = b''
        changed = False
        for line in lines:
            changed = self.fill.feed(line) or changed
        return changed

    def settle(self, native_id, until):
        """Once the launch has ended: read what the rollout gained since the last poll, then extend the fill with
        the rollout's last 4 MiB past where those reads stopped. A line is known by where it starts in the file, so
        it counts once, and the fill is never replaced: the tail can begin inside a long launch, after calls and
        compactions only the live reads saw. True when the fill changed."""
        from . import providers
        self.fill.until, self.retry_at = until, 0.0
        changed = False
        for _ in range(self.SETTLE_READS):
            offset = self.offset
            changed = self.poll(native_id) or changed
            if self.offset == offset:
                break
        end = providers.codex_rollout_end(native_id, self.project) if native_id else None
        if end and (self.path is None or end[0] == self.path):
            _, position, data = end
            # The live reads fed every complete line before this place in the file.
            read = self.offset - len(self.partial) if self.path is not None else 0
            for line in data.split(b'\n'):
                if position >= read:
                    changed = self.fill.feed(line) or changed
                position += len(line) + 1
        return changed


def launch_files(project, provider):
    """What the CLI loads by itself and whether memory hooks are installed, as sizes and flags only."""
    from .sessions import read_context
    files = []
    for name in CLI_FILES.get(provider, ()):
        content = read_context(project, name)
        if content is not None:
            files.append({'name': name, 'bytes': content[0]})
    if provider == 'cursor':
        rule = read_context(project, CURSOR_RULE)
        # Cursor's memory hook keeps a rule file; its size is the memory each prompt carries.
        hooks = {'installed': rule is not None, 'measured': rule is not None, 'bytes': rule[0] if rule else None}
        return {'cli_files': files, 'hooks': hooks}
    settings = read_context(project, HOOK_FILES[provider], 256 * 1024)
    installed = settings is not None and 'UserPromptSubmit' in settings[1]
    # Claude streams its hook output when asked to; Codex exec does not, so its hook memory is not measured.
    return {'cli_files': files, 'hooks': {'installed': installed, 'measured': installed and provider == 'claude', 'bytes': None}}
