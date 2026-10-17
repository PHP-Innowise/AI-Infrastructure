"""Fresh Swan supplied dual-fault controls; no actor/resource-readiness claim."""
import copy
import stat
import types
import unittest
from unittest import mock
import tests.test_memory_final_source_image as wren


class FinalReaderAbortTests(wren.FinalSourceImageTests):
    def faults(self, stage, primary, cleanup):
        events=[]
        def close():
            events.append('close')
            if cleanup is not None:raise cleanup
        def fstat(descriptor):
            events.append('fstat')
            if stage=='fstat':raise primary
            return types.SimpleNamespace(st_mode=stat.S_IFREG)
        class Stream:
            def read(self):
                events.append('read')
                if stage=='read':raise primary
                return 'SwanLine517\nSwanLine628\n'
            def close(self):close()
            def __enter__(self):return self
            def __exit__(self,*args):self.close()
        return events, mock.patch.multiple(self.r.os,
            open=mock.Mock(return_value=117),fstat=fstat,fdopen=mock.Mock(return_value=Stream()),
            close=lambda descriptor:close())

    def check_fault(self, stage, primary, cleanup, expected):
        events,patch=self.faults(stage,primary,cleanup)
        actual=None
        with patch:
            try:self.r._read_final_working_record(self.root/'SwanRecord517.md')
            except BaseException as error:actual=error
        self.observe('swan-dual-fault',{'stage':stage,'events':events,
            'primary':type(primary).__name__ if primary is not None else None,
            'cleanup':type(cleanup).__name__ if cleanup is not None else None,
            'observed':type(actual).__name__ if actual is not None else None,
            'expected_identity':actual is expected})
        self.assertIs(expected,actual)
        self.assertEqual(1,events.count('close'))

    def test_fstat_cancel_survives_raw_close_error(self):
        primary=KeyboardInterrupt('SwanStat517')
        self.check_fault('fstat',primary,OSError('SwanClose628'),primary)

    def test_read_cancel_survives_stream_close_error(self):
        primary=KeyboardInterrupt('SwanRead739')
        self.check_fault('read',primary,OSError('SwanClose841'),primary)

    def test_read_exit_survives_stream_close_error(self):
        primary=SystemExit('SwanExit952')
        self.check_fault('read',primary,OSError('SwanClose163'),primary)

    def test_ordinary_primary_error_keeps_identity(self):
        primary=OSError('SwanStat274')
        self.check_fault('fstat',primary,OSError('SwanClose385'),primary)

    def test_new_cleanup_cancellation_propagates(self):
        cleanup=KeyboardInterrupt('SwanCleanup496')
        self.check_fault('fstat',OSError('SwanStat507'),cleanup,cleanup)

    def test_successful_read_close_error_remains_failure(self):
        cleanup=OSError('SwanClose618')
        self.check_fault(None,None,cleanup,cleanup)

    def test_cancelled_final_read_cannot_publish_receipts(self):
        self.task()
        primary=KeyboardInterrupt('SwanPublication729')
        reader=self.r._read_final_working_record
        events,patch=self.faults('read',primary,OSError('SwanClose830'))
        def read(source_file):
            with patch:return reader(source_file)
        receipts=[]
        actual=None
        result=None
        with mock.patch.object(self.r,'_read_final_working_record',side_effect=read),mock.patch.object(
                self.r,'_commit_ordinary_receipts',side_effect=lambda *a,**k:receipts.append(copy.deepcopy(k))):
            try:
                result=self.r.retrieve(self.db,self.root,wren.PUBLIC['query'],wren.PUBLIC['task'],
                    limit=5,gate_mode='off',manifest_scope='local')
            except BaseException as error:actual=error
        self.observe('swan-publication',{'observed':type(actual).__name__ if actual is not None else None,
            'primary_identity':actual is primary,'receipts':receipts,'result':result,'events':events})
        self.assertIs(primary,actual)
        self.assertEqual([],receipts)
        self.assertIsNone(result)
        self.assertEqual(1,events.count('close'))


for _name in dir(wren.FinalSourceImageTests):
    if _name.startswith('test_'):
        setattr(FinalReaderAbortTests,_name,None)
if __name__=='__main__':unittest.main()
