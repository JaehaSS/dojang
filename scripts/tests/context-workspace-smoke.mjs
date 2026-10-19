import assert from 'node:assert/strict';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import react from '@vitejs/plugin-react';

// Render App itself. Only the native boundary is replaced with labelled sample data.
const directory = path.resolve('.praxis/verification/context-workspace');
const fixture = path.join(directory, 'fixture');
await mkdir(fixture, { recursive: true });
const now = Math.floor(Date.now() / 1000);
const tasks = [
  { id: 701, agent: 'codex', model: 'gpt-6-sol', instruction: '컨텍스트 구성 화면 구현' },
  { id: 702, agent: 'claude', model: 'opus', instruction: '컨텍스트 수집 범위 검토' },
].map(task => ({ ...task, repo: '/example/dojang', branch: 'feature/context-details', base: 'main', worktree_path: '/example/dojang/context-details', state: 'Working', mode: 'conversation', created_at: now, updated_at: now, ensemble: '' }));
const sampleDay = new Date().toLocaleDateString('sv-SE');
const todayItems = Array.from({ length: 20 }, (_, index) => ({
  id: index + 1, day: sampleDay, title: '예시 오늘 할 일 ' + (index + 1),
  note: null, status: index < 2 ? 'done' : 'open', position: index,
  repo: '/example/dojang', task_id: null, source: 'manual', source_ref: null,
  created_at: now, updated_at: now, done_at: null, carried_from: null,
}));
const knowCards = [{
  id: 1, task_id: 701, kind: 'you_should_know', topic: '예시', title: '예시 알아둘 것',
  body: '이 본문은 예시 데이터로 긴 카드의 배치와 내부 스크롤을 검증합니다. '.repeat(10),
  evidence: '예시 실행 기록', turn_label: '컨텍스트 구성', created_at: now,
  seen_at: now, asked_at: null,
}];
const metrics = { version: 1, user_bytes: 146, sent_user_bytes: 3926, instruction_bytes: 2014, skill_added_bytes: 920, goal_added_bytes: 0, capsule_added_bytes: 410, vault_added_bytes: 436 };
const records = (agent) => [
  { kind: 'model_snapshot', source: 'invocation', requested: agent === 'codex' ? 'gpt-6-sol' : 'opus' },
  { kind: 'user', text: 'Codex와 Claude의 컨텍스트를 세심하게 분류하고, 앱에서 확인할 수 있는 화면을 구성해 주세요.', prompt_metrics: metrics },
  { kind: 'text', text: '전체 사용량과 입력·도구 기록의 크기를 구분해 표시하겠습니다. 서브에이전트의 컨텍스트는 별도로 확인할 수 있도록 구성하겠습니다.' },
  { kind: 'tool_use', name: 'Read', summary: '기존 세션 화면과 입력 계측 구조 확인', tool_id: 'read-1' },
  { kind: 'tool_result', tool_use_id: 'read-1', result_chars: 12480, text: '관련 구성요소와 계측 필드를 확인했습니다.' },
  { kind: 'tool_use', name: 'Grep', summary: '컨텍스트 관측 이벤트 사용 위치 확인', tool_id: 'search-1' },
  { kind: 'tool_result', tool_use_id: 'search-1', result_chars: 2610, text: '사용량·모델 관측과 저장된 입력 기록을 찾았습니다.' },
  { kind: 'tool_use', name: 'Bash', summary: '화면 테스트 실행', tool_id: 'bash-1' },
  { kind: 'tool_result', tool_use_id: 'bash-1', result_chars: 1320, text: '예시 테스트 결과입니다.' },
  { kind: 'text', text: '컨텍스트 구성 화면을 연결했습니다. 입력창 오른쪽의 컨텍스트 잔여량을 누르면 상세창이 열립니다. 기록을 토큰으로 환산하지 않고, 바이트·문자를 각각 표시합니다.' },
  { kind: 'context_usage', source: agent === 'codex' ? 'app_server' : 'claude_message', valid: true, context_tokens: agent === 'codex' ? 129200 : 62000, context_window: agent === 'codex' ? 258400 : null, observed_at: now },
  { kind: 'result', cost_usd: 0, num_turns: 1, tokens_in: 0, tokens_out: 0 },
];
await writeFile(path.join(fixture, 'index.html'), '<link rel="icon" href="data:,"><div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, 'core.ts'), `
const tasks = ${JSON.stringify(tasks)};
const histories = ${JSON.stringify(Object.fromEntries(tasks.map(task => [task.id, records(task.agent)])))};
const todayItems = ${JSON.stringify(todayItems)};
const knowCards = ${JSON.stringify(knowCards)};
window.__contextWorkspace = { calls: [] };
export async function invoke(command, args = {}) {
  window.__contextWorkspace.calls.push(command);
  if (command === 'task_list') return tasks;
  if (command === 'convo_history') return { items: histories[args.id] || [], busy: false };
  if (command === 'today_list') return todayItems;
  if (command === 'know_cards_list') return args.taskId === 701 ? knowCards : [];
  if (command === 'notification_source_page') return { source_id: 'sample', after: null, cursor: 0, watermark: 0, results: [] };
  if (command.startsWith('notification_')) return { items: [], sources: [], enabled: false, delivery_error: null };
  if (command === 'attention_snapshot') return { source_id: 'sample', observed_at: ${now}, items: [] };
  if (command === 'usage_snapshot') return { vendors: [] };
  if (command === 'interaction_snapshot') return { enabled: false, phase: 'idle', items: [] };
  if (command === 'knowledge_vaults_get') return { vaults: [] };
  if (command === 'default_shell') return { cmd: '/bin/zsh', args: ['-l'] };
  if (command === 'agent_models_get') return { codex: 'gpt-6-sol', claude: 'opus' };
  if (command === 'convo_status') return { busy: false, task_id: args.id };
  if (command === 'task_diff') return { files: [], baseline: { kind: 'pinned' } };
  if (command.includes('diff_stat')) return '';
  if (command.includes('tool_cost') || command === 'task_context_diff') return null;
  if (command === 'font_settings_get' || command === 'editor_settings_get') throw new Error('Use default display settings in the sample');
  if (command === 'debate_round_cap_get') return 3;
  if (command.endsWith('_enabled_get') || command === 'use_worktree_get') return false;
  return [];
}
export const convertFileSrc = value => value;
export class Channel { onmessage = () => {}; }
`);
await writeFile(path.join(fixture, 'event.ts'), 'export const listen = async () => () => {}; export const emit = async () => {}; export const emitTo = async () => {};');
await writeFile(path.join(fixture, 'window.ts'), 'export const getCurrentWindow = () => ({ label: "sample", close: async () => {}, onCloseRequested: async () => () => {} });');
await writeFile(path.join(fixture, 'path.ts'), 'export const homeDir = async () => "/example";');
await writeFile(path.join(fixture, 'dialog.ts'), 'export const open = async () => null; export const save = async () => null;');
await writeFile(path.join(fixture, 'opener.ts'), 'export const openUrl = async () => {}; export const openPath = async () => {}; export const revealItemInDir = async () => {};');
await writeFile(path.join(fixture, 'main.tsx'), `
import React from 'react';
import { createRoot } from 'react-dom/client';
import App from '/src/App';
import { ErrorBoundary } from '/src/components/ErrorBoundary';
import { applyTheme } from '/src/lib/themes';
import '/src/index.css';
applyTheme('praxis-dark', false);
createRoot(document.getElementById('root')).render(<ErrorBoundary><App /></ErrorBoundary>);
(window as any).__contextWorkspaceTheme = (id: string) => applyTheme(id, false);
const label = document.createElement('div');
label.textContent = '예시 데이터 · 실제 앱 배치';
Object.assign(label.style, { position: 'fixed', top: '8px', left: '50%', transform: 'translateX(-50%)', zIndex: '100', pointerEvents: 'none', padding: '4px 10px', background: 'var(--c-raised)', color: 'var(--c-text2)', border: '1px solid var(--c-border)', borderRadius: '5px', fontSize: '11px' });
document.body.appendChild(label);
`);

let server;
let browser;
const errors = [];
try {
  server = await createServer({
    configFile: false, plugins: [react()], logLevel: 'error',
    resolve: { alias: Object.fromEntries([
      ['@tauri-apps/api/core', 'core.ts'], ['@tauri-apps/api/event', 'event.ts'],
      ['@tauri-apps/api/window', 'window.ts'], ['@tauri-apps/api/path', 'path.ts'],
      ['@tauri-apps/plugin-dialog', 'dialog.ts'], ['@tauri-apps/plugin-opener', 'opener.ts'],
    ].map(([name, file]) => [name, path.join(fixture, file)])) },
    server: { host: '127.0.0.1', port: 0 },
  });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || 'chrome' });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  page.setDefaultTimeout(15000);
  await page.goto(server.resolvedUrls.local[0] + path.relative(process.cwd(), fixture) + '/index.html');
  await page.locator('[data-task-key="local:701"]').click();
  const gauge = page.getByRole('button', { name: /컨텍스트 구성 보기/ });
  await gauge.waitFor();
  await page.getByText('컨텍스트 구성 화면을 연결했습니다.', { exact: false }).waitFor();
  const composer = page.locator('footer textarea').last();
  await composer.fill('도구별 사용 기록도 펼쳐서 확인해 보자');
  await page.screenshot({ path: path.join(directory, 'workspace-closed.png') });
  await gauge.click();
  await page.waitForFunction(() => document.activeElement?.hasAttribute('data-context-summary'));
  await page.getByRole('region', { name: '컨텍스트 요약' }).waitFor();
  await page.screenshot({ path: path.join(directory, 'workspace-open.png') });
  assert.equal(await composer.inputValue(), '도구별 사용 기록도 펼쳐서 확인해 보자');
  await page.setViewportSize({ width: 1440, height: 720 });
  const rail = page.locator('.absolute.top-4.right-4').first();
  assert.deepEqual(await rail.locator(':scope > aside').evaluateAll(cards => cards.map(card => card.getAttribute('aria-label'))),
    ['플로팅 작업 정보', '컨텍스트 요약', '오늘 할 일', '알아둘 것']);
  for (const label of ['오늘 할 일', '예시 알아둘 것']) {
    assert.ok(await page.getByText(label, { exact: true }).count() > 0, 'floating rail is missing ' + label);
  }
  await rail.evaluate(element => { element.scrollTop = element.scrollHeight; });
  const todayInput = rail.getByPlaceholder('여기에 한 줄로 적기');
  await todayInput.scrollIntoViewIfNeeded();
  const todayBox = await todayInput.boundingBox();
  assert.ok(todayBox != null && todayBox.y >= 0 && todayBox.y + todayBox.height <= 720, 'low-height rail must expose the Today input');
  const knowAction = page.getByRole('button', { name: '이미 앎' });
  await knowAction.scrollIntoViewIfNeeded();
  const actionBox = await knowAction.boundingBox();
  assert.ok(actionBox != null && actionBox.y >= 0 && actionBox.y + actionBox.height <= 720, 'low-height rail must expose the Know card action');
  await page.screenshot({ path: path.join(directory, 'workspace-low-height.png') });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole('button', { name: '컨텍스트 상세 펼치기' }).click();
  const activityDetail = page.getByRole('region', { name: '컨텍스트 상세' });
  await activityDetail.getByText('2,014 B', { exact: true }).waitFor();
  const activitySummary = page.getByRole('region', { name: '컨텍스트 요약' }).last();
  await activitySummary.waitFor();
  await page.screenshot({ path: path.join(directory, 'workspace-activity-dark.png') });
  await page.evaluate(() => window.__contextWorkspaceTheme('praxis-light'));
  await page.screenshot({ path: path.join(directory, 'workspace-activity-light.png') });
  const scrolled = await activityDetail.evaluate(element => {
    const container = element.parentElement?.parentElement?.parentElement;
    if (!container) return 0;
    container.scrollTop = 500;
    return container.scrollTop;
  });
  assert.ok(scrolled > 0, 'activity detail must actually scroll before checking the sticky summary');
  assert.ok(await activitySummary.evaluate(element => {
    const rect = element.getBoundingClientRect();
    return rect.top >= 0 && rect.top < 160;
  }), 'activity summary must remain sticky while detail scrolls');
  await page.getByRole('button', { name: '컨텍스트 상세 접기' }).click();
  await activitySummary.waitFor();
  await page.getByText('예시 알아둘 것', { exact: true }).waitFor();
  await page.getByRole('complementary', { name: '작업 정보', exact: true }).waitFor();
  await page.evaluate(() => window.__contextWorkspaceTheme('praxis-dark'));
  await page.setViewportSize({ width: 640, height: 700 });
  await gauge.click();
  const detail = page.getByRole('region', { name: '컨텍스트 상세' });
  await detail.getByText('2,014 B', { exact: true }).waitFor();
  await page.screenshot({ path: path.join(directory, 'workspace-narrow-detail.png') });
  await page.locator('[data-task-key="local:702"]').click();
  await gauge.waitFor();
  await gauge.click();
  await page.getByRole('region', { name: '컨텍스트 요약' }).getByText('62,000', { exact: true }).waitFor();
  await page.screenshot({ path: path.join(directory, 'workspace-claude-open.png') });
  assert.deepEqual(errors, []);
  const result = {
    passed: ['Actual App session layout with sidebar, transcript and composer', 'Gauge focuses the floating channel summary', 'Low-height rail keeps WorkInfo Context Today Know order and Know action reachable', 'Expanded activity sidebar keeps summary sticky and preserves WorkInfo Know after collapse', 'Narrow gauge opens activity detail without a dialog', 'Opening keeps composer draft', 'Claude session uses separate observation'],
    browserErrors: errors,
    screenshots: ['workspace-closed.png', 'workspace-open.png', 'workspace-low-height.png', 'workspace-activity-dark.png', 'workspace-activity-light.png', 'workspace-narrow-detail.png', 'workspace-claude-open.png'].map(file => path.join(directory, file)),
    limitation: 'Actual App rendering with sample native IPC responses; no live CLI or repository access.',
  };
  await writeFile(path.join(directory, 'summary.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
} catch (error) {
  console.error(JSON.stringify({ browserErrors: errors }));
  throw error;
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
