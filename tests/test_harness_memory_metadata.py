"""Fresh metadata bank discovery controls; owned files, no native runtime."""
import copy
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'harness'/'src'))
from harness import knowledge,sessions

FILES={'memory-bank/scripts/context.py': '# public inert availability marker\n', 'project-brain/config/runtime.json': '{"mode":"lightweight"}\n', 'memory-bank/chunks/note-z.md': '# Public Z\n\nKeep this preview text.\n', 'memory-bank/notes-a.md': '# Public A\n\nMetadata operations need no preview.\n', 'Symfony/memory-bank/scripts/context.py': '# public nested availability marker\n', 'Symfony/project-brain/config/runtime.json': '{"mode":"governed"}\n', 'Symfony/memory-bank/chunks/note-s.md': '# Public nested\n\nSeparate bank.\n'}
BANKS=[{'id': 'memory-bank', 'path': 'memory-bank', 'name': 'Project memory'}, {'id': 'Laravel/memory-bank', 'path': 'Laravel/memory-bank', 'name': 'Laravel'}, {'id': 'Symfony/memory-bank', 'path': 'Symfony/memory-bank', 'name': 'Symfony'}, {'id': 'PHP Core/memory-bank', 'path': 'PHP Core/memory-bank', 'name': 'PHP Core'}, {'id': 'Cms/wordpress/memory-bank', 'path': 'Cms/wordpress/memory-bank', 'name': 'WordPress'}]
INFO={'project_id': 'public-metadata-project', 'banks': [{'id': 'memory-bank', 'path': 'memory-bank', 'name': 'Project memory'}, {'id': 'Laravel/memory-bank', 'path': 'Laravel/memory-bank', 'name': 'Laravel'}, {'id': 'Symfony/memory-bank', 'path': 'Symfony/memory-bank', 'name': 'Symfony'}, {'id': 'PHP Core/memory-bank', 'path': 'PHP Core/memory-bank', 'name': 'PHP Core'}, {'id': 'Cms/wordpress/memory-bank', 'path': 'Cms/wordpress/memory-bank', 'name': 'WordPress'}], 'bank_id': 'memory-bank', 'runtime_available': True, 'brain_available': True, 'root': '', 'mode': 'lightweight', 'attached': None}
NESTED_INFO={'project_id': 'public-metadata-project', 'banks': [{'id': 'memory-bank', 'path': 'memory-bank', 'name': 'Project memory'}, {'id': 'Laravel/memory-bank', 'path': 'Laravel/memory-bank', 'name': 'Laravel'}, {'id': 'Symfony/memory-bank', 'path': 'Symfony/memory-bank', 'name': 'Symfony'}, {'id': 'PHP Core/memory-bank', 'path': 'PHP Core/memory-bank', 'name': 'PHP Core'}, {'id': 'Cms/wordpress/memory-bank', 'path': 'Cms/wordpress/memory-bank', 'name': 'WordPress'}], 'bank_id': 'Symfony/memory-bank', 'runtime_available': True, 'brain_available': True, 'root': 'Symfony', 'mode': 'governed', 'attached': None}
EMPTY_INFO={'project_id': 'public-metadata-project', 'banks': [], 'bank_id': None, 'runtime_available': False, 'brain_available': False, 'root': '', 'mode': None, 'attached': None}
LISTING={'project_id': 'public-metadata-project', 'banks': [{'id': 'memory-bank', 'path': 'memory-bank', 'name': 'Project memory'}, {'id': 'Laravel/memory-bank', 'path': 'Laravel/memory-bank', 'name': 'Laravel'}, {'id': 'Symfony/memory-bank', 'path': 'Symfony/memory-bank', 'name': 'Symfony'}, {'id': 'PHP Core/memory-bank', 'path': 'PHP Core/memory-bank', 'name': 'PHP Core'}, {'id': 'Cms/wordpress/memory-bank', 'path': 'Cms/wordpress/memory-bank', 'name': 'WordPress'}], 'bank_id': 'memory-bank', 'entries': [{'path': 'chunks/note-z.md', 'title': 'Public Z', 'bytes': 36, 'kind': 'chunk'}, {'path': 'notes-a.md', 'title': 'Public A', 'bytes': 49, 'kind': 'document'}], 'truncated': False}
DOCUMENT={'project_id': 'public-metadata-project', 'bank_id': 'memory-bank', 'path': 'chunks/note-z.md', 'title': 'Public Z', 'bytes': 36, 'kind': 'chunk', 'content': '# Public Z\n\nKeep this preview text.\n', 'truncated': False}
IO_EXPECTED={'before': {'markdown_reads': 2, 'markdown_bytes': 85, 'scandirs': 2, 'total_read_bytes': 108}, 'after': {'markdown_reads': 0, 'markdown_bytes': 0, 'scandirs': 0, 'total_read_bytes': 23}, 'script_read_limit': 0, 'config_read_limit': 65536, 'provider_tokens': None}


