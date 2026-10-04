"""Run view targets and receipts: relative, redacted, bounded, and never outcome-changing."""
from __future__ import annotations

import json
import os
from pathlib import Path, PurePosixPath, PureWindowsPath
import sys
import tempfile
import time
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import providers, run_activity, sessions
from harness.run_activity import Enricher, redact_command, relative, strip_root

POSIX = PurePosixPath('/home/dev/shop')
WINDOWS = PureWindowsPath('C:\\Users\\Dev\\Shop')


def use(call, name, parent=None, **arguments):
    return {'type': 'assistant', 'parent_tool_use_id': parent, 'session_id': 's', 'message': {
        'id': 'msg_' + call, 'role': 'assistant', 'content': [{'type': 'tool_use', 'id': call, 'name': name, 'input': arguments}]}}


def done(call, failed=False):
    return {'type': 'user', 'parent_tool_use_id': None, 'message': {'role': 'user', 'content': [
        {'type': 'tool_result', 'tool_use_id': call, 'content': 'PRIVATE OUTPUT', 'is_error': failed}]}}


def todos(*statuses):
    return [{'content': f'Step {number}', 'status': status, 'activeForm': f'Doing step {number}'}
            for number, status in enumerate(statuses, 1)]


class RelativePathTests(unittest.TestCase):
    def test_posix_paths_are_project_relative_or_a_bare_name(self):
        for path, expected in (('/home/dev/shop/app/Order.php', ('app/Order.php', False)),
                               ('app/Http/Kernel.php', ('app/Http/Kernel.php', False)),
                               ('./app/../routes/api.php', ('routes/api.php', False)),
                               ('/home/dev/shop/', ('', False)),
                               ('/home/dev/shop-api/app/x.php', ('x.php', True)),
                               ('../other/.env', ('.env', True)),
                               ('/home/dev/.ssh/config', ('config', True)),
                               ('/', ('', True))):
            with self.subTest(path=path):
                self.assertEqual(expected, relative(path, POSIX))
        self.assertEqual((None, False), relative(None, POSIX))
        self.assertEqual((None, False), relative('  ', POSIX))

    def test_worktree_alias_and_windows_paths(self):
        worktree = PurePosixPath('/state/worktrees/s1/shop')
        self.assertEqual(('app/x.php', False), relative('/state/worktrees/s1/shop/app/x.php', worktree, POSIX))
        self.assertEqual(('app/y.php', False), relative('/home/dev/shop/app/y.php', worktree, POSIX))
        self.assertEqual(('z.php', True), relative('/home/dev/elsewhere/z.php', worktree, POSIX))
        for path, expected in (('c:\\users\\dev\\shop\\App\\Order.php', ('App/Order.php', False)),
                               ('C:/Users/Dev/Shop/routes/api.php', ('routes/api.php', False)),
                               ('app\\Http\\Kernel.php', ('app/Http/Kernel.php', False)),
                               ('C:\\Users\\Dev\\Shop', ('', False)),
                               ('C:\\Users\\Dev\\Shop2\\x.php', ('x.php', True)),
                               ('D:\\secrets\\key.pem', ('key.pem', True)),
                               ('\\\\server\\share\\notes.txt', ('notes.txt', True))):
            with self.subTest(path=path):
                result = relative(path, WINDOWS)
                self.assertEqual(expected, result)
                self.assertNotRegex(result[0], r'^[A-Za-z]:|^[\\/]')

    def test_strip_root_removes_only_the_project_prefix(self):
        roots = [POSIX]
        self.assertEqual('cat app/x.php && cd . && ls "app"', strip_root('cat /home/dev/shop/app/x.php && cd /home/dev/shop && ls "/home/dev/shop/app"', roots))
        for untouched in ('cat /home/dev/shop-api/x.php', 'cat /x/home/dev/shop/a.php', 'ls /etc'):
            self.assertEqual(untouched, strip_root(untouched, roots))
        self.assertEqual('type app\\x.php & php artisan test', strip_root('type C:\\Users\\Dev\\Shop\\app\\x.php & php c:/users/dev/shop/artisan test', [WINDOWS]))
        self.assertEqual('ls /', strip_root('ls /', [PurePosixPath('/')]))
        self.assertIsNone(strip_root(None, roots))
        # After an attached flag and in file:// the root still goes; a trailing slash reads ./, never nothing.
        for command, expected in (('gcc -I/home/dev/shop/inc x.c', 'gcc -I./inc x.c'), ('php -c/home/dev/shop/php.ini x', 'php -c./php.ini x'),
                                  ('open file:///home/dev/shop/x', 'open file://./x'), ('cp a /home/dev/shop/', 'cp a ./'),
                                  ('cd /home/dev/shop/ && vendor/bin/phpunit', 'cd ./ && vendor/bin/phpunit'), ('cat "/home/dev/shop/"', 'cat "./"')):
            with self.subTest(command=command):
                self.assertEqual(expected, strip_root(command, roots))
        self.assertEqual('cd ./', strip_root('cd C:\\Users\\Dev\\Shop\\', [WINDOWS]))
        # A path under home that no root covers keeps ~ instead of the account name.
        self.assertEqual('cat app/x.php ~/.aws/credentials ~/shop-api/x', strip_root(
            'cat /home/dev/shop/app/x.php /home/dev/.aws/credentials /home/dev/shop-api/x', roots, PurePosixPath('/home/dev')))


