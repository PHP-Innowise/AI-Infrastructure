"""Bounded native boundary evidence; no error text or inferred root causes.

Counts are event occurrences, not distinct incidents. Guard return codes can
collide with native exits, setup errors and translated signals. Neither they
nor provider error events certify an authentication or network cause.
"""
import copy
import threading

LIMIT = 4096
SIGNALS = frozenset((
    'launch_exception', 'stdin_write_failed', 'output_line_limit',
    'output_total_limit', 'time_budget', 'token_budget', 'usd_budget',
    'cancelled', 'shutdown', 'generation_changed', 'missing_terminal',
    'identity_changed', 'wait_timeout', 'cleanup_confirmed',
    'cleanup_unconfirmed', 'empty_read_ended_or_error', 'malformed_line',
    'non_object_line', 'lifecycle_exception', 'recovery_no_draft',
    'recovery_skipped', 'shared_budget', 'guard_exit_before_cleanup', 'guard_exit_after_cleanup',
))
CLAUDE_SUBTYPES = {
    'success': 'success',
    'error_during_execution': 'execution_error',
    'error_max_turns': 'turn_limit',
    'error_max_budget_usd': 'usd_limit',
    'error_max_structured_output_retries': 'structured_output_retry_limit',
}
CLAUDE_REASONS = frozenset(('completed', 'max_turns', 'aborted_streaming', 'aborted_tools'))


class NativeDiagnostics:
    def __init__(self, provider):
        self.provider = provider if provider in ('codex', 'claude', 'cursor') else 'unsupported'
        self.lock = threading.Lock()
        self.final = None
        self.incomplete = self.overflow = False
        self.events = self.errors = self.success = self.failed = self.unknown = self.contradictory = 0
        self.subtypes, self.signals = {}, {}
        self.exits = {}
        self.http_statuses = []
        self.item_errors = 0
        self.reasons = {}

    def _count(self, values, key):
        old = values.get(key, 0)
        if old >= LIMIT:
            self.overflow = True
        else:
            values[key] = old + 1

    def observe(self, event):
        with self.lock:
            if self.final is not None:
                return
            if self.events >= LIMIT:
                self.overflow = True
                return
            self.events += 1
            if type(event) is not dict:
                self.incomplete = True
                return
            kind = event.get('type')
            if kind == 'error':
                self.errors += 1
            if self.provider == 'codex' and kind == 'item.completed':
                item = event.get('item')
                if type(item) is dict and item.get('type') == 'error':
                    self.item_errors += 1
            if self.provider == 'codex' and kind in ('turn.completed', 'turn.failed'):
                flagged = bool(event.get('error')) or event.get('is_error') is True
                if kind == 'turn.failed' or flagged:
                    self.failed += 1
                else:
                    self.success += 1
                self.contradictory += kind == 'turn.completed' and flagged
            elif self.provider == 'claude' and kind == 'result':
                subtype = event.get('subtype')
                bucket = CLAUDE_SUBTYPES.get(subtype, 'unknown') if type(subtype) is str else 'unknown'
                self._count(self.subtypes, bucket)
                flag = event.get('is_error')
                if bucket == 'success' and flag is False:
                    self.success += 1
                elif flag is True or bucket not in ('success', 'unknown'):
                    self.failed += 1
                else:
                    self.unknown += 1
                # Claude API failures can legitimately use subtype=success
                # with is_error=True; the flag identifies the reported failure.
                self.contradictory += bucket not in ('success', 'unknown') and flag is False
                reason = event.get('terminal_reason')
                reason_bucket = reason if type(reason) is str and reason in CLAUDE_REASONS else (
                    'unreported' if 'terminal_reason' not in event else
                    'unknown' if reason is None or type(reason) is str else 'invalid')
                self._count(self.reasons, reason_bucket)
                status = event.get('api_error_status')
                valid = type(status) is int and 100 <= status <= 599
                if len(self.http_statuses) >= 8:
                    self.overflow = True
                else:
                    self.http_statuses.append({'status': status if valid else None,
                        'state': 'observed' if valid else 'unreported' if 'api_error_status' not in event
                        else 'unknown' if status is None else 'invalid'})

    def boundary(self, signal, code=None):
        with self.lock:
            if self.final is not None:
                return
            if type(signal) is not str or signal not in SIGNALS:
                self.incomplete = True
                return
            self._count(self.signals, signal)
            if signal.startswith('guard_exit_'):
                valid = type(code) is int and -(2 ** 31) <= code <= 2 ** 32 - 1
                value = code if valid else None
                if signal not in self.exits:
                    self.exits[signal] = {'code': value, 'state': 'observed' if valid else 'unknown'}
                elif self.exits[signal]['code'] != value or self.exits[signal]['state'] == 'conflict':
                    self.exits[signal] = {'code': None, 'state': 'conflict'}

    def unavailable(self):
        with self.lock:
            if self.final is None:
                self.incomplete = True

    def _snapshot(self):
        return {'schema': 'native-boundary-diagnostics-v1',
                'profile': self.provider if self.provider in ('codex', 'claude') else 'unsupported',
                'events': self.events, 'provider_error_events': self.errors,
                'nonfatal_item_error_events': self.item_errors,
                'successful_terminals': self.success, 'failed_terminals': self.failed,
                'unknown_terminals': self.unknown, 'contradictory_terminals': self.contradictory,
                'reported_subtypes': dict(self.subtypes), 'boundary_signals': dict(self.signals),
                'reported_terminal_reasons': dict(self.reasons),
                'reported_http_statuses': copy.deepcopy(self.http_statuses),
                'guard_exits': copy.deepcopy(self.exits),
                'coverage': 'incomplete' if self.incomplete or self.overflow else 'observed_callbacks_only',
                'overflow': self.overflow, 'root_cause': 'unmeasured', 'stderr': 'not_captured',
                'contract': 'occurrences not incidents; success predicate not task correctness; reasons do not override flags; guard exits not native cause; missing events not zero errors'}

    def snapshot(self):
        with self.lock:
            return copy.deepcopy(self.final) if self.final is not None else self._snapshot()

    def finish(self):
        with self.lock:
            if self.final is None:
                self.final = self._snapshot()
            return copy.deepcopy(self.final)
