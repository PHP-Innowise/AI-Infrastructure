// Sessions › composer: slash commands as the session's CLI offers them. `/` at the start of the message lists the
// CLI's own commands: Claude Code's come from the CLI itself, and for Codex the Harness offers what it does in place of
// the Codex app. In a Codex session `$` where a word starts lists Codex's own skills. Tab inserts the highlighted name;
// Enter inserts it too and, for a command that needs no argument, sends at once, as the CLIs do. Page commands (/clear,
// /new, /model, /effort, /diff, /status) act here instead of reaching the CLI (commands.py). Classic script loaded after
// app-core.js; names and descriptions come from the CLIs and project files and reach the page only through textContent.
'use strict';
// Before the list arrives, the names the page acts on itself, per provider.
const PAGE_COMMANDS = {claude:{clear:'new',reset:'new',new:'new',model:'model',effort:'effort'},codex:{new:'new',model:'model',diff:'diff',status:'status'},cursor:{}};
// The token being typed at the caret: a `/command` that starts the message, or (Codex) a `$skill` where a word starts.
function commandToken(text, caret, skills) {
  const before = text.slice(0,caret);
  const command = /^\/(\S*)$/.exec(before);
  if (command) return {start:0,end:caret + /^\S*/.exec(text.slice(caret))[0].length,trigger:'/',query:command[1]};
  const skill = skills ? /(^|[\s(\[{"'`])\$([A-Za-z0-9._:-]{0,200})$/.exec(before) : null;
  if (!skill) return null;
  return {start:caret - skill[2].length - 1,end:caret + /^[A-Za-z0-9._:-]*/.exec(text.slice(caret))[0].length,trigger:'$',query:skill[2]};
}
// Claude Code's order of matches: a name or alias that starts with the letters, then one with a word that does
// (`:`, `_` and `-` separate words), then one that contains them, then a description that does.
function rankCommands(items, query) {
  const needle = query.toLowerCase(), ranked = [];
  items.forEach((item, index) => {
    const names = [item.name,...(item.aliases || [])].map(name => name.toLowerCase());
    const rank = names.some(name => name.startsWith(needle)) ? 0 : names.some(name => name.split(/[:_-]/).some(word => word.startsWith(needle))) ? 1
      : names.some(name => name.includes(needle)) ? 2 : (item.description || '').toLowerCase().includes(needle) ? 3 : -1;
    if (rank >= 0) ranked.push([rank,index,item]);
  });
  return ranked.sort((a, b) => a[0] - b[0] || a[1] - b[1]).map(entry => entry[2]);
}
// The message with the token replaced by the chosen name, a space when none follows, and where the caret lands.
function insertCommand(text, token, name) {
  const after = text.slice(token.end), replacement = token.trigger + name + (/^\s/.test(after) ? '' : ' ');
  return {replacement,value:text.slice(0,token.start) + replacement + after,caret:token.start + replacement.length};
}
// A message that is a page command, typed alone or with one value (`/model sonnet`): which action, with what argument.
// A sentence that merely starts with one ("/model picker is broken") is not: it is sent, and nothing is cleared.
function pageCommand(text, provider, commands) {
  const match = /^\/(\S+)(?:\s+(\S+))?$/.exec(text.trim());
  if (!match) return null;
  const listed = (commands || []).find(item => item.name === match[1] || (item.aliases || []).includes(match[1]));
  const action = listed ? listed.kind === 'page' && listed.action : (PAGE_COMMANDS[provider] || {})[match[1]];
  // Showing changes or usage takes no value.
  if (!action || match[2] && ['diff','status'].includes(action)) return null;
  return {action,name:match[1],argument:match[2] || ''};
}
const composerCommands = (() => {
  const prompt = $('prompt'), panel = $('skill-hints-panel'), list = $('skill-hints'), note = $('skill-hints-note'), status = $('skill-hints-status');
  // A list fetched for one workspace and CLI; a later open refreshes it once it is half a minute old.
  const FRESH_MS = 30000, SHOWN = 50;
  const ui = {key:null,data:null,error:'',errorAt:null,pending:false,loadedAt:0,epoch:0,open:false,items:[],active:0,token:null,dismissed:null,said:''};
  // Commands apply where the CLI receives the message as the person wrote it: Workspace sessions, not Fleet or Clash.
  function target() {
    if (state.selectedId) {
      const session = state.selected;
      if (!session || session.id !== state.selectedId || session.workflow !== 'native' || ['fleet','clash','creator','system_run','system_discovery'].some(name => session[name])) return null;
      return {key:'s:' + session.id,url:`/api/sessions/${encodeURIComponent(session.id)}/commands`,provider:session.provider};
    }
    const project = $('project').value, provider = $('provider').value;
    if (!project || !provider || $('workflow').value !== 'native' || fleetSelected() || clashSelected()) return null;
    // A chosen existing worktree has its own commands; a new worktree starts from the project's committed ones.
    const query = new URLSearchParams({provider}), worktree = $('workspace').value === 'existing-worktree' ? $('existing-worktree').value : '';
    if (worktree) query.set('worktree',worktree);
    return {key:`p:${project}?${query}`,url:`/api/projects/${encodeURIComponent(project)}/commands?${query}`,provider};
  }
  async function load(where) {
    if (ui.key === where.key && (ui.pending || ui.data && Date.now() - ui.loadedAt < FRESH_MS)) return;
    const epoch = ++ui.epoch;
    if (ui.key !== where.key) ui.data = null;
    Object.assign(ui,{key:where.key,pending:true,error:''});
    try {
      const data = await api(where.url);
      if (epoch !== ui.epoch) return;
      if (!Array.isArray(data?.commands) || !Array.isArray(data.skills) || [...data.commands,...data.skills].some(item => typeof item?.name !== 'string')) throw new Error('The runner returned an incomplete command list.');
      Object.assign(ui,{data,loadedAt:Date.now(),error:data.error || ''});
    } catch (error) { if (epoch === ui.epoch) { ui.data = null; ui.errorAt = ui.token?.start ?? null; ui.error = error.status === 0 ? 'Commands could not be loaded. Type / again to retry.' : textError(error); } }
    finally { if (epoch === ui.epoch) { ui.pending = false; if (ui.open) update(); } }
  }
  function close() {
    if (!ui.open && panel.hidden) return;
    Object.assign(ui,{open:false,items:[],token:null}); panel.hidden = true; prompt.removeAttribute('aria-activedescendant');
  }
  function update() {
    const where = target(), caret = prompt.selectionStart;
    const token = where && document.activeElement === prompt && !prompt.disabled && caret === prompt.selectionEnd ? commandToken(prompt.value,caret,where.provider === 'codex') : null;
    // Escape, or a choice just made, closes the list for that token; typing on, or leaving it, ends that.
    if (!token) { ui.dismissed = null; close(); return; }
    if (ui.dismissed && ui.dismissed.start === token.start && ui.dismissed.query === token.query) { close(); return; }
    ui.dismissed = null;
    // A failed list is fetched again for a new token, not on every key press inside the same one.
    const retry = !ui.error || ui.data || ui.errorAt !== token.start;
    if (ui.key !== where.key || !ui.pending && retry && (!ui.data || Date.now() - ui.loadedAt >= FRESH_MS)) load(where);
    const changed = !ui.open || ui.token?.start !== token.start || ui.token?.query !== token.query;
    const source = ui.key === where.key && ui.data ? token.trigger === '/' ? ui.data.commands : ui.data.skills : [];
    ui.items = rankCommands(source,token.query).slice(0,SHOWN);
    Object.assign(ui,{open:true,token,active:changed ? 0 : Math.min(ui.active,Math.max(ui.items.length - 1,0))});
    render();
  }
  function render() {
    panel.hidden = !ui.open;
    if (!ui.open) return;
    list.replaceChildren(...ui.items.map((item, index) => {
      const row = el('li','skill-hint'); row.id = 'skill-hint-' + index; row.setAttribute('role','option'); row.setAttribute('aria-selected',String(index === ui.active));
      const title = el('span','skill-hint-name',ui.token.trigger + item.name);
      if (item.hint) title.append(el('span','skill-hint-argument',' ' + item.hint));
      row.append(title);
      const about = [item.description || '',item.aliases?.length ? `Also ${item.aliases.map(alias => '/' + alias).join(', ')}` : '',item.kind === 'page' ? 'In the Harness' : ''].filter(Boolean).join(' · ');
      if (about) row.append(el('span','skill-hint-description',about));
      row.addEventListener('click',() => choose(index,false));
      return row;
    }));
    list.hidden = !ui.items.length;
    const total = (ui.token.trigger === '/' ? ui.data?.commands : ui.data?.skills)?.length || 0;
    note.textContent = ui.error || (ui.pending && !ui.data ? `Asking ${providerFor(target()?.provider)?.name || 'the CLI'} for its ${ui.token.trigger === '/' ? 'commands' : 'skills'}…` : !ui.data ? '' : !total ? `No ${ui.token.trigger === '/' ? 'commands' : 'skills'} here.` : !ui.items.length ? `Nothing matches “${ui.token.query}”.` : '');
    note.hidden = !note.textContent;
    if (ui.items.length) { prompt.setAttribute('aria-activedescendant','skill-hint-' + ui.active); list.children[ui.active]?.scrollIntoView?.({block:'nearest'}); }
    else prompt.removeAttribute('aria-activedescendant');
    const said = ui.items.length ? `${ui.items.length} ${ui.token.trigger === '/' ? ui.items.length === 1 ? 'command' : 'commands' : ui.items.length === 1 ? 'skill' : 'skills'}. Up and down arrows choose, Tab inserts, Enter runs.` : note.textContent;
    if (said !== ui.said) { ui.said = said; status.textContent = said; }
  }
  // Tab and a click insert the name. Enter does too, then runs a page command, or sends a command that takes no
  // argument (or only an optional one), as Claude Code and Codex do.
  function choose(index, run) {
    const item = ui.items[index], token = ui.token;
    if (!item || !token) return;
    const next = insertCommand(prompt.value,token,item.name);
    ui.dismissed = {start:token.start,query:item.name}; close();
    prompt.focus(); prompt.setSelectionRange(token.start,token.end);
    // insertText keeps the browser's undo history; a field that refuses it takes the text directly.
    if (!document.execCommand?.('insertText',false,next.replacement)) { prompt.value = next.value; prompt.dispatchEvent(new Event('input',{bubbles:true})); }
    prompt.setSelectionRange(next.caret,next.caret);
    if (!run || token.trigger !== '/') return;
    if (item.kind === 'page') { if (intercept(prompt.value)) return; }
    else if (!item.hint || item.hint.startsWith('[') || /\boptional\b/i.test(item.hint)) { if (!$('send').disabled) $('session-form').requestSubmit(); }
  }
  function focusField(id) { if (state.selectedId && $('session-settings').hidden) $('session-settings-toggle').click(); $(id).focus(); }
  // A page command runs here and the message is cleared; anything else returns false and is sent.
  function intercept(text) {
    const where = target(), found = where && pageCommand(text,where.provider,ui.key === where.key ? ui.data?.commands : null);
    if (!found) return false;
    const done = () => { prompt.value = ''; prompt.dispatchEvent(new Event('input',{bubbles:true})); close(); };
    // A draft is already a new session: starting over would only discard its settings and attachments.
    if (found.action === 'new') { done(); if (state.selectedId) newSession(); return true; }
    if (found.action === 'model') {
      if (found.argument) { refreshModelChoices(found.argument,$('thinking-effort').value); updateControls(); }
      done(); focusField('model-choice'); return true;
    }
    if (found.action === 'effort') {
      if (found.argument && [...$('thinking-effort').options].some(option => option.value === found.argument)) { $('thinking-effort').value = found.argument; $('thinking-effort').dispatchEvent(new Event('change',{bubbles:true})); }
      done(); focusField('thinking-effort'); return true;
    }
    if (!state.selectedId) { showError('composer-error',`/${found.name} shows an open session's ${found.action === 'diff' ? 'changes' : 'usage'}. Start the session first.`); return true; }
    done(); setView(found.action === 'diff' ? 'changes' : 'usage'); return true;
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
    if (event.key === 'Tab' && !event.shiftKey) { event.preventDefault(); choose(ui.active,false); return; }
    if (event.key === 'Enter' && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey) { event.preventDefault(); choose(ui.active,true); }
  });
  $('session-form').addEventListener('submit',() => { ui.dismissed = null; close(); });
  // app-core calls this when the session, project, provider or workspace changes: an open list may now be wrong.
  function sync() { if (ui.open) update(); }
  return {sync,close,intercept};
})();
