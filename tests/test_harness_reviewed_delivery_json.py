"""Fresh reviewed-delivery formatting checks; no provider or retrieval execution."""
import ast
import copy
import json
from pathlib import Path
import sqlite3
import sys
import threading
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'harness'/'src'))
from harness import sessions,cost_observation

CAPSULE={'query': 'Lossless delivery example', 'working': {'goal': 'Continue the public note', 'details': ' leading and trailing  ', 'steps': ['second', 'first']}, 'procedural': [{'category': 'policy', 'source': 'policies/reference.md', 'snippet': 'Сверить правило; пробелы  внутри строки.', 'score': 7, 'tags': ['alpha', 'beta']}], 'semantic': [{'category': 'durable', 'source': 'chunks/example.md', 'snippet': 'Keep every value: \\ and : and \n next line.', 'score': 2, 'optional': None}], 'episodic': [{'category': 'dynamic', 'source': 'records/work.md', 'snippet': 'Delivery fixture №3 — supplied data.', 'flags': {'ready': True, 'pending': False}}], 'selected': [{'category': 'policy', 'source': 'policies/reference.md', 'snippet': 'Сверить правило; пробелы  внутри строки.', 'score': 7, 'tags': ['alpha', 'beta']}, {'category': 'durable', 'source': 'chunks/example.md', 'snippet': 'Keep every value: \\ and : and \n next line.', 'score': 2, 'optional': None}, {'category': 'dynamic', 'source': 'records/work.md', 'snippet': 'Delivery fixture №3 — supplied data.', 'flags': {'ready': True, 'pending': False}}], 'categories': {'policy': [{'category': 'policy', 'source': 'policies/reference.md', 'snippet': 'Сверить правило; пробелы  внутри строки.', 'score': 7, 'tags': ['alpha', 'beta']}], 'durable': [{'category': 'durable', 'source': 'chunks/example.md', 'snippet': 'Keep every value: \\ and : and \n next line.', 'score': 2, 'optional': None}], 'dynamic': [{'category': 'dynamic', 'source': 'records/work.md', 'snippet': 'Delivery fixture №3 — supplied data.', 'flags': {'ready': True, 'pending': False}}]}, 'omitted': {'episodic': 1}, 'format_extra': {'integer': 0, 'null': None, 'boolean': False}}
ORDINARY_BODY='{"query": "Lossless delivery example", "working": {"goal": "Continue the public note", "details": " leading and trailing  ", "steps": ["second", "first"]}, "procedural": [{"category": "policy", "source": "policies/reference.md", "snippet": "Сверить правило; пробелы  внутри строки.", "score": 7, "tags": ["alpha", "beta"]}], "semantic": [{"category": "durable", "source": "chunks/example.md", "snippet": "Keep every value: \\\\ and : and \\n next line.", "score": 2, "optional": null}], "episodic": [{"category": "dynamic", "source": "records/work.md", "snippet": "Delivery fixture №3 — supplied data.", "flags": {"ready": true, "pending": false}}], "selected": [{"category": "policy", "source": "policies/reference.md", "snippet": "Сверить правило; пробелы  внутри строки.", "score": 7, "tags": ["alpha", "beta"]}, {"category": "durable", "source": "chunks/example.md", "snippet": "Keep every value: \\\\ and : and \\n next line.", "score": 2, "optional": null}, {"category": "dynamic", "source": "records/work.md", "snippet": "Delivery fixture №3 — supplied data.", "flags": {"ready": true, "pending": false}}], "categories": {"policy": [{"category": "policy", "source": "policies/reference.md", "snippet": "Сверить правило; пробелы  внутри строки.", "score": 7, "tags": ["alpha", "beta"]}], "durable": [{"category": "durable", "source": "chunks/example.md", "snippet": "Keep every value: \\\\ and : and \\n next line.", "score": 2, "optional": null}], "dynamic": [{"category": "dynamic", "source": "records/work.md", "snippet": "Delivery fixture №3 — supplied data.", "flags": {"ready": true, "pending": false}}]}, "omitted": {"episodic": 1}, "format_extra": {"integer": 0, "null": null, "boolean": false}}'
DELIVERY_BODY='{"query":"Lossless delivery example","working":{"goal":"Continue the public note","details":" leading and trailing  ","steps":["second","first"]},"procedural":[{"category":"policy","source":"policies/reference.md","snippet":"Сверить правило; пробелы  внутри строки.","score":7,"tags":["alpha","beta"]}],"semantic":[{"category":"durable","source":"chunks/example.md","snippet":"Keep every value: \\\\ and : and \\n next line.","score":2,"optional":null}],"episodic":[{"category":"dynamic","source":"records/work.md","snippet":"Delivery fixture №3 — supplied data.","flags":{"ready":true,"pending":false}}],"selected":[{"category":"policy","source":"policies/reference.md","snippet":"Сверить правило; пробелы  внутри строки.","score":7,"tags":["alpha","beta"]},{"category":"durable","source":"chunks/example.md","snippet":"Keep every value: \\\\ and : and \\n next line.","score":2,"optional":null},{"category":"dynamic","source":"records/work.md","snippet":"Delivery fixture №3 — supplied data.","flags":{"ready":true,"pending":false}}],"categories":{"policy":[{"category":"policy","source":"policies/reference.md","snippet":"Сверить правило; пробелы  внутри строки.","score":7,"tags":["alpha","beta"]}],"durable":[{"category":"durable","source":"chunks/example.md","snippet":"Keep every value: \\\\ and : and \\n next line.","score":2,"optional":null}],"dynamic":[{"category":"dynamic","source":"records/work.md","snippet":"Delivery fixture №3 — supplied data.","flags":{"ready":true,"pending":false}}]},"omitted":{"episodic":1},"format_extra":{"integer":0,"null":null,"boolean":false}}'
ORDINARY_ENVELOPE='Project memory (reference data; verify against canonical project sources):\n{"query": "Lossless delivery example", "working": {"goal": "Continue the public note", "details": " leading and trailing  ", "steps": ["second", "first"]}, "procedural": [{"category": "policy", "source": "policies/reference.md", "snippet": "Сверить правило; пробелы  внутри строки.", "score": 7, "tags": ["alpha", "beta"]}], "semantic": [{"category": "durable", "source": "chunks/example.md", "snippet": "Keep every value: \\\\ and : and \\n next line.", "score": 2, "optional": null}], "episodic": [{"category": "dynamic", "source": "records/work.md", "snippet": "Delivery fixture №3 — supplied data.", "flags": {"ready": true, "pending": false}}], "selected": [{"category": "policy", "source": "policies/reference.md", "snippet": "Сверить правило; пробелы  внутри строки.", "score": 7, "tags": ["alpha", "beta"]}, {"category": "durable", "source": "chunks/example.md", "snippet": "Keep every value: \\\\ and : and \\n next line.", "score": 2, "optional": null}, {"category": "dynamic", "source": "records/work.md", "snippet": "Delivery fixture №3 — supplied data.", "flags": {"ready": true, "pending": false}}], "categories": {"policy": [{"category": "policy", "source": "policies/reference.md", "snippet": "Сверить правило; пробелы  внутри строки.", "score": 7, "tags": ["alpha", "beta"]}], "durable": [{"category": "durable", "source": "chunks/example.md", "snippet": "Keep every value: \\\\ and : and \\n next line.", "score": 2, "optional": null}], "dynamic": [{"category": "dynamic", "source": "records/work.md", "snippet": "Delivery fixture №3 — supplied data.", "flags": {"ready": true, "pending": false}}]}, "omitted": {"episodic": 1}, "format_extra": {"integer": 0, "null": null, "boolean": false}}\n\n'
DELIVERY_ENVELOPE='Project memory (reference data; verify against canonical project sources):\n{"query":"Lossless delivery example","working":{"goal":"Continue the public note","details":" leading and trailing  ","steps":["second","first"]},"procedural":[{"category":"policy","source":"policies/reference.md","snippet":"Сверить правило; пробелы  внутри строки.","score":7,"tags":["alpha","beta"]}],"semantic":[{"category":"durable","source":"chunks/example.md","snippet":"Keep every value: \\\\ and : and \\n next line.","score":2,"optional":null}],"episodic":[{"category":"dynamic","source":"records/work.md","snippet":"Delivery fixture №3 — supplied data.","flags":{"ready":true,"pending":false}}],"selected":[{"category":"policy","source":"policies/reference.md","snippet":"Сверить правило; пробелы  внутри строки.","score":7,"tags":["alpha","beta"]},{"category":"durable","source":"chunks/example.md","snippet":"Keep every value: \\\\ and : and \\n next line.","score":2,"optional":null},{"category":"dynamic","source":"records/work.md","snippet":"Delivery fixture №3 — supplied data.","flags":{"ready":true,"pending":false}}],"categories":{"policy":[{"category":"policy","source":"policies/reference.md","snippet":"Сверить правило; пробелы  внутри строки.","score":7,"tags":["alpha","beta"]}],"durable":[{"category":"durable","source":"chunks/example.md","snippet":"Keep every value: \\\\ and : and \\n next line.","score":2,"optional":null}],"dynamic":[{"category":"dynamic","source":"records/work.md","snippet":"Delivery fixture №3 — supplied data.","flags":{"ready":true,"pending":false}}]},"omitted":{"episodic":1},"format_extra":{"integer":0,"null":null,"boolean":false}}\n\n'
DELIVERY_PARTS={'rules': 423, 'bank': 402, 'brain': 829}
DELIVERY_METER={'characters': 1577, 'limit': 8000, 'kinds': {'brain': 752, 'rules': 423, 'bank': 402}, 'items': {'brain': 1, 'rules': 1, 'bank': 1}, 'repeats': 824, 'dropped': {'episodic': 1}, 'prompt_characters': 1654}
ORDINARY_METER={'characters': 1577, 'limit': 8000, 'kinds': {'brain': 752, 'rules': 423, 'bank': 402}, 'items': {'brain': 1, 'rules': 1, 'bank': 1}, 'repeats': 824, 'dropped': {'episodic': 1}, 'prompt_characters': 1777}
BYTE_COUNTS={'ordinary_body_bytes': 1811, 'delivery_body_bytes': 1688, 'ordinary_body_characters': 1700, 'delivery_body_characters': 1577, 'saving_bytes': 123, 'saving_characters': 123}
MISSING=object()

