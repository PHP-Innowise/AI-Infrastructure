#!/usr/bin/env python3
"""Browse Kit 3 and manage Agent Skills with the pinned Vercel Skills CLI."""
from __future__ import annotations

import argparse
import functools
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from urllib.parse import unquote, urlsplit

if __package__:
    from . import install_open_source_kit as catalog
else:
    import install_open_source_kit as catalog

SKILLS_PACKAGE = "skills@1.5.23"
AGENTS = ("claude-code", "codex", "cursor")


def search_resources(resources, query="", agent=None, category=None, review=None):
    terms = query.casefold().split()
    found = []
    for item in resources:
        if agent and agent not in item.get("tool_compatibility", []):
            continue
        if category and item["category"] != category:
            continue
        text = " ".join(str(item.get(key, "")) for key in ("id", "name", "description", "category"))
        if not all(term in text.casefold() for term in terms):
            continue
        status = catalog.registry_status(item) or "unreviewed"
        if review and review != status:
            continue
        found.append({**item, "review_status": status})
    return found


def safe_argument(value):
    if not value.strip() or value.startswith("-") or any(ord(c) < 32 for c in value):
        raise catalog.KitError("arguments must be nonempty names, not options or control characters")
    return value


def resolve_source(value, resources):
    """Resolve catalog shortcuts; never interpret prose install commands as code."""
    safe_argument(value)
    item = next((entry for entry in resources if entry["id"] == value), None)
    if item:
        catalog.registry_status(item)
        if not item.get("skills_source"):
            raise catalog.KitError(
                f"{value} has no Agent Skills install path; use `kit3 show {value}` for its manual guide"
            )
        value = item["skills_source"]
    safe_argument(value)
    if value.startswith(("./", "../", "/", "~/")):
        local = Path(value).expanduser()
        if local.is_dir():
            return str(local.resolve())
        raise catalog.KitError(f"local source is not a directory: {value}")
    if re.fullmatch(r"[A-Za-z0-9_][\w.-]*/[A-Za-z0-9_][\w.-]*", value, re.ASCII):
        return value
    parsed = urlsplit(value)
    if parsed.scheme == "https" and parsed.hostname and not parsed.username and not parsed.password:
        return value
    if parsed.scheme == "file" and not parsed.netloc and Path(unquote(parsed.path)).is_dir():
        return value
    raise catalog.KitError("source must be a supported catalog ID, owner/repo, HTTPS URL, local Git file URI or local skill directory")


def native_command(args, resources):
    command = ["npx", "--yes", SKILLS_PACKAGE, args.command]
    if args.command == "find":
        if args.query:
            command.append(safe_argument(args.query))
        return command
    if args.command == "add":
        command.append(resolve_source(args.source, resources))
        for skill in args.skill:
            command.extend(["--skill", safe_argument(skill)])
        if args.list:
            command.append("--list")
        if args.copy:
            command.append("--copy")
    elif args.command in ("remove", "update"):
        command.extend(safe_argument(name) for name in args.names)
    if args.command == "list" and args.json:
        command.append("--json")
    for agent in getattr(args, "agent", []) or []:
        command.extend(["--agent", agent])
    if getattr(args, "global_scope", False):
        command.append("--global")
    elif args.command == "update":
        command.append("--project")
    if getattr(args, "yes", False):
        command.append("--yes")
    return command


def run_native(args, resources):
    target = args.target.expanduser().resolve()
    if not target.is_dir():
        raise catalog.KitError(f"target is not a directory: {target}")
    command = native_command(args, resources)
    print(f"{'WOULD_RUN' if args.dry_run else 'RUN'}\t{shlex.join(command)}", file=sys.stderr, flush=True)
    print(f"TARGET\t{target}", file=sys.stderr, flush=True)
    if args.dry_run:
        return 0
    if not shutil.which("npx"):
        raise catalog.KitError("Node.js with npx is required for skill management; catalog browsing works without it")
    # The external manager owns installation state, its prompts and collision policy.
    # No README text or catalog command is passed to a shell.
    return subprocess.run(command, cwd=target, check=False).returncode


