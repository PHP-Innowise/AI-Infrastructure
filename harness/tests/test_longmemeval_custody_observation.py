"""NEW mocked-source controls; no closed corpus or native filesystem replay."""
import copy
from contextlib import ExitStack
import hashlib
import json
from pathlib import Path, PurePosixPath
import stat
import sys
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'src'))
from harness import longmemeval_projection as projection

FIXTURE=Path(__file__).resolve().parent/'fixtures/longmemeval_custody_observation'

class ReflectiveAbort(BaseException):
    @property
    def __class__(self):raise AssertionError('reflection_forbidden')
    def __str__(self):raise AssertionError('formatting_forbidden')

class InvalidTuple(tuple):
    def __len__(self):raise AssertionError('length_forbidden')

class InvalidString(str):
    def __hash__(self):raise AssertionError('hash_forbidden')

class Source:
    """Independent public stat/images; every mocked OS/hash call is recorded."""
    def __init__(self,oracle,case):
        self.oracle,self.case=oracle,case;self.trace=[]
        self.file_stats=self.reads=self.paths=0
        self.error=OSError('public-control-error-not-retained')
        self.abort=ReflectiveAbort('public-control-abort-not-retained')
        self.real_sha=hashlib.sha256
    def lstat(self,node):
        name='root' if node.name=='public-new-diagnostic-901' else 'specs' if node.name=='specs' else 'source'
        self.trace.append('lstat:'+name)
        if name=='root' and self.case['action']=='directory_stat':raise self.error
        dev,ino=self.oracle['directory_ids'][name]
        return SimpleNamespace(st_dev=dev,st_ino=ino,st_mode=stat.S_IFREG if name=='root' and self.case['action']=='directory_type' else stat.S_IFDIR|0o700)
    def fstat(self,fd):
        if fd in (10,11,12):
            self.trace.append('fstat:'+str(fd));name={10:'root',11:'specs',12:'source'}[fd]
            dev,ino=self.oracle['directory_ids'][name]
            if fd==10 and self.case['action']=='directory_identity':ino+=1
            return SimpleNamespace(st_dev=dev,st_ino=ino)
        self.file_stats+=1;part='first' if self.file_stats==1 else 'after';self.trace.append('fstat:20:'+part)
        if self.case['action']=='file_stat_before' and part=='first':raise self.error
        if self.case['action']=='file_stat_after' and part=='after':raise self.error
        data=dict(self.oracle['file_stat']);action=self.case['action']
        if action in ('file_nonregular','close_masks_shape'):data['st_mode']=stat.S_IFDIR|0o700
        if action=='file_links':data['st_nlink']=2
        if action=='file_size':data['st_size']+=1
        if action=='file_identity':data['st_ino']+=1
        if action=='file_generation_changed' and part=='after':data['st_mtime_ns']+=1
        return SimpleNamespace(**data)
    def path(self,value):
        self.paths+=1;self.trace.append('path:expected' if self.paths==1 else 'path:open')
        if self.case['action']=='expected_entries':raise self.error
        return PurePosixPath(value)
    def lseek(self,fd,position,how):return 0  # The rewind before a scan is not a custody stage.
    def scandir(self,fd):
        self.trace.append('scandir:'+str(fd))
        if self.case['action']=='scan_open':raise self.error
        source=self
        class Scanner:
            def __enter__(self):source.trace.append('scan_enter');self.done=False;return self
            def __iter__(self):return self
            def __next__(self):
                if self.done or source.case['action']=='missing_entry':source.trace.append('scan_end');raise StopIteration
                self.done=True;source.trace.append('scan_item')
                if source.case['action']=='scan_iteration':raise source.error
                name='public-other-stage-907.md' if source.case['action']=='unexpected_entry' else PurePosixPath(source.oracle['relative']).name
                return SimpleNamespace(name=name)
            def __exit__(self,*args):
                source.trace.append('scan_exit')
                if source.case['action']=='scan_exit':raise source.error
        return Scanner()
    def open(self,*args,**kwargs):
        self.trace.append('open:20')
        if self.case['action']=='file_open':raise self.error
        return 20
    def read(self,fd,maximum):
        self.reads+=1;self.trace.append('read:chunk' if self.reads==1 else 'read:eof')
        if self.case['action']=='file_read':raise self.error
        if self.case['action']=='read_abort':raise self.abort
        if self.reads>1:return b''
        raw=self.oracle['body'].encode()
        if self.case['action']=='file_short_image':return raw[:-1]
        if self.case['action']=='file_digest_changed':return b'B'+raw[1:]
        return raw
    def close(self,fd):
        self.trace.append('close:'+str(fd))
        if self.case['action'] in ('file_close','close_masks_shape'):raise self.error
    def digest(self,raw):self.trace.append('sha256');return self.real_sha(raw)

class Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.oracle=json.loads((FIXTURE/'controls.json').read_bytes())
        cls.accessor=json.loads((FIXTURE/'accessor-controls.json').read_bytes())
        cls.storage=json.loads((FIXTURE/'storage-controls.json').read_bytes())
        cls.constructor=json.loads((FIXTURE/'constructor-controls.json').read_bytes())
    def owner(self,prior=None):
        memory=projection.NativeProjectionMemory.__new__(projection.NativeProjectionMemory)
        memory.repository=Path('/public-new-diagnostic-901');memory._lock=threading.RLock()
        memory._root_fd,memory._specs_fd,memory._source_fd=10,11,12
        memory._closed=memory._fenced=False;memory._custody_failure=tuple(prior) if prior else ('unobserved',None)
        item=projection.Projection(self.oracle['trajectory_id'],self.oracle['relative'],self.oracle['body'].encode(),self.oracle['body_sha256'],'0'*64,1)
        memory._documents={item.trajectory_id:item};memory._file_ids={item.trajectory_id:(1,901)}
        memory._last=copy.deepcopy(self.accessor['query_metadata']);return memory
    def run_case(self,case):
        memory=self.owner(case['prior']);source=Source(self.oracle,case);capture=[]
        writes=[]
        if 'setter_failure' in case:
            class Blocked(projection.NativeProjectionMemory):
                def __setattr__(self,name,value):
                    if name=='_custody_failure':
                        writes.append(name)
                        if case['setter_failure']=='abort':raise ReflectiveAbort()
                        raise ValueError()
                    super().__setattr__(name,value)
            blocked=Blocked.__new__(Blocked);blocked.__dict__.update(memory.__dict__);memory=blocked
        if case['action']=='pre_fenced':memory._fenced=True
        if case['action']=='pre_closed':memory._closed=True
        original=memory._capture_custody_failure
        def capturer(stage):
            capture.append(stage)
            if case['capture']=='disabled':return
            if case['capture']=='dirty_error':memory._custody_failure=('stage_reported','file_stat_before');raise ValueError()
            if case['capture']=='error':raise ValueError()
            if case['capture']=='abort':raise KeyboardInterrupt()
            original(stage)
        caught=None
        with ExitStack() as stack:
            # A replacement descriptor is bound explicitly; it receives the Path.
            stack.enter_context(patch.object(Path,'lstat',lambda node:source.lstat(node)))
            for name in ('fstat','lseek','scandir','open','read','close'):stack.enter_context(patch.object(projection.os,name,getattr(source,name)))
            stack.enter_context(patch.object(projection,'PurePosixPath',source.path))
            stack.enter_context(patch.object(projection.hashlib,'sha256',source.digest))
            stack.enter_context(patch.object(memory,'_capture_custody_failure',capturer))
            try:memory._custody()
            except BaseException as error:caught=error
            record=memory.custody_failure_observation()
        if case['outcome']=='returned':self.assertIsNone(caught)
        elif case['outcome']=='original_abort':self.assertIs(caught,source.abort)
        else:
            self.assertIs(type(caught),projection.ProjectionError);self.assertEqual(caught.args,(case['outcome'],))
            if case['action'] in ('file_open','file_close','close_masks_shape','file_read','file_stat_before','file_stat_after','directory_stat','expected_entries','scan_open','scan_iteration','scan_exit'):
                self.assertIs(caught.__context__,source.error)
        self.assertEqual(source.trace,case['expected_trace']);self.assertEqual(record,case['expected_record'])
        self.assertEqual(memory._fenced,case['fenced']);self.assertEqual(memory._last,self.accessor['query_metadata'])
        self.assertEqual(len(capture),case.get('capture_calls',1 if case['outcome']=='source_changed' else 0))
        if 'metadata_write_attempts' in case:self.assertEqual(len(writes),case['metadata_write_attempts'])
        return memory
    def group(self,names):
        self.assertTrue(names <= {case['name'] for case in self.oracle['cases']})
        seen=0
        for case in self.oracle['cases']:
            if case['name'] in names:
                seen+=1
                with self.subTest(name=case['name']):self.run_case(case)
        self.assertEqual(seen,len(names))
    def test_directory_and_population_regions_preserve_whole_traces(self):
        self.group({'directory_stat_oserror','directory_type_rejected','directory_binding_rejected','expected_entries_oserror','scan_open_oserror','unexpected_entry','scan_iteration_oserror','scan_exit_oserror','missing_entry'})
    def test_leaf_metadata_regions_keep_compound_short_circuit(self):
        self.group({'file_open_oserror','file_stat_before_oserror','file_nonregular','file_links','file_size','file_identity_rejected'})
    def test_read_image_regions_keep_lazy_hash_and_cleanup(self):
        self.group({'file_read_oserror','file_stat_after_oserror','file_generation_changed','file_short_image','file_digest_changed'})
    def test_close_failure_keeps_existing_masking_and_close_site(self):
        self.group({'file_close_oserror','close_masks_shape'})
    def test_capture_failures_disabled_and_stale_do_not_mask_primary(self):
        self.group({'capture_ordinary_failure','capture_abort_failure','capture_disabled','stale_capture_failure'})
        self.run_case(self.accessor['capture_dirty_error'])
    def test_metadata_write_failure_cannot_replace_primary_refusal(self):
        for case in self.storage['cases']:
            with self.subTest(name=case['name']):self.run_case(case)
    def test_initial_metadata_write_failure_preserves_configuration_guard(self):
        oracle=self.constructor;writes=[];lookups=[];checks=[];calls=[]
        class Blocked(projection.NativeProjectionMemory):
            def __setattr__(self,name,value):
                if name=='_custody_failure':writes.append(name);raise ReflectiveAbort()
                super().__setattr__(name,value)
        class Native:
            def __getattr__(self,name):
                lookups.append(name)
                def never(*args,**kwargs):calls.append(name);raise AssertionError('native_call_forbidden')
                return never
        memory=Blocked.__new__(Blocked)
        def absent(node):checks.append(str(node));return False
        with patch.object(Path,'is_file',absent),patch.object(projection.os,'open',side_effect=AssertionError('native_fs_forbidden')):
            with self.assertRaises(projection.ProjectionError) as caught:memory.__init__(oracle['root'],Native())
        self.assertEqual(caught.exception.args,(oracle['expected_guard'],))
        self.assertEqual(lookups,oracle['native_methods']);self.assertEqual(checks,oracle['expected_configuration_checks'])
        self.assertEqual(len(writes),oracle['expected_metadata_write_attempts']);self.assertEqual(len(memory._fds),oracle['expected_fd_population'])
        self.assertEqual(calls,[]);self.assertEqual(memory.custody_failure_observation(),oracle['expected_record'])
    def test_uncaught_abort_identity_and_availability_outside_coverage(self):
        self.group({'uncaught_read_abort','fenced_prebody','closed_prebody'})
    def test_success_default_and_last_failure_retention(self):
        self.group({'clean_success','prior_success_retained'})
    def test_detached_accessor_and_query_schema_are_independent(self):
        memory=self.owner(['stage_reported','file_open']);got=memory.custody_failure_observation()
        got['stage']='caller-edited';got['private-extra']='public-irrelevant'
        self.assertEqual(memory.custody_failure_observation(),self.accessor['detached_expected'])
        self.assertEqual(memory.post_query_hook(),self.accessor['query_metadata'])
    def test_invalid_private_images_are_bounded_unmeasured(self):
        values={'none':None,'dict':{},'wrong_length':('unobserved',),'wrong_state':('unknown',None),'wrong_state_type':(1,None),'wrong_stage_type':('stage_reported',[]),'oversize_stage':('stage_reported','x'*33),'unknown_stage':('stage_reported','other'),'unobserved_with_stage':('unobserved','file_open'),'unmeasured_with_stage':('unmeasured','file_read'),'tuple_subclass':InvalidTuple(('stage_reported','file_open')),'string_subclass':('stage_reported',InvalidString('file_open'))}
        for tag in self.accessor['invalid_tags']:
            with self.subTest(tag=tag):
                memory=self.owner();memory._custody_failure=values[tag]
                self.assertEqual(memory.custody_failure_observation(),self.accessor['invalid_expected'])
    def test_getter_reads_image_after_lock_admission(self):
        memory=self.owner(self.accessor['lock_before']);events=[];oracle=self.accessor
        class Admission:
            def __enter__(self):events.append('enter');memory._custody_failure=tuple(oracle['lock_at_enter'])
            def __exit__(self,*args):events.append('exit')
        memory._lock=Admission()
        self.assertEqual(memory.custody_failure_observation(),self.accessor['lock_expected'])
        self.assertEqual(events,self.accessor['lock_events'])

if __name__=='__main__':unittest.main()
