// Run via Playwright browser_run_code_unsafe({filename: this file's absolute path}).
// All session requests are intercepted; this never launches a model.
async (page) => {
  const context = await page.context().browser().newContext({viewport:{width:1440,height:1000}});
  const tab = await context.newPage(), errors = [], sent = [];
  tab.on('pageerror',error => errors.push(error.message));
  const check = (ok,message) => { if (!ok) throw new Error(message); };
  const boot = await (await page.request.get('http://127.0.0.1:8766/api/bootstrap')).json();
  boot.projects = [{id:'fixture',name:'Fixture project',path:'/fixture',available:true}];
  boot.providers = [{id:'codex',name:'Fixture Codex',available:true,model_options:{models:[],efforts:[]}}]; boot.sessions = [];
  let session = null, fail = true; const events = [];
  await tab.route('**/api/**',async route => {
    const request = route.request(), path = request.url().split('/').slice(3).join('/').split('?')[0];
    if (request.method() === 'POST') {
      const data = request.postDataJSON(); sent.push(data);
      check(path === 'api/sessions' || path === 'api/sessions/attachment-session/messages','Unexpected write route');
      if (fail) { fail = false; return route.fulfill({status:400,json:{error:'Fixture rejected: retry with the selected files.'}}); }
      session = {id:'attachment-session',title:'Attachment check',project_id:'fixture',provider:'codex',workflow:'native',mode:'plan',status:'completed',native_session_id:'fixture-native',project_context:false,agents_enabled:false,agent_count:3,workspace:'project',budgets:{usd:null,tokens:null,seconds:null},budget_revision:0};
      events.push({id:events.length+1,kind:'user',text:data.prompt,attachments:data.attachments.map((file,index)=>({id:String(events.length+index+1).padStart(32,'0'),name:file.name,size:file.data.length*3/4}))});
      boot.sessions = [session]; return route.fulfill({status:path==='api/sessions'?201:200,json:{session}});
    }
    if (path === 'api/bootstrap') return route.fulfill({json:boot});
    if (path.endsWith('/git')) return route.fulfill({json:{project_id:'fixture',is_git:false,worktree_available:false,dirty:false,branch:null,head:null}});
    if (path === 'api/sessions/attachment-session') return route.fulfill({json:{session,events}});
    return route.fulfill({json:{}});
  });
  try {
    await tab.goto('http://127.0.0.1:8766/');
    await tab.locator('#attach-files:not(:disabled)').waitFor();
    const choosing = tab.waitForEvent('filechooser'); await tab.getByRole('button',{name:'Attach files',exact:true}).click();
    await (await choosing).setFiles(['/home/aliaksei/Desktop/AI-Infrastructure/harness/README.md','/home/aliaksei/Desktop/AI-Infrastructure/CHANGELOG.md']);
    check(await tab.locator('#attachment-list .attachment-chip').count() === 2,'Both selected files must appear');
    await tab.getByRole('button',{name:'Remove CHANGELOG.md',exact:true}).click();
    await tab.locator('#prompt').fill('Review the attached documentation');
    await tab.locator('#send').click();
    await tab.locator('#composer-error').waitFor({state:'visible'});
    check(await tab.locator('#attachment-list .attachment-chip').count() === 1,'A failed send must retain attachments');
    check(await tab.locator('#prompt').inputValue() === 'Review the attached documentation','A failed send must retain the message');
    await tab.locator('#send').click();
    await tab.locator('#events a[download="README.md"]').waitFor();
    check(sent[1].attachments.length === 1 && sent[1].attachments[0].data.length > 0,'Send must include only the retained file bytes');
    check(await tab.locator('#attachment-list .attachment-chip').count() === 0,'Successful send must clear pending files');
    await tab.locator('#attachment-input').setInputFiles('/home/aliaksei/Desktop/AI-Infrastructure/CHANGELOG.md');
    await tab.locator('#prompt').fill('Now compare this changelog'); await tab.locator('#send').click();
    await tab.locator('#events a[download="CHANGELOG.md"]').waitFor();
    check(sent[2].attachments[0].name === 'CHANGELOG.md','Follow-up must carry the new attachment');
    await tab.reload(); await tab.locator('#events a[download="README.md"]').waitFor();
    check(await tab.locator('#events a[download]').count() === 2,'Uploaded files must remain in restored history');
    await tab.setViewportSize({width:390,height:844});
    await tab.locator('#attachment-input').setInputFiles('/home/aliaksei/Desktop/AI-Infrastructure/harness/README.md');
    check(await tab.locator('#attach-files').isVisible(),'The attach control must remain visible on mobile');
    check(await tab.evaluate(()=>document.documentElement.scrollWidth <= innerWidth),'Attachments must not cause horizontal overflow');
    check(errors.length === 0,errors.join('; '));
    return {ok:true,checks:['native file picker','multiple files','remove','failed send preserves files','create','follow-up','history reload','mobile','no JavaScript errors']};
  } finally { await context.close(); }
}
