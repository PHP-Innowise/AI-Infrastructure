#!/usr/bin/env node
/**
 * Zero-dependency Node 22 ESM Headless Chrome CDP E2E smoke suite for the
 * AI-Infrastructure Harness web UI.
 */
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

function assert(condition, message) {
  if (!condition) {
    throw new Error(`Assertion failed: ${message}`);
  }
}

function parseRgb(cssColor) {
  const m = String(cssColor || '').match(
    /^rgba?\(\s*([\d.]+)\s*[,\s]\s*([\d.]+)\s*[,\s]\s*([\d.]+)(?:\s*[,/]\s*([\d.]+))?\s*\)$/i,
  );
  if (!m) return null;
  return {
    r: Number(m[1]),
    g: Number(m[2]),
    b: Number(m[3]),
    a: m[4] === undefined ? 1 : Number(m[4]),
  };
}

function relativeLuminance({ r, g, b }) {
  const channel = (v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrastRatio(c1, c2) {
  const l1 = relativeLuminance(c1);
  const l2 = relativeLuminance(c2);
  const hi = Math.max(l1, l2);
  const lo = Math.min(l1, l2);
  return (hi + 0.05) / (lo + 0.05);
}

function blendedContrastRatio(fgCss, bgCss, fgOpacity = 1) {
  const fg = parseRgb(fgCss);
  const bg = parseRgb(bgCss);
  if (!fg || !bg) return null;
  const alpha = Math.max(0, Math.min(1, fg.a * fgOpacity));
  const blended = {
    r: fg.r * alpha + bg.r * (1 - alpha),
    g: fg.g * alpha + bg.g * (1 - alpha),
    b: fg.b * alpha + bg.b * (1 - alpha),
  };
  return contrastRatio(blended, bg);
}

function parseArgs(argv) {
  const opts = {
    url: '',
    projectId: '',
    sessionId: '',
    otherSessionId: '',
    commitA: '',
    commitB: '',
    systemRunId: '',
    skillcacheCapsuleId: '',
    skillcacheRequalify: false,
    chrome: process.env.HARNESS_CHROME_BIN || '/opt/google/chrome/chrome',
    timeoutMs: 120_000,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (!arg.startsWith('--')) {
      throw new Error(`Unexpected positional argument: ${arg}`);
    }
    const eq = arg.indexOf('=');
    const key = eq >= 0 ? arg.slice(2, eq) : arg.slice(2);
    const val = eq >= 0 ? arg.slice(eq + 1) : argv[++i];
    if (val === undefined) {
      throw new Error(`Missing value for --${key}`);
    }
    switch (key) {
      case 'url':
        opts.url = val;
        break;
      case 'project-id':
        opts.projectId = val;
        break;
      case 'session-id':
        opts.sessionId = val;
        break;
      case 'other-session-id':
        opts.otherSessionId = val;
        break;
      case 'commit-a':
        opts.commitA = val;
        break;
      case 'commit-b':
        opts.commitB = val;
        break;
      case 'system-run-id':
        opts.systemRunId = val;
        break;
      case 'skillcache-capsule-id':
        opts.skillcacheCapsuleId = val;
        break;
      case 'skillcache-requalify':
        opts.skillcacheRequalify = val === '1' || val === 'true';
        break;
      case 'chrome':
        opts.chrome = val;
        break;
      case 'timeout-ms': {
        const parsed = Number.parseInt(val, 10);
        if (!Number.isFinite(parsed) || parsed <= 0) {
          throw new Error(`Invalid --timeout-ms: ${val}`);
        }
        opts.timeoutMs = parsed;
        break;
      }
      default:
        throw new Error(`Unknown CLI flag: --${key}`);
    }
  }
  if (!opts.url) {
    throw new Error('Missing required --url argument');
  }
  new URL(opts.url);
  return opts;
}

class CdpClient {
  #ws;
  #nextId = 1;
  #pending = new Map();
  #listeners = new Map();
  #closed = false;

  constructor(ws) {
    this.#ws = ws;
    ws.addEventListener('message', (event) => {
      const msg = JSON.parse(String(event.data));
      if (msg.id !== undefined) {
        const entry = this.#pending.get(msg.id);
        if (!entry) return;
        this.#pending.delete(msg.id);
        clearTimeout(entry.timer);
        if (msg.error) {
          entry.reject(new Error(`CDP ${entry.method}: ${msg.error.message} (${msg.error.code})`));
        } else {
          entry.resolve(msg.result || {});
        }
      } else if (msg.method) {
        const set = this.#listeners.get(msg.method);
        if (set) {
          for (const fn of set) fn(msg.params || {});
        }
      }
    });
    ws.addEventListener('close', () => {
      this.#closed = true;
      for (const [, entry] of this.#pending) {
        clearTimeout(entry.timer);
        entry.reject(new Error(`CDP socket closed during ${entry.method}`));
      }
      this.#pending.clear();
    });
  }

  static async connect(wsUrl, timeoutMs = 15_000) {
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`WebSocket connect timeout: ${wsUrl}`)), timeoutMs);
      ws.addEventListener('open', () => {
        clearTimeout(timer);
        resolve();
      }, { once: true });
      ws.addEventListener('error', (err) => {
        clearTimeout(timer);
        reject(new Error(`WebSocket error (${wsUrl}): ${err.message || 'connection failed'}`));
      }, { once: true });
    });
    return new CdpClient(ws);
  }

  on(method, handler) {
    let set = this.#listeners.get(method);
    if (!set) {
      set = new Set();
      this.#listeners.set(method, set);
    }
    set.add(handler);
    return () => set.delete(handler);
  }

  send(method, params = {}, timeoutMs = 20_000) {
    if (this.#closed) {
      return Promise.reject(new Error(`CDP socket already closed before ${method}`));
    }
    const id = this.#nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error(`CDP timeout (${timeoutMs}ms) waiting for ${method}`));
      }, timeoutMs);
      this.#pending.set(id, { method, resolve, reject, timer });
      this.#ws.send(JSON.stringify({ id, method, params }));
    });
  }

  close() {
    this.#closed = true;
    try {
      this.#ws.close();
    } catch {
      // ignore close errors during teardown
    }
  }
}

async function launchChrome(chromeBin, timeoutMs) {
  if (!fs.existsSync(chromeBin)) {
    throw new Error(`Chrome binary not found at ${chromeBin}`);
  }
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-e2e-chrome-'));
  const chromeArgs = [
    '--headless=new',
    '--no-sandbox',
    '--disable-gpu',
    '--disable-dev-shm-usage',
    '--remote-debugging-port=0',
    `--user-data-dir=${userDataDir}`,
    '--window-size=1440,900',
    'about:blank',
  ];
  const proc = spawn(chromeBin, chromeArgs, { stdio: ['ignore', 'pipe', 'pipe'] });
  let stderrBuf = '';
  let exited = false;
  proc.once('exit', () => {
    exited = true;
  });

  const browserWsUrl = await new Promise((resolve, reject) => {
    const deadline = setTimeout(() => {
      reject(new Error(`Timed out waiting for Chrome DevTools WS URL. stderr:\n${stderrBuf}`));
    }, Math.min(timeoutMs, 20_000));

    const tryMatch = (chunk) => {
      stderrBuf += chunk.toString('utf8');
      const m = stderrBuf.match(/DevTools listening on (ws:\/\/127\.0\.0\.1:(\d+)\/[^\s]+)/);
      if (m) {
        clearTimeout(deadline);
        resolve(m[1]);
      }
    };
    proc.stderr.on('data', tryMatch);
    proc.once('exit', (code, signal) => {
      clearTimeout(deadline);
      reject(new Error(`Chrome exited prematurely (code=${code}, signal=${signal}). stderr:\n${stderrBuf}`));
    });
  });

  const port = new URL(browserWsUrl).port;
  let pageTarget = null;
  for (let i = 0; i < 40; i += 1) {
    const res = await fetch(`http://127.0.0.1:${port}/json/list`);
    if (res.ok) {
      const targets = await res.json();
      pageTarget = targets.find((t) => t.type === 'page' && t.webSocketDebuggerUrl);
      if (pageTarget) break;
    }
    await new Promise((r) => setTimeout(r, 50));
  }
  assert(pageTarget?.webSocketDebuggerUrl, 'Could not find Chrome CDP page target');

  const cleanup = async () => {
    if (!exited) {
      proc.kill('SIGTERM');
      await new Promise((resolve) => {
        const timer = setTimeout(() => {
          if (!exited) {
            try {
              proc.kill('SIGKILL');
            } catch {
              // ignore
            }
          }
          resolve();
        }, 2000);
        proc.once('exit', () => {
          clearTimeout(timer);
          resolve();
        });
      });
    }
    for (let attempt = 0; attempt < 20; attempt += 1) {
      try {
        fs.rmSync(userDataDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
        break;
      } catch (err) {
        if (attempt === 19) {
          console.error(`Warning: failed to remove Chrome temp profile ${userDataDir}: ${err.message}`);
        } else {
          await new Promise((r) => setTimeout(r, 100));
        }
      }
    }
  };

  return { browserWsUrl, pageWsUrl: pageTarget.webSocketDebuggerUrl, cleanup };
}

function createHarnessDriver(cdp, baseUrl, defaultTimeoutMs) {
  const origin = new URL(baseUrl).origin;
  const runtimeErrors = [];
  const consoleErrors = [];
  const httpErrors = [];
  const loadingErrors = [];
  const requestUrlById = new Map();
  const responsesByUrl = [];
  const finishedBodies = new Map();
  const sseMessages = [];
  const sseConnections = [];
  let expectedNetworkScope = null;

  cdp.on('Runtime.exceptionThrown', (params) => {
    const d = params.exceptionDetails || {};
    const msg = d.exception?.description || d.text || 'Unknown Runtime exception';
    runtimeErrors.push(`${msg} (line ${d.lineNumber ?? '?'}, col ${d.columnNumber ?? '?'})`);
  });

  cdp.on('Runtime.consoleAPICalled', (params) => {
    if (params.type !== 'error') return;
    const text = (params.args || [])
      .map((a) => a.value ?? a.description ?? JSON.stringify(a))
      .join(' ');
    if (expectedNetworkScope) {
      expectedNetworkScope.consoleErrors.push(text);
    } else {
      consoleErrors.push(text);
    }
  });

  cdp.on('Network.requestWillBeSent', (params) => {
    requestUrlById.set(params.requestId, {
      url: params.request?.url || '',
      method: params.request?.method || 'GET',
    });
  });

  cdp.on('Network.responseReceived', (params) => {
    const r = params.response || {};
    const url = r.url || requestUrlById.get(params.requestId)?.url || '';
    const record = {
      requestId: params.requestId,
      url,
      status: r.status,
      mimeType: r.mimeType || '',
      headers: Object.fromEntries(
        Object.entries(r.headers || {}).map(([k, v]) => [k.toLowerCase(), String(v)]),
      ),
    };
    responsesByUrl.push(record);
    if (url.startsWith(`${origin}/api/events/stream`)) {
      sseConnections.push(record);
    }
    if (r.status >= 400) {
      const msg = `HTTP ${r.status} ${url}`;
      if (expectedNetworkScope) {
        expectedNetworkScope.httpErrors.push(msg);
      } else {
        httpErrors.push(msg);
      }
    }
  });

  cdp.on('Network.loadingFinished', (params) => {
    const req = requestUrlById.get(params.requestId);
    if (req) {
      finishedBodies.set(params.requestId, req.url);
    }
  });

  cdp.on('Network.loadingFailed', (params) => {
    if (params.canceled) return;
    const req = requestUrlById.get(params.requestId);
    const msg = `${params.errorText || 'load failed'} (${req?.url || params.requestId})`;
    if (expectedNetworkScope) {
      expectedNetworkScope.loadingErrors.push(msg);
    } else {
      loadingErrors.push(msg);
    }
  });

  async function allowHttpErrors(fn) {
    assert(!expectedNetworkScope, 'allowHttpErrors scopes must not be nested');
    const scope = { httpErrors: [], consoleErrors: [], loadingErrors: [] };
    expectedNetworkScope = scope;
    try {
      const result = await fn(scope);
      return { ...scope, result };
    } finally {
      expectedNetworkScope = null;
    }
  }

  cdp.on('Network.eventSourceMessageReceived', (params) => {
    sseMessages.push({
      eventName: params.eventName || 'message',
      eventId: params.eventId || '',
      data: params.data || '',
      timestamp: params.timestamp,
    });
  });

  function assertNoErrors(phase) {
    const problems = [
      ...runtimeErrors.map((e) => `[RuntimeException] ${e}`),
      ...consoleErrors.map((e) => `[console.error] ${e}`),
      ...httpErrors.map((e) => `[HTTP >=400] ${e}`),
      ...loadingErrors.map((e) => `[NetworkFailed] ${e}`),
    ];
    if (problems.length > 0) {
      throw new Error(`Browser error(s) during ${phase}:\n  - ${problems.join('\n  - ')}`);
    }
  }

  async function evalJs(expression) {
    const res = await cdp.send('Runtime.evaluate', {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (res.exceptionDetails) {
      const desc = res.exceptionDetails.exception?.description || res.exceptionDetails.text;
      throw new Error(`evalJs failed: ${desc}\nExpression: ${expression}`);
    }
    return res.result ? res.result.value : undefined;
  }

  async function callFn(fn, ...args) {
    const serialized = args.map((a) => JSON.stringify(a)).join(', ');
    return evalJs(`(${fn.toString()})(${serialized})`);
  }

  async function waitFor(predicateFn, description, timeoutMs = Math.min(defaultTimeoutMs, 25_000), ...args) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      assertNoErrors(description);
      const val = await callFn(predicateFn, ...args);
      if (val) return val;
      await new Promise((r) => setTimeout(r, 80));
    }
    assertNoErrors(description);
    throw new Error(`Timed out after ${timeoutMs}ms waiting for: ${description}`);
  }

  async function waitUntil(asyncCheck, description, timeoutMs = Math.min(defaultTimeoutMs, 25_000)) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      assertNoErrors(description);
      const val = await asyncCheck();
      if (val) return val;
      await new Promise((r) => setTimeout(r, 80));
    }
    assertNoErrors(description);
    throw new Error(`Timed out after ${timeoutMs}ms waiting for: ${description}`);
  }

  async function elementBox(selector) {
    const box = await callFn(async (sel) => {
      const sameRect = (a, b) => a && b
        && Math.abs(a.left - b.left) < 0.5
        && Math.abs(a.top - b.top) < 0.5
        && Math.abs(a.width - b.width) < 0.5
        && Math.abs(a.height - b.height) < 0.5;
      const deadline = performance.now() + 2500;
      let el = document.querySelector(sel);
      let prev = null;
      while (performance.now() < deadline) {
        if (!el || !el.isConnected) {
          el = document.querySelector(sel);
          prev = null;
        }
        if (!el || !el.isConnected) {
          await new Promise((r) => requestAnimationFrame(r));
          continue;
        }
        el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
        await new Promise((r) => requestAnimationFrame(r));
        if (!el.isConnected) {
          el = document.querySelector(sel);
          prev = null;
          continue;
        }
        const cur = el.getBoundingClientRect();
        if (cur.width > 0 && cur.height > 0 && sameRect(prev, cur)) {
          return {
            x: cur.left,
            y: cur.top,
            w: cur.width,
            h: cur.height,
            cx: cur.left + cur.width / 2,
            cy: cur.top + cur.height / 2,
          };
        }
        prev = cur;
      }
      if (!el || !el.isConnected) {
        el = document.querySelector(sel);
      }
      if (!el) return { x: 0, y: 0, w: 0, h: 0, cx: 0, cy: 0 };
      const r = el.getBoundingClientRect();
      return { x: r.left, y: r.top, w: r.width, h: r.height, cx: r.left + r.width / 2, cy: r.top + r.height / 2 };
    }, selector);
    assert(box && box.w > 0 && box.h > 0, `Element missing or zero-sized: ${selector}`);
    return box;
  }

  async function clickAt(x, y, { modifiers = 0, button = 'left' } = {}) {
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, modifiers });
    await cdp.send('Input.dispatchMouseEvent', {
      type: 'mousePressed',
      x,
      y,
      button,
      clickCount: 1,
      modifiers,
    });
    await cdp.send('Input.dispatchMouseEvent', {
      type: 'mouseReleased',
      x,
      y,
      button,
      clickCount: 1,
      modifiers,
    });
  }

  async function rightClickAt(x, y, options = {}) {
    await clickAt(x, y, { ...options, button: 'right' });
  }

  async function dragAt(x1, y1, x2, y2, { button = 'left', modifiers = 0, steps = 6 } = {}) {
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: x1, y: y1, modifiers });
    await cdp.send('Input.dispatchMouseEvent', {
      type: 'mousePressed',
      x: x1,
      y: y1,
      button,
      clickCount: 1,
      modifiers,
    });
    for (let s = 1; s <= steps; s += 1) {
      const t = s / steps;
      await cdp.send('Input.dispatchMouseEvent', {
        type: 'mouseMoved',
        x: x1 + (x2 - x1) * t,
        y: y1 + (y2 - y1) * t,
        button,
        buttons: button === 'left' ? 1 : 2,
        modifiers,
      });
    }
    await cdp.send('Input.dispatchMouseEvent', {
      type: 'mouseReleased',
      x: x2,
      y: y2,
      button,
      clickCount: 1,
      modifiers,
    });
  }

  async function click(selector, options = {}) {
    const box = await elementBox(selector);
    await clickAt(box.cx, box.cy, options);
  }

  async function pressKey({ key, code, keyCode, modifiers = 0 }) {
    await cdp.send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key,
      code,
      windowsVirtualKeyCode: keyCode,
      nativeVirtualKeyCode: keyCode,
      modifiers,
    });
    await cdp.send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key,
      code,
      windowsVirtualKeyCode: keyCode,
      nativeVirtualKeyCode: keyCode,
      modifiers,
    });
  }

  async function readLatestFinishedJson(urlSubstring, matchFn = null) {
    for (let i = responsesByUrl.length - 1; i >= 0; i -= 1) {
      const rec = responsesByUrl[i];
      if (!rec.url.includes(urlSubstring) || rec.status !== 200) continue;
      if (!finishedBodies.has(rec.requestId)) continue;
      try {
        const body = await cdp.send('Network.getResponseBody', { requestId: rec.requestId });
        const raw = body.base64Encoded ? Buffer.from(body.body, 'base64').toString('utf8') : body.body;
        const parsed = JSON.parse(raw);
        if (matchFn && !matchFn(parsed, rec)) continue;
        return parsed;
      } catch {
        // try older response if current body unavailable
      }
    }
    return null;
  }

  return {
    origin,
    responsesByUrl,
    sseMessages,
    sseConnections,
    assertNoErrors,
    allowHttpErrors,
    evalJs,
    callFn,
    waitFor,
    waitUntil,
    elementBox,
    clickAt,
    rightClickAt,
    dragAt,
    click,
    pressKey,
    readLatestFinishedJson,
  };
}

async function suite1BootThemeTransport(driver, cdp, opts) {
  console.log('[Suite 1] WASM boot, pre-WASM theme & transport');
  const initialTheme = await driver.waitFor(() => {
    const connecting = document.getElementById('connecting');
    const banner = document.getElementById('connection-banner');
    const switcher = document.getElementById('project-switcher');
    const ready = document.readyState === 'complete'
      && connecting?.hidden === true
      && banner?.hidden === true
      && switcher && switcher.options.length > 0 && switcher.value !== '';
    if (!ready) return null;
    return document.documentElement.getAttribute('data-theme');
  }, 'WASM boot completion (#connecting hidden, #connection-banner hidden, data-theme set)');

  assert(initialTheme === 'light' || initialTheme === 'dark', `Invalid initial data-theme: ${initialTheme}`);
  const bannerHidden = await driver.callFn(() => {
    const b = document.getElementById('connection-banner');
    return Boolean(b && b.hidden && getComputedStyle(b).display === 'none');
  });
  assert(bannerHidden, '#connection-banner must remain hidden after boot');

  if (opts.projectId) {
    const switched = await driver.callFn((pid) => {
      const sw = document.getElementById('project-switcher');
      if (!sw) return false;
      const hasOption = Array.from(sw.options).some((o) => o.value === pid);
      if (!hasOption) return 'missing';
      if (sw.value !== pid) {
        sw.value = pid;
        sw.dispatchEvent(new Event('change', { bubbles: true }));
      }
      return sw.value === pid;
    }, opts.projectId);
    assert(switched === true, `Project ${opts.projectId} not selectable in #project-switcher (got ${switched})`);
  }

  const wasmResp = driver.responsesByUrl.find((r) => new URL(r.url).pathname === '/assets/infra_ui_bg.wasm');
  assert(wasmResp, 'Expected /assets/infra_ui_bg.wasm network request');
  assert(wasmResp.status === 200, `Expected WASM 200, got ${wasmResp.status}`);
  assert(wasmResp.mimeType.includes('wasm'), `Expected application/wasm MIME, got ${wasmResp.mimeType}`);
  const wasmEnc = wasmResp.headers['content-encoding'];
  if (wasmEnc) {
    assert(wasmEnc === 'br' || wasmEnc === 'gzip', `Unexpected WASM Content-Encoding: ${wasmEnc}`);
  }

  // Flip theme and verify data-theme + computed CSS color change
  const initialBg = await driver.callFn(() => getComputedStyle(document.documentElement).backgroundColor);
  const nextTheme = initialTheme === 'dark' ? 'light' : 'dark';
  await driver.click(`#theme-${nextTheme}`);
  const flipped = await driver.waitFor((expected) => {
    return document.documentElement.getAttribute('data-theme') === expected;
  }, `data-theme flip to ${nextTheme}`, 5_000, nextTheme);
  assert(flipped === true, 'Theme toggle did not flip data-theme');
  const nextBg = await driver.callFn(() => getComputedStyle(document.documentElement).backgroundColor);
  assert(nextBg !== initialBg, `Expected background color to change across theme toggle (${initialBg} vs ${nextBg})`);

  // Restore original theme cleanly
  await driver.click(`#theme-${initialTheme}`);
  await driver.waitFor((expected) => {
    return document.documentElement.getAttribute('data-theme') === expected;
  }, `data-theme restore to ${initialTheme}`, 5_000, initialTheme);

  // Verify direct navigation to /?view=sessions&project=... boots cleanly with HTTP 200 and live SSE
  if (opts.projectId) {
    const deepUrl = `${driver.origin}/?view=sessions&project=${encodeURIComponent(opts.projectId)}`;
    const respBefore = driver.responsesByUrl.length;
    await cdp.send('Page.navigate', { url: deepUrl });
    await driver.waitFor((pid) => {
      const connecting = document.getElementById('connecting');
      const banner = document.getElementById('connection-banner');
      const sessionsView = document.getElementById('sessions-view');
      const sw = document.getElementById('project-switcher');
      return Boolean(
        document.readyState === 'complete'
          && connecting?.hidden === true
          && banner?.hidden === true
          && sessionsView && !sessionsView.hidden
          && sw && sw.value === pid,
      );
    }, 'Cold boot via /?view=sessions&project=... root query URL', 20_000, opts.projectId);
    const rootResp = driver.responsesByUrl
      .slice(respBefore)
      .find((r) => {
        try {
          const u = new URL(r.url);
          return u.pathname === '/'
            && u.searchParams.get('view') === 'sessions'
            && u.searchParams.get('project') === opts.projectId;
        } catch {
          return false;
        }
      });
    assert(rootResp && rootResp.status === 200, `Expected HTTP 200 for ${deepUrl}, got ${JSON.stringify(rootResp)}`);
    await driver.waitFor(
      () => document.getElementById('sse-status')?.getAttribute('data-state') === 'live',
      'SSE live after /?view=sessions&project=... cold boot',
      12_000,
    );
  }
  driver.assertNoErrors('Suite 1');
}

async function assertDenseCanvasGeometry(driver, canvasSelector, label, { minDpr = 1 } = {}) {
  const geom = await driver.callFn((sel) => {
    const host = document.querySelector(sel);
    if (!host) return { error: `Missing canvas host ${sel}` };
    const canvas = host.querySelector('canvas.graph-dense-canvas[data-dense-canvas="true"]');
    const plane = host.querySelector('.graph-map-plane');
    if (!canvas) return { error: `Missing dense canvas inside ${sel}` };
    if (!plane) return { error: `Missing .graph-map-plane inside ${sel}` };
    const hostRect = host.getBoundingClientRect();
    const canvasRect = canvas.getBoundingClientRect();
    const planeRect = plane.getBoundingClientRect();
    const cs = getComputedStyle(canvas);
    const ctx = canvas.getContext('2d');
    let nonZeroPixels = 0;
    if (ctx && canvas.width > 0 && canvas.height > 0) {
      const img = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
      for (let i = 3; i < img.length; i += 4) {
        if (img[i] > 0) {
          nonZeroPixels += 1;
          if (nonZeroPixels >= 8) break;
        }
      }
    }
    const blank = document.createElement('canvas');
    blank.width = canvas.width;
    blank.height = canvas.height;
    const dataUrlDiffers = canvas.toDataURL() !== blank.toDataURL();
    const overlapY = Math.min(planeRect.bottom, hostRect.bottom) - Math.max(planeRect.top, hostRect.top);
    const overlapX = Math.min(planeRect.right, hostRect.right) - Math.max(planeRect.left, hostRect.left);
    const nodeCount = plane.querySelectorAll('.graph-map-node').length;
    return {
      dpr: window.devicePixelRatio || 1,
      width: canvas.width,
      height: canvas.height,
      position: cs.position,
      pointerEvents: cs.pointerEvents,
      hostRect: { top: hostRect.top, bottom: hostRect.bottom, left: hostRect.left, right: hostRect.right, height: hostRect.height },
      canvasRect: { top: canvasRect.top, bottom: canvasRect.bottom, left: canvasRect.left, right: canvasRect.right, width: canvasRect.width, height: canvasRect.height },
      planeRect: { top: planeRect.top, bottom: planeRect.bottom, left: planeRect.left, right: planeRect.right },
      overlapY,
      overlapX,
      nonZeroPixels,
      dataUrlDiffers,
      nodeCount,
    };
  }, canvasSelector);

  assert(!geom.error, `${label}: ${geom.error}`);
  assert(geom.width > 0 && geom.height > 0, `${label}: dense canvas zero sized (${geom.width}x${geom.height})`);
  if (minDpr > 1) {
    assert(
      geom.dpr >= minDpr
        && geom.width >= Math.floor(geom.canvasRect.width * minDpr * 0.9)
        && geom.height >= Math.floor(geom.canvasRect.height * minDpr * 0.9),
      `${label}: expected HiDPI >=${minDpr}x backing store, got ${JSON.stringify(geom)}`,
    );
  }
  assert(geom.position === 'absolute', `${label}: expected .graph-dense-canvas position === "absolute", got "${geom.position}"`);
  assert(geom.nonZeroPixels > 0 && geom.dataUrlDiffers, `${label}: dense canvas is blank`);
  assert(
    geom.overlapY > 0 && geom.overlapX > 0 && geom.planeRect.top < geom.hostRect.bottom - 4,
    `${label}: .graph-map-plane does not overlap stage (planeTop=${geom.planeRect.top}, hostBottom=${geom.hostRect.bottom}, overlapY=${geom.overlapY})`,
  );
  assert(geom.nodeCount > 0, `${label}: expected visible .graph-map-node elements in plane`);
}

