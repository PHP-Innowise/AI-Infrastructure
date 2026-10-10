#!/usr/bin/env python3
"""Prevent shared-core synchronization from erasing framework specialization."""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
EDITIONS = (".agents", ".claude", ".cursor")
LEAKED_NATIVE_MARKERS = (
    "This flow keeps native PHP work",
    "# Native PHP Memory Bank",
    "# Native PHP Project Brain",
    "Manage durable native-PHP project memory",
    "Govern shared native-PHP task context",
    "generic PHP advice",
    "module/use-case boundaries",
    "data-access gateways",
    "worker/queue",
)
SKILL_DOCUMENTS = (
    "SKILL FLOW.md",
    "memory-bank/SKILL.md",
    "project-brain/SKILL.md",
)
FRAMEWORK_PATHS = {
    "Laravel": Path("Laravel"),
    "Symfony": Path("Symfony"),
    "PHP Core": Path("PHP Core"),
    "WordPress": Path("Cms/wordpress"),
}


class FrameworkSemanticPreservationTest(unittest.TestCase):
    def read(self, framework: str, edition: str, relative: str) -> str:
        return (ROOT / FRAMEWORK_PATHS[framework] / edition / "skills" / relative).read_text(
            encoding="utf-8"
        )

    def assert_shared_context_updates(self, framework: str, edition: str) -> None:
        memory = self.read(framework, edition, "memory-bank/SKILL.md")
        brain = self.read(framework, edition, "project-brain/SKILL.md")

        self.assertIn("approved-without-review", memory)
        self.assertIn("reviewer: null", memory)
        self.assertIn("independently reviewed", memory)
        for phase in (
            "understanding",
            "planning",
            "implementation",
            "verification",
            "finalization",
        ):
            self.assertIn(phase, brain)
        self.assertIn("merge completion candidate is advisory", brain.lower())
        self.assertIn("at most 3 semantic and 2 episodic items", brain)
        self.assertIn("8,000 serialized characters", brain)
        self.assertIn("automatic_promotion=false", brain)
        self.assertIn("brain-create", brain)
        self.assertIn("brain-update", brain)
        self.assertIn("brain-get", brain)

    def assert_no_native_php_replacement(self, *documents: str) -> None:
        combined = "\n".join(documents)
        for marker in LEAKED_NATIVE_MARKERS:
            self.assertNotIn(marker, combined)

    def assert_not_php_core_copy(self, framework: str, edition: str) -> None:
        for relative in SKILL_DOCUMENTS:
            with self.subTest(edition=edition, relative=relative):
                self.assertNotEqual(
                    self.read(framework, edition, relative),
                    self.read("PHP Core", edition, relative),
                )

    def assert_mirror_invariants(self, framework: str) -> None:
        for relative in ("memory-bank/SKILL.md", "project-brain/SKILL.md"):
            canonical = self.read(framework, ".agents", relative)
            self.assertEqual(canonical, self.read(framework, ".claude", relative))
            self.assertEqual(canonical, self.read(framework, ".cursor", relative))
        self.assertEqual(
            self.read(framework, ".claude", "SKILL FLOW.md"),
            self.read(framework, ".cursor", "SKILL FLOW.md"),
        )
        self.assertNotEqual(
            self.read(framework, ".agents", "SKILL FLOW.md"),
            self.read(framework, ".claude", "SKILL FLOW.md"),
        )

    def test_laravel_skills_keep_laravel_routing_and_shared_updates(self) -> None:
        required_flow_markers = (
            "eloquent",
            "queues-jobs",
            "events-notifications",
            "auth-scaffolding",
            "caching",
            "console-scheduler",
            "file-storage",
            "package-developer",
            "filament",
        )
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                flow = self.read("Laravel", edition, "SKILL FLOW.md")
                memory = self.read("Laravel", edition, "memory-bank/SKILL.md")
                brain = self.read("Laravel", edition, "project-brain/SKILL.md")

                self.assertIn("Laravel work", flow)
                for marker in required_flow_markers:
                    self.assertIn(marker, flow)
                self.assertIn("# Laravel Memory Bank", memory)
                self.assertIn("Eloquent model boundaries", memory)
                self.assertIn("MEM-YYYYMMDD-xxxxxxxx", memory)
                self.assertIn("reindex-bank", memory)
                self.assertIn(".memory-counter` as retired", memory)
                self.assertIn("promote-apply --promotion-id ID", memory)
                self.assertIn("# Laravel Project Brain", brain)
                self.assertIn("generic Laravel advice", brain)
                for command in (
                    "--owner OWNER start --task-id ID --goal GOAL",
                    "update --task-id ID --revision REVISION",
                    "complete --task-id ID --revision REVISION",
                    "promote-propose --source-id UUID",
                    "context.py validate",
                    "context.py parity",
                ):
                    self.assertIn(command, brain)
                self.assert_no_native_php_replacement(flow, memory, brain)
                self.assert_shared_context_updates("Laravel", edition)
                self.assert_not_php_core_copy("Laravel", edition)

                if edition == ".agents":
                    self.assertIn("Use `project-brain`", flow)
                    self.assertNotIn("Use `/project-brain`", flow)
                else:
                    self.assertIn("Use `/project-brain`", flow)
        self.assert_mirror_invariants("Laravel")

    def test_symfony_skills_keep_symfony_routing_and_shared_updates(self) -> None:
        required_flow_markers = (
            "doctrine-migration-designer",
            "api-platform-designer",
            "security-voter-designer",
            "form-validator-designer",
            "messenger-designer",
            "event-subscriber-designer",
            "console-command-coder",
            "fixture-factory-generator",
            "architecture-boundary-reviewer",
            "repository-reviewer",
            "twig-ux-reviewer",
            "container-reviewer",
        )
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                flow = self.read("Symfony", edition, "SKILL FLOW.md")
                memory = self.read("Symfony", edition, "memory-bank/SKILL.md")
                brain = self.read("Symfony", edition, "project-brain/SKILL.md")

                self.assertIn("Symfony", flow)
                self.assertIn("Doctrine", flow)
                self.assertIn("Messenger", flow)
                self.assertIn("specs/MANIFEST.md", flow)
                self.assertIn("examples/symfony-clean-code-patterns.md", flow)
                for marker in required_flow_markers:
                    self.assertIn(marker, flow)
                self.assertIn("# Symfony Memory Bank", memory)
                self.assertIn("Controller -> Service -> Repository", memory)
                self.assertIn("Messenger worker", memory)
                self.assertIn("# Symfony Project Brain", brain)
                self.assertIn("generic Symfony advice", brain)
                self.assert_no_native_php_replacement(flow, memory, brain)
                self.assert_shared_context_updates("Symfony", edition)
                self.assert_not_php_core_copy("Symfony", edition)

                if edition == ".agents":
                    self.assertIn("using-git-worktrees", flow)
                    self.assertIn("systematic-debugger", flow)
                    self.assertNotIn("/requirements-analyst", flow)
                else:
                    self.assertIn("/git-worktrees", flow)
                    self.assertIn("/debugger", flow)
                    self.assertIn("/requirements-analyst", flow)
        self.assert_mirror_invariants("Symfony")

    def test_wordpress_skills_keep_wordpress_routing_and_shared_updates(self) -> None:
        required_flow_markers = (
            "plugin-development",
            "theme-development",
            "block-development",
            "hooks-events",
            "rest-api",
            "content-modeling",
            "wp-cli",
            "multisite",
            "woocommerce",
            "cron-background-processing",
        )
        for edition in EDITIONS:
            with self.subTest(edition=edition):
                flow = self.read("WordPress", edition, "SKILL FLOW.md")
                memory = self.read("WordPress", edition, "memory-bank/SKILL.md")
                brain = self.read("WordPress", edition, "project-brain/SKILL.md")

                self.assertIn("WordPress work", flow)
                for marker in required_flow_markers:
                    self.assertIn(marker, flow)
                self.assertIn("# WordPress Memory Bank", memory)
                self.assertIn("generic WordPress advice", memory)
                self.assertIn("# WordPress Project Brain", brain)
                self.assertIn("generic WordPress advice", brain)
                self.assert_no_native_php_replacement(flow, memory, brain)
                self.assert_shared_context_updates("WordPress", edition)
                self.assert_not_php_core_copy("WordPress", edition)

                if edition == ".agents":
                    self.assertIn("Use `project-brain`", flow)
                else:
                    self.assertIn("Use `/project-brain`", flow)
        self.assert_mirror_invariants("WordPress")

    def test_wordpress_specialties_preserve_platform_safety_contracts(self) -> None:
        root = ROOT / FRAMEWORK_PATHS["WordPress"]
        required = {
            "plugin-development": ("activation", "uninstall", "compatibility"),
            "theme-development": ("theme.json", "child-theme", "accessibility"),
            "block-development": ("block.json", "deprecations", "serialization"),
            "hooks-events": ("priority", "recursion", "filter"),
            "rest-api": ("permission_callback", "WP_REST_Response", "WP_Error"),
            "content-modeling": ("post type", "taxonomy", "metadata"),
            "wp-cli": ("idempotent", "dry-run", "multisite"),
            "multisite": ("switch_to_blog()", "restore_current_blog()", "network"),
            "woocommerce": ("HPOS", "CRUD", "Action Scheduler"),
            "cron-background-processing": ("WP-Cron", "idempotent", "locking"),
        }
        for skill, markers in required.items():
            canonical = (root / ".agents/skills" / skill / "SKILL.md").read_text(
                encoding="utf-8"
            )
            for marker in markers:
                with self.subTest(skill=skill, marker=marker):
                    self.assertIn(marker, canonical)
            for mirror in (".claude", ".cursor"):
                self.assertEqual(
                    canonical,
                    (root / mirror / "skills" / skill / "SKILL.md").read_text(
                        encoding="utf-8"
                    ),
                )

        policy = (root / "AGENTS.md").read_text(encoding="utf-8")
        for marker in (
            "A nonce is CSRF protection, not permission",
            "`$wpdb->prepare()`",
            "permission_callback",
            "flush_rewrite_rules()",
            "Autoloaded",
            "options MUST stay small",
            "data is per-site, network-wide, per-user, locale-specific, or global",
        ):
            self.assertIn(marker, policy)

        settings = (root / ".claude/settings.json").read_text(encoding="utf-8")
        self.assertIn('"Bash(wp:*)"', settings)
        self.assertIn('"Read(wp-config.php)"', settings)
        hook = (root / ".claude/hooks/bash-validator.sh").read_text(encoding="utf-8")
        self.assertIn('"argv|wp|db reset|', hook)
        self.assertIn("wp-config", hook)


