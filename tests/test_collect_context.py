#!/usr/bin/env python3
"""Contract tests for scripts/collect_context.py.

Two kinds of test live here. The first kind runs everywhere, including CI,
and needs no `code2prompt` binary: it pins the argv this script builds and
the boundary it must not cross. The second kind actually invokes the binary
and is skipped when it is absent — CI has no Rust toolchain and this tool is
never a blocking gate.

The load-bearing one is test_containment. code2prompt is a developer-local
convenience; the moment an edition, an inventory or an installed hook refers
to it, every consuming project inherits a dependency it never asked for.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "collect_context.py"

sys.path.insert(0, str(ROOT / "scripts"))
import collect_context as cc  # noqa: E402


def namespace(**overrides) -> argparse.Namespace:
    """A parsed-args stand-in with the parser's defaults."""
    defaults = dict(scope="tooling", edition=None, include=[], exclude=[],
                    base="origin/main", format="markdown", encoding="cl100k",
                    output=None, stdout=False, dry_run=False, top=0,
                    with_task=False, with_mirrors=False)
    defaults.update(overrides)
    return argparse.Namespace(**defaults)


def binary_available() -> bool:
    name = os.environ.get(cc.BIN_ENV) or cc.DEFAULT_BIN
    return shutil.which(name) is not None


class TestContainment(unittest.TestCase):
    """The tool must not leak out of the repository root."""

    def test_containment(self):
        proc = subprocess.run(
            ["git", "grep", "-lI", "-i", "code2prompt", "--",
             "Laravel/", "Symfony/", "PHP Core/", "Infrastructure-Creator/",
             "install/"],
            cwd=ROOT, capture_output=True, text=True)
        # git grep exits 1 when nothing matched, which is the passing case.
        hits = [line for line in proc.stdout.splitlines() if line.strip()]
        self.assertEqual(hits, [], msg=(
            "code2prompt is referenced inside an edition or the installer. It "
            "is an optional developer tool: nothing that ships to a consuming "
            "project may depend on it."))

    def test_not_in_installer_inventories(self):
        for inventory in sorted((ROOT / "install" / "inventories").glob("*.json")):
            text = inventory.read_text(encoding="utf-8")
            self.assertNotIn("collect_context", text, f"{inventory.name} ships the tool")
            self.assertNotIn("code2prompt", text, f"{inventory.name} ships the tool")


