"""Automatic native Stop continuation and guarded semantic-result saves."""
import contextlib
from datetime import datetime, timedelta
import hashlib
import importlib
import json
import os
from pathlib import Path
import re
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


def requested(host,output):
    """The save step a Stop hook's output asks for, as each host carries it."""
    if host=='claude':return output['hookSpecificOutput']['additionalContext']
    return output['followup_message'] if host=='cursor' else output['reason']


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
        output=json.loads(response.stdout)
        self.assertNotIn('decision',output)
        self.assertIn('native-result --request',requested('claude',output))
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
        # A save made more than ten minutes after its request - spooled, say,
        # until the session's next hook - yields to what was written since.
        # One made within them is recorded where the task now is
        # (NativeFunnelJournalTests).
        self.call('start','--task-id','research','--goal','Inspect routing')
        self.begin();self.stop()
        self.call('update','--task-id','research','--revision','auto','--progress','A newer human plan.')
        candidate,state=self.state();state['updated']-=11*60;candidate.write_text(json.dumps(state))
        before=self.authority();result=self.save(ok=False)
        self.assertNotEqual(0,result.returncode);self.assertEqual(before,self.authority())
        self.assertIn(b'memory-result-v1:prepared_stale_no_write',result.stderr)

    def test_a_task_created_since_a_delayed_request_holds_its_result(self):
        self.begin();self.stop()
        self.call('start','--task-id','research','--goal','Inspect routing')
        candidate,state=self.state();state['updated']-=11*60;candidate.write_text(json.dumps(state))
        before=self.authority();result=self.save(ok=False)
        self.assertIn(b'native-result:task-changed: the branch task was created',result.stderr)
        self.assertEqual(before,self.authority())

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

    def test_a_commit_while_the_save_is_written_does_not_refuse_it(self):
        # The person, or another session of the branch, commits while the
        # agent writes the save: the branch and the work are the same, and
        # the save was refused as scope-changed, learnings and all.
        self.begin();self.stop()
        (self.root/'notes.md').write_text('# Notes\n');self.git('add','notes.md');self.git('commit','-qm','Notes')
        saved=json.loads(self.save().stdout)
        self.assertEqual(['created'],[item['state'] for item in saved['records']])

    def test_a_detached_head_moved_since_the_request_refuses_the_save(self):
        # Detached - in the middle of a rebase - the commit is all a save
        # can be pinned to.
        self.git('checkout','-q','--detach')
        self.begin();self.stop()
        (self.root/'notes.md').write_text('# Notes\n');self.git('add','notes.md');self.git('commit','-qm','Notes')
        before=self.authority();result=self.save(ok=False)
        self.assertIn(b'native-result:scope-changed',result.stderr);self.assertEqual(before,self.authority())

    def test_a_commit_in_a_worktree_codex_made_does_not_refuse_the_save(self):
        # Codex's own worktree stays on a detached HEAD, where the thread that
        # owns it names the task: a commit there moves HEAD and leaves the
        # task as it was. Pinned to the commit, the save was refused.
        self.git('checkout','-q','--detach')
        (self.root/'.git'/'codex-thread.json').write_text(json.dumps(
            {'version':1,'ownerThreadId':'0a664c1a-8a2f-50fc-9fe2-8f660527ee0c'}))
        self.begin();self.stop()
        (self.root/'notes.md').write_text('# Notes\n');self.git('add','notes.md');self.git('commit','-qm','Notes')
        saved=json.loads(self.save().stdout)
        self.assertEqual(['created'],[item['state'] for item in saved['records']])

    def test_a_worktree_codex_handed_to_another_thread_refuses_the_save(self):
        self.git('checkout','-q','--detach')
        record=self.root/'.git'/'codex-thread.json'
        record.write_text(json.dumps({'version':1,'ownerThreadId':'0a664c1a-8a2f-50fc-9fe2-8f660527ee0c'}))
        self.begin();self.stop()
        record.write_text(json.dumps({'version':1,'ownerThreadId':'6329976c-f7d5-d370-c1ae-1282d898694d'}))
        before=self.authority();result=self.save(ok=False)
        self.assertIn(b'native-result:scope-changed',result.stderr);self.assertEqual(before,self.authority())

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

    def test_a_state_this_runtime_cannot_read_is_replaced_at_the_next_prompt(self):
        # Written by another version of the runtime or cut short, the state
        # refused every later prompt of its session, which saved nothing more.
        def prompt(session,ok=True):
            return self.call('native-prompt','--host','codex','--task-id','research','--json',ok=ok,
                             data={'hook_event_name':'UserPromptSubmit','session_id':session,'prompt':'Inspect it.'})
        damages={'another version':lambda path:path.write_text(json.dumps(dict(json.loads(path.read_text()),phase='queued'))),
                 'cut short':lambda path:path.write_bytes(path.read_bytes()[:40])}
        for n,(name,damage) in enumerate(damages.items()):
            with self.subTest(state=name):
                session=f'unreadable-{n}';stop=dict(self.STOP,session_id=session)
                key=json.loads(prompt(session).stdout)['request']
                path=self.root/'project-brain/local/native-results'/(key+'.json')
                damage(path)
                self.one(self.written(lambda:self.call('native-stop','--host','codex','--task-id','research',
                                                       data=stop,ok=False)),'stop','error','native-result:invalid-state')
                self.one(self.written(lambda:prompt(session)),'prompt','armed','previous-unreadable')
                self.assertIn('INTERNAL_NATIVE_MEMORY_RESULT',json.loads(
                    self.call('native-stop','--host','codex','--task-id','research',data=stop).stdout)['reason'])
                token=json.loads(path.read_text())['token']
                self.one(self.written(lambda:self.call('native-result','--request',key,'--token',token,
                                                       '--input','-','--json',data=self.payload)),'save','saved')
        # Storage that is not a plain file is still refused, never replaced.
        key=json.loads(prompt('unsafe').stdout)['request']
        path=self.root/'project-brain/local/native-results'/(key+'.json')
        path.unlink();path.mkdir()
        self.one(self.written(lambda:prompt('unsafe',ok=False)),'prompt','error','native-result:unsafe-storage')
        self.assertTrue(path.is_dir())

    def queued(self,host='claude',text='And how are credit notes numbered?',transcript=None):
        payload={'hook_event_name':'UserPromptSubmit','session_id':'session-a','prompt':text}
        if transcript: payload['transcript_path']=transcript
        return self.call('native-prompt','--host',host,'--task-id','research','--json',data=payload)

    def stopped(self,active,host='claude'):
        return self.call('native-stop','--host',host,'--task-id','research',data=dict(self.STOP,stop_hook_active=active))

    def test_a_prompt_sent_while_the_save_is_written_does_not_cancel_it(self):
        # Claude Code queues a prompt sent while the agent writes the save and
        # runs the prompt hook at once, mid-turn. Offline, re-arming there
        # refused that save on its way ("request-unavailable") and warned the
        # agent of an unsaved result in the middle of saving it.
        self.begin(host='claude')
        self.one(self.written(lambda:self.stopped(False)),'stop','requested',host='claude')
        candidate,state=self.state()
        out=[]
        self.one(self.written(lambda:out.append(self.queued())),'prompt','armed','previous-in-flight',host='claude')
        self.assertNotIn('warning',json.loads(out[0].stdout))
        self.one(self.written(lambda:self.call('native-result','--request',candidate.stem,'--token',state['token'],
                                               '--input','-','--json',data=self.payload)),'save','saved','in-flight',
                 host='claude')
        self.one(self.written(lambda:self.stopped(True)),'stop','skipped','recovery-saved',host='claude')
        self.one(self.written(lambda:out.append(self.queued(text='Next question'))),'prompt','armed',host='claude')
        self.assertNotIn('warning',json.loads(out[1].stdout))
        self.state()  # the request moved aside went with the prompt after
        self.assertEqual(1,len(self.records('tasks')))

    def test_a_request_moved_aside_is_reported_at_the_next_prompt_when_unsaved(self):
        self.begin(host='claude');self.stopped(False)
        self.one(self.written(self.queued),'prompt','armed','previous-in-flight',host='claude')
        # The continuation ended without the save: its Stop settles the request moved aside.
        self.one(self.written(lambda:self.stopped(True)),'stop','failed','unsaved',host='claude')
        out=[]
        self.one(self.written(lambda:out.append(self.queued(text='Next question'))),'prompt','armed','previous-failed',
                 host='claude')
        self.assertEqual('Previous native memory result was not saved completely.',json.loads(out[0].stdout)['warning'])
        # A person who answered No to the save moved aside: not asked again for a day.
        self.stopped(False)
        token=self.state()[1]['token']
        self.queued()
        refused=self.refused_transcript(token,'user-rejected')
        self.one(self.written(lambda:self.queued(text='Again',transcript=refused)),'prompt','skipped','host-refused',
                 host='claude')
        self.one(self.written(lambda:self.stopped(False)),'stop','skipped','phase-declined',host='claude')

    def test_a_request_asked_long_ago_is_reported_at_once_and_codex_continues_one_moved_aside(self):
        self.begin(host='claude');self.stopped(False)
        candidate,state=self.state();state['updated']-=11*60;candidate.write_text(json.dumps(state))
        self.one(self.written(self.queued),'prompt','armed','previous-requested',host='claude')
        refused=self.call('native-result','--request',candidate.stem,'--token',state['token'],'--input','-','--json',
                          data=self.payload,ok=False)
        self.assertIn(b'native-result:request-unavailable',refused.stderr)
        # Codex hands the save step back as a prompt: one carrying the token of
        # the request moved aside is that continuation, not a new prompt.
        self.begin(host='codex',session='session-c')
        reason=json.loads(self.call('native-stop','--host','codex','--task-id','research',
                                    data=dict(self.STOP,session_id='session-c')).stdout)['reason']
        prompt={'hook_event_name':'UserPromptSubmit','session_id':'session-c','prompt':'Steer: also the credit notes.'}
        self.one(self.written(lambda:self.call('native-prompt','--host','codex','--task-id','research','--json',
                                               data=prompt)),'prompt','armed','previous-in-flight')
        self.one(self.written(lambda:self.call('native-prompt','--host','codex','--task-id','research','--json',
                                               data=dict(prompt,prompt=reason))),'prompt','continuation')

    def readonly(self):
        """The project as a sandbox that writes only below a subdirectory
        sees it: the Brain and the runtime's local state cannot be written."""
        modes={}
        for top in (self.root/'project-brain',self.root/'memory-bank/local'):
            for path in [top,*top.rglob('*')]:
                modes[path]=path.stat().st_mode;path.chmod(modes[path]&~0o222)
        return modes

    @unittest.skipUnless(os.name=='posix' and os.geteuid()!=0,'permission bits do not stop root')
    def test_a_save_the_sandbox_cannot_write_is_made_by_the_next_hook(self):
        # Codex started in a subdirectory mounts the project read-only: the
        # save failed with a read-only file system after every answer.
        spool=Path(tempfile.mkdtemp(prefix='native-spool-',dir=self.root.parent))
        env=dict(self.env,TMPDIR=str(spool))
        def prompt():
            return self.call('native-prompt','--host','codex','--task-id','research','--json',env=env,
                             data={'hook_event_name':'UserPromptSubmit','session_id':'session-a','prompt':'Inspect it.'})
        def stopped(active):
            return self.call('native-stop','--host','codex','--task-id','research',env=env,
                             data=dict(self.STOP,stop_hook_active=active))
        for case,settle in (("the continuation's Stop",lambda:stopped(True)),('the next prompt',prompt)):
            with self.subTest(case=case):
                prompt();stopped(False)
                candidate,state=self.state()
                def save(data):
                    return self.call('native-result','--request',candidate.stem,'--token',state['token'],
                                     '--input','-','--json',data=data,env=env,ok=False)
                modes=self.readonly()
                try:
                    # A malformed result is refused while the agent can still correct it.
                    refused=save({'progress':'Inspected.','learnings':[{'type':'finding'}]})
                    self.assertEqual(1,refused.returncode);self.assertIn(b'learning',refused.stderr)
                    spooled=save(self.payload)
                    self.assertEqual(0,spooled.returncode,spooled.stderr.decode())
                    self.assertEqual('spooled',json.loads(spooled.stdout)['state'])
                finally:
                    for path,mode in modes.items():path.chmod(mode)
                self.assertEqual(1,len(list(spool.rglob('*.json'))))
                lines=[(line['stage'],line['event'],line.get('code')) for line in self.written(settle)]
                self.assertEqual(('save','saved','spooled'),lines[0])
                self.assertEqual([],list(spool.rglob('*.json')))
                self.assertEqual(1,len(self.records('tasks')))

    def spooled_save(self):
        """A Codex save its sandbox could not write: the spool folder, the
        environment its hooks run with, and their next prompt hook."""
        spool=Path(tempfile.mkdtemp(prefix='native-spool-',dir=self.root.parent))
        env=dict(self.env,TMPDIR=str(spool))
        def prompt():
            return self.call('native-prompt','--host','codex','--task-id','research','--json',env=env,
                             data={'hook_event_name':'UserPromptSubmit','session_id':'session-a','prompt':'Inspect it.'})
        prompt();self.call('native-stop','--host','codex','--task-id','research',env=env,data=self.STOP)
        candidate,state=self.state()
        modes=self.readonly()
        try:
            spooled=self.call('native-result','--request',candidate.stem,'--token',state['token'],'--input','-',
                              '--json',data=self.payload,env=env)
            self.assertEqual('spooled',json.loads(spooled.stdout)['state'])
        finally:
            for path,mode in modes.items():path.chmod(mode)
        return spool,env,prompt

    @unittest.skipUnless(os.name=='posix' and os.geteuid()!=0,
                         'permission bits do not stop root; a flock and a process group stand in for the hook')
    def test_a_save_whose_hook_was_stopped_while_it_waited_is_made_by_the_next_hook(self):
        # The Stop hook drains the spool under `timeout` with a 5 s budget. A
        # save that waited on another session's lock outlived it: its file
        # was removed before the save began, and the save was lost with
        # nothing journaled.
        import fcntl,signal,time
        spool,env,prompt=self.spooled_save()
        lock=(self.root/'project-brain/local/runtime.lock').open('a+')
        self.addCleanup(lock.close)
        fcntl.flock(lock,fcntl.LOCK_EX)
        stop=subprocess.Popen([sys.executable,'-B',str(self.root/'memory-bank/scripts/context.py'),
                               '--root',str(self.root),'native-stop','--host','codex','--task-id','research'],
                              cwd=self.root,env=env,stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL,start_new_session=True)
        stop.stdin.write(json.dumps(dict(self.STOP,stop_hook_active=True)).encode());stop.stdin.close()
        deadline=time.monotonic()+10
        while not list(spool.rglob('*.attempt1.json')) and stop.poll() is None and time.monotonic()<deadline:
            time.sleep(0.02)
        begun=list(spool.rglob('*.attempt1.json'))
        # The save waits on the lock; the budget ends the hook as timeout does.
        os.killpg(stop.pid,signal.SIGTERM);stop.wait()
        fcntl.flock(lock,fcntl.LOCK_UN)
        self.assertEqual(1,len(begun))
        self.assertEqual(begun,list(spool.rglob('*.json')))
        self.assertEqual([],self.records('tasks'))
        lines=[(line['stage'],line['event'],line.get('code')) for line in self.written(prompt)]
        self.assertEqual(('save','saved','spooled'),lines[0])
        self.assertEqual([],list(spool.rglob('*.json')))
        self.assertEqual(1,len(self.records('tasks')))

    @unittest.skipUnless(os.name=='posix' and os.geteuid()!=0,'permission bits do not stop root')
    def test_a_save_three_stopped_hooks_began_is_given_up_as_interrupted(self):
        spool,env,_=self.spooled_save()
        [waiting]=list(spool.rglob('*.json'))
        waiting.rename(waiting.with_name(waiting.stem+'.attempt3.json'))
        lines=[(line['stage'],line['event'],line.get('code')) for line in self.written(
            lambda:self.call('native-stop','--host','codex','--task-id','research',env=env,
                             data=dict(self.STOP,stop_hook_active=True)))]
        self.assertEqual(('save','refused','interrupted'),lines[0])
        self.assertEqual([],list(spool.rglob('*.json')))
        self.assertEqual([],self.records('tasks'))

    @unittest.skipUnless(os.name=='posix' and os.geteuid()!=0,'permission bits do not stop root')
    def test_a_save_claude_codes_sandbox_spools_under_its_own_folder_is_made_by_the_next_hook(self):
        # Claude Code's sandbox gives a command claude-<uid> under its own
        # temporary directory as TMPDIR; its hooks run outside the sandbox,
        # with that directory itself. A session below the root or in a
        # worktree may not write the project, so its save waits there.
        base=Path(tempfile.mkdtemp(prefix='claude-sandbox-',dir=self.root.parent))
        inside=base/f'claude-{os.getuid()}'
        inside.mkdir(mode=0o700)
        hooks=dict(self.env,TMPDIR=str(base))
        self.call('native-prompt','--host','claude','--task-id','research','--json',env=hooks,
                  data={'hook_event_name':'UserPromptSubmit','session_id':'session-a','prompt':'Inspect it.'})
        self.call('native-stop','--host','claude','--task-id','research',env=hooks,data=self.STOP)
        candidate,state=self.state()
        modes=self.readonly()
        try:
            spooled=self.call('native-result','--request',candidate.stem,'--token',state['token'],'--input','-',
                              '--json',data=self.payload,env=dict(self.env,TMPDIR=str(inside)),ok=False)
            self.assertEqual(0,spooled.returncode,spooled.stderr.decode())
            self.assertEqual('spooled',json.loads(spooled.stdout)['state'])
        finally:
            for path,mode in modes.items():path.chmod(mode)
        self.assertEqual(1,len(list(inside.rglob('*.json'))))
        lines=[(line['stage'],line['event'],line.get('code')) for line in self.written(
            lambda:self.call('native-stop','--host','claude','--task-id','research',env=hooks,
                             data=dict(self.STOP,stop_hook_active=True)))]
        self.assertEqual(('save','saved','spooled'),lines[0])
        self.assertEqual([],list(inside.rglob('*.json')))
        self.assertEqual(1,len(self.records('tasks')))

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

    def refused_transcript(self,token,kind):
        """A Claude Code transcript whose save command the host refused."""
        path=self.root.parent/'transcript.jsonl'
        call={'type':'tool_use','id':'toolu_save','name':'Bash',
              'input':{'command':f'cd x && echo -E \'{{}}\' | python3 memory-bank/scripts/context.py native-result '
                                  f'--request r --token {token} --input - --json'}}
        result={'type':'tool_result','tool_use_id':'toolu_save','is_error':True,
                'content':'This Bash command contains multiple operations. The following part requires approval'}
        # A line that is not a message, or whose message is not an object, is passed over.
        records=[{'type':'summary','message':'tool_result in a string'},['tool_result'],
                 {'type':'assistant','message':{'role':'assistant','content':[call]}},
                 {'type':'user','message':{'role':'user','content':[result]},**({'toolDenialKind':kind} if kind else {})}]
        path.write_text(''.join(json.dumps(record)+'\n' for record in records),encoding='utf-8')
        return str(path)

    def test_a_session_whose_host_refused_the_save_is_not_asked_again(self):
        # Offline, Claude Code under -p in a project whose trust was never
        # accepted ignored its allow rules and refused the save after every
        # answer: two model calls each time and a warning on every prompt.
        def request(payload,host='claude'):
            return self.call('native-stop','--host',host,'--task-id','research',data=payload)
        def prompt(host='claude'):
            return self.call('native-prompt','--host',host,'--task-id','research',data={
                'hook_event_name':'UserPromptSubmit','session_id':'session-a','prompt':'Next question'})
        self.begin(host='claude')
        self.one(self.written(lambda:request(self.STOP)),'stop','requested',host='claude')
        token=self.state()[1]['token']
        recovery={**self.STOP,'stop_hook_active':True,'transcript_path':self.refused_transcript(token,'user-rejected')}
        self.one(self.written(lambda:request(recovery)),'stop','failed','host-refused',host='claude')
        warned=[]
        self.one(self.written(lambda:warned.append(prompt())),'prompt','skipped','host-refused',host='claude')
        self.assertEqual(b'Previous native memory result was not saved completely.\n',warned[0].stdout)
        quiet=[]
        self.one(self.written(lambda:quiet.append(prompt())),'prompt','skipped','host-refused',host='claude')
        self.assertEqual(b'',quiet[0].stdout)
        self.one(self.written(lambda:request(self.STOP)),'stop','skipped','phase-declined',host='claude')
        # A day later the session is asked again.
        candidate,state=self.state();state['updated']-=24*60*60+1;candidate.write_text(json.dumps(state))
        self.one(self.written(prompt),'prompt','armed',host='claude')

    def test_a_save_refused_in_the_dialog_is_not_asked_again(self):
        # Claude Code's terminal, offline: a person answered No to the save
        # command and the turn ended without a Stop, so the request stayed
        # pending and the next answer asked again.
        def prompt(transcript=None):
            payload={'hook_event_name':'UserPromptSubmit','session_id':'session-a','prompt':'Next question'}
            if transcript: payload['transcript_path']=transcript
            return self.call('native-prompt','--host','claude','--task-id','research',data=payload)
        self.begin(host='claude')
        self.call('native-stop','--host','claude','--task-id','research',data=self.STOP)
        token=self.state()[1]['token']
        warned=[]
        lines=self.written(lambda:warned.append(prompt(self.refused_transcript(token,'user-rejected'))))
        self.one(lines,'prompt','skipped','host-refused',host='claude')
        self.assertEqual(b'Previous native memory result was not saved completely.\n',warned[0].stdout)
        self.one(self.written(prompt),'prompt','skipped','host-refused',host='claude')
        self.one(self.written(lambda:self.call('native-stop','--host','claude','--task-id','research',
                                               data=self.STOP)),'stop','skipped','phase-declined',host='claude')

    def test_a_refusal_written_after_the_stop_is_read_at_the_next_prompt(self):
        # Offline in auto mode: the denial was created 10 ms before the
        # recovery Stop and was not on disk when that Stop read the transcript.
        self.begin(host='claude')
        self.call('native-stop','--host','claude','--task-id','research',data=self.STOP)
        token=self.state()[1]['token']
        late=self.root.parent/'late.jsonl'
        late.write_text('',encoding='utf-8')
        recovery={**self.STOP,'stop_hook_active':True,'transcript_path':str(late)}
        self.one(self.written(lambda:self.call('native-stop','--host','claude','--task-id','research',data=recovery)),
                 'stop','failed','unsaved',host='claude')
        payload={'hook_event_name':'UserPromptSubmit','session_id':'session-a','prompt':'Next question',
                 'transcript_path':self.refused_transcript(token,'automode-blocked')}
        lines=self.written(lambda:self.call('native-prompt','--host','claude','--task-id','research',data=payload))
        self.one(lines,'prompt','skipped','host-refused',host='claude')

    def test_an_interrupted_save_is_not_a_refusal(self):
        self.begin(host='claude')
        self.call('native-stop','--host','claude','--task-id','research',data=self.STOP)
        token=self.state()[1]['token']
        for kind in ('interrupted','automode-parsing-error',None):
            with self.subTest(kind=kind):
                candidate,state=self.state();state['phase']='requested';candidate.write_text(json.dumps(state))
                recovery={**self.STOP,'stop_hook_active':True,'transcript_path':self.refused_transcript(token,kind)}
                lines=self.written(lambda:self.call('native-stop','--host','claude','--task-id','research',data=recovery))
                self.one(lines,'stop','failed','unsaved',host='claude')

    def session_state(self,session):
        digest=hashlib.sha256(session.encode()).hexdigest()
        for candidate in (self.root/'project-brain/local/native-results').glob('*.json'):
            state=json.loads(candidate.read_text())
            if state['session']==digest: return candidate,state
        self.fail('no request for '+session)

    def save_as(self,session,data,ok=True):
        candidate,state=self.session_state(session)
        return self.call('native-result','--request',candidate.stem,'--token',state['token'],
                         '--input','-','--json',data=data,ok=ok)

    OTHER={'progress':'Checked the callers of the dispatcher.','verified':True,'learnings':[
        {'type':'decision','title':'Callers send idempotent requests',
         'consequence':'Callers of the Amber dispatcher send it only idempotent requests.','sources':['rules.md']}]}

    def test_two_sessions_of_a_branch_both_save_in_the_same_seconds(self):
        # Offline, two sessions of one branch were asked to save at once. The
        # first save moved the task's revision, and the second was refused -
        # its learning with it - and told its agent to report a held save to
        # a person who had read the answer. It is recorded at the task as it
        # now is, and its progress is the task's, as a moment later.
        self.call('start','--task-id','research','--goal','Inspect routing')
        for session in ('session-a','session-b'):
            self.begin(session=session);self.stop(session=session)
        self.one(self.written(lambda:self.save_as('session-b',self.OTHER)),'save','saved')
        saved=[]
        lines=self.written(lambda:saved.append(json.loads(self.save_as('session-a',self.payload).stdout)))
        self.assertEqual(1,self.one(lines,'save','saved','rebased')['learnings'])
        self.assertEqual(['created'],[item['state'] for item in saved[0]['records']])
        task,=self.records('tasks')
        self.assertEqual((self.payload['progress'],self.payload['next_steps']),(task['progress'],task['next_steps']))
        self.assertEqual({'Amber retry boundary','Callers send idempotent requests'},
                         {record['title'] for kind in ('findings','decisions') for record in self.records(kind)})
        # Its own Stop settles it as any save.
        self.one(self.written(lambda:self.stop(session='session-a',active=True)),'stop','skipped','recovery-saved')

    def test_a_turn_checkpoint_since_the_request_takes_the_save(self):
        # The other session's Stop hook flushed its turns into the task.
        self.call('start','--task-id','research','--goal','Inspect routing')
        self.begin();self.stop()
        self.call('update','--task-id','research','--revision','auto','--file','rules.md')
        self.one(self.written(lambda:self.save()),'save','saved','rebased')
        task,=self.records('tasks')
        self.assertEqual(self.payload['progress'],task['progress'])

    def test_a_task_another_session_created_since_the_request_takes_the_save(self):
        # In a new project the other session's save created the branch task.
        for session in ('session-a','session-b'):
            self.begin(session=session);self.stop(session=session)
        self.one(self.written(lambda:self.save_as('session-b',self.OTHER)),'save','saved')
        saved=[]
        lines=self.written(lambda:saved.append(json.loads(self.save_as('session-a',self.payload).stdout)))
        self.one(lines,'save','saved','rebased')
        self.assertEqual(['created'],[item['state'] for item in saved[0]['records']])
        task,=self.records('tasks')
        self.assertEqual(self.payload['progress'],task['progress'])

    def test_a_task_replaced_since_the_request_still_refuses_the_save(self):
        # Completed and started again: the save was asked for the task before.
        self.call('start','--task-id','research','--goal','Inspect routing')
        self.begin();self.stop()
        task,=self.records('tasks')
        self.call('complete','--task-id','research','--outcome','Routing inspected.','--revision',str(task['revision']))
        self.call('start','--task-id','research','--goal','Inspect routing again')
        before=self.authority();refused=[]
        lines=self.written(lambda:refused.append(self.save(ok=False)))
        self.one(lines,'save','refused','native-result:task-changed')
        self.assertIn(b'native-result:task-changed: the branch task was replaced after this save was requested; '
                      b'nothing was written: report the held save, do not retry it',refused[0].stderr)
        self.assertEqual(before,self.authority())

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

    def test_a_refused_input_says_what_to_correct(self):
        # The agent gets one retry. A code alone does not say what to fix;
        # a shell's echo turns the \\n of a JSON string into a real newline.
        self.begin();self.stop();candidate,state=self.state()
        cases=((b'{"progress": "Line one\nline two", "verified": true, "learnings": []}',
                'write a newline as \\n and a tab as \\t (a plain echo in zsh turns \\n into a line break: pipe with echo -E)'),
               (b'{"progress": "a"}{"progress": "b"}','more than one JSON object'),
               (b'[]','got an array'),
               (b'progress: read the file','not one JSON object (Expecting value at line 1 column 1)'))
        for data,text in cases:
            with self.subTest(text=text):
                results=[]
                self.one(self.written(lambda:results.append(self.raw('native-result','--request',candidate.stem,
                    '--token',state['token'],'--input','-','--json',data=data))),
                    'save','refused','native-result:invalid-input')
                self.assertIn(text.encode(),results[0].stderr)
        learning={**self.payload['learnings'][0],'verified':True}
        refused=self.save({**self.payload,'learnings':[learning]},ok=False)
        self.assertIn(b'unexpected: verified (verified is a top-level boolean',refused.stderr)
        missing={key:value for key,value in self.payload['learnings'][0].items() if key!='sources'}
        refused=self.save({**self.payload,'learnings':[missing]},ok=False)
        self.assertIn(b'missing: sources',refused.stderr)
        gone={**self.payload['learnings'][0],'sources':['src/Gone.php#L3']}
        refused=self.save({**self.payload,'learnings':[gone]},ok=False)
        self.assertIn(b'Source is missing or is not a regular file: src/Gone.php\n',refused.stderr)
        twice=[self.payload['learnings'][0],{**self.payload['learnings'][0],'consequence':'Another rule.'}]
        refused=self.save({**self.payload,'learnings':twice},ok=False)
        self.assertIn(b"two findings titled 'Amber retry boundary' cite the same sources; merge them",refused.stderr)
        refused=self.save({'verified':True,'learnings':[]},ok=False)
        self.assertIn(b'Nothing to record: give a progress summary',refused.stderr)
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
        # A temporary directory of its own: another suite's hook running
        # beside this one leaves its own file in a shared one for a moment.
        private=Path(tempfile.mkdtemp(prefix='native-hook-tmp-',dir=self.root.parent))
        for host,session in (('claude','unsaved-claude'),('codex','unsaved-codex')):
            with self.subTest(host=host):
                hooks=self.install_hooks(host)
                self.begin(host,session);self.stop(host,session);self.stop(host,session,active=True)
                if report.exists():report.unlink()
                result=self.hook(hooks,'working-memory-read.sh',{'hook_event_name':'UserPromptSubmit',
                    'session_id':session,'cwd':str(self.root),'prompt':'Continue the Amber routing research.'},
                    env=dict(self.env,TMPDIR=str(private)))
                lines=result.stdout.decode().splitlines()
                self.assertEqual(notice,lines[0])
                self.assertEqual(1,lines.count(notice))
                self.assertFalse(report.exists() and 'native_capture_incomplete' in report.read_text())
                self.assertNotIn('native memory result not saved completely',result.stdout.decode())
                self.assertFalse(list(private.glob('working-memory-native.*')))
        hooks=self.install_hooks('cursor')
        self.begin('cursor','unsaved-cursor');self.stop('cursor','unsaved-cursor');self.stop('cursor','unsaved-cursor',active=True)
        result=self.hook(hooks,'working-memory-read.sh',{'hook_event_name':'beforeSubmitPrompt',
            'conversation_id':'unsaved-cursor','prompt':'Continue the Amber routing research.'})
        self.assertEqual({'continue':True},json.loads(result.stdout))
        rule=(self.root/'.cursor/rules/working-memory.mdc').read_text()
        self.assertIn('native memory result not saved completely',rule)


    @unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'POSIX shell hook profile')
    def test_a_codex_save_continuation_gets_no_second_capsule(self):
        """A save continuation sent back through UserPromptSubmit gets no
        capsule: the turn's own prompt brought one, and the continuation is
        bookkeeping, while the arm still records it as a continuation.
        Codex 0.163 adds the continuation to the turn as a <hook_prompt>
        item instead; the guard stays for a host that sends it as a prompt."""
        hooks=self.install_hooks('codex')
        ask={'hook_event_name':'UserPromptSubmit','session_id':'cont-codex','cwd':str(self.root),
             'prompt':'Inspect the Amber retry policy.','turn_id':'t1','model':'m','permission_mode':'default'}
        first=self.hook(hooks,'working-memory-read.sh',ask)
        self.assertIn('working:',first.stdout.decode())
        stop=self.hook(hooks,'working-memory-write.sh',{'hook_event_name':'Stop','session_id':'cont-codex',
            'cwd':str(self.root),'stop_hook_active':False,'turn_id':'t1','model':'m','permission_mode':'default',
            'last_assistant_message':'The Amber dispatcher retries only idempotent requests.'})
        reason=json.loads(stop.stdout.decode().splitlines()[0])['reason']
        before=len(self.journal())
        continuation=self.hook(hooks,'working-memory-read.sh',dict(ask,prompt=reason,turn_id='t2'))
        self.assertEqual(b'',continuation.stdout)
        self.assertIn(('prompt','continuation'),[(line['stage'],line['event']) for line in self.journal()[before:]])
        # The runtime decides it, for every hook that passes the prompt.
        for host in ('claude','codex'):
            quiet=self.call('refresh','--host',host,'--query',reason,'--task-id','research','--ephemeral')
            self.assertEqual(b'',quiet.stdout)
        cursor=self.call('refresh','--host','cursor','--query',reason,'--task-id','research','--ephemeral','--json')
        self.assertIn('capsule_text',json.loads(cursor.stdout))


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

    def test_a_plan_mode_turn_is_not_asked_to_save(self):
        # Claude Code and Codex both send permission_mode; in plan mode the
        # host tells the agent to change nothing, overriding the request.
        for host in ('claude','codex'):
            with self.subTest(host=host):
                session='plan-'+host
                stop={'hook_event_name':'Stop','session_id':session,'stop_hook_active':False,
                      'permission_mode':'plan','last_assistant_message':'Planned the change.'}
                self.assertTrue(self.begin(host=host,session=session)['armed'])
                before=len(self.journal())
                planned=self.call('native-stop','--host',host,'--task-id','research',data=stop)
                self.assertEqual(b'',planned.stdout.strip())
                self.assertEqual([('stop','skipped','plan-mode')],
                                 [(line['stage'],line['event'],line.get('code')) for line in self.journal()[before:]])
                # Out of plan mode, the next answer is asked as before.
                self.begin(host=host,session=session)
                asked=self.call('native-stop','--host',host,'--task-id','research',
                                data=dict(stop,permission_mode='default'))
                self.assertIn('native-result --request',asked.stdout.decode())

    def rollout(self,name,turns,pad=0):
        """A Codex rollout: session_meta, then one turn_context per (turn, mode,
        sandbox[, file-system policy, working directory])."""
        path=self.root.parent/name
        lines=[json.dumps({'type':'session_meta','payload':{'id':'0','source':'vscode'}})]
        for turn,mode,sandbox,*files in turns:
            context={'turn_id':turn,'collaboration_mode':{'mode':mode},'sandbox_policy':{'type':sandbox}}
            if files:context['file_system_sandbox_policy']=files[0]
            if files[1:] and files[1] is not None:context['cwd']=files[1]
            lines.append(json.dumps({'type':'turn_context','payload':context}))
            lines.append(json.dumps({'type':'response_item','payload':{'type':'message','role':'user',
                'content':[{'type':'input_text','text':'x'*pad}]}}))
        path.write_text('\n'.join(lines)+'\n')
        return path

    def test_a_codex_plan_or_read_only_turn_is_not_asked_to_save(self):
        # Codex's permission_mode follows the approval policy; its Plan mode
        # and sandbox show only in the rollout's turn_context.
        cases={'plan mode':('plan','workspace-write','plan-mode'),
               'read-only sandbox':('default','read-only','read-only-sandbox'),
               'default':('default','workspace-write',None)}
        for n,(name,(mode,sandbox,skipped)) in enumerate(cases.items()):
            with self.subTest(case=name):
                session=f'turn-{n}'
                path=self.rollout(f'{session}.jsonl',[('t0','default','workspace-write'),('t1',mode,sandbox)])
                stop={'hook_event_name':'Stop','session_id':session,'stop_hook_active':False,'turn_id':'t1',
                      'permission_mode':'bypassPermissions','transcript_path':str(path),
                      'last_assistant_message':'Answered.'}
                self.begin(host='codex',session=session)
                before=len(self.journal())
                out=self.call('native-stop','--host','codex','--task-id','research',data=stop).stdout
                lines=[(line['event'],line.get('code')) for line in self.journal()[before:] if line['stage']=='stop']
                if skipped:
                    self.assertEqual(b'',out.strip())
                    self.assertEqual([('skipped',skipped)],lines)
                else:
                    self.assertIn(b'native-result --request',out)

    def test_a_codex_workspace_that_cannot_write_the_brain_is_not_asked_to_save(self):
        # Codex started in a subdirectory writes only there: offline on Codex
        # 0.163 the save failed with a read-only file system after every answer.
        root=os.path.realpath(self.root)
        link=str(self.root.parent/'linked project')
        try: os.symlink(root,link,target_is_directory=True)
        except (OSError,NotImplementedError): link=None  # Windows without the symlink privilege
        def files(*writable,read_only=(),temporary=False):
            entries=[{'path':{'type':'special','value':{'kind':'root'}},'access':'read'}]
            entries+=[{'path':{'type':'path','path':path},'access':'write'} for path in writable]
            entries+=[{'path':{'type':'path','path':path},'access':'read'} for path in read_only]
            if temporary:
                entries+=[{'path':{'type':'special','value':{'kind':kind}},'access':'write'} for kind in ('slash_tmp','tmpdir')]
            return {'kind':'restricted','entries':entries}
        # A policy read as not letting the turn write its own directory is in
        # a form the reading does not know: asked as before.
        cases={'a subdirectory':(files(root+'/src'),root+'/src','outside-sandbox'),
               'the Brain carved out':(files(root,read_only=(root+'/project-brain',)),root,'outside-sandbox'),
               'the project root':(files(root),root,None),
               'an added writable root':(files(root+'/src',root),root+'/src',None),
               'its own directory unwritable':(files(root+'/elsewhere'),root+'/src',None),
               'no working directory':(files(root+'/src'),None,None),
               # Codex's workspace-write sandbox writes the temporary
               # directories too: the save waits there for the next hook.
               'a subdirectory with temporary directories':(files(root+'/src',temporary=True),root+'/src',None),
               # Codex names the workspace as it was opened; a link to the
               # project (macOS's /tmp is one) is the project itself.
               **({'the project through a link':(files(link),link,None),
                   'a subdirectory through a link':(files(link+'/src'),link+'/src','outside-sandbox')}
                  if link else {})}
        for n,(name,(policy,cwd,skipped)) in enumerate(cases.items()):
            with self.subTest(case=name):
                session=f'workspace-{n}'
                path=self.rollout(f'{session}.jsonl',[('t1','default','workspace-write',policy,cwd)])
                stop={'hook_event_name':'Stop','session_id':session,'stop_hook_active':False,'turn_id':'t1',
                      'permission_mode':'bypassPermissions','transcript_path':str(path),
                      'last_assistant_message':'Answered.'}
                self.begin(host='codex',session=session)
                before=len(self.journal())
                out=self.call('native-stop','--host','codex','--task-id','research',data=stop).stdout
                lines=[(line['event'],line.get('code')) for line in self.journal()[before:] if line['stage']=='stop']
                if skipped:
                    self.assertEqual(b'',out.strip())
                    self.assertEqual([('skipped',skipped)],lines)
                else:
                    self.assertIn(b'native-result --request',out)

    def test_the_codex_temporary_directory_on_windows_is_the_users_own(self):
        # Codex names the temporary directory `tmpdir`. Windows rarely sets
        # TMPDIR: there it is the user's TEMP folder, where the save waits.
        policy={'kind':'restricted','entries':[{'path':{'type':'special','value':{'kind':'tmpdir'}},'access':'write'}]}
        temporary=str(self.root.parent/'windows-temp')
        # Resolved, as the Stop passes it: a Windows TEMP path can carry 8.3
        # short names (RUNNER~1) that the policy's resolved entry does not.
        spool=os.path.realpath(os.path.join(temporary,'ai-accelerator-native-spool-user'))
        with self.runtime() as (native,_,_):
            os.environ.pop('TMPDIR',None)
            with mock.patch.object(native.tempfile,'gettempdir',return_value=temporary):
                with mock.patch.object(native.os,'name','nt'):
                    self.assertTrue(native._sandbox_writes(policy,spool))
                with mock.patch.object(native.os,'name','posix'):
                    self.assertFalse(native._sandbox_writes(policy,spool))
            os.environ['TMPDIR']=temporary
            with mock.patch.object(native.os,'name','posix'):
                self.assertTrue(native._sandbox_writes(policy,spool))

    def test_an_unknown_codex_turn_is_asked_as_before(self):
        # Another turn's context, or one beyond the tail the Stop reads, is unknown.
        cases={'other turn':(self.rollout('other.jsonl',[('t1','plan','read-only')]),'t2'),
               'beyond the tail':(self.rollout('far.jsonl',[('t1','plan','read-only')],pad=600*1024),'t1'),
               'no rollout':(self.root.parent/'gone.jsonl','t1')}
        for n,(name,(path,turn)) in enumerate(cases.items()):
            with self.subTest(case=name):
                session=f'unknown-{n}'
                stop={'hook_event_name':'Stop','session_id':session,'stop_hook_active':False,'turn_id':turn,
                      'permission_mode':'bypassPermissions','transcript_path':str(path),
                      'last_assistant_message':'Answered.'}
                self.begin(host='codex',session=session)
                out=self.call('native-stop','--host','codex','--task-id','research',data=stop).stdout
                self.assertIn(b'native-result --request',out)

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
        return requested(host,json.loads(reply.stdout))

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

    def test_claude_shows_what_the_step_is_before_its_marker(self):
        # Claude Code shows the first lines of a Stop hook's reason under "Ran
        # N stop hooks" after every answer; Codex and Cursor send it back as a
        # prompt, known by the marker that opens it.
        lines=self.ask(session='marker-claude').splitlines()
        self.assertTrue(lines[0].startswith('This is an internal continuation'))
        self.assertRegex(lines[-1],r'^INTERNAL_NATIVE_MEMORY_RESULT [0-9a-f]{32}$')
        for host in ('codex','cursor'):
            with self.subTest(host=host):
                self.assertRegex(self.ask(host=host,session='marker-'+host).splitlines()[0],
                                 r'^INTERNAL_NATIVE_MEMORY_RESULT [0-9a-f]{32}$')

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


