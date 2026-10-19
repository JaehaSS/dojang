import assert from 'node:assert/strict';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import react from '@vitejs/plugin-react';

// Real conversation/card/tab components; native executor observations can be replayed.
const directory = path.resolve('.dojang/verification/codex-subagent-cards');
const fixture = path.join(directory, 'fixture');
await mkdir(fixture, { recursive: true });
const root = 'smoke-root';
const update = (child, state, status = 'available') => ({
  kind: 'subagent_update', vendor: 'codex', root_thread_id: root,
  parent_thread_id: root, child_thread_id: child, execution_id: 'smoke-execution',
  child_turn_id: 'turn-1', source_key: `${child}:${state}:${status}`,
  title: child, model: 'observed-child-model', state,
  observation_source: 'session_log', observation_status: status,
});
const publicOutput = (child) => ({ kind: 'subagent_message', parent_id: `codex:${root}:${child}`,
  source_key: `${child}:result:1`, child_turn_id: 'turn-1', text: `${child}_RESULT`, is_error: false });
const liveEvents = process.env.DOJANG_CODEX_SMOKE_EVENTS
  ? JSON.parse(await readFile(process.env.DOJANG_CODEX_SMOKE_EVENTS, 'utf8')) : null;
await writeFile(path.join(fixture, 'index.html'), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, 'event.ts'), 'export const listen = async () => () => {}; export const emit = async () => {}; export const emitTo = async () => {};');
await writeFile(path.join(fixture, 'core.ts'), 'export const invoke = async () => []; export const convertFileSrc = (v) => v;');
await writeFile(path.join(fixture, 'events.json'), JSON.stringify({ initial: [update('Alpha', 'running'), update('Beta', 'running')],
  complete: [publicOutput('Alpha'), update('Alpha', 'done'), publicOutput('Beta'), update('Beta', 'done')],
  unknown: update('Beta', 'unknown', 'missing'), live: liveEvents }));
