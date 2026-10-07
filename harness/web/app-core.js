// Shell, navigation and Sessions: theme, state, budgets, results, routing, preferences, history, settings and the session stream.
// Classic script loaded in order by index.html; top-level names are shared with the other app-*.js files.
'use strict';
const $ = id => document.getElementById(id);
const outputResizer = $('output-resizer');
const sessionConfiguration = document.querySelector('#sessions-view .configuration');
// The height the person asked for; the shown height is that, clamped to what the composer and the Run strip leave.
let outputHeightRequest = null;
function resizeOutput(configurationHeight, requested = true) {
  if (requested) outputHeightRequest = configurationHeight;
  const available = $('sessions-view').clientHeight - $('composer-area').offsetHeight - $('run-view').offsetHeight - outputResizer.offsetHeight;
  const height = Math.max(80,Math.min(available - 100,configurationHeight));
  $('sessions-view').style.setProperty('--configuration-height',`${height}px`);
  $('sessions-view').classList.add('output-resized');
}
// When the space around it changes (the Run strip or its panel opens or closes), a resized output is clamped again from
// the person's request, so it shrinks while the strip needs room and grows back afterwards.
function refitOutput() { if ($('sessions-view').classList.contains('output-resized') && outputHeightRequest !== null) resizeOutput(outputHeightRequest,false); }
outputResizer.addEventListener('pointerdown',event => {
  if (!event.isPrimary || event.button !== 0) return;
  event.preventDefault(); outputResizer.focus(); outputResizer.setPointerCapture(event.pointerId);
});
outputResizer.addEventListener('pointermove',event => {
  if (outputResizer.hasPointerCapture(event.pointerId)) resizeOutput(event.clientY - sessionConfiguration.getBoundingClientRect().top);
});
const finishOutputResize = event => { if (outputResizer.hasPointerCapture(event.pointerId)) outputResizer.releasePointerCapture(event.pointerId); };
outputResizer.addEventListener('pointerup',finishOutputResize);
outputResizer.addEventListener('pointercancel',finishOutputResize);
outputResizer.addEventListener('keydown',event => {
  const height = sessionConfiguration.getBoundingClientRect().height;
  const requested = {ArrowUp:height - 40,ArrowDown:height + 40,Home:80,End:$('sessions-view').clientHeight}[event.key];
  if (requested !== undefined) { event.preventDefault(); resizeOutput(requested); }
});
outputResizer.addEventListener('dblclick',() => {
  $('sessions-view').classList.remove('output-resized'); $('sessions-view').style.removeProperty('--configuration-height');
});
new ResizeObserver(() => {
  const outputHeight = $('conversation').clientHeight;
  const total = outputHeight + sessionConfiguration.getBoundingClientRect().height;
  if (!total) return;
  const percent = Math.round(outputHeight / total * 100);
  outputResizer.setAttribute('aria-valuenow',String(percent));
  outputResizer.setAttribute('aria-valuetext',`${percent}% AI output`);
}).observe($('conversation'));
// The head script already applied the saved theme; this wires the sidebar switch and follows the system
// setting and other Harness tabs. No saved value means "follow the system". The Kit 3 frame reads the
// same stored value and listens for its changes itself.
const themeKey = 'harness.theme.v1', systemDark = window.matchMedia('(prefers-color-scheme: dark)');
function readThemePreference() {
  try { const saved = localStorage.getItem(themeKey); return saved === 'light' || saved === 'dark' ? saved : 'system'; } catch (_) { return 'system'; }
}
function saveThemePreference(preference) {
  try { if (preference === 'system') localStorage.removeItem(themeKey); else localStorage.setItem(themeKey,preference); } catch (_) { /* Storage may be disabled; the choice then lasts until reload. */ }
}
let themePreference = readThemePreference();
function applyTheme() {
  document.documentElement.dataset.theme = themePreference === 'system' ? (systemDark.matches ? 'dark' : 'light') : themePreference;
  for (const input of document.querySelectorAll('input[name="theme"]')) input.checked = input.value === themePreference;
}
for (const input of document.querySelectorAll('input[name="theme"]')) input.addEventListener('change',() => { themePreference = input.value; saveThemePreference(themePreference); applyTheme(); });
systemDark.addEventListener('change',applyTheme);
window.addEventListener('storage',event => { if (event.key === themeKey || event.key === null) { themePreference = readThemePreference(); applyTheme(); } });
applyTheme();
const el = (tag, className, text) => { const element = document.createElement(tag); if (className) element.className = className; if (text !== undefined) element.textContent = text; return element; };
// Motion follows the system setting; a scroll started from script is smooth only when motion is welcome.
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
const scrollMotion = () => reducedMotion.matches ? 'auto' : 'smooth';
// Numbers follow one grammar: exact values are grouped digits, estimates carry ≈ and two significant digits,
// bounds carry ≤, ≥ or +, and unknown is —, never 0. Each helper returns the visible text and its spoken label.
const numberText = new Intl.NumberFormat('en-US',{maximumFractionDigits:2});
// Thousands and millions in short form; `significant` keeps an estimate's second digit (2.0k, not 2k).
function compactNumber(value, significant = false) {
  const size = Math.abs(value), short = (scaled, unit) => `${significant && Math.abs(scaled) < 10 ? scaled.toFixed(1) : numberText.format(Math.abs(scaled) >= 10 ? Math.round(scaled) : scaled)}${unit}`;
  return size >= 1e6 ? short(value / 1e6,'M') : size >= 1e3 ? short(value / 1e3,'k') : numberText.format(value);
}
const fmt = {
  exact: (value, unit = '') => Number.isFinite(value) ? fmt.same(numberText.format(value) + unit) : fmt.unknown(),
  estimate: (value, unit = '') => { if (!Number.isFinite(value)) return fmt.unknown(); const rounded = Number(value.toPrecision(2)); return {text:`≈ ${compactNumber(rounded,true)}${unit}`, label:`about ${numberText.format(rounded)}${unit}`}; },
  atMost: (value, unit = '') => Number.isFinite(value) ? {text:`≤ ${compactNumber(value)}${unit}`, label:`at most ${numberText.format(value)}${unit}`} : fmt.unknown(),
  atLeast: (value, unit = '') => Number.isFinite(value) ? {text:`≥ ${compactNumber(value)}${unit}`, label:`at least ${numberText.format(value)}${unit}`} : fmt.unknown(),
  capped: (value, unit = '') => Number.isFinite(value) ? {text:`${numberText.format(value)}+${unit}`, label:`more than ${numberText.format(value)}${unit}`} : fmt.unknown(),
  // Provider costs are the CLI's own estimates, so money always reads as one.
  cost: value => { if (!Number.isFinite(value) || value < 0) return fmt.unknown(); const amount = value >= 1 ? value.toFixed(2) : value.toFixed(4).replace(/(\.\d\d\d??)0+$/,'$1'); return {text:`≈ $${amount}`, label:`about ${amount} US dollars`}; },
  unknown: (label = 'not reported') => ({text:'—', label}),
  same: text => ({text, label:text}),
};
function numberNode(value) {
  if (value.label === value.text) return el('span','',value.text);
  const node = el('span'), shown = el('span','',value.text); shown.setAttribute('aria-hidden','true');
  node.append(shown,el('span','sr-only',value.label)); return node;
}
// Keeps one node per key across polls: an unchanged item keeps its node, so an open <details>, focus and scroll
// survive; a changed item is rebuilt in place and stays open if it was; items that left are removed.
function keyedRender(container, items, keyOf, build, signatureOf = item => JSON.stringify(item)) {
  const previous = new Map([...container.children].map(node => [node.dataset.key, node]));
  items.forEach((item, index) => {
    const key = String(keyOf(item)), signature = signatureOf(item), old = previous.get(key);
    let node = old;
    if (!old || old.dataset.signature !== signature) {
      node = build(item); node.dataset.key = key; node.dataset.signature = signature;
      if (old?.open) node.open = true;
      if (old) old.replaceWith(node);
    }
    previous.delete(key);
    if (container.children[index] !== node) container.insertBefore(node,container.children[index] || null);
  });
  for (const node of previous.values()) node.remove();
}
const state = { bootstrap:null, sessions:[], selected:null, selectedId:null, view:'sessions', pending:null, loading:false, pollTimer:null, pollController:null, epoch:0, eventIds:new Set(), assistantTexts:new Set(), stepCalls:new Map(), contextEpoch:0, authFailed:false };
let attachedFiles = [];
function renderAttachments() {
  $('attachment-list').replaceChildren(); $('attachment-list').hidden = !attachedFiles.length;
  $('attachment-hint').hidden = !attachedFiles.length && $('attachment-error').hidden;
  attachedFiles.forEach((file,index) => {
    const chip = el('div','attachment-chip'); const remove = el('button','icon-button','×'); remove.type = 'button'; remove.disabled = Boolean(state.pending); remove.setAttribute('aria-label',`Remove ${file.name}`);
    remove.addEventListener('click',() => { attachedFiles.splice(index,1); showError('attachment-error',''); renderAttachments(); });
    chip.append(el('span','',`${file.name} · ${bytesLabel(file.size)}`),remove); $('attachment-list').append(chip);
  });
}
function clearAttachments() { attachedFiles = []; $('attachment-input').value = ''; showError('attachment-error',''); renderAttachments(); }
$('attach-files').addEventListener('click',() => $('attachment-input').click());
$('attachment-input').addEventListener('change',() => {
  const files = [...attachedFiles,...$('attachment-input').files]; $('attachment-input').value = '';
  if (files.length > 5 || files.some(file => file.size > 4 * 1024 * 1024) || files.reduce((sum,file) => sum + file.size,0) > 8 * 1024 * 1024) { showError('attachment-error','Choose up to 5 files, at most 4 MiB each and 8 MiB total.'); return; }
  attachedFiles = files; showError('attachment-error',''); renderAttachments();
});
function encodeAttachment(file) {
  return new Promise((resolve,reject) => {
    const reader = new FileReader(); reader.onload = () => resolve({name:file.name,data:String(reader.result).split(',')[1]});
    reader.onerror = () => reject(new Error(`Could not read ${file.name}. Attach it again.`)); reader.onabort = reader.onerror; reader.readAsDataURL(file);
  });
}
const memoryState = {projectId:null,bankId:null,path:null,banks:[],entries:[],truncated:false,listEpoch:0,fileEpoch:0,listController:null,fileController:null};
const brainState = {projectId:null,bankId:null,path:null,banks:[],entries:[],selected:null,truncated:false,listEpoch:0,fileEpoch:0,listController:null,fileController:null};
const knowledgeState = {pending:null,memory:{meta:null,epoch:0,controller:null,pending:false},brain:{meta:null,epoch:0,controller:null,pending:false}};
const brainLinkDraft = {projectId:null,bankId:null,banks:[],tasks:[],meta:null,epoch:0,controller:null,loading:false,error:''};
const linkedBrain = {sid:null,data:null,epoch:0,controller:null,loading:false,fetchKey:null,contextKey:null,stale:false};
const setupState = {pending:null,registerPending:false,projectsPending:false,projectId:null,data:null,loading:false,epoch:0,controller:null,tools:new Set(),toolsInitialized:false,preferredEdition:null,preview:null,previewEpoch:0,expiryTimer:null};
const skillsState = {catalog:null,catalogPending:false,catalogEpoch:0,catalogError:false,sourceId:null,discoveredSourceId:null,skills:[],selectedSkills:new Set(),selectedAgents:new Set(),agentsInitialized:false,pending:null,actionEpoch:0,preview:null,installed:[],installedProjectId:null,installedEpoch:0,installedController:null,installedPending:false,installedError:false,changePreview:null,changeTimer:null};
const createSkillState = {selectedAgents:new Set(),agentsInitialized:false,touched:new Set(),pending:null,epoch:0,preview:null,resultProjectId:null};
const projectGitState = {projectId:null,data:null,pending:false,error:null,epoch:0,controller:null};
// Workspace › Existing Git worktree: the project's other checkouts; `preferred` is the choice a draft or the person made.
const projectWorktreeState = {projectId:null,data:null,pending:false,error:null,epoch:0,controller:null,preferred:''};
const fleetUi = {lenses:new Set(),initialized:false,reviewers:new Map(),stage:null,resultKey:null,reportOpenedFor:null};
const active = session => session && ['queued','running'].includes(session.status);
const isFleetSession = session => session?.workflow === 'fleet-review';
const statusLabel = value => ({queued:'Queued',running:'Running',completed:'Process complete',failed:'Failed',cancelled:'Cancelled',interrupted:'Interrupted',awaiting_context:'Review context',awaiting_approval:'Awaiting approval',rejected:'Report rejected'})[value] || value || 'Ready';
const sessionStatusLabel = session => isFleetSession(session) && session.status === 'completed' ? 'Report saved' : statusLabel(session.status);
const projectFor = id => state.bootstrap?.projects.find(project => project.id === id);
const providerFor = id => state.bootstrap?.providers.find(provider => provider.id === id);
// What each CLI says about its own sign-in, asked without a model call. Advisory: a run is never refused on it.
const signIn = {states:{}, pending:null};
const signedOut = id => signIn.states[id]?.state === 'signed_out';
const providerLabel = item => `${item.name}${!item.available ? ' · unavailable' : signedOut(item.id) ? ' · not signed in' : ''}`;
async function loadSignIn(refresh = false) {
  if (!state.bootstrap || state.authFailed || signIn.pending) return;
  signIn.pending = true;
  try {
    const data = await api('/api/providers/sign-in' + (refresh ? '?refresh=1' : ''));
    signIn.states = Object.fromEntries((Array.isArray(data.providers) ? data.providers : []).filter(item => typeof item?.id === 'string').map(item => [item.id,item]));
    for (const option of $('provider').options) { const item = providerFor(option.value); if (item) option.textContent = providerLabel(item); }
    updateControls();
  } catch (_) { /* Without an answer a run still reports a refused sign-in itself. */ }
  finally { signIn.pending = null; }
}
// Updates of this clone, offered as a desktop application offers them (updates.py): an Update button in the header
// while the branch the clone follows has moved on. A click fast-forwards the clone and restarts the Harness, and
// this page reloads itself on the new version.
const appUpdate = {status:null, pending:false, restarting:false, instance:null, note:'', failed:false};
function renderAppUpdate() {
  const status = appUpdate.status || {}, button = $('app-update'), note = $('app-update-note');
  const available = status.state === 'available', busy = appUpdate.pending || appUpdate.restarting;
  button.hidden = !available && !busy; button.disabled = busy;
  button.textContent = appUpdate.restarting ? 'Restarting…' : appUpdate.pending ? 'Updating…' : 'Update';
  button.title = available ? [status.detail,...(status.commits || []).slice(0,10).map(commit => `• ${commit.subject}`)].join('\n') : '';
  const text = appUpdate.note || (status.state === 'diverged' ? status.detail : ''), banner = $('app-update-banner');
  if (note.textContent !== text) note.textContent = text;
  banner.hidden = !text; banner.classList.toggle('error',appUpdate.failed);
}
async function loadAppUpdate(check = false) {
  if (!state.bootstrap || state.authFailed || appUpdate.pending || appUpdate.restarting) return;
  try { appUpdate.status = await api('/api/app/update' + (check ? '?check=1' : '')); renderAppUpdate(); } catch (_) { /* Asked again later. */ }
  // A check fetches in the background: look again once it has had time to.
  if (check) setTimeout(() => loadAppUpdate(false),8000);
}
async function waitForRestart() {
  const deadline = Date.now() + 120000;
  while (Date.now() < deadline) {
    await new Promise(resolve => setTimeout(resolve,1000));
    try { const health = await (await fetch('/api/health',{cache:'no-store'})).json(); if (health.instance && health.instance !== appUpdate.instance) { location.reload(); return; } }
    catch (_) { /* Still restarting. */ }
  }
  Object.assign(appUpdate,{restarting:false,failed:true,note:'The Harness did not come back after the update. Start it again from the application icon.'}); renderAppUpdate();
}
async function applyAppUpdate() {
  if (appUpdate.pending || appUpdate.restarting) return;
  Object.assign(appUpdate,{pending:true,failed:false,note:''}); renderAppUpdate();
  try {
    appUpdate.instance = (await (await fetch('/api/health',{cache:'no-store'})).json()).instance;
    const result = await api('/api/app/update',{method:'POST',body:{}});
    if (result.current) { Object.assign(appUpdate,{pending:false,note:result.detail}); appUpdate.status = {...appUpdate.status,state:'current'}; renderAppUpdate(); return; }
    Object.assign(appUpdate,{pending:false,restarting:result.restarting === true,note:result.restarting === true ? `Updated to ${result.to}. The Harness is restarting; this page reloads by itself.` : `Updated to ${result.to}. Restart the Harness to use it.`});
    appUpdate.status = {...appUpdate.status,state:'updated'}; renderAppUpdate();
    if (appUpdate.restarting) waitForRestart();
  } catch (error) { Object.assign(appUpdate,{pending:false,failed:true,note:textError(error)}); renderAppUpdate(); }
}
$('app-update').addEventListener('click',applyAppUpdate);
setInterval(() => loadAppUpdate(true),10 * 60 * 1000);
// Signing in happens in a terminal: ask again when the person comes back to this page. An update check too, once
// the last one is old.
window.addEventListener('focus',() => { loadSignIn(true); loadAppUpdate(true); });
document.addEventListener('visibilitychange',() => { if (document.visibilityState === 'visible') loadSignIn(true); });
const CUSTOM_MODEL = '--custom--';
const modelMetadata = (providerId = $('provider').value) => providerFor(providerId)?.model_options || {models:[],efforts:[],detail:''};
const pickerProvider = prefix => prefix === 'clash-' ? $('clash-challenger').value : $('provider').value;
const selectedModel = (prefix = '') => ($(prefix+'model-choice').value === CUSTOM_MODEL ? $(prefix+'model').value.trim() : $(prefix+'model-choice').value) || null;
function supportedEfforts(prefix = '') { const metadata = modelMetadata(pickerProvider(prefix)); const model = (metadata.models || []).find(item => item.id === selectedModel(prefix)); return [...new Set((model?.efforts || metadata.efforts || []).filter(value => typeof value === 'string' && value))]; }
function refreshEffortChoices(requested = $('thinking-effort').value, prefix = '') {
  const defaultOption = el('option','','Provider default'); defaultOption.value = ''; $(prefix+'thinking-effort').replaceChildren(defaultOption);
  for (const effort of supportedEfforts(prefix)) { const option = el('option','',effort === 'ultracode' ? 'Ultracode (workflows)' : effort.charAt(0).toUpperCase() + effort.slice(1)); option.value = effort; $(prefix+'thinking-effort').append(option); }
  $(prefix+'thinking-effort').value = supportedEfforts(prefix).includes(requested) ? requested : '';
}
function refreshModelChoices(model = selectedModel(), effort = $('thinking-effort').value, prefix = '') {
  const models = modelMetadata(pickerProvider(prefix)).models || []; const defaultOption = el('option','','Provider default'); defaultOption.value = ''; $(prefix+'model-choice').replaceChildren(defaultOption);
  for (const item of models) { const option = el('option','',item.label || item.id); option.value = item.id; $(prefix+'model-choice').append(option); }
  const custom = el('option','','Custom model…'); custom.value = CUSTOM_MODEL; $(prefix+'model-choice').append(custom);
  $(prefix+'model-choice').value = model ? models.some(item => item.id === model) ? model : CUSTOM_MODEL : '';
  $(prefix+'model').value = $(prefix+'model-choice').value === CUSTOM_MODEL ? model : '';
  refreshEffortChoices(effort,prefix);
}
const maxAgentCount = () => state.bootstrap?.runtime?.max_agents || 40;
function modelRouting() {
  if (!$('model-routing-enabled').checked || fleetSelected() || clashSelected()) return null;
  return Object.fromEntries(['plan','edit'].map(role => [role,{model:selectedModel(`routing-${role}-`),thinking_effort:$(`routing-${role}-thinking-effort`).value || null}]));
}
function restoreModelRouting(routing) {
  $('model-routing-enabled').checked = Boolean(routing);
  for (const role of ['plan','edit']) refreshModelChoices(routing?.[role]?.model || null,routing?.[role]?.thinking_effort || null,`routing-${role}-`);
}
for (const [role,label] of [['plan','Planning'],['edit','Editing']]) {
  const prefix = `routing-${role}-`; const row = el('div');
  row.innerHTML = `<label class="field">${label} model<select id="${prefix}model-choice" form="session-form"></select></label><label class="field" id="${prefix}custom-field" hidden>${label} custom model ID<input id="${prefix}model" form="session-form" maxlength="120" disabled></label><label class="field">${label} thinking effort<select id="${prefix}thinking-effort" form="session-form"></select></label>`;
  $('model-routing-fields').append(row);
  $(prefix+'model-choice').addEventListener('change',() => { refreshEffortChoices($(prefix+'thinking-effort').value,prefix); updateControls(); });
  $(prefix+'model').addEventListener('input',() => { refreshEffortChoices($(prefix+'thinking-effort').value,prefix); updateControls(); });
  $(prefix+'thinking-effort').addEventListener('change',updateControls);
}
$('model-routing-enabled').addEventListener('change',() => {
  if ($('model-routing-enabled').checked) {
    const current = {model:selectedModel(),thinking_effort:$('thinking-effort').value || null};
    restoreModelRouting({plan:current,edit:current});
  } else if (state.selected?.workflow === 'native') $('mode').value = state.selected.mode;
  updateControls();
});
function updateModelRoutingControls(locked) {
  const routing = modelRouting(); const enabled = Boolean(routing); let valid = true;
  $('model-routing-config').hidden = fleetSelected() || clashSelected();
  $('model-routing-enabled').disabled = locked;
  $('model-routing-fields').hidden = !enabled;
  $('model-routing-summary').textContent = enabled ? 'Planning + editing' : 'One model';
  for (const role of ['plan','edit']) {
    const prefix = `routing-${role}-`; const custom = $(prefix+'model-choice').value === CUSTOM_MODEL;
    $(prefix+'model-choice').disabled = locked || !enabled;
    $(prefix+'model').disabled = locked || !enabled || !custom; $(prefix+'model').required = enabled && custom;
    $(prefix+'custom-field').hidden = !custom;
    $(prefix+'thinking-effort').disabled = locked || !enabled || !supportedEfforts(prefix).length;
    const value = routing?.[role];
    if (enabled && (custom && (!value.model || value.model.length > 120 || value.model.startsWith('-') || /[\x00-\x1f\x7f]/.test(value.model)) || value?.thinking_effort === 'ultracode' && !$('agents-enabled').checked)) valid = false;
  }
  let hint = '';
  if (enabled) {
    const role = $('workflow').value === 'sdd' ? $('sdd-phase').value === 'implement' ? 'edit' : 'plan' : $('mode').value;
    const selected = routing[role];
    refreshModelChoices(selected.model,selected.thinking_effort);
    hint = `Next turn: ${role === 'edit' ? 'Editing' : 'Planning'} · ${selected.model || 'provider default'} · ${selected.thinking_effort || 'default effort'}. Changes apply when you send.`;
  }
  $('model-routing-hint').textContent = hint;
  showError('model-routing-error',valid ? '' : 'Check both custom model IDs. Ultracode requires additional agents for either role.');
  return valid;
}
const defaultAgentCount = () => state.bootstrap?.runtime?.default_agent_count || 3;
const validAgentCount = () => Number.isInteger($('agent-count').valueAsNumber) && $('agent-count').valueAsNumber >= 1 && $('agent-count').valueAsNumber <= maxAgentCount();
const fleetSelected = () => $('workflow').value === 'fleet-review';
const clashMetadata = () => state.bootstrap?.clash || {workflows:['native','review'],stages:{},max_rounds:3,default_rounds:2};
const clashAvailable = () => (clashMetadata().workflows || ['native','review']).includes($('workflow').value);
const clashSelected = () => $('clash-enabled').checked && clashAvailable();
const isClashSession = session => Boolean(session?.clash);
const fleetMetadata = () => state.bootstrap?.runtime?.fleet || {available:false,detail:'Fleet review is not configured in this runner.',lenses:[],max_worker_timeout:3600};
const fleetDryRun = () => fleetSelected() && $('fleet-dry-run').checked;
const fleetBudgetSupported = () => fleetDryRun() || $('provider').value === 'claude';
const maxFleetTimeout = () => fleetMetadata().max_worker_timeout || 3600;
const humanLabel = value => typeof value === 'string' ? value.replace(/[_-]+/g,' ').replace(/^./,character => character.toUpperCase()) : '';
const fleetStageName = stage => ({scope:'Define scope',review:'Run reviewers',collect:'Collect findings',gate:'Report decision',record:'Save report'})[stage] || humanLabel(stage);
const fleetLensName = id => (fleetMetadata().lenses || []).find(lens => lens.id === id)?.name || humanLabel(id);
// One native form shared by session launches and Creator checkpoints.
function budgetEditor(id, save) {
  const details=el('details','fleet-report budget-editor'); const summary=el('summary','','Budgets'); const summaryValue=el('span','budget-summary'); summaryValue.id=id+'-summary'; summary.append(summaryValue); details.append(summary);
  details.append(el('p','knowledge-note budget-scope')); details.lastChild.id=id+'-scope';
  const fields=el('div','page-toolbar');
  for(const [key,label,min,max,step,placeholder] of [['usd','Money (USD)',.01,1000,'any','No cap'],['tokens','Total tokens',1,1000000000,1,'No cap'],['seconds','Time (seconds)',1,86400,1,'No time limit']]) {
    const field=el(key==='tokens'?'div':'label','field'), name=el('span','field-label',key==='tokens'?undefined:label); field.append(name); const input=el('input'); input.id=id+'-'+key;
    if(key==='tokens') { const text=el('label','',label); text.htmlFor=input.id; const explain=el('button','explain','?'); explain.type='button'; explain.setAttribute('aria-label','About total tokens'); explain.setAttribute('aria-expanded','false'); explain.setAttribute('aria-controls',id+'-tokens-explain'); name.append(text,explain); } input.type='number'; input.min=min; input.max=max; input.step=step; input.placeholder=placeholder; input.setAttribute('aria-describedby',id+'-hint '+id+'-error'); field.append(input); fields.append(field);
  }
  const tokensExplain=el('p','knowledge-note','Tokens include input, output and cached input.'); tokensExplain.id=id+'-tokens-explain'; tokensExplain.hidden=true;
  details.append(fields,tokensExplain);
  const hint=el('p','knowledge-note'); hint.id=id+'-hint'; details.append(hint);
  const usage=el('p','knowledge-note'); usage.id=id+'-usage'; usage.setAttribute('role','status'); details.append(usage);
  const error=el('p','error-text'); error.id=id+'-error'; error.setAttribute('role','alert'); error.hidden=true; details.append(error);
  if(save) { const button=el('button','button','Save budgets'); button.id=id+'-save'; button.type='button'; button.addEventListener('click',save); details.append(button); }
  {
    const split=el('div','budget-split'); split.id=id+'-split'; split.append(el('h4','','Split per agent'));
    const summary=el('p','knowledge-note'); summary.id=id+'-agent-summary'; summary.setAttribute('role','status'); split.append(summary);
    const fields=el('div','page-toolbar');
    for(const [key,label,min,max,step] of [['usd','USD per agent',.000001,1000,.000001],['tokens','Tokens per agent',1,1000000000,1],['seconds','Seconds per agent',1,86400,1]]) {
      const labelNode=el('label','field',label), input=el('input'); input.id=id+'-agent-'+key; input.type='number'; input.min=min; input.max=max; input.step=step; input.placeholder=key==='seconds'?'Use defaults':'No cap'; input.setAttribute('aria-describedby',id+'-agent-note '+id+'-agent-error'); input.addEventListener('input',()=>updateBudgetForm(id)); labelNode.append(input); fields.append(labelNode);
    }
    const apply=el('button','button','Fill totals above'); apply.type='button'; apply.id=id+'-agent-apply'; apply.addEventListener('click',()=>applyAgentBudgets(id));
    const note=el('p','knowledge-note'); note.id=id+'-agent-note';
    const error=el('p','error-text'); error.id=id+'-agent-error'; error.setAttribute('role','alert'); error.hidden=true;
    split.append(fields,note,error,apply); details.append(split);
  }
  if(id==='session-budgets') details.open=true;
  $(id).append(details);
}
function updateBudgetForm(id) { if(id==='session-budgets') updateControls(); else if(id==='creator-budgets') creatorControls(); else updateCreatorBudgetControls(); }
function agentBudgetPopulation(id) {
  const session=id==='session-budgets', run=id==='creator-run-budgets'?creatorUi.detail?.run:null;
  const fleet=session && fleetSelected();
  const helpers=run ? run.agent_count : $(session?'agent-count':'creator-count').valueAsNumber;
  const enabled=run ? run.agents_enabled : $(session?'agents-enabled':'creator-agents').checked;
  const valid=Number.isInteger(helpers) && helpers>=1 && helpers<=maxAgentCount();
  const count=fleet ? fleetUi.lenses.size : enabled ? valid?1+helpers:0 : 1;
  const concurrent=fleet ? valid?Math.min(count,helpers):0 : count;
  return {fleet,count,concurrent,waves:concurrent?Math.ceil(count/concurrent):0};
}
function perAgentTotals(values,population) {
  const {count,waves,fleet}=population;
  return {usd:values.usd===null?null:Math.round(values.usd*1000000)*count/1000000,
          tokens:values.tokens===null?null:values.tokens*count,
          seconds:values.seconds===null?null:values.seconds*(fleet?waves:1)};
}
function agentBudgetControls(id,provider,locked) {
  const population=agentBudgetPopulation(id), {fleet,count,concurrent,waves}=population;
  $(id+'-split').hidden=!fleet && count===1; if($(id+'-split').hidden) for(const key of ['usd','tokens','seconds']) $(id+'-agent-'+key).value='';
  const supported=fleet ? fleetBudgetSupported() : provider==='claude';
  for(const key of ['usd','tokens','seconds']) $(id+'-agent-'+key).disabled=locked || key==='usd' && !supported;
  $(id+'-agent-seconds').max=fleet?maxFleetTimeout():86400;
  const totals=id==='session-budgets'?sessionBudgets():readBudgets(id), seconds=totals.seconds;
  const money=totals.usd===null?'uncapped':`~$${Math.floor(totals.usd*1000000/count)/1000000}`;
  const tokens=totals.tokens===null?'uncapped':`${Math.floor(totals.tokens/count).toLocaleString()} tokens${totals.tokens%count ? ` (${totals.tokens%count} unallocated)` : ''}`;
  const time = seconds===null ? (fleet ? `up to ${fleetSettings().worker_timeout}s per reviewer, no shared time limit` : 'no time limit') : `up to ${fleet?Math.min(seconds,fleetSettings().worker_timeout):seconds}s within ${seconds}s shared time`;
  const entered=[totals.usd,totals.tokens,totals.seconds].some(value=>value!==null) || ['usd','tokens','seconds'].some(key=>$(id+'-agent-'+key).value);
  $(id+'-agent-summary').textContent=!count || !concurrent ? 'Select reviewers and a valid concurrency limit.' : entered ? `Per ${fleet?'reviewer':'agent'} (${count} ${fleet?(count===1?'reviewer':'reviewers'):count===1?'agent':'agents'}, ${concurrent} at once): ${money}, ${tokens}, ${time}.` : '';
  $(id+'-agent-note').textContent=fleet ? `Multiplied by ${count} ${count===1?'reviewer':'reviewers'}; time × ${waves} ${waves===1?'wave':'waves'}, plus setup. USD shares can shrink after retries.` : `Multiplied by ${count} ${count===1?'agent':'agents'} (main + helpers); time stays one shared deadline. Per-agent shares are not enforced.`;
  const values=readBudgets(id+'-agent'); if(!supported) values.usd=null;
  const formed=perAgentTotals(values,population);
  const valid=count>0 && concurrent>0 && ['usd','tokens','seconds'].every(key=>{const input=$(id+'-agent-'+key); return input.disabled || input.validity.valid && (!input.value || Number.isFinite(input.valueAsNumber));}) &&
    (formed.usd===null || formed.usd>=.01 && formed.usd<=1000) && (formed.tokens===null || Number.isSafeInteger(formed.tokens) && formed.tokens>=1 && formed.tokens<=1000000000) && (formed.seconds===null || Number.isInteger(formed.seconds) && formed.seconds>=1 && formed.seconds<=86400);
  showError(id+'-agent-error',valid?'':'Use valid per-agent values whose totals stay within USD 0.01–1,000, tokens 1–1,000,000,000 and time 1–86,400 seconds.');
  $(id+'-agent-apply').disabled=locked || !valid;
  return formed;
}
function applyAgentBudgets(id) {
  if($(id+'-agent-apply').disabled) return;
  const provider=id==='session-budgets'?$('provider').value:id==='creator-budgets'?$('creator-provider').value:creatorUi.detail.run.provider;
  const formed=agentBudgetControls(id,provider,false);
  for(const key of ['usd','tokens','seconds']) $(id+'-'+key).value=formed[key] ?? '';
  if(id==='session-budgets' && fleetSelected()) { $('fleet-budget').value=formed.usd ?? ''; $('fleet-worker-timeout').value=$(id+'-agent-seconds').value || Math.min(300,maxFleetTimeout()); }
  updateBudgetForm(id);
}
function readBudgets(id) { return Object.fromEntries(['usd','tokens','seconds'].map(key=>{const input=$(id+'-'+key); return [key,!input.value.trim() ? null : input.valueAsNumber];})); }
function restoreBudgets(id, budgets, revision) { if($(id).dataset.revision===revision) return; for(const key of ['usd','tokens','seconds']) $(id+'-'+key).value=budgets?.[key] ?? ''; $(id).dataset.revision=revision; for(const key of ['usd','tokens','seconds']) $(id+'-agent-'+key).value=''; }
function budgetUsage(usage, label = 'Last launch (reported)') { return usage ? `${label}: ${Number.isFinite(usage.tokens) ? `${fmt.exact(usage.tokens).text} tokens` : 'tokens not reported'} · ${Number.isFinite(usage.cost_usd) ? fmt.cost(usage.cost_usd).text : 'cost not reported'} · ${usage.seconds}s${usage.limit_reached ? ' · '+usage.limit_reached+' limit reached' : ''}.` : ''; }
function budgetControls(id, provider, locked, usage, fleet=false) {
  for(const key of ['usd','tokens','seconds']) $(id+'-'+key).disabled=locked || key==='usd' && (provider!=='claude' || fleet);
  $(id+'-usd').parentElement.hidden=fleet; if(provider!=='claude' && !fleet) $(id+'-usd').value='';
  $(id+'-seconds').placeholder='No time limit';
  $(id+'-hint').textContent=(fleet ? 'Set the USD cap under Reviewers.' : provider==='claude' ? 'USD is a stop threshold: the last request can overshoot, so leave headroom.' : 'No USD cap for this provider.')+' Token limits are soft; usage may arrive only at completion.';
  $(id+'-scope').textContent=id==='session-budgets' ? fleet ? 'Shared by all reviewers, retries included.' : 'Limits apply to each launch.' : 'Limits apply to each scan and generate phase.';
  $(id+'-summary').textContent=budgetSummary(id==='session-budgets' ? sessionBudgets() : readBudgets(id));
  $(id+'-usage').textContent=budgetUsage(usage);
  agentBudgetControls(id,provider,locked);
  const valid=['usd','tokens','seconds'].every(key=>{const input=$(id+'-'+key); const value=input.valueAsNumber; return input.disabled || input.validity.valid && (!input.value || Number.isFinite(value) && (key==='usd' || Number.isInteger(value)));});
  showError(id+'-error',valid ? '' : 'Use USD 0.01–1,000, whole tokens 1–1,000,000,000 and whole seconds 1–86,400; blank means no cap.');
  return valid;
}
function sessionBudgets() { const budgets=readBudgets('session-budgets'); if(fleetSelected()) budgets.usd=fleetSettings().budget_usd; return budgets; }
function budgetsDirty() { return Boolean(state.selected && (JSON.stringify(sessionBudgets())!==JSON.stringify(state.selected.budgets) || isFleetSession(state.selected) && fleetSettings().worker_timeout!==state.selected.fleet.worker_timeout)); }
function sessionBudgetControls() {
  const locked=!state.bootstrap || state.authFailed || Boolean(state.pending || state.loading || active(state.selected) || state.selected?.creator);
  const valid=budgetControls('session-budgets',$('provider').value,locked,state.selected?.budget_usage,fleetSelected());
  $('session-budgets-save').hidden=!state.selected || Boolean(state.selected.creator);
  $('session-budgets-save').disabled=locked || !valid || !budgetsDirty() || fleetSelected() && !updateFleetSettingsControls();
  return valid;
}
async function saveSessionBudgets() {
  if($('session-budgets-save').disabled) return;
  const session=state.selected; const body={budgets:sessionBudgets(),revision:session.budget_revision,...(isFleetSession(session)?{worker_timeout:fleetSettings().worker_timeout}:{})};
  state.pending='budgets'; stopPolling(); updateControls();
  try { const data=await api(`/api/sessions/${session.id}/budgets`,{method:'POST',body}); upsert(data.session); restoreBudgets('session-budgets',data.session.budgets,data.session.id+':'+data.session.budget_revision); if(isFleetSession(data.session)) restoreFleetSettings(data.session.fleet); showError('composer-error',''); }
  catch(error) { showError('composer-error',textError(error)); }
  finally { state.pending=null; updateControls(); }
}
budgetEditor('session-budgets',saveSessionBudgets); budgetEditor('creator-budgets'); budgetEditor('creator-run-budgets',()=>creatorAction('budgets'));
for(const key of ['usd','tokens','seconds']) $('session-budgets-'+key).addEventListener('input',updateControls);
const resultUi={epoch:0,timer:null,data:null,pending:false};
const deliveryUi={sid:null,epoch:0,data:null,preview:null,paths:new Set()};
function invalidateDeliveryPreview() {
  deliveryUi.preview=null; $('delivery-commit-preview').hidden=true; $('delivery-preview').hidden=true; deliveryControls();
}
function deliveryControls() {
  const locked=!state.bootstrap || state.authFailed || !deliveryUi.data?.available || deliveryUi.sid!==state.selectedId || active(state.selected) || state.loading || Boolean(state.pending) || resultUi.pending;
  const message=$('delivery-message').value.trim(); const tooLong=new TextEncoder().encode(message).length>4000;
  $('delivery-message').setCustomValidity(tooLong?'Use a commit message of at most 4,000 UTF-8 bytes.':'');
  $('delivery-message').setAttribute('aria-invalid',String(tooLong)); showError('delivery-message-error',tooLong?'Use a commit message of at most 4,000 UTF-8 bytes.':'');
  for(const input of $('delivery-options').querySelectorAll('input,select')) input.disabled=locked;
  $('delivery-commit-preview-button').disabled=locked || !deliveryUi.paths.size || !message || tooLong;
  const requiresCheck=$('delivery-check-required').checked;
  const validCheck=$('delivery-check-command').value.trim() && new TextEncoder().encode($('delivery-check-command').value).length<=4000 && $('delivery-check-timeout').validity.valid;
  $('delivery-check-command').disabled=locked || !requiresCheck; $('delivery-check-timeout').disabled=locked || !requiresCheck;
  $('delivery-preview-button').disabled=locked || !$('delivery-source').value || !$('delivery-target').value || (requiresCheck && !validCheck);
  $('delivery-commit').disabled=locked || deliveryUi.preview?.kind!=='commit' || deliveryUi.preview.can_apply!==true;
  const preview=deliveryUi.preview, check=preview?.target_check;
  $('delivery-run-check').hidden=!preview?.check; $('delivery-run-check').disabled=locked || !preview?.check || !preview.can_apply;
  $('delivery-cancel-check').hidden=!check || !['queued','running'].includes(check.status);
  $('delivery-cancel-check').disabled=Boolean(state.pending) || resultUi.pending;
  $('delivery-apply').disabled=locked || preview?.kind!=='transfer' || preview.can_apply!==true || (Boolean(preview.check) && (check?.status!=='passed' || check.workspace_changed || check.candidate!==preview.candidate));
}
function resetDeliveryRead() {
  ++deliveryUi.epoch; deliveryUi.data=null; invalidateDeliveryPreview(); $('delivery-options').hidden=true;
  if(deliveryUi.sid!==state.selectedId) {
    deliveryUi.sid=state.selectedId; deliveryUi.paths.clear(); $('delivery-message').value=''; $('delivery-source').replaceChildren(); $('delivery-target').replaceChildren(); $('delivery-status').textContent='';
  }
  $('delivery-availability').textContent='Loading delivery options…'; showError('delivery-error','');
}
function renderDelivery(data) {
  deliveryUi.data=data; deliveryUi.paths.clear(); $('delivery-options').hidden=!data.available && !data.preview;
  $('delivery-availability').textContent=data.available ? data.notice || 'Create a local commit, then review its transfer to a local branch.' : data.reason || 'Delivery is unavailable for this session.';
  if(!data.available) { if(data.preview) renderDeliveryPreview(data.preview); deliveryControls(); return; }
  $('delivery-workspace').textContent=`Worktree: ${data.workspace} · Branch: ${data.branch} · HEAD: ${data.head}`;
  const files=$('delivery-files'); files.replaceChildren();
  for(const file of data.files) {
    const label=el('label','checkbox'); label.style.cssText='white-space:normal;padding:9px 0'; const input=el('input'); input.type='checkbox'; input.value=file.path;
    label.append(input,el('span','file-path',file.path),el('span','file-status',file.status)); files.append(label);
    input.addEventListener('change',()=>{ if(input.checked) deliveryUi.paths.add(file.path); else deliveryUi.paths.delete(file.path); ++deliveryUi.epoch; invalidateDeliveryPreview(); $('delivery-status').textContent=''; });
  }
  if(!data.files.length) files.append(el('p','knowledge-note','No pending files to commit.'));
  const source=$('delivery-source').value, target=$('delivery-target').value;
  setOptions($('delivery-source'),[{sha:'',subject:'Choose a commit'},...data.commits],item=>item.sha?`${item.sha.slice(0,12)} · ${item.subject}`:item.subject,item=>item.sha,source);
  setOptions($('delivery-target'),[{name:'',head:''},...data.branches],item=>item.name?`${item.name} · ${item.head.slice(0,12)}`:'Choose a local branch',item=>item.name,target);
  if(data.preview) renderDeliveryPreview(data.preview);
  deliveryControls();
}
async function loadDelivery(sid) {
  const epoch=deliveryUi.epoch;
  try {
    const data=await api(`/api/sessions/${encodeURIComponent(sid)}/delivery`);
    if(epoch!==deliveryUi.epoch || sid!==state.selectedId || !isResultView(state.view)) return;
    renderDelivery(data);
  } catch(error) { if(epoch===deliveryUi.epoch && sid===state.selectedId && isResultView(state.view)) { $('delivery-availability').textContent='Refresh results to reload delivery options.'; showError('delivery-error',textError(error)); } }
}
function renderDeliveryPreview(preview) {
  deliveryUi.preview=preview; const commit=preview.kind==='commit', prefix=commit?'delivery-commit':'delivery-preview';
  $(prefix+'-summary').textContent=commit ? `Commit message: ${preview.message} · Worktree HEAD: ${preview.source_head}` : `Commit ${preview.commit} → ${preview.target_branch} · Target HEAD: ${preview.target_head} · Resulting commit: ${preview.candidate || 'unavailable'} · Worktree HEAD: ${preview.source_head}`;
  $(prefix+'-files').textContent=preview.files.map(file=>`${file.status}  ${file.path}`).join('\n') || 'No changed files.';
  $(prefix+'-diff').textContent=preview.diff || 'No text diff available.';
  $(prefix+'-checks').textContent=(preview.checks || []).length ? 'Recorded source-workspace checks (they do not certify this transfer): '+preview.checks.map(check=>`${check.status} · ${check.command} · exit ${check.exit_code ?? 'unknown'}`).join('; ') : 'No source-workspace checks recorded.';
  $(prefix+'-note').textContent=[preview.notice,...(preview.conflicts || []),preview.can_apply?'Review this content before confirming. A selection change requires a new preview.':'This preview cannot be applied.'].filter(Boolean).join('\n');
  if(!commit) {
    const check=preview.target_check;
    $('delivery-check-status').textContent=preview.check ? `${check?.status.toUpperCase() || 'NOT RUN'} · ${preview.check.command} · ${preview.check.timeout}s limit · Commit: ${preview.candidate || 'unavailable'}${check?' · exit: '+(check.exit_code ?? 'unavailable'):''}` : 'Target verification was not selected for this preview.';
    $('delivery-check-output').textContent=check?.output || '';
    $('delivery-check-required').checked=Boolean(preview.check);
    $('delivery-check-command').value=preview.check?.command || '';
    $('delivery-check-timeout').value=preview.check?.timeout || 300;
  }
  $(commit?'delivery-commit-preview':'delivery-preview').hidden=false; deliveryControls();
}
async function deliveryAction(action,body) {
  const sid=state.selectedId, epoch=++deliveryUi.epoch, pending='delivery-'+action;
  const mutation=action==='commit' || action==='apply'; invalidateDeliveryPreview(); clearTimeout(resultUi.timer); state.pending=pending;
  const current=()=>epoch===deliveryUi.epoch && sid===state.selectedId && isResultView(state.view);
  showError('delivery-error',''); $('delivery-status').textContent=mutation ? action==='commit'?'Creating the reviewed local commit…':'Applying the reviewed commit locally…' : 'Preparing a preview…'; updateControls(); resultControls();
  try {
    const data=await api(`/api/sessions/${encodeURIComponent(sid)}/delivery`,{method:'POST',body:{action,...body}});
    if(!current()) return;
    if(mutation) {
      if(data.ok!==true || !data.commit) throw new Error('The runner returned an incomplete result. Refresh results and inspect local commits before retrying.');
      $('delivery-status').textContent=`${data.detail || (action==='commit'?'Created local commit.':'Applied commit locally.')} Commit: ${data.commit}${data.branch?' · Branch: '+data.branch:''}`;
      if(action==='commit') $('delivery-message').value='';
      await loadResults();
    } else {
      if(!data.preview_id || data.kind!==(action==='preview_commit'?'commit':'transfer')) throw new Error('The runner returned an incomplete preview. Refresh results and try again.');
      renderDeliveryPreview(data); $('delivery-status').textContent=data.can_apply?'Preview ready. Review the files and diff before confirming.':'Preview is blocked; review the details below.';
    }
  } catch(error) {
    if(current()) { if(mutation) { deliveryUi.data=null; $('delivery-availability').textContent='Refresh results before preparing another delivery preview.'; } $('delivery-status').textContent=''; showError('delivery-error',error.status===0 && mutation ? 'The connection was lost. The operation may have completed. Refresh results and inspect the local commit or target branch before preparing another preview.' : textError(error)+' Prepare a new preview before retrying.'); }
  } finally { if(state.pending===pending) state.pending=null; updateControls(); resultControls(); }
}
$('delivery-message').addEventListener('input',()=>{ ++deliveryUi.epoch; invalidateDeliveryPreview(); $('delivery-status').textContent=''; });
for(const id of ['delivery-source','delivery-target','delivery-check-required','delivery-check-command','delivery-check-timeout']) $(id).addEventListener('change',()=>{ ++deliveryUi.epoch; invalidateDeliveryPreview(); $('delivery-status').textContent=''; });
for(const id of ['delivery-check-command','delivery-check-timeout']) $(id).addEventListener('input',()=>{ ++deliveryUi.epoch; invalidateDeliveryPreview(); });
$('delivery-commit-form').addEventListener('submit',event=>{ event.preventDefault(); if(!$('delivery-commit-preview-button').disabled) deliveryAction('preview_commit',{snapshot_id:deliveryUi.data.snapshot_id,paths:[...deliveryUi.paths],message:$('delivery-message').value.trim()}); });
$('delivery-preview-form').addEventListener('submit',event=>{ event.preventDefault(); if(!$('delivery-preview-button').disabled) deliveryAction('preview_transfer',{commit:$('delivery-source').value,target_branch:$('delivery-target').value,check:$('delivery-check-required').checked ? {command:$('delivery-check-command').value.trim(),timeout:$('delivery-check-timeout').valueAsNumber} : null}); });
$('delivery-commit').addEventListener('click',()=>{ if(!$('delivery-commit').disabled) deliveryAction('commit',{preview_id:deliveryUi.preview.preview_id}); });
$('delivery-apply').addEventListener('click',()=>{ if(!$('delivery-apply').disabled) deliveryAction('apply',{preview_id:deliveryUi.preview.preview_id}); });
async function targetCheckAction(cancel=false) {
  const sid=state.selectedId, preview=deliveryUi.preview;
  if(!preview || (cancel ? $('delivery-cancel-check').disabled : $('delivery-run-check').disabled)) return;
  state.pending='target-check'; resultControls(); updateHeader();
  try {
    const data=await api(`/api/sessions/${encodeURIComponent(sid)}/${cancel?'cancel':'delivery'}`,{method:'POST',body:cancel?{}:{action:'check',preview_id:preview.preview_id}});
    if(sid!==state.selectedId || !isResultView(state.view)) return;
    upsert(data.session); await loadResults();
  } catch(error) { if(sid===state.selectedId) showError('delivery-error',textError(error)); }
  finally { if(state.pending==='target-check') state.pending=null; resultControls(); updateControls(); }
}
$('delivery-run-check').addEventListener('click',()=>targetCheckAction());
$('delivery-cancel-check').addEventListener('click',()=>targetCheckAction(true));
function resultControls() {
  const locked=!state.selected || active(state.selected) || state.loading || Boolean(state.pending) || resultUi.pending;
  $('check-start').disabled=locked || !resultUi.data?.snapshot || Boolean(state.selected?.creator) || ! $('check-command').value.trim() || !$('check-timeout').validity.valid;
  $('check-command').disabled=locked || Boolean(state.selected?.creator); $('check-timeout').disabled=locked;
  $('results-refresh').disabled=resultUi.pending || Boolean(state.pending); deliveryControls();
}
function renderResults(data) {
  const snapshot=data.snapshot;
  $('results-workspace').textContent=data.workspace; $('delivery-section').hidden=state.selected?.workspace!=='worktree';
  const checking=data.checks.some(check=>['queued','running'].includes(check.status));
  $('results-status').textContent=checking ? 'Verification is queued or running; agent outcome stays separate.' : active(state.selected) ? 'Agent is active. Refresh the diff after this launch finishes.' : `Agent: ${sessionStatusLabel(state.selected)}.`;
  $('results-base').textContent=snapshot?.available ? `Branch: ${snapshot.branch || 'detached HEAD'} · Base: ${snapshot.base}${snapshot.baseline_recorded?' (first launch)':' (current HEAD; original baseline unavailable)'}${snapshot.preexisting_changes ? ' · Workspace already had uncommitted changes at first launch.' : ''}` : '';
  $('results-diff-note').textContent=snapshot ? (snapshot.message || '')+(snapshot.available && !snapshot.complete?' Preview is truncated or omits file contents; verification freshness cannot be certified.':'') : 'Diff is unavailable while a launch is active.';
  $('results-files').textContent=snapshot?.files?.map(file=>`${file.status}  ${file.path}`).join('\n') || 'No changed files listed.';
  $('results-diff').textContent=snapshot?.diff || 'No text diff available.';
  $('results-usage-note').textContent=data.notice;
  // A total with no report behind it reads —; launches without a report are named, never added as 0.
  const totals=data.totals, total=(value,format)=>value.unknown_launches && !value.reported ? fmt.unknown() : format(value.reported);
  const unreported=[['tokens',totals.tokens],['cost',totals.cost_usd]].filter(([,value])=>value.unknown_launches).map(([name,value])=>`${name} not reported for ${value.unknown_launches} launch${value.unknown_launches===1?'':'es'}`);
  // The totals line is a live region, so it is rewritten only when the numbers change.
  if($('results-totals').dataset.key!==JSON.stringify(totals)) { $('results-totals').dataset.key=JSON.stringify(totals);
    $('results-totals').replaceChildren(numberNode(total(totals.tokens,value=>fmt.exact(value,' tokens'))),' · ',numberNode(total(totals.cost_usd,fmt.cost)),' · ',numberNode(total(totals.seconds,value=>fmt.exact(Math.round(value*10)/10,' s'))),...unreported.map(note=>` · ${note}`)); }
  renderContextUsage(data);
  if(data.launches.length) keyedRender($('results-launches'),data.launches,run=>run.id,run=>{ const details=el('details','fleet-report'); details.append(el('summary','',`${run.started_at} · ${run.kind} · ${run.status}`),el('p','knowledge-note',`${run.settings.provider} / ${run.settings.model || 'default model'} / ${run.settings.thinking_effort || 'default effort'}`),...[contextLaunchLine(run)].filter(Boolean),el('p','',budgetUsage(run.usage,'Reported')),el('pre','fleet-report-text',JSON.stringify({sdd:run.settings.sdd,mode:run.settings.mode,model_routing:run.settings.model_routing,budgets:run.settings.budgets,agent_budget_plan:run.settings.agent_budget_plan,started_at:run.started_at,finished_at:run.finished_at},null,2))); return details; });
  else $('results-launches').replaceChildren(el('p','knowledge-note','No launches recorded by this version yet.'));
  const holder=$('results-checks');
  keyedRender(holder,data.checks,check=>check.id,check=>{
    const details=el('details','fleet-report'); details.dataset.id=check.id; details.open=['queued','running','failed','timed_out','output_limit'].includes(check.status);
    const current=snapshot?.complete && check.snapshot_complete && snapshot.id===check.snapshot_id && !check.workspace_changed;
    details.append(el('summary','',`${check.status.toUpperCase()} · ${check.command}`),el('p','knowledge-note',`${check.started_at || check.created_at} · exit code: ${check.exit_code ?? 'unavailable'} · ${check.seconds ?? 'unknown'}s · ${check.kind==='target'?`Target ${check.target_branch} · base ${check.target_head} · tested commit ${check.candidate}${check.workspace_changed?' · Check changed the workspace':''}`:current?'Matches the current diff':snapshot && check.snapshot_id && snapshot.id!==check.snapshot_id || check.workspace_changed?'Workspace changed since this check':'Freshness unavailable'}`),el('pre','fleet-report-text',check.output || 'Waiting for output…')); return details;
  },check=>JSON.stringify([check,snapshot?.id,snapshot?.complete]));
  if(!data.checks.length) holder.replaceChildren(el('p','knowledge-note','No independent checks run yet.'));
  resultControls(); updateHeader();
  renderViewTabs();
}
async function loadResults() {
  clearTimeout(resultUi.timer); if(!state.selectedId || !isResultView(state.view)) return;
  const sid=state.selectedId, epoch=++resultUi.epoch; resultUi.pending=true; resetDeliveryRead(); resultControls();
  try {
    const current=await api(`/api/sessions/${sid}`);
    if(epoch!==resultUi.epoch || !isResultView(state.view) || sid!==state.selectedId) return;
    upsert(current.session); updateHeader();
    const data=await api(`/api/sessions/${sid}/results`);
    if(epoch!==resultUi.epoch || !isResultView(state.view) || sid!==state.selectedId) return;
    resultUi.data=data; renderResults(data); showError('results-error',''); await loadDelivery(sid);
    if(epoch!==resultUi.epoch || !isResultView(state.view) || sid!==state.selectedId) return;
    if(active(state.selected)) resultUi.timer=setTimeout(loadResults,1200);
  } catch(error) { if(epoch===resultUi.epoch) { $('delivery-availability').textContent='Refresh results to reload delivery options.'; showError('results-error',textError(error)); } }
  finally { if(epoch===resultUi.epoch) { resultUi.pending=false; resultControls(); } }
}
$('results-refresh').addEventListener('click',loadResults);
$('check-command').addEventListener('input',resultControls); $('check-timeout').addEventListener('input',resultControls);
$('check-form').addEventListener('submit',async event=>{
  event.preventDefault(); if($('check-start').disabled) return;
  const sid=state.selectedId, body={command:$('check-command').value.trim(),timeout:$('check-timeout').valueAsNumber,snapshot_id:resultUi.data.snapshot.id};
  state.pending='check'; resultControls(); updateHeader();
  try { const data=await api(`/api/sessions/${sid}/check`,{method:'POST',body}); upsert(data.session); await loadResults(); }
  catch(error) { showError('results-error',textError(error)); }
  finally { state.pending=null; resultControls(); updateControls(); }
});
function fleetSettings() { return {lenses:[...fleetUi.lenses],dry_run:$('fleet-dry-run').checked,budget_usd:fleetBudgetSupported() && $('fleet-budget').value.trim() ? $('fleet-budget').valueAsNumber : null,worker_timeout:$('fleet-worker-timeout').valueAsNumber}; }
function renderFleetLenses() {
  const lenses = fleetMetadata().lenses || [];
  if (!fleetUi.initialized && lenses.length) { fleetUi.lenses = new Set(lenses.map(lens => lens.id)); fleetUi.initialized = true; }
  const holder = $('fleet-lenses'); holder.replaceChildren();
  for (const lens of lenses) { const label = el('label','checkbox'); const input = el('input'); input.type = 'checkbox'; input.value = lens.id; input.checked = fleetUi.lenses.has(lens.id); input.setAttribute('form','session-form'); label.append(input,document.createTextNode(lens.name)); holder.append(label); input.addEventListener('change',() => { if (input.checked) fleetUi.lenses.add(lens.id); else fleetUi.lenses.delete(lens.id); updateControls(); }); }
  if (!lenses.length) holder.append(el('span','fleet-note','No reviewers are available.'));
  $('fleet-worker-timeout').max = String(maxFleetTimeout());
}
function restoreFleetSettings(settings) { fleetUi.lenses = new Set(Array.isArray(settings?.lenses) ? settings.lenses : (fleetMetadata().lenses || []).map(lens => lens.id)); fleetUi.initialized = true; $('fleet-dry-run').checked = settings?.dry_run === true; $('fleet-budget').value = typeof settings?.budget_usd === 'number' ? String(settings.budget_usd) : ''; $('fleet-worker-timeout').value = Number.isInteger(settings?.worker_timeout) ? String(settings.worker_timeout) : String(Math.min(300,maxFleetTimeout())); renderFleetLenses(); }
function resetFleetProgress() { fleetUi.reviewers.clear(); fleetUi.stage = null; fleetUi.resultKey = null; fleetUi.reportOpenedFor = null; $('fleet-report').open = false; showError('fleet-action-error',''); }
function updateFleetSettingsControls() {
  const selected = fleetSelected(); const locked = !state.bootstrap || !selected || Boolean(state.selectedId || state.pending || state.loading); const settings = fleetSettings(); const errors = [];
  $('fleet-settings').hidden = !selected; document.querySelector('.configuration').classList.toggle('fleet-configuration',selected); $('sessions-view').classList.toggle('fleet-session',isFleetSession(state.selected));
  for (const input of $('fleet-lenses').querySelectorAll('input')) input.disabled = locked;
  $('fleet-dry-run').disabled = locked; const budgetLocked = !state.bootstrap || !selected || Boolean(state.pending || state.loading || active(state.selected)); $('fleet-worker-timeout').disabled = budgetLocked; $('fleet-budget').disabled = budgetLocked || !fleetBudgetSupported();
  if (selected && !settings.lenses.length) errors.push('Select at least one reviewer.');
  const timeoutValid = Number.isInteger(settings.worker_timeout) && settings.worker_timeout >= 1 && settings.worker_timeout <= maxFleetTimeout();
  if (selected && !timeoutValid) errors.push(`Choose a whole-number reviewer timeout from 1 to ${maxFleetTimeout()} seconds.`);
  const budgetValid = $('fleet-budget').disabled || $('fleet-budget').validity.valid && (settings.budget_usd === null || Number.isFinite(settings.budget_usd) && settings.budget_usd >= .01 && settings.budget_usd <= 1000);
  if (selected && !budgetValid) errors.push('Enter a USD budget from 0.01 to 1,000, or leave it blank.');
  $('fleet-worker-timeout').setAttribute('aria-invalid',String(selected && !timeoutValid)); $('fleet-budget').setAttribute('aria-invalid',String(selected && !budgetValid));
  $('fleet-budget-hint').textContent = fleetDryRun() ? 'Optional shared budget for simulated review costs.' : fleetBudgetSupported() ? 'Shared by all reviewers. The last request can overshoot, so leave headroom.' : 'A native USD budget is available for Claude reviewers.';
  $('fleet-runtime-detail').textContent = fleetDryRun() ? 'Simulated reviewers; no model reviews code.' : fleetMetadata().available === false ? fleetMetadata().detail || 'Fleet review is unavailable.' : '';
  showError('fleet-config-error',errors.join(' '));
  return !selected || fleetMetadata().available === true && !errors.length;
}
const textError = error => error.message || 'The local runner could not complete this request.';
function showError(id, message) { $(id).textContent = message || ''; $(id).hidden = !message; }
function connectionError(error) { if (appUpdate.restarting) return; if ([401,403].includes(error.status)) state.authFailed = true; $('connection-message').textContent = state.authFailed ? 'The local connection token is no longer accepted. Reconnect before sending another request.' : 'The local runner is not responding. Check that it is still running, then reconnect.'; $('connection-banner').hidden = false; updateControls(); }
async function api(path, options = {}) {
  // The token goes with every request: some reads (a CLI's command list) start native processes and require it.
  const headers = { Accept:'application/json', 'X-Harness-Token':state.bootstrap?.csrf || '' };
  if (options.body !== undefined) headers['Content-Type'] = 'application/json';
  let response;
  try { response = await fetch(path,{ method:options.method || 'GET', headers, credentials:'same-origin', cache:'no-store', signal:options.signal, ...(options.body !== undefined ? {body:JSON.stringify(options.body)} : {}) }); }
  catch (error) { if (error.name === 'AbortError') throw error; const failure = new Error('Could not reach the local runner. Refresh sessions before retrying an uncertain request.'); failure.status = 0; connectionError(failure); throw failure; }
  let data; try { data = await response.json(); } catch (_) { data = {}; }
  if (!response.ok) { const failure = new Error(typeof data.error === 'string' ? data.error : typeof data.message === 'string' ? data.message : `The local runner returned HTTP ${response.status}.`); failure.status = response.status; if ([401,403].includes(response.status)) connectionError(failure); throw failure; }
  return data;
}
const mobileNav = window.matchMedia('(max-width:760px)');
function setMenu(open) { const wasOpen = $('sidebar').dataset.open === 'true'; open = Boolean(open && mobileNav.matches); $('sidebar').dataset.open = String(open); $('scrim').dataset.open = String(open); $('sidebar').inert = mobileNav.matches && !open; $('main').inert = open; $('menu-toggle').setAttribute('aria-expanded',String(open)); $('menu-toggle').setAttribute('aria-label',open ? 'Close navigation' : 'Open navigation'); if (open) $('new-session').focus(); else if (wasOpen && mobileNav.matches) $('menu-toggle').focus(); }
mobileNav.addEventListener('change',() => setMenu(false)); setMenu(false);
$('menu-toggle').addEventListener('click',() => setMenu($('sidebar').dataset.open !== 'true'));
$('scrim').addEventListener('click',() => setMenu(false));
document.addEventListener('keydown',event => {
  if (event.key !== 'Escape') return;
  setMenu(false);
  if (sessionOptions.open && ($('panel-'+sessionOptions.open).contains(document.activeElement) || document.activeElement === $('option-'+sessionOptions.open))) { const chip = $('option-'+sessionOptions.open); sessionOptions.open = null; renderSessionOptions(); chip.focus(); }
});
document.querySelector('.skip').addEventListener('click',event => { event.preventDefault(); $('main').focus(); });
// Five sections; each groups related views behind tabs. Every view has its own #/view address. The project is
// chosen in Sessions (the sidebar Project selector and the Project chip), as DeepSeek Harness chooses a workspace.
const resultViews = ['changes','checks','usage'], isResultView = view => resultViews.includes(view);
const viewGroups = {sessions:['sessions',...resultViews],systems:['systems','system-changes'],knowledge:['memory-use','brain','memory','context'],skills:['skills','create-skill'],accelerators:['accelerators','creator','kit3','setup']};
const viewLabels = {sessions:'Conversation',systems:'Services','system-changes':'Changes',changes:'Changes',checks:'Checks',usage:'Usage','memory-use':'Memory use',brain:'Project Brain',memory:'Memory bank',context:'Context files',skills:'Library','create-skill':'Create skill',accelerators:'Overview',creator:'Infrastructure Creator',kit3:'Open Source Kit',setup:'Install into project'};
const groupOf = view => Object.keys(viewGroups).find(group => viewGroups[group].includes(view));
const lastViewInGroup = {};
function viewFromHash() { let view = ''; try { view = decodeURIComponent(location.hash.replace(/^#\/?/,'')); } catch (_) { return null; } if (view === 'results') view = 'changes'; return groupOf(view) ? view : null; }
// Restoring a saved session must not rewrite the address the page was opened with.
const initialView = viewFromHash(); let initialRouteApplied = false, restoringRoute = true;
// Result tabs carry a count once results are loaded: changed files, passed checks, reported cost.
function tabLabel(view) {
  const data = isResultView(state.view) ? resultUi.data : null;
  if (view === 'changes' && Array.isArray(data?.snapshot?.files)) return `Changes · ${data.snapshot.files.length}`;
  if (view === 'checks' && data?.checks?.length) return `Checks · ${data.checks.filter(check => check.status === 'passed').length}/${data.checks.length}`;
  if (view === 'usage' && data?.totals?.cost_usd?.reported > 0) return `Usage · ${fmt.cost(data.totals.cost_usd.reported).text}`;
  // Memory use carries a count only for trouble: chunks past review or citing changed files, and stalled promotions.
  if (view === 'memory-use' && typeof memoryUse !== 'undefined' && memoryUse.count > 0) return `Memory use · ${memoryUse.count}`;
  return viewLabels[view];
}
function renderViewTabs() {
  const views = viewGroups[groupOf(state.view)].filter(view => !isResultView(view) || state.selectedId), refocus = $('view-tabs').contains(document.activeElement);
  $('view-tabs').hidden = views.length < 2; $('view-tabs').replaceChildren();
  for (const view of views) {
    const tab = el('button','view-tab',tabLabel(view)); tab.type = 'button'; if (view === state.view) tab.setAttribute('aria-current','page');
    tab.addEventListener('click',() => openView(view)); $('view-tabs').append(tab);
  }
  if (refocus) $('view-tabs').querySelector('[aria-current="page"]')?.focus();
}
function openView(view, record = true) {
  if (view === state.view) { setMenu(false); return; }
  const fromResults = isResultView(state.view); if (isResultView(view) && !fromResults) resultUi.data = null;
  setView(view,record); if (view === 'sessions' && fromResults) pollSession(state.epoch);
}
for (const button of document.querySelectorAll('.navigation [data-nav]')) button.addEventListener('click',() => { const group = button.dataset.nav; openView(group === 'sessions' ? 'sessions' : lastViewInGroup[group] || viewGroups[group][0]); });
window.addEventListener('popstate',() => { const view = viewFromHash() || 'sessions'; openView(isResultView(view) && !state.selectedId ? 'sessions' : view,false); });
function setView(view, record = true) {
  const previousView = state.view;
  if (state.view === 'memory' && view !== 'memory') { cancelMemoryRequests(); cancelKnowledgeRead('memory'); }
  if (state.view === 'memory-use' && view !== 'memory-use') leaveMemoryUse();
  if (state.view === 'brain' && view !== 'brain') { cancelBrainRequests(); cancelKnowledgeRead('brain'); }
  if (state.view === 'creator' && view !== 'creator') { clearTimeout(creatorUi.timer); ++creatorUi.epoch; }
  if (groupOf(state.view) === 'systems' && groupOf(view) !== 'systems' && typeof systemUi !== 'undefined') { clearTimeout(systemUi.timer); ++systemUi.epoch; systemUi.controller?.abort(); }
  if(isResultView(state.view) && !isResultView(view)) { clearTimeout(resultUi.timer); ++resultUi.epoch; resultUi.pending=false; ++deliveryUi.epoch; deliveryUi.data=null; invalidateDeliveryPreview(); }
  state.view = view; lastViewInGroup[groupOf(view)] = view;
  const viewId = isResultView(view) ? 'results-view' : `${view}-view`; for (const node of document.querySelectorAll('#main > section.view')) node.hidden = node.id !== viewId;
  if (isResultView(view)) $('results-view').dataset.tab = view;
  for (const button of document.querySelectorAll('.navigation [data-nav]')) { if (button.dataset.nav === groupOf(view)) button.setAttribute('aria-current','page'); else button.removeAttribute('aria-current'); }
  if (record && !restoringRoute && viewFromHash() !== view) { if (!location.hash) history.replaceState(null,'','#/'+previousView); history.pushState(null,'','#/'+view); }
  if (isResultView(view) && !isResultView(previousView)) loadResults();
  if (view === 'creator') openCreator();
  if (groupOf(view) === 'systems' && groupOf(previousView) !== 'systems' && typeof openSystems === 'function') openSystems();
  if (view === 'setup') openSetup();
  if (view === 'context') loadContext();
  if (view === 'memory') loadMemory();
  if (view === 'memory-use') openMemoryUse();
  if (view === 'brain') loadBrain();
  if (view === 'skills') openSkills();
  if (view === 'create-skill') openCreateSkill();
  if (view === 'sessions') loadProjectGit();
  if (view === 'kit3' && !$('kit3-frame').getAttribute('src')) $('kit3-frame').src = '/kit3/';
  renderViewTabs(); updateHeader(); setMenu(false);
  // A control that hid its own view must not leave focus on the page body.
  if (document.activeElement === document.body || document.activeElement?.closest('.view[hidden]')) $('view-heading').focus();
}
for (const button of document.querySelectorAll('[data-view]')) button.addEventListener('click',() => setView(button.dataset.view));
function setOptions(select, items, getLabel, getValue, previous) {
  select.replaceChildren();
  for (const item of items) { const option = el('option','',getLabel(item)); option.value = getValue(item); if (item.available === false) option.disabled = true; select.append(option); }
  if (previous && [...select.options].some(option => option.value === previous && !option.disabled)) select.value = previous;
  else { const first = [...select.options].find(option => !option.disabled); if (first) select.value = first.value; }
  if (!items.length) { const option = el('option','','None available'); option.value = ''; select.append(option); }
}
const projectSelects = {sessions:'project',changes:'project',checks:'project',usage:'project',context:'context-project','memory-use':'memory-use-project',brain:'brain-project',memory:'memory-project',skills:'skills-project','create-skill':'create-skill-project',setup:'setup-project',creator:'creator-project',accelerators:'accelerator-project',systems:'system-project','system-changes':'system-project'};
const currentProject = () => groupOf(state.view) === 'sessions' ? state.selected?.project_id || $('project').value : projectSelects[state.view] ? $(projectSelects[state.view]).value : $('project-switcher').value;
// Every view follows the sidebar project; views in the middle of an operation keep theirs until it ends.
function switchProject(id) {
  if (id === CHOOSE_FOLDER) { $('project-switcher').value = currentProject(); if (!state.pending) chooseSessionProject($('project-switcher')); return; }
  if (!projectFor(id) || state.pending) { $('project-switcher').value = currentProject(); return; }
  if (state.selectedId && state.selected?.project_id !== id) newSession('',id,groupOf(state.view) === 'sessions');
  else if ($('project').value !== id) { $('project').value = id; $('project').dispatchEvent(new Event('change')); }
  syncProjectSelects(id,true); if (state.view === 'accelerators') renderAccelerators();
  updateHeader();
}
// Opening a session or Creator run of another project moves the other views along; only the visible view reloads.
function syncProjectSelects(id, reloadVisible = false) {
  const busy = {'memory-project':knowledgeState.pending,'brain-project':knowledgeState.pending,'skills-project':Boolean(skillsState.pending),'create-skill-project':Boolean(createSkillState.pending),'setup-project':Boolean(setupState.pending || setupState.registerPending),'creator-project':creatorUi.pending,
    'system-project':typeof systemDiscovery !== 'undefined' && Boolean(systemUi.pending || (inSystems() && systemUi.detail?.active) || systemEditor.pending || systemEditor.dirty || systemDiscovery.active)};
  for (const select of ['context-project','memory-use-project','memory-project','brain-project','skills-project','create-skill-project','setup-project','creator-project','accelerator-project','system-project']) if (!busy[select] && projectFor(id) && $(select).value !== id) $(select).value = id;
  // The draft's own handler may already have moved a select, so the visible view reloads regardless.
  const visible = projectSelects[state.view];
  if (reloadVisible && visible && visible !== 'project' && !busy[visible] && $(visible).value === id) $(visible).dispatchEvent(new Event('change'));
}
$('project-switcher').addEventListener('change',() => switchProject($('project-switcher').value));
function populateSettings() {
  const boot = state.bootstrap;
  const draftModel = selectedModel(); const draftEffort = $('thinking-effort').value;
  for (const id of ['project','context-project','accelerator-project']) setProjectChoices($(id),boot.projects,$(id).value);
  if (!(typeof systemUi !== 'undefined' && systemUi.pending)) setProjectChoices($('system-project'),boot.projects,$('system-project').value || $('project').value);
  setProjectChoices($('project-switcher'),boot.projects,currentProject());
  for (const scope of ['memory','brain']) if (!knowledgeState.pending) setProjectChoices($(`${scope}-project`),boot.projects,$(`${scope}-project`).value || $('project').value);
  setProjectChoices($('memory-use-project'),boot.projects,$('memory-use-project').value || $('project').value);
  if (!setupState.pending && !setupState.registerPending) setProjectChoices($('setup-project'),boot.projects,$('setup-project').value || $('project').value,true);
  setOptions($('provider'),boot.providers,providerLabel,item => item.id,$('provider').value);
  if (!skillsState.pending) { const previousProject = $('skills-project').value; setProjectChoices($('skills-project'),boot.projects,previousProject || $('project').value); if (previousProject && previousProject !== $('skills-project').value) invalidateSkillsPreview(); }
  if (!createSkillState.pending) { const previousProject = $('create-skill-project').value; setProjectChoices($('create-skill-project'),boot.projects,previousProject || $('project').value); if (previousProject && previousProject !== $('create-skill-project').value) invalidateCreateSkillPreview(); }
  setOptions($('workflow'),boot.workflows,item => item.name,item => item.id,$('workflow').value);
  setOptions($('sdd-phase'),boot.sdd_phases || [],item => item.name,item => item.id,$('sdd-phase').value || 'specify');
  $('clash-rounds').max = String(boot.clash?.max_rounds || 3);
  if (!$('clash-rounds').dataset.initialized) { $('clash-rounds').value = String(boot.clash?.default_rounds || 2); $('clash-rounds').dataset.initialized = 'true'; }
  renderClashChallengers();
  const runtime = boot.runtime || {};
  renderFleetLenses();
  $('agent-count').max = String(maxAgentCount());
  if (!$('agent-count').dataset.initialized) { $('agent-count').value = defaultAgentCount(); $('agent-count').dataset.initialized = 'true'; }
  $('runtime-label').textContent = `Local runner${runtime.max_active ? ` · ${runtime.max_active} active ${runtime.max_active === 1 ? 'slot' : 'slots'}` : ''}`;
  if (state.selected) applySessionSettings(state.selected,false); else restoreModelRouting(modelRouting());
  if (active(state.selected) || isFleetSession(state.selected)) refreshModelChoices(state.selected.model || null,state.selected.thinking_effort || null);
  else refreshModelChoices(draftModel,draftEffort);
  updateControls(); renderAccelerators(); loadProjectGit(true);
}
async function bootstrap() {
  $('reconnect').disabled = true; $('connecting').hidden = false;
  try {
    const boot = await api('/api/bootstrap');
    if (!boot.csrf || !Array.isArray(boot.projects) || !Array.isArray(boot.providers)) throw new Error('The runner returned an incomplete workspace configuration.');
    state.bootstrap = boot; state.authFailed = false; state.sessions = boot.sessions || []; boot.workflows = boot.workflows || []; boot.accelerators = boot.accelerators || [];
    $('connection-banner').hidden = true; populateSettings(); renderHistory(); loadSignIn();
    appUpdate.status = boot.update || null; renderAppUpdate(); loadAppUpdate(true);
    if (!sessionPreferencesReady) await restoreSessionPreferences();
    else if (state.selectedId) { stopPolling(); await pollSession(state.epoch); }
    let routed = false;
    if (!initialRouteApplied) { initialRouteApplied = true; if (initialView && initialView !== state.view) { setView(isResultView(initialView) && !state.selectedId ? 'sessions' : initialView,false); routed = true; } restoringRoute = false; if (viewFromHash() !== state.view) history.replaceState(null,'','#/'+state.view); }
    if (routed) return;
    if (state.view === 'creator') openCreator();
    if (groupOf(state.view) === 'systems' && typeof openSystems === 'function') openSystems();
    if (state.view === 'setup') openSetup(true);
    if (state.view === 'context') loadContext();
    if (state.view === 'memory') loadMemory();
    if (state.view === 'brain') loadBrain();
    if (state.view === 'skills') openSkills(true);
    if (state.view === 'create-skill') openCreateSkill(true);
  } catch (error) { $('connection-message').textContent = textError(error); $('connection-banner').hidden = false; $('history').replaceChildren(el('p','history-empty','The local runner is unavailable.')); }
  finally { $('connecting').hidden = true; $('reconnect').disabled = false; updateControls(); }
}
$('reconnect').addEventListener('click',bootstrap);
const sessionPreferencesKey = 'harness.sessions.preferences.v1';
const sessionPreferenceFields = ['provider','workflow','mode','agents-enabled','agent-count','workspace','worktree-branch','sdd-feature','sdd-phase','clash-enabled','clash-challenger','clash-rounds','fleet-dry-run','fleet-budget','fleet-worker-timeout','brain-link-kind','brain-link-review','brain-link-task-id','brain-link-goal','brain-link-query',...['usd','tokens','seconds'].flatMap(key => ['session-budgets-'+key,'session-budgets-agent-'+key])];
let sessionPreferencesReady = false, sessionDraftProject = null, defaultSessionPreferences = null, restoringSessionPreferences = false, preferenceEpoch = 0;
function readSessionPreferences() {
  try {
    const raw = localStorage.getItem(sessionPreferencesKey);
    if (!raw || raw.length > 500000) return {};
    const saved = JSON.parse(raw);
    return saved && typeof saved === 'object' && !Array.isArray(saved) ? saved : {};
  } catch (_) { return {}; }
}
function captureSessionPreferences() {
  // Until the knowledge roots load, their selects are empty: keep the root and task this project last saved.
  const settled = brainLinkLoaded() && !brainLinkDraft.error, kept = settled ? null : readSessionPreferences().drafts?.[sessionDraftProject || $('project').value];
  return {fields:Object.fromEntries(sessionPreferenceFields.map(id => [id,$(id).type === 'checkbox' ? $(id).checked : $(id).value])),model:selectedModel(),effort:$('thinking-effort').value,routing:modelRouting(),lenses:[...fleetUi.lenses],
    bank:settled ? $('brain-link-bank').value : typeof kept?.bank === 'string' ? kept.bank : '',task:settled ? $('brain-link-task').value : typeof kept?.task === 'string' ? kept.task : '',
    worktree:projectWorktreeState.preferred};
}
function saveSessionPreferences(projectId = sessionDraftProject || $('project').value) {
  if (!sessionPreferencesReady || restoringSessionPreferences || !state.bootstrap) return;
  if (!state.selectedId && brainLinkStrict() && (brainLinkDraft.loading || brainLinkDraft.error)) return;
  if (state.selectedId) projectId = $('project').value;
  if (!projectFor(projectId)) return;
  const saved = readSessionPreferences();
  const drafts = saved.drafts && typeof saved.drafts === 'object' && !Array.isArray(saved.drafts) ? saved.drafts : {};
  if (!state.selectedId) drafts[projectId] = captureSessionPreferences();
  // Only form preferences and selection IDs; credentials and run content stay out.
  try { localStorage.setItem(sessionPreferencesKey,JSON.stringify({project_id:projectId,session_id:state.selectedId,drafts})); } catch (_) { /* Storage may be disabled or full. */ }
}
async function restoreProjectPreferences(projectId) {
  const epoch = ++preferenceEpoch; restoringSessionPreferences = true; sessionDraftProject = projectId;
  const saved = readSessionPreferences().drafts?.[projectId];
  const text = value => typeof value === 'string' && value.length <= 120 ? value : null;
  try {
    resetBrainLink();
    for (const draft of [defaultSessionPreferences,saved]) {
      // A draft saved before project memory ran by itself chose an existing task for nearly everyone; it keeps the new default.
      const before = draft?.fields && !('brain-link-review' in draft.fields);
      for (const id of sessionPreferenceFields) {
        if (before && id === 'brain-link-kind') continue;
        const input = $(id), value = draft?.fields?.[id];
        if (input.type === 'checkbox') { if (typeof value === 'boolean') input.checked = value; }
        else if (typeof value === 'string' && value.length <= (input.maxLength > 0 ? input.maxLength : 4000)) {
          if (input.tagName !== 'SELECT' || [...input.options].some(option => option.value === value) || id === 'provider') input.value = value;
        }
      }
    }
    const draft = saved && typeof saved === 'object' ? saved : defaultSessionPreferences;
    const routing = draft?.routing;
    restoreModelRouting(routing?.plan && routing?.edit ? Object.fromEntries(['plan','edit'].map(role => [role,{model:text(routing[role].model),thinking_effort:text(routing[role].thinking_effort)}])) : null);
    refreshModelChoices(text(draft?.model),text(draft?.effort));
    fleetUi.lenses = new Set((Array.isArray(draft?.lenses) ? draft.lenses : []).filter(id => (fleetMetadata().lenses || []).some(lens => lens.id === id))); renderFleetLenses();
    // The picker re-applies the saved choice on its next render, even for the list it already shows.
    projectWorktreeState.preferred = /^[a-f0-9]{16}$/.test(saved?.worktree || '') ? saved.worktree : ''; $('existing-worktree').value = ''; delete $('existing-worktree').dataset.key;
    loadProjectGit(true); updateControls();
    await loadBrainLinkTasks(true);
    if (epoch !== preferenceEpoch || state.selectedId) return;
    // The root and task are this project's own choices; another project's defaults name neither.
    const own = saved && typeof saved === 'object' ? saved : null;
    if (own?.bank && ![...$('brain-link-bank').options].some(option => option.value === own.bank)) {
      // A named task must not silently move to another root; the default just uses the root the project has.
      if (brainLinkStrict()) { $('brain-link-bank').value = ''; $('brain-link-task').value = ''; brainLinkDraft.error = 'The saved knowledge root is unavailable. Choose a root to continue.'; }
      updateControls(); return;
    }
    if (typeof own?.bank === 'string' && [...$('brain-link-bank').options].some(option => option.value === own.bank) && $('brain-link-bank').value !== own.bank) { $('brain-link-bank').value = own.bank; await loadBrainLinkTasks(); }
    if (epoch !== preferenceEpoch || state.selectedId) return;
    // A missing saved task must not silently select a different task.
    $('brain-link-task').value = typeof own?.task === 'string' ? own.task : '';
    updateControls();
  } finally { if (epoch === preferenceEpoch) { restoringSessionPreferences = false; saveSessionPreferences(); } }
}
async function restoreSessionPreferences() {
  const saved = readSessionPreferences();
  defaultSessionPreferences ||= captureSessionPreferences();
  if (typeof saved.session_id === 'string' && state.sessions.some(session => session.id === saved.session_id && !session.creator)) await selectSession(saved.session_id);
  else {
    if (typeof saved.project_id === 'string' && projectFor(saved.project_id)) $('project').value = saved.project_id;
    await restoreProjectPreferences($('project').value);
  }
  sessionPreferencesReady = true;
}
for (const event of ['input','change','click']) sessionConfiguration.addEventListener(event,() => saveSessionPreferences());
window.addEventListener('pagehide',() => saveSessionPreferences());
document.addEventListener('visibilitychange',() => { if (document.visibilityState === 'hidden') saveSessionPreferences(); });
function dateLabel(value) { const date = new Date(typeof value === 'number' && value < 1e12 ? value * 1000 : value); return Number.isNaN(date.getTime()) ? '' : date.toLocaleDateString(undefined,{month:'short',day:'numeric'}); }
function openHistorySession(id) {
  const session = state.sessions.find(item => item.id === id); if (!session) return;
  syncProjectSelects(session.project_id);
  if (session.creator) { setView('creator'); $('creator-project').value = session.project_id; loadCreator(session.creator.run_id); } else selectSession(session.id);
}
function renderHistory() {
  const list = $('history');
  if (!state.sessions.length) { list.replaceChildren(el('p','history-empty','Your sessions will appear here.')); return; }
  list.querySelector('.history-empty')?.remove();
  const rows = new Map([...list.querySelectorAll('.history-item')].map(button => [button.dataset.id,button]));
  [...state.sessions].sort((a,b) => String(b.updated_at || '').localeCompare(String(a.updated_at || ''))).forEach((session,index) => {
    let button = rows.get(session.id); rows.delete(session.id);
    if (!button) { button = el('button','history-item'); button.type = 'button'; button.dataset.id = session.id; button.append(el('span','history-title'),el('span','history-meta')); button.addEventListener('click',() => openHistorySession(button.dataset.id)); }
    const title = session.title || 'Untitled session', meta = `${isFleetSession(session) ? 'Fleet · ' : isClashSession(session) ? 'Clash · ' : ''}${sessionStatusLabel(session)}${session.updated_at ? ` · ${dateLabel(session.updated_at)}` : ''}`;
    if (button.title !== title) { button.title = title; button.firstChild.textContent = title; }
    if (button.lastChild.dataset.text !== `${session.status}|${meta}`) { button.lastChild.replaceChildren(el('span',`status-dot ${session.status}`),document.createTextNode(meta)); button.lastChild.dataset.text = `${session.status}|${meta}`; }
    if (session.id === state.selectedId) button.setAttribute('aria-current','true'); else button.removeAttribute('aria-current');
    if (list.children[index] !== button) list.insertBefore(button,list.children[index] || null);
  });
  for (const button of rows.values()) button.remove();
}
function upsert(session) { if (!session?.id) return; const index = state.sessions.findIndex(item => item.id === session.id); if (index < 0) state.sessions.unshift(session); else state.sessions[index] = session; if (state.selectedId === session.id) state.selected = session; renderHistory(); }
async function refreshSessions() { $('refresh-sessions').disabled = true; try { const data = await api('/api/sessions'); state.sessions = data.sessions || []; renderHistory(); } catch (error) { showError('events-error',textError(error)); } finally { $('refresh-sessions').disabled = false; } }
$('refresh-sessions').addEventListener('click',refreshSessions);
function updateHeader() {
  // While an opened session loads, its list entry names it, so the header does not flash 'New session'.
  const session = state.selected || (state.loading && state.selectedId ? state.sessions.find(item => item.id === state.selectedId) || null : null);
  const sessionLine = session ? `${projectFor(session.project_id)?.name || 'Project'} · ${isFleetSession(session) ? 'Fleet review · ' : isClashSession(session) ? `Clash vs ${providerFor(session.clash?.challenger)?.name || session.clash?.challenger || 'challenger'} · ` : ''}${providerFor(session.provider)?.name || session.provider}` : '';
  $('page-title').textContent = groupOf(state.view) === 'sessions' ? session?.title || 'New session' : {systems:'System Orchestration',knowledge:'Knowledge',skills:'Skills',accelerators:'Accelerators'}[groupOf(state.view)];
  $('page-subtitle').textContent = groupOf(state.view) === 'sessions' ? session ? sessionLine : [projectFor($('project').value)?.name,providerFor($('provider').value)?.name].filter(Boolean).join(' · ') : [state.view === 'kit3' ? '' : projectFor(currentProject())?.name,groupOf(state.view) === 'systems' ? $('system-config').value.trim() : ({brain:'Browsing runs no commands.',memory:'Browsing runs no commands.',context:'Files added to a session when Project context is on.'})[state.view]].filter(Boolean).join(' · ');
  $('page-subtitle').hidden = !$('page-subtitle').textContent;
  $('open-kit3').hidden = state.view !== 'kit3';
  if ($('project-switcher').value !== currentProject() && projectFor(currentProject())) $('project-switcher').value = currentProject(); $('project-switcher').disabled = !state.bootstrap || Boolean(state.pending);
  $('session-status').hidden = groupOf(state.view) !== 'sessions' || !session;
  const statusKey = session ? `${session.status}|${sessionStatusLabel(session)}` : ''; if ($('session-status').dataset.key !== statusKey) { $('session-status').dataset.key = statusKey; $('session-status').replaceChildren(); if (session) $('session-status').append(el('span',`status-dot ${session.status}`),document.createTextNode(sessionStatusLabel(session))); }
  $('cancel-session').hidden = groupOf(state.view) !== 'sessions' || !active(session);
  $('cancel-session').disabled = Boolean(state.pending); $('cancel-session').textContent = state.pending === 'cancel' ? 'Cancelling…' : isResultView(state.view) && resultUi.data?.checks.some(check=>['queued','running'].includes(check.status)) ? 'Cancel check' : 'Cancel session';
  const viewName = state.view === 'sessions' ? '' : viewLabels[state.view]; $('view-heading').textContent = viewLabels[state.view];
  // The tab says what the selected session's run is doing, so a person in another window can tell without looking.
  document.title = `${runView.titlePrefix(state.selected)}${viewName && viewName !== $('page-title').textContent ? `${viewName} · ` : ''}${state.view === 'sessions' && session ? session.title : $('page-title').textContent} — AI Infrastructure Harness`;
  runView.icon(state.selected);
}
function updateWorkspaceControls() {
  const projectId = $('project').value; const hasSession = Boolean(state.selectedId); const ready = Boolean(state.bootstrap) && !state.authFailed;
  const data = projectGitState.projectId === projectId ? projectGitState.data : null; const checking = projectGitState.projectId === projectId && projectGitState.pending;
  const worktree = $('workspace').value === 'worktree'; const available = Boolean(data?.is_git && data.head && data.worktree_available && !checking && !projectGitState.error);
  const existing = $('workspace').value === 'existing-worktree'; const listed = renderExistingWorktrees();
  $('workspace').disabled = !ready || hasSession || Boolean(state.pending) || state.loading;
  $('workspace-worktree-option').disabled = !available && !(hasSession && worktree);
  $('workspace-existing-option').disabled = !data?.is_git && !(hasSession && existing);
  $('worktree-branch-field').hidden = hasSession || !worktree;
  $('worktree-branch').disabled = !ready || hasSession || !worktree || Boolean(state.pending) || state.loading;
  $('existing-worktree-field').hidden = hasSession || !existing;
  $('existing-worktree').disabled = !ready || hasSession || !existing || Boolean(state.pending) || state.loading || !listed.choices.length;
  $('existing-worktree-hint').textContent = hasSession || !existing ? '' : listed.choice ? `Folder: ${listed.choice.folder}` : listed.skipped ? `${listed.skipped} more ${listed.skipped === 1 ? 'worktree is' : 'worktrees are'} not listed: a missing or linked folder, no project folder in it, or past the first 200.` : '';
  $('existing-worktree-hint').hidden = !$('existing-worktree-hint').textContent;
  $('refresh-project-git').disabled = !state.bootstrap || !projectId || checking;
  let status = 'Choose a project to check its Git branch.';
  if (checking) status = 'Checking project Git…';
  else if (projectGitState.projectId === projectId && projectGitState.error) status = 'Project Git status unavailable';
  else if (data && !data.is_git) status = 'No Git repository';
  else if (data?.is_git) status = `${data.branch ? `Project branch: ${data.branch}` : data.head ? `Project checkout: detached HEAD · ${data.head.slice(0,8)}` : 'Git repository · no committed HEAD'}${data.head ? data.dirty ? ' · uncommitted changes' : ' · clean' : data.branch ? ' · no commits' : ''}`;
  $('project-git-status').textContent = status;
  $('workspace-hint').textContent = hasSession ? 'Workspace settings stay fixed for this session. Git status above is for the registered project.' : worktree ? `${checking ? 'Checking worktree availability. ' : ''}A new worktree starts from committed HEAD. Uncommitted edits are not copied.` : existing ? `${listed.loading ? 'Listing worktrees. ' : ''}Runs in the chosen checkout as it is: its branch, uncommitted files and the environment you prepared there.` : data?.is_git ? 'Runs in the current project checkout, including uncommitted files and edits.' : 'The agent runs in the selected project folder.';
  let message = '';
  if (!hasSession && worktree && !checking && !available) message = projectGitState.error || (data?.reason || (data && !data.is_git ? 'This project has no Git repository. Choose Current project folder.' : 'A worktree requires an available Git repository with a committed HEAD. Refresh Git or choose Current project folder.'));
  else if (!hasSession && existing && !checking && data && !data.is_git) message = 'This project has no Git repository. Choose Current project folder.';
  else if (!hasSession && existing && !listed.loading) message = listed.error || (!listed.choices.length ? 'Git lists no other worktree of this project. Create one with git worktree add or your own script, then Refresh Git.' : '');
  else if (projectGitState.projectId === projectId && projectGitState.error) message = projectGitState.error;
  if (!hasSession && worktree && $('worktree-branch').value.length > 256) message = 'Use at most 256 characters for the new branch name.';
  showError('workspace-error',message);
  $('workspace-session-info').hidden = !state.selected;
  $('workspace-session-branch').textContent = state.selected ? `Branch when created: ${state.selected.branch || 'none recorded'} · ${workspaceLabel(state.selected.workspace)}` : '';
  $('workspace-session-path').textContent = state.selected ? `Working folder: ${state.selected.project_path || 'not recorded'}` : '';
  return hasSession || $('workspace').value === 'project' || worktree && available && $('worktree-branch').value.length <= 256 || existing && Boolean(listed.choice);
}
// The placeholder names the keys that open the CLI's commands and Codex's skills (composer-commands.js).
function commandCue() {
  const session = state.selected, provider = session?.provider || $('provider').value, workflow = session?.workflow || $('workflow').value;
  if (workflow !== 'native' || (session ? ['fleet','clash','creator','system_run','system_discovery'].some(name => session[name]) : fleetSelected() || clashSelected())) return '';
  return provider === 'codex' ? ' Type / for commands, $ for skills.' : ' Type / for commands.';
}
function workspaceLabel(workspace) { return {worktree:'Git worktree','existing-worktree':'Existing Git worktree'}[workspace] || 'Project folder'; }
// A checkout reads as its folder and what it has checked out; the hint under the picker gives the whole path.
function worktreeLabel(item) {
  const name = item.path.split(/[\\/]/).filter(Boolean).pop() || item.path;
  return `${name} · ${item.branch || (item.head ? `detached ${item.head.slice(0,8)}` : 'no commits')}${item.locked ? ' · locked' : ''}`;
}
// Fills the picker only when the list changes, so a poll never resets a choice; nothing is chosen for the person.
function renderExistingWorktrees() {
  const select = $('existing-worktree'), projectId = $('project').value;
  const own = projectWorktreeState.projectId === projectId, data = own ? projectWorktreeState.data : null, choices = data?.worktrees || [];
  const key = JSON.stringify([projectId,choices.map(item => [item.id,worktreeLabel(item)])]);
  if (select.dataset.key !== key) {
    const wanted = [select.value,projectWorktreeState.preferred].find(id => id && choices.some(item => item.id === id)) || ''; select.dataset.key = key;
    select.replaceChildren(el('option','',choices.length ? 'Choose a worktree…' : 'No other worktrees'),...choices.map(item => { const option = el('option','',worktreeLabel(item)); option.value = item.id; return option; }));
    select.firstChild.value = ''; select.value = wanted;
  }
  return {choices,choice:choices.find(item => item.id === select.value) || null,skipped:data?.skipped || 0,
    loading:own && projectWorktreeState.pending && !data,error:own ? projectWorktreeState.error || data?.reason || '' : ''};
}
async function loadProjectWorktrees(force = false) {
  const projectId = $('project').value;
  if (!force && projectWorktreeState.projectId === projectId && (projectWorktreeState.pending || projectWorktreeState.data || projectWorktreeState.error)) return;
  projectWorktreeState.controller?.abort(); const controller = new AbortController(); const epoch = ++projectWorktreeState.epoch;
  // A refresh keeps the list it replaces, so the picker does not flicker empty.
  if (projectWorktreeState.projectId !== projectId) projectWorktreeState.data = null;
  Object.assign(projectWorktreeState,{projectId,error:null,pending:Boolean(projectId && state.bootstrap),controller});
  if (!projectId || !state.bootstrap) { projectWorktreeState.controller = null; return; }
  try {
    const data = await api(`/api/projects/${encodeURIComponent(projectId)}/worktrees`,{signal:controller.signal});
    if (epoch !== projectWorktreeState.epoch || projectId !== $('project').value) return;
    if (data.project_id !== projectId || typeof data.is_git !== 'boolean' || !Array.isArray(data.worktrees) || !Number.isInteger(data.skipped)
        || data.worktrees.some(item => !/^[a-f0-9]{16}$/.test(item?.id || '') || typeof item.path !== 'string' || typeof item.folder !== 'string')) throw new Error('The runner returned an incomplete worktree list. Refresh Git to try again.');
    projectWorktreeState.data = data;
  } catch (error) { if (error.name !== 'AbortError' && epoch === projectWorktreeState.epoch && projectId === $('project').value) { projectWorktreeState.data = null; projectWorktreeState.error = error.status === 0 ? 'Worktrees could not be listed. Refresh Git to try again.' : textError(error); } }
  finally { if (epoch === projectWorktreeState.epoch) { projectWorktreeState.pending = false; projectWorktreeState.controller = null; updateControls(); } }
}
async function loadProjectGit(force = false) {
  loadProjectWorktrees(force);
  const projectId = $('project').value;
  if (!force && projectGitState.projectId === projectId && (projectGitState.pending || projectGitState.data || projectGitState.error)) return;
  projectGitState.controller?.abort(); const controller = new AbortController(); const epoch = ++projectGitState.epoch;
  projectGitState.projectId = projectId; projectGitState.data = null; projectGitState.error = null; projectGitState.pending = Boolean(projectId && state.bootstrap); projectGitState.controller = controller; updateControls();
  if (!projectId || !state.bootstrap) { projectGitState.controller = null; return; }
  try {
    const data = await api(`/api/projects/${encodeURIComponent(projectId)}/git`,{signal:controller.signal});
    if (epoch !== projectGitState.epoch || projectId !== $('project').value) return;
    if (data.project_id !== projectId || typeof data.is_git !== 'boolean' || typeof data.worktree_available !== 'boolean' || typeof data.dirty !== 'boolean' || (data.branch !== null && typeof data.branch !== 'string') || (data.head !== null && typeof data.head !== 'string')) throw new Error('The runner returned incomplete Git status. Refresh Git before creating a worktree.');
    projectGitState.data = data;
  } catch (error) { if (error.name !== 'AbortError' && epoch === projectGitState.epoch && projectId === $('project').value) projectGitState.error = error.status === 0 ? 'Git status could not be read. Refresh Git to try again.' : textError(error); }
  finally { if (epoch === projectGitState.epoch) { projectGitState.pending = false; projectGitState.controller = null; updateControls(); } }
}
let sddDocumentSession = null;
function sddSettings() { return {feature:$('sdd-feature').value.trim(),phase:$('sdd-phase').value}; }
function updateSddControls() {
  const selected = $('workflow').value === 'sdd'; const settings = sddSettings();
  const locked = !state.bootstrap || state.authFailed || state.pending || state.loading || active(state.selected) || state.selected?.status === 'awaiting_context';
  $('sdd-settings').hidden = !selected;
  $('sdd-feature').disabled = !selected || locked || Boolean(state.selectedId); $('sdd-feature').required = selected;
  $('sdd-phase').disabled = !selected || locked;
  if (selected) $('mode').value = settings.phase === 'review' ? 'plan' : 'edit';
  const valid = /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(settings.feature) && settings.feature.length <= 80 && settings.feature !== 'memory' && (state.bootstrap?.sdd_phases || []).some(phase => phase.id === settings.phase);
  showError('sdd-error',selected && !valid && (settings.feature || sddSlugTouched || $('prompt').value.trim()) ? 'Enter a feature slug using lowercase letters, digits and single hyphens; memory is reserved.' : '');
  $('sdd-hint').textContent = `${(state.bootstrap?.sdd_phases || []).find(phase => phase.id === settings.phase)?.description || ''} Documents: specs/${settings.feature || '<feature>'}/. Feature stays fixed; phase can change between turns.`;
  $('sdd-refresh').disabled = !selected || !state.selected?.sdd || locked;
  if (sddDocumentSession !== state.selectedId) { $('sdd-documents').replaceChildren(); sddDocumentSession = state.selectedId; }
  return !selected || valid;
}
let sddSlugTouched = false;
$('sdd-feature').addEventListener('input',updateControls);
$('sdd-feature').addEventListener('blur',() => { sddSlugTouched = true; updateControls(); });
$('sdd-phase').addEventListener('change',updateControls);
$('sdd-refresh').addEventListener('click',async () => {
  const sid = state.selectedId; if (!sid) return;
  $('sdd-refresh').disabled = true; $('sdd-documents').replaceChildren(el('p','knowledge-note','Reading documents…'));
  try {
    const data = await api(`/api/sessions/${encodeURIComponent(sid)}/sdd`);
    if (sid !== state.selectedId) return;
    const nodes = data.files.map(file => { const details = el('details','fleet-report'); details.append(el('summary','',`${file.path}${file.available ? file.truncated ? ' · truncated' : '' : ' · missing or unreadable'}`)); if (file.available) details.append(el('pre','fleet-report-text',file.text)); return details; });
    $('sdd-documents').replaceChildren(el('p','knowledge-note','Document snapshot. Read again after a phase or external edit.'),...nodes);
  } catch (error) { if (sid === state.selectedId) $('sdd-documents').replaceChildren(el('p','error-text',textError(error))); }
  finally { updateControls(); }
});
const clashUi = {challenger:null,resultKey:null};
function clashSettings() { return {challenger:$('clash-challenger').value,rounds:$('clash-rounds').valueAsNumber,challenger_model:selectedModel('clash-'),challenger_thinking_effort:$('clash-thinking-effort').value || null}; }
function renderClashChallengers(selected = $('clash-challenger').value) {
  const candidates = (state.bootstrap?.providers || []).filter(provider => provider.id !== $('provider').value);
  if ([...$('clash-challenger').options].map(option => option.value).join() !== candidates.map(item => item.id).join() || selected !== $('clash-challenger').value) {
    setOptions($('clash-challenger'),candidates,item => `${item.name}${item.available ? '' : ' · unavailable'}`,item => item.id,selected);
    if (selected && [...$('clash-challenger').options].some(option => option.value === selected)) $('clash-challenger').value = selected;
  }
  if ($('clash-challenger').value !== clashUi.challenger) { clashUi.challenger = $('clash-challenger').value; refreshModelChoices(null,null,'clash-'); }
}
function restoreClashSettings(settings) {
  $('clash-enabled').checked = Boolean(settings);
  renderClashChallengers(settings?.challenger || $('clash-challenger').value);
  $('clash-rounds').value = Number.isInteger(settings?.rounds) ? String(settings.rounds) : String(clashMetadata().default_rounds || 2);
  refreshModelChoices(settings?.challenger_model || null,settings?.challenger_thinking_effort || null,'clash-');
}
function updateClashControls() {
  const session = state.selected; const available = clashAvailable();
  const locked = !state.bootstrap || state.authFailed || Boolean(state.pending) || state.loading || active(session) || session?.status === 'awaiting_context' || fleetDryRun() || isFleetSession(session) || Boolean(session?.creator);
  if (!available) $('clash-enabled').checked = false;
  $('clash-enabled').disabled = locked || !available;
  $('clash-availability').textContent = available ? 'A second provider disputes the result.' : 'Clash is available for Workspace and Review sessions.'; $('clash-availability').hidden = clashSelected();
  const selected = clashSelected();
  $('clash-settings').hidden = !selected;
  if (!selected) { showError('clash-error',''); return true; }
  renderClashChallengers();
  const settings = clashSettings(); const errors = []; const metadata = clashMetadata(); const stage = $('mode').value === 'edit' ? 'implement' : 'review';
  $('clash-challenger').disabled = locked; $('clash-rounds').disabled = locked;
  const custom = $('clash-model-choice').value === CUSTOM_MODEL;
  $('clash-model-choice').disabled = locked; $('clash-model').disabled = locked || !custom; $('clash-model').required = custom; $('clash-custom-field').hidden = !custom;
  $('clash-thinking-effort').disabled = locked || !supportedEfforts('clash-').length;
  for (const option of $('clash-thinking-effort').options) option.disabled = option.value === 'ultracode';
  const challenger = providerFor(settings.challenger);
  if (!settings.challenger || settings.challenger === $('provider').value) errors.push('Choose a challenger that differs from the first participant.');
  else if (challenger?.available === false) errors.push(`${challenger.name} is not available as a challenger. Install and sign in with its CLI, then restart the server.`);
  const maxRounds = metadata.max_rounds || 3; $('clash-rounds').max = String(maxRounds);
  if (!Number.isInteger(settings.rounds) || settings.rounds < 1 || settings.rounds > maxRounds) errors.push(`Choose a whole number of challenge rounds from 1 to ${maxRounds}.`);
  const model = settings.challenger_model;
  if (custom && !(model && model.length <= 120 && !model.startsWith('-') && !/[\x00-\x1f\x7f]/.test(model))) errors.push('Enter a challenger model ID of 1–120 characters, without a leading dash or control characters.');
  if (settings.challenger_thinking_effort === 'ultracode') errors.push('The challenger cannot use Ultracode.');
  $('clash-hint').textContent = `${stage === 'implement' ? 'Implementation clash' : 'Review clash'}: ${(metadata.stages || {})[stage] || ''}${session ? ' Rounds, the challenger and its model can change between cycles.' : ''}`;
  showError('clash-error',errors.join(' '));
  return !errors.length;
}
function renderClashSession() {
  const session = state.selected; const result = isClashSession(session) ? session.clash_result : null;
  $('clash-outcome').hidden = !result || typeof result !== 'object';
  if (!result || typeof result !== 'object') { clashUi.resultKey = null; return; }
  const stage = result.stage === 'review' ? 'Review' : 'Implementation';
  const protagonist = providerFor(result.protagonist?.provider)?.name || result.protagonist?.provider || 'First participant';
  const challenger = providerFor(result.challenger?.provider)?.name || result.challenger?.provider || 'Challenger';
  const outcome = result.status === 'finished' ? ({converged:'converged — the challenger accepted',unresolved:'unresolved — the challenger still rejects',incomplete:'incomplete — a turn did not finish'})[result.outcome] || humanLabel(result.outcome) : active(session) ? 'in progress' : 'interrupted';
  $('clash-outcome-title').textContent = `${stage} clash · ${outcome}`;
  $('clash-outcome-summary').textContent = `${protagonist} (${result.protagonist?.role || 'first participant'}) vs ${challenger} (challenger) · cycle ${result.cycle} · round ${result.round} of ${result.rounds} · challenger verdict: ${result.verdict || 'none yet'}${typeof result.message === 'string' && result.message ? ` · ${result.message}` : ''}`;
  $('clash-verdict-note').hidden = !result.verdict_note; $('clash-verdict-note').textContent = typeof result.verdict_note === 'string' ? result.verdict_note : '';
  const key = JSON.stringify([result.items,result.turns,result.report,result.status]);
  if (key === clashUi.resultKey) return;
  clashUi.resultKey = key; $('clash-items').replaceChildren();
  const items = Array.isArray(result.items) ? result.items.filter(item => item && typeof item === 'object' && !Array.isArray(item)) : [];
  if (!items.length) $('clash-items').append(el('p','fleet-note',result.stage === 'review' ? 'No findings recorded yet.' : 'No objections recorded yet.'));
  for (const item of items) {
    const block = el('article','fleet-finding'); const heading = el('div','fleet-finding-heading'); const severity = typeof item.severity === 'string' ? item.severity : 'unrated'; const status = typeof item.status === 'string' ? item.status : '';
    heading.append(el('span','',`#${item.id}`),el('span',`fleet-severity ${['high','medium','low'].includes(severity) ? severity : ''}`,humanLabel(severity)),el('span',`clash-state ${/^[a-z]+$/.test(status) ? status : ''}`,humanLabel(status) || 'Unknown'));
    if (typeof item.file === 'string' && item.file) heading.append(el('span','fleet-finding-location',`${item.file}${Number.isInteger(item.line) && item.line > 0 ? `:${item.line}` : ''}`));
    heading.append(el('span','',`raised by ${item.origin === 'challenger' ? challenger : protagonist}`));
    block.append(heading,el('p','fleet-finding-claim',typeof item.claim === 'string' ? item.claim : ''));
    const history = Array.isArray(item.history) ? item.history.filter(entry => entry && typeof entry === 'object') : [];
    if (typeof item.evidence === 'string' && item.evidence || history.length > 1) {
      const details = el('details','fleet-evidence'); details.append(el('summary','','Evidence and exchange')); const lines = [];
      if (typeof item.evidence === 'string' && item.evidence) lines.push(`Evidence: ${item.evidence}`);
      for (const entry of history.slice(1)) lines.push(`Cycle ${entry.cycle} · round ${entry.round} · ${entry.actor === 'challenger' ? challenger : protagonist}: ${humanLabel(entry.action)}${typeof entry.note === 'string' && entry.note ? ` — ${entry.note}` : ''}`);
      details.append(el('pre','',lines.join('\n'))); block.append(details);
    }
    $('clash-items').append(block);
  }
  $('clash-turn-list').replaceChildren();
  for (const turn of Array.isArray(result.turns) ? result.turns : []) {
    if (!turn || typeof turn !== 'object') continue;
    const cost = fleetCost(turn.cost_usd); const provider = providerFor(turn.provider)?.name || turn.provider;
    $('clash-turn-list').append(el('p','fleet-note',`Cycle ${turn.cycle} · ${turn.round ? `round ${turn.round}` : 'opening'} · ${provider} (${turn.role}) · ${turn.ok ? 'completed' : `failed${typeof turn.error === 'string' && turn.error ? `: ${turn.error}` : ''}`}${turn.verdict ? ` · verdict ${turn.verdict}` : ''}${cost ? ` · ${cost}` : ''}${typeof turn.seconds === 'number' && Number.isFinite(turn.seconds) ? ` · ${turn.seconds.toFixed(1)}s` : ''}${typeof turn.summary === 'string' && turn.summary ? ` — ${turn.summary}` : ''}`));
  }
  const report = typeof result.report === 'string' ? result.report : '';
  $('clash-report').hidden = !report.trim(); $('clash-report-text').textContent = report;
}
$('clash-enabled').addEventListener('change',updateControls); $('clash-rounds').addEventListener('input',updateControls);
$('clash-challenger').addEventListener('change',() => { renderClashChallengers(); updateControls(); });
$('clash-model-choice').addEventListener('change',() => { refreshEffortChoices($('clash-thinking-effort').value,'clash-'); updateControls(); if ($('clash-model-choice').value === CUSTOM_MODEL) $('clash-model').focus(); });
$('clash-model').addEventListener('input',() => { refreshEffortChoices($('clash-thinking-effort').value,'clash-'); updateControls(); });
$('clash-thinking-effort').addEventListener('change',updateControls);
// Exceptions stay visible; the provider's reference text sits behind the ? toggle and is still read by screen readers.
function renderModelHint(dryRun, ready, customModel) {
  const exception = dryRun ? 'Offline dry-run does not use a native provider, model, or thinking effort.' : !ready ? '' : customModel ? 'The native CLI checks a custom model ID at launch; effort choices do not guarantee support.' : supportedEfforts().length ? '' : 'No separate thinking effort for this selection.';
  const detail = dryRun || !ready ? '' : modelMetadata().detail || '';
  const toggle = document.querySelector('.model-field .explain'); toggle.hidden = !detail;
  const explained = toggle.getAttribute('aria-expanded') === 'true';
  $('model-hint').replaceChildren(...(exception ? [el('span','',exception + (detail ? ' ' : ''))] : []),el('span',explained ? '' : 'sr-only',detail));
  $('model-hint').dataset.visible = String(Boolean(exception || explained && detail));
}
function helperHint(fleet, clashMode, ultracode, provider) {
  if (clashMode) return 'Not available with Clash.';
  if (fleet) return 'Maximum reviewers running at once. Reviewers cannot start nested agents.';
  const count = $('agent-count').valueAsNumber, helpers = Number.isInteger(count) ? `${count} helper${count === 1 ? '' : 's'}` : 'the selected helpers';
  const detail = provider?.agent_control_detail || '';
  if (!$('agents-enabled').checked) return provider?.id === 'cursor' ? detail : '';
  if (provider?.id === 'cursor') return `Asks Cursor for ${helpers} each turn. ${detail}`;
  return `${ultracode ? 'Ultracode asks' : 'Asks'} for exactly ${helpers} each turn, in batches if concurrency is lower; the result confirms how many ran. ${detail}`.trim();
}
function budgetSummary(budgets) {
  const parts = [Number.isFinite(budgets?.usd) ? `$${budgets.usd}` : '',Number.isFinite(budgets?.tokens) ? `${budgets.tokens.toLocaleString()} tokens` : '',Number.isFinite(budgets?.seconds) ? `${budgets.seconds}s` : ''].filter(Boolean);
  return parts.length ? parts.join(' · ') : 'No limits';
}
// Option chips: each opens one panel below the chip row, one at a time. A chip shows its current value and flags a problem inside its panel.
const sessionOptions = {open:null};
function renderSessionOptions() {
  const hasSession = Boolean(state.selectedId), session = state.selected, fleet = fleetSelected(), clashMode = clashSelected();
  for (const id of ['project','provider','workflow']) $(id).closest('label').hidden = hasSession;
  const shown = {project:!hasSession, helpers:!clashMode && !session?.creator, clash:clashAvailable() && !fleet && !isFleetSession(session) && !session?.creator, workspace:!hasSession, brain:!hasSession, budgets:!session?.creator, models:!fleet && !clashMode && !fleetDryRun()};
  const valid = sessionOptions.validity || {}, invalid = {project:!projectFor($('project').value), helpers:valid.helpers === false, clash:valid.clash === false, workspace:valid.workspace === false, brain:valid.brain === false, budgets:valid.budgets === false || !$('session-budgets-agent-error').hidden, models:valid.models === false};
  const on = {project:Boolean(projectFor($('project').value)?.accelerator?.mode), helpers:fleet || $('agents-enabled').checked, clash:clashMode, workspace:$('workspace').value !== 'project', brain:brainLinkActive(), budgets:budgetSummary(sessionBudgets()) !== 'No limits', models:Boolean(modelRouting())};
  const count = $('agent-count').valueAsNumber, git = projectGitState.projectId === $('project').value ? projectGitState.data : null, branch = $('worktree-branch').value.trim();
  const chosenProject = projectFor($('project').value), accelerator = chosenProject?.accelerator;
  $('option-project-value').textContent = chosenProject ? `${chosenProject.name}${accelerator?.mode === 'attached' ? ` · ${accelerator.edition}` : accelerator?.mode === 'installed' ? ' · accelerator installed' : ''}` : 'choose a folder';
  $('option-helpers-label').textContent = fleet ? 'Reviewers at once' : 'Helpers';
  $('option-helpers-value').textContent = on.helpers ? (Number.isInteger(count) ? String(count) : '') : $('provider').value === 'cursor' ? 'Off · not enforced' : 'Off';
  $('option-clash-value').textContent = clashMode ? `${providerFor($('clash-challenger').value)?.name || 'challenger'} · ${$('clash-rounds').value} ${$('clash-rounds').value === '1' ? 'round' : 'rounds'}` : 'Off';
  const chosen = $('workspace').value === 'existing-worktree' ? (projectWorktreeState.data?.worktrees || []).find(item => item.id === $('existing-worktree').value) : null;
  $('option-workspace-value').textContent = $('workspace').value === 'existing-worktree' ? `Worktree · ${chosen ? worktreeLabel(chosen) : 'choose one'}` : on.workspace ? `New worktree${branch ? ` · ${branch}` : ''}` : `Project folder${git?.branch ? ` · ${git.branch}` : ''}${git?.dirty ? ' · uncommitted changes' : ''}`;
  $('option-budgets-value').textContent = budgetSummary(sessionBudgets());
  if (sessionOptions.open && !shown[sessionOptions.open]) sessionOptions.open = null;
  sessionOptions.blocked = Object.keys(shown).find(name => shown[name] && invalid[name]) || null;
  for (const name of Object.keys(shown)) {
    const chip = $('option-'+name), open = sessionOptions.open === name;
    chip.hidden = !shown[name]; chip.setAttribute('aria-expanded',String(open)); chip.dataset.on = String(on[name]); chip.dataset.invalid = String(invalid[name]);
    let alert = chip.querySelector('.chip-alert'); if (!alert) { alert = el('span','sr-only chip-alert',', needs attention'); chip.append(alert); } alert.hidden = !invalid[name];
    $('panel-'+name).hidden = !open;
  }
}
for (const chip of document.querySelectorAll('.option-bar button.chip')) chip.addEventListener('click',() => { const name = chip.id.slice('option-'.length); sessionOptions.open = sessionOptions.open === name ? null : name; renderSessionOptions(); if (sessionOptions.open === 'project') loadProjectAccelerator(); if (sessionOptions.open) { $('panel-'+name).tabIndex = -1; $('panel-'+name).focus(); } });
// An open session shows one summary line; its next-turn settings expand on request.
function renderSessionSummary() {
  const session = state.selected, toggle = $('session-settings-toggle');
  $('session-summary').hidden = !session;
  if (!session) { toggle.setAttribute('aria-expanded','false'); $('session-settings').hidden = false; renderSendBlock(false); return; }
  const expanded = toggle.getAttribute('aria-expanded') === 'true';
  $('session-settings').hidden = !expanded;
  toggle.textContent = isFleetSession(session) ? 'Settings' : 'Next-turn settings';
  // Fixed launch facts come from the session; what the next turn will use comes from the form.
  const phase = (state.bootstrap?.sdd_phases || []).find(item => item.id === $('sdd-phase').value)?.name;
  const workflow = session.workflow === 'sdd' ? `SDD${phase ? ` · ${phase}` : ''}` : (state.bootstrap?.workflows || []).find(item => item.id === session.workflow)?.name || session.workflow;
  const count = $('agent-count').valueAsNumber, budgets = budgetSummary(sessionBudgets());
  $('session-summary-text').textContent = [workflow,session.workflow === 'native' ? $('mode').value === 'edit' ? 'Edit' : 'Plan' : '',providerFor(session.provider)?.name || session.provider,
    isFleetSession(session) ? '' : selectedModel() || 'default model',!isFleetSession(session) && $('thinking-effort').value ? humanLabel($('thinking-effort').value) : '',
    `${{worktree:'Worktree','existing-worktree':'Existing worktree'}[session.workspace] || 'Project folder'}${session.branch ? ` · ${session.branch}` : ''}`,
    clashSelected() ? `Clash vs ${providerFor($('clash-challenger').value)?.name || 'challenger'}` : !isFleetSession(session) && $('agents-enabled').checked && Number.isInteger(count) ? `${count} ${count === 1 ? 'helper' : 'helpers'}` : '',
    session.brain?.task_id ? `Task ${session.brain.task_id}` : '',budgets === 'No limits' ? '' : budgets].filter(Boolean).join(' · ');
  renderSendBlock(!expanded);
}
// When Send is disabled by a setting the user cannot see, the note under the prompt says where to look.
function renderSendBlock(collapsed) {
  const hidden = sessionOptions.blocked && sessionOptions.open !== sessionOptions.blocked ? sessionOptions.blocked : null;
  const settingsError = collapsed && (Boolean(hidden) || [...$('session-settings').querySelectorAll('.model-error,.agent-error,.workspace-error,.fleet-error,.error-text')].some(node => !node.hidden));
  $('session-settings-toggle').dataset.invalid = String(Boolean(settingsError));
  if (!$('send').disabled || state.pending || !$('prompt').value.trim() || !$('composer-note').classList.contains('shortcut')) return;
  const label = hidden === 'helpers' ? $('option-helpers-label').textContent : {clash:'Clash',workspace:'Run in',brain:'Memory',budgets:'Budgets',models:'Models'}[hidden];
  if (settingsError) $('composer-note').textContent = 'Open Next-turn settings: a setting needs attention.';
  else if (label) $('composer-note').textContent = `Check ${label} to send.`;
  else return;
  $('composer-note').classList.remove('shortcut');
}
$('session-settings-toggle').addEventListener('click',() => { const toggle = $('session-settings-toggle'); toggle.setAttribute('aria-expanded',String(toggle.getAttribute('aria-expanded') !== 'true')); renderSessionSummary(); });
// "?" buttons reveal reference text next to a control; the model hint redraws through updateControls.
document.addEventListener('click',event => {
  const button = event.target.closest('button.explain'); if (!button) return;
  const expanded = button.getAttribute('aria-expanded') !== 'true'; button.setAttribute('aria-expanded',String(expanded));
  if (button.closest('.model-field')) updateControls(); else if ($(button.getAttribute('aria-controls'))) $(button.getAttribute('aria-controls')).hidden = !expanded;
});
function updateControls() {
  const hasSession = Boolean(state.selectedId); const pending = Boolean(state.pending); const ready = Boolean(state.bootstrap) && !state.authFailed;
  const fleet = fleetSelected(); const dryRun = fleetDryRun(); const existingFleet = isFleetSession(state.selected);
  if (fleet && !hasSession && !dryRun && !$('agents-enabled').disabled && fleetUi.autoHelpers !== $('workflow').value) { fleetUi.autoTicked = !$('agents-enabled').checked; $('agents-enabled').checked = true; fleetUi.autoHelpers = $('workflow').value; }
  if (!fleet && fleetUi.autoHelpers) { if (fleetUi.autoTicked && !hasSession) $('agents-enabled').checked = false; fleetUi.autoHelpers = null; fleetUi.autoTicked = false; } const awaitingContext = state.selected?.status === 'awaiting_context'; const linked = Boolean(state.selected?.brain); const linkValid = updateBrainLinkControls();
  for (const option of $('provider').options) option.disabled = !dryRun && providerFor(option.value)?.available === false;
  if (dryRun && !$('provider').value && state.bootstrap?.providers.length) { $('provider').value = state.bootstrap.providers[0].id; refreshModelChoices(null,null); }
  const workspaceValid = updateWorkspaceControls();
  const sddValid = updateSddControls(); const clashValid = updateClashControls(); const clashMode = clashSelected();
  const fleetValid = updateFleetSettingsControls(); const budgetValid = sessionBudgetControls();
  $('new-session').disabled = pending;
  for (const button of $('history').querySelectorAll('button')) button.disabled = pending;
  for (const id of ['project','provider','workflow']) $(id).disabled = !ready || hasSession || pending;
  const agentsLocked = !ready || pending || state.loading || active(state.selected) || existingFleet || awaitingContext || Boolean(state.selected?.creator);
  if (clashMode) $('agents-enabled').checked = false;
  $('agents-enabled').disabled = agentsLocked || dryRun || clashMode;
  $('agent-count').disabled = agentsLocked || !fleet && !$('agents-enabled').checked;
  if (!fleet && !$('agents-enabled').checked && !validAgentCount()) $('agent-count').value = defaultAgentCount();
  const agentsValid = validAgentCount();
  $('agent-count').setAttribute('aria-invalid',String(!agentsValid));
  showError('agent-count-error',agentsValid ? '' : `Choose a whole number from 1 to ${maxAgentCount()}.`);
  if (['plan','review','fleet-review'].includes($('workflow').value)) $('mode').value = 'plan';
  $('mode').disabled = !ready || pending || state.loading || active(state.selected) || awaitingContext || hasSession && !modelRouting() || ['plan','review','fleet-review','sdd'].includes($('workflow').value);
  $('mode').closest('label').hidden = ['plan','review','fleet-review','sdd'].includes($('workflow').value) || hasSession && !modelRouting();
  const routingValid = updateModelRoutingControls(!ready || pending || state.loading || active(state.selected) || existingFleet || dryRun || awaitingContext);
  const routed = Boolean(modelRouting());
  const provider = providerFor($('provider').value); const isActive = active(state.selected);
  const ultracode = !dryRun && $('thinking-effort').value === 'ultracode';
  const agentModeValid = fleet ? dryRun || !ultracode && $('agents-enabled').checked : !ultracode || $('agents-enabled').checked;
  for (const option of $('thinking-effort').options) option.disabled = (fleet || clashMode) && option.value === 'ultracode';
  $('thinking-effort').setAttribute('aria-invalid',String(!agentModeValid));
  showError('agent-mode-error',agentModeValid ? '' : fleet ? ultracode ? 'Ultracode is unavailable for Fleet review because reviewers cannot start nested workflows. Choose another thinking effort.' : 'Fleet review requires Use additional agents. Enable it to run the selected reviewers.' : 'Ultracode requires additional agents. Enable Use additional agents to continue.');
  $('agent-count-label').textContent = fleet ? 'Concurrent reviewers' : 'Required helpers';
  const customModel = $('model-choice').value === CUSTOM_MODEL; const model = selectedModel();
  const modelValid = dryRun || !customModel || Boolean(model && model.length <= 120 && !model.startsWith('-') && !/[\x00-\x1f\x7f]/.test(model));
  const modelLocked = !ready || isActive || pending || state.loading || existingFleet || dryRun || awaitingContext;
  $('model-choice').disabled = modelLocked || routed; $('model').disabled = modelLocked || routed || !customModel; $('model').required = customModel && !dryRun; $('custom-model-field').hidden = !customModel;
  $('thinking-effort').disabled = modelLocked || routed || supportedEfforts().length === 0;
  $('model').setAttribute('aria-invalid',String(!modelValid));
  showError('model-error',modelValid ? '' : 'Enter a model ID of 1–120 characters, without a leading dash or control characters.');
  renderModelHint(dryRun,ready,customModel);
  const terminalLinkedTask = linked && !existingFleet && ['completed','cancelled'].includes(linkedTask()?.status);
  const noResume = !awaitingContext && !existingFleet && hasSession && state.selected && !isActive && !state.selected.native_session_id && !state.loading;
  // System changes and AI scans run here as transcripts; they continue under System Orchestration.
  const systemOwned = state.selected?.system_run ? 'run' : state.selected?.system_discovery ? 'scan' : null;
  $('send').disabled = !routingValid || !sddValid || !clashValid || !budgetValid || budgetsDirty() || !ready || pending || state.loading || isActive || noResume || terminalLinkedTask || existingFleet || awaitingContext || !linkValid || brainLinkPending() || !hasSession && projectFor($('project').value)?.available === false || !agentsValid || !agentModeValid || !modelValid || !workspaceValid || !fleetValid || !$('project').value || !provider || !dryRun && !provider.available || !$('prompt').value.trim();
  $('composer-area').hidden = existingFleet || awaitingContext; $('prompt').disabled = pending || state.loading || noResume || terminalLinkedTask || existingFleet; $('prompt').placeholder = clashMode && !hasSession ? ($('mode').value === 'edit' ? 'Describe the task; the implementer builds it and the challenger attacks the result…' : 'Describe the scope; both providers review it and dispute each other’s findings…') : fleet && !hasSession ? 'Describe the review scope, files, or Git changes to inspect…' : terminalLinkedTask ? 'Start a new session with an active or new task…' : noResume ? 'Start a new session to continue…' : `${hasSession ? 'Write a follow-up for this session…' : 'Describe a task for this project…'}${commandCue()}`;
  $('attach-files').disabled = !ready || pending || state.loading || isActive || noResume || terminalLinkedTask || existingFleet || awaitingContext;
  $('attachment-input').disabled = $('attach-files').disabled;
  for (const button of $('attachment-list').querySelectorAll('button')) button.disabled = pending;
  $('prompt-label').textContent = fleet ? 'Review scope' : clashMode ? 'Task or follow-up for the next clash cycle' : 'Task or follow-up message';
  // Reviewed memory stops before each turn for a person; unattended memory needs nothing and changes no label.
  const reviewing = hasSession ? linked && brainReviewed(state.selected.brain) : brainLinkActive() && $('brain-link-review').checked;
  const remembering = hasSession ? linked && !brainReviewed(state.selected.brain) : brainLinkActive() && !$('brain-link-review').checked;
  $('send-label').textContent = state.pending === 'create' ? reviewing ? 'Preparing…' : 'Starting…' : state.pending === 'followup' ? 'Sending…' : !hasSession && ($('workspace').value === 'worktree' && projectGitState.pending || $('workspace').value === 'existing-worktree' && projectWorktreeState.pending && !projectWorktreeState.data) ? 'Checking Git…' : isActive ? 'Session active' : noResume || terminalLinkedTask ? 'New session needed' : hasSession ? reviewing ? 'Prepare follow-up' : clashMode ? 'Start next cycle' : 'Send follow-up' : reviewing ? 'Prepare session' : fleet ? dryRun ? 'Start dry-run' : 'Start fleet review' : clashMode ? 'Start clash' : 'Start session';
  $('composer-note').textContent = budgetsDirty() ? 'Save budget changes before launching.' : terminalLinkedTask ? 'Choose an active or new Brain task in a new session.' : noResume ? systemOwned ? 'Continue in System Orchestration.' : 'Use New session in the sidebar.' : isActive ? 'Wait for completion, or cancel the session.' : 'Ctrl / ⌘ + Enter to send';
  $('composer-note').classList.toggle('shortcut',$('composer-note').textContent === 'Ctrl / ⌘ + Enter to send');
  const caption = terminalLinkedTask ? 'The linked Brain task is completed or cancelled. Start a new session with an active or new task to continue.' : reviewing && !noResume ? 'You review the prepared workspace and context before the agent runs.' : remembering && !noResume && !clashMode && !fleet ? 'Project memory is retrieved for each message and saved when the run completes.' : clashMode && !hasSession ? 'Both participants share the session budgets.' : clashMode && !noResume ? 'A follow-up starts the next cycle; both native sessions resume. Untick Clash for a normal follow-up.' : fleet && !hasSession ? 'Scope and reviewers are fixed once the review starts.' : noResume ? systemOwned === 'run' ? 'This launch belongs to a system change. Resume, cancel or review it under System Orchestration › Changes.' : systemOwned === 'scan' ? 'This is an AI scan of service folders. Start a new scan from the system editor.' : state.selected.status === 'cancelled' ? 'Cancelled before a resumable native session was created.' : 'No resumable native session was returned. Start a new session to continue.' : '';
  $('composer-caption').textContent = caption; $('composer-caption').hidden = !caption;
  const signInNote = !dryRun && provider?.available && signedOut(provider.id) ? `${provider.name} is not signed in. Sign in from a terminal with ${signIn.states[provider.id].login || 'its CLI'}${hasSession ? ', then send again' : ''}; this page checks again when you come back to it.` : '';
  const providerNote = dryRun ? 'Offline dry-run can use any provider selection; no native CLI is launched.' : ready && !provider?.available ? 'No provider is ready. Install and sign in with a native CLI, then restart the Harness server.' : signInNote || (provider?.id === 'cursor' ? 'Cursor can’t disable or cap helpers; helper settings are instructions only.' : '');
  const switchTo = signInNote && !hasSession ? (state.bootstrap?.providers || []).find(item => item.available && signIn.states[item.id]?.state === 'signed_in') : null;
  const hintKey = `${providerNote}|${switchTo?.id || ''}`;
  if ($('provider-hint').dataset.key !== hintKey) {
    $('provider-hint').dataset.key = hintKey;
    // The sign-in command as code, ready to copy into a terminal.
    const login = signInNote ? signIn.states[provider.id].login : '', at = login ? providerNote.indexOf(login) : -1;
    $('provider-hint').replaceChildren(...(at < 0 ? [providerNote] : [providerNote.slice(0,at),el('code','',login),providerNote.slice(at + login.length)]));
    if (switchTo) { const use = el('button','button compact',`Use ${switchTo.name}`); use.type = 'button'; use.addEventListener('click',() => { $('provider').value = switchTo.id; $('provider').dispatchEvent(new Event('change')); }); $('provider-hint').append(' ',use); }
  }
  $('provider-hint').hidden = !providerNote;
  $('agent-hint').textContent = helperHint(fleet,clashMode,ultracode,provider);
  // A Harness check sets the session running without a launch; the Run strip follows the newest launch instead.
  const waitingText = runView.phase(state.selected) === 'check' ? 'A Harness check is running. Its result appears in Checks.' : linked && brainReviewed(state.selected.brain) && !state.selected.brain.context_id ? 'Preparing workspace and context. The provider has not started this turn.' : state.selected?.status === 'queued' ? 'Queued — waiting for an available runner.' : existingFleet ? 'Fleet review is running. Stage and reviewer updates appear above.' : isClashSession(state.selected) ? `${providerFor(state.selected.provider)?.name || state.selected.provider} and ${providerFor(state.selected.clash?.challenger)?.name || 'the challenger'} are clashing. Turn events appear above.` : `${providerFor(state.selected?.provider)?.name || 'Agent'} is running. New events will appear here.`;
  runView.sync(waitingText);
  $('welcome').hidden = hasSession; sessionOptions.validity = {helpers:agentsValid && agentModeValid,clash:clashValid,workspace:workspaceValid,brain:linkValid,budgets:budgetValid,models:routingValid}; renderSessionOptions(); renderSessionSummary(); updateHeader(); updateSkillsControls(); updateKnowledgeControls(); updateSetupControls(); renderFleetSession(); renderClashSession(); renderLinkedSession(); if (typeof renderContextMeter === 'function') renderContextMeter(); if (typeof composerCommands === 'object') composerCommands.sync(); resultControls();
}
function applySessionSettings(session, restoreModel = true) { $('sdd-feature').value = session.sdd?.feature || ''; $('sdd-phase').value = session.sdd?.phase || 'specify'; for (const id of ['project','provider','workflow','mode']) $(id).value = session[id === 'project' ? 'project_id' : id] || (id === 'mode' ? 'plan' : id === 'workflow' ? 'native' : ''); if (restoreModel || active(session) || isFleetSession(session)) { $('agents-enabled').checked = session.agents_enabled === true; $('agent-count').value = Number.isInteger(session.agent_count) ? session.agent_count : defaultAgentCount(); } $('workspace').value = ['worktree','existing-worktree'].includes(session.workspace) ? session.workspace : 'project'; $('worktree-branch').value = session.workspace === 'worktree' ? session.branch || '' : ''; restoreBudgets('session-budgets',session.budgets,session.id+':'+session.budget_revision); restoreModelRouting(session.model_routing); restoreClashSettings(session.clash || null); if (isFleetSession(session)) restoreFleetSettings(session.fleet); if (restoreModel || isFleetSession(session)) refreshModelChoices(session.model || null,session.thinking_effort || null); loadProjectGit(); }
for (const id of ['workflow','mode','thinking-effort']) $(id).addEventListener('change',updateControls);
$('provider').addEventListener('change',() => { refreshModelChoices(null,null); restoreModelRouting(null); renderClashChallengers(); if (fleetSelected() && !fleetBudgetSupported() && !state.selectedId) $('fleet-budget').value = ''; updateControls(); });
$('fleet-dry-run').addEventListener('change',() => { if (!fleetBudgetSupported()) $('fleet-budget').value = ''; updateControls(); });
$('fleet-budget').addEventListener('input',updateControls); $('fleet-worker-timeout').addEventListener('input',updateControls);
$('model-choice').addEventListener('change',() => { refreshEffortChoices(); updateControls(); if ($('model-choice').value === CUSTOM_MODEL) $('model').focus(); });
$('model').addEventListener('input',() => { refreshEffortChoices(); updateControls(); });
$('project').addEventListener('change',() => { clearAttachments(); saveSessionPreferences(); $('context-project').value = $('project').value; if (!knowledgeState.pending) { $('memory-project').value = $('project').value; $('brain-project').value = $('project').value; } $('accelerator-project').value = $('project').value; restoreProjectPreferences($('project').value); if (sessionOptions.open === 'project') loadProjectAccelerator(); renderSessionOptions(); });
$('workspace').addEventListener('change',() => { if ($('workspace').value !== 'project') loadProjectGit(true); updateControls(); });
$('existing-worktree').addEventListener('change',() => { projectWorktreeState.preferred = $('existing-worktree').value; updateControls(); });
$('worktree-branch').addEventListener('input',updateControls);
$('refresh-project-git').addEventListener('click',() => loadProjectGit(true));
$('prompt').addEventListener('input',updateControls);
$('agents-enabled').addEventListener('change',updateControls);
$('agent-count').addEventListener('input',updateControls);
$('prompt').addEventListener('keydown',event => { if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) { event.preventDefault(); if (!$('send').disabled) $('session-form').requestSubmit(); } });
function stopPolling() { clearTimeout(state.pollTimer); state.pollTimer = null; state.pollController?.abort(); state.pollController = null; }
function newSession(prompt = '', projectId = $('project').value, show = true) {
  saveSessionPreferences();
  clearAttachments();
  stopPolling(); state.epoch++; state.selectedId = null; state.selected = null; state.loading = false; state.eventIds.clear(); state.assistantTexts.clear(); state.stepCalls.clear(); runView.reset(); $('events').replaceChildren(); $('prompt').value = prompt;
  sessionOptions.open = null; $('session-settings-toggle').setAttribute('aria-expanded','false'); sddSlugTouched = false; $('sessions-view').classList.remove('output-resized'); $('sessions-view').style.removeProperty('--configuration-height');
  resetBrainLink(); restoreModelRouting(null); $('sdd-feature').value = ''; $('sdd-phase').value = 'specify'; restoreClashSettings(null);
  $('agents-enabled').checked = false; $('agent-count').value = defaultAgentCount();
  restoreFleetSettings(null); restoreBudgets('session-budgets',null,'new:'+state.epoch); resetFleetProgress();
  $('workspace').value = 'project'; $('worktree-branch').value = ''; loadProjectGit(true);
  refreshModelChoices(null,null);
  showError('events-error',''); showError('composer-error',''); renderHistory(); if (show) setView('sessions'); updateControls(); if (show) $('prompt').focus();
  $('project').value = projectId; restoreProjectPreferences(projectId);
}
$('new-session').addEventListener('click',() => { if (!state.pending) newSession(); });
function fleetCost(value) { return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? `$${value.toFixed(4).replace(/0+$/,'').replace(/\.$/,'.00')}` : null; }
function observeFleetEvent(event) {
  if (event.kind === 'fleet_stage' && typeof event.stage === 'string') fleetUi.stage = {stage:event.stage,status:typeof event.status === 'string' ? event.status : ''};
  if (event.kind === 'fleet_reviewer' && typeof event.lens === 'string') fleetUi.reviewers.set(event.lens,{status:typeof event.status === 'string' ? event.status : '',cost_usd:event.cost_usd,duration_seconds:event.duration_seconds,limit_reached:event.limit_reached === 'USD' ? 'USD' : null});
}
function renderFleetSession() {
  const session = state.selected; const fleet = isFleetSession(session); $('fleet-progress').hidden = !fleet; $('fleet-outcome').hidden = !fleet;
  if (!fleet) return;
  const result = session.fleet_result; const dryRun = session.fleet?.dry_run === true || result?.dry_run === true; const ready = Boolean(state.bootstrap) && !state.authFailed && !state.pending && !state.loading;
  const stage = typeof result?.stage === 'string' ? result.stage : fleetUi.stage?.stage;
  const lenses = Array.isArray(session.fleet?.lenses) ? session.fleet.lenses : [...fleetUi.reviewers.keys()];
  $('fleet-stage').textContent = `${sessionStatusLabel(session)}${stage ? ` · ${fleetStageName(stage)}` : ''}`;
  $('fleet-run-detail').textContent = `${dryRun ? 'Offline dry-run · simulated review' : providerFor(session.provider)?.name || session.provider} · ${lenses.length} ${lenses.length === 1 ? 'reviewer' : 'reviewers'} · up to ${session.agent_count || 1} concurrently`;
  $('fleet-brain-note').hidden = typeof result?.brain_available !== 'boolean'; $('fleet-brain-note').textContent = result?.brain_available ? `Project Brain journal is active${typeof result.task_id === 'string' ? ` · Task: ${result.task_id}` : ''}.` : dryRun ? 'Offline dry-run does not write to the Project Brain journal.' : 'No Project Brain journal is available in this project. The review uses its local checkpoint.';
  $('fleet-reviewers').replaceChildren();
  for (const lens of lenses) { const reviewer = fleetUi.reviewers.get(lens); const card = el('div','fleet-reviewer'); const details = [reviewer?.status ? humanLabel(reviewer.status) : state.loading ? 'Loading history…' : 'No status recorded yet']; if (reviewer?.limit_reached === 'USD') details.push('USD budget reached; review incomplete'); const cost = fleetCost(reviewer?.cost_usd); if (cost) details.push(`${dryRun ? 'Simulated ' : ''}${cost}`); if (typeof reviewer?.duration_seconds === 'number' && Number.isFinite(reviewer.duration_seconds) && reviewer.duration_seconds >= 0) details.push(`${reviewer.duration_seconds.toFixed(1)}s`); card.append(el('div','fleet-reviewer-name',fleetLensName(lens)),el('div','fleet-reviewer-status',details.join(' · '))); $('fleet-reviewers').append(card); }
  const resumable = ['interrupted','failed','cancelled'].includes(session.status);
  $('fleet-resume-actions').hidden = !resumable; $('fleet-resume').disabled = !ready || fleetMetadata().available !== true || budgetsDirty(); $('fleet-resume').textContent = state.pending === 'fleet-resume' ? 'Resuming…' : 'Resume review';
  $('fleet-outcome').hidden = !result && !['awaiting_approval','completed','rejected'].includes(session.status);
  const findings = Array.isArray(result?.findings) ? result.findings.filter(finding => finding && typeof finding === 'object' && !Array.isArray(finding)) : []; const savedReport = typeof result?.report === 'string' ? result.report : ''; const report = savedReport || (typeof result?.preview === 'string' ? result.preview : ''); const reportReady = Boolean(report.trim());
  const cost = fleetCost(result?.cost_usd); const skipped = Array.isArray(result?.skipped) ? result.skipped : [];
  $('fleet-outcome-title').textContent = session.status === 'rejected' ? 'Rejected report' : session.status === 'completed' ? 'Approved report' : 'Review findings';
  $('fleet-result-summary').textContent = `${findings.length} ${findings.length === 1 ? 'finding' : 'findings'}${active(session) ? ' recorded so far' : ''} · ${cost ? `${dryRun ? 'simulated cost ' : 'reported cost '}${cost}` : 'cost unavailable'}${dryRun ? ' · dry-run output' : ''}`;
  const skippedNames = skipped.map(item => typeof item === 'string' ? fleetLensName(item) : typeof item?.lens === 'string' ? `${fleetLensName(item.lens)}${typeof item.reason === 'string' ? ` (${item.reason})` : ''}` : '').filter(Boolean);
  $('fleet-skipped').textContent = skippedNames.length ? `Skipped reviewers: ${skippedNames.join(', ')}` : ''; $('fleet-skipped').hidden = !skippedNames.length;
  const resultKey = JSON.stringify({findings,report});
  if (resultKey !== fleetUi.resultKey) {
    fleetUi.resultKey = resultKey; $('fleet-findings').replaceChildren();
    for (const finding of findings) {
      const block = el('article','fleet-finding'); const heading = el('div','fleet-finding-heading'); const severity = typeof finding.severity === 'string' ? finding.severity : 'unspecified';
      heading.append(el('span',`fleet-severity ${['critical','high','medium','low'].includes(severity) ? severity : ''}`,humanLabel(severity)));
      if (typeof finding.file === 'string' && finding.file) heading.append(el('span','fleet-finding-location',`${finding.file}${Number.isInteger(finding.line) && finding.line > 0 ? `:${finding.line}` : ''}`));
      if (typeof finding.lens === 'string') heading.append(el('span','',fleetLensName(finding.lens)));
      block.append(heading,el('p','fleet-finding-claim',typeof finding.claim === 'string' ? finding.claim : 'No finding text was returned.'));
      if (typeof finding.evidence === 'string' && finding.evidence) { const evidence = el('details','fleet-evidence'); evidence.append(el('summary','','Evidence'),el('pre','',finding.evidence)); block.append(evidence); }
      $('fleet-findings').append(block);
    }
    $('fleet-report-text').textContent = report;
  }
  $('fleet-report').hidden = !reportReady;
  if (session.status === 'awaiting_approval' && reportReady && fleetUi.reportOpenedFor !== session.id) { $('fleet-report').open = true; fleetUi.reportOpenedFor = session.id; }
  $('fleet-decision-actions').hidden = session.status !== 'awaiting_approval';
  $('fleet-approve').disabled = !ready || !reportReady || fleetMetadata().available !== true; $('fleet-reject').disabled = !ready || !reportReady || fleetMetadata().available !== true;
  $('fleet-approve').textContent = state.pending === 'fleet-approve' ? 'Approving…' : 'Approve report'; $('fleet-reject').textContent = state.pending === 'fleet-reject' ? 'Rejecting…' : 'Reject report';
  $('fleet-decision-hint').textContent = session.status === 'awaiting_approval' ? reportReady ? 'Review the findings and report. Approve saves the report; Reject finishes this review without saving it.' : 'Waiting for the report preview before a decision can be made.' : session.status === 'completed' ? 'This report was approved and saved.' : session.status === 'rejected' ? 'This report was rejected and was not saved.' : reportReady ? 'The report is a preview until it is approved.' : 'Review results will appear as the workflow progresses.';
  $('fleet-download').hidden = session.status !== 'completed' || !savedReport.trim();
  if (session.status === 'completed' && savedReport.trim()) $('fleet-download').href = `/api/sessions/${encodeURIComponent(session.id)}/report`; else $('fleet-download').removeAttribute('href');
}
async function fleetAction(action) {
  const session = state.selected; if (!isFleetSession(session) || state.pending || state.loading || !state.bootstrap || state.authFailed) return;
  const button = $(`fleet-${action}`); if (button.disabled || (action !== 'resume' && session.status !== 'awaiting_approval') || (action === 'resume' && !['interrupted','failed','cancelled'].includes(session.status))) return;
  const sid = session.id; const epoch = state.epoch; const pending = `fleet-${action}`; stopPolling(); state.pending = pending; showError('fleet-action-error',''); updateControls();
  try {
    const data = await api(`/api/sessions/${encodeURIComponent(sid)}/${action === 'resume' ? 'resume' : 'decision'}`,{method:'POST',body:action === 'resume' ? {} : {approve:action === 'approve'}});
    if (epoch !== state.epoch || sid !== state.selectedId) return;
    if (data.session) { if (data.session.id !== sid) throw new Error('The runner returned a different session. Refresh this review before retrying.'); upsert(data.session); applySessionSettings(data.session); }
    await pollSession(epoch);
  } catch (error) { if (epoch === state.epoch && sid === state.selectedId) { showError('fleet-action-error',error.status === 0 ? 'The connection was lost. Refresh this session before retrying; the requested action may have completed.' : textError(error)); if (error.status === 0) await pollSession(epoch); } }
  finally { if (state.pending === pending) { state.pending = null; updateControls(); } }
}
$('fleet-approve').addEventListener('click',() => fleetAction('approve')); $('fleet-reject').addEventListener('click',() => fleetAction('reject')); $('fleet-resume').addEventListener('click',() => fleetAction('resume'));
const MEMORY_DRAFT_BLOCK = /```memory-draft[^\S\n]*\n[\s\S]*?\n[^\S\n]*```/g;
// The reply points at where the draft went instead of repeating its JSON: the project memory line the run adds
// once it has saved the draft itself.
function withoutMemoryDraft(text) { return text.replace(MEMORY_DRAFT_BLOCK,'[Memory draft: saved to project memory when the run completes.]').trim(); }
// Consecutive steps share one collapsed row. Its summary counts are kept as steps arrive, never recounted from the rows.
function stepGroup() {
  let group = $('events').lastElementChild;
  if (!group?.classList.contains('activity-group')) {
    group = el('details','activity-group'); group.setAttribute('aria-live','off'); group.append(el('summary'),el('div','activity-steps'));
    group.steps = {steps:0,tools:new Map(),patterns:[],opened:new Set(),edited:new Set()}; $('events').append(group);
  }
  return group;
}
function countStep(group, name) { group.steps.steps++; group.steps.tools.set(name,(group.steps.tools.get(name) || 0) + 1); group.firstElementChild.textContent = RunModel.groupSummary(group.steps); }
const stepWord = event => event.outcome === 'not_run' ? ['notrun','not run'] : Number.isInteger(event.exit_code) ? [event.ok === false ? 'failed' : 'done',`exit ${event.exit_code}`] : event.ok === false ? ['failed','failed'] : ['done','done'];
// A step row names the tool, its target and its state. A completion that carries its call updates the start row in place;
// a label-only completion stays a row of its own and is not counted as a step.
// Calls pair within their launch: Codex numbers its items item_0, item_1… again in every resumed turn.
const stepKey = (event, call) => `${typeof event.launch_id === 'string' ? event.launch_id : ''}|${call}`;
function appendStep(event) {
  const step = RunModel.describe(event), known = step.call ? state.stepCalls.get(stepKey(event,step.call)) : null;
  // Codex repeats an open helper operation's label while it runs; the row of its start already stands for it.
  if (step.progress && [...state.stepCalls.values()].some(entry => entry.launch === event.launch_id && entry.op === step.progress && entry.row.dataset.state === 'running')) return;
  if (!step.start && known) {
    const [mark,word] = stepWord(event); known.row.dataset.state = mark; known.row.lastChild.textContent = word;
    if (event.ok !== false && event.outcome !== 'not_run' && known.edit) { for (const path of known.paths) known.group.steps.edited.add(path); known.group.firstElementChild.textContent = RunModel.groupSummary(known.group.steps); }
    return;
  }
  const group = stepGroup(), row = el('div','step-row'), counted = step.start || Boolean(step.call && typeof event.tool === 'string'), [mark,word] = step.start ? [step.call ? 'running' : 'started',step.call ? 'running' : 'started'] : stepWord(event);
  const by = typeof event.provider === 'string' && event.provider ? `${providerFor(event.provider)?.name || event.provider} · ` : '';
  row.dataset.eventId = event.id; row.dataset.state = mark; if (step.call) row.dataset.call = step.call;
  row.append(el('span','step-name',`${by}${step.helper ? 'Helper · ' : ''}${step.tool}`));
  if (step.target) { const target = el('bdi','step-target',step.target); target.dir = 'ltr'; row.append(target); }
  row.append(el('span','step-state',word)); group.lastElementChild.append(row);
  if (step.call && step.start) state.stepCalls.set(stepKey(event,step.call),{row,group,launch:event.launch_id,edit:step.edit,paths:step.paths,op:step.op});
  if (!counted) return;
  if (step.pattern) group.steps.patterns.push(step.pattern);
  if (step.read) for (const path of step.paths) group.steps.opened.add(path);
  if (!step.start && step.edit && event.ok !== false && event.outcome !== 'not_run') for (const path of step.paths) group.steps.edited.add(path);
  countStep(group,step.tool);
}
// The end of a launch: calls it never answered show that instead of "running".
function closeSteps(launch) { if (typeof launch === 'string') for (const entry of state.stepCalls.values()) if (entry.launch === launch && entry.row.dataset.state === 'running') { entry.row.dataset.state = 'unfinished'; entry.row.lastChild.textContent = 'no result'; } }
// A launch that raised ends on the worker's error line instead of a closing status; its receipt follows that line.
function closeAborted(event) { if (!RunModel.aborted(event)) return; closeSteps(event.launch_id); const node = runView.statusNode(event); if (node) $('events').append(node); }
function appendEvent(event) {
  if (event.id === undefined || event.id === null || state.eventIds.has(String(event.id))) return;
  state.eventIds.add(String(event.id)); runView.observe(event); let text = typeof event.text === 'string' ? event.text : '';
  if (event.kind === 'error' && typeof event.sign_in === 'string') loadSignIn(true);  // a run was refused for its sign-in
  const kind = event.kind === 'result' && event.ok === false ? 'error' : event.kind;
  if (kind === 'fleet_stage') {
    observeFleetEvent(event); text = [fleetStageName(event.stage),humanLabel(event.status)].filter(Boolean).join(' · ');
  } else if (kind === 'fleet_reviewer') {
    observeFleetEvent(event); const cost = fleetCost(event.cost_usd); text = [fleetLensName(event.lens),humanLabel(event.status),cost ? `${state.selected?.fleet?.dry_run ? 'Simulated' : 'Reported'} cost: ${cost}` : '',typeof event.duration_seconds === 'number' && Number.isFinite(event.duration_seconds) && event.duration_seconds >= 0 ? `Duration: ${event.duration_seconds.toFixed(1)}s` : ''].filter(Boolean).join('\n');
  } else if (kind === 'usage') {
    const fields = {input_tokens:'Input tokens',output_tokens:'Output tokens',cached_input_tokens:'Cached input tokens',cache_read_input_tokens:'Cache read tokens',cache_creation_input_tokens:'Cache creation tokens',cost_usd:'Cost (USD)'};
    const lines = Object.entries(fields).filter(([key]) => typeof event[key] === 'number' && Number.isFinite(event[key]) && event[key] >= 0).map(([key,label]) => `${label}: ${event[key]}`);
    text = lines.join('\n');
  } else if (kind === 'session' && typeof event.native_session_id === 'string') {
    text = `Native session: ${event.native_session_id}`;
  }
  if (!text.trim()) return;
  if (kind === 'memory') {
    const block = el('p','memory-event'); block.dataset.ok = String(event.ok !== false); block.dataset.eventId = event.id; block.setAttribute('role','status'); block.textContent = text; $('events').append(block);
  } else if (kind === 'delegation') {
    const block = el('div','memory-notice'); block.dataset.eventId = event.id; block.setAttribute('role','status'); block.append(el('strong','',Number.isInteger(event.required_count) ? event.status === 'confirmed' ? 'Required helper count confirmed. ' : 'Required helper count not confirmed. ' : event.status === 'confirmed' ? 'Helper launch confirmed. ' : 'Helper launch not confirmed. '),document.createTextNode(text)); $('events').append(block);
  } else if (kind === 'clash_turn') {
    const block = el('div','clash-turn'); block.dataset.eventId = event.id; block.setAttribute('role','status'); block.textContent = text; $('events').append(block);
  } else if (kind === 'agent') {
    // System Orchestration shows these per agent; the runner transcript keeps a compact trail.
    const block = el('div','clash-turn'); block.dataset.eventId = event.id; block.setAttribute('role','status'); block.textContent = [text,typeof event.summary === 'string' ? event.summary : ''].filter(Boolean).join('\n'); $('events').append(block);
  } else if (['user','text','assistant','error','result'].includes(kind)) {
    const normalized = text.trim();
    if (kind === 'result' && state.assistantTexts.has(normalized)) return;
    if (['text','assistant','result'].includes(kind)) state.assistantTexts.add(normalized);
    const previous = $('events').lastElementChild;
    if (kind === 'error' && previous?.classList.contains('error') && previous.classList.contains('message') && previous.dataset.provider === String(event.provider || '')) { previous.setAttribute('aria-atomic','false'); previous.append(el('pre','message-body',text)); closeAborted(event); return; }
    const type = kind === 'text' ? 'assistant' : kind; const block = el('article',`message ${type}`); block.dataset.provider = String(event.provider || ''); block.dataset.eventId = event.id; const heading = el('div','message-label');
    const tagged = typeof event.provider === 'string' && event.provider ? `${providerFor(event.provider)?.name || event.provider}${typeof event.role === 'string' && event.role ? ` · ${humanLabel(event.role)}` : ''}${Number.isInteger(event.round) && event.round > 0 ? ` · round ${event.round}` : ''}` : '';
    const label = kind === 'user' ? 'You' : kind === 'error' ? `Session error${tagged ? ` · ${tagged}` : ''}` : kind === 'result' ? 'Result' : tagged || providerFor(state.selected?.provider)?.name || 'Assistant';
    heading.append(el('span','avatar',kind === 'user' ? 'Y' : kind === 'error' ? '!' : 'AI'),document.createTextNode(label)); block.append(heading,el('pre','message-body',['text','assistant','result'].includes(kind) ? withoutMemoryDraft(text) : text));
    if (kind === 'user' && Array.isArray(event.attachments)) {
      const files = el('div','attachment-list');
      for (const file of event.attachments) {
        if (!/^[a-f0-9]{32}$/.test(file.id) || typeof file.name !== 'string') continue;
        const chip = el('div','attachment-chip'), link = el('a','',`${file.name} · ${bytesLabel(file.size)}`);
        link.href = `/api/sessions/${encodeURIComponent(state.selectedId)}/attachments/${file.id}`; link.download = file.name; chip.append(link); files.append(chip);
      }
      block.append(files);
    }
    if (kind === 'error') block.setAttribute('role','alert'); $('events').append(block); if (kind === 'error') closeAborted(event);
  } else if (kind === 'tool') {
    appendStep(event);
  } else if (kind === 'status') {
    if (event.outcome || /^Run \w+\. Process completion is not/.test(text)) closeSteps(event.launch_id);
    let block = runView.statusNode(event);
    if (!block) { block = el('details','activity'); block.append(el('summary','','Runner status'),el('pre','',text)); }
    block.dataset.eventId = event.id; $('events').append(block);
  } else if (['usage','session','fleet_stage','fleet_reviewer','agent_activity'].includes(kind)) {
    const block = el('details','activity'); const label = kind === 'fleet_reviewer' ? `Reviewer · ${fleetLensName(event.lens)}` : kind === 'agent_activity' ? `Agent activity${typeof event.agent === 'string' ? ` · ${event.agent}` : ''}${typeof event.type === 'string' ? ` · ${humanLabel(event.type)}` : ''}` : ({usage:'Usage',session:'Native session',fleet_stage:'Fleet stage'})[kind]; block.append(el('summary','',label),el('pre','',text));
    block.dataset.eventId = event.id; const group = stepGroup(); group.lastElementChild.append(block);
    countStep(group,kind === 'fleet_reviewer' ? 'Reviewer' : kind === 'agent_activity' ? typeof event.agent === 'string' && event.agent ? event.agent : 'Agent' : ({usage:'Usage',session:'Session',fleet_stage:'Stage'})[kind]);
  }
}
async function pollSession(epoch) {
  if (!state.selectedId || epoch !== state.epoch) return;
  clearTimeout(state.pollTimer); const controller = new AbortController(); state.pollController = controller;
  try {
    const cursor = [...state.eventIds].reduce((last,id) => Number.isFinite(Number(id)) ? Math.max(last,Number(id)) : last,0);
    const data = await api(`/api/sessions/${encodeURIComponent(state.selectedId)}?after=${cursor}`,{signal:controller.signal});
    if (epoch !== state.epoch) return;
    const conversation = $('conversation'); const atBottom = conversation.scrollHeight - conversation.scrollTop - conversation.clientHeight < 130;
    if (data.session) {
      const restore = state.loading || active(state.selected) || active(data.session), ended = active(state.selected) && !active(data.session);
      upsert(data.session); if (restore) applySessionSettings(data.session);
      if (ended) memoryUseSessionEnded(data.session.project_id);
    }
    // One page is folded into the run model event by event, then drawn once; history pages are drawn without motion.
    const events = data.events || []; runView.begin(events.length);
    for (const event of events) appendEvent(event);
    runView.end(events.length); runView.connection(true); showError('events-error',''); state.loading = false; updateControls();
    if (atBottom) conversation.scrollTop = conversation.scrollHeight;
    // A completed run may still have more than one page of recorded events.
    if ((data.events || []).length === 250) state.pollTimer = setTimeout(() => pollSession(epoch),0);
    else if (active(state.selected)) state.pollTimer = setTimeout(() => pollSession(epoch),1000);
  } catch (error) {
    if (error.name === 'AbortError' || epoch !== state.epoch) return;
    state.loading = false; showError('events-error',textError(error)); if (error.status === 0) runView.connection(false); updateControls();
    if (active(state.selected) && ![401,403,404].includes(error.status)) state.pollTimer = setTimeout(() => pollSession(epoch),2500);
  } finally { if (state.pollController === controller) state.pollController = null; }
}
async function selectSession(id) {
  if (state.pending) return;
  if (id !== state.selectedId) { sessionOptions.open = null; $('session-settings-toggle').setAttribute('aria-expanded','false'); $('sessions-view').classList.remove('output-resized'); $('sessions-view').style.removeProperty('--configuration-height'); }
  clearAttachments();
  saveSessionPreferences(); ++preferenceEpoch; restoringSessionPreferences = false;
  // The list holds summaries; a session's settings come from its full record, which the first poll brings.
  const listed = state.sessions.find(item => item.id === id);
  stopPolling(); const epoch = ++state.epoch; state.selectedId = id; state.selected = listed && !listed.summary ? listed : null; state.eventIds.clear(); state.assistantTexts.clear(); state.stepCalls.clear(); state.loading = true;
  runView.reset(id);
  resetFleetProgress();
  $('events').replaceChildren(); $('prompt').value = ''; showError('composer-error',''); showError('events-error','');
  if (state.selected) applySessionSettings(state.selected);
  renderHistory(); setView('sessions'); updateControls(); await pollSession(epoch); saveSessionPreferences();
}
$('session-form').addEventListener('submit',async event => {
  event.preventDefault(); if ($('send').disabled || state.pending || isFleetSession(state.selected)) return;
  // /clear, /new, /model and the like act on the page; they never reach the CLI.
  if (typeof composerCommands === 'object' && composerCommands.intercept($('prompt').value)) return;
  const prompt = $('prompt').value.trim(); const followup = Boolean(state.selectedId); state.pending = followup ? 'followup' : 'create'; showError('composer-error',''); updateControls();
  const turn = {prompt,agents_enabled:$('agents-enabled').checked,agent_count:$('agent-count').valueAsNumber,model_routing:modelRouting(),...(followup && $('workflow').value === 'native' && modelRouting() ? {mode:$('mode').value} : {}),...($('workflow').value === 'sdd' ? {sdd:sddSettings()} : {}),...(clashSelected() ? {clash:clashSettings()} : followup ? {clash:null} : {}),model:fleetDryRun() ? null : selectedModel(),thinking_effort:fleetDryRun() ? null : $('thinking-effort').value || null};
  const body = followup ? turn : {...turn,budgets:sessionBudgets(),project_id:$('project').value,provider:$('provider').value,mode:$('mode').value,workflow:$('workflow').value,project_context:true,workspace:$('workspace').value,...($('workspace').value === 'worktree' && $('worktree-branch').value.trim() ? {worktree_branch:$('worktree-branch').value.trim()} : {}),...($('workspace').value === 'existing-worktree' ? {worktree_id:$('existing-worktree').value} : {}),...(fleetSelected() ? {fleet:fleetSettings()} : {}),...(brainLinkActive() ? {brain:brainLinkConfig()} : {})};
  try {
    if (attachedFiles.length) body.attachments = await Promise.all(attachedFiles.map(encodeAttachment));
    const data = await api(followup ? `/api/sessions/${encodeURIComponent(state.selectedId)}/messages` : '/api/sessions',{method:'POST',body});
    if (!followup && !data.session?.id) throw new Error('The runner did not return a session. Refresh the session list before retrying.');
    if (data.session) { upsert(data.session); if (followup) applySessionSettings(data.session); } $('prompt').value = ''; clearAttachments();
    if (!followup) { state.pending = null; await selectSession(data.session.id); }
    else { if (data.session) state.selected = data.session; await pollSession(state.epoch); }
  } catch (error) { showError('composer-error',textError(error)); }
  finally { state.pending = null; updateControls(); }
});
$('cancel-session').addEventListener('click',async () => {
  if (!active(state.selected) || state.pending) return; state.pending = 'cancel'; updateControls(); const errorId = isFleetSession(state.selected) ? 'fleet-action-error' : 'composer-error'; showError(errorId,'');
  try { const data = await api(`/api/sessions/${encodeURIComponent(state.selectedId)}/cancel`,{method:'POST',body:{}}); if (data.session) upsert(data.session); stopPolling(); await pollSession(state.epoch); }
  catch (error) { showError(errorId,textError(error)); }
  finally { state.pending = null; updateControls(); }
});
