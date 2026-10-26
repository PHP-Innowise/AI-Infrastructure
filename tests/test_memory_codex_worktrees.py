"""Codex started in a linked worktree of an installed project.

Codex reads a project's .codex/ only in the folders between the session's
folder and the nearest .git, which in a worktree is the worktree's own, and
Git gives a worktree none of an installation it does not track: Codex 0.164
started in a worktree of an installed project ran none of its hooks. A Codex
session in the project root keeps a dispatcher in the person's own Codex hooks
(memory-bank/scripts/codex_worktrees.py), which Codex runs once the person has
trusted the entries in its own hook review: it runs the root's hooks for a
session in a worktree of it, with the root's memory and the worktree's own
task, brings the project's AGENTS.md, and does nothing anywhere else.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

from tests.hook_shell import BASH

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = os.name == "nt"
EDITIONS = ("PHP Core", "Laravel", "Symfony", "Cms/wordpress")
SCRIPTS = ROOT / "PHP Core" / "memory-bank" / "scripts"
# The command of every hook in an edition's .codex/hooks.json.
LAUNCHER = ("sh -c 'd=$(pwd); until [ -f \"$d/.codex/hooks.json\" ]; do [ -n \"$d\" ] || { echo \"$1: no "
            ".codex/hooks.json at or above the working directory\" >&2; exit 127; }; d=${d%/*}; done; "
            "exec \"$d/.codex/hooks/$1\"' sh ")


def load(name):
    sys.path.insert(0, str(SCRIPTS))
    try:
        spec = importlib.util.spec_from_file_location(f"{name}_under_test", SCRIPTS / f"{name}.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    finally:
        sys.path.remove(str(SCRIPTS))
    return module


dispatch = load("codex_dispatch")
setup = load("codex_worktrees")


def git(folder, *args):
    return subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                           "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args],
                          cwd=folder, check=True, capture_output=True, text=True).stdout


@unittest.skipIf(BASH is None or WINDOWS, "Codex runs a hook's command with bash; not set up on Windows")
class Fixture(unittest.TestCase):
    # What a hook the dispatcher runs saw: its folder, the three roots and its input.
    ECHO = 'printf "%s|%s|%s|%s|%s" "$PWD" "$ACCELERATOR_HOME" "$ACCELERATOR_STATE_DIR" "$ACCELERATOR_PROJECT_DIR" "$(cat)"\n'

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="codex-worktrees-")
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.home = self.base / "codex-home"
        (self.home / "sessions" / "2026" / "10" / "26").mkdir(parents=True)
        self.transcript = self.home / "sessions" / "2026" / "10" / "26" / "rollout-session.jsonl"
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("CLAUDE", "CODEX", "ACCELERATOR_", "CONTEXT_", "GIT_"))}
        self.env.update(HOME=str(self.base / "home"), CODEX_HOME=str(self.home), PYTHONDONTWRITEBYTECODE="1")
        (self.base / "home").mkdir()

    def project(self, name="project", hooks=None, policy="# Policy\nRule.\n", wiring=None):
        """A project with Codex hooks wired as an edition wires them - one
        launcher per script, `wiring` maps an event to its scripts and their
        settings - and the dispatcher's scripts; `hooks` maps a script to its
        body."""
        root = self.base / name
        (root / "memory-bank" / "scripts").mkdir(parents=True)
        (root / ".codex" / "hooks").mkdir(parents=True)
        (root / "src" / "Billing").mkdir(parents=True)
        for script in ("codex_dispatch.py", "codex_dispatch.sh", "codex_worktrees.py", "claude_subfolders.py",
                       "claude_dispatch.py"):
            shutil.copy2(SCRIPTS / script, root / "memory-bank" / "scripts" / script)
        (root / "memory-bank" / "scripts" / "context.py").write_text("# the memory CLI\n", encoding="utf-8")
        hooks = hooks or {"echo.sh": self.ECHO}
        for hook, body in hooks.items():
            path = root / ".codex" / "hooks" / hook
            path.write_text("#!/bin/bash\n" + body, encoding="utf-8")
            path.chmod(0o755)
        wiring = wiring or {"SessionStart": [("echo.sh", {})]}
        (root / ".codex" / "hooks.json").write_text(json.dumps({"hooks": {
            event: [{"hooks": [{"type": "command", "command": LAUNCHER + script, **extra}]}
                    for script, extra in scripts]
            for event, scripts in wiring.items()}}, indent=2), encoding="utf-8")
        if policy is not None:
            (root / "AGENTS.md").write_text(policy, encoding="utf-8")
        git(root, "init", "-q")
        return root

    def committed(self, **options):
        """A project whose code is committed and whose installation is not,
        set up by a session in its root."""
        root = self.project(**options)
        (root / "src" / "Billing" / "Invoice.php").write_text("<?php\n", encoding="utf-8")
        git(root, "add", "src")
        git(root, "commit", "-qm", "base")
        self.setup_dispatcher(root)
        return root

    def setup_dispatcher(self, root, transcript=None, check=True):
        payload = {"session_id": "s", "transcript_path": str(transcript or self.transcript),
                   "hook_event_name": "SessionStart", "source": "startup", "cwd": str(root)}
        done = subprocess.run([sys.executable, str(root / "memory-bank" / "scripts" / "codex_worktrees.py")],
                              input=json.dumps(payload).encode(), env=self.env, cwd=root, capture_output=True)
        if check:
            self.assertEqual(done.returncode, 0, done.stderr)
        return done

    @property
    def dispatcher(self):
        return self.home / "ai-accelerator" / "ai-accelerator-dispatch.sh"

    def user_hooks(self):
        return json.loads((self.home / "hooks.json").read_text(encoding="utf-8"))

    def dispatched(self, hooks=None):
        """(event, matcher, name, the entry's other settings) of every dispatcher entry."""
        marker = shlex.quote(str(self.dispatcher)) + " "
        found = []
        for event, groups in (hooks or self.user_hooks()).get("hooks", {}).items():
            for group in groups:
                for hook in group["hooks"]:
                    if hook.get("command", "").startswith(marker):
                        found.append((event, group.get("matcher", ""), hook["command"][len(marker):],
                                      {key: value for key, value in hook.items() if key not in ("type", "command")}))
        return found

    def entry(self, name, event=None):
        """The command of the entry for `name`, as hooks.json holds it."""
        for found_event, groups in self.user_hooks()["hooks"].items():
            for group in groups:
                for hook in group["hooks"]:
                    if hook["command"].endswith(" " + name) and event in (None, found_event):
                        return hook["command"]
        raise AssertionError(f"no entry for {name}")

    def run_entry(self, name, start, payload=b"{}", env=None):
        """The entry's command as Codex 0.164 runs it: bash -c, in the session's folder."""
        return subprocess.run([BASH, "-c", self.entry(name)], input=payload, env=env or self.env, cwd=start,
                              capture_output=True)

    def assert_served(self, done, root, project, payload="{}"):
        self.assertEqual(done.returncode, 0, done.stderr)
        fields = done.stdout.decode().split("|", 4)
        self.assertEqual(len(fields), 5, done.stdout)
        where, home, state, served, given = fields
        self.assertEqual((where, served, given), (str(project), str(project), payload), done.stdout)
        self.assertEqual((home, state), (str(root), str(root)))

    def assert_silent(self, done):
        self.assertEqual((done.returncode, done.stdout, done.stderr), (0, b"", b""))


class DispatcherVersion(Fixture):
    # Raise DISPATCH_VERSION with any change to codex_dispatch.sh or .py,
    # then record the scripts' new fingerprint here.
    FINGERPRINTS = {1: "4fe155a8e29f21bf84325bb36aa8278f3555c115433edbf6500fbbccd69942c4"}

    def test_both_scripts_carry_the_version_and_a_change_raises_it(self):
        shell = (SCRIPTS / "codex_dispatch.sh").read_text(encoding="utf-8")
        self.assertIn(f"# dispatch-version: {dispatch.DISPATCH_VERSION} ", shell)
        digest = hashlib.sha256((SCRIPTS / "codex_dispatch.sh").read_bytes()
                                + (SCRIPTS / "codex_dispatch.py").read_bytes()).hexdigest()
        self.assertEqual(self.FINGERPRINTS.get(dispatch.DISPATCH_VERSION), digest,
                         "codex_dispatch changed: raise DISPATCH_VERSION and record this fingerprint")

    def test_a_newer_dispatcher_in_place_keeps_its_scripts_and_entries(self):
        root = self.project(wiring={"SessionStart": [("echo.sh", {"timeout": 30})]})
        folder = self.home / "ai-accelerator"
        folder.mkdir(mode=0o700)
        newer = f"DISPATCH_VERSION = {dispatch.DISPATCH_VERSION + 1}\n"
        (folder / "ai-accelerator-dispatch.py").write_text(newer, encoding="utf-8")
        (folder / "ai-accelerator-dispatch.sh").write_text("# newer\n", encoding="utf-8")
        written = {"hooks": {"SessionStart": [{"hooks": [{"type": "command", "timeout": 45,
                                                          "command": f"{shlex.quote(str(self.dispatcher))} echo.sh"}]}]}}
        (self.home / "hooks.json").write_text(json.dumps(written), encoding="utf-8")
        self.setup_dispatcher(root)
        self.assertEqual((folder / "ai-accelerator-dispatch.py").read_text(encoding="utf-8"), newer)
        self.assertEqual((folder / "ai-accelerator-dispatch.sh").read_text(encoding="utf-8"), "# newer\n")
        self.assertEqual((folder / "projects").read_text(encoding="utf-8").splitlines(), [str(root)])
        self.assertIn(("SessionStart", "", "echo.sh", {"timeout": 45}), self.dispatched(),
                      "the newer release's entry stays as it wrote it")
        (folder / "ai-accelerator-dispatch.py").write_text("DISPATCH_VERSION = 0\n", encoding="utf-8")
        self.setup_dispatcher(root)
        self.assertEqual((folder / "ai-accelerator-dispatch.py").read_bytes(),
                         (SCRIPTS / "codex_dispatch.py").read_bytes(), "an older one is replaced")
        self.assertIn(("SessionStart", "", "echo.sh", {"timeout": 30}), self.dispatched())


class Setup(Fixture):
    def test_a_root_session_writes_one_entry_per_project_hook_and_one_for_the_policy(self):
        wiring = {
            "SessionStart": [("echo.sh", {}), ("context.sh", {"additionalContextLimit": 6000})],
            "UserPromptSubmit": [("read.sh", {"additionalContextLimit": 4000, "statusMessage": "Reading"})],
            "PreToolUse": [("guard.sh", {"timeout": 30})],
        }
        root = self.project(hooks={name: "exit 0\n" for name in ("echo.sh", "context.sh", "read.sh", "guard.sh")},
                            wiring=wiring)
        self.setup_dispatcher(root)
        self.assertEqual(self.dispatched(), [
            ("SessionStart", "", "echo.sh", {}),
            ("SessionStart", "", "context.sh", {"additionalContextLimit": 6000}),
            ("SessionStart", "", "policy", {"timeout": 10, "additionalContextLimit": dispatch.POLICY_LIMIT}),
            ("UserPromptSubmit", "", "read.sh", {"additionalContextLimit": 4000, "statusMessage": "Reading"}),
            ("PreToolUse", "", "guard.sh", {"timeout": 30}),
        ])
        folder = self.home / "ai-accelerator"
        self.assertEqual((folder / "projects").read_text(encoding="utf-8").splitlines(), [str(root)])
        self.assertEqual((folder / "ai-accelerator-dispatch.sh").read_bytes(), (SCRIPTS / "codex_dispatch.sh").read_bytes())
        self.assertEqual(stat.S_IMODE((folder / "ai-accelerator-dispatch.sh").stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE(folder.stat().st_mode), 0o700)

    def test_every_editions_hooks_get_an_entry_each(self):
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                hooks = json.loads((ROOT / edition / ".codex" / "hooks.json").read_text(encoding="utf-8"))["hooks"]
                scripts = [(event, re.search(r"(\S+\.sh)$", hook["command"]).group(1))
                           for event, groups in hooks.items() for group in groups for hook in group["hooks"]]
                entries = setup.wanted_entries(ROOT / edition / ".codex" / "hooks.json", self.dispatcher)
                marker = shlex.quote(str(self.dispatcher)) + " "
                named = [(event, group["hooks"][0]["command"][len(marker):]) for event, group in entries]
                self.assertEqual(named, scripts + [("SessionStart", "policy")])

    def test_the_persons_own_hooks_keep_their_place_and_trust_stays_theirs(self):
        root = self.project()
        own = {"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "/usr/bin/true"}]}],
                         "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "graphify check"}]}]},
               "note": "kept"}
        (self.home / "hooks.json").write_text(json.dumps(own), encoding="utf-8")
        config = ('model = "gpt-5.5"\n\n[hooks.state."' + str(self.home / "hooks.json")
                  + ':pre_tool_use:0:0"]\ntrusted_hash = "sha256:abc"\n')
        (self.home / "config.toml").write_text(config, encoding="utf-8")
        self.setup_dispatcher(root)
        hooks = self.user_hooks()
        self.assertEqual(hooks["note"], "kept")
        # Codex keys a hook's approval by its place: the person's stay first.
        self.assertEqual(hooks["hooks"]["SessionStart"][0], own["hooks"]["SessionStart"][0])
        self.assertEqual(hooks["hooks"]["PreToolUse"], own["hooks"]["PreToolUse"])
        self.assertEqual([name for _, _, name, _ in self.dispatched()], ["echo.sh", "policy"])
        self.assertEqual((self.home / "config.toml").read_text(encoding="utf-8"), config,
                         "no approval is written for the person")

    def test_a_second_session_and_a_second_project_add_nothing_twice(self):
        first = self.project("first")
        second = self.project("second")
        self.setup_dispatcher(first)
        written = (self.home / "hooks.json").read_bytes()
        changed = (self.home / "hooks.json").stat().st_mtime_ns
        time.sleep(0.01)
        self.setup_dispatcher(first)
        self.setup_dispatcher(second)
        self.assertEqual((self.home / "hooks.json").read_bytes(), written)
        self.assertEqual((self.home / "hooks.json").stat().st_mtime_ns, changed, "an unchanged file is not rewritten")
        self.assertEqual((self.home / "ai-accelerator" / "projects").read_text(encoding="utf-8").splitlines(),
                         [str(first), str(second)])
        shutil.rmtree(first / "memory-bank")
        self.setup_dispatcher(second)
        self.assertEqual((self.home / "ai-accelerator" / "projects").read_text(encoding="utf-8").splitlines(),
                         [str(second)], "a project removed since is dropped")

    def test_nothing_is_written_for_a_hook_another_program_started(self):
        root = self.project()
        claude = self.base / "claude" / "projects" / "-project" / "session.jsonl"
        for transcript in (claude, self.base / "elsewhere.jsonl", "relative.jsonl"):
            with self.subTest(transcript=transcript):
                self.setup_dispatcher(root, transcript=transcript)
                self.assertFalse((self.home / "ai-accelerator").exists())
                self.assertFalse((self.home / "hooks.json").exists())

    def test_nothing_for_a_person_who_keeps_hooks_in_config_toml(self):
        # Codex loads both forms and warns at every start that a layer has two.
        root = self.project()
        configs = ['[[hooks.Stop]]\n[[hooks.Stop.hooks]]\ntype = "command"\ncommand = "true"\n',
                   '[hooks]\nSessionStart = [{ hooks = [{ type = "command", command = "true" }] }]\n',
                   'hooks.PreToolUse = []\n']
        if sys.version_info >= (3, 11):
            # One tomllib cannot parse; Codex refuses it too.
            configs.append('not = [valid toml\n')
        for config in configs:
            with self.subTest(config=config):
                (self.home / "config.toml").write_text(config, encoding="utf-8")
                self.setup_dispatcher(root)
                self.assertFalse((self.home / "hooks.json").exists())
        (self.home / "config.toml").write_text('[hooks.state."x:stop:0:0"]\ntrusted_hash = "sha256:0"\n',
                                               encoding="utf-8")
        self.setup_dispatcher(root)
        self.assertTrue(self.dispatched(), "approvals alone are not hooks")

    def test_config_toml_is_read_the_same_without_tomllib(self):
        cases = {
            '[[hooks.Stop]]\n[[hooks.Stop.hooks]]\ntype = "command"\n': True,
            '[hooks]\nSessionStart = []\n': True,
            '[ "hooks" . "PreToolUse" ]\n': True,
            'hooks.Stop = []\n': True,
            '[hooks]\nstate = {}\n': False,
            '[hooks.state]\n\n[hooks.state."/x/hooks.json:stop:0:0"]\ntrusted_hash = "sha256:1"\n': False,
            '[projects."/x"]\ntrust_level = "trusted"\nhooks_note = 1\n': False,
            '[features]\nhooks = true\n': False,
            '': False,
        }
        config = self.base / "config.toml"
        for text, expected in cases.items():
            config.write_text(text, encoding="utf-8")
            with self.subTest(text=text):
                with mock.patch.dict(sys.modules, {"tomllib": None}):
                    self.assertIs(setup.toml_hooks(config), expected)
                if sys.version_info >= (3, 11):
                    self.assertIs(setup.toml_hooks(config), expected)
        self.assertFalse(setup.toml_hooks(self.base / "missing.toml"))

    def test_hooks_it_cannot_read_as_an_object_are_left_as_they_are(self):
        root = self.project()
        for text in ("[]", "{not json", '{"hooks": []}', '{"hooks": {"Stop": {}}}'):
            with self.subTest(text=text):
                (self.home / "hooks.json").write_text(text, encoding="utf-8")
                self.setup_dispatcher(root)
                self.assertEqual((self.home / "hooks.json").read_text(encoding="utf-8"), text)

    def test_a_file_named_off_turns_it_off_for_good(self):
        root = self.committed()
        worktree = self.base / "feature"
        git(root, "worktree", "add", "-q", str(worktree), "-b", "feature/x")
        (self.home / "hooks.json").unlink()
        (self.home / "ai-accelerator" / "off").write_text("", encoding="utf-8")
        self.setup_dispatcher(root)
        self.assertFalse((self.home / "hooks.json").exists())
        done = subprocess.run([BASH, "-c", f"{shlex.quote(str(self.dispatcher))} echo.sh"], input=b"{}",
                              env=self.env, cwd=worktree, capture_output=True)
        self.assert_silent(done)

    def test_a_linked_hooks_file_is_written_where_it_lives(self):
        root = self.project()
        dotfiles = self.base / "dotfiles" / "codex-hooks.json"
        dotfiles.parent.mkdir()
        dotfiles.write_text('{"hooks": {}}', encoding="utf-8")
        (self.home / "hooks.json").symlink_to(dotfiles)
        self.setup_dispatcher(root)
        self.assertTrue((self.home / "hooks.json").is_symlink())
        self.assertTrue(self.dispatched(json.loads(dotfiles.read_text(encoding="utf-8"))))

    def test_a_folder_that_is_a_link_gets_nothing(self):
        root = self.project()
        elsewhere = self.base / "elsewhere"
        elsewhere.mkdir()
        (self.home / "ai-accelerator").symlink_to(elsewhere, target_is_directory=True)
        self.setup_dispatcher(root)
        self.assertEqual(list(elsewhere.iterdir()), [])
        self.assertFalse((self.home / "hooks.json").exists())

    def test_a_project_without_codex_hooks_or_a_person_without_codex_gets_nothing(self):
        root = self.project()
        (root / ".codex" / "hooks.json").unlink()
        self.setup_dispatcher(root)
        self.assertFalse((self.home / "ai-accelerator").exists())
        root = self.project("other")
        shutil.rmtree(self.home)
        done = subprocess.run([sys.executable, str(root / "memory-bank" / "scripts" / "codex_worktrees.py"),
                               "--install"], env=self.env, cwd=root, capture_output=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertFalse(self.home.exists())


class Dispatcher(Fixture):
    """Codex runs the entries in every session: they serve a worktree of a
    listed root and are silent everywhere else."""

    def test_a_worktree_in_the_project_or_beside_it_runs_the_roots_hook_for_it(self):
        root = self.committed()
        inner = root / ".claude" / "worktrees" / "refunds"
        beside = self.home / "worktrees" / "b3c4" / "project"
        git(root, "worktree", "add", "-q", str(inner), "-b", "feature/SHOP-9-refunds")
        git(root, "worktree", "add", "-q", str(beside), "-b", "feature/SHOP-10-credit-notes")
        for project in (inner, beside):
            for start in (project, project / "src" / "Billing"):
                with self.subTest(start=start):
                    self.assert_served(self.run_entry("echo.sh", start, b'{"hook": 1}'), root, project,
                                       '{"hook": 1}')
        self.assertFalse((inner / "memory-bank").exists() or (beside / "memory-bank").exists(),
                         "nothing is written into a worktree")

    def test_the_root_the_folders_below_it_and_other_projects_are_left_to_codex(self):
        # Codex runs the project's own hooks in the root and below it.
        root = self.committed()
        other = self.base / "other"
        other.mkdir()
        git(other, "init", "-q")
        for start in (root, root / "src" / "Billing", other, self.base):
            with self.subTest(start=start):
                self.assert_silent(self.run_entry("echo.sh", start))

    def test_a_project_in_a_folder_of_a_monorepo_is_that_folder_of_its_worktree(self):
        monorepo = self.base / "monorepo"
        monorepo.mkdir()
        git(monorepo, "init", "-q")
        root = self.project("monorepo/backend")
        shutil.rmtree(root / ".git")
        (root / "src" / "Billing" / "Invoice.php").write_text("<?php\n", encoding="utf-8")
        git(monorepo, "add", "backend/src")
        git(monorepo, "commit", "-qm", "base")
        self.setup_dispatcher(root)
        worktree = self.base / "monorepo-feature"
        git(monorepo, "worktree", "add", "-q", str(worktree), "-b", "feature/x")
        self.assert_served(self.run_entry("echo.sh", worktree / "backend" / "src"), root, worktree / "backend")
        self.assert_silent(self.run_entry("echo.sh", worktree))

    def test_a_worktree_git_records_by_a_relative_path(self):
        root = self.committed()
        inner = root / ".claude" / "worktrees" / "relative"
        beside = self.base / "project-relative"
        for path, branch in ((inner, "feature/relative"), (beside, "feature/beside")):
            git(root, "-c", "worktree.useRelativePaths=true", "worktree", "add", "-q", str(path), "-b", branch)
        if os.path.isabs((inner / ".git").read_text(encoding="utf-8").strip()[len("gitdir: "):]):
            self.skipTest("this Git writes worktree links absolute")
        for project in (inner, beside):
            with self.subTest(project=project):
                self.assert_served(self.run_entry("echo.sh", project / "src" / "Billing"), root, project)

    def test_a_session_a_launcher_lends_an_accelerator_to_is_left_to_it(self):
        # The Harness's attached launch passes its own hooks with -c.
        root = self.committed()
        inner = root / ".claude" / "worktrees" / "refunds"
        git(root, "worktree", "add", "-q", str(inner), "-b", "feature/SHOP-9-refunds")
        env = dict(self.env, ACCELERATOR_HOME=str(self.base / "clone" / "PHP Core"))
        self.assert_silent(self.run_entry("echo.sh", inner, env=env))

    def test_a_worktree_with_an_installation_of_its_own_or_of_another_repository_is_left_alone(self):
        root = self.committed()
        own = root / ".claude" / "worktrees" / "own"
        git(root, "worktree", "add", "-q", str(own), "-b", "feature/own")
        (own / "memory-bank" / "scripts").mkdir(parents=True)
        (own / "memory-bank" / "scripts" / "context.py").write_text("# its own runtime\n", encoding="utf-8")
        other = self.base / "other"
        (other / "src").mkdir(parents=True)
        (other / "src" / "a.php").write_text("<?php\n", encoding="utf-8")
        git(other, "init", "-q")
        git(other, "add", ".")
        git(other, "commit", "-qm", "base")
        foreign = self.base / "other-feature"
        git(other, "worktree", "add", "-q", str(foreign), "-b", "feature/x")
        # A .git file naming a worktree Git does not know, and a submodule's.
        forged = self.base / "forged"
        forged.mkdir()
        (forged / ".git").write_text(f"gitdir: {root / '.git' / 'worktrees' / 'ghost'}\n", encoding="utf-8")
        module = root / "src" / "module"
        module.mkdir()
        (module / ".git").write_text("gitdir: ../../.git/modules/module\n", encoding="utf-8")
        for start in (own, own / "src" / "Billing", foreign, foreign / "src", forged, module):
            with self.subTest(start=start):
                self.assert_silent(self.run_entry("echo.sh", start))

    def test_a_hook_the_root_no_longer_has_runs_nothing(self):
        root = self.committed()
        inner = root / ".claude" / "worktrees" / "refunds"
        git(root, "worktree", "add", "-q", str(inner), "-b", "feature/SHOP-9-refunds")
        (root / ".codex" / "hooks" / "echo.sh").unlink()
        self.assert_silent(self.run_entry("echo.sh", inner))
        for name in ("../echo.sh", ".hidden", ""):
            done = subprocess.run([BASH, str(self.dispatcher), name], input=b"{}", env=self.env, cwd=inner,
                                  capture_output=True)
            self.assert_silent(done)

    def test_the_policy_comes_whole_at_a_new_start_and_not_again_on_resume(self):
        text = "# Policy\n\n" + "".join(f"## Rule {number}\nKeep invoices per tenant — № {number}.\n"
                                        for number in range(400))
        root = self.committed(policy=text)
        inner = root / ".claude" / "worktrees" / "refunds"
        git(root, "worktree", "add", "-q", str(inner), "-b", "feature/SHOP-9-refunds")
        for source in ("startup", "clear", "compact"):
            with self.subTest(source=source):
                done = self.run_entry("policy", inner / "src", json.dumps({"source": source}).encode(),
                                      env=dict(self.env, LC_ALL="C", LANG="C"))
                self.assertEqual(done.returncode, 0, done.stderr)
                output = json.loads(done.stdout)["hookSpecificOutput"]
                self.assertEqual(output["hookEventName"], "SessionStart")
                self.assertEqual(output["additionalContext"],
                                 f"Contents of {root / 'AGENTS.md'} (the project's policy):\n\n{text}")
        self.assertEqual(self.run_entry("policy", inner, b'{"source": "resume"}').stdout, b"")
        self.assertEqual(self.run_entry("policy", root / "src", b'{"source": "startup"}').stdout, b"",
                         "Codex reads the root's own AGENTS.md there")
        (inner / "AGENTS.md").write_text("# The worktree's own\n", encoding="utf-8")
        self.assertEqual(self.run_entry("policy", inner, b'{"source": "startup"}').stdout, b"")


@unittest.skipIf(BASH is None or WINDOWS, "Codex runs a hook's command with bash; not set up on Windows")
class SaveInWorktree(unittest.TestCase):
    """The root's Codex hooks, run by the dispatcher in a worktree of an
    installed project: the Stop's save step names the roots, so the agent's
    save - a command Codex runs in the session, which no hook can give
    variables to - checks the worktree's files and saves into the root's
    Project Brain under the worktree's branch."""

    BRANCH = "feature/SHOP-9-refunds"

    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="codex-worktree-save-")
        base = Path(cls.temporary.name).resolve()
        cls.main = base / "shop"
        (cls.main / "src").mkdir(parents=True)
        (cls.main / "composer.json").write_text('{"name": "acme/shop"}\n', encoding="utf-8")
        (cls.main / "src" / "Invoice.php").write_text("<?php\nfinal class Invoice {}\n", encoding="utf-8")
        git(cls.main, "init", "-q", "-b", "feature/SHOP-7-invoices")
        git(cls.main, "add", ".")
        git(cls.main, "commit", "-qm", "Shop")
        cls.home = base / "codex-home"
        (cls.home / "sessions").mkdir(parents=True)
        cls.env = {key: value for key, value in os.environ.items()
                   if not key.startswith(("CLAUDE", "CODEX", "ACCELERATOR_", "CONTEXT_", "GIT_"))}
        cls.env.update(PYTHONDONTWRITEBYTECODE="1", HOME=str(base), CODEX_HOME=str(cls.home), TMPDIR=str(base))
        done = subprocess.run([sys.executable, str(ROOT / "scripts" / "install_accelerator.py"), "--edition",
                               "PHP Core", "--target", str(cls.main), "--merge-existing"],
                              cwd=ROOT, env=cls.env, capture_output=True, text=True, stdin=subprocess.DEVNULL)
        if done.returncode:
            raise AssertionError(done.stdout[-2000:] + done.stderr[-2000:])
        transcript = cls.home / "sessions" / "root.jsonl"
        transcript.write_text("", encoding="utf-8")
        done = subprocess.run([sys.executable, str(cls.main / "memory-bank" / "scripts" / "codex_worktrees.py")],
                              input=json.dumps({"transcript_path": str(transcript)}), env=cls.env, cwd=cls.main,
                              capture_output=True, text=True)
        if done.returncode:
            raise AssertionError(done.stderr)
        cls.worktree = cls.main / ".claude" / "worktrees" / "refunds"
        git(cls.main, "worktree", "add", "-q", str(cls.worktree), "-b", cls.BRANCH)
        # The branch's own file, which the main checkout does not have.
        (cls.worktree / "src" / "Refund.php").write_text("<?php\nfinal class Refund {}\n", encoding="utf-8")
        git(cls.worktree, "add", "src/Refund.php")
        git(cls.worktree, "commit", "-qm", "Refunds")
        cls.entries = json.loads((cls.home / "hooks.json").read_text(encoding="utf-8"))["hooks"]

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def run_hook(self, event, script, payload):
        [command] = [hook["command"] for group in self.entries[event] for hook in group["hooks"]
                     if hook["command"].endswith(" " + script)]
        done = subprocess.run([BASH, "-c", command], input=json.dumps(payload), env=self.env, cwd=self.worktree,
                              capture_output=True, text=True, timeout=120)
        self.assertEqual(done.returncode, 0, done.stderr)
        return done.stdout

    def test_the_save_step_names_the_roots_and_the_save_lands_under_the_worktrees_branch(self):
        transcript = self.home / "sessions" / "worktree.jsonl"
        transcript.write_text("", encoding="utf-8")
        session = {"session_id": "codex-worktree", "turn_id": "turn-1", "transcript_path": str(transcript),
                   "cwd": str(self.worktree), "model": "gpt-5.5", "permission_mode": "default"}
        capsule = self.run_hook("UserPromptSubmit", "working-memory-read.sh",
                                dict(session, hook_event_name="UserPromptSubmit", prompt="How are refunds numbered?"))
        self.assertRegex(capsule, re.compile(rf"^working: ({re.escape(self.BRANCH)}|not recorded yet)", re.M))
        stop = self.run_hook("Stop", "working-memory-write.sh",
                             dict(session, hook_event_name="Stop", stop_hook_active=False,
                                  last_assistant_message="Refunds reuse the invoice number."))
        reason = json.loads(stop)["reason"]
        found = re.search(r"\((cd .*?--json)\)", reason)
        self.assertIsNotNone(found, reason)
        command = found.group(1)
        words = shlex.split(command)
        self.assertIn(f"ACCELERATOR_HOME={self.main}", words)
        self.assertIn(f"ACCELERATOR_STATE_DIR={self.main}", words)
        self.assertIn(f"ACCELERATOR_PROJECT_DIR={self.worktree}", words)
        result = json.dumps({"progress": "Refunds checked.", "verified": True, "learnings": [
            {"type": "finding", "title": "Refunds reuse the invoice number",
             "consequence": "A refund cites the invoice it reverses.", "sources": ["src/Refund.php"]}]})
        # Without the roots the command serves the main checkout, which has
        # no such file: the worktree's learning is refused.
        bare = re.sub(r"ACCELERATOR_\w+=\S+ ", "", command)
        refused = subprocess.run([BASH, "-c", bare], input=result, env=self.env, cwd=self.worktree,
                                 capture_output=True, text=True, timeout=120)
        self.assertNotEqual(refused.returncode, 0, refused.stdout)
        saved = subprocess.run([BASH, "-c", command], input=result, env=self.env, cwd=self.worktree,
                               capture_output=True, text=True, timeout=120)
        self.assertEqual(saved.returncode, 0, saved.stdout + saved.stderr)
        self.assertEqual([record["state"] for record in json.loads(saved.stdout)["records"]], ["created"],
                         saved.stdout)
        self.run_hook("Stop", "working-memory-write.sh",
                      dict(session, hook_event_name="Stop", stop_hook_active=True, last_assistant_message="Saved."))
        journal = [json.loads(line) for line in
                   (self.main / "memory-bank" / "local" / "native-results.ndjson").read_text().splitlines()]
        self.assertIn(("stop", "skipped", "recovery-saved"),
                      [(line["stage"], line["event"], line.get("code")) for line in journal])
        tasks = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                 for path in (self.main / "project-brain" / "dynamic" / "tasks").glob("*.md")]
        self.assertTrue(any(task["external_id"] == self.BRANCH and task.get("progress") for task in tasks))
        findings = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                    for path in (self.main / "project-brain" / "dynamic" / "findings").glob("*.md")]
        self.assertIn("src/Refund.php", [source["path"] for finding in findings
                                         for source in finding.get("source_fingerprints") or []])
        self.assertEqual(git(self.worktree, "status", "--porcelain", "--ignored"), "")

    def test_the_root_and_claude_code_keep_the_step_as_it_was(self):
        # Claude Code's commands get the roots from the dispatcher's
        # environment entry; the root's own sessions need none. Run in the
        # worktree without them, the runtime finds the worktree by itself
        # (workspace_roots.worktree_project), and Codex's step names the roots.
        program = ("import sys\nfrom pathlib import Path\nsys.path.insert(0, sys.argv[1])\nimport native_results\n"
                   "print(native_results._instruction(Path(sys.argv[2]), 'a' * 64, {'token': '0' * 32}, "
                   "host=sys.argv[3]))\n")
        shared = {"ACCELERATOR_HOME": str(self.main), "ACCELERATOR_STATE_DIR": str(self.main),
                  "ACCELERATOR_PROJECT_DIR": str(self.worktree)}
        for host, layout, where, named in (("codex", shared, self.worktree, True),
                                           ("claude", shared, self.worktree, False),
                                           ("codex", {}, self.main, False), ("codex", {}, self.worktree, True),
                                           ("cursor", shared, self.worktree, False)):
            with self.subTest(host=host, shared=bool(layout), where=where.name):
                done = subprocess.run([sys.executable, "-c", program, str(self.main / "memory-bank" / "scripts"),
                                       str(self.main), host], env=dict(self.env, **layout), cwd=where,
                                      capture_output=True, text=True, timeout=60)
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertIn("native-result", done.stdout)
                self.assertEqual("ACCELERATOR_PROJECT_DIR=" in done.stdout, named, done.stdout)