class TestInvocation(unittest.TestCase):
    """The two ways in: `/collect` in Claude Code, the file in a shell."""

    COMMAND = ROOT / ".claude" / "commands" / "collect.md"
    WRAPPER = ROOT / "collect"

    def test_wrapper_is_executable_and_delegates(self):
        self.assertTrue(self.WRAPPER.is_file(), "missing ./collect")
        self.assertTrue(os.access(self.WRAPPER, os.X_OK), "./collect is not executable")
        text = self.WRAPPER.read_text(encoding="utf-8")
        self.assertTrue(text.startswith("#!/bin/sh\n"))
        self.assertIn("scripts/collect_context.py", text)
        self.assertIn("exec ", text, "the wrapper must exec, so the exit status "
                                     "is the script's own")

    def test_wrapper_forwards_arguments_and_exit_status(self):
        listing = subprocess.run([str(self.WRAPPER)], cwd=ROOT,
                                 capture_output=True, text=True)
        self.assertEqual(listing.returncode, 0, listing.stderr)
        self.assertIn("Scopes", listing.stdout)
        # An argument the script rejects must come back as the script's failure,
        # not as a shell success.
        rejected = subprocess.run([str(self.WRAPPER), "tooling", "--edition", "Laravel"],
                                  cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(rejected.returncode, 1)
        self.assertIn("does not take --edition", rejected.stderr)

    def test_wrapper_is_covered_by_the_shell_lint_gate(self):
        """It has no .sh extension, so CI must list it by name."""
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
        lint_lines = [line for line in workflow.splitlines()
                      if "git ls-files" in line and "'*.sh'" in line]
        self.assertTrue(lint_lines, "no shell lint step found in the workflow")
        for line in lint_lines:
            self.assertIn("'collect'", line,
                          f"shell lint step skips ./collect: {line.strip()}")

    def test_script_is_executable_with_a_shebang(self):
        self.assertTrue(os.access(SCRIPT, os.X_OK),
                        "scripts/collect_context.py is not executable")
        with SCRIPT.open(encoding="utf-8") as handle:
            self.assertEqual(handle.readline().rstrip(), "#!/usr/bin/env python3")

    def test_slash_command_exists_at_the_repository_root(self):
        # At the root, not inside an edition: an installed accelerator must not
        # carry a command for a tool it does not ship.
        self.assertTrue(self.COMMAND.is_file(), f"missing {self.COMMAND}")
        for edition in cc.EDITIONS:
            stray = ROOT / edition / ".claude" / "commands" / "collect.md"
            self.assertFalse(stray.exists(), f"{stray} would ship the tool")

    def test_slash_command_invokes_the_script(self):
        text = self.COMMAND.read_text(encoding="utf-8")
        self.assertIn("scripts/collect_context.py", text)
        self.assertIn("$ARGUMENTS", text)
        # The injected output is capped: a bundle must never land in the
        # session context, which is the whole point of writing it to a file.
        self.assertIn("tail -", text)

    def test_a_bare_invocation_lists_the_scopes(self):
        env = dict(os.environ, **{cc.BIN_ENV: "/nonexistent/code2prompt"})
        proc = subprocess.run([sys.executable, str(SCRIPT)], cwd=ROOT,
                              capture_output=True, text=True, env=env)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("Scopes", proc.stdout)


class TestScopeDefinitions(unittest.TestCase):

    def test_every_scope_is_documented(self):
        for name, spec in cc.SCOPES.items():
            self.assertTrue(spec["summary"], f"{name} has no summary")
            self.assertIn("needs_edition", spec, f"{name} does not declare needs_edition")
            self.assertIn("root", spec, f"{name} does not declare a root")

    def test_only_computed_scopes_may_start_empty(self):
        computed = {"diff", "custom"}
        for name, spec in cc.SCOPES.items():
            if name in computed:
                continue
            self.assertTrue(spec["include"], f"{name} declares no include patterns")

    def test_top_level_wants_are_anchored(self):
        """code2prompt's `*` crosses separators; only `./<literal>` anchors."""
        _, includes, _, _ = cc.resolve_scope(namespace(scope="docs"))
        anchored = [p for p in includes if p.startswith("./")]
        self.assertIn("./CHANGELOG.md", anchored)
        for pattern in anchored:
            self.assertNotIn("*", pattern, "an anchored literal cannot hold a glob")
            self.assertEqual(pattern.count("/"), 1, f"{pattern} is not top-level")

    def test_include_local_is_expanded_not_passed_through(self):
        for name, spec in cc.SCOPES.items():
            if not spec.get("include_local"):
                continue
            self.assertNotIn("*", " ".join(spec["include"]).replace("**", ""),
                             f"{name} mixes a bare star into the recursive list")

    def test_edition_scopes_target_the_edition_directory(self):
        for name, spec in cc.SCOPES.items():
            if not spec["needs_edition"]:
                continue
            self.assertEqual(spec["root"], "{edition}", f"{name} has a fixed root")


class TestExcludeContract(unittest.TestCase):

    def resolve(self, **overrides):
        return cc.resolve_scope(namespace(**overrides))

    def test_git_is_always_excluded(self):
        for scope, extra in (("tooling", {}), ("docs", {}),
                             ("skills", {"edition": "Laravel"})):
            _, _, excludes, _ = self.resolve(scope=scope, **extra)
            self.assertIn(".git/**", excludes)
            self.assertIn("**/.git/**", excludes)

    def test_client_data_excluded_by_default(self):
        _, _, excludes, _ = self.resolve(scope="edition", edition="Laravel")
        self.assertIn("**/Task/**", excludes)

    def test_client_data_requires_an_explicit_optin(self):
        _, _, excludes, _ = self.resolve(scope="edition", edition="Laravel",
                                         with_task=True)
        self.assertNotIn("**/Task/**", excludes)

    def test_generated_mirrors_excluded_by_default(self):
        _, _, excludes, _ = self.resolve(scope="edition", edition="Symfony")
        for pattern in cc.MIRROR_EXCLUDE:
            self.assertIn(pattern, excludes)

    def test_with_mirrors_adds_the_trees_to_canon_anchored_scopes(self):
        # Lifting the exclusion alone would be a silent no-op here: a scope
        # anchored on .agents/skills never listed the mirrors to begin with.
        _, includes, _, _ = self.resolve(scope="skills", edition="Laravel",
                                         with_mirrors=True)
        self.assertIn(".claude/skills/**", includes)
        self.assertIn(".cursor/skills/**", includes)

    def test_hooks_scope_opts_itself_into_mirrors(self):
        _, _, excludes, _ = self.resolve(scope="hooks", edition="Laravel")
        for pattern in cc.MIRROR_EXCLUDE:
            self.assertNotIn(pattern, excludes)

    def test_machine_local_state_and_credentials_always_excluded(self):
        _, _, excludes, _ = self.resolve(scope="edition", edition="PHP Core")
        for pattern in ("**/memory-bank/local/**", "**/*.db", "**/.env",
                        "**/*.pem", "**/__pycache__/**"):
            self.assertIn(pattern, excludes)

    def test_user_excludes_are_added_not_substituted(self):
        _, _, excludes, _ = self.resolve(scope="docs", exclude=["docs/superpowers/**"])
        self.assertIn("docs/superpowers/**", excludes)
        self.assertIn(".git/**", excludes)


class TestScopeResolution(unittest.TestCase):

    def test_edition_scope_requires_an_edition(self):
        with self.assertRaises(cc.CollectError):
            cc.resolve_scope(namespace(scope="skills"))

    def test_root_scope_rejects_an_edition(self):
        with self.assertRaises(cc.CollectError):
            cc.resolve_scope(namespace(scope="tooling", edition="Laravel"))

    def test_custom_scope_requires_include_patterns(self):
        with self.assertRaises(cc.CollectError):
            cc.resolve_scope(namespace(scope="custom"))

    def test_custom_scope_accepts_include_patterns(self):
        target, includes, _, label = cc.resolve_scope(
            namespace(scope="custom", include=["harness/**"]))
        self.assertEqual(target, ROOT)
        self.assertEqual(includes, ["harness/**"])
        self.assertEqual(label, "custom")

    def test_edition_with_a_space_produces_a_usable_label(self):
        _, _, _, label = cc.resolve_scope(namespace(scope="core", edition="PHP Core"))
        self.assertEqual(label, "core-php-core")
        self.assertNotIn(" ", label)

    def test_target_points_at_the_edition_directory(self):
        target, _, _, _ = cc.resolve_scope(namespace(scope="skills", edition="Symfony"))
        self.assertEqual(target, ROOT / "Symfony")


class TestArgv(unittest.TestCase):

    def test_argv_is_fully_explicit(self):
        argv = cc.build_argv("code2prompt", ROOT / "Laravel", ["AGENTS.md"],
                             [".git/**"], "json", "cl100k")
        self.assertEqual(argv[0], "code2prompt")
        self.assertEqual(argv[1], str(ROOT / "Laravel"))
        for flag in ("--hidden", "--quiet", "--output-format", "--encoding"):
            self.assertIn(flag, argv)
        self.assertIn("--include", argv)
        self.assertIn("AGENTS.md", argv)
        self.assertIn(".git/**", argv)

    def test_hidden_is_mandatory(self):
        # The canon lives in .agents/skills; without --hidden a skills bundle
        # silently collects nothing but AGENTS.md.
        argv = cc.build_argv("code2prompt", ROOT, ["**/*.md"], [], "json", "cl100k")
        self.assertIn("--hidden", argv)

    def test_output_goes_to_stdout_not_the_cwd(self):
        argv = cc.build_argv("code2prompt", ROOT, ["*.md"], [], "markdown", "cl100k")
        self.assertEqual(argv[argv.index("--output-file") + 1], "-")


class TestVersionGate(unittest.TestCase):

    def test_parses_the_upstream_version_banner(self):
        self.assertEqual(cc.parse_version("code2prompt 4.3.0"), (4, 3, 0))
        self.assertEqual(cc.parse_version("4.2.0\n"), (4, 2, 0))

    def test_rejects_unparseable_output(self):
        with self.assertRaises(cc.CollectError):
            cc.parse_version("no version here")

    def test_bounds_bracket_the_verified_release(self):
        self.assertLessEqual(cc.MIN_VERSION, (4, 3, 0))
        self.assertLess((4, 3, 0), cc.MAX_VERSION)


class TestMissingBinary(unittest.TestCase):

    def test_absence_is_reported_without_a_traceback(self):
        env = dict(os.environ, **{cc.BIN_ENV: "/nonexistent/code2prompt"})
        proc = subprocess.run([sys.executable, str(SCRIPT), "docs", "--dry-run"],
                              cwd=ROOT, capture_output=True, text=True, env=env)
        self.assertEqual(proc.returncode, 1)
        self.assertIn("code2prompt not found", proc.stderr)
        self.assertNotIn("Traceback", proc.stderr)

    def test_list_works_without_the_binary(self):
        env = dict(os.environ, **{cc.BIN_ENV: "/nonexistent/code2prompt"})
        proc = subprocess.run([sys.executable, str(SCRIPT), "--list"],
                              cwd=ROOT, capture_output=True, text=True, env=env)
        self.assertEqual(proc.returncode, 0)
        for scope in cc.SCOPES:
            self.assertIn(scope, proc.stdout)


@unittest.skipUnless(binary_available(), "code2prompt not installed")
class TestAgainstTheBinary(unittest.TestCase):
    """Live checks. Skipped in CI, which has no Rust toolchain."""

    def run_script(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), *args],
                              cwd=ROOT, capture_output=True, text=True, timeout=180)

    def bundle_files(self, *args) -> list[str]:
        proc = self.run_script(*args, "--format", "json", "--stdout")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return list(json.loads(proc.stdout)["files"])

    def test_dry_run_reports_and_writes_nothing(self):
        before = sorted(p.name for p in (ROOT / ".c2p").glob("*")) \
            if (ROOT / ".c2p").is_dir() else []
        proc = self.run_script("skills", "--edition", "Laravel", "--dry-run")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("dry run", proc.stderr)
        self.assertIn("files", proc.stderr)
        after = sorted(p.name for p in (ROOT / ".c2p").glob("*")) \
            if (ROOT / ".c2p").is_dir() else []
        self.assertEqual(before, after, "--dry-run wrote a bundle")

    def test_empty_match_fails_loudly(self):
        proc = self.run_script("custom", "--include", "*.no-such-extension",
                               "--dry-run")
        self.assertEqual(proc.returncode, 1)
        self.assertIn("matched no files", proc.stderr)

    def test_client_specs_stay_out_of_an_edition_bundle(self):
        collected = self.bundle_files("edition", "--edition", "Laravel")
        self.assertTrue(collected)
        self.assertEqual([f for f in collected if f.startswith("Task/")], [])

    def test_the_client_data_guard_is_load_bearing(self):
        # Without the guard the same scope reaches the Epic specs; if this ever
        # stops being true the test above has become vacuous.
        collected = self.bundle_files("custom", "--include", "Laravel/**/*.md",
                                      "--with-task")
        self.assertTrue([f for f in collected if "/Task/Epics/" in f])

    def test_generated_mirrors_stay_out_of_an_edition_bundle(self):
        collected = self.bundle_files("skills", "--edition", "Symfony")
        self.assertTrue(collected)
        for path in collected:
            self.assertFalse(path.startswith((".claude/", ".cursor/", ".codex/")),
                             f"generated mirror in the bundle: {path}")

    def test_config_isolation_survives_a_c2pconfig_in_the_cwd(self):
        # A .c2pconfig in the working directory is auto-loaded with no opt-out;
        # if isolation broke, line numbers would appear in the bundle.
        marker = ROOT / ".c2pconfig"
        self.assertFalse(marker.exists(), "refusing to overwrite a real .c2pconfig")
        marker.write_text("line_numbers = true\n", encoding="utf-8")
        try:
            proc = self.run_script("custom", "--include",
                                   "scripts/check_links_ignore.txt", "--stdout")
            self.assertEqual(proc.returncode, 0, proc.stderr)
            self.assertNotIn("   1 | ", proc.stdout)
        finally:
            marker.unlink()

    def test_docs_scope_does_not_sweep_the_editions(self):
        # `-i "*.md"` here would read every markdown file in the repository —
        # 419 files against the ~33 this scope means.
        collected = self.bundle_files("docs")
        self.assertTrue(collected)
        for path in collected:
            self.assertTrue(path.startswith("docs/") or "/" not in path,
                            f"docs scope reached outside docs/: {path}")

    def test_skills_scope_reaches_nothing_outside_the_canon(self):
        collected = self.bundle_files("skills", "--edition", "Laravel")
        self.assertIn("AGENTS.md", collected)  # the anchored top-level one
        for path in collected:
            self.assertTrue(path == "AGENTS.md" or path.startswith(".agents/skills/"),
                            f"skills scope reached outside the canon: {path}")

    def test_json_bundle_carries_the_manifest_fields(self):
        proc = self.run_script("harness", "--format", "json", "--stdout")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        payload = json.loads(proc.stdout)
        self.assertGreater(len(payload["files"]), 0)
        self.assertGreater(payload["token_count"], 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
