// Knowledge › Memory use: what agent memory holds, how it moves and how often retrieval selects it.
// Classic script loaded after app-knowledge.js. The view reads one payload that the server builds without the
// knowledge lock and never writes; "Check eligibility" is its only request that runs project code, on a click.
'use strict';
const memoryUse = {projectId:null,bankId:null,data:null,epoch:0,controller:null,loading:false,error:'',window:30,open:null,
  active:null,pinned:null,chunk:null,filters:new Set(),check:null,checkPending:false,checkError:'',anchor:null,played:null,visibleSince:0,count:0,
  sort:{chunks:['review',1],retrievals:['time',-1]}};
const memoryWindows = [[7,'7 d'],[30,'30 d'],[90,'90 d'],[0,'All']];
const memoryKinds = [['brain','Project Brain'],['bank','Memory bank'],['rules','Rules & docs']];
const memoryPhone = window.matchMedia('(max-width:760px)');
const DAY_MS = 86400000;
// Dates read '3 Oct', or '31 Aug 2027' outside the current year; month names are fixed rather than locale-dependent ('Sept').
const memoryMonths = ['Jan','Feb','Mar','Apr','May','Jun','Jul','Aug','Sep','Oct','Nov','Dec'];
const memoryClock = {time:new Intl.DateTimeFormat('en-GB',{hour:'2-digit',minute:'2-digit'})};
// Date-only values are calendar days in local time, the clock the runtime's review check uses.
const calendarDay = value => typeof value === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(value) ? new Date(`${value}T00:00:00`) : new Date(value);
function memoryDate(value) {
  const date = calendarDay(value); if (Number.isNaN(date.getTime())) return '—';
  return `${date.getDate()} ${memoryMonths[date.getMonth()]}${date.getFullYear() === new Date().getFullYear() ? '' : ` ${date.getFullYear()}`}`;
}
const memoryTime = value => `${memoryDate(value)} ${memoryClock.time.format(new Date(value))}`;
const daysBetween = (from, to) => Math.round((calendarDay(to) - calendarDay(from)) / DAY_MS);
const reviewLabel = days => days === null ? '—' : days === 0 ? 'today' : days > 0 ? `in ${fmt.exact(days).text} d` : `${fmt.exact(-days).text} d overdue`;
const plural = (count, one, many = one + 's') => `${fmt.exact(count).text} ${count === 1 ? one : many}`;
const reasonLabel = reason => typeof reason === 'string' ? reason.replace(/-/g,' ') : '—';
// Band thickness uses fixed bins so a count of 9 and one of 10 look alike and 1 still shows.
const bandWidth = count => !count ? 1 : count <= 2 ? 2 : count <= 5 ? 4 : count <= 10 ? 8 : count <= 25 ? 12 : count <= 50 ? 16 : 24;
const motionToken = name => parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name)) || 0;
const easingToken = name => getComputedStyle(document.documentElement).getPropertyValue(name).trim() || 'ease';

// Everything the view shows is counted here from the payload, for one flow window; the strip and horizon do not use the window.
function memoryUseModel(data, windowDays, now = Date.now()) {
  const start = windowDays ? now - windowDays * DAY_MS : -Infinity, within = value => Boolean(value) && calendarDay(value).getTime() >= start;
  const chunks = data.chunks?.items || [], records = data.brain?.items || [], promotions = data.promotions?.items || [], rows = data.retrievals?.items || [];
  const review = chunk => chunk.review_after ? daysBetween(data.today, chunk.review_after) : null;
  const active = chunks.filter(chunk => chunk.status === 'active'), retired = chunks.filter(chunk => ['superseded','archived'].includes(chunk.status));
  const selectedIn = new Map(), cutIn = new Map(), tasks = new Map(), cuts = {}, routes = {}, kinds = {brain:0,bank:0,rules:0};
  for (const row of rows) {
    for (const id of new Set(row.chunks)) { selectedIn.set(id,(selectedIn.get(id) || 0) + 1); if (!tasks.has(id)) tasks.set(id,new Set()); tasks.get(id).add(row.task); }
    for (const [id,reason] of row.cuts) { cutIn.set(id,(cutIn.get(id) || 0) + 1); cuts[reason] = (cuts[reason] || 0) + 1; }
    routes[row.route] = (routes[row.route] || 0) + 1; for (const [kind] of memoryKinds) kinds[kind] += row[kind];
  }
  const since = rows[0]?.at || null, health = (data.health?.items || []).filter(line => since && line.at >= since);
  const pending = mode => promotions.filter(item => item.mode === mode && ['proposed','reviewed'].includes(item.status));
  const applied = promotions.filter(item => item.status === 'applied' && within(item.applied_at));
  const pastReview = active.filter(chunk => review(chunk) !== null && review(chunk) < 0), changed = active.filter(chunk => chunk.sources_changed === true);
  const stalled = data.promotions ? pending('automatic') : [];
  return {
    windowDays, review, selectedIn, cutIn, count:pastReview.length + changed.length + stalled.length,
    brain:data.brain && {total:records.length, open:records.filter(record => record.open).length, archived:records.filter(record => record.archived).length,
      private:records.filter(record => record.private).length, resolved:records.filter(record => within(record.resolved_at)).length,
      notPromoted:records.filter(record => within(record.resolved_at) && record.promotable && !record.promoted).length, truncated:data.brain.truncated, records},
    promotion:data.promotions && {proposed:promotions.filter(item => within(item.created_at)).length, applied:applied.length,
      auto:applied.filter(item => item.mode === 'automatic').length, human:applied.filter(item => item.mode === 'human').length,
      waiting:pending('human'), stalled, rejected:promotions.filter(item => item.status === 'rejected' && within(item.created_at)).length},
    bank:data.chunks && {files:chunks.length, active:active.length, person:active.filter(chunk => !chunk.auto).length, auto:active.filter(chunk => chunk.auto).length,
      reattested:active.filter(chunk => chunk.auto && chunk.last_verified > chunk.created).length, drafts:chunks.filter(chunk => chunk.status === 'needs-review').length,
      superseded:chunks.filter(chunk => chunk.status === 'superseded').length, archived:chunks.filter(chunk => chunk.status === 'archived').length,
      retired:retired.length, bytes:data.chunks.bytes, created:chunks.filter(chunk => within(chunk.created)).length,
      retiredInWindow:retired.filter(chunk => within(chunk.valid_to)).length, pastReview, changed, truncated:data.chunks.truncated,
      due:active.filter(chunk => review(chunk) !== null && review(chunk) >= 0 && review(chunk) <= 30)},
    selected:{rows, withChunk:rows.filter(row => row.chunks.length).length, selections:rows.reduce((sum,row) => sum + row.chunks.length,0), distinct:selectedIn.size,
      reused:[...tasks.values()].filter(set => set.size > 1).length, cuts, cutTotal:Object.values(cuts).reduce((sum,value) => sum + value,0), routes, kinds, since,
      dropped:health.filter(line => line.dropped === true).length, measured:health.filter(line => line.dropped !== null).length, merged:data.retrievals?.merged || 0,
      found:data.retrievals?.found || 0, limit:data.retrievals?.limit || 200},
  };
}
// What changed since the stored visit: chunks added, retired, re-attested or crossing review, promotions applied, retrievals.
function memoryChanges(data, anchor) {
  if (!anchor || !anchor.chunks || typeof anchor.at !== 'string') return null;
  const chunks = data.chunks?.items || [], before = id => Array.isArray(anchor.chunks[id]) ? anchor.chunks[id] : null;
  const rows = data.retrievals?.items || [], fresh = rows.filter(row => !anchor.newest || row.at > anchor.newest);
  return {since:anchor.at,
    added:chunks.filter(chunk => chunk.status === 'active' && !before(chunk.id)),
    retired:chunks.filter(chunk => before(chunk.id)?.[0] === 'active' && chunk.status !== 'active'),
    reattested:chunks.filter(chunk => before(chunk.id) && chunk.last_verified && chunk.last_verified > (before(chunk.id)[1] || '')),
    moved:chunks.filter(chunk => before(chunk.id) && chunk.status === 'active' && before(chunk.id)[2] && before(chunk.id)[2] !== chunk.review_after),
    crossed:chunks.filter(chunk => chunk.status === 'active' && chunk.review_after < data.today && before(chunk.id) && !(before(chunk.id)[2] < anchor.today)),
    applied:(data.promotions?.items || []).filter(item => item.status === 'applied' && (anchor.promotions || {})[memoryPromotionKey(item)] !== 'applied'),
    retrievals:fresh, firstNew:rows.length - fresh.length};
}
const memoryPromotionKey = item => `${item.created_at}|${item.mode}`;
function memorySnapshot(data) {
  return {at:new Date().toISOString(), today:data.today,
    chunks:Object.fromEntries((data.chunks?.items || []).map(chunk => [chunk.id,[chunk.status,chunk.last_verified,chunk.review_after]])),
    promotions:Object.fromEntries((data.promotions?.items || []).map(item => [memoryPromotionKey(item),item.status])),
    newest:(data.retrievals?.items || []).at(-1)?.at || null};
}
const memoryAnchorKey = () => `harness.memory-use.v1:${memoryUse.projectId}:${memoryUse.bankId}`;
function readMemoryAnchor() { try { const value = JSON.parse(localStorage.getItem(memoryAnchorKey()) || 'null'); return value && typeof value === 'object' ? value : null; } catch (_) { return null; } }
// The anchor moves only after the view was seen for two seconds, so a quick pass does not swallow the changes.
function moveMemoryAnchor() {
  if (!memoryUse.data || !memoryUse.visibleSince || performance.now() - memoryUse.visibleSince < 2000) return;
  const snapshot = memorySnapshot(memoryUse.data);
  try { localStorage.setItem(memoryAnchorKey(),JSON.stringify(snapshot)); } catch (_) { /* Storage may be disabled; every visit then reads as the first. */ }
  memoryUse.anchor = snapshot; memoryUse.visibleSince = 0;
}

