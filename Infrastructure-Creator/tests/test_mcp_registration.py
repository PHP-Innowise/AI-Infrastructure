"""Generated client registration is owned, portable and protected by the gate."""
import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('mcp_generator_gate', ROOT / '.agents/skills/bootstrap-verifier/scripts/validate_generated.py')
gate = importlib.util.module_from_spec(spec); spec.loader.exec_module(gate)
source = ROOT / '.agents/skills/memory-seed/assets/scripts/mcp_config.py'
spec = importlib.util.spec_from_file_location('mcp_generator_config', source)
mcp = importlib.util.module_from_spec(spec); spec.loader.exec_module(mcp)


class GeneratedMcpTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(); self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        helper = self.root / 'memory-bank/scripts/mcp_config.py'; helper.parent.mkdir(parents=True)
        shutil.copyfile(source, helper)
        self.files = {'memory-bank/scripts/mcp_config.py': {}}
        for host, relative in mcp.PATHS.items():
            destination = self.root / relative; destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(mcp.template(host)); self.files[relative] = {}

    def test_selected_registrations_pass_without_native_client_calls(self):
        errors = []; gate.validate_memory_mcp(self.root, list(mcp.PATHS), self.files, errors)
        self.assertEqual([], errors)

    def test_unowned_registration_is_rejected(self):
        del self.files['.mcp.json']; errors = []
        gate.validate_memory_mcp(self.root, ['claude'], self.files, errors)
        self.assertTrue(any('not manifest-owned' in error for error in errors))

    def test_changed_transport_or_cwd_is_rejected(self):
        (self.root / '.cursor/mcp.json').write_text('{"mcpServers":{"harness-memory":{"type":"http","url":"https://foreign.invalid"}}}')
        with (self.root / '.codex/config.toml').open('a') as handle: handle.write('cwd="/foreign"\n')
        errors = []; gate.validate_memory_mcp(self.root, ['codex','cursor'], self.files, errors)
        self.assertTrue(errors)


if __name__ == '__main__': unittest.main()
