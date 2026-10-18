"""Fresh supplied-observation cost controls; no provider/dataset/closed suite."""
import copy
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import memory_benchmark_cost as subject

PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_cost_public.json').read_text())
EXPECTED = json.loads((ROOT / 'tests/fixtures/memory_cost_expected.json').read_text())


def run_case(case, *, grades=True, missing_judge=False):
    ledger = subject.Ledger(case['horizon'])
    for row in case['calls']:
        ticket = ledger.begin_call(row['phase'], row['ordinal'], row['call_id'], row['attempt_id'])
        if row['pending']:
            continue
        report = {key: row[key] for key in ('outcome', 'usage', 'usd_micros', 'billing', 'component_ns')}
        if missing_judge and row['phase'] == 'judge':
            report['usage'] = None
        ledger.finish_call(ticket, **report)
        if row['duplicate']:
            ledger.finish_call(ticket, **report)
        if row['conflict'] is not None:
            report['usage'] = row['conflict']
            ledger.finish_call(ticket, **report)
    for row in case['units']:
        ledger.close_unit(**row)
    if grades:
        for ordinal, correct in case['grades']:
            ledger.grade(ordinal, correct)
    return ledger


def usage(inp=23, out=7, cache=5, reasoning=2):
    return {'input_tokens': inp, 'output_tokens': out, 'total_tokens': inp + out,
            'cached_input_tokens': cache, 'reasoning_output_tokens': reasoning}


def close(ledger, phase, ordinal, *, model='reported_calls', coverage='complete', billed=True):
    ledger.close_unit(phase, ordinal, model_work=model, coverage=coverage,
                      billing_complete=billed, local_ns=0, constructed_bytes=0)


class WholePublicOracles(unittest.TestCase):
    def test_complete_literal_all_phase_and_scope_snapshot(self):
        case = PUBLIC['cases'][0]
        self.assertEqual(EXPECTED[case['name']], run_case(case).finalize())

    def test_partial_failure_hidden_pending_preserves_whole_literal_snapshot(self):
        case = PUBLIC['cases'][1]
        self.assertEqual(EXPECTED[case['name']], run_case(case).finalize())

    def test_conflict_removes_attempt_and_grade_from_whole_literal_snapshot(self):
        case = PUBLIC['cases'][2]
        self.assertEqual(EXPECTED[case['name']], run_case(case).finalize())