async function withFetchOverride(cdp, patterns, handler, fn) {
  const unsub = cdp.on('Fetch.requestPaused', async (params) => {
    try {
      const handled = await handler(params);
      if (!handled) {
        await cdp.send('Fetch.continueRequest', { requestId: params.requestId });
      }
    } catch {
      await cdp.send('Fetch.continueRequest', { requestId: params.requestId }).catch(() => {});
    }
  });
  await cdp.send('Fetch.enable', { patterns });
  try {
    return await fn();
  } finally {
    unsub();
    await cdp.send('Fetch.disable').catch(() => {});
  }
}

async function closeDialogByBackdropClick(driver, dialogSel, label) {
  const outsidePt = await driver.waitFor((sel) => {
    const dlg = document.querySelector(sel);
    if (!dlg || !dlg.hasAttribute('open')) return null;
    const r = dlg.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return null;
    const candidates = [
      [Math.max(4, Math.floor(r.left / 2)), Math.max(4, Math.floor(r.top / 2))],
      [4, 4],
      [Math.min(window.innerWidth - 4, Math.ceil(r.right + 12)), 12],
      [12, Math.min(window.innerHeight - 4, Math.ceil(r.bottom + 12))],
    ];
    for (const [x, y] of candidates) {
      if (x < r.left - 4 || x > r.right + 4 || y < r.top - 4 || y > r.bottom + 4) {
        return { x, y, left: r.left, top: r.top, right: r.right, bottom: r.bottom };
      }
    }
    return null;
  }, `${label}: find backdrop point outside ${dialogSel}`, 6_000, dialogSel);
  await driver.clickAt(outsidePt.x, outsidePt.y);
  await driver.waitFor((sel) => {
    const dlg = document.querySelector(sel);
    return Boolean(dlg && !dlg.hasAttribute('open'));
  }, `${label}: backdrop click closes ${dialogSel}`, 6_000, dialogSel);
}

async function suite2Canvas2dAndMapLayout(driver, cdp, opts) {
  console.log('[Suite 2] Canvas2D & Dependency Map layout');
  // Ensure sessions view and Code graph right tab are active
  await driver.click('button.nav-item[data-nav="sessions"]');
  await driver.click('#right-tab-graph');

  await driver.waitFor(() => {
    const ws = document.getElementById('session-graph-workspace');
    return ws && ws.querySelector('#session-graph-canvas .graph-map-plane');
  }, 'embedded Code graph plane render');

  // Unfold all so >= 180 nodes render dense Canvas2D
  const alreadyUnfolded = await driver.callFn(() => {
    return document.getElementById('session-graph-unfold')?.getAttribute('aria-pressed') === 'true';
  });
  if (!alreadyUnfolded) {
    await driver.click('#session-graph-unfold');
  }

  await driver.waitFor(() => {
    const c = document.getElementById('session-graph-canvas');
    return c
      && c.getAttribute('aria-busy') !== 'true'
      && c.querySelector('canvas.graph-dense-canvas[data-dense-canvas="true"]');
  }, 'embedded dense Canvas2D (#session-graph-canvas)');
  await assertDenseCanvasGeometry(driver, '#session-graph-canvas', 'Embedded Code Graph');

  // Exercise embedded #session-graph-workspace controls (zoom, pan, node select, Isolate, Changes)
  const embeddedZoomBefore = await driver.callFn(() => document.getElementById('session-graph-zoom-level')?.textContent || '');
  await driver.click('#session-graph-zoom-in');
  await driver.waitFor((prev) => {
    const txt = document.getElementById('session-graph-zoom-level')?.textContent || '';
    return txt && txt !== prev;
  }, 'embedded #session-graph-zoom-in updates zoom level', 5_000, embeddedZoomBefore);
  await driver.click('#session-graph-zoom-level');

  const embeddedCanvasBox = await driver.elementBox('#session-graph-canvas');
  const embeddedPlaneBeforePan = await driver.callFn(() => document.querySelector('#session-graph-canvas .graph-map-plane')?.style.transform || '');
  await driver.dragAt(embeddedCanvasBox.x + 24, embeddedCanvasBox.y + 24, embeddedCanvasBox.x + 84, embeddedCanvasBox.y + 56);
  await driver.waitFor((prev) => {
    const t = document.querySelector('#session-graph-canvas .graph-map-plane')?.style.transform || '';
    return t && t !== prev;
  }, 'embedded #session-graph-canvas pan transform update', 5_000, embeddedPlaneBeforePan);

  const embeddedNode = await driver.callFn(() => {
    const host = document.getElementById('session-graph-canvas');
    const cr = host.getBoundingClientRect();
    for (const n of host.querySelectorAll('.graph-map-node')) {
      const nr = n.getBoundingClientRect();
      const cx = nr.left + nr.width / 2;
      const cy = nr.top + nr.height / 2;
      if (cx > cr.left + 8 && cx < cr.right - 8 && cy > cr.top + 8 && cy < cr.bottom - 8) {
        return { cx, cy };
      }
    }
    return null;
  });
  assert(embeddedNode, 'Expected clickable visible node inside #session-graph-canvas');
  await driver.clickAt(embeddedNode.cx, embeddedNode.cy);
  await driver.waitFor(() => {
    const side = document.getElementById('session-graph-side');
    return Boolean(side && side.textContent.trim().length > 0);
  }, 'embedded #session-graph-side updates on node select');

  await driver.click('#session-graph-isolate-toggle');
  await driver.waitFor(
    () => document.getElementById('session-graph-isolate-toggle')?.getAttribute('aria-pressed') === 'true'
      && document.getElementById('session-graph-canvas')?.getAttribute('aria-busy') !== 'true',
    'embedded Isolate active',
  );
  await driver.click('#session-graph-isolate-toggle');
  await driver.waitFor(
    () => document.getElementById('session-graph-isolate-toggle')?.getAttribute('aria-pressed') === 'false',
    'embedded Isolate cleared',
  );

  await driver.click('#session-graph-mode-changes');
  await driver.waitFor(
    () => document.getElementById('session-graph-mode-changes')?.getAttribute('aria-pressed') === 'true'
      && document.getElementById('session-graph-canvas')?.getAttribute('aria-busy') !== 'true',
    'embedded Changes mode active',
  );
  await driver.click('#session-graph-mode-whole');
  await driver.waitFor(
    () => document.getElementById('session-graph-mode-whole')?.getAttribute('aria-pressed') === 'true'
      && document.getElementById('session-graph-canvas')?.getAttribute('aria-busy') !== 'true',
    'embedded Whole graph mode restored',
  );

  const selectAppearance = await driver.callFn(() => {
    const p = document.getElementById('project-switcher');
    const iso = document.getElementById('session-graph-isolate-depth');
    return {
      project: p ? getComputedStyle(p).appearance : '',
      isolate: iso ? getComputedStyle(iso).appearance : '',
    };
  });
  assert(
    selectAppearance.project === 'none' && selectAppearance.isolate === 'none',
    `Expected appearance:none on <select> controls: ${JSON.stringify(selectAppearance)}`,
  );

  // Expanded #session-graph-workspace acts as accessible modal (role="dialog", aria-modal="true", inert siblings) and restores on Escape
  await driver.click('#session-graph-expand');
  const expandedState = await driver.waitFor(() => {
    const ws = document.getElementById('session-graph-workspace');
    if (!ws || ws.getAttribute('data-expanded') !== 'true') return null;
    return {
      expanded: ws.getAttribute('data-expanded'),
      role: ws.getAttribute('role'),
      ariaModal: ws.getAttribute('aria-modal'),
      wsInert: ws.hasAttribute('inert'),
      sidebarInert: document.getElementById('sidebar')?.hasAttribute('inert'),
      chatColInert: document.getElementById('sessions-chat-column')?.hasAttribute('inert'),
      expandPressed: document.getElementById('session-graph-expand')?.getAttribute('aria-expanded'),
    };
  }, '#session-graph-workspace expands to modal dialog state', 6_000);
  assert(
    expandedState.role === 'dialog'
      && expandedState.ariaModal === 'true'
      && !expandedState.wsInert
      && expandedState.sidebarInert
      && expandedState.chatColInert,
    `Expanded #session-graph-workspace modal inertness check failed: ${JSON.stringify(expandedState)}`,
  );
  await driver.pressKey({ key: 'Escape', code: 'Escape', keyCode: 27 });
  await driver.waitFor(() => {
    const ws = document.getElementById('session-graph-workspace');
    return ws
      && ws.getAttribute('data-expanded') === 'false'
      && !document.getElementById('sidebar')?.hasAttribute('inert')
      && !document.getElementById('sessions-chat-column')?.hasAttribute('inert');
  }, '#session-graph-workspace collapses on Escape and clears inert', 6_000);

  // Open full #graph-map modal
  await driver.click('#session-graph-open-full');
  await driver.waitFor(() => {
    const dlg = document.getElementById('graph-map');
    const c = document.getElementById('graph-map-canvas');
    return dlg?.hasAttribute('open')
      && c
      && c.getAttribute('aria-busy') !== 'true'
      && c.querySelector('canvas.graph-dense-canvas[data-dense-canvas="true"]');
  }, '#graph-map dialog open with dense Canvas2D');
  await assertDenseCanvasGeometry(driver, '#graph-map-canvas', 'Full #graph-map Modal');

  // Wheel zoom test via CDP
  const beforeCam = await driver.callFn(() => {
    const plane = document.querySelector('#graph-map-canvas .graph-map-plane');
    return {
      transform: plane?.style.transform || '',
      scaleAttr: plane?.getAttribute('data-scale') || '',
      zoomText: document.getElementById('graph-map-zoom-level')?.textContent || '',
    };
  });
  const canvasBox = await driver.elementBox('#graph-map-canvas');
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mouseWheel',
    x: canvasBox.cx,
    y: canvasBox.cy,
    deltaX: 0,
    deltaY: -180,
  });
  const afterWheel = await driver.waitFor((prev) => {
    const plane = document.querySelector('#graph-map-canvas .graph-map-plane');
    if (!plane) return null;
    const t = plane.style.transform || '';
    const s = plane.getAttribute('data-scale') || '';
    if (t === prev.transform && s === prev.scaleAttr) return null;
    return {
      transform: t,
      scaleAttr: s,
      nodes: plane.querySelectorAll('.graph-map-node').length,
    };
  }, 'wheel zoom update on #graph-map-canvas', 8_000, beforeCam);
  assert(afterWheel.nodes > 0, 'Nodes must stay mounted after wheel zoom');

  // Pan drag test on empty canvas point via CDP
  const dragPt = await driver.callFn(() => {
    const canvas = document.getElementById('graph-map-canvas');
    const r = canvas.getBoundingClientRect();
    const candidates = [
      [r.left + 28, r.top + 28],
      [r.right - 28, r.top + 28],
      [r.left + 28, r.bottom - 28],
      [r.left + r.width * 0.5, r.top + 28],
      [r.left + r.width * 0.5, r.bottom - 28],
    ];
    for (const [x, y] of candidates) {
      const hit = document.elementFromPoint(x, y);
      if (hit && canvas.contains(hit) && !hit.closest('.graph-map-node, button, a, .graph-map-radar')) {
        return { x, y };
      }
    }
    return { x: r.left + 24, y: r.top + 24 };
  });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: dragPt.x, y: dragPt.y });
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mousePressed',
    x: dragPt.x,
    y: dragPt.y,
    button: 'left',
    clickCount: 1,
  });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: dragPt.x + 40, y: dragPt.y + 30 });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: dragPt.x + 90, y: dragPt.y + 60 });
  await cdp.send('Input.dispatchMouseEvent', {
    type: 'mouseReleased',
    x: dragPt.x + 90,
    y: dragPt.y + 60,
    button: 'left',
    clickCount: 1,
  });

  const afterPan = await driver.waitFor((prevTransform) => {
    const plane = document.querySelector('#graph-map-canvas .graph-map-plane');
    if (!plane) return null;
    const t = plane.style.transform || '';
    if (t === prevTransform) return null;
    return { transform: t, nodes: plane.querySelectorAll('.graph-map-node').length };
  }, 'pan drag transform update on #graph-map-canvas', 8_000, afterWheel.transform);
  assert(afterPan.nodes > 0, 'Nodes must stay mounted after pan');

  // Click a visible node inside #graph-map-canvas
  const nodeTarget = await driver.callFn(() => {
    const canvas = document.getElementById('graph-map-canvas');
    const cr = canvas.getBoundingClientRect();
    const nodes = Array.from(canvas.querySelectorAll('.graph-map-node'));
    for (const n of nodes) {
      const nr = n.getBoundingClientRect();
      const cx = nr.left + nr.width / 2;
      const cy = nr.top + nr.height / 2;
      if (cx > cr.left + 8 && cx < cr.right - 8 && cy > cr.top + 8 && cy < cr.bottom - 8) {
        const hit = document.elementFromPoint(cx, cy);
        if (hit && (hit === n || n.contains(hit))) {
          return { cx, cy, payload: n.getAttribute('data-payload') || '' };
        }
      }
    }
    return null;
  });
  assert(nodeTarget, 'Expected at least one clickable visible .graph-map-node inside #graph-map-canvas');
  await driver.clickAt(nodeTarget.cx, nodeTarget.cy);
  await driver.waitFor(() => {
    const side = document.getElementById('graph-map-side');
    const selected = document.querySelector('#graph-map-canvas .graph-map-node[data-selected="true"]');
    return Boolean(selected && side && side.textContent.trim().length > 0);
  }, 'node selection & inspector update in #graph-map-side');

  const mapLayoutChecks = await driver.callFn(() => {
    const side = document.getElementById('graph-map-side');
    const isoBtn = document.getElementById('graph-map-isolate-toggle');
    const isoSel = document.getElementById('graph-map-isolate-depth');
    const btnR = isoBtn?.getBoundingClientRect();
    const selR = isoSel?.getBoundingClientRect();
    const cw = side?.clientWidth || 0;
    const wide = side
      ? Array.from(side.querySelectorAll('*'))
          .map((el) => ({
            tag: el.tagName,
            cls: el.className,
            sw: el.scrollWidth,
            w: el.getBoundingClientRect().width,
            txt: (el.textContent || '').trim().slice(0, 60),
          }))
          .filter((x) => x.sw > cw + 2 || x.w > cw + 2)
          .slice(0, 8)
      : [];
    return {
      sideScrollW: side?.scrollWidth || 0,
      sideClientW: cw,
      isoRowDelta: btnR && selR ? Math.abs(btnR.top - selR.top) : 999,
      wide,
    };
  });
  assert(
    mapLayoutChecks.sideScrollW <= mapLayoutChecks.sideClientW + 2,
    `Expected #graph-map-side without horizontal overflow: ${JSON.stringify(mapLayoutChecks)}`,
  );
  assert(
    mapLayoutChecks.isoRowDelta < 10,
    `Expected #graph-map Isolate button & hop select on same row: ${JSON.stringify(mapLayoutChecks)}`,
  );

  // Test From -> To path trace using live /graph/map edges (prefer multi-hop path)
  const mapJson = await driver.readLatestFinishedJson('/graph/map');
  let pathFrom = '';
  let pathTo = '';
  if (mapJson?.edges?.length > 0) {
    const adj = new Map();
    for (const e of mapJson.edges) {
      if (!e.from || !e.to || e.from === e.to) continue;
      if (!adj.has(e.from)) adj.set(e.from, []);
      adj.get(e.from).push(e.to);
    }
    outer: for (const [start, nexts] of adj) {
      for (const mid of nexts) {
        const hops2 = adj.get(mid) || [];
        for (const end of hops2) {
          if (end !== start && end !== mid) {
            pathFrom = start;
            pathTo = end;
            break outer;
          }
        }
      }
    }
    if (!pathFrom || !pathTo) {
      pathFrom = mapJson.edges[0].from || '';
      pathTo = mapJson.edges[0].to || '';
    }
  }
  if (!pathFrom || !pathTo) {
    const fallback = await driver.callFn(() => {
      const nodes = Array.from(document.querySelectorAll('#graph-map-canvas .graph-map-node'));
      return nodes.slice(0, 2).map((n) => JSON.parse(n.getAttribute('data-payload') || '""'));
    });
    pathFrom = fallback[0] || '';
    pathTo = fallback[1] || '';
  }
  assert(pathFrom && pathTo, 'Expected two node paths for From -> To trace');
  await driver.callFn((fromVal, toVal) => {
    const a = document.getElementById('graph-map-path-from');
    const b = document.getElementById('graph-map-path-to');
    a.value = fromVal;
    a.dispatchEvent(new Event('input', { bubbles: true }));
    b.value = toVal;
    b.dispatchEvent(new Event('input', { bubbles: true }));
  }, pathFrom, pathTo);
  await driver.click('#graph-map-path-trace');
  await driver.waitFor(() => {
    const clr = document.getElementById('graph-map-path-clear');
    const pathNodes = document.querySelectorAll('#graph-map-canvas .graph-map-node[data-shared-path="true"]').length;
    return clr && !clr.hidden && pathNodes >= 2;
  }, '#graph-map-path-clear visible and traced path nodes highlighted after From -> To');
  await driver.click('#graph-map-path-clear');
  await driver.waitFor(() => document.getElementById('graph-map-path-clear')?.hidden === true, 'path clear hides #graph-map-path-clear');

  // Test Isolate toggle on focused node
  await driver.click('#graph-map-isolate-toggle');
  await driver.waitFor(() => {
    const btn = document.getElementById('graph-map-isolate-toggle');
    const c = document.getElementById('graph-map-canvas');
    return btn?.getAttribute('aria-pressed') === 'true' && c?.getAttribute('aria-busy') !== 'true';
  }, 'Isolate toggle active');
  await driver.click('#graph-map-isolate-toggle');
  await driver.waitFor(() => {
    const btn = document.getElementById('graph-map-isolate-toggle');
    const c = document.getElementById('graph-map-canvas');
    return btn?.getAttribute('aria-pressed') === 'false' && c?.getAttribute('aria-busy') !== 'true';
  }, 'Isolate toggle cleared');

  // HiDPI deviceScaleFactor=2 while #graph-map Canvas2D is open
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 1440,
    height: 900,
    deviceScaleFactor: 2,
    mobile: false,
  });
  await driver.callFn(() => window.dispatchEvent(new Event('resize')));
  await driver.click('#graph-map-zoom-in');
  await driver.waitFor(() => {
    const c = document.querySelector('#graph-map-canvas canvas.graph-dense-canvas[data-dense-canvas="true"]');
    return Boolean(c && (window.devicePixelRatio || 1) >= 2 && c.width >= Math.floor(c.getBoundingClientRect().width * 1.8));
  }, 'HiDPI deviceScaleFactor:2 backing store resize on open #graph-map', 8_000);
  await assertDenseCanvasGeometry(driver, '#graph-map-canvas', 'HiDPI 2x #graph-map Modal', { minDpr: 2 });
  await driver.click('#graph-map-zoom-level');
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 1440,
    height: 900,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await driver.callFn(() => window.dispatchEvent(new Event('resize')));
  await driver.waitFor(() => (window.devicePixelRatio || 1) <= 1.05, 'devicePixelRatio restored to 1', 4_000);

  // Hover neighbor lighting + right-click Explain on Canvas2D + Shift+drag pin/unpin + Back/Forward
  const visibleNodeTargets = () => driver.callFn(() => {
    const host = document.getElementById('graph-map-canvas');
    const cr = host.getBoundingClientRect();
    const out = [];
    for (const n of host.querySelectorAll('.graph-map-node')) {
      const nr = n.getBoundingClientRect();
      const cx = nr.left + nr.width / 2;
      const cy = nr.top + nr.height / 2;
      if (cx > cr.left + 20 && cx < cr.right - 20 && cy > cr.top + 20 && cy < cr.bottom - 20) {
        out.push({
          cx,
          cy,
          payload: n.getAttribute('data-payload') || '',
          focused: n.getAttribute('data-focused') === 'true',
        });
      }
    }
    return out;
  });
  const historyState = () => driver.callFn(() => {
    const canvas = document.getElementById('graph-map-canvas');
    const back = document.getElementById('graph-map-nav-back');
    const forward = document.getElementById('graph-map-nav-forward');
    const crumbs = Array.from(document.querySelectorAll('#graph-map-crumbs .graph-map-history-crumb'));
    const current = crumbs.find((c) => c.getAttribute('aria-current') === 'location');
    return {
      busy: canvas?.getAttribute('aria-busy') === 'true',
      backDisabled: Boolean(back?.disabled),
      forwardDisabled: Boolean(forward?.disabled),
      crumbCount: crumbs.length,
      activeIdx: current ? Number.parseInt(current.getAttribute('data-payload') || '-1', 10) : -1,
    };
  });
  const initialTargets = await visibleNodeTargets();
  assert(initialTargets.length >= 1, 'Expected visible node targets in #graph-map-canvas for hover/pin/explain');
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: initialTargets[0].cx, y: initialTargets[0].cy });
  await driver.callFn(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

  // Camera & selection history Back / Forward (wait for each history push & re-measure after camera shifts)
  if (initialTargets.length >= 2) {
    const firstTarget = initialTargets.find((t) => !t.focused) || initialTargets[0];
    const beforeFirst = await historyState();
    await driver.clickAt(firstTarget.cx, firstTarget.cy);
    const afterFirst = await driver.waitUntil(async () => {
      const s = await historyState();
      if (!s.busy && !s.backDisabled && s.crumbCount >= 2 && (s.activeIdx > beforeFirst.activeIdx || s.crumbCount > beforeFirst.crumbCount)) {
        return s;
      }
      return null;
    }, 'First node click recorded in #graph-map-crumbs history', 8_000);

    const targetsAfterFirst = await visibleNodeTargets();
    const secondTarget = targetsAfterFirst.find((t) => t.payload !== firstTarget.payload && !t.focused)
      || targetsAfterFirst.find((t) => t.payload !== firstTarget.payload);
    assert(secondTarget, 'Expected a second distinct visible node after focusing first node');
    await driver.clickAt(secondTarget.cx, secondTarget.cy);
    const afterSecond = await driver.waitUntil(async () => {
      const s = await historyState();
      if (!s.busy && !s.backDisabled && s.crumbCount >= 2 && s.activeIdx > afterFirst.activeIdx) {
        return s;
      }
      return null;
    }, 'Second node click recorded in #graph-map-crumbs history', 8_000);

    assert(!afterSecond.backDisabled, '#graph-map-nav-back must be enabled after two focus clicks');
    await driver.click('#graph-map-nav-back');
    await driver.waitUntil(async () => {
      const s = await historyState();
      return !s.busy && !s.forwardDisabled && s.activeIdx < afterSecond.activeIdx;
    }, '#graph-map-nav-forward enabled after Back navigation', 8_000);

    await driver.click('#graph-map-nav-forward');
    await driver.waitUntil(async () => {
      const s = await historyState();
      return !s.busy && s.forwardDisabled && s.activeIdx === afterSecond.activeIdx;
    }, '#graph-map-nav-forward disabled at history head', 8_000);
  }

  // Shift+drag pin + single-node unpin / Reset pins (re-measure after history navigation)
  const pinTargets = await visibleNodeTargets();
  assert(pinTargets.length >= 1, 'Expected visible node target in #graph-map-canvas for Shift+drag pin');
  await driver.dragAt(pinTargets[0].cx, pinTargets[0].cy, pinTargets[0].cx + 36, pinTargets[0].cy + 20, { modifiers: 8 });
  const pinControls = await driver.waitFor(() => {
    const reset = document.getElementById('graph-map-reset-pins');
    const undo = document.getElementById('graph-map-pin-undo');
    const unpin = document.querySelector('#graph-map-side .graph-map-unpin-btn');
    const pinned = document.querySelector('#graph-map-canvas .graph-map-node[data-pinned="true"]');
    if ((reset && !reset.hidden) || (undo && !undo.hidden) || unpin || pinned) {
      return { hasReset: Boolean(reset && !reset.hidden), hasUnpin: Boolean(unpin), hasUndo: Boolean(undo && !undo.hidden) };
    }
    return null;
  }, 'Shift+drag pin state reflected in UI controls', 5_000);
  if (pinControls.hasUnpin) {
    await driver.click('#graph-map-side .graph-map-unpin-btn');
  } else if (pinControls.hasUndo) {
    await driver.click('#graph-map-pin-undo');
  } else if (pinControls.hasReset) {
    await driver.click('#graph-map-reset-pins');
  }
  await driver.callFn(() => {
    const reset = document.getElementById('graph-map-reset-pins');
    if (reset && !reset.hidden) reset.click();
  });

  // Canvas2D right-click Explain menu & dialog
  await driver.rightClickAt(pinTargets[0].cx, pinTargets[0].cy);
  const explainText = await driver.waitFor(() => {
    const btn = document.getElementById('graph-explain-action');
    if (!btn || btn.hidden) return null;
    const text = (btn.textContent || '').trim();
    return text.toLowerCase().includes('explain') ? text : null;
  }, 'right-click #graph-explain-action context menu', 5_000);
  assert(explainText.toLowerCase().includes('explain'), `Unexpected explain action text: ${explainText}`);
  await driver.click('#graph-explain-action');
  await driver.waitFor(() => {
    const dlg = document.getElementById('graph-explanation');
    const subj = document.getElementById('graph-explanation-subject');
    return Boolean(dlg && dlg.hasAttribute('open') && subj && subj.textContent.trim().length > 0);
  }, '#graph-explanation dialog opens with subject from Canvas2D context menu', 8_000);
  await driver.click('#graph-explanation-close');
  await driver.waitFor(() => !document.getElementById('graph-explanation')?.hasAttribute('open'), '#graph-explanation closes cleanly');

  // Cmd/Ctrl+K palette opens visibly ABOVE <dialog open> #graph-map in top layer
  await driver.pressKey({ key: 'k', code: 'KeyK', keyCode: 75, modifiers: 2 });
  await driver.waitFor(() => {
    const p = document.getElementById('command-palette');
    const input = document.getElementById('command-palette-input');
    return Boolean(p && !p.hidden && (p.tagName !== 'DIALOG' || p.hasAttribute('open')) && document.activeElement === input);
  }, 'Cmd/Ctrl+K palette opens above <dialog open> #graph-map');
  const paletteAboveGraph = await driver.callFn(() => {
    const p = document.getElementById('command-palette');
    const card = p.querySelector('.command-palette-card') || p;
    const r = card.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return {
      graphOpen: Boolean(document.getElementById('graph-map')?.hasAttribute('open')),
      hitInPalette: Boolean(hit && p.contains(hit)),
      w: r.width,
      h: r.height,
    };
  });
  assert(
    paletteAboveGraph.graphOpen && paletteAboveGraph.hitInPalette && paletteAboveGraph.w > 100 && paletteAboveGraph.h > 40,
    `Expected #command-palette stacked above #graph-map: ${JSON.stringify(paletteAboveGraph)}`,
  );
  await driver.pressKey({ key: 'Escape', code: 'Escape', keyCode: 27 });
  await driver.waitFor(() => {
    const p = document.getElementById('command-palette');
    const g = document.getElementById('graph-map');
    return Boolean(p && p.hidden && !p.hasAttribute('open') && g && g.hasAttribute('open'));
  }, 'Cmd/Ctrl+K Escape closes palette without closing #graph-map');

  // Test A..B commit range UI in commit dock
  await driver.click('#graph-commit-graph-toggle');
  await driver.waitFor(() => {
    const panel = document.getElementById('graph-commit-graph-panel');
    const rows = document.querySelectorAll('#graph-commit-graph-list .git-graph-row[data-action="graph-commit-select"]');
    return panel && !panel.hidden && rows.length >= 2;
  }, 'commit dock open with >= 2 commit rows');

  const commitSelectors = await driver.callFn((commitA, commitB) => {
    const rows = Array.from(document.querySelectorAll('#graph-commit-graph-list .git-graph-row[data-action="graph-commit-select"]'));
    const findRow = (sha) => sha ? rows.find((r) => (r.getAttribute('data-payload') || '').includes(sha)) : null;
    const rowB = findRow(commitB) || rows[0];
    const rowA = findRow(commitA) || rows.find((r) => r !== rowB) || rows[1];
    return rowA && rowB ? { aId: `#${rowA.id}`, bId: `#${rowB.id}` } : null;
  }, opts.commitA, opts.commitB);
  assert(commitSelectors, 'Expected two commit rows for A..B range test');

  const mapCountBeforeCommit = driver.responsesByUrl.length;
  await driver.click(commitSelectors.bId);
  await driver.waitUntil(async () => {
    const settled = await driver.callFn(() => {
      return document.getElementById('graph-map-canvas')?.getAttribute('aria-busy') !== 'true';
    });
    const hasCommitMap = driver.responsesByUrl.slice(mapCountBeforeCommit).some((r) => {
      if (r.status !== 200) return false;
      try {
        return decodeURIComponent(r.url).includes('target=commit:');
      } catch {
        return r.url.includes('target=commit');
      }
    });
    return settled && hasCommitMap;
  }, 'single commit selection settled');

  const mapCountBeforeRange = driver.responsesByUrl.length;
  await driver.click(commitSelectors.aId, { modifiers: 8 /* Shift */ });
  await driver.waitUntil(async () => {
    const uiReady = await driver.callFn(() => {
      const banner = document.getElementById('graph-commit-banner');
      const c = document.getElementById('graph-map-canvas');
      return Boolean(banner && !banner.hidden && banner.querySelector('.graph-commit-range-bar') && c?.getAttribute('aria-busy') !== 'true');
    });
    const rangeApiOk = driver.responsesByUrl.slice(mapCountBeforeRange).some((r) => {
      if (r.status !== 200) return false;
      try {
        return decodeURIComponent(r.url).includes('target=range:');
      } catch {
        return r.url.includes('target=range');
      }
    });
    return uiReady && rangeApiOk;
  }, 'A..B commit range banner rendered and target=range: API completed');

  const mapCountBeforeClear = driver.responsesByUrl.length;
  await driver.click('#graph-commit-banner [data-action="graph-commit-range-clear"]');
  await driver.waitUntil(async () => {
    const uiCleared = await driver.callFn(() => {
      const banner = document.getElementById('graph-commit-banner');
      const c = document.getElementById('graph-map-canvas');
      return Boolean(banner && !banner.querySelector('.graph-commit-range-bar') && c?.getAttribute('aria-busy') !== 'true');
    });
    const clearedMapOk = driver.responsesByUrl.slice(mapCountBeforeClear).some((r) => {
      if (r.status !== 200 || !r.url.includes('/graph/map')) return false;
      try {
        return !decodeURIComponent(r.url).includes('target=range:');
      } catch {
        return !r.url.includes('target=range');
      }
    });
    return uiCleared && clearedMapOk;
  }, 'A..B commit range cleared');
  await driver.click('#graph-commit-graph-close');

  driver.assertNoErrors('Suite 2');
}

