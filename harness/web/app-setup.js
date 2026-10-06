// Projects & Setup: project registration, the folder picker, readiness and edition installation.
// Classic script loaded in order by index.html; top-level names are shared with the other app-*.js files.
'use strict';
function projectChoiceLabel(item,projects) { const project = projects.find(candidate => candidate.id === item.id); return `${item.name}${project?.available === false ? ' · unavailable' : ''}${project?.accelerator?.mode === 'attached' ? ` · ${project.accelerator.edition} attached` : ''}`; }
function setProjectChoices(select,projects,previous,allowUnavailable = false) {
  setOptions(select,projects.map(project => ({...project,...(allowUnavailable ? {available:true} : {})})),item => projectChoiceLabel(item,projects),item => item.id,previous);
  if (previous && projects.some(project => project.id === previous)) select.value = previous;
}
const folderPicker = {path:null,parent:null,valid:false,controller:null,epoch:0,onSelect:null,opener:null};
const folderPickerNote = $('folder-picker-description').textContent;
function chooseProjectFolder(folder,onSelect,opener,title='Choose a project folder',note=folderPickerNote) {
  folderPicker.onSelect=onSelect; folderPicker.opener=opener; $('folder-picker-title').textContent=title; $('folder-picker-description').textContent=note;
  $('folder-picker').showModal(); loadFolderPicker(folder || folderPicker.path || undefined);
}
async function loadFolderPicker(path,query = '') {
  if (!state.bootstrap || state.authFailed) return;
  folderPicker.controller?.abort(); const controller = new AbortController(); folderPicker.controller = controller; const epoch = ++folderPicker.epoch;
  folderPicker.valid = false; $('folder-picker-use').disabled = true; $('folder-picker-parent').disabled = true; $('folder-picker-search').disabled = true;
  $('folder-picker-location').disabled = true; $('folder-picker-location-form').querySelector('button').disabled = true;
  $('folder-picker-query').value = query; $('folder-picker-list').replaceChildren(); $('folder-picker-list').setAttribute('aria-busy','true'); $('folder-picker-current').textContent = ''; $('folder-picker-status').textContent = query ? 'Searching folders…' : 'Loading folders…'; showError('folder-picker-error','');
  try {
    const data = await api('/api/projects/browse',{method:'POST',body:{...(path ? {path} : {}),query,hidden:$('folder-picker-hidden').checked},signal:controller.signal});
    if (epoch !== folderPicker.epoch || !$('folder-picker').open) return;
    folderPicker.path = data.path; folderPicker.parent = data.parent; folderPicker.valid = true; $('folder-picker-location').value = data.path; $('folder-picker-current').textContent = data.path;
    for (const entry of data.entries) {
      const button = el('button','folder-picker-entry'); button.type = 'button'; button.append(el('strong','',entry.name),el('span','',data.search ? entry.relative : entry.path));
      button.addEventListener('click',() => loadFolderPicker(entry.path)); $('folder-picker-list').append(button);
    }
    $('folder-picker-status').textContent = `${data.entries.length} folder${data.entries.length === 1 ? '' : 's'} ${data.search ? 'found' : 'shown'}.${data.truncated ? ' Partial results: narrow the folder or search term to see more.' : ''}${data.skipped ? ` ${data.skipped} inaccessible folders skipped.` : ''}${data.search ? ' Search skips dependency folders and symbolic links.' : ' Symbolic links are not shown.'}`;
    if (!data.entries.length) $('folder-picker-list').append(el('p','knowledge-note',data.search ? 'No matching folders found in the searched locations.' : 'No subfolders to display. You can select this folder.'));
  } catch (error) {
    if (error.name !== 'AbortError' && epoch === folderPicker.epoch) { $('folder-picker-status').textContent = 'Could not browse this folder. Enter another path or choose Home.'; showError('folder-picker-error',textError(error)); }
  } finally {
    if (epoch === folderPicker.epoch) { folderPicker.controller = null; $('folder-picker-location').disabled = false; $('folder-picker-location-form').querySelector('button').disabled = false; $('folder-picker-list').setAttribute('aria-busy','false'); $('folder-picker-use').disabled = !folderPicker.valid; $('folder-picker-search').disabled = !folderPicker.valid; $('folder-picker-parent').disabled = !folderPicker.valid || !folderPicker.parent; }
  }
}
$('setup-browse').addEventListener('click',() => { if ($('setup-browse').disabled) return; chooseProjectFolder($('setup-project-path').value.trim(),folder=>{ $('setup-project-path').value=folder; showError('setup-register-error',''); $('setup-register-status').hidden=true; $('setup-register').focus(); },$('setup-browse')); });
$('folder-picker-close').addEventListener('click',() => $('folder-picker').close());
$('folder-picker').addEventListener('close',() => { folderPicker.controller?.abort(); folderPicker.epoch++; folderPicker.opener?.focus(); });
$('folder-picker-location-form').addEventListener('submit',event => { event.preventDefault(); loadFolderPicker($('folder-picker-location').value.trim()); });
$('folder-picker-search-form').addEventListener('submit',event => { event.preventDefault(); if (folderPicker.valid) loadFolderPicker(folderPicker.path,$('folder-picker-query').value.trim()); });
$('folder-picker-home').addEventListener('click',() => loadFolderPicker());
$('folder-picker-parent').addEventListener('click',() => { if (folderPicker.valid && folderPicker.parent) loadFolderPicker(folderPicker.parent); });
$('folder-picker-clear').addEventListener('click',() => loadFolderPicker(folderPicker.path || undefined));
$('folder-picker-hidden').addEventListener('change',() => loadFolderPicker(folderPicker.path || undefined,$('folder-picker-query').value.trim()));
$('folder-picker-use').addEventListener('click',() => {
  if (!folderPicker.valid || !folderPicker.onSelect) return;
  const selected=folderPicker.path, callback=folderPicker.onSelect; $('folder-picker').close(); callback(selected);
});
function updateRegisteredProjects(projects,preferredSetupId = null) {
  if (!Array.isArray(projects) || projects.some(project => !project || typeof project.id !== 'string' || typeof project.name !== 'string')) throw new Error('The runner returned an incomplete project list.');
  state.bootstrap.projects = projects;
  for (const id of ['project','context-project','memory-project','brain-project','skills-project','create-skill-project','accelerator-project','system-project']) setProjectChoices($(id),projects,$(id).value);
  setProjectChoices($('setup-project'),projects,preferredSetupId || $('setup-project').value || $('project').value,true);
  setProjectChoices($('project-switcher'),projects,currentProject());
  renderHistory(); updateControls();
}
function setupSelection() { return {project_id:$('setup-project').value,edition:$('setup-edition').value,tools:[...setupState.tools].sort()}; }
function setupSelectionKey() { return JSON.stringify(setupSelection()); }
function setupTargetLabel(selection) { const project = projectFor(selection.project_id); return `${project?.name || selection.project_id} · ${project?.path || setupState.data?.path || ''} · ${selection.edition} · ${selection.tools.join(', ')}`; }
function invalidateSetupPreview(message = '') { setupState.previewEpoch++; setupState.preview = null; clearTimeout(setupState.expiryTimer); setupState.expiryTimer = null; $('setup-preview').hidden = true; $('setup-preview-files').replaceChildren(); if (!setupState.pending) { $('setup-action-status').textContent = message; $('setup-action-status').hidden = !message; showError('setup-action-error',''); } updateSetupControls(); }
function updateSetupControls() {
  const ready = Boolean(state.bootstrap) && !state.authFailed; const busy = Boolean(setupState.pending); const locked = busy || setupState.registerPending || setupState.projectsPending; const project = projectFor($('setup-project').value); const available = project && project.available !== false && (setupState.projectId !== project.id || setupState.data?.available !== false);
  $('setup-project-path').disabled = !ready || locked; $('setup-register').disabled = !ready || locked; $('setup-register').textContent = setupState.registerPending ? 'Adding project…' : 'Add project';
  $('setup-browse').disabled = !ready || locked;
  $('setup-project').disabled = !state.bootstrap || locked; $('setup-refresh-projects').disabled = !ready || locked;
  $('setup-edition').disabled = !ready || locked || setupState.loading || !available || !setupState.data?.editions?.length;
  for (const input of $('setup-tools').querySelectorAll('input')) input.disabled = !ready || locked || setupState.loading || !available;
  $('setup-preview-button').disabled = !ready || locked || setupState.loading || !available || setupState.projectId !== project?.id || !setupState.data || !$('setup-edition').value || !setupState.tools.size;
  $('setup-preview-button').textContent = setupState.pending === 'preview' ? 'Preparing file preview…' : 'Preview accelerator installation';
  const preview = setupState.preview; const expired = Boolean(preview && Number.isFinite(preview.expires_at) && preview.expires_at * 1000 <= Date.now());
  $('setup-install-button').disabled = !ready || locked || !available || !preview?.can_install || preview.selectionKey !== setupSelectionKey() || expired;
  $('setup-install-button').textContent = setupState.pending === 'install' ? 'Installing…' : 'Install reviewed accelerator';
  $('setup-preview-actions').disabled = busy || !preview; $('setup-preview-path').disabled = busy || !preview;
  if (expired) $('setup-preview-note').textContent = 'This preview expired. Generate a new preview before installing.';
  $('setup-start-session').disabled = !ready || busy || setupState.registerPending || Boolean(state.pending) || !available;
  const edition = setupState.data?.editions?.find(item => item.id === $('setup-edition').value); $('setup-edition-detail').textContent = edition?.release ? `Release ${edition.release}` : '';
}
function renderSetupReadiness() {
  const data = setupState.data; $('setup-readiness').hidden = !data; $('setup-readiness-grid').replaceChildren(); $('setup-providers').replaceChildren(); $('setup-diagnostics').replaceChildren(); if (!data) return;
  const git = data.git || {}; const gitLabel = git.is_git ? `${git.branch || (git.head ? 'Detached HEAD' : 'Unborn branch')}${git.head ? ` · ${git.head.slice(0,8)}` : ''}${git.dirty === true ? ' · uncommitted changes' : git.dirty === false && git.head ? ' · clean' : ''}` : git.reason || 'No Git repository detected';
  const checks = [['Git checkout',gitLabel],['Recorded edition',data.installed_edition ? `${data.installed_edition} · ${data.payload_verified ? 'matches the reviewed payload' : 'payload differs or is unverified'}` : 'No edition recorded by Setup'],['Project files',[['policy','Policy'],['memory_runtime','Memory runtime'],['brain_runtime','Brain runtime']].map(([key,name]) => `${name}: ${data.readiness?.[key] ? 'present' : 'not detected'}`).join('\n')]];
  for (const [label,text] of checks) { const card = el('div','setup-check'); card.append(el('strong','',label),el('span','',text)); $('setup-readiness-grid').append(card); }
  for (const provider of data.providers || []) { const row = el('div','setup-provider'); row.append(el('strong','',`${provider.name || provider.id} · ${provider.available ? 'CLI found' : 'CLI unavailable'}`)); if (!provider.available && provider.detail) row.append(el('span','',provider.detail)); $('setup-providers').append(row); }
  if (data.scope?.length) $('setup-diagnostics').append(el('p','knowledge-note',data.scope.map(String).join(' ')));
  for (const diagnostic of data.diagnostics || []) $('setup-diagnostics').append(el('p','memory-notice',String(diagnostic)));
}
const attachState = {pending:null,codexEpoch:0};
function renderSetupAttach() {
  const info = setupState.data?.accelerator; $('setup-attach').hidden = !info; showError('setup-attach-error',''); $('setup-attach-codex').hidden = true; if (!info) return;
  const editions = (info.editions || []).map(id => ({id,name:id})); const selected = info.edition || info.detected || ''; setOptions($('setup-attach-edition'),[{id:'',name:'Choose an edition…'},...editions],item => item.name,item => item.id,selected);
  if (info.mode === 'installed') $('setup-attach-summary').textContent = 'This project has its own installed accelerator; sessions use those files and nothing is attached.';
  else if (info.mode === 'attached') $('setup-attach-summary').textContent = `${info.edition} is attached from ${info.home}. Nothing is installed in this project. Project Brain, Memory Bank and the index for it are kept in ${info.state}.`;
  else $('setup-attach-summary').textContent = info.detected ? `Nothing is attached yet. The project points to ${info.detected} (${info.evidence}).` : `Nothing is attached. No edition fits automatically (${info.evidence}); choose one to attach.`;
  updateAttachControls();
  const codex = state.bootstrap?.providers?.find(provider => provider.id === 'codex');
  if (info.mode === 'attached' && codex?.available) loadCodexHooks(setupState.projectId);
}
function updateAttachControls() {
  const info = setupState.data?.accelerator; const busy = Boolean(attachState.pending) || Boolean(setupState.pending); const installed = info?.mode === 'installed'; const edition = $('setup-attach-edition').value;
  $('setup-attach-edition').disabled = busy || installed; $('setup-attach-button').disabled = busy || installed || !edition || (info?.mode === 'attached' && info.edition === edition);
  $('setup-attach-button').textContent = attachState.pending === 'attach' ? 'Attaching…' : info?.mode === 'attached' ? 'Switch edition' : 'Attach';
  $('setup-detach-button').hidden = info?.mode !== 'attached'; $('setup-detach-button').disabled = busy; $('setup-detach-button').textContent = attachState.pending === 'detach' ? 'Detaching…' : 'Detach';
  $('setup-attach-codex-trust').disabled = busy; $('setup-attach-codex-trust').textContent = attachState.pending === 'trust' ? 'Recording approvals…' : 'Trust accelerator hooks in Codex';
}
function renderCodexHooks(data) {
  $('setup-attach-codex').hidden = false; const missing = data.total - data.trusted;
  $('setup-attach-codex-status').textContent = missing ? `Codex runs the accelerator's hooks only after they are trusted once: ${data.trusted} of ${data.total} trusted. Trusting records their hashes in your Codex configuration, as its /hooks review does.` : `Codex trusts all ${data.total} accelerator hooks.`;
  $('setup-attach-codex-trust').hidden = !missing;
}
async function loadCodexHooks(projectId) {
  const epoch = ++attachState.codexEpoch; $('setup-attach-codex').hidden = false; $('setup-attach-codex-status').textContent = 'Checking the accelerator hooks Codex trusts…'; $('setup-attach-codex-trust').hidden = true;
  try { const data = await api(`/api/projects/${encodeURIComponent(projectId)}/accelerator/codex-hooks`); if (epoch === attachState.codexEpoch && projectId === setupState.projectId) renderCodexHooks(data); }
  catch (error) { if (epoch === attachState.codexEpoch) $('setup-attach-codex-status').textContent = `Codex hook approval could not be checked: ${textError(error)}`; }
}
async function changeAttachment(action) {
  const projectId = setupState.projectId; if (!projectId || attachState.pending) return; attachState.pending = action; showError('setup-attach-error',''); updateAttachControls();
  try { const data = await api(`/api/projects/${encodeURIComponent(projectId)}/accelerator`,{method:'POST',body:action === 'attach' ? {action,edition:$('setup-attach-edition').value} : {action}}); if (Array.isArray(data.projects)) updateRegisteredProjects(data.projects,projectId); attachState.pending = null; await loadSetup({preservePreview:true}); }
  catch (error) { showError('setup-attach-error',textError(error)); }
  finally { attachState.pending = null; updateAttachControls(); }
}
async function trustCodexHooks() {
  const projectId = setupState.projectId; if (!projectId || attachState.pending) return; attachState.pending = 'trust'; showError('setup-attach-error',''); updateAttachControls();
  try { renderCodexHooks(await api(`/api/projects/${encodeURIComponent(projectId)}/accelerator/codex-hooks`,{method:'POST',body:{}})); }
  catch (error) { showError('setup-attach-error',textError(error)); }
  finally { attachState.pending = null; updateAttachControls(); }
}
function renderSetupTools() {
  const tools = setupState.data?.tools || []; const allowed = new Set(tools.map(tool => tool.id)); setupState.tools = new Set([...setupState.tools].filter(id => allowed.has(id)));
  if (!setupState.toolsInitialized && tools.length) { const initial = allowed.has($('provider').value) ? $('provider').value : allowed.has('codex') ? 'codex' : tools[0].id; setupState.tools.add(initial); setupState.toolsInitialized = true; }
  $('setup-tools').replaceChildren(); for (const tool of tools) { const label = el('label','checkbox'); const input = el('input'); input.type = 'checkbox'; input.value = tool.id; input.checked = setupState.tools.has(tool.id); label.append(input,document.createTextNode(tool.name || tool.id)); if (tool.installed) label.append(el('span','setup-tool-status','files detected')); input.addEventListener('change',() => { if (input.checked) setupState.tools.add(tool.id); else setupState.tools.delete(tool.id); invalidateSetupPreview('Tool selection changed. Prepare a new preview.'); }); $('setup-tools').append(label); }
  updateSetupControls();
}
async function loadSetup({preservePreview = false} = {}) {
  if (!state.bootstrap || setupState.pending) return; const projectId = $('setup-project').value; const changed = projectId !== setupState.projectId; if (changed || !preservePreview) invalidateSetupPreview(); setupState.controller?.abort(); const controller = new AbortController(); setupState.controller = controller; const epoch = ++setupState.epoch;
  if (changed) { $('setup-result').hidden = true; setupState.preferredEdition ||= $('setup-edition').value || null; setOptions($('setup-edition'),[],item => item.name,item => item.id); $('setup-tools').replaceChildren(); }
  setupState.projectId = projectId; setupState.loading = true; setupState.data = null; $('setup-attach').hidden = true; $('setup-readiness-status').textContent = projectId ? 'Inspecting the selected project…' : 'Add an existing local project to get started.'; $('setup-project-location').textContent = projectFor(projectId)?.path || ''; showError('setup-readiness-error',''); renderSetupReadiness(); updateSetupControls();
  if (!projectId) { setupState.loading = false; setupState.controller = null; updateSetupControls(); return; }
  try { const data = await api(`/api/projects/${encodeURIComponent(projectId)}/setup`,{signal:controller.signal}); if (epoch !== setupState.epoch || projectId !== $('setup-project').value) return;
    if (data.project_id !== projectId || !Array.isArray(data.editions) || !Array.isArray(data.tools)) throw new Error('The runner returned incomplete project setup information. Refresh to try again.');
    setupState.data = data; if (typeof data.available === 'boolean' && projectFor(projectId)) projectFor(projectId).available = data.available; $('setup-project-location').textContent = data.path || projectFor(projectId)?.path || ''; $('setup-readiness-status').textContent = projectFor(projectId)?.available === false ? 'This registered project is currently unavailable. Its path is kept in the project list.' : '';
    const preferred = setupState.preferredEdition || $('setup-edition').value || data.installed_edition; setOptions($('setup-edition'),[{id:'',name:'Choose an edition…'},...data.editions],item => item.name,item => item.id,preferred || ''); setupState.preferredEdition = null; renderSetupReadiness(); renderSetupAttach(); renderSetupTools();
  } catch (error) { if (error.name !== 'AbortError' && epoch === setupState.epoch) { $('setup-readiness-status').textContent = projectFor(projectId)?.available === false ? 'The registered folder is unavailable. Restore its path or add an existing folder.' : 'Project inspection could not finish.'; showError('setup-readiness-error',textError(error)); } }
  finally { if (epoch === setupState.epoch) { setupState.loading = false; setupState.controller = null; updateSetupControls(); } }
}
function openSetup(refresh = false) { if (!state.bootstrap) { updateSetupControls(); return; } if (!setupState.pending && (refresh || !setupState.data || setupState.projectId !== $('setup-project').value || setupState.preferredEdition)) loadSetup({preservePreview:!refresh}); else updateSetupControls(); }
function openProjectSetup(projectId,edition) { if (!setupState.pending && !setupState.registerPending) { if (projectId && projectFor(projectId)) $('setup-project').value = projectId; setupState.preferredEdition = edition || null; invalidateSetupPreview(); } setView('setup'); }
async function refreshProjects() {
  if (!state.bootstrap || setupState.projectsPending || setupState.registerPending || setupState.pending) return; setupState.projectsPending = true; updateSetupControls(); showError('setup-register-error','');
  try { const data = await api('/api/projects'); updateRegisteredProjects(data.projects); if (state.view === 'setup') await loadSetup(); }
  catch (error) { showError('setup-register-error',textError(error)); }
  finally { setupState.projectsPending = false; updateSetupControls(); }
}
async function registerProject(event) {
  event.preventDefault(); if ($('setup-register').disabled || !$('setup-register-form').reportValidity()) return; const path = $('setup-project-path').value.trim(); if (!path.startsWith('/') || /[\x00-\x1f\x7f]/.test(path)) { showError('setup-register-error','Enter an absolute path to an existing local project folder.'); return; }
  setupState.registerPending = true; showError('setup-register-error',''); $('setup-register-status').textContent = 'Registering this local folder…'; $('setup-register-status').hidden = false; updateSetupControls();
  try { const data = await api('/api/projects',{method:'POST',body:{path}}); if (!data.project?.id || !Array.isArray(data.projects)) throw new Error('The runner returned an incomplete registration result. Refresh projects before retrying.'); updateRegisteredProjects(data.projects,data.project.id); $('setup-register-status').textContent = `Registered ${data.project.name || path}. The project is available without restarting the runner.`; $('setup-project-path').value = ''; invalidateSetupPreview(); await loadSetup(); }
  catch (error) { $('setup-register-status').hidden = true; showError('setup-register-error',(error.status === 0 ? 'The registration result is uncertain. Refresh projects before trying again. ' : '') + textError(error)); }
  finally { setupState.registerPending = false; updateSetupControls(); }
}
function renderSetupPreviewFiles() {
  const preview = setupState.preview; $('setup-preview-files').replaceChildren(); if (!preview) return; const filter = $('setup-preview-actions').value; const query = $('setup-preview-path').value.trim().toLowerCase(); const files = preview.files.filter(file => (filter === 'all' || filter === 'collision' ? filter === 'all' || file.action === 'collision' : file.action !== 'unchanged') && file.path.toLowerCase().includes(query));
  $('setup-preview-count').textContent = `${files.length} of ${preview.files.length} files shown. Expand a file to inspect its diff.`;
  for (const file of files) { const details = el('details','setup-file'); const summary = el('summary'); summary.append(el('span',`setup-file-action${file.action === 'collision' ? ' collision' : ''}`,humanLabel(file.action)),el('span','setup-file-path',file.path)); details.append(summary); let populated = false;
    details.addEventListener('toggle',() => { if (!details.open || populated) return; populated = true; details.append(el('p','knowledge-note',`${bytesLabel(file.before_bytes)} → ${bytesLabel(file.after_bytes)}${file.source_path ? ` · Source: ${file.source_path}` : ''}`)); if (file.reason) details.append(el('p',file.action === 'collision' ? 'error-text' : 'knowledge-note',file.reason)); if (file.diff) details.append(el('pre','knowledge-result',file.diff)); else details.append(el('p','knowledge-note',file.action === 'unchanged' ? 'This file is unchanged.' : 'No text diff was returned for this file.')); if (file.diff_truncated) details.append(el('p','memory-notice','The text diff is truncated. Review the full source and target files locally if needed.')); }); $('setup-preview-files').append(details); }
  if (!files.length) $('setup-preview-files').append(el('p','empty-note','No files match this action filter and path.'));
}
async function previewSetup() {
  if ($('setup-preview-button').disabled) return; const selection = setupSelection(); const selectionKey = setupSelectionKey(); invalidateSetupPreview(); setupState.pending = 'preview'; const epoch = ++setupState.previewEpoch; $('setup-action-status').textContent = 'Preparing file actions and diffs… This does not install project files.'; $('setup-action-status').hidden = false; showError('setup-action-error',''); updateSetupControls();
  try { const data = await api('/api/accelerators/preview',{method:'POST',body:selection}); if (epoch !== setupState.previewEpoch || selectionKey !== setupSelectionKey()) return;
    if (data.project_id !== selection.project_id || data.edition !== selection.edition || !Array.isArray(data.tools) || JSON.stringify([...data.tools].sort()) !== JSON.stringify(selection.tools) || !Array.isArray(data.files) || data.files.some(file => typeof file.path !== 'string') || !Array.isArray(data.collisions) || typeof data.can_install !== 'boolean' || typeof data.preview_id !== 'string' || !Number.isFinite(data.expires_at)) throw new Error('The runner returned an incomplete installation preview. Generate a new preview.');
    setupState.preview = {...data,selectionKey}; $('setup-preview-target').textContent = setupTargetLabel(selection); $('setup-preview-summary').textContent = data.summary || `${data.file_count} files · ${data.changed_count} proposed changes`; $('setup-preview-note').textContent = data.can_install ? `Review this selection before installing. Preview expires at ${new Date(data.expires_at * 1000).toLocaleTimeString()}; it can be used once.` : 'Installation is blocked by this preview. Resolve the reported issues and prepare a new preview.';
    $('setup-collisions').hidden = !data.collisions.length; $('setup-collisions').textContent = data.collisions.length ? `${data.collisions.length} collisions block installation. Filter by Collisions to inspect their paths and reasons.` : ''; $('setup-preview-actions').value = data.collisions.length ? 'collision' : 'changed'; $('setup-preview-path').value = ''; $('setup-preview').hidden = false; renderSetupPreviewFiles();
    $('setup-action-status').textContent = data.can_install ? 'Preview ready. No project files have been installed.' : 'Preview complete. Installation is blocked.'; setupState.expiryTimer = setTimeout(updateSetupControls,Math.max(0,data.expires_at * 1000 - Date.now()) + 5);
  } catch (error) { if (epoch === setupState.previewEpoch) { $('setup-action-status').hidden = true; showError('setup-action-error',textError(error)); } }
  finally { if (epoch === setupState.previewEpoch) { setupState.pending = null; updateSetupControls(); } }
}
async function installSetup() {
  if ($('setup-install-button').disabled || !setupState.preview) return; const selection = setupSelection(); const previewId = setupState.preview.preview_id; const label = setupTargetLabel(selection); setupState.preview = null; clearTimeout(setupState.expiryTimer); setupState.expiryTimer = null; setupState.pending = 'install'; const epoch = ++setupState.previewEpoch; let confirmed = false; $('setup-preview-note').textContent = 'This preview cannot be reused. Generate a new preview before another installation.'; $('setup-action-status').textContent = 'Installing the reviewed files… You can visit another section; installation will continue.'; $('setup-action-status').hidden = false; showError('setup-action-error',''); $('setup-result').hidden = true; updateSetupControls();
  try { const data = await api('/api/accelerators/install',{method:'POST',body:{preview_id:previewId}}); if (epoch !== setupState.previewEpoch) return;
    if (typeof data.ok !== 'boolean' || !Array.isArray(data.installed) || !Array.isArray(data.merged) || !Array.isArray(data.unchanged)) throw new Error('The runner returned an incomplete installation result. Some files may have been written. Refresh and prepare a new preview before retrying.');
    confirmed = data.ok; setupState.resultProjectId = selection.project_id; $('setup-result-title').textContent = data.ok ? 'Accelerator installation complete' : 'Installation did not complete'; $('setup-result-target').textContent = label; $('setup-result-summary').textContent = data.summary || (data.ok ? 'Reviewed files were installed.' : 'Check the reported file results before preparing another preview.'); $('setup-result-paths').textContent = [...data.installed.map(path => `Installed  ${path}`),...data.merged.map(path => `Merged     ${path}`),...(Array.isArray(data.repaired) ? data.repaired : []).map(path => `Mode fixed ${path}`),...data.unchanged.map(path => `Unchanged  ${path}`)].join('\n'); $('setup-result').hidden = false; showError('setup-action-error',data.ok ? '' : data.error || 'Installation may be partial. Review the reported files and prepare a new preview.'); $('setup-action-status').textContent = data.ok ? 'Installation complete. Refreshing project readiness…' : 'Installation did not complete. Refresh and prepare a new preview.';
  } catch (error) { if (epoch === setupState.previewEpoch) { $('setup-action-status').textContent = 'Installation was not confirmed.'; showError('setup-action-error',`${textError(error)} The preview is consumed. Refresh and generate a new preview before retrying.${error.status === 0 ? ' The connection was lost; some files may have been installed.' : ''}`); } }
  finally { if (epoch === setupState.previewEpoch) { setupState.pending = null; $('setup-preview').hidden = true; $('setup-preview-files').replaceChildren(); updateSetupControls(); if ($('setup-project').value === selection.project_id) await loadSetup({preservePreview:true}); if (confirmed) $('setup-action-status').textContent = setupState.data ? 'Installation complete. Project readiness refreshed.' : 'Installation complete. Refresh to inspect the project.'; } }
}
$('setup-register-form').addEventListener('submit',registerProject); $('setup-refresh-projects').addEventListener('click',refreshProjects); $('setup-project').addEventListener('change',() => loadSetup()); $('setup-edition').addEventListener('change',() => invalidateSetupPreview('Edition changed. Prepare a new preview.'));
$('setup-attach-edition').addEventListener('change',updateAttachControls); $('setup-attach-button').addEventListener('click',() => changeAttachment('attach')); $('setup-detach-button').addEventListener('click',() => changeAttachment('detach')); $('setup-attach-codex-trust').addEventListener('click',trustCodexHooks);
$('setup-preview-button').addEventListener('click',previewSetup); $('setup-install-button').addEventListener('click',installSetup); $('setup-preview-actions').addEventListener('change',renderSetupPreviewFiles); $('setup-preview-path').addEventListener('input',renderSetupPreviewFiles);
$('setup-start-session').addEventListener('click',() => { if (!$('setup-start-session').disabled) newSession('',$('setup-project').value); });
$('setup-result-start').addEventListener('click',() => { const projectId = setupState.resultProjectId; if (projectId && projectFor(projectId) && !state.pending) newSession('',projectId); });
