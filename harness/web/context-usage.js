// Sessions › Usage › Context: how full the agent's context window is, how much of it is memory the Harness
// sent, how the turn grew it and when compaction cleared it. Classic script loaded after app-core.js.
// Fill, window, growth and free space are the provider's exact numbers; only memory parts are estimates.
'use strict';
const contextUi = {session:null,selected:null,follow:true,page:0,open:new Set(),announced:null,key:null};
// Estimator ratios, calibrated to cl100k: capsule JSON and hook text, then prose (scripts/context_budget.py).
const CAPSULE_RATIO = 3.6, PROSE_RATIO = 4.7;
const contextParts = [['brain','Project Brain'],['bank','Memory bank'],['rules','Rules & docs'],['rest','Everything else at turn start'],['added','Added this turn'],['free','Free']];
const contextPhone = window.matchMedia('(max-width:760px)');
const percentText = (part, whole, estimated = false) => {
  if (!Number.isFinite(part) || !Number.isFinite(whole) || whole <= 0) return null;
  const ratio = part / whole; if (ratio > 0 && ratio < .005) return `${estimated ? '≈ ' : ''}<1%`;
  return `${estimated ? '≈ ' : ''}${Math.floor(ratio * 100 + .5)}%`;
};

// One native launch as a turn: exact provider numbers next to the ledger's estimates.
function contextTurn(launch, ordinal) {
  const context = launch.context, ledger = context?.ledger, fill = context?.fill || {}, capsule = ledger?.capsule, excerpts = ledger?.excerpts || [];
  const kinds = capsule?.kinds || {brain:0,bank:0,rules:0}, excerptCharacters = excerpts.reduce((sum,item) => sum + item.characters,0);
  const characters = {brain:kinds.brain, bank:kinds.bank, rules:kinds.rules + excerptCharacters};
  const parts = {brain:kinds.brain / CAPSULE_RATIO, bank:kinds.bank / CAPSULE_RATIO, rules:kinds.rules / CAPSULE_RATIO + excerptCharacters / PROSE_RATIO};
  const memory = parts.brain + parts.bank + parts.rules, int = value => Number.isInteger(value) ? value : null;
  const start = int(fill.start), end = int(fill.end), window = int(fill.window), compactions = Array.isArray(fill.compactions) ? fill.compactions : [];
  return {id:launch.id, ordinal, at:launch.started_at, running:launch.status === 'running', recorded:Boolean(ledger), provider:context?.provider || launch.settings?.provider,
    ledger, capsule, excerpts, characters, parts, memory, start, end, window, peak:int(fill.peak), calls:int(fill.calls), cache:Number.isFinite(fill.cache_share) ? fill.cache_share : null,
    compactions, compacted:compactions.length > 0, hooks:context?.hooks || null, hookParts:fill.hooks || null, cliFiles:context?.cli_files || [],
    instructions:ledger ? (ledger.instructions + ledger.message + (ledger.attachments?.characters || 0)) / PROSE_RATIO : null,
    // The remainder is exact start minus estimated memory, floored at 0; unknown when memory exceeds the fill.
    rest:start === null ? null : memory > start ? null : start - memory,
    added:start === null || end === null || compactions.length ? null : end - start, free:window === null || end === null ? null : window - end,
    sent:ledger ? (capsule?.inserted || 0) + excerptCharacters : null};
}
function contextTurns(launches) { return launches.filter(launch => launch.kind === 'native').map((launch,index) => contextTurn(launch,index + 1)); }
// Memory sent by earlier turns may still sit in the conversation; the bound resets at a compaction.
function earlierMemory(turns, index) {
  let total = 0;
  for (let position = index - 1; position >= 0; position--) {
    const turn = turns[position]; if (turn.compacted) break;
    if (!turn.recorded) return {unknown:turn.ordinal};
    total += turn.memory;
  }
  return {total,from:index > 0 ? turns[index - 1].ordinal : null};
}