async function suite3MapLenses(driver) {
  console.log('[Suite 3] 5 Map Lenses (role="tab" + aria-selected)');
  const lenses = ['changes', 'architecture', 'tests', 'runs', 'dependencies'];
  for (const lens of lenses) {
    const mapCountBefore = driver.responsesByUrl.length;
    await driver.click(`#graph-map-lens-${lens}`);
    const state = await driver.waitUntil(async () => {
      const hasMapResponse = driver.responsesByUrl
        .slice(mapCountBefore)
        .some((r) => r.status === 200 && r.url.includes('/graph/map'));
      if (!hasMapResponse) return null;
      return driver.callFn((activeLens, allLenses) => {
        const c = document.getElementById('graph-map-canvas');
        if (!c || c.getAttribute('aria-busy') === 'true') return null;
        const activeBtn = document.getElementById(`graph-map-lens-${activeLens}`);
        if (activeBtn?.getAttribute('aria-selected') !== 'true') return null;
        if (c.querySelectorAll('.graph-map-node').length === 0) return null;

        const tabs = allLenses.map((name) => {
          const el = document.getElementById(`graph-map-lens-${name}`);
          return {
            name,
            role: el?.getAttribute('role'),
            ariaSelected: el?.getAttribute('aria-selected'),
            ariaPressed: el?.getAttribute('aria-pressed'),
          };
        });
        const pill = document.getElementById('graph-map-freshness');
        const pillRect = pill?.getBoundingClientRect();
        return {
          tabs,
          pillVisible: Boolean(pill && !pill.hidden && getComputedStyle(pill).display !== 'none' && pillRect && pillRect.width > 0 && pillRect.height > 0),
          pillTone: pill?.getAttribute('data-tone') || '',
        };
      }, lens, ['dependencies', 'changes', 'architecture', 'tests', 'runs']);
    }, `lens ${lens} activation`, 20_000);

    for (const tab of state.tabs) {
      assert(tab.role === 'tab', `Lens ${tab.name}: expected role="tab", got ${tab.role}`);
      assert(tab.ariaPressed === null, `Lens ${tab.name}: must not use aria-pressed (got ${tab.ariaPressed})`);
      const expected = tab.name === lens ? 'true' : 'false';
      assert(tab.ariaSelected === expected, `Lens ${tab.name} under active=${lens}: expected aria-selected="${expected}", got "${tab.ariaSelected}"`);
    }
    assert(state.pillVisible, `Lens ${lens}: #graph-map-freshness pill must be visible`);
    assert(
      ['ready', 'ir', 'fresh', 'stale', 'warning', 'building', 'error', 'failed'].includes(state.pillTone),
      `Lens ${lens}: unexpected freshness data-tone="${state.pillTone}"`,
    );

    const renderedLens = await driver.callFn((activeLens) => {
      const plane = document.querySelector('#graph-map-canvas .graph-map-plane');
      const sideText = (document.getElementById('graph-map-side')?.textContent || '').trim();
      const legendText = (document.getElementById('graph-map-legend')?.textContent || '').trim();
      const openedCount = document.querySelectorAll('#graph-map-canvas [data-run-opened="true"], #graph-map-canvas .graph-map-node-opened').length;
      const archActive = plane?.getAttribute('data-architecture') === 'true'
        || document.querySelectorAll('#graph-map-canvas [data-architecture-layer="true"], #graph-map-canvas [data-layer="true"]').length > 0;
      const nodeCount = document.querySelectorAll('#graph-map-canvas .graph-map-node').length;
      return { activeLens, sideText, legendText, openedCount, archActive, nodeCount };
    }, lens);
    assert(renderedLens.nodeCount > 0, `Lens ${lens}: expected rendered nodes in #graph-map-canvas`);
    if (lens === 'architecture') {
      assert(
        renderedLens.archActive && renderedLens.sideText.length > 0,
        `Architecture lens must render layer attributes and side details: ${JSON.stringify(renderedLens)}`,
      );
    } else if (lens === 'tests') {
      assert(
        renderedLens.sideText.length > 0 || renderedLens.legendText.length > 0,
        `Tests lens must render coverage/test-reach summary: ${JSON.stringify(renderedLens)}`,
      );
    } else if (lens === 'runs') {
      assert(
        renderedLens.openedCount > 0 || renderedLens.legendText.includes('◉') || renderedLens.sideText.length > 0,
        `Runs lens must render opened/run details: ${JSON.stringify(renderedLens)}`,
      );
    }
  }

  await driver.click('#graph-map-close');
  await driver.waitFor(() => !document.getElementById('graph-map')?.hasAttribute('open'), '#graph-map dialog close');
  driver.assertNoErrors('Suite 3');
}

