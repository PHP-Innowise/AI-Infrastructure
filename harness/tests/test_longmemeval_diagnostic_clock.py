"""NEW supplied-clock controls; no actual corpus/nativebank/provider/actor."""
import copy
import json
from pathlib import Path
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from harness import longmemeval_bridge as bridge
from harness import longmemeval_observed as observed
from harness import longmemeval_projection as projection

FIXTURE = Path(__file__).parent / 'fixtures/longmemeval_diagnostic_clock/controls.json'
NATIVE = projection.NativeProjectionMemory
OBSERVER = observed.ObservedProjectionMemory


class Abort(BaseException):
    pass


class PrimaryError(Exception):
    pass


class CleanupError(OSError):
    pass


class DiagnosticClockTests(unittest.TestCase):
    def setUp(self):
        self.public = json.loads(FIXTURE.read_text())
        self.public['observers'].extend(json.loads(FIXTURE.with_name('supplement.json').read_text())['observers'])
        extra = json.loads(FIXTURE.with_name('adversarial.json').read_text())
        self.public['native'].extend(extra['native'])
        self.public['invalid_numeric_kinds'].extend(extra['invalid_numeric_kinds'])

    def _native(self, name):
        case = next(row for row in self.public['native'] if row['name'] == name)
        trace, touches, reads = [], [], []
        errors = {'body_error': PrimaryError('body'), 'cleanup_error': CleanupError('cleanup'),
                  'end_abort': Abort('end')}
        body_lines = ['Public reference data:', 'The revised unit rate is 17.']
        capsule = {'semantic': ['public-rate'], 'episodic': []}
        root = Path(self.public['public_root'])

        def entry(label, *args):
            trace.append([label, *copy.deepcopy(args)])

        def close():
            entry('connection_close')
            if case['close_error']:
                raise errors['cleanup_error']

        connection = SimpleNamespace(close=close)

        def sanitize(value):
            entry('sanitize', value)
            return 'safe-rate-query'

        def connect(path):
            entry('connect', str(path))
            return connection

        def refresh(actual, repository):
            self.assertIs(actual, connection)
            entry('refresh', str(repository))
            return None, {'phase_seconds': {'index': 0.0}}, []

        def distill(actual, value):
            self.assertIs(actual, connection)
            entry('distill', value)
            return 'rate-distilled'

        def assemble(actual, repository, **kwargs):
            self.assertIs(actual, connection)
            self.assertEqual(repository, root)
            entry('assemble', kwargs)
            if case['body_error']:
                raise errors['body_error']
            return copy.deepcopy(capsule)

        def excerpts(actual, value):
            self.assertIs(actual, connection)
            entry('excerpts', value)
            return ['literal excerpt']

        def render(value, items, *, trace):
            entry('render', value, items)
            self.assertEqual(trace, {})
            return body_lines

        native = SimpleNamespace(sanitize_automatic_query=sanitize,
            reject_secrets=lambda *v: entry('reject_secrets', *v),
            reject_capsule_privacy=lambda *v: entry('reject_privacy', *v),
            configured_mode=lambda p: (entry('mode', str(p)), 'governed')[1],
            configured_retrieval_gate=lambda p: (entry('gate', str(p)), 'auto')[1],
            connect=connect, refresh_layers=refresh, distill_capsule_query=distill,
            assemble_capsule=assemble, capsule_excerpts=excerpts, render_capsule_lines=render,
            record_render_trace=lambda p, c, t: entry('record_trace', str(p), c, t))
        owner = NATIVE.__new__(NATIVE)
        owner.repository, owner.native = root, native
        owner._lock = threading.RLock()
        owner._closed = owner._fenced = False
        owner._queries, owner._bytes = 5, 177
        owner._documents = {'public-a': None, 'public-b': None, 'public-c': None}
        owner._last = {'prior': 'not delivered by this call'}
        owner._custody = lambda: entry('custody')

        class FloatTrap(float):
            def __sub__(self, other):
                touches.append('float_subtraction')
                raise AssertionError('forbidden numeric dispatch')
            def __float__(self):
                touches.append('float_coercion')
                raise AssertionError('forbidden numeric dispatch')

        class IntTrap(int):
            def __sub__(self, other):
                touches.append('int_subtraction')
                raise AssertionError('forbidden numeric dispatch')
            def __float__(self):
                touches.append('int_coercion')
                raise AssertionError('forbidden numeric dispatch')

        class EqualityTrap(type):
            def __eq__(cls, other):
                touches.append('class_equality')
                raise AssertionError('forbidden class equality')

        class ForeignNumber(metaclass=EqualityTrap):
            def __float__(self):
                touches.append('foreign_coercion')
                raise AssertionError('forbidden foreign coercion')
            def __ge__(self, other):
                touches.append('foreign_comparison')
                raise AssertionError('forbidden foreign comparison')
            def __sub__(self, other):
                touches.append('foreign_subtraction')
                raise AssertionError('forbidden foreign subtraction')

        timer = case['timer']
        end = {'valid': 100.375, 'nan': float('nan'), 'positive_infinity': float('inf'),
               'negative_infinity': float('-inf'), 'negative': 99.0,
               'bool': True, 'wrong_type': 'clock', 'float_subclass': FloatTrap(101),
               'int_subclass': IntTrap(101), 'foreign_metaclass': ForeignNumber()}

        def clock():
            reads.append(len(reads))
            if len(reads) == 1:
                return 100.0
            self.assertEqual(len(reads), 2)
            if timer == 'ordinary':
                raise OSError('timer')
            if timer == 'abort':
                raise errors['end_abort']
            return end[timer]

        actual_error, result = None, None
        with patch.object(projection.time, 'monotonic', clock):
            try:
                result = owner.query(self.public['public_query'])
            except BaseException as error:
                actual_error = error
        expected_error = errors.get(case['outward'])
        self.assertIs(actual_error, expected_error)
        if actual_error is not None:
            self.assertIs(actual_error.__context__, errors.get(case['outward_context']))
        self.assertEqual(result, case['result'])
        self.assertEqual(trace, case['trace'])
        self.assertEqual(reads, [0, 1])
        self.assertEqual(touches, [])
        self.assertEqual(owner.post_query_hook(), case['metadata'])
        self.assertEqual(owner._queries, 6)
        self.assertEqual(owner._bytes, 177)
        self.assertEqual(owner._documents, {'public-a': None, 'public-b': None, 'public-c': None})
        self.assertFalse(owner._closed)
        self.assertFalse(owner._fenced)

    def test_native_normal_elapsed_and_context(self):
        self._native('normal')

    def test_native_ordinary_timer_error_retains_context(self):
        self._native('ordinary_end_error')

    def test_native_timer_abort_clears_delivery(self):
        self._native('end_abort')

    def test_native_body_primary_survives_timer_abort(self):
        self._native('body_and_timer_abort')

    def test_native_cleanup_primary_survives_timer_abort(self):
        self._native('cleanup_and_timer_abort')

    def test_native_body_cleanup_timer_precedence_and_context(self):
        self._native('body_cleanup_timer_abort')

    def test_native_invalid_elapsed_is_unknown_without_numeric_dispatch(self):
        for kind in self.public['invalid_numeric_kinds']:
            with self.subTest(kind=kind):
                self._native('invalid_' + kind)

    def _observer(self, name):
        case = next(row for row in self.public['observers'] if row['name'] == name)
        errors = {'start_abort': Abort('start'), 'end_abort': Abort('end'),
                  'cleanup_error': CleanupError('backend')}
        reads, closes, factories = [], [], []
        schedule = ([] if case['absent'] else [10, 28]) + [
            errors['start_abort'] if case['start_abort'] else 80,
            errors['end_abort'] if case['end_abort'] else 98]

        def clock():
            value = schedule[len(reads)]
            reads.append(len(reads))
            if issubclass(type(value), BaseException):
                raise value
            return value

        def close():
            closes.append('backend_close')
            if case['cleanup_error']:
                raise errors['cleanup_error']

        backend = SimpleNamespace(close=close)

        def factory(*args):
            factories.append(args)
            return backend

        with patch.object(projection, 'NativeProjectionMemory', factory):
            if case['bridge']:
                class Memory:
                    def __init__(self, parameters):
                        self._query_context_local = threading.local()

                api = SimpleNamespace(Memory=Memory, MEMORY_TYPES={})
                api.register_memory = lambda cls: api.MEMORY_TYPES.setdefault(cls.memory_type, cls)
                def observer_factory(**parameters):
                    return OBSERVER(clock=clock, **parameters)
                with patch.object(bridge, 'ObservedProjectionMemory', observer_factory):
                    cls = bridge.register_bridge(api, lambda: ('public-clock-root', object()))
                    target = cls({'horizon': 3})
                inspector = target._observer
            else:
                target = OBSERVER(3, clock=clock)
                inspector = target
                if not case['absent']:
                    target.open('public-clock-root', object())
            self.assertEqual(len(factories), 0 if case['absent'] else 1)
            actual_error, actual_return = None, None
            try:
                actual_return = target.close()
            except BaseException as error:
                actual_error = error
            self.assertIs(actual_error, errors.get(case['outward']))
            self.assertEqual(actual_return, case['snapshot'] if case['outward'] is None else None)
            if actual_error is not None:
                self.assertIs(actual_error.__context__, None)
            self.assertEqual(len(closes), case['close_attempts'])
            self.assertEqual(len(reads), case['clock_reads'])
            self.assertEqual(target.snapshot(), case['snapshot'])
            for phase, expected in case['failures'].items():
                self.assertEqual(inspector.failure_observation(phase=phase), expected)
            before = (list(reads), list(closes), len(factories))
            self.assertEqual(target.close(), case['snapshot'])
            self.assertEqual(target.snapshot(), case['snapshot'])
            self.assertEqual((reads, closes, len(factories)), before)
            if case['bridge']:
                self.assertTrue(target._closed)
                self.assertFalse(target._busy)
                self.assertEqual(target._final, case['snapshot'])
            self.assertFalse(inspector._active)

    def test_observer_start_abort_still_closes_backend(self):
        self._observer('start_abort_success')

    def test_observer_backend_error_outranks_start_clock_abort(self):
        self._observer('start_abort_cleanup_error')

    def test_observer_start_abort_outranks_end_abort(self):
        self._observer('start_and_end_abort')

    def test_observer_absent_backend_start_abort(self):
        self._observer('absent_start_abort')

    def test_observer_end_abort_retains_successful_cleanup(self):
        self._observer('end_abort_success')

    def test_bridge_start_abort_freezes_successful_cleanup(self):
        self._observer('bridge_start_abort')

    def test_bridge_cleanup_primary_freezes_failed_cleanup(self):
        self._observer('bridge_start_abort_cleanup_error')

    def test_observer_normal_close_retains_full_known_timing(self):
        self._observer('normal_success')

    def test_observer_backend_error_outranks_end_clock_abort(self):
        self._observer('backend_error_end_abort')
