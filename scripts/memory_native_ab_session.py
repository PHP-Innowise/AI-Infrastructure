"""Bounded native event transport; no model invocation on import."""
from __future__ import annotations

import json
import math
import os
from pathlib import Path
from contextlib import ExitStack
import selectors
import signal
import subprocess
import time

from memory_native_ab_io import PilotError, _json

TOOLS = {'command_execution','mcp_tool_call','web_search','file_change','collab_tool_call'}


def run_process(command, cwd, events_path, *, timeout, cap, env=None, max_bytes=8*1024*1024, native=True, stderr_path=None):
    if (isinstance(timeout,bool) or not isinstance(timeout,(int,float)) or not 0 < timeout <= 900 or not math.isfinite(timeout) or type(cap) is not int or not 1 <= cap <= 100):
        raise PilotError('invalid process budget')
    if type(max_bytes) is not int or not 1 <= max_bytes <= 8*1024*1024:
        raise PilotError('invalid output budget')
    started = time.monotonic();deadline = started + timeout
    result = {'stopped':'finished','completed':False,'calls':0,'known_usage':{'input_tokens':0,'output_tokens':0},
              'usage_complete':True,'usage_events':[],'sessions':[],'turns':0,'answer':''}
    completed, failed, turn, seen, total, pending = False,False,0,set(),0,bytearray()
    process = None
    fd = os.open(events_path,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
    try:
        with os.fdopen(fd,'wb') as log, selectors.DefaultSelector() as selector, ExitStack() as stack:
            errors = None
            if stderr_path is not None:
                errfd=os.open(stderr_path,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600)
                errors=stack.enter_context(os.fdopen(errfd,'wb'))
            process = subprocess.Popen(command,cwd=cwd,env=env,stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,stderr=subprocess.PIPE,bufsize=0,start_new_session=True)
            for stream in (process.stdout,process.stderr):
                os.set_blocking(stream.fileno(),False);selector.register(stream,selectors.EVENT_READ)
            while selector.get_map() or process.poll() is None:
                left=deadline-time.monotonic()
                if left<=0:result['stopped']='timeout';break
                for key,_ in selector.select(min(left,.05)):
                    block=os.read(key.fileobj.fileno(),min(65536,max_bytes-total+1))
                    if not block:selector.unregister(key.fileobj);continue
                    total+=len(block)
                    if total>max_bytes:result['stopped']='output-limit';break
                    if key.fileobj is process.stderr:
                        if errors is not None:errors.write(block)
                        continue
                    log.write(block)
                    if not native:continue
                    pending.extend(block)
                    if len(pending)>1024*1024:result['stopped']='output-limit';break
                    while b'\n' in pending:
                        line,_,rest=pending.partition(b'\n');pending=bytearray(rest)
                        try:event=_json(line)
                        except (ValueError,UnicodeError,RecursionError):result['stopped']='invalid-events';break
                        if not isinstance(event,dict):result['stopped']='invalid-events';break
                        kind=event.get('type');item=event.get('item',{})
                        if kind=='thread.started':
                            sid=event.get('thread_id')
                            if isinstance(sid,str) and sid not in result['sessions']:result['sessions'].append(sid)
                        if kind=='turn.started':
                            turn+=1;completed=False;seen=set();result['answer']=''
                        if kind=='turn.failed':failed=True;result['stopped']='native-failed';break
                        if kind in ('item.started','item.completed') and (not isinstance(item,dict) or completed):
                            result['stopped']='invalid-events';break
                        if kind in ('item.started','item.completed'):
                            if item.get('type') in TOOLS:
                                identity=item.get('id')
                                if not isinstance(identity,str) or not identity:result['stopped']='invalid-events';break
                                if identity not in seen:seen.add(identity);result['calls']+=1
                                if result['calls']>=cap:result['stopped']='cap';break
                            if kind=='item.completed' and item.get('type')=='agent_message' and isinstance(item.get('text'),str):
                                if len(item['text'].encode())>65536:
                                    result['stopped']='output-limit';break
                                result['answer']=item['text']
                        if kind=='turn.completed':
                            if completed:result['stopped']='invalid-events';break
                            completed=True;result['turns']+=1
                            usage=event.get('usage')
                            safe={k:v for k,v in (usage.items() if isinstance(usage,dict) else [])
                                  if k in ('input_tokens','output_tokens','cached_input_tokens',
                                           'cache_write_input_tokens','reasoning_output_tokens')
                                  and type(v) is int and 0<=v<=2**53-1}
                            result['usage_events'].append(safe)
                            if any(k not in safe for k in result['known_usage']):
                                result['usage_complete']=False
                            for k in result['known_usage']:
                                result['known_usage'][k]+=safe.get(k,0)
                        if result['turns']>16 or len(result['sessions'])>2:result['stopped']='output-limit';break
                    if result['stopped']!='finished':break
                if result['stopped']!='finished':break
            if pending and result['stopped']=='finished':result['stopped']='invalid-events'
            if process.poll() is None and result['stopped']=='finished':result['stopped']='timeout'
    finally:
        if process is not None:
            # Signal the group even after its leader exits: inherited FDs are not a lifecycle proof.
            try:os.killpg(process.pid,signal.SIGKILL)
            except ProcessLookupError:pass
            try:process.wait(timeout=1)
            finally:
                process.stdout.close();process.stderr.close()
    result['exit']=process.returncode if process is not None else None
    result['completed']=bool(result['stopped']=='finished' and result['exit']==0 and (completed or not native) and not failed)
    result['seconds']=round(time.monotonic()-started,3)
    if not result['completed']:result['usage_complete']=False
    return result
