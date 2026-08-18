#!/usr/bin/env python3
"""Contract tests for the seven Phase 1 discovery scanners.

A full pipeline run against a real Symfony/UniteCMS target showed that the
scanner prompts and the machine plan schema had drifted apart: no scanner
mentioned evidence ids, sha256 fingerprints or line ranges, so a literal
execution produced zero gate-eligible evidence; parallel scanners invented
`TASK-1` and `TASK-001` in the same run; cross-scanner steps said "take it
from stack-scanner findings" without an address; and several detection steps
named one filename where the target used another member of the same class
(DDEV instead of Dockerfile, `platform-overrides` instead of `platform`,
`deploy.sh`/`.ddev` hooks/`#[AsCommand]` instead of Composer scripts, config
wiring instead of `composer.json` `require`).

These tests pin the fixes: one shared contract every scanner points at, the
two-artifact output convention, the zero-padded task directory, addressed
sibling inputs, one identical secrets rule in the two scanners that used to
contradict each other, and the breadth checklists that replaced the
single-filename assumptions. They also pin the contract's evidence record to
what `validate_skill_quality.py` and the profile schema actually accept.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKILLS = ROOT / ".agents/skills"
CONTRACT = SKILLS / "stack-scanner/references/scan-evidence-contract.md"
SCHEMA_DOC = (
    SKILLS / "profile-synthesizer/references/project-profile-schema.md"
)
VALIDATOR = (
    SKILLS / "bootstrap-verifier/scripts/validate_skill_quality.py"
)

SCANNERS = (
    "stack-scanner",
    "architecture-scanner",
    "conventions-scanner",
    "domain-behavior-scanner",
    "integration-scanner",
    "security-compliance-scanner",
    "infra-ops-scanner",
)

CONTRACT_REF = "stack-scanner/references/scan-evidence-contract.md"

SECRETS_RULE_SCANNERS = ("integration-scanner", "security-compliance-scanner")

# The behavioral source-type vocabulary the profile schema fixes; the scan
# contract must offer scanners exactly these, never a private synonym.
SOURCE_TYPES = (
    "spec/ADR",
    "test",
    "database constraint",
    "workflow configuration",
    "authorization rule",
    "domain code",
    "application code",
    "configuration",
    "interview answer",
)


def skill_text(name: str) -> str:
    return (SKILLS / name / "SKILL.md").read_text(encoding="utf-8")


def contract_text() -> str:
    return CONTRACT.read_text(encoding="utf-8")


def collapsed(text: str) -> str:
    """Collapse whitespace so assertions survive re-wrapping."""
    return re.sub(r"\s+", " ", text)


class ScannerOutputContractTest(unittest.TestCase):
    def test_contract_exists_and_names_every_scanner(self) -> None:
        text = contract_text()
        for name in SCANNERS:
            self.assertIn(f"`{name}`", text, f"{name} missing from the contract")

    def test_every_scanner_points_at_the_single_contract(self) -> None:
        for name in SCANNERS:
            self.assertIn(CONTRACT_REF, skill_text(name), name)

    def test_every_scanner_declares_both_artifacts(self) -> None:
        for name in SCANNERS:
            text = skill_text(name)
            self.assertIn(f"tasks/TASK-{{NNN}}/{name}-findings.md", text, name)
            self.assertIn(f"tasks/TASK-{{NNN}}/{name}-evidence.json", text, name)

    def test_report_and_ledger_are_separately_exactly_one(self) -> None:
        """`exactly one` must attach to each artifact kind, not to the pair."""
        for name in SCANNERS:
            body = collapsed(skill_text(name))
            self.assertRegex(
                body,
                rf"exactly one report `tasks/TASK-\{{NNN\}}/{name}-findings\.md` "
                rf"and exactly one evidence ledger "
                rf"`tasks/TASK-\{{NNN\}}/{name}-evidence\.json`",
                name,
            )

    def test_no_scanner_uses_the_unpadded_task_directory(self) -> None:
        for name in SCANNERS:
            self.assertNotIn("TASK-{N}/", skill_text(name), name)

    def test_contract_pins_three_digit_zero_padding(self) -> None:
        body = collapsed(contract_text())
        self.assertIn("zero-padded to three digits", body)
        self.assertIn("`TASK-001`", body)
        self.assertIn("`TASK-1` is malformed", body)

    def test_scanners_no_longer_claim_a_single_findings_file(self) -> None:
        for name in SCANNERS:
            self.assertNotIn(
                "Write exactly one findings file", skill_text(name), name
            )


class EvidenceRecordContractTest(unittest.TestCase):
    def test_contract_declares_every_accepted_evidence_member(self) -> None:
        body = contract_text()
        for field in (
            "id",
            "path",
            "url",
            "source_type",
            "authority",
            "confidence",
            "line_range",
            "fingerprint",
            "supported_claims",
        ):
            self.assertIn(f"`{field}`", body, field)

    def test_contract_members_match_the_validator_allow_list(self) -> None:
        source = VALIDATOR.read_text(encoding="utf-8")
        block = re.search(
            r"allowed_evidence_fields = \{(.*?)\}", source, re.S
        )
        self.assertIsNotNone(block, "validator allow-list not found")
        allowed = set(re.findall(r'"([a-z_]+)"', block.group(1)))
        body = contract_text()
        for field in allowed:
            self.assertIn(f"`{field}`", body, field)

    def test_contract_source_types_match_the_profile_schema(self) -> None:
        """The scanners' vocabulary is the schema's, not one of their own."""
        schema = SCHEMA_DOC.read_text(encoding="utf-8")
        body = contract_text()
        for source_type in SOURCE_TYPES:
            self.assertIn(f"`{source_type}`", schema, source_type)
            self.assertIn(f"`{source_type}`", body, source_type)

    def test_contract_requires_a_whole_file_sha256_fingerprint(self) -> None:
        body = collapsed(contract_text())
        self.assertIn("sha256:", body)
        self.assertIn("whole file's bytes", body)
        self.assertIn("hashlib.sha256", body)

    def test_contract_forbids_absolute_and_task_paths_in_evidence(self) -> None:
        body = collapsed(contract_text())
        self.assertIn("relative to the target root", body)
        self.assertIn("Never absolute", body)
        self.assertIn("never a path under `tasks/TASK-*`", body)

    def test_every_scanner_is_required_to_emit_the_ledger(self) -> None:
        for name in SCANNERS:
            body = collapsed(skill_text(name))
            self.assertIn("MUST emit both artifacts", body, name)
            self.assertIn("`sha256:` fingerprint", body, name)

    def test_evidence_ids_are_namespaced_per_scanner(self) -> None:
        body = contract_text()
        for tag in ("STK", "ARC", "CNV", "DOM", "INT", "SEC", "OPS"):
            self.assertIn(f"`{tag}`", body, tag)


class SiblingInputContractTest(unittest.TestCase):
    def test_contract_addresses_sibling_artifacts_and_degradation(self) -> None:
        body = collapsed(contract_text())
        self.assertIn("tasks/TASK-{NNN}/<sibling>-findings.md", body)
        self.assertIn("tasks/TASK-{NNN}/<sibling>-evidence.json", body)
        self.assertIn("Missing sibling input:", body)
        self.assertIn("do not invent its verdict", body)

    def test_conventions_scanner_addresses_stack_scanner_artifacts(self) -> None:
        body = collapsed(skill_text("conventions-scanner"))
        self.assertIn("tasks/TASK-{NNN}/stack-scanner-findings.md", body)
        self.assertIn("tasks/TASK-{NNN}/stack-scanner-evidence.json", body)
        self.assertIn("sibling-input fallback", body)

    def test_domain_scanner_addresses_both_siblings_it_consumes(self) -> None:
        body = collapsed(skill_text("domain-behavior-scanner"))
        self.assertIn("tasks/TASK-{NNN}/architecture-scanner-findings.md", body)
        self.assertIn("tasks/TASK-{NNN}/stack-scanner-findings.md", body)
        self.assertIn("sibling-input fallback", body)

    def test_consumers_declare_a_degradation_guardrail(self) -> None:
        for name in ("conventions-scanner", "domain-behavior-scanner"):
            body = collapsed(skill_text(name))
            self.assertRegex(
                body, r"MUST take sibling .*this run's task directory", name
            )


class SecretsRuleTest(unittest.TestCase):
    def test_the_two_scanners_state_one_identical_rule(self) -> None:
        rules = []
        for name in SECRETS_RULE_SCANNERS:
            match = re.search(
                r"\*\*Secrets rule\*\*[^\n]*?\):(.*?)(?:\n\n)",
                skill_text(name),
                re.S,
            )
            self.assertIsNotNone(match, f"{name} has no secrets rule")
            rules.append(collapsed(match.group(1)).strip())
        shared = (
            "never open, read, print, or fingerprint `.env`/`.env.*` or any "
            "credential store, and never record a value."
        )
        for rule in rules:
            self.assertIn(shared, rule)
            self.assertIn("non-secret committed source", rule)
            self.assertIn("`.env.example`", rule)

    def test_the_two_scanners_cross_reference_each_other(self) -> None:
        self.assertIn(
            "identical in `security-compliance-scanner`",
            skill_text("integration-scanner"),
        )
        self.assertIn(
            "identical in `integration-scanner`",
            skill_text("security-compliance-scanner"),
        )

    def test_security_scanner_may_list_key_names_without_reading_env(self) -> None:
        body = collapsed(skill_text("security-compliance-scanner"))
        self.assertIn("without opening a secret file", body)
        self.assertIn("key *names*", body)
        self.assertIn("stays `unknown`", body)

    def test_contract_holds_the_full_rule_once(self) -> None:
        body = collapsed(contract_text())
        self.assertIn("Secrets rule (identical for every scanner)", body)
        self.assertIn("Existence may be recorded; contents may not.", body)


class DetectionBreadthTest(unittest.TestCase):
    """The single-filename assumptions the real target falsified."""

    def test_contract_states_the_signal_class_principle(self) -> None:
        body = collapsed(contract_text())
        self.assertIn("Signal classes, not filenames", body)
        self.assertIn("search the whole class", body)

    def test_containerization_class_covers_environment_managers(self) -> None:
        contract = collapsed(contract_text())
        for member in (".ddev/config.yaml", ".lando.yml", ".devcontainer/"):
            self.assertIn(member, contract, member)
        skill = collapsed(skill_text("infra-ops-scanner"))
        self.assertIn("signal class, never from one filename", skill)
        self.assertIn(".ddev/config.yaml", skill)

    def test_command_sites_cover_deploy_hooks_and_console_commands(self) -> None:
        contract = collapsed(contract_text())
        for member in (
            "deploy.sh",
            ".ddev/config.yaml` `hooks:",
            "#[AsCommand]",
            "console.command",
        ):
            self.assertIn(member, contract, member)
        for name in ("stack-scanner", "infra-ops-scanner"):
            self.assertIn("checklist", collapsed(skill_text(name)), name)

    def test_stack_scanner_separates_constraint_from_pin(self) -> None:
        body = collapsed(skill_text("stack-scanner"))
        self.assertIn("platform-overrides", body)
        self.assertIn("never present a constraint as the resolved version", body)
        self.assertIn("platform` mirrors the *constraint*", body)

    def test_stack_scanner_flags_history_and_schema_writes(self) -> None:
        body = collapsed(skill_text("stack-scanner"))
        self.assertIn("doctrine:schema:update --force", body)
        self.assertIn("history rewrites", body)

    def test_infra_ops_searches_every_site_for_destructive_commands(self) -> None:
        body = collapsed(skill_text("infra-ops-scanner"))
        self.assertIn("command declaration site", body)
        self.assertIn("doctrine:schema:update --force", body)
        self.assertIn('"None detected" is reportable only after', body)

    def test_integration_scanner_looks_beyond_composer_require(self) -> None:
        body = collapsed(skill_text("integration-scanner"))
        self.assertIn("not from `require` alone", body)
        self.assertIn("config/bundles.php", body)
        self.assertIn("transitively", body)
        self.assertIn('"No integrations" is reportable only after', body)

    def test_integration_source_checklist_lists_the_five_sources(self) -> None:
        body = collapsed(contract_text())
        self.assertIn("Integration candidate sources", body)
        for member in (
            "config/packages/**",
            "composer.lock",
            "deploy scripts",
            "package.json",
        ):
            self.assertIn(member, body, member)


class ReportTemplateTest(unittest.TestCase):
    def test_templates_live_in_the_contract_appendix(self) -> None:
        body = contract_text()
        self.assertIn("## Appendix A - report templates", body)
        for name in SCANNERS:
            self.assertIn(f"### {name}\n", body, name)

    def test_every_scanner_points_at_its_appendix_template(self) -> None:
        for name in SCANNERS:
            body = collapsed(skill_text(name))
            self.assertIn(
                f"Follow the `{name}` report template in appendix A", body, name
            )
            self.assertNotIn("## Output Template", skill_text(name), name)

    def test_each_template_keeps_a_confidence_summary(self) -> None:
        blocks = re.findall(
            r"### ([a-z-]+-scanner)\n\n```markdown\n(.*?)\n```",
            contract_text(),
            re.S,
        )
        self.assertEqual(len(blocks), len(SCANNERS))
        for name, block in blocks:
            self.assertIn("## Confidence Summary", block, name)


if __name__ == "__main__":
    unittest.main()
