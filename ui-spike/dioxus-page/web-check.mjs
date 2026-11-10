// Section 10's try, in a real browser: the window's components served by page-web (dioxus-liveview), driven by headless
// Chrome with real mouse and key input over CDP. One JSON line a behaviour, then the server's memory per tab.
//   cargo build --release -p dioxus-page --no-default-features --features web --bin page-web
//   node rust/ui-spike/dioxus-page/web-check.mjs <shots dir>
// Not Rust because it drives a browser (CLAUDE.md, Coordination 5); the same reason as test/scripts/shot, whose flags it takes.
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';

const root = fileURLToPath(new URL('../../..', import.meta.url));
const { bin, flags } = await import(`${root}private/dashboard/test/scripts/chrome.mjs`);
const shots = process.argv[2];
if (shots) mkdirSync(shots, { recursive: true });
if (spawnSync(`${root}bin/browser-slot`, [String(process.pid)], { stdio: 'inherit' }).status !== 0) process.exit(1);
const sleep = ms => new Promise(r => setTimeout(r, ms));
const port = 7800 + Math.floor(Math.random() * 100);
const server = spawn(`${root}rust/ui-spike/target/release/page-web`, [String(port)], { stdio: 'ignore' });
const dir = mkdtempSync(tmpdir() + '/web-check-');
const cdp = 9300 + Math.floor(Math.random() * 600);
const chrome = spawn(bin, [...flags, '--hide-scrollbars', `--user-data-dir=${dir}`, `--remote-debugging-port=${cdp}`, 'about:blank'], { stdio: 'ignore' });
const done = code => { chrome.kill('SIGKILL'); server.kill('SIGKILL'); rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }); process.exit(code); };
// A failed top-level await is uncaughtException, never unhandledRejection: both stop what this started.
for (const event of ['unhandledRejection', 'uncaughtException']) process.on(event, e => { console.error(String(e)); done(1); });
setTimeout(() => { console.error('web-check: timeout'); done(2); }, 150000);

// dioxus-liveview 0.8 alpha mounts on a catch-all that does not match a bare /: any other path serves the app.
const url = `http://127.0.0.1:${port}/`;
for (let i = 0; i < 50; i++) { try { await fetch(`http://127.0.0.1:${cdp}/json`); await fetch(`${url}page`); break; } catch { await sleep(200); } }

