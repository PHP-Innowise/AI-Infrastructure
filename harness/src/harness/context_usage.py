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
import time

# Live fill reaches the database at most this often, unless it moved by the share below.
WRITE_SECONDS = 2.0
WRITE_SHARE = .005
CLI_FILES = {'claude': ('CLAUDE.md', '.claude/CLAUDE.md'), 'codex': ('AGENTS.md',), 'cursor': ()}
HOOK_FILES = {'claude': '.claude/settings.json', 'codex': '.codex/hooks.json', 'cursor': '.cursor/hooks.json'}
CURSOR_RULE = '.cursor/rules/working-memory.mdc'


def _count(value):
    return value if type(value) is int and value >= 0 else None


def hook_parts(text):
    """Characters of a rendered capsule by memory kind, from its line shapes.

    Item lines are '  path — title'; the bank's chunk paths are Memory bank, Brain
    paths and local episodes are Project Brain, anything else is rules and docs.
    The envelope (working:, Last turn:, layer headings, notes) is Project Brain.
    """
    parts = {'brain': 0, 'bank': 0, 'rules': 0}
    for line in text.splitlines(keepends=True):
        kind = 'brain'
        if line.startswith('  ') and ' — ' in line:
            path = line.strip().split(' — ', 1)[0]
            kind = ('bank' if path.startswith('memory-bank/chunks/') else
                    'brain' if path.startswith(('project-brain/', 'episode ')) else 'rules')
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
                    found = hook_parts(event['stdout'])
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

    def merge(self, fill):
        """Take fill read after the launch (Codex rollouts) where the stream reported none."""
        if not fill:
            return
        for key in ('start', 'end', 'peak', 'window'):
            if getattr(self, key) is None and _count(fill.get(key)):
                setattr(self, key, fill[key])
        if not self.calls and _count(fill.get('calls')):
            self.calls = fill['calls']
        if not self.compactions and isinstance(fill.get('compactions'), list):
            self.compactions = fill['compactions']

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


def codex_fill(tail, since, until):
    """Context fill of one Codex launch from its rollout: token_count events and compactions inside the launch."""
    fills, window, compactions = [], None, []
    for line in tail.splitlines() if tail else []:
        try:
            record = json.loads(line)
            stamp = datetime.fromisoformat(str(record.get('timestamp', '')).replace('Z', '+00:00'))
        except (ValueError, TypeError, AttributeError, RecursionError):
            continue
        if not isinstance(record, dict) or stamp.tzinfo is None or not since <= stamp.timestamp() <= until:
            continue
        payload = record.get('payload') if isinstance(record.get('payload'), dict) else {}
        if record.get('type') == 'event_msg' and payload.get('type') == 'token_count' and isinstance(payload.get('info'), dict):
            info = payload['info']
            last = info.get('last_token_usage') if isinstance(info.get('last_token_usage'), dict) else {}
            if _count(last.get('input_tokens')):
                fills.append(last['input_tokens'])
                for compaction in compactions:
                    if compaction['post'] is None and compaction['call'] == len(fills) - 1:
                        compaction['post'] = last['input_tokens']
            if _count(info.get('model_context_window')):
                window = info['model_context_window']
        elif record.get('type') == 'compacted':
            compactions.append({'pre': fills[-1] if fills else None, 'post': None, 'call': len(fills)})
    if not fills:
        return None
    return {'start': fills[0], 'end': fills[-1], 'peak': max([*fills, *(item['pre'] for item in compactions if item['pre'])]),
            'calls': len(fills), 'window': window, 'compactions': compactions[:20]}


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
