"""Fresh injected-argument/receipt controls, not provider or native quality."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import longmemeval_lifecycle as base
from harness.longmemeval_bridge import PROFILE as BRIDGE_PROFILE
from harness.longmemeval_observed import PROFILE as LOCAL_PROFILE
from harness.memory_benchmark_cost import PROFILE as LEDGER_PROFILE
from harness.longmemeval_provider_usage import CompletionCapture
from harness.longmemeval_chat_receipts import ChatController, PROFILE

FIXTURES = ROOT / 'tests/fixtures'
PUBLIC = json.loads((FIXTURES / 'longmemeval_chat_receipts_public.json').read_text())
EXPECTED = json.loads((FIXTURES / 'longmemeval_chat_receipts_expected.json').read_text())


class SuppliedMemory:
    """NEW controlled local profile; no native bank/index/runtime is imported."""
    def __init__(self):
        self.closed = False
        self.receipts = []
        self.close_calls = self.context_depth = 0
        self.close_error = None
        self.snapshot_calls = 0

    def set_query_context(self, **_):
        self.context_depth += 1

    def clear_query_context(self):
        self.context_depth -= 1

    def query(self, question, image):
        spec = PUBLIC['memory_spec']
        self.receipts.append({'ordinal': len(self.receipts), 'outcome': 'succeeded',
            'state': 'delivered', 'native_metadata_bound': True,
            'input_bytes': len(question.encode()), 'output_bytes': spec['query_output_bytes'],
            'component_ns': spec['query_ns'][len(self.receipts)], 'query_image_ignored': False})
        return copy.deepcopy(spec['context'])

    def post_query_hook(self, **_):
        return {'query_receipt': copy.deepcopy(self.receipts[-1])}

    def snapshot(self):
        self.snapshot_calls += 1
        spec = PUBLIC['memory_spec']
        return {'profile': BRIDGE_PROFILE, 'closed': self.closed,
            'local_observation': {'profile': LOCAL_PROFILE,
                'ledger': {'profile': LEDGER_PROFILE, 'horizon': 2, 'frozen': self.closed},
                'observed': {'query_calls': len(self.receipts), 'insert_calls': 1,
                    'query_receipts': copy.deepcopy(self.receipts),
                    'local_token_profile': 'pinned_local_no_model',
                    'insert_ingress_bytes': spec['insert_ingress_bytes'],
                    'build_component_ns': spec['build_ns'], 'observation_incomplete': False,
                    'initialization_outcome': 'succeeded',
                    'cleanup_outcome': 'succeeded' if self.closed else 'not_attempted'}}}

    def close(self):
        self.close_calls += 1
        self.closed = True
        if self.close_error is not None:
            raise self.close_error
        return self.snapshot()


def make(*, budget=4096, clock=None, questions=None, **changes):
    memory = SuppliedMemory()
    args = {'reader_model': PUBLIC['reader_model'], 'judge_model': PUBLIC['judge_model'],
            'reader_max_completion_tokens': PUBLIC['reader_max_completion_tokens'],
            'judge_max_completion_tokens': PUBLIC['judge_max_completion_tokens'],
            'reader_budget': budget, 'clock': clock}
    args.update(changes)
    ctrl = ChatController(memory, copy.deepcopy(PUBLIC['questions']) if questions is None else questions, **args)
    return ctrl, memory


def response(i=0):
    return PUBLIC['raw_responses'][i].encode()


def text(data):
    return json.loads(data)['choices'][0]['message']['content']


def readers(ctrl):
    ctrl.query(0)
    ctrl.query(1)
    error = ValueError(PUBLIC['first_decoder_error'])
    def bad(_):
        raise error
    try:
        ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(0), bad)
    except ValueError as caught:
        if caught is not error:
            raise
    else:
        raise AssertionError('original decoder error missing')
    ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(1), text)
    ctrl.dispatch(ctrl.prepare_reader(1), lambda _: response(2), text)


class ChatReceiptTests(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory(prefix='memory-chat-public-control-')
        self.addCleanup(self._temp.cleanup)

    def test_full_independent_body_seal_and_known_cost_oracles(self):
        clock = iter(PUBLIC['clock_points'])
        ctrl, memory = make(clock=lambda: next(clock))
        readers(ctrl)
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['outcome'], 'failed')
        gate = ctrl.seal_construction()
        self.assertEqual(gate.body, EXPECTED['construction_utf8'].encode())
        self.assertEqual(gate.sha256, EXPECTED['construction_sha256'])
        self.assertEqual(memory.close_calls, 1)
        self.assertEqual(memory.context_depth, 0)
        packet = Path(self._temp.name) / 'reader-construction.json'
        packet.write_bytes(gate.body)
        submitted, lookups = [], []
        for ordinal in range(2):
            ticket = ctrl.prepare_judge(ordinal, gate)
            def factory(ordinal=ordinal):
                lookups.append(gate.sha256)
                # Public evaluator lookup is deliberately after persisted gate.
                self.assertEqual(packet.read_bytes(), EXPECTED['construction_utf8'].encode())
                def send(body):
                    submitted.append(body)
                    return response(ordinal + 3)
                return send, lambda data: json.loads(text(data))
            self.assertEqual(ctrl.dispatch(ticket, factory), PUBLIC['scripted_grades'][ordinal])
        final = ctrl.finalize()
        self.assertEqual(submitted, [s.encode() for s in EXPECTED['judge_request_utf8']])
        self.assertEqual(len(lookups), 2)
        self.assertEqual(final['profile'], PROFILE)
        self.assertEqual(final['callback_entries'], 5)
        self.assertEqual(final['injected_transport_boundaries'], 5)
        for scope in ('product', 'experiment'):
            metric = final['ledger'][scope]
            for field in ('tokens', 'input', 'output', 'component_ns', 'request_bytes'):
                actual = {'tokens': 'known_tokens', 'input': 'known_input_tokens', 'output': 'known_output_tokens',
                          'component_ns': 'known_component_ns', 'request_bytes': 'known_constructed_bytes'}[field]
                self.assertEqual(metric[actual], EXPECTED[scope + '_known_' + field])
            self.assertIsNone(metric['tokens'])
            self.assertIsNone(metric['usd_micros'])
            self.assertFalse(metric['complete']['coverage'])
        journal = ctrl.private_judge_journal()
        self.assertEqual(json.loads(journal.body)['reader_construction_sha256'], gate.sha256)
        self.assertEqual([row['request'] for row in json.loads(journal.body)['judge_attempts']], EXPECTED['judge_requests'])
        self.assertIs(ctrl.private_judge_journal(), journal)

    def test_final_body_budget_retains_drops_and_refuses_before_entry(self):
        for budget, expected in ((369, 'reader_request_utf8'), (368, 'baseline_request_utf8')):
            ctrl, _ = make(budget=budget)
            ctrl.query(0)
            ticket = ctrl.prepare_reader(0)
            seen = []
            ctrl.dispatch(ticket, lambda body: seen.append(body) or response(), text)
            self.assertEqual(seen, [EXPECTED[expected].encode()])
        ctrl, _ = make(budget=254)
        ctrl.query(0)
        with self.assertRaisesRegex(base.ControllerError, '^chat_request_bound$'):
            ctrl.prepare_reader(0)
        self.assertEqual(ctrl.snapshot()['injected_transport_boundaries'], 0)
        self.assertEqual(ctrl.snapshot()['reader_attempts'], [])

    def test_identical_bodies_are_owned_by_distinct_ordinals(self):
        ctrl, _ = make()
        seen = []
        for ordinal in range(2):
            ctrl.query(ordinal)
            ctrl.dispatch(ctrl.prepare_reader(ordinal), lambda body: seen.append(body) or response(), text)
        self.assertEqual(seen[0], seen[1])
        rows = ctrl.snapshot()['reader_attempts']
        self.assertEqual([r['ordinal'] for r in rows], [0, 1])
        self.assertEqual([r['usage']['total_tokens'] for r in rows], [48, 48])
        self.assertEqual(ctrl.snapshot()['ledger']['product']['known_tokens'], 96)

    def test_paid_decoder_error_cancel_and_original_identity(self):
        for error in (ValueError('unretained'), KeyboardInterrupt(), SystemExit(8)):
            ctrl, _ = make()
            ctrl.query(0)
            def decode(_):
                raise error
            with self.assertRaises(type(error)) as caught:
                ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), decode)
            self.assertIs(caught.exception, error)
            row = ctrl.snapshot()['reader_attempts'][0]
            self.assertEqual(row['usage']['total_tokens'], 48)
            self.assertEqual(row['outcome'], 'failed' if isinstance(error, Exception) else 'cancelled')
            self.assertEqual(ctrl.snapshot()['ledger']['phases']['reader']['attempts'], 1)

    def test_invalid_decoded_value_cannot_discard_paid_usage(self):
        for value in (42, {'usage': {'input_tokens': 999}}, None, '\ud800'):
            ctrl, _ = make()
            ctrl.query(0)
            with self.assertRaises(ValueError):
                ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), lambda _: value)
            self.assertEqual(ctrl.snapshot()['ledger']['product']['known_tokens'], 48)
            self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['outcome'], 'failed')

    def test_async_decoder_value_is_refused_without_awaiting_or_dropping_usage(self):
        ctrl, _ = make()
        ctrl.query(0)
        saved = []
        async def later():
            return 'not observed'
        def decode(_):
            value = later()
            saved.append(value)
            return value
        try:
            with self.assertRaises(ValueError):
                ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), decode)
            self.assertEqual(ctrl.snapshot()['ledger']['product']['known_tokens'], 48)
        finally:
            for value in saved:
                value.close()

    def test_decoder_error_beats_end_clock_cancellation(self):
        error, clock_error = ValueError('decoder'), KeyboardInterrupt()
        points = iter([10, clock_error])
        def clock():
            value = next(points)
            if type(value) is not int:
                raise value
            return value
        ctrl, _ = make(clock=clock)
        ctrl.query(0)
        def decode(_):
            raise error
        with self.assertRaises(ValueError) as caught:
            ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), decode)
        self.assertIs(caught.exception, error)
        snap = ctrl.snapshot()
        self.assertEqual(snap['ledger']['product']['known_tokens'], 48)
        self.assertEqual(snap['reader_attempts'][0]['outcome'], 'failed')
        self.assertGreater(snap['observation_faults'], 0)

    def test_successful_paid_result_retained_on_end_clock_cancellation(self):
        points = iter([10, KeyboardInterrupt()])
        def clock():
            value = next(points)
            if type(value) is not int:
                raise value
            return value
        ctrl, _ = make(clock=clock)
        ctrl.query(0)
        with self.assertRaises(KeyboardInterrupt):
            ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), text)
        snap = ctrl.snapshot()
        self.assertEqual(snap['ledger']['product']['known_tokens'], 48)
        self.assertEqual(snap['reader_attempts'][0]['outcome'], 'succeeded')

    def test_start_clock_cancel_has_zero_transport_entries_and_bytes(self):
        ctrl, _ = make(clock=lambda: (_ for _ in ()).throw(KeyboardInterrupt()))
        ctrl.query(0)
        seen = []
        with self.assertRaises(KeyboardInterrupt):
            ctrl.dispatch(ctrl.prepare_reader(0), lambda body: seen.append(body) or response(), text)
        snap = ctrl.snapshot()
        self.assertEqual(seen, [])
        self.assertEqual(snap['injected_transport_boundaries'], 0)
        self.assertEqual(snap['reader_attempts'][0]['request_bytes'], 0)
        self.assertEqual(snap['ledger']['product']['known_tokens'], 0)  # empty usable subtotal
        self.assertIsNone(snap['ledger']['product']['tokens'])

    def test_factory_failure_zero_transport_bytes_and_no_early_lookup(self):
        ctrl, _ = make()
        ctrl.query(0)
        ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), text)
        lookups = []
        def factory():
            lookups.append(True)
            raise ValueError('public failed factory')
        with self.assertRaises(base.ControllerError):
            ctrl.prepare_judge(0, base.Construction(b'{}'))
        self.assertEqual(lookups, [])
        gate = ctrl.seal_construction()
        with self.assertRaises(ValueError):
            ctrl.dispatch(ctrl.prepare_judge(0, gate), factory)
        final = ctrl.finalize()
        self.assertEqual(lookups, [True])
        self.assertEqual(final['judge_attempts'][0]['injected_transport_boundaries'], 0)
        self.assertEqual(final['ledger']['phases']['judge']['known_constructed_bytes'], 0)

    def test_foreign_replayed_and_wrong_api_arguments_refuse_before_entry(self):
        one, _ = make()
        two, _ = make()
        one.query(0)
        two.query(0)
        ticket, foreign = one.prepare_reader(0), two.prepare_reader(0)
        seen = []
        send = lambda body: seen.append(body) or response()
        with self.assertRaises(base.ControllerError):
            one.dispatch(foreign, send, text)
        with self.assertRaises(base.ControllerError):
            one.dispatch(ticket, send)
        self.assertEqual(seen, [])
        one.dispatch(ticket, send, text)
        with self.assertRaises(base.ControllerError):
            one.dispatch(ticket, send, text)
        self.assertEqual(len(seen), 1)

    def test_reentrant_prepare_and_seal_refuse_while_transport_runs(self):
        ctrl, _ = make()
        ctrl.query(0)
        def send(_):
            with self.assertRaises(base.ControllerError):
                ctrl.prepare_reader(1)
            with self.assertRaises(base.ControllerError):
                ctrl.seal_construction()
            return response()
        ctrl.dispatch(ctrl.prepare_reader(0), send, text)
        self.assertEqual(ctrl.snapshot()['injected_transport_boundaries'], 1)

    def test_caller_plan_mutation_cannot_change_held_final_argument(self):
        questions = copy.deepcopy(PUBLIC['questions'])
        ctrl, _ = make(questions=questions)
        questions[0]['question'] = 'tampered'
        ctrl.query(0)
        seen = []
        ctrl.dispatch(ctrl.prepare_reader(0), lambda body: seen.append(body) or response(), text)
        self.assertEqual(seen, [EXPECTED['reader_request_utf8'].encode()])

    def test_nonbytes_transport_cannot_spoof_normalized_usage(self):
        ctrl, _ = make()
        ctrl.query(0)
        decode_calls = []
        with self.assertRaises(base.ControllerError):
            ctrl.dispatch(ctrl.prepare_reader(0), lambda _: {'usage': {'input_tokens': 999}}, decode_calls.append)
        self.assertEqual(decode_calls, [])
        self.assertIsNone(ctrl.snapshot()['reader_attempts'][0]['usage']['input_tokens'])
        self.assertEqual(ctrl.snapshot()['ledger']['product']['known_tokens'], 0)

    def test_missing_usage_is_unknown_even_when_decoder_returns_answer(self):
        ctrl, _ = make()
        ctrl.query(0)
        data = json.loads(response())
        del data['usage']
        ctrl.dispatch(ctrl.prepare_reader(0), lambda _: base._wire(data), text)
        snap = ctrl.snapshot()
        self.assertEqual(snap['reader_attempts'][0]['usage_state'], 'missing')
        self.assertIsNone(snap['reader_attempts'][0]['usage']['input_tokens'])
        self.assertIsNone(snap['ledger']['product']['tokens'])

    def test_capture_metadata_failure_marks_unknown_and_task_continues(self):
        ctrl, _ = make()
        ctrl.query(0)
        with patch.object(CompletionCapture, 'snapshot', side_effect=RuntimeError('not retained')):
            self.assertEqual(ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), text), 'unused paid answer')
        snap = ctrl.snapshot()
        self.assertEqual(snap['reader_attempts'][0]['usage_state'], 'unmeasured')
        self.assertIsNone(snap['reader_attempts'][0]['usage']['total_tokens'])
        self.assertGreater(snap['observation_faults'], 0)

    def test_capture_read_once_prevents_paid_usage_downgrade_after_invalid_answer(self):
        ctrl, _ = make()
        ctrl.query(0)
        original = CompletionCapture.snapshot
        calls = []
        def observed_snapshot(capture):
            calls.append(True)
            if len(calls) > 1:
                raise RuntimeError('second snapshot must not be used')
            return original(capture)
        with patch.object(CompletionCapture, 'snapshot', observed_snapshot):
            with self.assertRaises(ValueError):
                ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), lambda _: 42)
        self.assertEqual(len(calls), 1)
        self.assertEqual(ctrl.snapshot()['ledger']['product']['known_tokens'], 48)
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['outcome'], 'failed')

    def test_capture_constructor_failure_is_observational_and_does_not_poison_ticket(self):
        ctrl, _ = make()
        ctrl.query(0)
        with patch('harness.longmemeval_chat_receipts.CompletionCapture', side_effect=RuntimeError('not retained')):
            ticket = ctrl.prepare_reader(0)
        self.assertEqual(ctrl.dispatch(ticket, lambda _: response(), text), 'unused paid answer')
        row = ctrl.snapshot()['reader_attempts'][0]
        self.assertEqual(row['outcome'], 'succeeded')
        self.assertEqual(row['usage_state'], 'unmeasured')
        self.assertIsNone(row['usage']['total_tokens'])
        self.assertEqual(row['response_bytes'], len(response()))
        self.assertGreater(ctrl.snapshot()['observation_faults'], 0)

    def test_capture_constructor_cancellation_precedes_ticket_issuance(self):
        ctrl, _ = make()
        ctrl.query(0)
        error = KeyboardInterrupt()
        with patch('harness.longmemeval_chat_receipts.CompletionCapture', side_effect=error):
            with self.assertRaises(KeyboardInterrupt) as caught:
                ctrl.prepare_reader(0)
        self.assertIs(caught.exception, error)
        self.assertEqual(ctrl.snapshot()['reader_attempts'], [])
        self.assertEqual(ctrl.snapshot()['pending_tickets'], 0)
        self.assertEqual(ctrl.snapshot()['injected_transport_boundaries'], 0)

    def test_capture_snapshot_cancel_finishes_attempt_then_propagates(self):
        ctrl, _ = make()
        ctrl.query(0)
        error = KeyboardInterrupt()
        with patch.object(CompletionCapture, 'snapshot', side_effect=error):
            with self.assertRaises(KeyboardInterrupt) as caught:
                ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), text)
        self.assertIs(caught.exception, error)
        row = ctrl.snapshot()['reader_attempts'][0]
        self.assertEqual(row['outcome'], 'cancelled')
        self.assertEqual(row['response_bytes'], len(response()))
        self.assertIsNone(row['usage']['total_tokens'])
        self.assertEqual(ctrl.snapshot()['ledger']['phases']['reader']['finished_attempts'], 1)

    def test_decoder_error_outranks_capture_snapshot_cancellation(self):
        ctrl, _ = make()
        ctrl.query(0)
        error = ValueError('original decoder')
        def decode(_):
            raise error
        with patch.object(CompletionCapture, 'snapshot', side_effect=KeyboardInterrupt()):
            with self.assertRaises(ValueError) as caught:
                ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), decode)
        self.assertIs(caught.exception, error)
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['outcome'], 'failed')
        self.assertEqual(ctrl.snapshot()['ledger']['phases']['reader']['finished_attempts'], 1)

    def test_paid_answer_held_refusal_preserves_first_terminal_report(self):
        ctrl, _ = make()
        ctrl.query(0)
        ticket = ctrl.prepare_reader(0)
        with patch.object(base, 'MAX_HELD', ctrl._held):  # stricter synthetic ceiling only
            with self.assertRaises(base.ControllerError):
                ctrl.dispatch(ticket, lambda _: response(), text)
        self.assertEqual(ctrl.snapshot()['ledger']['product']['known_tokens'], 48)
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['outcome'], 'failed')

    def test_augmented_seal_overflow_never_issues_base_gate(self):
        ctrl, _ = make()
        readers(ctrl)
        with patch.object(base, 'MAX_HELD', ctrl._held):
            with self.assertRaises(base.ControllerError):
                ctrl.seal_construction()
        snap = ctrl.snapshot()
        self.assertTrue(snap['construction_blocked'])
        self.assertIsNone(snap['construction_sha256'])
        with self.assertRaises(base.ControllerError):
            ctrl.prepare_judge(0, base.Construction(EXPECTED['construction_utf8'].encode()))
        self.assertTrue(ctrl.finalize()['ledger']['frozen'])

    def test_cleanup_cancel_closes_once_and_blocks_judge_admission(self):
        ctrl, memory = make()
        ctrl.query(0)
        ctrl.dispatch(ctrl.prepare_reader(0), lambda _: response(), text)
        error = KeyboardInterrupt()
        memory.close_error = error
        with self.assertRaises(KeyboardInterrupt) as caught:
            ctrl.seal_construction()
        self.assertIs(caught.exception, error)
        self.assertIsNone(ctrl.snapshot()['construction_sha256'])
        final = ctrl.finalize()
        self.assertTrue(final['closed'])
        self.assertEqual(memory.close_calls, 1)
        self.assertEqual(final['ledger']['product']['known_tokens'], 48)

    def test_journal_overflow_cannot_change_frozen_ledger_or_reopen_gate(self):
        ctrl, _ = make()
        readers(ctrl)
        gate = ctrl.seal_construction()
        ctrl.dispatch(ctrl.prepare_judge(0, gate), lambda: (lambda _: response(3), lambda data: json.loads(text(data))))
        ctrl.finalize()
        before = ctrl.snapshot()['ledger']
        with patch.object(base, 'MAX_HELD', ctrl._held):
            with self.assertRaises(base.ControllerError):
                ctrl.private_judge_journal()
        self.assertEqual(ctrl.snapshot()['ledger'], before)
        self.assertEqual(ctrl.snapshot()['construction_sha256'], gate.sha256)
        self.assertTrue(ctrl.snapshot()['closed'])

    def test_private_journal_requires_finalization_and_returns_same_artifact(self):
        ctrl, _ = make()
        readers(ctrl)
        gate = ctrl.seal_construction()
        with self.assertRaises(base.ControllerError):
            ctrl.private_judge_journal()
        ctrl.finalize()
        journal = ctrl.private_judge_journal()
        self.assertIs(journal, ctrl.private_judge_journal())
        self.assertEqual(json.loads(journal.body)['reader_construction_sha256'], gate.sha256)
        self.assertEqual(json.loads(journal.body)['judge_attempts'], [])

    def test_judge_capture_cancel_has_no_reservation_leak_and_same_budget_still_works(self):
        ctrl, _ = make()
        readers(ctrl)
        gate = ctrl.seal_construction()
        before = ctrl._held
        error = KeyboardInterrupt()
        local_bytes = len(EXPECTED['judge_requests'][0]['messages'][1]['content'].encode())
        fixed_ceiling = before + local_bytes + EXPECTED['judge_bytes'][0]
        with patch.object(base, 'MAX_HELD', fixed_ceiling):
            with patch('harness.longmemeval_chat_receipts.CompletionCapture', side_effect=error):
                with self.assertRaises(KeyboardInterrupt) as caught:
                    ctrl.prepare_judge(0, gate)
            self.assertIs(caught.exception, error)
            self.assertEqual(ctrl._held, before)
            self.assertEqual(ctrl.snapshot()['judge_attempts'], [])
            self.assertEqual(ctrl._units[('judge', 0)]['issued'], 0)
            ticket = ctrl.prepare_judge(0, gate)
            self.assertIsNotNone(ticket)
            self.assertEqual(ctrl._units[('judge', 0)]['issued'], 1)
            self.assertEqual(ctrl._held, fixed_ceiling)

    def test_judge_final_body_bound_and_held_refusal_rollback_before_issue(self):
        for failure in ('request', 'held'):
            ctrl, _ = make()
            readers(ctrl)
            gate = ctrl.seal_construction()
            before = ctrl._held
            setting = 'MAX_REQUEST' if failure == 'request' else 'MAX_HELD'
            ceiling = 300 if failure == 'request' else before
            with patch.object(base, setting, ceiling):  # stricter synthetic limits only
                with self.assertRaises(base.ControllerError):
                    ctrl.prepare_judge(0, gate)
            self.assertEqual(ctrl._held, before)
            self.assertEqual(ctrl.snapshot()['judge_attempts'], [])
            self.assertEqual(ctrl._units[('judge', 0)]['issued'], 0)

    def test_failed_and_unused_reader_reservations_remain_in_construction(self):
        ctrl, _ = make()
        ctrl.query(0)
        ctrl.query(1)
        ctrl.prepare_reader(0)
        ctrl.prepare_reader(1)
        gate = ctrl.seal_construction()
        rows = json.loads(gate.body)['reader_attempts']
        self.assertEqual([row['ordinal'] for row in rows], [0, 1])
        self.assertTrue(all(row['request_bytes'] == 0 and row['outcome'] == 'unlaunched' for row in rows))
        self.assertTrue(all(row['usage']['total_tokens'] is None for row in rows))

    def test_numeric_export_excludes_private_model_ids_answers_and_error_prose(self):
        ctrl, _ = make()
        readers(ctrl)
        ctrl.seal_construction()
        final = ctrl.finalize()
        encoded = json.dumps(final)
        for sentinel in ('PUBLIC-LARCH-ID', PUBLIC['reader_model'], PUBLIC['reader_answer'],
                         PUBLIC['first_decoder_error'], 'Larch interval?', 'Larch beacon'):
            self.assertNotIn(sentinel, encoded)
        self.assertEqual(final['provider_coverage'], 'UNKNOWN')
        self.assertIsNone(final['physical_attempts'])
        self.assertFalse(final['provider_usage_verified'])

    def test_constructor_profile_and_images_refuse_before_memory_snapshot(self):
        for changes in ({'reader_model': 7}, {'judge_model': ''},
                        {'reader_max_completion_tokens': True}, {'judge_max_completion_tokens': 65537}):
            memory = SuppliedMemory()
            config = {'reader_model': PUBLIC['reader_model'], 'judge_model': PUBLIC['judge_model'],
                      'reader_max_completion_tokens': 37, 'judge_max_completion_tokens': 19}
            config.update(changes)
            with self.assertRaises(ValueError):
                ChatController(memory, copy.deepcopy(PUBLIC['questions']), **config)
            self.assertEqual(memory.snapshot_calls, 0)
            self.assertEqual(memory.close_calls, 0)
        questions = copy.deepcopy(PUBLIC['questions'])
        questions[0]['query_image'] = 'public-image-not-supported'
        memory = SuppliedMemory()
        with self.assertRaises(base.ControllerError):
            ChatController(memory, questions, reader_model=PUBLIC['reader_model'], judge_model=PUBLIC['judge_model'],
                           reader_max_completion_tokens=37, judge_max_completion_tokens=19)
        self.assertEqual(memory.snapshot_calls, 0)


if __name__ == '__main__':
    unittest.main()
