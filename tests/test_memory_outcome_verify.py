"""Public independent outcome oracles; no models or private dataset."""
import json
import os
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'scripts'))
import memory_outcome_verify as verify

class VerifierTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name); self.snapshot=self.root/'candidate'; self.snapshot.mkdir()
        self.answer=self.root/'answer.txt'; self.answer.write_text('Gold.php: 41')

    def test_right_file_name_is_not_correctness(self):
        result=verify.verify_task(self.snapshot,self.answer,{'mode':'answer','equals':'42'})
        self.assertIs(False,result['correct'])
        self.answer.write_text('42\n')
        self.assertIs(True,verify.verify_task(self.snapshot,self.answer,{'mode':'answer','equals':'42'})['correct'])
        self.assertNotIn('42',json.dumps(result))

    def test_invalid_or_unsupported_rubric_is_unavailable(self):
        for case in ({'mode':'answer'}, {'mode':'llm'}, {'mode':'answer','equals':'42','unknown':True}):
            self.assertIsNone(verify.verify_task(self.snapshot,self.answer,case)['correct'])

    def test_patch_oracle_is_external_and_unchanged(self):
        oracle=self.root/'oracle.py'; oracle.write_text("import json\nfrom pathlib import Path\nprint(json.dumps({'schema':'task-outcome-verifier-v1','completed':True,'correct':Path('result.txt').read_text()=='42'}))\n")
        original=oracle.read_bytes()
        case={'mode':'patch','argv':[sys.executable,str(oracle)],'timeout':2,'read_roots':[str(self.root)]}
        with patch.object(verify,'verifier_command',side_effect=lambda snapshot,argv,roots: list(argv)):
            (self.snapshot/'result.txt').write_text('41')
            (self.snapshot/'tests.py').write_text('assert True')
            self.assertIs(False,verify.verify_task(self.snapshot,self.answer,case)['correct'])
            (self.snapshot/'result.txt').write_text('42')
            self.assertIs(True,verify.verify_task(self.snapshot,self.answer,case)['correct'])
        self.assertEqual(original,oracle.read_bytes())

    def test_actual_isolated_patch_driver_returns_independent_receipt(self):
        import memory_outcome_isolation as isolation
        private = self.root / 'oracle'; private.mkdir()
        driver = private / 'driver.py'
        driver.write_text("import json\nfrom pathlib import Path\n"
            "print(json.dumps({'schema':'task-outcome-verifier-v1','completed':True,'correct':Path('result.txt').read_text()=='42'}))\n")
        try: view = isolation.prepare_agent_view(self.snapshot, private, command=[sys.executable])
        except isolation.IsolationError as error: self.skipTest(str(error))
        if not isolation.probe_agent_view(view, driver): self.skipTest('namespace unavailable')
        (self.snapshot / 'result.txt').write_text('42')
        case = {'mode': 'patch', 'argv': [sys.executable, str(driver)], 'timeout': 2, 'read_roots': [str(private)]}
        self.assertIs(True, verify.verify_task(self.snapshot, self.answer, case)['correct'])
        (self.snapshot / 'result.txt').write_text('41')
        self.assertIs(False, verify.verify_task(self.snapshot, self.answer, case)['correct'])

    def test_timeout_missing_toolchain_and_unsafe_snapshot_are_unknown(self):
        with patch.object(verify,'verifier_command',side_effect=lambda snapshot,argv,roots: list(argv)):
            for argv in ([str(self.root/'missing')],[sys.executable,'-c','import time;time.sleep(10)']):
                result=verify.verify_task(self.snapshot,self.answer,{'mode':'patch','argv':argv,'timeout':.05,'read_roots':[]})
                self.assertIsNone(result['correct'])
        if hasattr(__import__('os'),'symlink'):
            try: (self.snapshot/'escape').symlink_to(self.answer)
            except OSError: return
            self.assertIsNone(verify.verify_task(self.snapshot,self.answer,{'mode':'answer','equals':'42'})['correct'])

    def test_oracle_identity_binds_implementation_bytes(self):
        oracle=self.root/'sealed.py'; oracle.write_text('raise SystemExit(0)')
        case={'mode':'patch','argv':[sys.executable,str(oracle)],'timeout':2,'read_roots':[str(self.root)]}
        first=verify.verifier_digest(case)
        oracle.write_text('raise SystemExit(1)')
        self.assertNotEqual(first,verify.verifier_digest(case))

    def test_oracle_seal_includes_hidden_labels_empty_directories_and_modes(self):
        private = self.root/'private'; private.mkdir()
        driver = private/'oracle.py'; driver.write_text('print("public oracle")\n')
        case = {'mode': 'patch', 'argv': [sys.executable, str(driver)],
                'timeout': 2, 'read_roots': [str(private)]}
        for folder in ('.git', '__pycache__'):
            label = private/folder/'expected.txt'; label.parent.mkdir()
            label.write_text('42')
            first = verify.verifier_digest(case)
            label.write_text('41')
            self.assertNotEqual(first, verify.verifier_digest(case), folder)
        first = verify.verifier_digest(case)
        (private/'empty-label-group').mkdir()
        self.assertNotEqual(first, verify.verifier_digest(case))
        if os.name == 'posix':
            first = verify.verifier_digest(case); driver.chmod(0o755)
            self.assertNotEqual(first, verify.verifier_digest(case))

    def test_duplicate_receipt_keys_cannot_report_success(self):
        private = self.root/'private'; private.mkdir()
        oracle = private/'oracle.py'
        case = {'mode': 'patch', 'argv': [sys.executable, str(oracle)],
                'timeout': 2, 'read_roots': [str(private)]}
        receipts = [
            '{"schema":"task-outcome-verifier-v1","completed":true,"correct":false,"correct":true}',
            '{"schema":"task-outcome-verifier-v1","completed":false,"completed":true,"correct":true}',
        ]
        with patch.object(verify, 'verifier_command', side_effect=lambda snapshot, argv, roots: list(argv)):
            for receipt in receipts:
                with self.subTest(receipt=receipt):
                    oracle.write_text('print(' + repr(receipt) + ')\n')
                    self.assertIsNone(verify.verify_task(self.snapshot, self.answer, case)['correct'])

    def test_missing_and_linked_empty_candidate_roots_are_unavailable(self):
        self.answer.write_text('42')
        case = {'mode': 'answer', 'equals': '42'}
        self.assertIsNone(verify.verify_task(self.root/'missing', self.answer, case)['correct'])
        linked = self.root/'linked-candidate'
        try: linked.symlink_to(self.snapshot, target_is_directory=True)
        except (OSError, NotImplementedError): return
        self.assertIsNone(verify.verify_task(linked, self.answer, case)['correct'])

    @unittest.skipUnless(os.name == 'posix', 'POSIX special mode fixture')
    def test_special_oracle_permission_bits_are_refused(self):
        oracle = self.root/'oracle.py'; oracle.write_text('print("unused")\n')
        case = {'mode': 'patch', 'argv': [sys.executable, str(oracle)],
                'timeout': 2, 'read_roots': [str(oracle)]}
        oracle.chmod(0o4644)
        with self.assertRaises(ValueError): verify.verifier_digest(case)

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'POSIX special file fixture')
    def test_special_answer_and_oracle_files_are_refused_without_blocking(self):
        oracle = self.root/'oracle.py'; oracle.write_text('print("unused")\n')
        fifo = self.root/'label'; os.mkfifo(fifo)
        nested = self.root/'private'; nested.mkdir(); os.mkfifo(nested/'label')
        code = ('import json,sys;from pathlib import Path;sys.path.insert(0,sys.argv[1]);'
                'import memory_outcome_verify as v;'
                'case=json.loads(sys.argv[4]);'
                'print(json.dumps(v.verify_task(Path(sys.argv[2]),Path(sys.argv[3]),case)))')
        for answer, case in [
            (fifo, {'mode': 'answer', 'equals': '42'}),
            (self.answer, {'mode': 'patch', 'argv': [sys.executable, str(oracle)],
                          'timeout': .01, 'read_roots': [str(fifo)]}),
            (self.answer, {'mode': 'patch', 'argv': [sys.executable, str(oracle)],
                          'timeout': .01, 'read_roots': [str(nested)]}),
        ]:
            with self.subTest(answer=answer.name, case=case):
                result = subprocess.run([sys.executable, '-B', '-c', code,
                    str(Path(verify.__file__).parent), str(self.snapshot), str(answer), json.dumps(case)],
                    capture_output=True, timeout=2)
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertIsNone(json.loads(result.stdout)['correct'])

    def test_candidate_owned_test_discovery_is_not_an_independent_oracle(self):
        (self.snapshot/'test_fake.py').write_text('import unittest\n')
        case={'mode':'patch','argv':[sys.executable,'-m','unittest','discover'],'timeout':2,'read_roots':[]}
        with patch.object(verify,'verifier_command',side_effect=lambda snapshot,argv,roots: list(argv)) as command:
            self.assertIsNone(verify.verify_task(self.snapshot,self.answer,case)['correct'])
            self.assertFalse(command.called)

    def test_appended_external_file_is_not_the_executed_oracle(self):
        oracle=self.root/'oracle.py'; oracle.write_text('raise SystemExit(0)')
        case={'mode':'patch','argv':[sys.executable,'-c','pass',str(oracle)],'timeout':2,'read_roots':[str(self.root)]}
        with patch.object(verify,'verifier_command',side_effect=lambda snapshot,argv,roots: list(argv)) as command:
            self.assertIsNone(verify.verify_task(self.snapshot,self.answer,case)['correct'])
            self.assertFalse(command.called)

    def test_oversized_sparse_candidate_is_unavailable_before_copy(self):
        with (self.snapshot/'huge').open('wb') as stream: stream.truncate(2**40)
        with patch.object(verify.shutil,'copytree') as copied:
            self.assertIsNone(verify.verify_task(self.snapshot,self.answer,{'mode':'answer','equals':'42'})['correct'])
            self.assertFalse(copied.called)

    def test_exit_zero_without_trusted_completion_receipt_is_not_success(self):
        oracle=self.root/'oracle.py'; oracle.write_text('raise SystemExit(0)')
        case={'mode':'patch','argv':[sys.executable,str(oracle)],'timeout':2,'read_roots':[str(self.root)]}
        with patch.object(verify,'verifier_command',side_effect=lambda snapshot,argv,roots: list(argv)):
            self.assertIsNone(verify.verify_task(self.snapshot,self.answer,case)['correct'])
