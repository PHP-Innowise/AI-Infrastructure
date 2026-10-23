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
JOURNAL = 'memory-bank/local/native-results.ndjson'
# All a journal line may hold: when, which host, which step, a short code, the
# Stop hook's exit status and two counts - never text, paths or identifiers.
JOURNAL_FIELDS = {'at','host','stage','event','code','status','learnings','promoted','answer'}


class NativeCaptureFixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='native-semantic-capture-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / 'public project'
        self.root.mkdir()
        self.env = dict(os.environ, PROJECT_BRAIN_OWNER='tester', GIT_OPTIONAL_LOCKS='0',
                        PYTHONDONTWRITEBYTECODE='1')
        # A suite run from a Claude Code session inherits that session's marks.
        for key in ('ACCELERATOR_HOME','ACCELERATOR_PROJECT_DIR','ACCELERATOR_STATE_DIR',
                    'CONTEXT_MEMORY_RECOVERY','CONTEXT_CAPSULE_DELIVERED','PROJECT_BRAIN_MODE',
                    'CONTEXT_NATIVE_RESULT_REQUEST','CONTEXT_NATIVE_RESULT_ANSWER',
                    'CLAUDE_CODE_SESSION_ATTENDED','CLAUDE_CODE_ENTRYPOINT'):
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


class NativeCaptureTests(NativeCaptureFixture):
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


