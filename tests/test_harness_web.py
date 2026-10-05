"""Local HTTP boundary checks; no native CLI probes or model processes."""

import contextlib
import base64
import hashlib
import http.client
import io
import json
import os
from datetime import datetime, timedelta, timezone
from pathlib import Path, PurePosixPath
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
from urllib.parse import urlencode

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import providers, run_activity, sessions, web

WEB = Path(__file__).resolve().parents[1] / "harness/web"


def ui_script():
    """The page's scripts in load order as one text; the Node checks slice functions out of it."""
    page = (WEB / "index.html").read_text(encoding="utf-8")
    return "\n".join((WEB / name).read_text(encoding="utf-8") for name in re.findall(r'<script src="/([\w.-]+\.js)"></script>', page))


def stylesheet(page):
    """A page's styles: inline, or the files its <link> elements name next to it."""
    html = page.read_text(encoding="utf-8")
    if "<style>" in html:
        return html[html.index("<style>"):html.index("</style>")]
    return "\n".join((page.parent / name).read_text(encoding="utf-8") for name in re.findall(r'<link rel="stylesheet" href="/([\w.-]+\.css)">', html))


class HarnessWebTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / "project"
        self.project.mkdir()
        discovery = patch.object(providers, "discover_providers", return_value=[{
            "id": "codex", "name": "Fixture Codex", "available": False,
            "executable": "/never-launched/fixture-cli", "detail": "Offline fixture",
        }, {
            "id": "claude", "name": "Fixture Claude", "available": True,
            "executable": "/never-launched/fixture-claude", "detail": "Offline fixture",
        }])
        discovery.start()
        self.addCleanup(discovery.stop)
        codex_catalog = {
            "models": [
                {"id": "fixture-model", "label": "Fixture", "efforts": ["low", "high"]},
                {"id": "fixture-small", "label": "Small fixture", "efforts": ["low"]},
            ],
            "efforts": ["low", "high"], "detail": "Offline fixture catalog",
        }
        claude_catalog = {
            "models": [{"id": "fixture-claude", "label": "Claude fixture",
                        "efforts": ["high", "ultracode"]}],
            "efforts": ["high", "ultracode"], "detail": "Offline Claude fixture catalog",
        }
        catalog = patch.object(providers, "model_options", side_effect=lambda provider:
                               claude_catalog if provider == "claude" else codex_catalog)
        catalog.start()
        self.addCleanup(catalog.stop)
        # HTTP tests exercise the persistent queue; process execution has its own suite.
        worker = patch.object(web.Sessions, "_worker", return_value=None)
        worker.start()
        self.addCleanup(worker.stop)
        self.server = web.HarnessServer(("127.0.0.1", 0), self.root / "state", [self.project])
        self.closed = False
        self.thread = threading.Thread(target=self.server.serve_forever,
                                       kwargs={"poll_interval": .01}, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)
        self.project_id = next(iter(self.server.sessions.projects))
        self.token = self.server.token

    def close_server(self):
        if not self.closed:
            self.server.close()
            self.thread.join(2)
            self.closed = True
            self.assertFalse(self.thread.is_alive(), "HTTP server did not stop")

    def request(self, path, method="GET", data=None, raw=None, headers=None):
        body = raw if raw is not None else json.dumps(data).encode() if data is not None else None
        outgoing = {"Host": f"127.0.0.1:{self.server.server_port}"}
        if body is not None:
            outgoing.update({"Content-Type": "application/json", "Content-Length": str(len(body))})
        for name, value in (headers or {}).items():
            if value is None:
                outgoing.pop(name, None)
            else:
                outgoing[name] = value
        connection = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=30 if path.endswith('/delivery') else 3)
        try:
            # Explicit headers allow testing missing Host/Content-Length without client defaults.
            connection.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
            for name, value in outgoing.items():
                connection.putheader(name, value)
            connection.endheaders(body)
            response = connection.getresponse()
            payload = response.read()
            response_headers = dict(response.getheaders())
            if method != "HEAD" and response_headers.get("Content-Type", "").startswith("application/json"):
                payload = json.loads(payload)
            return response.status, payload, response_headers
        finally:
            connection.close()

    def post(self, path, data=None, raw=None, headers=None):
        return self.request(path, "POST", data, raw,
                            {"X-Harness-Token": self.token, **(headers or {})})

    def options(self, **changes):
        return {"project_id": self.project_id, "provider": "codex", "prompt": "Inspect the fixture",
                "mode": "plan", "workflow": "native", "project_context": False, **changes}

    def test_session_attachments_are_validated_bound_to_messages_and_downloaded_as_data(self):
        body = 'Требования <script>example</script>\n'.encode() * 2000
        upload = {'name':'requirements.txt', 'data':base64.b64encode(body).decode()}
        options = self.options(provider='claude', attachments=[upload])
        self.assertEqual(self.post('/api/sessions', options, headers={'X-Harness-Token':None})[0], 403)
        status, result, _ = self.post('/api/sessions', options)
        self.assertEqual(status, 201)
        sid = result['session']['id']; store = self.server.sessions
        event = next(item for item in store.events(sid) if item['kind'] == 'user')
        item = event['attachments'][0]
        self.assertEqual(set(item), {'id','name','size'})
        self.assertEqual(item['size'], len(body))
        self.assertNotIn(upload['data'], json.dumps(event))
        path = f"/api/sessions/{sid}/attachments/{item['id']}"
        status, content, headers = self.request(path)
        self.assertEqual((status, content), (200, body))
        self.assertEqual(headers['Content-Type'], 'application/octet-stream')
        self.assertTrue(headers['Content-Disposition'].startswith('attachment;'))
        self.assertEqual(headers['X-Content-Type-Options'], 'nosniff')
        self.assertEqual(list(self.project.iterdir()), [])
        _, _, local_path = store.attachments.read(sid, item['id'])
        self.assertIn(local_path, store._prompt(result['session'], 'Inspect'))
        store.db.execute("UPDATE sessions SET status='completed',native_session_id='native-fixture' WHERE id=?", (sid,)); store.db.commit()
        second = {'name':'requirements.txt','data':base64.b64encode(b'Follow-up revision').decode()}
        status, _, _ = self.post(f'/api/sessions/{sid}/messages', {'prompt':'Compare the revision', 'attachments':[second]})
        self.assertEqual(status, 200)
        latest = store.attachments.current(sid)
        self.assertEqual(latest[0][1], b'Follow-up revision')
        self.assertNotEqual(latest[0][0]['id'], item['id'])
        self.assertEqual(self.request(path)[1], body)
        _, other, _ = self.post('/api/sessions', self.options(provider='claude'))
        self.assertEqual(self.request(path.replace(sid, other['session']['id']))[0], 400)
        self.assertEqual(self.request(path+'?path=/etc/passwd')[0], 400)
        for invalid in ([{'name':'../escape','data':'YQ=='}], [{'name':'bad\nname','data':'YQ=='}], [{'name':'x','data':'not base64'}], [upload]*6):
            with self.subTest(invalid=invalid[0]['name']):
                before = len(store.list())
                self.assertEqual(self.post('/api/sessions', self.options(provider='claude',attachments=invalid))[0], 400)
                self.assertEqual(len(store.list()), before)
        from harness.attachments import validate
        with self.assertRaises(web.SessionError):
            validate([{'name':'large','data':base64.b64encode(b'x'*(4*1024*1024+1)).decode()}])
        with self.assertRaises(web.SessionError):
            validate([{'name':'x','data':base64.b64encode(b'x'*(3*1024*1024)).decode()}]*3)
        stored = Path(local_path); stored.unlink(); stored.symlink_to(self.project.parent / 'outside.txt')
        (self.project.parent / 'outside.txt').write_bytes(body)
        self.assertNotEqual(self.request(path)[0], 200)

    def test_sdd_api_reads_only_feature_documents_and_routes_phase_changes(self):
        store = self.server.sessions
        with patch.object(store.jobs, 'put_nowait'):
            status, data, _ = self.post('/api/sessions', self.options(
                provider='claude', workflow='sdd', sdd={'phase':'specify','feature':'login'}))
        self.assertLess(status, 300)
        sid = data['session']['id']
        store.db.execute("UPDATE sessions SET status='completed',native_session_id='fixture' WHERE id=?", (sid,))
        store.db.commit()
        folder = self.project / 'specs/login'; folder.mkdir(parents=True)
        (folder / 'spec.md').write_text('# Login\n- [ ] Users can sign in.\n')
        status, data, _ = self.request(f'/api/sessions/{sid}/sdd')
        self.assertEqual(status, 200)
        self.assertEqual(data['files'][1]['text'], '# Login\n- [ ] Users can sign in.\n')
        self.assertEqual(len(data['files']), 5)
        self.assertEqual(self.request(f'/api/sessions/{sid}/sdd?path=README.md')[0], 400)
        self.assertEqual(self.post(f'/api/sessions/{sid}/messages', {'prompt':'Plan',
            'sdd':{'phase':'plan','feature':'login'}}, headers={'X-Harness-Token':None})[0], 403)
        with patch.object(store.jobs, 'put_nowait'):
            status, data, _ = self.post(f'/api/sessions/{sid}/messages', {'prompt':'Plan',
                'sdd':{'phase':'plan','feature':'login'}})
        self.assertEqual(status, 200)
        self.assertEqual(data['session']['sdd']['phase'], 'plan')
        self.assertEqual(data['session']['mode'], 'edit')
        boot = self.request('/api/bootstrap')[1]
        self.assertIn('sdd', [item['id'] for item in boot['workflows']])
        self.assertEqual(len(boot['sdd_phases']), 6)

    def test_optional_routing_http_selects_model_for_workspace_mode(self):
        routing = {'plan':{'model':'fixture-claude','thinking_effort':'high'},
                   'edit':{'model':'custom-editor','thinking_effort':None}}
        status, data, _ = self.post('/api/sessions', self.options(provider='claude',model_routing=routing))
        self.assertLess(status, 300)
        sid = data['session']['id']
        self.assertEqual(data['session']['model'], 'fixture-claude')
        self.assertEqual(self.post(f'/api/sessions/{sid}/messages', {'prompt':'Edit','mode':'edit'})[0], 400)
        store = self.server.sessions
        store.db.execute("UPDATE sessions SET status='completed',native_session_id='fixture' WHERE id=?", (sid,))
        store.db.commit()
        status, data, _ = self.post(f'/api/sessions/{sid}/messages', {'prompt':'Edit','mode':'edit'})
        self.assertEqual(status, 200)
        self.assertEqual(data['session']['model'], 'custom-editor')
        self.assertIsNone(data['session']['thinking_effort'])
        self.assertEqual(data['session']['model_routing'], routing)
        self.assertEqual(data['session']['mode'], 'edit')

    @unittest.skipUnless(shutil.which('node'), 'Browser calculator check requires Node')
    def test_session_list_carries_summaries_and_the_full_session_comes_on_its_own(self):
        self.server.sessions.providers["codex"]["available"] = True
        session = self.post("/api/sessions", self.options())[1]["session"]
        with self.server.sessions.lock:
            self.server.sessions.db.execute("UPDATE sessions SET brain=? WHERE id=?", (json.dumps({"capsule": {"working": {"goal": "x" * 6000}}}), session["id"]))
            self.server.sessions.db.commit()
        boot, listed = self.request("/api/bootstrap")[1]["sessions"], self.request("/api/sessions")[1]["sessions"]
        self.assertEqual(boot, listed)
        self.assertEqual(set(web.Sessions.SUMMARY_FIELDS) | {"summary"}, set(listed[0]))
        self.assertEqual((session["id"], session["title"], session["status"], True), (listed[0]["id"], listed[0]["title"], listed[0]["status"], listed[0]["summary"]))
        # The heavy parts stay with the session itself.
        self.assertNotIn("x" * 6000, json.dumps(listed))
        full = self.request(f"/api/sessions/{session['id']}")[1]["session"]
        self.assertIn("x" * 6000, json.dumps(full))
        self.assertNotIn("summary", full)

    def test_memory_use_reads_without_the_knowledge_lock_and_its_check_conflicts_when_busy(self):
        base = f"/api/projects/{self.project_id}/memory-use"
        status, empty, _ = self.request(base)
        self.assertEqual((200, None, None), (status, empty["bank_id"], empty["chunks"]))
        self.assertEqual(400, self.request(base + "?path=chunks")[0])
        (self.project / "memory-bank/chunks").mkdir(parents=True)
        self.assertTrue(self.server.knowledge.lock.acquire(blocking=False))
        try:
            status, payload, _ = self.request(base)
            self.assertEqual((200, "memory-bank", []), (status, payload["bank_id"], payload["chunks"]["items"]))
            status, busy, _ = self.post(base + "/check", {})
            self.assertEqual((409, "Another knowledge operation is running. Wait for it to finish."), (status, busy["error"]))
        finally:
            self.server.knowledge.lock.release()
        self.assertEqual(400, self.post(base + "/check", {"bank": 3})[0])
        self.assertEqual(400, self.post(base + "/check?bank=memory-bank", {})[0])

    def test_accelerator_startup_bytes_match_the_ci_budget_measurement(self):
        sys.path.insert(0, str(ROOT / "scripts"))
        import context_budget
        status, data, _ = self.request("/api/accelerators/startup")
        self.assertEqual(200, status)
        self.assertEqual(["Laravel", "Symfony", "PHP Core", "WordPress"], [row["edition"] for row in data["editions"]])
        for row in data["editions"]:
            measured = context_budget.measure_edition(row["edition"])
            self.assertEqual({key: measured[key] for key in context_budget.STARTUP_CATEGORIES}, row["bytes"])
            self.assertEqual((sum(row["bytes"].values()), context_budget.startup_tokens(measured)), (row["total"], row["tokens"]))
            self.assertLessEqual(row["total"], row["ceiling_total"])
        self.assertEqual("docs/TOKEN-ECONOMY-RESEARCH.md as of 9435dfc1^", data["calibration"])
        self.assertEqual(400, self.request("/api/accelerators/startup?edition=Laravel")[0])

    def test_selecting_brain_record_focuses_editable_progress_without_erasing_draft(self):
        page = ui_script()
        source = page[page.index('\nfunction fillKnowledgeSelection('):page.index('\nfunction submitKnowledgeForm(')]
        script = """const assert = require('node:assert/strict');
const fields = {
  'brain-knowledge-action': {value:'brain-update'},
  'brain-op-record_id': {value:''}, 'brain-op-revision': {value:''},
  'brain-op-progress': {value:'Unsaved draft', focus(){this.focused=true;},
    scrollIntoView(){this.scrolled=true;}}
};
const $ = id => fields[id];
const knowledgeActionAvailable = () => true;
const knowledgeViewer = () => ({selected:{id:'record-a', revision:7}});
const showError = () => {};
""" + source + """
fillKnowledgeSelection('brain');
assert.equal(fields['brain-op-record_id'].value,'record-a');
assert.equal(fields['brain-op-revision'].value,7);
assert.equal(fields['brain-op-progress'].value,'Unsaved draft');
assert.equal(fields['brain-op-progress'].focused,true);
assert.equal(fields['brain-op-progress'].scrolled,true);
"""
        subprocess.run([shutil.which('node'), '-e', script], check=True, capture_output=True, text=True)

    @unittest.skipUnless(shutil.which('node'), 'Save to memory check requires Node')
    def test_save_to_memory_keeps_edits_until_a_new_draft_and_reports_what_was_saved(self):
        page = ui_script()
        notes = page[page.index('\nconst memorySaveNotes'):page.index('\nfunction memoryField(')]
        render = page[page.index('\nfunction renderMemorySave('):page.index('\nfunction updateMemorySaveControls(')]
        summary = page[page.index('\nfunction memorySaveSummary('):page.index('\nasync function submitMemorySave(')]
        shown = page[page.index('\nconst MEMORY_DRAFT_BLOCK'):page.index('\nfunction appendEvent(')]
        script = """const assert = require('node:assert/strict');
const fields = {'memory-save-progress':{value:''}, 'memory-save-next':{value:''}, 'memory-save-verified':{checked:true},
  'memory-save-note':{textContent:''}, 'memory-save-learnings':{children:[], replaceChildren(...rows){ this.children = rows; }}};
const $ = id => fields[id]; const showError = () => {}; const updateMemorySaveControls = () => {};
const memoryLearningRow = learning => ({learning}); const el = (tag, className, text) => ({className, textContent:text});
const linkedBrain = {sid:'s1', memoryKey:null, data:{memory_draft:{state:'drafted', event_id:9,
  draft:{progress:'P', next_steps:['A','B'], learnings:[{type:'finding', title:'T', consequence:'C', sources:['x']}]}}}};
""" + notes + render + summary + shown + """
// The reply points at the form instead of repeating the draft's JSON.
assert.equal(withoutMemoryDraft('Done.\\n\\n```memory-draft\\n{"progress": "P"}\\n```\\n'),
  'Done.\\n\\n[Memory draft: review it under Save to memory below.]');
assert.equal(withoutMemoryDraft('No draft here.'),'No draft here.');
renderMemorySave();
assert.equal(fields['memory-save-progress'].value,'P');
assert.equal(fields['memory-save-next'].value,'A\\nB');
assert.equal(fields['memory-save-learnings'].children.length,1);
assert.equal(fields['memory-save-verified'].checked,false);
assert.match(fields['memory-save-note'].textContent,/Drafted by the agent/);
// A refresh of the same draft keeps what the person edited.
fields['memory-save-progress'].value = 'Edited'; renderMemorySave();
assert.equal(fields['memory-save-progress'].value,'Edited');
// A new run's draft replaces the form.
linkedBrain.data.memory_draft = {state:'missing', event_id:null, draft:null}; renderMemorySave();
assert.equal(fields['memory-save-progress'].value,'');
assert.equal(fields['memory-save-learnings'].children.length,0);
assert.match(fields['memory-save-note'].textContent,/left no memory draft/);
const text = data => memorySaveSummary(data).map(line => line.textContent);
assert.deepEqual(text({ok:true, saved:{task:{revision:4}, records:[{type:'finding', title:'Cobalt', status:'resolved'}],
  promotion:{enabled:true, promoted:[{memory_id:'MEM-20261004-aaaaaaaa'}], blocked:[], failed:[]}}}),
  ['Task updated to revision 4.', 'Saved finding “Cobalt” as resolved.', 'Promoted to durable memory as MEM-20261004-aaaaaaaa.']);
assert.match(text({ok:true, saved:{task:null, records:[{type:'decision', title:'D', status:'accepted'}],
  promotion:{enabled:false, promoted:[], blocked:[], failed:[]}}}).at(-1), /Automatic promotion is off/);
const stopped = memorySaveSummary({ok:false, error:'Stale revision.', saved:{task:{revision:5}, records:[], promotion:null}});
assert.equal(stopped.at(-1).className,'error-text');
assert.equal(stopped.at(-1).textContent,'Stopped: Stale revision. What is listed above was saved.');
"""
        subprocess.run([shutil.which('node'), '-e', script], check=True, capture_output=True, text=True)

    @unittest.skipUnless(shutil.which('node'), 'Project memory composer check requires Node')
    def test_project_memory_is_on_by_default_and_never_blocks_a_project_without_it(self):
        page = ui_script()
        source = page[page.index('\nfunction brainLinkConfig('):page.index('\nasync function loadBrainLinkTasks(')]
        script = """const assert = require('node:assert/strict');
const control = value => ({value, checked:false, disabled:false, hidden:false, textContent:''});
const fields = {}; for (const id of ['brain-link-enabled','brain-link-review','brain-link-kind','brain-link-bank','brain-link-task','brain-link-task-id',
  'brain-link-goal','brain-link-query','brain-link-refresh','brain-link-config','brain-link-fields','brain-link-summary','brain-link-mode-note',
  'brain-link-existing-field','brain-link-id-field','brain-link-goal-field','brain-link-query-field','brain-link-availability','project']) fields[id] = control('');
const $ = id => fields[id]; const errors = []; const showError = (id, text) => { if (text) errors.push(text); };
const state = {bootstrap:{}, authFailed:false, selectedId:null, pending:null}; let dryRun = false; const fleetDryRun = () => dryRun;
const brainLinkDraft = {epoch:0, projectId:null, bankId:null, banks:[], tasks:[], meta:null, loading:false, error:'', controller:null};
""" + source + """
fields.project.value = 'p1'; resetBrainLink();
assert.equal(fields['brain-link-enabled'].checked,true);
assert.deepEqual(brainLinkConfig(),{bank:'',review:false,auto:true});
// Until the project's memory is read, a new session waits rather than starting without it; nothing is flagged.
Object.assign(brainLinkDraft,{projectId:'p1', loading:true});
assert.equal(updateBrainLinkControls(),true); assert.equal(brainLinkPending(),true);
// A project without a governed runtime starts its sessions without memory; nothing blocks them.
Object.assign(brainLinkDraft,{loading:false, meta:{runtime_available:false}});
assert.equal(updateBrainLinkControls(),true); assert.equal(brainLinkActive(),false); assert.equal(brainLinkPending(),false);
assert.equal(fields['brain-link-summary'].textContent,'Project files'); assert.match(fields['brain-link-availability'].textContent,/excerpts of its reference files/);
brainLinkDraft.error = 'Read failed.'; assert.equal(updateBrainLinkControls(),true); assert.deepEqual(errors,[]); brainLinkDraft.error = '';
// With a governed runtime the default needs nothing from a person: no task, no query, no review.
Object.assign(brainLinkDraft,{meta:{runtime_available:true, mode:'governed'}, banks:[{id:'memory-bank'}]}); fields['brain-link-bank'].value = 'memory-bank';
assert.equal(updateBrainLinkControls(),true); assert.equal(brainLinkActive(),true);
assert.equal(fields['brain-link-summary'].textContent,'Automatic'); assert.equal(fields['brain-link-query-field'].hidden,true);
assert.deepEqual(brainLinkConfig(),{bank:'memory-bank',review:false,auto:true});
dryRun = true; assert.equal(updateBrainLinkControls(),true); assert.equal(brainLinkActive(),false); dryRun = false;
// Review is the explicit choice: it asks for a query and holds the session to it.
fields['brain-link-review'].checked = true; assert.equal(updateBrainLinkControls(),false); assert.equal(fields['brain-link-query-field'].hidden,false);
fields['brain-link-query'].value = 'cobalt'; assert.equal(updateBrainLinkControls(),true);
assert.deepEqual(brainLinkConfig(),{bank:'memory-bank',review:true,query:'cobalt',auto:true});
fields['brain-link-kind'].value = 'create'; fields['brain-link-review'].checked = false; assert.equal(updateBrainLinkControls(),false);
fields['brain-link-task-id'].value = 'TASK-9'; fields['brain-link-goal'].value = 'Ship it'; assert.equal(updateBrainLinkControls(),true);
assert.deepEqual(brainLinkConfig(),{bank:'memory-bank',review:false,task_id:'TASK-9',create:true,goal:'Ship it'});
// A named task is a choice that has to hold: a project without a runtime refuses it instead of dropping it.
brainLinkDraft.meta = {runtime_available:false}; assert.equal(updateBrainLinkControls(),false); assert.equal(brainLinkActive(),true);
fields['brain-link-enabled'].checked = false; assert.equal(updateBrainLinkControls(),true); assert.equal(brainLinkActive(),false);
assert.equal(fields['brain-link-summary'].textContent,'Off');
// Sessions linked before memory ran by itself were linked for review.
assert.equal(brainReviewed({task_id:'T'}),true); assert.equal(brainReviewed({review:false}),false);
"""
        subprocess.run([shutil.which('node'), '-e', script], check=True, capture_output=True, text=True)

    @unittest.skipUnless(shutil.which('node'), 'Conversation check requires Node')
    def test_the_conversation_shows_what_project_memory_did_for_each_turn(self):
        page = ui_script()
        shown = page[page.index('\nconst MEMORY_DRAFT_BLOCK'):page.index('\nfunction appendEvent(')]
        append = page[page.index('\nfunction appendEvent('):]
        append = append[:append.index('\n}\n') + 3]
        script = """const assert = require('node:assert/strict');
const nodes = []; const events = {append(node){ nodes.push(node); }, get lastElementChild(){ return nodes.at(-1); }};
const $ = id => id === 'events' ? events : null;
const el = (tag, className, text) => ({tag, className, textContent:text, dataset:{}, attributes:{}, children:[],
  setAttribute(name, value){ this.attributes[name] = value; }, append(...items){ this.children.push(...items); }, classList:{contains(){ return false; }}});
const state = {eventIds:new Set(), assistantTexts:new Set(), selected:{provider:'codex', brain:{review:false}}};
const brainReviewed = brain => brain?.review !== false; const providerFor = () => ({name:'Codex'}); const humanLabel = value => value;
const document = {createTextNode: text => ({textContent:text})};
const runView = {observe() {}, statusNode: () => null};
""" + shown + append + """
appendEvent({id:1, kind:'memory', ok:true, text:'Project memory for this turn: task harness/x-1, 1 Memory Bank chunk (812 characters).'});
appendEvent({id:2, kind:'memory', ok:false, text:'Project memory was not retrieved for this turn: Capsule unavailable.'});
assert.deepEqual(nodes.map(node => [node.className, node.dataset.ok, node.attributes.role]), [['memory-event','true','status'],['memory-event','false','status']]);
assert.match(nodes[0].textContent,/task harness\\/x-1/);
appendEvent({id:3, kind:'text', text:'Done.\\n\\n```memory-draft\\n{"progress": "P"}\\n```\\n'});
assert.equal(nodes.at(-1).children.at(-1).textContent,'Done.\\n\\n[Memory draft: saved to project memory when the run completes.]');
"""
        subprocess.run([shutil.which('node'), '-e', script], check=True, capture_output=True, text=True)

    @unittest.skipUnless(shutil.which('node'), 'Browser calculator check requires Node')
    def test_browser_budget_calculator_forms_shared_limits_with_parallel_deadlines(self):
        page=ui_script()
        source=page[page.index('\nfunction perAgentTotals('):page.index('\nfunction agentBudgetControls(')]
        scenarios=[
            ({'usd':2,'tokens':10000,'seconds':120},{'count':4,'waves':1,'fleet':False},{'usd':8,'tokens':40000,'seconds':120}),
            ({'usd':1,'tokens':1000,'seconds':100},{'count':5,'waves':3,'fleet':True},{'usd':5,'tokens':5000,'seconds':300}),
            ({'usd':.1,'tokens':1,'seconds':20},{'count':3,'waves':1,'fleet':False},{'usd':.3,'tokens':3,'seconds':20}),
            ({'usd':None,'tokens':None,'seconds':None},{'count':41,'waves':1,'fleet':False},{'usd':None,'tokens':None,'seconds':None}),
        ]
        script="const assert=require('node:assert/strict');\n"+source+"\nfor(const [values,population,expected] of "+json.dumps(scenarios)+") assert.deepEqual(perAgentTotals(values,population),expected);"
        subprocess.run([shutil.which('node'),'-e',script],check=True,capture_output=True,text=True)

    def test_result_and_verification_routes_keep_csrf_and_input_boundaries(self):
        _,data,_=self.post('/api/sessions',self.options(provider='claude'))
        sid=data['session']['id']; self.server.sessions._status(sid,'completed')
        path=f'/api/sessions/{sid}/results'
        status,result,_=self.request(path);self.assertEqual(status,200)
        self.assertEqual(result['launches'],[]);self.assertIsNone(result['snapshot']['id'])
        self.assertEqual(self.request(path+'?diff=1')[0],400)
        self.assertNotIn('snapshot',self.request(path+'?diff=0')[1])
        body={'command':'python3 --version','timeout':2,'snapshot_id':None}
        self.assertEqual(self.post(f'/api/sessions/{sid}/check',body,headers={'X-Harness-Token':None})[0],403)
        self.assertEqual(self.post(f'/api/sessions/{sid}/check?extra=1',body)[0],400)
        self.assertEqual(self.post(f'/api/sessions/{sid}/check',body)[0],202)
        self.assertEqual(self.request(path)[1]['checks'][0]['status'],'queued')

    def test_session_budget_endpoint_requires_token_revision_and_idle_state(self):
        status,data,_=self.post('/api/sessions',self.options(provider='claude',budgets={'usd':1,'tokens':200,'seconds':10}))
        self.assertEqual(status,201); sid=data['session']['id']; path=f'/api/sessions/{sid}/budgets'
        body={'revision':0,'budgets':{'usd':2,'tokens':400,'seconds':20}}
        self.assertEqual(self.post(path,body,headers={'X-Harness-Token':None})[0],403)
        self.assertEqual(self.post(path,body)[0],400)
        self.server.sessions._status(sid,'failed')
        status,data,_=self.post(path,body); self.assertEqual(status,200)
        self.assertEqual(data['session']['budgets'],body['budgets']); self.assertEqual(data['session']['budget_revision'],1)
        self.assertEqual(self.post(path,body)[0],400)
        self.assertEqual(self.request('/api/sessions/'+sid)[1]['session']['budgets'],body['budgets'])
        status,data,_=self.post(path,{'revision':1,'budgets':{'usd':None,'tokens':None,'seconds':None}})
        self.assertEqual(status,200)
        self.assertIsNone(data['session']['budgets']['seconds'])
        self.assertIsNone(data['session']['agent_budget_plan']['shared_seconds'])
        self.assertIsNone(self.request('/api/sessions/'+sid)[1]['session']['budgets']['seconds'])

    @unittest.skipUnless(shutil.which('git'), 'Delivery routes require Git')
    def test_delivery_routes_require_review_token_fresh_state_and_explicit_actions(self):
        self.initialize_git()
        self.git_command('config', 'user.name', 'HTTP delivery fixture')
        self.git_command('config', 'user.email', 'fixture@example.invalid')
        store = self.server.sessions
        status, created, _ = self.post('/api/sessions', self.options(
            provider='claude', workspace='worktree', worktree_branch='codex/http-delivery'))
        self.assertEqual(status, 201)
        session = created['session']
        sid = session['id']
        path = f'/api/sessions/{sid}/delivery'
        store.results.baseline(session)
        store._status(sid, 'completed')
        worktree = Path(session['project_path'])
        (worktree / 'README.md').write_text('Reviewed HTTP delivery\n')
        status, current, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertTrue(current['available'])
        self.assertIn('README.md', [entry['path'] for entry in current['files']])
        self.assertEqual(self.request(path + '?extra=1')[0], 400)
        body = {'action': 'preview_commit', 'snapshot_id': current['snapshot_id'],
                'paths': ['README.md'], 'message': 'HTTP reviewed commit'}
        self.assertEqual(self.post(path, body, headers={'X-Harness-Token': None})[0], 403)
        self.assertEqual(self.post(path + '?extra=1', body)[0], 400)
        self.assertEqual(self.post(path, {**body, 'unexpected': True})[0], 400)
        self.assertEqual(self.post(path, {**body, 'action': 'push'})[0], 400)
        self.assertEqual(self.post(path, {**body, 'snapshot_id': 'stale'})[0], 400)
        status, preview, _ = self.post(path, body)
        self.assertEqual(status, 200)
        self.assertTrue(preview['can_apply'])
        commit_body = {'action': 'commit', 'preview_id': preview['preview_id']}
        store._status(sid, 'running')
        self.assertEqual(self.post(path, commit_body)[0], 400)
        store._status(sid, 'completed')
        status, committed, _ = self.post(path, commit_body)
        self.assertEqual(status, 200)
        self.assertTrue(committed['ok'])
        self.assertEqual((self.project / 'README.md').read_text(), 'Committed HTTP fixture\n')
        status, preview, _ = self.post(path, {'action': 'preview_transfer',
                                              'commit': committed['commit'], 'target_branch': 'main',
                                              'check': {'command': 'python3 --version', 'timeout': 10}})
        self.assertEqual(status, 200)
        self.assertTrue(preview['can_apply'])
        apply_body = {'action': 'apply', 'preview_id': preview['preview_id']}
        self.assertEqual(self.post(path, apply_body, headers={'X-Harness-Token': None})[0], 403)
        self.assertEqual(self.post(path, apply_body)[0], 400)
        check_body={'action':'check','preview_id':preview['preview_id']}
        self.assertEqual(self.post(path,check_body,headers={'X-Harness-Token':None})[0],403)
        self.assertEqual(self.post(path,{**check_body,'command':'unreviewed'})[0],400)
        self.assertEqual(self.post(path,check_body)[0],200)
        self.assertEqual(self.post(path,apply_body)[0],400)
        # Drain the unused provider fixture job, then execute only the explicit check.
        while not store.jobs.empty():
            job_sid,job,generation=store.jobs.get_nowait()
            if isinstance(job,dict): store.results.run_check(job_sid,job['check_id'],generation)
            store.jobs.task_done()
        checked=self.request(path)[1]['preview']['target_check']
        self.assertEqual(checked['status'],'passed',checked['output'])
        self.assertEqual(checked['candidate'],preview['candidate'])
        status, applied, _ = self.post(path, apply_body)
        self.assertEqual(status, 200)
        self.assertTrue(applied['ok'])
        self.assertEqual(applied['branch'], 'main')
        self.assertEqual(self.git_command('rev-parse', 'HEAD'), applied['commit'])
        self.assertEqual((self.project / 'README.md').read_text(), 'Reviewed HTTP delivery\n')
        self.assertEqual(self.request('/api/sessions/' + sid)[1]['session']['project_path'], str(worktree))

    def test_current_project_delivery_route_reports_unsupported_and_rejects_mutation(self):
        _, created, _ = self.post('/api/sessions', self.options(provider='claude'))
        sid = created['session']['id']
        self.server.sessions._status(sid, 'completed')
        path = f'/api/sessions/{sid}/delivery'
        status, delivery, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertFalse(delivery['available'])
        self.assertTrue(delivery['reason'])
        self.assertEqual(self.post(path, {'action': 'preview_commit', 'snapshot_id': None,
                                          'paths': ['README.md'], 'message': 'Unsupported'})[0], 400)

    def test_creator_routes_enforce_review_and_csrf(self):
        path='/api/creator?'+urlencode({'project_id':self.project_id})
        status,data,_=self.request(path)
        self.assertEqual(status,200);self.assertEqual(data['runs'],[])
        self.assertEqual(self.request('/api/creator')[0],400)
        self.assertEqual(self.request(path+'&project_id=other')[0],400)
        body={'project_id':self.project_id,'provider':'claude','tools':['claude']}
        self.assertEqual(self.request('/api/creator','POST',body)[0],403)
        (self.project/'composer.json').write_text('{}')
        with patch('harness.creator.shutil.which',return_value='/usr/bin/bwrap'):
            status,data,_=self.post('/api/creator',body)
        self.assertEqual(status,201)
        run=data['run'];self.assertEqual(run['status'],'scanning')
        self.assertEqual(self.request('/api/creator/'+run['id'])[0],200)
        self.assertEqual(self.post('/api/creator/'+run['id'],{'action':'generate','revision':run['revision']})[0],400)
        self.assertEqual(self.post('/api/creator/'+run['id'],{'action':'cancel','revision':run['revision']})[0],200)
        self.assertFalse((self.project/'AGENTS.md').exists())

    def git_command(self, *arguments):
        environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        environment.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                            "GIT_AUTHOR_NAME": "Harness fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
                            "GIT_COMMITTER_NAME": "Harness fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
                            "GIT_TERMINAL_PROMPT": "0"})
        return subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgSign=false",
                               "-C", str(self.project), *arguments], env=environment,
                              stdin=subprocess.DEVNULL, capture_output=True, text=True, check=True).stdout.strip()

    def initialize_git(self):
        self.git_command("init", "--quiet")
        self.git_command("symbolic-ref", "HEAD", "refs/heads/main")
        (self.project / "README.md").write_text("Committed HTTP fixture\n", encoding="utf-8")
        self.git_command("add", "README.md")
        self.git_command("commit", "--quiet", "-m", "Initialize HTTP fixture")

    @unittest.skipUnless(shutil.which("git"), "Git workspace tests require git")
    def test_git_http_metadata_tracks_branch_and_dirty_state_and_project_is_default(self):
        path = f"/api/projects/{self.project_id}/git"
        status, plain, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual(plain["project_id"], self.project_id)
        self.assertFalse(plain["is_git"])
        self.assertFalse(plain["worktree_available"])
        self.assertEqual(self.request("/api/projects/unregistered/git")[0], 400)
        self.assertEqual(self.request(path + "?other=1")[0], 400)
        self.initialize_git()
        status, clean, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertTrue(clean["is_git"])
        self.assertTrue(clean["worktree_available"])
        self.assertEqual(clean["branch"], "main")
        self.assertEqual(clean["head"], self.git_command("rev-parse", "HEAD"))
        self.assertFalse(clean["dirty"])
        self.git_command("checkout", "--quiet", "-b", "codex/http-current")
        (self.project / "README.md").write_text("Dirty primary HTTP fixture\n", encoding="utf-8")
        changed = self.request(path)[1]
        self.assertEqual(changed["branch"], "codex/http-current")
        self.assertTrue(changed["dirty"])
        self.server.sessions.providers["codex"]["available"] = True
        status, created, _ = self.post("/api/sessions", self.options())
        self.assertEqual(status, 201)
        session = created["session"]
        self.assertEqual((session["workspace"], session["project_path"], session["branch"]),
                         ("project", str(self.project), "codex/http-current"))
        self.assertEqual((self.project / "README.md").read_text(), "Dirty primary HTTP fixture\n")

    @unittest.skipUnless(shutil.which("git"), "Git workspace tests require git")
    def test_worktree_http_creation_and_followup_keep_the_selected_workspace(self):
        self.initialize_git()
        store = self.server.sessions
        store.providers["codex"]["available"] = True
        refs = self.git_command("show-ref")
        for changes in ({"workspace": []}, {"workspace": "unknown"},
                        {"workspace": "worktree", "worktree_branch": "main"},
                        {"workspace": "worktree", "worktree_branch": "bad\0branch"}):
            with self.subTest(changes=changes):
                self.assertEqual(self.post("/api/sessions", self.options(**changes))[0], 400)
        self.assertEqual(store.list(), [])
        self.assertEqual(store.jobs.qsize(), 0)
        self.assertEqual(self.git_command("show-ref"), refs)
        status, created, _ = self.post("/api/sessions", self.options(
            workspace="worktree", worktree_branch="codex/http-session",
        ))
        self.assertEqual(status, 201)
        original = created["session"]
        worktree = store.state_dir / "worktrees" / original["id"]
        self.assertEqual((original["workspace"], original["branch"], original["project_path"]),
                         ("worktree", "codex/http-session", str(worktree)))
        self.assertEqual(original["git_common_dir"], str(self.project / ".git"))
        self.assertEqual((worktree / "README.md").read_text(), "Committed HTTP fixture\n")
        self.assertEqual(self.git_command("branch", "--show-current"), "main")
        with store.lock:
            store.db.execute("UPDATE sessions SET status='completed',native_session_id='fixture-worktree' WHERE id=?",
                             (original["id"],))
            store.db.commit()
        path = f'/api/sessions/{original["id"]}'
        before = self.request(path)[1], store.jobs.qsize()
        for changes in ({"workspace": "project"}, {"worktree_branch": "other"},
                        {"branch": "other"}, {"project_path": str(self.project)}):
            with self.subTest(changes=changes):
                self.assertEqual(self.post(path + "/messages", {"prompt": "Change workspace", **changes})[0], 400)
                self.assertEqual((self.request(path)[1], store.jobs.qsize()), before)
        status, resumed, _ = self.post(path + "/messages", {"prompt": "Continue this worktree"})
        self.assertEqual(status, 200)
        for field in ("workspace", "branch", "project_id", "project_path", "git_common_dir"):
            self.assertEqual(resumed["session"][field], original[field])
        self.assertEqual(resumed["session"]["native_session_id"], "fixture-worktree")

    def test_fleet_http_decisions_resume_and_report_enforce_state_auth_and_schema(self):
        store = self.server.sessions
        runtime = {"available": True, "executable": "/never-launched/fleet-python",
                   "detail": "Offline fixture", "lenses": [], "max_worker_timeout": 3600}
        options = self.options(workflow="fleet-review", fleet={
            "lenses": ["code-reviewer"], "dry_run": True, "budget_usd": None, "worker_timeout": 30,
        })
        with patch("harness.sessions.fleet_runtime", return_value=runtime):
            status, created, _ = self.post("/api/sessions", options)
            self.assertEqual(status, 201)
            original = created["session"]
            self.assertEqual(original["fleet"], options["fleet"])
            self.assertFalse(original["agents_enabled"])
            path = f'/api/sessions/{original["id"]}'
            self.assertEqual(self.request(path + "/report")[0], 400)
            for action, payload in (("decision", {"approve": True}), ("resume", {}),
                                    ("messages", {"prompt": "No Fleet chat followup"})):
                self.assertEqual(self.post(path + "/" + action, payload)[0], 400)
            store._status(original["id"], "awaiting_approval")
            before = self.request(path)[1], store.jobs.qsize()
            for value in (None, 0, 1, "true", [], {}):
                with self.subTest(approve=value):
                    self.assertEqual(self.post(path + "/decision", {"approve": value})[0], 400)
            for payload in ({}, {"approve": True, "extra": True}):
                self.assertEqual(self.post(path + "/decision", payload)[0], 400)
            self.assertEqual(self.post(path + "/decision", {"approve": True},
                                       headers={"X-Harness-Token": None})[0], 403)
            self.assertEqual((self.request(path)[1], store.jobs.qsize()), before)
            status, approved, _ = self.post(path + "/decision", {"approve": True})
            self.assertEqual(status, 200)
            self.assertEqual(approved["session"]["status"], "queued")
            self.assertEqual(approved["session"]["fleet"], original["fleet"])
            self.assertEqual(self.post(path + "/decision", {"approve": False})[0], 400)
            self.assertEqual(self.request(path + "/report")[0], 400)

            # Simulate an interrupted queued approval. Resume must retain that decision.
            store._status(original["id"], "interrupted")
            before = self.request(path)[1], store.jobs.qsize()
            self.assertEqual(self.post(path + "/resume", {"extra": True})[0], 400)
            self.assertEqual(self.post(path + "/resume", {}, headers={"X-Harness-Token": "wrong"})[0], 403)
            self.assertEqual((self.request(path)[1], store.jobs.qsize()), before)
            status, resumed, _ = self.post(path + "/resume", {})
            self.assertEqual(status, 200)
            self.assertEqual(resumed["session"]["status"], "queued")
            self.assertEqual(store.db.execute("SELECT fleet_action FROM sessions WHERE id=?",
                                              (original["id"],)).fetchone()[0], "approve")

            # Supply the completed runner receipt; this HTTP fixture never starts a graph.
            report = "# Approved fixture Fleet report\n"
            with store.lock:
                store.db.execute("UPDATE sessions SET status='completed',fleet_result=? WHERE id=?",
                                 (json.dumps({"report": report}), original["id"]))
                store.db.commit()
            status, body, headers = self.request(path + "/report")
            self.assertEqual((status, body), (200, report.encode("utf-8")))
            self.assertTrue(headers["Content-Type"].startswith("text/markdown"))
            self.assertEqual(self.request(path + "/report?extra=1")[0], 400)
            store._status(original["id"], "rejected")
            self.assertEqual(self.request(path + "/report")[0], 400)
        self.assertEqual(self.request("/api/sessions/unregistered/report")[0], 400)
        self.assertEqual(list(self.project.iterdir()), [])

    def test_bootstrap_and_only_explicit_static_routes_are_served(self):
        status, boot, headers = self.request("/api/bootstrap")
        self.assertEqual(status, 200)
        self.assertEqual(boot["csrf"], self.token)
        self.assertEqual(boot["projects"], [{"id": self.project_id, "name": "project", "path": str(self.project), "available": True}])
        self.assertFalse(boot["providers"][0]["available"])
        self.assertNotIn("executable", boot["providers"][0])
        self.assertEqual({kit["id"] for kit in boot["accelerators"]}, {"kit1", "kit2", "kit3"})
        self.assertEqual((boot["runtime"]["max_agents"], boot["runtime"]["default_agent_count"]), (40, 3))
        # Usage › Context labels project reference excerpts by the size the prompt sends.
        self.assertEqual(boot["runtime"]["context_excerpt_bytes"], 3000)
        self.assertEqual(headers["Cache-Control"], "no-store")
        self.assertEqual(headers["X-Content-Type-Options"], "nosniff")
        self.assertIn("frame-ancestors 'self'", headers["Content-Security-Policy"])
        page_html = (WEB / "index.html").read_text(encoding="utf-8")
        self.assertEqual(set(re.findall(r'(?:src|href)="/([^"/]+)"', page_html)), set(web.ASSETS))
        served = self.request("/")[1]
        self.assertEqual(len(web.ASSETS), len(re.findall(rb'(?:src|href)="/[^"/?]+\?v=[0-9a-f]{16}"', served)))
        for name, content_type in web.ASSETS.items():
            with self.subTest(asset=name):
                status, body, asset_headers = self.request("/" + name)
                self.assertEqual((status, body), (200, (WEB / name).read_bytes()))
                self.assertEqual(asset_headers["Content-Type"], content_type)
                self.assertEqual(asset_headers["X-Content-Type-Options"], "nosniff")
                # Page files are kept by the browser and revalidated by their hash on every load.
                self.assertEqual(asset_headers["Cache-Control"], "no-cache")
                tag = asset_headers["ETag"]
                self.assertEqual(tag, '"' + hashlib.sha256(body).hexdigest()[:32] + '"')
                self.assertEqual(self.request("/" + name, headers={"If-None-Match": tag})[:2], (304, b""))
                self.assertEqual(self.request("/" + name, headers={"If-None-Match": '"stale", ' + tag})[0], 304)
                self.assertEqual(self.request("/" + name, headers={"If-None-Match": '"stale"'})[:2], (200, body))
                # The served page names the asset by its hash; that address may be kept for good.
                version = re.search(r'="/%s\?v=([0-9a-f]{16})"' % re.escape(name), served.decode()).group(1)
                self.assertEqual(version, hashlib.sha256(body).hexdigest()[:16])
                pinned = self.request(f"/{name}?v={version}")
                self.assertEqual((pinned[0], pinned[1], pinned[2]["Cache-Control"]), (200, body, "private, max-age=31536000, immutable"))
                self.assertEqual(self.request(f"/{name}?v=0000000000000000")[2]["Cache-Control"], "no-cache")
                head_status, head, head_headers = self.request("/" + name, "HEAD")
                self.assertEqual((head_status, head, int(head_headers["Content-Length"])), (200, b"", len(body)))
        for path, marker in (("/", b"AI Infrastructure Harness"),
                             ("/kit3/", b"Open Source Kit"),
                             ("/kit3/index.html", b"Open Source Kit")):
            with self.subTest(path=path):
                status, page, page_headers = self.request(path)
                self.assertEqual(status, 200)
                self.assertIn(marker, page)
                self.assertEqual(self.request(path, headers={"If-None-Match": page_headers["ETag"]})[:2], (304, b""))
                head_status, head, head_headers = self.request(path, "HEAD")
                self.assertEqual(head_status, 200)
                self.assertEqual(head, b"")
                self.assertEqual(int(head_headers["Content-Length"]), len(page))
        for path in ("/index.html", "/kit3", "/api/bootstrap/", "/harness/web/index.html", "/harness/web/app.css",
                     "/app-unknown.js", "/app.css/", "/APP.CSS",
                     "/scripts/install_accelerator.py", "/../state/sessions.sqlite3", "/%2e%2e/secret"):
            with self.subTest(path=path):
                self.assertEqual(self.request(path)[0], 404)

    def test_host_origin_and_fetch_metadata_reject_cross_site_api_access(self):
        port = self.server.server_port
        for headers in ({"Host": None}, {"Host": f"attacker.invalid:{port}"},
                        {"Host": "127.0.0.1:1"}, {"Origin": "https://attacker.invalid"},
                        {"Origin": "null"}, {"Origin": f"http://localhost:{port + 1}"},
                        {"Sec-Fetch-Site": "cross-site"}):
            with self.subTest(headers=headers):
                status, error, _ = self.request("/api/bootstrap", headers=headers)
                self.assertEqual(status, 403)
                self.assertNotIn(self.token, json.dumps(error))
                self.assertEqual(self.post("/api/sessions", self.options(), headers=headers)[0], 403)
        self.assertEqual(self.request("/api/bootstrap", headers={
            "Host": f"localhost:{port}", "Origin": f"http://localhost:{port}",
            "Sec-Fetch-Site": "same-origin",
        })[0], 200)

    def test_post_requires_the_current_token_before_mutation(self):
        for token in (None, "", "wrong-token", "non-ascii-é"):
            with self.subTest(token=token):
                self.assertEqual(self.post("/api/sessions", self.options(),
                                           headers={"X-Harness-Token": token})[0], 403)
        self.assertEqual(self.request("/api/sessions")[1], {"sessions": []})

    def test_json_framing_constants_duplicate_fields_and_nonobjects_are_rejected(self):
        for raw in (b"", b"{", b"null", b"[]", b"true", b'"text"',
                    b'{"prompt":"first","prompt":"second"}',
                    b'{"nested":{"id":1,"id":2}}', b'{"model":NaN}',
                    b'{"model":Infinity}', b'{"model":-Infinity}', b'{"prompt":"\xff"}'):
            with self.subTest(raw=raw):
                self.assertEqual(self.post("/api/sessions", raw=raw)[0], 400)
        for headers in ({"Content-Type": "text/plain"}, {"Content-Length": None},
                        {"Content-Length": "invalid"}, {"Content-Length": "-1"},
                        {"Content-Length": str(web.MAX_JSON_BYTES + 1)}, {"Transfer-Encoding": "chunked"}):
            with self.subTest(headers=headers):
                self.assertEqual(self.post("/api/sessions", raw=b"{}", headers=headers)[0], 400)
        self.assertEqual(self.server.sessions.list(), [])

    def test_validated_creation_event_cursor_and_cancellation_use_the_same_session(self):
        self.assertEqual(self.post("/api/sessions", self.options())[0], 400)  # Provider unavailable.
        self.server.sessions.providers["codex"]["available"] = True
        status, created, _ = self.post("/api/sessions", self.options(model="fixture-model"))
        self.assertEqual(status, 201)
        session = created["session"]
        self.assertIs(session["agents_enabled"], False)
        self.assertEqual(session["agent_count"], 3)
        self.assertEqual((session["project_id"], session["provider"], session["status"]),
                         (self.project_id, "codex", "queued"))
        path = f'/api/sessions/{session["id"]}'
        status, snapshot, _ = self.request(path + "?after=0")
        self.assertEqual(status, 200)
        self.assertEqual(snapshot["events"][0]["text"], "Inspect the fixture")
        cursor = snapshot["events"][-1]["id"]
        self.assertEqual(self.request(path + f"?after={cursor}")[1]["events"], [])
        for query in ("after=-1", "after=NaN", "after=0&after=1", "other=1"):
            with self.subTest(query=query):
                self.assertEqual(self.request(path + "?" + query)[0], 400)
        self.assertEqual(self.post(path + "/messages", {"prompt": "Already active"})[0], 400)
        self.assertEqual(self.post(path + "/cancel", {"unexpected": True})[0], 400)
        self.assertEqual(self.post(path + "/cancel", {})[1]["session"]["status"], "cancelled")
        self.assertEqual(self.post(path + "/messages", {"prompt": "No native resume ID"})[0], 400)
        self.assertEqual(self.request("/api/sessions")[1]["sessions"][0]["id"], session["id"])

    def test_invalid_session_option_types_never_reach_the_queue(self):
        self.server.sessions.providers["codex"]["available"] = True
        invalid = [{"prompt": value} for value in (None, [], "", " ", "a\0b", "é" * 16001)]
        invalid += [{"project_id": str(self.root)}, {"project_id": {}}, {"provider": []},
                    {"mode": []}, {"workflow": {}}, {"workflow": "review", "mode": "edit"},
                    {"project_context": 1}, {"project_context": "true"}, {"native_session_id": "injected"}]
        invalid += [{"model": value} for value in ([], {}, False, 0, "-unsafe", "x" * 121)]
        invalid += [{"agents_enabled": value} for value in (None, 0, 1, "true", "false", [], {})]
        invalid += [{"agent_count": value} for value in (None, False, True, "3", 0, -1, 41, 1.5, 3.0, [], {})]
        invalid += [{"thinking_effort": value} for value in (False, True, 0, 1, [], {}, "unknown", "high\0")]
        invalid += [{"model": "fixture-small", "thinking_effort": "high"}]
        for changes in invalid:
            with self.subTest(changes=changes):
                self.assertEqual(self.post("/api/sessions", self.options(**changes))[0], 400)
        self.assertEqual(self.server.sessions.list(), [])

    def test_agent_settings_can_change_through_followup_with_validation(self):
        self.server.sessions.providers["codex"]["available"] = True
        status, data, _ = self.post("/api/sessions", self.options(agents_enabled=True, agent_count=40))
        self.assertEqual(status, 201)
        session = data["session"]
        path = f'/api/sessions/{session["id"]}'
        self.assertIs(session["agents_enabled"], True)
        self.assertEqual(session["agent_count"], 40)
        self.assertEqual(self.request(path)[1]["session"], session)
        # Supply a resumable receipt without launching a provider in this HTTP suite.
        with self.server.sessions.lock:
            self.server.sessions.db.execute(
                "UPDATE sessions SET status='completed',native_session_id='fixture-native' WHERE id=?",
                (session["id"],),
            )
            self.server.sessions.db.commit()
        before = self.request(path)[1]
        for change in [{"agents_enabled":v} for v in (None, 0, 1, "false", [], {})] + [{"agent_count":v} for v in (None, False, 0, 41, 1.5, "3", [], {})]:
            with self.subTest(change=change):
                self.assertEqual(self.post(path + "/messages", {"prompt": "Change settings", **change})[0], 400)
                self.assertEqual(self.request(path)[1], before)
        status, resumed, _ = self.post(path + "/messages", {"prompt": "Continue", "agents_enabled":False, "agent_count":3})
        self.assertEqual(status, 200)
        self.assertIs(resumed["session"]["agents_enabled"], False)
        self.assertEqual(resumed["session"]["agent_count"], 3)
        self.assertEqual(self.post(path + "/messages", {"prompt":"Active update", "agent_count":4})[0], 400)
        self.assertEqual(self.request(path)[1]["session"]["agent_count"], 3)
        self.assertEqual([event["text"] for event in self.request(path)[1]["events"] if event["kind"] == "user"],
                         ["Inspect the fixture", "Continue"])

    def test_followup_model_and_effort_overrides_return_effective_settings(self):
        self.server.sessions.providers["codex"]["available"] = True
        status, data, _ = self.post("/api/sessions", self.options(
            model="fixture-model", thinking_effort="high", agents_enabled=True, agent_count=7,
        ))
        self.assertEqual(status, 201)
        original = data["session"]
        self.assertEqual((original["model"], original["thinking_effort"]), ("fixture-model", "high"))
        sid = original["id"]
        path = f"/api/sessions/{sid}"
        store = self.server.sessions

        def complete():
            with store.lock:
                store.db.execute("UPDATE sessions SET status='completed',native_session_id='fixture-native' WHERE id=?", (sid,))
                store.db.commit()

        def assert_rejected(changes):
            before = self.request(path)[1], store.jobs.qsize()
            self.assertEqual(self.post(path + "/messages", {"prompt": "Rejected update", **changes})[0], 400)
            self.assertEqual((self.request(path)[1], store.jobs.qsize()), before)

        # Even valid option changes must leave an active session untouched.
        for active_status in ("queued", "running"):
            store._status(sid, active_status)
            assert_rejected({"model": "fixture-small", "thinking_effort": "low"})
        complete()
        invalid = [{"provider": "cursor"}, {"project_id": self.project_id}, {"mode": "edit"},
                   {"workflow": "review"}, {"project_context": True}, {"agents_enabled": "false"},
                   {"agent_count": 0}, {"native_session_id": "other"}, {"model": []},
                   {"model": "x" * 121}, {"thinking_effort": []}, {"thinking_effort": True},
                   {"thinking_effort": "unknown"}, {"model": "fixture-small"}]
        for changes in invalid:
            with self.subTest(rejected=changes):
                assert_rejected(changes)

        changes_and_expected = [
            ({"model": "fixture-small", "thinking_effort": "low"}, ("fixture-small", "low")),
            ({}, ("fixture-small", "low")),
            ({"thinking_effort": None}, ("fixture-small", None)),
            ({"model": "custom-" + "x" * 113}, ("custom-" + "x" * 113, None)),
            ({"model": "", "thinking_effort": ""}, (None, None)),
            ({"model": "fixture-model", "thinking_effort": "high"}, ("fixture-model", "high")),
            ({"model": None, "thinking_effort": None}, (None, None)),
        ]
        for changes, expected in changes_and_expected:
            with self.subTest(update=changes):
                complete()
                status, data, _ = self.post(path + "/messages", {"prompt": "Continue with settings", **changes})
                self.assertEqual(status, 200)
                session = data["session"]
                self.assertEqual((session["model"], session["thinking_effort"]), expected)
                self.assertEqual(session["native_session_id"], "fixture-native")
                for field in ("id", "provider", "project_id", "project_path", "mode", "workflow",
                              "project_context", "agents_enabled", "agent_count"):
                    self.assertEqual(session[field], original[field])
                self.assertEqual(self.request(path)[1]["session"], session)
                listed = next(row for row in self.request("/api/sessions")[1]["sessions"] if row["id"] == sid)
                self.assertEqual((listed["model"], listed["thinking_effort"]), expected)

    def test_ultracode_http_lifecycle_requires_immutable_enabled_helpers(self):
        store = self.server.sessions
        options = self.options(provider="claude", model="fixture-claude",
                               thinking_effort="ultracode", agent_count=7)
        self.assertEqual(self.post("/api/sessions", {**options, "agents_enabled": False})[0], 400)
        self.assertEqual(store.list(), [])
        self.assertEqual(store.jobs.qsize(), 0)
        self.assertEqual(store.db.execute("SELECT COUNT(*) FROM events").fetchone()[0], 0)
        status, created, _ = self.post("/api/sessions", {**options, "agents_enabled": True})
        self.assertEqual(status, 201)
        sid = created["session"]["id"]
        path = f"/api/sessions/{sid}"
        self.assertEqual(created["session"]["thinking_effort"], "ultracode")

        def complete(session_id):
            with store.lock:
                store.db.execute("UPDATE sessions SET status='completed',native_session_id='fixture-claude-native' WHERE id=?", (session_id,))
                store.db.commit()

        for effort in ("high", "ultracode"):
            complete(sid)
            status, data, _ = self.post(path + "/messages", {
                "prompt": "Continue Claude session", "thinking_effort": effort,
            })
            self.assertEqual(status, 200)
            current = data["session"]
            self.assertEqual((current["thinking_effort"], current["agents_enabled"],
                              current["agent_count"], current["native_session_id"]),
                             (effort, True, 7, "fixture-claude-native"))
            self.assertEqual(self.request(path)[1]["session"], current)
        status, created, _ = self.post("/api/sessions", {
            **options, "thinking_effort": "high", "agents_enabled": False,
        })
        self.assertEqual(status, 201)
        off_sid = created["session"]["id"]
        complete(off_sid)
        off_path = f"/api/sessions/{off_sid}"
        before = self.request(off_path)[1], store.jobs.qsize(), dict(store.generations)
        for changes in ({"thinking_effort": "ultracode"},
                        {"thinking_effort": "ultracode", "agents_enabled": False}):
            with self.subTest(rejected_followup=changes):
                self.assertEqual(self.post(off_path + "/messages", {"prompt": "Rejected", **changes})[0], 400)
                self.assertEqual((self.request(off_path)[1], store.jobs.qsize(), dict(store.generations)), before)

    def test_completed_session_history_remains_available_after_the_first_page(self):
        self.server.sessions.providers["codex"]["available"] = True
        session = self.post("/api/sessions", self.options())[1]["session"]
        for index in range(249):
            self.server.sessions._event(session["id"], {"kind": "status", "text": f"Fixture event {index}"})
        self.server.sessions._event(session["id"], {"kind": "text", "text": "Final answer on page two"})
        self.server.sessions._status(session["id"], "completed")
        path = f'/api/sessions/{session["id"]}'
        page_one = self.request(path)[1]
        self.assertEqual(page_one["session"]["status"], "completed")
        self.assertEqual(len(page_one["events"]), 250)
        cursor = page_one["events"][-1]["id"]
        page_two = self.request(path + f"?after={cursor}")[1]
        self.assertEqual([event["text"] for event in page_two["events"]], ["Final answer on page two"])

    def test_context_cannot_follow_symlinks_or_address_an_unregistered_project(self):
        outside = self.root / "private"
        outside.mkdir()
        (outside / "README.md").write_text("PRIVATE OUTSIDE CONTENT")
        (self.project / "AGENTS.md").symlink_to(outside / "README.md")
        (self.project / "project-brain").symlink_to(outside, target_is_directory=True)
        (self.project / "README.md").write_text("Regular context")
        status, data, _ = self.request(f"/api/projects/{self.project_id}/context")
        self.assertEqual(status, 200)
        files = {item["path"]: item for item in data["files"]}
        self.assertTrue(data["context_available"])
        self.assertEqual(files["README.md"]["bytes"], len("Regular context"))
        self.assertFalse(files["AGENTS.md"]["exists"])
        self.assertFalse(files["project-brain/README.md"]["exists"])
        self.assertNotIn("PRIVATE OUTSIDE CONTENT", json.dumps(data))
        self.assertEqual(self.request("/api/projects/unregistered/context")[0], 400)

    def test_memory_bank_discovery_lists_only_fixed_existing_banks(self):
        path = f"/api/projects/{self.project_id}/memory"
        status, empty, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual((empty["project_id"], empty["banks"], empty["bank_id"], empty["entries"]),
                         (self.project_id, [], None, []))
        self.assertFalse(empty["truncated"])

        # An existing empty edition bank is valid even when the root bank is absent.
        (self.project / "PHP Core/memory-bank").mkdir(parents=True)
        status, edition, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual(edition["bank_id"], "PHP Core/memory-bank")
        self.assertEqual(edition["entries"], [])
        bank_ids = ("memory-bank", "Laravel/memory-bank", "Symfony/memory-bank",
                    "PHP Core/memory-bank", "Cms/wordpress/memory-bank")
        for bank in bank_ids:
            (self.project / bank).mkdir(parents=True, exist_ok=True)
        (self.project / "arbitrary/memory-bank").mkdir(parents=True)
        status, banks, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual([bank["id"] for bank in banks["banks"]], list(bank_ids))
        self.assertEqual(banks["bank_id"], "memory-bank")
        for bank in banks["banks"]:
            self.assertIsInstance(bank["name"], str)
            self.assertTrue(bank["name"])
            self.assertIsInstance(bank["path"], str)
        status, selected, _ = self.request(path + "?" + urlencode({"bank": "PHP Core/memory-bank"}))
        self.assertEqual(status, 200)
        self.assertEqual((selected["bank_id"], selected["entries"]), ("PHP Core/memory-bank", []))

    def test_memory_bank_listing_and_document_reads_leave_project_and_database_unchanged(self):
        bank = self.project / "memory-bank"
        allowed = {
            "README.md": "# Bank overview\nFixture overview.\n",
            "chunks/MEM-20260905-note.md": "# Fixture memory\nПроверенная заметка.\n",
            "local/decision.md": "# Local decision\nFixture decision.\n",
        }
        excluded = (".private.md", "chunks/.private.md", "local/.private.md",
                    "scripts/example.md", "templates/example.md", "tests/example.md",
                    "chunks/nested/hidden.md", "local/nested/hidden.md", "context.db", "local/context.db")
        for name, content in {**allowed, **{name: "EXCLUDED PRIVATE FIXTURE" for name in excluded}}.items():
            target = bank / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content, encoding="utf-8")
        before = {str(item.relative_to(self.project)): (item.stat().st_mtime_ns, item.read_bytes())
                  for item in self.project.rglob("*") if item.is_file()}
        store = self.server.sessions
        database_before = list(store.db.iterdump())
        path = f"/api/projects/{self.project_id}/memory"
        status, listing, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual({entry["path"] for entry in listing["entries"]}, set(allowed))
        self.assertEqual(listing["entries"][0]["path"], "chunks/MEM-20260905-note.md")
        self.assertFalse(listing["truncated"])
        for entry in listing["entries"]:
            self.assertIsInstance(entry["title"], str)
            self.assertTrue(entry["title"])
            self.assertIsInstance(entry["kind"], str)
            self.assertEqual(entry["bytes"], len(allowed[entry["path"]].encode("utf-8")))
            self.assertNotIn("content", entry)
            status, payload, _ = self.request(path + "?" + urlencode({"bank": "memory-bank", "path": entry["path"]}))
            self.assertEqual(status, 200)
            document = payload
            self.assertEqual(document["path"], entry["path"])
            self.assertEqual(document["content"], allowed[entry["path"]])
            self.assertEqual(document["bytes"], entry["bytes"])
            self.assertFalse(document["truncated"])
        for name in excluded:
            with self.subTest(excluded=name):
                status, payload, _ = self.request(path + "?" + urlencode({"bank": "memory-bank", "path": name}))
                self.assertEqual(status, 400)
                self.assertNotIn("EXCLUDED PRIVATE FIXTURE", json.dumps(payload))
        self.assertNotIn("EXCLUDED PRIVATE FIXTURE", json.dumps(listing))
        self.assertEqual({str(item.relative_to(self.project)): (item.stat().st_mtime_ns, item.read_bytes())
                          for item in self.project.rglob("*") if item.is_file()}, before)
        self.assertEqual(list(store.db.iterdump()), database_before)
        self.assertEqual(store.jobs.qsize(), 0)

    def test_memory_bank_caps_document_bytes_and_listing_entries(self):
        chunks = self.project / "memory-bank/chunks"
        chunks.mkdir(parents=True)
        content = "# Large fixture\n" + "x" * (256 * 1024 + 100)
        (chunks / "large.md").write_text(content, encoding="utf-8")
        path = f"/api/projects/{self.project_id}/memory"
        status, payload, _ = self.request(path + "?" + urlencode({"bank": "memory-bank", "path": "chunks/large.md"}))
        self.assertEqual(status, 200)
        document = payload
        self.assertTrue(document["truncated"])
        self.assertEqual(document["bytes"], len(content.encode("utf-8")))
        self.assertEqual(document["content"], content[:256 * 1024])
        for index in range(499):
            (chunks / f"MEM-{index:04d}.md").write_text("# Small fixture\n", encoding="utf-8")
        status, listing, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual(len(listing["entries"]), 500)
        self.assertFalse(listing["truncated"])
        (chunks / "overflow.md").write_text("# Overflow fixture\n", encoding="utf-8")
        status, listing, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual(len(listing["entries"]), 500)
        self.assertEqual(len({entry["path"] for entry in listing["entries"]}), 500)
        self.assertTrue(listing["truncated"])

    def test_memory_bank_rejects_invalid_queries_and_out_of_scope_document_paths(self):
        bank = self.project / "memory-bank"
        bank.mkdir()
        (bank / "README.md").write_text("# Valid fixture\n", encoding="utf-8")
        path = f"/api/projects/{self.project_id}/memory"
        queries = ["other=1", "bank", "path", "bank=", "path=", "bank=memory-bank&path=",
                   "bank=memory-bank&bank=memory-bank",
                   "bank=memory-bank&path=README.md&path=README.md"]
        invalid_banks = ("elsewhere", "Symfony/memory-bank", "../memory-bank", "/memory-bank",
                         "memory-bank/", "./memory-bank", "memory-bank\\chunks", "memory-bank\0", "memory-bank\n")
        queries += [urlencode({"bank": bank_id}) for bank_id in invalid_banks]
        invalid_paths = ("../README.md", "/README.md", "./README.md", "chunks/../README.md",
                         "chunks//note.md", "README.md/", "chunks\\note.md", "README.md\0",
                         "README.md\n", "README.md\x7f", "missing.md", ".private.md",
                         "scripts/example.md", "templates/example.md", "tests/example.md",
                         "chunks/nested/note.md", "local/context.db", "context.db")
        queries += [urlencode({"bank": "memory-bank", "path": name}) for name in invalid_paths]
        for query in queries:
            with self.subTest(query=query):
                self.assertEqual(self.request(path + "?" + query)[0], 400)
        self.assertEqual(self.request("/api/projects/unregistered/memory")[0], 400)
        self.assertEqual(self.server.sessions.list(), [])
        self.assertEqual(self.server.sessions.jobs.qsize(), 0)

    def test_memory_bank_ignores_and_refuses_links_and_nonregular_files(self):
        bank = self.project / "memory-bank"
        bank.mkdir()
        outside = self.root / "outside"
        outside.mkdir()
        secret = outside / "secret.md"
        secret.write_text("PRIVATE OUTSIDE CONTENT", encoding="utf-8")
        (bank / "symlink.md").symlink_to(secret)
        os.link(secret, bank / "hardlink.md")
        os.mkfifo(bank / "pipe.md")
        (bank / "directory.md").mkdir()
        (bank / "chunks").symlink_to(outside, target_is_directory=True)
        (bank / "local").mkdir()
        (bank / "local/linked.md").symlink_to(secret)
        path = f"/api/projects/{self.project_id}/memory"
        status, listing, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual(listing["entries"], [])
        for name in ("symlink.md", "hardlink.md", "pipe.md", "directory.md", "chunks/secret.md", "local/linked.md"):
            with self.subTest(path=name):
                status, payload, _ = self.request(path + "?" + urlencode({"bank": "memory-bank", "path": name}))
                self.assertEqual(status, 400)
                self.assertNotIn("PRIVATE OUTSIDE CONTENT", json.dumps(payload))
        # The same file safety applies to fixed context reads.
        os.link(secret, self.project / "README.md")
        context = self.request(f"/api/projects/{self.project_id}/context")[1]
        self.assertFalse(next(item for item in context["files"] if item["path"] == "README.md")["exists"])

    def test_memory_bank_discovery_refuses_symlink_banks_and_edition_ancestors(self):
        outside = self.root / "outside"
        (outside / "memory-bank").mkdir(parents=True)
        (outside / "memory-bank/README.md").write_text("PRIVATE BANK", encoding="utf-8")
        (self.project / "memory-bank").symlink_to(outside / "memory-bank", target_is_directory=True)
        (self.project / "Laravel").symlink_to(outside, target_is_directory=True)
        (self.project / "Cms").mkdir()
        (self.project / "Cms/wordpress").symlink_to(outside, target_is_directory=True)
        (self.project / "PHP Core").mkdir()
        (self.project / "PHP Core/memory-bank").write_text("Not a directory", encoding="utf-8")
        path = f"/api/projects/{self.project_id}/memory"
        status, listing, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual((listing["banks"], listing["bank_id"], listing["entries"]), ([], None, []))
        for bank_id in ("memory-bank", "Laravel/memory-bank", "Cms/wordpress/memory-bank", "PHP Core/memory-bank"):
            with self.subTest(bank=bank_id):
                status, payload, _ = self.request(path + "?" + urlencode({"bank": bank_id}))
                self.assertEqual(status, 400)
                self.assertNotIn("PRIVATE BANK", json.dumps(payload))

    def test_memory_and_context_refuse_a_replaced_registered_project_ancestor(self):
        parent = self.root / "registered-parent"
        project = parent / "project"
        (project / "memory-bank").mkdir(parents=True)
        (project / "memory-bank/README.md").write_text("# Registered memory\n", encoding="utf-8")
        (project / "README.md").write_text("# Registered project\n", encoding="utf-8")
        self.server.sessions.projects[self.project_id]["path"] = str(project)
        path = f"/api/projects/{self.project_id}/memory"
        status, listing, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual([item["path"] for item in listing["entries"]], ["README.md"])

        outside = self.root / "outside-parent"
        (outside / "project/memory-bank").mkdir(parents=True)
        (outside / "project/memory-bank/secret.md").write_text("PRIVATE REPLACEMENT MEMORY", encoding="utf-8")
        (outside / "project/README.md").write_text("PRIVATE REPLACEMENT CONTEXT", encoding="utf-8")
        # Replace an ancestor, leaving the final project component a real directory.
        # Server state remains in a separate, untouched directory under self.root.
        parent.rename(self.root / "original-parent")
        parent.symlink_to(outside, target_is_directory=True)

        status, listing, _ = self.request(path)
        self.assertEqual(status, 200)
        self.assertEqual((listing["banks"], listing["bank_id"], listing["entries"]), ([], None, []))
        status, document, _ = self.request(path + "?" + urlencode({"bank": "memory-bank", "path": "secret.md"}))
        self.assertEqual(status, 400)
        status, context, _ = self.request(f"/api/projects/{self.project_id}/context")
        self.assertEqual(status, 200)
        self.assertFalse(context["context_available"])
        self.assertTrue(all(not item["exists"] for item in context["files"]))
        self.assertNotIn("PRIVATE REPLACEMENT", json.dumps([listing, document, context]))

    def test_skill_routes_require_authenticated_preview_before_project_install(self):
        status, catalog, _ = self.request("/api/skills")
        self.assertEqual(status, 200)
        self.assertEqual({item["id"] for item in catalog["agents"]}, {"claude", "codex", "cursor"})
        source_id = catalog["sources"][0]["id"]
        installed_path = f"/api/projects/{self.project_id}/skills"
        self.assertEqual([{k: item[k] for k in ("name", "agent", "path")} for item in self.request(installed_path)[1]["installed"]], [])
        fixture = {"fixture": {"name": "Fixture skill", "description": "Offline fixture", "files": {
            "SKILL.md": b"---\nname: fixture\ndescription: Offline fixture\n---\n# Fixture\n",
        }}}
        with patch.object(self.server.skills, "_load", return_value=fixture) as load, \
                patch.object(self.server.skills, "_run", side_effect=AssertionError("Unexpected native CLI")) as native:
            self.assertEqual(self.post("/api/skills/discover", {"source_id": source_id},
                                       headers={"X-Harness-Token": None})[0], 403)
            load.assert_not_called()
            status, discovered, _ = self.post("/api/skills/discover", {"source_id": source_id})
            self.assertEqual(status, 200)
            self.assertEqual(discovered["source_id"], source_id)
            self.assertEqual(discovered["skills"][0]["id"], "fixture")
            options = {"project_id": self.project_id, "source_id": source_id,
                       "skills": ["fixture"], "agents": ["codex"]}
            status, preview, _ = self.post("/api/skills/preview", options)
            self.assertEqual(status, 200)
            self.assertTrue(preview["can_install"])
            target = ".agents/skills/fixture/SKILL.md"
            self.assertEqual(preview["files"], [{"path": target, "status": "new"}])
            self.assertEqual(list(self.project.iterdir()), [])
            install = {"preview_id": preview["preview_id"]}
            self.assertEqual(self.post("/api/skills/install", install,
                                       headers={"X-Harness-Token": "wrong"})[0], 403)
            self.assertEqual(list(self.project.iterdir()), [])
            status, result, _ = self.post("/api/skills/install", install)
            self.assertEqual(status, 200)
            self.assertTrue(result["ok"])
            self.assertEqual(result["installed"], [target])
            self.assertEqual((self.project / target).read_bytes(), fixture["fixture"]["files"]["SKILL.md"])
            self.assertEqual([{k: item[k] for k in ("name", "agent", "path")} for item in self.request(installed_path)[1]["installed"]], [
                {"name": "fixture", "agent": "codex", "path": ".agents/skills/fixture"},
            ])
            self.assertEqual(self.post("/api/skills/install", install)[0], 400)
            native.assert_not_called()
        self.assertEqual(self.server.sessions.jobs.qsize(), 0)

    def test_skill_http_routes_reject_unregistered_projects_and_extra_fields(self):
        for path, payload in (
            ("/api/skills/discover", {}),
            ("/api/skills/discover", {"source_id": []}),
            ("/api/skills/discover", {"source_id": "unknown", "extra": True}),
            ("/api/skills/preview", {"project_id": self.project_id, "source_id": "unknown",
                                    "skills": ["fixture"], "agents": ["codex"]}),
            ("/api/skills/preview", {"project_id": {}, "source_id": "unknown",
                                    "skills": ["fixture"], "agents": ["codex"]}),
            ("/api/skills/install", {"preview_id": "unknown"}),
            ("/api/skills/install", {"preview_id": []}),
            ("/api/skills/install", {"preview_id": "unknown", "project_id": self.project_id}),
        ):
            with self.subTest(path=path, payload=payload):
                self.assertEqual(self.post(path, payload)[0], 400)
        self.assertEqual(self.request("/api/projects/unregistered/skills")[0], 400)
        self.assertEqual(self.request(f"/api/projects/{self.project_id}/skills?extra=1")[0], 400)
        self.assertEqual(list(self.project.iterdir()), [])
        self.assertEqual(self.server.sessions.jobs.qsize(), 0)

    def test_skill_updates_and_removal_require_review_authentication_and_preserve_local_edits(self):
        manager = self.server.skills
        source_id = manager.catalog()['sources'][0]['id']
        old = {'fixture': {'name':'Fixture', 'description':'Fixture', 'revision':'a'*40,
                          'files':{'SKILL.md':b'old skill body\n'}}}
        new = {'fixture': {**old['fixture'], 'revision':'b'*40,
                          'files':{'SKILL.md':b'updated skill body\n'}}}
        with patch.object(manager, '_load', return_value=old):
            self.assertEqual(self.post('/api/skills/discover', {'source_id':source_id,'refresh':True})[0], 200)
        selection = {'project_id':self.project_id,'source_id':source_id,'skills':['fixture'],'agents':['codex']}
        preview = self.post('/api/skills/preview', selection)[1]
        self.assertEqual(self.post('/api/skills/install', {'preview_id':preview['preview_id']})[0], 200)
        target = self.project / '.agents/skills/fixture/SKILL.md'
        request = {'project_id':self.project_id,'agent':'codex','name':'fixture','operation':'update'}
        with patch.object(manager, '_load', return_value=new):
            self.assertEqual(self.post('/api/skills/change-preview', request, headers={'X-Harness-Token':'wrong'})[0], 403)
            self.assertEqual(self.post('/api/skills/change-preview?extra=1', request)[0], 400)
            status, preview, _ = self.post('/api/skills/change-preview', request)
        self.assertEqual(status, 200)
        self.assertTrue(preview['can_apply'])
        self.assertIn('+updated skill body', preview['files'][0]['diff'])
        self.assertEqual(target.read_bytes(), b'old skill body\n')
        apply = {'preview_id':preview['preview_id']}
        self.assertEqual(self.post('/api/skills/apply', apply, headers={'X-Harness-Token':'wrong'})[0], 403)
        self.assertEqual(self.post('/api/skills/apply?extra=1', apply)[0], 400)
        self.assertEqual(self.post('/api/skills/apply', {**apply,'extra':1})[0], 400)
        status, result, _ = self.post('/api/skills/apply', apply)
        self.assertEqual((status,result['ok']), (200,True))
        self.assertEqual(target.read_bytes(), b'updated skill body\n')
        self.assertEqual(self.post('/api/skills/apply', apply)[0], 400)
        installed = self.request(f'/api/projects/{self.project_id}/skills')[1]['installed'][0]
        self.assertEqual((installed['managed'],installed['revision'],installed['state']), (True,'b'*40,'clean'))
        target.write_bytes(b'local edit')
        request['operation'] = 'remove'
        status, preview, _ = self.post('/api/skills/change-preview', request)
        self.assertEqual((status,preview['can_apply']), (200,False))
        self.assertEqual(self.post('/api/skills/apply', {'preview_id':preview['preview_id']})[0], 400)
        self.assertEqual(target.read_bytes(), b'local edit')
        target.write_bytes(b'updated skill body\n')
        preview = self.post('/api/skills/change-preview', request)[1]
        self.assertTrue(self.post('/api/skills/apply', {'preview_id':preview['preview_id']})[1]['ok'])
        self.assertFalse(target.exists())
        self.assertEqual(self.request(f'/api/projects/{self.project_id}/skills')[1]['installed'], [])
        self.assertEqual(self.server.sessions.jobs.qsize(), 0)

    def test_create_skill_http_preview_requires_auth_and_saves_without_a_loaded_source(self):
        manager = self.server.skills
        data = {"project_id": self.project_id, "agents": ["claude"], "name": "local-notes",
                "description": 'Заметки: "project" #local',
                "instructions": "# Локальные заметки\n\nRead `AGENTS.md` first.\n"}
        path = "/api/skills/create-preview"
        self.assertEqual(manager.loaded, {})
        with patch.object(manager, "_load", side_effect=AssertionError("Unexpected source load")) as load, \
                patch.object(manager, "_run", side_effect=AssertionError("Unexpected native CLI")) as native, \
                patch("harness.skills.urlopen", side_effect=AssertionError("Unexpected download")) as download, \
                patch("harness.skills.shutil.which", return_value=None):
            self.assertEqual(self.post(path, data, headers={"X-Harness-Token": None})[0], 403)
            for payload in ({}, {**data, "source_id": "local"}, {**data, "path": "custom/SKILL.md"},
                            {**data, "project_id": {}}, {**data, "agents": []}, {**data, "name": "Bad Name"}):
                with self.subTest(payload=payload):
                    self.assertEqual(self.post(path, payload)[0], 400)
            for raw in (b"null", b"[]"):
                with self.subTest(raw=raw):
                    self.assertEqual(self.post(path, raw=raw)[0], 400)
            self.assertEqual(manager.previews, {})
            self.assertEqual(list(self.project.iterdir()), [])

            status, preview, _ = self.post(path, data)
            self.assertEqual(status, 200)
            self.assertEqual((preview["source_id"], preview["name"]), ("local", "local-notes"))
            self.assertTrue(preview["can_install"])
            target = ".claude/skills/local-notes/SKILL.md"
            self.assertEqual(preview["files"], [{"path": target, "status": "new"}])
            self.assertEqual(list(self.project.iterdir()), [])
            selection = {"preview_id": preview["preview_id"]}
            self.assertEqual(self.post("/api/skills/install", selection,
                                       headers={"X-Harness-Token": "wrong"})[0], 403)
            self.assertEqual(list(self.project.iterdir()), [])
            status, result, _ = self.post("/api/skills/install", selection)
            self.assertEqual(status, 200)
            self.assertEqual((result["ok"], result["installed"], result["unchanged"]), (True, [target], []))
            self.assertEqual((self.project / target).read_text(encoding="utf-8"), preview["content"])
            self.assertEqual([{k: item[k] for k in ("name", "agent", "path")} for item in self.request(f"/api/projects/{self.project_id}/skills")[1]["installed"]], [
                {"name": "local-notes", "agent": "claude", "path": ".claude/skills/local-notes"},
            ])
            maximum = {**data, "name": "max-payload", "description": "😃" * 1024,
                       "instructions": "\\" * 32000}
            encoded = json.dumps(maximum).encode("utf-8")
            self.assertGreater(len(encoded), 65536)
            self.assertLessEqual(len(encoded), 131072)
            status, maximum_preview, _ = self.post(path, raw=encoded)
            self.assertEqual(status, 200)
            self.assertTrue(maximum_preview["can_install"])
            expected = ('---\nname: "max-payload"\ndescription: "' + maximum["description"]
                        + '"\n---\n\n' + maximum["instructions"] + '\n')
            self.assertEqual(len(maximum_preview["content"]), len(expected))
            self.assertEqual(maximum_preview["content"], expected)
            self.assertFalse((self.project / ".claude/skills/max-payload").exists())
            previews_before = set(manager.previews)
            self.assertEqual(self.post(path, raw=b"{}", headers={"Content-Length": "131073"})[0], 400)
            self.assertEqual(self.post("/api/sessions", raw=encoded)[0], 400)
            self.assertEqual(set(manager.previews), previews_before)
            load.assert_not_called()
            native.assert_not_called()
            download.assert_not_called()
        self.assertEqual(manager.loaded, {})
        self.assertEqual(self.server.sessions.jobs.qsize(), 0)

    def test_setup_http_requires_authenticated_immutable_preview_and_installs_selected_tool_only(self):
        from harness.setup import SetupManager
        from tests.test_harness_setup import setup_source_fixture

        self.server.setup_manager.close()
        source = setup_source_fixture(self.root / "setup-source")
        self.server.setup_manager = SetupManager(self.server.sessions, source_root=source)
        options = {"project_id": self.project_id, "edition": "PHP Core", "tools": ["cursor"]}
        for changes in ({"edition": "../../invalid"}, {"edition": []}, {"project_id": str(self.root)},
                        {"project_id": {}}, {"tools": "cursor"}, {"tools": ["unknown"]}, {"extra": True}):
            with self.subTest(changes=changes):
                self.assertEqual(self.post("/api/accelerators/preview", {**options, **changes})[0], 400)
        self.assertEqual(self.request("/api/accelerators/preview", "POST", options)[0], 403)
        self.assertEqual(self.post("/api/accelerators/preview?extra=1", options)[0], 400)
        self.assertEqual(list(self.project.iterdir()), [])
        status, preview, _ = self.post("/api/accelerators/preview", options)
        self.assertEqual(status, 200)
        self.assertTrue(preview["can_install"], preview)
        self.assertTrue(preview["files"])
        self.assertEqual(preview["tools"], ["cursor"])
        self.assertEqual(list(self.project.iterdir()), [])
        selection = {"preview_id": preview["preview_id"]}
        self.assertEqual(self.request("/api/accelerators/install", "POST", selection)[0], 403)
        self.assertEqual(self.post("/api/accelerators/install", {**selection, "tools": ["claude"]})[0], 400)
        self.assertEqual(self.post("/api/accelerators/install", {"preview_id": "unknown"})[0], 400)
        self.assertEqual(self.post("/api/accelerators/install?extra=1", selection)[0], 400)
        self.assertEqual(list(self.project.iterdir()), [])
        status, result, _ = self.post("/api/accelerators/install", selection)
        self.assertEqual(status, 200)
        self.assertTrue(result["ok"], result)
        self.assertTrue((self.project / ".cursor/skills/fixture/SKILL.md").is_file())
        self.assertFalse((self.project / ".claude").exists())
        self.assertFalse((self.project / ".agents").exists())
        self.assertEqual(self.post("/api/accelerators/install", selection)[0], 400)
        status, setup, _ = self.request(f"/api/projects/{self.project_id}/setup")
        self.assertEqual(status, 200)
        self.assertTrue(setup["readiness"]["policy"])
        self.assertEqual(self.request(f"/api/projects/{self.project_id}/setup?extra=1")[0], 400)
        self.assertEqual(self.request("/api/projects/unknown/setup")[0], 400)
        self.assertEqual(self.server.sessions.list(), [])

    def test_projects_http_registration_is_strict_authenticated_and_immediately_selectable(self):
        added = self.root / "new project"
        added.mkdir()
        data = {"path": str(added)}
        initial = self.request("/api/projects")[1]["projects"]
        self.assertEqual(initial[0]["id"], self.project_id)
        self.assertTrue(initial[0]["available"])
        self.assertEqual(self.request("/api/projects", "POST", data)[0], 403)
        for body in ({}, {"path": []}, {"path": "relative"}, {"path": str(self.root / "missing")},
                     {**data, "provider": "claude"}):
            with self.subTest(body=body):
                self.assertEqual(self.post("/api/projects", body)[0], 400)
        self.assertEqual(self.post("/api/projects?extra=1", data)[0], 400)
        self.assertEqual(self.request("/api/projects?extra=1")[0], 400)
        self.assertEqual(self.request("/api/projects")[1]["projects"], initial)
        status, registered, _ = self.post("/api/projects", data)
        self.assertEqual(status, 201)
        project = registered["project"]
        self.assertEqual((project["path"], project["available"]), (str(added), True))
        self.assertEqual(len(registered["projects"]), 2)
        repeated = self.post("/api/projects", data)[1]
        self.assertEqual(repeated, registered)
        self.assertEqual(self.request("/api/bootstrap")[1]["projects"], registered["projects"])
        status, session, _ = self.post("/api/sessions", self.options(project_id=project["id"], provider="claude"))
        self.assertEqual(status, 201)
        self.assertEqual(session["session"]["project_path"], str(added))
        self.assertEqual(list(added.iterdir()), [])

    def test_project_browser_lists_searches_and_selects_without_writing(self):
        root = self.root / 'folders'
        root.mkdir()
        (root / 'client apps' / 'My Project').mkdir(parents=True)
        (root / '.hidden project').mkdir()
        (root / 'vendor' / 'another project').mkdir(parents=True)
        (root / 'project.txt').write_text('Not a directory')
        (root / 'linked project').symlink_to(self.project, target_is_directory=True)
        endpoint = '/api/projects/browse'
        body = {'path': str(root)}
        before = self.server.sessions.list_projects()
        with patch('harness.project_browser.Path.home', return_value=root):
            status, data, _ = self.post(endpoint, {})
        self.assertEqual(status, 200)
        self.assertEqual(data['path'], str(root))
        self.assertEqual(data['parent'], str(root.parent))
        self.assertEqual([x['name'] for x in data['entries']], ['client apps', 'vendor'])
        data = self.post(endpoint, {**body, 'query': 'PROJECT', 'hidden': True})[1]
        self.assertEqual([x['name'] for x in data['entries']], ['.hidden project', 'My Project'])
        self.assertEqual(data['entries'][1]['relative'], 'client apps/My Project')
        self.assertFalse(data['truncated'])
        self.assertEqual(self.server.sessions.list_projects(), before)
        self.assertEqual(self.server.sessions.list(), [])
        selected = data['entries'][1]['path']
        self.assertEqual(self.post(endpoint, {'path': selected})[1]['entries'], [])
        status, registered, _ = self.post('/api/projects', {'path': selected})
        self.assertEqual(status, 201)
        self.assertEqual(registered['project']['path'], selected)
        self.assertEqual(list(Path(selected).iterdir()), [])

    def test_project_browser_rejects_untrusted_requests_and_invalid_paths(self):
        endpoint = '/api/projects/browse'
        body = {'path': str(self.root)}
        self.assertEqual(self.request(endpoint, 'POST', body)[0], 403)
        self.assertEqual(self.post(endpoint, body, headers={'Origin': 'https://attacker.invalid'})[0], 403)
        self.assertEqual(self.post(endpoint + '?extra=1', body)[0], 400)
        link = self.root / 'link'
        link.symlink_to(self.project, target_is_directory=True)
        for extra in ({'path': None}, {'path': 'relative'}, {'path': str(self.root / '..')},
                      {'path': str(link)}, {'path': str(self.root / 'missing')},
                      {'path': str(self.root / 'invalid\x00')}, {'query': []}, {'query': 'x' * 101},
                      {'query': '\n'}, {'hidden': 'true'}, {'unknown': 1}):
            with self.subTest(extra=extra):
                self.assertEqual(self.post(endpoint, {**body, **extra})[0], 400)

    def test_project_browser_reports_scan_limits_and_inaccessible_subfolders(self):
        root = self.root / 'search'
        root.mkdir()
        (root / 'one' / 'deep match').mkdir(parents=True)
        (root / 'two').mkdir()
        endpoint = '/api/projects/browse'
        for limit in ('MAX_RESULTS', 'MAX_ENTRIES', 'MAX_SECONDS', 'MAX_DEPTH'):
            with self.subTest(limit=limit), patch('harness.project_browser.' + limit, 0):
                status, data, _ = self.post(endpoint, {'path': str(root), 'query': 'match'})
                self.assertEqual(status, 200)
                self.assertTrue(data['truncated'])
        original = web.browse_projects.__globals__['open_project_path']
        def opened(path, *args, **kwargs):
            if path.name == 'two':
                raise PermissionError('fixture')
            return original(path, *args, **kwargs)
        with patch('harness.project_browser.open_project_path', side_effect=opened):
            data = self.post(endpoint, {'path': str(root), 'query': 'match'})[1]
        self.assertEqual(data['skipped'], 1)
        self.assertEqual([x['name'] for x in data['entries']], ['deep match'])

    def test_knowledge_routes_preserve_readonly_browsing_and_authorize_lifecycle_and_export(self):
        from tests.test_harness_knowledge import install_knowledge_fixture

        base = f"/api/projects/{self.project_id}"
        status, absent, _ = self.request(base + "/knowledge")
        self.assertEqual(status, 200)
        self.assertEqual(absent["banks"], [])
        self.assertFalse(absent["runtime_available"])
        self.assertEqual(self.request(base + "/brain")[1]["entries"], [])
        self.assertEqual(list(self.project.iterdir()), [])
        install_knowledge_fixture(self.project)
        status, info, _ = self.request(base + "/knowledge?bank=memory-bank")
        self.assertEqual(status, 200)
        self.assertTrue(info["runtime_available"])
        self.assertFalse((self.project / "memory-bank/local/context.db").exists())
        for suffix in ("/knowledge?bank=memory-bank&bank=memory-bank", "/knowledge?path=README.md",
                       "/brain?path=x&path=y", "/brain?unknown=value", "/brain?path=../config/runtime.json"):
            with self.subTest(suffix=suffix):
                self.assertEqual(self.request(base + suffix)[0], 400)
        self.assertEqual(self.request("/api/projects/unknown/knowledge")[0], 400)
        data = {"action": "start", "task_id": "TASK-HTTP", "goal": "Verify the cobalt authority rule",
                "sources": ["specs/authority.md"]}
        self.assertEqual(self.request(base + "/knowledge", "POST", data)[0], 403)
        self.assertEqual(self.post(base + "/knowledge", {**data, "extra": True})[0], 400)
        self.assertEqual(self.post(base + "/knowledge?bank=memory-bank", data)[0], 400)
        self.assertFalse((self.project / "memory-bank/local/context.db").exists())
        status, started, _ = self.post(base + "/knowledge", data)
        self.assertEqual(status, 200)
        self.assertTrue(started["ok"], started)
        record_id = started["result"]["task_uuid"]
        listing = self.request(base + "/brain")[1]
        entry = next(item for item in listing["entries"] if item.get("id") == record_id and item.get("type") == "task")
        status, document, _ = self.request(base + "/brain?" + urlencode({"bank": "memory-bank", "path": entry["path"]}))
        self.assertEqual(status, 200)
        self.assertEqual(document["metadata"]["revision"], 1)
        update = {"action": "brain-update", "record_id": record_id, "revision": 1,
                  "progress": "Canonical source verified"}
        status, changed, _ = self.post(base + "/knowledge", update)
        self.assertEqual(status, 200)
        self.assertTrue(changed["ok"], changed)
        status, stale, _ = self.post(base + "/knowledge", update)
        self.assertEqual(status, 200)
        self.assertFalse(stale["ok"])
        self.assertIn("revision", stale["error"].lower())
        status, exported, _ = self.post(base + "/knowledge", {"action": "export"})
        self.assertEqual(status, 200)
        self.assertTrue(exported["ok"], exported)
        download = "/api/knowledge/exports/" + exported["download_id"]
        status, payload, headers = self.request(download)
        self.assertEqual(status, 200)
        self.assertTrue(payload.startswith(b"PK"))
        self.assertEqual(headers["Content-Type"], "application/zip")
        self.assertIn("project-knowledge.zip", headers["Content-Disposition"])
        self.assertEqual(headers["Cache-Control"], "no-store")
        self.assertEqual(self.request(download + "?unknown=value")[0], 400)
        self.assertEqual(self.request("/api/knowledge/exports/unknown")[0], 400)
        self.assertEqual(self.server.sessions.list(), [])
        self.assertEqual(self.server.sessions.jobs.qsize(), 0)

    def test_linked_task_context_routes_require_current_receipt_and_keep_task_identity_bound(self):
        from tests.test_harness_knowledge import install_knowledge_fixture

        install_knowledge_fixture(self.project)
        data = {"project_id": self.project_id, "provider": "claude", "prompt": "Verify the cobalt rule",
                "brain": {"bank": "memory-bank", "task_id": "TASK-SESSION-HTTP", "query": "cobalt allocation",
                          "create": True, "goal": "Verify the cobalt allocation rule", "review": True}}
        for extra in ({"capsule": {}}, {"approved": True}, {"context_id": "injected"}):
            with self.subTest(extra=extra):
                self.assertEqual(self.post("/api/sessions", {**data, "brain": {**data["brain"], **extra}})[0], 400)
        self.assertEqual(self.server.sessions.list(), [])
        status, created, _ = self.post("/api/sessions", data)
        self.assertEqual(status, 201)
        store = self.server.sessions
        sid = created["session"]["id"]
        base = "/api/sessions/" + sid
        # This HTTP fixture deliberately has no worker. Run only the stdlib
        # prepare stage synchronously; provider dispatch remains queued.
        prepared = store._task_context().prepare(store.get(sid))
        store._save_brain(sid, prepared)
        store._status(sid, "awaiting_context")
        current = store.get(sid)
        queue_size = store.jobs.qsize()
        for action, body in (("run", {"context_id": prepared["context_id"]}),
                             ("context", {"query": "cobalt rule"}),
                             ("brain", {"action": "rebind"}),
                             ("memory", {"progress": "Unreviewed"})):
            self.assertEqual(self.request(base + "/" + action, "POST", body)[0], 403)
        for body in ({"progress": "x", "unexpected": True}, {"learnings": [{"type": "task"}]}, {}):
            with self.subTest(memory=body):
                self.assertEqual(self.post(base + "/memory", body)[0], 400)
        for body in ({}, {"context_id": "different"}, {"context_id": prepared["context_id"], "capsule": {}},
                     {"context_id": prepared["context_id"], "approved": True}):
            with self.subTest(run=body):
                self.assertEqual(self.post(base + "/run", body)[0], 400)
        self.assertEqual(self.post(base + "/context", {"query": "cobalt", "task_id": "OTHER"})[0], 400)
        self.assertEqual(self.post(base + "/context", {"query": []})[0], 400)
        self.assertEqual(self.post(base + "/run?extra=1", {"context_id": prepared["context_id"]})[0], 400)
        self.assertEqual(store.get(sid), current)
        self.assertEqual(store.jobs.qsize(), queue_size)
        status, info, _ = self.request(base + "/brain")
        self.assertEqual(status, 200)
        self.assertEqual(info["task"]["id"], prepared["task"]["id"])
        self.assertEqual(info["context_id"], prepared["context_id"])
        self.assertEqual(self.request(base + "/brain?path=anything")[0], 400)
        status, refreshed, _ = self.post(base + "/context", {"query": "cobalt allocation owner"})
        self.assertEqual(status, 200)
        self.assertFalse(refreshed["session"]["brain"]["context_id"])
        self.assertEqual(refreshed["session"]["brain"]["query"], "cobalt allocation owner")
        prepared_again = store._task_context().prepare(store.get(sid))
        store._save_brain(sid, prepared_again)
        store._status(sid, "awaiting_context")
        self.assertEqual(self.post(base + "/run", {"context_id": prepared["context_id"]})[0], 400)
        status, approved, _ = self.post(base + "/run", {"context_id": prepared_again["context_id"]})
        self.assertEqual(status, 200)
        self.assertEqual(approved["session"]["status"], "queued")
        self.assertTrue(approved["session"]["brain"]["approved"])
        store._status(sid, "completed")
        for changed in ({"task_id": "OTHER"}, {"bank": "PHP Core/memory-bank"}, {"result": "unreviewed"}):
            with self.subTest(completion=changed):
                self.assertEqual(self.post(base + "/brain", {"action": "complete",
                    "revision": prepared_again["task"]["revision"], "outcome": "Explicit verified result", **changed})[0], 400)
        status, result, _ = self.post(base + "/brain", {"action": "complete",
            "revision": prepared_again["task"]["revision"], "outcome": "Explicit verified result",
            "verification": ["Offline HTTP fixture"]})
        self.assertEqual(status, 200)
        self.assertTrue(result["ok"], result)
        info = self.request(base + "/brain")[1]
        self.assertEqual(info["task"]["status"], "completed")

    def test_authenticated_shutdown_stops_http_and_releases_owned_resources(self):
        catalog = Path(self.server.catalog_dir.name)
        status, data, _ = self.post("/api/shutdown", {})
        self.assertEqual((status, data), (200, {"ok": True}))
        self.thread.join(2)
        self.assertFalse(self.thread.is_alive())
        self.close_server()
        self.assertEqual(self.server.socket.fileno(), -1)
        self.assertFalse(self.server.sessions.worker.is_alive())
        self.assertFalse(catalog.exists())
        with self.assertRaises(sqlite3.ProgrammingError):
            self.server.sessions.db.execute("SELECT 1")

    def test_start_resolves_executable_paths_before_the_daemon_changes_directory(self):
        arguments = ["harness-server", "start", "--state-dir", str(self.root / "cli-state"),
                     "--codex-bin", "./tools/codex", "--claude-bin", "claude-fixture"]
        previous = Path.cwd()
        try:
            os.chdir(self.project)
            with patch.object(sys, "argv", arguments), patch.object(
                web, "running", side_effect=[None, {"port": 12345}]
            ), patch.object(web.subprocess, "Popen") as spawn, contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(web.main(), 0)
            command = spawn.call_args.args[0]
            self.assertEqual(command[command.index("--codex-bin") + 1], str(self.project / "tools/codex"))
            self.assertEqual(command[command.index("--claude-bin") + 1], "claude-fixture")
            self.assertEqual(command[command.index("--project") + 1], str(self.project))
            self.assertEqual(spawn.call_args.kwargs["cwd"], web.ROOT)
        finally:
            os.chdir(previous)


