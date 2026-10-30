#!/usr/bin/env node
// Strict ISO comparison between the native `dbsrt top` (reference, SSH in tmux) and the deployed
// web-top (Ratzilla WASM in headless Chromium driven over CDP).
//
// Both sides get the same grid size and the same key sequence; after each step the screens are
// captured as text, volatile fields are masked (explicit list below) and compared line by line.
// Exit code: 0 when every step matches, 1 on any ISO mismatch, 2 on harness failure,
// 3 when the only mismatches happened while the web feed was down (infra, not ISO).
//
// Env: O2SWITCH_HOST (default o2switch), WEB_TOP_URL, CHROME_BIN, CDP_PORT, ISO_OUT_DIR, ISO_STEPS,
// ISO_VIEWPORTS (CSS px "WxH" list; a width under 600 is emulated as an Android phone: Chrome
// Android user agent, touch screen).
// A `tap:<text>` step touches (mouse-clicks on desktop) the first cell of <text> found from the
// bottom of the web grid; the native side gets <text> typed as keys, so ISO proves the tap.
// A `swipe:<n>` step drags the finger (mouse on desktop) up n grid rows (down when n < 0): the
// list follows the finger, so the native side gets n Down (or Up) keys.
// The read-only API key is read over SSH and never written to disk or stdout.

import { execFileSync, spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const HOST = process.env.O2SWITCH_HOST ?? 'o2switch';
const WEB_URL = process.env.WEB_TOP_URL ?? 'https://2026-pydbsrt.fr/dbsrt/top/';
const CHROME = process.env.CHROME_BIN ?? 'chromium';
const CDP_PORT = Number(process.env.CDP_PORT ?? 9333);
const OUT_DIR = process.env.ISO_OUT_DIR ?? 'target/top_iso';
const TMUX_SESSION = 'dbsrt_top_iso';
const NATIVE_CMD =
  'cd dbsrt && PATH=$HOME/dbsrt/bin:$PATH RAYON_NUM_THREADS=2 ./bin/dbsrt top';
// Esc is only sent from a sub-view or to cancel a search: on the active jobs view it quits the
// native app. A multi-char step is typed as text (search query).
const DEFAULT_STEPS = [
  'start,1,2,3,Down,Enter,Esc,f,o,4,Down,Enter,Esc,5,6,?,Tab,BackTab,?,Tab,1',
  // search: filter, scroll the filtered list, cancel
  '3,/,the,Enter,PageDown,/,Esc',
  // scrolling on the history, media browser and help views
  'PageDown,PageDown,End,Home,4,End,PageUp,Home,?,PageDown,End,Home,?,1',
  // tab bar taps (touch on the phone viewport)
  'tap:3,tap:5,tap:2,tap:1',
  // swipe scrolling: past the visible history rows (the phone layout marks the selected row by
  // color only, so short swipes leave the text unchanged), then the help text
  '3,swipe:20,swipe:20,swipe:-5,?,swipe:5,swipe:-2,?,1',
].join(',');
const STEPS = (process.env.ISO_STEPS ?? DEFAULT_STEPS).split(',').filter(Boolean);
const VIEWPORTS = (process.env.ISO_VIEWPORTS ?? '1400x900,412x915').split(',').filter(Boolean)
  .map((v) => {
    const [width, height] = v.split('x').map(Number);
    if (!(width > 0 && height > 0)) throw new Error(`bad ISO_VIEWPORTS entry: ${v}`);
    return { label: v, width, height, mobile: width < 600 };
  });
const READONLY_KEY_CMD =
  "grep -m 1 -v -e '^#' -e '^[[:space:]]*$' ~/dbsrt/etc/api_key_readonly.secret";
const SETTLE_MS = 2500;
// Live job/batch transitions land on each side at a different poll: re-capture before judging.
const RETRIES = 4;
// The web client shows "⚠️ <HTTP code|Erreur|Échec réseau>" while its feed is down: its frame is stale.
// The web grid pads a wide emoji with a blank cell, hence `\s+`.
const WEB_FEED_DOWN = /⚠️\s+(\d{3} |Erreur|Échec réseau)/;
const UA =
  'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36';
// Ratzilla sizes its grid from the screen when the user agent says "Mobile", like on a real phone.
const ANDROID_UA =
  'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36';
const TAP_PREFIX = 'tap:';
const SWIPE_PREFIX = 'swipe:';

// Volatile fields: values sampled at different instants by the two sides.
const VOLATILE = [
  [/\b\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2})?\S*/g, '<datetime>'],
  [/\b\d{1,2}:\d{2}(:\d{2})?\b/g, '<clock>'],
  [/\d+(\.\d+)?\s?(%|[KMGT]i?B(\/s)?|B\/s|[kMG]bps|°C|[MG]Hz|fps|x)(?![\w])/g, '<qty>'],
  [/(<qty> )\(\d+\/\d+/g, '$1(<frames>'],
  // The header may clip a labelled field at the right border, dropping its unit.
  [/(:\s*)\d+(\.\d+)?\s?(?=│)/g, '$1<qty>'],
  [/\d+(\.\d+)?\s*\/\s*\d+\.?\d*(?=│)/g, '<qty>/<qty>'],
  // The web grid blanks the second cell of a wide emoji; native text packs adjacent ones.
  [/(\p{Extended_Pictographic}️?) (?=\p{Extended_Pictographic}|\.\.\.)/gu, '$1'],
  [/\b\d+(\.\d+)?\s?(d|h|min|ms|m|s)(\d+(h|min|ms|m|s))*\b(\s+ago)?/g, '<dur>'],
  [/\bload\s+[\d.]+\s+[\d.]+\s+[\d.]+/gi, 'load <loadavg>'],
  [/[▁▂▃▄▅▆▇█▏▎▍▌▋▊▉░▒▓⣀⣄⣤⣦⣶⣷⣿⡀⢀]+/g, '<bar>'],
  [/\d+(\.\d+)?\s*\/\s*<qty>/g, '<qty>/<qty>'],
  // Right-aligned values pad with a width that depends on the masked value.
  [/\( +</g, '(<'],
];

let apiKey = '';
const redact = (s) => (apiKey ? String(s).split(apiKey).join('<KEY>') : String(s));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export function normalize(screen) {
  return screen
    .split('\n')
    .map((line) => {
      let out = line.replace(/ /g, ' ');
      for (const [re, tag] of VOLATILE) out = out.replace(re, tag);
      // Masked values change width, so alignment padding is not compared.
      return out.replace(/ {2,}/g, ' ').trimEnd();
    })
    .join('\n')
    .replace(/\n+$/, '');
}

export function diffLines(native, web) {
  const a = native.split('\n');
  const b = web.split('\n');
  const diffs = [];
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if ((a[i] ?? '') !== (b[i] ?? '')) diffs.push({ line: i + 1, native: a[i] ?? '', web: b[i] ?? '' });
  }
  return diffs;
}