class NativeFunnelJournalTests(NativeCaptureFixture):
    """Each native step leaves one metadata line, so the save funnel reads from disk."""

    STOP={'hook_event_name':'Stop','session_id':'session-a','stop_hook_active':False}

    def journal(self):
        path=self.root/JOURNAL
        return [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines()] if path.exists() else []

    def written(self,action):
        before=len(self.journal());action()
        return self.journal()[before:]

    def one(self,lines,stage,event,code=None,host='codex'):
        self.assertEqual(1,len(lines),lines)
        line=lines[0]
        self.assertEqual((host,stage,event,code),(line['host'],line['stage'],line['event'],line.get('code')))
        self.assertLessEqual(set(line),JOURNAL_FIELDS)
        self.assertRegex(line['at'],r'^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$')
        return line

    def raw(self,*args,data):
        return subprocess.run([sys.executable,'-B',str(self.root/'memory-bank/scripts/context.py'),
            '--root',str(self.root),*args],cwd=self.root,env=self.env,input=data,capture_output=True)

    def install_hooks(self,host):
        tree={'claude':'.claude','codex':'.codex','cursor':'.cursor'}[host]
        hooks=self.root/tree/'hooks';hooks.mkdir(parents=True,exist_ok=True)
        for name in ('working-memory-read.sh','working-memory-write.sh'):
            shutil.copy2(ROOT/'PHP Core'/tree/'hooks'/name,hooks/name)
        self.git('add',tree);self.git('commit','-qm','Install public '+host+' hooks')
        return hooks

    def hook(self,hooks,name,payload,env=None):
        result=subprocess.run(['bash',str(hooks/name)],cwd=self.root,env=env or self.env,
                              input=json.dumps(payload).encode(),capture_output=True)
        self.assertEqual(0,result.returncode,result.stderr.decode());return result

    def test_each_prompt_path_writes_one_line_with_its_code(self):
        prompt={'hook_event_name':'UserPromptSubmit','session_id':'session-a','prompt':'Inspect the retry policy.'}
        def arm(payload=prompt,**values):
            return self.call('native-prompt','--host','codex','--task-id','research','--json',
                             data=payload,env=dict(self.env,**values))
        for values,code in (({'PROJECT_BRAIN_MODE':'lightweight'},'not-governed'),
                            ({'CONTEXT_CAPSULE_DELIVERED':'1'},'capsule-delivered'),
                            ({'CONTEXT_MEMORY_RECOVERY':'1'},'recovery-session')):
            with self.subTest(code=code):
                self.one(self.written(lambda:arm(**values)),'prompt','skipped',code)
        self.one(self.written(lambda:arm({'hook_event_name':'UserPromptSubmit'})),'prompt','skipped','no-session-id')
        self.one(self.written(self.begin),'prompt','armed')
        reason=json.loads(self.stop().stdout)['reason']
        self.one(self.written(lambda:arm({**prompt,'prompt':reason})),'prompt','continuation')
        self.stop(active=True)
        # The warning the prompt hook prints, counted by what the last request became.
        self.one(self.written(self.begin),'prompt','armed','previous-failed')

    def test_each_stop_path_writes_one_line_with_its_code(self):
        def request(payload=self.STOP,host='codex',**values):
            return self.call('native-stop','--host',host,'--task-id','research',data=payload,
                             env=dict(self.env,**values))
        for values,code in (({'PROJECT_BRAIN_MODE':'lightweight'},'not-governed'),
                            ({'CONTEXT_CAPSULE_DELIVERED':'1'},'capsule-delivered'),
                            ({'CONTEXT_MEMORY_RECOVERY':'1'},'recovery-session')):
            with self.subTest(code=code):
                self.one(self.written(lambda:request(**values)),'stop','skipped',code)
        for payload,code in (({'hook_event_name':'Stop'},'no-session-id'),
                             ({**self.STOP,'hook_event_name':'SubagentStop'},'not-stop-event'),
                             (self.STOP,'not-armed')):
            with self.subTest(code=code):
                self.one(self.written(lambda:request(payload)),'stop','skipped',code)
        aborted={'conversation_id':'session-a','hook_event_name':'stop','status':'aborted','loop_count':0}
        self.one(self.written(lambda:request(aborted,'cursor')),'stop','skipped','not-completed',host='cursor')
        self.begin()
        self.one(self.written(request),'stop','requested')
        self.one(self.written(request),'stop','skipped','phase-requested')
        recovery={**self.STOP,'stop_hook_active':True}
        self.one(self.written(lambda:request(recovery)),'stop','failed','unsaved')
        self.one(self.written(lambda:request(recovery)),'stop','skipped','recovery-failed')

    def test_a_save_that_began_and_stopped_fails_as_incomplete(self):
        lines=self.written(self.interrupt_save)
        self.assertEqual([('prompt','armed',None),('stop','requested',None),('save','error','OSError'),
                          ('stop','failed','incomplete')],
                         [(line['stage'],line['event'],line.get('code')) for line in lines])

    def test_a_finished_save_the_state_never_took_is_held_unconfirmed(self):
        self.begin();self.stop();self.save();candidate,state=self.state()
        state['phase']='requested';candidate.write_text(json.dumps(state))
        self.one(self.written(lambda:self.stop(active=True)),'stop','held','unconfirmed')

    def test_a_closed_task_is_not_asked_for_a_save(self):
        self.call('start','--task-id','research','--goal','Inspect routing')
        with self.runtime() as (_,_,brain):
            task=brain.find_record(self.root,'research',record_type='task')[1]
            brain.update_record(self.root,task['id'],expected_revision=task['revision'],
                progress=None,next_steps=[],files=[],sources=[],actor='tester',
                transition_to='completed',reason='Public fixture finished')
        self.begin()
        self.one(self.written(self.stop),'stop','skipped','task-completed')

    def test_each_refused_save_writes_one_line_with_its_code(self):
        self.begin();self.stop();candidate,state=self.state()
        learning=self.payload['learnings'][0]
        cases=[
            ({**self.payload,'learnings':[{**learning,'sources':['missing.md']}]},
             'memory-result-v1:source_missing','Source is missing or is not a regular file'),
            ({**self.payload,'learnings':[{**learning,'sources':['.env']}]},
             'memory-result-v1:source_path','source paths are refused'),
            ({**self.payload,'learnings':[{**learning,'sources':[]}]},'memory-result-v1:no_sources','Cite sources'),
            ({**self.payload,'verified':False},'memory-result-v1:unverified','verified=true'),
            # The likeliest slip: verified inside the learning instead of beside it.
            ({'progress':'Checked the retry rule.','learnings':[{**learning,'verified':True}]},
             'memory-result-v1:learning_fields','Each learning needs type, title, consequence and sources'),
            ({**self.payload,'learnings':[{**learning,'type':'rule'}]},'memory-result-v1:learning_type','finding or decision'),
            ({**self.payload,'learnings':[learning,{**learning,'consequence':'The Amber dispatcher retries nothing.'}]},
             'memory-result-v1:conflicting_learnings','Conflicting learning identities'),
            ({**self.payload,'summary':'Inspected.'},'memory-result-v1:schema','Unknown result fields'),
            ({**self.payload,'progress':'x'*1001},'memory-result-v1:text_limit','within 1000 characters'),
            ({**self.payload,'progress':'stderr: the dispatcher failed'},'memory-result-v1:raw_text','pasted log'),
            ({**self.payload,'progress':'Asked fixture.owner@example.invalid about retries.'},
             'memory-result-v1:sensitive','personal data'),
            ({'learnings':[]},'memory-result-v1:nothing','Nothing to record'),
            ({'next_steps':[],'learnings':[]},'native-result:nothing-to-record','native-result:nothing-to-record'),
        ]
        for data,code,text in cases:
            with self.subTest(code=code):
                results=[]
                self.one(self.written(lambda:results.append(self.save(data,ok=False))),'save','refused',code)
                self.assertNotEqual(0,results[0].returncode)
                # The agent still reads the same correctable text.
                self.assertIn(text.encode(),results[0].stderr)
        for token,data,code in ((state['token'],b'{"progress": "unterminated','native-result:invalid-input'),
                                ('0'*32,json.dumps(self.payload).encode(),'native-result:request-unavailable'),
                                ('not-a-token',json.dumps(self.payload).encode(),'native-result:invalid-token')):
            with self.subTest(code=code):
                results=[]
                self.one(self.written(lambda:results.append(self.raw('native-result','--request',candidate.stem,
                    '--token',token,'--input','-','--json',data=data))),'save','refused',code)
                self.assertNotEqual(0,results[0].returncode)
        self.assertEqual([],self.records('tasks'))
        self.assertEqual('requested',self.state()[1]['phase'])

    def test_saved_held_and_omitted_lines_count_learnings_and_chunks_only(self):
        self.begin();self.stop()
        line=self.one(self.written(self.save),'save','saved')
        self.assertEqual((1,1),(line['learnings'],line['promoted']))
        line=self.one(self.written(self.save),'save','saved','repeat')
        self.assertEqual((0,0),(line['learnings'],line['promoted']))
        self.begin();self.stop();candidate,state=self.state()
        self.one(self.written(lambda:self.call('native-result','--request',candidate.stem,'--token',state['token'],
                                              '--skip','--json')),'save','omitted')
        self.begin();self.stop()
        changed={**self.payload['learnings'][0],'consequence':'The Amber dispatcher retries no request.',
                 'expected_revision':99}
        line=self.one(self.written(lambda:self.save({**self.payload,'learnings':[changed]})),'save','held')
        self.assertEqual((0,0),(line['learnings'],line['promoted']))

    def test_a_save_after_a_failed_stop_is_counted_late(self):
        self.interrupt_save()
        self.one(self.written(self.save),'save','saved','late')

    def test_journal_lines_are_metadata_only(self):
        marker='Zircon'
        prompt={'hook_event_name':'UserPromptSubmit','session_id':'session-'+marker,'prompt':marker+' routing question'}
        stop={'hook_event_name':'Stop','session_id':'session-'+marker,'stop_hook_active':False,
              'last_assistant_message':marker+' answer'}
        self.call('native-prompt','--host','claude','--task-id','research',data=prompt)
        self.call('native-stop','--host','claude','--task-id','research',data=stop)
        candidate,state=self.state()
        learning={**self.payload['learnings'][0],'title':marker+' retry boundary'}
        save=lambda data,ok=True:self.call('native-result','--request',candidate.stem,'--token',state['token'],
                                           '--input','-','--json',data=data,ok=ok)
        save({**self.payload,'learnings':[{**learning,'sources':[marker+'.md']}]},ok=False)
        save({**self.payload,'progress':marker+' progress','learnings':[learning]})
        self.call('native-stop','--host','claude','--task-id','research',data={**stop,'stop_hook_active':True})
        lines=self.journal()
        self.assertEqual([('prompt','armed'),('stop','requested'),('save','refused'),('save','saved'),('stop','skipped')],
                         [(line['stage'],line['event']) for line in lines])
        text=(self.root/JOURNAL).read_text(encoding='utf-8')
        for content in (marker,state['token'],candidate.stem,str(self.root),self.root.name,'rules.md','research',
                        'tester',learning['consequence']):
            self.assertNotIn(content,text)
        for line in lines:
            self.assertLessEqual(set(line),JOURNAL_FIELDS)
            for field in ('stage','event','code'):
                if field in line:self.assertRegex(line[field],r'^[a-z0-9][a-z0-9_.:-]{0,79}$')
            if 'answer' in line:self.assertIn(line['answer'],{'brief','repeat'})

    def test_journal_keeps_a_bounded_window_of_the_newest_lines(self):
        config_file=self.root/'project-brain/config/runtime.json'
        config=json.loads(config_file.read_text());config['refresh_health_retention']=1
        config_file.write_text(json.dumps(config))
        with self.runtime() as (native,_,_):
            for _ in range(60):
                native.begin(self.root,'research','codex',{'hook_event_name':'UserPromptSubmit'},
                             owner='tester',mode='governed')
            native.begin(self.root,'research','codex',{'hook_event_name':'UserPromptSubmit','session_id':'session-a'},
                         owner='tester',mode='governed')
        lines=self.journal()
        # Eight journal lines per refresh-health record, never fewer.
        self.assertGreaterEqual(len(lines),8)
        self.assertLess(len(lines),40)
        self.assertEqual('armed',lines[-1]['event'])

    @unittest.skipUnless(hasattr(os,'symlink'),'symlinks unavailable')
    def test_a_linked_journal_is_never_written_through(self):
        outside=self.root.parent/'outside.ndjson';outside.write_text('')
        local=self.root/'memory-bank/local';local.mkdir(parents=True,exist_ok=True)
        try:(local/'native-results.ndjson').symlink_to(outside)
        except OSError:self.skipTest('symbolic links need a privilege here')
        self.assertTrue(self.begin()['armed'])
        self.assertEqual('',outside.read_text())

    @unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'POSIX shell hook profile')
    def test_stop_hook_records_its_save_request_exit_for_each_host(self):
        prompt={'hook_event_name':'UserPromptSubmit','session_id':'session-a','prompt':'Inspect the Amber routing policy.'}
        payloads={'claude':(prompt,self.STOP),'codex':(prompt,self.STOP),
                  'cursor':({'hook_event_name':'beforeSubmitPrompt','conversation_id':'session-a',
                             'prompt':'Inspect the Amber routing policy.'},
                            {'hook_event_name':'stop','conversation_id':'session-a','status':'completed','loop_count':0})}
        for host,(asked,stopped) in payloads.items():
            with self.subTest(host=host):
                hooks=self.install_hooks(host)
                self.hook(hooks,'working-memory-read.sh',asked)
                lines=self.written(lambda:self.hook(hooks,'working-memory-write.sh',stopped))
                self.assertEqual([('stop','requested',None),('stop-hook','exited',0)],
                                 [(line['stage'],line['event'],line.get('status')) for line in lines])
                self.assertEqual({host},{line['host'] for line in lines})
                self.assertLessEqual(set(lines[-1]),JOURNAL_FIELDS)

    @unittest.skipUnless(os.name=='posix' and shutil.which('bash') and shutil.which('timeout'),
                         'POSIX shell hook profile with timeout')
    def test_stop_hook_records_a_failed_killed_or_skipped_save_request(self):
        hooks=self.install_hooks('claude')
        # The runtime stand-in: the checkpoint succeeds, the save request fails or hangs.
        (self.root/'memory-bank/scripts/context_cli.py').write_text(
            'import os, sys, time\n'
            'if "native-stop" in sys.argv:\n'
            '    if os.environ.get("FIXTURE_NATIVE_STOP") == "hang":\n'
            '        time.sleep(30)\n'
            '    sys.exit(3)\n')
        for mode,budget,expected in (('fail','5',('exited',3,None)),('hang','1',('exited',124,None)),
                                     ('skip','5',('skipped',None,'capsule-delivered'))):
            with self.subTest(mode=mode):
                env=dict(self.env,FIXTURE_NATIVE_STOP=mode,CONTEXT_HOOK_BUDGET=budget)
                if mode=='skip':env['CONTEXT_CAPSULE_DELIVERED']='1'
                results=[]
                lines=self.written(lambda:results.append(self.hook(hooks,'working-memory-write.sh',self.STOP,env=env)))
                self.assertEqual(b'',results[0].stdout)
                self.assertEqual([('stop-hook',*expected)],
                                 [(line['stage'],line['event'],line.get('status'),line.get('code')) for line in lines])
                self.assertEqual('claude',lines[0]['host'])

    @unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'POSIX shell hook profile')
    def test_stop_hook_never_appends_through_a_linked_journal(self):
        hooks=self.install_hooks('claude')
        outside=self.root.parent/'outside.ndjson';outside.write_text('')
        local=self.root/'memory-bank/local';local.mkdir(parents=True,exist_ok=True)
        (local/'native-results.ndjson').symlink_to(outside)
        self.hook(hooks,'working-memory-write.sh',self.STOP)
        self.assertEqual('',outside.read_text())


    @unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'POSIX shell hook profile')
    def test_an_unsaved_result_notice_reaches_each_host_by_one_fixed_path(self):
        """Claude Code's and Codex's prompt hooks arm the next request beside
        the refresh: their notice is the hook's own first line, and nothing
        the refresh reads waits on the arm. Cursor's hook answers in JSON
        only, so its arm writes the notice for the capsule and runs first."""
        notice='Previous native memory result was not saved completely.'
        report=self.root/'memory-bank/local/last-turn-report.json'
        for host,session in (('claude','unsaved-claude'),('codex','unsaved-codex')):
            with self.subTest(host=host):
                hooks=self.install_hooks(host)
                self.begin(host,session);self.stop(host,session);self.stop(host,session,active=True)
                if report.exists():report.unlink()
                result=self.hook(hooks,'working-memory-read.sh',{'hook_event_name':'UserPromptSubmit',
                    'session_id':session,'cwd':str(self.root),'prompt':'Continue the Amber routing research.'})
                lines=result.stdout.decode().splitlines()
                self.assertEqual(notice,lines[0])
                self.assertEqual(1,lines.count(notice))
                self.assertFalse(report.exists() and 'native_capture_incomplete' in report.read_text())
                self.assertNotIn('native memory result not saved completely',result.stdout.decode())
                self.assertFalse(list(Path(self.env.get('TMPDIR') or tempfile.gettempdir()).glob('working-memory-native.*')))
        hooks=self.install_hooks('cursor')
        self.begin('cursor','unsaved-cursor');self.stop('cursor','unsaved-cursor');self.stop('cursor','unsaved-cursor',active=True)
        result=self.hook(hooks,'working-memory-read.sh',{'hook_event_name':'beforeSubmitPrompt',
            'conversation_id':'unsaved-cursor','prompt':'Continue the Amber routing research.'})
        self.assertEqual({'continue':True},json.loads(result.stdout))
        rule=(self.root/'.cursor/rules/working-memory.mdc').read_text()
        self.assertIn('native memory result not saved completely',rule)


