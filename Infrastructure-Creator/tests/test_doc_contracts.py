#!/usr/bin/env python3
"""Doc-to-validator coherence tests for canonical LLM-prompt documents.

These pin the executable instructions in the canonical docs against the
actual validation gates so a doc edit cannot quietly contradict a validator:

- infra-generate step 2 must demand per-skill `routing_cases[]`
  (`skills[].routing_cases`), never a top-level member that
  `validate_skill_quality.py` blocks with PLAN_FIELD_UNKNOWN.
- The project-profile-schema exemplars must satisfy the exact plan/skill
  membership and the fixed flow-phase vocabulary the validators enforce.
- The schema doc must use only the neutral acme-billing fixture family,
  never a leaked real-project domain.
- command-forge must not instruct running `validate_flow_contracts.py`
  before `skill-flow-composer` exists; that gate is orchestrator-owned and
  documented (with the runnable recipe) in infra-generate.
- The command-forge frontmatter guardrail must carry the flow-command
  exception that the flow-contract validator mechanically enforces.
- AGENTS.md must state one consistent schema policy: only 1.2 is
  approvable; 1.0/1.1 are audit-readable and must be re-synthesized.
"""

from __future__ import annotations

import json
import re
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(SCRIPTS))

from validate_flow_contracts import PHASES  # noqa: E402
import validate_skill_quality  # noqa: E402
from validate_skill_quality import (  # noqa: E402
    OWNERSHIP_ID_PATTERN,
    SCHEMA_1_2_PLAN_FIELDS,
    SCHEMA_1_2_SKILL_FIELDS,
)

SCHEMA_DOC = (
    ROOT
    / ".agents/skills/profile-synthesizer/references/project-profile-schema.md"
)
INFRA_GENERATE_DOC = ROOT / ".agents/skills/infra-generate/SKILL.md"
PROCESS_CATALOG_DOC = (
    ROOT / ".agents/skills/skill-forge/references/php-process-skills.md"
)
SKILL_FORGE_DOC = ROOT / ".agents/skills/skill-forge/SKILL.md"
RUNTIME_CONTRACT_ASSET = (
    ROOT / ".agents/skills/memory-seed/assets/runtime-contract.json"
)
CATALOG_ROW = re.compile(r"^\| `[^`]+` \|.*\| ([a-z-]+) \|$", re.M)
COMMAND_FORGE_DOC = ROOT / ".agents/skills/command-forge/SKILL.md"
AGENTS_DOC = ROOT / "AGENTS.md"
BOOTSTRAP_VERIFIER_DOC = ROOT / ".agents/skills/bootstrap-verifier/SKILL.md"

JSON_BLOCK = re.compile(r"```json\n(.*?)\n```", re.DOTALL)
BASH_BLOCK = re.compile(r"```bash\n(.*?)\n```", re.DOTALL)


def normalized(path: Path) -> str:
    """Collapse whitespace so assertions survive line re-wrapping."""
    return re.sub(r"\s+", " ", path.read_text(encoding="utf-8"))


def json_examples(path: Path) -> list:
    """Every ```json exemplar in the doc must parse as standalone JSON."""
    return [
        json.loads(payload)
        for payload in JSON_BLOCK.findall(path.read_text(encoding="utf-8"))
    ]


class SchemaDocExemplarTest(unittest.TestCase):
    def setUp(self) -> None:
        self.blocks = json_examples(SCHEMA_DOC)
        self.text = SCHEMA_DOC.read_text(encoding="utf-8")

    def _only(self, predicate) -> dict:
        matches = [
            block
            for block in self.blocks
            if isinstance(block, dict) and predicate(block)
        ]
        self.assertEqual(len(matches), 1)
        return matches[0]

    def test_plan_top_level_example_matches_validator_membership(self) -> None:
        plan = self._only(lambda block: "schema_version" in block)
        self.assertEqual(set(plan), set(SCHEMA_1_2_PLAN_FIELDS))

    def test_skill_exemplar_carries_schema_1_2_fields(self) -> None:
        skill = self._only(lambda block: block.get("name") == "testing")
        self.assertLessEqual(set(SCHEMA_1_2_SKILL_FIELDS), set(skill))

    def test_skill_exemplar_phase_is_in_fixed_vocabulary(self) -> None:
        skill = self._only(lambda block: block.get("name") == "testing")
        self.assertIn(skill["phase"], PHASES)

    def test_doc_lists_exactly_the_validator_phase_vocabulary(self) -> None:
        for phase in PHASES:
            self.assertIn(f"`{phase}`", self.text)
        self.assertNotIn("`execution`", self.text)

    def test_invariant_example_shape_matches_validator(self) -> None:
        block = self._only(lambda item: item.get("critical_invariants"))
        for invariant in block["critical_invariants"]:
            self.assertEqual(
                set(invariant),
                {"id", "statement", "evidence_ids", "skill_names", "assertions"},
            )
            self.assertRegex(invariant["id"], OWNERSHIP_ID_PATTERN)
            for assertion in invariant["assertions"]:
                self.assertEqual(set(assertion), {"skill_name", "verification_id"})
                self.assertIn(assertion["skill_name"], invariant["skill_names"])

    def test_flow_contract_example_uses_legal_phases(self) -> None:
        block = self._only(
            lambda item: item.get("flow_contracts", {}).get("flows")
        )
        graph = block["flow_contracts"]
        for entry in graph["roster"]:
            self.assertIn(entry["phase"], PHASES)
        for flow in graph["flows"]:
            for stage in flow["stages"]:
                self.assertIn(stage["phase"], PHASES)

    def test_examples_use_only_the_neutral_fixture_family(self) -> None:
        lowered = self.text.lower()
        self.assertNotIn("contentjob", lowered)
        self.assertNotIn("content-job", lowered)
        self.assertIn("billing-rules-review", self.text)