async function loadMemoryUse({bank = memoryUse.bankId, quiet = false} = {}) {
  const projectId = $('memory-use-project').value;
  if (projectId !== memoryUse.projectId || bank !== memoryUse.bankId) { moveMemoryAnchor(); if (projectId !== memoryUse.projectId) bank = null; memoryUse.data = null; memoryUse.check = null; memoryUse.pinned = null; memoryUse.chunk = null; memoryUse.filters.clear(); }
  memoryUse.controller?.abort(); const controller = new AbortController(); memoryUse.controller = controller; const epoch = ++memoryUse.epoch;
  memoryUse.projectId = projectId; memoryUse.loading = true; memoryUse.error = ''; renderMemoryUseState(quiet);
  if (!state.bootstrap || !projectId) { memoryUse.loading = false; renderMemoryUseState(); return; }
  try {
    const data = await api(`/api/projects/${encodeURIComponent(projectId)}/memory-use${bank ? `?bank=${encodeURIComponent(bank)}` : ''}`,{signal:controller.signal});
    if (epoch !== memoryUse.epoch) return;
    memoryUse.data = data; memoryUse.bankId = data.bank_id; memoryUse.anchor = readMemoryAnchor();
    if (state.view === 'memory-use' && !memoryUse.visibleSince) memoryUse.visibleSince = performance.now();
  } catch (error) { if (error.name === 'AbortError' || epoch !== memoryUse.epoch) return; memoryUse.error = textError(error); }
  finally { if (epoch === memoryUse.epoch) { memoryUse.loading = false; memoryUse.controller = null; renderMemoryUseState(); renderMemoryUse(); } }
}
function renderMemoryUseState(quiet = false) {
  const busy = memoryUse.loading && Boolean(memoryUse.data);
  $('memory-use-body').setAttribute('aria-busy',String(memoryUse.loading)); $('memory-use-body').dataset.refreshing = String(busy);
  $('memory-use-refresh').disabled = memoryUse.loading || !state.bootstrap; $('memory-use-bank').disabled = memoryUse.loading || !memoryUse.data?.banks?.length;
  showError('memory-use-error',memoryUse.error);
  if (memoryUse.loading && !memoryUse.data && !quiet) { $('memory-use-since').textContent = 'Reading memory use…'; $('memory-use-body').replaceChildren(); }
}
function openMemoryUse() { memoryUse.visibleSince = memoryUse.data ? performance.now() : 0; if (memoryUse.projectId !== $('memory-use-project').value || !memoryUse.data) loadMemoryUse(); else renderMemoryUse(); }
function leaveMemoryUse() { moveMemoryAnchor(); memoryUse.controller?.abort(); }
function memoryUseSessionEnded(projectId) { if (state.view === 'memory-use' && projectId === memoryUse.projectId && !memoryUse.loading) loadMemoryUse({quiet:true}); }

function renderMemoryUse() {
  const data = memoryUse.data, body = $('memory-use-body');
  setOptions($('memory-use-bank'),data?.banks || [],item => item.name,item => item.id,memoryUse.bankId);
  if (!data) { if (!memoryUse.loading) { $('memory-use-since').textContent = ''; body.replaceChildren(el('p','empty-note',state.bootstrap && $('memory-use-project').value ? memoryUse.error ? 'Memory use could not be read. Refresh to try again.' : '' : 'Choose a project.')); } return; }
  if (!data.bank_id) { $('memory-use-since').textContent = ''; body.replaceChildren(el('p','empty-note','No memory bank found in this project.')); memoryUse.count = 0; renderViewTabs(); return; }
  const model = memoryUseModel(data,memoryUse.window), changes = memoryChanges(data,memoryUse.anchor);
  const bands = new Map([...body.querySelectorAll('.memory-band-part')].map(part => [`${part.closest('.memory-band').dataset.band}:${part.dataset.kind}`,part.style.getPropertyValue('--band')]));
  memoryUse.model = model; memoryUse.count = model.count;
  renderMemorySince(changes,model);
  const flow = el('section','memory-flow-section'); flow.setAttribute('aria-labelledby','memory-flow-title');
  flow.append(memorySectionHead('memory-flow-title','Memory flow',`${memoryUse.window ? `last ${memoryUse.window} d` : 'all time'}`,'Counts come from project files: Project Brain records and promotions, Memory bank chunk frontmatter, and the last 200 retrieval manifests on this machine. Selected by retrieval means a retrieval put the chunk in a capsule; it does not show that the agent read it.'),
    memoryFlow(model,changes),memoryFlowDetail(model));
  body.replaceChildren(flow,memoryHealth(model),memoryStrip(model,changes),memoryHorizon(model,changes),memoryTables(model),memoryKey());
  renderViewTabs();
  if (!reducedMotion.matches) for (const part of body.querySelectorAll('.memory-band-part')) {
    const before = bands.get(`${part.closest('.memory-band').dataset.band}:${part.dataset.kind}`), after = part.style.getPropertyValue('--band'), size = memoryPhone.matches ? 'width' : 'height';
    if (before && before !== after) part.animate([{[size]:before},{[size]:after}],{duration:motionToken('--motion-base'),easing:easingToken('--ease-standard')});
  }
  // Replay what changed since the last visit, once per stored visit, when motion is welcome.
  if (changes && memoryUse.played !== changes.since && state.view === 'memory-use') { memoryUse.played = changes.since; if (!reducedMotion.matches) requestAnimationFrame(() => playMemoryChanges(changes)); }
}
function memorySectionHead(id, title, range, help) {
  const head = el('div','memory-section-head'), heading = el('h3','',title); heading.id = id; head.append(heading);
  if (range) head.append(el('span','memory-range',range));
  if (!help) return head;
  const button = el('button','explain','?'); button.type = 'button'; button.setAttribute('aria-expanded','false'); button.setAttribute('aria-controls',id + '-help');
  button.setAttribute('aria-label',`About ${title.toLowerCase()}`); head.append(button);
  const note = el('p','knowledge-note memory-help',help); note.id = id + '-help'; note.hidden = true;
  const box = el('div','memory-head-box'); box.append(head,note); return box;
}
function renderMemorySince(changes, model) {
  const parts = [];
  if (changes) {
    const auto = changes.added.filter(chunk => chunk.auto).length;
    if (changes.added.length) parts.push(`+${changes.added.length} ${changes.added.length === 1 ? 'chunk' : 'chunks'}${auto ? ` (${auto} auto)` : ''}`);
    if (changes.retired.length) parts.push(`−${changes.retired.length} retired`);
    if (changes.reattested.length) parts.push(`${changes.reattested.length} re-attested`);
    if (changes.crossed.length) parts.push(`${changes.crossed.length} past review`);
    if (changes.retrievals.length) parts.push(`${plural(changes.retrievals.length,'retrieval')}, ${fmt.exact(changes.retrievals.filter(row => row.chunks.length).length).text} selected a chunk`);
  }
  $('memory-use-since').textContent = parts.length ? `Since ${memoryDate(changes.since)}: ${parts.join(' · ')}` : '';
}

