"""Fresh Avocet optional-publication, native provenance and fd ownership controls."""
from __future__ import annotations
import copy
import hashlib
import importlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT=Path(__file__).resolve().parents[1]
PUBLIC=json.loads((ROOT/'tests/fixtures/memory_optional_publication_public.json').read_text(encoding='utf-8'))
sys.path.insert(0,str(ROOT/'harness/src'))
from harness.task_context import TaskContext
from harness.sessions import SessionError


class NativeOptionalPublicationTests(unittest.TestCase):
    def setUp(self):
        tmp=tempfile.TemporaryDirectory(prefix='avocet-publication-');self.addCleanup(tmp.cleanup);self.root=Path(tmp.name)
        self.env={k:v for k,v in os.environ.items() if not k.startswith(('GIT_','CONTEXT_','ACCELERATOR_'))}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',PYTHONIOENCODING='utf-8')
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config','project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        (self.root/'memory-bank/chunks').mkdir()
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps({'mode':'governed','automatic_promotion':False}),encoding='utf-8')
        (self.root/'.gitignore').write_text('memory-bank/local/\nproject-brain/local/\n',encoding='utf-8')
        for name,text in [(PUBLIC['source'],PUBLIC['source_text']),(PUBLIC['unrelated_source'],PUBLIC['unrelated_text'])]:
            leaf=self.root/name;leaf.parent.mkdir(exist_ok=True);leaf.write_text(text,encoding='utf-8')
        subprocess.run(['git','-C',str(self.root),'init','-q','-b',PUBLIC['task_id']],env=self.env,check=True,capture_output=True)
        self.cli=self.root/'memory-bank/scripts/context.py'
        task=self.call('start','--task-id',PUBLIC['task_id'],'--goal',PUBLIC['goal'],'--source',PUBLIC['source'],'--json')
        self.uuid=task['task_uuid']
        self.call('index','--json')
        sys.path.insert(0,str(self.cli.parent))
        runtime=('context','context_retrieval','brain_runtime','automatic_query','validate','memory_results','workspace_roots','context_handoff')
        try:
            for name in runtime:
                sys.modules.pop(name,None)
            self.context=importlib.import_module('context');self.retrieval=importlib.import_module('context_retrieval')
        finally:sys.path.pop(0)
        # These modules live in this test's temporary copy; a later test in the
        # same process must not import them from a deleted directory.
        self.addCleanup(lambda:[sys.modules.pop(name,None) for name in runtime])
        self.db=self.context.connect(self.root/'memory-bank/local/context.db');self.addCleanup(self.db.close)

    def call(self,*args):
        result=subprocess.run([sys.executable,'-B',str(self.cli),'--root',str(self.root),*args],env=self.env,cwd=self.root,capture_output=True,timeout=12)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'));return json.loads(result.stdout)
    def packet(self,*,ephemeral=True,sink=None):
        extra={} if sink is None else {'source_provenance_sink':sink}
        return self.context.assemble_capsule(self.db,self.root,mode='governed',query=PUBLIC['query'],task_id=PUBLIC['task_id'],limit=3,ephemeral=ephemeral,refresh_index=False,gate_mode='off',host='codex',entry_point='refresh',session_id='avocet-public-session',render=True,**extra)
    def receipts(self):
        return [tuple(row) for row in self.db.execute('SELECT key,value FROM index_state WHERE key IN (?,?) ORDER BY key',(self.retrieval.LAST_RETRIEVAL_KEY,self.retrieval.SESSION_DELIVERIES_KEY))]
    def assert_fact(self,packet):
        selected=[item['path'] for item in packet['selected']]
        self.assertEqual(PUBLIC['oracles']['selection'],selected)
        text='\n'.join(self.context.render_capsule_lines(packet,self.context.capsule_excerpts(self.db,packet)))
        self.assertIn(PUBLIC['fact'],text);self.assertLessEqual(len(self.context.serialize_capsule(packet)),8000)
    def failure(self,leaf,value):raise PermissionError('Avocet controlled publication refusal')

    def test_local_publication_failure_preserves_fact_without_receipt_pair(self):
        before=self.receipts()
        with mock.patch.object(self.retrieval,'atomic_json',side_effect=self.failure):
            packet=self.packet()
        self.assert_fact(packet);self.assertIsNone(packet['manifest']);self.assertEqual(before,self.receipts())

    def test_governed_publication_failure_precedes_receipt_commit(self):
        before=self.receipts()
        with mock.patch.object(self.retrieval,'atomic_json',side_effect=self.failure):
            with self.assertRaises(PermissionError):self.packet(ephemeral=False)
        self.assertEqual(before,self.receipts())

    def test_pruning_failure_keeps_confirmed_manifest_and_fact(self):
        with mock.patch.object(self.retrieval,'_prune_local_manifests',side_effect=OSError('Avocet prune refused')):
            packet=self.packet()
        self.assert_fact(packet);self.assertTrue((self.root/packet['manifest']).is_file());self.assertTrue(self.receipts())

    def test_unconfirmed_effect_after_write_does_not_adopt_or_claim_receipt(self):
        write=self.retrieval.atomic_json;written=[]
        def partial(leaf,value):
            write(leaf,value);written.append(Path(leaf));raise OSError('Avocet return lost after write')
        before=self.receipts();sink={}
        with mock.patch.object(self.retrieval,'atomic_json',side_effect=partial):packet=self.packet(sink=sink)
        self.assert_fact(packet);self.assertIsNone(packet['manifest']);self.assertEqual(before,self.receipts())
        self.assertEqual(1,len(written));self.assertTrue(written[0].is_file());self.assertEqual('unconfirmed',sink['publication'])

    def test_cancellation_propagates_before_new_receipt_claims(self):
        before=self.receipts()
        with mock.patch.object(self.retrieval,'atomic_json',side_effect=KeyboardInterrupt('Avocet cancel')):
            with self.assertRaisesRegex(KeyboardInterrupt,'Avocet cancel'):self.packet()
        self.assertEqual(before,self.receipts())

    def test_inline_transport_copies_native_full_hash_without_disk_rehash(self):
        sink={}
        with mock.patch.object(self.retrieval,'atomic_json',side_effect=self.failure):packet=self.packet(sink=sink)
        self.assert_fact(packet);self.assertEqual([{'path':PUBLIC['source'],'source_hash':PUBLIC['digest']}],sink['selected'])
        self.assertEqual(PUBLIC['transport_profile'],sink['profile']);self.assertEqual(self.uuid,sink['task_uuid'])
        digest=hashlib.sha256(json.dumps(packet,ensure_ascii=False,separators=(',',':'),sort_keys=True,allow_nan=False).encode('utf-8')).hexdigest()
        self.assertEqual(digest,sink['capsule_sha256']);self.assertNotIn('source_provenance',packet)
        with mock.patch('harness.task_context.read_context',side_effect=AssertionError('no later read/rehash')):
            hashes=TaskContext._source_hashes(self.root,packet,provenance=sink,allow_local=True)
        self.assertEqual({PUBLIC['source']:PUBLIC['digest']},hashes)

    def test_local_transport_flag_is_explicit_and_model_capsule_is_unchanged(self):
        suffix="\n_original_avocet_atomic=atomic_json\ndef atomic_json(leaf,value):\n    if 'retrieval-manifests' in Path(leaf).parts: raise PermissionError('Avocet controlled helper I/O')\n    return _original_avocet_atomic(leaf,value)\n"
        leaf=self.cli.parent/'context_retrieval.py';leaf.write_text(leaf.read_text(encoding='utf-8')+suffix,encoding='utf-8')
        response=self.call('retrieve',PUBLIC['query'],'--task-id',PUBLIC['task_id'],'--ephemeral','--gate','off','--provenance-response','--json')
        self.assertEqual(PUBLIC['transport_profile'],response['response_profile']);self.assertIsNone(response['capsule']['manifest'])
        self.assertEqual(PUBLIC['digest'],response['source_provenance']['selected'][0]['source_hash'])
        self.assertLessEqual(len(json.dumps(response['capsule'],ensure_ascii=False,separators=(',',':'))),8000)
        ordinary=self.call('retrieve',PUBLIC['query'],'--task-id',PUBLIC['task_id'],'--ephemeral','--gate','off','--json')
        self.assertNotIn('response_profile',ordinary);self.assertIsNone(ordinary['manifest'])

    def test_deleted_source_failure_never_gains_inline_admission(self):
        (self.root/PUBLIC['source']).unlink();self.call('index','--json');sink={}
        with mock.patch.object(self.retrieval,'atomic_json',side_effect=self.failure):packet=self.packet(sink=sink)
        self.assertNotIn(PUBLIC['source'],{row['path'] for row in sink['selected']})
        self.assertNotIn(PUBLIC['fact'],json.dumps(packet));self.assertEqual([],sink['selected'])

    def test_actual_private_record_is_absent_from_unconfirmed_public_transport(self):
        title='Avocet private buoy details'
        goal='Avocet estuary secret counters carry eleven jade alarms.'
        created=self.call('brain-create','finding','--external-id','finding/avocet-private-buoy',
                          '--title',title,'--goal',goal,'--privacy','private',
                          '--source',PUBLIC['source'],'--json')
        self.call('index','--json');sink={}
        with mock.patch.object(self.retrieval,'atomic_json',side_effect=self.failure):packet=self.packet(sink=sink)
        self.assert_fact(packet)
        encoded=json.dumps({'capsule':packet,'source_provenance':sink},ensure_ascii=False)
        self.assertNotIn(title,encoded);self.assertNotIn(goal,encoded)
        self.assertNotIn(created['id'],encoded)
        self.assertEqual([{'path':PUBLIC['source'],'source_hash':PUBLIC['digest']}],sink['selected'])

    def test_reused_sink_is_cleared_on_current_warning_and_failed_lookup(self):
        sink={}
        with mock.patch.object(self.retrieval,'atomic_json',side_effect=self.failure):self.packet(sink=sink)
        self.assertTrue(sink)
        packet=self.context.assemble_capsule(self.db,self.root,mode='governed',query=PUBLIC['query'],
            task_id=PUBLIC['task_id'],limit=3,ephemeral=True,refresh_index=False,
            index_warnings=['Avocet current index admission refused'],source_provenance_sink=sink)
        self.assertEqual({},sink);self.assertEqual([],packet['selected'])
        self.assertTrue(packet['working']);self.assertTrue(packet['warnings'])
        sink.update({'Avocet':'not current'})
        with self.assertRaises(self.context.ContextError):
            self.context.assemble_capsule(self.db,self.root,mode='governed',query=PUBLIC['query'],
                task_id='feature/avocet-unknown',limit=3,ephemeral=True,refresh_index=False,
                source_provenance_sink=sink)
        self.assertEqual({},sink)

    def harness(self,review=False):
        from harness import sessions
        from harness.knowledge import KnowledgeManager
        state=tempfile.TemporaryDirectory(prefix='avocet-harness-state-');self.addCleanup(state.cleanup)
        for owner,name,value in [(sessions.providers,'discover_providers',[{'id':'codex','available':True,'name':'Avocet no models','executable':'unused'}]),(sessions.Sessions,'_worker',None),(sessions.Sessions,'_keep_accelerators_current',None)]:
            patcher=mock.patch.object(owner,name,return_value=value);patcher.start();self.addCleanup(patcher.stop)
        forbid=mock.patch.object(sessions.providers,'build_command',side_effect=AssertionError('no model CLI'));forbid.start();self.addCleanup(forbid.stop)
        for rel in ('memory-bank/README.md','memory-bank/INDEX.md','project-brain/README.md','project-brain/PROTOCOL.md'):
            shutil.copy2(ROOT/'PHP Core'/rel,self.root/rel)
        store=sessions.Sessions(Path(state.name)/'sessions',[self.root]);self.addCleanup(store.close)
        manager=KnowledgeManager(store);self.addCleanup(manager.close);store.knowledge=manager
        project_id=next(iter(store.projects))
        session=store.create({'project_id':project_id,'provider':'codex','mode':'plan','prompt':PUBLIC['query'],'brain':{'bank':'memory-bank','task_id':PUBLIC['task_id'],'record_id':self.uuid,'query':PUBLIC['query'],'create':False,'review':review}})
        return store,manager,session

    def install_publication_fault(self):
        leaf=self.cli.parent/'context_retrieval.py'
        suffix="\n_original_avocet_atomic=atomic_json\ndef atomic_json(leaf,value):\n    if 'retrieval-manifests' in Path(leaf).parts: raise PermissionError('Avocet controlled helper I/O')\n    return _original_avocet_atomic(leaf,value)\n"
        leaf.write_text(leaf.read_text(encoding='utf-8')+suffix,encoding='utf-8')

    @unittest.skipUnless(hasattr(os,'O_NOFOLLOW') and hasattr(os,'O_NONBLOCK'),
                         'secure helper provenance marker profile required')
    def test_actual_harness_unattended_prepare_accepts_same_invocation_transport(self):
        store,manager,session=self.harness();self.install_publication_fault();context=TaskContext(manager)
        prepared=context.prepare(session)
        self.assertIsNone(prepared['capsule']['manifest']);self.assertIn(PUBLIC['fact'],prepared['capsule_text'])
        self.assertEqual({PUBLIC['source']:PUBLIC['digest']},prepared['source_hashes'])
        self.assertNotIn('source_provenance',prepared['capsule'])

    @unittest.skipUnless(hasattr(os,'O_NOFOLLOW') and hasattr(os,'O_NONBLOCK'),
                         'secure helper provenance marker profile required')
    def test_actual_harness_reviewed_freshness_accepts_local_proof_without_reapproval(self):
        store,manager,session=self.harness(review=True);context=TaskContext(manager)
        prepared=context.prepare(session);prepared['approved']=True;store._save_brain(session['id'],prepared)
        self.install_publication_fault();fresh=context.ensure_fresh(store.get(session['id']))
        self.assertEqual({PUBLIC['source']:PUBLIC['digest']},prepared['source_hashes'])
        self.assertTrue(fresh)

    def test_actual_harness_initial_governed_review_still_refuses_publication_failure(self):
        store,manager,session=self.harness(review=True);self.install_publication_fault();context=TaskContext(manager)
        with self.assertRaises(SessionError):context.prepare(session)

    def test_old_interface_marker_keeps_legacy_raw_retrieval_contract(self):
        store,manager,session=self.harness();info=manager.info(session['project_id'],'memory-bank',_root=self.root)
        self.assertEqual(hasattr(os,'O_NOFOLLOW') and hasattr(os,'O_NONBLOCK'),
                         manager.supports_provenance_response(session['project_id'],info,_root=self.root))
        marker='SOURCE_PROVENANCE_PROFILE = "local-source-provenance-v1"'
        self.cli.write_text(self.cli.read_text(encoding='utf-8').replace(marker,'# Avocet old interface marker absent'),encoding='utf-8')
        self.assertFalse(manager.supports_provenance_response(session['project_id'],info,_root=self.root))
        prepared=TaskContext(manager).prepare(session);self.assertIsInstance(prepared['capsule']['manifest'],str)
        self.assertEqual({PUBLIC['source']:PUBLIC['digest']},prepared['source_hashes'])


class InlineSourceAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.cap={'task_uuid':'da883ac6-6ea4-4280-9873-3141a389c2fa','task_revision':7,'manifest':None,'manifest_scope':'local','semantic':[{'path':PUBLIC['source'],'snippet':PUBLIC['fact']}],'procedural':[],'episodic':[],'selected':[{'path':PUBLIC['source']}]}
        self.proof={'schema_version':1,'profile':PUBLIC['transport_profile'],'scope':'local','publication':'unconfirmed','task_uuid':self.cap['task_uuid'],'task_revision':7,'selected':[{'path':PUBLIC['source'],'source_hash':PUBLIC['digest']}],'capsule_sha256':self.digest(self.cap)}
    def digest(self,cap):return hashlib.sha256(json.dumps(cap,ensure_ascii=False,separators=(',',':'),sort_keys=True,allow_nan=False).encode()).hexdigest()
    def accept(self,cap=None,proof=None,allow=True):return TaskContext._source_hashes(Path('/unused-avocet'),cap or self.cap,provenance=self.proof if proof is None else proof,allow_local=allow)

    def test_complete_explicit_bound_proof_and_legitimate_empty_coverage(self):
        self.assertEqual({PUBLIC['source']:PUBLIC['digest']},self.accept())
        cap=dict(self.cap,semantic=[],selected=[]);proof=dict(self.proof,selected=[],capsule_sha256=self.digest(cap));self.assertEqual({},self.accept(cap,proof))
        with self.assertRaises(SessionError):self.accept(cap,{})

    def test_wrong_profile_scope_task_revision_frame_and_permission_refused(self):
        for changed in ({'profile':'foreign'},{'scope':'governed'},{'task_uuid':'foreign'},{'task_revision':True},{'capsule_sha256':'0'*64},{'extra':0}):
            with self.subTest(changed=changed):
                with self.assertRaises(SessionError):self.accept(proof=dict(self.proof,**changed))
        with self.assertRaises(SessionError):self.accept(allow=False)
        cap=dict(self.cap,semantic=[{'path':PUBLIC['source'],'snippet':'Altered Avocet frame'}])
        with self.assertRaises(SessionError):self.accept(cap)

    def test_incomplete_duplicate_extra_and_unsafe_selected_rows_refused(self):
        for rows in ([],self.proof['selected']*2,[{'path':PUBLIC['source'],'source_hash':'bad'}],[{'path':'../foreign','source_hash':PUBLIC['digest']}],self.proof['selected']+[{'path':PUBLIC['unrelated_source'],'source_hash':'b'*64}]):
            with self.subTest(rows=rows):
                with self.assertRaises(SessionError):self.accept(proof=dict(self.proof,selected=rows))

    def test_present_missing_or_corrupt_manifest_never_downgrades_to_inline(self):
        cap=dict(self.cap,manifest='memory-bank/local/retrieval-manifests/da883ac6-6ea4-4280-9873-3141a389c2fa.json')
        with mock.patch('harness.task_context.read_context',return_value=None):
            with self.assertRaises(SessionError):self.accept(cap)
        with mock.patch('harness.task_context.read_context',return_value=(8,'not JSON')):
            with self.assertRaises(SessionError):self.accept(cap)

    def test_proof_transport_bound_and_finite_scalars_are_enforced(self):
        with self.assertRaises(SessionError):self.accept(proof=dict(self.proof,selected=self.proof['selected']*33))
        with self.assertRaises(SessionError):self.accept(proof=dict(self.proof,capsule_sha256='x'*65537))
        cap=dict(self.cap,task_revision=float('nan'))
        with self.assertRaises(SessionError):self.accept(cap)

    def test_capsule_invalid_utf8_and_unserializable_values_refuse_as_session_error(self):
        task={'id':self.cap['task_uuid'],'external_id':PUBLIC['task_id'],'revision':7}
        cap=dict(self.cap,task_id=PUBLIC['task_id'],working={'task_id':PUBLIC['task_id']})
        for value in ('\ud800',{'avocet-set'},float('nan')):
            with self.subTest(value_type=type(value).__name__):
                with self.assertRaises(SessionError):TaskContext._validate_capsule(dict(cap,extra=value),task)


