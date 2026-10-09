"""Memory MCP protocol and persistence exercised in isolated installed projects."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor

from tests.test_harness_knowledge import install_knowledge_fixture, metadata


class MemoryMcpTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'project'; self.root.mkdir()
        install_knowledge_fixture(self.root)
        self.server = self.root / 'memory-bank/scripts/mcp_server.py'
        self.cli = self.root / 'memory-bank/scripts/context.py'

    def cli_call(self, *args):
        result = subprocess.run([sys.executable, str(self.cli), '--root', str(self.root), *args, '--json'],
            capture_output=True, text=True, timeout=10)
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def exchange(self, requests, root=None):
        messages = [{'jsonrpc': '2.0', 'id': 0, 'method': 'initialize',
            'params': {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'test', 'version': '1'}}},
            {'jsonrpc': '2.0', 'method': 'notifications/initialized'}, *requests]
        result = subprocess.run([sys.executable, str(self.server), '--root', str(root or self.root)],
            input=''.join(json.dumps(m) + '\n' for m in messages), capture_output=True, text=True, timeout=20)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual('', result.stderr)
        return [json.loads(line) for line in result.stdout.splitlines()][1:]

    def call(self, name, **arguments):
        return self.exchange([{'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
            'params': {'name': name, 'arguments': arguments}}])[0]['result']

    def start(self):
        self.cli_call('start', '--task-id', 'TASK-MCP', '--goal', 'Check cobalt allocation')
        return self.cli_call('get', '--task-id', 'TASK-MCP')['revision']

    def draft(self, revision):
        return {'task_id': 'TASK-MCP', 'result_id': 'result-1', 'revision': revision,
            'progress': 'Cobalt allocation checked.', 'next_steps': [], 'verified': True,
            'learnings': [{'type': 'finding', 'title': 'Cobalt allocation requires an owner',
                'consequence': 'Every cobalt allocation must name exactly one owner.',
                'sources': ['specs/authority.md']}]}

    def test_protocol_and_warming_retrieval_leave_task_unprovisioned(self):
        reply = self.exchange([{'jsonrpc': '2.0', 'id': 1, 'method': 'tools/list'},
            {'jsonrpc': '2.0', 'id': 2, 'method': 'not-a-method'}])
        self.assertEqual(4, len(reply[0]['result']['tools']))
        self.assertEqual(-32601, reply[1]['error']['code'])
        found = self.call('memory_retrieve', task_id='TASK-MCP', query='cobalt allocation', paths=['specs/authority.md'])
        self.assertFalse(found['isError'], found)
        self.assertIn('capsule', found['structuredContent'])
        self.assertEqual([], list((self.root / 'project-brain/dynamic/tasks').glob('*')))

    def test_native_client_metadata_is_accepted_for_listing_and_tool_calls(self):
        replies = self.exchange([
            {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/list',
             'params': {'_meta': {'progressToken': 7, 'clientTag': 'native-codex'}}},
            {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
             'params': {'name': 'memory_status', 'arguments': {}, '_meta': {'progressToken': 'status-1'}}}])
        self.assertEqual(4, len(replies[0]['result']['tools']))
        self.assertFalse(replies[1]['result']['isError'], replies[1])
        self.assertEqual('governed', replies[1]['result']['structuredContent']['mode'])

    def test_invalid_metadata_and_unsupported_list_parameters_are_refused(self):
        requests = []
        for bad in (None, [], True, 'native-codex'):
            requests.extend([
                {'jsonrpc': '2.0', 'id': len(requests) + 1, 'method': 'tools/list', 'params': {'_meta': bad}},
                {'jsonrpc': '2.0', 'id': len(requests) + 2, 'method': 'tools/call',
                 'params': {'name': 'memory_status', '_meta': bad}}])
        requests.append({'jsonrpc': '2.0', 'id': 20, 'method': 'tools/list', 'params': {'cursor': 'unsupported'}})
        replies = self.exchange(requests)
        self.assertEqual([-32602] * len(requests), [reply['error']['code'] for reply in replies])

    def test_records_promote_and_replay_safely_after_server_restart(self):
        draft = self.draft(self.start())
        first = self.call('memory_record_result', **draft)
        self.assertFalse(first['isError'], first)
        self.assertEqual(1, len(first['structuredContent']['records']))
        self.assertEqual(1, len(list((self.root / 'memory-bank/chunks').glob('*.md'))))
        record = next((self.root / 'project-brain/dynamic/findings').glob('*.md'))
        data = metadata(record)
        self.assertEqual(('verified', 'resolved'), (data['authority'], data['status']))
        self.assertIn('agent-attested', data['transitions'][-1]['reason'])
        second = self.call('memory_record_result', **draft)
        self.assertTrue(second['structuredContent']['replayed'])
        self.assertEqual(1, len(list((self.root / 'project-brain/dynamic/findings').glob('*.md'))))
        changed = self.call('memory_record_result', **{**draft, 'progress': 'Different result'})
        self.assertTrue(changed['isError'])

    def test_concurrent_replay_has_one_record(self):
        draft = self.draft(self.start())
        with ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(lambda _: self.call('memory_record_result', **draft), range(2)))
        self.assertTrue(all(not r['isError'] for r in results), results)
        self.assertEqual(1, len(list((self.root / 'project-brain/dynamic/findings').glob('*.md'))))

    def test_stale_revision_and_unverified_or_foreign_sources_do_not_mutate_task(self):
        revision = self.start(); original = self.cli_call('get', '--task-id', 'TASK-MCP')
        outside = self.root.parent / 'outside.md'; outside.write_text('outside')
        (self.root / 'foreign.md').symlink_to(outside)
        for changes in ({'revision': revision + 1}, {'verified': False},
            {'learnings': [{**self.draft(revision)['learnings'][0], 'sources': ['../outside.md']}]},
            {'learnings': [{**self.draft(revision)['learnings'][0], 'sources': ['foreign.md']}]}):
            reply = self.call('memory_record_result', **{**self.draft(revision), **changes})
            self.assertTrue(reply['isError'], reply)
        (self.root / 'foreign.md').unlink()
        self.assertEqual(original, self.cli_call('get', '--task-id', 'TASK-MCP'))
        self.assertEqual([], list((self.root / 'project-brain/dynamic/findings').glob('*.md')))

    def test_first_turn_checkpoint_can_record_without_edits(self):
        checkpoint = self.call('memory_checkpoint', task_id='TASK-MCP', goal='Check cobalt allocation')
        self.assertFalse(checkpoint['isError'], checkpoint)
        draft = self.draft(checkpoint['structuredContent']['revision'])
        self.assertFalse(self.call('memory_record_result', **draft)['isError'])

    def test_shared_replay_survives_local_receipt_deletion_and_current_revision(self):
        import shutil
        draft = self.draft(self.start())
        self.assertFalse(self.call('memory_record_result', **draft)['isError'])
        shutil.rmtree(self.root / 'project-brain/local/results')
        current = self.cli_call('get', '--task-id', 'TASK-MCP')
        reply = self.call('memory_record_result', **{**draft, 'revision': current['revision']})
        self.assertTrue(reply['structuredContent']['replayed'])
        self.assertEqual(current, self.cli_call('get', '--task-id', 'TASK-MCP'))
        self.assertEqual(1, len(list((self.root / 'project-brain/dynamic/findings').glob('*.md'))))

    def test_sensitive_sources_and_unauthorized_actor_are_refused_before_writes(self):
        revision = self.start()
        (self.root / '.env').write_text('fixture')
        for source in ('.env', 'memory-bank/local/context.db', '.git/config'):
            draft = self.draft(revision); draft['learnings'][0]['sources'] = [source]
            self.assertTrue(self.call('memory_record_result', **draft)['isError'])
        self.assertFalse((self.root / 'project-brain/local/results').exists())
        result = subprocess.run([sys.executable, str(self.cli), '--root', str(self.root), '--owner', 'alice',
            'start', '--task-id', 'TASK-ALICE', '--goal', 'Check cobalt', '--json'], capture_output=True, text=True, timeout=10)
        self.assertEqual(0, result.returncode, result.stderr)
        task = self.cli_call('get', '--task-id', 'TASK-ALICE')
        draft = {**self.draft(task['revision']), 'task_id': 'TASK-ALICE'}
        self.assertTrue(self.call('memory_record_result', **draft)['isError'])
        self.assertTrue(self.call('memory_retrieve', task_id='TASK-ALICE', query='cobalt')['isError'])
        self.assertFalse((self.root / 'project-brain/local/results').exists())

    def test_a_stale_revision_writes_nothing_and_its_replay_finishes(self):
        draft = self.draft(self.start())
        self.cli_call('update', '--task-id', 'TASK-MCP', '--revision', 'auto', '--progress', 'Moved on by a checkpoint.')
        before = self.cli_call('get', '--task-id', 'TASK-MCP')
        self.assertTrue(self.call('memory_record_result', **draft)['isError'])
        self.assertEqual(before, self.cli_call('get', '--task-id', 'TASK-MCP'))
        self.assertEqual([], list((self.root / 'project-brain/dynamic/findings').glob('*.md')))
        reply = self.call('memory_record_result', **{**draft, 'revision': before['revision']})
        self.assertFalse(reply['isError'], reply)
        self.assertEqual(1, len(list((self.root / 'project-brain/dynamic/findings').glob('*.md'))))

    def test_saved_result_can_replay_after_task_completion(self):
        draft = self.draft(self.start())
        self.assertFalse(self.call('memory_record_result', **draft)['isError'])
        task = self.cli_call('get', '--task-id', 'TASK-MCP')
        self.cli_call('complete', '--task-id', 'TASK-MCP', '--revision', str(task['revision']),
            '--outcome', 'Cobalt rule checked', '--verification', 'Isolated fixture passed')
        self.assertTrue(self.call('memory_record_result', **draft)['structuredContent']['replayed'])
        self.assertFalse(self.call('memory_retrieve', task_id='TASK-MCP', query='cobalt')['isError'])

    def test_private_and_restricted_tasks_cannot_deliver_working_state(self):
        for privacy in ('private', 'restricted'):
            identifier = 'TASK-' + privacy.upper()
            code = ('import sys; from pathlib import Path; sys.path.insert(0, sys.argv[1]); '
                'from brain_runtime import create_task; '
                'create_task(Path(sys.argv[2]), sys.argv[3], "Cobalt allocation", [], [], owner="local", privacy=sys.argv[4])')
            created = subprocess.run([sys.executable, '-c', code, str(self.server.parent),
                str(self.root), identifier, privacy], capture_output=True, text=True, timeout=10)
            self.assertEqual(0, created.returncode, created.stderr)
            self.assertTrue(self.call('memory_retrieve', task_id=identifier, query='cobalt')['isError'])

    def test_a_save_that_stopped_half_way_is_finished_by_its_replay(self):
        # The first attempt wrote the learning's record and stopped before
        # closing it or updating the task: the replay closes that record and
        # writes no second one.
        draft = self.draft(self.start())
        sys.path.insert(0, str(self.root / 'memory-bank/scripts'))
        try:
            import memory_results
            import brain_runtime
            uuid = brain_runtime.find_record(self.root, 'TASK-MCP', record_type='task')[1]['id']
            identifier = memory_results.learning_id(uuid, draft['learnings'][0])
        finally:
            sys.path.remove(str(self.root / 'memory-bank/scripts'))
        learning = draft['learnings'][0]
        self.cli_call('brain-create', 'finding', '--external-id', identifier, '--title', learning['title'],
            '--goal', learning['consequence'], '--source', learning['sources'][0])
        reply = self.call('memory_record_result', **draft)
        self.assertFalse(reply['isError'], reply)
        self.assertEqual(['completed'], [item['state'] for item in reply['structuredContent']['records']])
        findings = list((self.root / 'project-brain/dynamic/findings').glob('*.md'))
        self.assertEqual(1, len(findings))
        self.assertIn('resolved', findings[0].read_text())
        self.assertEqual('Cobalt allocation checked.', self.cli_call('get', '--task-id', 'TASK-MCP')['progress'])

    def test_protocol_rejects_bad_params_and_arbitrary_root(self):
        self.assertTrue(self.call('memory_status', root=str(self.root.parent))['isError'])
        reply = self.exchange([{'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': []}])
        self.assertEqual(-32602, reply[0]['error']['code'])
        result = subprocess.run([sys.executable, str(self.server), '--root', str(self.root.parent)],
            input='', capture_output=True, text=True, timeout=10)
        self.assertEqual(1, result.returncode); self.assertEqual('', result.stdout)

    def test_malformed_json_and_initialization_order(self):
        result = subprocess.run([sys.executable, str(self.server), '--root', str(self.root)],
            input='{oops}\n' + json.dumps({'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'}) + '\n',
            capture_output=True, text=True, timeout=10)
        replies = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual([-32700, -32602], [r['error']['code'] for r in replies])

    def test_a_bearer_token_in_a_result_is_refused_and_never_stored(self):
        token = 'Zx81Kq0vLm2Np3Qr4St5Uv6Wx7'
        draft = self.draft(self.start())
        draft['learnings'][0]['consequence'] = f'Call the API with Authorization: Bearer {token}.'
        self.assertTrue(self.call('memory_record_result', **draft)['isError'])
        for path in (self.root / 'project-brain').rglob('*'):
            if path.is_file():
                self.assertNotIn(token, path.read_text(encoding='utf-8', errors='replace'))

    def test_deeply_nested_json_is_a_parse_error_and_the_server_stays_up(self):
        # Under the 128 KiB message limit, past the parser's recursion depth.
        deep = '[' * 60000 + ']' * 60000
        self.assertLess(len(deep), 128 * 1024)
        messages = [json.dumps({'jsonrpc': '2.0', 'id': 0, 'method': 'initialize',
                                'params': {'protocolVersion': '2025-06-18', 'capabilities': {},
                                           'clientInfo': {'name': 'test', 'version': '1'}}}),
                    json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}),
                    deep, json.dumps({'jsonrpc': '2.0', 'id': 7, 'method': 'tools/list'})]
        result = subprocess.run([sys.executable, str(self.server), '--root', str(self.root)],
            input=''.join(line + '\n' for line in messages), capture_output=True, text=True, timeout=20)
        self.assertEqual(0, result.returncode, result.stderr)
        replies = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(-32700, replies[1]['error']['code'])
        self.assertEqual(7, replies[2]['id'])
        self.assertIn('tools', replies[2]['result'])

    def test_invalid_jsonrpc_shape_and_repeated_initialize(self):
        replies = self.exchange([[], {'jsonrpc': '1.0', 'method': 'ping', 'id': 1},
            {'jsonrpc': '2.0', 'method': 'ping', 'id': True},
            {'jsonrpc': '2.0', 'method': 'initialize', 'id': 2,
             'params': {'protocolVersion': '2025-06-18'}}])
        self.assertEqual([-32600, -32600, -32600, -32602], [r['error']['code'] for r in replies])

    def test_unicode_result_uses_utf8_even_with_ascii_process_encoding(self):
        draft = self.draft(self.start()); draft['progress'] = 'Проверено распределение кобальта.'
        requests = [{'jsonrpc': '2.0', 'id': 0, 'method': 'initialize',
            'params': {'protocolVersion': '2025-06-18'}},
            {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
            {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
             'params': {'name': 'memory_record_result', 'arguments': draft}},
            {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/call',
             'params': {'name': 'memory_retrieve', 'arguments': {'task_id': 'TASK-MCP', 'query': 'кобальт'}}}]
        result = subprocess.run([sys.executable, str(self.server), '--root', str(self.root)],
            input=''.join(json.dumps(m) + '\n' for m in requests).encode(), capture_output=True, timeout=20,
            env={**os.environ, 'PYTHONIOENCODING': 'ascii'})
        self.assertEqual(0, result.returncode, result.stderr)
        replies = [json.loads(line) for line in result.stdout.decode('utf-8').splitlines()][1:]
        self.assertTrue(all(not r['result']['isError'] for r in replies), replies)
        self.assertEqual(draft['progress'], self.cli_call('get', '--task-id', 'TASK-MCP')['progress'])


if __name__ == '__main__': unittest.main()
