// Accelerators overview and Infrastructure Creator; starts the app once every script has loaded.
// Classic script loaded in order by index.html; top-level names are shared with the other app-*.js files.
'use strict';
function renderAccelerators() {
  const holder = $('accelerators'); holder.replaceChildren();
  const kits = state.bootstrap.accelerators;
  for (const [index,kit] of kits.entries()) {
    const block = el('article','accelerator'); const content = el('div'); block.append(el('div','kit-number',`Kit ${index + 1}`),content); content.append(el('h3','',kit.name),el('p','',kit.description));
    const actions = el('div','accelerator-actions'); const identity = `${kit.id} ${kit.name}`.toLowerCase();
    if (Array.isArray(kit.editions)) {
      const label = el('label','field','Edition'); const select = el('select'); select.id = 'preview-edition'; for (const edition of kit.editions) { const option = el('option','',typeof edition === 'string' ? edition : edition.name); option.value = typeof edition === 'string' ? edition : edition.id; select.append(option); } label.append(select);
      const button = el('button','button','Open project setup'); button.type = 'button'; button.addEventListener('click',() => openProjectSetup($('accelerator-project').value,select.value)); actions.append(label,button); content.append(actions,el('p','small-note','Choose target tools, review file changes, then install from Projects & Setup.'));
    } else if (identity.includes('kit3') || identity.includes('open-source') || identity.includes('open source')) {
      const button = el('button','button','Explore the catalog'); button.type = 'button'; button.addEventListener('click',() => setView('kit3')); actions.append(button); content.append(actions);
    } else {
      const button=el('button','button','Open Infrastructure Creator'); button.type='button'; button.addEventListener('click',()=>{setView('creator'); $('creator-project').value=$('accelerator-project').value; loadCreator();}); actions.append(button); content.append(actions);
    }
    holder.append(block);
  }
  if (!kits.length) holder.append(el('p','empty-note','No accelerators were reported by this runner.'));
}
const creatorUi = {initialized:false,epoch:0,pending:false,detail:null,timer:null,available:false,formOpen:false};
const creatorBusy = run => ['scanning','generating','applying','rolling_back'].includes(run?.status);
// Scan → Review profile → Generate → Review files → Apply; a stopped phase marks its own step.
const creatorSteps = ['Scan','Review profile','Generate','Review files','Apply'];
function renderCreatorStepper(run) {
  const attention = ['needs_input','failed','cancelled','interrupted','rolled_back'].includes(run.status);
  const current = attention ? {scan:0,generate:2,apply:4,rollback:4}[run.phase] ?? 0 : {new:0,scanning:0,review:1,generating:2,preview:3,applying:4,rolling_back:4,complete:5}[run.status] ?? 0;
  $('creator-stepper').replaceChildren(...creatorSteps.map((label,index) => {
    const stepState = index < current ? 'done' : index > current ? 'upcoming' : attention ? 'attention' : creatorBusy(run) || run.status === 'new' ? 'running' : 'current';
    const item = el('li','step'); item.dataset.state = stepState; if (index === current) item.setAttribute('aria-current','step');
    item.append(el('span','step-mark',stepState === 'done' ? '✓' : String(index + 1)),el('span','step-label',label)); return item;
  }));
}
// Once runs exist, the selected run leads and the form for a new run opens on request.
function renderCreatorLayout() {
  const hasRun = Boolean(creatorUi.detail); $('creator-new-run').hidden = !hasRun;
  $('creator-form').hidden = hasRun && !creatorUi.formOpen; $('creator-new-run').setAttribute('aria-expanded',String(!$('creator-form').hidden));
}

function creatorModels() {
  const metadata = providerFor($('creator-provider').value)?.model_options || {};
  const models = metadata.models || []; $('creator-models').replaceChildren();
  for (const model of models) { const option=el('option','',model.label || model.id); option.value=model.id; $('creator-models').append(option); }
  const previous=$('creator-effort').value; const efforts=models.find(model=>model.id===$('creator-model').value.trim())?.efforts || metadata.efforts || [];
  const def=el('option','','Provider default'); def.value=''; $('creator-effort').replaceChildren(def);
  for (const effort of efforts) { const option=el('option','',effort); option.value=effort; $('creator-effort').append(option); }
  $('creator-effort').value=efforts.includes(previous)?previous:'';
  $('creator-model-detail').textContent=metadata.detail || 'Leave model and effort blank for native provider defaults.';
}
function creatorControls() {
  $('creator-runs').disabled=creatorUi.pending;
  $('creator-count').max=String(maxAgentCount()); $('creator-count').disabled=!$('creator-agents').checked || creatorUi.pending;
  $('creator-branch-field').hidden=$('creator-workspace').value!=='worktree'; $('creator-worktree-note').hidden=$('creator-workspace').value!=='worktree';
  const helpers=$('creator-count').valueAsNumber; $('creator-agent-hint').textContent=$('creator-agents').checked && Number.isInteger(helpers) ? `Each scan and generate phase asks for exactly ${helpers} ${helpers===1?'helper':'helpers'}; the log shows how many ran.` : '';
  $('creator-start').disabled=creatorUi.pending || !creatorUi.available || !providerFor($('creator-provider').value)?.available;
  for (const input of $('creator-form').querySelectorAll('input,textarea,select,button')) if (input.id!=='creator-start' && input.id!=='creator-count') input.disabled=creatorUi.pending;
  creatorBudgetControls();
}
  function creatorBudgetControls() {
    const valid=budgetControls('creator-budgets',$('creator-provider').value,creatorUi.pending,null);
    $('creator-start').disabled ||= !valid;
  }
