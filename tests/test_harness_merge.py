"""Offline selected-chat merge lifecycle and privacy regressions."""
import json
import os
import queue
import re
import shutil
import sqlite3
import subprocess
from pathlib import Path
import sys
import tempfile
import unittest
import uuid
from concurrent.futures import ThreadPoolExecutor
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import chat_merge, sessions

WEB = Path(__file__).resolve().parents[1] / 'harness/web'
# The composer's prefilled instruction for a merged task, as the page sends it unchanged.
MERGE_PROMPT = re.search(r"const MERGE_PROMPT = '([^']*)';", (WEB / 'app-core.js').read_text(encoding='utf-8')).group(1)


def ui_script():
    """The page's scripts in load order as one text; the Node checks slice functions out of it."""
    page = (WEB / 'index.html').read_text(encoding='utf-8')
    return '\n'.join((WEB / name).read_text(encoding='utf-8') for name in re.findall(r'<script src="/([\w.-]+\.js)"></script>', page))


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

    def test_deleting_a_saved_context_frees_merge_storage_and_keeps_the_task(self):
        a,b=self.source('A'),self.source('B'); first=self.store.merge(self.request([a,b]))
        folder=chat_merge.archive_path(self.store,first['id']).parent
        with patch.object(chat_merge,'MAX_ARCHIVES',1):
            with self.assertRaisesRegex(sessions.SessionError,'Delete the saved context'): self.store.merge(self.request([a,b]))
            # A queued or running task may still read its archive.
            with self.assertRaises(sessions.SessionError): self.store.delete_merge_archive(first['id'])
            self.store._status(first['id'],'failed')
            deleted=self.store.delete_merge_archive(first['id'])
            self.assertTrue(deleted['merge']['archive_deleted_at']); self.assertEqual([a,b],[s['id'] for s in deleted['merge']['sources']])
            self.assertFalse(folder.exists())
            with self.assertRaises(sessions.SessionError): self.store.merge_archive(first['id'])
            # Without its saved copy the task can neither restart nor launch from the chats.
            with self.assertRaises(sessions.SessionError): self.store.restart_merge(first['id'])
            with self.assertRaises(sessions.SessionError): self.store._prompt(deleted,'Continue')
            second=self.store.merge(self.request([a,b]))
        self.assertEqual(2,len(self.store.merge_archive(second['id'])['sources']))
        nested=self.store.merge_archive(self.store.merge(self.request([first['id'],b]))['id'])['sources'][0]
        self.assertTrue(nested['inherited_context_deleted']); self.assertNotIn('inherited_context',nested)
        # A folder a failed removal left behind goes at the next start; the task keeps its sources.
        folder.mkdir(); (folder/'context.json').write_text('stale')
        self.store.close(); self.store=sessions.Sessions(self.root/'state',[self.project,self.other])
        self.assertFalse(folder.exists()); self.assertTrue(self.store.get(first['id'])['merge']['archive_deleted_at'])

    def test_restart_rewrites_a_missing_or_changed_archive_from_its_record(self):
        row=self.store.merge(self.request([self.source('A'),self.source('B')])); path=chat_merge.archive_path(self.store,row['id'])
        saved=path.read_bytes()
        for damage in (lambda: path.unlink(), lambda: path.write_bytes(saved+b' '), lambda: path.parent.rename(path.parent.with_name('gone'))):
            damage(); self.store._status(row['id'],'failed')
            with self.assertRaises(sessions.SessionError): self.store._prompt(self.store.get(row['id']),'Continue')
            self.assertEqual('queued',self.store.restart_merge(row['id'])['status'])
            self.assertEqual(saved,path.read_bytes()); self.assertEqual(0o600,path.stat().st_mode & 0o777)
            self.assertIn('Merged chat context',self.store._prompt(self.store.get(row['id']),'Continue'))
        self.assertIn('rewritten from',json.dumps(self.store.events(row['id'])))
        # A link in the archive's place is never followed or written through.
        outside=self.root/'outside'; outside.mkdir(); (outside/'context.json').write_text('keep')
        shutil.rmtree(path.parent); path.parent.symlink_to(outside,target_is_directory=True); self.store._status(row['id'],'failed')
        with self.assertRaises(sessions.SessionError): self.store.restart_merge(row['id'])
        self.assertEqual('keep',(outside/'context.json').read_text()); self.assertEqual('failed',self.store.get(row['id'])['status'])

    def test_merged_task_memory_names_the_chats_not_only_the_prefilled_instruction(self):
        a,b=self.source('Design the payment retry queue'),self.source('Webhook idempotency keys')
        request=self.request([a,b]); request['destination'].update(prompt=MERGE_PROMPT,brain={'bank':'memory-bank','auto':True})
        row=self.store.merge(request); brain=row['brain']
        for text in (brain['goal'],brain['query']):
            self.assertIn('payment retry',text); self.assertIn('Webhook idempotency',text)
        self.assertTrue(brain['task_id'].startswith('harness/merged-design-the-payment'),brain['task_id'])
        # The first message stays as written; only memory reads the chats' subjects with it.
        first=json.loads(self.store.db.execute('SELECT data FROM events WHERE session_id=? ORDER BY id LIMIT 1',(row['id'],)).fetchone()[0])
        self.assertEqual(MERGE_PROMPT,first['text'])
        # A merged chat is named by the chats it merged, not by its own prefilled instruction.
        self.store._status(row['id'],'completed'); request=self.request([row['id'],self.source('Order export CSV')])
        request['destination'].update(prompt=MERGE_PROMPT,brain={'bank':'memory-bank','auto':True})
        goal=self.store.merge(request)['brain']['goal']
        self.assertTrue(goal.startswith('Merged “Design the payment retry queue”, “Webhook idempotency keys”, “Order export CSV”: '),goal)

    @unittest.skipUnless(shutil.which('node'),'Merge picker check requires Node')
    def test_merge_picker_leaves_out_system_runs_it_reads_from_session_summaries(self):
        from harness import web
        ordinary=self.source('Ordinary chat')
        system=self.store.create({'project_id':self.project_id,'provider':'codex','prompt':'System change','workflow':'native'},_system_run={'run_id':'a'*32,'nonce':'b'*32})
        scan=self.store.create({'project_id':self.project_id,'provider':'codex','prompt':'Discover services','workflow':'native','mode':'plan'},_system_discovery={'run_id':'c'*32,'nonce':'d'*32})
        for row in (system,scan): self.store._status(row['id'],'completed')
        page=ui_script(); start=page.index('\nconst isFleetSession'); eligible=page.index('\nconst mergeSourceEligible')
        source=page[start:page.index('\n',start+1)]+page[eligible:page.index('\nconst mergeRestartable',eligible)]
        script="const assert = require('node:assert/strict');\n"+source+"""
const listed = JSON.parse(process.argv[1]).filter(session => mergeSourceEligible(session,process.argv[2])).map(session => session.title);
assert.deepEqual(listed,['Ordinary chat']);
"""
        subprocess.run([shutil.which('node'),'-e',script,json.dumps(self.store.summaries()),self.project_id],check=True,capture_output=True,text=True)

    @unittest.skipUnless(shutil.which('node'),'Session preference check requires Node')
    def test_leaving_a_merge_draft_keeps_the_projects_saved_new_session_preferences(self):
        page=ui_script()
        def cut(start,end): return page[page.index(start):page.index(end,page.index(start))]
        source=(cut('\nfunction readSessionPreferences(','\nfunction captureSessionPreferences(')
                +cut('\nfunction saveSessionPreferences(','\nasync function restoreProjectPreferences(')
                +cut('\nfunction discardMergeDraft(','\nfunction closeMergeArchive(')
                +cut('\nasync function selectSession(',"\n$('session-form')"))
        script="""const assert = require('node:assert/strict');
const store = {}; const localStorage = {getItem: key => store[key] ?? null, setItem: (key, value) => { store[key] = value; }};
const sessionPreferencesKey = 'prefs'; let sessionPreferencesReady = true, restoringSessionPreferences = false, sessionDraftProject = null, preferenceEpoch = 0;
const element = () => ({value:'', setAttribute(){}, classList:{remove(){}}, style:{removeProperty(){}}, replaceChildren(){}});
const fields = {project:{...element(), value:'p1'}, workflow:{...element(), value:'review'}}; const $ = id => fields[id] || (fields[id] = element());
const state = {bootstrap:{}, pending:null, selectedId:null, selected:null, sessions:[], epoch:0, eventIds:new Set(), assistantTexts:new Set(), stepCalls:new Set()};
const mergeUi = {draft:null, sourcesKey:null}; const sessionOptions = {};
const brainLinkStrict = () => false, brainLinkDraft = {}, projectFor = id => id === 'p1';
const captureSessionPreferences = () => ({fields:{workflow:fields.workflow.value}});
const noop = () => {}; const closeMergeArchive = noop, clearAttachments = noop, stopPolling = noop, resetFleetProgress = noop, showError = noop,
  applySessionSettings = noop, renderHistory = noop, setView = noop, updateControls = noop, pollSession = async () => {};
const runView = {reset: noop};
""" + source + """
(async () => {
  saveSessionPreferences();
  // A merge draft forces a native task in the project folder; selecting the merged task must not keep that as the default.
  mergeUi.draft = {projectId:'p1'}; fields.workflow.value = 'native';
  await selectSession('merged-task');
  assert.equal(JSON.parse(store.prefs).drafts.p1.fields.workflow,'review');
  assert.equal(mergeUi.draft,null);
})().catch(error => { console.error(error); process.exit(1); });
"""
        subprocess.run([shutil.which('node'),'-e',script],check=True,capture_output=True,text=True)

    @unittest.skipUnless(shutil.which('node'),'Merge panel check requires Node')
    def test_merge_panel_asks_before_deleting_a_saved_context_and_says_when_it_is_gone(self):
        page=ui_script()
        def cut(start,end): return page[page.index(start):page.index(end,page.index(start))]
        source=(cut('\nconst active =','\nconst isFleetSession')+cut('\nconst mergeRestartable','\nfunction discardMergeDraft(')
                +cut('\nfunction renderMergeContext(','\nfunction applyMergeControls('))
        script="""const assert = require('node:assert/strict');
const fields = {}; const $ = id => fields[id] || (fields[id] = {hidden:false, disabled:false, textContent:'', replaceChildren(){}, classList:{set:new Set(), toggle(name, on){ on ? this.set.add(name) : this.set.delete(name); }}});
const el = (tag, className, text) => ({tag, className, textContent:text, addEventListener(){}}); const providerFor = () => null;
const state = {pending:null, bootstrap:{}, authFailed:false, selectedId:'t1', selected:null};
const mergeUi = {draft:null, sourcesKey:null, deleteArmed:null};
""" + source + """
const merge = {sources:[{id:'a', title:'A'}, {id:'b', title:'B'}]};
state.selected = {id:'t1', status:'failed', native_session_id:null, merge}; renderMergeContext();
assert.equal($('merge-restart').hidden,false); assert.equal($('merge-delete').hidden,false); assert.equal($('merge-delete-keep').hidden,true);
// The first click only asks: the note says what goes and what stays, and Keep it backs out.
mergeUi.deleteArmed = 't1'; renderMergeContext();
assert.match($('merge-context-note').textContent,/cannot be undone/); assert.equal($('merge-delete').textContent,'Delete saved copy');
assert.equal($('merge-delete-keep').hidden,false); assert.ok($('merge-delete').classList.set.has('danger')); assert.equal($('merge-archive').hidden,true);
// A run may still read it: a queued task offers no deletion and drops the question.
state.selected = {...state.selected, status:'queued'}; renderMergeContext();
assert.equal($('merge-delete').hidden,true); assert.equal(mergeUi.deleteArmed,null);
// Once deleted, the task keeps its sources but offers neither the copy nor a restart from it.
state.selected = {id:'t1', status:'failed', native_session_id:null, merge:{...merge, archive_deleted_at:'2026-10-10T00:00:00+00:00'}}; renderMergeContext();
assert.match($('merge-context-note').textContent,/was deleted/);
assert.deepEqual(['merge-restart','merge-archive','merge-delete'].map(id => $(id).hidden),[true,true,true]);
"""
        subprocess.run([shutil.which('node'),'-e',script],check=True,capture_output=True,text=True)

if __name__=='__main__': unittest.main()
