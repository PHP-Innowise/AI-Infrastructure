"""Provider boundaries, tested offline with documented NDJSON shapes."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness" / "src"))
from harness import providers


class BudgetTests(unittest.TestCase):
    def test_native_cap_and_cached_token_accounting(self):
        with tempfile.TemporaryDirectory() as project:
            command=providers.build_command('claude','claude',Path(project),'Task',budget_usd=.5)
            self.assertEqual(command[command.index('--max-budget-usd')+1],'0.5')
            for provider,amount in [('codex',1),('cursor',1),('claude',float('inf')),('claude',True)]:
                with self.assertRaises(ValueError): providers.build_command(provider,provider,Path(project),'Task',budget_usd=amount)
        self.assertEqual(providers.total_tokens({'input_tokens':10,'output_tokens':2,'cached_input_tokens':8}),12)
        self.assertEqual(providers.total_tokens({'input_tokens':10,'output_tokens':2,'cache_read_input_tokens':8,'cache_creation_input_tokens':3}),23)
        self.assertIsNone(providers.total_tokens({'input_tokens':10}))
        self.assertIsNone(providers.total_tokens({'input_tokens':True,'output_tokens':2}))


class DiscoveryTests(unittest.TestCase):
    def test_discovery_checks_identity_and_never_starts_agents(self):
        def probe(command, **kwargs):
            label = {"claude": "2.1.259 (Claude Code)", "codex": "codex-cli 0.153.2",
                     "agent": "grok 1.0.0 - supports Cursor imports"}[Path(command[0]).name]
            return subprocess.CompletedProcess(command, 0, label, "")
        def which(name):
            return "/fake/" + name if name in ("claude", "codex", "agent") else None
        with tempfile.TemporaryDirectory() as home, patch.object(Path, "home", return_value=Path(home)), \
                patch.object(providers.shutil, "which", side_effect=which), \
                patch.object(providers.subprocess, "run", side_effect=probe) as run:
            found = providers.discover_providers()
        self.assertEqual([item["available"] for item in found], [True, True, False])
        self.assertIsNone(found[2]["executable"])
        self.assertIn("--cursor-bin", found[2]["detail"])
        self.assertNotIn("grok 1.0.0", found[2]["detail"])
        self.assertIn("Ultracode", found[0]["agent_control_detail"])
        self.assertIn("main agent", found[1]["agent_control_detail"])
        self.assertIn("Instruction only", found[2]["agent_control_detail"])
        self.assertTrue(all(set(item["model_options"]) == {"models", "efforts", "detail"} for item in found))
        for call in run.call_args_list:
            command = call.args[0]
            self.assertTrue(command[-1] in ("--version", "--help")
                            or command[1:] == ["debug", "models", "--bundled"])
            if Path(call.args[0][0]).name == "agent":
                self.assertIn("--disable-auto-update", call.args[0])
            self.assertEqual(call.kwargs["timeout"], providers.PROBE_TIMEOUT)
            self.assertEqual(call.kwargs["stdin"], subprocess.DEVNULL)

    def test_cursor_override_needs_identity_and_required_flags(self):
        help_text = "Cursor Agent --print --output-format --resume --workspace --sandbox --mode"
        with tempfile.TemporaryDirectory() as home, patch.object(Path, "home", return_value=Path(home)), \
                patch.object(providers.shutil, "which", side_effect=lambda name: name if name == "/opt/cursor-agent" else None), \
                patch.object(providers.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, help_text, "")) as run:
            cursor = providers.discover_providers({"cursor": "/opt/cursor-agent"})[2]
        self.assertTrue(cursor["available"])
        self.assertEqual(cursor["executable"], "/opt/cursor-agent")
        self.assertEqual([call.args[0] for call in run.call_args_list], [
            ["/opt/cursor-agent", "--disable-auto-update", "--version"],
            ["/opt/cursor-agent", "--disable-auto-update", "--help"],
        ])

    def test_failed_or_timed_out_probe_is_unavailable_without_raw_errors(self):
        for failure in (OSError("PRIVATE PATH"), subprocess.TimeoutExpired("PRIVATE COMMAND", 3)):
            with self.subTest(failure=failure), patch.object(providers.shutil, "which", return_value="/fake/bin"), \
                    patch.object(providers.subprocess, "run", side_effect=failure):
                found = providers.discover_providers({name: "/fake/bin" for name in providers.PROVIDERS})
            self.assertTrue(all(not item["available"] for item in found))
            self.assertNotIn("PRIVATE", json.dumps(found))


class CommandTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.project = Path(self.tmp.name)
        self.prompt = "--dangerously-bypass-approvals-and-sandbox\n$(touch nope)"

    def test_claude_plan_edit_and_resume_use_stdin(self):
        for mode, permission in (("plan", "plan"), ("edit", "acceptEdits")):
            command = providers.build_command("claude", "/bin/claude", self.project, self.prompt,
                                              mode, "sonnet", "session-1")
            settings_index = command.index("--settings")
            settings = json.loads(command[settings_index + 1])
            self.assertEqual(settings, {"env": {"CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS": "1",
                                                "CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH": "1"},
                                        "ultracode": False, "disableWorkflows": True})
            del command[settings_index:settings_index + 2]
            self.assertEqual(command, ["/bin/claude", "--print", "--output-format", "stream-json",
                                      "--verbose", "--permission-mode", permission,
                                      "--disallowedTools", "Agent", "Task", "Workflow",
                                      "--resume", "session-1", "--model", "sonnet"])
            self.assertEqual(providers.input_text("claude", self.prompt), self.prompt)

    def test_codex_resume_retains_global_sandbox_and_never_approval(self):
        for mode, sandbox in (("plan", "read-only"), ("edit", "workspace-write")):
            for session in (None, "session-1"):
                command = providers.build_command("codex", "/bin/codex", self.project, self.prompt,
                                                  mode, "model-name", session)
                prefix = ["/bin/codex", "--ask-for-approval", "never", "--sandbox", sandbox,
                          "--cd", str(self.project), "-c", "agents.enabled=false",
                          "-c", "features.multi_agent=false", "-c", "features.multi_agent_v2=false", "exec"]
                tail = (["resume"] if session else []) + ["--json", "--model", "model-name"]
                tail += ([session] if session else []) + ["-"]
                self.assertEqual(command, prefix + tail)
                self.assertNotIn(self.prompt, command)
                self.assertEqual(providers.input_text("codex", self.prompt), self.prompt)

    def test_cursor_prompt_is_one_positional_argument_and_no_force(self):
        for mode in ("plan", "edit"):
            command = providers.build_command("cursor", "/bin/cursor-agent", self.project, self.prompt,
                                              mode, None, "session-1")
            self.assertEqual(command[-2:], ["--", self.prompt])
            self.assertIn("--disable-auto-update", command)
            self.assertIn("--sandbox", command)
            self.assertIn("enabled", command)
            self.assertIn("--resume", command)
            self.assertEqual("--mode" in command, mode == "plan")
            self.assertFalse({"--force", "--yolo", "--trust", "--approve-mcps", "--stream-partial-output"} & set(command))
            self.assertIsNone(providers.input_text("cursor", self.prompt))

    def test_enabled_claude_uses_concurrency_and_depth_limits_on_fresh_and_resumed_runs(self):
        for count in (1, 3, 40):
            for session in (None, "session-1"):
                command = providers.build_command("claude", "/bin/claude", self.project, "hello",
                                                  session_id=session, agents_enabled=True, agent_count=count)
                deny_index = command.index("--disallowedTools")
                self.assertEqual(command[deny_index + 1:command.index("--settings")], ["Workflow"])
                settings = json.loads(command[command.index("--settings") + 1])
                self.assertEqual(settings, {"env": {
                    "CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS": str(count),
                    "CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH": "1",
                }, "ultracode": False, "disableWorkflows": True})
                self.assertNotIn("CLAUDE_CODE_MAX_SUBAGENTS_PER_SESSION", json.dumps(settings))
                self.assertEqual(command[command.index('--allowedTools') + 1:command.index('--allowedTools') + 3], ['Agent', 'Task'])

    def test_codex_enabled_limits_count_additional_helpers_for_both_backends(self):
        for count in (1, 3, 40):
            for session in (None, "session-1"):
                command = providers.build_command("codex", "/bin/codex", self.project, "hello",
                                                  session_id=session, agents_enabled=True, agent_count=count)
                configs = [command[index + 1] for index, value in enumerate(command) if value == "-c"]
                self.assertEqual(configs, ["agents.enabled=true", "features.multi_agent=true",
                    f"agents.max_concurrent_threads_per_session={count}",
                    "features.multi_agent_v2={enabled=true,"
                    f"max_concurrent_threads_per_session={count + 1}}}"])
                self.assertTrue(all(index < command.index("exec") for index, value in enumerate(command) if value == "-c"))
                self.assertEqual(command[-1], "-")
                self.assertEqual("resume" in command, session is not None)

    def test_cursor_agent_options_do_not_invent_cli_flags(self):
        command = providers.build_command("cursor", "/bin/cursor-agent", self.project, "hello")
        self.assertEqual(command, providers.build_command(
            "cursor", "/bin/cursor-agent", self.project, "hello", agents_enabled=True, agent_count=40))

    def test_agent_environment_contains_only_overrides_and_validates_types(self):
        self.assertEqual(providers.agent_environment("claude", True, 40), {
            "CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS": "40", "CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH": "1"})
        self.assertEqual(providers.agent_environment("claude", False, 40), {
            "CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS": "1", "CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH": "1"})
        for provider in ("cursor", "codex"):
            self.assertEqual(providers.agent_environment(provider, True, 3), {})
        with self.assertRaises(ValueError):
            providers.agent_environment("unknown", True, 3)
        invalid = [(enabled, 3) for enabled in (0, 1, None, "true", [])]
        invalid += [(True, count) for count in (False, True, 0, 41, 1.5, "3", None)]
        for enabled, count in invalid:
            with self.subTest(enabled=enabled, count=count):
                with self.assertRaises(ValueError):
                    providers.agent_environment("claude", enabled, count)
                with self.assertRaises(ValueError):
                    providers.build_command("claude", "/bin/claude", self.project, "hello",
                                            agents_enabled=enabled, agent_count=count)

    def test_explicit_effort_reaches_native_cli_and_resume_without_shell(self):
        metadata = {"models": [{"id": "fixture-model", "label": "Fixture", "efforts": ["high"]}],
                    "efforts": ["high"], "detail": "Offline fixture"}
        with patch.object(providers, "model_options", return_value=metadata):
            for session in (None, "native-session"):
                claude = providers.build_command("claude", "/bin/claude", self.project, "hello",
                    model="fixture-model", session_id=session, thinking_effort="high")
                self.assertEqual(claude[claude.index("--effort") + 1], "high")
                settings = json.loads(claude[claude.index("--settings") + 1])
                self.assertEqual(settings["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "high")
                self.assertEqual(settings["env"]["CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH"], "1")
                self.assertEqual(claude[claude.index("--model") + 1], "fixture-model")
                codex = providers.build_command("codex", "/bin/codex", self.project, "hello",
                    model="fixture-model", session_id=session, thinking_effort="high")
                position = codex.index('model_reasoning_effort="high"')
                self.assertEqual(codex[position - 1], "-c")
                self.assertLess(position, codex.index("exec"))
                self.assertEqual(codex[codex.index("--model") + 1], "fixture-model")
                self.assertEqual("resume" in codex, session is not None)

    def test_default_effort_leaves_native_configuration_in_control(self):
        for provider in providers.PROVIDERS:
            command = providers.build_command(provider, "/bin/native", self.project, "hello")
            self.assertNotIn("--effort", command)
            self.assertFalse(any("model_reasoning_effort" in value for value in command))
            self.assertFalse(any("CLAUDE_CODE_EFFORT_LEVEL" in value for value in command))
            if provider == "claude":
                settings = json.loads(command[command.index("--settings") + 1])
                self.assertIs(settings["ultracode"], False)
                self.assertIs(settings["disableWorkflows"], True)
        variant = "custom-model[effort=high,context=long]"
        command = providers.build_command("cursor", "/bin/cursor-agent", self.project, "hello", model=variant)
        self.assertEqual(command[command.index("--model") + 1], variant)
        with self.assertRaises(ValueError):
            providers.build_command("cursor", "/bin/cursor-agent", self.project, "hello", model=variant,
                                    thinking_effort="high")

    def test_ultracode_enters_and_leaves_native_workflow_mode_on_resume(self):
        for session in (None, "native-session"):
            for effort in ("ultracode", "high", None, "ultracode"):
                with self.subTest(session=session, effort=effort):
                    command = providers.build_command("claude", "/fake/claude", self.project, "Hello",
                        model="claude-opus-5", session_id=session, thinking_effort=effort,
                        agents_enabled=True, agent_count=3)
                    settings = json.loads(command[command.index("--settings") + 1])
                    if effort == "ultracode":
                        self.assertEqual(command[command.index("--effort") + 1], "ultracode")
                        self.assertEqual(command[command.index("--allowedTools") + 1], "Workflow")
                        self.assertNotIn("--disallowedTools", command)
                        self.assertIs(settings["ultracode"], True)
                        self.assertEqual(settings["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "xhigh")
                        self.assertNotIn("disableWorkflows", settings)
                        self.assertNotIn("enableWorkflows", settings)
                    else:
                        self.assertIs(settings["ultracode"], False)
                        self.assertIs(settings["disableWorkflows"], True)
                        self.assertEqual(command[command.index("--disallowedTools") + 1], "Workflow")
        with self.assertRaisesRegex(ValueError, "requires additional agents"):
            providers.build_command("claude", "/fake/claude", self.project, "Hello",
                                    model="claude-opus-5", thinking_effort="ultracode")

    def test_untrusted_options_and_invalid_input_are_rejected(self):
        for kwargs in ({"provider": "unknown"}, {"mode": "bypass"}, {"model": "--force"},
                       {"session_id": "--last"}, {"session_id": "../outside"},
                       {"prompt": "\0"}, {"prompt": ""}, {"project": self.project / "missing"}):
            arguments = dict(provider="codex", executable="/bin/codex", project=self.project, prompt="hello")
            arguments.update(kwargs)
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                providers.build_command(**arguments)


class ModelOptionsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.cache = self.root / "models_cache.json"
        self.environ = patch.dict(providers.os.environ, {"CODEX_HOME": str(self.root)})
        self.environ.start()
        self.addCleanup(self.environ.stop)
        self.which = patch.object(providers.shutil, "which", return_value=None)
        self.which.start()
        self.addCleanup(self.which.stop)
        providers._model_options.cache_clear()
        self.addCleanup(providers._model_options.cache_clear)

    def model(self, slug="fixture-model", **changes):
        result = {"slug": slug, "display_name": "Fixture model", "visibility": "list", "priority": 1,
                  "supported_reasoning_levels": [{"effort": "low"}, {"effort": "high"}]}
        result.update(changes)
        return result

    def test_codex_cache_exposes_only_public_metadata_and_keeps_subscription_models(self):
        self.cache.write_text(json.dumps({"fetched_at": "PRIVATE TIMESTAMP", "models": [
            self.model(base_instructions="PRIVATE PROMPT", secret="PRIVATE SECRET"),
            self.model("hidden-model", visibility="hide"),
            self.model("subscription-model", display_name="Subscription model", priority=0,
                       supported_in_api=False, supported_reasoning_levels=[{"effort": "xhigh"}]),
        ]}))
        result = providers.model_options("codex")
        self.assertEqual(result["models"], [
            {"id": "subscription-model", "label": "Subscription model", "efforts": ["xhigh"]},
            {"id": "fixture-model", "label": "Fixture model", "efforts": ["low", "high"]},
        ])
        self.assertIn("Local cached", result["detail"])
        self.assertNotIn("PRIVATE", json.dumps(result))

    def test_missing_cache_uses_offline_bundled_command_once_and_returns_copies(self):
        payload = json.dumps({"models": [self.model()]}).encode()
        def response(command, **_):
            output = payload if command[-1] == "--bundled" else "codex-cli 0.153.2"
            return subprocess.CompletedProcess(command, 0, output, "")
        with patch.object(providers.shutil, "which", return_value="/fake/codex"), \
                patch.object(providers.subprocess, "run", side_effect=response) as run:
            first = providers.model_options("codex")
            first["models"][0]["label"] = "MUTATED"
            second = providers.model_options("codex")
        self.assertEqual(second["models"][0]["label"], "Fixture model")
        self.assertIn("Bundled offline", second["detail"])
        self.assertEqual(run.call_count, 3)  # identity probes, then one cached catalog load
        self.assertEqual(run.call_args.args[0], ["/fake/codex", "debug", "models", "--bundled"])
        self.assertEqual(run.call_args.kwargs["stdin"], subprocess.DEVNULL)
        self.assertEqual(run.call_args.kwargs["timeout"], providers.PROBE_TIMEOUT)

    def test_bundled_fallback_never_runs_an_unverified_codex_executable(self):
        with patch.object(providers.shutil, "which", return_value="/fake/codex"), \
                patch.object(providers.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "Other tool", "")) as run:
            self.assertEqual(providers.model_options("codex")["models"], [])
        self.assertEqual([call.args[0][-1] for call in run.call_args_list], ["--version", "--help"])

    def test_cache_symlinks_oversize_and_malformed_values_are_rejected(self):
        outside = self.root / "outside.json"
        outside.write_text(json.dumps({"models": [self.model("must-not-read")]}))
        self.cache.symlink_to(outside)
        self.assertEqual(providers.model_options("codex")["models"], [])
        self.cache.unlink()
        for payload in (b"{invalid json", b"[]", json.dumps({"models": [None, self.model(slug=[]),
                self.model("--option"), self.model("no-levels", supported_reasoning_levels={})]}).encode()):
            providers._model_options.cache_clear()
            self.cache.write_bytes(payload)
            self.assertEqual(providers.model_options("codex")["models"], [])
        providers._model_options.cache_clear()
        with self.cache.open("wb") as handle:
            handle.truncate(providers.MODEL_CATALOG_LIMIT + 1)
        self.assertEqual(providers.model_options("codex")["models"], [])

    def test_fallback_timeout_is_safe_and_cursor_never_queries_account_models(self):
        with patch.object(providers.shutil, "which", return_value="/fake/codex"), \
                patch.object(providers.subprocess, "run", side_effect=subprocess.TimeoutExpired("PRIVATE", 3)) as run:
            codex = providers.model_options("codex")
            cursor = providers.model_options("cursor")
        self.assertEqual(codex["models"], [])
        self.assertNotIn("PRIVATE", json.dumps(codex))
        self.assertEqual(cursor["models"], [])
        self.assertEqual(cursor["efforts"], [])
        self.assertEqual(run.call_count, 1)

    def test_model_effort_validation_uses_known_combinations_and_preserves_custom_ids(self):
        self.cache.write_text(json.dumps({"models": [self.model()]}))
        providers.validate_model_effort("codex", "fixture-model", "low")
        providers.validate_model_effort("codex", "custom-deployment", "ultra")
        providers.validate_model_effort("claude", "opus", "max")
        providers.validate_model_effort("claude", "haiku", None)
        providers.validate_model_effort("claude", "claude-opus-4-6", "max")
        for model in ("claude-fable-5-1", "claude-fable-5", "claude-opus-5",
                      "claude-opus-4-8", "claude-opus-4-7", "claude-sonnet-5"):
            providers.validate_model_effort("claude", model, "xhigh")
            providers.validate_model_effort("claude", model, "ultracode", agents_enabled=True)
        for model in ("claude-opus-4-6", "claude-sonnet-4-6", "haiku", "claude-haiku-4-5-20251001"):
            with self.subTest(model=model), self.assertRaises(ValueError):
                providers.validate_model_effort("claude", model, "ultracode", agents_enabled=True)
        for arguments in (("codex", "fixture-model", "ultra"), ("claude", "haiku", "low"),
                          ("claude", "claude-haiku-4-5-20251001", "low"),
                          ("claude", "claude-opus-4-6", "xhigh"), ("cursor", "custom", "high"),
                          ("unknown", None, None)):
            with self.subTest(arguments=arguments), self.assertRaises(ValueError):
                providers.validate_model_effort(*arguments)
        for value in (True, 1, [], {}, "", "ultracode", "HIGH", "--option", "high\n"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                providers.validate_model_effort("claude", "opus", value)

    def test_versioned_picker_choices_send_explicit_ids_on_create_and_resume(self):
        choices = {item["label"]: item["id"] for item in providers.model_options("claude")["models"]}
        for label, identifier in (("Fable 5.1", "claude-fable-5-1"), ("Opus 5", "claude-opus-5")):
            self.assertEqual(choices[label], identifier)
            for session in (None, "session-1"):
                command = providers.build_command("claude", "/fake/claude", self.root, "Hello",
                                                  model=choices[label], session_id=session, thinking_effort="low")
                self.assertEqual(command[command.index("--model") + 1], identifier)
        self.assertEqual(choices["Fable (auto)"], "fable")
        self.assertEqual(choices["Opus (auto)"], "opus")


class EventTests(unittest.TestCase):
    def test_claude_public_text_tools_session_and_terminal_failure(self):
        init = {"type": "system", "subtype": "init", "session_id": "session-1", "apiKeySource": "SECRET"}
        self.assertEqual(providers.normalize_event("claude", init), [{"kind": "session", "native_session_id": "session-1"}])
        message = {"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "thinking", "thinking": "PRIVATE REASONING"},
            {"type": "text", "text": "Checking the file."},
            {"type": "tool_use", "name": "Read", "input": {"path": "PRIVATE INPUT"}},
        ]}}
        events = providers.normalize_event("claude", message)
        self.assertEqual([item["kind"] for item in events], ["text", "tool"])
        self.assertNotIn("PRIVATE", json.dumps(events))
        failed = providers.normalize_event("claude", {"type": "result", "subtype": "error_max_turns",
            "is_error": True, "errors": ["Turn limit reached"], "session_id": "session-1",
            "total_cost_usd": 0.2, "usage": {"input_tokens": 10, "output_tokens": 2}})
        self.assertEqual(failed[-1]["kind"], "result")
        self.assertFalse(failed[-1]["ok"])
        self.assertIn("Turn limit", failed[-1]["text"])
        self.assertEqual(failed[0]["cost_usd"], 0.2)

    def test_codex_thread_text_tools_usage_and_terminal_results(self):
        self.assertEqual(providers.normalize_event("codex", {"type": "thread.started", "thread_id": "thread-1"}),
                         [{"kind": "session", "native_session_id": "thread-1"}])
        self.assertEqual(providers.normalize_event("codex", {"type": "item.completed", "item": {
            "type": "reasoning", "text": "PRIVATE REASONING"}}), [])
        message = {"type": "item.completed", "item": {"type": "agent_message", "text": "Done"}}
        self.assertEqual(providers.normalize_event("codex", message), [{"kind": "text", "text": "Done"}])
        tool = {"type": "item.completed", "item": {"type": "command_execution", "status": "completed",
                "exit_code": 1, "command": "PRIVATE INPUT", "aggregated_output": "PRIVATE OUTPUT"}}
        result = providers.normalize_event("codex", tool)
        self.assertFalse(result[0]["ok"])
        self.assertNotIn("PRIVATE", json.dumps(result))
        terminal = providers.normalize_event("codex", {"type": "turn.completed", "usage": {
            "input_tokens": 11, "cached_input_tokens": 3, "output_tokens": 2}})
        self.assertEqual(terminal[0]["input_tokens"], 11)
        self.assertTrue(terminal[-1]["ok"])
        failed = providers.normalize_event("codex", {"type": "turn.failed", "error": {"message": "Request failed"}})
        self.assertEqual(failed, [{"kind": "result", "ok": False, "text": "Request failed"}])

    def test_cursor_public_events_do_not_include_tool_payloads_or_partial_duplicates(self):
        message = {"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "text", "text": "I will read README."}]}}
        self.assertEqual(providers.normalize_event("cursor", message)[0]["text"], "I will read README.")
        self.assertEqual(providers.normalize_event("cursor", dict(message, timestamp_ms=123)), [])
        tool = {"type": "tool_call", "subtype": "completed", "tool_call": {"readToolCall": {
            "args": {"path": "README.md"}, "result": {"success": {"content": "PRIVATE FILE CONTENT"}}}}}
        self.assertEqual(providers.normalize_event("cursor", tool),
                         [{"kind": "tool", "text": "readToolCall: completed", "ok": True}])
        final = {"type": "result", "subtype": "success", "is_error": False,
                 "result": "Complete answer", "session_id": "session-1"}
        normalized = providers.normalize_event("cursor", final)
        self.assertEqual(normalized, [{"kind": "result", "ok": True, "text": "Complete answer",
                                      "native_session_id": "session-1"}])

    def test_delegation_requirements_and_receipts_distinguish_attempts_from_helpers(self):
        for provider in providers.PROVIDERS:
            for count in (1, 40):
                requirement = providers.delegation_instructions(provider, True, count)
                self.assertIn(f'MUST launch exactly {count} distinct additional agents', requirement)
                self.assertIn(f'at most {count} additional agents', requirement)
                self.assertIn('state the concrete reason', requirement)
                self.assertIn('disabled', providers.delegation_instructions(provider, False, count))
        tracker = providers.DelegationTracker('codex')
        tracker.observe({'type':'item.completed','item':{'type':'agent_message','text':'I used 40 helpers'}})
        self.assertEqual(tracker.summary()['status'], 'unconfirmed')
        item = {'type':'collab_tool_call','tool':'spawn_agent','status':'failed','receiver_thread_ids':['PRIVATE']}
        tracker.observe({'type':'item.completed','item':item})
        self.assertTrue(tracker.attempted)
        self.assertFalse(tracker.confirmed)
        tracker.observe({'type':'item.completed','item':{**item, 'status':'completed', 'receiver_thread_ids':[]}})
        self.assertFalse(tracker.confirmed)
        tracker.observe({'type':'item.completed','item':{**item, 'status':'completed'}})
        self.assertEqual(tracker.summary()['status'], 'confirmed')
        self.assertNotIn('PRIVATE', json.dumps(tracker.summary()))
        for tool, failed, expected in (('Agent', False, True), ('Task', False, True), ('Agent', True, False), ('Workflow', False, False)):
            tracker = providers.DelegationTracker('claude')
            tracker.observe({'type':'assistant','message':{'content':[{'type':'tool_use','name':tool,'id':'PRIVATE'}]}})
            self.assertTrue(tracker.attempted)
            self.assertFalse(tracker.confirmed)
            tracker.observe({'type':'user','message':{'content':[{'type':'tool_result','tool_use_id':'PRIVATE','is_error':failed}]}})
            self.assertEqual(tracker.confirmed, expected)
            self.assertNotIn('PRIVATE', json.dumps(tracker.summary()))
        self.assertEqual(providers.DelegationTracker('cursor').summary()['status'], 'unconfirmed')

    def test_codex_v2_subagent_activity_confirms_helpers_without_exposing_identity(self):
        # Codex 0.153.2 V2 emits SubAgentActivity instead of a spawn collab call.
        # Native rollout item fields are serialized in exec JSON as snake_case.
        item = {'type':'sub_agent_activity', 'id':'PRIVATE CALL', 'kind':'started',
                'agent_thread_id':'01a077b1-db6a-7460-b717-293ca1ecda3e',
                'agent_path':'/root/PRIVATE'}
        for kind in ('started', 'completed'):
            event = {'type':'item.completed','item':{**item,'kind':kind}}
            tracker = providers.DelegationTracker('codex')
            tracker.observe(event)
            self.assertEqual(tracker.summary()['status'], 'confirmed')
            normalized = providers.normalize_event('codex', event)
            self.assertEqual(normalized, [{'kind':'tool','text':f'Agent activity: {kind}','ok':True}])
            self.assertNotIn('PRIVATE', json.dumps(normalized))
            self.assertNotIn(item['agent_thread_id'], json.dumps(normalized))
        for changes in ({'kind':'unknown'}, {'agent_thread_id':''}, {'agent_thread_id':None},
                        {'agent_thread_id':[]}, {'agent_thread_id':'bad\nidentity'}):
            event = {'type':'item.completed','item':{**item,**changes}}
            tracker = providers.DelegationTracker('codex'); tracker.observe(event)
            self.assertEqual(tracker.summary()['status'], 'unconfirmed')
            self.assertEqual(providers.normalize_event('codex',event), [])
        event = {'type':'unrelated','item':item}
        tracker = providers.DelegationTracker('codex'); tracker.observe(event)
        self.assertEqual(tracker.summary()['status'], 'unconfirmed')

    def test_required_counts_deduplicate_helpers_and_detect_shortfall_or_excess(self):
        tracker = providers.DelegationTracker('codex', 3)
        for phase in ('started', 'completed', 'completed'):
            tracker.observe({'type':'item.completed','item':{'type':'sub_agent_activity',
                'kind':phase,'agent_thread_id':'child-1'}})
        tracker.observe({'type':'item.completed','item':{'type':'collab_tool_call',
            'tool':'spawn_agent','status':'completed','receiver_thread_ids':['child-1','child-2']}})
        summary = tracker.summary()
        self.assertEqual((summary['status'],summary['confirmed_count'],summary['required_count'],summary['missing_count']),
                         ('partial',2,3,1))
        with patch.object(providers, 'codex_journal_activity', return_value=[
                {'phase':'started','agent_id':'child-1'}, {'phase':'completed','agent_id':'child-1'},
                {'phase':'started','agent_id':'child-3'}, {'phase':'completed','agent_id':'child-3'}]):
            tracker.reconcile('parent', Path('/fixture'), 0, 1)
            self.assertEqual(tracker.reconcile('parent', Path('/fixture'), 0, 1), [])
        self.assertEqual((tracker.summary()['status'],tracker.summary()['confirmed_count']), ('confirmed',3))
        self.assertNotIn('child-', json.dumps(tracker.summary()))
        tracker.observe({'type':'item.completed','item':{'type':'sub_agent_activity',
            'kind':'started','agent_thread_id':'child-4'}})
        self.assertEqual(tracker.summary()['status'], 'exceeded')
        claude = providers.DelegationTracker('claude', 2)
        for tool_id, inputs in (('call-1',{}), ('resume-call',{'resume':'existing-agent'}), ('call-2',{})):
            claude.observe({'type':'assistant','message':{'content':[
                {'type':'tool_use','name':'Agent','id':tool_id,'input':inputs}]}})
            claude.observe({'type':'system','subtype':'task_started','tool_use_id':tool_id})
            for _ in range(2):
                claude.observe({'type':'user','message':{'content':[
                    {'type':'tool_result','tool_use_id':tool_id,'is_error':False}]}})
            if tool_id == 'resume-call':
                self.assertEqual(claude.summary()['confirmed_count'], 1)
        self.assertEqual((claude.summary()['status'],claude.summary()['confirmed_count']), ('confirmed',2))

    def test_codex_collaboration_events_expose_only_allowlisted_tool_and_status(self):
        for tool in ("spawn_agent", "send_input", "wait", "close_agent"):
            for phase, status in (("started", "in_progress"), ("updated", "in_progress"),
                                  ("completed", "completed"), ("completed", "failed")):
                item = {"id": "PRIVATE ITEM", "type": "collab_tool_call", "tool": tool,
                        "status": status, "sender_thread_id": "PRIVATE SENDER",
                        "receiver_thread_ids": ["PRIVATE RECEIVER"], "prompt": "PRIVATE PROMPT",
                        "agents_states": {"PRIVATE RECEIVER": {"status": "running", "message": "PRIVATE MESSAGE"}}}
                event = {"type": "item." + phase, "item": item}
                with self.subTest(tool=tool, status=status, phase=phase):
                    self.assertEqual(providers.normalize_event("codex", event), [
                        {"kind": "tool", "text": f"Agent {tool}: {status}", "ok": status != "failed"}])
        for key in ("tool", "status"):
            malformed = {"type": "collab_tool_call", "tool": "spawn_agent", "status": "completed"}
            malformed[key] = "PRIVATE UNSUPPORTED VALUE"
            self.assertEqual(providers.normalize_event("codex", {"type": "item.completed", "item": malformed}), [])

    def test_text_and_partial_errors_are_not_success_receipts(self):
        for provider in providers.PROVIDERS:
            error = providers.normalize_event(provider, {"type": "error", "error": {"message": "Connection lost"}})
            self.assertEqual(error, [{"kind": "error", "text": "Connection lost", "ok": False}])
            self.assertEqual(providers.normalize_event(provider, {"type": "stream_event", "event": {
                "type": "content_block_delta", "delta": {"type": "thinking_delta", "thinking": "SECRET"}}}), [])
        for payload in ({"type": "result"}, {"type": "result", "subtype": "success", "is_error": True}):
            self.assertFalse(providers.normalize_event("claude", payload)[-1]["ok"])
        self.assertEqual(providers.normalize_event("cursor", {"type": "turn.completed"}), [])

    def test_malformed_payloads_and_nonfinite_usage_are_not_forwarded(self):
        for event in (None, [], {"type": []}, {"type": "assistant", "message": []},
                      {"type": "item.completed", "item": []}):
            for provider in providers.PROVIDERS:
                with self.subTest(event=event, provider=provider):
                    self.assertEqual(providers.normalize_event(provider, event), [])
        result = providers.normalize_event("claude", {"type": "result", "subtype": "success", "is_error": False,
            "total_cost_usd": float("nan"), "usage": {"input_tokens": True, "output_tokens": -1}})
        self.assertEqual(result, [{"kind": "result", "ok": True, "text": ""}])


if __name__ == "__main__":
    unittest.main()