function selfCheck() {
  const n = normalize('CPU 12.5%  RAM 28.8 / 3.2 GiB  12:01:02  ██▓░  up 1h20m  ( 62.2 Mbps)  0.6% (97/150');
  const w = normalize('CPU 9%     RAM 3/3.25 GiB 12:01:05  █░░░  up 1h19m  (538.8 Mbps)  0.6% (96/150');
  if (n !== w) throw new Error(`normalize self-check failed:\n${n}\n${w}`);
  const clip = (s) => normalize(`│ RAM : ${s}│ (🧠👩...`);
  if (clip('29.7 / 125.') !== normalize('│ RAM : 29.5 / 125.│ (🧠 👩 ...')) {
    throw new Error(`normalize clip/emoji self-check failed: ${clip('29.7 / 125.')}`);
  }
  const field = (s) => normalize(`│ ⚡ FLUX REEL │ ${s}│`);
  if (field('Net RX: 1.0 ') !== field('Net RX: 0.2 ') || field('Host CPU: 7') !== field('Host CPU: 12')) {
    throw new Error(`normalize clipped-field self-check failed: ${field('Net RX: 1.0 ')}`);
  }
  if (field('Actifs: 3   ') === field('Actifs: 4   ')) {
    throw new Error('normalize must keep padded (unclipped) values');
  }
  if (diffLines('a\nb', 'a\nc').length !== 1) throw new Error('diffLines self-check failed');
  if (tapText('tap:3') !== '3' || tapText('3') !== null) throw new Error('tapText self-check failed');
  if (swipeRows('swipe:-2') !== -2 || swipeRows('Down') !== null) throw new Error('swipeRows self-check failed');
  if (!WEB_FEED_DOWN.test('📢 ⚠️ 421 SNI Mismatch') || !WEB_FEED_DOWN.test('📢  ⚠️  421 SNI') || WEB_FEED_DOWN.test('📢 Lecture seule')) {
    throw new Error('WEB_FEED_DOWN self-check failed');
  }
}

