/* System coordinator UI. All source and worker content is rendered as text. */
const systemUi = {epoch:0, pending:false, controller:null, timer:null, catalog:null, detail:null};
function systemFailure(error) {
  if (error.name !== 'AbortError') { $('system-error').textContent=error.message; $('system-error').hidden=false; }
}
function systemControls() {
  const run=systemUi.detail, busy=systemUi.pending || !state.bootstrap, live=Boolean(run?.active);
  for (const id of ['system-project','system-config','system-load','system-prepare','system-runs','system-refresh','system-provider','system-mode','system-timeout','system-reviewed','system-retry','system-accept-changes']) {
    $(id).disabled=busy || (['system-provider','system-mode','system-timeout','system-reviewed'].includes(id) && Boolean(run?.session_id));
  }
  $('system-execute').disabled=busy || !run || live || Boolean(run.session_id) || run.plan.status!=='needs_review' || !run.providers?.some(p=>p.id===$('system-provider').value && p.available) || !$('system-reviewed').checked;
  $('system-resume').disabled=busy || live || !run?.providers?.some(p=>p.id===run.provider && p.available);
  $('system-cancel').disabled=busy || !live;
}
function systemReset() {
  clearTimeout(systemUi.timer); systemUi.controller?.abort(); ++systemUi.epoch;
  systemUi.catalog=null; systemUi.detail=null;
  $('system-catalog').hidden=true; $('system-run').hidden=true; $('system-error').hidden=true;
  $('system-message').textContent=''; $('system-reviewed').checked=false;
}
async function openSystems() {
  if (!state.bootstrap) return;
  setProjectChoices($('system-project'),state.bootstrap.projects,$('system-project').value || $('project').value);
  await systemHistory();
}
async function systemHistory() {
  const epoch=++systemUi.epoch, project=$('system-project').value;
  clearTimeout(systemUi.timer); systemUi.controller?.abort(); systemUi.controller=new AbortController();
  if (!project) return;
  try {
    const data=await api('/api/system-runs?project_id='+encodeURIComponent(project),{signal:systemUi.controller.signal});
    if (state.view!=='systems' || epoch!==systemUi.epoch || project!==$('system-project').value) return;
    const previous=systemUi.detail?.project_id===project ? systemUi.detail.id : $('system-runs').value;
    setOptions($('system-runs'),data.runs,run=>`${run.change_id} · ${run.status} · ${run.id.slice(0,8)}`,run=>run.id,previous);
    if (data.runs.length) await systemRead($('system-runs').value);
    else { systemUi.detail=null; $('system-run').hidden=true; }
  } catch(error) { if(epoch===systemUi.epoch) systemFailure(error); }
  systemControls();
}
async function systemRead(id) {
  clearTimeout(systemUi.timer); systemUi.controller?.abort(); const epoch=++systemUi.epoch;
  systemUi.controller=new AbortController(); if(!id) return;
  try {
    const run=await api('/api/system-runs/'+encodeURIComponent(id),{signal:systemUi.controller.signal});
    if(state.view!=='systems' || epoch!==systemUi.epoch || run.project_id!==$('system-project').value) return;
    const changed=systemUi.detail?.id!==run.id;
    systemUi.detail=run;
    if(changed) { $('system-reviewed').checked=false; $('system-accept-changes').checked=false; $('system-provider').replaceChildren(); }
    renderSystemRun();
    if(run.active) systemUi.timer=setTimeout(()=>systemRead(id),1100);
  } catch(error) { if(epoch===systemUi.epoch) systemFailure(error); }
}
async function systemMutation(endpoint,body,apply) {
  if(systemUi.pending) return;
  systemUi.pending=true; const epoch=++systemUi.epoch;
  clearTimeout(systemUi.timer); systemUi.controller?.abort();
  $('system-error').hidden=true; $('system-message').textContent='Working…'; systemControls();
  try {
    const data=await api(endpoint,{method:'POST',body});
    if(state.view==='systems' && epoch===systemUi.epoch) { apply(data); $('system-message').textContent=''; }
  } catch(error) { if(epoch===systemUi.epoch) { systemFailure(error); $('system-message').textContent=''; } }
  finally {
    systemUi.pending=false; systemControls();
    if(state.view==='systems' && systemUi.detail?.active) systemUi.timer=setTimeout(()=>systemRead(systemUi.detail.id),700);
  }
}
function systemSvg(tag,attributes={},text) {
  const node=document.createElementNS('http://www.w3.org/2000/svg',tag);
  for(const [key,value] of Object.entries(attributes)) node.setAttribute(key,String(value));
  if(text!==undefined) node.textContent=text;
  return node;
}
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
  const columns=Math.min(4,Math.max(1,ordered.length)), width=columns*236+30, height=Math.ceil(ordered.length/columns)*130+42;
  const svg=systemSvg('svg',{viewBox:`0 0 ${width} ${height}`,role:'img','aria-label':'Declared service contract graph'});
  const defs=systemSvg('defs'), marker=systemSvg('marker',{id:'system-arrow',viewBox:'0 0 10 10',refX:9,refY:5,markerWidth:6,markerHeight:6,orient:'auto-start-reverse'});
  marker.append(systemSvg('path',{d:'M 0 0 L 10 5 L 0 10 z',fill:'#9c8cba'})); defs.append(marker); svg.append(defs);
  const positions=new Map(ordered.map((id,i)=>[id,{x:24+(i%columns)*236,y:32+Math.floor(i/columns)*130}]));
  for(const edge of catalog.relationships) {
    const a=positions.get(edge.provider),b=positions.get(edge.consumer), same=a.y===b.y;
    const x1=a.x+(same?190:95),y1=a.y+(same?36:76),x2=b.x+(same?0:95),y2=b.y+(same?36:0);
    const curve=systemSvg('path',{d:same?`M${x1},${y1} C${x1+20},${y1} ${x2-20},${y2} ${x2-5},${y2}`:`M${x1},${y1} C${x1},${y1+40} ${x2},${y2-40} ${x2},${y2-5}`,fill:'none',stroke:'#b4a5cb','stroke-width':1.5,'marker-end':'url(#system-arrow)'});
    curve.append(systemSvg('title',{},`${edge.provider} → ${edge.consumer}: ${edge.contract}`)); svg.append(curve);
  }
  for(const id of ordered) {
    const service=services.find(s=>s.id===id),pos=positions.get(id),group=systemSvg('g');
    group.append(systemSvg('rect',{x:pos.x,y:pos.y,width:190,height:76,rx:9,fill:'#fff',stroke:service.access==='available'?'#d8cfe8':'#debda0'}));
    group.append(systemSvg('text',{x:pos.x+13,y:pos.y+25,fill:'#3e3157','font-size':13,'font-weight':600},id));
    group.append(systemSvg('text',{x:pos.x+13,y:pos.y+46,fill:'#84778f','font-size':10},service.declared?.owner || 'No passport'));
    group.append(systemSvg('text',{x:pos.x+13,y:pos.y+63,fill:'#84778f','font-size':10},service.access));
    group.append(systemSvg('title',{},service.root)); svg.append(group);
  }
  $('system-map').replaceChildren(svg);
}
function renderSystemCatalog(data) {
  systemUi.catalog=data; const catalog=data.catalog;
  $('system-catalog').hidden=false; $('system-name').textContent=catalog.system;
  $('system-catalog-summary').textContent=`${catalog.services.length} services · ${catalog.relationships.length} declared contract links · policies and source files stay with their owning service`;
  renderSystemMap(catalog);
  $('system-services').replaceChildren(); $('system-relationships').replaceChildren(); $('system-banks').replaceChildren();
  for(const service of catalog.services) {
    const card=el('div','system-card'),label=el('label','system-choice'),input=el('input');
    input.type='checkbox'; input.value=service.id; input.disabled=service.access!=='available';
    label.append(input,document.createTextNode(service.id)); card.append(label);
    card.append(el('p','',service.declared?.description || service.access),el('p','',service.root));
    for(const capability of service.declared?.capabilities || []) card.append(el('p','',`${capability.id} · ${capability.status}\n${capability.description}\nSources: ${capability.sources.join(', ') || 'none declared'}`));
    if(service.declared?.relationships_complete===false) card.append(el('p','','Relationships are declared incomplete. Confirm missing dependencies.'));
    $('system-services').append(card);
  }
  for(const edge of catalog.relationships) $('system-relationships').append(el('p','system-note',`${edge.provider} → ${edge.consumer} · ${edge.contract}`));
  for(const warning of catalog.warnings) $('system-relationships').append(el('p','system-note',JSON.stringify(warning)));
  for(const bank of [{id:'System coordinator',root:data.root},...catalog.services.map(s=>({id:s.id,root:s.root}))]) {
    const card=el('div','system-card');
    card.append(el('h3','',bank.id),el('p','',`${bank.root}/project-brain`),el('p','',`${bank.root}/memory-bank`));
    $('system-banks').append(card);
  }
}
function renderSystemRun() {
  const run=systemUi.detail,plan=run.plan,execution=run.execution;
  $('system-run').hidden=false; $('system-run-title').textContent=`${run.change_id} · ${run.system}`;
  $('system-run-status').textContent=`${run.status}${execution?.error?' · '+execution.error:''}`;
  $('system-plan-summary').textContent=`Participants: ${plan.context.services.join(', ') || 'none selected'} · ${plan.context_chars.toLocaleString()} / ${plan.context_budget_chars.toLocaleString()} context characters · ${plan.context.sources.length} sources · ${plan.omitted_sources.length} omitted\n${run.config_path} · ${run.mode || 'Execution not started'}`;
  $('system-flow').replaceChildren();
  for(const step of execution?.steps || plan.steps) {
    const card=el('div',`system-stage ${step.status || ''}`);
    card.append(el('strong','',step.id),el('span','',step.status ? `${step.status} · attempt ${step.attempt}` : step.goal)); $('system-flow').append(card);
  }
  $('system-plan-json').textContent=JSON.stringify(plan,null,2);
  $('system-launch').hidden=Boolean(run.session_id);
  $('system-recovery').hidden=!run.session_id || run.active || run.status==='completed';
  const selected=run.provider || $('system-provider').value || 'codex';
  setOptions($('system-provider'),run.providers || [],p=>`${p.name}${p.available?'':' · unavailable'}`,p=>p.id,selected);
  for(const option of $('system-provider').options) option.disabled=!run.providers?.find(p=>p.id===option.value)?.available;
  systemProviderNote();
  const previous=$('system-retry').value;
  setOptions($('system-retry'),[{id:'',label:'Reconcile saved receipt / continue pending dispatch'},...(execution?.steps || []).filter(s=>['running','interrupted','blocked'].includes(s.status)).map(s=>({id:s.id,label:`Retry ${s.id} · ${s.status}`}))],s=>s.label,s=>s.id,previous);
  $('system-cancel').hidden=!run.active; $('system-open-session').hidden=!run.session_id;
  $('system-results').hidden=!execution; $('system-receipts').replaceChildren(); $('system-native-tasks').replaceChildren();
  for(const receipt of run.receipts || []) {
    const card=el('div','system-card'); card.append(el('h3','',`${receipt.phase} · ${receipt.ok?'completed':'blocked'}`),el('p','',receipt.report?.summary || receipt.error || 'No report'));
    for(const check of receipt.report?.checks || []) card.append(el('p','',`${check.name}: ${check.status}\n${check.detail}`));
    if(receipt.report?.changed_files?.length) card.append(el('p','',`Changed files: ${receipt.report.changed_files.join(', ')}`)); $('system-receipts').append(card);
  }
  for(const [service,task] of Object.entries(execution?.tasks || {})) {
    const card=el('div','system-card'); card.append(el('h3','',`${service} · ${task.closed?'closed':'open'}`),el('p','',task.external_id),el('p','',task.uuid || 'Task not created yet')); $('system-native-tasks').append(card);
  }
  $('system-handoff').textContent=run.handoff?`Handoff saved: ${execution.run_dir}/handoff.json\n${run.handoff.knowledge}`:'';
  $('system-events').hidden=!run.events?.length;
  $('system-events').textContent=(run.events || []).filter(e=>e.kind!=='user').map(e=>e.text || '').join('\n');
  const option=[...$('system-runs').options].find(o=>o.value===run.id); if(option) option.textContent=`${run.change_id} · ${run.status} · ${run.id.slice(0,8)}`;
  systemControls();
}
$('system-project').addEventListener('change',()=>{systemReset();systemHistory();});
$('system-config').addEventListener('input',()=>{systemUi.catalog=null;$('system-catalog').hidden=true;});
$('system-load').addEventListener('click',()=>systemMutation('/api/systems/catalog',{project_id:$('system-project').value,config_path:$('system-config').value},renderSystemCatalog));
$('system-plan-form').addEventListener('submit',event=>{
  event.preventDefault(); if(!systemUi.catalog) return;
  const body={project_id:$('system-project').value,config_path:$('system-config').value,task:$('system-task').value,change_id:$('system-change-id').value,services:[...$('system-services').querySelectorAll('input:checked')].map(i=>i.value),contracts:$('system-contracts').value.split(',').map(s=>s.trim()).filter(Boolean),budget:Number($('system-budget').value)};
  systemMutation('/api/system-runs',body,run=>{
    systemUi.detail=run; $('system-reviewed').checked=false; $('system-provider').replaceChildren();
    for(const empty of [...$('system-runs').options].filter(option=>!option.value)) empty.remove();
    const option=el('option','',`${run.change_id} · ${run.status} · ${run.id.slice(0,8)}`); option.value=run.id;
    $('system-runs').prepend(option); $('system-runs').value=run.id;
    renderSystemRun(); $('system-run').scrollIntoView({behavior:'smooth',block:'start'});
  });
});
async function systemAction(action,extra={}) {
  const run=systemUi.detail;if(!run) return;
  await systemMutation('/api/system-runs/'+run.id,{action,revision:run.revision,...extra},updated=>{systemUi.detail=updated;renderSystemRun();});
}
function systemProviderNote() {
  const run=systemUi.detail, provider=(run?.providers || []).find(p=>p.id===(run.provider || $('system-provider').value));
  $('system-provider-note').textContent=provider?.available?`${provider.name} uses its configured CLI and default model. The provider is fixed after launch. Read-only runs create native Brain task records. Edit mode writes in each service’s current checkout.`:'Selected provider CLI is unavailable. Planning and catalog browsing remain available.';
}
$('system-provider').addEventListener('change',()=>{systemProviderNote();systemControls();});
$('system-reviewed').addEventListener('change',systemControls);
$('system-execute').addEventListener('click',()=>{if($('system-reviewed').checked) systemAction('execute',{provider:$('system-provider').value,mode:$('system-mode').value,timeout:Number($('system-timeout').value)});});
$('system-cancel').addEventListener('click',()=>systemAction('cancel'));
$('system-resume').addEventListener('click',()=>systemAction('resume',{...($('system-retry').value?{retry_step:$('system-retry').value}:{}),accept_source_changes:$('system-accept-changes').checked}));
$('system-refresh').addEventListener('click',systemHistory);
$('system-runs').addEventListener('change',()=>systemRead($('system-runs').value));
$('system-open-session').addEventListener('click',()=>{const id=systemUi.detail?.session_id;if(id) {setView('sessions');selectSession(id);}});
systemControls();
if (state.view==='systems' && state.bootstrap) openSystems();
