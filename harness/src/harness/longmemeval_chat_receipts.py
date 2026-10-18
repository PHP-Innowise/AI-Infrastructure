"""Owned injected Chat arguments and supplied usage; no SDK/network dispatch.

Separate opt-in benchmark profile. A trusted synchronous transport callable may
hide arbitrary work; its boundary is not physical provider coverage. Production
memory and the original callback controller are unchanged.
"""
from __future__ import annotations

import copy
import json

from . import longmemeval_lifecycle as base
from .cost_observation import counter
from .longmemeval_projection import _text
from .longmemeval_provider_usage import CompletionCapture, PROFILE as USAGE_PROFILE
from .memory_benchmark_cost import _usage, USAGE_KEYS

PROFILE = 'longmemeval/injected-chat-controller-v1'
CONSTRUCTION_PROFILE = 'longmemeval/injected-chat-reader-construction-v1'
JOURNAL_PROFILE = 'longmemeval/injected-chat-judge-journal-v1'
STATES = ('unobserved', 'missing', 'partial', 'complete', 'invalid', 'unmeasured', 'unsupported')


class ChatController(base.Controller):
    def __init__(self, memory, questions, *, reader_model, judge_model,
                 reader_max_completion_tokens, judge_max_completion_tokens,
                 reader_budget=4096, clock=None):
        for name in (reader_model, judge_model):
            _text(name, 128, nonempty=True)
        for value in (reader_max_completion_tokens, judge_max_completion_tokens):
            if type(value) is not int or not 1 <= value <= base.MAX_REQUEST:
                raise base.ControllerError('chat_parameters')
        self._chat_profiles = {'reader': (reader_model, reader_max_completion_tokens),
                               'judge': (judge_model, judge_max_completion_tokens)}
        self._chat_rows = []
        self._judge_journal = None
        super().__init__(memory, questions, reader_budget=reader_budget, clock=clock)

    def _chat(self, local_body, phase):
        value = json.loads(local_body)
        value.pop('system', None)
        model, maximum = self._chat_profiles[phase]
        return base._wire({'model': model,
            'messages': [{'role': 'system', 'content': base.SYSTEM},
                         {'role': 'user', 'content': base._wire(value).decode('utf-8')}],
            'max_completion_tokens': maximum, 'n': 1, 'stream': False})

    def _issue(self, phase, ordinal, local_body):
        unit = self._units[(phase, ordinal)]
        if unit['issued'] >= 2 or unit['outcome'] in ('pending', 'succeeded', 'cancelled'):
            raise base.ControllerError('controller_attempt_budget')
        body = self._chat(local_body, phase)
        if len(body) > (self._budget if phase == 'reader' else base.MAX_REQUEST):
            raise base.ControllerError('chat_request_bound')
        try:
            capture = CompletionCapture()
        except Exception:
            capture = None
            self._faults += 1
        # Cancellation here precedes holding/issuing any final ticket.
        held_before, issued_before, outcome_before = self._held, unit['issued'], unit['outcome']
        old_tickets, old_rows = set(self._tickets), len(self._chat_rows)
        try:
            self._hold(len(body) + (len(local_body) if phase == 'judge' else 0))
            ticket = super()._issue(phase, ordinal, body)
            row = self._tickets[ticket]
            row.update(reservation=unit['issued'], capture=capture, capture_read=False,
                       local_body=local_body, injected_transport_boundaries=0, request_bytes=0,
                       response_bytes=None, usage_state='unobserved',
                       usage=dict.fromkeys(USAGE_KEYS), outcome='unlaunched')
            self._chat_rows.append(row)
            return ticket
        except BaseException:
            self._held = held_before
            unit['issued'], unit['outcome'] = issued_before, outcome_before
            for item in list(self._tickets):
                if item not in old_tickets:
                    del self._tickets[item]
            del self._chat_rows[old_rows:]
            raise

    def prepare_reader(self, ordinal):
        with self._lock:
            self._ready()
            self._ordinal(ordinal)
            if self._seal is not None or self._slots[ordinal]['query_outcome'] != 'succeeded':
                raise base.ControllerError('controller_reader_state')
            unit = self._units[('reader', ordinal)]
            if unit['issued'] >= 2 or unit['outcome'] in ('pending', 'succeeded', 'cancelled'):
                raise base.ControllerError('controller_attempt_budget')
            slot = self._slots[ordinal]
            if slot['reader_body'] is None:
                value = {'system': base.SYSTEM, 'question': self._questions[ordinal]['question'],
                         'memory_context': slot['context']}
                local_body = base._wire(value)
                if len(self._chat(local_body, 'reader')) > self._budget:
                    value['memory_context'] = []
                    local_body = base._wire(value)
                if len(self._chat(local_body, 'reader')) > self._budget:
                    raise base.ControllerError('chat_request_bound')
                self._hold(len(local_body))
                slot['reader_body'] = local_body
            return self._issue('reader', ordinal, slot['reader_body'])

    def prepare_judge(self, ordinal, construction):
        with self._lock:
            self._ready()
            self._ordinal(ordinal)
            if self._seal is None or construction is not self._seal:
                raise base.ControllerError('controller_construction_binding')
            answer = self._units[('reader', ordinal)]
            if answer['outcome'] != 'succeeded':
                raise base.ControllerError('controller_judge_state')
            body = base._wire({'question': self._questions[ordinal]['question'],
                              'answer': answer['value'], 'construction_sha256': self._seal.sha256})
            if len(body) > base.MAX_REQUEST:
                raise base.ControllerError('controller_request_bound')
            # _issue owns both reservations; no untracked local hold precedes it.
            return self._issue('judge', ordinal, body)

    def _capture_usage(self, row, interruptions):
        if row['capture_read']:
            return copy.deepcopy(row['usage'])
        row['capture_read'] = True
        try:
            if row['capture'] is None:
                raise base.ControllerError('chat_capture_unavailable')
            snap = row['capture'].snapshot()
            base._keys(snap, ('consumed', 'decoder_state', 'observation'))
            obs = snap['observation']
            base._keys(obs, ('profile', 'response_bytes', 'usage_state', 'usage', 'usage_scope',
                            'provider_coverage', 'physical_attempts', 'usd_micros', 'billing', 'usage_verified'))
            if (type(obs['profile']) is not str or obs['profile'] != USAGE_PROFILE
                    or type(obs['usage_state']) is not str or obs['usage_state'] not in STATES
                    or obs['response_bytes'] is not None and counter(obs['response_bytes']) is None
                    or type(obs['provider_coverage']) is not str or obs['provider_coverage'] != 'UNKNOWN'
                    or obs['physical_attempts'] is not None or obs['usd_micros'] is not None
                    or type(obs['billing']) is not str or obs['billing'] != 'unknown'
                    or obs['usage_verified'] is not False):
                raise base.ControllerError('chat_capture_schema')
            values, valid = _usage(obs['usage'])
            if not valid:
                raise base.ControllerError('chat_capture_schema')
            row['usage'] = dict(zip(USAGE_KEYS, values))
            row['usage_state'] = obs['usage_state']
        except BaseException as error:
            self._faults += 1
            row['usage'] = dict.fromkeys(USAGE_KEYS)
            row['usage_state'] = 'unmeasured'
            if not issubclass(type(error), Exception):
                interruptions.append(error)
        return copy.deepcopy(row['usage'])

    def dispatch(self, ticket, transport, decoder=None):
        """Reader uses (transport,decoder); judge uses a post-gate pair factory.

        Only internally generated reports reach the base controller. No external
        sink, scope, normalized usage, USD or request-body override is accepted.
        """
        with self._lock:
            self._ready()
            if type(ticket) is not base.DispatchTicket or ticket not in self._tickets:
                raise base.ControllerError('controller_ticket_binding')
            row = self._tickets[ticket]
            phase, ordinal = row['phase'], row['ordinal']
            if (row['consumed'] or phase == 'reader' and self._seal is not None
                    or not callable(transport) or phase == 'reader' and not callable(decoder)
                    or phase == 'judge' and decoder is not None):
                raise base.ControllerError('chat_dispatch_binding')
            unit = self._units[(phase, ordinal)]
            entries_before = unit['entries']
            task_error = None
            interruptions = []

            def report(outcome, value):
                usage = self._capture_usage(row, interruptions)
                if interruptions and outcome == 'succeeded':
                    outcome, value = 'cancelled', None
                return base._report({'outcome': outcome, 'value': value,
                    'usage': usage, 'usd_micros': None,
                    'billing': 'unknown'}, phase)

            def failed(error):
                nonlocal task_error
                task_error = error
                outcome = 'failed' if issubclass(type(error), Exception) else 'cancelled'
                return report(outcome, None)

            def invoke(body, send, decode):
                try:
                    if body is not row['body'] or not callable(send) or not callable(decode):
                        raise base.ControllerError('chat_argument_binding')
                    row['injected_transport_boundaries'] = 1
                    row['request_bytes'] = len(body)
                    response = send(body)
                    if type(response) is not bytes:
                        raise base.ControllerError('chat_response_profile')
                    row['response_bytes'] = len(response)
                    value = (row['capture'].deliver(response, decode)
                             if row['capture'] is not None else decode(response))
                    # Validate before base._report can discard the paid capture.
                    return report('succeeded', value)
                except BaseException as error:
                    return failed(error)

            def reader(body):
                return invoke(body, transport, decoder)

            def judge_factory():
                try:
                    pair = transport()
                    if (type(pair) is not tuple or len(pair) != 2
                            or not all(callable(item) for item in pair)):
                        raise base.ControllerError('chat_factory_profile')
                    return lambda body: invoke(body, pair[0], pair[1])
                except BaseException as error:
                    failure = failed(error)
                    return lambda _: failure

            try:
                result = super().dispatch(ticket, judge_factory if phase == 'judge' else reader)
            except BaseException:
                if task_error is not None:
                    raise task_error from None
                if interruptions:
                    raise interruptions[0] from None
                raise
            finally:
                # Base includes pre-factory callback bytes. This lane charges
                # final argument bytes only at the injected transport boundary.
                if unit['entries'] > entries_before and not row['injected_transport_boundaries']:
                    unit['bytes'] -= len(row['body'])
                row['outcome'] = unit['outcome']
            if task_error is not None:
                raise task_error
            if interruptions:
                raise interruptions[0]
            return result

    def _records(self, phase, *, bodies):
        result = []
        for row in self._chat_rows:
            if row['phase'] != phase:
                continue
            item = {'ordinal': row['ordinal'], 'reservation': row['reservation'],
                    'prepared_request_bytes': len(row['body']), 'request_bytes': row['request_bytes'],
                    'injected_transport_boundaries': row['injected_transport_boundaries'],
                    'response_bytes': row['response_bytes'], 'usage_state': row['usage_state'],
                    'usage': copy.deepcopy(row['usage']), 'outcome': row['outcome']}
            if bodies:
                item['request'] = json.loads(row['body'])
            result.append(item)
        return result

    def seal_construction(self):
        """One final reader gate; no temporary base gate is issued."""
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
                    unit = self._units[('reader', i)]
                    slots.append({'ordinal': i, 'question': question['question'],
                        'query_outcome': slot['query_outcome'], 'local_ordinal': slot['local_ordinal'],
                        'reader_request': json.loads(slot['reader_body']) if slot['reader_body'] else None,
                        'reader_outcome': unit['outcome'], 'reader_answer': unit['value']})
                body = base._wire({'profile': CONSTRUCTION_PROFILE, 'horizon': len(self._questions),
                                  'slots': slots, 'reader_attempts': self._records('reader', bodies=True)})
                if len(body) > base.MAX_HELD:
                    raise base.ControllerError('chat_construction_bound')
                self._hold(len(body))
                self._seal = base.Construction(body)
                return self._seal
            except BaseException:
                self._blocked = True
                raise
            finally:
                self._busy = False

    def private_judge_journal(self):
        """Separate post-gate artifact; does not mutate the frozen cost ledger."""
        with self._lock:
            if not self._closed or self._seal is None:
                raise base.ControllerError('chat_journal_state')
            if self._judge_journal is None:
                body = base._wire({'profile': JOURNAL_PROFILE,
                    'reader_construction_sha256': self._seal.sha256,
                    'judge_attempts': self._records('judge', bodies=True)})
                if len(body) > base.MAX_HELD:
                    raise base.ControllerError('chat_journal_bound')
                self._hold(len(body))
                self._judge_journal = base.Construction(body)
            return self._judge_journal

    def snapshot(self):
        with self._lock:
            result = super().snapshot()
            result['profile'] = PROFILE
            result['physical_attempts'] = None
            result['injected_transport_boundaries'] = sum(row['injected_transport_boundaries'] for row in self._chat_rows)
            result['reader_attempts'] = self._records('reader', bodies=False)
            result['judge_attempts'] = self._records('judge', bodies=False)
            return result
