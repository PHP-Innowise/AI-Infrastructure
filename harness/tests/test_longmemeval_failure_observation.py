"""NEW standalone offline error-observation controls, no closed corpus replay."""
import ast
import copy
import json
from pathlib import Path
import sys
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'src'))
from harness import longmemeval_failure_observation as failure
from harness import longmemeval_observed as observed
from harness import longmemeval_bridge as bridge
from harness.longmemeval_projection import ProjectionError

PUBLIC = Path(__file__).resolve().parent/'fixtures/longmemeval_failure_observation/controls.json'

class ReflectiveAbort(BaseException):
    @property
    def __class__(self): raise AssertionError('reflection_forbidden')
    def __str__(self): raise AssertionError('formatting_forbidden')

class Subclass(ProjectionError):
    def __getattribute__(self, name): raise AssertionError('attributes_forbidden')
    def __str__(self): raise AssertionError('formatting_forbidden')

class Backend:
    def __init__(self, insert_error=None, close_error=None):
        self.insert_error, self.close_error = insert_error, close_error
    def insert(self, value):
        if self.insert_error is not None: raise self.insert_error
    def close(self):
        if self.close_error is not None: raise self.close_error

class Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.oracle=json.loads(PUBLIC.read_bytes())
        cls.integration=json.loads((PUBLIC.parent/'integration-controls.json').read_bytes())

    def owner(self, backend):
        item=observed.ObservedProjectionMemory(2)
        with patch.object(observed.projection,'NativeProjectionMemory',return_value=backend):
            item.open('public-not-opened', object())
        return item

    def test_whole_new_scalar_oracles_without_formatting(self):
        for case in self.oracle['cases']:
            kind=case['kind'];args=case['args']
            error=ProjectionError(*args) if kind=='owned' else Subclass(*args) if kind=='subclass' else ValueError(*args)
            with patch.object(ProjectionError,'__str__',side_effect=AssertionError('formatting_forbidden')):
                self.assertEqual(failure.projection_failure_metadata(error,phase=case['phase']),case['expected'])

    def test_source_guard_allowlist_and_all_fourteen_labels(self):
        self.assertEqual(set(self.oracle['guards']),failure.GUARDS)
        source=ast.parse(Path(observed.projection.__file__).read_bytes())
        literal={call.args[0].value for call in ast.walk(source) if isinstance(call,ast.Call) and isinstance(call.func,ast.Name) and call.func.id=='_refuse' and call.args and isinstance(call.args[0],ast.Constant)}
        self.assertEqual(literal,failure.GUARDS)
        for guard in self.oracle['guards']:
            self.assertEqual(failure.projection_failure_metadata(ProjectionError(guard),phase='insert')['reported_guard'],guard)

    def test_malformed_and_sensitive_owned_arguments_are_unknown(self):
        for args in [(),('input_bound','extra'),(object(),),(None,),('x'*1000,),('source_changed\x00',),('источник',)]:
            got=failure.projection_failure_metadata(ProjectionError(*args),phase='query')
            self.assertIsNone(got['reported_guard']);self.assertEqual(got['observation_state'],'unmeasured')
            self.assertIsNone(got['root_cause']);self.assertIsNone(got['provider_tokens'])

    def test_invalid_phase_is_refused_without_echoing(self):
        for phase in [None,True,[], 'query\x00', 'x'*1000]:
            with self.assertRaisesRegex(ValueError,'^failure_phase_profile$'):
                failure.projection_failure_metadata(ProjectionError('input_schema'),phase=phase)

    def test_initialization_capture_and_existing_snapshot_schema(self):
        item=observed.ObservedProjectionMemory(2)
        with self.assertRaises(ProjectionError): item.open('public-not-opened',object())
        self.assertEqual(item.failure_observation(phase='initialization'),self.oracle['automatic_init_expected'])
        self.assertEqual(set(item.snapshot()),set(self.integration['snapshot_top_keys']))
        item.close()
        self.assertEqual(item.failure_observation(phase='initialization'),self.oracle['automatic_init_expected'])

    def test_insert_capture_preserves_identity_and_detached_record(self):
        error=ProjectionError('corpus_bound');item=self.owner(Backend(insert_error=error))
        with self.assertRaises(ProjectionError) as caught:item.insert(copy.deepcopy(self.integration['trajectory']))
        self.assertIs(caught.exception,error)
        got=item.failure_observation(phase='insert');self.assertEqual(got,self.oracle['automatic_insert_expected'])
        got['reported_guard']='caller-edited';self.assertEqual(item.failure_observation(phase='insert'),self.oracle['automatic_insert_expected'])
        self.assertEqual(item.failure_observation(phase='query'),self.oracle['empty']);item.close()

    def test_query_admission_capture_without_backend_query(self):
        item=self.owner(Backend())
        with self.assertRaises(ProjectionError):item.query([],None)
        self.assertEqual(item.failure_observation(phase='query'),self.oracle['automatic_query_expected'])
        self.assertEqual(item.snapshot()['observed']['query_calls'],1);item.close()

    def test_cleanup_unknown_preserves_prior_phase_record(self):
        insert_error=ProjectionError('corpus_bound');close_error=OSError('arbitrary-private-prose')
        item=self.owner(Backend(insert_error,close_error))
        with self.assertRaises(ProjectionError):item.insert(copy.deepcopy(self.integration['trajectory']))
        with self.assertRaises(OSError) as caught:item.close()
        self.assertIs(caught.exception,close_error)
        self.assertEqual(item.failure_observation(phase='insert'),self.oracle['automatic_insert_expected'])
        self.assertEqual(item.failure_observation(phase='cleanup'),self.oracle['automatic_cleanup_expected'])
        self.assertEqual(item.close()['observed']['status'],'cleanup_failed')

    def test_capture_failure_cannot_replace_primary_exception(self):
        error=ProjectionError('corpus_bound');item=self.owner(Backend(insert_error=error))
        with patch.object(observed,'projection_failure_metadata',side_effect=KeyboardInterrupt('private')):
            with self.assertRaises(ProjectionError) as caught:item.insert(copy.deepcopy(self.integration['trajectory']))
        self.assertIs(caught.exception,error)
        self.assertEqual(item.failure_observation(phase='insert'),self.oracle['capture_failure_expected']);item.close()

    def test_reflective_abort_remains_original_cancelled_exception(self):
        error=ReflectiveAbort('private');item=self.owner(Backend(insert_error=error))
        with self.assertRaises(ReflectiveAbort) as caught:item.insert(copy.deepcopy(self.integration['trajectory']))
        self.assertIs(caught.exception,error);self.assertEqual(item.snapshot()['observed']['insert_cancelled'],1)
        self.assertEqual(item.failure_observation(phase='insert'),self.oracle['capture_failure_expected']);item.close()

    def test_prebody_refusal_is_unobserved_and_close_is_idempotent(self):
        item=observed.ObservedProjectionMemory(2)
        with self.assertRaises(observed.ObservationError):item.insert(copy.deepcopy(self.integration['trajectory']))
        self.assertEqual(item.failure_observation(phase='insert'),self.oracle['empty'])
        self.assertEqual(item.close(),item.close())

    def test_registered_bridge_delegates_optional_accessor(self):
        class Memory:
            def __init__(self,params):self._query_context_local=threading.local()
        class API:
            MEMORY_TYPES={}
            def register_memory(self,candidate):self.MEMORY_TYPES[candidate.memory_type]=candidate
        api=API();api.Memory=Memory
        candidate=bridge.register_bridge(api,lambda:('public-not-opened',object()))
        error=ProjectionError('corpus_bound')
        with patch.object(observed.projection,'NativeProjectionMemory',return_value=Backend(insert_error=error)):
            item=candidate({'horizon':2})
        with self.assertRaises(ProjectionError):item.insert(copy.deepcopy(self.integration['trajectory']))
        self.assertEqual(item.failure_observation(phase='insert'),self.oracle['automatic_insert_expected'])
        self.assertEqual(set(item.snapshot()['local_observation']),set(self.integration['snapshot_top_keys']));item.close()

if __name__=='__main__':unittest.main()