ROOT = Path(__file__).resolve().parents[1]
# The harness serves the Kit 3 catalog in a same-origin frame, so both pages share one theme contract.
THEMED_PAGES = (ROOT / "harness/web/index.html", ROOT / "install/open-source-kit/web/index.html")
COLOR_LITERAL = re.compile(r"#[0-9a-fA-F]{3,8}\b|(?<![\w-])(?:white|black)(?![\w-])"
                           r"|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color-mix)\(")


class HarnessThemeTests(unittest.TestCase):
    def run_node(self, script):
        result = subprocess.run([shutil.which("node"), "-e", script], capture_output=True, text=True)
        if result.returncode:
            self.fail(result.stderr)

    def test_stylesheets_take_every_color_from_matching_light_and_dark_tokens(self):
        for page in THEMED_PAGES:
            with self.subTest(page=page.relative_to(ROOT).as_posix()):
                html = page.read_text(encoding="utf-8")
                css = stylesheet(page)
                light = re.search(r"\n\s*:root \{([^}]*)\}", css).group(1)
                dark = re.search(r'\n\s*:root\[data-theme="dark"\] \{([^}]*)\}', css).group(1)
                tokens = set(re.findall(r"(--[\w-]+)\s*:", light))
                self.assertEqual(tokens, set(re.findall(r"(--[\w-]+)\s*:", dark)),
                                 "every token needs a light and a dark value")
                self.assertEqual(COLOR_LITERAL.findall(css.replace(light, "").replace(dark, "")), [],
                                 "add a token to both :root sets instead of a literal color")
                defined = set(re.findall(r"(--[\w-]+)\s*:", " ".join(re.findall(r":root[^{]*\{([^}]*)\}", css))))
                runtime = set(re.findall(r"setProperty\('(--[\w-]+)'", html + ui_script()))
                self.assertLessEqual(set(re.findall(r"var\((--[\w-]+)", css)) - runtime, defined)
        switch = re.findall(r'<input type="radio" name="theme" value="(\w+)">', THEMED_PAGES[0].read_text(encoding="utf-8"))
        self.assertEqual(switch, ["system", "light", "dark"])

    def test_stylesheets_take_type_and_shape_from_the_scale_and_space_from_the_grid(self):
        for page in THEMED_PAGES:
            with self.subTest(page=page.relative_to(ROOT).as_posix()):
                rules = re.sub(r":root[^{]*\{[^}]*\}", "", stylesheet(page))
                for prop in ("font-size", "line-height", "font-weight", "border-radius", "font"):
                    literal = [value for value in re.findall(rf"(?<![\w-]){prop}:\s*([^;}}]+)", rules)
                               if any(unit != "%" and float(number) for number, unit in
                                      re.findall(r"(?<![\w-])(\d*\.?\d+)(px|em|rem|%)?(?!\w)", re.sub(r"var\([^)]*\)", "", value)))]
                    self.assertEqual(literal, [], f"{prop} must come from a scale token")
                spacing = re.findall(r"(?<![\w-])(?:padding|margin|gap|row-gap|column-gap)(?:-[a-z]+)?:\s*([^;}]+)", rules)
                off_grid = sorted({px for value in spacing for px in re.findall(r"(\d+(?:\.\d+)?)px", value)
                                   if float(px) not in (1, 2, 4, 8, 12, 16, 24, 32, 48, 64)})
                self.assertEqual(off_grid, [], "spacing must sit on the 4px grid")

    def test_motion_comes_from_tokens_and_reduced_motion_stops_it_everywhere(self):
        for page in THEMED_PAGES:
            with self.subTest(page=page.relative_to(ROOT).as_posix()):
                css = stylesheet(page)
                rules = re.sub(r":root[^{]*\{[^}]*\}", "", css)
                motion = re.findall(r"(?<![\w-])(?:transition|animation)(?:-[a-z]+)?:\s*([^;}]+)", rules)
                self.assertTrue(motion)
                literal = [value for value in motion if "cubic-bezier(" in value
                           or re.search(r"(?<![\w.-])\d*\.?\d+m?s\b", re.sub(r"var\([^)]*\)", "", value))]
                self.assertEqual(literal, [], "durations and easings come from the motion tokens")
                # Pseudo-elements animate too (the agents panel's live dot), so reduced motion must name them.
                reduced = re.search(r"@media \(prefers-reduced-motion:reduce\) \{ ([^{]+)\{([^}]*)\}", css)
                self.assertIsNotNone(reduced)
                self.assertEqual({part.strip() for part in reduced.group(1).split(",")}, {"*", "*::before", "*::after"})
                self.assertIn("animation:none !important", reduced.group(2))
                self.assertIn("transition:none !important", reduced.group(2))

    def test_memory_colors_read_as_marks_on_both_surfaces_in_both_themes(self):
        def luminance(color):
            channels = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
            return sum(weight * (c / 12.92 if c <= .04045 else ((c + .055) / 1.055) ** 2.4)
                       for weight, c in zip((.2126, .7152, .0722), channels))
        def contrast(a, b):
            high, low = sorted((luminance(a), luminance(b)), reverse=True)
            return (high + .05) / (low + .05)
        css = stylesheet(THEMED_PAGES[0])
        for block in (re.search(r"\n\s*:root \{([^}]*)\}", css).group(1), re.search(r':root\[data-theme="dark"\] \{([^}]*)\}', css).group(1)):
            tokens = dict(re.findall(r"(--[\w-]+):(#[0-9a-f]{6})\b", block))
            tokens.setdefault("--paper", "#ffffff")
            marks = {name: value for name, value in tokens.items() if name.startswith(("--mem-", "--ctx-"))}
            self.assertEqual(set(marks), {"--mem-rules", "--mem-bank", "--mem-brain", "--mem-auto", "--ctx-rest", "--ctx-added"})
            for name, value in marks.items():
                for surface in ("--paper", "--soft"):
                    with self.subTest(token=name, surface=surface, paper=tokens["--paper"]):
                        self.assertGreaterEqual(contrast(value, tokens[surface]), 3, "chart marks need 3:1 (WCAG 1.4.11)")

    @unittest.skipUnless(shutil.which("node"), "Theme script check requires Node")
    def test_saved_theme_applies_before_first_paint_in_the_harness_and_its_catalog(self):
        heads = []
        for page in THEMED_PAGES:
            head = page.read_text(encoding="utf-8").split("</head>", 1)[0]
            heads.append(head[head.index("<script>") + len("<script>"):head.index("</script>")])
        self.run_node("const assert = require('node:assert/strict');\nconst heads = " + json.dumps(heads) + """;
for (const saved of [null,'light','dark','sepia','denied']) for (const systemDark of [false,true]) {
  const themes = heads.map(source => {
    const root = {dataset:{}};
    const localStorage = {getItem(key) { assert.equal(key,'harness.theme.v1'); if (saved === 'denied') throw new Error('denied'); return saved; }};
    const window = {matchMedia: () => ({matches:systemDark, addEventListener() {}}), addEventListener() {}};
    new Function('window','document','localStorage',source)(window,{documentElement:root},localStorage);
    return root.dataset.theme;
  });
  const expected = saved === 'light' || saved === 'dark' ? saved : systemDark ? 'dark' : 'light';
  assert.deepEqual(themes,[expected,expected],`saved ${saved}, system ${systemDark ? 'dark' : 'light'}`);
}""")

    @unittest.skipUnless(shutil.which("node"), "Theme script check requires Node")
    def test_theme_switch_saves_the_choice_and_follows_the_system_and_other_tabs(self):
        page = ui_script()
        start = page.index("\nconst themeKey = ") + 1
        source = page[start:page.index("\napplyTheme();\n", start) + len("\napplyTheme();\n")]
        self.run_node("const assert = require('node:assert/strict');\nconst source = " + json.dumps(source) + """;
function boot({saved = null, dark = false, denied = false} = {}) {
  const env = {stored:saved === null ? {} : {'harness.theme.v1':saved}, denied, listeners:{}, root:{dataset:{}}};
  env.system = {matches:dark, addEventListener: (type,listener) => { env.listeners.system = listener; }};
  env.radios = ['system','light','dark'].map(value => ({value, checked:false, addEventListener(type,listener) { this.change = listener; }}));
  const localStorage = {
    getItem: key => { if (env.denied) throw new Error('denied'); return key in env.stored ? env.stored[key] : null; },
    setItem: (key,value) => { if (env.denied) throw new Error('denied'); env.stored[key] = String(value); },
    removeItem: key => { if (env.denied) throw new Error('denied'); delete env.stored[key]; },
  };
  const window = {matchMedia: () => env.system, addEventListener: (type,listener) => { env.listeners[type] = listener; }};
  new Function('window','document','localStorage',source)(window,{documentElement:env.root, querySelectorAll: () => env.radios},localStorage);
  env.choose = value => { const radio = env.radios.find(r => r.value === value); radio.checked = true; radio.change(); };
  env.checked = () => env.radios.filter(r => r.checked).map(r => r.value);
  env.flip = matches => { env.system.matches = matches; env.listeners.system(); };
  return env;
}
let env = boot();
assert.equal(env.root.dataset.theme,'light'); assert.deepEqual(env.checked(),['system']);
env.flip(true); assert.equal(env.root.dataset.theme,'dark');
env.choose('light'); assert.equal(env.stored['harness.theme.v1'],'light'); assert.equal(env.root.dataset.theme,'light'); assert.deepEqual(env.checked(),['light']);
env.flip(false); env.flip(true); assert.equal(env.root.dataset.theme,'light');
env.stored['harness.theme.v1'] = 'dark'; env.listeners.storage({key:'harness.theme.v1'});
assert.equal(env.root.dataset.theme,'dark'); assert.deepEqual(env.checked(),['dark']);
env.choose('system'); assert.equal('harness.theme.v1' in env.stored,false); assert.equal(env.root.dataset.theme,'dark');
env = boot({saved:'sepia', dark:true});
assert.equal(env.root.dataset.theme,'dark'); assert.deepEqual(env.checked(),['system']);
env = boot({saved:'light', dark:true, denied:true});
assert.equal(env.root.dataset.theme,'dark'); assert.deepEqual(env.checked(),['system']);
env.choose('light'); assert.equal(env.root.dataset.theme,'light'); assert.deepEqual(env.checked(),['light']);""")


