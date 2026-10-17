"""Local text construction/callback controller, not official provider dispatch.

Memory, clocks and callbacks are trusted, exclusively owned operator inputs.
Generic callbacks never establish complete physical provider coverage. Private
construction bytes contain requests/answers; numeric exports contain no prose.
"""
from __future__ import annotations

import copy
from dataclasses import dataclass
import hashlib
import json
import secrets
import threading
import time

from .cost_observation import counter
from .longmemeval_bridge import PROFILE as BRIDGE_PROFILE
from .longmemeval_observed import PROFILE as LOCAL_PROFILE
from .longmemeval_projection import _text, CONTEXT_BYTES, QUERY_CHARS
from .memory_benchmark_cost import Ledger, PROFILE as LEDGER_PROFILE, USAGE_KEYS

PROFILE = 'longmemeval/local-text-callback-controller-v1'
CONSTRUCTION_PROFILE = 'longmemeval/local-text-callback-construction-v1'
SYSTEM = 'Use reference memory; verify canonical sources.'
MAX_HORIZON = 32
MAX_REQUEST = 65536
MAX_REPLY_CHARS = 32768
MAX_HELD = 8388608
RECEIPT_KEYS = ('ordinal', 'outcome', 'state', 'native_metadata_bound',
                'input_bytes', 'output_bytes', 'component_ns', 'query_image_ignored')


class ControllerError(ValueError):
    """Fixed code only."""


class DispatchTicket:
    """Identity-only one-use permission; fields live in the owning controller."""
    __slots__ = ()

    def __repr__(self):
        return '<opaque local callback ticket>'


@dataclass(frozen=True)
class Construction:
    __slots__ = ('body',)
    body: bytes

    @property
    def sha256(self):
        return hashlib.sha256(self.body).hexdigest()


def _wire(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False,
                      separators=(',', ':'), allow_nan=False).encode('utf-8')


def _keys(value, names):
    if (type(value) is not dict or len(value) != len(names)
            or any(type(key) is not str for key in value)
            or any(key not in value for key in names)):
        raise ControllerError('controller_schema')


def _mapping(value, maximum):
    if (type(value) is not dict or len(value) > maximum
            or any(type(key) is not str for key in value)):
        raise ControllerError('controller_local_shape')
    return value


def _context(value):
    if type(value) is not list or len(value) > 1:
        raise ControllerError('controller_text_profile')
    for item in value:
        _keys(item, ('type', 'value'))
        if type(item['type']) is not str or item['type'] != 'text':
            raise ControllerError('controller_text_profile')
        _text(item['value'], CONTEXT_BYTES, multiline=True)
    raw = _wire(value)
    if len(raw) > CONTEXT_BYTES:
        raise ControllerError('controller_context_bound')
    return copy.deepcopy(value), len(raw)


def _receipt(value, ordinal):
    _keys(value, RECEIPT_KEYS)
    if (type(value['ordinal']) is not int or value['ordinal'] != ordinal
            or type(value['outcome']) is not str or value['outcome'] not in ('succeeded', 'failed', 'cancelled', 'unknown')
            or type(value['state']) is not str or value['state'] not in
            ('delivered', 'no_evidence', 'sanitized_empty', 'refresh_failed', 'cleanup_failed',
             'failed', 'cancelled', 'input_refused', 'unmeasured')
            or type(value['native_metadata_bound']) is not bool
            or type(value['query_image_ignored']) is not bool
            or counter(value['input_bytes']) is None or counter(value['output_bytes']) is None
            or counter(value['input_bytes'] + value['output_bytes']) is None
            or (value['component_ns'] is not None and counter(value['component_ns']) is None)):
        raise ControllerError('controller_local_receipt')
    return dict(value)