class ShippedContentIndexTest(unittest.TestCase):
    """Every edition's own shipped documents stay retrievable.

    The index masks a value a secret pattern matches, and excludes a document
    whose masking does not converge. An over-eager pattern once excluded the
    Laravel architect skill - all three tool copies - over a documented
    `php artisan down --secret=...` example, in every install; a pattern that
    fired on shipped documents now would silently mask their text instead.
    """

    def test_no_shipped_document_is_excluded_as_a_secret(self) -> None:
        for framework, edition in FRAMEWORK_PATHS.items():
            with self.subTest(edition=framework), tempfile.TemporaryDirectory(
                prefix="index-screen-"
            ) as temporary:
                target = Path(temporary) / "edition"
                listed = subprocess.run(
                    ["git", "ls-files", "-z", "--", str(edition)],
                    cwd=ROOT, capture_output=True, check=True,
                ).stdout.decode("utf-8").split("\0")
                for name in filter(None, listed):
                    destination = target / Path(name).relative_to(edition)
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(ROOT / name, destination)
                subprocess.run(["git", "init", "--quiet"], cwd=target, check=True)
                subprocess.run(["git", "add", "--all"], cwd=target, check=True)
                result = subprocess.run(
                    [sys.executable, "memory-bank/scripts/context.py", "index", "--json"],
                    cwd=target, capture_output=True, text=True,
                )
                self.assertEqual(0, result.returncode, result.stderr)
                indexed = json.loads(result.stdout)
                excluded = [
                    item["path"] for item in indexed["excluded"]
                    if item.get("reason") == "secret"
                ]
                self.assertEqual([], excluded)
                self.assertEqual([], indexed["redacted"])


