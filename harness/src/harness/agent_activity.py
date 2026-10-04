"""Bounded, redacted agent activity for the System Orchestration agents panel.

Runners emit two display-only event kinds into their Harness session:
``agent`` (a dispatch started or finished) and ``agent_activity`` (what that
agent says, reasons, reads, runs and edits). Receipts, checkpoints and native
Brain records never depend on them. Raw provider output is parsed line by line
and discarded; file contents, diffs and command output are never forwarded.
"""
from __future__ import annotations

from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import re

from ai_system_lib import SECRET, inside
from .providers import ACTIVITY_THINKING_LIMIT, activity_events, total_tokens

# The session store fails a launch above 5000 events or 4 MiB. Everything this
# stream emits counts against one budget: activity stops at the soft limits, and
# even lifecycle/usage events stop at the hard limits, so a launch never fails.
EVENT_LIMIT = 3000
BYTE_LIMIT = 2_500_000
HARD_EVENT_LIMIT = 4500
HARD_BYTE_LIMIT = 3_600_000
AGENT_EVENT_LIMIT = 400
LIST_LIMIT = 12
CLI = {'claude': ('Claude Code', 'claude auth login'), 'codex': ('Codex', 'codex login'),
       'cursor': ('Cursor Agent', 'cursor-agent login')}
AUTH_ERROR = re.compile(r'authenticat|logged in|log in|login|oauth|api key|unauthori[sz]ed|forbidden|\b40[13]\b', re.I)
LIMIT_ERROR = re.compile(r'rate.?limit|usage limit|quota|\b429\b|overloaded', re.I)


def explain(provider, reason):
    """A provider-reported error with the action that usually resolves it."""
    name, login = CLI.get(provider, (provider, None))
    if login and AUTH_ERROR.search(reason):
        return (f'{name} is not signed in or its login expired ({reason}). '
                f'Sign in again in a terminal with "{login}", then retry.')
    if LIMIT_ERROR.search(reason):
        return f'{name} reported a usage or rate limit ({reason}). Retry later or choose another provider.'
    return f'{name} reported: {reason}'


def now():
    return datetime.now(timezone.utc).isoformat(timespec='milliseconds')


def redact(value):
    return SECRET.sub('[redacted]', value) if isinstance(value, str) else value


def bounded(details):
    """Lifecycle details stay small whatever the number of services or files."""
    result = {}
    for key, value in details.items():
        if isinstance(value, list):
            result[key] = [redact(str(item)[:300]) for item in value[:LIST_LIMIT]]
            if len(value) > LIST_LIMIT:
                result[key + '_count'] = len(value)
        else:
            result[key] = redact(value[:1024]) if isinstance(value, str) else value
    return result


