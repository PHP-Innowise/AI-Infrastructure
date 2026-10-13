"""Automatic native-client registration and portable real MCP launches."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from tests.test_harness_knowledge import install_knowledge_fixture

REPO = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('memory_mcp_registration', REPO / 'PHP Core/memory-bank/scripts/mcp_config.py')
mcp = importlib.util.module_from_spec(spec); spec.loader.exec_module(mcp)


class McpRegistrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(); self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / 'project with spaces'; self.root.mkdir()
        install_knowledge_fixture(self.root)

    def write_configs(self):
        for host, relative in mcp.PATHS.items():
            destination = self.root / relative; destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(mcp.template(host))

    def launch(self, host, root, cwd, environment=None):
        args = mcp.entry(host)['args']
        args = [arg.replace('${workspaceFolder}', str(root)) for arg in args]
        messages = [dict(jsonrpc='2.0', id=0, method='initialize', params={'protocolVersion': '2025-06-18'}),
            dict(jsonrpc='2.0', method='notifications/initialized'), dict(jsonrpc='2.0', id=1, method='tools/list')]
        env = dict(os.environ); env.pop('CLAUDE_PROJECT_DIR', None)
        env.update(environment or {})
        return subprocess.run([sys.executable, *args], cwd=cwd, env=env,
            input=''.join(json.dumps(message) + '\n' for message in messages),
            capture_output=True, text=True, timeout=15)

    def test_each_client_starts_real_memory_from_nested_directory_without_git(self):
        self.write_configs(); nested = self.root / 'src/deep'; nested.mkdir(parents=True)
        for host in mcp.PATHS:
            with self.subTest(host=host):
                result = self.launch(host, self.root, nested)
                self.assertEqual(0, result.returncode, result.stderr)
                self.assertEqual('', result.stderr)
                replies = [json.loads(line) for line in result.stdout.splitlines()]
                self.assertEqual(4, len(replies[-1]['result']['tools']))

    def test_claude_uses_declared_project_root_and_bootstrap_stops_at_nested_marker(self):
        self.write_configs(); outside = self.root.parent / 'other'; outside.mkdir()
        result = self.launch('claude', self.root, outside, {'CLAUDE_PROJECT_DIR': str(self.root)})
        self.assertEqual(0, result.returncode, result.stderr)
        nested = self.root / 'nested'; (nested / '.codex').mkdir(parents=True)
        (nested / '.codex/config.toml').write_text('# Another project, without memory\n')
        result = self.launch('codex', self.root, nested)
        self.assertNotEqual(0, result.returncode)
        self.assertEqual('', result.stdout)
        self.assertIn('Install the memory runtime in this project', result.stderr)

    def test_same_configuration_runs_in_a_different_worktree(self):
        self.write_configs()
        alternate = self.root.parent / 'another worktree'; alternate.mkdir()
        install_knowledge_fixture(alternate)
        for host, relative in mcp.PATHS.items():
            destination = alternate / relative; destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((self.root / relative).read_bytes())
            result = self.launch(host, alternate, alternate)
            self.assertEqual(0, result.returncode, result.stderr)
            self.assertNotIn(str(self.root), destination.read_text())

    def test_json_merge_preserves_other_servers_settings_and_is_idempotent(self):
        for host in ('claude', 'cursor'):
            previous = json.dumps({'mcpServers': {'other': {'command': 'other-server', 'args': ['keep']}}, 'custom': [1]}).encode()
            merged = mcp.merge(mcp.PATHS[host], previous, mcp.template(host))
            data = json.loads(merged)
            self.assertEqual({'command': 'other-server', 'args': ['keep']}, data['mcpServers']['other'])
            self.assertEqual([1], data['custom'])
            self.assertEqual(merged, mcp.merge(mcp.PATHS[host], merged, mcp.template(host)))
            data['mcpServers']['harness-memory']['env'] = {'PROJECT_BRAIN_OWNER': 'alice'}
            with self.assertRaises(mcp.ConfigError):
                mcp.merge(mcp.PATHS[host], json.dumps(data).encode(), mcp.template(host))

    def test_foreign_server_malformed_and_duplicate_json_are_refused(self):
        for previous in (b'{oops}', b'[]', b'{"mcpServers":[]}',
            b'{"mcpServers":{},"mcpServers":{}}', b'{"custom":NaN}',
            b'{"mcpServers":{"harness-memory":null}}', b'{"mcpServers":{"harness-memory":{"command":"foreign"}}}'):
            with self.subTest(previous=previous), self.assertRaises(mcp.ConfigError):
                mcp.merge('.mcp.json', previous, mcp.template('claude'))

    def test_toml_merge_preserves_unrelated_text_and_is_idempotent(self):
        previous = b'# User settings\nmodel = "existing"\n\n[features]\nhooks = false\n\n[mcp_servers.other]\ncommand = "keep"\nargs = ["one", "two"]\n'
        merged = mcp.merge('.codex/config.toml', previous, mcp.template('codex'))
        self.assertTrue(merged.startswith(previous))
        self.assertEqual(merged, mcp.merge('.codex/config.toml', merged, mcp.template('codex')))
        with_tail = merged + b'\n\n[extra]\nvalue = "preserve"\n'
        changed = mcp.merge('.codex/config.toml', with_tail, mcp.template('codex'), ('py', ['-3']))
        self.assertTrue(changed.endswith(b'\n\n[extra]\nvalue = "preserve"\n'))

    def test_an_older_bootstrap_version_is_ours_and_is_replaced(self):
        older = mcp.BOOTSTRAP.replace('bootstrap v1', 'bootstrap v0', 1) + '\n# superseded launcher'
        raw = json.dumps({'mcpServers': {'harness-memory': {'type': 'stdio', 'command': 'python',
            'args': ['-c', older, 'claude']}}}).encode()
        merged = json.loads(mcp.merge('.mcp.json', raw, mcp.template('claude')))
        self.assertEqual(mcp.entry('claude'), merged['mcpServers']['harness-memory'])
        block = mcp.codex_block().replace(json.dumps(mcp.BOOTSTRAP), json.dumps(older))
        self.assertNotEqual(block, mcp.codex_block())
        self.assertEqual(mcp.codex_block().encode(), mcp.merge('.codex/config.toml', block.encode(), mcp.template('codex')))
        foreign = json.dumps({'mcpServers': {'harness-memory': {'type': 'stdio', 'command': 'python3',
            'args': ['-c', 'print(1)', 'claude']}}}).encode()
        with self.assertRaises(mcp.ConfigError): mcp.merge('.mcp.json', foreign, mcp.template('claude'))

    def test_toml_foreign_quoted_dotted_inline_and_malformed_blocks_are_refused(self):
        cases = (b'[mcp_servers.harness_memory]\ncommand="foreign"\n',
            b'["mcp_servers"."harness_memory"]\ncommand="foreign"\n',
            b'mcp_servers.harness_memory.command="foreign"\n',
            b'[mcp_servers]\nharness_memory={command="foreign"}\n',
            b'mcp_servers={other={command="keep"}}\n', (mcp.BEGIN+'\n').encode(),
            mcp.codex_block().replace('enabled = true','enabled = false').encode())
        for previous in cases:
            with self.subTest(previous=previous), self.assertRaises(mcp.ConfigError):
                mcp.merge('.codex/config.toml', previous, mcp.template('codex'))

    def test_hidden_transport_disable_and_actor_overrides_are_refused(self):
        for extra in ({'enabled': False}, {'url': 'https://foreign.invalid/mcp'},
            {'env': {'PROJECT_BRAIN_OWNER': 'admin'}}, {'type': 'http'}):
            raw = json.dumps({'mcpServers': {'harness-memory': {**mcp.entry('claude'), **extra}}}).encode()
            with self.assertRaises(mcp.ConfigError): mcp.merge('.mcp.json', raw, mcp.template('claude'))
        for extra in ('cwd = "/foreign"\n', 'env = { PROJECT_BRAIN_OWNER = "admin" }\n', 'enabled = false\n'):
            with self.assertRaises(mcp.ConfigError):
                mcp.merge('.codex/config.toml', (mcp.codex_block()+extra).encode(), mcp.template('codex'))

    def test_claude_resolves_project_alias_without_arg_whitespace_warning(self):
        self.write_configs(); alias = self.root.parent / 'alias'; alias.symlink_to(self.root, target_is_directory=True)
        result = self.launch('claude', self.root, self.root.parent, {'CLAUDE_PROJECT_DIR': str(alias)})
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(mcp.BOOTSTRAP.strip(), mcp.BOOTSTRAP)

    def test_failed_second_registration_restores_content_modes_and_directories(self):
        first = self.root / '.mcp.json'; original = b'{"custom":"keep"}\n'; first.write_bytes(original); first.chmod(0o640)
        plans = [(first, original, mcp.template('claude')),
            (self.root / '.cursor/mcp.json', None, mcp.template('cursor')),
            (self.root / '.codex/config.toml', None, mcp.template('codex'))]
        replace = mcp.os.replace; count = [0]
        def fail_second(*args):
            count[0] += 1
            if count[0] == 2: raise OSError('Injected replacement failure')
            return replace(*args)
        with patch.object(mcp.os, 'replace', fail_second), self.assertRaises(mcp.ConfigError):
            mcp.apply(self.root, plans)
        self.assertEqual(original, first.read_bytes())
        self.assertEqual(0o640, first.stat().st_mode & 0o777)
        self.assertFalse((self.root / '.cursor').exists())
        self.assertFalse((self.root / '.codex').exists())
        self.assertEqual([], list(self.root.glob('**/.memory-mcp-*')))

    def test_python39_toml_fallback_ignores_header_text_inside_multiline_strings(self):
        previous = b'description = """\n[mcp_servers.harness_memory]\nnot a table\n"""\n[mcp_servers.other]\nargs = [\n "keep",\n]\n'
        with patch.dict(sys.modules, {'tomllib': None}):
            merged = mcp.merge('.codex/config.toml', previous, mcp.template('codex'))
            self.assertTrue(merged.startswith(previous))
            with self.assertRaises(mcp.ConfigError):
                mcp.merge('.codex/config.toml', b'["mcp_servers"."harness_memory"]\ncommand="foreign"\n', mcp.template('codex'))

    def test_toml_arrays_of_tables_keep_separate_key_scopes(self):
        previous = b'[[custom.rules]]\nname="first"\n[custom.rules.options]\nvalue=1\n[[custom.rules]]\nname="second"\n[custom.rules.options]\nvalue=2\n'
        for fallback in (False, True):
            with self.subTest(fallback=fallback), patch.dict(sys.modules, {'tomllib': None} if fallback else {}):
                merged = mcp.merge('.codex/config.toml', previous, mcp.template('codex'))
                self.assertTrue(merged.startswith(previous))
                self.assertEqual(merged, mcp.merge('.codex/config.toml', merged, mcp.template('codex')))
                with self.assertRaises(mcp.ConfigError):
                    mcp.merge('.codex/config.toml', b'[[custom.rules]]\nname="one"\nname="two"\n', mcp.template('codex'))

    def test_python39_fallback_rejects_malformed_values_before_any_merge(self):
        invalid = (b'x =\n', b'x = { key = }\n', b'x = 1 trailing\n',
            b'x = [true false]\n', b'x = {a=1,}\n', b'x = {a=1,a=2}\n',
            b'x = 01\n', b'x = 1.\n', b'x = "\\q"\n', b'x = 2026-02-30\n', b'x = 25:00:00\n',
            b'[other]\nx=1\n[other]\ny=2\n', b'[other]]\nx=1\n',
            b'x=1\n[x]\ny=2\n', b'x.y=1\n[x]\nz=2\n', b'x=1\nx.y=2\n',
            b'"bad\\/key"=1\n')
        with patch.dict(sys.modules, {'tomllib': None}):
            for previous in invalid:
                with self.subTest(previous=previous), self.assertRaises(mcp.ConfigError):
                    mcp.merge('.codex/config.toml', previous, mcp.template('codex'))

    def test_python39_fallback_preserves_supported_native_values(self):
        previous = b'''literal = 'keep'
multiline = ''' + b"'''\nkeep\n'''\n" + b'''escaped = "\\u0041\\U00000042"
numbers = [+1, -2, 1_000, 0xff, 0o70, 0b10, 1.5e-2, inf, -nan]
dates = [2026-10-08, 2026-10-08 12:00:00Z, 12:00:00.5]
options = {command="keep", env={NOTE="opaque"}, args=["one", "two"]}
'''
        with patch.dict(sys.modules, {'tomllib': None}):
            merged = mcp.merge('.codex/config.toml', previous, mcp.template('codex'))
            self.assertTrue(merged.startswith(previous))
            self.assertEqual(merged, mcp.merge('.codex/config.toml', merged, mcp.template('codex')))

    def test_native_windows_python_selection_falls_back_to_py3(self):
        with patch.object(mcp.os, 'name', 'nt'), patch.object(mcp.subprocess, 'run', side_effect=[OSError(), type('Result', (), {'returncode': 0})()]) as run:
            self.assertEqual(('py', ['-3']), mcp.python_command())
            self.assertEqual(['py', '-3', '-c'], run.call_args.args[0][:3])

    def test_registration_cli_dry_run_and_conflict_preflight_write_nothing(self):
        helper = self.root / 'memory-bank/scripts/mcp_config.py'
        preview = subprocess.run([sys.executable, str(helper), '--dry-run'], capture_output=True, text=True, timeout=15)
        self.assertEqual(0, preview.returncode, preview.stderr)
        self.assertFalse((self.root / '.mcp.json').exists())
        (self.root / '.cursor').mkdir()
        config = self.root / '.cursor/mcp.json'; config.write_text('{"mcpServers":{"harness-memory":{"command":"foreign"}}}')
        original = config.read_bytes()
        refused = subprocess.run([sys.executable, str(helper)], capture_output=True, text=True, timeout=15)
        self.assertEqual(1, refused.returncode)
        self.assertEqual(original, config.read_bytes())
        self.assertFalse((self.root / '.mcp.json').exists())


class McpInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(); self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / 'installed worktree'; self.root.mkdir()

    def install(self, *flags):
        return subprocess.run([sys.executable, str(REPO / 'scripts/install_accelerator.py'),
            '--edition', 'PHP Core', '--target', str(self.root), *flags], capture_output=True, text=True, timeout=30)

    def test_install_merges_native_configs_and_reinstall_is_idempotent(self):
        for host in ('claude', 'cursor'):
            config = self.root / mcp.PATHS[host]; config.parent.mkdir(parents=True, exist_ok=True)
            config.write_text(json.dumps({'mcpServers': {'other': {'command': 'keep', 'env': {'NOTE': 'opaque-config-value'}}}}))
        config = self.root / mcp.PATHS['codex']
        before = {p.relative_to(self.root).as_posix(): p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        preview = self.install('--dry-run', '--merge-existing')
        self.assertEqual(0, preview.returncode, preview.stderr)
        self.assertEqual(before, {p.relative_to(self.root).as_posix(): p.read_bytes() for p in self.root.rglob('*') if p.is_file()})
        first = self.install('--merge-existing')
        self.assertEqual(0, first.returncode, first.stderr)
        for host in ('claude', 'cursor'):
            servers = json.loads((self.root / mcp.PATHS[host]).read_bytes())['mcpServers']
            self.assertEqual('opaque-config-value', servers['other']['env']['NOTE'])
            self.assertTrue(mcp._owned(servers['harness-memory'], host))
        self.assertIn('hooks = true', config.read_text())
        registered = {name: (self.root / name).read_bytes() for name in mcp.PATHS.values()}
        repeated = self.install('--merge-existing')
        self.assertEqual(0, repeated.returncode, repeated.stderr)
        self.assertEqual(registered, {name: (self.root / name).read_bytes() for name in mcp.PATHS.values()})

    def test_client_codex_config_is_a_collision_not_a_block_append(self):
        config = self.root / mcp.PATHS['codex']; config.parent.mkdir(parents=True)
        config.write_text('model = "existing"\n')
        merged = self.install('--tool', 'codex', '--merge-existing')
        self.assertEqual(2, merged.returncode, merged.stderr)
        self.assertIn('.codex/config.toml\texisting-file', merged.stderr)
        self.assertEqual('model = "existing"\n', config.read_text())
        overwritten = self.install('--tool', 'codex', '--overwrite')
        self.assertEqual(0, overwritten.returncode, overwritten.stderr)
        self.assertIn('hooks = true', config.read_text())
        self.assertIn(mcp.BEGIN, config.read_text())

    def test_foreign_memory_name_blocks_all_files_even_under_overwrite(self):
        config = self.root / '.mcp.json'; original = b'{"mcpServers":{"harness-memory":{"command":"foreign"}}}\n'
        config.write_bytes(original)
        for flag in ('--merge-existing', '--overwrite'):
            result = self.install(flag)
            self.assertEqual(2, result.returncode, result.stderr)
            self.assertEqual(original, config.read_bytes())
            self.assertEqual(['.mcp.json'], sorted(p.relative_to(self.root).as_posix() for p in self.root.rglob('*') if p.is_file()))

    def test_selecting_codex_does_not_register_unselected_clients(self):
        result = self.install('--tool', 'codex')
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertTrue((self.root / '.codex/config.toml').is_file())
        self.assertFalse((self.root / '.mcp.json').exists())
        self.assertFalse((self.root / '.cursor/mcp.json').exists())


if __name__ == '__main__': unittest.main()