class InfraGenerateDocTest(unittest.TestCase):
    def test_step_2_demands_per_skill_routing_cases(self) -> None:
        text = normalized(INFRA_GENERATE_DOC)
        self.assertNotIn("top-level `routing_cases[]`", text)
        self.assertIn("`routing_cases[]` (`skills[].routing_cases`)", text)
        self.assertIn("canonical top-level `flow_contracts`", text)

    def test_orchestrator_keeps_the_runnable_flow_validator_recipe(self) -> None:
        bash_blocks = BASH_BLOCK.findall(
            INFRA_GENERATE_DOC.read_text(encoding="utf-8")
        )
        self.assertTrue(
            any("validate_flow_contracts.py" in block for block in bash_blocks)
        )


class CommandForgeDocTest(unittest.TestCase):
    def test_flow_validation_is_deferred_to_the_orchestrator(self) -> None:
        text = COMMAND_FORGE_DOC.read_text(encoding="utf-8")
        bash_blocks = BASH_BLOCK.findall(text)
        self.assertFalse(
            any("validate_flow_contracts.py" in block for block in bash_blocks)
        )
        flat = normalized(COMMAND_FORGE_DOC)
        self.assertIn("do NOT run it here", flat)
        self.assertIn("`infra-generate` step 9", flat)

    def test_frontmatter_guardrail_carries_the_flow_command_exception(
        self,
    ) -> None:
        flat = normalized(COMMAND_FORGE_DOC)
        self.assertIn("Flow commands are the sole exception", flat)
        self.assertIn(
            "`flow` + ordered `stages` frontmatter and one fenced "
            "`json flow-contract` block",
            flat,
        )


class BootstrapVerifierDedupDocTest(unittest.TestCase):
    """The documented dedup thresholds must be the ones the gate applies.

    `bootstrap-verifier/SKILL.md` publishes the skeleton pass's calibration as
    concrete numbers so a reader can judge the false-positive margin. Numbers
    in prose drift silently, so they are pinned to the constants here.
    """

    def setUp(self) -> None:
        self.flat = normalized(BOOTSTRAP_VERIFIER_DOC)

    def test_documented_thresholds_match_the_validator(self) -> None:
        for constant, rendered in (
            (validate_skill_quality.SKELETON_LINE_FAIL, "0.38"),
            (validate_skill_quality.SKELETON_TOKEN_FAIL, "0.26"),
            (validate_skill_quality.SKELETON_LINE_WARN, "0.28"),
            (validate_skill_quality.SKELETON_TOKEN_WARN, "0.20"),
        ):
            self.assertEqual(f"{constant:.2f}", rendered)
            self.assertIn(rendered, self.flat)
        self.assertIn("blocks at 0.38 / 0.26", self.flat)
        self.assertIn("a warning sits at 0.28 / 0.20", self.flat)

    def test_documented_codes_and_severities_match_the_validator(self) -> None:
        for code in (
            "SKILL_TEMPLATE_REUSE",
            "SKILL_TEMPLATE_BLOCK",
            "SKILL_SIMILARITY",
            "REPEATED_BLOCK",
        ):
            self.assertIn(f"`{code}`", self.flat)
        # The doc claims the block pass is a warning; the validator must agree.
        diagnostics: list = []
        repeated = [
            "trace the xid payload through the transport and record every "
            "redelivery the handler accepts",
            "reject the xid allocation once the invoice reaches its terminal "
            "state and report the refusal",
        ]
        validate_skill_quality._report_skeleton_blocks(
            {"left": repeated, "right": repeated}, diagnostics
        )
        self.assertEqual(
            [(item.code, item.severity) for item in diagnostics],
            [("SKILL_TEMPLATE_BLOCK", "warning")],
        )
        self.assertIn("deliberately a *warning*", self.flat.lower())

    def test_documented_block_size_matches_the_validator(self) -> None:
        self.assertEqual(validate_skill_quality.SKELETON_BLOCK_SIZE, 2)
        self.assertIn(
            "two adjacent shared skeleton lines, down from three "
            "byte-identical ones",
            self.flat,
        )


