"""NEW public selected-source async/sync controls; no actual SDK/provider."""
import __future__
import argparse
import asyncio
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
from typing import Any
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import longmemeval_lifecycle as base
from harness.longmemeval_bridge import PROFILE as BRIDGE
from harness.longmemeval_observed import PROFILE as LOCAL
from harness.memory_benchmark_cost import PROFILE as LEDGER
from harness.longmemeval_upstream_async import UpstreamController, PROFILE

FIXTURES = ROOT / 'tests/fixtures'
PUBLIC = json.loads((FIXTURES / 'longmemeval_upstream_async_public.json').read_text())
EXPECTED = json.loads((FIXTURES / 'longmemeval_upstream_async_expected.json').read_text())


def selected():
    names = {'longmemeval_reader_calls_2cc8c540.py': '06bb4d73591af5ea8cbaa11ad1cd94a51e49a25f900da8ff4de9433b3227f009',
             'longmemeval_judge_calls_2cc8c540.py': '42c17caa741deea9d7cc01d35aea2655fe9b2135cf97bf17cca9fc888c4b7786',
             'longmemeval_reader_helpers_2cc8c540.py': '2d2ebbcc6f7efed44f98f8194ddcb94cbc26df08a65d115a838a2cf9567e914d'}
    scope = {'Any': Any, 'argparse': argparse, 'AsyncOpenAI': object}
    for name, digest in names.items():
        raw = (FIXTURES / name).read_bytes()
        if hashlib.sha256(raw).hexdigest() != digest:
            raise AssertionError('selected_source_binding')
        exec(compile(raw, name, 'exec', flags=__future__.annotations.compiler_flag,
                     dont_inherit=True), scope)
    return scope


class Memory:
    """NEW independently supplied local profile, no native engine import/call."""
    def __init__(self):
        self.receipts = []
        self.closed = False
        self.closes = self.contexts = self.snapshots = 0
        self.close_error = None

    def set_query_context(self, **_):
        self.contexts += 1

    def clear_query_context(self):
        self.contexts -= 1

    def query(self, question, image):
        ordinal = len(self.receipts)
        self.receipts.append({'ordinal': ordinal, 'outcome': 'succeeded', 'state': 'delivered',
            'native_metadata_bound': True, 'input_bytes': EXPECTED['query_input_bytes'],
            'output_bytes': EXPECTED['query_output_bytes'], 'component_ns': PUBLIC['memory']['query_ns'][ordinal],
            'query_image_ignored': False})
        return copy.deepcopy(PUBLIC['memory']['context'])

    def post_query_hook(self, **_):
        return {'query_receipt': copy.deepcopy(self.receipts[-1])}

    def snapshot(self):
        self.snapshots += 1
        return {'profile': BRIDGE, 'closed': self.closed, 'local_observation': {
            'profile': LOCAL, 'ledger': {'profile': LEDGER, 'horizon': 2, 'frozen': self.closed},
            'observed': {'query_calls': len(self.receipts), 'insert_calls': 1,
                'query_receipts': copy.deepcopy(self.receipts), 'local_token_profile': 'pinned_local_no_model',
                'insert_ingress_bytes': PUBLIC['memory']['insert_bytes'],
                'build_component_ns': PUBLIC['memory']['build_ns'], 'observation_incomplete': False,
                'initialization_outcome': 'succeeded',
                'cleanup_outcome': 'succeeded' if self.closed else 'not_attempted'}}}

    def close(self):
        self.closed = True
        self.closes += 1
        if self.close_error is not None:
            raise self.close_error
        return self.snapshot()


class Response:
    """Supplied SDK-shaped value with independent detached serialization."""
    def __init__(self, raw):
        self.raw = raw
        value = json.loads(raw)
        self.choices = [SimpleNamespace(message=SimpleNamespace(**item['message'])) for item in value.get('choices', [])]
        if 'usage' in value:
            self.usage = SimpleNamespace(**value['usage']) if value['usage'] is not None else None


