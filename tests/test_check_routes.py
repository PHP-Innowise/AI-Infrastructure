#!/usr/bin/env python3
"""Regression tests for scripts/check_routes.py.

Each fixture is a miniature edition inside a temporary Git repository - the
hook check reads executable bits from the index, not the disk, so the tests
have to exercise a real index rather than route around it. The clean fixture
is built to pass every check; each test then breaks exactly one reference and
asserts the gate names it (and that the matching allowlist entry silences it).
The last test runs the gate over this repository and requires zero errors.

Run: python3 -m unittest tests.test_check_routes
"""

from __future__ import annotations

import contextlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import check_routes  # noqa: E402

EDITION = "Laravel"

SKILL = "---\nname: {name}\ndescription: {name} skill.\n---\n\n# {name}\n"

CLAUDE_AGENT = """---
name: {name}
description: "Use this agent for {name} work."
model: sonnet
invokes: {name}
phase: implementation
---

# {name} Agent

1. Use the Skill tool to invoke `{name}` skill
2. STOP.

**Next by flow:** `/code-reviewer [context summary]` - review the change.
- `/debugger [context summary]` - investigate a failure.
"""

CURSOR_AGENT = """---
name: {name}
description: "Use this agent for {name} work."
---

# {name} Agent

1. Use the Skill tool to invoke `{name}` skill
2. STOP.

**Next by flow:** `/code-reviewer [context summary]` - review the change.
"""

COMMAND = """---
spawns: {agent}-agent
phase: implementation
flow-next: {next}
flow-alternatives: [debugger, memory]
---

# {agent}

## Instructions

Use the Task tool to spawn a sub-agent:
- **subagent_type:** `{agent}`
- **prompt:** `$ARGUMENTS`
"""

CURSOR_COMMAND = """---
name: {stem}
description: "Spawn the {agent} agent."
---

# {agent}

- **subagent_type:** `{agent}`
"""

MEMORY_COMMAND = """---
name: memory
description: "Refresh repository-local context."
---

Read `{tree}/skills/memory/SKILL.md`, execute exactly one refresh, and stop.
"""

FLOW_COMMAND = """---
flow: feature
stages:
  - { phase: implementation, agents: [coder] }
  - { phase: verification, agents: [code-reviewer], parallel: true }
---

# Flow: Feature

If a stage fails, stop and suggest `/debugger`.
"""

SLASH_FLOW = """# Skill Flow

```text
/coder
  -> /code-reviewer
  -> /debugger
```

- Use `memory` to refresh context.

## Phase Map

| Phase | Commands |
| --- | --- |
| Implementation | `/coder` |
| Quality | `/code-reviewer`, `/debugger` |
| Utility | `memory` |
"""

CODEX_FLOW = """# Skill Flow

```text
coder
  -> code-reviewer
  -> systematic-debugger
```

- Use `memory` to refresh context.

## Phase Map

| Phase | Skills |
| --- | --- |
| Quality | `/code-reviewer`, `systematic-debugger` |
"""

AGENTS_MD = """# AGENTS.md

- A sanctioned flow command (`/flow-feature`) MAY spawn several agents.
- MUST use the argument-free `memory` skill to refresh context.
- Built-in subagents are denied by the `subagent-gate` hook.
- Delegate reviews to the `code-reviewer` agent.
"""

CLAUDE_SETTINGS = {
    "hooks": {
        "PreToolUse": [
            {
                "matcher": "Agent|Task",
                "hooks": [
                    {
                        "type": "command",
                        "command": '"$CLAUDE_PROJECT_DIR"/.claude/hooks/subagent-gate.sh',
                    }
                ],
            }
        ]
    }
}
CURSOR_HOOKS = {
    "version": 1,
    "hooks": {"subagentStart": [{"command": ".cursor/hooks/subagent-gate.sh"}]},
}
CODEX_HOOKS = {
    "hooks": {
        "PreToolUse": [
            {
                "hooks": [
                    {
                        "type": "command",
                        "command": '"$(git rev-parse --show-toplevel)"/.codex/hooks/subagent-gate.sh',
                    }
                ]
            }
        ]
    }
}

AGENT_SKILLS = ("coder", "code-reviewer", "systematic-debugger")


class RoutesFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="check-routes-test-")
        self.root = Path(self.temporary.name)
        self.git("init", "--quiet")
        self.edition = self.root / EDITION
        self.build_clean_edition()
        self.stage()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    # -- fixture plumbing --------------------------------------------------

    def git(self, *args: str) -> None:
        subprocess.run(
            ["git", "-C", str(self.root), *args], check=True, capture_output=True
        )

    def write(self, relative: str, content: str) -> Path:
        path = self.edition / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def write_json(self, relative: str, data: dict) -> Path:
        return self.write(relative, json.dumps(data, indent=2) + "\n")

    def hook(self, relative: str, executable: bool = True) -> None:
        path = self.write(relative, "#!/usr/bin/env bash\nexit 0\n")
        path.chmod(0o755 if executable else 0o644)

    def stage(self) -> None:
        self.git("add", "-A")
        # Set the index mode explicitly: core.fileMode is false on Windows
        # checkouts, where the on-disk bit says nothing.
        for tool in (".claude", ".cursor", ".codex"):
            hooks = self.edition / tool / "hooks"
            for script in hooks.glob("*.sh") if hooks.is_dir() else ():
                mode = "+x" if script.stat().st_mode & 0o111 else "-x"
                self.git("update-index", f"--chmod={mode}", "--", self.rel(script))

    def rel(self, path: Path) -> str:
        return path.relative_to(self.root).as_posix()

    def build_clean_edition(self) -> None:
        self.write("AGENTS.md", AGENTS_MD)
        self.write_json(".claude/settings.json", CLAUDE_SETTINGS)
        self.write_json(".cursor/hooks.json", CURSOR_HOOKS)
        self.write_json(".codex/hooks.json", CODEX_HOOKS)
        for tool in (".claude", ".cursor", ".codex"):
            self.hook(f"{tool}/hooks/subagent-gate.sh")
        for name in AGENT_SKILLS + ("memory",):
            self.write(f".agents/skills/{name}/SKILL.md", SKILL.format(name=name))
        for name in AGENT_SKILLS:
            self.write(f".claude/agents/{name}-agent.md", CLAUDE_AGENT.format(name=name))
            self.write(f".cursor/agents/{name}-agent.md", CURSOR_AGENT.format(name=name))
        commands = {
            "coder": ("coder", "code-reviewer"),
            "code-reviewer": ("code-reviewer", "null"),
            "debugger": ("systematic-debugger", "coder"),
        }
        for stem, (agent, flow_next) in commands.items():
            self.write(
                f".claude/commands/{stem}.md", COMMAND.format(agent=agent, next=flow_next)
            )
            self.write(
                f".cursor/commands/{stem}.md", CURSOR_COMMAND.format(stem=stem, agent=agent)
            )
        self.write(".claude/commands/memory.md", MEMORY_COMMAND.format(tree=".claude"))
        self.write(".cursor/commands/memory.md", MEMORY_COMMAND.format(tree=".cursor"))
        self.write(".claude/commands/flow-feature.md", FLOW_COMMAND)
        self.write(".claude/skills/SKILL FLOW.md", SLASH_FLOW)
        self.write(".cursor/skills/SKILL FLOW.md", SLASH_FLOW)
        self.write(".agents/skills/SKILL FLOW.md", CODEX_FLOW)

    def replace(self, relative: str, old: str, new: str) -> None:
        path = self.edition / relative
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text, f"fixture text not found in {relative}")
        path.write_text(text.replace(old, new, 1), encoding="utf-8")

    # -- running the gate --------------------------------------------------

    def run_gate(self, allowlist: dict | None = None) -> check_routes.Report:
        return check_routes.check_repo(self.root, [EDITION], allowlist or {})

    def messages(self, report: check_routes.Report, severity: str = "error") -> list:
        return [
            f"{item.location()} [{item.check}] {item.message}"
            for item in report.findings
            if item.severity == severity
        ]

    def assert_single_error(self, report: check_routes.Report, check: str, *needles: str):
        errors = report.errors
        self.assertEqual(len(errors), 1, "\n".join(self.messages(report)))
        self.assertEqual(errors[0].check, check)
        for needle in needles:
            self.assertIn(needle, f"{errors[0].location()} {errors[0].message}")
        return errors[0]


class CleanFixtureTest(RoutesFixture):
    def test_clean_fixture_has_no_findings(self) -> None:
        report = self.run_gate()
        self.assertEqual(self.messages(report), [])
        self.assertEqual(self.messages(report, "warning"), [])

    def test_every_hook_prefix_form_counts_as_wiring(self) -> None:
        # Bare relative (Cursor), "$CLAUDE_PROJECT_DIR"/... (Claude) and the
        # git-toplevel form (Codex) are all recognised: no script is reported
        # unwired, and every one is checked for its executable bit.
        self.git("update-index", "--chmod=-x", "--", f"{EDITION}/.claude/hooks/subagent-gate.sh")
        self.git("update-index", "--chmod=-x", "--", f"{EDITION}/.codex/hooks/subagent-gate.sh")
        report = self.run_gate()
        self.assertEqual(self.messages(report, "warning"), [])
        paths = sorted(item.path for item in report.errors)
        self.assertEqual(
            paths,
            [f"{EDITION}/.claude/settings.json", f"{EDITION}/.codex/hooks.json"],
        )


