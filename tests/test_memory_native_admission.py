"""Native snapshots: real byte admission, complete trees and no model on failure."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import memory_native_ab_admission as admission
import memory_native_ab_io as io


class ContentAdmissionTests(unittest.TestCase):
    def check(self, text):
        return admission.admit_image([('config/public.yaml', text.encode(), 0o644)])

    def test_real_defaults_and_structured_credential_forms_are_not_placeholders(self):
        cases = ['DB_PASSWORD=root', 'password: 0', 'password: null', 'password: example',
                 '{"password": "example"}', "['passphrase' => 'unit-test-value']",
                 'clientSecret = "synthetic-private-value"', 'passwordHash: "synthetic-hash"',
                 '[DEFAULT]\npassword = synthetic-only\n',
                 '[DEFAULT]\npassword = synthetic-only\n[defaults]\npassword = "[REDACTED]"\n',
                 'REGISTRY_CREDENTIALS: opaque-value', 'registry_credentials:\n  nested: value',
                 'MEILI_MASTER_KEY: synthetic-only', 'redis://:root@localhost/0',
                 'envs:\n- key: APP_SECRET\n  type: SECRET\n  value: "synthetic-only"',
                 '{"envs":[{"key":"APP_SECRET","type":"SECRET","value":"synthetic-only"}]}',
                 'registry_credentials: |\n  value', 'password:\n  "literal"',
                 'envs:\n- key: APP_SECRET\n  value: EV[synthetic-only]',
                 'EV[\nsynthetic-only\n]', 'EV["DB_PASSWORD"] = "synthetic-only"',
                 'mysql://root:root@localhost/db',
                 '-----BEGIN PRIVATE KEY-----\nSYNTHETIC-ONLY\n-----END PRIVATE KEY-----']
        for text in cases:
            with self.subTest(form=text.splitlines()[0]):
                with self.assertRaises(ValueError): self.check(text)

    def test_each_same_line_assignment_is_checked(self):
        for text in ['{"password":"[REDACTED]", "apiKey":"synthetic-only"}',
                     'password="[REDACTED]"; DB_PASSWORD="synthetic-only"']:
            with self.subTest(text=text):
                with self.assertRaises(ValueError):self.check(text)
        for text in [json.dumps({'message':'DB_PASSWORD=synthetic-only'}),
                     '{"p\\u0061ssword":"synthetic-only"}',
                     '\ufeff{"p\\u0061ssword":"synthetic-only"}',
                     '{"p\\u0061ssword":"one","p\\u0061ssword":"two"}',
                     '{"p\\u0061ssword":"synthetic-only",}',
                     '{"message":"password=synthetic-only",}',
                     '[{"p\\u0061ssword":"synthetic-only",}]']:
            with self.assertRaises(ValueError):self.check(text)

    def test_subscript_assignments_and_structured_headers_are_screened(self):
        for text in ["$config['password'] = 'synthetic-only';",
                     'settings["password"] = "synthetic-only"',
                     "$config['nested']['api_key'] = 'synthetic-only';",
                     'settings["password" ] = "synthetic-only"',
                     '{"headers":{"Authorization":"Basic dXNlcjpwYXNzMTIz"}}',
                     '{"headers":{"Authorization":"Bearer alphabetsynthetic"}}',
                     '{"Proxy-Authorization":"Basic dXNlcjpwYXNzMTIz"}',
                     'Authorization: Bearer alphabetsynthetic']:
            with self.subTest(form=text),self.assertRaises(ValueError):self.check(text)
        for text in ['{"authorization":{"roles":["reader"]}}',
                     'settings["public_title"] = "documentation"']:
            self.assertEqual(1,self.check(text)['files'])

    def test_multiline_subscripts_and_normalized_header_values_are_screened(self):
        for text in ['settings["password"\n] = "synthetic-only"',
                     "$config['password'\r\n] \n= 'synthetic-only';",
                     'settings["password"' + '\n' * 2048 + '] = "synthetic-only"',
                     *[json.dumps({key: prefix + scheme + ' alphabetsynthetic'})
                       for key in ('Authorization', 'Proxy-Authorization')
                       for prefix in (' ', '\t', '\r\n', '\r\n\t')
                       for scheme in ('Basic', 'Bearer')],
                     *[json.dumps({key: form})
                       for key in ('Authorization', 'Proxy-Authorization')
                       for prefix in ('', ' ', '\r\n\t')
                       for form in ([prefix + 'Bearer alphabetsynthetic'], [[prefix + 'Bearer alphabetsynthetic']])]]:
            with self.subTest(form=text[:80]),self.assertRaises(ValueError):self.check(text)
        for value in ({'roles':['reader']}, ['public-role'], '', ' \r\n\t'):
            self.assertEqual(1,self.check(json.dumps({'Authorization':value}))['files'])

    def test_exact_markers_and_whole_references_are_supported(self):
        for text in ['password: "[REDACTED_PRIVATE_VALUE]"', 'password: [REDACTED]',
                     'DB_PASSWORD="${DB_PASSWORD}"', 'private_key: "%env(resolve:JWT_KEY)%"',
                     '{"apiKey":"[REDACTED]", "password":"[REDACTED]"}',
                     'password: "[REDACTED]"\nfoo: bar',
                     '[www]\nmode = dynamic\n',
                     '[www]\npassword = "[REDACTED]"\n',
                     'private_key: "%env(JWT_KEY)%"\nfoo: bar',
                     'envs:\n- key: APP_SECRET\n  type: SECRET\n  value: "[REDACTED]"\nnext: public']:
            with self.subTest(text=text):self.assertEqual(1, self.check(text)['files'])
        for text in ['password: "[REDACTED]suffix"', 'password: ${MISSING:-literal}',
                     'password: ${DB_PASSWORD}\n  literal-continuation',
                     'password: [REDACTED] extra',
                     *['password: '+value+suffix for value in ('${DB_PASSWORD}','$DB_PASSWORD','[REDACTED]')
                       for suffix in ('#suffix',';suffix',',suffix',' '*1300+'suffix')]]:
            with self.subTest(text=text):
                with self.assertRaises(ValueError):self.check(text)

    def test_invalid_encoding_controls_and_duplicate_paths_fail_closed(self):
        for raw in (b'\xff', b'public\0private', b'archive\x01data', b'a\x7fb'):
            with self.subTest(raw=raw):
                with self.assertRaises(ValueError):admission.admit_image([('public.bin',raw,0o644)])
        for image in [[('a.md', b'a', 0o644)] * 2, [('../a', b'a',0o644)],
                      [('.env.public', b'public',0o644)], [('a.md',b'a',True)]]:
            with self.assertRaises(ValueError):admission.admit_image(image)

    def test_reports_and_errors_do_not_contain_secret_or_filename_values(self):
        sentinel='SYNTHETIC-CANARY-47391'
        with self.assertRaises(ValueError) as caught:self.check('password: '+sentinel)
        self.assertNotIn(sentinel,str(caught.exception))
        report=self.check('Public text without credentials.')
        self.assertNotIn('Public text',json.dumps(report))
        self.assertNotIn('config/public.yaml',json.dumps(report))

    def test_canonical_exemption_requires_exact_bytes_and_mode(self):
        name='memory-bank/scripts/automatic_query.py';raw,mode=admission._canonical()[name]
        result=admission.admit_image([(name,raw,mode)])
        self.assertEqual(1,result['exact_canonical_files'])
        for changed in [(raw+b'\n# changed\n',mode),(raw,mode^0o100)]:
            with self.assertRaises(ValueError):admission.admit_image([(name,*changed)])
        with self.assertRaises(ValueError):
            admission.admit_image([('memory-bank/scripts/extra.py',b'password="synthetic-only"',0o644)])

    def test_long_benign_prefix_is_bounded_and_expired_deadline_fails(self):
        code='''import sys,time
sys.path.insert(0,sys.argv[1])
from memory_native_ab_admission import _text
raw=('a_'*65536).encode()
_text(raw)
try:_text(raw,deadline=time.monotonic()-1)
except ValueError:print('bounded-and-expired')
else:raise SystemExit(9)
'''
        result=subprocess.run([sys.executable,'-B','-c',code,str(Path(__file__).resolve().parents[1]/'scripts')],
                              capture_output=True,text=True,timeout=5)
        self.assertEqual(0,result.returncode,result.stderr)
        self.assertEqual('bounded-and-expired',result.stdout.strip())


class MaterializedAdmissionTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory();self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name)/'copy'
        self.image=[('rules.md',b'Public rules.\n',0o644)]
        io.copy_image(self.image,self.root)
        import memory_native_ab
        memory_native_ab.install_runtime(self.root)

    def prepared(self):
        gate=admission.MaterializedGate(self.root,self.image)
        gate.seal_generated();gate.checkpoint()
        return gate

    def test_complete_inventory_binds_bytes_modes_and_empty_directories(self):
        for mutation in ('bytes','mode','file','directory','root-mode'):
            with self.subTest(mutation=mutation):
                gate=self.prepared();self.assertTrue(gate.verify()['verified'])
                if mutation=='bytes':(self.root/'rules.md').write_text('Changed rule.')
                elif mutation=='mode':(self.root/'rules.md').chmod(0o600)
                elif mutation=='file':(self.root/'new.md').write_text('public')
                elif mutation=='directory':(self.root/'new').mkdir()
                else:self.root.chmod(0o777)
                with self.assertRaises(ValueError):gate.verify()
                shutil.rmtree(self.root);io.copy_image(self.image,self.root)
                import memory_native_ab
                memory_native_ab.install_runtime(self.root)

    def test_initial_input_bytes_mode_presence_and_required_hooks_are_bound(self):
        for mutation in ('bytes','mode','missing-source','missing-hook','extra-directory','initial-directory'):
            with self.subTest(mutation=mutation):
                if mutation=='bytes':(self.root/'rules.md').write_text('Different safe text.')
                elif mutation=='mode':(self.root/'rules.md').chmod(0o600)
                elif mutation=='missing-source':(self.root/'rules.md').unlink()
                elif mutation=='missing-hook':(self.root/'.codex/hooks/working-memory-write.sh').unlink()
                elif mutation=='initial-directory':(self.root/'unexpected-empty').mkdir()
                else:
                    gate=admission.MaterializedGate(self.root,self.image);gate.seal_generated()
                    (self.root/'unexpected-empty').mkdir()
                    with self.assertRaises(ValueError):gate.checkpoint()
                if mutation!='extra-directory':
                    with self.assertRaises(ValueError):admission.MaterializedGate(self.root,self.image)
                shutil.rmtree(self.root);io.copy_image(self.image,self.root)
                import memory_native_ab
                memory_native_ab.install_runtime(self.root)

    def test_no_preexisting_git_database_or_forged_checkpoint(self):
        (self.root/'.git').mkdir()
        with self.assertRaises(ValueError):admission.MaterializedGate(self.root,self.image)
        (self.root/'.git').rmdir()
        gate=admission.MaterializedGate(self.root,self.image)
        with self.assertRaises(ValueError):gate.verify()
        with self.assertRaises(ValueError):gate.checkpoint()
        gate.seal_generated();gate.checkpoint()
        with self.assertRaises(ValueError):gate.checkpoint()

    def test_changes_to_derived_binary_after_preparation_are_rejected(self):
        gate=admission.MaterializedGate(self.root,self.image)
        gate.seal_generated()
        p=self.root/'memory-bank/local/context.db';p.parent.mkdir(parents=True);p.write_bytes(b'\0unsafe')
        with self.assertRaises(ValueError):gate.checkpoint()

    def test_new_state_text_is_scanned_not_trusted_by_directory(self):
        for relative in ('project-brain/dynamic/findings/new.md','memory-bank/local/unknown.json',
                         '.context-handoff/new.txt'):
            with self.subTest(relative=relative):
                gate=admission.MaterializedGate(self.root,self.image);gate.seal_generated()
                p=self.root/relative;p.parent.mkdir(parents=True,exist_ok=True)
                p.write_text('registry_credentials: EV[synthetic-only]')
                with self.assertRaises(ValueError):gate.checkpoint()
                shutil.rmtree(self.root);io.copy_image(self.image,self.root)
                import memory_native_ab
                memory_native_ab.install_runtime(self.root)

    def test_physical_links_and_special_files_are_not_skipped(self):
        for kind in ('symlink','hardlink','fifo'):
            with self.subTest(kind=kind):
                p=self.root/'unsafe'
                if kind=='symlink':p.symlink_to(self.root/'rules.md')
                elif kind=='hardlink':os.link(self.root/'rules.md',p)
                else:os.mkfifo(p)
                try:
                    with self.assertRaises(ValueError):admission.physical_image(self.root)
                finally:p.unlink()


class ControllerAdmissionTests(unittest.TestCase):
    def setUp(self):
        from tests.test_memory_native_ab import ControllerTests
        self.case=ControllerTests();self.case.setUp();self.addCleanup(self.case.doCleanups)
        self.c=self.case.controller

    def test_plan_blocks_before_creating_output_without_values_in_errors(self):
        (self.case.source/'public.yaml').write_text('registry_credentials: EV[SYNTHETIC-CANARY-41]')
        with self.assertRaises(ValueError) as caught:self.c.plan(self.case.config,self.case.run)
        self.assertNotIn('SYNTHETIC-CANARY-41',str(caught.exception))
        self.assertFalse(self.case.run.exists())

    def test_admission_module_participates_in_runtime_seal(self):
        initial=self.c.runtime_digest();original=io.read_file
        def changed(file,*args,**kwargs):
            raw,mode=original(file,*args,**kwargs)
            if Path(file).name=='memory_native_ab_admission.py':raw+=b'\n# changed\n'
            return raw,mode
        with mock.patch.object(io,'read_file',side_effect=changed):self.assertNotEqual(initial,self.c.runtime_digest())

    @unittest.skipUnless(sys.platform=='linux' and shutil.which('bwrap'),'Linux bubblewrap fixture')
    def test_stage_checks_the_held_source_image_before_copy(self):
        self.c.plan(self.case.config,self.case.run)
        original=io.snapshot;reads=0
        def drift(*args,**kwargs):
            nonlocal reads
            reads+=1;image=original(*args,**kwargs)
            return image if reads==1 else [('rules.md',b'Changed safe fact.',0o644)]
        with mock.patch.object(io,'snapshot',side_effect=drift),mock.patch.object(io,'copy_image') as copied:
            result=self.c.run(self.case.run,stage='source')
        self.assertEqual('failed',result['stages']['source']);copied.assert_not_called()

    @unittest.skipUnless(sys.platform=='linux' and shutil.which('bwrap'),'Linux bubblewrap fixture')
    def test_overlay_and_mount_injections_never_launch_model(self):
        for boundary,relative in [('install_runtime','public.yaml'),('make_view','.codex/extra.json'),
                                  ('make_view','memory-bank/local/late.json')]:
            with self.subTest(boundary=boundary,relative=relative):
                # Each independent plan keeps its own terminal failure.
                previous=self.case.run;self.case.run=self.case.root/('run-'+str(len(list(self.case.root.glob('run-*')))))
                original=getattr(self.c,boundary)
                def inject(*args,**kwargs):
                    result=original(*args,**kwargs)
                    p=args[0]/relative;p.parent.mkdir(parents=True,exist_ok=True)
                    p.write_text('registry_credentials: EV[synthetic-only]')
                    return result
                transport=self.c.run_process
                native_calls=[]
                def no_model(*args,**kwargs):
                    if kwargs.get('native',True):
                        native_calls.append(args);raise AssertionError('model launch after rejected image')
                    return transport(*args,**kwargs)
                with mock.patch.object(self.c,boundary,side_effect=inject),mock.patch.object(self.c,'run_process',side_effect=no_model):
                    result=self.case.drive_fixture()
                self.assertEqual('failed',result['stages']['source'])
                self.assertEqual([],native_calls)
                if boundary=='install_runtime':self.assertFalse((self.case.run/'work/source/.git').exists())
                self.case.run=previous

    @unittest.skipUnless(sys.platform=='linux' and shutil.which('bwrap'),'Linux bubblewrap fixture')
    def test_served_stage_rescans_new_knowledge_despite_valid_handoff_hash(self):
        fixture=Path(__file__).parent/'fixtures/memory_native_ab_cli.py'
        (self.case.source/'rules.md').write_text('The Amber dispatcher retries only idempotent requests.\n')
        (self.case.source/'fixture_cli.py').write_bytes(fixture.read_bytes())
        self.c.plan(self.case.config,self.case.run)
        view=self.c.make_view
        with mock.patch.object(self.c,'native_command',side_effect=lambda corpus,config,task:[sys.executable,str(corpus/'fixture_cli.py')]),\
             mock.patch.object(self.c,'make_view',side_effect=lambda *a,**k:view(*a,**k,credentials=False)):
            self.assertEqual('captured',self.c.run(self.case.run,stage='source')['stages']['source'])
            corpus=self.case.run/'work/source'
            (corpus/'memory-bank/chunks/unsafe.md').write_text('password: "synthetic-only"')
            state=io.read_json(self.case.run/'control/state.json')
            state['stages']['source']['result']['handoff_sha256']=io.image_digest(io.snapshot(corpus))
            io.write_json(self.case.run/'control/state.json',state)
            transport=self.c.run_process
            native_calls=[]
            def no_model(*args,**kwargs):
                if kwargs.get('native',True):
                    native_calls.append(args);raise AssertionError('B launched with unsafe new knowledge')
                return transport(*args,**kwargs)
            with mock.patch.object(self.c,'run_process',side_effect=no_model):result=self.c.run(self.case.run,stage='served')
        self.assertEqual('failed',result['stages']['served'])
        self.assertEqual([],native_calls)
        self.assertFalse((self.case.run/'work/served').exists())


if __name__=='__main__':unittest.main()