def populate(folder):
    for row in BANKS:(folder/row['id']).mkdir(parents=True,exist_ok=True)
    for name,text in FILES.items():
        filename=folder/name;filename.parent.mkdir(parents=True,exist_ok=True);filename.write_text(text,encoding='utf8')

class MemoryMetadataTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory(prefix='memory-metadata-public-')
        self.addCleanup(temporary.cleanup);self.folder=Path(temporary.name)
        self.project=self.folder/'project';self.project.mkdir();populate(self.project)
        self.store=object.__new__(sessions.Sessions)
        self.store.project=lambda key:{'path':str(self.project)}
        self.store.accelerators=SimpleNamespace(knowledge=lambda key,root:None)
        self.manager=object.__new__(knowledge.KnowledgeManager);self.manager.sessions=self.store
        environment=patch.dict(os.environ,{'PROJECT_BRAIN_MODE':''})
        environment.start();self.addCleanup(environment.stop)

    def test_selection_literal_priority_and_dynamic_inventory(self):
        self.assertEqual(sessions._memory_bank_selection(self.project),(BANKS,'memory-bank'))
        self.assertEqual(sessions._memory_bank_selection(self.project,'Symfony/memory-bank'),(BANKS,'Symfony/memory-bank'))
        shutil.rmtree(self.project/'Laravel/memory-bank')
        expected=[row for row in BANKS if row['id']!='Laravel/memory-bank']
        self.assertEqual(sessions._memory_bank_selection(self.project),(expected,'memory-bank'))
        (self.project/'Laravel/memory-bank').mkdir(parents=True)
        self.assertEqual(sessions._memory_bank_selection(self.project),(BANKS,'memory-bank'))
        empty=self.folder/'empty';empty.mkdir()
        self.assertEqual(sessions._memory_bank_selection(empty),([],None))
        self.assertEqual(self.manager.info('public-metadata-project',_root=empty),EMPTY_INFO)

    def test_supplied_selectors_and_bank_before_document_errors(self):
        for selected in ('missing-bank','',[],{},False):
            with self.subTest(selector=type(selected).__name__):
                with self.assertRaisesRegex(sessions.SessionError,r'^Memory bank is not available\.$'):
                    self.store.memory('public-metadata-project',selected,'../invalid')
                with self.assertRaisesRegex(sessions.SessionError,r'^Memory bank is not available\.$'):
                    self.manager.info('public-metadata-project',selected)
        with self.assertRaisesRegex(sessions.SessionError,r'^Memory bank is not available\.$'):
            self.store.memory('public-metadata-project',None,'../invalid')
        with self.assertRaisesRegex(sessions.SessionError,r'^Invalid memory document\.$'):
            self.store.memory('public-metadata-project','memory-bank','../invalid')

    def test_candidate_open_skip_and_close_exception_boundaries(self):
        with patch.object(sessions,'open_project_path',side_effect=[OSError('public skip'),101,102,103,104]),patch.object(sessions.fs,'close') as closed:
            banks,selected=sessions._memory_bank_selection(self.project)
            self.assertEqual(banks,BANKS[1:]);self.assertEqual(selected,'Laravel/memory-bank')
            self.assertEqual(closed.call_count,4)
        with patch.object(sessions,'open_project_path',side_effect=ValueError('public non-os')):
            with self.assertRaises(ValueError):sessions._memory_bank_selection(self.project)
        with patch.object(sessions,'open_project_path',return_value=101),patch.object(sessions.fs,'close',side_effect=OSError('public close')):
            with self.assertRaises(OSError):sessions._memory_bank_selection(self.project)

    def test_info_full_literals_without_scans_or_markdown_reads(self):
        original=sessions.read_context;requests=[]
        def read(root,name,limit=0):
            self.assertFalse(name.endswith('.md'));requests.append((name,limit));return original(root,name,limit)
        with patch.object(self.store,'memory',side_effect=AssertionError('unused public listing')),patch.object(sessions.fs,'scandir',side_effect=AssertionError('unused preview scan')),patch.object(knowledge,'read_context',side_effect=read):
            self.assertEqual(self.manager.info('public-metadata-project'),INFO)
            self.assertEqual(self.manager.info('public-metadata-project','Symfony/memory-bank'),NESTED_INFO)
        self.assertEqual(requests,[('memory-bank/scripts/context.py',0),('project-brain/config/runtime.json',65536),('Symfony/memory-bank/scripts/context.py',0),('Symfony/project-brain/config/runtime.json',65536)])

    def test_info_avoids_preview_failure_public_browser_still_closes(self):
        original_close=sessions.fs.close
        original_open=sessions.open_project_path
        opened_previews=[];live_previews=set();closed_previews=[]
        def opened_scan(root,name,directory=False):
            handle=original_open(root,name,directory)
            if name=='memory-bank/chunks':
                opened_previews.append(handle);live_previews.add(handle)
            return handle
        def closed_scan(handle):
            was_preview=handle in live_previews
            original_close(handle)
            if was_preview:
                live_previews.remove(handle);closed_previews.append(handle)
        with patch.object(sessions.fs,'scandir',side_effect=OSError('public preview scan')),patch.object(sessions.fs,'close',side_effect=closed_scan),patch.object(sessions,'open_project_path',side_effect=opened_scan):
            self.assertEqual(self.manager.info('public-metadata-project'),INFO)
            with self.assertRaises(OSError):self.store.memory('public-metadata-project')
            self.assertEqual(len(opened_previews),1)
            self.assertEqual(closed_previews,opened_previews)
            self.assertEqual(live_previews,set())
        # Preview-directory close failures remain observable after discovery.
        original_open=sessions.open_project_path;preview=set()
        def opened(root,name,directory=False):
            handle=original_open(root,name,directory)
            if name=='memory-bank/chunks':preview.add(handle)
            return handle
        def closing(handle):
            original_close(handle)
            if handle in preview:
                preview.remove(handle);raise OSError('public preview close')
        with patch.object(sessions,'open_project_path',side_effect=opened),patch.object(sessions.fs,'close',side_effect=closing):
            with self.assertRaises(OSError):self.store.memory('public-metadata-project')

    def test_public_listing_document_bounds_and_nofollow(self):
        self.assertEqual(self.store.memory('public-metadata-project'),LISTING)
        original=sessions.read_context;limits=[]
        def read(root,name,limit=0):
            limits.append((name,limit));return original(root,name,limit)
        with patch.object(sessions,'read_context',side_effect=read):
            self.assertEqual(self.store.memory('public-metadata-project','memory-bank','chunks/note-z.md'),DOCUMENT)
        self.assertEqual(limits,[('memory-bank/chunks/note-z.md',256*1024)])
        (self.project/'memory-bank/chunks/linked.md').symlink_to(self.project/'memory-bank/chunks/note-z.md')
        with self.assertRaisesRegex(sessions.SessionError,'Memory document is not available'):
            self.store.memory('public-metadata-project','memory-bank','chunks/linked.md')
        empty=self.folder/'linked-project';empty.mkdir()
        (empty/'memory-bank').symlink_to(self.project/'memory-bank',target_is_directory=True)
        self.assertEqual(sessions._memory_bank_selection(empty),([],None))
        with patch.object(sessions,'MEMORY_LIST_LIMIT',1):
            limited=self.store.memory('public-metadata-project')
        self.assertEqual(len(limited['entries']),1);self.assertTrue(limited['truncated'])

    def test_attached_state_and_explicit_root_resolution(self):
        state=self.folder/'state';state.mkdir();populate(state)
        (state/'memory-bank/scripts/context.py').unlink()
        unrelated=self.folder/'unattached';unrelated.mkdir()
        self.store.project=lambda key:{'path':str(unrelated)}
        home=self.folder/'home';(home/'memory-bank/scripts').mkdir(parents=True)
        (home/'memory-bank/scripts/context.py').write_text('# public attached marker\n')
        attached={'state':state,'project':unrelated,'home':home,'environment':{},'edition':'public-edition'}
        with patch.object(self.store.accelerators,'knowledge',return_value=attached) as attachment:
            expected={**INFO,'attached':{'edition':'public-edition','home':str(home),'state':str(state)}}
            self.assertEqual(self.manager.info('public-metadata-project'),expected)
            attachment.assert_called_once_with('public-metadata-project',unrelated)
            attachment.reset_mock()
            self.assertEqual(self.store.memory('public-metadata-project'),LISTING)
            attachment.assert_called_once_with('public-metadata-project',unrelated)
        with patch.object(self.store.accelerators,'knowledge',side_effect=AssertionError('duplicate root resolution')):
            self.assertEqual(self.store.memory('public-metadata-project',_root=state),LISTING)

    def test_modes_config_missing_and_brain_presence_remain_observable(self):
        for override,expected in [('governed','governed'),('lightweight','lightweight'),('invalid',None)]:
            with patch.dict(os.environ,{'PROJECT_BRAIN_MODE':override}):
                self.assertEqual(self.manager.info('public-metadata-project'),{**INFO,'mode':expected})
        config=self.project/'project-brain/config/runtime.json'
        config.write_text('{"mode":"not-a-mode"}\n')
        self.assertEqual(self.manager.info('public-metadata-project'),{**INFO,'mode':'governed'})
        shutil.rmtree(self.project/'project-brain')
        self.assertEqual(self.manager.info('public-metadata-project'),{**INFO,'brain_available':False,'mode':'governed'})
        (self.project/'memory-bank/scripts/context.py').unlink()
        self.assertEqual(self.manager.info('public-metadata-project'),{**INFO,'runtime_available':False,'brain_available':False,'mode':'governed'})

if __name__=='__main__':unittest.main()