class HookWiringTest(RoutesFixture):
    def test_non_executable_wired_script_is_an_error(self) -> None:
        self.git("update-index", "--chmod=-x", "--", f"{EDITION}/.cursor/hooks/subagent-gate.sh")
        error = self.assert_single_error(
            self.run_gate(), "hook-wiring", ".cursor/hooks.json:", "100644", "100755"
        )
        self.assertEqual(error.target, ".cursor/hooks/subagent-gate.sh")

    def test_missing_wired_script_is_an_error(self) -> None:
        settings = json.loads(json.dumps(CLAUDE_SETTINGS))
        settings["hooks"]["Stop"] = [
            {"hooks": [{"type": "command", "command": ".claude/hooks/ghost.sh"}]}
        ]
        self.write_json(".claude/settings.json", settings)
        self.assert_single_error(
            self.run_gate(), "hook-wiring", ".claude/settings.json:", "ghost.sh", "does not exist"
        )

    def test_untracked_wired_script_is_an_error(self) -> None:
        settings = json.loads(json.dumps(CLAUDE_SETTINGS))
        settings["hooks"]["Stop"] = [
            {"hooks": [{"type": "command", "command": ".claude/hooks/fresh.sh"}]}
        ]
        self.write_json(".claude/settings.json", settings)
        self.hook(".claude/hooks/fresh.sh")  # on disk, never staged
        self.assert_single_error(self.run_gate(), "hook-wiring", "fresh.sh", "not tracked")

    def test_invalid_wiring_json_is_an_error(self) -> None:
        self.write(".codex/hooks.json", '{"hooks": {\n  "PreToolUse": [\n}\n')
        self.assert_single_error(self.run_gate(), "hook-wiring", ".codex/hooks.json:", "invalid JSON")

    def test_wired_path_without_the_sh_extension_is_an_error(self) -> None:
        # The typo that drops ".sh" is still a wired reference: the hook
        # exits 127 at runtime, which both hosts treat as non-blocking.
        settings = json.loads(json.dumps(CLAUDE_SETTINGS))
        settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = (
            '"$CLAUDE_PROJECT_DIR"/.claude/hooks/subagent-gate'
        )
        self.write_json(".claude/settings.json", settings)
        report = self.run_gate()
        error = self.assert_single_error(
            report, "hook-wiring", ".claude/settings.json:", "subagent-gate", "does not exist"
        )
        self.assertEqual(error.target, ".claude/hooks/subagent-gate")
        self.assertEqual(len(report.warnings), 1, self.messages(report, "warning"))

    def test_wired_hooks_path_must_name_a_sh_script(self) -> None:
        self.hook(".cursor/hooks/gate.py")  # exists, tracked, executable - not a *.sh
        self.write_json(
            ".cursor/hooks.json",
            {
                "version": 1,
                "hooks": {
                    "subagentStart": [{"command": ".cursor/hooks/subagent-gate.sh"}],
                    "stop": [{"command": ".cursor/hooks/gate.py"}],
                    "beforeShellExecution": [{"command": "ls .cursor/hooks/"}],
                },
            },
        )
        self.stage()
        report = self.run_gate()
        targets = sorted(item.target for item in report.errors)
        self.assertEqual(targets, [".cursor/hooks/", ".cursor/hooks/gate.py"], self.messages(report))
        self.assertTrue(all("*.sh script" in item.message for item in report.errors))

    def test_codex_launcher_argument_must_name_a_sh_script(self) -> None:
        launcher = (
            "sh -c 'd=$(pwd); until [ -z \"$d\" ] || [ -f \"$d/.codex/hooks/$1\" ]; "
            "do d=${d%/*}; done; exec \"$d/.codex/hooks/$1\"' sh "
        )

        def wire(argument: str) -> None:
            hooks = json.loads(json.dumps(CODEX_HOOKS))
            hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = launcher + argument
            self.write_json(".codex/hooks.json", hooks)

        wire("subagent-gate.sh")
        self.assertEqual(self.messages(self.run_gate()), [])
        self.assertEqual(self.messages(self.run_gate(), "warning"), [])

        wire("subagent-gate")
        error = self.assert_single_error(
            self.run_gate(), "hook-wiring", ".codex/hooks.json:", "does not exist"
        )
        self.assertEqual(error.target, ".codex/hooks/subagent-gate")

        wire("")  # no $1: the launcher execs the hooks directory itself
        error = self.assert_single_error(self.run_gate(), "hook-wiring", ".codex/hooks.json:")
        self.assertEqual(error.target, ".codex/hooks/")

    def test_non_utf8_wiring_file_is_a_finding(self) -> None:
        path = self.edition / ".cursor/hooks.json"
        path.write_bytes(path.read_bytes() + b"\xff\n")
        report = self.run_gate()
        self.assertTrue(
            any(
                item.check == "hook-wiring"
                and item.path == f"{EDITION}/.cursor/hooks.json"
                and "not valid UTF-8" in item.message
                for item in report.errors
            ),
            self.messages(report),
        )

    def test_unwired_script_warns_until_allowlisted(self) -> None:
        self.hook(".codex/hooks/subagent-dispatch.sh")
        self.stage()
        report = self.run_gate()
        self.assertEqual(report.errors, [])
        warnings = report.warnings
        self.assertEqual(len(warnings), 1, self.messages(report, "warning"))
        self.assertIn("no wiring file references it", warnings[0].message)
        self.assertEqual(warnings[0].path, f"{EDITION}/.codex/hooks/subagent-dispatch.sh")

        allowlist = {
            "unwired_hooks": [
                {
                    "edition": "*",
                    "path": ".codex/hooks/subagent-dispatch.sh",
                    "reason": "Codex multi-agent is disabled.",
                }
            ]
        }
        report = self.run_gate(allowlist)
        self.assertEqual(report.findings, [])
        self.assertEqual(len(report.allowlisted), 1)


