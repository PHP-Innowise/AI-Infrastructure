#!/usr/bin/env python3
"""Tests for scripts/kit_fetcher.py - all network calls mocked, no live traffic.

The parsing/extraction logic is what can actually regress; the real network
call is a thin wrapper this suite deliberately does not exercise, so
`python3 -m unittest` stays fast, deterministic, and safe to run without
internet access or a GitHub API rate-limit budget.
"""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
_spec = importlib.util.spec_from_file_location("kit_fetcher", ROOT / "scripts" / "kit_fetcher.py")
kf = importlib.util.module_from_spec(_spec)
# dataclasses resolves deferred (`from __future__ import annotations`) type
# hints against sys.modules[cls.__module__] - it must be registered before
# exec_module runs the class bodies, or Python 3.9 crashes with an
# AttributeError instead of loading the module.
sys.modules["kit_fetcher"] = kf
_spec.loader.exec_module(kf)


README_ONE_CANDIDATE = """
# Some Tool

## Install

Run this:

```bash
npx some-tool@latest install
```

That's it.
"""

README_ZERO_CANDIDATES = """
# Some Tool

## Architecture

Here is a rendered diagram of the request flow:

```bash
echo "example output shown for illustration"
```
"""

README_TWO_CANDIDATES = """
## Install via npm

```bash
npm install -g some-tool
```

## Or install via pipx

```bash
pipx install some-tool
```
"""


class ParseOwnerRepoTests(unittest.TestCase):
    def test_plain_url(self) -> None:
        self.assertEqual(
            kf.parse_github_owner_repo("https://github.com/owner/repo"), ("owner", "repo")
        )

    def test_trailing_slash(self) -> None:
        self.assertEqual(
            kf.parse_github_owner_repo("https://github.com/owner/repo/"), ("owner", "repo")
        )

    def test_non_github_url_returns_none(self) -> None:
        self.assertIsNone(kf.parse_github_owner_repo("https://gitlab.com/owner/repo"))

    def test_invalid_hosts_and_paths_never_fetch(self) -> None:
        urls = (
            "https://notgithub.com/owner/repo", "https://evil.example/github.com/owner/repo",
            "https://github.com@evil.example/owner/repo", "https://evil@github.com/owner/repo",
            "http://github.com/owner/repo", "https://github.com:444/owner/repo",
            "https://github.com/owner/repo?other=value", "https://github.com/owner/repo#other",
            "https://github.com/owner/..", "https://github.com/owner/repo/tree/main",
            "https://[malformed", None,
        )
        for url in urls:
            with self.subTest(url=url), patch.object(kf, "_get") as get:
                metadata, candidates, errors = kf.refresh(url)
                self.assertIsNone(metadata)
                self.assertEqual(candidates, [])
                self.assertTrue(errors)
                get.assert_not_called()

    def test_git_suffix_is_removed(self) -> None:
        self.assertEqual(kf.parse_github_owner_repo("https://github.com/owner/repo.git"),
                         ("owner", "repo"))


class ExtractCandidatesTests(unittest.TestCase):
    def test_one_unambiguous_candidate(self) -> None:
        candidates = kf.extract_candidates(README_ONE_CANDIDATE)
        self.assertEqual(len(candidates), 1)
        self.assertIn("npx some-tool@latest install", candidates[0].command)

    def test_code_block_with_no_install_context_is_skipped(self) -> None:
        candidates = kf.extract_candidates(README_ZERO_CANDIDATES)
        self.assertEqual(candidates, [])

    def test_two_candidates_are_both_returned_as_ambiguous(self) -> None:
        """The caller decides zero/one/many; extraction itself never picks a winner."""
        candidates = kf.extract_candidates(README_TWO_CANDIDATES)
        self.assertEqual(len(candidates), 2)

    def test_empty_code_block_is_ignored(self) -> None:
        text = "install this:\n```bash\n\n```\n"
        self.assertEqual(kf.extract_candidates(text), [])