class NativeInstructionTests(NativeCaptureFixture):
    """The full save step while the conversation lacks one, a short one after."""

    ask=NativeAnswerTests.ask
    FULL='Exact schema example (replace placeholders, never save the example)'
    BACK='the rules of the full save step earlier in this conversation'

    def transcript(self,name,*records):
        # Claude Code and Codex write their records as compact JSON lines.
        path=self.root.parent/name
        path.write_text(''.join(json.dumps(record,separators=(',',':'))+'\n' for record in records),
                        encoding='utf-8')
        return path

    def test_a_later_step_points_back_to_the_full_one_the_conversation_holds(self):
        # Every answer carried the whole step, about 2,600 characters, and
        # Claude Code's terminal showed it whole each time.
        for host in ('claude','codex'):
            with self.subTest(host=host):
                first=self.ask(host=host,session=host+'-first')
                self.assertIn(self.FULL,first)
                held=self.transcript(host+'.jsonl',{'type':'user','message':{'content':first}})
                later=self.ask(host=host,session=host+'-later',transcript=held)
                self.assertNotIn(self.FULL,later)
                self.assertIn(self.BACK,later)
                self.assertIn('exact command: ',later)
                self.assertLess(len(later),len(first)-800)

    def test_cursor_gets_the_full_step_every_time(self):
        # Its transcript format and how it records a summarized conversation
        # are unknown here, so nothing tells a short step is safe.
        first=self.ask(host='cursor',session='cursor-first')
        held=self.transcript('cursor.jsonl',{'type':'user','message':{'content':first}})
        self.assertIn(self.FULL,self.ask(host='cursor',session='cursor-later',transcript=held))

    def test_the_full_step_comes_back_after_a_compaction_or_without_a_transcript(self):
        first=self.ask(session='before')
        for record in ({'type':'system','subtype':'compact_boundary'},{'type':'compacted','payload':{}}):
            with self.subTest(record=record['type']):
                path=self.transcript('compacted.jsonl',{'type':'user','message':{'content':first}},record)
                self.assertIn(self.FULL,self.ask(session='after-'+record['type'],transcript=path))
        # Codex's compaction record keeps earlier items in a retained context
        # that is not the model's history: the steps it holds are dropped too.
        carried=self.transcript('carried.jsonl',{'type':'user','message':{'content':first}},
                                {'type':'compacted','payload':{'message':'summary','retained_context':{'items':[first]}}})
        self.assertIn(self.FULL,self.ask(host='codex',session='after-carried',transcript=carried))
        self.assertIn(self.FULL,self.ask(session='gone',transcript=self.root.parent/'gone.jsonl'))
        self.assertIn(self.FULL,self.ask(session='none'))

    def test_the_setting_can_send_the_full_step_every_time(self):
        first=self.ask(session='setting-first')
        held=self.transcript('held.jsonl',{'type':'user','message':{'content':first}})
        cfg_file=self.root/'project-brain/config/runtime.json'
        for value in ('full','bogus'):
            with self.subTest(value=value):
                cfg=json.loads(cfg_file.read_text());cfg['native_result_instruction']=value
                cfg_file.write_text(json.dumps(cfg))
                self.assertIn(self.FULL,self.ask(session='setting-'+value,transcript=held))
        cfg=json.loads(cfg_file.read_text());cfg['native_result_instruction']='auto'
        cfg_file.write_text(json.dumps(cfg))
        self.assertNotIn(self.FULL,self.ask(session='setting-auto',transcript=held))
        self.assertIn(self.FULL,self.ask(session='setting-env',transcript=held,
                                         env={'CONTEXT_NATIVE_RESULT_INSTRUCTION':'full'}))