class RoutingTest(RoutesFixture):
    def test_spawns_must_name_an_agent(self) -> None:
        self.replace(".claude/commands/coder.md", "spawns: coder-agent", "spawns: codder-agent")
        self.assert_single_error(
            self.run_gate(), "routing", ".claude/commands/coder.md:2", "codder-agent"
        )

    def test_spawns_resolves_by_frontmatter_name_too(self) -> None:
        self.replace(".claude/commands/coder.md", "spawns: coder-agent", "spawns: coder")
        self.assertEqual(self.messages(self.run_gate()), [])

    def test_flow_next_must_name_a_command_or_skill(self) -> None:
        self.replace(".claude/commands/coder.md", "flow-next: code-reviewer", "flow-next: reviewer")
        self.assert_single_error(
            self.run_gate(), "routing", ".claude/commands/coder.md:4", "flow-next 'reviewer'"
        )

    def test_flow_alternatives_accept_a_skill_without_a_command(self) -> None:
        # In Claude Code every skill is also /<skill>: a skill-only name is fine.
        self.replace(
            ".claude/commands/coder.md",
            "flow-alternatives: [debugger, memory]",
            "flow-alternatives: [systematic-debugger]",
        )
        self.assertEqual(self.messages(self.run_gate()), [])

    def test_flow_stage_agents_must_exist(self) -> None:
        self.replace(".claude/commands/flow-feature.md", "agents: [code-reviewer]", "agents: [reviewer]")
        self.assert_single_error(
            self.run_gate(), "routing", ".claude/commands/flow-feature.md:5", "'reviewer'"
        )

    def test_subagent_type_must_be_the_frontmatter_name(self) -> None:
        # The host and subagent-gate.sh spawn by frontmatter name only; the
        # file stem and the `spawns` alias (coder-agent) are blocked at runtime.
        self.replace(".claude/commands/coder.md", "**subagent_type:** `coder`", "**subagent_type:** `coder-agent`")
        error = self.assert_single_error(
            self.run_gate(), "routing", ".claude/commands/coder.md:13", "subagent_type 'coder-agent'"
        )
        self.assertIn("frontmatter name", error.message)

    def test_flow_stage_agent_must_be_the_frontmatter_name(self) -> None:
        self.replace(
            ".claude/commands/flow-feature.md", "agents: [code-reviewer]", "agents: [code-reviewer-agent]"
        )
        self.assert_single_error(
            self.run_gate(), "routing", ".claude/commands/flow-feature.md:5", "'code-reviewer-agent'"
        )

    def test_agents_md_agent_must_be_the_frontmatter_name(self) -> None:
        self.replace("AGENTS.md", "the `code-reviewer` agent", "the `code-reviewer-agent` agent")
        self.assert_single_error(self.run_gate(), "routing", "AGENTS.md:6", "code-reviewer-agent")

    def test_agent_name_must_read_the_same_to_the_gate(self) -> None:
        # subagent-gate.sh deletes double quotes but keeps single quotes, so
        # name: 'coder' is "'coder'" on its roster and spawning coder fails.
        self.replace(".claude/agents/coder-agent.md", "name: coder\n", "name: 'coder'\n")
        report = self.run_gate()
        messages = "\n".join(self.messages(report))
        self.assertIn(".claude/agents/coder-agent.md:2", messages)
        self.assertIn("reads it as ''coder''", messages)
        self.assertIn(".claude/commands/coder.md:13 [routing] subagent_type 'coder'", messages)
        self.assertEqual(check_routes.roster_name(['---', 'name: "coder"  ', '---']), "coder")
        self.assertEqual(check_routes.roster_name(['---', 'x: 1', '---', 'name: late']), "")

    def test_command_body_spawn_phrases_must_name_an_agent(self) -> None:
        # Symfony ("Use the Task tool to spawn `x`."), WordPress ("Spawn the
        # `x` agent with ...", "Spawn `x` with ...") and the bare description
        # line ("Spawn x agent to ...") are routes too: a Cursor command has no
        # frontmatter `spawns` to fall back on.
        phrases = {
            "Use the Task tool to spawn `{name}`.": 8,
            "Spawn the `{name}` agent with `$ARGUMENTS`. It executes only the skill.": 8,
            "Spawn `{name}` with `$ARGUMENTS`.": 8,
            "Spawn {name} agent to review the change.": 8,
        }
        for phrase, line in phrases.items():
            with self.subTest(phrase=phrase):
                body = "---\nname: review\ndescription: \"Review.\"\n---\n\n# Review\n\n{}\n"
                self.write(".cursor/commands/review.md", body.format(phrase.format(name="code-reviewer")))
                self.assertEqual(self.messages(self.run_gate()), [])
                self.write(".cursor/commands/review.md", body.format(phrase.format(name="codder")))
                self.assert_single_error(
                    self.run_gate(), "routing", f".cursor/commands/review.md:{line}",
                    "spawn 'codder'", ".cursor/agents",
                )

    def test_spawn_prose_without_a_name_is_not_a_reference(self) -> None:
        self.write(
            ".claude/commands/review.md",
            "---\nspawns: code-reviewer-agent\nflow-next: null\n---\n\n"
            "Spawn all three agents of the single `parallel: true` stage in one message;\n"
            "then spawn each agent with the Task tool, and spawn the `parallel: true` stage's agents.\n"
            "Spawn one agent at a time. Spawn write-capable agents strictly one at a time.\n"
            "Spawn only agents from this accelerator's roster.\n"
            "Never spawn `general-purpose`; do not spawn `explore`, and don't spawn `plan`.\n"
            "Spawn `Explore` is a built-in, not a project agent name.\n",
        )
        self.assertEqual(self.messages(self.run_gate()), [])

    def test_command_body_invoked_skill_must_exist(self) -> None:
        body = "---\nname: mapper\ndescription: \"Map.\"\n---\n\nInvoke the {} skill and follow it completely.\n"
        self.write(".cursor/commands/mapper.md", body.format("code-reviewer"))
        self.assertEqual(self.messages(self.run_gate()), [])
        self.write(".cursor/commands/mapper.md", body.format("code-reviewr"))
        self.assert_single_error(
            self.run_gate(), "routing", ".cursor/commands/mapper.md:6", "invoked skill 'code-reviewr'"
        )

    def test_cursor_subagent_type_must_name_a_cursor_agent(self) -> None:
        (self.edition / ".cursor/agents/systematic-debugger-agent.md").unlink()
        report = self.run_gate()
        self.assertTrue(
            any(
                item.path.endswith(".cursor/commands/debugger.md")
                and "subagent_type 'systematic-debugger'" in item.message
                for item in report.errors
            ),
            self.messages(report),
        )

    def test_claude_agent_invokes_must_name_a_skill(self) -> None:
        self.replace(".claude/agents/coder-agent.md", "invokes: coder", "invokes: coding")
        self.assert_single_error(
            self.run_gate(), "routing", ".claude/agents/coder-agent.md:5", "invokes 'coding'"
        )

    def test_claude_agent_without_invokes_is_an_error(self) -> None:
        self.replace(".claude/agents/coder-agent.md", "invokes: coder\n", "")
        self.assert_single_error(self.run_gate(), "routing", "has no 'invokes'")

    def test_cursor_agent_body_skill_must_exist(self) -> None:
        self.replace(".cursor/agents/coder-agent.md", "invoke `coder` skill", "invoke `coding` skill")
        self.assert_single_error(
            self.run_gate(), "routing", ".cursor/agents/coder-agent.md:8", "'coding'"
        )

    def test_duplicate_agent_names_are_an_error(self) -> None:
        self.replace(".cursor/agents/code-reviewer-agent.md", "name: code-reviewer", "name: coder")
        report = self.run_gate()
        self.assertTrue(
            any("also used by" in item.message for item in report.errors), self.messages(report)
        )

    def test_command_skill_path_must_exist(self) -> None:
        self.replace(".claude/commands/memory.md", "skills/memory/", "skills/memroy/")
        self.assert_single_error(
            self.run_gate(), "routing", ".claude/commands/memory.md:6", ".claude/skills/memroy/"
        )

    def test_agent_slash_suggestion_must_resolve(self) -> None:
        self.replace(".claude/agents/coder-agent.md", "`/debugger [context", "`/debuger [context")
        self.assert_single_error(
            self.run_gate(), "routing", ".claude/agents/coder-agent.md:15", "/debuger"
        )

    def test_codex_flow_rejects_command_only_names(self) -> None:
        # /debugger is a Claude/Cursor command (and resolves in their flows);
        # Codex has no command layer, so its flow must use the skill name.
        self.replace(".agents/skills/SKILL FLOW.md", "-> systematic-debugger", "-> /debugger")
        error = self.assert_single_error(
            self.run_gate(), "routing", ".agents/skills/SKILL FLOW.md:6", "Codex has no command layer"
        )
        self.assertEqual(error.target, "debugger")

    def test_codex_flow_bare_step_must_resolve(self) -> None:
        self.replace(".agents/skills/SKILL FLOW.md", "-> systematic-debugger", "-> systematic-debuger")
        error = self.assert_single_error(
            self.run_gate(), "routing", ".agents/skills/SKILL FLOW.md:6", "flow step 'systematic-debuger'"
        )
        self.assertEqual(error.target, "systematic-debuger")

    def test_flow_step_alternatives_notes_and_arguments_are_checked(self) -> None:
        self.replace(
            ".agents/skills/SKILL FLOW.md",
            "  -> systematic-debugger\n",
            "  -> systematic-debugger or ghost-a   (when stuck)\n"
            "  -> coder, ghost-b, or code-reviewer\n"
            "  ghost-c <target>  OR  the scan's offer\n"
            "    ├─ ghost-d          ┐\n"
            "    │   └─ ghost-e  (needs a list)\n"
            "    → ghost-f\n",
        )
        report = self.run_gate()
        self.assertEqual(
            sorted(item.target for item in report.errors),
            ["ghost-a", "ghost-b", "ghost-c", "ghost-d", "ghost-e", "ghost-f"],
            self.messages(report),
        )

    def test_flow_diagram_prose_is_not_a_step(self) -> None:
        # Infrastructure-Creator's tree mixes steps with prose; a word that
        # goes on into more words is prose, whatever leads it.
        self.replace(
            ".agents/skills/SKILL FLOW.md",
            "  -> systematic-debugger\n",
            "  -> systematic-debugger\n"
            "    ├─ PHP evidence? ──no──► recognizable\n"
            "    │   ├─ no  → out of scope\n"
            "    │   └─ yes → offer\n"
            "    ├─ read <target>/.infra-manifest.json  (no manifest → ABORT)\n"
            "    ├─ re-validate profile → compare versions\n"
            "    └─ rewrite manifest → code-reviewer\n"
            "      + reference docs → mirror 3 editions → self-verify\n"
            "    → reports path to new sibling\n"
            "        v                       v\n"
            "       or a specialized implementation skill\n",
        )
        self.assertEqual(self.messages(self.run_gate()), [])

    def test_only_diagram_fences_hold_flow_steps(self) -> None:
        self.replace(
            ".agents/skills/SKILL FLOW.md",
            "- Use `memory` to refresh context.",
            "- Use `memory` to refresh context.\n\n```bash\nmake\n```",
        )
        self.assertEqual(self.messages(self.run_gate()), [])

    def test_skill_flow_backticked_name_with_arguments_must_resolve(self) -> None:
        self.replace(
            ".agents/skills/SKILL FLOW.md",
            "- Use `memory` to refresh context.",
            "- Use `memory` to refresh context; run `coder <task>`, then `ghost <target>`.",
        )
        error = self.assert_single_error(
            self.run_gate(), "routing", ".agents/skills/SKILL FLOW.md:9", "`ghost`"
        )
        self.assertEqual(error.target, "ghost")

    def test_skill_flow_backticked_name_must_resolve(self) -> None:
        self.replace(".claude/skills/SKILL FLOW.md", "Use `memory`", "Use `memroy`")
        self.assert_single_error(
            self.run_gate(), "routing", ".claude/skills/SKILL FLOW.md:9", "memroy"
        )

    def test_skill_flow_phase_map_items_must_resolve(self) -> None:
        self.replace(".cursor/skills/SKILL FLOW.md", "| Utility | `memory` |", "| Utility | memory, ghost |")
        self.assert_single_error(
            self.run_gate(), "routing", ".cursor/skills/SKILL FLOW.md:17", "Phase Map item 'ghost'"
        )

    def test_skill_flow_ignores_paths_and_files(self) -> None:
        self.replace(
            ".claude/skills/SKILL FLOW.md",
            "- Use `memory` to refresh context.",
            "- Use `memory` to refresh context; artifacts land in specs/x and `/tmp/run`, "
            "see <edition>/hooks and memory-bank/README.md.",
        )
        self.assertEqual(self.messages(self.run_gate()), [])

    def test_agents_md_references_must_resolve(self) -> None:
        self.write(
            "AGENTS.md",
            AGENTS_MD
            + "- Run `/flow-bugfix` for regressions.\n"
            + "- Use the `memroy` skill.\n"
            + "- Denied by the `subagent-guard` hook.\n"
            + "- Ask the `reviewer` agent.\n"
            + "- Paths like `<edition>/hooks` and `.env`/secrets are not references.\n",
        )
        report = self.run_gate()
        targets = sorted(item.target for item in report.errors)
        self.assertEqual(targets, ["flow-bugfix", "memroy", "reviewer", "subagent-guard"])
        self.assertTrue(all(item.path.endswith("AGENTS.md") for item in report.errors))

    def test_reference_allowlist_silences_one_reference(self) -> None:
        self.replace(".claude/commands/coder.md", "flow-next: code-reviewer", "flow-next: reviewer")
        allowlist = {
            "references": [
                {
                    "edition": EDITION,
                    "file": ".claude/commands/coder.md",
                    "target": "reviewer",
                    "reason": "Ambiguous: pending a decision on the reviewer rename.",
                }
            ]
        }
        report = self.run_gate(allowlist)
        self.assertEqual(report.findings, [])
        self.assertEqual(len(report.allowlisted), 1)


