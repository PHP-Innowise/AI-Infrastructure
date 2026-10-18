"""Bounded supplied-observation memory benchmark cost ledger (stdlib only).

The caller binds unique physical attempts and observes hidden work. This module
does not verify a provider/invoice, authorize work or convert raw provider events.
Input includes cache and output includes reasoning in this explicit profile.
Integer USD micros and summed component ns are not prices or wall-clock latency.
"""
from __future__ import annotations

import copy
from dataclasses import dataclass
import hashlib
import hmac
import json
import secrets
import threading

from .cost_observation import COUNTER_LIMIT, counter

PROFILE = 'memory-benchmark-ledger-v1'
USAGE_PROFILE = 'inclusive-input-output-v1'
PHASES = ('build', 'query', 'reader', 'judge')
MAX_HORIZON = 512
MAX_ATTEMPTS = 4096
MAX_EVENTS = 65536
SNAPSHOT_BYTES = 262144
USAGE_KEYS = ('input_tokens', 'output_tokens', 'total_tokens',
              'cached_input_tokens', 'reasoning_output_tokens')
OUTCOMES = ('succeeded', 'failed', 'cancelled')


class LedgerError(ValueError):
    """Fixed code only, without supplied IDs, values or exception prose."""


class AttemptTicket:
    __slots__ = ('_owner', '_key')

    def __init__(self, owner, key):
        self._owner, self._key = owner, key

    def __repr__(self):
        return '<opaque memory cost ticket>'


@dataclass(frozen=True)
class _Report:
    outcome: str
    usage: tuple
    usage_valid: bool
    usd_micros: object
    billing: str
    component_ns: object


def _usage(value):
    if value is None:
        return (None,) * 5, True
    if (type(value) is not dict or len(value) > 5
            or any(type(key) is not str or key not in USAGE_KEYS for key in value)):
        return (None,) * 5, False
    fields = tuple(value.get(key) for key in USAGE_KEYS)
    if any(item is not None and counter(item) is None for item in fields):
        return (None,) * 5, False
    inp, out, total, cached, reasoning = fields
    if total is not None and any(item is not None and item > total for item in (inp, out)):
        return (None,) * 5, False
    if inp is not None and out is not None:
        if inp + out > COUNTER_LIMIT or (total is not None and total != inp + out):
            return (None,) * 5, False
    if cached is not None and (inp is None or cached > inp):
        return (None,) * 5, False
    if reasoning is not None and (out is None or reasoning > out):
        return (None,) * 5, False
    return fields, True