class MemoryWiringClaimsTest(unittest.TestCase):
    """Shipped memory documents do not deny what the editions wire.

    Every edition registers the local memory MCP server (`harness-memory` in
    `.mcp.json`, running `memory-bank/scripts/mcp_server.py`) and wires a
    prompt hook that injects a Task Capsule. `project-brain/PROTOCOL.md` in
    the editions and the generator asset still said no MCP server and no
    automatic prompt injection were part of the runtime, the `project-brain`
    skill told agents never to claim automatic prompt injection, and the root
    READMEs, `docs/SECURITY.md`, `docs/TOOL-INTEGRATIONS.md` and
    `docs/CONTEXT-AND-MEMORY.md` repeated it. A denial is checked only where
    the wiring it contradicts exists, so removing the server or the hook frees
    the documents to say so again. Changelogs and dated design records quote
    history and are not checked.
    """

    DENIES_MCP = (
        r"\bno\b[^.]*\bMCP servers?\b[^.]*\bis part of\b",
        r"\bhas no\b[^.]*\bMCP server\b",
        r"\brequires? no MCP servers?\b",
        r"\bdoes not provide\b[^.]*\bMCP\b",
        r"\bнет\b[^.]*\bMCP\b",
    )
    DENIES_INJECTION = (
        r"\bautomatic prompt injection is part of\b",
        r"\bhas no\b[^.]*\bautomatic prompt injection\b",
        r"\bnever claim automatic prompt injection\b",
        r"\bdoes not include automatic prompt injection\b",
        r"\bretrieval happens? only through explicit CLI calls\b",
    )
    ROOT_DOCUMENTS = (
        "README_EN.md",
        "README_RU.md",
        "docs/CONTEXT-AND-MEMORY.md",
        "docs/SECURITY.md",
        "docs/TOOL-INTEGRATIONS.md",
    )
    GENERATOR = Path("Infrastructure-Creator")
    GENERATOR_ASSETS = GENERATOR / ".agents/skills/memory-seed/assets"

    @staticmethod
    def registers_memory_mcp(edition: Path) -> bool:
        try:
            servers = json.loads(
                (ROOT / edition / ".mcp.json").read_text(encoding="utf-8")
            )["mcpServers"]
        except (OSError, ValueError, KeyError):
            return False
        server = servers.get("harness-memory") or {}
        return "mcp_server.py" in json.dumps(server) and (
            ROOT / edition / "memory-bank/scripts/mcp_server.py"
        ).is_file()

    @staticmethod
    def injects_capsule(edition: Path) -> bool:
        try:
            hooks = json.loads(
                (ROOT / edition / ".claude/settings.json").read_text(encoding="utf-8")
            )["hooks"]["UserPromptSubmit"]
        except (OSError, ValueError, KeyError):
            return False
        return "working-memory-read.sh" in json.dumps(hooks) and (
            ROOT / edition / ".claude/hooks/working-memory-read.sh"
        ).is_file()

    @staticmethod
    def tracked_documents(prefix: Path) -> list:
        listed = subprocess.run(
            ["git", "ls-files", "-z", "--", str(prefix)],
            cwd=ROOT, capture_output=True, check=True,
        ).stdout.decode("utf-8").split("\0")
        return [
            Path(name) for name in listed
            if name.endswith(".md")
            and Path(name).name != "CHANGELOG.md"
            and "Task" not in Path(name).parts
        ]

    def denials(self, relative: Path, mcp: bool, injection: bool) -> list:
        flat = " ".join(
            (ROOT / relative).read_text(encoding="utf-8").split()
        )
        patterns = (self.DENIES_MCP if mcp else ()) + (
            self.DENIES_INJECTION if injection else ()
        )
        return [
            f"{relative.as_posix()}: {match.group(0)[:100]}"
            for pattern in patterns
            for match in re.finditer(pattern, flat, re.I)
        ]

    def test_no_shipped_document_denies_the_memory_server_or_the_capsule(self) -> None:
        offenders = []
        wired = []
        for edition in FRAMEWORK_PATHS.values():
            mcp, injection = self.registers_memory_mcp(edition), self.injects_capsule(edition)
            wired.append((mcp, injection))
            for relative in self.tracked_documents(edition):
                offenders += self.denials(relative, mcp, injection)
        # Sanity: the condition must hold today, or the test checks nothing.
        self.assertIn((True, True), wired)

        any_mcp = any(mcp for mcp, _ in wired)
        any_injection = any(injection for _, injection in wired)
        for name in self.ROOT_DOCUMENTS:
            offenders += self.denials(Path(name), any_mcp, any_injection)

        # The generator copies these assets into every project it builds, with
        # the MCP server and a hook-forge prompt hook beside them.
        asset_mcp = (ROOT / self.GENERATOR_ASSETS / "scripts/mcp_server.py").is_file()
        asset_injection = "working-memory-read.sh" in (
            ROOT / self.GENERATOR / ".agents/skills/hook-forge/SKILL.md"
        ).read_text(encoding="utf-8")
        for relative in self.tracked_documents(self.GENERATOR_ASSETS):
            offenders += self.denials(relative, asset_mcp, asset_injection)
        self.assertEqual([], offenders)


