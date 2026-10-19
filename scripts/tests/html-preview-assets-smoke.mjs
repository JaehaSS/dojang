import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".praxis/verification/html-preview-assets");
const fixture = path.join(directory, "fixture");
const source = (file) => `/@fs/${path.resolve(file)}`;
const svg = "data:image/svg+xml;base64," + Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><rect width="24" height="24" fill="red"/></svg>').toString("base64");
const document = '<!doctype html><html lang="ko"><head><link rel="stylesheet" href="css/main.css"></head><body><h1 id="heading">정적 HTML 미리보기</h1><div class="badge">CSS 배경</div><picture><source srcset="logo.svg 1x"><img id="logo" src="logo.svg" onload="parent.bad=true"></picture><script>parent.bad=true</script></body></html>';
let server;
let browser;
try {
  await mkdir(fixture, { recursive: true });
  await writeFile(path.join(fixture, "index.html"), '<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src \'self\'; style-src \'self\' \'unsafe-inline\'; img-src \'self\' data:; script-src \'self\' \'unsafe-inline\'; connect-src \'self\' ws:"><link rel="icon" href="data:,"><div id="root"></div><script type="module" src="./main.tsx"></script>');
  await writeFile(path.join(fixture, "tauri-core.ts"), `export const invoke = async (command, args) => {
    if (command === "fs_read") return window.readFixture("task:" + args.id, args.path);
    if (command === "fs_write") { window.diskHtml = args.content; window.saved = args.content; return 1; }
    throw new Error("unexpected invoke: " + command);
  };`);
  await writeFile(path.join(fixture, "tauri-opener.ts"), "export const openUrl = async () => undefined;");
  // Source switching uses a controlled textarea; preview and file-read wiring are the production code.
  await writeFile(path.join(fixture, "monaco.tsx"), `import React from "react";
    export default ({value, onChange}) => <textarea aria-label="source" value={value} onChange={event => onChange(event.target.value)} />;`);
  await writeFile(path.join(fixture, "monaco-language.ts"), 'export const langFromPath = () => "html";');
  await writeFile(path.join(fixture, "main.tsx"), `
import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import "${source("src/index.css")}";
import { EditorSplitView } from "${source("src/components/ide/EditorSplitView.tsx")}";
import { useWorkspaceFiles } from "${source("src/components/ide/useWorkspaceFiles.ts")}";
const original = ${JSON.stringify(document)};
const assets = {
  "docs/css/main.css": {kind:"text", content:'@import "nested/theme.css" layer(report) supports(display: block) screen; h1 { color: rgb(12, 34, 56); margin: 20px }'},
  "docs/css/nested/theme.css": {kind:"text", content:'.badge { background-image: url(../../logo.svg); width: 90px; height: 30px; margin:20px }'},
  "docs/logo.svg": {kind:"image", content:${JSON.stringify(svg)}}
};
window.reads = []; window.saved = null; window.bad = false; window.diskHtml = original;
window.readFixture = async (scope, path) => {
  window.reads.push({scope, path});
  if (path === "docs/page.html") return {kind:"text",content:window.diskHtml,mtime:1};
  if (!assets[path]) throw new Error("missing " + path);
  return {...assets[path], mtime:1};
};
const projectRead = path => window.readFixture("project:/project", path);
function Harness() {
  const [scope, setScope] = useState("task");
  const source = useMemo(() => scope === "project" ? {
    key:"project:/project", tree:async()=>[], read:projectRead,
    write:async (_path, value)=>{window.diskHtml=value;window.saved=value;return 1;}
  } : null, [scope]);
  const files = useWorkspaceFiles({task:scope === "task" ? {host:"local",id:7} : null, source, onError:message=>{throw new Error(message);}});
  useEffect(()=>{void files.openFile("docs/page.html");},[scope,files.openFile]);
  window.changeScope = setScope;
  window.changeHtml = value => files.changeFile("docs/page.html", value);
  window.reloadHtml = () => { assets["docs/css/main.css"].content = 'h1{color:rgb(90, 80, 70)}'; return files.reloadFile("docs/page.html"); };
  return <main style={{display:"flex", flexDirection:"column", width:960, height:640}}>
    <EditorSplitView taskId={scope === "task" ? 7 : null} host="local" rootPath="/project" windowId="smoke"
      readHtmlAsset={scope === "project" ? projectRead : undefined} files={files.openFiles} activeKey={files.activeKey}
      dark={false} ownsWindow onSelect={files.setActiveKey} onClose={files.closeTab} onChange={files.changeFile}
      onSave={files.saveFile} onReload={files.reloadFile} onOpenPath={()=>{}} onRevealPath={()=>{}} />
  </main>;
}
createRoot(document.getElementById("root")).render(<Harness />);
`);
  server = await createServer({
    configFile: false, root: fixture, plugins: [react()], logLevel: "error",
    resolve: { alias: [
      { find: "@tauri-apps/api/core", replacement: path.join(fixture, "tauri-core.ts") },
      { find: "@tauri-apps/plugin-opener", replacement: path.join(fixture, "tauri-opener.ts") },
      { find: "@monaco-editor/react", replacement: path.join(fixture, "monaco.tsx") },
      { find: /^(?:.*\/)?lib\/monaco(?:\.ts)?$/, replacement: path.join(fixture, "monaco-language.ts") },
    ] }, server: { host: "127.0.0.1", port: 0 },
  });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome" });
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  page.setDefaultTimeout(10_000);
  const errors = [];
  const remoteRequests = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => { if (request.url().startsWith("https://static.test/")) remoteRequests.push(request.url()); });
  await page.goto(server.resolvedUrls.local[0] + "index.html", { waitUntil: "domcontentloaded" });
  const frame = page.frameLocator('iframe[title="page.html"]');
  await frame.locator("#heading").waitFor();
  await page.waitForFunction(() => document.querySelector("iframe")?.getAttribute("srcdoc")?.includes("data:image/svg+xml"));
  assert.equal(await frame.locator("#heading").evaluate((element) => getComputedStyle(element).color), "rgb(12, 34, 56)");
  assert.match(await frame.locator(".badge").evaluate((element) => getComputedStyle(element).backgroundImage), /data:image\/svg\+xml/);
  assert.equal(await frame.locator("#logo").evaluate(async (element) => { await element.decode(); return element.naturalWidth; }), 24);
  assert.equal(await frame.locator("body").evaluate(() => document.compatMode), "CSS1Compat");
  assert.equal(await page.locator("iframe").getAttribute("sandbox"), "");
  assert.equal(await page.evaluate(() => window.bad), false);
  assert.deepEqual(await page.evaluate(() => window.reads.filter(({path}) => path !== "docs/page.html").map(({scope}) => scope)), ["task:7", "task:7", "task:7"]);
  await page.screenshot({ path: path.join(directory, "task-preview.png") });
  await page.getByRole("button", { name: "저장", exact: true }).click();
  assert.equal(await page.evaluate(() => window.saved), document);
  await page.getByRole("button", { name: "소스", exact: true }).click();
  const edited = document.replace("정적 HTML 미리보기", "수정한 HTML 미리보기");
  await page.getByRole("textbox", { name: "source" }).fill(edited);
  await page.getByRole("button", { name: "프리뷰", exact: true }).click();
  await page.waitForFunction(() => document.querySelector("iframe")?.getAttribute("srcdoc")?.includes("수정한 HTML 미리보기"));
  await frame.locator("#heading").filter({ hasText: "수정한 HTML 미리보기" }).waitFor();
  assert.equal(await frame.locator("#heading").evaluate(element => getComputedStyle(element).color), "rgb(12, 34, 56)");
  assert.equal(await frame.locator("#logo").evaluate(async element => { await element.decode(); return element.naturalWidth; }), 24);
  await page.screenshot({ path: path.join(directory, "edited-preview.png") });
  await page.getByRole("button", { name: "저장", exact: true }).click();
  assert.equal(await page.evaluate(() => window.saved), edited);
  await page.evaluate(() => { window.reads = []; window.changeScope("project"); });
  await page.waitForFunction(() => window.reads.filter(({path}) => path !== "docs/page.html").length === 3 && document.querySelector("iframe")?.getAttribute("srcdoc")?.includes("data:image/svg+xml"));
  assert.deepEqual(await page.evaluate(() => window.reads.filter(({path}) => path !== "docs/page.html").map(({scope}) => scope)), ["project:/project", "project:/project", "project:/project"]);
  assert.equal(await frame.locator("#heading").evaluate((element) => getComputedStyle(element).color), "rgb(12, 34, 56)");
  await page.evaluate(() => window.reloadHtml());
  await page.waitForFunction(() => document.querySelector("iframe")?.getAttribute("srcdoc")?.includes("90, 80, 70"));
  assert.equal(await frame.locator("#heading").evaluate((element) => getComputedStyle(element).color), "rgb(90, 80, 70)");
  await page.evaluate(() => window.changeHtml('<h1 id="heading">누락 확인</h1><link rel="stylesheet" href="missing.css"><img src="../../outside.png"><img src="https://static.test/beacon.png"><iframe src="https://static.test/page"></iframe>'));
  await page.getByRole("status").filter({ hasText: "불러오지 못했습니다" }).waitFor();
  await frame.locator("#heading").filter({ hasText: "누락 확인" }).waitFor();
  assert.equal(await page.evaluate(() => window.bad), false);
  assert.deepEqual(remoteRequests, [], "resource preparation must not issue remote requests outside the sandbox");
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ status: "passed", checks: ["task/project readers", "CSS/import/background", "picture/srcset image decoding", "standards mode", "script/event-handler denial", "source edit and unchanged save", "asset reload", "missing-resource notice", "remote requests denied"], screenshot: path.join(directory, "task-preview.png") }, null, 2));
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
