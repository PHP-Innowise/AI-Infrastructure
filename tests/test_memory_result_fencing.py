"""Fresh Opal result fences: actual native helper with owned temporary I/O."""
from __future__ import annotations
import copy, importlib, json, os
from pathlib import Path
import shutil, subprocess, sys, tempfile, unittest
from unittest import mock
ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / 'PHP Core/memory-bank/scripts'
PUBLIC = json.loads((ROOT/'tests/fixtures/memory_final_remediation_public.json').read_text(encoding='utf-8'))

class NativeResultFencingTests(unittest.TestCase):
    def setUp(self):
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.r = importlib.import_module('memory_results')
            self.b = importlib.import_module('brain_runtime')
        finally: sys.path.pop(0)
        folder = tempfile.TemporaryDirectory(prefix='opal-result-fence-')
        self.addCleanup(folder.cleanup)
        self.root = Path(folder.name)
        (self.root/'specs').mkdir(); (self.root/'specs/opal.md').write_text(PUBLIC['source'])
        (self.root/'memory-bank/chunks').mkdir(parents=True)
        config = self.root/'project-brain/config/runtime.json'; config.parent.mkdir(parents=True)
        config.write_text(json.dumps({'mode':'governed','automatic_promotion':False}))
        self.task = self.b.create_task(self.root,PUBLIC['task_alias'],PUBLIC['task_title'],[],[],owner='local')
        self.data = copy.deepcopy(PUBLIC['result'])
    def call(self, result_id=None, data=None, revision=1, **options):
        return self.r.record_result(self.root,self.task['id'],result_id or PUBLIC['result_id'],revision,
            self.data if data is None else data,owner='local',reason='Verified public Opal routes',**options)
    def task_now(self): return self.b.find_record(self.root,self.task['id'],record_type='task')[1]
    def marker(self, data=None):
        digest=self.r.request_digest(self.task['id'],self.r.normalize(self.root,self.data if data is None else data,check_files=False))
        return self.r._attempt(self.root,self.task['id'],PUBLIC['result_id'],digest)
    def learning_now(self): return self.b.find_record(self.root,self.r.learning_id(self.task['id'],self.data['learnings'][0]))[1]
    def images(self):
        return {str(item.relative_to(self.root)):item.read_bytes() for item in self.root.rglob('*')
                if item.is_file() and ('dynamic' in item.parts or 'indexes' in item.parts)}
    def late_edit(self):
        record = self.learning_now()
        self.b.update_record(self.root,record['id'],expected_revision=record['revision'],
            progress=PUBLIC['later_consequence'],next_steps=[],files=[],sources=[],actor='local',
            reason='Opal later canonical correction',reverify=True,attestation='agent')
        task=self.task_now()
        self.b.update_record(self.root,task['id'],expected_revision=task['revision'],
            progress=PUBLIC['later_progress'],next_steps=['Check the revised route'],files=[],sources=[],
            actor='local',reason='Opal later progress')
    def test_complete_marker_survives_missing_local_receipt_and_late_edits(self):
        self.call(); self.late_edit(); before=self.images()
        self.r._receipt(self.root,self.task['id'],PUBLIC['result_id']).unlink()
        reply=self.call(revision='auto')
        self.assertTrue(reply['replayed']); self.assertEqual('differs',reply['records'][0]['state'])
        self.assertEqual(before,self.images())
    def test_complete_marker_recovers_corrupt_local_receipt(self):
        self.call(); self.late_edit(); before=self.images()
        self.r._receipt(self.root,self.task['id'],PUBLIC['result_id']).write_text('{}')
        self.assertTrue(self.call()['replayed']); self.assertEqual(before,self.images())
    def test_changed_payload_under_a_reused_result_id_is_another_save(self):
        # A clone without this receipt - another developer's - reusing the
        # result ID for other content makes its own save, under its own proof,
        # and the first save still replays passively.
        self.call(); data=copy.deepcopy(self.data);data['progress']='Changed Opal request'
        self.r._receipt(self.root,self.task['id'],PUBLIC['result_id']).unlink()
        reply=self.call(data=data,revision='auto')
        self.assertFalse(reply['replayed']);self.assertEqual('Changed Opal request',self.task_now()['progress'])
        self.assertNotEqual(self.marker(),self.marker(data));self.assertTrue(self.marker(data).exists())
        before=self.images()
        self.assertTrue(self.call(revision='auto')['replayed']);self.assertEqual(before,self.images())
        # In the clone that holds the receipt, a third content is refused.
        data['progress']='Third Opal request'
        with self.assertRaisesRegex(self.b.BrainError,'different content'): self.call(data=data,revision='auto')
        self.assertEqual(before,self.images())
    def test_fence_precedes_first_effect_and_observed_created_stage_resumes(self):
        original=self.r.update_record
        def stop(*args,**kwargs):
            marker=self.r._read_attempt(self.marker())
            self.assertEqual('started',marker['state']);raise OSError('owned synthetic interruption')
        with mock.patch.object(self.r,'update_record',side_effect=stop):
            with self.assertRaises(OSError): self.call()
        record=self.learning_now();before_revision=record['revision']
        with mock.patch.object(self.r,'create_record',side_effect=AssertionError('Unexpected repeat write')):
            reply=self.call(revision='auto')
        self.assertEqual(record['id'],self.learning_now()['id']);self.assertEqual('resolved',self.learning_now()['status'])
        self.assertEqual(before_revision+1,self.learning_now()['revision']);self.assertFalse(reply['replayed'])
    def test_intent_write_failure_has_no_shared_record_effects(self):
        before=self.images(); original=self.r.atomic_json
        def fail(destination,value):
            if destination.parent.parent.name=='result-attempts':raise OSError('owned failure')
            return original(destination,value)
        with mock.patch.object(self.r,'atomic_json',side_effect=fail):
            with self.assertRaises(OSError): self.call()
        self.assertEqual(before,self.images())
    def test_complete_failure_resumes_observed_task_without_repeating_writes(self):
        original=self.r.atomic_json
        def fail(destination,value):
            if destination.parent.parent.name=='result-attempts' and value['state']=='complete':raise OSError('owned failure')
            return original(destination,value)
        with mock.patch.object(self.r,'atomic_json',side_effect=fail):
            with self.assertRaises(OSError): self.call()
        before=self.images()
        with mock.patch.object(self.r,'update_record',side_effect=AssertionError('Unexpected repeated effect')):
            self.call(revision='auto')
        self.assertEqual(before,self.images())
    def test_receipt_write_failure_still_has_complete_passive_proof(self):
        original=self.r.atomic_json
        def fail(destination,value):
            if destination.parent.name=='results':raise OSError('owned failure')
            return original(destination,value)
        with mock.patch.object(self.r,'atomic_json',side_effect=fail):
            with self.assertRaises(OSError): self.call()
        self.late_edit();before=self.images()
        self.assertTrue(self.call(revision='auto')['replayed']);self.assertEqual(before,self.images())
    def test_both_proofs_missing_cannot_adopt_later_learning(self):
        self.call();self.late_edit();learning=self.learning_now();path=self.b.find_record(self.root,learning['id'])[0];before=path.read_bytes()
        for destination in (self.marker(),self.r._receipt(self.root,self.task['id'],PUBLIC['result_id'])):destination.unlink()
        reply=self.call(revision=self.task_now()['revision'])
        self.assertEqual('differs',reply['records'][0]['state']);self.assertEqual(before,path.read_bytes())
        self.assertEqual(self.data['progress'],self.task_now()['progress'])
    def test_auto_fresh_request_binds_current_hook_updated_task(self):
        task=self.task_now();self.b.update_record(self.root,task['id'],expected_revision=task['revision'],
            progress=PUBLIC['later_progress'],next_steps=[],files=[],sources=[],actor='local',reason='Opal later task')
        revision=self.task_now()['revision'];reply=self.call(revision='auto')
        self.assertFalse(reply['replayed']);self.assertEqual(revision+1,self.task_now()['revision'])
        self.assertEqual(self.data['progress'],self.task_now()['progress'])
    def test_conflicting_duplicate_ids_refuse_before_first_effect(self):
        data=copy.deepcopy(self.data);data['learnings'].append({**data['learnings'][0],'consequence':PUBLIC['later_consequence']})
        before=self.images()
        with self.assertRaisesRegex(self.b.BrainError,'Conflicting learning'):self.call(data=data)
        self.assertEqual(before,self.images());self.assertFalse(self.marker().exists())
    def test_explicit_correction_uses_original_learning_cas_and_reverify(self):
        self.call();learning=self.learning_now();data=copy.deepcopy(self.data)
        data['learnings'][0].update(consequence=PUBLIC['later_consequence'],expected_revision=learning['revision'])
        reply=self.call(result_id='opal-correction',data=data,revision=self.task_now()['revision'])
        self.assertEqual('updated',reply['records'][0]['state']);self.assertEqual(PUBLIC['later_consequence'],self.learning_now()['progress'])
        self.assertEqual('agent',self.b.record_attestation(self.learning_now()))
    def test_stale_learning_cas_holds_only_that_learning(self):
        self.call();learning=self.learning_now();self.late_edit();data=copy.deepcopy(self.data)
        data['learnings'][0].update(consequence='New deliberate Opal correction',expected_revision=learning['revision'])
        data['learnings'].insert(0,{**self.data['learnings'][0],'title':'Another absent Opal finding'})
        path=self.b.find_record(self.root,learning['id'])[0];before=path.read_bytes()
        reply=self.call(result_id='opal-stale-correction',data=data,revision=self.task_now()['revision'])
        self.assertEqual(['created','differs'],[x['state'] for x in reply['records']])
        self.assertEqual('revision-conflict',reply['records'][1]['reason']);self.assertEqual(before,path.read_bytes())
        self.assertEqual(self.data['progress'],self.task_now()['progress'])
    def test_invalid_or_moved_marker_fail_closed(self):
        self.call();marker=self.marker();before=self.images()
        for raw in ('{}','{"profile":1,"profile":2}',marker.read_text().replace(self.task['id'],'00000000-0000-4000-8000-000000000000')):
            with self.subTest(raw_kind=len(raw)):
                marker.write_text(raw)
                with self.assertRaises(self.b.BrainError):self.call(revision='auto')
                self.assertEqual(before,self.images())
    def test_current_owner_authorization_still_applies_on_replay(self):
        self.call();before=self.images()
        with self.assertRaises(self.b.BrainError):
            self.r.record_result(self.root,self.task['id'],PUBLIC['result_id'],1,self.data,owner='stranger',reason='Public Opal check')
        self.assertEqual(before,self.images())
    def test_linked_attempt_directory_is_refused_before_mutation(self):
        control=self.root/'project-brain/control';control.mkdir(exist_ok=True)
        elsewhere=self.root/'unowned';elsewhere.mkdir()
        try:(control/'result-attempts').symlink_to(elsewhere,target_is_directory=True)
        except OSError:self.skipTest('Symlink unavailable')
        before=self.images()
        with self.assertRaisesRegex(self.b.BrainError,'attempt_unavailable'):self.call()
        self.assertEqual([],list(elsewhere.iterdir()));self.assertEqual(before,self.images())
    def test_explicit_agent_cas_cannot_repair_person_verified_open_learning(self):
        learning=self.data['learnings'][0]
        record=self.b.create_record(self.root,'finding',self.r.learning_id(self.task['id'],learning),
            learning['title'],[],learning['sources'],owner='local',authority='observed',goal=learning['consequence'])
        record=self.b.update_record(self.root,record['id'],expected_revision=record['revision'],
            progress=learning['consequence'],next_steps=[],files=[],sources=[],actor='local',
            reason='Verified Opal source by a person',authority='verified',attestation='person')
        self.assertEqual('open',record['status']);self.assertEqual('person',self.b.record_attestation(record))
        data=copy.deepcopy(self.data);data['learnings'][0]['expected_revision']=record['revision']
        path=self.b.find_record(self.root,record['id'])[0];before=path.read_bytes();reply=self.call(data=data)
        self.assertEqual('differs',reply['records'][0]['state']);self.assertEqual(before,path.read_bytes())
        self.assertEqual(self.data['progress'],self.task_now()['progress'])
    def test_person_repair_of_agent_verified_open_learning_reverifies_attestation(self):
        learning=self.data['learnings'][0]
        record=self.b.create_record(self.root,'finding',self.r.learning_id(self.task['id'],learning),
            learning['title'],[],learning['sources'],owner='local',authority='observed',goal=learning['consequence'])
        record=self.b.update_record(self.root,record['id'],expected_revision=record['revision'],
            progress=learning['consequence'],next_steps=[],files=[],sources=[],actor='local',
            reason='Agent initially verified Opal source',authority='verified',attestation='agent')
        data=copy.deepcopy(self.data);data['learnings'][0].update(expected_revision=record['revision'],consequence=PUBLIC['later_consequence'])
        self.call(data=data,attestation='person')
        current=self.learning_now();self.assertEqual('resolved',current['status']);self.assertEqual('verified',current['authority'])
        self.assertEqual(PUBLIC['later_consequence'],current['progress']);self.assertEqual('person',self.b.record_attestation(current))
    def test_agent_exact_cas_restatement_keeps_person_closed_record_unchanged(self):
        self.call(attestation='person');record=self.learning_now();data=copy.deepcopy(self.data)
        data['learnings'][0]['expected_revision']=record['revision'];before=self.images()
        reply=self.call(result_id='opal-readonly-restatement',data=data,revision=self.task_now()['revision'])
        self.assertEqual('existing',reply['records'][0]['state']);self.assertEqual(before,self.images())
        self.assertEqual('person',self.b.record_attestation(self.learning_now()))
    def test_present_corrupt_local_proof_without_marker_never_becomes_first_write(self):
        receipt=self.r._receipt(self.root,self.task['id'],PUBLIC['result_id']);receipt.parent.mkdir(parents=True)
        for raw in (b'not JSON',b'x'*4097):
            with self.subTest(length=len(raw)):
                receipt.write_bytes(raw);before=self.images()
                with self.assertRaisesRegex(self.b.BrainError,'receipt_unavailable'):self.call()
                self.assertEqual(before,self.images());self.assertFalse(self.marker().exists())
    def test_local_receipt_directory_failure_occurs_after_complete_shared_proof(self):
        expected=self.r._receipt(self.root,self.task['id'],PUBLIC['result_id']).parent
        original=Path.mkdir
        def fail(destination,*args,**kwargs):
            if destination==expected:
                marker=self.r._read_attempt(self.marker())
                self.assertEqual('complete',marker['state']);raise OSError('owned local cache directory failure')
            return original(destination,*args,**kwargs)
        with mock.patch.object(Path,'mkdir',new=fail):
            with self.assertRaises(OSError):self.call()
        self.late_edit();before=self.images()
        self.assertTrue(self.call(revision='auto')['replayed']);self.assertEqual(before,self.images())
    def test_fresh_cli_and_mcp_share_fence_and_optional_cas_schema(self):
        shutil.copytree(SCRIPTS,self.root/'memory-bank/scripts',ignore=shutil.ignore_patterns('__pycache__'))
        request=self.root/'request.json';request.write_text(json.dumps(self.data))
        run=subprocess.run([sys.executable,str(self.root/'memory-bank/scripts/context.py'),'--root',str(self.root),'record-result',
            '--task-id',self.task['id'],'--result-id',PUBLIC['result_id'],'--revision','1','--input',str(request),'--json'],
            env=dict(os.environ,GIT_OPTIONAL_LOCKS='0'),capture_output=True,text=True,check=False)
        self.assertEqual(0,run.returncode,run.stderr);self.assertFalse(json.loads(run.stdout)['replayed'])
        self.r._receipt(self.root,self.task['id'],PUBLIC['result_id']).unlink();self.late_edit();before=self.images()
        sys.path.insert(0,str(SCRIPTS))
        try:
            spec=importlib.util.spec_from_file_location('opal_installed_mcp',self.root/'memory-bank/scripts/mcp_server.py')
            mcp=importlib.util.module_from_spec(spec);spec.loader.exec_module(mcp)
        finally:sys.path.pop(0)
        memory=mcp.Memory(self.root)
        self.assertTrue(memory.call('memory_record_result',{'task_id':self.task['id'],'result_id':PUBLIC['result_id'],'revision':1,**self.data})['replayed'])
        schema=next(item for item in mcp.TOOLS if item['name']=='memory_record_result')
        self.assertIn('expected_revision',schema['inputSchema']['properties']['learnings']['items']['properties'])
        self.assertEqual(before,self.images())

if __name__=='__main__':unittest.main()
