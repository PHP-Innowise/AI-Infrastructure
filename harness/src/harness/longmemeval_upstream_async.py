"""Owned selected-upstream async reader/sync judge; injected SDK-shaped calls.

Trusted pinned helpers, serializers and creates are supplied by the operator.
No heavy upstream/SDK imports, provider bootstrap or physical wire proof.
"""
from __future__ import annotations

import copy
import json
import math
import secrets
from types import SimpleNamespace

from . import longmemeval_lifecycle as base
from .longmemeval_chat_receipts import ChatController
from .longmemeval_provider_usage import RAW_BYTES

PROFILE = 'longmemeval/selected-upstream-mixed-controller-v1'
CONSTRUCTION_PROFILE = 'longmemeval/upstream-sdk-argument-construction-v1'
JOURNAL_PROFILE = 'longmemeval/upstream-sdk-argument-judge-journal-v1'


class UpstreamController(ChatController):
    def __init__(self, memory, questions, *, reader_builder, reader_helper,
                 judge_helper, reader_timeout, judge_timeout, **parameters):
        if not all(callable(item) for item in (reader_builder, reader_helper, judge_helper)):
            raise base.ControllerError('upstream_helpers')
        for value in (reader_timeout, judge_timeout):
            if type(value) not in (int, float) or not 0 < value <= 3600 or not math.isfinite(value):
                raise base.ControllerError('upstream_timeout')
        self._reader_builder, self._reader_helper, self._judge_helper = reader_builder, reader_helper, judge_helper
        self._timeouts = {'reader': reader_timeout, 'judge': judge_timeout}
        super().__init__(memory, questions, **parameters)

    def _args(self):
        model, maximum = self._chat_profiles['reader']
        return SimpleNamespace(model=model, max_completion_tokens=maximum,
            timeout_seconds=self._timeouts['reader'], base_url=None,
            reasoning_effort=None, temperature=None, top_p=None,
            presence_penalty=None, top_k=None, repetition_penalty=None,
            reader_enable_thinking=True)

    def _chat(self, local_body, phase):
        value = json.loads(local_body)
        value.pop('system', None)
        model, maximum = self._chat_profiles[phase]
        messages = [{'role': 'system', 'content': base.SYSTEM},
                    {'role': 'user', 'content': base._wire(value).decode('utf-8')}]
        expected = {'model': model, 'messages': messages,
                    'max_completion_tokens': maximum, 'timeout': self._timeouts[phase]}
        if phase == 'reader':
            compiled = self._reader_builder(self._args(), copy.deepcopy(messages))
            if base._wire(compiled) != base._wire(expected):
                raise base.ControllerError('upstream_request_profile')
        return base._wire(expected)

    def dispatch(self, *_, **__):
        raise base.ControllerError('upstream_use_owned_phase_method')

    def _enter(self, ticket, phase, bindings):
        with self._lock:
            self._ready()
            if type(ticket) is not base.DispatchTicket or ticket not in self._tickets:
                raise base.ControllerError('upstream_ticket')
            row = self._tickets[ticket]
            if (row['phase'] != phase or row['consumed']
                    or phase == 'reader' and self._seal is not None
                    or not all(callable(item) for item in bindings)):
                raise base.ControllerError('upstream_binding')
            observed = self._ledger.begin_call(phase, row['ordinal'], secrets.token_hex(16), secrets.token_hex(16))
            row['consumed'] = True
            self._busy = True
            return row, self._units[(phase, row['ordinal'])], observed

    def _boundary(self, row, kwargs):
        # Active owner excludes mutating callers throughout the await. The short
        # lock is released before trusted creation; RLock is never held across await.
        with self._lock:
            if (not self._busy or row['injected_transport_boundaries']
                    or base._wire(kwargs) != row['body']):
                raise base.ControllerError('upstream_create_binding')
            detached = json.loads(row['body'])
            row['injected_transport_boundaries'] = 1
            row['request_bytes'] = len(row['body'])
            self._units[(row['phase'], row['ordinal'])]['bytes'] += len(row['body'])
            return detached

    def _capture(self, row, response, serializer):
        try:
            raw = serializer(response)
            if type(raw) is str:
                raw = raw.encode('utf-8')
            if type(raw) is not bytes or len(raw) > RAW_BYTES:
                raise base.ControllerError('upstream_serialization_profile')
            row['response_bytes'] = len(raw)
            if row['capture'] is not None:
                # Original response identity and selected helper return contracts
                # survive; the detached serialization is observation only.
                row['capture'].deliver(raw, lambda _: None)
        except BaseException as error:
            self._faults += 1
            row['capture'] = None
            if not issubclass(type(error), Exception):
                raise
        return response

    def _started(self, unit):
        with self._lock:
            start = self._now()
            unit['entries'] += 1
            self._entries += 1
            return start

    def _finish(self, row, unit, observed, start, value, error):
        with self._lock:
            try:
                return self._finish_owned(row, unit, observed, start, value, error)
            finally:
                self._busy = False

    def _finish_owned(self, row, unit, observed, start, value, error):
        interruptions = []
        usage = self._capture_usage(row, interruptions)
        if error is None and interruptions:
            error = interruptions[0]
        outcome = ('succeeded' if error is None else
                   'failed' if issubclass(type(error), Exception) else 'cancelled')
        reply = {'outcome': outcome, 'value': value if error is None else None,
                 'usage': usage, 'usd_micros': None, 'billing': 'unknown'}
        try:
            reply = base._report(reply, row['phase'])
            if row['phase'] == 'reader' and error is None:
                self._hold(len(value.encode('utf-8')))
        except BaseException as invalid:
            if error is None:
                error = invalid
            reply['outcome'] = 'failed' if issubclass(type(error), Exception) else 'cancelled'
            reply['value'] = None
        unit['value'], unit['outcome'] = reply['value'], reply['outcome']
        row['outcome'] = reply['outcome']
        clock_error = None
        try:
            try:
                end = self._now()
            except BaseException as interrupted:
                end, clock_error = None, interrupted
                self._faults += 1
            duration = end - start if start is not None and end is not None and end >= start else None
            self._ledger.finish_call(observed, outcome=reply['outcome'], usage=usage,
                usd_micros=None, billing='unknown', component_ns=duration)
        except BaseException as bookkeeping_error:
            self._faults += 1
            self._blocked = True
            if error is None:
                error = bookkeeping_error
        return error if error is not None else clock_error

    async def dispatch_reader(self, ticket, create, serializer):
        row, unit, observed = self._enter(ticket, 'reader', (create, serializer))
        start = value = outward = error = None
        async def sdk_create(**kwargs):
            detached = self._boundary(row, kwargs)
            response = await create(**detached)
            return self._capture(row, response, serializer)
        try:
            client = SimpleNamespace(chat=SimpleNamespace(completions=SimpleNamespace(create=sdk_create)))
            start = self._started(unit)
            kwargs = json.loads(row['body'])
            outward = await self._reader_helper(client, self._args(), kwargs['messages'])
            if type(outward) is not tuple or len(outward) != 2:
                raise base.ControllerError('upstream_reader_result')
            value = outward[0]  # legacy zero-filled counts NEVER enter the ledger
        except BaseException as caught:
            error = caught
        finally:
            terminal = self._finish(row, unit, observed, start, value, error)
        if terminal is not None:
            raise terminal
        return outward

    def dispatch_judge(self, ticket, factory):
        row, unit, observed = self._enter(ticket, 'judge', (factory,))
        start = value = error = None
        try:
            start = self._started(unit)
            pair = factory()
            if (type(pair) is not tuple or len(pair) != 3
                    or not all(callable(item) for item in pair)):
                raise base.ControllerError('upstream_judge_factory')
            create, serializer, decode_grade = pair
            def sdk_create(**kwargs):
                detached = self._boundary(row, kwargs)
                return self._capture(row, create(**detached), serializer)
            client = SimpleNamespace(chat=SimpleNamespace(completions=SimpleNamespace(create=sdk_create)))
            kwargs = json.loads(row['body'])
            text = self._judge_helper(client=client, model=kwargs['model'], messages=kwargs['messages'],
                max_completion_tokens=kwargs['max_completion_tokens'], reasoning_effort=None,
                temperature=None, top_p=None, timeout_seconds=kwargs['timeout'])
            value = decode_grade(text)  # explicitly local decoder, not official evaluator
        except BaseException as caught:
            error = caught
        finally:
            terminal = self._finish(row, unit, observed, start, value, error)
        if terminal is not None:
            raise terminal
        return value

    def _records(self, phase, *, bodies):
        records = super()._records(phase, bodies=bodies)
        for row in records:
            row['sdk_argument_bytes'] = row.pop('request_bytes')
            row['sdk_create_boundaries'] = row.pop('injected_transport_boundaries')
            row['response_proxy_bytes'] = row.pop('response_bytes')
        return records

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
                    if not issubclass(type(error), Exception):
                        # Teardown is once-only. Preserve the available frozen
                        # local final before original cancellation propagates.
                        self._blocked = True
                        try:
                            final = self._memory.snapshot()
                            self._join_local(final)
                        except BaseException:
                            self._faults += 1
                        try:
                            self._close_units('reader')
                        except BaseException:
                            self._faults += 1
                        raise
                    final = self._memory.snapshot()
                self._join_local(final)
                self._close_units('reader')
                slots = []
                for ordinal, (question, slot) in enumerate(zip(self._questions, self._slots)):
                    reader = self._units[('reader', ordinal)]
                    slots.append({'ordinal': ordinal, 'question': question['question'],
                        'query_outcome': slot['query_outcome'], 'local_ordinal': slot['local_ordinal'],
                        'reader_request': json.loads(slot['reader_body']) if slot['reader_body'] else None,
                        'reader_outcome': reader['outcome'], 'reader_answer': reader['value']})
                body = base._wire({'profile': CONSTRUCTION_PROFILE, 'horizon': len(self._questions),
                                  'slots': slots, 'reader_attempts': self._records('reader', bodies=True)})
                self._hold(len(body))
                self._seal = base.Construction(body)
                return self._seal
            except BaseException:
                self._blocked = True
                raise
            finally:
                self._busy = False

    def private_judge_journal(self):
        with self._lock:
            if not self._closed or self._seal is None:
                raise base.ControllerError('upstream_journal_state')
            if self._judge_journal is None:
                body = base._wire({'profile': JOURNAL_PROFILE,
                    'reader_construction_sha256': self._seal.sha256,
                    'judge_attempts': self._records('judge', bodies=True)})
                self._hold(len(body))
                self._judge_journal = base.Construction(body)
            return self._judge_journal

    def snapshot(self):
        with self._lock:
            result = super().snapshot()
            result['profile'] = PROFILE
            result['sdk_create_boundaries'] = result.pop('injected_transport_boundaries')
            result['request_byte_scope'] = 'canonical_sdk_argument_proxy'
            result['response_byte_scope'] = 'supplied_serialization_proxy'
            return result
