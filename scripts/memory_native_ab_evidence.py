"""Conservative observed native-save consistency; unsigned evidence is not truth."""
from __future__ import annotations

import json
from pathlib import Path
import re
import shlex
import sys
from itertools import islice

from memory_native_ab_io import PilotError, read_file, read_json, sha, relative_path, _json, check_deadline


def metadata(path):
    raw=read_file(path)[0]
    text=raw.decode('utf-8').replace('\r\n','\n').replace('\r','\n')
    match=re.match(r'\A---\n(.*?)\n---(?:\n|$)',text,re.S)
    if not match:raise PilotError('record metadata unavailable')
    return _json(match.group(1).encode())


def citations(root, meta, field, deadline=None):
    declared=meta.get('sources');rows=meta.get(field)
    if not isinstance(rows,list) or not isinstance(declared,list) or not 1<=len(rows)<=16:
        return False
    names=[]
    for row in rows:
        check_deadline(deadline)
        if not isinstance(row,dict):return False
        name=relative_path(row.get('path'),memory=True).as_posix();names.append(name)
        expected=row.get('sha256')
        if not isinstance(expected,str) or not re.fullmatch('[0-9a-f]{64}',expected):return False
        data=read_file(root/name)[0]
        if b'\0' not in data[:8192]:data=data.replace(b'\r\n',b'\n')
        if sha(data)!=expected:return False
    return len(set(names))==len(names) and set(names)==set(declared)


def shell_script(command):
    if not isinstance(command,str):return None
    try:outer=shlex.split(command)
    except ValueError:return None
    if not outer:return None
    if outer[0] in ('bash','sh','/bin/bash','/usr/bin/bash','/bin/sh','/usr/bin/sh'):
        return outer[2] if len(outer)==3 and outer[1] in ('-c','-lc','-ic','-lic') else None
    return command


def canonical_args(script,root):
    if not isinstance(script,str):return None
    script=script.strip()
    grouped=script.startswith('(') and script.endswith(')')
    try:argv=shlex.split(script[1:-1] if grouped else script)
    except ValueError:return None
    if grouped:
        if argv[:3]!=['cd',str(root),'&&']:return None
        argv=argv[3:]
    if len(argv)<2 or argv[0] not in ('python3','python','/usr/bin/python3',
                                    '/bin/python3',str(Path(sys.executable).resolve())):
        return None
    if argv[1] not in ('memory-bank/scripts/context.py',str(root/'memory-bank/scripts/context.py')):
        return None
    args=argv[2:]
    if args[:2]==['--root',str(root)]:args=args[2:]
    return args


def command_args(command,root):
    return canonical_args(shell_script(command),root)


def save_call(command, root):
    """Canonical invocation with literal JSON, directly or through plain cat.

    Unsupported shell/Python encodings are unknown; never execute extracted text.
    """
    script=shell_script(command)
    if script is None:return None
    pipe=re.fullmatch(r"(?:cat|/bin/cat|/usr/bin/cat)[ \t]+<<[ \t]*'([A-Za-z0-9_]+)'"
                      r"[ \t]*\|[ \t]*(\([^\n]+\))[ \t]*\n(.*?)\n\1\s*",script,re.S)
    if pipe:
        invocation,body=pipe[2],pipe[3]
    else:
        match=re.fullmatch(r"(.+?)\s*<<\s*'([A-Za-z0-9_]+)'\n(.*?)\n\2\s*",script,re.S)
        if not match:return None
        invocation,body=match[1],match[3]
    args=canonical_args(invocation,root)
    if args is None:return None
    if len(args)!=8 or args[0]!='native-result' or args[1]!='--request' or args[3]!='--token' or args[5:]!=['--input','-','--json']:
        return None
    if not re.fullmatch('[0-9a-f]{64}',args[2]) or not re.fullmatch('[0-9a-f]{32}',args[4]):return None
    return args[2],args[4],_json(body.encode())