class FetchRepoMetadataTests(unittest.TestCase):
    def test_malformed_metadata_becomes_fetch_error(self) -> None:
        valid = {"stargazers_count": 1, "forks_count": 0, "open_issues_count": 0,
                 "pushed_at": "2026-09-05T00:00:00Z", "license": None}
        cases = [None, [], 42, "text", {"stargazers_count": 1}]
        cases.extend(dict(valid, **{field: value}) for field, value in (
            ("stargazers_count", True), ("forks_count", -1),
            ("open_issues_count", "0"), ("pushed_at", []),
            ("license", []), ("license", {"spdx_id": 1}),
        ))
        for data in cases:
            with self.subTest(data=data), patch.object(kf, "_get", return_value=json.dumps(data).encode()):
                with self.assertRaises(kf.FetchError):
                    kf.fetch_repo_metadata("owner", "repo")
        with patch.object(kf, "_get", return_value=b"\xff"):
            with self.assertRaises(kf.FetchError):
                kf.fetch_repo_metadata("owner", "repo")
    def test_successful_response_is_parsed(self) -> None:
        body = json.dumps({
            "stargazers_count": 42,
            "forks_count": 3,
            "pushed_at": "2026-09-01T00:00:00Z",
            "open_issues_count": 5,
            "license": {"spdx_id": "MIT"},
        }).encode("utf-8")
        with patch.object(kf, "_get", return_value=body):
            meta = kf.fetch_repo_metadata("owner", "repo")
        self.assertEqual(meta.stars, 42)
        self.assertEqual(meta.license_spdx, "MIT")

    def test_missing_license_does_not_crash(self) -> None:
        body = json.dumps({
            "stargazers_count": 1, "forks_count": 0,
            "pushed_at": "2026-09-01T00:00:00Z", "open_issues_count": 0,
            "license": None,
        }).encode("utf-8")
        with patch.object(kf, "_get", return_value=body):
            meta = kf.fetch_repo_metadata("owner", "repo")
        self.assertIsNone(meta.license_spdx)

    def test_api_error_message_is_wrapped_not_raised_as_keyerror(self) -> None:
        body = json.dumps({"message": "Not Found"}).encode("utf-8")
        with patch.object(kf, "_get", return_value=body):
            with self.assertRaises(kf.FetchError) as ctx:
                kf.fetch_repo_metadata("owner", "does-not-exist")
        self.assertIn("Not Found", str(ctx.exception))

    def test_non_json_body_raises_fetch_error_not_json_decode_error(self) -> None:
        with patch.object(kf, "_get", return_value=b"not json at all"):
            with self.assertRaises(kf.FetchError):
                kf.fetch_repo_metadata("owner", "repo")


class RefreshTests(unittest.TestCase):
    """`refresh()` must never raise - it collects failures instead."""

    def test_non_github_url_reports_an_error_without_raising(self) -> None:
        metadata, candidates, errors = kf.refresh("https://gitlab.com/owner/repo")
        self.assertIsNone(metadata)
        self.assertEqual(candidates, [])
        self.assertTrue(errors)

    def test_metadata_failure_does_not_block_readme_success(self) -> None:
        with patch.object(kf, "fetch_repo_metadata", side_effect=kf.FetchError("boom")):
            with patch.object(kf, "fetch_readme_text", return_value=README_ONE_CANDIDATE):
                metadata, candidates, errors = kf.refresh("https://github.com/owner/repo")
        self.assertIsNone(metadata)
        self.assertEqual(len(candidates), 1)
        self.assertEqual(len(errors), 1)

    def test_readme_failure_does_not_block_metadata_success(self) -> None:
        fake_meta = kf.RepoMetadata(1, 1, "2026-09-01T00:00:00Z", 0, "MIT")
        with patch.object(kf, "fetch_repo_metadata", return_value=fake_meta):
            with patch.object(kf, "fetch_readme_text", side_effect=kf.FetchError("boom")):
                metadata, candidates, errors = kf.refresh("https://github.com/owner/repo")
        self.assertEqual(metadata, fake_meta)
        self.assertEqual(candidates, [])
        self.assertEqual(len(errors), 1)

    def test_both_succeed(self) -> None:
        fake_meta = kf.RepoMetadata(1, 1, "2026-09-01T00:00:00Z", 0, "MIT")
        with patch.object(kf, "fetch_repo_metadata", return_value=fake_meta):
            with patch.object(kf, "fetch_readme_text", return_value=README_ONE_CANDIDATE):
                metadata, candidates, errors = kf.refresh("https://github.com/owner/repo")
        self.assertEqual(metadata, fake_meta)
        self.assertEqual(len(candidates), 1)
        self.assertEqual(errors, [])

    def test_a_urlerror_becomes_a_fetch_error_not_an_uncaught_exception(self) -> None:
        with patch(
            "urllib.request.urlopen",
            side_effect=urllib.error.URLError("no network"),
        ):
            metadata, candidates, errors = kf.refresh("https://github.com/owner/repo")
        self.assertIsNone(metadata)
        self.assertEqual(candidates, [])
        self.assertEqual(len(errors), 2)  # metadata call and README call both fail

    def test_timeout_is_bounded_and_reported_for_each_endpoint(self) -> None:
        with patch("urllib.request.urlopen", side_effect=TimeoutError("timed out")) as get:
            metadata, candidates, errors = kf.refresh("https://github.com/owner/repo")
        self.assertIsNone(metadata)
        self.assertEqual(candidates, [])
        self.assertEqual(len(errors), 2)
        self.assertEqual(get.call_count, 2)
        for call in get.call_args_list:
            self.assertEqual(call.kwargs["timeout"], kf.TIMEOUT_SECONDS)

    def test_malformed_metadata_does_not_block_readme(self) -> None:
        with patch.object(kf, "_get", side_effect=[b"[]", README_ONE_CANDIDATE.encode()]):
            metadata, candidates, errors = kf.refresh("https://github.com/owner/repo")
        self.assertIsNone(metadata)
        self.assertEqual(len(candidates), 1)
        self.assertEqual(len(errors), 1)


if __name__ == "__main__":
    unittest.main()