class NativeRequestSwitchTests(NativeCaptureFixture):
    """`native_result_request: off` arms no prompt and asks no Stop; the
    default asks after every answer, as before. The request costs one more
    model call per answer, which a team may not want on every turn."""

    def set_request(self,value):
        cfg_file=self.root/'project-brain/config/runtime.json'
        cfg=json.loads(cfg_file.read_text());cfg['native_result_request']=value
        cfg_file.write_text(json.dumps(cfg))

    def journal(self):
        path=self.root/JOURNAL
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def test_switched_off_no_prompt_arms_and_no_stop_asks(self):
        self.set_request('off')
        for host in ('codex','claude','cursor'):
            with self.subTest(host=host):
                self.assertFalse(self.begin(host=host,session='off-'+host)['armed'])
                self.assertEqual(b'',self.stop(host=host,session='off-'+host).stdout.strip())
        lines=[(line['stage'],line['event'],line.get('code')) for line in self.journal()]
        self.assertEqual(3,lines.count(('prompt','skipped','switched-off')))
        self.assertEqual(3,lines.count(('stop','skipped','switched-off')))
        self.assertFalse(list((self.root/'project-brain/local/native-results').glob('*.json')))

    def test_a_prompt_armed_before_the_switch_went_off_asks_nothing(self):
        self.assertTrue(self.begin()['armed'])
        self.set_request('off')
        self.assertEqual(b'',self.stop().stdout.strip())
        self.assertIn(('stop','skipped','switched-off'),
                      [(line['stage'],line['event'],line.get('code')) for line in self.journal()])

    def test_the_variable_overrides_the_setting_and_an_unknown_value_is_refused(self):
        self.set_request('off')
        prompt={'hook_event_name':'UserPromptSubmit','session_id':'override','prompt':'Inspect the retry policy.'}
        armed=self.call('native-prompt','--host','codex','--task-id','research','--json',data=prompt,
                        env=dict(self.env,CONTEXT_NATIVE_RESULT_REQUEST='every-turn'))
        self.assertTrue(json.loads(armed.stdout)['armed'])
        self.set_request('sometimes')
        refused=self.call('native-prompt','--host','codex','--task-id','research','--json',data=prompt,ok=False)
        self.assertNotEqual(0,refused.returncode)
        self.assertIn('native-result:invalid-request-setting',refused.stderr.decode())
        self.assertIn(('prompt','error','native-result:invalid-request-setting'),
                      [(line['stage'],line['event'],line.get('code')) for line in self.journal()])



