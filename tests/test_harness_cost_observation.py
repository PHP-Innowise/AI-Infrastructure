"""Launch-level accounting integration contracts; no provider or memory replay."""
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import cost_observation as cost


class LaunchCostTests(unittest.TestCase):
    def started(self, observation, label, tokens):
        observation.new_phase(label, 'native')
        observation.process(label, 'started')
        observation.raw(label, {'type': 'thread.started', 'thread_id': 'native'})
        observation.raw(label, {'type': 'turn.completed', 'usage': {'input_tokens': tokens, 'output_tokens': 2}})

    def test_execution_selection_and_prompt_subsets_do_not_double_count(self):
        observed = cost.LaunchObservation('codex')
        self.started(observed, 'main', 10)
        observed.new_phase('recovery', 'native'); observed.process('recovery', 'skipped')
        observed.payload('main_prompt', 'é body instructions')
        observed.payload('capsule_body', 'body')
        observed.payload('capsule_envelope', ' body ')
        observed.payload('draft_instruction', ' instructions')
        result = observed.finish('completed')
        self.assertEqual(12, result['provider']['provider_tokens'])
        self.assertEqual(1, len(result['provider']['executions']))
        self.assertEqual(len('é body instructions'.encode()), result['constructed_payloads']['main_prompt']['bytes'])
        self.assertFalse(result['processes']['recovery']['started'])
        observed.payload('main_prompt', 'changed')
        self.assertEqual(result, observed.finish('failed'))

    def test_incomplete_phase_creation_cannot_publish_main_only_complete_total(self):
        observed = cost.LaunchObservation('codex'); self.started(observed, 'main', 10)
        with patch.object(cost, 'Phase', side_effect=RuntimeError('PRIVATE_SENTINEL')):
            cost.safe(observed, 'new_phase', 'recovery', 'native')
        # Phase construction raised before adding the recovery. Existing phase stays countable.
        result = observed.finish('completed')
        self.assertTrue(result['observer_incomplete'])
        self.assertIsNone(result['provider']['provider_tokens'])
        self.assertEqual('incomplete', result['provider']['token_state'])
        self.assertEqual(12, result['provider']['known_token_subtotal'])
        self.assertNotIn('PRIVATE_SENTINEL', json.dumps(result))

    def test_existing_usd_attribution_remains_separate_from_raw_report(self):
        from harness.providers import RunCost
        observed = cost.LaunchObservation('claude')
        observed.new_phase('main', 'native'); observed.process('main', 'started')
        raw = {'type': 'result', 'subtype': 'success', 'is_error': False, 'session_id': 'native',
               'usage': {'input_tokens': 10, 'output_tokens': 2, 'cache_read_input_tokens': 8, 'cache_creation_input_tokens': 0}, 'total_cost_usd': .8}
        attributed = RunCost('claude', 'native', .5)
        attributed.observe({'type': 'system', 'subtype': 'init', 'session_id': 'native', 'claude_code_version': '2.1.277'})
        observed.raw('main', raw)
        observed.usd('main', attributed.own(.8), .8, attributed)
        result = observed.finish('completed')['provider']
        row = result['executions'][0]
        self.assertEqual(20, row['provider_tokens'])
        self.assertEqual(.8, row['terminal_reports'][0]['reported_usd'])
        self.assertEqual(.3, row['own_usd'])
        self.assertEqual('resumed_delta', row['usd_attributions'][0]['attribution'])
        self.assertEqual(.3, result['own_usd'])
        self.assertEqual('complete', result['usd_state'])

    def test_local_operation_filters_metadata_counts_retries_and_missing_io(self):
        observed = cost.LaunchObservation('codex')
        operation = observed.operation('run', 'record-result')
        operation.retry()
        operation.io({'stdout_known_bytes': 3, 'returned_stdout_bytes': None, 'stderr_known_bytes': 0,
                      'returned': False, 'started': True, 'stdout_eof': False, 'stderr_eof': True,
                      'seconds': .01, 'private': 'PRIVATE_BODY'})
        operation.finish({'ok': True, 'result': {'records': [{'state': 'created', 'title': 'PRIVATE_BODY'}]}})
        observed.operation('inspect').finish({'task': {'private': 'PRIVATE_BODY'}})
        records = observed.finish('failed')['local_operations']
        self.assertEqual(1, records[0]['lock_retries'])
        self.assertIsNone(records[0]['runtime_io'][0]['returned_stdout_bytes'])
        self.assertEqual({'created': 1}, records[0]['record_states'])
        self.assertEqual('unknown', records[1]['io_state'])
        self.assertNotIn('PRIVATE_BODY', json.dumps(records))

    def test_unsupported_workflow_and_observation_failure_remain_explicit(self):
        observed = cost.LaunchObservation('codex', supported=False)
        observed.new_phase('main'); observed.process('main', 'started')
        observed.raw('main', {'kind': 'usage', 'input_tokens': 100})
        result = observed.finish('completed')
        self.assertFalse(result['supported_workflow'])
        self.assertIsNone(result['provider']['provider_tokens'])
        self.assertEqual([], result['provider']['executions'])
        self.assertEqual('encoding_unknown', cost.text_size('\ud800')['state'])


    def test_local_failure_reaches_owner_and_io_is_unknown(self):
        observed = cost.LaunchObservation('codex')
        operation = observed.operation('run', 'refresh')
        with patch.object(operation, 'io', side_effect=RuntimeError('PRIVATE_IO_FAILURE')):
            cost.safe(operation, 'io', {})
        operation.finish({'ok': True, 'result': {'records': 123}})
        result = observed.finish('completed')
        self.assertTrue(result['observer_incomplete'])
        self.assertEqual('unknown', result['local_operations'][0]['io_state'])
        self.assertEqual('not_applicable', result['local_operations'][0]['record_states_state'])
        self.assertNotIn('PRIVATE_IO_FAILURE', json.dumps(result))

    def test_record_state_capacity_and_invalid_types_remain_unknown(self):
        class Uniterable:
            def __iter__(self):
                raise AssertionError('The observer must not iterate arbitrary values')
        for records in (123, Uniterable(), [{'state': 'created'}] * 129, [{'state': []}]):
            with self.subTest(records_type=type(records).__name__):
                observed = cost.LaunchObservation('codex')
                observed.operation('run', 'record-result').finish({'ok': True, 'result': {'records': records}})
                result = observed.finish('completed')
                self.assertTrue(result['observer_incomplete'])
                self.assertEqual('unknown', result['local_operations'][0]['record_states_state'])
                self.assertEqual({}, result['local_operations'][0]['record_states'])

if __name__ == '__main__': unittest.main()
