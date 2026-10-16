"""NEW Oriel public byte oracles, scanner work and cooperative cancellation."""
from __future__ import annotations

from dataclasses import fields, replace
import hashlib
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'harness/src'))
from harness import hindsight_export as he, hindsight_journal as hj
from harness.hindsight_recall import Scope

PUBLIC=json.loads((ROOT/'tests/fixtures/hindsight_screen_forward_public.json').read_text(encoding='utf-8'))


class CountedText(str):
    def __new__(cls,value):
        result=super().__new__(cls,value);result.inspections=0;return result

    def __getitem__(self,index):
        result=super().__getitem__(index)
        self.inspections+=len(result) if isinstance(index,slice) else 1
        return result


class HindsightForwardScreenTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='oriel-screen-')
        self.addCleanup(temporary.cleanup);self.root=Path(temporary.name)
        self.workspace=self.root/'public';self.workspace.mkdir()
        self.scope=Scope(PUBLIC['team_id'],PUBLIC['project_id'])
        approvals=[]
        for item in PUBLIC['sources']:
            raw=item['text'].encode('utf-8');leaf=self.workspace/item['path']
            leaf.parent.mkdir(parents=True,exist_ok=True);leaf.write_bytes(raw)
            approvals.append(he.PublicApproval(self.scope,item['path'],hashlib.sha256(raw).hexdigest()))
        self.approvals=tuple(approvals)

    def selected(self):
        return he.build_public_export(self.workspace,self.scope,self.approvals)

    def denied(self,reason,function,*args,**kwargs):
        with self.assertRaises(he.ExportError) as error:function(*args,**kwargs)
        self.assertEqual(reason,error.exception.reason)

    def journal(self):
        db=sqlite3.connect(self.root/'private.sqlite3',timeout=.01);self.addCleanup(db.close)
        db.execute('PRAGMA journal_mode=WAL');db.execute('PRAGMA synchronous=FULL')
        return hj.create_new(db,self.scope,PUBLIC['journal_id']),db

    def snapshot(self,db):
        return {table:db.execute('SELECT * FROM '+table+' ORDER BY rowid').fetchall()
                for table in ('journal','generations','intents','attempts')}

    def test_new_whole_source_request_and_packet_oracles_preserved(self):
        selected=self.selected()
        self.assertEqual(PUBLIC['oracle']['packet'].encode('utf-8'),selected.serialized)
        for item,expected,draft in zip(PUBLIC['sources'],PUBLIC['oracle']['sources'],selected.retain_drafts):
            self.assertEqual(expected['body'].encode('utf-8'),draft.body)
            self.assertEqual(expected['document_id'],draft.binding.document_id)
            self.assertEqual(expected['sha256'],draft.binding.source_sha256)
            self.assertEqual(item['text'].encode('utf-8'),(self.workspace/item['path']).read_bytes())
        self.assertEqual(0,selected.diagnostics['executed_transport_calls'])
        self.assertIsNone(selected.diagnostics['provider_tokens'])
        self.assertFalse(selected.diagnostics['total_cost_complete'])

    def test_ordinary_json_strings_do_not_consume_decoded_view_allowance(self):
        original=he.json.loads
        with mock.patch.object(he.json,'loads',wraps=original) as decoder:
            he._screen(PUBLIC['simple_json_literal'])
        self.assertEqual(0,decoder.call_count)

    def test_changed_view_depth_and_aggregate_limits_remain_fail_closed(self):
        he._screen(PUBLIC['changed_under_limit_literal'])
        self.denied('source_text',he._screen,PUBLIC['changed_limit_literal'])
        self.denied('source_text',he._screen,PUBLIC['aggregate_limit_literal'])
        self.denied('source_text',he._screen,PUBLIC['boundaries']['standalone_depth_literal'])

    def test_known_unicode_refusals_and_opaque_language_contract(self):
        for case in PUBLIC['reject']:
            with self.subTest(text=case['text']):
                if case['reason'] is None:he._screen(case['text'])
                else:self.denied(case['reason'],he._screen,case['text'])
        he._screen(PUBLIC['boundaries']['mixed_unsupported_known_escape'])
        self.denied(PUBLIC['boundaries']['terminal_backslash_reason'],he._screen,PUBLIC['boundaries']['terminal_backslash'])

    def test_nested_credential_and_raw_credential_keep_private_refusal(self):
        view=PUBLIC['nested_credential']
        for _ in range(3):view=json.dumps(view)
        self.denied('private_content',he._screen,view)
        self.denied('private_content',he._screen,PUBLIC['nested_credential']+' "unterminated\\')

    def test_forward_span_discovery_has_bounded_inspections_and_no_suffix_restarts(self):
        limit=PUBLIC['controls']['scanner_inspection_multiplier']
        for pairs in PUBLIC['unterminated_escaped_pairs']:
            value=CountedText('"'+'\\"'*pairs);spans=[]
            with self.subTest(pairs=pairs):
                with self.assertRaises(he.ExportError) as error:
                    spans.extend(he._json_string_spans(value))
                self.assertEqual('source_text',error.exception.reason)
                self.assertEqual([],spans)
                self.assertLessEqual(value.inspections,limit*len(value))
        value=CountedText(PUBLIC['simple_json_literal'])
        spans=list(he._json_string_spans(value))
        self.assertEqual(5200,len(spans));self.assertLessEqual(value.inspections,limit*len(value))

    def test_standalone_largest_escaped_run_finishes_under_fixed_hang_guard(self):
        code='import sys;sys.path.insert(0,sys.argv[1]);from harness import hindsight_export as he\ntry:he._screen(chr(34)+(chr(92)+chr(34))*int(sys.argv[2]))\nexcept he.ExportError as error:print(error.reason)'
        try:
            result=subprocess.run([sys.executable,'-B','-c',code,str(ROOT/'harness/src'),
                str(PUBLIC['unterminated_escaped_pairs'][-1])],capture_output=True,
                timeout=PUBLIC['controls']['standalone_hang_seconds'])
        except subprocess.TimeoutExpired:
            self.fail('fixed standalone hang guard exceeded; process killed and reaped by subprocess')
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'))
        self.assertEqual(b'source_text\n',result.stdout);self.assertEqual(b'',result.stderr)

    def cancellation_stage(self,kind):
        stage=[None];polls=[]
        owner,name=(he,'_text') if kind=='text' else (
            (he.RetainDraft,'__post_init__') if kind=='draft' else (he.PublicExport,'__post_init__'))
        original=getattr(owner,name)
        def wrapped(*args,**kwargs):
            prior=stage[0];stage[0]=kind
            try:return original(*args,**kwargs)
            finally:stage[0]=prior
        def active():
            polls.append(stage[0]);return stage[0]!=kind
        with mock.patch.object(owner,name,wrapped):
            self.denied('cancelled',he.build_public_export,self.workspace,self.scope,self.approvals,active=active)
        self.assertIn(kind,polls)

    def test_capture_cancel_during_initial_screen_returns_no_export(self):
        self.cancellation_stage('text')

    def test_capture_cancel_during_draft_constructor_returns_no_export(self):
        self.cancellation_stage('draft')

    def test_capture_cancel_during_export_revalidation_returns_no_export(self):
        self.cancellation_stage('export')

    def test_span_scanner_polls_inside_fixed_chunk(self):
        value=CountedText('"'+'\\"'*PUBLIC['unterminated_escaped_pairs'][-1])
        polls=[]
        def active():
            polls.append(value.inspections);return value.inspections==0
        self.denied('cancelled',lambda:list(he._json_string_spans(value,active=active)))
        self.assertGreaterEqual(len(polls),2)
        self.assertLessEqual(value.inspections,PUBLIC['controls']['scanner_poll_inspections'])

    def test_plan_cancellation_during_current_revalidation_returns_no_plan(self):
        selected=self.selected();phase=[False];original=he.IngestionPlan.__post_init__
        def wrapped(*args,**kwargs):
            phase[0]=True
            try:return original(*args,**kwargs)
            finally:phase[0]=False
        with mock.patch.object(he.IngestionPlan,'__post_init__',wrapped):
            self.denied('cancelled',he.plan_public_ingestion,selected,active=lambda:not phase[0])

    def test_prepare_cancellation_after_inserts_rolls_back_before_commit(self):
        journal,db=self.journal();selected=self.selected();before=self.snapshot(db);observed=[]
        def active():
            head=db.execute('SELECT head FROM journal').fetchone()[0]
            if head:observed.append(head);return False
            return True
        with self.assertRaises(hj.JournalError) as error:
            journal.prepare(selected,PUBLIC['generation'],0,active=active)
        self.assertEqual('input',error.exception.reason);self.assertEqual([1],observed)
        self.assertFalse(db.in_transaction);self.assertEqual(before,self.snapshot(db))
        self.assertIsNone(journal.current())

    def test_request_and_reservation_cancellation_preserve_frontier(self):
        journal,db=self.journal();prepared=journal.prepare(self.selected(),PUBLIC['generation'],0)
        before=self.snapshot(db)
        for function,args,kwargs in ((journal.request_body,(prepared.intent_ids[0],),{}),
                (journal.reserve_attempt,(prepared.intent_ids[0],'a14c7c31-b083-4705-ab83-1d19007a9506'),{'expected_revision':1})):
            with self.subTest(operation=function.__name__):
                with self.assertRaises(hj.JournalError) as error:function(*args,active=lambda:False,**kwargs)
                self.assertEqual('input',error.exception.reason);self.assertEqual(before,self.snapshot(db))

    def test_callbacks_are_init_only_and_default_manual_validation_still_works(self):
        selected=self.selected();draft=selected.retain_drafts[0]
        callback=lambda:True
        new=he.RetainDraft(draft.binding,draft.source_id,draft.body,active=callback)
        export=he.PublicExport(selected.catalogue,selected.retain_drafts,selected.serialized,selected._io,active=callback)
        plan=he.plan_public_ingestion(export,active=callback)
        for obj in (new,export,plan):
            self.assertNotIn('active',[f.name for f in fields(obj)])
            self.assertNotIn(callback,vars(obj).values());obj.__post_init__()
        self.assertEqual(selected.serialized,export.serialized)

    def test_dataclass_replace_preserves_fields_and_explicit_cancellation(self):
        selected=self.selected();plan=he.plan_public_ingestion(selected)
        for obj in (selected.retain_drafts[0],selected,plan):
            with self.subTest(kind=type(obj).__name__):
                replaced=replace(obj)
                self.assertEqual(obj,replaced);replaced.__post_init__()
                self.denied('cancelled',replace,obj,active=lambda:False)

    def test_complete_changed_spans_decode_once_and_preserve_valid_unicode(self):
        oracle=PUBLIC['adversarial_oracles'];original=he.json.loads
        with mock.patch.object(he.json,'loads',wraps=original) as decoder:
            he._screen(oracle['decoder_literal'])
        self.assertEqual(oracle['decoder_inputs'],[x.args[0] for x in decoder.call_args_list])
        self.assertEqual(oracle['decoder_call_count'],decoder.call_count)
        self.assertLessEqual(sum(len(x.args[0].encode('utf-8')) for x in decoder.call_args_list),
                            len(oracle['decoder_literal'].encode('utf-8')))

    def test_reservation_cancellation_after_insert_rolls_back_ticket_and_nonce(self):
        journal,db=self.journal();prepared=journal.prepare(self.selected(),PUBLIC['generation'],0)
        before=self.snapshot(db);observed=[];nonce=PUBLIC['adversarial_oracles']['reservation_nonce']
        def active():
            count=db.execute('SELECT COUNT(*) FROM attempts WHERE nonce=?',(nonce,)).fetchone()[0]
            if count:observed.append(count);return False
            return True
        with self.assertRaises(hj.JournalError) as error:
            journal.reserve_attempt(prepared.intent_ids[0],nonce,expected_revision=1,active=active)
        self.assertEqual('input',error.exception.reason);self.assertEqual([1],observed)
        self.assertEqual(before,self.snapshot(db));self.assertFalse(db.in_transaction)

    def test_historical_integrity_remains_without_current_screen_or_active(self):
        journal,db=self.journal();prepared=journal.prepare(self.selected(),PUBLIC['generation'],0)
        with mock.patch.object(he,'_active',side_effect=AssertionError('history polled current active')), \
             mock.patch.object(he,'_screen',side_effect=AssertionError('history screened current text')):
            fresh=hj.open_existing(db,self.scope,PUBLIC['journal_id'])
            self.assertEqual(prepared,fresh.current())
            self.assertEqual(1,fresh.diagnostics()['generations'])

    def test_callback_failure_is_fixed_and_keyboard_interrupt_rolls_back(self):
        def failed():raise RuntimeError('Oriel callback error must not be retained')
        self.denied('unavailable',he.build_public_export,self.workspace,self.scope,self.approvals,active=failed)
        journal,db=self.journal();selected=self.selected();before=self.snapshot(db)
        with self.assertRaises(hj.JournalError) as error:journal.prepare(selected,PUBLIC['generation'],0,active=failed)
        self.assertEqual('input',error.exception.reason);self.assertEqual(before,self.snapshot(db))
        def interrupted():
            if db.execute('SELECT head FROM journal').fetchone()[0]:raise KeyboardInterrupt
            return True
        with self.assertRaises(KeyboardInterrupt):journal.prepare(selected,PUBLIC['generation'],0,active=interrupted)
        self.assertEqual(before,self.snapshot(db));self.assertFalse(db.in_transaction)


if __name__=='__main__':unittest.main()
