import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ASSET = ROOT / ".agents/skills/memory-seed/assets/runtime-contract.json"
CONTEXT = ROOT / ".agents/skills/memory-seed/assets/scripts/context.py"
REGISTRY = ROOT / ".agents/skills/skill-forge/references/candidate-registry.json"


class RuntimeContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.contract = json.loads(ASSET.read_text(encoding="utf-8"))
        cls.context_source = CONTEXT.read_text(encoding="utf-8")

    def test_sqlite_path_and_checkpoint_tables_match_runtime(self) -> None:
        sqlite = self.contract["sqlite"]
        self.assertEqual("memory-bank/local/context.db", sqlite["path"])

        for table, specification in sqlite["checkpoint_tables"].items():
            match = re.search(
                rf"CREATE TABLE IF NOT EXISTS {re.escape(table)}\((.*?)\n\s*\)",
                self.context_source,
                flags=re.DOTALL,
            )
            self.assertIsNotNone(match, table)
            actual_columns = {
                line.strip().split()[0].rstrip(",")
                for line in match.group(1).splitlines()
                if line.strip()
            }
            self.assertEqual(set(specification["columns"]), actual_columns)

    def test_declared_commands_are_real_parser_commands(self) -> None:
        parser_commands = set(
            re.findall(r'commands\.add_parser\(\s*"([^"]+)"', self.context_source)
        )
        command_lines = []
        commands = self.contract["commands"]
        for value in commands.values():
            if isinstance(value, list):
                command_lines.extend(value)
            elif isinstance(value, dict):
                command_lines.extend(
                    item for item in value.values() if isinstance(item, str)
                )

        for line in command_lines:
            if not line.startswith("python3 "):
                continue
            command = line.split("context.py ", 1)[1].split()[0]
            self.assertIn(command, parser_commands, line)

    def test_runtime_paths_and_forbidden_legacy_paths_are_disjoint(self) -> None:
        paths = self.contract["path_contracts"]
        supported = set(paths["required_skeleton"])
        supported.update(item["path"] for item in paths["creatable"])
        self.assertTrue(supported.isdisjoint(paths["forbidden_invented_paths"]))
        self.assertIn("project-brain/dynamic/tasks/", supported)
        self.assertIn("project-brain/control/messages/", supported)

    def test_registry_points_to_runtime_contract_and_requires_invariant_mapping(self) -> None:
        registry = json.loads(REGISTRY.read_text(encoding="utf-8"))
        referenced = (REGISTRY.parent / registry["runtime_contract"]).resolve()
        self.assertEqual(ASSET.resolve(), referenced)

        requirements = registry["compilation_requirements"]
        self.assertTrue(requirements["bounded_evidence_anchor_required"])
        self.assertTrue(requirements["material_adjacency_required"])
        self.assertEqual(
            1,
            requirements["high_priority_invariant_mapping"][
                "verification_assertions_minimum"
            ],
        )


if __name__ == "__main__":
    unittest.main()
