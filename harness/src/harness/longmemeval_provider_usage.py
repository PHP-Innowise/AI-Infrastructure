"""Supplied ChatCompletion usage, captured before answer decoding (stdlib).

This is a five-counter response projection, not a transport or SDK observer.
It verifies no provider, final request, physical attempt or invoice. Unknown
coverage is never upgraded by a complete supplied usage tuple.
"""
from __future__ import annotations

import copy
import inspect
import json
import math
import threading

from .cost_observation import COUNTER_LIMIT, counter

PROFILE = 'supplied-chat-completion-usage-v1'
RAW_BYTES = 524288
DEPTH = 16
NODES = 4096
STRING_CHARS = 131072
OBJECT_MEMBERS = 64
ARRAY_ITEMS = 1024
NUMBER_CHARS = 64
KEYS = ('input_tokens', 'output_tokens', 'total_tokens',
        'cached_input_tokens', 'reasoning_output_tokens')
RAW_KEYS = ('prompt_tokens', 'completion_tokens', 'total_tokens',
            'prompt_tokens_details', 'completion_tokens_details')
PROMPT_DETAILS = ('cached_tokens', 'cache_write_tokens', 'audio_tokens',
                  'image_tokens', 'text_tokens')
COMPLETION_DETAILS = ('reasoning_tokens', 'audio_tokens', 'text_tokens',
                      'accepted_prediction_tokens', 'rejected_prediction_tokens')


class UsageError(ValueError):
    """Fixed observation/replay code; no response or exception prose."""


def _empty(size=None, state='unobserved'):
    return {'profile': PROFILE, 'response_bytes': size, 'usage_state': state,
            'usage': dict.fromkeys(KEYS), 'usage_scope': 'supplied_response_only',
            'provider_coverage': 'UNKNOWN', 'physical_attempts': None,
            'usd_micros': None, 'billing': 'unknown', 'usage_verified': False}


def _depth(text):
    """Bound JSON container nesting before invoking the stdlib decoder."""
    level, quoted, escaped = 0, False, False
    for char in text:
        if quoted:
            if escaped:
                escaped = False
            elif char == '\\':
                escaped = True
            elif char == '"':
                quoted = False
        elif char == '"':
            quoted = True
        elif char in '[{':
            level += 1
            if level > DEPTH:
                raise UsageError('admission')
        elif char in ']}':
            level -= 1


def _pairs(items):
    out = {}
    for key, value in items:
        if key in out:
            raise UsageError('json')
        out[key] = value
    return out


def _integer(text):
    if len(text) > NUMBER_CHARS:
        raise UsageError('admission')
    return int(text)


def _floating(text):
    if len(text) > NUMBER_CHARS:
        raise UsageError('admission')
    value = float(text)
    if not math.isfinite(value):
        raise UsageError('json')
    return value


def _constant(_):
    raise UsageError('json')


def _tree(value):
    count, pending = 0, [value]
    while pending:
        node = pending.pop()
        count += 1
        if count > NODES:
            raise UsageError('admission')
        if type(node) is dict:
            if len(node) > OBJECT_MEMBERS:
                raise UsageError('admission')
            pending.extend(node.keys())
            pending.extend(node.values())
        elif type(node) is list:
            if len(node) > ARRAY_ITEMS:
                raise UsageError('admission')
            pending.extend(node)
        elif type(node) is str:
            if len(node) > STRING_CHARS:
                raise UsageError('admission')
            node.encode('utf-8')  # rejects lone JSON-escaped surrogates


def _details(value, allowed, selected):
    if value is None:
        return None
    if type(value) is not dict:
        raise UsageError('usage')
    if any(key not in allowed for key in value):
        raise UsageError('unsupported')
    if any(item is not None and counter(item) is None for item in value.values()):
        raise UsageError('usage')
    return value.get(selected)