function memoryStage(stage, name, lead, lines, branches, extra) {
  const item = el('li','memory-stage'); item.dataset.stage = stage;
  const heading = el('h3','memory-stage-head'), toggle = el('button','memory-stage-name',name); toggle.type = 'button';
  toggle.setAttribute('aria-expanded',String(memoryUse.open === stage)); toggle.setAttribute('aria-controls','memory-flow-detail');
  toggle.addEventListener('click',() => { memoryUse.open = memoryUse.open === stage ? null : stage; renderMemoryUse(); $('memory-flow').querySelector(`[data-stage="${stage}"] .memory-stage-name`)?.focus(); });
  heading.append(toggle); item.append(heading);
  const leadNode = el('p','memory-lead'); leadNode.append(numberNode(lead)); item.append(leadNode);
  for (const line of lines) item.append(el('p','memory-sub',line));
  if (extra) item.append(extra);
  const list = el('ul','memory-branches'); for (const [text,warning] of branches) { const branch = el('li',warning ? 'memory-branch warning' : 'memory-branch',text); list.append(branch); }
  if (list.children.length) item.append(list);
  return item;
}
function memoryBand(name, parts, label) {
  const band = el('div','memory-band'); band.setAttribute('aria-hidden','true'); band.dataset.band = name;
  band.append(el('span','memory-band-count',label));
  const bar = el('span','memory-band-bar');
  for (const [kind,count] of parts) { const part = el('span','memory-band-part'); part.dataset.kind = kind; part.style.setProperty('--band',`${bandWidth(count)}px`); part.dataset.empty = String(!count); if (!count && parts.length > 1) part.hidden = true; bar.append(part); }
  band.append(bar); return band;
}
function memoryFlow(model, changes) {
  const list = el('ol','memory-flow'); list.id = 'memory-flow'; list.setAttribute('aria-label','Memory flow');
  const windowText = memoryUse.window ? `in ${memoryUse.window} d` : 'in all';
  const governed = Boolean(model.brain), needs = 'Needs a governed Project Brain.';
  const brain = governed ? memoryStage('brain','Project Brain',fmt.exact(model.brain.total),[`${plural(model.brain.total,'record')} · ${fmt.exact(model.brain.open).text} open`,`+${fmt.exact(model.brain.resolved).text} resolved ${windowText}`],
    [[`${fmt.exact(model.brain.archived).text} archived`],...(model.brain.private ? [[`${fmt.exact(model.brain.private).text} private or restricted`]] : [])]) : memoryStage('brain','Project Brain',fmt.unknown('not available'),[needs],[]);
  const held = memoryUse.check?.held_back;
  const promotion = governed ? memoryStage('promotion','Promotion',fmt.exact(model.promotion.applied),[`applied ${windowText} · ${fmt.exact(model.promotion.auto).text} auto`],
    [...(model.promotion.waiting.length ? [[`${fmt.exact(model.promotion.waiting.length).text} waiting · ${fmt.exact(Math.max(...model.promotion.waiting.map(item => daysBetween(item.created_at,new Date().toISOString())))).text} d`]] : []),
     ...(model.promotion.stalled.length ? [[`✕ ${fmt.exact(model.promotion.stalled.length).text} stalled`,true]] : []),
     ...(held ? Object.keys(held).length ? [[`${fmt.exact(Object.values(held).reduce((sum,value) => sum + value,0)).text} held back: ${Object.entries(held).map(([reason,count]) => `${reasonLabel(reason)} ${count}`).join(' · ')}`]] : [] :
       model.brain.notPromoted ? [[`${fmt.exact(model.brain.notPromoted).text} not promoted`]] : [])]) : memoryStage('promotion','Promotion',fmt.unknown('not available'),[needs],[]);
  if (!model.bank) { list.append(brain,promotion,memoryStage('bank','Memory bank',fmt.unknown('not available'),['This bank has no chunks folder.'],[]),
    memoryStage('selected','Selected by retrieval',fmt.unknown('not recorded'),['No chunks to select.'],[])); return list; }
  const bank = model.bank, provenance = el('div','memory-provenance');
  const bar = el('div','memory-provenance-bar'); bar.setAttribute('role','img'); bar.setAttribute('aria-label',`${plural(bank.active,'active chunk')}: ${fmt.exact(bank.person).text} written or reviewed by a person, ${fmt.exact(bank.auto).text} auto-promoted`);
  for (const [kind,count] of [['person',bank.person],['auto',bank.auto]]) { const part = el('span','memory-provenance-part'); part.dataset.kind = kind; part.style.width = bank.active ? `${count / bank.active * 100}%` : '0'; part.hidden = !count; bar.append(part); }
  provenance.append(bar,el('p','memory-sub',`${fmt.exact(bank.person).text} ● person · ${fmt.exact(bank.auto).text} ○ auto`));
  const delta = bank.created - bank.retiredInWindow;
  const bankStage = memoryStage('bank','Memory bank',fmt.exact(bank.active),[`active ${bank.active === 1 ? 'chunk' : 'chunks'} · ${delta ? `${delta > 0 ? '+' : '−'}${fmt.exact(Math.abs(delta)).text}` : 'no change'} ${windowText}`],
    [...(bank.pastReview.length ? [[`▲ ${fmt.exact(bank.pastReview.length).text} past review`,true]] : []),...(bank.changed.length ? [[`▲ ${fmt.exact(bank.changed.length).text} sources changed`,true]] : []),
     ...(bank.drafts ? [[`${fmt.exact(bank.drafts).text} ${bank.drafts === 1 ? 'draft' : 'drafts'}`]] : []),...(bank.retired ? [[`${fmt.exact(bank.retired).text} retired`]] : []),
     ...(memoryUse.check ? [[`Retrieval skips ${fmt.exact(Object.keys(memoryUse.check.skips).length).text}`]] : [])],provenance);
  if (changes?.added.length && reducedMotion.matches) bankStage.querySelector('.memory-lead').append(el('span','memory-plus',`+${changes.added.length}`));
  const selected = model.selected;
  const selectedStage = memoryStage('selected','Selected by retrieval',selected.rows.length ? fmt.exact(selected.withChunk) : fmt.unknown('not recorded'),
    selected.rows.length ? [`of ${plural(selected.rows.length,'retrieval')} · ${plural(selected.distinct,'chunk')}`,`${fmt.exact(selected.reused).text} reused across tasks`] : ['No retrievals recorded on this machine.'],
    selected.cutTotal ? [[`${fmt.exact(selected.cutTotal).text} cut at retrieval`]] : []);
  list.append(brain,promotion,bankStage,selectedStage);
  if (governed) {
    promotion.prepend(memoryBand('proposed',[['brain',model.promotion.proposed]],fmt.exact(model.promotion.proposed).text));
    bankStage.prepend(memoryBand('applied',[['person',model.promotion.human],['auto',model.promotion.auto]],fmt.exact(model.promotion.applied).text));
  }
  selectedStage.prepend(memoryBand('selected',[['bank',selected.selections]],fmt.exact(selected.selections).text));
  return list;
}
function memoryTable(caption, headers, rows) {
  const table = el('table','memory-table'); const head = el('tr');
  table.append(Object.assign(el('caption','',caption)));
  for (const header of headers) { const cell = el('th','',header); cell.scope = 'col'; head.append(cell); }
  const thead = el('thead'); thead.append(head); const tbody = el('tbody');
  for (const row of rows) { const line = el('tr'); row.forEach((value,index) => { const cell = el(index ? 'td' : 'th',''); if (!index) cell.scope = 'row'; cell.append(value instanceof Node ? value : document.createTextNode(String(value))); line.append(cell); }); tbody.append(line); }
  table.append(thead,tbody); return table;
}
function memoryCheckControls(scope) {
  const box = el('div','memory-check'), button = el('button','button','Check eligibility'); button.type = 'button';
  button.hidden = !memoryUse.data?.check_available; button.disabled = memoryUse.checkPending || Boolean(knowledgeState.pending);
  button.addEventListener('click',() => runMemoryUseCheck(scope)); box.append(button);
  const status = el('p','knowledge-note',memoryUse.checkPending ? 'Asking the installed runtime…' : memoryUse.checkError ? `— ${memoryUse.checkError}` : memoryUse.check ? `Checked ${memoryClock.time.format(new Date(memoryUse.check.checked_at))}` : 'Runs the installed runtime’s eligibility rules. It writes nothing.');
  status.setAttribute('role','status'); box.append(status); return box;
}
function memoryFlowDetail(model) {
  const box = el('div','memory-flow-detail'); box.id = 'memory-flow-detail'; box.hidden = !memoryUse.open; if (!memoryUse.open) return box;
  const windowText = memoryUse.window ? `${memoryUse.window} d` : 'all time';
  if ({brain:!model.brain,promotion:!model.promotion,bank:!model.bank}[memoryUse.open]) box.append(el('p','knowledge-note',memoryUse.open === 'bank' ? 'This bank has no chunks folder.' : 'Needs a governed Project Brain.'));
  else if (memoryUse.open === 'brain' && model.brain) {
    const counts = new Map(); for (const record of model.brain.records) if (!record.private) { const key = `${record.type}\u0000${record.status}`; counts.set(key,(counts.get(key) || 0) + 1); }
    const rows = [...counts].sort().map(([key,count]) => { const [type,status] = key.split('\u0000'); return [humanLabel(type),humanLabel(status),fmt.exact(count).text]; });
    if (model.brain.private) rows.push(['Private or restricted','—',fmt.exact(model.brain.private).text]);
    box.append(memoryTable('Project Brain records by type and status',['Type','Status','Records'],rows));
    if (model.brain.truncated) box.append(el('p','memory-notice','The record listing reached its limit: 5,000+ records.'));
  } else if (memoryUse.open === 'promotion' && model.promotion) {
    const oldest = items => items.length ? `${fmt.exact(Math.max(...items.map(item => daysBetween(item.created_at,new Date().toISOString())))).text} d` : '—';
    box.append(memoryTable(`Promotions, ${windowText}`,['Promotion','Count','Detail'],[
      ['Proposed',fmt.exact(model.promotion.proposed).text,''],['Applied after review',fmt.exact(model.promotion.human).text,''],['Applied automatically',fmt.exact(model.promotion.auto).text,'No person approved these.'],
      ['Waiting for review',fmt.exact(model.promotion.waiting.length).text,`oldest ${oldest(model.promotion.waiting)}`],['Stalled automatic',fmt.exact(model.promotion.stalled.length).text,model.promotion.stalled.length ? `oldest ${oldest(model.promotion.stalled)}` : ''],
      ['Rejected',fmt.exact(model.promotion.rejected).text,''],['Resolved but not promoted',fmt.exact(model.brain.notPromoted).text,'Check eligibility names the rule.']]));
    if (memoryUse.check?.held_back) box.append(memoryTable('Held back by the runtime (all records)',['Rule','Records'],Object.entries(memoryUse.check.held_back).map(([reason,count]) => [humanLabel(reason),fmt.exact(count).text])));
    box.append(memoryCheckControls('promotion'));
  } else if (memoryUse.open === 'bank' && model.bank) {
    const bank = model.bank;
    box.append(memoryTable('Memory bank chunks',['Group','Chunks'],[['Active · written or reviewed by a person',fmt.exact(bank.person).text],['Active · auto-promoted',fmt.exact(bank.auto).text],
      ['· re-attested since promotion',fmt.exact(bank.reattested).text],['Drafts (needs review)',fmt.exact(bank.drafts).text],['Superseded',fmt.exact(bank.superseded).text],['Archived',fmt.exact(bank.archived).text],
      ['Past review date',fmt.exact(bank.pastReview.length).text],['Due within 30 d',fmt.exact(bank.due.length).text],['Cited files changed',fmt.exact(bank.changed.length).text],
      ['Files',`${fmt.exact(bank.files).text}${bank.truncated ? '+' : ''} · ${bytesLabel(bank.bytes)}`]]));
    if (memoryUse.check) { const skips = {}; for (const reason of Object.values(memoryUse.check.skips)) skips[reason] = (skips[reason] || 0) + 1; box.append(memoryTable(`Retrieval skips ${Object.keys(memoryUse.check.skips).length}`,['Reason','Chunks'],Object.entries(skips).map(([reason,count]) => [reasonLabel(reason),fmt.exact(count).text]))); }
    box.append(memoryCheckControls('bank'));
  } else if (memoryUse.open === 'selected') {
    const selected = model.selected;
    if (!selected.rows.length) { box.append(el('p','knowledge-note','No retrievals recorded on this machine.')); return box; }
    box.append(memoryTable(`Routes of the last ${plural(selected.rows.length,'retrieval')}`,['Route','Retrievals'],Object.entries(selected.routes).sort((a,b) => b[1] - a[1]).map(([route,count]) => [route,fmt.exact(count).text])),
      memoryTable('Chunks cut at retrieval',['Reason','Chunks'],Object.entries(selected.cuts).map(([reason,count]) => [reasonLabel(reason),fmt.exact(count).text])));
    box.append(el('p','knowledge-note',`Reached the agent: at most ${fmt.exact(selected.withChunk).text} of ${fmt.exact(selected.rows.length).text}. ${selected.measured ? `${fmt.exact(selected.dropped).text} of ${plural(selected.measured,'refresh','refreshes')} dropped items to fit 8,000 characters.` : 'Dropped to fit: — (not recorded).'}${selected.merged ? ` ${plural(selected.merged,'freshness re-check')} counted with ${selected.merged === 1 ? 'its' : 'their'} Harness retrieval.` : ''}`));
  }
  return box;
}
function memoryHealth(model) {
  const box = el('div','memory-health'); box.setAttribute('aria-label','Memory health');
  const row = (symbol, text, actions = []) => { const line = el('div',symbol === '✕' ? 'memory-health-row error' : 'memory-health-row'); line.append(el('span','memory-health-text',`${symbol} ${text}`)); for (const [label,action] of actions) { const button = el('button','button',label); button.type = 'button'; button.addEventListener('click',action); line.append(button); } box.append(line); };
  const bank = model.bank; if (!bank) return box;
  if (bank.pastReview.length) row('▲',`${plural(bank.pastReview.length,'chunk is','chunks are')} past ${bank.pastReview.length === 1 ? 'its' : 'their'} review date. Retrieval skips ${bank.pastReview.length === 1 ? 'it' : 'them'}, and the bank fails validation until ${bank.pastReview.length === 1 ? 'it is' : 'they are'} re-attested or retired.`,
    [['Show',() => showMemoryFilter('past')],['Re-attest…',() => openMemoryAction('bank-reverify',bank.pastReview[0])]]);
  if (bank.changed.length) row('▲',`${plural(bank.changed.length,'chunk cites','chunks cite')} files that changed since ${bank.changed.length === 1 ? 'it was' : 'they were'} attested.`,[['Show',() => showMemoryFilter('changed')]]);
  if (model.promotion?.stalled.length) row('✕',`${plural(model.promotion.stalled.length,'automatic promotion')} stalled before applying. The runtime retries ${model.promotion.stalled.length === 1 ? 'it' : 'them'} on the next promotion run.`,[['Show',() => { memoryUse.open = 'promotion'; renderMemoryUse(); $('memory-flow-detail').scrollIntoView({block:'nearest',behavior:scrollMotion()}); }]]);
  if (bank.truncated) row('▲','The chunk listing reached its limit: 500+ chunks. Counts cover the newest 500.');
  return box;
}
function showMemoryFilter(filter) { memoryUse.filters = new Set([filter]); renderMemoryUse(); document.querySelector('.memory-horizon-section')?.scrollIntoView({block:'start',behavior:scrollMotion()}); }
function openMemoryAction(action, chunk) {
  if (!chunk) return; memoryState.projectId = memoryUse.projectId; memoryState.bankId = memoryUse.bankId; memoryState.path = chunk.path;
  if ($('memory-project').value !== memoryUse.projectId) $('memory-project').value = memoryUse.projectId;
  openView('memory');
  if (!action) return;
  const select = $('memory-knowledge-action'); if (!select) return; select.value = action; select.dispatchEvent(new Event('change'));
  if ($('memory-op-memory_id')) $('memory-op-memory_id').value = chunk.id;
  $('memory-op-memory_id')?.scrollIntoView({block:'center',behavior:'instant'});
}
async function runMemoryUseCheck(scope) {
  if (memoryUse.checkPending || !memoryUse.data?.check_available) return; const epoch = memoryUse.epoch;
  memoryUse.checkPending = true; memoryUse.checkError = ''; renderMemoryUse();
  try {
    const result = await api(`/api/projects/${encodeURIComponent(memoryUse.projectId)}/memory-use/check`,{method:'POST',body:{bank:memoryUse.bankId}});
    if (epoch === memoryUse.epoch) memoryUse.check = result;
  } catch (error) { if (epoch === memoryUse.epoch) memoryUse.checkError = textError(error); }
  finally { if (epoch === memoryUse.epoch) { memoryUse.checkPending = false; memoryUse.open = scope; renderMemoryUse(); } }
}

