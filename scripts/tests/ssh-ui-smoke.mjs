import assert from 'node:assert/strict';
import { mkdir, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { createServer } from 'vite';
import react from '@vitejs/plugin-react';
import { chromium } from 'playwright';

// Actual React components with a mocked native boundary. This is not a real SSH test.
const directory = path.resolve('.dojang/verification/ssh-session-workflow');
const fixture = path.join(directory, 'ui-fixture');
await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, 'index.html'), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, 'main.tsx'), `
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { SshServerPicker } from '/src/components/ide/SshServerPicker';
import '/src/index.css';
const profiles = [{name:'saved-a',host:'server.example',user:'developer',known_hosts:'/mock/known_hosts',local_port:47832}, {name:'direct-host',host:'',user:'',known_hosts:'',local_port:0,transport:'direct',endpoint:'https://runner.example'}];
let draft;
const calls = [];
const inspection = (phase) => ({id:'inspection-1',profile_name:draft?.name,phase,message:phase === 'unsupported' ? '지원되는 배포 파일이 없습니다.' : '',fingerprints:['SHA256:mock-server-key'],platform:'ubuntu-24.04',architecture:'x86_64',repositories:['/home/developer/projects/demo'],default_root:'/home/developer/projects'});
window.__sshTest = {calls,mode:'normal',resolve:null};
window.__TAURI_INTERNALS__ = {invoke:async (command,args) => {
  calls.push({command,args});
  switch(command) {
    case 'remote_profiles_list': return profiles;
    case 'remote_profile_options_get': return {display_name:'개발 서버',ssh_port:22,identity_file:null,port_mode:'auto',workspace_root:null,default_repo:null};
    case 'remote_pairing_token_get': return 'mock-only-not-a-credential';
    case 'remote_connect': return {connected:true,endpoint:'http://127.0.0.1:47832'};
    case 'remote_server_inspect': draft=args.draft; if(window.__sshTest.mode==='deferred') return new Promise(resolve=>{window.__sshTest.resolve=()=>resolve(inspection('needs_trust'))}); return inspection('needs_trust');
    case 'remote_server_trust': return inspection(window.__sshTest.mode==='unsupported'?'unsupported':'needs_setup');
    case 'remote_server_plan': return {id:'plan-1',inspection_id:'inspection-1',profile_name:draft.name,root:args.workspaceRoot,version:'1.2.3',sha256:'a'.repeat(64),install_paths:['~/.local/bin/praxis-runner','~/.config/praxis/runner.toml'],execution_policy:'require_approval',linger:true,expires_at:Date.now()/1000+300};
    case 'remote_server_install': return inspection('ready');
    case 'remote_server_save': {const profile={...profiles[0],name:draft.name,host:draft.host,user:draft.user}; profiles.push(profile);return profile;}
    case 'remote_server_cancel': return;
    default: throw new Error('Unexpected native command: '+command);
  }
}};
window.fetch = async () => new Response(JSON.stringify({status:'ok',repository_roots:['/home/developer/projects']}),{headers:{'content-type':'application/json'}});
function Harness(){const [host,setHost]=useState('local');return <main className="bg-bg text-text min-h-screen p-8"><SshServerPicker host={host} onPick={setHost}/><output aria-label="選択先">{host}</output></main>}
createRoot(document.getElementById('root')).render(<Harness/>);
`);
let server;
let browser;
let page;
try {
  server = await createServer({ configFile: false, plugins: [react()], server: { host: '127.0.0.1', port: 0 }, logLevel: 'error' });
  await server.listen();
  // Use the installed browser. Never install browser/tool binaries during verification.
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || 'chrome' });
  page = await browser.newPage({ viewport: { width: 960, height: 760 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const url = server.resolvedUrls.local[0] + path.relative(process.cwd(), fixture) + '/index.html';
  const load = () => page.goto(url);
  const calls = () => page.evaluate(() => window.__sshTest.calls);
  const add = async () => {
    await page.getByRole('button', { name: /SSH/ }).click();
    await page.getByRole('button', { name: '서버 추가…' }).click();
    await page.getByLabel('서버 주소').fill('new.example');
    await page.getByLabel('사용자명').fill('developer');
    await page.getByRole('button', { name: '연결 확인' }).click();
  };
  await load();
  await page.getByRole('button', { name: /SSH/ }).click();
  await page.getByRole('menuitem', { name: /개발 서버/ }).click();
  await page.waitForFunction(() => document.querySelector('output')?.textContent === 'saved-a' && window.__sshTest.calls.some(call => call.command === 'remote_connect'));
  await page.getByRole('button', { name: /SSH/ }).click();
  await page.getByRole('menuitem', { name: '이 컴퓨터' }).click();
  assert(!(await calls()).some(call => call.command === 'remote_disconnect'));
  console.log('PASS saved disconnected server: two clicks; local selection preserves remote connection');
  await page.getByRole('button', { name: /직접/ }).click();
  await page.getByRole('menuitem', { name: /direct-host/ }).click();
  await page.waitForFunction(() => document.querySelector('output')?.textContent === 'direct-host');
  assert(!(await calls()).some(call => call.command.startsWith('remote_server_')));
  console.log('PASS existing direct profile remains selectable with no SSH setup call');
  await load();
  await page.getByRole('button', { name: /SSH/ }).focus();
  await page.keyboard.press('ArrowDown');
  await page.getByRole('menu').waitFor();
  assert.equal(await page.getByRole('menuitem').evaluate(element => element === document.activeElement), true);
  await page.keyboard.press('Escape');
  assert.equal(await page.getByRole('button', { name: /SSH/ }).evaluate(element => element === document.activeElement), true);
  console.log('PASS keyboard SSH menu: ArrowDown enters; Escape closes and restores trigger focus');
  await load();
  await add();
  await page.getByRole('button', { name: '확인하고 계속' }).click();
  await page.getByLabel('작업 폴더').fill('/home/developer/projects');
  await page.getByRole('button', { name: '설치 계획 확인' }).click();
  await page.getByRole('button', { name: '동의하고 준비' }).waitFor();
  await page.screenshot({ path: path.join(directory, 'ssh-install-consent.png') });
  await page.evaluate(() => document.documentElement.classList.add('dark'));
  await page.screenshot({ path: path.join(directory, 'ssh-install-consent-dark.png') });
  assert(await page.getByRole('dialog').evaluate(element => element.scrollWidth <= element.clientWidth));
  await page.getByRole('button', { name: '동의하고 준비' }).click();
  await page.getByRole('button', { name: '이 서버 사용' }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  assert.equal((await calls()).filter(call => call.command === 'remote_server_install').length, 1);
  console.log('PASS registration: trust, canonical-root plan, consent, install, save');
  await load();
  await page.evaluate(() => { window.__sshTest.mode = 'deferred'; });
  await add();
  await page.getByRole('button', { name: '취소' }).click();
  await page.evaluate(() => window.__sshTest.resolve());
  await page.waitForFunction(() => window.__sshTest.calls.some(call => call.command === 'remote_server_cancel'));
  assert(!(await calls()).some(call => call.command === 'remote_server_save'));
  console.log('PASS cancel during inspection: late result cancelled, no profile saved');
  await load();
  await page.evaluate(() => { window.__sshTest.mode = 'unsupported'; });
  await add();
  await page.getByRole('button', { name: '확인하고 계속' }).click();
  await page.getByText('지원되는 배포 파일이 없습니다.').waitFor();
  assert.equal(await page.getByRole('button', { name: '동의하고 준비' }).count(), 0);
  await page.getByRole('button', { name: '취소' }).focus();
  await page.keyboard.press('Escape');
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  assert.equal(await page.getByRole('button', { name: /SSH/ }).evaluate(element => element === document.activeElement), true);
  console.log('PASS unsupported release: no install action; Escape dismisses dialog');
  assert.deepEqual(errors, []);
} catch (error) {
  if (page) await page.screenshot({ path: path.join(directory, 'ssh-ui-failure.png') }).catch(() => {});
  throw error;
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
