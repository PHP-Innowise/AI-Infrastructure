"""The generator's prompt-read contract: the fragment hook-forge documents and
the hooks the editions ship both pass the verifier; a hook that still routes
the read through the removed prompt supervisor, rewrites the call, or points
CONTEXT_CLI anywhere but the runtime entry, fails. Syntax only - no hook is
executed here."""
import importlib.util
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
CALL = 'REPORT=$(run python3 "$CONTEXT_CLI" "${ARGUMENTS[@]}" 2>"$ERROR_FILE")'
SUPERVISED = ('REPORT=$(run python3 "$ROOT_DIR/memory-bank/scripts/workspace_roots.py" prompt-supervise '
              '--budget "$BUDGET_SECONDS" -- "${ARGUMENTS[@]}" 2>"$ERROR_FILE")')
# The runtime entry: the loader with its fallback, or context.py alone in a
# hook generated before the loader.
LOADER = ('CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context_cli.py"\n'
          '[ -f "$CONTEXT_CLI" ] || CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"\n')
DIRECT = 'CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/context.py"\n'


class GeneratorPromptReadTests(unittest.TestCase):
    def setUp(self):
        source = ROOT / "Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_generated.py"
        spec = importlib.util.spec_from_file_location("prompt_read_validator", source)
        self.v = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.v)

    def fragment(self, host):
        initial = "ARGUMENTS=(refresh --host " + host + (" --delivery-profile cursor-rule" if host == "cursor" else "") + ")"
        return "\n".join([
            "#!/bin/bash", "run() {", "if command -v timeout > /dev/null 2>&1; then",
            'timeout "$BUDGET_SECONDS" "$@"', "else", '"$@"', "fi", "}",
            'TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)}"',
            initial, 'if [ -n "$QUERY" ] && [ -n "$TASK_ID" ]; then',
            'ARGUMENTS+=(--query "$QUERY" --task-id "$TASK_ID" --ephemeral --sanitize)',
            '[ -n "${CONTEXT_TASK_ID:-}" ] || ARGUMENTS+=(--expected-branch "$TASK_ID")', "fi",
            *(["ARGUMENTS+=(--json)"] if host == "cursor" else []),
            CALL, "HOOK_STATUS=$?", '[ "$HOOK_STATUS" -eq 124 ] && REPORT=""']) + "\n"

    def accepted(self, text, host):
        return self.v.memory_hook_identity_contract(text, host, "working-memory-read.sh")

    @staticmethod
    def with_entry(text, entry):
        return text.replace("#!/bin/bash\n", "#!/bin/bash\n" + entry, 1)

    def test_documented_fragment_is_accepted(self):
        text = (ROOT / "Infrastructure-Creator/.agents/skills/hook-forge/SKILL.md").read_text(encoding="utf-8")
        blocks = re.findall(r"```bash\n(.*?)\n```", text, re.S)
        fragment = next(b for b in blocks if "ARGUMENTS=(refresh --host claude)" in b)
        relay = next(b for b in blocks if b.startswith("run() {"))
        entry = next(b for b in blocks if b.startswith("CONTEXT_CLI="))
        self.assertEqual(LOADER, entry + "\n")
        self.assertTrue(self.accepted(entry + "\n" + relay + "\n" + fragment, "claude"))

    def test_shipped_hooks_are_accepted(self):
        for edition in ("PHP Core", "Laravel", "Symfony", "Cms/wordpress"):
            for host, tool in (("claude", ".claude"), ("codex", ".codex"), ("cursor", ".cursor")):
                with self.subTest(edition=edition, host=host):
                    text = (ROOT / edition / tool / "hooks/working-memory-read.sh").read_text(encoding="utf-8")
                    self.assertTrue(self.accepted(text, host))
                    self.assertNotIn("prompt-supervise", text)
            # Cursor's turn and session readers: the loader in one, context.py
            # alone in the other - both runtime entry forms ship today.
            for name in ("working-memory-write.sh", "local-context.sh"):
                with self.subTest(edition=edition, hook=name):
                    text = (ROOT / edition / ".cursor/hooks" / name).read_text(encoding="utf-8")
                    self.assertTrue(self.v.memory_hook_identity_contract(text, "cursor", name))

    def test_canonical_fragments_are_accepted(self):
        for host in ("claude", "codex", "cursor"):
            with self.subTest(host=host):
                self.assertTrue(self.accepted(self.fragment(host), host))

    def test_generated_native_capture_adapters_bind_host_task_and_stdin(self):
        contract=getattr(self.v,'native_capture_hook_contract',lambda *args:False)
        for host,tree in [('claude','.claude'),('codex','.codex'),('cursor','.cursor')]:
            for name in ['working-memory-read.sh','working-memory-write.sh']:
                source=(ROOT/'PHP Core'/tree/'hooks'/name).read_text()
                self.assertTrue(contract(source,host,name),(host,name))
                command='native-prompt' if name=='working-memory-read.sh' else 'native-stop'
                self.assertFalse(contract(source.replace('--host '+host+' --task-id "$TASK_ID"',
                    '--host wrong --task-id "$TASK_ID"'),host,name))
                self.assertFalse(contract(source.replace(command,'obsolete-native-adapter'),host,name))
                self.assertFalse(contract(source.replace("printf '%s' \"$HOOK_STDIN\" |",'echo ignored |'),host,name))
                if 'NATIVE_PID=$!' in source:
                    # An arm beside the refresh is taken only with its PID at
                    # once and awaited: it must not outlive the hook.
                    self.assertFalse(contract(source.replace('  wait "$NATIVE_PID"\n','  :\n'),host,name))
                    self.assertFalse(contract(source.replace('  NATIVE_PID=$!\n','  :\n'),host,name))

    def test_documented_stop_journal_is_the_one_the_editions_ship(self):
        # hook-forge's fragment for the Stop hook's line in the native result
        # journal is, code line for code line, the block every edition's
        # working-memory-write.sh carries, and every shipped copy passes the
        # generator's journal contract for its own host.
        text = (ROOT / "Infrastructure-Creator/.agents/skills/hook-forge/SKILL.md").read_text(encoding="utf-8")
        fragments = [b for b in re.findall(r"```bash\n(.*?)\n```", text, re.S) if "native_journal() {" in b]
        self.assertEqual(1, len(fragments))
        contract = getattr(self.v, "native_journal_hook_contract", lambda *args: False)
        for edition in ("PHP Core", "Laravel", "Symfony", "Cms/wordpress"):
            for host, tool in (("claude", ".claude"), ("codex", ".codex"), ("cursor", ".cursor")):
                with self.subTest(edition=edition, host=host):
                    source = (ROOT / edition / tool / "hooks/working-memory-write.sh").read_text(encoding="utf-8")
                    self.assertTrue(contract(source, host))
                    documented = self.v.memory_hook_code_lines(fragments[0].replace("--host claude", "--host " + host))
                    shipped = self.v.memory_hook_code_lines(source)
                    start = shipped.index(documented[0])
                    self.assertEqual(documented, shipped[start:start + len(documented)])

    def test_stop_rebase_fallback_is_immediate_and_unique(self):
        source = (ROOT / 'PHP Core/.cursor/hooks/working-memory-write.sh').read_text(encoding='utf-8')
        selector = 'TASK_ID="${CONTEXT_TASK_ID:-$(git -C "$PROJECT_DIR" branch --show-current 2>/dev/null)}"'
        fallback = '[ -n "$TASK_ID" ] || TASK_ID=$(rebased_branch)'
        source = source.replace(fallback + '\n', '')
        admitted = source.replace(selector, selector + '\n' + fallback, 1)
        self.assertTrue(self.v.memory_hook_identity_contract(admitted, 'cursor', 'working-memory-write.sh'))
        for text in (admitted + '\n' + fallback + '\n',
                     source.replace(selector, selector + '\necho altered\n' + fallback, 1)):
            self.assertFalse(self.v.memory_hook_identity_contract(text, 'cursor', 'working-memory-write.sh'))
        local = (ROOT / 'PHP Core/.cursor/hooks/local-context.sh').read_text(encoding='utf-8')
        self.assertFalse(self.v.memory_hook_identity_contract(local + '\n' + fallback, 'cursor', 'local-context.sh'))

    def test_the_owner_thread_fallback_follows_the_rebase_one(self):
        # A worktree Codex made is detached until a branch is made there: the
        # thread that owns it names the task, after the branch and a rebase.
        rebase = '[ -n "$TASK_ID" ] || TASK_ID=$(rebased_branch)'
        thread = '[ -n "$TASK_ID" ] || TASK_ID=$(codex_thread_task)'
        # The contract reads every prompt hook and Cursor's turn hook, which
        # renders the rule again; the others' turn hooks deliver nothing.
        for name, host in (('working-memory-read.sh', 'claude'), ('working-memory-read.sh', 'codex'),
                           ('working-memory-read.sh', 'cursor'), ('working-memory-write.sh', 'cursor')):
            with self.subTest(hook=name, host=host):
                source = (ROOT / 'PHP Core' / ('.' + host) / 'hooks' / name).read_text(encoding='utf-8')
                self.assertIn(rebase + '\n' + thread + '\n', source)
                self.assertTrue(self.v.memory_hook_identity_contract(source, host, name))
                for text in (source.replace(rebase + '\n' + thread, thread + '\n' + rebase),
                             source.replace(rebase + '\n', ''),
                             source.replace(thread + '\n', thread + '\n' + thread + '\n', 1),
                             source.replace(thread, '[ -n "$TASK_ID" ] || TASK_ID=$(codex_thread)')):
                    self.assertFalse(self.v.memory_hook_identity_contract(text, host, name))
        local = (ROOT / 'PHP Core/.cursor/hooks/local-context.sh').read_text(encoding='utf-8')
        self.assertFalse(self.v.memory_hook_identity_contract(local + '\n' + rebase + '\n' + thread, 'cursor',
                                                              'local-context.sh'))

    def test_documented_rebase_resolver_is_the_one_the_editions_ship(self):
        # A bisect keeps the branch it started from, as a rebase keeps the one
        # it rewrites: a generated project's hooks read both, as the editions'.
        text = (ROOT / "Infrastructure-Creator/.agents/skills/hook-forge/SKILL.md").read_text(encoding="utf-8")
        documented = re.search(r"^rebased_branch\(\) \{\n.*?^\}\n", text, re.S | re.M).group(0)
        self.assertIn("BISECT_START", documented)
        for edition in ("PHP Core", "Laravel", "Symfony", "Cms/wordpress"):
            for tool in (".claude", ".codex", ".cursor"):
                for name in ("working-memory-read.sh", "working-memory-write.sh"):
                    with self.subTest(edition=edition, tool=tool, hook=name):
                        self.assertIn(documented, (ROOT / edition / tool / "hooks" / name).read_text(encoding="utf-8"))

    def test_documented_owner_thread_resolver_is_the_one_the_editions_ship(self):
        text = (ROOT / "Infrastructure-Creator/.agents/skills/hook-forge/SKILL.md").read_text(encoding="utf-8")
        documented = re.search(r"^codex_thread_task\(\) \{\n.*?^\}\n", text, re.S | re.M).group(0)
        for edition in ("PHP Core", "Laravel", "Symfony", "Cms/wordpress"):
            for tool in (".claude", ".codex", ".cursor"):
                for name in ("working-memory-read.sh", "working-memory-write.sh"):
                    with self.subTest(edition=edition, tool=tool, hook=name):
                        self.assertIn(documented, (ROOT / edition / tool / "hooks" / name).read_text(encoding="utf-8"))

    def test_both_runtime_entry_forms_are_accepted(self):
        for host in ("claude", "codex", "cursor"):
            for name, entry in (("loader", LOADER), ("direct", DIRECT),
                                ("commented loader", LOADER.replace("\n[", "\n# An older runtime.\n[", 1))):
                with self.subTest(host=host, entry=name):
                    self.assertTrue(self.accepted(self.with_entry(self.fragment(host), entry), host))

    def test_other_runtime_entries_are_rejected(self):
        loader, fallback = LOADER.splitlines()
        variants = {
            "loader_without_fallback": loader + "\n",
            "fallback_without_loader": DIRECT + fallback + "\n",
            "misspelt_loader": LOADER.replace("context_cli.py", "context-cli.py"),
            "fallback_elsewhere": LOADER.replace("scripts/context.py", "scripts/contexts.py"),
            "project_runtime": DIRECT.replace("$ROOT_DIR", "$PROJECT_DIR"),
            "recorder": 'CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/record_cli.py"\n',
            "late_reassignment": LOADER + 'CONTEXT_CLI="$ROOT_DIR/memory-bank/scripts/record_cli.py"\n',
            "exported": "export " + DIRECT,
            "default_expansion": ': "${CONTEXT_CLI:=$ROOT_DIR/memory-bank/scripts/context.py}"\n',
        }
        for name, entry in variants.items():
            with self.subTest(entry=name):
                self.assertFalse(self.accepted(self.with_entry(self.fragment("claude"), entry), "claude"))

    def test_rewritten_reads_are_rejected(self):
        base = self.fragment("claude")
        mutations = {
            "supervised_call": base.replace(CALL, SUPERVISED),
            "extra_supervisor": base + 'python3 "$ROOT_DIR/memory-bank/scripts/workspace_roots.py" prompt-supervise\n',
            "call_before_marker": base.replace("fi\n" + CALL, CALL + "\nfi\n"),
            "array_mutation": base + "ARGUMENTS+=(--read-only)\n",
            "foreign_expected_branch": base + 'python3 "$CONTEXT_CLI" refresh --expected-branch main\n',
        }
        for name, text in mutations.items():
            with self.subTest(mutation=name):
                self.assertNotEqual(base, text)
                self.assertFalse(self.accepted(text, "claude"))

    def test_cursor_json_belongs_after_the_conditional(self):
        text = self.fragment("cursor").replace("fi\nARGUMENTS+=(--json)\n", "ARGUMENTS+=(--json)\nfi\n")
        self.assertFalse(self.accepted(text, "cursor"))


if __name__ == "__main__":
    unittest.main()