@unittest.skipUnless(shutil.which("node"), "Page helper checks require Node")
class PageHelperTests(unittest.TestCase):
    def run_node(self, source, checks):
        result = subprocess.run([shutil.which("node"), "-e", "const assert = require('node:assert/strict');\n" + source + checks],
                                capture_output=True, text=True)
        if result.returncode:
            self.fail(result.stderr)

    def test_numbers_follow_one_grammar_and_unknown_is_never_zero(self):
        page = ui_script()
        self.run_node(page[page.index("\nconst numberText"):page.index("\nfunction numberNode(")], r"""
assert.deepEqual(fmt.exact(76450), {text: '76,450', label: '76,450'});
assert.deepEqual(fmt.exact(Number.NaN), {text: '—', label: 'not reported'});
assert.deepEqual(fmt.estimate(4123), {text: '≈ 4.1k', label: 'about 4,100'});
assert.deepEqual([fmt.estimate(520.4).text, fmt.estimate(53210).text, fmt.estimate(1234567).text, fmt.estimate(1988).text, fmt.estimate(149e3).text],
  ['≈ 520', '≈ 53k', '≈ 1.2M', '≈ 2.0k', '≈ 150k']);
assert.deepEqual([fmt.atMost(4000).text, fmt.atLeast(12345).text], ['≤ 4k', '≥ 12k']);
assert.deepEqual(fmt.atMost(4100), {text: '≤ 4.1k', label: 'at most 4,100'});
assert.deepEqual([fmt.atLeast(13).text, fmt.capped(200).text, fmt.capped(200).label], ['≥ 13', '200+', 'more than 200']);
assert.deepEqual(fmt.cost(.3), {text: '≈ $0.30', label: 'about 0.30 US dollars'});
assert.deepEqual([fmt.cost(4.8312).text, fmt.cost(.0123).text, fmt.cost(.123).text], ['≈ $4.83', '≈ $0.0123', '≈ $0.123']);
for (const unknown of [null, undefined, -1, Infinity]) assert.equal(fmt.cost(unknown).text, '—');
assert.equal(fmt.unknown('not recorded').label, 'not recorded');
""")

    def test_capsule_meter_reads_exact_counts_and_marked_estimates(self):
        page = ui_script()
        parts = [page[page.index(start):page.index(end)] for start, end in (
            ("\nconst el = (tag", "\n// Motion follows"), ("\nconst numberText", "\n// Keeps one node per key"),
            ("\nconst capsuleKinds", "\nfunction renderLinkedSession("))]
        self.run_node(r"""
class Node { constructor() { this.children = []; this.dataset = {}; this.attributes = {}; this.style = {}; this.hidden = false; this.own = ''; }
  get textContent() { return this.children.length ? this.children.map(child => typeof child === 'string' ? child : child.textContent).join('') : this.own; }
  set textContent(value) { this.children = []; this.own = String(value); }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.own = ''; this.children = nodes; }
  querySelector(selector) { return this.children.find(child => selector === `[data-kind="${child.dataset.kind}"]`); } }
const document = {createElement: () => new Node()}, nodes = {}, $ = id => nodes[id] ??= new Node();
const state = {pending: null, bootstrap: {runtime: {context_excerpt_bytes: 3000}}};
for (const kind of ['brain', 'bank', 'rules']) { const part = new Node(); part.dataset.kind = kind; $('capsule-bar').append(part); }
""" + "".join(parts), r"""
const part = kind => $('capsule-bar').querySelector(`[data-kind="${kind}"]`);
renderCapsuleMeter({characters: 7650, limit: 8000, kinds: {brain: 2592, bank: 4296, rules: 762}, items: {brain: 2, bank: 3, rules: 1},
  repeats: 1448, dropped: {semantic: 2, episodic: 1}, prompt_characters: 7946});
assert.equal($('capsule-meter').hidden, false);
assert.equal($('capsule-facts').textContent, '7,650 of 8,000 characters · 2 semantic items dropped to fit · 1 episodic item dropped to fit · 350 characters left');
assert.equal($('capsule-repeats').textContent, 'Each item appears in 3 views: 1,448 characters repeat');
assert.deepEqual($('capsule-legend').children.map(entry => entry.textContent), ['Project Brain 2,592', 'Memory bank 4,296', 'Rules & docs 762']);
assert.deepEqual(['brain', 'bank', 'rules'].map(kind => part(kind).style.width), ['32.4%', '53.7%', '9.525%']);
assert.equal(part('bank').title, 'Memory bank · 3 items · 4,296 characters');
assert.equal($('capsule-bar').attributes['aria-label'], 'Capsule 7,650 of 8,000 characters: Project Brain 2,592, Memory bank 4,296, Rules and docs 762; 2 semantic items dropped to fit, 1 episodic item dropped to fit; 1,448 characters repeat.');
const cost = $('linked-context-cost').children;
assert.deepEqual([cost[0], cost[1].children[0].textContent, cost[1].children[0].attributes['aria-hidden'], cost[1].children[1].textContent, cost[2]],
  ['Adds ', '≈ 2.2k tokens', 'true', 'about 2,200 tokens', ' to every turn.']);
renderCapsuleMeter({characters: 900, limit: 8000, kinds: {brain: 900, bank: 0, rules: 0}, items: {brain: 0, bank: 0, rules: 0},
  repeats: 0, dropped: {}, prompt_characters: 1000});
assert.equal($('capsule-facts').textContent, '900 of 8,000 characters · No matching memory for this query.');
assert.deepEqual([$('capsule-repeats-line').hidden, part('bank').hidden, part('rules').hidden, $('capsule-legend').children.length], [true, true, true, 1]);
renderCapsuleMeter(null);
assert.equal($('capsule-meter').hidden, true);
""")

    def test_memory_use_model_counts_the_window_and_what_changed_since_the_last_visit(self):
        page = ui_script()
        self.run_node(page[page.index("\nconst numberText"):page.index("\nfunction numberNode(")]
                      + "\nconst memoryKinds = [['brain','Project Brain'],['bank','Memory bank'],['rules','Rules & docs']];"
                      + page[page.index("\nconst DAY_MS"):page.index("\nconst memoryAnchorKey")]
                      + page[page.index("\nconst memoryFilters"):page.index("\nconst memoryMatches")], r"""
const chunk = (id, fields) => ({id, title:id, type:'domain', status:'active', created:'2025-10-01', last_verified:'2025-10-01', review_after:'2026-12-01',
  valid_to:null, promoted:false, auto:false, bytes:100, sources:1, sources_changed:false, path:`chunks/${id}.md`, ...fields});
const data = {today:'2026-10-04',
  chunks:{bytes:600, truncated:false, items:[chunk('A',{review_after:'2026-10-01'}), chunk('B',{auto:true, promoted:true, created:'2026-09-30', last_verified:'2026-09-30', review_after:'2027-09-30'}),
    chunk('C',{auto:true, promoted:true, last_verified:'2026-09-02', review_after:'2027-09-02'}), chunk('D',{status:'needs-review'}),
    chunk('E',{status:'superseded', valid_to:'2026-09-20'}), chunk('F',{review_after:'2026-10-20', sources_changed:true})]},
  brain:{truncated:false, items:[{type:'task', status:'active', archived:false, open:true, resolved_at:null, promotable:false, promoted:false},
    {type:'finding', status:'resolved', archived:false, open:false, resolved_at:'2026-09-25T08:00:00+00:00', promotable:true, promoted:false},
    {type:'finding', status:'resolved', archived:true, open:false, resolved_at:'2026-09-29T08:00:00+00:00', promotable:true, promoted:true},
    {private:true, archived:true, open:false, resolved_at:'2026-01-02T08:00:00+00:00', promotable:true, promoted:false}]},
  promotions:{truncated:false, items:[{status:'applied', mode:'automatic', created_at:'2026-09-29T09:00:00+00:00', applied_at:'2026-09-30T09:00:00+00:00'},
    {status:'applied', mode:'human', created_at:'2026-08-01T09:00:00+00:00', applied_at:'2026-09-15T09:00:00+00:00'},
    {status:'proposed', mode:'human', created_at:'2026-10-01T09:00:00+00:00', applied_at:null}, {status:'reviewed', mode:'automatic', created_at:'2026-10-02T09:00:00+00:00', applied_at:null}]},
  retrievals:{found:3, merged:1, limit:200, items:[{at:'2026-09-27T10:00:00+00:00', route:'Claude hook', task:0, brain:2, bank:2, rules:3, chunks:['A','B'], cuts:[]},
    {at:'2026-10-02T10:00:00+00:00', route:'Harness', task:1, brain:1, bank:1, rules:2, chunks:['B'], cuts:[['C','layer-limit']]},
    {at:'2026-10-03T10:00:00+00:00', route:'not recorded', task:1, brain:1, bank:0, rules:2, chunks:[], cuts:[]}]},
  health:{truncated:false, items:[{at:'2026-09-20T10:00:00+00:00', dropped:true}, {at:'2026-09-27T10:00:00+00:00', dropped:true}, {at:'2026-10-02T10:00:00+00:00', dropped:false}, {at:'2026-10-03T10:00:00+00:00', dropped:null}]},
  history:{horizon:120, days:[{day:'2026-07-01', retrievals:40, with_chunk:6, chunks:{F:2}}, {day:'2026-09-28', retrievals:12, with_chunk:2, chunks:{A:1, B:1}},
    {day:'2026-10-02', retrievals:9, with_chunk:1, chunks:{B:1}}]}};
const now = Date.parse('2026-10-04T12:00:00Z'), model = memoryUseModel(data, 30, now);
// Trouble the tab counts: one chunk past review, one citing a changed file, one stalled automatic promotion.
assert.equal(model.count, 3);
assert.deepEqual([model.bank.active, model.bank.person, model.bank.auto, model.bank.reattested, model.bank.drafts, model.bank.superseded],[4, 2, 2, 1, 1, 1]);
assert.deepEqual([model.bank.created, model.bank.retiredInWindow, model.bank.pastReview.map(c => c.id), model.bank.due.map(c => c.id)],[1, 1, ['A'], ['F']]);
assert.deepEqual([model.brain.total, model.brain.open, model.brain.archived, model.brain.private, model.brain.resolved, model.brain.notPromoted],[4, 1, 2, 1, 2, 1]);
assert.deepEqual([model.promotion.proposed, model.promotion.applied, model.promotion.auto, model.promotion.human, model.promotion.waiting.length, model.promotion.stalled.length],[3, 2, 1, 1, 1, 1]);
const selected = model.selected;
assert.deepEqual([selected.withChunk, selected.selections, selected.distinct, selected.reused, selected.cutTotal, selected.kinds],[2, 3, 2, 1, 1, {brain:4, bank:3, rules:7}]);
// Dropped-to-fit counts only health lines inside the retrieval window, and an unmeasured line is not a zero.
assert.deepEqual([selected.dropped, selected.measured, selected.merged],[1, 2, 1]);
assert.deepEqual([model.review(data.chunks.items[0]), model.review(data.chunks.items[1])],[-3, 361]);
const week = memoryUseModel(data, 7, now);
assert.deepEqual([week.promotion.applied, week.brain.resolved, week.bank.created, week.bank.retiredInWindow],[1, 1, 1, 0]);
assert.equal(memoryUseModel(data, 0, now).promotion.applied, 2);
assert.deepEqual([bandWidth(0), bandWidth(1), bandWidth(3), bandWidth(10), bandWidth(11), bandWidth(41), bandWidth(51)],[1, 2, 4, 8, 12, 16, 24]);
assert.deepEqual([reviewLabel(-3), reviewLabel(0), reviewLabel(344), reviewLabel(null)],['3 d overdue', 'today', 'in 344 d', '—']);
const anchor = {at:'2026-09-28T16:40:00.000Z', today:'2026-09-28', newest:'2026-09-30T00:00:00+00:00',
  chunks:{A:['active','2025-10-01','2026-10-01'], C:['active','2025-10-01','2026-09-29'], D:['needs-review','2025-10-01','2026-12-01'], E:['active','2025-10-01','2026-12-01'], F:['active','2025-10-01','2026-10-20']},
  promotions:{'2026-09-29T09:00:00+00:00|automatic':'reviewed', '2026-08-01T09:00:00+00:00|human':'applied'}};
const changes = memoryChanges(data, anchor), ids = list => list.map(item => item.id);
assert.deepEqual([ids(changes.added), ids(changes.retired), ids(changes.reattested), ids(changes.moved), ids(changes.crossed)],[['B'], ['E'], ['C'], ['C'], ['A']]);
assert.deepEqual([changes.applied.map(item => item.mode), changes.retrievals.length, changes.firstNew],[['automatic'], 2, 1]);
assert.equal(memoryChanges(data, null), null);
const snapshot = memorySnapshot(data);
assert.deepEqual([snapshot.today, snapshot.newest, snapshot.chunks.A, Object.keys(snapshot.promotions).length],['2026-10-04', '2026-10-03T10:00:00+00:00', ['active','2025-10-01','2026-10-01'], 4]);
assert.deepEqual(memoryChanges(data, snapshot), {since:snapshot.at, added:[], retired:[], reattested:[], moved:[], crossed:[], applied:[], retrievals:[], firstNew:3});
// The kept history counts inside the flow window; per-chunk counts span everything kept.
assert.deepEqual([model.history.retrievals, model.history.withChunk, model.history.days.length, model.history.since, model.history.chunks.get('F'), model.history.chunks.get('B')],
  [21, 3, 2, '2026-07-01', 2, 2]);
assert.deepEqual([memoryUseModel(data, 0, now).history.retrievals, memoryUseModel(data, 7, now).history.retrievals, memoryUseModel(data, 3, now).history.retrievals], [61, 21, 9]);
const never = memoryFilters.find(([id]) => id === 'never')[2];
// F was never in the last retrievals, but the kept history saw it selected.
assert.deepEqual(['A', 'C', 'F'].map(id => never(data.chunks.items.find(item => item.id === id), model)), [false, true, false]);
""")

    def test_context_turns_split_exact_fill_from_estimated_memory_and_bound_earlier_copies(self):
        page = ui_script()
        source = page[page.index("// Sessions › Usage › Context"):page.index("\n$('context-meter').addEventListener")]
        self.run_node(page[page.index("\nconst numberText"):page.index("\nfunction numberNode(")]
                      + page[page.index("\nconst plural"):page.index("\nconst reasonLabel")]
                      + "\nconst window = {matchMedia: () => ({matches: false, addEventListener() {}})};"
                      + "\nconst el = (tag, className, text) => ({tag, className, textContent: text});\n" + source, r"""
const ledger = {message: 388, instructions: 300, total: 18000, attachments: {characters: 214, count: 1},
  capsule: {characters: 6895, inserted: 7157, kinds: {brain: 2409, bank: 1858, rules: 2890}, repeats: 3121, dropped: {semantic: 2}, items: {brain: 1, bank: 1, rules: 2}},
  excerpts: [{name: 'AGENTS.md', sent: 3000, full: 13998, characters: 3000}, {name: 'README.md', sent: 3000, full: 18245, characters: 3000},
             {name: 'project-brain/README.md', sent: 3000, full: 5919, characters: 3000}, {name: 'specs/MANIFEST.md', sent: 871, full: 871, characters: 871}]};
const launch = (id, fill, extra = {}) => ({id, kind: 'native', status: 'completed', started_at: '2026-10-03T15:05:00+00:00', settings: {provider: 'claude'},
  context: fill === undefined ? null : {version: 1, provider: 'claude', agents: 1, ledger, fill, hooks: {installed: true, measured: true, bytes: null}, cli_files: []}, ...extra});
const turns = contextTurns([
  launch('t1'),
  launch('t2', {start: 31840, end: 61870, peak: 61870, calls: 18, window: 200000, compactions: [], cache_share: .88, hooks: null}),
  launch('t3', {start: 66410, end: 52800, peak: 171900, calls: 31, window: 200000, compactions: [{pre: 171900, post: 29400, call: 24}], cache_share: .93, hooks: null}),
  launch('t4', {start: 57190, end: 76450, peak: 76450, calls: 14, window: 200000, compactions: [], cache_share: .91, hooks: null}),
  {id: 'fleet', kind: 'fleet', status: 'completed', started_at: '2026-10-03T15:10:00+00:00', settings: {provider: 'claude'}, context: {ledger, agents: 5, fill: null}},
  launch('t5', {start: 80960, end: 106600, peak: 106600, calls: 11, window: 200000, compactions: [], cache_share: .94, hooks: null}, {status: 'running'})]);
assert.deepEqual(turns.map(turn => turn.ordinal), [1, 2, 3, 4, 5]);
const t4 = turns[3];
// Memory is estimated from characters: capsule ÷ 3.6, excerpts ÷ 4.7, summed before rounding.
assert.equal(Math.round(t4.memory), 4088);
assert.deepEqual([fmt.estimate(t4.memory).text, fmt.estimate(t4.parts.brain).text, fmt.estimate(t4.parts.bank).text, fmt.estimate(t4.parts.rules).text],
  ['≈ 4.1k', '≈ 670', '≈ 520', '≈ 2.9k']);
assert.deepEqual([t4.added, t4.free, fmt.estimate(t4.rest).text, percentText(t4.end, t4.window), percentText(t4.memory, t4.end, true)],
  [19260, 123550, '≈ 53k', '38%', '≈ 5%']);
assert.deepEqual([t4.characters.rules, t4.calls, percentText(t4.cache, 1)], [2890 + 9871, 14, '91%']);
const geometry = contextGeometry(t4);
assert.equal(Math.round(geometry.brain + geometry.bank + geometry.rules + geometry.rest + geometry.added), 76450);
// A compacted turn draws its final fill unsplit and has no growth.
const t3 = turns[2];
assert.deepEqual([t3.compacted, t3.added, contextGeometry(t3), t3.free, percentText(t3.end, t3.window), percentText(t3.peak, t3.window)],
  [true, null, {added: 52800}, 147200, '26%', '86%']);
// Earlier copies: bounded by the memory of turns since the last compaction; an unrecorded turn makes it unknown.
assert.deepEqual(earlierMemory(turns, 4), {total: t4.memory, from: 4});
assert.deepEqual(earlierMemory(turns, 3), {total: 0, from: 3});
assert.deepEqual(earlierMemory(turns, 1), {unknown: 1});
assert.deepEqual([turns[0].recorded, turns[0].end, contextGeometry(turns[0])], [false, null, null]);
assert.equal(turns[4].running, true);
const unknown = contextTurn(launch('cursor', {start: null, end: null, peak: null, calls: null, window: null, compactions: [], cache_share: null, hooks: null}), 6);
assert.deepEqual([unknown.recorded, unknown.end, unknown.rest, unknown.free, contextGeometry(unknown)], [true, null, null, null, null]);
// Memory that exceeds the reported start leaves the remainder unknown rather than negative.
assert.equal(contextTurn(launch('small', {start: 3000, end: 3500, peak: 3500, calls: 1, window: 200000, compactions: [], cache_share: null, hooks: null}), 7).rest, null);
assert.deepEqual([percentText(1, 1000), percentText(0, 10), percentText(5, 0)], ['<1%', '0%', null]);
// Fleet and Clash send one prefix to every agent; their launch row names the memory once with the count.
assert.equal(contextLaunchLine({kind: 'fleet', context: {ledger, agents: 5}}).textContent, 'Memory prefix ≈\u00a04.1k sent to 5 agents');
assert.equal(contextLaunchLine({kind: 'clash', context: {ledger, agents: 2}}).textContent, 'Memory prefix ≈\u00a04.1k sent to 2 agents');
assert.deepEqual([contextLaunchLine({kind: 'native', context: {ledger, agents: 1}}), contextLaunchLine({kind: 'fleet', context: null})], [null, null]);
""")

    def test_keyed_render_keeps_open_nodes_across_polls(self):
        page = ui_script()
        self.run_node(page[page.index("\nfunction keyedRender("):page.index("\nconst state = {")], r"""
class Item { constructor(value) { this.value = value; this.dataset = {}; this.open = false; this.parent = null; }
  replaceWith(node) { const list = this.parent.children; list[list.indexOf(this)] = node; node.parent = this.parent; this.parent = null; }
  remove() { const list = this.parent.children; list.splice(list.indexOf(this), 1); this.parent = null; } }
const container = {children: [], insertBefore(node, before) {
  if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1);
  const at = before ? this.children.indexOf(before) : this.children.length; this.children.splice(at, 0, node); node.parent = this; }};
const placeholder = new Item('No launches'); container.insertBefore(placeholder, null);
let built = 0; const render = items => keyedRender(container, items, item => item.id, item => { built++; return new Item(item.value); });
render([{id: 1, value: 'a'}, {id: 2, value: 'b'}]);
assert.deepEqual(container.children.map(node => node.value), ['a', 'b']);
const first = container.children[0]; first.open = true;
render([{id: 1, value: 'a'}, {id: 2, value: 'b'}]);
assert.equal(built, 2); assert.equal(container.children[0], first); assert.equal(first.open, true);
render([{id: 1, value: 'a, running 4s'}, {id: 2, value: 'b'}]);
assert.notEqual(container.children[0], first); assert.equal(container.children[0].open, true);
render([{id: 3, value: 'c'}, {id: 2, value: 'b'}]);
assert.deepEqual(container.children.map(node => node.value), ['c', 'b']); assert.equal(built, 4);
""")


