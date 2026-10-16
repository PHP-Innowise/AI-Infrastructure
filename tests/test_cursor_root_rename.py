"""Fresh Opal recorded-root rename controls; no unknown legacy adoption."""
from __future__ import annotations
import importlib,json,os
from pathlib import Path
import shutil,subprocess,sys,tempfile,unittest
from unittest import mock
ROOT=Path(__file__).resolve().parents[1]
PUBLIC=json.loads((ROOT/'tests/fixtures/memory_final_remediation_public.json').read_text(encoding='utf-8'))
@unittest.skipUnless(shutil.which('git'),'Selected-platform rooted rename profile')
class CursorRootRenameTests(unittest.TestCase):
    def setUp(self):
        clean={key:value for key,value in os.environ.items() if not key.startswith(('GIT_','ACCELERATOR_','CONTEXT_'))}
        clean['GIT_OPTIONAL_LOCKS']='0';env=mock.patch.dict(os.environ,clean,clear=True);env.start();self.addCleanup(env.stop)
        sys.path.insert(0,str(ROOT/'PHP Core/memory-bank/scripts'))
        try:self.r=importlib.import_module('context_retrieval')
        finally:sys.path.pop(0)
        folder=tempfile.TemporaryDirectory(prefix='opal-root-rename-');self.addCleanup(folder.cleanup);self.base=Path(folder.name).resolve()
        self.root=self.base/'first';self.root.mkdir();subprocess.run(['git','init','-q',str(self.root)],check=True,capture_output=True)
        self.layout=mock.patch.object(self.r.workspace_roots,'TOOLING_ROOT',self.root);self.layout.start();self.addCleanup(self.layout.stop)
        self.assertEqual('written',self.call()['state'])
        self.raw=self.rule.read_bytes();self.old_proof=self.proof.read_bytes();self.inode=self.rule.stat().st_ino
    @property
    def rule(self):return self.root/'.cursor/rules/working-memory.mdc'
    @property
    def proof(self):return self.root/'memory-bank/local/cursor-rule.json'
    def call(self,action='write',**options):
        return self.r.cursor_rule(self.root,action,task=PUBLIC['cursor_task'],capsule=PUBLIC['cursor_capsule'],**options)
    def rename(self):
        destination=self.base/'renamed';self.root.rename(destination);self.root=destination
        self.layout.stop();self.layout=mock.patch.object(self.r.workspace_roots,'TOOLING_ROOT',self.root);self.layout.start();self.addCleanup(self.layout.stop)
    def reconciled(self,state):
        self.assertEqual(self.raw,self.rule.read_bytes());self.assertEqual(self.inode,self.rule.stat().st_ino)
        note=json.loads(self.proof.read_bytes());self.assertEqual(str(self.root),note['root']);self.assertEqual('prompt',note['phase'])
        self.assertEqual(PUBLIC['cursor_task'],note['task'])
    def test_same_capsule_rename_reconciles_receipt_only(self):
        self.rename();self.assertEqual('unchanged',self.call()['state']);self.reconciled('unchanged')
    def test_keep_reconciles_without_new_capsule(self):
        self.rename();self.assertEqual('kept',self.call('keep')['state']);self.reconciled('kept')
    def test_prompt_held_reconciles_and_preserves_phase(self):
        self.rename();self.assertEqual('kept',self.call('prompt-held')['state']);self.reconciled('kept')
    def test_turn_does_not_replace_held_prompt(self):
        self.rename();self.assertEqual('kept',self.call(phase='turn')['state']);self.reconciled('kept')
    def test_changed_capsule_uses_normal_publication(self):
        self.rename();reply=self.r.cursor_rule(self.root,'write',task=PUBLIC['cursor_task'],capsule=PUBLIC['cursor_changed'])
        self.assertEqual('written',reply['state']);self.assertIn(b'two current reviewers',self.rule.read_bytes());self.assertEqual(str(self.root),json.loads(self.proof.read_bytes())['root'])
    def test_retire_after_rename_counts_exact_bytes(self):
        self.rename();reply=self.call('retire');self.assertEqual('removed',reply['state']);self.assertEqual(len(self.raw),reply['bytes_removed'])
        self.assertFalse(self.rule.exists());self.assertFalse(self.proof.exists())
    def test_copied_root_reconciles_exact_managed_bytes(self):
        destination=self.base/'copy';shutil.copytree(self.root,destination);self.root=destination
        self.layout.stop();self.layout=mock.patch.object(self.r.workspace_roots,'TOOLING_ROOT',self.root);self.layout.start();self.addCleanup(self.layout.stop)
        self.assertEqual('unchanged',self.call()['state']);self.assertEqual(self.raw,self.rule.read_bytes())
        self.assertEqual(str(self.root),json.loads(self.proof.read_bytes())['root'])
    def test_edited_or_replaced_rule_keeps_foreign_bytes(self):
        self.rename()
        for replace in (False,True):
            raw=self.raw if replace else self.raw+b' human edit'
            if replace:
                temp=self.rule.with_name('foreign');temp.write_bytes(raw);temp.replace(self.rule)
            else:self.rule.write_bytes(raw)
            self.assertEqual('unchanged' if replace else 'preserved',self.call()['state'])
            self.assertEqual(raw,self.rule.read_bytes())
            if not replace:self.assertEqual(self.old_proof,self.proof.read_bytes())
    def test_orphan_after_rename_recreates_from_new_capsule_only(self):
        self.rename();self.rule.unlink()
        self.assertEqual('written',self.call()['state']);self.assertEqual(self.raw,self.rule.read_bytes())
        self.assertEqual(str(self.root),json.loads(self.proof.read_bytes())['root'])
    def test_tracked_leaf_prevents_receipt_reconciliation(self):
        self.rename();subprocess.run(['git','-C',str(self.root),'add','-f','.cursor/rules/working-memory.mdc'],check=True,capture_output=True)
        self.assertEqual('preserved',self.call('keep')['state']);self.assertEqual(self.old_proof,self.proof.read_bytes())
    def test_locked_recheck_preserves_swapped_foreign_leaf(self):
        self.rename();original=self.r._cursor_git_untracked;calls=[]
        def swap(root):
            calls.append(1)
            if len(calls)==2:self.rule.write_bytes(b'Foreign Opal leaf')
            return original(root)
        with mock.patch.object(self.r,'_cursor_git_untracked',side_effect=swap):self.assertEqual('preserved',self.call('keep')['state'])
        self.assertEqual(b'Foreign Opal leaf',self.rule.read_bytes());self.assertEqual(self.old_proof,self.proof.read_bytes())
    def test_receipt_failure_is_incomplete_and_keeps_rule(self):
        self.rename()
        with mock.patch.object(self.r,'_cursor_publish',side_effect=OSError('owned receipt failure')):
            self.assertEqual('incomplete',self.call('keep')['state'])
        self.assertEqual(self.raw,self.rule.read_bytes());self.assertEqual(self.old_proof,self.proof.read_bytes())
    def test_metadata_only_rename_uses_postpublication_race_guard(self):
        self.rename();original=self.r._cursor_publish
        note=json.loads(self.old_proof);note['file_identity'][4]+=1
        self.proof.write_text(json.dumps(note),encoding='utf-8')
        def swap(*args,**kwargs):
            result=original(*args,**kwargs);self.rule.write_bytes(b'Foreign leaf after metadata reconciliation');return result
        with mock.patch.object(self.r,'_cursor_publish',side_effect=swap):self.assertEqual('incomplete',self.call()['state'])
        self.assertEqual(b'Foreign leaf after metadata reconciliation',self.rule.read_bytes())
    def test_postpublication_race_is_incomplete_without_rollback(self):
        self.rename();original=self.r._cursor_publish
        def swap(*args,**kwargs):
            result=original(*args,**kwargs);self.rule.write_bytes(b'Foreign leaf after proof');return result
        with mock.patch.object(self.r,'_cursor_publish',side_effect=swap):self.assertEqual('incomplete',self.call('keep')['state'])
        self.assertEqual(b'Foreign leaf after proof',self.rule.read_bytes())
if __name__=='__main__':unittest.main()
