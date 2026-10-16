"""Fresh supplied-owner controls; no corpus, native FTS, filesystem custody or model."""
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

FIXTURE = Path(__file__).parent / 'fixtures/longmemeval_custody_forwarding/controls.json'
NATIVE = projection.NativeProjectionMemory
OBSERVER = observed.ObservedProjectionMemory
NATIVE_GETTER = NATIVE.custody_failure_observation


class GetterAbort(BaseException):
    pass


class TrapString(str):
    armed = False
    touches = 0

    def __hash__(self):
        if self.armed:
            type(self).touches += 1
            raise AssertionError('hostile_key_hash')
        return super().__hash__()

    def __eq__(self, other):
        if self.armed:
            type(self).touches += 1
            raise AssertionError('hostile_string_equality')
        return super().__eq__(other)

    def __len__(self):
        if self.armed:
            type(self).touches += 1
            raise AssertionError('hostile_string_length')
        return super().__len__()


class TrapDict(dict):
    touches = 0

    def __iter__(self):
        type(self).touches += 1
        raise AssertionError('hostile_mapping_iteration')


class ForeignOwner:
    def __init__(self):
        self.reads = 0

    @property
    def custody_failure_observation(self):
        self.reads += 1
        raise AssertionError('foreign_property')


class TraceLock:
    def __init__(self, name, trace, error=None):
        self.name, self.trace, self.error = name, trace, error

    def __enter__(self):
        self.trace.append(self.name + ':enter')
        if self.error is not None:
            raise self.error
        return self

    def __exit__(self, *_):
        self.trace.append(self.name + ':exit')


def api_profile():
    class Memory:
        def __init__(self, params):
            self._query_context_local = threading.local()

        def set_query_context(self, *, query_invocation_id):
            self._query_context_local.invocation = query_invocation_id

        def get_query_context(self):
            return {'query_invocation_id': self._query_context_local.invocation}

        def clear_query_context(self):
            if hasattr(self._query_context_local, 'invocation'):
                del self._query_context_local.invocation

    api = SimpleNamespace(Memory=Memory, MEMORY_TYPES={})
    api.register_memory = lambda cls: api.MEMORY_TYPES.__setitem__(cls.memory_type, cls)
    return api


class CustodyForwardingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads(FIXTURE.read_text())
        cls.nested = json.loads((FIXTURE.parent / 'nested.json').read_text())
        cls.touches = json.loads((FIXTURE.parent / 'touches.json').read_text())

    def native(self, stage='file_read', owner_type=NATIVE):
        node = owner_type.__new__(owner_type)
        node._lock = threading.RLock()
        node._custody_failure = ('stage_reported', stage)
        return node

    def owner(self, backend=None):
        node = OBSERVER(self.data['horizon'], clock=lambda: self.fail('getter_clock'))
        node._backend = backend
        return node

    def bridge_owner(self, observer):
        # A real registered bridge object with only its getter's required fields.
        cls = bridge.register_bridge(api_profile(), lambda: self.fail('getter_factory'))
        node = cls.__new__(cls)
        node._lock = threading.RLock()
        node._observer = observer
        return node

    def check_unknown(self, raw):
        node = self.owner(self.native())
        before = node.snapshot()
        with patch.object(observed, '_CUSTODY_NATIVE_GETTER', return_value=raw) as getter:
            self.assertEqual(node.custody_failure_observation(), self.data['custody_unknown'])
            getter.assert_called_once_with(node._backend)
        self.assertEqual(node.snapshot(), before)

    def task_owner(self, trace):
        ticks = iter(self.data['clock_values'])

        def clock():
            trace.append('clock')
            return next(ticks)

        return OBSERVER(self.data['horizon'], clock=clock)

    def task_patches(self, backend, trace, error=None):
        def construct(*_):
            trace.append('construct')
            return backend

        def query(node, query, query_image=None):
            self.assertIs(node, backend)
            self.assertEqual(query, self.data['query'])
            self.assertIsNone(query_image)
            trace.append('task_query')
            if error is not None:
                node._custody_failure = ('stage_reported', 'file_read')
                raise error
            return []

        def hook(node):
            self.assertIs(node, backend)
            trace.append('task_hook')
            return {'queries': 1, 'state': 'failed' if error else 'no_evidence',
                    'query_image_ignored': False}

        def close(node):
            self.assertIs(node, backend)
            trace.append('task_close')

        return (patch.object(projection, 'NativeProjectionMemory', construct),
                patch.object(NATIVE, 'query', query),
                patch.object(NATIVE, 'post_query_hook', hook),
                patch.object(NATIVE, 'close', close))

    def test_absent_owner_returns_unobserved_without_work(self):
        node = self.owner()
        self.assertEqual(node.snapshot(), self.data['observed_views']['new'])
        self.assertEqual(node.custody_failure_observation(), self.data['custody_empty'])
        self.assertEqual(node.snapshot(), self.data['observed_views']['new'])
        self.assertEqual(node.failure_observation(phase='query'), self.data['failure_empty'])

    def test_foreign_backend_and_subclass_do_not_dispatch_properties(self):
        class ForeignNative(NATIVE):
            reads = 0

            @property
            def custody_failure_observation(self):
                type(self).reads += 1
                raise AssertionError('subclass_dispatch')

        foreign = ForeignOwner()
        for backend in (foreign, self.native(owner_type=ForeignNative)):
            with self.subTest(owner=type(backend).__name__):
                node = self.owner(backend)
                self.assertEqual(node.custody_failure_observation(), self.data['custody_unknown'])
                self.assertEqual(node.snapshot(), self.data['observed_views']['new'])
        self.assertEqual(foreign.reads, 0)
        self.assertEqual(ForeignNative.reads, self.touches['native_subclass_property_reads'])

    def test_all_frozen_stages_detached_and_null_costs(self):
        backend = self.native()
        node = self.owner(backend)
        for stage in self.data['stages']:
            backend._custody_failure = ('stage_reported', stage)
            expected = dict(self.data['custody_stage'], stage=stage)
            row = node.custody_failure_observation()
            self.assertEqual(row, expected)
            row['stage'] = 'caller_changed'
            self.assertEqual(node.custody_failure_observation(), expected)
        self.assertEqual(node.snapshot(), self.data['observed_views']['new'])

    def test_native_binding_ignores_method_instance_and_module_replacements(self):
        backend = self.native()
        backend.custody_failure_observation = lambda: self.fail('instance_dispatch')
        node = self.owner(backend)
        with patch.object(NATIVE, 'custody_failure_observation', new=property(lambda _: self.fail('class_dispatch'))), \
                patch.object(projection, 'NativeProjectionMemory', ForeignOwner):
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])

    def test_frozen_profile_and_whitelist_reject_module_mutations(self):
        backend = self.native()
        node = self.owner(backend)
        with patch.object(projection, 'CUSTODY_FAILURE_PROFILE', 'widened-profile'):
            self.assertEqual(node.custody_failure_observation(), self.data['custody_unknown'])
        backend._custody_failure = ('stage_reported', 'caller_stage')
        with patch.object(projection, 'CUSTODY_FAILURE_STAGES', frozenset(('caller_stage',))):
            self.assertEqual(node.custody_failure_observation(), self.data['custody_unknown'])
        backend._custody_failure = ('stage_reported', 'file_read')
        self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])

    def test_malformed_exact_records_and_hostile_values_fail_closed(self):
        mutations = {
            'not_dict': lambda row: list(row.items()),
            'dict_subclass': TrapDict,
            'missing': lambda row: {k: v for k, v in row.items() if k != 'stage'},
            'extra': lambda row: dict(row, extra=None),
            'bad_key_type': lambda row: {17 if k == 'stage' else k: v for k, v in row.items()},
            'bad_key_subclass': lambda row: {TrapString(k) if k == 'stage' else k: v for k, v in row.items()},
            'long_key': lambda row: {'x' * 65 if k == 'stage' else k: v for k, v in row.items()},
            'bad_profile': lambda row: dict(row, profile='wrong'),
            'profile_subclass': lambda row: dict(row, profile=TrapString(row['profile'])),
            'bad_scope': lambda row: dict(row, scope='wrong'),
            'scope_subclass': lambda row: dict(row, scope=TrapString(row['scope'])),
            'bad_state': lambda row: dict(row, observation_state='complete'),
            'state_subclass': lambda row: dict(row, observation_state=TrapString(row['observation_state'])),
            'bad_stage': lambda row: dict(row, stage='unknown_stage'),
            'stage_subclass': lambda row: dict(row, stage=TrapString(row['stage'])),
            'long_stage': lambda row: dict(row, stage='x' * 33),
            'stage_list': lambda row: dict(row, stage=['file_read']),
            'empty_state_with_stage': lambda row: dict(row, observation_state='unobserved'),
            'reported_without_stage': lambda row: dict(row, stage=None),
            'rootcause_nonnull': lambda row: dict(row, root_cause='inferred'),
            'tokens_nonnull': lambda row: dict(row, provider_tokens=0),
            'money_nonnull': lambda row: dict(row, usd_micros=0),
        }
        self.assertEqual(set(mutations), set(self.data['malformed_tags']))
        for name in self.data['malformed_tags']:
            with self.subTest(shape=name):
                TrapString.armed, TrapString.touches = False, 0
                TrapDict.touches = 0
                raw = mutations[name](dict(self.data['custody_stage']))
                TrapString.armed = True
                try:
                    self.check_unknown(raw)
                    self.assertEqual(TrapString.touches, 0)
                    self.assertEqual(TrapDict.touches, self.touches['dict_subclass_iterations'])
                finally:
                    TrapString.armed = False

    def test_missing_and_aborted_native_getter_are_unknown(self):
        node = self.owner(self.native())
        for replacement in (None, lambda _: (_ for _ in ()).throw(RuntimeError('not_exported')),
                            lambda _: (_ for _ in ()).throw(GetterAbort())):
            with patch.object(observed, '_CUSTODY_NATIVE_GETTER', replacement):
                self.assertEqual(node.custody_failure_observation(), self.data['custody_unknown'])
        self.assertEqual(node.snapshot(), self.data['observed_views']['new'])
        self.assertEqual(node.failure_observation(phase='query'), self.data['failure_empty'])

    def test_observer_lock_admission_abort_is_not_suppressed(self):
        node = self.owner(self.native())
        trace, error = [], GetterAbort()
        node._lock = TraceLock('observed', trace, error)
        with self.assertRaises(GetterAbort) as caught:
            node.custody_failure_observation()
        self.assertIs(caught.exception, error)
        self.assertEqual(trace, ['observed:enter'])

    def test_bridge_exact_observer_gate_avoids_foreign_and_subclass_dispatch(self):
        class ForeignObserver(OBSERVER):
            reads = 0

            @property
            def custody_failure_observation(self):
                type(self).reads += 1
                raise AssertionError('subclass_observer_dispatch')

        foreign = ForeignOwner()
        for observer in (None, foreign, ForeignObserver(self.data['horizon'])):
            node = self.bridge_owner(observer)
            self.assertEqual(node.custody_failure_observation(), self.data['custody_unknown'])
        self.assertEqual(foreign.reads, 0)
        self.assertEqual(ForeignObserver.reads, self.touches['observer_subclass_property_reads'])

    def test_bridge_binding_detachment_and_nested_failure_are_bounded(self):
        observer = self.owner(self.native())
        observer.custody_failure_observation = lambda: self.fail('observer_instance_dispatch')
        node = self.bridge_owner(observer)
        with patch.object(OBSERVER, 'custody_failure_observation', new=property(lambda _: self.fail('observer_class_dispatch'))), \
                patch.object(bridge, 'ObservedProjectionMemory', ForeignOwner):
            row = node.custody_failure_observation()
            self.assertEqual(row, self.data['custody_stage'])
            row['profile'] = 'caller_changed'
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
        for value in (None, dict(self.data['custody_stage'], provider_tokens=0), TrapDict()):
            TrapDict.touches = 0
            with patch.object(bridge, '_CUSTODY_OBSERVER_GETTER', return_value=value):
                self.assertEqual(node.custody_failure_observation(), self.data['custody_unknown'])
                self.assertEqual(TrapDict.touches, self.touches['dict_subclass_iterations'])
        with patch.object(bridge, '_CUSTODY_OBSERVER_GETTER', side_effect=GetterAbort()):
            self.assertEqual(node.custody_failure_observation(), self.data['custody_unknown'])
        self.assertEqual(observer.snapshot(), self.data['observed_views']['new'])

    def test_bridge_lock_order_and_own_admission_abort(self):
        backend = self.native()
        observer = self.owner(backend)
        node = self.bridge_owner(observer)
        trace = []
        node._lock = TraceLock('bridge', trace)
        observer._lock = TraceLock('observed', trace)
        backend._lock = TraceLock('native', trace)
        self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
        self.assertEqual(trace, ['bridge:enter', 'observed:enter', 'native:enter',
                                 'native:exit', 'observed:exit', 'bridge:exit'])
        trace.clear()
        error = GetterAbort()
        node._lock = TraceLock('bridge', trace, error)
        with self.assertRaises(GetterAbort) as caught:
            node.custody_failure_observation()
        self.assertIs(caught.exception, error)
        self.assertEqual(trace, ['bridge:enter'])

    def test_nested_owner_admission_failures_are_diagnostic_only(self):
        backend = self.native()
        observer = self.owner(backend)
        node = self.bridge_owner(observer)
        trace, error = [], GetterAbort()
        observer._lock = TraceLock('observed', trace)
        backend._lock = TraceLock('native', trace, error)
        self.assertEqual(observer.custody_failure_observation(), self.nested['expected_observation'])
        self.assertEqual(trace, self.nested['native_lock_failure_trace'])
        trace.clear()
        backend._lock = threading.RLock()
        observer._lock = TraceLock('observed', trace, error)
        node._lock = TraceLock('bridge', trace)
        self.assertEqual(node.custody_failure_observation(), self.nested['expected_observation'])
        self.assertEqual(trace, self.nested['nested_observer_lock_failure_trace'])
        observer._lock = threading.RLock()
        self.assertEqual(observer.snapshot(), self.data['observed_views']['new'])
        self.assertEqual(observer.failure_observation(phase='query'), self.data['failure_empty'])

    def test_interleaved_getters_preserve_complete_observed_views_and_costs(self):
        trace, backend = [], self.native()
        node = self.task_owner(trace)
        self.assertEqual(node.snapshot(), self.data['observed_views']['new'])
        self.assertEqual(node.custody_failure_observation(), self.data['custody_empty'])
        one, two, three, four = self.task_patches(backend, trace)
        with one, two, three, four:
            node.open('supplied-root', object())
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
            self.assertEqual(node.snapshot(), self.data['observed_views']['opened'])
            self.assertEqual(node.query(self.data['query']), [])
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
            self.assertEqual(node.query_receipt(0), self.data['query_receipt'])
            self.assertEqual(node.snapshot(), self.data['observed_views']['queried'])
            node.close()
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
            self.assertEqual(node.snapshot(), self.data['observed_views']['closed'])
            self.assertEqual(node.failure_observation(phase='query'), self.data['failure_empty'])
        self.assertEqual(trace, self.data['task_trace'])

    def test_query_failure_preserves_primary_exception_receipt_and_costs(self):
        trace, backend = [], self.native()
        backend._custody_failure = ('unobserved', None)
        node = self.task_owner(trace)
        error = projection.ProjectionError('source_changed')
        one, two, three, four = self.task_patches(backend, trace, error)
        with one, two, three, four:
            node.open('supplied-root', object())
            self.assertEqual(node.custody_failure_observation(), self.data['custody_empty'])
            with self.assertRaises(projection.ProjectionError) as caught:
                node.query(self.data['query'])
            self.assertIs(caught.exception, error)
            self.assertEqual(node.failure_observation(phase='query'), self.data['failure_query'])
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
            self.assertEqual(node.snapshot(), self.data['failed_views']['queried'])
            node.close()
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
            self.assertEqual(node.snapshot(), self.data['failed_views']['closed'])
        self.assertEqual(trace, self.data['task_trace'])

    def test_interleaved_bridge_getters_preserve_whole_hook_and_frozen_views(self):
        trace, backend = [], self.native()
        observer = self.task_owner(trace)

        def factory():
            trace.append('factory')
            return ('supplied-root', object())

        cls = bridge.register_bridge(api_profile(), factory)
        one, two, three, four = self.task_patches(backend, trace)
        with one, two, three, four, patch.object(bridge, 'ObservedProjectionMemory', return_value=observer):
            node = cls({'horizon': self.data['horizon']})
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
            self.assertEqual(node.snapshot(), self.data['bridge_views']['opened'])
            node.set_query_context(query_invocation_id=self.data['invocation'])
            context = node.query(self.data['query'])
            self.assertEqual(context, [])
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
            hook = node.post_query_hook(query=self.data['query'], query_image=None, memory_context=context)
            self.assertEqual(hook, self.data['hook'])
            self.assertEqual(node.snapshot(), self.data['bridge_views']['queried'])
            node.close()
            self.assertEqual(node.custody_failure_observation(), self.data['custody_stage'])
            self.assertEqual(node.snapshot(), self.data['bridge_views']['closed'])
            self.assertEqual(node.failure_observation(phase='query'), self.data['failure_empty'])
        self.assertEqual(trace, self.data['bridge_trace'])
