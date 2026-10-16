"""NEW Scoter authority inventory and portable handoff UTF-8 paths."""
import contextlib
import copy
from datetime import datetime, timezone
import hashlib
import importlib
import io
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
P = json.loads((ROOT/'tests/fixtures/memory_authority_utf8_public.json').read_text(encoding='utf-8'))
POSIX = unittest.skipUnless(os.name == 'posix','Raw POSIX authority path bytes unavailable')


class AuthorityUtf8Tests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='scoter-authority-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()/'project'
        self.root.mkdir()
        env = {k:os.environ[k] for k in ('PATH','HOME','LANG','LC_ALL','LC_CTYPE',
               'SystemRoot','WINDIR','COMSPEC','PATHEXT') if k in os.environ}
        env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',
                   TMPDIR=temporary.name,XDG_CONFIG_HOME=str(self.root/'xdg'))
        patch = mock.patch.dict(os.environ,env,clear=True)
        patch.start()
        self.addCleanup(patch.stop)
        subprocess.run(['git','-C',str(self.root),'init','-q','-b','scoter-utf8'],
                       env=env,capture_output=True,check=True,timeout=8)
        runtime = ROOT/'PHP Core/memory-bank/scripts'
        names = [p.stem for p in runtime.glob('*.py')]
        previous = {n:sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:
                sys.modules.pop(n,None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0,str(runtime))
        try:
            self.b = importlib.import_module('brain_runtime')
            self.r = importlib.import_module('context_retrieval')
            self.h = importlib.import_module('context_handoff')
            self.c = importlib.import_module('context')
        finally:
            sys.path.remove(str(runtime))
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config',
                    'project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,
                            ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        cfg = json.loads((self.root/'project-brain/config/runtime.json').read_text(encoding='utf-8'))
        cfg.update(automatic_promotion=False,automatic_compaction=False)
        self.write('project-brain/config/runtime.json',json.dumps(cfg).encode('utf-8'))
        self.bad = bytes.fromhex(P['bad_hex']).decode('ascii','surrogateescape')
        self.record_rel = 'project-brain/dynamic/tasks/'+P['record']['id']+'.md'
        self.handoff_rel = 'project-brain/control/handoffs/'+P['record']['id']+'.md'
        self.write(self.record_rel,self.record_bytes(P['record']))
        self.write(self.handoff_rel,self.record_bytes(P['handoff']))
        self.input = self.write('scoter-input.json',json.dumps(P['curated']).encode('utf-8'))

    def write(self,rel,raw):
        p = self.root/rel
        p.parent.mkdir(parents=True,exist_ok=True)
        p.write_bytes(raw)
        return p

    def record_bytes(self,record):
        return ('---\n'+json.dumps(record,ensure_ascii=False)+'\n---\n'+P['body']).encode('utf-8')

    def malformed_record(self,directory=False,archive=False):
        record = copy.deepcopy(P['record'])
        record.update(id=P['other_id'],external_id='scoter-other')
        parent = 'project-brain/'+('archive' if archive else 'dynamic')+'/tasks/'
        if directory:
            parent += self.bad+'/'
            name = record['id']+'.md'
        else:
            name = 'zz-'+self.bad+'.md'
        return self.write(parent+name,self.record_bytes(record))

    def authority_image(self):
        result = {}
        for rel in ('project-brain/dynamic','project-brain/archive','project-brain/indexes','project-brain/control/handoffs'):
            for p in (self.root/rel).rglob('*'):
                if p.is_file():
                    result[p.relative_to(self.root).as_posix()] = (p.read_bytes(),stat.S_IMODE(p.stat().st_mode))
        return result

    def refused(self,call,kind=None,message=None):
        with self.assertRaises(kind or self.b.BrainError) as raised:
            call()
        self.assertEqual(message or P['brain_error'],str(raised.exception))
        self.assertNotIsInstance(raised.exception,self.b.BrainRecordNotFound)

    def connection(self):
        db = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(db.close)
        return db

    def sql_image(self,db):
        return {table:[tuple(row) for row in db.execute('SELECT * FROM '+table)] for table in
                ('documents','document_metadata','document_source_state','document_links',
                 'index_state','working_tasks','task_bindings','turn_deltas')}

    def cli(self,*args):
        out,err = io.StringIO(),io.StringIO()
        with mock.patch.object(self.c,'default_root',return_value=self.root), \
                mock.patch.object(sys,'argv',['context.py',*args]), \
                contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            status = self.c.main()
        out.getvalue().encode('utf-8')
        err.getvalue().encode('utf-8')
        return status,out.getvalue(),err.getvalue()

    @POSIX
    def test_leaf_inventory_refuses_before_first_valid_yield(self):
        self.malformed_record()
        self.refused(lambda:next(self.b.iter_records(self.root)))

    @POSIX
    def test_directory_inventory_refuses_before_first_valid_yield(self):
        self.malformed_record(directory=True)
        self.refused(lambda:next(self.b.iter_records(self.root)))

    @POSIX
    def test_archive_inventory_refuses_before_active_yield(self):
        self.malformed_record(archive=True)
        self.refused(lambda:next(self.b.iter_records(self.root,include_archive=True)))

    @POSIX
    def test_existing_alias_lookup_cannot_use_partial_catalogue(self):
        self.malformed_record()
        self.refused(lambda:self.b.get_task(self.root,P['record']['external_id']))

    @POSIX
    def test_absent_alias_never_becomes_positive_notfound(self):
        self.malformed_record()
        self.refused(lambda:self.b.find_task(self.root,'scoter-absent'))

    @POSIX
    def test_creation_refused_without_authority_changes(self):
        self.malformed_record()
        before = self.authority_image()
        self.refused(lambda:self.b.create_task(self.root,'scoter-new','Inspect the wicker flange',[],[],owner='scoter-agent',privacy='public'))
        self.assertEqual(before,self.authority_image())

    @POSIX
    def test_rebind_refused_without_sql_or_authority_changes(self):
        db = self.connection()
        self.malformed_record()
        before,sql = self.authority_image(),self.sql_image(db)
        self.refused(lambda:self.c.rebind_governed_task(db,self.root,P['record']['external_id']),self.c.ContextError)
        self.assertEqual(before,self.authority_image())
        self.assertEqual(sql,self.sql_image(db))

    @POSIX
    def test_index_refusal_preserves_previous_complete_rows(self):
        db = self.connection()
        self.c.index_repository(db,self.root)
        before = self.sql_image(db)
        self.assertEqual({self.record_rel,self.handoff_rel},{row[0] for row in before['documents']})
        self.malformed_record()
        self.refused(lambda:self.c.index_repository(db,self.root,incremental=True))
        self.assertEqual(before,self.sql_image(db))

    @POSIX
    def test_managed_handoff_name_failure_escapes_index_exclusion(self):
        db = self.connection()
        self.c.index_repository(db,self.root)
        before = self.sql_image(db)
        self.write('project-brain/control/handoffs/'+self.bad+'.md',self.record_bytes(P['handoff']))
        self.refused(lambda:self.c.index_repository(db,self.root,incremental=True))
        self.assertEqual(before,self.sql_image(db))

    @POSIX
    def test_managed_handoff_name_failure_escapes_validation_catch(self):
        self.write('project-brain/control/handoffs/'+self.bad+'.md',self.record_bytes(P['handoff']))
        self.refused(lambda:self.b.validate_repository(self.root,check_freshness=False))

    @POSIX
    def test_direct_record_read_refuses_bad_name(self):
        p = self.malformed_record()
        self.refused(lambda:self.b.read_record_text(p))

    def test_held_text_parse_refuses_unencodable_path(self):
        self.refused(lambda:self.b.parse_markdown_record(self.root/(self.bad+'.md'),text=self.record_bytes(P['record']).decode('utf-8')))

    @POSIX
    def test_native_cli_validation_fixed_error(self):
        self.malformed_record()
        self.assertEqual((1,'','context: '+P['brain_error']+'\n'),self.cli('validate','--json'))

    def test_healthy_record_full_image_and_unicode_direct_read(self):
        path = self.root/self.record_rel
        self.assertEqual([(path,P['record'],P['body'])],list(self.b.iter_records(self.root)))
        unicode_path = self.write('scoter-桜-é.md',self.record_bytes(P['record']))
        self.assertEqual((P['record'],P['body']),self.b.parse_markdown_record(unicode_path))
        self.assertEqual(P['record'],self.b.get_task(self.root,P['record']['external_id']))

    def save(self,output,**changes):
        args = dict(input_json=str(self.input),output=output,detail='summary',topic=None,
                    task_id=None,source_client='other',transcript_path=None)
        args.update(changes)
        return self.h.save_handoff(self.root,**args)

    @POSIX
    def test_portable_output_name_refused_before_creation(self):
        output = self.root/(self.bad+'.md')
        self.refused(lambda:self.save(output),self.h.HandoffError,P['handoff_error'])
        self.assertFalse(output.exists())

    @POSIX
    def test_portable_input_name_refused_before_output(self):
        source = self.write(self.bad+'.json',self.input.read_bytes())
        output = self.root/'scoter-input-result.md'
        self.refused(lambda:self.save(output,input_json=str(source)),self.h.HandoffError,P['handoff_error'])
        self.assertFalse(output.exists())

    @POSIX
    def test_portable_transcript_name_is_never_stored_so_it_is_not_refused(self):
        # A full handoff keeps the transcript's bytes, not its file name.
        transcript = self.write(self.bad+'.txt',b'Scoter visible public discussion.\n')
        output = self.root/'scoter-full.md'
        self.save(output,detail='full',transcript_path=transcript)
        self.assertTrue(output.exists())
        self.assertIn(b'Scoter visible public discussion.',output.read_bytes())
        self.assertNotIn(os.fsencode(transcript.name),output.read_bytes())

    @POSIX
    def test_portable_escaped_surrogate_citation_refused_before_creation(self):
        citation = self.write(self.bad+'.txt',b'Scoter public citation.\n')
        curated = copy.deepcopy(P['curated'])
        curated['files'] = [citation.name]
        self.input.write_bytes(json.dumps(curated,ensure_ascii=True).encode('utf-8'))
        output = self.root/'scoter-citation.md'
        self.refused(lambda:self.save(output),self.h.HandoffError,P['handoff_error'])
        self.assertFalse(output.exists())

    @POSIX
    def test_portable_load_refuses_bad_name(self):
        good = self.root/'scoter-before.md'
        self.save(good)
        bad = self.root/(self.bad+'.md')
        good.rename(bad)
        before = bad.read_bytes()
        self.refused(lambda:self.h.load_handoff(self.root,bad),self.h.HandoffError,P['handoff_error'])
        self.assertEqual(before,bad.read_bytes())

    def test_portable_relative_key_refused_before_filesystem(self):
        observed = mock.Mock(return_value=False)
        with mock.patch.object(Path,'is_symlink',observed):
            self.refused(lambda:self.h._safe_relative_path(self.root,self.bad+'.txt'),self.h.HandoffError,P['handoff_error'])
        observed.assert_not_called()

    def test_portable_unicode_roundtrip_full_expected_image(self):
        output = self.root/'scoter-桜-é.md'
        fixed = datetime(2026,10,15,7,0,tzinfo=timezone.utc)
        clock = mock.Mock()
        clock.now.return_value = fixed
        with mock.patch.object(self.h,'datetime',clock):
            saved = self.save(output)
        self.assertEqual({'output':str(output),'detail':'summary','files':0,'transcript':False},saved)
        body = P['portable_body'].encode('utf-8')
        metadata = {'schema_version':1,'type':'context-handoff','created_at':fixed.isoformat(),
                    'detail':'summary','topic':None,'source_client':'other','task_id':None,
                    'repository':{'branch':'scoter-utf8','commit':None},'files':[],
                    'context':P['curated'],'body_sha256':hashlib.sha256(body).hexdigest(),'transcript':None}
        self.assertEqual(b'---\n'+json.dumps(metadata,ensure_ascii=False,sort_keys=True).encode('utf-8')+b'\n---\n'+body,output.read_bytes())
        public_metadata = {key:value for key,value in metadata.items() if key!='context'}
        self.assertEqual({'input':str(output),'metadata':public_metadata,'context':P['curated'],'drift':[]},self.h.load_handoff(self.root,output))

    def escaped_load_fixture(self):
        source = self.root/'scoter-load-escaped.md'
        self.save(source)
        original = source.read_bytes()
        encoded,body = original[4:].split(b'\n---\n',1)
        metadata = json.loads(encoded.decode('utf-8'))
        metadata['context']['files'] = [P['load_bad_citation']]
        metadata['files'] = [{'path':P['load_bad_citation'],'sha256':None}]
        # Keep the complete literal public body and its original correct hash.
        self.assertEqual(hashlib.sha256(body).hexdigest(),metadata['body_sha256'])
        source.write_bytes(b'---\n'+json.dumps(metadata,ensure_ascii=True).encode('utf-8')+b'\n---\n'+body)
        return source

    def test_portable_loaded_escaped_citation_refused_before_probes(self):
        source = self.escaped_load_fixture()
        before = source.read_bytes()
        with mock.patch.object(self.h,'_git_value') as git_probe, \
                mock.patch.object(self.h,'fingerprint_files') as file_probe:
            self.refused(lambda:self.h.load_handoff(self.root,source),self.h.HandoffError,P['handoff_error'])
        git_probe.assert_not_called()
        file_probe.assert_not_called()
        self.assertEqual(before,source.read_bytes())

    def test_portable_cli_loaded_escaped_citation_fixed_error(self):
        source = self.escaped_load_fixture()
        before = source.read_bytes()
        self.assertEqual((1,'','context: '+P['handoff_error']+'\n'),self.cli('context-load','--input',str(source),'--json'))
        self.assertEqual(before,source.read_bytes())

    @contextlib.contextmanager
    def in_directory(self,directory):
        previous = Path.cwd()
        os.chdir(directory)
        try:
            yield
        finally:
            os.chdir(previous)

    @POSIX
    def test_portable_relative_save_under_a_non_utf8_directory(self):
        # The directories above the repository are this machine's and never
        # enter a handoff; one Latin-1 name among them used to refuse it.
        directory = self.root/self.bad
        directory.mkdir()
        (directory/'input.json').write_bytes(self.input.read_bytes())
        with self.in_directory(directory):
            self.h.save_handoff(Path('.'),input_json='input.json',output=Path('result.md'),
                detail='summary',topic=None,task_id=None,source_client='other',transcript_path=None)
        self.assertTrue((directory/'result.md').exists())
        self.assertNotIn(os.fsencode(self.bad),(directory/'result.md').read_bytes())

    @POSIX
    def test_a_project_under_a_non_utf8_directory_keeps_its_memory(self):
        # Only the paths below project-brain are keys; the machine's own
        # directories above the project were checked too, and one Latin-1
        # name among them switched the Brain off.
        project = self.root/self.bad/'project'
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config',
                    'project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,project/rel,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        environment = {k:v for k,v in os.environ.items() if not k.startswith(('GIT_','CONTEXT_','ACCELERATOR_'))}
        environment.update(PYTHONDONTWRITEBYTECODE='1',GIT_OPTIONAL_LOCKS='0')
        def run(*args):
            return subprocess.run(list(args),cwd=project,env=environment,capture_output=True,timeout=120)
        self.assertEqual(0,run('git','init','-q','-b','feature/scoter-latin').returncode)
        cli = os.fsdecode(project/'memory-bank/scripts/context.py')
        started = run(sys.executable,cli,'start','--task-id','feature/scoter-latin','--goal','Scoter ledger','--json')
        self.assertEqual(0,started.returncode,started.stderr)
        read = run(sys.executable,cli,'retrieve','scoter ledger','--task-id','feature/scoter-latin')
        self.assertEqual(0,read.returncode,read.stderr)
        self.assertIn(b'working: feature/scoter-latin',read.stdout)

    @POSIX
    def test_portable_relative_load_under_a_non_utf8_directory(self):
        good = self.root/'scoter-relative-before.md'
        self.save(good)
        directory = self.root/self.bad
        directory.mkdir()
        source = directory/'source.md'
        source.write_bytes(good.read_bytes())
        before = source.read_bytes()
        with self.in_directory(directory):
            loaded = self.h.load_handoff(Path('.'),Path('source.md'))
        self.assertRegex(loaded['metadata']['body_sha256'],r'^[0-9a-f]{64}$')
        self.assertEqual(before,source.read_bytes())

    @POSIX
    def test_portable_absolute_paths_ignore_unrelated_bad_cwd(self):
        directory = self.root/self.bad
        directory.mkdir()
        output = self.root/'scoter-absolute-桜.md'
        with self.in_directory(directory):
            saved = self.save(output)
            loaded = self.h.load_handoff(self.root,output)
        self.assertEqual({'output':str(output),'detail':'summary','files':0,'transcript':False},saved)
        self.assertEqual(P['curated'],loaded['context'])
        self.assertEqual([],loaded['drift'])
        self.assertEqual(P['portable_body'].encode('utf-8'),output.read_bytes().split(b'\n---\n',1)[1])

    def test_portable_relative_paths_in_valid_cwd_keep_full_body(self):
        with self.in_directory(self.root):
            saved = self.h.save_handoff(Path('.'),input_json='scoter-input.json',output=Path('scoter-relative.md'),
                detail='summary',topic=None,task_id=None,source_client='other',transcript_path=None)
            loaded = self.h.load_handoff(Path('.'),Path('scoter-relative.md'))
        output = self.root/'scoter-relative.md'
        self.assertEqual({'output':str(output),'detail':'summary','files':0,'transcript':False},saved)
        self.assertEqual(P['curated'],loaded['context'])
        self.assertEqual([],loaded['drift'])
        self.assertEqual(P['portable_body'].encode('utf-8'),output.read_bytes().split(b'\n---\n',1)[1])


if __name__ == '__main__':
    unittest.main()