class RedactCommandTests(unittest.TestCase):
    def test_short_and_prefixed_secrets_are_redacted(self):
        jwt = 'eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.c2lnbmF0dXJl'
        for command, secret in (('mysql -u root -psecretpass shop', 'secretpass'),
                                ('mysqldump --password=short shop', 'short'),
                                ('mysql --password short shop', 'short'),
                                ('curl -H "Authorization: Bearer ' + jwt + '" https://api.test', jwt),
                                ('curl -H "Authorization: token abc123" https://api.test', 'abc123'),
                                ('echo ' + jwt, 'eyJhbGci'),
                                ('DB_PASSWORD=hunter2 php artisan migrate', 'hunter2'),
                                ('export GITHUB_TOKEN=ghp_' + 'a' * 36, 'ghp_'),
                                ('STRIPE_SECRET=sk_live_51Habc php artisan stripe:sync', 'sk_live_'),
                                ('stripe --api-key sk_test_' + 'x' * 24, 'sk_test_'),
                                ('AWS_SECRET_ACCESS_KEY="a b c" aws s3 ls', 'a b c'),
                                ('PGPASSWORD=pw psql -h db', '=pw'),
                                ('git clone https://deploy:pa55word@github.com/acme/shop.git', 'pa55word'),
                                ('curl -u admin:hunter2 https://api.test', 'hunter2'),
                                ('SLACK=xoxb-1234567890-abcdefghij ./notify', 'xoxb-'),
                                ('php artisan app:import --token=abc', 'abc'),
                                ('echo sk-' + 'x' * 30, 'sk-xxx'),
                                ('client_secret=s3cr3t ./run', 's3cr3t'),
                                ('sshpass -p secretpw ssh deploy@host', 'secretpw'),
                                ('sshpass -v -p secretpw ssh -p 2222 deploy@host', 'secretpw'),
                                ('redis-cli -h cache -a secretpw ping', 'secretpw'),
                                ('docker login -u me -p hunter2 registry.test', 'hunter2'),
                                ('curl -H "X-Api-Key: abc123" https://api.test', 'abc123'),
                                ("curl --header 'Private-Token: abc123' https://git.test", 'abc123')):
            with self.subTest(command=command):
                redacted = redact_command(command)
                self.assertNotIn(secret, redacted)
                self.assertIn('[redacted]', redacted)
        self.assertEqual('DB_PASSWORD=[redacted] php artisan migrate', redact_command('DB_PASSWORD=hunter2 php artisan migrate'))
        self.assertEqual('mysql -u root -p[redacted] shop', redact_command('mysql -u root -psecretpass shop'))

    def test_ordinary_commands_and_plans_stay_readable(self):
        for command in ('php artisan test --filter=PasswordResetTest', 'vendor/bin/phpunit --filter=Password',
                        'grep -rn "password" app/', 'grep "password=" .env.example', 'mkdir -p storage/logs',
                        'docker run -p 8080:80 shop', 'ssh -p2222 deploy@host', 'git log -p -3',
                        'rg -n "API_KEY" config/', 'SSH_KEY_PATH=~/.ssh/id_ed25519 ./deploy.sh', 'sort --key=2 data.csv',
                        'php artisan key:generate', 'composer require laravel/passport', 'Add basic validation rules',
                        'Bearer tokens are checked in the middleware', 'php artisan test --filter=TokenRefreshTest',
                        'cat config/auth.php', 'pytest -k "test_password_reset"',
                        # mysql -p alone prompts; sshpass -f reads a file; ssh's own -p is a port.
                        'mysql -u root -p shop', 'sshpass -f pw.txt ssh -p 2222 deploy@host', 'redis-cli ping',
                        'Add X-Api-Key: header validation'):
            with self.subTest(command=command):
                self.assertEqual(command, redact_command(command))
        self.assertIsNone(redact_command(None))


class Replay(unittest.TestCase):
    def take(self, enricher, provider, events, count=0, output_bytes=0):
        """What the pump would store for these raw events, in order."""
        stored = []
        for event in events:
            for clean in providers.normalize_event(provider, event, targets=True):
                if clean['kind'] == 'plan':
                    plan = enricher.plan(clean)
                    stored += [plan] if plan else []
                    continue
                display = enricher.take(clean, count, output_bytes) if clean['kind'] == 'tool' else None
                self.assertNotIn('targets', clean)
                notice = None if display or clean['kind'] != 'tool' else enricher.notice()
                stored += ([notice] if notice else []) + [{**clean, **(display or {})}]
        self.assertNotIn('PRIVATE', json.dumps(stored))
        return stored


