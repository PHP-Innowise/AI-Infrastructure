"""Fresh Heron prefix-boundary semantics and actual lightweight cold memory."""
from __future__ import annotations
from contextlib import closing
import hashlib
import importlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

ROOT=Path(__file__).resolve().parents[1]
PUBLIC=json.loads((ROOT/'tests/fixtures/memory_credential_prefix_public.json').read_text(encoding='utf-8'))


class CredentialPrefixTests(unittest.TestCase):
    def setUp(self):
        directory=ROOT/'PHP Core/memory-bank/scripts'
        old={name:sys.modules.get(name) for name in ('automatic_query','workspace_roots')}
        for name in old: sys.modules.pop(name,None)
        sys.path.insert(0,str(directory))
        def restore():
            sys.path.remove(str(directory))
            for name,value in old.items():
                if value is None: sys.modules.pop(name,None)
                else: sys.modules[name]=value
        self.addCleanup(restore)
        path=Path(os.environ.get('HERON_ORIGINAL_VALIDATE',str(directory/'validate.py')))
        spec=importlib.util.spec_from_file_location('heron_validate',path)
        self.module=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.module)
        self.pattern=self.module.SECRET_PATTERNS['assigned credential']

    def view(self,matches): return [{'span':list(found.span()),'text':found.group(0)} for found in matches]

    def test_complete_literal_consumed_span_oracles(self):
        for row in PUBLIC['cases']:
            with self.subTest(name=row['name']):
                # The legacy regex is a reference on NEW short data, not an
                # oracle generator or a replay of any closed fixture.
                self.assertEqual(row['matches'],self.view(self.pattern.finditer(row['text'])))
                self.assertEqual(row['matches'],self.view(self.module._mask_pattern_matches('assigned credential',self.pattern,row['text'])))

    def test_complete_literal_mask_output_oracles(self):
        for row in PUBLIC['cases']:
            with self.subTest(name=row['name']):
                self.assertEqual((row['masked'],row['count']),self.module.mask_secrets(row['text']))

    def observed(self,call):
        calls=[];prior=sys.getprofile()
        def profile(frame,event,arg):
            if event=='c_call' and getattr(arg,'__self__',None) is self.pattern:
                calls.append(getattr(arg,'__name__',''))
        sys.setprofile(profile)
        try: result=call()
        finally: sys.setprofile(prior)
        return result,calls

    def test_nonliteral_prefix_has_one_anchor_not_repeated_search(self):
        row=PUBLIC['anchor_probe'];text=row['text']
        result,calls=self.observed(lambda:self.module.mask_secrets(text))
        self.assertEqual((text,row['masked_count']),result)
        self.assertEqual(row['original_finditer_calls'],calls.count('finditer'))
        self.assertEqual(row['original_match_calls'],calls.count('match'))

    def test_final_known_pattern_uses_same_anchored_enumeration(self):
        text=PUBLIC['anchor_probe']['text']
        result,calls=self.observed(lambda:self.module._mask_pattern_search('assigned credential',self.pattern,text))
        self.assertIsNone(result);self.assertEqual(['match'],calls)

    def test_replaced_policy_keeps_its_original_matches(self):
        row=PUBLIC['supplement']['modified_pattern'];pattern=re.compile(row['pattern'])
        with mock.patch.dict(self.module.SECRET_PATTERNS,{row['label']:pattern}):
            self.assertEqual((row['masked'],row['count']),self.module.mask_secrets(row['text']))
            self.assertIs(self.module._mask_pattern_search(row['label'],pattern,row['text']).re,pattern)

    def test_text_subclass_keeps_original_iterator_without_prefix_access(self):
        row=PUBLIC['supplement']['text_subclass'];touches=[]
        class Text(str):
            def __getitem__(self,key):
                if isinstance(key,int): touches.append(key)
                return super().__getitem__(key)
        matches=list(self.module._mask_pattern_matches('assigned credential',self.pattern,Text(row['text'])))
        self.assertEqual(row['spans'],[list(found.span()) for found in matches])
        self.assertEqual([row['match_pos']],[found.pos for found in matches])
        self.assertEqual(row['character_getitem_touches'],len(touches))

    def test_final_cued_cycle_retains_fixed_survivor_failure(self):
        row=PUBLIC['supplement']['final_cued_cycle']
        with mock.patch.dict(self.module.SECRET_PATTERNS,{row['label']:re.compile(row['pattern'])}):
            with self.assertRaises(self.module.ValidationError) as caught:self.module.mask_secrets(row['text'])
        self.assertEqual(row['error'],str(caught.exception))


