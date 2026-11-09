import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

// Only the desktop IPC boundary is faked. The panel, preparation, asset bundler and iframe are real.
const directory = path.resolve(".dojang/verification/artifact-review");
const fixture = path.join(directory, "fixture");
const source = file => `/@fs/${path.resolve(file)}`;
const original = '<!doctype html><html><head><meta http-equiv="refresh" content="0;url=https://artifact.invalid/redirect"><link rel="stylesheet" href="theme.css"></head><body><h1>처음 시안</h1><img id="logo" src="logo.svg" onload="parent.pwned=true"><script>parent.pwned=true</script><img src="https://artifact.invalid/beacon.png"></body></html>';
const image = "data:image/svg+xml;base64," + Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><rect width="24" height="24" fill="red"/></svg>').toString("base64");
let browser;
let server;
try {
  await mkdir(fixture, { recursive: true });
  await writeFile(path.join(fixture, "index.html"), '<!doctype html><link rel="icon" href="data:,"><div id="root"></div><script type="module" src="./main.tsx"></script>');
  await writeFile(path.join(fixture, "core.ts"), `export const invoke = (command,args) => window.invokeFixture(command,args);`);
  await writeFile(path.join(fixture, "main.tsx"), `
import React, {useState} from "react";
import {createRoot} from "react-dom/client";
import "${source("src/index.css")}";
import {ArtifactReviewPanel} from "${source("src/components/ide/ArtifactReviewPanel.tsx")}";
const disk = {
  "docs/page.html": {kind:"text",content:${JSON.stringify(original)},mtime:1},
  "docs/theme.css": {kind:"text",content:"h1 {color:rgb(12, 34, 56)}",mtime:1},
  "docs/logo.svg": {kind:"image",content:${JSON.stringify(image)},mtime:1}
};
window.pwned=false; window.reads=[]; window.saved=[]; window.attached=[]; window.opened=[];
const metadata = ({content,preview_html,...row}) => row;
window.invokeFixture = async (command,args) => {
  if(command==="fs_read") {window.reads.push(args.path);if(!disk[args.path])throw new Error("missing "+args.path);return {...disk[args.path]};}
  if(command==="review_artifact_list")return window.saved.filter(row=>row.task_id===args.id).map(metadata);
  if(command==="review_artifact_get")return {...window.saved.find(row=>row.id===args.artifactId&&row.task_id===args.id)};
  if(command==="review_artifact_save") {
    const input=args.input;
    if(disk[input.relative_path]?.content!==input.expected_content)throw new Error("source changed");
    for(const resource of input.resources)if(disk[resource.relative_path]?.content!==resource.expected_content)throw new Error("resource changed");
    const id=window.saved.length+1;
    const record={id,task_id:args.id,source_id:input.source_id,result_revision:input.expected_revision,
      relative_path:input.relative_path,version:id,kind:"html",content_hash:String(id).repeat(64),created_at:1,
      base_revision:"base",unavailable:input.unavailable,content:input.expected_content,preview_html:input.preview_html};
    window.saved.push(record);return {...record};
  }
  throw new Error("unexpected command "+command);
};
function Harness(){
  const [shown,setShown]=useState(true);const [revision,setRevision]=useState(1);
  window.hide=()=>setShown(false);window.show=()=>setShown(true);
  window.next=()=>{disk["docs/page.html"].content=disk["docs/page.html"].content.replace("처음 시안","수정 시안");disk["docs/theme.css"].content="h1 {color:rgb(90, 80, 70)}";setRevision(2);};
  window.deleteOriginal=()=>{for(const key of Object.keys(disk))delete disk[key];};
  return <main style={{padding:16}}>{shown&&<ArtifactReviewPanel taskId={7} sourceId="smoke" resultRevision={revision}
    paths={["docs/page.html"]} onAttachReview={value=>window.attached.push(value)} onOpenFile={value=>window.opened.push(value)}/>}</main>;
}
createRoot(document.getElementById("root")).render(<Harness/>);
`);
  server = await createServer({
    configFile: false, root: fixture, plugins: [react()], logLevel: "error",
    resolve: { alias: [{ find: "@tauri-apps/api/core", replacement: path.join(fixture, "core.ts") }] },
    server: { host: "127.0.0.1", port: 0 },
  });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.DOJANG_SMOKE_BROWSER || "chrome" });
  const page = await browser.newPage({ viewport: { width: 1400, height: 1000 } });
  page.setDefaultTimeout(15000);
  const errors = [];
  const remote = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("request", request => { if (request.url().startsWith("https://artifact.invalid/")) remote.push(request.url()); });
  await page.goto(server.resolvedUrls.local[0] + "index.html", { waitUntil: "domcontentloaded" });
  await page.getByRole("button", { name: "검토 버전 저장", exact: true }).click();
  await page.waitForFunction(() => window.saved.length === 1);
  await page.evaluate(() => window.next());
  await page.getByRole("button", { name: "검토 버전 저장", exact: true }).click();
  await page.waitForFunction(() => window.saved.length === 2);
  await page.getByLabel("이전 버전", { exact: true }).selectOption("1");
  await page.getByLabel("이후 버전", { exact: true }).selectOption("2");
  const before = page.frameLocator('iframe[title="이전 버전: docs/page.html"]');
  const after = page.frameLocator('iframe[title="이후 버전: docs/page.html"]');
  await before.getByRole("heading", { name: "처음 시안" }).waitFor();
  await after.getByRole("heading", { name: "수정 시안" }).waitFor();
  assert.equal(await before.locator("h1").evaluate(node => getComputedStyle(node).color), "rgb(12, 34, 56)");
  assert.equal(await after.locator("h1").evaluate(node => getComputedStyle(node).color), "rgb(90, 80, 70)");
  for (const frame of [before, after]) {
    assert.equal(await frame.locator("#logo").evaluate(async node => { await node.decode(); return node.naturalWidth; }), 24);
  }
  for (const sandbox of await page.locator("iframe").evaluateAll(nodes => nodes.map(node => node.getAttribute("sandbox")))) assert.equal(sandbox, "");
  assert.equal(await page.evaluate(() => window.pwned), false);
  assert.deepEqual(remote, []);
  const reads = await page.evaluate(() => window.reads.length);
  await page.evaluate(() => { window.deleteOriginal(); window.hide(); });
  await page.locator('section[aria-label="산출물 버전 비교"]').waitFor({ state: "detached" });
  await page.evaluate(() => window.show());
  await page.getByLabel("이전 버전", { exact: true }).selectOption("1");
  await page.getByLabel("이후 버전", { exact: true }).selectOption("2");
  await before.getByRole("heading", { name: "처음 시안" }).waitFor();
  await after.getByRole("heading", { name: "수정 시안" }).waitFor();
  assert.equal(await page.evaluate(() => window.reads.length), reads, "saved previews must never read current files");
  await page.getByRole("button", { name: "비교 대상을 검토 사유에 연결", exact: true }).click();
  assert.equal(await page.evaluate(() => window.attached.length), 1);
  const feedback = await page.evaluate(() => window.attached[0]);
  assert.match(feedback, /docs\/page\.html/);
  assert.ok(feedback.includes("1".repeat(64)) && feedback.includes("2".repeat(64)));
  assert.match(feedback, /처음 시안/);
  assert.match(feedback, /수정 시안/);
  await page.getByRole("button", { name: "현재 원본 열기", exact: true }).click();
  assert.deepEqual(await page.evaluate(() => window.opened), ["docs/page.html"]);
  assert.deepEqual(errors, []);
  await page.screenshot({ path: path.join(directory, "comparison.png"), fullPage: true });
  console.log(JSON.stringify({ status: "passed", checks: ["production panel and snapshot preparation", "CSS and image freezing", "sandbox and remote denial", "reopen after source deletion", "no live reads", "version-bound review context", "current-file navigation"], screenshot: path.join(directory, "comparison.png") }, null, 2));
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}
