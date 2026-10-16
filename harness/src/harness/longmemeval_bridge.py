"""Registration bridge for a trusted, already-loaded LongMemEval Memory API.

Selected API contracts only; no official environment import, provider, evaluator
or persistence. The operator supplies a fresh project/native factory outside JSON
configuration and owns root removal and cleanup after failed construction.
"""
from __future__ import annotations

import copy
import hashlib
import hmac
import json
import math
from pathlib import Path
import secrets
import threading

from .longmemeval_observed import (
    ObservedProjectionMemory, _bounded_custody_metadata, _custody_metadata,
)
from .longmemeval_projection import CONTEXT_BYTES, QUERY_CHARS, _text

PROFILE = 'longmemeval/native-fts5-registration-v1'
MEMORY_TYPE = 'native_fts5_states_v1'
MAX_HOOKS = 16
_REGISTRATION_LOCK = threading.RLock()
_CUSTODY_OBSERVER_TYPE = ObservedProjectionMemory
_CUSTODY_OBSERVER_GETTER = ObservedProjectionMemory.custody_failure_observation


class BridgeError(ValueError):
    """Fixed refusal code, never supplied IDs, paths or exception prose."""


class BridgeCancelled(RuntimeError):
    """Pre-work cancellation; not a launched local/provider query."""


def _params(value):
    if (type(value) is not dict or len(value) > 2 or
            any(type(key) is not str or key not in ('horizon', 'local_token_profile')
                for key in value) or 'horizon' not in value):
        raise BridgeError('bridge_parameters')
    horizon = value['horizon']
    profile = value.get('local_token_profile', 'unknown')
    if (type(horizon) is not int or not 1 <= horizon <= 512 or
            type(profile) is not str or len(profile) > 64 or
            profile not in ('unknown', 'pinned_local_no_model')):
        raise BridgeError('bridge_parameters')
    return {'horizon': horizon, 'local_token_profile': profile}


def _wire(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False,
                      separators=(',', ':'), allow_nan=False).encode('utf-8')