class AgentsPolicyDocTest(unittest.TestCase):
    def test_schema_policy_bullets_are_consistent(self) -> None:
        flat = normalized(AGENTS_DOC)
        self.assertIn("only a schema 1.4 plan is approvable", flat)
        self.assertIn(
            "Legacy 1.0/1.1/1.2/1.3 plans remain readable for audit but MUST be "
            "re-synthesized",
            flat,
        )
        self.assertNotIn("schema 1.0 is migration-only", flat)
        self.assertNotIn("Schema 1.1 uses", flat)
        self.assertNotIn("only a schema 1.2 plan is approvable", flat)
        self.assertNotIn("only a schema 1.3 plan is approvable", flat)


class ProcessCatalogPhaseVocabularyTest(unittest.TestCase):
    """C2: the catalog's Phase column feeds `phase` verbatim, so it may only
    use the vocabulary `validate_flow_contracts.py` accepts."""

    def setUp(self) -> None:
        self.text = PROCESS_CATALOG_DOC.read_text(encoding="utf-8")
        self.phases = CATALOG_ROW.findall(self.text)

    def test_catalog_lists_every_candidate_with_a_phase(self) -> None:
        self.assertGreaterEqual(len(self.phases), 18)

    def test_every_catalog_phase_is_in_the_validator_vocabulary(self) -> None:
        for phase in self.phases:
            self.assertIn(phase, PHASES)

    def test_the_retired_off_vocabulary_phases_are_gone(self) -> None:
        for retired in ("| utility |", "| execution |"):
            self.assertNotIn(retired, self.text)


class RuntimeFixedDecisionDocTest(unittest.TestCase):
    """C3: the quartet's status must be documented where a reader would
    otherwise mistake it for a selection failure, and must name the gate codes
    that actually enforce the substituted bar."""

    CODES = (
        "RUNTIME_PATH_UNSUPPORTED",
        "RUNTIME_PATH_FORBIDDEN",
        "RUNTIME_COMMAND_UNSUPPORTED",
        "RUNTIME_PROJECT_CLAIM_UNSUPPORTED",
    )

    def test_catalog_states_the_status_and_the_substituted_bar(self) -> None:
        text = PROCESS_CATALOG_DOC.read_text(encoding="utf-8")
        self.assertIn("not a selection failure", text)
        for code in self.CODES:
            self.assertIn(code, text)

    def test_schema_states_the_same_decision(self) -> None:
        text = SCHEMA_DOC.read_text(encoding="utf-8")
        self.assertIn("This is a decision, not a gap in selection.", text)
        for code in self.CODES:
            self.assertIn(code, text)

    def test_documented_gate_codes_exist_in_the_validator(self) -> None:
        source = (
            ROOT
            / ".agents/skills/bootstrap-verifier/scripts/validate_skill_quality.py"
        ).read_text(encoding="utf-8")
        for code in self.CODES:
            self.assertIn(f'"{code}"', source)

    def test_catalog_quotes_only_runtime_contract_command_forms(self) -> None:
        """Catalog prose may explain the contract but never add a CLI form."""
        contract = json.loads(RUNTIME_CONTRACT_ASSET.read_text(encoding="utf-8"))
        signatures = [
            validate_skill_quality._runtime_command_signature(form)
            for form in validate_skill_quality._runtime_command_forms(
                contract["commands"]
            )
        ]
        allowed = {
            (item[0], item[1]) for item in signatures if item is not None
        }
        text = PROCESS_CATALOG_DOC.read_text(encoding="utf-8")
        for match in re.finditer(r"context\.py ([a-z-]+)", text):
            self.assertIn(
                ("memory-bank/scripts/context.py", match.group(1)), allowed
            )


class InventoryReportingSplitDocTest(unittest.TestCase):
    """C3(d): a run that produced five derived skills plus the quartet must
    never be summarized as nine skills 'for your project'."""

    def test_infra_generate_output_template_splits_the_two_classes(self) -> None:
        flat = normalized(INFRA_GENERATE_DOC)
        self.assertIn("**Project skills generated:**", flat)
        self.assertIn("**Runtime guides generated:**", flat)
        self.assertNotIn("**Skills generated:**", flat)

    def test_skill_forge_output_template_splits_the_two_classes(self) -> None:
        flat = normalized(SKILL_FORGE_DOC)
        self.assertIn("**Project skills staged:**", flat)
        self.assertIn("**Runtime guides staged:**", flat)
        self.assertNotIn("**Skills staged:**", flat)

    def test_validator_exposes_the_split_the_docs_promise(self) -> None:
        self.assertTrue(hasattr(validate_skill_quality, "skill_class_split"))
        self.assertIn(
            "skill_classes",
            (
                ROOT
                / ".agents/skills/bootstrap-verifier/scripts/validate_skill_quality.py"
            ).read_text(encoding="utf-8"),
        )


if __name__ == "__main__":
    unittest.main()