async function suite4LiveSseAndStyling(driver, cdp, opts) {
  console.log('[Suite 4] Live SSE stream & .conversation-* / .md-body styling');
  assert(driver.sseConnections.length > 0, 'Expected /api/events/stream EventSource connection');
  const sseConn = driver.sseConnections[driver.sseConnections.length - 1];
  assert(sseConn.status === 200, `Expected SSE 200, got ${sseConn.status}`);
  assert(
    sseConn.mimeType.includes('text/event-stream') || (sseConn.headers['content-type'] || '').includes('text/event-stream'),
    `Expected text/event-stream MIME on /api/events/stream, got ${sseConn.mimeType}`,
  );
  const sseState = await driver.callFn(() => document.getElementById('sse-status')?.getAttribute('data-state') || '');
  if (sseState) {
    assert(sseState === 'live', `Expected #sse-status[data-state="live"], got "${sseState}"`);
  }

  // Verify #run-now #run-verb / #run-target DOM structure
  const runStruct = await driver.callFn(() => {
    const now = document.getElementById('run-now');
    const textWrap = document.getElementById('run-now-text');
    const verb = document.getElementById('run-verb');
    const target = document.getElementById('run-target');
    return {
      hasNow: Boolean(now),
      hasTextWrap: Boolean(textWrap && now?.contains(textWrap)),
      hasVerb: Boolean(verb && textWrap?.contains(verb)),
      targetTag: target?.tagName || '',
      targetInWrap: Boolean(target && textWrap?.contains(target)),
    };
  });
  assert(
    runStruct.hasNow && runStruct.hasTextWrap && runStruct.hasVerb && runStruct.targetInWrap && runStruct.targetTag === 'BDI',
    `Unexpected #run-now verb/target structure: ${JSON.stringify(runStruct)}`,
  );

  // Switch between other_session_id -> session_id if available, then ensure session_id is active
  if (opts.otherSessionId && opts.sessionId) {
    const otherSel = `#history [data-action="session-open"][data-payload="${JSON.stringify(opts.otherSessionId).replace(/"/g, '\\"')}"]`;
    const mainSel = `#history [data-action="session-open"][data-payload="${JSON.stringify(opts.sessionId).replace(/"/g, '\\"')}"]`;
    const hasBoth = await driver.callFn((a, b) => Boolean(document.querySelector(a) && document.querySelector(b)), otherSel, mainSel);
    assert(hasBoth, `Expected both session_id (${opts.sessionId}) and other_session_id (${opts.otherSessionId}) in #history`);
    await driver.click(otherSel);
    await driver.waitFor((sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true', 'switch to other_session_id', 10_000, otherSel);
    await driver.click(mainSel);
    await driver.waitFor((sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true', 'switch back to session_id', 10_000, mainSel);
  } else if (opts.sessionId) {
    const mainSel = `#history [data-action="session-open"][data-payload="${JSON.stringify(opts.sessionId).replace(/"/g, '\\"')}"]`;
    const already = await driver.callFn((sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true', mainSel);
    if (!already) {
      await driver.click(mainSel);
      await driver.waitFor((sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true', 'select session_id', 10_000, mainSel);
    }
  }

  // Trigger a live SSE stream turn via #prompt + #send (POST /api/sessions/{id}/inbox -> 202)
  const sseCountBefore = driver.sseMessages.length;
  const codeBlockCountBefore = await driver.callFn(() => document.querySelectorAll('#events .md-code-block').length);
  await driver.waitFor(() => {
    const prompt = document.getElementById('prompt');
    return prompt && !prompt.disabled;
  }, '#prompt enabled for live SSE turn');

  // Scroll #conversation to top with constrained height before #send so live SSE surfaces #events-jump-latest
  await driver.callFn(() => {
    const prompt = document.getElementById('prompt');
    prompt.value = 'Run browser E2E live SSE verification turn';
    prompt.dispatchEvent(new Event('input', { bubbles: true }));
    const conv = document.getElementById('conversation');
    if (conv) {
      conv.dataset.e2ePrevMaxHeight = conv.style.maxHeight || '';
      conv.style.maxHeight = '160px';
      conv.scrollTop = 0;
      conv.dispatchEvent(new Event('scroll'));
    }
  });
  try {
    await driver.waitFor(() => {
      const conv = document.getElementById('conversation');
      const send = document.getElementById('send');
      return Boolean(conv && conv.scrollHeight > conv.clientHeight + 20 && conv.scrollTop === 0 && send && !send.disabled);
    }, '#conversation scrollable at top and #send enabled before live SSE turn');
    await driver.click('#send');

    await driver.waitUntil(
      () => driver.sseMessages.slice(sseCountBefore).some((m) => m.eventName === 'session_event'),
      'live SSE session_event received on /api/events/stream after #send',
    );
    await driver.waitFor(() => {
      const btn = document.getElementById('events-jump-latest');
      return Boolean(btn && !btn.hidden && (btn.textContent || '').includes('Jump to latest ('));
    }, '#events-jump-latest shows unseen count when scrolled up during live SSE', 8_000);
    const scrollPinned = await driver.callFn(() => {
      const conv = document.getElementById('conversation');
      return conv ? conv.scrollTop : -1;
    });
    assert(scrollPinned === 0, `Expected scrolled-up #conversation scrollTop to stay anchored at 0 during live SSE, got ${scrollPinned}`);
    await driver.click('#events-jump-latest');
    await driver.waitFor(() => {
      const btn = document.getElementById('events-jump-latest');
      const conv = document.getElementById('conversation');
      return Boolean(btn?.hidden && conv && conv.scrollHeight - (conv.scrollTop + conv.clientHeight) <= 32);
    }, '#events-jump-latest scrolls to bottom and hides', 6_000);
  } finally {
    await driver.callFn(() => {
      const conv = document.getElementById('conversation');
      if (conv) {
        conv.style.maxHeight = conv.dataset.e2ePrevMaxHeight || '';
        delete conv.dataset.e2ePrevMaxHeight;
        conv.scrollTop = conv.scrollHeight;
        conv.dispatchEvent(new Event('scroll'));
      }
    });
  }

  await driver.waitFor((prevBlocks) => {
    const ev = document.getElementById('events');
    if (!ev) return false;
    const blocks = ev.querySelectorAll('.conversation-text .md-body .md-code-block').length;
    return Boolean(
      blocks > prevBlocks
      && ev.querySelector('.conversation-tool .tool-output'),
    );
  }, 'newly streamed .conversation-text, .md-body, .md-code-block, and .tool-output in #events', 25_000, codeBlockCountBefore);

  // Expand tool details so .tool-output is visibly laid out
  await driver.callFn(() => {
    const d = document.querySelector('#events details.conversation-tool');
    if (d && !d.open) d.open = true;
  });

  const styles = await driver.callFn(() => {
    const convTexts = document.querySelectorAll('#events .conversation-text');
    const convText = convTexts[convTexts.length - 1];
    const mdBody = convText.querySelector('.md-body');
    const codeBlocks = convText.querySelectorAll('.md-code-block');
    const codeBlock = codeBlocks[codeBlocks.length - 1];
    const toolOut = document.querySelector('#events .tool-output');
    const csConv = getComputedStyle(convText);
    const csMd = getComputedStyle(mdBody);
    const csCode = getComputedStyle(codeBlock);
    const csTool = getComputedStyle(toolOut);
    return {
      convMarginBottom: Number.parseFloat(csConv.marginBottom),
      mdFontSize: Number.parseFloat(csMd.fontSize),
      mdLineHeight: csMd.lineHeight,
      codeBorderStyle: csCode.borderTopStyle,
      codeBorderWidth: Number.parseFloat(csCode.borderTopWidth),
      codeBg: csCode.backgroundColor,
      codeRadius: Number.parseFloat(csCode.borderTopLeftRadius),
      toolPadTop: Number.parseFloat(csTool.paddingTop),
      toolPadLeft: Number.parseFloat(csTool.paddingLeft),
      toolBorderStyle: csTool.borderTopStyle,
      toolBorderWidth: Number.parseFloat(csTool.borderTopWidth),
      toolBg: csTool.backgroundColor,
      toolFont: csTool.fontFamily,
      toolRectHeight: toolOut.getBoundingClientRect().height,
    };
  });

  assert(styles.convMarginBottom > 0, `.conversation-text marginBottom should be styled (>0), got ${styles.convMarginBottom}`);
  assert(styles.mdFontSize > 0 && styles.mdLineHeight !== 'normal', `.md-body typography unstyled: fontSize=${styles.mdFontSize}, lineHeight=${styles.mdLineHeight}`);
  assert(
    styles.codeBorderStyle === 'solid' && styles.codeBorderWidth > 0 && styles.codeBg !== 'rgba(0, 0, 0, 0)' && styles.codeRadius > 0,
    `.md-code-block missing card CSS: ${JSON.stringify(styles)}`,
  );
  assert(
    styles.toolPadTop > 0 && styles.toolPadLeft > 0 && styles.toolBorderStyle === 'solid' && styles.toolBorderWidth > 0 && styles.toolBg !== 'rgba(0, 0, 0, 0)' && styles.toolRectHeight > 0,
    `.tool-output missing styled box CSS: ${JSON.stringify(styles)}`,
  );

  // Test code-block Copy button state transition Copy -> Copied -> Copy on the newest code block
  await driver.callFn(() => {
    const btns = document.querySelectorAll('#events .md-code-block button.md-copy-code[data-action="copy-code"]');
    btns.forEach((b) => b.removeAttribute('data-e2e-copy-target'));
    if (btns.length > 0) btns[btns.length - 1].setAttribute('data-e2e-copy-target', '1');
  });
  const copySel = '#events button.md-copy-code[data-e2e-copy-target="1"]';
  const initialCopyText = await driver.callFn((sel) => document.querySelector(sel)?.textContent?.trim(), copySel);
  assert(initialCopyText === 'Copy', `Expected initial Copy button label "Copy", got "${initialCopyText}"`);
  await driver.click(copySel);
  await driver.waitFor((sel) => document.querySelector(sel)?.textContent?.trim() === 'Copied', 'Copy button changes to "Copied"', 5_000, copySel);
  await driver.waitFor((sel) => document.querySelector(sel)?.textContent?.trim() === 'Copy', 'Copy button resets back to "Copy"', 4_000, copySel);

  await assertInstantDarkConversationUserContrast(driver);

  // Verify #events-load-earlier handles client slice reveal + ?before=<oldest_event_id> pagination
  if (opts?.otherSessionId && opts?.sessionId) {
    const otherSel = `#history [data-action="session-open"][data-payload="${JSON.stringify(opts.otherSessionId).replace(/"/g, '\\"')}"]`;
    const mainSel = `#history [data-action="session-open"][data-payload="${JSON.stringify(opts.sessionId).replace(/"/g, '\\"')}"]`;
    await driver.click(otherSel);
    await driver.waitFor(
      (sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true',
      'switch to other_session_id to snapshot payload for #events-load-earlier',
      10_000,
      otherSel,
    );
    const otherEnc = encodeURIComponent(opts.otherSessionId);
    const baseOther = await driver.waitUntil(
      async () => (
        await driver.readLatestFinishedJson(`/api/sessions/${otherEnc}?`, (j) => Boolean(j?.session))
        || await driver.readLatestFinishedJson(`/api/sessions/${opts.otherSessionId}?`, (j) => Boolean(j?.session))
      ),
      'capture finished session JSON payload for other_session_id',
      10_000,
    );
    assert(baseOther?.session, 'Expected captured JSON payload for other_session_id');
    await driver.click(mainSel);
    await driver.waitFor(
      (sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true',
      'switch back to session_id before Fetch override',
      10_000,
      mainSel,
    );

    const syntheticInitial = {
      ...baseOther,
      events: Array.from({ length: 135 }, (_, idx) => ({
        id: 201 + idx,
        kind: 'text',
        text: `Synthetic session history item ${201 + idx}`,
      })),
      has_older: true,
      oldest_event_id: 201,
    };
    const syntheticOlder = {
      events: Array.from({ length: 8 }, (_, idx) => ({
        id: 193 + idx,
        kind: 'text',
        text: `Loaded earlier server history entry ${193 + idx}`,
      })),
      has_older: false,
      oldest_event_id: 193,
    };
    let sawBeforeFetch = false;

    await withFetchOverride(
      cdp,
      [{ urlPattern: `${driver.origin}/api/sessions/*` }],
      async (params) => {
        const reqUrl = params.request?.url || '';
        const method = (params.request?.method || 'GET').toUpperCase();
        const parsed = new URL(reqUrl);
        const matchesOther = parsed.pathname === `/api/sessions/${opts.otherSessionId}`
          || parsed.pathname === `/api/sessions/${otherEnc}`;
        if (method === 'GET' && matchesOther) {
          if (parsed.searchParams.get('before') === '201') {
            sawBeforeFetch = true;
            await cdp.send('Fetch.fulfillRequest', {
              requestId: params.requestId,
              responseCode: 200,
              responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
              body: Buffer.from(JSON.stringify(syntheticOlder)).toString('base64'),
            });
            return true;
          }
          if (!parsed.searchParams.has('before') && !parsed.searchParams.has('after')) {
            await cdp.send('Fetch.fulfillRequest', {
              requestId: params.requestId,
              responseCode: 200,
              responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
              body: Buffer.from(JSON.stringify(syntheticInitial)).toString('base64'),
            });
            return true;
          }
          if (parsed.searchParams.has('after')) {
            await cdp.send('Fetch.fulfillRequest', {
              requestId: params.requestId,
              responseCode: 200,
              responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
              body: Buffer.from(JSON.stringify({ session: baseOther.session, events: [] })).toString('base64'),
            });
            return true;
          }
        }
        return false;
      },
      async () => {
        await driver.click(otherSel);
        await driver.waitFor(() => {
          const wrap = document.getElementById('events-load-earlier-wrap');
          const btn = document.getElementById('events-load-earlier');
          return Boolean(wrap && !wrap.hidden && btn && (btn.textContent || '').includes('15 loaded'));
        }, '#events-load-earlier shows client buffered count (15 loaded)', 10_000);

        await driver.click('#events-load-earlier');
        await driver.waitFor(() => {
          const evText = document.getElementById('events')?.textContent || '';
          const btnText = document.getElementById('events-load-earlier')?.textContent || '';
          return evText.includes('Synthetic session history item 201') && !btnText.includes('15 loaded');
        }, '#events-load-earlier expands client window first', 8_000);

        await driver.click('#events-load-earlier');
        await driver.waitFor(() => {
          const wrap = document.getElementById('events-load-earlier-wrap');
          const evText = document.getElementById('events')?.textContent || '';
          return Boolean(wrap?.hidden && evText.includes('Loaded earlier server history entry 193'));
        }, '#events-load-earlier fetches ?before=201 and hides wrap when has_older is false', 8_000);
        assert(sawBeforeFetch, 'Expected ?before=201 request on #events-load-earlier click');
      },
    );

    await driver.click(mainSel);
    await driver.waitFor(
      (sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true',
      'restore session_id after #events-load-earlier test',
      10_000,
      mainSel,
    );
  }

  driver.assertNoErrors('Suite 4');
}

async function assertInstantDarkConversationUserContrast(driver) {
  const hasUserMsg = await driver.callFn(() => Boolean(document.querySelector('#events .conversation-user')));
  assert(hasUserMsg, 'Expected at least one .conversation-user block inside #events');

  const { initialTheme, darkAlreadyChecked } = await driver.callFn(() => ({
    initialTheme: document.documentElement.getAttribute('data-theme') || 'light',
    darkAlreadyChecked: Boolean(document.getElementById('theme-dark')?.checked),
  }));
  if (initialTheme !== 'light' || darkAlreadyChecked) {
    await driver.click('#theme-light');
    await driver.waitFor(
      () => document.documentElement.getAttribute('data-theme') === 'light',
      'switch to light theme before dark-mode instant contrast check',
      5_000,
    );
  }

  // Ensure light-mode background on .conversation-user is settled before toggling to dark
  await driver.waitFor(() => {
    const el = document.querySelector('#events .conversation-user');
    if (!el) return false;
    const m = getComputedStyle(el).backgroundColor.match(/^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/i);
    return Boolean(m && Number(m[1]) > 180 && Number(m[2]) > 180 && Number(m[3]) > 180);
  }, '.conversation-user settled in light mode before #theme-dark click', 5_000);

  // Capture computed styles on the very next animation frame after #theme-dark change (no 500ms wait)
  await driver.callFn(() => {
    const input = document.getElementById('theme-dark');
    if (!input) throw new Error('Missing #theme-dark input');
    window.__e2eDarkThemeCapture = new Promise((resolve) => {
      let settled = false;
      const timer = setTimeout(() => {
        if (!settled) {
          settled = true;
          resolve({ fired: false });
        }
      }, 2000);
      input.addEventListener('change', () => {
        const clickTs = performance.now();
        requestAnimationFrame(() => {
          if (settled) return;
          settled = true;
          clearTimeout(timer);
          const el = document.querySelector('#events .conversation-user');
          const cs = el ? getComputedStyle(el) : null;
          resolve({
            fired: true,
            elapsedMs: performance.now() - clickTs,
            theme: document.documentElement.getAttribute('data-theme'),
            checked: Boolean(input.checked),
            backgroundColor: cs?.backgroundColor || '',
            color: cs?.color || '',
          });
        });
      }, { once: true });
    });
  });

  await driver.click('#theme-dark');
  const snap = await driver.evalJs(
    '(async () => { try { return await window.__e2eDarkThemeCapture; } finally { delete window.__e2eDarkThemeCapture; } })()',
  );

  assert(snap?.fired, '#theme-dark click did not trigger change event');
  assert(
    snap.theme === 'dark' && snap.checked,
    `Expected data-theme="dark" immediately after #theme-dark click, got ${JSON.stringify(snap)}`,
  );
  // Single animation frame check (well under --motion-slow 400ms transition window)
  assert(
    snap.elapsedMs <= 64,
    `Expected next-frame style inspection well before 400ms transition, got elapsedMs=${snap.elapsedMs}`,
  );

  const bg = parseRgb(snap.backgroundColor);
  const fg = parseRgb(snap.color);
  assert(bg && bg.a === 1, `Invalid .conversation-user dark backgroundColor: ${snap.backgroundColor}`);
  assert(fg && fg.a === 1, `Invalid .conversation-user dark color: ${snap.color}`);
  assert(
    bg.r < 60 && bg.g < 60 && bg.b < 60,
    `Expected .conversation-user background RGB < 60 immediately after #theme-dark (no 400ms gray flash), got ${snap.backgroundColor} at ${snap.elapsedMs.toFixed(1)}ms`,
  );
  assert(
    fg.r > 180 && fg.g > 180 && fg.b > 180,
    `Expected light .conversation-user text color in dark mode, got ${snap.color}`,
  );
  const ratio = contrastRatio(fg, bg);
  assert(
    ratio >= 4.5,
    `Expected .conversation-user dark contrast ratio >= 4.5:1 immediately after toggle, got ${ratio.toFixed(2)}:1 (fg=${snap.color}, bg=${snap.backgroundColor}, elapsed=${snap.elapsedMs.toFixed(1)}ms)`,
  );

  if (initialTheme !== 'dark') {
    await driver.click(`#theme-${initialTheme}`);
    await driver.waitFor(
      (expected) => document.documentElement.getAttribute('data-theme') === expected,
      `restore data-theme to ${initialTheme}`,
      5_000,
      initialTheme,
    );
  }
}

async function waitForRunPanelDetailedSettled(driver) {
  await driver.waitFor(async () => {
    const card = document.getElementById('run-card');
    const panel = document.getElementById('run-panel');
    const wrap = card?.querySelector('.run-panel-wrap');
    if (!card || card.getAttribute('data-detailed') !== 'true' || !panel || !wrap || panel.hasAttribute('inert')) {
      return false;
    }
    const h1 = wrap.getBoundingClientRect().height;
    const panelH1 = panel.getBoundingClientRect().height;
    if (h1 <= 60 || panelH1 <= 60) return false;
    if (typeof wrap.getAnimations === 'function' && wrap.getAnimations().some((a) => a.playState === 'running')) {
      return false;
    }
    await new Promise((r) => setTimeout(r, 40));
    const h2 = wrap.getBoundingClientRect().height;
    return Math.abs(h2 - h1) < 0.5 && h2 >= panel.getBoundingClientRect().height - 1;
  }, '#run-panel detailed view open and settled');
}

async function assertSessionSettingsToggleUnclipped(driver, label) {
  const geom = await driver.callFn(() => {
    const cfg = document.querySelector('#sessions-view .configuration');
    const summary = document.getElementById('session-summary');
    const toggle = document.getElementById('session-settings-toggle');
    if (!cfg || !summary || !toggle) {
      return { error: 'Missing .configuration, #session-summary, or #session-settings-toggle' };
    }
    const configRect = cfg.getBoundingClientRect();
    const toggleRect = toggle.getBoundingClientRect();
    return {
      summaryHidden: summary.hasAttribute('hidden'),
      configRect: {
        top: configRect.top,
        bottom: configRect.bottom,
        height: configRect.height,
      },
      toggleRect: {
        top: toggleRect.top,
        bottom: toggleRect.bottom,
        height: toggleRect.height,
        width: toggleRect.width,
      },
    };
  });
  assert(!geom.error, `${label}: ${geom.error}`);
  assert(!geom.summaryHidden, `${label}: expected #session-summary visible on active running session`);
  assert(
    geom.toggleRect.width > 0 && geom.toggleRect.height >= 24,
    `${label}: expected #session-settings-toggle height >= 24, got ${JSON.stringify(geom)}`,
  );
  assert(
    geom.toggleRect.top >= geom.configRect.top - 1 && geom.toggleRect.bottom <= geom.configRect.bottom + 1,
    `${label}: #session-settings-toggle clipped outside .configuration (${JSON.stringify(geom)})`,
  );
}

async function suite5ModalsApprovalsPaletteMobile(driver, cdp, opts) {
  console.log('[Suite 5] Modals, Approvals drawer, Command palette & Mobile drawer');
  // Focus a trigger before opening #command-palette so we can assert focus restoration
  await driver.callFn(() => {
    const btn = document.getElementById('command-palette-open');
    btn?.focus();
  });
  const beforePaletteFocusId = await driver.callFn(() => document.activeElement?.id || '');
  assert(beforePaletteFocusId === 'command-palette-open', `Expected pre-palette focus on #command-palette-open, got "${beforePaletteFocusId}"`);

  // Trigger Ctrl+K (modifiers=2)
  await driver.pressKey({ key: 'k', code: 'KeyK', keyCode: 75, modifiers: 2 });
  await driver.waitFor(() => {
    const p = document.getElementById('command-palette');
    const items = document.querySelectorAll('#command-palette-list .command-palette-item');
    return p && !p.hidden && document.activeElement?.id === 'command-palette-input' && items.length >= 2;
  }, '#command-palette open with input focus');

  const activeBeforeArrow = await driver.callFn(() => {
    return document.getElementById('command-palette-input')?.getAttribute('aria-activedescendant');
  });
  assert(activeBeforeArrow === 'command-palette-option-0', `Expected initial palette option 0, got ${activeBeforeArrow}`);

  await driver.pressKey({ key: 'ArrowDown', code: 'ArrowDown', keyCode: 40 });
  const activeAfterArrow = await driver.waitFor(() => {
    const id = document.getElementById('command-palette-input')?.getAttribute('aria-activedescendant');
    const opt1 = document.getElementById('command-palette-option-1');
    const opt0 = document.getElementById('command-palette-option-0');
    if (id !== 'command-palette-option-1') return null;
    if (opt1?.getAttribute('aria-selected') !== 'true') return null;
    if (opt0?.getAttribute('aria-selected') !== 'false') return null;
    return id;
  }, 'Command Palette ArrowDown moves selection');
  assert(activeAfterArrow === 'command-palette-option-1', `Expected aria-activedescendant command-palette-option-1, got ${activeAfterArrow}`);

  // Verify shortcut <kbd> badges and fuzzy kit3 matching ("osk" -> "Open Source Kit")
  const hasKbdBadge = await driver.callFn(() => {
    const kbds = Array.from(document.querySelectorAll('#command-palette-list .command-palette-kbd'));
    return kbds.some((el) => (el.textContent || '').includes('Alt+A'));
  });
  assert(hasKbdBadge, 'Expected .command-palette-kbd Alt+A shortcut badge in Command Palette');
  await driver.callFn(() => {
    const input = document.getElementById('command-palette-input');
    input.value = 'osk';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await driver.waitFor(() => {
    const items = Array.from(document.querySelectorAll('#command-palette-list .command-palette-item'));
    return items.some((el) => (el.textContent || '').includes('Open Source Kit'));
  }, 'Command Palette fuzzy query "osk" finds Open Source Kit (kit3)', 4_000);

  // Close with Escape and verify focus restoration
  await driver.pressKey({ key: 'Escape', code: 'Escape', keyCode: 27 });
  await driver.waitFor(() => {
    const p = document.getElementById('command-palette');
    return p && p.hidden && document.activeElement?.id === 'command-palette-open';
  }, 'Command Palette Escape closes & restores focus');

  // Verify #run-file-card dialog + inert by opening Detailed run panel and clicking a .run-file chip
  const canOpenRunPanel = await driver.callFn(() => {
    const detailed = document.getElementById('run-detailed');
    const view = document.getElementById('run-view');
    return Boolean(detailed && view?.getAttribute('data-open') === 'true');
  });
  assert(canOpenRunPanel, 'Expected #run-view[data-open="true"] and #run-detailed for #run-file-card check');
  {
    const wasDetailed = await driver.callFn(() => document.getElementById('run-card')?.getAttribute('data-detailed') === 'true');
    if (!wasDetailed) {
      await driver.click('#run-detailed');
    }
    await waitForRunPanelDetailedSettled(driver);
    await assertSessionSettingsToggleUnclipped(driver, 'Suite 5 (1440x900) with #run-detailed open');
    const runFileSel = '#run-map-main button.run-file[data-action="run-file"], #run-map button.run-file[data-action="run-file"]';
    const hasRunFile = await driver.callFn((sel) => Boolean(document.querySelector(sel)), runFileSel);
    assert(hasRunFile, 'Expected clickable button.run-file inside #run-map');
    await driver.click(runFileSel);
    await driver.waitFor(() => !document.getElementById('run-file-card')?.hidden, '#run-file-card opens on .run-file click');
    const fileCardAttrs = await driver.callFn(() => {
      const card = document.getElementById('run-file-card');
      return {
        tagName: card?.tagName || '',
        open: card?.hasAttribute('open') || false,
        role: card.getAttribute('role'),
        ariaModal: card.getAttribute('aria-modal'),
        mainInert: document.getElementById('main')?.hasAttribute('inert'),
      };
    });
    assert(
      fileCardAttrs.tagName === 'DIALOG'
        && fileCardAttrs.open
        && fileCardAttrs.role === 'dialog'
        && fileCardAttrs.ariaModal === 'true'
        && fileCardAttrs.mainInert,
      `Invalid #run-file-card dialog state: ${JSON.stringify(fileCardAttrs)}`,
    );
    await driver.pressKey({ key: 'Escape', code: 'Escape', keyCode: 27 });
    await driver.waitFor(() => document.getElementById('run-file-card')?.hidden && !document.getElementById('run-file-card')?.hasAttribute('open') && !document.getElementById('main')?.hasAttribute('inert'), '#run-file-card closes on Escape');
    await driver.click(runFileSel);
    await driver.waitFor(() => !document.getElementById('run-file-card')?.hidden && document.getElementById('run-file-card')?.hasAttribute('open'), '#run-file-card reopens for backdrop click dismiss');
    await closeDialogByBackdropClick(driver, '#run-file-card', '#run-file-card');
    const backdropFocusStolen = await driver.callFn(() => {
      const active = document.activeElement;
      return Boolean(active && active.matches && active.matches('button.run-file'));
    });
    assert(!backdropFocusStolen, 'Dismissing #run-file-card via backdrop (restore=false) must not steal focus back to .run-file chip');
    if (!wasDetailed) {
      await driver.click('#run-calm');
    }
  }

  // Verify z-index hierarchy across overlays and page
  const zHierarchy = await driver.callFn(() => {
    const zOf = (id) => {
      const raw = getComputedStyle(document.getElementById(id)).zIndex;
      const num = Number.parseInt(raw, 10);
      return Number.isFinite(num) ? num : 0;
    };
    return {
      palette: zOf('command-palette'),
      drawer: zOf('approvals-drawer'),
      fileCard: zOf('run-file-card'),
      main: zOf('main'),
    };
  });
  assert(
    zHierarchy.palette > zHierarchy.drawer && zHierarchy.drawer > zHierarchy.fileCard && zHierarchy.fileCard > zHierarchy.main,
    `Expected z-index command-palette (${zHierarchy.palette}) > approvals-drawer (${zHierarchy.drawer}) > run-file-card (${zHierarchy.fileCard}) > page (${zHierarchy.main})`,
  );

  // Open #approvals-drawer with active pending permission
  await driver.click('#approvals-queue-toggle');
  await driver.waitFor(() => {
    const d = document.getElementById('approvals-drawer');
    const rules = document.getElementById('permission-rules-list');
    return d
      && !d.hidden
      && d.querySelector('.conversation-permission[data-state="pending"] details.permission-diff-wrap > summary')
      && rules && !rules.textContent.includes('Loading remembered session rules');
  }, '#approvals-drawer opens with active pending permission and details/summary');

  const drawerState = await driver.callFn(() => {
    const d = document.getElementById('approvals-drawer');
    const main = document.getElementById('main');
    const sidebar = document.getElementById('sidebar');
    const focusables = Array.from(d.querySelectorAll('button:not([disabled]), a[href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), details > summary:not([tabindex="-1"]), [tabindex]:not([tabindex="-1"])'));
    return {
      role: d.getAttribute('role'),
      ariaModal: d.getAttribute('aria-modal'),
      mainInert: main?.hasAttribute('inert'),
      sidebarInert: sidebar?.hasAttribute('inert'),
      focusableCount: focusables.length,
    };
  });
  assert(drawerState.role === 'dialog', `Expected approvals-drawer role="dialog", got ${drawerState.role}`);
  assert(drawerState.ariaModal === 'true', `Expected approvals-drawer aria-modal="true", got ${drawerState.ariaModal}`);
  assert(drawerState.mainInert === true && drawerState.sidebarInert === true, `Expected #main and #sidebar inert while #approvals-drawer open: ${JSON.stringify(drawerState)}`);

  // Verify approvals drawer deduplicates items by key and request_id
  const drawerDedup = await driver.callFn(() => {
    const items = Array.from(document.querySelectorAll('#approvals-drawer-list .approvals-queue-item'));
    const keys = items.map((el) => el.getAttribute('data-approval-key') || '').filter(Boolean);
    const reqIds = Array.from(document.querySelectorAll('#approvals-drawer-list [data-permission-id]'))
      .map((el) => el.getAttribute('data-permission-id') || '')
      .filter(Boolean);
    return {
      keyCount: keys.length,
      uniqueKeyCount: new Set(keys).size,
      reqCount: reqIds.length,
      uniqueReqCount: new Set(reqIds).size,
    };
  });
  assert(
    drawerDedup.keyCount === drawerDedup.uniqueKeyCount && drawerDedup.reqCount === drawerDedup.uniqueReqCount,
    `Expected deduplicated approval cards in #approvals-drawer-list: ${JSON.stringify(drawerDedup)}`,
  );

  // Tab and Shift+Tab trap cycle through #approvals-drawer including <details><summary>
  const visitedTags = [];
  for (let i = 0; i < drawerState.focusableCount + 2; i += 1) {
    await driver.pressKey({ key: 'Tab', code: 'Tab', keyCode: 9 });
    const focusInfo = await driver.callFn(() => {
      const d = document.getElementById('approvals-drawer');
      const active = document.activeElement;
      return {
        inside: Boolean(d && active && d.contains(active)),
        tag: active?.tagName || '',
      };
    });
    assert(focusInfo.inside, `Tab focus escaped #approvals-drawer at step ${i}: tag=${focusInfo.tag}`);
    visitedTags.push(focusInfo.tag);
  }
  assert(visitedTags.includes('SUMMARY'), `Expected Tab trap in #approvals-drawer to visit <summary>; visited=${JSON.stringify(visitedTags)}`);

  await driver.pressKey({ key: 'Tab', code: 'Tab', keyCode: 9, modifiers: 8 /* Shift */ });
  const shiftTabInside = await driver.callFn(() => {
    const d = document.getElementById('approvals-drawer');
    return Boolean(d && document.activeElement && d.contains(document.activeElement));
  });
  assert(shiftTabInside, 'Shift+Tab focus escaped #approvals-drawer');

  // Verify approval card <details> state survives refresh, then resolve pending permission via Allow
  await driver.callFn(() => {
    const det = document.querySelector('#approvals-drawer .conversation-permission[data-state="pending"] details.permission-diff-wrap');
    if (det) det.open = true;
    window.dispatchEvent(new Event('focus'));
  });
  await driver.waitFor(
    () => Boolean(document.querySelector('#approvals-drawer .conversation-permission[data-state="pending"] details.permission-diff-wrap')?.open),
    '#approvals-drawer <details> state preserved across focus refresh',
    3_000,
  );

  // Escape closes #approvals-drawer and clears inert (Allow resolution runs after 1280x800 #run-detailed checks while session_id is active)
  await driver.pressKey({ key: 'Escape', code: 'Escape', keyCode: 27 });
  await driver.waitFor(() => {
    const d = document.getElementById('approvals-drawer');
    const main = document.getElementById('main');
    const sidebar = document.getElementById('sidebar');
    return d?.hidden === true && !main?.hasAttribute('inert') && !sidebar?.hasAttribute('inert');
  }, '#approvals-drawer closes on Escape and clears inert');

  // Verify Ctrl+K stacked above an open modal (#approvals-drawer): inert, Tab trap, and navigating execution closes underlying modal
  await driver.click('#approvals-queue-toggle');
  await driver.waitFor(() => !document.getElementById('approvals-drawer')?.hidden, '#approvals-drawer opens before stacked Ctrl+K check');
  await driver.pressKey({ key: 'k', code: 'KeyK', keyCode: 75, modifiers: 2 });
  const stackedState = await driver.waitFor(() => {
    const pal = document.getElementById('command-palette');
    const drawer = document.getElementById('approvals-drawer');
    if (!pal || pal.hidden || !pal.hasAttribute('open')) return null;
    return {
      paletteInert: pal.hasAttribute('inert'),
      drawerInert: Boolean(drawer?.hasAttribute('inert')),
      activeId: document.activeElement?.id || '',
    };
  }, '#command-palette stacks above #approvals-drawer and marks drawer inert', 5_000);
  assert(!stackedState.paletteInert && stackedState.drawerInert && stackedState.activeId === 'command-palette-input', `Unexpected stacked palette state: ${JSON.stringify(stackedState)}`);
  await driver.pressKey({ key: 'Tab', code: 'Tab', keyCode: 9 });
  const stackedTabInside = await driver.callFn(() => {
    const pal = document.getElementById('command-palette');
    return Boolean(pal && document.activeElement && pal.contains(document.activeElement));
  });
  assert(stackedTabInside, 'Tab focus escaped #command-palette while stacked over #approvals-drawer');
  await driver.callFn(() => {
    const input = document.getElementById('command-palette-input');
    input.value = 'Sessions';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const viewSessionsOptIdx = await driver.waitFor(() => {
    const items = Array.from(document.querySelectorAll('#command-palette-list .command-palette-item'));
    const idx = items.findIndex((el) => {
      const cat = el.querySelector('.command-palette-cat')?.textContent?.trim() || '';
      const title = el.querySelector('.command-palette-title')?.textContent?.trim() || '';
      return cat === 'View' && title === 'Sessions';
    });
    return idx >= 0 ? idx + 1 : null;
  }, 'Command Palette filters to View: Sessions while stacked over #approvals-drawer', 4_000);
  await driver.click(`#command-palette-option-${viewSessionsOptIdx - 1}`);
  await driver.waitFor(() => {
    const pal = document.getElementById('command-palette');
    const drawer = document.getElementById('approvals-drawer');
    const main = document.getElementById('main');
    return Boolean(pal?.hidden && !pal?.hasAttribute('open') && drawer?.hidden && !drawer?.hasAttribute('open') && !main?.hasAttribute('inert'));
  }, 'Executing navigating Command Palette entry closes underlying #approvals-drawer and clears inert', 6_000);

  // WCAG AA >=4.5:1 contrast on .composer textarea::placeholder & .skill-hint[aria-selected="true"] .skill-hint-argument (Light & Dark)
  // Switch to idle completed other_session_id so composer slash hints open (#skill-hints-panel suppresses while session is running)
  const otherSel = opts?.otherSessionId && opts?.sessionId
    ? `#history [data-action="session-open"][data-payload="${JSON.stringify(opts.otherSessionId).replace(/"/g, '\\"')}"]`
    : '';
  const mainSel = opts?.otherSessionId && opts?.sessionId
    ? `#history [data-action="session-open"][data-payload="${JSON.stringify(opts.sessionId).replace(/"/g, '\\"')}"]`
    : '';
  if (otherSel) {
    await driver.click(otherSel);
    await driver.waitFor(
      (sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true',
      'switch to idle other_session_id before /model hints WCAG check',
      10_000,
      otherSel,
    );
  }
  const themeBeforeContrast = await driver.callFn(() => document.documentElement.getAttribute('data-theme') || 'light');
  for (const theme of ['light', 'dark']) {
    await driver.click(`#theme-${theme}`);
    await driver.waitFor(
      (expected) => document.documentElement.getAttribute('data-theme') === expected,
      `switch theme to ${theme} for WCAG AA check`,
      4_000,
      theme,
    );
    await driver.callFn(() => {
      const p = document.getElementById('prompt');
      p.value = '/model';
      p.focus();
      p.setSelectionRange(p.value.length, p.value.length);
      p.dispatchEvent(new Event('input', { bubbles: true }));
      p.dispatchEvent(new KeyboardEvent('keyup', { key: 'l', code: 'KeyL', bubbles: true }));
    });
    const samples = await driver.waitFor(() => {
      const p = document.getElementById('prompt');
      const panel = document.getElementById('skill-hints-panel');
      const selHint = document.querySelector('#skill-hints .skill-hint[aria-selected="true"]');
      if (!p || !panel || panel.hidden || !selHint) return null;
      const bgOf = (el) => {
        const layers = [];
        for (let cur = el; cur; cur = cur.parentElement) {
          const c = getComputedStyle(cur).backgroundColor;
          const m = c.match(/rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:[,\s/]+([\d.]+))?\s*\)/i);
          if (m) layers.push({ r: Number(m[1]), g: Number(m[2]), b: Number(m[3]), a: m[4] === undefined ? 1 : Number(m[4]) });
        }
        let acc = { r: 255, g: 255, b: 255 };
        for (let i = layers.length - 1; i >= 0; i -= 1) {
          const l = layers[i];
          acc = {
            r: l.r * l.a + acc.r * (1 - l.a),
            g: l.g * l.a + acc.g * (1 - l.a),
            b: l.b * l.a + acc.b * (1 - l.a),
          };
        }
        return `rgb(${acc.r.toFixed(1)}, ${acc.g.toFixed(1)}, ${acc.b.toFixed(1)})`;
      };
      const phCs = getComputedStyle(p, '::placeholder');
      const argEl = selHint.querySelector('.skill-hint-argument');
      const nameEl = selHint.querySelector('.skill-hint-name') || selHint;
      const argCs = argEl ? getComputedStyle(argEl) : null;
      const nameCs = getComputedStyle(nameEl);
      return {
        promptBg: bgOf(p),
        placeholderColor: phCs.color,
        placeholderOpacity: Number.parseFloat(phCs.opacity || '1'),
        hintBg: bgOf(selHint),
        hintNameColor: nameCs.color,
        hintArgColor: argCs?.color || '',
        hintArgOpacity: argCs ? Number.parseFloat(argCs.opacity || '1') : 1,
      };
    }, `${theme} theme .composer placeholder & .skill-hint[aria-selected="true"] samples`, 6_000);
    const phRatio = blendedContrastRatio(samples.placeholderColor, samples.promptBg, samples.placeholderOpacity);
    assert(phRatio !== null && phRatio >= 4.5, `${theme} .composer textarea::placeholder WCAG contrast < 4.5:1 (${phRatio?.toFixed(2)}:1, ${JSON.stringify(samples)})`);
    const nameRatio = blendedContrastRatio(samples.hintNameColor, samples.hintBg, 1);
    assert(nameRatio !== null && nameRatio >= 4.5, `${theme} .skill-hint[aria-selected="true"] .skill-hint-name contrast < 4.5:1 (${nameRatio?.toFixed(2)}:1)`);
    if (samples.hintArgColor) {
      const argRatio = blendedContrastRatio(samples.hintArgColor, samples.hintBg, samples.hintArgOpacity);
      assert(argRatio !== null && argRatio >= 4.5, `${theme} .skill-hint[aria-selected="true"] .skill-hint-argument contrast < 4.5:1 (${argRatio?.toFixed(2)}:1, ${JSON.stringify(samples)})`);
    }
    await driver.callFn(() => {
      const p = document.getElementById('prompt');
      p.value = '';
      p.setSelectionRange(0, 0);
      p.dispatchEvent(new Event('input', { bubbles: true }));
      p.dispatchEvent(new KeyboardEvent('keyup', { key: 'Backspace', code: 'Backspace', bubbles: true }));
    });
  }
  await driver.click(`#theme-${themeBeforeContrast}`);
  await driver.waitFor((t) => document.documentElement.getAttribute('data-theme') === t, 'restore theme after WCAG checks', 4_000, themeBeforeContrast);

  if (otherSel && mainSel) {
    // Verify .skill-hint click (composer-choose) fills /model command and closes #skill-hints-panel
    await driver.callFn(() => {
      const p = document.getElementById('prompt');
      p.value = '/mo';
      p.focus();
      p.setSelectionRange(p.value.length, p.value.length);
      p.dispatchEvent(new Event('input', { bubbles: true }));
      p.dispatchEvent(new KeyboardEvent('keyup', { key: 'o', code: 'KeyO', bubbles: true }));
    });
    await driver.waitFor(() => {
      const panel = document.getElementById('skill-hints-panel');
      const hint = document.querySelector('#skill-hints button.skill-hint[data-action="composer-choose"]');
      return Boolean(panel && !panel.hidden && hint);
    }, '.skill-hint[data-action="composer-choose"] opens for /mo', 5_000);
    await driver.click('#skill-hints button.skill-hint[data-action="composer-choose"]');
    await driver.waitFor(() => {
      const p = document.getElementById('prompt');
      const panel = document.getElementById('skill-hints-panel');
      return Boolean(p && p.value.startsWith('/model') && panel?.hidden);
    }, '.skill-hint click fills composer command and closes #skill-hints-panel', 5_000);
    await driver.callFn(() => {
      const p = document.getElementById('prompt');
      p.value = '';
      p.dispatchEvent(new Event('input', { bubbles: true }));
    });

    // Verify permission-question-card (radio, multi-select checkbox, Other auto-check + Submit answers) and Alt+D shortcut
    const otherEnc = encodeURIComponent(opts.otherSessionId);
    const otherSnapshot = await driver.waitUntil(
      async () => (
        await driver.readLatestFinishedJson(`/api/sessions/${otherEnc}?`, (j) => Boolean(j?.session))
        || await driver.readLatestFinishedJson(`/api/sessions/${opts.otherSessionId}?`, (j) => Boolean(j?.session))
      ),
      'capture finished JSON payload for other_session_id before question card test',
      10_000,
    );
    if (otherSnapshot?.session) {
      await driver.click(mainSel);
      await driver.waitFor(
        (sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true',
        'park on session_id before permission-question-card Fetch override',
        10_000,
        mainSel,
      );
      const qPerm = {
        id: 90001,
        kind: 'permission',
        request_id: 'perm-q-e2e',
        tool: 'AskUserQuestion',
        tool_name: 'AskUserQuestion',
        ask_kind: 'question',
        state: 'pending',
        questions: [
          {
            question: 'Target stack?',
            header: 'Stack',
            multiSelect: false,
            options: [
              { label: 'Laravel', description: 'Laravel edition' },
              { label: 'Symfony', description: 'Symfony edition' },
            ],
          },
          {
            question: 'Checks to run?',
            header: 'Checks',
            multiSelect: true,
            options: [
              { label: 'Lint', description: 'Formatting and static checks' },
              { label: 'Unit', description: 'Fast test suite' },
            ],
          },
        ],
      };
      const denyPerm = {
        id: 90002,
        kind: 'permission',
        request_id: 'perm-d-e2e',
        tool: 'Edit',
        tool_name: 'Edit',
        state: 'pending',
        input: { file_path: 'entry_01.js', old_string: 'a', new_string: 'b' },
      };
      const qSessionPayload = {
        ...otherSnapshot,
        session: {
          ...otherSnapshot.session,
          status: 'running',
          waiting_permissions: 2,
        },
        waiting_permissions: [qPerm, denyPerm],
        events: [...(otherSnapshot.events || []), qPerm, denyPerm],
      };
      const decisionPosts = [];
      await withFetchOverride(
        cdp,
        [{ urlPattern: `${driver.origin}/api/sessions/*` }],
        async (params) => {
          const reqUrl = params.request?.url || '';
          const method = (params.request?.method || 'GET').toUpperCase();
          const parsed = new URL(reqUrl);
          const matchesOther = parsed.pathname === `/api/sessions/${opts.otherSessionId}`
            || parsed.pathname === `/api/sessions/${otherEnc}`;
          if (method === 'GET' && matchesOther) {
            const qDone = decisionPosts.some((p) => p.url.includes('perm-q-e2e'));
            const dDone = decisionPosts.some((p) => p.url.includes('perm-d-e2e'));
            const remaining = [qDone ? null : qPerm, dDone ? null : denyPerm].filter(Boolean);
            const payload = parsed.searchParams.has('after')
              ? {
                  session: { ...qSessionPayload.session, waiting_permissions: remaining.length },
                  waiting_permissions: remaining,
                  events: [],
                }
              : qSessionPayload;
            await cdp.send('Fetch.fulfillRequest', {
              requestId: params.requestId,
              responseCode: 200,
              responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
              body: Buffer.from(JSON.stringify(payload)).toString('base64'),
            });
            return true;
          }
          if (method === 'POST' && (reqUrl.includes('/permissions/perm-q-e2e') || reqUrl.includes('/permissions/perm-d-e2e'))) {
            let parsedBody = null;
            try {
              parsedBody = params.request?.postData ? JSON.parse(params.request.postData) : null;
            } catch {
              parsedBody = null;
            }
            decisionPosts.push({ url: reqUrl, body: parsedBody });
            await cdp.send('Fetch.fulfillRequest', {
              requestId: params.requestId,
              responseCode: 200,
              responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
              body: Buffer.from(JSON.stringify({ ok: true })).toString('base64'),
            });
            return true;
          }
          return false;
        },
        async () => {
          await driver.click(otherSel);
          await driver.waitFor(() => {
            const qCard = document.querySelector('#events [data-permission-id="perm-q-e2e"] .permission-question-card');
            const dCard = document.querySelector('#events [data-permission-id="perm-d-e2e"] button[data-decision="deny"]');
            return Boolean(qCard && dCard && qCard.querySelectorAll('fieldset.permission-question').length === 2);
          }, 'permission-question-card and Alt+D target card render in #events', 10_000);

          // Select radio option first, then type into Other input and assert Other radio auto-checks
          await driver.callFn(() => {
            const q0 = document.querySelector('#events [data-permission-id="perm-q-e2e"] fieldset.permission-question[data-question-index="0"]');
            const firstRadio = q0?.querySelector('input.permission-question-choice[type="radio"]:not(.permission-question-other-radio)');
            if (firstRadio) firstRadio.click();
            const otherInput = q0?.querySelector('input.permission-question-other-input');
            if (otherInput) {
              otherInput.value = 'PHP Core Custom';
              otherInput.dispatchEvent(new Event('input', { bubbles: true }));
            }
            const q1Checks = document.querySelectorAll('#events [data-permission-id="perm-q-e2e"] fieldset.permission-question[data-question-index="1"] input.permission-question-choice[type="checkbox"]');
            q1Checks.forEach((cb) => {
              if (!cb.checked) cb.click();
            });
          });
          await driver.waitFor(() => {
            const otherRadio = document.querySelector('#events [data-permission-id="perm-q-e2e"] fieldset.permission-question[data-question-index="0"] input.permission-question-other-radio');
            return Boolean(otherRadio?.checked);
          }, 'permission-question-other input auto-checks Other radio', 4_000);

          await driver.click('#events [data-permission-id="perm-q-e2e"] button[data-action="permission-decide"][data-decision="allow"]');
          await driver.waitUntil(
            () => decisionPosts.some((p) => p.url.includes('perm-q-e2e')),
            'Submit permission-question-card answers POSTs to server',
            6_000,
          );
          const qPost = decisionPosts.find((p) => p.url.includes('perm-q-e2e'));
          assert(
            qPost?.body?.decision === 'allow'
              && qPost?.body?.answers?.['Target stack?'] === 'PHP Core Custom'
              && Array.isArray(qPost?.body?.answers?.['Checks to run?'])
              && qPost.body.answers['Checks to run?'].includes('Lint')
              && qPost.body.answers['Checks to run?'].includes('Unit'),
            `Unexpected permission-question-card answers payload: ${JSON.stringify(qPost)}`,
          );

          // Focus perm-d-e2e button and trigger Alt+D (modifiers: 1) deny shortcut
          await driver.callFn(() => {
            document.querySelector('#events [data-permission-id="perm-d-e2e"] button[data-action="permission-decide"][data-decision="deny"]')?.focus();
          });
          await driver.pressKey({ key: 'd', code: 'KeyD', keyCode: 68, modifiers: 1 });
          await driver.waitUntil(
            () => decisionPosts.some((p) => p.url.includes('perm-d-e2e')),
            'Alt+D shortcut POSTs deny decision',
            6_000,
          );
          const dPost = decisionPosts.find((p) => p.url.includes('perm-d-e2e'));
          assert(dPost?.body?.decision === 'deny', `Expected Alt+D deny decision, got ${JSON.stringify(dPost)}`);
        },
      );

      // Verify #session-inbox queue & inbox-cancel flow while a native session is running
      let inboxItems = [];
      const runningInboxSession = {
        ...otherSnapshot,
        session: {
          ...otherSnapshot.session,
          status: 'running',
          workflow: 'native',
          agents_enabled: false,
          brain: null,
          provider: 'codex',
          mode: 'edit',
          native_session_id: 'nat-e2e-inbox',
          git_common_dir: '/repo/.git',
        },
      };
      await withFetchOverride(
        cdp,
        [{ urlPattern: `${driver.origin}/api/sessions/*` }],
        async (params) => {
          const reqUrl = params.request?.url || '';
          const method = (params.request?.method || 'GET').toUpperCase();
          const parsed = new URL(reqUrl);
          const isOtherBase = parsed.pathname === `/api/sessions/${opts.otherSessionId}`
            || parsed.pathname === `/api/sessions/${otherEnc}`;
          const isOtherInbox = parsed.pathname === `/api/sessions/${opts.otherSessionId}/inbox`
            || parsed.pathname === `/api/sessions/${otherEnc}/inbox`;
          const isOtherCancel = parsed.pathname.startsWith(`/api/sessions/${opts.otherSessionId}/inbox/`)
            || parsed.pathname.startsWith(`/api/sessions/${otherEnc}/inbox/`);

          if (method === 'GET' && isOtherBase) {
            const payload = parsed.searchParams.has('after')
              ? { session: runningInboxSession.session, waiting_permissions: [], events: [] }
              : runningInboxSession;
            await cdp.send('Fetch.fulfillRequest', {
              requestId: params.requestId,
              responseCode: 200,
              responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
              body: Buffer.from(JSON.stringify(payload)).toString('base64'),
            });
            return true;
          }
          if (method === 'GET' && isOtherInbox) {
            await cdp.send('Fetch.fulfillRequest', {
              requestId: params.requestId,
              responseCode: 200,
              responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
              body: Buffer.from(JSON.stringify({ items: inboxItems })).toString('base64'),
            });
            return true;
          }
          if (method === 'POST' && isOtherInbox) {
            const posted = JSON.parse(params.request?.postData || '{}');
            const receipt = {
              id: 701,
              request_id: posted.request_id || 'e2e-inbox-1',
              session_id: opts.otherSessionId,
              prompt: posted.prompt || 'Queued E2E follow-up',
              status: 'pending',
            };
            inboxItems = [receipt];
            await cdp.send('Fetch.fulfillRequest', {
              requestId: params.requestId,
              responseCode: 202,
              responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
              body: Buffer.from(JSON.stringify(receipt)).toString('base64'),
            });
            return true;
          }
          if (method === 'POST' && isOtherCancel && parsed.pathname.endsWith('/cancel')) {
            const cancelled = {
              ...inboxItems[0],
              status: 'cancelled',
            };
            inboxItems = [cancelled];
            await cdp.send('Fetch.fulfillRequest', {
              requestId: params.requestId,
              responseCode: 200,
              responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
              body: Buffer.from(JSON.stringify(cancelled)).toString('base64'),
            });
            return true;
          }
          return false;
        },
        async () => {
          await driver.click('#refresh-sessions');
          await driver.waitFor(() => {
            const p = document.getElementById('prompt');
            return Boolean(p && !p.disabled && document.getElementById('send-label')?.textContent?.includes('Queue'));
          }, 'running native session switches composer button to Queue', 8_000).catch(() => {});
          await driver.callFn(() => {
            const p = document.getElementById('prompt');
            p.value = 'Queued E2E follow-up';
            p.dispatchEvent(new Event('input', { bubbles: true }));
          });
          await driver.waitFor(() => !document.getElementById('send')?.disabled, '#send enabled for queued inbox message', 5_000);
          await driver.click('#send');
          await driver.waitFor(() => {
            const sec = document.getElementById('session-inbox');
            const cancelBtn = document.querySelector('#inbox-receipts button[data-action="inbox-cancel"]');
            return Boolean(sec && !sec.hidden && cancelBtn && (sec.textContent || '').includes('Queued for the next turn'));
          }, '#session-inbox shows pending receipt with Cancel button', 8_000);
          await driver.click('#inbox-receipts button[data-action="inbox-cancel"]');
          await driver.waitFor(() => {
            const text = document.getElementById('inbox-receipts')?.textContent || '';
            return text.includes('Cancelled') && !document.querySelector('#inbox-receipts button[data-action="inbox-cancel"]');
          }, 'inbox-cancel transitions queued receipt to Cancelled', 8_000);
        },
      );
    }
  }
  if (mainSel) {
    await driver.click(mainSel);
    await driver.waitFor(
      (sel) => {
        const rv = document.getElementById('run-view');
        const rvAnims = rv && typeof rv.getAnimations === 'function'
          ? rv.getAnimations().some((a) => a.playState === 'running')
          : false;
        return document.querySelector(sel)?.getAttribute('aria-current') === 'true'
          && rv?.getAttribute('data-open') === 'true'
          && rv.getBoundingClientRect().height > 20
          && !rvAnims;
      },
      'switch back to primary session_id with #run-view open and settled before 1280x800 #run-detailed checks',
      10_000,
      mainSel,
    );
  }

  // 1. Resize 1280x800 -> check #run-detailed + #session-settings-toggle unclipped while session_id is still running
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 1280,
    height: 800,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await driver.waitFor(() => {
    const toggle = document.getElementById('menu-toggle');
    return Boolean(toggle && window.innerWidth === 1280 && getComputedStyle(toggle).display === 'none');
  }, '1280x800 desktop layout before #run-detailed check');

  const wasDetailedBeforeDesktopCheck = await driver.callFn(() => document.getElementById('run-card')?.getAttribute('data-detailed') === 'true');
  if (!wasDetailedBeforeDesktopCheck) {
    await driver.click('#run-detailed');
  }
  await waitForRunPanelDetailedSettled(driver);
  await assertSessionSettingsToggleUnclipped(driver, 'Suite 5 (1280x800) with #run-detailed open');
  if (!wasDetailedBeforeDesktopCheck) {
    await driver.click('#run-calm');
  }

  // 2. Verify #history-filter-waiting lists waiting session before Allow, resolve perm-e2e-1 via Alt+A, then assert immediate clear
  if (mainSel && otherSel) {
    await driver.click('#history-filter-waiting');
    await driver.waitFor((waitingSel, idleSel) => {
      const btn = document.getElementById('history-filter-waiting');
      return Boolean(
        btn?.getAttribute('aria-pressed') === 'true'
          && document.querySelector(waitingSel)
          && !document.querySelector(idleSel),
      );
    }, '#history-filter-waiting keeps waiting session_id and filters out idle other_session_id', 6_000, mainSel, otherSel);
    await driver.click('#history-filter-waiting');
    await driver.waitFor((idleSel) => {
      const btn = document.getElementById('history-filter-waiting');
      return btn?.getAttribute('aria-pressed') === 'false' && Boolean(document.querySelector(idleSel));
    }, 'toggling #history-filter-waiting restores all sessions', 5_000, otherSel);
  }

  await driver.click('#approvals-queue-toggle');
  await driver.waitFor(
    () => {
      const btn = document.querySelector('#approvals-drawer .conversation-permission[data-state="pending"] button[data-action="permission-decide"]');
      return Boolean(btn && document.activeElement && btn.closest('#approvals-drawer .conversation-permission[data-state="pending"]'));
    },
    '#approvals-drawer auto-focuses pending permission control on open for immediate Alt+A',
  );
  const sseBeforeAllow = driver.sseMessages.length;
  await driver.pressKey({ key: 'a', code: 'KeyA', keyCode: 65, modifiers: 1 });
  await driver.waitUntil(
    () => driver.sseMessages.slice(sseBeforeAllow).some((m) => m.data.includes('Permission request handled') || m.eventName === 'session_status'),
    'SSE delivers permission resolution event after Alt+A Allow shortcut',
    15_000,
  );
  await driver.waitFor(() => {
    const toggle = document.getElementById('approvals-queue-toggle');
    const cnt = document.getElementById('approvals-queue-count');
    const eventsText = document.getElementById('events')?.textContent || '';
    const cleared = toggle?.getAttribute('data-count') === '0' && (!cnt || cnt.hidden || cnt.textContent.trim() === '0');
    const runClosed = document.getElementById('run-view')?.getAttribute('data-open') === 'false';
    return cleared && eventsText.includes('Permission request handled') && runClosed;
  }, 'approvals count clears immediately, #events shows Permission request handled, and session_id completes', 15_000);
  const drawerStillOpen = await driver.callFn(() => document.getElementById('approvals-drawer')?.hidden === false);
  if (drawerStillOpen) {
    await driver.pressKey({ key: 'Escape', code: 'Escape', keyCode: 27 });
    await driver.waitFor(() => document.getElementById('approvals-drawer')?.hidden === true, '#approvals-drawer closed after Allow');
  }

  // P0 check: #history-filter-waiting must immediately show 0 waiting sessions after approval (before & after #refresh-sessions)
  await driver.click('#history-filter-waiting');
  await driver.waitFor(() => {
    const rows = document.querySelectorAll('#history [data-action="session-open"]').length;
    const btn = document.getElementById('history-filter-waiting');
    return btn?.getAttribute('aria-pressed') === 'true' && rows === 0;
  }, 'Waiting approval filter clears immediately to 0 sessions after approval', 5_000);
  await driver.click('#refresh-sessions');
  await driver.waitFor(() => {
    const rows = document.querySelectorAll('#history [data-action="session-open"]').length;
    const cnt = document.getElementById('approvals-queue-count');
    const toggle = document.getElementById('approvals-queue-toggle');
    return rows === 0 && toggle?.getAttribute('data-count') === '0' && (!cnt || cnt.hidden || cnt.textContent.trim() === '0');
  }, 'Waiting approval filter stays at 0 sessions after #refresh-sessions', 6_000);
  await driver.click('#history-filter-waiting');
  await driver.waitFor(() => {
    const btn = document.getElementById('history-filter-waiting');
    return btn?.getAttribute('aria-pressed') === 'false' && document.querySelectorAll('#history [data-action="session-open"]').length >= 2;
  }, 'toggling #history-filter-waiting off restores sessions after P0 check', 5_000);

  // 3. Responsive 761-920px breakpoint check (840x900) + /resume sidebar inert & focus
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 840,
    height: 900,
    deviceScaleFactor: 1,
    mobile: false,
  });
  const midViewport = await driver.waitFor(() => {
    const btn = document.getElementById('menu-toggle');
    if (!btn || getComputedStyle(btn).display === 'none' || window.innerWidth !== 840) return null;
    return { scrollWidth: document.documentElement.scrollWidth, innerWidth: window.innerWidth };
  }, '840x900 responsive layout settled');
  assert(midViewport.scrollWidth <= midViewport.innerWidth + 1, `Horizontal overflow at 840x900: ${JSON.stringify(midViewport)}`);

  await driver.callFn(() => {
    const p = document.getElementById('prompt');
    p.value = '/resume';
    p.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await driver.click('#send');
  const resumeState = await driver.waitFor(() => {
    const sb = document.getElementById('sidebar');
    const main = document.getElementById('main');
    if (!sb || !sb.classList.contains('open')) return null;
    return {
      open: true,
      mainInert: main?.hasAttribute('inert'),
      sidebarInert: sb.hasAttribute('inert'),
      focusInsideSidebar: Boolean(document.activeElement && sb.contains(document.activeElement)),
    };
  }, '/resume opens #sidebar with #main inert and sidebar focus', 5_000);
  assert(
    resumeState.mainInert && !resumeState.sidebarInert && resumeState.focusInsideSidebar,
    `/resume sidebar inert/focus check failed: ${JSON.stringify(resumeState)}`,
  );
  const resumeScrim = await driver.callFn(() => {
    const scrim = document.getElementById('scrim');
    if (!scrim) return null;
    const r = scrim.getBoundingClientRect();
    return { x: Math.max(r.left + 8, r.right - 24), y: r.top + 120 };
  });
  assert(resumeScrim, 'Expected exposed #scrim at 840x900 after /resume');
  await driver.clickAt(resumeScrim.x, resumeScrim.y);
  await driver.waitFor(() => !document.getElementById('sidebar')?.classList.contains('open') && !document.getElementById('main')?.hasAttribute('inert'), 'close /resume drawer via #scrim');

  // 4. Mobile viewport 390x844 & #menu-toggle -> #sidebar.open + mobile Workspace tab without horizontal overflow
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 390,
    height: 844,
    deviceScaleFactor: 2,
    mobile: true,
  });
  await driver.waitFor(() => {
    const btn = document.getElementById('menu-toggle');
    return btn && getComputedStyle(btn).display !== 'none';
  }, 'mobile #menu-toggle visible at 390x844');
  const mobileTitleWidth = await driver.callFn(() => document.getElementById('page-title')?.getBoundingClientRect().width || 0);
  assert(mobileTitleWidth >= 48, `Expected readable #page-title width >= 48px at 390x844, got ${mobileTitleWidth}`);

  await driver.click('#menu-toggle');
  const mobileDrawer = await driver.waitFor(() => {
    const sb = document.getElementById('sidebar');
    if (!sb || !sb.classList.contains('open')) return null;
    return {
      open: true,
      scrollWidth: document.documentElement.scrollWidth,
      innerWidth: window.innerWidth,
      mainInert: document.getElementById('main')?.hasAttribute('inert'),
      sidebarInert: sb.hasAttribute('inert'),
    };
  }, '#sidebar.open at 390x844');

  assert(mobileDrawer.mainInert && !mobileDrawer.sidebarInert, `Expected #main inert and #sidebar active in mobile drawer: ${JSON.stringify(mobileDrawer)}`);
  assert(
    mobileDrawer.scrollWidth <= mobileDrawer.innerWidth + 1,
    `Horizontal overflow detected at 390x844: scrollWidth=${mobileDrawer.scrollWidth}, innerWidth=${mobileDrawer.innerWidth}`,
  );

  // Close mobile #sidebar via exposed #scrim (to the right of the 265px drawer), test Workspace mobile tab
  const scrimPt = await driver.callFn(() => {
    const scrim = document.getElementById('scrim');
    if (!scrim) return null;
    const r = scrim.getBoundingClientRect();
    const x = Math.max(r.left + 8, r.right - 24);
    const y = r.top + Math.min(160, Math.max(24, r.height * 0.35));
    const hit = document.elementFromPoint(x, y);
    return hit && (hit === scrim || scrim.contains(hit)) ? { x, y } : null;
  });
  assert(scrimPt, 'Expected clickable exposed #scrim outside mobile #sidebar');
  await driver.clickAt(scrimPt.x, scrimPt.y);
  await driver.waitFor(() => {
    const sb = document.getElementById('sidebar');
    const main = document.getElementById('main');
    return sb && !sb.classList.contains('open') && sb.hasAttribute('inert') && main && !main.hasAttribute('inert');
  }, 'mobile #sidebar closes on #scrim click and restores #main active');

  await driver.click('#session-mobile-tab-graph');
  const mobileWs = await driver.waitFor(() => {
    const tab = document.getElementById('session-mobile-tab-graph');
    const pane = document.getElementById('sessions-right-pane');
    if (tab?.getAttribute('aria-selected') !== 'true' || !pane) return null;
    const r = pane.getBoundingClientRect();
    return {
      width: r.width,
      height: r.height,
      scrollWidth: document.documentElement.scrollWidth,
      innerWidth: window.innerWidth,
    };
  }, 'mobile Workspace tab switches to #sessions-right-pane');
  assert(
    mobileWs.width > 120 && mobileWs.height > 120 && mobileWs.scrollWidth <= mobileWs.innerWidth + 1,
    `Mobile Workspace pane layout check failed: ${JSON.stringify(mobileWs)}`,
  );
  await driver.click('#session-mobile-tab-chat');
  await driver.waitFor(() => document.getElementById('session-mobile-tab-chat')?.getAttribute('aria-selected') === 'true', 'restore mobile Chat tab');

  // 5. Restore 1280x800 -> verify !sidebarOpen, !inert, and #theme-* click inside #sidebar
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: 1280,
    height: 800,
    deviceScaleFactor: 1,
    mobile: false,
  });
  const desktopShell = await driver.waitFor(async () => {
    const sb = document.getElementById('sidebar');
    const main = document.getElementById('main');
    const toggle = document.getElementById('menu-toggle');
    if (!sb || !main || !toggle) return null;
    if (window.innerWidth !== 1280 || getComputedStyle(toggle).display !== 'none') return null;
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    return {
      sidebarOpen: sb.classList.contains('open'),
      sidebarInert: sb.hasAttribute('inert'),
      mainInert: main.hasAttribute('inert'),
    };
  }, 'desktop 1280x800 layout restoration after mobile drawer close');
  assert(
    !desktopShell.sidebarOpen && !desktopShell.sidebarInert && !desktopShell.mainInert,
    `Expected #sidebar and #main not inert after restoring 1280x800 desktop viewport: ${JSON.stringify(desktopShell)}`,
  );

  // Verify clicking #theme-dark / #theme-light inside #sidebar succeeds cleanly (no element is inert)
  const themeBeforeSidebarClick = await driver.callFn(() => document.documentElement.getAttribute('data-theme') || 'light');
  const targetTheme = themeBeforeSidebarClick === 'dark' ? 'light' : 'dark';
  await driver.click(`#theme-${targetTheme}`);
  await driver.waitFor(
    (expected) => document.documentElement.getAttribute('data-theme') === expected,
    `#theme-${targetTheme} click inside #sidebar after mobile -> desktop restore`,
    3_000,
    targetTheme,
  );
  await driver.click(`#theme-${themeBeforeSidebarClick}`);
  await driver.waitFor(
    (expected) => document.documentElement.getAttribute('data-theme') === expected,
    `restore data-theme to ${themeBeforeSidebarClick} after #sidebar click check`,
    3_000,
    themeBeforeSidebarClick,
  );

  driver.assertNoErrors('Suite 5');
}

async function suite6OmnigraphStudio(driver, cdp) {
  console.log('[Suite 6] Omnigraph Context Studio (radar, Reset filters, search, pulse)');
  await driver.click('#sidebar-omnigraph-pill');
  await driver.waitFor(() => {
    const studio = document.getElementById('omnigraph-studio');
    const canvas = document.getElementById('omnigraph-canvas');
    const radar = document.querySelector('#omnigraph-canvas-radar .omni-radar-svg .omni-radar-view');
    const items = canvas?.querySelectorAll('.omni-node, canvas.graph-dense-canvas[data-dense-canvas="true"]').length || 0;
    return Boolean(studio && !studio.hidden && canvas && items > 0 && radar);
  }, 'Omnigraph studio constellation & radar visible', 15_000);

  // Pan constellation radar to bottom-right corner and verify clamped viewBox bounds
  const radarBoxes = await driver.callFn(() => {
    const svg = document.querySelector('#omnigraph-canvas-radar .omni-radar-svg');
    const view = svg?.querySelector('.omni-radar-view');
    if (!svg || !view) return null;
    const sr = svg.getBoundingClientRect();
    const vr = view.getBoundingClientRect();
    const vb = svg.viewBox?.baseVal;
    return {
      sx: sr.left,
      sy: sr.top,
      sw: sr.width,
      sh: sr.height,
      vx: vr.left + vr.width / 2,
      vy: vr.top + vr.height / 2,
      vbW: vb?.width || sr.width,
      vbH: vb?.height || sr.height,
    };
  });
  assert(radarBoxes, 'Expected .omni-radar-svg and .omni-radar-view inside #omnigraph-canvas-radar');
  await driver.dragAt(
    radarBoxes.vx,
    radarBoxes.vy,
    radarBoxes.sx + radarBoxes.sw - 3,
    radarBoxes.sy + radarBoxes.sh - 3,
    { steps: 8 },
  );
  const clampedRadar = await driver.callFn(async () => {
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    const svg = document.querySelector('#omnigraph-canvas-radar .omni-radar-svg');
    const view = svg?.querySelector('.omni-radar-view');
    if (!svg || !view) return null;
    const vb = svg.viewBox?.baseVal;
    const x = Number.parseFloat(view.getAttribute('x') || '0');
    const y = Number.parseFloat(view.getAttribute('y') || '0');
    const w = Number.parseFloat(view.getAttribute('width') || '0');
    const h = Number.parseFloat(view.getAttribute('height') || '0');
    return { x, y, w, h, vbW: vb?.width || 0, vbH: vb?.height || 0 };
  });
  assert(
    clampedRadar
      && clampedRadar.x >= -0.5
      && clampedRadar.y >= -0.5
      && clampedRadar.x + clampedRadar.w <= clampedRadar.vbW + 1
      && clampedRadar.y + clampedRadar.h <= clampedRadar.vbH + 1,
    `Expected Omnigraph radar view rect clamped inside SVG bounds: ${JSON.stringify(clampedRadar)}`,
  );

  // Select `main` branch, test search hit, then zero-result Reset filters on `main`
  await driver.callFn(() => {
    const sel = document.getElementById('omnigraph-branch');
    if (sel && sel.value !== 'main') {
      sel.value = 'main';
      sel.dispatchEvent(new Event('change', { bubbles: true }));
    }
  });
  await driver.waitFor(
    () => document.getElementById('omnigraph-branch')?.value === 'main'
      && document.getElementById('omnigraph-canvas')?.getAttribute('aria-busy') !== 'true',
    'Omnigraph main branch ready',
  );

  await driver.callFn(() => {
    const input = document.getElementById('omnigraph-search');
    input.value = 'backoff';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await driver.waitFor(() => {
    const c = document.getElementById('omnigraph-canvas');
    const text = c?.textContent || '';
    const count = c?.querySelectorAll('.omni-node, canvas.graph-dense-canvas[data-dense-canvas="true"]').length || 0;
    return c?.getAttribute('aria-busy') !== 'true' && (text.includes('backoff') || count > 0);
  }, 'Omnigraph search match for "backoff"', 10_000);

  await driver.callFn(() => {
    const input = document.getElementById('omnigraph-search');
    input.value = 'zzz-nonexistent-e2e-filter';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const resetBox = await driver.waitFor(() => {
    const c = document.getElementById('omnigraph-canvas');
    if (!c || c.getAttribute('aria-busy') === 'true') return null;
    const btn = Array.from(c.querySelectorAll('button')).find((b) => (b.textContent || '').includes('Reset filters'));
    if (!btn) return null;
    const r = btn.getBoundingClientRect();
    return r.width > 0 && r.height > 0 ? { cx: r.left + r.width / 2, cy: r.top + r.height / 2 } : null;
  }, 'Omnigraph zero-result empty state "Reset filters" button on main', 10_000);
  await driver.clickAt(resetBox.cx, resetBox.cy);
  await driver.waitFor(() => {
    const input = document.getElementById('omnigraph-search');
    const c = document.getElementById('omnigraph-canvas');
    const count = c?.querySelectorAll('.omni-node, canvas.graph-dense-canvas[data-dense-canvas="true"]').length || 0;
    return input?.value === '' && c?.getAttribute('aria-busy') !== 'true' && count > 0;
  }, 'Omnigraph Reset filters clears query and restores constellation', 10_000);

  // Pause & resume pulse toggle + Omnigraph navigation history bar present
  await driver.waitFor(
    () => Boolean(document.querySelector('#omni-history-bar #omni-history-back') && document.querySelector('#omni-history-bar #omni-history-forward')),
    'Omnigraph history bar rendered',
  );
  await driver.click('#omni-toggle-pulse');
  await driver.waitFor(
    () => document.getElementById('omnigraph-view')?.getAttribute('data-pulse-paused') === 'true'
      && (document.getElementById('omni-toggle-pulse')?.textContent || '').includes('Play pulse'),
    'Omnigraph pulse paused',
  );
  await driver.click('#omni-toggle-pulse');
  await driver.waitFor(
    () => document.getElementById('omnigraph-view')?.getAttribute('data-pulse-paused') === 'false',
    'Omnigraph pulse resumed',
  );

  // Exercise #omnigraph-transfer-dialog (Export/Import tabs + preview) and close via backdrop click
  await withFetchOverride(
    cdp,
    [{ urlPattern: `${driver.origin}/api/omnigraph/export*` }],
    async (params) => {
      if ((params.request?.url || '').includes('/api/omnigraph/export')) {
        const exportPayload = {
          ok: true,
          filename: 'omnigraph-context.json',
          source_branch: 'main',
          counts: { atoms: 1, files: 1, edges: 1, sessions: 1 },
          bundle: {
            format: 'harness.omnigraph.bundle.v1',
            source_branch: 'main',
            atoms: [{ id: 'atom-finding-backoff', kind: 'finding', title: 'Retry backoff finding' }],
            files: [{ path: 'entry_00.js' }],
            edges: [],
          },
        };
        await cdp.send('Fetch.fulfillRequest', {
          requestId: params.requestId,
          responseCode: 200,
          responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
          body: Buffer.from(JSON.stringify(exportPayload)).toString('base64'),
        });
        return true;
      }
      return false;
    },
    async () => {
      await driver.click('#omnigraph-export-open');
      await driver.waitFor(() => Boolean(document.getElementById('omnigraph-transfer-dialog')?.hasAttribute('open')), '#omnigraph-transfer-dialog opens', 5_000);
      await driver.click('#omni-transfer-tab-import');
      await driver.waitFor(() => {
        const pane = document.getElementById('omni-transfer-pane-import');
        return Boolean(pane && !pane.hidden && document.getElementById('omni-transfer-tab-import')?.getAttribute('aria-selected') === 'true');
      }, '#omnigraph-transfer-dialog switches to Import tab', 4_000);
      await driver.click('#omni-transfer-tab-export');
      await driver.waitFor(() => document.getElementById('omni-transfer-tab-export')?.getAttribute('aria-selected') === 'true', 'restore Export tab', 4_000);
      await driver.click('#omni-export-preview-btn');
      await driver.waitFor(() => (document.getElementById('omni-export-preview-code')?.textContent || '').includes('atom-finding-backoff'), '#omni-export-preview-code populated', 8_000);
      await closeDialogByBackdropClick(driver, '#omnigraph-transfer-dialog', 'Omnigraph transfer dialog');
    },
  );

  // Exercise #omnigraph-share-dialog (from sidebar Share button) and close via backdrop click
  await driver.click('#share-sessions-context');
  await driver.waitFor(() => Boolean(document.getElementById('omnigraph-share-dialog')?.hasAttribute('open')), '#omnigraph-share-dialog opens', 5_000);
  await driver.click('#omni-share-mode-broadcast');
  await driver.waitFor(() => !document.getElementById('omni-share-source-field')?.hidden, '#omnigraph-share-dialog switches to broadcast mode', 4_000);
  await closeDialogByBackdropClick(driver, '#omnigraph-share-dialog', 'Omnigraph share dialog');

  driver.assertNoErrors('Suite 6');
}

async function suite7SystemMap(driver, opts) {
  console.log('[Suite 7] System Map (kinds_off legend, unfold risk, side-tab nav, waves)');
  await driver.click('button.nav-item[data-nav="systems"]');
  await driver.waitFor(() => {
    const cat = document.getElementById('system-catalog');
    const c = document.getElementById('system-map-canvas');
    const nodes = c?.querySelectorAll('.system-map-node, .graph-map-node, canvas.graph-dense-canvas[data-dense-canvas="true"]').length || 0;
    const legendBtns = document.querySelectorAll('#system-map-arrow-legend button[data-action="system-map-kind-toggle"]').length;
    return Boolean(cat && !cat.hidden && nodes > 0 && legendBtns > 0);
  }, 'System catalog & #system-map-canvas rendered with legend', 15_000);

  // Legend kinds_off filter toggle
  const firstLegendKind = '#system-map-arrow-legend button[data-action="system-map-kind-toggle"][aria-pressed="true"]';
  const kindPayload = await driver.callFn((sel) => document.querySelector(sel)?.getAttribute('data-sys-edge-kind') || '', firstLegendKind);
  assert(kindPayload, 'Expected active system arrow legend kind button');
  const kindBtnSel = `#system-map-arrow-legend button[data-action="system-map-kind-toggle"][data-sys-edge-kind="${kindPayload}"]`;
  await driver.click(kindBtnSel);
  await driver.waitFor(
    (sel) => document.querySelector(sel)?.getAttribute('aria-pressed') === 'false',
    `SystemMap legend kind "${kindPayload}" toggled off (kinds_off)`,
    5_000,
    kindBtnSel,
  );
  await driver.click(kindBtnSel);
  await driver.waitFor(
    (sel) => document.querySelector(sel)?.getAttribute('aria-pressed') === 'true',
    `SystemMap legend kind "${kindPayload}" restored on`,
    5_000,
    kindBtnSel,
  );

  // Unfold contracts & unmatched risks (gateway -> payments)
  await driver.click('#system-map-unfold');
  await driver.waitFor(
    () => document.getElementById('system-map-unfold')?.getAttribute('aria-pressed') === 'true'
      && document.getElementById('system-map-canvas')?.getAttribute('aria-busy') !== 'true',
    'SystemMap unfold with unmatched contract risk settled',
    10_000,
  );

  // Select service node & keyboard navigate #system-map-side tabs
  const sysNode = await driver.callFn(() => {
    const host = document.getElementById('system-map-canvas');
    const cr = host.getBoundingClientRect();
    for (const n of host.querySelectorAll('.system-map-node, .graph-map-node')) {
      const nr = n.getBoundingClientRect();
      const cx = nr.left + nr.width / 2;
      const cy = nr.top + nr.height / 2;
      if (cx > cr.left + 8 && cx < cr.right - 8 && cy > cr.top + 8 && cy < cr.bottom - 8) {
        return { cx, cy };
      }
    }
    return null;
  });
  assert(sysNode, 'Expected clickable node inside #system-map-canvas');
  await driver.clickAt(sysNode.cx, sysNode.cy);
  const sideTabs = await driver.waitFor(() => {
    const tabs = Array.from(document.querySelectorAll('#system-map-side [role="tab"][data-action="system-map-side-tab"]'));
    return tabs.length > 0 ? tabs.length : null;
  }, '#system-map-side tabs rendered after node selection', 5_000);
  const sysSideMetrics = await driver.callFn(() => {
    const side = document.getElementById('system-map-side');
    return {
      width: side?.getBoundingClientRect().width || 0,
      scrollWidth: side?.scrollWidth || 0,
      clientWidth: side?.clientWidth || 0,
    };
  });
  assert(
    sysSideMetrics.width >= 260 && sysSideMetrics.scrollWidth <= sysSideMetrics.clientWidth + 2,
    `Expected unclipped #system-map-side tabs & >=260px width: ${JSON.stringify(sysSideMetrics)}`,
  );
  if (sideTabs >= 2) {
    await driver.callFn(() => {
      document.querySelector('#system-map-side [role="tab"][aria-selected="true"]')?.focus();
    });
    await driver.pressKey({ key: 'ArrowRight', code: 'ArrowRight', keyCode: 39 });
    await driver.waitFor(() => {
      const active = document.activeElement;
      const side = document.getElementById('system-map-side');
      return Boolean(side && active && side.contains(active) && active.getAttribute('role') === 'tab');
    }, '#system-map-side ArrowRight tab keyboard navigation', 4_000);
  }

  // Open System Changes view & test wave play/pause without DOM teardown
  await driver.click('#view-tabs [data-view="system-changes"]');
  await driver.waitFor((preferredId) => {
    const sel = document.getElementById('system-runs');
    if (!sel) return false;
    const opts = Array.from(sel.options).filter((o) => o.value);
    if (opts.length === 0) return false;
    const chosen = (preferredId && opts.some((o) => o.value === preferredId)) ? preferredId : opts[0].value;
    if (sel.value !== chosen) {
      sel.value = chosen;
      sel.dispatchEvent(new Event('change', { bubbles: true }));
    }
    return Boolean(document.querySelector('#system-change-topology-arrow-legend [data-action="system-map-wave-play"]'));
  }, 'System change topology wave play button ready', 12_000, opts.systemRunId || '');

  const flowCheck = await driver.waitFor(() => {
    const flow = document.getElementById('system-flow');
    if (!flow || flow.children.length === 0) return null;
    const children = Array.from(flow.children);
    return {
      count: children.length,
      keys: children.map((el) => el.getAttribute('data-key') || ''),
    };
  }, '#system-flow renders keyed step or "__empty__" placeholder', 6_000);
  assert(
    (flowCheck.count === 1 && flowCheck.keys[0] === '__empty__')
      || (!flowCheck.keys.includes('__empty__') && flowCheck.keys.every(Boolean)),
    `Expected #system-flow without keyless empty-state leak: ${JSON.stringify(flowCheck)}`,
  );

  const waveBefore = await driver.callFn(() => {
    window.__e2eWaveCanvas = document.getElementById('system-change-topology-canvas');
    window.__e2eWaveChild = window.__e2eWaveCanvas?.firstElementChild || null;
    window.__e2eWaveLegend = document.getElementById('system-change-topology-arrow-legend');
    return Boolean(window.__e2eWaveCanvas && window.__e2eWaveLegend);
  });
  assert(waveBefore, 'Expected #system-change-topology-canvas and legend before wave play');

  const playSel = '#system-change-topology-arrow-legend [data-action="system-map-wave-play"]';
  const pauseSel = '#system-change-topology-arrow-legend [data-action="system-map-wave-pause"][aria-pressed="true"]';
  await driver.click(playSel);
  await driver.waitFor(
    (sel) => Boolean(document.querySelector(sel)),
    'SystemMap wave playback switched to Pause control',
    5_000,
    pauseSel,
  );
  await driver.callFn(() => new Promise((r) => setTimeout(r, 250)));
  await driver.click(pauseSel);
  await driver.waitFor(
    (sel) => Boolean(document.querySelector(sel)),
    'SystemMap wave playback paused and restored Play control',
    5_000,
    playSel,
  );
  const waveAfter = await driver.callFn(() => {
    const canvas = document.getElementById('system-change-topology-canvas');
    const sameCanvas = canvas === window.__e2eWaveCanvas;
    const sameChild = !window.__e2eWaveChild || canvas?.firstElementChild === window.__e2eWaveChild;
    const sameLegend = document.getElementById('system-change-topology-arrow-legend') === window.__e2eWaveLegend;
    delete window.__e2eWaveCanvas;
    delete window.__e2eWaveChild;
    delete window.__e2eWaveLegend;
    return { sameCanvas, sameChild, sameLegend };
  });
  assert(
    waveAfter.sameCanvas && waveAfter.sameChild && waveAfter.sameLegend,
    `Expected wave play/pause without DOM teardown: ${JSON.stringify(waveAfter)}`,
  );

  // Cross-graph jumps: System Map -> Omnigraph -> System Map -> Dependency Map
  await driver.click('#view-tabs [data-view="systems"]');
  await driver.waitFor(() => {
    const c = document.getElementById('system-map-canvas');
    const nodes = c?.querySelectorAll('.system-map-node, .graph-map-node, canvas.graph-dense-canvas[data-dense-canvas="true"]').length || 0;
    return Boolean(c && !document.getElementById('system-catalog')?.hidden && c.getAttribute('aria-busy') !== 'true' && nodes > 0);
  }, 'restore System Map view before cross-graph jump check', 10_000);

  const openOmniBtn = await driver.waitFor(() => {
    const btn = document.querySelector('#system-map-side [data-action="system-map-open-omnigraph"]');
    if (btn) return true;
    const host = document.getElementById('system-map-canvas');
    const node = host?.querySelector('.system-map-node, .graph-map-node');
    if (node) node.click();
    return Boolean(document.querySelector('#system-map-side [data-action="system-map-open-omnigraph"]'));
  }, '#system-map-side Context graph jump button ready', 6_000);
  assert(openOmniBtn, 'Expected [data-action="system-map-open-omnigraph"] in #system-map-side');
  await driver.click('#system-map-side [data-action="system-map-open-omnigraph"]');
  await driver.waitFor(() => {
    const ov = document.getElementById('omnigraph-view');
    const studio = document.getElementById('omnigraph-studio');
    const canvas = document.getElementById('omnigraph-canvas');
    return Boolean(ov && !ov.hidden && studio && !studio.hidden && canvas?.getAttribute('aria-busy') !== 'true');
  }, 'System -> Omnigraph jump opens Context Graph studio', 10_000);
  // Allow any post-navigation highlight/search microtask from open_from_system_service to settle cleanly
  await driver.callFn(() => new Promise((r) => setTimeout(r, 300)));

  const backoffHitSel = '#omnigraph-search-hits button[data-action="omnigraph-focus-hit"][data-node-id="atom-finding-backoff"]';
  const searchBeforeBackoff = driver.responsesByUrl.length;
  await driver.callFn(() => {
    const tab = document.getElementById('omni-drawer-tab-inspector');
    if (tab && tab.getAttribute('aria-selected') !== 'true') tab.click();
    const input = document.getElementById('omnigraph-search');
    input.value = 'backoff';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await driver.waitUntil(
    () => driver.responsesByUrl.slice(searchBeforeBackoff).some((r) => r.status === 200 && r.url.includes('/api/omnigraph/search') && r.url.includes('backoff')),
    'Omnigraph /api/omnigraph/search for backoff completes',
    10_000,
  );
  await driver.waitFor(
    (sel) => Boolean(document.querySelector(sel)),
    'Omnigraph search hit ready for atom-finding-backoff',
    10_000,
    backoffHitSel,
  );
  await driver.click(backoffHitSel);
  await driver.waitFor(() => {
    const body = document.getElementById('omnigraph-inspector-body');
    return Boolean(
      body?.querySelector('[data-action="omnigraph-open-system-map"]')
        && body?.querySelector('[data-action="omnigraph-open-code-map"]'),
    );
  }, 'Omnigraph inspector displays Open in System Map and Open in Code Map jump controls', 8_000);

  await driver.click('#omnigraph-inspector-body [data-action="omnigraph-open-system-map"]');
  await driver.waitFor(() => {
    const sv = document.getElementById('systems-view');
    const cat = document.getElementById('system-catalog');
    return Boolean(sv && !sv.hidden && cat && !cat.hidden);
  }, 'Omnigraph -> System Map jump opens Systems workspace', 10_000);

  await driver.click('button.nav-item[data-nav="omnigraph"]');
  await driver.waitFor(() => {
    const ov = document.getElementById('omnigraph-view');
    const studio = document.getElementById('omnigraph-studio');
    return Boolean(ov && !ov.hidden && studio && !studio.hidden);
  }, 'return to Omnigraph studio before Code Map jump', 8_000);
  const alreadyHasCodeBtn = await driver.callFn(() => Boolean(document.querySelector('#omnigraph-inspector-body [data-action="omnigraph-open-code-map"]')));
  if (!alreadyHasCodeBtn) {
    const hasHit = await driver.callFn((sel) => Boolean(document.querySelector(sel)), backoffHitSel);
    if (!hasHit) {
      await driver.callFn(() => {
        const input = document.getElementById('omnigraph-search');
        if (input) {
          input.value = 'backoff';
          input.dispatchEvent(new Event('input', { bubbles: true }));
        }
      });
      await driver.waitFor((sel) => Boolean(document.querySelector(sel)), 're-focus backoff hit', 8_000, backoffHitSel);
    }
    await driver.click(backoffHitSel);
  }
  await driver.waitFor(
    () => Boolean(document.querySelector('#omnigraph-inspector-body [data-action="omnigraph-open-code-map"]')),
    'Omnigraph inspector has Open in Code Map button',
    8_000,
  );
  await driver.click('#omnigraph-inspector-body [data-action="omnigraph-open-code-map"]');
  await driver.waitFor(() => {
    const modalOpen = Boolean(document.getElementById('graph-map')?.hasAttribute('open'));
    const embeddedOpen = !document.getElementById('sessions-view')?.hidden && Boolean(document.getElementById('session-graph-canvas'));
    return modalOpen || embeddedOpen;
  }, 'Omnigraph -> Dependency Map jump opens code map', 10_000);
  const fullMapOpen = await driver.callFn(() => Boolean(document.getElementById('graph-map')?.hasAttribute('open')));
  if (fullMapOpen) {
    await driver.click('#graph-map-close');
    await driver.waitFor(() => !document.getElementById('graph-map')?.hasAttribute('open'), 'close #graph-map after Omnigraph jump', 5_000);
  }
  await driver.click('button.nav-item[data-nav="omnigraph"]');
  await driver.waitFor(() => !document.getElementById('omnigraph-view')?.hidden, 'restore omnigraph view before clearing search', 5_000);
  await driver.callFn(() => {
    const input = document.getElementById('omnigraph-search');
    if (input) {
      input.value = '';
      input.dispatchEvent(new Event('input', { bubbles: true }));
    }
  });

  driver.assertNoErrors('Suite 7');
}

async function suite8NegativeNetworkAndSseRecovery(driver, cdp) {
  console.log('[Suite 8] Negative 403 error handling & SSE disconnect/reconnect recovery');
  await driver.click('button.nav-item[data-nav="sessions"]');
  await driver.waitFor(
    () => document.getElementById('sse-status')?.getAttribute('data-state') === 'live',
    'SSE live before negative recovery tests',
    8_000,
  );

  let mode = 'passthrough';
  const unsubFetch = cdp.on('Fetch.requestPaused', async (params) => {
    try {
      const reqUrl = params.request?.url || '';
      if (mode === 'forbid_api' && reqUrl.startsWith(`${driver.origin}/api/`)) {
        const body = Buffer.from(JSON.stringify({ error: 'Forbidden fixture token check' })).toString('base64');
        await cdp.send('Fetch.fulfillRequest', {
          requestId: params.requestId,
          responseCode: 403,
          responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
          body,
        });
        return;
      }
      if (mode === 'drop_stream' && (reqUrl.includes('/api/events/stream') || reqUrl.includes('/api/health'))) {
        await cdp.send('Fetch.failRequest', {
          requestId: params.requestId,
          errorReason: 'ConnectionReset',
        });
        return;
      }
      await cdp.send('Fetch.continueRequest', { requestId: params.requestId });
    } catch {
      // ignore closed request/session races during teardown
    }
  });

  await driver.allowHttpErrors(async (scope) => {
    try {
      // Phase A: Inject HTTP 403 refusal -> verify #connection-banner displays error
      mode = 'forbid_api';
      await cdp.send('Fetch.enable', {
        patterns: [{ urlPattern: `${driver.origin}/api/*`, requestStage: 'Request' }],
      });
      await driver.click('#refresh-sessions');
      await driver.waitUntil(async () => {
        const ui = await driver.callFn(() => {
          const b = document.getElementById('connection-banner');
          const msg = document.getElementById('connection-message')?.textContent || '';
          return Boolean(b && !b.hidden && msg.trim().length > 0);
        });
        return ui && scope.httpErrors.some((e) => e.includes('HTTP 403'));
      }, '#connection-banner surfaced after 403 API refusal', 8_000);

      // Phase B: Attempt #reconnect while /api/events/stream + /api/health fail -> verify disconnected state, then recover cleanly
      mode = 'drop_stream';
      await cdp.send('Fetch.enable', {
        patterns: [
          { urlPattern: `${driver.origin}/api/events/stream*`, requestStage: 'Request' },
          { urlPattern: `${driver.origin}/api/health*`, requestStage: 'Request' },
        ],
      });
      await driver.click('#reconnect');
      await driver.waitFor(() => {
        const b = document.getElementById('connection-banner');
        const sse = document.getElementById('sse-status')?.getAttribute('data-state') || '';
        return Boolean(b && !b.hidden && sse !== 'live');
      }, 'SSE remains disconnected while stream/health requests fail', 8_000);

      mode = 'passthrough';
      await cdp.send('Fetch.disable');
      await driver.click('#reconnect');
      await driver.waitFor(() => {
        const b = document.getElementById('connection-banner');
        const sse = document.getElementById('sse-status')?.getAttribute('data-state') || '';
        return Boolean(b && b.hidden && sse === 'live');
      }, '#connection-banner hides and #sse-status returns to live after #reconnect', 12_000);
    } finally {
      mode = 'passthrough';
      unsubFetch();
      await cdp.send('Fetch.disable').catch(() => {});
    }
  });

  driver.assertNoErrors('Suite 8');
}

async function suite9DiffChecksAndResults(driver, cdp, opts) {
  console.log('[Suite 9] Right-pane Diff/Checks & #results-view (changes, checks, usage)');
  await driver.click('button.nav-item[data-nav="sessions"]');
  await driver.waitFor(() => !document.getElementById('sessions-view')?.hidden, 'sessions view open before Suite 9', 8_000);

  if (opts?.sessionId) {
    const mainSel = `#history [data-action="session-open"][data-payload="${JSON.stringify(opts.sessionId).replace(/"/g, '\\"')}"]`;
    const current = await driver.callFn((sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true', mainSel);
    if (!current) {
      await driver.click(mainSel);
      await driver.waitFor((sel) => document.querySelector(sel)?.getAttribute('aria-current') === 'true', 'select session_id for Suite 9', 8_000, mainSel);
    }
  }

  await driver.click('#right-tab-diff');
  await driver.waitFor(() => {
    const pane = document.getElementById('workspace-diff-pane');
    const tab = document.getElementById('right-tab-diff');
    return Boolean(pane && !pane.hidden && tab?.getAttribute('aria-selected') === 'true' && document.querySelectorAll('#workspace-diff-viewer .diff-file-group').length >= 1);
  }, '#right-tab-diff renders workspace diff', 10_000);

  const baseResults = await driver.waitUntil(
    async () => driver.readLatestFinishedJson('/results', (j) => Boolean(j?.snapshot)),
    'capture finished /results JSON payload',
    8_000,
  );
  assert(baseResults?.snapshot, 'Expected real /results JSON payload');
  const fillerLines = Array.from({ length: 265 }, (_, i) => `+export const line_${i} = ${i};`).join('\n');
  const multiFileDiff = [
    'diff --git a/entry_01.js b/entry_01.js',
    '--- a/entry_01.js',
    '+++ b/entry_01.js',
    '@@ -1,2 +1,267 @@',
    ' export const keep = 1;',
    fillerLines,
    'diff --git a/entry_00.js b/entry_00.js',
    '--- a/entry_00.js',
    '+++ b/entry_00.js',
    '@@ -1 +1,2 @@',
    ' export function runEntry() {}',
    '+// synthetic suite9 change',
  ].join('\n');
  const augmentedResults = {
    ...baseResults,
    snapshot: {
      ...baseResults.snapshot,
      available: true,
      files: [
        { path: 'entry_01.js', status: 'modified' },
        { path: 'entry_00.js', status: 'modified' },
      ],
      diff: multiFileDiff,
    },
  };

  await withFetchOverride(
    cdp,
    [{ urlPattern: `${driver.origin}/api/sessions/*/results*` }],
    async (params) => {
      if ((params.request?.method || 'GET').toUpperCase() === 'GET' && params.request.url.includes('/results')) {
        await cdp.send('Fetch.fulfillRequest', {
          requestId: params.requestId,
          responseCode: 200,
          responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
          body: Buffer.from(JSON.stringify(augmentedResults)).toString('base64'),
        });
        return true;
      }
      return false;
    },
    async () => {
      await driver.click('#workspace-refresh-diff');
      await driver.waitFor(() => {
        return document.querySelectorAll('#workspace-diff-viewer .diff-file-group').length === 2
          && Boolean(document.querySelector('#workspace-diff-viewer button.diff-show-more[data-action="diff-show-more"]'));
      }, '#workspace-diff-viewer renders 2 diff groups with Show all N lines truncation', 8_000);

      // Filter diff files by substring then clear
      await driver.callFn(() => {
        const filter = document.querySelector('#workspace-diff-viewer .diff-file-filter[data-action="diff-filter"]');
        filter.value = 'entry_00';
        filter.dispatchEvent(new Event('input', { bubbles: true }));
      });
      await driver.waitFor(() => {
        const g1 = document.querySelector('#workspace-diff-viewer .diff-file-group[data-diff-file="entry_01.js"]');
        const g0 = document.querySelector('#workspace-diff-viewer .diff-file-group[data-diff-file="entry_00.js"]');
        return Boolean(g1 && g0 && getComputedStyle(g1).display === 'none' && getComputedStyle(g0).display !== 'none');
      }, '.diff-file-filter hides non-matching file groups', 5_000);
      await driver.callFn(() => {
        const filter = document.querySelector('#workspace-diff-viewer .diff-file-filter[data-action="diff-filter"]');
        filter.value = '';
        filter.dispatchEvent(new Event('input', { bubbles: true }));
      });
      await driver.waitFor(() => {
        const g1 = document.querySelector('#workspace-diff-viewer .diff-file-group[data-diff-file="entry_01.js"]');
        return Boolean(g1 && getComputedStyle(g1).display !== 'none');
      }, 'clearing .diff-file-filter restores all diff groups', 4_000);

      // Copy path -> Copied, collapse all, expand all, show all lines
      const copyPathBtn = '#workspace-diff-viewer button[data-action="diff-copy-path"]';
      await driver.click(copyPathBtn);
      await driver.waitFor((sel) => document.querySelector(sel)?.textContent?.trim() === 'Copied', 'diff-copy-path transitions to Copied', 5_000, copyPathBtn);

      await driver.click('#workspace-diff-viewer button[data-action="diff-collapse-all"]');
      await driver.waitFor(() => {
        const groups = Array.from(document.querySelectorAll('#workspace-diff-viewer details.diff-file-group'));
        return groups.length === 2 && groups.every((g) => !g.open);
      }, 'diff-collapse-all collapses every diff file group', 4_000);
      await driver.click('#workspace-diff-viewer button[data-action="diff-expand-all"]');
      await driver.waitFor(() => {
        const groups = Array.from(document.querySelectorAll('#workspace-diff-viewer details.diff-file-group'));
        return groups.length === 2 && groups.every((g) => g.open);
      }, 'diff-expand-all expands every diff file group', 4_000);

      await driver.click('#workspace-diff-viewer button.diff-show-more[data-action="diff-show-more"]');
      await driver.waitFor(() => {
        const g1 = document.querySelector('#workspace-diff-viewer .diff-file-group[data-diff-file="entry_01.js"]');
        return Boolean(
          g1
            && !g1.querySelector('button.diff-show-more')
            && (g1.querySelector('.diff-file-lines')?.textContent || '').includes('line_264'),
        );
      }, 'diff-show-more expands all 270+ diff lines', 6_000);
    },
  );

  // Switch to #right-tab-checks and navigate via #workspace-open-checks to #results-view
  await driver.click('#right-tab-checks');
  await driver.waitFor(() => {
    const pane = document.getElementById('workspace-checks-pane');
    return Boolean(pane && !pane.hidden && document.getElementById('right-tab-checks')?.getAttribute('aria-selected') === 'true');
  }, '#right-tab-checks opens #workspace-checks-pane', 5_000);
  await driver.click('#workspace-open-checks');
  await driver.waitFor(() => {
    const rv = document.getElementById('results-view');
    return Boolean(rv && !rv.hidden && rv.getAttribute('data-tab') === 'checks' && document.getElementById('check-form'));
  }, '#workspace-open-checks opens #results-view checks tab', 8_000);

  await driver.click('#view-tabs [data-view="changes"]');
  await driver.waitFor(() => {
    const rv = document.getElementById('results-view');
    return Boolean(rv && rv.getAttribute('data-tab') === 'changes' && document.getElementById('results-files'));
  }, '#results-view changes tab renders #results-files & #results-diff', 8_000);

  await driver.click('#view-tabs [data-view="usage"]');
  await driver.waitFor(() => {
    const rv = document.getElementById('results-view');
    const totals = document.getElementById('results-totals');
    return Boolean(rv && rv.getAttribute('data-tab') === 'usage' && totals && totals.textContent.trim().length > 0);
  }, '#results-view usage tab renders #results-totals', 8_000);

  // Verify results stay preserved across sessions <-> usage switches when a session stays selected
  await driver.click('#view-tabs [data-view="sessions"]');
  await driver.waitFor(() => !document.getElementById('sessions-view')?.hidden, 'switch to #sessions-view while keeping session results', 6_000);
  await driver.click('#view-tabs [data-view="usage"]');
  await driver.waitFor(() => {
    const rv = document.getElementById('results-view');
    const totals = document.getElementById('results-totals');
    return Boolean(rv && !rv.hidden && rv.getAttribute('data-tab') === 'usage' && totals && totals.textContent.trim().length > 0);
  }, 'returning to #results-view usage preserves #results-totals without clearing results', 6_000);

  // Restore sessions conversation tab
  await driver.click('#view-tabs [data-view="sessions"]');
  await driver.waitFor(() => !document.getElementById('sessions-view')?.hidden, 'return to #sessions-view after Suite 9', 6_000);

  driver.assertNoErrors('Suite 9');
}

async function suite10KnowledgeSkillsManagementDialogs(driver, cdp, opts) {
  console.log('[Suite 10] Knowledge, Skills, Create Skill, Accelerators/Creator/Setup & Pickers');

  // 1. Knowledge tabs: memory-use (with retrieval strip DOM preservation + promotion review/apply) -> brain -> memory -> context
  let promotionStatus = 'proposed';
  await withFetchOverride(
    cdp,
    [
      { urlPattern: `${driver.origin}/api/projects/*/memory-use*` },
      { urlPattern: `${driver.origin}/api/projects/*/knowledge` },
    ],
    async (params) => {
      const reqUrl = params.request?.url || '';
      const method = (params.request?.method || 'GET').toUpperCase();
      if (method === 'GET' && reqUrl.includes('/memory-use')) {
        const memPayload = {
          project_id: opts.projectId,
          bank_id: 'default',
          banks: [{ id: 'default', name: 'default' }],
          today: '2026-11-22',
          check_available: false,
          brain: { items: [{ id: 'f-1', type: 'finding', status: 'open', created_at: '2026-11-20T00:00:00Z' }] },
          promotions: {
            items: [
              {
                id: 'prop-e2e-1',
                mode: 'human',
                status: promotionStatus,
                source_ids: ['f-1'],
                created_at: '2026-11-20T01:00:00Z',
              },
            ],
          },
          chunks: {
            bytes: 512,
            items: [
              {
                id: 'MEM-20261120-abcd1234-test',
                status: 'active',
                created: '2026-11-20',
                last_verified: '2026-11-21',
                review_after: '2026-12-20',
                auto: true,
              },
            ],
          },
          retrievals: {
            found: 2,
            limit: 50,
            items: [
              {
                at: '2026-11-22T09:00:00Z',
                route: 'context',
                brain: 2,
                bank: 1,
                rules: 1,
                chunks: ['MEM-20261120-abcd1234-test'],
                cuts: [],
              },
              {
                at: '2026-11-22T10:00:00Z',
                route: 'retrieve',
                brain: 1,
                bank: 2,
                rules: 1,
                chunks: ['MEM-20261120-abcd1234-test'],
                cuts: [['MEM-cut-1', 'budget']],
              },
            ],
          },
          history: { days: [] },
        };
        await cdp.send('Fetch.fulfillRequest', {
          requestId: params.requestId,
          responseCode: 200,
          responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
          body: Buffer.from(JSON.stringify(memPayload)).toString('base64'),
        });
        return true;
      }
      if (method === 'POST' && reqUrl.includes('/knowledge')) {
        const posted = JSON.parse(params.request?.postData || '{}');
        if (posted.action === 'promote-review') {
          promotionStatus = posted.reject ? 'rejected' : 'reviewed';
        } else if (posted.action === 'promote-apply') {
          promotionStatus = 'applied';
        }
        await cdp.send('Fetch.fulfillRequest', {
          requestId: params.requestId,
          responseCode: 200,
          responseHeaders: [{ name: 'Content-Type', value: 'application/json' }],
          body: Buffer.from(JSON.stringify({ ok: true })).toString('base64'),
        });
        return true;
      }
      return false;
    },
    async () => {
      await driver.click('button.nav-item[data-nav="knowledge"]');
      await driver.waitFor(() => {
        const view = document.getElementById('memory-use-view');
        const winBtns = document.querySelectorAll('#memory-use-window button').length;
        const cols = document.querySelectorAll('#memory-use-strip .memory-column').length;
        const sub = document.getElementById('page-subtitle')?.textContent || '';
        return Boolean(view && !view.hidden && winBtns >= 1 && cols === 2 && sub.trim().length > 0 && !sub.includes('Your local workspace'));
      }, 'Knowledge defaults to memory-use view with subtitle, strip columns, and #memory-use-window controls', 10_000);

      // Strip click updates aria-selected in place without replacing #memory-use-strip node
      await driver.callFn(() => {
        window.__e2eMemStripRef = document.getElementById('memory-use-strip');
      });
      await driver.click('#memory-use-strip .memory-column[data-position="0"]');
      await driver.waitFor(() => {
        const strip = document.getElementById('memory-use-strip');
        const col0 = document.querySelector('#memory-use-strip .memory-column[data-position="0"]');
        return Boolean(strip && strip === window.__e2eMemStripRef && col0?.getAttribute('aria-selected') === 'true');
      }, '#memory-use-strip column click preserves strip DOM node and updates aria-selected', 5_000);
      await driver.callFn(() => { delete window.__e2eMemStripRef; });

      // Promotion queue review -> apply
      await driver.callFn(() => {
        const rev = document.getElementById('memory-promotion-reviewer');
        rev.value = 'Alice';
        rev.dispatchEvent(new Event('input', { bubbles: true }));
      });
      await driver.click('#memory-use-promotions button[data-action="memory-promotion-review"]');
      await driver.waitFor(() => {
        const applyBtn = document.querySelector('#memory-use-promotions button[data-action="memory-promotion-apply"]');
        const status = document.querySelector('#memory-use-promotions .promotion-status')?.textContent || '';
        return Boolean(applyBtn && status.includes('Approved promotion prop-e2e-1'));
      }, 'memory-promotion-review approves proposal and offers Apply', 6_000);
      await driver.click('#memory-use-promotions button[data-action="memory-promotion-apply"]');
      await driver.waitFor(() => {
        const status = document.querySelector('#memory-use-promotions .promotion-status')?.textContent || '';
        return status.includes('Applied promotion prop-e2e-1');
      }, 'memory-promotion-apply completes proposal', 6_000);
    },
  );

  const memWinBtns = await driver.callFn(() => {
    const btns = Array.from(document.querySelectorAll('#memory-use-window button'));
    const alt = btns.find((b) => b.getAttribute('aria-checked') === 'false');
    if (alt) alt.click();
    return btns.length;
  });
  assert(memWinBtns >= 2, `Expected multiple #memory-use-window radio options, got ${memWinBtns}`);

  await driver.click('#view-tabs [data-view="brain"]');
  await driver.waitFor(() => {
    const v = document.getElementById('brain-view');
    return Boolean(v && !v.hidden && document.getElementById('brain-filter'));
  }, 'Knowledge brain tab visible', 8_000);

  await driver.click('#view-tabs [data-view="memory"]');
  await driver.waitFor(() => {
    const v = document.getElementById('memory-view');
    return Boolean(v && !v.hidden && document.getElementById('memory-filter'));
  }, 'Knowledge memory tab visible', 8_000);

  await driver.click('#view-tabs [data-view="context"]');
  await driver.waitFor(() => {
    const v = document.getElementById('context-view');
    const summary = document.getElementById('context-summary');
    return Boolean(v && !v.hidden && summary && summary.textContent.trim().length > 0);
  }, 'Knowledge context tab visible with #context-summary', 8_000);

  // 2. Skills & Create Skill form validation + byte counter + preview
  await driver.click('button.nav-item[data-nav="skills"]');
  await driver.waitFor(() => {
    const sv = document.getElementById('skills-view');
    const agentOpts = document.querySelectorAll('#skills-agents input[type="checkbox"]').length;
    const sub = document.getElementById('page-subtitle')?.textContent || '';
    return Boolean(sv && !sv.hidden && document.getElementById('skills-installed-agent') && agentOpts >= 1 && sub.trim().length > 0 && !sub.includes('Your local workspace'));
  }, 'Skills view loaded with subtitle and tool checkboxes', 10_000);

  await driver.click('#view-tabs [data-view="create-skill"]');
  await driver.waitFor(() => {
    const cv = document.getElementById('create-skill-view');
    const checks = document.querySelectorAll('#create-skill-agents input[type="checkbox"]').length;
    return Boolean(cv && !cv.hidden && checks >= 1);
  }, 'Create skill view rendered with tool targets', 8_000);

  await driver.callFn(() => {
    const name = document.getElementById('create-skill-name');
    const desc = document.getElementById('create-skill-description');
    const inst = document.getElementById('create-skill-instructions');
    const firstAgent = document.querySelector('#create-skill-agents input[type="checkbox"]');
    if (firstAgent && !firstAgent.checked) firstAgent.click();
    name.value = 'e2e-check-skill';
    name.dispatchEvent(new Event('input', { bubbles: true }));
    desc.value = 'Use during E2E skill preview checks.';
    desc.dispatchEvent(new Event('input', { bubbles: true }));
    inst.value = '## Steps\n1. Inspect diff.\n';
    inst.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await driver.waitFor(() => {
    const bytes = document.getElementById('create-skill-bytes')?.textContent || '';
    const previewBtn = document.getElementById('create-skill-preview-button');
    return bytes.startsWith('26 ') && previewBtn && !previewBtn.disabled;
  }, 'Create skill byte counter updates and enables Preview skill', 5_000);
  await driver.click('#create-skill-preview-button');
  await driver.waitFor(() => {
    const sec = document.getElementById('create-skill-preview');
    const content = document.getElementById('create-skill-content')?.textContent || '';
    return Boolean(sec && !sec.hidden && content.includes('name:') && content.includes('e2e-check-skill'));
  }, 'Create skill preview renders generated SKILL.md', 8_000);

  // 3. Accelerators, kit3 iframe stability, Creator (keyless empty state), and Setup readiness
  await driver.click('button.nav-item[data-nav="accelerators"]');
  await driver.waitFor(() => {
    const av = document.getElementById('accelerators-view');
    const box = document.getElementById('accelerators');
    const sub = document.getElementById('page-subtitle')?.textContent || '';
    return Boolean(av && !av.hidden && box && box.textContent.trim().length > 0 && sub.trim().length > 0 && !sub.includes('Your local workspace'));
  }, 'Accelerators overview renders catalog content with subtitle', 8_000);

  await driver.click('#view-tabs [data-view="kit3"]');
  await driver.waitFor(() => {
    const kv = document.getElementById('kit3-view');
    const frame = document.getElementById('kit3-frame');
    const openKit = document.getElementById('open-kit3');
    return Boolean(kv && !kv.hidden && openKit && !openKit.hidden && frame && (frame.getAttribute('src') || '').includes('/kit3') && frame.contentDocument?.readyState === 'complete');
  }, '#kit3-view loads /kit3/ iframe cleanly', 10_000);
  await driver.callFn(() => {
    document.getElementById('kit3-frame').contentWindow.__e2eKit3Alive = true;
  });
  await driver.click('#view-tabs [data-view="setup"]');
  await driver.waitFor(() => !document.getElementById('setup-view')?.hidden, 'switch from kit3 to setup view', 6_000);
  await driver.click('#view-tabs [data-view="kit3"]');
  await driver.waitFor(() => {
    const kv = document.getElementById('kit3-view');
    const frame = document.getElementById('kit3-frame');
    return Boolean(kv && !kv.hidden && frame?.contentWindow?.__e2eKit3Alive === true);
  }, 'returning to #kit3-view preserves existing iframe instance without reloading', 6_000);

  await driver.click('#view-tabs [data-view="creator"]');
  await driver.waitFor(() => {
    const cv = document.getElementById('creator-view');
    const reports = document.getElementById('creator-reports');
    return Boolean(cv && !cv.hidden && document.getElementById('creator-form') && reports);
  }, 'Creator view renders form and runs bar', 8_000);
  const creatorCheck = await driver.callFn(() => {
    const detail = document.getElementById('creator-detail');
    const reports = document.getElementById('creator-reports');
    const children = Array.from(reports?.children || []);
    return {
      detailHidden: Boolean(detail?.hidden),
      childCount: children.length,
      keys: children.map((el) => el.getAttribute('data-key') || ''),
    };
  });
  if (!creatorCheck.detailHidden && creatorCheck.childCount > 0) {
    const hasEmpty = creatorCheck.keys.includes('__empty__');
    const nonEmptyOk = creatorCheck.keys.every((k) => k && k !== '__empty__');
    assert(
      (creatorCheck.childCount === 1 && hasEmpty) || (!hasEmpty && nonEmptyOk),
      `Expected #creator-reports to avoid keyless empty-state leak: ${JSON.stringify(creatorCheck)}`,
    );
  }

  await driver.click('#view-tabs [data-view="setup"]');
  await driver.waitFor(() => {
    const sv = document.getElementById('setup-view');
    const readiness = document.getElementById('setup-readiness');
    const cards = document.querySelectorAll('#setup-readiness-grid > *').length;
    return Boolean(sv && !sv.hidden && readiness && !readiness.hidden && cards >= 1);
  }, 'Setup view renders #setup-readiness grid', 10_000);

  // 4. Dialogs: #folder-picker and #merge-picker (open, interact, backdrop click close)
  await driver.click('button.nav-item[data-nav="sessions"]');
  await driver.waitFor(() => !document.getElementById('sessions-view')?.hidden, 'return to #sessions-view before picker checks', 6_000);
  await driver.click('#new-session');
  await driver.waitFor(() => {
    const panel = document.getElementById('panel-project');
    const chip = document.getElementById('option-project');
    return Boolean(chip && panel);
  }, 'New session configuration visible', 5_000);
  const projectPanelHidden = await driver.callFn(() => Boolean(document.getElementById('panel-project')?.hidden));
  if (projectPanelHidden) {
    await driver.click('#option-project');
    await driver.waitFor(() => !document.getElementById('panel-project')?.hidden, '#panel-project opens', 4_000);
  }
  await driver.click('#project-choose-folder');
  await driver.waitFor(() => {
    const dlg = document.getElementById('folder-picker');
    return Boolean(dlg && dlg.hasAttribute('open') && document.querySelectorAll('#folder-picker-list button').length >= 1);
  }, '#folder-picker dialog opens and lists directories', 8_000);
  await closeDialogByBackdropClick(driver, '#folder-picker', '#folder-picker');

  await driver.click('#merge-chats');
  await driver.waitFor(() => {
    const dlg = document.getElementById('merge-picker');
    const checks = document.querySelectorAll('#merge-options input[type="checkbox"]').length;
    return Boolean(dlg && dlg.hasAttribute('open') && checks >= 2);
  }, '#merge-picker opens with >= 2 finished session checkboxes', 8_000);
  await driver.callFn(() => {
    const checks = Array.from(document.querySelectorAll('#merge-options input[type="checkbox"]')).slice(0, 2);
    checks.forEach((cb) => {
      if (!cb.checked) cb.click();
    });
  });
  await driver.waitFor(() => {
    const btn = document.getElementById('merge-picker-continue');
    const cnt = document.getElementById('merge-selection-count')?.textContent || '';
    return Boolean(btn && !btn.disabled && cnt.includes('2'));
  }, '#merge-picker enables continue button with 2 selected sessions', 4_000);
  await closeDialogByBackdropClick(driver, '#merge-picker', '#merge-picker');

  // 5. Command Palette "palette-skill" deep-link populates #skills-filter and navigates cleanly to #skills-view
  await driver.pressKey({ key: 'k', code: 'KeyK', keyCode: 75, modifiers: 2 });
  await driver.waitFor(() => !document.getElementById('command-palette')?.hidden, '#command-palette opens for palette-skill check', 4_000);
  await driver.callFn(() => {
    const input = document.getElementById('command-palette-input');
    input.value = 'pdf';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const skillItemIdx = await driver.waitFor(() => {
    const items = Array.from(document.querySelectorAll('#command-palette-list .command-palette-item'));
    const idx = items.findIndex((el) => {
      const cat = el.querySelector('.command-palette-cat')?.textContent?.trim() || '';
      const title = el.querySelector('.command-palette-title')?.textContent?.toLowerCase() || '';
      return cat === 'Skill' && title.includes('pdf');
    });
    return idx >= 0 ? idx + 1 : null;
  }, 'Command Palette surfaces Skill entry for "pdf"', 5_000);
  await driver.click(`#command-palette-option-${skillItemIdx - 1}`);
  await driver.waitFor(() => {
    const pal = document.getElementById('command-palette');
    const sv = document.getElementById('skills-view');
    const filterVal = document.getElementById('skills-filter')?.value || '';
    const installedTxt = document.getElementById('skills-installed-list')?.textContent?.toLowerCase() || '';
    return Boolean(pal?.hidden && sv && !sv.hidden && filterVal.toLowerCase() === 'pdf' && installedTxt.includes('pdf'));
  }, 'palette-skill navigates to #skills-view, sets #skills-filter="pdf", and displays matching skill', 8_000);
  await driver.click('button.nav-item[data-nav="sessions"]');
  await driver.waitFor(() => !document.getElementById('sessions-view')?.hidden, 'return to #sessions-view after palette-skill check', 6_000);

  driver.assertNoErrors('Suite 10');
}

async function suite11ColdBootMobileAndReducedMotion(browserCdp, browserWsUrl, opts) {
  console.log('[Suite 11] Cold-boot 390x844 mobile layout + prefers-reduced-motion: reduce');
  const { targetId } = await browserCdp.send('Target.createTarget', { url: 'about:blank' });
  const port = new URL(browserWsUrl).port;
  let mobileTarget = null;
  for (let i = 0; i < 40; i += 1) {
    const res = await fetch(`http://127.0.0.1:${port}/json/list`);
    if (res.ok) {
      const targets = await res.json();
      mobileTarget = targets.find((t) => t.id === targetId && t.webSocketDebuggerUrl);
      if (mobileTarget) break;
    }
    await new Promise((r) => setTimeout(r, 50));
  }
  assert(mobileTarget?.webSocketDebuggerUrl, 'Could not find fresh CDP page target for Suite 11');

  const mobileCdp = await CdpClient.connect(mobileTarget.webSocketDebuggerUrl, opts.timeoutMs);
  try {
    await mobileCdp.send('Runtime.enable');
    await mobileCdp.send('Page.enable');
    await mobileCdp.send('DOM.enable');
    await mobileCdp.send('Network.enable');
    await mobileCdp.send('Emulation.setDeviceMetricsOverride', {
      width: 390,
      height: 844,
      deviceScaleFactor: 2,
      mobile: true,
    });
    await mobileCdp.send('Emulation.setEmulatedMedia', {
      features: [{ name: 'prefers-reduced-motion', value: 'reduce' }],
    });

    const mobileDriver = createHarnessDriver(mobileCdp, opts.url, opts.timeoutMs);
    await mobileCdp.send('Page.navigate', { url: opts.url });

    const boot = await mobileDriver.waitFor(() => {
      const connecting = document.getElementById('connecting');
      const sse = document.getElementById('sse-status')?.getAttribute('data-state');
      const sb = document.getElementById('sidebar');
      const main = document.getElementById('main');
      const menuBtn = document.getElementById('menu-toggle');
      const tabs = document.getElementById('session-mobile-tabs');
      if (!connecting?.hidden || sse !== 'live' || !sb || !main || !menuBtn || !tabs) return null;
      const working = document.getElementById('composer-working');
      const runView = document.getElementById('run-view');
      const csWork = working ? getComputedStyle(working) : null;
      const csRun = runView ? getComputedStyle(runView) : null;
      return {
        innerWidth: window.innerWidth,
        scrollWidth: document.documentElement.scrollWidth,
        sidebarOpen: sb.classList.contains('open'),
        sidebarInert: sb.hasAttribute('inert'),
        mainInert: main.hasAttribute('inert'),
        menuDisplay: getComputedStyle(menuBtn).display,
        tabsDisplay: getComputedStyle(tabs).display,
        reducedMotionMatch: window.matchMedia('(prefers-reduced-motion: reduce)').matches,
        workAnim: csWork?.animationName || '',
        runTrans: csRun?.transitionDuration || '',
      };
    }, 'cold-boot mobile 390x844 WASM ready with reduced motion', 30_000);

    assert(boot.innerWidth === 390, `Expected cold-boot mobile width 390, got ${boot.innerWidth}`);
    assert(boot.scrollWidth <= boot.innerWidth + 1, `Cold-boot horizontal overflow at 390x844: ${JSON.stringify(boot)}`);
    assert(!boot.sidebarOpen && boot.sidebarInert && !boot.mainInert, `Unexpected mobile cold-boot drawer/inert state: ${JSON.stringify(boot)}`);
    assert(boot.menuDisplay !== 'none' && boot.tabsDisplay !== 'none', `Expected #menu-toggle and #session-mobile-tabs visible at 390x844: ${JSON.stringify(boot)}`);
    assert(boot.reducedMotionMatch, 'Expected matchMedia("(prefers-reduced-motion: reduce)").matches === true');
    assert(boot.workAnim === 'none' && (boot.runTrans === '0s' || boot.runTrans === '0ms'), `Expected reduced-motion animation/transition disabled: ${JSON.stringify(boot)}`);

    mobileDriver.assertNoErrors('Suite 11');
  } finally {
    mobileCdp.close();
    await browserCdp.send('Target.closeTarget', { targetId }).catch(() => {});
  }
}

async function suite12SkillCache(driver, opts) {
  console.log('[Suite 12] SkillCache: composer option, stream cards, run tab & Skill capsules view');
  assert(opts.skillcacheCapsuleId && opts.otherSessionId && opts.projectId, 'Suite 12 needs the seeded capsule, the finished session and the project');
  const capsuleId = opts.skillcacheCapsuleId;
  const setMode = (mode) => driver.callFn((value) => {
    const select = document.getElementById('skillcache-mode');
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  }, mode);

  // 1. The composer's option: off until chosen, named on the chip, remembered with the draft.
  await driver.click('button.nav-item[data-nav="sessions"]');
  await driver.waitFor(() => !document.getElementById('sessions-view')?.hidden, 'sessions view open before Suite 12', 8_000);
  await driver.click('#new-session');
  await driver.waitFor(() => {
    const chip = document.getElementById('option-skillcache');
    return Boolean(chip && chip.getBoundingClientRect().width > 0
      && document.getElementById('option-skillcache-value')?.textContent === 'Off');
  }, '#option-skillcache chip visible and Off in a new draft', 6_000);
  await driver.click('#option-skillcache');
  await driver.waitFor(() => {
    const panel = document.getElementById('panel-skillcache');
    const chip = document.getElementById('option-skillcache');
    const note = document.getElementById('skillcache-note')?.textContent || '';
    return Boolean(panel && !panel.hidden && chip?.getAttribute('aria-expanded') === 'true' && note.startsWith('Off.'));
  }, '#panel-skillcache opens and says nothing is recorded', 5_000);
  const modelDisabledWhileOff = await driver.callFn(() => document.getElementById('skillcache-compiler-model')?.disabled === true);
  assert(modelDisabledWhileOff, 'The compiler model has no meaning while SkillCache is off');

  await setMode('observe');
  await driver.waitFor(() => {
    const note = document.getElementById('skillcache-note')?.textContent || '';
    return document.getElementById('option-skillcache-value')?.textContent === 'Observe'
      && note.includes('redacted trace') && note.includes('30 days')
      && document.getElementById('skillcache-compiler-model')?.disabled === false;
  }, 'Observe names what is kept and for how long', 5_000);
  const remembered = await driver.callFn(() => {
    const tab = sessionStorage.getItem('harness.sessions.tab-drafts.v1') || '';
    const shared = localStorage.getItem('harness.sessions.preferences.v1') || '';
    return tab.includes('"skillcache-mode":"observe"') || shared.includes('"skillcache-mode":"observe"');
  });
  assert(remembered, 'The chosen SkillCache mode is saved with the draft');

  await setMode('auto');
  await driver.waitFor(() => {
    const note = document.getElementById('skillcache-note')?.textContent || '';
    // Claude runs a capsule for `$skill`; a `/command` stays native and is only observed.
    return document.getElementById('option-skillcache-value')?.textContent === 'Auto'
      && note.includes('$skill') && note.includes('`/command` runs natively');
  }, 'Auto says how a Claude session runs a capsule and keeps /command native', 5_000);
  await setMode('observe');
  await driver.click('#option-skillcache');
  await driver.waitFor(() => document.getElementById('panel-skillcache')?.hidden === true, '#panel-skillcache closes', 4_000);

  // 2. The finished session: its stored option is its own, and its turn's SkillCache cards are in the conversation.
  const otherSel = `#history [data-action="session-open"][data-payload="${JSON.stringify(opts.otherSessionId).replace(/"/g, '\\"')}"]`;
  await driver.click(otherSel);
  await driver.waitFor(() => {
    const phases = Array.from(document.querySelectorAll('#events .conversation-skillcache')).map((e) => e.dataset.phase);
    return ['observed', 'hit', 'program_failed', 'complete'].every((phase) => phases.includes(phase));
  }, 'SkillCache cards of the finished turn are in the conversation', 12_000);
  // The session's own stored option replaces the draft's on the chip.
  await driver.waitFor(() => document.getElementById('option-skillcache-value')?.textContent === 'Off'
    && document.getElementById('skillcache-mode')?.value === 'off',
  'A session that never opted in shows SkillCache Off', 8_000);
  const cards = await driver.callFn(() => Array.from(document.querySelectorAll('#events .conversation-skillcache')).map((e) => ({
    phase: e.dataset.phase,
    tone: e.dataset.tone,
    text: e.textContent || '',
    border: getComputedStyle(e).borderLeftColor,
  })));
  const failedCard = cards.find((card) => card.phase === 'program_failed');
  assert(failedCard.tone === 'warn' && failedCard.text.includes('Step handed back') && failedCard.text.includes('nonzero_exit'),
    `program_failed card names the handed-back step and its reason: ${JSON.stringify(failedCard)}`);
  assert(!failedCard.text.includes('main.sh') && !failedCard.text.includes('printf'), 'A card never shows program code');
  assert(!cards.some((card) => card.phase === 'started'), 'The conversation shows what a person may act on, not every phase');
  const hitCard = cards.find((card) => card.phase === 'hit');
  assert(hitCard.border !== failedCard.border, `Tones are told apart by token colors (${hitCard.border} vs ${failedCard.border})`);
  // The observed turn was compiled already: its card says so instead of offering to compile it again.
  const observedCard = cards.find((card) => card.phase === 'observed');
  await driver.waitFor(() => {
    const card = document.querySelector('#events .conversation-skillcache[data-phase="observed"]');
    return Boolean(card && card.textContent.includes('Compilation completed') && !card.querySelector('[data-action="skillcache-compile"]'));
  }, `observed card shows its compilation once the session's state is read (${observedCard.text.slice(0, 80)})`, 8_000);

  // 3. The run view's SkillCache tab lists every phase of the turn.
  const openTab = '.run-receipt [data-action="run-open"][data-payload="\\"skillcache\\""]';
  await driver.waitFor((sel) => Boolean(document.querySelector(sel)), 'Last run line offers the SkillCache tab', 8_000, openTab);
  await driver.click(openTab);
  await driver.waitFor(() => {
    const tab = document.getElementById('run-tab-skillcache');
    const pane = document.getElementById('run-pane-skillcache');
    return Boolean(tab && !tab.hidden && tab.getAttribute('aria-selected') === 'true' && pane && !pane.hidden
      && pane.querySelectorAll('.skillcache-phase').length >= 5);
  }, '#run-pane-skillcache lists the phases', 8_000);
  const pane = await driver.callFn(() => {
    const failed = document.querySelector('#run-pane-skillcache .skillcache-phase[data-phase="program_failed"]');
    return {
      phases: Array.from(document.querySelectorAll('#run-pane-skillcache .skillcache-phase')).map((e) => e.dataset.phase),
      failed: failed?.textContent || '',
      tone: failed?.dataset.tone,
      opens: document.querySelectorAll('#run-pane-skillcache [data-action="skillcache-open-capsule"]').length,
    };
  });
  assert(pane.phases.includes('started') && pane.phases.includes('complete'), `The tab lists phases the conversation leaves out: ${pane.phases}`);
  assert(pane.tone === 'warn' && pane.failed.includes('reason: nonzero_exit') && pane.failed.includes('step 1') && pane.failed.includes('program extract'),
    `program_failed row names reason, step and program: ${pane.failed}`);
  assert(pane.opens >= 1, 'The tab links to the capsule the turn used');

  // 4. From the turn to its capsule: the Skill capsules view with the capsule open.
  // Activated in the page, not by pointer coordinates: in the detailed run panel the pane scrolls
  // under the sticky composer, so the centered button may sit beneath it.
  await driver.callFn(() => document.querySelector('#run-pane-skillcache [data-action="skillcache-open-capsule"]').click());
  await driver.waitFor(() => {
    const view = document.getElementById('skill-capsules-view');
    const detail = document.getElementById('skill-capsule-detail');
    const tab = document.querySelector('#view-tabs [data-view="skill-capsules"]');
    return Boolean(view && !view.hidden && tab?.getAttribute('aria-selected') === 'true'
      && detail && !detail.hidden && detail.textContent.includes('Plan (π)'));
  }, 'Skill capsules view opens with the capsule inspected', 10_000);
  const inspected = await driver.callFn(() => {
    const detail = document.getElementById('skill-capsule-detail');
    const row = document.querySelector('#skill-capsules-list .skill-capsule-row');
    return {
      text: detail.textContent || '',
      rows: document.querySelectorAll('#skill-capsules-list .skill-capsule-row').length,
      current: row?.getAttribute('aria-current'),
      rowText: row?.textContent || '',
      actions: Array.from(detail.querySelectorAll('.skill-capsule-actions button')).map((b) => b.textContent),
      inlineColors: detail.innerHTML.match(/style="[^"]*#[0-9a-fA-F]{3,8}/g),
    };
  });
  assert(inspected.rows === 1 && inspected.current === 'true', 'The open capsule is the current row');
  assert(inspected.rowText.includes('pdf-rows') && inspected.rowText.includes('Needs recompile') && inspected.rowText.includes('handed a program step back'),
    `The row names the capsule, its state and why: ${inspected.rowText}`);
  for (const part of ['Values it needs from the request (B)', 'Programs (P)', 'Plan (π)', 'Verification (V)', 'Gate history', 'Round 1 · gate1 failed', 'output_unread', 'Qualified as a hybrid capsule in round 2.']) {
    assert(inspected.text.includes(part), `The inspector shows "${part}"`);
  }
  // What compiling cost is known; what a reuse saves is not, and is not a zero.
  assert(inspected.text.includes('Compilation cost$0.0512'), `Compilation cost is shown: ${inspected.text.slice(0, 200)}`);
  assert(inspected.text.includes('Saving per reuse—') && inspected.text.includes('Break-even—'), 'An unknown saving is a dash, not a zero');
  assert(!inspected.text.includes('printf'), 'The inspector lists package files and never their contents');
  assert(JSON.stringify(inspected.actions) === JSON.stringify(['Requalify', 'Invalidate', 'Stop using', 'Delete…']), `Actions follow the status: ${inspected.actions}`);
  assert(!inspected.inlineColors, `No inline colors in the inspector: ${inspected.inlineColors}`);

  // Verify Command Palette indexes live SkillCache capsules when loaded
  await driver.pressKey({ key: 'k', code: 'KeyK', keyCode: 75, modifiers: 2 });
  await driver.waitFor(() => !document.getElementById('command-palette')?.hidden, '#command-palette opens in Suite 12', 4_000);
  await driver.callFn(() => {
    const input = document.getElementById('command-palette-input');
    input.value = 'pdf-rows';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await driver.waitFor(() => {
    const items = Array.from(document.querySelectorAll('#command-palette-list .command-palette-item'));
    return items.some((el) => (el.textContent || '').includes('Capsule') && (el.textContent || '').includes('pdf-rows'));
  }, 'Command Palette surfaces loaded SkillCache capsule "pdf-rows"', 5_000);
  await driver.pressKey({ key: 'Escape', code: 'Escape', keyCode: 27 });
  await driver.waitFor(() => document.getElementById('command-palette')?.hidden, '#command-palette closes after Suite 12 capsule check', 4_000);

  // 5. Themes: the status color is a token, so it changes with the theme.
  const warnColor = () => driver.callFn(() => getComputedStyle(document.querySelector('#skill-capsules-list .skill-capsule-chip[data-tone="warn"]')).color);
  const initialTheme = await driver.callFn(() => document.documentElement.getAttribute('data-theme'));
  const otherTheme = initialTheme === 'dark' ? 'light' : 'dark';
  const colorBefore = await warnColor();
  await driver.click(`#theme-${otherTheme}`);
  await driver.waitFor((expected) => document.documentElement.getAttribute('data-theme') === expected, `theme flips to ${otherTheme}`, 5_000, otherTheme);
  const colorAfter = await warnColor();
  assert(colorBefore !== colorAfter, `Status chip color follows the theme (${colorBefore} vs ${colorAfter})`);
  await driver.click(`#theme-${initialTheme}`);
  await driver.waitFor((expected) => document.documentElement.getAttribute('data-theme') === expected, `theme restores to ${initialTheme}`, 5_000, initialTheme);

  // 6. A changing request without the page's token is refused.
  let refused = 0;
  await driver.allowHttpErrors(async () => {
    refused = await driver.callFn(async (path) => {
      const response = await fetch(path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' });
      return response.status;
    }, `/api/projects/${opts.projectId}/skillcache/capsules/${capsuleId}/retire`);
  });
  assert(refused === 403, `A capsule change without the token is refused with 403, got ${refused}`);
  await driver.click('#skill-capsules-refresh');
  await driver.waitFor(() => (document.querySelector('#skill-capsules-list .skill-capsule-row')?.textContent || '').includes('Needs recompile'),
    'The refused request changed nothing', 6_000);

  // 7. Filters, Stop using (retire) -> Use again (requalify), and two-click delete.
  await driver.callFn(() => {
    const select = document.getElementById('skill-capsules-status');
    select.value = 'active';
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await driver.waitFor(() => document.querySelectorAll('#skill-capsules-list .skill-capsule-row').length === 0, 'Status filter narrows the list', 6_000);
  await driver.callFn(() => {
    const select = document.getElementById('skill-capsules-status');
    select.value = '';
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await driver.waitFor(() => document.querySelectorAll('#skill-capsules-list .skill-capsule-row').length === 1, 'Clearing the filter restores the list', 6_000);

  // Stop using -> retired (kept visible in All via include_retired=true), then Use again -> active.
  await driver.click('#skill-capsule-detail [data-action="skill-capsule-retire"]');
  await driver.waitFor(() => {
    const row = document.querySelector('#skill-capsules-list .skill-capsule-row');
    const detail = document.getElementById('skill-capsule-detail');
    const actions = Array.from(detail?.querySelectorAll('.skill-capsule-actions button') || []).map((b) => b.textContent);
    return Boolean(row && (row.textContent || '').includes('Retired')
      && detail && !detail.hidden && (detail.textContent || '').includes('Retired')
      && JSON.stringify(actions) === JSON.stringify(['Use again', 'Delete…']));
  }, 'Stop using retires the capsule and offers Use again', 8_000);
  await driver.callFn(() => {
    const select = document.getElementById('skill-capsules-status');
    select.value = 'retired';
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await driver.waitFor(() => document.querySelectorAll('#skill-capsules-list .skill-capsule-row').length === 1, 'Status=retired filter shows the retired capsule', 6_000);
  await driver.callFn(() => {
    const select = document.getElementById('skill-capsules-status');
    select.value = '';
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await driver.waitFor(() => document.querySelectorAll('#skill-capsules-list .skill-capsule-row').length === 1, 'All filter keeps retired capsule visible', 6_000);
  if (opts.skillcacheRequalify) {
    await driver.click('#skill-capsule-detail [data-action="skill-capsule-requalify"]');
    await driver.waitFor(() => {
      const row = document.querySelector('#skill-capsules-list .skill-capsule-row');
      const detail = document.getElementById('skill-capsule-detail');
      const actions = Array.from(detail?.querySelectorAll('.skill-capsule-actions button') || []).map((b) => b.textContent);
      return Boolean(row && (row.textContent || '').includes('Active')
        && detail && !detail.hidden
        && JSON.stringify(actions) === JSON.stringify(['Requalify', 'Invalidate', 'Stop using', 'Delete…']));
    }, 'Use again requalifies the retired capsule back to Active', 20_000);
  }
  const armSel = '#skill-capsule-detail [data-action="skill-capsule-arm"]';
  await driver.waitFor((sel) => Boolean(document.querySelector(sel)), 'Delete… is offered', 6_000, armSel);
  await driver.click(armSel);
  await driver.waitFor(() => Boolean(document.querySelector('#skill-capsule-detail [data-action="skill-capsule-delete"]')
    && document.querySelector('#skill-capsule-detail [data-action="skill-capsule-keep"]')), 'Delete asks to be confirmed', 4_000);
  await driver.click('#skill-capsule-detail [data-action="skill-capsule-keep"]');
  await driver.waitFor((sel) => Boolean(document.querySelector(sel)) && !document.querySelector('#skill-capsule-detail [data-action="skill-capsule-delete"]'),
    'Keep it takes the delete back', 4_000, armSel);
  await driver.click(armSel);
  await driver.waitFor(() => Boolean(document.querySelector('#skill-capsule-detail [data-action="skill-capsule-delete"]')), 'Delete armed again', 4_000);
  await driver.click('#skill-capsule-detail [data-action="skill-capsule-delete"]');
  await driver.waitFor(() => {
    const list = document.getElementById('skill-capsules-list');
    const detail = document.getElementById('skill-capsule-detail');
    return Boolean(list && list.querySelectorAll('.skill-capsule-row').length === 0
      && (list.textContent || '').includes('No capsule yet') && detail?.hidden);
  }, 'The deleted capsule leaves the list and the inspector', 10_000);

  // 8. The draft kept its choice while another session and another view were open.
  await driver.click('button.nav-item[data-nav="sessions"]');
  await driver.waitFor(() => !document.getElementById('sessions-view')?.hidden, 'return to #sessions-view after Suite 12', 6_000);
  await driver.click('#new-session');
  await driver.waitFor(() => document.getElementById('option-skillcache-value')?.textContent === 'Observe',
    'A new draft restores the remembered SkillCache mode', 6_000);
  await setMode('off');
  await driver.waitFor(() => document.getElementById('option-skillcache-value')?.textContent === 'Off', 'SkillCache is left Off', 4_000);

  driver.assertNoErrors('Suite 12');
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const { browserWsUrl, pageWsUrl, cleanup } = await launchChrome(opts.chrome, opts.timeoutMs);
  let browserCdp = null;
  let pageCdp = null;
  let overallTimer = null;

  try {
    const timeoutPromise = new Promise((_, reject) => {
      overallTimer = setTimeout(() => {
        reject(new Error(`Browser E2E timed out after ${opts.timeoutMs}ms`));
      }, opts.timeoutMs);
    });

    const runSuites = (async () => {
      const origin = new URL(opts.url).origin;
      browserCdp = await CdpClient.connect(browserWsUrl, opts.timeoutMs);
      await browserCdp.send('Browser.grantPermissions', {
        origin,
        permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite'],
      });

      pageCdp = await CdpClient.connect(pageWsUrl, opts.timeoutMs);
      await pageCdp.send('Runtime.enable');
      await pageCdp.send('Page.enable');
      await pageCdp.send('DOM.enable');
      await pageCdp.send('Network.enable');
      await pageCdp.send('Page.bringToFront');
      await pageCdp.send('Emulation.setDeviceMetricsOverride', {
        width: 1440,
        height: 900,
        deviceScaleFactor: 1,
        mobile: false,
      });

      const driver = createHarnessDriver(pageCdp, opts.url, opts.timeoutMs);
      await pageCdp.send('Page.navigate', { url: opts.url });
      await pageCdp.send('Page.bringToFront');

      await suite1BootThemeTransport(driver, pageCdp, opts);
      await suite2Canvas2dAndMapLayout(driver, pageCdp, opts);
      await suite3MapLenses(driver);
      await suite4LiveSseAndStyling(driver, pageCdp, opts);
      await suite5ModalsApprovalsPaletteMobile(driver, pageCdp, opts);
      await suite6OmnigraphStudio(driver, pageCdp);
      await suite7SystemMap(driver, opts);
      await suite9DiffChecksAndResults(driver, pageCdp, opts);
      await suite10KnowledgeSkillsManagementDialogs(driver, pageCdp, opts);
      await suite12SkillCache(driver, opts);

      driver.assertNoErrors('Suites 1-7, 9-10 & 12 check');
      await suite8NegativeNetworkAndSseRecovery(driver, pageCdp);
      await suite11ColdBootMobileAndReducedMotion(browserCdp, browserWsUrl, opts);

      driver.assertNoErrors('final check');
      console.log('ALL 12 HEADLESS CHROME CDP E2E SUITES PASSED');
    })();

    await Promise.race([runSuites, timeoutPromise]);
  } finally {
    if (overallTimer) clearTimeout(overallTimer);
    if (browserCdp) {
      try {
        await browserCdp.send('Browser.close', {}, 2000);
      } catch {
        // ignore if already closed
      }
    }
    if (pageCdp) pageCdp.close();
    if (browserCdp) browserCdp.close();
    await cleanup();
  }
}

main().catch((err) => {
  console.error(err?.stack || String(err));
  process.exit(1);
});
