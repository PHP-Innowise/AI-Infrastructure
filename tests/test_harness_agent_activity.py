"""Agents panel stream: bounded, redacted, path-mapped and never outcome-changing."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import agent_activity
from harness.agent_activity import ActivityStream

ROOTS = {'__system__': '/work/system', 'orders': '/work/system/services/orders', 'payments': '/work/payments'}


def line(event):
    return json.dumps(event).encode()


def tool(call, name, **arguments):
    return line({'type': 'assistant', 'message': {'role': 'assistant', 'content': [
        {'type': 'tool_use', 'id': call, 'name': name, 'input': arguments}]}})


class ActivityStreamTests(unittest.TestCase):
    def stream(self, provider='claude', aliases=None):
        self.events = []
        return ActivityStream(self.events.append, provider, ROOTS, aliases)

    def test_lifecycle_paths_and_usage_are_tagged_per_agent(self):
        stream = self.stream()
        stream.start('service-orders', 2, '/work/system/services/orders', service='orders', mode='edit',
                     readable=['orders', 'payments'], writable=['orders', 'payments'])
        for call, path in (('t1', '/work/system/services/orders/src/a.py'), ('t2', 'docs/b.md'),
                           ('t3', '/work/payments'), ('t4', '/work/system/specs/x.md'), ('t5', '/elsewhere/c.md')):
            stream.line(tool(call, 'Read', file_path=path))
        stream.line(b'not json')
        stream.line(line({'type': 'result', 'subtype': 'success', 'is_error': False, 'total_cost_usd': 0.25,
                          'usage': {'input_tokens': 100, 'output_tokens': 20}}))
        stream.finish('completed', ok=True, error=None, summary='Done for orders')
        self.assertEqual(['agent'] + ['agent_activity'] * 5 + ['usage', 'agent'], [e['kind'] for e in self.events])
        self.assertTrue(all(e['agent'] == 'service-orders' and e['attempt'] == 2 and e['at'] for e in self.events))
        located = [(e.get('service'), e['path'], e.get('outside', False)) for e in self.events if e['kind'] == 'agent_activity']
        # The nested service root wins over the system root that contains it; outside every root only the name stays.
        self.assertEqual([('orders', 'src/a.py', False), ('orders', 'docs/b.md', False), ('payments', '', False),
                          ('__system__', 'specs/x.md', False), (None, 'c.md', True)], located)
        self.assertEqual('Read · payments', self.events[3]['text'])
        self.assertEqual('Read · c.md (outside the project)', self.events[5]['text'])
        self.assertNotIn('/elsewhere', json.dumps(self.events))
        started, finished = self.events[0], self.events[-1]
        self.assertEqual(('running', ['orders', 'payments']), (started['status'], started['writable']))
        self.assertEqual(('completed', 120, 0.25, 'Done for orders'),
                         (finished['status'], finished['tokens'], finished['cost_usd'], finished['summary']))

    def test_codex_changes_and_commands_never_show_an_absolute_path(self):
        stream = self.stream('codex')
        stream.start('service-orders', 1, '/work/system/services/orders')
        for item in ({'id': 'i1', 'type': 'file_change', 'status': 'completed', 'changes': [{'path': '/home/u/.aws/credentials', 'kind': 'update'}]},
                     {'id': 'i2', 'type': 'file_change', 'status': 'completed', 'changes': [
                         {'path': '/work/system/services/orders/a.php', 'kind': 'update'}, {'path': '/home/u/.ssh/config', 'kind': 'add'}]},
                     {'id': 'i3', 'type': 'command_execution', 'command': 'cat /work/system/services/orders/x /work/payments/y',
                      'status': 'completed', 'exit_code': 0},
                     {'id': 'i4', 'type': 'todo_list', 'items': [{'text': 'Fix /work/system/services/orders/a.php', 'completed': False}]}):
            stream.line(line({'type': 'item.completed', 'item': item}))
        activity = [event for event in self.events if event['kind'] == 'agent_activity']
        self.assertEqual(['update credentials (outside the project)', 'update orders:a.php, add config (outside the project)', 'cat x y'],
                         [event['detail'] for event in activity[:3]])
        self.assertEqual(('○ Fix a.php', 'Fix a.php'), (activity[3]['text'], activity[3]['items'][0]['text']))
        self.assertEqual(('credentials', True), (activity[0]['path'], activity[0]['outside']))
        stored = json.dumps(activity)
        for absolute in ('/home/u', '/work/'):
            self.assertNotIn(absolute, stored)

    def test_unreported_usage_stays_unknown_rather_than_zero(self):
        stream = self.stream('codex')
        stream.start('service-orders', 1, '/work/system/services/orders')
        # Codex reports tokens but never a cost; a usage event without token counts leaves tokens unknown.
        stream.line(line({'type': 'turn.completed', 'usage': {'input_tokens': 100, 'output_tokens': 20}}))
        stream.finish('completed', ok=True, error=None)
        self.assertEqual((120, None), (self.events[-1]['tokens'], self.events[-1]['cost_usd']))
        stream.start('service-payments', 1, '/work/payments')
        stream.line(line({'type': 'turn.completed', 'usage': {'input_tokens': 100, 'output_tokens': 20}}))
        stream.line(line({'type': 'turn.completed', 'usage': {'input_tokens': 5}}))
        stream.finish('completed', ok=True, error=None)
        self.assertEqual((None, None), (self.events[-1]['tokens'], self.events[-1]['cost_usd']))

    def test_secrets_are_redacted_from_activity_and_summaries(self):
        stream = self.stream()
        stream.start('contracts', 1, '/work/system')
        secret = 'sk-' + 'a' * 30
        stream.line(tool('t1', 'Bash', command='curl -H "api_key=' + 'B' * 20 + '" https://example.com'))
        # Shorter secrets than the shared pattern catches, and a plan's structured items.
        stream.line(tool('t2', 'Bash', command='DB_PASSWORD=hunter2 mysql -uroot -pshortpw app'))
        stream.line(tool('t3', 'TodoWrite', todos=[{'content': 'Export GITHUB_TOKEN=ghp_' + 'c' * 24, 'status': 'pending'}]))
        stream.line(line({'type': 'assistant', 'message': {'role': 'assistant', 'content': [
            {'type': 'text', 'text': 'Found ' + secret}]}}))
        stream.finish('blocked', ok=False, error='worker_blocked', summary='Leaked ' + secret)
        stored = json.dumps(self.events)
        for value in (secret, 'B' * 20, 'hunter2', 'shortpw', 'c' * 24):
            self.assertNotIn(value, stored)
        self.assertEqual('DB_PASSWORD=[redacted] mysql -uroot -p[redacted] app', self.events[2]['detail'])
        self.assertEqual('Export GITHUB_TOKEN=[redacted]', self.events[3]['items'][0]['text'])
        command, message, finished = self.events[1], self.events[4], self.events[5]
        self.assertTrue(all('[redacted]' in value for value in (command['detail'], command['text'],
                                                               message['text'], finished['summary'])))

    def test_budgets_limit_each_agent_and_the_launch_but_not_lifecycle(self):
        with mock.patch.object(agent_activity, 'AGENT_EVENT_LIMIT', 3), mock.patch.object(agent_activity, 'EVENT_LIMIT', 5):
            stream = self.stream()
            for agent in ('service-orders', 'service-payments'):
                stream.start(agent, 1, '/work/system')
                for number in range(5):
                    stream.line(tool(agent + str(number), 'Read', file_path='spec.md'))
                stream.line(line({'type': 'result', 'subtype': 'success', 'is_error': False,
                                  'usage': {'input_tokens': 1, 'output_tokens': 1}}))
                stream.finish('completed', ok=True, error=None)
        def kinds(agent):
            return [e.get('type') or e['kind'] for e in self.events if e['agent'] == agent]
        # Lifecycle and usage events count against the same launch budget as activity.
        self.assertEqual(['agent', 'tool', 'tool', 'tool', 'status', 'usage', 'agent'], kinds('service-orders'))
        self.assertEqual(['agent', 'status', 'usage', 'agent'], kinds('service-payments'))
        notes = [e['text'] for e in self.events if e.get('type') == 'status']
        self.assertIn('this agent', notes[0])
        self.assertIn('this launch', notes[1])

    def test_hard_limit_silences_everything_and_lifecycle_lists_stay_small(self):
        services = ['service-' + str(number) for number in range(20)]
        with mock.patch.object(agent_activity, 'HARD_EVENT_LIMIT', 4):
            stream = self.stream()
            stream.start('contracts', 1, '/work/system', readable=services, writable=[], root='/work/system')
            for number in range(5):
                stream.line(tool('t' + str(number), 'Read', file_path='spec.md'))
            stream.finish('completed', ok=True, error=None, changed_files=services)
        self.assertEqual(['agent', 'agent_activity', 'agent_activity', 'agent_activity', 'agent_activity'],
                         [e['kind'] for e in self.events])
        self.assertIn('display limit', self.events[-1]['text'])
        self.assertEqual((services[:12], 20), (self.events[0]['readable'], self.events[0]['readable_count']))

    def test_failed_agent_carries_the_cli_error_and_its_fix(self):
        stream = self.stream()
        stream.start('service-orders', 1, '/work/system/services/orders')
        stream.line(line({'type': 'assistant', 'error': 'authentication_failed', 'message': {'role': 'assistant', 'content': []}}))
        stream.line(line({'type': 'result', 'subtype': 'success', 'is_error': True,
                          'result': 'Failed to authenticate: OAuth session expired and could not be refreshed'}))
        stream.finish('blocked', ok=False, error='worker_exit_failure')
        reason = self.events[-1]['reason']
        self.assertIn('Claude Code is not signed in or its login expired (Failed to authenticate', reason)
        self.assertIn('claude auth login', reason)
        self.assertEqual(reason, stream.reasons['service-orders'])
        stream.start('verify', 1, '/work/system')
        stream.finish('completed', ok=True, error=None)
        self.assertNotIn('reason', self.events[-1])

    def test_discovery_aliases_cursor_thinking_and_closed_output(self):
        stream = self.stream('cursor', {Path('/work/agent/evidence/000001.txt'): ('orders', 'src/app.py')})
        stream.start('discovery', 1, '/work/agent')
        for text in ('Check ', 'the consumer'):
            stream.line(line({'type': 'thinking', 'subtype': 'delta', 'text': text}))
        stream.line(line({'type': 'tool_call', 'subtype': 'started', 'call_id': 'c1',
                          'tool_call': {'readToolCall': {'args': {'path': 'evidence/000001.txt'}}}}))
        stream.line(line({'type': 'thinking', 'subtype': 'delta', 'text': 'Unfinished thought'}))
        stream.finish('completed', ok=True, error=None)
        activity = [e for e in self.events if e['kind'] == 'agent_activity']
        self.assertEqual(['thinking', 'tool', 'thinking'], [e['type'] for e in activity])
        self.assertEqual('Check the consumer', activity[0]['text'])
        self.assertEqual(('orders', 'src/app.py'), (activity[1]['service'], activity[1]['path']))
        self.assertEqual('Unfinished thought', activity[2]['text'])

        def closed(event):
            raise OSError('closed pipe')
        broken = ActivityStream(closed, 'claude', ROOTS)
        broken.start('verify', 1, '/work/system')
        broken.line(tool('t1', 'Read', file_path='a.md'))
        broken.finish('completed', ok=True, error=None)


if __name__ == '__main__':
    unittest.main()
