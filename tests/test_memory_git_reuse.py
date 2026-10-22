"""Fewer Git starts per prompt, with every safety check still in force.

A prompt refresh started Git nine times: three branch checks and two ignore
checks of three processes each. The branch is now asked once and re-checked
by the checkout's HEAD identity, and the second ignore check reuses the frame
the first one confirmed while every file that decides it is unchanged. These
tests run the real CLI and real Git through a logging shim, and move the
branch or the repository at the worst moment.
"""
from __future__ import annotations
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import unittest.mock

ROOT = Path(__file__).resolve().parents[1]
EDITION = ROOT / 'PHP Core'
TASK = 'feature/heron-git'
QUERY = 'Which rounding does the heron ledger use?'


def full_capsule(root):
    """Pin the full automatic capsule in a fixture project.

    The editions ship the recall capsule - working state, recent files and
    the task's own saves. A test of a layer only the full capsule carries -
    shared documents and what is built on them - pins `full`, which a
    project can still choose.
    """
    config = Path(root) / 'project-brain/config/runtime.json'
    data = json.loads(config.read_text(encoding='utf-8'))
    data['capsule_profile'] = 'full'
    config.write_text(json.dumps(data), encoding='utf-8')


@unittest.skipUnless(os.name == 'posix' and shutil.which('git') and shutil.which('bash'), 'POSIX Git shim')
class GitReuseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='heron-git-')
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base / 'project'
        self.root.mkdir()
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        self.env.update(HOME=str(self.base / 'home'), XDG_CONFIG_HOME=str(self.base / 'xdg'),
                        TMPDIR=str(self.base), PYTHONDONTWRITEBYTECODE='1', GIT_OPTIONAL_LOCKS='0')
        (self.base / 'home').mkdir()
        self.real_git = shutil.which('git')
        for rel in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config',
                    'project-brain/schemas', 'project-brain/templates'):
            shutil.copytree(EDITION / rel, self.root / rel, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        (self.root / 'docs').mkdir()
        # A project document the query selects: delivering it checks its own
        # ignore status again, after the index did.
        (self.root / 'docs/ledger.md').write_text(
            '# Heron ledger\n\nThe heron ledger export cadence is nightly. Heron ledger rounding is half-even.\n',
            encoding='utf-8')
        self.git('init', '-q', '-b', TASK)
        self.git('config', 'user.email', 'heron@example.invalid')
        self.git('config', 'user.name', 'Heron')
        self.git('add', '-A')
        self.git('commit', '-q', '-m', 'fixture')
        self.cli('start', '--task-id', TASK, '--goal', 'Heron ledger export', '--json')
        self.cli('index')
        self.log = self.base / 'git.log'

    def git(self, *args, cwd=None):
        subprocess.run([self.real_git, '-C', str(cwd or self.root), *args], env=self.env, check=True,
                       capture_output=True, timeout=60)

    def cli(self, *args):
        return subprocess.run([sys.executable, str(self.root / 'memory-bank/scripts/context.py'), *args],
                              cwd=self.root, env=self.env, check=True, capture_output=True, text=True, timeout=120)

    def shim(self, action=''):
        binaries = self.base / 'bin'
        binaries.mkdir(exist_ok=True)
        (binaries / 'git').write_text(f'''#!/usr/bin/env bash
printf '%s\\n' "$*" >> "{self.log}"
"{self.real_git}" "$@"
status=$?
{action}
exit $status
''', encoding='utf-8')
        (binaries / 'git').chmod(0o755)
        return dict(self.env, PATH=str(binaries) + os.pathsep + self.env['PATH'])

    def refresh(self, environment):
        return subprocess.run([sys.executable, str(self.root / 'memory-bank/scripts/context.py'), 'refresh',
                               '--host', 'claude', '--query', QUERY, '--task-id', TASK, '--ephemeral',
                               '--sanitize', '--expected-branch', TASK],
                              cwd=self.root, env=environment, capture_output=True, text=True, timeout=120)

    def calls(self):
        return self.log.read_text(encoding='utf-8').splitlines() if self.log.exists() else []

    def test_a_prompt_asks_the_branch_once_and_the_frame_twice(self):
        full_capsule(self.root)  # counts the Git calls of the full search path
        result = self.refresh(self.shim())
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn('working: ' + TASK, result.stdout)
        self.assertIn('docs/ledger.md', result.stdout)
        calls = self.calls()
        # The first ignore check confirms the frame before and after its
        # query; every later one reuses it. The branch is asked once.
        self.assertGreaterEqual(sum('check-ignore' in call for call in calls), 2, calls)
        self.assertEqual(2, sum('rev-parse' in call for call in calls), calls)
        self.assertEqual(1, sum('symbolic-ref' in call for call in calls), calls)

    def test_a_rebase_keeps_the_branch_it_rewrites_as_the_task(self):
        # A rebase detaches HEAD on the commits it replays, and the branch it
        # rewrites is still the work in hand - often the conflict a turn asks
        # about. It had no memory at all.
        self.git('checkout', '-q', '--detach')
        state = self.root / '.git/rebase-merge'
        state.mkdir()
        (state / 'head-name').write_text('refs/heads/' + TASK + '\n', encoding='utf-8')
        result = self.refresh(self.env)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn('working: ' + TASK, result.stdout)
        # Another branch's rebase, or none, admits nothing.
        (state / 'head-name').write_text('refs/heads/feature/other\n', encoding='utf-8')
        result = self.refresh(self.env)
        self.assertNotEqual(0, result.returncode)
        self.assertIn('Inferred task branch changed or unavailable', result.stderr)
        shutil.rmtree(state)
        result = self.refresh(self.env)
        self.assertNotEqual(0, result.returncode)
        self.assertIn('Inferred task branch changed or unavailable', result.stderr)

    def test_a_branch_switched_during_the_refresh_is_still_refused(self):
        # The switch lands after the first answer about the branch and before
        # delivery: the checkout's HEAD is a new file, so Git is asked again.
        switch = (f'case "$*" in *check-ignore*) [ -e "{self.base}/switched" ] || '
                  f'{{ touch "{self.base}/switched"; "{self.real_git}" -C "{self.root}" checkout -q -b feature/other; }} ;; esac')
        result = self.refresh(self.shim(switch))
        self.assertNotEqual(0, result.returncode)
        self.assertIn('Inferred task branch changed or unavailable', result.stderr)
        self.assertNotIn('working: ' + TASK, result.stdout)
        self.assertGreaterEqual(sum('symbolic-ref' in call for call in self.calls()), 2)

    def test_a_repository_retargeted_during_the_reused_frame_is_refused(self):
        full_capsule(self.root)  # counts the Git calls of the full search path
        foreign = self.base / 'foreign'
        foreign.mkdir()
        self.git('init', '-q', '-b', TASK, cwd=foreign)
        # On the second ignore check - the one that reuses the confirmed frame
        # - the project's .git is swapped for another repository.
        swap = (f'case "$*" in *check-ignore*) if [ -e "{self.base}/first-done" ]; then '
                f'[ -e "{self.base}/swapped" ] || {{ touch "{self.base}/swapped"; mv "{self.root}/.git" "{self.base}/original-admin"; '
                f'ln -s "{foreign}/.git" "{self.root}/.git"; }}; else touch "{self.base}/first-done"; fi ;; esac')
        result = self.refresh(self.shim(swap))
        self.assertTrue((self.base / 'swapped').exists(), self.calls())
        self.assertNotEqual(0, result.returncode)
        self.assertIn('Git source context unavailable', result.stderr)
        self.assertNotIn('working: ' + TASK, result.stdout)


@unittest.skipUnless(shutil.which('git'), 'Git')
class ManyDocumentsIgnoreTests(unittest.TestCase):
    """`git check-ignore` asked the index about every path, a tenth of a
    millisecond each in a 20,000-file checkout: such a project outlasted the
    check's two seconds and every refresh failed. Above IGNORE_INDEX_PATHS the
    rules are asked without the index and the tracked files they name are
    taken out - the same answer, in milliseconds."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='heron-ignore-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        env = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        env.update(HOME=str(self.root), XDG_CONFIG_HOME=str(self.root / 'xdg'), GIT_OPTIONAL_LOCKS='0')
        patcher = unittest.mock.patch.dict(os.environ, env, clear=True)
        patcher.start(); self.addCleanup(patcher.stop)
        sys.path.insert(0, str(EDITION / 'memory-bank/scripts'))
        try:
            import workspace_roots
        finally:
            sys.path.pop(0)
        self.roots = workspace_roots
        (self.root / 'docs/keep').mkdir(parents=True)
        (self.root / 'docs/build').mkdir()
        (self.root / '.gitignore').write_text('docs/build/\n*.secret.md\n', encoding='utf-8')
        for n in range(300):
            (self.root / f'docs/keep/heron-{n}.md').write_text(f'# Heron {n}\n', encoding='utf-8')
        (self.root / 'docs/build/forced.md').write_text('# Forced\n', encoding='utf-8')
        for args in (('init', '-q', '-b', 'main'), ('add', '.gitignore', 'docs/keep'),
                     ('add', '-f', 'docs/build/forced.md'),
                     ('-c', 'user.email=h@example.invalid', '-c', 'user.name=H', 'commit', '-q', '-m', 'fixture')):
            subprocess.run(['git', '-C', str(self.root), *args], check=True, capture_output=True, timeout=60)
        for n in range(20):
            (self.root / f'docs/build/page-{n}.md').write_text('# Built\n', encoding='utf-8')
        (self.root / 'docs/keep/notes.secret.md').write_text('# Secret notes\n', encoding='utf-8')
        self.paths = sorted(path.relative_to(self.root).as_posix() for path in self.root.glob('docs/**/*.md'))

    def test_many_paths_get_the_answer_few_paths_get(self):
        self.assertGreater(len(self.paths), self.roots.IGNORE_INDEX_PATHS)
        many = self.roots._source_git_ignored(self.root, self.paths, 10.0)
        few = set()
        for start in range(0, len(self.paths), self.roots.IGNORE_INDEX_PATHS):
            few |= self.roots._source_git_ignored(
                self.root, self.paths[start:start + self.roots.IGNORE_INDEX_PATHS], 10.0)
        self.assertEqual(few, many)
        self.assertEqual(21, len(many))
        self.assertIn(b'docs/keep/notes.secret.md', many)
        # Tracked, though the rules name it: Git never counts it as ignored.
        self.assertNotIn(b'docs/build/forced.md', many)

    def test_many_ignored_paths_are_told_from_tracked_in_one_call(self):
        # Generated pages the project ignores, by the hundred: their tracked
        # status is asked once for their directory, not path by path.
        for n in range(20, 220):
            (self.root / f'docs/build/page-{n}.md').write_text('# Built\n', encoding='utf-8')
        paths = sorted(path.relative_to(self.root).as_posix() for path in self.root.glob('docs/**/*.md'))
        listings = []
        real = self.roots._source_git_run
        def run(project, arguments, environment, deadline, payload=None):
            if 'ls-files' in arguments:
                listings.append(arguments)
            return real(project, arguments, environment, deadline, payload)
        many = self.roots._source_git_ignored(self.root, paths, 10.0, _run=run)
        self.assertEqual(1, len(listings))
        few = set()
        for start in range(0, len(paths), self.roots.IGNORE_INDEX_PATHS):
            few |= self.roots._source_git_ignored(self.root, paths[start:start + self.roots.IGNORE_INDEX_PATHS], 10.0)
        self.assertEqual(few, many)
        self.assertEqual(221, len(many))
        self.assertNotIn(b'docs/build/forced.md', many)

    def test_the_index_is_not_asked_path_by_path(self):
        seen = []
        real = self.roots._source_git_run
        def run(project, arguments, environment, deadline, payload=None):
            seen.append([argument for argument in arguments if not argument.startswith('--git-dir')
                         and not argument.startswith('--work-tree')][:3])
            return real(project, arguments, environment, deadline, payload)
        ignored = self.roots._source_git_ignored(self.root, self.paths, 10.0, _run=run)
        self.assertIn(['check-ignore', '--no-index', '--stdin'], seen)
        self.assertIn(['--literal-pathspecs', 'ls-files', '-z'], seen)
        self.assertEqual(21, len(ignored))


if __name__ == '__main__':
    unittest.main()
