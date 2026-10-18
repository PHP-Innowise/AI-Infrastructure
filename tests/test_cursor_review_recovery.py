"""Fresh registered-slot recovery controls, portable except explicit POSIX primitives."""
from __future__ import annotations
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
PUBLIC = json.loads((ROOT/'tests/fixtures/cursor_review_recovery_public.json').read_text(encoding='utf-8'))


@unittest.skipUnless(shutil.which('git'), 'Git required for the actual selected index')
class CursorReviewRecoveryTests(unittest.TestCase):
    def setUp(self):
        clean = {k:v for k,v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        clean['GIT_OPTIONAL_LOCKS'] = '0'
        environment = mock.patch.dict(os.environ, clean, clear=True); environment.start(); self.addCleanup(environment.stop)
        sys.path.insert(0, str(ROOT/'PHP Core/memory-bank/scripts'))
        try: self.runtime = importlib.import_module('context_retrieval')
        finally: sys.path.pop(0)
        temp = tempfile.TemporaryDirectory(prefix='kite-cursor-recovery-'); self.addCleanup(temp.cleanup)
        self.base = Path(temp.name).resolve(); self.root = self.base/'original'; self.root.mkdir()
        subprocess.run(['git','init','-q',str(self.root)],check=True,capture_output=True)
        (self.root/'.gitignore').write_text(PUBLIC['ignore_entry'],encoding='utf-8')
        self.layout = mock.patch.object(self.runtime.workspace_roots, 'TOOLING_ROOT', self.root)
        self.layout.start(); self.addCleanup(self.layout.stop)

    @property
    def rule(self): return self.root/'.cursor/rules/working-memory.mdc'
    @property
    def receipt(self): return self.root/'memory-bank/local/cursor-rule.json'
    def call(self, action='write', task=None, capsule=None, **kw):
        return self.runtime.cursor_rule(self.root, action, task=task or PUBLIC['task'],
            capsule=capsule if capsule is not None else PUBLIC['capsule'], **kw)
    def legacy(self, body=None):
        self.rule.parent.mkdir(parents=True,exist_ok=True)
        self.rule.write_bytes((PUBLIC['legacy_prefix']+(body or PUBLIC['capsule']).rstrip('\n')+PUBLIC['legacy_suffix']).encode('utf-8'))
    def fresh(self):
        self.assertEqual('written', self.call()['state'])
        return self.rule.read_bytes(), self.rule.stat().st_ino

    def test_full_registered_legacy_envelope_upgrades_without_manual_step(self):
        self.legacy()
        self.assertEqual('written', self.call()['state'])
        self.assertTrue(self.receipt.exists())
        self.assertEqual('removed', self.call('keep',task=PUBLIC['other_task'])['state'])
        self.assertFalse(self.rule.exists())

    def test_stale_old_branch_writer_recovers_to_current_task(self):
        self.fresh(); self.rule.unlink(); self.legacy()
        reply = self.call(task=PUBLIC['other_task'],capsule=PUBLIC['other_capsule'])
        self.assertEqual('written', reply['state'])
        self.assertIn(PUBLIC['other_task'].encode(), self.rule.read_bytes())
        self.assertNotIn(PUBLIC['capsule'].encode().strip(), self.rule.read_bytes())

    def test_touch_and_stale_device_fields_reconcile_bytes_not_physical_history(self):
        raw, inode = self.fresh(); os.utime(self.rule, None)
        note=json.loads(self.receipt.read_bytes()); note['root_identity'][0]+=11; note['rules_identity'][0]+=11
        note['file_identity'][0]+=11; note['file_identity'][3]+=13
        self.receipt.write_text(json.dumps(note),encoding='utf-8')
        self.assertEqual('unchanged',self.call()['state'])
        self.assertEqual(raw,self.rule.read_bytes());self.assertEqual(inode,self.rule.stat().st_ino)

    def test_copy_with_exact_rule_and_receipt_reconciles_at_current_root(self):
        raw,_ = self.fresh(); destination=self.base/'copy';shutil.copytree(self.root,destination)
        self.root=destination;self.layout.stop();self.layout=mock.patch.object(self.runtime.workspace_roots,'TOOLING_ROOT',self.root)
        self.layout.start();self.addCleanup(self.layout.stop)
        self.assertEqual('unchanged',self.call()['state']);self.assertEqual(raw,self.rule.read_bytes())
        self.assertEqual(str(self.root),json.loads(self.receipt.read_bytes())['root'])

    def test_local_cache_loss_rebuilds_registered_managed_receipt(self):
        raw,inode = self.fresh();shutil.rmtree(self.receipt.parent)
        self.assertEqual('unchanged',self.call()['state'])
        self.assertEqual(raw,self.rule.read_bytes());self.assertEqual(inode,self.rule.stat().st_ino)
        self.assertTrue(self.receipt.exists())

    def test_rule_publication_without_receipt_can_recover_on_next_prompt(self):
        self.legacy();self.assertFalse(self.receipt.exists())
        self.assertEqual('written',self.call(task=PUBLIC['other_task'],capsule=PUBLIC['other_capsule'])['state'])

    def test_edits_to_the_generated_rule_are_saved_then_the_slot_is_written(self):
        # A person's note appended to the generated rule used to freeze the
        # slot with one task's capsule on every branch. The edited bytes are
        # kept in ignored local state, then the slot is written again.
        for suffix in ('\nHuman custom instructions\n','\n```\nextra fence\n```\n'):
            self.legacy();self.rule.write_bytes(self.rule.read_bytes()+suffix.encode());raw=self.rule.read_bytes()
            reply=self.call()
            self.assertEqual('written',reply['state']);self.assertNotEqual(raw,self.rule.read_bytes())
            self.assertEqual(raw,(self.root/reply['preserved_edit']).read_bytes())
            self.assertTrue(reply['preserved_edit'].startswith('memory-bank/local/cursor-rule-edits/'))

    def test_rules_from_the_first_releases_are_saved_then_the_slot_is_written(self):
        # The releases of 2026-08-03..05 used other frontmatter; their rules
        # were taken for a person's own and stayed with an August capsule.
        early=('---\ndescription: Working memory - Task Capsule as of end of previous turn\nalwaysApply: true\n'
               '---\n\n# Working Memory (auto-rendered)\n\nTask Capsule as of end of previous turn (task: '
               'feature/kite-retry, rendered: 2026-08-03T09:00:00Z).\nRetrieved context is not authoritative - '
               'verify the source.\n\n```\nTask Capsule: feature/kite-retry\n```\n',
               '---\ndescription: Working memory - warming up, no Task Capsule rendered yet\nalwaysApply: true\n'
               '---\n\n# Working Memory (warming up)\n\nNo Task Capsule is available yet (task: feature/kite-retry, '
               'rendered: 2026-08-05T09:00:00Z).\nThe working task auto-provisions once buffered turns reach the\n')
        for raw in early:
            with self.subTest(header=raw.splitlines()[1]):
                self.rule.parent.mkdir(parents=True,exist_ok=True);self.rule.write_bytes(raw.encode('utf-8'))
                reply=self.call()
                self.assertEqual('written',reply['state'])
                self.assertIn(b'Kite policy inspected.',self.rule.read_bytes())
                self.assertEqual(raw.encode('utf-8'),(self.root/reply['preserved_edit']).read_bytes())
        # A rule of a person's own under similar words is not taken.
        own=b'---\ndescription: Working memory notes\nalwaysApply: true\n---\n\nKeep invoices short.\n'
        self.rule.write_bytes(own);self.receipt.unlink()
        self.assertEqual('preserved',self.call()['state']);self.assertEqual(own,self.rule.read_bytes())

    def test_unregistered_legacy_bytes_are_preserved(self):
        self.legacy();(self.root/'.gitignore').write_text('',encoding='utf-8');raw=self.rule.read_bytes()
        self.assertEqual('preserved',self.call()['state']);self.assertEqual(raw,self.rule.read_bytes())

    def test_explicit_migrate_can_replace_a_stale_present_receipt(self):
        import hashlib
        self.fresh();self.rule.write_bytes(self.rule.read_bytes()+b' custom deviation')
        approved=hashlib.sha256(self.rule.read_bytes()).hexdigest()
        self.assertEqual('written',self.call('migrate',approved_sha256=approved)['state'])

    def test_protected_exact_trust_is_projected_after_ownership_refusal(self):
        from types import SimpleNamespace
        calls=[]
        def probe(argv, **options):
            calls.append((argv,options['env']))
            return SimpleNamespace(returncode=1 if 'safe.directory='+str(self.root.resolve()) in argv else 128)
        with mock.patch.object(self.runtime,'_cursor_protected_safe_root',return_value=True,create=True), \
                mock.patch.object(self.runtime.subprocess,'run',side_effect=probe):
            self.assertTrue(self.runtime._cursor_git_untracked(self.root))
        self.assertEqual(3,len(calls))
        self.assertTrue(all(env['GIT_CONFIG_GLOBAL']==os.devnull for _,env in calls))
        self.assertFalse(any('safe.directory=*' in arg for argv,_ in calls for arg in argv))

    def test_untrusted_root_never_gets_a_command_trust_exception(self):
        from types import SimpleNamespace
        with mock.patch.object(self.runtime,'_cursor_protected_safe_root',return_value=False,create=True), \
                mock.patch.object(self.runtime.subprocess,'run',return_value=SimpleNamespace(returncode=128)) as probe:
            self.assertFalse(self.runtime._cursor_git_untracked(self.root))
        self.assertFalse(any('safe.directory=' in arg for arg in probe.call_args.args[0]))

    def test_protected_system_global_reset_and_exact_path_semantics(self):
        # One listing of every file Git reads, each value with its scope.
        listing=[]
        def read(argv, environment):
            self.assertIn('--show-scope',argv)
            return 0,b''.join(scope.encode()+b'\0'+value.encode()+b'\0' for scope,value in listing)
        with mock.patch.object(self.runtime,'_cursor_git_config_read',side_effect=read,create=True):
            listing[:]=[('system',str(self.root)),('global','')]
            self.assertFalse(self.runtime._cursor_protected_safe_root(self.root))
            listing.append(('global',str(self.root)))
            self.assertTrue(self.runtime._cursor_protected_safe_root(self.root))
            listing[:]=[('system',''),('global',str(self.base/'unrelated'))]
            self.assertFalse(self.runtime._cursor_protected_safe_root(self.root))
            listing[:]=[('global','*')]
            self.assertTrue(self.runtime._cursor_protected_safe_root(self.root))
            # A repository's own config cannot trust itself.
            listing[:]=[('local',str(self.root))]
            self.assertFalse(self.runtime._cursor_protected_safe_root(self.root))

    @unittest.skipUnless(sys.platform.startswith('linux'), 'actual Linux no-clobber rename')
    def test_initial_publication_does_not_require_hard_links(self):
        with mock.patch.object(self.runtime.os,'link',side_effect=OSError('hard links unavailable')):
            self.assertEqual('written',self.call()['state'])
        self.assertTrue(self.receipt.exists())

    @unittest.skipUnless(os.name == 'posix', 'POSIX publication window')
    def test_owned_old_rule_publication_pair_recovers(self):
        self.legacy()
        alias=self.rule.parent/'.cursor-rule-5ca77589-16b7-4fd7-9938-8c8d966455dd.tmp'
        os.link(self.rule,alias)
        self.assertEqual('written',self.call(task=PUBLIC['other_task'],capsule=PUBLIC['other_capsule'])['state'])
        self.assertFalse(alias.exists());self.assertEqual(1,self.rule.stat().st_nlink)
        self.assertIn(PUBLIC['other_task'].encode(),self.rule.read_bytes())

    @unittest.skipUnless(os.name == 'posix', 'POSIX publication window')
    def test_owned_old_receipt_publication_pair_recovers(self):
        raw,inode=self.fresh()
        alias=self.receipt.parent/'.cursor-rule-5ca77589-16b7-4fd7-9938-8c8d966455dd.tmp'
        os.link(self.receipt,alias)
        self.assertEqual('unchanged',self.call()['state'])
        self.assertFalse(alias.exists());self.assertEqual(raw,self.rule.read_bytes())

    @unittest.skipUnless(os.name == 'posix', 'POSIX publication window')
    def test_foreign_unregistered_and_ambiguous_links_are_preserved(self):
        self.legacy();original=self.rule.read_bytes()
        foreign=self.base/'outside-owned-slot.md';os.link(self.rule,foreign)
        self.assertEqual('preserved',self.call()['state']);self.assertEqual(original,foreign.read_bytes())
        foreign.unlink()
        alias=self.rule.parent/'.cursor-rule-5ca77589-16b7-4fd7-9938-8c8d966455dd.tmp'
        os.link(self.rule,alias);(self.root/'.gitignore').write_text('',encoding='utf-8')
        self.assertEqual('preserved',self.call()['state']);self.assertTrue(alias.exists())
        (self.root/'.gitignore').write_text(PUBLIC['ignore_entry'],encoding='utf-8');os.link(self.rule,foreign)
        self.assertEqual('preserved',self.call()['state']);self.assertTrue(alias.exists())
        self.assertEqual(original,self.rule.read_bytes())

    @unittest.skipUnless(os.name == 'posix', 'POSIX descriptor publication')
    def test_appearing_target_never_gets_clobbered(self):
        self.rule.parent.mkdir(parents=True)
        descriptor=os.open(self.rule.parent,os.O_RDONLY|os.O_DIRECTORY)
        self.addCleanup(os.close,descriptor)
        original=self.runtime._cursor_publish_absent
        def appears(parent,temporary,name):
            self.rule.write_bytes(b'human-owned bytes')
            return original(parent,temporary,name)
        with mock.patch.object(self.runtime,'_cursor_publish_absent',side_effect=appears):
            with self.assertRaises(OSError):
                self.runtime._cursor_publish(descriptor,self.rule.name,b'new capsule',None,16384)
        self.assertEqual(b'human-owned bytes',self.rule.read_bytes())

    @unittest.skipUnless(os.name == 'posix', 'POSIX publication window')
    def test_pair_cleanup_detects_equal_byte_foreign_replacement(self):
        self.legacy();raw=self.rule.read_bytes()
        alias=self.rule.parent/'.cursor-rule-5ca77589-16b7-4fd7-9938-8c8d966455dd.tmp'
        os.link(self.rule,alias);original=self.runtime._cursor_image;swapped=[]
        def image(parent,name,limit,**options):
            if name==self.rule.name and not alias.exists() and not swapped:
                temporary=self.rule.with_name('foreign-replacement')
                temporary.write_bytes(raw);temporary.replace(self.rule);swapped.append(True)
            return original(parent,name,limit,**options)
        with mock.patch.object(self.runtime,'_cursor_image',side_effect=image):
            self.assertEqual('incomplete',self.call()['state'])
        self.assertEqual([True],swapped);self.assertEqual(raw,self.rule.read_bytes())
        self.assertFalse(self.receipt.exists())

    @unittest.skipUnless(os.name == 'posix', 'POSIX publication window')
    def test_tracked_temporary_alias_and_custom_pair_are_preserved(self):
        self.legacy();alias=self.rule.parent/'.cursor-rule-5ca77589-16b7-4fd7-9938-8c8d966455dd.tmp'
        os.link(self.rule,alias)
        subprocess.run(['git','-C',str(self.root),'add','--',str(alias.relative_to(self.root))],
                       check=True,capture_output=True)
        raw=self.rule.read_bytes();self.assertEqual('preserved',self.call()['state'])
        self.assertTrue(alias.exists());self.assertEqual(raw,self.rule.read_bytes())
        subprocess.run(['git','-C',str(self.root),'rm','--cached','--',str(alias.relative_to(self.root))],
                       check=True,capture_output=True)
        self.rule.write_bytes(raw+b' custom deviation')
        self.assertEqual('preserved',self.call()['state']);self.assertTrue(alias.exists())
        self.assertEqual(raw+b' custom deviation',alias.read_bytes())

    @unittest.skipUnless(sys.platform.startswith('linux'), 'Linux renameat2 profile')
    def test_no_clobber_errors_do_not_retry_as_link(self):
        import ctypes
        import errno
        from types import SimpleNamespace
        for failure in (errno.ENOSYS,errno.EEXIST,errno.EPERM,errno.EIO,errno.EINTR):
            def rename(*unused):ctypes.set_errno(failure);return -1
            with self.subTest(failure=failure),mock.patch.object(ctypes,'CDLL',return_value=SimpleNamespace(renameat2=rename)), \
                    mock.patch.object(self.runtime.os,'link') as link:
                if failure==errno.ENOSYS:
                    self.runtime._cursor_publish_absent(0,'owned.tmp','owned.rule');link.assert_called_once()
                else:
                    with self.assertRaises(OSError) as caught:self.runtime._cursor_publish_absent(0,'owned.tmp','owned.rule')
                    self.assertEqual(failure,caught.exception.errno);link.assert_not_called()

    def test_git_reader_thread_setup_failure_reaps_and_closes(self):
        import threading
        original=self.runtime.subprocess.Popen;processes=[]
        def launch(*args,**options):
            process=original(*args,**options);processes.append(process);return process
        with mock.patch.object(self.runtime.subprocess,'Popen',side_effect=launch), \
                mock.patch.object(threading.Thread,'start',side_effect=RuntimeError('owned setup failure')):
            self.assertIsNone(self.runtime._cursor_git_config_read(['git','--version'],dict(os.environ)))
        self.assertEqual(1,len(processes));self.assertIsNotNone(processes[0].poll())
        self.assertTrue(processes[0].stdout.closed)

    def test_real_git_missing_global_config_is_an_empty_observation(self):
        environment={k:v for k,v in os.environ.items() if not k.startswith('GIT_')}
        environment.update(HOME=str(self.base/'unused-home'),XDG_CONFIG_HOME=str(self.base/'unused-xdg'),GIT_OPTIONAL_LOCKS='0')
        self.assertEqual((1,b''),self.runtime._cursor_git_config_read(
            ['git','config','--global','--includes','--null','--get-all','safe.directory'],environment))

    def test_unresolvable_protected_path_is_unavailable(self):
        original=Path.resolve
        def resolve(leaf,*args,**options):
            if str(leaf)=='/unresolvable-heron-loop':raise RuntimeError('owned old pathlib loop')
            return original(leaf,*args,**options)
        with mock.patch.object(self.runtime,'_cursor_git_config_read',return_value=(0,b'global\0/unresolvable-heron-loop\0')), \
                mock.patch.object(Path,'resolve',resolve):
            self.assertFalse(self.runtime._cursor_protected_safe_root(self.root))

    @unittest.skipUnless(os.name=='posix','private shell wrapper for real Git ownership policy')
    def test_actual_git_honors_global_trust_and_refuses_local_self_trust(self):
        import shlex
        binary=shutil.which('git');home=self.base/'private-profile';home.mkdir()
        tools=self.base/'git-wrapper';tools.mkdir()
        wrapper=tools/'git'
        wrapper.write_text('#!/bin/sh\nGIT_TEST_ASSUME_DIFFERENT_OWNER=1 exec '+shlex.quote(binary)+' "$@"\n',encoding='utf-8')
        wrapper.chmod(0o755)
        profile=home/'.gitconfig'
        profile.write_text('[safe]\n\tdirectory =\n\tdirectory = '+str(self.root)+'\n',encoding='utf-8')
        with mock.patch.dict(os.environ,{'HOME':str(home),'XDG_CONFIG_HOME':str(home/'xdg'),
                'PATH':str(tools)+os.pathsep+os.environ.get('PATH','')}):
            self.assertTrue(self.runtime._cursor_git_untracked(self.root))
            profile.write_text('[safe]\n\tdirectory =\n',encoding='utf-8')
            with (self.root/'.git/config').open('a',encoding='utf-8') as stream:
                stream.write('\n[safe]\n\tdirectory = '+str(self.root)+'\n')
            self.assertFalse(self.runtime._cursor_git_untracked(self.root))


    @unittest.skipUnless(os.name=='posix','private shell wrapper for real Git ownership policy')
    def test_actual_git_trust_from_the_xdg_file_beside_a_home_gitconfig(self):
        # `git config --global` reads only ~/.gitconfig when it exists; Git
        # itself also honours $XDG_CONFIG_HOME/git/config.
        import shlex
        binary=shutil.which('git');home=self.base/'xdg-profile';home.mkdir()
        tools=self.base/'xdg-git-wrapper';tools.mkdir()
        wrapper=tools/'git'
        wrapper.write_text('#!/bin/sh\nGIT_TEST_ASSUME_DIFFERENT_OWNER=1 exec '+shlex.quote(binary)+' "$@"\n',encoding='utf-8')
        wrapper.chmod(0o755)
        (home/'.gitconfig').write_text('[user]\n\tname = Heron\n',encoding='utf-8')
        xdg=home/'xdg'/'git';xdg.mkdir(parents=True)
        (xdg/'config').write_text('[safe]\n\tdirectory = '+str(self.root)+'\n',encoding='utf-8')
        with mock.patch.dict(os.environ,{'HOME':str(home),'XDG_CONFIG_HOME':str(home/'xdg'),
                'PATH':str(tools)+os.pathsep+os.environ.get('PATH','')}):
            self.assertTrue(self.runtime._cursor_protected_safe_root(self.root))
            self.assertTrue(self.runtime._cursor_git_untracked(self.root))

if __name__=='__main__':unittest.main()
