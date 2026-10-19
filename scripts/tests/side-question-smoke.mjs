import assert from 'node:assert/strict';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import react from '@vitejs/plugin-react';

// Real components with a deterministic text-only transport. This does not claim native CLI coverage.
const directory = path.resolve('.dojang/verification/side-question');
const fixture = path.join(directory, 'fixture');
await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, 'index.html'), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, 'main.tsx'), String.raw`
import React, { useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { WorkspaceSplit } from '/src/components/ide/WorkspaceSplit';
import { CodeColumnTabs } from '/src/components/ide/CodeColumnTabs';
import { SideQuestionPanel } from '/src/components/ide/SideQuestionPanel';
import { AgentComposer } from '/src/components/ide/AgentComposer';
import { QuestionReferenceAttachments } from '/src/components/ide/QuestionReferenceAttachments';
import { useQuestionReferences } from '/src/components/ide/useQuestionReferences';
import { useSessionDraft } from '/src/components/ide/useSessionDraft';
import { formatQuestionReferences } from '/src/lib/side-question';
import { registerTransport } from '/src/lib/transport';
import { requestComposerFocus } from '/src/lib/composer-focus';
import { applyTheme } from '/src/lib/themes';
import '/src/index.css';
registerTransport({ hostId: 'smoke', kind: 'remote', skillsList: async () => [], knowledgeSearch: async () => [] } as any);
window.__submissions = [];
window.__theme = applyTheme;
let turns = [];
const snapshot = () => ({ task_id: 7, thread_id: 1, generation: 0, model: 'Claude · mock', supported: true, reason: null, turns });
function Harness() {
 const [scope, setScope] = useState('smoke:7');
 const [draft, setDraft, consume] = useSessionDraft(scope);
 const refs = useQuestionReferences(scope);
 const [open, setOpen] = useState(false);
 const [tab, setTab] = useState('question');
 const [mode, setMode] = useState('tabs');
 const api = useMemo(() => ({ read: async () => snapshot(), send: async (input) => {
   turns = [...turns, { ...input, id: turns.length + 1, state: 'completed', answer: '검증한 선택 답변입니다.\n\n코드와 긴 한글 문장도 메인에 자동 전달되지 않습니다.\n\n~~~ts\nconst scoped = true;\n~~~', error: null, created_at: 1 }]; return snapshot();
 }, cancel: async () => snapshot(), reset: async () => { turns = []; return snapshot(); } }), [scope]);
 const back = () => { setOpen(false); requestAnimationFrame(() => requestComposerFocus(7)); };
 const hidden = open && tab === 'question' && mode === 'tabs';
 return <main className="mx-auto flex h-screen flex-col overflow-hidden bg-bg text-text" style={{ width: 'calc(100% - 240px)' }} data-shell data-mode={mode}>
  <header className="flex gap-4 border-b border-border p-2"><button onClick={() => { setTab('question'); setOpen(true); }}>따로 질문 열기</button><button onClick={() => setScope(scope === 'smoke:7' ? 'other:7' : 'smoke:7')}>호스트 전환</button></header>
  <WorkspaceSplit codeOpen={open} onCloseCode={back} onModeChange={setMode} session={<div className="p-4">메인 작업 대화</div>} code={<CodeColumnTabs active={tab} onActivate={setTab} previewAvailable={false} editorPoppedOut={false} activity={<div>작업정보</div>} file={<div>파일</div>} diff={<div>변경</div>} question={<SideQuestionPanel key={scope} sessionKey={scope} api={api} active={open && tab === 'question'} onBack={back} onAttach={(ref) => { refs.attach(ref); if (mode === 'tabs') setOpen(false); requestAnimationFrame(() => requestComposerFocus(7)); }} />} />} />
  <footer hidden={hidden} inert={hidden} data-main-composer className="p-3 border-t border-border">
   <AgentComposer taskId={7} host="smoke" draftKey={scope} value={draft} onChange={setDraft} files={[]} label="메인에 요청" sendLabel="메인에 보내기" active={!hidden} onInterrupt={() => {}} onHistory={() => {}} attachments={<QuestionReferenceAttachments references={refs.references} onChange={refs.setReferences} />} onSend={async (text) => { const submitted = structuredClone(refs.references); window.__submissions.push(text + '\n' + formatQuestionReferences(submitted)); consume(scope, draft); refs.consume(scope, submitted); return true; }} />
  </footer>
 </main>;
}
createRoot(document.getElementById('root')!).render(<Harness />);
`);
let server, browser;
const outcomes = [];
try {
  server = await createServer({ configFile: false, plugins: [react()], server: { host: '127.0.0.1', port: 0 }, logLevel: 'error' });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || 'chrome' });
  const page = await browser.newPage({ viewport: { width: 960, height: 900 } });
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  await page.goto(server.resolvedUrls.local[0] + path.relative(process.cwd(), fixture) + '/index.html');
  const main = page.getByRole('textbox', { name: '메인에 요청', exact: true });
  await main.fill('이 답변을 고려해서 검증을 추가해줘');
  await page.getByRole('button', { name: '따로 질문 열기' }).click();
  const question = page.getByRole('textbox', { name: '따로 질문', exact: true });
  await question.fill('메인 세션에 영향을 주지 않고 확인할 수 있을까?');
  assert.equal(await main.isVisible(), false);
  assert.equal(await page.locator('[data-main-composer]').getAttribute('inert'), '');
  await page.getByRole('button', { name: '질문 보내기', exact: true }).click();
  await page.getByRole('button', { name: '참고자료로 선택', exact: true }).click();
  const edit = page.getByRole('textbox', { name: '메인에 첨부할 답변' });
  await edit.fill('선택해서 편집한 답변');
  await page.getByRole('button', { name: '메인 입력창에 첨부', exact: true }).click();
  await main.waitFor({ state: 'visible' });
  await page.waitForFunction(() => document.activeElement?.getAttribute('aria-label') === '메인에 요청');
  assert.equal(await main.inputValue(), '이 답변을 고려해서 검증을 추가해줘');
  assert.deepEqual(await page.evaluate(() => window.__submissions), []);
  await page.getByRole('button', { name: '메인에 보내기', exact: true }).click();
  const submitted = await page.evaluate(() => window.__submissions);
  assert.equal(submitted.length, 1);
  assert.match(submitted[0], /선택해서 편집한 답변/);
  assert.doesNotMatch(submitted[0], /const scoped/);
  outcomes.push('960px: isolated send, selected edit, no auto send, main draft/focus restored, one explicit submission');
  await page.getByRole('button', { name: '따로 질문 열기' }).click();
  // Actual central widths, in sequence, exercise hysteresis.
  for (const [width, expected] of [[839, 'tabs'], [840, 'tabs'], [899, 'tabs'], [900, 'split'], [899, 'split'], [840, 'split'], [839, 'tabs']]) {
    await page.setViewportSize({ width: width + 240, height: 900 });
    await page.waitForFunction((expected) => document.querySelector('[data-shell]')?.getAttribute('data-mode') === expected, expected);
    assert.equal(await main.isVisible(), expected === 'split');
  }
  outcomes.push('839/840/899/900 central-width hysteresis and main composer visibility');
  for (const width of [960, 1280, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    for (const theme of ['praxis-dark', 'praxis-light']) {
      for (const density of ['compact', 'relaxed']) {
        await page.evaluate(({ theme, density }) => { window.__theme(theme, false); document.documentElement.dataset.density = density; }, { theme, density });
        await question.waitFor({ state: 'visible' });
        const send = await page.getByRole('button', { name: '질문 보내기', exact: true }).boundingBox();
        assert.ok(send && send.x >= 0 && send.x + send.width <= width && send.y + send.height <= 900);
        const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
        assert.equal(overflow, false);
        await page.screenshot({ path: path.join(directory, `${width}-${theme}-${density}.png`) });
      }
    }
  }
  outcomes.push('12 renders: 960/1280/1440, light/dark, compact/relaxed; visible controls and no page overflow');
  assert.deepEqual(errors, []);
  await writeFile(path.join(directory, 'results.json'), JSON.stringify({ outcomes, errors, limitation: 'Composed real components, mocked transport; native execution verified separately.' }, null, 2));
  console.log(JSON.stringify({ outcomes, screenshots: directory }, null, 2));
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
