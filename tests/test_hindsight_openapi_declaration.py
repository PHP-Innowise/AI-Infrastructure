"""New Loon OpenAPI raw-byte and offline full-pipeline correspondence controls."""
from __future__ import annotations
import base64
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest import mock

ROOT=Path(__file__).resolve().parents[1]
PUBLIC=json.loads((ROOT/'tests/fixtures/hindsight_openapi_public.json').read_text(encoding='utf-8'))
sys.path.insert(0,str(ROOT/'harness/src'))
from harness import hindsight_export as export
from harness import hindsight_journal as journal
from harness.hindsight_recall import Scope,SourceBinding,Catalogue
CASES={x['name']:x for x in PUBLIC['cases']}
POSITIVES=[x for x in PUBLIC['cases'] if x['accepted']]
NEGATIVES=[x for x in PUBLIC['cases'] if not x['accepted']]
def image(case,key):return base64.b64decode(case[key+'_base64'])
def scope():return Scope(PUBLIC['team_id'],PUBLIC['project_id'])


def supplied(case):
    binding=SourceBinding(scope(),case['document_id'],PUBLIC['source_path'],case['raw_sha256'])
    draft=export._historical_draft(binding,case['source_id'],image(case,'body'))
    packet=object.__new__(export.PublicExport)
    fields={'catalogue':Catalogue(scope(),(binding,)),'retain_drafts':(draft,),
            'serialized':image(case,'packet'),'_io':(('source_read_bytes',case['raw_bytes']),
            ('source_read_calls',(case['raw_bytes']+8191)//8192+1),('opened_descriptors',2),
            ('closed_descriptors',2),('close_errors',0))}
    for key,value in fields.items():object.__setattr__(packet,key,value)
    return packet


def new_journal(owner):
    temporary=tempfile.TemporaryDirectory(prefix='loon-journal-');owner.addCleanup(temporary.cleanup)
    connection=sqlite3.connect(str(Path(temporary.name)/'owned.sqlite'),isolation_level=None,timeout=0.1)
    owner.addCleanup(connection.close)
    connection.execute('PRAGMA journal_mode=DELETE');connection.execute('PRAGMA synchronous=EXTRA')
    connection.execute('PRAGMA busy_timeout=100');connection.execute('PRAGMA foreign_keys=ON')
    return connection,journal.create_new(connection,scope(),PUBLIC['journal_id'])


class DeclarationScreenTests(unittest.TestCase):
    def admit(self,name):
        case=CASES[name];raw=image(case,'raw')
        self.assertEqual(raw.decode('utf-8'),export._text(raw))
    def refuse(self,names):
        for name in names:
            with self.subTest(name=name):
                case=CASES[name]
                with self.assertRaises(export.ExportError) as denied:export._text(image(case,'raw'))
                self.assertEqual(case['capture_error'],denied.exception.reason)

    def test_plain_original_json_declaration(self):self.admit('plain')
    def test_documentation_only_jwt_format(self):self.admit('jwt')
    def test_scheme_name_case_is_bound_to_its_actual_node(self):self.admit('upper_name')
    def test_crlf_unicode_prefix_and_escaped_key_keep_exact_raw_bytes(self):self.admit('crlf_escaped_name')
    def test_equal_objects_outside_the_approved_path_are_refused(self):
        self.refuse(['identical_outside_after','identical_outside_before'])
    def test_other_composer_maps_and_embedded_documents_have_no_exemption(self):
        self.refuse(['composer_elsewhere','encoded_composer_elsewhere','embedded_openapi_string'])
    def test_wrong_context_and_unknown_scheme_fields_are_refused(self):
        self.refuse(['unsupported_version','components_array','extra_description','extra_ref',
                     'extra_extension','extra_token','wrong_type','wrong_scheme','unknown_format','bool_format'])
    def test_duplicate_escaped_alias_nonfinite_and_trailing_input_are_refused(self):
        self.refuse(['duplicate_root','escaped_duplicate_leaf','nonfinite','trailing_json'])
    def test_profile_depth_structure_and_byte_limits_give_no_exemption(self):
        self.refuse(['depth_bound','structural_bound','source_byte_bound'])
    def test_classification_callback_cancellation_is_not_parse_fallback(self):
        calls=[]
        def active():calls.append(1);return len(calls)<3
        with self.assertRaises(export.ExportError) as denied:
            export._openapi_bearer_spans(image(CASES['jwt'],'raw').decode('utf-8'),active=active)
        self.assertEqual('cancelled',denied.exception.reason);self.assertEqual(3,len(calls))
    def test_classification_callback_failure_remains_fixed_unavailable(self):
        def active():raise RuntimeError('Loon callback failure must not escape')
        with self.assertRaises(export.ExportError) as denied:
            export._openapi_bearer_spans(image(CASES['jwt'],'raw').decode('utf-8'),active=active)
        self.assertEqual('unavailable',denied.exception.reason);self.assertEqual('unavailable',str(denied.exception))


class SuppliedPipelineTests(unittest.TestCase):
    def test_positive_constructor_and_planner_compare_complete_independent_oracles(self):
        for case in POSITIVES:
            with self.subTest(name=case['name']):
                packet=supplied(case);packet.__post_init__();plan=export.plan_public_ingestion(packet)
                self.assertEqual(image(case,'packet'),packet.serialized)
                self.assertEqual(image(case,'body'),packet.retain_drafts[0].body)
                self.assertEqual(case['plan_diagnostics'],plan.diagnostics)
                self.assertEqual((('GET',case['reconciliation_resource']),),plan.reconciliation_drafts)
                self.assertEqual((packet.retain_drafts[0],),plan.retain_candidates)
                self.assertEqual((),plan.retire_candidates)
    def test_journal_manifest_rows_and_returned_body_match_full_oracles(self):
        for case in POSITIVES:
            with self.subTest(name=case['name']):
                db,store=new_journal(self);packet=supplied(case)
                prepared=store.prepare(packet,PUBLIC['generation_id'],0)
                self.assertEqual(1,prepared.revision);self.assertEqual(PUBLIC['generation_id'],prepared.generation)
                self.assertEqual(image(case,'manifest'),prepared.manifest);self.assertEqual((case['intent_id'],),prepared.intent_ids)
                self.assertEqual(image(case,'body'),store.request_body(case['intent_id']))
                self.assertEqual([(1,PUBLIC['generation_id'],image(case,'manifest'),case['manifest_sha256'])],db.execute('SELECT * FROM generations').fetchall())
                self.assertEqual([(case['intent_id'],1,0,image(case,'binding'),case['source_id'],image(case,'body'),case['body_sha256'])],db.execute('SELECT * FROM intents').fetchall())
                self.assertEqual([],db.execute('SELECT * FROM attempts').fetchall())
                self.assertEqual(prepared,store.current())
                diagnostics=store.diagnostics();self.assertEqual(case['persisted_payload_bytes'],diagnostics['persisted_payload_bytes'])
                self.assertEqual(0,diagnostics['executed_transport_calls']);self.assertIsNone(diagnostics['provider_tokens']);self.assertFalse(diagnostics['total_cost_complete'])
    def test_rejected_constructed_packets_plans_and_preparation_leave_rows_unchanged(self):
        db,store=new_journal(self);before=list(db.iterdump())
        for case in NEGATIVES:
            with self.subTest(name=case['name']):
                try:packet=supplied(case)
                except export.ExportError as error:
                    self.assertEqual('approval',error.reason);continue
                with self.assertRaises(export.ExportError) as denied:packet.__post_init__()
                self.assertEqual(case['constructor_error'],denied.exception.reason)
                with self.assertRaises(export.ExportError):export.plan_public_ingestion(packet)
                with self.assertRaises(journal.JournalError) as denied:store.prepare(packet,PUBLIC['generation_id'],0)
                self.assertEqual(case['journal_error'],denied.exception.reason);self.assertEqual(before,list(db.iterdump()))
        self.assertIsNone(store.current());self.assertEqual(before,list(db.iterdump()))
    def test_cancellation_during_current_journal_screen_does_not_publish(self):
        db,store=new_journal(self);before=list(db.iterdump());packet=supplied(CASES['jwt'])
        actual=export._openapi_bearer_spans;entered=[]
        def classification(view,*,active):
            entered.append(True);return actual(view,active=active)
        with mock.patch.object(export,'_openapi_bearer_spans',side_effect=classification):
            with self.assertRaises(journal.JournalError) as denied:
                store.prepare(packet,PUBLIC['generation_id'],0,active=lambda:not entered)
        self.assertEqual('input',denied.exception.reason);self.assertTrue(entered);self.assertEqual(before,list(db.iterdump()))


@unittest.skipUnless(os.name=='posix' and hasattr(os,'O_NOFOLLOW'),'actual protected POSIX export capture required')
class ActualCaptureTests(unittest.TestCase):
    def root(self,case):
        temporary=tempfile.TemporaryDirectory(prefix='loon-openapi-');self.addCleanup(temporary.cleanup)
        root=Path(temporary.name);leaf=root/PUBLIC['source_path'];leaf.parent.mkdir()
        leaf.write_bytes(image(case,'raw'));return root,leaf
    def test_approved_capture_and_serialization_keep_every_original_byte(self):
        for case in POSITIVES:
            with self.subTest(name=case['name']):
                root,leaf=self.root(case);approval=export.PublicApproval(scope(),PUBLIC['source_path'],case['raw_sha256'])
                packet=export.build_public_export(root,scope(),(approval,))
                self.assertEqual(image(case,'packet'),packet.serialized);self.assertEqual(image(case,'body'),packet.retain_drafts[0].body)
                self.assertEqual(image(case,'raw'),leaf.read_bytes());self.assertEqual(case['raw_sha256'],packet.catalogue.sources[0].source_sha256)
                diagnostics=packet.diagnostics;opened=2+len(root.parts[1:])+len(PUBLIC['source_path'].split('/')[:-1])
                self.assertEqual(case['raw_bytes'],diagnostics['source_read_bytes']);self.assertEqual(2,diagnostics['source_read_calls'])
                self.assertEqual(opened,diagnostics['opened_descriptors']);self.assertEqual(opened,diagnostics['closed_descriptors']);self.assertEqual(0,diagnostics['close_errors'])
                self.assertEqual(0,diagnostics['executed_transport_calls']);self.assertIsNone(diagnostics['provider_tokens']);self.assertFalse(diagnostics['total_cost_complete'])
    def test_rejected_whole_capture_returns_fixed_codes_and_closes_owned_handles(self):
        for case in NEGATIVES:
            with self.subTest(name=case['name']):
                root,leaf=self.root(case);approval=export.PublicApproval(scope(),PUBLIC['source_path'],case['raw_sha256'])
                with self.assertRaises(export.ExportError) as denied:export.build_public_export(root,scope(),(approval,))
                self.assertEqual(case['capture_error'],denied.exception.reason)
                observed=denied.exception.observation;self.assertEqual(observed['opened_descriptors'],observed['closed_descriptors']);self.assertEqual(0,observed['close_errors'])
                self.assertEqual(image(case,'raw'),leaf.read_bytes())


if __name__=='__main__':unittest.main()
