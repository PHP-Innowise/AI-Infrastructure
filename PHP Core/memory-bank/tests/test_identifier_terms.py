#!/usr/bin/env python3
"""Identifier-aware query expansion for the local context index."""

from __future__ import annotations

import importlib.util
import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "context.py"
SPEC = importlib.util.spec_from_file_location("identifier_context", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("Cannot load context engine")
CONTEXT = importlib.util.module_from_spec(SPEC)
sys.path.insert(0, str(SCRIPT.parent))
try:
    SPEC.loader.exec_module(CONTEXT)
finally:
    sys.path.pop(0)
RETRIEVAL = sys.modules["context_retrieval"]


class IdentifierFormTest(unittest.TestCase):
    def test_camel_snake_namespace_and_dotted_names_expand(self) -> None:
        cases = {
            "getUserById": "get user by id",
            "HTTPClient": "http client",
            "get_user_by_id": "get user by id",
            "Foo\\Bar": "foo bar",
            "memory_eval": "memory eval",
            "AUTH_TOKEN": "auth token",
        }
        for token, expected in cases.items():
            with self.subTest(token=token):
                self.assertEqual(expected, RETRIEVAL.identifier_form(token))

    def test_ordinary_words_and_short_tokens_do_not_expand(self) -> None:
        for token in ("user", "User", "normalword", "HTTP", "v2", ""):
            with self.subTest(token=token):
                self.assertIsNone(RETRIEVAL.identifier_form(token))

    def test_document_terms_deduplicate_and_ignore_paths(self) -> None:
        rows = RETRIEVAL.identifier_terms(
            "specs/memory_eval.md",
            "Call getUserById and getUserById again; AUTH_TOKEN stays.",
        )
        self.assertIn(("getUserById", "get user by id"), rows)
        self.assertIn(("AUTH_TOKEN", "auth token"), rows)
        self.assertFalse(
            any("memoryeval" in identifier.casefold() for identifier, _ in rows), rows
        )
        self.assertEqual(len(rows), len(set(rows)))

    def test_citation_frontmatter_sources_references_and_files_are_not_identifiers(self) -> None:
        content = (
            "---\n{\"sources\": [\"src/AsterWillow.php\"]}\n---\n\n"
            "# Spindle\n\nCall getUserById when the woven seal is retained.\n\n"
            "## Sources\n- src/AsterWillow.php\n\n"
            "## References\n- src/AsterWillow.php\n\n"
            "## Files\n- src/AsterWillow.php\n"
        )
        rows = RETRIEVAL.identifier_terms("memory-bank/chunks/MEM-x.md", content)
        self.assertFalse(
            any("asterwillow" in identifier.casefold() for identifier, _ in rows), rows
        )
        self.assertIn(("getUserById", "get user by id"), rows)


class IdentifierRetrievalTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="identifier-terms-test-")
        self.repository = Path(self.temporary.name)
        subprocess.run(
            ["git", "init", "--quiet", str(self.repository)],
            check=True,
            capture_output=True,
            text=True,
        )
        self.repository.joinpath("specs").mkdir()
        self.repository.joinpath("memory-bank/chunks").mkdir(parents=True)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_context(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--root",
                str(self.repository),
                "--mode",
                "lightweight",
                *arguments,
            ],
            text=True,
            capture_output=True,
        )

    def write(self, relative: str, content: str) -> None:
        path = self.repository / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def index(self) -> None:
        result = self.run_context("index", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn('"documents"', result.stdout)

    def search(self, query: str) -> list[str]:
        result = self.run_context("search", query, "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        return [item["path"] for item in json.loads(result.stdout)["documents"]]

    def rows(self) -> list[tuple[str, str]]:
        connection = sqlite3.connect(self.repository / "memory-bank/local/context.db")
        try:
            return [
                (str(row[0]), str(row[1]))
                for row in connection.execute(
                    "SELECT identifier, expansion FROM identifier_terms "
                    "ORDER BY identifier, expansion"
                )
            ]
        finally:
            connection.close()

    def test_a_file_extension_does_not_enter_the_expansion(self) -> None:
        self.write(
            "specs/totals.md",
            "# Totals\n\nInvoiceTotal.php stores the minor units.\n",
        )
        self.index()
        self.assertIn("specs/totals.md", self.search("invoice total"))

    def test_words_reach_a_camel_case_identifier(self) -> None:
        # The title deliberately shares no word with the query: a match on
        # "user" from a "Users" heading would prove nothing about expansion.
        self.write(
            "specs/accounts.md",
            "# Accounts\n\nCall findUserByEmail when resolving a login.\n",
        )
        self.index()
        self.assertIn("specs/accounts.md", self.search("find user by email"))

    def test_an_identifier_does_not_expand_into_shared_subwords(self) -> None:
        self.write(
            "specs/loading.md",
            "# Loading\n\nTo load a profile, get the user by id from the route.\n",
        )
        self.write("specs/ids.md", "# Ids\n\nThe user id is stored elsewhere.\n")
        self.index()
        self.assertEqual([], self.search("getUserById"))

    def test_capsule_relevance_uses_the_expanded_query(self) -> None:
        self.write(
            "specs/accounts.md",
            "# Accounts\n\nCall findUserByEmail when resolving a login.\n",
        )
        self.index()
        result = self.run_context(
            "context", "find user by email", "--limit", "3", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        paths = [item["path"] for item in json.loads(result.stdout)["semantic"]]
        self.assertIn("specs/accounts.md", paths)

    def test_an_opaque_identifier_does_not_match_shared_subwords(self) -> None:
        self.write(
            "specs/signal.md",
            "# Signal\n\nSkylarkSignal842 uses the indigo gate.\n",
        )
        self.index()
        self.assertEqual([], self.search("SkylarkEpisode695"))

    def test_incremental_reindex_replaces_the_old_identifier(self) -> None:
        self.write("specs/users.md", "# Users\n\nCall getUserById.\n")
        self.index()
        self.assertIn(("getUserById", "get user by id"), self.rows())

        self.write("specs/users.md", "# Users\n\nCall findUserByEmail.\n")
        self.index()
        rows = self.rows()
        self.assertNotIn(("getUserById", "get user by id"), rows)
        self.assertIn(("findUserByEmail", "find user by email"), rows)

    def test_a_full_rebuild_drops_stale_identifier_rows(self) -> None:
        self.write("specs/accounts.md", "# Accounts\n\nCall findUserByEmail.\n")
        self.index()
        database = self.repository / "memory-bank/local/context.db"
        connection = sqlite3.connect(database)
        try:
            connection.execute("DROP TABLE documents")
            connection.commit()
        finally:
            connection.close()

        self.write("specs/accounts.md", "# Accounts\n\nCall findUserByPhone.\n")
        self.index()
        rows = self.rows()
        self.assertIn(("findUserByPhone", "find user by phone"), rows)
        self.assertNotIn(("findUserByEmail", "find user by email"), rows)

    def test_deleting_a_document_drops_its_identifier_rows(self) -> None:
        self.write("specs/accounts.md", "# Accounts\n\nCall findUserByEmail.\n")
        self.write("specs/totals.md", "# Totals\n\nCall InvoiceTotal.\n")
        self.index()
        (self.repository / "specs/accounts.md").unlink()
        self.index()
        rows = self.rows()
        self.assertNotIn(("findUserByEmail", "find user by email"), rows)
        self.assertIn(("InvoiceTotal", "invoice total"), rows)

    def test_ensure_identifier_table_repairs_a_missing_index(self) -> None:
        connection = sqlite3.connect(":memory:")
        try:
            connection.execute(
                "CREATE TABLE identifier_terms(path TEXT, identifier TEXT, expansion TEXT)"
            )
            self.assertFalse(RETRIEVAL.ensure_identifier_table(connection))
            indexes = {
                str(row[1])
                for row in connection.execute("PRAGMA index_list('identifier_terms')")
            }
        finally:
            connection.close()
        self.assertIn("identifier_terms_expansion", indexes)

    def test_an_older_index_rebuilds_the_derived_table(self) -> None:
        self.write("specs/users.md", "# Users\n\nCall getUserById.\n")
        self.index()
        database = self.repository / "memory-bank/local/context.db"
        connection = sqlite3.connect(database)
        try:
            connection.execute("DROP TABLE identifier_terms")
            connection.execute(
                "DELETE FROM index_state WHERE key = 'identifier_terms_ready'"
            )
            connection.commit()
        finally:
            connection.close()

        self.index()
        self.assertIn(("getUserById", "get user by id"), self.rows())
        self.assertIn("specs/users.md", self.search("get user by id"))


if __name__ == "__main__":
    unittest.main()
