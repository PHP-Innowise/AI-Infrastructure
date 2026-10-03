"""Real queue, OS isolation and three deterministic native CLI envelopes."""
from __future__ import annotations

from copy import deepcopy
import json
import os
from pathlib import Path
import shutil
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from tests import test_harness_web as http_helpers
from harness import providers, web
from harness.system_discovery import collect, validate_proposal
from ai_system_execution import load
from ai_system_lib import SystemError


ADAPTER = '''#!/usr/bin/env python3
import json, pathlib, sys, time
prompt = sys.stdin.read()
if '--workspace' in sys.argv:
    assert not prompt
    prompt = sys.argv[sys.argv.index('--') + 1]
value = json.loads(prompt.split('Discovery input:\\n', 1)[1])
inventory = json.loads(pathlib.Path('evidence/inventory.json').read_text())
all_text = ''.join(pathlib.Path(v['copy']).read_text() for files in inventory.values() for v in files.values())
if 'SLOW_DISCOVERY' in all_text:
    time.sleep(30)
if 'FAIL_DISCOVERY' in all_text:
    sys.exit(7)
reports = []
for sid in value['services']:
    files = inventory[sid]
    owner_sources = ['CODEOWNERS'] if 'CODEOWNERS' in files else []
    contract = {'id': sid + '.created', 'kind': 'event', 'version': '1', 'sources': ['openapi.yaml']}
    consumes = [{'service': 'orders', 'contract': 'orders.created', 'version': '1'}] if sid == 'payments' else []
    passport = {'schema_version': 1, 'id': sid, 'description': 'Manages ' + sid,
        'owner': 'team-' + sid if owner_sources else 'unknown', 'relationships_complete': False,
        'capabilities': [{'id': 'create', 'description': 'Create ' + sid, 'status': 'implemented', 'sources': ['src/app.py'], 'keywords': [sid, 'create']}],
        'provides': [contract], 'consumes': consumes,
        'sources': [{'path': name, 'kind': entry['kind']} for name, entry in files.items()]}
    reports.append({'id': sid, 'passport': passport, 'owner_sources': owner_sources,
        'description_sources': ['README.md'], 'consumption_sources': [{'service': 'orders', 'contract': 'orders.created', 'sources': ['src/app.py']}] if consumes else [],
        'uncertainties': ['Complete dependency coverage requires review.']})
report = {'name': 'AI discovered system', 'services': reports,
    'shared_sources': [{'path': 'AGENTS.md', 'kind': 'policy'}], 'warnings': []}
if 'BAD_PATH' in all_text:
    reports[0]['passport']['capabilities'][0]['sources'] = ['../outside.py']
if 'MISSING_SERVICE' in all_text:
    report['services'].pop()
if 'BAD_DEPENDENCY' in all_text:
    reports[-1]['passport']['consumes'][0]['version'] = '2'
if 'AUTH_FAILURE' in all_text:
    print(json.dumps({'type': 'assistant', 'error': 'authentication_failed', 'message': {'role': 'assistant', 'content': []}}))
    print(json.dumps({'type': 'result', 'subtype': 'success', 'is_error': True,
                      'result': 'Failed to authenticate: OAuth session expired and could not be refreshed'}))
    sys.stderr.write('Claude configuration file not found at: /home/user/.claude.json\\n')
    sys.exit(1)
if '--json' in sys.argv:
    print(json.dumps({'type': 'item.completed', 'item': {'type': 'agent_message', 'text': json.dumps(report)}}))
    print(json.dumps({'type': 'turn.completed'}))
elif '--json-schema' in sys.argv:
    copy = inventory['orders']['README.md']['copy']
    print(json.dumps({'type': 'assistant', 'message': {'role': 'assistant', 'content': [
        {'type': 'tool_use', 'id': 'toolu_1', 'name': 'Read', 'input': {'file_path': copy}}]}}))
    print(json.dumps({'type': 'result', 'subtype': 'success', 'is_error': False, 'result': '', 'structured_output': report}))
else:
    print(json.dumps({'type': 'result', 'subtype': 'success', 'is_error': False, 'result': json.dumps(report)}))
'''