def completion_usage(raw):
    """Project bounded supplied nonstreaming ChatCompletion JSON, without IO.

    'complete' means three explicit consistent main counters only. Optional
    breakdowns stay null. Payload admission is not an OS/RSS or hard-time bound.
    Rejected observations never imply zero tokens or stop the trusted decoder.
    """
    if type(raw) is not bytes:
        return _empty(state='unmeasured')
    size = len(raw)
    if size > RAW_BYTES:
        return _empty(size, 'unmeasured')
    try:
        text = raw.decode('utf-8')
        _depth(text)
        body = json.loads(text, object_pairs_hook=_pairs, parse_int=_integer,
                          parse_float=_floating, parse_constant=_constant)
        _tree(body)
        if (type(body) is not dict or body.get('object') != 'chat.completion'
                or 'error' in body):
            return _empty(size, 'unsupported')
        usage = body.get('usage')
        if usage is None:
            return _empty(size, 'missing')
        if type(usage) is not dict:
            raise UsageError('usage')
        if any(key not in RAW_KEYS for key in usage):
            raise UsageError('unsupported')
        inp, out, total = (usage.get(key) for key in RAW_KEYS[:3])
        if any(item is not None and counter(item) is None for item in (inp, out, total)):
            raise UsageError('usage')
        cached = _details(usage.get('prompt_tokens_details'), PROMPT_DETAILS, 'cached_tokens')
        reasoning = _details(usage.get('completion_tokens_details'), COMPLETION_DETAILS, 'reasoning_tokens')
        if total is not None and any(item is not None and item > total for item in (inp, out)):
            raise UsageError('usage')
        if inp is not None and out is not None:
            if inp + out > COUNTER_LIMIT or (total is not None and total != inp + out):
                raise UsageError('usage')
        if cached is not None and (inp is None or cached > inp):
            raise UsageError('usage')
        if reasoning is not None and (out is None or reasoning > out):
            raise UsageError('usage')
        fields = (inp, out, total, cached, reasoning)
        state = ('complete' if all(item is not None for item in fields[:3])
                 else 'partial' if any(item is not None for item in fields) else 'missing')
        result = _empty(size, state)
        result['usage'] = dict(zip(KEYS, fields))
        return result
    except UsageError as exc:
        code = exc.args[0]
        return _empty(size, 'unmeasured' if code == 'admission'
                      else 'unsupported' if code == 'unsupported' else 'invalid')
    except (UnicodeError, ValueError, RecursionError):
        return _empty(size, 'invalid')


class CompletionCapture:
    """One owned response capture before a trusted answer decoder.

    No raw response, decoder result, caller IDs or exception text is retained.
    This is not a controller/ordinal binding or execution-authorization token.
    Python/operator objects and private-attribute access are trusted boundaries.
    """
    __slots__ = ('_lock', '_consumed', '_observation', '_decoder_state')

    def __init__(self):
        self._lock = threading.RLock()
        self._consumed = False
        self._observation = _empty()
        self._decoder_state = 'not_entered'

    def deliver(self, raw, decoder):
        if not callable(decoder):
            raise UsageError('decoder')
        with self._lock:
            if self._consumed:
                raise UsageError('capture_consumed')
            self._consumed = True
            try:
                self._observation = completion_usage(raw)
            except Exception:
                # Metadata failure must not prevent the decoder/task.
                self._observation = _empty(state='unmeasured')
            except BaseException:
                self._observation = _empty(state='unmeasured')
                raise
            self._decoder_state = 'running'
        try:
            value = decoder(raw)
        except BaseException as exc:
            with self._lock:
                self._decoder_state = ('failed' if issubclass(type(exc), Exception)
                                       else 'cancelled')
            raise
        try:
            awaitable = inspect.isawaitable(value)
        except Exception:
            with self._lock:
                self._decoder_state = 'returned_unknown'
            return value
        except BaseException:
            with self._lock:
                self._decoder_state = 'returned_unknown'
            raise
        with self._lock:
            self._decoder_state = 'returned_awaitable' if awaitable else 'succeeded'
        return value

    def snapshot(self):
        with self._lock:
            return {'consumed': self._consumed, 'decoder_state': self._decoder_state,
                    'observation': copy.deepcopy(self._observation)}
