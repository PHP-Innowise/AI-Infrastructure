"""Fresh bounded Lark FIFO control; ROOT temp helper, not actor readiness."""
import json
import os
import subprocess
import sys
import unittest
import tests.test_memory_final_source_image as wren


class FinalTaskFifoTests(wren.FinalSourceImageTests):
    def test_final_task_fifo_is_withheld_without_open_block(self):
        if not hasattr(os, 'mkfifo'):
            self.skipTest('This single POSIX FIFO prerequisite is unavailable')
        self.task()
        child = '''
import json,os,sys
from pathlib import Path
root=Path(sys.argv[1]);sys.path.insert(0,str(root/'memory-bank/scripts'))
import context,context_retrieval
connection=context.connect(root/'memory-bank/local/context.db')
task_file,record,body=context_retrieval.find_task(root,sys.argv[2])
print('LARK-FIFO-READY628',flush=True)
def pack(value):
    task_file.unlink();os.mkfifo(task_file,0o600);return value
result=context_retrieval.retrieve(connection,root,sys.argv[3],sys.argv[2],limit=5,
    gate_mode='off',manifest_scope='local',pack=pack)
print(json.dumps({'working':result['working'],'paths':sorted(d['path'] for d in result['selected'])}),flush=True)
'''
        try:
            result = subprocess.run([sys.executable, '-B', '-c', child, str(self.root),
                wren.PUBLIC['task'], wren.PUBLIC['query']], cwd=self.root, env=self.env,
                capture_output=True, timeout=2.0)
        except subprocess.TimeoutExpired as error:
            self.observe('lark-fifo-child', {'state':'TIMEOUT_CENSORED','limit_seconds':2.0,
                'stdout':(error.stdout or b'').decode('utf-8'),'stderr':(error.stderr or b'').decode('utf-8')})
            self.fail('Final task FIFO blocked the started helper at the fixed2s control bound')
        self.observe('lark-fifo-child', {'state':'COMPLETED','returncode':result.returncode,
            'stdout':result.stdout.decode('utf-8'),'stderr':result.stderr.decode('utf-8')})
        self.assertEqual(0,result.returncode,result.stderr)
        self.assertEqual(b'',result.stderr)
        lines=result.stdout.decode('utf-8').splitlines()
        self.assertEqual('LARK-FIFO-READY628',lines[0])
        self.assertEqual({'working':None,'paths':sorted([wren.PUBLIC['primary'],wren.PUBLIC['survivor']])},json.loads(lines[1]))
        self.assertEqual(2,len(lines))


for _name in dir(wren.FinalSourceImageTests):
    if _name.startswith('test_'):
        setattr(FinalTaskFifoTests,_name,None)
if __name__=='__main__':unittest.main()
