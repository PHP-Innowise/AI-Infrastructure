// Knowledge: project context, Memory bank, Project Brain and the Brain task linked to a session.
// Classic script loaded in order by index.html; top-level names are shared with the other app-*.js files.
'use strict';
function bytesLabel(bytes) { if (!Number.isFinite(bytes)) return '—'; return bytes < 1024 ? `${bytes}\u00a0B` : bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)}\u00a0KB` : `${(bytes / (1024 * 1024)).toFixed(1)}\u00a0MB`; }
async function loadContext() {
  const id = $('context-project').value; if (!id || !state.bootstrap) return; const epoch = ++state.contextEpoch; $('refresh-context').disabled = true; $('context-summary').textContent = 'Reading project context…'; $('context-files').replaceChildren(); showError('context-error','');
  try {
    const data = await api(`/api/projects/${encodeURIComponent(id)}/context`); if (epoch !== state.contextEpoch) return;
    const files = data.files || []; $('context-summary').replaceChildren(el('span','',data.context_available ? `${files.filter(file => file.exists).length} of ${files.length} files present` : 'No project context files were found.'));
    if (!files.length) $('context-files').append(el('p','empty-note','No context files were reported for this project.'));
    for (const file of files) { const row = el('div','file-row'); row.append(el('span','file-path',file.path),el('span',file.exists ? 'file-status' : 'file-status missing',file.exists ? 'Present' : 'Missing'),el('span','file-size',file.exists ? bytesLabel(file.bytes) : '—')); $('context-files').append(row); }
  } catch (error) { if (epoch === state.contextEpoch) { $('context-summary').textContent = ''; showError('context-error',textError(error)); } }
  finally { if (epoch === state.contextEpoch) $('refresh-context').disabled = false; }
}
$('context-project').addEventListener('change',loadContext); $('refresh-context').addEventListener('click',loadContext);
function cancelMemoryRequests() { memoryState.listEpoch++; memoryState.fileEpoch++; memoryState.listController?.abort(); memoryState.fileController?.abort(); memoryState.listController = null; memoryState.fileController = null; }
function clearMemoryContent(message) { $('memory-detail-header').hidden = true; $('memory-metadata').hidden = true; $('memory-content').hidden = true; $('memory-content').textContent = ''; $('memory-content-notice').hidden = true; showError('memory-file-error',''); $('memory-message').textContent = message; $('memory-message').hidden = !message; }
function memoryKind(kind) { return ({chunk:'Memory chunk',local:'Local note',document:'Document'})[kind] || kind; }
function renderMemoryFiles() {
  const query = $('memory-filter').value.trim().toLowerCase();
  const visible = memoryState.entries.filter(entry => [entry.title,entry.path,entry.status].join(' ').toLowerCase().includes(query));
  $('memory-files').replaceChildren();
  for (const entry of visible) {
    const button = el('button','memory-entry'); button.type = 'button'; if (entry.path === memoryState.path) button.setAttribute('aria-current','true');
    button.append(el('span','memory-entry-title',entry.title || entry.path),el('span','memory-entry-path',entry.path),el('span','memory-entry-meta',[memoryKind(entry.kind),entry.status,entry.type,bytesLabel(entry.bytes)].filter(Boolean).join(' · ')));
    button.addEventListener('click',() => selectMemoryEntry(entry.path,true)); $('memory-files').append(button);
  }
  $('memory-count').textContent = `${visible.length}${query ? ` of ${memoryState.entries.length}` : ''} ${(query ? memoryState.entries.length : visible.length) === 1 ? 'file' : 'files'}${memoryState.truncated ? ' · listing limited' : ''}`;
  if (!visible.length) $('memory-files').append(el('p','empty-note',query ? 'No matching files. Try another title, path, or status.' : memoryState.bankId ? 'This bank has no readable Markdown files.' : 'No memory bank found in this project.'));
}
async function loadMemory({bank = memoryState.bankId,preserve = true} = {}) {
  const projectId = $('memory-project').value; const projectChanged = projectId !== memoryState.projectId;
  if (projectChanged) { bank = null; preserve = false; $('memory-filter').value = ''; }
  const previousPath = preserve && bank === memoryState.bankId ? memoryState.path : null;
  cancelKnowledgeRead('memory'); knowledgeState.memory.meta = null; knowledgeState.memory.error = ''; updateKnowledgeControls();
  cancelMemoryRequests(); const epoch = memoryState.listEpoch; const controller = new AbortController(); memoryState.listController = controller;
  memoryState.projectId = projectId; memoryState.bankId = bank; memoryState.path = previousPath; memoryState.entries = []; memoryState.truncated = false;
  $('refresh-memory').disabled = true; $('memory-filter').disabled = true; $('memory-count').textContent = 'Loading memory bank…'; $('memory-bank-path').textContent = ''; $('memory-files').replaceChildren(); $('memory-list-notice').hidden = true; showError('memory-error',''); clearMemoryContent('Loading memory bank…');
  if (projectChanged) { memoryState.bankId = null; memoryState.banks = []; setOptions($('memory-bank'),[],item => item.name,item => item.id); $('memory-bank').disabled = true; }
  if (!state.bootstrap || !projectId) { $('memory-count').textContent = ''; clearMemoryContent('No registered project is available.'); $('refresh-memory').disabled = true; memoryState.listController = null; return; }
  try {
    const endpoint = `/api/projects/${encodeURIComponent(projectId)}/memory`; let data;
    try { data = await api(endpoint + (bank ? `?bank=${encodeURIComponent(bank)}` : ''),{signal:controller.signal}); }
    catch (error) { if (!bank || ![400,404].includes(error.status) || epoch !== memoryState.listEpoch) throw error; data = await api(endpoint,{signal:controller.signal}); }
    if (epoch !== memoryState.listEpoch) return;
    memoryState.banks = data.banks || []; memoryState.bankId = data.bank_id || null; memoryState.entries = data.entries || []; memoryState.truncated = Boolean(data.truncated);
    setOptions($('memory-bank'),memoryState.banks,item => item.name,item => item.id,memoryState.bankId);
    $('memory-bank').disabled = !memoryState.banks.length; $('memory-filter').disabled = !memoryState.entries.length;
    $('memory-bank-path').textContent = memoryState.banks.find(item => item.id === memoryState.bankId)?.path || '';
    $('memory-list-notice').hidden = !memoryState.truncated;
    const selected = memoryState.entries.find(entry => entry.path === previousPath && memoryState.bankId === bank) || memoryState.entries.find(entry => entry.kind === 'chunk') || memoryState.entries[0];
    renderMemoryFiles(); loadKnowledgeMetadata('memory');
    if (selected) selectMemoryEntry(selected.path);
    else clearMemoryContent(memoryState.bankId ? 'This bank has no readable Markdown files.' : 'No memory bank found in this project.');
  } catch (error) {
    if (error.name === 'AbortError' || epoch !== memoryState.listEpoch) return;
    $('memory-count').textContent = ''; showError('memory-error',textError(error)); clearMemoryContent('Memory could not be loaded. Refresh to try again.');
  } finally { if (epoch === memoryState.listEpoch) { memoryState.listController = null; $('refresh-memory').disabled = false; updateKnowledgeControls(); } }
}
async function selectMemoryEntry(path, scrollToContent = false) {
  const entry = memoryState.entries.find(item => item.path === path); if (!entry || !memoryState.bankId) return;
  memoryState.fileController?.abort(); const controller = new AbortController(); memoryState.fileController = controller; const epoch = ++memoryState.fileEpoch; const bankEpoch = memoryState.listEpoch;
  const projectId = memoryState.projectId; const bankId = memoryState.bankId; memoryState.path = path; renderMemoryFiles(); clearMemoryContent('Loading file…');
  $('memory-detail-header').hidden = false; $('memory-title').textContent = entry.title || path; $('memory-path').textContent = path; $('memory-meta').textContent = [memoryKind(entry.kind),entry.status,bytesLabel(entry.bytes)].filter(Boolean).join(' · ');
  try {
    const query = new URLSearchParams({bank:bankId,path}); const data = await api(`/api/projects/${encodeURIComponent(projectId)}/memory?${query}`,{signal:controller.signal});
    if (epoch !== memoryState.fileEpoch || bankEpoch !== memoryState.listEpoch) return;
    $('memory-title').textContent = data.title || data.path; $('memory-path').textContent = data.path; $('memory-meta').textContent = [memoryKind(data.kind),data.id,data.status,bytesLabel(data.bytes)].filter(Boolean).join(' · '); Object.assign(entry,data); updateKnowledgeControls();
    const content = typeof data.content === 'string' ? data.content : '', front = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(content);
    $('memory-metadata').hidden = !front; $('memory-metadata-content').textContent = front ? front[1] : '';
    $('memory-content').textContent = front ? content.slice(front[0].length).replace(/^\s+/,'') : content; $('memory-content').hidden = false; $('memory-content').scrollTop = 0; $('memory-message').hidden = true; $('memory-content-notice').hidden = !data.truncated;
    if (scrollToContent && window.matchMedia('(max-width:760px)').matches) $('memory-details').scrollIntoView({block:'nearest',behavior:'instant'});
  } catch (error) { if (error.name !== 'AbortError' && epoch === memoryState.fileEpoch && bankEpoch === memoryState.listEpoch) { $('memory-message').hidden = true; showError('memory-file-error',textError(error)); } }
  finally { if (epoch === memoryState.fileEpoch) memoryState.fileController = null; }
}
$('memory-project').addEventListener('change',() => loadMemory());
$('memory-bank').addEventListener('change',() => { $('memory-filter').value = ''; loadMemory({bank:$('memory-bank').value,preserve:false}); });
$('memory-filter').addEventListener('input',renderMemoryFiles);
$('refresh-memory').addEventListener('click',() => loadMemory());
const knowledgeActions = {
  status:{label:'Status',note:'Read local index counts. Like every context command, this opens or creates the local SQLite database.',fields:[]},
  index:{label:'Index',note:'Rebuild the local context index from project files. Shared source files are unchanged.',fields:[]},
  validate:{label:'Validate',note:'Validate active and archived Project Brain records using the context CLI.',fields:[]},
  'bank-audit':{label:'Audit memory bank',note:'Report changed source references and chunks overdue for review. This does not verify the meaning of a chunk.',fields:[]},
  'reindex-bank':{label:'Reindex memory bank',note:'Regenerate the Git-tracked memory-bank/INDEX.md from durable memory chunks. This changes a shared file.',fields:[]},
  compact:{label:'Compact / archive',note:'Move terminal and superseded Project Brain records into the archive. Active records remain in place; archived records remain available in Archive.',fields:[]},
  export:{label:'Export ZIP',note:'Create a downloadable export from the selected project knowledge root.',fields:[['include_archive','Include archive','checkbox'],['include_superseded','Include superseded records','checkbox']]},
  start:{label:'Start task',note:'Create or start a working task in the selected Project Brain.',fields:[['task_id','Task ID','text',true,'TASK-123'],['goal','Goal','textarea',true,'What should this task achieve?'],['files','Files','lines',false,'One project-relative path per line'],['sources','Sources','lines',false,'One source reference per line']]},
  'brain-create':{label:'Create record',note:'Create a finding, bug, incident, decision, or event. Use Start task for a task record.',fields:[['record_type','Record type','select',true,[['finding','Finding'],['bug','Bug'],['incident','Incident'],['decision','Decision'],['event','Event']]],['external_id','External ID','text',true,'FINDING-123'],['title','Title','text',true],['goal','Goal (optional)','textarea'],['files','Files','lines',false,'One project-relative path per line'],['sources','Sources','lines',false,'One source reference per line']]},
  'brain-update':{label:'Update record',note:'Progress replaces the current progress text; next steps are appended. Revision must match the current record. Refresh and inspect the record if it changed.',fields:[['record_id','Record ID','text',true],['revision','Current revision','number',true],['progress','Replace progress (optional)','textarea'],['next_steps','Append next steps','lines',false,'One next step per line'],['phase','Task phase (optional)','select',false,[['','Keep current phase'],['understanding','Understanding'],['planning','Planning'],['implementation','Implementation'],['verification','Verification'],['finalization','Finalization']]],['transition','Status transition (optional)','text',false,'Target status allowed by this record type'],['reason','Reason (optional)','textarea']]},
  complete:{label:'Complete task',note:'Record the task outcome and verification evidence. The current revision is required.',fields:[['task_id','Task ID','text',true],['revision','Current revision','number',true],['outcome','Outcome','textarea',true],['verification','Verification','lines',false,'One verification result per line'],['sources','Sources','lines',false,'One source reference per line']]},
  'bank-reverify':{label:'Re-attest selected chunk',note:'By running this action, you attest that you checked this chunk against its current sources. The CLI records your attestation; it does not check the meaning for you.',fields:[['memory_id','Memory ID','text',true],['review_after','Next review date (optional)','date',false]]},
  'bank-retire':{label:'Retire selected chunk',note:'Close this chunk’s validity period. Without a replacement it is archived; with a replacement it is marked superseded.',fields:[['memory_id','Memory ID','text',true],['valid_to','Valid through','date',true],['superseded_by','Replacement memory ID (optional)','text'],['reason','Reason (optional)','textarea']]}
};
const knowledgeActionLists = {brain:['start','brain-create','brain-update','complete','compact','validate','export'],memory:['status','index','validate','bank-audit','reindex-bank','export','bank-reverify','bank-retire']};
const knowledgeViewer = scope => scope === 'brain' ? brainState : memoryState;
function knowledgeTarget(scope) { const viewer = knowledgeViewer(scope); return {project_id:$(`${scope}-project`).value,bank:viewer.bankId}; }
function knowledgeTargetKey(target) { return JSON.stringify([target.project_id,target.bank]); }
function knowledgeTargetLabel(scope,target) { const viewer = knowledgeViewer(scope); const bank = viewer.banks.find(item => item.id === target.bank); return `${projectFor(target.project_id)?.name || target.project_id} · ${bank?.path || bank?.name || target.bank || 'default knowledge root'}`; }
function makeKnowledgeField(scope,definition) {
  const [name,label,type,required=false,hint] = definition; const wrapper = el('label',type === 'checkbox' ? 'checkbox' : `field${['textarea','lines'].includes(type) ? ' wide' : ''}`);
  const input = el(type === 'select' ? 'select' : ['textarea','lines'].includes(type) ? 'textarea' : 'input'); input.id = `${scope}-op-${name}`; input.name = name; input.required = required;
  if (type === 'select') for (const [value,text] of hint || []) { const option = el('option','',text); option.value = value; input.append(option); }
  else if (type === 'textarea' || type === 'lines') { input.rows = 3; input.maxLength = 12000; }
  else { input.type = type; if (type === 'number') { input.min = '1'; input.max = '2147483647'; input.step = '1'; } else if (type === 'text') input.maxLength = ['record_id','memory_id','task_id','external_id','superseded_by'].includes(name) ? 256 : 2000; }
  if (typeof hint === 'string') input.placeholder = hint;
  if (type === 'checkbox') wrapper.append(input,document.createTextNode(label)); else wrapper.append(el('span','',label),input);
  return wrapper;
}
function buildKnowledgeTools(scope) {
  const panel = el('details','knowledge-tools'); panel.id = `${scope}-knowledge-panel`; panel.append(el('summary','',scope === 'brain' ? 'Record tools & maintenance' : 'Search & maintenance'));
  const runtime = el('p','knowledge-note','Checking context runtime…'); runtime.id = `${scope}-knowledge-runtime`; runtime.setAttribute('role','status'); panel.append(runtime);
  if (scope === 'memory') {
    const search = el('form','knowledge-search'); search.id = 'memory-search-form'; const grid = el('div','knowledge-form-grid');
    const label = el('label','field'); label.append(el('span','','Search indexed content')); const input = el('input'); input.id = 'memory-search-query'; input.type = 'search'; input.required = true; input.maxLength = 2000; input.placeholder = 'Search words or a topic…'; label.append(input); grid.append(label);
    const layer = el('label','field'); layer.append(el('span','','Memory layer')); const select = el('select'); select.id = 'memory-search-layer'; for (const [id,name] of [['','All layers'],['procedural','Procedural'],['semantic','Semantic'],['episodic','Episodic']]) { const option = el('option','',name); option.value = id; select.append(option); } layer.append(select); grid.append(layer); search.append(grid);
    search.append(el('p','knowledge-note','Search reads the existing local index, including indexed project context. Run Index below if results are missing or stale.'));
    const button = el('button','button','Search index'); button.type = 'submit'; button.id = 'memory-search-button'; search.append(button); const results = el('section','knowledge-output'); results.id = 'memory-search-results'; results.hidden = true; results.setAttribute('aria-label','Indexed search results'); search.append(results);
    search.addEventListener('submit',event => { event.preventDefault(); if (!search.reportValidity()) return; const query = input.value.trim(); if (query) runKnowledgeAction('memory','search',{query,...(select.value ? {layer:select.value} : {})}); }); panel.append(search);
  }
  const form = el('form'); form.id = `${scope}-knowledge-form`; const selectLabel = el('label','field'); selectLabel.append(el('span','','Operation')); const select = el('select'); select.id = `${scope}-knowledge-action`;
  for (const action of knowledgeActionLists[scope]) { const option = el('option','',knowledgeActions[action].label); option.value = action; select.append(option); } selectLabel.append(select); form.append(selectLabel);
  const note = el('p','knowledge-note'); note.id = `${scope}-knowledge-note`; form.append(note); const fields = el('div','knowledge-form-grid'); fields.id = `${scope}-knowledge-fields`; form.append(fields);
  const actions = el('div','knowledge-actions'); const use = el('button','button',scope === 'brain' ? 'Use selected record' : 'Use selected chunk'); use.type = 'button'; use.id = `${scope}-use-selected`; use.addEventListener('click',() => fillKnowledgeSelection(scope)); const submit = el('button','button'); submit.type = 'submit'; submit.id = `${scope}-knowledge-submit`; actions.append(use,submit); form.append(actions);
  const error = el('p','error-text'); error.id = `${scope}-knowledge-error`; error.hidden = true; error.setAttribute('role','alert'); form.append(error);
  select.addEventListener('change',() => renderKnowledgeFields(scope)); form.addEventListener('submit',event => { event.preventDefault(); submitKnowledgeForm(scope); }); panel.append(form);
  const output = el('section','knowledge-output'); output.id = `${scope}-knowledge-output`; output.hidden = true; output.setAttribute('aria-label','Operation result'); panel.append(output); $(`${scope}-tools`).append(panel); renderKnowledgeFields(scope);
}
function renderKnowledgeFields(scope) {
  const action = $(`${scope}-knowledge-action`).value; const definition = knowledgeActions[action]; const fields = $(`${scope}-knowledge-fields`); fields.replaceChildren();
  for (const field of definition.fields) fields.append(makeKnowledgeField(scope,field)); $(`${scope}-knowledge-note`).textContent = definition.note; showError(`${scope}-knowledge-error`,''); updateKnowledgeControls();
}
function cancelKnowledgeRead(scope) { const item = knowledgeState[scope]; item.epoch++; item.controller?.abort(); item.controller = null; item.pending = false; }
async function loadKnowledgeMetadata(scope) {
  const item = knowledgeState[scope]; const target = knowledgeTarget(scope); const key = knowledgeTargetKey(target); cancelKnowledgeRead(scope); const epoch = item.epoch;
  if (item.targetKey !== key) { item.meta = null; item.targetKey = key; renderKnowledgeFields(scope); }
  if (!state.bootstrap || !target.project_id || !target.bank) { item.meta = null; updateKnowledgeControls(); return; }
  const controller = new AbortController(); item.controller = controller; item.pending = true; item.error = ''; updateKnowledgeControls();
  try {
    const data = await api(`/api/projects/${encodeURIComponent(target.project_id)}/knowledge?bank=${encodeURIComponent(target.bank)}`,{signal:controller.signal});
    if (epoch !== item.epoch || knowledgeTargetKey(knowledgeTarget(scope)) !== key) return; item.meta = data;
  } catch (error) { if (error.name !== 'AbortError' && epoch === item.epoch) { item.meta = null; item.error = textError(error); } }
  finally { if (epoch === item.epoch) { item.pending = false; item.controller = null; updateKnowledgeControls(); } }
}
function knowledgeAvailable(scope) { const item = knowledgeState[scope]; const target = knowledgeTarget(scope); return Boolean(state.bootstrap && !state.authFailed && !knowledgeState.pending && !item.pending && item.meta?.runtime_available && target.bank && item.targetKey === knowledgeTargetKey(target) && knowledgeViewer(scope).projectId === target.project_id); }
function knowledgeActionAvailable(scope,action) { return knowledgeAvailable(scope) && (!['start','brain-create','brain-update','complete'].includes(action) || knowledgeState[scope].meta?.mode === 'governed'); }
function updateKnowledgeControls() {
  for (const scope of ['memory','brain']) {
    if (!$(`${scope}-knowledge-form`)) continue; const item = knowledgeState[scope]; const viewer = knowledgeViewer(scope); const ready = knowledgeAvailable(scope); const action = $(`${scope}-knowledge-action`).value;
    const canRun = knowledgeActionAvailable(scope,action);
    for (const input of $(`${scope}-knowledge-form`).querySelectorAll('input,textarea,select,button')) input.disabled = !canRun;
    $(`${scope}-knowledge-action`).disabled = !ready;
    $(`${scope}-knowledge-note`).textContent = `${knowledgeActions[action]?.note || ''}${ready && !canRun ? ' Editing Project Brain records requires governed mode; this root uses lightweight mode. Browsing, validation, and export remain available.' : ''}`;
    $(`${scope}-project`).disabled = !state.bootstrap || Boolean(knowledgeState.pending);
    $(`${scope}-bank`).disabled = !viewer.banks.length || Boolean(viewer.listController) || Boolean(knowledgeState.pending);
    $(`refresh-${scope}`).disabled = !state.bootstrap || Boolean(viewer.listController) || Boolean(knowledgeState.pending);
    const entry = scope === 'brain' ? viewer.selected : viewer.entries.find(entry => entry.path === viewer.path);
    const usable = scope === 'brain' ? action === 'brain-update' && entry?.id && Number.isInteger(entry.revision) || action === 'complete' && entry?.type === 'task' && (entry.external_id || entry.id) && Number.isInteger(entry.revision) : entry?.kind === 'chunk';
    $(`${scope}-use-selected`).hidden = scope === 'brain' ? !['brain-update','complete'].includes(action) : !['bank-reverify','bank-retire'].includes(action);
    $(`${scope}-use-selected`).disabled = !canRun || !usable;
    $(`${scope}-knowledge-submit`).textContent = knowledgeState.pending?.scope === scope && knowledgeState.pending.action !== 'search' ? 'Running…' : `Run ${knowledgeActions[action]?.label || 'operation'}`;
    const sameTarget = item.targetKey === knowledgeTargetKey(knowledgeTarget(scope));
    $(`${scope}-knowledge-runtime`).textContent = item.pending ? 'Checking context runtime…' : !sameTarget || !item.meta ? item.error || 'No context runtime is available for this selection. File browsing remains available.' : item.meta.runtime_available ? `${item.meta.mode ? `${item.meta.mode} mode · ` : ''}${item.meta.root || 'Selected knowledge root'}. Commands use the project’s context CLI and may take up to a minute.` : 'Context runtime is unavailable for this root. File browsing remains available.';
    if (scope === 'memory') { for (const input of $('memory-search-form').querySelectorAll('input,select,button')) input.disabled = !ready; $('memory-search-button').textContent = knowledgeState.pending?.action === 'search' ? 'Searching…' : 'Search index'; }
  }
}
function fillKnowledgeSelection(scope) {
  if (!knowledgeActionAvailable(scope,$(`${scope}-knowledge-action`).value)) return; const viewer = knowledgeViewer(scope); const entry = scope === 'brain' ? viewer.selected : viewer.entries.find(entry => entry.path === viewer.path); if (!entry) return;
  const action = $(`${scope}-knowledge-action`).value;
  if (scope === 'brain') { const field = action === 'complete' ? 'task_id' : 'record_id'; if ($(`brain-op-${field}`)) $(`brain-op-${field}`).value = action === 'complete' ? entry.external_id || entry.id || '' : entry.id || ''; if ($('brain-op-revision')) $('brain-op-revision').value = Number.isInteger(entry.revision) ? entry.revision : ''; }
  else if ($('memory-op-memory_id')) $('memory-op-memory_id').value = entry.id || entry.path.split('/').pop().replace(/\.md$/i,'');
  showError(`${scope}-knowledge-error`,'');
  const edit = $(scope === 'brain' ? `brain-op-${action === 'brain-update' ? 'progress' : 'task_id'}` : 'memory-op-memory_id');
  if (edit) { edit.scrollIntoView({block:'center',behavior:'instant'}); edit.focus({preventScroll:true}); }
}
function submitKnowledgeForm(scope) {
  if (!knowledgeActionAvailable(scope,$(`${scope}-knowledge-action`).value) || !$(`${scope}-knowledge-form`).reportValidity()) return; const action = $(`${scope}-knowledge-action`).value; const body = {}; let error = '';
  for (const [name,label,type,required] of knowledgeActions[action].fields) {
    const input = $(`${scope}-op-${name}`); const value = type === 'checkbox' ? input.checked : input.value.trim();
    if (required && !value) { error = `${label} is required.`; break; }
    if (type === 'number') { const number = Number(value); if (!Number.isSafeInteger(number) || number < 1 || number > 2147483647) { error = `${label} must be a whole number from 1 to 2147483647.`; break; } body[name] = number; }
    else if (type === 'checkbox') body[name] = value;
    else if (type === 'lines') { body[name] = value.split(/\r?\n/).map(line => line.trim()).filter(Boolean); if (body[name].length > 50) { error = `Use at most 50 lines for ${label.toLowerCase()}.`; break; } }
    else if (value) body[name] = value;
  }
  if (!error && action === 'brain-update' && !body.progress && !body.next_steps?.length && !body.phase && !body.transition) error = 'Enter progress, a next step, a phase, or a status transition to update.';
  showError(`${scope}-knowledge-error`,error); if (!error) runKnowledgeAction(scope,action,body);
}
function renderKnowledgeSearch(output,result) {
  if (!result || typeof result !== 'object' || !Array.isArray(result.documents) && !Array.isArray(result.episodes)) { output.append(el('pre','knowledge-result',typeof result === 'string' ? result : JSON.stringify(result,null,2))); return; }
  const documents = result.documents || []; const episodes = result.episodes || [];
  if (!documents.length && !episodes.length) output.append(el('p','knowledge-note','No matches in the current index. Try different words or run Index if files are missing or stale.'));
  for (const doc of documents) { const article = el('article','knowledge-search-result'); article.append(el('h4','',doc.title || doc.path || 'Indexed document'),el('p','memory-detail-path',[doc.path,doc.layer,doc.kind].filter(Boolean).join(' · ')),el('p','',doc.snippet || '')); output.append(article); }
  for (const episode of episodes) { const article = el('article','knowledge-search-result'); article.append(el('h4','',episode.summary || `Episode ${episode.id}`),el('p','',episode.outcome || '')); for (const key of ['files','sources','verification']) if (episode[key]?.length) article.append(el('p','memory-detail-path',`${key}: ${Array.isArray(episode[key]) ? episode[key].join('\n') : episode[key]}`)); output.append(article); }
}
async function runKnowledgeAction(scope,action,fields) {
  if (!knowledgeActionAvailable(scope,action)) return; const target = knowledgeTarget(scope); const targetKey = knowledgeTargetKey(target); const targetLabel = knowledgeTargetLabel(scope,target); const output = $(action === 'search' ? 'memory-search-results' : `${scope}-knowledge-output`);
  knowledgeState.pending = {scope,action,target}; output.hidden = false; output.replaceChildren(el('h3','',action === 'search' ? 'Indexed search' : knowledgeActions[action].label),el('p','knowledge-note',targetLabel));
  const status = el('p','knowledge-note','Running… You can visit another section; this command will continue.'); status.setAttribute('role','status'); output.append(status); showError(`${scope}-knowledge-error`,''); updateKnowledgeControls();
  try {
    const data = await api(`/api/projects/${encodeURIComponent(target.project_id)}/knowledge`,{method:'POST',body:{bank:target.bank,action,...fields}});
    status.textContent = data.ok ? 'Command complete.' : 'Command failed.';
    if (data.error) { const error = el('p','error-text',typeof data.error === 'string' ? data.error : JSON.stringify(data.error)); error.setAttribute('role','alert'); output.append(error); }
    if (action === 'search' && data.ok) renderKnowledgeSearch(output,data.result);
    else if (data.result !== undefined && data.result !== null) output.append(el('pre','knowledge-result',typeof data.result === 'string' ? data.result : JSON.stringify(data.result,null,2)));
    if (data.ok && typeof data.download_id === 'string' && data.download_id) { const link = el('a','button','Download ZIP'); link.href = `/api/knowledge/exports/${encodeURIComponent(data.download_id)}`; link.download = ''; output.append(link); }
    if (data.ok && action !== 'search') {
      for (const view of ['brain','memory']) if (state.view === view && knowledgeTargetKey(knowledgeTarget(view)) === targetKey) { if (view === 'brain') await loadBrain(); else await loadMemory(); }
    }
  } catch (error) { status.textContent = 'Command did not complete successfully.'; const note = el('p','error-text',`${textError(error)}${error.status === 0 ? ' The command may have run. Refresh the selected records or check Status before retrying.' : ''}`); note.setAttribute('role','alert'); output.append(note); }
  finally { knowledgeState.pending = null; updateKnowledgeControls(); }
}
function cancelBrainRequests() { brainState.listEpoch++; brainState.fileEpoch++; brainState.listController?.abort(); brainState.fileController?.abort(); brainState.listController = null; brainState.fileController = null; }
function clearBrainContent(message) { brainState.selected = null; $('brain-detail-header').hidden = true; $('brain-metadata').hidden = true; $('brain-content').hidden = true; $('brain-content').textContent = ''; $('brain-content-notice').hidden = true; showError('brain-file-error',''); $('brain-message').textContent = message; $('brain-message').hidden = !message; updateKnowledgeControls(); }
function brainCategory(entry) { const category = entry.category || (entry.path || '').split('/').slice(0,-1).join('/'); if (category.startsWith('archive/')) return 'archive'; if (category.startsWith('dynamic/') || category.startsWith('control/')) return category.split('/')[1]; return category || 'documents'; }
function brainCategoryLabel(category) { return ({tasks:'Tasks',task:'Tasks',findings:'Findings',finding:'Findings',bugs:'Bugs',bug:'Bugs',incidents:'Incidents',incident:'Incidents',decisions:'Decisions',decision:'Decisions',events:'Events',event:'Events',handoffs:'Handoffs',handoff:'Handoffs',archive:'Archive',documents:'Documents',control:'Control'})[category] || category.replace(/[-_]/g,' ').replace(/^./,letter => letter.toUpperCase()); }
function renderBrainCategories() { const previous = $('brain-category').value; const categories = [...new Set(['tasks','findings','decisions','handoffs','archive',...brainState.entries.map(brainCategory)])]; setOptions($('brain-category'),['',...categories],item => item ? brainCategoryLabel(item) : 'All categories',item => item,previous); }
function renderBrainFiles() {
  const query = $('brain-filter').value.trim().toLowerCase(); const category = $('brain-category').value;
  const entries = brainState.entries.filter(entry => (!category || brainCategory(entry) === category) && [entry.title,entry.path,entry.id,entry.external_id,entry.status,entry.owner,entry.type].join(' ').toLowerCase().includes(query)); $('brain-files').replaceChildren();
  for (const entry of entries) { const button = el('button','memory-entry'); button.type = 'button'; if (entry.path === brainState.path) button.setAttribute('aria-current','true'); button.append(el('span','memory-entry-title',entry.title || entry.path),el('span','memory-entry-path',entry.path),el('span','memory-entry-meta',[entry.type,entry.status,entry.phase,Number.isInteger(entry.revision) ? `rev ${entry.revision}` : '',bytesLabel(entry.bytes)].filter(Boolean).join(' · '))); button.addEventListener('click',() => selectBrainEntry(entry.path,true)); $('brain-files').append(button); }
  $('brain-count').textContent = `${entries.length}${query || category ? ` of ${brainState.entries.length}` : ''} records${brainState.truncated ? ' · listing limited' : ''}`;
  if (!entries.length) $('brain-files').append(el('p','empty-note',query || category ? 'No records match this category and filter.' : brainState.bankId ? 'No readable records yet. Start a task from Record tools & maintenance.' : 'No Project Brain found in this project.'));
}
async function loadBrain({bank = brainState.bankId,preserve = true} = {}) {
  const projectId = $('brain-project').value; const projectChanged = projectId !== brainState.projectId; if (projectChanged) { bank = null; preserve = false; $('brain-filter').value = ''; $('brain-category').value = ''; }
  const previousPath = preserve && bank === brainState.bankId ? brainState.path : null; cancelBrainRequests(); cancelKnowledgeRead('brain'); knowledgeState.brain.meta = null; knowledgeState.brain.error = ''; const epoch = brainState.listEpoch; const controller = new AbortController(); brainState.listController = controller;
  brainState.projectId = projectId; brainState.bankId = bank; brainState.path = previousPath; brainState.entries = []; brainState.truncated = false; $('brain-files').replaceChildren(); $('brain-count').textContent = 'Loading Project Brain…'; $('brain-root').textContent = ''; $('brain-list-notice').hidden = true; clearBrainContent('Loading records…'); showError('brain-error','');
  if (projectChanged) { brainState.banks = []; setOptions($('brain-bank'),[],item => item.name,item => item.id); } updateKnowledgeControls();
  if (!state.bootstrap || !projectId) { brainState.listController = null; $('brain-count').textContent = ''; clearBrainContent('No registered project is available.'); return; }
  try {
    const endpoint = `/api/projects/${encodeURIComponent(projectId)}/brain`; let data;
    try { data = await api(endpoint + (bank ? `?bank=${encodeURIComponent(bank)}` : ''),{signal:controller.signal}); }
    catch (error) { if (!bank || ![400,404].includes(error.status) || epoch !== brainState.listEpoch) throw error; data = await api(endpoint,{signal:controller.signal}); }
    if (epoch !== brainState.listEpoch) return; brainState.banks = data.banks || []; brainState.bankId = data.bank_id || null; brainState.entries = data.entries || []; brainState.truncated = Boolean(data.truncated); setOptions($('brain-bank'),brainState.banks,item => item.name,item => item.id,brainState.bankId);
    $('brain-root').textContent = data.bank_id ? `${data.root ? `${data.root}/` : ''}project-brain` : ''; $('brain-list-notice').hidden = !brainState.truncated; renderBrainCategories(); renderBrainFiles(); loadKnowledgeMetadata('brain');
    const entry = brainState.entries.find(item => item.path === previousPath && brainState.bankId === bank) || brainState.entries.find(item => brainCategory(item) === 'tasks' || item.type === 'task') || brainState.entries[0];
    if (entry) selectBrainEntry(entry.path); else clearBrainContent(data.brain_available === false ? 'No Project Brain exists in this knowledge root.' : '');
  } catch (error) { if (error.name !== 'AbortError' && epoch === brainState.listEpoch) { $('brain-count').textContent = ''; showError('brain-error',textError(error)); clearBrainContent('Project Brain could not be loaded. Refresh to try again.'); } }
  finally { if (epoch === brainState.listEpoch) { brainState.listController = null; updateKnowledgeControls(); } }
}
async function selectBrainEntry(path,scroll = false) {
  const entry = brainState.entries.find(item => item.path === path); if (!entry || !brainState.bankId) return; brainState.fileController?.abort(); const controller = new AbortController(); brainState.fileController = controller; const epoch = ++brainState.fileEpoch; const listEpoch = brainState.listEpoch; brainState.path = path; renderBrainFiles(); clearBrainContent('Loading record…');
  try {
    const query = new URLSearchParams({bank:brainState.bankId,path}); const data = await api(`/api/projects/${encodeURIComponent(brainState.projectId)}/brain?${query}`,{signal:controller.signal}); if (epoch !== brainState.fileEpoch || listEpoch !== brainState.listEpoch) return;
    brainState.selected = {...entry,...(data.metadata || {}),...data}; $('brain-detail-header').hidden = false; $('brain-title').textContent = data.title || entry.title || path; $('brain-path').textContent = data.path || path; $('brain-meta').textContent = [brainState.selected.type,brainState.selected.status,brainState.selected.phase,brainState.selected.owner ? `Owner: ${brainState.selected.owner}` : '',Number.isInteger(brainState.selected.revision) ? `Revision ${brainState.selected.revision}` : '',bytesLabel(data.bytes)].filter(Boolean).join(' · ');
    $('brain-metadata').hidden = !data.metadata || !Object.keys(data.metadata).length; $('brain-metadata-content').textContent = JSON.stringify(data.metadata || {},null,2); $('brain-content').textContent = typeof data.content === 'string' ? data.content : ''; $('brain-content').hidden = false; $('brain-content').scrollTop = 0; $('brain-message').hidden = true; $('brain-content-notice').hidden = !data.truncated; updateKnowledgeControls();
    if (scroll && window.matchMedia('(max-width:760px)').matches) $('brain-details').scrollIntoView({block:'nearest',behavior:'instant'});
  } catch (error) { if (error.name !== 'AbortError' && epoch === brainState.fileEpoch && listEpoch === brainState.listEpoch) { $('brain-message').hidden = true; showError('brain-file-error',textError(error)); } }
  finally { if (epoch === brainState.fileEpoch) brainState.fileController = null; }
}
for (const scope of ['memory','brain']) buildKnowledgeTools(scope);
$('brain-project').addEventListener('change',() => loadBrain());
$('brain-bank').addEventListener('change',() => { $('brain-filter').value = ''; $('brain-category').value = ''; loadBrain({bank:$('brain-bank').value,preserve:false}); });
$('brain-filter').addEventListener('input',renderBrainFiles); $('brain-category').addEventListener('change',renderBrainFiles); $('refresh-brain').addEventListener('click',() => loadBrain());
function brainLinkConfig() {
  const create = $('brain-link-kind').value === 'create'; const task = brainLinkDraft.tasks.find(item => item.id === $('brain-link-task').value);
  return {bank:$('brain-link-bank').value,task_id:create ? $('brain-link-task-id').value.trim() : task?.external_id || task?.id || '',...(create ? {} : task?.id ? {record_id:task.id} : {}),query:$('brain-link-query').value.trim(),create,...(create ? {goal:$('brain-link-goal').value.trim()} : {})};
}
function resetBrainLink() { brainLinkDraft.epoch++; brainLinkDraft.controller?.abort(); Object.assign(brainLinkDraft,{projectId:null,bankId:null,banks:[],tasks:[],meta:null,loading:false,error:''}); $('brain-link-enabled').checked = false; for (const id of ['brain-link-task-id','brain-link-goal','brain-link-query']) $(id).value = ''; $('brain-link-kind').value = 'existing'; showError('brain-link-error',''); }
function updateBrainLinkControls() {
  const ready = Boolean(state.bootstrap) && !state.authFailed; const hasSession = Boolean(state.selectedId); const enabled = $('brain-link-enabled').checked; const creating = $('brain-link-kind').value === 'create'; const locked = !ready || hasSession || Boolean(state.pending); const unavailable = fleetDryRun();
  $('brain-link-config').hidden = hasSession; $('brain-link-enabled').disabled = locked || unavailable; $('brain-link-fields').hidden = !enabled; $('brain-link-summary').textContent = !enabled ? 'None' : creating ? $('brain-link-task-id').value.trim() || 'New task' : $('brain-link-task').value || 'Choose a task';
  for (const id of ['brain-link-bank','brain-link-kind','brain-link-task','brain-link-task-id','brain-link-goal','brain-link-query','brain-link-refresh']) $(id).disabled = locked || !enabled || unavailable || brainLinkDraft.loading;
  $('brain-link-bank').disabled ||= !brainLinkDraft.banks.length; $('brain-link-task').disabled ||= creating || !brainLinkDraft.tasks.length;
  $('brain-link-existing-field').hidden = creating; $('brain-link-id-field').hidden = !creating; $('brain-link-goal-field').hidden = !creating;
  $('brain-link-availability').textContent = brainLinkDraft.loading ? 'Reading knowledge roots and tasks…' : brainLinkDraft.error || (!brainLinkDraft.meta?.runtime_available ? 'The selected root has no context runtime. You can start an unlinked session.' : brainLinkDraft.meta.mode !== 'governed' ? 'Linking a task requires governed mode.' : !creating && !brainLinkDraft.tasks.length ? 'No active tasks were found. Choose Create a task; completed and cancelled tasks remain visible in Project Brain.' : 'Prepare writes the task, and any new worktree, before you review the context.');
  const config = brainLinkConfig(); let error = '';
  if (enabled && !hasSession) { if (unavailable) error = 'Offline Fleet dry-run cannot link a Brain task. Disable task linking or turn off dry-run.'; else if (brainLinkDraft.error) error = brainLinkDraft.error; else if (!brainLinkDraft.loading && brainLinkDraft.meta?.runtime_available && brainLinkDraft.meta.mode === 'governed' && !(config.bank && config.task_id && config.query && (!creating || config.goal))) $('brain-link-availability').textContent = creating ? 'Enter a task ID, its goal and a context query.' : 'Choose a task and write a context query.'; }
  showError('brain-link-error',error);
  return !enabled || hasSession || !unavailable && !brainLinkDraft.loading && brainLinkDraft.projectId === $('project').value && brainLinkDraft.meta?.runtime_available === true && brainLinkDraft.meta.mode === 'governed' && Boolean(config.bank && config.task_id && config.query && (!creating || config.goal));
}
async function loadBrainLinkTasks(reset = false) {
  const projectId = $('project').value; const changed = reset || brainLinkDraft.projectId !== projectId; const bank = changed ? null : $('brain-link-bank').value || brainLinkDraft.bankId; const selected = changed ? null : $('brain-link-task').value;
  brainLinkDraft.controller?.abort(); const epoch = ++brainLinkDraft.epoch; const controller = new AbortController(); brainLinkDraft.controller = controller; brainLinkDraft.projectId = projectId; brainLinkDraft.loading = true; brainLinkDraft.meta = null; brainLinkDraft.error = ''; if (changed) { brainLinkDraft.bankId = null; brainLinkDraft.banks = []; brainLinkDraft.tasks = []; }
  updateControls();
  if (!state.bootstrap || !projectId || state.selectedId) { brainLinkDraft.loading = false; return; }
  try { const data = await api(`/api/projects/${encodeURIComponent(projectId)}/brain${bank ? `?bank=${encodeURIComponent(bank)}` : ''}`,{signal:controller.signal}); if (epoch !== brainLinkDraft.epoch || projectId !== $('project').value) return;
    brainLinkDraft.meta = data; brainLinkDraft.banks = data.banks || []; brainLinkDraft.bankId = data.bank_id || null; brainLinkDraft.tasks = (data.entries || []).filter(item => item.type === 'task' && !['completed','cancelled'].includes(item.status) && !String(item.path || '').startsWith('archive/'));
    setOptions($('brain-link-bank'),brainLinkDraft.banks,item => `${item.name} · ${item.path}`,item => item.id,brainLinkDraft.bankId); setOptions($('brain-link-task'),brainLinkDraft.tasks,item => `${item.external_id || item.id} · ${item.title || 'Task'} · ${item.status || 'unknown'}`,item => item.id,selected);
  } catch (error) { if (error.name !== 'AbortError' && epoch === brainLinkDraft.epoch) brainLinkDraft.error = textError(error); }
  finally { if (epoch === brainLinkDraft.epoch) { brainLinkDraft.loading = false; brainLinkDraft.controller = null; updateControls(); } }
}
const linkedRecordDefinitions = {
  complete:{label:'Complete linked task',note:'Write a concise outcome and verification evidence after the provider run completes. This changes the linked Brain task; process completion alone does not verify the result.',fields:knowledgeActions.complete.fields.filter(field => field[0] !== 'task_id').map(field => field[0] === 'verification' ? [...field.slice(0,3),true,field[4]] : field)},
  'brain-update':{label:'Update linked task',note:'Progress replaces its current text; next steps are appended. Use the latest displayed task revision.',fields:knowledgeActions['brain-update'].fields.filter(field => field[0] !== 'record_id')},
  'source-update':{label:'Update evidence record',note:'Update a finding or decision from this session workspace by its UUID and current revision. Verified authority is your explicit attestation. A finding must be resolved or a decision accepted before it can qualify for promotion.',fields:[['record_id','Evidence record ID','text',true],['revision','Current record revision','number',true],['progress','Evidence / progress (optional)','textarea'],['authority','Evidence authority','select',false,[['','Keep current authority'],['verified','Verified — I checked the evidence']]],['transition','Lifecycle transition','select',false,[['','Keep current state'],['investigating','Investigating (finding)'],['resolved','Resolved (finding)'],['accepted','Accepted (decision)'],['rejected','Rejected (decision)']]],['reason','Reason (optional)','textarea']]},
  'brain-create':{label:'Create evidence record',note:'Write an evidence-backed finding or decision. This is a new source record in the actual session workspace. Use Update evidence record to record a verified authority and resolved/accepted state when justified.',fields:[['record_type','Record type','select',true,[['finding','Finding'],['decision','Decision']]],['external_id','External ID','text',true],['title','Title','text',true],['goal','Finding or decision','textarea',true],['authority','Evidence authority','select',true,[['observed','Observed'],['verified','Verified — I checked the evidence']]],['files','Files','lines',false,'One project-relative path per line'],['sources','Evidence sources','lines',true,'One source reference per line']]},
  rebind:{label:'Rebind linked task',note:'Restore the local task binding from the session’s fixed task record. This does not create another task.',fields:[]}
};
function linkedTask() { const data = linkedBrain.sid === state.selectedId ? linkedBrain.data : null; return data?.task?.record || data?.task || state.selected?.brain?.task?.record || state.selected?.brain?.task || null; }
function linkedSessionTarget() { const session = state.selected; return session ? `${projectFor(session.project_id)?.name || session.project_id} · ${session.project_path || 'workspace pending'} · ${session.brain?.bank || ''}` : ''; }
function linkedSessionReady() { return Boolean(state.selected?.brain && state.bootstrap && !state.authFailed && !state.loading && !state.pending && !active(state.selected) && !linkedBrain.loading && linkedBrain.sid === state.selectedId && linkedBrain.data); }
function buildLinkedRecordTools() {
  const details = el('details','knowledge-tools'); details.id = 'linked-record-details'; details.append(el('summary','','Result & evidence record tools')); const form = el('form'); form.id = 'linked-record-form'; const label = el('label','field'); label.append(el('span','','Operation')); const select = el('select'); select.id = 'linked-record-action';
  for (const [key,definition] of Object.entries(linkedRecordDefinitions)) { const option = el('option','',definition.label); option.value = key; select.append(option); } label.append(select); form.append(label); const evidenceLabel = el('label','field'); evidenceLabel.id = 'linked-evidence-field'; evidenceLabel.append(el('span','','Evidence record in this workspace')); const evidenceSelect = el('select'); evidenceSelect.id = 'linked-evidence-select'; evidenceLabel.append(evidenceSelect); form.append(evidenceLabel); const evidencePreview = el('details'); evidencePreview.id = 'linked-evidence-details'; evidencePreview.append(el('summary','','Selected evidence record')); const evidenceText = el('pre','knowledge-result'); evidenceText.id = 'linked-evidence-preview'; evidenceText.tabIndex = 0; evidencePreview.append(evidenceText); form.append(evidencePreview); evidenceSelect.addEventListener('change',renderLinkedEvidence); const note = el('p','knowledge-note'); note.id = 'linked-record-note'; form.append(note); const fields = el('div','knowledge-form-grid'); fields.id = 'linked-record-fields'; form.append(fields);
  const actions = el('div','knowledge-actions'); const fill = el('button','button','Use current task revision'); fill.id = 'linked-record-fill'; fill.type = 'button'; const submit = el('button','button'); submit.id = 'linked-record-submit'; submit.type = 'submit'; actions.append(fill,submit); form.append(actions); const error = el('p','error-text'); error.id = 'linked-record-error'; error.setAttribute('role','alert'); error.hidden = true; form.append(error); details.append(form); $('linked-record-tools').append(details);
  select.addEventListener('change',renderLinkedRecordFields); fill.addEventListener('click',fillLinkedRecordFields); form.addEventListener('submit',event => { event.preventDefault(); submitLinkedRecord(); }); renderLinkedRecordFields();
}
function renderLinkedRecordFields() { const action = $('linked-record-action').value; const definition = linkedRecordDefinitions[action]; $('linked-record-fields').replaceChildren(); for (const field of definition.fields) $('linked-record-fields').append(makeKnowledgeField('linked',field)); $('linked-record-note').textContent = definition.note; $('linked-record-submit').textContent = definition.label; showError('linked-record-error',''); renderLinkedEvidence(); updateLinkedRecordControls(); }
function fillLinkedRecordFields() { if (!linkedSessionReady()) return; const action = $('linked-record-action').value; const record = action === 'source-update' ? selectedLinkedEvidence() : linkedTask(); if (!record) return; if ($('linked-op-record_id')) $('linked-op-record_id').value = record.id || ''; if ($('linked-op-revision')) $('linked-op-revision').value = Number.isInteger(record.revision) ? record.revision : ''; }
function updateLinkedRecordControls() {
  if (!$('linked-record-form')) return; const ready = linkedSessionReady(); const action = $('linked-record-action').value; const allowed = ready && (action !== 'complete' || state.selected.status === 'completed' && !['completed','cancelled'].includes(linkedTask()?.status));
  for (const input of $('linked-record-form').querySelectorAll('input,textarea,select,button')) input.disabled = !allowed; $('linked-record-action').disabled = !ready;
  $('linked-record-fill').hidden = !['complete','brain-update','source-update'].includes(action); $('linked-record-fill').textContent = action === 'source-update' ? 'Use selected evidence revision' : 'Use current task revision'; $('linked-record-fill').disabled = !allowed || !Number.isInteger((action === 'source-update' ? selectedLinkedEvidence() : linkedTask())?.revision);
  $('linked-record-note').textContent = linkedRecordDefinitions[action].note + (action === 'complete' && state.selected?.status !== 'completed' ? ' Complete becomes available after the provider run finishes with completed status.' : action === 'complete' && ['completed','cancelled'].includes(linkedTask()?.status) ? ' This task is already terminal; its outcome cannot be completed again.' : '');
  $('linked-evidence-field').hidden = action !== 'source-update'; $('linked-evidence-details').hidden = action !== 'source-update' || !selectedLinkedEvidence();
  for (const input of $('linked-proposal-form').querySelectorAll('input,textarea,button')) input.disabled = !ready; $('linked-propose').disabled = !ready || !linkedBrain.sourceIds.size || linkedBrain.sourceIds.size > 20 || !$('linked-proposal-title').value.trim() || !$('linked-proposal-content').value.trim();
  $('linked-promotion-select').disabled = !ready || !(linkedBrain.data?.promotions || []).length; const proposal = selectedLinkedPromotion(); const proposed = proposal?.status === 'proposed' && proposal.review_mode !== 'automatic'; const approved = proposal?.status === 'reviewed' && proposal.outcome === 'approved';
  $('linked-reviewer-field').hidden = !proposed; $('linked-reviewer-note').hidden = !proposed; $('linked-promotion-reviewer').disabled = !ready || !proposed; const reviewer = $('linked-promotion-reviewer').value.trim();
  $('linked-promotion-approve').hidden = !proposed; $('linked-promotion-reject').hidden = !proposed; $('linked-promotion-apply').hidden = !approved;
  $('linked-promotion-approve').disabled = $('linked-promotion-reject').disabled = !ready || !proposed || !reviewer || reviewer === proposal?.proposer;
  $('linked-promotion-apply').disabled = !ready || !approved;
  $('linked-brain-refresh').disabled = !state.bootstrap || Boolean(state.pending) || linkedBrain.loading || !state.selected?.brain;
}
function submitLinkedRecord() {
  if (!linkedSessionReady() || $('linked-record-submit').disabled || !$('linked-record-form').reportValidity()) return; const action = $('linked-record-action').value; const fields = {}; let error = '';
  for (const [name,label,type,required] of linkedRecordDefinitions[action].fields) { const value = $(`linked-op-${name}`).value.trim(); if (required && !value) { error = `${label} is required.`; break; } if (type === 'number') { const number = Number(value); if (!Number.isInteger(number) || number < 1 || number > 2147483647) { error = 'Use a positive current revision from the displayed record.'; break; } fields[name] = number; } else if (type === 'lines') { if (value) fields[name] = value.split(/\r?\n/).map(line => line.trim()).filter(Boolean); if ((fields[name] || []).length > 50) { error = `Use at most 50 lines for ${label.toLowerCase()}.`; break; } } else if (value) fields[name] = value; }
  if (!error && ['brain-update','source-update'].includes(action) && !['progress','next_steps','phase','transition','authority'].some(key => fields[key]?.length)) error = 'Choose an explicit change to the record.';
  showError('linked-record-error',error); if (!error) { if (action === 'brain-update') fields.record_id = linkedTask()?.id; runLinkedBrainOperation(action === 'source-update' ? 'brain-update' : action,fields); }
}
function selectedLinkedEvidence() { return (linkedBrain.data?.records || []).find(item => item.id === $('linked-evidence-select').value && ['finding','decision'].includes(item.type)) || linkedBrain.lastRecord; }
function renderLinkedEvidence() { const record = selectedLinkedEvidence(); $('linked-evidence-preview').textContent = record ? JSON.stringify(record,null,2) : ''; updateLinkedRecordControls(); }
function selectedLinkedPromotion() { return (linkedBrain.data?.promotions || []).find(item => item.id === linkedBrain.promotionId); }
function renderLinkedPromotions() {
  const data = linkedBrain.data || {}; const records = (data.records || []).filter(item => ['finding','decision'].includes(item.type)); setOptions($('linked-evidence-select'),records,item => `${item.title || item.external_id || item.id} · ${item.status} · revision ${item.revision}`,item => item.id,$('linked-evidence-select').value || linkedBrain.lastRecord?.id); renderLinkedEvidence(); const eligible = (data.eligible_sources || []).filter(item => ['finding','decision'].includes(item.type) && typeof item.id === 'string'); const ids = new Set(eligible.map(item => item.id)); linkedBrain.sourceIds = new Set([...linkedBrain.sourceIds].filter(id => ids.has(id))); $('linked-source-list').replaceChildren();
  for (const source of eligible) { const label = el('label','linked-source'); const input = el('input'); input.type = 'checkbox'; input.value = source.id; input.checked = linkedBrain.sourceIds.has(source.id); const text = el('span','',source.title || source.external_id || source.id); text.append(el('small','',[source.id,source.type,source.status,source.authority,Number.isInteger(source.revision) ? `revision ${source.revision}` : ''].filter(Boolean).join(' · '))); label.append(input,text); input.addEventListener('change',() => { if (input.checked) linkedBrain.sourceIds.add(source.id); else linkedBrain.sourceIds.delete(source.id); updateLinkedRecordControls(); }); $('linked-source-list').append(label); }
  if (!eligible.length) $('linked-source-list').append(el('p','knowledge-note','No eligible finding or decision is available. Create an evidence record, verify it, and resolve or accept it before proposing memory.'));
  $('linked-promotion-note').textContent = `Choose 1–20 eligible evidence records and write reusable knowledge. Tasks and raw transcripts are never source records.${data.automatic_promotion === true ? ' This root has automatic promotion enabled; inspect existing proposals and applied memory before creating a duplicate.' : ''}`;
  const proposals = data.promotions || []; setOptions($('linked-promotion-select'),proposals,item => `${item.title || item.id} · ${item.status || 'unknown'}`,item => item.id,linkedBrain.promotionId); linkedBrain.promotionId = $('linked-promotion-select').value || null; renderLinkedPromotionPreview();
}
function renderLinkedPromotionPreview() { const proposal = selectedLinkedPromotion(); $('linked-promotion-preview').hidden = !proposal; $('linked-promotion-preview').textContent = proposal ? JSON.stringify(proposal,null,2) : ''; $('linked-promotion-state').textContent = proposal ? `${humanLabel(proposal.status)}${proposal.outcome ? ` · ${proposal.outcome}` : ''}${proposal.destination_memory_id ? ` · Memory ${proposal.destination_memory_id}` : ''}` : 'No saved proposals in this workspace.'; updateLinkedRecordControls(); }
function resetLinkedSelection(session) {
  linkedBrain.epoch++; linkedBrain.controller?.abort(); Object.assign(linkedBrain,{sid:session?.id || null,data:null,loading:false,fetchKey:null,contextKey:null,stale:false,sourceIds:new Set(),promotionId:null,lastRecord:null,records:new Map()});
  $('linked-context-query').value = session?.brain?.query || ''; $('linked-proposal-title').value = ''; $('linked-proposal-content').value = ''; $('linked-promotion-reviewer').value = ''; $('linked-brain-output').hidden = true; $('linked-brain-output').replaceChildren(); $('linked-record-action').value = 'complete'; renderLinkedRecordFields(); showError('linked-brain-error',''); showError('linked-context-error','');
}
function renderLinkedSession() {
  const session = state.selected; const linked = Boolean(session?.brain); $('linked-context').hidden = !linked; $('linked-result').hidden = !linked; if (!linked) { if (linkedBrain.sid) resetLinkedSelection(null); return; }
  if (linkedBrain.sid !== session.id) resetLinkedSelection(session); const brain = session.brain; const waiting = session.status === 'awaiting_context'; const capsule = brain.capsule; const contextId = brain.context_id || null; const key = `${session.id}:${contextId || ''}`;
  if (linkedBrain.contextKey !== key) { linkedBrain.contextKey = key; linkedBrain.stale = false; $('linked-capsule').textContent = contextId && capsule ? typeof capsule === 'string' ? capsule : JSON.stringify(capsule,null,2) : 'Context is being prepared. The provider has not started this prepared turn.'; if (waiting) $('linked-capsule-details').open = true; }
  $('linked-context').classList.toggle('linked-context-review',waiting); $('linked-context-title').textContent = waiting ? 'Review context before running' : 'Linked Brain task'; $('linked-context-state').textContent = waiting ? contextId ? 'Awaiting your review' : 'Context refresh required' : contextId ? brain.approved ? 'Context accepted for this turn' : 'Saved reviewed context' : ['queued','running'].includes(session.status) ? 'Preparing context' : 'Context preparation required';
  const task = linkedTask(); $('linked-context-target').textContent = `${linkedSessionTarget()}\nTask: ${brain.task_id || task?.external_id || ''}${task?.id ? ` · ${task.id}` : ''}${Number.isInteger(task?.revision) ? ` · current task revision ${task.revision}` : ''}`;
  const dirtyQuery = $('linked-context-query').value.trim() !== (brain.query || '').trim(); const ready = state.bootstrap && !state.authFailed && !state.pending && !state.loading && !active(session); const canRefresh = ready && !['completed','rejected','awaiting_approval'].includes(session.status);
  $('linked-context-note').textContent = linkedBrain.stale || waiting && !contextId ? 'This context could not be accepted. Refresh it and review the new capsule before running.' : dirtyQuery ? 'The query was edited. Refresh context to prepare a capsule for this query.' : waiting ? 'The workspace and context are prepared. Review this capsule, then explicitly run the selected provider with it.' : session.status === 'completed' ? isFleetSession(session) ? 'This is the reviewed context for the completed Fleet run.' : ['completed','cancelled'].includes(task?.status) ? 'This is the reviewed context for the completed turn. The linked task is terminal; start a new session with an active or new task to continue.' : 'This is the reviewed context for the completed turn. Send a follow-up to prepare another turn.' : 'This is the persisted context for the session’s actual working folder. Refresh retrieves context without running the provider.';
  $('linked-context-query').disabled = !canRefresh; $('linked-context-refresh').disabled = !canRefresh || !$('linked-context-query').value.trim(); $('linked-context-run').hidden = !waiting; $('linked-context-run').disabled = !ready || !contextId || !capsule || linkedBrain.stale || dirtyQuery || budgetsDirty();
  $('linked-brain-target').textContent = linkedSessionTarget(); $('linked-brain-task').textContent = task ? JSON.stringify(task,null,2) : 'Task metadata is not yet available.'; $('linked-brain-status').textContent = linkedBrain.loading ? 'Reading task and evidence records from this session workspace…' : linkedBrain.data ? 'Record operations below use this session workspace. Review evidence before saving an outcome or promoting knowledge.' : 'Prepare context before editing linked records.';
  const fetchKey = `${session.id}:${session.status}:${contextId || ''}`; if (!active(session) && !state.loading && !state.pending && linkedBrain.fetchKey !== fetchKey) { linkedBrain.fetchKey = fetchKey; loadLinkedBrain(); }
  updateLinkedRecordControls();
}
async function loadLinkedBrain() {
  const session = state.selected; if (!session?.brain || state.pending) return; const sid = session.id; linkedBrain.controller?.abort(); const controller = new AbortController(); linkedBrain.controller = controller; const epoch = ++linkedBrain.epoch; linkedBrain.loading = true; showError('linked-brain-error',''); updateLinkedRecordControls();
  try { const data = await api(`/api/sessions/${encodeURIComponent(sid)}/brain`,{signal:controller.signal}); if (epoch !== linkedBrain.epoch || sid !== state.selectedId) return; linkedBrain.data = data; renderLinkedPromotions(); }
  catch (error) { if (error.name !== 'AbortError' && epoch === linkedBrain.epoch && sid === state.selectedId) { linkedBrain.data = null; showError('linked-brain-error',textError(error)); } }
  finally { if (epoch === linkedBrain.epoch && sid === state.selectedId) { linkedBrain.loading = false; linkedBrain.controller = null; updateControls(); } }
}
async function linkedContextAction(action) {
  const session = state.selected; if (!session?.brain || state.pending || active(session) || state.loading || !state.bootstrap || state.authFailed) return; const sid = session.id; const epoch = state.epoch; const query = $('linked-context-query').value.trim(); const run = action === 'run'; if (run ? $('linked-context-run').disabled : $('linked-context-refresh').disabled || !query) return;
  state.pending = `brain-context-${action}`; stopPolling(); showError('linked-context-error',''); updateControls();
  try { const data = await api(`/api/sessions/${encodeURIComponent(sid)}/${run ? 'run' : 'context'}`,{method:'POST',body:run ? {context_id:session.brain.context_id} : {query}}); if (epoch !== state.epoch || sid !== state.selectedId) return; if (data.session) { if (data.session.id !== sid) throw new Error('The runner returned a different session. Refresh before retrying.'); upsert(data.session); applySessionSettings(data.session); } if (!run) { linkedBrain.stale = false; $('linked-context-query').value = query; } await pollSession(epoch); }
  catch (error) { if (epoch === state.epoch && sid === state.selectedId) { if (run) linkedBrain.stale = true; showError('linked-context-error',textError(error)); } }
  finally { state.pending = null; updateControls(); }
}
async function runLinkedBrainOperation(action,fields) {
  if (!linkedSessionReady()) return; const session = state.selected; const sid = session.id; const epoch = state.epoch; const output = $('linked-brain-output'); const target = linkedSessionTarget(); state.pending = `brain-${action}`; output.hidden = false; output.replaceChildren(el('h3','',humanLabel(action)),el('p','knowledge-note',target)); const status = el('p','knowledge-note','Running in the session workspace… You can visit another section; this command will continue.'); status.setAttribute('role','status'); output.append(status); updateControls();
  try { const data = await api(`/api/sessions/${encodeURIComponent(sid)}/brain`,{method:'POST',body:{action,...fields}}); if (sid !== state.selectedId || epoch !== state.epoch) return; status.textContent = data.ok ? 'Operation complete.' : 'Operation failed.'; if (data.error) output.append(el('p','error-text',typeof data.error === 'string' ? data.error : JSON.stringify(data.error))); if (data.result !== undefined) output.append(el('pre','knowledge-result',typeof data.result === 'string' ? data.result : JSON.stringify(data.result,null,2)));
    if (data.ok && action === 'rebind') {
      linkedBrain.stale = true; $('linked-capsule').textContent = 'The task was rebound. Refresh context before running.';
      try { const current = await api(`/api/sessions/${encodeURIComponent(sid)}`); if (sid !== state.selectedId || epoch !== state.epoch) return; if (current.session?.id !== sid) throw new Error('The runner did not return the current session.'); upsert(current.session); applySessionSettings(current.session,false); }
      catch (error) { if (sid === state.selectedId && epoch === state.epoch) { status.textContent = 'Task rebound; session refresh is needed.'; output.append(el('p','error-text',`${textError(error)} Refresh the session before running with context.`)); } }
    }
    if (data.ok && ['brain-create','brain-update'].includes(action)) { const record = data.result?.record || data.result; if (record?.id && ['finding','decision'].includes(record.type)) { linkedBrain.lastRecord = record; linkedBrain.records.set(record.id,record); } }
    if (data.ok && ['promote-propose','promote-review','promote-apply'].includes(action) && data.result?.id) linkedBrain.promotionId = data.result.id;
  } catch (error) { if (sid === state.selectedId && epoch === state.epoch) { status.textContent = 'Operation did not complete successfully.'; output.append(el('p','error-text',`${textError(error)}${error.status === 0 ? ' Refresh task records before retrying; changes may have been applied.' : ''}`)); } }
  finally { state.pending = null; if (sid === state.selectedId && epoch === state.epoch) { await loadLinkedBrain(); updateControls(); } }
}
buildLinkedRecordTools();
$('brain-link-enabled').addEventListener('change',() => { if ($('brain-link-enabled').checked) loadBrainLinkTasks(); updateControls(); }); $('brain-link-bank').addEventListener('change',() => loadBrainLinkTasks()); $('brain-link-refresh').addEventListener('click',() => loadBrainLinkTasks());
for (const id of ['brain-link-kind','brain-link-task']) $(id).addEventListener('change',updateControls); for (const id of ['brain-link-task-id','brain-link-goal','brain-link-query']) $(id).addEventListener('input',updateControls);
$('linked-context-form').addEventListener('submit',event => { event.preventDefault(); if ($('linked-context-form').reportValidity()) linkedContextAction('refresh'); }); $('linked-context-run').addEventListener('click',() => linkedContextAction('run')); $('linked-context-query').addEventListener('input',renderLinkedSession); $('linked-brain-refresh').addEventListener('click',() => loadLinkedBrain());
$('linked-proposal-form').addEventListener('submit',event => { event.preventDefault(); if ($('linked-propose').disabled || !$('linked-proposal-form').reportValidity()) return; runLinkedBrainOperation('promote-propose',{source_ids:[...linkedBrain.sourceIds],title:$('linked-proposal-title').value.trim(),content:$('linked-proposal-content').value.trim()}); });
for (const id of ['linked-proposal-title','linked-proposal-content','linked-promotion-reviewer']) $(id).addEventListener('input',updateLinkedRecordControls);
$('linked-promotion-select').addEventListener('change',() => { linkedBrain.promotionId = $('linked-promotion-select').value || null; $('linked-promotion-reviewer').value = ''; renderLinkedPromotionPreview(); });
for (const [id,reject] of [['linked-promotion-approve',false],['linked-promotion-reject',true]]) $(id).addEventListener('click',() => { if (!$(id).disabled) runLinkedBrainOperation('promote-review',{promotion_id:linkedBrain.promotionId,reviewer:$('linked-promotion-reviewer').value.trim(),reject}); });
$('linked-promotion-apply').addEventListener('click',() => { if (!$('linked-promotion-apply').disabled) runLinkedBrainOperation('promote-apply',{promotion_id:linkedBrain.promotionId}); });