class ReachabilityTest(RoutesFixture):
    def test_orphan_skill_is_an_error_until_allowlisted(self) -> None:
        self.write(".agents/skills/orphan/SKILL.md", SKILL.format(name="orphan"))
        error = self.assert_single_error(
            self.run_gate(), "reachability", ".agents/skills/orphan/SKILL.md", "'orphan'"
        )
        self.assertIsNone(error.line)

        allowlist = {
            "unreachable_skills": [
                {"edition": EDITION, "skill": "orphan", "reason": "Description-triggered only."}
            ]
        }
        report = self.run_gate(allowlist)
        self.assertEqual(report.findings, [])
        self.assertEqual(len(report.allowlisted), 1)

    def test_a_flow_reference_alone_makes_a_skill_reachable(self) -> None:
        self.write(".agents/skills/council/SKILL.md", SKILL.format(name="council"))
        self.replace(".agents/skills/SKILL FLOW.md", "- Use `memory`", "- Use `council` for trade-offs.\n- Use `memory`")
        self.assertEqual(self.messages(self.run_gate()), [])


class AllowlistTest(RoutesFixture):
    def test_entry_without_reason_or_with_unknown_edition_is_an_error(self) -> None:
        allowlist = {
            "unreachable_skills": [
                {"edition": EDITION, "skill": "coder", "reason": " "},
                {"edition": "Laraval", "skill": "coder", "reason": "typo"},
            ],
            "surprise": [],
        }
        report = self.run_gate(allowlist)
        problems = sorted(item.message for item in report.errors if item.check == "allowlist")
        self.assertEqual(len(problems), 3, problems)
        self.assertTrue(any("non-empty 'reason'" in item for item in problems))
        self.assertTrue(any("unknown edition 'Laraval'" in item for item in problems))
        self.assertTrue(any("unknown section 'surprise'" in item for item in problems))

    def test_malformed_entries_are_findings_not_crashes(self) -> None:
        # Each malformed entry is reported by validate_allowlist and takes no
        # part in matching: no KeyError, no TypeError, and it silences nothing.
        self.write(".agents/skills/orphan/SKILL.md", SKILL.format(name="orphan"))
        self.hook(".codex/hooks/subagent-dispatch.sh")
        self.stage()
        allowlist = {
            "unreachable_skills": [{"edition": "*", "reason": "x", "skil": "orphan"}],
            "unwired_hooks": 5,
            "references": [{"edition": "*", "reason": "x", "file": "AGENTS.md", "target": 7}],
        }
        report = self.run_gate(allowlist)
        problems = sorted(item.message for item in report.errors if item.check == "allowlist")
        self.assertEqual(
            problems,
            [
                "references[0] needs a non-empty 'target'",
                "section 'unwired_hooks' must be a list",
                "unreachable_skills[0] needs a non-empty 'skill'",
            ],
        )
        self.assertEqual([item.target for item in report.errors if item.check == "reachability"], ["orphan"])
        self.assertEqual(len(report.warnings), 1, self.messages(report, "warning"))
        self.assertEqual(report.allowlisted, [])

    def test_stale_entry_is_a_warning(self) -> None:
        allowlist = {
            "unreachable_skills": [
                {"edition": "*", "skill": "long-gone", "reason": "Was description-triggered."}
            ]
        }
        report = self.run_gate(allowlist)
        self.assertEqual(report.errors, [])
        self.assertEqual(len(report.warnings), 1)
        self.assertIn("stale entry unreachable_skills[0]", report.warnings[0].message)

    def test_filtered_run_does_not_call_a_wildcard_entry_stale(self) -> None:
        allowlist = {
            "unreachable_skills": [
                {"edition": "*", "skill": "long-gone", "reason": "Was description-triggered."}
            ]
        }
        report = check_routes.check_repo(self.root, [EDITION], allowlist, filtered=True)
        self.assertEqual(report.findings, [])


