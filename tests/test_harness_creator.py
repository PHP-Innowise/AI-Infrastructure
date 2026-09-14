"""Creator checkpoints, real subprocess isolation and canonical publication recovery."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'harness/src'))
from harness import creator, creator_runner, sessions


class CreatorTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name); self.target=self.root/'project'; self.target.mkdir()
        (self.target/'composer.json').write_text('{"require":{"php":"^8.2"}}')
        self.fake=self.target/'fixture-provider'
        self.fake.write_text('#!'+sys.executable+'\n'+'''import sys,json,pathlib
prompt=sys.stdin.read()
if 'invalid-artifacts' not in prompt:
    pathlib.Path('tasks/TASK-001/harness-questions.md').write_text('Which PHP framework should be supported?')
print(json.dumps({'type':'turn.completed'}),flush=True)
'''); self.fake.chmod(0o700)
        provider=patch.object(sessions.providers,'discover_providers',return_value=[{'id':'codex','available':True,'executable':str(self.fake)}]);provider.start();self.addCleanup(provider.stop)
        self.store=sessions.Sessions(self.root/'state',[self.target],timeout=15);self.addCleanup(self.store.close)
        self.manager=creator.CreatorManager(self.store);self.pid=next(iter(self.store.projects))

    def options(self,**values):
        return {'project_id':self.pid,'provider':'codex','tools':['codex'],**values}

    def wait(self,rid):
        deadline=time.monotonic()+12
        while time.monotonic()<deadline:
            data=self.manager.get(rid)
            if data['run']['status'] not in ('scanning','generating','applying','rolling_back'):return data
            time.sleep(.05)
        self.fail('Creator phase did not finish')

    def prepared(self,operation='generate'):
        rid='a'*32;directory=self.manager.root/rid; task=directory/'agent'/creator.TASK
        stage=task/'infra-generate-staging';stage.mkdir(parents=True)
        for name in creator.REPORTS[:3]: (task/name).write_text('{}')
        (task/creator.PLAN_NAMES[0]).write_text('AGENTS.md\n')
        (stage/'AGENTS.md').write_text('generated\n')
        (stage/'.infra-manifest.json').write_text(json.dumps({'files':{'AGENTS.md':hashlib.sha256(b'generated\n').hexdigest()}}))
        stat=self.target.stat()
        run={'id':rid,'project_id':self.pid,'target':str(self.target),'identity':[stat.st_dev,stat.st_ino],
             'directory':str(directory),'status':'preview','revision':2,'phase':'generate','operation':operation,
             'provider':'codex','tools':['codex'],'model':None,'thinking_effort':None,'agents_enabled':False,'agent_count':3,
             'workspace':'project','branch':None,'goal':'','answers':'','created_at':sessions.now()}
        run['approved']=creator.profile_hashes(directory);self.manager._save(run)
        return run,directory,stage

    def test_checkpoint_budget_save_preserves_preview_and_applies_only_to_model_phases(self):
        run,directory,stage=self.prepared(); rid=run['id']
        data=self.manager.act(rid,{'action':'budgets','revision':2,'budgets':{'tokens':200,'seconds':10}})
        self.assertEqual(data['run']['approved'],run['approved']); self.assertEqual(data['run']['status'],'preview')
        self.assertEqual(data['run']['revision'],3)
        with self.assertRaises(sessions.SessionError): self.manager.act(rid,{'action':'budgets','revision':2,'budgets':{}})
        with patch.object(self.store,'_run',return_value=None):
            model=self.manager._launch(data['run'],'generate')
            self.assertEqual(model['session']['budgets']['tokens'],200)
            request=json.loads(next(directory.glob('request-*.json')).read_text())
            self.assertEqual(request['budgets']['seconds'],10)
            self.assertIn('200 total input',creator_runner.prompt_for(request))
            self.store._status(model['session']['id'],'failed')
            publish=self.manager._launch(data['run'],'apply')
            self.assertEqual(publish['session']['budgets'],sessions.DEFAULT_BUDGETS)

    def test_creator_forwards_numeric_usage(self):
        import io, contextlib
        output=io.StringIO()
        code="import json; print(json.dumps({'type':'turn.completed','usage':{'input_tokens':11,'output_tokens':2}}))"
        with contextlib.redirect_stdout(output):
            creator_runner.run_process([sys.executable,'-c',code],self.target,provider='codex')
        events=[json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(events[0]['input_tokens'],11); self.assertEqual(events[0]['output_tokens'],2)

    def test_creator_requires_helpers_and_forwards_native_delegation_evidence(self):
        import io, contextlib
        request = {'phase':'scan', 'operation':'create', 'target':str(self.target),
                   'tools':['codex'], 'goal':'Inspect', 'answers':'', 'provider':'codex',
                   'agents_enabled':True, 'agent_count':1, 'thinking_effort':None}
        self.assertIn('MUST launch exactly 1 distinct additional agents', creator_runner.prompt_for(request))
        self.assertIn('Execute scanner/skill roles sequentially yourself',
                      creator_runner.prompt_for({**request, 'agents_enabled':False}))
        events = [{'type':'item.completed', 'item':{'type':'collab_tool_call','tool':'spawn_agent',
                   'status':'completed','receiver_thread_ids':['child']}}, {'type':'turn.completed'}]
        code = 'import json\nfor event in ' + repr(events) + ': print(json.dumps(event))'
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            creator_runner.run_process([sys.executable,'-c',code],self.target,provider='codex',agents_enabled=True)
        observed = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(observed[-1]['kind'], 'delegation')
        self.assertEqual(observed[-1]['status'], 'confirmed')
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            creator_runner.run_process([sys.executable,'-c',code],self.target,
                                       provider='codex',agents_enabled=True,agent_count=3)
        summary = json.loads(output.getvalue().splitlines()[-1])
        self.assertEqual((summary['status'],summary['confirmed_count'],summary['required_count']), ('partial',1,3))

    def test_creator_recovers_helper_receipts_omitted_from_native_stream(self):
        import io, contextlib
        from tests.test_harness_sessions import FAKE_CLI
        fake = self.target/'fixture-provider.py'; fake.write_text(FAKE_CLI)
        config = {'provider':'codex','behavior':'with_helper_journal','pid_path':str(self.target/'native.pid')}
        output = io.StringIO()
        with patch.dict(os.environ, {'CODEX_HOME':str(self.target/'codex-home')}), contextlib.redirect_stdout(output):
            creator_runner.run_process([sys.executable,str(fake),json.dumps(config)],self.target,
                                       provider='codex',agents_enabled=True)
        events = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(events[-1]['status'], 'confirmed')
        self.assertTrue(any('native session journal' in e.get('text','') for e in events))

    def approve(self,run,directory):
        preview=self.manager.preview(run['id']);creator.write_json(directory/'approved-preview.json',preview);return preview

    @unittest.skipUnless(shutil.which('bwrap'),'bubblewrap required')
    def test_real_native_phase_questions_then_invalid_artifacts_cannot_pass_review(self):
        data=self.manager.start(self.options());rid=data['run']['id'];data=self.wait(rid)
        self.assertEqual(data['run']['status'],'needs_input')
        self.assertIn('Which PHP',data['reports'][0]['text'])
        self.assertEqual(self.store.get(data['run']['session_id'])['workspace'],'creator')
        with self.assertRaises(sessions.SessionError): self.store.send(data['run']['session_id'],'bypass review')
        data=self.manager.act(rid,{'action':'revise','revision':data['run']['revision'],'answers':'invalid-artifacts'})
        data=self.wait(rid);self.assertEqual(data['run']['status'],'failed')
        self.assertFalse((self.target/'AGENTS.md').exists())

    @unittest.skipUnless(shutil.which('bwrap'),'bubblewrap required')
    def test_real_mounts_allow_staging_and_deny_target_and_receipt_writes(self):
        run,directory,_=self.prepared();work=directory/'agent'
        for path in (self.target/'composer.json',directory/'private-receipt'):
            code='from pathlib import Path; Path("allowed").write_text("ok"); Path('+repr(str(path))+').write_text("bad")'
            result=subprocess.run(creator.sandbox_command(work,self.target,[sys.executable,'-c',code]),capture_output=True)
            self.assertNotEqual(result.returncode,0);self.assertTrue((work/'allowed').exists())
        self.assertFalse((directory/'private-receipt').exists())
        self.assertIn('php',(self.target/'composer.json').read_text())

    @unittest.skipUnless(shutil.which('bwrap'),'bubblewrap required')
    def test_installed_runtime_check_uses_disposable_sqlite_cache(self):
        run,directory,stage=self.prepared();work=directory/'agent'
        assets=creator.GENERATOR/'.agents/skills/memory-seed/assets'
        bank=self.target/'memory-bank';bank.mkdir();(bank/'local').mkdir()
        shutil.copytree(assets/'scripts',bank/'scripts',ignore=shutil.ignore_patterns('__pycache__'))
        shutil.copytree(assets/'project-brain',self.target/'project-brain')
        config=self.target/'project-brain/config';template=config/'runtime.json.template'
        (config/'runtime.json').write_text(template.read_text().replace('{{TARGET_FRAMEWORK}}','generic').replace('{{CANONICAL_EDITION}}','.agents'))
        for action in ('status','validate'):
            command=[sys.executable,str(bank/'scripts/context.py'),'--root',str(self.target),action]
            result=subprocess.run(creator.sandbox_command(work,self.target,command,runtime_cache=True),capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        self.assertEqual(list((bank/'local').iterdir()),[])

    def test_options_and_update_ownership_are_validated_before_queue(self):
        for options in (self.options(operation='update'),self.options(tools=['codex','codex']),self.options(agent_count=True),self.options(agents_enabled='true'),self.options(extra=True)):
            with self.subTest(options=options),self.assertRaises(sessions.SessionError): self.manager.start(options)
        (self.target/'composer.json').unlink();(self.target/'src').mkdir()
        with self.assertRaisesRegex(sessions.SessionError,'PHP'):self.manager.start(self.options())
        self.assertEqual(self.store.list(),[])

    def test_seatbelt_backend_confines_writes_like_bubblewrap_and_reports_availability(self):
        run,directory,_=self.prepared();work=directory/'agent'
        codex_home=self.root/'codex-home';codex_home.mkdir()
        rule=lambda kind,path:f'({kind} file-write* (subpath "{Path(path).resolve()}"))'
        with patch.object(creator,'isolation_backend',return_value=('seatbelt',creator.SEATBELT)),patch.dict(os.environ,{'CODEX_HOME':str(codex_home)}):
            command=creator.sandbox_command(work,self.target,[sys.executable,'-c','pass'],provider='codex')
            self.assertEqual((command[0],command[1],command[3],command[4]),(creator.SEATBELT,'-f','--','/usr/bin/env'))
            self.assertEqual(command[5:8],[f'{name}={directory/"tmp"}' for name in ('TMPDIR','TMP','TEMP')])
            self.assertEqual(command[8:],[sys.executable,'-c','pass'])
            self.assertTrue((directory/'tmp').is_dir())
            profile=Path(command[2]);lines=profile.read_text().splitlines()
            self.assertEqual((profile.parent,profile.stat().st_mode&0o777,lines[:3]),(directory,0o600,['(version 1)','(allow default)','(deny file-write*)']))
            for path in (work,directory/'tmp',codex_home): self.assertIn(rule('allow',path),lines)
            self.assertIn(rule('deny',self.target),lines)
            self.assertGreater(lines.index(rule('deny',self.target)),lines.index(rule('allow',work)))
            self.assertNotIn(rule('allow',directory),lines)  # run receipts stay read-only, as under bubblewrap
            bank=self.target/'memory-bank/local';bank.mkdir(parents=True)
            cached=creator.sandbox_command(work,self.target,['check'],runtime_cache=True)
            cached_lines=Path(cached[2]).read_text().splitlines()
            self.assertNotEqual(cached[2],command[2]);self.assertNotIn(rule('allow',bank),lines)
            self.assertGreater(cached_lines.index(rule('allow',bank)),cached_lines.index(rule('deny',self.target)))
        self.assertEqual(creator._sbpl('a"b\\c'),'"a\\"b\\\\c"')
        with patch.object(creator.shutil,'which',return_value=None):
            with patch.object(creator.sys,'platform','darwin'),patch.object(creator.os,'access',return_value=True):
                self.assertEqual(creator.isolation_backend(),('seatbelt',creator.SEATBELT))
            with patch.object(creator.sys,'platform','linux'):
                self.assertIsNone(creator.isolation_backend())
                self.assertFalse(self.manager.list(self.pid)['available'])
                with self.assertRaisesRegex(sessions.SessionError,'sandbox-exec'):creator.sandbox_command(work,self.target,['check'])
                with self.assertRaisesRegex(sessions.SessionError,'sandbox-exec'):self.manager.start(self.options())

    @unittest.skipUnless(shutil.which('bwrap'),'bubblewrap required')
    def test_claude_generator_shell_permission_keeps_agent_preference_and_boundary(self):
        run,directory,stage=self.prepared();run.update(provider='claude',phase='scan',executable=str(self.fake))
        commands=[]
        def native(command,*args):
            commands.append(command)
            (directory/'agent'/creator.TASK/'harness-questions.md').write_text('Need a project decision.')
        with patch.object(creator_runner,'run_process',side_effect=native):self.assertEqual(creator_runner.execute(run),'needs_input')
        command=commands[0]
        self.assertEqual(command[0],shutil.which('bwrap'))
        self.assertIn('--ro-bind',command);self.assertIn('--allowedTools',command);self.assertIn('Bash',command)
        self.assertIn('--disallowedTools',command);self.assertIn('Agent',command)

    def test_generation_requires_current_review_and_persists_exact_approval(self):
        run,directory,stage=self.prepared();run['status']='review';run.pop('approved');self.manager._save(run)
        with self.assertRaises(sessions.SessionError):self.manager.act(run['id'],{'action':'generate','revision':1})
        with patch.object(self.store,'_run'):
            result=self.manager.act(run['id'],{'action':'generate','revision':2})
        generated=result['run'];self.assertEqual(generated['status'],'generating')
        self.assertEqual(generated['approved'],creator.profile_hashes(directory))
        self.assertEqual(self.store.get(generated['session_id'])['creator']['phase'],'generate')
        with self.assertRaises(sessions.SessionError):self.manager.act(run['id'],{'action':'apply','revision':generated['revision']})

    def test_stale_review_and_modified_existing_file_need_explicit_decisions(self):
        run,directory,stage=self.prepared('update');(self.target/'AGENTS.md').write_text('team edit\n')
        (self.target/'.infra-manifest.json').write_text(json.dumps({'files':{'AGENTS.md':hashlib.sha256(b'previous\n').hexdigest()}}))
        preview=self.approve(run,directory)
        self.assertTrue(preview['files'][0]['requires_decision'])
        for body in ({'action':'apply','revision':1,'preview_id':preview['preview_id'],'replace':['AGENTS.md']},
                     {'action':'apply','revision':2,'preview_id':preview['preview_id'],'replace':[]}):
            with self.assertRaises(sessions.SessionError):self.manager.act(run['id'],body)
        (self.target/'AGENTS.md').write_text('newer team edit\n')
        with patch.object(creator_runner,'verify') as gate, self.assertRaisesRegex(sessions.SessionError,'changed after approval'):
            creator_runner.apply(run)
        gate.assert_not_called();self.assertEqual((self.target/'AGENTS.md').read_text(),'newer team edit\n')

    def test_changed_staging_and_reviewed_profile_invalidate_approval(self):
        for name in ('AGENTS.md','profile'):
            with self.subTest(name=name):
                run,directory,stage=self.prepared() if name=='AGENTS.md' else (run,directory,stage)
                self.approve(run,directory)
                path=stage/name if name!='profile' else directory/'agent'/creator.TASK/creator.REPORTS[0]
                path.write_text('changed')
                with patch.object(creator_runner,'verify'),self.assertRaisesRegex(sessions.SessionError,'changed after approval'):creator_runner.apply(run)
        self.assertFalse((self.target/'AGENTS.md').exists())

    def test_canonical_publication_and_postcheck_failure_roll_back(self):
        run,directory,stage=self.prepared();(self.target/'AGENTS.md').write_text('original\n')
        self.approve(run,directory)
        with patch.object(creator_runner,'verify',side_effect=[None,sessions.SessionError('postcheck failed')]):
            with self.assertRaisesRegex(sessions.SessionError,'postcheck failed'):creator_runner.apply(run)
        self.assertEqual((self.target/'AGENTS.md').read_text(),'original\n')
        self.assertFalse((self.target/'.infra-manifest.json').exists())
        self.assertEqual(json.loads((directory/'journal/journal.json').read_text())['status'],'rolled-back')
        (directory/'journal').rename(directory/'old-journal')
        self.approve(run,directory)
        with patch.object(creator_runner,'verify') as gate:creator_runner.apply(run)
        self.assertEqual(gate.call_count,2)
        self.assertEqual((self.target/'AGENTS.md').read_text(),'generated\n')
        self.assertEqual(json.loads((directory/'journal/journal.json').read_text())['status'],'verified')

    def test_recovery_survives_truncated_canonical_journal(self):
        run,directory,stage=self.prepared();(self.target/'AGENTS.md').write_text('original\n')
        self.approve(run,directory); pub=creator_runner.publisher(); publish=pub.publish
        def crash(*args):
            publish(*args)
            (directory/'journal/journal.json').write_text('{')
            raise KeyboardInterrupt('simulated crash')
        with patch.object(creator_runner,'verify'),patch.object(pub,'publish',side_effect=crash):
            with self.assertRaises(KeyboardInterrupt):creator_runner.apply(run)
        self.assertTrue(self.manager.get(run['id'])['run']['recovery_available'])
        creator_runner.rollback(run)
        self.assertEqual((self.target/'AGENTS.md').read_text(),'original\n')
        self.assertFalse((self.target/'.infra-manifest.json').exists())

    def test_partial_publication_recovery_preserves_later_edits(self):
        run,directory,stage=self.prepared();(self.target/'AGENTS.md').write_text('original\n')
        approval=self.approve(run,directory);pub=creator_runner.publisher()
        snapshot=pub.build_snapshot(self.target,approval['paths']);pub.prepare_journal(self.target,approval['paths'],directory/'journal',snapshot)
        (self.target/'AGENTS.md').write_text('external newer edit\n')
        self.assertTrue(self.manager.get(run['id'])['run']['recovery_available'])
        with self.assertRaises(pub.PublicationError):creator_runner.rollback(run)
        self.assertEqual((self.target/'AGENTS.md').read_text(),'external newer edit\n')
        self.assertEqual(json.loads((directory/'journal/journal.json').read_text())['status'],'rolled-back-with-conflicts')

    def test_real_ownership_gate_refuses_runtime_overwrite_even_when_approved(self):
        run,directory,stage=self.prepared();task=stage.parent
        path='memory-bank/INDEX.md';(stage/'memory-bank').mkdir();(stage/path).write_text('new')
        (self.target/'memory-bank').mkdir();(self.target/path).write_text('team memory')
        (task/creator.PLAN_NAMES[0]).write_text('AGENTS.md\n'+path+'\n');self.approve(run,directory)
        with patch.object(creator_runner,'verify'),self.assertRaises(creator_runner.publisher().PublicationError):creator_runner.apply(run)
        self.assertEqual((self.target/path).read_text(),'team memory')

    def test_preview_rejects_links_and_application_paths(self):
        run,directory,stage=self.prepared();(stage/'AGENTS.md').unlink();(stage/'AGENTS.md').symlink_to(self.target/'composer.json')
        with self.assertRaises(sessions.SessionError):self.manager.preview(run['id'])
        (stage/'AGENTS.md').unlink();(stage/'AGENTS.md').write_text('restored')
        (stage.parent/creator.PLAN_NAMES[0]).write_text('composer.json\n')
        with self.assertRaisesRegex(sessions.SessionError,'application source'):self.manager.preview(run['id'])

    def test_stopped_publication_cannot_be_cancelled_or_continued_as_chat(self):
        run,directory,stage=self.prepared()
        with patch.object(self.store,'_run'):
            result=self.manager._launch(run,'apply');sid=result['run']['session_id']
            with self.assertRaisesRegex(sessions.SessionError,'Publication'):self.store.cancel(sid)
            with self.assertRaises(sessions.SessionError):self.store.send(sid,'continue')
        with self.assertRaises(sessions.SessionError):self.store.create({'project_id':self.pid,'provider':'codex','prompt':'bad','creator':{}})


if __name__=='__main__':unittest.main()
