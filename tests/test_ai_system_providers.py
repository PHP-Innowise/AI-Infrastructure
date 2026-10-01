"""Native report envelopes, fail-closed terminal handling, and argv limits."""
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from ai_system_providers import invocation, worker_result, discover_executable
from ai_system_lib import SystemError

REPORT = {'status': 'completed', 'summary': 'Fixture report', 'checks': [],
          'changed_files': [], 'service_order': []}


def stream(*events):
    return '\n'.join(json.dumps(e) for e in events).encode()


class SystemProviderTests(unittest.TestCase):
    def test_claude_schema_envelope_and_cursor_terminal_report(self):
        claude = {'type': 'result', 'subtype': 'success', 'is_error': False,
                  'result': 'Prose is not the report', 'structured_output': REPORT}
        self.assertEqual(REPORT, worker_result(stream(claude), 'claude'))
        cursor = {'type': 'result', 'subtype': 'success', 'is_error': False,
                  'result': json.dumps(REPORT)}
        self.assertEqual(REPORT, worker_result(stream({'type': 'assistant', 'message':
            {'content': [{'type': 'text', 'text': 'Interim prose'}]}}, cursor), 'cursor'))

    def test_native_errors_truncation_and_missing_reports_are_rejected(self):
        for provider in ('claude', 'cursor'):
            valid = {'type': 'result', 'subtype': 'success', 'is_error': False,
                     'result': json.dumps(REPORT), 'structured_output': REPORT}
            invalid = [dict(valid, subtype='error_max_turns'), dict(valid, is_error=True),
                       {k: v for k, v in valid.items() if k != 'is_error'},
                       {'type': 'assistant', 'message': {'content': [{'type':'text', 'text':json.dumps(REPORT)}]}}]
            invalid.append(dict(valid, structured_output=None) if provider == 'claude' else dict(valid, result=''))
            for event in invalid:
                with self.subTest(provider=provider, event=event):
                    with self.assertRaises(SystemError):
                        worker_result(stream(event), provider)
            with self.assertRaises(SystemError):
                worker_result(stream(valid, valid), provider)
            with self.assertRaises(SystemError):
                worker_result(stream(valid, {'type': 'error', 'message': 'failure'}), provider)
            with self.assertRaises(SystemError):
                worker_result(stream(valid)[:-2], provider)

    def test_codex_requires_completed_message_and_one_successful_terminal(self):
        message = {'type': 'item.completed', 'item': {'type': 'agent_message', 'text': json.dumps(REPORT)}}
        done = {'type': 'turn.completed'}
        self.assertEqual(REPORT, worker_result(stream(message, done), 'codex'))
        for events in ((message,), (message, dict(done, error='failure')), (message, dict(done, is_error=True)),
                       (message, done, done), (done, message), (message, done, {'type':'turn.failed'}),
                       (dict(message, type='item.updated'), done)):
            with self.subTest(events=events):
                with self.assertRaises(SystemError):
                    worker_result(stream(*events), 'codex')

    def test_cursor_utf8_argument_limit_is_checked_before_launch(self):
        with self.assertRaisesRegex(SystemError, 'argument byte limit'):
            invocation('cursor', '/trusted/cursor', Path.cwd(), 'я' * 60001, {}, Path('schema'), 'read-only')

    def test_discovery_only_selects_the_requested_available_native_cli(self):
        with patch('ai_system_providers.discover_providers', return_value=[
                {'id':'codex', 'available':True, 'executable':'/codex'},
                {'id':'cursor', 'available':False, 'executable':'/wrong-agent'},
                {'id':'claude', 'available':True, 'executable':'/claude'}]):
            self.assertEqual('/claude', discover_executable('claude'))
            self.assertIsNone(discover_executable('cursor'))


if __name__ == '__main__':
    unittest.main()