class EnricherTests(Replay):
    def test_fields_are_relative_redacted_and_bounded(self):
        enricher = Enricher('claude', POSIX, str(POSIX))
        stored = self.take(enricher, 'claude', [
            use('toolu_1', 'Read', file_path='/home/dev/shop/app/Order\u202e.php'),
            use('toolu_2', 'Read', file_path='/home/dev/.config/php.ini'),
            use('toolu_3', 'Bash', command='cd /home/dev/shop && DB_PASSWORD=hunter2 php artisan migrate', description='PRIVATE'),
            use('toolu_4', 'Grep', pattern='rules(', path='/home/dev/shop/app/Http'),
            use('toolu_5', 'WebFetch', url='https://docs.test/validation?token=PRIVATE', prompt='PRIVATE'),
            use('toolu_6', 'Bash', command='echo ' + 'x' * 400),
            done('toolu_1'), done('toolu_3', failed=True)])
        self.assertEqual([('Read', 'app/Order.php', None, None), ('Read', 'php.ini', True, None),
                          ('Bash', None, None, 'cd . && DB_PASSWORD=[redacted] php artisan migrate'),
                          ('Grep', 'app/Http', None, '"rules("'), ('WebFetch', None, None, 'https://docs.test/validation')],
                         [(e['tool'], e.get('path'), e.get('outside'), e.get('detail')) for e in stored[:5]])
        self.assertEqual(300, len(stored[5]['detail']))
        self.assertEqual([{'kind': 'tool', 'text': 'Tool: completed', 'ok': True, 'state': 'completed', 'call': 'toolu_1'},
                          {'kind': 'tool', 'text': 'Tool: completed', 'ok': False, 'state': 'completed', 'call': 'toolu_3'}], stored[6:])
        codex = Enricher('codex', WINDOWS)
        changes = self.take(codex, 'codex', [{'type': 'item.completed', 'item': {
            'id': 'item_1', 'type': 'file_change', 'status': 'completed', 'changes': [
                {'path': 'C:\\Users\\Dev\\Shop\\app\\New.php', 'kind': 'add'}, {'path': 'D:\\tmp\\scratch.txt', 'kind': 'weird'}]}},
            {'type': 'item.completed', 'item': {'id': 'item_2', 'type': 'command_execution', 'status': 'failed', 'exit_code': 2,
                                                'command': 'php c:\\users\\dev\\shop\\artisan test', 'aggregated_output': 'PRIVATE'}}])
        self.assertEqual([{'path': 'app/New.php', 'kind': 'add'}, {'path': 'scratch.txt', 'kind': 'update', 'outside': True}],
                         changes[0]['changes'])
        self.assertEqual(('php artisan test', 2, False), (changes[1]['detail'], changes[1]['exit_code'], changes[1]['ok']))

    def test_event_and_byte_caps_stop_once_with_one_notice(self):
        enricher = Enricher('claude', POSIX)
        events = [use(f'toolu_{n}', 'Read', file_path=f'/home/dev/shop/app/F{n}.php') for n in range(run_activity.ENRICH_EVENTS + 50)]
        stored = self.take(enricher, 'claude', events)
        tools = [e for e in stored if e['kind'] == 'tool']
        self.assertEqual(run_activity.ENRICH_EVENTS + 50, len(tools))
        self.assertEqual(run_activity.ENRICH_EVENTS, sum('path' in e for e in tools))
        self.assertEqual([run_activity.LIMITED], [e for e in stored if e.get('targets') == 'limited'])
        self.assertTrue(all('path' not in e for e in tools[run_activity.ENRICH_EVENTS:]))
        self.assertEqual((run_activity.ENRICH_EVENTS + 50, True), (enricher.ledger.receipt()['steps'], enricher.ledger.receipt()['limited']))
        # Long non-ASCII paths reach the byte cap first.
        enricher = Enricher('claude', POSIX)
        long = '/home/dev/shop/' + 'каталог/' * 100
        stored = self.take(enricher, 'claude', [use(f'toolu_{n}', 'Read', file_path=f'{long}{n}.php') for n in range(1000)])
        self.assertLessEqual(enricher.bytes, run_activity.ENRICH_BYTES)
        self.assertLess(sum('path' in e for e in stored), 1000)
        self.assertEqual(1, sum(e.get('targets') == 'limited' for e in stored))
        # Near the launch's own limit nothing more is recorded.
        for count, output_bytes in ((run_activity.STOP_COUNT + 1, 0), (0, run_activity.STOP_BYTES + 1)):
            enricher = Enricher('claude', POSIX)
            self.assertEqual([run_activity.LIMITED, {'kind': 'tool', 'text': 'Read: started'}],
                             self.take(enricher, 'claude', [use('toolu_1', 'Read', file_path='a.php')], count, output_bytes))

    def test_malformed_targets_never_raise(self):
        enricher = Enricher('claude', POSIX)
        for targets, fields in (({'path': 5, 'detail': ['x'], 'changes': 'x', 'exit_code': True, 'call': 'bad id'},
                                 {'state': 'started', 'ok': True}),
                                ({'changes': [None, {'path': 3}, {'path': 'a.php', 'kind': 'add'}], 'state': object(), 'ok': 0},
                                 {'state': 'started', 'ok': True, 'changes': [{'path': 'a.php', 'kind': 'add'}]}),
                                ('not a dict', None)):
            clean = {'kind': 'tool', 'text': 'X: started', 'targets': targets}
            self.assertEqual(fields, enricher.take(clean, 0, 0))
            self.assertNotIn('targets', clean)
        with mock.patch.object(run_activity, 'relative', side_effect=RuntimeError('boom')):
            clean = {'kind': 'tool', 'text': 'Read: started', 'targets': {'state': 'started', 'path': 'a.php'}}
            self.assertIsNone(enricher.take(clean, 0, 0))
            self.assertEqual({'kind': 'tool', 'text': 'Read: started'}, clean)
        self.assertIsNone(enricher.plan({'kind': 'plan', 'items': 'broken'}))
        self.assertEqual([], enricher.compaction([None]))

    def test_plans_and_compactions_share_the_extra_cap(self):
        enricher = Enricher('claude', POSIX)
        plan = {'kind': 'plan', 'items': [{'text': 'Rotate GITHUB_TOKEN=ghp_' + 'a' * 30, 'status': 'active'}] +
                [{'text': f'Item {n}', 'status': 'pending'} for n in range(40)], 'total': 41, 'active_form': 'Rotating'}
        first = enricher.plan(plan)
        self.assertEqual((30, 41, 'Rotate GITHUB_TOKEN=[redacted]', 'Rotating'),
                         (len(first['items']), first['total'], first['items'][0]['text'], first['active_form']))
        self.assertIsNone(enricher.plan(dict(plan)))
        stored = [first]
        for version in range(300):
            items = [{'text': f'Version {version} ' + 'я' * 190, 'status': 'done'}] * 30
            stored += [event for event in [enricher.plan({'kind': 'plan', 'items': items, 'total': 30})] if event]
        stored += enricher.compaction([{'pre': 150000 + n, 'post': None if n % 2 else 30000} for n in range(40)])
        self.assertIsNone(enricher.notice())
        enricher.stopped = True
        stored.append(enricher.notice())
        self.assertIsNone(enricher.notice())
        plans = [e for e in stored if e['kind'] == 'plan']
        compactions = [e for e in stored if 'compaction' in e]
        self.assertLessEqual(len(plans), run_activity.PLAN_EVENTS)
        self.assertLessEqual(len(compactions), run_activity.COMPACTION_EVENTS)
        self.assertLessEqual(len(stored), run_activity.EXTRA_EVENTS)
        self.assertLessEqual(sum(len(json.dumps({**e, 'launch_id': 'f' * 32, 'at': '2026-10-04T21:00:00.123+00:00'},
                                                ensure_ascii=False).encode()) for e in stored), run_activity.EXTRA_BYTES)
        self.assertEqual(run_activity.LIMITED, stored[-1])
        enricher = Enricher('claude', POSIX)
        self.assertEqual([{'kind': 'status', 'compaction': {'pre': 166040, 'post': 38900},
                           'text': 'Context compacted · 166,040 → 38,900 tokens. The CLI replaced earlier conversation with a summary.'},
                          {'kind': 'status', 'compaction': {'pre': None, 'post': None},
                           'text': 'Context compacted · — → — tokens. The CLI replaced earlier conversation with a summary.'}],
                         enricher.compaction([{'pre': 166040, 'post': 38900, 'call': 4}, {'pre': None, 'post': None, 'call': 9}]))
        self.assertEqual([], enricher.compaction([{'pre': 1, 'post': 1}, {'pre': 2, 'post': 2}]))