@unittest.skipUnless(os.name!='nt' and shutil.which('bash') and shutil.which('timeout'),'actual POSIX profile; native Windows unrun')
class CredentialPrefixHookTests(unittest.TestCase):
    def setUp(self):
        self.setup_started=time.monotonic();self.measurements=[]
        temporary=tempfile.TemporaryDirectory(prefix='heron-prefix-');self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name)
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config','project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        original=os.environ.get('HERON_ORIGINAL_VALIDATE')
        if original:shutil.copyfile(original,self.root/'memory-bank/scripts/validate.py')
        for host in ('.claude','.codex'):
            p=self.root/host/'hooks';p.mkdir(parents=True)
            shutil.copyfile(ROOT/'PHP Core'/host/'hooks/working-memory-read.sh',p/'working-memory-read.sh')
        (self.root/'memory-bank/chunks').mkdir()
        (self.root/'.gitignore').write_text('memory-bank/local/\nproject-brain/local/\n',encoding='utf-8')
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps({'mode':'lightweight','automatic_promotion':False,'retrieval_gate':'off'}),encoding='utf-8')
        self.env={k:v for k,v in os.environ.items() if not k.startswith(('GIT_','CONTEXT_','ACCELERATOR_'))}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',PYTHONIOENCODING='utf-8',HOME=str(self.root/'private-home'),TMPDIR=str(self.root/'private-tmp'))
        Path(self.env['HOME']).mkdir();Path(self.env['TMPDIR']).mkdir()
        launcher=self.root/'private-bin';launcher.mkdir();(launcher/'python3').symlink_to(Path(sys.executable).resolve())
        self.env['PATH']=str(launcher)+os.pathsep+self.env.get('PATH','')
        subprocess.run(['git','-C',str(self.root),'init','-q','-b',PUBLIC['task_id']],env=self.env,check=True,capture_output=True)
        self.cli=self.root/'memory-bank/scripts/context.py'
        # The stress source is added after real saves, so neither setup call
        # accidentally warms the cold index under a larger test timeout.
        for row in PUBLIC['native_docs']:
            if row['path']==PUBLIC['source']:
                p=self.root/row['path'];p.parent.mkdir(parents=True);p.write_text(row['raw'],encoding='utf-8')
        started=self.call('start','--task-id',PUBLIC['task_id'],'--goal',PUBLIC['goal'],'--source',PUBLIC['source'],'--json')
        self.assertEqual(PUBLIC['task_id'],started['task_id']);self.assertEqual(PUBLIC['goal'],started['goal'])
        updated=self.call('update','--task-id',PUBLIC['task_id'],'--progress',PUBLIC['progress'],'--next-step',PUBLIC['next_step'],'--json')
        self.assertEqual(PUBLIC['progress'],updated['progress']);self.assertEqual([PUBLIC['next_step']],updated['next_steps'])
        self.database=self.root/'memory-bank/local/context.db'
        with closing(sqlite3.connect(self.database)) as db:
            self.working=db.execute('SELECT * FROM working_tasks ORDER BY task_id').fetchall()
            self.assertEqual(1,len(self.working))
            self.assertEqual([PUBLIC['source']],json.loads(db.execute('SELECT sources FROM working_tasks').fetchone()[0]))
            self.assertEqual(0,db.execute('SELECT COUNT(*) FROM documents').fetchone()[0])
        self.stats={};self.source_bytes=0
        for row in PUBLIC['native_docs']:
            p=self.root/row['path'];p.parent.mkdir(parents=True,exist_ok=True);p.write_text(row['raw'],encoding='utf-8')
            self.assertEqual(row['raw_sha256'],hashlib.sha256(p.read_bytes()).hexdigest())
            status=p.stat();self.stats[row['path']]=(status.st_mtime_ns,status.st_size,None);self.source_bytes+=status.st_size
        self.setup_seconds=time.monotonic()-self.setup_started;self.addCleanup(self.receipt)

    def call(self,*args):
        result=subprocess.run([sys.executable,'-B',str(self.cli),*args],cwd=self.root,env=self.env,capture_output=True,timeout=3)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'));return json.loads(result.stdout)

    def receipt(self):
        directory=os.environ.get('HERON_RESULT_DIR')
        if directory:
            p=Path(directory);p.mkdir(parents=True,exist_ok=True);name=('old' if os.environ.get('HERON_ORIGINAL_VALIDATE') else 'current')+'-cold.json'
            with closing(sqlite3.connect(self.database)) as db:
                observed={table:db.execute('SELECT COUNT(*) FROM '+table).fetchone()[0] for table in ('documents','document_metadata','document_source_state','document_links')}
                same=self.working==db.execute('SELECT * FROM working_tasks ORDER BY task_id').fetchall()
            (p/name).write_text(json.dumps({'calls':self.measurements,'source_bytes':self.source_bytes,'setup_seconds':self.setup_seconds,'observed_native_rows':observed,'saved_working_unchanged':same,'mode':'lightweight','provider_tokens':None,'full_lifecycle_cost':False},indent=2)+'\n',encoding='utf-8')

    def hook(self,host):
        import resource
        before=resource.getrusage(resource.RUSAGE_CHILDREN);start=time.monotonic()
        result=subprocess.run(['bash',str(self.root/host/'hooks/working-memory-read.sh')],input=json.dumps({'prompt':PUBLIC['query']}).encode('utf-8'),cwd=self.root,env=self.env,capture_output=True,timeout=7)
        elapsed=time.monotonic()-start;after=resource.getrusage(resource.RUSAGE_CHILDREN)
        self.measurements.append({'host':host,'seconds':elapsed,'child_cpu_seconds':after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime,'returncode':result.returncode,'stdout_bytes':len(result.stdout),'stderr_bytes':len(result.stderr),'stdout':result.stdout.decode('utf-8'),'database_bytes':self.database.stat().st_size,'hook_budget_seconds':5,'helper_python':str(Path(sys.executable).resolve())})
        self.assertEqual(0,result.returncode);return result.stdout.decode('utf-8'),elapsed

    def test_real_saved_working_and_full_cold_index_survive_prefix_lookup(self):
        text,elapsed=self.hook('.claude')
        self.assertNotIn('Memory refresh unavailable',text)
        self.assertIn(PUBLIC['progress'],text);self.assertIn(PUBLIC['fact'],text)
        self.assertLessEqual(elapsed,PUBLIC['expected']['cold_seconds_max'])
        with closing(sqlite3.connect(self.database)) as db:
            self.assertEqual(sorted(tuple(row['document']) for row in PUBLIC['native_docs']),db.execute('SELECT path,layer,kind,title,summary,content FROM documents ORDER BY path').fetchall())
            self.assertEqual(sorted(tuple(row['metadata']) for row in PUBLIC['native_docs']),db.execute('SELECT * FROM document_metadata ORDER BY path').fetchall())
            self.assertEqual(PUBLIC['expected']['links'],db.execute('SELECT * FROM document_links ORDER BY path,ref_path,ref_kind').fetchall())
            self.assertEqual(sorted((path,*value) for path,value in self.stats.items()),db.execute('SELECT * FROM document_source_state ORDER BY path').fetchall())
            self.assertEqual(self.working,db.execute('SELECT * FROM working_tasks ORDER BY task_id').fetchall())
            generation=dict(db.execute('SELECT key,value FROM index_state'))['index-generation']
            self.assertRegex(generation,r'^[0-9a-f-]{36}$')
        saved=self.call('get','--task-id',PUBLIC['task_id'],'--json')
        self.assertEqual(PUBLIC['progress'],saved['progress']);self.assertEqual([PUBLIC['next_step']],saved['next_steps'])
        warm,_=self.hook('.codex')
        strip=lambda value:'\n'.join(line for line in value.splitlines() if not re.fullmatch(r'phases: stat \d+ms index \d+ms retrieval \d+ms',line))
        self.assertEqual(strip(text),strip(warm))
        with closing(sqlite3.connect(self.database)) as db:
            self.assertEqual(generation,dict(db.execute('SELECT key,value FROM index_state'))['index-generation'])
            self.assertEqual(self.working,db.execute('SELECT * FROM working_tasks ORDER BY task_id').fetchall())


if __name__=='__main__':unittest.main()