class CommandLineTest(RoutesFixture):
    def run_main(self, *args: str) -> tuple:
        allowlist = self.root / "allowlist.json"
        if not allowlist.exists():
            allowlist.write_text("{}\n", encoding="utf-8")
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            status = check_routes.main(
                ["--root", str(self.root), "--allowlist", str(allowlist), "--edition", EDITION, *args]
            )
        return status, stdout.getvalue()

    def test_clean_run_exits_zero_with_a_grouped_report(self) -> None:
        status, output = self.run_main()
        self.assertEqual(status, 0, output)
        self.assertIn(f"{EDITION}: ok", output)
        self.assertIn("0 error(s), 0 warning(s)", output)

    def test_errors_exit_one_and_print_file_and_line(self) -> None:
        self.replace(".claude/commands/coder.md", "spawns: coder-agent", "spawns: codder-agent")
        status, output = self.run_main()
        self.assertEqual(status, 1)
        self.assertIn(f"{EDITION}/.claude/commands/coder.md:2", output)

    def test_json_report(self) -> None:
        self.replace(".claude/commands/coder.md", "spawns: coder-agent", "spawns: codder-agent")
        status, output = self.run_main("--json")
        self.assertEqual(status, 1)
        data = json.loads(output)
        self.assertEqual(data["editions"], [EDITION])
        self.assertEqual(data["errors"], 1)
        finding = data["findings"][0]
        self.assertEqual(finding["check"], "routing")
        self.assertEqual(finding["line"], 2)
        self.assertEqual(finding["target"], "codder-agent")

    def test_non_utf8_file_is_a_finding_with_valid_json(self) -> None:
        path = self.edition / ".claude/agents/coder-agent.md"
        path.write_bytes(path.read_bytes() + b"\n\xe9t\xe9\n")
        status, output = self.run_main("--json")
        self.assertEqual(status, 1, output)
        data = json.loads(output)
        findings = [
            item for item in data["findings"]
            if item["path"] == f"{EDITION}/.claude/agents/coder-agent.md"
        ]
        self.assertEqual(len(findings), 1, data["findings"])
        self.assertEqual(findings[0]["severity"], "error")
        self.assertIn("not valid UTF-8", findings[0]["message"])
        # The rest of the file was still checked: nothing else broke.
        self.assertEqual(data["errors"], 1, data["findings"])

    def test_malformed_allowlist_entry_exits_one_with_valid_json(self) -> None:
        self.hook(".codex/hooks/subagent-dispatch.sh")
        self.stage()
        (self.root / "allowlist.json").write_text(
            json.dumps(
                {"unwired_hooks": [{"edition": "*", "reason": "x", "pth": ".codex/hooks/subagent-dispatch.sh"}]}
            ),
            encoding="utf-8",
        )
        status, output = self.run_main("--json")
        self.assertEqual(status, 1, output)
        data = json.loads(output)
        self.assertEqual(data["errors"], 1)
        self.assertIn("needs a non-empty 'path'", data["findings"][0]["message"])

    def test_unreadable_allowlist_exits_two_with_valid_json(self) -> None:
        (self.root / "allowlist.json").write_bytes(b'{"_comment": "caf\xe9"}\n')
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            status, output = self.run_main("--json")
        self.assertEqual(status, 2)
        self.assertIn("not valid UTF-8", json.loads(output)["error"])
        self.assertIn("not valid UTF-8", stderr.getvalue())

    def test_edition_aliases(self) -> None:
        self.assertEqual(check_routes.resolve_edition("wordpress"), "Cms/wordpress")
        self.assertEqual(check_routes.resolve_edition("PHP Core"), "PHP Core")
        with self.assertRaises(Exception):
            check_routes.resolve_edition("Drupal")