# Run view. Raw streams in the shapes the CLIs print (claude -p stream-json, codex exec --json, cursor-agent
# stream-json) go through the backend's own normalize_event and Enricher, the way the session pump stores them,
# so a change on either side of the event contract fails here rather than on a real project.
def claude_raw(root="/repo"):
    def use(i, name, **inputs):
        return {"type": "tool_use", "id": f"toolu_{i:02d}", "name": name, "input": inputs}
    def say(*blocks, parent=None):
        return {"type": "assistant", "parent_tool_use_id": parent, "message": {"role": "assistant", "content": list(blocks)}}
    def done(*ids, error=()):
        return {"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": f"toolu_{i:02d}", "content": "PRIVATE OUTPUT", "is_error": i in error} for i in ids]}}
    steps = [("Inspect OrderController@store", "Inspecting the controller"), ("Add StoreOrderRequest", "Adding the request class"),
             ("Use the request in the controller", "Wiring the request"), ("Write feature tests", "Writing feature tests"),
             ("Run the order tests", "Running the order tests")]
    todos = lambda *states: {"todos": [{"content": text, "status": state, "activeForm": form} for (text, form), state in zip(steps, states)]}
    r = root + "/"
    return [
        {"type": "system", "subtype": "init", "session_id": "s-claude", "model": "claude-sonnet"},
        say({"type": "text", "text": "I'll look at how orders are created."}, use(1, "Read", file_path=r + "app/Http/Controllers/OrderController.php")), done(1),
        say(use(2, "Read", file_path=r + "app/Models/Order.php"), use(3, "Read", file_path=r + "routes/api.php"), use(4, "Read", file_path=r + "app/Models/OrderItem.php")), done(2, 3, 4),
        say(use(5, "TodoWrite", **todos("in_progress", "pending", "pending", "pending", "pending"))), done(5),
        say(use(6, "Grep", pattern="rules(", path=r + "app/Http"), use(7, "Glob", pattern="**/*Order*.php")), done(6, 7),
        say(use(8, "Write", file_path=r + "app/Http/Requests/StoreOrderRequest.php", content="PRIVATE")), done(8),
        say(use(9, "Edit", file_path=r + "app/Http/Controllers/OrderController.php", old_string="PRIVATE", new_string="PRIVATE")), done(9),
        say(use(10, "TodoWrite", **todos("completed", "completed", "completed", "in_progress", "pending"))), done(10),
        say(use(11, "Write", file_path=r + "tests/Feature/OrderValidationTest.php", content="PRIVATE")), done(11),
        say(use(12, "Bash", command="php artisan test --filter=OrderTest", description="Run the order tests")), done(12, error=(12,)),
        say(use(13, "Edit", file_path=r + "app/Http/Requests/StoreOrderRequest.php", old_string="x", new_string="y")), done(13),
        say(use(14, "Bash", command="php artisan test --filter=OrderTest")), done(14, error=(14,)),
        say(use(15, "Task", description="Find validation tests", prompt="PRIVATE", subagent_type="general-purpose")),
        say(use(16, "Grep", pattern="foreignId", path=r + "database/migrations"), parent="toolu_15"), done(16),
        say(use(17, "Read", file_path=r + "database/migrations/2024_01_01_create_orders_table.php"), parent="toolu_15"), done(17), done(15),
        say(use(18, "Edit", file_path=r + "tests/Feature/OrderValidationTest.php", old_string="a", new_string="b")), done(18),
        say(use(19, "Bash", command="php artisan test --filter=OrderTest")), done(19),
        say(use(20, "Read", file_path="/home/dev/.config/composer/auth.json")), done(20, error=(20,)),
        say(use(21, "Bash", command="vendor/bin/phpstan analyse app | tail -20")), done(21),
        say(use(22, "TodoWrite", todos=todos("completed", "completed", "completed", "completed", "completed")["todos"][:4]
                + [{"content": "Note the new rules in docs/api/orders.md", "status": "pending", "activeForm": "Noting the rules"}])), done(22),
        say(use(23, "Bash", command="vendor/bin/pint app/Http")), done(23),
        say({"type": "text", "text": "Added StoreOrderRequest and tests."}),
        {"type": "result", "subtype": "success", "is_error": False, "result": "Added StoreOrderRequest and tests.", "total_cost_usd": 1.37,
         "usage": {"input_tokens": 41200, "cache_read_input_tokens": 360000, "output_tokens": 11680}},
    ]


