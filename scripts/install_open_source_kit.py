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


def registry_verdict(resource_id: str) -> str | None:
    """The admission verdict for a catalog id, or None when unreviewed.

    Advisory only: this prints a warning, it does not refuse. Enforcement is
    deliberately not wired yet - see the README's "Not built yet" section.
    """
    path = REGISTRY_DIR / f"{resource_id}.json"
    if not path.is_file():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8")).get("verdict")
    except (OSError, json.JSONDecodeError):
        return None


def admission_warning(resource_id: str) -> str | None:
    """A one-line warning when a pick is not an approved candidate."""
    verdict = registry_verdict(resource_id)
    if verdict == "approved":
        return None
    if verdict is None:
        return (
            "NOT REVIEWED - no registry entry; nothing has judged this against "
            "the twelve gates"
        )
    return (
        f"admission verdict is {verdict.upper()} - see "
        f"install/open-source-kit/registry/{resource_id}.json"
    )


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
    manifest: dict, selected: list[dict], pins: dict[str, str], today: str
) -> dict:
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
        manifest["entries"][entry["id"]] = {
            "name": entry["name"],
            "url": entry["url"],
            "category": entry["category"],
            "license": entry.get("license"),
            "reviewed_date": entry.get("reviewed_date"),
            "selected_date": today,
            "pinned_ref": pinned_ref,
        }
    return manifest


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--resources", type=Path, default=DEFAULT_RESOURCES)
    parser.add_argument("--list", action="store_true", help="print the catalog and exit")
    parser.add_argument("--select", help="comma-separated resource ids, or 'all'")
    parser.add_argument("--target", type=Path, help="target project root for the manifest")
    parser.add_argument(
        "--pin", action="append", default=[], help="record the ref actually installed: ID=REF, repeatable"
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if not args.list and args.target is None:
        parser.error("--target is required unless --list is given")
    return args


def main() -> int:
    args = parse_args()
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
        manifest = apply_selection(manifest, selected, pins, today)

        for entry in selected:
            action = "WOULD_SELECT" if args.dry_run else "SELECTED"
            print(f"{action}\t{entry['id']}\t{entry['name']}")
            print(f"  url:  {entry['url']}")
            print(f"  how:  {guidance_for(entry)}")
            print(f"  risk: {entry.get('risk_notes', 'n/a')}")
            warning = admission_warning(entry["id"])
            if warning:
                print(f"  !!    {warning}")
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
