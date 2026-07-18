#!/usr/bin/env python3
"""Regression tests for skill evaluation reports."""

from __future__ import annotations

import unittest

from scripts.generate_report import generate_html


class GenerateReportTest(unittest.TestCase):
    def test_no_holdout_results_render_as_an_empty_test_set(self) -> None:
        iteration = {
            "iteration": 1,
            "description": "Improved description",
            "train_passed": 2,
            "train_failed": 0,
            "train_total": 2,
            "train_results": [],
            "test_passed": None,
            "test_failed": None,
            "test_total": None,
            "test_results": None,
            "passed": 2,
            "failed": 0,
            "total": 2,
            "results": [],
        }
        output = {
            "original_description": "Original description",
            "best_description": "Improved description",
            "best_score": "2/2",
            "iterations_run": 1,
            "holdout": 0,
            "train_size": 2,
            "test_size": 0,
            "history": [iteration],
        }

        html = generate_html(output, skill_name="example-skill")

        self.assertIn("Improved description", html)
        self.assertIn("example-skill", html)


if __name__ == "__main__":
    unittest.main()
