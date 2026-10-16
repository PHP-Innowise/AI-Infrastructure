"""NEW Oriole booking observations; no provider, dataset or old ledger replay."""
import copy
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'harness/src'))
from harness import memory_benchmark_cost as subject

P = json.loads((ROOT/'tests/fixtures/memory_cost_replay_public.json').read_text(encoding='utf-8'))


class ClosedBookingReplayTests(unittest.TestCase):
    def prepare(self, *, pending=False, event_limit=65536, conflict=False):
        ledger = subject.Ledger(1, event_limit=event_limit)
        ticket = ledger.begin_call('reader', 0, P['call_id'], P['attempt_id'])
        if not pending:
            ledger.finish_call(ticket, **self.report())
            if conflict:
                changed = self.report()
                changed['outcome'] = 'cancelled'
                ledger.finish_call(ticket, **changed)
        for phase, ordinal, work, duration, size in P['units']:
            ledger.close_unit(phase, ordinal, coverage='complete', model_work=work,
                              billing_complete=True, local_ns=duration, constructed_bytes=size)
        return ledger, ticket

    def report(self):
        return {'outcome': 'succeeded', 'usage': copy.deepcopy(P['usage']),
                'usd_micros': P['usd_micros'], 'billing': 'provider_reported', 'component_ns': P['attempt_ns']}

    def replay(self, ledger):
        try:
            return ledger.begin_call('reader', 0, P['call_id'], P['attempt_id'])
        except subject.LedgerError as error:
            self.fail('Exact existing booking must replay before finalfreeze: '+str(error))

    def expected_snapshot(self):
        product = copy.deepcopy(P['product'])
        experiment = copy.deepcopy(product)
        complete_names = list(product['complete'])
        experiment['complete'] = {key: False for key in complete_names}
        full_names = ['input_tokens', 'output_tokens', 'tokens', 'cached_input_tokens',
                      'reasoning_output_tokens', 'uncached_input_tokens', 'usd_micros',
                      'component_ns', 'constructed_bytes', 'tokens_per_planned_query',
                      'usd_micros_per_planned_query', 'tokens_per_correct', 'usd_micros_per_correct']
        for key in full_names:
            experiment[key] = None
        phases = {}
        for phase, (attempts, finished, duration, size) in P['expected_phase_work'].items():
            metric = {k: copy.deepcopy(v) for k, v in product.items() if '_per_' not in k}
            if phase != 'reader':
                for key in metric:
                    if key not in ('complete', 'overflow'):
                        metric[key] = 0
            metric['known_component_ns'] = duration
            metric['known_constructed_bytes'] = size
            metric['component_ns'] = duration
            metric['constructed_bytes'] = size
            if phase == 'judge':
                metric['complete'] = {key: False for key in complete_names}
                for key in full_names:
                    if key in metric:
                        metric[key] = None
            phases[phase] = {'expected_units': 1, 'closed_units': 0 if phase == 'judge' else 1,
                             'attempts': attempts, 'finished_attempts': finished, 'conflicting_attempts': 0,
                             'outcomes': {'succeeded': finished, 'failed': 0, 'cancelled': 0, 'pending': 0, 'conflict': 0},
                             **metric}
        return {'profile': 'memory-benchmark-ledger-v1', 'usage_profile': 'inclusive-input-output-v1',
                'horizon': 1, 'attempt_limit': 4096, 'attempts': 1, 'duplicates': 0,
                'observation_errors': 0, 'overflow': False, 'frozen': False, 'phases': phases,
                'grading': {'observed': 0, 'known_correct': 0, 'correct': None, 'complete': False, 'conflicting': 0},
                'product': product, 'experiment': experiment}

    def unchanged_except_duplicates(self, before, after, count=1):
        expected = copy.deepcopy(before)
        expected['duplicates'] += count
        self.assertEqual(expected, after)

    def test_closed_exact_booking_preserves_full_arithmetic_snapshot(self):
        ledger, ticket = self.prepare()
        before = ledger.snapshot()
        self.assertEqual(self.expected_snapshot(), before)
        self.assertIs(ticket, self.replay(ledger))
        self.unchanged_except_duplicates(before, ledger.snapshot())

    def test_closed_pending_booking_is_known_but_never_becomes_finished(self):
        ledger, ticket = self.prepare(pending=True)
        before = ledger.snapshot()
        self.assertIsNone(before['product']['tokens'])
        self.assertEqual(0, before['phases']['reader']['finished_attempts'])
        self.assertIs(ticket, self.replay(ledger))
        self.unchanged_except_duplicates(before, ledger.snapshot())
        with self.assertRaisesRegex(subject.LedgerError, '^late_report$'):
            ledger.finish_call(ticket, **self.report())
        self.assertIsNone(ledger.snapshot()['product']['tokens'])

    def test_new_retry_after_close_remains_refused(self):
        ledger, _ = self.prepare()
        with self.assertRaisesRegex(subject.LedgerError, '^unit_closed$'):
            ledger.begin_call('reader', 0, P['call_id'], P['new_attempt_id'])
        snapshot = ledger.snapshot()
        self.assertEqual((1, 0, 1), (snapshot['attempts'], snapshot['duplicates'], snapshot['observation_errors']))
        self.assertEqual(28, snapshot['product']['known_tokens'])
        self.assertIsNone(snapshot['product']['tokens'])

    def test_closed_cross_scope_reuse_marks_original_conflict(self):
        ledger, _ = self.prepare()
        with self.assertRaisesRegex(subject.LedgerError, '^attempt_scope_conflict$'):
            ledger.begin_call('query', 0, P['call_id'], P['attempt_id'])
        snapshot = ledger.snapshot()
        self.assertEqual(1, snapshot['attempts'])
        self.assertEqual(1, snapshot['phases']['reader']['conflicting_attempts'])
        self.assertEqual(0, snapshot['product']['known_tokens'])
        self.assertIsNone(snapshot['product']['tokens'])

    def test_mutated_existing_ticket_is_never_repaired_or_returned(self):
        for field, value in [('_owner', object()), ('_key', b'x'*32), ('_key', None)]:
            with self.subTest(field=field, value_type=type(value).__name__):
                ledger, ticket = self.prepare()
                setattr(ticket, field, value)
                with self.assertRaisesRegex(subject.LedgerError, '^ticket_binding$'):
                    ledger.begin_call('reader', 0, P['call_id'], P['attempt_id'])
                self.assertIs(value, getattr(ticket, field))
                snapshot = ledger.snapshot()
                self.assertEqual((1, 0, 1), (snapshot['attempts'], snapshot['duplicates'], snapshot['observation_errors']))
                self.assertIsNone(snapshot['product']['tokens'])

    def test_final_freeze_precedes_even_exact_replay(self):
        ledger, _ = self.prepare()
        before = ledger.finalize()
        with self.assertRaisesRegex(subject.LedgerError, '^ledger_frozen$'):
            ledger.begin_call('reader', 0, P['call_id'], P['attempt_id'])
        self.assertEqual(before, ledger.snapshot())

    def test_exact_replay_cannot_bypass_event_budget(self):
        ledger, _ = self.prepare(event_limit=5)
        before = ledger.snapshot()
        self.assertEqual(28, before['product']['tokens'])
        with self.assertRaisesRegex(subject.LedgerError, '^event_capacity$'):
            ledger.begin_call('reader', 0, P['call_id'], P['attempt_id'])
        snapshot = ledger.snapshot()
        self.assertTrue(snapshot['overflow'])
        self.assertEqual(0, snapshot['duplicates'])
        self.assertIsNone(snapshot['product']['tokens'])

    def test_known_receipt_after_closed_booking_counts_once(self):
        ledger, ticket = self.prepare()
        before = ledger.snapshot()
        self.assertIs(ticket, self.replay(ledger))
        ledger.finish_call(ticket, **self.report())
        self.unchanged_except_duplicates(before, ledger.snapshot(), 2)

    def test_exact_booking_does_not_heal_conflicted_report(self):
        ledger, ticket = self.prepare(conflict=True)
        before = ledger.snapshot()
        self.assertIsNone(before['product']['tokens'])
        self.assertEqual(0, before['product']['known_tokens'])
        self.assertIs(ticket, self.replay(ledger))
        self.unchanged_except_duplicates(before, ledger.snapshot())

    def test_concurrent_exact_closed_bookings_keep_one_attempt(self):
        ledger, ticket = self.prepare()
        before = ledger.snapshot()
        with ThreadPoolExecutor(max_workers=7) as pool:
            tickets = list(pool.map(lambda _: self.replay(ledger), range(7)))
        self.assertTrue(all(value is ticket for value in tickets))
        self.unchanged_except_duplicates(before, ledger.snapshot(), 7)

    def test_foreign_ticket_class_is_rejected_before_attribute_dispatch(self):
        ledger, ticket = self.prepare()
        touched = []
        class ForeignTicket:
            __slots__ = ('_owner', '_key')
            def __getattribute__(inner, name):
                if name == '_key':
                    touched.append(name)
                    raise RuntimeError('Oriole supplied hostile getter')
                return object.__getattribute__(inner, name)
        ticket.__class__ = ForeignTicket
        self.assertIs(type(ticket), ForeignTicket)
        try:
            ledger.begin_call('reader', 0, P['call_id'], P['attempt_id'])
        except subject.LedgerError as error:
            self.assertEqual('ticket_binding', str(error))
        except Exception as error:
            self.fail('Foreign ticket dispatched an attribute before type admission: '+type(error).__name__)
        else:
            self.fail('Foreign ticket was accepted')
        self.assertEqual([], touched)
        snapshot = ledger.snapshot()
        self.assertEqual((1, 0, 1), (snapshot['attempts'], snapshot['duplicates'], snapshot['observation_errors']))
        self.assertIsNone(snapshot['product']['tokens'])


if __name__ == '__main__':
    unittest.main()
