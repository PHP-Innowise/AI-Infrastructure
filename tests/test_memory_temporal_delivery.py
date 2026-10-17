"""New Finch public native-index temporal admission controls."""
import copy
from datetime import date as RealDate
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
PUBLIC = json.loads((ROOT/'tests/fixtures/memory_temporal_delivery_public.json').read_text(encoding='utf-8'))

class Clock(RealDate):
    value = RealDate(2031,4,12)
    @classmethod
    def today(cls):return cls.value

class TemporalDeliveryTests(unittest.TestCase):
    def setUp(self):
        tmp=tempfile.TemporaryDirectory(prefix='finch-temporal-');self.addCleanup(tmp.cleanup)
        self.base=Path(tmp.name);self.root=self.base/'project';self.root.mkdir()
        env={k:os.environ[k] for k in ('PATH','HOME','LANG','LC_ALL','LC_CTYPE','SystemRoot','WINDIR','COMSPEC','PATHEXT') if k in os.environ}
        env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',XDG_CONFIG_HOME=str(self.base/'xdg'),TMPDIR=str(self.base))
        p=mock.patch.dict(os.environ,env,clear=True);p.start();self.addCleanup(p.stop)
        subprocess.run(['git','-C',str(self.root),'init','-q','-b','finch-temporal'],env=env,check=True,capture_output=True,timeout=8)
        runtime=ROOT/'PHP Core/memory-bank/scripts';names=[f.stem for f in runtime.glob('*.py')]
        old={n:sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:sys.modules.pop(n,None)
            sys.modules.update(old)
        self.addCleanup(restore);sys.path.insert(0,str(runtime))
        try:self.c=importlib.import_module('context');self.r=importlib.import_module('context_retrieval');self.v=importlib.import_module('validate')
        finally:sys.path.remove(str(runtime))
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config','project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        cfg=json.loads((self.root/'project-brain/config/runtime.json').read_text(encoding='utf-8'));cfg.update(automatic_promotion=False,automatic_compaction=False)
        self.write('project-brain/config/runtime.json',json.dumps(cfg))
        self.write(PUBLIC['source'],'<?php // Finch public source\n')
        self.target='memory-bank/chunks/MEM-0842-finch-umber-latch.md'
        self.survivor='memory-bank/chunks/MEM-20310412-91abcdef-finch-future.md'
        self.doc='specs/finch-dated.md'
        self.write(self.doc,'# Finch umber latch\n\nOrdinary dated prose: review_after 1999-01-01.\n')
        for i in range(8):self.write('specs/neutral-%d.md'%i,'# Neutral\n\nUnrelated public inventory %d.\n'%i)
        self.db=self.c.connect(self.root/'memory-bank/local/context.db');self.addCleanup(self.db.close)
        Clock.value=RealDate.fromisoformat(PUBLIC['boundary'])
        for module in (self.c,self.v):
            p=mock.patch.object(module,'date',Clock);p.start();self.addCleanup(p.stop)
        self.receipts=[];self.beforepack={};self.beforeorder=[]

    def write(self,name,text):
        f=self.root/name;f.parent.mkdir(parents=True,exist_ok=True)
        with f.open('w',encoding='utf-8',newline='\n') as out:out.write(text)
        return f

    def chunk(self,name,review,valid_to=None):
        mid='MEM-0842' if name==self.target else 'MEM-20310412-91abcdef'
        metadata={'id':mid,'title':'Finch umber latch','type':'constraint','status':'active','scope':['public'],'tags':['finch'],
                  'created':'2031-04-01','last_verified':'2031-04-01','review_after':review,'sources':[PUBLIC['source']],
                  'supersedes':[],'superseded_by':None,'valid_to':valid_to}
        self.write(name,'---\n'+json.dumps(metadata)+'\n---\n# Finch umber latch\n\n'+('FINCH_EXPIRES' if name==self.target else 'FINCH_SURVIVES')+' public bounded rule.\n')

    def index(self,kind='review',nullable=False):
        self.chunk(self.target,PUBLIC['boundary'] if kind=='review' else PUBLIC['future'],PUBLIC['boundary'] if kind=='valid' else None)
        self.chunk(self.survivor,PUBLIC['future'])
        value=self.c.index_repository(self.db,self.root)
        self.assertEqual({self.target,self.survivor}, {r['path'] for r in self.db.execute("SELECT path FROM documents WHERE kind='memory'")})
        self.images={name:((self.root/name).read_bytes(),(self.root/name).stat().st_mtime_ns) for name in (self.target,self.survivor,self.doc)}
        self.assertEqual([],self.db.execute('PRAGMA foreign_key_check').fetchall())
        return value

    def raw(self):return self.c.search_documents(self.db,PUBLIC['query'],20,repository=self.root)

    def retrieve(self,pack=None):
        candidates=self.r._candidates
        self.ranked=[]
        def capture_candidates(*args,**kwargs):
            value=candidates(*args,**kwargs);self.ranked.extend(value[0]);return value
        original=self.r._commit_ordinary_receipts
        def capture(*args,**kwargs):self.receipts.append(copy.deepcopy(kwargs));return original(*args,**kwargs)
        with mock.patch.object(self.r,'_commit_ordinary_receipts',side_effect=capture),mock.patch.object(self.r,'_candidates',side_effect=capture_candidates):
            return self.r.retrieve(self.db,self.root,PUBLIC['query'],None,limit=10,gate_mode='off',manifest_scope='local',pack=pack)

    def oracle(self,result,expired):
        expected={self.survivor,self.doc} if expired else {self.target,self.survivor,self.doc}
        self.assertEqual(expected,{d['path'] for d in result['selected']})
        self.assertEqual(expected,{d['path'] for key in ('procedural','semantic','episodic') for d in result[key] if 'path' in d})
        for view in [result[k] for k in ('procedural','semantic','episodic')]+list(result['categories'].values()):
            self.assertTrue({d['path'] for d in view if 'path' in d}<=expected)
        self.assertEqual(expected,{d['path'] for items in result['categories'].values() for d in items})
        manifest=json.loads((self.root/result['manifest']).read_text(encoding='utf-8'))
        self.assertEqual(expected,{d['path'] for d in manifest['selected']})
        if expired:self.assertNotIn('FINCH_EXPIRES',json.dumps(result))
        self.assertEqual(sum(d['estimated_tokens'] for d in result['selected']),result['token_estimates']['total'])
        self.assertEqual(result['gate'],manifest['gate'])
        self.assertEqual(result['token_estimates'],manifest['token_estimates'])
        if expired:self.assertIn({'path':self.target,'reason':self.expected_reason},manifest['excluded'])
        else:self.assertNotIn(self.target,{d['path'] for d in manifest['excluded']})
        if self.beforeorder:self.assertEqual([n for n in self.beforeorder if n in expected],[d['path'] for d in result['selected']])
        identities=sorted((d['path'],d['source_hash']) for d in manifest['selected'])
        self.assertEqual(hashlib.sha256('\n'.join(n+'\0'+h for n,h in identities).encode('utf-8')).hexdigest(),self.receipts[-1]['baseline']['signature']['paths'])
        for row in manifest['selected']:
            self.assertEqual(hashlib.sha256(self.images[row['path']][0]).hexdigest(),row['source_hash'])
            if row['path'] in self.beforepack:self.assertEqual(round(self.beforepack[row['path']],6),row['score'])
        for name,image in self.images.items():self.assertEqual(image,((self.root/name).read_bytes(),(self.root/name).stat().st_mtime_ns))

    def case(self,kind,mode):
        self.index(kind)
        self.expected_reason=PUBLIC['oracles'][kind+'_reason']
        if mode=='boundary':self.oracle(self.retrieve(),False);return
        if mode=='raw':
            self.assertEqual({self.target,self.survivor,self.doc},{r['path'] for r in self.raw()})
            Clock.value=RealDate.fromisoformat(PUBLIC['next_day'])
            self.assertEqual({self.survivor,self.doc},{r['path'] for r in self.raw()})
            for name,image in self.images.items():self.assertEqual(image,((self.root/name).read_bytes(),(self.root/name).stat().st_mtime_ns))
            return
        if mode=='cached':Clock.value=RealDate.fromisoformat(PUBLIC['next_day']);self.oracle(self.retrieve(),True);return
        if mode=='pack':
            def pack(value):
                self.assertEqual({self.target,self.survivor,self.doc},{r['path'] for r in value['selected']})
                self.beforeorder=[d['path'] for d in value['selected']]
                self.beforepack={d['path']:float(d['adjusted_score']) for d in self.ranked if d['path'] in {v['path'] for v in value['selected']}}
                Clock.value=RealDate.fromisoformat(PUBLIC['next_day']);return value
            self.oracle(self.retrieve(pack),True)

    def test_temporal_declared_invalid(self):
        self.assertEqual('invalid',self.v.memory_temporal_reason({'review_after':'finch-no-date'}))
        self.assertEqual('invalid',self.v.memory_temporal_reason({'review_after':None}))
        self.assertEqual('invalid',self.v.memory_temporal_reason({'valid_to':2042}))

    def test_temporal_nullable_and_legacy(self):
        self.assertIsNone(self.v.memory_temporal_reason({'valid_to':None,'review_after':PUBLIC['future']}))
        self.assertIsNone(self.v.memory_temporal_reason({}))

    def test_temporal_single_calendar_observation(self):
        clock=mock.Mock();clock.today.return_value=RealDate.fromisoformat(PUBLIC['next_day']);clock.fromisoformat.side_effect=RealDate.fromisoformat
        with mock.patch.object(self.v,'date',clock):
            self.assertEqual('retired',self.v.memory_temporal_reason({'review_after':PUBLIC['boundary'],'valid_to':PUBLIC['boundary']}))
        clock.today.assert_called_once_with()

for kind in ('review','valid'):
    for mode in ('boundary','raw','cached','pack'):
        def test(self,kind=kind,mode=mode):self.case(kind,mode)
        setattr(TemporalDeliveryTests,'test_%s_%s'%(kind,mode),test)

if __name__=='__main__':unittest.main()
