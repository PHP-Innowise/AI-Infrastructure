"""The export screen's second review: credential forms it let through, and
ordinary PHP, JavaScript and prose it refused. Synthetic values only; no
provider or service dispatch."""
import hashlib
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import hindsight_export as he
from harness.hindsight_recall import Scope

SECRET = 'craneSynthetic9'
# Each form once let the whole value into the export packet.
LEAKED = {
    'camel_case': f'$dbPassword = "{SECRET}";',
    'camel_case_json': f'{{"clientSecret": "{SECRET}"}}',
    'bare_token': f'TOKEN={SECRET}',
    'api_token': f'API_TOKEN={SECRET}',
    'db_pass': f'DB_PASS={SECRET}',
    'yaml_pass': f'pass: {SECRET}',
    'php_pwd': f'$pwd = "{SECRET}";',
    'env_default': f"'password' => env('DB_PASSWORD', '{SECRET}'),",
    'getenv_default': f"token = os.getenv('API_TOKEN', '{SECRET}')",
    'identified_by': f"CREATE USER 'crane'@'%' IDENTIFIED BY '{SECRET}';",
    'postgres_password': f"CREATE ROLE crane LOGIN PASSWORD '{SECRET}';",
    'salt': f"'salt' => '{SECRET}',",
    'raw_app_key': 'APP_KEY=craneSyntheticApplicationKey',
    'encryption_key': f'encryption_key: {SECRET}',
    'passphrase': 'passphrase = "crane synthetic nine"',
    'code_point_key': f'{{"\\u{{70}}assword": "{SECRET}"}}',
}
# Ordinary public text that must keep passing.
KEPT = {
    'prose': 'Each crane lift requires two operators.',
    'empty_env_default': "'password' => env('DB_PASSWORD', ''),",
    'runtime_reference': "'password' => getenv('CRANE_PASSWORD')",
    'yaml_reference': 'password: ${CRANE_PASSWORD}',
    'shell_directory': 'cd "$PWD" && PWD=/srv/crane make',
    'token_prose': 'Set the token: see below.',
    'validation_rule': 'password: required',
    'tokenizer': 'tokenizer = build_tokenizer()',
    'placeholder_sql': "CREATE USER 'crane'@'%' IDENTIFIED BY 'password';",
}


class ExportScreenGapTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='crane-export-gaps-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.scope = Scope('7d8f2c1e-4b6a-4f3e-9c2d-1a5b8e7f6c40', '2e9c4a7b-1f3d-4c8e-a6b5-9d0e3f2a1c87')

    def export(self, path, text):
        source = self.root / path
        source.parent.mkdir(parents=True, exist_ok=True)
        raw = text.encode('utf-8')
        source.write_bytes(raw)
        approval = he.PublicApproval(self.scope, path, hashlib.sha256(raw).hexdigest())
        return he.build_public_export(self.root, self.scope, (approval,))

    def refused(self, path, text):
        with self.assertRaises(he.ExportError) as caught:
            self.export(path, text)
        return caught.exception.reason

    def test_credential_forms_refuse_the_capture(self):
        for name, text in LEAKED.items():
            for path in ('docs/crane.md', 'src/crane.php'):
                with self.subTest(name=name, path=path):
                    self.assertEqual('private_content', self.refused(path, text + '\n'))

    def test_ordinary_text_still_exports(self):
        for name, text in KEPT.items():
            with self.subTest(name=name):
                self.export('docs/crane.md', text + '\n')

    def test_code_point_escapes_are_read_not_refused(self):
        self.export('src/crane.js', 'const smile = "\\u{1F600}";\n')
        self.export('src/crane.php', '<?php $smile = "\\u{1F600}";\n')
        self.assertEqual('source_text', self.refused('src/crane.js', 'const bad = "\\u{110000}";\n'))

    def test_php_namespaces_and_prose_paths_keep_a_literal_backslash_u(self):
        self.export('src/crane.php', '<?php $class = "App\\utils\\Crane";\n')
        self.export('docs/crane.md', 'Logs go to "C:\\users\\crane\\logs".\n')
        # Where `\u` is always an escape, a broken one still refuses.
        self.assertEqual('source_text', self.refused('config/crane.json', '{"path": "App\\utils"}\n'))
        self.assertEqual('source_text', self.refused('src/crane.js', 'const path = "App\\utils";\n'))

    def test_an_unmatched_quote_does_not_pair_with_the_next_line(self):
        text = 'Use "\\n to break a line.\nThe "crane" lifts.\n'
        self.export('docs/crane.md', text)
        self.export('src/crane.py', text)
        # An unmatched quote at the very end of prose is prose too...
        self.export('docs/crane.md', 'Ends with "a \\n quote')
        # ...but not in a JSON-like source, where it stays unfinished text.
        self.assertEqual('source_text', self.refused('config/crane.json', '{"a": "b \\n'))


if __name__ == '__main__':
    unittest.main()
