"""Fresh Wren final source-image controls; public temporary projects only."""
import copy
import hashlib
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
PUBLIC = json.loads((ROOT/'tests/fixtures/memory_final_source_image_public.json').read_text(encoding='utf-8'))
OBSERVATIONS = os.environ.get('WREN_OBSERVATION_DIR')


class FinalSourceImageTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='wren-image-')
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base/'project'
        self.root.mkdir()
        self.env = {k: os.environ[k] for k in ('PATH','HOME','LANG','LC_ALL','LC_CTYPE',
                    'SystemRoot','WINDIR','COMSPEC','PATHEXT') if k in os.environ}
        self.env.update(GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1',
                        XDG_CONFIG_HOME=str(self.base/'xdg'), TMPDIR=str(self.base))
        patch = mock.patch.dict(os.environ,self.env,clear=True)
        patch.start()
        self.addCleanup(patch.stop)
        subprocess.run(['git','-C',str(self.root),'init','-q','-b',PUBLIC['task']],env=self.env,
                       check=True,capture_output=True,timeout=8)
        self.runtime = ROOT/'PHP Core/memory-bank/scripts'
        names = [p.stem for p in self.runtime.glob('*.py')]
        previous = {n:sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:sys.modules.pop(n,None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0,str(self.runtime))
        try:
            self.c=importlib.import_module('context')
            self.r=importlib.import_module('context_retrieval')
        finally:sys.path.remove(str(self.runtime))
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config',
                    'project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        config=json.loads((self.root/'project-brain/config/runtime.json').read_text(encoding='utf-8'))
        config.update(automatic_promotion=False,automatic_compaction=False)
        self.write('project-brain/config/runtime.json',json.dumps(config))
        self.write(PUBLIC['source'],PUBLIC['source_text'])
        self.db=self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(self.db.close)
        self.r.ensure_metadata_tables(self.db)
        self.observations=[]
        self.rows={}
        self.row(PUBLIC['primary'],PUBLIC['primary_text'])
        self.row(PUBLIC['survivor'],PUBLIC['survivor_text'])
        for i in range(4):self.row('docs/wren-neutral-%s.md'%i,'# Unrelated\n\nNeutral archive inventory %s.\n'%i)
        self.prepack_scores={}
        self.receipts=[]

    def write(self,name,text):
        p=self.root/name
        p.parent.mkdir(parents=True,exist_ok=True)
        with p.open('w',encoding='utf-8',newline='\n') as stream:stream.write(text)
        return p

    def observe(self,label,value):
        self.observations.append({'label':label,'value':value})
        if OBSERVATIONS:
            dest=Path(OBSERVATIONS)
            dest.mkdir(parents=True,exist_ok=True)
            (dest/(self._testMethodName+'.json')).write_text(json.dumps(self.observations,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')

    def row(self,name,text,*,kind='spec',privacy='public',category='evidence',sources=None,fingerprints=None):
        self.write(name,text)
        self.db.execute('DELETE FROM documents WHERE path=?',(name,))
        self.db.execute('DELETE FROM document_metadata WHERE path=?',(name,))
        self.db.execute('DELETE FROM document_links WHERE path=?',(name,))
        title=next((line[2:] for line in text.splitlines() if line.startswith('# ')),'Wren record')
        sha=hashlib.sha256(text.encode('utf-8')).hexdigest()
        self.db.execute('INSERT INTO documents(path,layer,kind,title,summary,content) VALUES(?,?,?,?,?,?)',
            (name,'semantic',kind,title,'',text))
        self.db.execute('INSERT INTO document_metadata VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)',
            (name,category,privacy,'*','verified','active',sha,None,'[]',json.dumps(fingerprints or []),'',1.0,''))
        for source in sources or []:self.db.execute('INSERT INTO document_links VALUES(?,?,?)',(name,source,'source'))
        self.db.commit()
        self.rows[name]=sha

    def cli(self,*args):
        result=subprocess.run([sys.executable,'-B',str(self.root/'memory-bank/scripts/context.py'),
            '--root',str(self.root),*args],cwd=self.root,env=self.env,capture_output=True,timeout=25)
        self.observe(args[0],{'returncode':result.returncode,'stdout':result.stdout.decode('utf-8'),'stderr':result.stderr.decode('utf-8')})
        self.assertEqual(0,result.returncode,result.stderr)
        return json.loads(result.stdout)

    def task(self):
        self.cli('start','--task-id',PUBLIC['task'],'--goal',PUBLIC['goal'],'--source',PUBLIC['source'],'--json')
        self.cli('update','--task-id',PUBLIC['task'],'--revision','1','--progress',PUBLIC['progress'],'--json')
        return self.r.find_task(self.root,PUBLIC['task'])

    def result_oracle(self,result,expected,*,query=None,working=None):
        expected=set(expected)
        self.assertEqual(PUBLIC['query'] if query is None else query,result['query'])
        self.assertEqual(expected,{d['path'] for d in result['selected']})
        self.assertEqual(expected,{d['path'] for key in ('procedural','semantic','episodic') for d in result[key] if 'path' in d})
        self.assertEqual(expected,{d['path'] for items in result['categories'].values() for d in items})
        self.assertEqual(working is not None,result['working'] is not None)
        manifest=json.loads((self.root/result['manifest']).read_text(encoding='utf-8'))
        self.assertEqual(expected,{d['path'] for d in manifest['selected']})
        for d in manifest['selected']:
            self.assertEqual(self.rows[d['path']],d['source_hash'])
            if d['path'] in self.prepack_scores:self.assertEqual(round(self.prepack_scores[d['path']],6),d['score'])
        self.assertEqual(sum(d['estimated_tokens'] for d in result['selected']),result['token_estimates']['total'])
        self.assertEqual(result['token_estimates'],manifest['token_estimates'])
        self.assertEqual(result['query'],manifest['query'])
        self.assertEqual('off',result['gate']['mode'])
        self.assertEqual(result['gate'],manifest['gate'])
        if PUBLIC['primary'] not in expected:self.assertNotIn(PUBLIC['public_marker'],json.dumps(result))
        if working is None:
            self.assertNotIn(PUBLIC['progress'],json.dumps(result))
            self.assertNotIn(PUBLIC['new_progress'],json.dumps(result))
        self.assertEqual(1,len(self.receipts))
        identities=sorted((d['path'],self.rows[d['path']]) for d in result['selected'])
        digest=hashlib.sha256('\n'.join(n+'\0'+h for n,h in identities).encode('utf-8')).hexdigest()
        self.assertEqual(digest,self.receipts[0]['baseline']['signature']['paths'])
        self.observe('final-oracle',{'capsule':result,'manifest':manifest,'receipts':self.receipts})

    def run_case(self,case):
        primary=self.root/PUBLIC['primary']
        survivor=PUBLIC['survivor']
        expected=[PUBLIC['primary'],survivor]
        if case in ('missing_digested_source','missing_undigested_source','newly_changed_source',
                    'initially_changed_score_once','restored_citation_set'):
            fp=[self.r.fingerprint(self.root,PUBLIC['source'])] if case!='missing_undigested_source' else []
            self.row(PUBLIC['primary'],PUBLIC['primary_text'],sources=[PUBLIC['source']],fingerprints=fp)
            if case in ('initially_changed_score_once','restored_citation_set'):self.write(PUBLIC['source'],PUBLIC['source_changed'])
        if case.startswith('brain_'):
            text='---\n'+json.dumps({'privacy':'public','status':'active','sources':[]})+'\n---\n'+PUBLIC['primary_text']
            self.row(PUBLIC['primary'],text,kind='brain-task',category='dynamic')
        working=case.startswith('working_') or case=='native_render_unconfirmed_proof'
        task_file=record=body=None
        if working:task_file,record,body=self.task()
        task_id=PUBLIC['task'] if working else None
        derived=case in ('working_privacy_derived','working_deleted_derived')
        original_candidates=self.r._candidates
        captured=[]
        def candidates(*args,**kwargs):
            value=original_candidates(*args,**kwargs);captured.extend(value[0]);return value
        reads=[]
        original_read=Path.read_text
        original_receipts=self.r._commit_ordinary_receipts
        def receipts(*args,**kwargs):
            self.receipts.append(copy.deepcopy(kwargs));return original_receipts(*args,**kwargs)
        final_read_patch=None
        pack_calls=[]
        def pack(value):
            nonlocal final_read_patch
            pack_calls.append(1)
            self.observe('before-pack',copy.deepcopy(value))
            self.prepack_scores={d['path']:float(d['adjusted_score']) for d in captured}
            self.assertEqual(set(expected),{d['path'] for d in value['selected']})
            if case=='ordinary_same_size_mtime_edit':
                info=primary.stat();self.assertEqual(len(PUBLIC['primary_text']),len(PUBLIC['replacement']))
                self.write(PUBLIC['primary'],PUBLIC['replacement']);os.utime(primary,ns=(info.st_atime_ns,info.st_mtime_ns))
            elif case=='ordinary_deleted':primary.unlink()
            elif case=='ordinary_invalid_utf8':primary.write_bytes(b'\xffWrenInvalidUTF8')
            elif case.startswith('brain_'):
                text=primary.read_text(encoding='utf-8')
                text=text.replace('"privacy": "public"','"privacy": "restricted"') if case=='brain_privacy_edit' else text.replace('"status": "active"','"status": "completed"')
                self.write(PUBLIC['primary'],text)
            elif case.startswith('missing_'):(self.root/PUBLIC['source']).unlink()
            elif case=='newly_changed_source':self.write(PUBLIC['source'],PUBLIC['source_changed'])
            elif case=='restored_citation_set':self.write(PUBLIC['source'],PUBLIC['source_text'])
            elif case=='tightened_config':
                config=json.loads((self.root/'project-brain/config/runtime.json').read_text(encoding='utf-8'))
                config['allowed_privacy']=['team'];self.write('project-brain/config/runtime.json',json.dumps(config))
            elif case=='packed_out_no_final_read':
                for key in ('selected','semantic'):value[key]=[d for d in value[key] if d['path']!=PUBLIC['primary']]
                value['categories']={key:[d for d in items if d['path']!=PUBLIC['primary']] for key,items in value['categories'].items()}
                def record_read(p,*args,**kwargs):
                    if p==primary:reads.append(str(p))
                    return original_read(p,*args,**kwargs)
                final_read_patch=mock.patch.object(Path,'read_text',record_read);final_read_patch.start()
            elif case.startswith('working_'):
                if case=='working_deleted_derived':task_file.unlink()
                elif case=='working_invalid_utf8':task_file.write_bytes(b'\xffWrenBadTask')
                else:
                    edited=dict(record)
                    if case=='working_progress_explicit':edited['progress']=PUBLIC['new_progress']
                    elif case=='working_privacy_derived':edited['privacy']='restricted'
                    elif case=='working_status_explicit':edited['status']='completed'
                    self.write(task_file.relative_to(self.root).as_posix(),'---\n'+json.dumps(edited)+'\n---\n'+body)
            elif case=='cancellation_propagates':raise KeyboardInterrupt('Wren supplied cancellation')
            return value
        with mock.patch.object(self.r,'_candidates',side_effect=candidates),mock.patch.object(self.r,'_commit_ordinary_receipts',side_effect=receipts):
            try:
                if case=='cancellation_propagates':
                    with self.assertRaises(KeyboardInterrupt):self.r.retrieve(self.db,self.root,PUBLIC['query'],task_id,limit=5,gate_mode='off',manifest_scope='local',pack=pack)
                    self.assertEqual([],self.receipts);return
                if case=='native_render_unconfirmed_proof':
                    proof={};native_pack=self.c.enforce_governed_capsule_contract
                    def native(value):
                        value=native_pack(value);self.write(PUBLIC['primary'],PUBLIC['replacement']);return value
                    with mock.patch.object(self.c,'enforce_governed_capsule_contract',side_effect=native),mock.patch.object(self.r,'atomic_json',side_effect=OSError('Wren supplied local publication uncertainty')):
                        result=self.c.assemble_capsule(self.db,self.root,mode='governed',query=PUBLIC['query'],task_id=PUBLIC['task'],limit=5,ephemeral=True,refresh_index=False,gate_mode='off',render=True,source_provenance_sink=proof)
                    text='\n'.join(self.c.render_capsule_lines(result,{}))
                    self.observe('native-proof',{'result':result,'text':text,'proof':proof})
                    self.assertNotIn(PUBLIC['public_marker'],text)
                    self.assertNotIn(PUBLIC['public_marker'],json.dumps(result))
                    self.assertEqual({survivor},{d['path'] for d in result['selected']})
                    self.assertEqual('unconfirmed',proof['publication'])
                    expected_sha=hashlib.sha256(json.dumps(result,ensure_ascii=False,separators=(',',':'),sort_keys=True,allow_nan=False).encode('utf-8')).hexdigest()
                    self.assertEqual(expected_sha,proof['capsule_sha256'])
                    self.assertEqual([{'path':survivor,'source_hash':self.rows[survivor]}],proof['selected'])
                    self.assertEqual([],self.receipts);return
                result=self.r.retrieve(self.db,self.root,PUBLIC['query'],task_id,limit=5,gate_mode='off',manifest_scope='local',query_source='task' if derived else 'explicit',pack=pack)
            finally:
                if final_read_patch:final_read_patch.stop()
        self.assertEqual(1,len(pack_calls))
        if case not in ('healthy_unchanged','initially_changed_score_once') and not working and case!='tightened_config':expected=[survivor]
        if case=='tightened_config' or derived:expected=[]
        self.result_oracle(result,expected,query=PUBLIC['task'] if derived else None)
        if case=='packed_out_no_final_read':self.assertEqual([],reads)
        if case=='initially_changed_score_once':
            row=next(d for d in json.loads((self.root/result['manifest']).read_text(encoding='utf-8'))['selected'] if d['path']==PUBLIC['primary'])
            self.assertTrue(row['source_changed'])


for _case in PUBLIC['matrix']:
    def _control(self,case=_case):self.run_case(case)
    _control.__name__='test_'+_case
    setattr(FinalSourceImageTests,_control.__name__,_control)
if __name__=='__main__':unittest.main()
