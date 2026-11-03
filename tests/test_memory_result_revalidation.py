"""Explicit cross-task checked knowledge revisions, not implicit freshness."""
from __future__ import annotations

import copy
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile
import subprocess
from concurrent.futures import ThreadPoolExecutor
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / 'PHP Core/memory-bank/scripts'


class RevalidationFixture(unittest.TestCase):
    def setUp(self):
        environment = mock.patch.dict(os.environ, {k:v for k,v in os.environ.items()
            if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}, clear=True)
        environment.start(); self.addCleanup(environment.stop)
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.brain = importlib.import_module('brain_runtime')
            self.results = importlib.import_module('memory_results')
        finally:
            sys.path.pop(0)
        temporary = tempfile.TemporaryDirectory(prefix='cedar-revalidation-')
        self.addCleanup(temporary.cleanup); self.root = Path(temporary.name)
        self.source = self.root / 'src/rule.txt'; self.source.parent.mkdir()
        self.source.write_text('Cedar reservation uses four slots.\n')
        self.config = self.root / 'project-brain/config/runtime.json'
        self.config.parent.mkdir(parents=True)
        self.config.write_text(json.dumps({'mode':'governed','automatic_promotion':True}))
        bank = self.root / 'memory-bank'; (bank/'chunks').mkdir(parents=True)
        (bank/'README.md').write_text('# Cedar memory\n')
        (bank/'INDEX.md').write_text('# Memory Index\n\n| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n| --- | --- | --- | --- | --- | --- | --- | --- |\n')
        self.a = self.brain.create_task(self.root, 'cedar/a', 'Inspect Cedar rule', [], [], owner='local')
        self.learning = {'type':'finding','title':'Cedar reservation rule',
            'consequence':'Cedar reservations round up to a multiple of four slots.', 'sources':['src/rule.txt']}
        saved = self.save(self.a, 'source', self.learning)
        self.record_id = saved['records'][0]['id']
        self.old_memory_id = saved['promotion']['promoted'][0]['memory_id']
        self.old_chunk = next((bank/'chunks').glob(self.old_memory_id+'-*.md'))
        self.b = self.brain.create_task(self.root, 'cedar/b', 'Extend Cedar code', [], [], owner='local')

    def save(self, task, result_id, learning, **options):
        return self.results.record_result(self.root, task['id'], result_id, 'auto',
            {'progress':'Checked Cedar sources.', 'learnings':[learning], 'verified':True},
            owner=options.pop('owner','local'), reason='Checked Cedar current claim and sources', **options)

    def target(self):
        return self.brain.get_record(self.root, self.record_id)

    def reference(self, **changes):
        return {**copy.deepcopy(self.learning), 'record_id':self.record_id,
            'expected_revision':self.target()['revision'],
            'source_fingerprints':self.brain.source_fingerprints(self.root, self.learning['sources']), **changes}

    def change_source(self):
        self.source.write_text(self.source.read_text()+'Another method was added.\n')

    def bodies(self):
        return {p.name:self.brain._parse_chunk(p) for p in (self.root/'memory-bank/chunks').glob('*.md')}