def make(*, budget=4096, clock=None, questions=None, **changes):
    helpers = selected()
    memory = Memory()
    config = {'reader_builder': helpers['build_reader_request'],
              'reader_helper': helpers['call_reader_model_async'], 'judge_helper': helpers['_call_chat_completion'],
              'reader_timeout': PUBLIC['reader_timeout'], 'judge_timeout': PUBLIC['judge_timeout'],
              'reader_model': PUBLIC['reader_model'], 'judge_model': PUBLIC['judge_model'],
              'reader_max_completion_tokens': PUBLIC['reader_max'],
              'judge_max_completion_tokens': PUBLIC['judge_max'], 'reader_budget': budget, 'clock': clock}
    config.update(changes)
    return UpstreamController(memory, copy.deepcopy(PUBLIC['questions']) if questions is None else questions, **config), memory


def response(index=1, raw=None):
    return Response(PUBLIC['responses'][index] if raw is None else raw)


def serialize(item):
    return item.raw


async def reads(ctrl):
    ctrl.query(0)
    ctrl.query(1)
    async def first(**_):
        return response(0)
    try:
        await ctrl.dispatch_reader(ctrl.prepare_reader(0), first, serialize)
    except RuntimeError as error:
        if str(error) != 'Model returned empty text':
            raise
    else:
        raise AssertionError('actual upstream require did not refuse')
    returns = []
    for ordinal, index in ((0, 1), (1, 2)):
        async def create(index=index, **_):
            return response(index)
        returns.append(await ctrl.dispatch_reader(ctrl.prepare_reader(ordinal), create, serialize))
    return returns


class UpstreamAsyncTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='memory-upstream-cobalt-')
        self.addCleanup(self.temp.cleanup)

    async def test_complete_literal_kwargs_gate_cost_and_sync_judge_oracles(self):
        ticks = iter(PUBLIC['clock_points'])
        ctrl, memory = make(clock=lambda: next(ticks))
        self.assertEqual([list(value) for value in await reads(ctrl)], EXPECTED['reader_returns'])
        gate = ctrl.seal_construction()
        self.assertEqual(gate.body, EXPECTED['construction_utf8'].encode())
        self.assertEqual(gate.sha256, EXPECTED['construction_sha256'])
        self.assertEqual(memory.closes, 1)
        packet = Path(self.temp.name) / 'reader-packet.json'
        packet.write_bytes(gate.body)
        submitted, lookups = [], []
        for ordinal in range(2):
            def factory(ordinal=ordinal):
                self.assertEqual(packet.read_bytes(), EXPECTED['construction_utf8'].encode())
                lookups.append(gate.sha256)
                def create(**kwargs):
                    submitted.append(kwargs)
                    return response(ordinal + 3)
                return create, serialize, json.loads
            self.assertEqual(ctrl.dispatch_judge(ctrl.prepare_judge(ordinal, gate), factory), ordinal == 0)
        self.assertEqual(submitted, EXPECTED['judge_requests'])
        final = ctrl.finalize()
        self.assertEqual(final['profile'], PROFILE)
        self.assertEqual(final['sdk_create_boundaries'], 5)
        self.assertEqual(final['callback_entries'], 5)
        for scope in ('product', 'experiment'):
            item = final['ledger'][scope]
            for key, value in EXPECTED[scope + '_known'].items():
                field = {'tokens': 'known_tokens', 'input': 'known_input_tokens', 'output': 'known_output_tokens',
                         'component_ns': 'known_component_ns', 'bytes': 'known_constructed_bytes'}[key]
                self.assertEqual(item[field], value)
            self.assertIsNone(item['tokens'])
            self.assertIsNone(item['usd_micros'])
            self.assertFalse(item['complete']['coverage'])
        journal = ctrl.private_judge_journal()
        self.assertEqual(json.loads(journal.body)['reader_construction_sha256'], gate.sha256)
        self.assertEqual([row['request'] for row in json.loads(journal.body)['judge_attempts']], EXPECTED['judge_requests'])
        self.assertEqual(memory.contexts, 0)
        self.assertEqual(len(lookups), 2)

    async def test_real_await_cancel_books_once_and_blocks_competing_mutations(self):
        ctrl, _ = make()
        ctrl.query(0)
        ctrl.query(1)
        ticket, other = ctrl.prepare_reader(0), ctrl.prepare_reader(1)
        entered, release = asyncio.Event(), asyncio.Event()
        async def create(**_):
            entered.set()
            await release.wait()
            return response()
        job = asyncio.create_task(ctrl.dispatch_reader(ticket, create, serialize))
        await entered.wait()
        for mutation in (lambda: ctrl.prepare_reader(0), ctrl.seal_construction, ctrl.finalize):
            with self.assertRaises(base.ControllerError):
                mutation()
        with self.assertRaises(base.ControllerError):
            await ctrl.dispatch_reader(other, create, serialize)
        self.assertEqual(ctrl.snapshot()['sdk_create_boundaries'], 1)
        job.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await job
        snap = ctrl.snapshot()
        self.assertEqual(snap['reader_attempts'][0]['outcome'], 'cancelled')
        self.assertIsNone(snap['reader_attempts'][0]['usage']['total_tokens'])
        self.assertEqual(snap['ledger']['phases']['reader']['finished_attempts'], 1)
        ctrl.seal_construction()  # busy released; second unused reservation retained

    async def test_paid_choices_decode_error_survives_supplied_usage(self):
        ctrl, _ = make()
        ctrl.query(0)
        value = json.loads(PUBLIC['responses'][0])
        value['choices'] = []
        async def create(**_):
            return response(raw=base._wire(value).decode())
        with self.assertRaises(IndexError):
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        row = ctrl.snapshot()['reader_attempts'][0]
        self.assertEqual(row['usage']['total_tokens'], 67)
        self.assertEqual(row['outcome'], 'failed')
        self.assertEqual(ctrl.snapshot()['ledger']['phases']['reader']['finished_attempts'], 1)

    async def test_partial_usage_outward_zeros_do_not_become_ledger_cost(self):
        ctrl, _ = make()
        ctrl.query(0)
        async def create(**_):
            return response(raw=PUBLIC['partial_response'])
        outward = await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        self.assertEqual(list(outward), EXPECTED['partial_return'])
        snap = ctrl.snapshot()
        self.assertEqual(snap['reader_attempts'][0]['usage'], EXPECTED['partial_observation'])
        self.assertIsNone(snap['ledger']['product']['tokens'])
        self.assertEqual(snap['ledger']['product']['known_input_tokens'], 31)
        self.assertEqual(snap['ledger']['product']['known_tokens'], 0)  # no complete usable input/output pair

    async def test_missing_usage_stays_unknown_despite_compatibility_zero_tuple(self):
        ctrl, _ = make()
        ctrl.query(0)
        value = json.loads(PUBLIC['responses'][1])
        del value['usage']
        async def create(**_):
            return response(raw=base._wire(value).decode())
        outward = await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        self.assertEqual(outward[1], {'prompt_tokens': 0, 'completion_tokens': 0, 'total_tokens': 0})
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['usage_state'], 'missing')
        self.assertIsNone(ctrl.snapshot()['reader_attempts'][0]['usage']['total_tokens'])

    async def test_actual_reader_reasoning_fallback_is_preserved(self):
        ctrl, _ = make()
        ctrl.query(0)
        async def create(**_):
            return response(raw=PUBLIC['reasoning_response'])
        outward = await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        self.assertEqual(outward[0], 'public reasoning fallback')
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['usage']['total_tokens'], 67)

    async def test_observer_failures_allow_original_helper_and_mark_unmeasured(self):
        for bad in (42, b'x' * 524289, '\ud800'):
            ctrl, _ = make()
            ctrl.query(0)
            async def create(**_):
                return response()
            outward = await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, lambda _: bad)
            self.assertEqual(outward[0], PUBLIC['answer'])
            self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['usage_state'], 'unmeasured')
            self.assertIsNone(ctrl.snapshot()['reader_attempts'][0]['usage']['total_tokens'])
        ctrl, _ = make()
        ctrl.query(0)
        def broken(_):
            raise RuntimeError('unretained observer error')
        self.assertEqual((await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, broken))[0], PUBLIC['answer'])
        self.assertGreater(ctrl.snapshot()['observation_faults'], 0)

    async def test_serializer_cancel_finishes_after_paid_response_but_unknown_usage(self):
        ctrl, _ = make()
        ctrl.query(0)
        error = asyncio.CancelledError()
        async def create(**_):
            return response()
        def cancel(_):
            raise error
        with self.assertRaises(asyncio.CancelledError) as caught:
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, cancel)
        self.assertIs(caught.exception, error)
        snap = ctrl.snapshot()
        self.assertEqual(snap['reader_attempts'][0]['outcome'], 'cancelled')
        self.assertIsNone(snap['reader_attempts'][0]['usage']['total_tokens'])
        self.assertEqual(snap['ledger']['phases']['reader']['finished_attempts'], 1)

    async def test_one_serialization_and_one_finish_with_paid_empty_response(self):
        ctrl, _ = make()
        ctrl.query(0)
        calls = []
        async def create(**_):
            return response(0)
        def once(item):
            calls.append(True)
            if len(calls) > 1:
                raise RuntimeError('second serialization')
            return item.raw
        with self.assertRaises(RuntimeError):
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, once)
        self.assertEqual(len(calls), 1)
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['usage']['total_tokens'], 67)
        self.assertEqual(ctrl.snapshot()['ledger']['phases']['reader']['finished_attempts'], 1)

    async def test_two_suspended_sessions_cannot_exchange_owned_receipts(self):
        first, _ = make()
        second, _ = make()
        ready = [asyncio.Event(), asyncio.Event()]
        release = asyncio.Event()
        async def create(index, **_):
            ready[index].set()
            await release.wait()
            return response(index + 1)
        jobs = []
        for index, ctrl in enumerate((first, second)):
            ctrl.query(0)
            async def send(index=index, **kwargs):
                return await create(index, **kwargs)
            jobs.append(asyncio.create_task(ctrl.dispatch_reader(ctrl.prepare_reader(0), send, serialize)))
        await asyncio.gather(*(event.wait() for event in ready))
        self.assertEqual(first.snapshot()['sdk_create_boundaries'], 1)
        self.assertEqual(second.snapshot()['sdk_create_boundaries'], 1)
        release.set()
        await asyncio.gather(*jobs)
        self.assertEqual(first.snapshot()['reader_attempts'][0]['usage']['total_tokens'], 75)
        self.assertEqual(second.snapshot()['reader_attempts'][0]['usage']['total_tokens'], 78)

    async def test_complete_sdk_argument_budget_retain_drop_and_empty_refusal(self):
        for budget, expected in ((363, 'reader_utf8'), (362, 'empty_utf8')):
            ctrl, _ = make(budget=budget)
            ctrl.query(0)
            seen = []
            async def create(**kwargs):
                seen.append(base._wire(kwargs))
                return response()
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
            self.assertEqual(seen, [EXPECTED[expected].encode()])
        ctrl, _ = make(budget=255)
        ctrl.query(0)
        with self.assertRaises(base.ControllerError):
            ctrl.prepare_reader(0)
        self.assertEqual(ctrl.snapshot()['sdk_create_boundaries'], 0)

    async def test_same_body_different_ordinals_and_replayed_foreign_tickets(self):
        one, _ = make()
        two, _ = make()
        one.query(0)
        one.query(1)
        two.query(0)
        ticket, foreign = one.prepare_reader(0), two.prepare_reader(0)
        seen = []
        async def create(**kwargs):
            seen.append(base._wire(kwargs))
            return response()
        with self.assertRaises(base.ControllerError):
            await one.dispatch_reader(foreign, create, serialize)
        await one.dispatch_reader(ticket, create, serialize)
        with self.assertRaises(base.ControllerError):
            await one.dispatch_reader(ticket, create, serialize)
        await one.dispatch_reader(one.prepare_reader(1), create, serialize)
        self.assertEqual(seen, [EXPECTED['reader_utf8'].encode()] * 2)
        self.assertEqual([r['ordinal'] for r in one.snapshot()['reader_attempts']], [0, 1])

    async def test_actual_argument_mismatch_refuses_before_injected_create(self):
        helpers = selected()
        async def altered(client, args, messages):
            request = helpers['build_reader_request'](args, messages)
            request['stream'] = True
            return await client.chat.completions.create(**request)
        ctrl, _ = make(reader_helper=altered)
        ctrl.query(0)
        seen = []
        async def create(**_):
            seen.append(True)
        with self.assertRaises(base.ControllerError):
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        self.assertEqual(seen, [])
        self.assertEqual(ctrl.snapshot()['sdk_create_boundaries'], 0)

    async def test_changed_builder_profile_refuses_before_ticket(self):
        helpers = selected()
        def altered(args, messages):
            req = helpers['build_reader_request'](args, messages)
            req['max_tokens'] = req.pop('max_completion_tokens')
            return req
        ctrl, _ = make(reader_builder=altered)
        ctrl.query(0)
        with self.assertRaises(base.ControllerError):
            ctrl.prepare_reader(0)
        self.assertEqual(ctrl.snapshot()['pending_tickets'], 0)

    async def test_caller_mutation_during_await_cannot_rewrite_held_arguments(self):
        questions = copy.deepcopy(PUBLIC['questions'])
        ctrl, _ = make(questions=questions)
        ctrl.query(0)
        seen = []
        async def create(**kwargs):
            await asyncio.sleep(0)
            questions[0]['question'] = 'foreign edited prompt'
            seen.append(base._wire(kwargs))
            return response()
        await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        self.assertEqual(seen, [EXPECTED['reader_utf8'].encode()])

    async def test_start_clock_cancel_has_zero_entries_and_arguments(self):
        error = KeyboardInterrupt()
        ctrl, _ = make(clock=lambda: (_ for _ in ()).throw(error))
        ctrl.query(0)
        async def create(**_):
            raise AssertionError('must not enter')
        with self.assertRaises(KeyboardInterrupt) as caught:
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        self.assertIs(caught.exception, error)
        snap = ctrl.snapshot()
        self.assertEqual(snap['sdk_create_boundaries'], 0)
        self.assertEqual(snap['callback_entries'], 0)
        self.assertEqual(snap['reader_attempts'][0]['sdk_argument_bytes'], 0)

    async def test_paid_upstream_error_precedes_end_clock_cancellation(self):
        points = iter([10, KeyboardInterrupt()])
        def clock():
            value = next(points)
            if not isinstance(value, int):
                raise value
            return value
        ctrl, _ = make(clock=clock)
        ctrl.query(0)
        async def create(**_):
            return response(0)
        with self.assertRaisesRegex(RuntimeError, '^Model returned empty text$'):
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['usage']['total_tokens'], 67)

    async def test_successful_paid_answer_survives_end_clock_cancel(self):
        points = iter([10, KeyboardInterrupt()])
        def clock():
            value = next(points)
            if not isinstance(value, int):
                raise value
            return value
        ctrl, _ = make(clock=clock)
        ctrl.query(0)
        async def create(**_):
            return response()
        with self.assertRaises(KeyboardInterrupt):
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['outcome'], 'succeeded')
        self.assertEqual(ctrl.snapshot()['ledger']['product']['known_tokens'], 75)

    async def test_sync_judge_empty_string_list_and_local_grade_failure_paid(self):
        for content in ('   ', [], 'not-json'):
            ctrl, _ = make()
            await reads(ctrl)
            gate = ctrl.seal_construction()
            value = json.loads(PUBLIC['responses'][3])
            value['choices'][0]['message']['content'] = content
            seen, grade_inputs = [], []
            def factory():
                def create(**kwargs):
                    seen.append(kwargs)
                    return response(raw=base._wire(value).decode())
                def decode(text):
                    grade_inputs.append(text)
                    return json.loads(text)
                return create, serialize, decode
            with self.assertRaises(ValueError):
                ctrl.dispatch_judge(ctrl.prepare_judge(0, gate), factory)
            self.assertEqual(seen, [EXPECTED['judge_requests'][0]])
            self.assertEqual(grade_inputs, [] if content == [] else ['' if content == '   ' else 'not-json'])
            self.assertEqual(ctrl.snapshot()['judge_attempts'][0]['usage']['total_tokens'], 26)
            self.assertEqual(ctrl.snapshot()['judge_attempts'][0]['outcome'], 'failed')

    async def test_factory_gate_and_factory_failure_zero_create_bytes(self):
        ctrl, _ = make()
        await reads(ctrl)
        calls = []
        def factory():
            calls.append(True)
            raise ValueError('unretained public factory error')
        with self.assertRaises(base.ControllerError):
            ctrl.prepare_judge(0, base.Construction(b'{}'))
        self.assertEqual(calls, [])
        gate = ctrl.seal_construction()
        with self.assertRaises(ValueError):
            ctrl.dispatch_judge(ctrl.prepare_judge(0, gate), factory)
        self.assertEqual(calls, [True])
        self.assertEqual(ctrl.snapshot()['judge_attempts'][0]['sdk_argument_bytes'], 0)
        self.assertEqual(ctrl.snapshot()['judge_attempts'][0]['sdk_create_boundaries'], 0)

    async def test_reader_held_refusal_and_whole_seal_refusal_preserve_cost(self):
        ctrl, _ = make()
        ctrl.query(0)
        ticket = ctrl.prepare_reader(0)
        async def create(**_):
            return response()
        with patch.object(base, 'MAX_HELD', ctrl._held):
            with self.assertRaises(base.ControllerError):
                await ctrl.dispatch_reader(ticket, create, serialize)
        self.assertEqual(ctrl.snapshot()['ledger']['product']['known_tokens'], 75)
        with patch.object(base, 'MAX_HELD', ctrl._held):
            with self.assertRaises(base.ControllerError):
                ctrl.seal_construction()
        self.assertTrue(ctrl.snapshot()['construction_blocked'])
        self.assertIsNone(ctrl.snapshot()['construction_sha256'])

    async def test_json_snapshot_excludes_prompt_model_response_and_error_prose(self):
        ctrl, _ = make()
        await reads(ctrl)
        ctrl.seal_construction()
        final = ctrl.finalize()
        encoded = json.dumps(final)
        for text in ('PUBLIC-COBALT-ID', 'Cobalt coolant limit?', PUBLIC['reader_model'], PUBLIC['answer'], 'Model returned empty text'):
            self.assertNotIn(text, encoded)
        self.assertEqual(final['provider_coverage'], 'UNKNOWN')
        self.assertIsNone(final['physical_attempts'])
        self.assertFalse(final['provider_usage_verified'])
        self.assertEqual(final['request_byte_scope'], 'canonical_sdk_argument_proxy')
        self.assertEqual(final['response_byte_scope'], 'supplied_serialization_proxy')

    async def test_pre_dispatch_cancel_never_enters_and_sync_dispatch_api_refuses(self):
        ctrl, _ = make()
        ctrl.query(0)
        ticket = ctrl.prepare_reader(0)
        async def create(**_):
            raise AssertionError('unlaunched')
        job = asyncio.create_task(ctrl.dispatch_reader(ticket, create, serialize))
        job.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await job
        self.assertEqual(ctrl.snapshot()['callback_entries'], 0)
        self.assertEqual(ctrl.snapshot()['sdk_create_boundaries'], 0)
        with self.assertRaises(base.ControllerError):
            ctrl.dispatch(ticket, create, serialize)

    async def test_timeout_helpers_and_images_refuse_before_memory_observation(self):
        for bad in (True, float('nan'), float('inf'), 0, 3601):
            memory = Memory()
            helpers = selected()
            with self.assertRaises(base.ControllerError):
                UpstreamController(memory, PUBLIC['questions'], reader_builder=helpers['build_reader_request'],
                    reader_helper=helpers['call_reader_model_async'], judge_helper=helpers['_call_chat_completion'],
                    reader_timeout=bad, judge_timeout=9.0)
            self.assertEqual(memory.snapshots, 0)
        with self.assertRaises(base.ControllerError):
            make(reader_helper=42)
        questions = copy.deepcopy(PUBLIC['questions'])
        questions[0]['query_image'] = 'unsupported-image'
        with self.assertRaises(base.ControllerError):
            make(questions=questions)

    async def test_cleanup_cancel_imports_available_local_subtotal_once(self):
        ticks = iter([1, 5])
        ctrl, memory = make(clock=lambda: next(ticks))
        ctrl.query(0)
        async def create(**_):
            return response()
        await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        error = asyncio.CancelledError()
        memory.close_error = error
        with self.assertRaises(asyncio.CancelledError) as caught:
            ctrl.seal_construction()
        self.assertIs(caught.exception, error)
        self.assertIsNone(ctrl.snapshot()['construction_sha256'])
        final = ctrl.finalize()
        self.assertEqual(memory.closes, 1)
        self.assertEqual(final['local_imports'], 1)
        self.assertEqual(final['ledger']['product']['known_tokens'], 75)
        local_bytes = PUBLIC['memory']['insert_bytes'] + EXPECTED['query_input_bytes'] + EXPECTED['query_output_bytes']
        self.assertEqual(final['ledger']['product']['known_constructed_bytes'], local_bytes + EXPECTED['reader_bytes'])
        self.assertEqual(final['ledger']['product']['known_component_ns'], 7 + 2 + 4)

    async def test_reflective_abort_uses_actual_type_and_never_strands_busy(self):
        class ReflectiveAbort(BaseException):
            @property
            def __class__(self):
                raise RuntimeError('must not be reflected')
        ctrl, _ = make()
        ctrl.query(0)
        error = ReflectiveAbort()
        async def create(**_):
            raise error
        try:
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        except BaseException as caught:
            self.assertIs(caught, error)
        else:
            self.fail('original abort missing')
        snap = ctrl.snapshot()
        self.assertEqual(snap['reader_attempts'][0]['outcome'], 'cancelled')
        self.assertEqual(snap['ledger']['phases']['reader']['finished_attempts'], 1)
        self.assertFalse(ctrl._busy)
        ctrl.seal_construction()

    async def test_post_admission_namespace_failure_finishes_without_sdk_entry(self):
        ctrl, _ = make()
        ctrl.query(0)
        ticket = ctrl.prepare_reader(0)
        error = RuntimeError('unretained client setup failure')
        async def create(**_):
            raise AssertionError('must not enter')
        with patch('harness.longmemeval_upstream_async.SimpleNamespace', side_effect=error):
            with self.assertRaises(RuntimeError) as caught:
                await ctrl.dispatch_reader(ticket, create, serialize)
        self.assertIs(caught.exception, error)
        snap = ctrl.snapshot()
        self.assertEqual(snap['sdk_create_boundaries'], 0)
        self.assertEqual(snap['ledger']['phases']['reader']['finished_attempts'], 1)
        self.assertFalse(ctrl._busy)
        ctrl.seal_construction()

    async def test_finish_failure_preserves_original_task_error_and_blocks_publication(self):
        ctrl, _ = make()
        ctrl.query(0)
        original = ValueError('original create error')
        async def create(**_):
            raise original
        with patch.object(ctrl._ledger, 'finish_call', side_effect=RuntimeError('bookkeeping failure')) as finish:
            with self.assertRaises(ValueError) as caught:
                await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
            self.assertEqual(finish.call_count, 1)
        self.assertIs(caught.exception, original)
        self.assertFalse(ctrl._busy)
        self.assertTrue(ctrl.snapshot()['construction_blocked'])
        self.assertIsNone(ctrl.snapshot()['ledger']['product']['tokens'])
        self.assertGreater(ctrl.snapshot()['observation_faults'], 0)

    async def test_snapshot_cancel_cannot_replace_original_upstream_task_error(self):
        from harness.longmemeval_provider_usage import CompletionCapture
        ctrl, _ = make()
        ctrl.query(0)
        async def create(**_):
            return response(0)
        with patch.object(CompletionCapture, 'snapshot', side_effect=asyncio.CancelledError()):
            with self.assertRaisesRegex(RuntimeError, '^Model returned empty text$'):
                await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
        self.assertEqual(ctrl.snapshot()['ledger']['phases']['reader']['finished_attempts'], 1)
        self.assertFalse(ctrl._busy)

    async def test_secondary_cleanup_error_preserves_original_cancel_and_no_retry(self):
        for failure in ('exception', 'invalid'):
            ctrl, memory = make()
            ctrl.query(0)
            async def create(**_):
                return response()
            await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, serialize)
            error = KeyboardInterrupt()
            memory.close_error = error
            def unavailable():
                if failure == 'exception':
                    raise RuntimeError('unavailable final observation')
                return {'closed': True}
            with patch.object(memory, 'snapshot', unavailable):
                with self.assertRaises(KeyboardInterrupt) as caught:
                    ctrl.seal_construction()
            self.assertIs(caught.exception, error)
            self.assertIsNone(ctrl.snapshot()['construction_sha256'])
            self.assertFalse(ctrl._busy)
            final = ctrl.finalize()
            self.assertTrue(final['ledger']['frozen'])
            self.assertEqual(memory.closes, 1)
            self.assertEqual(final['local_imports'], 0)

    async def test_large_integer_timeouts_have_fixed_refusal_before_memory(self):
        helpers = selected()
        for bad in (10 ** 400, -(10 ** 400)):
            memory = Memory()
            with self.assertRaisesRegex(base.ControllerError, '^upstream_timeout$'):
                UpstreamController(memory, PUBLIC['questions'], reader_builder=helpers['build_reader_request'],
                    reader_helper=helpers['call_reader_model_async'], judge_helper=helpers['_call_chat_completion'],
                    reader_timeout=bad, judge_timeout=9.0)
            self.assertEqual(memory.snapshots, 0)

    async def test_sdk_response_identity_is_not_reflected_by_observation_decoder(self):
        class ReflectiveResponse(Response):
            @property
            def __class__(self):
                raise KeyboardInterrupt()
        ctrl, _ = make()
        ctrl.query(0)
        obj = ReflectiveResponse(PUBLIC['responses'][1])
        seen = []
        async def create(**_):
            return obj
        def observed(item):
            self.assertIs(item, obj)
            seen.append(True)
            return item.raw
        value = await ctrl.dispatch_reader(ctrl.prepare_reader(0), create, observed)
        self.assertEqual(value[0], PUBLIC['answer'])
        self.assertEqual(seen, [True])
        self.assertEqual(ctrl.snapshot()['reader_attempts'][0]['usage']['total_tokens'], 75)


if __name__ == '__main__':
    unittest.main()
