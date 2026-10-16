"""Fresh Gannet native-source diagnostics, strict inventory and publication cuts."""
from __future__ import annotations

import copy
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

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT/'tests/fixtures/memory_source_resilience_public.json').read_text(encoding='utf-8'))


class MemorySourceResilienceTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='gannet-source-')
        self.addCleanup(temporary.cleanup);self.root=Path(temporary.name)
        self.env={k:v for k,v in os.environ.items() if not k.startswith(('GIT_','CONTEXT_','ACCELERATOR_'))}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',PYTHONIOENCODING='utf-8')
        for relative in ('memory-bank/scripts','memory-bank/templates','project-brain/config','project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/relative,self.root/relative,
                ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        (self.root/'memory-bank/chunks').mkdir()
        (self.root/'.gitignore').write_text('memory-bank/local/\nproject-brain/local/\n',encoding='utf-8')
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps(
            {'mode':'governed','automatic_promotion':False}),encoding='utf-8')
        for name,text in PUBLIC['sources'].items():
            leaf=self.root/name;leaf.parent.mkdir(exist_ok=True);leaf.write_text(text,encoding='utf-8')
        subprocess.run(['git','-C',str(self.root),'init','-q','-b','main'],env=self.env,check=True,capture_output=True)
        self.cli=self.root/'memory-bank/scripts/context.py';self.task=PUBLIC['task_id']
        self.call('start','--task-id',self.task,'--goal',PUBLIC['goal'],'--source','specs/gannet-cache.md','--json')
        reply=self.refresh();self.uuid=reply['capsule']['task_uuid']
        self.assertIn(PUBLIC['fact'],reply['capsule_text'])
        self.handoff=self.root/f'project-brain/control/handoffs/{self.uuid}.md'
        self.database=self.root/'memory-bank/local/context.db'
        # The runtime imported here comes from this temporary copy and leaves
        # sys.modules with it: a later module in the same process (the Windows
        # job runs them together) must not import files that are gone.
        names=[leaf.stem for leaf in self.cli.parent.glob('*.py')]
        previous={name:sys.modules.pop(name) for name in names if name in sys.modules}
        def restore():
            for name in names:sys.modules.pop(name,None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        scripts=str(self.cli.parent);sys.path.insert(0,scripts)
        try:
            self.context=importlib.import_module('context');self.brain=importlib.import_module('brain_runtime')
            self.retrieval=importlib.import_module('context_retrieval');self.results=importlib.import_module('memory_results')
            self.mcp=importlib.import_module('mcp_server')
        finally:sys.path.remove(scripts)
        self.connection=self.context.connect(self.database);self.addCleanup(self.connection.close)

    def process(self,*args):
        return subprocess.run([sys.executable,'-B',str(self.cli),'--root',str(self.root),*args],
            env=self.env,cwd=self.root,capture_output=True,timeout=20)

    def call(self,*args,expected=0):
        result=self.process(*args)
        self.assertEqual(expected,result.returncode,result.stderr.decode('utf-8','replace'))
        self.assertNotIn(b'Traceback',result.stderr)
        self.assertTrue(result.stdout.strip(),result.stderr.decode('utf-8','replace'))
        return json.loads(result.stdout)

    def refresh(self,*,expected=0):
        return self.call('refresh','--query',PUBLIC['query'],'--task-id',self.task,
            '--host','codex','--sanitize','--ephemeral','--json',expected=expected)

    def snapshot(self):
        return {table:[tuple(row) for row in self.connection.execute('SELECT * FROM '+table+' ORDER BY rowid')]
                for table in ('documents','document_metadata','document_links','document_source_state','index_state')}

    def bad_dynamic(self,content=None):
        leaf=self.root/'project-brain/dynamic/findings/gannet-corrupt.md';leaf.parent.mkdir(exist_ok=True)
        raw=bytes.fromhex(PUBLIC['invalid_utf8_hex']) if content is None else content
        leaf.write_bytes(raw);return leaf,raw

    def test_invalid_derived_handoff_isolated_and_atomically_evicted(self):
        relative=self.handoff.relative_to(self.root).as_posix()
        for table in ('documents','document_metadata','document_links'):
            self.assertGreater(self.connection.execute('SELECT COUNT(*) FROM '+table+' WHERE path=?',(relative,)).fetchone()[0],0)
        raw=bytes.fromhex(PUBLIC['invalid_utf8_hex']);self.handoff.write_bytes(raw)
        indexed=self.context.index_repository(self.connection,self.root,incremental=True)
        reply=self.refresh()
        self.assertIn(PUBLIC['fact'],reply['capsule_text'])
        self.assertIn({'path':relative,'reason':'invalid-utf8'},indexed['excluded'])
        for table in ('documents','document_metadata','document_links','document_source_state'):
            self.assertEqual(0,self.connection.execute('SELECT COUNT(*) FROM '+table+' WHERE path=?',(relative,)).fetchone()[0])
        self.assertEqual(raw,self.handoff.read_bytes())
        self.assertTrue(all(reply[layer]=='updated' for layer in self.context.DOCUMENT_LAYERS))

    def test_corrupt_dynamic_record_is_isolated_and_the_rest_delivered(self):
        # One undecodable record is left out, as an invalid record always was;
        # it no longer costs every other record its memory.
        leaf,raw=self.bad_dynamic()
        reply=self.refresh()
        self.assertTrue(all(reply[layer]=='updated' for layer in self.context.DOCUMENT_LAYERS))
        self.assertIn(PUBLIC['fact'],reply['capsule_text'])
        relative=leaf.relative_to(self.root).as_posix()
        self.assertEqual(0,self.connection.execute('SELECT COUNT(*) FROM documents WHERE path=?',(relative,)).fetchone()[0])
        self.assertNotIn('UnicodeDecodeError',json.dumps(reply));self.assertEqual(raw,leaf.read_bytes())
        problems=[];list(self.brain.iter_records(self.root,problems=problems))
        self.assertEqual([leaf],[problem['path'] for problem in problems])

    def test_escaped_invalid_native_title_is_incomplete_without_publication(self):
        leaf,metadata,body=self.brain.find_task(self.root,self.task)
        before=self.snapshot();original=leaf.read_bytes()
        profile=PUBLIC['unicode_metadata']
        for literal in profile['bad_json_string_literals']:
            with self.subTest(literal=literal):
                changed=dict(metadata,title=json.loads(literal))
                raw=('---\n'+json.dumps(changed,ensure_ascii=True,indent=2,sort_keys=True)+'\n---\n\n'+body).encode('utf-8')
                leaf.write_bytes(raw)
                # The layers refresh and the turn keeps the rest of its
                # memory; the task whose own record is invalid has no working
                # state in it, and the warning names the file and cause.
                reply=self.refresh()
                capsule=reply['capsule']
                self.assertIsNone(capsule['working']);self.assertTrue(capsule['task_record_unreadable'])
                self.assertIn(PUBLIC['fact'],reply['capsule_text'])
                self.assertIn("working: unavailable (the task's record cannot be read)",reply['capsule_text'])
                self.assertNotIn('not recorded yet',reply['capsule_text'])
                self.assertNotIn('Gannet \ufffd title',reply['capsule_text'])
                self.assertIn(leaf.name,' '.join(reply['warnings']))
                self.assertIn(profile['expected']['fixed_diagnostic'],' '.join(reply['warnings']))
                self.assertNotIn('not found',' '.join(reply['warnings']))
                self.assertEqual(raw,leaf.read_bytes())
        leaf.write_bytes(original)

    def test_native_validator_rejects_surrogates_and_preserves_valid_unicode(self):
        _,metadata,_=self.brain.find_task(self.root,self.task)
        profile=PUBLIC['unicode_metadata']
        for literal in profile['bad_json_string_literals']:
            with self.subTest(literal=literal):
                with self.assertRaisesRegex(self.brain.BrainError,profile['expected']['fixed_diagnostic']):
                    self.brain.validate_record(dict(metadata,title=json.loads(literal)))
        valid=json.loads(profile['valid_json_string_literal'])
        self.assertEqual(profile['expected']['valid_utf8_hex'],valid.encode('utf-8').hex())
        self.brain.validate_record(dict(metadata,title=valid))

    def test_malformed_dynamic_record_is_isolated_like_an_undecodable_one(self):
        leaf,raw=self.bad_dynamic(PUBLIC['malformed_literal'].encode('utf-8'))
        reply=self.refresh()
        self.assertIn(PUBLIC['fact'],reply['capsule_text']);self.assertEqual(raw,leaf.read_bytes())
        problems=[];list(self.brain.iter_records(self.root,problems=problems))
        self.assertEqual([leaf],[problem['path'] for problem in problems])

    def test_native_invalid_utf8_is_typed_and_validate_names_its_source(self):
        leaf,raw=self.bad_dynamic()
        with self.assertRaisesRegex(self.brain.BrainError,'gannet-corrupt.md.*invalid UTF-8'):
            self.brain.parse_markdown_record(leaf)
        reply=self.process('validate','--json')
        self.assertEqual(1,reply.returncode);self.assertNotIn(b'Traceback',reply.stderr)
        # validate reports every bad record by name in its list of errors
        # instead of stopping at the first one.
        errors=json.loads(reply.stdout)['errors']
        self.assertTrue(any(leaf.name in error and 'invalid UTF-8' in error for error in errors),errors)
        self.assertNotIn(b'UnicodeDecodeError',reply.stdout+reply.stderr)
        self.assertEqual(raw,leaf.read_bytes())

    def test_late_unreadable_subtree_prevents_first_yield_and_publication(self):
        blocked=self.root/'project-brain/dynamic'/PUBLIC['late_subtree'];blocked.mkdir()
        before=self.snapshot();actual=os.scandir
        def scan(candidate):
            if os.fspath(candidate)==str(blocked):raise PermissionError('Gannet denied subtree')
            return actual(candidate)
        with mock.patch.object(self.brain.os,'scandir',side_effect=scan):
            with self.assertRaisesRegex(self.brain.BrainError,'inventory unavailable'):
                next(self.brain.iter_records(self.root))
            layers,_,warnings=self.context.refresh_layers(self.connection,self.root)
            self.assertTrue(all(value=='failed' for value in layers.values()));self.assertTrue(warnings)
        self.assertEqual(before,self.snapshot())

    def test_stat_and_iteration_errors_are_unknown_not_absent(self):
        authority=self.root/'project-brain/dynamic';actual=Path.lstat
        def stat(candidate):
            if candidate==authority:raise PermissionError('Gannet metadata unavailable')
            return actual(candidate)
        with mock.patch.object(Path,'lstat',stat):
            with self.assertRaisesRegex(self.brain.BrainError,'inventory unavailable'):
                list(self.brain.iter_records(self.root))

    def test_error_text_not_found_never_authorizes_native_creation(self):
        before=sorted((self.root/'project-brain/dynamic').rglob('*.md'))
        with mock.patch.object(self.brain,'find_record',side_effect=self.brain.BrainError(PUBLIC['not_found_phrase'])):
            with self.assertRaises(self.brain.BrainError):
                self.brain.create_record(self.root,'finding','gannet-new-alias','Gannet independent finding',[],[],owner='local')
        self.assertEqual(before,sorted((self.root/'project-brain/dynamic').rglob('*.md')))

    def test_error_text_not_found_never_becomes_an_absent_learning(self):
        with mock.patch.object(self.results,'find_record',side_effect=self.brain.BrainError(PUBLIC['not_found_phrase'])):
            with self.assertRaises(self.brain.BrainError):self.results._existing(self.root,'gannet-learning-lookup')

    def test_rebind_error_text_not_found_never_authorizes_autoprovision(self):
        alias='feature/gannet-not-created';before=sorted((self.root/'project-brain/dynamic').rglob('*.md'))
        with mock.patch.object(self.context,'rebind_governed_task',side_effect=self.context.ContextError(PUBLIC['not_found_phrase'])):
            with self.assertRaises(self.context.ContextError):
                self.context.ensure_working_task(self.connection,self.root,alias,'governed','local')
        self.assertEqual(before,sorted((self.root/'project-brain/dynamic').rglob('*.md')))

    def test_unknown_promotion_source_is_not_retired_from_error_text(self):
        proposal={'source_records':[{'id':'gannet-source-image','revision':1,'path':'project-brain/dynamic/findings/gannet.md'}]}
        with mock.patch.object(self.brain,'find_record',side_effect=self.brain.BrainError(PUBLIC['not_found_phrase'])):
            self.assertIsNone(self.brain.obsolete_promotion(self.root,proposal,{}))

    def test_handoff_type_in_dynamic_authority_does_not_hide_identity(self):
        path,record,body=self.brain.find_record(self.root,self.task)
        record={**record,'type':'handoff'}
        path.write_text(self.brain.render_markdown_record(record,body),encoding='utf-8')
        problems=[];list(self.brain.iter_records(self.root,problems=problems))
        self.assertEqual([path],[problem['path'] for problem in problems])
        self.assertRegex(problems[0]['error'],'handoff.*authority')
        # Skipped, its identity still answers: no lookup reports absence.
        with self.assertRaisesRegex(self.brain.BrainError,'record invalid'):
            self.brain.find_record(self.root,self.task)

    def test_atomic_publication_failure_preserves_every_published_row(self):
        before=self.snapshot()
        self.connection.execute("CREATE TRIGGER gannet_cut BEFORE INSERT ON document_metadata BEGIN SELECT RAISE(ABORT,'Gannet publication cut'); END")
        self.connection.commit()
        (self.root/'specs/gannet-cache.md').write_text(PUBLIC['sources']['specs/gannet-cache.md']+'\nGannet newly changed public source.\n',encoding='utf-8')
        layers,_,warnings=self.context.refresh_layers(self.connection,self.root)
        self.assertTrue(all(value=='failed' for value in layers.values()));self.assertTrue(warnings)
        self.assertEqual(before,self.snapshot())

    def test_proven_working_only_capsule_keeps_task_path_and_one_warning(self):
        self.connection.execute("CREATE TRIGGER gannet_cut BEFORE INSERT ON document_metadata BEGIN SELECT RAISE(ABORT,'Gannet publication cut'); END")
        self.connection.commit()
        (self.root/'specs/gannet-cache.md').write_text(PUBLIC['sources']['specs/gannet-cache.md']+'\nGannet changed source.\n',encoding='utf-8')
        result=self.process('refresh','--query',PUBLIC['query'],'--task-id',self.task,'--host','codex','--ephemeral')
        self.assertEqual(1,result.returncode);text=result.stdout.decode('utf-8')
        self.assertEqual(1,text.count('warning: Index refresh failed:'))
        self.assertIn(f'task record: project-brain/dynamic/tasks/{self.uuid}.md',text)
        self.assertNotIn(PUBLIC['fact'],text)

    def test_positive_native_absence_still_allows_normal_creation(self):
        with self.assertRaises(self.brain.BrainError) as missing:
            self.brain.find_record(self.root,'gannet-genuine-absence')
        self.assertEqual('BrainRecordNotFound',type(missing.exception).__name__)
        created=self.brain.create_record(self.root,'finding','gannet-genuine-absence','Gannet genuine independent fact',[],[],owner='local')
        self.assertEqual(created['id'],self.brain.find_record(self.root,'gannet-genuine-absence')[1]['id'])

    def test_mcp_unknown_lookup_is_not_reclassified_by_error_text(self):
        server=self.mcp.Memory(self.root)
        with mock.patch.object(self.mcp,'get_record',side_effect=self.brain.BrainError(PUBLIC['not_found_phrase'])):
            with self.assertRaises(self.brain.BrainError):server.existing('gannet-mcp-lookup')

    def test_working_warning_cites_actual_renamed_task_image(self):
        original,_,_=self.brain.find_task(self.root,self.task)
        renamed=original.with_name('gannet-renamed.md');original.rename(renamed)
        self.connection.execute("CREATE TRIGGER gannet_cut BEFORE INSERT ON document_metadata BEGIN SELECT RAISE(ABORT,'Gannet publication cut'); END")
        self.connection.commit()
        result=self.process('refresh','--query',PUBLIC['query'],'--task-id',self.task,'--host','codex','--ephemeral')
        text=result.stdout.decode('utf-8');self.assertEqual(1,result.returncode)
        self.assertIn('task record: '+renamed.relative_to(self.root).as_posix(),text)
        self.assertNotIn('task record: '+original.relative_to(self.root).as_posix(),text)
        self.assertEqual(1,text.count('warning: Index refresh failed:'))

    @unittest.skipIf(os.name=='nt','actual installed directory symlink requires Windows symlink privileges')
    def test_installed_parent_link_and_dangling_link_are_not_absence(self):
        temporary=tempfile.TemporaryDirectory(prefix='gannet-outside-');self.addCleanup(temporary.cleanup)
        outside=Path(temporary.name)/'outside-brain';anchor=self.root/'project-brain';anchor.rename(outside)
        before={str(f.relative_to(outside)):f.read_bytes() for f in outside.rglob('*') if f.is_file()}
        anchor.symlink_to(outside,target_is_directory=True)
        with self.assertRaisesRegex(self.brain.BrainError,'inventory unavailable'):list(self.brain.iter_records(self.root))
        with self.assertRaisesRegex(self.brain.BrainError,'inventory unavailable'):
            self.brain.create_record(self.root,'finding','gannet-linked-alias','Gannet linked record',[],[],owner='local')
        self.assertEqual(before,{str(f.relative_to(outside)):f.read_bytes() for f in outside.rglob('*') if f.is_file()})
        anchor.unlink();anchor.symlink_to(outside/'missing',target_is_directory=True)
        with self.assertRaisesRegex(self.brain.BrainError,'inventory unavailable'):list(self.brain.iter_records(self.root))

    def test_native_unhashable_identity_is_unknown_not_absent(self):
        original,record,body=self.brain.find_task(self.root,self.task)
        for identifier in ([],{}):
            with self.subTest(identity=identifier):
                original.write_text(self.brain.render_markdown_record({**record,'id':identifier},body),encoding='utf-8')
                with self.assertRaisesRegex(self.brain.BrainError,'identity'):
                    self.brain.find_record(self.root,'gannet-absent-identity')
                result=self.process('validate','--json');self.assertEqual(1,result.returncode)
                self.assertNotIn(b'Traceback',result.stderr)

    def test_handoff_unhashable_task_identity_is_isolated(self):
        record,body=self.brain.parse_markdown_record(self.handoff)
        for identifier in ([],{}):
            with self.subTest(identity=identifier):
                self.handoff.write_text(self.brain.render_markdown_record({**record,'task_id':identifier},body),encoding='utf-8')
                indexed=self.context.index_repository(self.connection,self.root,incremental=True)
                self.assertIn({'path':self.handoff.relative_to(self.root).as_posix(),'reason':'task-filter-or-invalid'},indexed['excluded'])
                self.assertIn(PUBLIC['fact'],self.refresh()['capsule_text'])

    def test_archive_denial_is_known_before_any_active_record_is_yielded(self):
        archive=self.root/'project-brain/archive';archive.mkdir(exist_ok=True)
        actual=os.scandir
        def scan(candidate):
            if os.fspath(candidate)==str(archive):raise PermissionError('Gannet archive denied')
            return actual(candidate)
        with mock.patch.object(self.brain.os,'scandir',side_effect=scan):
            with self.assertRaisesRegex(self.brain.BrainError,'inventory unavailable'):
                next(self.brain.iter_records(self.root,include_archive=True))

    def test_scanner_next_error_prevents_any_native_yield(self):
        authority=self.root/'project-brain/dynamic';actual=os.scandir
        class FailedScan:
            def __enter__(self):return self
            def __exit__(self,*args):return False
            def __iter__(self):return self
            def __next__(self):raise PermissionError('Gannet scanner next denied')
        def scan(candidate):return FailedScan() if os.fspath(candidate)==str(authority) else actual(candidate)
        with mock.patch.object(self.brain.os,'scandir',side_effect=scan):
            with self.assertRaisesRegex(self.brain.BrainError,'inventory unavailable'):
                next(self.brain.iter_records(self.root))

    def exchange(self,name,arguments):
        frames=[{'jsonrpc':'2.0','id':0,'method':'initialize','params':{
            'protocolVersion':'2025-06-18','capabilities':{},'clientInfo':{'name':'Gannet public control','version':'1'}}},
            {'jsonrpc':'2.0','method':'notifications/initialized'},
            {'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':arguments}}]
        result=subprocess.run([sys.executable,'-B',str(self.cli.parent/'mcp_server.py'),'--root',str(self.root)],
            input=''.join(json.dumps(frame)+'\n' for frame in frames).encode('utf-8'),env=self.env,
            cwd=self.root,capture_output=True,timeout=20)
        self.assertEqual(0,result.returncode);self.assertEqual(b'',result.stderr)
        replies=[json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(2,len(replies));self.assertEqual([0,1],[reply['id'] for reply in replies])
        return replies[1]['result']

    def test_actual_mcp_repeated_save_and_recall_preserves_progress_and_one_learning(self):
        first={'progress':'Verified Gannet cache lifetime','next_steps':['Check Gannet stale entries'],'verified':True,
            'learnings':[{'type':'finding','title':'Gannet cache lifetime','consequence':PUBLIC['fact'],'sources':['specs/gannet-cache.md']}]}
        second={'progress':'Completed Gannet cache verification','next_steps':['Review Gannet gateway deployment'],'verified':True,
            'learnings':[{'type':'finding','title':'Gannet cache lifetime',
                'consequence':'Expire Gannet gateways after nineteen minutes and select the western keeper.','sources':['specs/gannet-cache.md']}]}
        identities=[]
        for identifier,payload in (('gannet-first-save',first),('gannet-second-save',second)):
            revision=self.brain.get_task(self.root,self.task)['revision']
            result=self.exchange('memory_record_result',{'task_id':self.task,'revision':revision,'result_id':identifier,**payload})
            self.assertFalse(result['isError'],result)
            saved=result['structuredContent'];identities.append(saved['records'][0]['id'])
            task=self.brain.get_task(self.root,self.task)
            self.assertEqual(payload['progress'],task['progress']);self.assertEqual(payload['next_steps'],task['next_steps'])
        self.assertEqual(identities[0],identities[1])
        record=self.brain.find_record(self.root,identities[1])[1]
        self.assertEqual(second['learnings'][0]['consequence'],record['progress'])
        reply=self.exchange('memory_retrieve',{'task_id':self.task,'query':PUBLIC['query']})
        self.assertFalse(reply['isError']);self.assertIn(PUBLIC['fact'],reply['structuredContent']['capsule_text'])
        self.assertIn('specs/gannet-cache.md',reply['structuredContent']['capsule_text'])

    def test_actual_mcp_corrupt_lookup_returns_fixed_error_without_new_records(self):
        bad=self.root/'project-brain/dynamic/findings/gannet-not found.md';bad.parent.mkdir(exist_ok=True)
        raw=bytes.fromhex(PUBLIC['invalid_utf8_hex']);bad.write_bytes(raw)
        before={str(f.relative_to(self.root)):f.read_bytes() for f in (self.root/'project-brain/dynamic').rglob('*.md')}
        reply=self.exchange('memory_checkpoint',{'task_id':'gannet-mcp-never-created'})
        self.assertTrue(reply['isError']);text=reply['content'][0]['text']
        self.assertNotIn(str(self.root),text);self.assertNotIn('UnicodeDecodeError',text)
        self.assertEqual(before,{str(f.relative_to(self.root)):f.read_bytes() for f in (self.root/'project-brain/dynamic').rglob('*.md')})
        self.assertEqual(raw,bad.read_bytes())

    def test_native_shape_errors_are_typed_and_isolated(self):
        original,record,body=self.brain.find_task(self.root,self.task)
        relative=original.relative_to(self.root).as_posix()
        for key,value in (('status',[]),('status',{}),('source_fingerprints',None),('progress',[]),('revision',True)):
            with self.subTest(field=key,value=value):
                image={**record,key:value}
                with self.assertRaises(self.brain.BrainError):self.brain.validate_record(image)
                original.write_text(self.brain.render_markdown_record(image,body),encoding='utf-8')
                # The invalid record leaves the index; the rest refreshes.
                layers,_,_=self.context.refresh_layers(self.connection,self.root)
                self.assertTrue(all(v=='updated' for v in layers.values()))
                self.assertEqual(0,self.connection.execute('SELECT COUNT(*) FROM documents WHERE path=?',(relative,)).fetchone()[0])
                self.assertTrue(any(relative in error for error in self.brain.validate_repository(self.root,check_freshness=False)))
        history=copy.deepcopy(record['transitions'])
        history.append({'from':record['status'],'to':[],'at':record['updated_at'],'actor':'local','reason':'Gannet invalid transition control'})
        with self.assertRaises(self.brain.BrainError):self.brain.validate_record({**record,'transitions':history})

    def test_handoff_shape_errors_are_isolated_without_hashing_containers(self):
        record,body=self.brain.parse_markdown_record(self.handoff)
        for key,value in (('status',[]),('status',{}),('current_state',[]),('revision',True)):
            with self.subTest(field=key,value=value):
                image={**record,key:value}
                with self.assertRaises(self.brain.BrainError):self.brain.validate_handoff(image)
                self.handoff.write_text(self.brain.render_markdown_record(image,body),encoding='utf-8')
                indexed=self.context.index_repository(self.connection,self.root,incremental=True)
                self.assertIn({'path':self.handoff.relative_to(self.root).as_posix(),'reason':'task-filter-or-invalid'},indexed['excluded'])
                self.assertIn(PUBLIC['fact'],self.refresh()['capsule_text'])

    def test_known_closed_archived_handoff_remains_a_derived_slot(self):
        record,body=self.brain.parse_markdown_record(self.handoff)
        archive=self.root/f'project-brain/archive/handoffs/{self.uuid}.md';archive.parent.mkdir(parents=True)
        archive.write_text(self.brain.render_markdown_record({**record,'status':'closed'},body),encoding='utf-8')
        identities=[item[1]['id'] for item in self.brain.iter_records(self.root,include_archive=True)]
        self.assertEqual([self.uuid],identities)

    def test_invalid_archived_handoff_shape_cannot_hide_unknown_identity(self):
        record,body=self.brain.parse_markdown_record(self.handoff)
        archive=self.root/f'project-brain/archive/handoffs/{self.uuid}.md';archive.parent.mkdir(parents=True)
        for key,value in (('schema_version',True),('status',[]),('external_id','gannet-hidden-alias')):
            with self.subTest(field=key):
                raw=self.brain.render_markdown_record({**record,'status':'closed',key:value},body)
                archive.write_text(raw,encoding='utf-8')
                problems=[];list(self.brain.iter_records(self.root,include_archive=True,problems=problems))
                self.assertIn(archive,[problem['path'] for problem in problems])
                if key=='external_id':
                    with self.assertRaises(self.brain.BrainError):
                        self.brain.find_record(self.root,value,include_archive=True)
                self.assertEqual(raw,archive.read_text(encoding='utf-8'))


if __name__=='__main__':unittest.main()
