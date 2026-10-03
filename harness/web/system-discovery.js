/* AI proposals populate ordinary forms; Save is the sole metadata writer. */
const systemDiscovery={id:null,timer:null,epoch:0,active:false,providerKey:'',
  panel:createAgentPanel($('system-discovery-agent'),{emptyText:'The discovery agent has not started yet.'})};
const DISCOVERY_OUTCOME={completed:'completed',stale:'completed',failed:'blocked',cancelled:'interrupted',interrupted:'interrupted'};
function discoveryAgent(job) {
  $('system-discovery-agents').hidden=false; $('system-discovery-live').hidden=!job.active;
  systemDiscovery.panel.setPlan([{id:'discovery',service:'__system__',mode:'read-only',label:'Discovery agent',
    goal:'Reads the captured evidence and fills the service forms.',journal:job.active?null:DISCOVERY_OUTCOME[job.status] || null}],job.active);
  systemDiscovery.panel.load(job.id);
}
function discoveryKey() {const d=systemEditor.data;return d?`harness-discovery:${d.project_id}:${d.config_path}`:null;}
function discoveryRemember(id) {try {const key=discoveryKey();if(key) id?localStorage.setItem(key,id):localStorage.removeItem(key);} catch(_) {}}
function discoveryControls() {
  const select=$('system-discovery-provider'), providers=(state.bootstrap?.providers || []).filter(p=>['codex','claude','cursor'].includes(p.id));
  const key=JSON.stringify(providers.map(p=>[p.id,p.available]));
  if(key!==systemDiscovery.providerKey) {
    const prior=select.value;select.replaceChildren();
    for(const provider of providers) {const option=el('option','',provider.name+(provider.available?'':' · unavailable'));option.value=provider.id;option.disabled=!provider.available;select.append(option);}
    select.value=providers.some(p=>p.id===prior && p.available)?prior:providers.find(p=>p.available)?.id || '';
    systemDiscovery.providerKey=key;
  }
  $('system-discovery-start').disabled=systemEditor.pending || !systemEditor.data?.services.length || !providers.some(p=>p.id===select.value && p.available);
  $('system-discovery-cancel').hidden=!systemDiscovery.active;
  $('system-discovery-cancel').disabled=!systemDiscovery.id;
}
function resetDiscovery() {
  clearTimeout(systemDiscovery.timer);++systemDiscovery.epoch;systemDiscovery.active=false;systemDiscovery.id=null;
  $('system-discovery-status').textContent='';$('system-discovery-report').hidden=true;
  $('system-discovery-agents').hidden=true;systemDiscovery.panel.reset();
}
function discoveryReport(proposal) {
  const panel=$('system-discovery-report');panel.replaceChildren();panel.hidden=false;
  panel.append(el('p','system-note','AI filled the forms below. Verify inferred behavior, ownership and dependencies before saving. Complete dependency coverage remains unconfirmed.'));
  for(const warning of proposal.warnings) panel.append(el('p','system-warning',warning));
  for(const report of proposal.reports) {
    const details=el('details','skills-preview'),summary=el('summary','',`${report.id} · ${report.passport.capabilities.length} capabilities · ${report.passport.provides.length} provided · ${report.passport.consumes.length} consumed contracts`);
    details.append(summary);
    const paths=new Set([...report.description_sources,...report.owner_sources,...report.passport.sources.map(s=>s.path),...report.passport.capabilities.flatMap(c=>c.sources),...report.passport.provides.flatMap(c=>c.sources),...report.consumption_sources.flatMap(c=>c.sources)]);
    for(const path of paths) {const proof=proposal.inventory[report.id][path];details.append(el('p','system-note',`${path} · ${proof.kind} · SHA-256 ${proof.sha256.slice(0,12)}`));}
    for(const warning of report.uncertainties) details.append(el('p','system-warning',warning));
    panel.append(details);
  }
}
async function pollDiscovery(epoch) {
  if(epoch!==systemDiscovery.epoch || !systemDiscovery.id) return;
  clearTimeout(systemDiscovery.timer);
  try {
    const job=await api('/api/system-discoveries/'+systemDiscovery.id);
    if(epoch!==systemDiscovery.epoch) return;
    systemDiscovery.active=job.active;discoveryAgent(job);
    const last=job.events.filter(e=>['status','error','text'].includes(e.kind)).at(-1);
    $('system-discovery-status').textContent=`AI scan · ${job.provider} · ${job.status}${job.active && last?.text?' · '+last.text:''}`;
    if(job.active) {systemDiscovery.timer=setTimeout(()=>pollDiscovery(epoch),1000);systemControls();return;}
    ++systemDiscovery.epoch;systemEditor.pending=false;
    if(job.proposal) {
      systemEditor.data=job.proposal.editor;systemEditor.dirty=true;renderSystemEditor();discoveryReport(job.proposal);
      $('system-editor-status').textContent='AI filled the service details. Review sources and uncertainties, then Save system.';
    } else {
      const failure=job.error || job.events.find(e=>e.kind==='error')?.text;
      $('system-discovery-status').textContent=(failure || `AI scan ${job.status}.`)+ ' Your original form is retained; you can retry.';
    }
    systemControls();refreshSessions();
  } catch(error) {
    if(epoch!==systemDiscovery.epoch) return;
    if([400,404].includes(error.status)) {
      try {
        const fallback=await api('/api/sessions/'+systemDiscovery.id);
        if(epoch!==systemDiscovery.epoch) return;
        if(!['queued','running'].includes(fallback.session.status)) {
          ++systemDiscovery.epoch;systemDiscovery.active=false;systemEditor.pending=false;systemEditor.dirty=true;
          $('system-discovery-status').textContent=error.message+' Your original form is retained; start a new scan.';
          discoveryRemember(null);systemControls();return;
        }
      } catch(fallbackError) {
        if(epoch!==systemDiscovery.epoch) return;
        if([400,404].includes(fallbackError.status)) {
          ++systemDiscovery.epoch;systemDiscovery.active=false;systemEditor.pending=false;systemEditor.dirty=true;
          $('system-discovery-status').textContent='AI scan is no longer available. Your original form is retained.';
          discoveryRemember(null);systemControls();return;
        }
      }
    }
    // Retain the locked draft while a possibly running server job reconnects.
    $('system-discovery-status').textContent='Reconnecting to AI scan: '+error.message;
    systemDiscovery.timer=setTimeout(()=>pollDiscovery(epoch),3000);
  }
}
async function restoreDiscovery() {
  resetDiscovery();const key=discoveryKey();let id;try {id=key && localStorage.getItem(key);} catch(_) {}if(!id) return;
  try {
    const job=await api('/api/system-discoveries/'+id);
    if(!systemEditor.data || discoveryKey()!==key) return;
    systemDiscovery.id=id;discoveryAgent(job);
    if(job.active) {systemEditor.data=job.draft;systemEditor.dirty=true;systemEditor.pending=true;systemDiscovery.active=true;renderSystemEditor();}
    else if(job.proposal) {systemEditor.data=job.proposal.editor;systemEditor.dirty=true;renderSystemEditor();discoveryReport(job.proposal);}
    else if(job.draft.revision===systemEditor.data.revision) {systemEditor.data=job.draft;systemEditor.dirty=true;renderSystemEditor();}
    $('system-discovery-status').textContent=job.error || `Previous AI scan · ${job.status}`;
    if(job.active) pollDiscovery(systemDiscovery.epoch);
  } catch(error) {$('system-discovery-status').textContent='Previous AI scan unavailable: '+error.message;}
}
$('system-discovery-start').addEventListener('click',async()=>{
  if(systemEditor.pending || !systemEditor.data) return;
  resetDiscovery();const epoch=systemDiscovery.epoch;systemEditor.pending=true;systemControls();$('system-editor-error').hidden=true;
  $('system-discovery-status').textContent='Capturing service evidence and queueing AI scan…';
  try {
    const data=systemEditor.data,services=data.services.map(({path,...service})=>service);
    const job=await api('/api/system-discoveries',{method:'POST',body:{editor:{...data,services},provider:$('system-discovery-provider').value,timeout:Number($('system-discovery-timeout').value)}});
    if(epoch!==systemDiscovery.epoch) return;
    systemDiscovery.id=job.id;systemDiscovery.active=job.active;discoveryRemember(job.id);systemControls();refreshSessions();pollDiscovery(epoch);
  } catch(error) {editorError(error);systemEditor.pending=false;systemControls();$('system-discovery-status').textContent='AI scan did not start. Your form is retained.';}
});
$('system-discovery-cancel').addEventListener('click',async()=>{
  if(!systemDiscovery.id) return;
  $('system-discovery-cancel').disabled=true;
  try {await api('/api/sessions/'+systemDiscovery.id+'/cancel',{method:'POST',body:{}});await pollDiscovery(systemDiscovery.epoch);}
  catch(error) {editorError(error);discoveryControls();}
});
$('system-discovery-provider').addEventListener('change',discoveryControls);
systemControls();