def capture(root, identity, session_id, events_path, before, *, deadline=None):
    result={'native_saved':False,'new_chunks':[],'record_ids':[],
            'producer_authenticated':False,'semantic_verification':'unproven'}
    try:
        check_deadline(deadline)
        import memory_outcome_ab as ab
        ab.runtime()
        import memory_results as mr
        from brain_runtime import get_task
        raw=read_file(events_path,8*1024*1024)[0]
        for line in raw.splitlines():
            check_deadline(deadline)
            event=_json(line);item=event.get('item',{})
            if (event.get('type')!='item.completed' or not isinstance(item,dict)
                    or item.get('type')!='command_execution' or type(item.get('exit_code')) is not int
                    or item['exit_code']!=0):continue
            observed=save_call(item.get('command',''),root)
            if not observed:continue
            key,token,payload=observed
            answer=_json(item.get('aggregated_output','').encode())
            state=read_json(root/'project-brain/local/native-results'/ (key+'.json'))
            project=sha(str(root.resolve()).encode())
            session_hash=sha(session_id.encode())
            expected_key=sha(json.dumps([project,state['owner'],'codex',session_hash,identity['task_id']]).encode())
            if (key!=expected_key or state.get('phase')!='saved' or state.get('host')!='codex'
                    or state.get('project')!=project or state.get('session')!=session_hash
                    or state.get('task_id')!=identity['task_id'] or state.get('task_uuid')!=identity['task_uuid']
                    or state.get('token')!=token or state.get('owner') is None):continue
            normalized=mr.normalize(root,payload,check_files=False)
            digest=mr.request_digest(identity['task_uuid'],normalized)
            if state.get('digest')!=digest or answer.get('attestation')!='agent' or answer.get('replayed') is not False:
                continue
            result_id='native-'+token
            receipt_name=sha((identity['task_uuid']+'\0'+result_id).encode())+'.json'
            receipt=read_json(root/'project-brain/local/results'/receipt_name)
            proof_name=sha((identity['task_uuid']+'\0'+result_id+'\0'+digest).encode())+'.json'
            proof=read_json(root/'project-brain/control/result-attempts'/identity['task_uuid']/proof_name)
            if (receipt.get('digest')!=digest or receipt.get('task_id')!=identity['task_uuid']
                    or receipt.get('result_id')!=result_id or proof.get('digest')!=digest
                    or proof.get('task_id')!=identity['task_uuid'] or proof.get('result_id')!=result_id
                    or proof.get('state')!='complete' or proof.get('profile')!='memory-result-proof-v1'
                    or proof.get('attestation')!='agent' or proof.get('owner')!=state['owner']):continue
            task=get_task(root,identity['task_uuid'])
            reported=answer.get('task')
            if receipt.get('task_revision') is None:
                if reported is not None:continue
            elif (not isinstance(reported,dict) or reported.get('id')!=identity['task_uuid']
                    or reported.get('revision')!=receipt['task_revision']
                    or task['revision']!=receipt['task_revision']):continue
            if normalized['progress'] is not None and task['progress']!=normalized['progress']:continue
            rows=answer.get('records')
            if not isinstance(rows,list) or len(rows)!=len(normalized['learnings']):continue
            chunks=[];ids=[]
            for row,learning in zip(rows,normalized['learnings']):
                check_deadline(deadline)
                external=mr.learning_id(identity['task_uuid'],learning)
                record_id=row['id']
                if not re.fullmatch(r'[0-9a-f-]{36}',record_id):raise PilotError('record id')
                record_path='project-brain/dynamic/'+('findings' if learning['type']=='finding' else 'decisions')+'/'+record_id+'.md'
                record=metadata(root/record_path)
                closed='resolved' if learning['type']=='finding' else 'accepted'
                if (row.get('external_id')!=external or record.get('external_id')!=external
                        or record.get('id')!=record_id or record.get('type')!=learning['type']
                        or row.get('type')!=learning['type'] or record.get('title')!=learning['title']
                        or set(record.get('sources',[]))!=set(learning['sources'])
                        or row.get('status')!=record.get('status') or row.get('revision')!=record.get('revision')
                        or row.get('state') not in ('created','updated','completed')
                        or record.get('status')!=closed or record.get('goal')!=learning['consequence']
                        or not citations(root,record,'source_fingerprints',deadline)):raise PilotError('learning mismatch')
                ids.append(record_id)
                if row['state']!='created' or record_path in before:continue
                promoted=((answer.get('promotion') or {}).get('promoted') or [])
                memory_ids={p['memory_id'] for p in promoted if p.get('record_id')==record_id}
                paths=list(islice((root/'memory-bank/chunks').glob('*.md'),2001))
                if len(paths)>2000:raise PilotError('chunk scan bound')
                for path in paths:
                    check_deadline(deadline)
                    name=path.relative_to(root).as_posix()
                    if name in before:continue
                    if not any(path.name.startswith(mid+'-') or path.stem==mid for mid in memory_ids):continue
                    meta=metadata(path)
                    if (meta.get('id') in memory_ids and record_path in meta.get('sources',[])
                            and citations(root,meta,'source_digests',deadline)):
                        chunks.append(name)
            result.update(native_saved=True,new_chunks=sorted(set(chunks)),record_ids=ids,
                result_id=result_id,digest=digest,receipt='project-brain/local/results/'+receipt_name,
                receipt_sha256=sha(read_file(root/'project-brain/local/results'/receipt_name)[0]))
            return result
    except (OSError,ValueError,KeyError,TypeError,AttributeError,UnicodeError,RecursionError):pass
    return result