@unittest.skipIf(BASH is None or WINDOWS, "Codex runs a hook's command with bash; not set up on Windows")
class RootSession(unittest.TestCase):
    """The Codex copy of local-context.sh sets the dispatcher up; the Claude
    Code and Cursor copies keep setting up Claude Code's."""

    def test_each_editions_codex_start_hook_sets_up_codex_and_only_in_the_root(self):
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                codex = (ROOT / edition / ".codex" / "hooks" / "local-context.sh").read_text(encoding="utf-8")
                self.assertIn('[ "$PROJECT_DIR" = "$ROOT_DIR" ]', codex)
                self.assertIn('"$ROOT_DIR/memory-bank/scripts/codex_worktrees.py" > /dev/null 2>&1 &', codex)
                self.assertNotIn("claude_subfolders.py", codex)
                for tool in (".claude", ".cursor"):
                    other = (ROOT / edition / tool / "hooks" / "local-context.sh").read_text(encoding="utf-8")
                    self.assertIn("claude_subfolders.py", other)
                    self.assertNotIn("codex_worktrees.py", other)

    def test_a_codex_session_in_the_root_writes_the_entries(self):
        with tempfile.TemporaryDirectory(prefix="codex-root-session-") as temporary:
            base = Path(temporary).resolve()
            root = base / "shop"
            for relative in (".codex", "memory-bank/scripts", "memory-bank/templates", "project-brain/config",
                             "project-brain/schemas", "project-brain/templates"):
                shutil.copytree(ROOT / "PHP Core" / relative, root / relative,
                                ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
            for name in ("AGENTS.md", "VERSION"):
                shutil.copy2(ROOT / "PHP Core" / name, root / name)
            (root / "src").mkdir()
            git(root, "init", "-q")
            home = base / "codex-home"
            (home / "sessions").mkdir(parents=True)
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith(("CLAUDE", "CODEX", "ACCELERATOR_", "CONTEXT_", "GIT_"))}
            env.update(HOME=str(base), CODEX_HOME=str(home), PYTHONDONTWRITEBYTECODE="1")
            payload = json.dumps({"session_id": "root", "transcript_path": str(home / "sessions" / "root.jsonl"),
                                  "hook_event_name": "SessionStart", "source": "startup", "cwd": str(root)})
            done = subprocess.run([BASH, str(root / ".codex" / "hooks" / "local-context.sh")], input=payload.encode(),
                                  env=env, cwd=root, capture_output=True, timeout=60)
            self.assertEqual(done.returncode, 0, done.stderr)
            hooks = home / "hooks.json"
            deadline = time.time() + 30
            while time.time() < deadline and not hooks.exists():
                time.sleep(0.2)
            marker = shlex.quote(str(home / "ai-accelerator" / "ai-accelerator-dispatch.sh")) + " "
            commands = [hook["command"] for groups in json.loads(hooks.read_text(encoding="utf-8"))["hooks"].values()
                        for group in groups for hook in group["hooks"]]
            self.assertIn(marker + "local-context.sh", commands)
            self.assertIn(marker + "policy", commands)
            self.assertFalse((home / "config.toml").exists(), "Codex's config is not touched")