await writeFile(path.join(fixture, 'main.tsx'), String.raw`
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ConversationView, appendConvoEvent, appendConvoItems, eventToItems } from '/src/components/ide/ConversationView';
import { SubagentView } from '/src/components/ide/SubagentView';
import { subagentRootMap, subagentThreadRootOf, subagentThreads } from '/src/lib/activity';
import { applyTheme } from '/src/lib/themes';
import data from './events.json';
import '/src/index.css';
window.__theme = applyTheme;
function Harness() {
 const [raw, writeRaw] = useState(data.initial);
 const [items, writeItems] = useState(data.initial.reduce(appendConvoEvent, []));
 const [busy, writeBusy] = useState(true);
 const [tab, writeTab] = useState(null);
 window.__push = (events, running = false) => { writeRaw(prev => [...prev, ...events]); writeItems(prev => events.reduce(appendConvoEvent, prev)); writeBusy(running); };
 window.__complete = () => window.__push(data.complete);
 window.__unknown = () => window.__push([data.unknown]);
 window.__limited = () => window.__push([{...data.initial[0],state:'done',observation_status:'limited'}]);
 window.__restore = () => { writeItems(raw.flatMap(eventToItems)); writeTab(null); };
 window.__paged = () => {
  const first = [...data.initial, data.complete[0], ...Array.from({length:997}, () => ({kind:'other'}))];
  writeItems(appendConvoItems(first.flatMap(eventToItems), data.complete.flatMap(eventToItems), false));
  writeBusy(false); writeTab(null);
 };
 window.__live = () => { writeRaw(data.live); writeItems(data.live.flatMap(eventToItems)); writeBusy(false); writeTab(null); };
 const threads = subagentThreads(items);
 const roots = subagentRootMap(items);
 const main = items.filter(item => subagentThreadRootOf(item, roots) == null);
 const selected = threads.find(thread => thread.id === tab);
 return <main className="flex h-screen flex-col bg-bg text-text">
  <header className="flex gap-3 border-b border-border p-3"><button onClick={() => writeTab(null)}>메인 대화</button><span>Codex 카드 검사</span></header>
  {tab ? <SubagentView entry={selected ?? null} items={selected?.items ?? []} parentBusy={busy} /> :
   <ConversationView items={main} busy={busy} subagents={new Map(threads.map(thread => [thread.id, thread]))} onOpenSubagent={writeTab} />}
 </main>;
}
createRoot(document.getElementById('root')!).render(<Harness />);
`);
let server, browser;
try {
  server = await createServer({ configFile: false, plugins: [react()],
    resolve: { alias: { '@tauri-apps/api/event': path.join(fixture, 'event.ts'), '@tauri-apps/api/core': path.join(fixture, 'core.ts') } },
    server: { host: '127.0.0.1', port: 0 }, logLevel: 'error' });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || 'chrome' });
  const page = await browser.newPage({ viewport: { width: 960, height: 720 } });
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(server.resolvedUrls.local[0] + path.relative(process.cwd(), fixture) + '/index.html');
  const cards = page.getByRole('button', { name: /서브 에이전트/ });
  await cards.first().waitFor();
  assert.equal(await cards.count(), 2);
  assert.equal(await page.getByRole('button', { name: '탭에서 열기 ›' }).count(), 0);
  await page.evaluate(() => window.__complete());
  await cards.first().filter({ hasText: '완료' }).waitFor();
  await cards.filter({ hasText: 'Alpha' }).click();
  await page.getByText('Alpha_RESULT', { exact: true }).waitFor();
  await page.getByRole('button', { name: '탭에서 열기 ›' }).click();
  await page.getByText('Alpha_RESULT', { exact: true }).waitFor();
  assert.equal(await cards.count(), 0);
  await page.getByRole('button', { name: '메인 대화', exact: true }).click();
  await page.evaluate(() => window.__restore());
  await cards.first().waitFor();
  assert.equal(await cards.count(), 2, 'restoring all metadata must keep one anchor per child');
  await cards.filter({ hasText: 'Alpha' }).click();
  assert.equal(await page.getByText('Alpha_RESULT', { exact: true }).count(), 1);
  await page.evaluate(() => window.__paged());
  assert.equal(await cards.count(), 2, 'remote page boundaries must keep one anchor per child');
  if (await cards.filter({hasText:'Alpha'}).getAttribute('aria-expanded') === 'false') await cards.filter({hasText:'Alpha'}).click();
  assert.equal(await page.getByText('Alpha_RESULT', { exact: true }).count(), 1);
  await page.evaluate(() => window.__push([{kind:'subagent_message',parent_id:'codex:smoke-root:Alpha',source_key:'Alpha:result:1',child_turn_id:'turn-1',text:'Alpha_RESULT',is_error:false}]));
  assert.equal(await page.getByText('Alpha_RESULT', { exact: true }).count(), 1);
  await page.evaluate(() => window.__limited());
  await page.getByText('일부 상세 기록을 확인할 수 없습니다.', {exact:true}).waitFor();
  await page.getByRole('button', {name:'탭에서 열기 ›'}).click();
  await page.getByText('일부 상세 기록을 확인할 수 없습니다.', {exact:true}).waitFor();
  await page.getByRole('button', {name:'메인 대화',exact:true}).click();
  await page.evaluate(() => window.__unknown());
  await cards.filter({ hasText: '완료 확인 불가' }).waitFor();
  for (const width of [420, 960]) {
    await page.setViewportSize({ width, height: 720 });
    for (const theme of ['praxis-dark', 'praxis-light']) {
      await page.evaluate((theme) => window.__theme(theme, false), theme);
      assert.equal(await page.getByRole('button', { name: '메인 대화', exact: true }).count(), 1);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
      await page.screenshot({ path: path.join(directory, `${width}-${theme}.png`) });
    }
  }
  if (liveEvents) {
    await page.evaluate(() => window.__live());
    await cards.first().waitFor();
    assert.equal(await cards.count(), 2);
    for (let index = 0; index < 2; index++) {
      assert.match(await cards.nth(index).innerText(), /완료/);
      await cards.nth(index).click();
    }
    await page.getByText('CARD_ALPHA_OK', { exact: true }).waitFor();
    await page.getByText('CARD_BETA_OK', { exact: true }).waitFor();
    await page.screenshot({ path: path.join(directory, 'live-executor-replay.png') });
  }
  assert.deepEqual(errors, []);
  const result = { passed: ['two collapsed cards', 'observed model and completion', 'expand result and open tab', 'restore single anchors/results', 'remote 1,000-row page boundary', 'duplicate source suppression', 'unknown completion', 'partial observation in card and tab', 'dark/light 420/960px without overflow'],
    liveExecutorReplay: Boolean(liveEvents), errors,
    limitation: 'Real React components and browser; Tauri IPC is stubbed. Live input, when present, comes from the desktop run_turn executor. Installed app replacement is not exercised.' };
  await writeFile(path.join(directory, 'results.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
