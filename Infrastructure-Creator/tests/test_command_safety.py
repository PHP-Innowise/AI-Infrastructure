#!/usr/bin/env python3
"""Regression tests for static command and script-alias safety analysis."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ANALYZER_PATH = (
    ROOT / ".agents/skills/bootstrap-verifier/scripts/analyze_commands.py"
)
SPEC = importlib.util.spec_from_file_location("analyze_commands", ANALYZER_PATH)
analyzer_module = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = analyzer_module
SPEC.loader.exec_module(analyzer_module)


class CommandSafetyTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="command-safety-")
        self.target = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)

    def write_json(self, name: str, value: object) -> None:
        (self.target / name).write_text(
            json.dumps(value, indent=2) + "\n", encoding="utf-8"
        )

    def analyzer(
        self,
        composer_scripts: dict | None = None,
        package_scripts: dict | None = None,
    ):
        if composer_scripts is not None:
            self.write_json("composer.json", {"scripts": composer_scripts})
        if package_scripts is not None:
            self.write_json("package.json", {"scripts": package_scripts})
        return analyzer_module.CommandAnalyzer(self.target)

    def assert_category(self, analysis, category: str) -> None:
        self.assertIn(category, analysis.categories, analysis.to_dict())

    def assert_code(self, analysis, code: str) -> None:
        self.assertIn(code, [finding.code for finding in analysis.findings])

    def test_non_mutating_commands_are_verification_safe(self) -> None:
        command_analyzer = self.analyzer()
        for command in (
            "vendor/bin/phpunit --testsuite unit",
            "vendor/bin/phpstan analyse --no-progress",
            "eslint src",
            "prettier --check src",
            "git status --short",
        ):
            with self.subTest(command=command):
                analysis = command_analyzer.analyze(command, verification=True)
                self.assertEqual(
                    analysis.categories,
                    (analyzer_module.Risk.NON_MUTATING.value,),
                )
                self.assertTrue(analysis.verification_safe)
                self.assertNotIn(
                    "UNSAFE_VERIFICATION",
                    [finding.code for finding in analysis.findings],
                )

    def test_fix_write_and_format_defaults_are_workspace_mutations(self) -> None:
        command_analyzer = self.analyzer()
        for command in (
            "eslint . --fix",
            "prettier --write src",
            "vendor/bin/pint",
            "php-cs-fixer fix src",
            "ruff --fix .",
        ):
            with self.subTest(command=command):
                analysis = command_analyzer.analyze(command, verification=True)
                self.assert_category(analysis, "workspace_mutation")
                self.assertFalse(analysis.verification_safe)
                self.assert_code(analysis, "UNSAFE_VERIFICATION")

    def test_dry_run_formatters_remain_non_mutating(self) -> None:
        command_analyzer = self.analyzer()
        for command in (
            "vendor/bin/pint --test",
            "php-cs-fixer fix --dry-run --diff",
            "ruff format --check .",
        ):
            with self.subTest(command=command):
                analysis = command_analyzer.analyze(command, verification=True)
                self.assertEqual(analysis.categories, ("non_mutating",))
                self.assertTrue(analysis.verification_safe)

    def test_database_deploy_and_destructive_commands_are_high_risk(self) -> None:
        command_analyzer = self.analyzer()
        for command in (
            "php artisan migrate --force",
            "php artisan db:wipe",
            "terraform apply plan.tfplan",
            "kubectl delete deployment api",
            "helm upgrade app chart/",
            "git reset --hard HEAD~1",
            "rm -rf build",
        ):
            with self.subTest(command=command):
                analysis = command_analyzer.analyze(command, verification=True)
                self.assert_category(
                    analysis, "destructive_database_deploy"
                )
                self.assertFalse(analysis.verification_safe)

    def test_provider_network_and_dependency_operations_are_classified(self) -> None:
        command_analyzer = self.analyzer()
        for command in (
            "curl https://example.test/health",
            "aws s3 sync build s3://bucket",
            "stripe products create --name test",
            "php artisan stripe:sync",
            "git push origin HEAD",
        ):
            with self.subTest(command=command):
                analysis = command_analyzer.analyze(command)
                self.assert_category(analysis, "external_provider_network")

        dependency = command_analyzer.analyze("npm install")
        self.assert_category(dependency, "workspace_mutation")
        self.assert_category(dependency, "external_provider_network")

    def test_composer_aliases_resolve_transitively_and_preserve_arrays(self) -> None:
        command_analyzer = self.analyzer(
            composer_scripts={
                "unit": "@php vendor/bin/phpunit --testsuite unit",
                "check": ["@unit", "@static"],
                "static": "vendor/bin/phpstan analyse",
            }
        )
        analysis = command_analyzer.analyze(
            "composer run-script check", verification=True
        )
        self.assertTrue(analysis.verification_safe)
        self.assertEqual(
            analysis.aliases,
            ("composer:check", "composer:unit", "composer:static"),
        )
        self.assertEqual(
            analysis.expanded_commands,
            (
                "@php vendor/bin/phpunit --testsuite unit",
                "vendor/bin/phpstan analyse",
            ),
        )

    def test_package_aliases_resolve_transitively_and_forward_arguments(self) -> None:
        command_analyzer = self.analyzer(
            package_scripts={
                "lint:base": "eslint src",
                "lint": "npm run lint:base",
            }
        )
        analysis = command_analyzer.analyze(
            "npm run lint -- --max-warnings=0", verification=True
        )
        self.assertTrue(analysis.verification_safe)
        self.assertEqual(
            analysis.aliases, ("package:lint", "package:lint:base")
        )
        self.assertEqual(
            analysis.expanded_commands,
            ("eslint src -- --max-warnings=0",),
        )

    def test_alias_expansion_carries_mutation_and_network_risk(self) -> None:
        command_analyzer = self.analyzer(
            composer_scripts={"verify": "composer install"},
            package_scripts={"lint": "eslint . --fix"},
        )
        composer = command_analyzer.analyze(
            "composer verify", verification=True
        )
        self.assert_category(composer, "workspace_mutation")
        self.assert_category(composer, "external_provider_network")
        self.assertFalse(composer.verification_safe)

        package = command_analyzer.analyze("npm run lint", verification=True)
        self.assert_category(package, "workspace_mutation")
        self.assertFalse(package.verification_safe)

    def test_package_lifecycle_aliases_are_included_in_risk(self) -> None:
        command_analyzer = self.analyzer(
            package_scripts={
                "pretest": "eslint . --fix",
                "test": "vitest run",
                "posttest": "curl https://example.test/report",
            }
        )
        analysis = command_analyzer.analyze("npm test", verification=True)
        self.assertEqual(
            analysis.aliases,
            ("package:test", "package:pretest", "package:posttest"),
        )
        self.assert_category(analysis, "workspace_mutation")
        self.assert_category(analysis, "external_provider_network")
        self.assertFalse(analysis.verification_safe)

    def test_alias_cycles_fail_closed_with_chain(self) -> None:
        command_analyzer = self.analyzer(
            composer_scripts={"a": "@b", "b": "@a"},
            package_scripts={"left": "npm run right", "right": "npm run left"},
        )
        for command in ("composer a", "npm run left"):
            with self.subTest(command=command):
                analysis = command_analyzer.analyze(
                    command, verification=True
                )
                self.assert_code(analysis, "ALIAS_CYCLE")
                self.assert_code(analysis, "UNSAFE_VERIFICATION")
                self.assertFalse(analysis.verification_safe)

    def test_unknown_explicit_alias_fails_closed_for_verification(self) -> None:
        command_analyzer = self.analyzer()
        for command in (
            "npm run missing",
            "npm run --silent",
            "composer run-script missing",
            "composer run --quiet",
            "composer missing",
        ):
            with self.subTest(command=command):
                analysis = command_analyzer.analyze(
                    command, verification=True
                )
                self.assert_code(analysis, "UNKNOWN_ALIAS")
                self.assert_code(analysis, "UNSAFE_VERIFICATION")
                self.assertFalse(analysis.verification_safe)

    def test_shell_composition_and_invalid_quoting_fail_closed(self) -> None:
        command_analyzer = self.analyzer()
        for command, expected in (
            ("phpunit && curl example.test", "SHELL_COMPOSITION"),
            ("phpunit | tee result.txt", "SHELL_COMPOSITION"),
            ("phpunit &", "SHELL_COMPOSITION"),
            ("phpunit > result.txt", "SHELL_COMPOSITION"),
            ("echo $(phpunit)", "SHELL_COMPOSITION"),
            ("bash -c 'vendor/bin/phpunit'", "SHELL_INTERPRETER"),
            ("phpunit 'unterminated", "TOKENIZE_ERROR"),
        ):
            with self.subTest(command=command):
                analysis = command_analyzer.analyze(
                    command, verification=True
                )
                self.assert_code(analysis, expected)
                self.assertFalse(analysis.verification_safe)
                if expected != "SHELL_INTERPRETER":
                    self.assertEqual(analysis.expanded_commands, ())

    def test_environment_assignments_are_tokenized_without_execution(self) -> None:
        command_analyzer = self.analyzer()
        analysis = command_analyzer.analyze(
            "env APP_ENV=test XDEBUG_MODE=off vendor/bin/phpunit",
            verification=True,
        )
        self.assertTrue(analysis.verification_safe)
        self.assertEqual(analysis.expanded_commands, (
            "env APP_ENV=test XDEBUG_MODE=off vendor/bin/phpunit",
        ))

    def test_all_scripts_are_namespaced_when_names_overlap(self) -> None:
        command_analyzer = self.analyzer(
            composer_scripts={"test": "vendor/bin/phpunit"},
            package_scripts={"test": "vitest run"},
        )
        analyses = command_analyzer.analyze_scripts(verification=True)
        self.assertEqual(
            set(analyses), {"composer:test", "package:test"}
        )
        self.assertTrue(all(item.verification_safe for item in analyses.values()))

    def test_malformed_or_invalid_script_metadata_is_rejected(self) -> None:
        cases = (
            ("composer.json", "{"),
            ("composer.json", json.dumps({"scripts": []})),
            ("composer.json", json.dumps({"scripts": {"test": 1}})),
            ("package.json", json.dumps({"scripts": {"test": ["vitest"]}})),
        )
        for filename, content in cases:
            with self.subTest(filename=filename, content=content):
                for path in self.target.iterdir():
                    path.unlink()
                (self.target / filename).write_text(content, encoding="utf-8")
                with self.assertRaises(analyzer_module.CommandAnalysisError):
                    analyzer_module.CommandAnalyzer(self.target)

    def test_public_api_returns_deterministic_serializable_results(self) -> None:
        self.analyzer(package_scripts={"test": "vitest run"})
        result = analyzer_module.analyze_target(
            self.target,
            ["git status --short"],
            verification=True,
        )
        document = result.to_dict()
        self.assertEqual(list(document["scripts"]), ["package:test"])
        self.assertEqual(document["commands"][0]["categories"], (
            "non_mutating",
        ))
        json.dumps(document)

    def test_cli_outputs_json_and_uses_verification_exit_status(self) -> None:
        self.write_json(
            "package.json",
            {"scripts": {"test": "vitest run", "lint": "eslint . --fix"}},
        )
        command = [
            sys.executable,
            str(ANALYZER_PATH),
            "--target",
            str(self.target),
            "--verification",
        ]
        result = subprocess.run(
            command, text=True, capture_output=True, check=False
        )
        self.assertEqual(result.returncode, 1, result.stderr)
        document = json.loads(result.stdout)
        self.assertTrue(document["scripts"]["package:test"]["verification_safe"])
        self.assertFalse(document["scripts"]["package:lint"]["verification_safe"])

        explicit = subprocess.run(
            [
                *command,
                "--no-scripts",
                "--command",
                "vendor/bin/phpunit",
            ],
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(explicit.returncode, 0, explicit.stderr)
        self.assertEqual(
            json.loads(explicit.stdout)["commands"][0]["categories"],
            ["non_mutating"],
        )


if __name__ == "__main__":
    unittest.main()
