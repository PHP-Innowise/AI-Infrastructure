"""Real localhost API, native Brain and deterministic native CLI subprocesses."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from tests import test_harness_web as http_helpers
from tests.test_ai_system_execution import ADAPTER
from harness import providers, web

ROOT = Path(__file__).resolve().parents[1]


class HarnessSystemTests(unittest.TestCase):
    request = http_helpers.HarnessWebTests.request
    post = http_helpers.HarnessWebTests.post
    close_server = http_helpers.HarnessWebTests.close_server

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'project'
        shutil.copytree(ROOT / 'docs/examples/ai-system', self.project)
        self.adapter = self.root / 'fixture-codex'
        self.adapter.write_text(ADAPTER)
        self.adapter.chmod(0o700)
        discovery = patch.object(providers, 'discover_providers', return_value=[{
            'id': pid, 'name': 'Fixture ' + pid, 'available': True,
            'executable': str(self.adapter), 'detail': 'Deterministic fixture; no model calls'} for pid in ('codex', 'claude', 'cursor')])
        discovery.start(); self.addCleanup(discovery.stop)
        models = patch.object(providers, 'model_options', return_value={'models': [], 'efforts': [], 'detail': 'Fixture'})
        models.start(); self.addCleanup(models.stop)
        self.server = web.HarnessServer(('127.0.0.1', 0), self.root / 'state', [self.project])
        self.closed = False
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       kwargs={'poll_interval': .01}, daemon=True)
        self.thread.start(); self.addCleanup(self.close_server)
        self.project_id = next(iter(self.server.sessions.projects))
        self.token = self.server.token

    def prepare(self, **changes):
        data = {'project_id': self.project_id, 'config_path': 'system.json',
                'task': 'Investigate order cancellation and its consumers', 'change_id': 'change-001',
                'services': ['orders'], **changes}
        status, result, _ = self.post('/api/system-runs', data)
        self.assertEqual(201, status, result)
        return result

    def act(self, run, action, **changes):
        return self.post('/api/system-runs/' + run['id'],
                         {'action': action, 'revision': run['revision'], **changes})

    def wait_run(self, run, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            status, result, _ = self.request('/api/system-runs/' + run['id'])
            self.assertEqual(200, status, result)
            if not result['active']:
                return result
            time.sleep(.05)
        self.fail('System run did not settle')

    def test_native_provider_selection_and_pinned_recovery(self):
        for pid in ('claude', 'cursor'):
            with self.subTest(provider=pid):
                marker = self.project / 'services/orders/fail-worker'
                marker.write_text('fixture failure')
                run = self.prepare(change_id='change-' + pid)
                self.assertEqual({'codex', 'claude', 'cursor'}, {p['id'] for p in run['providers']})
                self.assertTrue(all('executable' not in p for p in run['providers']))
                status, run, _ = self.act(run, 'execute', provider=pid, mode='read-only')
                self.assertEqual(200, status, run)
                run = self.wait_run(run)
                self.assertEqual('blocked', run['status'])
                self.assertEqual(pid, run['provider'])
                self.assertEqual(pid, run['execution']['provider'])
                session = self.server.sessions.get(run['session_id'])
                self.assertEqual(pid, session['provider'])
                self.assertEqual(400, self.act(run, 'resume', provider='codex')[0])
                self.server.sessions.providers[pid]['available'] = False
                self.assertEqual(400, self.act(run, 'resume', retry_step='service-orders')[0])
                self.server.sessions.providers[pid]['available'] = True
                marker.unlink()
                status, run, _ = self.act(run, 'resume', retry_step='service-orders')
                self.assertEqual(200, status, run)
                run = self.wait_run(run)
                self.assertEqual('completed', run['status'])
                self.assertEqual(pid, run['provider'])
                self.assertEqual(5, len(run['receipts']))
                self.assertTrue(all(t['closed'] for t in run['execution']['tasks'].values()))
                self.assertEqual(1, next(s for s in run['execution']['steps'] if s['id']=='contracts')['attempt'])
                self.assertEqual(2, next(s for s in run['execution']['steps'] if s['id']=='service-orders')['attempt'])

    def test_legacy_codex_request_and_metadata_remain_recoverable(self):
        marker = self.project / 'fail-worker'
        marker.touch()
        run = self.prepare()
        status, run, _ = self.act(run, 'execute', timeout=10)
        self.assertEqual(200, status, run)
        run = self.wait_run(run)
        self.assertEqual('blocked', run['status'])
        with self.server.sessions.lock:
            record = self.server.systems._record(run['id'])
            record.pop('provider')
            self.server.systems._save(record)
        run = self.request('/api/system-runs/' + run['id'])[1]
        self.assertEqual('codex', run['provider'])
        directory = self.server.systems.root / run['id']
        request_file = next(directory.glob('request-*.json'))
        request = json.loads(request_file.read_text())
        request.pop('provider')
        request['retry_step'] = 'contracts'
        request_file.write_text(json.dumps(request))
        marker.unlink()
        result = subprocess.run([sys.executable, str(ROOT / 'harness/src/harness/system_runner.py'),
                                 '--request', str(request_file)], capture_output=True, timeout=30)
        self.assertEqual(0, result.returncode, result.stdout)
        done = self.request('/api/system-runs/' + run['id'])[1]
        self.assertEqual('completed', done['status'])
        self.assertEqual('codex', done['execution']['provider'])

    def test_unavailable_or_untrusted_provider_cannot_launch(self):
        run = self.prepare()
        self.server.sessions.providers['claude']['available'] = False
        for extra in ({'provider': 'claude'}, {'provider': {}}, {'provider': 'command'},
                      {'provider': 'cursor', 'executable': str(self.adapter)}):
            self.assertEqual(400, self.act(run, 'execute', **extra)[0])
        self.assertFalse((self.project / 'project-brain').exists())
        self.assertFalse(self.server.sessions.list())

    def test_catalog_and_http_authority_boundaries(self):
        body = {'project_id': self.project_id, 'config_path': 'system.json'}
        self.assertEqual(403, self.post('/api/systems/catalog', body, headers={'X-Harness-Token': None})[0])
        status, data, _ = self.post('/api/systems/catalog', body)
        self.assertEqual(200, status, data)
        self.assertEqual(3, len(data['catalog']['services']))
        self.assertIn('flowchart', data['diagram'])
        self.assertEqual(200, self.request('/system.js')[0])
        for config in ('../project/system.json', str(self.project/'system.json'), './system.json'):
            self.assertEqual(400, self.post('/api/systems/catalog', {**body, 'config_path': config})[0])
        (self.project/'linked.json').symlink_to(self.project/'system.json')
        self.assertEqual(400, self.post('/api/systems/catalog', {**body, 'config_path': 'linked.json'})[0])
        for field in ('allow_roots', 'executable', 'run_dir', 'provider'):
            self.assertEqual(400, self.post('/api/systems/catalog', {**body, field: str(self.root)})[0])
        self.assertEqual(400, self.request('/api/system-runs?project_id=missing')[0])
        self.assertEqual(404, self.request('/api/system-runs/../../etc/passwd')[0])

    def test_external_service_requires_registered_project(self):
        outside = self.root/'outside'
        shutil.copytree(self.project/'services/orders', outside)
        config = json.loads((self.project/'system.json').read_text())
        config['services'][0]['root'] = str(outside)
        (self.project/'system.json').write_text(json.dumps(config))
        body = {'project_id': self.project_id, 'config_path': 'system.json'}
        data = self.post('/api/systems/catalog', body)[1]
        service = next(s for s in data['catalog']['services'] if s['id']=='orders')
        self.assertEqual('denied', service['access'])
        run = self.prepare()
        self.assertEqual(400, self.act(run, 'execute')[0])
        self.assertFalse((outside/'project-brain').exists())
        self.server.sessions.add_project({'path': str(outside)})
        self.assertEqual('available', next(s for s in self.post('/api/systems/catalog', body)[1]['catalog']['services'] if s['id']=='orders')['access'])

    def test_history_limit_is_per_project(self):
        run = self.prepare()
        other = self.root/'other-project'; other.mkdir()
        project = self.server.sessions.add_project({'path': str(other)})
        with self.server.sessions.lock:
            for number in range(201):
                self.server.systems._save({**run, 'id': format(number, '032x'), 'project_id': project['id']})
        status, history, _ = self.request('/api/system-runs?project_id='+self.project_id)
        self.assertEqual(200, status, history)
        self.assertEqual([run['id']], [record['id'] for record in history['runs']])

    def test_stale_selection_and_settings_fail_before_native_task_mutation(self):
        run = self.prepare(task='Investigate order cancellation.\nCheck contract consumers.')
        self.assertEqual('Investigate order cancellation. Check contract consumers.', run['plan']['context']['task'])
        self.assertEqual(400, self.act(run, 'execute', revision=True)[0])
        self.assertEqual(400, self.act(run, 'execute', provider='unknown')[0])
        self.assertEqual(400, self.act(run, 'execute', timeout=True)[0])
        (self.project/'services/orders/specs/cancellation.md').write_text('Changed source')
        self.assertEqual(400, self.act(run, 'execute')[0])
        self.assertFalse((self.project/'project-brain').exists())
        run = self.prepare(services=[], task='Unmatched topic')
        self.assertEqual('needs_selection', run['plan']['status'])
        self.assertEqual(400, self.act(run, 'execute')[0])

    def test_actual_queue_receipts_native_tasks_and_protected_followups(self):
        run = self.prepare()
        status, queued, _ = self.act(run, 'execute', mode='read-only', timeout=10)
        self.assertEqual(200, status, queued)
        self.assertTrue(queued['active'])
        self.assertEqual(400, self.act(run, 'execute')[0])
        result = self.wait_run(queued)
        self.assertEqual('completed', result['status'], result['events'])
        self.assertEqual(5, len(result['receipts']))
        self.assertTrue(all(task['closed'] and task['uuid'] for task in result['execution']['tasks'].values()))
        self.assertIn('knowledge', result['handoff'])
        self.assertEqual(400, self.act(result, 'execute')[0])
        self.assertEqual(400, self.act(result, 'resume')[0])
        sid = result['session_id']
        self.assertEqual(400, self.post('/api/sessions/'+sid+'/messages', {'prompt':'Run a new command'})[0])
        status, listing, _ = self.request('/api/system-runs?project_id='+self.project_id)
        self.assertEqual(200, status)
        self.assertEqual('completed', listing['runs'][0]['status'])

    def test_failure_explicit_retry_skips_completed_workers(self):
        marker = self.project/'services/orders/fail-worker'; marker.touch()
        run = self.prepare()
        status, queued, _ = self.act(run, 'execute', timeout=10)
        self.assertEqual(200, status, queued)
        blocked = self.wait_run(queued)
        self.assertEqual('blocked', blocked['status'])
        steps = {s['id']: s for s in blocked['execution']['steps']}
        self.assertEqual('completed', steps['contracts']['status'])
        marker.unlink()
        status, resumed, _ = self.act(blocked, 'resume', retry_step='service-orders')
        self.assertEqual(200, status, resumed)
        done = self.wait_run(resumed)
        self.assertEqual('completed', done['status'], done['events'])
        steps = {s['id']: s for s in done['execution']['steps']}
        self.assertEqual(1, steps['contracts']['attempt'])
        self.assertEqual(2, steps['service-orders']['attempt'])

    def test_cancel_claude_and_cursor_reaps_workers_and_preserves_provider(self):
        for pid in ('claude', 'cursor'):
            with self.subTest(provider=pid):
                self._cancel_reaps_detached_worker_and_descendant(pid)

    def test_cancel_reaps_detached_worker_and_descendant(self):
        self._cancel_reaps_detached_worker_and_descendant('codex')

    def _cancel_reaps_detached_worker_and_descendant(self, provider):
        extra = '''
if (root / "hold-worker").exists():
    import subprocess, time
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"])
    (root / "worker-pids.json").write_text(json.dumps([os.getpid(), child.pid]))
    time.sleep(120)
'''
        self.adapter.write_text(ADAPTER.replace('marker = root / "fail-worker"', extra+'\nmarker = root / "fail-worker"'))
        marker = self.project/'hold-worker'; marker.touch()
        run = self.prepare()
        status, queued, _ = self.act(run, 'execute', timeout=120, provider=provider)
        self.assertEqual(200, status, queued)
        pid_file = self.project/'worker-pids.json'
        deadline = time.monotonic()+15
        while not pid_file.exists() and time.monotonic()<deadline:
            time.sleep(.05)
        self.assertTrue(pid_file.exists())
        pids = json.loads(pid_file.read_text())
        self.assertEqual(200, self.act(queued, 'cancel')[0])
        stopped = self.wait_run(queued)
        self.assertEqual('interrupted', stopped['status'])
        for pid in pids:
            deadline = time.monotonic()+5
            while self.alive(pid) and time.monotonic()<deadline:
                time.sleep(.05)
            self.assertFalse(self.alive(pid), 'Provider descendant survived cancellation')
        self.assertEqual(provider, stopped['provider'])
        pid_file.unlink()
        marker.unlink()
        status, resumed, _ = self.act(stopped, 'resume', retry_step='contracts')
        self.assertEqual(200, status, resumed)
        self.assertEqual('completed', self.wait_run(resumed)['status'])

    def test_durable_pending_launch_recovers_lost_session_link(self):
        run = self.prepare()
        original = self.server.systems._save
        def crash_after_enqueue(record):
            if record['session_id'] is not None and not record.get('pending_nonce'):
                raise OSError('Simulated interruption before linking the session')
            return original(record)
        with patch.object(self.server.systems, '_save', side_effect=crash_after_enqueue):
            self.assertEqual(400, self.act(run, 'execute', timeout=10, provider='claude')[0])
        # The saved pending nonce discovers the already committed session. A
        # retry must never create another session or execute the plan twice.
        recovered = self.request('/api/system-runs/'+run['id'])[1]
        self.assertIsNotNone(recovered['session_id'])
        self.assertIsNone(recovered['pending_nonce'])
        self.assertEqual(1, len(self.server.sessions.list()))
        self.assertIsNotNone(self.server.sessions.get(recovered['session_id'])['system_run'])
        self.assertEqual(400, self.act(recovered, 'execute')[0])
        done = self.wait_run(recovered)
        self.assertEqual('completed', done['status'])
        self.close_server()
        self.server = web.HarnessServer(('127.0.0.1',0), self.root/'state', [self.project])
        self.closed = False
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       kwargs={'poll_interval': .01}, daemon=True)
        self.thread.start(); self.token = self.server.token
        restored = self.request('/api/system-runs/'+run['id'])[1]
        self.assertEqual('completed', restored['status'])
        self.assertEqual('claude', restored['provider'])
        self.assertEqual(done['session_id'], restored['session_id'])
        self.assertEqual(400, self.act(restored, 'execute')[0])

    def test_watchdog_reaps_worker_when_executor_is_killed(self):
        pid_file = self.root/'orphan-pids.json'
        child = ('import os,sys,subprocess,time,json,pathlib; '
                 'p=subprocess.Popen([sys.executable,"-c","import time; time.sleep(120)"]); '
                 'pathlib.Path('+repr(str(pid_file))+').write_text(json.dumps([os.getpid(),p.pid])); '
                 'time.sleep(120)')
        owner_code = ('import sys,pathlib; sys.path.insert(0,'+repr(str(ROOT/'scripts'))+'); '
                      'import ai_system_execution as e; e.PROCESS_GUARD=pathlib.Path('
                      +repr(str(ROOT/'harness/src/harness/process_guard.py'))+'); '
                      'e.run_process([sys.executable,"-c",'+repr(child)+'],pathlib.Path('
                      +repr(str(self.root))+'),"",120)')
        owner = subprocess.Popen([sys.executable,'-c',owner_code], stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            deadline = time.monotonic()+10
            while not pid_file.exists() and time.monotonic()<deadline:
                time.sleep(.05)
            self.assertTrue(pid_file.exists())
            pids = json.loads(pid_file.read_text())
            owner.kill(); owner.wait(timeout=5)
            for pid in pids:
                deadline = time.monotonic()+5
                while self.alive(pid) and time.monotonic()<deadline:
                    time.sleep(.05)
                self.assertFalse(self.alive(pid), 'Provider survived executor death')
        finally:
            if owner.poll() is None:
                owner.kill()
            owner.wait(timeout=5)

    def session_events(self, sid):
        events, after = [], 0
        while True:
            status, page, _ = self.request(f'/api/sessions/{sid}?after={after}')
            self.assertEqual(200, status, page)
            events += page['events']
            if len(page['events']) < 250:
                return events
            after = page['events'][-1]['id']

    def test_agents_panel_events_launch_history_and_shared_folder_access(self):
        run = self.prepare(change_id='change-agents')
        self.assertEqual(400, self.act(run, 'execute', provider='cursor', mode='edit', access='all')[0])
        self.assertEqual(400, self.act(run, 'execute', provider='claude', access='everything')[0])
        marker = self.project / 'services/orders/fail-worker'
        marker.write_text('fixture failure')
        status, run, _ = self.act(run, 'execute', provider='claude', mode='edit', access='all')
        self.assertEqual(200, status, run)
        run = self.wait_run(run)
        self.assertEqual(('blocked', 'all'), (run['status'], run['access']))
        marker.unlink()
        status, run, _ = self.act(run, 'resume', retry_step='service-orders')
        self.assertEqual(200, status, run)
        run = self.wait_run(run)
        self.assertEqual(('completed', 'all'), (run['status'], run['access']))
        self.assertEqual(2, len(run['launches']))
        self.assertEqual(run['session_id'], run['launches'][-1]['session_id'])
        # Runner progress stays readable; agent activity is read per launch session.
        self.assertTrue(run['events'] and all(e['kind'] in ('status', 'text', 'error', 'result') for e in run['events']))
        self.assertEqual('Run completed. Process completion is not an independent verification of the task.',
                         run['events'][-1]['text'])
        first, second = (self.session_events(launch['session_id']) for launch in run['launches'])
        self.assertIn(('service-orders', 'blocked'), [(e['agent'], e['status']) for e in first if e['kind'] == 'agent'])
        lifecycle = [(e['agent'], e['status']) for e in second if e['kind'] == 'agent']
        # Every dispatch of the resumed launch starts and then completes, retrying orders first.
        self.assertEqual(['running', 'completed'] * (len(lifecycle) // 2), [status for _, status in lifecycle])
        self.assertEqual([agent for agent, _ in lifecycle][::2], [agent for agent, _ in lifecycle][1::2])
        self.assertEqual('service-orders', lifecycle[0][0])
        self.assertEqual('verify', lifecycle[-1][0])
        started = next(e for e in second if e['kind'] == 'agent' and e['agent'] == 'service-orders')
        selected = run['execution']['steps']
        services = sorted(s['service'] for s in selected if s['service'] != '__system__')
        self.assertEqual((services, services), (sorted(started['readable']), sorted(started['writable'])))
        reads = [e for e in second if e['kind'] == 'agent_activity' and e.get('tool') == 'Read' and e['agent'] == 'service-orders']
        self.assertEqual([('orders', 'spec.md')], [(e['service'], e['path']) for e in reads])
        self.assertTrue(all(e['attempt'] == 2 for e in second if e.get('agent') == 'service-orders'))
        self.assertNotIn('PRIVATE FILE CONTENT', json.dumps(first + second))
        usage = self.server.sessions.get(run['session_id'])['budget_usage']
        self.assertEqual(15 * len(lifecycle) // 2, usage['tokens'])

    def test_blocked_dispatch_names_the_cli_error_and_its_fix(self):
        marker = self.project / 'services/orders/auth-failure-worker'
        marker.touch()
        run = self.prepare(change_id='change-auth')
        status, run, _ = self.act(run, 'execute', provider='claude')
        self.assertEqual(200, status, run)
        run = self.wait_run(run)
        self.assertEqual(('blocked', 'worker_exit_failure'), (run['status'], run['execution']['error']))
        reasons = [e['text'] for e in run['events'] if e['kind'] == 'error' and 'orders agent' in e['text']]
        self.assertEqual(1, len(reasons), run['events'])
        self.assertIn('Claude Code is not signed in or its login expired', reasons[0])
        self.assertIn('claude auth login', reasons[0])
        finished = [e for e in self.session_events(run['session_id']) if e['kind'] == 'agent' and e['status'] == 'blocked']
        self.assertEqual(['service-orders'], [e['agent'] for e in finished])
        self.assertIn('claude auth login', finished[0]['reason'])
        marker.unlink()

    @staticmethod
    def alive(pid):
        try:
            # A zombie has exited; the host's init is responsible for reaping it.
            proc = Path('/proc')/str(pid)/'stat'
            if proc.exists() and proc.read_text().split(') ',1)[1].startswith('Z'):
                return False
            os.kill(pid, 0)
            return True
        except ProcessLookupError:
            return False


class NativeWindowsSystemTests(unittest.TestCase):
    """The native Windows Harness starts and names System Orchestration as unavailable."""
    request = http_helpers.HarnessWebTests.request
    post = http_helpers.HarnessWebTests.post
    close_server = http_helpers.HarnessWebTests.close_server
    REASON = 'System Orchestration needs Linux or macOS. It is not available on native Windows yet.'

    def test_server_imports_without_posix_only_modules(self):
        code = ("import sys\nfor name in ('fcntl', 'pwd', 'grp', 'resource', 'termios'):\n    sys.modules[name] = None\n"
                "sys.path[:0] = [sys.argv[1]]\nimport harness.web")
        completed = subprocess.run([sys.executable, '-c', code, str(ROOT / 'harness/src')],
                                   capture_output=True, text=True, timeout=60)
        self.assertEqual(0, completed.returncode, completed.stderr)

    def test_every_entry_point_refuses_and_the_server_still_starts(self):
        from harness import system_discovery, system_orchestration
        self.root = Path(tempfile.mkdtemp()); self.addCleanup(shutil.rmtree, self.root)
        project = self.root / 'project'
        shutil.copytree(ROOT / 'docs/examples/ai-system', project)
        for target in (patch.object(system_orchestration, 'UNAVAILABLE', self.REASON),
                       patch.object(system_discovery, 'UNAVAILABLE', self.REASON),
                       patch.object(web, 'SYSTEM_UNAVAILABLE', self.REASON),
                       patch.object(providers, 'discover_providers', return_value=[]),
                       patch.object(providers, 'model_options', return_value={'models': [], 'efforts': [], 'detail': 'Fixture'})):
            target.start(); self.addCleanup(target.stop)
        self.server = web.HarnessServer(('127.0.0.1', 0), self.root / 'state', [project])
        self.closed = False
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={'poll_interval': .01}, daemon=True)
        self.thread.start(); self.addCleanup(self.close_server)
        self.token = self.server.token
        project_id = next(iter(self.server.sessions.projects))
        self.assertFalse((self.root / 'state/ai-system').exists())
        status, bootstrap, _ = self.request('/api/bootstrap')
        self.assertEqual(200, status, bootstrap)
        self.assertEqual(self.REASON, bootstrap['runtime']['system_unavailable'])
        self.assertEqual(self.REASON, bootstrap['runtime']['discovery_sandbox'])
        # Reads keep the generic answer; the page names the reason from the bootstrap instead.
        for path in ('/api/system-runs?project_id=' + project_id, '/api/system-runs/' + 'a' * 32,
                     '/api/system-discoveries/' + 'a' * 32):
            self.assertEqual(400, self.request(path)[0])
        body = {'project_id': project_id, 'config_path': 'system.json'}
        for status, result, _ in (self.post('/api/systems/catalog', body),
                                  self.post('/api/systems/editor', body),
                                  self.post('/api/system-runs', {**body, 'task': 'Check', 'change_id': 'change-001'}),
                                  self.post('/api/system-discoveries', {'editor': {}, 'provider': 'codex'})):
            self.assertEqual((400, self.REASON), (status, result.get('error')))


if __name__ == '__main__':
    unittest.main()
