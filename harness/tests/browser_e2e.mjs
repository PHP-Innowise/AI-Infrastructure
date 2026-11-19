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

function parseArgs(argv) {
  const opts = {
    url: '',
    projectId: '',
    sessionId: '',
    otherSessionId: '',
    commitA: '',
    commitB: '',
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
    consoleErrors.push(text);
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
      httpErrors.push(`HTTP ${r.status} ${url}`);
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
    loadingErrors.push(`${params.errorText || 'load failed'} (${req?.url || params.requestId})`);
  });

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
      const el = document.querySelector(sel);
      if (!el) return null;
      const sameRect = (a, b) => a && b
        && Math.abs(a.left - b.left) < 0.5
        && Math.abs(a.top - b.top) < 0.5
        && Math.abs(a.width - b.width) < 0.5
        && Math.abs(a.height - b.height) < 0.5;
      const deadline = performance.now() + 2500;
      let prev = null;
      while (performance.now() < deadline) {
        el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
        await new Promise((r) => requestAnimationFrame(r));
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
      const r = el.getBoundingClientRect();
      return { x: r.left, y: r.top, w: r.width, h: r.height, cx: r.left + r.width / 2, cy: r.top + r.height / 2 };
    }, selector);
    assert(box && box.w > 0 && box.h > 0, `Element missing or zero-sized: ${selector}`);
    return box;
  }

  async function clickAt(x, y, { modifiers = 0 } = {}) {
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, modifiers });
    await cdp.send('Input.dispatchMouseEvent', {
      type: 'mousePressed',
      x,
      y,
      button: 'left',
      clickCount: 1,
      modifiers,
    });
    await cdp.send('Input.dispatchMouseEvent', {
      type: 'mouseReleased',
      x,
      y,
      button: 'left',
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

  async function readLatestFinishedJson(urlSubstring) {
    for (let i = responsesByUrl.length - 1; i >= 0; i -= 1) {
      const rec = responsesByUrl[i];
      if (!rec.url.includes(urlSubstring) || rec.status !== 200) continue;
      if (!finishedBodies.has(rec.requestId)) continue;
      try {
        const body = await cdp.send('Network.getResponseBody', { requestId: rec.requestId });
        const raw = body.base64Encoded ? Buffer.from(body.body, 'base64').toString('utf8') : body.body;
        return JSON.parse(raw);
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
    evalJs,
    callFn,
    waitFor,
    waitUntil,
    elementBox,
    clickAt,
    click,
    pressKey,
    readLatestFinishedJson,
  };
}

async function suite1BootThemeTransport(driver, opts) {
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
  driver.assertNoErrors('Suite 1');
}

async function assertDenseCanvasGeometry(driver, canvasSelector, label) {
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
      width: canvas.width,
      height: canvas.height,
      position: cs.position,
      pointerEvents: cs.pointerEvents,
      hostRect: { top: hostRect.top, bottom: hostRect.bottom, left: hostRect.left, right: hostRect.right, height: hostRect.height },
      canvasRect: { top: canvasRect.top, bottom: canvasRect.bottom, left: canvasRect.left, right: canvasRect.right },
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
  assert(geom.position === 'absolute', `${label}: expected .graph-dense-canvas position === "absolute", got "${geom.position}"`);
  assert(geom.nonZeroPixels > 0 && geom.dataUrlDiffers, `${label}: dense canvas is blank`);
  assert(
    geom.overlapY > 0 && geom.overlapX > 0 && geom.planeRect.top < geom.hostRect.bottom - 4,
    `${label}: .graph-map-plane does not overlap stage (planeTop=${geom.planeRect.top}, hostBottom=${geom.hostRect.bottom}, overlapY=${geom.overlapY})`,
  );
  assert(geom.nodeCount > 0, `${label}: expected visible .graph-map-node elements in plane`);
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
    await driver.click(`#graph-map-lens-${lens}`);
    const state = await driver.waitFor((activeLens, allLenses) => {
      const c = document.getElementById('graph-map-canvas');
      if (c?.getAttribute('aria-busy') === 'true') return null;
      const activeBtn = document.getElementById(`graph-map-lens-${activeLens}`);
      if (activeBtn?.getAttribute('aria-selected') !== 'true') return null;

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
    }, `lens ${lens} activation`, 20_000, lens, ['dependencies', 'changes', 'architecture', 'tests', 'runs']);

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
  }

  await driver.click('#graph-map-close');
  await driver.waitFor(() => !document.getElementById('graph-map')?.hasAttribute('open'), '#graph-map dialog close');
  driver.assertNoErrors('Suite 3');
}

async function suite4LiveSseAndStyling(driver, opts) {
  console.log('[Suite 4] Live SSE stream & .conversation-* / .md-body styling');
  assert(driver.sseConnections.length > 0, 'Expected /api/events/stream EventSource connection');
  const sseConn = driver.sseConnections[driver.sseConnections.length - 1];
  assert(sseConn.status === 200, `Expected SSE 200, got ${sseConn.status}`);
  assert(
    sseConn.mimeType.includes('text/event-stream') || (sseConn.headers['content-type'] || '').includes('text/event-stream'),
    `Expected text/event-stream MIME on /api/events/stream, got ${sseConn.mimeType}`,
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

  await driver.callFn(() => {
    const prompt = document.getElementById('prompt');
    prompt.value = 'Run browser E2E live SSE verification turn';
    prompt.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await driver.waitFor(() => {
    const send = document.getElementById('send');
    return send && !send.disabled;
  }, '#send enabled after typing prompt (check inbox eligibility on running session)');
  await driver.click('#send');

  await driver.waitUntil(
    () => driver.sseMessages.slice(sseCountBefore).some((m) => m.eventName === 'session_event'),
    'live SSE session_event received on /api/events/stream after #send',
  );

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

async function suite5ModalsApprovalsPaletteMobile(driver, cdp) {
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
        role: card.getAttribute('role'),
        ariaModal: card.getAttribute('aria-modal'),
        mainInert: document.getElementById('main')?.hasAttribute('inert'),
      };
    });
    assert(fileCardAttrs.role === 'dialog' && fileCardAttrs.ariaModal === 'true' && fileCardAttrs.mainInert, `Invalid #run-file-card dialog state: ${JSON.stringify(fileCardAttrs)}`);
    await driver.pressKey({ key: 'Escape', code: 'Escape', keyCode: 27 });
    await driver.waitFor(() => document.getElementById('run-file-card')?.hidden && !document.getElementById('main')?.hasAttribute('inert'), '#run-file-card closes on Escape');
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

  // Escape closes #approvals-drawer and clears inert
  await driver.pressKey({ key: 'Escape', code: 'Escape', keyCode: 27 });
  await driver.waitFor(() => {
    const d = document.getElementById('approvals-drawer');
    const main = document.getElementById('main');
    const sidebar = document.getElementById('sidebar');
    return d?.hidden === true && !main?.hasAttribute('inert') && !sidebar?.hasAttribute('inert');
  }, '#approvals-drawer closes on Escape and clears inert');

  // Mobile viewport 390x844 & #menu-toggle -> #sidebar.open without horizontal overflow
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

  // Close mobile #sidebar via exposed #scrim (to the right of the 265px drawer), then restore 1280x800 desktop viewport
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

  // Verify #session-settings-toggle ("Next-turn settings") stays unclipped inside .configuration at 1280x800 with #run-detailed open
  const wasDetailedBeforeDesktopCheck = await driver.callFn(() => document.getElementById('run-card')?.getAttribute('data-detailed') === 'true');
  if (!wasDetailedBeforeDesktopCheck) {
    await driver.click('#run-detailed');
  }
  await waitForRunPanelDetailedSettled(driver);
  await assertSessionSettingsToggleUnclipped(driver, 'Suite 5 (1280x800) with #run-detailed open');
  if (!wasDetailedBeforeDesktopCheck) {
    await driver.click('#run-calm');
  }

  driver.assertNoErrors('Suite 5');
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

      await suite1BootThemeTransport(driver, opts);
      await suite2Canvas2dAndMapLayout(driver, pageCdp, opts);
      await suite3MapLenses(driver);
      await suite4LiveSseAndStyling(driver, opts);
      await suite5ModalsApprovalsPaletteMobile(driver, pageCdp);

      driver.assertNoErrors('final check');
      console.log('ALL 5 HEADLESS CHROME CDP E2E SUITES PASSED');
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