@unittest.skipIf(WINDOWS, "a pseudo-terminal is POSIX's")
class InstallTime(unittest.TestCase):
    """A person installing in a terminal gets the dispatcher at once; a
    scripted install leaves it to the project's first Codex session in its root."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="codex-install-")
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.home = self.base / "codex-home"
        self.home.mkdir()
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("CLAUDE", "CODEX", "ACCELERATOR_", "CONTEXT_", "GIT_"))}
        self.env.update(HOME=str(self.base / "home"), CODEX_HOME=str(self.home),
                        CLAUDE_CONFIG_DIR=str(self.base / "claude"), PYTHONDONTWRITEBYTECODE="1")
        (self.base / "home").mkdir()
        self.target = self.base / "shop"
        self.target.mkdir()
        git(self.target, "init", "-q")

    def install(self, terminal):
        command = [sys.executable, str(ROOT / "scripts" / "install_accelerator.py"), "--edition", "PHP Core",
                   "--target", str(self.target)]
        if not terminal:
            done = subprocess.run(command, env=self.env, cwd=ROOT, capture_output=True, text=True,
                                  stdin=subprocess.DEVNULL, timeout=300)
            return done.returncode, done.stdout + done.stderr
        import pty
        leader, follower = pty.openpty()
        process = subprocess.Popen(command, stdin=follower, stdout=follower, stderr=follower, env=self.env, cwd=ROOT)
        os.close(follower)
        output = b""
        while True:
            try:
                chunk = os.read(leader, 65536)
            except OSError:
                break
            if not chunk:
                break
            output += chunk
        os.close(leader)
        return process.wait(timeout=300), output.decode("utf-8", "replace")

    def test_a_person_installing_in_a_terminal_gets_the_dispatcher_at_once(self):
        code, output = self.install(terminal=True)
        self.assertEqual(code, 0, output[-2000:])
        self.assertIn("CODEX\tworktrees\t" + str(self.home / "ai-accelerator"), output)
        self.assertEqual((self.home / "ai-accelerator" / "projects").read_text(encoding="utf-8").splitlines(),
                         [str(self.target.resolve())])
        self.assertIn("local-context.sh", (self.home / "hooks.json").read_text(encoding="utf-8"))
        self.assertFalse((self.home / "config.toml").exists())

    def test_a_scripted_install_leaves_it_to_the_first_session_in_the_root(self):
        code, output = self.install(terminal=False)
        self.assertEqual(code, 0, output[-2000:])
        self.assertNotIn("CODEX\tworktrees", output)
        self.assertFalse((self.home / "ai-accelerator").exists())
        self.assertFalse((self.home / "hooks.json").exists())


if __name__ == "__main__":
    unittest.main()
