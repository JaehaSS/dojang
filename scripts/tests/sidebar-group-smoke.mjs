import assert from 'node:assert/strict';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { chromium } from 'playwright';
import { createServer } from 'vite';
import react from '@vitejs/plugin-react';

const directory = path.resolve('.dojang/verification/sidebar-group-smoke');
const fixture = path.join(directory, 'ui-fixture');
const screenshot = path.join(directory, 'nested.png');
await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, 'index.html'), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, 'main.tsx'), `
import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { SessionTaskNavigation } from '/src/components/ide/SessionTaskNavigation';
import { loadProjectGroups, saveProjectGroups } from '/src/lib/project-groups';
import '/src/index.css';
const alpha = '/workspace/alpha', beta = '/workspace/beta', gamma = '/workspace/gamma', delta = '/workspace/delta';
const repos = location.search === '?legacy' ? [alpha, beta] : [alpha, beta, gamma, delta];
const task = (id, repo) => ({ id, host:'local', repo, branch:'main', base:'main', worktree_path:repo, instruction:repo, state:'Running', created_at:id, updated_at:id, mode:'conversation' });
function Navigation() {
  const [groups, setGroups] = useState(loadProjectGroups);
  const change = next => { saveProjectGroups(next); setGroups(next); };
  return <SessionTaskNavigation tasks={repos.map((repo, index) => task(4 - index, repo))} selectedKey={null} projects={repos} onOpenTask={()=>{}} onNewInRepo={()=>{}} onDeleteTask={()=>{}} onRemoveProject={()=>{}} onDiscardOrphans={()=>{}} groups={groups} onProjectGroupsChange={change} />;
}
createRoot(document.getElementById('root')).render(<main className="bg-bg text-text" style={{width:240,height:120,overflow:'auto'}}><Navigation /></main>);
`);

const nested = {
  version: 1,
  // root/other intentionally omit parentId: they are valid legacy version-1 records.
  groups: [
    { id: 'root', name: '루트', collapsed: false },
    { id: 'child', name: '하위', collapsed: false, parentId: 'root' },
    { id: 'sibling', name: '동급', collapsed: false, parentId: 'root' },
    { id: 'other', name: '다른 루트', collapsed: false },
  ],
  assignment: { '/workspace/alpha': 'child', '/workspace/beta': 'root', '/workspace/gamma': 'sibling', '/workspace/delta': 'other' },
};

