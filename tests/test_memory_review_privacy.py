"""Fresh Tern privacy controls through native save, copied CLI/MCP and notices."""
from __future__ import annotations
import copy
import importlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_review_privacy_public.json').read_text(encoding='utf-8'))


class MemoryReviewPrivacyTests(unittest.TestCase):
    def setUp(self):
        clean = {k:v for k,v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        clean.update(GIT_OPTIONAL_LOCKS='0', PROJECT_BRAIN_OWNER='local', PYTHONIOENCODING='utf-8')
        environment=mock.patch.dict(os.environ,clean,clear=True); environment.start(); self.addCleanup(environment.stop)
        sys.path.insert(0,str(ROOT/'PHP Core/memory-bank/scripts'))
        try:
            self.result=importlib.import_module('memory_results'); self.brain=importlib.import_module('brain_runtime')
        finally:
            sys.path.pop(0)
        temporary=tempfile.TemporaryDirectory(prefix='tern-private-save-'); self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name)
        for folder in ('specs','project-brain/config','memory-bank/chunks'):
            (self.root/folder).mkdir(parents=True)
        (self.root/PUBLIC['source_path']).write_text(PUBLIC['source'],encoding='utf-8')
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps(
            {'mode':'governed','automatic_promotion':False}),encoding='utf-8')
        self.task=self.brain.create_task(self.root,PUBLIC['task_alias'],PUBLIC['task_title'],[],[],owner='local')
        self.data={'progress':PUBLIC['progress'],'next_steps':PUBLIC['next_steps'],
                   'learnings':[copy.deepcopy(PUBLIC['learning'])],'verified':True}
        self.save('tern-seed',self.data)
        identity=self.result.learning_id(self.task['id'],PUBLIC['learning'])
        self.leaf,self.learning,_=self.brain.find_record(self.root,identity)

    def save(self,identifier,data):
        return self.result.record_result(self.root,self.task['id'],identifier,'auto',data,
            owner='local',reason='Checked public Tern policy')

    def protect(self,*,privacy='private',archive=False):
        raw=self.leaf.read_text(encoding='utf-8'); pieces=raw.split('---',2); fields=json.loads(pieces[1])
        fields.update(title=PUBLIC['private_title'],privacy=privacy)
        pieces[1]='\n'+json.dumps(fields)+'\n'; self.leaf.write_text('---'.join(pieces),encoding='utf-8')
        if archive:
            destination=self.root/'project-brain/archive/findings'/self.leaf.name
            destination.parent.mkdir(parents=True); self.leaf.rename(destination); self.leaf=destination
        self.protected=self.leaf.read_bytes()

    def check_reply(self,reply):
        item=reply['records'][0]; expected=PUBLIC['expected_protected_record']
        self.assertEqual(expected,{k:item.get(k) for k in expected})
        self.assertEqual(self.result.learning_id(self.task['id'],PUBLIC['learning']),item['external_id'])
        rendered=json.dumps(reply,ensure_ascii=False)
        self.assertNotIn(PUBLIC['private_title'],rendered)
        self.assertNotIn(self.learning['id'],rendered)
        self.assertEqual(self.protected,self.leaf.read_bytes())

    def test_private_learning_explicit_cas_cannot_mutate_or_disclose_metadata(self):
        self.protect()
        data=copy.deepcopy(self.data);data['progress']='Tern independent next verification saved'
        data['learnings'][0].update(consequence='Tern restatement after source verification.',
                                     expected_revision=self.learning['revision'])
        data['learnings'].append(copy.deepcopy(PUBLIC['new_learning']))
        reply=self.save('tern-private-cas',data); self.check_reply(reply)
        self.assertEqual('created',reply['records'][1]['state'])
        self.assertEqual(data['progress'],self.brain.find_record(self.root,self.task['id'])[1]['progress'])

    def test_archived_private_learning_only_returns_incoming_sanitized_fields(self):
        self.protect(archive=True);reply=self.save('tern-private-archive',self.data);self.check_reply(reply)

    def test_completed_replay_hides_private_or_disallowed_restricted_current_image(self):
        for privacy,archive in (('private',False),('restricted',True)):
            with self.subTest(privacy=privacy,archive=archive):
                self.protect(privacy=privacy,archive=archive)
                before=self.brain.find_record(self.root,self.task['id'])[0].read_bytes()
                self.check_reply(self.save('tern-seed',self.data))
                self.assertEqual(before,self.brain.find_record(self.root,self.task['id'])[0].read_bytes())

    def test_default_promotion_hides_unrelated_private_id_but_keeps_public_outcomes(self):
        self.protect()
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps(
            {'mode':'governed','automatic_promotion':True}),encoding='utf-8')
        data=copy.deepcopy(self.data);data['learnings']=[copy.deepcopy(PUBLIC['new_learning'])]
        reply=self.save('tern-unrelated-private',data)
        self.assertEqual('created',reply['records'][0]['state'])
        public_id=reply['records'][0]['id'];self.assertIsNotNone(public_id)
        self.assertNotIn(self.learning['id'],json.dumps(reply))
        self.assertNotIn(PUBLIC['private_title'],json.dumps(reply))
        self.assertTrue(reply['promotion']['details_withheld'])
        self.assertTrue(any(item['record_id']==public_id for field in ('promoted','failed','blocked')
                            for item in reply['promotion'][field]),reply)
        self.assertEqual(self.protected,self.leaf.read_bytes())

    def test_copied_cli_mcp_and_harness_notice_never_echo_private_metadata(self):
        self.protect()
        # The packaged default promotes automatically; its blocked/closed
        # diagnostics must respect the same privacy scope as record summaries.
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps(
            {'mode':'governed','automatic_promotion':True}),encoding='utf-8')
        shutil.copytree(ROOT/'PHP Core/memory-bank/scripts',self.root/'memory-bank/scripts')
        request=self.root/'tern-request.json'; request.write_text(json.dumps(self.data),encoding='utf-8')
        command=[sys.executable,str(self.root/'memory-bank/scripts/context.py'),'--root',str(self.root),
                 'record-result','--json','--task-id',self.task['id'],'--result-id','tern-cli',
                 '--revision','auto','--input',str(request)]
        result=subprocess.run(command,cwd=self.root,capture_output=True,timeout=30)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'))
        reply=json.loads(result.stdout);self.check_reply(reply)
        revision=self.brain.find_record(self.root,self.task['id'])[1]['revision']
        messages=[{'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-06-18'}},
            {'jsonrpc':'2.0','method':'notifications/initialized'},
            {'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':'memory_record_result',
             'arguments':{'task_id':self.task['id'],'result_id':'tern-mcp','revision':revision,**self.data}}}]
        result=subprocess.run([sys.executable,str(self.root/'memory-bank/scripts/mcp_server.py'),
            '--root',str(self.root)],input=''.join(json.dumps(m)+'\n' for m in messages).encode(),
            cwd=self.root,capture_output=True,timeout=30)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'))
        response=json.loads(result.stdout.splitlines()[-1])['result'];self.assertFalse(response['isError'])
        self.check_reply(response['structuredContent'])
        self.assertNotIn(PUBLIC['private_title'],result.stdout.decode())
        sys.path.insert(0,str(ROOT/'harness/src'))
        try:
            from harness import memory_draft
            from harness.task_context import TaskContext
        finally:
            sys.path.pop(0)
        outcome=TaskContext._memory_outcome(reply,[])
        event={'kind':'memory','text':memory_draft.summary('ready',outcome),'result':outcome}
        self.assertNotIn(PUBLIC['private_title'],json.dumps(event))
        self.assertNotIn(self.learning['id'],json.dumps(event))
        self.assertIn(PUBLIC['learning']['title'],event['text'])


if __name__=='__main__':unittest.main()
