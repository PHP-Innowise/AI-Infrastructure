"""Local operation observations for the standalone text-projection pilot.

No upstream registration, evaluator, provider conversion or service dispatch.
The explicit no-model profile is a trusted source-path attestation, not inferred
from missing usage. Dedicated roots, native modules and clocks are trusted inputs.
"""
from __future__ import annotations

import copy
from functools import wraps
import hashlib
import json
import threading
import time

from . import longmemeval_projection as projection
from .longmemeval_failure_observation import (
    PHASES as FAILURE_PHASES, empty_failure, projection_failure_metadata,
    unmeasured_failure, validate_phase,
)
from .cost_observation import COUNTER_LIMIT, counter
from .memory_benchmark_cost import Ledger

PROFILE = 'native-fts5/observed-local-operations-v1'
MAX_INSERT_CALLS = 4096
STATES = ('delivered', 'no_evidence', 'sanitized_empty', 'refresh_failed',
          'cleanup_failed', 'failed')

# Trusted import-time bindings; no reflective dispatch on supplied backends.
# These prove local format/owner consistency, not runtime authenticity/freshness.
_CUSTODY_NATIVE_TYPE = projection.NativeProjectionMemory
_CUSTODY_NATIVE_GETTER = getattr(_CUSTODY_NATIVE_TYPE, 'custody_failure_observation', None)
_CUSTODY_PROFILE = projection.CUSTODY_FAILURE_PROFILE
_CUSTODY_SCOPE = 'last_caught_custody_body_failure'
_CUSTODY_STAGES = frozenset(projection.CUSTODY_FAILURE_STAGES)
_CUSTODY_FIELDS = frozenset(('profile', 'scope', 'observation_state', 'stage',
                            'root_cause', 'provider_tokens', 'usd_micros'))


def _custody_metadata(state='unmeasured', stage=None):
    return {'profile': _CUSTODY_PROFILE, 'scope': _CUSTODY_SCOPE,
            'observation_state': state, 'stage': stage, 'root_cause': None,
            'provider_tokens': None, 'usd_micros': None}


def _bounded_custody_metadata(item):
    # Inspect exact key types before hashing, equality, lookup or copying.
    if (type(item) is not dict or len(item) != 7 or
            any(type(key) is not str or len(key) > 64 for key in item) or
            frozenset(item) != _CUSTODY_FIELDS):
        return _custody_metadata()
    for key in ('profile', 'scope', 'observation_state'):
        if type(item[key]) is not str or len(item[key]) > 64:
            return _custody_metadata()
    state, stage = item['observation_state'], item['stage']
    if (item['profile'] != _CUSTODY_PROFILE or item['scope'] != _CUSTODY_SCOPE or
            state not in ('unobserved', 'unmeasured', 'stage_reported') or
            any(item[key] is not None for key in
                ('root_cause', 'provider_tokens', 'usd_micros'))):
        return _custody_metadata()
    if state == 'stage_reported':
        if type(stage) is not str or len(stage) > 32 or stage not in _CUSTODY_STAGES:
            return _custody_metadata()
    elif stage is not None:
        return _custody_metadata()
    return _custody_metadata(state, stage)


class ObservationError(ValueError):
    """Fixed refusal code without request, context or exception prose."""


def _wire(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(',', ':'), allow_nan=False).encode('utf-8')


def _release_active(method):
    @wraps(method)
    def guarded(self, *args, **kwargs):
        with self._lock:
            prior = self._active
            try:
                return method(self, *args, **kwargs)
            finally:
                if not prior:
                    self._active = False
    return guarded


