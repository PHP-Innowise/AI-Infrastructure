/* Live agent cards and timelines from display-only session events.
   Agent output is untrusted: every value is rendered as text. */
const AGENT_STATUS = {pending:'Waiting', running:'Working', completed:'Completed', blocked:'Blocked', interrupted:'Stopped'};
const AGENT_KINDS = {tool:'Tool', text:'Says', thinking:'Reasons', plan:'Plan', error:'Error', status:'Note'};
function agentClock(value) {
  const date=new Date(value); return Number.isNaN(date.getTime()) ? '' : date.toLocaleTimeString([], {hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false});
}
function agentCount(count,noun) { return count ? `${count} ${noun}${count===1 ? '' : 's'}` : ''; }
function agentDuration(seconds) {
  if(!Number.isFinite(seconds) || seconds<0) return '';
  const whole=Math.round(seconds), minutes=Math.floor(whole/60);
  return minutes ? `${minutes}m ${String(whole%60).padStart(2,'0')}s` : `${whole}s`;
}
function agentTokens(value) {
  return Number.isFinite(value) && value>0 ? (value>=1000 ? `${(value/1000).toFixed(value>=100000?0:1)}k` : String(value))+' tokens' : '';
}
function createAgentPanel(root, {emptyText='No agent has started yet.', labels={}}={}) {
  const panel={root, sessionId:null, cursor:0, epoch:0, plan:[], agents:new Map(), order:[], selected:null, follow:true, active:false, renderedKey:'', pending:null};
  const cards=el('div','agent-cards'), timeline=el('div','agent-timeline'), heading=el('div','agent-timeline-heading'), list=el('ol','agent-events');
  const follow=el('label','checkbox agent-follow'), followInput=document.createElement('input');
  followInput.type='checkbox'; followInput.checked=true; follow.append(followInput,document.createTextNode('Follow the active agent'));
  timeline.append(heading,list); root.replaceChildren(follow,cards,timeline);
  timeline.setAttribute('aria-live','polite');
  followInput.addEventListener('change',()=>{panel.follow=followInput.checked; render();});

  function key(agent,attempt) { return `${agent}#${attempt}`; }
  function entry(agent,attempt) {
    const id=key(agent,attempt);
    if(!panel.agents.has(id)) { panel.agents.set(id,{id,agent,attempt,status:'running',events:[],calls:new Map(),tools:0,messages:0,tokens:0}); panel.order.push(id); }
    return panel.agents.get(id);
  }
  function observe(event) {
    if(typeof event.agent!=='string' || !Number.isInteger(event.attempt)) return;
    const agent=entry(event.agent,event.attempt);
    if(event.kind==='agent') {
      // Lifecycle lists are trimmed by the runner; *_count carries the full size.
      if(event.status==='running') Object.assign(agent,{status:'running',started:event.at,service:event.service,mode:event.mode,provider:event.provider,root:event.root,readable:event.readable||[],writable:event.writable||[],readableCount:event.readable_count,writableCount:event.writable_count});
      else Object.assign(agent,{status:event.status,finished:event.at,ok:event.ok,error:event.error,duration:event.duration_seconds,summary:event.summary,changed:event.changed_files||[],changedCount:event.changed_files_count,checks:event.checks,reason:event.reason,tokens:Number.isFinite(event.tokens)?event.tokens:event.tokens===null?null:agent.tokens,cost:event.cost_usd});
    } else if(event.kind==='usage') {
      const tokens=['input_tokens','output_tokens','cache_read_input_tokens','cache_creation_input_tokens'].reduce((sum,name)=>sum+(Number.isFinite(event[name])?event[name]:0),0);
      if(!agent.finished) agent.tokens+=tokens;
    } else if(event.kind==='agent_activity') {
      if(event.type==='tool' && typeof event.call==='string' && agent.calls.has(event.call) && event.state==='completed') {
        const started=agent.calls.get(event.call); started.state='completed'; started.ok=event.ok; started.done=event.at; return;
      }
      const item={...event}; agent.events.push(item);
      if(event.type==='tool') { agent.tools+=1; if(typeof event.call==='string') agent.calls.set(event.call,item); }
      if(event.type==='text') agent.messages+=1;
      agent.last=item;
    }
  }
  function agentsInOrder() {
    const seen=new Set(), result=[];
    for(const step of panel.plan) {
      const attempts=panel.order.map(id=>panel.agents.get(id)).filter(a=>a.agent===step.id);
      const agent=attempts.at(-1);
      // key names recorded activity; a step that has not run in this launch has none.
      result.push({...step,...(agent||{}),key:agent?.id || null,step:step.id,label:step.label,earlier:!agent && step.journal==='completed',
        status:step.journal && !panel.active ? step.journal : agent?.status || step.journal || 'pending'});
      if(agent) seen.add(agent.id);
    }
    for(const id of panel.order) if(!seen.has(id) && !panel.plan.some(step=>step.id===panel.agents.get(id).agent)) { const agent=panel.agents.get(id); result.push({...agent,key:agent.id,step:agent.agent,label:labels[agent.agent] || agent.agent}); }
    return result;
  }
  function describe(item) {
    if(item.type!=='tool') return item.text || '';
    return [item.service,item.path,item.detail].filter(Boolean).join(' · ');
  }
  function row(item) {
    const node=el('li',`agent-event ${item.type}${item.type==='tool' && item.ok===false ? ' failed' : ''}`);
    node.append(el('time','agent-event-time',agentClock(item.at)),el('span','agent-event-kind',item.type==='tool' ? item.tool || 'Tool' : AGENT_KINDS[item.type] || item.type));
    const body=el(item.type==='text' || item.type==='plan' || item.type==='thinking' ? 'pre' : 'span','agent-event-body',describe(item));
    node.append(body);
    if(item.type==='tool') node.append(el('span','agent-event-state',item.state==='completed' ? (item.ok===false ? 'failed' : 'done') : 'running'));
    return node;
  }
  function card(agent) {
    const button=el('button',`agent-card ${agent.status}`); button.type='button';
    button.setAttribute('aria-pressed',String(selection(agent)===panel.selected));
    const end=agent.finished ? new Date(agent.finished) : new Date(), start=agent.started ? new Date(agent.started) : null;
    const elapsed=start ? agentDuration(Number.isFinite(agent.duration) && agent.finished ? agent.duration : (end-start)/1000) : '';
    const top=el('div','agent-card-top'); top.append(el('span','agent-dot'),el('strong','',agent.label),el('span','agent-card-status',[AGENT_STATUS[agent.status] || humanLabel(agent.status),elapsed].filter(Boolean).join(' · ')));
    button.append(top);
    const names=(items,count)=>items.join(', ')+(count>items.length ? ` +${count-items.length} more` : '');
    const scope=[agent.mode,agent.attempt>1 ? `attempt ${agent.attempt}` : '',agent.readable?.length ? `reads ${names(agent.readable,agent.readableCount)}` : '',agent.writable?.length ? `writes ${names(agent.writable,agent.writableCount)}` : ''].filter(Boolean).join(' · ');
    if(scope) button.append(el('p','agent-card-scope',scope));
    const current=agent.status==='running' ? agent.last : null;
    if(current) button.append(el('p','agent-card-now',`${current.type==='tool' ? (current.tool || 'Tool')+' · ' : ''}${describe(current)}`.slice(0,180)));
    else if(agent.earlier) button.append(el('p','agent-card-summary','Completed in an earlier launch.'));
    else if(agent.summary || agent.reason || agent.error || agent.goal) button.append(el('p','agent-card-summary',(agent.summary || agent.reason || (agent.error ? humanLabel(String(agent.error)) : agent.goal)).slice(0,180)));
    const stats=[agentCount(agent.tools,'tool call'),agentCount(agent.messages,'message'),agentTokens(agent.tokens),Number.isFinite(agent.cost) && agent.cost>0 ? `$${agent.cost.toFixed(4)}` : '',agentCount(agent.changedCount || agent.changed?.length,'changed file')].filter(Boolean).join(' · ');
    if(stats) button.append(el('p','agent-card-stats',stats));
    button.addEventListener('click',()=>{panel.selected=selection(agent); panel.follow=false; followInput.checked=false; render();});
    return button;
  }
  function selection(agent) { return agent.key || 'step:'+agent.step; }
  function render() {
    const agents=agentsInOrder();
    if(panel.follow) {
      // Follow the working agent, otherwise the last one that actually ran.
      const running=agents.filter(a=>a.status==='running' && a.key).at(-1), latest=agents.filter(a=>a.key).at(-1);
      panel.selected=(running || latest)?.key || null;
    }
    if(panel.selected?.startsWith('step:')) {
      // A chosen waiting card becomes that agent's timeline once it starts.
      const started=agents.find(a=>a.key && selection({step:a.step})===panel.selected);
      if(started) panel.selected=started.key;
    }
    cards.replaceChildren(...agents.map(card));
    const chosen=panel.agents.get(panel.selected), waiting=agents.find(a=>!a.key && selection(a)===panel.selected);
    const renderKey=`${panel.selected}|${chosen?.events.length}|${chosen?.events.filter(e=>e.state==='completed').length}|${chosen?.status}`;
    if(renderKey===panel.renderedKey) return;
    panel.renderedKey=renderKey;
    const stick=list.scrollHeight-list.scrollTop-list.clientHeight<40;
    heading.replaceChildren();
    if(waiting) {
      heading.append(el('strong','',waiting.label));
      list.replaceChildren(el('li','agent-placeholder',waiting.earlier ? 'This agent completed in an earlier launch; choose that launch above.' : 'This agent has not run in this launch.'));
      return;
    }
    if(!chosen) { heading.append(el('span','',emptyText)); list.replaceChildren(); return; }
    const label=agents.find(a=>a.id===chosen.id)?.label || chosen.agent;
    heading.append(el('strong','',label),el('span','',[chosen.provider ? (providerFor(chosen.provider)?.name || chosen.provider) : '',chosen.root || ''].filter(Boolean).join(' · ')));
    list.replaceChildren(...(chosen.events.length ? chosen.events.map(row) : [el('li','agent-placeholder',chosen.status==='running' ? 'Waiting for the first action…' : 'No activity was recorded for this agent.')]));
    if(chosen.summary || chosen.reason || chosen.error) list.append(el('li',`agent-outcome ${chosen.ok ? 'ok' : 'failed'}`,[chosen.summary,chosen.reason].filter(Boolean).join('\n') || humanLabel(String(chosen.error))));
    if(stick) list.scrollTop=list.scrollHeight;
  }
  async function fetchEvents(epoch) {
    for(let page=0; page<40; page++) {
      const data=await api(`/api/sessions/${encodeURIComponent(panel.sessionId)}?after=${panel.cursor}`);
      if(epoch!==panel.epoch) return;
      for(const event of data.events || []) {
        if(!Number.isInteger(event.id) || event.id<=panel.cursor) continue;
        panel.cursor=event.id; observe(event);
      }
      if((data.events || []).length<250) return;
    }
  }
  panel.setPlan=(steps,active)=>{ panel.plan=steps; panel.active=Boolean(active); render(); };
  panel.load=sessionId=>{
    if(sessionId!==panel.sessionId) {
      ++panel.epoch; Object.assign(panel,{sessionId,cursor:0,selected:null,renderedKey:'',pending:null}); panel.agents.clear(); panel.order.length=0;
      panel.follow=true; followInput.checked=true; render();
    }
    if(!sessionId) return Promise.resolve();
    // One reader per session: overlapping polls would observe the same page twice.
    if(panel.pending) return panel.pending;
    const epoch=panel.epoch;
    panel.pending=(async()=>{
      try { await fetchEvents(epoch); if(epoch===panel.epoch) render(); }
      catch(error) { if(epoch===panel.epoch) { panel.renderedKey=''; heading.replaceChildren(el('span','agent-error','Agent activity is unavailable: '+error.message)); } }
      finally { if(epoch===panel.epoch) panel.pending=null; }
    })();
    return panel.pending;
  };
  panel.reset=()=>{ ++panel.epoch; Object.assign(panel,{sessionId:null,cursor:0,selected:null,plan:[],renderedKey:'',pending:null}); panel.agents.clear(); panel.order.length=0; render(); };
  render();
  return panel;
}