class LedgerTests(Replay):
    def test_claude_receipt_counts_files_searches_commands_and_helpers(self):
        enricher = Enricher('claude', POSIX)
        self.take(enricher, 'claude', [
            use('toolu_1', 'Read', file_path='/home/dev/shop/app/Order.php'), done('toolu_1'),
            use('toolu_2', 'Read', file_path='app/Order.php'), done('toolu_2'),
            use('toolu_3', 'Read', file_path='/etc/php/php.ini'), done('toolu_3'),
            use('toolu_4', 'Write', file_path='/home/dev/shop/app/Requests/StoreOrder.php', content='PRIVATE'), done('toolu_4'),
            use('toolu_5', 'Edit', file_path='/home/dev/shop/app/Order.php', old_string='PRIVATE', new_string='PRIVATE'), done('toolu_5'),
            use('toolu_6', 'Edit', file_path='/home/dev/shop/app/Broken.php', old_string='PRIVATE', new_string='PRIVATE'), done('toolu_6', True),
            use('toolu_7', 'Grep', pattern='rules(', path='app'), done('toolu_7'),
            use('toolu_8', 'Grep', pattern='rules(', path='tests'), done('toolu_8'),
            use('toolu_9', 'Bash', command='php artisan test --filter=OrderTest'), done('toolu_9', True),
            use('toolu_10', 'Bash', command='php artisan test --filter=OrderTest'), done('toolu_10'),
            use('toolu_11', 'Task', description='Find tests', prompt='PRIVATE'),
            use('toolu_12', 'Glob', parent='toolu_11', pattern='tests/**/*Test.php'), done('toolu_12'), done('toolu_11'),
            use('toolu_13', 'TodoWrite', todos=todos('completed', 'in_progress', 'pending')), done('toolu_13'),
            use('toolu_14', 'Bash', command='sleep 100')])
        receipt = enricher.ledger.receipt()
        self.assertEqual({'steps': 14, 'opened': 2, 'opened_paths': ['app/Order.php'], 'edited': 2,
                          'edited_paths': ['app/Requests/StoreOrder.php', 'app/Order.php'], 'created': 1, 'deleted': 0,
                          'searched': 3, 'patterns': ['"rules("', '"tests/**/*Test.php"'], 'commands': 3, 'failed': 1, 'not_run': 0,
                          'failed_steps': 2, 'last_commands': [
                              {'command': 'php artisan test --filter=OrderTest', 'ok': False, 'outcome': None, 'exit_code': None},
                              {'command': 'php artisan test --filter=OrderTest', 'ok': True, 'outcome': None, 'exit_code': None},
                              {'command': 'sleep 100', 'ok': None, 'outcome': None, 'exit_code': None}],
                          'plan': {'done': 1, 'total': 3}, 'helpers': 1, 'limited': False}, receipt)

    def test_codex_receipt_counts_changes_exit_codes_and_declines(self):
        enricher = Enricher('codex', POSIX)
        item = lambda phase, **fields: {'type': 'item.' + phase, 'item': fields}
        self.take(enricher, 'codex', [
            item('started', id='item_1', type='command_execution', command="/bin/bash -lc 'vendor/bin/phpstan analyse'", status='in_progress'),
            item('completed', id='item_1', type='command_execution', command="/bin/bash -lc 'vendor/bin/phpstan analyse'",
                 status='failed', exit_code=1, aggregated_output='PRIVATE'),
            item('completed', id='item_2', type='command_execution', command='rm -rf vendor', status='declined'),
            item('completed', id='item_3', type='file_change', status='completed', changes=[
                {'path': '/home/dev/shop/app/New.php', 'kind': 'add'}, {'path': '/home/dev/shop/app/Old.php', 'kind': 'delete'},
                {'path': '/home/dev/shop/app/Order.php', 'kind': 'update'}]),
            item('completed', id='item_4', type='file_change', status='failed', changes=[{'path': 'app/Nope.php', 'kind': 'update'}]),
            item('started', id='item_5', type='collab_tool_call', tool='spawn_agent', status='in_progress', prompt='PRIVATE'),
            item('completed', id='item_5', type='collab_tool_call', tool='spawn_agent', status='completed', prompt='PRIVATE'),
            item('completed', id='item_6', type='sub_agent_activity', kind='started', agent_thread_id='child-1'),
            item('updated', id='item_7', type='todo_list', items=[{'text': 'A', 'completed': True}, {'text': 'B', 'completed': False}])])
        receipt = enricher.ledger.receipt()
        self.assertEqual((6, 3, ['app/New.php', 'app/Old.php', 'app/Order.php'], 1, 1, 2, 1, 1, 2, {'done': 1, 'total': 2}),
                         (receipt['steps'], receipt['edited'], receipt['edited_paths'], receipt['created'], receipt['deleted'],
                          receipt['commands'], receipt['failed'], receipt['not_run'], receipt['helpers'], receipt['plan']))
        self.assertEqual([{'command': 'vendor/bin/phpstan analyse', 'ok': False, 'outcome': None, 'exit_code': 1},
                          {'command': 'rm -rf vendor', 'ok': False, 'outcome': 'not_run', 'exit_code': None}], receipt['last_commands'])

    def test_cursor_calls_pair_whatever_their_id(self):
        # Cursor's OpenAI-model call IDs (call_1\nfc_2) are not safe to show: each becomes a stable token, so the start
        # and completion still pair and an edit or a failed command is counted, in the receipt and in the run view.
        enricher = Enricher('cursor', POSIX)
        call = lambda ident, phase, name, **body: {'type': 'tool_call', 'subtype': phase, 'call_id': ident, 'tool_call': {name: body}}
        stored = self.take(enricher, 'cursor', [
            call('call_1\nfc_2', 'started', 'editToolCall', args={'path': '/home/dev/shop/src/a.ts', 'streamContent': 'PRIVATE'}),
            call('call_1\nfc_2', 'completed', 'editToolCall', args={'path': '/home/dev/shop/src/a.ts'}, result={'success': {}}),
            call('call_2\nfc_3', 'started', 'shellToolCall', args={'command': 'false'}),
            call('call_2\nfc_3', 'completed', 'shellToolCall', args={'command': 'false'}, result={'error': {'message': 'PRIVATE'}})])
        calls = [event.get('call') for event in stored]
        self.assertEqual((calls[0], calls[2]), (calls[1], calls[3]))
        self.assertNotEqual(calls[0], calls[2])
        self.assertTrue(all(run_activity._CALL.fullmatch(value) and '\n' not in value for value in calls))
        receipt = enricher.ledger.receipt()
        self.assertEqual((2, 1, ['src/a.ts'], 1, 1, [{'command': 'false', 'ok': False, 'outcome': None, 'exit_code': None}]),
                         (receipt['steps'], receipt['edited'], receipt['edited_paths'], receipt['commands'], receipt['failed'],
                          receipt['last_commands']))
        # Without any call ID a completion closes the oldest open start of the same tool, and is not another step.
        enricher = Enricher('cursor', POSIX)
        self.take(enricher, 'cursor', [{**event, 'call_id': None} for event in (
            call(None, 'started', 'editToolCall', args={'path': 'src/a.ts'}), call(None, 'started', 'shellToolCall', args={'command': 'false'}),
            call(None, 'completed', 'shellToolCall', args={'command': 'false'}, result={'error': {}}),
            call(None, 'completed', 'editToolCall', args={'path': 'src/a.ts'}, result={'success': {}}))])
        receipt = enricher.ledger.receipt()
        self.assertEqual((2, 1, 1, 1), (receipt['steps'], receipt['edited'], receipt['commands'], receipt['failed']))

    def test_files_outside_the_project_count_by_their_full_path(self):
        enricher = Enricher('claude', POSIX)
        stored = self.take(enricher, 'claude', [use('toolu_1', 'Read', file_path='/etc/app/config.php'), done('toolu_1'),
                                                use('toolu_2', 'Read', file_path='/opt/other/config.php'), done('toolu_2'),
                                                use('toolu_3', 'Read', file_path='/opt/other/../other/config.php'), done('toolu_3')])
        self.assertEqual([('config.php', True)] * 3, [(e['path'], e['outside']) for e in stored if e.get('path')])
        self.assertEqual((2, []), (enricher.ledger.receipt()['opened'], enricher.ledger.receipt()['opened_paths']))
        codex = Enricher('codex', POSIX)
        self.take(codex, 'codex', [{'type': 'item.completed', 'item': {'id': 'item_1', 'type': 'file_change', 'status': 'completed', 'changes': [
            {'path': '/tmp/a/notes.md', 'kind': 'add'}, {'path': '/tmp/b/notes.md', 'kind': 'add'}, {'path': 'app/x.php', 'kind': 'update'}]}}])
        receipt = codex.ledger.receipt()
        self.assertEqual((3, 2, ['app/x.php']), (receipt['edited'], receipt['created'], receipt['edited_paths']))
        self.assertNotIn('/tmp', json.dumps(receipt))

    def test_a_write_creates_only_a_file_that_was_not_there(self):
        # Each turn is a new launch of the same native session: a Write may replace a file read in an earlier turn.
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'app').mkdir()
            (root / 'app/Order.php').write_text('<?php\n')
            enricher = Enricher('claude', root)
            self.take(enricher, 'claude', [
                use('toolu_1', 'Write', file_path=str(root / 'app/Order.php'), content='PRIVATE'), done('toolu_1'),
                use('toolu_2', 'Write', file_path='app/New.php', content='PRIVATE'), done('toolu_2')])
            receipt = enricher.ledger.receipt()
            self.assertEqual((2, 1, ['app/Order.php', 'app/New.php']), (receipt['edited'], receipt['created'], receipt['edited_paths']))

    def test_commands_lose_heredoc_bodies_and_later_lines(self):
        seeder = ("cat > database/seeders/CustomerSeeder.php <<'EOF'\n<?php\nCustomer::create(['email' => 'jane.roe@corp.example', "
                  "'iban' => 'DE89370400440532013000']);\nEOF")
        claude = Enricher('claude', POSIX)
        stored = self.take(claude, 'claude', [
            use('toolu_1', 'Bash', command=seeder), done('toolu_1'),
            use('toolu_2', 'Bash', command='git commit -m "$(cat <<\'EOF\'\nFix orders for jane.roe@corp.example\nEOF\n)"'),
            use('toolu_3', 'Bash', command="python3 - <<EOF\nprint('jane.roe@corp.example')\nEOF"),
            use('toolu_4', 'Bash', command='php -r \'$x = 1;\necho "jane.roe@corp.example";\''),
            use('toolu_5', 'Bash', command='cd /home/dev/shop && \\\n  php artisan test'),
            use('toolu_6', 'Bash', command='echo $((1<<2))')])
        self.assertEqual(["cat > database/seeders/CustomerSeeder.php <<'EOF' …", 'git commit -m "$(cat <<\'EOF\' …',
                          'python3 - <<EOF …', "php -r '$x = 1; …", 'cd . && php artisan test', 'echo $((1<<2))'],
                         [event['detail'] for event in stored if event.get('state') == 'started'])
        codex = Enricher('codex', POSIX)
        patch = "apply_patch <<'EOF'\n*** Begin Patch\n*** Update File: app/x.php\n@@\n-  return $old;\n+  return $secretValue;\n*** End Patch\nEOF"
        stored = self.take(codex, 'codex', [{'type': 'item.completed', 'item': {
            'id': 'item_1', 'type': 'command_execution', 'command': ['bash', '-lc', patch], 'status': 'completed', 'exit_code': 0}}])
        self.assertEqual("apply_patch <<'EOF' …", stored[0]['detail'])
        receipts = json.dumps([claude.ledger.receipt(), codex.ledger.receipt()])
        for private in ('jane.roe', 'DE89', 'Customer::create', 'secretValue', 'Begin Patch'):
            self.assertNotIn(private, receipts + json.dumps(stored))

    def test_plan_text_loses_the_root_and_merge_updates_change_items_by_id(self):
        claude = Enricher('claude', POSIX)
        plan = claude.plan({'kind': 'plan', 'items': [{'text': 'Edit /home/dev/shop/app/Order.php', 'status': 'active'}], 'total': 1,
                            'active_form': 'Editing /home/dev/shop/app/Order.php'})
        self.assertEqual(('Edit app/Order.php', 'Editing app/Order.php'), (plan['items'][0]['text'], plan['active_form']))
        enricher = Enricher('cursor', POSIX)
        todo = lambda merge, *todos: {'type': 'tool_call', 'subtype': 'started', 'call_id': f'c{len(todos)}{merge}', 'tool_call': {
            'updateTodosToolCall': {'args': {'merge': merge, 'todos': list(todos)}}}}
        stored = self.take(enricher, 'cursor', [
            todo(False, {'id': '1', 'content': 'Read', 'status': 'TODO_STATUS_COMPLETED'}, {'id': '2', 'content': 'Fix', 'status': 'TODO_STATUS_IN_PROGRESS'},
                 {'id': '3', 'content': 'Test', 'status': 'TODO_STATUS_PENDING'}),
            todo(True, {'id': '2', 'content': 'Fix the rules', 'status': 'TODO_STATUS_COMPLETED'}, {'id': '3', 'content': 'Test', 'status': 'TODO_STATUS_IN_PROGRESS'}),
            todo(True, {'id': '3', 'status': 'TODO_STATUS_COMPLETED'}),
            todo(True, {'id': '4', 'content': 'Document', 'status': 'TODO_STATUS_PENDING'})])
        plans = [(event['items'], event['total']) for event in stored if event['kind'] == 'plan']
        self.assertEqual([([{'text': 'Read', 'status': 'done'}, {'text': 'Fix', 'status': 'active'}, {'text': 'Test', 'status': 'pending'}], 3),
                          ([{'text': 'Read', 'status': 'done'}, {'text': 'Fix the rules', 'status': 'done'}, {'text': 'Test', 'status': 'active'}], 3),
                          ([{'text': 'Read', 'status': 'done'}, {'text': 'Fix the rules', 'status': 'done'}, {'text': 'Test', 'status': 'done'}], 3),
                          ([{'text': 'Read', 'status': 'done'}, {'text': 'Fix the rules', 'status': 'done'}, {'text': 'Test', 'status': 'done'},
                            {'text': 'Document', 'status': 'pending'}], 4)], plans)
        self.assertEqual({'done': 3, 'total': 4}, enricher.ledger.receipt()['plan'])
        # The tool's own result, when it lists the whole plan, outranks the request.
        events = providers.normalize_event('cursor', {'type': 'tool_call', 'subtype': 'completed', 'call_id': 'c9', 'tool_call': {'updateTodosToolCall': {
            'args': {'merge': True, 'todos': [{'id': '2', 'status': 'TODO_STATUS_COMPLETED'}]},
            'result': {'success': {'todos': [{'id': '1', 'content': 'A', 'status': 'TODO_STATUS_COMPLETED'},
                                             {'id': '2', 'content': 'B', 'status': 'TODO_STATUS_COMPLETED'}]}}}}}, targets=True)
        self.assertEqual({'kind': 'plan', 'items': [{'text': 'A', 'status': 'done', 'id': '1'}, {'text': 'B', 'status': 'done', 'id': '2'}],
                          'total': 2}, events[-1])

    def test_worktree_commands_lose_the_source_checkout_but_its_files_stay_outside(self):
        worktree = PurePosixPath('/state/worktrees/s1/shop')
        enricher = Enricher('claude', worktree, str(worktree), strip=(POSIX,), home=PurePosixPath('/home/dev'))
        stored = self.take(enricher, 'claude', [
            use('toolu_1', 'Bash', command='cd /home/dev/shop && git log -1'), use('toolu_2', 'Bash', command='cat /home/dev/shop/.env.example'),
            use('toolu_3', 'Read', file_path='/home/dev/shop/app/x.php'), use('toolu_4', 'Bash', command='cat /home/dev/.ssh/config'),
            use('toolu_5', 'Bash', command='ls /state/worktrees/s1/shop/app')])
        self.assertEqual(['cd . && git log -1', 'cat .env.example', None, 'cat ~/.ssh/config', 'ls app'], [event.get('detail') for event in stored])
        self.assertEqual(('x.php', True), (stored[2]['path'], stored[2]['outside']))
        self.assertNotIn('/home/dev', json.dumps(stored + [enricher.ledger.receipt()]))