class ObservedProjectionMemory:
    """Exclusive backend ownership, fixed query horizon and ordinal receipts.

    Construction of the observer, accounting/export overhead, fixture setup,
    waiting between calls and operator project removal are outside timed spans.
    Backend initialization/insertion/query/descriptor teardown are observed.
    Sources/manifests remain on disk after close. Never share the private backend.
    """

    def __init__(self, horizon, *, local_token_profile='unknown', clock=None):
        if (type(local_token_profile) is not str or local_token_profile not in
                ('unknown', 'pinned_local_no_model') or
                (clock is not None and not callable(clock))):
            raise ObservationError('observation_profile')
        self._ledger = Ledger(horizon)
        self._model_work = ('none_observed_local' if local_token_profile ==
                            'pinned_local_no_model' else 'unknown')
        self._profile = local_token_profile
        self._clock = time.monotonic_ns if clock is None else clock
        self._lock = threading.RLock()
        self._active = False
        self._backend = None
        self._state = 'new'
        self._initialization = 'not_attempted'
        self._cleanup = 'not_attempted'
        self._incomplete = False
        self._build_ns = 0
        self._query_ns = 0
        self._insert_bytes = self._query_bytes = self._output_bytes = 0
        self._insert_calls = self._inserted = self._replayed = 0
        self._insert_failed = self._insert_cancelled = 0
        self._query_failed = self._query_cancelled = 0
        self._native_queries = 0
        self._images = {}
        self._projection_bytes = 0
        self._receipts = []
        self._failures = dict.fromkeys(FAILURE_PHASES)
        self._failure_capture_failed = dict.fromkeys(FAILURE_PHASES, False)

    def _note_failure(self, error, phase):
        # Best-effort diagnostics must never replace the paid task exception.
        try:
            self._failures[phase] = projection_failure_metadata(error, phase=phase)
            self._failure_capture_failed[phase] = False
        except BaseException:
            self._failures[phase] = None
            self._failure_capture_failed[phase] = True

    def failure_observation(self, *, phase):
        """Detached last exception per body phase; no change to snapshot schema.

        Pre-body lifecycle/capacity guards and end-clock/finally failures are
        outside coverage. Recording is inside the failure-operation timing.
        A failed bridge constructor does not return an instance to inspect.
        """
        validate_phase(phase)
        with self._lock:
            item = self._failures[phase]
            if item is not None:
                return copy.deepcopy(item)
            return (unmeasured_failure(phase) if self._failure_capture_failed[phase]
                    else empty_failure())

    def custody_failure_observation(self):
        """Detached optional last caught custody stage, including after close.

        Getter overhead is outside operation timing/accounting. Missing backend
        is unobserved; unsupported/failed/malformed diagnostics are unmeasured.
        A retained stage is not evidence about the current call or root cause.
        """
        with self._lock:
            backend = self._backend
            if backend is None:
                return _custody_metadata('unobserved')
            if type(backend) is not _CUSTODY_NATIVE_TYPE or _CUSTODY_NATIVE_GETTER is None:
                return _custody_metadata()
            try:
                return _bounded_custody_metadata(_CUSTODY_NATIVE_GETTER(backend))
            except BaseException:
                return _custody_metadata()

    @property
    def horizon(self):
        return self._ledger.horizon

    def _require(self, *states):
        if self._active or self._state not in states:
            raise ObservationError('observation_lifecycle')

    def _now(self):
        try:
            value = self._clock()
            if counter(value) is not None:
                return value
        except Exception:
            pass
        self._incomplete = True
        return None

    def _duration(self, start):
        end = self._now()
        if start is None or end is None or end < start:
            self._incomplete = True
            return None
        return end - start

    def _add(self, current, amount):
        if current is None or amount is None or current + amount > COUNTER_LIMIT:
            self._incomplete = True
            return None
        return current + amount

    def _end_clock(self, started):
        try:
            return self._duration(started), None
        except BaseException as error:
            self._incomplete = True
            return None, error

    @staticmethod
    def _outcome(error):
        return 'failed' if issubclass(type(error), Exception) else 'cancelled'

    @_release_active
    def open(self, repository, native):
        with self._lock:
            self._require('new')
            self._active = True
            started, task_error = None, None
            self._state = 'building'
            try:
                started = self._now()
                self._backend = projection.NativeProjectionMemory(repository, native)
                self._initialization = 'succeeded'
                return self
            except BaseException as error:
                task_error = error
                self._initialization = self._outcome(error)
                self._state = 'initialization_failed'
                self._note_failure(error, 'initialization')
                raise
            finally:
                duration, clock_error = self._end_clock(started)
                self._build_ns = self._add(self._build_ns, duration)
                if clock_error is not None and task_error is None:
                    self._initialization = 'cancelled'
                    self._state = 'initialization_failed'
                    raise clock_error

    @_release_active
    def insert(self, trajectory):
        with self._lock:
            self._require('building')
            if self._insert_calls >= MAX_INSERT_CALLS:
                raise ObservationError('insert_capacity')
            self._active = True
            started, task_error = None, None
            self._insert_calls += 1
            try:
                started = self._now()
                projection._admit_tree(trajectory)
                raw = _wire(trajectory)
                if len(raw) > projection.INPUT_BYTES:
                    raise ObservationError('input_bound')
                self._insert_bytes = self._add(self._insert_bytes, len(raw))
                captured = projection.decode_trajectory(raw)
                item = projection.project_trajectory(captured)
                key = hashlib.sha256(item.trajectory_id.encode('utf-8')).digest()
                prior = self._images.get(key)
                self._backend.insert(captured)
                if prior is None:
                    self._images[key] = item.input_sha256
                    self._projection_bytes += len(item.body)
                    self._inserted += 1
                elif prior == item.input_sha256:
                    self._replayed += 1
                else:
                    # A backend accepting changed identity violates this profile.
                    self._state = 'backend_failed'
                    raise ObservationError('backend_identity')
            except BaseException as error:
                task_error = error
                self._note_failure(error, 'insert')
                if self._outcome(error) == 'cancelled':
                    self._insert_cancelled += 1
                else:
                    self._insert_failed += 1
                raise
            finally:
                duration, clock_error = self._end_clock(started)
                self._build_ns = self._add(self._build_ns, duration)
                if clock_error is not None and task_error is None:
                    self._insert_cancelled += 1
                    raise clock_error

    def _metadata(self, query_image):
        try:
            meta = self._backend.post_query_hook()
            if (type(meta) is dict and type(meta.get('queries')) is int and
                    meta['queries'] == self._native_queries + 1 and
                    type(meta.get('state')) is str and meta['state'] in STATES and
                    type(meta.get('query_image_ignored')) is bool and
                    meta['query_image_ignored'] == (query_image is not None)):
                self._native_queries = meta['queries']
                return meta['state'], True
        except Exception:
            pass
        self._incomplete = True
        return 'unmeasured', False

    @_release_active
    def query(self, query, query_image=None):
        with self._lock:
            self._require('building', 'querying')
            if len(self._receipts) >= self.horizon:
                raise ObservationError('query_capacity')
            self._active = True
            self._state = 'querying'
            ordinal = len(self._receipts)
            started, task_error = None, None
            ingress = egress = 0
            state, outcome, bound = 'input_refused', 'failed', False
            called = False
            try:
                started = self._now()
                projection._text(query, projection.QUERY_CHARS, nonempty=True, multiline=True)
                if query_image is not None:
                    projection._text(query_image, 4096, nonempty=True)
                ingress = len(_wire({'query': query, 'query_image': query_image}))
                called = True
                context = self._backend.query(query, query_image=query_image)
                state, bound = self._metadata(query_image)
                if type(context) is not list or len(context) > 1:
                    raise ObservationError('context_schema')
                for item in context:
                    if (type(item) is not dict or set(item) != {'type', 'value'} or
                            item['type'] != 'text'):
                        raise ObservationError('context_schema')
                    projection._text(item['value'], projection.CONTEXT_BYTES, multiline=True)
                raw = _wire(context)
                if len(raw) > projection.CONTEXT_BYTES:
                    raise ObservationError('context_bound')
                result = json.loads(raw.decode('utf-8'))
                egress = len(raw)
                outcome = ('failed' if state in ('refresh_failed', 'cleanup_failed', 'failed')
                           else 'unknown' if not bound else 'succeeded')
                return result
            except BaseException as error:
                task_error = error
                self._note_failure(error, 'query')
                outcome = self._outcome(error)
                if called and not bound:
                    state, bound = self._metadata(query_image)
                if outcome == 'cancelled':
                    state = 'cancelled'
                elif called and state not in ('refresh_failed', 'cleanup_failed', 'failed'):
                    state = 'failed'
                raise
            finally:
                duration, clock_error = self._end_clock(started)
                if clock_error is not None and task_error is None:
                    egress, state, outcome = 0, 'cancelled', 'cancelled'
                self._query_ns = self._add(self._query_ns, duration)
                self._query_bytes = self._add(self._query_bytes, ingress)
                self._output_bytes = self._add(self._output_bytes, egress)
                self._query_failed += outcome == 'failed'
                self._query_cancelled += outcome == 'cancelled'
                self._receipts.append({'ordinal': ordinal, 'state': state,
                                       'outcome': outcome, 'native_metadata_bound': bound,
                                       'input_bytes': ingress, 'output_bytes': egress,
                                       'component_ns': duration,
                                       'query_image_ignored': query_image is not None})
                self._ledger.close_unit('query', ordinal, coverage='complete',
                                        model_work=self._model_work, local_ns=duration,
                                        constructed_bytes=ingress + egress)
                if clock_error is not None and task_error is None:
                    raise clock_error

    def query_receipt(self, ordinal):
        with self._lock:
            if type(ordinal) is not int or not 0 <= ordinal < len(self._receipts):
                raise ObservationError('receipt_ordinal')
            return copy.deepcopy(self._receipts[ordinal])

    @_release_active
    def close(self):
        with self._lock:
            if self._active:
                raise ObservationError('observation_lifecycle')
            if self._state in ('finalized', 'cleanup_failed'):
                return self.snapshot()
            self._active = True
            started, task_error, start_clock_error = None, None, None
            try:
                try:
                    started = self._now()
                except BaseException as error:
                    self._incomplete = True
                    start_clock_error = error
                # An optional clock cannot prevent once-only owned teardown.
                if self._backend is None:
                    self._cleanup = 'not_needed'
                else:
                    self._backend.close()
                    self._cleanup = 'succeeded'
                self._state = 'finalized'
            except BaseException as error:
                task_error = error
                self._cleanup = self._outcome(error)
                self._state = 'cleanup_failed'
                self._note_failure(error, 'cleanup')
                raise
            finally:
                duration, clock_error = self._end_clock(started)
                self._build_ns = self._add(self._build_ns, duration)
                self._ledger.close_unit('build', None,
                                        coverage='complete' if self._initialization == 'succeeded' else 'incomplete',
                                        model_work=self._model_work, local_ns=self._build_ns,
                                        constructed_bytes=self._insert_bytes)
                self._ledger.finalize()
                if task_error is None:
                    # Successful cleanup remains succeeded/not_needed even when
                    # diagnostic interruption propagates. The body-only failure
                    # getter does not claim coverage of these finally errors.
                    if start_clock_error is not None:
                        raise start_clock_error
                    if clock_error is not None:
                        raise clock_error
            return self.snapshot()

    def snapshot(self):
        with self._lock:
            view = {'status': self._state, 'local_token_profile': self._profile,
                    'initialization_attempted': self._initialization != 'not_attempted',
                    'initialization_outcome': self._initialization,
                    'cleanup_outcome': self._cleanup, 'insert_calls': self._insert_calls,
                    'inserted': self._inserted, 'replayed': self._replayed,
                    'insert_failed': self._insert_failed, 'insert_cancelled': self._insert_cancelled,
                    'query_calls': len(self._receipts), 'query_failed': self._query_failed,
                    'query_cancelled': self._query_cancelled,
                    'unlaunched_queries': self.horizon - len(self._receipts),
                    'unique_projection_bytes': self._projection_bytes,
                    'insert_ingress_bytes': self._insert_bytes,
                    'query_ingress_bytes': self._query_bytes,
                    'returned_context_bytes': self._output_bytes, 'images_ingested': 0,
                    'observation_incomplete': self._incomplete,
                    'build_component_ns': self._build_ns, 'query_component_ns': self._query_ns,
                    'query_receipts': self._receipts}
            return {'profile': PROFILE, 'observed': copy.deepcopy(view),
                    'ledger': self._ledger.snapshot()}
