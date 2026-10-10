"""Offline selected-chat merge lifecycle and privacy regressions."""
import json
import os
import queue
import sqlite3
from pathlib import Path
import sys
import tempfile
import unittest
import uuid
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import chat_merge, sessions


class HarnessMergeTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup)
        self.root = Path(temp.name); self.project = self.root / 'project'; self.project.mkdir()
        self.other = self.root / 'other'; self.other.mkdir()
        for name, value in [('discover_providers', [{'id':host,'available':True,'executable':'/never-run'} for host in ('codex','claude','cursor')]), ('model_options', {'models':[], 'efforts':[]})]:
            mock = patch.object(sessions.providers, name, return_value=value); mock.start(); self.addCleanup(mock.stop)
        mock = patch.object(sessions.Sessions, '_worker', return_value=None); mock.start(); self.addCleanup(mock.stop)
        self.store = sessions.Sessions(self.root / 'state', [self.project,self.other]); self.addCleanup(lambda: self.store.close())
        self.project_id = next(iter(self.store.projects))

    def source(self, text, provider='codex', **options):
        row = self.store.create({'project_id':self.project_id,'provider':provider,'prompt':text,**options})
        self.store._status(row['id'],'completed')
        self.store._event(row['id'],{'kind':'text','text':'Decision: '+text+'; progress verified.'})
        return row['id']

    def request(self, sources, provider='codex'):
        return {'source_ids':sources,'request_id':str(uuid.uuid4()),'destination':{
            'project_id':self.project_id,'provider':provider,'prompt':'Reconcile sources and continue','mode':'plan'}}

    def test_all_providers_get_fresh_task_and_immutable_attributed_history(self):
        a,b = self.source('Use SQL'),self.source('Use files','claude')
        self.store._event(a,{'kind':'tool','text':'PRIVATE_TOOL_PAYLOAD'})
        self.store._event(a,{'kind':'result','ok':False,'text':'PRIVATE_ERROR_PAYLOAD'})
        self.store._event(a,{'kind':'user','text':'Later requirement','attachments':[{'path':'PRIVATE_ATTACHMENT'}]})
        for provider in ('codex','claude','cursor'):
            row=self.store.merge(self.request([a,b],provider))
            self.assertIsNone(row['native_session_id']); self.assertEqual(provider,row['provider'])
            self.assertEqual([a,b],[source['id'] for source in row['merge']['sources']])
            self.assertNotIn('messages',json.dumps(row['merge']))
            archive=self.store.merge_archive(row['id']); before=json.dumps(archive)
            self.assertIn('Use SQL',before); self.assertIn('Use files',before)
            self.assertNotIn('PRIVATE_',before)
            self.store._event(a,{'kind':'text','text':'Changed after merge'})
            self.assertEqual(before,json.dumps(self.store.merge_archive(row['id'])))
            prompt=self.store._prompt(row,'Current instruction')
            self.assertIn('Use SQL',prompt); self.assertIn('Use files',prompt)
            self.assertIn('conflicts are not evaluated',prompt)
            self.assertIn('Current instruction',prompt)
            self.assertNotIn('Merged chat context',self.store._prompt({**row,'native_session_id':'fresh-native'},'Followup'))
            path=chat_merge.archive_path(self.store,row['id'])
            self.assertEqual(0o600,path.stat().st_mode & 0o777)
            self.assertEqual(archive,json.loads(path.read_text()))

    def test_duplicate_requests_create_only_one_task_even_concurrently(self):
        request=self.request([self.source('A'),self.source('B')]); before=len(self.store.list())
        with ThreadPoolExecutor(max_workers=3) as pool:
            rows=list(pool.map(lambda _:self.store.merge(request),range(3)))
        self.assertEqual(1,len({row['id'] for row in rows})); self.assertEqual(before+1,len(self.store.list()))
        request['destination']['prompt']='Changed intent'
        with self.assertRaises(sessions.SessionError): self.store.merge(request)

    def test_invalid_selection_does_not_create_task_or_archive(self):
        a,b=self.source('A'),self.source('B')
        for ids in ([a],[a,a],[a,str(uuid.uuid4())],[a,{}],[a,'../../escape']):
            with self.subTest(ids=ids), self.assertRaises(sessions.SessionError): self.store.merge(self.request(ids))
        foreign=self.source('Foreign',project_id=list(self.store.projects)[1])
        with self.assertRaises(sessions.SessionError): self.store.merge(self.request([a,foreign]))
        self.store._status(b,'running')
        with self.assertRaises(sessions.SessionError): self.store.merge(self.request([a,b]))
        self.assertFalse((self.store.state_dir/'merges').exists())
        self.assertEqual(3,len(self.store.list()))

    def test_persistence_failure_rolls_back_target_and_retry_is_possible(self):
        request=self.request([self.source('A'),self.source('B')]); before=len(self.store.list())
        with patch.object(chat_merge,'persist',side_effect=OSError('disk full')), self.assertRaises(OSError): self.store.merge(request)
        self.assertEqual(before,len(self.store.list()))
        self.assertEqual(0,self.store.db.execute('SELECT count(*) FROM session_merges').fetchone()[0])
        row=self.store.merge(request); self.assertEqual(2,len(row['merge']['sources']))

    def test_initial_event_failure_rolls_back_binding_and_archive(self):
        request=self.request([self.source('A'),self.source('B')])
        self.store.db.execute("CREATE TRIGGER reject_event BEFORE INSERT ON events BEGIN SELECT raise(ABORT,'fixture failure'); END")
        with self.assertRaises(sqlite3.IntegrityError): self.store.merge(request)
        self.assertEqual(2,len(self.store.list()))
        self.assertEqual([],list((self.store.state_dir/'merges').glob('*/context.json')))
        self.store.db.execute('DROP TRIGGER reject_event')
        self.assertIsNotNone(self.store.merge(request)['merge'])

    def test_queue_failure_can_retry_without_duplicate_target(self):
        request=self.request([self.source('A'),self.source('B')])
        with patch.object(self.store.jobs,'put_nowait',side_effect=queue.Full), self.assertRaises(sessions.SessionError):
            self.store.merge(request)
        row=self.store.list()[0]; self.assertEqual('interrupted',row['status'])
        retried=self.store.merge(request)
        self.assertEqual(row['id'],retried['id']); self.assertEqual('queued',retried['status'])
        self.assertEqual(3,len(self.store.list()))

    def test_restart_after_server_restart_preserves_frozen_context(self):
        request=self.request([self.source('A'),self.source('B')]); row=self.store.merge(request)
        archive=self.store.merge_archive(row['id']); self.store.close()
        restored=self.store=sessions.Sessions(self.root/'state',[self.project,self.other])
        self.assertEqual('interrupted',restored.get(row['id'])['status'])
        resumed=restored.restart_merge(row['id'])
        self.assertEqual('queued',resumed['status']); self.assertIsNone(resumed['native_session_id'])
        self.assertEqual(archive,restored.merge_archive(row['id']))

    def test_duplicate_request_never_automatically_replays_an_attempted_launch(self):
        request=self.request([self.source('A'),self.source('B')]); row=self.store.merge(request)
        self.store.results.start_launch(row['id'],uuid.uuid4().hex)
        self.store._status(row['id'],'interrupted')
        queued=self.store.jobs.qsize()
        retried=self.store.merge(request)
        self.assertEqual(row['id'],retried['id']); self.assertEqual('interrupted',retried['status'])
        self.assertEqual(queued,self.store.jobs.qsize())
        self.assertEqual('queued',self.store.restart_merge(row['id'])['status'])
        self.assertEqual(queued+1,self.store.jobs.qsize())

    def test_startup_removes_only_unreferenced_owned_archives(self):
        row=self.store.merge(self.request([self.source('A'),self.source('B')]))
        folder=self.store.state_dir/'merges'
        orphan=folder/str(uuid.uuid4()); orphan.mkdir(); (orphan/'context.json').write_text('private history')
        foreign=folder/'manual-backup'; foreign.mkdir(); (foreign/'context.json').write_text('keep')
        linked=folder/str(uuid.uuid4()); linked.symlink_to(foreign,target_is_directory=True)
        self.store.close(); self.store=sessions.Sessions(self.root/'state',[self.project,self.other])
        self.assertFalse(orphan.exists()); self.assertTrue(linked.is_symlink())
        self.assertEqual('keep',(foreign/'context.json').read_text())
        self.assertTrue(chat_merge.archive_path(self.store,row['id']).is_file())

    def test_storage_quota_preserves_saved_context_and_modified_archive_fails_closed(self):
        request=self.request([self.source('A'),self.source('B')]); row=self.store.merge(request)
        for limit in ('MAX_ARCHIVES','MAX_STORED_BYTES'):
            with patch.object(chat_merge,limit,1), self.assertRaises(sessions.SessionError):
                self.store.merge(self.request(request['source_ids']))
        self.assertEqual(3,len(self.store.list()))
        chat_merge.archive_path(self.store,row['id']).write_text('{}')
        with self.assertRaises(sessions.SessionError): self.store._prompt(row,'Continue')

    def test_result_duplicate_filtered_and_nested_merge_keeps_inherited_context(self):
        a,b=self.source('A'),self.source('B')
        self.store._event(a,{'kind':'result','ok':True,'text':'Decision: A; progress verified.'})
        first=self.store.merge(self.request([a,b])); self.store._status(first['id'],'completed')
        self.assertEqual(2,len(self.store.merge_archive(first['id'])['sources'][0]['messages']))
        nested=self.store.merge(self.request([first['id'],b]))
        inherited=self.store.merge_archive(nested['id'])['sources'][0]['inherited_context']
        self.assertEqual([a,b],[source['id'] for source in inherited['sources']])

    def test_limits_reject_without_silent_partial_sources(self):
        a,b=self.source('A'*1000),self.source('B'*1000)
        for limit in ('MAX_SOURCE_BYTES','MAX_BUNDLE_BYTES','MAX_EVENTS'):
            with patch.object(chat_merge,limit,1), self.assertRaises(sessions.SessionError): self.store.merge(self.request([a,b]))
        self.assertEqual(2,len(self.store.list()))

    def test_large_unicode_preview_is_bounded_and_archive_keeps_middle(self):
        ids=[self.source('😀'*4000+'MIDDLE-'+str(n)+'漢'*4000) for n in range(8)]
        row=self.store.merge(self.request(ids)); archive=self.store.merge_archive(row['id'])
        prompt=self.store.db.execute('SELECT context FROM session_merges WHERE session_id=?',(row['id'],)).fetchone()[0]
        self.assertLessEqual(len(prompt.encode()),chat_merge.PREVIEW_BYTES)
        for sid in ids: self.assertIn(sid,prompt)
        for n in range(8): self.assertIn('MIDDLE-'+str(n),json.dumps(archive,ensure_ascii=False))

    def test_explicit_merge_disallows_side_effecting_workspace_before_creation(self):
        request=self.request([self.source('A'),self.source('B')]); request['destination']['workspace']='worktree'
        with patch.object(self.store,'_new_workspace') as workspace, self.assertRaises(sessions.SessionError): self.store.merge(request)
        workspace.assert_not_called()


    def test_merge_keeps_automatic_memory_and_the_first_message_as_any_new_session_has_them(self):
        request=self.request([self.source('A'),self.source('B')])
        request['destination']['brain']={'bank':'memory-bank','auto':True}
        row=self.store.merge(request)
        self.assertEqual(2,len(row['merge']['sources'])); self.assertTrue(row['brain']['task_id'])
        first=json.loads(self.store.db.execute('SELECT data FROM events WHERE session_id=? ORDER BY id LIMIT 1',(row['id'],)).fetchone()[0])
        self.assertEqual(('user','Reconcile sources and continue'),(first['kind'],first['text'])); self.assertIn('at',first)

    def test_visible_history_leaves_out_memory_drafts_recovery_replies_and_special_runs(self):
        a,b=self.source('A'),self.source('B')
        self.store._event(a,{'kind':'text','text':'Done.\n\n```memory-draft\n{"progress": "PRIVATE_DRAFT"}\n```'})
        self.store._event(a,{'kind':'text','text':'PRIVATE_RECOVERY','memory_recovery':True})
        archive=json.dumps(self.store.merge_archive(self.store.merge(self.request([a,b]))['id']),ensure_ascii=False)
        self.assertIn('Done.',archive); self.assertIn(chat_merge.DRAFT_NOTE,archive); self.assertNotIn('PRIVATE_',archive)
        for column in ('system_run','system_discovery','creator','clash'):
            special=self.source('Special '+column)
            self.store.db.execute(f'UPDATE sessions SET {column}=? WHERE id=?',(json.dumps({'run_id':'0'*32}),special)); self.store.db.commit()
            with self.subTest(column=column), self.assertRaises(sessions.SessionError): self.store.merge(self.request([a,special]))

    def test_claude_command_prompt_and_held_runs_are_refused_before_anything_is_saved(self):
        a,b=self.source('A'),self.source('B'); before=len(self.store.list())
        request=self.request([a,b],'claude'); request['destination']['prompt']='/review the merged work'
        with self.assertRaises(sessions.SessionError): self.store.merge(request)
        row=self.store.merge(self.request([a,b])); self.store._status(row['id'],'interrupted')
        self.assertEqual(0,self.store.hold_runs('Updating the Harness.'))
        with self.assertRaises(sessions.SessionError): self.store.merge(self.request([a,b]))
        with self.assertRaises(sessions.SessionError): self.store.restart_merge(row['id'])
        self.assertEqual('interrupted',self.store.get(row['id'])['status'])
        self.store.release_runs(); self.assertEqual('queued',self.store.restart_merge(row['id'])['status'])
        self.assertEqual(before+1,len(self.store.list()))

if __name__=='__main__': unittest.main()