class NativeAnswerTests(NativeCaptureFixture):
    """A session a person reads as it goes is not asked to return its answer
    again after the save: the answer is on screen above the continuation, and
    the repeat doubled it. A headless run - the final message is all its
    caller gets - and any session the host does not mark keep the repeat."""

    BRIEF='The person has already read your answer above this continuation'
    REPEAT='return the substantive answer to the original user request'

    def journal(self):
        path=self.root/JOURNAL
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def ask(self,host='claude',session='answer',env=None,transcript=None):
        prompt={'hook_event_name':'UserPromptSubmit','session_id':session,'prompt':'Inspect the retry policy.'}
        stop={'hook_event_name':'Stop','session_id':session,'stop_hook_active':False,
              'last_assistant_message':'The Amber dispatcher retries only idempotent requests.'}
        if host=='cursor':
            prompt={'hook_event_name':'beforeSubmitPrompt','conversation_id':session}
            stop={'conversation_id':session,'hook_event_name':'stop','status':'completed','loop_count':0}
        if transcript is not None:stop['transcript_path']=str(transcript)
        environment=dict(self.env,**(env or {}))
        self.call('native-prompt','--host',host,'--task-id','research','--json',data=prompt,env=environment)
        reply=self.call('native-stop','--host',host,'--task-id','research',data=stop,env=environment)
        reason=json.loads(reply.stdout)
        return reason.get('followup_message') if host=='cursor' else reason['reason']

    def rollout(self,name,source,pad=0):
        path=self.root.parent/name
        meta={'type':'session_meta','payload':{'id':'0','source':source,'base_instructions':'x'*pad}}
        path.write_text(json.dumps(meta)+'\n'+json.dumps({'type':'turn_context'})+'\n')
        return path

    def test_an_attended_claude_session_is_not_asked_to_repeat_its_answer(self):
        for entrypoint in ('cli','claude-desktop','claude-vscode'):
            with self.subTest(entrypoint=entrypoint):
                reason=self.ask(session='attended-'+entrypoint,
                                env={'CLAUDE_CODE_SESSION_ATTENDED':'1','CLAUDE_CODE_ENTRYPOINT':entrypoint})
                self.assertIn(self.BRIEF,reason)
                self.assertNotIn(self.REPEAT,reason)
                # The save itself is asked for exactly as before.
                self.assertIn('native-result --request',reason)
        answers=[line.get('answer') for line in self.journal() if line['stage']=='stop' and line['event']=='requested']
        self.assertEqual(['brief']*3,answers)

    def test_a_headless_child_or_unmarked_claude_run_repeats_its_answer(self):
        cases={'child of a desktop session':{'CLAUDE_CODE_SESSION_ATTENDED':'0','CLAUDE_CODE_ENTRYPOINT':'claude-desktop'},
               'claude -p':{'CLAUDE_CODE_SESSION_ATTENDED':'0','CLAUDE_CODE_ENTRYPOINT':'sdk-cli'},
               'slack':{'CLAUDE_CODE_SESSION_ATTENDED':'1','CLAUDE_CODE_ENTRYPOINT':'claude-in-slack'},
               'no marks':{}}
        for n,(name,env) in enumerate(cases.items()):
            with self.subTest(case=name):
                reason=self.ask(session=f'headless-{n}',env=env)
                self.assertIn(self.REPEAT,reason)
                self.assertNotIn(self.BRIEF,reason)
        answers=[line.get('answer') for line in self.journal() if line['stage']=='stop' and line['event']=='requested']
        self.assertEqual(['repeat']*4,answers)

    def test_a_codex_session_ends_briefly_only_when_its_rollout_says_it_is_watched(self):
        cases={'app or terminal':(self.rollout('app.jsonl','vscode'),True),
               'older terminal':(self.rollout('cli.jsonl','cli'),True),
               'codex exec':(self.rollout('exec.jsonl','exec'),False),
               'subagent':(self.rollout('sub.jsonl',{'subagent':{'thread_spawn':{'depth':1}}}),False),
               'first line past the bound':(self.rollout('long.jsonl','vscode',pad=300*1024),False),
               'missing rollout':(self.root.parent/'gone.jsonl',False),
               'no rollout':(None,False)}
        for n,(name,(path,brief)) in enumerate(cases.items()):
            with self.subTest(case=name):
                reason=self.ask(host='codex',session=f'codex-{n}',transcript=path)
                self.assertIn(self.BRIEF if brief else self.REPEAT,reason)
                self.assertNotIn(self.REPEAT if brief else self.BRIEF,reason)

    def test_the_setting_decides_and_an_unknown_value_keeps_the_save(self):
        cfg_file=self.root/'project-brain/config/runtime.json'
        def set_answer(value):
            cfg=json.loads(cfg_file.read_text());cfg['native_result_answer']=value
            cfg_file.write_text(json.dumps(cfg))
        attended={'CLAUDE_CODE_SESSION_ATTENDED':'1','CLAUDE_CODE_ENTRYPOINT':'cli'}
        set_answer('repeat')
        self.assertIn(self.REPEAT,self.ask(session='set-repeat',env=attended))
        set_answer('brief')
        self.assertIn(self.BRIEF,self.ask(host='cursor',session='set-brief'))
        self.assertIn(self.REPEAT,self.ask(session='env-repeat',env=dict(attended,CONTEXT_NATIVE_RESULT_ANSWER='repeat')))
        set_answer('short')
        reason=self.ask(session='set-unknown',env=attended)
        self.assertIn(self.REPEAT,reason)
        self.assertIn('native-result --request',reason)
        last=[line for line in self.journal() if line['stage']=='stop'][-1]
        self.assertEqual(('requested','native-result:invalid-answer-setting','repeat'),
                         (last['event'],last.get('code'),last.get('answer')))


if __name__=='__main__':unittest.main()
