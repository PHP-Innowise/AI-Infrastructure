"""Automatic native Stop continuation and guarded semantic-result saves."""
import contextlib
from datetime import datetime, timedelta
import importlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]


class NativeCaptureTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='native-semantic-capture-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / 'public project'
        self.root.mkdir()
        self.env = dict(os.environ, PROJECT_BRAIN_OWNER='tester', GIT_OPTIONAL_LOCKS='0',
                        PYTHONDONTWRITEBYTECODE='1')
        for key in ('ACCELERATOR_HOME','ACCELERATOR_PROJECT_DIR','ACCELERATOR_STATE_DIR',
                    'CONTEXT_MEMORY_RECOVERY','CONTEXT_CAPSULE_DELIVERED','PROJECT_BRAIN_MODE'):
            self.env.pop(key, None)
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config',
                    'project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,
                            ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        (self.root/'memory-bank/chunks').mkdir()
        (self.root/'memory-bank/INDEX.md').write_text('# Memory\n')
        cfg_file=self.root/'project-brain/config/runtime.json'
        cfg=json.loads(cfg_file.read_text());cfg.update(automatic_promotion=True,
            automatic_compaction=False,automatic_completion=False,owners=['tester'])
        cfg_file.write_text(json.dumps(cfg))
        (self.root/'.gitignore').write_text('/memory-bank/local/\n/project-brain/local/\n__pycache__/\n')
        (self.root/'rules.md').write_text('# Routing\nThe Amber dispatcher retries only idempotent requests.\n')
        for args in (['init','-q','-b','research'],['config','user.name','Public Fixture'],
                     ['config','user.email','fixture@example.invalid'],['add','.'],['commit','-qm','Public baseline']):
            self.git(*args)
        self.payload={'progress':'Inspected the Amber retry contract.',
            'next_steps':['Check callers before changing the dispatcher.'], 'verified':True,
            'learnings':[{'type':'finding','title':'Amber retry boundary',
                          'consequence':'The Amber dispatcher retries only idempotent requests.',
                          'sources':['rules.md']}]}

    def git(self,*args):
        return subprocess.check_output(['git',*args],cwd=self.root,env=self.env)

    def call(self,*args,data=None,ok=True,env=None):
        result=subprocess.run([sys.executable,'-B',str(self.root/'memory-bank/scripts/context.py'),
            '--root',str(self.root),*args],cwd=self.root,env=env or self.env,
            input=json.dumps(data).encode() if data is not None else None,capture_output=True)
        if ok: self.assertEqual(0,result.returncode,result.stderr.decode())
        return result

    def begin(self,host='codex',session='session-a'):
        payload={'hook_event_name':'UserPromptSubmit','session_id':session,'prompt':'Inspect the retry policy.'}
        if host=='cursor':payload={'hook_event_name':'beforeSubmitPrompt','conversation_id':session}
        result=self.call('native-prompt','--host',host,'--task-id','research','--json',data=payload)
        return json.loads(result.stdout)

    def stop(self,host='codex',session='session-a',active=False,status='completed'):
        payload={'hook_event_name':'Stop','session_id':session,'stop_hook_active':active,
                 'last_assistant_message':'The Amber dispatcher retries only idempotent requests.'}
        if host=='cursor':payload={'conversation_id':session,'hook_event_name':'stop',
                                  'status':status,'loop_count':int(active)}
        return self.call('native-stop','--host',host,'--task-id','research',data=payload)

    def state(self):
        candidates=list((self.root/'project-brain/local/native-results').glob('*.json'))
        self.assertEqual(1,len(candidates))
        return candidates[0],json.loads(candidates[0].read_text())

    def save(self,data=None,ok=True,env=None):
        candidate,state=self.state()
        return self.call('native-result','--request',candidate.stem,'--token',state['token'],
                         '--input','-','--json',data=data or self.payload,ok=ok,env=env)

    def records(self,kind):
        folder=self.root/'project-brain/dynamic'/kind
        return [json.loads(item.read_text().split('---',2)[1]) for item in folder.glob('*.md')]

    def authority(self):
        return {str(item.relative_to(self.root)):item.read_bytes() for prefix in
                ('project-brain/dynamic','project-brain/archive','project-brain/control',
                 'project-brain/indexes','memory-bank/chunks')
                for item in (self.root/prefix).rglob('*') if item.is_file()}

    @contextlib.contextmanager
    def runtime(self):
        scripts=self.root/'memory-bank/scripts'
        names={item.stem for item in scripts.glob('*.py')}
        previous={name:sys.modules.pop(name) for name in names if name in sys.modules}
        sys.path.insert(0,str(scripts))
        try:
            with mock.patch.dict(os.environ,self.env,clear=True):
                yield (importlib.import_module('native_results'),
                       importlib.import_module('memory_results'),
                       importlib.import_module('brain_runtime'))
        finally:
            sys.path.remove(str(scripts))
            for name in names:sys.modules.pop(name,None)
            sys.modules.update(previous)

    def interrupt_save(self):
        self.call('start','--task-id','research','--goal','Inspect routing')
        self.begin();self.stop();candidate,state=self.state()
        with self.runtime() as (native,results,_):
            with mock.patch.object(results,'update_record',
                                   side_effect=OSError('Public transient close failure')):
                with self.assertRaisesRegex(OSError,'Public transient close failure'):
                    native.save(self.root,candidate.stem,state['token'],self.payload,
                        owner='tester',mode='governed',provision=lambda task:None)
        self.assertEqual(['open'],[item['status'] for item in self.records('findings')])
        self.assertTrue(self.state()[1]['digest'])
        self.stop(active=True)
        self.assertEqual('failed',self.state()[1]['phase'])

    def test_clean_research_creates_result_and_new_session_recalls_it(self):
        source=(self.root/'rules.md').read_bytes();self.assertEqual(b'',self.git('status','--porcelain'))
        self.begin();self.assertEqual([],self.records('tasks'))
        self.assertEqual('block',json.loads(self.stop().stdout)['decision'])
        self.assertEqual([],self.records('tasks'))
        saved=json.loads(self.save().stdout);self.assertEqual('agent',saved['attestation'])
        self.assertEqual(source,(self.root/'rules.md').read_bytes())
        tasks=self.records('tasks');self.assertEqual(1,len(tasks));self.assertNotEqual('completed',tasks[0]['status'])
        self.assertEqual(self.payload['progress'],tasks[0]['progress'])
        self.assertEqual(self.payload['next_steps'],tasks[0]['next_steps'])
        self.assertEqual(1,len(self.records('findings')))
        self.assertEqual(1,len(list((self.root/'memory-bank/chunks').glob('*.md'))))
        response=json.loads(self.call('refresh','--task-id','research','--query','Amber retry boundary',
            '--session-id','fresh-session','--ephemeral','--gate','off','--json').stdout)
        self.assertIn(self.payload['learnings'][0]['consequence'],response['capsule_text'])
        self.assertEqual(b'',self.stop(active=True).stdout)

    @unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'POSIX shell pipeline profile')
    def test_emitted_save_command_accepts_pipe_from_another_directory(self):
        self.begin();reason=json.loads(self.stop().stdout)['reason']
        command=reason.split('exact command: ',1)[1].split('\n',1)[0]
        result=subprocess.run(['bash','-c','cat | '+command],cwd=self.root.parent,env=self.env,
                              input=json.dumps(self.payload).encode(),capture_output=True)
        self.assertEqual(0,result.returncode,result.stderr.decode())
        self.assertEqual(self.payload['progress'],self.records('tasks')[0]['progress'])

    def test_duplicate_stop_and_exact_save_leave_authority_unchanged(self):
        self.begin();self.stop();self.save();before=self.authority()
        self.assertEqual(b'',self.stop().stdout)
        self.save();self.assertEqual(before,self.authority())

    def test_failed_started_save_resumes_exact_result_once(self):
        self.interrupt_save()
        self.save()
        self.assertEqual(['resolved'],[item['status'] for item in self.records('findings')])
        self.assertEqual(self.payload['progress'],self.records('tasks')[0]['progress'])
        self.assertEqual(1,len(list((self.root/'memory-bank/chunks').glob('*.md'))))
        before=self.authority()
        self.assertTrue(json.loads(self.save().stdout)['replayed'])
        self.assertEqual(before,self.authority())
        self.assertEqual(b'',self.stop(active=True).stdout)
        self.assertEqual(b'',self.stop().stdout)

    def test_completed_exact_replay_does_not_require_live_source(self):
        self.begin();self.stop();self.save();(self.root/'rules.md').unlink()
        before=self.authority()
        self.assertTrue(json.loads(self.save().stdout)['replayed'])
        self.assertEqual(before,self.authority())

    def test_completed_archived_task_replay_uses_original_uuid(self):
        self.begin();self.stop();self.save()
        with self.runtime() as (_,_,brain):
            task=brain.find_record(self.root,self.state()[1]['task_uuid'],record_type='task')[1]
            brain.update_record(self.root,task['id'],expected_revision=task['revision'],
                progress=None,next_steps=[],files=[],sources=[],actor='tester',
                transition_to='completed',reason='Public fixture finished')
            brain.compact(self.root)
            self.assertEqual([],self.records('tasks'))
            replacement=brain.create_task(self.root,'research','A different task reuses the alias',
                                           [],[],owner='tester')
        self.assertNotEqual(replacement['id'],self.state()[1]['task_uuid'])
        before=self.authority()
        self.assertTrue(json.loads(self.save().stdout)['replayed'])
        self.assertEqual(before,self.authority())

    def test_failed_retry_cannot_change_content_or_write_without_current_source(self):
        self.interrupt_save();before=self.authority()
        changed={**self.payload,'progress':'A different result.'}
        self.assertNotEqual(0,self.save(changed,ok=False).returncode)
        self.assertEqual(before,self.authority())
        (self.root/'rules.md').unlink()
        self.assertNotEqual(0,self.save(ok=False).returncode)
        self.assertEqual(before,self.authority())

    def test_failed_without_started_save_cannot_create_first_result(self):
        self.begin();self.stop();self.stop(active=True);before=self.authority()
        self.assertIsNone(self.state()[1]['digest'])
        self.assertNotEqual(0,self.save(ok=False).returncode)
        self.assertEqual(before,self.authority())

    def test_begun_retry_without_canonical_proof_cannot_become_fresh_write(self):
        self.begin();self.stop();self.save()
        shutil.rmtree(self.root/'project-brain/local/results')
        task=self.records('tasks')[0]
        candidate,state=self.state()
        with self.runtime() as (_,results,_):
            results._attempt(self.root,state['task_uuid'],'native-'+state['token'],
                             state['digest']).unlink()
        # Even a locally edited revision cannot turn replay into a new write.
        state['revision']=task['revision'];candidate.write_text(json.dumps(state))
        before=self.authority()
        result=self.save(ok=False)
        self.assertNotEqual(0,result.returncode)
        self.assertIn(b'receipt_unavailable',result.stderr)
        self.assertEqual(before,self.authority())

    def test_shared_completion_proof_survives_disposable_receipt_loss(self):
        self.begin();self.stop();self.save()
        shutil.rmtree(self.root/'project-brain/local/results')
        (self.root/'rules.md').unlink();before=self.authority()
        self.assertTrue(json.loads(self.save().stdout)['replayed'])
        self.assertEqual(before,self.authority())

    def test_begun_retry_requires_pinned_task_and_revision(self):
        self.begin();self.stop();self.save();candidate,valid=self.state()
        before=self.authority()
        for field in ('task_uuid','revision'):
            with self.subTest(field=field):
                candidate.write_text(json.dumps({**valid,field:None}))
                result=self.save(ok=False)
                self.assertNotEqual(0,result.returncode)
                self.assertNotIn(b'Traceback',result.stderr)
                self.assertEqual(before,self.authority())

    def test_new_prompt_rearms_and_progress_only_preserves_next_steps(self):
        self.begin();self.stop();self.save();_,old=self.state()
        self.begin();self.stop();_,new=self.state();self.assertNotEqual(old['token'],new['token'])
        self.save({'progress':'Inspected the caller; implementation remains pending.','learnings':[]})
        task=self.records('tasks')[0];self.assertEqual(self.payload['next_steps'],task['next_steps'])
        self.assertEqual(1,len(self.records('findings')))

    def test_internal_continuation_prompt_keeps_original_challenge(self):
        self.begin();response=json.loads(self.stop().stdout);before=self.state()[1]
        self.call('native-prompt','--host','codex','--task-id','research',data={
            'session_id':'session-a','hook_event_name':'UserPromptSubmit','prompt':response['reason']})
        self.assertEqual(before,self.state()[1])

    @unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'POSIX shell hook profile')
    def test_real_hooks_request_save_without_application_delta_or_recovery_checkpoint(self):
        import sqlite3
        hook_dir=self.root/'.claude/hooks';hook_dir.mkdir(parents=True)
        for name in ('working-memory-read.sh','working-memory-write.sh'):
            shutil.copy2(ROOT/'PHP Core/.claude/hooks'/name,hook_dir/name)
        self.git('add','.claude');self.git('commit','-qm','Install public hooks')
        def hook(name,payload):
            result=subprocess.run(['bash',str(hook_dir/name)],cwd=self.root,env=self.env,
                                  input=json.dumps(payload).encode(),capture_output=True)
            self.assertEqual(0,result.returncode,result.stderr.decode());return result
        hook('working-memory-read.sh',{'hook_event_name':'UserPromptSubmit','session_id':'session-a',
                                      'prompt':'Inspect the Amber routing policy.'})
        response=hook('working-memory-write.sh',{'hook_event_name':'Stop','session_id':'session-a',
                     'stop_hook_active':False,'last_assistant_message':'Inspected the retry contract.'})
        self.assertEqual('block',json.loads(response.stdout)['decision'])
        self.assertEqual([],self.records('tasks'));self.save();before=self.authority()
        hook('working-memory-write.sh',{'hook_event_name':'Stop','session_id':'session-a',
                                       'stop_hook_active':True,'last_assistant_message':'Saved.'})
        self.assertEqual(before,self.authority())
        with sqlite3.connect(self.root/'memory-bank/local/context.db') as connection:
            self.assertEqual(0,connection.execute('SELECT count(*) FROM turn_deltas').fetchone()[0])

    def test_empty_or_unverified_result_does_not_provision_task(self):
        self.begin();self.stop()
        for data in ({'learnings':[]},{'next_steps':[],'learnings':[]},{**self.payload,'verified':False}):
            result=self.save(data,ok=False);self.assertNotEqual(0,result.returncode)
            self.assertEqual([],self.records('tasks'));self.assertEqual([],self.records('findings'))

    def test_conflicting_learnings_can_be_corrected_in_same_continuation(self):
        self.call('start','--task-id','research','--goal','Inspect routing')
        self.begin();self.stop();before=self.authority()
        original=self.payload['learnings'][0]
        variants=[{**original,'consequence':'The Amber dispatcher rejects unsafe repeated requests.'},
                  {**original,'title':'AMBER  retry boundary'},
                  {**original,'expected_revision':1}]
        for other in variants:
            with self.subTest(other=other):
                invalid=self.save({**self.payload,'learnings':[original,other]},ok=False)
                self.assertNotEqual(0,invalid.returncode)
                self.assertIn(b'Conflicting learning identities',invalid.stderr)
                self.assertEqual(before,self.authority())
                self.assertIsNone(self.state()[1]['digest'])
        self.save()
        self.assertEqual(1,len(self.records('findings')))
        self.assertEqual(1,len(list((self.root/'memory-bank/chunks').glob('*.md'))))

    def test_conflicting_learnings_do_not_provision_or_bind_first_save(self):
        self.begin();self.stop();before=self.authority()
        original=self.payload['learnings'][0]
        conflicting={**original,'consequence':'The Amber dispatcher rejects unsafe repeated requests.'}
        invalid=self.save({**self.payload,'learnings':[original,conflicting]},ok=False)
        self.assertNotEqual(0,invalid.returncode)
        self.assertEqual(before,self.authority())
        self.assertEqual([],self.records('tasks'))
        self.assertIsNone(self.state()[1]['digest'])
        self.save()
        self.assertEqual(1,len(self.records('tasks')))
        self.assertEqual(1,len(self.records('findings')))

    def test_identical_duplicate_learnings_save_and_replay_once(self):
        self.begin();self.stop()
        original=self.payload['learnings'][0]
        data={**self.payload,'learnings':[original,dict(original)]}
        self.save(data)
        self.assertEqual(1,len(self.records('findings')))
        self.assertEqual(1,len(list((self.root/'memory-bank/chunks').glob('*.md'))))
        before=self.authority()
        self.assertTrue(json.loads(self.save(data).stdout)['replayed'])
        self.assertEqual(before,self.authority())

    def test_interrupted_promotion_replay_cannot_duplicate_after_date_change(self):
        self.call('start','--task-id','research','--goal','Inspect routing')
        (self.root/'memory-bank/README.md').write_text('# Public Amber memory\n')
        self.begin();self.stop();candidate,state=self.state()

        class PromotionInterrupted(BaseException):
            pass

        class PriorDay(datetime):
            @classmethod
            def now(cls,tz=None):
                return datetime.now(tz)-timedelta(days=1)

        with self.runtime() as (native,_,brain):
            real_write=brain.atomic_write
            real_apply=brain.apply_promotion
            def write(path,content):
                real_write(path,content)
                if Path(path).parent==self.root/'memory-bank/chunks':
                    raise PromotionInterrupted()
            def apply(*args,**kwargs):
                with mock.patch.object(brain,'datetime',PriorDay):
                    return real_apply(*args,**kwargs)
            with mock.patch.object(brain,'atomic_write',side_effect=write), \
                    mock.patch.object(brain,'apply_promotion',side_effect=apply):
                with self.assertRaises(PromotionInterrupted):
                    native.save(self.root,candidate.stem,state['token'],self.payload,
                        owner='tester',mode='governed',provision=lambda task:None)
        before=self.authority()
        result=json.loads(self.save().stdout)
        self.assertTrue(result['replayed'])
        self.assertEqual([],result['promotion']['promoted'])
        self.assertEqual(1,len(result['promotion']['failed']))
        self.assertIn('near-duplicate',result['promotion']['failed'][0]['reason'])
        self.assertEqual(1,len(list((self.root/'memory-bank/chunks').glob('*.md'))))
        self.assertEqual(before,self.authority())

    def test_foreign_actor_cannot_consume_challenge(self):
        self.begin();self.stop()
        result=self.save(ok=False,env=dict(self.env,PROJECT_BRAIN_OWNER='other'))
        self.assertNotEqual(0,result.returncode);self.assertEqual([],self.records('tasks'))

    def test_newer_task_revision_holds_delayed_result(self):
        self.call('start','--task-id','research','--goal','Inspect routing')
        self.begin();self.stop()
        self.call('update','--task-id','research','--revision','auto','--progress','A newer human plan.')
        before=self.authority();result=self.save(ok=False)
        self.assertNotEqual(0,result.returncode);self.assertEqual(before,self.authority())

    def test_changed_task_uuid_in_local_metadata_cannot_redirect_save(self):
        self.call('start','--task-id','research','--goal','Inspect routing')
        self.call('start','--task-id','other-task','--goal','Other work')
        self.begin();self.stop();candidate,state=self.state()
        other=next(item for item in self.records('tasks') if item['external_id']=='other-task')
        state.update(task_uuid=other['id'],revision=other['revision']);candidate.write_text(json.dumps(state))
        before=self.authority();result=self.save(ok=False)
        self.assertNotEqual(0,result.returncode);self.assertEqual(before,self.authority())

    def test_corrupt_challenge_fields_are_refused_without_authority_writes(self):
        self.begin();self.stop();candidate,valid=self.state()
        for field,value in [('phase',[]),('task_uuid','/outside'),('host',{}),('scope',False),
                            ('task_id','other-task'),('session','wrong'),('updated',float('inf'))]:
            with self.subTest(field=field):
                candidate.write_text(json.dumps({**valid,field:value}))
                result=self.call('native-result','--request',candidate.stem,'--token',valid['token'],
                                 '--input','-',data=self.payload,ok=False)
                self.assertNotEqual(0,result.returncode);self.assertNotIn(b'Traceback',result.stderr)
                self.assertEqual([],self.records('tasks'))
        candidate.write_text(json.dumps(valid))

    def test_checkout_drift_cannot_save_prior_branch_result(self):
        self.begin();self.stop();self.git('checkout','-qb','other-work')
        before=self.authority();result=self.save(ok=False)
        self.assertNotEqual(0,result.returncode);self.assertEqual(before,self.authority())

    def test_adapter_change_invalidates_pending_request(self):
        self.begin();self.stop()
        module=self.root/'memory-bank/scripts/native_results.py'
        with module.open('a') as output:output.write('\n# changed runtime image\n')
        self.assertNotEqual(0,self.save(ok=False).returncode);self.assertEqual([],self.records('tasks'))

    def test_completed_proof_without_outcome_metadata_stays_held(self):
        self.begin();self.stop();self.save();candidate,state=self.state()
        state['phase']='requested';candidate.write_text(json.dumps(state))
        self.stop(active=True);self.assertEqual('held',self.state()[1]['phase'])
        self.assertIn('warning',self.begin())

    def test_expired_request_does_not_disable_future_capture(self):
        self.begin();self.stop();candidate,state=self.state();state['updated']-=2*24*60*60
        candidate.write_text(json.dumps(state));self.assertNotEqual(0,self.save(ok=False).returncode)
        self.assertTrue(self.begin()['armed']);self.assertIn('decision',json.loads(self.stop().stdout))

    def test_missing_and_sensitive_sources_do_not_create_task(self):
        self.begin();self.stop()
        for source in ('missing.md','.env','../outside.md'):
            data={**self.payload,'learnings':[{**self.payload['learnings'][0],'sources':[source]}]}
            self.assertNotEqual(0,self.save(data,ok=False).returncode)
            self.assertEqual([],self.records('tasks'))

    def test_skip_is_explicit_and_creates_no_authoritative_records(self):
        self.begin();self.stop();candidate,state=self.state()
        self.call('native-result','--request',candidate.stem,'--token',state['token'],'--skip','--json')
        self.assertEqual([],self.records('tasks'));self.assertEqual(b'',self.stop(active=True).stdout)
        self.assertEqual('omitted',self.state()[1]['phase'])

    def test_recovery_stop_is_bounded_and_failure_warns_next_prompt(self):
        self.begin();self.stop();self.stop(active=True)
        self.assertEqual('failed',self.state()[1]['phase'])
        result=self.call('native-prompt','--host','codex','--task-id','research',data={
            'session_id':'session-a','hook_event_name':'UserPromptSubmit','prompt':'Continue'})
        self.assertIn(b'not saved',result.stdout);self.assertEqual([],self.records('tasks'))

    def test_cursor_continues_once_and_aborted_stop_never_requests_save(self):
        self.begin('cursor');self.assertIn('followup_message',json.loads(self.stop('cursor').stdout))
        self.assertEqual(b'',self.stop('cursor',active=True).stdout)
        self.begin('cursor');self.assertEqual(b'',self.stop('cursor',status='aborted').stdout)
        self.assertEqual([],self.records('tasks'))

    @unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'POSIX shell hook profile')
    def test_cursor_failed_save_keeps_one_json_prompt_response_and_capsule_notice(self):
        self.begin('cursor');self.stop('cursor');self.stop('cursor',active=True)
        hook_dir=self.root/'.cursor/hooks';hook_dir.mkdir(parents=True)
        shutil.copy2(ROOT/'PHP Core/.cursor/hooks/working-memory-read.sh',hook_dir/'working-memory-read.sh')
        response=subprocess.run(['bash',str(hook_dir/'working-memory-read.sh')],cwd=self.root,env=self.env,
            input=json.dumps({'hook_event_name':'beforeSubmitPrompt','conversation_id':'session-a',
                              'prompt':'Continue the Amber routing research.'}).encode(),capture_output=True)
        self.assertEqual(0,response.returncode,response.stderr.decode())
        self.assertEqual({'continue':True},json.loads(response.stdout))
        report=json.loads((self.root/'memory-bank/local/last-turn-report.json').read_text())
        self.assertIs(report['native_capture_incomplete'],True)
        capsule=self.call('refresh','--task-id','research','--query','Amber','--ephemeral','--json')
        self.assertIn('native memory result not saved completely',json.loads(capsule.stdout)['capsule_text'])

    def test_cursor_wiring_caps_automatic_followup(self):
        for edition in ('PHP Core','Laravel','Symfony','Cms/wordpress'):
            wiring=json.loads((ROOT/edition/'.cursor/hooks.json').read_text())
            entries=[item for item in wiring['hooks']['stop'] if item['command'].endswith('working-memory-write.sh')]
            self.assertEqual(1,len(entries));self.assertEqual(1,entries[0].get('loop_limit'))

    def test_unarmed_or_missing_session_payload_never_requests_save(self):
        self.assertEqual(b'',self.stop().stdout)
        result=self.call('native-stop','--host','codex','--task-id','research',data={
            'hook_event_name':'Stop','last_assistant_message':'Example result in arbitrary text.'})
        self.assertEqual(b'',result.stdout);self.assertEqual([],self.records('tasks'))

    def test_lightweight_and_harness_sessions_do_not_arm(self):
        for values in ({'PROJECT_BRAIN_MODE':'lightweight'},{'CONTEXT_CAPSULE_DELIVERED':'1'},
                       {'CONTEXT_MEMORY_RECOVERY':'1'}):
            result=self.call('native-prompt','--host','codex','--task-id','research','--json',data={
                'hook_event_name':'UserPromptSubmit','session_id':'session-a'},env=dict(self.env,**values))
            self.assertFalse(json.loads(result.stdout)['armed'])
        self.assertEqual([],self.records('tasks'))

    @unittest.skipUnless(hasattr(os,'symlink'),'symlinks unavailable')
    def test_linked_challenge_storage_is_refused_without_external_writes(self):
        outside=self.root.parent/'outside';outside.mkdir()
        local=self.root/'project-brain/local';local.mkdir(exist_ok=True)
        (local/'native-results').symlink_to(outside,target_is_directory=True)
        result=self.call('native-prompt','--host','codex','--task-id','research','--json',data={
            'hook_event_name':'UserPromptSubmit','session_id':'session-a'},ok=False)
        self.assertNotEqual(0,result.returncode);self.assertEqual([],list(outside.iterdir()))


if __name__=='__main__':unittest.main()
