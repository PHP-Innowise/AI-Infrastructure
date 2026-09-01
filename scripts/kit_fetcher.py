#!/usr/bin/env python3
"""Fetch a Kit 3 catalog resource's current repo metadata and README, and
extract install-command candidates from it - the mechanism
`install_open_source_kit.py --refresh` calls before recording an install
command into a manifest.

This reads two public GitHub endpoints (repo metadata, raw README) and looks
for fenced code blocks whose preceding text mentions an install-related
keyword. It never installs or executes anything, and it never guesses: when
it finds exactly one unambiguous candidate, that candidate is usable; zero or
several candidates means "a human still has to read the README", the same
conclusion as if this script did not exist. A network failure is caught and
reported, never raised past this module's own functions, so one flaky call
cannot crash a selection run over other resources.

Unauthenticated GitHub API calls are rate-limited to 60/hour per IP - fine for
occasional use, tight if refreshing the full catalog repeatedly in a short
window.

    python3 scripts/kit_fetcher.py --id graphify
    python3 scripts/kit_fetcher.py --url https://github.com/owner/repo
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CATALOG = ROOT / "install" / "open-source-kit" / "resources.json"

GITHUB_API = "https://api.github.com"
USER_AGENT = "accelerator-php-kit3-fetcher"
TIMEOUT_SECONDS = 10

INSTALL_KEYWORDS = (
    "install", "npx ", "npm ", "pip ", "pipx", "uv tool", "plugin",
    "clone", "curl", "marketplace", "setup",
)
CODE_BLOCK_RE = re.compile(r"```(?:bash|sh|shell|console|text)?\n(.*?)```", re.DOTALL)
GITHUB_URL_RE = re.compile(r"github\.com/([^/]+)/([^/]+?)/?$")


class FetchError(Exception):
    """A network call itself failed - not a parsing problem."""


@dataclass
class RepoMetadata:
    stars: int
    forks: int
    pushed_at: str
    open_issues: int
    license_spdx: str | None


@dataclass
class Candidate:
    context: str
    command: str


def parse_github_owner_repo(url: str) -> tuple[str, str] | None:
    match = GITHUB_URL_RE.search(url.rstrip("/"))
    if not match:
        return None
    return match.group(1), match.group(2)


def _get(url: str, accept: str) -> bytes:
    request = urllib.request.Request(
        url, headers={"User-Agent": USER_AGENT, "Accept": accept}
    )
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
            return response.read()
    except (urllib.error.URLError, TimeoutError, ConnectionError) as error:
        raise FetchError(f"{url}: {error}") from error


def fetch_repo_metadata(owner: str, repo: str) -> RepoMetadata:
    raw = _get(f"{GITHUB_API}/repos/{owner}/{repo}", "application/vnd.github+json")
    try:
        data = json.loads(raw)
    except json.JSONDecodeError as error:
        raise FetchError(f"{owner}/{repo}: metadata response was not JSON") from error
    if "stargazers_count" not in data:
        raise FetchError(
            f"{owner}/{repo}: {data.get('message', 'unexpected API response')}"
        )
    license_info = data.get("license") or {}
    return RepoMetadata(
        stars=data["stargazers_count"],
        forks=data["forks_count"],
        pushed_at=data["pushed_at"],
        open_issues=data["open_issues_count"],
        license_spdx=license_info.get("spdx_id"),
    )


def fetch_readme_text(owner: str, repo: str) -> str:
    raw = _get(f"{GITHUB_API}/repos/{owner}/{repo}/readme", "application/vnd.github.raw")
    return raw.decode("utf-8", errors="replace")


def extract_candidates(readme_text: str, context_window: int = 200) -> list[Candidate]:
    """Fenced code blocks whose preceding text mentions an install keyword.

    Deliberately simple: a heuristic, not a parser. It exists to narrow a
    human's search, not to replace their reading of the README - see the
    module docstring's rule on zero/multiple candidates.
    """
    candidates = []
    for match in CODE_BLOCK_RE.finditer(readme_text):
        start = match.start()
        preceding = readme_text[max(0, start - context_window):start].lower()
        if not any(keyword in preceding for keyword in INSTALL_KEYWORDS):
            continue
        command = match.group(1).strip()
        if not command:
            continue
        context_line = next(
            (line.strip() for line in reversed(preceding.splitlines()) if line.strip()),
            "",
        )
        candidates.append(Candidate(context=context_line, command=command))
    return candidates


def refresh(url: str) -> tuple[RepoMetadata | None, list[Candidate], list[str]]:
    """Best-effort refresh for one resource. Never raises - collects errors.

    Returns (metadata_or_None, candidates, errors). The caller decides what to
    do with a partial result: this function's only job is to not let one
    flaky network call crash a selection run over other resources.
    """
    errors: list[str] = []
    owner_repo = parse_github_owner_repo(url)
    if owner_repo is None:
        return None, [], [f"not a github.com URL: {url}"]
    owner, repo = owner_repo

    metadata = None
    try:
        metadata = fetch_repo_metadata(owner, repo)
    except FetchError as error:
        errors.append(str(error))

    candidates: list[Candidate] = []
    try:
        readme = fetch_readme_text(owner, repo)
        candidates = extract_candidates(readme)
    except FetchError as error:
        errors.append(str(error))

    return metadata, candidates, errors


def _load_catalog_entry(resource_id: str) -> dict:
    data = json.loads(CATALOG.read_text(encoding="utf-8"))
    for entry in data["resources"]:
        if entry["id"] == resource_id:
            return entry
    raise SystemExit(f"kit-fetcher: unknown resource id: {resource_id}")


def _report(url: str) -> int:
    metadata, candidates, errors = refresh(url)
    print(f"URL\t{url}")
    if metadata:
        print(
            f"  live: stars={metadata.stars} forks={metadata.forks} "
            f"open_issues={metadata.open_issues} pushed_at={metadata.pushed_at} "
            f"license={metadata.license_spdx or 'unknown'}"
        )
    if candidates:
        print(f"  candidates: {len(candidates)}")
        for c in candidates:
            print(f"    [{c.context[:60]}]")
            for line in c.command.splitlines():
                print(f"      {line}")
        if len(candidates) == 1:
            print("  -> unambiguous: usable as a fresh install command")
        else:
            print("  -> ambiguous: a human still has to pick one from the README")
    else:
        print("  candidates: 0 - a human still has to read the README")
    for error in errors:
        print(f"  ERROR\t{error}", file=sys.stderr)
    return 1 if errors and metadata is None and not candidates else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--id", help="catalog resource id to look up and refresh")
    group.add_argument("--url", help="a github.com URL to refresh directly")
    args = parser.parse_args()

    url = args.url or _load_catalog_entry(args.id)["url"]
    return _report(url)


if __name__ == "__main__":
    sys.exit(main())
