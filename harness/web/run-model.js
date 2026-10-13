// Run view model: one pure reducer over a session's events, keyed by launch. No DOM: tests load this file in Node as is.
// The Run strip, its Files, Plan and Commands tabs, the log's step groups and the run receipt all read the same state,
// so history and a live run are the same fold of the same events.
'use strict';
const RunModel = (() => {
  // Agent strings stay text; bidi controls are dropped so a path or command cannot reorder what is shown around it.
  const clean = value => String(value ?? '').replace(/[\u202A-\u202E\u2066-\u2069]/g,'');
  const READ = new Set(['read']), EDIT = new Set(['edit','write','multiedit','notebookedit','delete']), SEARCH = new Set(['grep','glob','ls']);
  const COMMAND = new Set(['bash','shell']), TASK = new Set(['task','agent']), PLAN = new Set(['todowrite','updatetodos']);
  const CODEX = {command_execution:'Shell',file_change:'Edit',mcp_tool_call:'MCP',web_search:'Web search'};
  const VERB = {read:'Reading',grep:'Searching',glob:'Searching',ls:'Listing files',edit:'Editing',multiedit:'Editing',notebookedit:'Editing',write:'Writing',delete:'Deleting',
    bash:'Running',shell:'Running',webfetch:'Fetching',websearch:'Searching the web','web search':'Searching the web',task:'Delegating to a helper',agent:'Delegating to a helper',
    todowrite:'Updating the plan',updatetodos:'Updating the plan'};
  // Labels name the tool and nothing else, so the Now line says what kind of step it is.
  const VAGUE = {read:'Reading a file',grep:'Searching',glob:'Searching for files',ls:'Listing files',edit:'Editing a file',multiedit:'Editing a file',write:'Writing a file',delete:'Deleting a file',
    bash:'Running a command',shell:'Running a command',webfetch:'Fetching a page',websearch:'Searching the web','web search':'Searching the web',task:'Delegating to a helper',
    agent:'Delegating to a helper',todowrite:'Updating the plan',updatetodos:'Updating the plan',mcp:'Using an MCP tool'};
  const PLURAL = {read:n => `Reading ${n} files`,edit:n => `Editing ${n} files`,grep:n => `Running ${n} searches`,glob:n => `Running ${n} searches`,bash:n => `Running ${n} commands`,shell:n => `Running ${n} commands`};
  const COLD = ['vendor','node_modules','storage','var/cache','.git'];
  const STATUSES = ['done','active','pending','cancelled'], OUTCOMES = ['completed','failed','cancelled','interrupted'];
  // A launch that raised (an output limit, a lost process) ends on the worker's error line, not on a closing status.
  const ABORTED = /^Run failed \([A-Za-z]\w*\)\. /;
  const count = (n, one, many = one + 's') => `${n} ${n === 1 ? one : many}`;
  const tally = tools => [...tools].map(([name,n]) => n > 1 ? `${name} ×${n}` : name).join(', ');

  // A tool's name: the enriched `tool` field, else the label before ': '. Cursor keys lose ToolCall; Codex item types read as tools.
  function toolName(event) {
    const text = typeof event.text === 'string' ? event.text : '', cut = text.indexOf(': ');
    let name = typeof event.tool === 'string' && event.tool ? event.tool : cut > 0 ? text.slice(0,cut) : text;
    name = clean(name).replace(/ToolCall$/,'').trim();
    if (CODEX[name]) name = CODEX[name]; else if (/^Agent\b/.test(name) && !event.tool) name = 'Agent';
    return name ? name[0].toUpperCase() + name.slice(1,60) : 'Tool';
  }
  // started/in_progress opens a step; completed/failed closes one. Enriched events say so in `state`.
  function isStart(event) {
    if (event.state === 'started' || event.state === 'completed') return event.state === 'started';
    const text = typeof event.text === 'string' ? event.text : '', cut = text.indexOf(': ');
    return cut < 0 || !/^(?:completed|failed|declined)\b/.test(text.slice(cut + 2));
  }
  const callOf = event => typeof event.call === 'string' && /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/.test(event.call) ? event.call : null;
  const pathOf = value => typeof value === 'string' && value ? clean(value).replace(/\\/g,'/').replace(/^\.\//,'').slice(0,400) : null;
  const detailOf = value => typeof value === 'string' && value.trim() ? clean(value).replace(/\s+/g,' ').trim().slice(0,300) : null;
  const changesOf = value => Array.isArray(value) ? value.slice(0,20).filter(change => change && pathOf(change.path)).map(change => ({path:pathOf(change.path),kind:['add','update','delete'].includes(change.kind) ? change.kind : 'update',outside:change.outside === true})) : null;
  // Codex names its helper operations in the detail of an Agent call.
  const AGENT = {spawn_agent:'Starting a helper',wait:'Waiting for helpers',close_agent:'Closing a helper',send_input:'Messaging a helper'};
  // Codex repeats a helper operation's label while it runs (item.started, then item.updated) and only the start carries
  // targets: a label-only repeat of an operation already open is one step, not two.
  const PROGRESS = /^Agent (spawn_agent|send_input|wait|close_agent): in_progress$/;
  const progressOf = event => callOf(event) === null && typeof event.tool !== 'string' && typeof event.text === 'string' ? PROGRESS.exec(event.text)?.[1] || null : null;
  const verbOf = call => call.changes ? 'Changing files' : call.key === 'agent' && AGENT[call.detail] ? AGENT[call.detail]
    : /^mcp/i.test(call.tool) ? 'Using an MCP tool' : VERB[call.key] || `Using ${call.tool}`;
  function targetOf(call) {
    if (call.changes) return call.changes.length > 3 ? `${call.changes.slice(0,3).map(change => change.path).join(', ')} +${call.changes.length - 3}` : call.changes.map(change => change.path).join(', ');
    if (SEARCH.has(call.key)) return [call.detail,call.path ? `in ${call.path}` : ''].filter(Boolean).join(' ');
    if (call.path && !COMMAND.has(call.key)) return call.outside ? `${call.path} [outside project]` : call.path;
    if (call.key === 'agent' && AGENT[call.detail]) return call.detail;
    return call.detail || '';
  }
  // One step as the log shows it: name, target and the call it belongs to. Pure, for the conversation's step rows.
  function describe(event) {
    const tool = toolName(event), call = {tool,key:tool.toLowerCase(),path:pathOf(event.path),detail:detailOf(event.detail),outside:event.outside === true,changes:changesOf(event.changes)};
    return {tool,key:call.key,call:callOf(event),start:isStart(event),target:PLAN.has(call.key) ? '' : targetOf(call),helper:typeof event.parent === 'string',
      op:call.key === 'agent' && AGENT[call.detail] ? call.detail : null,progress:progressOf(event),
      paths:call.changes ? call.changes.map(change => change.path) : call.path && (READ.has(call.key) || EDIT.has(call.key)) ? [call.path] : [],
      pattern:SEARCH.has(call.key) && call.detail ? call.detail : null,read:READ.has(call.key),edit:EDIT.has(call.key) || Boolean(call.changes)};
  }
  // A step group's summary: what ran, what was looked for and what was opened. Search and file are related only by order.
  function groupSummary(group) {
    const parts = [count(group.steps,'step')];
    if (group.tools.size) parts.push(tally(group.tools));
    if (group.patterns.length) parts.push(`Looked for ${group.patterns.slice(0,3).join(', ')}${group.patterns.length > 3 ? ` +${group.patterns.length - 3}` : ''}`);
    if (group.opened.size) parts.push(`opened ${count(group.opened.size,'file')}`);
    if (group.edited.size) parts.push(`edited ${group.edited.size}`);
    return parts.join(' · ');
  }

  // Shell text → one check. Wrappers (cd, env, timeout, containers, php -d, Windows shells) are peeled off; a pipe, || true,
  // a later ; command or a background & masks the exit status. A fixer changes files and is never a check.
  function shellParts(text) {
    const parts = []; let current = '', quote = null;
    for (let i = 0; i < text.length; i++) {
      const ch = text[i], two = text.slice(i,i + 2);
      if (quote) { current += ch; if (ch === quote) quote = null; else if (ch === '\\' && quote === '"' && text[i + 1] === '"') current += text[++i]; continue; }
      if (ch === '"' || ch === "'") { quote = ch; current += ch; continue; }
      if (two === '&&' || two === '||') { parts.push({text:current.trim(),next:two}); current = ''; i++; continue; }
      if (ch === '|' || ch === ';') { parts.push({text:current.trim(),next:ch}); current = ''; continue; }
      // `2>&1` and `&>` are redirects; PowerShell's leading `&` calls a program.
      if (ch === '&' && text[i - 1] !== '>' && text[i + 1] !== '>') { if (current.trim()) { parts.push({text:current.trim(),next:'&'}); current = ''; } continue; }
      current += ch;
    }
    parts.push({text:current.trim(),next:null});
    return parts.filter((part,index) => part.text || index === parts.length - 1);
  }
  function words(text) {
    const list = []; let raw = '', value = '', quote = null, open = false;
    for (let i = 0; i <= text.length; i++) {
      const ch = text[i];
      if (ch === undefined || !quote && /\s/.test(ch)) { if (open) list.push({raw,value}); raw = ''; value = ''; open = false; continue; }
      open = true; raw += ch;
      if (quote) { if (ch === quote) quote = null; else if (ch === '\\' && quote === '"' && text[i + 1] === '"') { raw += text[++i]; value += '"'; } else value += ch; }
      else if (ch === '"' || ch === "'") quote = ch; else value += ch;
    }
    return list;
  }
  const exe = word => word.value.replace(/\\/g,'/').replace(/^\.\//,'').split('/').pop().toLowerCase().replace(/\.(?:bat|cmd|exe|phar)$/,'');
  const FAMILIES = {phpunit:['PHPUnit','tests'],'simple-phpunit':['PHPUnit','tests'],pest:['Pest','tests'],paratest:['ParaTest','tests'],phpstan:['PHPStan','project'],psalm:['Psalm','project'],
    phpcs:['PHPCS','project'],phpcbf:['PHPCBF','project'],'parallel-lint':['PHP lint','project'],phpmd:['PHPMD','project'],deptrac:['Deptrac','project'],behat:['Behat','tests'],
    infection:['Infection','tests'],pint:['Pint','project'],'php-cs-fixer':['PHP-CS-Fixer','project'],rector:['Rector','project']};
  const VALUE_FLAGS = new Set(['-c','--configuration','--memory-limit','--error-format','--standard','--report','--format','--bootstrap','--log-junit','--cache-dir','--config','--set','--autoload-file','--threads','-j','--processes','-p']);
  function significant(args, family) {
    const kept = [];
    for (let i = 0; i < args.length; i++) {
      const arg = args[i].value, flag = /^--(filter|testsuite|group|exclude-group|level)(?:=(.*))?$/.exec(arg) || (family === 'PHPStan' && arg === '-l' ? [arg,'level'] : null);
      if (flag) { const value = flag[2] ?? args[++i]?.value; kept.push(`--${flag[1]}=${value ?? ''}`); continue; }
      if (arg.startsWith('-')) { if (VALUE_FLAGS.has(arg)) i++; continue; }
      kept.push(arg.replace(/\\/g,'/'));
    }
    return kept.join(' ');
  }
  function identify(list) {
    let i = 0; const name = () => list[i] ? exe(list[i]) : '';
    if (/^php(?:\d+(?:\.\d+)?)?$/.test(name())) {
      if (list[i + 1]?.value === '-l' && list[i + 2]) return {family:'PHP lint',target:list[i + 2].value.replace(/\\/g,'/'),check:true};
      i++; while (list[i] && list[i].value.startsWith('-')) i += /^-[dc]$/.test(list[i].value) ? 2 : 1;
    }
    const args = () => list.slice(i + 1), sub = list[i + 1]?.value || '';
    if (name() === 'artisan') return sub === 'test' ? {family:'Artisan test',target:significant(list.slice(i + 2),'Artisan test') || 'all tests',check:true} : null;
    if (name() === 'composer') {
      if (sub === 'exec') { i += list[i + 2]?.value === '--' ? 3 : 2; return identify(list.slice(i)); }
      if (['validate','audit'].includes(sub)) return {family:'Composer',target:sub,check:true};
      if (sub === 'test' || ['run','run-script'].includes(sub) && list[i + 2]?.value === 'test') return {family:'Composer',target:'test',check:true};
      return null;
    }
    if (name() === 'console' && /^lint:\S+$/.test(sub)) return {family:'Symfony lint',target:[sub,significant(list.slice(i + 2))].filter(Boolean).join(' '),check:true};
    if (name() === 'console' && sub === 'doctrine:schema:validate') return {family:'Doctrine schema',target:'validate',check:true};
    if (name() === 'wp' && /^(?:core|plugin)$/.test(sub) && list[i + 2]?.value === 'verify-checksums') return {family:'WP checksums',target:sub,check:true};
    if (name() === 'codecept' && sub === 'run') return {family:'Codeception',target:significant(list.slice(i + 2)) || 'all tests',check:true};
    const known = FAMILIES[name()]; if (!known || list.slice(i + 1).some(word => ['--version','--help','-V','-h'].includes(word.value))) return null;
    const [family,scope] = known, flags = list.slice(i + 1).map(word => word.value);
    if (family === 'PHPStan') { if (sub && !sub.startsWith('-') && !/^analy[sz]e$/.test(sub)) return null; const rest = /^analy[sz]e$/.test(sub) ? list.slice(i + 2) : args(); return {family,target:significant(rest,family) || 'project',check:true}; }
    if (family === 'PHP-CS-Fixer') { if (!['fix','check'].includes(sub)) return null; const fixer = sub === 'fix' && !flags.includes('--dry-run'); return {family,target:significant(list.slice(i + 2)) || scope,check:!fixer,fixer}; }
    if (family === 'Rector') { const rest = sub === 'process' ? list.slice(i + 2) : args(), fixer = !flags.includes('--dry-run'); return {family,target:significant(rest) || scope,check:!fixer,fixer}; }
    const fixer = family === 'PHPCBF' || family === 'Pint' && !flags.includes('--test') || family === 'Psalm' && flags.some(flag => flag.startsWith('--alter'));
    return {family,target:significant(args(),family) || (scope === 'tests' ? 'all tests' : 'project'),check:!fixer,fixer};
  }
  // Segments that only prepare the shell; an assignment alone is one, an assignment before a command is peeled off below.
  const SETUP = /^(?:cd|pushd|popd|export|set|source|\.|Set-Location)(?:\s|$)|^\$env:|^(?:[A-Za-z_][A-Za-z0-9_]*=\S*\s*)+$/;
  const HARMLESS = /^(?:echo|printf|true|:|exit \$(?:\?|LASTEXITCODE))(?:\s|$)/;
  const shown = word => word.replace(/\\/g,'/').split('/').pop();
  function classifyCommand(raw) {
    let text = clean(raw).trim(); const removed = [];
    // Shells that wrap one command: bash -lc '…', pwsh -Command "…", cmd /c ….
    for (let unwrapped = true; unwrapped;) {
      unwrapped = false;
      const shell = /^(?:\S*\/)?(?:ba|z)?sh\s+-l?c\s+(['"])([\s\S]*)\1$/.exec(text)
        || /^(?:"[^"]*(?:pwsh|powershell)(?:\.exe)?"|\S*(?:pwsh|powershell)(?:\.exe)?)(?:\s+-(?:NoProfile|NonInteractive|NoLogo|ExecutionPolicy\s+\S+))*\s+-(?:Command|c)\s+(['"]?)([\s\S]*)\1$/i.exec(text)
        || /^cmd(?:\.exe)?\s+\/[cC]\s+(["']?)([\s\S]*)\1$/.exec(text);
      if (shell) { text = shell[2].trim(); unwrapped = true; }
    }
    const parts = shellParts(text); let index = -1, found = null, list = [];
    for (let i = 0; i < parts.length && !found; i++) {
      if (SETUP.test(parts[i].text)) continue;
      list = words(parts[i].text);
      // Prefixes that only change how the command starts: environment, timeouts and container wrappers.
      for (let peeled = true; peeled && list.length;) {
        peeled = false; const first = list[0].value, base = exe(list[0]);
        if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(first)) { list.shift(); removed.push('environment'); peeled = true; }
        else if (first === 'env') { list.shift(); while (list[0] && /^[A-Za-z_][A-Za-z0-9_]*=/.test(list[0].value)) list.shift(); removed.push('environment'); peeled = true; }
        else if (first === 'timeout') { list.shift(); while (list[0]?.value.startsWith('-')) list.shift(); list.shift(); removed.push('timeout'); peeled = true; }
        else if (['time','nice','nohup','command','call'].includes(first)) { list.shift(); peeled = true; }
        else if (base === 'sail') {
          list.shift(); removed.push('Sail'); peeled = true; const sub = list[0]?.value;
          if (sub === 'test') list.splice(0,1,{raw:'php',value:'php'},{raw:'artisan',value:'artisan'},{raw:'test',value:'test'});
          else if (sub === 'artisan') list.unshift({raw:'php',value:'php'});
          else if (sub === 'bin' && list[1]) list.splice(0,2,{raw:`vendor/bin/${list[1].raw}`,value:`vendor/bin/${list[1].value}`});
          else if (sub === 'pint') list.splice(0,1,{raw:'vendor/bin/pint',value:'vendor/bin/pint'});
        }
        else if (base === 'ddev' || base === 'lando') { list.shift(); if (list[0]?.value === 'exec') list.shift(); removed.push(base === 'ddev' ? 'DDEV' : 'Lando'); peeled = true; }
        else if (base === 'docker' || base === 'docker-compose') {
          let at = base === 'docker' && list[1]?.value === 'compose' ? 2 : 1;
          if (!['exec','run'].includes(list[at]?.value)) break;
          at++; while (list[at]?.value.startsWith('-')) at += /^(?:-u|--user|-w|--workdir|-e|--env)$/.test(list[at].value) ? 2 : 1;
          list.splice(0,at + 1); removed.push('container'); peeled = true;
        }
      }
      // Redirects are not arguments; a check run by Harness keeps its own output.
      const redirects = list.filter(word => /^\d*>>?(?:&\d)?$|^\d*>>?\S+$|^<$/.test(word.raw));
      if (redirects.length) { for (let r = list.length - 1; r >= 0; r--) if (redirects.includes(list[r])) { list.splice(r,/^\d*>>?$|^<$/.test(list[r].raw) ? 2 : 1); } removed.push('redirect'); }
      found = list.length ? identify(list) : null; if (found) index = i;
    }
    if (!found) return {family:null,target:'',key:null,check:false,fixer:false,masked:null,bare:'',removed:[],runnable:false};
    const before = parts.slice(0,index).map(part => part.text);
    if (before.some(text => /^(?:export|set|source|\.)\s|^\$env:|^[A-Za-z_][A-Za-z0-9_]*=/.test(text) && !/pipefail/.test(text))) removed.unshift('environment');
    if (before.some(text => /^(?:cd|pushd|Set-Location)\b/.test(text))) removed.unshift('cd');
    let masked = null; const next = parts[index].next, after = parts[index + 1]?.text || '', word = words(after)[0]?.value || 'the next command';
    const pipefail = parts.slice(0,index).some(part => /\bpipefail\b/.test(part.text));
    if (next === '|') { if (!pipefail) masked = `piped to ${shown(word)}: exit status is ${shown(word)}'s`; removed.push(`| ${shown(word)}`); }
    else if (next === '||' && /^(?:true|:|exit 0|echo)(?:\s|$)/.test(after)) { masked = `|| ${word}: a failure is ignored`; removed.push(`|| ${word}`); }
    else if (next === ';' && after && !/^exit \$(?:\?|LASTEXITCODE)\b/.test(after)) { masked = `followed by ${word}: the last command sets the exit status`; removed.push(`; ${word}`); }
    else if (next === '&&' && after && !HARMLESS.test(after)) { masked = `followed by ${word}: a failure may be its`; removed.push(`&& ${word}`); }
    else if (next === '&') { masked = 'runs in the background: the exit status is not the check\'s'; removed.push('&'); }
    const bare = list.map(item => item.raw).join(' '), target = found.target || '';
    return {family:found.family,target,key:`${found.family}|${target}`,check:Boolean(found.check),fixer:Boolean(found.fixer),masked,bare,removed:[...new Set(removed)],runnable:runnable(bare)};
  }
  // What Results.check_command accepts: one command, no shell operators, at most 4000 bytes and 100 words.
  function runnable(command) {
    if (typeof command !== 'string' || !command.trim() || /[\x00-\x1f]/.test(command) || new TextEncoder().encode(command).length > 4000) return false;
    if ((command.match(/"/g) || []).length % 2 || (command.match(/'/g) || []).length % 2) return false;
    const list = words(command); return list.length > 0 && list.length <= 100 && !list.some(word => ['|','||','&&',';','>','>>','<'].includes(word.raw));
  }

  const changes = () => ({live:true,runs:new Set(),newPaths:[],touched:new Set(),edited:new Set(),failed:new Set(),started:[],milestones:[],plan:false,commands:false,finished:null,compaction:false,limited:false,memory:false,check:false});
  const create = () => ({runs:new Map(),order:[],offset:null,lastAt:null,lastUserId:0,lastLaunchEventId:0,seen:new Set()});
  function newRun(model, id) {
    return {id,ordinal:model.order.length + 1,startAt:null,cliAt:null,endAt:null,outcome:null,finished:false,aborted:false,closingEventId:null,approx:false,kinds:new Set(),mode:null,
      calls:new Map(),open:new Set(),openHelpers:new Set(),anon:[],paths:new Map(),searches:[],commands:[],plan:null,planChange:null,dropped:[],helpers:new Map(),
      counters:{steps:0,opened:0,edited:0,edits:0,searched:0,commands:0,failed:0,notRun:0,messages:0,commandFailed:0},tools:new Map(),moments:[],limited:null,
      compactions:[],usage:null,result:null,memory:null,error:null,delegation:null,editLog:[],firstEventId:null,lastEventId:null,firstFailedEventId:null};
  }
  const moment = (run, kind, label, eventId, at, repeat = false) => { if (repeat || !run.moments.some(item => item.kind === kind)) run.moments.push({kind,label,eventId,at}); };
  function touch(model, run, call, how, path, outside, at, ch) {
    const key = (outside ? 'outside:' : '') + path; let p = run.paths.get(key); const fresh = !p;
    if (!p) { p = {key,path,outside,opened:0,edited:false,edits:0,created:false,deleted:false,failed:false,helper:false,main:false,pending:null,firstEventId:call.eventId,lastEventId:call.eventId,firstStep:call.step,lastStep:call.step,firstAt:at,lastAt:at}; run.paths.set(key,p); ch.newPaths.push(key); }
    if (how === 'open') { if (!p.opened) run.counters.opened++; p.opened++; }
    // Claude's Write refuses a file it has not read in the session, so a Write to a path not seen in this session creates it.
    if (how === 'write' && fresh && !model.seen.has(key)) p.pending = 'create';
    if (how === 'add') p.pending = 'create'; if (how === 'delete') p.pending = 'delete';
    p.lastEventId = call.eventId; p.lastStep = call.step; p.lastAt = at; if (call.parent) p.helper = true; else p.main = true;
    model.seen.add(key); call.pathKeys.push(key); ch.touched.add(key);
  }
  function markEdited(run, p, call, at, ch) {
    if (!p.edited) run.counters.edited++; p.edited = true; p.edits++; run.counters.edits++;
    if (p.pending === 'create') p.created = true; if (p.pending === 'delete') p.deleted = true; p.pending = null;
    run.editLog.push({at,key:p.key,step:call.step}); ch.edited.add(p.key);
    if (!run.moments.some(item => item.kind === 'first-edit')) { moment(run,'first-edit','First edit',call.eventId,call.startAt); ch.milestones.push(`First edit: ${p.path}`); }
  }
  // A Glob without a path searches from its pattern's leading folders: "app/**/*Order*.php" looks in app/, "**/*.php" everywhere.
  const globRoot = pattern => { const dirs = []; for (const part of String(pattern || '').replace(/^"|"$/g,'').replace(/^\.\//,'').split('/').slice(0,-1)) { if (!part || part === '.' || part === '..' || /[*?[\]{}]/.test(part)) break; dirs.push(part); } return dirs.join('/'); };
  function openCall(model, run, event, at, ch) {
    const tool = toolName(event), key = tool.toLowerCase(), call = callOf(event);
    run.counters.steps++; run.tools.set(tool,(run.tools.get(tool) || 0) + 1);
    if (!call) { run.anon.push({tool,key,op:progressOf(event),startAt:at,eventId:event.id}); if (COMMAND.has(key)) run.counters.commands++; ch.started.push({tool,anon:true}); return null; }
    const c = {call,tool,key,path:pathOf(event.path),detail:detailOf(event.detail),outside:event.outside === true,parent:typeof event.parent === 'string' ? event.parent : null,changes:changesOf(event.changes),
      state:'running',ok:null,outcome:null,exitCode:null,startAt:at,endAt:null,eventId:event.id,endEventId:null,step:run.counters.steps,pathKeys:[],cmd:null};
    run.calls.set(call,c); ch.started.push(c);
    if (c.parent) run.openHelpers.add(call); else run.open.add(call);
    if (c.changes) for (const change of c.changes) touch(model,run,c,change.kind,change.path,change.outside,at,ch);
    else if (READ.has(key) && c.path) touch(model,run,c,'open',c.path,c.outside,at,ch);
    // Cursor's delete tool removes the file it names; like an edit, it counts once it reports success.
    else if (EDIT.has(key) && c.path) touch(model,run,c,key === 'write' || key === 'delete' ? key : 'edit',c.path,c.outside,at,ch);
    else if (SEARCH.has(key)) { run.searches.push({pattern:c.detail || '',scope:c.path ? c.path.replace(/\/$/,'') : key === 'glob' ? globRoot(c.detail) : '',eventId:event.id,step:c.step,at,helper:Boolean(c.parent),tool}); run.counters.searched++; }
    else if (COMMAND.has(key)) { c.cmd = {command:c.detail || '',cls:classifyCommand(c.detail || ''),state:'running',ok:null,outcome:null,exitCode:null,startAt:at,endAt:null,eventId:event.id,call,parent:c.parent}; run.commands.push(c.cmd); run.counters.commands++; ch.commands = true; }
    else if (TASK.has(key)) run.helpers.set(call,{label:c.detail || 'Helper',state:'working',steps:0,startAt:at,eventId:event.id});
    if (c.parent && run.helpers.has(c.parent)) run.helpers.get(c.parent).steps++;
    return c;
  }
  function closeCall(run, c, event, at, ch) {
    c.state = 'completed'; c.ok = event.ok !== false; c.outcome = event.outcome === 'not_run' ? 'not_run' : null; c.exitCode = Number.isInteger(event.exit_code) ? event.exit_code : null; c.endAt = at; c.endEventId = event.id;
    if (!c.detail) c.detail = detailOf(event.detail);
    run.open.delete(c.call); run.openHelpers.delete(c.call);
    const notRun = c.outcome === 'not_run';
    if (!c.ok) { if (notRun) run.counters.notRun++; else { run.counters.failed++; run.firstFailedEventId ??= c.eventId; } }
    for (const key of c.pathKeys) {
      const p = run.paths.get(key); if (!p) continue; ch.touched.add(key);
      if (!c.ok) { if (!notRun) { p.failed = true; ch.failed.add(key); } p.pending = null; } else if (EDIT.has(c.key) || c.changes) markEdited(run,p,c,at,ch);
    }
    if (c.cmd) {
      const cmd = c.cmd; Object.assign(cmd,{state:'done',ok:c.ok,outcome:c.outcome,exitCode:c.exitCode,endAt:at}); ch.commands = true;
      if (!c.ok && !notRun) { run.counters.commandFailed++; if (!run.moments.some(item => item.kind === 'first-failed-command')) { moment(run,'first-failed-command','First failed command',cmd.eventId,at); ch.milestones.push(`Command failed: ${cmd.command}`); } }
      if (cmd.cls.check && !cmd.cls.masked && c.ok && !notRun) {
        const runs = run.commands.filter(item => item.cls.key === cmd.cls.key && item.state === 'done' && item.outcome !== 'not_run');
        if (runs.length > 1 && runs[runs.length - 2].ok === false) { moment(run,'check-green',`${cmd.cls.family} passed`,cmd.eventId,at,true); ch.milestones.push(`${cmd.cls.family} passed after ${runs.length} runs`); }
      }
    }
    if (TASK.has(c.key) && run.helpers.has(c.call)) Object.assign(run.helpers.get(c.call),{state:c.ok ? 'joined' : 'failed'});
  }
  function onTool(model, run, event, at, ch) {
    const enriched = callOf(event) !== null || typeof event.tool === 'string';
    if (enriched) run.mode = 'targets'; else if (!run.mode) run.mode = 'labels';
    const call = callOf(event), op = progressOf(event);
    if (op && ([...run.open].some(id => run.calls.get(id).key === 'agent' && run.calls.get(id).detail === op) || run.anon.some(item => item.op === op))) return;
    if (isStart(event)) { if (!call || !run.calls.has(call)) openCall(model,run,event,at,ch); return; }
    let c = call ? run.calls.get(call) : null;
    // Codex declines and Cursor completions can arrive without a start: the step starts and ends at once.
    if (!c && call && typeof event.tool === 'string') c = openCall(model,run,event,at,ch);
    if (c) { if (c.state === 'running') closeCall(run,c,event,at,ch); return; }
    const tool = toolName(event), index = run.anon.findIndex(item => item.tool === tool);
    run.anon.splice(index >= 0 ? index : 0,1);
    // A label-only failure names no call: the step row of its own completion is the one to show.
    if (event.ok === false) { if (event.outcome === 'not_run') run.counters.notRun++; else { run.counters.failed++; run.firstFailedEventId ??= event.id ?? null; if (COMMAND.has(tool.toLowerCase())) run.counters.commandFailed++; } }
  }
  function onPlan(run, event, at, ch) {
    const items = (Array.isArray(event.items) ? event.items : []).filter(item => item && typeof item.text === 'string' && item.text.trim()).slice(0,100)
      .map(item => ({text:clean(item.text).trim().slice(0,300),status:STATUSES.includes(item.status) ? item.status : 'pending'}));
    const previous = run.plan, added = [], dropped = [];
    if (previous) {
      const before = new Set(previous.items.map(item => item.text)), after = new Set(items.map(item => item.text));
      for (const item of items) if (!before.has(item.text)) added.push(item.text);
      for (const item of previous.items) if (!after.has(item.text)) dropped.push(item.text);
      if (added.length || dropped.length) { run.planChange = {at,added,dropped,eventId:event.id}; moment(run,'plan-changed','Plan changed',event.id,at); }
      run.dropped = [...run.dropped.filter(text => !after.has(text)),...dropped.filter(text => !run.dropped.includes(text))];
    }
    run.plan = {items,total:Number.isInteger(event.total) && event.total >= items.length ? event.total : items.length,activeForm:typeof event.active_form === 'string' && event.active_form.trim() ? clean(event.active_form).trim().slice(0,200) : null,
      at,eventId:event.id,added:new Set(added)};
    ch.plan = true;
  }
  function finish(run, outcome, at, eventId, ch) {
    Object.assign(run,{finished:true,outcome,endAt:at,closingEventId:eventId});
    // Calls still open when the launch ended never reported a result.
    for (const id of [...run.open,...run.openHelpers]) { const c = run.calls.get(id); c.state = 'unfinished'; if (c.cmd) c.cmd.state = 'unfinished'; }
    run.open.clear(); run.openHelpers.clear(); run.anon = []; ch.finished = run.id; ch.milestones.push('Run finished');
  }
  function onStatus(run, event, at, ch) {
    if (event.compaction && typeof event.compaction === 'object') {
      run.compactions.push({pre:Number.isFinite(event.compaction.pre) ? event.compaction.pre : null,post:Number.isFinite(event.compaction.post) ? event.compaction.post : null,at,eventId:event.id});
      moment(run,'compaction','Compaction',event.id,at,true); ch.compaction = true; return;
    }
    if (event.targets === 'limited') {
      if (!run.limited) {
        // Later completions are plain labels without their call: calls still open wait as unnamed steps, which those labels
        // close in order, and their own results stay unknown.
        for (const id of [...run.open,...run.openHelpers]) { const c = run.calls.get(id); c.state = 'unfinished'; if (c.cmd) c.cmd.state = 'unfinished'; run.anon.push({tool:c.tool,key:c.key,op:c.key === 'agent' && AGENT[c.detail] ? c.detail : null,startAt:c.startAt,eventId:c.eventId}); }
        run.open.clear(); run.openHelpers.clear();
        run.limited = {at,step:run.counters.steps,eventId:event.id}; moment(run,'limit','Step details stop',event.id,at); ch.limited = true; ch.milestones.push('Step details are no longer recorded');
      }
      return;
    }
    const closing = /^Run (completed|failed|cancelled|interrupted)\. Process completion is not/.exec(typeof event.text === 'string' ? event.text : '');
    if (OUTCOMES.includes(event.outcome) || closing) finish(run,OUTCOMES.includes(event.outcome) ? event.outcome : closing[1],at,event.id,ch);
  }
  // Events in, changes out. `at` is the server's clock; without it a live event uses its arrival and history has no time.
  function observe(model, events, {arrival = Date.now(), live = false, changes:ch = changes()} = {}) {
    for (const event of events) {
      if (!event || typeof event !== 'object') continue;
      const stamp = typeof event.at === 'string' ? Date.parse(event.at) : NaN;
      if (live && Number.isFinite(stamp)) model.offset = model.offset === null ? arrival - stamp : Math.min(model.offset,arrival - stamp);
      const at = Number.isFinite(stamp) ? stamp : live ? arrival - (model.offset || 0) : null;
      if (at !== null) model.lastAt = at;
      const id = Number(event.id);
      if (event.kind === 'user') { if (Number.isFinite(id)) model.lastUserId = id; continue; }
      if (event.kind === 'status' && /^Check \w+: /.test(typeof event.text === 'string' ? event.text : '')) ch.check = true;
      const launch = typeof event.launch_id === 'string' && event.launch_id ? event.launch_id : null; if (!launch) continue;
      if (Number.isFinite(id)) model.lastLaunchEventId = id;
      let run = model.runs.get(launch); if (!run) { run = newRun(model,launch); model.runs.set(launch,run); model.order.push(launch); }
      ch.runs.add(launch); run.kinds.add(event.kind);
      if (run.firstEventId === null) run.firstEventId = event.id; run.lastEventId = event.id;
      if (run.startAt === null && at !== null) run.startAt = at; if (at === null || !Number.isFinite(stamp)) run.approx = true;
      if (run.cliAt === null && ['session','text','tool','plan','usage','result','error'].includes(event.kind)) run.cliAt = at ?? 0;
      if (run.finished && event.kind !== 'memory') continue;
      if (event.kind === 'tool') onTool(model,run,event,at,ch);
      else if (event.kind === 'plan') onPlan(run,event,at,ch);
      else if (event.kind === 'status') onStatus(run,event,at,ch);
      else if (event.kind === 'text') run.counters.messages++;
      else if (event.kind === 'usage') run.usage = event;
      else if (event.kind === 'result') { run.result = {ok:event.ok !== false,eventId:event.id}; if (event.ok === false && !run.error) { run.error = {eventId:event.id}; moment(run,'error','Error',event.id,at); } }
      else if (event.kind === 'error') {
        if (!run.error) { run.error = {eventId:event.id,text:clean(event.text).slice(0,300)}; moment(run,'error','Error',event.id,at); }
        if (ABORTED.test(typeof event.text === 'string' ? event.text : '')) { run.aborted = true; finish(run,'failed',at,event.id,ch); }
      }
      else if (event.kind === 'memory') { run.memory = {ok:event.ok !== false,text:clean(event.text).slice(0,600),eventId:event.id}; ch.memory = true; }
      else if (event.kind === 'delegation') run.delegation = {status:event.status,text:clean(event.text).slice(0,400),required:Number.isInteger(event.required_count)};
    }
    return ch;
  }
  const current = model => model.runs.get(model.order[model.order.length - 1]) || null;
  // A page animates only when it is news: history and catch-up pages of 250 are drawn as they end (design 3.5).
  const live = ({loaded, count, running, hidden}) => Boolean(loaded && count < 250 && running && !hidden);
  // Fleet, Clash and System runs keep their own surfaces; the Run view reads native and SDD launches.
  const native = run => Boolean(run) && !['fleet_stage','fleet_reviewer','fleet_state','clash_turn','clash_state','agent','agent_activity'].some(kind => run.kinds.has(kind));

  // What the Now line says. Clocks are seconds since the call started, on the server's clock; null means unknown, never 0.
  function now(model, run, nowMs = Date.now()) {
    if (!run || run.finished) return null;
    const server = nowMs - (model.offset || 0), since = at => at === null || at === undefined ? null : Math.max(0,(server - at) / 1000);
    if (run.cliAt === null) return {kind:'preparing'};
    const open = [...run.open].map(id => run.calls.get(id)), helping = [...run.openHelpers].map(id => run.calls.get(id));
    if (run.anon.length && !open.length) {
      const first = run.anon[0];
      return run.anon.length === 1 ? {kind:'labels',verb:VAGUE[first.key] || `Using ${first.tool}`,target:'',clock:since(first.startAt),eventId:first.eventId}
        : {kind:'labels',verb:`${run.anon.length} steps running`,target:'',clock:null,eventId:first.eventId,count:run.anon.length};
    }
    if (open.length === 1 && TASK.has(open[0].key) && helping.length) {
      const h = helping[helping.length - 1];
      return {kind:'helper',verb:`Helper · ${verbOf(h)}`,target:targetOf(h),path:Boolean(h.path) && !SEARCH.has(h.key) && !COMMAND.has(h.key),call:h,clock:since(h.startAt),eventId:h.eventId,helpers:run.openHelpers.size};
    }
    if (open.length + run.anon.length === 1) { const c = open[0]; return {kind:'call',verb:verbOf(c),target:PLAN.has(c.key) || c.key === 'agent' && AGENT[c.detail] ? '' : targetOf(c),path:Boolean(c.path) && !SEARCH.has(c.key) && !COMMAND.has(c.key),call:c,clock:since(c.startAt),eventId:c.eventId}; }
    if (open.length) {
      const first = open[0], earliest = Math.min(...open.map(c => c.startAt ?? Infinity)), n = open.length + run.anon.length;
      if (!run.anon.length && open.every(c => c.key === first.key) && PLURAL[first.key]) return {kind:'parallel',verb:PLURAL[first.key](n),target:targetOf(first),path:Boolean(first.path) && (READ.has(first.key) || EDIT.has(first.key)),more:`+${n - 1}`,clock:Number.isFinite(earliest) ? since(earliest) : null,eventId:first.eventId,count:n};
      return {kind:'parallel',verb:`${n} steps running`,target:'',clock:Number.isFinite(earliest) ? since(earliest) : null,eventId:first.eventId,count:n};
    }
    const quiet = since(model.lastAt);
    return {kind:'model',verb:"Model's turn",target:'',clock:quiet,quiet};
  }
  // Check series in first-appearance order: Artisan test ✗ ✗ ✓, PHPStan ?.
  const glyph = cmd => cmd.state === 'running' ? 'run' : cmd.state === 'unfinished' ? 'unknown' : cmd.outcome === 'not_run' ? 'notrun' : cmd.cls.masked ? 'unknown' : cmd.ok ? 'ok' : 'fail';
  function checks(run) {
    const rows = new Map();
    for (const cmd of run.commands) {
      if (!cmd.cls.check) continue;
      let row = rows.get(cmd.cls.key); if (!row) { row = {key:cmd.cls.key,family:cmd.cls.family,target:cmd.cls.target,bare:cmd.cls.bare,removed:cmd.cls.removed,runnable:cmd.cls.runnable,masked:null,runs:[]}; rows.set(cmd.cls.key,row); }
      row.runs.push({glyph:glyph(cmd),cmd}); row.masked = cmd.cls.masked;
    }
    for (const row of rows.values()) {
      const done = row.runs.filter(item => !['run','notrun'].includes(item.glyph)); row.last = row.runs[row.runs.length - 1];
      let streak = 0; row.maxFails = 0; for (const item of done) { streak = item.glyph === 'fail' ? streak + 1 : 0; row.maxFails = Math.max(row.maxFails,streak); }
      const last = done[done.length - 1];
      if (last?.glyph === 'ok' && done.length > 1 && done[done.length - 2].glyph === 'fail') {
        let i = done.length - 2; while (i > 0 && done[i - 1].glyph === 'fail') i--;
        const from = done[i].cmd.endAt, to = last.cmd.startAt;
        row.green = {runs:done.length,edits:from === null || to === null ? null : run.editLog.filter(edit => edit.at !== null && edit.at > from && edit.at < to).length};
      }
    }
    return [...rows.values()];
  }
  // Map layout: the top folder is a district (a WordPress plugin or theme is one), up to three more folders make a block.
  function districtOf(p) {
    if (p.outside) return {district:'outside',block:''};
    const cold = COLD.find(prefix => p.path === prefix || p.path.startsWith(prefix + '/')); if (cold) return {district:`cold:${cold}`,block:''};
    const folder = p.path.split('/').slice(0,-1); if (!folder.length) return {district:'',block:''};
    const depth = folder[0] === 'wp-content' && ['plugins','themes'].includes(folder[1]) && folder.length > 2 ? 3 : 1;
    const rest = folder.slice(depth);
    return {district:folder.slice(0,depth).join('/'),block:rest.length ? rest.slice(0,3).join('/') + (rest.length > 3 ? '/…' : '') : ''};
  }
  const searchedIn = (run, p) => p.outside ? [] : run.searches.filter(item => !item.scope || item.scope === '.' || p.path === item.scope || p.path.startsWith(`${item.scope}/`)).map(item => item.pattern).filter(Boolean);
  // What changed while the tab was hidden, from two snapshots of the same run.
  const snapshot = run => ({steps:run.counters.steps,edits:run.editLog.length,messages:run.counters.messages,failed:run.counters.commandFailed,
    checks:new Map(checks(run).map(row => [row.key,row.runs.filter(item => item.glyph !== 'run').map(item => item.glyph)]))});
  const GLYPH = {fail:'✗',ok:'✓',unknown:'?',notrun:'⊘',run:'●'};
  function awaySummary(run, before, minutes) {
    const edits = run.editLog.slice(before.edits), files = new Set(edits.map(edit => edit.key)), parts = [count(run.counters.steps - before.steps,'step')];
    parts.push(edits.length ? `${count(edits.length,'edit')} in ${count(files.size,'file')}` : 'no edits');
    for (const row of checks(run)) {
      const was = before.checks.get(row.key) || [], done = row.runs.filter(item => item.glyph !== 'run').map(item => item.glyph);
      if (done.length > was.length) parts.push(was.length ? `${row.family} ${GLYPH[was[was.length - 1]]} → ${GLYPH[done[done.length - 1]]}` : `${row.family} ${done.slice(was.length).map(item => GLYPH[item]).join(' ')}`);
    }
    if (run.plan) parts.push(`plan ${run.plan.items.filter(item => item.status === 'done').length} of ${run.plan.total}`);
    if (run.finished) parts.unshift(run.outcome === 'completed' ? 'run finished' : `run ${run.outcome}`);
    return `While you were away${minutes ? ` (${minutes} min)` : ''}: ${parts.join(' · ')}`;
  }
  const aborted = event => event?.kind === 'error' && ABORTED.test(typeof event.text === 'string' ? event.text : '');
  return {clean,create,changes,observe,aborted,current,live,native,now,checks,classifyCommand,runnable,toolName,isStart,describe,groupSummary,verbOf,targetOf,districtOf,searchedIn,snapshot,awaySummary,tally,
    kinds:{READ,EDIT,SEARCH,COMMAND,TASK,PLAN},GLYPH};
})();