class FrameworkApiCurrencyTest(unittest.TestCase):
    """Snippets that fatal or mislead on the framework versions an edition
    declares. Each pattern was shipped once and found by an audit."""

    PHP_FENCE = re.compile(r"^```php[^\n]*\n(.*?)^```", re.S | re.M)

    def canon(self, edition: str) -> list:
        root = ROOT / edition
        paths = [*sorted((root / ".agents" / "skills").rglob("*.md"))]
        examples = root / "examples"
        if examples.is_dir():
            paths += sorted(examples.rglob("*.md"))
        return [(path, path.read_text(encoding="utf-8")) for path in paths]

    def php_blocks(self, text: str) -> list:
        return self.PHP_FENCE.findall(text)

    def test_laravel_snippets_match_the_11_plus_skeleton(self) -> None:
        offenders = []
        for path, text in self.canon("Laravel"):
            relative = path.relative_to(ROOT).as_posix()
            for block in self.php_blocks(text):
                # Filament v4+: `string|BackedEnum|null`; `?string` is a fatal
                # property-type mismatch.
                if "?string $navigationIcon" in block:
                    offenders.append(f"{relative}: ?string $navigationIcon")
                # The 11+ base Controller has no AuthorizesRequests.
                if "$this->authorize(" in block and "AuthorizesRequests" not in text:
                    offenders.append(f"{relative}: $this->authorize() without AuthorizesRequests")
                # The skeleton defines local, public and s3 - no `private` disk.
                if re.search(r"(?:disk|fake)\('private'\)|, 'private'\)", block) and "'private' =>" not in text:
                    offenders.append(f"{relative}: undefined 'private' disk")
        self.assertEqual([], offenders)

    def test_laravel_never_claims_dispatch_is_deferred_by_default(self) -> None:
        for path, text in self.canon("Laravel"):
            with self.subTest(path=path.relative_to(ROOT).as_posix()):
                self.assertNotRegex(text, r"which Laravel defers automatically")
                self.assertNotIn("EventServiceProvider.php", text)
                self.assertNotIn("#[AsListener]", text)

    def test_symfony_voters_take_the_8x_vote_parameter(self) -> None:
        offenders = []
        for path, text in self.canon("Symfony"):
            for signature in re.findall(r"function voteOnAttribute\([^)]*\)", text):
                if "?Vote $vote = null" not in signature:
                    offenders.append(f"{path.relative_to(ROOT).as_posix()}: {signature}")
        self.assertEqual([], offenders)


if __name__ == "__main__":
    unittest.main()