def codex_raw(root="/repo"):
    def command(i, text, exit_code=0):
        wrapped = f"/bin/bash -lc '{text}'"
        return [{"type": "item.started", "item": {"id": f"item_{i}", "type": "command_execution", "command": wrapped, "status": "in_progress"}},
                {"type": "item.completed", "item": {"id": f"item_{i}", "type": "command_execution", "command": wrapped, "aggregated_output": "PRIVATE",
                                                    "exit_code": exit_code, "status": "completed" if exit_code == 0 else "failed"}}]
    def change(i, *changes):
        item = {"id": f"item_{i}", "type": "file_change", "changes": [{"path": f"{root}/{path}", "kind": kind} for path, kind in changes]}
        return [{"type": "item.started", "item": {**item, "status": "in_progress"}}, {"type": "item.completed", "item": {**item, "status": "completed"}}]
    todo = lambda *done: {"type": "item.updated", "item": {"id": "item_3", "type": "todo_list", "items": [
        {"text": text, "completed": flag} for text, flag in zip(("Inspect the order model", "Add StoreOrderRequest", "Run the tests"), done)]}}
    return [
        {"type": "thread.started", "thread_id": "t-codex"}, {"type": "turn.started"},
        *command(1, "sed -n 1,200p app/Models/Order.php"), *command(2, 'rg -n "rules(" app/Http'), todo(True, False, False),
        *change(4, ("app/Http/Requests/StoreOrderRequest.php", "add")), *command(5, "php artisan test --filter=OrderTest", 1),
        *change(6, ("app/Http/Controllers/OrderController.php", "update"), ("app/Http/Legacy/OrderValidator.php", "delete")),
        *command(7, "php artisan test --filter=OrderTest"),
        {"type": "item.completed", "item": {"id": "item_8", "type": "command_execution", "command": "rm -rf build", "status": "declined"}},
        {"type": "item.started", "item": {"id": "item_9", "type": "collab_tool_call", "tool": "spawn_agent", "status": "in_progress", "receiver_thread_ids": ["PRIVATE"]}},
        # Codex repeats the operation while it runs; only the start carries targets.
        {"type": "item.updated", "item": {"id": "item_9", "type": "collab_tool_call", "tool": "spawn_agent", "status": "in_progress", "receiver_thread_ids": ["PRIVATE"]}},
        {"type": "item.completed", "item": {"id": "item_9", "type": "collab_tool_call", "tool": "spawn_agent", "status": "completed", "receiver_thread_ids": ["PRIVATE"]}},
        todo(True, True, True),
        {"type": "item.completed", "item": {"id": "item_10", "type": "agent_message", "text": "Done. The order tests pass."}},
        {"type": "turn.completed", "usage": {"input_tokens": 12000, "cached_input_tokens": 8000, "output_tokens": 3000}},
    ]


