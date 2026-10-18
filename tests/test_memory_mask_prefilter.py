"""Fresh Sanderling exact masking and complete cold-hook controls."""
from __future__ import annotations

import hashlib
from contextlib import closing
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
PUBLIC=json.loads((ROOT/'tests/fixtures/memory_mask_prefilter_public.json').read_text(encoding='utf-8'))


class MaskPrefilterTests(unittest.TestCase):
    def setUp(self):
        self.directory=ROOT/'PHP Core/memory-bank/scripts'
        self.previous={k:sys.modules.get(k) for k in ('automatic_query','workspace_roots')}
        for k in self.previous: sys.modules.pop(k,None)
        sys.path.insert(0,str(self.directory));self.addCleanup(sys.path.remove,str(self.directory))
        self.automatic=importlib.import_module('automatic_query')
        self.patterns=dict(self.automatic.SECRET_PATTERNS)
        self.addCleanup(self.restore)
        self.load()

    def restore(self):
        self.automatic.SECRET_PATTERNS.clear();self.automatic.SECRET_PATTERNS.update(self.patterns)
        for k,value in self.previous.items():
            if value is None: sys.modules.pop(k,None)
            else: sys.modules[k]=value

    def load(self):
        path=Path(os.environ.get('SANDERLING_ORIGINAL_VALIDATE',str(self.directory/'validate.py')))
        spec=importlib.util.spec_from_file_location('sanderling_validate',path)
        self.module=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.module)

    def calls(self,text):
        calls=[]
        def profile(frame,event,arg):
            if event=='c_call' and getattr(arg,'__name__','') in ('finditer','search') and isinstance(getattr(arg,'__self__',None),re.Pattern):
                calls.append((arg.__name__,arg.__self__))
        prior=sys.getprofile();sys.setprofile(profile)
        try: result=self.module.mask_secrets(text)
        finally: sys.setprofile(prior)
        return result,calls

    def test_complete_independent_literal_oracles(self):
        for row in PUBLIC['samples']:
            with self.subTest(name=row['name']):
                # A sample GitHub push protection would take for a live secret is stored in parts.
                text=row['text'] if 'text' in row else ''.join(row['text_parts'])
                self.assertEqual((row['masked'],row['count']),self.module.mask_secrets(text))

    def test_absent_cues_skip_all_actual_pattern_scans(self):
        text='The api route carries linen wheels through the windlass archive.\n'*16
        result,calls=self.calls(text)
        self.assertEqual((text,0),result);self.assertEqual([],calls)

    def test_one_present_provider_cue_runs_only_its_original_pattern(self):
        row=next(r for r in PUBLIC['samples'] if r['name']=='ghp_')
        result,calls=self.calls(row['text'])
        self.assertEqual((row['masked'],1),result)
        self.assertEqual([('finditer',self.patterns['GitHub token'])],calls)

    def test_secret_policy_fingerprint_unchanged(self):
        self.assertEqual(PUBLIC['expected']['mask_policy_version'],self.module.MASK_POLICY_VERSION)
        self.assertEqual(PUBLIC['expected']['secret_policy_sha256'],self.module.secret_policy_fingerprint())

    def test_replaced_known_pattern_after_import_is_not_skipped(self):
        with mock.patch.dict(self.module.SECRET_PATTERNS,{'GitHub token':re.compile('SANDERLING_SENTINEL')}):
            self.assertEqual(('[redacted: GitHub token]',1),self.module.mask_secrets('SANDERLING_SENTINEL'))

    def test_unknown_pattern_added_before_import_is_not_pinned(self):
        self.automatic.SECRET_PATTERNS['future provider']=re.compile('SANDERLING_SENTINEL')
        self.load()
        self.assertEqual(('[redacted: future provider]',1),self.module.mask_secrets('SANDERLING_SENTINEL'))

    def test_known_pattern_changed_before_import_is_not_pinned(self):
        self.automatic.SECRET_PATTERNS['GitHub token']=re.compile('SANDERLING_SENTINEL',re.ASCII)
        self.load()
        self.assertEqual(('[redacted: GitHub token]',1),self.module.mask_secrets('SANDERLING_SENTINEL'))

    def test_same_source_with_changed_flags_runs_full_scan(self):
        original=self.patterns['GitHub token']
        with mock.patch.dict(self.module.SECRET_PATTERNS,{'GitHub token':re.compile(original.pattern,original.flags|re.IGNORECASE)}):
            self.assertEqual(('[redacted: GitHub token]',1),self.module.mask_secrets('GHP_'+'A'*25))

    def test_unknown_self_matching_marker_retains_survivor_failure(self):
        with mock.patch.dict(self.module.SECRET_PATTERNS,{'surviving':re.compile(r'SANDERLING_SENTINEL|\[redacted: surviving\]')}):
            with self.assertRaisesRegex(self.module.ValidationError,'^possible secret survives masking; value intentionally not printed$'):
                self.module.mask_secrets('SANDERLING_SENTINEL')

    def test_new_cue_created_by_first_pass_is_screened_again(self):
        with mock.patch.dict(self.module.SECRET_PATTERNS,{'password=SanderlingPublic7':re.compile('SANDERLING_SENTINEL')}):
            self.assertEqual(('[redacted: password=[redacted: assigned credential]',2),self.module.mask_secrets('SANDERLING_SENTINEL'))

    def test_nonpattern_replacement_keeps_existing_custom_scanner(self):
        class Custom:
            def finditer(self,text): return re.finditer('SANDERLING_SENTINEL',text)
            def search(self,text): return re.search('SANDERLING_SENTINEL',text)
        with mock.patch.dict(self.module.SECRET_PATTERNS,{'future provider':Custom()}):
            self.assertEqual(('[redacted: future provider]',1),self.module.mask_secrets('SANDERLING_SENTINEL'))

    def test_known_custom_scanner_before_import_does_not_reflect_class(self):
        touches=[]
        class Custom:
            @property
            def __class__(self):
                touches.append('class');raise RuntimeError('Sanderling reflected class must remain untouched')
            def finditer(self,text): return re.finditer('SANDERLING_REFLECTED_PUBLIC',text)
            def search(self,text): return re.search('SANDERLING_REFLECTED_PUBLIC',text)
        self.automatic.SECRET_PATTERNS['GitHub token']=Custom()
        self.load()
        self.assertEqual(('[redacted: GitHub token]',1),self.module.mask_secrets('SANDERLING_REFLECTED_PUBLIC'))
        self.assertEqual([],touches)

    def test_compiled_source_subclass_cannot_spoof_known_pattern_pin(self):
        touches=[];original=self.patterns['GitHub token']
        class Source(str):
            def encode(self,*args,**kwargs):
                touches.append('encode');return original.pattern.encode('utf-8')
        self.automatic.SECRET_PATTERNS['GitHub token']=re.compile(Source('SANDERLING_SOURCE_SUBCLASS_PUBLIC'),original.flags)
        self.load()
        self.assertEqual(('[redacted: GitHub token]',1),self.module.mask_secrets('SANDERLING_SOURCE_SUBCLASS_PUBLIC'))
        self.assertEqual([],touches)