class DiscoveryTests(unittest.TestCase):
    request = http_helpers.HarnessWebTests.request
    post = http_helpers.HarnessWebTests.post
    close_server = http_helpers.HarnessWebTests.close_server

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='system-discovery-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'system'; self.project.mkdir()
        (self.project / 'AGENTS.md').write_text('Use verified source evidence.\n')
        for sid in ('orders', 'payments'):
            folder = self.project / sid
            (folder / 'src').mkdir(parents=True)
            (folder / 'README.md').write_text('Manages ' + sid)
            (folder / 'CODEOWNERS').write_text('* @team-' + sid)
            (folder / 'openapi.yaml').write_text('version: 1\nevent: ' + sid + '.created\n')
            (folder / 'src/app.py').write_text('# Creates ' + sid + '; consumes orders.created version 1\n')
        adapter = self.root / 'fixture-agent'; adapter.write_text(ADAPTER); adapter.chmod(0o700)
        discovery = patch.object(providers, 'discover_providers', return_value=[{
            'id': pid, 'name': 'Fixture ' + pid, 'available': True, 'executable': str(adapter),
            'detail': 'No paid model calls'} for pid in ('codex', 'claude', 'cursor')])
        discovery.start(); self.addCleanup(discovery.stop)
        models = patch.object(providers, 'model_options', return_value={'models': [], 'efforts': [], 'detail': 'Fixture'})
        models.start(); self.addCleanup(models.stop)
        self.server = web.HarnessServer(('127.0.0.1', 0), self.root / 'state', [self.project])
        self.closed = False
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={'poll_interval': .01}, daemon=True)
        self.thread.start(); self.addCleanup(self.close_server)
        self.project_id = next(iter(self.server.sessions.projects)); self.token = self.server.token

    def draft(self, folders=('orders', 'payments')):
        status, draft, _ = self.post('/api/systems/editor', {'project_id': self.project_id, 'config_path': 'system.json'})
        self.assertEqual(200, status, draft)
        for folder in folders:
            status, service, _ = self.post('/api/systems/service', {'project_id': self.project_id, 'folder': folder})
            self.assertEqual(200, status, service)
            service.pop('path'); draft['services'].append(service)
        return draft

    def start(self, **changes):
        status, job, _ = self.post('/api/system-discoveries', {'editor': self.draft(), 'provider': 'codex', **changes})
        self.assertEqual(201, status, job)
        return job

    def wait_job(self, job, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            status, result, _ = self.request('/api/system-discoveries/' + job['id'])
            self.assertEqual(200, status, result)
            if not result['active']:
                return result
            time.sleep(.04)
        self.fail('AI discovery did not settle')

    def private_request(self, job):
        context = self.server.sessions.get(job['id'])['system_discovery']
        folder = self.root / 'state/system-discovery' / context['run_id']
        return load(folder, 'request-' + context['nonce'] + '.json'), folder

    def test_all_native_providers_fill_save_and_route_without_writing_sources(self):
        original = {str(p): p.read_bytes() for p in self.project.rglob('*') if p.is_file()}
        for provider in ('codex', 'claude', 'cursor'):
            job = self.wait_job(self.start(provider=provider))
            self.assertEqual('completed', job['status'], job)
            proposal = job['proposal']; self.assertIsNotNone(proposal)
            passports = [s['passport'] for s in proposal['editor']['services']]
            self.assertEqual(['orders', 'payments'], [p['id'] for p in passports])
            self.assertEqual('team-orders', passports[0]['owner'])
            self.assertEqual('implemented', passports[0]['capabilities'][0]['status'])
            self.assertEqual('orders.created', passports[1]['consumes'][0]['contract'])
            self.assertEqual(original, {str(p): p.read_bytes() for p in self.project.rglob('*') if p.is_file()})
        draft = proposal['editor']
        for service in draft['services']:
            service.pop('path')
        status, preview, _ = self.post('/api/systems/preview', draft)
        self.assertEqual(200, status, preview)
        status, saved, _ = self.post('/api/systems/apply', {'preview_id': preview['preview_id']})
        self.assertEqual(200, status, saved)
        self.assertEqual(1, len(saved['catalog']['relationships']))

    def test_discovery_agent_activity_names_original_service_paths(self):
        job = self.wait_job(self.start(provider='claude'))
        self.assertEqual('completed', job['status'], job)
        events = self.server.sessions.events(job['id'])
        self.assertEqual(['running', 'completed'], [e['status'] for e in events if e['kind'] == 'agent'])
        reads = [e for e in events if e['kind'] == 'agent_activity' and e.get('tool') == 'Read']
        self.assertEqual([('discovery', 'orders', 'README.md')], [(e['agent'], e['service'], e['path']) for e in reads])
        self.assertTrue(all(e['kind'] in ('status', 'text', 'error', 'result') for e in job['events']))

    def test_blocked_sandbox_is_named_before_any_scan_is_queued(self):
        from harness import discovery_sandbox
        from subprocess import CompletedProcess
        denied = CompletedProcess([], 1, '', 'bwrap: setting up uid map: Permission denied\n')
        def sysctl(name):
            return '1' if name == 'kernel.apparmor_restrict_unprivileged_userns' else None
        with patch.object(discovery_sandbox, '_probe', return_value=denied), \
                patch.object(discovery_sandbox, '_sysctl', side_effect=sysctl):
            status, result, _ = self.post('/api/system-discoveries', {'editor': self.draft(), 'provider': 'claude'})
            boot = self.request('/api/bootstrap')[1]
        self.assertEqual(400, status, result)
        for message in (result['error'], boot['runtime']['discovery_sandbox']):
            self.assertIn('setting up uid map: Permission denied', message)
            self.assertIn('AppArmor', message)
            self.assertIn('Troubleshooting AI discovery', message)
        self.assertFalse(self.server.sessions.list())
        self.assertIsNone(self.request('/api/bootstrap')[1]['runtime']['discovery_sandbox'])

    def test_expired_cli_login_is_reported_with_its_fix(self):
        (self.project / 'orders/README.md').write_text('AUTH_FAILURE')
        job = self.wait_job(self.start(provider='claude'))
        self.assertEqual('failed', job['status'], job)
        failure = next(e['text'] for e in job['events'] if e['kind'] == 'error')
        self.assertIn('Claude Code is not signed in or its login expired', failure)
        self.assertIn('OAuth session expired', failure)
        self.assertIn('claude auth login', failure)
        self.assertNotIn('configuration file not found', failure)
        finished = [e for e in self.server.sessions.events(job['id']) if e['kind'] == 'agent'][-1]
        self.assertEqual(('blocked', True), (finished['status'], 'claude auth login' in finished['reason']))

    def test_failure_reasons_name_the_cause_and_skip_cli_noise(self):
        from harness.discovery_runner import failure
        base = {'returncode': 1, 'error': None, 'stdout': b'', 'stderr_tail': b''}
        cases = [({'stderr_tail': b'bwrap: setting up uid map: Permission denied\n'}, None, 'sandbox could not start'),
                 ({'stderr_tail': b'bwrap: execvp /opt/codex: No such file or directory\n'}, None, 'could not start inside'),
                 ({'error': 'timeout'}, None, 'scan timeout (30 s)'),
                 ({}, 'Rate limit reached: 429', 'usage or rate limit'),
                 ({'returncode': 2, 'stderr_tail': b'Error: unexpected token\nA backup file exists at: /x\n'}, None,
                  'exited with code 2 (Error: unexpected token)')]
        for changes, reported, expected in cases:
            with self.subTest(expected=expected):
                self.assertIn(expected, failure('codex', {**base, **changes}, reported, 30))

    def test_invalid_ai_output_is_not_exposed_as_a_partial_proposal(self):
        for marker in ('BAD_PATH', 'MISSING_SERVICE', 'BAD_DEPENDENCY', 'FAIL_DISCOVERY'):
            with self.subTest(marker=marker):
                (self.project / 'orders/README.md').write_text(marker)
                job = self.wait_job(self.start())
                self.assertEqual('failed', job['status'], job)
                self.assertIsNone(job['proposal'])
                self.assertFalse((self.project / 'system.json').exists())

    def test_source_add_change_and_passport_change_make_completed_proposals_stale(self):
        for mutation in ('source', 'new_file', 'passport'):
            job = self.wait_job(self.start()); self.assertEqual('completed', job['status'], job)
            target = self.project / 'orders' / {'source': 'src/app.py', 'new_file': 'new.py', 'passport': 'ai-service.json'}[mutation]
            old = target.read_bytes() if target.exists() else None
            target.write_text('# externally changed')
            status, stale, _ = self.request('/api/system-discoveries/' + job['id'])
            self.assertEqual(200, status); self.assertEqual('stale', stale['status']); self.assertIsNone(stale['proposal'])
            target.write_bytes(old) if old is not None else target.unlink()

    def test_cancel_and_timeout_do_not_populate_or_save(self):
        (self.project / 'orders/README.md').write_text('SLOW_DISCOVERY')
        job = self.start()
        self.assertEqual(400, self.post('/api/sessions/' + job['id'] + '/messages', {'prompt': 'edit files'})[0])
        self.assertEqual(200, self.post('/api/sessions/' + job['id'] + '/cancel', {})[0])
        job = self.wait_job(job); self.assertEqual('cancelled', job['status']); self.assertIsNone(job['proposal'])
        job = self.wait_job(self.start(timeout=1)); self.assertEqual('failed', job['status']); self.assertIsNone(job['proposal'])
        self.assertTrue(any('timeout' in e.get('text', '').lower() or 'time limit' in e.get('text', '').lower() for e in job['events']), job['events'])
        self.assertFalse((self.project / 'orders/ai-service.json').exists())

    def test_ids_ownership_claims_and_authority_are_validated(self):
        job = self.wait_job(self.start()); request, folder = self.private_request(job)
        report = load(folder, 'proposal.json')
        mutations = [lambda r: r['services'][0].update(id='invented'),
                     lambda r: r['services'][0].update(owner_sources=[]),
                     lambda r: r['services'][0]['passport'].update(relationships_complete=True),
                     lambda r: r['services'][0]['passport']['capabilities'][0].update(sources=['README.md']),
                     lambda r: r['services'][0]['passport']['sources'][0].update(kind='memory'),
                     lambda r: r['services'][0]['passport']['sources'][0].update(kind='code')]
        for mutate in mutations:
            invalid = deepcopy(report); mutate(invalid)
            with self.assertRaises(SystemError):
                validate_proposal(invalid, request)

    def test_bounded_collection_refuses_links_credentials_binary_and_unreviewed_memory(self):
        folder = self.project / 'orders'
        (folder / '.env').write_text('password=secret')
        (folder / 'secret.md').write_text('api_key=' + 'a' * 30)
        (folder / 'binary.md').write_bytes(b'a\0b')
        (folder / 'link.md').symlink_to(folder / 'README.md')
        (folder / 'hard-source.md').write_text('Hard link contents')
        os.link(folder / 'hard-source.md', folder / 'hardlink.md')
        (folder / 'memory-bank/chunks').mkdir(parents=True)
        (folder / 'memory-bank/chunks/MEM-unreviewed.md').write_text('No verified metadata')
        (folder / '.cursor/rules').mkdir(parents=True)
        (folder / '.cursor/rules/project.mdc').write_text('Use service-owned contracts.')
        (folder / '.cursor/settings.json').write_text('{"private": "configuration"}')
        evidence, signature = collect(folder)
        self.assertEqual({'README.md', 'CODEOWNERS', 'openapi.yaml', 'src/app.py', '.cursor/rules/project.mdc'}, set(evidence))
        self.assertEqual('policy', evidence['.cursor/rules/project.mdc']['kind'])
        self.assertGreater(signature['omitted'], 0)

    def test_stale_evidence_is_refused_at_preview_and_at_apply(self):
        for stage in ('preview', 'apply'):
            job = self.wait_job(self.start()); self.assertEqual('completed', job['status'], job)
            draft = job['proposal']['editor']
            for service in draft['services']:
                service.pop('path')
            if stage == 'apply':
                status, preview, _ = self.post('/api/systems/preview', draft)
                self.assertEqual(200, status, preview)
            target = self.project / 'orders/src/app.py'; prior = target.read_bytes()
            target.write_text('# source changed after proposal was displayed')
            status, result, _ = self.post('/api/systems/preview', draft) if stage == 'preview' else self.post('/api/systems/apply', {'preview_id': preview['preview_id']})
            self.assertEqual(400, status, result)
            self.assertFalse((self.project / 'system.json').exists())
            target.write_bytes(prior)

    def test_sandbox_cannot_read_original_or_unselected_project_files(self):
        from harness.discovery_sandbox import sandbox_command
        from ai_system_execution import run_process
        workspace = self.root / 'isolated/agent'; (workspace / 'evidence').mkdir(parents=True)
        (workspace / 'evidence/source.txt').write_text('safe evidence')
        sentinel = self.root / 'other-project.env'; sentinel.write_text('private sentinel')
        script = self.root / 'check-isolation'
        script.write_text('#!/usr/bin/env python3\nfrom pathlib import Path\n'
                          'assert Path("evidence/source.txt").read_text()=="safe evidence"\n'
                          'for name in ' + repr([str(sentinel), str(self.project / 'orders/README.md')]) + ':\n'
                          ' try: Path(name).read_text()\n'
                          ' except (FileNotFoundError, PermissionError): pass\n'
                          ' else: raise AssertionError("outside evidence is readable")\n'
                          'try: Path("evidence/source.txt").write_text("changed")\n'
                          'except (PermissionError, OSError): pass\n'
                          'else: raise AssertionError("evidence is writable")\n'
                          'Path("scratch.txt").write_text("allowed")\n')
        script.chmod(0o700)
        command = sandbox_command(workspace, [str(script)], 'codex', [self.project])
        result = run_process(command, workspace, '', 10)
        self.assertEqual(0, result['returncode'], result)
        self.assertEqual('allowed', (workspace / 'scratch.txt').read_text())

    def test_duplicate_blank_folder_names_receive_distinct_frozen_ids(self):
        for prefix in ('a', 'b'):
            shutil.copytree(self.project / 'orders', self.project / prefix / 'service')
        draft = self.draft(('a/service', 'b/service'))
        status, job, _ = self.post('/api/system-discoveries', {'editor': draft, 'provider': 'codex'})
        self.assertEqual(201, status, job)
        request, _ = self.private_request(job)
        self.assertEqual(['service', 'service-2'], [s['passport']['id'] for s in request['editor']['services']])
        self.assertEqual('completed', self.wait_job(job)['status'])

    def test_restart_interrupts_scan_and_retains_the_editable_original_draft(self):
        (self.project / 'orders/README.md').write_text('SLOW_DISCOVERY')
        job = self.start()
        self.close_server()
        self.server = web.HarnessServer(('127.0.0.1', 0), self.root / 'state', [self.project])
        self.closed = False; self.token = self.server.token
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={'poll_interval': .01}, daemon=True)
        self.thread.start()
        status, interrupted, _ = self.request('/api/system-discoveries/' + job['id'])
        self.assertEqual(200, status, interrupted)
        self.assertEqual('interrupted', interrupted['status']); self.assertIsNone(interrupted['proposal'])
        self.assertEqual(['orders', 'payments'], [s['passport']['id'] for s in interrupted['draft']['services']])
        self.assertFalse((self.project / 'system.json').exists())

    def test_request_boundaries_and_failed_staging_cleanup(self):
        draft = self.draft()
        invalid = deepcopy(draft); invalid['services'][0]['folder'] = '../outside'
        self.assertEqual(400, self.post('/api/system-discoveries', {'editor': invalid, 'provider': 'codex'})[0])
        self.assertEqual(400, self.post('/api/system-discoveries?extra=1', {'editor': draft, 'provider': 'codex'})[0])
        self.assertEqual(400, self.post('/api/system-discoveries', {'editor': draft, 'provider': 'command'})[0])
        for invalid_value in (None, [], 'text', {'id': 'partial'}):
            invalid = deepcopy(draft); invalid['services'][0]['passport'] = invalid_value
            self.assertEqual(400, self.post('/api/system-discoveries', {'editor': invalid, 'provider': 'codex'})[0])
        with patch('harness.system_discovery.isolation_backend', return_value=None):
            self.assertEqual(400, self.post('/api/system-discoveries', {'editor': draft, 'provider': 'codex'})[0])
        with patch('harness.system_discovery.save', side_effect=SystemError('Staging failed')):
            self.assertEqual(400, self.post('/api/system-discoveries', {'editor': draft, 'provider': 'codex'})[0])
        directory = self.root / 'state/system-discovery'
        self.assertEqual([], list(directory.iterdir()) if directory.exists() else [])

    def test_mac_sandbox_profile_has_read_allowlist_and_no_global_temp_writes(self):
        from harness.discovery_sandbox import sandbox_command
        workspace = self.root / 'mac/agent'; (workspace / 'evidence').mkdir(parents=True)
        with patch('harness.discovery_sandbox.isolation_backend', return_value=('seatbelt', '/usr/bin/sandbox-exec')):
            command = sandbox_command(workspace, ['/usr/bin/true'], 'claude', [self.project])
        profile = Path(command[2]).read_text()
        self.assertIn('(deny file-read*)', profile)
        self.assertIn('(deny file-write*', profile)
        self.assertNotIn('(allow file-write* (subpath "/private/tmp"))', profile)
        self.assertNotIn(str(self.project), profile)
        self.assertTrue(any(value.startswith('TMPDIR=') for value in command))

    def test_source_parent_of_public_runtime_is_refused(self):
        from harness.discovery_sandbox import sandbox_command
        from harness.sessions import SessionError
        workspace = self.root / 'sandbox/agent'; (workspace / 'evidence').mkdir(parents=True)
        for source_root in (Path('/'), Path('/etc')):
            with self.assertRaises(SessionError):
                sandbox_command(workspace, ['/usr/bin/true'], 'codex', [source_root])


if __name__ == '__main__':
    unittest.main()
