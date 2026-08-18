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

        checked = 0
        for line in command_lines:
            if not line.startswith("python3 "):
                continue
            # The contract also declares standalone validators
            # (memory-bank/scripts/validate.py), which carry their own CLI and
            # are covered by test_read_health_commands_are_actually_read_only.
            if "context.py " not in line:
                continue
            command = line.split("context.py ", 1)[1].split()[0]
            self.assertIn(command, parser_commands, line)
            checked += 1
        self.assertGreater(checked, 0, "no context.py command was checked")

    def test_read_health_commands_are_actually_read_only(self) -> None:
        """read_health is an attestation the quality gate trusts.

        validate_skill_quality accepts these commands as safe verification
        even though an interpreter invocation can never be proven
        non-mutating. That trust is only sound while every declared script
        really is read-only, so assert it against the shipped sources rather
        than against intent.
        """
        write_markers = re.compile(
            r"open\([^)]*[\"']([wax])[\"']|write_text|write_bytes|mkdir|unlink"
            r"|rmtree|os\.remove|\.commit\(|INSERT |UPDATE |DELETE "
        )
        read_health = self.contract["commands"]["read_health"]
        self.assertTrue(read_health, "read_health must not be empty")
        seen_scripts = set()
        for command in read_health:
            parts = command.split()
            self.assertEqual(parts[0], "python3", command)
            # Declared paths are target-relative (memory-bank/scripts/x.py);
            # the shipped sources live in the seed asset tree.
            declared = parts[1]
            self.assertTrue(
                declared.startswith("memory-bank/scripts/"),
                f"unexpected script location: {declared}",
            )
            script = ASSET.parent / "scripts" / Path(declared).name
            self.assertTrue(script.is_file(), f"{script} is declared but missing")
            seen_scripts.add(script)
        # context.py owns the mutating surface too, so only the standalone
        # validators can be asserted wholesale; context.py is covered by
        # test_declared_commands_are_real_parser_commands.
        for script in seen_scripts:
            if script.name == "context.py":
                continue
            source = script.read_text(encoding="utf-8")
            hits = sorted(set(write_markers.findall(source)))
            self.assertEqual(
                [], [h for h in hits if h],
                f"{script.name} is declared read-only but writes: {hits}",
            )

    def test_mutating_command_groups_are_not_in_read_health(self) -> None:
        """A mutating command must never migrate into the attested set."""
        commands = self.contract["commands"]
        read_health = set(commands["read_health"])
        mutating = []
        for group in ("refresh_retrieve", "governed_task", "dynamic_records"):
            value = commands.get(group)
            if isinstance(value, list):
                mutating.extend(value)
        for entry in mutating:
            self.assertNotIn(entry, read_health, f"{entry} mutates state")

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
