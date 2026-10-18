"""Fresh trusted-callback type controls, not native/provider/actor coverage."""
import copy
import json
from pathlib import Path
import unittest

from harness import longmemeval_lifecycle as lifecycle

FIXTURE = Path(__file__).parent / 'fixtures/longmemeval_actual_error_type/controls.json'


class ActualErrorTypeTests(unittest.TestCase):
    def setUp(self):
        self.public = json.loads(FIXTURE.read_text())

    def _guard(self, callback, code):
        seen = None
        try:
            callback()
        except BaseException as error:
            seen = error
        self.assertIs(type(seen), lifecycle.ControllerError)
        self.assertEqual(seen.args, (code,))

    def _case(self, name):
        case = next(row for row in self.public['cases'] if row['name'] == name)
        touches, trace, clocks, callback_bodies = [], [], [], []

        class SpoofAbort(BaseException):
            @property
            def __class__(self):
                touches.append('spoof_class')
                return Exception

        class ThrowAbort(BaseException):
            @property
            def __class__(self):
                touches.append('throw_class')
                raise LookupError('class property must not run')

        class OrdinaryProbe(Exception):
            @property
            def __class__(self):
                touches.append('ordinary_class')
                raise LookupError('ordinary class property must not run')

        error_type = {'spoof_abort': SpoofAbort, 'throw_abort': ThrowAbort,
                      'ordinary_probe': OrdinaryProbe}[case['kind']]
        primary = error_type('new public type control')
        public = self.public
        test = self

        class SuppliedMemory:
            def __init__(self):
                self.view = copy.deepcopy(case['initial_memory'])
                self.close_calls = self.context_calls = 0

            def snapshot(self):
                trace.append('snapshot')
                return copy.deepcopy(self.view)

            def set_query_context(self, *, query_invocation_id):
                test.assertIs(type(query_invocation_id), str)
                test.assertTrue(query_invocation_id.startswith('controller-'))
                test.assertEqual(len(query_invocation_id), 43)
                self.context_calls += 1
                trace.append('set_context')

            def clear_query_context(self):
                trace.append('clear_context')

            def query(self, question, image):
                test.assertEqual(question, public['questions'][0]['question'])
                test.assertIsNone(image)
                trace.append('query')
                if case['stage'] == 'query':
                    raise primary
                view = self.view['local_observation']['observed']
                view['query_calls'] = 1
                view['query_receipts'] = [copy.deepcopy(public['receipt'])]
                return []

            def post_query_hook(self, *, query, query_image, memory_context):
                test.assertEqual(query, public['questions'][0]['question'])
                test.assertIsNone(query_image)
                test.assertEqual(memory_context, [])
                trace.append('post_hook')
                return {'query_receipt': copy.deepcopy(public['receipt'])}

            def close(self):
                trace.append('close')
                self.close_calls += 1
                self.view['closed'] = True
                self.view['local_observation']['ledger']['frozen'] = True
                self.view['local_observation']['observed']['cleanup_outcome'] = (
                    case['final_memory']['local_observation']['observed']['cleanup_outcome'])
                raise primary

            def external_count(self, value):
                trace.append('external_insert_2' if value == 2 else 'restore_insert_1')
                self.view['local_observation']['observed']['insert_calls'] = value

        def clock():
            self.assertLess(len(clocks), 2)
            value = (200, 233)[len(clocks)]
            clocks.append(value)
            return value

        memory = SuppliedMemory()
        controller = lifecycle.Controller(memory, copy.deepcopy(public['questions']), clock=clock)
        self.assertEqual(controller.snapshot(), case['initial_controller'])
        self.assertEqual(memory.view, case['initial_memory'])
        actual_error, actual_return = None, None
        stage = case['stage']
        if stage == 'dispatch':
            self.assertEqual(controller.query(0), [])
            ticket = controller.prepare_reader(0)
            def callback(body):
                trace.append('callback')
                callback_bodies.append(body)
                raise primary
            try:
                actual_return = controller.dispatch(ticket, callback)
            except BaseException as error:
                actual_error = error
            if case['kind'] == 'ordinary_probe':
                retry = controller.prepare_reader(0)
                self.assertIsNot(retry, ticket)
                self.assertEqual(len(controller._tickets), 2)
            self._guard(lambda: controller.prepare_reader(0), case['guard'])
        elif stage == 'query':
            try:
                actual_return = controller.query(0)
            except BaseException as error:
                actual_error = error
            self._guard(lambda: controller.query(0), case['guard'])
        elif stage == 'seal':
            try:
                actual_return = controller.seal_construction()
            except BaseException as error:
                actual_error = error
            if case['kind'] == 'ordinary_probe':
                self.assertIs(type(actual_return), lifecycle.Construction)
                self.assertEqual(actual_return.body.decode(), case['construction_body_utf8'])
                self.assertIs(controller.seal_construction(), actual_return)
            else:
                self._guard(controller.seal_construction, case['guard'])
        else:
            self.assertEqual(stage, 'finalize_fallback')
            memory.external_count(2)
            self._guard(lambda: controller.query(0), 'controller_external_insert')
            memory.external_count(1)
            try:
                actual_return = controller.finalize()
            except BaseException as error:
                actual_error = error
            self.assertEqual(controller.finalize(), case['final_controller'])
            if case['kind'] == 'ordinary_probe':
                self.assertEqual(actual_return, case['final_controller'])

        self.assertIs(actual_error, primary if case['outward_original_error'] else None)
        self.assertIs(primary.__context__, None)
        if case['outward_original_error']:
            self.assertIsNone(actual_return)
        self.assertEqual(touches, case['class_property_touches'])
        self.assertEqual(trace, case['trace'])
        self.assertEqual(len(clocks), case['clock_reads'])
        self.assertEqual(memory.close_calls, case['memory_close_attempts'])
        self.assertEqual(memory.context_calls, case['context_set_calls'])
        self.assertEqual(len(callback_bodies), case['callback_calls'])
        if callback_bodies:
            self.assertEqual(callback_bodies, [case['reader_body_utf8'].encode()])
        self.assertEqual(controller._slots[0]['query_outcome'], case['slot_query_outcome'])
        self.assertEqual(controller._held, case['held_bytes'])
        if case['reader_unit'] is not None:
            self.assertEqual(controller._units[('reader', 0)], case['reader_unit'])
        self.assertEqual(controller.snapshot(), case['final_controller'])
        self.assertEqual(memory.view, case['final_memory'])
        self.assertFalse(controller._busy)
        self.assertEqual(touches, [])

    def test_query_spoof_abort(self):
        self._case('query_spoof_abort')

    def test_query_throw_abort(self):
        self._case('query_throw_abort')

    def test_query_ordinary_probe(self):
        self._case('query_ordinary_probe')

    def test_dispatch_spoof_abort(self):
        self._case('dispatch_spoof_abort')

    def test_dispatch_throw_abort(self):
        self._case('dispatch_throw_abort')

    def test_dispatch_ordinary_probe(self):
        self._case('dispatch_ordinary_probe')

    def test_seal_spoof_abort(self):
        self._case('seal_spoof_abort')

    def test_seal_throw_abort(self):
        self._case('seal_throw_abort')

    def test_seal_ordinary_probe(self):
        self._case('seal_ordinary_probe')

    def test_finalize_fallback_spoof_abort(self):
        self._case('finalize_fallback_spoof_abort')

    def test_finalize_fallback_throw_abort(self):
        self._case('finalize_fallback_throw_abort')

    def test_finalize_fallback_ordinary_probe(self):
        self._case('finalize_fallback_ordinary_probe')
