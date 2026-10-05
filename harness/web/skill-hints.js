// Sessions › composer: skill hints. Typing / (or $, as Codex writes it) where a word starts lists the skills the
// session's provider loads from its workspace; choosing one writes its name into the message. The message sends the
// names chosen here that it still contains, and the runner turns them, with a skill the message starts with, into an
// explicit request at launch (skill_hints.plan); a name only typed elsewhere stays text. Classic script loaded after
// app-core.js; skill names and descriptions come from project files and reach the page only through textContent.
'use strict';
const SKILL_TOKEN = /(^|[\s(\[{"'`])([/$])([A-Za-z0-9._-]{0,100})$/;
// The /name or $name being typed at the caret: where it starts and ends, its trigger and what follows the trigger.
function skillToken(text, caret) {
  const match = SKILL_TOKEN.exec(text.slice(0,caret));
  if (!match) return null;
  const rest = /^[A-Za-z0-9._-]*/.exec(text.slice(caret))[0];
  return {start:caret - match[3].length - 1,end:caret + rest.length,trigger:match[2],query:match[3]};
}
// Names that start with what was typed come first, then names and descriptions that contain it.
function matchSkills(skills, query) {
  const needle = query.toLowerCase(), ranked = [];
  for (const skill of skills) {
    const name = skill.name.toLowerCase(), text = (skill.description || '').toLowerCase();
    const rank = name.startsWith(needle) ? 0 : name.includes(needle) ? 1 : text.includes(needle) ? 2 : -1;
    if (rank >= 0) ranked.push([rank,skill]);
  }
  return ranked.sort((a, b) => a[0] - b[0] || a[1].name.localeCompare(b[1].name)).map(item => item[1]);
}
// The names a message still contains, in order, out of those chosen from the list for it (the runner's REFERENCE).
function pickedSkills(text, chosen) {
  const found = [];
  for (const match of text.matchAll(/(?<![^\s(\[{"'`])[/$]([A-Za-z0-9][A-Za-z0-9._-]{0,99})(?![A-Za-z0-9._\/-])/g)) {
    const name = chosen.has(match[1]) ? match[1] : match[1].replace(/\.+$/,'');
    if (chosen.has(name) && !found.includes(name)) found.push(name);
  }
  return found.slice(0,5);
}
// The message with the token replaced by the chosen name, a space when none follows, and where the caret lands.
function insertSkill(text, token, name) {
  const after = text.slice(token.end), replacement = token.trigger + name + (/^\s/.test(after) ? '' : ' ');
  return {replacement,value:text.slice(0,token.start) + replacement + after,caret:token.start + replacement.length};
}
const skillHints = (() => {
  const prompt = $('prompt'), panel = $('skill-hints-panel'), list = $('skill-hints'), note = $('skill-hints-note'), status = $('skill-hints-status');
  // A list fetched for one workspace and provider; a later open refreshes it once it is half a minute old.
  const FRESH_MS = 30000, SHOWN = 50;
  const ui = {key:null,data:null,error:'',errorAt:null,pending:false,loadedAt:0,epoch:0,open:false,items:[],active:0,token:null,dismissed:null,said:'',chosen:new Set(),chosenKey:null};
  // Skills apply where the runner writes the prompt itself: not Fleet, Clash or the runs other pages own.
  function target() {
    if (state.selectedId) {
      const session = state.selected;
      if (!session || session.id !== state.selectedId || ['fleet','clash','creator','system_run','system_discovery'].some(name => session[name])) return null;
      return {key:'s:' + session.id,url:`/api/sessions/${encodeURIComponent(session.id)}/skill-hints`,provider:session.provider};
    }
    const project = $('project').value, provider = $('provider').value;
    if (!project || !provider || fleetSelected() || clashSelected()) return null;
    // A chosen existing worktree has its own skills; a new worktree starts from the project's committed ones.
    const query = new URLSearchParams({provider}), worktree = $('workspace').value === 'existing-worktree' ? $('existing-worktree').value : '';
    if (worktree) query.set('worktree',worktree);
    return {key:`p:${project}?${query}`,url:`/api/projects/${encodeURIComponent(project)}/skill-hints?${query}`,provider};
  }
  async function load(where) {
    if (ui.key === where.key && (ui.pending || ui.data && Date.now() - ui.loadedAt < FRESH_MS)) return;
    const epoch = ++ui.epoch;
    if (ui.key !== where.key) ui.data = null;
    Object.assign(ui,{key:where.key,pending:true,error:''});
    try {
      const data = await api(where.url);
      if (epoch !== ui.epoch) return;
      if (!Array.isArray(data?.skills) || data.skills.some(skill => typeof skill?.name !== 'string' || typeof skill.path !== 'string')) throw new Error('The runner returned an incomplete skill list.');
      Object.assign(ui,{data,loadedAt:Date.now()});
    } catch (error) { if (epoch === ui.epoch) { ui.data = null; ui.errorAt = ui.token?.start ?? null; ui.error = error.status === 0 ? 'Skills could not be loaded. Type / again to retry.' : textError(error); } }
    finally { if (epoch === ui.epoch) { ui.pending = false; if (ui.open) update(); } }
  }
  function close() {
    if (!ui.open && panel.hidden) return;
    Object.assign(ui,{open:false,items:[],token:null}); panel.hidden = true; prompt.removeAttribute('aria-activedescendant');
  }
  // Choices belong to one message in one place: a new workspace, or an empty message after sending, starts over.
  function forget(where) {
    const key = where?.key || null;
    if (ui.chosenKey !== key || !prompt.value.trim()) { ui.chosen.clear(); ui.chosenKey = key; }
  }
  function update() {
    const where = target(), caret = prompt.selectionStart;
    forget(where);
    const token = where && document.activeElement === prompt && !prompt.disabled && caret === prompt.selectionEnd ? skillToken(prompt.value,caret) : null;
    // Escape, or a choice just made, closes the list for that token; typing on, or leaving it, ends that.
    if (!token) { ui.dismissed = null; close(); return; }
    if (ui.dismissed && ui.dismissed.start === token.start && ui.dismissed.query === token.query) { close(); return; }
    ui.dismissed = null;
    // A failed list is fetched again for a new token, not on every key press inside the same one.
    const retry = !ui.error || ui.errorAt !== token.start;
    if (ui.key !== where.key || !ui.pending && retry && (!ui.data || Date.now() - ui.loadedAt >= FRESH_MS)) load(where);
    const changed = !ui.open || ui.token?.start !== token.start || ui.token?.query !== token.query;
    ui.items = ui.key === where.key && ui.data ? matchSkills(ui.data.skills,token.query).slice(0,SHOWN) : [];
    Object.assign(ui,{open:true,token,active:changed ? 0 : Math.min(ui.active,Math.max(ui.items.length - 1,0))});
    render();
  }
  function render() {
    panel.hidden = !ui.open;
    if (!ui.open) return;
    list.replaceChildren(...ui.items.map((skill, index) => {
      const item = el('li','skill-hint'); item.id = 'skill-hint-' + index; item.setAttribute('role','option'); item.setAttribute('aria-selected',String(index === ui.active));
      item.append(el('span','skill-hint-name',ui.token.trigger + skill.name));
      // Claude runs such a skill only when a message starts with it; elsewhere in the text it stays a name.
      const about = [skill.start_only ? 'Only at the start of a message' : '',skill.description || ''].filter(Boolean).join(' · ');
      if (about) item.append(el('span','skill-hint-description',about));
      item.addEventListener('click',() => choose(index));
      return item;
    }));
    list.hidden = !ui.items.length;
    const total = ui.data?.skills.length || 0;
    note.textContent = ui.error || (ui.pending && !ui.data ? 'Loading skills…' : !ui.data ? '' : !total ? `No skills in ${ui.data.base || 'this workspace'} for this provider.` : !ui.items.length ? `No skill matches “${ui.token.query}”.` : ui.data.truncated ? `Showing the first ${total} skills of this workspace.` : '');
    note.hidden = !note.textContent;
    if (ui.items.length) { prompt.setAttribute('aria-activedescendant','skill-hint-' + ui.active); list.children[ui.active]?.scrollIntoView?.({block:'nearest'}); }
    else prompt.removeAttribute('aria-activedescendant');
    const said = ui.items.length ? `${ui.items.length} ${ui.items.length === 1 ? 'skill' : 'skills'}. Up and down arrows choose, Enter inserts.` : note.textContent;
    if (said !== ui.said) { ui.said = said; status.textContent = said; }
  }
  function choose(index) {
    const skill = ui.items[index], token = ui.token;
    if (!skill || !token) return;
    const next = insertSkill(prompt.value,token,skill.name);
    ui.chosen.add(skill.name); ui.dismissed = {start:token.start,query:skill.name}; close();
    prompt.focus(); prompt.setSelectionRange(token.start,token.end);
    // insertText keeps the browser's undo history; a field that refuses it takes the text directly.
    if (!document.execCommand?.('insertText',false,next.replacement)) { prompt.value = next.value; prompt.dispatchEvent(new Event('input',{bubbles:true})); }
    prompt.setSelectionRange(next.caret,next.caret);
  }
  prompt.setAttribute('aria-autocomplete','list'); prompt.setAttribute('aria-controls','skill-hints');
  // Mouse down on the list, its scrollbar included, keeps the caret in the message; a click on a row then chooses it.
  panel.addEventListener('mousedown',event => event.preventDefault());
  prompt.addEventListener('input',update);
  prompt.addEventListener('click',update);
  prompt.addEventListener('keyup',event => { if (['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) update(); });
  prompt.addEventListener('blur',close);
  prompt.addEventListener('keydown',event => {
    if (!ui.open || event.isComposing) return;
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); ui.dismissed = {start:ui.token.start,query:ui.token.query}; close(); return; }
    if (!ui.items.length) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); ui.active = (ui.active + (event.key === 'ArrowDown' ? 1 : ui.items.length - 1)) % ui.items.length; render(); return; }
    if (event.key === 'Enter' && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey || event.key === 'Tab' && !event.shiftKey) { event.preventDefault(); choose(ui.active); }
  });
  $('session-form').addEventListener('submit',() => { ui.dismissed = null; close(); });
  // app-core calls this when the session, project, provider or workspace changes: an open list may now be wrong.
  function sync() { forget(target()); if (ui.open) update(); }
  // What a message sends as `skills`: the names chosen from the list that it still contains.
  function picked(text) { return target() ? pickedSkills(text,ui.chosen) : []; }
  return {sync,close,picked};
})();
