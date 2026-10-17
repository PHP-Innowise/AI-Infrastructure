"""Fresh compiler-only controls: never import old cases or their data fixtures."""
import __future__
import ast
import hashlib
import json
from pathlib import Path
import types
import unittest

ROOT=Path(__file__).resolve().parents[1]
PUBLIC=json.loads((ROOT/'tests/fixtures/longmemeval_compile_portability_public.json').read_text(encoding='utf-8'))


class PinnedApiCompilerTests(unittest.TestCase):
    def check_caller(self,index):
        caller,filename=PUBLIC['callers'][index]
        tree=ast.parse((ROOT/caller).read_text(encoding='utf-8'))
        [call]=[node for node in ast.walk(tree) if isinstance(node,ast.Call)
            and isinstance(node.func,ast.Name) and node.func.id=='compile'
            and len(node.args)==3 and isinstance(node.args[1],ast.Constant) and node.args[1].value==filename]
        raw=(ROOT/'tests/fixtures/longmemeval_memory_api_2cc8c540.py').read_bytes()
        self.assertEqual(PUBLIC['whole_sha256'],hashlib.sha256(raw).hexdigest())
        prefix=raw[:PUBLIC['prefix_bytes']]
        self.assertEqual(PUBLIC['prefix_sha256'],hashlib.sha256(prefix).hexdigest())
        # Only the compile expression is evaluated. None of the surrounding
        # caller module or its PUBLIC/EXPECTED readers executes.
        expression=ast.Expression(body=call);ast.fix_missing_locations(expression)
        compiled=eval(compile(expression,'new-kestrel-caller-compile','eval',dont_inherit=True),
            {'compile':compile,'__future__':__future__,'body':prefix,'prefix':prefix})
        module=types.ModuleType('kestrel_compiler_only')
        exec(compiled,module.__dict__)
        self.assertEqual({},module.MEMORY_TYPES)
        self.assertEqual(PUBLIC['deferred_annotation'],module.Memory.query.__annotations__['query_image'])
        self.assertEqual(PUBLIC['prefix_sha256'],hashlib.sha256(prefix).hexdigest())

    def test_bridge_compiles_same_source_with_explicit_deferred_annotations(self):
        self.check_caller(0)

    def test_lifecycle_compiles_same_source_with_explicit_deferred_annotations(self):
        self.check_caller(1)


if __name__=='__main__':unittest.main()