function memoryStrip(model, changes) {
  const section = el('section','memory-section memory-strip-section'); section.setAttribute('aria-labelledby','memory-strip-title');
  const rows = model.selected.rows, shown = memoryPhone.matches ? rows.slice(-80) : rows, offset = rows.length - shown.length;
  section.append(memorySectionHead('memory-strip-title',`Last ${plural(shown.length,'retrieval')}`,shown.length ? `${memoryDate(shown[0].at)} – ${memoryDate(shown.at(-1).at)}` : '',
    'One column per retrieval, oldest on the left. Each block is one selected item in its memory colour; grey blocks above Memory bank are chunks cut at retrieval. A Harness launch re-checks its context before running, and that re-check counts with its retrieval.'));
  if (!rows.length) { section.append(el('p','knowledge-note','No retrievals recorded on this machine.')); return section; }
  const most = kind => Math.max(1,...shown.map(row => row[kind] + (kind === 'bank' ? row.cuts.length : 0)));
  const strip = el('div','memory-strip'), labels = el('div','memory-strip-labels'), totals = el('div','memory-strip-totals');
  labels.setAttribute('aria-hidden','true'); totals.setAttribute('aria-hidden','true');
  const sums = {brain:0,bank:0,rules:0}; for (const row of shown) for (const [kind] of memoryKinds) sums[kind] += row[kind];
  for (const [kind,label] of memoryKinds) {
    const name = el('span','memory-strip-label',label); name.style.setProperty('--row',`${most(kind) * 4}px`); labels.append(name);
    const total = el('span','memory-strip-total',kind === 'bank' ? `${fmt.exact(sums.bank).text} in ${fmt.exact(shown.filter(row => row.chunks.length).length).text}` : `${fmt.exact(sums[kind]).text} selected`); total.style.setProperty('--row',`${most(kind) * 4}px`); totals.append(total);
  }
  const scroller = el('div','memory-strip-scroll'), lane = el('div','memory-strip-lane'), columns = el('div','memory-strip-columns');
  lane.setAttribute('aria-hidden','true');
  columns.setAttribute('role','listbox'); columns.setAttribute('aria-label',`Last ${shown.length} retrievals`); columns.setAttribute('aria-describedby','memory-strip-detail');
  const firstNew = changes && changes.retrievals.length ? Math.max(0,changes.firstNew - offset) : shown.length;
  let previousDay = null; const days = el('div','memory-strip-days'); days.setAttribute('aria-hidden','true');
  shown.forEach((row,index) => {
    const column = el('div','memory-column'), day = calendarDay(row.at).toDateString();
    column.setAttribute('role','option'); column.dataset.index = String(index + offset); column.tabIndex = -1; column.setAttribute('aria-selected',String(memoryUse.pinned === index + offset));
    if (day !== previousDay) { if (previousDay !== null) column.dataset.dayStart = 'true'; const tick = el('span','memory-strip-day',memoryDate(row.at)); tick.dataset.index = String(index); days.append(tick); previousDay = day; }
    if (index >= firstNew) column.dataset.new = 'true';
    column.setAttribute('aria-label',memoryRetrievalSentence(row,index + offset,rows.length));
    for (const [kind] of memoryKinds) {
      const cell = el('span','memory-cell'); cell.dataset.kind = kind; cell.style.setProperty('--row',`${most(kind) * 4}px`);
      if (kind === 'bank' && row.cuts.length) { const cut = el('span','memory-units cut'); cut.style.setProperty('--units',`${row.cuts.length * 4}px`); cell.append(cut); }
      const units = el('span','memory-units'); units.style.setProperty('--units',`${row[kind] * 4}px`); if (!row[kind]) units.hidden = true; cell.append(units); column.append(cell);
    }
    columns.append(column);
  });
  if (firstNew < shown.length) { const bracket = el('span','memory-strip-bracket'); bracket.dataset.from = String(firstNew); lane.append(bracket,el('span','memory-strip-new',`${fmt.exact(shown.length - firstNew).text} new`)); }
  const active = Number.isInteger(memoryUse.active) && memoryUse.active >= offset ? memoryUse.active : rows.length - 1;
  columns.querySelector(`[data-index="${active}"]`)?.setAttribute('tabindex','0');
  columns.addEventListener('keydown',memoryStripKeys); columns.addEventListener('focusin',event => { const column = event.target.closest('.memory-column'); if (column) showMemoryRetrieval(Number(column.dataset.index)); });
  columns.addEventListener('pointerover',event => { const column = event.target.closest('.memory-column'); if (column && memoryUse.pinned === null) showMemoryRetrieval(Number(column.dataset.index),false); });
  columns.addEventListener('click',event => { const column = event.target.closest('.memory-column'); if (column) { pinMemoryRetrieval(Number(column.dataset.index)); column.focus(); } });
  scroller.append(lane,columns,days); strip.append(labels,scroller,totals);
  const detail = el('div','memory-detail'); detail.id = 'memory-strip-detail';
  section.append(strip,detail);
  requestAnimationFrame(() => { placeMemoryStripMarks(columns,lane,days); showMemoryRetrieval(memoryUse.pinned ?? active,false); });
  return section;
}
// Day labels and the new-since bracket sit over measured columns, so they follow day gaps and the phone cut.
function placeMemoryStripMarks(columns, lane, days) {
  const origin = columns.getBoundingClientRect().left, nodes = columns.children;
  let last = -Infinity;
  for (const tick of days.children) { const column = nodes[Number(tick.dataset.index)]; if (!column) continue; const left = column.getBoundingClientRect().left - origin; tick.style.left = `${left}px`; tick.hidden = left - last < 48; if (!tick.hidden) last = left; }
  const bracket = lane.querySelector('.memory-strip-bracket'); if (!bracket) return;
  const first = nodes[Number(bracket.dataset.from)], end = nodes[nodes.length - 1];
  if (!first || !end) return;
  const left = first.getBoundingClientRect().left - origin, right = end.getBoundingClientRect().right - origin, label = lane.querySelector('.memory-strip-new');
  bracket.style.left = `${left}px`; bracket.style.width = `${right - left}px`;
  // The label starts with the bracket, or ends with it when the new columns are few.
  label.style.left = `${Math.max(0,Math.min(left,right - label.offsetWidth))}px`;
}
function memoryRetrievalSentence(row, index, total) {
  const cut = row.cuts.length ? `; ${plural(row.cuts.length,'chunk')} cut, ${[...new Set(row.cuts.map(([,reason]) => reasonLabel(reason)))].join(', ')}` : '';
  return `Retrieval ${index + 1} of ${total}, ${memoryTime(row.at)}, ${row.route}: Project Brain ${row.brain}, Memory bank ${row.bank}, Rules and docs ${row.rules}${cut}`;
}
function showMemoryRetrieval(index, announce = true) {
  const rows = memoryUse.model?.selected.rows || [], row = rows[index], detail = $('memory-strip-detail'); if (!detail) return;
  if (!row) { detail.replaceChildren(); return; }
  memoryUse.active = index; const card = el('div','memory-card');
  card.append(el('p','memory-card-title',`${memoryTime(row.at)} · ${row.route}${memoryUse.pinned === index ? ' · pinned' : ''}`),
    el('p','memory-card-line',memoryKinds.map(([kind,label]) => `${label} ${row[kind]}`).join(' · ')));
  if (row.chunks.length) { const line = el('p','memory-card-line','Chunks: '); row.chunks.forEach((id,position) => { if (position) line.append(', '); line.append(memoryChunkLink(id)); }); card.append(line); }
  if (row.cuts.length) card.append(el('p','memory-card-line',`Cut: ${row.cuts.map(([id,reason]) => `${id} (${reasonLabel(reason)})`).join(', ')}`));
  if (row.version !== null && row.version < 3) card.append(el('p','memory-card-line',`Manifest version ${row.version}: route not recorded.`));
  detail.replaceChildren(card); detail.setAttribute('aria-live',announce ? 'polite' : 'off');
}
function memoryChunkLink(id) {
  const chunk = memoryUse.data?.chunks?.items.find(item => item.id === id); if (!chunk) return document.createTextNode(id);
  const link = el('button','memory-link',id); link.type = 'button'; link.title = chunk.title || id; link.addEventListener('click',() => openMemoryAction(null,chunk)); return link;
}
function pinMemoryRetrieval(index, pinned = memoryUse.pinned === index ? null : index) {
  memoryUse.pinned = pinned; for (const column of document.querySelectorAll('.memory-column')) column.setAttribute('aria-selected',String(Number(column.dataset.index) === pinned));
  showMemoryRetrieval(index);
}
function memoryStripKeys(event) {
  const columns = [...event.currentTarget.children], current = columns.findIndex(column => column === document.activeElement); if (current < 0) return;
  const dayOf = position => calendarDay(memoryUse.model.selected.rows[Number(columns[position].dataset.index)].at).toDateString();
  let next = current;
  if (event.key === 'ArrowLeft') next = Math.max(0,current - 1); else if (event.key === 'ArrowRight') next = Math.min(columns.length - 1,current + 1);
  else if (event.key === 'Home') next = 0; else if (event.key === 'End') next = columns.length - 1;
  else if (event.key === 'PageUp') { const day = dayOf(current); next = current; while (next > 0 && dayOf(next) === day) next--; const target = dayOf(next); while (next > 0 && dayOf(next - 1) === target) next--; }
  else if (event.key === 'PageDown') { const day = dayOf(current); next = current; while (next < columns.length - 1 && dayOf(next) === day) next++; }
  else if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); pinMemoryRetrieval(Number(columns[current].dataset.index)); return; }
  else if (event.key === 'Escape') { if (memoryUse.pinned !== null) pinMemoryRetrieval(Number(columns[current].dataset.index),null); return; }
  else return;
  event.preventDefault(); columns[current].tabIndex = -1; columns[next].tabIndex = 0; columns[next].focus(); columns[next].scrollIntoView({block:'nearest',inline:'nearest',behavior:scrollMotion()});
}