FAKE_CLI = r'''
import json, pathlib, sys, time
config = json.loads(sys.argv[1])
sys.stdin.read()
for line in pathlib.Path(config["events"]).read_text(encoding="utf-8").splitlines():
    if line == "SLEEP":
        time.sleep(30)
    print(line, flush=True)
'''


@unittest.skipUnless(hasattr(os, 'killpg'), 'Session workers require POSIX process groups')
class LaunchTests(unittest.TestCase):
    """The real session pump with a scripted Claude stream."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'project'
        self.project.mkdir()
        self.fake = self.root / 'fake_provider.py'
        self.fake.write_text(FAKE_CLI)
        self.script = self.root / 'events.jsonl'
        for target, value in ((sessions.providers, 'discover_providers'), (sessions.providers, 'model_options'),
                              (sessions.providers, 'build_command')):
            patcher = mock.patch.object(target, value, **{
                'discover_providers': {'return_value': [{'id': 'claude', 'available': True, 'executable': str(self.fake)}]},
                'model_options': {'return_value': {'models': [], 'efforts': [], 'detail': 'Offline fixture'}},
                'build_command': {'side_effect': lambda *args, **options: [sys.executable, '-u', str(self.fake),
                                                                            json.dumps({'events': str(self.script)})]}}[value])
            patcher.start()
            self.addCleanup(patcher.stop)
        self.store = sessions.Sessions(self.root / 'state', [self.project], timeout=120)
        self.addCleanup(self.store.close)

    def run_script(self, events, prompt='Validate orders'):
        self.script.write_text('\n'.join(event if isinstance(event, str) else json.dumps(event, ensure_ascii=False)
                                         for event in events), encoding='utf-8')
        sid = self.store.create({'project_id': next(iter(self.store.projects)), 'provider': 'claude',
                                 'prompt': prompt, 'project_context': False})['id']
        return sid, self.settled(sid)

    def settled(self, sid, timeout=60):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.store.get(sid)['status'] not in sessions.ACTIVE and not self.store.jobs.unfinished_tasks:
                return self.store.get(sid)
            time.sleep(.02)
        self.fail('The launch did not settle')

    def stored(self, sid):
        with self.store.lock:
            return [json.loads(row[0]) for row in self.store.db.execute('SELECT data FROM events WHERE session_id=? ORDER BY id', (sid,))]

    def receipt(self, sid):
        return self.store.results.history(sid)['launches'][-1]['receipt']

    def test_targets_never_move_the_launch_failure_point(self):
        long = str(self.project) + '/' + 'каталог/' * 60
        flood = [{'type': 'system', 'subtype': 'init', 'session_id': 'native-flood'}] + [
            use(f'toolu_{n}', 'Read', file_path=f'{long}файл_{n}.php') for n in range(6000)]
        sid, session = self.run_script(flood)
        with mock.patch.object(sessions.run_activity, 'Enricher', lambda *args, **options: None):
            plain_sid, plain = self.run_script(flood)
        enriched, labels = self.stored(sid), self.stored(plain_sid)
        self.assertEqual(('failed', 'failed'), (session['status'], plain['status']))
        self.assertIn('Run failed (SessionError)', enriched[-1]['text'])
        tools = [event for event in enriched if event['kind'] == 'tool']
        self.assertEqual(sum(event['kind'] == 'tool' for event in labels), len(tools))
        self.assertEqual(4999, len(tools))
        self.assertEqual(1, sum(event.get('targets') == 'limited' for event in enriched))
        self.assertTrue(0 < sum('path' in event for event in tools) < len(tools))
        self.assertNotIn(str(self.project), json.dumps(tools, ensure_ascii=False))
        receipt = self.receipt(sid)
        self.assertEqual((5000, 5000, True), (receipt['steps'], receipt['opened'], receipt['limited']))
        self.assertIsNone(self.receipt(plain_sid))

    def test_extras_stay_capped_and_recent_events_still_see_the_whole_launch(self):
        script = [{'type': 'system', 'subtype': 'init', 'session_id': 'native-long'},
                  {'type': 'assistant', 'parent_tool_use_id': None, 'message': {'role': 'assistant', 'content': [
                      {'type': 'text', 'text': 'First answer'}]}}]
        for version in range(150):
            script.append(use(f'toolu_plan_{version}', 'TodoWrite', todos=todos(*['completed'] * (version % 3), 'in_progress', 'pending')
                              + [{'content': f'Revision {version}', 'status': 'pending'}]))
        for number in range(30):
            script.append({'type': 'system', 'subtype': 'compact_boundary',
                           'compact_metadata': {'trigger': 'auto', 'pre_tokens': 150000 + number, 'post_tokens': 30000}})
        # With the session, both texts and the result this counts 4994 of the launch's 5000 events.
        script += [use(f'toolu_{n}', 'Read', file_path=f'{self.project}/app/F{n}.php') for n in range(4840)]
        script += [{'type': 'assistant', 'parent_tool_use_id': None, 'message': {'role': 'assistant', 'content': [
                       {'type': 'text', 'text': 'Last answer'}]}},
                   {'type': 'result', 'subtype': 'success', 'is_error': False, 'session_id': 'native-long', 'result': 'Last answer'}]
        sid, session = self.run_script(script)
        self.assertEqual('completed', session['status'])
        events = self.stored(sid)
        extras = [event for event in events if event['kind'] == 'plan' or 'compaction' in event or event.get('targets') == 'limited']
        self.assertEqual(run_activity.PLAN_EVENTS, sum(event['kind'] == 'plan' for event in extras))
        self.assertEqual(run_activity.COMPACTION_EVENTS, sum('compaction' in event for event in extras))
        self.assertEqual(1, sum(event.get('targets') == 'limited' for event in extras))
        self.assertLessEqual(len(extras), run_activity.EXTRA_EVENTS)
        self.assertLessEqual(sum(len(json.dumps(event, ensure_ascii=False).encode()) for event in extras), run_activity.EXTRA_BYTES)
        self.assertGreater(len(events), 5000)
        self.assertEqual('Validate orders', self.store.recent_events(sid, ('user',))[0]['text'])
        self.assertEqual('Last answer', self.store.recent_events(sid, ('text',), 1)[0]['text'])
        self.assertTrue(all(event['at'].endswith('+00:00') and len(event['at']) == 29 for event in events))
        self.assertEqual({'kind': 'status', 'outcome': 'completed'}, {key: events[-1][key] for key in ('kind', 'outcome')})
        receipt = self.receipt(sid)
        self.assertEqual((4990, 4840, {'done': 2, 'total': 5}, True),
                         (receipt['steps'], receipt['opened'], receipt['plan'], receipt['limited']))

    def test_codex_compaction_dividers_follow_the_live_rollout_not_the_tail(self):
        # The live reader's list only grows; the tail read after the launch is the rollout's last 4 MiB and may begin
        # inside a long launch, so its list can start one compaction later. Dividers must not be matched to it by position.
        c1, c2, c3 = ({'pre': pre, 'post': 1000, 'call': call} for pre, call in ((150001, 3), (150002, 7), (150003, 12)))

        class Live:
            def __init__(self, project, since):
                self.offset, self.reads, self.fill, self.lists = 0, 0, self, [[c1, c2], [c1, c2, c3]]

            def poll(self, native_id):
                if not native_id or self.reads >= len(self.lists):
                    return False
                self.reads, self.offset = self.reads + 1, self.offset + 100
                return True

            def snapshot(self):
                return {'start': 1, 'end': 2, 'peak': 3, 'calls': 12, 'window': 200000, 'compactions': list(self.lists[self.reads - 1])}

        shifted = {'start': 1, 'end': 2, 'peak': 3, 'calls': 9, 'window': 200000,
                   'compactions': [{**c2, 'call': 4}, {**c3, 'call': 9}]}
        with mock.patch.object(sessions.providers, 'discover_providers', return_value=[{'id': 'codex', 'available': True, 'executable': str(self.fake)}]), \
                mock.patch.object(sessions.context_usage, 'CodexLive', Live), \
                mock.patch.object(sessions.context_usage, 'codex_fill', return_value=shifted), \
                mock.patch.object(sessions.providers, 'codex_rollout_tail', return_value='tail'):
            store = sessions.Sessions(self.root / 'codex-state', [self.project], timeout=120)
            self.addCleanup(store.close)
            self.script.write_text('\n'.join(json.dumps(event) for event in (
                {'type': 'thread.started', 'thread_id': 'thread-1'}, {'type': 'turn.started'},
                {'type': 'item.completed', 'item': {'id': 'item_0', 'type': 'agent_message', 'text': 'Done'}},
                {'type': 'turn.completed', 'usage': {'input_tokens': 10, 'cached_input_tokens': 0, 'output_tokens': 2}})))
            sid = store.create({'project_id': next(iter(store.projects)), 'provider': 'codex', 'prompt': 'Compact', 'project_context': False})['id']
            deadline = time.monotonic() + 60
            while (store.get(sid)['status'] in sessions.ACTIVE or store.jobs.unfinished_tasks) and time.monotonic() < deadline:
                time.sleep(.02)
            with store.lock:
                events = [json.loads(row[0]) for row in store.db.execute('SELECT data FROM events WHERE session_id=? ORDER BY id', (sid,))]
        self.assertEqual('completed', store.get(sid)['status'])
        self.assertEqual([150001, 150002, 150003], [event['compaction']['pre'] for event in events if 'compaction' in event])

    def test_a_launch_that_cannot_start_still_gets_its_receipt(self):
        self.script.write_text('')
        with mock.patch.object(sessions.process_runtime, 'launch_guarded', side_effect=OSError('missing executable')):
            sid = self.store.create({'project_id': next(iter(self.store.projects)), 'provider': 'claude',
                                     'prompt': 'Validate orders', 'project_context': False})['id']
            session = self.settled(sid)
        self.assertEqual('failed', session['status'])
        receipt = self.receipt(sid)
        self.assertEqual((0, 0, 0, False), (receipt['steps'], receipt['opened'], receipt['commands'], receipt['limited']))

    def test_receipt_and_launch_survive_cancel(self):
        sid, session = self.run_script([{'type': 'system', 'subtype': 'init', 'session_id': 'native-cancel'},
                                        use('toolu_1', 'Read', file_path='app/a.php'), done('toolu_1'),
                                        {'type': 'result', 'subtype': 'success', 'is_error': False, 'session_id': 'native-cancel',
                                         'result': 'Read it'}])
        self.assertEqual({'kind': 'native', 'status': 'completed'}, {key: session['launch'][key] for key in ('kind', 'status')})
        self.assertEqual({'id', 'kind', 'status', 'started_at', 'finished_at'}, set(session['launch']))
        self.assertEqual(1, self.receipt(sid)['opened'])
        self.script.write_text('\n'.join([json.dumps(use('toolu_2', 'Bash', command='php artisan test')), json.dumps(done('toolu_2')),
                                          json.dumps(use('toolu_3', 'Read', file_path='app/b.php')), 'SLEEP']))
        self.store.send(sid, 'Again')
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and not any(event.get('path') == 'app/b.php' for event in self.stored(sid)):
            time.sleep(.02)
        self.assertEqual('running', self.store.get(sid)['launch']['status'])
        self.store.cancel(sid)
        self.assertEqual('cancelled', self.settled(sid)['status'])
        receipt = self.receipt(sid)
        self.assertEqual((2, 1, 1, ['app/b.php']), (receipt['steps'], receipt['commands'], receipt['opened'], receipt['opened_paths']))
        self.assertEqual('cancelled', self.store.get(sid)['launch']['status'])


if __name__ == '__main__':
    unittest.main()
