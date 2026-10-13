// Skills: catalog, installation, installed skills and Create skill.
// Classic script loaded in order by index.html; top-level names are shared with the other app-*.js files.
'use strict';
function skillsSelection() { return {project_id:$('skills-project').value,source_id:$('skills-source').value,skills:[...skillsState.selectedSkills].sort(),agents:[...skillsState.selectedAgents].sort()}; }
function skillsSelectionKey() { return JSON.stringify(skillsSelection()); }
function skillAgentName(id) { return skillsState.catalog?.agents.find(agent => agent.id === id)?.name || id; }
function skillsSelectionLabel(selection) { const source = skillsState.catalog?.sources.find(item => item.id === selection.source_id); return `${projectFor(selection.project_id)?.name || selection.project_id} · ${source?.name || selection.source_id} · ${selection.agents.map(skillAgentName).join(', ')}`; }
function setSkillsActionStatus(text) { $('skills-action-status').textContent = text; $('skills-action-status').hidden = !text; }
function invalidateSkillsPreview() {
  skillsState.preview = null; skillsState.actionEpoch++; $('skills-preview').hidden = true; $('skills-result').hidden = true;
  if (!skillsState.pending) { setSkillsActionStatus(''); showError('skills-action-error',''); }
  updateSkillsControls();
}
function updateSkillsControls() {
  const ready = Boolean(state.bootstrap) && !state.authFailed; const locked = Boolean(skillsState.pending) || skillsState.catalogPending;
  const available = ready && skillsState.catalog?.available === true && !skillsState.catalogError;
  const selection = skillsSelection(); const loaded = skillsState.discoveredSourceId === selection.source_id;
  $('skills-project').disabled = !state.bootstrap || Boolean(skillsState.pending);
  $('skills-source').disabled = !ready || locked || !skillsState.catalog?.sources.length;
  $('skills-discover').disabled = !available || locked || !selection.source_id;
  $('skills-source-refresh').disabled = $('skills-discover').disabled;
  updateSkillChangeControls();
  $('skills-discover').textContent = skillsState.pending === 'discover' ? 'Loading skills…' : 'Load skills';
  for (const input of $('skills-agents').querySelectorAll('input')) input.disabled = !available || locked;
  for (const input of $('skills-list').querySelectorAll('input')) input.disabled = !available || locked;
  $('skills-filter').disabled = locked || !skillsState.skills.length;
  $('skills-preview-button').disabled = !available || locked || !selection.project_id || !loaded || !selection.skills.length || !selection.agents.length;
  $('skills-preview-button').textContent = skillsState.pending === 'preview' ? 'Preparing preview…' : 'Preview installation';
  $('skills-install').disabled = !available || locked || !skillsState.preview?.can_install || !skillsState.preview?.preview_id || skillsState.preview.selectionKey !== skillsSelectionKey();
  $('skills-install').textContent = skillsState.pending === 'install' ? 'Installing…' : 'Install selected skills';
  $('skills-installed-refresh').disabled = !state.bootstrap || !selection.project_id || skillsState.installedPending || Boolean(skillsState.pending);
  $('skills-installed-agent').disabled = skillsState.installedPending;
  updateCreateSkillControls();
}
function renderSkillSource() {
  const source = skillsState.catalog?.sources.find(item => item.id === $('skills-source').value);
  $('skills-source-description').textContent = source?.description || ''; $('skills-source-link').hidden = true; $('skills-source-link').removeAttribute('href');
  if (source?.url) { try { const url = new URL(source.url); if (['https:','http:'].includes(url.protocol)) { $('skills-source-link').href = url.href; $('skills-source-link').hidden = false; } } catch (_) {} }
}
function clearDiscoveredSkills() { $('skills-availability').hidden = false; skillsState.discoveredSourceId = null; skillsState.skills = []; skillsState.selectedSkills.clear(); $('skills-filter').value = ''; $('skills-discovery-notice').hidden = true; invalidateSkillsPreview(); renderSkillChoices(); }
function visibleSkills() { const query = $('skills-filter').value.trim().toLowerCase(); return skillsState.skills.filter(skill => [skill.id,skill.name,skill.description].join(' ').toLowerCase().includes(query)); }
function renderSkillCounts() { const count = skillsState.selectedSkills.size, tools = [...skillsState.selectedAgents].map(skillAgentName); $('skills-selection-summary').textContent = count && tools.length ? `${count} ${count === 1 ? 'skill' : 'skills'} for ${tools.join(', ')}` : 'Select at least one skill and one tool.'; $('skills-selection-count').textContent = skillsState.discoveredSourceId ? `${skillsState.selectedSkills.size} of ${skillsState.skills.length} selected` : 'No skills loaded'; $('skills-visible-count').textContent = $('skills-filter').value.trim() ? `${visibleSkills().length} matching` : ''; }
function renderSkillChoices() {
  const list = $('skills-list'); list.replaceChildren(); const visible = visibleSkills();
  for (const [index,skill] of visible.entries()) {
    const label = el('label','skill-choice'); const input = el('input'); input.type = 'checkbox'; input.value = skill.id; input.checked = skillsState.selectedSkills.has(skill.id); input.id = `skill-choice-${index}`;
    const copy = el('span','skill-choice-copy'); copy.append(el('span','skill-choice-name',skill.name || skill.id)); if (skill.name && skill.name !== skill.id) copy.append(el('span','skill-choice-id',skill.id)); if (skill.description) { const description = el('span','skill-choice-description',skill.description); description.title = skill.description; copy.append(description); } label.append(input,copy); list.append(label);
    input.addEventListener('change',() => { if (input.checked) skillsState.selectedSkills.add(skill.id); else skillsState.selectedSkills.delete(skill.id); invalidateSkillsPreview(); renderSkillCounts(); });
  }
  if (!visible.length) list.append(el('p','empty-note',!skillsState.discoveredSourceId ? 'Choose a source and load its skills to make a selection.' : skillsState.skills.length ? 'No matching skills. Try another name or description.' : 'This source returned no installable skills.'));
  renderSkillCounts(); updateSkillsControls();
}
function renderSkillAgents() {
  const holder = $('skills-agents'); holder.replaceChildren();
  for (const agent of skillsState.catalog?.agents || []) { const label = el('label','checkbox'); const input = el('input'); input.type = 'checkbox'; input.value = agent.id; input.checked = skillsState.selectedAgents.has(agent.id); label.append(input,document.createTextNode(agent.name)); holder.append(label); input.addEventListener('change',() => { if (input.checked) skillsState.selectedAgents.add(agent.id); else skillsState.selectedAgents.delete(agent.id); invalidateSkillsPreview(); renderSkillCounts(); }); }
  if (!holder.children.length) holder.append(el('span','skills-note','No installation tools were reported.'));
  const previous = $('skills-installed-agent').value; const all = el('option','','All tools'); all.value = ''; $('skills-installed-agent').replaceChildren(all);
  for (const agent of skillsState.catalog?.agents || []) { const option = el('option','',agent.name); option.value = agent.id; $('skills-installed-agent').append(option); }
  $('skills-installed-agent').value = [...$('skills-installed-agent').options].some(option => option.value === previous) ? previous : '';
  renderInstalledSkills(); updateSkillsControls();
}
async function loadSkillsCatalog() {
  if (skillsState.catalogPending || skillsState.pending || !state.bootstrap) return;
  skillsState.catalogPending = true; const epoch = ++skillsState.catalogEpoch; showError('skills-catalog-error',''); $('skills-availability').textContent = 'Loading the local skill installer…'; updateSkillsControls();
  try {
    const data = await api('/api/skills'); if (epoch !== skillsState.catalogEpoch) return;
    if (!Array.isArray(data.sources) || !Array.isArray(data.agents) || typeof data.available !== 'boolean') throw new Error('The runner returned an incomplete skill catalog.');
    const before = skillsSelectionKey(); skillsState.catalog = data; skillsState.catalogError = false;
    setOptions($('skills-source'),data.sources,item => item.name,item => item.id,skillsState.sourceId);
    const selectedSource = $('skills-source').value; if (skillsState.sourceId !== selectedSource) clearDiscoveredSkills(); skillsState.sourceId = selectedSource;
    const agentIds = data.agents.map(agent => agent.id); skillsState.selectedAgents = new Set([...skillsState.selectedAgents].filter(id => agentIds.includes(id)));
    if (!skillsState.agentsInitialized) { const preferred = agentIds.includes($('provider').value) ? $('provider').value : agentIds.includes('codex') ? 'codex' : agentIds[0]; if (preferred) skillsState.selectedAgents.add(preferred); skillsState.agentsInitialized = true; }
    if (before !== skillsSelectionKey()) invalidateSkillsPreview();
    $('skills-availability').textContent = !data.available ? data.detail || 'Skill installation is unavailable.' : !data.sources.length ? 'No skill sources are configured.' : 'The first load downloads the source and can take minutes.';
    renderSkillSource(); renderSkillAgents();
  } catch (error) { if (epoch === skillsState.catalogEpoch) { skillsState.catalogError = true; skillsState.preview = null; $('skills-availability').textContent = 'The skill catalog is unavailable. Reopen Skills or reconnect to try again.'; showError('skills-catalog-error',textError(error)); } }
  finally { if (epoch === skillsState.catalogEpoch) { skillsState.catalogPending = false; if (state.view === 'create-skill' || createSkillState.agentsInitialized) renderCreateSkillAgents(); updateSkillsControls(); } }
}
function openSkills(refresh = false) { if (!state.bootstrap) { updateSkillsControls(); return; } if (!skillsState.catalog || skillsState.catalogError || refresh) loadSkillsCatalog(); if (skillsState.pending !== 'install') refreshInstalledSkills(); updateSkillsControls(); }
async function discoverSkills(refresh = false) {
  if ($('skills-discover').disabled) return; const sourceId = $('skills-source').value;
  invalidateSkillsPreview(); clearDiscoveredSkills(); skillsState.pending = 'discover'; const epoch = ++skillsState.actionEpoch;
  setSkillsActionStatus('Downloading and reading the selected source… This may take a few minutes on the first load.'); updateSkillsControls();
  try {
    const data = await api('/api/skills/discover',{method:'POST',body:{source_id:sourceId,refresh}});
    if (epoch !== skillsState.actionEpoch || sourceId !== $('skills-source').value) return;
    if (data.source_id !== sourceId || !Array.isArray(data.skills)) throw new Error('The runner returned an unexpected skill selection. Load the source again.');
    skillsState.discoveredSourceId = sourceId; skillsState.skills = data.skills;
    $('skills-discovery-notice').textContent = data.notice || ''; $('skills-discovery-notice').hidden = !data.notice;
    setSkillsActionStatus(''); $('skills-availability').hidden = true; renderSkillChoices();
  } catch (error) { if (epoch === skillsState.actionEpoch) { setSkillsActionStatus(''); showError('skills-action-error',error.status === 0 ? 'The connection was lost while loading skills. Reconnect, then load the source again.' : textError(error)); } }
  finally { if (epoch === skillsState.actionEpoch) { skillsState.pending = null; updateSkillsControls(); } }
}
function appendSkillFile(holder, path, status, label = status) { const row = el('div','skills-file'); row.append(el('code','',path),el('span',`skills-file-status ${['new','identical','conflict'].includes(status) ? status : ''}`,label)); holder.append(row); }
async function previewSkills() {
  if ($('skills-preview-button').disabled) return;
  const selection = skillsSelection(); const selectionKey = skillsSelectionKey(); invalidateSkillsPreview(); skillsState.pending = 'preview'; const epoch = ++skillsState.actionEpoch;
  setSkillsActionStatus('Preparing an exact file preview…'); updateSkillsControls();
  try {
    const data = await api('/api/skills/preview',{method:'POST',body:selection});
    if (epoch !== skillsState.actionEpoch || selectionKey !== skillsSelectionKey()) return;
    if (data.project_id !== selection.project_id || data.source_id !== selection.source_id || !Array.isArray(data.files) || typeof data.preview_id !== 'string' || typeof data.can_install !== 'boolean') throw new Error('The runner returned an incomplete installation preview. Preview again before installing.');
    skillsState.preview = {...data,selectionKey}; $('skills-preview-context').textContent = skillsSelectionLabel(selection); $('skills-preview-summary').textContent = data.summary || ''; $('skills-preview-files').replaceChildren();
    for (const file of data.files) appendSkillFile($('skills-preview-files'),file.path,file.status,({new:'New',identical:'Identical',conflict:'Conflict'})[file.status] || file.status);
    if (!data.files.length) $('skills-preview-files').append(el('p','skills-note','No file changes were returned.'));
    $('skills-preview-note').textContent = data.can_install ? 'New files will be copied. Identical files will remain unchanged.' : 'Installation is blocked. Address the reported issues, then create a new preview.';
    $('skills-preview').hidden = false; setSkillsActionStatus(data.can_install ? 'Preview ready. Review the files below before installing.' : 'This preview cannot be installed. No files have been installed.');
  } catch (error) { if (epoch === skillsState.actionEpoch) { setSkillsActionStatus(''); showError('skills-action-error',error.status === 0 ? 'The connection was lost while preparing the preview. Reconnect and create a new preview.' : textError(error)); } }
  finally { if (epoch === skillsState.actionEpoch) { skillsState.pending = null; updateSkillsControls(); } }
}
async function installSkills() {
  if ($('skills-install').disabled || !skillsState.preview) return;
  const previewId = skillsState.preview.preview_id; const selection = skillsSelection(); const contextLabel = skillsSelectionLabel(selection);
  skillsState.preview = null; skillsState.pending = 'install'; const epoch = ++skillsState.actionEpoch; showError('skills-action-error',''); $('skills-result').hidden = true;
  $('skills-preview-note').textContent = 'This preview is being used. A new preview is required before another installation.';
  setSkillsActionStatus('Installing selected files… You can visit another page here; this installation will continue.'); updateSkillsControls();
  try {
    const data = await api('/api/skills/install',{method:'POST',body:{preview_id:previewId}}); if (epoch !== skillsState.actionEpoch) return;
    if (data.ok !== true || !Array.isArray(data.installed) || !Array.isArray(data.unchanged)) throw new Error('The runner returned an incomplete installation result. Refresh installed skills to check the outcome.');
    $('skills-result-context').textContent = contextLabel; $('skills-result-summary').textContent = data.summary || 'The selected skills were installed.'; $('skills-result-files').replaceChildren();
    for (const path of data.installed) appendSkillFile($('skills-result-files'),path,'new','Installed');
    for (const path of data.unchanged) appendSkillFile($('skills-result-files'),path,'identical','Unchanged');
    $('skills-result').hidden = false; $('skills-preview-note').textContent = 'This preview has been used. Create a new preview for any further installation.';
    setSkillsActionStatus('Installation complete. Start a new agent session to load new skills.');
  } catch (error) { if (epoch === skillsState.actionEpoch) { setSkillsActionStatus(''); $('skills-preview-note').textContent = 'This preview is no longer available. Create a new preview before trying again.'; showError('skills-action-error',(error.status === 0 ? 'The connection was lost during installation. Its outcome is uncertain. Reconnect and refresh installed skills to check what was written.' : textError(error)) + ' Create a new preview before retrying.'); } }
  finally { if (epoch === skillsState.actionEpoch) { skillsState.pending = null; updateSkillsControls(); refreshInstalledSkills(); } }
}
function invalidateSkillChange() {
  skillsState.changePreview = null; clearTimeout(skillsState.changeTimer); skillsState.changeTimer = null;
  $('skills-change').hidden = true; $('skills-change-result').hidden = true; showError('skills-change-error','');
}
function updateSkillChangeControls() {
  const busy = Boolean(skillsState.pending); const ready = Boolean(state.bootstrap) && !state.authFailed;
  for (const button of $('skills-installed-list').querySelectorAll('button')) button.disabled = !ready || busy || skillsState.installedPending;
  const preview = skillsState.changePreview;
  const expired = preview && preview.expires_at * 1000 <= Date.now();
  $('skills-change-apply').disabled = !ready || busy || !preview?.can_apply || expired || preview?.project_id !== $('skills-project').value;
  $('skills-change-apply').textContent = skillsState.pending === 'change-apply' ? 'Applying…' : !preview ? 'Preview required' : preview.operation === 'remove' ? 'Remove reviewed skill' : 'Apply reviewed update';
  if (expired) $('skills-change-note').textContent = 'This preview expired. Check the skill again before applying.';
}
async function previewSkillChange(skill, operation) {
  if (skillsState.pending || skillsState.installedPending || skillsState.installedProjectId !== $('skills-project').value) return;
  const projectId = $('skills-project').value; invalidateSkillChange(); invalidateSkillsPreview();
  skillsState.pending = 'change-preview'; const epoch = ++skillsState.actionEpoch;
  setSkillsActionStatus(operation === 'update' ? 'Checking the current source commit and preparing a diff…' : 'Checking tracked files before removal…'); updateSkillsControls();
  try {
    const data = await api('/api/skills/change-preview',{method:'POST',body:{project_id:projectId,agent:skill.agent,name:skill.name,operation}});
    if (epoch !== skillsState.actionEpoch || projectId !== $('skills-project').value) return;
    if (data.project_id !== projectId || data.path !== skill.path || data.operation !== operation || !Array.isArray(data.files) || typeof data.can_apply !== 'boolean' || typeof data.preview_id !== 'string' || !Number.isFinite(data.expires_at)) throw new Error('Incomplete skill preview. Check the skill again.');
    skillsState.changePreview = data; $('skills-change-title').textContent = operation === 'remove' ? 'Review skill removal' : 'Review skill update';
    $('skills-change-context').textContent = `${projectFor(projectId)?.name || projectId} · ${skillAgentName(skill.agent)} · ${skill.path}`;
    $('skills-change-version').textContent = `${data.source} · ${data.revision}${data.next_revision ? ` → ${data.next_revision}` : ''}`;
    $('skills-change-summary').textContent = data.summary || ''; $('skills-change-files').replaceChildren();
    for (const file of data.files.filter(file => file.status !== 'identical')) {
      const details = el('details','setup-file'); const title = el('summary','',`${humanLabel(file.status)} · ${file.path}`);
      details.append(title); details.addEventListener('toggle',() => { if (!details.open || details.childElementCount > 1) return;
        details.append(el('p','skills-note',`${file.before_bytes} → ${file.after_bytes} bytes · permissions ${file.before_mode == null ? '—' : file.before_mode.toString(8)} → ${file.after_mode == null ? '—' : file.after_mode.toString(8)}`),el('pre','',file.diff || 'No text difference.'));
        if (file.diff_truncated) details.append(el('p','skills-note','Diff truncated. Inspect the full files locally if needed.'));
      }); $('skills-change-files').append(details);
    }
    if (!data.files.some(file => file.status !== 'identical')) $('skills-change-files').append(el('p','skills-note','No file changes. Applying records the checked source revision.'));
    $('skills-change-note').textContent = data.can_apply ? `Review before applying. Valid until ${new Date(data.expires_at * 1000).toLocaleTimeString()}, once only.` : 'Blocked: local changes are preserved. Restore the tracked version before trying again.';
    $('skills-change').hidden = false; setSkillsActionStatus('Skill check complete. Review the result below.');
    skillsState.changeTimer = setTimeout(updateSkillChangeControls,Math.max(0,data.expires_at * 1000 - Date.now()) + 5);
  } catch (error) { if (epoch === skillsState.actionEpoch) { setSkillsActionStatus('Skill check failed; project files were kept.'); showError('skills-change-error',textError(error)); } }
  finally { if (epoch === skillsState.actionEpoch) { skillsState.pending = null; updateSkillsControls(); } }
}
async function applySkillChange() {
  if ($('skills-change-apply').disabled || !skillsState.changePreview) return;
  const preview = skillsState.changePreview; skillsState.changePreview = null; clearTimeout(skillsState.changeTimer);
  skillsState.pending = 'change-apply'; const epoch = ++skillsState.actionEpoch; updateSkillsControls();
  $('skills-change-note').textContent = 'Preview consumed. Applying the reviewed change…'; showError('skills-change-error','');
  try {
    const data = await api('/api/skills/apply',{method:'POST',body:{preview_id:preview.preview_id}});
    if (epoch !== skillsState.actionEpoch) return;
    if (typeof data.ok !== 'boolean' || !Array.isArray(data.changed)) throw new Error('Incomplete result. Refresh installed skills and inspect the project before retrying.');
    $('skills-change-result').textContent = [data.summary,...data.changed].join('\n'); $('skills-change-result').hidden = false;
    showError('skills-change-error',data.ok ? '' : data.error || 'The operation stopped after a partial change. Inspect the reported files.');
    setSkillsActionStatus(data.ok ? 'Skill change complete.' : 'Skill change did not complete.');
  } catch (error) { if (epoch === skillsState.actionEpoch) showError('skills-change-error',`${textError(error)} Some files may have changed. Refresh installed skills and inspect the project before retrying.`); }
  finally { if (epoch === skillsState.actionEpoch) { skillsState.pending = null; $('skills-change-note').textContent = 'This preview cannot be reused. Check the skill again for another change.'; updateSkillsControls(); refreshInstalledSkills(); } }
}
$('skills-change-apply').addEventListener('click',applySkillChange);
function renderInstalledSkills() {
  const holder = $('skills-installed-list'); holder.replaceChildren(); const agent = $('skills-installed-agent').value;
  const visible = skillsState.installed.filter(skill => !agent || skill.agent === agent);
  const groups = new Map(); for (const skill of visible) { if (!groups.has(skill.name)) groups.set(skill.name,[]); groups.get(skill.name).push(skill); }
  for (const [name,copies] of groups) {
    const row = el('div','skills-file installed-skill'), copy = el('div'), heading = el('p','skills-installed-name',name), tools = el('span','installed-tools');
    for (const skill of copies) {
      const detail = `${skill.path}${skill.managed ? ` · ${skill.source} · ${skill.revision} · ${humanLabel(skill.state)}` : ' · untracked'}`;
      const chip = el('span',`tool-chip${skill.managed ? ' managed' : ''}`,skillAgentName(skill.agent)); chip.title = detail; chip.append(el('span','sr-only',` (${detail})`)); tools.append(chip);
    }
    heading.append(tools); copy.append(heading);
    const actions = el('div','skills-actions');
    for (const skill of copies.filter(item => item.managed)) {
      copy.append(el('p','skills-note',`${skillAgentName(skill.agent)}: ${skill.source} · ${skill.revision} · ${humanLabel(skill.state)}`));
      if (skill.source_id !== 'local') { const update = el('button','button compact',`Check update · ${skillAgentName(skill.agent)}`); update.type = 'button'; update.setAttribute('aria-label',`Check update for ${skill.name} in ${skillAgentName(skill.agent)}`); update.addEventListener('click',() => previewSkillChange(skill,'update')); actions.append(update); }
      const remove = el('button','button compact',`Preview removal · ${skillAgentName(skill.agent)}`); remove.type = 'button'; remove.setAttribute('aria-label',`Preview removal of ${skill.name} in ${skillAgentName(skill.agent)}`); remove.addEventListener('click',() => previewSkillChange(skill,'remove')); actions.append(remove);
    }
    row.append(copy,actions); holder.append(row);
  }
  if (skillsState.installedPending || skillsState.installedError) return;
  const managed = visible.filter(skill => skill.managed).length, untracked = visible.length - managed;
  $('skills-installed-status').textContent = !skillsState.installedProjectId ? 'Choose a project to see its installed skills.' : !visible.length ? `0 installed${agent ? ` for ${skillAgentName(agent)}` : ''}` : `${groups.size} ${groups.size === 1 ? 'skill' : 'skills'} · ${visible.length} ${visible.length === 1 ? 'copy' : 'copies'}${agent ? ` for ${skillAgentName(agent)}` : ''} · ${!managed ? 'all untracked' : untracked ? `${managed} managed, ${untracked} untracked` : 'all managed'}`;
  $('skills-untracked-help').hidden = !untracked;
  if (!visible.length && skillsState.installedProjectId) holder.append(el('p','empty-note',agent ? 'No installed skills were found for this tool.' : 'No installed skills were found in this project.'));
}
async function refreshInstalledSkills() {
  const projectId = $('skills-project').value; if (!state.bootstrap || skillsState.pending === 'install' || skillsState.pending === 'change-apply') return;
  skillsState.installedController?.abort(); const controller = new AbortController(); const epoch = ++skillsState.installedEpoch; skillsState.installedController = controller;
  skillsState.installedProjectId = projectId || null; skillsState.installed = []; skillsState.installedError = false; skillsState.installedPending = true;
  showError('skills-installed-error',''); $('skills-installed-status').textContent = projectId ? 'Reading installed skills…' : 'No registered project is available.'; $('skills-installed-list').replaceChildren(); updateSkillsControls();
  try {
    if (!projectId) return;
    const data = await api(`/api/projects/${encodeURIComponent(projectId)}/skills`,{signal:controller.signal});
    if (epoch !== skillsState.installedEpoch || projectId !== $('skills-project').value) return;
    if (data.project_id !== projectId || !Array.isArray(data.installed)) throw new Error('The runner returned an incomplete installed skill list.');
    skillsState.installed = data.installed;
  } catch (error) { if (error.name !== 'AbortError' && epoch === skillsState.installedEpoch) { skillsState.installedError = true; $('skills-installed-status').textContent = ''; showError('skills-installed-error',textError(error)); } }
  finally { if (epoch === skillsState.installedEpoch) { skillsState.installedPending = false; skillsState.installedController = null; renderInstalledSkills(); updateSkillsControls(); } }
}
$('skills-project').addEventListener('change',() => { invalidateSkillChange(); invalidateSkillsPreview(); refreshInstalledSkills(); });
$('skills-source').addEventListener('change',() => { skillsState.sourceId = $('skills-source').value; clearDiscoveredSkills(); renderSkillSource(); });
$('skills-filter').addEventListener('input',renderSkillChoices);
$('skills-discover').addEventListener('click',() => discoverSkills());
$('skills-source-refresh').addEventListener('click',() => discoverSkills(true));
$('skills-preview-button').addEventListener('click',previewSkills);
$('skills-install').addEventListener('click',installSkills);
$('skills-installed-agent').addEventListener('change',renderInstalledSkills);
$('skills-installed-refresh').addEventListener('click',() => refreshInstalledSkills());
function createSkillDraft() { return {project_id:$('create-skill-project').value,agents:[...createSkillState.selectedAgents].sort(),name:$('create-skill-name').value,description:$('create-skill-description').value,instructions:$('create-skill-instructions').value}; }
function createSkillDraftKey() { return JSON.stringify(createSkillDraft()); }
function createSkillErrors() {
  const draft = createSkillDraft(); const errors = {}; const instructionBytes = new TextEncoder().encode(draft.instructions).length;
  if (!draft.project_id || !projectFor(draft.project_id)) errors.project = 'Choose a registered project for this skill.';
  if (!draft.name) errors.name = 'Enter a skill name, such as review-database-migrations.';
  else if (draft.name.length > 64 || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(draft.name)) errors.name = 'Use 1–64 lowercase letters or numbers, separated by single hyphens. Do not start or end with a hyphen.';
  if (!draft.agents.length) errors.agents = 'Choose at least one tool for this skill.';
  if (!draft.description.trim()) errors.description = 'Describe when the agent should use this skill.';
  else if (Array.from(draft.description).length > 1024) errors.description = 'Keep the description within 1,024 characters.';
  if (!draft.instructions.trim()) errors.instructions = 'Write the instructions the agent should follow.';
  else if (instructionBytes > 32000) errors.instructions = 'Keep the instructions within 32,000 UTF-8 bytes. Some characters use more than one byte.';
  return errors;
}
function updateCreateSkillControls() {
  const ready = Boolean(state.bootstrap) && !state.authFailed; const pending = Boolean(createSkillState.pending); const errors = createSkillErrors();
  for (const name of ['project','name','description','instructions']) { const field = $(`create-skill-${name}`); field.disabled = !state.bootstrap || pending; const message = createSkillState.touched.has(name) ? errors[name] || '' : ''; field.setAttribute('aria-invalid',String(Boolean(message))); showError(`create-skill-${name}-error`,message); }
  for (const input of $('create-skill-agents').querySelectorAll('input')) input.disabled = !state.bootstrap || pending;
  showError('create-skill-agents-error',createSkillState.touched.has('agents') ? errors.agents || '' : '');
  $('create-skill-bytes').textContent = `${new TextEncoder().encode($('create-skill-instructions').value).length.toLocaleString()} / 32,000 UTF-8 bytes`;
  $('create-skill-preview-button').disabled = !ready || pending || !createSkillState.agentsInitialized;
  $('create-skill-preview-button').textContent = createSkillState.pending === 'preview' ? 'Preparing preview…' : 'Preview skill';
  $('create-skill-save').disabled = !ready || pending || Object.keys(errors).length > 0 || !createSkillState.preview?.can_install || !createSkillState.preview?.preview_id || createSkillState.preview.draftKey !== createSkillDraftKey();
  $('create-skill-save').textContent = createSkillState.pending === 'save' ? 'Saving skill…' : 'Save skill';
  $('create-skill-view-installed').disabled = pending || Boolean(skillsState.pending) || !createSkillState.resultProjectId || !projectFor(createSkillState.resultProjectId);
}
function setCreateSkillStatus(text) { $('create-skill-action-status').textContent = text; $('create-skill-action-status').hidden = !text; }
function invalidateCreateSkillPreview() {
  createSkillState.preview = null; createSkillState.resultProjectId = null; createSkillState.epoch++; $('create-skill-preview').hidden = true; $('create-skill-result').hidden = true;
  if (!createSkillState.pending) { setCreateSkillStatus(''); showError('create-skill-action-error',''); }
  updateCreateSkillControls();
}
function renderCreateSkillAgents() {
  if (createSkillState.pending) { updateCreateSkillControls(); return; }
  const agents = skillsState.catalog?.agents || []; const before = createSkillDraftKey(); const agentIds = agents.map(agent => agent.id);
  createSkillState.selectedAgents = new Set([...createSkillState.selectedAgents].filter(id => agentIds.includes(id)));
  if (!createSkillState.agentsInitialized && agents.length) { const preferred = agentIds.includes($('provider').value) ? $('provider').value : agentIds.includes('codex') ? 'codex' : agentIds[0]; createSkillState.selectedAgents.add(preferred); createSkillState.agentsInitialized = true; }
  const holder = $('create-skill-agents'); holder.replaceChildren();
  for (const agent of agents) { const label = el('label','checkbox'); const input = el('input'); input.type = 'checkbox'; input.value = agent.id; input.checked = createSkillState.selectedAgents.has(agent.id); label.append(input,document.createTextNode(agent.name)); holder.append(label); input.addEventListener('change',() => { if (input.checked) createSkillState.selectedAgents.add(agent.id); else createSkillState.selectedAgents.delete(agent.id); createSkillState.touched.add('agents'); invalidateCreateSkillPreview(); }); }
  $('create-skill-catalog-status').textContent = agents.length ? 'Choose the tools that should have access to your skill. No source download is needed.' : skillsState.catalogPending || !skillsState.catalog ? 'Loading supported tools…' : 'No supported tools were reported by the runner.';
  showError('create-skill-catalog-error',!agents.length && skillsState.catalogError ? 'Could not load supported tools. Reopen Create skill or reconnect to try again.' : '');
  if (before !== createSkillDraftKey()) invalidateCreateSkillPreview(); else updateCreateSkillControls();
}
function openCreateSkill(refresh = false) {
  if (!state.bootstrap || createSkillState.pending) { updateCreateSkillControls(); return; }
  if (!skillsState.catalog || skillsState.catalogError || refresh) loadSkillsCatalog();
  renderCreateSkillAgents();
}
function createSkillContext(draft) { return `${draft.name} · ${projectFor(draft.project_id)?.name || draft.project_id} · ${draft.agents.map(skillAgentName).join(', ')}`; }
async function previewCreateSkill(event) {
  event?.preventDefault(); if (createSkillState.pending) return;
  for (const name of ['project','name','agents','description','instructions']) createSkillState.touched.add(name); updateCreateSkillControls();
  if ($('create-skill-preview-button').disabled || Object.keys(createSkillErrors()).length) { const first = Object.keys(createSkillErrors())[0]; if (first === 'agents') $('create-skill-agents').querySelectorAll('input')[0]?.focus(); else if (first) $(`create-skill-${first}`).focus(); return; }
  const draft = createSkillDraft(); const draftKey = createSkillDraftKey(); invalidateCreateSkillPreview(); createSkillState.pending = 'preview'; const epoch = ++createSkillState.epoch;
  setCreateSkillStatus('Generating SKILL.md and checking the target files…'); updateCreateSkillControls();
  try {
    const data = await api('/api/skills/create-preview',{method:'POST',body:draft});
    if (epoch !== createSkillState.epoch || draftKey !== createSkillDraftKey()) return;
    if (data.project_id !== draft.project_id || data.source_id !== 'local' || data.name !== draft.name || typeof data.content !== 'string' || !Array.isArray(data.files) || typeof data.preview_id !== 'string' || typeof data.can_install !== 'boolean') throw new Error('The runner returned an incomplete skill preview. Preview again before saving.');
    createSkillState.preview = {...data,draftKey}; $('create-skill-preview-context').textContent = createSkillContext(draft); $('create-skill-preview-summary').textContent = data.summary || ''; $('create-skill-content').textContent = data.content; $('create-skill-content').scrollTop = 0; $('create-skill-preview-files').replaceChildren();
    for (const file of data.files) appendSkillFile($('create-skill-preview-files'),file.path,file.status,({new:'New',identical:'Identical',conflict:'Conflict'})[file.status] || file.status);
    if (!data.files.length) $('create-skill-preview-files').append(el('p','skills-note','No target files were returned.'));
    $('create-skill-preview-note').textContent = data.can_install ? 'Save copies the new files and keeps identical files unchanged.' : 'Saving is blocked. Address the reported issues, then preview the skill again.';
    $('create-skill-preview').hidden = false; $('create-skill-preview').scrollIntoView({block:'nearest'}); setCreateSkillStatus(data.can_install ? 'Preview ready. Review SKILL.md and its target paths before saving.' : 'This preview cannot be saved. No project files have been changed.');
  } catch (error) { if (epoch === createSkillState.epoch) { setCreateSkillStatus(''); showError('create-skill-action-error',error.status === 0 ? 'The connection was lost while preparing your skill. Your draft is still here. Reconnect and preview it again.' : textError(error)); } }
  finally { if (epoch === createSkillState.epoch) { createSkillState.pending = null; renderCreateSkillAgents(); updateCreateSkillControls(); } }
}
async function saveCreatedSkill() {
  if ($('create-skill-save').disabled || !createSkillState.preview) return;
  const previewId = createSkillState.preview.preview_id; const draft = createSkillDraft(); const context = createSkillContext(draft);
  createSkillState.preview = null; createSkillState.pending = 'save'; const epoch = ++createSkillState.epoch; showError('create-skill-action-error',''); $('create-skill-result').hidden = true;
  $('create-skill-preview-note').textContent = 'This preview is being used. A new preview is required before another save.';
  setCreateSkillStatus('Saving your skill… You can visit another page here; this save will continue.'); updateCreateSkillControls();
  try {
    const data = await api('/api/skills/install',{method:'POST',body:{preview_id:previewId}}); if (epoch !== createSkillState.epoch) return;
    if (data.ok !== true || !Array.isArray(data.installed) || !Array.isArray(data.unchanged)) throw new Error('The runner returned an incomplete save result. Check installed skills before trying again.');
    createSkillState.resultProjectId = draft.project_id; $('create-skill-result-context').textContent = context; $('create-skill-result-summary').textContent = data.summary || 'Your skill was saved.'; $('create-skill-result-files').replaceChildren();
    for (const path of data.installed) appendSkillFile($('create-skill-result-files'),path,'new','Created');
    for (const path of data.unchanged) appendSkillFile($('create-skill-result-files'),path,'identical','Unchanged');
    $('create-skill-result').hidden = false; $('create-skill-preview-note').textContent = 'This preview has been used. Preview the skill again before another save.';
    setCreateSkillStatus('Skill saved. Start a new agent session to load it.');
  } catch (error) { if (epoch === createSkillState.epoch) { setCreateSkillStatus(''); $('create-skill-preview-note').textContent = 'This preview is no longer available. Preview the skill again before retrying.'; showError('create-skill-action-error',(error.status === 0 ? 'The connection was lost during saving. Its outcome is uncertain. Reconnect and refresh installed skills to check what was written.' : textError(error)) + ' Create a new preview before retrying.'); } }
  finally { if (epoch === createSkillState.epoch) { createSkillState.pending = null; renderCreateSkillAgents(); updateCreateSkillControls(); if ($('skills-project').value === draft.project_id) refreshInstalledSkills(); } }
}
for (const name of ['name','description','instructions']) { const field = $(`create-skill-${name}`); field.addEventListener('input',() => { createSkillState.touched.add(name); invalidateCreateSkillPreview(); }); field.addEventListener('blur',() => { createSkillState.touched.add(name); updateCreateSkillControls(); }); }
$('create-skill-project').addEventListener('change',() => { createSkillState.touched.add('project'); invalidateCreateSkillPreview(); });
$('create-skill-form').addEventListener('submit',previewCreateSkill);
$('create-skill-save').addEventListener('click',saveCreatedSkill);
$('create-skill-view-installed').addEventListener('click',() => { if ($('create-skill-view-installed').disabled) return; if ($('skills-project').value !== createSkillState.resultProjectId) { $('skills-project').value = createSkillState.resultProjectId; invalidateSkillsPreview(); } $('skills-installed-agent').value = ''; setView('skills'); });