class Ledger:
    def __init__(self, horizon, *, attempt_limit=MAX_ATTEMPTS, event_limit=MAX_EVENTS):
        if (type(horizon) is not int or not 1 <= horizon <= MAX_HORIZON
                or type(attempt_limit) is not int or not 1 <= attempt_limit <= MAX_ATTEMPTS
                or type(event_limit) is not int or not 1 <= event_limit <= MAX_EVENTS):
            raise LedgerError('ledger_bounds')
        self._horizon, self._attempt_limit, self._event_limit = horizon, attempt_limit, event_limit
        self._key, self._owner = secrets.token_bytes(32), object()
        self._lock = threading.RLock()
        self._records = {}
        self._units = {(phase, ordinal): {'keys': [], 'close': None, 'conflict': False, 'close_conflict': False}
                       for phase in PHASES for ordinal in ([None] if phase == 'build' else range(horizon))}
        self._grades, self._grade_conflicts = {}, set()
        self._events = self._duplicates = self._errors = 0
        self._overflow = self._invalid = self._frozen = False

    @property
    def horizon(self):
        return self._horizon

    @property
    def attempt_limit(self):
        return self._attempt_limit

    def _error(self, code):
        self._errors = min(self._errors + 1, MAX_EVENTS)
        self._invalid = True
        raise LedgerError(code)

    def _tick(self):
        if self._frozen:
            raise LedgerError('ledger_frozen')
        if self._events >= self._event_limit:
            self._overflow = True
            self._error('event_capacity')
        self._events += 1

    def _scope(self, phase, ordinal):
        if type(phase) is not str or phase not in PHASES:
            self._error('phase_schema')
        if ((phase == 'build' and ordinal is not None)
                or (phase != 'build' and (type(ordinal) is not int or not 0 <= ordinal < self.horizon))):
            self._error('ordinal_schema')
        return phase, ordinal

    def _identifier(self, value):
        if type(value) is not str or not 1 <= len(value) <= 256:
            self._error('identifier_schema')
        try:
            raw = value.encode('utf-8')
        except UnicodeError:
            self._error('identifier_schema')
        if any(ord(char) < 32 or ord(char) == 127 for char in value):
            self._error('identifier_schema')
        return raw

    def begin_call(self, phase, ordinal, call_id, attempt_id):
        """Book observation identity; this ticket never authorizes execution."""
        with self._lock:
            self._tick()
            scope = self._scope(phase, ordinal)
            unit = self._units[scope]
            call, attempt = self._identifier(call_id), self._identifier(attempt_id)
            # A supplied physical identity binds one scope, rather than being
            # counted again when an observer accidentally assigns another scope.
            message = len(call).to_bytes(4, 'big') + call + len(attempt).to_bytes(4, 'big') + attempt
            key = hmac.new(self._key, message, hashlib.sha256).digest()
            if key in self._records:
                existing = self._records[key]
                if existing['scope'] != scope:
                    existing['conflict'] = True
                    self._units[existing['scope']]['conflict'] = unit['conflict'] = True
                    self._error('attempt_scope_conflict')
                ticket = existing['ticket']
                if type(ticket) is not AttemptTicket:
                    self._error('ticket_binding')
                ticket_key = getattr(ticket, '_key', None)
                if (getattr(ticket, '_owner', None) is not self._owner
                        or type(ticket_key) is not bytes or len(ticket_key) != 32 or ticket_key != key):
                    self._error('ticket_binding')
                self._duplicates += 1
                return ticket
            if unit['close'] is not None:
                self._error('unit_closed')
            if len(self._records) >= self.attempt_limit:
                self._overflow = True
                self._error('attempt_capacity')
            ticket = AttemptTicket(self._owner, key)
            self._records[key] = {'ticket': ticket, 'scope': scope, 'report': None, 'conflict': False}
            unit['keys'].append(key)
            return ticket

    def finish_call(self, ticket, *, outcome, usage=None, usd_micros=None,
                    billing='unknown', component_ns=None):
        with self._lock:
            self._tick()
            if type(ticket) is not AttemptTicket:
                self._error('ticket_binding')
            owner, key = getattr(ticket, '_owner', None), getattr(ticket, '_key', None)
            if owner is not self._owner or type(key) is not bytes or len(key) != 32:
                self._error('ticket_binding')
            record = self._records.get(key)
            if record is None or record['ticket'] is not ticket:
                self._error('ticket_binding')
            if type(outcome) is not str or outcome not in OUTCOMES:
                record['conflict'] = True
                self._error('outcome_schema')
            fields, valid = _usage(usage)
            if not valid:
                self._errors = min(self._errors + 1, MAX_EVENTS)
            if usd_micros is not None and counter(usd_micros) is None:
                usd_micros = None
                self._errors = min(self._errors + 1, MAX_EVENTS)
            if type(billing) is not str or billing not in ('unknown', 'provider_reported', 'invoice_allocated'):
                billing = 'unknown'
                self._errors = min(self._errors + 1, MAX_EVENTS)
            if component_ns is not None and counter(component_ns) is None:
                component_ns = None
                self._errors = min(self._errors + 1, MAX_EVENTS)
            report = _Report(outcome, fields, valid, usd_micros, billing, component_ns)
            old = record['report']
            unit = self._units[record['scope']]
            if old is not None:
                if old == report and not record['conflict']:
                    self._duplicates += 1
                else:
                    record['conflict'] = True
                    if unit['close'] is not None:
                        unit['conflict'] = True
                        self._error('late_report')
                return
            if unit['close'] is not None:
                record['conflict'] = unit['conflict'] = True
                self._error('late_report')
            record['report'] = report

    def close_unit(self, phase, ordinal, *, coverage, model_work,
                   billing_complete=False, local_ns=None, constructed_bytes=None):
        with self._lock:
            self._tick()
            scope = self._scope(phase, ordinal)
            unit = self._units[scope]
            if (type(coverage) is not str or coverage not in ('complete', 'incomplete')
                    or type(model_work) is not str or model_work not in ('reported_calls', 'none_observed_local', 'unknown')
                    or type(billing_complete) is not bool
                    or (local_ns is not None and counter(local_ns) is None)
                    or (constructed_bytes is not None and counter(constructed_bytes) is None)):
                unit['conflict'] = unit['close_conflict'] = True
                self._error('unit_schema')
            close = (coverage, model_work, billing_complete, local_ns, constructed_bytes)
            if unit['close'] is not None:
                if unit['close'] == close:
                    self._duplicates += 1
                else:
                    unit['conflict'] = unit['close_conflict'] = True
                return
            if (model_work == 'none_observed_local' and (phase not in ('build', 'query') or unit['keys'])):
                unit['conflict'] = True
                self._error('no_model_conflict')
            if model_work == 'reported_calls' and not unit['keys']:
                unit['conflict'] = True
                self._error('missing_reported_calls')
            unit['close'] = close

    def grade(self, ordinal, correct):
        """One final evaluator result per fixed ordinal; null remains ungraded."""
        with self._lock:
            self._tick()
            self._scope('judge', ordinal)
            if correct is not None and type(correct) is not bool:
                self._error('grade_schema')
            if ordinal in self._grades:
                if self._grades[ordinal] == correct and ordinal not in self._grade_conflicts:
                    self._duplicates += 1
                else:
                    self._grade_conflicts.add(ordinal)
            else:
                self._grades[ordinal] = correct

    def _metrics(self, phases):
        units = [unit for (phase, _), unit in self._units.items() if phase in phases]
        records = [self._records[key] for unit in units for key in unit['keys']]
        reports = [record['report'] for record in records
                   if record['report'] is not None and not record['conflict']]
        coverage = not (self._overflow or self._invalid)
        for unit in units:
            close = unit['close']
            coverage &= bool(close is not None and close[0] == 'complete'
                             and close[1] != 'unknown' and not unit['conflict'])
        coverage &= all(record['report'] is not None and not record['conflict'] for record in records)
        usable = [report for report in reports if report.usage_valid]
        paired = [report for report in usable if report.usage[0] is not None and report.usage[1] is not None]
        known = {
            'known_input_tokens': sum(report.usage[0] or 0 for report in usable),
            'known_output_tokens': sum(report.usage[1] or 0 for report in usable),
            'known_tokens': sum(report.usage[0] + report.usage[1] for report in paired),
            'known_usd_micros': sum(report.usd_micros or 0 for report in reports),
            'known_component_ns': sum(report.component_ns or 0 for report in reports)
                                  + sum(unit['close'][3] or 0 for unit in units
                                        if unit['close'] is not None and not unit['close_conflict']),
            'known_constructed_bytes': sum(unit['close'][4] or 0 for unit in units
                                           if unit['close'] is not None and not unit['close_conflict']),
        }
        overflow = any(value > COUNTER_LIMIT for value in known.values())
        known = {key: value if value <= COUNTER_LIMIT else None for key, value in known.items()}
        tokens = coverage and len(paired) == len(records)
        cached = sum(report.usage[3] or 0 for report in paired)
        reasoning = sum(report.usage[4] or 0 for report in paired)
        overflow |= cached > COUNTER_LIMIT or reasoning > COUNTER_LIMIT
        complete = {
            'coverage': coverage,
            'tokens': tokens,
            'cache': tokens and all(report.usage[3] is not None for report in paired),
            'reasoning': tokens and all(report.usage[4] is not None for report in paired),
            'billing': coverage and all(unit['close'][2] for unit in units)
                       and all(report.usd_micros is not None and report.billing != 'unknown' for report in reports),
            'timing': coverage and all(unit['close'][3] is not None for unit in units)
                      and all(report.component_ns is not None for report in reports),
            'bytes': coverage and all(unit['close'][4] is not None for unit in units),
        }
        if overflow:
            complete = {key: False for key in complete}
        return {**known,
                'input_tokens': known['known_input_tokens'] if complete['tokens'] else None,
                'output_tokens': known['known_output_tokens'] if complete['tokens'] else None,
                'tokens': known['known_tokens'] if complete['tokens'] else None,
                'cached_input_tokens': cached if complete['cache'] else None,
                'reasoning_output_tokens': reasoning if complete['reasoning'] else None,
                'uncached_input_tokens': known['known_input_tokens'] - cached if complete['cache'] else None,
                'usd_micros': known['known_usd_micros'] if complete['billing'] else None,
                'component_ns': known['known_component_ns'] if complete['timing'] else None,
                'constructed_bytes': known['known_constructed_bytes'] if complete['bytes'] else None,
                'complete': complete, 'overflow': overflow}

    def _snapshot(self):
        phases = {}
        for phase in PHASES:
            units = [unit for (name, _), unit in self._units.items() if name == phase]
            records = [self._records[key] for unit in units for key in unit['keys']]
            outcomes = {key: 0 for key in (*OUTCOMES, 'pending', 'conflict')}
            for record in records:
                kind = 'conflict' if record['conflict'] else record['report'].outcome if record['report'] else 'pending'
                outcomes[kind] += 1
            phases[phase] = {'expected_units': len(units),
                             'closed_units': sum(unit['close'] is not None for unit in units),
                             'attempts': len(records),
                             'finished_attempts': sum(record['report'] is not None for record in records),
                             'conflicting_attempts': sum(record['conflict'] for record in records),
                             'outcomes': outcomes, **self._metrics((phase,))}
        observed = [correct for ordinal, correct in self._grades.items()
                    if correct is not None and ordinal not in self._grade_conflicts]
        grade_complete = len(observed) == self.horizon
        correct = sum(observed) if grade_complete else None
        grading = {'observed': len(observed), 'known_correct': sum(observed),
                   'correct': correct, 'complete': grade_complete, 'conflicting': len(self._grade_conflicts)}
        scopes = {}
        for name, included in [('product', PHASES[:3]), ('experiment', PHASES)]:
            metric = self._metrics(included)
            scopes[name] = {**metric,
                            'tokens_per_planned_query': metric['tokens'] / self.horizon if metric['tokens'] is not None else None,
                            'usd_micros_per_planned_query': metric['usd_micros'] / self.horizon if metric['usd_micros'] is not None else None,
                            'tokens_per_correct': metric['tokens'] / correct if correct and metric['tokens'] is not None else None,
                            'usd_micros_per_correct': metric['usd_micros'] / correct if correct and metric['usd_micros'] is not None else None}
        result = {'profile': PROFILE, 'usage_profile': USAGE_PROFILE,
                  'horizon': self.horizon, 'attempt_limit': self.attempt_limit,
                  'attempts': len(self._records), 'duplicates': self._duplicates,
                  'observation_errors': self._errors,
                  'overflow': self._overflow or any(item['overflow'] for item in [*phases.values(), *scopes.values()]),
                  'frozen': self._frozen, 'phases': phases, 'grading': grading, **scopes}
        if len(json.dumps(result, separators=(',', ':'), allow_nan=False).encode()) > SNAPSHOT_BYTES:
            raise LedgerError('snapshot_capacity')
        return result

    def snapshot(self):
        with self._lock:
            return copy.deepcopy(self._snapshot())

    def finalize(self):
        with self._lock:
            self._frozen = True
            return copy.deepcopy(self._snapshot())
