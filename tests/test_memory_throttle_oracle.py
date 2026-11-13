"""Future oracle v2 uses synthetic copies, never completed-study artifacts."""
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import memory_throttle_oracle_v2 as oracle
import memory_outcome_verify as verify

FIXTURES = Path(__file__).parent / 'fixtures/memory_throttle'
AVAILABLE = sys.platform == 'linux' and shutil.which('php') and shutil.which('bwrap')


class OracleContractTests(unittest.TestCase):
    def test_portable_public_controls_and_new_version(self):
        controls = oracle.variants((FIXTURES / 'source.php').read_bytes())
        self.assertEqual(52, len(controls))
        self.assertEqual(2, sum(expected for _, _, expected in controls))
        self.assertEqual('entryfetch-throttle-oracle-v2', oracle.PROFILE)

    def test_collision_uses_bare_argument_and_both_readonly_sources(self):
        with mock.patch.object(oracle, 'execute', return_value={}) as execute:
            oracle.nested(b'<?php', b'<?php // selected', b'<?php',
                          'bare-collision', shadow_source=b'<?php // shadow')
        command = execute.call_args[0][0]
        self.assertEqual('EntryFetchThrottle.php', command[-1])
        self.assertIn('include_path=/shadow:.', command)
        self.assertIn('/work/EntryFetchThrottle.php', command)
        self.assertIn('/shadow/EntryFetchThrottle.php', command)
        self.assertIn('--unshare-net', command)
        self.assertIn('--unshare-pid', command)
        self.assertIsInstance(execute.call_args.kwargs['startup_marker'], bytes)

    def test_programs_and_system_folders_as_the_system_lays_them_out(self):
        # Debian and Ubuntu, as on GitHub's runners: /usr/bin/php links through
        # /etc/alternatives, which the namespace does not mount, and /lib64
        # leads to usr/lib64. Folders bound as Arch lays them out held no PHP.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'usr/bin').mkdir(parents=True)
            (root / 'usr/lib64').mkdir()
            (root / 'bin').symlink_to('usr/bin')
            (root / 'lib64').symlink_to('usr/lib64')
            (root / 'lib').mkdir()
            self.assertEqual(['--symlink', 'usr/bin', '/bin', '--ro-bind', str(root / 'lib'), '/lib',
                              '--symlink', 'usr/lib64', '/lib64'], oracle.system_links(str(root)))
            php = root / 'usr/bin/php8.3'
            php.write_bytes(b'')
            (root / 'alternatives').mkdir()
            (root / 'alternatives/php').symlink_to(php)
            (root / 'php').symlink_to(root / 'alternatives/php')
            with mock.patch.object(oracle, 'PHP', str(root / 'php')), \
                    mock.patch.object(oracle, 'execute', return_value={}) as execute:
                oracle.nested(b'<?php', b'<?php', b'<?php')
            command = execute.call_args[0][0]
            self.assertIn(os.path.realpath(php), command)
            self.assertNotIn(str(root / 'php'), command)


@unittest.skipUnless(AVAILABLE, 'Linux, PHP and Bubblewrap required')
class IsolatedOracleTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.candidate = self.root / 'candidate'
        self.candidate.mkdir()
        self.source = (FIXTURES / 'source.php').read_bytes()
        self.test = (FIXTURES / 'public-test.php').read_bytes()
        for relative, data in ((oracle.SOURCE_REL, self.source), (oracle.TEST_REL, self.test)):
            target = self.candidate / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        self.script = (FIXTURES / 'reference.php').read_bytes()
        self.unresolved = self.script.replace(b'require realpath($argv[1]);', b'require $argv[1];')
        self.assertNotEqual(self.script, self.unresolved)
        self.broken = next(raw for name, raw, _ in oracle.variants(self.source) if name == 'allow-all')

    def test_collision_exposes_wrong_selection_in_both_directions(self):
        for selected, shadow, expected in ((self.source, self.broken, False),
                                           (self.broken, self.source, True)):
            with self.subTest(selected_good=selected == self.source):
                checked = oracle.nested(self.unresolved, selected, self.test,
                                        'bare-collision', shadow_source=shadow)
                self.assertIsNone(checked['limit'])
                # The unresolved loader tests the opposite (shadow) source.
                self.assertEqual(expected, checked['code'] == 0)

    def test_future_external_verifier_accepts_resolved_and_rejects_unresolved_loader(self):
        answer = self.root / 'answer.txt'
        answer.write_text('Artifact outcome is independent of this text.')
        artifact = self.candidate / 'verification/throttle_regression.php'
        artifact.parent.mkdir()
        script_file = ROOT / 'scripts/memory_throttle_oracle_v2.py'
        source_file = FIXTURES / 'source.php'
        test_file = FIXTURES / 'public-test.php'
        # Sealed as the files the links lead to, and the PHP named to the
        # oracle: the verifier seals no link, and its namespace has no
        # /etc/alternatives for one to lead through.
        php = os.path.realpath(oracle.PHP)
        case = {'mode': 'patch', 'argv': [sys.executable, str(script_file),
                '--source', str(source_file), '--public-test', str(test_file), '--php', php],
                'timeout': 90, 'read_roots': [str(script_file), str(source_file), str(test_file),
                                             php, os.path.realpath(oracle.BWRAP)]}
        for script, correct in ((self.script, True), (self.unresolved, False)):
            with self.subTest(resolved=correct):
                artifact.write_bytes(script)
                checked = verify.verify_task(self.candidate, answer, case)
                self.assertIs(correct, checked['correct'], checked)
                self.assertEqual('passed' if correct else 'failed', checked['status'])
                self.assertEqual(verify.verifier_digest(case), checked['verifier_digest'])

    def test_missing_php_remains_infrastructure_unknown(self):
        with mock.patch.object(oracle, 'PHP', '/usr/bin/no-such-php-for-oracle-v2'):
            with self.assertRaises(oracle.InfrastructureUnknown):
                oracle.nested(b'<?php echo "ok";', self.source, self.test)


if __name__ == '__main__':
    unittest.main()
