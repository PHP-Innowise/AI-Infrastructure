#!/usr/bin/env python3
"""Build the offline Open Source Kit catalog. No frontend dependencies needed."""

from __future__ import annotations

import argparse
import json
import shlex
import sys
from pathlib import Path
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parent))
import install_open_source_kit as kit

ROOT = Path(__file__).resolve().parent.parent
TEMPLATE = ROOT / "install/open-source-kit/web/index.html"
DEFAULT_OUTPUT = ROOT / "output/kit3-site"


def build_site(output: Path) -> Path:
    """Validate the source data before writing one self-contained HTML page."""
    resources = []
    for resource in kit.load_resources(kit.DEFAULT_RESOURCES):
        status = kit.registry_status(resource)  # Invalid evidence must fail the build.
        if urlsplit(resource["url"]).scheme != "https":
            raise kit.KitError(f"catalog URL must use HTTPS: {resource['id']}")
        dossier = None
        if status is not None:
            dossier = json.loads(
                (kit.REGISTRY_DIR / f"{resource['id']}.json").read_text(encoding="utf-8")
            )
        source = resource.get("skills_source")
        if source is not None and (not isinstance(source, str) or not source.strip()):
            raise kit.KitError(f"skills_source must be nonempty text: {resource['id']}")
        command = (
            f"./kit3 add {shlex.quote(source)} --agent codex"
            if source else f"./kit3 show {shlex.quote(resource['id'])}"
        )
        resources.append({
            **resource,
            "review_status": status,
            "dossier": dossier,
            "command": command,
            "install_guidance": kit.guidance_for(resource),
        })

    data = json.dumps(resources, ensure_ascii=True, separators=(",", ":"))
    # JSON is embedded in an HTML raw-text element, not an ordinary JS string.
    data = data.replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
    template = TEMPLATE.read_text(encoding="utf-8")
    if template.count("__KIT3_DATA__") != 1:
        raise kit.KitError("catalog template must contain exactly one data placeholder")
    html = template.replace("__KIT3_DATA__", data)
    output.mkdir(parents=True, exist_ok=True)
    path = output / "index.html"
    path.write_text(html, encoding="utf-8")
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    try:
        print(build_site(args.output.expanduser().resolve()))
        return 0
    except (kit.KitError, OSError, ValueError) as error:
        print(f"build-kit3-catalog: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
