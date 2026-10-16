"""Fresh public history-integrity/current-admission and private SQLite controls."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import re
import sqlite3
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'harness/src'))
from harness import hindsight_export as he, hindsight_journal as hj
from harness.hindsight_recall import Scope

PUBLIC=json.loads((Path(__file__).parent/'fixtures/hindsight_history_admission_public.json').read_text(encoding='utf-8'))


def encoded(value):
    return json.dumps(value,ensure_ascii=False,separators=(',',':'),allow_nan=False).encode('utf-8')


class HindsightHistoryAdmissionTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='sandpiper-history-')
        self.addCleanup(temporary.cleanup);self.root=Path(temporary.name)
        self.workspace=self.root/'public';self.workspace.mkdir()
        self.scope=Scope(PUBLIC['team_id'],PUBLIC['project_id'])
        self.dbfile=self.root/'private.sqlite3';self.db=self.connect()
        self.journal=hj.create_new(self.db,self.scope,PUBLIC['journal_id'])
        self.selection=self.export(PUBLIC['sources'])
        self.prepared=self.journal.prepare(self.selection,PUBLIC['generations'][0],0)
        self.assertEqual(PUBLIC['oracle'][0]['packet'].encode('utf-8'),self.selection.serialized)
        self.assertEqual(PUBLIC['oracle'][0]['manifest'].encode('utf-8'),self.prepared.manifest)

    def connect(self):
        db=sqlite3.connect(self.dbfile,timeout=.01);self.addCleanup(db.close)
        db.execute('PRAGMA journal_mode=WAL');db.execute('PRAGMA synchronous=FULL');return db

    def reopen(self):
        return hj.open_existing(self.connect(),self.scope,PUBLIC['journal_id'])

    def export(self,sources):
        approvals=[]
        for item in sources:
            raw=item['text'].encode('utf-8');leaf=self.workspace/item['path']
            leaf.parent.mkdir(parents=True,exist_ok=True);leaf.write_bytes(raw)
            approvals.append(he.PublicApproval(self.scope,item['path'],hashlib.sha256(raw).hexdigest()))
        return he.build_public_export(self.workspace,self.scope,tuple(approvals))

    def snapshot(self):
        return {table:self.db.execute('SELECT * FROM '+table+' ORDER BY rowid').fetchall()
                for table in ('journal','generations','intents','attempts')}

    def denied(self,reason,function,*args,**kwargs):
        with self.assertRaises(hj.JournalError) as failure:function(*args,**kwargs)
        self.assertEqual(reason,failure.exception.reason)

    def content_drift(self):
        rules=dict(he.SECRET_PATTERNS)
        rules['sandpiper-public-policy-drift']=re.compile(re.escape(PUBLIC['policy_trigger']))
        return mock.patch.object(he,'SECRET_PATTERNS',rules)

    def path_drift(self):
        original=he.source_path_problem
        def screen(name):
            return 'newly-excluded' if name==PUBLIC['sources'][0]['path'] else original(name)
        return mock.patch.object(he,'source_path_problem',side_effect=screen)

    def history_and_settlement(self,drift):
        ticket=self.journal.reserve_attempt(self.prepared.intent_ids[1],PUBLIC['nonces'][0],expected_revision=1)
        before=self.snapshot()
        with drift:
            fresh=self.reopen();self.assertEqual(self.prepared,fresh.current())
            self.assertEqual(1,fresh.diagnostics()['pending_attempts'])
            self.denied('input',fresh.request_body,self.prepared.intent_ids[0])
            self.denied('input',fresh.reserve_attempt,self.prepared.intent_ids[0],PUBLIC['nonces'][1],expected_revision=1)
            self.assertEqual(before,self.snapshot())
            fresh.attest_terminal(ticket,'settled_unknown',profile=hj.QUIESCENT)
            challenge=fresh.challenge(PUBLIC['nonces'][2])
            observations=tuple(hj.DocumentObservation(binding,'source_match') for binding in challenge.expected)
            result=fresh.evaluate(challenge,observations,profile=hj.VISIBLE)
            self.assertEqual('supplied_selection_current',result['state'])
            for key in ('authorized_mutations','executed_transport_calls'):
                self.assertEqual(0,result[key])
            self.assertIsNone(result['provider_tokens']);self.assertFalse(result['total_cost_complete'])
            self.assertEqual(0,fresh.diagnostics()['pending_attempts'])

    def test_content_policy_drift_preserves_history_settlement_and_refuses_dispatch(self):
        self.history_and_settlement(self.content_drift())

    def test_path_policy_drift_preserves_history_settlement_and_refuses_dispatch(self):
        self.history_and_settlement(self.path_drift())

    def test_history_only_operations_never_call_current_screens(self):
        ticket=self.journal.reserve_attempt(self.prepared.intent_ids[0],PUBLIC['nonces'][0],expected_revision=1)
        with mock.patch.object(he,'_screen',side_effect=AssertionError('history called current screen')), \
             mock.patch.object(he,'_source_path',side_effect=AssertionError('history called current path admission')):
            fresh=self.reopen();self.assertEqual(self.prepared,fresh.current())
            fresh.diagnostics();fresh.attest_terminal(ticket,'definite_no_write',profile=hj.QUIESCENT)
            challenge=fresh.challenge(PUBLIC['nonces'][2])
            observations=tuple(hj.observe_document(binding,404,b'',eof=True) for binding in challenge.expected)
            self.assertEqual('missing_source_images',fresh.evaluate(challenge,observations,profile=hj.VISIBLE)['state'])

    def test_body_read_checks_only_selected_frozen_draft(self):
        with mock.patch.object(he,'_screen',wraps=he._screen) as text, \
             mock.patch.object(he,'_source_path',wraps=he._source_path) as paths:
            body=self.journal.request_body(self.prepared.intent_ids[1])
        self.assertEqual(PUBLIC['oracle'][0]['documents'][1]['body'].encode('utf-8'),body)
        self.assertEqual(PUBLIC['boundary_oracles']['selected_screen_views'],[x.args[0] for x in text.call_args_list])
        self.assertEqual([PUBLIC['sources'][1]['path']],[x.args[0] for x in paths.call_args_list])

    def test_reservation_checks_only_selected_frozen_draft(self):
        with mock.patch.object(he,'_screen',wraps=he._screen) as text, \
             mock.patch.object(he,'_source_path',wraps=he._source_path) as paths:
            ticket=self.journal.reserve_attempt(self.prepared.intent_ids[1],PUBLIC['nonces'][0],expected_revision=1)
        self.assertEqual(self.prepared.intent_ids[1],ticket.intent_id)
        self.assertEqual(PUBLIC['boundary_oracles']['selected_screen_views'],[x.args[0] for x in text.call_args_list])
        self.assertEqual([PUBLIC['sources'][1]['path']],[x.args[0] for x in paths.call_args_list])

    def test_replacement_preserves_new_whole_oracle_and_historical_previous_paths(self):
        replacement=self.export(PUBLIC['replacement'])
        with self.path_drift():
            prepared=self.journal.prepare(replacement,PUBLIC['generations'][1],1,previous_owned=self.selection.catalogue)
            fresh=self.reopen();self.assertEqual(prepared,fresh.current())
            challenge=fresh.challenge(PUBLIC['nonces'][2])
        expected=PUBLIC['oracle'][1]
        self.assertEqual(expected['packet'].encode('utf-8'),replacement.serialized)
        self.assertEqual(expected['manifest'].encode('utf-8'),prepared.manifest)
        self.assertEqual(expected['manifest_sha256'],hashlib.sha256(prepared.manifest).hexdigest())
        self.assertEqual(3,len(challenge.expected))

    def test_allowed_sibling_dispatch_ignores_newly_denied_unrelated_history(self):
        with self.content_drift(),self.path_drift():
            body=self.journal.request_body(self.prepared.intent_ids[1])
            ticket=self.journal.reserve_attempt(self.prepared.intent_ids[1],PUBLIC['nonces'][0],expected_revision=1)
            before=self.snapshot()
            self.denied('input',self.journal.reserve_attempt,self.prepared.intent_ids[0],PUBLIC['nonces'][1],expected_revision=1)
            self.assertEqual(before,self.snapshot())
        self.assertEqual(PUBLIC['oracle'][0]['documents'][1]['body'].encode('utf-8'),body)
        self.assertEqual(self.prepared.intent_ids[1],ticket.intent_id)

    def test_replacement_reconciliation_plan_preserves_historical_paths(self):
        replacement=self.export(PUBLIC['replacement'])
        with self.path_drift():
            plan=he.plan_public_ingestion(replacement,previous_owned=self.selection.catalogue)
            plan.__post_init__()
        oracle=PUBLIC['boundary_oracles']['replacement_plan']
        self.assertEqual(oracle['retire_document_ids'],[x.document_id for x in plan.retire_candidates])
        self.assertEqual(oracle['reconciliation_document_ids'],[x.document_id for x in plan.reconciliation])
        self.assertEqual(0,plan.diagnostics['authorized_mutations'])
        self.assertEqual(0,plan.diagnostics['executed_transport_calls'])

    def test_new_prepare_and_public_constructor_still_screen_forged_current_objects(self):
        before=self.snapshot();draft=self.selection.retain_drafts[0]
        with self.content_drift():
            with self.assertRaises(he.ExportError):he.RetainDraft(draft.binding,draft.source_id,draft.body)
            self.denied('input',self.journal.prepare,self.selection,PUBLIC['generations'][1],1)
        object.__setattr__(draft,'body',draft.body+b' ')
        self.denied('input',self.journal.prepare,self.selection,PUBLIC['generations'][1],1)
        self.assertEqual(before,self.snapshot())

    def test_public_text_refusal_preserves_original_fixed_error_contract(self):
        draft=self.selection.retain_drafts[0]
        with self.content_drift():
            with self.assertRaises(he.ExportError) as error:
                he.RetainDraft(draft.binding,draft.source_id,draft.body)
        self.assertEqual(PUBLIC['current_boundary_oracles']['public_text_rejection_reason'],error.exception.reason)

    def test_historical_factory_cannot_bypass_public_export_or_prepare(self):
        original=self.selection.retain_drafts[0];before=self.snapshot()
        with self.content_drift():
            historical=he._historical_draft(original.binding,original.source_id,original.body)
            drafts=(historical,self.selection.retain_drafts[1])
            with self.assertRaises(he.ExportError):
                he.PublicExport(self.selection.catalogue,drafts,self.selection.serialized,self.selection._io)
            forged=object.__new__(he.PublicExport)
            for name,value in (('catalogue',self.selection.catalogue),('retain_drafts',drafts),
                               ('serialized',self.selection.serialized),('_io',self.selection._io)):
                object.__setattr__(forged,name,value)
            self.denied('input',self.journal.prepare,forged,PUBLIC['generations'][1],1)
        self.assertEqual(before,self.snapshot())

    def test_rehashed_changed_body_cannot_hide_source_image_corruption(self):
        identity=self.prepared.intent_ids[0]
        raw=self.db.execute('SELECT body FROM intents WHERE identity=?',(identity,)).fetchone()[0]
        image=json.loads(raw);image['items'][0]['content']='Sandpiper foreign replacement.\n';changed=encoded(image)
        self.db.execute('UPDATE intents SET body=?,digest=? WHERE identity=?',(changed,hashlib.sha256(changed).hexdigest(),identity));self.db.commit()
        with self.content_drift():self.denied('corrupt',self.reopen)

    def test_rehashed_binding_cannot_hide_document_identity_corruption(self):
        identity=self.prepared.intent_ids[0]
        raw=self.db.execute('SELECT binding FROM intents WHERE identity=?',(identity,)).fetchone()[0]
        image=json.loads(raw);image['document_id']='source-'+'1'*64
        self.db.execute('UPDATE intents SET binding=? WHERE identity=?',(encoded(image),identity));self.db.commit()
        self.denied('corrupt',self.reopen)

    def test_rehashed_manifest_cannot_hide_packet_correspondence_corruption(self):
        image=json.loads(self.prepared.manifest);image['export_sha256']='2'*64;raw=encoded(image)
        self.db.execute('UPDATE generations SET manifest=?,digest=? WHERE revision=1',(raw,hashlib.sha256(raw).hexdigest()));self.db.commit()
        self.denied('corrupt',self.reopen)

    def test_rehashed_noncanonical_request_cannot_hide_duplicate_fields(self):
        identity=self.prepared.intent_ids[0]
        raw=self.db.execute('SELECT body FROM intents WHERE identity=?',(identity,)).fetchone()[0]
        changed=b'{"async":false,'+raw[1:]
        self.db.execute('UPDATE intents SET body=?,digest=? WHERE identity=?',(changed,hashlib.sha256(changed).hexdigest(),identity));self.db.commit()
        self.denied('corrupt',self.reopen)

    def test_history_only_admission_preserves_fixed_profiles_and_existing_ids(self):
        with self.content_drift():
            fresh=self.reopen();current=fresh.current()
        expected=PUBLIC['oracle'][0]
        self.assertEqual(expected['manifest'].encode('utf-8'),current.manifest)
        self.assertEqual(tuple(x['descriptor']['intent_id'] for x in expected['documents']),current.intent_ids)
        self.assertEqual(PUBLIC['scope_key'],self.scope.key)
        self.assertEqual('hindsight-ingestion-journal-v1',hj.PROFILE)
        self.assertEqual('hindsight-public-export-v1',he.PROFILE)


if __name__=='__main__':unittest.main()
