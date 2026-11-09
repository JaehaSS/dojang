import assert from 'node:assert/strict';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import react from '@vitejs/plugin-react';

// Production navigation, completed-history list and archive IPC wrappers. Native IPC is a fixture.
const directory = path.resolve('.dojang/verification/task-archive');
const fixture = path.join(directory, 'fixture');
const source = file => `/@fs/${path.resolve(file)}`;
let server;
let browser;
try {
  await mkdir(fixture, { recursive: true });
  await writeFile(path.join(fixture, 'index.html'), '<!doctype html><link rel="icon" href="data:,"><div id="root"></div><script type="module" src="/main.tsx"></script>');
  await writeFile(path.join(fixture, 'core.ts'), 'export const invoke=(command,args)=>window.invokeFixture(command,args);');
  await writeFile(path.join(fixture, 'event.ts'), 'export const listen=async()=>()=>{};');
  await writeFile(path.join(fixture, 'opener.ts'), 'export const openUrl=async url=>{window.opened.push(url)};');
  await writeFile(path.join(fixture, 'main.tsx'), `
import React,{useState} from 'react';
import {createRoot} from 'react-dom/client';
import '${source('src/index.css')}';
import {SessionTaskNavigation} from '${source('src/components/ide/SessionTaskNavigation.tsx')}';
import {CompletedTaskArchiveList} from '${source('src/components/ide/CompletedTaskArchiveList.tsx')}';
import {taskArchive,taskRestore} from '${source('src/lib/task-archive.ts')}';
window.calls=[];window.opened=[];window.records=[];
const archive={archived_at:1792970000,branch:'feature/archive',head_revision:null,worktree_path:'/missing/worktree',worktree_exists:false,result_path:'/missing/worktree/result.md',pr_url:'javascript:alert(1)'};
window.invokeFixture=async(command,args)=>{
 window.calls.push({command,args});
 if(command==='notification_snapshot')return {items:[],sources:[],enabled:false,delivery_error:null};
 if(command==='task_archive')return structuredClone(archive);
 if(command==='task_restore')return;
 throw new Error('Unexpected IPC '+command);
};
const task={id:7,host:'local',repo:'/workspace/project',branch:'feature/archive',base:'main',worktree_path:'/missing/worktree',instruction:'로컬 이력',state:'AwaitingReview',created_at:Date.now()/1000,updated_at:Date.now()/1000,mode:'conversation'};
function App(){
 const [tasks,setTasks]=useState([task,{...task,host:'runner',instruction:'동일 ID 원격'},{...task,id:8,state:'Done',instruction:'종료 이력'}]);
 const update=async(t,archived)=>{const a=archived?await taskArchive(t.id):(await taskRestore(t.id),null);setTasks(rows=>rows.map(row=>row.host===t.host&&row.id===t.id?{...row,archive:a}:row));};
 return <div className="bg-bg text-text p-4 flex gap-6"><aside className="w-80"><SessionTaskNavigation tasks={tasks} selectedKey={null} projects={['/workspace/project']} onOpenTask={t=>window.records.push(t.instruction)} onNewInRepo={()=>{}} onDeleteTask={()=>{throw new Error('unexpected deletion')}} onRemoveProject={()=>{}} onDiscardOrphans={()=>{}} onArchiveTask={t=>update(t,true)} onRestoreTask={t=>update(t,false)}/></aside><div><CompletedTaskArchiveList tasks={tasks} onOpenTask={t=>window.records.push(t.instruction)} onArchiveTask={t=>update(t,true)}/></div></div>;
}
createRoot(document.getElementById('root')).render(<App/>);
`);
  server = await createServer({ root: fixture, configFile: false, plugins: [react()], resolve: { alias: {
    '@tauri-apps/api/core': path.join(fixture, 'core.ts'),
    '@tauri-apps/api/event': path.join(fixture, 'event.ts'),
    '@tauri-apps/plugin-opener': path.join(fixture, 'opener.ts'),
  } }, server: { host: '127.0.0.1', port: 0, fs: { allow: [process.cwd()] } } });
  await server.listen();
  browser = await chromium.launch({ channel: process.env.DOJANG_SMOKE_BROWSER || 'chrome' });
  const page = await browser.newPage({ viewport: { width: 1200, height: 1000 } });
  page.setDefaultTimeout(15000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(server.resolvedUrls.local[0] + 'index.html', { waitUntil: 'domcontentloaded' });
  await page.getByText('로컬 이력', { exact: true }).first().click({ button: 'right' });
  await page.getByRole('button', { name: '보관 (기록·폴더 유지)', exact: true }).click();
  const archived = page.getByRole('region', { name: '보관한 작업', exact: true });
  await archived.getByText('워크트리 없음', { exact: true }).waitFor();
  assert.match(await archived.innerText(), /커밋 미확인/);
  assert.match(await archived.innerText(), /목록만 되돌리며 워크트리를 다시 만들지 않습니다/);
  assert.equal(await archived.getByText('PR 열기', { exact: true }).count(), 0);
  assert.equal(await page.locator('[href^="javascript:"]').count(), 0);
  assert.ok(await page.getByText('동일 ID 원격', { exact: true }).count() > 0);
  await archived.getByRole('button', { name: '로컬 이력 이력 열기', exact: true }).focus();
  await page.keyboard.press('Enter');
  assert.deepEqual(await page.evaluate(() => window.records), ['로컬 이력']);
  await archived.getByText('로컬 이력', { exact: true }).click({ button: 'right' });
  assert.ok(await page.getByRole('button', { name: '버리기 (워크트리 정리)', exact: true }).count());
  await page.keyboard.press('Escape');
  await page.screenshot({ path: path.join(directory, 'archive-list.png') });
  await archived.getByRole('button', { name: '복원', exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('[data-archive-task-key="local:7"]'));
  await page.getByText('종료 작업 보관 (1)', { exact: true }).click();
  await page.getByRole('button', { name: '종료 이력 보관', exact: true }).click();
  await archived.getByText('종료 이력', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.calls.filter(call => /delete|discard|cancel/.test(call.command)).length), 0);
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ status: 'passed', checks: ['manual archive IPC', 'separate list', 'same-id remote retained', 'missing worktree history open by keyboard', 'restore IPC', 'completed history archive', 'truthful removal label', 'unsafe PR link rejected', 'no destructive IPC'], screenshot: path.join(directory, 'archive-list.png') }, null, 2));
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
