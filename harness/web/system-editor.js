/* System/service forms; the backend generates and validates file formats. */
const systemEditor = {data:null,pending:false,epoch:0,dirty:false};
function editorControls() {
  // The AI scan's agent panel stays usable while the scan locks the form.
  for(const input of $('system-editor').querySelectorAll('input,select,textarea,button')) if(!input.closest('#system-discovery-agents')) input.disabled=systemEditor.pending;
  for(const id of ['system-choose-project','system-project','system-config','system-load']) $(id).disabled=systemEditor.pending || systemEditor.dirty || systemUi.pending || !state.bootstrap;
  $('system-edit').disabled=systemEditor.pending || systemUi.pending || !state.bootstrap;
  $('system-editor-add').disabled=$('system-editor-save').disabled=systemEditor.pending || !systemEditor.data;
  $('system-editor-save').textContent=systemEditor.pending?'Working…':'Save system';
  if(typeof discoveryControls==='function') discoveryControls();
}
function closeSystemEditor() {
  if(systemEditor.pending) return;
  ++systemEditor.epoch; systemEditor.data=null; systemEditor.dirty=false; $('system-editor').hidden=true;
  if(typeof resetDiscovery==='function') resetDiscovery();
  systemLayout();
}
function editorError(error) { $('system-editor-error').textContent=error.message; $('system-editor-error').hidden=false; }
function editorChanged() { systemEditor.dirty=true; $('system-editor-status').textContent='Unsaved changes';systemControls(); }
function editorField(parent,label,value,update,{choices=null,multiline=false,required=false,limit=2000}={}) {
  const wrapper=el('label','field',label), input=document.createElement(choices?'select':multiline?'textarea':'input');
  if(!choices && !multiline) input.type='text';
  if(choices) for(const choice of choices) { const option=el('option','',choice); option.value=choice; input.append(option); }
  input.value=value ?? ''; input.required=required; if(!choices) input.maxLength=limit; if(multiline) input.rows=2;
  if(multiline) wrapper.classList.add('wide');
  input.addEventListener(choices?'change':'input',()=>{update(input.value);editorChanged();}); wrapper.append(input); parent.append(wrapper); return input;
}
function editorLines(parent,label,values,update) {
  return editorField(parent,label,(values || []).join('\n'),value=>update(value.split('\n').map(v=>v.trim()).filter(Boolean)),{multiline:true,limit:102400});
}
function editorButton(parent,label,action,className='button') {
  const button=el('button',className,label); button.type='button'; button.addEventListener('click',action); parent.append(button); return button;
}
function editorRows(parent,values,{title,addLabel,empty,draw}) {
  const render=()=>{
    parent.replaceChildren();
    const summary=parent.closest('details')?.querySelector('summary');
    if(summary?.dataset.editorTitle) summary.textContent=`${summary.dataset.editorTitle} (${values.length})`;
    values.forEach((value,index)=>{
      const row=el('div','system-editor-row'), head=el('div','system-editor-row-head'), grid=el('div','system-editor-grid');
      head.append(el('h6','',`${title} ${index+1}`)); row.append(head,grid); draw(grid,value);
      editorButton(head,'Remove',()=>{values.splice(index,1);editorChanged();render();},'button quiet').setAttribute('aria-label',`Remove ${title.toLowerCase()} ${index+1}`); parent.append(row);
    });
    editorButton(parent,addLabel,()=>{values.push(empty());editorChanged();render();});
  };render();
}
function editorSources(parent,values) {
  editorRows(parent,values,{title:'Source',addLabel:'Add source',empty:()=>({path:'',kind:'code'}),draw:(grid,value)=>{
    editorField(grid,'Source path',value.path,text=>value.path=text,{required:true,limit:1024});
    editorField(grid,'Source kind',value.kind,text=>value.kind=text,{choices:['policy','spec','contract','code','test','memory']});
  }});
}
function editorSection(card,title) {
  const section=el('details','system-editor-section'), rows=el('div'), summary=el('summary','',title);
  summary.dataset.editorTitle=title.replace(/ \(\d+\)$/,'');section.append(summary,rows);card.append(section);return rows;
}
async function editorRegister(folder) {
  const registered=await api('/api/projects',{method:'POST',body:{path:folder}});
  updateRegisteredProjects(registered.projects);
  setProjectChoices($('system-project'),registered.projects,$('system-project').value);
  return registered.project;
}
function editorPickService(existing=null,opener=$('system-editor-add')) {
  chooseProjectFolder(existing?.path || projectFor($('system-project').value)?.path, async folder=>{
    systemEditor.pending=true;systemControls();$('system-editor-error').hidden=true;
    try {
      const project=await editorRegister(folder);
      const service=await api('/api/systems/service',{method:'POST',body:{project_id:project.id}});
      if(existing && !service.fingerprint) service.passport=structuredClone(existing.passport);
      if(existing) systemEditor.data.services.splice(systemEditor.data.services.indexOf(existing),1,service);
      else systemEditor.data.services.push(service);
      editorChanged();renderSystemEditor();
    } catch(error) {editorError(error);}
    finally {systemEditor.pending=false;systemControls();}
  },opener,'Choose a service folder');
}
function renderSystemEditor() {
  const data=systemEditor.data;
  const parent=data.config_path.split('/').slice(0,-1).join('/');
  $('system-editor').hidden=false; $('system-editor-location').textContent=(projectFor(data.project_id)?.path || '')+(parent?'/'+parent:'')+'/'+data.config_path.split('/').pop();
  $('system-editor-name').value=data.name; $('system-editor-services').replaceChildren();
  for(const service of data.services) {
    const passport=service.passport, card=el('section','system-editor-service'), head=el('div','system-editor-service-head'), title=el('div'), actions=el('div','system-editor-service-actions'), grid=el('div','system-editor-grid');
    title.append(el('h5','',passport.id || 'New service'),el('p','system-note',service.path)); head.append(title,actions); card.append(head,grid);
    editorField(grid,'Service ID',passport.id,value=>passport.id=value,{required:true,limit:80});
    editorField(grid,'Owning team',passport.owner,value=>passport.owner=value,{required:true,limit:200});
    editorField(grid,'Service description',passport.description,value=>passport.description=value,{multiline:true,required:true});
    const survey=el('label','checkbox'), checkbox=document.createElement('input'); checkbox.type='checkbox';checkbox.checked=passport.relationships_complete;
    checkbox.addEventListener('change',()=>{passport.relationships_complete=checkbox.checked;editorChanged();});survey.append(checkbox,document.createTextNode('I have described all known dependencies'));card.append(survey);
    editorButton(actions,'Choose another folder',event=>editorPickService(service,event.currentTarget),'button quiet');
    editorButton(actions,'Remove',()=>{data.services.splice(data.services.indexOf(service),1);editorChanged();renderSystemEditor();},'button quiet').setAttribute('aria-label',`Remove ${passport.id || 'this service'} from the system`);
    editorRows(editorSection(card,`Capabilities (${passport.capabilities.length})`),passport.capabilities,{
      title:'Capability',addLabel:'Add capability',empty:()=>({id:'',description:'',status:'unknown',sources:[],keywords:[]}),draw:(grid,value)=>{
        editorField(grid,'Capability ID',value.id,text=>value.id=text,{required:true,limit:80});
        editorField(grid,'Capability status',value.status,text=>value.status=text,{choices:['unknown','planned','partial','implemented']});
        editorField(grid,'Capability description',value.description,text=>value.description=text,{multiline:true,required:true});
        editorLines(grid,'Capability source paths (one per line)',value.sources,text=>value.sources=text);
        editorLines(grid,'Routing keywords (one per line)',value.keywords,text=>value.keywords=text);
      }});
    editorRows(editorSection(card,`Provided contracts (${passport.provides.length})`),passport.provides,{
      title:'Provided contract',addLabel:'Add provided contract',empty:()=>({id:'',kind:'http',version:'1',sources:[]}),draw:(grid,value)=>{
        editorField(grid,'Provided contract ID',value.id,text=>value.id=text,{required:true,limit:80});
        editorField(grid,'Contract type',value.kind,text=>value.kind=text,{choices:['http','event','rpc','graphql','other']});
        editorField(grid,'Provided contract version',value.version,text=>value.version=text,{required:true,limit:100});
        editorLines(grid,'Contract source paths (one per line)',value.sources,text=>value.sources=text);
      }});
    editorRows(editorSection(card,`Consumed contracts (${passport.consumes.length})`),passport.consumes,{
      title:'Consumed contract',addLabel:'Add consumed contract',empty:()=>({service:'',contract:'',version:'1'}),draw:(grid,value)=>{
        editorField(grid,'Provider service ID',value.service,text=>value.service=text,{required:true,limit:80});
        editorField(grid,'Consumed contract ID',value.contract,text=>value.contract=text,{required:true,limit:80});
        editorField(grid,'Consumed contract version',value.version,text=>value.version=text,{required:true,limit:100});
      }});
    editorSources(editorSection(card,`Sources of context (${passport.sources.length})`),passport.sources);
    $('system-editor-services').append(card);
  }
  editorSources($('system-editor-shared'),data.shared_sources); editorControls(); systemLayout();
}
async function openSystemEditor() {
  if(systemEditor.pending) return;
  const epoch=++systemEditor.epoch; systemEditor.pending=true; systemControls();
  $('system-editor-error').hidden=true; $('system-editor-status').textContent='Loading system details…';
  try {
    const data=await api('/api/systems/editor',{method:'POST',body:{project_id:$('system-project').value,config_path:$('system-config').value}});
    if(epoch!==systemEditor.epoch) return;
    systemEditor.data=data;systemEditor.dirty=false;renderSystemEditor();$('system-editor-status').textContent='';
    if(typeof restoreDiscovery==='function') await restoreDiscovery();
    $('system-editor').scrollIntoView({behavior:scrollMotion(),block:'start'});
  } catch(error) { $('system-editor').hidden=false; editorError(error); $('system-editor-status').textContent=''; systemLayout(); }
  finally {systemEditor.pending=typeof systemDiscovery!=='undefined' && systemDiscovery.active;systemControls();}
}
$('system-edit').addEventListener('click',()=>{
  if(systemEditor.data && systemEditor.dirty) { $('system-editor').hidden=false; systemLayout(); $('system-editor').scrollIntoView({block:'start'}); return; }
  openSystemEditor();
});
// Choosing another system folder registers it and makes it the working project in every view.
$('system-choose-project').addEventListener('click',event=>chooseProjectFolder(projectFor($('system-project').value)?.path,async folder=>{
  systemEditor.pending=true;systemControls();
  try {
    const project=await editorRegister(folder); systemEditor.data=null;systemEditor.dirty=false;
    systemEditor.pending=false; switchProject(project.id);
    if($('system-project').value!==project.id) { $('system-project').value=project.id;systemReset();systemUi.project=project.id;await systemHistory(); }
  }
  catch(error) {systemFailure(error);}
  finally {systemEditor.pending=false;systemControls();}
  await openSystemEditor();
},event.currentTarget,'Choose a system folder','Browse folders on the computer running Harness. The folder you select joins your projects and becomes the working project in every view.'));
$('system-editor-add').addEventListener('click',()=>editorPickService());
$('system-editor-name').addEventListener('input',()=>{if(systemEditor.data) systemEditor.data.name=$('system-editor-name').value;editorChanged();});
// Close keeps the draft for this page; Edit system reopens it.
$('system-editor-close').addEventListener('click',()=>{ $('system-editor').hidden=true; systemLayout(); $('system-edit').focus(); });
$('system-editor-discard').addEventListener('click',()=>{if(typeof discoveryRemember==='function') discoveryRemember(null);closeSystemEditor();systemControls();$('system-edit').focus();});
$('system-editor').addEventListener('submit',async event=>{
  event.preventDefault(); if(systemEditor.pending || !systemEditor.data) return;
  for(const input of $('system-editor').querySelectorAll(':invalid')) {
    const section=input.closest('details');if(section) section.open=true;
  }
  if(!$('system-editor').reportValidity()) return;
  systemEditor.pending=true;systemControls();$('system-editor-error').hidden=true;$('system-editor-status').textContent='Validating and saving…';
  let saved=false;
  try {
    const data=systemEditor.data, services=data.services.map(({path,...service})=>service);
    const prepared=await api('/api/systems/preview',{method:'POST',body:{...data,services}});
    const result=await api('/api/systems/apply',{method:'POST',body:{preview_id:prepared.preview_id}});
    systemEditor.data=result.editor;systemEditor.dirty=false;renderSystemCatalog(result);
    if(typeof discoveryRemember==='function') discoveryRemember(null);
    saved=true;
  } catch(error) {editorError(error);$('system-editor-status').textContent='Save did not complete. Your form is retained; reload existing metadata before retrying if files changed.';}
  finally {systemEditor.pending=false;systemControls();}
  // The saved system replaces the editor with its updated map.
  if(saved) { closeSystemEditor(); $('system-message').textContent='System saved. The map shows the saved services.'; $('system-edit').focus(); }
});
systemControls();
