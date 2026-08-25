#!/usr/bin/env python3
"""
Categorize git commits based on conventional commit prefixes.

Usage:
    python categorize_commits.py <commit-list-file>

Input format (one commit per line):
    abc1234 feat: add new feature
    def5678 fix: resolve bug
"""

import sys
import re
from typing import Dict, List, Tuple
from dataclasses import dataclass


@dataclass
class Commit:
    hash: str
    message: str
    category: str


CATEGORIES = {
    "Added": ["feat", "feature"],
    "Fixed": ["fix", "bugfix"],
    "Changed": ["refactor", "perf", "improvement", "update"],
    "Deprecated": ["deprecate", "deprecated"],
    "Removed": ["remove", "delete"],
    "Security": ["security", "sec"],
    "Documentation": ["docs", "doc"],
    "Build": ["build", "ci", "chore"],
}


def parse_commit(line: str) -> Tuple[str, str]:
    """Parse a git log line into hash and message."""
    parts = line.strip().split(" ", 1)
    if len(parts) == 2:
        return parts[0], parts[1]
    return parts[0], ""


def categorize_commit(message: str) -> str:
    """Categorize a commit message based on conventional commit prefix."""
    # Extract prefix before colon
    match = re.match(r"^(\w+)(?:\([\w-]+\))?:\s*(.+)", message)

    if not match:
        return "Changed"  # Default category

    prefix = match.group(1).lower()

    # Find matching category
    for category, prefixes in CATEGORIES.items():
        if prefix in prefixes:
            return category

    return "Changed"  # Default category


def format_commit(commit: Commit) -> str:
    """Format a commit for changelog output."""
    # Remove conventional commit prefix for cleaner output
    message = re.sub(r"^(\w+)(?:\([\w-]+\))?:\s*", "", commit.message)

    # Capitalize first letter
    if message:
        message = message[0].upper() + message[1:]

    return f"- {message} ({commit.hash})"


def main():
    if len(sys.argv) != 2:
        print("Usage: python categorize_commits.py <commit-list-file>")
        sys.exit(1)

    commits_file = sys.argv[1]

    # Read commits
    commits: List[Commit] = []
    with open(commits_file, "r") as f:
        for line in f:
            if not line.strip():
                continue

            commit_hash, message = parse_commit(line)
            category = categorize_commit(message)
            commits.append(Commit(commit_hash, message, category))

    # Group by category
    categorized: Dict[str, List[Commit]] = {}
    for commit in commits:
        if commit.category not in categorized:
            categorized[commit.category] = []
        categorized[commit.category].append(commit)

    # Output in Keep a Changelog order
    changelog_order = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"]

    for category in changelog_order:
        if category in categorized:
            print(f"\n### {category}\n")
            for commit in categorized[category]:
                print(format_commit(commit))

    # Output remaining categories (Build, Documentation, etc.) at the end
    other_categories = sorted(set(categorized.keys()) - set(changelog_order))
    for category in other_categories:
        print(f"\n### {category}\n")
        for commit in categorized[category]:
            print(format_commit(commit))


if __name__ == "__main__":
    main()