async function openCreator() {
  if (!state.bootstrap) return;
  if (!creatorUi.initialized) {
    setProjectChoices($('creator-project'),state.bootstrap.projects,$('project').value);
    setOptions($('creator-provider'),state.bootstrap.providers,p=>p.name,p=>p.id,$('provider').value);
    $('creator-model').value=selectedModel() || ''; creatorModels(); $('creator-effort').value=$('thinking-effort').value;
    $('creator-agents').checked=$('agents-enabled').checked; $('creator-count').value=$('agent-count').value;
    creatorUi.initialized=true;
  } else setProjectChoices($('creator-project'),state.bootstrap.projects,$('creator-project').value);
  await loadCreator();
}
async function loadCreator(rid=null) {
  clearTimeout(creatorUi.timer); const epoch=++creatorUi.epoch; const project=$('creator-project').value;
  creatorControls(); showError('creator-error','');
  try {
    const [data,git]=await Promise.all([api(`/api/creator?project_id=${encodeURIComponent(project)}`),api(`/api/projects/${encodeURIComponent(project)}/git`)]);
    if (epoch!==creatorUi.epoch || state.view!=='creator') return;
    creatorUi.available=data.available;
    $('creator-git').textContent=git.is_git ? `Current branch: ${git.branch || 'detached HEAD'}${git.dirty?' · uncommitted changes':''}` : 'No Git repository. Use the current project folder.';
    if (!data.available) showError('creator-error','Creator needs filesystem isolation: install bubblewrap on Linux, or run the server on macOS where sandbox-exec is built in.');
    const selected=rid || $('creator-runs').value;
    setOptions($('creator-runs'),data.runs,r=>`${r.created_at} · ${r.operation} · ${r.status}`,r=>r.id,selected);
    if (data.runs.length) await readCreator($('creator-runs').value,epoch);
    else { creatorUi.detail=null; $('creator-detail').hidden=true; }
    renderCreatorLayout();
  } catch(error) { if(epoch===creatorUi.epoch) showError('creator-error',textError(error)); }
  finally { if(epoch===creatorUi.epoch) creatorControls(); }
}
async function readCreator(rid,epoch=++creatorUi.epoch) {
  clearTimeout(creatorUi.timer);
  try {
    const data=await api(`/api/creator/${encodeURIComponent(rid)}`);
    if(epoch!==creatorUi.epoch || state.view!=='creator') return;
    if(creatorUi.detail?.run.id!==rid || creatorUi.detail?.run.revision!==data.run.revision) $('creator-answers').value='';
    // On switching to a different run, mirror its settings into the form so
    // the fields show what a re-launch would apply (and what the user then
    // edits), instead of leaving stale defaults. Not on every poll - that
    // would wipe edits while a phase runs.
    if(creatorUi.detail?.run.id!==data.run.id){const r=data.run;if(r.provider && [...$('creator-provider').options].some(option=>option.value===r.provider)) $('creator-provider').value=r.provider;$('creator-model').value=r.model||'';creatorModels();$('creator-effort').value=r.thinking_effort||'';if(r.thinking_effort && $('creator-effort').value!==r.thinking_effort){const option=el('option','',r.thinking_effort);option.value=r.thinking_effort;$('creator-effort').append(option);$('creator-effort').value=r.thinking_effort;}$('creator-agents').checked=r.agents_enabled===true;$('creator-count').value=Number.isInteger(r.agent_count)?r.agent_count:defaultAgentCount();creatorControls();}
    creatorUi.detail=data; renderCreator();
    if(creatorBusy(data.run)) creatorUi.timer=setTimeout(()=>readCreator(rid,epoch),2000);
  } catch(error) { if(epoch===creatorUi.epoch) showError('creator-action-error',textError(error)); }
}
function renderCreator() {
  const data=creatorUi.detail; if(!data) return; const run=data.run;
  if(data.session) upsert(data.session);
  const selectedRun=[...$('creator-runs').options].find(o=>o.value===run.id); if(selectedRun) selectedRun.textContent=`${run.created_at} · ${run.operation} · ${run.status}`;
  $('creator-detail').hidden=false; $('creator-status').textContent=humanLabel(run.status); renderCreatorStepper(run); renderCreatorLayout();
  $('creator-target').textContent=`${run.target} · ${run.workspace} · branch: ${run.branch || 'none'} · ${run.provider} / ${run.model || 'default model'} / ${run.thinking_effort || 'default effort'} · ${run.agents_enabled?`${run.agent_count} required helpers`:'helpers off'}`;
  $('creator-message').textContent=run.message || 'The phase is queued or running. Artifacts appear as they are written.';
  restoreBudgets('creator-run-budgets',run.budgets,run.id+':'+run.revision);
  updateCreatorBudgetControls();
  const reports=$('creator-reports'); const opened=new Set([...reports.querySelectorAll('details[open]')].map(d=>d.dataset.name)); reports.replaceChildren();
  for(const report of data.reports) { const details=el('details','fleet-report'); details.dataset.name=report.name; details.open=opened.has(report.name) || report.name==='harness-questions.md'; details.append(el('summary','',report.name+(report.truncated?' (truncated)':'')),el('pre','fleet-report-text',report.text)); reports.append(details); }
  $('creator-events').textContent=data.events.map(event=>`[${event.kind}] ${event.text || ''}`).join('\n');
  $('creator-preview').hidden=!data.preview; $('creator-files').replaceChildren();
  if(data.preview?.conflict) $('creator-files').append(el('p','error-text',data.preview.conflict));
  for(const file of data.preview?.files || []) {
    const details=el('details','fleet-report'); details.append(el('summary','',`${file.action} · ${file.path}`));
    details.append(el('pre','fleet-report-text',file.diff || 'No text diff. Compare file sizes and modes in the local staging folder.'));
    if(file.diff_truncated) details.append(el('p','knowledge-note','Diff truncated; inspect the complete file in the staging folder before approving.'));
    $('creator-files').append(details);
    if(file.requires_decision) { const label=el('label','checkbox'); label.style.whiteSpace='normal'; const check=el('input'); check.type='checkbox'; check.value=file.path; check.className='creator-replace'; check.addEventListener('change',()=>{const apply=$('creator-apply'); if(apply) apply.disabled=creatorUi.pending || !data.preview.can_apply || [...document.querySelectorAll('.creator-replace')].some(i=>!i.checked);}); label.append(check,document.createTextNode(`Approve ${file.action}: ${file.path}`)); $('creator-files').append(label); }
  }
  $('creator-corrections').hidden=creatorBusy(run) || run.status==='complete' || run.recovery_available;
  const actions=$('creator-actions'); actions.replaceChildren();
  function button(action,label,primary=false) { const b=el('button',`button${primary?' primary':''}`,label); b.type='button'; b.disabled=creatorUi.pending; if(['generate','revise','rescan'].includes(action)) { b.dataset.launch='true'; b.disabled ||= creatorBudgetsDirty(); } b.addEventListener('click',()=>creatorAction(action)); actions.append(b); return b; }
  if(run.recovery_available) button('rollback','Recover interrupted publication',true);
  else if(run.status==='review') { button('generate','Approve profile & generate',true); button('revise','Revise scan'); }
  else if(run.status==='preview') { const apply=button('apply','Apply reviewed files',true); apply.id='creator-apply'; apply.disabled=creatorUi.pending || !data.preview.can_apply || data.preview.files.some(file=>file.requires_decision); button('revise','Revise generated files'); }
  else if(['needs_input','failed','cancelled','interrupted','rolled_back'].includes(run.status)) button('revise','Continue with answers / corrections',true);
  if(!creatorBusy(run) && !run.recovery_available && run.status!=='complete') button('rescan','Scan again for review');
  if(['scanning','generating'].includes(run.status)) button('cancel','Cancel phase');
  updateCreatorBudgetControls();
  if(data.session) { const result=el('button','button','View results & run usage'); result.type='button'; result.addEventListener('click',async()=>{await selectSession(data.session.id); setView('changes');}); actions.append(result); }
  if(run.status==='complete') { const update=el('button','button','Prepare next update'); update.type='button'; update.addEventListener('click',()=>{creatorUi.formOpen=true; renderCreatorLayout(); $('creator-operation').value='update'; $('creator-goal').focus(); $('creator-form').scrollIntoView({behavior:scrollMotion()});}); actions.append(update); }
}
function creatorBudgetsDirty() { return Boolean(creatorUi.detail && JSON.stringify(readBudgets('creator-run-budgets'))!==JSON.stringify(creatorUi.detail.run.budgets)); }
function updateCreatorBudgetControls() {
  const data=creatorUi.detail; if(!data) return;
  const locked=creatorUi.pending || creatorBusy(data.run) || data.run.recovery_available;
  const valid=budgetControls('creator-run-budgets',data.run.provider,locked,data.session?.budget_usage);
  $('creator-run-budgets-save').disabled=locked || !valid || !creatorBudgetsDirty();
  for(const b of $('creator-actions').querySelectorAll('[data-launch]')) b.disabled=creatorUi.pending || !valid || creatorBudgetsDirty();
}
for(const key of ['usd','tokens','seconds']) { $('creator-budgets-'+key).addEventListener('input',creatorControls); $('creator-run-budgets-'+key).addEventListener('input',updateCreatorBudgetControls); }
async function creatorAction(action) {
  if(creatorUi.pending || !creatorUi.detail) return;
  const data=creatorUi.detail; const epoch=++creatorUi.epoch; clearTimeout(creatorUi.timer); creatorUi.pending=true;
  const body={action,revision:data.run.revision};
  if(action==='budgets') body.budgets=readBudgets('creator-run-budgets');
  if(['revise','rescan'].includes(action)) body.answers=$('creator-answers').value;
  if(['revise','rescan','generate'].includes(action)) body.routing={model:$('creator-model').value.trim() || null,thinking_effort:$('creator-effort').value || null,agents_enabled:$('creator-agents').checked,agent_count:$('creator-count').valueAsNumber};
  if(action==='apply') { body.preview_id=data.preview.preview_id; body.replace=[...document.querySelectorAll('.creator-replace:checked')].map(i=>i.value); }
  for(const b of $('creator-actions').querySelectorAll('button')) b.disabled=true;
  creatorControls(); showError('creator-action-error','');
  try { const result=await api(`/api/creator/${data.run.id}`,{method:'POST',body}); if(epoch===creatorUi.epoch) { creatorUi.detail=result; if(action!=='budgets') $('creator-answers').value=''; } }
  catch(error) { if(epoch===creatorUi.epoch) showError('creator-action-error',textError(error)); }
  finally { creatorUi.pending=false; creatorControls(); if(epoch===creatorUi.epoch) { renderCreator(); if(creatorBusy(creatorUi.detail?.run)) creatorUi.timer=setTimeout(()=>readCreator(data.run.id,epoch),1000); } }
}
$('creator-form').addEventListener('submit',async event=>{
  event.preventDefault(); if(creatorUi.pending) return; clearTimeout(creatorUi.timer); const epoch=++creatorUi.epoch;
  const body={budgets:readBudgets('creator-budgets'),project_id:$('creator-project').value,provider:$('creator-provider').value,model:$('creator-model').value.trim() || null,thinking_effort:$('creator-effort').value || null,operation:$('creator-operation').value,workspace:$('creator-workspace').value,worktree_branch:$('creator-workspace').value==='worktree'?$('creator-branch').value.trim():'',agents_enabled:$('creator-agents').checked,agent_count:$('creator-count').valueAsNumber,goal:$('creator-goal').value,tools:[...document.querySelectorAll('[name="creator-tool"]:checked')].map(i=>i.value)};
  creatorUi.pending=true; creatorControls(); showError('creator-error','');
  try { const data=await api('/api/creator',{method:'POST',body}); if(epoch===creatorUi.epoch) { creatorUi.detail=data; creatorUi.formOpen=false; renderCreator(); await loadCreator(data.run.id); $('creator-detail').scrollIntoView({behavior:scrollMotion(),block:'start'}); refreshSessions(); } }
  catch(error) { if(epoch===creatorUi.epoch) showError('creator-error',textError(error)); }
  finally { creatorUi.pending=false; creatorControls(); if(creatorUi.detail) renderCreator(); }
});
$('creator-provider').addEventListener('change',()=>{$('creator-model').value='';creatorModels();creatorControls();});
$('creator-model').addEventListener('input',creatorModels);
$('creator-count').addEventListener('input',creatorControls);
$('creator-agents').addEventListener('change',creatorControls); $('creator-workspace').addEventListener('change',creatorControls);
$('creator-project').addEventListener('change',()=>{creatorUi.detail=null;$('creator-detail').hidden=true;$('creator-runs').replaceChildren();loadCreator();});
$('creator-runs').addEventListener('change',()=>readCreator($('creator-runs').value));
$('creator-new-run').addEventListener('click',()=>{ creatorUi.formOpen=!creatorUi.formOpen; renderCreatorLayout(); if(creatorUi.formOpen) $('creator-project').focus(); }); $('creator-refresh').addEventListener('click',()=>loadCreator());
bootstrap();
