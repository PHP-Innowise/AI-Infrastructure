"""NEW Aster actual CLI/MCP/promotion/hook integration; no model calls."""
from datetime import datetime,timedelta,timezone
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from uuid import UUID

ROOT = Path(__file__).resolve().parents[1]
P = json.loads((ROOT/'tests/fixtures/memory_native_roundtrip_public.json').read_text(encoding='utf-8'))


class NativeRoundtripTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='aster-native-roundtrip-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)/'project'
        self.root.mkdir()
        self.env = {k:os.environ[k] for k in ('PATH','HOME','LANG','LC_ALL','LC_CTYPE',
                    'SystemRoot','WINDIR','COMSPEC','PATHEXT') if k in os.environ}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',
                        TMPDIR=temporary.name,XDG_CONFIG_HOME=str(self.root/'xdg'),PROJECT_BRAIN_OWNER=P['owner'])
        if os.name=='posix':
            private_bin = Path(temporary.name)/'runtime-bin'
            private_bin.mkdir()
            (private_bin/'python3').symlink_to(sys.executable)
            self.env['PATH'] = str(private_bin)+os.pathsep+self.env.get('PATH','')
        self.run_process(['git','init','-q','-b','aster-native'])
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config',
                    'project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,
                            ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        (self.root/'memory-bank/chunks').mkdir()
        (self.root/'memory-bank/README.md').write_bytes(b'# Aster public Memory Bank\n')
        (self.root/'memory-bank/INDEX.md').write_bytes(b'# Aster public index\n')
        (self.root/'.gitignore').write_bytes(b'/memory-bank/local/\n/project-brain/local/\n')
        cfg_path = self.root/'project-brain/config/runtime.json'
        cfg = json.loads(cfg_path.read_text(encoding='utf-8'))
        cfg.update(mode='governed',provider='sqlite-fts5',owners=[P['owner']],
                   allowed_privacy=['public','team'],allowed_authority=['observed','verified'],
                   automatic_promotion=True,automatic_completion=False,automatic_compaction=False,retrieval_gate='off')
        cfg_path.write_bytes(json.dumps(cfg).encode('utf-8'))
        self.cli_file = self.root/'memory-bank/scripts/context.py'
        self.assertIn(b'record-result',self.run_process([sys.executable,'-B',str(self.cli_file),'--help']).stdout)
        self.queries = 0

    def run_process(self,command,data=b''):
        result = subprocess.run(command,cwd=self.root,env=self.env,input=data,capture_output=True,timeout=30)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'))
        self.assertEqual(b'',result.stderr)
        return result

    def cli(self,*args,data=b''):
        return json.loads(self.run_process([sys.executable,'-B',str(self.cli_file),*args],data).stdout)

    def prepare(self,name):
        self.case = P['cases'][name]
        self.source = self.root/self.case['source']
        self.source.parent.mkdir(parents=True)
        self.raw_source = ('<?php\n// '+self.case['fact']+'\n').encode('utf-8')
        self.source.write_bytes(self.raw_source)
        self.source_image = (self.source.read_bytes(),self.source.stat().st_mtime_ns)
        task = self.cli('start','--task-id',self.case['task'],'--goal','Verify '+self.case['title'],'--json')
        self.task_uuid = task['task_uuid']
        self.assertEqual(4,UUID(self.task_uuid).version)
        created,updated = task.pop('created_at'),task.pop('updated_at')
        self.assertEqual(created,updated)
        self.assertIsNotNone(datetime.fromisoformat(created).tzinfo)
        self.assertEqual({'task_id':self.case['task'],'task_uuid':self.task_uuid,'phase':None,'revision':1,
            'status':'active','goal':'Verify '+self.case['title'],'progress':'','auto_checkpoint':None,
            'next_steps':[],'files':[],'sources':[],'authority':'project-brain'},task)
        self.payload = {'progress':P['progress'],'next_steps':P['next_steps'],'verified':True,
            'learnings':[{'type':'finding','title':self.case['title'],'consequence':self.case['fact'],
                          'sources':[self.case['source']]}]}

    def mcp(self,name,arguments):
        requests = [{'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-06-18'}},
                    {'jsonrpc':'2.0','method':'notifications/initialized'},
                    {'jsonrpc':'2.0','id':2,'method':'tools/call','params':{'name':name,'arguments':arguments}}]
        raw = b''.join(json.dumps(value,ensure_ascii=False).encode('utf-8')+b'\n' for value in requests)
        server = self.root/'memory-bank/scripts/mcp_server.py'
        result = self.run_process([sys.executable,'-B',str(server),'--root',str(self.root),'--host','cli'],raw)
        lines = result.stdout.decode('utf-8').splitlines()
        self.assertEqual(2,len(lines))
        initialized,called = map(json.loads,lines)
        self.assertEqual({'jsonrpc':'2.0','id':1,'result':P['initialize']},initialized)
        self.assertEqual({'jsonrpc','id','result'},set(called))
        self.assertEqual('2.0',called['jsonrpc'])
        self.assertEqual(2,called['id'])
        packet = called['result']
        if packet['isError']:
            self.assertEqual({'content':[{'type':'text','text':P['generic_mcp_error']}],'isError':True},packet)
            return packet
        self.assertEqual({'content','structuredContent','isError'},set(packet))
        self.assertEqual([{'type':'text','text':json.dumps(packet['structuredContent'],ensure_ascii=False)}],packet['content'])
        self.assertIs(False,packet['isError'])
        return packet['structuredContent']

    def saved(self,result):
        self.assertEqual({'task','records','promotion','attestation','replayed'},set(result))
        self.assertEqual({'id':self.task_uuid,'revision':2},result['task'])
        self.assertEqual('agent',result['attestation'])
        self.assertIs(False,result['replayed'])
        self.assertEqual(1,len(result['records']))
        record = result['records'][0]
        self.record_uuid = record['id']
        self.assertEqual(4,UUID(self.record_uuid).version)
        alias_raw = json.dumps([self.task_uuid,'finding',' '.join(self.case['title'].casefold().split()),[self.case['source']]],ensure_ascii=False).encode('utf-8')
        alias = 'learning-'+hashlib.sha256(alias_raw).hexdigest()[:24]
        self.assertEqual({'id':self.record_uuid,'external_id':alias,'type':'finding','title':self.case['title'],
                          'status':'resolved','revision':2,'state':'created'},record)
        today = datetime.now(timezone.utc).date()
        self.memory_id = 'MEM-'+today.strftime('%Y%m%d')+'-'+self.record_uuid.replace('-','')[:8]
        self.assertEqual({'enabled':True,'promoted':[{'record_id':self.record_uuid,'type':'finding','memory_id':self.memory_id}],
                          'failed':[],'blocked':[],'closed':[],'skipped':0},result['promotion'])
        slug = re.sub('[^a-z0-9]+','-',self.case['title'].lower()).strip('-')
        self.chunk_rel = 'memory-bank/chunks/'+self.memory_id+'-'+slug+'.md'
        record_rel = 'project-brain/dynamic/findings/'+self.record_uuid+'.md'
        sources = sorted([record_rel,self.case['source']])
        digests = [{'path':n,'sha256':hashlib.sha256((self.root/n).read_bytes()).hexdigest()} for n in sources]
        metadata = {'id':self.memory_id,'title':self.case['title'],'type':'domain','status':'active',
            'scope':['application'],'tags':['project-brain','promoted','auto-promoted','agent-attested'],
            'created':today.isoformat(),'last_verified':today.isoformat(),'review_after':(today+timedelta(days=365)).isoformat(),
            'sources':sources,'supersedes':[],'superseded_by':None,'valid_from':today.isoformat(),'valid_to':None,
            'source_digests':digests}
        body = '# '+self.case['title']+'\n\n'+self.case['fact']+'\n\n## Sources\n- '+self.case['source']+'\n'
        expected = ('---\n'+json.dumps(metadata,ensure_ascii=False,indent=2,sort_keys=True)+'\n---\n\n'+body).encode('utf-8')
        self.assertEqual(expected,(self.root/self.chunk_rel).read_bytes())
        self.assertEqual([self.chunk_rel],[p.relative_to(self.root).as_posix() for p in (self.root/'memory-bank/chunks').glob('*.md')])
        index_rows = [line for line in (self.root/'memory-bank/INDEX.md').read_text(encoding='utf-8').splitlines() if line.startswith('| MEM-')]
        self.assertEqual(['| '+self.memory_id+' | '+self.case['title']+' | domain | application | project-brain, promoted, auto-promoted, agent-attested | active | '+today.isoformat()+' | chunks/'+Path(self.chunk_rel).name+' |'],index_rows)

    def corrupt_neighbor(self):
        p = self.root/P['bad_document']
        p.parent.mkdir()
        p.write_bytes(bytes.fromhex(P['bad_bytes_hex']))

    def recall(self,via,expected_fact=True):
        self.queries += 1
        if via=='mcp':
            value = self.mcp('memory_retrieve',{'task_id':self.case['task'],'query':self.case['title']})
        else:
            value = self.cli('refresh','--task-id',self.case['task'],'--query',self.case['title'],'--gate','off','--ephemeral','--json')
        self.assertNotIn('isError',value)
        self.assertEqual('governed',value['mode'])
        for layer in ('procedural','semantic','episodic'):
            self.assertEqual('updated',value[layer])
        self.assertEqual([],value['warnings'])
        self.assertEqual({'invalid-utf8':1},value['source_notices'])
        capsule = value['capsule']
        self.assertEqual(self.task_uuid,capsule['task_uuid'])
        self.assertEqual(2,capsule['task_revision'])
        self.assertEqual(P['progress'],capsule['working']['progress'])
        self.assertEqual(P['next_steps'],capsule['working']['next_steps'])
        self.assertEqual({'invalid-utf8':1},capsule['source_notices'])
        expected_paths = [self.chunk_rel] if expected_fact else []
        self.assertEqual(expected_paths,[item['path'] for item in capsule['semantic']])
        self.assertEqual(expected_paths,[item['path'] for item in capsule['selected']])
        self.assertEqual([],capsule['procedural'])
        self.assertEqual([],capsule['episodic'])
        manifest = json.loads((self.root/capsule['manifest']).read_text(encoding='utf-8'))
        self.assertEqual(expected_paths,[item['path'] for item in manifest['selected']])
        for item in manifest['selected']:
            self.assertEqual('durable',item['category'])
            self.assertEqual(hashlib.sha256((self.root/item['path']).read_bytes()).hexdigest(),item['source_hash'])
        for item in capsule['selected']:
            self.assertEqual(self.case['title'],item['title'])
            self.assertEqual('memory',item['kind'])
            self.assertEqual('semantic',item['layer'])
            self.assertEqual('durable',item['category'])
            self.assertIsNone(item['record_id'])
            self.assertEqual([],item['conflicts'])
            self.assertIs(type(item['estimated_tokens']),int)
            self.assertGreater(item['estimated_tokens'],0)
        rendered = value['capsule_text']
        self.assertTrue(rendered.startswith('working: '+self.case['task']))
        self.assertIn(P['notice'],rendered)
        self.assertEqual(expected_fact,self.case['fact'] in rendered)
        self.assertNotIn('ASTER_PUBLIC_DECODE_CANARY',json.dumps(value))
        self.assertNotIn(P['bad_document'],rendered)
        return value

    def authority_image(self):
        image = {str(p.relative_to(self.root)):(p.read_bytes(),stat.S_IMODE(p.stat().st_mode)) for base in
                ('project-brain/dynamic','project-brain/control','project-brain/indexes','memory-bank/chunks')
                for p in (self.root/base).rglob('*') if p.is_file()}
        index = self.root/'memory-bank/INDEX.md'
        image['memory-bank/INDEX.md'] = (index.read_bytes(),stat.S_IMODE(index.stat().st_mode))
        return image

    def test_cli_save_promotes_then_mcp_recall_tracks_source_presence(self):
        self.prepare('cli')
        saved = self.cli('record-result','--task-id',self.case['task'],'--result-id','aster-cli-save-01',
                         '--revision','1','--input','-','--json',data=json.dumps(self.payload).encode('utf-8'))
        self.saved(saved)
        self.corrupt_neighbor()
        self.recall('mcp')
        before = self.authority_image()
        self.source.unlink()
        self.recall('mcp',False)
        self.source.write_bytes(self.raw_source)
        self.recall('mcp')
        self.assertEqual(before,self.authority_image())
        self.assertEqual(P['query_horizon_cli'],self.queries)

    def test_mcp_save_promotes_then_cli_recall_and_private_request_refusal(self):
        self.prepare('mcp')
        saved = self.mcp('memory_record_result',{'task_id':self.case['task'],'result_id':'aster-mcp-save-01',
                                               'revision':1,**self.payload})
        self.saved(saved)
        self.corrupt_neighbor()
        self.recall('cli')
        before = self.authority_image()
        self.queries += 1
        refused = self.mcp('memory_retrieve',{'task_id':self.case['task'],'query':P['private_query']})
        self.assertTrue(refused['isError'])
        self.assertNotIn(P['private_query'],json.dumps(refused))
        self.assertEqual(before,self.authority_image())
        self.assertEqual(P['query_horizon_mcp'],self.queries)

    @unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'Native POSIX hook profile')
    def test_saved_fact_claude_codex_native_hook_outputs_match(self):
        self.prepare('hooks')
        saved = self.cli('record-result','--task-id',self.case['task'],'--result-id','aster-hooks-save-01',
                         '--revision','1','--input','-','--json',data=json.dumps(self.payload).encode('utf-8'))
        self.saved(saved)
        self.corrupt_neighbor()
        before = self.authority_image()
        outputs = []
        self.env['CONTEXT_TASK_ID'] = self.case['task']
        for tool in ('claude','codex'):
            hook = self.root/('.'+tool)/'hooks/working-memory-read.sh'
            hook.parent.mkdir(parents=True)
            shutil.copyfile(ROOT/'PHP Core'/('.'+tool)/'hooks/working-memory-read.sh',hook)
            payload = json.dumps({'prompt':self.case['title'],'session_id':'aster-public-'+tool}).encode('utf-8')
            outputs.append(self.run_process([shutil.which('bash'),str(hook)],payload).stdout)
            self.queries += 1
        self.assertEqual(outputs[0],outputs[1])
        text = outputs[0].decode('utf-8')
        self.assertTrue(text.startswith('working: '+self.case['task']))
        for value in (P['progress'],self.case['fact'],P['notice'],self.chunk_rel):
            self.assertIn(value,text)
        self.assertNotIn('ASTER_PUBLIC_DECODE_CANARY',text)
        self.assertNotIn(P['bad_document'],text)
        self.assertEqual(before,self.authority_image())
        self.assertEqual(self.source_image,(self.source.read_bytes(),self.source.stat().st_mtime_ns))
        self.assertEqual(P['query_horizon_hooks'],self.queries)


if __name__ == '__main__':
    unittest.main()