def _report(value, phase):
    _keys(value, ('outcome', 'value', 'usage', 'usd_micros', 'billing'))
    outcome, answer, usage = value['outcome'], value['value'], value['usage']
    if type(outcome) is not str or outcome not in ('succeeded', 'failed', 'cancelled'):
        raise ControllerError('controller_report')
    if outcome == 'succeeded':
        if phase == 'reader':
            _text(answer, MAX_REPLY_CHARS, multiline=True)
        elif answer is not None and type(answer) is not bool:
            raise ControllerError('controller_grade')
    elif answer is not None:
        raise ControllerError('controller_report')
    if usage is not None:
        if (type(usage) is not dict or len(usage) > 5
                or any(type(key) is not str or key not in USAGE_KEYS for key in usage)
                or any(item is not None and counter(item) is None for item in usage.values())):
            raise ControllerError('controller_usage')
    if (value['usd_micros'] is not None and counter(value['usd_micros']) is None
            or type(value['billing']) is not str or value['billing'] not in
            ('unknown', 'provider_reported', 'invoice_allocated')):
        raise ControllerError('controller_billing')
    return copy.deepcopy(value)


class Controller:
    def __init__(self, memory, questions, *, reader_budget=4096, clock=None):
        if (type(questions) is not list or not 1 <= len(questions) <= MAX_HORIZON
                or type(reader_budget) is not int or not 32 <= reader_budget <= MAX_REQUEST
                or clock is not None and not callable(clock)):
            raise ControllerError('controller_parameters')
        for item in questions:
            _keys(item, ('question', 'query_image'))
            _text(item['question'], QUERY_CHARS, nonempty=True, multiline=True)
            if item['query_image'] is not None:
                raise ControllerError('controller_images_unsupported')
        self._questions = copy.deepcopy(questions)
        self._memory, self._budget = memory, reader_budget
        self._clock = time.monotonic_ns if clock is None else clock
        self._ledger = Ledger(len(questions))
        self._lock = threading.RLock()
        self._busy = self._closed = self._joined = self._blocked = self._native_teardown = False
        self._seal = self._final = None
        self._next_local = self._entries = self._faults = 0
        self._held = len(_wire(self._questions))
        self._tickets = {}
        self._slots = [{'query_outcome': 'unlaunched', 'local_ordinal': None,
                        'local_receipt': None, 'context': None, 'reader_body': None}
                       for _ in questions]
        self._units = {(phase, i): {'issued': 0, 'entries': 0, 'bytes': 0,
                                   'outcome': 'unlaunched', 'value': None, 'closed': False}
                       for phase in ('reader', 'judge') for i in range(len(questions))}
        view = _mapping(memory.snapshot(), 32)
        local = _mapping(view['local_observation'], 8)
        ledger, observed = _mapping(local['ledger'], 32), _mapping(local['observed'], 40)
        if (type(view.get('profile')) is not str or view['profile'] != BRIDGE_PROFILE or view.get('closed') is not False
                or type(ledger['horizon']) is not int or ledger['horizon'] != len(questions)
                or type(observed['query_calls']) is not int or observed['query_calls'] != 0):
            raise ControllerError('controller_fresh_memory')
        if counter(observed['insert_calls']) is None:
            raise ControllerError('controller_fresh_memory')
        self._initial_inserts = observed['insert_calls']

    def _ready(self, *, finalizing=False):
        if self._closed or self._busy or self._blocked and not finalizing:
            raise ControllerError('controller_lifecycle')

    def _ordinal(self, ordinal):
        if type(ordinal) is not int or not 0 <= ordinal < len(self._questions):
            raise ControllerError('controller_ordinal')

    def _hold(self, size):
        if self._held + size > MAX_HELD:
            raise ControllerError('controller_held_bound')
        self._held += size

    def _now(self):
        try:
            value = self._clock()
            if counter(value) is not None:
                return value
        except Exception:
            pass
        self._faults += 1
        return None

    def query(self, ordinal):
        with self._lock:
            self._ready()
            self._ordinal(ordinal)
            slot = self._slots[ordinal]
            if self._seal is not None or slot['query_outcome'] != 'unlaunched':
                raise ControllerError('controller_query_reused')
            observed = self._memory.snapshot()['local_observation']['observed']
            before = observed['query_calls']
            if observed['insert_calls'] != self._initial_inserts:
                self._faults += 1
                self._blocked = True
                raise ControllerError('controller_external_insert')
            if before != self._next_local:
                self._faults += 1
                raise ControllerError('controller_external_query')
            slot['query_outcome'] = 'pending'
            self._busy = True
            entered = False
            try:
                self._memory.set_query_context(query_invocation_id='controller-' + secrets.token_hex(16))
                entered = True
                question = self._questions[ordinal]
                result = self._memory.query(question['question'], None)
                context, size = _context(result)
                hook = _mapping(self._memory.post_query_hook(query=question['question'], query_image=None,
                                                             memory_context=result), 16)
                receipt = _receipt(hook['query_receipt'], before)
                if (receipt['outcome'] != 'succeeded' or not receipt['native_metadata_bound']
                        or receipt['state'] not in ('delivered', 'no_evidence', 'sanitized_empty')):
                    slot['query_outcome'] = receipt['outcome'] if receipt['outcome'] != 'succeeded' else 'unknown'
                    raise ControllerError('controller_query_not_succeeded')
                self._hold(size)
                slot['context'] = context
                slot['query_outcome'] = 'succeeded'
                return copy.deepcopy(context)
            except BaseException as error:
                if slot['query_outcome'] == 'pending':
                    slot['query_outcome'] = 'failed' if issubclass(type(error), Exception) else 'cancelled'
                raise
            finally:
                try:
                    after = self._memory.snapshot()['local_observation']['observed']
                    delta = after['query_calls'] - before
                    if delta == 1:
                        receipt = _receipt(after['query_receipts'][before], before)
                        slot['local_ordinal'], slot['local_receipt'] = before, receipt
                        self._next_local += 1
                    elif delta != 0:
                        self._faults += 1
                        raise ControllerError('controller_external_query')
                finally:
                    try:
                        if entered:
                            self._memory.clear_query_context()
                    finally:
                        self._busy = False

    def _issue(self, phase, ordinal, body):
        unit = self._units[(phase, ordinal)]
        if unit['issued'] >= 2 or unit['outcome'] in ('pending', 'succeeded', 'cancelled'):
            raise ControllerError('controller_attempt_budget')
        ticket = DispatchTicket()
        unit['issued'] += 1
        unit['outcome'] = 'pending'
        self._tickets[ticket] = {'phase': phase, 'ordinal': ordinal,
                                 'body': body, 'consumed': False}
        return ticket

    def prepare_reader(self, ordinal):
        with self._lock:
            self._ready()
            self._ordinal(ordinal)
            if self._seal is not None or self._slots[ordinal]['query_outcome'] != 'succeeded':
                raise ControllerError('controller_reader_state')
            slot = self._slots[ordinal]
            if slot['reader_body'] is None:
                value = {'system': SYSTEM, 'question': self._questions[ordinal]['question'],
                         'memory_context': slot['context']}
                body = _wire(value)
                if len(body) > self._budget:
                    value['memory_context'] = []
                    body = _wire(value)
                if len(body) > self._budget:
                    raise ControllerError('controller_request_bound')
                self._hold(len(body))
                slot['reader_body'] = body
            return self._issue('reader', ordinal, slot['reader_body'])

    def prepare_judge(self, ordinal, construction):
        with self._lock:
            self._ready()
            self._ordinal(ordinal)
            if self._seal is None or construction is not self._seal:
                raise ControllerError('controller_construction_binding')
            answer = self._units[('reader', ordinal)]
            if answer['outcome'] != 'succeeded':
                raise ControllerError('controller_judge_state')
            unit = self._units[('judge', ordinal)]
            if unit['issued'] >= 2 or unit['outcome'] in ('pending', 'succeeded', 'cancelled'):
                raise ControllerError('controller_attempt_budget')
            body = _wire({'question': self._questions[ordinal]['question'],
                          'answer': answer['value'], 'construction_sha256': self._seal.sha256})
            if len(body) > MAX_REQUEST:
                raise ControllerError('controller_request_bound')
            self._hold(len(body))
            return self._issue('judge', ordinal, body)

    def dispatch(self, ticket, callback):
        """Judge callback is a factory invoked only after owned construction seal."""
        with self._lock:
            self._ready()
            if type(ticket) is not DispatchTicket or ticket not in self._tickets:
                raise ControllerError('controller_ticket_binding')
            row = self._tickets[ticket]
            phase, ordinal = row['phase'], row['ordinal']
            if row['consumed'] or phase == 'reader' and self._seal is not None or not callable(callback):
                raise ControllerError('controller_ticket_reused')
            row['consumed'] = True
            unit = self._units[(phase, ordinal)]
            observed = self._ledger.begin_call(phase, ordinal, secrets.token_hex(16), secrets.token_hex(16))
            self._busy = True
            start = None
            task_error = None
            reply = {'outcome': 'failed', 'value': None, 'usage': None,
                     'usd_micros': None, 'billing': 'unknown'}
            try:
                start = self._now()
                unit['entries'] += 1
                unit['bytes'] += len(row['body'])
                self._entries += 1
                dispatch = callback() if phase == 'judge' else callback
                if not callable(dispatch):
                    raise ControllerError('controller_evaluator_factory')
                reply = _report(dispatch(row['body']), phase)
                if phase == 'reader' and reply['outcome'] == 'succeeded':
                    self._hold(len(reply['value'].encode('utf-8')))
                unit['value'] = reply['value']
                return reply['value']
            except BaseException as error:
                task_error = error
                reply['outcome'] = 'failed' if issubclass(type(error), Exception) else 'cancelled'
                unit['value'] = None
                raise
            finally:
                clock_error = None
                try:
                    try:
                        end = self._now()
                    except BaseException as error:
                        end, clock_error = None, error
                        self._faults += 1
                    duration = end - start if start is not None and end is not None and end >= start else None
                    unit['outcome'] = reply['outcome']
                    self._ledger.finish_call(observed, outcome=reply['outcome'], usage=reply['usage'],
                                             usd_micros=reply['usd_micros'], billing=reply['billing'],
                                             component_ns=duration)
                finally:
                    self._busy = False
                if clock_error is not None and task_error is None:
                    raise clock_error

    def _join_local(self, final):
        if self._joined:
            return
        final = _mapping(final, 32)
        if type(final.get('profile')) is not str or final['profile'] != BRIDGE_PROFILE or final.get('closed') is not True:
            raise ControllerError('controller_local_final')
        local = _mapping(final['local_observation'], 8)
        ledger, view = _mapping(local['ledger'], 32), _mapping(local['observed'], 40)
        if (type(local['profile']) is not str or local['profile'] != LOCAL_PROFILE
                or type(ledger['profile']) is not str or ledger['profile'] != LEDGER_PROFILE
                or ledger['frozen'] is not True or type(ledger['horizon']) is not int
                or ledger['horizon'] != len(self._questions)
                or type(view['query_receipts']) is not list or len(view['query_receipts']) != self._next_local
                or type(view['query_calls']) is not int or view['query_calls'] != self._next_local
                or type(view['insert_calls']) is not int or view['insert_calls'] != self._initial_inserts
                or type(view['local_token_profile']) is not str or view['local_token_profile'] not in
                ('unknown', 'pinned_local_no_model')
                or counter(view['insert_ingress_bytes']) is None
                or view['build_component_ns'] is not None and counter(view['build_component_ns']) is None
                or type(view['observation_incomplete']) is not bool
                or type(view['initialization_outcome']) is not str
                or view['initialization_outcome'] not in ('succeeded', 'failed', 'cancelled', 'not_attempted')
                or type(view['cleanup_outcome']) is not str
                or view['cleanup_outcome'] not in ('succeeded', 'failed', 'cancelled', 'not_needed', 'not_attempted')):
            raise ControllerError('controller_local_final')
        receipts = [_receipt(item, i) for i, item in enumerate(view['query_receipts'])]
        mapping = [slot['local_ordinal'] for slot in self._slots if slot['local_ordinal'] is not None]
        if sorted(mapping) != list(range(self._next_local)):
            raise ControllerError('controller_local_mapping')
        for slot in self._slots:
            if slot['local_ordinal'] is not None and slot['local_receipt'] != receipts[slot['local_ordinal']]:
                raise ControllerError('controller_local_changed')
        # Full projection was validated before the first destination mutation.
        model = 'none_observed_local' if view['local_token_profile'] == 'pinned_local_no_model' else 'unknown'
        coverage = 'incomplete' if view['observation_incomplete'] or view['initialization_outcome'] != 'succeeded' else 'complete'
        self._ledger.close_unit('build', None, coverage=coverage, model_work=model,
                                local_ns=view['build_component_ns'], constructed_bytes=view['insert_ingress_bytes'])
        for i, slot in enumerate(self._slots):
            item = slot['local_receipt']
            self._ledger.close_unit('query', i, coverage=coverage if item else 'incomplete',
                                    model_work=model if item else 'unknown',
                                    local_ns=item['component_ns'] if item else None,
                                    constructed_bytes=item['input_bytes'] + item['output_bytes'] if item else None)
        self._joined = True

    def _close_units(self, phase):
        for i in range(len(self._questions)):
            unit = self._units[(phase, i)]
            if not unit['closed']:
                # Generic callback entry does not prove physical provider coverage.
                self._ledger.close_unit(phase, i, coverage='incomplete',
                                        model_work='reported_calls' if unit['entries'] else 'unknown',
                                        local_ns=None, constructed_bytes=unit['bytes'] if unit['entries'] else None)
                unit['closed'] = True
                if phase == 'judge':
                    self._ledger.grade(i, unit['value'] if unit['outcome'] == 'succeeded' else None)

    def _close_native(self):
        if self._native_teardown:
            return self._memory.snapshot()
        self._native_teardown = True
        return self._memory.close()

    def seal_construction(self):
        with self._lock:
            self._ready()
            if self._seal is not None:
                return self._seal
            self._busy = True
            try:
                try:
                    final = self._close_native()
                except BaseException as error:
                    self._faults += 1
                    final = self._memory.snapshot()
                    if not issubclass(type(error), Exception):
                        self._blocked = True
                        try:
                            self._join_local(final)
                            self._close_units('reader')
                        except BaseException:
                            self._faults += 1
                        raise
                self._join_local(final)
                self._close_units('reader')
                slots = []
                for i, (question, slot) in enumerate(zip(self._questions, self._slots)):
                    reader = self._units[('reader', i)]
                    slots.append({'ordinal': i, 'question': question['question'],
                                  'query_outcome': slot['query_outcome'], 'local_ordinal': slot['local_ordinal'],
                                  'reader_request': json.loads(slot['reader_body']) if slot['reader_body'] else None,
                                  'reader_outcome': reader['outcome'], 'reader_answer': reader['value']})
                body = _wire({'profile': CONSTRUCTION_PROFILE, 'horizon': len(self._questions), 'slots': slots})
                if len(body) > MAX_HELD:
                    raise ControllerError('controller_construction_bound')
                self._hold(len(body))
                self._seal = Construction(body)
                return self._seal
            except BaseException:
                self._blocked = True
                raise
            finally:
                self._busy = False

    def snapshot(self):
        with self._lock:
            if self._final is not None:
                return copy.deepcopy(self._final)
            return {'profile': PROFILE, 'closed': self._closed, 'local_imports': int(self._joined),
                    'planned_to_local': [s['local_ordinal'] for s in self._slots],
                    'callback_entries': self._entries, 'pending_tickets': sum(not r['consumed'] for r in self._tickets.values()),
                    'observation_faults': self._faults, 'provider_coverage': 'UNKNOWN',
                    'construction_blocked': self._blocked,
                    'native_teardown_attempted': self._native_teardown,
                    'provider_usage_verified': False,
                    'construction_sha256': self._seal.sha256 if self._seal else None,
                    'ledger': self._ledger.snapshot()}

    def finalize(self):
        with self._lock:
            if self._closed:
                return self.snapshot()
            self._ready(finalizing=True)
            cancellation = None
            try:
                self.seal_construction()
            except Exception:
                self._faults += 1
            except BaseException as error:
                self._faults += 1
                cancellation = error
            self._busy = True
            try:
                if not self._native_teardown:
                    try:
                        final = self._close_native()
                    except BaseException as error:
                        self._faults += 1
                        if not issubclass(type(error), Exception) and cancellation is None:
                            cancellation = error
                        try:
                            final = self._memory.snapshot()
                        except BaseException:
                            self._faults += 1
                            final = None
                    if final is not None:
                        try:
                            self._join_local(final)
                        except BaseException:
                            self._faults += 1
                self._close_units('reader')
                self._close_units('judge')
                self._ledger.finalize()
                self._closed = True
                self._final = self.snapshot()
            finally:
                self._busy = False
            if cancellation is not None:
                raise cancellation
            return self.snapshot()
