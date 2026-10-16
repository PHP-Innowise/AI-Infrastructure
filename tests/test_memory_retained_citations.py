"""Fresh Skua native retained-memory citation admission regressions."""
from __future__ import annotations
import copy
import base64
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

ROOT=Path(__file__).resolve().parents[1]
PUBLIC=json.loads((ROOT/'tests/fixtures/memory_retained_citation_public.json').read_text(encoding='utf-8'))


class RetainedCitationTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='skua-citations-');self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name);self.chunk=self.root/PUBLIC['chunk_path']
        self.env={key:value for key,value in os.environ.items() if not key.startswith(('GIT_','CONTEXT_','ACCELERATOR_'))}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',PYTHONIOENCODING='utf-8')
        for relative in ('memory-bank/scripts','memory-bank/templates','project-brain/config','project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/relative,self.root/relative,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        self.chunk.parent.mkdir(parents=True)
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps({'mode':'governed','automatic_promotion':False}),encoding='utf-8')
        (self.root/'.gitignore').write_text('memory-bank/local/\nproject-brain/local/\n',encoding='utf-8')
        for name,text in [(PUBLIC['source'],PUBLIC['source_text']),(PUBLIC['other_source'],PUBLIC['other_text']),('README.md',PUBLIC['README'])]:
            leaf=self.root/name;leaf.parent.mkdir(parents=True,exist_ok=True);leaf.write_text(text,encoding='utf-8')
        subprocess.run(['git','-C',str(self.root),'init','-q','-b',PUBLIC['task_id']],env=self.env,capture_output=True,check=True)
        self.cli=self.root/'memory-bank/scripts/context.py'
        self.call('start','--task-id',PUBLIC['task_id'],'--goal',PUBLIC['goal'],'--source',PUBLIC['other_source'],'--json')

    def call(self,*args):
        result=subprocess.run([sys.executable,'-B',str(self.cli),'--root',str(self.root),*args],env=self.env,cwd=self.root,capture_output=True,timeout=12)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'));return json.loads(result.stdout)

    def warm(self,metadata=None):
        self.metadata=copy.deepcopy(PUBLIC['metadata'] if metadata is None else metadata)
        self.chunk.write_text('---\n'+json.dumps(self.metadata,ensure_ascii=False,indent=2,sort_keys=True)+'\n---\n\n'+PUBLIC['body'],encoding='utf-8')
        self.call('index','--json')
        names=('context','context_retrieval','brain_runtime','automatic_query','validate','memory_results','workspace_roots','context_handoff')
        # Modules imported from this temporary copy leave sys.modules when the
        # test ends: a later module in the same process (the Windows job runs
        # them together) must not import a runtime whose files are gone.
        previous={name:sys.modules.pop(name) for name in names if name in sys.modules}
        def restore():
            for name in names:sys.modules.pop(name,None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0,str(self.cli.parent))
        try:
            self.context=importlib.import_module('context')
        finally:sys.path.pop(0)
        self.db=self.context.connect(self.root/'memory-bank/local/context.db');self.addCleanup(self.db.close)
        self.original=self.chunk.read_bytes();info=self.chunk.stat();self.stat=(info.st_mtime_ns,info.st_size)
        self.assertEqual(1,self.db.execute('SELECT COUNT(*) FROM documents WHERE path=?',(PUBLIC['chunk_path'],)).fetchone()[0])
        self.assertIn(PUBLIC['chunk_path'],{row['path'] for row in self.packet()['selected']})

    def packet(self):
        return self.context.assemble_capsule(self.db,self.root,mode='governed',query=PUBLIC['query'],task_id=PUBLIC['task_id'],limit=3,ephemeral=True,refresh_index=False,gate_mode='off',render=True)

    def refresh(self):
        layers,result,warnings=self.context.refresh_layers(self.db,self.root)
        self.assertEqual([],warnings);self.assertEqual({'updated'},set(layers.values()));self.assertTrue(result['incremental'])
        return result

    def assert_evicted(self,result,reason='invalid',unchanged=True):
        self.assertEqual(1,result['removed'])
        self.assertIn({'path':PUBLIC['chunk_path'],'reason':reason},result['excluded'])
        for table in ('documents','document_metadata','document_links','document_source_state'):
            with self.subTest(table=table):self.assertEqual(0,self.db.execute('SELECT COUNT(*) FROM '+table+' WHERE path=?',(PUBLIC['chunk_path'],)).fetchone()[0])
        packet=self.packet();self.assertTrue(packet['working']);self.assertEqual([],packet['warnings'])
        self.assertNotIn(PUBLIC['chunk_path'],{row['path'] for row in packet['selected']})
        text='\n'.join(self.context.render_capsule_lines(packet,self.context.capsule_excerpts(self.db,packet)))
        self.assertNotIn('nine amber prisms',text)
        if unchanged:
            self.assertEqual(self.original,self.chunk.read_bytes());info=self.chunk.stat();self.assertEqual(self.stat,(info.st_mtime_ns,info.st_size))

    def test_missing_digest_chunk_evicted_after_citation_deletion(self):
        self.warm();(self.root/PUBLIC['source']).unlink();self.assert_evicted(self.refresh())

    def test_partial_digest_chunk_evicted_after_undigested_citation_deletion(self):
        metadata=dict(PUBLIC['metadata'],sources=[PUBLIC['source'],PUBLIC['other_source']],
                      source_digests=[{'path':PUBLIC['other_source'],'sha256':PUBLIC['other_sha256']}])
        self.warm(metadata);(self.root/PUBLIC['source']).unlink();self.assert_evicted(self.refresh())

    def test_complete_digest_chunk_evicted_without_freshening_hashes(self):
        metadata=dict(PUBLIC['metadata'],source_digests=[{'path':PUBLIC['source'],'sha256':PUBLIC['source_sha256']}])
        self.warm(metadata);(self.root/PUBLIC['source']).unlink();self.assert_evicted(self.refresh())

    def test_surviving_partial_citations_retain_exact_metadata_and_ordinary_cache(self):
        metadata=dict(PUBLIC['metadata'],sources=[PUBLIC['source'],PUBLIC['other_source']],
                      source_digests=[{'path':PUBLIC['other_source'],'sha256':PUBLIC['other_sha256']}])
        self.warm(metadata)
        before=[tuple(row) for row in self.db.execute('SELECT * FROM document_metadata ORDER BY path')]
        reader=Path.read_text;opened=[]
        def read(leaf,*args,**kwargs):
            if leaf==self.root/'README.md':raise AssertionError('ordinary cached text must not be reread')
            if leaf==self.chunk:opened.append(leaf)
            return reader(leaf,*args,**kwargs)
        with mock.patch.object(Path,'read_text',read):result=self.refresh()
        self.assertGreaterEqual(len(opened),1);self.assertLessEqual(len(opened),2)
        self.assertEqual(0,result['removed']);self.assertEqual(before,[tuple(row) for row in self.db.execute('SELECT * FROM document_metadata ORDER BY path')])
        self.assertEqual(self.original,self.chunk.read_bytes());self.assertIn(PUBLIC['chunk_path'],{row['path'] for row in self.packet()['selected']})

    def test_external_url_claim_remains_admitted_without_lookup_or_new_hash(self):
        metadata=dict(PUBLIC['metadata'],sources=['https://example.invalid/skua#policy'])
        self.warm(metadata)
        with mock.patch('urllib.request.urlopen',side_effect=AssertionError('no external citation lookup')):result=self.refresh()
        self.assertEqual(0,result['removed']);self.assertEqual(self.original,self.chunk.read_bytes())
        self.assertIn(PUBLIC['chunk_path'],{row['path'] for row in self.packet()['selected']})

    def test_anchor_and_local_provenance_citation_follow_current_existence(self):
        source='project-brain/archive/decisions/skua-provenance.txt';leaf=self.root/source;leaf.parent.mkdir(parents=True,exist_ok=True);leaf.write_text('Skua public archived provenance.\n',encoding='utf-8')
        metadata=dict(PUBLIC['metadata'],sources=[PUBLIC['source']+'#tide',source+'#receipt'])
        self.warm(metadata);self.assertEqual(0,self.refresh()['removed'])
        leaf.unlink();self.assert_evicted(self.refresh())

    @unittest.skipUnless(os.name=='posix','actual unprivileged symlink containment profile required')
    def test_citation_replaced_with_external_symlink_is_evicted(self):
        self.warm();temp=tempfile.TemporaryDirectory(prefix='skua-foreign-');self.addCleanup(temp.cleanup)
        foreign=Path(temp.name)/'outside.php';foreign.write_text(PUBLIC['source_text'],encoding='utf-8')
        leaf=self.root/PUBLIC['source'];leaf.unlink();leaf.symlink_to(foreign)
        self.assert_evicted(self.refresh())

    def test_same_stat_invalid_utf8_chunk_is_excluded_per_file(self):
        self.warm();info=self.chunk.stat();self.chunk.write_bytes(b'\xff'+self.original[1:]);os.utime(self.chunk,ns=(info.st_atime_ns,info.st_mtime_ns))
        self.assert_evicted(self.refresh(),reason='invalid-utf8',unchanged=False)

    def assert_current_image_excluded(self,name):
        self.warm();case=PUBLIC['same_stat_images'][name];image=base64.b64decode(case['base64'])
        self.assertEqual(len(self.original),case['bytes']);self.assertEqual(len(self.original),len(image))
        info=self.chunk.stat();self.chunk.write_bytes(image);os.utime(self.chunk,ns=(info.st_atime_ns,info.st_mtime_ns))
        self.assert_evicted(self.refresh(),reason=case['reason'],unchanged=False)
        self.assertEqual(image,self.chunk.read_bytes());self.assertEqual(self.stat,(self.chunk.stat().st_mtime_ns,self.chunk.stat().st_size))

    def test_same_stat_terminal_status_never_reuses_active_chunk(self):
        self.assert_current_image_excluded('status')

    def test_same_stat_expiry_never_reuses_future_cached_boundary(self):
        self.assert_current_image_excluded('expired')

    def test_same_stat_current_secret_failure_isolated_without_raw_diagnostic(self):
        self.assert_current_image_excluded('secret')


if __name__=='__main__':unittest.main()
