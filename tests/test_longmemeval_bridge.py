"""NEW pinned-prefix registration controls; no full upstream/model runtime."""
import __future__
import copy
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import threading
import types
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
EDITION = ROOT / 'PHP Core'
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import longmemeval_bridge as subject
from harness import longmemeval_observed as observed

PUBLIC = json.loads((ROOT / 'tests/fixtures/longmemeval_bridge_public.json').read_text())
EXPECTED = json.loads((ROOT / 'tests/fixtures/longmemeval_bridge_expected.json').read_text())
WHOLE_SHA = '607ca21320067df2abc3583b80debeb819781e90955776bdea4313a8b93a0586'
PREFIX_SHA = '6bdc677c32b42ec6d7e142e07213adbcfb21403804aac16e2a2d810319756025'
SUFFIX = (b'from .no_retrieval import NoRetrievalMemory  # noqa: E402,F401\n'
          b'from .codex import CodexMemory  # noqa: E402,F401\n'
          b'from .agentrunbook_c import AgentRunbookC  # noqa: E402,F401\n'
          b'from .agentrunbook_c_v2 import AgentRunbookCV2  # noqa: E402,F401\n'
          b'from .agentrunbook_r import AgentRunbookR  # noqa: E402,F401\n'
          b'from .rag import RagMemory  # noqa: E402,F401\n')


def prefix_api():
    raw = (ROOT / 'tests/fixtures/longmemeval_memory_api_2cc8c540.py').read_bytes()
    if hashlib.sha256(raw).hexdigest() != WHOLE_SHA or not raw.endswith(SUFFIX):
        raise RuntimeError('pinned_whole_memory_api_mismatch')
    body = raw[:-len(SUFFIX)]
    if len(body) != 7960 or hashlib.sha256(body).hexdigest() != PREFIX_SHA:
        raise RuntimeError('pinned_memory_api_prefix_mismatch')
    api = types.ModuleType('new_pinned_stdlib_memory_api')
    # Offline selected-source compatibility only: keep pinned bytes/hashes,
    # defer newer annotations explicitly when this stdlib test runs on 3.9.
    exec(compile(body, 'pinned_unchanged_memory_api_prefix.py', 'exec',
                 flags=__future__.annotations.compiler_flag, dont_inherit=True), api.__dict__)
    return api


class Clock:
    def __init__(self):
        self.value = 0

    def __call__(self):
        self.value += 100
        return self.value


class Delegate:
    def __init__(self, repository, native):
        self.counter, self.closed = 0, 0
        self.last = None
        self.actions = ['delivered', 'no_evidence']
        self.context = copy.deepcopy(PUBLIC['controlled_context'])
        self.query_callback = None
        self.close_error = None
        self.inserted = []

    def insert(self, trajectory):
        self.inserted.append(copy.deepcopy(trajectory))

    def query(self, query, query_image=None):
        if self.query_callback is not None:
            self.query_callback()
        self.counter += 1
        state = self.actions.pop(0) if self.actions else 'delivered'
        self.last = {'queries': self.counter, 'state': state if state != 'cancelled' else 'failed',
                     'query_image_ignored': query_image is not None}
        if state == 'cancelled':
            raise KeyboardInterrupt('PUBLIC_QUERY_CANCEL_SENTINEL')
        if state == 'failed':
            raise OSError('PUBLIC_QUERY_FAIL_SENTINEL')
        return self.context if state == 'delivered' else []

    def post_query_hook(self):
        return dict(self.last)

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


