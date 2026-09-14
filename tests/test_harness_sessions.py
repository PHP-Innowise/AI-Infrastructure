"""Offline session lifecycle checks using a real, disposable provider process."""
import contextlib
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness" / "src"))
from harness import sessions


FAKE_CLI = r'''
import json, os, pathlib, sys, time
config = json.loads(sys.argv[1])
pathlib.Path(config["pid_path"]).write_text(str(os.getpid()))
pathlib.Path(config["pid_path"] + ".cwd").write_text(str(pathlib.Path.cwd()))
behavior = config["behavior"]
if behavior == "no_stdin":
    time.sleep(30)
sys.stdin.read()
for line in config.get('prompt', '').splitlines():
    if line.startswith('[{"name":') and '"path":' in line:
        import base64
        files = json.loads(line)
        pathlib.Path(config['pid_path'] + '.attachments').write_text(json.dumps([
            base64.b64encode(pathlib.Path(item['path']).read_bytes()).decode() for item in files]))
if config["provider"] == "claude":
    print(json.dumps({"type": "system", "subtype": "init", "session_id": "native-original"}), flush=True)
    print(json.dumps({"type": "assistant", "message": {"role": "assistant", "content": [
        {"type": "text", "text": "Fixture answer"}]}}), flush=True)
    print(json.dumps({"type": "result", "subtype": "success", "is_error": False,
                      "session_id": "native-original", "result": "Fixture answer"}), flush=True)
    sys.exit(0)
print(json.dumps({"type": "thread.started", "thread_id":
                  "native-other" if behavior == "changed_id" else "native-original"}), flush=True)
if behavior == "with_helper":
    print(json.dumps({"type":"item.completed", "item":{"type":"collab_tool_call",
        "tool":"spawn_agent", "status":"completed", "receiver_thread_ids":["fixture-child"]}}), flush=True)
if behavior == "with_helper_v2":
    for phase in ("started", "completed"):
        print(json.dumps({"type":"item.completed", "item":{"type":"sub_agent_activity",
            "id":"fixture-activity-" + phase, "kind":phase,
            "agent_thread_id":"fixture-child", "agent_path":"/root/reviewer"}}), flush=True)
if behavior == "with_helper_journal":
    import sqlite3, datetime
    home = pathlib.Path(os.environ['CODEX_HOME']); folder = home / 'sessions'; folder.mkdir(parents=True, exist_ok=True)
    rollout = folder / 'rollout-native-original.jsonl'
    timestamp = datetime.datetime.now(datetime.timezone.utc).isoformat()
    records = [{'type':'session_meta','payload':{'id':'native-original','cwd':str(pathlib.Path.cwd()),'source':'exec'}},
        {'type':'event_msg','timestamp':timestamp,'payload':{'type':'task_started','turn_id':'fixture-turn'}},
        {'type':'event_msg','timestamp':timestamp,'payload':{'type':'item_completed','thread_id':'native-original',
            'turn_id':'fixture-turn','item':{'type':'SubAgentActivity','id':'activity','kind':'started',
            'agent_thread_id':'fixture-child','agent_path':'/root/reviewer'}}}]
    rollout.write_text(''.join(json.dumps(record)+'\n' for record in records))
    with sqlite3.connect(home/'state_5.sqlite') as db:
        db.execute('CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT)')
        db.execute('INSERT OR REPLACE INTO threads VALUES (?,?,?)',('native-original',str(rollout),str(pathlib.Path.cwd())))
if behavior == "double_id":
    print(json.dumps({"type": "thread.started", "thread_id": "native-other"}), flush=True)
if behavior == "sleep":
    time.sleep(30)
if behavior == "oversize":
    sys.stdout.write("x" * (2 * 1024 * 1024 + 1))
    sys.stdout.flush()
    time.sleep(30)
print(json.dumps({"type": "item.completed", "item":
                  {"type": "agent_message", "text": "Fixture answer"}}), flush=True)
if behavior != "no_terminal":
    result = {"type": "turn.failed", "error": {"message": "Fixture failure"}} if behavior == "failed" else {"type": "turn.completed"}
    sys.stdout.write(json.dumps(result) + ("" if behavior == "no_newline" else "\n"))
    sys.stdout.flush()
sys.exit(7 if behavior == "exit_error" else 0)
'''


