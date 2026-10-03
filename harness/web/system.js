/* System coordinator UI. All source and worker content is rendered as text. */
const systemUi = {epoch:0, pending:false, controller:null, timer:null, project:null, catalog:null, detail:null, listed:false, readError:false, launch:null, launchCount:0, formOpen:false,
  agents:createAgentPanel($('system-agents'),{emptyText:'Select an agent card to follow its work.'})};
const SYSTEM_AGENTS = {contracts:['Contract agent','Agree contracts, invariants and the service order.'],
  verify:['Verification agent','Check producer/consumer compatibility and the end-to-end scenario.']};
// Two tabs share one state: Services (the system, its map and editor) and Changes (plans, runs and receipts).
const systemViews = ['systems','system-changes'], inSystems = () => systemViews.includes(state.view);
const systemSteps = ['Plan','Review','Run','Receipts'];
// Errors and progress appear on the tab the action started from.
const systemNotice = () => state.view === 'system-changes' ? ['system-run-error','system-run-message'] : ['system-error','system-message'];
function systemFailure(error, [errorId] = systemNotice()) {
  if (error.name !== 'AbortError') { $(errorId).textContent=error.message; $(errorId).hidden=false; }
}
function systemReadFailure(error) {
  if (error.name === 'AbortError') return;
  systemUi.readError=true; $('system-run-error').textContent=error.message; $('system-run-error').hidden=false;
}
function systemReadRecovered() { if (systemUi.readError) { systemUi.readError=false; $('system-run-error').hidden=true; } }
function systemControls() {
  const requesting=systemUi.pending || !state.bootstrap, run=systemUi.detail, live=Boolean(run?.active);
  const busy=requesting || (typeof systemEditor!=='undefined' && systemEditor.pending);
  const editing=typeof systemEditor!=='undefined' && (systemEditor.pending || systemEditor.dirty);
  for (const id of ['system-project','system-config','system-load','system-edit','system-choose-project','system-prepare','system-plan-change','system-new-change','system-runs','system-refresh','system-provider','system-mode','system-access','system-timeout','system-reviewed','system-retry','system-accept-changes']) {
    const editorLocked=editing && ['system-project','system-config','system-load','system-choose-project','system-prepare','system-plan-change','system-new-change'].includes(id);
    $(id).disabled=(['system-runs','system-refresh'].includes(id) ? requesting : busy) || editorLocked || (['system-provider','system-mode','system-access','system-timeout','system-reviewed'].includes(id) && Boolean(run?.session_id)) || (id==='system-reviewed' && run?.plan.status!=='needs_review');
  }
  $('system-execute').disabled=busy || editing || !run || live || Boolean(run.session_id) || run.plan.status!=='needs_review' || !run.providers?.some(p=>p.id===$('system-provider').value && p.available) || !$('system-reviewed').checked;
  $('system-resume').disabled=busy || editing || live || !run?.providers?.some(p=>p.id===run.provider && p.available);
  $('system-cancel').disabled=requesting || !live;
  const scanning=typeof systemDiscovery!=='undefined' && systemDiscovery.active;
  $('system-editor-lock').textContent=scanning ? 'An AI scan is filling the system editor on Services. Planning and launching wait until it ends.' : typeof systemEditor!=='undefined' && systemEditor.dirty ? 'The system editor on Services has unsaved changes. Save or discard them to plan or launch a change.' : '';
  $('system-editor-lock').hidden=!$('system-editor-lock').textContent;
  if(typeof editorControls==='function') editorControls();
}
// Services shows the editor, the loaded system or how to start; Changes leads with the selected change, and the form opens on request.
function systemLayout() {
  const editing=!$('system-editor').hidden, hasRun=Boolean(systemUi.detail), hasRuns=[...$('system-runs').options].some(option=>option.value);
  $('system-catalog').hidden=!systemUi.catalog || editing;
  $('system-empty').hidden=Boolean(systemUi.catalog) || editing;
  $('system-plan-form').hidden=!systemUi.formOpen;
  $('system-run').hidden=!hasRun || systemUi.formOpen;
  $('system-runs-empty').hidden=!systemUi.listed || hasRuns || systemUi.formOpen;
  $('system-new-change').setAttribute('aria-expanded',String(systemUi.formOpen));
  $('system-new-change').classList.toggle('primary',systemUi.listed && !hasRuns && !systemUi.formOpen);
  $('system-edit').textContent=systemUi.catalog ? 'Edit system' : 'Create or edit system';
  $('system-runs').closest('.field').hidden=!hasRuns;
}
function systemReset() {
  clearTimeout(systemUi.timer); systemUi.controller?.abort(); ++systemUi.epoch;
  systemUi.catalog=null; systemUi.detail=null; systemUi.listed=false; systemUi.readError=false; systemUi.launch=null; systemUi.launchCount=0; systemUi.formOpen=false; systemUi.agents.reset(); $('system-agents-section').hidden=true;
  if(typeof closeSystemEditor==='function') closeSystemEditor();
  $('system-plan-services').replaceChildren(); $('system-runs').replaceChildren(new Option('No changes yet',''));
  for(const [errorId,messageId] of [['system-error','system-message'],['system-run-error','system-run-message']]) { $(errorId).hidden=true; $(messageId).textContent=''; }
  $('system-reviewed').checked=false; systemLayout();
}
async function openSystems() {
  if (!state.bootstrap) return;
  const sidebar=$('project-switcher').value, held=systemUi.pending || (typeof systemEditor!=='undefined' && (systemEditor.pending || systemEditor.dirty)) || (typeof systemDiscovery!=='undefined' && systemDiscovery.active);
  setProjectChoices($('system-project'),state.bootstrap.projects,!held && projectFor(sidebar) ? sidebar : $('system-project').value || sidebar || $('project').value);
  // The sidebar may have moved the project while another section was open; its system and changes do not carry over.
  if ($('system-project').value!==systemUi.project) { systemReset(); systemUi.project=$('system-project').value; }
  await systemHistory();
}
// An unlaunched plan that matched no service reports that instead of review.
const systemStatus = run => !run.session_id && run.plan && run.plan.status!=='needs_review' ? run.plan.status : run.status;
const systemRunLabel = run => `${run.change_id} · ${humanLabel(systemStatus(run))} · ${run.id.slice(0,8)}`;
async function systemHistory() {
  const epoch=++systemUi.epoch, project=$('system-project').value;
  clearTimeout(systemUi.timer); systemUi.controller?.abort(); systemUi.controller=new AbortController();
  if (!project) return;
  try {
    const data=await api('/api/system-runs?project_id='+encodeURIComponent(project),{signal:systemUi.controller.signal});
    if (!inSystems() || epoch!==systemUi.epoch || project!==$('system-project').value) return;
    const previous=systemUi.detail?.project_id===project ? systemUi.detail.id : $('system-runs').value;
    setOptions($('system-runs'),data.runs,systemRunLabel,run=>run.id,previous);
    if (!data.runs.length) $('system-runs').options[0].textContent='No changes yet';
    systemUi.listed=true; systemReadRecovered(); if (!data.runs.length) systemUi.detail=null; systemLayout();
    if (data.runs.length) await systemRead($('system-runs').value);
  } catch(error) { if(epoch===systemUi.epoch) systemReadFailure(error); }
  systemControls();
}
async function systemRead(id) {
  clearTimeout(systemUi.timer); systemUi.controller?.abort(); const epoch=++systemUi.epoch;
  systemUi.controller=new AbortController(); if(!id) return;
  try {
    const run=await api('/api/system-runs/'+encodeURIComponent(id),{signal:systemUi.controller.signal});
    if(!inSystems() || epoch!==systemUi.epoch || run.project_id!==$('system-project').value) return;
    const changed=systemUi.detail?.id!==run.id;
    systemUi.detail=run;
    if(changed) { $('system-reviewed').checked=false; $('system-accept-changes').checked=false; $('system-provider').replaceChildren(); systemUi.launch=null; systemUi.launchCount=0; }
    renderSystemRun(); systemReadRecovered();
    if(run.active) systemUi.timer=setTimeout(()=>systemRead(id),1100);
  } catch(error) { if(epoch===systemUi.epoch) systemReadFailure(error); }
}
async function systemMutation(endpoint,body,apply) {
  if(systemUi.pending) return;
  const [errorId,messageId]=systemNotice(); if(errorId==='system-run-error') systemUi.readError=false;
  systemUi.pending=true; const epoch=++systemUi.epoch;
  clearTimeout(systemUi.timer); systemUi.controller?.abort();
  $(errorId).hidden=true; $(messageId).textContent='Working…'; systemControls();
  try {
    const data=await api(endpoint,{method:'POST',body});
    if(inSystems() && epoch===systemUi.epoch) { apply(data); $(messageId).textContent=''; }
  } catch(error) { if(epoch===systemUi.epoch) { systemFailure(error,[errorId]); $(messageId).textContent=''; } }
  finally {
    systemUi.pending=false; systemControls();
    if(inSystems() && systemUi.detail?.active) systemUi.timer=setTimeout(()=>systemRead(systemUi.detail.id),700);
  }
}
function systemSvg(tag,attributes={},text) {
  const node=document.createElementNS('http://www.w3.org/2000/svg',tag);
  for(const [key,value] of Object.entries(attributes)) node.setAttribute(key,String(value));
  if(text!==undefined) node.textContent=text;
  return node;
}
// The map is drawn at its natural size, so its text matches the page; colors and type come from app.css.
function renderSystemMap(catalog) {
  const services=catalog.services, indegree=new Map(services.map(s=>[s.id,0]));
  for(const edge of catalog.relationships) indegree.set(edge.consumer,indegree.get(edge.consumer)+1);
  const queue=services.filter(s=>indegree.get(s.id)===0).map(s=>s.id), ordered=[];
  while(queue.length) {
    const id=queue.shift(); if(ordered.includes(id)) continue; ordered.push(id);
    for(const edge of catalog.relationships.filter(e=>e.provider===id)) {
      indegree.set(edge.consumer,indegree.get(edge.consumer)-1);
      if(indegree.get(edge.consumer)===0) queue.push(edge.consumer);
    }
  }
  for(const service of services) if(!ordered.includes(service.id)) ordered.push(service.id);
  const columns=Math.min(4,Math.max(1,ordered.length)), width=columns*236+2, height=Math.ceil(ordered.length/columns)*124;
  const svg=systemSvg('svg',{viewBox:`0 0 ${width} ${height}`,width,height,role:'img','aria-label':'Declared service contract graph'});
  const defs=systemSvg('defs'), marker=systemSvg('marker',{id:'system-arrow',viewBox:'0 0 10 10',refX:9,refY:5,markerWidth:6,markerHeight:6,orient:'auto-start-reverse'});
  marker.append(systemSvg('path',{d:'M 0 0 L 10 5 L 0 10 z',class:'system-arrow'})); defs.append(marker); svg.append(defs);
  const positions=new Map(ordered.map((id,i)=>[id,{x:24+(i%columns)*236,y:24+Math.floor(i/columns)*124}]));
  for(const edge of catalog.relationships) {
    const a=positions.get(edge.provider),b=positions.get(edge.consumer), same=a.y===b.y;
    const x1=a.x+(same?190:95),y1=a.y+(same?38:76),x2=b.x+(same?0:95),y2=b.y+(same?38:0);
    const curve=systemSvg('path',{d:same?`M${x1},${y1} C${x1+20},${y1} ${x2-20},${y2} ${x2-5},${y2}`:`M${x1},${y1} C${x1},${y1+40} ${x2},${y2-40} ${x2},${y2-5}`,class:'system-edge','marker-end':'url(#system-arrow)'});
    curve.append(systemSvg('title',{},`${edge.provider} → ${edge.consumer}: ${edge.contract}`)); svg.append(curve);
  }
  for(const id of ordered) {
    const service=services.find(s=>s.id===id),pos=positions.get(id),group=systemSvg('g',{class:`system-node${service.access==='available'?'':' unavailable'}`});
    group.append(systemSvg('rect',{x:pos.x,y:pos.y,width:190,height:76,rx:8}));
    group.append(systemSvg('text',{x:pos.x+16,y:pos.y+28,class:'system-node-title'},id));
    group.append(systemSvg('text',{x:pos.x+16,y:pos.y+48,class:'system-node-meta'},service.declared?.owner || 'No passport'));
    // Only a service that cannot be used says so; an available one needs no label.
    if(service.access!=='available') group.append(systemSvg('text',{x:pos.x+16,y:pos.y+64,class:'system-node-meta'},humanLabel(service.access)));
    group.append(systemSvg('title',{},service.root)); svg.append(group);
  }
  $('system-map').replaceChildren(svg);
  return ordered;
}
// Service folders inside the system folder read as relative paths; others keep their full path.
const systemPath = (root,path) => path.startsWith(root+'/') ? path.slice(root.length+1) : path;
const systemWarning = warning => typeof warning==='string' ? warning : [warning.service,humanLabel(warning.reason),warning.path].filter(Boolean).join(' · ');
function renderSystemCatalog(data) {
  systemUi.catalog=data; const catalog=data.catalog;
  $('system-name').textContent=catalog.system;
  $('system-catalog-summary').textContent=`${catalog.services.length} ${catalog.services.length===1?'service':'services'} · ${catalog.relationships.length} declared contract ${catalog.relationships.length===1?'link':'links'} · policies and sources stay with their owning service`;
  const order=renderSystemMap(catalog);
  $('system-services').replaceChildren(); $('system-relationships').replaceChildren(); $('system-banks').replaceChildren();
  const chosen=new Set([...$('system-plan-services').querySelectorAll('input:checked')].map(input=>input.value)); $('system-plan-services').replaceChildren();
  for(const service of order.map(id=>catalog.services.find(s=>s.id===id))) {
    const card=el('article',`system-card${service.access==='available'?'':' unavailable'}`), head=el('div','system-card-head');
    head.append(el('h4','',service.id)); if(service.access!=='available') head.append(el('span','system-badge',humanLabel(service.access))); card.append(head);
    if(service.declared?.description) card.append(el('p','',service.declared.description));
    card.append(el('p','system-card-meta',[service.declared?.owner,systemPath(data.root,service.root)].filter(Boolean).join(' · ')));
    const capabilities=service.declared?.capabilities || [];
    if(capabilities.length) {
      const list=el('ul','system-capabilities');
      for(const capability of capabilities) {
        const item=el('li'); item.append(el('strong','',capability.id),document.createTextNode(` · ${humanLabel(capability.status)} — ${capability.description}`),el('span','system-card-meta',`Sources: ${capability.sources.join(', ') || 'none declared'}`));
        list.append(item);
      }
      card.append(list);
    }
    if(service.declared?.relationships_complete===false) card.append(el('p','system-card-warning','Relationships are declared incomplete. Confirm missing dependencies.'));
    $('system-services').append(card);
    const choice=el('label','system-choice'), input=el('input');
    input.type='checkbox'; input.value=service.id; input.disabled=service.access!=='available'; input.checked=chosen.has(service.id) && !input.disabled;
    choice.append(input,document.createTextNode(service.id+(service.declared?.relationships_complete===false?' · dependencies incomplete':''))); $('system-plan-services').append(choice);
  }
  for(const edge of catalog.relationships) $('system-relationships').append(el('p','system-note',`${edge.provider} → ${edge.consumer} · ${edge.contract}`));
  for(const warning of catalog.warnings) $('system-relationships').append(el('p','system-warning',systemWarning(warning)));
  for(const bank of [{id:'System coordinator',root:data.root},...catalog.services.map(s=>({id:s.id,root:s.root}))]) {
    const card=el('div','system-card');
    card.append(el('h4','',bank.id),el('p','system-card-meta',`${bank.root}/project-brain`),el('p','system-card-meta',`${bank.root}/memory-bank`));
    $('system-banks').append(card);
  }
  systemLayout(); updateHeader();
}
function systemAccessLabel(access) {
  return access==='all' ? 'every selected service folder (read; write in edit mode)' : 'own service folder only';
}
function renderSystemAgents(run) {
  const launches=run.launches || [];
  $('system-agents-section').hidden=!launches.length; $('system-agents-live').hidden=!run.active;
  if(!launches.length) { systemUi.agents.reset(); return; }
  const latest=launches.at(-1).session_id;
  // A new execute/resume launch is followed; an older launch chosen by hand stays until then.
  if(launches.length!==systemUi.launchCount || !launches.some(l=>l.session_id===systemUi.launch)) systemUi.launch=latest;
  systemUi.launchCount=launches.length;
  $('system-agents-launch-field').hidden=launches.length<2;
  setOptions($('system-agents-launch'),launches.map((l,i)=>({id:l.session_id,label:`Launch ${i+1} · ${humanLabel(l.status)}`})),l=>l.label,l=>l.id,systemUi.launch);
  // The journal is authoritative for the newest launch; older launches show their own recorded outcome.
  const current=systemUi.launch===latest, mode=run.mode || $('system-mode').value;
  const steps=run.execution?.steps || [{id:'contracts',service:'__system__',mode:'read-only'},...run.plan.context.services.map(sid=>({id:'service-'+sid,service:sid,mode})),{id:'verify',service:'__system__',mode:'read-only'}];
  systemUi.agents.setPlan(steps.map(step=>{
    const [label,goal]=SYSTEM_AGENTS[step.id] || [`${step.service} agent`,`Works on ${step.service}; folder access: ${systemAccessLabel(run.access)}.`];
    return {id:step.id,service:step.service,mode:step.mode,label,goal,
      journal:current && step.status ? (step.status==='running' && !run.active ? 'interrupted' : step.status) : null};
  }),current && run.active);
  systemUi.agents.load(systemUi.launch);
}
// Plan, Review, Run, Receipts: a change stops at Review until someone launches it.
function renderSystemStepper(run) {
  const attention=!run.active && (['blocked','interrupted','failed','cancelled'].includes(run.status) || !run.session_id && run.plan.status!=='needs_review');
  const current=!run.session_id ? 1 : run.status==='completed' && !run.active ? 4 : 2;
  $('system-stepper').replaceChildren(...systemSteps.map((label,index)=>{
    const stepState=index<current ? 'done' : index>current ? 'upcoming' : attention ? 'attention' : run.active ? 'running' : 'current';
    const item=el('li','step'); item.dataset.state=stepState; if(index===current) item.setAttribute('aria-current','step');
    item.append(el('span','step-mark',stepState==='done' ? '✓' : String(index+1)),el('span','step-label',label)); return item;
  }));
}
function renderSystemRun() {
  const run=systemUi.detail,plan=run.plan,execution=run.execution;
  renderSystemStepper(run);
  $('system-run-title').textContent=`${run.change_id} · ${run.system}`;
  // Error codes read as words; a sentence from the runner stays as written.
  const error=typeof execution?.error==='string' && /^[a-z0-9_]+$/.test(execution.error) ? humanLabel(execution.error) : execution?.error;
  const status=systemStatus(run);
  $('system-run-status').replaceChildren(el('span',`status-dot ${status}`),document.createTextNode(`${humanLabel(status)}${error?' · '+error:''}`));
  $('system-selection-note').hidden=Boolean(run.session_id) || plan.status==='needs_review';
  const facts=[['Participants',plan.context.services.join(', ') || 'None selected'],
    ['Context',`${plan.context_chars.toLocaleString()} of ${plan.context_budget_chars.toLocaleString()} characters · ${plan.context.sources.length} sources · ${plan.omitted_sources.length} omitted`],
    ['System file',run.config_path],
    ...(plan.warnings?.length ? [['Warnings',plan.warnings.slice(0,3).map(systemWarning).join('; ')+(plan.warnings.length>3?` and ${plan.warnings.length-3} more`:'')]] : []),
    ...(run.mode ? [['Launch',`${humanLabel(run.mode)} · folder access: ${systemAccessLabel(run.access)}`]] : [])];
  $('system-plan-summary').replaceChildren(...facts.flatMap(([term,value])=>[el('dt','',term),el('dd','',value)]));
  $('system-flow').replaceChildren(); $('system-flow').hidden=Boolean(run.launches?.length);
  for(const step of execution?.steps || plan.steps) {
    const item=el('li',`system-stage ${step.status || ''}`);
    item.append(el('strong','',step.id),el('span','',step.status ? `${humanLabel(step.status)} · attempt ${step.attempt}` : step.goal)); $('system-flow').append(item);
  }
  renderSystemAgents(run);
  $('system-plan-json').textContent=JSON.stringify(plan,null,2);
  $('system-launch').hidden=Boolean(run.session_id);
  $('system-recovery').hidden=!run.session_id || run.active || run.status==='completed';
  const selected=run.provider || $('system-provider').value || 'codex';
  setOptions($('system-provider'),run.providers || [],p=>`${p.name}${p.available?'':' · unavailable'}`,p=>p.id,selected);
  for(const option of $('system-provider').options) option.disabled=!run.providers?.find(p=>p.id===option.value)?.available;
  if(run.session_id) $('system-access').value=run.access;
  systemProviderNote();
  const previous=$('system-retry').value;
  setOptions($('system-retry'),[{id:'',label:'Reconcile saved receipt / continue pending dispatch'},...(execution?.steps || []).filter(s=>['running','interrupted','blocked'].includes(s.status)).map(s=>({id:s.id,label:`Retry ${s.id} · ${humanLabel(s.status)}`}))],s=>s.label,s=>s.id,previous);
  $('system-cancel').hidden=!run.active; $('system-open-session').hidden=!run.session_id;
  $('system-results').hidden=!execution; $('system-receipts').replaceChildren(); $('system-native-tasks').replaceChildren();
  for(const receipt of run.receipts || []) {
    const card=el('article',`system-card${receipt.ok?'':' blocked'}`), head=el('div','system-card-head');
    head.append(el('h5','',receipt.phase)); if(!receipt.ok) head.append(el('span','system-badge','Blocked')); card.append(head,el('p','',receipt.report?.summary || receipt.error || 'No report'));
    for(const check of receipt.report?.checks || []) card.append(el('p','system-card-meta',`${check.name}: ${check.status}\n${check.detail}`));
    if(receipt.report?.changed_files?.length) card.append(el('p','system-card-meta',`Changed files: ${receipt.report.changed_files.join(', ')}`)); $('system-receipts').append(card);
  }
  const tasks=Object.entries(execution?.tasks || {});
  for(const [service,task] of tasks) {
    const item=el('li'); item.append(el('strong','',service),document.createTextNode(` · ${task.closed?'Closed':'Open'}`),el('code','',task.external_id),el('code','',task.uuid || 'Task not created yet')); $('system-native-tasks').append(item);
  }
  $('system-native-details').hidden=!tasks.length; $('system-native-summary').textContent=`Native task references (${tasks.length})`;
  $('system-handoff').textContent=run.handoff?`Handoff saved: ${execution.run_dir}/handoff.json\n${run.handoff.knowledge}`:'';
  $('system-events-log').hidden=!run.events?.length;
  $('system-events').textContent=(run.events || []).filter(e=>e.kind!=='user').map(e=>e.text || '').join('\n');
  const option=[...$('system-runs').options].find(o=>o.value===run.id); if(option) option.textContent=systemRunLabel(run);
  systemLayout(); systemControls();
}
// A new change starts from the loaded system: its services are the starting choices.
async function openSystemPlan() {
  if(!systemUi.catalog) await systemMutation('/api/systems/catalog',{project_id:$('system-project').value,config_path:$('system-config').value},renderSystemCatalog);
  if(!systemUi.catalog || !inSystems()) return;
  systemUi.formOpen=true; systemLayout(); $('system-task').focus();
}
function closeSystemPlan() { systemUi.formOpen=false; systemLayout(); ($('system-new-change').disabled ? $('view-heading') : $('system-new-change')).focus(); }
$('system-project').addEventListener('change',()=>{systemReset();systemUi.project=$('system-project').value;systemHistory();updateHeader();});
// Another system file needs its own load; a half-filled change for the old one closes.
$('system-config').addEventListener('input',()=>{systemUi.catalog=null;systemUi.formOpen=false;$('system-plan-services').replaceChildren();systemLayout();updateHeader();});
$('system-load').addEventListener('click',()=>systemMutation('/api/systems/catalog',{project_id:$('system-project').value,config_path:$('system-config').value},data=>{
  if(!$('system-editor').hidden && !systemEditor.dirty) closeSystemEditor();
  renderSystemCatalog(data);
}));
$('system-plan-change').addEventListener('click',()=>{openView('system-changes');openSystemPlan();});
$('system-new-change').addEventListener('click',()=>systemUi.formOpen ? closeSystemPlan() : openSystemPlan());
$('system-plan-cancel').addEventListener('click',closeSystemPlan);
$('system-plan-form').addEventListener('submit',event=>{
  event.preventDefault(); if(!systemUi.catalog) return;
  const body={project_id:$('system-project').value,config_path:$('system-config').value,task:$('system-task').value,change_id:$('system-change-id').value,services:[...$('system-plan-services').querySelectorAll('input:checked')].map(i=>i.value),contracts:$('system-contracts').value.split(',').map(s=>s.trim()).filter(Boolean),budget:Number($('system-budget').value)};
  systemMutation('/api/system-runs',body,run=>{
    systemUi.detail=run; systemUi.formOpen=false; $('system-plan-form').reset(); $('system-reviewed').checked=false; $('system-provider').replaceChildren();
    for(const empty of [...$('system-runs').options].filter(option=>!option.value)) empty.remove();
    const option=el('option','',systemRunLabel(run)); option.value=run.id;
    $('system-runs').prepend(option); $('system-runs').value=run.id;
    renderSystemRun(); $('system-run').scrollIntoView({behavior:'smooth',block:'start'}); $('system-run-title').focus({preventScroll:true});
  });
});
async function systemAction(action,extra={}) {
  const run=systemUi.detail;if(!run) return;
  await systemMutation('/api/system-runs/'+run.id,{action,revision:run.revision,...extra},updated=>{systemUi.detail=updated;renderSystemRun();});
}
function systemProviderNote() {
  const run=systemUi.detail, provider=(run?.providers || []).find(p=>p.id===(run.provider || $('system-provider').value));
  // Cursor's CLI has no verified option to grant other folders.
  const shared=[...$('system-access').options].find(option=>option.value==='all');
  shared.disabled=provider?.id==='cursor';
  if(shared.disabled && $('system-access').value==='all' && !run?.session_id) $('system-access').value='service';
  const access=$('system-access').value==='all'
    ? 'Every agent can read all selected service folders; in edit mode each service agent may also change files in any of them. The system folder stays read-only.'
    : 'Each service agent reads and writes only its own folder; contract and verification agents read all selected services.';
  const edit=(run?.mode || $('system-mode').value)==='edit' ? ' Edit mode changes files in each service’s current checkout.' : '';
  const resume=(run?.providers || []).find(p=>p.id===run?.provider);
  $('system-resume-note').textContent=!run?.session_id ? '' : resume?.available ? `Resumes with ${resume.name} and the same folder access (${systemAccessLabel(run.access)}). The system folder stays read-only.` : `${resume?.name || 'The saved provider'} CLI is unavailable, so this run cannot resume.`;
  $('system-provider-note').textContent=provider?.available?`${provider.name} uses its configured CLI and default model. The provider and folder access are fixed after launch. ${access}${edit} Read-only runs create native Brain task records.${shared.disabled?' Cursor Agent cannot be granted other service folders.':''}`:'Selected provider CLI is unavailable. Planning and catalog browsing remain available.';
}
$('system-provider').addEventListener('change',()=>{systemProviderNote();systemControls();});
$('system-access').addEventListener('change',systemProviderNote);
$('system-mode').addEventListener('change',systemProviderNote);
$('system-agents-launch').addEventListener('change',()=>{systemUi.launch=$('system-agents-launch').value; if(systemUi.detail) renderSystemAgents(systemUi.detail);});
$('system-reviewed').addEventListener('change',systemControls);
$('system-execute').addEventListener('click',()=>{if($('system-reviewed').checked) systemAction('execute',{provider:$('system-provider').value,mode:$('system-mode').value,access:$('system-access').value,timeout:Number($('system-timeout').value)});});
$('system-cancel').addEventListener('click',()=>systemAction('cancel'));
$('system-resume').addEventListener('click',()=>systemAction('resume',{...($('system-retry').value?{retry_step:$('system-retry').value}:{}),accept_source_changes:$('system-accept-changes').checked}));
$('system-refresh').addEventListener('click',systemHistory);
$('system-runs').addEventListener('change',()=>{systemUi.formOpen=false; systemLayout(); systemRead($('system-runs').value);});
$('system-open-session').addEventListener('click',()=>{const id=systemUi.detail?.session_id;if(id) {setView('sessions');selectSession(id);}});
systemControls(); systemLayout();
if (inSystems() && state.bootstrap) openSystems();