class ActivityStream:
    """Translate native CLI lines into tagged session events within fixed budgets."""

    def __init__(self, emit, provider, roots, aliases=None):
        self.emit = emit
        self.provider = provider
        # Deepest root first, so nested monorepo services win over their parents.
        self.roots = sorted(((Path(path), sid) for sid, path in roots.items()),
                            key=lambda item: len(item[0].parts), reverse=True)
        self.aliases = {Path(path): value for path, value in (aliases or {}).items()}
        self.count = self.bytes = 0
        self.limited = self.silenced = False
        self.agent = None
        self.reasons = {}

    def start(self, agent, attempt, cwd, **details):
        self.agent = {'agent': agent, 'attempt': attempt}
        self.cwd = Path(cwd)
        self.agent_count = 0
        self.agent_limited = False
        self.last_error = None
        self.thinking = ''
        self.usage = {'tokens': 0, 'cost_usd': 0.0, 'reported': False}
        label = details.pop('label', agent)
        self._send({'kind': 'agent', **self.agent, 'status': 'running', **bounded(details),
                    'text': f'{label} started · attempt {attempt}'})

    def finish(self, status, **details):
        if self.agent is None:
            return
        self._flush_thinking()
        summary = details.pop('summary', None)
        event = {'kind': 'agent', **self.agent, 'status': status, **bounded(details),
                 'text': f"{self.agent['agent']} {status}"}
        if isinstance(summary, str) and summary:
            event['summary'] = redact(summary[:300])
        if status != 'completed' and self.last_error:
            # The CLI's own last error explains a failure better than an exit code.
            event['reason'] = self.reasons[self.agent['agent']] = explain(self.provider, self.last_error)
        if self.usage['reported']:
            event['tokens'] = self.usage['tokens']
            event['cost_usd'] = None if self.usage['cost_usd'] is None else round(self.usage['cost_usd'], 6)
        self._send(event)
        self.agent = None

    def line(self, raw):
        if self.agent is None:
            return
        try:
            event = json.loads(raw)
        except (ValueError, UnicodeError, RecursionError):
            return
        try:
            items = activity_events(self.provider, event)
        except (ValueError, TypeError, AttributeError, KeyError, RecursionError):
            return
        for item in items:
            self._activity(item)

    def _flush_thinking(self):
        if self.thinking.strip():
            text, self.thinking = self.thinking.strip(), ''
            self._activity({'type': 'thinking', 'text': text})
        self.thinking = ''

    def _activity(self, item):
        if item['type'] == 'thinking_delta':
            self.thinking = (self.thinking + item['text'])[:ACTIVITY_THINKING_LIMIT]
            return
        if item['type'] == 'thinking_end' or self.thinking:
            self._flush_thinking()
            if item['type'] == 'thinking_end':
                return
        if item['type'] == 'error' and isinstance(item.get('text'), str) and item['text'].strip():
            self.last_error = redact(' '.join(item['text'].split()))[:300]
        if item['type'] == 'usage':
            fields = {key: value for key, value in item.items() if key != 'type'}
            usage, tokens, cost = self.usage, total_tokens(fields), fields.get('cost_usd')
            usage['reported'] = True
            # A report without a number makes the agent's total unknown, not smaller.
            usage['tokens'] = None if tokens is None or usage['tokens'] is None else usage['tokens'] + tokens
            known = type(cost) in (int, float) and math.isfinite(cost) and cost >= 0
            usage['cost_usd'] = None if not known or usage['cost_usd'] is None else usage['cost_usd'] + cost
            # Usage also feeds the session's launch totals.
            self._send({'kind': 'usage', **fields, **self.agent})
            return
        if self.agent_limited or self.limited:
            return
        if self.count + 1 > EVENT_LIMIT or self.bytes > BYTE_LIMIT:
            self.limited = True
            self._send({'kind': 'agent_activity', **self.agent, 'type': 'status',
                        'text': 'Further agent activity in this launch is not shown. Receipts remain authoritative.'})
            return
        if self.agent_count + 1 > AGENT_EVENT_LIMIT:
            self.agent_limited = True
            self._send({'kind': 'agent_activity', **self.agent, 'type': 'status',
                        'text': 'Further activity of this agent is not shown. Its receipt remains authoritative.'})
            return
        event = {'kind': 'agent_activity', **self.agent}
        for key, value in item.items():
            event[key] = redact(value)
        if 'path' in event:
            service, path = self.locate(event.pop('path'))
            event['path'] = path
            if service:
                event['service'] = service
        event['text'] = self.describe(event)
        self.agent_count += 1
        self._send(event)

    def locate(self, value):
        """Map an absolute or cwd-relative path to (service ID, service-relative path)."""
        path = Path(value)
        if not path.is_absolute():
            path = self.cwd / path
        path = Path(os.path.normpath(str(path)))
        if path in self.aliases:
            return self.aliases[path]
        for root, sid in self.roots:
            if inside(path, root):
                relative = path.relative_to(root).as_posix()
                return sid, '' if relative == '.' else relative
        return None, value

    @staticmethod
    def describe(event):
        if event['type'] != 'tool':
            return event.get('text') or ''
        target = ' · '.join(part for part in (event.get('service'), event.get('path')) if part)
        parts = [event.get('tool', 'Tool'), target, event.get('detail', '')]
        if event['state'] == 'completed':
            parts.append('done' if event.get('ok') else 'failed')
        return ' · '.join(part for part in parts if part)

    def _send(self, event):
        if self.silenced:
            return
        event = {**event, 'at': now()}
        size = len(json.dumps(event, ensure_ascii=False))
        if self.count + 1 > HARD_EVENT_LIMIT or self.bytes + size > HARD_BYTE_LIMIT:
            self.silenced = True
            event = {'kind': 'agent_activity', **(self.agent or {}), 'type': 'status', 'at': event['at'],
                     'text': 'Agent display limit reached; follow the remaining dispatches in their receipts.'}
        self.count += 1
        self.bytes += size
        try:
            self.emit(event)
        except (OSError, ValueError):
            pass  # Display-only: a closed pipe must never change a dispatch outcome.
