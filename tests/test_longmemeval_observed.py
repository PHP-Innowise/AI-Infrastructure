"""NEW public local-cost producer controls; no old suites, models or gold."""
import copy
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
EDITION = ROOT / 'PHP Core'
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import longmemeval_observed as subject

PUBLIC = json.loads((ROOT / 'tests/fixtures/longmemeval_observed_public.json').read_text())
EXPECTED = json.loads((ROOT / 'tests/fixtures/longmemeval_observed_expected.json').read_text())
ADVERSARIAL = json.loads((ROOT / 'tests/fixtures/longmemeval_observed_adversarial.json').read_text())


class Clock:
    def __init__(self):
        self.value = 0

    def __call__(self):
        self.value += 100
        return self.value


class Delegate:
    """Controlled owned backend, separate from the actual native smoke method."""
    def __init__(self, _repository, _native):
        self.counter = 0
        self.inputs = {}
        self.last = None
        self.actions = ['delivered', 'no_evidence']
        self.hook_mode = 'normal'
        self.close_error = None
        self.insert_error = None
        self.context = copy.deepcopy(PUBLIC['controlled_context'])
        self.closed = 0

    def insert(self, trajectory):
        if self.insert_error is not None:
            raise self.insert_error
        old = self.inputs.get(trajectory['id'])
        if old is not None and old != trajectory:
            raise ValueError('PUBLIC_BACKEND_ERROR_SENTINEL')
        self.inputs[trajectory['id']] = copy.deepcopy(trajectory)

    def query(self, query, query_image=None):
        self.counter += 1
        action = self.actions.pop(0) if self.actions else 'delivered'
        self.last = {'queries': self.counter, 'state': action,
                     'query_image_ignored': query_image is not None,
                     'private': 'PUBLIC_CALLBACK_SENTINEL', 'query': query}
        if action == 'cancelled':
            self.last['state'] = 'failed'
            raise KeyboardInterrupt('PUBLIC_CANCEL_SENTINEL')
        if action == 'cleanup_failed':
            raise OSError('PUBLIC_CLEANUP_SENTINEL')
        if action == 'failed':
            raise RuntimeError('PUBLIC_ERROR_SENTINEL')
        return self.context if action == 'delivered' else []

    def post_query_hook(self):
        if self.hook_mode == 'throws':
            raise ValueError('PUBLIC_HOOK_SENTINEL')
        value = copy.deepcopy(self.last)
        if self.hook_mode == 'missing_queries':
            value.pop('queries')
        elif self.hook_mode == 'unchanged_counter':
            value['queries'] -= 1
        elif self.hook_mode == 'unknown_state':
            value['state'] = 'PUBLIC_HOOK_SENTINEL'
        return value

    def close(self):
        self.closed += 1
        if self.close_error is not None:
            raise self.close_error


