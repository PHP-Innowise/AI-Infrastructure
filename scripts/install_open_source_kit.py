#!/usr/bin/env python3
"""Select curated open-source Claude Code resources (Kit 3) and record the choice.

This never downloads or executes third-party code. It lists the reviewed
catalog (`install/open-source-kit/resources.json`), lets the caller pick only
the resources a project actually needs, prints tool-appropriate install
guidance for each, and writes/updates a `.kit3-manifest.json` in the target
project so the selection has the same kind of audit trail Kit 1 already gets
from `.infra-manifest.json`.
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KIT_DIR = ROOT / "install" / "open-source-kit"
DEFAULT_RESOURCES = KIT_DIR / "resources.json"
REGISTRY_DIR = KIT_DIR / "registry"
MANIFEST_NAME = ".kit3-manifest.json"

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kit_fetcher  # noqa: E402 - needs sys.path set first

INSTALL_GUIDANCE = {
    "claude-marketplace": (
        "Inside a Claude Code session: `/plugin marketplace add {owner_repo}`, "
        "then `/plugin install <name>@<marketplace-slug>` - read the repo's own "
        "README first for the exact plugin name(s) it registers."
    ),
    "mcp-server": (
        "Register through Claude Code's MCP config (`claude mcp add ...` or a "
        "manual `.mcp.json` entry) - the repo README names the exact server(s); "
        "enable only the ones actually needed."
    ),
    "reference-clone": (
        "`git clone --depth 1 {url} <local-path>` and copy in only the specific "
        "files reviewed and wanted - do not bulk-copy the whole tree."
    ),
    "npx-cli": (
        "Run via `npx` - the repo README names the current package and flags. "
        "Review every file it proposes to write before accepting."
    ),
    "discovery-index": (
        "This is a curated list, not an installable resource. Browse it, then "
        "vet whatever specific resource is found there with this same "
        "checklist before adding it."
    ),
}


class KitError(Exception):
    """Raised when the resource catalog, a selection, or a pin is invalid."""


def load_resources(path: Path) -> list[dict]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise KitError(f"resource catalog unreadable: {error}") from error
    resources = data.get("resources")
    if not isinstance(resources, list) or not resources:
        raise KitError(f"resource catalog has no resources: {path}")
    return resources


def guidance_for(resource: dict) -> str:
    if resource.get("verified_command"):
        return resource["verified_command"]
    template = INSTALL_GUIDANCE.get(resource["install_type"])
    if template is None:
        raise KitError(f"unknown install_type: {resource['install_type']}")
    owner_repo = resource["url"].rstrip("/").split("github.com/", 1)[-1]
    return template.format(owner_repo=owner_repo, url=resource["url"])


def refresh_selected(selected: list[dict]) -> dict[str, tuple]:
    """Fetch current repo metadata + README for each pick, once each.

    Discovery-index entries are skipped - "install" is not a concept for a
    curated list. A network failure for one resource does not stop the
    others; `kit_fetcher.refresh` already guarantees that per entry.
    """
    results: dict[str, tuple] = {}
    for entry in selected:
        if entry["category"] == "discovery-index":
            continue
        results[entry["id"]] = kit_fetcher.refresh(entry["url"])
    return results


def resolve_guidance(entry: dict, refresh_result: tuple | None) -> tuple[str, str, list[str]]:
    """Decide install guidance for one entry, and say where it came from.

    Returns (guidance_text, source, notes). `source` is one of `live_fetch`,
    `verified_command`, `generic_template` - written into the manifest so the
    audit trail states how current the recorded command actually is, not just
    that some text was recorded.

    A fresh fetch is used only when it resolves to exactly one unambiguous
    candidate. Zero or several candidates fall back to whatever was already
    trusted, because picking among several would present a guess as a fact -
    the one thing this whole registry exists to refuse to do.
    """
    notes: list[str] = []
    if refresh_result is not None:
        metadata, candidates, errors = refresh_result
        for error in errors:
            notes.append(f"refresh error: {error}")
        if len(candidates) == 1:
            notes.append("refresh found 1 unambiguous candidate in the current README")
            return candidates[0].command, "live_fetch", notes
        if len(candidates) > 1:
            notes.append(
                f"refresh found {len(candidates)} candidate commands - ambiguous, "
                "keeping the stored guidance instead of guessing"
            )
        elif not errors:
            notes.append("refresh found 0 install-command candidates in the current README")

    source = "verified_command" if entry.get("verified_command") else "generic_template"
    return guidance_for(entry), source, notes


def print_catalog(resources: list[dict]) -> None:
    by_category: dict[str, list[dict]] = {}
    for entry in resources:
        by_category.setdefault(entry["category"], []).append(entry)
    for category in sorted(by_category):
        print(f"\n# {category}")
        for entry in by_category[category]:
            tools = ",".join(entry.get("tool_compatibility") or []) or "n/a"
            print(f"  {entry['id']:<28} [{tools}] {entry['name']}")
            print(f"    {entry['description']}")


def prompt_selection(resources: list[dict]) -> list[str]:
    print("Select resources to add to this target (Kit 3 - install only what is needed).\n")
    for index, entry in enumerate(resources, start=1):
        print(f"  [{index:>2}] {entry['id']:<28} ({entry['category']}) {entry['name']}")
    print("\nEnter numbers separated by commas (e.g. 1,4,7), 'all', or empty to cancel.")
    raw = input("> ").strip()
    if not raw:
        return []
    if raw.lower() == "all":
        return [entry["id"] for entry in resources]
    ids = []
    for token in raw.split(","):
        token = token.strip()
        if not token:
            continue
        try:
            position = int(token)
        except ValueError as error:
            raise KitError(f"not a number: {token}") from error
        if not 1 <= position <= len(resources):
            raise KitError(f"out of range: {token}")
        ids.append(resources[position - 1]["id"])
    return ids


def resolve_selection(resources: list[dict], ids: list[str]) -> list[dict]:
    by_id = {entry["id"]: entry for entry in resources}
    resolved = []
    for resource_id in ids:
        if resource_id not in by_id:
            raise KitError(f"unknown resource id: {resource_id}")
        resolved.append(by_id[resource_id])
    return resolved


def registry_status(resource_id: str) -> str | None:
    """What the registry found for a catalog id, or None when unreviewed.

    The registry describes; it never refuses. This script installs nothing
    either way, so refusing would only block writing the choice down - and an
    install that happened anyway would then be missing from the audit trail.
    """
    path = REGISTRY_DIR / f"{resource_id}.json"
    if not path.is_file():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8")).get("status")
    except (OSError, json.JSONDecodeError):
        return None


def review_note(resource_id: str) -> str | None:
    """One line naming what review found, so a pick is made with eyes open."""
    status = registry_status(resource_id)
    if status == "clear":
        return None
    reference = f"install/open-source-kit/registry/{resource_id}.json"
    if status is None:
        return "NOT REVIEWED - no registry entry; nothing checked this against the twelve gates"
    if status == "known_risks":
        return f"KNOWN RISKS recorded - read {reference} before installing"
    return f"OPEN QUESTIONS remain - read {reference} before installing"


def load_manifest(path: Path) -> dict:
    if not path.exists():
        return {"schema_version": 1, "kit": "open-source-kit", "entries": {}}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise KitError(f"existing manifest unreadable: {error}") from error
    data.setdefault("entries", {})
    return data


def parse_pins(values: list[str]) -> dict[str, str]:
    pins: dict[str, str] = {}
    for value in values:
        if "=" not in value:
            raise KitError(f"--pin must be ID=REF, got: {value}")
        resource_id, ref = value.split("=", 1)
        pins[resource_id.strip()] = ref.strip()
    return pins


def apply_selection(
    manifest: dict,
    selected: list[dict],
    pins: dict[str, str],
    today: str,
    refresh_results: dict[str, tuple] | None = None,
) -> dict:
    refresh_results = refresh_results or {}
    for entry in selected:
        existing = manifest["entries"].get(entry["id"], {})
        # A pin already in the manifest survives a later selection that does not
        # carry one. The catalog's own pinned_ref is null for every entry, so
        # falling back to it would silently erase the recorded ref - destroying
        # the audit trail this manifest exists to be, and doing it during the
        # documented workflow (pin after install, select again later).
        pinned_ref = pins.get(entry["id"])
        if pinned_ref is None:
            pinned_ref = existing.get("pinned_ref") or entry.get("pinned_ref")
        # The manifest is committed to the client project as the audit trail, so
        # it records what review found - not only what was picked. A warning
        # printed to a terminal survives nothing; this is reviewable in a diff.
        status = registry_status(entry["id"])
        guidance, guidance_source, _notes = resolve_guidance(
            entry, refresh_results.get(entry["id"])
        )
        manifest["entries"][entry["id"]] = {
            "name": entry["name"],
            "url": entry["url"],
            "category": entry["category"],
            "license": entry.get("license"),
            "reviewed_date": entry.get("reviewed_date"),
            "selected_date": today,
            "pinned_ref": pinned_ref,
            # `guidance`, not `command`: this is what the tool proposed, not a
            # record of what a human actually ran. How something was installed
            # is part of its risk - `curl | bash` is not `git clone` - so the
            # manifest has to answer "how", not only "what". `guidance_source`
            # says whether that answer was checked against the README on this
            # run (`live_fetch`), taken from a prior manual check
            # (`verified_command`), or is the generic install_type template.
            "install_method": entry["install_type"],
            "install_guidance": guidance,
            "install_guidance_source": guidance_source,
            "install_guidance_fetched_at": today if guidance_source == "live_fetch" else None,
            "risk_notes": entry.get("risk_notes"),
            "review": {"status": status, "reviewed": status is not None},
        }
    return manifest


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--resources", type=Path, default=DEFAULT_RESOURCES)
    parser.add_argument("--list", action="store_true", help="print the catalog and exit")
    parser.add_argument("--select", help="comma-separated resource ids, or 'all'")
    parser.add_argument("--target", type=Path, help="target project root for the manifest")
    parser.add_argument(
        "--pin", action="append", default=[], help="record the ref actually installed: ID=REF, repeatable"
    )
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument(
        "--refresh",
        action="store_true",
        help=(
            "before recording, fetch each pick's current README from GitHub and "
            "use its install command if exactly one unambiguous candidate is "
            "found; otherwise keep the stored guidance. Requires network access; "
            "unauthenticated GitHub API calls are rate-limited to 60/hour. Opt-in "
            "because it makes the run network-dependent and non-deterministic - "
            "off by default so browsing, testing, and offline/CI use stay exactly "
            "as fast and reliable as recording a selection always has been."
        ),
    )
    args = parser.parse_args(argv)
    if not args.list and args.target is None:
        parser.error("--target is required unless --list is given")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        resources = load_resources(args.resources.resolve())

        if args.list:
            print_catalog(resources)
            return 0

        if args.select:
            if args.select == "all":
                ids = [entry["id"] for entry in resources]
            else:
                ids = [token.strip() for token in args.select.split(",") if token.strip()]
        else:
            ids = prompt_selection(resources)

        if not ids:
            print("SELECTED\t0 resources; nothing to do")
            return 0

        selected = resolve_selection(resources, ids)
        pins = parse_pins(args.pin)

        # A pin for something not being selected is a typo, and silently
        # dropping it leaves the operator believing a ref was recorded.
        unmatched = sorted(set(pins) - {entry["id"] for entry in selected})
        if unmatched:
            raise KitError(
                f"--pin names id(s) not in this selection: {', '.join(unmatched)}"
            )

        target = args.target.expanduser().resolve()
        # Checked before any selection output is printed, so a bad path fails
        # with one line instead of a traceback after lines that read as success.
        if not target.is_dir():
            raise KitError(f"--target is not an existing directory: {target}")
        manifest_path = target / MANIFEST_NAME
        manifest = load_manifest(manifest_path)
        today = date.today().isoformat()

        refresh_results: dict[str, tuple] = {}
        if args.refresh:
            print(f"REFRESHING\t{len(selected)} resource(s) against their current README...\n")
            refresh_results = refresh_selected(selected)

        manifest = apply_selection(manifest, selected, pins, today, refresh_results)

        for entry in selected:
            action = "WOULD_SELECT" if args.dry_run else "SELECTED"
            print(f"{action}\t{entry['id']}\t{entry['name']}")
            print(f"  url:  {entry['url']}")
            guidance, guidance_source, guidance_notes = resolve_guidance(
                entry, refresh_results.get(entry["id"])
            )
            source_tag = f" [{guidance_source}]" if args.refresh else ""
            print(f"  how:  {guidance}{source_tag}")
            for guidance_note in guidance_notes:
                print(f"  ~     {guidance_note}")
            print(f"  risk: {entry.get('risk_notes', 'n/a')}")
            note = review_note(entry["id"])
            if note:
                print(f"  !!    {note}")
            if not pins.get(entry["id"]) and not entry.get("pinned_ref"):
                print(
                    "  !     not pinned yet - once installed, record the exact ref with "
                    f"`--pin {entry['id']}=<ref>`"
                )

        if args.dry_run:
            print(f"\nWOULD_WRITE\t{manifest_path}")
        else:
            manifest_path.write_text(
                json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
            )
            print(f"\nWROTE\t{manifest_path}")
        return 0
    except KitError as error:
        print(f"install-open-source-kit: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
