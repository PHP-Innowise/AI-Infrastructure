// Sessions › Run view: the Run strip above the composer, its Detailed tabs (Files, Plan, Commands), the run receipt in the
// log and Away Watch (tab title, icon, opt-in notifications, the "While you were away" line and the "New since" separator).
// Classic script loaded right after app-core.js, before bootstrap() polls. Everything it shows comes from RunModel and the
// results endpoint; agent strings reach the page only through textContent, paths and commands inside <bdi dir="ltr">.
'use strict';
const runView = (() => {
  const PREFS = 'harness.runview.v1', SEEN = 'harness.runview.seen.v1';
  const CHECK_WORD = {fail:'failed',ok:'no error',unknown:'result unknown',run:'running',notrun:'not run'};
  const PLAN_GLYPH = {done:'✓',active:'→',pending:'○',cancelled:'⊘'}, PLAN_WORD = {done:'checked',active:'in progress',pending:'not started',cancelled:'cancelled'};
  const PLAN_TOOL = {claude:'TodoWrite',codex:'plan',cursor:'todos'}, TABS = ['files','plan','commands'];
  const storage = {
    read(key) { try { const value = JSON.parse(localStorage.getItem(key) || 'null'); return value && typeof value === 'object' && !Array.isArray(value) ? value : {}; } catch (_) { return {}; } },
    write(key, value) { try { localStorage.setItem(key,JSON.stringify(value)); } catch (_) { /* Storage may be disabled; the choice lasts for this page. */ } },
  };
  const saved = storage.read(PREFS);
  const ui = {sid:null,epoch:0,model:RunModel.create(),pending:null,pendingLive:true,live:false,arrival:0,raf:0,loaded:false,mode:'closed',showAfter:null,
    detailed:saved.mode === 'detailed',tab:TABS.includes(saved.tab) ? saved.tab : 'files',filesView:'map',filter:'all',commandFilter:'all',treeOpen:new Set(['changed','opened','searched']),
    map:null,coldOpen:new Set(),card:null,cardTimer:0,nowKey:'',nowSince:0,chipKey:'',paneKey:{},ticker:0,data:null,dataPending:false,dataStale:false,receipts:new Map(),
    watch:null,ended:null,away:null,awayLine:null,hiddenAt:document.hidden ? Date.now() : null,lostAt:null,lostNotified:false,iconKey:'',reading:false,seenWritten:0,announced:-Infinity,announceTimer:0,announceText:null};
  // The file card moves between <body> and the map (narrow windows): keep the node, never look it up by id.
  const cardNode = $('run-file-card');
  const token = name => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const ms = name => parseFloat(token(name)) || 0;
  // Every scripted animation passes here: reduced motion, a hidden tab and the reading guard each skip it (design 3.6).
  function motion(node, frames, options) {
    if (reducedMotion.matches || document.hidden || reading() || typeof node?.animate !== 'function') return null;
    return node.animate(frames,options);
  }
  // Reading guard: a scrolled-up log, selected text or a draft in progress holds the strip and panel still.
  function reading() {
    const selection = window.getSelection?.(), selected = Boolean(selection && !selection.isCollapsed && selection.anchorNode && $('events').contains(selection.anchorNode));
    return ui.reading || selected || document.activeElement === $('prompt');
  }
  const pad = n => String(n).padStart(2,'0');
  const clock = seconds => { if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) return '—'; const s = Math.max(0,Math.floor(seconds)), h = Math.floor(s / 3600); return h ? `${h}:${pad(Math.floor(s / 60) % 60)}:${pad(s % 60)}` : `${Math.floor(s / 60)}:${pad(s % 60)}`; };
  const duration = seconds => { const s = Math.max(0,Math.round(seconds)), h = Math.floor(s / 3600); return h ? `${h}h ${pad(Math.floor(s / 60) % 60)}m` : `${Math.floor(s / 60)}m ${pad(s % 60)}s`; };
  const wall = at => at === null || at === undefined ? '—' : new Date(at + (ui.model.offset || 0)).toLocaleTimeString([],{hour:'2-digit',minute:'2-digit'});
  const count = (n, one, many = one + 's') => `${fmt.exact(n).text} ${n === 1 ? one : many}`;
  const basename = path => String(path).split('/').pop();
  const middle = (path, max) => { if (path.length <= max) return path; const parts = path.split('/'), tail = parts[parts.length - 1]; const short = parts.length > 2 ? `${parts[0]}/…/${tail}` : tail; return short.length <= max ? short : tail; };
  const bdi = (text, className = '') => { const node = el('bdi',className,RunModel.clean(text)); node.dir = 'ltr'; return node; };
  const button = (text, className, action) => { const node = el('button',className,text); node.type = 'button'; node.addEventListener('click',action); return node; };
  const providerName = id => providerFor(id)?.name || id || 'Agent';
  const savePrefs = () => storage.write(PREFS,{mode:ui.detailed ? 'detailed' : 'calm',tab:ui.tab});

  // ---------- Which launch the strip follows
  // A Harness check sets the session running without a launch, so the newest launch says whether the agent runs.
  // Until the server reports launches, the session status decides as it always did.
  function phase(session = state.selected) {
    if (!session || !active(session)) return null;
    if (session.status === 'queued') return 'queued';
    if (!('launch' in session) || session.launch?.status === 'running') return 'running';
    if (!session.launch) return 'preparing';
    if (!ui.loaded) return 'running';
    // A new turn starts with the person's message; a check adds none.
    return ui.model.lastUserId > ui.model.lastLaunchEventId ? 'preparing' : 'check';
  }
  const nextTurn = (session = state.selected) => ['running','preparing'].includes(phase(session)) || session?.status === 'queued' && ui.model.lastUserId > ui.model.lastLaunchEventId;
  const plainSession = session => !session || isFleetSession(session) || isClashSession(session) || Boolean(session.creator || session.system_run || session.system_discovery);
  function shownRun(session = state.selected) {
    // An earlier turn's details open only while nothing runs; an active launch always shows itself.
    if (ui.showAfter && !phase(session)) return ui.model.runs.get(ui.showAfter) || null;
    // The session names its newest launch: until that launch's events arrive, no earlier turn stands in for it.
    const run = session && 'launch' in session ? session.launch?.id ? ui.model.runs.get(session.launch.id) : null : RunModel.current(ui.model);
    return run && !run.finished && RunModel.native(run) ? run : null;
  }
  function modeOf(session) {
    const p = phase(session), run = shownRun(session);
    if (ui.showAfter && !p && run) return 'after';
    if (!p) return 'closed';
    // While history pages arrive, open calls may still close on a later page: the strip waits for the whole record.
    if (!ui.loaded) return 'waiting';
    return p === 'running' && !plainSession(session) && run && run.cliAt !== null ? 'now' : 'waiting';
  }

  // ---------- Events in (from appendEvent), one render per poll page
  function reset(sid = null) {
    ++ui.epoch; Object.assign(ui,{sid,model:RunModel.create(),pending:null,pendingLive:true,loaded:false,showAfter:null,filter:'all',map:null,chipKey:'',nowKey:'',paneKey:{},data:null,dataPending:false,dataStale:false,watch:null,ended:null,away:document.hidden ? {at:Date.now(),snap:null,runId:null,events:0,firstId:null} : null,awayLine:null,lostAt:null,lostNotified:false,seenWritten:0});
    ui.receipts.clear(); ui.coldOpen.clear(); hideCard(); cancelAnimationFrame(ui.raf); ui.raf = 0;
    // A milestone still waiting to be said belongs to the session being left.
    clearTimeout(ui.announceTimer); ui.announceTimer = 0; ui.announceText = null; $('run-announcer').textContent = '';
    for (const id of ['run-map-main','run-map-rows','run-tree','run-pane-plan','run-pane-commands','run-files-filters']) $(id).replaceChildren();
    $('run-files-filters').dataset.key = ''; $('check-prefill-note').hidden = true; render(null);
  }
  function begin(count) { ui.arrival = Date.now(); ui.live = RunModel.live({loaded:ui.loaded,count,running:phase() === 'running',hidden:document.hidden}); }
  function observe(event) {
    const ch = ui.pending ||= RunModel.changes(); if (!ui.live) ui.pendingLive = false;
    RunModel.observe(ui.model,[event],{arrival:ui.arrival || Date.now(),live:ui.live,changes:ch});
    if (ch.check) { ui.dataStale = true; ch.check = false; }
    if (ch.finished) ui.dataStale = true;
    const id = Number(event.id);
    if (ui.away && ui.loaded && Number.isFinite(id)) { ui.away.events++; if (ui.away.firstId === null) ui.away.firstId = id; }
  }
  function end(count) {
    if (!ui.loaded && count < 250) { ui.loaded = true; firstLoad(); }
    if (ui.loaded && !document.hidden && state.view === 'sessions') rememberSeen(!ui.seenWritten);
    schedule();
  }
  function schedule() { if (!ui.raf) ui.raf = requestAnimationFrame(() => { ui.raf = 0; const ch = ui.pending; ui.pending = null; if (ch) ch.live = ch.live && ui.pendingLive; ui.pendingLive = true; render(ch); }); }

  // ---------- The strip: Now line and chips, or the waiting text
  const chipFor = (spec) => {
    const node = el(spec.tab || spec.act ? 'button' : 'span','run-chip'); if (node.tagName === 'BUTTON') node.type = 'button'; node.dataset.key = spec.key;
    if (spec.label) node.append(el('span','run-chip-label',spec.label));
    if (spec.text) node.append(document.createTextNode(spec.text));
    if (spec.glyphs) spec.glyphs.forEach((list,index) => { node.append(document.createTextNode(index ? ' · ' : ' ')); list.forEach((glyph,at) => { if (at) node.append(document.createTextNode(' ')); node.append(glyphNode(glyph)); }); });
    node.title = spec.title || node.textContent; node.setAttribute('aria-label',spec.aria || node.textContent);
    if (spec.tab) node.addEventListener('click',() => setDetailed(true,spec.tab));
    if (spec.act === 'failed') node.addEventListener('click',firstFailed);
    return node;
  };
  function glyphNode(glyph) { const node = el('span','run-glyph',RunModel.GLYPH[glyph]); node.dataset.glyph = glyph; node.setAttribute('aria-hidden','true'); return node; }
  function chipSpecs(run, session) {
    const specs = [], c = run.counters, ge = run.limited ? '≥' : '', limited = run.limited ? `Step details are not recorded after step ${fmt.exact(run.limited.step).text}; counts are a lower bound.` : '';
    if (run.mode === 'labels') specs.push({key:'steps',text:count(c.steps,'step'),aria:count(c.steps,'step'),compact:`${c.steps}`,tab:'files'});
    else if (session?.provider === 'codex') specs.push({key:'files',label:'Files',text:`shell reads not listed · ${ge}${c.edited} changed`,title:limited,aria:`Files: shell reads not listed, ${ge ? 'at least ' : ''}${c.edited} changed`,compact:`${ge}${c.edited}`,tab:'files'});
    else if (c.opened || c.edited) specs.push({key:'files',label:'Files',text:`${ge}${c.opened} opened · ${ge}${c.edited} changed`,title:limited,aria:`Files: ${ge ? 'at least ' : ''}${c.opened} opened, ${c.edited} changed`,compact:`${ge}${c.opened} · ${ge}${c.edited}`,tab:'files'});
    else specs.push({key:'files',text:'No files opened yet',compact:'0',tab:'files'});
    if (session?.workflow === 'sdd' && session.sdd?.phase) { const name = (state.bootstrap?.sdd_phases || []).find(item => item.id === session.sdd.phase)?.name || humanLabel(session.sdd.phase); specs.push({key:'sdd',text:`SDD · ${name}`}); }
    if (run.plan) { const done = run.plan.items.filter(item => item.status === 'done').length; specs.push({key:'plan',label:'Plan',text:`${done} of ${run.plan.total}`,aria:`Plan ${done} of ${run.plan.total} checked by the agent`,compact:`${done}/${run.plan.total}`,tab:'plan'}); }
    const series = RunModel.checks(run).slice(-2);
    if (series.length) specs.push({key:'checks',label:'Checks',glyphs:series.map(row => row.runs.slice(-4).map(item => item.glyph)),tab:'commands',title:series.map(row => `${row.family} ${row.target}`).join(' · '),
      aria:`Checks: ${series.map(row => `${row.family} ${row.runs.slice(-4).map(item => CHECK_WORD[item.glyph]).join(', ')}`).join('; ')}`});
    if (c.failed) specs.push(run.firstFailedEventId === null ? {key:'failed',text:count(c.failed,'failed step')}
      : {key:'failed',text:count(c.failed,'failed step'),act:'failed',aria:`${count(c.failed,'failed step')}; show the first one`});
    return specs;
  }
  function elapsedOf(run, session) {
    const started = session?.launch?.id === run.id ? Date.parse(session.launch.started_at || '') : NaN, start = Number.isFinite(started) ? started : run.startAt === null ? null : run.startAt + (ui.model.offset || 0);
    if (start === null) return null;
    return ((run.finished && run.endAt !== null ? run.endAt + (ui.model.offset || 0) : Date.now()) - start) / 1000;
  }
  function renderTime(run, session) {
    const node = $('run-time'); if (!node || !run) return;
    const elapsed = elapsedOf(run,session), cap = session?.budgets?.seconds, warn = !run.finished && Number.isFinite(cap) && elapsed !== null && elapsed >= cap * .8;
    if (warn) {
      const stops = Date.now() + (cap - elapsed) * 1000, at = new Date(stops).toLocaleTimeString([],{hour:'2-digit',minute:'2-digit'});
      node.textContent = `▲ ${clock(elapsed)} of ${clock(cap)} · stops at ${at}`; node.setAttribute('aria-label',`Time ${clock(elapsed)} of ${clock(cap)}; the launch stops at ${at}.`);
    } else { node.textContent = clock(elapsed); node.setAttribute('aria-label',elapsed === null ? 'Run time not recorded' : `Run time ${clock(elapsed)}`); }
    if (node.dataset.warn !== String(warn)) { node.dataset.warn = String(warn); if (warn) motion(node,[{opacity:.4},{opacity:1}],{duration:ms('--motion-fast'),easing:token('--ease-standard')}); }
  }
  function renderChips(run, session) {
    const specs = chipSpecs(run,session), key = JSON.stringify([ui.mode,specs.map(spec => [spec.key,spec.text,spec.glyphs,spec.title])]);
    if (key === ui.chipKey) return; ui.chipKey = key;
    const box = $('run-chips'); box.replaceChildren(...specs.map(chipFor)); if (ui.mode === 'now') { const time = el('span','run-chip run-time'); time.id = 'run-time'; box.append(time); }
    const compact = $('run-compact'), bits = specs.filter(spec => spec.compact).map(spec => spec.compact), series = specs.find(spec => spec.glyphs);
    compact.replaceChildren(document.createTextNode(bits.join(' · ')));
    if (series) { compact.append(document.createTextNode(bits.length ? ' · ' : '')); for (const glyph of series.glyphs[series.glyphs.length - 1]) compact.append(glyphNode(glyph)); }
    compact.setAttribute('aria-label',`Run details. ${specs.map(spec => spec.aria || spec.text).join('. ')}`);
  }
  // The map's "now" chip takes the pulse only when exactly one main-thread call is open and the map is on screen.
  function pulseKey(run) {
    if (!run || run.finished || !ui.detailed || ui.tab !== 'files' || ui.filesView !== 'map' || ui.mode !== 'now' || run.open.size !== 1) return null;
    const call = run.calls.get([...run.open][0]), key = call?.pathKeys?.[0], tile = key ? ui.map?.tiles.get(key) : null;
    // A chip in a folded block or a closed vendor/ or outside row is not on screen; the strip's dot keeps the loop.
    return tile && tile.node.isConnected && tile.node.offsetParent !== null ? key : null;
  }
  // One loop on screen: called after the map is drawn, so a chip made on this page is seen before the dot gives way.
  function renderPulse(run) { $('run-dot').dataset.pulse = String(ui.mode === 'now' && !pulseKey(run)); }
  function renderStrip(ch, anim) {
    const view = $('run-view'), session = state.selected, open = ui.mode !== 'closed', run = shownRun(session);
    const n = ui.mode === 'now' && run ? RunModel.now(ui.model,run) : null, narrow = mobileNav.matches;
    const waiting = !open || ui.mode === 'waiting' || !run || ui.mode === 'now' && !n, panel = !waiting && ui.detailed;
    // Keyboard focus in a part that is about to hide or go inert would fall to <body>: it moves first, to the Detailed
    // switch while the Now line stays, else to the turn's receipt or the composer.
    const focused = document.activeElement, inPanel = $('run-panel').contains(focused) || cardNode.contains(focused);
    if ((view.contains(focused) || inPanel) && (waiting || !panel && inPanel)) {
      if (waiting) focusOut(); else detailToggle().focus({preventScroll:true});
    }
    if (!panel && ui.card) hideCard();
    if (view.dataset.open !== String(open)) { view.dataset.open = String(open); view.inert = !open; }
    $('run-card').dataset.detailed = String(panel); $('run-panel').inert = !panel;
    $('run-calm').setAttribute('aria-pressed',String(!ui.detailed)); $('run-detailed').setAttribute('aria-pressed',String(ui.detailed)); $('run-compact').setAttribute('aria-pressed',String(ui.detailed));
    $('waiting').hidden = !open || !waiting; $('run-now').hidden = waiting; $('run-line2').hidden = waiting;
    if (waiting) { stopTicker(); return; }
    let verb, target = '', more = '', time = '', plain = false, full = '';
    if (ui.mode === 'after') {
      const span = secondsOf(run,launchOf(run),false); verb = `Turn ${run.ordinal} ${run.outcome === 'completed' ? 'finished' : run.outcome || 'ended'}${span ? ` · ${span.approx ? '≈ ' : ''}${duration(span.seconds)}` : ''}`; plain = true;
    } else if (n?.kind === 'preparing') { verb = $('waiting-text').textContent; plain = true; }
    else { verb = n.verb; full = n.target || ''; target = n.path ? middle(full,narrow ? 32 : 72) : full; more = n.more || ''; time = clock(n.clock); }
    const key = `${verb}|${full}|${more}`;
    if (key !== ui.nowKey) {
      // One change per poll page; text that held under three seconds is replaced without a crossfade, so quick searches do not flicker.
      if (ch && anim && Date.now() - ui.nowSince >= 3000) motion($('run-now-text'),[{opacity:0},{opacity:1}],{duration:ms('--motion-fast'),easing:token('--ease-enter')});
      ui.nowKey = key; ui.nowSince = Date.now();
    }
    $('run-verb').textContent = verb; $('run-verb').dataset.plain = String(plain); $('run-target').textContent = RunModel.clean(target); $('run-target').title = RunModel.clean(full); $('run-more').textContent = more; $('run-clock').textContent = n?.kind === 'preparing' ? '' : time;
    renderPulse(run); $('run-dot').dataset.state = ui.mode === 'now' ? 'running' : 'done';
    const quiet = n?.kind === 'model' && n.quiet !== null && n.quiet >= 30, away = ui.awayLine?.runId === run.id ? ui.awayLine.text : null;
    $('run-away').hidden = !away; if (away) $('run-away-text').textContent = away;
    $('run-quiet').hidden = Boolean(away) || !quiet; if (quiet) $('run-quiet').textContent = `No new events for ${Math.floor(n.quiet)} s. The launch is still running.`;
    // The quiet and away lines take the chips' place; the narrow window's compact chip stays, as it is the only Detailed switch there.
    $('run-chips').hidden = Boolean(away) || quiet;
    $('run-notify').hidden = ui.mode !== 'now' || !('Notification' in window) || Notification.permission !== 'default';
    $('run-close').hidden = ui.mode !== 'after';
    renderChips(run,session); renderTime(run,session);
    const chips = chipSpecs(run,session).map(spec => spec.aria || spec.text).join(', ');
    $('run-now').setAttribute('aria-label',ui.mode === 'after' ? `${verb}. Close the run details` : `Now: ${verb}${full ? ` ${RunModel.clean(full)}` : ''}. ${chips}. Show steps`);
    if (ui.mode === 'now') startTicker(); else stopTicker();
  }
  // Clocks are text that ticks once a second; nothing moves.
  function startTicker() { if (!ui.ticker) ui.ticker = setInterval(() => { if (!document.hidden && ui.mode === 'now') renderStrip(null,false); },1000); }
  function stopTicker() { clearInterval(ui.ticker); ui.ticker = 0; }

  // ---------- Detailed panel
  function setDetailed(on, tab) {
    if (tab) ui.tab = tab; ui.detailed = on; savePrefs(); if (!on) hideCard();
    render(null);
    if (on && tab) $(`run-tab-${ui.tab}`)?.focus({preventScroll:true});
  }
  // The panel takes at most min(55vh, 520px), 50vh in a narrow window, and never pushes the composer off screen.
  function fitPanel() {
    const view = $('sessions-view'); if (!view.clientHeight) return;
    const used = $('composer-area').offsetHeight + $('run-strip').offsetHeight + $('output-resizer').offsetHeight + 120 + 48;
    const cap = mobileNav.matches ? innerHeight * .5 : Math.min(innerHeight * .55,520);
    $('run-panel').style.maxHeight = `${Math.max(120,Math.min(cap,view.clientHeight - used))}px`;
  }
  function renderPanel(ch, anim) {
    const run = shownRun(), visible = ui.mode !== 'closed' && ui.mode !== 'waiting' && ui.detailed;
    if (!run) return;
    const tabs = {files:true,plan:Boolean(run.plan),commands:run.counters.commands > 0}; if (!tabs[ui.tab]) ui.tab = 'files';
    for (const name of TABS) {
      const tab = $(`run-tab-${name}`), selected = ui.tab === name; tab.hidden = !tabs[name]; tab.setAttribute('aria-selected',String(selected)); tab.tabIndex = selected ? 0 : -1; $(`run-pane-${name}`).hidden = !selected;
    }
    $('run-badge-files').textContent = run.mode === 'targets' ? `${run.paths.size}` : '';
    $('run-badge-plan').textContent = run.plan ? `${run.plan.items.filter(item => item.status === 'done').length}/${run.plan.total}` : '';
    $('run-badge-commands').textContent = run.counters.commands ? `${run.counters.commands}` : '';
    if (!visible) return;
    fitPanel();
    if (ui.tab === 'files') renderFiles(run,ch,anim);
    if (ui.tab === 'plan') renderPlan(run,ch,anim);
    if (ui.tab === 'commands') renderCommands(run,ch,anim);
  }

  // ---------- Files: named chips in folder blocks, and the same set as a tree
  const fileWords = p => [p.opened ? p.opened === 1 ? 'opened once' : `opened ${p.opened} times` : '',p.created ? 'created' : '',p.deleted ? 'deleted' : '',p.edited ? p.edits > 1 ? `edit reported ok ${p.edits} times` : 'edit reported ok' : '',
    p.failed ? 'a tool reported an error' : '',p.helper ? 'by a helper agent' : '',p.outside ? 'outside the project, name only' : ''].filter(Boolean);
  const matches = (run, p) => ui.filter === 'all' || ui.filter === 'edited' && p.edited || ui.filter === 'searched' && RunModel.searchedIn(run,p).length > 0 || ui.filter === 'failed' && p.failed || ui.filter === 'outside' && p.outside;
  function renderFiles(run, ch, anim) {
    const labels = run.mode === 'labels', codex = state.selected?.provider === 'codex', paths = [...run.paths.values()], ge = run.limited ? '≥' : '';
    const edited = paths.filter(p => p.edited).length, failed = paths.filter(p => p.failed).length, folders = new Set(paths.filter(p => !p.outside).map(p => p.path.split('/').slice(0,-1).join('/') || '.')).size;
    $('run-files-summary').textContent = labels ? 'Tool names only' : paths.length ? `${ge}${count(paths.length,'file')} · ${count(folders,'folder')} · ${ge}${edited} edited · ${failed} failed` : 'No files yet';
    const note = $('run-files-note'); note.hidden = !labels && !codex;
    note.textContent = labels ? `This run recorded tool names only${run.tools.size ? ` (${RunModel.tally(run.tools)})` : ''}, without file paths.` : 'Codex reads through shell commands; reads are not listed. Edits come from file_change.';
    $('run-files-caption').hidden = labels; $('run-files-banner').hidden = !run.limited || labels; $('run-files-empty').hidden = labels || paths.length > 0;
    $('run-files-empty').textContent = codex ? 'Files the agent changes appear here.' : 'Files the agent opens appear here.';
    $('run-files-view').hidden = labels; $('run-view-map').setAttribute('aria-pressed',String(ui.filesView === 'map')); $('run-view-list').setAttribute('aria-pressed',String(ui.filesView === 'list'));
    const map = !labels && ui.filesView === 'map'; $('run-map').hidden = !map; $('run-tree').hidden = labels || ui.filesView !== 'list'; $('run-legend').hidden = !map || !paths.length; $('run-files-filters').hidden = !map || !paths.length;
    if (labels) return;
    renderFilters(run,paths); renderMap(run,ch,anim && map);
    if (ui.filesView === 'list') renderTree(run);
  }
  function renderFilters(run, paths) {
    const counts = {all:paths.length,edited:paths.filter(p => p.edited).length,searched:paths.filter(p => RunModel.searchedIn(run,p).length).length,failed:paths.filter(p => p.failed).length,outside:paths.filter(p => p.outside).length};
    const box = $('run-files-filters'), key = JSON.stringify([counts,ui.filter]); if (box.dataset.key === key) return; box.dataset.key = key;
    box.replaceChildren(...[['all','All'],['edited','Edited'],['searched','Searched'],['failed','Failed'],['outside','Outside']].filter(([id]) => id === 'all' || counts[id]).map(([id,label]) => {
      const node = button(id === 'all' ? label : `${label} ${counts[id]}`,'run-filter',() => { ui.filter = id; render(null); }); node.setAttribute('aria-pressed',String(ui.filter === id)); return node;
    }));
  }
  function district(name) {
    let d = ui.map.districts.get(name); if (d) return d;
    const cold = name.startsWith('cold:') || name === 'outside';
    if (cold) {
      const wrap = el('div','run-cold'), toggle = el('button','run-cold-row'), blocks = el('div','run-blocks'); toggle.type = 'button'; blocks.hidden = !ui.coldOpen.has(name);
      toggle.addEventListener('click',() => { if (ui.coldOpen.has(name)) ui.coldOpen.delete(name); else ui.coldOpen.add(name); blocks.hidden = !ui.coldOpen.has(name); coldLabel(d); });
      wrap.append(toggle,blocks); $('run-map-rows').append(wrap); d = {name,el:wrap,label:toggle,blocks,cold:true};
    } else {
      const wrap = el('div','run-district'), label = el('div','run-district-label'), mark = el('span','run-search-mark'), blocks = el('div','run-blocks');
      label.append(bdi(name ? `${name}/` : './'),mark); wrap.append(label,blocks); $('run-map-main').append(wrap); d = {name,el:wrap,label,mark,blocks,cold:false};
    }
    ui.map.districts.set(name,d); return d;
  }
  function coldLabel(d) {
    const files = [...ui.map.tiles.values()].filter(tile => tile.block.district === d).length, open = ui.coldOpen.has(d.name);
    d.label.textContent = d.name === 'outside' ? `Outside the project ${open ? '▾' : '▸'} ${files}` : `${d.name.slice(5)}/ ${open ? '▾' : '▸'} ${files} opened`;
    d.label.setAttribute('aria-expanded',String(open)); d.label.setAttribute('aria-label',d.name === 'outside' ? `Outside the project: ${count(files,'file')}, names only` : `${d.name.slice(5)}: ${count(files,'file')}, collapsed by default`);
  }
  function block(name, blockName, run) {
    const id = `${name}|${blockName}`; let b = ui.map.blocks.get(id); if (b) return {b,fresh:false};
    const d = district(name), box = el('div','run-block'), head = el('button','run-block-head'), label = el('span','run-block-name'), mark = el('span','run-search-mark'), files = el('div','run-files');
    head.type = 'button'; box.setAttribute('role','group'); if (d.cold || blockName) label.append(bdi(d.cold ? name === 'outside' ? 'names only' : 'opened files' : blockName));
    head.append(label,mark); box.append(head,files); d.blocks.append(box);
    box.addEventListener('keydown',onBlockKey);
    b = {id,el:box,head,mark,files,district:d,folder:d.cold ? null : [name,blockName.replace(/\/…$/,'')].filter(Boolean).join('/'),named:Boolean(d.cold || blockName),collapsed:false,manual:false,firstStep:run.counters.steps};
    head.addEventListener('click',() => { b.manual = true; b.collapsed = !b.collapsed; files.hidden = b.collapsed; renderBlockHead(b); });
    ui.map.blocks.set(id,b); return {b,fresh:true};
  }
  function renderBlockHead(b) {
    const run = shownRun(); if (!run) return;
    const files = [...b.files.children].map(node => run.paths.get(node.dataset.key)).filter(Boolean), edits = files.filter(p => p.edited).length, created = files.filter(p => p.created).length;
    const where = b.district.cold ? b.district.name === 'outside' ? 'Outside the project' : b.district.name.slice(5) : b.folder || 'Project root';
    b.el.setAttribute('aria-label',`${where}, ${count(files.length,'file')}${edits ? `, ${edits} edited` : ''}${created ? `, ${created} created` : ''}`);
    b.head.setAttribute('aria-expanded',String(!b.collapsed)); b.head.dataset.collapsed = String(b.collapsed);
    // A search over a whole district (Grep in app/, Glob "app/**") is marked on the district's label, not on each of its blocks.
    const hits = b.folder ? run.searches.filter(item => item.scope && item.scope !== b.district.name && (b.folder === item.scope || b.folder.startsWith(`${item.scope}/`))) : [];
    b.el.dataset.searched = String(hits.length > 0);
    b.mark.textContent = b.collapsed ? `▸ ${count(files.length,'file')}` : hits.length ? `⌕ ${hits[hits.length - 1].pattern}${hits.length > 1 ? ` +${hits.length - 1}` : ''}` : '';
    b.mark.title = hits.map(item => `${item.pattern} in ${item.scope}`).join('\n');
    // Files directly in a district (routes/api.php) need no block name; their head shows only to carry a search or a fold.
    b.head.hidden = !b.named && !b.mark.textContent;
  }
  function fileChip(p) {
    const node = el('button','run-file'), mark = el('span','run-file-mark'), name = bdi(basename(p.path),'run-file-name'), times = el('span','run-file-count');
    node.type = 'button'; node.dataset.key = p.key; node.tabIndex = -1; node.title = p.path;
    for (const part of [mark,name,times]) part.setAttribute('aria-hidden','true'); node.append(mark,name,times);
    node.addEventListener('pointerenter',() => { if (!mobileNav.matches) showCard(p.key,node,false); });
    node.addEventListener('pointerleave',hideCardSoon);
    node.addEventListener('focus',() => { for (const chip of node.parentElement.children) chip.tabIndex = chip === node ? 0 : -1; if (!ui.card?.pinned) showCard(p.key,node,false); });
    node.addEventListener('blur',() => { if (!ui.card?.pinned) hideCardSoon(); });
    node.addEventListener('click',() => showCard(p.key,node,true));
    return node;
  }
  function renderMap(run, ch, anim) {
    if (ui.map?.run !== run.id) { hideCard(); ui.map = {run:run.id,tiles:new Map(),blocks:new Map(),districts:new Map()}; $('run-map-main').replaceChildren(); $('run-map-rows').replaceChildren(); }
    let animated = 0; const fresh = [];
    for (const p of run.paths.values()) {
      let tile = ui.map.tiles.get(p.key);
      if (!tile) {
        const where = RunModel.districtOf(p), made = block(where.district,where.block,run), node = fileChip(p);
        tile = {node,block:made.b}; ui.map.tiles.set(p.key,tile); made.b.files.append(node); if (made.fresh) fresh.push(made.b);
        if (!made.b.files.querySelector('[tabindex="0"]')) node.tabIndex = 0;
        if (anim && made.fresh) motion(made.b.el,[{opacity:0},{opacity:1}],{duration:ms('--motion-fast'),easing:token('--ease-enter')});
        // New files fade and scale in one after another; past twelve on one page they simply appear.
        if (anim && ch?.newPaths.includes(p.key) && animated < 12) { motion(node,[{opacity:0,transform:'scale(.6)'},{opacity:1,transform:'none'}],{duration:ms('--motion-base'),easing:token('--ease-enter'),delay:animated * ms('--motion-stagger'),fill:'backwards'}); animated++; }
      }
      const node = tile.node, wasFailed = node.dataset.failed === 'true', wasEdited = node.dataset.edited === 'true';
      node.dataset.edited = String(p.edited); node.dataset.failed = String(p.failed); node.dataset.created = String(p.created); node.dataset.deleted = String(p.deleted); node.dataset.helper = String(p.helper && !p.main);
      // + created and ! failed are both shown: a failed edit of a file this run created still reads as failed.
      node.firstChild.textContent = `${p.created ? '+' : ''}${p.failed ? '!' : ''}`;
      node.lastChild.textContent = p.opened > 1 ? `×${p.opened}` : '';
      node.dataset.dim = String(!matches(run,p));
      if (anim && (p.failed && !wasFailed || p.edited && !wasEdited)) motion(node,[{opacity:.5},{opacity:1}],{duration:ms(p.edited ? '--motion-base' : '--motion-fast'),easing:token('--ease-standard')});
    }
    // Above about 150 chips, one cold block (no edit, failure or open call, untouched for 50 steps) folds for each new block.
    if (ui.map.tiles.size > 150) for (let made = 0; made < fresh.length; made++) {
      const openKeys = new Set([...run.open,...run.openHelpers].flatMap(id => run.calls.get(id)?.pathKeys || []));
      const cold = [...ui.map.blocks.values()].find(b => !b.manual && !b.collapsed && !b.district.cold && [...b.files.children].every(node => { const p = run.paths.get(node.dataset.key); return p && !p.edited && !p.failed && !openKeys.has(p.key) && p.lastStep < run.counters.steps - 50; }));
      if (cold) { cold.collapsed = true; cold.files.hidden = true; }
    }
    // Each chip's "now" word comes from the call open on that file, not from whichever call opened first.
    const pulse = pulseKey(run), openOn = new Map();
    for (const id of [...run.open,...run.openHelpers]) { const call = run.calls.get(id); for (const key of call?.pathKeys || []) if (!openOn.has(key)) openOn.set(key,call); }
    for (const [key,tile] of ui.map.tiles) {
      const p = run.paths.get(key), call = run.finished ? null : openOn.get(key), now = call ? key === pulse ? 'pulse' : 'static' : null;
      if (now) tile.node.dataset.now = now; else delete tile.node.dataset.now;
      const word = call ? RunModel.kinds.READ.has(call.key) ? 'reading now' : RunModel.kinds.EDIT.has(call.key) || call.changes ? 'editing now' : 'in use now' : '';
      tile.node.setAttribute('aria-label',[p.path,...fileWords(p),word].filter(Boolean).join(', '));
    }
    for (const b of ui.map.blocks.values()) renderBlockHead(b);
    for (const d of ui.map.districts.values()) {
      if (d.cold) { coldLabel(d); continue; }
      const hits = run.searches.filter(item => item.scope === d.name);
      d.mark.textContent = hits.length ? `⌕ ${hits[hits.length - 1].pattern}${hits.length > 1 ? ` +${hits.length - 1}` : ''}` : ''; d.mark.title = hits.map(item => item.pattern).join('\n');
    }
    if (ui.card) fillCard(ui.card.key);
  }
  // Roving focus: arrows move inside a block, Tab moves between blocks, Enter opens the chip's card.
  function onBlockKey(event) {
    const chip = event.target.closest('.run-file'); if (!chip) return;
    const list = [...chip.parentElement.querySelectorAll('.run-file')], index = list.indexOf(chip), step = {ArrowRight:1,ArrowDown:1,ArrowLeft:-1,ArrowUp:-1}[event.key];
    const next = step ? list[(index + step + list.length) % list.length] : event.key === 'Home' ? list[0] : event.key === 'End' ? list[list.length - 1] : null;
    if (next) { event.preventDefault(); next.focus(); }
    else if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); showCard(chip.dataset.key,chip,true); }
  }
  function fillCard(key) {
    const run = shownRun(), p = run?.paths.get(key), card = cardNode; if (!p) return;
    const searched = RunModel.searchedIn(run,p), facts = [];
    if (p.opened) facts.push(`Opened ×${p.opened}`); if (searched.length) facts.push(`Searched in ${searched.slice(-2).join(', ')}`);
    if (p.created) facts.push(state.selected?.provider === 'codex' ? 'Added (file_change)' : 'Created by Write (not opened earlier in this session)');
    if (p.deleted) facts.push('Deleted (file_change)');
    if (p.edited) facts.push(p.edits > 1 ? `Edit reported ok ×${p.edits}` : 'Edit reported ok'); if (p.failed) facts.push('A tool reported an error'); if (p.helper) facts.push('by a helper agent');
    const when = p.firstAt === null ? '' : p.firstAt === p.lastAt ? ` · ${wall(p.firstAt)}` : ` · ${wall(p.firstAt)}–${wall(p.lastAt)}`;
    const path = el('p','run-card-path'); path.append(bdi(p.outside ? `${p.path} (outside the project, name only)` : p.path));
    const actions = el('div','run-card-actions'), copy = button('Copy path','button compact',async () => { copy.textContent = await copyText(p.path) ? 'Copied' : 'Copy failed'; });
    actions.append(button('Show in conversation','button compact',() => { hideCard(); reveal(p.lastEventId); }),button('Add path to message','button compact',() => { hideCard(); insertDraft(p.path); }),copy);
    card.replaceChildren(path,el('p','run-card-facts',facts.join(' · ')),el('p','run-card-meta',`First step ${fmt.exact(p.firstStep).text} · last step ${fmt.exact(p.lastStep).text}${when}`),actions);
    card.setAttribute('aria-label',`File details: ${p.path}`);
  }
  function showCard(key, anchor, pinned) {
    clearTimeout(ui.cardTimer); const card = cardNode; ui.card = {key,anchor,pinned}; fillCard(key); card.hidden = false;
    if (mobileNav.matches) { card.dataset.inline = 'true'; anchor.closest('.run-block').after(card); }
    else {
      delete card.dataset.inline; if (card.parentElement !== document.body) document.body.append(card);
      const box = anchor.getBoundingClientRect(), width = card.offsetWidth, height = card.offsetHeight;
      card.style.left = `${Math.min(Math.max(8,box.left - 8),innerWidth - width - 8)}px`;
      card.style.top = `${box.bottom + 8 + height > innerHeight - 8 ? Math.max(8,box.top - height - 8) : box.bottom + 8}px`;
    }
    if (pinned) card.querySelector('button')?.focus({preventScroll:true});
  }
  function hideCardSoon() { if (ui.card?.pinned) return; clearTimeout(ui.cardTimer); ui.cardTimer = setTimeout(() => hideCard(),200); }
  function hideCard(restore = false) {
    const card = cardNode, anchor = ui.card?.anchor; clearTimeout(ui.cardTimer); card.hidden = true; ui.card = null;
    if (card.parentElement !== document.body) document.body.append(card);
    if (restore && anchor?.isConnected) anchor.focus();
  }
  function renderTree(run) {
    const tree = $('run-tree'), paths = [...run.paths.values()], focused = document.activeElement?.closest?.('#run-tree [data-tree-key]')?.dataset.treeKey;
    const groups = [
      ['changed','Changed (reported by tools)',paths.filter(p => p.edited).sort((a,b) => a.path.localeCompare(b.path)).map(p => ({key:p.key,text:p.path,facts:[p.created ? 'created' : '',p.deleted ? 'deleted' : '',p.edits > 1 ? `edited ×${p.edits}` : 'edited',p.opened ? `opened ×${p.opened}` : ''].filter(Boolean).join(' · '),eventId:p.lastEventId}))],
      ['opened','Opened',paths.filter(p => p.opened).sort((a,b) => a.path.localeCompare(b.path)).map(p => ({key:p.key,text:p.path,facts:[`×${p.opened}`,p.failed ? 'failed' : '',p.helper ? 'helper' : '',p.outside ? 'outside the project' : ''].filter(Boolean).join(' · '),eventId:p.lastEventId}))],
      ['searched','Searched',[...run.searches].filter(item => item.pattern).sort((a,b) => a.pattern.localeCompare(b.pattern)).map(item => ({key:String(item.eventId),text:item.pattern,facts:`in ${item.scope || '.'}${item.helper ? ' · helper' : ''}`,eventId:item.eventId}))]];
    tree.replaceChildren(); let first = true;
    for (const [id,label,items] of groups) {
      if (!items.length) continue; const open = ui.treeOpen.has(id), group = el('li','run-tree-group');
      group.setAttribute('role','treeitem'); group.setAttribute('aria-expanded',String(open)); group.setAttribute('aria-level','1'); group.dataset.treeKey = `g:${id}`; group.tabIndex = first ? 0 : -1; first = false;
      const head = el('div','run-tree-label'); head.append(el('span','',`${open ? '▾' : '▸'} ${label}`),el('span','run-tree-count',String(items.length))); group.append(head);
      head.addEventListener('click',() => { if (ui.treeOpen.has(id)) ui.treeOpen.delete(id); else ui.treeOpen.add(id); renderTree(run); });
      if (open) {
        const list = el('ul'); list.setAttribute('role','group');
        for (const item of items) {
          const leaf = el('li','run-tree-leaf'); leaf.setAttribute('role','treeitem'); leaf.setAttribute('aria-level','2'); leaf.tabIndex = -1; leaf.dataset.treeKey = `${id}:${item.key}`;
          leaf.append(bdi(item.text,'run-tree-path'),el('span','run-tree-facts',item.facts)); leaf.addEventListener('click',() => reveal(item.eventId)); list.append(leaf);
        }
        group.append(list);
      }
      tree.append(group);
    }
    if (focused) [...tree.querySelectorAll('[data-tree-key]')].find(node => node.dataset.treeKey === focused)?.focus();
  }
  function onTreeKey(event) {
    const items = [...$('run-tree').querySelectorAll('[role="treeitem"]')], current = event.target.closest('[role="treeitem"]'), index = items.indexOf(current); if (index < 0) return;
    const go = node => { if (!node) return; for (const item of items) item.tabIndex = -1; node.tabIndex = 0; node.focus(); };
    const group = current.getAttribute('aria-level') === '1' ? current.dataset.treeKey.slice(2) : null;
    if (event.key === 'ArrowDown') { event.preventDefault(); go(items[index + 1]); }
    else if (event.key === 'ArrowUp') { event.preventDefault(); go(items[index - 1]); }
    else if (event.key === 'Home' || event.key === 'End') { event.preventDefault(); go(event.key === 'Home' ? items[0] : items[items.length - 1]); }
    else if (group && (event.key === 'ArrowRight' || event.key === 'ArrowLeft')) { event.preventDefault(); if (event.key === 'ArrowRight') ui.treeOpen.add(group); else ui.treeOpen.delete(group); const run = shownRun(); if (run) renderTree(run); }
    else if (event.key === 'Enter') { event.preventDefault(); current.firstChild.click(); if (!group) current.click(); }
  }

  // ---------- Plan
  function renderPlan(run, ch, anim) {
    const pane = $('run-pane-plan'), plan = run.plan, prompt = !$('prompt').disabled && run.finished;
    const key = JSON.stringify([plan?.eventId,run.finished,prompt,run.dropped,state.selected?.provider]); if (ui.paneKey.plan === key) return; ui.paneKey.plan = key;
    const before = new Set([...pane.querySelectorAll('.run-plan-text')].map(node => node.textContent)); pane.replaceChildren(); if (!plan) return;
    const provider = state.selected?.provider, done = plan.items.filter(item => item.status === 'done').length, name = providerName(provider);
    pane.append(el('p','run-pane-title',`${name}'s plan · ${done} of ${plan.total} checked by the agent`));
    if (!run.finished && plan.activeForm) pane.append(el('p','run-pane-line',`Now: ${plan.activeForm}`));
    else if (!run.finished && !plan.items.some(item => item.status === 'active')) { const next = plan.items.find(item => item.status === 'pending'); if (next) pane.append(el('p','run-pane-line',`Next unchecked: ${next.text}`)); }
    if (run.planChange) { const change = run.planChange; pane.append(el('p','run-pane-meta',`Plan changed at ${wall(change.at)}${change.added.length ? ` · +${change.added.length} added` : ''}${change.dropped.length ? ` · ${change.dropped.length} dropped` : ''}`)); }
    const list = el('ol','run-plan');
    for (const item of plan.items.slice(0,30)) {
      const li = el('li'), glyph = el('span','run-plan-glyph',PLAN_GLYPH[item.status]), body = el('span'); li.dataset.status = item.status; glyph.setAttribute('aria-hidden','true');
      body.append(el('span','sr-only',`${PLAN_WORD[item.status]}: `),el('span','run-plan-text',item.text)); li.append(glyph,body,el('span','run-plan-tag',plan.added.has(item.text) ? '+ added' : '')); list.append(li);
      if (anim && ch?.plan && before.size && !before.has(item.text)) motion(li,[{opacity:0,transform:'translateY(-4px)'},{opacity:1,transform:'none'}],{duration:ms('--motion-base'),easing:token('--ease-enter')});
    }
    pane.append(list);
    if (plan.total > Math.min(30,plan.items.length)) pane.append(el('p','run-pane-meta',`Showing the first ${Math.min(30,plan.items.length)} of ${plan.total} items`));
    if (run.dropped.length) {
      const details = el('details','run-plan-dropped'), dropped = el('ol','run-plan'); details.append(el('summary','',`Dropped (${run.dropped.length})`));
      for (const text of run.dropped) { const li = el('li'), glyph = el('span','run-plan-glyph','⊘'), body = el('span'); li.dataset.status = 'cancelled'; glyph.setAttribute('aria-hidden','true'); body.append(el('span','sr-only','dropped: '),el('span','run-plan-text',text)); li.append(glyph,body); dropped.append(li); }
      details.append(dropped); pane.append(details);
    }
    pane.append(el('p','run-caption',`Ticked by ${name}'s own ${PLAN_TOOL[provider] || 'plan'}. Harness does not verify them.`));
    const unchecked = plan.items.filter(item => item.status !== 'done' && item.status !== 'cancelled');
    if (prompt && unchecked.length) pane.append(button(`Continue with ${count(unchecked.length,'unchecked item')}`,'button compact run-pane-action',() => continuePlan(run)));
  }
  // Fills the draft only; Send stays the person's.
  function continuePlan(run) {
    const unchecked = run.plan.items.filter(item => item.status !== 'done' && item.status !== 'cancelled').map(item => `- ${item.text}`);
    insertDraft(`Continue with the plan items that are not checked yet:\n${unchecked.join('\n')}${run.dropped.length ? `\nDropped by the agent: ${run.dropped.join('; ')}.` : ''}`,true);
  }

  // ---------- Commands
  function lastLabel(row) {
    const last = row.last, provider = state.selected?.provider;
    if (last.glyph === 'run') return '● running';
    if (last.glyph === 'notrun') return '⊘ not run';
    if (last.cmd.state === 'unfinished') return '? no result recorded';
    if (row.masked) return `? unknown · ${row.masked}`;
    let text = Number.isInteger(last.cmd.exitCode) ? `exit ${last.cmd.exitCode}` : last.glyph === 'ok' ? provider === 'cursor' ? 'ran' : 'no error' : provider === 'cursor' ? 'reported failure' : 'error reported';
    if (row.green) text += ` · green after ${row.green.runs} runs${row.green.edits === null ? '' : ` · ${count(row.green.edits,'edit')} between`}`;
    return text;
  }
  function renderCommands(run, ch, anim) {
    const pane = $('run-pane-commands'), provider = state.selected?.provider, after = run.finished && !active(state.selected);
    const key = JSON.stringify([run.commands.map(cmd => [cmd.state,cmd.ok,cmd.outcome,cmd.exitCode]),run.counters.commands,run.counters.commandFailed,run.limited,ui.commandFilter,after,run.mode]); if (ui.paneKey.commands === key) return; ui.paneKey.commands = key;
    const before = new Set([...pane.querySelectorAll('tbody tr')].map(row => row.dataset.key)); pane.replaceChildren();
    if (run.mode !== 'targets') {
      const name = [...run.tools.keys()].find(tool => RunModel.kinds.COMMAND.has(tool.toLowerCase())) || 'Commands';
      pane.append(el('p','run-pane-title',provider === 'claude' ? `${name} ×${run.counters.commands} (details not recorded)` : `Commands ${run.counters.commands} · ${run.counters.commandFailed} failed (details not recorded)`),
        el('p','run-caption',provider === 'claude' ? "Claude Code's completions carry no tool name, so failures cannot be matched to commands." : 'Command text was not recorded for this run.'));
      return;
    }
    if (run.limited) pane.append(el('p','run-banner','Command record is incomplete for this launch.'));
    const rows = RunModel.checks(run); pane.append(el('h3','run-pane-sub','Verification'));
    if (rows.length) {
      const wrap = el('div','run-table-wrap'), table = el('table','run-table'), head = el('tr'), body = el('tbody'), thead = el('thead');
      for (const label of ['Check','Target','Runs','Last']) { const th = el('th',label === 'Target' ? 'run-target-col' : '',label); th.scope = 'col'; head.append(th); }
      thead.append(head);
      for (const row of rows) {
        const tr = el('tr'), th = el('th','',row.family), target = el('td','run-mono run-target-col'), runs = el('td','run-runs'), under = el('span','run-check-target'); tr.dataset.key = row.key; th.scope = 'row'; target.append(bdi(row.target));
        under.append(bdi(row.target)); th.append(under);
        row.runs.forEach((item,index) => {
          const glyph = button('','run-glyph-button',() => reveal(item.cmd.eventId)); glyph.append(glyphNode(item.glyph)); glyph.setAttribute('aria-label',`Run ${index + 1}: ${CHECK_WORD[item.glyph]}. Show in conversation`);
          glyph.title = item.glyph === 'fail' && provider === 'claude' ? 'Claude Code reported an error (non-zero exit, timeout or blocked — Harness cannot tell which)' : /test|PHPUnit|Pest|ParaTest/.test(row.family) ? `${CHECK_WORD[item.glyph]} · --filter that matches no tests can still exit 0` : CHECK_WORD[item.glyph];
          runs.append(glyph);
        });
        const last = el('td','',lastLabel(row)); tr.append(th,target,runs,last);
        // After the run the row's two actions sit under its last result, so a narrow window needs no extra column.
        if (after) { const actions = el('div','run-row-actions'); if (row.runnable) actions.append(button('Run in Harness','button compact',() => prefillCheck(row))); actions.append(button('Ask to re-run','button compact',() => askRerun(row))); last.append(actions); }
        body.append(tr);
        if (anim && before.size && !before.has(row.key)) motion(tr,[{opacity:0},{opacity:1}],{duration:ms('--motion-base'),easing:token('--ease-enter')});
      }
      table.append(thead,body); wrap.append(table); pane.append(wrap);
      for (const row of rows) {
        if (row.maxFails >= 2) pane.append(el('p','run-fact',`${row.family}: the same check failed ${row.maxFails} times in a row${row.green ? ', then passed' : ''}.`));
      }
    } else pane.append(el('p','run-pane-meta','No PHP checks recognised in this run.'));
    const others = run.commands.filter(cmd => !cmd.cls.check);
    pane.append(el('h3','run-pane-sub','Other commands'));
    const filters = el('div','run-seg'); filters.setAttribute('role','group'); filters.setAttribute('aria-label','Filter commands');
    for (const [id,label] of [['all','All'],['failed','Failed'],['notrun','Not run']]) { const node = button(label,'',() => { ui.commandFilter = id; render(null); }); node.setAttribute('aria-pressed',String(ui.commandFilter === id)); filters.append(node); }
    pane.append(filters);
    const shown = others.filter(cmd => ui.commandFilter === 'all' || ui.commandFilter === 'failed' && cmd.ok === false && cmd.outcome !== 'not_run' || ui.commandFilter === 'notrun' && cmd.outcome === 'not_run').slice(-200);
    const list = el('ol','run-commands');
    for (const cmd of shown) {
      const glyph = cmd.state === 'running' ? 'run' : cmd.state === 'unfinished' || cmd.cls.masked ? 'unknown' : cmd.outcome === 'not_run' ? 'notrun' : cmd.ok ? 'ok' : 'fail', li = el('li');
      const go = button('','run-glyph-button',() => reveal(cmd.eventId)); go.append(glyphNode(glyph)); go.setAttribute('aria-label',`${CHECK_WORD[glyph]}. Show in conversation`);
      const code = el('code','run-mono'); code.append(bdi(cmd.command || '(command not recorded)'));
      const at = cmd.startAt !== null && run.startAt !== null ? clock((cmd.startAt - run.startAt) / 1000) : '';
      li.append(go,code,el('span','run-command-meta',[cmd.cls.fixer ? '✎ changed files · not a check' : '',Number.isInteger(cmd.exitCode) ? `exit ${cmd.exitCode}` : '',at].filter(Boolean).join(' · ')));
      list.append(li);
    }
    if (!shown.length) list.append(el('li','run-pane-meta',ui.commandFilter === 'all' ? 'No other commands yet.' : 'None in this list.'));
    pane.append(list);
    if (provider === 'codex') pane.append(el('p','run-caption',"Helper agents' commands may not be shown."));
    pane.append(el('p','run-caption',"Exit status means 'finished without error' — not which tests ran, how many, or whether they cover the change. Output is never sent to Harness."));
  }
  // Opens Checks with the command filled in; nothing runs until the person presses Run check.
  function prefillCheck(row) {
    if (!row.runnable || active(state.selected)) return;
    openView('checks'); $('check-command').value = row.bare;
    $('check-prefill-note').textContent = `${row.removed.length ? `Removed from the agent's command: ${row.removed.join(', ')}. ` : ''}Filled from this run. Review it, then press Run check; nothing runs until you do.`;
    $('check-prefill-note').hidden = false; $('check-command').dispatchEvent(new Event('input')); $('check-command').focus();
  }
  function askRerun(row) {
    const command = row.last.cmd.command || row.bare;
    insertDraft(`Re-run \`${row.masked ? row.bare : command}\`${row.masked ? ` without ${row.removed.filter(item => /^[|;&]/.test(item)).join(' or ') || 'the wrapper'}` : ''} and report its exit status.`,true);
  }

  // ---------- Run receipt
  const launchOf = run => (ui.data?.launches || []).find(launch => launch.id === run.id) || null;
  function ledgerOf(launch) { const ledger = launch?.receipt; return ledger && typeof ledger === 'object' && !Array.isArray(ledger) ? ledger : null; }
  function secondsOf(run, launch, latest) {
    const usage = launch?.usage || (latest ? state.selected?.budget_usage : null);
    if (Number.isFinite(usage?.seconds) && usage.seconds > 0) return {seconds:usage.seconds,approx:false};
    const from = Date.parse(launch?.started_at || ''), to = Date.parse(launch?.finished_at || '');
    if (Number.isFinite(from) && Number.isFinite(to)) return {seconds:(to - from) / 1000,approx:false};
    return run.startAt !== null && run.endAt !== null ? {seconds:(run.endAt - run.startAt) / 1000,approx:run.approx} : null;
  }
  function stamp(run, launch, latest, edited) {
    const span = secondsOf(run,launch,latest), time = span ? `${span.approx ? '≈ ' : ''}${duration(span.seconds)}` : '—', usage = launch?.usage || (latest ? state.selected?.budget_usage : null);
    const settings = launch?.settings || state.selected || {}, provider = providerName(settings.provider), mode = settings.sdd?.phase ? `SDD ${settings.sdd.phase}` : settings.mode;
    if (usage?.limit_reached) return {glyph:'⏱',text:`STOPPED AT ${usage.limit_reached === 'USD' ? 'USD' : String(usage.limit_reached).toUpperCase()} LIMIT · ${time}`,word:'Stopped at a limit',action:'budgets'};
    if (run.outcome === 'completed') return {glyph:'✓',text:`PROCESS COMPLETE · ${time} · ${provider} · ${settings.model || 'default model'}${mode ? ` · ${mode}` : ''}`,word:'Process complete'};
    if (run.outcome === 'cancelled') return {glyph:'■',text:`CANCELLED at ${time}${edited ? ` · ${count(edited,'file')} edited so far` : ''}`,word:'Cancelled',action:'changes'};
    if (run.outcome === 'interrupted') return {glyph:'!',text:"INTERRUPTED · Runner restarted; this run's last records may be incomplete",word:'Interrupted'};
    return {glyph:'✕',text:`FAILED · ${time} · ${run.aborted ? 'The run stopped with an error' : 'Provider did not complete successfully'}`,word:'Failed',action:'error'};
  }
  function receiptFacts(run, latest) {
    const launch = launchOf(run), ledger = ledgerOf(launch), paths = [...run.paths.values()], ge = run.limited && !ledger ? '≥' : '';
    const opened = ledger && Number.isInteger(ledger.opened) ? {n:ledger.opened,list:(ledger.opened_paths || []).filter(item => typeof item === 'string')} : {n:paths.filter(p => p.opened).length,list:paths.filter(p => p.opened).map(p => p.path)};
    const edited = ledger && Number.isInteger(ledger.edited) ? {n:ledger.edited,list:(ledger.edited_paths || []).filter(item => typeof item === 'string')} : {n:paths.filter(p => p.edited).length,list:paths.filter(p => p.edited).map(p => `${p.path}${p.created ? ' · created' : ''}${p.deleted ? ' · deleted' : ''}${p.edits > 1 ? ` · ×${p.edits}` : ''}`)};
    const patterns = ledger && Array.isArray(ledger.patterns) ? {n:Number.isInteger(ledger.searched) ? ledger.searched : ledger.patterns.length,list:ledger.patterns.filter(item => typeof item === 'string').map(RunModel.clean)} : {n:run.searches.length,list:run.searches.map(item => item.pattern).filter(Boolean)};
    let commands = run.commands;
    if (ledger && Array.isArray(ledger.last_commands) && !commands.length) commands = ledger.last_commands.filter(item => item && typeof item.command === 'string').map(item => ({command:RunModel.clean(item.command),cls:RunModel.classifyCommand(item.command),state:'done',ok:item.ok !== false,outcome:item.outcome === 'not_run' ? 'not_run' : null,exitCode:Number.isInteger(item.exit_code) ? item.exit_code : null,startAt:null,endAt:null,eventId:null}));
    const ran = ledger && Number.isInteger(ledger.commands) ? {n:ledger.commands,failed:ledger.failed || 0,notRun:ledger.not_run || 0} : {n:run.counters.commands,failed:run.counters.commandFailed,notRun:commands.filter(cmd => cmd.outcome === 'not_run').length};
    const plan = run.plan ? {done:run.plan.items.filter(item => item.status === 'done').length,total:run.plan.total,next:run.plan.items.find(item => item.status !== 'done')?.text} : ledger?.plan && Number.isInteger(ledger.plan.total) ? {done:ledger.plan.done,total:ledger.plan.total} : null;
    const labels = run.mode !== 'targets' && !ledger;
    return {launch,ledger,ge,opened,edited,patterns,commands,ran,plan,labels,checks:RunModel.checks({commands,editLog:run.editLog}),stamp:stamp(run,launch,latest,edited.n)};
  }
  function receipt(run) {
    const node = el('article','run-receipt'); node.dataset.run = run.id; node.setAttribute('aria-live','off');
    ui.receipts.set(run.id,{node,key:''}); renderReceipts(); return node;
  }
  // Only a receipt the person watched arrive rises into place; earlier ones and history appear as they are.
  function riseReceipt(runId) {
    const node = ui.receipts.get(runId)?.node; if (!node?.isConnected) return;
    motion(node,[{opacity:0,transform:'translateY(4px)'},{opacity:1,transform:'none'}],{duration:ms('--motion-base'),easing:token('--ease-enter')});
    [...node.querySelectorAll('.run-receipt-facts > dt')].slice(0,10).forEach((dt,index) => { for (const item of [dt,dt.nextElementSibling]) motion(item,[{opacity:0},{opacity:1}],{duration:ms('--motion-base'),easing:token('--ease-enter'),delay:index * ms('--motion-stagger'),fill:'backwards'}); });
  }
  function renderReceipts() {
    const finished = ui.model.order.map(id => ui.model.runs.get(id)).filter(run => run.finished && ui.receipts.has(run.id));
    const newest = ui.model.order.map(id => ui.model.runs.get(id)).filter(run => RunModel.native(run)).pop(), latestId = newest?.finished ? newest.id : null;
    for (const run of finished) {
      // A Harness check leaves the receipt as it is; only the next turn folds it into one line.
      const entry = ui.receipts.get(run.id), latest = run.id === latestId && !nextTurn();
      const key = JSON.stringify([latest,active(state.selected),run.lastEventId,run.memory?.text,ui.data ? ui.data.launches?.length : null,ui.data?.checks?.[0]?.status,ui.data?.snapshot?.id,state.selected?.budget_usage,ui.ended?.sid === ui.sid,ui.away === null && ui.awayLine?.runId === run.id ? ui.awayLine.text : null]);
      if (entry.key !== key) { entry.key = key; buildReceipt(entry.node,run,latest); }
    }
  }
  function buildReceipt(node, run, latest) {
    const facts = receiptFacts(run,latest), {launch,ledger,ge,stamp:mark} = facts, session = state.selected, provider = launch?.settings?.provider || session?.provider;
    const titleId = `run-receipt-${run.id}`, title = el('h3','run-receipt-title',`Run receipt · Turn ${run.ordinal}`); title.id = titleId; node.setAttribute('aria-labelledby',titleId);
    const dl = el('dl','run-receipt-facts');
    const row = (label, ...content) => { const dd = el('dd'); for (const item of content) if (item !== null && item !== undefined && item !== '') dd.append(typeof item === 'string' ? document.createTextNode(item) : item); dl.append(el('dt','',label),dd); return dd; };
    const note = text => el('span','run-note',text);
    const list = (summary, items, total) => { if (!items.length) return null; const details = el('details','run-receipt-list'), ul = el('ul'); details.append(el('summary','',summary)); for (const item of items.slice(0,40)) { const li = el('li'); li.append(bdi(item)); ul.append(li); } if (total > Math.min(40,items.length)) ul.append(el('li','run-note',`+${fmt.exact(total - Math.min(40,items.length)).text} more`)); details.append(ul); return details; };
    const show = (label, tab) => active(session) ? null : button(label,'run-link',() => open(run.id,tab));
    if (facts.labels) {
      row('Activity','Activity details were not recorded for this run.');
      row('Steps',`${count(run.counters.steps,'step')}${run.tools.size ? ` · ${RunModel.tally(run.tools)}` : ''}${run.counters.failed ? ` · ${count(run.counters.failed,'failed step')}` : ''}`);
    } else {
      if (provider === 'codex') row('Opened','shell reads not listed',note('Codex reads through shell commands'));
      else row('Opened',`${ge}${count(facts.opened.n,'file')}`,note('observed by Harness'),run.paths.size ? show('Show files','files') : null,list('Opened files',facts.opened.list,facts.opened.n));
      if (facts.patterns.n) row('Searched',`${count(facts.patterns.n,'pattern')}`,facts.patterns.list.length ? ` · ${facts.patterns.list.slice(0,2).join(' · ')}${facts.patterns.n > 2 ? ` · +${facts.patterns.n - 2}` : ''}` : '');
      row('Edited',`${ge}${count(facts.edited.n,'file')} (reported by tools)`,list('Edited files',facts.edited.list,facts.edited.n));
      row('Ran',`${ge}${count(facts.ran.n,'command')}${facts.ran.failed ? ` · ${facts.ran.failed} failed` : ''}${facts.ran.notRun ? ` · ${facts.ran.notRun} not run` : ''}`,run.counters.commands ? show('Show commands','commands') : null,
        list(ledger && !run.commands.length ? 'Last commands' : 'Commands',facts.commands.map(cmd => `${cmd.outcome === 'not_run' ? '⊘' : cmd.ok === false ? '✗' : cmd.ok ? '✓' : '?'} ${cmd.command}`),facts.commands.length));
      if (facts.checks.length) {
        const dd = row('Checks');
        facts.checks.forEach((check,index) => { if (index) dd.append(document.createTextNode(' · ')); dd.append(document.createTextNode(`${check.family} `)); check.runs.forEach((item,at) => { if (at) dd.append(document.createTextNode(' ')); dd.append(glyphNode(item.glyph)); }); if (check.masked) dd.append(document.createTextNode(` (${check.masked.split(':')[0]})`)); dd.append(el('span','sr-only',` ${check.runs.map(item => CHECK_WORD[item.glyph]).join(', ')}`)); });
      }
    }
    if (facts.plan) row('Plan',`${facts.plan.done} of ${facts.plan.total} marked done by the agent`,facts.plan.next ? ` → ${facts.plan.next}` : '',run.plan ? show('Show plan','plan') : null);
    else if (!facts.labels) row('Plan','No plan recorded');
    if (run.delegation) row('Helpers',run.delegation.text || (run.delegation.status === 'confirmed' ? 'Confirmed' : 'Not confirmed'));
    const usage = launch?.usage || (latest ? session?.budget_usage : null), cap = launch?.settings?.budgets?.usd ?? (latest ? session?.budgets?.usd : null);
    const tokens = Number.isFinite(usage?.tokens) ? `${fmt.exact(usage.tokens).text} tokens reported` : 'Tokens —';
    const cost = provider === 'codex' ? 'Cost: — not reported by Codex' : Number.isFinite(usage?.cost_usd) ? `Cost ${fmt.cost(usage.cost_usd).text}${Number.isFinite(cap) ? ` of $${cap.toFixed(2)} cap` : ''}` : 'Cost —';
    row('Tokens & cost',`${tokens} · ${cost}`,note(`reported by ${providerName(provider)}`));
    const fill = launch?.context?.fill;
    if (provider === 'cursor') row('Context','not reported');
    else if (fill && Number.isFinite(fill.end) && Number.isFinite(fill.window)) { const times = Array.isArray(fill.compactions) ? fill.compactions.length : 0; row('Context',`${compactNumber(fill.end)} of ${compactNumber(fill.window)} at last call${times ? ` · compacted ${times === 1 ? 'once' : `${times} times`}` : ''}`); }
    if (run.memory) row('Memory',run.memory.text);
    const moments = run.moments.filter(item => item.kind !== 'result');
    if (moments.length && !facts.labels) {
      const dd = row('Jump to'), box = el('div','run-moments'); dd.append(box);
      for (const item of moments) box.append(button(`${item.label}${item.at !== null && run.startAt !== null ? ` ${clock((item.at - run.startAt) / 1000)}` : ''}`,'run-moment',() => reveal(item.eventId)));
    }
    if (run.limited && ledger) row('Note',`The file map stopped at step ${fmt.exact(run.limited.step).text}; the counts above come from the full run record.`);
    else if (run.limited) row('Note',`Step details stopped at step ${fmt.exact(run.limited.step).text}; counts marked ≥ are a lower bound.`);
    if (run.aborted) row('Note','Counts cover the run until it stopped.');
    const head = el('div','run-receipt-stamp'); head.append(el('span','run-receipt-mark',mark.glyph),document.createTextNode(` ${mark.text}`));
    const sub = el('p','run-receipt-sub','Not an independent verification of the task.');
    if (!latest) {
      const span = secondsOf(run,launch,false), details = el('details','run-receipt-line');
      details.append(el('summary','',[`Turn ${run.ordinal}`,mark.word,span ? `${span.approx ? '≈ ' : ''}${duration(span.seconds)}` : '',facts.labels ? '' : `${ge}${facts.edited.n} edited`,Number.isFinite(usage?.cost_usd) && provider !== 'codex' ? fmt.cost(usage.cost_usd).text : ''].filter(Boolean).join(' · ')),head,sub,dl);
      node.dataset.compact = 'true'; node.replaceChildren(details); return;
    }
    delete node.dataset.compact;
    const top = el('div','run-receipt-head'), copy = button('Copy summary','button compact',async () => { copy.textContent = await copyText(receiptText(node)) ? 'Copied' : 'Copy failed'; });
    top.append(title,copy);
    const body = el('div','run-receipt-body'); body.append(head);
    if (mark.action === 'error' && run.error) body.append(button('Show error','run-link',() => reveal(run.error.eventId)));
    if (mark.action === 'budgets') body.append(button('Edit budgets','run-link',editBudgets));
    if (mark.action === 'changes') body.append(button('Review changes','run-link',() => openView('changes')));
    body.append(sub);
    if (ui.awayLine?.runId === run.id) body.append(el('p','run-receipt-away',ui.awayLine.text));
    body.append(dl);
    node.replaceChildren(top,body,yourMove(run,facts));
  }
  function yourMove(run, facts) {
    const move = el('section','run-receipt-move'), session = state.selected; move.setAttribute('aria-label','Your move'); move.append(el('h4','run-receipt-move-title','Your move'));
    const line = (label, value, action) => { const row = el('div','run-move'); row.append(el('span','run-move-label',label)); const cell = el('div','run-move-value'); cell.append(...value); row.append(cell,action || el('span')); move.append(row); return cell; };
    const snapshot = ui.data?.snapshot;
    if (snapshot?.available && Array.isArray(snapshot.files)) {
      const names = snapshot.files.map(file => RunModel.clean(file.path)), value = [document.createTextNode(`${count(names.length,'file')} vs session baseline${snapshot.complete === false ? ' (list may be incomplete)' : ''}`)];
      // Paths every turn's tools reported editing; a gap in that record (labels only, or a list cut at 40) leaves the comparison out.
      const known = new Set(), runs = ui.model.order.map(id => ui.model.runs.get(id)).filter(RunModel.native); let complete = true;
      for (const item of runs) {
        const ledger = ledgerOf(launchOf(item));
        // The ledger lists project paths up to 40; a shorter list is the whole list.
        if (ledger && Array.isArray(ledger.edited_paths) && ledger.edited_paths.length < 40) ledger.edited_paths.forEach(path => known.add(path));
        else if (item.mode === 'targets' && !item.limited) for (const p of item.paths.values()) { if (p.edited && !p.outside) known.add(p.path); }
        // A turn recorded by tool names alone edited nothing through edit tools when none of them ran.
        else if (item.mode === 'targets' || [...item.tools.keys()].some(name => RunModel.kinds.EDIT.has(name.toLowerCase()))) complete = false;
      }
      const outside = complete ? names.filter(name => !known.has(name)) : [];
      if (outside.length) { const sub = el('span','run-move-sub',`${outside.length} changed outside edit tools: `); sub.append(bdi(outside.slice(0,6).join(', ')),document.createTextNode(`${outside.length > 6 ? ` +${outside.length - 6}` : ''} (shell, generator, formatter or IDE; Harness cannot tell who)`)); value.push(sub); }
      line('Workspace diff',value,button('Review changes','button compact',() => openView('changes')));
    } else if (snapshot && snapshot.message) line('Workspace diff',[document.createTextNode(RunModel.clean(snapshot.message))],button('Review changes','button compact',() => openView('changes')));
    else line('Workspace diff',[document.createTextNode(ui.data ? 'Not available for this session' : 'Reading the workspace…')],button('Review changes','button compact',() => openView('changes')));
    // The launch row may settle after the results were read; the closing event's time stands in for it.
    const check = (ui.data?.checks || []).find(item => item.kind !== 'target'), finished = Date.parse(facts.launch?.finished_at || '') || run.endAt, ran = Date.parse(check?.started_at || check?.created_at || '');
    const verify = el('span','run-verify'); let text = 'Not checked since this turn';
    if (check && Number.isFinite(finished) && Number.isFinite(ran) && ran >= finished - 1000) {
      const command = RunModel.clean(check.command);
      if (['queued','running'].includes(check.status)) text = `${check.status.toUpperCase()} · ${command}`;
      else { text = `${String(check.status).toUpperCase()} · ${command} · exit ${check.exit_code ?? 'unavailable'} · ran after this turn`; verify.dataset.passed = String(check.status === 'passed'); }
    }
    verify.textContent = text;
    line('Verification',[verify],checkMenu(facts.checks));
    const actions = el('div','run-move-actions');
    const unchecked = run.plan ? run.plan.items.filter(item => item.status !== 'done' && item.status !== 'cancelled') : [];
    if (unchecked.length && !$('prompt').disabled) actions.append(button(`Continue with ${count(unchecked.length,'unchecked item')}`,'button compact',() => continuePlan(run)));
    actions.append(button('Usage details','button compact',() => openView('usage')));
    if (session?.workspace === 'worktree') actions.append(button('Deliver…','button compact',() => openView('changes')));
    move.append(actions); return move;
  }
  // "Run a check" lists the run's own checks that Harness can run as one command, and an empty form.
  function checkMenu(rows) {
    const host = el('span','run-menu-host'), toggle = button('Run a check ▾','button compact',() => { const open = menu.hidden; menu.hidden = !open; toggle.setAttribute('aria-expanded',String(open)); if (open) menu.querySelector('[role="menuitem"]')?.focus(); });
    const menu = el('div','run-menu'); menu.setAttribute('role','menu'); menu.hidden = true; toggle.setAttribute('aria-haspopup','menu'); toggle.setAttribute('aria-expanded','false');
    const close = () => { menu.hidden = true; toggle.setAttribute('aria-expanded','false'); };
    for (const row of rows.filter(item => item.runnable)) {
      const item = button('','run-menu-item',() => { close(); prefillCheck(row); }); item.setAttribute('role','menuitem'); item.append(bdi(row.bare,'run-mono'));
      if (row.removed.length) item.append(el('span','run-menu-note',`${row.removed.some(part => part.startsWith('|')) ? 'Pipe removed' : 'Wrapper removed'} — review before running`));
      menu.append(item);
    }
    const own = button('Write my own','run-menu-item',() => { close(); openView('checks'); $('check-command').focus(); }); own.setAttribute('role','menuitem'); menu.append(own);
    menu.addEventListener('keydown',event => {
      const items = [...menu.querySelectorAll('[role="menuitem"]')], index = items.indexOf(document.activeElement);
      if (event.key === 'Escape') { event.stopPropagation(); close(); toggle.focus(); }
      else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); items[(index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length].focus(); }
    });
    menu.addEventListener('focusout',event => { if (!host.contains(event.relatedTarget)) close(); });
    host.append(toggle,menu); return host;
  }
  function receiptText(node) {
    const lines = [node.querySelector('.run-receipt-stamp')?.textContent.trim(),node.querySelector('.run-receipt-sub')?.textContent];
    for (const dt of node.querySelectorAll('.run-receipt-facts > dt')) {
      if (dt.textContent === 'Jump to') continue;
      const copy = dt.nextElementSibling.cloneNode(true); copy.querySelectorAll('details,button,.sr-only').forEach(item => item.remove());
      lines.push(`${dt.textContent}: ${copy.textContent.replace(/\s+/g,' ').trim()}`);
    }
    const verify = node.querySelector('.run-verify'); if (verify) lines.push(`Verification: ${verify.textContent}`);
    return lines.filter(Boolean).join('\n');
  }
  function editBudgets() {
    sessionOptions.open = 'budgets'; $('session-settings-toggle').setAttribute('aria-expanded','true'); renderSessionSummary(); renderSessionOptions();
    $('panel-budgets').tabIndex = -1; $('panel-budgets').focus();
  }
  // Results feed the receipts: per-launch usage, the server's run ledger, checks and the workspace's changed names.
  async function loadData() {
    const sid = ui.sid, epoch = ui.epoch; if (!sid || ui.dataPending) return; ui.dataPending = true; ui.dataStale = false;
    try {
      let data;
      try { data = await api(`/api/sessions/${encodeURIComponent(sid)}/results?diff=names`); }
      catch (error) { if (error.status !== 400) throw error; data = await api(`/api/sessions/${encodeURIComponent(sid)}/results?diff=0`); }
      if (epoch !== ui.epoch) return; ui.data = data; renderReceipts();
    } catch (_) { /* The receipt keeps what the events say; Refresh results shows the error. */ }
    finally { if (epoch === ui.epoch) ui.dataPending = false; }
  }

  // ---------- Status lines the log draws differently: the receipt, compaction and the end of step details
  function statusNode(event) {
    const run = typeof event.launch_id === 'string' ? ui.model.runs.get(event.launch_id) : null;
    if (event.compaction && typeof event.compaction === 'object') {
      const {pre,post} = event.compaction;
      // Only the sizes the CLI reported, as the server writes it; it may report the size before without the size after.
      const known = value => Number.isFinite(value), size = known(pre) && known(post) ? ` · ${fmt.exact(pre).text} → ${fmt.exact(post).text} tokens`
        : known(pre) ? ` at ${fmt.exact(pre).text} tokens` : known(post) ? ` to ${fmt.exact(post).text} tokens` : '';
      return divider(typeof event.text === 'string' && event.text ? event.text : `Context compacted${size}. The CLI replaced earlier conversation with a summary.`,'run-compaction',event.id,true);
    }
    // The announcer says this milestone once; the log's copy is a separator it does not read out.
    if (event.targets === 'limited') return quiet(divider('Step details are not recorded after this point; steps are still counted.','run-limited',event.id,false));
    const session = state.selected;
    if (run?.finished && run.closingEventId === event.id && RunModel.native(run) && !isFleetSession(session) && !session?.creator && !session?.system_run && !session?.system_discovery) {
      const node = receipt(run); node.dataset.eventId = event.id;
      if (ui.loaded) ui.dataStale = true; return node;
    }
    return null;
  }
  // A divider the polite log must not announce: its text is hidden from the live region, its separator role keeps a name.
  function quiet(node) { node.setAttribute('role','separator'); node.setAttribute('aria-label',node.textContent); node.firstChild.setAttribute('aria-hidden','true'); return node; }
  function divider(text, className, eventId, appear) {
    const node = el('p',`run-divider ${className}`); node.dataset.eventId = eventId; node.append(el('span','',RunModel.clean(text)));
    if (appear && ui.live) requestAnimationFrame(() => motion(node,[{opacity:0},{opacity:1}],{duration:ms('--motion-fast'),easing:token('--ease-enter')}));
    return node;
  }

  // ---------- Conversation helpers
  function reveal(eventId) {
    const id = Number(eventId); if (!Number.isFinite(id)) return;
    let node = null; for (const item of $('events').querySelectorAll('[data-event-id]')) { if (Number(item.dataset.eventId) <= id) node = item; else if (node) break; }
    if (!node) return;
    for (let parent = node.parentElement; parent && parent.id !== 'events'; parent = parent.parentElement) if (parent.tagName === 'DETAILS') parent.open = true;
    const conversation = $('conversation'), box = conversation.getBoundingClientRect(), rect = node.getBoundingClientRect();
    conversation.scrollTo({top:conversation.scrollTop + rect.top - box.top - Math.max(16,(box.height - rect.height) / 2),behavior:scrollMotion()});
    node.classList.add('run-flash'); setTimeout(() => node.classList.remove('run-flash'),1000);
  }
  function firstFailed() { const run = shownRun(); if (run && run.firstFailedEventId !== null) reveal(run.firstFailedEventId); }
  function insertDraft(text, block = false) {
    const prompt = $('prompt'); if (prompt.disabled) return false;
    const start = prompt.selectionStart ?? prompt.value.length, end = prompt.selectionEnd ?? start, before = prompt.value.slice(0,start), after = prompt.value.slice(end);
    const lead = before && !/\s$/.test(before) ? block ? '\n\n' : ' ' : '', trail = after && !/^\s/.test(after) ? ' ' : '';
    prompt.value = before + lead + text + trail + after; const at = (before + lead + text).length; prompt.focus(); prompt.setSelectionRange(at,at);
    prompt.dispatchEvent(new Event('input')); return true;
  }
  async function copyText(text) {
    try { await navigator.clipboard.writeText(text); return true; } catch (_) {
      const area = el('textarea'); area.value = text; area.className = 'sr-only'; document.body.append(area); area.select();
      let ok = false; try { ok = document.execCommand('copy'); } catch (_) { ok = false; } area.remove(); return ok;
    }
  }
  function open(runId, tab) { if (phase()) return; ui.showAfter = runId; ui.mode = modeOf(state.selected); setDetailed(true,tab); }
  function close() { ui.showAfter = null; ui.mode = modeOf(state.selected); hideCard(); render(null); $('prompt').focus({preventScroll:true}); }

  // ---------- Away Watch: title, icon, notifications, the return line and "New since"
  function titlePrefix(session) {
    if (!session) return '';
    const p = phase(session);
    if (p === 'running' || p === 'preparing') return '● Running · ';
    if (p === 'queued') return 'Queued · ';
    if (p === 'check') return 'Checking · ';
    if (session.status === 'awaiting_approval') return '⚑ Needs approval · ';
    if (session.status === 'awaiting_context') return '⚑ Review context · ';
    if (ui.ended?.sid !== session.id) return '';
    return ui.ended.limit === 'time' ? '✕ Time limit · ' : {completed:'✓ Finished · ',failed:'✕ Failed · ',cancelled:'■ Cancelled · ',interrupted:'✕ Interrupted · '}[ui.ended.outcome] || '';
  }
  // The tab shows the application's hare - the icon the application list shows - and a run's state as a badge
  // drawn over it from the theme's own tokens: a dot while running, ! when Harness needs you, ✓ or × at the end.
  const appIcon = {href:document.querySelector('link[rel="icon"]')?.getAttribute('href') || '', image:null};
  if (appIcon.href) { const image = new Image(); image.onload = () => { appIcon.image = image; ui.iconKey = ''; icon(state.selected); }; image.src = appIcon.href; }
  function badge(draw, kind) {
    const fill = {running:'#ffffff',needs:token('--warning-dot'),done:token('--success-dot'),failed:token('--error-dot')}[kind];
    draw.fillStyle = fill; draw.beginPath(); draw.arc(47,47,16,0,Math.PI * 2); draw.fill();
    draw.strokeStyle = '#ffffff'; draw.fillStyle = '#ffffff'; draw.lineWidth = 4.5;
    if (kind === 'running') { draw.fillStyle = token('--purple'); draw.beginPath(); draw.arc(47,47,8,0,Math.PI * 2); draw.fill(); }
    else if (kind === 'needs') { draw.beginPath(); draw.moveTo(47,38); draw.lineTo(47,48); draw.stroke(); draw.beginPath(); draw.arc(47,55,2.6,0,Math.PI * 2); draw.fill(); }
    else if (kind === 'done') { draw.beginPath(); draw.moveTo(40,47); draw.lineTo(45,52); draw.lineTo(54,42); draw.stroke(); }
    else { draw.beginPath(); draw.moveTo(41,41); draw.lineTo(53,53); draw.moveTo(53,41); draw.lineTo(41,53); draw.stroke(); }
  }
  function icon(session) {
    const p = phase(session), kind = p ? 'running' : ['awaiting_approval','awaiting_context'].includes(session?.status) ? 'needs' : session && ui.ended?.sid === session.id ? ui.ended.outcome === 'completed' && !ui.ended.limit ? 'done' : ui.ended.outcome === 'cancelled' ? 'idle' : 'failed' : 'idle';
    const key = `${kind}|${document.documentElement.dataset.theme}|${Boolean(appIcon.image)}`; if (key === ui.iconKey) return; ui.iconKey = key;
    let link = document.querySelector('link[rel="icon"]'); if (!link) { link = document.createElement('link'); link.rel = 'icon'; document.head.append(link); }
    if (kind === 'idle' && appIcon.href) { link.href = appIcon.href; return; }
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 64; const draw = canvas.getContext?.('2d'); if (!draw) return;
    draw.lineCap = 'round'; draw.lineJoin = 'round';
    if (appIcon.image) { draw.drawImage(appIcon.image,0,0,64,64); badge(draw,kind); link.href = canvas.toDataURL('image/png'); return; }
    draw.lineWidth = 6;
    const ring = color => { draw.strokeStyle = color; draw.beginPath(); draw.arc(32,32,25,0,Math.PI * 2); draw.stroke(); };
    if (kind === 'running') { draw.fillStyle = token('--purple'); draw.beginPath(); draw.arc(32,32,20,0,Math.PI * 2); draw.fill(); }
    else if (kind === 'needs') { ring(token('--warning-dot')); draw.beginPath(); draw.moveTo(32,18); draw.lineTo(32,36); draw.stroke(); draw.fillStyle = token('--warning-dot'); draw.beginPath(); draw.arc(32,45,4,0,Math.PI * 2); draw.fill(); }
    else if (kind === 'done') { ring(token('--success-dot')); draw.beginPath(); draw.moveTo(20,33); draw.lineTo(28,41); draw.lineTo(44,24); draw.stroke(); }
    else if (kind === 'failed') { ring(token('--error-dot')); draw.beginPath(); draw.moveTo(23,23); draw.lineTo(41,41); draw.moveTo(41,23); draw.lineTo(23,41); draw.stroke(); }
    else { draw.strokeStyle = token('--purple'); draw.lineWidth = 5; draw.beginPath(); for (let i = 0; i < 6; i++) { const angle = Math.PI / 3 * i - Math.PI / 2; draw[i ? 'lineTo' : 'moveTo'](32 + 24 * Math.cos(angle),32 + 24 * Math.sin(angle)); } draw.closePath(); draw.stroke(); }
    link.href = canvas.toDataURL('image/png');
  }
  // Private by default: no session title, path, command or agent text. Only on the person's opt-in, and only when the tab was away.
  function notify(title, body, tag) {
    if (!('Notification' in window) || Notification.permission !== 'granted' || !document.hidden || ui.hiddenAt === null || Date.now() - ui.hiddenAt < 20000) return;
    try { const note = new Notification(title,{body,tag,silent:true}); note.onclick = () => { window.focus(); note.close(); }; } catch (_) { /* Some browsers allow notifications only from a service worker. */ }
  }
  async function askPermission() {
    if (!('Notification' in window)) return;
    const result = await Notification.requestPermission(); render(null);
    if (result === 'granted') try { new Notification('Harness notifications are on',{body:'You will hear when a run you are watching finishes while this tab is in the background.',tag:'harness-notifications',silent:true}); } catch (_) { /* Shown by the browser only where allowed. */ }
  }
  function transitions(session, p) {
    const was = ui.watch, launch = session?.launch;
    ui.watch = session ? {sid:session.id,phase:p,status:session.status,launch:launch ? {id:launch.id,status:launch.status} : null} : null;
    if (!session || !was || was.sid !== session.id) return;
    // The session settles before its launch row does, and polling stops once it is idle: the end is the session going idle.
    const ended = !p && (was.launch ? was.launch.status === 'running' : was.phase === 'running');
    if (ended) {
      const usage = session.budget_usage || {}, outcome = session.status === 'completed' ? 'completed' : ['failed','cancelled','interrupted'].includes(session.status) ? session.status : 'failed';
      ui.ended = {sid:session.id,outcome,limit:usage.limit_reached || null}; ui.dataStale = true; ui.iconKey = '';
      const minutes = Number.isFinite(usage.seconds) ? `${Math.max(1,Math.round(usage.seconds / 60))} min` : '', cost = session.provider !== 'codex' && Number.isFinite(usage.cost_usd) ? fmt.cost(usage.cost_usd).text : '';
      const facts = [minutes,cost].filter(Boolean).join(' · ');
      if (outcome !== 'cancelled') notify(usage.limit_reached ? `Run stopped at its ${usage.limit_reached} limit` : outcome === 'completed' ? 'Run finished' : 'Run failed',
        `${facts ? `${facts} — ` : ''}${outcome === 'completed' ? 'not independently verified. ' : ''}Open Harness to review.`,`${session.id}:${outcome}`);
    }
    if (was.status !== session.status && ['awaiting_approval','awaiting_context'].includes(session.status)) notify(session.status === 'awaiting_approval' ? 'A review report awaits your decision' : 'Context is ready for your review','Open Harness to continue.',`${session.id}:${session.status}`);
  }
  function connection(ok) {
    if (ok) { ui.lostAt = null; ui.lostNotified = false; return; }
    ui.lostAt ??= Date.now();
    if (!ui.lostNotified && Date.now() - ui.lostAt >= 60000 && document.hidden) { ui.lostNotified = true; notify('Harness lost its local runner','Polling has failed for a minute. Open Harness to reconnect.','harness-connection'); }
  }
  // The last event this browser showed per session (at most 200 sessions), so "New since" also works after a reload.
  function rememberSeen(force = false) {
    if (!ui.sid || !force && Date.now() - ui.seenWritten < 5000) return;
    const last = [...state.eventIds].reduce((max,id) => Math.max(max,Number(id) || 0),0); if (!last) return;
    const seen = storage.read(SEEN); ui.seenWritten = Date.now(); if (seen[ui.sid]?.id === last) { seen[ui.sid].t = Date.now(); storage.write(SEEN,seen); return; }
    seen[ui.sid] = {id:last,at:ui.model.lastAt === null ? null : ui.model.lastAt + (ui.model.offset || 0),t:Date.now()};
    const ids = Object.keys(seen); if (ids.length > 200) for (const id of ids.sort((a,b) => (seen[a].t || 0) - (seen[b].t || 0)).slice(0,ids.length - 200)) delete seen[id];
    storage.write(SEEN,seen);
  }
  function firstLoad() {
    const seen = storage.read(SEEN)[ui.sid], last = [...state.eventIds].reduce((max,id) => Math.max(max,Number(id) || 0),0);
    // A reload or a short look away is not "away": the line needs two minutes since this browser last showed the session.
    if (seen && Number.isInteger(seen.id) && seen.id < last && Date.now() - (seen.t || 0) >= 120000) separator(seen.id + 1,typeof seen.at === 'number' ? seen.at : null);
    if (ui.model.order.some(id => ui.model.runs.get(id).finished)) loadData();
  }
  // "New since" sits before the first event after the person left; there is one at a time.
  function separator(fromId, since) {
    $('events').querySelector('.run-new-since')?.remove();
    let target = null; for (const item of $('events').querySelectorAll('[data-event-id]')) if (Number(item.dataset.eventId) >= fromId) { target = item; break; }
    if (!target) return null;
    const host = target.closest('.activity-group') && target.classList.contains('step-row') && target.previousElementSibling ? target : target.closest('#events > *') || target;
    const node = el('p','run-divider run-new-since'); node.tabIndex = -1;
    node.append(el('span','',since === null ? 'New since your last visit' : `New since ${new Date(since).toLocaleTimeString([],{hour:'2-digit',minute:'2-digit'})}`));
    // The return is announced once, by the away summary; the separator stays reachable by its name.
    quiet(node); host.before(node);
    // Inside a step group that began before the person left, the group opens so the line is seen.
    const group = node.closest('details'); if (group) group.open = true;
    return node;
  }
  function visibility() {
    if (document.hidden) {
      if (ui.loaded && state.view === 'sessions') rememberSeen(true);
      ui.hiddenAt = Date.now(); const run = RunModel.current(ui.model);
      ui.away = {at:Date.now(),snap:run && !run.finished ? RunModel.snapshot(run) : null,runId:run?.id || null,events:0,firstId:null}; return;
    }
    const away = ui.away; ui.hiddenAt = null; ui.away = null;
    if (!away || !ui.loaded || state.view !== 'sessions') return;
    const minutes = Math.round((Date.now() - away.at) / 60000);
    if (Date.now() - away.at >= 120000 && away.events > 0) {
      separator(away.firstId,away.at);
      const run = away.runId && ui.model.runs.get(away.runId);
      if (run && away.snap) {
        ui.awayLine = {text:RunModel.awaySummary(run,away.snap,minutes),runId:run.id};
        const messages = run.counters.messages - away.snap.messages, failed = run.counters.commandFailed - away.snap.failed;
        say(`While you were away: ${[run.finished ? `run ${run.outcome === 'completed' ? 'finished' : run.outcome}` : '',messages ? count(messages,'new message') : '',failed ? `${count(failed,'command')} failed` : ''].filter(Boolean).join(', ') || count(run.counters.steps - away.snap.steps,'step')}.`);
        if (run.finished) { const entry = ui.receipts.get(run.id); if (entry) entry.key = ''; }
      }
    }
    rememberSeen(true); render(null,true);
  }
  function jumpToNew() {
    const node = $('events').querySelector('.run-new-since'); ui.awayLine = null; render(null);
    if (node) { const conversation = $('conversation'); conversation.scrollTo({top:conversation.scrollTop + node.getBoundingClientRect().top - conversation.getBoundingClientRect().top - 16,behavior:scrollMotion()}); node.focus({preventScroll:true}); }
  }
  // One polite status for milestones, at most once in ten seconds; a newer one replaces one still waiting.
  function say(text) {
    ui.announceText = text; const wait = ui.announced + 10000 - Date.now();
    if (wait <= 0) { $('run-announcer').textContent = text; ui.announced = Date.now(); ui.announceText = null; return; }
    if (!ui.announceTimer) ui.announceTimer = setTimeout(() => { ui.announceTimer = 0; if (ui.announceText) say(ui.announceText); },wait);
  }
  function finishedLine(run) {
    const span = secondsOf(run,launchOf(run),true), edited = [...run.paths.values()].filter(p => p.edited).length;
    const total = span ? Math.round(span.seconds) : 0, minutes = Math.floor(total / 60), seconds = total % 60;
    const time = span ? `${minutes ? `${count(minutes,'minute')} ` : ''}${count(seconds,'second')}` : 'an unrecorded time';
    return `Run ${run.outcome === 'completed' ? 'complete' : run.outcome || 'ended'} in ${time}. ${count(edited,'file')} edited, ${count(run.counters.commandFailed,'command')} failed. Receipt below.`;
  }

  // ---------- One render
  function render(ch, catchUp = false) {
    const session = state.selected, view = $('run-view'); ui.mode = modeOf(session);
    const anim = Boolean(ch?.live) && !reducedMotion.matches && !document.hidden && !reading();
    // A page drawn without motion (history, a hidden tab's catch-up, the reading guard) holds CSS transitions too; a render
    // the person asked for (Detailed, a tab, a filter) keeps the transitions it starts.
    const still = catchUp || Boolean(ch) && !anim;
    if (still) view.dataset.still = 'true';
    renderStrip(ch,anim); renderPanel(ch,anim); if (ui.mode === 'now') renderPulse(shownRun(session));
    renderReceipts(); if (anim && ch.finished) riseReceipt(ch.finished);
    // The new styles apply while transitions are off; turning them back on afterwards starts none.
    if (still) { void view.offsetHeight; delete view.dataset.still; }
    if (ch?.live && !document.hidden) for (const text of ch.milestones) { const run = text === 'Run finished' ? ui.model.runs.get(ch.finished) : null; say(run ? finishedLine(run) : text); }
  }
  // Called by updateControls: the waiting text, the session's phase and the transitions that title, icon and notifications follow.
  function sync(waitingText) {
    const session = state.selected, p = phase(session);
    if (session?.id !== ui.watch?.sid || p !== ui.watch?.phase || session?.launch?.status !== ui.watch?.launch?.status || session?.status !== ui.watch?.status) transitions(session,p);
    if (p && p !== 'check' && ui.showAfter) ui.showAfter = null;
    if ($('waiting-text').textContent !== waitingText) $('waiting-text').textContent = waitingText;
    if (ui.dataStale && ui.loaded && !active(session)) loadData();
    const mode = modeOf(session); if (mode !== ui.mode) render(null); else if (mode === 'waiting' || mode === 'closed') renderStrip(null,false);
  }

  // The Calm/Detailed switch on screen: the narrow window shows only the compact chip.
  function detailToggle() { return mobileNav.matches ? $('run-compact') : $('run-calm'); }
  // Focus leaves a strip that is closing for the turn's receipt, or the composer when there is none. The receipt itself
  // takes it, named by its title: results arriving later rebuild its contents, never the article.
  function focusOut() {
    const node = [...ui.receipts.values()].map(entry => entry.node).filter(item => item.isConnected).pop();
    if (node) { node.tabIndex = -1; node.focus({preventScroll:true}); } else $('prompt').focus({preventScroll:true});
  }

  // ---------- Wiring
  $('run-calm').addEventListener('click',() => setDetailed(false));
  $('run-detailed').addEventListener('click',() => setDetailed(true));
  $('run-compact').addEventListener('click',() => setDetailed(!ui.detailed));
  $('run-close').addEventListener('click',close);
  $('run-notify').addEventListener('click',askPermission);
  $('run-away-jump').addEventListener('click',jumpToNew);
  $('run-away-close').addEventListener('click',() => { ui.awayLine = null; render(null); $('run-now').focus(); });
  $('run-now').addEventListener('click',() => {
    if (ui.mode === 'after') { close(); return; }
    const n = RunModel.now(ui.model,shownRun()); if (n?.eventId !== undefined && n?.eventId !== null) reveal(n.eventId); else { const conversation = $('conversation'); conversation.scrollTo({top:conversation.scrollHeight,behavior:scrollMotion()}); }
  });
  for (const tab of document.querySelectorAll('.run-tab')) tab.addEventListener('click',() => { ui.tab = tab.dataset.tab; savePrefs(); hideCard(); render(null); });
  $('run-tabs').addEventListener('keydown',event => {
    const tabs = [...document.querySelectorAll('.run-tab')].filter(tab => !tab.hidden), index = tabs.indexOf(document.activeElement); if (index < 0) return;
    const next = {ArrowRight:tabs[(index + 1) % tabs.length],ArrowLeft:tabs[(index - 1 + tabs.length) % tabs.length],Home:tabs[0],End:tabs[tabs.length - 1]}[event.key];
    if (next) { event.preventDefault(); ui.tab = next.dataset.tab; savePrefs(); render(null); next.focus(); }
  });
  $('run-view-map').addEventListener('click',() => { ui.filesView = 'map'; hideCard(); render(null); });
  $('run-view-list').addEventListener('click',() => { ui.filesView = 'list'; hideCard(); render(null); });
  $('run-tree').addEventListener('keydown',onTreeKey);
  cardNode.addEventListener('pointerenter',() => clearTimeout(ui.cardTimer));
  cardNode.addEventListener('pointerleave',hideCardSoon);
  cardNode.addEventListener('focusout',event => { if (ui.card && !cardNode.contains(event.relatedTarget) && event.relatedTarget !== ui.card.anchor) hideCardSoon(); });
  document.addEventListener('keydown',event => {
    if (event.key !== 'Escape') return;
    // A card under the pointer closes without taking focus from where the person types.
    if (ui.card) { hideCard(ui.card.pinned || cardNode.contains(document.activeElement)); return; }
    if (ui.detailed && $('run-view').contains(document.activeElement)) { setDetailed(false); detailToggle().focus({preventScroll:true}); }
  });
  document.addEventListener('pointerdown',event => { if (ui.card?.pinned && !cardNode.contains(event.target) && !event.target.closest?.('.run-file')) hideCard(); });
  document.addEventListener('visibilitychange',visibility);
  $('conversation').addEventListener('scroll',() => { const conversation = $('conversation'); ui.reading = conversation.scrollHeight - conversation.scrollTop - conversation.clientHeight >= 130; },{passive:true});
  $('check-command').addEventListener('input',event => { if (event.isTrusted) $('check-prefill-note').hidden = true; });
  new MutationObserver(() => { ui.iconKey = ''; icon(state.selected); }).observe(document.documentElement,{attributes:true,attributeFilter:['data-theme']});
  // The strip and panel change height; a resized layout keeps the composer on screen.
  new ResizeObserver(refitOutput).observe($('run-view'));
  addEventListener('resize',() => { if (ui.card && !ui.card.pinned) hideCard(); if (ui.detailed) fitPanel(); });
  mobileNav.addEventListener('change',() => { ui.nowKey = ''; hideCard(); render(null); });
  return {reset,begin,observe,end,sync,phase,statusNode,titlePrefix,icon,connection,reveal};
})();