const memoryFilters = [['all','All',() => true],['past','Past review',(chunk,model) => model.review(chunk) < 0],['due','Due ≤ 30 d',(chunk,model) => model.review(chunk) >= 0 && model.review(chunk) <= 30],
  ['changed','Sources changed',chunk => chunk.sources_changed === true],['unattested','Not re-attested since promotion',chunk => chunk.auto && !(chunk.last_verified > chunk.created)],
  ['never','Never selected',(chunk,model) => !model.selectedIn.has(chunk.id)]];
const memoryMatches = (chunk, model) => [...memoryUse.filters].every(name => memoryFilters.find(([id]) => id === name)?.[2](chunk,model) ?? true);
function memoryHorizon(model, changes) {
  const section = el('section','memory-section memory-horizon-section'); section.setAttribute('aria-labelledby','memory-horizon-title');
  section.append(memorySectionHead('memory-horizon-title','Review horizon','days until each active chunk’s review date',
    'Each dot is an active chunk, placed by days until its review date: ● written or reviewed by a person, ○ auto-promoted and not re-attested since, ● in magenta auto-promoted and re-attested since. Retrieval skips a chunk after its review date, and re-attesting moves it about a year ahead.'));
  const chunks = (memoryUse.data.chunks?.items || []).filter(chunk => chunk.status === 'active' && chunk.review_after).sort((a,b) => a.review_after.localeCompare(b.review_after) || a.id.localeCompare(b.id));
  if (!chunks.length) { section.append(el('p','knowledge-note','No active chunks with a review date.')); return section; }
  const chips = el('div','memory-chips'); chips.setAttribute('role','group'); chips.setAttribute('aria-label','Filter chunks');
  for (const [id,label,test] of memoryFilters) {
    const count = chunks.filter(chunk => test(chunk,model)).length, chip = el('button','chip',`${label} ${fmt.exact(count).text}`); chip.type = 'button'; chip.dataset.filter = id;
    chip.setAttribute('aria-pressed',String(id === 'all' ? !memoryUse.filters.size : memoryUse.filters.has(id)));
    chip.addEventListener('click',() => { if (id === 'all') memoryUse.filters.clear(); else if (memoryUse.filters.has(id)) memoryUse.filters.delete(id); else memoryUse.filters.add(id); refreshMemoryFilters(); });
    if (!count && id !== 'all') chip.disabled = true; chips.append(chip);
  }
  const span = 395, bin = memoryPhone.matches ? 14 : 7, chart = el('div','memory-horizon');
  const past = el('div','memory-horizon-past'); past.setAttribute('aria-hidden','true'); past.style.width = `${30 / span * 100}%`; past.append(el('span','',memoryPhone.matches ? 'Past' : 'Past review date'));
  const zero = el('div','memory-horizon-zero'); zero.setAttribute('aria-hidden','true'); zero.style.left = `${30 / span * 100}%`;
  const group = el('div','memory-horizon-bins'); group.setAttribute('role','group'); group.setAttribute('aria-label','Active chunks by review date');
  const bins = new Map();
  for (const chunk of chunks) { const days = Math.max(-30,Math.min(365,model.review(chunk))), key = Math.floor((days + 30) / bin); if (!bins.has(key)) bins.set(key,[]); bins.get(key).push(chunk); }
  for (const [key,members] of bins) {
    const column = el('div','memory-bin'); column.style.left = `${key * bin / span * 100}%`; column.style.width = `${bin / span * 100}%`;
    members.forEach((chunk,index) => {
      const dot = el('button','memory-dot'); dot.type = 'button'; dot.tabIndex = -1; dot.dataset.id = chunk.id; dot.dataset.provenance = chunk.auto ? chunk.last_verified > chunk.created ? 'auto-attested' : 'auto' : 'person';
      dot.setAttribute('aria-label',memoryChunkSentence(chunk,model)); if (index >= 4) dot.dataset.overflow = 'true';
      dot.addEventListener('click',() => { memoryUse.chunk = chunk.id; showMemoryChunk(chunk); });
      dot.addEventListener('focus',() => showMemoryChunk(chunk)); dot.addEventListener('pointerenter',() => { if (!memoryUse.chunk) showMemoryChunk(chunk,false); });
      column.append(dot);
    });
    if (members.length > 4) column.append(el('span','memory-bin-more',`+${members.length - 4}`));
    group.append(column);
  }
  const dots = [...group.querySelectorAll('.memory-dot')]; (dots.find(dot => dot.dataset.id === memoryUse.chunk) || dots[0]).tabIndex = 0;
  group.addEventListener('keydown',event => {
    const ordered = [...group.querySelectorAll('.memory-dot')].sort((a,b) => chunks.findIndex(chunk => chunk.id === a.dataset.id) - chunks.findIndex(chunk => chunk.id === b.dataset.id));
    const current = ordered.indexOf(document.activeElement); if (current < 0) return;
    const next = {ArrowRight:current + 1,ArrowDown:current + 1,ArrowLeft:current - 1,ArrowUp:current - 1,Home:0,End:ordered.length - 1}[event.key];
    if (event.key === 'Enter') { event.preventDefault(); const chunk = chunks.find(item => item.id === document.activeElement.dataset.id); openMemoryAction(null,chunk); return; }
    if (next === undefined) return; event.preventDefault(); const target = ordered[Math.max(0,Math.min(ordered.length - 1,next))]; ordered[current].tabIndex = -1; target.tabIndex = 0; target.focus();
  });
  const ticks = el('div','memory-horizon-ticks'); ticks.setAttribute('aria-hidden','true');
  for (const day of memoryPhone.matches ? [0,180,365] : [-30,0,90,180,270,365]) { const tick = el('span','',day === -30 ? '−30' : day === 365 ? '365 d' : String(day)); tick.style.left = `${(day + 30) / span * 100}%`; ticks.append(tick); }
  chart.append(past,zero,group);
  const detail = el('div','memory-detail'); detail.id = 'memory-horizon-detail';
  section.append(chips,chart,ticks,detail);
  requestAnimationFrame(() => { refreshMemoryFilters(false); const chosen = chunks.find(chunk => chunk.id === memoryUse.chunk); if (chosen) showMemoryChunk(chosen,false); else detail.replaceChildren(el('p','knowledge-note','Focus or point at a dot to see its chunk.')); });
  return section;
}
function memoryChunkSentence(chunk, model) {
  const provenance = chunk.auto ? chunk.last_verified > chunk.created ? 'auto-promoted, re-attested since promotion' : 'auto-promoted, not re-attested since promotion' : 'written or reviewed by a person';
  return `${chunk.id}, ${provenance}, review ${reviewLabel(model.review(chunk))}, selected in ${fmt.exact(model.selectedIn.get(chunk.id) || 0).text} of ${fmt.exact(model.selected.rows.length).text} retrievals`;
}
function showMemoryChunk(chunk, announce = true) {
  const model = memoryUse.model, detail = $('memory-horizon-detail'); if (!detail || !model) return;
  const card = el('div','memory-card'), days = model.review(chunk);
  card.append(el('p','memory-card-title',`${chunk.id} · ${chunk.title || 'Untitled'}`),
    el('p','memory-card-line',`${humanLabel(chunk.type) || '—'} · ${chunk.auto ? 'auto-promoted' : chunk.promoted ? 'promoted after review' : 'written by a person'}${chunk.auto && chunk.last_verified > chunk.created ? ' · re-attested since' : ''}`),
    el('p','memory-card-line',`Review ${reviewLabel(days)} (${memoryDate(chunk.review_after)})${days < -30 ? ' · shown at ≤ −30' : days > 365 ? ' · shown at ≥ 365' : ''} · last verified ${memoryDate(chunk.last_verified)}`),
    el('p','memory-card-line',`Sources: ${chunk.sources_changed === true ? '▲ a cited file changed' : chunk.sources_changed === false ? `${plural(chunk.sources,'file')} unchanged` : 'not tracked'} · selected in ${fmt.exact(model.selectedIn.get(chunk.id) || 0).text} of ${fmt.exact(model.selected.rows.length).text} retrievals${model.cutIn.get(chunk.id) ? ` · cut in ${fmt.exact(model.cutIn.get(chunk.id)).text}` : ''} · ${bytesLabel(chunk.bytes)}`));
  if (memoryUse.check?.skips?.[chunk.id]) card.append(el('p','memory-card-line',`Retrieval skips it: ${reasonLabel(memoryUse.check.skips[chunk.id])}`));
  const actions = el('div','memory-card-actions');
  for (const [label,action] of [['Open in Memory bank',null],['Re-attest…','bank-reverify'],['Retire…','bank-retire']]) { const button = el('button','button',label); button.type = 'button'; button.addEventListener('click',() => openMemoryAction(action,chunk)); actions.append(button); }
  card.append(actions); detail.replaceChildren(card); detail.setAttribute('aria-live',announce ? 'polite' : 'off');
}
function refreshMemoryFilters(rerenderTable = true) {
  const model = memoryUse.model; if (!model) return;
  for (const chip of document.querySelectorAll('.memory-chips .chip')) { const id = chip.dataset.filter; chip.setAttribute('aria-pressed',String(id === 'all' ? !memoryUse.filters.size : memoryUse.filters.has(id))); }
  for (const dot of document.querySelectorAll('.memory-dot')) { const chunk = memoryUse.data.chunks.items.find(item => item.id === dot.dataset.id); dot.dataset.dim = String(Boolean(memoryUse.filters.size) && !memoryMatches(chunk,model)); }
  if (rerenderTable) { const table = document.querySelector('.memory-chunk-table'); if (table) table.replaceWith(memoryChunkTable(model)); }
}

