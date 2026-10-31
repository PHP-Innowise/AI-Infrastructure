"""Public independent outcome oracles; no models or private dataset."""
import json
from pathlib import Path
import sys
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