@unittest.skipUnless(os.name=='posix' and hasattr(os,'O_DIRECTORY'),'descriptor-relative native trace profile required')
class TraceOwnershipTests(unittest.TestCase):
    def setUp(self):
        sys.path.insert(0,str(ROOT/'PHP Core/memory-bank/scripts'))
        try:
            sys.modules.pop('context_retrieval',None);self.r=importlib.import_module('context_retrieval')
        finally:sys.path.pop(0)
        tmp=tempfile.TemporaryDirectory(prefix='avocet-trace-');self.addCleanup(tmp.cleanup);self.root=Path(tmp.name)
        (self.root/'memory-bank/local/retrieval-traces').mkdir(parents=True)
        self.identifier='f6911b38-7534-46c2-ac92-ce9712b7e188';self.close=os.close;self.open=os.open
    def alive(self,fd):
        try:os.fstat(fd);return True
        except OSError:return False

    def test_save_trace_close_error_cannot_replace_optional_body_failure(self):
        fd=self.open(self.root,os.O_RDONLY|os.O_DIRECTORY);calls=[]
        def close(number):calls.append(number);self.close(number);raise OSError('Avocet close after effect')
        try:
            with mock.patch.object(self.r,'_trace_directory',return_value=fd),mock.patch.object(self.r,'_write_trace',side_effect=PermissionError('Avocet optional body')),mock.patch.object(self.r.os,'close',side_effect=close):
                self.r._save_trace(self.root,self.identifier,self.r.RetrievalTrace({}),{})
            self.assertEqual([fd],calls);self.assertFalse(self.alive(fd))
        finally:
            if self.alive(fd):self.close(fd)

    def test_directory_registers_child_before_parent_close_and_never_retries(self):
        opened=[];calls=[]
        def opened_fd(*args,**kwargs):
            fd=self.open(*args,**kwargs);opened.append(fd);return fd
        def close(number):
            calls.append(number);self.close(number)
            if len(calls)==1:raise OSError('Avocet parent close after effect')
        try:
            with mock.patch.object(self.r.os,'open',side_effect=opened_fd),mock.patch.object(self.r.os,'close',side_effect=close):
                result=self.r._trace_directory(self.root)
            self.assertIsNone(result);self.assertEqual(opened,calls);self.assertTrue(all(not self.alive(fd) for fd in opened))
        finally:
            for fd in set(opened):
                if self.alive(fd):self.close(fd)

    def test_render_refusal_drains_file_and_directory_after_close_error(self):
        directory=self.open(self.root/'memory-bank/local/retrieval-traces',os.O_RDONLY|os.O_DIRECTORY)
        leaf=self.root/'memory-bank/local/retrieval-traces'/f'{self.identifier}.json';leaf.write_text('{}',encoding='utf-8');alias=leaf.with_suffix('.alias');os.link(leaf,alias)
        calls=[]
        def close(number):
            calls.append(number);self.close(number)
            if number!=directory:raise OSError('Avocet file close after effect')
        try:
            with mock.patch.object(self.r,'_trace_directory',return_value=directory),mock.patch.object(self.r.os,'close',side_effect=close):
                self.r.record_render_trace(self.root,{'manifest':f'memory-bank/local/retrieval-manifests/{self.identifier}.json'}, {})
            self.assertEqual(2,len(calls));self.assertEqual(directory,calls[-1]);self.assertTrue(all(not self.alive(fd) for fd in calls))
        finally:
            for fd in set(calls+[directory]):
                if self.alive(fd):self.close(fd)

    def test_write_trace_preserves_primary_and_attempts_temp_cleanup(self):
        directory=self.open(self.root/'memory-bank/local/retrieval-traces',os.O_RDONLY|os.O_DIRECTORY);calls=[]
        def close(number):calls.append(number);self.close(number);raise OSError('Avocet close after effect')
        try:
            with mock.patch.object(self.r.os,'fdopen',side_effect=RuntimeError('Avocet primary fdopen')),mock.patch.object(self.r.os,'close',side_effect=close):
                with self.assertRaisesRegex(RuntimeError,'Avocet primary'):self.r._write_trace(directory,self.identifier,{})
            self.assertEqual(1,len(calls));self.assertFalse((self.root/'memory-bank/local/retrieval-traces'/('.'+self.identifier+'.tmp')).exists())
        finally:self.close(directory)

    def test_close_cancellation_drains_other_owned_fds_once(self):
        first=self.open(self.root,os.O_RDONLY|os.O_DIRECTORY);second=self.open(self.root,os.O_RDONLY|os.O_DIRECTORY);calls=[]
        def close(number):
            calls.append(number);self.close(number)
            if number==first:raise KeyboardInterrupt('Avocet close cancellation')
        try:
            with mock.patch.object(self.r.os,'close',side_effect=close):
                with self.assertRaisesRegex(KeyboardInterrupt,'Avocet close cancellation'):self.r._close_trace_descriptors(first,second)
            self.assertEqual([first,second],calls);self.assertFalse(self.alive(first));self.assertFalse(self.alive(second))
        finally:
            for fd in (first,second):
                if self.alive(fd):self.close(fd)

    def test_cleanup_cancellation_preserves_active_primary(self):
        first=self.open(self.root,os.O_RDONLY|os.O_DIRECTORY);second=self.open(self.root,os.O_RDONLY|os.O_DIRECTORY);calls=[]
        def close(number):
            calls.append(number);self.close(number)
            if number==first:raise KeyboardInterrupt('Avocet secondary cancellation')
        def operation():
            try:raise RuntimeError('Avocet active primary')
            finally:self.r._close_trace_descriptors(first,second)
        try:
            with mock.patch.object(self.r.os,'close',side_effect=close):
                with self.assertRaisesRegex(RuntimeError,'Avocet active primary'):operation()
            self.assertEqual([first,second],calls)
        finally:
            for fd in (first,second):
                if self.alive(fd):self.close(fd)

    def cancelling_file_proxy(self, operation, message, closed):
        fdopen=os.fdopen
        class FileProxy:
            def __init__(self,handle):self.handle=handle
            def __getattr__(self,name):return getattr(self.handle,name)
            def __enter__(self):return self
            def __exit__(self,*args):self.close()
            def write(self,*args):
                if operation=='write':raise KeyboardInterrupt(message)
                return self.handle.write(*args)
            def read(self,*args):
                if operation=='read':raise KeyboardInterrupt(message)
                return self.handle.read(*args)
            def close(self):
                closed.append(self.handle.fileno());self.handle.close()
                raise OSError('Avocet file-object close after effect')
        return lambda *args,**kwargs:FileProxy(fdopen(*args,**kwargs))

    def test_write_file_close_error_preserves_body_cancellation(self):
        directory=self.open(self.root/'memory-bank/local/retrieval-traces',os.O_RDONLY|os.O_DIRECTORY)
        closed=[];message='Avocet write body cancellation'
        try:
            with mock.patch.object(self.r.os,'fdopen',side_effect=self.cancelling_file_proxy('write',message,closed)):
                with self.assertRaisesRegex(KeyboardInterrupt,message):self.r._write_trace(directory,self.identifier,{})
            self.assertEqual(1,len(closed));self.assertFalse(self.alive(closed[0]))
            self.assertFalse((self.root/'memory-bank/local/retrieval-traces'/('.'+self.identifier+'.tmp')).exists())
            self.assertTrue(self.alive(directory))
        finally:self.close(directory)

    def test_render_file_close_error_preserves_body_cancellation(self):
        directory=self.open(self.root/'memory-bank/local/retrieval-traces',os.O_RDONLY|os.O_DIRECTORY)
        leaf=self.root/'memory-bank/local/retrieval-traces'/f'{self.identifier}.json'
        original=b'{"records":[]}';leaf.write_bytes(original);closed=[];message='Avocet render body cancellation'
        try:
            with mock.patch.object(self.r,'_trace_directory',return_value=directory),mock.patch.object(self.r.os,'fdopen',side_effect=self.cancelling_file_proxy('read',message,closed)):
                with self.assertRaisesRegex(KeyboardInterrupt,message):
                    self.r.record_render_trace(self.root,{'manifest':f'memory-bank/local/retrieval-manifests/{self.identifier}.json'}, {})
            self.assertEqual(1,len(closed));self.assertFalse(self.alive(closed[0]));self.assertFalse(self.alive(directory))
            self.assertEqual(original,leaf.read_bytes())
        finally:
            if self.alive(directory):self.close(directory)


if __name__=='__main__':unittest.main()