class BridgeContract(unittest.TestCase):
    def create(self, config=None):
        api = prefix_api()
        allocated = []
        def factory():
            allocated.append(True)
            return ('fresh-private-root', object())
        original = observed.ObservedProjectionMemory
        def observer_factory(**params):
            return original(**params, clock=Clock())
        with patch.object(subject, 'ObservedProjectionMemory', observer_factory), \
                patch.object(observed.projection, 'NativeProjectionMemory', Delegate):
            cls = subject.register_bridge(api, factory)
            memory = api.build_memory(copy.deepcopy(PUBLIC['config'] if config is None else config))
        return api, cls, memory, allocated

    def invoke(self, memory, index, hooks=1):
        value = PUBLIC['queries'][index]
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][index])
        try:
            result = memory.query(value['text'], value['query_image'])
            payloads = [memory.post_query_hook(query=value['text'], query_image=value['query_image'],
                                               memory_context=result) for _ in range(hooks)]
            return result, payloads
        finally:
            memory.clear_query_context()

    def test_pinned_real_registry_build_and_whole_new_hook_oracles(self):
        api, cls, memory, allocated = self.create()
        self.assertIs(api.MEMORY_TYPES[subject.MEMORY_TYPE], cls)
        self.assertIsInstance(memory, api.Memory)
        self.assertEqual(EXPECTED['normalized_config'], memory.memory_config)
        memory.insert(PUBLIC['trajectory'])
        self.assertEqual(0, memory.snapshot()['local_observation']['ledger']['phases']['build']['closed_units'])
        result0, hooks0 = self.invoke(memory, 0, hooks=2)
        result1, hooks1 = self.invoke(memory, 1)
        self.assertEqual(PUBLIC['controlled_context'], result0)
        self.assertEqual([], result1)
        self.assertEqual([EXPECTED['controlled_hooks'][0]] * 2, hooks0)
        self.assertEqual([EXPECTED['controlled_hooks'][1]], hooks1)
        final = memory.close()
        self.assertEqual(EXPECTED['bridge_final_projection'],
                         {key: final[key] for key in EXPECTED['bridge_final_projection']})
        view = final['local_observation']['observed']
        self.assertEqual(EXPECTED['local_final_projection'],
                         {key: view[key] for key in EXPECTED['local_final_projection']})
        self.assertIsNone(final['local_observation']['ledger']['product']['tokens'])
        self.assertIsNone(final['local_observation']['ledger']['product']['usd_micros'])
        self.assertEqual([True], allocated)

    def test_foreign_registry_collision_refuses_without_overwrite_or_factory(self):
        api = prefix_api()
        foreign = object()
        api.MEMORY_TYPES[subject.MEMORY_TYPE] = foreign
        called = []
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_registry_collision$'):
            subject.register_bridge(api, lambda: called.append(True))
        self.assertIs(foreign, api.MEMORY_TYPES[subject.MEMORY_TYPE])
        self.assertEqual([], called)

    def test_invalid_parameters_fail_before_resource_allocation(self):
        api = prefix_api()
        allocated = []
        cls = subject.register_bridge(api, lambda: allocated.append(True))
        invalid = [{}, {'horizon': 0}, {'horizon': True}, {'horizon': 513},
                   {'horizon': 1, 'local_token_profile': 'PUBLIC_UNKNOWN'},
                   {'horizon': 1, 'repository': 'PUBLIC_PATH'},
                   {'horizon': 1, 'factory': 'PUBLIC_FACTORY'},
                   {'horizon': 1, 'answer': 'PUBLIC_EVALUATOR_SENTINEL'}]
        for params in invalid:
            with self.subTest(keys=list(params)), self.assertRaises(subject.BridgeError):
                cls(params)
        self.assertEqual([], allocated)

    def test_memory_config_and_public_params_mutation_cannot_change_scope(self):
        _, _, memory, _ = self.create()
        memory.memory_params['horizon'] = 512
        exposed = memory.memory_config
        exposed['memory_params']['horizon'] = 9
        self.assertEqual(EXPECTED['normalized_config'], memory.memory_config)
        self.assertEqual(2, memory.snapshot()['local_observation']['ledger']['horizon'])
        memory.close()

    def test_missing_or_uncleared_invocation_refuses_before_local_query(self):
        _, _, memory, _ = self.create()
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_invocation_missing$'):
            memory.query('x')
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_invocation_uncleared$'):
            memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][1])
        self.assertEqual(0, memory.snapshot()['local_observation']['observed']['query_calls'])
        memory.clear_query_context()
        memory.close()

    def test_exact_id_aliases_replay_and_clear_without_query_are_consumed(self):
        _, _, memory, _ = self.create()
        for identity in EXPECTED['adversarial']['duplicate_ids'][1:]:
            with self.assertRaisesRegex(subject.BridgeError, '^bridge_invocation_alias$'):
                memory.set_query_context(query_invocation_id=identity)
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        memory.clear_query_context()
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_invocation_reused$'):
            memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        self.invoke(memory, 1)
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_invocation_capacity$'):
            memory.set_query_context(query_invocation_id='public-nacre-no-replacement')
        final = memory.close()
        self.assertEqual(1, final['local_observation']['observed']['unlaunched_queries'])
        self.assertIsNone(final['local_observation']['ledger']['phases']['query']['tokens'])

    def test_hook_binds_exact_request_image_context_and_private_copies(self):
        _, _, memory, _ = self.create()
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        q = PUBLIC['queries'][0]
        result = memory.query(q['text'], q['query_image'])
        for args in ({'query': 'PUBLIC_OTHER_QUERY', 'query_image': None, 'memory_context': result},
                     {'query': q['text'], 'query_image': 'public-images/wrong.png', 'memory_context': result},
                     {'query': q['text'], 'query_image': None, 'memory_context': EXPECTED['adversarial']['hook_wrong_context']}):
            with self.assertRaisesRegex(subject.BridgeError, '^bridge_hook_binding$'):
                memory.post_query_hook(**args)
        original = memory.post_query_hook(query=q['text'], query_image=None, memory_context=result)
        original['query_receipt']['ordinal'] = 999
        repeated = memory.post_query_hook(query=q['text'], query_image=None, memory_context=result)
        self.assertEqual(EXPECTED['controlled_hooks'][0], repeated)
        self.assertEqual(1, memory.snapshot()['local_observation']['observed']['query_calls'])
        memory.clear_query_context()
        with self.assertRaises(subject.BridgeError):
            memory.post_query_hook(query=q['text'], query_image=None, memory_context=result)
        memory.close()

    def test_hook_before_query_and_repeated_query_refuse_without_duplicate_work(self):
        _, _, memory, _ = self.create()
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_hook_state$'):
            memory.post_query_hook(query='x', query_image=None, memory_context=[])
        self.query_one(memory)
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_invocation_reused$'):
            self.query_one(memory)
        self.assertEqual(1, memory._observer._backend.counter)
        memory.clear_query_context()
        memory.close()

    def query_one(self, memory):
        return memory.query(PUBLIC['queries'][0]['text'], None)

    def test_failed_and_cancelled_queries_retain_ids_and_local_receipts(self):
        for outcome in ('failed', 'cancelled'):
            with self.subTest(outcome=outcome):
                _, _, memory, _ = self.create()
                memory._observer._backend.actions = [outcome]
                memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
                with self.assertRaises(OSError if outcome == 'failed' else KeyboardInterrupt):
                    self.query_one(memory)
                memory.clear_query_context()
                with self.assertRaisesRegex(subject.BridgeError, '^bridge_invocation_reused$'):
                    memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
                final = memory.close()
                receipt = final['local_observation']['observed']['query_receipts'][0]
                self.assertEqual(outcome, receipt['outcome'])
                self.assertEqual(0, receipt['output_bytes'])
                self.assertNotIn('PUBLIC_QUERY_', json.dumps(final))

    def test_persistence_class_refusal_never_creates_output_or_allocates_again(self):
        api, cls, memory, allocated = self.create()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'must-not-exist'
            for call in (lambda: api.save_memory(memory, output), lambda: memory._save_backend(output),
                         lambda: memory._load_backend(output),
                         lambda: cls.reconcile_loaded_memory_config({}, None)):
                with self.assertRaisesRegex(subject.BridgeError, '^bridge_persistence_unsupported$'):
                    call()
                self.assertFalse(output.exists())
        self.assertEqual([True], allocated)
        memory.close()

    def test_generic_upstream_loader_reads_config_before_class_refusal(self):
        api, _, memory, allocated = self.create()
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            (folder / 'memory_config.json').write_text(json.dumps(EXPECTED['normalized_config']))
            original = api.load_memory_config
            reads = []
            def counted(node):
                reads.append(True)
                return original(node)
            with patch.object(api, 'load_memory_config', counted):
                with self.assertRaisesRegex(subject.BridgeError, '^bridge_persistence_unsupported$'):
                    api.load_memory(folder)
            self.assertEqual([True], reads)
        self.assertEqual([True], allocated)
        memory.close()

    def test_constructor_failure_attempts_owned_observer_cleanup(self):
        api = prefix_api()
        created = []
        original = observed.ObservedProjectionMemory
        def observer_factory(**params):
            item = original(**params, clock=Clock())
            created.append(item)
            return item
        with patch.object(subject, 'ObservedProjectionMemory', observer_factory), \
                patch.object(observed.projection, 'NativeProjectionMemory', side_effect=OSError('PUBLIC_INIT_SENTINEL')):
            cls = subject.register_bridge(api, lambda: ('fresh', object()))
            with self.assertRaises(OSError):
                cls({'horizon': 1})
        self.assertEqual('not_needed', created[0].snapshot()['observed']['cleanup_outcome'])
        self.assertTrue(created[0].snapshot()['ledger']['frozen'])

    def test_runtime_kwargs_are_validated_ignored_without_io_and_cancel_before_work(self):
        _, _, memory, _ = self.create()
        event = threading.Event()
        runtime = copy.deepcopy(EXPECTED['adversarial']['runtime_overrides'])
        runtime['cancel_event'] = event
        memory.configure_runtime(**runtime)
        event.set()
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        with self.assertRaises(subject.BridgeCancelled):
            self.query_one(memory)
        memory.clear_query_context()
        final = memory.close()
        self.assertEqual(1, final['prework_cancelled'])
        self.assertTrue(final['runtime_overrides_ignored'])
        self.assertEqual(0, final['local_observation']['observed']['query_calls'])
        self.assertEqual(2, final['local_observation']['observed']['unlaunched_queries'])
        self.assertEqual(1, memory._observer._backend.closed)
        self.assertNotIn('PUBLIC_NO_TRACE_IO', json.dumps(final))

    def test_invalid_runtime_and_late_overrides_refuse_without_queries(self):
        _, _, memory, _ = self.create()
        for kwargs in ({'api_key': 'PUBLIC_SECRET_SENTINEL'}, {'generation_temperature': True},
                       {'generation_top_p': float('inf')}, {'cancel_event': object()}):
            with self.subTest(keys=list(kwargs)), self.assertRaises(subject.BridgeError):
                memory.configure_runtime(**kwargs)
        self.assertFalse(memory.snapshot()['runtime_overrides_ignored'])
        memory.insert(PUBLIC['trajectory'])
        with self.assertRaises(subject.BridgeError):
            memory.configure_runtime(cancel_event=threading.Event())
        memory.close()

    def test_foreign_large_hook_item_refuses_before_context_serialization(self):
        _, _, memory, _ = self.create()
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        self.query_one(memory)
        item = {'type': 'text', 'value': 'x'}
        item.update({'PUBLIC_EXTRA_%d' % index: None for index in range(10000)})
        before = memory.snapshot()
        q = PUBLIC['queries'][0]
        with patch.object(subject, '_wire', wraps=subject._wire) as serialization:
            with self.assertRaisesRegex(subject.BridgeError, '^bridge_hook_context$'):
                memory.post_query_hook(query=q['text'], query_image=None, memory_context=[item])
            self.assertEqual(1, serialization.call_count)
            self.assertEqual({'query': q['text'], 'query_image': None}, serialization.call_args.args[0])
        self.assertEqual(before, memory.snapshot())
        memory.clear_query_context()
        memory.close()

    def test_runtime_foreign_string_and_path_callbacks_are_never_invoked(self):
        callbacks = []
        class ForeignText(str):
            def __str__(self):
                callbacks.append(True)
                raise RuntimeError('PUBLIC_FOREIGN_CALLBACK')
        class ForeignPath(type(Path())):
            def __str__(self):
                callbacks.append(True)
                raise RuntimeError('PUBLIC_FOREIGN_CALLBACK')
        _, _, memory, _ = self.create()
        before = memory.snapshot()
        for trace in (ForeignText('PUBLIC_FOREIGN_TRACE'), ForeignPath('PUBLIC_FOREIGN_PATH')):
            with self.assertRaises(subject.BridgeError):
                memory.configure_runtime(query_trace_dir=trace)
        self.assertEqual([], callbacks)
        self.assertEqual(before, memory.snapshot())
        memory.close()

    def test_foreign_hook_key_equality_is_rejected_before_lookup(self):
        callbacks = []
        class ForeignKey(str):
            __hash__ = str.__hash__
            def __eq__(self, other):
                callbacks.append(True)
                raise RuntimeError('PUBLIC_FOREIGN_EQUALITY')
        _, _, memory, _ = self.create()
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        self.query_one(memory)
        before = memory.snapshot()
        q = PUBLIC['queries'][0]
        context = [{ForeignKey('type'): 'text', 'value': 'x'}]
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_hook_context$'):
            memory.post_query_hook(query=q['text'], query_image=None, memory_context=context)
        self.assertEqual([], callbacks)
        self.assertEqual(before, memory.snapshot())
        self.assertEqual(1, memory.snapshot()['local_observation']['observed']['query_calls'])
        memory.clear_query_context()
        memory.close()

    def test_foreign_runtime_keyword_equality_is_rejected_before_lookup(self):
        callbacks = []
        class ForeignKeyword(str):
            __hash__ = str.__hash__
            def __eq__(self, other):
                # The interpreter may bind keyword names before our body.
                if type(other) is str and other == 'self':
                    return str.__eq__(self, other)
                callbacks.append(True)
                raise RuntimeError('PUBLIC_FOREIGN_KEYWORD_EQUALITY')
        _, _, memory, _ = self.create()
        before = memory.snapshot()
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_runtime_parameters$'):
            memory.configure_runtime(**{ForeignKeyword('query_trace_dir'): None})
        self.assertEqual([], callbacks)
        self.assertEqual(before, memory.snapshot())
        memory.close()

    def test_malformed_bridge_query_is_unlaunched_and_id_cannot_be_replaced(self):
        _, _, memory, _ = self.create()
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        with self.assertRaises(ValueError):
            memory.query(17)
        memory.clear_query_context()
        with self.assertRaisesRegex(subject.BridgeError, '^bridge_invocation_reused$'):
            memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        result = memory.close()
        self.assertEqual(1, result['queries_attempted'])
        self.assertEqual(0, result['local_observation']['observed']['query_calls'])
        self.assertEqual(2, result['local_observation']['observed']['unlaunched_queries'])
        self.assertIsNone(result['local_observation']['ledger']['product']['tokens'])

    def test_pending_context_clear_after_close_preserves_frozen_snapshot(self):
        _, _, memory, _ = self.create()
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        final = memory.close()
        self.assertEqual(1, final['pending_contexts'])
        self.assertEqual({}, memory.get_query_context())
        memory.clear_query_context()
        memory.clear_query_context()
        self.assertEqual(final, memory.close())
        self.assertEqual(final, memory.snapshot())
        final['local_observation']['observed']['status'] = 'CALLER_CHANGED'
        self.assertNotEqual(final, memory.snapshot())

    def test_cleanup_failure_freezes_metadata_and_never_repeats_backend_close(self):
        _, _, memory, _ = self.create()
        backend = memory._observer._backend
        backend.close_error = OSError('PUBLIC_CLEANUP_SENTINEL')
        with self.assertRaises(OSError):
            memory.close()
        final = memory.snapshot()
        self.assertTrue(final['closed'])
        self.assertEqual('failed', final['local_observation']['observed']['cleanup_outcome'])
        self.assertFalse(final['local_observation']['observed']['observation_incomplete'])
        self.assertTrue(final['local_observation']['ledger']['frozen'])
        self.assertIsNone(final['local_observation']['ledger']['product']['tokens'])
        self.assertNotIn('PUBLIC_CLEANUP_SENTINEL', json.dumps(final))
        self.assertEqual(final, memory.close())
        self.assertEqual(final, memory.snapshot())
        self.assertEqual(1, backend.closed)

    def test_reentrant_mutation_cannot_clear_binding_or_close_active_query(self):
        _, _, memory, _ = self.create()
        refusals = []
        def callback():
            for action in (memory.clear_query_context, memory.close,
                           lambda: memory.set_query_context(query_invocation_id='public-nested')):
                try:
                    action()
                except subject.BridgeError:
                    refusals.append(True)
        memory._observer._backend.query_callback = callback
        _, hooks = self.invoke(memory, 0)
        self.assertEqual([True, True, True], refusals)
        self.assertEqual(EXPECTED['controlled_hooks'][0], hooks[0])
        memory.close()

    def test_interleaved_thread_hooks_keep_first_receipt_after_second_query(self):
        _, _, memory, _ = self.create()
        first_queried, second_done = threading.Event(), threading.Event()
        payloads, errors = {}, []
        def worker(index):
            try:
                if index == 1 and not first_queried.wait(3):
                    raise RuntimeError('first_query_timeout')
                memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][index])
                q = PUBLIC['queries'][index]
                try:
                    result = memory.query(q['text'], q['query_image'])
                    if index == 0:
                        first_queried.set()
                        if not second_done.wait(3):
                            raise RuntimeError('second_hook_timeout')
                    payloads[index] = memory.post_query_hook(query=q['text'], query_image=q['query_image'],
                                                              memory_context=result)
                finally:
                    memory.clear_query_context()
                    if index == 1:
                        second_done.set()
            except BaseException as error:
                errors.append(type(error).__name__)
                first_queried.set()
                second_done.set()
        threads = [threading.Thread(target=worker, args=(index,), daemon=True) for index in (0, 1)]
        try:
            for thread in threads:
                thread.start()
        finally:
            for thread in threads:
                if thread.ident is not None:
                    thread.join(4)
        self.assertTrue(all(not thread.is_alive() for thread in threads))
        self.assertEqual([], errors)
        self.assertEqual(dict(enumerate(EXPECTED['controlled_hooks'])), payloads)
        self.assertEqual(2, memory.snapshot()['local_observation']['observed']['query_calls'])
        memory.close()

    def test_metadata_export_contains_no_invocation_query_context_or_callback_text(self):
        _, _, memory, _ = self.create()
        self.invoke(memory, 0)
        final = memory.close()
        raw = json.dumps(final)
        for value in [*PUBLIC['invocation_ids'], PUBLIC['queries'][0]['text'],
                      PUBLIC['controlled_context'][0]['value'], PUBLIC['trajectory']['id']]:
            self.assertNotIn(value, raw)

    def test_hook_capacity_refusal_does_not_repeat_or_charge_query(self):
        _, _, memory, _ = self.create()
        memory.set_query_context(query_invocation_id=PUBLIC['invocation_ids'][0])
        q = PUBLIC['queries'][0]
        context = self.query_one(memory)
        for _ in range(16):
            memory.post_query_hook(query=q['text'], query_image=None, memory_context=context)
        before = memory.snapshot()
        with self.assertRaises(subject.BridgeError):
            memory.post_query_hook(query=q['text'], query_image=None, memory_context=context)
        self.assertEqual(before, memory.snapshot())
        memory.clear_query_context()
        memory.close()

    def test_registered_prefix_actual_native_corpus_is_sealed_before_support_checks(self):
        sys.path.insert(0, str(EDITION / 'memory-bank/scripts'))
        import context as native
        api = prefix_api()
        with tempfile.TemporaryDirectory() as temporary:
            project = Path(temporary)
            install(project)
            factories = []
            def factory():
                factories.append(True)
                return project, native
            cls = subject.register_bridge(api, factory)
            memory = api.build_memory(copy.deepcopy(PUBLIC['config']))
            try:
                memory.configure_runtime(query_trace_dir=project / 'unused-trace',
                                         generation_temperature=None, generation_top_p=None,
                                         cancel_event=threading.Event())
                memory.insert(copy.deepcopy(PUBLIC['trajectory']))
                results = [self.invoke(memory, index) for index in (0, 1)]
            finally:
                final = memory.close()
            envelope = {'profile': 'unchanged-upstream-prefix-registered-native-text-only',
                        'outputs_and_hooks': results, 'summary': final}
            raw = json.dumps(envelope, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()
            seal = {'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw),
                    'builder_retriever_benchmark_gold_reads': 0, 'official_full_runtime': 'UNRUN'}
            output = project / 'public-construction.json'
            output.write_bytes(raw)
            (project / 'public-construction-seal.json').write_text(json.dumps(seal))
            # Separate logical evaluator, after construction seal, no OS boundary.
            self.assertEqual(seal['sha256'], hashlib.sha256(output.read_bytes()).hexdigest())
            published = list((project / 'specs/longmemeval').glob('*.md'))
            self.assertEqual(1, len(published))
            self.assertEqual(EXPECTED['document']['text'].encode(), published[0].read_bytes())
            self.assertIn('Nacre sail calibration uses a 31 degree yaw offset.', results[0][0][0]['value'])
            self.assertIn('reference data', results[0][0][0]['value'].lower())
            self.assertEqual([], results[1][0])
            receipts = final['local_observation']['observed']['query_receipts']
            self.assertEqual(['delivered', 'no_evidence'], [r['state'] for r in receipts])
            self.assertTrue(all(r['native_metadata_bound'] and r['outcome'] == 'succeeded' for r in receipts))
            self.assertEqual(0, final['local_observation']['observed']['query_failed'])
            self.assertFalse(final['local_observation']['observed']['observation_incomplete'])
            self.assertTrue(receipts[1]['query_image_ignored'])
            self.assertIsNone(final['local_observation']['ledger']['product']['tokens'])
            self.assertIsNone(final['local_observation']['ledger']['product']['usd_micros'])
            self.assertEqual([True], factories)
            self.assertIs(api.MEMORY_TYPES[subject.MEMORY_TYPE], cls)
            self.assertFalse((project / 'unused-trace').exists())
            self.assertFalse((project / 'public-images').exists())


if __name__ == '__main__':
    unittest.main()
