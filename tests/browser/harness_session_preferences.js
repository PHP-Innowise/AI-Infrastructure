// Run with Playwright browser_run_code_unsafe({filename: this file's absolute path}).
// Isolated browser context and read-only API fixtures; no model runs or project writes.
async (page) => {
  const context = await page.context().browser().newContext({viewport:{width:1440,height:1000}});
  const tab = await context.newPage();
  const check = (ok,message) => { if (!ok) throw new Error(message); };
  const boot = await (await page.request.get('http://127.0.0.1:8766/api/bootstrap')).json();
  boot.projects = ['a','b'].map(id => ({id,name:`Project ${id}`,path:`/fixture/${id}`,available:true}));
  boot.providers = ['claude','codex'].map(id => ({id,name:id,available:true,model_options:{models:[],efforts:['high','low']}}));
  const session = {id:'saved-session',title:'Saved session',project_id:'b',provider:'codex',workflow:'native',mode:'plan',model:'saved-model',thinking_effort:'high',status:'completed',native_session_id:'native-fixture',project_context:true,agents_enabled:true,agent_count:2,workspace:'project',budgets:{usd:null,tokens:777,seconds:120},budget_revision:1};
  boot.sessions = [session];
  const writes = [], errors = []; let removedBank = false;
  tab.on('pageerror',error => errors.push(error.message));
  await tab.route('**/api/**',async route => {
    const request = route.request(), path = request.url().split('/').slice(3).join('/').split('?')[0];
    if (request.method() !== 'GET') { writes.push(path); return route.abort(); }
    let data;
    if (path === 'api/bootstrap') data = boot;
    else if (path === 'api/sessions/saved-session') data = {session,events:[{id:1,kind:'assistant',text:'Saved result'}]};
    else if (path.endsWith('/git')) data = {project_id:path.split('/')[2],is_git:true,worktree_available:true,dirty:false,branch:'main',head:'a'.repeat(40)};
    else if (path.endsWith('/brain')) data = {runtime_available:true,mode:'governed',bank_id:request.url().includes('bank=bank-two')?'bank-two':'bank-one',banks:(removedBank?['bank-one']:['bank-one','bank-two']).map(id=>({id,name:id,path:id})),entries:['task-one','task-two'].map(id=>({id,type:'task',title:id,status:'active'}))};
    else data = {};
    await route.fulfill({json:data});
  });
  const ready = async () => { await tab.locator('#connecting').waitFor({state:'hidden'}); await tab.locator('#provider option[value="codex"]').waitFor({state:'attached'}); };
  const snapshot = () => tab.evaluate(() => Object.fromEntries(['project','provider','workflow','mode','model','thinking-effort','agent-count','workspace','worktree-branch','session-budgets-usd','session-budgets-tokens','session-budgets-seconds'].map(id => [id,document.getElementById(id).value]).concat([['agents-enabled',document.getElementById('agents-enabled').checked]])));
  try {
    await tab.goto('http://127.0.0.1:8766/'); await ready();
    await tab.locator('#project').selectOption('b');
    await tab.locator('#provider').selectOption('claude');
    await tab.locator('#mode').selectOption('edit');
    await tab.locator('#model-choice').selectOption('--custom--');
    await tab.locator('#model').fill('draft-model');
    await tab.locator('#thinking-effort').selectOption('high');
    await tab.locator('#agents-enabled').check(); await tab.locator('#agent-count').fill('5');
    await tab.locator('#workspace-worktree-option:not(:disabled)').waitFor({state:'attached'});
    await tab.locator('#workspace').selectOption('worktree'); await tab.locator('#worktree-branch').fill('feature/draft');
    await tab.locator('#session-budgets > details > summary').click();
    for (const [key,value] of Object.entries({usd:'12',tokens:'12345',seconds:'600'})) await tab.locator('#session-budgets-'+key).fill(value);
    const expected = await snapshot();
    await tab.reload(); await ready();
    check(JSON.stringify(await snapshot()) === JSON.stringify(expected),'Reload must preserve the selected project and all launch settings');
    await tab.locator('#project').selectOption('a');
    check(await tab.locator('#session-budgets-tokens').inputValue() === '','A new project must not inherit another project budget');
    await tab.locator('#provider').selectOption('codex');
    await tab.locator('#project').selectOption('b');
    check(JSON.stringify(await snapshot()) === JSON.stringify(expected),'Switching projects must restore their own settings');
    await tab.getByRole('button',{name:/Saved session/}).click();
    await tab.locator('#events .message').waitFor();
    await tab.reload(); await ready();
    await tab.locator('#events .message').waitFor();
    check(await tab.locator('#model').inputValue() === 'saved-model','The selected session must reload its server settings');
    check(await tab.locator('#session-budgets-tokens').inputValue() === '777','Session budgets must come from the server');
    await tab.getByRole('button',{name:'New session',exact:true}).click();
    check(JSON.stringify(await snapshot()) === JSON.stringify(expected),'New session must restore the project draft');
    await tab.locator('#project').selectOption('a');
    await tab.locator('#workflow').selectOption('sdd'); await tab.locator('#sdd-feature').fill('saved-feature');
    await tab.locator('#model-routing-config > summary').click(); await tab.locator('#model-routing-enabled').check();
    for (const role of ['plan','edit']) {
      await tab.locator('#routing-'+role+'-model-choice').selectOption('--custom--');
      await tab.locator('#routing-'+role+'-model').fill(role+'-model');
      await tab.locator('#routing-'+role+'-thinking-effort').selectOption(role==='plan'?'high':'low');
    }
    await tab.locator('#session-budgets > details > summary').click();
    await tab.locator('#session-budgets-agent-tokens').fill('321');
    await tab.locator('#brain-link-config > summary').click(); await tab.locator('#brain-link-enabled').check();
    await tab.locator('#brain-link-bank').selectOption('bank-two');
    await tab.locator('#brain-link-task').selectOption('task-two'); await tab.locator('#brain-link-query').fill('Saved context query');
    const extras = () => tab.evaluate(() => Object.fromEntries(['sdd-feature','sdd-phase','routing-plan-model','routing-edit-model','routing-plan-thinking-effort','routing-edit-thinking-effort','brain-link-bank','brain-link-task','brain-link-query','session-budgets-agent-tokens'].map(id=>[id,document.getElementById(id).value]).concat(['model-routing-enabled','brain-link-enabled'].map(id=>[id,document.getElementById(id).checked]))));
    const expectedExtras = await extras();
    await tab.reload(); await ready();
    check(JSON.stringify(await extras()) === JSON.stringify(expectedExtras),'Routing, SDD, per-agent budgets and Brain selections must survive reload');
    const stored = await tab.evaluate(() => JSON.parse(localStorage.getItem('harness.sessions.preferences.v1')));
    check(!('csrf' in stored) && !JSON.stringify(stored).includes(boot.csrf),'Preferences must never include the control token');
    removedBank = true;
    await tab.reload(); await ready();
    check(await tab.locator('#brain-link-bank').inputValue() === '' && await tab.locator('#brain-link-task').inputValue() === '','A missing saved root must not silently bind another root or task');
    removedBank = false;
    await tab.reload(); await ready();
    check(await tab.locator('#brain-link-bank').inputValue() === 'bank-two','An unavailable root must not erase the last saved selection');
    boot.projects = boot.projects.filter(project=>project.id!=='a');
    await tab.reload(); await ready();
    check(await tab.locator('#project').inputValue() === 'b','A removed project must fall back to a registered project');
    check(await tab.locator('#model').inputValue() === 'draft-model','The fallback project must use its own saved settings');
    await tab.evaluate(() => localStorage.setItem('harness.sessions.preferences.v1','{invalid'));
    // Simulate a storage write failure so pagehide cannot replace the corrupt value.
    await tab.evaluate(() => { Storage.prototype.setItem = () => { throw new Error('Storage unavailable'); }; });
    await tab.reload(); await ready();
    check(await tab.locator('#project').inputValue() === 'b','Malformed storage must fall back to usable defaults');
    check(errors.length === 0,'Preference restoration must not cause JavaScript errors: '+errors.join('; '));
    check(writes.length === 0,'Restoration must never launch or mutate anything on the server');
    return {ok:true,checks:['reload','project isolation','selected session','new session','model routing','SDD','Brain','per-agent budgets','removed project','corrupt storage','no API writes or JavaScript errors']};
  } finally { await context.close(); }
}
