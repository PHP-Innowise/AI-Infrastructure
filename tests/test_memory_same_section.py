"""Actual CLI delivery of new answers in a previously quoted section."""
import contextlib
import hashlib
import importlib
import io
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
A = 'InvoiceQuantizer rounds displayed totals to integer cents after allocation.'
B = 'RefundGate rejects a reversal unless the original settlement is final.'


class SameSectionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='same-section-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.env = dict(os.environ, GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1')
        for name in ('ACCELERATOR_HOME','ACCELERATOR_PROJECT_DIR','ACCELERATOR_STATE_DIR',
                     'PROJECT_BRAIN_MODE','CONTEXT_CAPSULE_DELIVERED','CONTEXT_MEMORY_RECOVERY'):
            self.env.pop(name, None)
        subprocess.run(['git','init','-q',str(self.root)],env=self.env,check=True)
        (self.root/'docs').mkdir()
        self.document = self.root/'docs/ledger.md'
        self.document.write_text('# Ledger rules\n\n'+A+'\n\n'
                                 +'Ordinary settlement commentary. '*180+'\n\n'+B+'\n')

    def refresh(self, query, session='ledger', *extra):
        completed = subprocess.run([sys.executable,'-B',str(ROOT/'PHP Core/memory-bank/scripts/context.py'),
            '--root',str(self.root),'refresh','--task-id','TASK-LEDGER','--query',query,
            '--session-id',session,'--ephemeral','--json',*extra],env=self.env,capture_output=True,timeout=20)
        self.assertEqual(0,completed.returncode,completed.stderr.decode())
        result = json.loads(completed.stdout)
        self.assertLessEqual(len(result['capsule_text']),3600)
        return result

    def state(self, transform=None):
        # closing(): the connection's own context manager only commits, and an
        # open handle keeps Windows from removing the temporary project.
        with contextlib.closing(sqlite3.connect(self.root/'memory-bank/local/context.db')) as connection, connection:
            row=connection.execute("SELECT key,value FROM index_state WHERE key LIKE '%session%deliver%'").fetchone()
            self.assertIsNotNone(row)
            state=json.loads(row[1])
            if transform is not None:
                transform(state)
                connection.execute('UPDATE index_state SET value=? WHERE key=?',(json.dumps(state),row[0]))
            return state

    @contextlib.contextmanager
    def runtime(self):
        scripts=ROOT/'PHP Core/memory-bank/scripts'
        names={item.stem for item in scripts.glob('*.py')}
        previous={name:sys.modules.pop(name) for name in names if name in sys.modules}
        sys.path.insert(0,str(scripts))
        try:
            yield importlib.import_module('context'),importlib.import_module('context_retrieval')
        finally:
            sys.path.remove(str(scripts))
            for name in names:sys.modules.pop(name,None)
            sys.modules.update(previous)

    def test_new_answer_is_delivered_and_both_old_windows_stay_suppressed(self):
        first=self.refresh('InvoiceQuantizer')
        self.assertIn(A,first['capsule_text']);self.assertNotIn(B,first['capsule_text'])
        second=self.refresh('RefundGate')
        self.assertIn(B,second['capsule_text']);self.assertNotIn(A,second['capsule_text'])
        third=self.refresh('InvoiceQuantizer')
        self.assertNotIn(A,third['capsule_text']);self.assertNotIn(B,third['capsule_text'])
        self.assertEqual(1,third['capsule']['repeated'])
        fourth=self.refresh('RefundGate')
        self.assertNotIn(B,fourth['capsule_text']);self.assertEqual(1,fourth['capsule']['repeated'])

    def test_incidentally_seen_answer_stays_suppressed_when_rank_changes_window(self):
        self.document.write_text('# Ledger rules\n\n'+A+'\n\n'+'Context boundary. '*28+'\n\n'+B+'\n')
        (self.root/'docs/priority.md').write_text('# RefundGate dispatch\n\nRefundGate waits for the processor receipt.\n\n'
                                                +'Ordinary processor commentary. '*40)
        first=self.refresh('InvoiceQuantizer')
        self.assertIn(A,first['capsule_text']);self.assertIn(B,first['capsule_text'])
        second=self.refresh('RefundGate')
        self.assertNotIn(B,second['capsule_text'])
        self.assertNotIn('docs/ledger.md',[item['path'] for item in second['capsule']['semantic']])
        self.assertEqual(1,second['capsule']['repeated'])

    def test_substring_or_qualified_sentence_does_not_cover_unseen_standalone_answer(self):
        for number,first_unit in enumerate(('Not'+B,'Only in archival mode, '+B)):
            with self.subTest(first_unit=first_unit):
                self.document.write_text('# Ledger rules\n\n'+first_unit+'\n\n'
                    +'Ordinary settlement commentary. '*180+'\n\n'+B+'\n')
                session='qualified-'+str(number)
                query='NotRefundGate' if number==0 else 'archival mode'
                first=self.refresh(query,session)
                self.assertIn(first_unit,first['capsule_text'])
                second=self.refresh('RefundGate',session)
                self.assertIn('  … '+B,second['capsule_text'])

    def test_sanitizer_offset_change_does_not_make_seen_answer_novel(self):
        self.document.write_text('# Ledger rules\n\n'+A+'\n\ncustomer ID 10492.\n\n'
                                 +'Context boundary. '*26+'\n\n'+B+'\n')
        (self.root/'docs/priority.md').write_text('# RefundGate dispatch\n\nRefundGate waits for the processor receipt.\n\n'
                                                +'Ordinary processor commentary. '*40)
        first=self.refresh('InvoiceQuantizer')
        self.assertIn(A,first['capsule_text']);self.assertIn(B,first['capsule_text'])
        self.assertNotIn('10492',first['capsule_text'])
        second=self.refresh('RefundGate')
        self.assertNotIn(B,second['capsule_text']);self.assertEqual(1,second['capsule']['repeated'])

    def test_new_session_receives_answer_and_warm_ignored_source_stays_private(self):
        self.refresh('InvoiceQuantizer')
        self.assertIn(B,self.refresh('RefundGate','fresh')['capsule_text'])
        (self.root/'.gitignore').write_text('docs/ledger.md\n')
        result=self.refresh('RefundGate')
        self.assertNotIn(B,result['capsule_text']);self.assertEqual([],result['capsule']['semantic'])

    def test_legacy_or_malformed_coverage_keeps_conservative_section_suppression(self):
        for number,bad in enumerate((None,{'version':True,'receipts':{}},{'version':1,'receipts':[]})):
            with self.subTest(coverage=bad):
                session='legacy-'+str(number)
                self.refresh('InvoiceQuantizer',session)
                def corrupt(state):
                    entry=state[session]
                    if bad is None:entry.pop('render_coverage',None)
                    else:entry['render_coverage']=bad
                self.state(corrupt)
                result=self.refresh('RefundGate',session)
                self.assertNotIn(B,result['capsule_text']);self.assertEqual(1,result['capsule']['repeated'])

    def test_coverage_is_bounded_hash_metadata_without_source_prose(self):
        self.refresh('InvoiceQuantizer');self.refresh('RefundGate')
        coverage=self.state()['ledger']['render_coverage']
        self.assertEqual(1,coverage['version'])
        self.assertLessEqual(len(coverage['receipts']),32)
        for receipt in coverage['receipts'].values():
            self.assertLessEqual(len(receipt['exposures']),4)
            for exposure in receipt['exposures']:
                self.assertLessEqual(len(exposure['units']),32)
                for digest in exposure['units']:self.assertRegex(digest,r'^[0-9a-f]{64}$')
        encoded=json.dumps(coverage)
        for text in (A,B,'InvoiceQuantizer','RefundGate','Ordinary settlement commentary'):
            self.assertNotIn(text,encoded)

    def test_oversized_excerpt_piece_never_claims_a_whole_source_unit(self):
        with self.runtime() as (context,retrieval):
            unit='Before the change '+'background context '*100+B
            spans=[]
            body=retrieval.excerpt_window([unit],800,coverage=spans)
            self.assertEqual([],spans)
            self.assertEqual({'units':[],'matched':[]},context._visible_excerpt_units(body,[]))

    def test_final_render_shrink_covers_only_source_spans_that_survive(self):
        with self.runtime() as (context,retrieval):
            first='QuotaGuard requires a current receipt.'
            second='RefundGate requires '+('the original processor confirmation '*5)+'before reversal.'
            spans=[];text=retrieval.excerpt_window([first,second],800,coverage=spans)
            excerpt=context.Excerpt(text)
            excerpt.units=[(span['text'],1,span['start'],span['end']) for span in spans]
            item={'path':'docs/ledger.md','title':'Ledger','layer':'semantic'}
            capsule={'working':None,'warnings':[],'omitted':{},'semantic':[item],
                     'procedural':[],'episodic':[]}
            coverage={}
            with mock.patch.object(context,'RENDERED_CAPSULE_LIMIT',240):
                lines,shown=context._render_capsule(capsule,{'docs/ledger.md':excerpt},unit_coverage=coverage)
            self.assertEqual([item],shown)
            self.assertIn(first,'\n'.join(lines));self.assertNotIn(second,'\n'.join(lines))
            self.assertEqual([hashlib.sha256(first.encode()).hexdigest()],coverage['docs/ledger.md']['units'])

    def test_overflow_and_unknown_exposure_do_not_authorize_new_windows(self):
        self.refresh('InvoiceQuantizer')
        def overflow(state):
            receipt=next(iter(state['ledger']['render_coverage']['receipts'].values()))
            receipt['exposures'][0]['units']=['%064x'%number for number in range(33)]
        self.state(overflow)
        result=self.refresh('RefundGate')
        self.assertNotIn(B,result['capsule_text'])
        self.assertEqual(1,result['capsule']['repeated'])

    def test_invalid_optional_unit_description_does_not_break_delivery(self):
        with self.runtime() as (context,_):
            for number,invalid in enumerate((None,[],True)):
                with self.subTest(description=invalid):
                    output=io.StringIO();error=io.StringIO()
                    arguments=['context.py','--root',str(self.root),'refresh','--task-id','TASK-LEDGER',
                        '--query','InvoiceQuantizer','--session-id','invalid-'+str(number),'--ephemeral','--json']
                    with mock.patch.object(context,'describe_excerpt_units',return_value=invalid), \
                            mock.patch.object(sys,'argv',arguments), \
                            contextlib.redirect_stdout(output),contextlib.redirect_stderr(error):
                        code=context.main()
                    self.assertEqual(0,code,error.getvalue())
                    self.assertIn(A,json.loads(output.getvalue())['capsule_text'])


if __name__=='__main__':unittest.main()