def cursor_raw(root="/repo"):
    call = lambda phase, i, key, **body: {"type": "tool_call", "subtype": phase, "call_id": f"c{i}", "tool_call": {key: body}}
    return [
        {"type": "system", "subtype": "init", "session_id": "c-cursor", "model": "auto"},
        call("started", 1, "readToolCall", args={"path": f"{root}/src/Order.php"}),
        call("completed", 1, "readToolCall", args={"path": f"{root}/src/Order.php"}, result={"success": {"content": "PRIVATE"}}),
        call("started", 2, "grepToolCall", args={"pattern": "function store", "path": f"{root}/src"}), call("completed", 2, "grepToolCall", result={"success": {}}),
        call("started", 3, "editToolCall", args={"path": f"{root}/src/Order.php", "streamContent": "PRIVATE"}), call("completed", 3, "editToolCall", result={"success": {}}),
        call("started", 4, "shellToolCall", args={"command": "vendor/bin/phpunit --filter OrderTest"}),
        call("completed", 4, "shellToolCall", args={"command": "vendor/bin/phpunit --filter OrderTest"}, result={"success": {"exitCode": 0}}),
        call("completed", 5, "shellToolCall", args={"command": "rm -rf vendor"}, result={"rejected": {"command": "rm -rf vendor", "reason": "PRIVATE"}}),
        call("started", 6, "updateTodosToolCall", args={"todos": [{"content": "Read Order", "status": "TODO_STATUS_COMPLETED"}, {"content": "Fix store()", "status": "TODO_STATUS_IN_PROGRESS"}]}),
        call("completed", 6, "updateTodosToolCall", result={"success": {}}),
        {"type": "assistant", "model_call_id": "m1", "message": {"role": "assistant", "content": [{"type": "text", "text": "Fixed store()."}]}},
        {"type": "result", "subtype": "success", "is_error": False, "result": "Fixed store().", "usage": {"inputTokens": 100, "outputTokens": 20}},
    ]