function memorySortable(kind, columns, rows, caption) {
  const [key,direction] = memoryUse.sort[kind], column = columns.find(([id]) => id === key) || columns[0];
  const sorted = [...rows].sort((a,b) => { const x = column[2](a), y = column[2](b); return (x === y ? 0 : x === null ? 1 : y === null ? -1 : x < y ? -1 : 1) * direction; });
  const table = el('table','memory-table'), head = el('tr'), body = el('tbody');
  table.append(el('caption','',caption));
  for (const [id,label] of columns) {
    const cell = el('th'); cell.scope = 'col'; cell.setAttribute('aria-sort',id === key ? direction > 0 ? 'ascending' : 'descending' : 'none');
    const button = el('button','memory-sort',label); button.type = 'button';
    button.addEventListener('click',() => { memoryUse.sort[kind] = [id,id === key ? -direction : 1]; const holder = table.closest('.memory-table-box'); holder.replaceWith(kind === 'chunks' ? memoryChunkTable(memoryUse.model) : memoryRetrievalTable(memoryUse.model)); document.querySelector(`.memory-table-box[data-kind="${kind}"] th[aria-sort]:not([aria-sort="none"]) button`)?.focus(); });
    cell.append(button); head.append(cell);
  }
  for (const row of sorted) { const line = el('tr'); for (const [, , , show] of columns) { const cell = el('td'); const value = show(row); cell.append(value instanceof Node ? value : document.createTextNode(value)); line.append(cell); } body.append(line); }
  const thead = el('thead'); thead.append(head); table.append(thead,body);
  const box = el('div','memory-table-box'); box.dataset.kind = kind; box.append(table); return box;
}
function memoryChunkTable(model) {
  const chunks = (memoryUse.data.chunks?.items || []).filter(chunk => !memoryUse.filters.size || chunk.status === 'active' && memoryMatches(chunk,model));
  const box = memorySortable('chunks',[
    ['id','ID',chunk => chunk.id,chunk => memoryChunkLink(chunk.id)],['title','Title',chunk => chunk.title || '',chunk => chunk.title || '—'],['type','Type',chunk => chunk.type || '',chunk => humanLabel(chunk.type) || '—'],
    ['provenance','Provenance',chunk => chunk.auto ? 1 : 0,chunk => chunk.auto ? chunk.last_verified > chunk.created ? '● auto, re-attested' : '○ auto' : '● person'],
    ['review','Review',chunk => chunk.status === 'active' ? model.review(chunk) : null,chunk => chunk.status === 'active' ? reviewLabel(model.review(chunk)) : chunk.status === 'needs-review' ? 'draft' : chunk.status || '—'],
    ['sources','Sources',chunk => chunk.sources_changed === true ? 1 : 0,chunk => chunk.sources_changed === true ? '▲ changed' : chunk.sources_changed === false ? 'unchanged' : '—'],
    ['selected','Selected',chunk => model.selectedIn.get(chunk.id) || 0,chunk => fmt.exact(model.selectedIn.get(chunk.id) || 0).text],['cut','Cut',chunk => model.cutIn.get(chunk.id) || 0,chunk => fmt.exact(model.cutIn.get(chunk.id) || 0).text],
    ['size','Size',chunk => chunk.bytes,chunk => bytesLabel(chunk.bytes)]],chunks,`${plural(chunks.length,'chunk')}${memoryUse.filters.size ? ' matching the filters' : ''}`);
  box.classList.add('memory-chunk-table'); return box;
}
function memoryRetrievalTable(model) {
  return memorySortable('retrievals',[['time','Time',row => row.at,row => memoryTime(row.at)],['route','Route',row => row.route,row => row.route],
    ...memoryKinds.map(([kind,label]) => [kind,label,row => row[kind],row => fmt.exact(row[kind]).text]),['cut','Cut',row => row.cuts.length,row => fmt.exact(row.cuts.length).text]],model.selected.rows,`${plural(model.selected.rows.length,'retrieval')}`);
}
function memoryTables(model) {
  const box = el('div','memory-tables');
  const chunks = el('details','fleet-report'); chunks.append(el('summary','',`Chunks as a table (${fmt.exact(memoryUse.data.chunks?.items.length || 0).text})`)); chunks.addEventListener('toggle',() => { if (chunks.open && !chunks.querySelector('table')) chunks.append(memoryChunkTable(memoryUse.model)); });
  const retrievals = el('details','fleet-report'); retrievals.append(el('summary','',`Retrievals as a table (${fmt.exact(model.selected.rows.length).text})`)); retrievals.addEventListener('toggle',() => { if (retrievals.open && !retrievals.querySelector('table')) retrievals.append(memoryRetrievalTable(memoryUse.model)); });
  box.append(chunks,retrievals); return box;
}
function memoryKey() {
  const key = el('p','memory-key'); key.append('Key: band thickness counts items moved, in fixed steps · ');
  for (const [kind,label] of [['person','person'],['auto','auto-promoted'],['cut','cut at retrieval']]) { const swatch = el('span','memory-swatch'); swatch.dataset.kind = kind; key.append(swatch,` ${label} · `); }
  key.append('purple marks what is new since your last visit.'); return key;
}