def register_bridge(api, project_factory):
    """Register once, refusing collisions; other registry writers are quiescent.

    The caller authenticates the loaded API and factory. This does not bootstrap
    services or load arbitrary modules/roots from configuration.
    """
    if (not isinstance(getattr(api, 'Memory', None), type) or
            type(getattr(api, 'MEMORY_TYPES', None)) is not dict or
            not callable(getattr(api, 'register_memory', None)) or
            not callable(project_factory)):
        raise BridgeError('bridge_api_profile')
    with _REGISTRATION_LOCK:
        if MEMORY_TYPE in api.MEMORY_TYPES:
            raise BridgeError('bridge_registry_collision')

        class NativeStatesMemory(api.Memory):
            memory_type = MEMORY_TYPE

            def __init__(self, memory_params):
                self._params = _params(memory_params)
                super().__init__(dict(self._params))
                self._lock = threading.RLock()
                self._secret = secrets.token_bytes(32)
                self._contexts = {}
                self._busy = self._closed = self._started = False
                self._final = None
                self._cancel = None
                self._runtime_ignored = False
                self._queries = self._hooks = self._replays = self._cleared = 0
                self._cancelled = 0
                self._observer = ObservedProjectionMemory(**self._params)
                try:
                    owned = project_factory()
                    if type(owned) is not tuple or len(owned) != 2:
                        raise BridgeError('bridge_factory_profile')
                    self._observer.open(*owned)
                except BaseException:
                    # Preserve the primary construction error. The operator
                    # still owns root cleanup and any incomplete close.
                    try:
                        self._observer.close()
                    except BaseException:
                        pass
                    raise

            @property
            def memory_config(self):
                return {'memory_type': MEMORY_TYPE, 'memory_params': dict(self._params)}

            def _ready(self):
                if self._closed or self._busy:
                    raise BridgeError('bridge_lifecycle')

            def _digest(self, raw):
                return hmac.new(self._secret, raw, hashlib.sha256).digest()

            def _request(self, query, query_image):
                _text(query, QUERY_CHARS, nonempty=True, multiline=True)
                if query_image is not None:
                    _text(query_image, 4096, nonempty=True)
                return self._digest(_wire({'query': query, 'query_image': query_image}))

            def _context_digest(self, context):
                if type(context) is not list or len(context) > 1:
                    raise BridgeError('bridge_hook_context')
                for item in context:
                    if (type(item) is not dict or len(item) != 2 or
                            any(type(key) is not str for key in item) or
                            'type' not in item or 'value' not in item or
                            type(item['type']) is not str or item['type'] != 'text'):
                        raise BridgeError('bridge_hook_context')
                    _text(item['value'], CONTEXT_BYTES, multiline=True)
                raw = _wire(context)
                if len(raw) > CONTEXT_BYTES:
                    raise BridgeError('bridge_hook_context')
                return self._digest(raw)

            def _current(self):
                context = getattr(self._query_context_local, 'bridge', None)
                if context is None or context['cleared']:
                    raise BridgeError('bridge_invocation_missing')
                return context

            def _check_cancel(self):
                if self._cancel is not None and self._cancel.is_set():
                    self._cancelled += 1
                    raise BridgeCancelled('bridge_cancelled_before_work')

            def configure_runtime(self, **kwargs):
                with self._lock:
                    self._ready()
                    if self._started or len(kwargs) > 4 or any(type(key) is not str or key not in
                            ('query_trace_dir', 'generation_temperature',
                             'generation_top_p', 'cancel_event') for key in kwargs):
                        raise BridgeError('bridge_runtime_parameters')
                    trace = kwargs.get('query_trace_dir')
                    if trace is not None and (type(trace) not in (str, type(Path())) or
                                              len(str(trace)) > 4096):
                        raise BridgeError('bridge_runtime_parameters')
                    for key in ('generation_temperature', 'generation_top_p'):
                        value = kwargs.get(key)
                        if value is not None and (type(value) not in (int, float) or
                                not 0 <= value <= 1000 or not math.isfinite(value)):
                            raise BridgeError('bridge_runtime_parameters')
                    event = kwargs.get('cancel_event')
                    if event is not None and type(event) is not threading.Event:
                        raise BridgeError('bridge_runtime_parameters')
                    self._cancel = event
                    self._runtime_ignored = any(kwargs.get(key) is not None for key in
                            ('query_trace_dir', 'generation_temperature', 'generation_top_p'))

            def set_query_context(self, *, query_invocation_id):
                with self._lock:
                    self._ready()
                    identity = _text(query_invocation_id, 256, nonempty=True)
                    if identity != identity.strip():
                        raise BridgeError('bridge_invocation_alias')
                    if getattr(self._query_context_local, 'bridge', None) is not None:
                        raise BridgeError('bridge_invocation_uncleared')
                    key = self._digest(identity.encode('utf-8'))
                    if key in self._contexts:
                        raise BridgeError('bridge_invocation_reused')
                    if len(self._contexts) >= self._params['horizon']:
                        raise BridgeError('bridge_invocation_capacity')
                    context = {'started': False, 'cleared': False, 'hook_count': 0,
                               'request': None, 'result': None, 'payload': None}
                    self._contexts[key] = context
                    self._query_context_local.bridge = context
                    super().set_query_context(query_invocation_id=identity)

            def get_query_context(self):
                with self._lock:
                    return {} if self._closed else super().get_query_context()

            def clear_query_context(self):
                with self._lock:
                    if self._busy:
                        raise BridgeError('bridge_lifecycle')
                    context = getattr(self._query_context_local, 'bridge', None)
                    if context is not None:
                        context['cleared'] = True
                        del self._query_context_local.bridge
                        if not self._closed:
                            self._cleared += 1
                    super().clear_query_context()

            def insert(self, trajectory):
                with self._lock:
                    self._ready()
                    self._started = True
                    self._check_cancel()
                    self._busy = True
                    try:
                        return self._observer.insert(trajectory)
                    finally:
                        self._busy = False

            def query(self, query, query_image=None):
                with self._lock:
                    self._ready()
                    context = self._current()
                    if context['started']:
                        raise BridgeError('bridge_invocation_reused')
                    context['started'] = True
                    self._started = True
                    self._queries += 1
                    self._check_cancel()
                    self._busy = True
                    try:
                        request = self._request(query, query_image)
                        ordinal = self._observer.snapshot()['observed']['query_calls']
                        result = self._observer.query(query, query_image)
                        receipt = self._observer.query_receipt(ordinal)
                        context['request'] = request
                        context['result'] = self._context_digest(result)
                        context['payload'] = {'profile': PROFILE, 'ordinal': ordinal,
                            'stage': 'constructed_before_reader_truncation',
                            'query_receipt': receipt, 'reader_usage': None,
                            'reader_usage_complete': False}
                        return result
                    finally:
                        self._busy = False

            def post_query_hook(self, *, query, query_image, memory_context):
                with self._lock:
                    self._ready()
                    context = self._current()
                    if context['payload'] is None or context['hook_count'] >= MAX_HOOKS:
                        raise BridgeError('bridge_hook_state')
                    if (not hmac.compare_digest(self._request(query, query_image), context['request']) or
                            not hmac.compare_digest(self._context_digest(memory_context), context['result'])):
                        raise BridgeError('bridge_hook_binding')
                    self._replays += context['hook_count'] > 0
                    context['hook_count'] += 1
                    self._hooks += 1
                    return copy.deepcopy(context['payload'])

            def snapshot(self):
                with self._lock:
                    if self._final is not None:
                        return copy.deepcopy(self._final)
                    return {'profile': PROFILE, 'closed': self._closed,
                            'contexts_admitted': len(self._contexts), 'queries_attempted': self._queries,
                            'hooks_verified': self._hooks, 'hook_replays': self._replays,
                            'contexts_cleared': self._cleared, 'prework_cancelled': self._cancelled,
                            'pending_contexts': sum(not row['cleared'] for row in self._contexts.values()),
                            'runtime_overrides_ignored': self._runtime_ignored,
                            'local_observation': self._observer.snapshot()}

            def failure_observation(self, *, phase):
                """Optional local diagnostic; source/receipt/ledger views unchanged."""
                with self._lock:
                    return self._observer.failure_observation(phase=phase)

            def custody_failure_observation(self):
                """Bounded optional diagnostic; retained ownership survives close.

                No task work, lifecycle guard or frozen-view/accounting mutation.
                Exact owner/binding checks provide consistency, not authenticity.
                """
                with self._lock:
                    observer = self._observer
                    if type(observer) is not _CUSTODY_OBSERVER_TYPE:
                        return _custody_metadata()
                    try:
                        return _bounded_custody_metadata(_CUSTODY_OBSERVER_GETTER(observer))
                    except BaseException:
                        return _custody_metadata()

            def close(self):
                with self._lock:
                    if self._busy:
                        raise BridgeError('bridge_lifecycle')
                    if self._closed:
                        return self.snapshot()
                    self._busy = True
                    try:
                        self._observer.close()
                    finally:
                        self._busy = False
                        self._closed = True
                        self._final = self.snapshot()
                    return self.snapshot()

            def save_memory(self, output_dir):
                raise BridgeError('bridge_persistence_unsupported')

            def _save_backend(self, output_dir):
                raise BridgeError('bridge_persistence_unsupported')

            def _load_backend(self, input_dir):
                raise BridgeError('bridge_persistence_unsupported')

            @classmethod
            def reconcile_loaded_memory_config(cls, saved_config, requested_config):
                raise BridgeError('bridge_persistence_unsupported')

        api.register_memory(NativeStatesMemory)
        return NativeStatesMemory
