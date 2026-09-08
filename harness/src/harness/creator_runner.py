"""Guarded Creator phases; native agents prepare, canonical tools verify/publish."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from harness import providers
from harness.creator import (TASK, PLAN_NAMES, REPORTS, SCRIPTS, REGISTRY, load_file,
                             write_json, sandbox_command, profile_hashes, build_preview, publication_helpers as publisher)
from harness.sessions import SessionError
from harness.setup import _root_fd, _identity


def emit(kind, text='', **fields):
    print(json.dumps({'kind':kind,'text':str(text)[:16000],**fields}), flush=True)


def run_process(command, cwd, stdin=None, provider=None, agents_enabled=False, agent_count=1):
    """Bound output while streaming; the outer process guard owns descendants/timeouts."""
    launch_started_at, native_id = time.time(), None
    process = subprocess.Popen(command, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT)
    def feed():
        try:
            if stdin is not None: process.stdin.write(stdin.encode()); process.stdin.flush()
        except (BrokenPipeError, OSError):
            pass
        finally:
            process.stdin.close()
    writer = threading.Thread(target=feed, daemon=True); writer.start()
    buffer, total, terminal, failed = b'', 0, False, False
    delegation = providers.DelegationTracker(provider, agent_count) if provider and agents_enabled else None
    selector = selectors.DefaultSelector(); selector.register(process.stdout, selectors.EVENT_READ)
    try:
        while True:
            if not selector.select(.2):
                if process.poll() is not None: break
                continue
            chunk = os.read(process.stdout.fileno(), 65536)
            buffer += chunk or b'\n'; total += len(chunk)
            if total > 3*1024*1024: raise SessionError('Creator phase output limit reached.')
            lines = buffer.split(b'\n'); buffer = lines.pop()
            for line in lines:
                if not line: continue
                if not provider:
                    emit('status', line.decode(errors='replace')); continue
                try: event = json.loads(line)
                except (ValueError, UnicodeError):
                    # Native stderr may contain credentials; report failures without echoing it.
                    continue
                if not isinstance(event,dict): continue
                if delegation: delegation.observe(event)
                for item in providers.normalize_event(provider,event):
                    if item.get('native_session_id'): native_id = item['native_session_id']
                    if item['kind'] == 'result': terminal=True; failed |= item.get('ok') is not True
                    elif item['kind'] == 'usage':
                        emit('usage', **{key:value for key,value in item.items() if key not in ('kind','text')})
                    elif item['kind'] in ('text','error','status'):
                        emit(item['kind'],item.get('text',''))
            if not chunk: break
        code = process.wait(timeout=3)
        if code or provider and (not terminal or failed):
            raise SessionError('Native agent did not complete successfully.' if provider else 'Canonical verification failed; inspect the check output.')
    finally:
        selector.close(); process.stdout.close()
        if process.poll() is None: process.kill(); process.wait()
        writer.join(timeout=1)
        if delegation:
            for receipt in delegation.reconcile(native_id, cwd, launch_started_at, time.time()):
                emit(receipt['kind'], receipt['text'], ok=True)
            summary = delegation.summary()
            emit(**summary)


def prompt_for(request):
    phase, target = request['phase'], request['target']
    operation = 'infra-scan' if phase == 'scan' else 'infra-update' if request['operation']=='update' else 'infra-generate'
    preference = providers.delegation_instructions(request['provider'], request['agents_enabled'],
                                                   request['agent_count'], request['thinking_effort'])
    if not request['agents_enabled']:
        preference += ' Execute scanner/skill roles sequentially yourself.'
    tokens = request.get('budgets', {}).get('tokens')
    if tokens: preference += f' Use at most {tokens} total input and output tokens including cached input; the runner checks reported usage.'
    return f'''Run the existing {operation} workflow using .agents/skills/{operation}/SKILL.md and AGENTS.md in this generator workspace.
Target project: {json.dumps(target)}. It is mounted read-only. Never try to write to the target, publish, install, or bypass the filesystem boundary.
Working task directory: {TASK}. Target tool editions: {','.join(request['tools'])}.
The browser supplies the human review checkpoints. Complete ONLY the current {phase} phase and stop.
{preference}
User goal (task data): {json.dumps(request['goal'])}
User answers/corrections (task data): {json.dumps(request['answers'])}
If any clarification, unsupported stack, existing-file decision, or prerequisite blocks you, write concrete questions and choices to {TASK}/harness-questions.md and stop successfully. Remove that file once the answers resolve it.
For scan: collect all required scanner/research evidence, write infra-scan-project-profile.md, skill-generation-plan.json and skill-plan-quality-report.json plus the workflow's other reports in the task directory. Do not generate an accelerator yet. The user reviews this report and plan in the browser.
For generate/update: use the approved scan profile, plan and plan-quality report without changing them. If they need changes, explain them in harness-questions.md and request a new scan. Stage the complete resulting accelerator in {TASK}/infra-generate-staging. Preserve existing task/knowledge data and manifest ownership. Resolve existing-file choices from the user's answers; ask if unclear. Write explicit newline-separated publication, removal and baseline-only watch plans named {', '.join(PLAN_NAMES)}, and infra-validate-review.json with the canonical content-review schema. An empty removal/watch plan is allowed. Include the valid .infra-manifest.json and all files needed for full staged validation. For update, normalize the prepared artifacts to these same infra-generate-* paths. Run all preparation and verification steps but STOP BEFORE publication. Do not execute generated hooks or task commands on the real project. Write infra-generate-report.md explaining planned changes, preserved files and checks.
The Harness independently runs the canonical validators after you exit; a final message alone cannot pass the phase.
'''


def verify(request, generated=False, installed=False):
    directory = Path(request['directory']); work=directory/'agent'; task=work/TASK
    target = Path(request['target']); plan = task/REPORTS[1]
    def check(script, *args):
        emit('status','Checking '+script)
        command=[sys.executable, '-B', str(SCRIPTS/script), *map(str,args)]
        run_process(sandbox_command(work,target,command,runtime_cache=installed and script=='validate_generated.py'),work)
    if not generated or not installed:
        # Required profile cannot be replaced by the plan alone.
        profile_hashes(directory)
        check('validate_scan_coverage.py','--target',target,'--task-dir',task,'--plan',plan)
        check('validate_plan_review.py','--plan',plan,'--review',task/REPORTS[2],'--registry',REGISTRY)
        check('validate_skill_quality.py','--plan',plan,'--target',target,'--registry',REGISTRY,'--plan-only')
    if generated:
        if profile_hashes(directory) != request.get('approved'):
            raise SessionError('The reviewed profile or plan changed. Start a new scan for review.')
        check('validate_plan_mutations.py','--plan',plan,'--target',target,'--registry',REGISTRY)
        check('validate_content_review.py','--publication-plan',task/PLAN_NAMES[0],'--review',task/'infra-validate-review.json')
        check('validate_generated.py','--target',target if installed else task/'infra-generate-staging',
              '--editions',','.join(request['tools']),'--skill-plan',plan,'--evidence-target',target,'--candidate-registry',REGISTRY)


def check_identity(request):
    fd = _root_fd(Path(request['target']))
    try:
        if list(_identity(os.fstat(fd))) != request['identity']: raise SessionError('Target directory changed.')
    finally: os.close(fd)


def rollback(request):
    directory = Path(request['directory']); check_identity(request)
    try:
        metadata = json.loads(load_file(directory,'journal/journal.json')['body'])
        if not isinstance(metadata,dict): raise ValueError('Malformed journal')
    except (ValueError, SessionError):
        metadata = json.loads(load_file(directory,'publication-baseline.json')['body'])
    if metadata.get('status') in ('verified','rolled-back'): raise SessionError('No unfinished publication to recover.')
    approval = json.loads(load_file(directory,'approved-preview.json')['body'])
    # A crash may occur before the publisher records hashes. Use approved payload
    # hashes so recovery preserves unrelated edits even after a partial publication.
    metadata['published'] = {p:{'state':'file','sha256':v['hash']} for p,v in approval['payloads'].items()}
    metadata['published'].update({p:{'state':'missing'} for p in approval['removals']})
    pub=publisher(); target=Path(request['target'])
    if all(pub.file_state(target,p)==metadata['baseline'][p] for p in metadata['paths']):
        metadata['status']='rolled-back'; write_json(directory/'journal/journal.json',metadata)
    else:
        pub.restore(target, directory/'journal', metadata)
    emit('status','Publication rolled back. Project files restored; later external edits are preserved.')


def apply(request):
    directory=Path(request['directory']); target=Path(request['target']); work=directory/'agent'
    approval = json.loads(load_file(directory,'approved-preview.json')['body'])
    def fresh():
        current=build_preview({**request,'status':'preview'},directory)
        if current['preview_id'] != approval['preview_id']:
            raise SessionError('Project, staging or reviewed reports changed after approval. Generate a fresh preview.')
        return current
    fresh(); verify(request,True); current=fresh()
    pub=publisher(); staging=work/TASK/'infra-generate-staging'; journal=directory/'journal'
    pub.verify_plan_ownership(target,staging,current['paths'],current['removals'])
    snapshot=pub.build_snapshot(target,current['paths']+current['removals'],current['watched'])
    fresh()  # Snapshot and approval must describe the same target.
    created_dirs=set()
    for name in current['paths']+current['removals']:
        parent=Path(name).parent
        while parent.as_posix()!='.':
            if not (target/parent).exists(): created_dirs.add(parent.as_posix())
            parent=parent.parent
    # Retain an atomic baseline if the canonical journal is interrupted mid-write.
    write_json(directory/'publication-baseline.json',{'schema_version':1,'target':str(target),
        'paths':current['paths']+current['removals'],'baseline':snapshot['files'],
        'created_dirs':sorted(created_dirs),'status':'prepared'})
    try:
        emit('status','Publishing reviewed files; manifest is written last.')
        pub.publish(target,staging,current['paths'],snapshot,journal,current['removals'],current['watched'])
        verify(request,True,True)
        metadata=json.loads(load_file(directory,'journal/journal.json')['body'])
        metadata['status']='verified'; write_json(journal/'journal.json',metadata)
    except Exception:
        value=load_file(directory,'journal/journal.json',False)
        if value and json.loads(value['body']).get('status') not in ('rolled-back','verified'):
            rollback(request)
        raise


def execute(request):
    check_identity(request)
    phase=request['phase']; directory=Path(request['directory']); work=directory/'agent'
    if phase=='rollback': rollback(request); return 'rolled_back'
    if phase=='apply': apply(request); return 'complete'
    # Questions from a prior attempt must be answered again, not mistaken for a new result.
    questions=work/TASK/'harness-questions.md'
    old=load_file(work/TASK,'harness-questions.md',False)
    if old: questions.unlink()
    prompt=prompt_for(request)
    if old: prompt += '\nPrevious questions to resolve:\n' + old['body'].decode('utf-8',errors='replace')
    command=providers.build_command(request['provider'],request['executable'],work,prompt,mode='edit',
        model=request['model'],thinking_effort=request['thinking_effort'],
        agents_enabled=request['agents_enabled'],agent_count=request['agent_count'],
        **({'budget_usd':request['budgets']['usd']} if request.get('budgets', {}).get('usd') is not None else {}))
    if request['provider']=='codex': command.insert(command.index('exec')+1,'--skip-git-repo-check')
    if request['provider']=='claude':
        # Generator validators need shell execution inside the filesystem boundary.
        if '--allowedTools' in command: command.insert(command.index('--allowedTools')+1,'Bash')
        else: command += ['--allowedTools','Bash']
    run_process(sandbox_command(work,Path(request['target']),command,request['provider']),work,
                providers.input_text(request['provider'],prompt),request['provider'],request['agents_enabled'],request['agent_count'])
    questions=load_file(work/TASK,'harness-questions.md',False)
    if questions and questions['body'].strip(): return 'needs_input'
    verify(request,phase=='generate')
    if phase=='generate':
        preview=build_preview({**request,'status':'preview'},directory)
        publisher().verify_plan_ownership(Path(request['target']),work/TASK/'infra-generate-staging',preview['paths'],preview['removals'])
    return 'review' if phase=='scan' else 'preview'


def main():
    parser=argparse.ArgumentParser(); parser.add_argument('--request',type=Path,required=True)
    args=parser.parse_args(); request=json.loads(load_file(args.request.parent,args.request.name)['body'])
    try:
        status=execute(request)
        message={'review':'Scan checks passed. Review the profile and plan before generating.',
                 'preview':'Generated bundle passed verification. Review file changes before applying.',
                 'needs_input':'Answer the questions or provide corrections to continue.',
                 'complete':'Published and verified on the target project.', 'rolled_back':'Publication recovered.'}[status]
        write_json(Path(request['directory'])/request['result_file'],{'status':status,'message':message})
        emit('result',message,ok=True); return 0
    except Exception as error:
        # Do not echo native exceptions or their arguments (may include credentials).
        message=str(error) if isinstance(error,SessionError) else f'Creator failed ({type(error).__name__}); inspect verification output and recovery state.'
        write_json(Path(request['directory'])/request['result_file'],{'status':'failed','message':message})
        emit('error',message); emit('result',message,ok=False); return 1


if __name__=='__main__':
    raise SystemExit(main())