let server;
let browser;
try {
  server = await createServer({ configFile: false, plugins: [react()], server: { host: '127.0.0.1', port: 0 }, logLevel: 'error' });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || 'chrome' });
  const page = await browser.newPage({ viewport: { width: 320, height: 420 } });
  const url = server.resolvedUrls.local[0] + path.relative(process.cwd(), fixture) + '/index.html';
  const group = id => page.locator(`[data-group-box="${id}"]`);
  const groupHeader = id => group(id).locator(':scope > [data-group-header]');
  const groupBody = id => group(id).locator(':scope > [data-group-body]');
  const project = name => page.locator('[draggable="true"]:not([aria-expanded])').filter({ hasText: name }).last();
  const saved = () => page.evaluate(() => JSON.parse(localStorage.getItem('praxis-project-groups')));
  const savedGroup = async id => (await saved()).groups.find(item => item.id === id);
  const parentId = async id => (await savedGroup(id)).parentId ?? null;
  const openMenu = async (target, at) => {
    if (at) {
      await target.evaluate((node, point) => node.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: point.x, clientY: point.y })), at);
      return;
    }
    await target.scrollIntoViewIfNeeded();
    const box = await target.boundingBox();
    assert(box, 'visible menu target');
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, { button: 'right' });
  };
  const bounds = async () => {
    const box = await page.getByRole('menu').boundingBox();
    assert(box && box.x >= 8 && box.y >= 8 && box.x + box.width <= 312 && box.y + box.height <= 412);
  };
  const mouseDrag = async (source, target, edge = false) => {
    await source.scrollIntoViewIfNeeded();
    const from = await source.boundingBox();
    if (!('x' in target)) await target.scrollIntoViewIfNeeded();
    const to = 'x' in target ? target : await target.boundingBox();
    assert(from && to, 'drag source and destination are visible');
    const x = 'x' in target ? to.x : to.x + to.width / 2;
    const y = 'x' in target ? to.y : edge ? to.y + 2 : to.y + to.height / 2;
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2 + 2, from.y + from.height / 2 + 2, { steps: 2 });
    await page.mouse.move(x, y, { steps: 10 });
    await page.mouse.up();
  };
  const bodyGutter = async id => groupBody(id).evaluate(node => {
    const box = node.getBoundingClientRect();
    for (let y = Math.ceil(box.top); y < Math.floor(box.bottom); y += 1) {
      for (let x = Math.ceil(box.left); x < Math.floor(box.right); x += 1) {
        if (document.elementFromPoint(x, y) === node) return { x, y };
      }
    }
    return { x: box.right - 2, y: box.bottom - 2 };
  });
  const setMainHeight = height => page.locator('main').evaluate((node, value) => { node.style.height = `${value}px`; }, height);

  await page.goto(url);
  await page.evaluate(state => localStorage.setItem('praxis-project-groups', JSON.stringify(state)), nested);
  await page.setViewportSize({ width: 320, height: 900 });
  await page.reload();
  await group('root').waitFor();
  await setMainHeight(850);
  const layout = await page.evaluate(() => {
    const root = document.querySelector('[data-group-box="root"]');
    const body = root?.querySelector(':scope > [data-group-body]');
    const child = body?.querySelector('[data-group-box="child"]');
    const beta = Array.from(body?.querySelectorAll('[draggable="true"]:not([aria-expanded])') ?? [])
      .find(node => node.textContent?.includes('beta'));
    return {
      nested: child?.parentElement?.closest('[data-group-box]') === root,
      directSibling: beta?.closest('[data-group-box]') === root,
      overflow: (document.querySelector('main')?.scrollWidth ?? 0) > (document.querySelector('main')?.clientWidth ?? 0),
    };
  });
  assert.equal(layout.nested, true, 'group > group hierarchy renders');
  assert.equal(layout.directSibling, true, 'direct project sibling renders beside nested group');
  assert.equal(layout.overflow, false, '240px sidebar has no horizontal overflow');
  await page.locator('main').screenshot({ path: screenshot });

  // Header center reparents while keeping the descendant's project assignment intact.
  await mouseDrag(groupHeader('child'), groupHeader('other'));
  assert.equal(await parentId('child'), 'other');
  assert.equal((await saved()).assignment['/workspace/alpha'], 'child');
  // A nested body is the closest project target, rather than its outer group.
  await mouseDrag(project('beta'), groupBody('child'));
  assert.equal((await saved()).assignment['/workspace/beta'], 'child');
  await mouseDrag(groupHeader('child'), await bodyGutter('root'));
  assert.equal(await parentId('child'), 'root');
  assert.equal((await saved()).assignment['/workspace/beta'], 'child');

  // Dragging the group to the root drop zone and back keeps its descendant projects together.
  await mouseDrag(groupHeader('child'), page.locator('[data-unassigned]'));
  assert.equal(await parentId('child'), null);
  assert.equal((await saved()).assignment['/workspace/alpha'], 'child');
  assert.equal((await saved()).assignment['/workspace/beta'], 'child');
  await mouseDrag(groupHeader('child'), await bodyGutter('root'));
  assert.equal(await parentId('child'), 'root');

  await groupHeader('root').click();
  assert.equal(await group('child').count(), 0, 'collapse hides descendants');
  await page.reload();
  await group('root').waitFor();
  await setMainHeight(850);
  assert.equal(await groupHeader('root').getAttribute('aria-expanded'), 'false');
  assert.equal(await group('child').count(), 0, 'collapsed state persists after reload');
  await groupHeader('root').click();
  await group('child').waitFor();

  // Dropping a parent onto its descendant is ignored.
  await mouseDrag(groupHeader('root'), groupHeader('child'));
  assert.equal(await parentId('root'), null);
  assert.equal(await parentId('child'), 'root');
  // Header boundary reorders siblings without changing their parent.
  const orderBefore = (await saved()).groups.map(item => item.id);
  if (orderBefore.indexOf('sibling') < orderBefore.indexOf('child')) {
    await mouseDrag(groupHeader('child'), groupHeader('sibling'), true);
  } else {
    await mouseDrag(groupHeader('sibling'), groupHeader('child'), true);
  }
  const reordered = await saved();
  assert.notDeepEqual(reordered.groups.map(item => item.id), orderBefore);
  assert.equal(await parentId('sibling'), 'root');

  // Removing a root promotes its children and keeps their project assignments.
  await openMenu(groupHeader('root'));
  await page.getByRole('button', { name: /그룹 해제/ }).click();
  const removed = await saved();
  assert.equal(removed.groups.some(item => item.id === 'root'), false);
  assert.equal(await parentId('child'), null);
  assert.equal(await parentId('sibling'), null);
  assert.equal(removed.assignment['/workspace/alpha'], 'child');
  assert.equal(removed.assignment['/workspace/beta'], 'child');

  // Preserve the original create/move/reload/cancel/unassign/dissolve browser path.
  await page.setViewportSize({ width: 320, height: 420 });
  await page.evaluate(() => localStorage.clear());
  await page.goto(`${url}?legacy`);
  await openMenu(project('alpha'), { x: 310, y: 410 });
  await bounds();
  await page.getByRole('button', { name: '그룹으로 이동' }).click();
  await bounds();
  await page.getByRole('button', { name: '새 그룹…' }).click();
  await page.getByLabel('그룹 이름').fill('사내용');
  await page.getByLabel('그룹 이름').press('Enter');
  await openMenu(project('beta'));
  await page.getByRole('button', { name: '그룹으로 이동' }).click();
  await page.getByRole('menu').getByRole('button', { name: '사내용', exact: true }).click();
  const legacy = await saved();
  assert.equal(legacy.groups[0].name, '사내용');
  assert.deepEqual(legacy.assignment, { '/workspace/alpha': legacy.groups[0].id, '/workspace/beta': legacy.groups[0].id });
  await page.reload();
  await page.getByText('사내용').waitFor();
  await openMenu(project('alpha'));
  await page.getByRole('button', { name: '그룹으로 이동' }).click();
  await page.getByRole('button', { name: '새 그룹…' }).click();
  await page.getByLabel('그룹 이름').press('Escape');
  assert.deepEqual(await saved(), legacy);
  await openMenu(project('alpha'));
  await page.getByRole('button', { name: '그룹에서 빼기' }).click();
  await setMainHeight(400);
  await mouseDrag(project('alpha'), group(legacy.groups[0].id));
  assert.equal((await saved()).assignment['/workspace/alpha'], legacy.groups[0].id);
  await mouseDrag(project('alpha'), page.locator('[data-unassigned]'));
  assert.equal((await saved()).assignment['/workspace/alpha'], undefined);
  await openMenu(groupHeader(legacy.groups[0].id));
  await page.getByRole('button', { name: /그룹 해제/ }).click();
  assert.deepEqual(await saved(), { version: 1, groups: [], assignment: {} });
  console.log('PASS sidebar groups: nested drag/reparent/reject/reorder/collapse/removal/240px plus prior group scenarios');
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