def pumped(provider, raw, targets=True, launch="L1", compaction_after=None, outcome="completed", start=datetime(2026, 10, 4, 14, 0, tzinfo=timezone.utc), first_id=1):
    """What the session pump stores for one native launch: plans and limited notices outside its count, targets after it."""
    enricher = run_activity.Enricher(provider, PurePosixPath("/repo"), None) if targets else None
    stored = [{"kind": "user", "text": "Add validation to POST /orders and cover it with tests."},
              {"kind": "status", "text": f"Running {provider} in edit mode.", "launch_id": launch}]
    count = output_bytes = 0
    for index, event in enumerate(raw):
        for clean in providers.normalize_event(provider, event, targets=targets):
            if clean.get("kind") == "plan":
                plan = enricher.plan(clean) if enricher else None
                if plan:
                    stored.append({**plan, "launch_id": launch})
                continue
            display = enricher.take(clean, count, output_bytes) if enricher and clean.get("kind") == "tool" else None
            output_bytes += len(json.dumps(clean))
            count += 1
            if display:
                clean.update(display)
            elif enricher and clean.get("kind") == "tool" and (limited := enricher.notice()):
                stored.append({**limited, "launch_id": launch})
            stored.append({**clean, "launch_id": launch})
        if enricher and index == compaction_after:
            stored.extend({**notice, "launch_id": launch} for notice in enricher.compaction([{"pre": 166040, "post": 38900}]))
    stored.append({"kind": "status", "outcome": outcome, "launch_id": launch,
                   "text": f"Run {outcome}. Process completion is not an independent verification of the task."})
    return [{**event, "id": first_id + offset, "at": (start + timedelta(seconds=10 * offset)).isoformat(timespec="milliseconds")}
            for offset, event in enumerate(stored)]