class RevalidationTests(RevalidationFixture):
    def test_explicit_recheck_restores_bank_after_independent_bank_reverify_and_clock_skew(self):
        from datetime import datetime,timedelta
        for bank_first,skew in ((False,True),(True,False),(True,True)):
            with self.subTest(bank_first=bank_first,skew=skew):
                # Each combination owns its complete state; no completed
                # proof from another combination can make this a replay.
                fixture=RevalidationFixture();fixture.setUp()
                try:
                    fixture.change_source()
                    if bank_first:fixture.brain.reverify_chunk(fixture.root,fixture.old_memory_id)
                    if skew:
                        previous=next(iter(fixture.brain.promotion_ledger(fixture.root)[1].values()))
                        stamp=(datetime.fromisoformat(previous['updated_at'])-timedelta(seconds=10)).isoformat()
                        with mock.patch.object(fixture.brain,'utc_now',return_value=stamp):
                            result=fixture.save(fixture.b,'recheck',fixture.reference())
                    else:result=fixture.save(fixture.b,'recheck',fixture.reference())
                    self.assertEqual('updated',result['records'][0]['state'])
                    self.assertEqual(1,len(result['promotion']['promoted']))
                    self.assertEqual([],fixture.brain.audit_bank(fixture.root)['source_changed'])
                    self.assertEqual('superseded',fixture.brain._parse_chunk(fixture.old_chunk)[0]['status'])
                finally:fixture.doCleanups()

    def test_ordinary_record_churn_does_not_create_a_new_checked_version(self):
        target=self.target()
        self.brain.update_record(self.root,target['id'],expected_revision=target['revision'],
            progress=None,next_steps=['Metadata changed only.'],files=[],sources=[],actor='local')
        result=self.brain.auto_promote(self.root,owner='local')
        self.assertEqual([],result['promoted']);self.assertEqual(1,len(self.bodies()))

    def test_legacy_promotion_clock_order_cannot_authorize_a_metadata_only_refresh(self):
        from datetime import datetime,timedelta
        previous=next(iter(self.brain.promotion_ledger(self.root)[1].values()))
        previous.pop('source_verifications')
        stamp=(datetime.fromisoformat(self.target()['created_at'])-timedelta(seconds=10)).isoformat()
        for field in ('created_at','reviewed_at','updated_at'):previous[field]=stamp
        destination=self.root/'project-brain/control/promotions'/(previous['id']+'.json')
        self.brain.atomic_json(destination,previous)
        target=self.target()
        self.brain.update_record(self.root,target['id'],expected_revision=target['revision'],progress=None,
            next_steps=['Metadata only.'],files=[],sources=[],actor='local')
        self.assertEqual([],self.brain.auto_promote(self.root,owner='local')['promoted'])
        self.assertEqual(1,len(self.bodies()))
        # Real changed checked evidence remains sufficient for legacy records.
        self.change_source();result=self.save(self.b,'legacy-new-image',self.reference())
        self.assertEqual(1,len(result['promotion']['promoted']))

    def test_persistent_index_failure_rolls_back_predecessor_and_can_replay(self):
        self.change_source();reference=self.reference();write=self.brain._PublicationFiles.write
        original=self.old_chunk.read_bytes()
        # The publication's own write, which Windows does through the NT
        # backend: os.replace failed the index on POSIX only.
        def failed(files,relative,content,expected):
            if Path(relative).name=='INDEX.md':raise PermissionError('index remains unavailable')
            return write(files,relative,content,expected)
        with mock.patch.object(self.brain._PublicationFiles,'write',autospec=True,side_effect=failed):
            with self.assertRaises(PermissionError):self.save(self.b,'persistent-index',reference)
        self.assertEqual(original,self.old_chunk.read_bytes())
        self.assertEqual(1,sum(m['status']=='active' for m,_ in self.bodies().values()))
        revision=self.target()['revision'];result=self.save(self.b,'persistent-index',reference)
        self.assertTrue(result['replayed']);self.assertEqual(revision,self.target()['revision'])
        self.assertEqual(1,sum(m['status']=='active' for m,_ in self.bodies().values()))
        self.assertEqual([],self.brain.audit_bank(self.root)['source_changed'])

    def test_confirmed_old_claim_reuses_record_and_versions_bank(self):
        previous = self.target(); old_body = self.brain._parse_chunk(self.old_chunk)[1]
        self.change_source(); reply = self.save(self.b, 'confirm', self.reference())
        self.assertEqual(self.record_id, reply['records'][0]['id'])
        self.assertEqual('updated', reply['records'][0]['state'])
        self.assertEqual(previous['revision']+1, self.target()['revision'])
        self.assertEqual(1,len(list((self.root/'project-brain/control/result-learning-images').glob('*.json'))))
        self.assertEqual('agent', self.brain.record_attestation(self.target()))
        self.assertTrue(self.brain.sources_are_fresh(self.root,self.target()))
        [promoted] = reply['promotion']['promoted']
        old, body = self.brain._parse_chunk(self.old_chunk)
        self.assertEqual(old_body,body); self.assertEqual('superseded',old['status'])
        self.assertEqual(promoted['memory_id'],old['superseded_by'])
        successor = next(m for m,_ in self.bodies().values() if m['id']==promoted['memory_id'])
        self.assertEqual([self.old_memory_id],successor['supersedes'])
        self.assertEqual(1,sum(m['status']=='active' for m,_ in self.bodies().values()))
        self.assertEqual([],self.brain.audit_bank(self.root)['source_changed'])

    def test_corrected_claim_preserves_predecessor_body(self):
        old_body=self.brain._parse_chunk(self.old_chunk)[1]
        self.source.write_text('Cedar reservation now uses eight slots.\n')
        reply=self.save(self.b,'correct',self.reference(consequence='Cedar reservations round up to a multiple of eight slots.'))
        self.assertEqual('updated',reply['records'][0]['state'])
        self.assertIn('eight slots',self.target()['progress'])
        self.assertEqual(old_body,self.brain._parse_chunk(self.old_chunk)[1])
        self.assertEqual('superseded',self.brain._parse_chunk(self.old_chunk)[0]['status'])

    def test_already_checked_same_image_is_noop(self):
        before=self.target(); chunks=self.bodies()
        reply=self.save(self.b,'unchanged',self.reference())
        self.assertEqual('existing',reply['records'][0]['state'])
        self.assertEqual(before,self.target()); self.assertEqual(chunks,self.bodies())

    def test_source_edit_or_ordinary_update_does_not_confirm(self):
        self.change_source(); original=self.target()['source_fingerprints']
        current=self.target()
        self.brain.update_record(self.root,self.record_id,expected_revision=current['revision'],
            progress=None,next_steps=['Recheck later'],files=[],sources=[],actor='local')
        self.brain.auto_promote(self.root,owner='local')
        self.assertEqual(original,self.target()['source_fingerprints'])
        self.assertEqual(1,len(self.bodies()))
        self.assertTrue(self.brain.audit_bank(self.root)['source_changed'])

    def test_stale_cas_holds_reference_without_orphan(self):
        reference=self.reference(); self.change_source()
        current=self.target()
        self.brain.update_record(self.root,self.record_id,expected_revision=current['revision'],
            progress=None,next_steps=['Changed elsewhere'],files=[],sources=[],actor='local')
        before=self.target();reply=self.save(self.b,'old-cas',reference)
        self.assertEqual('revision-conflict',reply['records'][0]['reason'])
        self.assertEqual(before,self.target());self.assertEqual(1,len(self.bodies()))

    def test_checked_source_drift_refuses_before_record_write(self):
        reference=self.reference();self.change_source();before=self.target()
        with self.assertRaisesRegex(self.brain.BrainError,'source.*changed|source.*image'):
            self.save(self.b,'drift',reference)
        self.assertEqual(before,self.target());self.assertEqual(1,len(self.bodies()))

    def test_completed_replay_never_reverifies_later_sources(self):
        self.change_source();reference=self.reference();self.save(self.b,'once',reference)
        before=self.target();chunks=self.bodies();self.change_source()
        with mock.patch.object(self.results,'source_fingerprints',side_effect=AssertionError('passive replay read source')):
            reply=self.save(self.b,'once',reference)
        self.assertTrue(reply['replayed']);self.assertEqual(before,self.target());self.assertEqual(chunks,self.bodies())

    def test_reference_fields_are_all_required(self):
        for field in ('record_id','expected_revision','source_fingerprints'):
            with self.subTest(field=field):
                ref=self.reference();del ref[field]
                # expected_revision on ordinary legacy learning remains valid.
                with self.assertRaises(self.brain.BrainError):self.save(self.b,'bad-'+field,ref)

    def test_missing_reference_never_creates(self):
        ref=self.reference(record_id='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa')
        reply=self.save(self.b,'missing',ref)
        self.assertEqual('record-not-found',reply['records'][0]['reason'])
        self.assertEqual(1,len(self.bodies()))

    def test_reference_cannot_drop_unchecked_source(self):
        other=self.root/'src/limit.txt';other.write_text('Cedar has a bounded input.\n')
        current=self.target()
        self.brain.update_record(self.root,self.record_id,expected_revision=current['revision'],
            progress=None,next_steps=[],files=[],sources=['src/limit.txt'],actor='local')
        before=self.target();reply=self.save(self.b,'subset',self.reference())
        self.assertEqual('reference-mismatch',reply['records'][0]['reason']);self.assertEqual(before,self.target())

    def test_person_and_unknown_attestation_are_held_even_same_claim(self):
        for attestation in ('person',None):
            with self.subTest(attestation=attestation):
                path,record,_=self.brain.find_record(self.root,self.record_id)
                for row in record['transitions']:
                    if row['from']=='observed' and row['to']=='verified':
                        row['reason']='Checked by a person [attestation:person]' if attestation else 'Legacy check'
                self.brain._write_record(self.root,record,path)
                before=self.target();self.change_source()
                reply=self.save(self.b,'protected-'+str(attestation),self.reference())
                self.assertEqual('person-or-unknown-attestation',reply['records'][0]['reason'])
                self.assertEqual(before,self.target())

    def test_generic_agent_reverify_cannot_downgrade_person(self):
        current=self.target()
        self.brain.update_record(self.root,self.record_id,expected_revision=current['revision'],
            progress=None,next_steps=[],files=[],sources=[],actor='local',reverify=True,attestation='person')
        current=self.target()
        with self.assertRaisesRegex(self.brain.BrainError,'person|attestation'):
            self.brain.update_record(self.root,self.record_id,expected_revision=current['revision'],
                progress=None,next_steps=[],files=[],sources=[],actor='local',reverify=True,attestation='agent')
        self.assertEqual(current,self.target())

    def test_legacy_and_explicit_alias_cannot_write_same_target_twice(self):
        self.change_source();before=self.target()
        legacy={**self.learning,'consequence':'Cedar reservations use eight slots.'}
        ref=self.reference(consequence='Cedar reservations use sixteen slots.')
        with self.assertRaisesRegex(self.brain.BrainError,'different learning identities'):
            self.results.record_result(self.root,self.a['id'],'mixed','auto',
                {'learnings':[legacy,ref],'verified':True},owner='local',reason='Checked Cedar')
        self.assertEqual(before,self.target());self.assertEqual(1,len(self.bodies()))

    def test_multiple_anchors_have_one_consistent_file_image(self):
        current=self.target()
        self.brain.update_record(self.root,self.record_id,expected_revision=current['revision'],progress=None,
            next_steps=[],files=[],sources=['src/rule.txt#second'],actor='local')
        self.change_source();self.learning['sources'].append('src/rule.txt#second')
        ref=self.reference()
        self.assertEqual(2,len(ref['source_fingerprints']))
        reply=self.save(self.b,'anchors',ref)
        self.assertEqual('updated',reply['records'][0]['state'])
        self.assertEqual(1,len(self.target()['source_fingerprints']))
        self.assertEqual(1,len(self.results.checked_fingerprints(self.root,self.learning['sources'])))

    def test_source_race_never_stamps_unseen_bytes(self):
        self.change_source();reference=self.reference();write=self.brain._write_record;changed=[]
        def raced(root,record,path):
            if record['id']==self.record_id and not changed:
                self.source.write_text('A concurrent Cedar edit was not inspected.\n');changed.append(True)
            return write(root,record,path)
        with mock.patch.object(self.brain,'_write_record',side_effect=raced):
            self.save(self.b,'raced',reference)
        self.assertEqual(reference['source_fingerprints'],self.target()['source_fingerprints'])
        self.assertFalse(self.brain.sources_are_fresh(self.root,self.target()))
        self.assertEqual(1,len(self.bodies()))

    def test_malformed_old_chunk_digest_does_not_crash_finished_save(self):
        metadata,body=self.brain._parse_chunk(self.old_chunk)
        metadata['source_digests']=[{'sha256':'0'*64}]
        self.brain.atomic_write(self.old_chunk,self.brain._render_chunk(metadata,body))
        self.change_source();reply=self.save(self.b,'bad-old-digest',self.reference())
        self.assertEqual('updated',reply['records'][0]['state'])
        self.assertTrue(reply['promotion']['blocked'])
        self.assertEqual(1,len(self.bodies()))

    def test_manual_predecessor_edit_is_preserved_and_promotion_reported_failed(self):
        metadata,body=self.brain._parse_chunk(self.old_chunk)
        body+='\nA team member added a separate checked exception.\n'
        self.brain.atomic_write(self.old_chunk,self.brain._render_chunk(metadata,body))
        self.change_source();reply=self.save(self.b,'manual-chunk',self.reference())
        self.assertTrue(reply['promotion']['failed'])
        self.assertEqual(body.strip(),self.brain._parse_chunk(self.old_chunk)[1].strip())
        self.assertEqual('active',self.brain._parse_chunk(self.old_chunk)[0]['status'])

    def test_failed_bank_write_replays_without_rechecking_record(self):
        self.change_source();ref=self.reference();write=self.brain.atomic_write
        def failed(path,text):
            if Path(path).parent==self.old_chunk.parent and Path(path)!=self.old_chunk:
                raise OSError('Injected Cedar bank failure')
            return write(path,text)
        # Existing promotion reports BrainError failures; an I/O error leaves
        # the complete record proof and its safely rolled-back bank write.
        with mock.patch.object(self.brain,'atomic_write',side_effect=failed):
            with self.assertRaises(OSError):self.save(self.b,'partial-bank',ref)
        revision=self.target()['revision']
        reply=self.save(self.b,'partial-bank',ref)
        self.assertTrue(reply['replayed']);self.assertEqual(revision,self.target()['revision'])
        self.assertEqual(1,sum(m['status']=='active' for m,_ in self.bodies().values()))
        self.assertEqual('superseded',self.brain._parse_chunk(self.old_chunk)[0]['status'])

    def test_concurrent_reviews_of_one_checked_revision_have_one_writer(self):
        self.change_source();ref=self.reference()
        c=self.brain.create_task(self.root,'cedar/c','Recheck same Cedar claim',[],[],owner='local')
        with ThreadPoolExecutor(max_workers=2) as pool:
            futures=[pool.submit(self.save,t,r,ref) for t,r in [(self.b,'review-b'),(c,'review-c')]]
            replies=[future.result() for future in futures]
        self.assertEqual(['differs','updated'],sorted(r['records'][0]['state'] for r in replies))
        self.assertEqual(3,self.target()['revision']);self.assertEqual(2,len(self.bodies()))

    def test_source_digest_cli_is_read_only_and_crlf_neutral(self):
        self.source.write_bytes(b'Cedar reservation uses four slots.\r\n')
        before={str(p.relative_to(self.root)):p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        result=subprocess.run([sys.executable,'-B',str(SCRIPTS/'context.py'),'--root',str(self.root),
            'source-fingerprints','--source','src/rule.txt','--json'],capture_output=True,text=True,timeout=10)
        self.assertEqual(0,result.returncode,result.stderr)
        self.assertEqual(self.target()['source_fingerprints'],json.loads(result.stdout)['source_fingerprints'])
        after={str(p.relative_to(self.root)):p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        self.assertEqual(before,after)


from tests.hook_shell import BASH
from tests.test_memory_native_capture import NativeCaptureFixture


class NativeReferenceTests(NativeCaptureFixture):
    def test_pre_answer_review_notice_command_runs_from_a_subdirectory(self):
        self.begin();self.stop();self.save();self.git('checkout','-qb','research-b')
        self.call('start','--task-id','research-b','--goal','Inspect Amber helper','--json')
        self.env['CONTEXT_NATIVE_MEMORY_MAINTENANCE']='review'
        subdirectory=self.root/'src';subdirectory.mkdir()
        found=subprocess.run([sys.executable,'-B',str(self.root/'memory-bank/scripts/context.py'),
            '--root',str(self.root),'retrieve','Amber retry boundary','--task-id','research-b','--json'],
            cwd=subdirectory,env=self.env,capture_output=True,text=True,timeout=15)
        self.assertEqual(0,found.returncode,found.stderr)
        notice=json.loads(found.stdout)['memory_review']
        command=notice.split('run ',1)[1].split('. Read',1)[0]
        # Quoted for bash, which the hosts run it with - Git Bash on Windows;
        # cmd.exe, which shell=True starts there, kept the quotes in the path.
        if BASH is None:self.skipTest('no bash to run the command with')
        result=subprocess.run([BASH,'-c',command],cwd=subdirectory,env=self.env,capture_output=True,text=True,timeout=10)
        self.assertEqual(0,result.returncode,result.stderr)
        self.assertEqual('unavailable',json.loads(result.stdout)['state'])

    def test_pre_answer_review_cli_applies_checked_claim_then_native_saves_progress(self):
        self.begin();self.stop();self.save();record=self.records('findings')[0]
        self.git('checkout','-qb','research-b')
        self.call('start','--task-id','research-b','--goal','Extend Amber retry policy','--json')
        self.env['CONTEXT_NATIVE_MEMORY_MAINTENANCE']='review'
        self.call('native-prompt','--host','codex','--task-id','research-b','--json',data={
            'session_id':'session-b','prompt':'Extend Amber retry policy.'})
        found=json.loads(self.call('retrieve','Amber retry boundary','--task-id','research-b','--json').stdout)
        self.assertIn('BEFORE your final answer',found['memory_review'])
        plain=self.call('retrieve','Amber retry boundary','--task-id','research-b')
        self.assertIn('MEMORY_REVIEW_STAGE',plain.stdout.decode())
        (self.root/'rules.md').write_text('# Routing\nThe Amber dispatcher retries only idempotent requests.\nNew helper added.\n')
        plan=json.loads(self.call('memory-review','--task-id','research-b','--json').stdout)
        pins=json.loads(self.call('source-fingerprints','--source','rules.md','--json').stdout)['source_fingerprints']
        response=self.call('memory-review','--task-id','research-b','--input','-','--json',data={
            'plan_id':plan['plan_id'],'verified':True,'decisions':[{
                'record_id':record['id'],'action':'confirm','source_fingerprints':pins}]})
        self.assertEqual('applied',json.loads(response.stdout)['state'])
        self.assertEqual(3,self.records('findings')[0]['revision'])
        self.call('native-stop','--host','codex','--task-id','research-b',data={
            'hook_event_name':'Stop','session_id':'session-b'})
        request,state=next((f,json.loads(f.read_text())) for f in
            (self.root/'project-brain/local/native-results').glob('*.json') if json.loads(f.read_text())['task_id']=='research-b')
        self.call('native-result','--request',request.stem,'--token',state['token'],'--input','-','--json',data={
            'progress':'Checked Amber changes and completed the explicit review stage.','verified':False,'learnings':[]})
        self.assertEqual(1,len(self.records('findings')))

    def test_normal_retrieval_supplies_bounded_maintenance_hint_to_native_stop(self):
        self.begin();self.stop();self.save()
        record=self.records('findings')[0]
        self.git('checkout','-qb','research-b')
        self.call('start','--task-id','research-b','--goal','Extend Amber retry policy','--json')
        self.env['CONTEXT_NATIVE_MEMORY_MAINTENANCE']='encountered'
        self.call('native-prompt','--host','codex','--task-id','research-b','--json',data={
            'session_id':'session-b','prompt':'Extend Amber retry policy.'})
        self.call('retrieve','Amber retry boundary','--task-id','research-b','--json')
        self.assertTrue(list((self.root/'project-brain/local/native-maintenance').glob('*.json')))
        (self.root/'rules.md').write_text('# Routing\nAmber now retries an idempotent request at most once.\n')
        response=self.call('native-stop','--host','codex','--task-id','research-b',data={
            'hook_event_name':'Stop','session_id':'session-b'})
        self.assertIn(record['id'],response.stdout.decode())
        self.assertIn('Experimental maintenance',response.stdout.decode())
        self.assertEqual(record,self.records('findings')[0])

    def test_fresh_native_session_corrects_existing_record_and_capture_verifies_it(self):
        self.begin();self.stop();first=json.loads(self.save().stdout)
        record=self.records('findings')[0]
        self.git('checkout','-qb','research-b')
        self.call('start','--task-id','research-b','--goal','Recheck the existing Amber claim','--json')
        (self.root/'rules.md').write_text('# Routing\nAmber retries one idempotent request at most once.\n')
        self.call('native-prompt','--host','codex','--task-id','research-b','--json',data={
            'session_id':'session-b','prompt':'Extend the dispatcher while checking its old rule.'})
        self.call('native-stop','--host','codex','--task-id','research-b',data={
            'hook_event_name':'Stop','session_id':'session-b','last_assistant_message':'The existing rule was corrected.'})
        task=json.loads(self.call('get','--task-id','research-b','--json').stdout)
        image=json.loads(self.call('source-fingerprints','--source','rules.md','--json').stdout)['source_fingerprints']
        payload={'progress':'Corrected the checked Amber rule.','verified':True,'learnings':[
            {**self.payload['learnings'][0],'consequence':'Amber retries one idempotent request at most once.',
             'record_id':record['id'],'expected_revision':record['revision'],'source_fingerprints':image}]}
        state_path,state=next((p,json.loads(p.read_text())) for p in
            (self.root/'project-brain/local/native-results').glob('*.json') if json.loads(p.read_text())['task_id']=='research-b')
        before={str(p.relative_to(self.root)) for p in self.root.rglob('*') if p.is_file()}
        saved=self.call('native-result','--request',state_path.stem,'--token',state['token'],'--input','-','--json',data=payload)
        result=json.loads(saved.stdout);self.assertEqual('updated',result['records'][0]['state'])
        self.assertEqual(record['id'],result['records'][0]['id'])
        sys.path.insert(0,str(ROOT/'scripts'))
        try:
            from memory_native_ab_evidence import capture
            observed="python3 memory-bank/scripts/context.py native-result --request "+state_path.stem+' --token '+state['token']+" --input - --json <<'NATIVE_RESULT'\n"+json.dumps(payload)+'\nNATIVE_RESULT'
            events=self.root/'observed-events.jsonl'
            events.write_text(json.dumps({'type':'item.completed','item':{'type':'command_execution','exit_code':0,
                'command':observed,'aggregated_output':saved.stdout.decode()}})+'\n')
            evidence=capture(self.root,{'task_id':'research-b','task_uuid':task['task_uuid']},'session-b',events,before)
        finally:sys.path.pop(0)
        self.assertTrue(evidence['native_saved'],evidence)
        self.assertEqual([record['id']],evidence['record_ids'])
        self.assertEqual([],evidence['new_chunks'])


class CitationImageTests(unittest.TestCase):
    def test_native_evidence_normalizes_anchors_and_consistent_duplicate_hashes(self):
        sys.path.insert(0,str(ROOT/'scripts'))
        try:
            from memory_native_ab_evidence import citations
        finally:sys.path.pop(0)
        import hashlib
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/'rule.md').write_bytes(b'Cedar rule\r\n')
            row={'path':'rule.md','sha256':hashlib.sha256(b'Cedar rule\n').hexdigest()}
            meta={'sources':['rule.md#one','rule.md#two'],'source_fingerprints':[row,row]}
            self.assertTrue(citations(root,meta,'source_fingerprints'))
            meta['source_fingerprints']=[row,{**row,'sha256':'0'*64}]
            self.assertFalse(citations(root,meta,'source_fingerprints'))


class OrdinaryMaintenanceTests(RevalidationFixture):
    def setUp(self):
        super().setUp()
        sys.path.insert(0, str(SCRIPTS))
        self.addCleanup(sys.path.remove, str(SCRIPTS))
        self.native = importlib.import_module('native_results')
        self.retrieval = importlib.import_module('context_retrieval')
        self.state = {'owner':'local','task_id':'cedar/b','task_uuid':self.b['id']}

    def enable(self):
        cfg=json.loads(self.config.read_text());cfg['native_memory_maintenance']='encountered'
        self.config.write_text(json.dumps(cfg))

    def encounter(self, target=None, **changes):
        target=target or self.old_chunk
        relative=target.relative_to(self.root).as_posix()
        cap=self.retrieval._IndexImageCapsule({'manifest':'published.json','task_uuid':self.b['id'],
            'semantic':[{'path':relative}]}, [(relative,('title','summary',target.read_text()))])
        cap.update(changes)
        self.native.observe_maintenance(self.root,cap,'cedar/b',owner='local')

    def hint(self, **changes):
        return self.native.maintenance_instruction(self.root,{**self.state,**changes})

    def test_default_off_neither_captures_nor_adds_instructions(self):
        self.encounter();self.change_source()
        self.assertEqual('',self.hint())
        self.assertFalse((self.root/'project-brain/local/native-maintenance').exists())

    def test_same_task_encounter_offers_exact_reference_only_after_source_change(self):
        self.enable();self.encounter()
        self.assertEqual('',self.hint());self.change_source()
        hint=self.hint()
        self.assertIn(self.record_id,hint);self.assertIn('expected_revision',hint)
        self.assertIn('original task work already checked',hint)
        self.assertNotIn('"source_fingerprints"',hint)
        self.assertEqual('',self.hint(owner='someone-else'))
        self.assertEqual('',self.hint(task_id='other/task'))
        self.assertEqual('',self.hint(task_uuid=self.a['id']))

    def test_indexed_unseen_image_and_packed_out_rows_are_not_captured(self):
        self.enable();self.encounter(semantic=[]);self.change_source()
        self.assertEqual('',self.hint())
        relative=self.old_chunk.relative_to(self.root).as_posix()
        cap=self.retrieval._IndexImageCapsule({'manifest':'x','semantic':[{'path':relative}]},
            [(relative,('t','s','older unseen text'))])
        self.native.observe_maintenance(self.root,cap,'cedar/b',owner='local')
        self.assertEqual('',self.hint())

    def test_unpublished_retrieval_is_not_captured(self):
        self.enable();self.encounter(manifest=None);self.change_source()
        self.assertEqual('',self.hint())

    def test_newer_record_behind_unchanged_chunk_is_not_adopted(self):
        self.enable();self.encounter();self.change_source();target=self.target()
        self.brain.update_record(self.root,self.record_id,expected_revision=target['revision'],
            progress='Changed since encounter.',next_steps=[],files=[],sources=[],actor='local')
        self.assertEqual('',self.hint())

    def test_changed_encounter_image_is_omitted(self):
        self.enable();self.encounter();self.change_source()
        self.old_chunk.write_text(self.old_chunk.read_text()+'manual change\n')
        self.assertEqual('',self.hint())

    def test_direct_brain_reference_rechecks_attestation(self):
        self.enable();target,record,_=self.brain.find_record(self.root,self.record_id)
        self.encounter(target);self.change_source();self.assertIn(self.record_id,self.hint())
        for row in record['transitions']:
            if row['to']=='verified':row['reason']='Human check [attestation:person]'
        self.brain._write_record(self.root,record,target);self.encounter(target)
        self.assertEqual('',self.hint())

    def test_corrupt_expired_or_linked_packet_does_not_break_progress(self):
        self.enable();self.encounter();self.change_source()
        packet=self.native._maintenance_file(self.root,'local','cedar/b')
        value=json.loads(packet.read_text());value['at']=0;packet.write_text(json.dumps(value))
        self.assertEqual('',self.hint());packet.write_text('{')
        self.assertEqual('',self.hint());packet.unlink();packet.symlink_to(self.source)
        self.assertEqual('',self.hint())

    def test_hint_is_present_in_cached_short_native_instruction(self):
        self.enable();self.encounter();self.change_source()
        text=self.native._instruction(self.root,'a'*64,{**self.state,'token':'b'*32},host='codex',full=False)
        self.assertIn(self.record_id,text);self.assertIn('do not start new research',text)

    def test_crlf_encounter_preserves_reference_admission(self):
        self.enable();self.old_chunk.write_bytes(self.old_chunk.read_bytes().replace(b'\n',b'\r\n'))
        self.encounter();self.change_source()
        self.assertIn(self.record_id,self.hint())

    def test_warming_encounter_does_not_provision_task(self):
        self.enable();self.change_source()
        relative=self.old_chunk.relative_to(self.root).as_posix()
        cap=self.retrieval._IndexImageCapsule({'manifest':'x','task_uuid':None,
            'semantic':[{'path':relative}]},[(relative,('t','s',self.old_chunk.read_text()))])
        tasks=list((self.root/'project-brain/dynamic/tasks').glob('*.md'))
        self.native.observe_maintenance(self.root,cap,'unprovisioned',owner='local')
        self.assertIn(self.record_id,self.hint(task_id='unprovisioned',task_uuid=None))
        self.assertEqual(tasks,list((self.root/'project-brain/dynamic/tasks').glob('*.md')))

    def test_privacy_and_actor_policy_revocation_holds_encounter(self):
        self.enable();self.encounter();self.change_source()
        cfg=json.loads(self.config.read_text());cfg['allowed_privacy']=['public']
        self.config.write_text(json.dumps(cfg));self.assertEqual('',self.hint())
        cfg['allowed_privacy']=['team'];cfg['owners']=['another-owner']
        self.config.write_text(json.dumps(cfg));self.assertEqual('',self.hint())

    def test_authority_policy_revocation_holds_encounter(self):
        self.enable();self.encounter();self.change_source()
        self.assertIn(self.record_id,self.hint())
        cfg=json.loads(self.config.read_text());cfg['allowed_authority']=['observed']
        self.config.write_text(json.dumps(cfg));self.assertEqual('',self.hint())

    def test_linked_record_whole_image_secret_policy_is_rechecked(self):
        self.enable();target,record,_=self.brain.find_record(self.root,self.record_id)
        record['next_steps']=['password = "synthetic-restricted-value"']
        # Simulate a legacy raw image; sanctioned writes already refuse this.
        target.write_text('---\n'+json.dumps(record)+'\n---\n'+self.brain._record_body(record))
        chunk,body=self.brain._parse_chunk(self.old_chunk)
        for pin in chunk['source_digests']:
            if pin['path']==target.relative_to(self.root).as_posix():
                pin['sha256']=self.brain.fingerprint(self.root,pin['path'])['sha256']
        self.old_chunk.write_text('---\n'+json.dumps(chunk)+'\n---\n'+body)
        self.encounter();self.change_source();self.assertEqual('',self.hint())


class PreAnswerReviewTests(RevalidationFixture):
    def setUp(self):
        super().setUp()
        sys.path.insert(0,str(SCRIPTS));self.addCleanup(sys.path.remove,str(SCRIPTS))
        self.native=importlib.import_module('native_results')
        self.retrieval=importlib.import_module('context_retrieval')
        self.state={'owner':'local','task_id':'cedar/b','task_uuid':self.b['id']}

    encounter=OrdinaryMaintenanceTests.encounter

    def enable(self):
        cfg=json.loads(self.config.read_text());cfg['native_memory_maintenance']='review'
        self.config.write_text(json.dumps(cfg))

    def plan(self):
        self.enable();self.encounter();self.change_source()
        return self.native.memory_review_plan(self.root,'cedar/b',owner='local')

    def decision(self,plan,action='confirm',**values):
        row={'record_id':self.record_id,'action':action}
        if action=='defer':row['reason']='Needs a separate source review.'
        else:row['source_fingerprints']=self.brain.source_fingerprints(self.root,self.learning['sources'])
        row.update(values)
        return {'plan_id':plan['plan_id'],'verified':True,'decisions':[row]}

    def apply(self,data,**values):
        return self.native.memory_review_apply(self.root,values.get('task_id','cedar/b'),data,owner=values.get('owner','local'))

    def authority(self):
        return {str(p.relative_to(self.root)):p.read_bytes() for prefix in
            ('project-brain/dynamic','project-brain/control','project-brain/indexes','memory-bank/chunks')
            for p in (self.root/prefix).rglob('*') if p.is_file()}

    def test_disabled_review_does_not_write_or_provision(self):
        before=self.authority()
        with self.assertRaisesRegex(self.brain.BrainError,'disabled'):
            self.native.memory_review_plan(self.root,'cedar/b',owner='local')
        self.assertEqual(before,self.authority())

    def test_plan_writes_only_local_metadata_and_preserves_authority(self):
        self.enable();self.encounter();self.change_source();before=self.authority()
        plan=self.native.memory_review_plan(self.root,'cedar/b',owner='local')
        self.assertEqual('review-required',plan['state']);self.assertEqual(self.record_id,plan['records'][0]['record_id'])
        self.assertEqual(before,self.authority());self.assertFalse((self.root/'memory-bank/local/context.db').exists())

    def test_unprovisioned_review_is_unavailable_without_creating_task(self):
        self.enable();before=self.authority()
        with self.assertRaises(self.brain.BrainRecordNotFound):
            self.native.memory_review_plan(self.root,'unknown/task',owner='local')
        self.assertEqual(before,self.authority())

    def test_confirm_existing_claim_applies_before_native_save_and_preserves_history(self):
        plan=self.plan();reply=self.apply(self.decision(plan))
        self.assertEqual('applied',reply['state']);self.assertEqual('agent',reply['attestation'])
        self.assertEqual(self.record_id,reply['result']['records'][0]['id']);self.assertEqual(3,self.target()['revision'])
        self.assertEqual('superseded',self.brain._parse_chunk(self.old_chunk)[0]['status'])
        self.assertEqual([],self.brain.audit_bank(self.root)['source_changed'])
        self.assertEqual(self.b['revision'],self.brain.get_record(self.root,self.b['id'])['revision'])

    def test_correct_applies_explicit_checked_consequence(self):
        plan=self.plan();reply=self.apply(self.decision(plan,'correct',consequence='Cedar now uses eight slots.'))
        self.assertEqual('applied',reply['state']);self.assertEqual('Cedar now uses eight slots.',self.target()['progress'])

    def test_defer_does_not_write_shared_state_or_extend_review(self):
        plan=self.plan();before=self.authority()
        with mock.patch.object(self.native,'record_result',side_effect=AssertionError('defer wrote knowledge')):
            reply=self.apply(self.decision(plan,'defer'))
        self.assertEqual('deferred',reply['state']);self.assertEqual('not-performed',reply['attestation'])
        self.assertEqual(before,self.authority())

    def test_missing_and_duplicate_decisions_are_held(self):
        plan=self.plan();request=self.decision(plan);before=self.authority()
        for rows in [[],request['decisions']*2]:
            with self.subTest(rows=len(rows)),self.assertRaises(self.brain.BrainError):
                self.apply({**request,'decisions':rows})
        self.assertEqual(before,self.authority())

    def test_agent_attestation_required_for_checked_actions(self):
        plan=self.plan();data=self.decision(plan);data['verified']=False
        with self.assertRaisesRegex(self.brain.BrainError,'attestation'):self.apply(data)
        self.assertEqual(2,self.target()['revision'])

    def test_source_drift_and_raw_same_revision_record_edit_are_fenced(self):
        plan=self.plan();data=self.decision(plan);self.change_source();before=self.authority()
        with self.assertRaisesRegex(self.brain.BrainError,'source-changed'):self.apply(data)
        self.assertEqual(before,self.authority())

    def test_raw_same_revision_claim_edit_is_fenced(self):
        plan=self.plan();data=self.decision(plan)
        target,record,_=self.brain.find_record(self.root,self.record_id)
        target.write_text(target.read_text()+'Manual body change.\n');before=self.authority()
        with self.assertRaisesRegex(self.brain.BrainError,'claim-or-source-changed'):self.apply(data)
        self.assertEqual(before,self.authority())

    def test_wrong_actor_or_task_and_policy_change_are_refused(self):
        plan=self.plan();data=self.decision(plan)
        for options in [{'owner':'someone-else'},{'task_id':'cedar/a'}]:
            with self.subTest(options=options),self.assertRaises(self.brain.BrainError):self.apply(data,**options)
        cfg=json.loads(self.config.read_text());cfg['allowed_authority']=['observed'];self.config.write_text(json.dumps(cfg))
        with self.assertRaisesRegex(self.brain.BrainError,'scope-changed'):self.apply(data)

    def test_first_decisions_are_frozen_and_completed_replay_is_passive(self):
        plan=self.plan();data=self.decision(plan);self.apply(data);before=self.authority()
        changed=self.decision(plan,'correct',consequence='A different decision.')
        with self.assertRaisesRegex(self.brain.BrainError,'frozen'):self.apply(changed)
        self.source.unlink()
        stored=self.native._review_file(self.root,'local','cedar/b',plan['plan_id'])
        value=json.loads(stored.read_text());value['at']=0;stored.write_text(json.dumps(value))
        reply=self.apply(data);self.assertTrue(reply['replayed']);self.assertEqual(before,self.authority())

    def test_new_plan_source_pins_cannot_claim_a_different_checked_image(self):
        plan=self.plan();data=self.decision(plan);data['decisions'][0]['source_fingerprints'][0]['sha256']='0'*64
        with self.assertRaisesRegex(self.brain.BrainError,'does-not-match'):self.apply(data)

    def test_pre_answer_obligation_requires_supported_task_and_encounter(self):
        self.enable();self.assertIsNone(self.native.review_notice(self.root,'cedar/b',owner='local'))
        self.encounter();text=self.native.review_notice(self.root,'cedar/b',owner='local')
        self.assertIn('BEFORE your final answer',text);self.assertIn('memory-review',text)
        self.assertEqual('',self.native.maintenance_instruction(self.root,self.state))

    def test_unhashable_input_and_corrupt_plan_are_bounded_refusals(self):
        plan=self.plan();data=self.decision(plan);before=self.authority()
        for field in ['record_id','action']:
            bad=copy.deepcopy(data);bad['decisions'][0][field]=[]
            with self.subTest(field=field),self.assertRaises(self.brain.BrainError):self.apply(bad)
        stored=self.native._review_file(self.root,'local','cedar/b',plan['plan_id'])
        damaged=json.loads(stored.read_text());damaged['records']=[None];stored.write_text(json.dumps(damaged))
        with self.assertRaises(self.brain.BrainError):self.apply(data)
        self.assertEqual(before,self.authority())

    def test_review_plan_larger_than_result_proof_envelope_is_supported(self):
        plan=self.plan();stored=self.native._review_file(self.root,'local','cedar/b',plan['plan_id'])
        stored.write_text(stored.read_text()+' '*5000)
        self.assertGreater(stored.stat().st_size,4096)
        self.assertEqual('applied',self.apply(self.decision(plan))['state'])

    def test_missing_plan_header_and_corrupt_receipt_refuse_without_traceback(self):
        plan=self.plan();data=self.decision(plan)
        stored=self.native._review_file(self.root,'local','cedar/b',plan['plan_id'])
        original=json.loads(stored.read_text());damaged={k:v for k,v in original.items() if k!='task_revision'}
        stored.write_text(json.dumps(damaged))
        with self.assertRaises(self.brain.BrainError):self.apply(data)
        stored.write_text(json.dumps(original));defer=self.decision(plan,'defer');self.apply(defer)
        receipt=self.native._review_file(self.root,'local','cedar/b',plan['plan_id'],receipt=True)
        value=json.loads(receipt.read_text());del value['state'];receipt.write_text(json.dumps(value))
        with self.assertRaises(self.brain.BrainError):self.apply(defer)
        value['state']=[];receipt.write_text(json.dumps(value))
        with self.assertRaises(self.brain.BrainError):self.apply(defer)

    def test_forged_deferred_status_cannot_bypass_checked_claim_fences(self):
        plan=self.plan();data=self.decision(plan)
        with mock.patch.object(self.native,'record_result',side_effect=OSError('Before canonical save')):
            with self.assertRaises(OSError):self.apply(data)
        receipt=self.native._review_file(self.root,'local','cedar/b',plan['plan_id'],receipt=True)
        value=json.loads(receipt.read_text());value['state']='deferred';receipt.write_text(json.dumps(value))
        stored=self.native._review_file(self.root,'local','cedar/b',plan['plan_id'])
        value=json.loads(stored.read_text());value['at']=0;stored.write_text(json.dumps(value))
        target,_,_=self.brain.find_record(self.root,self.record_id);target.write_text(target.read_text()+'Changed body.\n')
        before=self.authority()
        with self.assertRaises(self.brain.BrainError):self.apply(data)
        self.assertEqual(before,self.authority())


class SemanticReviewTests(RevalidationFixture):
    """Controller contracts, not evidence that an LLM judged claims correctly."""
    def setUp(self):
        super().setUp()
        sys.path.insert(0,str(SCRIPTS));self.addCleanup(sys.path.remove,str(SCRIPTS))
        self.native=importlib.import_module('native_results')
        self.retrieval=importlib.import_module('context_retrieval')
        self.state={'owner':'local','task_id':'cedar/b','task_uuid':self.b['id']}
        self.enable()

    encounter=OrdinaryMaintenanceTests.encounter
    decision=PreAnswerReviewTests.decision
    authority=PreAnswerReviewTests.authority

    def enable(self):
        cfg=json.loads(self.config.read_text());cfg['native_memory_maintenance']='review-verified'
        self.config.write_text(json.dumps(cfg))

    def stage(self,**changes):
        self.encounter();self.change_source()
        plan=self.native.memory_review_plan(self.root,'cedar/b',owner='local')
        data=self.decision(plan,'correct',consequence='Cedar now uses eight slots.',**changes)
        reply=self.native.memory_review_apply(self.root,'cedar/b',data,owner='local')
        return plan,reply

    def export(self,reply):
        return self.native.semantic_review_export(self.root,'cedar/b',reply['proposal_id'],reply['proposal_sha256'],owner='local')

    def verdict(self,packet,decision='accept'):
        records=[]
        for record in packet['records']:
            citation={'path':packet['sources'][0]['path'],'sha256':packet['sources'][0]['sha256'],
                      'quote':packet['sources'][0]['text'].splitlines()[0]}
            review={'record_id':record['record_id'],'boundary_checks':[
                {'case':'Check the bounded slot boundary.','status':'supported','evidence':[citation]}]}
            for key,status in [('old_clauses','changed-with-evidence'),('new_clauses','supported')]:
                review[key]=[{'id':row['id'],'sha256':row['sha256'],'status':status,
                              'reason':'Checked the complete current source.','evidence':[citation]} for row in record[key]]
            records.append(review)
        return {'profile':self.native.SEMANTIC_VERDICT,'packet_sha256':packet['sha256'],
                'proposal_sha256':packet['proposal_sha256'],'verdict':decision,'records':records}

    def authorization(self,packet,**changes):
        return {'profile':self.native.SEMANTIC_AUTH,'packet_sha256':packet['sha256'],
                'primary_request':'a'*64,'reviewer_session':'independent-review-session',
                'verdict':self.verdict(packet),**changes}

    def publish(self,reply,packet,authorization=None,**state_changes):
        import hashlib
        state={'phase':'saved','owner':'local','task_id':'cedar/b','task_uuid':self.b['id'],
               'session':hashlib.sha256(b'primary-session').hexdigest(),**state_changes}
        with mock.patch.object(self.native,'_read',return_value=state),mock.patch.object(self.native,'_proven',return_value=True):
            return self.native.semantic_review_apply(self.root,'cedar/b',reply['proposal_id'],reply['proposal_sha256'],
                packet,authorization or self.authorization(packet),owner='local')

    def test_staging_is_proposal_only_even_when_old_review_mode_reenabled(self):
        self.encounter();self.change_source()
        plan=self.native.memory_review_plan(self.root,'cedar/b',owner='local')
        self.assertTrue(plan['verification_required']);before=self.authority()
        with mock.patch.dict(os.environ,CONTEXT_NATIVE_MEMORY_MAINTENANCE='review'),mock.patch.object(self.native,'record_result',side_effect=AssertionError('primary published')):
            reply=self.native.memory_review_apply(self.root,'cedar/b',self.decision(plan),owner='local')
        self.assertEqual('proposal-staged',reply['state']);self.assertFalse(reply['applied'])
        self.assertEqual(before,self.authority())

    def test_complete_full_source_packet_does_not_change_authority(self):
        _,reply=self.stage();before=self.authority();packet=self.export(reply)
        self.assertEqual(before,self.authority());self.assertEqual(self.source.read_bytes().decode('utf-8'),packet['sources'][0]['text'])
        self.assertEqual(self.record_id,packet['records'][0]['record_id'])
        self.assertEqual(1,len(packet['records'][0]['old_memories']))

    def test_clause_checklist_covers_exact_unicode_and_negation(self):
        text='Keep NBSP\u00a0and formfeed\f. Do not grant *.\nEmpty arrays deny.'
        rows=self.native._semantic_clauses(text,'old')
        self.assertEqual(text,''.join(row['text'] for row in rows));self.assertGreater(len(rows),1)
        self.assertIn('\u00a0',rows[0]['text']);self.assertIn('\f',rows[0]['text'])

    def test_claim_rendering_mismatch_never_stages(self):
        self.encounter();self.change_source();plan=self.native.memory_review_plan(self.root,'cedar/b',owner='local')
        before=self.authority()
        with self.assertRaisesRegex(self.brain.BrainError,'rendering-would-change'):
            self.native.memory_review_apply(self.root,'cedar/b',self.decision(plan,'correct',consequence='Literal\u00a0NBSP remains.'),owner='local')
        self.assertEqual(before,self.authority())

    def test_accept_updates_original_identity_and_preserves_history(self):
        _,reply=self.stage();packet=self.export(reply);old_body=self.brain._parse_chunk(self.old_chunk)[1]
        result=self.publish(reply,packet)
        self.assertTrue(result['applied']);self.assertEqual('agent',result['attestation'])
        self.assertEqual(self.record_id,result['result']['records'][0]['id'])
        self.assertEqual('Cedar now uses eight slots.',self.target()['progress'])
        metadata,body=self.brain._parse_chunk(self.old_chunk)
        self.assertEqual(old_body,body);self.assertEqual('superseded',metadata['status'])
        self.assertEqual(1,sum(m['status']=='active' for m,_ in self.bodies().values()))

    def test_reject_and_uncertain_preserve_authority(self):
        _,reply=self.stage();packet=self.export(reply);before=self.authority()
        for verdict in ('reject','unresolved'):
            with self.subTest(verdict=verdict):
                # Direct checker is pure; publication freezes its first verdict.
                outcome=self.native.semantic_verdict_check(packet,self.verdict(packet,verdict))
                self.assertFalse(outcome['publish'])
        auth=self.authorization(packet,verdict=self.verdict(packet,'reject'))
        self.assertEqual('reject',self.publish(reply,packet,auth)['state']);self.assertEqual(before,self.authority())
        with self.assertRaisesRegex(self.brain.BrainError,'already-frozen'):
            self.publish(reply,packet)
        self.assertEqual(before,self.authority())

    def test_accept_with_unresolved_or_contradicted_clause_is_held(self):
        _,reply=self.stage();packet=self.export(reply)
        for status in ('unresolved','contradicted'):
            verdict=self.verdict(packet);verdict['records'][0]['new_clauses'][0]['status']=status
            self.assertFalse(self.native.semantic_verdict_check(packet,verdict)['publish'])
        verdict=self.verdict(packet);verdict['records'][0]['boundary_checks'][0]['status']='unresolved'
        self.assertFalse(self.native.semantic_verdict_check(packet,verdict)['publish'])

    def test_coverage_binding_and_literal_quotes_are_required(self):
        _,reply=self.stage();packet=self.export(reply)
        cases=[]
        v=self.verdict(packet);v['records'][0]['old_clauses']=[];cases.append(v)
        v=self.verdict(packet);v['records'][0]['new_clauses'][0]['sha256']='b'*64;cases.append(v)
        v=self.verdict(packet);v['records'][0]['new_clauses'][0]['evidence'][0]['quote']='fabricated citation';cases.append(v)
        v=self.verdict(packet);v['records'][0]['boundary_checks']=[];cases.append(v)
        v=self.verdict(packet);v['packet_sha256']='b'*64;cases.append(v)
        v=self.verdict(packet);v['records']*=2;cases.append(v)
        for verdict in cases:
            with self.subTest(verdict=verdict),self.assertRaises(self.brain.BrainError):
                self.native.semantic_verdict_check(packet,verdict)

    def test_primary_must_finish_in_another_session(self):
        import hashlib
        _,reply=self.stage();packet=self.export(reply);before=self.authority()
        for changes in ({'phase':'requested'},{'session':hashlib.sha256(b'independent-review-session').hexdigest()}):
            with self.subTest(changes=changes),self.assertRaisesRegex(self.brain.BrainError,'unfinished-or-same-session'):
                self.publish(reply,packet,**changes)
        with mock.patch.object(self.native,'_read',return_value=None),self.assertRaises(self.brain.BrainError):
            self.native.semantic_review_apply(self.root,'cedar/b',reply['proposal_id'],reply['proposal_sha256'],
                packet,self.authorization(packet),owner='local')
        self.assertEqual(before,self.authority())

    def test_started_publication_does_not_rebase_an_unreviewed_task_frontier(self):
        _,reply=self.stage();packet=self.export(reply);authorization=self.authorization(packet)
        with mock.patch.object(self.results,'update_record',side_effect=OSError('interrupted before effect')):
            with self.assertRaisesRegex(OSError,'interrupted before effect'):
                self.publish(reply,packet,authorization)
        task=self.brain.get_record(self.root,self.b['id'])
        self.brain.update_record(self.root,task['id'],expected_revision=task['revision'],
            progress='Another task update.',next_steps=[],files=[],sources=[],actor='local')
        before=self.authority()
        with self.assertRaisesRegex(self.brain.BrainError,'prepared_stale_no_write'):
            self.publish(reply,packet,authorization)
        self.assertEqual(before,self.authority())

    def test_source_record_task_and_bank_drift_are_rejected(self):
        _,reply=self.stage();packet=self.export(reply);before=self.authority()
        self.change_source()
        with self.assertRaises(self.brain.BrainError):self.publish(reply,packet)
        self.source.write_bytes(packet['sources'][0]['text'].encode('utf-8'))
        original=self.old_chunk.read_bytes();self.old_chunk.write_bytes(original+b'Changed body.\n')
        with self.assertRaisesRegex(self.brain.BrainError,'frontier-changed'):self.publish(reply,packet)
        self.old_chunk.write_bytes(original)
        self.brain.update_record(self.root,self.b['id'],expected_revision=self.b['revision'],progress='New work',next_steps=[],files=[],sources=[],actor='local')
        with self.assertRaisesRegex(self.brain.BrainError,'task-or-packet'):self.publish(reply,packet)
        self.assertEqual(before['memory-bank/chunks/'+self.old_chunk.name],self.old_chunk.read_bytes())

    def test_complete_replay_outlives_primary_state_source_and_expiry(self):
        _,reply=self.stage();packet=self.export(reply);auth=self.authorization(packet)
        self.publish(reply,packet,auth);before=self.authority();self.source.unlink()
        with mock.patch.object(self.native,'_read',side_effect=AssertionError('read old challenge')),mock.patch.object(self.native.time,'time',return_value=10**15):
            repeated=self.native.semantic_review_apply(self.root,'cedar/b',reply['proposal_id'],reply['proposal_sha256'],packet,auth,owner='local')
        self.assertTrue(repeated['result']['replayed']);self.assertEqual(before,self.authority())

    def test_first_authorization_is_frozen_after_success(self):
        _,reply=self.stage();packet=self.export(reply);self.publish(reply,packet);before=self.authority()
        auth=self.authorization(packet,reviewer_session='different-reviewer')
        with self.assertRaisesRegex(self.brain.BrainError,'already-frozen'):self.publish(reply,packet,auth)
        self.assertEqual(before,self.authority())

    def test_partial_promotion_can_resume_without_rewriting_record(self):
        _,reply=self.stage();packet=self.export(reply);auth=self.authorization(packet)
        with mock.patch.object(self.brain,'apply_promotion',side_effect=self.brain.BrainError('temporary promotion failure')):
            result=self.publish(reply,packet,auth)
        self.assertEqual('held',result['state']);revision=self.target()['revision']
        result=self.publish(reply,packet,auth)
        self.assertTrue(result['applied']);self.assertEqual(revision,self.target()['revision'])
        self.assertEqual(1,sum(m['status']=='active' for m,_ in self.bodies().values()))

    def test_private_linked_memory_is_not_exported(self):
        _,reply=self.stage()
        metadata,body=self.brain._parse_chunk(self.old_chunk)
        metadata['privacy']='private'
        sibling=self.old_chunk.with_name('private-linked.md')
        sibling.write_text(self.brain._render_chunk(metadata,body+'PRIVATE_REVIEW_SENTINEL'))
        with self.assertRaisesRegex(self.brain.BrainError,'memory-not-admitted'):self.export(reply)

    def test_binary_source_is_held_without_decode_traceback(self):
        self.encounter();self.source.write_bytes(b'\xff')
        plan=self.native.memory_review_plan(self.root,'cedar/b',owner='local')
        reply=self.native.memory_review_apply(self.root,'cedar/b',self.decision(plan),owner='local')
        with self.assertRaisesRegex(self.brain.BrainError,'source-unavailable'):self.export(reply)

    def test_malformed_bank_metadata_is_held_without_partial_packet(self):
        _,reply=self.stage();original=self.old_chunk.read_text()
        for change in ({'sources':None},{'id':None},{'privacy':[]}):
            metadata,body=self.brain._parse_chunk(self.old_chunk);metadata.update(change)
            self.old_chunk.write_text(self.brain._render_chunk(metadata,body))
            with self.subTest(change=change),self.assertRaisesRegex(self.brain.BrainError,'memory-unavailable'):
                self.export(reply)
            self.old_chunk.write_text(original)

    def test_completed_replay_uses_original_archived_task_uuid(self):
        _,reply=self.stage();packet=self.export(reply);auth=self.authorization(packet)
        self.publish(reply,packet,auth)
        task=self.brain.get_record(self.root,self.b['id'])
        self.brain.update_record(self.root,task['id'],expected_revision=task['revision'],progress=None,
            next_steps=[],files=[],sources=[],actor='local',transition_to='completed',reason='Fixture finished')
        self.brain.compact(self.root)
        other=self.brain.create_task(self.root,'cedar/b','A replacement task',[],[],owner='local')
        self.assertNotEqual(task['id'],other['id']);before=self.authority()
        result=self.publish(reply,packet,auth)
        self.assertTrue(result['result']['replayed']);self.assertEqual(before,self.authority())

    def test_packet_fields_cannot_be_substituted_even_with_new_packet_hash(self):
        _,reply=self.stage();packet=self.export(reply);before=self.authority()
        for key,value in [('task_uuid','bad'),('records',[None]),('sources',[None])]:
            changed=copy.deepcopy(packet);changed[key]=value
            changed['sha256']=self.native._semantic_digest({k:v for k,v in changed.items() if k!='sha256'})
            with self.subTest(key=key),self.assertRaises(self.brain.BrainError):self.publish(reply,changed,self.authorization(packet))
        self.assertEqual(before,self.authority())

    def test_external_controller_files_must_be_outside_project(self):
        internal=self.root/'authorization.json';internal.write_text('{}')
        with self.assertRaisesRegex(self.brain.BrainError,'external-input-required'):
            self.native.semantic_controller_input(self.root,str(internal))
        with tempfile.TemporaryDirectory() as folder:
            external=Path(folder)/'authorization.json';external.write_text('{"a":1}')
            self.assertEqual({'a':1},self.native.semantic_controller_input(self.root,str(external)))
            external.write_text('{"a":1,"a":2}')
            with self.assertRaisesRegex(self.brain.BrainError,'external-input-unavailable'):
                self.native.semantic_controller_input(self.root,str(external))

    def test_malformed_json_and_proposal_fields_have_bounded_refusals(self):
        for value in ({'bad':'\ud800'},{'bad':float('nan')},{'bad':{1}}):
            with self.subTest(value=repr(value)),self.assertRaisesRegex(self.brain.BrainError,'invalid-json-image'):
                self.native._semantic_digest(value)
        _,reply=self.stage()
        destination=self.native._semantic_path(self.root,'local','cedar/b',reply['proposal_id'])
        original=json.loads(destination.read_text())
        for key,value in [('records',None),('task_uuid',3)]:
            candidate=copy.deepcopy(original);candidate['plan'][key]=value
            candidate['sha256']=self.native._semantic_digest({k:v for k,v in candidate.items() if k!='sha256'})
            destination.write_text(json.dumps(candidate))
            with self.subTest(key=key),self.assertRaises(self.brain.BrainError):
                self.native.semantic_review_export(self.root,'cedar/b',reply['proposal_id'],candidate['sha256'],owner='local')


class NativeSemanticControllerTests(NativeCaptureFixture):
    """Installed CLI/state integration; verdict is a fixture, not a model grade."""
    def setUp(self):
        super().setUp()
        self.begin();self.stop();self.save();self.original=self.records('findings')[0]
        self.git('checkout','-qb','research-b')
        self.call('start','--task-id','research-b','--goal','Extend Amber retry policy','--json')
        self.env['CONTEXT_NATIVE_MEMORY_MAINTENANCE']='review-verified'
        self.call('native-prompt','--host','codex','--task-id','research-b','--json',data={
            'session_id':'session-b','prompt':'Extend Amber retry policy.'})
        found=json.loads(self.call('retrieve','Amber retry boundary','--task-id','research-b','--json').stdout)
        self.assertIn('proposal',found['memory_review'])
        with (self.root/'rules.md').open('a') as handle:handle.write('New helper added.\n')
        plan=json.loads(self.call('memory-review','--task-id','research-b','--json').stdout)
        pins=json.loads(self.call('source-fingerprints','--source','rules.md','--json').stdout)['source_fingerprints']
        self.proposal=json.loads(self.call('memory-review','--task-id','research-b','--input','-','--json',data={
            'plan_id':plan['plan_id'],'verified':True,'decisions':[{'record_id':self.original['id'],
            'action':'confirm','source_fingerprints':pins}]}).stdout)
        self.assertEqual(self.original,self.records('findings')[0])
        self.controller=('memory-review-controller','--task-id','research-b','--proposal-id',self.proposal['proposal_id'],
                         '--proposal-sha',self.proposal['proposal_sha256'])

    def close_primary(self):
        notice=self.call('native-stop','--host','codex','--task-id','research-b',data={
            'hook_event_name':'Stop','session_id':'session-b'})
        self.assertIn('use learnings=[]',notice.stdout.decode())
        self.request,self.native_state=next((f,json.loads(f.read_text())) for f in
            (self.root/'project-brain/local/native-results').glob('*.json') if json.loads(f.read_text())['task_id']=='research-b')
        self.call('native-result','--request',self.request.stem,'--token',self.native_state['token'],
            '--input','-','--json',data={'progress':'Checked Amber sources and staged the proposal.','learnings':[]})

    def test_malformed_fingerprint_container_is_a_bounded_cli_refusal(self):
        before=self.authority()
        for pins in (None,3,True,{},'unexpected'):
            result=self.call('memory-review','--task-id','research-b','--input','-','--json',
                data={'plan_id':self.proposal['proposal_id'],'verified':True,'decisions':[
                    {'record_id':self.original['id'],'action':'confirm','source_fingerprints':pins}]},ok=False)
            with self.subTest(pins=pins):
                self.assertNotEqual(0,result.returncode);self.assertNotIn(b'Traceback',result.stderr)
                self.assertEqual(before,self.authority())

    def controller_files(self):
        packet=json.loads(self.call(*self.controller,'--action','export','--json').stdout)
        with self.runtime() as (native,_,_):
            self.native=native;verdict=SemanticReviewTests.verdict(self,packet)
            authorization={'profile':native.SEMANTIC_AUTH,'packet_sha256':packet['sha256'],
                'primary_request':self.request.stem,'reviewer_session':'independent-review-session','verdict':verdict}
        temporary=tempfile.TemporaryDirectory(prefix='semantic-host-');self.addCleanup(temporary.cleanup)
        folder=Path(temporary.name);packet_file=folder/'packet.json';auth_file=folder/'authorization.json'
        packet_file.write_text(json.dumps(packet));auth_file.write_text(json.dumps(authorization))
        return packet_file,auth_file

    def test_installed_cli_publishes_only_after_primary_native_proof_and_replays(self):
        self.close_primary();packet,authorization=self.controller_files()
        result=json.loads(self.call(*self.controller,'--action','apply','--packet',str(packet),
                                   '--authorization',str(authorization),'--json').stdout)
        self.assertTrue(result['applied']);self.assertEqual(3,self.records('findings')[0]['revision'])
        self.assertEqual(1,len(self.records('findings')))
        self.request.unlink();(self.root/'rules.md').unlink();before=self.authority()
        result=json.loads(self.call(*self.controller,'--action','apply','--packet',str(packet),
                                   '--authorization',str(authorization),'--json').stdout)
        self.assertTrue(result['result']['replayed']);self.assertEqual(before,self.authority())

    def test_controller_rejects_project_owned_authorization_and_unfinished_primary(self):
        # Host can export before Stop, but publication requires its native proof.
        self.request=next(f for f in (self.root/'project-brain/local/native-results').glob('*.json')
                          if json.loads(f.read_text())['task_id']=='research-b')
        packet,authorization=self.controller_files();before=self.authority()
        held=self.call(*self.controller,'--action','apply','--packet',str(packet),
                       '--authorization',str(authorization),'--json',ok=False)
        self.assertNotEqual(0,held.returncode);self.assertIn(b'unfinished-or-same-session',held.stderr)
        internal=self.root/'review.json';internal.write_bytes(authorization.read_bytes())
        held=self.call(*self.controller,'--action','apply','--packet',str(packet),
                       '--authorization',str(internal),'--json',ok=False)
        self.assertNotEqual(0,held.returncode);self.assertIn(b'external-input-required',held.stderr)
        self.assertEqual(before,self.authority())


if __name__=='__main__':unittest.main()