class ClaudeSaveStepTests(NativeCaptureFixture):
    """Claude Code drew the save step after every answer as a hook error,
    with an error notification, and handed the model its text twice; and it
    asked before the save command each time, whatever the allow rules say,
    because the command held a subshell. Found by driving Claude Code 2.1.278
    offline against a scripted model."""

    def ask(self,host,session):
        self.call('native-prompt','--host',host,'--task-id','research','--json',
                  data={'hook_event_name':'UserPromptSubmit','session_id':session,'prompt':'Inspect the retry policy.'})
        stop={'hook_event_name':'Stop','session_id':session,'stop_hook_active':False,
              'last_assistant_message':'The Amber dispatcher retries only idempotent requests.'}
        return json.loads(self.call('native-stop','--host',host,'--task-id','research',data=stop).stdout)

    @staticmethod
    def command(text,name='exact command: '):
        return text.split(name,1)[1].split('\n',1)[0]

    def test_claude_continues_with_hook_feedback_and_codex_with_a_block(self):
        output=self.ask('claude','feedback-claude')
        self.assertEqual({'hookSpecificOutput'},set(output))
        self.assertEqual('Stop',output['hookSpecificOutput']['hookEventName'])
        self.assertIn('native-result --request',output['hookSpecificOutput']['additionalContext'])
        self.assertEqual('block',self.ask('codex','feedback-codex')['decision'])

    def test_the_claude_save_command_holds_no_subshell_group_or_heredoc(self):
        command=self.command(requested('claude',self.ask('claude','shape')))
        self.assertTrue(command.startswith('cd '))
        self.assertRegex(command,r" && echo -E '<JSON>' \| (python3|python|py -3) memory-bank/scripts/context\.py ")
        bare=re.sub(r"'[^']*'","''",command)
        for operator in ('(',')','{','}','<<','`','$',';'):
            with self.subTest(operator=operator):self.assertNotIn(operator,bare)

    @unittest.skipUnless(os.name=='posix','POSIX shells')
    def test_the_claude_save_command_keeps_the_json_verbatim_in_each_shell(self):
        # Claude Code's Bash tool runs bash or zsh, zsh first unless $SHELL is
        # bash; zsh's plain echo would turn the JSON's backslashes into characters.
        shells=[shell for shell in ('bash','zsh') if shutil.which(shell)]
        work=self.root/'src';work.mkdir()
        for n,shell in enumerate(shells):
            with self.subTest(shell=shell):
                progress=(f"Run {n}: `Invoice::next($tenant)` costs $5; line one.\nTab\there; "
                          f"path C:\\shop\\src; it's \"quoted\"; счёт.")
                data=dict(self.payload,progress=progress,learnings=[])
                quoted="'"+json.dumps(data,ensure_ascii=False).replace("'","'\\''")+"'"
                command=self.command(requested('claude',self.ask('claude',f'verbatim-{n}'))).replace("'<JSON>'",quoted)
                done=subprocess.run([shell,'-c',command],cwd=work,env=self.env,capture_output=True)
                self.assertEqual(0,done.returncode,done.stdout.decode()+done.stderr.decode())
                self.assertEqual(progress,self.records('tasks')[0]['progress'])

    @unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'POSIX shell')
    def test_the_claude_skip_command_runs_as_given(self):
        text=requested('claude',self.ask('claude','skip'))
        command=self.command(text,'run this command instead: ')
        self.assertTrue(command.endswith('--skip --json'))
        done=subprocess.run(['bash','-c',command],cwd=self.root.parent,env=self.env,capture_output=True)
        self.assertEqual(0,done.returncode,done.stderr.decode())
        self.assertEqual('omitted',json.loads(done.stdout)['state'])
        self.assertEqual([],self.records('tasks'))


if __name__=='__main__':unittest.main()
