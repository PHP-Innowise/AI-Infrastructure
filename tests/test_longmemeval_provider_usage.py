"""Fresh supplied-response controls; no official SDK, gold or model imports."""
import __future__
import ast
import asyncio
import copy
import hashlib
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import longmemeval_provider_usage as observed

FIXTURES = ROOT / 'tests/fixtures'
PUBLIC = json.loads((FIXTURES / 'longmemeval_provider_usage_public.json').read_text())
EXPECTED = json.loads((FIXTURES / 'longmemeval_provider_usage_expected.json').read_text())


def raw(name='paid'):
    return PUBLIC['raw_responses'][name].encode('utf-8')


def encode(value):
    return json.dumps(value, separators=(',', ':')).encode()


def namespace(value):
    if type(value) is dict:
        return SimpleNamespace(**{k: namespace(v) for k, v in value.items()})
    if type(value) is list:
        return [namespace(v) for v in value]
    return value


def selected_source(filename, globals_):
    data = (FIXTURES / filename).read_bytes()
    pins = {'longmemeval_reader_calls_2cc8c540.py': '06bb4d73591af5ea8cbaa11ad1cd94a51e49a25f900da8ff4de9433b3227f009', 'longmemeval_judge_calls_2cc8c540.py': '42c17caa741deea9d7cc01d35aea2655fe9b2135cf97bf17cca9fc888c4b7786'}
    if hashlib.sha256(data).hexdigest() != pins[filename]:
        raise AssertionError('selected_source_pin')
    # Exactly pinned selected functions, without whole heavy runner imports.
    tree = ast.parse(data)
    exec(compile(tree, filename, 'exec', flags=__future__.annotations.compiler_flag), globals_)
    return globals_


