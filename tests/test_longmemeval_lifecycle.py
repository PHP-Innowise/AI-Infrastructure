"""Fresh local callback lifecycle cells; no provider or old suite replay."""
import __future__
import copy
import dataclasses
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import longmemeval_lifecycle as subject
from harness.longmemeval_bridge import register_bridge, PROFILE as BRIDGE_PROFILE
from harness.longmemeval_observed import PROFILE as LOCAL_PROFILE
from harness.memory_benchmark_cost import PROFILE as LEDGER_PROFILE
from harness.cost_observation import COUNTER_LIMIT

PUBLIC = json.loads((ROOT / 'tests/fixtures/longmemeval_lifecycle_public.json').read_text())
EXPECTED = json.loads((ROOT / 'tests/fixtures/longmemeval_lifecycle_expected.json').read_text())


def wire(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()


class Clock:
    def __init__(self, values=None):
        self.values = iter(PUBLIC['clock_values'] if values is None else values)

    def __call__(self):
        value = next(self.values)
        if isinstance(value, BaseException):
            raise value
        return value


class Memory:
    """New supplied producer profile, never an authenticity/native proof."""
    def __init__(self, actions=None, profile='pinned_local_no_model'):
        self.actions = list(actions or ['success', 'success'])
        self.profile = profile
        self.receipts = []
        self.closed = False
        self.close_calls = self.clear_calls = self.snapshot_calls = 0
        self.insert_calls = 1
        self.cleanup = 'not_attempted'
        self.close_error = None
        self.current = None
        self.corrupt_final = None
        self.contexts = copy.deepcopy(PUBLIC['contexts'])

    def snapshot(self):
        self.snapshot_calls += 1
        result = {'profile': BRIDGE_PROFILE, 'closed': self.closed,
                  'local_observation': {'profile': LOCAL_PROFILE,
                    'ledger': {'profile': LEDGER_PROFILE, 'horizon': 2, 'frozen': self.closed},
                    'observed': {'query_calls': len(self.receipts), 'query_receipts': copy.deepcopy(self.receipts),
                        'local_token_profile': self.profile, 'insert_calls': self.insert_calls,
                        'insert_ingress_bytes': 173, 'build_component_ns': 13,
                        'observation_incomplete': any(r['outcome'] == 'unknown' for r in self.receipts),
                        'initialization_outcome': 'succeeded', 'cleanup_outcome': self.cleanup}}}
        if self.closed and self.corrupt_final is not None:
            self.corrupt_final(result)
        return result

    def set_query_context(self, *, query_invocation_id):
        self.current = query_invocation_id

    def clear_query_context(self):
        self.current = None
        self.clear_calls += 1

    def query(self, question, query_image):
        index = [q['question'] for q in PUBLIC['questions']].index(question)
        action = self.actions.pop(0)
        if action == 'prework':
            raise RuntimeError('PUBLIC_PREWORK_SENTINEL')
        context = self.contexts[index] if action == 'success' else []
        outcome = ('cancelled' if action == 'cancel' else 'failed' if action == 'warning'
                   else 'unknown' if action == 'unmeasured' else 'succeeded')
        state = ('cancelled' if action == 'cancel' else 'refresh_failed' if action == 'warning'
                 else 'unmeasured' if action == 'unmeasured' else 'delivered' if context else 'no_evidence')
        self.receipts.append({'ordinal': len(self.receipts), 'state': state, 'outcome': outcome,
                              'native_metadata_bound': action != 'unmeasured',
                              'input_bytes': EXPECTED['local_query_input_bytes'][index],
                              'output_bytes': len(wire(context)) if action != 'cancel' else 0,
                              'component_ns': PUBLIC['local_query_ns'][index], 'query_image_ignored': False})
        if action == 'cancel':
            raise KeyboardInterrupt('PUBLIC_LOCAL_CANCEL_SENTINEL')
        return copy.deepcopy(context)

    def post_query_hook(self, **kwargs):
        return {'query_receipt': copy.deepcopy(self.receipts[-1])}

    def close(self):
        if not self.closed:
            self.close_calls += 1
            self.closed = True
            self.cleanup = ('succeeded' if self.close_error is None else
                            'failed' if isinstance(self.close_error, Exception) else 'cancelled')
            if self.close_error is not None:
                raise self.close_error
        return self.snapshot()


def report(value='answer', usage=None, outcome='succeeded'):
    return {'outcome': outcome, 'value': value, 'usage': usage,
            'usd_micros': None, 'billing': 'unknown'}


class LifecycleControls(unittest.TestCase):
    def create(self, memory=None, **kwargs):
        memory = Memory() if memory is None else memory
        return subject.Controller(memory, copy.deepcopy(PUBLIC['questions']), **kwargs), memory

    def reader(self, controller, ordinal, reply=None):
        ticket = controller.prepare_reader(ordinal)
        value = report() if reply is None else copy.deepcopy(reply)
        return controller.dispatch(ticket, lambda body: value)

    def test_whole_fresh_cycle_literal_requests_seal_and_cost_oracles(self):
        controller, memory = self.create(reader_budget=PUBLIC['reader_budget'], clock=Clock())
        for ordinal in (0, 1):
            controller.query(ordinal)
        seen = []
        for ordinal, reply in zip((0, 0, 1), PUBLIC['reader_reports']):
            ticket = controller.prepare_reader(ordinal)
            def callback(body, reply=reply):
                seen.append(body)
                return copy.deepcopy(reply)
            controller.dispatch(ticket, callback)
        self.assertEqual([EXPECTED['reader_requests'][i].encode() for i in (0, 0, 1)], seen)
        with self.assertRaises(subject.ControllerError):
            controller.prepare_judge(0, subject.Construction(b'{}'))
        seal = controller.seal_construction()
        self.assertEqual(EXPECTED['construction'].encode(), seal.body)
        self.assertEqual(EXPECTED['construction_sha256'], seal.sha256)
        self.assertIs(seal, controller.seal_construction())
        self.assertEqual(1, memory.close_calls)
        judge_seen, reads = [], []
        with tempfile.TemporaryDirectory() as temporary:
            stored = Path(temporary) / 'fresh-construction.json'
            stored.write_bytes(seal.body)
            for ordinal in (0, 1):
                ticket = controller.prepare_judge(ordinal, seal)
                def factory(ordinal=ordinal):
                    self.assertEqual(seal.sha256, hashlib.sha256(stored.read_bytes()).hexdigest())
                    grades = json.loads((ROOT / 'tests/fixtures/longmemeval_lifecycle_public_grades.json').read_text())
                    reads.append(ordinal)
                    def callback(body):
                        judge_seen.append(body)
                        value = copy.deepcopy(PUBLIC['judge_reports'][ordinal])
                        value['value'] = grades[str(ordinal)]
                        return value
                    return callback
                controller.dispatch(ticket, factory)
        self.assertEqual([x.encode() for x in EXPECTED['judge_requests']], judge_seen)
        self.assertEqual([0, 1], reads)
        final = controller.finalize()
        self.assertEqual(5, final['callback_entries'])
        self.assertEqual([0, 1], final['planned_to_local'])
        self.assertEqual(1, final['local_imports'])
        self.assertEqual(5, final['ledger']['attempts'])
        self.assertEqual(EXPECTED['grades'], final['ledger']['grading'])
        for scope in ('product', 'experiment'):
            for key, value in EXPECTED['known_' + scope].items():
                self.assertEqual(value, final['ledger'][scope][key], (scope, key))
            for key, value in EXPECTED['full_scopes'].items():
                self.assertEqual(value, final['ledger'][scope][key], (scope, key))
        self.assertEqual(final, controller.finalize())

    def test_whole_envelope_budget_retains_drops_and_refuses(self):
        for budget, context in ((209, PUBLIC['contexts'][0]), (208, [])):
            controller, _ = self.create(reader_budget=budget)
            controller.query(0)
            captured = []
            controller.dispatch(controller.prepare_reader(0), lambda body: captured.append(body) or report())
            decoded = json.loads(captured[0])
            self.assertEqual(context, decoded['memory_context'])
            self.assertLessEqual(len(captured[0]), budget)
            controller.finalize()
        controller, _ = self.create(reader_budget=32)
        controller.query(0)
        with self.assertRaisesRegex(subject.ControllerError, 'controller_request_bound'):
            controller.prepare_reader(0)
        self.assertEqual(0, controller.snapshot()['callback_entries'])
        controller.finalize()

    def test_nonnull_question_image_refuses_before_producer_methods(self):
        memory = Memory()
        questions = copy.deepcopy(PUBLIC['questions'])
        questions[0]['query_image'] = 'PUBLIC_IMAGE_NOT_READ'
        with self.assertRaisesRegex(subject.ControllerError, 'controller_images_unsupported'):
            subject.Controller(memory, questions)
        self.assertEqual(0, memory.snapshot_calls)
        self.assertEqual(0, memory.close_calls)

    def test_context_images_refuse_and_preserve_paid_query_mapping(self):
        memory = Memory()
        memory.contexts[0] = [{'type': 'image', 'value': 'PUBLIC_IMAGE_NOT_READ'}]
        controller, _ = self.create(memory)
        with self.assertRaisesRegex(subject.ControllerError, 'controller_text_profile'):
            controller.query(0)
        self.assertEqual([0, None], controller.snapshot()['planned_to_local'])
        with self.assertRaises(subject.ControllerError):
            controller.prepare_reader(0)
        controller.finalize()

    def test_query_plan_caller_mutation_does_not_change_request(self):
        questions = copy.deepcopy(PUBLIC['questions'])
        controller = subject.Controller(Memory(), questions)
        questions[0]['question'] = 'PUBLIC_MUTATED_QUESTION'
        context = controller.query(0)
        context[0]['value'] = 'PUBLIC_MUTATED_CONTEXT'
        captured = []
        controller.dispatch(controller.prepare_reader(0), lambda body: captured.append(body) or report())
        self.assertEqual(EXPECTED['reader_requests'][0].encode(), captured[0])
        controller.finalize()

    def test_one_use_foreign_and_replayed_tickets_never_repeat_callback(self):
        controller, _ = self.create()
        other, _ = self.create()
        controller.query(0)
        ticket = controller.prepare_reader(0)
        seen = []
        with self.assertRaises(subject.ControllerError):
            other.dispatch(ticket, lambda body: seen.append(body))
        with self.assertRaises(subject.ControllerError):
            controller.dispatch(subject.DispatchTicket(), lambda body: seen.append(body))
        controller.dispatch(ticket, lambda body: seen.append(body) or report())
        before = controller.snapshot()
        with self.assertRaises(subject.ControllerError):
            controller.dispatch(ticket, lambda body: seen.append(body))
        self.assertEqual(1, len(seen))
        self.assertEqual(before, controller.snapshot())
        controller.finalize()
        other.finalize()

    def test_retry_requires_fresh_ticket_and_fixed_two_attempt_bound(self):
        controller, _ = self.create()
        controller.query(0)
        first = controller.prepare_reader(0)
        with self.assertRaises(subject.ControllerError):
            controller.prepare_reader(0)
        controller.dispatch(first, lambda body: report(None, outcome='failed'))
        second = controller.prepare_reader(0)
        self.assertIsNot(first, second)
        controller.dispatch(second, lambda body: report(None, outcome='failed'))
        with self.assertRaises(subject.ControllerError):
            controller.prepare_reader(0)
        self.assertEqual(2, controller.snapshot()['ledger']['attempts'])
        controller.finalize()

    def test_prework_gap_preserves_fixed_horizon_and_compact_mapping(self):
        controller, _ = self.create(Memory(['prework', 'success']))
        with self.assertRaises(RuntimeError):
            controller.query(0)
        controller.query(1)
        self.assertEqual([None, 0], controller.snapshot()['planned_to_local'])
        final = controller.finalize()
        self.assertEqual(2, final['ledger']['horizon'])
        self.assertIsNone(final['ledger']['phases']['query']['tokens'])
        self.assertEqual(18, final['ledger']['product']['known_component_ns'])

    def test_entered_native_cancel_preserves_original_error_and_next_ordinal(self):
        controller, _ = self.create(Memory(['cancel', 'success']))
        with self.assertRaises(KeyboardInterrupt):
            controller.query(0)
        controller.query(1)
        self.assertEqual([0, 1], controller.snapshot()['planned_to_local'])
        final = controller.finalize()
        self.assertEqual(21, final['ledger']['product']['known_component_ns'])

    def test_returned_warning_and_unmeasured_query_are_not_reader_success(self):
        for action, outcome in (('warning', 'failed'), ('unmeasured', 'unknown')):
            controller, _ = self.create(Memory([action, 'success']))
            with self.assertRaisesRegex(subject.ControllerError, 'controller_query_not_succeeded'):
                controller.query(0)
            with self.assertRaises(subject.ControllerError):
                controller.prepare_reader(0)
            seal = controller.seal_construction()
            self.assertEqual(outcome, json.loads(seal.body)['slots'][0]['query_outcome'])
            self.assertEqual([0, None], controller.snapshot()['planned_to_local'])
            controller.finalize()

    def test_external_insert_and_query_are_not_adopted(self):
        controller, memory = self.create()
        memory.insert_calls += 1
        with self.assertRaisesRegex(subject.ControllerError, 'controller_external_insert'):
            controller.query(0)
        final = controller.finalize()
        self.assertEqual(0, final['local_imports'])
        self.assertEqual(1, memory.close_calls)
        controller, memory = self.create()
        memory.query(PUBLIC['questions'][0]['question'], None)
        with self.assertRaisesRegex(subject.ControllerError, 'controller_external_query'):
            controller.query(0)
        final = controller.finalize()
        self.assertEqual(0, final['local_imports'])
        self.assertEqual(1, memory.close_calls)

    def test_contradictory_succeeded_unbound_receipt_cannot_open_reader(self):
        for modification in ({'native_metadata_bound': False}, {'state': 'refresh_failed'}):
            controller, memory = self.create()
            original = memory.post_query_hook
            def hook(**kwargs):
                memory.receipts[-1].update(modification)
                return original(**kwargs)
            memory.post_query_hook = hook
            with self.assertRaisesRegex(subject.ControllerError, 'controller_query_not_succeeded'):
                controller.query(0)
            with self.assertRaises(subject.ControllerError):
                controller.prepare_reader(0)
            seal = controller.seal_construction()
            self.assertEqual('unknown', json.loads(seal.body)['slots'][0]['query_outcome'])
            self.assertEqual(0, controller.snapshot()['callback_entries'])
            controller.finalize()

    def test_local_paired_byte_overflow_refuses_before_any_scalar_import(self):
        controller, memory = self.create()
        original = memory.post_query_hook
        def hook(**kwargs):
            memory.receipts[-1]['input_bytes'] = COUNTER_LIMIT
            memory.receipts[-1]['output_bytes'] = COUNTER_LIMIT
            return original(**kwargs)
        memory.post_query_hook = hook
        with self.assertRaisesRegex(subject.ControllerError, 'controller_local_receipt'):
            controller.query(0)
        final = controller.finalize()
        self.assertEqual(0, final['local_imports'])
        self.assertEqual(0, final['ledger']['phases']['build']['closed_units'])

    def test_entire_frozen_projection_is_validated_before_import(self):
        for corrupt in (lambda v: v['local_observation']['ledger'].update(frozen=False),
                        lambda v: v['local_observation']['observed']['query_receipts'][0].update(ordinal=9),
                        lambda v: v['local_observation']['observed']['query_receipts'][0].update(input_bytes=-1)):
            controller, memory = self.create()
            controller.query(0)
            memory.corrupt_final = corrupt
            with self.assertRaises(subject.ControllerError):
                controller.seal_construction()
            final = controller.finalize()
            self.assertEqual(0, final['local_imports'])
            self.assertEqual(0, final['ledger']['phases']['build']['closed_units'])

    def test_missing_usage_and_explicit_zero_never_complete_provider_coverage(self):
        for usage, known in ((None, 0), ({'input_tokens': 0, 'output_tokens': 0, 'total_tokens': 0}, 0),
                             ({'input_tokens': 7}, 7)):
            controller, _ = self.create()
            controller.query(0)
            self.reader(controller, 0, report(usage=usage))
            final = controller.finalize()
            self.assertEqual(known, final['ledger']['product']['known_input_tokens'])
            self.assertIsNone(final['ledger']['product']['tokens'])
            self.assertFalse(final['provider_usage_verified'])

    def test_default_unknown_local_profile_is_not_upgraded(self):
        controller, _ = self.create(Memory(profile='unknown'))
        for i in (0, 1):
            controller.query(i)
        final = controller.finalize()
        self.assertIsNone(final['ledger']['phases']['build']['tokens'])
        self.assertIsNone(final['ledger']['phases']['query']['tokens'])
        self.assertEqual(21, final['ledger']['product']['known_component_ns'])

    def test_reply_normalization_foreign_keys_refuse_without_equality_callbacks(self):
        calls = []
        class ForeignKey(str):
            __hash__ = str.__hash__
            def __eq__(self, other):
                calls.append(True)
                raise RuntimeError('PUBLIC_FOREIGN_KEY')
        controller, _ = self.create()
        controller.query(0)
        value = report()
        value.pop('usage')
        value[ForeignKey('usage')] = None
        with self.assertRaises(subject.ControllerError):
            controller.dispatch(controller.prepare_reader(0), lambda body: value)
        self.assertEqual([], calls)
        final = controller.finalize()
        self.assertEqual(1, final['ledger']['phases']['reader']['outcomes']['failed'])

    def test_reentrant_callback_cannot_mutate_or_seal_active_controller(self):
        controller, _ = self.create()
        controller.query(0)
        refusals = []
        def callback(body):
            for action in (lambda: controller.query(1), controller.seal_construction,
                           controller.finalize, lambda: controller.prepare_reader(0)):
                with self.assertRaises(subject.ControllerError):
                    action()
                refusals.append(True)
            return report()
        controller.dispatch(controller.prepare_reader(0), callback)
        self.assertEqual([True] * 4, refusals)
        controller.finalize()

    def test_callback_failure_cancel_and_unlaunched_ticket_remain_observed(self):
        for error, outcome in ((OSError('PUBLIC_CALLBACK_FAIL'), 'failed'),
                               (KeyboardInterrupt('PUBLIC_CALLBACK_CANCEL'), 'cancelled')):
            controller, _ = self.create()
            controller.query(0)
            def callback(body):
                raise error
            with self.assertRaises(type(error)):
                controller.dispatch(controller.prepare_reader(0), callback)
            final = controller.finalize()
            self.assertEqual(1, final['ledger']['phases']['reader']['outcomes'][outcome])
            self.assertNotIn('PUBLIC_CALLBACK_', json.dumps(final))
        controller, _ = self.create()
        controller.query(0)
        controller.prepare_reader(0)
        final = controller.finalize()
        self.assertEqual(1, final['pending_tickets'])
        self.assertEqual(0, final['callback_entries'])
        self.assertIsNone(final['ledger']['product']['tokens'])

    def test_clock_start_cancel_books_reservation_but_zero_callback_entry(self):
        controller, _ = self.create(clock=Clock([KeyboardInterrupt('PUBLIC_START_CLOCK'), 5]))
        controller.query(0)
        entered = []
        with self.assertRaises(KeyboardInterrupt):
            controller.dispatch(controller.prepare_reader(0), lambda body: entered.append(body) or report())
        final = controller.finalize()
        self.assertEqual([], entered)
        self.assertEqual(0, final['callback_entries'])
        self.assertEqual(1, final['ledger']['attempts'])
        self.assertEqual(1, final['ledger']['phases']['reader']['outcomes']['cancelled'])

    def test_clock_end_cancel_retains_paid_report_and_releases_busy(self):
        controller, _ = self.create(clock=Clock([0, KeyboardInterrupt('PUBLIC_END_CLOCK')]))
        controller.query(0)
        with self.assertRaises(KeyboardInterrupt):
            controller.dispatch(controller.prepare_reader(0), lambda body: copy.deepcopy(PUBLIC['reader_reports'][1]))
        final = controller.finalize()
        self.assertEqual(10, final['ledger']['product']['known_tokens'])
        self.assertEqual(7, final['ledger']['product']['known_usd_micros'])
        self.assertEqual(1, final['callback_entries'])
        self.assertEqual(1, final['ledger']['phases']['reader']['finished_attempts'])

    def test_cleanup_cancel_never_issues_judge_gate_and_preserves_local_cost(self):
        for error in (KeyboardInterrupt('PUBLIC_CLEANUP_CANCEL'), SystemExit('PUBLIC_CLEANUP_EXIT')):
            controller, memory = self.create()
            controller.query(0)
            self.reader(controller, 0)
            memory.close_error = error
            with self.assertRaises(type(error)):
                controller.seal_construction()
            with self.assertRaises(subject.ControllerError):
                controller.prepare_judge(0, subject.Construction(b'{}'))
            final = controller.finalize()
            self.assertTrue(final['construction_blocked'])
            self.assertIsNone(final['construction_sha256'])
            self.assertEqual(1, memory.close_calls)
            self.assertEqual(16, final['ledger']['product']['known_component_ns'] - final['ledger']['phases']['reader']['known_component_ns'])

    def test_finalize_propagates_cleanup_cancel_after_freezing(self):
        controller, memory = self.create()
        memory.close_error = KeyboardInterrupt('PUBLIC_FINALIZE_CANCEL')
        with self.assertRaises(KeyboardInterrupt):
            controller.finalize()
        self.assertTrue(controller.snapshot()['closed'])
        self.assertTrue(controller.snapshot()['ledger']['frozen'])
        self.assertEqual(controller.snapshot(), controller.finalize())

    def test_ordinary_cleanup_failure_is_recorded_without_invented_cause(self):
        controller, memory = self.create()
        controller.query(0)
        memory.close_error = OSError('PUBLIC_CLEANUP_FAIL')
        seal = controller.seal_construction()
        self.assertIsInstance(seal, subject.Construction)
        final = controller.finalize()
        self.assertEqual(1, final['local_imports'])
        self.assertGreater(final['observation_faults'], 0)
        self.assertNotIn('PUBLIC_CLEANUP_FAIL', json.dumps(final))

    def test_owned_immutable_gate_is_required_after_native_teardown(self):
        controller, memory = self.create()
        controller.query(0)
        self.reader(controller, 0)
        with self.assertRaises(subject.ControllerError):
            controller.prepare_judge(0, True)
        seal = controller.seal_construction()
        self.assertTrue(memory.closed)
        self.assertFalse(hasattr(seal, '__dict__'))
        with self.assertRaises(dataclasses.FrozenInstanceError):
            seal.body = b'{}'
        with self.assertRaises(subject.ControllerError):
            controller.prepare_judge(0, subject.Construction(seal.body))
        controller.finalize()

    def test_failed_judge_retry_records_one_final_grade(self):
        controller, _ = self.create()
        controller.query(0)
        self.reader(controller, 0)
        seal = controller.seal_construction()
        controller.dispatch(controller.prepare_judge(0, seal), lambda: lambda body: report(None, outcome='failed'))
        controller.dispatch(controller.prepare_judge(0, seal), lambda: lambda body: report(True))
        final = controller.finalize()
        self.assertEqual(2, final['ledger']['phases']['judge']['attempts'])
        self.assertEqual(1, final['ledger']['grading']['known_correct'])
        self.assertEqual(0, final['ledger']['grading']['conflicting'])
        self.assertFalse(final['ledger']['grading']['complete'])

    def test_construction_copy_is_charged_to_aggregate_held_bound(self):
        controller, _ = self.create()
        controller.query(0)
        self.reader(controller, 0)
        oracle = json.loads((ROOT / 'tests/fixtures/longmemeval_lifecycle_aggregate.json').read_text())
        self.assertEqual(oracle['held_before_seal'], controller._held)
        with patch.object(subject, 'MAX_HELD', oracle['test_cap']):
            with self.assertRaisesRegex(subject.ControllerError, 'controller_held_bound'):
                controller.seal_construction()
        self.assertIsNone(controller.snapshot()['construction_sha256'])
        controller.finalize()

    def test_frozen_metadata_contains_no_queries_answers_or_error_prose(self):
        controller, _ = self.create()
        controller.query(0)
        self.reader(controller, 0, report('PUBLIC_ANSWER_PRIVACY_SENTINEL'))
        final = controller.finalize()
        encoded = json.dumps(final)
        for value in (PUBLIC['questions'][0]['question'], PUBLIC['contexts'][0][0]['value'],
                      'PUBLIC_ANSWER_PRIVACY_SENTINEL'):
            self.assertNotIn(value, encoded)
        final['ledger']['product']['known_tokens'] = 99
        self.assertNotEqual(final, controller.snapshot())

    def test_actual_registered_native_maple_cycle_seals_before_public_evaluator(self):
        edition = ROOT / 'PHP Core'
        sys.path.insert(0, str(edition / 'memory-bank/scripts'))
        import context as native
        raw = (ROOT / 'tests/fixtures/longmemeval_memory_api_2cc8c540.py').read_bytes()
        self.assertEqual('607ca21320067df2abc3583b80debeb819781e90955776bdea4313a8b93a0586', hashlib.sha256(raw).hexdigest())
        marker = b'from .no_retrieval import NoRetrievalMemory  # noqa: E402,F401\n'
        prefix = raw[:raw.index(marker)]
        self.assertEqual('6bdc677c32b42ec6d7e142e07213adbcfb21403804aac16e2a2d810319756025', hashlib.sha256(prefix).hexdigest())
        api = types.ModuleType('new_maple_selected_memory_api')
        # This deferred-annotation test profile does not change the pinned
        # source bytes or certify the complete official benchmark runtime.
        exec(compile(prefix, 'new_maple_unchanged_api_prefix.py', 'exec',
                     flags=__future__.annotations.compiler_flag, dont_inherit=True), api.__dict__)
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary)
            for name in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/templates',
                         'project-brain/config', 'project-brain/schemas', 'project-brain/indexes'):
                shutil.copytree(edition / name, project / name, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
            for name in ('memory-bank/README.md', 'memory-bank/INDEX.md', 'project-brain/README.md', 'project-brain/PROTOCOL.md'):
                shutil.copyfile(edition / name, project / name)
            (project / 'memory-bank/chunks').mkdir()
            (project / 'specs').mkdir()
            register_bridge(api, lambda: (project, native))
            memory = api.build_memory({'memory_type': 'native_fts5_states_v1', 'memory_params': {'horizon': 2}})
            memory.insert(copy.deepcopy(PUBLIC['native_trajectory']))
            controller = subject.Controller(memory, copy.deepcopy(PUBLIC['questions']), reader_budget=4096)
            entered, gold_reads = [], []
            try:
                contexts = [controller.query(i) for i in (0, 1)]
                for i in (0, 1):
                    def reader(body, i=i):
                        entered.append(body)
                        return report(('17 seconds', 'Unknown; no evidence.')[i])
                    controller.dispatch(controller.prepare_reader(i), reader)
                seal = controller.seal_construction()
                stored = project / 'fresh-maple-construction.json'
                stored.write_bytes(seal.body)
                # Public evaluator/body checks occur after construction persistence.
                self.assertEqual(EXPECTED['native_document']['text'].encode(), next((project / 'specs/longmemeval').glob('*.md')).read_bytes())
                self.assertIn('Maple buoy emits every 17 seconds.', contexts[0][0]['value'])
                self.assertEqual([], contexts[1])
                for i in (0, 1):
                    self.assertEqual(contexts[i], json.loads(entered[i])['memory_context'])
                    def factory(i=i):
                        self.assertEqual(seal.sha256, hashlib.sha256(stored.read_bytes()).hexdigest())
                        grades = json.loads((ROOT / 'tests/fixtures/longmemeval_lifecycle_public_grades.json').read_text())
                        gold_reads.append(i)
                        return lambda body: report(grades[str(i)])
                    controller.dispatch(controller.prepare_judge(i, seal), factory)
                self.assertEqual([0, 1], gold_reads)
            finally:
                final = controller.finalize()
            self.assertEqual([0, 1], final['planned_to_local'])
            self.assertEqual(4, final['callback_entries'])
            self.assertEqual(1, final['local_imports'])
            self.assertIsNone(final['ledger']['product']['tokens'])
            self.assertIsNone(final['ledger']['experiment']['usd_micros'])
            self.assertFalse(final['provider_usage_verified'])
            self.assertEqual(0, final['observation_faults'])
if __name__ == '__main__':
    unittest.main()
