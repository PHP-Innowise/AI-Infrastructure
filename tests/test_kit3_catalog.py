"""Checks for the offline web catalog's data boundary."""

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import build_kit3_catalog as catalog


class CatalogBuildTests(unittest.TestCase):
    def test_build_contains_all_resources_and_escapes_html_data(self):
        resources = catalog.kit.load_resources(catalog.kit.DEFAULT_RESOURCES)
        resources[0]["description"] = '</script><script>alert("catalog")</script> & < >'
        resources[0]["skills_source"] = "owner/skills"
        with tempfile.TemporaryDirectory() as directory, patch.object(
            catalog.kit, "load_resources", return_value=resources
        ):
            path = catalog.build_site(Path(directory))
            html = path.read_text(encoding="utf-8")
        self.assertNotIn('__KIT3_DATA__', html)
        self.assertNotIn('</script><script>alert(', html)
        raw = html.split('<script id="kit3-data" type="application/json">', 1)[1].split('</script>', 1)[0]
        data = json.loads(raw)
        self.assertEqual(len(data), len(resources))
        self.assertEqual(data[0]["description"], resources[0]["description"])
        self.assertEqual(data[0]["command"], "./kit3 add owner/skills --agent codex")
        manual = next(item for item in data if not item.get("skills_source"))
        self.assertEqual(manual["command"], f"./kit3 show {manual['id']}")
        self.assertIn('review_status', data[0])

    def test_invalid_registry_fails_before_any_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            registry = root / "registry"
            registry.mkdir()
            entry = json.loads((catalog.kit.REGISTRY_DIR / "graphify.json").read_text())
            entry["status"] = "clear"
            (registry / "graphify.json").write_text(json.dumps(entry))
            output = root / "site"
            with patch.object(catalog.kit, "REGISTRY_DIR", registry):
                with self.assertRaises(catalog.kit.KitError):
                    catalog.build_site(output)
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