@unittest.skipUnless(shutil.which("node"), "Run view checks require Node")
class RunViewTests(unittest.TestCase):
    """RunModel (harness/web/run-model.js) has no DOM: these checks load the file in Node as the page does."""

    @classmethod
    def setUpClass(cls):
        cls.streams = {"claude": pumped("claude", claude_raw(), compaction_after=20), "codex": pumped("codex", codex_raw()),
                       "cursor": pumped("cursor", cursor_raw()), "claude_labels": pumped("claude", claude_raw(), targets=False),
                       "codex_labels": pumped("codex", codex_raw(), targets=False)}
        with patch.object(run_activity, "ENRICH_EVENTS", 12):
            cls.streams["claude_limited"] = pumped("claude", claude_raw())

    def run_node(self, checks, prelude=""):
        source = "const assert = require('node:assert/strict');\n" + (WEB / "run-model.js").read_text(encoding="utf-8") \
            + "\nconst streams = " + json.dumps(self.streams) + ";\n" + prelude + "\n" + checks
        result = subprocess.run([shutil.which("node"), "-e", source], capture_output=True, text=True)
        if result.returncode:
            self.fail(result.stderr)

    def test_claude_targets_fold_into_files_searches_checks_plan_and_moments(self):
        self.run_node(r"""
const model = RunModel.create(); RunModel.observe(model, streams.claude); const run = RunModel.current(model);
assert.deepEqual([run.mode, run.finished, run.outcome, run.ordinal], ['targets', true, 'completed', 1]);
assert.deepEqual(run.counters, {steps:23, opened:6, edited:3, edits:5, searched:3, commands:5, failed:3, notRun:0, messages:2, commandFailed:2});
// Files in first-touch order; Write to a path not seen in the session created it, an Edit did not.
const paths = [...run.paths.values()];
assert.deepEqual(paths.map(p => p.key), ['app/Http/Controllers/OrderController.php', 'app/Models/Order.php', 'routes/api.php', 'app/Models/OrderItem.php',
  'app/Http/Requests/StoreOrderRequest.php', 'tests/Feature/OrderValidationTest.php', 'database/migrations/2024_01_01_create_orders_table.php', 'outside:auth.json']);
const file = key => run.paths.get(key);
assert.deepEqual([file('app/Http/Requests/StoreOrderRequest.php').created, file('app/Http/Requests/StoreOrderRequest.php').edits, file('app/Http/Controllers/OrderController.php').created], [true, 2, false]);
assert.deepEqual([file('outside:auth.json').path, file('outside:auth.json').outside, file('outside:auth.json').failed], ['auth.json', true, true]);
const helper = file('database/migrations/2024_01_01_create_orders_table.php'); assert.deepEqual([helper.helper, helper.main], [true, false]);
assert.deepEqual(run.searches.map(item => [item.pattern, item.scope, item.helper]), [['"rules("', 'app/Http', false], ['"**/*Order*.php"', '', false], ['"foreignId"', 'database/migrations', true]]);
assert.deepEqual(RunModel.searchedIn(run, file('app/Http/Controllers/OrderController.php')), ['"rules("', '"**/*Order*.php"']);
// A Claude completion names only its call; it pairs with the start that named the tool.
assert.deepEqual([run.calls.get('toolu_12').tool, run.calls.get('toolu_12').ok, run.calls.get('toolu_19').ok], ['Bash', false, true]);
assert.deepEqual(RunModel.checks(run).map(row => [row.family, row.target, row.runs.map(item => item.glyph), row.green || null, row.masked]),
  [['Artisan test', '--filter=OrderTest', ['fail', 'fail', 'ok'], {runs:3, edits:2}, null], ['PHPStan', 'app', ['unknown'], null, "piped to tail: exit status is tail's"]]);
assert.deepEqual(run.commands.filter(cmd => cmd.cls.fixer).map(cmd => cmd.command), ['vendor/bin/pint app/Http']);
// The plan compares snapshots by exact text only.
assert.deepEqual([run.plan.items.filter(item => item.status === 'done').length, run.plan.total, [...run.plan.added]], [4, 5, ['Note the new rules in docs/api/orders.md']]);
assert.deepEqual([run.planChange.added, run.planChange.dropped, run.dropped], [['Note the new rules in docs/api/orders.md'], ['Run the order tests'], ['Run the order tests']]);
assert.deepEqual(run.moments.map(item => item.kind), ['first-edit', 'first-failed-command', 'compaction', 'check-green', 'plan-changed']);
assert.deepEqual(run.compactions.map(item => [item.pre, item.post]), [[166040, 38900]]);
assert.equal(RunModel.now(model, run), null);
// The receipt and the step groups read the same facts.
assert.equal(RunModel.groupSummary({steps:7, tools:new Map([['Read', 4], ['Grep', 2], ['Edit', 1]]), patterns:['"rules("', '"**/*Order*.php"'], opened:new Set(['a', 'b', 'c', 'd']), edited:new Set(['a'])}),
  '7 steps · Read ×4, Grep ×2, Edit · Looked for "rules(", "**/*Order*.php" · opened 4 files · edited 1');
assert.doesNotMatch(JSON.stringify(run, (key, value) => value instanceof Map || value instanceof Set ? [...value] : value), /PRIVATE|\/repo\//);
""")

    def test_live_pages_pair_calls_and_say_what_runs_now(self):
        self.run_node(r"""
const model = RunModel.create(), events = streams.claude, at = id => Date.parse(events.find(event => event.id === id).at);
let run = null;
const feed = upTo => { for (const event of events.filter(event => event.id <= upTo && !feed.seen.has(event.id))) { feed.seen.add(event.id); RunModel.observe(model, [event], {arrival:Date.parse(event.at) + 400, live:true}); } run = RunModel.current(model); };
feed.seen = new Set();
const id = (call, state) => events.find(event => event.call === call && event.state === state).id;
feed(2); assert.deepEqual(RunModel.now(model, run, at(2) + 1000), {kind:'preparing'});
// The browser's clock runs 400 ms behind the stored `at`; clocks subtract the offset.
feed(id('toolu_01', 'started')); assert.equal(model.offset, 400);
let now = RunModel.now(model, run, at(id('toolu_01', 'started')) + 400 + 3000);
assert.deepEqual([now.kind, now.verb, now.target, now.path, now.clock], ['call', 'Reading', 'app/Http/Controllers/OrderController.php', true, 3]);
feed(id('toolu_04', 'started')); now = RunModel.now(model, run);
assert.deepEqual([now.kind, now.verb, now.target, now.more, now.count], ['parallel', 'Reading 3 files', 'app/Models/Order.php', '+2', 3]);
feed(id('toolu_02', 'completed')); assert.equal(RunModel.now(model, run).verb, 'Reading 2 files');
feed(id('toolu_16', 'started')); now = RunModel.now(model, run);
assert.deepEqual([now.kind, now.verb, now.target], ['helper', 'Helper · Searching', '"foreignId" in database/migrations']);
feed(id('toolu_15', 'completed')); now = RunModel.now(model, run, at(id('toolu_15', 'completed')) + 400 + 45000);
assert.deepEqual([now.kind, now.verb, now.clock], ['model', "Model's turn", 45]);
feed(id('toolu_21', 'started')); now = RunModel.now(model, run); assert.deepEqual([now.verb, now.target, now.path], ['Running', 'vendor/bin/phpstan analyse app | tail -20', false]);
// One live page of changes: what the strip announces and animates.
const ch = RunModel.observe(model, events.filter(event => event.id > id('toolu_21', 'started')), {arrival:Date.now(), live:true});
assert.deepEqual([ch.finished, ch.plan, ch.milestones], [run.id, true, ['Run finished']]);
const first = RunModel.observe(RunModel.create(), events.filter(event => event.id <= id('toolu_12', 'completed')));
assert.deepEqual(first.milestones, ['First edit: app/Http/Requests/StoreOrderRequest.php', 'Command failed: php artisan test --filter=OrderTest']);
// History pages and catch-up pages of 250 are drawn final; only a short page of a watched run is live.
assert.equal(RunModel.live({loaded:false, count:12, running:true, hidden:false}), false);
assert.equal(RunModel.live({loaded:true, count:250, running:true, hidden:false}), false);
assert.equal(RunModel.live({loaded:true, count:12, running:true, hidden:true}), false);
assert.equal(RunModel.live({loaded:true, count:12, running:false, hidden:false}), false);
assert.equal(RunModel.live({loaded:true, count:12, running:true, hidden:false}), true);
// Without `at`, history has no clock (never 0:00) and a live event uses its arrival, marked approximate.
const old = RunModel.create(), plain = streams.claude_labels.filter(event => event.id <= 6).map(({at, ...event}) => event);
RunModel.observe(old, plain); const oldRun = RunModel.current(old);
assert.deepEqual([oldRun.startAt, oldRun.approx, RunModel.now(old, oldRun).clock], [null, true, null]);
RunModel.observe(old, [{id:900, kind:'tool', text:'Bash: started', launch_id:'L1'}], {arrival:5000, live:true});
assert.deepEqual([RunModel.now(old, oldRun, 9000).verb, RunModel.now(old, oldRun, 9000).clock], ['Running a command', 4]);
RunModel.observe(old, [{id:901, kind:'tool', text:'Read: started', launch_id:'L1'}], {arrival:6000, live:true});
assert.deepEqual([RunModel.now(old, oldRun, 9000).verb, RunModel.now(old, oldRun, 9000).clock], ['2 steps running', null]);
""")

    def test_codex_and_cursor_targets_read_changes_exit_codes_and_not_run(self):
        self.run_node(r"""
let model = RunModel.create(); RunModel.observe(model, streams.codex); let run = RunModel.current(model);
assert.equal(run.mode, 'targets');
assert.deepEqual(run.counters, {steps:8, opened:0, edited:3, edits:3, searched:0, commands:5, failed:1, notRun:1, messages:1, commandFailed:1});
assert.deepEqual([...run.paths.values()].map(p => [p.path, p.created, p.deleted]), [['app/Http/Requests/StoreOrderRequest.php', true, false],
  ['app/Http/Controllers/OrderController.php', false, false], ['app/Http/Legacy/OrderValidator.php', false, true]]);
assert.deepEqual(run.commands.map(cmd => [cmd.command, cmd.exitCode, cmd.outcome]), [['sed -n 1,200p app/Models/Order.php', 0, null], ['rg -n "rules(" app/Http', 0, null],
  ['php artisan test --filter=OrderTest', 1, null], ['php artisan test --filter=OrderTest', 0, null], ['rm -rf build', null, 'not_run']]);
assert.deepEqual(RunModel.checks(run).map(row => row.runs.map(item => item.glyph)), [['fail', 'ok']]);
assert.deepEqual([run.plan.activeForm, run.plan.items.map(item => item.status)], [null, ['done', 'done', 'done']]);
const spawn = run.calls.get('item_9'); assert.deepEqual([spawn.tool, RunModel.verbOf(spawn)], ['Agent', 'Starting a helper']);
// The label-only repeat of the open spawn is not a second step, so nothing is left running after it completes.
const live = RunModel.create(), upTo = streams.codex.findIndex(event => event.call === 'item_9' && event.state === 'completed');
RunModel.observe(live, streams.codex.slice(0, upTo + 1)); const liveRun = RunModel.current(live);
assert.deepEqual([liveRun.anon.length, liveRun.open.size, RunModel.now(live, liveRun).kind], [0, 0, 'model']);
model = RunModel.create(); RunModel.observe(model, streams.cursor); run = RunModel.current(model);
assert.deepEqual(run.counters, {steps:6, opened:1, edited:1, edits:1, searched:1, commands:2, failed:0, notRun:1, messages:1, commandFailed:0});
assert.deepEqual([...run.tools.keys()], ['Read', 'Grep', 'Edit', 'Shell', 'UpdateTodos']);
assert.deepEqual(RunModel.checks(run).map(row => [row.family, row.target, row.runs.map(item => item.glyph)]), [['PHPUnit', '--filter=OrderTest', ['ok']]]);
assert.deepEqual(run.plan.items.map(item => item.status), ['done', 'active']);
""")

    def test_label_only_runs_count_steps_by_starts_and_keep_failures_unattributed(self):
        self.run_node(r"""
// Recorded before targets existed: Claude starts carry no ok, completions are unnamed 'Tool: completed'.
let model = RunModel.create(); RunModel.observe(model, streams.claude_labels); let run = RunModel.current(model);
assert.equal(run.mode, 'labels'); assert.equal(run.paths.size, 0);
assert.deepEqual([run.counters.steps, run.counters.failed, run.counters.commands, run.counters.commandFailed], [23, 3, 5, 0]);
assert.equal(RunModel.tally(run.tools), 'Read ×6, TodoWrite ×3, Grep ×2, Glob, Write ×2, Edit ×3, Bash ×5, Task');
assert.equal(RunModel.toolName({text:'Tool: completed'}), 'Tool');
// The Now line names the kind of step; with more than one open it counts them and shows no step clock.
model = RunModel.create(); const events = streams.claude_labels, upTo = n => events.filter(event => event.id <= n);
const read = events.find(event => event.text === 'Read: started').id;
RunModel.observe(model, upTo(read)); run = RunModel.current(model);
let now = RunModel.now(model, run, Date.parse(events.find(event => event.id === read).at) + 5000);
assert.deepEqual([now.kind, now.verb, now.clock], ['labels', 'Reading a file', 5]);
const parallel = events.filter(event => event.text === 'Read: started')[3].id;
RunModel.observe(model, events.filter(event => event.id > read && event.id <= parallel)); now = RunModel.now(model, run);
assert.deepEqual([now.verb, now.clock], ['3 steps running', null]);
// Codex labels name the item type; a failed command is a failed command, a helper journal is a helper.
model = RunModel.create(); RunModel.observe(model, streams.codex_labels); run = RunModel.current(model);
assert.deepEqual([run.mode, run.counters.steps, run.counters.commands, run.counters.commandFailed, [...run.tools.keys()]], ['labels', 7, 4, 1, ['Shell', 'Edit', 'Agent']]);
assert.deepEqual(['command_execution: started', 'file_change: completed', 'mcp_tool_call: started', 'web_search: completed', 'Agent activity: started (native session journal)',
  'Agent wait: in_progress', 'readToolCall: started', 'Read: started'].map(text => [RunModel.toolName({text}), RunModel.isStart({text})]),
  [['Shell', true], ['Edit', false], ['MCP', true], ['Web search', false], ['Agent', true], ['Agent', true], ['Read', true], ['Read', true]]);
""")

    def test_failed_steps_name_the_first_one_to_show_and_plans_keep_their_full_count(self):
        self.run_node(r"""
// With targets the first failed call is the step to show; label-only failures name no call, so their own row is.
let model = RunModel.create(); RunModel.observe(model, streams.claude); let run = RunModel.current(model);
const first = streams.claude.find(event => event.call && event.state === 'started' && run.calls.get(event.call)?.ok === false && run.calls.get(event.call).outcome !== 'not_run');
assert.equal(run.firstFailedEventId, first.id);
model = RunModel.create(); RunModel.observe(model, streams.claude_labels); run = RunModel.current(model);
assert.equal(run.calls.size, 0);
assert.equal(run.firstFailedEventId, streams.claude_labels.find(event => event.kind === 'tool' && event.ok === false).id);
model = RunModel.create(); RunModel.observe(model, streams.claude.filter(event => event.ok !== false)); assert.equal(RunModel.current(model).firstFailedEventId, null);
// The backend keeps 30 items and sends the full count: the away line and the plan chip say the same denominator.
model = RunModel.create();
RunModel.observe(model, [{id:1, kind:'plan', launch_id:'L1', items:Array.from({length:30}, (_, n) => ({text:`Item ${n}`, status:n < 12 ? 'done' : 'pending'})), total:42}]);
run = RunModel.current(model);
assert.match(RunModel.awaySummary(run, RunModel.snapshot(run), 3), /plan 12 of 42$/);
""")

    def test_enrichment_limit_turns_counts_into_lower_bounds(self):
        self.run_node(r"""
const model = RunModel.create(), ch = RunModel.observe(model, streams.claude_limited, {arrival:Date.now(), live:true}), run = RunModel.current(model);
const notices = streams.claude_limited.filter(event => event.targets === 'limited');
assert.equal(notices.length, 1);
// Twelve enriched events are seven calls: starts and completions both carry targets.
assert.deepEqual([run.mode, run.limited.step, run.counters.steps], ['targets', 7, 23]);
assert.ok(ch.milestones.includes('Step details are no longer recorded'));
assert.ok(run.counters.opened < 6, 'after the notice, paths are no longer known');
assert.ok(run.moments.some(item => item.kind === 'limit'));
// Grep and Glob were open at the notice; their completions arrive as plain labels without a call. They close them in
// order, so the Now line does not stay on two searches, and their own results stay unknown.
const open = RunModel.create(); RunModel.observe(open, streams.claude_limited.filter(event => !/^Run completed/.test(event.text || '')));
const openRun = RunModel.current(open);
assert.deepEqual([openRun.open.size, openRun.anon.length, RunModel.now(open, openRun).kind], [0, 0, 'model']);
assert.deepEqual(['toolu_06', 'toolu_07'].map(call => openRun.calls.get(call).state), ['unfinished', 'unfinished']);
""")

    def test_cursor_delete_tool_marks_the_file_deleted(self):
        # cursor-agent's delete tool names the file it removes; the receipt's ledger counts it as edited and deleted.
        raw = [{"type": "tool_call", "subtype": "started", "call_id": "c1", "tool_call": {"deleteToolCall": {"args": {"path": "/repo/src/Old.php"}}}},
               {"type": "tool_call", "subtype": "completed", "call_id": "c1", "tool_call": {"deleteToolCall": {"args": {"path": "/repo/src/Old.php"}, "result": {"success": {}}}}}]
        events = pumped("cursor", raw)
        ledger = run_activity.Enricher("cursor", PurePosixPath("/repo"), None)
        for event in raw:
            for clean in providers.normalize_event("cursor", event, targets=True):
                ledger.take(clean, 0, 0)
        self.assertEqual((1, 1, ["src/Old.php"]), (ledger.ledger.receipt()["edited"], ledger.ledger.receipt()["deleted"], ledger.ledger.receipt()["edited_paths"]))
        self.run_node(r"""
const model = RunModel.create(); RunModel.observe(model, events); const run = RunModel.current(model), file = run.paths.get('src/Old.php');
assert.deepEqual([run.calls.get('c1').tool, file.deleted, file.edited, run.counters.edited], ['Delete', true, true, 1]);
assert.deepEqual([RunModel.describe(events.find(event => event.state === 'started')).edit, RunModel.verbOf(run.calls.get('c1'))], [true, 'Deleting']);
""", prelude="const events = " + json.dumps(events) + ";")

    def real_launch(self, provider, raw):
        """One native launch through the real session pump with a scripted CLI: (session, stored events, receipt, project)."""
        fake_cli = ("import json, pathlib, sys\nconfig = json.loads(sys.argv[1])\nsys.stdin.read()\n"
                    "for line in pathlib.Path(config['events']).read_text(encoding='utf-8').splitlines():\n    print(line, flush=True)\n")
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp); project = root / "project"; project.mkdir()
            fake, script = root / "fake_provider.py", root / "events.jsonl"
            fake.write_text(fake_cli)
            script.write_text("\n".join(json.dumps(event, ensure_ascii=False) for event in raw(str(project))), encoding="utf-8")
            with patch.object(providers, "discover_providers", return_value=[{"id": provider, "available": True, "executable": str(fake)}]), \
                    patch.object(providers, "model_options", return_value={"models": [], "efforts": [], "detail": "Offline fixture"}), \
                    patch.object(providers, "build_command", side_effect=lambda *args, **options: [sys.executable, "-u", str(fake), json.dumps({"events": str(script)})]):
                store = sessions.Sessions(root / "state", [project], timeout=120)
                try:
                    sid = store.create({"project_id": next(iter(store.projects)), "provider": provider, "prompt": "Add validation to POST /orders",
                                        "project_context": False})["id"]
                    deadline = time.monotonic() + 60
                    while (store.get(sid)["status"] in sessions.ACTIVE or store.jobs.unfinished_tasks) and time.monotonic() < deadline:
                        time.sleep(.02)
                    return store.get(sid), store.events(sid), store.results.history(sid)["launches"][-1]["receipt"], str(project)
                finally:
                    store.close()

    @unittest.skipUnless(hasattr(os, "killpg"), "Session workers require POSIX process groups")
    def test_the_real_pump_stores_what_run_model_reads_and_the_receipt_agrees(self):
        # pumped() above mirrors the pump; this runs the real one, so the two sides of the contract meet as they do in use.
        for provider, raw in (("claude", claude_raw), ("codex", codex_raw), ("cursor", cursor_raw)):
            with self.subTest(provider=provider):
                session, events, receipt, project = self.real_launch(provider, raw)
                self.assertEqual(("completed", "completed"), (session["status"], session["launch"]["status"]))
                tools = [event for event in events if event["kind"] in ("tool", "plan")]
                self.assertNotIn(project, json.dumps(tools, ensure_ascii=False))
                self.run_node(r"""
const model = RunModel.create(); RunModel.observe(model, events); const run = RunModel.current(model), paths = [...run.paths.values()];
assert.deepEqual([run.id, run.mode, run.outcome, run.approx], [launch.id, 'targets', 'completed', false]);
// The server counts every tool event whatever the display caps; the browser folds the stored ones. Unlimited, they agree.
const listed = paths.filter(p => !p.outside);
assert.deepEqual([run.counters.steps, run.counters.opened, run.counters.edited, run.counters.searched, run.counters.commands, run.counters.commandFailed, run.counters.notRun, run.counters.failed],
  [receipt.steps, receipt.opened, receipt.edited, receipt.searched, receipt.commands, receipt.failed, receipt.not_run, receipt.failed_steps]);
assert.deepEqual([paths.filter(p => p.created).length, paths.filter(p => p.deleted).length], [receipt.created, receipt.deleted]);
assert.deepEqual(listed.filter(p => p.opened).map(p => p.path), receipt.opened_paths);
assert.deepEqual(listed.filter(p => p.edited).map(p => p.path).sort(), [...receipt.edited_paths].sort());
assert.deepEqual(run.commands.map(cmd => [cmd.command, cmd.ok, cmd.outcome, cmd.exitCode]), receipt.last_commands.map(cmd => [cmd.command, cmd.ok, cmd.outcome, cmd.exit_code]));
assert.deepEqual(run.searches.map(item => item.pattern), receipt.patterns);
assert.deepEqual({done:run.plan.items.filter(item => item.status === 'done').length, total:run.plan.total}, receipt.plan);
""", prelude=f"const events = {json.dumps(events)}, launch = {json.dumps(session['launch'])}, receipt = {json.dumps(receipt)};")

    @unittest.skipUnless(hasattr(os, "killpg"), "Session workers require POSIX process groups")
    def test_a_launch_that_raises_ends_on_the_worker_error_with_its_receipt(self):
        # A launch that raises (here the CLI changes its session identity) has no closing status: the worker's error line ends it,
        # and the receipt the server saved on that path is drawn after it.
        def raw(root):
            return [{"type": "system", "subtype": "init", "session_id": "s-one"},
                    {"type": "assistant", "parent_tool_use_id": None, "message": {"role": "assistant", "content": [
                        {"type": "tool_use", "id": "toolu_01", "name": "Read", "input": {"file_path": root + "/app/Order.php"}}]}},
                    {"type": "system", "subtype": "init", "session_id": "s-two"}]
        session, events, receipt, _ = self.real_launch("claude", raw)
        self.assertEqual(("failed", "failed"), (session["status"], session["launch"]["status"]))
        self.assertEqual((1, ["app/Order.php"], False), (receipt["steps"], receipt["opened_paths"], receipt["limited"]))
        self.assertFalse(any("outcome" in event for event in events))
        self.run_node(r"""
const model = RunModel.create(), ch = RunModel.observe(model, events), run = RunModel.current(model), last = events[events.length - 1];
assert.deepEqual([RunModel.aborted(last), last.launch_id], [true, run.id]);
assert.deepEqual([run.finished, run.outcome, run.aborted, run.closingEventId, ch.finished], [true, 'failed', true, last.id, run.id]);
assert.deepEqual([run.calls.get('toolu_01').state, RunModel.now(model, run)], ['unfinished', null]);
assert.equal(RunModel.aborted({kind:'error', text:'Provider did not complete successfully. Check CLI authentication.'}), false);
""", prelude=f"const events = {json.dumps(events)};")

    def test_plan_changes_compare_exact_text_and_return_dropped_items(self):
        self.run_node(r"""
const model = RunModel.create(); let id = 0;
const plan = (...items) => RunModel.observe(model, [{id:++id, kind:'plan', launch_id:'L1', items:items.map(([text, status]) => ({text, status})), total:items.length}]);
plan(['A', 'active'], ['B', 'pending']); const run = RunModel.current(model);
assert.equal(run.planChange, null);
plan(['B', 'pending'], ['A', 'done']); assert.equal(run.planChange, null, 'order and status are not changes');
plan(['A', 'done'], ['B, reworded', 'pending'], ['C', 'pending']);
assert.deepEqual([run.planChange.added, run.planChange.dropped, run.dropped, [...run.plan.added]], [['B, reworded', 'C'], ['B'], ['B'], ['B, reworded', 'C']]);
plan(['A', 'done'], ['B', 'pending'], ['C', 'pending']); assert.deepEqual(run.dropped, ['B, reworded'], 'a returning item leaves Dropped');
plan(['A', 'done'], ['\u202eevil', 'bogus']); assert.deepEqual(run.plan.items[1], {text:'evil', status:'pending'});
assert.deepEqual(run.moments.filter(item => item.kind === 'plan-changed').length, 1);
""")

    def test_away_summary_counts_what_changed_while_the_tab_was_hidden(self):
        self.run_node(r"""
const model = RunModel.create(), events = streams.claude, cut = events.find(event => event.call === 'toolu_12' && event.state === 'started').id;
RunModel.observe(model, events.filter(event => event.id <= cut)); const run = RunModel.current(model), before = RunModel.snapshot(run);
RunModel.observe(model, events.filter(event => event.id > cut && event.call !== 'toolu_21' && !/Run completed/.test(event.text || '')));
assert.equal(RunModel.awaySummary(run, before, 4), 'While you were away (4 min): 10 steps · 2 edits in 2 files · Artisan test ✗ ✗ ✓ · plan 4 of 5');
RunModel.observe(model, events.filter(event => /Run completed/.test(event.text || '')));
assert.match(RunModel.awaySummary(run, before, 6), /^While you were away \(6 min\): run finished · /);
""")

    def test_classify_command_reads_php_checks_wrappers_masks_and_windows_forms(self):
        cases = [
            # command, family, target, check, fixer, masked (substring or None), bare
            ("php artisan test --filter=OrderTest", "Artisan test", "--filter=OrderTest", True, False, None, "php artisan test --filter=OrderTest"),
            ("php artisan test", "Artisan test", "all tests", True, False, None, "php artisan test"),
            ("php artisan test --parallel --testsuite=Feature", "Artisan test", "--testsuite=Feature", True, False, None, None),
            ("vendor/bin/phpunit", "PHPUnit", "all tests", True, False, None, None),
            ("./vendor/bin/phpunit --filter OrderTest", "PHPUnit", "--filter=OrderTest", True, False, None, None),
            ("vendor/bin/phpunit tests/Feature/OrderTest.php", "PHPUnit", "tests/Feature/OrderTest.php", True, False, None, None),
            ("bin/phpunit --group slow", "PHPUnit", "--group=slow", True, False, None, None),
            ('vendor/bin/pest --filter="creates order"', "Pest", "--filter=creates order", True, False, None, 'vendor/bin/pest --filter="creates order"'),
            ("vendor/bin/paratest -p4", "ParaTest", "all tests", True, False, None, None),
            ("vendor/bin/phpstan analyse app --level=8 --memory-limit=1G", "PHPStan", "app --level=8", True, False, None, None),
            ("vendor/bin/phpstan analyse -c phpstan.neon", "PHPStan", "project", True, False, None, None),
            ("php -d memory_limit=-1 vendor/bin/phpstan analyse", "PHPStan", "project", True, False, None, "php -d memory_limit=-1 vendor/bin/phpstan analyse"),
            ("vendor/bin/psalm --no-cache", "Psalm", "project", True, False, None, None),
            ("vendor/bin/phpcs --standard=PSR12 src", "PHPCS", "src", True, False, None, None),
            ("php -l app/Models/Order.php", "PHP lint", "app/Models/Order.php", True, False, None, None),
            ("vendor/bin/pint", "Pint", "project", False, True, None, None),
            ("vendor/bin/pint --test", "Pint", "project", True, False, None, None),
            ("vendor/bin/php-cs-fixer fix --dry-run --diff", "PHP-CS-Fixer", "project", True, False, None, None),
            ("vendor/bin/php-cs-fixer fix src", "PHP-CS-Fixer", "src", False, True, None, None),
            ("vendor/bin/phpcbf src", "PHPCBF", "src", False, True, None, None),
            ("vendor/bin/rector process --dry-run", "Rector", "project", True, False, None, None),
            ("vendor/bin/rector", "Rector", "project", False, True, None, None),
            ("bin/console lint:yaml config", "Symfony lint", "lint:yaml config", True, False, None, None),
            ("php bin/console doctrine:schema:validate", "Doctrine schema", "validate", True, False, None, None),
            ("wp core verify-checksums", "WP checksums", "core", True, False, None, None),
            ("composer validate --strict", "Composer", "validate", True, False, None, None),
            ("composer audit", "Composer", "audit", True, False, None, None),
            ("composer test", "Composer", "test", True, False, None, None),
            ("composer exec -- phpunit", "PHPUnit", "all tests", True, False, None, None),
            ("vendor/bin/codecept run unit", "Codeception", "unit", True, False, None, None),
            ("cd /repo && vendor/bin/phpunit", "PHPUnit", "all tests", True, False, None, "vendor/bin/phpunit"),
            ("XDEBUG_MODE=off vendor/bin/phpunit --testsuite=Unit", "PHPUnit", "--testsuite=Unit", True, False, None, "vendor/bin/phpunit --testsuite=Unit"),
            ("timeout 600 php artisan test", "Artisan test", "all tests", True, False, None, "php artisan test"),
            ("docker compose exec -T app php artisan test --filter=Order", "Artisan test", "--filter=Order", True, False, None, "php artisan test --filter=Order"),
            ("docker-compose exec app vendor/bin/phpunit", "PHPUnit", "all tests", True, False, None, "vendor/bin/phpunit"),
            ("ddev exec vendor/bin/phpstan analyse", "PHPStan", "project", True, False, None, "vendor/bin/phpstan analyse"),
            ("lando php vendor/bin/phpcs", "PHPCS", "project", True, False, None, "php vendor/bin/phpcs"),
            ("./vendor/bin/sail artisan test", "Artisan test", "all tests", True, False, None, "php artisan test"),
            ("sail test --filter=Order", "Artisan test", "--filter=Order", True, False, None, "php artisan test --filter=Order"),
            ("vendor/bin/phpstan analyse app | tail -20", "PHPStan", "app", True, False, "piped to tail", "vendor/bin/phpstan analyse app"),
            ("php artisan test 2>&1 | tail -50", "Artisan test", "all tests", True, False, "piped to tail", "php artisan test"),
            ("vendor/bin/phpunit || true", "PHPUnit", "all tests", True, False, "a failure is ignored", "vendor/bin/phpunit"),
            ('vendor/bin/phpunit; echo "exit: $?"', "PHPUnit", "all tests", True, False, "the last command sets the exit status", None),
            ("vendor/bin/phpunit && echo OK", "PHPUnit", "all tests", True, False, None, "vendor/bin/phpunit"),
            ("vendor/bin/phpunit > /tmp/out.txt 2>&1", "PHPUnit", "all tests", True, False, None, "vendor/bin/phpunit"),
            ("vendor/bin/phpunit &", "PHPUnit", "all tests", True, False, "background", None),
            ("set -o pipefail && vendor/bin/phpunit | tee out.log", "PHPUnit", "all tests", True, False, None, "vendor/bin/phpunit"),
            ("/bin/bash -lc 'vendor/bin/phpunit --filter Order'", "PHPUnit", "--filter=Order", True, False, None, "vendor/bin/phpunit --filter Order"),
            (r"vendor\bin\phpunit.bat --filter OrderTest", "PHPUnit", "--filter=OrderTest", True, False, None, r"vendor\bin\phpunit.bat --filter OrderTest"),
            (r"php.exe vendor\bin\phpstan analyse src", "PHPStan", "src", True, False, None, None),
            (r".\vendor\bin\pest.bat", "Pest", "all tests", True, False, None, None),
            (r'& "C:\php\php.exe" artisan test', "Artisan test", "all tests", True, False, None, r'"C:\php\php.exe" artisan test'),
            (r'pwsh -NoProfile -Command "vendor\bin\phpunit | Select-Object -Last 20"', "PHPUnit", "all tests", True, False, "piped to Select-Object", r"vendor\bin\phpunit"),
            (r'powershell -Command "vendor\bin\phpstan.bat analyse src; exit $LASTEXITCODE"', "PHPStan", "src", True, False, None, r"vendor\bin\phpstan.bat analyse src"),
            (r"cmd /c vendor\bin\phpcs.bat --standard=PSR12 src", "PHPCS", "src", True, False, None, None),
            (r'$env:XDEBUG_MODE="off"; vendor\bin\phpunit.bat', "PHPUnit", "all tests", True, False, None, r"vendor\bin\phpunit.bat"),
        ]
        others = ["npm test", "git status", "sed -n '1,200p' app/Models/Order.php", 'rg -n "rules(" app/Http', "composer install",
                  "vendor/bin/phpstan --version", "php artisan migrate", "ls -la", "cat composer.json | head"]
        self.assertGreaterEqual(len(cases) + len(others), 40)
        self.run_node("const cases = " + json.dumps(cases) + ", others = " + json.dumps(others) + ";\n" + r"""
for (const [command, family, target, check, fixer, masked, bare] of cases) {
  const result = RunModel.classifyCommand(command), label = `classify ${command}`;
  assert.deepEqual([result.family, result.target, result.check, result.fixer], [family, target, check, fixer], label);
  if (masked) assert.match(result.masked || '', new RegExp(masked.replace(/[|]/g, '\\|')), label); else assert.equal(result.masked, null, label);
  if (bare) assert.equal(result.bare, bare, label);
  assert.equal(result.key, `${family}|${target}`, label);
  assert.equal(result.runnable, true, `${label} leaves one command Harness can run`);
}
for (const command of others) assert.deepEqual([RunModel.classifyCommand(command).family, RunModel.classifyCommand(command).check], [null, false], command);
// What was peeled off is named, so Run in Harness can say it.
assert.deepEqual(RunModel.classifyCommand('cd app && XDEBUG_MODE=off vendor/bin/phpunit 2>&1 | tail -5').removed, ['cd', 'environment', 'redirect', '| tail']);
assert.deepEqual(RunModel.classifyCommand('docker compose exec -T app vendor/bin/phpunit').removed, ['container']);
// Results.check_command refuses shell operators, control characters and more than 4000 bytes.
assert.deepEqual(['vendor/bin/phpunit', 'vendor/bin/phpunit | tail', 'a && b', 'x\ny', 'x'.repeat(4001), 'say "open', ''].map(RunModel.runnable), [true, false, false, false, false, false, false]);
assert.equal(RunModel.classifyCommand('vendor/bin/phpunit --filter \u202eOrder').target, '--filter=Order');
""")

    def test_step_rows_name_their_tool_and_target_and_groups_count_as_steps_arrive(self):
        page = ui_script()
        helpers = page[page.index("\nfunction stepGroup("):page.index("\nfunction appendEvent(")]
        append = page[page.index("\nfunction appendEvent("):]
        append = append[:append.index("\n}\n") + 3]
        self.run_node(r"""
class Node { constructor(tag, className = '', text) { Object.assign(this, {tag, className, own:text ?? '', children:[], dataset:{}, attributes:{}}); }
  get textContent() { return this.children.length ? this.children.map(child => child.textContent).join('') : this.own; }
  set textContent(value) { this.children = []; this.own = String(value); }
  get classList() { return {contains: name => this.className.split(' ').includes(name)}; }
  get firstElementChild() { return this.children[0]; } get lastElementChild() { return this.children[this.children.length - 1]; } get lastChild() { return this.lastElementChild; }
  setAttribute(name, value) { this.attributes[name] = String(value); } append(...nodes) { this.children.push(...nodes); } }
const events = new Node('div'), $ = id => id === 'events' ? events : null, el = (tag, className, text) => new Node(tag, className, text);
const document = {createTextNode: text => new Node('#text', '', text)};
const state = {eventIds:new Set(), assistantTexts:new Set(), stepCalls:new Map(), selected:{provider:'claude'}};
const runView = {observe() {}, statusNode: () => null}, providerFor = () => ({name:'Claude Code'}), humanLabel = value => value, brainReviewed = () => true;
const withoutMemoryDraft = text => text, bytesLabel = () => '';
""" + helpers + append + r"""
const render = stream => { events.children = []; state.eventIds.clear(); state.stepCalls.clear(); for (const event of streams[stream]) appendEvent(event);
  const groups = events.children.filter(node => node.className === 'activity-group');
  return {summaries:groups.map(group => group.firstElementChild.textContent), rows:groups.flatMap(group => group.lastElementChild.children.filter(node => node.className === 'step-row'))}; };
let {summaries, rows} = render('claude');
assert.deepEqual(summaries, ['1 step · Session',
  '13 steps · Read ×4, TodoWrite ×2, Grep, Glob, Write ×2, Edit ×2, Bash · Looked for "rules(", "**/*Order*.php" · opened 4 files · edited 3',
  '10 steps · Bash ×4, Task, Grep, Read ×2, Edit, TodoWrite · Looked for "foreignId" · opened 2 files · edited 1', '1 step · Usage']);
// Completions update their start row: one row per call, with the target in a left-to-right <bdi>.
assert.equal(rows.length, 23);
const row = call => rows.find(item => item.dataset.call === call), text = node => node.children.map(child => child.textContent).join(' · ');
assert.equal(text(row('toolu_01')), 'Read · app/Http/Controllers/OrderController.php · done');
assert.equal(text(row('toolu_12')), 'Bash · php artisan test --filter=OrderTest · failed');
assert.equal(text(row('toolu_16')), 'Helper · Grep · "foreignId" in database/migrations · done');
assert.deepEqual([row('toolu_01').children[1].tag, row('toolu_01').children[1].dir, row('toolu_12').dataset.state], ['bdi', 'ltr', 'failed']);
assert.ok(rows.every(item => Number.isInteger(item.dataset.eventId)));
assert.doesNotMatch(summaries.join(' '), /found|matches|results/);
// A label-only run: each completion stays its own row and is not a step; names come from the label.
({summaries, rows} = render('claude_labels'));
assert.equal(summaries[1], '23 steps · Read ×6, TodoWrite ×3, Grep ×2, Glob, Write ×2, Edit ×3, Bash ×5, Task');
assert.deepEqual(rows.slice(0, 2).map(text), ['Read · started', 'Tool · done']);
({summaries, rows} = render('codex'));
assert.deepEqual(rows.map(text).slice(0, 3), ['Shell · sed -n 1,200p app/Models/Order.php · exit 0', 'Shell · rg -n "rules(" app/Http · exit 0', 'Edit · app/Http/Requests/StoreOrderRequest.php · done']);
assert.ok(rows.map(text).includes('Shell · rm -rf build · not run'));
assert.deepEqual(rows.map(text).filter(line => line.startsWith('Agent')), ['Agent · spawn_agent · done']);
// A launch that raised ends on the worker's error line: a call it never answered says so instead of "running".
events.children = []; state.stepCalls.clear();
appendEvent({id:9001, kind:'tool', text:'Read: started', tool:'Read', call:'toolu_x', state:'started', ok:true, path:'app/Order.php', launch_id:'LX'});
appendEvent({id:9002, kind:'error', text:'Run failed (SessionError). Check the native CLI and server setup.', launch_id:'LX'});
assert.deepEqual([state.stepCalls.get('LX|toolu_x').row.dataset.state, text(state.stepCalls.get('LX|toolu_x').row)], ['unfinished', 'Read · app/Order.php · no result']);
// Codex numbers its items again in every resumed turn: item_1 of turn 2 is not turn 1's call.
events.children = []; state.stepCalls.clear();
appendEvent({id:9101, kind:'tool', text:'command_execution: started', tool:'Shell', call:'item_1', state:'started', ok:true, detail:'php artisan test', launch_id:'T1'});
appendEvent({id:9102, kind:'tool', text:'command_execution: completed', tool:'Shell', call:'item_1', state:'completed', ok:true, exit_code:0, detail:'php artisan test', launch_id:'T1'});
appendEvent({id:9103, kind:'tool', text:'file_change: completed', tool:'Edit', call:'item_1', state:'completed', ok:true, changes:[{path:'app/Order.php', kind:'update'}], launch_id:'T2'});
const turns = events.children.filter(node => node.className === 'activity-group').flatMap(group => group.lastElementChild.children);
assert.deepEqual(turns.map(text), ['Shell · php artisan test · exit 0', 'Edit · app/Order.php · done']);
assert.match(events.children[events.children.length - 1].firstElementChild.textContent, /^2 steps .* edited 1$/);
""")

    def test_run_view_scripts_render_text_only_and_guard_every_animation(self):
        for name in ("run-model.js", "run-view.js"):
            source = (WEB / name).read_text(encoding="utf-8")
            with self.subTest(script=name):
                self.assertIsNone(re.search(r"innerHTML|outerHTML|insertAdjacentHTML|document\.write", source))
                # The scripts strip bidi controls from agent text and must not carry any themselves (written as \u escapes).
                self.assertIsNone(re.search("[\u202a-\u202e\u2066-\u2069\u200e\u200f]", source))
                # Smooth scrolling follows the system setting through scrollMotion().
                self.assertEqual(re.findall(r"behavior:\s*(?!scrollMotion\(\))[^,}]+", source), [])
        view = (WEB / "run-view.js").read_text(encoding="utf-8")
        helper = view[view.index("  function motion("):view.index("\n  }\n", view.index("  function motion(")) + 4]
        self.assertIn("reducedMotion.matches", helper)
        self.assertEqual(view.count(".animate("), helper.count(".animate("), "every element.animate goes through motion()")
        # requestAnimationFrame only schedules a render or a guarded motion() call.
        for call in re.findall(r"requestAnimationFrame\(([^\n]{0,120})", view):
            self.assertTrue(call.startswith("() => { ui.raf = 0") or "motion(" in call, call)
        self.assertNotIn("innerHTML", (WEB / "app-core.js").read_text(encoding="utf-8")[(WEB / "app-core.js").read_text(encoding="utf-8").index("\nfunction stepGroup("):])

    def test_file_chips_read_as_text_on_their_fills_in_both_themes(self):
        def luminance(color):
            channels = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
            return sum(weight * (c / 12.92 if c <= .04045 else ((c + .055) / 1.055) ** 2.4) for weight, c in zip((.2126, .7152, .0722), channels))
        def contrast(a, b):
            high, low = sorted((luminance(a), luminance(b)), reverse=True)
            return (high + .05) / (low + .05)
        css = stylesheet(THEMED_PAGES[0])
        self.assertIn('.run-file[data-edited="true"] { background:var(--purple-soft); border-color:var(--purple); color:var(--purple); }', css)
        for block in (re.search(r"\n\s*:root \{([^}]*)\}", css).group(1), re.search(r':root\[data-theme="dark"\] \{([^}]*)\}', css).group(1)):
            tokens = dict(re.findall(r"(--[\w-]+):(#[0-9a-f]{6})\b", block)); tokens.setdefault("--paper", "#ffffff")
            for text, surface in (("--purple", "--purple-soft"), ("--ink-2", "--paper"), ("--error", "--paper"), ("--muted", "--soft"), ("--warning", "--warning-surface")):
                with self.subTest(text=text, surface=surface, paper=tokens["--paper"]):
                    self.assertGreaterEqual(contrast(tokens[text], tokens[surface]), 4.5, "chip and strip text needs 4.5:1")

    def test_a_glob_without_a_path_searches_from_its_leading_folders(self):
        self.run_node(r"""
const model = RunModel.create(); let id = 0;
const tool = fields => ({id:++id, kind:'tool', launch_id:'L1', at:'2026-10-04T21:00:00.000+00:00', state:'started', ok:true, ...fields});
RunModel.observe(model, [tool({text:'Glob: started', tool:'Glob', call:'g1', detail:'"app/**/*Order*.php"'}), tool({text:'Glob: started', tool:'Glob', call:'g2', detail:'"**/*.php"'}),
  tool({text:'Glob: started', tool:'Glob', call:'g3', detail:'"*.php"', path:'tests'}),
  tool({text:'Read: started', tool:'Read', call:'r1', path:'routes/api.php'}), tool({text:'Read: started', tool:'Read', call:'r2', path:'app/Models/Order.php'})]);
const run = RunModel.current(model);
assert.deepEqual(run.searches.map(item => item.scope), ['app', '', 'tests']);
// A pattern that starts in app/ was not run over routes/: the file card and the Searched filter say so.
assert.deepEqual(RunModel.searchedIn(run, run.paths.get('routes/api.php')), ['"**/*.php"']);
assert.deepEqual(RunModel.searchedIn(run, run.paths.get('app/Models/Order.php')), ['"app/**/*Order*.php"', '"**/*.php"']);
""")

    def test_hidden_text_in_receipts_and_panes_stays_inside_its_scroller(self):
        # .sr-only is absolutely positioned: inside a static receipt or pane it escaped the conversation's scroller and
        # made the page itself scrollable, so scrollIntoView or a focus could shift the whole app up.
        css = stylesheet(THEMED_PAGES[0])
        for selector in (".run-pane", ".run-receipt"):
            with self.subTest(selector=selector):
                self.assertRegex(css, re.escape(selector) + r" \{ position:relative;")


if __name__ == "__main__":
    unittest.main()
