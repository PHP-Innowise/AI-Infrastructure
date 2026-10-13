"""Native report envelopes, fail-closed terminal handling, and argv limits."""
import json
from pathlib import Path
import sys
import tempfile
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

    def test_extra_folders_follow_each_cli_grant_model(self):
        with tempfile.TemporaryDirectory() as root:
            folders = [Path(root) / 'payments', Path(root) / 'notifications']
            command, stdin = invocation('claude', '/trusted/claude', Path(root), 'Prompt', {'type': 'object'},
                                        Path('schema'), 'read-only', folders)
            self.assertEqual('stream-json', command[command.index('--output-format') + 1])
            self.assertIn('--verbose', command)
            self.assertEqual([str(f) for f in folders], command[command.index('--add-dir') + 1:command.index('--json-schema')])
            self.assertEqual('Prompt', stdin)
            # Codex already reads the whole disk; only edit dispatches receive extra writable roots.
            command, _ = invocation('codex', '/trusted/codex', Path(root), 'Prompt', {}, Path('schema'), 'read-only', folders)
            self.assertNotIn('--add-dir', command)
            command, _ = invocation('codex', '/trusted/codex', Path(root), 'Prompt', {}, Path('schema'), 'edit', folders)
            self.assertEqual([str(f) for f in folders], [command[i + 1] for i, part in enumerate(command) if part == '--add-dir'])
            self.assertLess(command.index('--add-dir'), command.index('--output-schema'))
            self.assertEqual('-', command[-1])
            with self.assertRaisesRegex(SystemError, 'additional service folders'):
                invocation('cursor', '/trusted/cursor', Path(root), 'Prompt', {}, Path('schema'), 'edit', folders)
            command, _ = invocation('cursor', '/trusted/cursor', Path(root), 'Prompt', {}, Path('schema'), 'edit')
            self.assertNotIn('--add-dir', command)

    def test_claude_stream_returns_the_terminal_structured_output(self):
        events = [{'type': 'system', 'subtype': 'init', 'session_id': 'stream'},
                  {'type': 'assistant', 'message': {'role': 'assistant', 'content': [
                      {'type': 'text', 'text': 'Working'},
                      {'type': 'tool_use', 'id': 'toolu_1', 'name': 'StructuredOutput', 'input': REPORT}]}},
                  {'type': 'user', 'message': {'role': 'user', 'content': [
                      {'type': 'tool_result', 'tool_use_id': 'toolu_1', 'content': 'ok', 'is_error': False}]}},
                  {'type': 'result', 'subtype': 'success', 'is_error': False, 'result': '', 'structured_output': REPORT}]
        self.assertEqual(REPORT, worker_result(stream(*events), 'claude'))
        with self.assertRaises(SystemError):
            worker_result(stream(*events[:-1]), 'claude')

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
