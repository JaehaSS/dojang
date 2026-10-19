import assert from 'node:assert/strict';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import react from '@vitejs/plugin-react';

// Real composer, queue owner, per-session drafts and styles; deterministic Runner transport.
const directory = path.resolve('.dojang/verification/conversation-queue');
const fixture = path.join(directory, 'fixture');
await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, 'index.html'), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, 'event.ts'), 'export const listen = async () => () => {}; export const emit = async () => {}; export const emitTo = async () => {};');
await writeFile(path.join(fixture, 'main.tsx'), String.raw`
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { AgentComposer } from '/src/components/ide/AgentComposer';
import { ConversationQueue } from '/src/components/ide/ConversationQueue';
import { useConversationQueue } from '/src/components/use-conversation-queue';
import { useSessionDraft } from '/src/components/ide/useSessionDraft';
import { ConversationSubmitter } from '/src/lib/conversation-submit';
import { registerTransport } from '/src/lib/transport';
import { applyTheme } from '/src/lib/themes';
import '/src/index.css';
const task = { host: 'smoke', id: 7, state: 'Running', mode: 'conversation' };
const tasks = [task];
const receipts = new Map();
window.__submissions = [];
window.__finish = () => { task.state = 'AwaitingReview'; };
window.__theme = applyTheme;
registerTransport({ hostId: 'smoke', kind: 'remote', skillsList: async () => [], knowledgeSearch: async () => [],
 taskList: async () => tasks,
 conversationSubmit: async (taskId, id, message, images) => {
  if (!receipts.has(id)) { window.__submissions.push({ taskId, id, message, images }); task.state = 'Running'; }
  const receipt = { request_id: id, status: 'accepted', error: null }; receipts.set(id, receipt); return receipt;
 },
 conversationReceipt: async (_, id) => receipts.get(id) ?? { request_id: id, status: 'not_found', error: null },
} as any);
const submitter = new ConversationSubmitter();
function Harness() {
 const [session, select] = useState(7);
 const key = 'smoke:' + session;
 const [draft, write, consume] = useSessionDraft(key);
 const { queue, flush } = useConversationQueue({ submitter, tasks, onSending: () => {}, onAccepted: () => {}, onFailed: () => {} });
 const snapshot = queue.snapshot(key);
 return <main className="mx-auto flex h-screen max-w-3xl flex-col bg-bg text-text">
  <header className="flex gap-4 border-b border-border p-3"><button onClick={() => select(session === 7 ? 8 : 7)}>세션 전환</button><span>세션 {session}</span></header>
  <div className="flex-1 p-4 text-text-secondary">현재 요청을 처리하고 있습니다…</div>
  <footer className="border-t border-border p-3">
   <AgentComposer taskId={session} host="smoke" draftKey={key} files={[]} value={draft} onChange={write}
    label="메인에 요청" sendLabel="대기열에 추가" onHistory={() => {}} onInterrupt={() => queue.pause(key, '응답 중단 — 대기 요청 보존')}
    onSend={(text, images) => { const accepted = queue.enqueue({ host: 'smoke', id: session }, text, images); if (accepted) { consume(key, draft); void flush(); } return accepted; }}
    attachments={<ConversationQueue queue={snapshot} connected onRemove={(id) => queue.remove(key, id)} onPause={() => queue.pause(key)}
     onResume={() => { queue.resume(key); void flush(); }} />} />
  </footer>
 </main>;
}
createRoot(document.getElementById('root')!).render(<Harness />);
`);
let server, browser;
try {
  server = await createServer({ configFile: false, plugins: [react()],
    resolve: { alias: { '@tauri-apps/api/event': path.join(fixture, 'event.ts') } },
    server: { host: '127.0.0.1', port: 0 }, logLevel: 'error' });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || 'chrome' });
  const page = await browser.newPage({ viewport: { width: 960, height: 800 } });
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(server.resolvedUrls.local[0] + path.relative(process.cwd(), fixture) + '/index.html');
  const composer = page.getByRole('textbox', { name: '메인에 요청', exact: true });
  await composer.fill('첫 번째 후속 요청');
  await composer.press('Enter');
  assert.equal(await composer.inputValue(), '');
  await composer.fill('두 번째 후속 요청\n첨부된 맥락도 함께 유지');
  await page.getByRole('button', { name: '대기열에 추가', exact: true }).click();
  assert.equal(await page.locator('li').count(), 2);
  assert.deepEqual(await page.evaluate(() => window.__submissions), []);
  await composer.fill('세 번째 요청의 초안');
  await page.getByRole('button', { name: '세션 전환', exact: true }).click();
  assert.equal(await composer.inputValue(), '');
  assert.equal(await page.getByRole('region', { name: '요청 대기열' }).count(), 0);
  await page.evaluate(() => window.__finish());
  await page.waitForFunction(() => window.__submissions.length === 1);
  assert.equal(await page.evaluate(() => window.__submissions[0].message), '첫 번째 후속 요청');
  await page.getByRole('button', { name: '세션 전환', exact: true }).click();
  assert.equal(await composer.inputValue(), '세 번째 요청의 초안');
  assert.equal(await page.locator('li').count(), 1);
  await page.getByRole('button', { name: '인터럽트', exact: true }).click();
  await page.evaluate(() => window.__finish());
  await page.waitForTimeout(1_200);
  assert.equal(await page.evaluate(() => window.__submissions.length), 1);
  for (const width of [420, 960, 1440]) {
    await page.setViewportSize({ width, height: 800 });
    for (const theme of ['praxis-dark', 'praxis-light']) {
      await page.evaluate((theme) => window.__theme(theme, false), theme);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
      const send = await page.getByRole('button', { name: '대기열에 추가', exact: true }).boundingBox();
      assert.ok(send && send.x >= 0 && send.x + send.width <= width && send.y + send.height <= 800);
      await page.screenshot({ path: path.join(directory, `${width}-${theme}.png`) });
    }
  }
  await page.getByRole('button', { name: '계속 보내기', exact: true }).click();
  await page.waitForFunction(() => window.__submissions.length === 2);
  assert.equal(await page.evaluate(() => window.__submissions[1].message), '두 번째 후속 요청\n첨부된 맥락도 함께 유지');
  assert.equal(await composer.inputValue(), '세 번째 요청의 초안');
  await composer.press('Enter');
  await page.getByRole('button', { name: '대기 요청 1 삭제', exact: true }).click();
  assert.equal(await page.locator('li').count(), 0);
  assert.deepEqual(errors, []);
  const result = { passed: ['Enter/button enqueue and clear', 'FIFO and hidden-session delivery', 'preserve new draft', 'interrupt/pause/resume', 'remove pending request', 'six dark/light renders at 420/960/1440px without overflow'], errors, limitation: 'Real React components with mocked Runner transport; no native vendor CLI execution.' };
  await writeFile(path.join(directory, 'results.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