// Motion only replays real change since the stored visit; every final state is drawn first, so reduced motion loses nothing.
function playMemoryChanges(changes) {
  const fast = motionToken('--motion-fast'), slow = motionToken('--motion-slow'), travel = motionToken('--motion-travel'), base = motionToken('--motion-base');
  const enter = easingToken('--ease-enter'), standard = easingToken('--ease-standard');
  const fresh = [...document.querySelectorAll('.memory-column[data-new="true"]')], step = fresh.length ? Math.min(motionToken('--motion-stagger'),400 / fresh.length) : 0;
  fresh.forEach((column,index) => column.animate([{opacity:0},{opacity:1}],{duration:fast,delay:index * step,easing:enter,fill:'backwards'}));
  document.querySelector('.memory-strip-bracket')?.animate([{transform:'scaleX(0)'},{transform:'scaleX(1)'}],{duration:slow,delay:fresh.length * step,easing:standard,fill:'backwards'});
  for (const chunk of changes.added) document.querySelector(`.memory-dot[data-id="${CSS.escape(chunk.id)}"]`)?.animate([{opacity:0,transform:'scale(.6)'},{opacity:1,transform:'scale(1)'}],{duration:base,easing:enter,fill:'backwards'});
  // A re-attested chunk slides from where its old review date put it; one crossing its date slides into the past zone.
  const chart = document.querySelector('.memory-horizon'), anchor = memoryUse.anchor;
  if (chart && anchor?.today) for (const chunk of new Set([...changes.moved,...changes.crossed])) {
    const dot = document.querySelector(`.memory-dot[data-id="${CSS.escape(chunk.id)}"]`); if (!dot) continue;
    const before = Math.max(-30,Math.min(365,daysBetween(anchor.today,anchor.chunks[chunk.id][2]))), after = Math.max(-30,Math.min(365,memoryUse.model.review(chunk)));
    const shift = (before - after) / 395 * chart.getBoundingClientRect().width; if (Math.abs(shift) > 1) dot.animate([{transform:`translateX(${shift}px)`},{transform:'none'}],{duration:slow,easing:standard,fill:'backwards'});
  }
  playMemoryPromotions(changes.applied.slice(-5),travel,slow,standard);
}
function playMemoryPromotions(applied, travel, leg, easing) {
  const flow = document.querySelector('.memory-flow-section'), stage = name => flow?.querySelector(`[data-stage="${name}"] .memory-lead`); if (!flow || !applied.length) return;
  const from = stage('brain')?.getBoundingClientRect(), via = stage('promotion')?.getBoundingClientRect(), to = stage('bank')?.getBoundingClientRect(), box = flow.getBoundingClientRect();
  if (!from || !to || from.bottom < 0 || to.top > innerHeight) return;
  const layer = el('div','memory-travel'); layer.setAttribute('aria-hidden','true'); flow.append(layer);
  const point = rect => [rect.left + 8 - box.left, rect.top + rect.height / 2 - box.top];
  const done = applied.map((item,index) => {
    const dot = el('span','memory-travel-dot'); dot.dataset.kind = item.mode === 'automatic' ? 'auto' : 'person'; layer.append(dot);
    const [x0,y0] = point(from), [x2,y2] = point(to), [x1,y1] = via ? point(via) : [(x0 + x2) / 2,y0];
    const frames = item.mode === 'automatic' ? [{transform:`translate(${x0}px,${y0}px)`,opacity:0},{opacity:1,offset:.1},{opacity:1,offset:.9},{transform:`translate(${x2}px,${y2}px)`,opacity:0}]
      : [{transform:`translate(${x0}px,${y0}px)`,opacity:0},{opacity:1,offset:.05},{transform:`translate(${x1}px,${y1}px)`,opacity:1,offset:.42},{transform:`translate(${x1}px,${y1}px)`,opacity:1,offset:.58},{transform:`translate(${x2}px,${y2}px)`,opacity:0}];
    // A person's promotion stops at review; an automatic one travels in one leg because nobody reviewed it.
    const animation = dot.animate(frames,{duration:item.mode === 'automatic' ? travel : leg * 2 + 150,delay:index * 60,easing,fill:'both'});
    return animation.finished.catch(() => {});
  });
  Promise.race([Promise.all(done),new Promise(resolve => setTimeout(resolve,travel * 3 + 400))]).then(() => layer.remove());
}

