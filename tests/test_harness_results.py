"""Result evidence is independent from provider success; run entirely offline."""
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'harness/src'))
from harness import sessions, results


class ResultTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name); self.project=self.root/'project';self.project.mkdir()
        self.git('init','-q');self.git('config','user.email','test@example.invalid');self.git('config','user.name','Fixture')
        (self.project/'value.txt').write_text('before\n');self.git('add','value.txt');self.git('commit','-qm','baseline')
        self.fake=self.root/'provider.py';self.fake.write_text('''import json,sys
sys.stdin.read()
print(json.dumps({'type':'thread.started','thread_id':'fixture-original'}),flush=True)
print(json.dumps({'type':'turn.completed','usage':{'input_tokens':12,'output_tokens':3}}),flush=True)
''')
        discover=patch.object(sessions.providers,'discover_providers',return_value=[{'id':'codex','available':True,'executable':str(self.fake)}]);discover.start();self.addCleanup(discover.stop)
        builder=patch.object(sessions.providers,'build_command',return_value=[sys.executable,str(self.fake)]);builder.start();self.addCleanup(builder.stop)
        self.store=sessions.Sessions(self.root/'state',[self.project]);self.addCleanup(lambda:self.store.close())
        self.sid=self.store.create({'project_id':next(iter(self.store.projects)),'provider':'codex','prompt':'fixture','project_context':False})['id'];self.wait()

    def git(self,*args,cwd=None):
        env={k:v for k,v in os.environ.items() if not k.startswith('GIT_')}
        return subprocess.run(['git','-c','core.hooksPath=/dev/null','-c','commit.gpgSign=false','-C',str(cwd or self.project),*args],env=env,check=True,capture_output=True,text=True).stdout.strip()

    def wait(self):
        deadline=time.monotonic()+12
        while time.monotonic()<deadline:
            if self.store.get(self.sid)['status'] not in sessions.ACTIVE and not self.store.jobs.unfinished_tasks:return
            time.sleep(.03)
        self.fail('Run did not finish')

    def check(self,code,timeout=5):
        snapshot=self.store.results.snapshot(self.sid)
        self.store.results.start_check(self.sid,{'command':shlex.join([sys.executable,'-c',code]),'timeout':timeout,'snapshot_id':snapshot['id']})
        self.wait();return self.store.results.history(self.sid)['checks'][0]

    def test_diff_covers_committed_staged_unstaged_untracked_without_following_links(self):
        base=self.store.get(self.sid)['result_base']['head']
        (self.project/'value.txt').write_text('committed by agent\n');self.git('commit','-qam','agent change')
        (self.project/'new.txt').write_text('new data\n');(self.project/'linked').symlink_to('/etc/passwd')
        snapshot=self.store.results.snapshot(self.sid)
        self.assertEqual(snapshot['base'],base);self.assertIn('+committed by agent',snapshot['diff'])
        self.assertIn('+new data',snapshot['diff']);self.assertNotIn('root:x:',snapshot['diff'])
        self.assertFalse(snapshot['complete'])

    def test_checks_persist_output_exit_status_and_do_not_overwrite_agent_outcome(self):
        failed=self.check("import sys; print('independent error'); sys.exit(7)")
        self.assertEqual(failed['status'],'failed');self.assertEqual(failed['exit_code'],7);self.assertIn('independent error',failed['output'])
        self.assertEqual(self.store.get(self.sid)['status'],'completed')
        passed=self.check("print('test passed')")
        self.assertEqual(passed['status'],'passed');self.assertFalse(passed['workspace_changed'])
        (self.project/'value.txt').write_text('changed after verification\n')
        self.assertNotEqual(self.store.results.snapshot(self.sid)['id'],passed['snapshot_id'])
        with self.assertRaises(sessions.SessionError):
            self.store.results.start_check(self.sid,{'command':'true','timeout':2,'snapshot_id':passed['snapshot_id']})
        self.store.close();self.store=sessions.Sessions(self.root/'state',[self.project])
        self.assertEqual(len(self.store.results.history(self.sid)['checks']),2)
        # Persist the state left by a server crash during a check.
        passed.update(status='running',previous_status='completed')
        self.store.results.save_check(passed);self.store._status(self.sid,'running')
        self.store.close();self.store=sessions.Sessions(self.root/'state',[self.project])
        restored=next(c for c in self.store.results.history(self.sid)['checks'] if c['id']==passed['id'])
        self.assertEqual(restored['status'],'interrupted');self.assertEqual(restored['output'],passed['output'])
        self.assertEqual(self.store.get(self.sid)['status'],'completed')

    def test_timeout_output_limit_cancellation_and_validation(self):
        timed=self.check('import time; time.sleep(20)',timeout=1)
        self.assertEqual(timed['status'],'timed_out');self.assertLess(timed['seconds'],5)
        with patch.object(results,'OUTPUT_LIMIT',1024):
            limited=self.check("print('x'*2048)")
        self.assertEqual(limited['status'],'output_limit');self.assertLessEqual(len(limited['output']),1024)
        snapshot=self.store.results.snapshot(self.sid)
        for invalid in ({'command':'echo hi && echo bye','timeout':2}, {'command':'true','timeout':False}, {'command':'true','timeout':0}):
            with self.assertRaises(sessions.SessionError): self.store.results.start_check(self.sid,{**invalid,'snapshot_id':snapshot['id']})
        self.store.results.start_check(self.sid,{'command':shlex.join([sys.executable,'-c','import time;time.sleep(20)']),'timeout':30,'snapshot_id':snapshot['id']})
        with self.assertRaises(sessions.SessionError): self.store.send(self.sid,'overlap')
        self.store.cancel(self.sid); self.wait()
        self.assertEqual(self.store.results.history(self.sid)['checks'][0]['status'],'cancelled')
        self.assertEqual(self.store.get(self.sid)['status'],'completed')
        with patch.object(results.subprocess,'Popen') as launch:
            cancelled=self.store.results.capture(['never-launch-after-cancel'],self.project,sid=self.sid)
            self.assertEqual(cancelled['reason'],'cancelled'); launch.assert_not_called()

    def test_history_accumulates_turns_and_preserves_unknown_after_restart(self):
        first=self.store.results.history(self.sid)
        self.assertEqual(first['totals']['tokens'],{'reported':15,'unknown_launches':0})
        self.store.send(self.sid,'second turn');self.wait()
        history=self.store.results.history(self.sid)
        self.assertEqual(len(history['launches']),2);self.assertEqual(history['totals']['tokens']['reported'],30)
        self.assertEqual(history['totals']['cost_usd'],{'reported':0,'unknown_launches':2})
        self.store.set_budgets(self.sid,{'budgets':{'seconds':2},'revision':0})
        self.assertEqual(self.store.results.history(self.sid)['launches'],history['launches'])
        self.store.close();self.store=sessions.Sessions(self.root/'state',[self.project])
        self.assertEqual(self.store.results.history(self.sid)['launches'],history['launches'])

    def test_queued_check_rejects_workspace_changes_before_execution(self):
        import threading
        gate=threading.Event(); entered=threading.Event(); original=self.store.results.run_check
        def delayed(*args): entered.set();gate.wait(4);return original(*args)
        snapshot=self.store.results.snapshot(self.sid)
        with patch.object(self.store.results,'run_check',side_effect=delayed):
            self.store.results.start_check(self.sid,{'command':shlex.join([sys.executable,'-c',"print('must not execute')"]),'timeout':3,'snapshot_id':snapshot['id']})
            self.assertTrue(entered.wait(2));(self.project/'value.txt').write_text('changed while queued');gate.set();self.wait()
        check=self.store.results.history(self.sid)['checks'][0]
        self.assertEqual(check['status'],'failed');self.assertIn('changed while queued',check['output'])
        self.assertNotIn('must not execute',check['output'])

    def test_worktree_check_runs_in_worktree_and_leaves_primary_unchanged(self):
        sid=self.store.create({'project_id':next(iter(self.store.projects)),'provider':'codex','prompt':'worktree fixture','workspace':'worktree'})['id']
        self.sid=sid;self.wait();project=Path(self.store.get(sid)['project_path'])
        result=self.check("from pathlib import Path; print(Path.cwd()); Path('check-artifact').write_text('ok')")
        self.assertIn(str(project),result['output']);self.assertTrue(result['workspace_changed'])
        self.assertFalse((self.project/'check-artifact').exists())


if __name__=='__main__': unittest.main()