class LedgerBoundaries(unittest.TestCase):
    def test_missing_usage_differs_from_explicit_zero_report(self):
        for report, total, complete in [(None, None, False), (usage(0, 0, 0, 0), 0, True)]:
            with self.subTest(report=report):
                ledger = subject.Ledger(1)
                ticket = ledger.begin_call('reader', 0, 'PUBLIC-READ', 'A')
                ledger.finish_call(ticket, outcome='failed', usage=report)
                close(ledger, 'reader', 0, billed=False)
                phase = ledger.snapshot()['phases']['reader']
                self.assertEqual(total, phase['tokens'])
                self.assertEqual(complete, phase['complete']['tokens'])
                self.assertIsNone(phase['usd_micros'])
                self.assertEqual(1, phase['outcomes']['failed'])

    def test_valid_totals_do_not_require_cache_reasoning_or_billing(self):
        ledger = subject.Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'PUBLIC-PARTIAL-BREAKDOWN', 'A')
        ledger.finish_call(ticket, outcome='succeeded', usage={'input_tokens': 31, 'output_tokens': 9},
                           usd_micros=18000, billing='unknown', component_ns=4)
        close(ledger, 'reader', 0, billed=False)
        phase = ledger.snapshot()['phases']['reader']
        self.assertEqual(40, phase['tokens'])
        self.assertEqual(18000, phase['known_usd_micros'])
        self.assertIsNone(phase['usd_micros'])
        self.assertIsNone(phase['cached_input_tokens'])
        self.assertIsNone(phase['reasoning_output_tokens'])
        self.assertIsNone(phase['uncached_input_tokens'])
        self.assertEqual(4, phase['component_ns'])

    def test_malformed_counter_total_and_subset_reports_are_unusable(self):
        mutations = [('input_tokens', True), ('output_tokens', -1),
                     ('total_tokens', 999), ('cached_input_tokens', 24),
                     ('reasoning_output_tokens', 8), ('input_tokens', float('nan')),
                     ('input_tokens', subject.COUNTER_LIMIT + 1)]
        for key, value in mutations:
            with self.subTest(key=key, value=str(value)):
                ledger = subject.Ledger(1)
                ticket = ledger.begin_call('reader', 0, 'PUBLIC-BAD', 'A')
                report = usage()
                report[key] = value
                ledger.finish_call(ticket, outcome='succeeded', usage=report)
                close(ledger, 'reader', 0, billed=False)
                phase = ledger.snapshot()['phases']['reader']
                self.assertEqual(0, phase['known_tokens'])
                self.assertIsNone(phase['tokens'])
                self.assertEqual(1, ledger.snapshot()['observation_errors'])

    def test_unpaired_scalars_are_diagnostics_not_complete_attempt_subtotals(self):
        ledger = subject.Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'PUBLIC-UNPAIRED', 'A')
        ledger.finish_call(ticket, outcome='cancelled', usage={'input_tokens': 19, 'total_tokens': 34})
        close(ledger, 'reader', 0, billed=False)
        phase = ledger.snapshot()['phases']['reader']
        self.assertEqual((19, 0, 0), (phase['known_input_tokens'], phase['known_output_tokens'], phase['known_tokens']))
        self.assertIsNone(phase['output_tokens'])
        self.assertIsNone(phase['tokens'])

    def test_no_model_attestation_is_explicit_local_and_has_no_attempts(self):
        ledger = subject.Ledger(1)
        self.assertIsNone(ledger.snapshot()['phases']['build']['tokens'])
        close(ledger, 'build', None, model='none_observed_local')
        self.assertEqual(0, ledger.snapshot()['phases']['build']['tokens'])
        for phase, ordinal, attempt in [('reader', 0, False), ('query', 0, True)]:
            bad = subject.Ledger(1)
            if attempt:
                bad.begin_call(phase, ordinal, 'PUBLIC-REAL-RESERVATION', 'A')
            with self.assertRaisesRegex(subject.LedgerError, 'no_model_conflict'):
                close(bad, phase, ordinal, model='none_observed_local')
            self.assertIsNone(bad.snapshot()['phases'][phase]['tokens'])
        empty = subject.Ledger(1)
        with self.assertRaisesRegex(subject.LedgerError, 'missing_reported_calls'):
            close(empty, 'reader', 0)
        self.assertIsNone(empty.snapshot()['phases']['reader']['tokens'])

    def test_physical_attempt_identity_cannot_be_counted_in_another_scope(self):
        for phase, ordinal in [('query', 0), ('reader', 1)]:
            with self.subTest(phase=phase, ordinal=ordinal):
                ledger = subject.Ledger(2)
                ticket = ledger.begin_call('reader', 0, 'PUBLIC-ONE-PHYSICAL', 'A')
                ledger.finish_call(ticket, outcome='succeeded', usage=usage())
                with self.assertRaisesRegex(subject.LedgerError, 'attempt_scope_conflict'):
                    ledger.begin_call(phase, ordinal, 'PUBLIC-ONE-PHYSICAL', 'A')
                snapshot = ledger.snapshot()
                self.assertEqual(1, snapshot['attempts'])
                self.assertEqual(1, snapshot['phases']['reader']['conflicting_attempts'])
                self.assertEqual(0, snapshot['product']['known_tokens'])
                self.assertIsNone(snapshot['product']['tokens'])

    def test_same_reservation_and_receipt_are_idempotent_cost_not_authority(self):
        ledger = subject.Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'PUBLIC-IDEMPOTENT', 'A')
        self.assertIs(ticket, ledger.begin_call('reader', 0, 'PUBLIC-IDEMPOTENT', 'A'))
        ledger.finish_call(ticket, outcome='failed', usage=usage())
        ledger.finish_call(ticket, outcome='failed', usage=usage())
        close(ledger, 'reader', 0, billed=False)
        ledger.finish_call(ticket, outcome='failed', usage=usage())
        snapshot = ledger.snapshot()
        self.assertEqual(1, snapshot['attempts'])
        self.assertEqual(30, snapshot['phases']['reader']['tokens'])
        self.assertEqual(3, snapshot['duplicates'])

    def test_closed_membership_pending_and_changed_receipts_stay_incomplete(self):
        ledger = subject.Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'PUBLIC-PENDING', 'A')
        close(ledger, 'reader', 0, billed=False)
        self.assertIsNone(ledger.snapshot()['phases']['reader']['tokens'])
        with self.assertRaisesRegex(subject.LedgerError, 'late_report'):
            ledger.finish_call(ticket, outcome='succeeded', usage=usage())
        with self.assertRaisesRegex(subject.LedgerError, 'unit_closed'):
            ledger.begin_call('reader', 0, 'PUBLIC-NEW-LATE', 'B')
        self.assertEqual(1, ledger.snapshot()['attempts'])
        self.assertIsNone(ledger.snapshot()['product']['tokens'])
        other = subject.Ledger(1)
        second = other.begin_call('reader', 0, 'PUBLIC-CHANGED', 'A')
        other.finish_call(second, outcome='succeeded', usage=usage())
        close(other, 'reader', 0, billed=False)
        with self.assertRaisesRegex(subject.LedgerError, 'late_report'):
            other.finish_call(second, outcome='succeeded', usage=usage(24, 7, 5, 2))
        self.assertEqual(0, other.snapshot()['phases']['reader']['known_tokens'])
        self.assertIsNone(other.snapshot()['phases']['reader']['tokens'])

    def test_foreign_reconstructed_and_mutated_tickets_are_refused(self):
        ledger = subject.Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'PUBLIC-OWNER', 'A')
        other = subject.Ledger(1)
        with self.assertRaisesRegex(subject.LedgerError, 'ticket_binding'):
            other.finish_call(ticket, outcome='succeeded', usage=usage())
        forged = subject.AttemptTicket(ticket._owner, ticket._key)
        with self.assertRaisesRegex(subject.LedgerError, 'ticket_binding'):
            ledger.finish_call(forged, outcome='succeeded', usage=usage())
        ticket._key = []
        with self.assertRaisesRegex(subject.LedgerError, 'ticket_binding'):
            ledger.finish_call(ticket, outcome='succeeded', usage=usage())
        self.assertEqual(0, ledger.snapshot()['phases']['reader']['finished_attempts'])

    def test_capacity_overflow_never_truncates_to_complete_cost(self):
        ledger = subject.Ledger(1, attempt_limit=1)
        ticket = ledger.begin_call('reader', 0, 'PUBLIC-ONE', 'A')
        ledger.finish_call(ticket, outcome='succeeded', usage=usage())
        with self.assertRaisesRegex(subject.LedgerError, 'attempt_capacity'):
            ledger.begin_call('judge', 0, 'PUBLIC-TWO', 'A')
        snapshot = ledger.finalize()
        self.assertEqual(1, snapshot['attempts'])
        self.assertTrue(snapshot['overflow'])
        self.assertEqual(30, snapshot['product']['known_tokens'])
        self.assertIsNone(snapshot['product']['tokens'])
        limited = subject.Ledger(1, event_limit=1)
        pending = limited.begin_call('reader', 0, 'PUBLIC-EVENT', 'A')
        with self.assertRaisesRegex(subject.LedgerError, 'event_capacity'):
            limited.finish_call(pending, outcome='succeeded', usage=usage())
        self.assertTrue(limited.finalize()['overflow'])

    def test_derived_and_aggregate_counter_overflow_are_explicit(self):
        invalid = subject.Ledger(1)
        ticket = invalid.begin_call('reader', 0, 'PUBLIC-SUM', 'A')
        invalid.finish_call(ticket, outcome='succeeded', usage={'input_tokens': subject.COUNTER_LIMIT, 'output_tokens': 1})
        close(invalid, 'reader', 0, billed=False)
        self.assertIsNone(invalid.snapshot()['phases']['reader']['tokens'])
        ledger = subject.Ledger(1)
        for ordinal in (0, 1):
            ticket = ledger.begin_call('reader', 0, 'PUBLIC-AGGREGATE', str(ordinal))
            ledger.finish_call(ticket, outcome='succeeded', usage=usage(subject.COUNTER_LIMIT, 0, 0, 0))
        close(ledger, 'reader', 0, billed=False)
        snapshot = ledger.snapshot()
        self.assertTrue(snapshot['overflow'])
        self.assertIsNone(snapshot['phases']['reader']['known_tokens'])
        self.assertIsNone(snapshot['phases']['reader']['tokens'])

    def test_missing_judge_usage_preserves_complete_product_only(self):
        ledger = run_case(PUBLIC['cases'][0], missing_judge=True)
        snapshot = ledger.finalize()
        self.assertEqual(270, snapshot['product']['tokens'])
        self.assertEqual(270, snapshot['product']['tokens_per_correct'])
        self.assertIsNone(snapshot['experiment']['tokens'])
        self.assertIsNone(snapshot['experiment']['tokens_per_correct'])
        self.assertEqual(335000, snapshot['experiment']['usd_micros'])

    def test_incomplete_and_zero_grades_have_fixed_denominators(self):
        ledger = run_case(PUBLIC['cases'][0], grades=False)
        snapshot = ledger.snapshot()
        self.assertEqual(135, snapshot['product']['tokens_per_planned_query'])
        self.assertIsNone(snapshot['product']['tokens_per_correct'])
        ledger.grade(0, False)
        ledger.grade(1, False)
        snapshot = ledger.finalize()
        self.assertEqual(0, snapshot['grading']['correct'])
        self.assertIsNone(snapshot['product']['tokens_per_correct'])
        self.assertIsNone(snapshot['product']['usd_micros_per_correct'])
        null = subject.Ledger(1)
        null.grade(0, None)
        self.assertEqual(0, null.snapshot()['grading']['observed'])
        self.assertIsNone(null.snapshot()['grading']['correct'])

    def test_nested_snapshots_final_freeze_and_configuration_are_independent(self):
        ledger = run_case(PUBLIC['cases'][0])
        first = ledger.snapshot()
        first['product']['tokens'] = -999
        first['phases']['reader']['complete']['tokens'] = False
        self.assertEqual(270, ledger.snapshot()['product']['tokens'])
        final = ledger.finalize()
        expected = copy.deepcopy(final)
        final['grading']['correct'] = 999
        self.assertEqual(expected, ledger.finalize())
        with self.assertRaises(AttributeError):
            ledger.horizon = 99
        with self.assertRaises(AttributeError):
            ledger.attempt_limit = 0
        with self.assertRaisesRegex(subject.LedgerError, 'ledger_frozen'):
            ledger.grade(0, False)
        self.assertEqual(expected, ledger.snapshot())

    def test_privacy_sentinels_and_arbitrary_provenance_never_export(self):
        sentinel = 'PRIVATE_CALLBACK_BODY_SENTINEL'
        ledger = subject.Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'person@example.com', sentinel)
        report = usage()
        report['prompt'] = sentinel
        ledger.finish_call(ticket, outcome='failed', usage=report, usd_micros=4,
                           billing=sentinel, component_ns=3)
        close(ledger, 'reader', 0, billed=False)
        encoded = json.dumps(ledger.finalize())
        self.assertNotIn(sentinel, encoded)
        self.assertNotIn('person@example.com', encoded)
        self.assertNotIn(ticket._key.hex(), encoded)
        self.assertNotIn(sentinel, repr(ticket))
        self.assertIsNone(ledger.snapshot()['phases']['reader']['tokens'])

    def test_invalid_bounds_ordinals_types_and_outcome_are_fixed_refusals(self):
        for horizon in (True, 0, 513, float('inf')):
            with self.subTest(horizon=horizon), self.assertRaisesRegex(subject.LedgerError, 'ledger_bounds'):
                subject.Ledger(horizon)
        for phase, ordinal in [('build', 0), ('reader', True), ('judge', -1), ('query', 1), ('not-a-phase', 0)]:
            ledger = subject.Ledger(1)
            with self.assertRaises(subject.LedgerError):
                ledger.begin_call(phase, ordinal, 'PUBLIC-INVALID', 'A')
            self.assertEqual(0, ledger.snapshot()['attempts'])
        ledger = subject.Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'PUBLIC-OUTCOME', 'A')
        with self.assertRaisesRegex(subject.LedgerError, '^outcome_schema$'):
            ledger.finish_call(ticket, outcome='SECRET_ERROR_PROSE', usage=usage())
        self.assertNotIn('SECRET_ERROR_PROSE', json.dumps(ledger.snapshot()))

    def test_partial_total_cannot_be_less_than_any_known_component(self):
        for report in ({'input_tokens': 29, 'total_tokens': 14},
                       {'output_tokens': 11, 'total_tokens': 8}):
            ledger = subject.Ledger(1)
            ticket = ledger.begin_call('reader', 0, 'PUBLIC-PARTIAL-IMPOSSIBLE', 'A')
            ledger.finish_call(ticket, outcome='succeeded', usage=report)
            close(ledger, 'reader', 0, billed=False)
            phase = ledger.snapshot()['phases']['reader']
            self.assertEqual((0, 0, 0), (phase['known_input_tokens'], phase['known_output_tokens'], phase['known_tokens']))
            self.assertIsNone(phase['tokens'])

    def test_conflicting_unit_closure_removes_only_its_component_scalars(self):
        ledger = subject.Ledger(1)
        arguments = dict(coverage='complete', model_work='none_observed_local',
                         billing_complete=True, local_ns=13, constructed_bytes=27)
        ledger.close_unit('query', 0, **arguments)
        ledger.close_unit('query', 0, **arguments)
        first = ledger.snapshot()['phases']['query']
        self.assertEqual((13, 27), (first['known_component_ns'], first['known_constructed_bytes']))
        arguments.update(local_ns=14, constructed_bytes=28)
        ledger.close_unit('query', 0, **arguments)
        final = ledger.snapshot()['phases']['query']
        self.assertEqual((0, 0), (final['known_component_ns'], final['known_constructed_bytes']))
        self.assertIsNone(final['component_ns'])
        self.assertIsNone(final['constructed_bytes'])
        self.assertIsNone(final['tokens'])

    def test_invalid_money_and_duration_do_not_invalidate_valid_tokens(self):
        ledger = subject.Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'PUBLIC-SEPARATE-METRICS', 'A')
        ledger.finish_call(ticket, outcome='succeeded', usage=usage(),
                           usd_micros=True, billing='provider_reported', component_ns=float('inf'))
        close(ledger, 'reader', 0)
        phase = ledger.snapshot()['phases']['reader']
        self.assertEqual(30, phase['tokens'])
        self.assertIsNone(phase['usd_micros'])
        self.assertIsNone(phase['component_ns'])
        self.assertEqual(2, ledger.snapshot()['observation_errors'])

    def test_exact_close_replay_preserves_independent_bytes_after_attempt_conflict(self):
        ledger = subject.Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'PUBLIC-INDEPENDENT-UNIT', 'A')
        ledger.finish_call(ticket, outcome='succeeded', usage=usage(), component_ns=9)
        arguments = dict(coverage='complete', model_work='reported_calls',
                         billing_complete=False, local_ns=13, constructed_bytes=27)
        ledger.close_unit('reader', 0, **arguments)
        with self.assertRaisesRegex(subject.LedgerError, 'late_report'):
            ledger.finish_call(ticket, outcome='failed', usage=usage(), component_ns=9)
        ledger.close_unit('reader', 0, **arguments)
        phase = ledger.snapshot()['phases']['reader']
        self.assertEqual((13, 27), (phase['known_component_ns'], phase['known_constructed_bytes']))
        self.assertEqual(0, phase['known_tokens'])
        self.assertIsNone(phase['tokens'])
        self.assertIsNone(phase['component_ns'])

    def test_uninitialized_and_deleted_ticket_slots_use_fixed_refusal(self):
        for kind in ('uninitialized', 'deleted_key', 'deleted_owner'):
            with self.subTest(kind=kind):
                ledger = subject.Ledger(1)
                ticket = ledger.begin_call('reader', 0, 'PUBLIC-MISSING-SLOT', 'A')
                if kind == 'uninitialized':
                    ticket = object.__new__(subject.AttemptTicket)
                elif kind == 'deleted_key':
                    del ticket._key
                else:
                    del ticket._owner
                with self.assertRaisesRegex(subject.LedgerError, '^ticket_binding$'):
                    ledger.finish_call(ticket, outcome='succeeded', usage=usage())
                snapshot = ledger.finalize()
                self.assertEqual(1, snapshot['observation_errors'])
                self.assertIsNone(snapshot['product']['tokens'])


if __name__ == '__main__':
    unittest.main()