$('memory-use-project').addEventListener('change',() => loadMemoryUse());
$('memory-use-bank').addEventListener('change',() => loadMemoryUse({bank:$('memory-use-bank').value}));
$('memory-use-refresh').addEventListener('click',() => loadMemoryUse({quiet:true}));
for (const [days,label] of memoryWindows) {
  const chip = el('button','chip',label); chip.type = 'button'; chip.setAttribute('role','radio'); chip.setAttribute('aria-checked',String(days === memoryUse.window)); chip.tabIndex = days === memoryUse.window ? 0 : -1;
  chip.addEventListener('click',() => { memoryUse.window = days; for (const other of $('memory-use-window').children) { const on = other === chip; other.setAttribute('aria-checked',String(on)); other.tabIndex = on ? 0 : -1; } renderMemoryUse(); });
  $('memory-use-window').append(chip);
}
$('memory-use-window').addEventListener('keydown',event => {
  const chips = [...$('memory-use-window').children], current = chips.indexOf(document.activeElement), next = {ArrowRight:current + 1,ArrowDown:current + 1,ArrowLeft:current - 1,ArrowUp:current - 1}[event.key];
  if (current < 0 || next === undefined) return; event.preventDefault(); const target = chips[(next + chips.length) % chips.length]; target.focus(); target.click();
});
document.addEventListener('visibilitychange',() => { if (document.hidden && state.view === 'memory-use') moveMemoryAnchor(); else if (!document.hidden && state.view === 'memory-use' && memoryUse.data) memoryUse.visibleSince = performance.now(); });
memoryPhone.addEventListener('change',() => { if (state.view === 'memory-use' && memoryUse.data) renderMemoryUse(); });