function renderContextUsage(data) {
  const box = $('context-usage'), turns = contextTurns(data?.launches || []);
  if (contextUi.session !== state.selectedId) Object.assign(contextUi,{session:state.selectedId,selected:null,follow:true,page:0,open:new Set(),key:null});
  box.hidden = !turns.length; if (!turns.length) return;
  if (contextUi.follow || !turns.some(turn => turn.id === contextUi.selected)) contextUi.selected = turns.at(-1).id;
  const index = turns.findIndex(turn => turn.id === contextUi.selected), turn = turns[index];
  const key = JSON.stringify([turns.map(item => [item.id,item.end,item.window,item.calls,item.running,item.compactions.length,item.cache,item.recorded]),contextUi.selected,[...contextUi.open],contextUi.page,contextPhone.matches]);
  if (key === contextUi.key) return; contextUi.key = key;
  if (!box.firstChild) buildContextUsage(box);
  $('context-turn-label').textContent = `Turn ${turn.ordinal} · ${memoryClock.time.format(new Date(turn.at))}${turn.running ? ' · running' : ''}`;
  $('context-latest').hidden = contextUi.follow || index === turns.length - 1;
  renderContextHeadline(turn); renderContextBar(turn); renderContextLegend(turn,turns,index); renderContextTurns(turns); renderContextTable(turns);
  // One polite line, once, when a turn crosses 80% of its window or compacts.
  const alert = turn.compacted ? `Turn ${turn.ordinal} compacted its context.` : turn.end !== null && turn.window && turn.end / turn.window >= .8 ? `Turn ${turn.ordinal} filled ${percentText(turn.end,turn.window)} of the context window.` : '';
  if (alert && contextUi.announced !== `${turn.id}:${alert}`) { contextUi.announced = `${turn.id}:${alert}`; $('context-status').textContent = alert; }
}
function buildContextUsage(box) {
  const head = el('div','context-head'), title = el('h3','','Context'); head.append(title,el('span','context-turn-label'));
  head.lastChild.id = 'context-turn-label';
  const latest = el('button','chip','Latest'); latest.type = 'button'; latest.id = 'context-latest'; latest.hidden = true; latest.addEventListener('click',() => { contextUi.follow = true; renderContextUsage(resultUi.data); });
  const explain = el('button','explain','?'); explain.type = 'button'; explain.setAttribute('aria-expanded','false'); explain.setAttribute('aria-controls','context-help'); explain.setAttribute('aria-label','About context');
  head.append(latest,explain);
  const help = el('p','knowledge-note','Fill, window, growth and free space are the provider’s own token counts. Memory parts are estimates from characters the Harness sent: capsule JSON at 3.6 characters per token, prose at 4.7. Claude may count 15–35% more. A part that is not zero is drawn at least 2px wide. The CLI compacts before the window is full. Cursor reports no usage.');
  help.id = 'context-help'; help.hidden = true;
  const headline = el('p','context-headline'); headline.id = 'context-headline';
  const lines = el('div','context-lines'); lines.id = 'context-lines';
  const bar = el('div','context-bar'); bar.id = 'context-bar'; bar.setAttribute('role','img');
  for (const [kind] of contextParts.slice(0,5)) { const part = el('span','context-part'); part.dataset.kind = kind; bar.append(part); }
  const peak = el('span','context-peak'); peak.hidden = true; bar.append(peak);
  const scale = el('div','context-scale'); scale.id = 'context-scale'; scale.setAttribute('aria-hidden','true');
  const legend = el('table','context-legend'); legend.id = 'context-legend';
  const status = el('p','sr-only'); status.id = 'context-status'; status.setAttribute('role','status');
  const turnsHead = el('div','context-turns-head'); turnsHead.append(el('h4','','Turns'));
  for (const [id,label,step] of [['context-earlier','‹ Earlier',1],['context-newer','Newer ›',-1]]) {
    const pager = el('button','button quiet',label); pager.type = 'button'; pager.id = id; pager.hidden = true;
    pager.addEventListener('click',() => { contextUi.page = Math.max(0,contextUi.page + step); renderContextUsage(resultUi.data); }); turnsHead.append(pager);
  }
  const strip = el('div','context-turns'); strip.id = 'context-turns'; strip.setAttribute('role','radiogroup'); strip.setAttribute('aria-label','Turns');
  strip.addEventListener('keydown',contextTurnKeys);
  const table = el('details','fleet-report context-turn-table'); table.append(el('summary','','Turn table')); table.id = 'context-turn-table';
  box.append(head,help,headline,lines,bar,scale,legend,turnsHead,strip,table,status);
}
function renderContextHeadline(turn) {
  const headline = $('context-headline'), lines = $('context-lines'), exact = value => fmt.exact(value).text, estimate = value => fmt.estimate(value,' tokens');
  if (turn.end !== null && turn.window) headline.textContent = `${exact(turn.end)} of ${exact(turn.window)} tokens in context · ${percentText(turn.end,turn.window)}`;
  else if (turn.end !== null) headline.textContent = `${exact(turn.end)} tokens in context · window not reported`;
  else headline.textContent = turn.running ? turn.provider === 'codex' ? 'Context size arrives when the turn ends.' : 'Waiting for the first model call.'
    : turn.recorded ? 'Context size not reported by this provider.' : 'Context details start with the next launch.';
  const rows = [];
  if (turn.recorded) {
    const memory = el('p','context-line'); memory.append('Memory sent this turn ',numberNode(estimate(turn.memory)));
    // The memory share is of what is in context, not of the window.
    if (turn.end) memory.append(` (${percentText(turn.memory,turn.end,true)})`);
    if (turn.added !== null) memory.append(` · +${exact(turn.added)} this turn`);
    rows.push(memory);
    if (turn.end === null && turn.sent !== null) rows.push(el('p','context-line',`Added by the Harness: ${exact(turn.ledger.total)} characters`));
  }
  if (turn.calls) rows.push(el('p','context-line',`Sent with each of ${plural(turn.calls,'model call')}${turn.cache !== null ? ` · ${percentText(turn.cache,1)} read from cache` : ''}`));
  for (const compaction of turn.compactions) rows.push(el('p','context-line warning',`Compacted during this turn: ${exact(compaction.pre)} → ${compaction.post === null ? '—' : exact(compaction.post)}${turn.peak && turn.window ? ` · peak ${percentText(turn.peak,turn.window)}` : ''}`));
  lines.replaceChildren(...rows);
}
function contextGeometry(turn) {
  // Shares of the window in bar order; a compacted turn draws only its final fill, unsplit.
  if (turn.end === null || !turn.window) return null;
  if (turn.compacted) return {added:turn.end};
  const memory = turn.rest === null ? 0 : turn.memory;
  return {brain:memory ? turn.parts.brain : 0, bank:memory ? turn.parts.bank : 0, rules:memory ? turn.parts.rules : 0, rest:turn.rest ?? turn.start, added:turn.added ?? 0};
}
function renderContextBar(turn) {
  const bar = $('context-bar'), geometry = contextGeometry(turn), width = bar.getBoundingClientRect().width || 600;
  for (const part of bar.querySelectorAll('.context-part')) {
    const value = geometry?.[part.dataset.kind] || 0, share = turn.window ? value / turn.window : 0;
    part.hidden = !value; part.style.width = value ? `${Math.max(share * 100,2 / width * 100)}%` : '0';
    part.dataset.compacted = String(turn.compacted && part.dataset.kind === 'added');
  }
  const peak = bar.querySelector('.context-peak'); peak.hidden = !(turn.compacted && turn.peak && turn.window); if (!peak.hidden) peak.style.left = `${Math.min(100,turn.peak / turn.window * 100)}%`;
  const about = value => fmt.estimate(value).label;
  bar.setAttribute('aria-label',geometry ? `Turn ${turn.ordinal}: ${fmt.exact(turn.end).text} of ${fmt.exact(turn.window).text} tokens in context.${turn.compacted ? ` Compacted during the turn; peak ${fmt.exact(turn.peak).text}.` :
    ` Memory ${about(turn.memory)}: Project Brain ${about(turn.parts.brain)}, Memory bank ${about(turn.parts.bank)}, Rules and docs ${about(turn.parts.rules)}. Everything else at turn start ${turn.rest === null ? 'not known' : about(turn.rest)}. Added this turn ${fmt.exact(turn.added).text}.`} Free ${fmt.exact(turn.free).text}.`
    : `Turn ${turn.ordinal}: context size not reported.`);
  bar.hidden = !geometry; $('context-scale').hidden = !geometry;
  if (geometry) $('context-scale').replaceChildren(el('span','','0'),el('span','',`${fmt.exact(turn.window).text} window`));
}
function contextRow(kind, label, detail, tokens, characters, expandable) {
  const row = el('tr'); row.dataset.kind = kind;
  const name = el('th'); name.scope = 'row'; const swatch = el('span','context-swatch'); swatch.dataset.kind = kind; swatch.setAttribute('aria-hidden','true');
  name.append(swatch,label); row.append(name,el('td','context-detail',detail));
  const value = el('td','context-number'); value.append(numberNode(tokens)); row.append(value,el('td','context-number',characters === null ? '' : fmt.exact(characters).text));
  const toggle = el('td');
  if (expandable) { const button = el('button','context-expand','›'); button.type = 'button'; button.setAttribute('aria-expanded',String(contextUi.open.has(kind))); button.setAttribute('aria-label',`Details: ${label}`);
    button.addEventListener('click',() => { if (contextUi.open.has(kind)) contextUi.open.delete(kind); else contextUi.open.add(kind); renderContextUsage(resultUi.data); document.querySelector(`#context-legend tr[data-kind="${kind}"] .context-expand`)?.focus(); }); toggle.append(button); }
  row.append(toggle);
  for (const type of ['pointerenter','focusin']) row.addEventListener(type,() => highlightContextPart(kind)); for (const type of ['pointerleave','focusout']) row.addEventListener(type,() => highlightContextPart(null));
  return row;
}
function highlightContextPart(kind) { for (const part of document.querySelectorAll('#context-bar .context-part')) part.dataset.dim = String(Boolean(kind) && part.dataset.kind !== kind); }
function contextExpansion(kind, nodes) { const row = el('tr','context-expansion'); const cell = el('td'); cell.colSpan = 5; cell.append(...nodes); row.append(cell); row.dataset.expands = kind; return row; }
function renderContextLegend(turn, turns, index) {
  const legend = $('context-legend'), body = el('tbody'), estimate = value => value === null ? fmt.unknown() : fmt.estimate(value,' tokens'), exact = value => value === null ? fmt.unknown() : fmt.exact(value,' tokens');
  const head = el('thead'), header = el('tr'); for (const label of ['Part','','≈ tokens','Characters','']) { const cell = el('th','',label); cell.scope = 'col'; header.append(cell); } head.append(header);
  const capsule = turn.capsule, files = turn.excerpts.length, chunks = capsule?.items?.bank || 0;
  const rows = [['brain','Project Brain',capsule ? 'task + capsule' : 'none this turn',estimate(turn.recorded ? turn.parts.brain : null),turn.recorded ? turn.characters.brain : null,Boolean(capsule)],
    ['bank','Memory bank',plural(chunks,'chunk'),estimate(turn.recorded ? turn.parts.bank : null),turn.recorded ? turn.characters.bank : null,Boolean(capsule)],
    ['rules','Rules & docs',[capsule?.items?.rules ? plural(capsule.items.rules,'capsule item') : '',files ? plural(files,'file') : ''].filter(Boolean).join(' · ') || 'none this turn',estimate(turn.recorded ? turn.parts.rules : null),turn.recorded ? turn.characters.rules : null,Boolean(files || capsule)],
    ['rest','Everything else at turn start','CLI prompt, tools, rules it loads, conversation',estimate(turn.rest),null,true],
    ['added','Added this turn',turn.compacted ? 'compacted during the turn' : 'tool results and replies',turn.compacted ? fmt.exact(turn.end,' tokens') : exact(turn.added),null,false],
    ['free','Free',turn.window ? `of ${fmt.exact(turn.window).text}` : 'window not reported',exact(turn.free),null,false]];
  body.append(el('tr','context-group'));
  body.lastChild.append(Object.assign(el('th','','Memory'),{colSpan:5,scope:'rowgroup'}));
  rows.forEach(([kind,label,detail,tokens,characters,expandable],position) => {
    if (position === 3) { const group = el('tr','context-group'); group.append(Object.assign(el('th','','Not memory'),{colSpan:5,scope:'rowgroup'})); body.append(group); }
    body.append(contextRow(kind,label,detail,tokens,characters,expandable));
    if (expandable && contextUi.open.has(kind)) body.append(contextExpansion(kind,contextDetails(kind,turn,turns,index)));
  });
  const caption = el('caption','sr-only',`Context parts of turn ${turn.ordinal}`);
  legend.replaceChildren(caption,head,body);
}
function contextDetails(kind, turn, turns, index) {
  const exact = value => fmt.exact(value).text, nodes = [];
  if (kind === 'brain' && turn.capsule) nodes.push(el('p','knowledge-note',`The working task, last turn, handoffs, Brain records and the capsule’s own fields: ${exact(turn.characters.brain)} characters.`));
  if (kind === 'bank' && turn.capsule) {
    const capsule = turn.capsule, bar = el('div','capsule-bar'); bar.setAttribute('role','img');
    for (const [part] of capsuleKinds) { const node = el('span','capsule-part'); node.dataset.kind = part; const share = capsule.kinds[part] / capsule.inserted * capsule.characters / 8000; node.style.width = `${Math.min(100,share * 100)}%`; node.hidden = !capsule.kinds[part]; bar.append(node); }
    bar.setAttribute('aria-label',`Capsule ${exact(capsule.characters)} of 8,000 characters`);
    const dropped = Object.entries(capsule.dropped || {}).map(([layer,count]) => `${plural(count,`${layer} item`)} dropped`);
    nodes.push(bar,el('p','knowledge-note',[`${exact(capsule.characters)} of 8,000 characters`,...dropped,capsule.repeats ? `${exact(capsule.repeats)} repeat` : ''].filter(Boolean).join(' · ')));
  }
  if (kind === 'rules') {
    if (turn.capsule?.kinds.rules) nodes.push(el('p','knowledge-note',`Capsule rules and docs: ${exact(turn.capsule.kinds.rules)} characters.`));
    const largest = Math.max(1,...turn.excerpts.map(item => item.full));
    for (const item of turn.excerpts) {
      const row = el('div','context-excerpt'), track = el('span','context-excerpt-track'), sent = el('span','context-excerpt-sent'), cap = el('span','context-excerpt-cap');
      track.style.width = `${item.full / largest * 100}%`; sent.style.width = `${item.sent / item.full * 100}%`; cap.style.left = `${Math.min(100,(bootstrapExcerptBytes() / item.full) * 100)}%`; cap.hidden = item.full <= bootstrapExcerptBytes();
      track.append(sent,cap); track.setAttribute('aria-hidden','true');
      row.append(el('span','context-excerpt-name',item.name),el('span','context-number',`${exact(item.sent)} of ${exact(item.full)}`),track); nodes.push(row);
    }
  }
  if (kind === 'rest') {
    const earlier = earlierMemory(turns,index), sub = [];
    if (turn.instructions !== null) { const line = el('p','knowledge-note'); line.append('Harness instructions and message ',numberNode(fmt.estimate(turn.instructions,' tokens'))); sub.push(line); }
    sub.push(el('p','knowledge-note',earlier.unknown ? `Earlier turns’ memory: — (turn ${earlier.unknown} not recorded)` : earlier.total ? `Earlier turns’ memory ${fmt.atMost(Number(earlier.total.toPrecision(2)),' tokens').text} (turn ${earlier.from})` : index ? 'Earlier turns’ memory: none since the last compaction' : 'Earlier turns’ memory: none, this is the first turn'));
    if (turn.cliFiles.length) sub.push(el('p','knowledge-note',`Rules the CLI loads itself: ${turn.cliFiles.map(file => `${file.name} ${bytesLabel(file.bytes)}`).join(' · ')}`));
    const hooks = turn.hooks;
    // Measured hook output wins: hooks can come from the user's own settings rather than the project's.
    const hookTotal = turn.hookParts ? Object.values(turn.hookParts).reduce((sum,value) => sum + value,0) : 0;
    sub.push(el('p','knowledge-note',hookTotal ? `Hook memory ${fmt.estimate(hookTotal / CAPSULE_RATIO,' tokens').text}: Project Brain ${exact(turn.hookParts.brain)}, Memory bank ${exact(turn.hookParts.bank)}, Rules & docs ${exact(turn.hookParts.rules)} characters`
      : Number.isInteger(hooks?.bytes) ? `Hook memory: ${bytesLabel(hooks.bytes)} rule file at launch` : !hooks?.installed ? 'Hook memory: no memory hooks in this project'
      : hooks.measured ? 'Hook memory: no hook output recorded this turn' : 'Hook memory: not measured'));
    nodes.push(...sub);
  }
  return nodes;
}
const bootstrapExcerptBytes = () => state.bootstrap?.runtime?.context_excerpt_bytes || 3000;
function renderContextTurns(turns) {
  const size = contextPhone.matches ? 6 : 12, pages = Math.max(0,Math.ceil(turns.length / size) - 1); contextUi.page = Math.min(contextUi.page,pages);
  const end = turns.length - contextUi.page * size, shown = turns.slice(Math.max(0,end - size),end);
  $('context-earlier').hidden = end - size <= 0; $('context-newer').hidden = !contextUi.page;
  keyedRender($('context-turns'),shown,turn => turn.id,turn => {
    const column = el('button','context-turn'); column.type = 'button'; column.setAttribute('role','radio'); column.dataset.id = turn.id;
    const track = el('span','context-turn-track'); track.setAttribute('aria-hidden','true');
    const geometry = contextGeometry(turn);
    if (geometry) for (const [kind] of contextParts.slice(0,5)) { const value = geometry[kind] || 0; if (!value) continue; const part = el('span','context-turn-part'); part.dataset.kind = kind; part.style.height = `${Math.max(1,value / turn.window * 64)}px`; track.append(part); }
    else track.append(el('span','context-turn-unknown','—'));
    if (turn.compacted && turn.peak && turn.window) { const peak = el('span','context-turn-peak'); peak.style.bottom = `${Math.min(64,turn.peak / turn.window * 64)}px`; track.append(peak); }
    const label = el('span','context-turn-label',geometry ? percentText(turn.end,turn.window) : '—');
    column.append(track,label,el('span','context-turn-ordinal',`${turn.ordinal}${turn.compacted ? ' c' : ''}${turn.running ? ' running' : ''}`));
    column.dataset.running = String(turn.running);
    column.setAttribute('aria-label',`Turn ${turn.ordinal}, ${memoryClock.time.format(new Date(turn.at))}, ${geometry ? `${percentText(turn.end,turn.window)} of the window${turn.compacted ? ` after compaction from ${fmt.exact(turn.compactions[0].pre).text}` : ''}` : 'not recorded'}${turn.running ? ', running' : ''}`);
    column.addEventListener('click',() => selectContextTurn(turn.id));
    return column;
  },turn => JSON.stringify([turn.end,turn.window,turn.running,turn.compactions.length,turn.recorded,turn.memory]));
  for (const column of $('context-turns').children) { const on = column.dataset.id === contextUi.selected; column.setAttribute('aria-checked',String(on)); column.tabIndex = on ? 0 : -1; }
}
function selectContextTurn(id) { const turns = contextTurns(resultUi.data?.launches || []); contextUi.selected = id; contextUi.follow = id === turns.at(-1)?.id; renderContextUsage(resultUi.data); }
function contextTurnKeys(event) {
  const columns = [...$('context-turns').children], current = columns.indexOf(document.activeElement); if (current < 0) return;
  const next = {ArrowLeft:current - 1,ArrowRight:current + 1,Home:0,End:columns.length - 1}[event.key]; if (next === undefined) return;
  event.preventDefault(); const target = columns[Math.max(0,Math.min(columns.length - 1,next))]; selectContextTurn(target.dataset.id); $('context-turns').querySelector(`[data-id="${CSS.escape(target.dataset.id)}"]`)?.focus();
}
function renderContextTable(turns) {
  const box = $('context-turn-table'), exact = value => value === null ? '—' : fmt.exact(value).text, table = el('table','memory-table'), head = el('tr'), body = el('tbody');
  table.append(el('caption','sr-only','Every turn'));
  for (const label of ['Turn','Started','Calls','Start','End','Peak','Window','Fill','Memory ≈','Added','Compaction','Cache']) { const cell = el('th','',label); cell.scope = 'col'; head.append(cell); }
  for (const turn of turns) {
    const row = el('tr');
    for (const value of [String(turn.ordinal),memoryTime(turn.at),exact(turn.calls),exact(turn.start),exact(turn.end),exact(turn.peak),exact(turn.window),percentText(turn.end,turn.window) || '—',
      turn.recorded ? fmt.estimate(turn.memory).text : '—',exact(turn.added),turn.compactions.map(item => `${exact(item.pre)} → ${exact(item.post)}`).join(', ') || '—',turn.cache === null ? '—' : percentText(turn.cache,1)]) row.append(el('td','',value));
    body.append(row);
  }
  const thead = el('thead'); thead.append(head); table.append(thead,body);
  box.querySelector('table')?.remove(); box.append(table);
}
// Fleet and Clash send one prefix to every agent; their launch rows say how much memory that was, and to how many.
function contextLaunchLine(run) {
  if (!['fleet','clash'].includes(run.kind) || !run.context?.ledger) return null;
  const turn = contextTurn(run,0); return el('p','knowledge-note',`Memory prefix ${fmt.estimate(turn.memory).text} sent to ${plural(run.context.agents || 1,'agent')}`);
}
// The composer meter appears only when the latest native turn reached half its window, or compacted.
function renderContextMeter() {
  const meter = $('context-meter'), last = state.selected?.context_last; if (!meter) return;
  const ratio = last && last.window ? last.fill / last.window : null, show = Boolean(last && last.window && (ratio >= .5 || last.compacted));
  meter.hidden = !show; if (!show) return;
  const bar = $('context-meter-bar'); bar.max = last.window; bar.value = Math.min(last.fill,last.window); bar.low = last.window * .5; bar.high = last.window * .8; bar.optimum = 0;
  bar.setAttribute('aria-valuetext',`${fmt.exact(last.fill).text} of ${fmt.exact(last.window).text} tokens`);
  const percent = percentText(last.fill,last.window), full = ratio >= .8;
  $('context-meter-text').textContent = `${full ? '▲ ' : ''}${percent}${last.compacted ? ' · compacted' : full ? ' · nearly full' : ''}`;
  meter.dataset.full = String(full); meter.setAttribute('aria-label',`Context ${percent} full${last.compacted ? ', compacted' : ''}. Open usage.`);
}
$('context-meter').addEventListener('click',() => { contextUi.follow = true; contextUi.selected = null; openView('usage'); });
contextPhone.addEventListener('change',() => { contextUi.key = null; if (state.view === 'usage' && resultUi.data) renderContextUsage(resultUi.data); });