// ---------- native side (tmux + ssh) ----------
const tmux = (...args) =>
  execFileSync('tmux', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
const TMUX_KEYS = {
  Tab: 'Tab', BackTab: 'BTab', Enter: 'Enter', Esc: 'Escape', Up: 'Up', Down: 'Down',
  Left: 'Left', Right: 'Right', PageUp: 'PPage', PageDown: 'NPage', Home: 'Home', End: 'End',
  Backspace: 'BSpace',
};

function nativeStart(cols, rows) {
  try { tmux('kill-session', '-t', TMUX_SESSION); } catch { /* not running */ }
  tmux('new-session', '-d', '-s', TMUX_SESSION, '-x', String(cols), '-y', String(rows),
    `ssh -tt ${HOST} "${NATIVE_CMD}"`);
  // A running tmux server may impose its own window-size policy: pin the exact web grid size.
  tmux('set-option', '-t', TMUX_SESSION, 'window-size', 'manual');
  tmux('resize-window', '-t', TMUX_SESSION, '-x', String(cols), '-y', String(rows));
}
const tapText = (step) => (step.startsWith(TAP_PREFIX) ? step.slice(TAP_PREFIX.length) : null);

const swipeRows = (step) => {
  if (!step.startsWith(SWIPE_PREFIX)) return null;
  const rows = Number(step.slice(SWIPE_PREFIX.length));
  if (!Number.isInteger(rows) || rows === 0) throw new Error(`bad swipe step: ${step}`);
  return rows;
};

function nativeKey(step) {
  const rows = swipeRows(step);
  if (rows) {
    for (let i = 0; i < Math.abs(rows); i++) nativeKey(rows > 0 ? 'Down' : 'Up');
    return;
  }
  step = tapText(step) ?? step;
  if (TMUX_KEYS[step]) tmux('send-keys', '-t', TMUX_SESSION, TMUX_KEYS[step]);
  else tmux('send-keys', '-t', TMUX_SESSION, '-l', step);
}
const nativeCapture = () => tmux('capture-pane', '-p', '-t', TMUX_SESSION);
// Quit the remote app with `q` first so no orphan `dbsrt top` keeps running on o2switch.
async function nativeStop() {
  try {
    tmux('send-keys', '-t', TMUX_SESSION, 'Escape', 'q');
    await sleep(1500);
    tmux('kill-session', '-t', TMUX_SESSION);
  } catch { /* already gone */ }
}

// ---------- web side (Chromium over CDP) ----------
const CDP_KEYS = {
  Tab: ['Tab', 9], BackTab: ['Tab', 9], Enter: ['Enter', 13], Esc: ['Escape', 27],
  Up: ['ArrowUp', 38], Down: ['ArrowDown', 40], Left: ['ArrowLeft', 37], Right: ['ArrowRight', 39],
  PageUp: ['PageUp', 33], PageDown: ['PageDown', 34], Home: ['Home', 36], End: ['End', 35],
  Backspace: ['Backspace', 8],
};

async function cdpConnect() {
  for (let i = 0; i < 50; i++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`)).json();
      const page = targets.find((t) => t.type === 'page');
      if (page) return page.webSocketDebuggerUrl;
    } catch { /* chromium still starting */ }
    await sleep(200);
  }
  throw new Error('Chromium CDP endpoint not reachable');
}

function cdpSession(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let id = 0;
  const pending = new Map();
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) {
      const { resolve, reject } = pending.get(m.id);
      pending.delete(m.id);
      m.error ? reject(new Error(redact(m.error.message))) : resolve(m.result);
    }
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const i = ++id;
    pending.set(i, { resolve, reject });
    ws.send(JSON.stringify({ id: i, method, params }));
  });
  return new Promise((resolve) => { ws.onopen = () => resolve({ send, close: () => ws.close() }); });
}

async function webKey(cdp, step) {
  if (CDP_KEYS[step]) {
    const [key, vk] = CDP_KEYS[step];
    const modifiers = step === 'BackTab' ? 8 : 0;
    await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key, code: key, windowsVirtualKeyCode: vk, modifiers });
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code: key, windowsVirtualKeyCode: vk, modifiers });
    return;
  }
  for (const ch of step) {
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: ch, text: ch });
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: ch });
  }
}

// Center of the first cell spelling `text`, searched from the bottom row (footer) upwards.
const CELL_OF_JS = (text) => `(() => {
  const want = ${JSON.stringify(text)};
  const rows = [...(document.getElementById('grid')?.children ?? [])];
  for (let r = rows.length - 1; r >= 0; r--) {
    const cells = [...rows[r].children];
    for (let i = 0; i < cells.length; i++) {
      let got = '';
      for (let j = i; j < cells.length && got.length < want.length; j++) got += cells[j].textContent;
      if (got === want) {
        const b = cells[i].getBoundingClientRect();
        return { x: b.left + b.width / 2, y: b.top + b.height / 2 };
      }
    }
  }
  return null;
})()`;
// Records the pointer type the page really received, so a tap step proves the touch path.
const POINTER_PROBE_JS = `window.__isoPointer = null;
if (!window.__isoProbe) {
  window.__isoProbe = true;
  document.addEventListener('pointerup', (e) => { window.__isoPointer = e.pointerType; }, true);
}`;

async function webTap(cdp, text, mobile) {
  const r = await cdp.send('Runtime.evaluate', { expression: CELL_OF_JS(text), returnByValue: true });
  const at = r.result.value;
  if (!at) throw new Error(`tap target "${text}" not found on the web grid`);
  await cdp.send('Runtime.evaluate', { expression: POINTER_PROBE_JS });
  await pointerPath(cdp, [at], mobile);
  await expectPointer(cdp, `tap "${text}"`, mobile);
}

// Grid center and row height, to drag over a known number of rows.
const GRID_GEOMETRY_JS = `(() => {
  const g = document.getElementById('grid').getBoundingClientRect();
  const rows = document.getElementById('grid').children.length;
  return { x: g.left + g.width / 2, y: g.top + g.height / 2, rowPx: g.height / rows, top: g.top, bottom: g.bottom };
})()`;

async function webSwipe(cdp, rows, mobile) {
  const g = (await cdp.send('Runtime.evaluate', { expression: GRID_GEOMETRY_JS, returnByValue: true })).result.value;
  // Half a row more than needed: the page scrolls one line per whole row travelled.
  const travel = (Math.abs(rows) + 0.5) * g.rowPx * Math.sign(rows);
  const start = g.y + travel / 2;
  const inGrid = (y) => y >= g.top && y <= g.bottom;
  if (!inGrid(start) || !inGrid(start - travel)) throw new Error(`swipe:${rows} exceeds the grid height`);
  const path = [];
  for (let i = 0; i <= 2 * Math.abs(rows) + 1; i++) {
    path.push({ x: g.x, y: start - (travel * i) / (2 * Math.abs(rows) + 1) });
  }
  await cdp.send('Runtime.evaluate', { expression: POINTER_PROBE_JS });
  await pointerPath(cdp, path, mobile);
  await expectPointer(cdp, `swipe:${rows}`, mobile);
}

// Presses at the first point, moves through the others and releases at the last one: a real
// touch sequence on the phone viewport, a left-button mouse drag on desktop.
async function pointerPath(cdp, path, mobile) {
  const [first, ...rest] = path;
  if (mobile) {
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [first] });
    for (const p of rest) await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [p] });
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    return;
  }
  const last = path[path.length - 1];
  await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...first, button: 'left', clickCount: 1 });
  for (const p of rest) await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...p, button: 'left', buttons: 1 });
  await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...last, button: 'left', clickCount: 1 });
}

async function expectPointer(cdp, label, mobile) {
  const want = mobile ? 'touch' : 'mouse';
  let seen = null;
  for (let i = 0; i < 10 && seen !== want; i++) {
    seen = (await cdp.send('Runtime.evaluate', { expression: 'window.__isoPointer', returnByValue: true })).result.value;
    if (seen !== want) await sleep(100);
  }
  if (seen !== want) throw new Error(`${label}: page saw pointerType ${seen}, expected ${want}`);
}

// Ratzilla DomBackend::size() reports the DOM grid minus one column and one row, so the app
// draws a (cols-1)x(rows-1) frame: only that frame is captured and mirrored in tmux.
const GRID_JS = `(() => {
  const grid = document.getElementById('grid');
  if (!grid) return null;
  const rows = [...grid.children];
  // One <span> per buffer cell: text length is wrong for emoji with variation selectors.
  const cols = Math.max(0, ...rows.map((r) => r.children.length)) - 1;
  const frame = rows.slice(0, -1);
  const text = frame
    .map((r) => [...r.children].slice(0, cols).map((c) => c.textContent).join(''))
    .join('\\n');
  return { rows: frame.length, cols, text };
})()`;
async function webGrid(cdp) {
  const r = await cdp.send('Runtime.evaluate', { expression: GRID_JS, returnByValue: true });
  return r.result.value;
}

// ---------- driver ----------
// Replays every step on one viewport (fresh page load, fresh native session of the same grid).
async function runViewport(cdp, vp) {
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: vp.width, height: vp.height, deviceScaleFactor: vp.mobile ? 2.625 : 1, mobile: vp.mobile,
    screenWidth: vp.width, screenHeight: vp.height,
  });
  await cdp.send('Emulation.setUserAgentOverride', {
    userAgent: vp.mobile ? ANDROID_UA : UA, platform: vp.mobile ? 'Linux armv8l' : 'Linux x86_64',
  });
  await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: vp.mobile, maxTouchPoints: vp.mobile ? 5 : 1 });
  await cdp.send('Page.navigate', { url: `${WEB_URL}?token=${encodeURIComponent(apiKey)}` });

  let grid = null;
  for (let i = 0; i < 60 && !(grid && grid.rows > 0 && grid.cols > 0); i++) {
    await sleep(500);
    grid = await webGrid(cdp);
  }
  if (!grid?.rows) throw new Error(`web-top grid never rendered (${vp.label})`);
  console.log(`viewport ${vp.label}${vp.mobile ? ' mobile' : ''}: grid ${grid.cols}x${grid.rows}`);

  nativeStart(grid.cols, grid.rows);
  // Both sides must show live data (same header) before the first step: the web side may still
  // be backing off after an upstream failure.
  let ready = false;
  for (let i = 0; i < 90 && !ready; i++) {
    await sleep(1000);
    const head = (t) => normalize(t).split('\n')[0];
    ready = head(nativeCapture()) === head((await webGrid(cdp)).text);
  }
  if (!ready) throw new Error('native and web headers never converged (web-top has no data?)');

  const summary = [];
  let failures = 0;
  let infraFailures = 0;
  for (const [index, step] of STEPS.entries()) {
    if (step !== 'start') {
      nativeKey(step);
      const text = tapText(step);
      const rows = swipeRows(step);
      if (text) await webTap(cdp, text, vp.mobile);
      else if (rows) await webSwipe(cdp, rows, vp.mobile);
      else await webKey(cdp, step);
    }
    let nativeRaw, webRaw, diffs;
    for (let attempt = 0; attempt <= RETRIES && (!diffs || diffs.length); attempt++) {
      await sleep(SETTLE_MS);
      nativeRaw = nativeCapture();
      webRaw = redact((await webGrid(cdp)).text);
      diffs = diffLines(normalize(nativeRaw), normalize(webRaw));
    }
    const tag = `${vp.label}_${String(index).padStart(2, '0')}_${step.replace(/[^\w]/g, (c) => `x${c.charCodeAt(0)}`)}`;
    writeFileSync(join(OUT_DIR, `${tag}.native.txt`), nativeRaw);
    writeFileSync(join(OUT_DIR, `${tag}.web.txt`), webRaw);
    writeFileSync(join(OUT_DIR, `${tag}.diff.txt`),
      diffs.map((d) => `L${d.line}\n  native: ${d.native}\n  web:    ${d.web}`).join('\n'));
    // A diff while the web feed is down says nothing about ISO: report it apart.
    const status = !diffs.length ? 'OK  ' : WEB_FEED_DOWN.test(webRaw) ? 'INFRA' : 'FAIL';
    summary.push(`${status} ${tag} (${diffs.length} lines differ)`);
    if (status === 'FAIL') failures++;
    if (status === 'INFRA') infraFailures++;
  }
  console.log(summary.join('\n'));
  return { failures, infraFailures };
}

async function main() {
  selfCheck();
  // The web side authenticates with the read-only key, exactly like the shared QR code.
  apiKey = execFileSync('ssh', [HOST, READONLY_KEY_CMD], { encoding: 'utf8' }).trim();
  if (!apiKey) throw new Error('empty read-only API key (run o2switch_ctl.sh rotate-readonly-key)');
  mkdirSync(OUT_DIR, { recursive: true });

  const profile = mkdtempSync(join(tmpdir(), 'dbsrt-iso-'));
  const chrome = spawn(CHROME, ['--headless=new', `--remote-debugging-port=${CDP_PORT}`,
    `--user-data-dir=${profile}`, '--window-size=1400,900', 'about:blank'], { stdio: 'ignore' });
  let failures = 0;
  let infraFailures = 0;
  try {
    const cdp = await cdpSession(await cdpConnect());
    await cdp.send('Network.setUserAgentOverride', { userAgent: UA });
    for (const vp of VIEWPORTS) {
      const r = await runViewport(cdp, vp);
      failures += r.failures;
      infraFailures += r.infraFailures;
      await nativeStop();
    }
    console.log(`captures: ${OUT_DIR}`);
    cdp.close();
  } finally {
    await nativeStop();
    chrome.kill();
    rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
  process.exit(failures ? 1 : infraFailures ? 3 : 0);
}

main().catch((e) => {
  console.error(redact(e.stack ?? e));
  process.exit(2);
});