def parser():
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    find = commands.add_parser("find", help="search the curated catalog; --remote searches skills.sh")
    find.add_argument("query", nargs="?", default="")
    find.add_argument("--agent", choices=AGENTS)
    find.add_argument("--category")
    find.add_argument("--review", choices=("unreviewed", "clear", "open_questions", "known_risks"))
    find.add_argument("--json", action="store_true")
    find.add_argument("--remote", action="store_true")
    show = commands.add_parser("show", help="read one resource's installation guide and review")
    show.add_argument("id")
    show.add_argument("--json", action="store_true")
    add = commands.add_parser("add", help="install Agent Skills through Vercel Skills")
    add.add_argument("source", help="catalog ID, owner/repo, HTTPS URL or local directory")
    add.add_argument("--skill", "-s", action="append", default=[])
    add.add_argument("--list", action="store_true", help="only list skills available at this source")
    add.add_argument("--copy", action="store_true")
    listing = commands.add_parser("list", help="list installed skills; --recorded reads the separate Kit 3 selection")
    listing.add_argument("--recorded", action="store_true")
    listing.add_argument("--json", action="store_true", help="machine-readable installed skills or recorded selections")
    remove = commands.add_parser("remove", help="remove installed skills using the native manager")
    remove.add_argument("names", nargs="*")
    update = commands.add_parser("update", help="update project skills, or --global")
    update.add_argument("names", nargs="*")
    record = commands.add_parser("record", help="record catalog selections and review; does not install")
    record.add_argument("ids", nargs="+")
    record.add_argument("--pin", action="append", default=[])
    for cmd in (find, add, listing, remove, update, record):
        cmd.add_argument("--target", type=Path, default=Path.cwd())
        cmd.add_argument("--dry-run", action="store_true")
    for cmd in (add, listing, remove):
        cmd.add_argument("--agent", "-a", action="append", choices=AGENTS, default=[])
    for cmd in (add, listing, remove, update):
        cmd.add_argument("--global", "-g", dest="global_scope", action="store_true")
    for cmd in (add, remove, update):
        cmd.add_argument("--yes", "-y", action="store_true", help="accept the native manager's prompts")
    build = commands.add_parser("build", help="build the static web catalog")
    build.add_argument("--output", type=Path, default=catalog.ROOT / "output" / "kit3-site")
    serve = commands.add_parser("serve", help="preview the web catalog on localhost")
    serve.add_argument("--port", type=int, default=8765)
    return root


def main(argv=None):
    cli = parser()
    args = cli.parse_args(argv)
    try:
        if args.command in ("build", "serve"):
            if __package__:
                from .build_kit3_catalog import build_site
            else:
                from build_kit3_catalog import build_site
            if args.command == "build":
                print(build_site(args.output.expanduser()))
                return 0
            if not 0 <= args.port <= 65535:
                raise catalog.KitError("port must be between 0 and 65535")
            with tempfile.TemporaryDirectory(prefix="kit3-site-") as temporary:
                build_site(Path(temporary))
                handler = functools.partial(SimpleHTTPRequestHandler, directory=temporary)
                with ThreadingHTTPServer(("127.0.0.1", args.port), handler) as server:
                    print(f"Kit 3 catalog: http://127.0.0.1:{server.server_port}", flush=True)
                    try:
                        server.serve_forever()
                    except KeyboardInterrupt:
                        pass
            return 0
        if args.command == "record":
            legacy = ["--target", str(args.target), "--select", ",".join(args.ids)]
            for pin in args.pin:
                legacy.extend(["--pin", pin])
            if args.dry_run:
                legacy.append("--dry-run")
            return catalog.main(legacy)
        if args.command == "list" and args.recorded:
            if args.agent or args.global_scope:
                raise catalog.KitError("--recorded is the project selection; --agent/--global describe native installed skills")
            target = Path(os.path.abspath(args.target.expanduser()))
            data = catalog.load_manifest(target / catalog.MANIFEST_NAME)
            if args.json:
                print(json.dumps(data, indent=2))
            elif not data["entries"]:
                print("No recorded selections.")
            else:
                for name, entry in sorted(data["entries"].items()):
                    print(f"{name}\tselected (installation not verified)\t{entry.get('pinned_ref') or 'unpinned'}")
            return 0
        resources = catalog.load_resources(catalog.DEFAULT_RESOURCES)
        if args.command == "find" and not args.remote:
            items = search_resources(resources, args.query, args.agent, args.category, args.review)
            if args.json:
                print(json.dumps(items, indent=2))
            elif items:
                for item in items:
                    print(f"{item['id']:<29} {item['review_status']:<16} {item['name']}")
                    print(f"  {item['description']}")
            else:
                print("No matches. Try a shorter query or `kit3 find QUERY --remote`.")
            return 0
        if args.command == "find" and (args.agent or args.category or args.review or args.json):
            raise catalog.KitError("local catalog filters and JSON cannot be combined with --remote")
        if args.command == "show":
            item = catalog.resolve_selection(resources, [args.id])[0]
            status = catalog.registry_status(item) or "unreviewed"
            guide = catalog.guidance_for(item)
            if args.json:
                print(json.dumps({**item, "review_status": status, "install_guidance": guide}, indent=2))
            else:
                print(f"{item['name']}\n{item['url']}\n\n{item['description']}")
                print(f"\nClients: {', '.join(item.get('tool_compatibility', []))}\nReview: {status}")
                print(f"\nInstall guide:\n{guide}\n\nRisks:\n{item.get('risk_notes', 'Not assessed')}")
                if item.get("skills_source"):
                    print(f"\nSkills-only install: ./kit3 add {shlex.quote(item['id'])}")
            return 0
        return run_native(args, resources)
    except (catalog.KitError, OSError, ValueError) as error:
        print(f"kit3: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