def install(project):
    for relative in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/templates',
                     'project-brain/config', 'project-brain/schemas', 'project-brain/indexes'):
        shutil.copytree(EDITION / relative, project / relative,
                        ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    for relative in ('memory-bank/README.md', 'memory-bank/INDEX.md',
                     'project-brain/README.md', 'project-brain/PROTOCOL.md'):
        shutil.copyfile(EDITION / relative, project / relative)
    (project / 'memory-bank/chunks').mkdir()
    (project / 'specs').mkdir()


class ObservedContract(unittest.TestCase):
    def create(self, horizon=2, profile='pinned_local_no_model', clock=None):
        memory = subject.ObservedProjectionMemory(horizon, local_token_profile=profile,
                                                  clock=Clock() if clock is None else clock)
        with patch.object(subject.projection, 'NativeProjectionMemory', Delegate):
            memory.open('unused-owned-test-root', object())
        return memory

    def query_public(self, memory, ordinal):
        value = PUBLIC['queries'][ordinal]
        return memory.query(value['text'], value['query_image'])

    def test_whole_new_literal_observation_and_ledger_oracles(self):
        memory = self.create()
        memory.insert(PUBLIC['trajectory'])
        memory.insert(PUBLIC['trajectory'])
        self.assertEqual(PUBLIC['controlled_context'], self.query_public(memory, 0))
        self.assertEqual([], self.query_public(memory, 1))
        result = memory.close()
        self.assertEqual(EXPECTED['success_control'], result['observed'])
        ledger = result['ledger']
        oracle = EXPECTED['ledger_contract']
        self.assertEqual(oracle['horizon'], ledger['horizon'])
        self.assertEqual(oracle['physical_provider_attempts'], ledger['attempts'])
        for phase in ('build', 'query'):
            actual = ledger['phases'][phase]
            projection = {key: actual[key] for key in oracle[phase] if key != 'billing_complete'}
            projection['billing_complete'] = actual['complete']['billing']
            self.assertEqual(oracle[phase], projection)
        for phase in ('reader', 'judge'):
            actual = ledger['phases'][phase]
            self.assertEqual(oracle[phase], {'closed_units': actual['closed_units'],
                'tokens': actual['tokens'], 'complete_coverage': actual['complete']['coverage']})
        self.assertEqual(oracle['product'], {key: ledger['product'][key] for key in oracle['product']})
        self.assertEqual(oracle['grading'], ledger['grading'])
        document = subject.projection.project_trajectory(PUBLIC['trajectory'])
        self.assertEqual(EXPECTED['document'], {'text': document.body.decode(),
            'sha256': document.sha256, 'bytes': len(document.body)})

    def test_unknown_profile_never_infers_free_work_or_reader_cost(self):
        memory = self.create(profile='unknown')
        memory.insert(PUBLIC['trajectory'])
        self.query_public(memory, 0)
        result = memory.close()
        for phase in ('build', 'query', 'reader', 'judge'):
            self.assertIsNone(result['ledger']['phases'][phase]['tokens'])
            self.assertIsNone(result['ledger']['phases'][phase]['usd_micros'])
        self.assertEqual(0, result['ledger']['attempts'])
        self.assertEqual(1, result['observed']['unlaunched_queries'])
        self.assertIsNone(result['ledger']['product']['tokens_per_correct'])

    def test_failed_second_input_has_full_pinned_receipt_without_stale_hook(self):
        memory = self.create()
        self.query_public(memory, 0)
        bad = ADVERSARIAL['failure_second_query']
        with self.assertRaises(subject.projection.ProjectionError):
            memory.query(bad['text'], bad['query_image'])
        receipt = memory.query_receipt(1)
        self.assertEqual(EXPECTED['failure_control']['query_receipt'], receipt)
        result = memory.close()['observed']
        for key, value in EXPECTED['failure_control'].items():
            if key != 'query_receipt':
                self.assertEqual(value, result[key])
        self.assertEqual(1, memory._backend.counter)

    def test_invalid_first_ordinal_does_not_advance_native_counter(self):
        memory = self.create()
        with self.assertRaises(subject.projection.ProjectionError):
            memory.query(17)
        self.query_public(memory, 0)
        self.assertEqual(1, memory._backend.counter)
        self.assertTrue(memory.query_receipt(1)['native_metadata_bound'])
        self.assertFalse(memory.query_receipt(0)['native_metadata_bound'])
        memory.close()

    def test_empty_failure_is_distinct_from_successful_no_evidence(self):
        memory = self.create()
        memory._backend.actions = ['refresh_failed', 'no_evidence']
        self.assertEqual([], self.query_public(memory, 0))
        self.assertEqual([], self.query_public(memory, 1))
        view = memory.close()['observed']
        self.assertEqual(['failed', 'succeeded'], [row['outcome'] for row in view['query_receipts']])
        self.assertEqual(1, view['query_failed'])
        self.assertEqual(4, view['returned_context_bytes'])

    def test_unknown_hooks_do_not_retain_callback_prose_or_assume_success(self):
        for mode in ('throws', 'missing_queries', 'unchanged_counter', 'unknown_state'):
            with self.subTest(mode=mode):
                memory = self.create(horizon=1)
                memory._backend.hook_mode = mode
                self.assertEqual(PUBLIC['controlled_context'], self.query_public(memory, 0))
                result = memory.close()
                receipt = result['observed']['query_receipts'][0]
                self.assertEqual('unknown', receipt['outcome'])
                self.assertFalse(receipt['native_metadata_bound'])
                self.assertTrue(result['observed']['observation_incomplete'])
                serialized = json.dumps(result)
                for value in ('PUBLIC_CALLBACK_SENTINEL', 'PUBLIC_HOOK_SENTINEL',
                              PUBLIC['queries'][0]['text'], PUBLIC['trajectory']['id']):
                    self.assertNotIn(value, serialized)

    def test_array_serialization_bound_charges_ingress_without_egress(self):
        memory = self.create(horizon=1)
        memory._backend.context = [{'type': 'text', 'value': '\n' * 131073}]
        with self.assertRaisesRegex(subject.ObservationError, '^context_bound$'):
            self.query_public(memory, 0)
        receipt = memory.query_receipt(0)
        self.assertEqual(0, receipt['output_bytes'])
        self.assertEqual(EXPECTED['captured']['query_bytes'][0], receipt['input_bytes'])
        self.assertEqual('failed', receipt['outcome'])
        memory.close()

    def test_lifecycle_and_horizon_refusals_do_not_mutate_snapshots(self):
        memory = subject.ObservedProjectionMemory(1, clock=Clock())
        before = memory.snapshot()
        for call in (lambda: memory.query('x'), lambda: memory.insert(PUBLIC['trajectory'])):
            with self.assertRaises(subject.ObservationError):
                call()
            self.assertEqual(before, memory.snapshot())
        with patch.object(subject.projection, 'NativeProjectionMemory', Delegate):
            memory.open('owned-test', object())
        self.query_public(memory, 0)
        before = memory.snapshot()
        for call in (lambda: memory.query('x'), lambda: memory.insert(PUBLIC['trajectory']),
                     lambda: memory.open('another', object())):
            with self.assertRaises(subject.ObservationError):
                call()
            self.assertEqual(before, memory.snapshot())
        frozen = memory.close()
        with self.assertRaises(subject.ObservationError):
            memory.query('x')
        self.assertEqual(frozen, memory.close())
        self.assertEqual(1, memory._backend.closed)

    def test_owned_input_egress_receipts_and_final_snapshots_are_independent(self):
        memory = self.create()
        original = copy.deepcopy(PUBLIC['trajectory'])
        memory.insert(original)
        original['states'][0]['accessibility_tree'] = 'CALLER_CHANGED'
        result = self.query_public(memory, 0)
        result[0]['value'] = 'CALLER_CHANGED'
        self.assertNotIn('CALLER_CHANGED', memory._backend.inputs[PUBLIC['trajectory']['id']]['states'][0]['accessibility_tree'])
        first = memory.query_receipt(0)
        first['state'] = 'CALLER_CHANGED'
        self.query_public(memory, 1)
        self.assertEqual('delivered', memory.query_receipt(0)['state'])
        final = memory.close()
        final['observed']['query_receipts'].clear()
        final['ledger']['phases']['build']['tokens'] = 999
        self.assertEqual(2, memory.snapshot()['observed']['query_calls'])
        self.assertEqual(0, memory.snapshot()['ledger']['phases']['build']['tokens'])

    def test_conflicting_insert_remains_charged_without_adoption(self):
        memory = self.create()
        memory.insert(PUBLIC['trajectory'])
        changed = copy.deepcopy(PUBLIC['trajectory'])
        changed['states'][0]['accessibility_tree'] = ADVERSARIAL['changed_trajectory_text']
        with self.assertRaises(ValueError):
            memory.insert(changed)
        result = memory.close()['observed']
        wire = lambda value: json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
        self.assertEqual(len(wire(PUBLIC['trajectory'])) + len(wire(changed)), result['insert_ingress_bytes'])
        self.assertEqual(1, result['inserted'])
        self.assertEqual(1, result['insert_failed'])
        self.assertEqual(EXPECTED['document']['bytes'], result['unique_projection_bytes'])

    def test_initialization_failure_is_observed_and_cannot_reopen(self):
        memory = subject.ObservedProjectionMemory(2, clock=Clock())
        with patch.object(subject.projection, 'NativeProjectionMemory', side_effect=OSError('PUBLIC_INIT_SENTINEL')):
            with self.assertRaises(OSError):
                memory.open('owned', object())
        with self.assertRaises(subject.ObservationError):
            memory.open('replacement', object())
        view = memory.close()
        self.assertEqual('failed', view['observed']['initialization_outcome'])
        self.assertEqual('not_needed', view['observed']['cleanup_outcome'])
        self.assertEqual(2, view['observed']['unlaunched_queries'])
        self.assertFalse(view['ledger']['phases']['build']['complete']['coverage'])
        self.assertNotIn('PUBLIC_INIT_SENTINEL', json.dumps(view))

    def test_query_cleanup_error_and_cancellation_retain_charged_spans(self):
        memory = self.create(horizon=3)
        memory._backend.actions = ['cleanup_failed', 'cancelled']
        with self.assertRaises(OSError):
            self.query_public(memory, 0)
        with self.assertRaises(KeyboardInterrupt):
            self.query_public(memory, 1)
        result = memory.close()
        view = result['observed']
        self.assertEqual(200, view['query_component_ns'])
        self.assertEqual(140, view['query_ingress_bytes'])
        self.assertEqual(0, view['returned_context_bytes'])
        self.assertEqual((1, 1, 1), (view['query_failed'], view['query_cancelled'], view['unlaunched_queries']))
        self.assertEqual(['cleanup_failed', 'cancelled'], [r['state'] for r in view['query_receipts']])
        self.assertIsNone(result['ledger']['phases']['query']['tokens'])
        self.assertNotIn('PUBLIC_CANCEL_SENTINEL', json.dumps(result))

    def test_teardown_failure_freezes_owned_metadata_without_retry(self):
        memory = self.create(horizon=1)
        self.query_public(memory, 0)
        memory._backend.close_error = OSError('PUBLIC_CLOSE_SENTINEL')
        with self.assertRaises(OSError):
            memory.close()
        result = memory.snapshot()
        self.assertEqual('cleanup_failed', result['observed']['status'])
        self.assertTrue(result['ledger']['frozen'])
        self.assertEqual(200, result['observed']['build_component_ns'])
        self.assertEqual(result, memory.close())
        self.assertEqual(1, memory._backend.closed)

    def test_clock_faults_preserve_results_with_unknown_timing(self):
        for values in ([True] * 8, [200, 100, 400, 300, 600, 500], [None] * 8):
            with self.subTest(values=values):
                iterator = iter(values)
                memory = self.create(horizon=1, clock=lambda: next(iterator))
                result = self.query_public(memory, 0)
                self.assertEqual(PUBLIC['controlled_context'], result)
                summary = memory.close()
                self.assertIsNone(summary['observed']['build_component_ns'])
                self.assertIsNone(summary['observed']['query_receipts'][0]['component_ns'])
                self.assertTrue(summary['observed']['observation_incomplete'])
                self.assertIsNone(summary['ledger']['phases']['query']['component_ns'])
                self.assertEqual(0, summary['ledger']['phases']['query']['tokens'])

    def test_evaluator_fields_are_refused_and_never_exported(self):
        memory = self.create()
        changed = copy.deepcopy(PUBLIC['trajectory'])
        changed.update(ADVERSARIAL['rejected_trajectory_extra'])
        with self.assertRaises(subject.projection.ProjectionError):
            memory.insert(changed)
        result = memory.close()
        self.assertEqual(0, result['observed']['inserted'])
        self.assertEqual({}, memory._backend.inputs)
        self.assertNotIn('PUBLIC_EVALUATOR_SENTINEL', json.dumps(result))

    def test_insert_capacity_refuses_before_backend_and_timing(self):
        memory = self.create()
        with patch.object(subject, 'MAX_INSERT_CALLS', 1):
            memory.insert(PUBLIC['trajectory'])
            before = memory.snapshot()
            with self.assertRaisesRegex(subject.ObservationError, '^insert_capacity$'):
                memory.insert(PUBLIC['trajectory'])
            self.assertEqual(before, memory.snapshot())
        memory.close()

    def test_throwing_clock_does_not_override_native_results_or_leak_prose(self):
        def broken():
            raise RuntimeError('PUBLIC_CLOCK_SENTINEL')
        memory = self.create(horizon=1, clock=broken)
        self.assertEqual(PUBLIC['controlled_context'], self.query_public(memory, 0))
        result = memory.close()
        self.assertIsNone(result['observed']['build_component_ns'])
        self.assertIsNone(result['observed']['query_receipts'][0]['component_ns'])
        self.assertTrue(result['observed']['observation_incomplete'])
        self.assertNotIn('PUBLIC_CLOCK_SENTINEL', json.dumps(result))

    def test_insert_error_and_cancellation_are_charged_without_fresh_records(self):
        for failure in (OSError('PUBLIC_INSERT_SENTINEL'), KeyboardInterrupt('PUBLIC_INSERT_CANCEL')):
            with self.subTest(kind=type(failure).__name__):
                memory = self.create()
                memory._backend.insert_error = failure
                with self.assertRaises(type(failure)):
                    memory.insert(PUBLIC['trajectory'])
                result = memory.close()['observed']
                self.assertEqual(EXPECTED['captured']['insert_bytes'], result['insert_ingress_bytes'])
                self.assertEqual(300, result['build_component_ns'])
                self.assertEqual(0, result['inserted'])
                self.assertEqual(0, result['unique_projection_bytes'])
                self.assertEqual(1, result['insert_failed'] + result['insert_cancelled'])
                self.assertNotIn('PUBLIC_INSERT_', json.dumps(result))

    def test_invalid_context_shapes_are_not_returned_as_success(self):
        for context in ('PUBLIC_CONTEXT_SENTINEL', [{'type': 'text', 'value': 'x', 'answer': 'PUBLIC_EXTRA'}]):
            with self.subTest(context_type=type(context).__name__):
                memory = self.create(horizon=1)
                memory._backend.context = context
                with self.assertRaisesRegex(subject.ObservationError, '^context_schema$'):
                    self.query_public(memory, 0)
                receipt = memory.query_receipt(0)
                self.assertEqual(('failed', 0), (receipt['outcome'], receipt['output_bytes']))
                self.assertNotIn('PUBLIC_CONTEXT_SENTINEL', json.dumps(memory.close()))

    def test_reentrant_backend_mutations_refuse_before_ordinal_allocation(self):
        memory = self.create(horizon=1)
        original = memory._backend.post_query_hook
        refusals = []
        def callback():
            for action in (lambda: memory.query('nested'), memory.close):
                try:
                    action()
                except subject.ObservationError:
                    refusals.append(True)
            return original()
        memory._backend.post_query_hook = callback
        self.assertEqual(PUBLIC['controlled_context'], self.query_public(memory, 0))
        result = memory.close()
        self.assertEqual([True, True], refusals)
        self.assertEqual(1, result['observed']['query_calls'])
        self.assertEqual(0, result['observed']['query_receipts'][0]['ordinal'])
        self.assertTrue(result['observed']['query_receipts'][0]['native_metadata_bound'])

    def test_start_and_end_clock_interruptions_record_cancellation_then_cleanup(self):
        for interrupt_at, native_calls, ingress in ((3, 0, 0), (4, 1, 57)):
            with self.subTest(clock_interrupt_at=interrupt_at):
                calls = [0]
                def interrupting_clock():
                    calls[0] += 1
                    if calls[0] == interrupt_at:
                        raise KeyboardInterrupt('PUBLIC_CLOCK_CANCEL_SENTINEL')
                    return calls[0] * 100
                memory = self.create(horizon=1, clock=interrupting_clock)
                with self.assertRaises(KeyboardInterrupt):
                    self.query_public(memory, 0)
                self.assertFalse(memory._active)
                self.assertEqual(native_calls, memory._backend.counter)
                receipt = memory.query_receipt(0)
                self.assertEqual(('cancelled', 'cancelled', ingress, 0, None),
                    (receipt['state'], receipt['outcome'], receipt['input_bytes'],
                     receipt['output_bytes'], receipt['component_ns']))
                result = memory.close()
                self.assertEqual(1, memory._backend.closed)
                self.assertTrue(result['ledger']['frozen'])
                self.assertNotIn('PUBLIC_CLOCK_CANCEL_SENTINEL', json.dumps(result))

    def test_two_local_threads_keep_query_hook_receipts_separate(self):
        memory = self.create()
        first_entered = threading.Event()
        second_requested = threading.Event()
        release_first = threading.Event()
        original = memory._backend.query
        outputs, errors = [], []
        def controlled(query, query_image=None):
            if query == PUBLIC['queries'][0]['text']:
                first_entered.set()
                if not release_first.wait(2):
                    raise RuntimeError('test_controller_timeout')
            return original(query, query_image)
        memory._backend.query = controlled
        def run(ordinal):
            try:
                if ordinal == 1:
                    second_requested.set()
                outputs.append((ordinal, self.query_public(memory, ordinal)))
            except BaseException as error:
                errors.append(type(error).__name__)
        workers = [threading.Thread(target=run, args=(ordinal,), daemon=True) for ordinal in (0, 1)]
        try:
            workers[0].start()
            self.assertTrue(first_entered.wait(2))
            workers[1].start()
            self.assertTrue(second_requested.wait(2))
        finally:
            release_first.set()
            for worker in workers:
                if worker.ident is not None:
                    worker.join(3)
        self.assertTrue(all(not worker.is_alive() for worker in workers))
        self.assertEqual([], errors)
        self.assertEqual([(0, PUBLIC['controlled_context']), (1, [])], sorted(outputs))
        result = memory.close()
        self.assertEqual([0, 1], [r['ordinal'] for r in result['observed']['query_receipts']])
        self.assertEqual(['delivered', 'no_evidence'], [r['state'] for r in result['observed']['query_receipts']])
        self.assertTrue(all(r['native_metadata_bound'] for r in result['observed']['query_receipts']))

    def test_actual_native_new_corpus_constructs_before_source_only_evaluation(self):
        sys.path.insert(0, str(EDITION / 'memory-bank/scripts'))
        import context as native
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary)
            install(project)
            memory = subject.ObservedProjectionMemory(2, local_token_profile='pinned_local_no_model')
            memory.open(project, native)
            try:
                memory.insert(copy.deepcopy(PUBLIC['trajectory']))
                outputs = [self.query_public(memory, ordinal) for ordinal in range(2)]
            finally:
                summary = memory.close()
            # NEW public construction is physically sealed before support grading.
            envelope = {'profile': subject.PROFILE, 'outputs': outputs, 'summary': summary}
            raw = json.dumps(envelope, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
            constructed = project / 'public-construction.json'
            constructed.write_bytes(raw)
            digest = hashlib.sha256(raw).hexdigest()
            seal = {'sha256': digest, 'bytes': len(raw), 'builder_or_retriever_gold_reads': 0,
                    'profile': 'NEW-public-text-only-native-construction'}
            (project / 'public-construction-seal.json').write_text(json.dumps(seal))
            # Logical evaluator/controller boundary, not an OS sandbox.
            self.assertEqual(digest, hashlib.sha256(constructed.read_bytes()).hexdigest())
            source = project / 'specs/longmemeval'
            published = list(source.glob('*.md'))
            self.assertEqual(1, len(published))
            self.assertEqual(EXPECTED['document']['text'].encode(), published[0].read_bytes())
            text = '\n'.join(item['value'] for item in outputs[0])
            self.assertIn('Saffron beacon retry delay is 23 seconds.', text)
            self.assertIn('reference data', text.lower())
            self.assertEqual([], outputs[1])
            self.assertEqual(2, summary['observed']['query_calls'])
            self.assertEqual(['delivered', 'no_evidence'],
                             [r['state'] for r in summary['observed']['query_receipts']])
            self.assertEqual(['succeeded', 'succeeded'],
                             [r['outcome'] for r in summary['observed']['query_receipts']])
            self.assertTrue(all(r['native_metadata_bound'] for r in summary['observed']['query_receipts']))
            self.assertEqual(0, summary['observed']['query_failed'])
            self.assertEqual(0, summary['observed']['query_cancelled'])
            self.assertFalse(summary['observed']['observation_incomplete'])
            self.assertTrue(summary['observed']['query_receipts'][1]['query_image_ignored'])
            self.assertEqual(0, summary['observed']['images_ingested'])
            self.assertEqual(0, summary['ledger']['phases']['query']['tokens'])
            self.assertIsNone(summary['ledger']['product']['tokens'])
            self.assertIsNone(summary['ledger']['product']['usd_micros'])
            self.assertIsNone(summary['ledger']['product']['tokens_per_correct'])
            self.assertTrue(all(type(r['component_ns']) is int and r['component_ns'] >= 0
                                for r in summary['observed']['query_receipts']))
            self.assertFalse((project / 'public-images').exists())
            self.assertTrue((project / 'memory-bank/local/context.db').exists())


if __name__ == '__main__':
    unittest.main()