@unittest.skipUnless(os.name != 'nt' and shutil.which('bash') and shutil.which('timeout'),'actual POSIX hook profile required; native Windows remains unrun')
class ColdMaskHookTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='sanderling-cold-');self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name)
        for relative in ('memory-bank/scripts','memory-bank/templates','project-brain/config','project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/relative,self.root/relative,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        old=os.environ.get('SANDERLING_ORIGINAL_VALIDATE')
        if old: shutil.copyfile(old,self.root/'memory-bank/scripts/validate.py')
        for host in ('.claude','.codex'):
            destination=self.root/host/'hooks';destination.mkdir(parents=True)
            shutil.copyfile(ROOT/'PHP Core'/host/'hooks/working-memory-read.sh',destination/'working-memory-read.sh')
        (self.root/'memory-bank/chunks').mkdir()
        (self.root/'.gitignore').write_text('memory-bank/local/\nproject-brain/local/\n',encoding='utf-8')
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps({'mode':'governed','automatic_promotion':False,'retrieval_gate':'off'}),encoding='utf-8')
        self.expected=[];self.stats={};self.source_bytes=0
        for row in PUBLIC['corpus']:
            if 'literal_text' in row: raw=row['literal_text'];kind='overview'
            else:
                remaining=row['bytes']-len(row['header'].encode('utf-8'));filler=row['filler']
                raw=row['header']+filler*(remaining//len(filler))+filler[:remaining%len(filler)];kind='spec'
            self.assertEqual(row['raw_sha256'],hashlib.sha256(raw.encode('utf-8')).hexdigest())
            masked=raw.replace("'SanderlingPublic7 multi word'",'[redacted: assigned credential]') if row['redactions'] else raw
            self.assertEqual(row['masked_sha256'],hashlib.sha256(masked.encode('utf-8')).hexdigest())
            leaf=self.root/row['path'];leaf.parent.mkdir(parents=True,exist_ok=True);leaf.write_text(raw,encoding='utf-8')
            status=leaf.stat();self.stats[row['path']]=(status.st_mtime_ns,status.st_size,None)
            self.source_bytes+=len(raw.encode('utf-8'))
            self.expected.append((row['path'],'semantic',kind,row['title'],row['summary'],masked))
        self.env={k:v for k,v in os.environ.items() if not k.startswith(('GIT_','CONTEXT_','ACCELERATOR_'))}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',PYTHONIOENCODING='utf-8',HOME=str(self.root/'private-home'),TMPDIR=str(self.root/'private-tmp'))
        Path(self.env['HOME']).mkdir();Path(self.env['TMPDIR']).mkdir()
        # Hooks resolve python3 from PATH. Pin their real interpreter to this
        # controller so the separate compatibility run tests that runtime.
        launcher=self.root/'private-bin';launcher.mkdir()
        (launcher/'python3').symlink_to(Path(sys.executable).resolve())
        self.env['PATH']=str(launcher)+os.pathsep+self.env.get('PATH','')
        subprocess.run(['git','-C',str(self.root),'init','-q','-b',PUBLIC['task_id']],env=self.env,check=True,capture_output=True)
        self.measurements=[];self.addCleanup(self.receipt)

    def receipt(self):
        directory=os.environ.get('SANDERLING_RESULT_DIR')
        if directory:
            p=Path(directory);p.mkdir(parents=True,exist_ok=True)
            name=('old' if os.environ.get('SANDERLING_ORIGINAL_VALIDATE') else 'current')+'-cold.json'
            (p/name).write_text(json.dumps({'calls':self.measurements,'source_bytes':self.source_bytes,'provider_tokens':None,'full_lifecycle_cost':False},indent=2)+'\n',encoding='utf-8')

    def hook(self,host):
        try:
            import resource
            before=resource.getrusage(resource.RUSAGE_CHILDREN)
        except ImportError: resource=None;before=None
        start=time.monotonic()
        result=subprocess.run(['bash',str(self.root/host/'hooks/working-memory-read.sh')],input=json.dumps({'prompt':PUBLIC['query']}).encode('utf-8'),cwd=self.root,env=self.env,capture_output=True,timeout=7)
        elapsed=time.monotonic()-start
        cpu=None
        if resource is not None:
            after=resource.getrusage(resource.RUSAGE_CHILDREN);cpu=(after.ru_utime+after.ru_stime)-(before.ru_utime+before.ru_stime)
        self.measurements.append({'host':host,'seconds':elapsed,'child_cpu_seconds':cpu,'returncode':result.returncode,'stdout_bytes':len(result.stdout),'stderr_bytes':len(result.stderr),'database_bytes':(self.root/'memory-bank/local/context.db').stat().st_size,'hook_budget_seconds':5,'helper_python':str(Path(sys.executable).resolve())})
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'))
        return result.stdout.decode('utf-8'),elapsed

    def test_complete_cold_hook_before_existing_ceiling_and_warm_same_output(self):
        text,elapsed=self.hook('.claude')
        self.assertNotIn('Memory refresh unavailable',text)
        self.assertIn(PUBLIC['fact'],text)
        self.assertLessEqual(elapsed,PUBLIC['expected']['cold_hook_seconds_max'])
        with closing(sqlite3.connect(self.root/'memory-bank/local/context.db')) as db:
            documents=db.execute('SELECT path,layer,kind,title,summary,content FROM documents ORDER BY path').fetchall()
            self.assertEqual(sorted(self.expected),documents)
            metadata=db.execute('SELECT * FROM document_metadata ORDER BY path').fetchall()
            self.assertEqual(sorted(tuple(r['metadata']) for r in PUBLIC['corpus']),metadata)
            self.assertEqual(PUBLIC['expected']['links'],db.execute('SELECT * FROM document_links ORDER BY path,ref_path,ref_kind').fetchall())
            self.assertEqual(sorted((path,*state) for path,state in self.stats.items()),db.execute('SELECT * FROM document_source_state ORDER BY path').fetchall())
            state=dict(db.execute('SELECT key,value FROM index_state'))
            self.assertRegex(state['index-generation'],r'^[0-9a-f-]{36}$')
            self.assertEqual(PUBLIC['expected']['secret_policy_sha256'],state['secret-policy-fingerprint'])
            self.assertFalse(any('SanderlingPublic7' in row[5] for row in documents))
            self.assertEqual(1,sum(row[5].count('[redacted: assigned credential]') for row in documents))
        warm,_=self.hook('.codex')
        # The existing refresh renderer emits measured phase diagnostics on
        # slow calls. Compare the complete capsule, removing only that line.
        def capsule(value):
            return '\n'.join(line for line in value.splitlines() if not re.fullmatch(r'phases: stat \d+ms index \d+ms retrieval \d+ms',line))
        self.assertEqual(capsule(text),capsule(warm))
        with closing(sqlite3.connect(self.root/'memory-bank/local/context.db')) as db:
            self.assertEqual(state['index-generation'],dict(db.execute('SELECT key,value FROM index_state'))['index-generation'])
        self.assertNotIn('SanderlingPublic7',warm)


if __name__=='__main__': unittest.main()
