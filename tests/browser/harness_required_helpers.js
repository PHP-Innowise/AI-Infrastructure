// Offline API fixture: no requests are forwarded to native models.
async (page) => {
  const context = await page.context().browser().newContext();
  const tab = await context.newPage(), errors = [], requests = [];
  tab.on('pageerror', error => errors.push(error.message));
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const boot = await (await page.request.get('http://127.0.0.1:8766/api/bootstrap')).json();
  boot.projects = [{id:'fixture',name:'Fixture project',path:'/fixture',available:true}];
  boot.providers = [{id:'codex',name:'Fixture Codex',available:true,model_options:{models:[],efforts:[]}}];
  boot.sessions = [];
  const events = []; let session;
  await tab.route('**/api/**', async route => {
    const req = route.request(), path = req.url().split('/').slice(3).join('/').split('?')[0];
    if (req.method() !== 'GET') {
      if (path === 'api/sessions/helpers/budgets' && req.method() === 'POST') {
        session.budgets = req.postDataJSON().budgets; session.budget_revision++;
        return route.fulfill({json:{session}});
      }
      check(req.method() === 'POST' && ['api/sessions','api/sessions/helpers/messages'].includes(path), 'Unexpected mutation');
      const data = req.postDataJSON(); requests.push(data);
      session = {id:'helpers',title:'Required count fixture',project_id:'fixture',provider:'codex',workflow:'native',mode:'plan',status:'completed',native_session_id:'fixture-native',project_context:false,agents_enabled:data.agents_enabled,agent_count:data.agent_count,workspace:'project',budgets:{usd:null,tokens:null,seconds:null},budget_revision:0};
      const confirmed = requests.length === 1 ? 1 : data.agent_count;
      if (data.agents_enabled) events.push({id:events.length+1,kind:'delegation',status:confirmed===data.agent_count?'confirmed':'partial',required_count:data.agent_count,confirmed_count:confirmed,text:`Confirmed ${confirmed}/${data.agent_count} required helper launches for this turn.`});
      boot.sessions = [session]; return route.fulfill({status:200,json:{session}});
    }
    if (path === 'api/bootstrap') return route.fulfill({json:boot});
    if (path.endsWith('/git')) return route.fulfill({json:{project_id:'fixture',is_git:false,worktree_available:false,dirty:false,branch:null,head:null}});
    if (path === 'api/sessions/helpers') return route.fulfill({json:{session,events}});
    return route.fulfill({json:{}});
  });
  try {
    await tab.goto('http://127.0.0.1:8766/');
    await tab.locator('#agents-enabled:not(:disabled)').waitFor();
    check(await tab.locator('#session-budgets-seconds').getAttribute('placeholder') === 'No time limit', 'Empty time must mean no limit');
    check(await tab.locator('#agent-count-label').innerText() === 'Required helpers', 'Native label must require helpers');
    await tab.locator('#agents-enabled').check(); await tab.locator('#agent-count').fill('10');
    await tab.locator('#prompt').fill('Fixture review'); await tab.locator('#send').click();
    await tab.getByText('Required helper count not confirmed.',{exact:true}).waitFor();
    check((await tab.locator('#events').innerText()).includes('1/10'), 'Shortfall must remain visible');
    check(requests[0].agent_count===10 && requests[0].agents_enabled, 'Required count must reach the API');
    check(requests[0].budgets.seconds===null, 'Empty time must send an explicit unlimited budget');
    check(await tab.locator('#agent-count').isEnabled(), 'Completed session must allow editing helpers');
    await tab.locator('#agent-count').fill('3');
    await tab.locator('#session-budgets > details > summary').click();
    await tab.locator('#session-budgets-tokens').fill('4000');
    await tab.locator('#session-budgets-seconds').fill('60');
    await tab.locator('#session-budgets-save').click();
    await tab.locator('#agents-enabled:not(:disabled)').waitFor();
    check(await tab.locator('#agent-count').inputValue() === '3', 'Saving budgets must preserve the pending helper count');
    check(session.budgets.seconds===60, 'Explicit time cap must be saved');
    await tab.locator('#session-budgets-seconds').fill('');
    await tab.locator('#session-budgets-save').click();
    await tab.locator('#agents-enabled:not(:disabled)').waitFor();
    check(session.budgets.seconds===null, 'Clearing time must remove the cap');
    check((await tab.locator('#session-budgets-agent-summary').innerText()).includes('no time limit'), 'Budget summary must show unlimited time');
    await tab.locator('#prompt').fill('Fixture follow-up'); await tab.locator('#send').click();
    await tab.getByText('Required helper count confirmed.',{exact:true}).waitFor();
    check(requests[1].agent_count===3 && requests[1].agents_enabled, 'Follow-up must submit changed count');
    await tab.reload(); await tab.getByText('Required helper count confirmed.',{exact:true}).waitFor();
    check(await tab.locator('#agent-count').inputValue() === '3', 'Changed count must survive reload');
    check((await tab.locator('#events').innerText()).includes('3/3'), 'Full count must be visible');
    session.status = 'running'; await tab.reload(); await tab.locator('#waiting').waitFor({state:'visible'});
    check(await tab.locator('#agent-count').isDisabled() && await tab.locator('#agents-enabled').isDisabled(), 'Active launch settings must be locked');
    session.status = 'completed'; await tab.reload(); await tab.locator('#agents-enabled:not(:disabled)').waitFor();
    await tab.locator('#agents-enabled').uncheck();
    check(await tab.locator('#agent-count').isDisabled(), 'Disabled helpers must disable count input');
    await tab.locator('#prompt').fill('Continue alone'); await tab.locator('#send').click();
    await tab.locator('#agents-enabled:not(:disabled)').waitFor();
    check(requests[2].agents_enabled===false && requests[2].agent_count===3, 'Follow-up must support turning helpers off');
    check(errors.length === 0, errors.join('; '));
    return {ok:true,checks:['unlimited time','save and clear time cap','required label','count sent','shortfall shown','budget save preserves helper edits','change count on follow-up','exact count confirmed','reload','active lock','turn helpers off','no JavaScript errors']};
  } finally { await context.close(); }
}
