"""Behavioral checks for the catalog facade and native process boundary."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts import kit3


class Kit3Tests(unittest.TestCase):
    def run_cli(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = kit3.main(list(args))
        return code, out.getvalue(), err.getvalue()

    def test_search_filters_and_json_are_offline(self):
        with patch('scripts.kit3.subprocess.run') as run:
            code, out, _ = self.run_cli('find', 'caveman', '--agent', 'codex', '--json')
        self.assertEqual(code, 0)
        self.assertEqual([x['id'] for x in json.loads(out)], ['caveman'])
        self.assertEqual(json.loads(out)[0]['review_status'], 'unreviewed')
        run.assert_not_called()
        self.assertEqual(json.loads(self.run_cli('find', 'unlikely-no-match', '--json')[1]), [])

    def test_non_skill_catalog_entry_does_not_execute_its_guide(self):
        with patch('scripts.kit3.subprocess.run') as run:
            code, _, err = self.run_cli('add', 'claude-plugins-official', '--dry-run')
        self.assertEqual(code, 1)
        self.assertIn('manual guide', err)
        run.assert_not_called()

    def test_relative_source_is_resolved_before_target_changes(self):
        source = str((Path.cwd() / "scripts").resolve())
        args = kit3.parser().parse_args(["add", "./scripts", "--target", "/other/project"])
        self.assertEqual(kit3.native_command(args, [])[4], source)
        with self.assertRaises(kit3.catalog.KitError):
            kit3.resolve_source("./nonexistent-kit3-source", [])

    def test_native_arguments_and_exit_status(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch('scripts.kit3.shutil.which', return_value='/bin/npx'), patch('scripts.kit3.subprocess.run') as run:
                run.return_value.returncode = 7
                code, _, _ = self.run_cli('add', 'caveman', '--target', directory, '--agent', 'codex', '--skill', 'a skill; echo nope', '--copy', '--yes')
            self.assertEqual(code, 7)
            self.assertEqual(run.call_args.args[0], ['npx', '--yes', kit3.SKILLS_PACKAGE, 'add', 'JuliusBrussee/caveman', '--skill', 'a skill; echo nope', '--copy', '--agent', 'codex', '--yes'])
            self.assertEqual(run.call_args.kwargs, {'cwd': Path(directory), 'check': False})
            self.assertFalse((Path(directory)/'.kit3-manifest.json').exists())

    def test_dry_run_performs_no_download_or_write(self):
        with tempfile.TemporaryDirectory() as directory, patch('scripts.kit3.subprocess.run') as run, patch('scripts.kit3.shutil.which') as which:
            code, _, err = self.run_cli('add', 'obra/superpowers', '--target', directory, '--dry-run')
            self.assertEqual(code, 0)
            self.assertIn('WOULD_RUN', err)
            self.assertEqual(list(Path(directory).iterdir()), [])
            run.assert_not_called(); which.assert_not_called()

    def test_update_never_implicitly_touches_global_skills(self):
        args = kit3.parser().parse_args(['update', 'my-skill', '--yes'])
        self.assertEqual(kit3.native_command(args, []), ['npx', '--yes', kit3.SKILLS_PACKAGE, 'update', 'my-skill', '--project', '--yes'])
        args = kit3.parser().parse_args(['update', '--global'])
        self.assertIn('--global', kit3.native_command(args, []))
        self.assertNotIn('--project', kit3.native_command(args, []))

    def test_native_json_diagnostics_do_not_pollute_stdout(self):
        with patch('scripts.kit3.shutil.which', return_value='/bin/npx'), patch('scripts.kit3.subprocess.run') as run:
            run.return_value.returncode = 0
            code, out, err = self.run_cli('list', '--json')
        self.assertEqual((code, out), (0, ''))
        self.assertIn('--json', run.call_args.args[0])
        self.assertIn('RUN', err)

    def test_record_and_native_state_are_distinct(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(self.run_cli('record', 'caveman', '--target', directory, '--pin', 'caveman=abc123')[0], 0)
            code, out, _ = self.run_cli('list', '--recorded', '--json', '--target', directory)
            self.assertEqual(code, 0)
            self.assertEqual(json.loads(out)['entries']['caveman']['pinned_ref'], 'abc123')
            text = self.run_cli('list', '--recorded', '--target', directory)[1]
            self.assertIn('installation not verified', text)

    def test_source_cannot_inject_options_or_shell(self):
        for source in ('--all', 'https://user:password@example.com/repo', 'owner/repo\n--all', 'owner/repo;touch x'):
            with self.subTest(source=source), self.assertRaises(kit3.catalog.KitError):
                kit3.resolve_source(source, [])

    def test_remote_search_rejects_ignored_local_filters(self):
        with patch('scripts.kit3.subprocess.run') as run:
            code, _, _ = self.run_cli('find', 'php', '--remote', '--review', 'clear')
        self.assertEqual(code, 1)
        run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