class ParsingTest(unittest.TestCase):
    def test_name_lists(self) -> None:
        self.assertEqual(check_routes.name_list("[a, b]"), ["a", "b"])
        self.assertEqual(check_routes.name_list("a"), ["a"])
        self.assertEqual(check_routes.name_list("null"), [])
        self.assertEqual(check_routes.name_list("[]"), [])
        self.assertEqual(check_routes.name_list("\n  - a\n  - 'b'"), ["a", "b"])
        self.assertEqual(check_routes.name_list("[a,\n  b]"), ["a", "b"])

    def test_frontmatter_lines(self) -> None:
        lines = ["---", "spawns: x-agent", "flow-alternatives:", "  - a", "---", "body"]
        frontmatter, body = check_routes.parse_frontmatter(lines)
        self.assertEqual(frontmatter["spawns"], ("x-agent", 2))
        self.assertEqual(check_routes.name_list(frontmatter["flow-alternatives"][0]), ["a"])
        self.assertEqual(body, 5)


class LiveRepositoryTest(unittest.TestCase):
    def test_repository_routes_resolve(self) -> None:
        allowlist = check_routes.load_allowlist(check_routes.DEFAULT_ALLOWLIST)
        report = check_routes.check_repo(ROOT, check_routes.EDITIONS, allowlist)
        errors = [
            f"{item.location()} [{item.check}] {item.message}" for item in report.errors
        ]
        self.assertEqual(errors, [], "\n".join(errors))


if __name__ == "__main__":
    unittest.main()