async function tab() {
  const t = await (await fetch(`http://127.0.0.1:${cdp}/json/new?about:blank`, { method: 'PUT' })).json();
  const ws = new WebSocket(t.webSocketDebuggerUrl);
  await new Promise(r => { ws.onopen = r; });
  let id = 0; const pend = {};
  ws.onmessage = m => { const d = JSON.parse(m.data); if (pend[d.id]) { pend[d.id](d.result || d); delete pend[d.id]; } };
  const send = (method, params = {}) => new Promise(r => { pend[++id] = r; ws.send(JSON.stringify({ id, method, params })); });
  await send('Page.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: 1200, height: 800, deviceScaleFactor: 2, mobile: false });
  await send('Page.navigate', { url: `${url}page` });
  return send;
}

const send = await tab();
const js = async e => (await send('Runtime.evaluate', { expression: e, returnByValue: true, awaitPromise: true })).result?.value;
const text = s => js(`document.querySelector(${JSON.stringify(s)})?.textContent ?? ''`);
const has = s => js(`!!document.querySelector(${JSON.stringify(s)})`);
const bg = s => js(`(e => e && getComputedStyle(e).backgroundColor)(document.querySelector(${JSON.stringify(s)}))`);
const mid = s => js(`(r => r && [r.left + r.width / 2, r.top + r.height / 2])(document.querySelector(${JSON.stringify(s)})?.getBoundingClientRect())`);
let at = [0, 0];
// When the last input left for the page: the time to show counts from here, never from its ack.
let inputAt = 0;
const mouse = (type, x, y, extra = {}) => { inputAt = performance.now(); return send('Input.dispatchMouseEvent', { type, x, y, ...extra }); };
const move = async p => { at = p; await mouse('mouseMoved', p[0], p[1]); };
const moveTo = async s => move(await mid(s));
const down = () => mouse('mousePressed', at[0], at[1], { button: 'left', buttons: 1, clickCount: 1 });
const up = () => mouse('mouseReleased', at[0], at[1], { button: 'left', clickCount: 1 });
const click = async s => { await moveTo(s); await down(); await up(); };
const type = t => { inputAt = performance.now(); return send('Input.insertText', { text: t }); };
const drag = async (from, to) => {
  await move(from); await down();
  for (let i = 1; i <= 12; i++) { await mouse('mouseMoved', from[0] + (to[0] - from[0]) * i / 12, from[1] + (to[1] - from[1]) * i / 12, { button: 'left', buttons: 1 }); await sleep(8); }
  at = to; await up();
};
const until = async (what, f, ms = 2000) => {
  const start = performance.now();
  while (performance.now() - start < ms) { if (await f()) return performance.now() - start; await sleep(4); }
  throw new Error(what);
};
const shot = async name => { if (shots) writeFileSync(`${shots}/web-${name}.png`, Buffer.from((await send('Page.captureScreenshot', { format: 'png' })).data, 'base64')); };

const lines = [];
async function behaviour(name, f) {
  const seen = [];
  const timed = async (what, g) => { await until(what, g); seen.push(performance.now() - inputAt); };
  try { await f(timed); lines.push({ behaviour: name, ok: true, ms_to_show_worst: Math.round(Math.max(0, ...seen)), ms_to_show_median: Math.round(seen.sort((a, b) => a - b)[seen.length >> 1] ?? 0) }); }
  catch (e) { lines.push({ behaviour: name, ok: false, why_not: e.message }); }
  console.log(JSON.stringify(lines.at(-1)));
}
const hover = async (timed, sel) => {
  const rest = await bg(sel);
  for (let i = 0; i < 5; i++) {
    await moveTo(sel); await timed('the :hover style', async () => (await bg(sel)) !== rest);
    await moveTo('.bar-title'); await timed('hover ends', async () => (await bg(sel)) === rest);
  }
};

await until('the page arrives over the socket', () => has('#card-101'), 10000);
await sleep(300);

// Measured first, on a fresh server, each tab proved mounted: after mail's 100,000-row search a new tab fills freed heap
// and footprint barely moves, and a tab whose socket has not connected holds no vdom yet.
const memory = async () => (await fetch(`${url}memory`)).json();
const one = await memory();
const more = [];
for (let i = 0; i < 9; i++) {
  const s = await tab();
  await until('a tab mounts', async () => (await s('Runtime.evaluate', { expression: `!!document.querySelector('#card-101')`, returnByValue: true })).result?.value, 10000);
  more.push(s);
}
await sleep(2000);
const ten = await memory();
// No await: a closed page never answers. The input below goes to the first tab, in front again.
for (const s of more) s('Page.close');
await send('Page.bringToFront');
await sleep(300);

await behaviour('board: hover', async t => { await moveTo('.bar-title'); await sleep(50); await hover(t, '#card-101'); await moveTo('#card-101'); await sleep(50); await shot('board-hover'); });
await behaviour('board: click, typing, dialog', async t => {
  for (let i = 0; i < 3; i++) {
    await click('#card-101'); await t('a click opens its dialog', () => has('#dialog'));
    await sleep(300); await click('#issue-title-input'); await type('!');
    await t('the key reaches the input', () => js(`document.querySelector('#issue-title-input').value.endsWith('!')`));
    if (i === 2) await shot('board-dialog');
    await click('#issue-save'); await t('the title changes', async () => !(await has('#dialog')) && (await text('#card-101')).includes('!'.repeat(i + 1)));
  }
});
await behaviour('board: drag and drop', async t => {
  await drag(await mid('#card-105'), await mid('#col-2')); await t('the card lands in In Progress', async () => (await text('#col-2')).includes('AAA-105'));
  await shot('board');
  await drag(await mid('#card-105'), await mid('#col-1')); await t('and comes back', async () => (await text('#col-1')).includes('AAA-105'));
});
await behaviour('board: dark theme', async t => { const rest = await bg('#card-103'); await click('#theme-toggle'); await t('dark colours', async () => (await bg('#card-103')) !== rest); await sleep(50); await shot('board-dark'); await click('#theme-toggle'); await t('and back', async () => (await bg('#card-103')) === rest); });

await click('#nav-diagram'); await until('the diagram opens', () => has('#canvas'));
const canvas = (x, y) => [200 + x, 56 + y];
await behaviour('diagram: click', async t => { await move(canvas(115, 230)); await down(); await up(); await t('selected', async () => (await text('#diagram-status')).includes('Views')); });
await behaviour('diagram: hover', async t => { await move(canvas(375, 230)); await t('under the pointer', async () => (await text('#diagram-hover')).includes('UI tree')); });
await behaviour('diagram: drag a shape', async t => { await drag(canvas(115, 230), canvas(195, 350)); await t('it lands', async () => (await text('#diagram-status')).includes('at 120,310')); await sleep(50); await shot('diagram'); await drag(canvas(195, 350), canvas(115, 230)); await t('and goes back', async () => (await text('#diagram-status')).includes('at 40,190')); });
await behaviour("diagram: a shape's own onclick", async t => { await move(canvas(115, 230)); await down(); await up(); await t('a path fires', async () => !(await text('#inner-clicks')).endsWith(': 0')); });

await click('#nav-mail'); await until('mail opens', () => has('#list'));
await behaviour('mail: hover', async t => { await moveTo('.bar-title'); await sleep(50); await hover(t, '#line-1'); await moveTo('#line-1'); await sleep(50); await shot('mail'); });
await behaviour('mail: click', async t => { await click('#star-1'); await t('star', async () => (await text('#mail-state')).startsWith('1 starred')); await click('#star-1'); await t('unstar', async () => (await text('#mail-state')).startsWith('0 starred')); });
await behaviour('mail: typing', async t => { await click('#mail-search'); await type('Row 4999'); await t('the search filters', async () => (await text('#mail-count')) === '11 lines', 5000); });
await behaviour('mail: wheel', async t => {
  inputAt = performance.now();
  await js(`(i => { i.value = ''; i.dispatchEvent(new Event('input', { bubbles: true })); })(document.querySelector('#mail-search'))`);
  await t('all lines', async () => (await text('#mail-count')) === '100000 lines', 5000);
  await moveTo('#list');
  for (let i = 0; i < 20; i++) { await mouse('mouseWheel', at[0], at[1], { deltaX: 0, deltaY: 700 }); await sleep(16); }
  // 20 steps of 700 px: row 269 at the top, so the rows before it are gone from the window.
  await t('the lines follow', async () => (await has('#line-280')) && !(await has('#line-1')), 3000);
});
await behaviour('mail: dialog', async t => { await click('#compose'); await t('it opens', () => has('#dialog')); await sleep(300); await click('#draft-input'); await type('hello'); await t('the draft', async () => (await text('#draft-echo')) === '5 characters'); await click('#send'); await t('it closes', async () => !(await has('#dialog'))); });

await click('#nav-comments'); await until('comments open', () => has('#block-1'));
await behaviour('comments: hover', async t => { await moveTo('.bar-title'); await sleep(50); await hover(t, '#block-1'); });
await behaviour('comments: click, typing', async t => { await moveTo('#block-1'); await sleep(50); await click('#add-1'); await t('the composer', () => has('#comment-input')); await click('#comment-input'); await type('Agreed'); await t('typed', () => js(`document.querySelector('#comment-input').value === 'Agreed'`)); await click('#comment-post'); await t('posted', async () => (await text('#thread-count')).startsWith('2 open')); await shot('comments'); });
await behaviour('comments: dialog', async t => { await click('#resolve-1'); await t('it opens', () => has('#dialog')); await sleep(300); await click('#resolve-yes'); await t('resolved', async () => (await text('#thread-count')).includes('1 resolved')); });

console.log(JSON.stringify({ server_one_tab_mb: one.current_mb, server_ten_tabs_mb: ten.current_mb, per_tab_mb: +((ten.current_mb - one.current_mb) / 9).toFixed(2), server_peak_mb: (await memory()).peak_mb }));
done(lines.every(l => l.ok) ? 0 : 1);
