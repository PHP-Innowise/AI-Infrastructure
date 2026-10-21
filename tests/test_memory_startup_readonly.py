"""Fresh Teal startup: real SQLite locks, read-only delivery and current guards."""
from __future__ import annotations
import importlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock
from tests.hook_shell import BASH
ROOT=Path(__file__).resolve().parents[1]
PUBLIC=json.loads((ROOT/'tests/fixtures/memory_startup_readonly_public.json').read_text(encoding='utf-8'))

class MemoryStartupReadonlyTests(unittest.TestCase):
    def setUp(self):
        tmp=tempfile.TemporaryDirectory(prefix='teal-startup-');self.addCleanup(tmp.cleanup);self.root=Path(tmp.name).resolve()
        self.env={k:v for k,v in os.environ.items() if not k.startswith(('GIT_','CONTEXT_','ACCELERATOR_'))}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',PYTHONIOENCODING='utf-8',CONTEXT_HOOK_BUDGET='1')
        for relative in ('memory-bank/scripts','memory-bank/templates','project-brain/config','project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/relative,self.root/relative,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        original = os.environ.get('TEAL_ORIGINAL_CONTEXT')
        if original:
            shutil.copy2(original, self.root/'memory-bank/scripts/context.py')
        for host in ('.claude','.codex','.cursor'):
            leaf=self.root/host/'hooks';leaf.mkdir(parents=True)
            shutil.copy2(ROOT/'PHP Core'/host/'hooks/working-memory-read.sh',leaf/'working-memory-read.sh')
        (self.root/'memory-bank/chunks').mkdir()
        (self.root/'.gitignore').write_text('memory-bank/local/\nproject-brain/local/\n.cursor/rules/working-memory.mdc\n',encoding='utf-8')
        shutil.copy2(ROOT/'PHP Core/.cursor/hooks.json', self.root/'.cursor/hooks.json')
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps({'mode':'governed','automatic_promotion':False}),encoding='utf-8')
        source=self.root/PUBLIC['source'];source.parent.mkdir();source.write_text(PUBLIC['source_text'],encoding='utf-8')
        subprocess.run(['git','-C',str(self.root),'init','-q','-b',PUBLIC['task_id']],env=self.env,check=True,capture_output=True)
        self.cli=self.root/'memory-bank/scripts/context.py';self.task=PUBLIC['task_id'];self.database=self.root/'memory-bank/local/context.db'
        self.call('start','--task-id',self.task,'--goal',PUBLIC['goal'],'--source',PUBLIC['source'],'--json')
        self.call('update','--task-id',self.task,'--progress',PUBLIC['progress'],'--next-step',PUBLIC['next'][0],'--revision','1','--json')
        normal=self.refresh(expected=0);self.uuid=normal['capsule']['task_uuid']
        self.assertIn(PUBLIC['forbidden_fact'],normal['capsule_text'])
        # The runtime imported here comes from this temporary copy and leaves
        # sys.modules with it: a later module in the same process (the Windows
        # job runs them together) must not import files that are gone.
        names=[leaf.stem for leaf in self.cli.parent.glob('*.py')]
        previous={name:sys.modules.pop(name) for name in names if name in sys.modules}
        def restore():
            for name in names:sys.modules.pop(name,None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        scripts=str(self.cli.parent);sys.path.insert(0,scripts)
        try:
            self.context=importlib.import_module('context');self.brain=importlib.import_module('brain_runtime')
        finally:sys.path.remove(scripts)

    def call(self,*args,expected=0,timeout=12):
        result=subprocess.run([sys.executable,'-B',str(self.cli),'--root',str(self.root),*args],cwd=self.root,env=self.env,capture_output=True,timeout=timeout)
        self.assertEqual(expected,result.returncode,result.stderr.decode('utf-8','replace'));self.assertNotIn(b'Traceback',result.stderr)
        return json.loads(result.stdout)
    def refresh(self,*extra,expected=1,timeout=12):
        return self.call('refresh','--query',PUBLIC['query'],'--task-id',self.task,'--host','codex','--ephemeral','--sanitize','--json',*extra,expected=expected,timeout=timeout)
    def readonly(self,*extra):return self.refresh('--read-only',*extra)
    def db(self):
        connection=sqlite3.connect(self.database);self.addCleanup(connection.close);return connection
    def assert_degraded(self,reply,working=True):
        expected=PUBLIC['expected'];self.assertEqual(expected['startup_profile'],reply['startup_profile'])
        for layer in ('procedural','semantic','episodic'):self.assertEqual('failed',reply[layer])
        for key in ('documents','reused','counts'):self.assertEqual(expected[key],reply[key])
        self.assertFalse(reply['persisted']);self.assertNotIn('funnel_diagnosis',reply)
        capsule=reply['capsule']
        if not working:
            self.assertIsNone(capsule);self.assertNotIn(PUBLIC['progress'],reply.get('capsule_text',''));return
        self.assertEqual(PUBLIC['progress'],capsule['working']['progress'])
        for layer in ('procedural','semantic','episodic'):self.assertEqual([],capsule[layer])
        for key in ('selected','no_match'):self.assertEqual([],capsule.get(key,[]))
        self.assertIsNone(capsule.get('manifest'));self.assertIsNone(capsule.get('last_turn'))
        self.assertNotIn(PUBLIC['forbidden_fact'],reply['capsule_text']);self.assertIn(expected['safe_warning'],reply['capsule_text'])
    def image(self):
        return {str(p.relative_to(self.root)):p.read_bytes() for p in self.root.rglob('*') if p.is_file() and '.git' not in p.relative_to(self.root).parts and '__pycache__' not in p.parts}
    def record_change(self,**changes):
        leaf,record,body=self.brain.find_task(self.root,self.uuid);record.update(changes);leaf.write_text(self.brain.render_markdown_record(record,body),encoding="utf-8");return leaf

    def test_actual_writer_reservation_returns_fresh_working_before_hook_ceiling(self):
        before=self.database.read_bytes()
        with self.db() as owner:
            owner.execute('BEGIN IMMEDIATE');started=time.monotonic()
            reply=self.refresh(timeout=1.5);elapsed=time.monotonic()-started
            self.assert_degraded(reply);self.assertLess(elapsed,1.0);self.assertEqual(before,self.database.read_bytes())

    @unittest.skipUnless(BASH and shutil.which('timeout'),'actual Bash hook tools required')
    def test_actual_one_second_claude_codex_hooks_keep_same_fresh_working(self):
        with self.db() as owner:
            owner.execute('BEGIN IMMEDIATE');outputs=[]
            for host in ('.claude','.codex'):
                result=subprocess.run([BASH,str(self.root/host/'hooks/working-memory-read.sh')],input=json.dumps({'prompt':PUBLIC['query']}).encode(),cwd=self.root,env=self.env,capture_output=True,timeout=3)
                self.assertEqual(0,result.returncode);text=result.stdout.decode('utf-8');outputs.append(text)
                self.assertIn(PUBLIC['progress'],text);self.assertIn(PUBLIC['expected']['safe_warning'],text);self.assertNotIn(PUBLIC['forbidden_fact'],text)
            self.assertEqual(outputs[0],outputs[1])

    def test_explicit_readonly_is_no_sql_or_file_publication(self):
        before=self.image();reply=self.readonly();self.assert_degraded(reply);self.assertEqual(before,self.image())
        connection=self.context.connect_read_only(self.database);self.addCleanup(connection.close)
        statements=[];connection.set_trace_callback(statements.append)
        forbidden=('refresh_layers','distill_capsule_query','record_render_trace','emit_refresh_telemetry','append_refresh_health','codebase_map_status','_cursor_topic_begin')
        with mock.patch.multiple(self.context,**{name:mock.Mock(side_effect=AssertionError(name)) for name in forbidden}):
            args=self.context.build_parser().parse_args(['--root',str(self.root),'refresh','--query',PUBLIC['query'],'--task-id',self.task,'--json','--read-only'])
            with mock.patch('sys.stdout'):self.assertEqual(1,self.context.degraded_refresh(connection,self.root,args,'governed','ordinary'))
        self.assertTrue(statements);self.assertTrue(all(s.lstrip().upper().startswith(('SELECT','PRAGMA')) for s in statements),statements)

    def test_readonly_missing_database_does_not_create_parent(self):
        shutil.rmtree(self.database.parent);before=self.image();reply=self.readonly();self.assert_degraded(reply,False)
        self.assertEqual(PUBLIC['expected']['unavailable_warning'],reply['warnings'][0]);self.assertFalse(self.database.parent.exists());self.assertEqual(before,self.image())

    def test_incompatible_schema_tokenizer_and_index_are_not_migrated(self):
        original=self.database.read_bytes()
        changes=['ALTER TABLE working_tasks RENAME COLUMN auto_checkpoint TO old_checkpoint','DROP INDEX document_links_source_path','DROP TABLE episodes','ALTER TABLE task_bindings RENAME COLUMN task_uuid TO old_uuid']
        for sql in changes:
            with self.subTest(sql=sql):
                self.database.write_bytes(original)
                with self.db() as db:db.execute(sql)
                before=self.database.read_bytes();self.assert_degraded(self.readonly(),False);self.assertEqual(before,self.database.read_bytes())
        self.database.write_bytes(original)
        with self.db() as db:
            db.execute('DROP TABLE documents');db.execute("CREATE VIRTUAL TABLE documents USING fts5(path UNINDEXED,layer UNINDEXED,kind UNINDEXED,title,summary,content, tokenize='porter')")
        before=self.database.read_bytes();self.assert_degraded(self.readonly(),False);self.assertEqual(before,self.database.read_bytes())

    def test_corrupt_and_exclusively_locked_database_stay_unknown(self):
        original=self.database.read_bytes();self.database.write_bytes(b'Teal is not a SQLite image\n');before=self.database.read_bytes()
        started=time.monotonic();self.assert_degraded(self.readonly(),False);unlocked=time.monotonic()-started
        self.assertEqual(before,self.database.read_bytes());self.database.write_bytes(original)
        # Locked, a refresh adds two 0.25 s shares of the 1 s hook budget to
        # what an unlocked degraded refresh costs on this machine (Python
        # starts several times slower on Windows runners, which also stall for
        # up to a second); 1.5 s absorbs that. Ignoring the budget in either
        # wait (2 s each) still overruns.
        with self.db() as db:
            db.execute('BEGIN EXCLUSIVE');self.assert_degraded(self.refresh(timeout=unlocked+2.0),False)

    def test_current_native_progress_wins_over_old_binding_revision(self):
        self.record_change(progress=PUBLIC['updated_progress'],revision=3)
        reply=self.readonly();self.assertEqual(PUBLIC['updated_progress'],reply['capsule']['working']['progress']);self.assertEqual(3,reply['capsule']['task_revision'])

    def test_native_privacy_lifecycle_authority_and_policy_are_fresh(self):
        leaf,record,body=self.brain.find_task(self.root,self.uuid);original=leaf.read_bytes()
        terminal = {'status':'cancelled', 'transitions':record['transitions'] + [
            {'from':record['status'], 'to':'cancelled', 'at':record['updated_at'],
             'actor':'local', 'reason':'Teal public cancellation fixture'}]}
        for changes in ({'privacy':'private'}, terminal, {'authority':'inferred'}):
            with self.subTest(changes=changes):
                leaf.write_bytes(original);changed=dict(record,**changes);self.brain.validate_record(changed)
                leaf.write_text(self.brain.render_markdown_record(changed,body),encoding="utf-8");self.assert_degraded(self.readonly(),False)
        leaf.write_bytes(original);(self.root/'project-brain/config/runtime.json').write_text(json.dumps({'mode':'governed','allowed_privacy':['public']}),encoding='utf-8');self.assert_degraded(self.readonly(),False)

    def test_changed_deleted_and_newly_ignored_citations_keep_working_state(self):
        # The read-only path answers as the full refresh does: an edited or
        # ignored citation keeps the task's own state; a deleted one makes the
        # record ineligible (source-missing) on both paths.
        source=self.root/PUBLIC['source'];source.write_text(PUBLIC['changed_source'],encoding='utf-8');self.assert_degraded(self.readonly(),True)
        source.unlink();self.assert_degraded(self.readonly(),False)
        source.write_text(PUBLIC['source_text'],encoding='utf-8');(self.root/'.gitignore').write_text(PUBLIC['source']+'\n',encoding='utf-8');self.assert_degraded(self.readonly(),True)

    def test_binding_alias_mismatch_and_ambiguous_native_catalogue_are_withheld(self):
        with self.db() as db:db.execute('UPDATE task_bindings SET external_id=?',('feature/foreign-teal',))
        self.assert_degraded(self.call('refresh','--read-only','--query',PUBLIC['query'],'--task-id','feature/foreign-teal','--json',expected=1),False)
        with self.db() as db:db.execute('UPDATE task_bindings SET external_id=?',(self.task,))
        leaf,_,_=self.brain.find_task(self.root,self.uuid);shutil.copy2(leaf,leaf.with_name('ambiguous-teal.md'));self.assert_degraded(self.readonly(),False)

    def test_private_source_and_unsafe_progress_or_configuration_are_withheld(self):
        leaf,record,body=self.brain.find_task(self.root,self.uuid);original=leaf.read_bytes()
        (self.root/'credentials.json').write_text('{}',encoding='utf-8');fp=self.brain.fingerprint(self.root,'credentials.json')
        leaf.write_text(self.brain.render_markdown_record(dict(record,sources=['credentials.json'],source_fingerprints=[fp]),body),encoding='utf-8');self.assert_degraded(self.readonly(),False)
        leaf.write_bytes(original);self.record_change(progress='password=teal-secret-value');self.assert_degraded(self.readonly(),False)
        leaf.write_bytes(original);(self.root/'project-brain/config/runtime.json').write_text('{not json}',encoding='utf-8')
        result=subprocess.run([sys.executable,'-B',str(self.cli),'--root',str(self.root),'refresh','--read-only','--query',PUBLIC['query'],'--task-id',self.task,'--json'],env=self.env,capture_output=True,timeout=2)
        self.assertEqual(1,result.returncode);self.assertNotIn(PUBLIC['progress'].encode(),result.stdout);self.assertNotIn(b'Traceback',result.stderr)

    def test_readonly_unknown_binding_is_not_warming_or_absence(self):
        with self.db() as db:db.execute('DELETE FROM task_bindings')
        reply=self.readonly();self.assert_degraded(reply,False);self.assertNotIn('not recorded yet',reply.get('capsule_text',''));self.assertNotIn('pending_turns',json.dumps(reply))

    def test_lightweight_committed_working_row_has_no_cached_retrieval(self):
        with self.db() as db:db.execute('UPDATE working_tasks SET goal=?, progress=? WHERE task_id=?',(PUBLIC['goal'],PUBLIC['progress'],self.task))
        reply=self.call('--mode','lightweight','refresh','--query',PUBLIC['query'],'--task-id',self.task,'--read-only','--json',expected=1)
        self.assertEqual(PUBLIC['progress'],reply['capsule']['working']['progress']);self.assertEqual([],reply['capsule']['semantic']);self.assertNotIn(PUBLIC['forbidden_fact'],reply['capsule_text'])

    def test_uuid_request_uses_canonical_alias_and_native_collision_is_withheld(self):
        reply=self.call('refresh','--read-only','--query',PUBLIC['query'],'--task-id',self.uuid,'--json',expected=1)
        self.assertEqual(self.task,reply['capsule']['task_id']);self.assertEqual(self.task,reply['capsule']['working']['task_id'])
        leaf,record,body=self.brain.find_task(self.root,self.uuid)
        second=dict(record,id='5e044f42-dde0-4259-a6f7-0dad98491c31',external_id=self.uuid)
        self.brain.validate_record(second)
        leaf.with_name('teal-collision.md').write_text(self.brain.render_markdown_record(second,body),encoding='utf-8')
        self.assert_degraded(self.call('refresh','--read-only','--query',PUBLIC['query'],'--task-id',self.uuid,'--json',expected=1),False)

    def test_missing_attached_state_reports_unknown_without_creation(self):
        parent=tempfile.TemporaryDirectory(prefix='teal-state-parent-');self.addCleanup(parent.cleanup)
        missing=Path(parent.name)/'absent-teal-state'
        env=dict(self.env,ACCELERATOR_HOME=str(self.root),ACCELERATOR_STATE_DIR=str(missing),ACCELERATOR_PROJECT_DIR=str(self.root))
        result=subprocess.run([sys.executable,'-B',str(self.cli),'--root',str(missing),'refresh','--read-only','--query',PUBLIC['query'],'--task-id',self.task,'--json'],env=env,cwd=self.root,capture_output=True,timeout=2)
        self.assertEqual(1,result.returncode,result.stderr.decode());self.assert_degraded(json.loads(result.stdout),False);self.assertFalse(missing.exists())

    def test_wal_profile_refused_before_sidecar_creation(self):
        db=sqlite3.connect(self.database)
        try:self.assertEqual('wal',db.execute('PRAGMA journal_mode=WAL').fetchone()[0])
        finally:db.close()
        before=self.image();self.assert_degraded(self.readonly(),False);self.assertEqual(before,self.image())

    @unittest.skipUnless(os.name=='posix' and getattr(os,'geteuid',lambda:0)()!=0,'non-root POSIX permission checks required')
    def test_actual_readonly_local_directory_retains_fresh_working(self):
        before=self.image();self.database.chmod(0o444);self.database.parent.chmod(0o555)
        try:
            self.assert_degraded(self.refresh())
            self.assertEqual(before,self.image())
            self.assertEqual(0o444,self.database.stat().st_mode & 0o777)
            self.assertEqual(0o555,self.database.parent.stat().st_mode & 0o777)
        finally:
            self.database.parent.chmod(0o755);self.database.chmod(0o644)

    def test_ordinary_connect_preserves_timeout_and_writable_migration(self):
        with self.db() as db:db.execute('ALTER TABLE working_tasks DROP COLUMN auto_checkpoint')
        native=sqlite3.connect
        with mock.patch.object(self.context.sqlite3,'connect',wraps=native) as observer:
            connection=self.context.connect(self.database)
            try:
                self.assertEqual(5.0,observer.call_args.kwargs['timeout'])
                self.assertIn('auto_checkpoint',[row['name'] for row in connection.execute('PRAGMA table_info(working_tasks)')])
            finally:connection.close()
        healthy=self.refresh(expected=0)
        self.assertIn(PUBLIC['forbidden_fact'],healthy['capsule_text'])

    def test_legitimate_working_checkpoint_migration_order_is_compatible(self):
        with self.db() as db:
            rows=db.execute('SELECT task_id,goal,progress,next_steps,files,sources,created_at,updated_at,auto_checkpoint FROM working_tasks').fetchall()
            db.execute('DROP TABLE working_tasks')
            db.execute("CREATE TABLE working_tasks(task_id TEXT PRIMARY KEY,goal TEXT NOT NULL,progress TEXT NOT NULL,next_steps TEXT NOT NULL,files TEXT NOT NULL,sources TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL)")
            db.execute("ALTER TABLE working_tasks ADD COLUMN auto_checkpoint TEXT NOT NULL DEFAULT ''")
            db.executemany('INSERT INTO working_tasks VALUES(?,?,?,?,?,?,?,?,?)',rows)
        before=self.image();self.assert_degraded(self.readonly());self.assertEqual(before,self.image())

    def test_tokenizer_comment_spoof_and_lightweight_case_alias_are_refused(self):
        original=self.database.read_bytes()
        with self.db() as db:
            db.execute('DROP TABLE documents')
            db.execute("CREATE VIRTUAL TABLE documents USING fts5(path UNINDEXED,layer UNINDEXED,kind UNINDEXED,title,summary,content, tokenize='porter' /* "+self.context.TOKENIZER+" */)")
        self.assert_degraded(self.readonly(),False)
        self.database.write_bytes(original)
        with self.db() as db:
            rows=db.execute('SELECT * FROM working_tasks').fetchall()
            db.execute('DROP TABLE working_tasks')
            db.execute("CREATE TABLE working_tasks(task_id TEXT PRIMARY KEY COLLATE NOCASE,goal TEXT NOT NULL,progress TEXT NOT NULL,auto_checkpoint TEXT NOT NULL DEFAULT '',next_steps TEXT NOT NULL,files TEXT NOT NULL,sources TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL)")
            db.executemany('INSERT INTO working_tasks VALUES(?,?,?,?,?,?,?,?,?)',rows)
        reply=self.call('--mode','lightweight','refresh','--read-only','--query',PUBLIC['query'],'--task-id',self.task.upper(),'--json',expected=1)
        self.assert_degraded(reply,False)

    @unittest.skipUnless(BASH and shutil.which('timeout'),'actual Bash hook tools required')
    def test_actual_cursor_degraded_rule_replaces_old_facts_and_preserves_custom(self):
        rules=self.root/'.cursor/rules';rules.mkdir(parents=True)
        managed=subprocess.run([sys.executable,'-B',str(self.cli),'--root',str(self.root),'cursor-rule','--action','write','--task-id',self.task],input=('working: '+PUBLIC['goal']+'\nmemory (reference data — verify against canonical sources)\n'+PUBLIC['forbidden_fact']).encode(),env=self.env,capture_output=True,timeout=3)
        self.assertEqual(0,managed.returncode);rule=rules/'working-memory.mdc';self.assertIn(PUBLIC['forbidden_fact'],rule.read_text(encoding='utf-8'))
        with self.db() as db:
            db.execute('BEGIN IMMEDIATE');hook=subprocess.run([BASH,str(self.root/'.cursor/hooks/working-memory-read.sh')],input=json.dumps({'prompt':PUBLIC['query']}).encode(),env=self.env,cwd=self.root,capture_output=True,timeout=5)
        self.assertEqual(0,hook.returncode);text=rule.read_text(encoding='utf-8');self.assertNotIn(PUBLIC['forbidden_fact'],text);self.assertIn(PUBLIC['progress'],text)
        rule.write_text('Human Teal rule must survive.\n',encoding='utf-8')
        with self.db() as db:
            db.execute('BEGIN IMMEDIATE');subprocess.run([BASH,str(self.root/'.cursor/hooks/working-memory-read.sh')],input=json.dumps({'prompt':PUBLIC['query']}).encode(),env=self.env,cwd=self.root,capture_output=True,timeout=5)
        self.assertEqual('Human Teal rule must survive.\n',rule.read_text(encoding='utf-8'))

if __name__=='__main__':unittest.main()