class ProviderUsageTests(unittest.TestCase):
    def assert_unknown(self, result, state):
        self.assertEqual(result['usage_state'], state)
        self.assertEqual(result['usage'], dict.fromkeys(observed.KEYS))
        self.assertEqual(result['provider_coverage'], 'UNKNOWN')
        self.assertIsNone(result['physical_attempts'])
        self.assertIsNone(result['usd_micros'])
        self.assertFalse(result['usage_verified'])

    def test_independent_literal_response_projection_oracles(self):
        for name in PUBLIC['raw_responses']:
            with self.subTest(name=name):
                self.assertEqual(observed.completion_usage(raw(name)), EXPECTED[name])

    def test_subsets_and_prediction_extensions_are_not_added_to_totals(self):
        result = observed.completion_usage(raw())
        self.assertEqual(result['usage']['total_tokens'], 49)
        self.assertEqual(result['usage']['input_tokens'] - result['usage']['cached_input_tokens'], 29)
        self.assertEqual(result['usage']['output_tokens'], 12)
        self.assertEqual(set(result['usage']), set(observed.KEYS))

    def test_absent_total_and_details_are_not_inferred(self):
        data = {'object': 'chat.completion', 'usage': {'prompt_tokens': 9, 'completion_tokens': 2}}
        result = observed.completion_usage(encode(data))
        self.assertEqual(result['usage_state'], 'partial')
        self.assertEqual(list(result['usage'].values()), [9, 2, None, None, None])
        self.assertIsNone(EXPECTED['no_details']['usage']['cached_input_tokens'])
        self.assertIsNone(EXPECTED['no_details']['usage']['reasoning_output_tokens'])

    def test_invalid_types_totals_and_details_invalidate_whole_tuple(self):
        controls = [('prompt_tokens', True), ('completion_tokens', 12.0), ('total_tokens', 48),
                    ('prompt_tokens', -1), ('prompt_tokens', '37'),
                    ('total_tokens', 2 ** 63), ('prompt_tokens_details', {'cached_tokens': 38}),
                    ('completion_tokens_details', {'reasoning_tokens': 13}),
                    ('prompt_tokens_details', []), ('prompt_tokens_details', {'cached_tokens': False})]
        for field, value in controls:
            with self.subTest(field=field, value=value):
                data = json.loads(raw())
                data['usage'][field] = value
                self.assert_unknown(observed.completion_usage(encode(data)), 'invalid')

    def test_pair_overflow_orphan_subset_and_partial_underflow(self):
        for usage in ({'prompt_tokens': 2 ** 63 - 1, 'completion_tokens': 1},
                      {'prompt_tokens_details': {'cached_tokens': 1}},
                      {'prompt_tokens': 9, 'total_tokens': 8}):
            self.assert_unknown(observed.completion_usage(encode({'object': 'chat.completion', 'usage': usage})), 'invalid')
        largest = {'prompt_tokens': 2 ** 63 - 1, 'completion_tokens': 0,
                   'total_tokens': 2 ** 63 - 1, 'prompt_tokens_details': {'cached_tokens': 2 ** 63 - 1}}
        self.assertEqual(observed.completion_usage(encode({'object': 'chat.completion', 'usage': largest}))['usage_state'], 'complete')

    def test_unknown_usage_extensions_refuse_this_profile(self):
        data = json.loads(raw())
        data['usage']['new_cost_family'] = 1
        self.assert_unknown(observed.completion_usage(encode(data)), 'unsupported')
        del data['usage']['new_cost_family']
        data['usage']['prompt_tokens_details']['new_cost_family'] = 1
        self.assert_unknown(observed.completion_usage(encode(data)), 'unsupported')

    def test_invalid_json_encoding_duplicates_nonfinite_and_surrogates(self):
        malformed = [b'{', b'\xff',
                     b'{"object":"chat.completion","usage":{"prompt_tokens":1,"prompt_tokens":2}}',
                     b'{"object":"chat.completion","usage":{"prompt_tokens":1,"\\u0070rompt_tokens":2}}',
                     b'{"object":"chat.completion","usage":{"prompt_tokens":NaN}}',
                     b'{"object":"chat.completion","usage":{"prompt_tokens":1e999}}',
                     b'{"object":"chat.completion","id":"\\ud800"}']
        for data in malformed:
            self.assert_unknown(observed.completion_usage(data), 'invalid')

    def test_payload_admission_refuses_without_invented_zero_usage(self):
        nested = '[' * 17 + '0' + ']' * 17
        over = [b' ' * (observed.RAW_BYTES + 1), nested.encode(),
                encode({str(i): 0 for i in range(65)}), encode([0] * 1025),
                encode({'object': 'chat.completion', 'extra': 'a' * (observed.STRING_CHARS + 1)}),
                ('{"object":"chat.completion","created":' + '1' * 65 + '}').encode(),
                encode({'object': 'chat.completion', 'extra': [[0] * 1024 for _ in range(4)]})]
        for data in over:
            self.assert_unknown(observed.completion_usage(data), 'unmeasured')
        for data in (bytearray(raw()), raw().decode(), None):
            self.assert_unknown(observed.completion_usage(data), 'unmeasured')

    def test_exact_raw_admission_and_string_brackets_do_not_fake_depth(self):
        padded = raw() + b' ' * (observed.RAW_BYTES - len(raw()))
        result = observed.completion_usage(padded)
        self.assertEqual(result['response_bytes'], observed.RAW_BYTES)
        self.assertEqual(result['usage'], EXPECTED['paid']['usage'])
        data = json.loads(raw())
        data['metadata'] = {'text': '[' * 50 + '\\"' + ']' * 50}
        self.assertEqual(observed.completion_usage(encode(data))['usage'], EXPECTED['paid']['usage'])

    def test_capture_precedes_decoder_and_passes_same_input_and_result(self):
        capture = observed.CompletionCapture()
        supplied = raw()
        result = object()
        def decoder(received):
            self.assertIs(received, supplied)
            self.assertEqual(capture.snapshot()['decoder_state'], 'running')
            self.assertEqual(capture.snapshot()['observation'], EXPECTED['paid'])
            return result
        self.assertIs(capture.deliver(supplied, decoder), result)
        self.assertEqual(capture.snapshot()['decoder_state'], 'succeeded')

    def test_paid_capture_survives_decoder_error_and_cancel(self):
        for error in (ValueError(PUBLIC['decoder_error']), KeyboardInterrupt(), SystemExit(7)):
            capture = observed.CompletionCapture()
            def decoder(_):
                raise error
            with self.assertRaises(type(error)) as caught:
                capture.deliver(raw(), decoder)
            self.assertIs(caught.exception, error)
            self.assertEqual(capture.snapshot()['observation'], EXPECTED['paid'])
            self.assertEqual(capture.snapshot()['decoder_state'], 'failed' if isinstance(error, Exception) else 'cancelled')

    def test_capture_replay_reentry_and_identical_instances_are_distinct(self):
        one, two = observed.CompletionCapture(), observed.CompletionCapture()
        calls = []
        def decoder(data):
            calls.append(data)
            with self.assertRaisesRegex(observed.UsageError, '^capture_consumed$'):
                one.deliver(data, calls.append)
            return 9
        self.assertEqual(one.deliver(raw(), decoder), 9)
        with self.assertRaises(observed.UsageError):
            one.deliver(raw('zero'), calls.append)
        two.deliver(raw(), calls.append)
        self.assertEqual(len(calls), 2)
        self.assertEqual(one.snapshot()['observation'], two.snapshot()['observation'])

    def test_snapshot_mutation_and_privacy_sentinels_never_escape(self):
        capture = observed.CompletionCapture()
        capture.deliver(raw(), lambda _: PUBLIC['decoder_success'])
        exported = capture.snapshot()
        exported['observation']['usage']['input_tokens'] = 999
        self.assertEqual(capture.snapshot()['observation'], EXPECTED['paid'])
        text = json.dumps(capture.snapshot())
        for sentinel in ('PUBLIC-ID-NEVER-EXPORT', 'Public cedar answer; never metadata.',
                         PUBLIC['decoder_success'], PUBLIC['decoder_error']):
            self.assertNotIn(sentinel, text)
        self.assertFalse(hasattr(capture, '__dict__'))

    def test_observation_failure_continues_but_observation_cancel_propagates(self):
        capture = observed.CompletionCapture()
        entered = []
        with patch.object(observed, 'completion_usage', side_effect=RuntimeError('unretained')):
            self.assertEqual(capture.deliver(raw(), lambda data: entered.append(data) or 5), 5)
        self.assertEqual(len(entered), 1)
        self.assert_unknown(capture.snapshot()['observation'], 'unmeasured')
        cancelled = observed.CompletionCapture()
        error = KeyboardInterrupt()
        with patch.object(observed, 'completion_usage', side_effect=error):
            with self.assertRaises(KeyboardInterrupt) as caught:
                cancelled.deliver(raw(), entered.append)
        self.assertIs(caught.exception, error)
        self.assertEqual(len(entered), 1)
        self.assertEqual(cancelled.snapshot()['decoder_state'], 'not_entered')
        self.assert_unknown(cancelled.snapshot()['observation'], 'unmeasured')

    def test_observation_refusal_does_not_stop_decoder_and_awaitable_is_not_complete(self):
        capture = observed.CompletionCapture()
        supplied = b' ' * (observed.RAW_BYTES + 1)
        self.assertIs(capture.deliver(supplied, lambda data: data), supplied)
        self.assert_unknown(capture.snapshot()['observation'], 'unmeasured')
        async def decoder(_):
            return 3
        pending = observed.CompletionCapture()
        value = pending.deliver(raw(), decoder)
        try:
            self.assertEqual(pending.snapshot()['decoder_state'], 'returned_awaitable')
            self.assertEqual(asyncio.run(value), 3)
            # Externally awaiting does not upgrade this capture's completion state.
            self.assertEqual(pending.snapshot()['decoder_state'], 'returned_awaitable')
        finally:
            value.close()

    def test_result_classification_failure_and_cancel_retain_paid_observation(self):
        for error in (RuntimeError('unretained'), KeyboardInterrupt()):
            class Result:
                @property
                def __class__(self):
                    raise error
            value = Result()
            capture = observed.CompletionCapture()
            if type(error) is RuntimeError:
                self.assertIs(capture.deliver(raw(), lambda _: value), value)
            else:
                with self.assertRaises(KeyboardInterrupt) as caught:
                    capture.deliver(raw(), lambda _: value)
                self.assertIs(caught.exception, error)
            self.assertEqual(capture.snapshot()['decoder_state'], 'returned_unknown')
            self.assertEqual(capture.snapshot()['observation'], EXPECTED['paid'])

    def test_decoder_exception_class_property_cannot_replace_original(self):
        class Interrupted(BaseException):
            @property
            def __class__(self):
                raise RuntimeError('classification must not replace original')
        error = Interrupted()
        capture = observed.CompletionCapture()
        def decoder(_):
            raise error
        try:
            capture.deliver(raw(), decoder)
            self.fail('original exception missing')
        except BaseException as caught:
            self.assertIs(caught, error)
        self.assertEqual(capture.snapshot()['decoder_state'], 'cancelled')
        self.assertEqual(capture.snapshot()['observation'], EXPECTED['paid'])

    def test_failed_decoder_paid_projection_joins_real_ledger_without_coverage_upgrade(self):
        from harness.memory_benchmark_cost import Ledger
        capture = observed.CompletionCapture()
        error = ValueError(PUBLIC['decoder_error'])
        def decoder(_):
            raise error
        with self.assertRaises(ValueError):
            capture.deliver(raw(), decoder)
        ledger = Ledger(1)
        ticket = ledger.begin_call('reader', 0, 'fresh-public-reader-call', 'fresh-public-attempt')
        ledger.finish_call(ticket, outcome='failed', usage=capture.snapshot()['observation']['usage'])
        ledger.close_unit('reader', 0, coverage='incomplete', model_work='reported_calls')
        product = ledger.finalize()['product']
        self.assertEqual([product['known_input_tokens'], product['known_output_tokens'], product['known_tokens']],
                         [37, 12, 49])
        self.assertIsNone(product['tokens'])
        self.assertIsNone(product['usd_micros'])
        self.assertFalse(product['complete']['coverage'])

    def test_selected_upstream_reader_keeps_raw_usage_before_zero_fill(self):
        ns = {'extract_text_from_response_message': lambda m: m.content.strip(),
              'require': lambda valid, _: valid or (_ for _ in ()).throw(ValueError('empty'))}
        selected_source('longmemeval_reader_calls_2cc8c540.py', ns)
        args = SimpleNamespace(model='public-reader-name', timeout_seconds=2.0, base_url=None,
                               max_completion_tokens=11, reasoning_effort=None, temperature=None,
                               top_p=None, presence_penalty=None, top_k=None, repetition_penalty=None)
        for name in ('paid', 'missing'):
            capture = observed.CompletionCapture()
            submitted = []
            async def create(**kwargs):
                submitted.append(copy.deepcopy(kwargs))
                return capture.deliver(raw(name), lambda data: namespace(json.loads(data)))
            client = SimpleNamespace(chat=SimpleNamespace(completions=SimpleNamespace(create=create)))
            answer, counts = asyncio.run(ns['call_reader_model_async'](client, args, PUBLIC['new_reader_messages']))
            self.assertTrue(answer)
            self.assertEqual(submitted, [{'model': 'public-reader-name', 'messages': PUBLIC['new_reader_messages'],
                                         'timeout': 2.0, 'max_completion_tokens': 11}])
            self.assertEqual(capture.snapshot()['observation'], EXPECTED[name])
            self.assertEqual(list(counts.values()), [37, 12, 49] if name == 'paid' else [0, 0, 0])

    def test_selected_upstream_judge_drops_usage_but_capture_preserves_it(self):
        ns = selected_source('longmemeval_judge_calls_2cc8c540.py', {})
        for mode in ('valid', 'empty_string', 'empty_list'):
            capture = observed.CompletionCapture()
            submitted = []
            def create(**kwargs):
                submitted.append(copy.deepcopy(kwargs))
                def decoder(data):
                    result = namespace(json.loads(data))
                    if mode == 'empty_string':
                        result.choices[0].message.content = ''
                    elif mode == 'empty_list':
                        result.choices[0].message.content = []
                    return result
                return capture.deliver(raw(), decoder)
            client = SimpleNamespace(chat=SimpleNamespace(completions=SimpleNamespace(create=create)))
            call = lambda: ns['_call_chat_completion'](client=client, model='public-judge-name',
                  messages=PUBLIC['new_judge_messages'], max_completion_tokens=7, reasoning_effort=None,
                  temperature=None, top_p=None, timeout_seconds=3.0)
            if mode == 'empty_list':
                with self.assertRaises(ValueError):
                    call()
            else:
                self.assertEqual(call(), '' if mode == 'empty_string' else 'Public cedar answer; never metadata.')
            self.assertEqual(capture.snapshot()['observation'], EXPECTED['paid'])
            self.assertEqual(submitted, [{'model': 'public-judge-name', 'messages': PUBLIC['new_judge_messages'],
                                         'max_completion_tokens': 7, 'timeout': 3.0}])


if __name__ == '__main__':
    unittest.main()