@unittest.skipUnless(hasattr(os, "killpg"), "Session workers require POSIX process groups")
class SessionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        codex_home = patch.dict(os.environ, {'CODEX_HOME':str(self.root / 'codex-home')})
        codex_home.start(); self.addCleanup(codex_home.stop)
        self.project = self.root / "project"
        self.project.mkdir()
        self.fake = self.root / "fake_provider.py"
        self.fake.write_text(FAKE_CLI)
        self.calls = []
        self.managers = []
        discovery = patch.object(sessions.providers, "discover_providers", return_value=[
            {"id": "codex", "available": True, "executable": str(self.fake)},
            {"id": "claude", "available": True, "executable": str(self.fake)},
            {"id": "cursor", "available": False, "executable": None},
        ])
        discovery.start()
        self.addCleanup(discovery.stop)
        codex_catalog = {
            "models": [
                {"id": "fixture-model", "label": "Fixture", "efforts": ["low", "high"]},
                {"id": "fixture-small", "label": "Small fixture", "efforts": ["low"]},
            ],
            "efforts": ["low", "high"], "detail": "Offline fixture catalog",
        }
        claude_catalog = {
            "models": [{"id": "fixture-claude", "label": "Claude fixture",
                        "efforts": ["high", "ultracode"]}],
            "efforts": ["high", "ultracode"], "detail": "Offline Claude fixture catalog",
        }
        catalog = patch.object(sessions.providers, "model_options", side_effect=lambda provider:
                               claude_catalog if provider == "claude" else codex_catalog)
        catalog.start()
        self.addCleanup(catalog.stop)
        builder = patch.object(sessions.providers, "build_command", side_effect=self.build_command)
        builder.start()
        self.addCleanup(builder.stop)
        self.addCleanup(self.close_managers)

    def close_managers(self):
        for manager in self.managers:
            manager.close()

    def build_command(self, provider, executable, project, prompt, **options):
        call = dict(provider=provider, executable=executable, project=str(project),
                    prompt=prompt, **options)
        call["behavior"] = prompt.splitlines()[0]
        call["pid_path"] = str(self.root / f"process-{len(self.calls)}.pid")
        self.calls.append(call)
        return [sys.executable, "-u", str(self.fake), json.dumps(call)]

    def manager(self, timeout=5, state=None, projects=None):
        manager = sessions.Sessions(state or self.root / "state", projects or [self.project], timeout=timeout)
        self.managers.append(manager)
        return manager

    def create(self, manager, prompt="complete", **options):
        return manager.create(dict(project_id=next(iter(manager.projects)), provider="codex",
                                   prompt=prompt, **options))["id"]

    def wait_for(self, predicate, timeout=6):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(.01)
        self.fail("Timed out waiting for the fixture process/session")

    def settled(self, manager, sid):
        self.wait_for(lambda: manager.get(sid)["status"] not in sessions.ACTIVE)
        return manager.get(sid)

    def assert_process_gone(self, call):
        pid_path = Path(call["pid_path"])
        self.wait_for(pid_path.is_file)
        pid = int(pid_path.read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_sdd_phases_validate_documents_resume_and_retain_launch_settings(self):
        from harness import sdd
        manager = self.manager()
        for value in (None, {}, {'phase': [], 'feature': 'login'},
                      {'phase': 'specify', 'feature': '../escape'},
                      {'phase': 'specify', 'feature': 'memory'}):
            with self.assertRaises(sessions.SessionError):
                self.create(manager, workflow='sdd', sdd=value)
        with self.assertRaises(sessions.SessionError):
            self.create(manager, sdd={'phase': 'specify', 'feature': 'login'})
        settings = {'phase': 'specify', 'feature': 'login'}
        sid = self.create(manager, workflow='sdd', sdd=settings)
        self.assertEqual(self.settled(manager, sid)['mode'], 'edit')
        self.assertIn('Harness SDD phase: specify', self.calls[-1]['prompt'])
        self.assertIn('Additional agents are disabled', self.calls[-1]['prompt'])
        folder = self.project / 'specs/login'; folder.mkdir(parents=True)
        for phase, required in (('plan', 'spec.md'), ('tasks', 'plan.md'), ('implement', 'tasks.md')):
            options = {'sdd': {**settings, 'phase': phase}}
            before = len(self.calls)
            with self.assertRaisesRegex(sessions.SessionError, required):
                manager.send(sid, 'Continue', options)
            self.assertEqual(len(self.calls), before)
            (folder / required).write_text('# Reviewed document\n- [ ] Acceptance criterion\n')
            manager.send(sid, 'Continue', options)
            self.assertEqual(self.settled(manager, sid)['sdd']['phase'], phase)
        manager.send(sid, 'Review', {'sdd': {**settings, 'phase': 'review'}})
        self.assertEqual(self.settled(manager, sid)['mode'], 'plan')
        self.assertEqual(self.calls[-1]['session_id'], 'native-original')
        self.assertEqual(self.calls[-1]['mode'], 'plan')
        self.assertEqual(manager.get(sid)['project_path'], str(self.project))
        launches = manager.results.history(sid)['launches']
        self.assertEqual({run['settings']['sdd']['phase'] for run in launches},
                         {'specify', 'plan', 'tasks', 'implement', 'review'})
        with self.assertRaisesRegex(sessions.SessionError, 'stays fixed'):
            manager.send(sid, 'Switch', {'sdd': {'phase': 'specify', 'feature': 'other'}})
        (folder / 'spec.md').write_text('[NEEDS CLARIFICATION: roles]')
        with self.assertRaisesRegex(sessions.SessionError, 'CLARIFICATION'):
            manager.send(sid, 'Plan', {'sdd': {**settings, 'phase': 'plan'}})
        (folder / 'spec.md').unlink()
        secret = self.root / 'outside.md'; secret.write_text('OUTSIDE_SENTINEL')
        (folder / 'spec.md').symlink_to(secret)
        self.assertNotIn('OUTSIDE_SENTINEL', json.dumps(sdd.artifacts(self.project, settings)))
        with self.assertRaises(sessions.SessionError):
            sdd.check(self.project, {**settings, 'phase': 'plan'})
        manager.close(); self.managers.remove(manager)
        reopened = self.manager()
        self.assertEqual(reopened.get(sid)['sdd'], {**settings, 'phase': 'review'})

    def test_sdd_checks_actual_worktree_before_launching_provider(self):
        for args in (['init', '-b', 'main'], ['-c', 'user.name=Fixture', '-c',
                     'user.email=fixture@example.invalid', 'commit', '--allow-empty', '-m', 'Base']):
            subprocess.run(['git', '-C', str(self.project), *args], check=True, capture_output=True)
        folder = self.project / 'specs/login'; folder.mkdir(parents=True)
        (folder / 'spec.md').write_text('Uncommitted specification: not copied to worktree.')
        manager = self.manager()
        sid = self.create(manager, workflow='sdd', workspace='worktree',
                          sdd={'phase':'plan', 'feature':'login'})
        self.assertEqual(self.settled(manager, sid)['status'], 'failed')
        self.assertEqual(self.calls, [])
        self.assertTrue(any('specs/login/sdd-spec.md' in event.get('text', '') for event in manager.events(sid)))
        self.assertEqual((folder / 'spec.md').read_text(), 'Uncommitted specification: not copied to worktree.')

    def test_optional_model_routing_selects_phase_not_document_write_permissions(self):
        manager = self.manager()
        routing = {'plan': {'model':'fixture-model', 'thinking_effort':'high'},
                   'edit': {'model':'fixture-small', 'thinking_effort':'low'}}
        sid = self.create(manager, workflow='sdd', sdd={'phase':'specify','feature':'login'},
                          model_routing=routing, model='ignored-common-model')
        self.settled(manager, sid)
        self.assertEqual((self.calls[-1]['model'], self.calls[-1]['thinking_effort'], self.calls[-1]['mode']),
                         ('fixture-model', 'high', 'edit'))
        folder = self.project / 'specs/login'; folder.mkdir(parents=True)
        for name in ('spec.md','plan.md','tasks.md'):
            (folder/name).write_text('# Reviewed input')
        for phase, model, effort, mode in (('plan','fixture-model','high','edit'),
                ('implement','fixture-small','low','edit'), ('review','fixture-model','high','plan')):
            manager.send(sid, 'Continue', {'sdd':{'phase':phase,'feature':'login'}})
            self.settled(manager, sid)
            self.assertEqual((self.calls[-1]['model'],self.calls[-1]['thinking_effort'],self.calls[-1]['mode']), (model,effort,mode))
            self.assertEqual(self.calls[-1]['session_id'], 'native-original')
        snapshot = manager.results.history(sid)['launches'][0]['settings']
        self.assertEqual(snapshot['model_routing'], routing)
        manager.send(sid, 'Manual selection', {'model_routing':None,'model':'fixture-small','thinking_effort':'low'})
        self.assertIsNone(self.settled(manager, sid)['model_routing'])
        self.assertEqual(self.calls[-1]['model'], 'fixture-small')
        # Ordinary Workspace sessions can switch the mode explicitly while routing is enabled.
        ordinary = self.create(manager, model_routing=routing)
        self.settled(manager, ordinary)
        manager.send(ordinary, 'Edit now', {'mode':'edit'})
        self.assertEqual(self.settled(manager, ordinary)['model'], 'fixture-small')
        manager.close(); self.managers.remove(manager)
        reopened = self.manager()
        self.assertEqual(reopened.get(ordinary)['model_routing'], routing)
        self.assertEqual(reopened.results.history(sid)['launches'][0]['settings'], snapshot)
        # Validate both roles, including the currently unused one, before launching anything.
        for invalid in ({}, {'plan':routing['plan']}, {'plan':routing['plan'],'edit':None},
                        {**routing,'edit':{'model':'fixture-small','thinking_effort':'high'}},
                        {**routing,'edit':{'model':'-invalid','thinking_effort':None}}):
            with self.assertRaises(sessions.SessionError):
                self.create(reopened, model_routing=invalid)
        with self.assertRaises(sessions.SessionError):
            self.create(reopened, workflow='fleet-review', model_routing=routing)
        plain = self.create(reopened, model='fixture-model', thinking_effort='low')
        self.assertIsNone(self.settled(reopened, plain)['model_routing'])
        self.assertEqual(self.calls[-1]['model'],'fixture-model')
        with self.assertRaises(sessions.SessionError):
            reopened.send(plain, 'Edit', {'mode':'edit'})

    def test_budget_validation_persistence_and_stale_or_active_edits(self):
        for invalid in ({'usd':float('nan')},{'usd':True},{'usd':0},{'tokens':1.5},
                        {'tokens':True},{'tokens':0},{'seconds':86401},{'seconds':False},{'extra':1}):
            with self.subTest(invalid=invalid), self.assertRaises(sessions.SessionError):
                sessions.validate_budgets(invalid,'claude')
        with self.assertRaises(sessions.SessionError): sessions.validate_budgets({'usd':1},'codex')
        manager=self.manager(); sid=self.create(manager)
        original=self.settled(manager,sid); usage=original['budget_usage']
        self.assertIsNone(usage['tokens']); self.assertIsNone(usage['cost_usd'])
        options={'budgets':{'usd':None,'tokens':500,'seconds':2},'revision':0}
        changed=manager.set_budgets(sid,options)
        self.assertEqual(changed['budgets'],options['budgets']); self.assertEqual(changed['budget_usage'],usage)
        self.assertEqual(changed['native_session_id'],original['native_session_id'])
        with self.assertRaises(sessions.SessionError): manager.set_budgets(sid,options)
        manager.close(); self.managers.remove(manager); restarted=self.manager()
        self.assertEqual(restarted.get(sid)['budgets'],options['budgets'])
        restarted._status(sid,'queued')
        with self.assertRaises(sessions.SessionError): restarted.set_budgets(sid,{**options,'revision':1})

    def test_agent_budget_plan_accounts_for_main_agent_reviewers_and_shared_time(self):
        native={'agents_enabled':True,'agent_count':3,'fleet':None,
                'budgets':{'usd':1,'tokens':101,'seconds':120}}
        plan=sessions.agent_budget_plan(native)
        self.assertEqual((plan['agents'],plan['concurrent'],plan['waves']),(4,4,1))
        self.assertEqual((plan['usd'],plan['tokens'],plan['unallocated_tokens'],plan['seconds']),(.25,25,1,120))
        solo=sessions.agent_budget_plan({**native,'agents_enabled':False})
        self.assertEqual((solo['agents'],solo['usd'],solo['tokens']),(1,1,101))
        fleet={**native,'agent_count':2,'fleet':{'lenses':['a','b','c','d','e'],'worker_timeout':90}}
        plan=sessions.agent_budget_plan(fleet)
        self.assertEqual((plan['agents'],plan['concurrent'],plan['waves'],plan['seconds']),(5,2,3,90))
        self.assertEqual(plan['tokens'],20); self.assertEqual(plan['shared_seconds'],120)
        limited=sessions.agent_budget_plan({**fleet,'budgets':{'usd':.01,'tokens':3,'seconds':60}})
        self.assertEqual((limited['usd'],limited['tokens'],limited['unallocated_tokens'],limited['seconds']),(.002,0,3,60))
        blank=sessions.agent_budget_plan({**native,'budgets':sessions.DEFAULT_BUDGETS})
        self.assertIsNone(blank['usd']); self.assertIsNone(blank['tokens']); self.assertIsNone(blank['seconds']); self.assertIsNone(blank['shared_seconds'])
        fleet_blank=sessions.agent_budget_plan({**fleet,'budgets':sessions.DEFAULT_BUDGETS})
        self.assertEqual(fleet_blank['seconds'],90); self.assertIsNone(fleet_blank['shared_seconds'])
        rounded=sessions.agent_budget_plan({**native,'agent_count':2,'budgets':{'usd':.01,'tokens':1,'seconds':None}})
        self.assertEqual(rounded['usd'],.003333); self.assertLessEqual(rounded['usd']*3,.01)
        manager=self.manager(); sid=self.create(manager,agents_enabled=True,agent_count=3,
            budgets={'tokens':100,'seconds':10})
        session=self.settled(manager,sid)
        self.assertEqual(session['agent_budget_plan']['tokens'],25)
        self.assertIn('planning shares across 4',self.calls[-1]['prompt'])
        launch=manager.results.history(sid)['launches'][0]
        self.assertEqual(launch['settings']['agent_budget_plan'],session['agent_budget_plan'])
        manager.set_budgets(sid,{'budgets':{'tokens':200,'seconds':20},'revision':0})
        self.assertEqual(manager.get(sid)['agent_budget_plan']['tokens'],50)
        self.assertEqual(manager.results.history(sid)['launches'][0]['settings']['agent_budget_plan']['tokens'],25)

    def test_reported_token_threshold_and_native_money_cap(self):
        self.fake.write_text(FAKE_CLI.replace('"session_id": "native-original", "result": "Fixture answer"',
            '"session_id": "native-original", "result": "Fixture answer", "total_cost_usd": 0.02, "usage": {"input_tokens": 3, "output_tokens": 2, "cache_read_input_tokens": 10, "cache_creation_input_tokens": 5}'))
        manager=self.manager(); sid=manager.create({'project_id':next(iter(manager.projects)),
            'provider':'claude','prompt':'complete','project_context':False,
            'budgets':{'usd':.5,'tokens':19,'seconds':2}})['id']
        result=self.settled(manager,sid)
        self.assertEqual(result['status'],'failed'); self.assertEqual(result['budget_usage']['tokens'],20)
        self.assertEqual(result['budget_usage']['cost_usd'],.02)
        self.assertEqual(result['budget_usage']['limit_reached'],'token')
        self.assertEqual(self.calls[0]['budget_usd'],.5)
        self.assertTrue(any('token limit' in event.get('text','') for event in manager.events(sid)))
        self.assert_process_gone(self.calls[0])

    def test_explicit_time_budget_terminates_process_and_can_be_increased(self):
        manager=self.manager(timeout=10); sid=self.create(manager,prompt='sleep',project_context=False,budgets={'seconds':1})
        result=self.settled(manager,sid)
        self.assertEqual(result['status'],'failed'); self.assertEqual(result['budget_usage']['limit_reached'],'time')
        self.assertLess(result['budget_usage']['seconds'],5); self.assert_process_gone(self.calls[0])
        manager.set_budgets(sid,{'budgets':{'seconds':4},'revision':0})
        manager.send(sid,'complete'); result=self.settled(manager,sid)
        self.assertEqual(result['status'],'completed'); self.assertIsNone(result['budget_usage']['limit_reached'])

    def test_empty_time_budget_outlives_server_default_and_can_be_cleared(self):
        self.fake.write_text(FAKE_CLI.replace('if behavior == "sleep":',
            'if behavior == "brief_wait":\n    time.sleep(1.4)\nif behavior == "sleep":'))
        manager = self.manager(timeout=1)
        sid = self.create(manager, 'brief_wait', budgets=dict(sessions.DEFAULT_BUDGETS))
        result = self.settled(manager, sid)
        self.assertEqual(result['status'], 'completed')
        self.assertGreater(result['budget_usage']['seconds'], 1)
        self.assertIsNone(result['budget_usage']['limit_reached'])
        self.assertIsNone(result['agent_budget_plan']['shared_seconds'])
        self.assertTrue(any('time uncapped' in e.get('text','') for e in manager.events(sid)))
        manager.set_budgets(sid, {'budgets':{'seconds':1},'revision':0})
        manager.send(sid, 'brief_wait')
        self.assertEqual(self.settled(manager, sid)['budget_usage']['limit_reached'], 'time')
        self.assertTrue(any('time limit (1s)' in e.get('text','') for e in manager.events(sid)))
        manager.set_budgets(sid, {'budgets':dict(sessions.DEFAULT_BUDGETS),'revision':1})
        manager.close(); self.managers.remove(manager)
        reopened = self.manager(timeout=1)
        self.assertIsNone(reopened.get(sid)['budgets']['seconds'])
        reopened.send(sid, 'brief_wait')
        self.assertEqual(self.settled(reopened, sid)['status'], 'completed')
        default_sid = self.create(reopened)
        self.assertEqual(self.settled(reopened,default_sid)['budgets']['seconds'],1)

    def git_command(self, *arguments, cwd=None):
        environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        environment.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                            "GIT_AUTHOR_NAME": "Harness fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
                            "GIT_COMMITTER_NAME": "Harness fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
                            "GIT_TERMINAL_PROMPT": "0"})
        return subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgSign=false",
                               "-C", str(cwd or self.project), *arguments], env=environment,
                              stdin=subprocess.DEVNULL, capture_output=True, text=True, check=True).stdout.strip()

    def initialize_git(self, cwd=None, commit=True):
        project = cwd or self.project
        project.mkdir(parents=True, exist_ok=True)
        self.git_command("init", "--quiet", cwd=project)
        self.git_command("symbolic-ref", "HEAD", "refs/heads/main", cwd=project)
        if commit:
            (project / "AGENTS.md").write_text("COMMITTED WORKTREE CONTEXT\n", encoding="utf-8")
            (project / "tracked.txt").write_text("Committed fixture\n", encoding="utf-8")
            self.git_command("add", "AGENTS.md", "tracked.txt", cwd=project)
            self.git_command("commit", "--quiet", "-m", "Initialize fixture", cwd=project)

    @unittest.skipUnless(shutil.which("git"), "Git workspace tests require git")
    def test_attached_files_reach_native_worktree_process_and_survive_restart(self):
        self.initialize_git()
        before = self.git_command('status','--porcelain')
        manager = self.manager()
        content = b'Fixture requirements\n'
        sid = self.create(manager, workspace='worktree', attachments=[{
            'name':'CLAUDE.md', 'data':base64.b64encode(content).decode()}])
        self.assertEqual(self.settled(manager,sid)['status'], 'completed')
        call = self.calls[-1]
        self.assertEqual(json.loads(Path(call['pid_path']+'.attachments').read_text()), [base64.b64encode(content).decode()])
        item, body, path = manager.attachments.current(sid)[0]
        self.assertEqual(body, content)
        self.assertEqual(Path(path).name, 'upload-CLAUDE.md')
        self.assertFalse(Path(path).is_relative_to(Path(call['project'])))
        self.assertEqual(self.git_command('status','--porcelain'), before)
        manager.close()
        self.managers.remove(manager)
        manager = self.manager()
        self.assertEqual(manager.attachments.read(sid,item['id'])[1], content)
        manager.send(sid,'success',{'attachments':[{'name':'revised.txt','data':'cmV2aXNlZA=='}]})
        self.assertEqual(self.settled(manager,sid)['status'], 'completed')
        self.assertEqual(json.loads(Path(self.calls[-1]['pid_path']+'.attachments').read_text()), ['cmV2aXNlZA=='])

    @unittest.skipUnless(shutil.which("git"), "Git workspace tests require git")
    def test_worktree_uses_committed_context_and_keeps_dirty_primary_unchanged_across_resume(self):
        self.initialize_git()
        head = self.git_command("rev-parse", "HEAD")
        (self.project / "AGENTS.md").write_text("DIRTY PRIMARY CONTEXT\n", encoding="utf-8")
        (self.project / "untracked.txt").write_text("PRIVATE UNCOMMITTED FIXTURE\n", encoding="utf-8")
        primary_status = self.git_command("status", "--porcelain")
        manager = self.manager()
        sid = self.create(manager, workspace="worktree", worktree_branch="codex/fixture-session",
                          project_context=True)
        original = self.settled(manager, sid)
        self.assertEqual(original["status"], "completed")
        worktree = manager.state_dir / "worktrees" / sid
        self.assertEqual((original["workspace"], original["branch"], original["project_path"]),
                         ("worktree", "codex/fixture-session", str(worktree)))
        self.assertEqual(Path(original["git_common_dir"]), self.project / ".git")
        self.assertEqual(self.git_command("rev-parse", "HEAD", cwd=worktree), head)
        self.assertEqual(self.git_command("branch", "--show-current", cwd=worktree), original["branch"])
        self.assertEqual((worktree / "AGENTS.md").read_text(), "COMMITTED WORKTREE CONTEXT\n")
        self.assertFalse((worktree / "untracked.txt").exists())
        self.assertEqual(self.calls[-1]["project"], str(worktree))
        self.assertEqual(Path(self.calls[-1]["pid_path"] + ".cwd").read_text(), str(worktree))
        self.assertIn("COMMITTED WORKTREE CONTEXT", self.calls[-1]["prompt"])
        self.assertNotIn("DIRTY PRIMARY CONTEXT", self.calls[-1]["prompt"])
        self.assertEqual(self.git_command("status", "--porcelain"), primary_status)
        self.assertEqual(self.git_command("branch", "--show-current"), "main")
        self.assertEqual((self.project / "AGENTS.md").read_text(), "DIRTY PRIMARY CONTEXT\n")

        manager.close()
        self.managers.remove(manager)
        restarted = self.manager()
        self.assertEqual(restarted.get(sid), original)
        restarted.send(sid, "complete in the same worktree")
        resumed = self.settled(restarted, sid)
        self.assertEqual(resumed["status"], "completed")
        for field in ("project_id", "project_path", "workspace", "branch", "git_common_dir", "native_session_id"):
            self.assertEqual(resumed[field], original[field])
        self.assertEqual(self.calls[-1]["project"], str(worktree))
        self.assertEqual(self.calls[-1]["session_id"], "native-original")
        self.assertEqual(Path(self.calls[-1]["pid_path"] + ".cwd").read_text(), str(worktree))
        self.assertEqual(self.git_command("status", "--porcelain"), primary_status)

    @unittest.skipUnless(shutil.which("git"), "Git workspace tests require git")
    def test_git_metadata_handles_plain_unborn_dirty_and_detached_projects(self):
        with patch.object(sessions.Sessions, "_worker", return_value=None):
            manager = self.manager()
            key = next(iter(manager.projects))
            plain = manager.git(key)
            self.assertEqual(plain["project_id"], key)
            self.assertFalse(plain["is_git"])
            self.assertFalse(plain["worktree_available"])
            with self.assertRaises(sessions.SessionError):
                self.create(manager, workspace="worktree")
            self.initialize_git(commit=False)
            unborn = manager.git(key)
            self.assertTrue(unborn["is_git"])
            self.assertFalse(unborn["worktree_available"])
            self.assertFalse(unborn["head"])
            with self.assertRaises(sessions.SessionError):
                self.create(manager, workspace="worktree")
            self.assertEqual(manager.list(), [])
            self.assertEqual(manager.jobs.qsize(), 0)
            self.initialize_git()
            clean = manager.git(key)
            self.assertEqual(clean["branch"], "main")
            self.assertEqual(clean["head"], self.git_command("rev-parse", "HEAD"))
            self.assertFalse(clean["dirty"])
            self.assertTrue(clean["worktree_available"])
            self.git_command("checkout", "--quiet", "-b", "feature-metadata")
            (self.project / "untracked.txt").write_text("Uncommitted\n")
            changed = manager.git(key)
            self.assertEqual(changed["branch"], "feature-metadata")
            self.assertTrue(changed["dirty"])
            self.git_command("checkout", "--quiet", "--detach", "HEAD")
            detached = manager.git(key)
            self.assertTrue(detached["worktree_available"])
            self.assertEqual(detached["head"], clean["head"])
            sid = self.create(manager, workspace="worktree", worktree_branch="")
            created = manager.get(sid)
            self.assertTrue(created["branch"].startswith("codex/harness-"))
            self.assertEqual(self.git_command("branch", "--show-current", cwd=Path(created["project_path"])),
                             created["branch"])
            self.assertEqual(self.git_command("rev-parse", "HEAD", cwd=Path(created["project_path"])), clean["head"])
            with patch.object(sessions, "run_git", side_effect=sessions.SessionError("Git unavailable")):
                ordinary = manager.get(self.create(manager))
            self.assertEqual((ordinary["workspace"], ordinary["project_path"], ordinary["branch"]),
                             ("project", str(self.project), None))

    @unittest.skipUnless(shutil.which("git"), "Git workspace tests require git")
    def test_invalid_worktree_options_do_not_create_branches_sessions_or_workspaces(self):
        self.initialize_git()
        manager = self.manager()
        refs_before = self.git_command("show-ref")
        worktrees_before = self.git_command("worktree", "list", "--porcelain")
        invalid = [{"workspace": value} for value in (None, False, [], {}, "unknown")]
        invalid += [{"workspace": "worktree", "worktree_branch": value}
                    for value in (None, [], {}, False, 0, "main", "../escape", "--unsafe", "bad branch",
                                  "bad..name", "bad\0branch", "x" * 257)]
        invalid += [{"workspace": "project", "worktree_branch": "new-branch"}]
        for options in invalid:
            with self.subTest(options=options), self.assertRaises(sessions.SessionError):
                self.create(manager, **options)
        self.assertEqual(manager.list(), [])
        self.assertEqual(manager.jobs.qsize(), 0)
        self.assertEqual(self.calls, [])
        self.assertEqual(self.git_command("show-ref"), refs_before)
        self.assertEqual(self.git_command("worktree", "list", "--porcelain"), worktrees_before)
        self.assertFalse(any((manager.state_dir / "worktrees").glob("*")))

    @unittest.skipUnless(shutil.which("git"), "Git workspace tests require git")
    def test_worktree_resume_rejects_symlink_missing_or_unrelated_directory_without_enqueuing(self):
        self.initialize_git()
        manager = self.manager()
        sid = self.create(manager, workspace="worktree")
        self.assertEqual(self.settled(manager, sid)["status"], "completed")
        self.wait_for(lambda: manager.jobs.unfinished_tasks == 0)
        worktree = Path(manager.get(sid)["project_path"])

        def assert_rejected():
            before = (manager.get(sid), manager.events(sid), manager.jobs.qsize(),
                      dict(manager.generations), len(self.calls))
            with self.assertRaises(sessions.SessionError):
                manager.send(sid, "Do not resume a replaced workspace", {"model": "custom-after-replacement"})
            self.assertEqual((manager.get(sid), manager.events(sid), manager.jobs.qsize(),
                              dict(manager.generations), len(self.calls)), before)

        moved = self.root / "moved-worktree"
        worktree.rename(moved)
        worktree.symlink_to(moved, target_is_directory=True)
        assert_rejected()
        worktree.unlink()
        moved.rename(worktree)
        self.git_command("worktree", "remove", "--force", str(worktree))
        assert_rejected()
        self.initialize_git(cwd=worktree)
        assert_rejected()

    @unittest.skipUnless(shutil.which("git"), "Git workspace tests require git")
    def test_nested_project_worktree_keeps_prefix_and_requires_a_committed_directory(self):
        self.initialize_git()
        nested = self.project / "packages/app"
        nested.mkdir(parents=True)
        (nested / "AGENTS.md").write_text("NESTED COMMITTED CONTEXT\n", encoding="utf-8")
        self.git_command("add", "packages/app/AGENTS.md")
        self.git_command("commit", "--quiet", "-m", "Add nested project")
        (nested / "AGENTS.md").write_text("NESTED DIRTY CONTEXT\n", encoding="utf-8")
        manager = self.manager(projects=[nested])
        sid = self.create(manager, workspace="worktree", project_context=True)
        session = self.settled(manager, sid)
        expected = manager.state_dir / "worktrees" / sid / "packages/app"
        self.assertEqual((session["status"], session["project_path"]), ("completed", str(expected)))
        self.assertEqual(self.calls[-1]["project"], str(expected))
        self.assertEqual(Path(self.calls[-1]["pid_path"] + ".cwd").read_text(), str(expected))
        self.assertIn("NESTED COMMITTED CONTEXT", self.calls[-1]["prompt"])
        self.assertNotIn("NESTED DIRTY CONTEXT", self.calls[-1]["prompt"])
        self.assertEqual((nested / "AGENTS.md").read_text(), "NESTED DIRTY CONTEXT\n")
        manager.send(sid, "complete nested followup")
        self.assertEqual(self.settled(manager, sid)["project_path"], str(expected))

        absent = self.project / "uncommitted-project"
        absent.mkdir()
        absent_manager = self.manager(state=self.root / "absent-state", projects=[absent])
        metadata = absent_manager.git(next(iter(absent_manager.projects)))
        self.assertFalse(metadata["worktree_available"])
        before = self.git_command("show-ref"), self.git_command("worktree", "list", "--porcelain")
        with self.assertRaises(sessions.SessionError):
            self.create(absent_manager, workspace="worktree")
        self.assertEqual(absent_manager.list(), [])
        self.assertEqual((self.git_command("show-ref"), self.git_command("worktree", "list", "--porcelain")), before)
        self.assertFalse(any((absent_manager.state_dir / "worktrees").glob("*")))

    def test_completion_requires_successful_terminal_and_zero_exit(self):
        manager = self.manager()
        for behavior, expected in (("complete", "completed"), ("no_newline", "completed"),
                                   ("no_terminal", "failed"), ("exit_error", "failed"),
                                   ("failed", "failed")):
            with self.subTest(behavior=behavior):
                sid = self.create(manager, behavior)
                self.assertEqual(self.settled(manager, sid)["status"], expected)
                events = manager.events(sid)
                self.assertTrue(any(event.get("text") == "Fixture answer" for event in events))
                self.assertEqual(manager.events(sid, events[0]["id"]), events[1:])
                self.assert_process_gone(self.calls[-1])

    def test_cancel_and_timeout_terminate_even_when_child_does_not_read_stdin(self):
        for behavior, cancel in (("sleep", True), ("no_stdin", False)):
            with self.subTest(behavior=behavior):
                manager = self.manager(timeout=1 if not cancel else 5,
                                       state=self.root / behavior)
                prompt = behavior + "\n" + "x" * 31000
                call_index = len(self.calls)
                sid = self.create(manager, prompt, **({'budgets':dict(sessions.DEFAULT_BUDGETS)} if cancel else {}))
                self.wait_for(lambda: len(self.calls) > call_index
                              and Path(self.calls[call_index]["pid_path"]).is_file())
                call = self.calls[call_index]
                if cancel:
                    manager.cancel(sid)
                self.assertEqual(self.settled(manager, sid)["status"], "cancelled" if cancel else "failed")
                self.assert_process_gone(call)
                if not cancel:
                    self.assertTrue(any("time limit" in event.get("text", "")
                                        for event in manager.events(sid)))

    def test_cancel_between_spawn_and_process_registration_is_not_lost(self):
        manager = self.manager()
        spawned, release = threading.Event(), threading.Event()
        real_popen = subprocess.Popen
        child = []

        def spawn(*args, **kwargs):
            process = real_popen(*args, **kwargs)
            child.append(process)
            spawned.set()
            release.wait(3)
            return process

        with patch.object(sessions.subprocess, "Popen", side_effect=spawn):
            try:
                sid = self.create(manager, "sleep")
                self.assertTrue(spawned.wait(2))
                manager.cancel(sid)
            finally:
                release.set()
            self.wait_for(lambda: child and child[0].poll() is not None)
        self.assertEqual(self.settled(manager, sid)["status"], "cancelled")

    def test_resume_keeps_original_identity_and_rejects_changed_native_id(self):
        manager = self.manager()
        sid = self.create(manager, model="fixture-model")
        original = self.settled(manager, sid)
        self.assertEqual(original["native_session_id"], "native-original")
        manager.send(sid, "complete again")
        resumed = self.settled(manager, sid)
        for field in ("provider", "project_id", "project_path", "native_session_id", "model", "mode"):
            self.assertEqual(resumed[field], original[field])
        self.assertEqual(self.calls[-1]["provider"], "codex")
        self.assertEqual(self.calls[-1]["project"], str(self.project))
        self.assertEqual(self.calls[-1]["session_id"], "native-original")
        manager.send(sid, "changed_id")
        changed = self.settled(manager, sid)
        self.assertEqual(changed["status"], "failed")
        self.assertEqual(changed["native_session_id"], "native-original")
        conflicting_sid = self.create(manager, "double_id")
        conflicting = self.settled(manager, conflicting_sid)
        self.assertEqual(conflicting["status"], "failed")
        self.assertEqual(conflicting["native_session_id"], "native-original")
        manager.providers["codex"]["available"] = False
        with self.assertRaises(sessions.SessionError):
            manager.send(sid, "Do not switch providers")
        manager.providers["codex"]["available"] = True
        manager.projects[original["project_id"]]["path"] = str(self.root)
        with self.assertRaises(sessions.SessionError):
            manager.send(sid, "Do not switch workspaces")

    def test_queue_limit_and_restart_preserve_history_and_interrupt_active_runs(self):
        with patch.object(sessions.Sessions, "_worker", return_value=None):
            manager = self.manager()
            ids = [self.create(manager) for _ in range(16)]
            with self.assertRaises(sessions.SessionError):
                self.create(manager)
            self.assertEqual(len(manager.list()), 16)
            with self.assertRaises(sessions.SessionError):
                manager.send(ids[0], "Already queued")
            manager._status(ids[1], "running")
            # Simulate a crashed server: close storage without graceful status updates.
            manager.worker.join(1)
            manager.db.close()
            os.close(manager.runner_lock)  # A real crash closes the owner's descriptors.
            self.managers.remove(manager)
            restarted = self.manager()
            self.assertTrue(all(row["status"] == "interrupted" for row in restarted.list()))
            for sid in ids[:2]:
                events = restarted.events(sid)
                self.assertEqual(events[0]["text"], "complete")
                self.assertTrue(any("restarted" in event.get("text", "") for event in events))
                with self.assertRaises(sessions.SessionError):
                    restarted.send(sid, "No native session was created")

    def test_model_and_effort_changes_reach_native_resume_and_survive_restart(self):
        manager = self.manager()
        sid = self.create(manager, model="fixture-model", thinking_effort="high",
                          agents_enabled=True, agent_count=7)
        original = self.settled(manager, sid)
        self.assertEqual(original["status"], "completed")
        self.assertEqual((self.calls[-1]["model"], self.calls[-1]["thinking_effort"]),
                         ("fixture-model", "high"))
        for options in (None, {}):
            manager.send(sid, "complete unchanged", options)
            unchanged = self.settled(manager, sid)
            self.assertEqual((unchanged["model"], unchanged["thinking_effort"]),
                             ("fixture-model", "high"))
        manager.send(sid, "complete with different model", {
            "model": "fixture-small", "thinking_effort": "low",
        })
        changed = self.settled(manager, sid)
        self.assertEqual(changed["status"], "completed")
        self.assertEqual((changed["model"], changed["thinking_effort"]),
                         ("fixture-small", "low"))
        for field in ("id", "provider", "project_id", "project_path", "native_session_id",
                      "mode", "workflow", "agents_enabled", "agent_count"):
            self.assertEqual(changed[field], original[field])
        self.assertEqual((self.calls[-1]["model"], self.calls[-1]["thinking_effort"],
                          self.calls[-1]["session_id"]), ("fixture-small", "low", "native-original"))
        history = manager.events(sid)
        manager.close()
        self.managers.remove(manager)
        restarted = self.manager()
        self.assertEqual(restarted.get(sid), changed)
        self.assertEqual(restarted.events(sid), history)
        restarted.send(sid, "complete after restart")
        self.assertEqual(self.settled(restarted, sid)["status"], "completed")
        self.assertEqual((self.calls[-1]["model"], self.calls[-1]["thinking_effort"],
                          self.calls[-1]["session_id"]), ("fixture-small", "low", "native-original"))

    def test_explicit_defaults_reset_model_and_effort_independently(self):
        manager = self.manager()
        sid = self.create(manager, model="fixture-model", thinking_effort="high")
        self.assertEqual(self.settled(manager, sid)["status"], "completed")
        for default in (None, ""):
            with self.subTest(default=default):
                manager.send(sid, "complete reset effort", {"thinking_effort": default})
                reset_effort = self.settled(manager, sid)
                self.assertEqual(reset_effort["model"], "fixture-model")
                self.assertIsNone(reset_effort["thinking_effort"])
                manager.send(sid, "complete reset model", {"model": default})
                reset_model = self.settled(manager, sid)
                self.assertIsNone(reset_model["model"])
                self.assertIsNone(reset_model["thinking_effort"])
                self.assertEqual((self.calls[-1]["model"], self.calls[-1]["thinking_effort"]),
                                 (None, None))
                manager.send(sid, "complete restore explicit settings", {
                    "model": "fixture-model", "thinking_effort": "high",
                })
                self.assertEqual(self.settled(manager, sid)["status"], "completed")
        custom = "custom-" + "x" * 113
        custom_sid = self.create(manager, model=custom)
        custom_session = self.settled(manager, custom_sid)
        self.assertEqual(custom_session["status"], "completed")
        self.assertEqual(custom_session["model"], custom)
        self.assertIsNone(custom_session["thinking_effort"])
        self.assertEqual(self.calls[-1]["model"], custom)

    def test_rejected_model_updates_do_not_change_session_history_or_queue(self):
        with patch.object(sessions.Sessions, "_worker", return_value=None):
            manager = self.manager()
            sid = self.create(manager, model="fixture-model", thinking_effort="high")
            with manager.lock:
                manager.db.execute("UPDATE sessions SET native_session_id='fixture-native' WHERE id=?", (sid,))
                manager.db.commit()

            def assert_rejected(options):
                before = manager.get(sid), manager.events(sid), manager.jobs.qsize(), dict(manager.generations)
                with self.assertRaises(sessions.SessionError):
                    manager.send(sid, "Do not enqueue rejected changes", options)
                self.assertEqual((manager.get(sid), manager.events(sid), manager.jobs.qsize(),
                                  dict(manager.generations)), before)

            for status in ("queued", "running"):
                with self.subTest(status=status):
                    manager._status(sid, status)
                    assert_rejected({"model": "fixture-small", "thinking_effort": "low"})
            manager._status(sid, "completed")
            invalid = [[], "options", {"provider": "cursor"}, {"agent_count": 0},
                       {"model": "x" * 121}, {"model": []}, {"model": "fixture-small"},
                       {"thinking_effort": "unknown"}, {"thinking_effort": []},
                       {"thinking_effort": True}, {"model": "fixture-small", "thinking_effort": "high"}]
            for options in invalid:
                with self.subTest(options=options):
                    assert_rejected(options)

    def test_ultracode_requires_helpers_and_survives_allowed_followups(self):
        manager = self.manager()
        options = {"project_id": next(iter(manager.projects)), "provider": "claude",
                   "model": "fixture-claude", "thinking_effort": "ultracode",
                   "prompt": "complete ultracode", "agent_count": 7}
        for changes in ({"agents_enabled": False},
                        {"provider": "codex", "model": "fixture-model", "agents_enabled": True}):
            with self.subTest(rejected_creation=changes), self.assertRaises(sessions.SessionError):
                manager.create({**options, **changes})
            self.assertEqual(manager.list(), [])
            self.assertEqual(manager.jobs.qsize(), 0)
            self.assertEqual(manager.db.execute("SELECT COUNT(*) FROM events").fetchone()[0], 0)
            self.assertEqual(self.calls, [])
        sid = manager.create({**options, "agents_enabled": True})["id"]
        initial = self.settled(manager, sid)
        self.assertEqual(initial["status"], "completed")
        self.assertEqual(initial["thinking_effort"], "ultracode")
        self.assertEqual((self.calls[-1]["provider"], self.calls[-1]["thinking_effort"],
                          self.calls[-1]["agents_enabled"]), ("claude", "ultracode", True))
        for changes, expected in (({"thinking_effort": "high"}, "high"),
                                  ({"thinking_effort": "ultracode"}, "ultracode"),
                                  ({}, "ultracode")):
            manager.send(sid, "complete Claude follow-up", changes)
            resumed = self.settled(manager, sid)
            self.assertEqual(resumed["status"], "completed")
            self.assertEqual(resumed["thinking_effort"], expected)
            self.assertEqual((resumed["agents_enabled"], resumed["agent_count"],
                              resumed["native_session_id"]), (True, 7, "native-original"))
            self.assertEqual((self.calls[-1]["thinking_effort"], self.calls[-1]["session_id"]),
                             (expected, "native-original"))
        off_sid = manager.create({**options, "thinking_effort": "high", "agents_enabled": False})["id"]
        self.assertEqual(self.settled(manager, off_sid)["status"], "completed")
        self.wait_for(lambda: manager.jobs.unfinished_tasks == 0)
        before = (manager.get(off_sid), manager.events(off_sid), manager.jobs.qsize(),
                  dict(manager.generations), len(self.calls))
        for changes in ({"thinking_effort": "ultracode"},
                        {"thinking_effort": "ultracode", "agents_enabled": False}):
            with self.subTest(rejected_followup=changes), self.assertRaises(sessions.SessionError):
                manager.send(off_sid, "Do not enable Ultracode", changes)
            self.assertEqual((manager.get(off_sid), manager.events(off_sid), manager.jobs.qsize(),
                              dict(manager.generations), len(self.calls)), before)

        manager.send(off_sid, 'Enable helpers and Ultracode together',
                     {'thinking_effort':'ultracode', 'agents_enabled':True, 'agent_count':2})
        changed = self.settled(manager, off_sid)
        self.assertEqual((changed['thinking_effort'],changed['agents_enabled'],changed['agent_count']), ('ultracode',True,2))
        with self.assertRaises(sessions.SessionError):
            manager.send(off_sid, 'Invalid: Ultracode without helpers', {'agents_enabled':False})

    def test_cancelled_queued_prompt_is_not_run_after_immediate_resend(self):
        manager = self.manager()
        sid = self.create(manager)
        self.assertEqual(self.settled(manager, sid)["status"], "completed")
        blocker = self.create(manager, "sleep")
        self.wait_for(lambda: len(self.calls) == 2
                      and Path(self.calls[1]["pid_path"]).is_file())
        manager.send(sid, "obsolete prompt")
        self.assertEqual(manager.cancel(sid)["status"], "cancelled")
        manager.send(sid, "replacement prompt")
        manager.cancel(blocker)
        self.wait_for(lambda: manager.jobs.unfinished_tasks == 0)
        self.assertEqual(manager.get(sid)["status"], "completed")
        self.assertEqual(manager.get(sid)["native_session_id"], "native-original")
        for call in self.calls:
            if call["prompt"].splitlines()[0] == "obsolete prompt":
                self.assertFalse(Path(call["pid_path"]).exists())
        replacement = [call for call in self.calls if call["prompt"].splitlines()[0] == "replacement prompt"]
        self.assertEqual(len(replacement), 1)
        self.assertTrue(replacement[0]["prompt"].startswith("replacement prompt\n\nHarness session delegation requirement:\n"))
        self.assert_process_gone(replacement[0])
        self.assertEqual(sum(event.get("text") == "Fixture answer"
                             for event in manager.events(sid)), 2)

    def test_oversized_output_fails_and_the_next_session_still_runs(self):
        manager = self.manager()
        sid = self.create(manager, "oversize")
        self.assertEqual(self.settled(manager, sid)["status"], "failed")
        self.assert_process_gone(self.calls[-1])
        next_sid = self.create(manager)
        self.assertEqual(self.settled(manager, next_sid)["status"], "completed")

    def test_invalid_requests_do_not_create_sessions(self):
        manager = self.manager()
        base = dict(project_id=next(iter(manager.projects)), provider="codex", prompt="hello")
        invalid = [{"prompt": value} for value in (None, "", "  ", "a\0b", "é" * 16001)]
        invalid += [{"project_id": "unknown"}, {"project_id": {}}, {"provider": []},
                    {"provider": "cursor"}, {"mode": "bypass"}, {"workflow": "unknown"},
                    {"mode": "edit", "workflow": "review"}, {"model": []},
                    {"model": "bad\0model"}, {"model": "x" * 121}, {"project_context": 1},
                    {"native_session_id": "forged"}]
        invalid += [{"agents_enabled": value} for value in (None, 0, 1, "false", "true", [], {})]
        invalid += [{"agent_count": value} for value in (None, False, True, "3", 0, -1, 41, 1.5, 3.0, [], {})]
        invalid += [{"thinking_effort": value} for value in (False, True, 0, 1, [], {}, "unknown", "high\0")]
        invalid += [{"model": "fixture-small", "thinking_effort": "high"}]
        for changes in invalid:
            with self.subTest(changes=changes), self.assertRaises(sessions.SessionError):
                manager.create(dict(base, **changes))
        self.assertEqual(manager.list(), [])
        self.assertEqual(sessions.validate_prompt("é" * 16000), "é" * 16000)
        for count in (1, 40):
            with self.subTest(valid_count=count):
                sid = self.create(manager, agents_enabled=True, agent_count=count)
                self.assertEqual(self.settled(manager, sid)["status"], "completed")
                self.assertEqual(self.calls[-1]["agent_count"], count)

    def test_agent_preferences_reach_provider_and_survive_restart_and_followup(self):
        manager = self.manager()
        default_sid = self.create(manager, "complete default")
        default = self.settled(manager, default_sid)
        self.assertEqual(default["status"], "completed")
        self.assertIs(default["agents_enabled"], False)
        self.assertEqual(default["agent_count"], 3)
        default_call = self.calls[-1]
        self.assertIs(default_call["agents_enabled"], False)
        self.assertEqual(default_call["agent_count"], 3)
        self.assertTrue(default_call["prompt"].startswith("complete default\n\nHarness session delegation requirement:\n"))
        self.assertIn("Additional agents are disabled", default_call["prompt"])
        self.assertIn("Perform the task in the main agent", default_call["prompt"])
        self.assertIn("Do not spawn subagents", default_call["prompt"])

        sid = self.create(manager, "complete enabled", agents_enabled=True, agent_count=7)
        original = self.settled(manager, sid)
        self.assertEqual(original["status"], "completed")
        self.assertIs(original["agents_enabled"], True)
        self.assertEqual(original["agent_count"], 7)
        first_call = self.calls[-1]
        self.assertIs(first_call["agents_enabled"], True)
        self.assertEqual(first_call["agent_count"], 7)
        self.assertTrue(first_call["prompt"].startswith("complete enabled\n\nHarness session delegation requirement:\n"))
        self.assertIn("at most 7 additional agents concurrently", first_call["prompt"])
        self.assertIn("The main agent does not count", first_call["prompt"])
        self.assertNotIn("Use fewer helpers", first_call["prompt"])
        self.assertIn("MUST launch exactly 7 distinct additional agents", first_call["prompt"])
        self.assertIn("Wait for every helper's result", first_call["prompt"])
        self.assertIn("Do not delegate recursively", first_call["prompt"])
        history = manager.events(sid)
        manager.close()
        self.managers.remove(manager)

        restarted = self.manager()
        self.assertEqual(restarted.get(sid), original)
        self.assertEqual(restarted.events(sid), history)
        restarted.send(sid, "complete follow-up")
        resumed = self.settled(restarted, sid)
        self.assertEqual(resumed["status"], "completed")
        self.assertIs(resumed["agents_enabled"], True)
        self.assertEqual(resumed["agent_count"], 7)
        resumed_call = self.calls[-1]
        self.assertEqual((resumed_call["agents_enabled"], resumed_call["agent_count"],
                          resumed_call["session_id"]), (True, 7, "native-original"))
        self.assertTrue(resumed_call["prompt"].startswith("complete follow-up\n\nHarness session delegation requirement:\n"))
        self.assertIn("at most 7 additional agents concurrently", resumed_call["prompt"])
        self.assertIn("MUST launch exactly 7 distinct additional agents", resumed_call["prompt"])
        self.assertEqual([event["text"] for event in restarted.events(sid) if event["kind"] == "user"],
                         ["complete enabled", "complete follow-up"])

    def test_required_helper_receipts_are_per_launch_and_not_model_claims(self):
        manager = self.manager()
        sid = self.create(manager, 'with_helper', agents_enabled=True, agent_count=1)
        self.assertEqual(self.settled(manager, sid)['status'], 'completed')
        self.assertIn('MUST launch exactly 1 distinct additional agents', self.calls[-1]['prompt'])
        self.assertEqual([e['status'] for e in manager.events(sid) if e['kind'] == 'delegation'], ['confirmed'])
        manager.send(sid, 'complete')
        self.assertEqual(self.settled(manager, sid)['status'], 'completed')
        self.assertEqual([e['status'] for e in manager.events(sid) if e['kind'] == 'delegation'], ['confirmed', 'unconfirmed'])
        solo = self.create(manager, 'complete', agents_enabled=False, agent_count=1)
        self.assertEqual(self.settled(manager, solo)['status'], 'completed')
        self.assertFalse(any(e['kind'] == 'delegation' for e in manager.events(solo)))

    def test_single_helper_does_not_satisfy_required_count(self):
        manager = self.manager()
        sid = self.create(manager, 'with_helper_v2', agents_enabled=True, agent_count=10)
        self.assertEqual(self.settled(manager, sid)['status'], 'completed')
        self.assertIn('MUST launch exactly 10 distinct additional agents', self.calls[-1]['prompt'])
        summary = next(e for e in manager.events(sid) if e['kind'] == 'delegation')
        self.assertEqual((summary['status'],summary['confirmed_count'],summary['required_count'],summary['missing_count']),
                         ('partial',1,10,9))
        # CLI completion is independent; no automatic paid retry for missing receipts.
        self.assertEqual(len(self.calls), 1)

    def test_changed_helper_settings_reach_next_launch_and_preserve_previous_counts(self):
        manager = self.manager()
        sid = self.create(manager, agents_enabled=True, agent_count=10, budgets={'tokens':1100})
        self.settled(manager, sid)
        for options, enabled, count in (({'agent_count':3},True,3),
                                        ({'agents_enabled':False},False,3),
                                        ({'agents_enabled':True,'agent_count':1},True,1)):
            manager.send(sid, 'complete changed helpers', options)
            session = self.settled(manager, sid)
            self.assertEqual((session['agents_enabled'],session['agent_count']), (enabled,count))
            call = self.calls[-1]
            self.assertEqual((call['agents_enabled'],call['agent_count'],call['session_id']), (enabled,count,'native-original'))
            self.assertIn(f'MUST launch exactly {count}' if enabled else 'Additional agents are disabled', call['prompt'])
            self.assertEqual(session['agent_budget_plan']['tokens'], 1100 // (count+1 if enabled else 1))
        launches = manager.results.history(sid)['launches']
        self.assertEqual([(row['settings']['agents_enabled'],row['settings']['agent_count']) for row in launches],
                         [(True,10),(True,3),(False,3),(True,1)])
        manager.close(); self.managers.remove(manager)
        restarted = self.manager()
        self.assertEqual((restarted.get(sid)['agents_enabled'],restarted.get(sid)['agent_count']), (True,1))

    def test_v2_native_helper_activity_is_visible_and_confirms_delegation(self):
        manager = self.manager()
        sid = self.create(manager, 'with_helper_v2', agents_enabled=True, agent_count=1)
        self.assertEqual(self.settled(manager, sid)['status'], 'completed')
        events = manager.events(sid)
        self.assertEqual([e['status'] for e in events if e['kind'] == 'delegation'], ['confirmed'])
        self.assertEqual([e['text'] for e in events if e['kind'] == 'tool'],
                         ['Agent activity: started', 'Agent activity: completed'])

    def test_missing_stream_receipts_use_only_current_launch_native_journal(self):
        manager = self.manager()
        sid = self.create(manager, 'with_helper_journal', agents_enabled=True, agent_count=1)
        self.assertEqual(self.settled(manager, sid)['status'], 'completed')
        self.assertEqual([e['status'] for e in manager.events(sid) if e['kind']=='delegation'], ['confirmed'])
        manager.send(sid, 'complete')
        self.assertEqual(self.settled(manager, sid)['status'], 'completed')
        self.assertEqual([e['status'] for e in manager.events(sid) if e['kind']=='delegation'], ['confirmed','unconfirmed'])

    def test_journal_evidence_rejects_other_workspaces_turns_and_modified_metadata(self):
        manager = self.manager()
        sid = self.create(manager, 'with_helper_journal', agents_enabled=True, agent_count=1)
        self.settled(manager, sid)
        journal = self.root/'codex-home/sessions/rollout-native-original.jsonl'
        original = journal.read_text(); records = [json.loads(line) for line in original.splitlines()]
        since, until = time.time()-60, time.time()+1
        read = sessions.providers.codex_journal_activity
        self.assertEqual(read('native-original',self.project,since,until), [{'phase':'started','agent_id':'fixture-child'}])
        self.assertEqual(read('native-other',self.project,since,until), [])
        self.assertEqual(read('native-original',self.root,since,until), [])
        for row, key, value in ((0,'id','other'), (0,'cwd','/other'), (0,'source','other'),
                                (2,'turn_id','other'), (2,'thread_id','other')):
            changed = json.loads(json.dumps(records)); changed[row]['payload'][key] = value
            journal.write_text(''.join(json.dumps(record)+'\n' for record in changed))
            self.assertEqual(read('native-original',self.project,since,until), [])
        journal.write_text('malformed')
        self.assertEqual(read('native-original',self.project,since,until), [])
        journal.write_text(original); saved = journal.with_suffix('.saved'); journal.rename(saved)
        journal.symlink_to(saved)
        self.assertEqual(read('native-original',self.project,since,until), [])

    def test_legacy_thirteen_column_database_keeps_session_and_history_during_migration(self):
        state = self.root / "legacy-state"
        state.mkdir()
        project_id = hashlib.sha256(str(self.project).encode()).hexdigest()[:16]
        sid = "00000000-0000-4000-8000-000000000001"
        timestamp = "2026-09-05T00:00:00+00:00"
        legacy = (sid, "Legacy session", project_id, str(self.project), "codex", "legacy-model",
                  "edit", "native", 1, "completed", "native-original", timestamp, timestamp)
        with contextlib.closing(sqlite3.connect(state / "sessions.sqlite3")) as database, database:
            database.executescript('''
                CREATE TABLE sessions (
                    id TEXT PRIMARY KEY, title TEXT NOT NULL, project_id TEXT NOT NULL,
                    project_path TEXT NOT NULL, provider TEXT NOT NULL, model TEXT,
                    mode TEXT NOT NULL, workflow TEXT NOT NULL, project_context INTEGER NOT NULL,
                    status TEXT NOT NULL, native_session_id TEXT, created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL);
                CREATE TABLE events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, data TEXT NOT NULL);
            ''')
            database.execute("INSERT INTO sessions VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)", legacy)
            database.execute("INSERT INTO events VALUES (7,?,?)", (sid, json.dumps({
                "kind": "user", "text": "Original legacy request",
            })))
            self.assertEqual(len(database.execute("PRAGMA table_info(sessions)").fetchall()), 13)
        manager = self.manager(state=state)
        restored = manager.get(sid)
        names = ("id", "title", "project_id", "project_path", "provider", "model", "mode",
                 "workflow", "project_context", "status", "native_session_id", "created_at", "updated_at")
        self.assertEqual(tuple(restored[name] for name in names), legacy)
        self.assertIs(restored["agents_enabled"], False)
        self.assertEqual(restored["agent_count"], 3)
        self.assertIsNone(restored["thinking_effort"])
        self.assertEqual(restored["workspace"], "project")
        self.assertIsNone(restored["branch"])
        self.assertIsNone(restored["git_common_dir"])
        self.assertEqual(manager.events(sid), [{"id": 7, "kind": "user", "text": "Original legacy request"}])

    def test_context_and_state_symlinks_are_not_followed(self):
        outside = self.root / "outside.txt"
        outside.write_text("OUTSIDE SECRET")
        (self.project / "private.txt").write_text("SYMLINK TARGET")
        (self.project / "AGENTS.md").symlink_to(outside)
        (self.project / "README.md").symlink_to("private.txt")
        (self.project / "CLAUDE.md").write_text("REGULAR PROJECT CONTEXT")
        manager = self.manager()
        key = next(iter(manager.projects))
        files = {item["path"]: item for item in manager.context(key)["files"]}
        self.assertFalse(files["AGENTS.md"]["exists"])
        self.assertFalse(files["README.md"]["exists"])
        sid = self.create(manager, project_context=True)
        self.assertEqual(self.settled(manager, sid)["status"], "completed")
        self.assertIn("REGULAR PROJECT CONTEXT", self.calls[-1]["prompt"])
        self.assertNotIn("OUTSIDE SECRET", self.calls[-1]["prompt"])
        self.assertNotIn("SYMLINK TARGET", self.calls[-1]["prompt"])
        alias = self.root / "state-alias"
        alias.symlink_to(manager.state_dir, target_is_directory=True)
        with self.assertRaises(sessions.SessionError):
            self.manager(state=alias)
        other_state = self.root / "other-state"
        other_state.mkdir()
        (other_state / "sessions.sqlite3").symlink_to(outside)
        with self.assertRaises(sessions.SessionError):
            self.manager(state=other_state)
        self.assertEqual(outside.read_text(), "OUTSIDE SECRET")


if __name__ == "__main__":
    unittest.main()