class ReviewedDeliveryTests(unittest.TestCase):
    def prompt(self,review=MISSING,text=None,delivery=False,route=None,capsule=CAPSULE):
        manager=object.__new__(sessions.Sessions)
        brain={'capsule':copy.deepcopy(capsule),'capsule_text':text,'task_id':'public-delivery-task'} if capsule is not None else None
        if brain is not None and review is not MISSING:brain['review']=review
        settings={'workflow':'native','provider':'codex','project_context':False,'agents_enabled':False,
                  'agent_count':1,'thinking_effort':'ultra','budgets':{},'brain':brain}
        original=copy.deepcopy(settings)
        ledger={};observation=cost_observation.LaunchObservation('codex')
        prompt=manager._prompt(settings,'Complete the public delivery note',ledger,route,observation,delivery=delivery)
        self.assertEqual(settings,original)
        return prompt,ledger,observation

    def getter(self,brain):
        manager=object.__new__(sessions.Sessions)
        manager.lock=threading.RLock();manager.timeout=900
        manager.db=sqlite3.connect(':memory:');manager.db.row_factory=sqlite3.Row
        self.addCleanup(manager.db.close)
        fields=('fleet','fleet_result','brain','creator','system_run','system_discovery','budgets','budget_usage',
                'result_base','sdd','model_routing','clash','clash_result','cost_totals')
        manager.db.execute('CREATE TABLE sessions (id TEXT PRIMARY KEY,project_context INTEGER,agents_enabled INTEGER,agent_count INTEGER,'+
                           ','.join(field+' TEXT' for field in fields)+')')
        manager.db.execute('CREATE TABLE session_merges (session_id TEXT,summary TEXT)')
        row={'id':'public-session','project_context':0,'agents_enabled':0,'agent_count':1,
             **{field:None for field in fields},'brain':json.dumps(brain,ensure_ascii=False),
             'budgets':json.dumps({'seconds':None,'usd':None,'tokens':None})}
        manager.db.execute('INSERT INTO sessions ('+','.join(row)+') VALUES ('+','.join('?' for field in row)+')',tuple(row.values()))
        return manager

    def test_reviewed_legacy_default_and_real_delivery_are_lossless(self):
        for review in (MISSING,True,None):
            with self.subTest(review='missing' if review is MISSING else review):
                old,old_ledger,old_observation=self.prompt(review)
                new,ledger,observation=self.prompt(review,delivery=True)
                self.assertTrue(old.startswith(ORDINARY_ENVELOPE));self.assertTrue(new.startswith(DELIVERY_ENVELOPE))
                body=new[len(sessions.BRAIN_CONTEXT_HEADER):len(DELIVERY_ENVELOPE)-2]
                self.assertEqual(body,DELIVERY_BODY);self.assertEqual(json.loads(body),CAPSULE)
                self.assertEqual(json.loads(body)['working']['steps'],['second','first'])
                self.assertEqual(json.loads(body)['working']['details'],' leading and trailing  ')
                self.assertEqual(len(old.encode())-len(new.encode()),123)
                self.assertEqual(old_ledger['instructions'],ledger['instructions'])
                self.assertEqual(old_observation.payloads['draft_instruction'],observation.payloads['draft_instruction'])

    def test_actual_prompt_parts_and_cost_metadata_use_full_literal_oracles(self):
        prompt,ledger,observation=self.prompt(True,delivery=True)
        self.assertEqual(ledger['capsule'],{'characters':DELIVERY_METER['characters'],'inserted':DELIVERY_METER['prompt_characters'],
             'kinds':DELIVERY_PARTS,'repeats':DELIVERY_METER['repeats'],'dropped':DELIVERY_METER['dropped'],'items':DELIVERY_METER['items']})
        self.assertEqual(sum(DELIVERY_PARTS.values()),len(DELIVERY_ENVELOPE))
        self.assertEqual(ledger['total'],len(prompt))
        self.assertEqual(ledger['instructions']+ledger['message']+ledger['capsule']['inserted'],ledger['total'])
        self.assertEqual(observation.payloads['capsule_body']['characters'],BYTE_COUNTS['delivery_body_characters'])
        self.assertEqual(observation.payloads['capsule_body']['bytes'],BYTE_COUNTS['delivery_body_bytes'])
        self.assertEqual(observation.payloads['capsule_envelope']['characters'],len(DELIVERY_ENVELOPE))
        self.assertEqual(observation.payloads['capsule_envelope']['bytes'],len(DELIVERY_ENVELOPE.encode()))
        self.assertNotIn('text',observation.payloads['capsule_body'])
        cost_observation.safe(observation,'payload','main_prompt',prompt)
        self.assertEqual(observation.payloads['main_prompt']['characters'],len(prompt))
        self.assertEqual(observation.payloads['main_prompt']['bytes'],len(prompt.encode()))

    def test_old_helpers_and_unattended_delivery_defaults_are_preserved(self):
        self.assertEqual(sessions.capsule_prompt_text(CAPSULE),ORDINARY_BODY)
        self.assertEqual(sessions.capsule_envelope(CAPSULE),ORDINARY_ENVELOPE)
        self.assertEqual(sessions.capsule_meter(CAPSULE),ORDINARY_METER)
        self.assertEqual(sessions.capsule_meter(CAPSULE,compact=True),DELIVERY_METER)
        old,old_ledger,old_observation=self.prompt(False)
        new,ledger,observation=self.prompt(False,delivery=True)
        self.assertEqual(old,new);self.assertEqual(old_ledger,ledger)
        self.assertEqual(old_observation.payloads,observation.payloads)
        truthy,_,_=self.prompt(True,delivery=1)
        self.assertTrue(truthy.startswith(ORDINARY_ENVELOPE))

    def test_rendered_precedence_and_empty_rendered_fallback(self):
        rendered=' \nNew public rendered note: preserve  internal spaces.\nSecond line.  \n'
        old,old_ledger,old_observation=self.prompt(True,rendered)
        new,ledger,observation=self.prompt(True,rendered,True)
        self.assertEqual(new,old);self.assertEqual(ledger,old_ledger);self.assertEqual(observation.payloads,old_observation.payloads)
        self.assertTrue(new.startswith(sessions.BRAIN_CONTEXT_HEADER+rendered.strip()+'\n\n'))
        for text in (None,'',' \n\t ',False):
            with self.subTest(type=type(text).__name__):
                prompt,_,_=self.prompt(True,text,True);self.assertTrue(prompt.startswith(DELIVERY_ENVELOPE))

    def test_native_command_and_absent_capsule_keep_existing_paths(self):
        route={'mode':'native','text':'/public-command','requests':[]}
        old,old_ledger,old_observation=self.prompt(True,route=route)
        new,ledger,observation=self.prompt(True,delivery=True,route=route)
        self.assertEqual(new,'Complete the public delivery note');self.assertEqual(new,old)
        self.assertEqual(ledger,old_ledger);self.assertEqual(observation.payloads,old_observation.payloads)
        self.assertIsNone(ledger['capsule']);self.assertNotIn('capsule_body',observation.payloads)
        old,old_ledger,old_observation=self.prompt(capsule=None)
        new,ledger,observation=self.prompt(delivery=True,capsule=None)
        self.assertEqual(new,old);self.assertEqual(ledger,old_ledger);self.assertEqual(observation.payloads,old_observation.payloads)

    def test_real_get_preserves_old_meter_and_capsule_adds_delivery_meter(self):
        for review in (MISSING,True,None,False):
            brain={'capsule':copy.deepcopy(CAPSULE),'capsule_text':None,'task_id':'public-delivery-task'}
            if review is not MISSING:brain['review']=review
            result=self.getter(brain).get('public-session')
            self.assertEqual(result['brain'],brain)
            self.assertEqual(result['capsule_meter'],DELIVERY_METER if review is False else ORDINARY_METER)
            self.assertEqual(result['capsule_delivery_meter'],DELIVERY_METER)
            self.assertEqual({key:value for key,value in result['capsule_meter'].items() if key!='prompt_characters'},
                             {key:value for key,value in result['capsule_delivery_meter'].items() if key!='prompt_characters'})

    def test_real_get_missing_and_rendered_capsule_meter_paths(self):
        for brain in (None,{'task_id':'public-delivery-task'}, {'capsule':None}):
            result=self.getter(brain).get('public-session')
            self.assertIsNone(result['capsule_meter']);self.assertIsNone(result['capsule_delivery_meter'])
        brain={'capsule':copy.deepcopy(CAPSULE),'capsule_text':' New rendered public view ','review':True}
        result=self.getter(brain).get('public-session')
        self.assertEqual(result['capsule_meter'],result['capsule_delivery_meter'])

    def test_notice_defaults_and_delivery_counts_keep_all_items(self):
        brain={'capsule':copy.deepcopy(CAPSULE),'capsule_text':None,'review':True,'task_id':'public-delivery-task'}
        ordinary=sessions.memory_notice(brain);delivery=sessions.memory_notice(brain,delivery=True)
        self.assertIn('(1,777 characters)',ordinary);self.assertIn('(1,654 characters)',delivery)
        prefix='Project memory for this turn: task public-delivery-task, 1 rule or doc excerpt, 1 Memory Bank chunk, 1 Project Brain item'
        self.assertTrue(delivery.startswith(prefix));self.assertTrue(ordinary.startswith(prefix))
        brain['review']=False;self.assertEqual(sessions.memory_notice(brain),sessions.memory_notice(brain,delivery=True))

    def test_production_launch_wiring_uses_delivery_without_gate_changes(self):
        root=Path(__file__).resolve().parents[1]
        source=(root/'harness/src/harness/sessions.py').read_text();tree=ast.parse(source)
        body=next(node for node in ast.walk(tree) if isinstance(node,ast.FunctionDef) and node.name=='_run_body')
        calls=[node for node in ast.walk(body) if isinstance(node,ast.Call) and isinstance(node.func,ast.Attribute) and node.func.attr=='_prompt']
        self.assertEqual(len(calls),3)
        for call in calls:
            delivery=next(keyword.value for keyword in call.keywords if keyword.arg=='delivery')
            self.assertIsInstance(delivery,ast.Constant);self.assertIs(delivery.value,True)
        recall=next(node for node in ast.walk(tree) if isinstance(node,ast.FunctionDef) and node.name=='_recall')
        notices=[node for node in ast.walk(recall) if isinstance(node,ast.Call) and isinstance(node.func,ast.Name) and node.func.id=='memory_notice']
        self.assertEqual(len(notices),1)
        self.assertIs(next(keyword.value.value for keyword in notices[0].keywords if keyword.arg=='delivery'),True)
        web=(root/'harness/web/app-knowledge.js').read_text()
        self.assertIn('session.capsule_delivery_meter || session.capsule_meter',web)
        self.assertIn('JSON.stringify(capsule,null,2)',web)

if __name__=='__main__':unittest.main()
